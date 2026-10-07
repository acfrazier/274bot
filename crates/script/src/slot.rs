//! Per-uid script runner. `SlotScript` owns at most one compiled `Script`
//! and gates it on operator intent (`want_run`) and client presence
//! (`on_is_up`). `tick` runs on the caller's pump at a game-tick edge and
//! must return; panics are caught, never abort the process.

#[cfg(feature = "load")]
#[path = "slot/api.rs"]
mod api_seat;
pub(crate) mod compiled;
mod pending;
use crate::native::{Interrupt, RetainedMemory, ScriptFailure, ScriptFlow, StopReason};
pub use compiled::{prepare_config, CompiledDelivery};
use compiled::{CompiledRun, Preparation};

pub use pending::{
    PendingBankOp, PendingBankOpKind, PendingFillBaseline, PendingWithdrawResult, PendingWithdrawX,
    PendingWithdrawXPhase,
};

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::ctx::ScriptCtx;
#[cfg(feature = "load")]
use crate::isolate_fb::{IsolateBuf, SnapshotFingerprint};
#[cfg(feature = "load")]
use crate::load::{LoadIsolate, LoadShape, Ready};
use crate::watchdog::{ProgressWatchdog, Tile as WatchdogTile, WatchdogAction};
use api::native_input::NativeInputAuthority;
use api::random::{DetectedRandom, RandomClaim};
use serde::Serialize;
#[cfg(feature = "load")]
const CUT_RESTART_LIMIT: usize = 3;
#[cfg(feature = "load")]
const CUT_RESTART_WINDOW: Duration = Duration::from_secs(5 * 60);

/// Failure at initial loaded-script Start, distinct from an operational refusal.
#[cfg(feature = "load")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartLoadError {
    /// The caller must retain this Start until an operator hold is resolved.
    Waiting(String),
    Refused(String),
    RuntimeLoad(String),
}

#[cfg(feature = "load")]
impl std::fmt::Display for StartLoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Waiting(message) | Self::Refused(message) | Self::RuntimeLoad(message) => {
                f.write_str(message)
            }
        }
    }
}

#[cfg(feature = "load")]
impl std::error::Error for StartLoadError {}

/// Lifecycle of the script slot. `starting` is the Load-setup window
/// (spawn has returned, V8 is not ready). `paused` covers both operator
/// Pause and the not-`is_up` gate; `stopping` is the Load-join window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunState {
    Idle,
    Starting,
    Running,
    Paused,
    Stopping,
    Error,
}

/// Terminal script lifecycle states retained by the native host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScriptTerminalState {
    Stopped,
    Completed,
    Failed,
    Cancelled,
}

/// One bounded, non-consuming terminal receipt for the latest Start.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ScriptLifecycleReceipt {
    pub runtime_generation: u64,
    pub state: ScriptTerminalState,
    pub tick: u64,
    pub reason: String,
}

/// Frozen Load identity retained for watchdog recreate. Shared via Arc so
/// observe never copies source bytes.
#[cfg(feature = "load")]
#[derive(Clone)]
pub struct SlotLoadIdentity {
    pub source: Arc<str>,
    pub shape: LoadShape,
    pub siblings: Arc<[(String, String)]>,
    pub settings_bag: Option<Arc<serde_json::Map<String, serde_json::Value>>>,
    pub game_data: Option<Arc<api::game_data::SelectedGameData>>,
    pub named_banks: Arc<api::named_banks::NamedBankFacts>,
    pub loadouts: Arc<[crate::loadouts_store::Loadout]>,
    pub api_family: Option<crate::ApiFamily>,
}

#[cfg(feature = "load")]
#[derive(Clone)]
enum AfterStop {
    Idle,
    Restart,
    CutLimit(String),
    Start,
    Fail(String),
}

/// How the latest operator Load Start settled. Start returns before V8
/// setup, so the caller that owns the assignment and the load diagnostic
/// commits them from this, never from Start's `Ok`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartOutcome {
    /// Setup finished; the runtime is live (Running, or Paused by the
    /// operator or the gate).
    Ready,
    /// Setup failed. The slot is Idle and `last_error` holds this
    /// diagnostic — the one Start used to return synchronously.
    Failed(String),
    /// Stopped before setup finished; nothing ran.
    Cancelled,
    /// Compiled preparation/factory validation failed; assignment is unchanged.
    Rejected(crate::native::StartError),
}

/// One atomic read of a pending operator Load Start ([`SlotScript::poll_start`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartPoll {
    /// Setup (or the reap a queued Start waits for) is still running.
    Pending,
    /// Settled since the last poll; returned exactly once.
    Settled(StartOutcome),
    /// Nothing owed: no Start was made, or its outcome was already taken
    /// (or the slot is gone).
    NotOwed,
}

/// Per-uid runner. Compiled XOR Load (a JS isolate) — never both.
pub struct SlotScript {
    pub want_run: bool,
    state: RunState,
    compiled: Option<Box<CompiledRun>>,
    /// Diagnostic only: a blocked run has no instance or restart authority.
    terminal_native_status: Option<Arc<crate::native::ScriptStatus>>,
    preparing: Option<Box<Preparation>>,
    #[cfg(feature = "load")]
    api: Option<Box<api_seat::ApiSeat>>,
    retained: Option<Arc<std::sync::Mutex<RetainedMemory>>>,
    native_runtime: crate::native::ledger::Runtime,
    quest_pairs: Option<Arc<dyn crate::quester::pair::QuestPairPort>>,
    pair_evidence: Option<api::quest_progress::EvidenceStamp>,
    /// Host-side Stop cleanup outlives the revoked native action owner.
    stop_prayer_cleanup: crate::combat::RaisedPrayers,
    incarnation: u64,
    /// The bound server profile's world type, bound by the host on spawn.
    world_members: api::selected::Truth,
    control_generation: u64,
    /// The compiled card's own interact queue: what its tick enqueued, drained
    /// by the host on the same frames the isolate's queue is. Never both — a
    /// slot runs a compiled script XOR a Load isolate.
    #[cfg(feature = "load")]
    compiled_interacts: Vec<crate::shim::InteractReq>,
    #[cfg(feature = "load")]
    compiled_interact_outcome_seqs: Vec<u64>,
    /// A compiled clue-machine session abort is owed to the pump thread.
    /// `reset_session_work` may run on the control thread and must not touch
    /// the slot thread's TLS machine, so it marks here and
    /// [`SlotScript::sync_compiled_clue`] applies it on the next observed
    /// frame. It drops the machine's live step and its token alone: the strip
    /// list and the abandon latch are the session's own and survive a
    /// connection boundary.
    #[cfg(feature = "load")]
    clue_abort_owed: bool,
    /// A whole clue-machine instance reset is owed to the pump thread:
    /// operator Stop is a fresh task instance, so the machine's step, its
    /// strip list and its abandon latch all start over. Marked by `stop` and
    /// applied by [`SlotScript::sync_compiled_clue`] the way the abort is; it
    /// subsumes [`Self::clue_abort_owed`].
    #[cfg(feature = "load")]
    clue_stop_owed: bool,
    /// JS Load isolate, spawned by `start_load` on Start (not on Load).
    #[cfg(feature = "load")]
    load: Option<LoadIsolate>,
    /// True once [`LoadIsolate::poll_ready`] has returned Ready.
    #[cfg(feature = "load")]
    load_ready: bool,
    /// Reaper completion for a non-blocking Stop.
    #[cfg(feature = "load")]
    stop_rx: Option<std::sync::mpsc::Receiver<Vec<String>>>,
    #[cfg(feature = "load")]
    after_stop: AfterStop,
    /// An operator Load Start has not settled yet (see [`StartOutcome`]).
    start_pending: bool,
    start_outcome: Option<StartOutcome>,
    /// `runtime_generation` before the unsettled setup's bump. A setup
    /// that fails restores it: a failed Start never counted as a Start.
    #[cfg(feature = "load")]
    setup_generation_base: Option<u64>,
    /// Per-slot last-post snapshot fingerprint (delta posts: only the
    /// fields that changed are re-sent) and the `NavWorld` identity the
    /// packed banks keyframe on. Cleared on Start so the first post is a
    /// keyframe.
    #[cfg(feature = "load")]
    last_snapshot: Option<SnapshotFingerprint>,
    #[cfg(feature = "load")]
    last_world_id: Option<usize>,
    #[cfg(feature = "load")]
    reach_cache: api::query::ReachPackCache,
    /// Reusable FlatBuffer builder for this slot's host→isolate snapshot
    /// posts. One per slot; the V8 isolate thread holds its own for
    /// interact/paint. Never a JSON document on either path.
    #[cfg(feature = "load")]
    ipc: IsolateBuf,
    last_error: Option<String>,
    /// Work generation that owns the active tick error, if any. A successful
    /// tick from a later session must not clear an older diagnostic.
    #[cfg(feature = "load")]
    active_tick_error_generation: Option<u64>,
    lifecycle_receipt: Option<ScriptLifecycleReceipt>,
    /// Isolate log lines not yet taken by the panel (`take_pending_logs`).
    pending_logs: Vec<String>,
    /// Dispatched game ticks since the last Start.
    ticks: u64,
    pending_withdraw_x: Option<PendingWithdrawX>,
    withdraw_x_result_seq: u64,
    withdraw_x_result: bool,
    withdraw_load_result_seq: u64,
    withdraw_load_result: bool,
    pending_bank_op: Option<PendingBankOp>,
    bank_op_result_seq: u64,
    bank_op_result: bool,
    work_epoch: u64,
    /// Frozen source/settings used for watchdog recreate. Cleared on
    /// operator Stop / new manual Start.
    #[cfg(feature = "load")]
    load_identity: Option<SlotLoadIdentity>,
    watchdog: ProgressWatchdog,
    /// Fixed-size rolling window for terminate-driven runtime recreates.
    /// The third cut inside [`CUT_RESTART_WINDOW`] stops the script.
    #[cfg(feature = "load")]
    cut_restart_times: [Option<Instant>; CUT_RESTART_LIMIT],
    #[cfg(feature = "load")]
    cut_restart_next: usize,
    /// Stable source identity key for this execution (`catalog:Name` / file path).
    source_identity: Option<String>,
    /// Bumped on each successful Start and watchdog isolate replacement.
    runtime_generation: u64,
    last_settings_fp: Option<String>,
    native_input: Arc<NativeInputAuthority>,
    /// Frozen `RecoveryHints`, kept across watchdog isolate restarts.
    #[cfg(feature = "load")]
    recovery_hints: Arc<crate::load::RecoveryHintsCell>,
    /// Test-only spawn failure seam: when set, the next `spawn_isolate`
    /// returns a deterministic thread-spawn diagnostic instead of spawning.
    /// Never set outside `#[cfg(test)]`; production builds have no field.
    #[cfg(all(test, feature = "load"))]
    fail_spawn_for_test: bool,
}

impl Default for SlotScript {
    fn default() -> Self {
        Self::new()
    }
}

impl SlotScript {
    pub fn new() -> Self {
        SlotScript {
            want_run: false,
            state: RunState::Idle,
            compiled: None,
            terminal_native_status: None,
            preparing: None,
            #[cfg(feature = "load")]
            api: None,
            retained: None,
            native_runtime: Default::default(),
            quest_pairs: None,
            pair_evidence: None,
            stop_prayer_cleanup: crate::combat::RaisedPrayers::empty(),
            incarnation: 0,
            world_members: api::selected::Truth::Unknown,
            control_generation: 0,
            #[cfg(feature = "load")]
            compiled_interacts: Vec::new(),
            #[cfg(feature = "load")]
            compiled_interact_outcome_seqs: Vec::new(),
            #[cfg(feature = "load")]
            clue_abort_owed: false,
            #[cfg(feature = "load")]
            clue_stop_owed: false,
            #[cfg(feature = "load")]
            load: None,
            #[cfg(feature = "load")]
            load_ready: false,
            #[cfg(feature = "load")]
            stop_rx: None,
            #[cfg(feature = "load")]
            after_stop: AfterStop::Idle,
            start_pending: false,
            start_outcome: None,
            #[cfg(feature = "load")]
            setup_generation_base: None,
            #[cfg(feature = "load")]
            last_snapshot: None,
            #[cfg(feature = "load")]
            last_world_id: None,
            #[cfg(feature = "load")]
            reach_cache: api::query::ReachPackCache::default(),
            #[cfg(feature = "load")]
            ipc: IsolateBuf::new(),
            last_error: None,
            #[cfg(feature = "load")]
            active_tick_error_generation: None,
            lifecycle_receipt: None,
            pending_logs: Vec::new(),
            ticks: 0,
            pending_withdraw_x: None,
            withdraw_x_result_seq: 0,
            withdraw_x_result: false,
            withdraw_load_result_seq: 0,
            withdraw_load_result: false,
            pending_bank_op: None,
            bank_op_result_seq: 0,
            bank_op_result: false,
            work_epoch: 0,
            #[cfg(feature = "load")]
            load_identity: None,
            watchdog: ProgressWatchdog::new(),
            #[cfg(feature = "load")]
            cut_restart_times: [None; CUT_RESTART_LIMIT],
            #[cfg(feature = "load")]
            cut_restart_next: 0,
            source_identity: None,
            runtime_generation: 0,
            last_settings_fp: None,
            native_input: NativeInputAuthority::new(),
            #[cfg(feature = "load")]
            recovery_hints: Arc::new(crate::load::RecoveryHintsCell::new()),
            #[cfg(all(test, feature = "load"))]
            fail_spawn_for_test: false,
        }
    }
    /// Install this Play's account-bound authority, without taking another slot lock.
    pub fn bind_quest_pairs(&mut self, port: Arc<dyn crate::quester::pair::QuestPairPort>) {
        self.quest_pairs = Some(port);
    }

    pub fn pair_world_changed(&self, host: &str, port: u16) {
        if let Some(pairs) = &self.quest_pairs {
            pairs.world_changed(host, port);
        }
    }

    /// True when either a compiled script or a JS isolate is installed.
    fn has_instance(&self) -> bool {
        self.compiled.is_some() || self.load_active()
    }

    /// Start a JS Load isolate (the isolate is spawned here, on Start, not
    /// at Load). Same state gating as [`SlotScript::start_compiled`].
    #[cfg(feature = "load")]
    pub fn start_load(
        &mut self,
        source: String,
        shape: LoadShape,
        siblings: Vec<(String, String)>,
    ) -> Result<(), String> {
        let loadouts = crate::loadouts_store::LoadoutsStore::with_default_path();
        self.start_load_with_loadouts(source, shape, siblings, loadouts.loadouts())
    }

    /// Start with explicit loadouts, avoiding operator filesystem inputs in
    /// disposable live fixtures. Commands are posted before the first tick.
    #[cfg(feature = "load")]
    pub fn start_load_with_loadouts(
        &mut self,
        source: String,
        shape: LoadShape,
        siblings: Vec<(String, String)>,
        loadouts: &[crate::loadouts_store::Loadout],
    ) -> Result<(), String> {
        self.start_load_with_loadouts_and_game_data(
            source,
            shape,
            siblings,
            loadouts,
            None,
            std::sync::Arc::new(api::named_banks::NamedBankFacts::empty()),
        )
    }

    /// Start with explicit loadouts and immutable selected-revision facts.
    #[cfg(feature = "load")]
    pub fn start_load_with_loadouts_and_game_data(
        &mut self,
        source: String,
        shape: LoadShape,
        siblings: Vec<(String, String)>,
        loadouts: &[crate::loadouts_store::Loadout],
        game_data: Option<std::sync::Arc<api::game_data::SelectedGameData>>,
        named_banks: std::sync::Arc<api::named_banks::NamedBankFacts>,
    ) -> Result<(), String> {
        self.start_load_with_loadouts_and_game_data_typed(
            source,
            shape,
            siblings,
            loadouts,
            game_data,
            named_banks,
        )
        .map_err(|e| e.to_string())
    }

    #[cfg(feature = "load")]
    fn start_load_with_loadouts_and_game_data_typed(
        &mut self,
        source: String,
        shape: LoadShape,
        siblings: Vec<(String, String)>,
        loadouts: &[crate::loadouts_store::Loadout],
        game_data: Option<std::sync::Arc<api::game_data::SelectedGameData>>,
        named_banks: std::sync::Arc<api::named_banks::NamedBankFacts>,
    ) -> Result<(), StartLoadError> {
        match self.state {
            RunState::Running | RunState::Paused | RunState::Starting => Err(
                StartLoadError::Refused("script already active: stop it first".to_string()),
            ),
            RunState::Stopping => {
                if self.compiled.is_some() {
                    return Err(StartLoadError::Refused(
                        "compiled script active: stop it first".to_string(),
                    ));
                }
                // A queued Start or a watchdog restart already owns the
                // reap; only a plain Stop (or a failed setup) may be followed.
                if matches!(self.after_stop, AfterStop::Start | AfterStop::Restart) {
                    return Err(StartLoadError::Refused(
                        "script already active: stop it first".to_string(),
                    ));
                }
                if let AfterStop::Fail(e) = &self.after_stop {
                    // The failed setup is being reaped; keep its diagnostic
                    // in the log instead of silently replacing it.
                    self.pending_logs.push(e.clone());
                }
                self.load_identity = Some(SlotLoadIdentity {
                    api_family: crate::resolve_api_family(&source)
                        .ok()
                        .map(|(_, family)| family),
                    source: Arc::from(source),
                    shape,
                    siblings: siblings.into(),
                    settings_bag: None,
                    game_data,
                    named_banks,
                    loadouts: Arc::from(loadouts.to_vec()),
                });
                self.after_stop = AfterStop::Start;
                self.reset_for_fresh_start();
                // The generation moves now, not when the reap finishes, so
                // the value a caller reads after Start is the one it runs.
                self.setup_generation_base = Some(self.runtime_generation);
                self.runtime_generation = self.runtime_generation.wrapping_add(1);
                self.last_settings_fp = None;
                self.begin_start_outcome();
                Ok(())
            }
            RunState::Idle | RunState::Error => {
                if self.compiled.is_some() {
                    return Err(StartLoadError::Refused(
                        "compiled script active: stop it first".to_string(),
                    ));
                }
                let identity = SlotLoadIdentity {
                    api_family: crate::resolve_api_family(&source)
                        .ok()
                        .map(|(_, family)| family),
                    source: Arc::from(source),
                    shape,
                    siblings: siblings.into(),
                    settings_bag: None,
                    game_data,
                    named_banks,
                    loadouts: Arc::from(loadouts.to_vec()),
                };
                self.spawn_isolate(identity, true)
                    .map_err(StartLoadError::RuntimeLoad)?;
                self.reset_for_fresh_start();
                self.begin_start_outcome();
                Ok(())
            }
        }
    }

    /// Operator-Start bookkeeping shared by an immediate and a queued Start.
    #[cfg(feature = "load")]
    fn reset_for_fresh_start(&mut self) {
        self.watchdog.arm_fresh(Instant::now());
        self.want_run = true;
        self.last_error = None;
        self.active_tick_error_generation = None;
        self.lifecycle_receipt = None;
        self.terminal_native_status = None;
        self.ticks = 0;
        self.pending_withdraw_x = None;
        self.withdraw_x_result_seq = 0;
        self.withdraw_x_result = false;
        self.withdraw_load_result_seq = 0;
        self.withdraw_load_result = false;
        self.pending_bank_op = None;
        self.bank_op_result_seq = 0;
        self.bank_op_result = false;
        self.cut_restart_times = [None; CUT_RESTART_LIMIT];
        self.cut_restart_next = 0;
    }

    #[cfg(feature = "load")]
    fn begin_start_outcome(&mut self) {
        self.start_pending = true;
        self.start_outcome = None;
    }

    fn settle_start(&mut self, outcome: StartOutcome) {
        if std::mem::take(&mut self.start_pending) {
            self.start_outcome = Some(outcome);
        }
    }

    /// Resolve the lifecycle and read the latest operator Load Start in one
    /// step, so no observe can settle it between the outcome read and the
    /// in-flight check (the slot thread observes every frame).
    pub fn poll_start(&mut self) -> StartPoll {
        self.observe_lifecycle();
        match self.start_outcome.take() {
            Some(outcome) => StartPoll::Settled(outcome),
            None if self.start_pending => StartPoll::Pending,
            None => StartPoll::NotOwed,
        }
    }

    /// Spawn the isolate for `identity` and enter Starting. `bump` moves
    /// the runtime generation (a queued Start already moved it).
    #[cfg(feature = "load")]
    fn spawn_isolate(&mut self, identity: SlotLoadIdentity, bump: bool) -> Result<(), String> {
        #[cfg(test)]
        if std::mem::take(&mut self.fail_spawn_for_test) {
            return Err("isolate thread: test-injected spawn failure".to_string());
        }
        let isolate = LoadIsolate::spawn_with_content(
            identity.source.to_string(),
            identity.shape,
            identity.siblings.iter().cloned().collect(),
            identity.game_data.clone(),
            Arc::clone(&identity.named_banks),
        )?;
        isolate.post_loadouts(&identity.loadouts);
        isolate.post_recovery_hints(Arc::clone(&self.recovery_hints));
        if let Some(bag) = identity.settings_bag.as_deref() {
            isolate.post_settings_bag(bag);
        }
        self.load = Some(isolate);
        self.load_ready = false;
        self.load_identity = Some(identity);
        self.last_snapshot = None;
        self.last_world_id = None;
        self.reach_cache.clear();
        self.ipc = IsolateBuf::new();
        self.last_error = None;
        self.active_tick_error_generation = None;
        self.lifecycle_receipt = None;
        self.ticks = 0;
        self.state = RunState::Starting;
        if bump {
            self.setup_generation_base = Some(self.runtime_generation);
            self.runtime_generation = self.runtime_generation.wrapping_add(1);
            self.last_settings_fp = None;
        }
        Ok(())
    }
    /// Test-only: arm the spawn-failure seam, then drive the real typed
    /// Start producer. Lets a test assert the `Idle|Error` branch maps a
    /// spawn failure to `RuntimeLoad` (not `Refused`) without spawning.
    #[cfg(all(test, feature = "load"))]
    pub(crate) fn start_with_injected_spawn_failure_for_test(
        &mut self,
        source: String,
        shape: crate::load::LoadShape,
        siblings: Vec<(String, String)>,
        loadouts: &[crate::loadouts_store::Loadout],
        game_data: Option<std::sync::Arc<api::game_data::SelectedGameData>>,
        named_banks: std::sync::Arc<api::named_banks::NamedBankFacts>,
    ) -> Result<(), StartLoadError> {
        self.fail_spawn_for_test = true;
        self.start_load_with_loadouts_and_game_data_typed(
            source,
            shape,
            siblings,
            loadouts,
            game_data,
            named_banks,
        )
    }

    #[cfg(feature = "load")]
    fn begin_async_stop(&mut self, after: AfterStop) -> bool {
        let Some(isolate) = self.load.take() else {
            return false;
        };
        self.teardown_api(StopReason::Replaced);
        let (tx, rx) = std::sync::mpsc::channel();
        isolate.join_detached(tx);
        self.stop_rx = Some(rx);
        self.load_ready = false;
        self.setup_generation_base = None;
        self.state = RunState::Stopping;
        self.after_stop = after.clone();
        if matches!(after, AfterStop::Idle) {
            self.load_identity = None;
            self.source_identity = None;
            self.runtime_generation = self.runtime_generation.wrapping_add(1);
            self.watchdog.cancel_clear();
            self.last_settings_fp = None;
        }
        true
    }

    fn finish_idle_stop(&mut self) {
        self.teardown_compiled(StopReason::Operator);
        #[cfg(feature = "load")]
        self.teardown_api(StopReason::Operator);
        #[cfg(feature = "load")]
        self.compiled_interacts.clear();
        #[cfg(feature = "load")]
        {
            self.compiled_interact_outcome_seqs.clear();
            self.last_snapshot = None;
            self.last_world_id = None;
            self.ipc = IsolateBuf::new();
            self.load_ready = false;
            self.stop_rx = None;
            self.after_stop = AfterStop::Idle;
            self.setup_generation_base = None;
            self.load_identity = None;
        }
        self.watchdog.cancel_clear();
        self.want_run = false;
        self.pending_withdraw_x = None;
        self.pending_bank_op = None;
        self.work_epoch = self.work_epoch.wrapping_add(1);
        self.state = RunState::Idle;
        self.source_identity = None;
        self.runtime_generation = self.runtime_generation.wrapping_add(1);
        self.last_settings_fp = None;
    }

    /// Resolve Start/Stop without blocking. The slot thread and UI pump
    /// call this once per frame.
    #[cfg(feature = "load")]
    fn observe_load_lifecycle(&mut self) {
        if let Some(isolate) = self.load.as_ref() {
            if !self.load_ready {
                match isolate.poll_ready() {
                    Ready::Pending => {}
                    Ready::Ready => {
                        self.load_ready = true;
                        self.setup_generation_base = None;
                        if self.state == RunState::Starting {
                            if self.want_run {
                                self.state = RunState::Running;
                                self.native_input.publish_live();
                            } else {
                                isolate.pause();
                                self.state = RunState::Paused;
                            }
                        }
                        self.settle_start(StartOutcome::Ready);
                    }
                    Ready::Failed(e) => {
                        let isolate = self.load.take().expect("failed setup still owns isolate");
                        self.teardown_api(StopReason::Replaced);
                        let (tx, rx) = std::sync::mpsc::channel();
                        isolate.join_detached(tx);
                        self.stop_rx = Some(rx);
                        self.load_ready = false;
                        self.state = RunState::Stopping;
                        self.after_stop = AfterStop::Fail(e);
                        self.want_run = false;
                    }
                }
            }
        }
        let script_cut = self.load.as_ref().is_some_and(LoadIsolate::take_script_cut);
        if script_cut {
            self.handle_script_cut(Instant::now());
        }
        let Some(rx) = self.stop_rx.take() else {
            return;
        };
        match rx.try_recv() {
            Ok(logs) => {
                self.pending_logs.extend(logs);
                self.complete_stop();
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => self.stop_rx = Some(rx),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => self.complete_stop(),
        }
    }
    #[cfg(feature = "load")]
    fn handle_script_cut(&mut self, now: Instant) {
        self.cut_restart_times[self.cut_restart_next] = Some(now);
        self.cut_restart_next = (self.cut_restart_next + 1) % CUT_RESTART_LIMIT;
        let recent = self
            .cut_restart_times
            .iter()
            .flatten()
            .filter(|cut| now.saturating_duration_since(**cut) <= CUT_RESTART_WINDOW)
            .count();
        if recent >= CUT_RESTART_LIMIT {
            let error =
                "script stopped after 3 runaway JavaScript cuts within 5 minutes".to_string();
            self.pending_logs.push(error.clone());
            self.last_error = Some(error.clone());
            self.active_tick_error_generation = None;
            self.want_run = false;
            self.revoke_native_input();
            if !self.begin_async_stop(AfterStop::CutLimit(error.clone())) {
                self.finish_cut_limit(error);
            }
            return;
        }
        if let Err(error) = self.apply_load_restart(now) {
            self.pending_logs.push(error.clone());
            self.last_error = Some(error.clone());
            self.want_run = false;
            self.revoke_native_input();
            if !self.begin_async_stop(AfterStop::CutLimit(error.clone())) {
                self.finish_cut_limit(error);
            }
        }
    }

    #[cfg(feature = "load")]
    fn finish_cut_limit(&mut self, error: String) {
        self.load_identity = None;
        self.source_identity = None;
        self.watchdog.cancel_clear();
        self.active_tick_error_generation = None;
        self.last_error = Some(error);
        self.want_run = false;
        self.state = RunState::Error;
        self.runtime_generation = self.runtime_generation.wrapping_add(1);
        self.last_settings_fp = None;
    }

    #[cfg(feature = "load")]
    fn complete_stop(&mut self) {
        self.stop_rx = None;
        self.last_snapshot = None;
        self.last_world_id = None;
        self.ipc = IsolateBuf::new();
        self.pending_withdraw_x = None;
        self.pending_bank_op = None;
        self.work_epoch = self.work_epoch.wrapping_add(1);
        match std::mem::replace(&mut self.after_stop, AfterStop::Idle) {
            AfterStop::Idle => {
                self.teardown_compiled(StopReason::Operator);
                self.compiled_interacts.clear();
                self.compiled_interact_outcome_seqs.clear();
                self.watchdog.cancel_clear();
                self.want_run = false;
                self.state = RunState::Idle;
            }
            AfterStop::Fail(e) => self.fail_setup(e),
            AfterStop::CutLimit(error) => self.finish_cut_limit(error),
            // The cooldown was stamped when the restart was decided.
            AfterStop::Restart => match self.load_identity.clone() {
                Some(identity) => {
                    if let Err(e) = self.spawn_isolate(identity, true) {
                        self.fail_setup(e);
                    }
                }
                None => self.fail_setup("watchdog restart: no retained identity".into()),
            },
            AfterStop::Start => match self.load_identity.clone() {
                Some(identity) => {
                    if let Err(e) = self.spawn_isolate(identity, false) {
                        self.fail_setup(e);
                    }
                }
                None => self.fail_setup("start: no retained identity".into()),
            },
        }
    }

    /// A setup that never reached Ready: no runtime ran, so the slot goes
    /// back to Idle with the diagnostic Start used to return, and the
    /// generation that setup took is given back.
    #[cfg(feature = "load")]
    fn fail_setup(&mut self, e: String) {
        self.last_error = Some(e.clone());
        self.active_tick_error_generation = None;
        self.pending_logs.push(e.clone());
        self.load_identity = None;
        self.source_identity = None;
        if let Some(base) = self.setup_generation_base.take() {
            self.runtime_generation = base;
        }
        self.watchdog.cancel_clear();
        self.last_settings_fp = None;
        self.want_run = false;
        self.state = RunState::Idle;
        self.settle_start(StartOutcome::Failed(e));
    }

    /// Operator Pause: `want_run` stays false (survives login) until
    /// Resume. Instance kept. The host ends the script's route and carries
    /// it to Resume; a watchdog recovery walk is re-armed on Resume instead
    /// ([`ProgressWatchdog::defer_recovery`]).
    pub fn pause(&mut self) {
        self.want_run = false;
        self.native_runtime.clock.observe(Instant::now(), false);
        self.revoke_native_input();
        self.interrupt_compiled(Interrupt::Pause);
        #[cfg(feature = "load")]
        self.interrupt_api(Interrupt::Pause);
        if let Some(pending) = &mut self.pending_withdraw_x {
            pending.freeze();
        }
        if let Some(pending) = &mut self.pending_bank_op {
            pending.freeze();
        }
        if self.has_instance() && matches!(self.state, RunState::Running | RunState::Starting) {
            #[cfg(feature = "load")]
            if let Some(isolate) = &self.load {
                isolate.pause();
            }
            {
                // A recovery walk is resumed with the script, not dropped.
                let _ = self.watchdog.defer_recovery();
                let _ = self.watchdog.set_frozen(true, Instant::now());
            }
            self.state = RunState::Paused;
        }
    }

    /// Operator Resume: `want_run` back on. Assumes the client is up; the
    /// next `on_is_up(false)` re-gates if it is not. No-op when there is
    /// no instance or the slot errored.
    pub fn resume(&mut self) {
        if self.state == RunState::Error {
            return;
        }
        self.interrupt_compiled(Interrupt::Resume);
        #[cfg(feature = "load")]
        self.interrupt_api(Interrupt::Resume);
        if self.state == RunState::Error {
            return;
        }
        self.want_run = true;
        if let Some(pending) = &mut self.pending_withdraw_x {
            pending.resume();
        }
        if let Some(pending) = &mut self.pending_bank_op {
            pending.resume();
        }
        if self.has_instance() && self.state == RunState::Paused {
            #[cfg(feature = "load")]
            if let Some(isolate) = &self.load {
                isolate.resume();
            }
            {
                let _ = self.watchdog.set_frozen(false, Instant::now());
            }
            self.state = {
                #[cfg(feature = "load")]
                {
                    if self.load.is_some() && !self.load_ready {
                        RunState::Starting
                    } else {
                        RunState::Running
                    }
                }
                #[cfg(not(feature = "load"))]
                {
                    RunState::Running
                }
            };
            self.native_input.resume();
        }
    }

    /// Operator Stop: start isolate teardown without blocking. The join
    /// (onStop hook plus the 2 s cap) runs on a reaper; observe completes
    /// it. Compiled teardown still runs on this thread.
    pub fn stop(&mut self) {
        if let Some(run) = self.compiled.as_ref() {
            match catch_unwind(AssertUnwindSafe(|| run.script.prayer_cleanup())) {
                Ok(owned) => self.stop_prayer_cleanup.merge(owned),
                Err(payload) => {
                    self.pending_logs.push(format!(
                        "prayer cleanup snapshot panic: {}",
                        panic_message(&payload)
                    ));
                }
            }
        }
        self.stop_with_reason(StopReason::Operator, "operator stop");
    }

    /// Transfer only accepted Combat raises to the ordinary host off-click pump.
    pub fn take_stop_prayer_cleanup(&mut self) -> crate::combat::RaisedPrayers {
        std::mem::take(&mut self.stop_prayer_cleanup)
    }

    /// Retire a removed slot, distinct from the operator's Stop command.
    pub fn stop_removed(&mut self) {
        self.stop_with_reason(StopReason::Removed, "removed");
    }

    fn stop_with_reason(&mut self, reason: StopReason, message: &'static str) {
        self.terminal_native_status = None;
        let native = self.compiled.is_some() || self.preparing.is_some();
        if native {
            self.lifecycle_receipt = Some(ScriptLifecycleReceipt {
                runtime_generation: self.control_generation.max(self.runtime_generation),
                state: ScriptTerminalState::Cancelled,
                tick: self.ticks,
                reason: message.into(),
            });
        }
        self.control_generation = self.control_generation.saturating_add(1);
        self.preparing = None;
        #[cfg(feature = "load")]
        self.teardown_api(reason);
        self.native_runtime.revoke();
        self.retained = None;
        self.revoke_native_input();
        // The compiled clue machine's abort belongs to the pump thread (its
        // runtime is thread-local, and Stop may arrive on the control thread):
        // mark it here, and `sync_compiled_clue` applies it on the slot's next
        // observed frame. A Load slot's isolate thread owns that machine.
        // Stop is a fresh task instance — the machine's step, its strip list
        // and its abandon latch all start over — which is the marker
        // `reset_session_work` must not make: that one is a connection
        // boundary and drops the step alone.
        #[cfg(feature = "load")]
        if !self.load_active() && self.state != RunState::Stopping {
            self.clue_stop_owed = true;
            self.clue_abort_owed = false;
        }
        // A failed setup already ends Idle with its diagnostic; a Stop
        // during its reap must not turn that into a silent cancel.
        #[cfg(feature = "load")]
        if self.state == RunState::Stopping && matches!(self.after_stop, AfterStop::Fail(_)) {
            return;
        }
        self.last_error = None;
        self.teardown_compiled(reason);
        #[cfg(feature = "load")]
        {
            self.active_tick_error_generation = None;
        }
        // A Start that has not reached Ready never ran: nothing to commit.
        self.settle_start(StartOutcome::Cancelled);
        #[cfg(feature = "load")]
        if self.state == RunState::Stopping {
            // Already reaping: a queued Start or a watchdog restart after
            // it is dropped, and the reap ends Idle.
            self.after_stop = AfterStop::Idle;
            self.setup_generation_base = None;
            self.want_run = false;
            self.load_identity = None;
            self.source_identity = None;
            self.runtime_generation = self.runtime_generation.wrapping_add(1);
            self.watchdog.cancel_clear();
            self.last_settings_fp = None;
            return;
        }
        #[cfg(feature = "load")]
        if self.begin_async_stop(AfterStop::Idle) {
            self.want_run = false;
            return;
        }
        self.finish_idle_stop();
    }

    /// Recompute the gate from client presence. With an instance, the slot
    /// is Running only when `up && want_run`; every other combination is
    /// Paused. Without an instance the state is untouched (Idle, or Error
    /// after a panic — `is_up` must not resurrect or wipe an error).
    pub fn on_is_up(&mut self, up: bool) {
        if !up {
            self.native_input.sync_live(false);
            #[cfg(feature = "load")]
            self.discard_mouse_interacts();
            if self.pending_withdraw_x.is_some() {
                self.complete_current_withdrawal(false);
            }
            if self.pending_bank_op.is_some() {
                self.complete_bank_op(false);
            }
            self.work_epoch = self.work_epoch.wrapping_add(1);
        }
        if !self.has_instance() {
            return;
        }
        if matches!(self.state, RunState::Starting | RunState::Stopping) {
            return;
        }
        self.state = if up && self.want_run {
            RunState::Running
        } else {
            RunState::Paused
        };
    }

    /// Re-gate a started script and invalidate deferred actions and snapshot
    /// deltas at a deliberate connection boundary (operator logout, Stop, or
    /// removal): a Load script's in-flight machine rows and task runtimes end.
    /// Operator run intent is retained.
    pub fn reset_session_work(&mut self) {
        self.session_boundary(false);
    }

    /// The connection dropped unexpectedly: as
    /// [`SlotScript::reset_session_work`], except that a Load script's work
    /// is held whole for the next session, whether automatic relog proceeds
    /// or a repeat guard waits for explicit Log in
    /// ([`LoadIsolate::reconnect_session_work`]); a compiled script's live
    /// step still ends. Returns whether the host should re-arm, on the new
    /// session, the script walk it was following: the script's work was held
    /// and the walk is not the watchdog's own recovery walk, which the
    /// boundary ends.
    pub fn reconnect_session_work(&mut self) -> bool {
        self.session_boundary(true)
    }

    fn session_boundary(&mut self, reconnect: bool) -> bool {
        self.native_runtime.revoke();
        self.native_runtime.clock.observe(Instant::now(), false);
        self.on_is_up(false);
        // The same run continues in a new session: evidence, owners and
        // receipts stamped before the boundary cannot meet a fence after it.
        if let Some(run) = self.compiled.as_mut() {
            run.rekey_session(self.work_epoch);
        }
        #[cfg(feature = "load")]
        self.api_session_boundary(reconnect);
        #[cfg(feature = "load")]
        if !self.load_active() {
            // Same rule as Stop: the compiled machine's abort lands on the
            // pump thread, never here. A connection boundary aborts the live
            // step and its token and keeps the rest of the session — the
            // Entrana strip list and the abandon latch survive a relog — so
            // this is the weaker marker and never `clue_stop_owed`.
            self.clue_abort_owed = true;
        }
        #[cfg(feature = "load")]
        {
            let carried_error = self.active_tick_error_generation.is_some();
            let held = reconnect && self.load.is_some();
            if let Some(isolate) = &self.load {
                let generation = if held {
                    isolate.reconnect_session_work()
                } else {
                    isolate.reset_session_work()
                };
                self.active_tick_error_generation = carried_error.then_some(generation);
            } else {
                self.active_tick_error_generation = None;
            }
            self.last_snapshot = None;
            self.last_world_id = None;
            self.reach_cache.clear();

            let abort = self.watchdog.abort_owned_recovery();
            let _ = self.watchdog.on_session_reset(Instant::now());
            held && abort != WatchdogAction::AbortWalk
        }
        #[cfg(not(feature = "load"))]
        {
            let _ = reconnect;
            false
        }
    }

    /// Post the host's FlatBuffer snapshot blob into a Load isolate (no-op
    /// for a compiled script). Call it before [`SlotScript::on_game_tick`]
    /// so the posted blob is what the tick's JS reads. `false`: the wedged
    /// isolate refused the post — nothing in it reached the script, the
    /// next encode is a keyframe, and the isolate refuses the paired tick.
    #[cfg(feature = "load")]
    pub fn post_snapshot(&mut self, bytes: Vec<u8>) -> bool {
        let Some(isolate) = &self.load else {
            return false;
        };
        let accepted = isolate.post_snapshot(bytes);
        if !accepted {
            self.last_snapshot = None;
        }
        accepted
    }
    /// Deliver one broker FlatBuffer batch to the current Load runtime.
    /// Generation/lifecycle fencing is performed by the Play broker before
    /// this crossing; a compiled card has no BroadcastChannel surface.
    #[cfg(feature = "load")]
    pub fn post_channel_events(&mut self, bytes: Vec<u8>) -> bool {
        self.load
            .as_ref()
            .is_some_and(|isolate| isolate.post_channel_events(bytes))
    }

    /// Post the merged operator settings bag into a Load isolate.
    #[cfg(feature = "load")]
    pub fn post_settings_bag(&mut self, bag: &serde_json::Map<String, serde_json::Value>) {
        if self.load.is_none() {
            return;
        }
        let fp = settings_fp(bag);
        if self.last_settings_fp.as_deref() == Some(fp.as_str()) {
            return;
        }
        if let Some(isolate) = &self.load {
            isolate.post_settings_bag(bag);
        }
        if let Some(identity) = &mut self.load_identity {
            identity.settings_bag = Some(Arc::new(bag.clone()));
        }
        self.last_settings_fp = Some(fp);
    }

    /// Deliver settings only when identity and runtime generation match.
    /// Returns false when the edit is stale or the bag is unchanged.
    #[cfg(feature = "load")]
    pub fn post_settings_bag_fenced(
        &mut self,
        bag: &serde_json::Map<String, serde_json::Value>,
        identity: &str,
        generation: u64,
    ) -> bool {
        if self.source_identity.as_deref() != Some(identity) {
            return false;
        }
        if self.runtime_generation != generation {
            return false;
        }
        let fp = settings_fp(bag);
        if self.last_settings_fp.as_deref() == Some(fp.as_str()) {
            return false;
        }
        if self.state == RunState::Stopping {
            if !matches!(self.after_stop, AfterStop::Start | AfterStop::Restart) {
                return false;
            }
            let Some(load_identity) = &mut self.load_identity else {
                return false;
            };
            load_identity.settings_bag = Some(Arc::new(bag.clone()));
            self.last_settings_fp = Some(fp);
            return true;
        }
        match self.state {
            RunState::Running | RunState::Paused | RunState::Starting => {}
            _ => return false,
        }
        if self.load.is_none() {
            return false;
        }
        self.post_settings_bag(bag);
        true
    }

    pub fn attach_source_identity(&mut self, key: impl Into<String>) {
        self.source_identity = Some(key.into());
    }

    pub fn source_identity(&self) -> Option<&str> {
        self.source_identity.as_deref()
    }

    pub fn runtime_generation(&self) -> u64 {
        self.runtime_generation
    }

    pub fn native_input(&self) -> Arc<NativeInputAuthority> {
        Arc::clone(&self.native_input)
    }

    /// Selected-revision facts pinned by a compiled run or a live Load seat.
    pub fn compiled_game_data(&self) -> Option<Arc<api::game_data::SelectedGameData>> {
        let selected = self.compiled.as_ref().map(|run| Arc::clone(&run.selected));
        #[cfg(feature = "load")]
        let selected = selected.or_else(|| self.api_game_data());
        selected
    }

    /// Pump-thread sync of the compiled clue machine: the owed abort or
    /// instance reset (Stop, a session reset, a panic) and this frame's
    /// pause/hold freeze, applied whether or not the frame dispatches a tick.
    ///
    /// Call it on the slot's own thread, once per observed frame — never from
    /// the control thread, and never for a Load slot (that slot's isolate
    /// thread runs the same hooks for its own instance). The machine's
    /// runtime is thread-local, so the instance this reaches is exactly the
    /// one this slot's compiled card calls.
    pub fn sync_compiled_clue(&mut self, held: bool) {
        let now = std::time::Instant::now();
        // The safety lease uses wall time even when no script tick is eligible.
        let _ = self.native_quiet_read(now);
        // Pause, hold, not-ready, recovery hold and Blocked freeze the clock.
        let eligible = self.dispatch_open() && !held;
        self.native_runtime.clock.observe(now, eligible);
        #[cfg(feature = "load")]
        {
            if self.load_active() {
                return;
            }
            if self.clue_stop_owed {
                // Stop is a fresh task instance: the whole session starts
                // over, strip list and abandon latch included. It subsumes
                // any session abort marked before it.
                self.clue_stop_owed = false;
                self.clue_abort_owed = false;
                crate::clue::on_stop();
            } else if self.clue_abort_owed {
                // The abort is about the machine's own session, not the
                // instance, so it is consumed even when no card is installed —
                // and it keeps the strip list the session still owes a reclaim
                // for.
                self.clue_abort_owed = false;
                crate::clue::on_reset();
            }
            if self.compiled.is_none() {
                return;
            }
            if self.state == RunState::Running && self.want_run {
                crate::clue::on_resume();
            } else {
                crate::clue::on_pause();
            }
            crate::clue::on_hold(held);
        }
        #[cfg(not(feature = "load"))]
        let _ = held;
    }

    /// Share the host SlotInput authority. Call on spawn before Start.
    pub fn bind_native_input(&mut self, authority: Arc<NativeInputAuthority>) {
        self.native_input = authority;
    }

    /// Bind the server profile's world type (members or free-to-play). Call on
    /// spawn before Start; a slot with no profile keeps `Unknown`.
    pub fn bind_world_members(&mut self, world_members: api::selected::Truth) {
        self.world_members = world_members;
    }

    /// The world type bound by [`SlotScript::bind_world_members`].
    pub fn world_members(&self) -> api::selected::Truth {
        self.world_members
    }

    fn revoke_native_input(&mut self) {
        self.native_input.revoke();
        #[cfg(feature = "load")]
        self.discard_mouse_interacts();
    }

    #[cfg(feature = "load")]
    fn discard_mouse_interacts(&self) {
        if let Some(isolate) = &self.load {
            isolate.discard_mouse_interacts();
        }
    }

    /// Whether the script may dispatch at all: Running by operator intent,
    /// not in a watchdog recovery hold, and not Blocked (which closes
    /// dispatch until Retry, though the instance is retained).
    fn dispatch_open(&self) -> bool {
        let blocked = self
            .compiled
            .as_ref()
            .and_then(|run| run.output.status.as_deref())
            .is_some_and(|status| status.phase == crate::native::NativePhase::Blocked);
        self.state == RunState::Running
            && self.want_run
            && self.has_instance()
            && !self.watchdog.holds_script_actions()
            && !blocked
            && !self
                .quest_pairs
                .as_ref()
                .zip(self.native_run())
                .is_some_and(|(pairs, run)| pairs.held(run))
    }

    pub fn sync_native_input_gate(&self) {
        let live = self.dispatch_open();
        self.native_input.sync_live(live);
        if !live {
            #[cfg(feature = "load")]
            self.discard_mouse_interacts();
        }
    }

    #[cfg(feature = "load")]
    pub fn post_loadouts(&mut self, loadouts: &[crate::loadouts_store::Loadout]) {
        if let Some(isolate) = &self.load {
            isolate.post_loadouts(loadouts);
        }
        if let Some(identity) = &mut self.load_identity {
            identity.loadouts = Arc::from(loadouts.to_vec());
        }
    }

    /// Start a JS Load isolate and optionally post the operator settings bag.
    #[cfg(feature = "load")]
    pub fn start_load_with_settings(
        &mut self,
        source: String,
        shape: LoadShape,
        bag: Option<&serde_json::Map<String, serde_json::Value>>,
        siblings: Vec<(String, String)>,
    ) -> Result<(), String> {
        self.start_load_with_settings_and_game_data(
            source,
            shape,
            bag,
            siblings,
            None,
            std::sync::Arc::new(api::named_banks::NamedBankFacts::empty()),
        )
    }

    /// Start with settings and the immutable facts selected by the owning Play.
    #[cfg(feature = "load")]
    pub fn start_load_with_settings_and_game_data(
        &mut self,
        source: String,
        shape: LoadShape,
        bag: Option<&serde_json::Map<String, serde_json::Value>>,
        siblings: Vec<(String, String)>,
        game_data: Option<std::sync::Arc<api::game_data::SelectedGameData>>,
        named_banks: std::sync::Arc<api::named_banks::NamedBankFacts>,
    ) -> Result<(), String> {
        self.start_load_with_settings_and_game_data_typed(
            source,
            shape,
            bag,
            siblings,
            game_data,
            named_banks,
        )
        .map_err(|e| e.to_string())
    }

    /// Typed initial-start boundary; refuses active slots before evaluating source.
    #[cfg(feature = "load")]
    pub fn start_load_with_settings_and_game_data_typed(
        &mut self,
        source: String,
        shape: LoadShape,
        bag: Option<&serde_json::Map<String, serde_json::Value>>,
        siblings: Vec<(String, String)>,
        game_data: Option<std::sync::Arc<api::game_data::SelectedGameData>>,
        named_banks: std::sync::Arc<api::named_banks::NamedBankFacts>,
    ) -> Result<(), StartLoadError> {
        let loadouts = crate::loadouts_store::LoadoutsStore::with_default_path();
        if let Some(bag) = bag {
            let wanted = bag.get("loadout").map(|value| value.as_str().unwrap_or(""));
            crate::loadouts_store::resolve_script_loadout_setting(loadouts.loadouts(), wanted)
                .map(|_| ())
                .map_err(|error| StartLoadError::Refused(error.to_string()))?;
        }
        self.start_load_with_loadouts_and_game_data_typed(
            source,
            shape,
            siblings,
            loadouts.loadouts(),
            game_data,
            named_banks,
        )?;
        if let Some(bag) = bag {
            self.post_settings_bag(bag);
        }
        Ok(())
    }

    /// Encode `input` into this slot's reusable isolate buffer and return
    /// the finished bytes. Stores the new last-post fingerprint so the
    /// next observe is a delta. Disjoint-field borrow of `ipc` and
    /// `last_snapshot` — no extra fingerprint clone.
    #[cfg(feature = "load")]
    pub fn encode_snapshot_delta(
        &mut self,
        input: &crate::isolate_fb::SnapshotInput<'_>,
        force_banks: bool,
    ) -> Vec<u8> {
        self.encode_snapshot_delta_with_native(
            input,
            crate::isolate_fb::NativeFactsInput::default(),
            force_banks,
        )
    }

    #[cfg(feature = "load")]
    pub fn encode_snapshot_delta_with_native(
        &mut self,
        input: &crate::isolate_fb::SnapshotInput<'_>,
        native: crate::isolate_fb::NativeFactsInput<'_>,
        force_banks: bool,
    ) -> Vec<u8> {
        let mut native = native;
        native.api_gather = self.api.as_ref().and_then(|seat| seat.page.as_deref());
        native.api_gather_outcome = self.api.as_ref().and_then(|seat| seat.terminal.as_ref());
        native.api_progress = self
            .api
            .as_ref()
            .and_then(|seat| seat.progress_page.as_ref());
        let (bytes, fp) = self.ipc.encode_snapshot_delta_with_native(
            self.last_snapshot.as_ref(),
            input,
            native,
            force_banks,
        );
        self.last_snapshot = Some(fp);
        bytes
    }

    #[cfg(feature = "load")]
    pub fn encode_snapshot_wake_with_native(
        &mut self,
        input: &crate::isolate_fb::SnapshotInput<'_>,
        native: crate::isolate_fb::NativeFactsInput<'_>,
        force_banks: bool,
        preserve_inv: bool,
    ) -> Vec<u8> {
        let mut native = native;
        native.api_gather = self.api.as_ref().and_then(|seat| seat.page.as_deref());
        native.api_gather_outcome = self.api.as_ref().and_then(|seat| seat.terminal.as_ref());
        native.api_progress = self
            .api
            .as_ref()
            .and_then(|seat| seat.progress_page.as_ref());
        let (bytes, fp) = self.ipc.encode_snapshot_wake_with_native(
            self.last_snapshot.as_mut(),
            input,
            native,
            force_banks,
            preserve_inv,
        );
        self.last_snapshot = Some(fp);
        bytes
    }

    /// Last accepted snapshot identities, used to detect host completions
    /// without retaining another per-slot generation table.
    #[cfg(feature = "load")]
    pub fn last_snapshot_fingerprint(&self) -> Option<&SnapshotFingerprint> {
        self.last_snapshot.as_ref()
    }

    /// The `NavWorld` identity the packed banks were posted against
    /// (`None` before the first post / after a Start). A world rebuild
    /// changes the identity, forcing the banks table onto the next post.
    #[cfg(feature = "load")]
    pub fn last_world_id(&self) -> Option<usize> {
        self.last_world_id
    }

    /// Store the `NavWorld` identity the packed banks were just posted
    /// against.
    #[cfg(feature = "load")]
    pub fn store_last_world_id(&mut self, id: Option<usize>) {
        self.last_world_id = id;
    }

    #[cfg(feature = "load")]
    pub fn reach_pack_cache(&mut self) -> &mut api::query::ReachPackCache {
        &mut self.reach_cache
    }

    pub fn native_quiet_read(
        &mut self,
        now: std::time::Instant,
    ) -> Option<crate::native::QuietReadOwner> {
        self.native_runtime.ledger.as_mut()?.quiet_read(now)
    }

    /// Whether a native or compatibility journal currently owns the hidden
    /// paint lease. Both paths are wall-clock revalidated on every read.
    pub fn journal_paint_hidden(&mut self, now: std::time::Instant) -> bool {
        if self.native_quiet_read(now).is_some() {
            return true;
        }
        #[cfg(feature = "load")]
        {
            self.load
                .as_ref()
                .is_some_and(|isolate| isolate.compat_journal_paint_hidden(now))
        }
        #[cfg(not(feature = "load"))]
        {
            false
        }
    }

    /// Live walk ownership, not a retained destination or reconnect carry.
    pub fn live_walking_operation(&self) -> bool {
        if self.native_runtime.ledger.as_ref().is_some_and(|ledger| {
            ledger
                .owner
                .as_ref()
                .is_some_and(|owner| owner.active_walk().is_some())
                && !ledger
                    .walk
                    .as_ref()
                    .is_some_and(|receipt| receipt.end == crate::native::WalkEnd::UserInput)
        }) {
            return true;
        }
        #[cfg(feature = "load")]
        if let Some(isolate) = &self.load {
            return isolate.live_walking_operation();
        }
        false
    }

    pub fn note_manual_walk_takeover(&mut self, intent_seq: u64, tick: u64) {
        // A deliberate takeover is gameplay progress, even if the human
        // clicked an unwalkable tile and no player-info movement follows.
        self.watchdog.stamp_gameplay(Instant::now());
        #[cfg(feature = "load")]
        if let Some(isolate) = &self.load {
            isolate.note_manual_walk_takeover(intent_seq);
        }
        #[cfg(not(feature = "load"))]
        let _ = intent_seq;
        let Some(ledger) = self.native_runtime.ledger.as_mut() else {
            return;
        };
        // A queued native walk has not reached NavBot yet; settle the same
        // owned request directly and leave its owner live to consume UserInput.
        if let Some(action) = ledger.outbox.iter().find(|action| {
            action.live() && matches!(action.effect, crate::native::HostEffect::Walk(_))
        }) {
            let request_id = action.request_id.get();
            ledger.walk = Some(crate::native::WalkReceipt {
                request_id,
                evidence: api::quest_progress::EvidenceStamp {
                    run: action.run(),
                    tick,
                    sequence: tick,
                },
                end: crate::native::WalkEnd::UserInput,
                blocked: None,
                detail: None,
                refusal: None,
                assessment: None,
                escape: None,
            });
            ledger
                .outbox
                .retain(|action| action.request_id.get() != request_id);
        }
    }

    /// Only observations update this; queued work keeps its original stamp.
    pub fn observe_walk_outcome_seq(&mut self, seq: u64) -> bool {
        let changed = self.native_runtime.observed_walk_outcome_seq != seq;
        self.native_runtime.observed_walk_outcome_seq = seq;
        changed
    }

    /// Queued native work the host may dispatch now. Work queued before a
    /// watchdog recovery hold waits for it to end; Blocked revoked its work.
    pub fn has_native_actions(&self) -> bool {
        self.native_runtime
            .ledger
            .as_ref()
            .is_some_and(|ledger| ledger.outbox.iter().any(|action| action.live()))
            && self.dispatch_open()
    }

    /// The host calls this only while holding the slot's final dispatch fence.
    /// Keep the bounded outbox's allocation for the next observed tick.
    pub fn take_native_action(&mut self) -> Option<crate::native::HostAction> {
        if !self.dispatch_open() {
            return None;
        }
        let ledger = self.native_runtime.ledger.as_mut()?;
        while !ledger.outbox.is_empty() {
            let action = ledger.outbox.remove(0);
            if action.live() {
                return Some(action);
            }
        }
        None
    }

    pub fn complete_native_walk(
        &mut self,
        authority: &crate::native::HostAuthority,
        receipt: crate::native::WalkReceipt,
    ) {
        if !authority.live()
            || authority.request_id().get() != receipt.request_id
            || authority.run() != receipt.evidence.run
        {
            return;
        }
        let Some(ledger) = self.native_runtime.ledger.as_mut() else {
            return;
        };
        if ledger.owner.as_ref().is_some_and(|owner| {
            owner.run == authority.run() && owner.id == authority.action_id() && owner.live()
        }) {
            ledger.walk = Some(receipt);
        }
    }

    /// Publish a non-terminal walk warning to the same run/action/request
    /// fence as a terminal receipt, without clearing that request's authority.
    pub fn notify_native_walk(
        &mut self,
        authority: &crate::native::HostAuthority,
        event: crate::native::WalkEvent,
    ) {
        if !authority.live()
            || authority.request_id().get() != event.request_id
            || authority.run() != event.evidence.run
        {
            return;
        }
        let Some(ledger) = self.native_runtime.ledger.as_mut() else {
            return;
        };
        if ledger.owner.as_ref().is_some_and(|owner| {
            owner.run == authority.run() && owner.id == authority.action_id() && owner.live()
        }) {
            ledger.walk_events.push(event);
        }
    }

    pub fn complete_native_interaction(
        &mut self,
        authority: &crate::native::HostAuthority,
        receipt: crate::native::InteractionReceipt,
    ) {
        if let Some(ledger) = self.native_runtime.ledger.as_mut() {
            ledger.complete_interaction(authority, receipt);
        }
    }

    pub fn complete_native_bank_pick(
        &mut self,
        authority: &crate::native::HostAuthority,
        receipt: crate::native_bank::BankPickReceipt,
    ) {
        if let Some(ledger) = self.native_runtime.ledger.as_mut() {
            ledger.complete_bank_pick(authority, receipt);
        }
    }

    pub fn complete_native_assess_walk(
        &mut self,
        authority: &crate::native::HostAuthority,
        receipt: crate::native::AssessReceipt,
    ) {
        if let Some(ledger) = self.native_runtime.ledger.as_mut() {
            ledger.complete_assess_walk(authority, receipt);
        }
    }

    /// Diagnostic/test drain without the observed-outcome stamp. Production
    /// host dispatch uses [`Self::drain_host_interacts`] so pre-takeover walking
    /// decisions cannot become fresh merely by passing through a raw queue.
    #[cfg(feature = "load")]
    pub fn drain_interacts(&mut self) -> Vec<crate::shim::InteractReq> {
        self.drain_interacts_stamped()
            .into_iter()
            .map(|queued| queued.req)
            .collect()
    }

    #[cfg(feature = "load")]
    fn drain_interacts_stamped(&mut self) -> Vec<crate::load::QueuedInteract> {
        debug_assert!(
            !(self.compiled.is_some() && self.load.is_some()),
            "a slot never owns both a compiled script and a Load isolate"
        );
        match &self.load {
            Some(isolate) => isolate.drain_interacts_stamped(),
            None => {
                let reqs = std::mem::take(&mut self.compiled_interacts);
                let stamps = std::mem::take(&mut self.compiled_interact_outcome_seqs);
                debug_assert_eq!(reqs.len(), stamps.len());
                reqs.into_iter()
                    .zip(stamps)
                    .map(
                        |(req, observed_walk_outcome_seq)| crate::load::QueuedInteract {
                            req,
                            observed_walk_outcome_seq,
                        },
                    )
                    .collect()
            }
        }
    }

    /// Drain one host frame's decoded isolate messages, separating the latest
    /// run-policy replacement from game/lifecycle interacts. The outer option
    /// is "an update was sent"; the inner option is replace versus clear.
    #[cfg(feature = "load")]
    pub fn drain_host_interacts(
        &mut self,
    ) -> (
        Option<Option<api::run_policy::RunPolicyOverride>>,
        Vec<crate::load::QueuedInteract>,
        bool,
    ) {
        let mut reqs = self.drain_interacts_stamped();
        let mut policy = None;
        let mut owned = self.api_owns_foreground();
        reqs.retain(|queued| match &queued.req {
            crate::shim::InteractReq::RunPolicyOverride { policy: update } => {
                policy = Some(*update);
                false
            }
            crate::shim::InteractReq::GatherRun { .. }
            | crate::shim::InteractReq::GatherStop { .. }
            | crate::shim::InteractReq::ProgressRead { .. } => {
                self.consume_api_control(&queued.req);
                owned |= self.api_owns_foreground();
                false
            }
            _ => true,
        });
        if owned {
            let before = reqs.len();
            reqs.retain(|queued| !queued.req.is_game());
            // Reconnect rows are script game work too; discard them at this
            // admission edge rather than replaying an old route after the seat.
            let held = self.take_held_host_walks();
            self.record_api_dropped_rows(before - reqs.len() + held.len());
        }
        if !self.api_owns_foreground() {
            self.log_api_drop_total();
        }
        (policy, reqs, owned)
    }

    /// Diagnostic/test injection: stamps raw rows with the runtime's current
    /// observed outcome. Production Pause/restoration must retain the original
    /// stamp through [`Self::restore_host_interacts`].
    #[cfg(feature = "load")]
    pub fn restore_interacts(&mut self, drained: Vec<crate::shim::InteractReq>) {
        debug_assert!(
            !(self.compiled.is_some() && self.load.is_some()),
            "a slot never owns both a compiled script and a Load isolate"
        );
        match &self.load {
            Some(isolate) => isolate.restore_interacts(drained),
            None => {
                let seq = self.native_runtime.observed_walk_outcome_seq;
                self.restore_host_interacts(
                    drained
                        .into_iter()
                        .map(|req| crate::load::QueuedInteract {
                            req,
                            observed_walk_outcome_seq: seq,
                        })
                        .collect(),
                );
            }
        }
    }

    #[cfg(feature = "load")]
    pub fn restore_host_interacts(&mut self, drained: Vec<crate::load::QueuedInteract>) {
        match &self.load {
            Some(isolate) => isolate.restore_interacts_stamped(drained),
            None => {
                let (mut reqs, mut stamps): (Vec<_>, Vec<_>) = drained
                    .into_iter()
                    .map(|queued| (queued.req, queued.observed_walk_outcome_seq))
                    .unzip();
                reqs.append(&mut self.compiled_interacts);
                stamps.append(&mut self.compiled_interact_outcome_seqs);
                self.compiled_interacts = reqs;
                self.compiled_interact_outcome_seqs = stamps;
            }
        }
    }

    #[cfg(feature = "load")]
    pub fn take_held_host_walks(&self) -> Vec<crate::load::QueuedInteract> {
        self.load
            .as_ref()
            .map_or_else(Vec::new, |isolate| isolate.take_held_walks_stamped())
    }

    /// Diagnostic/test reconnect drain that discards stamps. Production replay
    /// uses [`Self::take_held_host_walks`] to retain the original decision fence.
    #[cfg(feature = "load")]
    pub fn take_held_walks(&self) -> Vec<crate::shim::InteractReq> {
        match &self.load {
            Some(isolate) => isolate.take_held_walks(),
            None => Vec::new(),
        }
    }

    #[cfg(feature = "load")]
    pub fn drain_lifecycle(&self) -> Vec<crate::shim::InteractReq> {
        match &self.load {
            Some(isolate) => isolate.drain_lifecycle(),
            None => Vec::new(),
        }
    }

    #[cfg(feature = "load")]
    pub fn request_recovery_anchor(&self) {
        if let Some(isolate) = &self.load {
            isolate.request_recovery_anchor();
        }
    }

    #[cfg(feature = "load")]
    pub fn load_identity(&self) -> Option<&SlotLoadIdentity> {
        self.load_identity.as_ref()
    }

    /// API family captured with the active Load identity at Start.
    #[cfg(feature = "load")]
    pub fn api_family(&self) -> Option<crate::ApiFamily> {
        self.load.as_ref()?;
        self.load_identity.as_ref()?.api_family
    }

    pub fn watchdog(&self) -> &ProgressWatchdog {
        &self.watchdog
    }

    /// Record the run's current base levels (game-ready observations only).
    pub fn note_levels(&mut self, levels: impl Iterator<Item = i32>) {
        self.watchdog.note_levels(levels);
    }

    /// The armed run's runtime, gameplay idle time and levels gained.
    pub fn progress(&self, now: Instant) -> Option<crate::watchdog::ScriptProgress> {
        self.watchdog.progress(now)
    }

    /// Apply isolate lifecycle facts and host tile/XP, then decide.
    pub fn feed_watchdog(
        &mut self,
        now: Instant,
        here: Option<(i32, i32, i32)>,
        xp: &[i32],
        frozen: bool,
        running: bool,
        lifecycle: &[crate::shim::InteractReq],
    ) -> WatchdogAction {
        if !self.load_active() && self.compiled.is_none() {
            return WatchdogAction::None;
        }
        let frozen = frozen
            || self.compiled.as_ref().is_some_and(|run| {
                run.output
                    .status
                    .as_ref()
                    .is_some_and(|status| status.phase == crate::native::NativePhase::Blocked)
            });
        let freeze_action = self.watchdog.set_frozen(frozen, now);
        if matches!(freeze_action, WatchdogAction::AbortWalk) {
            return freeze_action;
        }
        let previous_gameplay = self.watchdog.gameplay_stamp();
        use crate::shim::InteractReq;
        let mut anchor_reply = None;
        for op in lifecycle {
            match op {
                InteractReq::NoteProgress => self.watchdog.stamp_note_progress(now),
                InteractReq::LoopSettled => self.watchdog.stamp_scheduler(now),
                InteractReq::WaitEnqueued => self.watchdog.on_wait_enqueued(now),
                InteractReq::WaitSettled => self.watchdog.on_wait_settled(now),
                InteractReq::RecoveryAnchor { x, z, level } => {
                    anchor_reply = Some(Some(WatchdogTile {
                        x: *x,
                        z: *z,
                        level: *level,
                    }));
                }
                InteractReq::RecoveryAnchorNone => anchor_reply = Some(None),
                _ => {}
            }
        }
        if let Some(reply) = anchor_reply {
            let player = here.map(|(x, z, level)| WatchdogTile { x, z, level });
            return self.watchdog.on_anchor(now, player, reply);
        }
        if let Some((x, z, level)) = here {
            let arrived = self.watchdog.on_tile(now, WatchdogTile { x, z, level });
            if arrived != WatchdogAction::None {
                return arrived;
            }
        }
        self.watchdog.on_xp(now, xp);
        if self.watchdog.gameplay_stamp() != previous_gameplay {
            if let (Some(pairs), Some(evidence)) = (&self.quest_pairs, self.pair_evidence) {
                pairs.gameplay_progress(evidence.run, evidence, now);
            }
        }
        let pair_wait = self
            .quest_pairs
            .as_ref()
            .zip(self.native_run())
            .is_some_and(|(pairs, run)| pairs.waiting(run));
        let action = self
            .watchdog
            .observe_with_pair_wait(now, running && self.want_run, pair_wait);
        if action == WatchdogAction::RequestAnchor {
            if let Some(run) = &self.compiled {
                let anchor = match catch_unwind(AssertUnwindSafe(|| run.script.recovery_anchor())) {
                    Ok(anchor) => anchor.map(|tile| WatchdogTile {
                        x: tile.x,
                        z: tile.z,
                        level: tile.level,
                    }),
                    Err(payload) => {
                        self.fail_compiled(ScriptFailure {
                            code: "recovery-anchor-panic".into(),
                            message: panic_message(&payload).into(),
                        });
                        return WatchdogAction::None;
                    }
                };
                return self.watchdog.on_anchor(
                    now,
                    here.map(|(x, z, level)| WatchdogTile { x, z, level }),
                    anchor,
                );
            }
            #[cfg(feature = "load")]
            if let Some(anchor) = self.api_recovery_anchor() {
                return self.watchdog.on_anchor(
                    now,
                    here.map(|(x, z, level)| WatchdogTile { x, z, level }),
                    anchor,
                );
            }
        }
        if matches!(action, WatchdogAction::WarnHungLoop) {
            self.pending_logs
                .push("watchdog: hung loop (10s, no scheduler progress)".into());
        }
        action
    }

    /// Recreate the active execution kind from its effective identity.
    pub fn restart_from_identity(&mut self, now: Instant) -> Result<(), String> {
        if !self.want_run || self.state == RunState::Paused {
            return Err("watchdog restart cancelled: operator is not running".into());
        }
        if self.watchdog.frozen() {
            return Err("watchdog restart cancelled: frozen".into());
        }
        if self.compiled.as_ref().is_some_and(|run| {
            run.output
                .status
                .as_ref()
                .is_some_and(|status| status.phase == crate::native::NativePhase::Blocked)
        }) {
            return Err("watchdog restart cancelled: blocked".into());
        }
        if self.compiled.is_some() {
            return self.restart_compiled(now);
        }
        #[cfg(feature = "load")]
        {
            if self.state == RunState::Stopping {
                // A respawn is already queued behind this reap.
                return match self.after_stop {
                    AfterStop::Restart | AfterStop::Start => Ok(()),
                    AfterStop::Idle | AfterStop::Fail(_) | AfterStop::CutLimit(_) => {
                        Err("watchdog restart cancelled: stopping".into())
                    }
                };
            }
            self.apply_load_restart(now)
        }
        #[cfg(not(feature = "load"))]
        Err("watchdog restart: no retained identity".into())
    }

    /// Common retained-identity recreate used by the stall watchdog and by a
    /// confirmed JavaScript cut. Policy checks belong to the callers: a
    /// Pause-deadline cut deliberately recreates while operator-paused.
    #[cfg(feature = "load")]
    fn apply_load_restart(&mut self, now: Instant) -> Result<(), String> {
        let Some(identity) = self.load_identity.clone() else {
            return Err("watchdog restart: no retained identity".into());
        };
        self.revoke_native_input();
        // Frozen StallGuard: the restarted script finds `pendingRecovery`
        // (`StallGuard.ts:34–39`).
        self.recovery_hints.note_restart();
        if self.begin_async_stop(AfterStop::Restart) {
            // Stamp the recovery and its cooldown at the decision, as the
            // synchronous restart did; the respawn follows the reap.
            self.watchdog.on_restart_applied(now);
            return Ok(());
        }
        self.spawn_isolate(identity, true)?;
        self.watchdog.on_restart_applied(now);
        Ok(())
    }

    pub fn notify_walk_failed(&mut self, now: Instant) -> WatchdogAction {
        self.watchdog.on_walk_failed(now)
    }

    pub fn notify_hold_during_walk(&mut self) -> WatchdogAction {
        self.watchdog.on_hold_during_walk()
    }

    pub fn abort_owned_recovery(&mut self) -> WatchdogAction {
        self.watchdog.abort_owned_recovery()
    }

    /// Latest shared output frame, from the single active execution kind.
    pub fn paint(&self) -> Option<Arc<crate::shim::ScriptPaint>> {
        if let Some(run) = &self.compiled {
            return run.output.paint.clone();
        }
        #[cfg(feature = "load")]
        {
            self.load.as_ref().and_then(|iso| iso.paint())
        }
        #[cfg(not(feature = "load"))]
        {
            None
        }
    }

    /// Forward a one-shot paint-button id to the Load isolate. No-op for a
    /// compiled script or a slot with no isolate.
    #[cfg(feature = "load")]
    pub fn paint_click(&self, id: &str) {
        if let Some(isolate) = &self.load {
            isolate.paint_click(id);
        }
    }

    /// Forward a persistent strip/rail/tabs selection to the Load isolate.
    /// No-op for a compiled script or a slot with no isolate.
    #[cfg(feature = "load")]
    pub fn paint_select(&self, key: &str, name: &str) {
        if let Some(isolate) = &self.load {
            isolate.paint_select(key, name);
        }
    }

    /// The bot instance's random-ignore list (a Load isolate's
    /// `inst.ignoredRandoms?.()`, default `[]`; empty for a compiled
    /// script — there is no instance). Cached on the isolate thread
    /// after each tick; no probe round-trip.
    #[cfg(feature = "load")]
    pub fn ignored_randoms(&self) -> Vec<String> {
        if self.state != RunState::Running || !self.want_run {
            return Vec::new();
        }
        match &self.load {
            Some(isolate) => isolate.ignored_randoms(),
            None => Vec::new(),
        }
    }

    /// Evaluate `expr` in the Load isolate's global scope (test/status
    /// read-back; e.g. the host asserting a posted snapshot field). Errors
    /// when the slot has no Load isolate.
    #[cfg(feature = "load")]
    pub fn probe(&self, expr: &str) -> Result<serde_json::Value, String> {
        match &self.load {
            Some(isolate) => isolate.probe(expr),
            None => Err("no load isolate".to_string()),
        }
    }
    /// Whether this slot's isolate owns an interruptible execution.
    #[cfg(feature = "load")]
    #[doc(hidden)]
    pub fn load_execution_active(&self) -> bool {
        self.load
            .as_ref()
            .is_some_and(LoadIsolate::execution_active)
    }

    /// Monotonic identity and activity of this slot's latest interruptible
    /// isolate execution.
    #[cfg(feature = "load")]
    #[doc(hidden)]
    pub fn load_execution_sequence(&self) -> (u64, bool) {
        self.load
            .as_ref()
            .map_or((0, false), LoadIsolate::execution_sequence)
    }

    /// Poll evidence-ready machines at the latest observed tick. This does
    /// not dispatch a JS game tick, advance real-tick waits or renew the native
    /// event budget. Held runs remain frozen.
    pub fn on_snapshot_change(&mut self, ctx: &mut ScriptCtx<'_>) {
        self.observe_lifecycle();
        if self.state != RunState::Running || !self.want_run || ctx.compiled.hold {
            return;
        }
        #[cfg(feature = "load")]
        if let Some(isolate) = &self.load {
            if let Some(failure) = trapped_failure(ctx) {
                self.stop_blocked(failure, ctx.tick);
                return;
            }
            isolate.on_snapshot_change_at(ctx.tick, self.native_input.lock().identity());
            self.tick_api(ctx);
            return;
        }
        self.poll_compiled_frame(ctx);
    }

    /// Call only on observed server tick. Dispatches the JS isolate's
    /// `on_game_tick` (compiled path) only while Running && want_run. A
    /// compiled panic is caught: the slot goes Error with the message, the
    /// instance is dropped, the run is over.
    ///
    /// Around a compiled tick this installs the slot's own interact queue as
    /// the ctx's enqueue sink, so the verbs the tick dispatches land on the
    /// same drain the isolate's forwarded requests ride; the queue is taken
    /// back whatever the tick does. A panic also marks the clue machine's
    /// abort, which the slot's next observed frame applies.
    pub fn on_game_tick(&mut self, ctx: &mut ScriptCtx<'_>) {
        self.observe_lifecycle();
        if self.state != RunState::Running || !self.want_run {
            return;
        }
        #[cfg(feature = "load")]
        if self.load.is_some() {
            if let Some(failure) = trapped_failure(ctx) {
                self.stop_blocked(failure, ctx.tick);
                return;
            }
        }
        #[cfg(feature = "load")]
        if let Some(isolate) = &self.load {
            isolate.on_game_tick_at(ctx.tick, self.native_input.lock().identity());
            self.tick_api(ctx);
            return;
        }
        if self.compiled.is_none() {
            return;
        }
        self.ticks += 1;
        self.poll_compiled_frame(ctx);
    }

    /// Both real ticks and evidence wakes use the same compiled frame path.
    /// `Runtime::frame_context` observes the tick idempotently, so a wake can
    /// consume evidence but cannot replenish an already-spent event budget.
    fn poll_compiled_frame(&mut self, ctx: &mut ScriptCtx<'_>) {
        let Some(run) = self.compiled.as_mut() else {
            return;
        };
        // Park this slot's interact queue in the ctx for the tick: the verbs
        // the card dispatches then land on the same drain the isolate's
        // forwarded requests ride.
        #[cfg(feature = "load")]
        {
            ctx.compiled.interacts = Some(std::mem::take(&mut self.compiled_interacts));
        }
        let result = match trapped_failure(ctx) {
            Some(failure) => Ok(ScriptFlow::Blocked(failure)),
            None => {
                let mut retained = self
                    .retained
                    .as_ref()
                    .expect("compiled retention")
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                self.pair_evidence = Some(api::quest_progress::EvidenceStamp {
                    run: run.run,
                    tick: ctx.tick,
                    sequence: ctx.tick,
                });
                run.tick(
                    ctx,
                    &mut retained,
                    &mut self.native_runtime,
                    self.quest_pairs.as_deref(),
                )
            }
        };
        if result.is_ok() {
            self.watchdog.stamp_scheduler(Instant::now());
        }
        #[cfg(feature = "load")]
        {
            self.compiled_interacts = ctx.compiled.interacts.take().unwrap_or_default();
            self.compiled_interact_outcome_seqs.resize(
                self.compiled_interacts.len(),
                self.native_runtime.observed_walk_outcome_seq,
            );
        }
        match result {
            Ok(ScriptFlow::Continue | ScriptFlow::Complete)
                if self.compiled.as_ref().is_some_and(|run| {
                    run.output.status.as_ref().is_some_and(|status| {
                        status.phase == crate::native::NativePhase::Blocked
                            && status.failure.is_some()
                    })
                }) =>
            {
                let failure = self
                    .compiled
                    .as_ref()
                    .and_then(|run| run.output.status.as_ref())
                    .and_then(|status| status.failure.clone())
                    .expect("terminal blocked status");
                self.stop_blocked(failure, ctx.tick);
            }
            Ok(ScriptFlow::Continue) => {}
            Ok(ScriptFlow::Blocked(failure)) => self.stop_blocked(failure, ctx.tick),
            Ok(ScriptFlow::Complete) => {
                self.revoke_native_input();
                #[cfg(feature = "load")]
                {
                    self.compiled_interacts.clear();
                    self.compiled_interact_outcome_seqs.clear();
                }
                self.teardown_compiled(StopReason::Completed);
                self.watchdog.cancel_clear();
                self.state = RunState::Idle;
                self.want_run = false;
                self.lifecycle_receipt = Some(ScriptLifecycleReceipt {
                    runtime_generation: self.runtime_generation,
                    state: ScriptTerminalState::Completed,
                    tick: ctx.tick,
                    reason: "completed".into(),
                });
            }
            Err(failure) => self.fail_compiled(failure),
        }
    }

    /// Use the operator Stop cleanup, retaining only the terminal diagnostic.
    /// A Blocked flow is terminal even when the card omitted its status;
    /// a published Blocked phase is terminal only when it carries a failure.
    fn stop_blocked(&mut self, failure: ScriptFailure, tick: u64) {
        #[cfg(feature = "load")]
        if self.compiled.is_none() {
            let generation = self.runtime_generation;
            let reason = format!("{}: {}", failure.code, failure.message);
            self.stop_with_reason(StopReason::Error, "blocked");
            self.pending_logs.push(reason.clone());
            self.last_error = Some(reason.clone());
            self.lifecycle_receipt = Some(ScriptLifecycleReceipt {
                runtime_generation: generation,
                state: ScriptTerminalState::Failed,
                tick,
                reason,
            });
            return;
        }

        use crate::native::NativeOutput;
        let run = self.compiled.as_mut().expect("blocked compiled run");
        let status = crate::native::ScriptStatus {
            run: run.run,
            card: run.config.card(),
            phase: crate::native::NativePhase::Blocked,
            active_settings: run.config.revision(),
            pending_settings: run.pending.as_ref().map(|next| next.revision()),
            fields: run
                .output
                .status
                .as_ref()
                .map_or_else(|| Arc::from([]), |status| Arc::clone(&status.fields)),
            failure: Some(failure),
        };
        // Publish through the same change-only logging seam as card statuses.
        run.output.status(status);
        let status = run.output.status.clone().expect("blocked status");
        let generation = self.runtime_generation;
        self.stop();
        self.lifecycle_receipt = Some(ScriptLifecycleReceipt {
            runtime_generation: generation,
            state: ScriptTerminalState::Failed,
            tick,
            reason: status
                .failure
                .as_ref()
                .expect("blocked failure")
                .message
                .to_string(),
        });
        self.terminal_native_status = Some(status);
    }

    pub fn state(&self) -> RunState {
        self.state
    }

    /// The random-event knock. A running Load isolate's cached
    /// `ignoredRandoms()` names return `Handle` when case-insensitively
    /// listed; this suppresses guardian action/hold without hiding the
    /// detected event. Unlisted Load events return `Host`; compiled scripts
    /// use their `on_random` hook. Rising edge only (the guardian owns the
    /// per-event signature). Idle / Paused / not-want-run slots answer
    /// `Host` without touching the script.
    pub fn on_random(&mut self, ev: &DetectedRandom) -> RandomClaim {
        if self.state != RunState::Running || !self.want_run {
            return RandomClaim::Host;
        }
        #[cfg(feature = "load")]
        if self
            .load
            .as_ref()
            .is_some_and(|isolate| isolate.ignores_random(&ev.name))
        {
            return RandomClaim::Handle;
        }
        let Some(run) = &mut self.compiled else {
            return RandomClaim::Host;
        };
        match catch_unwind(AssertUnwindSafe(|| run.script.on_random(ev))) {
            Ok(claim) => claim,
            Err(payload) => {
                self.fail_compiled(ScriptFailure {
                    code: "random-panic".into(),
                    message: panic_message(&payload).into(),
                });
                RandomClaim::Host
            }
        }
    }

    #[cfg(all(feature = "memory-profile", feature = "load"))]
    pub fn memory_metrics(&self) -> Option<serde_json::Value> {
        self.load.as_ref().map(|i| i.memory_metrics())
    }

    #[cfg(all(feature = "memory-profile", feature = "load"))]
    pub fn memory_progress(&self) -> serde_json::Value {
        let mut value = self
            .load
            .as_ref()
            .map(|i| i.memory_progress())
            .unwrap_or(serde_json::json!({}));
        if let Some(fp) = &self.last_snapshot {
            let rows = |rs: &[crate::isolate_fb::ItemRowFp]| {
                rs.iter()
                    .take(32)
                    .map(|r| serde_json::json!({"name":r.name,"count":r.count,"ops":r.ops}))
                    .collect::<Vec<_>>()
            };
            value["inventory"] = serde_json::json!(rows(&fp.inv));
            value["bank"] = serde_json::json!(rows(&fp.bank));
            value["bank_open"] = serde_json::json!(fp.bank_open);
            value["bank_loaded"] = serde_json::json!(fp.bank_loaded);
        }
        value
    }

    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    /// Latest ScriptRunner.stop receipt. Reading it never drains panel logs.
    pub fn lifecycle_receipt(&self) -> Option<ScriptLifecycleReceipt> {
        self.lifecycle_receipt.clone()
    }
    /// Generation whose Failed, Stopped, or Completed lifecycle releases host navigation.
    /// Copies only the scalar; the receipt's diagnostic remains borrowed in place.
    pub fn terminal_lifecycle_generation(&self) -> Option<u64> {
        let receipt = self.lifecycle_receipt.as_ref()?;
        matches!(
            receipt.state,
            ScriptTerminalState::Failed
                | ScriptTerminalState::Stopped
                | ScriptTerminalState::Completed
        )
        .then_some(receipt.runtime_generation)
    }

    /// Drain isolate tick / `this.log` lines. Tick errors update
    /// [`Self::last_error`]. Copies are kept for [`Self::take_pending_logs`].
    #[cfg(feature = "load")]
    pub fn drain_logs(&mut self) -> Vec<String> {
        self.observe_lifecycle();
        let (logs, outcomes, stopped, script_stop) = match &self.load {
            Some(isolate) => {
                let logs = isolate.drain_logs();
                let outcomes = isolate.drain_tick_outcomes();
                (
                    logs,
                    outcomes,
                    isolate.stopped(),
                    isolate.script_stop_receipt(),
                )
            }
            None => (Vec::new(), Vec::new(), false, None),
        };
        for outcome in outcomes {
            match outcome {
                crate::load::TickOutcome::Error {
                    tick,
                    generation,
                    message,
                } => {
                    self.last_error = Some(format!("tick {tick}: {message}"));
                    self.active_tick_error_generation = Some(generation);
                }
                crate::load::TickOutcome::Success { generation, .. }
                    if self.active_tick_error_generation == Some(generation) =>
                {
                    self.last_error = None;
                    self.active_tick_error_generation = None;
                }
                crate::load::TickOutcome::Success { .. } => {}
            }
        }
        self.pending_logs.extend(logs.iter().cloned());
        if stopped {
            let runtime_generation = self.runtime_generation;
            self.stop();
            if let Some(receipt) = script_stop {
                self.last_error = Some(format!(
                    "script requested stop on tick {}: {}",
                    receipt.tick, receipt.reason
                ));
                self.lifecycle_receipt = Some(ScriptLifecycleReceipt {
                    runtime_generation,
                    state: ScriptTerminalState::Stopped,
                    tick: receipt.tick,
                    reason: receipt.reason,
                });
            }
            self.observe_lifecycle();
        }
        logs
    }

    /// Take log lines staged by [`Self::drain_logs`] (panel log pane).
    pub fn take_pending_logs(&mut self) -> Vec<String> {
        #[cfg(feature = "load")]
        self.observe_lifecycle();
        std::mem::take(&mut self.pending_logs)
    }

    /// Whether a JS Load isolate is installed (feature-gated; always false
    /// in a build without the `load` feature).
    #[cfg(feature = "load")]
    pub fn load_active(&self) -> bool {
        self.load.is_some()
    }
    #[cfg(not(feature = "load"))]
    pub fn load_active(&self) -> bool {
        false
    }

    /// Whether a Load-isolate delta fingerprint was stored (always false for
    /// compiled-only slots). Used by host-play tests for OPT-004.
    #[doc(hidden)]
    pub fn has_snapshot_fingerprint(&self) -> bool {
        #[cfg(feature = "load")]
        {
            self.last_snapshot.is_some()
        }
        #[cfg(not(feature = "load"))]
        {
            false
        }
    }
}

impl Drop for SlotScript {
    fn drop(&mut self) {
        self.preparing = None;
        self.teardown_compiled(StopReason::Removed);
        #[cfg(feature = "load")]
        self.teardown_api(StopReason::Removed);
        #[cfg(feature = "load")]
        if let Some(isolate) = self.load.take() {
            let (tx, _rx) = std::sync::mpsc::channel();
            isolate.join_detached(tx);
        }
    }
}

/// An unheld running script standing on a random event's trap square (the Maze
/// or the Mime stage). Only that event's own solution leads off the square,
/// and an unheld frame means the host guardian is not running one (it gave
/// up, was explicitly ignored, or random events are off). Native and Load
/// scripts take the same terminal Blocked Stop rather than running against
/// a world they cannot leave through ordinary work.
fn trapped_failure(ctx: &ScriptCtx<'_>) -> Option<ScriptFailure> {
    if ctx.compiled.hold {
        return None;
    }
    let (x, z, level) = ctx.here?;
    let event = match api::random::trapped_area(x, z, level)? {
        api::random::RandomKind::Maze => "Maze",
        _ => "Mime",
    };
    Some(ScriptFailure {
        code: "random-trapped".into(),
        message: format!(
            "trapped in the {event} random event at ({x}, {z}, {level}) and the host is not solving it"
        )
        .into(),
    })
}

/// Best-effort panic payload to string. Downcasts the usual `&str` and
/// `String` payloads; also unwraps a re-boxed `Box<dyn Any + Send>` payload,
/// the shape `catch_unwind` can re-arm in some unwind runtimes.
fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        return (*s).to_string();
    }
    if let Some(s) = payload.downcast_ref::<String>() {
        return s.clone();
    }
    if let Some(inner) = payload.downcast_ref::<Box<dyn std::any::Any + Send>>() {
        if let Some(s) = inner.downcast_ref::<&str>() {
            return (*s).to_string();
        }
        if let Some(s) = inner.downcast_ref::<String>() {
            return s.clone();
        }
    }
    "(no message)".to_string()
}

#[cfg(feature = "load")]
fn settings_fp(bag: &serde_json::Map<String, serde_json::Value>) -> String {
    serde_json::to_string(bag).unwrap_or_default()
}

#[cfg(all(test, feature = "load"))]
#[path = "slot_tests.rs"]
mod tests;
