//! Per-uid script runner. `SlotScript` owns at most one compiled `Script`
//! and gates it on operator intent (`want_run`) and client presence
//! (`on_is_up`). `tick` runs on the caller's pump at a game-tick edge and
//! must return; panics are caught, never abort the process.

mod pending;

pub use pending::{
    PendingBankOp, PendingBankOpKind, PendingFillBaseline, PendingWithdrawResult, PendingWithdrawX,
    PendingWithdrawXPhase,
};

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::ctx::{Script, ScriptCtx};
#[cfg(feature = "load")]
use crate::isolate_fb::{IsolateBuf, SnapshotFingerprint};
#[cfg(feature = "load")]
use crate::load::{LoadIsolate, LoadShape, Ready};
#[cfg(feature = "load")]
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
    Refused(String),
    RuntimeLoad(String),
}

#[cfg(feature = "load")]
impl std::fmt::Display for StartLoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Refused(message) | Self::RuntimeLoad(message) => f.write_str(message),
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
#[cfg(feature = "load")]
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
}

/// One atomic read of a pending operator Load Start ([`SlotScript::poll_start`]).
#[cfg(feature = "load")]
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
    compiled: Option<Box<dyn Script>>,
    /// The compiled card's own interact queue: what its tick enqueued, drained
    /// by the host on the same frames the isolate's queue is. Never both — a
    /// slot runs a compiled script XOR a Load isolate.
    #[cfg(feature = "load")]
    compiled_interacts: Vec<crate::shim::InteractReq>,
    /// The selected-revision facts the owning `Play` pinned at this compiled
    /// Start, read by the ctx the host builds around each tick. Cleared with
    /// the instance.
    compiled_selected: Option<Arc<api::game_data::SelectedGameData>>,
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
    #[cfg(feature = "load")]
    start_pending: bool,
    #[cfg(feature = "load")]
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
    #[cfg(feature = "load")]
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
    /// Script-session run policy shared with the host slot.
    run_policy_override: Arc<api::run_policy::RunPolicyOverrideCell>,
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
            #[cfg(feature = "load")]
            compiled_interacts: Vec::new(),
            compiled_selected: None,
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
            #[cfg(feature = "load")]
            start_pending: false,
            #[cfg(feature = "load")]
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
            #[cfg(feature = "load")]
            watchdog: ProgressWatchdog::new(),
            #[cfg(feature = "load")]
            cut_restart_times: [None; CUT_RESTART_LIMIT],
            #[cfg(feature = "load")]
            cut_restart_next: 0,
            source_identity: None,
            runtime_generation: 0,
            last_settings_fp: None,
            native_input: NativeInputAuthority::new(),
            run_policy_override: Arc::new(api::run_policy::RunPolicyOverrideCell::new()),
        }
    }

    /// The host slot reads the same cell as this script slot's V8 binding.
    pub fn run_policy_override_cell(&self) -> Arc<api::run_policy::RunPolicyOverrideCell> {
        Arc::clone(&self.run_policy_override)
    }

    pub fn run_policy_override(&self) -> Option<api::run_policy::RunPolicyOverride> {
        self.run_policy_override.get()
    }

    /// True when either a compiled script or a JS isolate is installed.
    fn has_instance(&self) -> bool {
        self.compiled.is_some() || self.load_active()
    }

    /// Install a compiled script and start it. Refuses (no silent replace)
    /// while Starting, Running, Paused, or Stopping; allowed from Idle and
    /// Error (a fresh Start clears the previous error).
    ///
    /// `selected` is the owning `Play`'s selected-revision pin — the same
    /// facts a Load isolate is spawned with. The host hands it to the card on
    /// every tick (`ScriptCtx::compiled`); a `None` pin is what a compiled
    /// identify fails closed on.
    pub fn start_compiled(
        &mut self,
        script: Box<dyn Script>,
        selected: Option<Arc<api::game_data::SelectedGameData>>,
    ) -> Result<(), String> {
        match self.state {
            RunState::Running | RunState::Paused | RunState::Stopping | RunState::Starting => {
                Err("script already active: stop it first".to_string())
            }
            RunState::Idle | RunState::Error => {
                if self.load_active() {
                    return Err("loaded script active: stop it first".to_string());
                }
                self.run_policy_override.clear();
                self.compiled = Some(script);
                self.compiled_selected = selected;
                #[cfg(feature = "load")]
                self.compiled_interacts.clear();
                self.want_run = true;
                self.last_error = None;
                #[cfg(feature = "load")]
                {
                    self.active_tick_error_generation = None;
                }
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
                self.state = RunState::Running;
                self.runtime_generation = self.runtime_generation.wrapping_add(1);
                self.last_settings_fp = None;
                self.native_input.publish_live();
                Ok(())
            }
        }
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
                self.run_policy_override.clear();
                self.load_identity = Some(SlotLoadIdentity {
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
                self.run_policy_override.clear();
                let identity = SlotLoadIdentity {
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

    #[cfg(feature = "load")]
    fn settle_start(&mut self, outcome: StartOutcome) {
        if std::mem::take(&mut self.start_pending) {
            self.start_outcome = Some(outcome);
        }
    }

    /// Resolve the lifecycle and read the latest operator Load Start in one
    /// step, so no observe can settle it between the outcome read and the
    /// in-flight check (the slot thread observes every frame).
    #[cfg(feature = "load")]
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
        let isolate = LoadIsolate::spawn_with_content(
            identity.source.to_string(),
            identity.shape,
            identity.siblings.iter().cloned().collect(),
            identity.game_data.clone(),
            Arc::clone(&identity.named_banks),
            Arc::clone(&self.run_policy_override),
        )?;
        isolate.post_loadouts(&identity.loadouts);
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

    #[cfg(feature = "load")]
    fn begin_async_stop(&mut self, after: AfterStop) -> bool {
        let Some(isolate) = self.load.take() else {
            return false;
        };
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
        if let Some(mut script) = self.compiled.take() {
            script.on_stop();
        }
        self.compiled_selected = None;
        #[cfg(feature = "load")]
        self.compiled_interacts.clear();
        #[cfg(feature = "load")]
        {
            self.last_snapshot = None;
            self.last_world_id = None;
            self.ipc = IsolateBuf::new();
            self.load_ready = false;
            self.stop_rx = None;
            self.after_stop = AfterStop::Idle;
            self.setup_generation_base = None;
            self.load_identity = None;
            self.watchdog.cancel_clear();
        }
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
    pub fn observe_lifecycle(&mut self) {
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
        // Frozen ScriptRunner.ts:389-397 runs onStop before clearing the
        // RunManager overlay. The reaper has finished the hook at this point.
        self.run_policy_override.clear();
        self.stop_rx = None;
        self.last_snapshot = None;
        self.last_world_id = None;
        self.ipc = IsolateBuf::new();
        self.pending_withdraw_x = None;
        self.pending_bank_op = None;
        self.work_epoch = self.work_epoch.wrapping_add(1);
        match std::mem::replace(&mut self.after_stop, AfterStop::Idle) {
            AfterStop::Idle => {
                if let Some(mut script) = self.compiled.take() {
                    script.on_stop();
                }
                self.compiled_selected = None;
                self.compiled_interacts.clear();
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
    /// Resume. Instance kept. Returns whether a live recovery walk must be
    /// aborted on the host nav bot.
    pub fn pause(&mut self) -> bool {
        self.want_run = false;
        self.revoke_native_input();
        if let Some(pending) = &mut self.pending_withdraw_x {
            pending.freeze();
        }
        if let Some(pending) = &mut self.pending_bank_op {
            pending.freeze();
        }
        let mut abort_recovery = false;
        if self.has_instance() && matches!(self.state, RunState::Running | RunState::Starting) {
            #[cfg(feature = "load")]
            if let Some(isolate) = &self.load {
                isolate.pause();
            }
            #[cfg(feature = "load")]
            {
                abort_recovery = matches!(
                    self.watchdog.abort_owned_recovery(),
                    WatchdogAction::AbortWalk
                );
                let _ = self.watchdog.set_frozen(true, Instant::now());
            }
            self.state = RunState::Paused;
        }
        abort_recovery
    }

    /// Operator Resume: `want_run` back on. Assumes the client is up; the
    /// next `on_is_up(false)` re-gates if it is not. No-op when there is
    /// no instance or the slot errored.
    pub fn resume(&mut self) {
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
            #[cfg(feature = "load")]
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
        self.run_policy_override.clear();
        self.lifecycle_receipt = None;
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
        #[cfg(feature = "load")]
        {
            self.active_tick_error_generation = None;
        }
        // A Start that has not reached Ready never ran: nothing to commit.
        #[cfg(feature = "load")]
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
    /// deltas at a connection boundary. Operator run intent is retained.
    pub fn reset_session_work(&mut self) {
        self.on_is_up(false);
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
            if let Some(isolate) = &self.load {
                let generation = isolate.reset_session_work();
                self.active_tick_error_generation = carried_error.then_some(generation);
            } else {
                self.active_tick_error_generation = None;
            }
            self.last_snapshot = None;
            self.last_world_id = None;
            self.reach_cache.clear();

            let abort = self.watchdog.abort_owned_recovery();
            let reset = self.watchdog.on_session_reset(Instant::now());
            let _ = (abort, reset);
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

    /// Post the merged operator settings bag into a Load isolate.
    #[cfg(feature = "load")]
    pub fn post_settings_bag(&mut self, bag: &serde_json::Map<String, serde_json::Value>) {
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

    /// The selected-revision facts this compiled Start pinned — what the host
    /// copies into the ctx as `ScriptCtx::compiled.selected`. `None` for a
    /// Load slot, an idle slot, or a `Play` with no generated facts.
    pub fn compiled_game_data(&self) -> Option<Arc<api::game_data::SelectedGameData>> {
        self.compiled_selected.clone()
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

    pub fn sync_native_input_gate(&self) {
        let watchdog_hold = {
            #[cfg(feature = "load")]
            {
                self.watchdog.holds_script_actions()
            }
            #[cfg(not(feature = "load"))]
            {
                false
            }
        };
        let live = self.state == RunState::Running
            && self.want_run
            && self.has_instance()
            && !watchdog_hold;
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
        let (bytes, fp) = self.ipc.encode_snapshot_delta_with_native(
            self.last_snapshot.as_ref(),
            input,
            native,
            force_banks,
        );
        self.last_snapshot = Some(fp);
        bytes
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

    /// Drain the interact requests this slot's script queued, in tick order:
    /// the Load isolate's forwarded queue (the shim Bank/Banking queue), or
    /// the compiled card's own queue. The two are exclusive by construction —
    /// a slot runs a compiled script XOR a Load isolate — and the debug
    /// assertion is what keeps a future third path from silently merging
    /// them. Empty for a slot with neither.
    #[cfg(feature = "load")]
    pub fn drain_interacts(&mut self) -> Vec<crate::shim::InteractReq> {
        debug_assert!(
            !(self.compiled.is_some() && self.load.is_some()),
            "a slot never owns both a compiled script and a Load isolate"
        );
        match &self.load {
            Some(isolate) => isolate.drain_interacts(),
            None => std::mem::take(&mut self.compiled_interacts),
        }
    }

    /// Restore a batch drained by the host when Pause wins the final
    /// dispatch fence. The drained rows precede anything queued since the
    /// drain, preserving the script's original request order.
    #[cfg(feature = "load")]
    pub fn restore_interacts(&mut self, mut drained: Vec<crate::shim::InteractReq>) {
        debug_assert!(
            !(self.compiled.is_some() && self.load.is_some()),
            "a slot never owns both a compiled script and a Load isolate"
        );
        match &self.load {
            Some(isolate) => isolate.restore_interacts(drained),
            None => {
                drained.append(&mut self.compiled_interacts);
                self.compiled_interacts = drained;
            }
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

    #[cfg(feature = "load")]
    pub fn watchdog(&self) -> &ProgressWatchdog {
        &self.watchdog
    }

    /// Apply isolate lifecycle facts and host tile/XP, then decide.
    #[cfg(feature = "load")]
    pub fn feed_watchdog(
        &mut self,
        now: Instant,
        here: Option<(i32, i32, i32)>,
        xp: &[i32],
        frozen: bool,
        running: bool,
        lifecycle: &[crate::shim::InteractReq],
    ) -> WatchdogAction {
        if self.load.is_none() {
            return WatchdogAction::None;
        }
        let freeze_action = self.watchdog.set_frozen(frozen, now);
        if matches!(freeze_action, WatchdogAction::AbortWalk) {
            return freeze_action;
        }
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
            let player = here.map(|(x, z, _)| (x, z));
            return self.watchdog.on_anchor(now, player, reply);
        }
        if let Some((x, z, level)) = here {
            let arrived = self.watchdog.on_tile(now, WatchdogTile { x, z, level });
            if arrived != WatchdogAction::None {
                return arrived;
            }
        }
        self.watchdog.on_xp(now, xp);
        let action = self.watchdog.observe(now, running && self.want_run);
        if matches!(action, WatchdogAction::WarnHungLoop) {
            self.pending_logs
                .push("watchdog: hung loop (10s, no scheduler progress)".into());
        }
        action
    }

    /// Recreate the Load isolate from retained identity. Consumes cooldown.
    #[cfg(feature = "load")]
    pub fn restart_load_from_identity(&mut self, now: Instant) -> Result<(), String> {
        if !self.want_run || self.state == RunState::Paused {
            return Err("watchdog restart cancelled: operator is not running".into());
        }
        if self.watchdog.frozen() {
            return Err("watchdog restart cancelled: frozen".into());
        }
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

    /// Common retained-identity recreate used by the stall watchdog and by a
    /// confirmed JavaScript cut. Policy checks belong to the callers: a
    /// Pause-deadline cut deliberately recreates while operator-paused.
    #[cfg(feature = "load")]
    fn apply_load_restart(&mut self, now: Instant) -> Result<(), String> {
        let Some(identity) = self.load_identity.clone() else {
            return Err("watchdog restart: no retained identity".into());
        };
        self.run_policy_override.clear();
        self.revoke_native_input();
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

    #[cfg(feature = "load")]
    pub fn notify_walk_failed(&mut self, now: Instant) -> WatchdogAction {
        self.watchdog.on_walk_failed(now)
    }

    #[cfg(feature = "load")]
    pub fn notify_hold_during_walk(&mut self) -> WatchdogAction {
        self.watchdog.on_hold_during_walk()
    }

    #[cfg(feature = "load")]
    pub fn abort_owned_recovery(&mut self) -> WatchdogAction {
        self.watchdog.abort_owned_recovery()
    }

    /// The slot script's latest recorded paint frame (a Load isolate's
    /// host handle forwards it after every tick that painted); `None` for
    /// a compiled script or a slot that has not painted. The host shares
    /// the frame with the status row so the TUI/panel views can show it in
    /// the chat pane in place of the game chat.
    #[cfg(feature = "load")]
    pub fn paint(&self) -> Option<std::sync::Arc<crate::shim::ScriptPaint>> {
        self.load.as_ref().and_then(|iso| iso.paint())
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
        #[cfg(feature = "load")]
        self.observe_lifecycle();
        if self.state != RunState::Running || !self.want_run {
            return;
        }
        #[cfg(feature = "load")]
        if let Some(isolate) = &self.load {
            isolate.on_game_tick_at(ctx.tick, self.native_input.lock().identity());
            return;
        }
        let Some(script) = self.compiled.as_mut() else {
            return;
        };
        // Park this slot's interact queue in the ctx for the tick: the verbs
        // the card dispatches then land on the same drain the isolate's
        // forwarded requests ride.
        #[cfg(feature = "load")]
        {
            ctx.compiled.interacts = Some(std::mem::take(&mut self.compiled_interacts));
        }
        self.ticks += 1;
        let result = catch_unwind(AssertUnwindSafe(|| script.tick(ctx)));
        #[cfg(feature = "load")]
        {
            self.compiled_interacts = ctx.compiled.interacts.take().unwrap_or_default();
        }
        if let Err(payload) = result {
            let message = format!("script panic: {}", panic_message(&payload));
            self.pending_logs.push(message.clone());
            self.last_error = Some(message);
            self.state = RunState::Error;
            self.want_run = false;
            self.compiled = None;
            self.compiled_selected = None;
            // The card is gone: its machine session must not outlive it, and
            // the verbs a half-finished tick queued are not this session's to
            // send. The pump applies the abort on this same thread, next
            // observed frame. It is the session abort and not
            // `clue_stop_owed`: a dead card is not a fresh task instance, and
            // the strip list it may still owe a reclaim for outlives it —
            // Stop is what clears that.
            #[cfg(feature = "load")]
            {
                self.compiled_interacts.clear();
                self.clue_abort_owed = true;
            }
            self.revoke_native_input();
        }
    }

    pub fn state(&self) -> RunState {
        self.state
    }

    /// The random-event knock: ask the running compiled script whether it
    /// handles the detected event. Rising edge only (the guardian owns the
    /// per-event signature); JS isolates always answer `Host` — no isolate
    /// hook this tag. Idle / Paused / not-want-run slots answer `Host`
    /// without touching the script.
    pub fn on_random(&mut self, ev: &DetectedRandom) -> RandomClaim {
        if self.state != RunState::Running || !self.want_run {
            return RandomClaim::Host;
        }
        match &mut self.compiled {
            Some(script) => script.on_random(ev),
            // JS Load isolate: the knock has no JS arm this tag.
            None => RandomClaim::Host,
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
        self.run_policy_override.clear();
        #[cfg(feature = "load")]
        if let Some(isolate) = self.load.take() {
            let (tx, _rx) = std::sync::mpsc::channel();
            isolate.join_detached(tx);
        }
    }
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

fn settings_fp(bag: &serde_json::Map<String, serde_json::Value>) -> String {
    serde_json::to_string(bag).unwrap_or_default()
}

#[cfg(test)]
#[path = "slot_tests.rs"]
mod tests;
