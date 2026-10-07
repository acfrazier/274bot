//! The API-owned combat seat: one `api.combat.fight` session per slot.
//!
//! Lifecycle mirrors the gather seat (preparing off-pump, running, one
//! retained terminal per token). Whenever the seat drops a card — settled,
//! stopped, reset or torn down — it first hands the card's remaining
//! accepted Combat raises to the host's off-click pump, before any revoke,
//! as operator Stop does for a compiled card.
//!
//! Pause, reconnect and manual movement cancel an admitted session at any
//! lifecycle point: a preparing session records the cause and settles
//! `interrupted` on its next eligible tick without installing its card; an
//! installed card cancels its fight, begun or not, and settles after its
//! scoped prayer clear.
use super::*;
use crate::api_combat::InterruptCause;
use crate::combat::CombatTables;
use crate::native::{PreparedConfig, Script};

/// A lifecycle event that cancels an admitted session.
#[derive(Clone, Copy)]
pub(super) enum Cancel {
    Pause,
    Reconnect,
    UserInput,
}

impl Cancel {
    fn cause(self) -> InterruptCause {
        match self {
            Self::Pause => InterruptCause::Pause,
            Self::Reconnect => InterruptCause::Reconnect,
            Self::UserInput => InterruptCause::UserInput,
        }
    }
}

#[derive(Default)]
pub(super) enum CombatSeat {
    #[default]
    Idle,
    Preparing {
        token: u64,
        job: Preparation,
        cell: SettleCell,
        /// The first cancellation seen before install.
        cancelled: Option<InterruptCause>,
    },
    Running {
        token: u64,
        run: Box<CompiledRun>,
        cell: SettleCell,
    },
}

impl CombatSeat {
    pub(super) fn token(&self) -> Option<u64> {
        match self {
            Self::Idle => None,
            Self::Preparing { token, .. } | Self::Running { token, .. } => Some(*token),
        }
    }
}

fn running_page(token: u64, status: Option<Arc<ScriptStatus>>) -> Arc<CombatPage> {
    Arc::new(CombatPage {
        token,
        phase: GatherPhase::Running,
        status,
    })
}

fn terminal(token: u64, settle: Settle) -> CombatEnd {
    match settle {
        Settle::Fought(report) => CombatEnd::Fought { token, report },
        Settle::Interrupted(cause) => CombatEnd::Interrupted { token, cause },
        Settle::Refused(reason) => CombatEnd::Refused { token, reason },
        Settle::Failed(reason) => CombatEnd::Failed { token, reason },
    }
}

impl SlotScript {
    pub(super) fn start_api_combat(&mut self, token: u64, request: &Arc<CombatSessionRequest>) {
        if !self.load_active() {
            return;
        }
        let seat = self.api.get_or_insert_with(|| Box::new(ApiSeat::default()));
        seat.reap();
        if seat.combat.token() == Some(token)
            || seat
                .combat_terminal
                .as_ref()
                .is_some_and(|terminal| terminal.token() == token)
        {
            return;
        }
        let refuse = |seat: &mut ApiSeat, reason: &str| {
            seat.combat_terminal = Some(CombatEnd::Refused {
                token,
                reason: reason.into(),
            });
        };
        if seat.busy() || seat.retiring.len() + seat.progress_retiring.len() >= 4 {
            refuse(seat, "busy");
            return;
        }
        let Some(selected) = self
            .load_identity
            .as_ref()
            .and_then(|identity| identity.game_data.clone())
        else {
            refuse(seat, "unavailable:selected game data unavailable");
            return;
        };
        let Some(generation) = self
            .control_generation
            .max(self.runtime_generation)
            .checked_add(1)
        else {
            refuse(seat, "unavailable:run generation exhausted");
            return;
        };
        let run = RunKey {
            slot: self.incarnation,
            run: generation,
            session: self.work_epoch,
        };
        let cell = SettleCell::default();
        let card_cell = Arc::clone(&cell);
        let request = Arc::clone(request);
        let prepared = compiled::prepare_native_run(run, selected, String::new(), move |data| {
            // Built per session on the preparation worker and dropped with
            // the card: nothing is retained while no session is live.
            let tables = CombatTables::build(Arc::clone(data))
                .map_err(|_| StartError::Unavailable("combat tables unavailable".into()))?;
            let native = request.resolve(&tables).map_err(StartError::Config)?;
            let config = PreparedConfig::new(
                crate::combat_session::CARD_ID,
                1,
                1,
                Arc::new(SettingsBag::new()),
                (),
            );
            let card: Box<dyn Script> =
                Box::new(CombatSessionCard::new(Arc::new(native), tables, card_cell));
            Ok((card, config))
        });
        match prepared {
            Ok(job) => {
                self.control_generation = generation;
                seat.dropped_rows = 0;
                seat.reported_drops = 0;
                seat.dropped_owner = None;
                seat.combat_page = Some(Arc::new(CombatPage {
                    token,
                    phase: GatherPhase::Preparing,
                    status: None,
                }));
                seat.combat = CombatSeat::Preparing {
                    token,
                    job,
                    cell,
                    cancelled: None,
                };
            }
            Err(error) => refuse(seat, &refusal(error)),
        }
    }

    /// Hand a dropped card's owed raises to the host, then destroy it.
    fn retire_combat_run(&mut self, run: Box<CompiledRun>, reason: StopReason) {
        match catch_unwind(AssertUnwindSafe(|| run.script.prayer_cleanup())) {
            Ok(owned) => self.stop_prayer_cleanup.merge(owned),
            Err(payload) => self.pending_logs.push(format!(
                "combat prayer cleanup snapshot panic: {}",
                panic_message(&payload)
            )),
        }
        self.native_runtime.revoke();
        if let Some(error) = compiled::destroy_run(run, reason) {
            self.pending_logs.push(error);
        }
    }

    pub(super) fn stop_api_combat(&mut self, token: u64) {
        let Some(seat) = self.api.as_mut() else {
            return;
        };
        if seat.combat.token() != Some(token) {
            return;
        }
        let state = std::mem::take(&mut seat.combat);
        seat.combat_page = None;
        seat.combat_terminal = Some(CombatEnd::Stopped { token });
        match state {
            CombatSeat::Preparing { job, .. } => seat.retiring.push(job),
            CombatSeat::Running { run, .. } => self.retire_combat_run(run, StopReason::Operator),
            CombatSeat::Idle => unreachable!(),
        }
    }

    pub(super) fn tick_api_combat(&mut self, seat: &mut ApiSeat, ctx: &mut ScriptCtx<'_>) {
        if let CombatSeat::Preparing {
            cancelled: Some(cause),
            ..
        } = &seat.combat
        {
            let cause = *cause;
            let CombatSeat::Preparing { token, job, .. } = std::mem::take(&mut seat.combat) else {
                unreachable!()
            };
            // Never installed: nothing ran, so nothing is owed.
            seat.retiring.push(job);
            seat.combat_page = None;
            seat.combat_terminal = Some(CombatEnd::Interrupted { token, cause });
            return;
        }
        if matches!(&seat.combat, CombatSeat::Preparing { job, .. } if job.is_finished()) {
            let CombatSeat::Preparing {
                token, job, cell, ..
            } = std::mem::take(&mut seat.combat)
            else {
                unreachable!()
            };
            match job.join() {
                Ok(mut run) => {
                    run.rekey_session(self.work_epoch);
                    seat.combat_page = Some(running_page(token, None));
                    seat.combat = CombatSeat::Running {
                        token,
                        run: Box::new(run),
                        cell,
                    };
                    // Publish the install page before the first native tick.
                    return;
                }
                Err(error) => {
                    seat.combat_page = None;
                    seat.combat_terminal = Some(CombatEnd::Refused {
                        token,
                        reason: refusal(error).into(),
                    });
                    return;
                }
            }
        }
        let CombatSeat::Running { token, run, cell } = &mut seat.combat else {
            return;
        };
        let token = *token;
        ctx.compiled.interacts = None;
        let result = {
            let mut retained = self
                .retained
                .get_or_insert_with(|| Arc::new(Mutex::new(RetainedMemory::default())))
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            run.tick(ctx, &mut retained, &mut self.native_runtime, None)
        };
        let changed = match seat
            .combat_page
            .as_ref()
            .and_then(|page| page.status.as_ref())
        {
            Some(status) => run
                .output
                .status
                .as_ref()
                .is_none_or(|next| !Arc::ptr_eq(status, next)),
            None => run.output.status.is_some(),
        };
        if changed {
            seat.combat_page = Some(running_page(token, run.output.status.clone()));
        }
        let end = match result {
            Ok(ScriptFlow::Continue) => return,
            Ok(ScriptFlow::Complete) => {
                let settle = cell
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .take();
                settle.map_or_else(
                    || CombatEnd::Failed {
                        token,
                        reason: "no settlement".into(),
                    },
                    |settle| terminal(token, settle),
                )
            }
            Ok(ScriptFlow::Blocked(failure)) => CombatEnd::Failed {
                token,
                reason: failure.code,
            },
            Err(failure) => CombatEnd::Failed {
                token,
                reason: failure.message,
            },
        };
        let CombatSeat::Running { run, .. } = std::mem::take(&mut seat.combat) else {
            unreachable!()
        };
        seat.combat_page = None;
        seat.combat_terminal = Some(end);
        self.retire_combat_run(run, StopReason::Completed);
    }

    pub(super) fn interrupt_api_combat(&mut self, event: Interrupt) {
        if event == Interrupt::Pause {
            self.cancel_api_combat(Cancel::Pause);
        }
    }

    /// Cancel the admitted session wherever it is in its lifecycle.
    pub(super) fn cancel_api_combat(&mut self, cancel: Cancel) {
        let Some(seat) = self.api.as_mut() else {
            return;
        };
        let (token, run) = match &mut seat.combat {
            CombatSeat::Idle => return,
            CombatSeat::Preparing { cancelled, .. } => {
                cancelled.get_or_insert(cancel.cause());
                return;
            }
            CombatSeat::Running { token, run, .. } => (*token, run),
        };
        let signal = || match cancel {
            Cancel::Pause => run.script.interrupt(Interrupt::Pause),
            Cancel::Reconnect => run.script.interrupt(Interrupt::SessionEnded),
            Cancel::UserInput => run.script.user_input(),
        };
        if let Err(payload) = catch_unwind(AssertUnwindSafe(signal)) {
            let CombatSeat::Running { run, .. } = std::mem::take(&mut seat.combat) else {
                unreachable!()
            };
            seat.combat_page = None;
            seat.combat_terminal = Some(CombatEnd::Failed {
                token,
                reason: panic_message(&payload).into(),
            });
            self.retire_combat_run(run, StopReason::Error);
        }
    }

    /// A reconnect keeps the session but cancels its fight: the card settles
    /// `interrupted` / `reconnect` in the new session after its scoped
    /// clear. A reset ends it with no terminal.
    pub(super) fn combat_session_boundary(&mut self, reconnect: bool) {
        let Some(seat) = self.api.as_mut() else {
            return;
        };
        if reconnect {
            if let CombatSeat::Running { run, .. } = &mut seat.combat {
                run.rekey_session(self.work_epoch);
            }
            self.cancel_api_combat(Cancel::Reconnect);
            return;
        }
        seat.combat_page = None;
        seat.combat_terminal = None;
        match std::mem::take(&mut seat.combat) {
            CombatSeat::Preparing { job, .. } => seat.retiring.push(job),
            CombatSeat::Running { run, .. } => self.retire_combat_run(run, StopReason::Replaced),
            CombatSeat::Idle => {}
        }
    }

    pub(super) fn teardown_api_combat(&mut self, seat: &mut ApiSeat, reason: StopReason) {
        if let CombatSeat::Running { run, .. } = std::mem::take(&mut seat.combat) {
            self.retire_combat_run(run, reason);
        }
    }
}

#[cfg(test)]
#[path = "combat_tests.rs"]
mod tests;
