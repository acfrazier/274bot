//! Compiled registration transaction and shared presentation, owned by SlotScript.
use super::*;
#[cfg(feature = "load")]
use crate::native::HostFrame;
use crate::native::{
    ActionContext, ConfigError, Interrupt, NativeActions, NativeOutput, NativePhase, NativeTick,
    PrepareContext, PreparedConfig, RetainedMemory, Script, ScriptFailure, ScriptFlow,
    ScriptStatus, SettingsApply, SettingsBag, StartError, StopReason,
};
use api::quest_progress::EvidenceStamp;
use api::selected::{FamilyPreparation, RunKey, SelectedPin};
use std::sync::Mutex;

pub(super) struct CompiledRun {
    pub script: ScriptOwner,
    pub config: Arc<PreparedConfig>,
    pub pending: Option<PendingConfig>,
    pub run: RunKey,
    pub selected: Arc<api::game_data::SelectedGameData>,
    pub pin: Arc<SelectedPin>,
    pub output: Output,
    pub actions: NativeActions,
}

pub(super) struct PendingConfig {
    config: Arc<PreparedConfig>,
    boundary: bool,
}

impl PendingConfig {
    pub(super) fn revision(&self) -> u64 {
        self.config.revision()
    }
}

/// Also guards a factory result discarded on its worker after Stop/removal.
/// No callback or destructor may unwind into the host or a reaper.
pub(super) struct ScriptOwner(Option<Box<dyn Script>>);

impl std::ops::Deref for ScriptOwner {
    type Target = dyn Script;
    fn deref(&self) -> &Self::Target {
        self.0.as_deref().expect("live script owner")
    }
}
impl std::ops::DerefMut for ScriptOwner {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.0.as_deref_mut().expect("live script owner")
    }
}
impl ScriptOwner {
    pub(super) fn stop(&mut self, reason: StopReason) -> Option<String> {
        let mut script = self.0.take()?;
        let stopped = catch_unwind(AssertUnwindSafe(|| script.on_stop(reason)));
        let dropped = catch_unwind(AssertUnwindSafe(|| drop(script)));
        stopped
            .err()
            .or_else(|| dropped.err())
            .map(|payload| format!("script teardown panic: {}", panic_message(&payload)))
    }
}
impl Drop for ScriptOwner {
    fn drop(&mut self) {
        let _ = self.stop(StopReason::Replaced);
    }
}

pub(super) struct Preparation {
    generation: u64,
    worker: std::thread::JoinHandle<PreparationResult>,
}

impl Preparation {
    pub(super) fn is_finished(&self) -> bool {
        self.worker.is_finished()
    }

    pub(super) fn join(self) -> Result<CompiledRun, StartError> {
        self.worker
            .join()
            .map_err(|payload| {
                StartError::Unavailable(
                    format!("preparation worker panic: {}", panic_message(&payload)).into(),
                )
            })?
            .0
            .take()
            .expect("preparation result")
    }
}

/// A detached thread packet may be destroyed on either thread. Never let a
/// card-owned configuration destructor unwind through std's packet destructor.
struct PreparationResult(Option<Result<CompiledRun, StartError>>);

impl Drop for PreparationResult {
    fn drop(&mut self) {
        let result = self.0.take();
        let _ = catch_unwind(AssertUnwindSafe(|| drop(result)));
    }
}

/// Settings admission is distinct from persistence and from Load delivery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompiledDelivery {
    Applied,
    PendingBoundary,
    RestartRequired,
    Unchanged,
    Stale,
    NotRunning,
    Rejected(ConfigError),
}

#[derive(Default)]
pub(super) struct Output {
    pub status: Option<Arc<ScriptStatus>>,
    pub paint: Option<Arc<crate::shim::ScriptPaint>>,
    pub applied: Option<u64>,
    pub account: String,
}

fn status_text<'a>(status: &'a ScriptStatus, key: &str) -> Option<&'a str> {
    status.fields.iter().find_map(|field| {
        (field.key == key)
            .then_some(&field.value)
            .and_then(|value| match value {
                crate::native::StatusValue::Text(text) if !text.is_empty() => Some(text.as_ref()),
                _ => None,
            })
    })
}

fn status_text_changed(previous: Option<&ScriptStatus>, status: &ScriptStatus, key: &str) -> bool {
    let current = status_text(status, key);
    match previous {
        Some(previous) => status_text(previous, key) != current,
        None => current.is_some(),
    }
}

fn phase_label(phase: NativePhase) -> &'static str {
    match phase {
        NativePhase::Preparing => "preparing",
        NativePhase::Working => "working",
        NativePhase::Waiting => "waiting",
        NativePhase::Blocked => "blocked",
        NativePhase::Complete => "complete",
    }
}

fn log_status_change(slot: &str, previous: Option<&ScriptStatus>, status: &ScriptStatus) {
    let phase_changed = previous.is_none_or(|previous| previous.phase != status.phase);
    let failure_changed = match previous.and_then(|previous| previous.failure.as_ref()) {
        Some(previous) => status.failure.as_ref() != Some(previous),
        None => status.failure.is_some(),
    };
    let waiting_for_changed = status_text_changed(previous, status, "waiting_for");
    let action_state_changed = status_text_changed(previous, status, "action_state");
    let gatherer = status.card.0 == "Gatherer";
    let last_event_changed = gatherer && status_text_changed(previous, status, "last_event");
    if !(phase_changed
        || failure_changed
        || waiting_for_changed
        || action_state_changed
        || last_event_changed)
    {
        return;
    }

    let level = if phase_changed || failure_changed {
        api::hostlog::Level::Info
    } else {
        api::hostlog::Level::Debug
    };
    let failure = status.failure.as_ref();
    let waiting_for = status_text(status, "waiting_for");
    let action_state = status_text(status, "action_state");
    if gatherer {
        api::host_log!(
            api::hostlog::Category::ScriptLifecycle,
            level,
            slot = slot,
            "native {} phase={} failure={} failure_message={:?} waiting_for={:?} action_state={:?} last_event={:?}",
            status.card.0,
            phase_label(status.phase),
            failure.map_or("none", |failure| failure.code.as_ref()),
            failure.map(|failure| failure.message.as_ref()),
            waiting_for,
            action_state,
            status_text(status, "last_event"),
        );
    } else {
        api::host_log!(
            api::hostlog::Category::ScriptLifecycle,
            level,
            slot = slot,
            "native {} phase={} failure={} failure_message={:?} waiting_for={:?} action_state={:?}",
            status.card.0,
            phase_label(status.phase),
            failure.map_or("none", |failure| failure.code.as_ref()),
            failure.map(|failure| failure.message.as_ref()),
            waiting_for,
            action_state,
        );
    }
}

impl NativeOutput for Output {
    fn status(&mut self, status: ScriptStatus) {
        let previous = self.status.as_deref();
        log_status_change(&self.account, previous, &status);
        if previous != Some(&status) {
            self.status = Some(Arc::new(status));
        }
    }
    fn paint(&mut self, frame: Arc<crate::shim::ScriptPaint>) {
        if self.paint.as_deref() != Some(frame.as_ref()) {
            self.paint = Some(frame);
        }
    }
    fn log(&mut self, level: api::hostlog::Level, message: &str) {
        if self.account.is_empty() {
            api::hostlog::emit(
                api::hostlog::Emit {
                    category: api::hostlog::Category::ScriptLifecycle,
                    level,
                    slot: None,
                    always_stderr: false,
                },
                format_args!("{message}"),
            );
            return;
        }
        api::hostlog::record(&api::hostlog::Record {
            slot: Some(&self.account),
            tick: api::hostlog::slot_tick(&self.account),
            source: api::hostlog::Source::Script,
            level,
            message,
        });
    }
    fn settings_applied(&mut self, revision: u64) {
        self.applied = Some(revision);
    }
}

/// One frame's native authority/evidence for compiled runs and API reads.
pub(super) fn frame_context<'a>(
    ctx: &'a ScriptCtx<'_>,
    run: RunKey,
    pin: &'a SelectedPin,
    retained: &'a mut RetainedMemory,
    runtime: &'a mut crate::native::ledger::Runtime,
) -> ActionContext<'a> {
    let evidence = EvidenceStamp {
        run,
        tick: ctx.tick,
        sequence: ctx.tick,
    };
    let now = Instant::now();
    runtime.budget.observe(ctx.tick);
    ActionContext {
        evidence,
        pin,
        snapshot: api::snapshot::SnapshotView::new(ctx.snapshot, evidence)
            .with_reach(ctx.compiled.reach),
        retained,
        action_id: 0,
        active_now: runtime.clock.now(now),
        wall_now: now,
        ledger: &mut runtime.ledger,
        budget: &mut runtime.budget,
        eligible: !ctx.compiled.hold,
        observed_walk_outcome_seq: runtime.observed_walk_outcome_seq,
    }
}

impl CompiledRun {
    fn latest_config(&self) -> &PreparedConfig {
        self.pending
            .as_ref()
            .map_or(&self.config, |pending| &pending.config)
    }

    fn sync_settings_status(&mut self) {
        let Some(status) = self.output.status.as_mut() else {
            return;
        };
        let pending = self.pending.as_ref().map(PendingConfig::revision);
        if status.active_settings != self.config.revision() || status.pending_settings != pending {
            let status = Arc::make_mut(status);
            status.active_settings = self.config.revision();
            status.pending_settings = pending;
        }
    }

    /// A session boundary keeps the run and advances only its session, so
    /// stale-evidence fences see the reconnect. Controls re-read the key.
    pub(super) fn rekey_session(&mut self, session: u64) {
        self.run.session = session;
        if let Some(status) = self.output.status.as_mut() {
            if status.run != self.run {
                Arc::make_mut(status).run = self.run;
            }
        }
    }

    pub(super) fn tick(
        &mut self,
        ctx: &mut ScriptCtx<'_>,
        retained: &mut RetainedMemory,
        runtime: &mut crate::native::ledger::Runtime,
    ) -> Result<ScriptFlow, ScriptFailure> {
        #[cfg(feature = "load")]
        let interacts = ctx.compiled.interacts.take();
        let result = {
            let mut tick = NativeTick {
                actions: &mut self.actions,
                cx: frame_context(ctx, self.run, &self.pin, retained, runtime),
                output: &mut self.output,
                pairs: None,
                #[cfg(feature = "load")]
                frame: HostFrame {
                    here: ctx.here,
                    snapshot: ctx.snapshot,
                    obj_names: ctx.obj_names,
                    compiled: crate::CompiledTick {
                        selected: Some(&self.selected),
                        reach: ctx.compiled.reach,
                        hold: ctx.compiled.hold,
                        #[cfg(feature = "load")]
                        interacts,
                    },
                },
            };
            // Take the queue back even after a card panic, then let SlotScript
            // revoke/discard it before the host can drain any partial effects.
            let result = catch_unwind(AssertUnwindSafe(|| self.script.tick(&mut tick)));
            #[cfg(feature = "load")]
            {
                ctx.compiled.interacts = tick.frame.compiled.interacts.take();
            }
            result
        };
        if let Some(revision) = self.output.applied.take() {
            if self
                .pending
                .as_ref()
                .is_some_and(|next| next.boundary && next.revision() == revision)
            {
                self.config = self
                    .pending
                    .take()
                    .expect("matching pending revision")
                    .config;
            }
        }
        self.sync_settings_status();
        match result {
            Ok(flow) => flow,
            Err(payload) => Err(ScriptFailure {
                code: "panic".into(),
                message: format!("script panic: {}", panic_message(&payload)).into(),
                retryable: false,
            }),
        }
    }
}

/// Watchdog recreation starts from the effective revision. An accepted
/// boundary revision is then offered to the new instance through its
/// configure receiver, exactly as a live edit would be; its answer decides.
/// A restart-required revision stays pending for an operator restart. A
/// re-offer the new instance refuses is dropped visibly, never activated.
fn reoffer_pending(
    script: &mut ScriptOwner,
    output: &mut Output,
    effective: Arc<PreparedConfig>,
    pending: Option<PendingConfig>,
) -> Result<(Arc<PreparedConfig>, Option<PendingConfig>), StartError> {
    let pending = match pending {
        Some(pending) if pending.boundary => pending,
        other => return Ok((effective, other)),
    };
    match catch_unwind(AssertUnwindSafe(|| {
        script.configure(Arc::clone(&pending.config))
    })) {
        Ok(Ok(SettingsApply::Applied)) => Ok((pending.config, None)),
        Ok(Ok(SettingsApply::PendingBoundary)) => Ok((effective, Some(pending))),
        Ok(Ok(SettingsApply::RestartRequired)) => Ok((
            effective,
            Some(PendingConfig {
                boundary: false,
                ..pending
            }),
        )),
        Ok(Err(error)) => {
            output.log(
                api::hostlog::Level::Warn,
                &format!(
                    "settings revision {} dropped on watchdog restart: {} ({})",
                    pending.revision(),
                    error.message,
                    error.code
                ),
            );
            Ok((effective, None))
        }
        Err(payload) => Err(StartError::Unavailable(
            format!("configure panic: {}", panic_message(&payload)).into(),
        )),
    }
}

/// Prepare only; callers invoke this on a FamilyPreparation worker and never
/// persist/apply a draft before this returns a validated card-owned payload.
pub fn prepare_config(
    worker: &mut FamilyPreparation,
    id: crate::CompiledId,
    revision: u64,
    bag: Arc<SettingsBag>,
    selected: Arc<api::game_data::SelectedGameData>,
    banks: Arc<api::named_banks::NamedBankFacts>,
) -> Result<Arc<PreparedConfig>, StartError> {
    let card = crate::compiled_card(id)
        .ok_or_else(|| StartError::Unavailable(format!("not ported: {}", id.0).into()))?;
    let pin = selected.selected_pin().map_err(StartError::Facts)?;
    let mut cx = PrepareContext {
        selected,
        pin,
        banks,
        families: worker,
    };
    let config = catch_unwind(AssertUnwindSafe(|| (card.prepare)(&mut cx, revision, bag)))
        .map_err(|payload| {
            StartError::Unavailable(
                format!("preparation panic: {}", panic_message(&payload)).into(),
            )
        })??;
    if config.card() != card.id
        || config.schema_version() != card.schema_version
        || config.revision() != revision
        || revision == 0
    {
        return Err(StartError::Config(ConfigError::new(
            "",
            "prepared-identity",
            "preparer returned a different card/schema/revision",
        )));
    }
    Ok(config)
}

/// The single off-pump registration transaction used by Browse and API seats.
pub(super) fn prepare_run(
    card: &'static crate::native::CompiledCard,
    run: RunKey,
    bag: Arc<SettingsBag>,
    selected: Arc<api::game_data::SelectedGameData>,
    banks: Arc<api::named_banks::NamedBankFacts>,
    retained: Arc<Mutex<RetainedMemory>>,
    account: String,
) -> Result<Preparation, StartError> {
    let worker = FamilyPreparation::run(move |worker| {
        PreparationResult(Some((|| {
            let config = prepare_config(worker, card.id, 1, bag, Arc::clone(&selected), banks)?;
            let pin = selected.selected_pin().map_err(StartError::Facts)?;
            let mut retained = retained
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let script = catch_unwind(AssertUnwindSafe(|| {
                (card.create)(run, Arc::clone(&config), &mut retained)
            }))
            .map_err(|payload| {
                StartError::Unavailable(
                    format!("factory panic: {}", panic_message(&payload)).into(),
                )
            })??;
            Ok(CompiledRun {
                script: ScriptOwner(Some(script)),
                config,
                pending: None,
                run,
                selected,
                pin,
                output: Output {
                    account,
                    ..Default::default()
                },
                actions: NativeActions { _private: () },
            })
        })()))
    })
    .map_err(|error| StartError::Unavailable(error.to_string().into()))?;
    Ok(Preparation {
        generation: run.run,
        worker,
    })
}

pub(super) fn destroy_run(mut run: Box<CompiledRun>, reason: StopReason) -> Option<String> {
    let stopped = run.script.stop(reason);
    let dropped = catch_unwind(AssertUnwindSafe(|| drop(run)));
    stopped.or_else(|| {
        dropped
            .err()
            .map(|payload| format!("script drop panic: {}", panic_message(&payload)))
    })
}

impl SlotScript {
    /// Bind once to the owning host worker lifetime. A replaced profile gets a
    /// different incarnation even when its visible name is reused.
    pub fn bind_incarnation(&mut self, incarnation: u64) {
        assert!(incarnation != 0 && self.incarnation == 0 && !self.has_instance());
        self.incarnation = incarnation;
    }

    pub fn start_compiled(
        &mut self,
        account: &str,
        id: crate::CompiledId,
        bag: Arc<SettingsBag>,
        selected: Arc<api::game_data::SelectedGameData>,
        banks: Arc<api::named_banks::NamedBankFacts>,
    ) -> Result<(), StartError> {
        if !matches!(self.state, RunState::Idle | RunState::Error) || self.load_active() {
            return Err(StartError::Busy);
        }
        if self.incarnation == 0 {
            return Err(StartError::Unavailable("slot worker is not bound".into()));
        }
        let card = crate::compiled_card(id)
            .ok_or_else(|| StartError::Unavailable(format!("not ported: {}", id.0).into()))?;
        let generation = self
            .control_generation
            .max(self.runtime_generation)
            .checked_add(1)
            .ok_or_else(|| StartError::Unavailable("run generation exhausted".into()))?;
        let run = RunKey {
            slot: self.incarnation,
            run: generation,
            session: self.work_epoch,
        };
        let retained = Arc::clone(
            self.retained
                .get_or_insert_with(|| Arc::new(Mutex::new(RetainedMemory::default()))),
        );
        let job = prepare_run(
            card,
            run,
            bag,
            selected,
            banks,
            retained,
            account.to_owned(),
        )?;
        self.control_generation = generation;
        self.preparing = Some(Box::new(job));
        self.want_run = true;
        self.start_pending = true;
        self.start_outcome = None;
        self.state = RunState::Starting;
        Ok(())
    }

    pub(super) fn restart_compiled(&mut self, now: Instant) -> Result<(), String> {
        let current = self.compiled.as_ref().ok_or("no compiled identity")?;
        let card =
            crate::compiled_card(current.config.card()).ok_or("compiled card unavailable")?;
        let generation = self
            .control_generation
            .max(self.runtime_generation)
            .checked_add(1)
            .ok_or("run generation exhausted")?;
        let run = RunKey {
            slot: self.incarnation,
            run: generation,
            session: self.work_epoch,
        };
        // Only the accepted configuration is effective; pending edits must not
        // become active merely because the watchdog recreates the instance.
        let config = Arc::clone(&current.config);
        // An accepted pending revision is not dropped: the new instance is
        // offered it again below, as a settings edit would offer it.
        let pending = current.pending.as_ref().map(|pending| PendingConfig {
            config: Arc::clone(&pending.config),
            boundary: pending.boundary,
        });
        let selected = Arc::clone(&current.selected);
        let pin = Arc::clone(&current.pin);
        let account = current.output.account.clone();
        let retained = Arc::clone(self.retained.as_ref().ok_or("no retained memory")?);
        self.revoke_native_input();
        self.teardown_compiled(StopReason::Replaced);
        let worker = FamilyPreparation::run(move |_| {
            PreparationResult(Some((|| {
                let mut retained = retained
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                let script = catch_unwind(AssertUnwindSafe(|| {
                    (card.create)(run, Arc::clone(&config), &mut retained)
                }))
                .map_err(|payload| {
                    StartError::Unavailable(
                        format!("factory panic: {}", panic_message(&payload)).into(),
                    )
                })??;
                let mut script = ScriptOwner(Some(script));
                let mut output = Output {
                    account,
                    ..Default::default()
                };
                let (config, pending) = reoffer_pending(&mut script, &mut output, config, pending)?;
                Ok(CompiledRun {
                    script,
                    config,
                    pending,
                    run,
                    selected,
                    pin,
                    output,
                    actions: NativeActions { _private: () },
                })
            })()))
        });
        let worker = match worker {
            Ok(worker) => worker,
            Err(error) => {
                let message = error.to_string();
                self.fail_compiled(ScriptFailure {
                    code: "restart-worker".into(),
                    message: message.clone().into(),
                    retryable: false,
                });
                return Err(message);
            }
        };
        self.control_generation = generation;
        self.preparing = Some(Box::new(Preparation { generation, worker }));
        self.state = RunState::Starting;
        self.watchdog.on_restart_applied(now);
        Ok(())
    }

    fn poll_compiled(&mut self) {
        if !self.preparing.as_ref().is_some_and(|job| job.is_finished()) {
            return;
        }
        let job = self.preparing.take().expect("finished preparation");
        if job.generation != self.control_generation {
            // Joining also avoids leaving a finished packet to JoinHandle::drop.
            let _ = job.join();
            return;
        }
        match job.join() {
            Ok(run) => {
                self.install_compiled(run);
                self.settle_start(StartOutcome::Ready);
            }
            Err(error) => {
                self.last_error = Some(error.to_string());
                self.want_run = false;
                self.state = RunState::Idle;
                self.settle_start(StartOutcome::Rejected(error));
            }
        }
    }

    fn install_compiled(&mut self, run: CompiledRun) {
        if self.watchdog.state() == crate::WatchdogState::Idle {
            self.watchdog.arm_fresh(Instant::now());
        }
        self.source_identity = Some(crate::compiled_identity_key(run.config.card()));
        self.runtime_generation = run.run.run;
        self.compiled = Some(Box::new(run));
        self.last_error = None;
        self.lifecycle_receipt = None;
        self.ticks = 0;
        self.pending_withdraw_x = None;
        self.withdraw_x_result_seq = 0;
        self.withdraw_x_result = false;
        self.withdraw_load_result_seq = 0;
        self.withdraw_load_result = false;
        self.pending_bank_op = None;
        self.bank_op_result_seq = 0;
        self.bank_op_result = false;
        self.last_settings_fp = None;
        #[cfg(feature = "load")]
        {
            self.compiled_interacts.clear();
            self.compiled_interact_outcome_seqs.clear();
            self.active_tick_error_generation = None;
        }
        self.state = if self.want_run {
            RunState::Running
        } else {
            RunState::Paused
        };
        if self.want_run {
            self.native_input.publish_live();
        }
    }

    pub fn observe_lifecycle(&mut self) {
        self.poll_compiled();
        #[cfg(feature = "load")]
        self.observe_load_lifecycle();
    }

    pub fn native_run(&self) -> Option<RunKey> {
        self.compiled.as_ref().map(|run| run.run)
    }
    pub fn native_status(&self) -> Option<Arc<ScriptStatus>> {
        let status = self
            .compiled
            .as_ref()
            .and_then(|run| run.output.status.clone());
        #[cfg(feature = "load")]
        let status = status.or_else(|| self.api_status());
        status
    }

    pub fn native_settings_revision(&self) -> Option<u64> {
        self.compiled
            .as_ref()
            .map(|run| run.latest_config().revision())
    }

    pub fn control_key(&self) -> (u64, u64) {
        (
            self.incarnation,
            self.control_generation.max(self.runtime_generation),
        )
    }

    pub fn configure_compiled(
        &mut self,
        config: Arc<PreparedConfig>,
        target: RunKey,
    ) -> CompiledDelivery {
        if self.native_run() != Some(target) {
            return CompiledDelivery::Stale;
        }
        if !matches!(self.state, RunState::Running | RunState::Paused) {
            return CompiledDelivery::NotRunning;
        }
        let Some(run) = self.compiled.as_mut() else {
            return CompiledDelivery::NotRunning;
        };
        if config.card() != run.config.card()
            || config.schema_version() != run.config.schema_version()
        {
            return CompiledDelivery::Rejected(ConfigError::new(
                "",
                "config-identity",
                "configuration belongs to a different card/schema",
            ));
        }
        let latest = run.latest_config();
        if config.revision() <= latest.revision() {
            return CompiledDelivery::Stale;
        }
        if config.bag() == latest.bag() {
            return CompiledDelivery::Unchanged;
        }
        match catch_unwind(AssertUnwindSafe(|| {
            run.script.configure(Arc::clone(&config))
        })) {
            Ok(Ok(apply)) => {
                let delivery = match apply {
                    SettingsApply::Applied => {
                        run.config = config;
                        run.pending = None;
                        CompiledDelivery::Applied
                    }
                    SettingsApply::PendingBoundary | SettingsApply::RestartRequired => {
                        let boundary = apply == SettingsApply::PendingBoundary;
                        run.pending = Some(PendingConfig { config, boundary });
                        if boundary {
                            CompiledDelivery::PendingBoundary
                        } else {
                            CompiledDelivery::RestartRequired
                        }
                    }
                };
                run.sync_settings_status();
                delivery
            }
            Ok(Err(error)) => CompiledDelivery::Rejected(error),
            Err(payload) => {
                let error = ConfigError::new("", "configure-panic", panic_message(&payload));
                self.fail_compiled(ScriptFailure {
                    code: error.code.clone(),
                    message: error.message.clone(),
                    retryable: false,
                });
                CompiledDelivery::Rejected(error)
            }
        }
    }

    pub(super) fn interrupt_compiled(&mut self, event: Interrupt) {
        let Some(run) = self.compiled.as_mut() else {
            return;
        };
        if let Err(payload) = catch_unwind(AssertUnwindSafe(|| run.script.interrupt(event))) {
            self.fail_compiled(ScriptFailure {
                code: "interrupt-panic".into(),
                message: panic_message(&payload).into(),
                retryable: false,
            });
        }
    }

    pub(super) fn teardown_compiled(&mut self, reason: StopReason) {
        self.native_runtime.revoke();
        if let Some(run) = self.compiled.take() {
            if let Some(error) = destroy_run(run, reason) {
                self.last_error = Some(error);
            }
        }
    }

    pub(super) fn fail_compiled(&mut self, failure: ScriptFailure) {
        self.revoke_native_input();
        #[cfg(feature = "load")]
        {
            self.compiled_interacts.clear();
            self.compiled_interact_outcome_seqs.clear();
            self.clue_abort_owed = true;
        }
        self.pending_logs.push(failure.message.to_string());
        self.last_error = Some(failure.message.to_string());
        self.lifecycle_receipt = Some(ScriptLifecycleReceipt {
            runtime_generation: self.runtime_generation,
            state: ScriptTerminalState::Failed,
            tick: self.ticks,
            reason: failure.message.to_string(),
        });
        self.state = RunState::Error;
        self.want_run = false;
        self.teardown_compiled(StopReason::Error);
        self.watchdog.cancel_clear();
    }

    pub fn retry_compiled(&mut self, target: RunKey) -> Result<(), ScriptFailure> {
        let refused = || ScriptFailure {
            code: "retry-refused".into(),
            message: "stale or non-retryable run".into(),
            retryable: false,
        };
        let run = self
            .compiled
            .as_mut()
            .filter(|run| run.run == target)
            .ok_or_else(refused)?;
        if !run.output.status.as_ref().is_some_and(|status| {
            status.phase == NativePhase::Blocked
                && status
                    .failure
                    .as_ref()
                    .is_some_and(|failure| failure.retryable)
        }) {
            return Err(refused());
        }
        match catch_unwind(AssertUnwindSafe(|| run.script.retry())) {
            Ok(result) => result?,
            Err(payload) => {
                let failure = ScriptFailure {
                    code: "retry-panic".into(),
                    message: panic_message(&payload).into(),
                    retryable: false,
                };
                self.fail_compiled(failure.clone());
                return Err(failure);
            }
        }
        let status = Arc::make_mut(run.output.status.as_mut().expect("blocked status"));
        status.phase = NativePhase::Waiting;
        status.failure = None;
        Ok(())
    }

    /// Same run/session fence as other native controls. A successful request
    /// reopens Blocked dispatch so the script's retry-with-read can run.
    /// Working requests remain boundary-latched; this command itself never
    /// interrupts an owned dialogue or emits a game verb.
    pub fn read_journal_compiled(&mut self, target: RunKey) -> Result<(), ScriptFailure> {
        let run = self
            .compiled
            .as_mut()
            .filter(|run| run.run == target)
            .ok_or_else(|| ScriptFailure {
                code: "read-journal-refused".into(),
                message: "stale native run".into(),
                retryable: false,
            })?;
        match catch_unwind(AssertUnwindSafe(|| run.script.read_journal())) {
            Ok(result) => {
                result?;
                if let Some(status) = run.output.status.as_mut() {
                    if status.phase == NativePhase::Blocked {
                        let status = Arc::make_mut(status);
                        status.phase = NativePhase::Waiting;
                        status.failure = None;
                    }
                }
                Ok(())
            }
            Err(payload) => {
                let failure = ScriptFailure {
                    code: "read-journal-panic".into(),
                    message: panic_message(&payload).into(),
                    retryable: false,
                };
                self.fail_compiled(failure.clone());
                Err(failure)
            }
        }
    }

    /// Behavioral fixtures use the same compiled instance/tick path, without
    /// adding test cards or constructors to the production registry.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn start_test_script(
        &mut self,
        script: Box<dyn Script>,
        selected: Option<Arc<api::game_data::SelectedGameData>>,
    ) -> Result<(), String> {
        if !matches!(self.state, RunState::Idle | RunState::Error) || self.load_active() {
            return Err(StartError::Busy.to_string());
        }
        let selected = match selected {
            Some(selected) => selected,
            None => api::game_data::for_revision(api::selected::ClientRevision::R289)?,
        };
        let pin = selected.selected_pin().map_err(|e| format!("{e:?}"))?;
        self.control_generation = self
            .control_generation
            .max(self.runtime_generation)
            .checked_add(1)
            .ok_or("generation exhausted")?;
        self.retained
            .get_or_insert_with(|| Arc::new(Mutex::new(RetainedMemory::default())));
        self.want_run = true;
        self.install_compiled(CompiledRun {
            script: ScriptOwner(Some(script)),
            config: PreparedConfig::new(
                crate::CompiledId("test"),
                1,
                1,
                Arc::new(SettingsBag::new()),
                (),
            ),
            pending: None,
            run: RunKey {
                slot: self.incarnation,
                run: self.control_generation,
                session: self.work_epoch,
            },
            selected,
            pin,
            output: Output::default(),
            actions: NativeActions { _private: () },
        });
        Ok(())
    }
}

#[cfg(test)]
#[derive(Default)]
pub(crate) struct TestOutput {
    pub statuses: Vec<ScriptStatus>,
    pub paints: Vec<Arc<crate::shim::ScriptPaint>>,
    pub logs: Vec<String>,
}
#[cfg(test)]
impl NativeOutput for TestOutput {
    fn status(&mut self, status: ScriptStatus) {
        self.statuses.push(status);
    }
    fn paint(&mut self, paint: Arc<crate::shim::ScriptPaint>) {
        self.paints.push(paint);
    }
    fn log(&mut self, _level: api::hostlog::Level, message: &str) {
        self.logs.push(message.into());
    }
    fn settings_applied(&mut self, _revision: u64) {}
}

#[cfg(all(test, feature = "load"))]
#[path = "compiled_tests.rs"]
mod tests;
