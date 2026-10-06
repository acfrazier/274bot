//! API-owned Gatherer seat. Registration, ticking and authority remain native.
use super::*;
use crate::api_gather::{GatherCounts, GatherEnd, GatherFailure, GatherPage, GatherPhase};
use crate::native::{ScriptStatus, SettingsBag, StartError, StatusValue};
use api::selected::RunKey;
use std::sync::Mutex;

#[path = "progress.rs"]
mod progress;
use progress::ProgressSeat;

#[derive(Default)]
pub(super) struct ApiSeat {
    gather: GatherSeat,
    pub(super) page: Option<Arc<GatherPage>>,
    pub(super) terminal: Option<GatherEnd>,
    pub(super) retiring: Vec<Preparation>,
    progress: ProgressSeat,
    pub(super) progress_page: Option<crate::api_progress::ProgressPage>,
    progress_retiring: Vec<progress::ProgressJob>,
    dropped_rows: usize,
    reported_drops: usize,
    dropped_owner: Option<DropOwner>,
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
    Preparing {
        token: u64,
        job: Preparation,
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
            Self::Preparing { token, .. } | Self::Running { token, .. } => Some(*token),
        }
    }
}

#[derive(Clone, Copy)]
enum DropOwner {
    Gather,
    Progress,
}

impl DropOwner {
    fn label(self) -> &'static str {
        match self {
            Self::Gather => "gather",
            Self::Progress => "progress",
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
    fn foreground_owner(&self) -> Option<DropOwner> {
        if self.gather.token().is_some() {
            Some(DropOwner::Gather)
        } else if self.progress.token().is_some() {
            Some(DropOwner::Progress)
        } else {
            None
        }
    }
}

impl SlotScript {
    pub fn api_owns_foreground(&self) -> bool {
        self.api
            .as_ref()
            .is_some_and(|seat| seat.gather.token().is_some() || seat.progress.token().is_some())
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
        self.api.as_ref()?.page.as_ref()?.status.clone()
    }

    pub(super) fn api_game_data(&self) -> Option<Arc<api::game_data::SelectedGameData>> {
        match &self.api.as_ref()?.gather {
            GatherSeat::Running { run, .. } => Some(Arc::clone(&run.selected)),
            _ => self.api.as_ref()?.progress.selected(),
        }
    }

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
            crate::shim::InteractReq::ProgressRead { request_id, name } => {
                self.start_api_progress(*request_id, name);
            }
            _ => unreachable!("API control admission"),
        }
    }

    fn start_api_gather(&mut self, token: u64, settings: &Arc<SettingsBag>) {
        if !self.load_active() {
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
        if seat.gather.token().is_some()
            || seat.progress.token().is_some()
            || seat.retiring.len() + seat.progress_retiring.len() >= 4
        {
            // Genuine callers keep their family Busy until consuming the old
            // terminal; this also fails closed if a reserved control races it.
            seat.terminal = Some(GatherEnd::Refused {
                token,
                reason: "busy".into(),
            });
            return;
        }
        let selected = self
            .load_identity
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
                .load_identity
                .as_ref()
                .expect("Load identity")
                .named_banks,
        );
        let run = RunKey {
            slot: self.incarnation,
            run: generation,
            session: self.work_epoch,
        };
        let account = String::new();
        match compiled::prepare_run(
            &crate::gatherer::CARD,
            run,
            Arc::clone(settings),
            selected,
            banks,
            retained,
            account,
        ) {
            Ok(job) => {
                self.control_generation = generation;
                seat.dropped_rows = 0;
                seat.reported_drops = 0;
                seat.dropped_owner = None;
                seat.page = Some(Arc::new(GatherPage {
                    token,
                    phase: GatherPhase::Preparing,
                    status: None,
                }));
                seat.gather = GatherSeat::Preparing { token, job };
            }
            Err(error) => {
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
        if matches!(&seat.gather, GatherSeat::Preparing { job, .. } if job.is_finished()) {
            let GatherSeat::Preparing { token, job } = std::mem::take(&mut seat.gather) else {
                unreachable!()
            };
            match job.join() {
                Ok(mut run) => {
                    run.rekey_session(self.work_epoch);
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
        if let GatherSeat::Running { token, run } = &mut seat.gather {
            ctx.compiled.interacts = None;
            let result = {
                let mut retained = self
                    .retained
                    .as_ref()
                    .expect("seat retention")
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                run.tick(ctx, &mut retained, &mut self.native_runtime, None)
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
        if !self.api_owns_foreground() {
            self.log_api_drop_total();
        }
    }

    pub(super) fn interrupt_api(&mut self, event: Interrupt) {
        let Some(seat) = self.api.as_mut() else {
            return;
        };
        let GatherSeat::Running { run, token } = &mut seat.gather else {
            return;
        };
        if let Err(payload) = catch_unwind(AssertUnwindSafe(|| run.script.interrupt(event))) {
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

    pub(super) fn teardown_api(&mut self, reason: StopReason) {
        self.log_api_drop_total();
        let Some(mut seat) = self.api.take() else {
            return;
        };
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

    pub(super) fn record_api_dropped_rows(&mut self, count: usize) {
        if count == 0 {
            return;
        }
        let Some(seat) = self.api.as_mut() else {
            return;
        };
        let Some(owner) = seat.foreground_owner() else {
            return;
        };
        if seat.dropped_rows == seat.reported_drops {
            seat.dropped_owner = Some(owner);
            api::hostlog::emit(
                api::hostlog::Emit {
                    category: api::hostlog::Category::ScriptLifecycle,
                    level: api::hostlog::Level::Warn,
                    slot: None,
                    always_stderr: false,
                },
                format_args!(
                    "{} foreground: dropped {count} script game rows",
                    owner.label()
                ),
            );
        }
        seat.dropped_rows = seat.dropped_rows.saturating_add(count);
    }

    pub(super) fn log_api_drop_total(&mut self) {
        if let Some(seat) = self.api.as_mut() {
            if seat.dropped_rows != seat.reported_drops {
                if let Some(owner) = seat.dropped_owner {
                    let unreported = seat.dropped_rows.saturating_sub(seat.reported_drops);
                    api::hostlog::emit(
                        api::hostlog::Emit {
                            category: api::hostlog::Category::ScriptLifecycle,
                            level: api::hostlog::Level::Warn,
                            slot: None,
                            always_stderr: false,
                        },
                        format_args!(
                            "{} foreground: dropped {unreported} script game rows total",
                            owner.label()
                        ),
                    );
                    seat.reported_drops = seat.dropped_rows;
                    seat.dropped_owner = None;
                }
            }
        }
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
