//! Compiled registration transaction and shared presentation, owned by SlotScript.
use super::*;
use crate::native::HostFrame;
use crate::native::{
    ActionContext, ConfigError, Interrupt, NativeActions, NativeOutput, NativePhase, NativeTick,
    PrepareContext, PreparedConfig, RetainedMemory, Script, ScriptFailure, ScriptFlow,
    ScriptStatus, SettingsApply, SettingsBag, StartError, StopReason,
};
use crate::preparation::{self, PreparationInput, PreparationKind, PreparedTask};
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
    pending_registry: Option<Box<crate::quester::registry::PreparedRegistry>>,
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
    pub(super) fn stop_with_output(
        &mut self,
        reason: StopReason,
        output: &mut dyn NativeOutput,
    ) -> Option<String> {
        let mut script = self.0.take()?;
        let stopped = catch_unwind(AssertUnwindSafe(|| {
            script.on_stop_with_output(reason, output)
        }));
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

pub(super) enum PrepareRunError {
    Full,
    Other(StartError),
}

impl From<StartError> for PrepareRunError {
    fn from(error: StartError) -> Self {
        Self::Other(error)
    }
}

/// One operator Start retained while the bounded work ledger is temporarily
/// full. The slot owns exactly one demand and retries on an ordinary turn.
pub(super) struct PendingCompiledStart {
    card: &'static crate::native::CompiledCard,
    run: RunKey,
    bag: Arc<SettingsBag>,
    selected: Arc<api::game_data::SelectedGameData>,
    banks: Arc<api::named_banks::NamedBankFacts>,
    retained: Arc<Mutex<RetainedMemory>>,
    account: String,
}

impl PendingCompiledStart {
    fn try_admit(&self) -> Result<Preparation, PrepareRunError> {
        prepare_run(
            self.card,
            self.run,
            Arc::clone(&self.bag),
            Arc::clone(&self.selected),
            Arc::clone(&self.banks),
            Arc::clone(&self.retained),
            self.account.clone(),
        )
    }
}
pub(super) struct Preparation {
    generation: u64,
    worker: PreparedTask<PreparationResult>,
}

impl Preparation {
    pub(super) fn is_finished(&self) -> bool {
        self.worker.is_finished()
    }

    pub(super) fn join(self) -> Result<CompiledRun, StartError> {
        self.worker
            .join()
            .map_err(|error| StartError::Unavailable(error.to_string().into()))?
            .0
            .take()
            .expect("preparation result")
    }
}

/// An unconsumed WorkAdmission result may be destroyed on the service
/// worker. Never let a card-owned configuration destructor unwind there.
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
    pub applied: Option<std::num::NonZeroU64>,
    pub account: Option<Box<str>>,
}
struct StatusDetails<'a> {
    failure_message: Option<&'a str>,
    waiting_for: Option<&'a str>,
    action_state: Option<&'a str>,
    method: Option<&'a str>,
    area: Option<&'a str>,
    target: Option<&'a str>,
    last_event: Option<&'a str>,
}

fn distinct_status_field<'a>(value: Option<&'a str>, occupied: &[&str]) -> Option<&'a str> {
    value.filter(|value| {
        !value.is_empty()
            && *value != "—"
            && occupied
                .iter()
                .all(|text| !status_text_has_field(text, value))
    })
}

/// True when `value` is already a whole field or `key=value` token in `text`,
/// not merely a substring of some other word.
fn status_text_has_field(text: &str, value: &str) -> bool {
    if text == value {
        return true;
    }
    text.split(';').any(|segment| {
        let segment = segment.trim();
        if segment == value {
            return true;
        }
        segment.split_once('=').is_some_and(|(key, rhs)| {
            let key = key.trim();
            !key.is_empty()
                && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                && rhs.trim() == value
        })
    })
}

impl std::fmt::Display for StatusDetails<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(value) = self.failure_message {
            write!(f, " failure_message={value}")?;
        }
        if let Some(value) = self.waiting_for {
            write!(f, " waiting_for={value}")?;
        }
        if let Some(value) = self.action_state {
            write!(f, " action_state={value}")?;
        }
        let occupied = [
            self.failure_message.unwrap_or(""),
            self.last_event.unwrap_or(""),
        ];
        if let Some(value) = distinct_status_field(self.method, &occupied) {
            write!(f, " method={value}")?;
        }
        if let Some(value) = distinct_status_field(self.area, &occupied) {
            write!(f, " area={value}")?;
        }
        if let Some(value) = distinct_status_field(self.target, &occupied) {
            write!(f, " target={value}")?;
        }
        if let Some(value) = self.last_event {
            write!(f, " last_event={value}")?;
        }
        Ok(())
    }
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
    let last_event = if gatherer {
        status_text(status, "last_event")
    } else {
        None
    };
    let last_event_changed = gatherer && status_text_changed(previous, status, "last_event");
    let waiting_or_blocked_event = last_event_changed
        && (matches!(status.phase, NativePhase::Waiting | NativePhase::Blocked)
            || last_event.is_some_and(|event| event.starts_with("waiting")));
    if !(phase_changed
        || failure_changed
        || waiting_for_changed
        || action_state_changed
        || last_event_changed)
    {
        return;
    }

    let level = if phase_changed || failure_changed || waiting_or_blocked_event {
        api::hostlog::Level::Info
    } else {
        api::hostlog::Level::Debug
    };
    let failure = status.failure.as_ref();
    let waiting_for = status_text(status, "waiting_for");
    let action_state = status_text(status, "action_state");
    let details = StatusDetails {
        failure_message: failure.map(|failure| failure.message.as_ref()),
        waiting_for,
        action_state,
        method: if gatherer {
            status_text(status, "method")
        } else {
            None
        },
        area: if gatherer {
            status_text(status, "area")
        } else {
            None
        },
        target: if gatherer {
            status_text(status, "target")
        } else {
            None
        },
        last_event,
    };
    api::host_log!(
        api::hostlog::Category::ScriptLifecycle,
        level,
        slot = slot,
        "native {} phase={} failure={}{}",
        status.card.0,
        phase_label(status.phase),
        failure.map_or("none", |failure| failure.code.as_ref()),
        details,
    );
}

impl NativeOutput for Output {
    fn status(&mut self, status: ScriptStatus) {
        let previous = self.status.as_deref();
        log_status_change(self.account.as_deref().unwrap_or(""), previous, &status);
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
        let account = self.account.as_deref().unwrap_or("");
        if account.is_empty() {
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
            slot: Some(account),
            tick: api::hostlog::slot_tick(account),
            source: api::hostlog::Source::Script,
            level,
            message,
        });
    }
    fn settings_applied(&mut self, revision: u64) {
        self.applied = std::num::NonZeroU64::new(revision);
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
    runtime.budget.observe_frame(ctx.tick, ctx.snapshot);
    ActionContext {
        evidence,
        pin,
        snapshot: api::snapshot::SnapshotView::new(ctx.snapshot, evidence)
            .with_reach(ctx.compiled.reach)
            .with_bank_memory(ctx.compiled.bank_memory)
            .with_world_members(ctx.compiled.world_members),
        condition: ctx.compiled.condition.filter(|fact| {
            fact.session == api::condition::SessionMark::Live
                && ctx
                    .snapshot
                    .is_some_and(|snapshot| snapshot.tick() == fact.tick)
        }),
        poison_unknown_pulse: ctx.compiled.poison_unknown_pulse.unwrap_or(u8::MAX),
        cure_poison: ctx.compiled.cure_poison,
        retained,
        action_id: 0,
        active_now: runtime.clock.now(now).as_duration(),
        wall_now: now,
        ledger: &mut runtime.ledger,
        pending_note_progress: Some(&mut runtime.pending_note_progress),
        budget: &mut runtime.budget,
        eligible: !ctx.compiled.hold,
        observed_walk_outcome_seq: runtime.observed_walk_outcome.seq(),
        observed_walk_cancel_reason: if runtime.observed_walk_outcome.seq()
            == runtime.takeover_consumed_seq
        {
            crate::native::WalkCancelReason::None
        } else {
            runtime.observed_walk_outcome.cancel_reason()
        },
    }
}

/// Borrowed, owner-controlled seat capabilities passed to one compiled tick.
pub struct SeatTickPorts<'a> {
    pub(crate) authority: Option<crate::native::ServiceAuthority<'a>>,
    pub(crate) prompt: Option<&'a mut dyn crate::native::SeatPromptPort>,
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

    pub(super) fn tick<'tick>(
        &'tick mut self,
        ctx: &'tick mut ScriptCtx<'_>,
        retained: &'tick mut RetainedMemory,
        runtime: &'tick mut crate::native::ledger::Runtime,
        pairs: Option<&'tick dyn crate::quester::pair::QuestPairPort>,
        state: Option<&'tick crate::local_state::SlotState>,
        seat_ports: SeatTickPorts<'tick>,
    ) -> (
        Result<ScriptFlow, ScriptFailure>,
        Option<Vec<crate::shim::InteractReq>>,
    ) {
        if let Some(port) = pairs {
            let dead = crate::native::death::hitpoints_zero(
                ctx.snapshot.map(api::snapshot::GameSnapshot::stats),
            );
            if dead {
                port.invalidate(self.run);
            }
            let ready = ctx
                .snapshot
                .is_some_and(api::snapshot::GameSnapshot::ingame)
                && !dead
                && !ctx.compiled.hold;
            let evidence = EvidenceStamp {
                run: self.run,
                tick: ctx.tick,
                sequence: ctx.tick,
            };
            let binding = self.script.pair_binding();
            let inventory = binding
                .and_then(|_| api::snapshot::SnapshotView::new(ctx.snapshot, evidence).inventory());
            port.observe(
                crate::quester::pair::PairRegistration {
                    run: self.run,
                    pin: Arc::clone(&self.pin),
                    settings: self.script.pair_settings(),
                    ready,
                    evidence,
                },
                crate::quester::pair::PairFrame { binding, inventory },
            );
        }
        let interacts = ctx.compiled.interacts.take();
        let result = {
            let mut tick = NativeTick {
                actions: &mut self.actions,
                cx: frame_context(ctx, self.run, &self.pin, retained, runtime),
                output: &mut self.output,
                pairs,
                state,
                service_authority: seat_ports.authority,
                prompt_port: seat_ports.prompt,
                walk_danger: ctx.compiled.walk_danger,
                frame: HostFrame {
                    here: ctx.here,
                    snapshot: ctx.snapshot,
                    obj_names: ctx.obj_names,
                    compiled: crate::CompiledTick {
                        selected: Some(&self.selected),
                        reach: ctx.compiled.reach,
                        reach_flood: ctx.compiled.reach_flood,
                        bank_memory: ctx.compiled.bank_memory,
                        condition: ctx.compiled.condition,
                        poison_unknown_pulse: ctx.compiled.poison_unknown_pulse,
                        cure_poison: ctx.compiled.cure_poison,
                        world_members: ctx.compiled.world_members,
                        collision: ctx.compiled.collision,
                        walk_danger: ctx.compiled.walk_danger,
                        hold: ctx.compiled.hold,
                        service_gate: ctx.compiled.service_gate,
                        interacts,
                    },
                },
            };
            // Take the queue back even after a card panic, then let SlotScript
            // revoke/discard it before the host can drain any partial effects.
            let result = catch_unwind(AssertUnwindSafe(|| self.script.tick(&mut tick)));
            let interacts = tick.frame.compiled.interacts.take();
            (result, interacts)
        };
        let (result, interacts) = result;
        let flow = match result {
            Ok(flow) => flow,
            Err(payload) => Err(ScriptFailure {
                code: "panic".into(),
                message: format!("script panic: {}", panic_message(&payload)).into(),
            }),
        };
        (flow, interacts)
    }
    /// Complete the card tick after its seat borrow has ended. This includes
    /// panic results: the card may have accepted a boundary revision before
    /// it threw, and the status must still reflect that acceptance.
    pub(super) fn finish_tick(&mut self) {
        if let Some(revision) = self.output.applied.take() {
            if self
                .pending
                .as_ref()
                .is_some_and(|next| next.boundary && next.revision() == revision.get())
            {
                self.config = self
                    .pending
                    .take()
                    .expect("matching pending revision")
                    .config;
            }
        }
        self.sync_settings_status();
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
    quester: Option<crate::quester::card::QuesterRegistryReady>,
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
    let config = catch_unwind(AssertUnwindSafe(|| {
        if card.id == crate::quester::card::CARD.id {
            let ready = quester.ok_or_else(|| {
                StartError::Unavailable("Quester registry preparation is missing".into())
            })?;
            crate::quester::card::prepare_with_registry(&mut cx, revision, bag, ready)
        } else if quester.is_some() {
            Err(StartError::Unavailable(
                "unexpected Quester registry for another card".into(),
            ))
        } else {
            (card.prepare)(&mut cx, revision, bag)
        }
    }))
    .map_err(|payload| {
        StartError::Unavailable(format!("preparation panic: {}", panic_message(&payload)).into())
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

/// WorkAdmission never waits for a slot-owned lock. In normal slot
/// sequencing, the factory runs between ticks and this lock is free.
fn try_retained(
    retained: &Mutex<RetainedMemory>,
) -> Result<std::sync::MutexGuard<'_, RetainedMemory>, StartError> {
    match retained.try_lock() {
        Ok(value) => Ok(value),
        Err(std::sync::TryLockError::Poisoned(poisoned)) => Ok(poisoned.into_inner()),
        Err(std::sync::TryLockError::WouldBlock) => Err(StartError::Unavailable(
            "retained memory is still in use during preparation".into(),
        )),
    }
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
) -> Result<Preparation, PrepareRunError> {
    let quester = card.id == crate::quester::card::CARD.id;
    let cold_gathering =
        card.id == crate::CompiledId("Gatherer") && selected.try_gathering().is_none();
    let settings_input =
        PreparationInput::for_settings(run.run, &bag).map_err(StartError::WorkRefused)?;
    // Charge folder retention only for an enabled source. Keep that exact
    // source for the worker; a later settings change fails the publish fence.
    // The Quester-only state lives in its Stage variant. Other cards pay no
    // per-profile registry allocation or pointer in their admission closure.
    struct QuesterState {
        bag: Option<Arc<SettingsBag>>,
        banks: Option<Arc<api::named_banks::NamedBankFacts>>,
        source: Option<crate::quester::registry::FolderSource>,
        preparation: Option<Box<crate::quester::registry::RegistryPreparation<'static>>>,
        quests: Option<Arc<api::quest_facts::QuestCatalog>>,
        pending: Option<Box<crate::quester::registry::PreparedRegistry>>,
        actions: u32,
    }
    let quester_state = quester.then(|| {
        Box::new(QuesterState {
            bag: Some(Arc::clone(&bag)),
            banks: Some(Arc::clone(&banks)),
            source: Some(crate::quester::registry::source_for_preparation()),
            preparation: None,
            quests: None,
            pending: None,
            actions: 0,
        })
    });
    let input = if let Some(state) = quester_state.as_ref() {
        PreparationInput::for_quester_folder(
            run.run,
            settings_input,
            state.source.as_ref().expect("Quester source captured"),
        )
        .map_err(StartError::WorkRefused)?
    } else if cold_gathering {
        settings_input.with_gathering_family_decode()
    } else {
        settings_input
    };
    let slot = api::identity::SlotKey::from_incarnation(run.slot)
        .ok_or_else(|| StartError::Unavailable("slot worker is not bound".into()))?;
    enum Stage {
        Quester {
            state: Box<QuesterState>,
            configuring: bool,
        },
        Decode {
            bag: Arc<SettingsBag>,
            banks: Arc<api::named_banks::NamedBankFacts>,
        },
        Configure {
            bag: Arc<SettingsBag>,
            banks: Arc<api::named_banks::NamedBankFacts>,
        },
        Construct {
            config: Arc<PreparedConfig>,
            quester: Option<Box<QuesterState>>,
        },
        Done,
    }
    let mut stage = if let Some(state) = quester_state {
        Stage::Quester {
            state,
            configuring: false,
        }
    } else if cold_gathering {
        Stage::Decode { bag, banks }
    } else {
        Stage::Configure { bag, banks }
    };
    let mut account = Some(account.into_boxed_str());
    let worker = preparation::submit_cooperative(
        PreparationKind::Profile,
        api::identity::Scope::Slot(slot),
        input,
        move |worker| {
            let regular_total = if cold_gathering { 3 } else { 2 };
            let (mut completed, mut total) = match &stage {
                Stage::Quester {
                    state,
                    configuring: false,
                } => (state.actions, None),
                Stage::Quester {
                    state,
                    configuring: true,
                } => (state.actions + 1, Some(state.actions + 2)),
                Stage::Decode { .. } => (0, Some(regular_total)),
                Stage::Configure { .. } => (u32::from(cold_gathering), Some(regular_total)),
                Stage::Construct {
                    quester: Some(state),
                    ..
                } => (state.actions + 1, Some(state.actions + 2)),
                Stage::Construct { quester: None, .. } => (regular_total - 1, Some(regular_total)),
                Stage::Done => unreachable!("terminal preparation step is not dispatched"),
            };
            let result = (|| -> Result<Option<CompiledRun>, StartError> {
                match std::mem::replace(&mut stage, Stage::Done) {
                    Stage::Quester {
                        mut state,
                        configuring: false,
                    } => {
                        state.actions = state
                            .actions
                            .checked_add(1)
                            .expect("bounded Quester registry action count");
                        completed = state.actions;
                        if state.preparation.is_none() {
                            let quests = Arc::new(
                                api::quest_facts::QuestCatalog::from_identity(
                                    selected.quest_identity(),
                                )
                                .map_err(StartError::Facts)?,
                            );
                            state.preparation = Some(Box::new(
                                crate::quester::registry::RegistryPreparation::from_source(
                                    state.source.take().expect("Quester folder source captured"),
                                    Arc::clone(&selected),
                                    Arc::clone(&quests),
                                )
                                .map_err(|error| {
                                    StartError::Unavailable(
                                        format!(
                                            "{}: {}",
                                            error.code,
                                            error.detail.as_deref().unwrap_or("Path reload failed")
                                        )
                                        .into(),
                                    )
                                })?,
                            ));
                            state.quests = Some(quests);
                        }
                        let preparation = state
                            .preparation
                            .as_mut()
                            .expect("registry preparation initialized");
                        match preparation.step(worker).map_err(|error| {
                            StartError::Unavailable(
                                format!(
                                    "{}: {}",
                                    error.code,
                                    error.detail.as_deref().unwrap_or("Path reload failed")
                                )
                                .into(),
                            )
                        })? {
                            Some(candidate) => {
                                state.preparation = None;
                                state.pending = Some(Box::new(candidate));
                                total = Some(state.actions);
                                stage = Stage::Quester {
                                    state,
                                    configuring: true,
                                };
                            }
                            None => {
                                stage = Stage::Quester {
                                    state,
                                    configuring: false,
                                };
                            }
                        }
                        Ok(None)
                    }
                    Stage::Quester {
                        mut state,
                        configuring: true,
                    } => {
                        let ready = crate::quester::card::QuesterRegistryReady::new(
                            state
                                .pending
                                .as_ref()
                                .expect("Quester registry prepared")
                                .registry()
                                .clone(),
                            state.quests.take().expect("Quester quest facts prepared"),
                        );
                        let config = prepare_config(
                            worker,
                            card.id,
                            1,
                            state.bag.take().expect("Quester settings retained"),
                            Arc::clone(&selected),
                            state.banks.take().expect("Quester banks retained"),
                            Some(ready),
                        )?;
                        completed = state.actions + 1;
                        stage = Stage::Construct {
                            config,
                            quester: Some(state),
                        };
                        Ok(None)
                    }
                    Stage::Decode { bag, banks } => {
                        selected
                            .prepare_gathering(worker)
                            .map_err(StartError::Facts)?;
                        completed = 1;
                        stage = Stage::Configure { bag, banks };
                        Ok(None)
                    }
                    Stage::Configure { bag, banks } => {
                        let config = prepare_config(
                            worker,
                            card.id,
                            1,
                            bag,
                            Arc::clone(&selected),
                            banks,
                            None,
                        )?;
                        completed = regular_total - 1;
                        stage = Stage::Construct {
                            config,
                            quester: None,
                        };
                        Ok(None)
                    }
                    Stage::Construct {
                        config,
                        mut quester,
                    } => {
                        let pin = selected.selected_pin().map_err(StartError::Facts)?;
                        let mut retained = try_retained(&retained)?;
                        let script = catch_unwind(AssertUnwindSafe(|| {
                            (card.create)(run, Arc::clone(&config), &mut retained)
                        }))
                        .map_err(|payload| {
                            StartError::Unavailable(
                                format!("factory panic: {}", panic_message(&payload)).into(),
                            )
                        })??;
                        Ok(Some(CompiledRun {
                            script: ScriptOwner(Some(script)),
                            config,
                            pending: None,
                            pending_registry: quester
                                .as_mut()
                                .and_then(|state| state.pending.take()),
                            run,
                            selected: Arc::clone(&selected),
                            pin,
                            output: Output {
                                account: account.take(),
                                ..Default::default()
                            },
                            actions: NativeActions { _private: () },
                        }))
                    }
                    Stage::Done => unreachable!("terminal preparation step is not dispatched"),
                }
            })();
            match result {
                Ok(None) => preparation::CooperativePreparationStep::Continue { completed, total },
                Ok(Some(run)) => preparation::CooperativePreparationStep::Complete {
                    output: PreparationResult(Some(Ok(run))),
                    completed: total.expect("completed preparation has a known total"),
                    total,
                },
                Err(error) => preparation::CooperativePreparationStep::Complete {
                    output: PreparationResult(Some(Err(error))),
                    completed,
                    total,
                },
            }
        },
    )
    .map_err(|error| match error {
        preparation::PreparationError::Refused(api::work::SubmissionRefusal::Full) => {
            PrepareRunError::Full
        }
        preparation::PreparationError::Refused(refusal) => {
            PrepareRunError::Other(StartError::WorkRefused(refusal))
        }
        other => PrepareRunError::Other(StartError::Unavailable(other.to_string().into())),
    })?;
    Ok(Preparation {
        generation: run.run,
        worker,
    })
}

/// Off-pump preparation for an API seat's unlisted native card (no Browse
/// registry entry): `build` creates the card from the selected data on the
/// preparation worker and returns its prepared configuration.
pub(super) fn prepare_native_run(
    run: RunKey,
    selected: Arc<api::game_data::SelectedGameData>,
    account: String,
    build: impl FnOnce(
            &Arc<api::game_data::SelectedGameData>,
        ) -> Result<(Box<dyn Script>, Arc<PreparedConfig>), StartError>
        + Send
        + 'static,
) -> Result<Preparation, PrepareRunError> {
    let slot = api::identity::SlotKey::from_incarnation(run.slot).ok_or_else(|| {
        PrepareRunError::Other(StartError::Unavailable("slot worker is not bound".into()))
    })?;
    let input = PreparationInput::new(
        run.run,
        account.capacity(),
        2 * 1024 * 1024,
        2 * 1024 * 1024,
    )
    .map_err(|error| PrepareRunError::Other(StartError::WorkRefused(error)))?;
    let worker = preparation::submit(
        PreparationKind::Card,
        api::identity::Scope::Slot(slot),
        input,
        move |_| {
            PreparationResult(Some((|| {
                let pin = selected.selected_pin().map_err(StartError::Facts)?;
                let (script, config) = catch_unwind(AssertUnwindSafe(|| build(&selected)))
                    .map_err(|payload| {
                        StartError::Unavailable(
                            format!("preparation panic: {}", panic_message(&payload)).into(),
                        )
                    })??;
                Ok(CompiledRun {
                    script: ScriptOwner(Some(script)),
                    config,
                    pending: None,
                    pending_registry: None,
                    run,
                    selected,
                    pin,
                    output: Output {
                        account: Some(account.into_boxed_str()),
                        ..Default::default()
                    },
                    actions: NativeActions { _private: () },
                })
            })()))
        },
    )
    .map_err(|error| match error {
        preparation::PreparationError::Refused(api::work::SubmissionRefusal::Full) => {
            PrepareRunError::Full
        }
        preparation::PreparationError::Refused(refusal) => {
            PrepareRunError::Other(StartError::WorkRefused(refusal))
        }
        other => PrepareRunError::Other(StartError::Unavailable(other.to_string().into())),
    })?;
    Ok(Preparation {
        generation: run.run,
        worker,
    })
}

pub(super) fn destroy_run(mut run: Box<CompiledRun>, reason: StopReason) -> Option<String> {
    let stopped = run.script.stop_with_output(reason, &mut run.output);
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
    /// The WorkAdmission scope for requests owned by this slot lifetime.
    ///
    /// An unbound slot has no work identity and must not submit slot-scoped
    /// background work.
    pub fn work_scope(&self) -> Option<api::identity::Scope> {
        api::identity::SlotKey::from_incarnation(self.incarnation).map(api::identity::Scope::Slot)
    }

    pub fn start_compiled(
        &mut self,
        account: &str,
        id: crate::CompiledId,
        bag: Arc<SettingsBag>,
        selected: Arc<api::game_data::SelectedGameData>,
        banks: Arc<api::named_banks::NamedBankFacts>,
    ) -> Result<(), StartError> {
        if self.quest_pairs.as_ref().is_some_and(|pairs| pairs.busy()) {
            return Err(StartError::Unavailable("Stop this pair first".into()));
        }
        if !matches!(self.state, RunState::Idle | RunState::Error) || self.surface_active() {
            return Err(StartError::Busy);
        }
        if self.incarnation == 0 {
            return Err(StartError::Unavailable("slot worker is not bound".into()));
        }
        let card = crate::compiled_card(id)
            .ok_or_else(|| StartError::Unavailable(format!("not ported: {}", id.0).into()))?;
        if id == crate::CompiledId("Quester")
            && self.quest_pairs.is_none()
            && crate::quester::card::requires_pairs(&bag).map_err(StartError::Config)?
        {
            return Err(StartError::Unavailable(
                "partner capability is not installed in this Play".into(),
            ));
        }
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
            Arc::clone(&bag),
            Arc::clone(&selected),
            Arc::clone(&banks),
            Arc::clone(&retained),
            account.to_owned(),
        );
        match job {
            Ok(job) => self.preparing = Some(Box::new(job)),
            Err(PrepareRunError::Full) => {
                self.pending_compiled_start = Some(PendingCompiledStart {
                    card,
                    run,
                    bag,
                    selected,
                    banks,
                    retained,
                    account: account.to_owned(),
                });
            }
            Err(PrepareRunError::Other(error)) => return Err(error),
        }
        self.control_generation = generation;
        self.terminal_native_status = None;
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
        let input = PreparationInput::for_settings(generation, config.bag())
            .map_err(|error| error.to_string())?;
        let slot =
            api::identity::SlotKey::from_incarnation(run.slot).ok_or("slot worker is not bound")?;
        // Do not tear down the running identity until replacement admission
        // actually succeeds. Full retains one retry demand on this slot.
        let worker = preparation::submit(
            PreparationKind::Card,
            api::identity::Scope::Slot(slot),
            input,
            move |_| {
                PreparationResult(Some((|| {
                    let mut retained = try_retained(&retained)?;
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
                    let (config, pending) =
                        reoffer_pending(&mut script, &mut output, config, pending)?;
                    Ok(CompiledRun {
                        script,
                        config,
                        pending,
                        pending_registry: None,
                        run,
                        selected,
                        pin,
                        output,
                        actions: NativeActions { _private: () },
                    })
                })()))
            },
        );
        let worker = match worker {
            Ok(worker) => worker,
            Err(preparation::PreparationError::Refused(api::work::SubmissionRefusal::Full)) => {
                self.pending_compiled_restart = true;
                self.revoke_native_input();
                return Ok(());
            }
            Err(error) => {
                let message = error.to_string();
                self.fail_compiled(ScriptFailure {
                    code: "restart-worker".into(),
                    message: message.clone().into(),
                });
                return Err(message);
            }
        };
        self.pending_compiled_restart = false;
        self.revoke_native_input();
        self.teardown_compiled(StopReason::Replaced);
        self.control_generation = generation;
        self.preparing = Some(Box::new(Preparation { generation, worker }));
        self.state = RunState::Starting;
        self.watchdog.on_restart_applied(now);
        Ok(())
    }

    fn poll_compiled(&mut self) {
        if let Some(pending) = self.pending_compiled_start.take() {
            if pending.run.run != self.control_generation || !self.want_run {
                return;
            }
            match pending.try_admit() {
                Ok(job) => self.preparing = Some(Box::new(job)),
                Err(PrepareRunError::Full) => {
                    self.pending_compiled_start = Some(pending);
                    return;
                }
                Err(PrepareRunError::Other(error)) => {
                    self.last_error = Some(error.to_string());
                    self.want_run = false;
                    self.state = RunState::Idle;
                    self.settle_start(StartOutcome::Rejected(error));
                    return;
                }
            }
        }
        if self.pending_compiled_restart && self.compiled.is_some() {
            if let Err(error) = self.restart_compiled(Instant::now()) {
                self.last_error = Some(error);
            }
            return;
        }
        if !self.preparing.as_ref().is_some_and(|job| job.is_finished()) {
            return;
        }
        let job = self.preparing.take().expect("finished preparation");
        if job.generation != self.control_generation {
            // Joining also avoids leaving a finished packet to JoinHandle::drop.
            let _ = job.join();
            return;
        }
        let identity = job.worker.identity();
        let result = job.join().and_then(|mut run| {
            if let Some(candidate) = run.pending_registry.take() {
                crate::quester::registry::commit_snapshot(*candidate, identity, identity).map_err(
                    |error| {
                        StartError::Unavailable(
                            format!(
                                "{}: {}",
                                error.code,
                                error.detail.as_deref().unwrap_or("Path reload failed")
                            )
                            .into(),
                        )
                    },
                )?;
            }
            Ok(run)
        });
        match result {
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
        self.arm_compiled_seat(run.run, run.config.card());
        self.compiled = Some(Box::new(run));
        self.last_error = None;
        self.lifecycle_receipt = None;
        self.terminal_native_status = None;
        self.ticks = 0;
        self.pending_withdraw_x = None;
        self.withdraw_x_result_seq = 0;
        self.withdraw_x_result = false;
        self.pending_bank_op = None;
        self.bank_op_result_seq = 0;
        self.bank_op_result = false;
        self.last_settings_fp = None;
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
        self.observe_surface_lifecycle();
    }

    pub fn native_run(&self) -> Option<RunKey> {
        self.compiled.as_ref().map(|run| run.run)
    }
    /// Permissions frozen in this compiled instance's effective committed revision.
    /// Pending boundary/restart edits are not grants until the card applies them.
    pub fn native_walk_permissions(&self) -> Option<crate::native::WalkPermissions> {
        self.compiled
            .as_ref()
            .map(|run| run.config.walk_permissions())
    }

    pub fn native_status(&self) -> Option<Arc<ScriptStatus>> {
        let status = self
            .compiled
            .as_ref()
            .and_then(|run| run.output.status.clone());
        let status = status.or_else(|| self.api_status());
        status.or_else(|| self.terminal_native_status.clone())
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
                });
                CompiledDelivery::Rejected(error)
            }
        }
    }

    pub(super) fn interrupt_compiled(&mut self, event: Interrupt) {
        let Some(run) = self.compiled.as_mut() else {
            return;
        };
        if matches!(event, Interrupt::Pause | Interrupt::SessionEnded) {
            if let Some(pairs) = &self.quest_pairs {
                pairs.invalidate(run.run);
            }
        }
        if let Err(payload) = catch_unwind(AssertUnwindSafe(|| run.script.interrupt(event))) {
            self.fail_compiled(ScriptFailure {
                code: "interrupt-panic".into(),
                message: panic_message(&payload).into(),
            });
        }
    }

    pub(super) fn teardown_compiled(&mut self, reason: StopReason) {
        self.evidence_wait_run = None;
        self.native_runtime.clear_note_progress();
        if reason != StopReason::Completed {
            self.native_runtime.revoke();
        }
        if let (Some(pairs), Some(run)) = (&self.quest_pairs, &self.compiled) {
            pairs.invalidate(run.run);
        }
        self.pair_evidence = None;
        if let Some(run) = self.compiled.take() {
            if let Some(error) = destroy_run(run, reason) {
                self.last_error = Some(error);
            }
        }
    }

    pub(super) fn fail_compiled(&mut self, failure: ScriptFailure) {
        self.revoke_native_input();
        {
            self.compiled_interacts.clear();
            self.compiled_interact_outcome_seqs.clear();
            self.card_abort_owed = true;
        }
        self.pending_logs.push(failure.message.to_string());
        self.last_error = Some(failure.message.to_string());
        self.lifecycle_receipt = Some(ScriptLifecycleReceipt {
            runtime_generation: self.runtime_generation,
            state: ScriptTerminalState::Failed,
            tick: self.ticks,
            reason: failure.message.to_string(),
        });
        let terminal_status = self.publish_terminal_native_failure(failure);
        self.state = RunState::Error;
        self.want_run = false;
        self.teardown_compiled(StopReason::Error);
        self.watchdog.cancel_clear();
        self.terminal_native_status = terminal_status;
    }

    /// Same run/session fence as other native controls. A successful request
    /// asks the active script for one fresh journal read at its next safe
    /// boundary; it never revives a terminal run or clears its status.
    pub fn read_journal_compiled(&mut self, target: RunKey) -> Result<(), ScriptFailure> {
        let run = self
            .compiled
            .as_mut()
            .filter(|run| run.run == target)
            .ok_or_else(|| ScriptFailure {
                code: "read-journal-refused".into(),
                message: "stale native run".into(),
            })?;
        match catch_unwind(AssertUnwindSafe(|| run.script.read_journal())) {
            Ok(result) => {
                result?;
                Ok(())
            }
            Err(payload) => {
                let failure = ScriptFailure {
                    code: "read-journal-panic".into(),
                    message: panic_message(&payload).into(),
                };
                self.fail_compiled(failure.clone());
                Err(failure)
            }
        }
    }

    /// Request an operator skip only for a live compiled Quester run.
    pub fn skip_quest_compiled(
        &mut self,
        target: Option<RunKey>,
    ) -> Result<crate::native::SkippedQuest, ScriptFailure> {
        if !matches!(self.state, RunState::Running | RunState::Paused) {
            return Err(ScriptFailure {
                code: "not-quester".into(),
                message: "bot is not running the Quester queue".into(),
            });
        }
        let result = {
            let Some(run) = self.compiled.as_mut() else {
                return Err(ScriptFailure {
                    code: "not-quester".into(),
                    message: "bot is not running the Quester queue".into(),
                });
            };
            if run.config.card() != crate::CompiledId("Quester") {
                return Err(ScriptFailure {
                    code: "not-quester".into(),
                    message: "bot is not running the Quester queue".into(),
                });
            }
            if Some(run.run) != target {
                return Err(ScriptFailure {
                    code: "skip-quest-refused".into(),
                    message: "stale native run".into(),
                });
            }
            catch_unwind(AssertUnwindSafe(|| run.script.skip_quest()))
        };
        match result {
            Ok(result) => result,
            Err(payload) => {
                let failure = ScriptFailure {
                    code: "skip-quest-panic".into(),
                    message: panic_message(&payload).into(),
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
        self.start_test_script_with_card(crate::CompiledId("test"), script, selected)
    }

    /// Start a fixture script under an explicit compiled card identity.
    #[cfg(any(test, feature = "test-hooks"))]
    #[doc(hidden)]
    pub fn start_test_script_with_card(
        &mut self,
        card: crate::CompiledId,
        script: Box<dyn Script>,
        selected: Option<Arc<api::game_data::SelectedGameData>>,
    ) -> Result<(), String> {
        if !matches!(self.state, RunState::Idle | RunState::Error) || self.surface_active() {
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
            config: PreparedConfig::new(card, 1, 1, Arc::new(SettingsBag::new()), ()),
            pending: None,
            pending_registry: None,
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

/// Test-only Native output recorder for moved fixture assertions.
#[cfg(any(test, feature = "test-hooks"))]
#[derive(Default)]
pub struct TestOutput {
    pub(crate) statuses: Vec<ScriptStatus>,
    pub(crate) paints: Vec<Arc<crate::shim::ScriptPaint>>,
    pub(crate) logs: Vec<String>,
}

#[cfg(any(test, feature = "test-hooks"))]
impl TestOutput {
    /// Test-only view of the paint frames recorded by this output.
    pub fn paints(&self) -> &[Arc<crate::paint::ScriptPaint>] {
        &self.paints
    }
}

#[cfg(any(test, feature = "test-hooks"))]
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

// `pub(crate)`: the process host-log sink installs once, so the slot
// tests capture their finish line through this module's one sink.
#[cfg(test)]
#[path = "compiled_tests.rs"]
pub(crate) mod tests;
