//! API-owned Gatherer and combat seats. Registration, ticking and authority
//! remain native.
use super::*;
use crate::api_combat::{CombatEnd, CombatPage, CombatSessionRequest};
use crate::api_gather::{GatherCounts, GatherEnd, GatherFailure, GatherPage, GatherPhase};
use crate::combat_session::{CombatSessionCard, Settle, SettleCell};
use crate::native::{Script, ScriptStatus, SettingsBag, StartError, StatusValue};
use api::selected::RunKey;
use std::sync::Mutex;

#[path = "progress.rs"]
mod progress;
use progress::ProgressSeat;

#[path = "combat.rs"]
mod combat;
use combat::{Cancel, CombatSeat};

#[derive(Default)]
pub(super) struct ApiSeat {
    gather: GatherSeat,
    pub(super) page: Option<Arc<GatherPage>>,
    pub(super) terminal: Option<GatherEnd>,
    pub(super) retiring: Vec<Preparation>,
    combat: CombatSeat,
    pub(super) combat_page: Option<Arc<CombatPage>>,
    pub(super) combat_terminal: Option<CombatEnd>,
    progress: ProgressSeat,
    pub(super) progress_page: Option<crate::api_progress::ProgressPage>,
    progress_retiring: Vec<progress::ProgressJob>,
    #[cfg(feature = "test-hooks")]
    journal_evidence: Option<(
        api::quest_progress::EvidenceStamp,
        api::quest_progress::EvidenceStamp,
    )>,
}

#[derive(Default)]
enum GatherSeat {
    #[default]
    Idle,
    AwaitingAdmission {
        token: u64,
        run: RunKey,
        settings: Arc<SettingsBag>,
        selected: Arc<api::game_data::SelectedGameData>,
        banks: Arc<api::named_banks::NamedBankFacts>,
        retained: Arc<Mutex<RetainedMemory>>,
        user_input: bool,
    },
    Preparing {
        token: u64,
        job: Preparation,
        /// Manual movement took over before install.
        user_input: bool,
    },
    Running {
        token: u64,
        run: Box<CompiledRun>,
    },
}

impl GatherSeat {
    fn token(&self) -> Option<u64> {
        match self {
            Self::Idle => None,
            Self::AwaitingAdmission { token, .. }
            | Self::Preparing { token, .. }
            | Self::Running { token, .. } => Some(*token),
        }
    }
}

pub(super) fn counts(status: Option<&ScriptStatus>) -> GatherCounts {
    let integer = |key| {
        status
            .and_then(|status| status.fields.iter().find(|field| field.key == key))
            .and_then(|field| match field.value {
                StatusValue::Integer(value) => Some(value),
                _ => None,
            })
            .unwrap_or(0)
    };
    GatherCounts {
        yielded: integer("yielded").clamp(0, u32::MAX as i64) as u32,
        dropped: integer("dropped").clamp(0, u32::MAX as i64) as u32,
        deposited: integer("deposited").clamp(0, u32::MAX as i64) as u32,
        trips: integer("trips").clamp(0, u32::MAX as i64) as u32,
        xp: integer("xp").clamp(i32::MIN as i64, i32::MAX as i64) as i32,
    }
}

fn refusal(error: StartError) -> String {
    match error {
        StartError::Busy => "busy".into(),
        StartError::WorkRefused(refusal) => format!("work-refused:{refusal}"),
        StartError::Unavailable(reason) => format!("unavailable:{reason}"),
        StartError::Facts(error) => format!("facts:{error:?}"),
        StartError::Config(error) => crate::api_gather::settings_refusal(error),
    }
}

impl ApiSeat {
    fn reap(&mut self) {
        let mut index = 0;
        while index < self.retiring.len() {
            if self.retiring[index].is_finished() {
                if let Ok(run) = self.retiring.swap_remove(index).join() {
                    let _ = compiled::destroy_run(Box::new(run), StopReason::Replaced);
                }
            } else {
                index += 1;
            }
        }
        let mut index = 0;
        while index < self.progress_retiring.len() {
            if self.progress_retiring[index].is_finished() {
                let _ = self.progress_retiring.swap_remove(index).join();
            } else {
                index += 1;
            }
        }
    }

    /// Any live API session: they share the slot's one foreground.
    fn busy(&self) -> bool {
        self.gather.token().is_some()
            || self.combat.token().is_some()
            || self.progress.token().is_some()
    }

    /// The phase of an admitted API Session, not the preparation worker's
    /// own progress. Other native/JS families own their own page details.
    pub(super) fn phase_for(&self, id: RequestId) -> Option<crate::native::NativePhase> {
        use crate::native::NativePhase;
        let token = id.get();
        match &self.gather {
            GatherSeat::AwaitingAdmission { token: live, .. }
            | GatherSeat::Preparing { token: live, .. }
                if *live == token =>
            {
                return Some(NativePhase::Preparing)
            }
            GatherSeat::Running { token: live, run } if *live == token => {
                return Some(
                    run.output
                        .status
                        .as_ref()
                        .map_or(NativePhase::Working, |s| s.phase),
                )
            }
            _ => {}
        }
        match &self.combat {
            CombatSeat::AwaitingAdmission { token: live, .. }
            | CombatSeat::Preparing { token: live, .. }
                if *live == token =>
            {
                return Some(NativePhase::Preparing)
            }
            CombatSeat::Running {
                token: live, run, ..
            } if *live == token => {
                return Some(
                    run.output
                        .status
                        .as_ref()
                        .map_or(NativePhase::Working, |s| s.phase),
                )
            }
            _ => {}
        }
        match &self.progress {
            ProgressSeat::AwaitingAdmission { token: live, .. }
            | ProgressSeat::Preparing { token: live, .. }
                if *live == token =>
            {
                Some(NativePhase::Preparing)
            }
            ProgressSeat::Reading { token: live, .. } if *live == token => {
                Some(NativePhase::Waiting)
            }
            _ => None,
        }
    }
}

impl SlotScript {
    fn api_session_epoch(&self) -> u64 {
        self.seat
            .as_ref()
            .map_or(self.work_epoch, |seat| seat.run().session)
    }

    pub fn api_owns_foreground(&self) -> bool {
        self.api.as_ref().is_some_and(|seat| seat.busy())
    }

    /// A live Gather or Combat session: work in the world that manual
    /// movement takes over, with or without a walk in flight. A quest
    /// progress read moves nothing and is not included.
    pub fn api_session_takes_manual_input(&self) -> bool {
        self.api
            .as_ref()
            .is_some_and(|seat| seat.gather.token().is_some() || seat.combat.token().is_some())
    }

    /// Manual movement took over the slot: every live Gather or Combat
    /// session learns it, whether or not a walk was in flight.
    pub(super) fn api_user_input(&mut self) {
        self.cancel_api_combat(Cancel::UserInput);
        let Some(seat) = self.api.as_mut() else {
            return;
        };
        match &mut seat.gather {
            GatherSeat::Idle => {}
            GatherSeat::AwaitingAdmission { user_input, .. }
            | GatherSeat::Preparing { user_input, .. } => *user_input = true,
            GatherSeat::Running { .. } => {
                // The Gatherer blocks `manual-movement`, as for a cancelled walk.
                self.signal_api_gather(|script| script.user_input());
            }
        }
    }

    /// Read-only receipt seam for the ignored public Script API live cells.
    #[cfg(feature = "test-hooks")]
    #[doc(hidden)]
    pub fn api_live_test_probe(&mut self) -> serde_json::Value {
        let stamp = |value: api::quest_progress::EvidenceStamp| {
            serde_json::json!({
                "run": value.run.run,
                "session": value.run.session,
                "tick": value.tick,
                "sequence": value.sequence,
            })
        };
        let journal = self
            .api
            .as_ref()
            .and_then(|seat| seat.journal_evidence)
            .map(|(acquired, closed)| {
                serde_json::json!({
                    "acquired": stamp(acquired),
                    "closed": stamp(closed),
                })
            });
        serde_json::json!({
            "seat_present": self.api.is_some(),
            "foreground": self.api_owns_foreground(),
            "journal": journal,
            "paint_hidden": self.journal_paint_hidden(Instant::now()),
            "gather": self.probe("globalThis.__rs_api?.snapshot.gather ?? null").ok(),
        })
    }

    pub(super) fn api_status(&self) -> Option<Arc<ScriptStatus>> {
        let seat = self.api.as_ref()?;
        seat.page
            .as_ref()
            .or(seat.combat_page.as_ref())?
            .status
            .clone()
    }

    pub(super) fn api_game_data(&self) -> Option<Arc<api::game_data::SelectedGameData>> {
        match &self.api.as_ref()?.gather {
            GatherSeat::Running { run, .. } => Some(Arc::clone(&run.selected)),
            _ => match &self.api.as_ref()?.combat {
                CombatSeat::Running { run, .. } => Some(Arc::clone(&run.selected)),
                _ => self.api.as_ref()?.progress.selected(),
            },
        }
    }

    /// Permissions from a running Load API Gather card's committed config.
    /// Applies to its native Gatherer walks, not the Load script's own rows.
    pub fn api_gather_walk_permissions(&self) -> Option<crate::native::WalkPermissions> {
        let seat = self.api.as_ref()?;
        match &seat.gather {
            GatherSeat::Running { run, .. } => Some(run.config.walk_permissions()),
            GatherSeat::Idle
            | GatherSeat::AwaitingAdmission { .. }
            | GatherSeat::Preparing { .. } => None,
        }
    }

    /// Legacy family-level test injection only. Production starts cross the
    /// typed seat admission path in `SlotScript::consume_seat_batch`.
    #[cfg(test)]
    pub(super) fn consume_api_control(&mut self, request: &crate::shim::InteractReq) {
        match request {
            crate::shim::InteractReq::GatherRun {
                request_id,
                settings,
            } => {
                self.start_api_gather(*request_id, settings);
            }
            crate::shim::InteractReq::GatherStop { request_id } => {
                self.stop_api_gather(*request_id)
            }
            crate::shim::InteractReq::CombatFight {
                request_id,
                request,
            } => self.start_api_combat(*request_id, request),
            crate::shim::InteractReq::CombatStop { request_id } => {
                self.stop_api_combat(*request_id)
            }
            crate::shim::InteractReq::ProgressRead { request_id, name } => {
                self.start_api_progress(*request_id, name);
            }
            _ => unreachable!("API control admission"),
        }
        self.settle_api_seat_terminals();
    }

    pub(super) fn consume_api_release(&mut self, request: &crate::shim::InteractReq) {
        match request {
            crate::shim::InteractReq::GatherStop { request_id } => {
                self.stop_api_gather(*request_id)
            }
            crate::shim::InteractReq::CombatStop { request_id } => {
                self.stop_api_combat(*request_id)
            }
            _ => unreachable!("only reserved API release reaches this entry"),
        }
        self.settle_api_seat_terminals();
    }

    /// The family page keeps the full typed result; the seat publishes its
    /// bounded authority terminal. Final-tick native actions must be captured
    /// before this call, so ordinary completion cannot re-root them.
    pub(super) fn settle_api_seat_terminals(&mut self) {
        use api::script_contract::{AbortReason, AuthorityStatus};
        let (Some(families), Some(seat)) = (self.api.as_ref(), self.seat.as_mut()) else {
            return;
        };
        enum End {
            Done,
            Failed,
            Manual,
        }
        let gather = families.terminal.as_ref().map(|end| {
            (
                end.token(),
                if matches!(end, GatherEnd::Stopped { .. }) {
                    End::Done
                } else {
                    End::Failed
                },
            )
        });
        let combat = families.combat_terminal.as_ref().map(|end| {
            (
                end.token(),
                match end {
                    CombatEnd::Fought { .. }
                    | CombatEnd::Stopped { .. }
                    | CombatEnd::Interrupted { .. } => {
                        if matches!(
                            end,
                            CombatEnd::Interrupted {
                                cause: crate::api_combat::InterruptCause::UserInput,
                                ..
                            }
                        ) {
                            End::Manual
                        } else {
                            End::Done
                        }
                    }
                    CombatEnd::Refused { .. } | CombatEnd::Failed { .. } => End::Failed,
                },
            )
        });
        let progress = families.progress_page.as_ref().and_then(|page| match page {
            crate::api_progress::ProgressPage::Done { token, .. } => Some((*token, End::Done)),
            crate::api_progress::ProgressPage::Refused { token, .. } => Some((*token, End::Failed)),
            crate::api_progress::ProgressPage::Reading { .. } => None,
        });
        for (token, end) in [gather, combat, progress].into_iter().flatten() {
            let Ok(id) = RequestId::isolate(token) else {
                self.pending_logs.push(format!(
                    "seat API terminal has invalid isolate request id: {token}"
                ));
                continue;
            };
            if seat.lookup(id) != AuthorityStatus::Live {
                continue;
            }
            let settled = match end {
                End::Done => seat.complete(id, false),
                End::Failed => seat.complete(id, true),
                End::Manual => seat.revoke(id, AbortReason::UserInput),
            };
            if let Err(error) = settled {
                self.pending_logs
                    .push(format!("seat API terminal refused: {error:?}"));
            }
        }
    }

    /// A carried Cancel may revoke an admitted API Session (or its ancestor)
    /// while its family runner is still preparing or working. The seat result
    /// owns settlement; tear down the native runner before admitting another
    /// Session, without replacing that result with a family `Stopped` page.
    pub(super) fn stop_cancelled_api_work(&mut self) {
        use api::script_contract::AuthorityStatus;
        let (Some(families), Some(seat)) = (self.api.as_ref(), self.seat.as_ref()) else {
            return;
        };
        let cancelled = |token: Option<u64>| {
            token.filter(|token| {
                RequestId::isolate(*token).is_ok_and(|id| seat.lookup(id) != AuthorityStatus::Live)
            })
        };
        let gather = cancelled(families.gather.token());
        let combat = cancelled(families.combat.token());
        let progress = cancelled(families.progress.token());
        if let Some(token) = gather {
            self.stop_api_gather(token);
            self.api.as_mut().expect("family seat").terminal = None;
        }
        if let Some(token) = combat {
            self.stop_api_combat(token);
            self.api.as_mut().expect("family seat").combat_terminal = None;
        }
        if progress.is_some() {
            let families = self.api.as_mut().expect("family seat");
            families.progress.reset(&mut families.progress_retiring);
            families.progress_page = None;
            self.native_runtime.revoke();
        }
    }

    pub(super) fn start_api_gather(&mut self, token: u64, settings: &Arc<SettingsBag>) {
        if !self.surface_active() {
            return;
        }
        let seat = self.api.get_or_insert_with(|| Box::new(ApiSeat::default()));
        seat.reap();
        if seat.gather.token() == Some(token)
            || seat
                .terminal
                .as_ref()
                .is_some_and(|terminal| terminal.token() == token)
        {
            return;
        }
        if seat.busy() || seat.retiring.len() + seat.progress_retiring.len() >= 4 {
            // Genuine callers keep their family Busy until consuming the old
            // terminal; this also fails closed if a reserved control races it.
            seat.terminal = Some(GatherEnd::Refused {
                token,
                reason: "busy".into(),
            });
            return;
        }
        let selected = self
            .surface_start
            .as_ref()
            .and_then(|identity| identity.game_data.clone());
        let Some(selected) = selected else {
            seat.terminal = Some(GatherEnd::Refused {
                token,
                reason: "unavailable:selected game data unavailable".into(),
            });
            return;
        };
        let generation = self
            .control_generation
            .max(self.runtime_generation)
            .checked_add(1);
        let Some(generation) = generation else {
            seat.terminal = Some(GatherEnd::Refused {
                token,
                reason: "unavailable:run generation exhausted".into(),
            });
            return;
        };
        let retained = Arc::clone(
            self.retained
                .get_or_insert_with(|| Arc::new(Mutex::new(RetainedMemory::default()))),
        );
        let banks = Arc::clone(
            &self
                .surface_start
                .as_ref()
                .expect("Load identity")
                .named_banks,
        );
        let run = RunKey {
            slot: self.incarnation,
            run: generation,
            session: self
                .seat
                .as_ref()
                .map_or(self.work_epoch, |seat| seat.run().session),
        };
        match compiled::prepare_run(
            &crate::gatherer::CARD,
            run,
            Arc::clone(settings),
            Arc::clone(&selected),
            Arc::clone(&banks),
            Arc::clone(&retained),
            String::new(),
        ) {
            Ok(job) => {
                self.control_generation = generation;
                seat.page = Some(Arc::new(GatherPage {
                    token,
                    phase: GatherPhase::Preparing,
                    status: None,
                }));
                seat.gather = GatherSeat::Preparing {
                    token,
                    job,
                    user_input: false,
                };
            }
            Err(compiled::PrepareRunError::Full) => {
                self.control_generation = generation;
                seat.page = Some(Arc::new(GatherPage {
                    token,
                    phase: GatherPhase::Preparing,
                    status: None,
                }));
                seat.gather = GatherSeat::AwaitingAdmission {
                    token,
                    run,
                    settings: Arc::clone(settings),
                    selected,
                    banks,
                    retained,
                    user_input: false,
                };
            }
            Err(compiled::PrepareRunError::Other(error)) => {
                seat.terminal = Some(GatherEnd::Refused {
                    token,
                    reason: refusal(error).into(),
                })
            }
        }
    }

    fn stop_api_gather(&mut self, token: u64) {
        if !self
            .api
            .as_ref()
            .is_some_and(|seat| seat.gather.token() == Some(token))
        {
            return;
        }
        let seat = self.api.as_mut().expect("live seat");
        let state = std::mem::take(&mut seat.gather);
        let counts = counts(seat.page.as_ref().and_then(|page| page.status.as_deref()));
        self.native_runtime.revoke();
        match state {
            GatherSeat::AwaitingAdmission { .. } => {}
            GatherSeat::Preparing { job, .. } => seat.retiring.push(job),
            GatherSeat::Running { run, .. } => {
                if let Some(error) = compiled::destroy_run(run, StopReason::Operator) {
                    self.pending_logs.push(error);
                }
            }
            GatherSeat::Idle => unreachable!(),
        }
        if let Some(retained) = &self.retained {
            *retained
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .gather() = Default::default();
        }
        seat.page = None;
        seat.terminal = Some(GatherEnd::Stopped { token, counts });
    }

    pub(super) fn tick_api(&mut self, ctx: &mut ScriptCtx<'_>) {
        let Some(mut seat) = self.api.take() else {
            return;
        };
        seat.reap();
        if let GatherSeat::AwaitingAdmission {
            token,
            run,
            settings,
            selected,
            banks,
            retained,
            user_input,
        } = &seat.gather
        {
            match compiled::prepare_run(
                &crate::gatherer::CARD,
                *run,
                Arc::clone(settings),
                Arc::clone(selected),
                Arc::clone(banks),
                Arc::clone(retained),
                String::new(),
            ) {
                Ok(job) => {
                    seat.gather = GatherSeat::Preparing {
                        token: *token,
                        job,
                        user_input: *user_input,
                    };
                }
                Err(compiled::PrepareRunError::Full) => {}
                Err(compiled::PrepareRunError::Other(error)) => {
                    let token = *token;
                    seat.gather = GatherSeat::Idle;
                    seat.page = None;
                    seat.terminal = Some(GatherEnd::Refused {
                        token,
                        reason: refusal(error).into(),
                    });
                }
            }
        }
        if matches!(&seat.gather, GatherSeat::Preparing { job, .. } if job.is_finished()) {
            let GatherSeat::Preparing {
                token,
                job,
                user_input,
            } = std::mem::take(&mut seat.gather)
            else {
                unreachable!()
            };
            match job.join() {
                Ok(mut run) => {
                    run.rekey_session(self.api_session_epoch());
                    seat.page = Some(Arc::new(GatherPage {
                        token,
                        phase: GatherPhase::Running,
                        status: None,
                    }));
                    seat.gather = GatherSeat::Running {
                        token,
                        run: Box::new(run),
                    };
                    // Publish the install page before the first native tick.
                    self.api = Some(seat);
                    if user_input {
                        self.signal_api_gather(|script| script.user_input());
                    }
                    return;
                }
                Err(error) => {
                    seat.page = None;
                    seat.terminal = Some(GatherEnd::Refused {
                        token,
                        reason: refusal(error).into(),
                    });
                }
            }
        }
        self.tick_api_combat(&mut seat, ctx);
        if let GatherSeat::Running { token, run } = &mut seat.gather {
            ctx.compiled.interacts = None;
            let result = {
                let mut retained = self
                    .retained
                    .as_ref()
                    .expect("seat retention")
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                let (result, interacts) = run.tick(
                    ctx,
                    &mut retained,
                    &mut self.native_runtime,
                    None,
                    self.local_state.as_deref(),
                    compiled::SeatTickPorts {
                        authority: None,
                        prompt: self
                            .seat
                            .as_mut()
                            .map(|seat| seat.as_mut() as &mut dyn crate::native::SeatPromptPort),
                    },
                );
                ctx.compiled.interacts = interacts;
                run.finish_tick();
                result
            };
            let changed = match seat.page.as_ref().and_then(|page| page.status.as_ref()) {
                Some(status) => run
                    .output
                    .status
                    .as_ref()
                    .is_none_or(|next| !Arc::ptr_eq(status, next)),
                None => run.output.status.is_some(),
            };
            if changed {
                seat.page = Some(Arc::new(GatherPage {
                    token: *token,
                    phase: GatherPhase::Running,
                    status: run.output.status.clone(),
                }));
            }
            let counts = counts(run.output.status.as_deref());
            let terminal = match result {
                Ok(ScriptFlow::Continue) => None,
                Ok(ScriptFlow::Complete) => Some((
                    GatherEnd::Stopped {
                        token: *token,
                        counts,
                    },
                    StopReason::Completed,
                )),
                Ok(ScriptFlow::Blocked(failure)) => Some((
                    GatherEnd::Blocked {
                        token: *token,
                        counts,
                        failure: GatherFailure {
                            code: failure.code,
                            message: failure.message,
                        },
                    },
                    StopReason::Error,
                )),
                Err(failure) => Some((
                    GatherEnd::Failed {
                        token: *token,
                        counts,
                        reason: failure.message,
                    },
                    StopReason::Error,
                )),
            };
            if let Some((terminal, reason)) = terminal {
                self.native_runtime.revoke();
                let GatherSeat::Running { run, .. } = std::mem::take(&mut seat.gather) else {
                    unreachable!()
                };
                if let Some(error) = compiled::destroy_run(run, reason) {
                    self.pending_logs.push(error);
                }
                seat.page = None;
                seat.terminal = Some(terminal);
            }
        }
        self.tick_api_progress(&mut seat, ctx);
        self.api = Some(seat);
    }

    pub(super) fn interrupt_api(&mut self, event: Interrupt) {
        self.interrupt_api_combat(event);
        self.signal_api_gather(|script| script.interrupt(event));
    }

    /// Deliver a lifecycle signal to the installed Gatherer card; a panic
    /// ends the session `failed`.
    fn signal_api_gather(&mut self, signal: impl FnOnce(&mut dyn Script)) {
        let Some(seat) = self.api.as_mut() else {
            return;
        };
        let GatherSeat::Running { run, token } = &mut seat.gather else {
            return;
        };
        if let Err(payload) = catch_unwind(AssertUnwindSafe(|| signal(&mut *run.script))) {
            let terminal = GatherEnd::Failed {
                token: *token,
                counts: counts(run.output.status.as_deref()),
                reason: panic_message(&payload).into(),
            };
            self.native_runtime.revoke();
            let GatherSeat::Running { run, .. } = std::mem::take(&mut seat.gather) else {
                unreachable!()
            };
            if let Some(error) = compiled::destroy_run(run, StopReason::Error) {
                self.pending_logs.push(error);
            }
            seat.page = None;
            seat.terminal = Some(terminal);
        }
    }

    pub(super) fn api_session_boundary(&mut self, reconnect: bool) {
        self.combat_session_boundary(reconnect);
        if reconnect {
            if let Some(seat) = self.api.as_mut() {
                if let GatherSeat::Running { run, .. } = &mut seat.gather {
                    run.rekey_session(self.work_epoch);
                    if let Some(page) = seat.page.as_mut() {
                        Arc::make_mut(page).status = run.output.status.clone();
                    }
                }
                seat.progress.rekey(self.work_epoch);
            }
        } else if let Some(seat) = self.api.as_mut() {
            match std::mem::take(&mut seat.gather) {
                GatherSeat::AwaitingAdmission { .. } => {}
                GatherSeat::Preparing { job, .. } => seat.retiring.push(job),
                GatherSeat::Running { run, .. } => {
                    if let Some(error) = compiled::destroy_run(run, StopReason::Replaced) {
                        self.pending_logs.push(error);
                    }
                }
                GatherSeat::Idle => {}
            }
            seat.progress.reset(&mut seat.progress_retiring);
            seat.progress_page = None;
            seat.page = None;
            seat.terminal = None;
        }
    }

    /// A transient down observation may advance the local work epoch before
    /// F0 mints the replacement session. Align retained family run stamps
    /// without repeating their reconnect cancellation or preparation.
    pub(super) fn align_api_session(&mut self, session: u64) {
        let Some(seat) = self.api.as_mut() else {
            return;
        };
        match &mut seat.gather {
            GatherSeat::AwaitingAdmission { run, .. } => run.session = session,
            GatherSeat::Running { run, .. } => {
                run.rekey_session(session);
                if let Some(page) = seat.page.as_mut() {
                    Arc::make_mut(page).status = run.output.status.clone();
                }
            }
            GatherSeat::Idle | GatherSeat::Preparing { .. } => {}
        }
        match &mut seat.combat {
            CombatSeat::AwaitingAdmission { run, .. } => run.session = session,
            CombatSeat::Running { run, .. } => run.rekey_session(session),
            CombatSeat::Idle | CombatSeat::Preparing { .. } => {}
        }
        seat.progress.rekey(session);
    }

    pub(super) fn teardown_api(&mut self, reason: StopReason) {
        let Some(mut seat) = self.api.take() else {
            return;
        };
        self.teardown_api_combat(&mut seat, reason);
        self.native_runtime.revoke();
        if let GatherSeat::Running { run, .. } = std::mem::take(&mut seat.gather) {
            if let Some(error) = compiled::destroy_run(run, reason) {
                self.pending_logs.push(error);
            }
        }
        // Unfinished packets remain contained by PreparationResult/ScriptOwner.
        // Dropping a JoinHandle never waits in the host pump.
        let _ = catch_unwind(AssertUnwindSafe(|| drop(seat)));
    }

    pub(super) fn api_recovery_anchor(&mut self) -> Option<Option<WatchdogTile>> {
        let GatherSeat::Running { run, .. } = &self.api.as_ref()?.gather else {
            return None;
        };
        match catch_unwind(AssertUnwindSafe(|| run.script.recovery_anchor())) {
            Ok(anchor) => Some(anchor.map(|tile| WatchdogTile {
                x: tile.x,
                z: tile.z,
                level: tile.level,
            })),
            Err(payload) => {
                let seat = self.api.as_mut().expect("running seat");
                let GatherSeat::Running { token, run } = std::mem::take(&mut seat.gather) else {
                    unreachable!()
                };
                let terminal = GatherEnd::Failed {
                    token,
                    counts: counts(run.output.status.as_deref()),
                    reason: format!("gather recovery anchor panic: {}", panic_message(&payload))
                        .into(),
                };
                self.native_runtime.revoke();
                if let Some(error) = compiled::destroy_run(run, StopReason::Error) {
                    self.pending_logs.push(error);
                }
                seat.page = None;
                seat.terminal = Some(terminal);
                Some(None)
            }
        }
    }
}
