//! Per-uid script runner. `SlotScript` owns at most one compiled `Script`
//! and gates it on operator intent (`want_run`) and client presence
//! (`on_is_up`). `tick` runs on the caller's pump at a game-tick edge and
//! must return; panics are caught, never abort the process.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::ctx::{Script, ScriptCtx};
#[cfg(feature = "load")]
use crate::isolate_fb::{IsolateBuf, SnapshotFingerprint};
#[cfg(feature = "load")]
use crate::load::{LoadIsolate, LoadShape};
#[cfg(feature = "load")]
use crate::watchdog::{ProgressWatchdog, Tile as WatchdogTile, WatchdogAction};
use api::native_input::NativeInputAuthority;
use api::random::{DetectedRandom, RandomClaim};
use serde::Serialize;

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

/// Lifecycle of the script slot. `paused` covers both operator Pause and
/// the not-`is_up` gate; `stopping` is the Load-join window (later task).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunState {
    Idle,
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

/// Host-owned second phase of a bank Withdraw-X operation. The first phase
/// sent the X menu action; this record authorizes one count response only
/// while the same bank session remains current and before its deadline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingWithdrawXPhase {
    Dialog,
    Settlement,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingWithdrawResult {
    WithdrawX,
    WithdrawLoad,
}

#[derive(Debug, Clone, Copy)]
pub struct PendingFillBaseline {
    pub bank_item_id: i32,
    pub before_used: usize,
    pub before_count: i32,
    pub before_stock: i32,
}

#[derive(Debug, Clone, Copy)]
pub struct PendingWithdrawX {
    pub item_id: i32,
    pub count: i32,
    pub before: i32,
    pub target: i32,
    pub bank_generation: u64,
    pub phase: PendingWithdrawXPhase,
    pub deadline: Option<Instant>,
    pub remaining: Duration,
    pub result: PendingWithdrawResult,
    pub fill: Option<PendingFillBaseline>,
}

impl PendingWithdrawX {
    pub fn waiting_dialog(
        item_id: i32,
        count: i32,
        before: i32,
        target: i32,
        bank_generation: u64,
    ) -> Self {
        let remaining = Duration::from_millis(3000);
        Self {
            item_id,
            count,
            before,
            target,
            bank_generation,
            phase: PendingWithdrawXPhase::Dialog,
            deadline: Some(Instant::now() + remaining),
            remaining,
            result: PendingWithdrawResult::WithdrawX,
            fill: None,
        }
    }

    pub fn waiting_load_dialog(
        bank_item_id: i32,
        count: i32,
        before_used: usize,
        before_count: i32,
        before_stock: i32,
        bank_generation: u64,
    ) -> Self {
        let mut pending = Self::waiting_dialog(bank_item_id, count, 0, 0, bank_generation);
        pending.result = PendingWithdrawResult::WithdrawLoad;
        pending.fill = Some(PendingFillBaseline {
            bank_item_id,
            before_used,
            before_count,
            before_stock,
        });
        pending
    }

    pub fn waiting_settlement(mut self) -> Self {
        self.phase = PendingWithdrawXPhase::Settlement;
        self.remaining = Duration::from_millis(4000);
        self.deadline = Some(Instant::now() + self.remaining);
        self
    }

    pub fn expired(self) -> bool {
        self.deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
    }

    fn freeze(&mut self) {
        if let Some(deadline) = self.deadline.take() {
            self.remaining = deadline.saturating_duration_since(Instant::now());
        }
    }

    fn resume(&mut self) {
        if self.deadline.is_none() {
            self.deadline = Some(Instant::now() + self.remaining);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingBankOpKind {
    Deposit,
    Withdraw,
    /// Raw open-only `Withdraw X` (`Bank.withdraw(name, 'Withdraw X')`).
    /// The frozen caller reads the accepted click as the result
    /// (`Input.invButton` → `actions.menuAction`) and types the amount +
    /// Enter itself, so this kind never settles on an inventory delta:
    /// the sent action plus the still-current bank session acknowledge it.
    WithdrawXAction,
}

#[derive(Debug, Clone, Copy)]
pub struct PendingBankOp {
    pub kind: PendingBankOpKind,
    pub item_id: i32,
    pub before_count: i32,
    pub before_inventory_count: i32,
    pub bank_generation: u64,
    deadline: Option<Instant>,
    remaining: Duration,
}

impl PendingBankOp {
    pub fn new(
        kind: PendingBankOpKind,
        item_id: i32,
        before_count: i32,
        before_inventory_count: i32,
        bank_generation: u64,
    ) -> Self {
        let remaining = Duration::from_millis(match kind {
            PendingBankOpKind::Deposit => 2000,
            PendingBankOpKind::Withdraw => 4000,
            // The acknowledgment lands on the next observe pass; this bound
            // only backstops a stalled session, so it keeps the ordinary
            // withdrawal bound rather than inventing a new one.
            PendingBankOpKind::WithdrawXAction => 4000,
        });
        Self {
            kind,
            item_id,
            before_count,
            before_inventory_count,
            bank_generation,
            deadline: Some(Instant::now() + remaining),
            remaining,
        }
    }

    pub fn expired(self) -> bool {
        self.deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
    }

    fn freeze(&mut self) {
        if let Some(deadline) = self.deadline.take() {
            self.remaining = deadline.saturating_duration_since(Instant::now());
        }
    }

    fn resume(&mut self) {
        if self.deadline.is_none() {
            self.deadline = Some(Instant::now() + self.remaining);
        }
    }
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
    /// Per-slot last-post snapshot fingerprint (delta posts: only the
    /// fields that changed are re-sent) and the `NavWorld` identity the
    /// packed banks keyframe on. Cleared on Start so the first post is a
    /// keyframe.
    #[cfg(feature = "load")]
    last_snapshot: Option<SnapshotFingerprint>,
    #[cfg(feature = "load")]
    last_world_id: Option<usize>,
    /// Reusable FlatBuffer builder for this slot's host→isolate snapshot
    /// posts. One per slot; the V8 isolate thread holds its own for
    /// interact/paint. Never a JSON document on either path.
    #[cfg(feature = "load")]
    ipc: IsolateBuf,
    last_error: Option<String>,
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
    /// Stable source identity key for this execution (`catalog:Name` / file path).
    source_identity: Option<String>,
    /// Bumped on each successful Start and watchdog isolate replacement.
    runtime_generation: u64,
    last_settings_fp: Option<String>,
    native_input: Arc<NativeInputAuthority>,
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
            last_snapshot: None,
            #[cfg(feature = "load")]
            last_world_id: None,
            #[cfg(feature = "load")]
            ipc: IsolateBuf::new(),
            last_error: None,
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
            source_identity: None,
            runtime_generation: 0,
            last_settings_fp: None,
            native_input: NativeInputAuthority::new(),
        }
    }

    /// True when either a compiled script or a JS isolate is installed.
    fn has_instance(&self) -> bool {
        self.compiled.is_some() || self.load_active()
    }

    /// Install a compiled script and start it. Refuses (no silent replace)
    /// while Running, Paused, or Stopping; allowed from Idle and Error
    /// (a fresh Start clears the previous error).
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
            RunState::Running | RunState::Paused | RunState::Stopping => {
                Err("script already active: stop it first".to_string())
            }
            RunState::Idle | RunState::Error => {
                if self.load_active() {
                    return Err("loaded script active: stop it first".to_string());
                }
                self.compiled = Some(script);
                self.compiled_selected = selected;
                #[cfg(feature = "load")]
                self.compiled_interacts.clear();
                self.want_run = true;
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
            RunState::Running | RunState::Paused | RunState::Stopping => Err(
                StartLoadError::Refused("script already active: stop it first".to_string()),
            ),
            RunState::Idle | RunState::Error => {
                if self.compiled.is_some() {
                    return Err(StartLoadError::Refused(
                        "compiled script active: stop it first".to_string(),
                    ));
                }
                let source: Arc<str> = Arc::from(source);
                let siblings: Arc<[(String, String)]> = siblings.into();
                let loadouts_arc: Arc<[crate::loadouts_store::Loadout]> =
                    Arc::from(loadouts.to_vec());
                let isolate = LoadIsolate::spawn_with_content(
                    source.to_string(),
                    shape,
                    siblings.iter().cloned().collect(),
                    game_data.clone(),
                    Arc::clone(&named_banks),
                )
                .map_err(StartLoadError::RuntimeLoad)?;
                isolate.post_loadouts(loadouts);
                self.load = Some(isolate);
                self.load_identity = Some(SlotLoadIdentity {
                    source,
                    shape,
                    siblings,
                    settings_bag: None,
                    game_data,
                    named_banks,
                    loadouts: loadouts_arc,
                });
                self.watchdog.arm_fresh(Instant::now());
                self.want_run = true;
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
                // Fresh isolate: the first posted snapshot is a keyframe.
                self.last_snapshot = None;
                self.last_world_id = None;
                self.state = RunState::Running;
                self.runtime_generation = self.runtime_generation.wrapping_add(1);
                self.last_settings_fp = None;
                self.native_input.publish_live();
                Ok(())
            }
        }
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
        if self.has_instance() && self.state == RunState::Running {
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
            self.state = RunState::Running;
            self.native_input.resume();
        }
    }

    /// Operator Stop: join the Load isolate, run the compiled teardown
    /// hook, drop the instance, Idle.
    pub fn stop(&mut self) {
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
        if !self.load_active() {
            self.clue_stop_owed = true;
            self.clue_abort_owed = false;
        }
        #[cfg(feature = "load")]
        if let Some(isolate) = self.load.take() {
            let logs = isolate.join();
            self.pending_logs.extend(logs);
        }
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
        }
        self.want_run = false;
        self.pending_withdraw_x = None;
        self.pending_bank_op = None;
        self.work_epoch = self.work_epoch.wrapping_add(1);
        self.state = RunState::Idle;
        #[cfg(feature = "load")]
        {
            self.load_identity = None;
            self.watchdog.cancel_clear();
        }
        self.source_identity = None;
        self.runtime_generation = self.runtime_generation.wrapping_add(1);
        self.last_settings_fp = None;
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
            if let Some(isolate) = &self.load {
                isolate.reset_session_work();
            }
            self.last_snapshot = None;
            self.last_world_id = None;
            let abort = self.watchdog.abort_owned_recovery();
            let reset = self.watchdog.on_session_reset(Instant::now());
            let _ = (abort, reset);
        }
    }

    /// Current host-owned Withdraw-X continuation, if one is armed.
    pub fn pending_withdraw_x(&self) -> Option<PendingWithdrawX> {
        self.pending_withdraw_x
    }

    /// Replace the one bounded Withdraw-X continuation for this slot.
    pub fn set_pending_withdraw_x(&mut self, pending: Option<PendingWithdrawX>) {
        self.pending_withdraw_x = pending;
    }

    /// Freeze the monotonic deadline without discarding the operation.
    pub fn freeze_pending_withdraw_x(&mut self) {
        if let Some(pending) = &mut self.pending_withdraw_x {
            pending.freeze();
        }
    }

    /// Resume a previously frozen monotonic deadline.
    pub fn resume_pending_withdraw_x(&mut self) {
        if let Some(pending) = &mut self.pending_withdraw_x {
            pending.resume();
        }
    }

    /// Last host-owned Withdraw-X result posted to this isolate.
    pub fn withdraw_x_result(&self) -> (u64, bool) {
        (self.withdraw_x_result_seq, self.withdraw_x_result)
    }

    /// Last host-owned withdrawLoad result posted to this isolate.
    pub fn withdraw_load_result(&self) -> (u64, bool) {
        (self.withdraw_load_result_seq, self.withdraw_load_result)
    }

    pub fn pending_bank_op(&self) -> Option<PendingBankOp> {
        self.pending_bank_op
    }

    pub fn set_pending_bank_op(&mut self, pending: Option<PendingBankOp>) {
        self.pending_bank_op = pending;
    }

    pub fn freeze_pending_bank_op(&mut self) {
        if let Some(pending) = &mut self.pending_bank_op {
            pending.freeze();
        }
    }

    pub fn resume_pending_bank_op(&mut self) {
        if let Some(pending) = &mut self.pending_bank_op {
            pending.resume();
        }
    }

    pub fn bank_op_result(&self) -> (u64, bool) {
        (self.bank_op_result_seq, self.bank_op_result)
    }

    pub fn complete_bank_op(&mut self, result: bool) {
        self.pending_bank_op = None;
        self.bank_op_result_seq = self.bank_op_result_seq.wrapping_add(1);
        self.bank_op_result = result;
    }

    /// Lifecycle stamp used to reject work that raced a stop/reconnect.
    pub fn work_epoch(&self) -> u64 {
        self.work_epoch
    }

    /// Complete the current operation and advance the posted result token.
    pub fn complete_withdraw_x(&mut self, result: bool) {
        self.pending_withdraw_x = None;
        self.withdraw_x_result_seq = self.withdraw_x_result_seq.wrapping_add(1);
        self.withdraw_x_result = result;
    }

    pub fn complete_withdraw_load(&mut self, result: bool) {
        self.pending_withdraw_x = None;
        self.withdraw_load_result_seq = self.withdraw_load_result_seq.wrapping_add(1);
        self.withdraw_load_result = result;
    }

    /// Complete the armed shared withdrawal continuation on its result channel.
    pub fn complete_current_withdrawal(&mut self, result: bool) {
        match self.pending_withdraw_x.map(|pending| pending.result) {
            Some(PendingWithdrawResult::WithdrawLoad) => self.complete_withdraw_load(result),
            Some(PendingWithdrawResult::WithdrawX) | None => self.complete_withdraw_x(result),
        }
    }

    /// Post the host's FlatBuffer snapshot blob into a Load isolate (no-op
    /// for a compiled script). Call it before [`SlotScript::on_game_tick`]
    /// so the posted blob is what the tick's JS reads.
    #[cfg(feature = "load")]
    pub fn post_snapshot(&self, bytes: Vec<u8>) {
        if let Some(isolate) = &self.load {
            isolate.post_snapshot(bytes);
        }
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
        match self.state {
            RunState::Running | RunState::Paused => {}
            _ => return false,
        }
        let fp = settings_fp(bag);
        if self.last_settings_fp.as_deref() == Some(fp.as_str()) {
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
        self.revoke_native_input();
        let identity = self
            .load_identity
            .clone()
            .ok_or_else(|| "watchdog restart: no retained identity".to_string())?;
        if let Some(isolate) = self.load.take() {
            let logs = isolate.join();
            self.pending_logs.extend(logs);
        }
        self.last_snapshot = None;
        self.last_world_id = None;
        self.ipc = IsolateBuf::new();
        self.pending_withdraw_x = None;
        self.pending_bank_op = None;
        self.work_epoch = self.work_epoch.wrapping_add(1);
        self.state = RunState::Idle;
        let isolate = LoadIsolate::spawn_with_content(
            identity.source.to_string(),
            identity.shape,
            identity.siblings.iter().cloned().collect(),
            identity.game_data.clone(),
            Arc::clone(&identity.named_banks),
        )?;
        isolate.post_loadouts(&identity.loadouts);
        if let Some(bag) = identity.settings_bag.as_deref() {
            isolate.post_settings_bag(bag);
        }
        self.load = Some(isolate);
        self.last_error = None;
        self.lifecycle_receipt = None;
        self.ticks = 0;
        self.state = RunState::Running;
        self.runtime_generation = self.runtime_generation.wrapping_add(1);
        self.last_settings_fp = None;
        self.watchdog.on_restart_applied(now);
        self.native_input.publish_live();
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
    /// a compiled script or a slot that has not painted. The host copies
    /// it onto the status row so the TUI/panel views can show it in the
    /// chat pane in place of the game chat.
    #[cfg(feature = "load")]
    pub fn paint(&self) -> Option<crate::shim::ScriptPaint> {
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
            self.last_error = Some(format!("script panic: {}", panic_message(&payload)));
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
        let (logs, stopped, script_stop) = match &self.load {
            Some(isolate) => {
                let logs = isolate.drain_logs();
                (logs, isolate.stopped(), isolate.script_stop_receipt())
            }
            None => (Vec::new(), false, None),
        };
        if let Some(err) = logs
            .iter()
            .rev()
            .find(|l| l.starts_with("tick ") || l.contains("script requested stop"))
        {
            self.last_error = Some(err.clone());
        }
        self.pending_logs.extend(logs.iter().cloned());
        if stopped {
            let runtime_generation = self.runtime_generation;
            self.stop();
            self.lifecycle_receipt = script_stop.map(|receipt| ScriptLifecycleReceipt {
                runtime_generation,
                state: ScriptTerminalState::Stopped,
                tick: receipt.tick,
                reason: receipt.reason,
            });
        }
        logs
    }

    /// Take log lines staged by [`Self::drain_logs`] (panel log pane).
    pub fn take_pending_logs(&mut self) -> Vec<String> {
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
mod tests {
    use super::*;
    use crate::ctx::test_support::NullDriver;

    struct Noop;

    impl Script for Noop {
        fn name(&self) -> &str {
            "noop"
        }
        fn tick(&mut self, _ctx: &mut ScriptCtx<'_>) {}
    }

    #[cfg(feature = "load")]
    #[test]
    fn script_requested_stop_cleans_slot_work_and_allows_fresh_restart() {
        let source = r#"
import { ScriptRunner } from '../../runtime/ScriptRunner.js';
export default class T extends LoopingBot {
    loop() {
        (globalThis.__rs2b0t_host.interact ||= []).push({op: 'set-camera-yaw', yaw: 123});
        ScriptRunner.stop('finished');
    }
}
"#;
        let mut slot = SlotScript::new();
        slot.start_load_with_loadouts(source.into(), LoadShape::CompatClass, vec![], &[])
            .unwrap();
        let input = crate::isolate_fb::tests::empty_input(1);
        slot.encode_snapshot_delta(&input, false);
        slot.store_last_world_id(Some(123));
        slot.set_pending_withdraw_x(Some(PendingWithdrawX::waiting_dialog(2, 7, 0, 7, 3)));
        let epoch = slot.work_epoch();
        slot.load.as_ref().unwrap().on_game_tick(1);
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut logs = Vec::new();
        while slot.state() == RunState::Running && Instant::now() < deadline {
            logs.extend(slot.drain_logs());
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(slot.state(), RunState::Idle, "{logs:?}");
        assert!(logs
            .iter()
            .any(|line| line.contains("script requested stop")));
        assert!(!slot.has_instance());
        assert!(slot.pending_withdraw_x().is_none());
        assert_ne!(slot.work_epoch(), epoch);
        assert!(!slot.has_snapshot_fingerprint());
        assert_eq!(slot.last_world_id(), None);
        assert!(slot.drain_interacts().is_empty());
        assert!(slot.last_error().unwrap().contains("script requested stop"));
        assert_eq!(
            slot.lifecycle_receipt(),
            Some(ScriptLifecycleReceipt {
                runtime_generation: 1,
                state: ScriptTerminalState::Stopped,
                tick: 1,
                reason: "finished".into(),
            })
        );
        assert_eq!(slot.take_pending_logs(), logs);
        slot.on_is_up(true);
        assert_eq!(
            slot.state(),
            RunState::Idle,
            "login cannot restart a stopped card"
        );
        slot.start_load_with_loadouts(
            "export default class T extends LoopingBot { loop() { this.n = (this.n || 0) + 1; } }"
                .into(),
            LoadShape::CompatClass,
            vec![],
            &[],
        )
        .unwrap();
        slot.load.as_ref().unwrap().on_game_tick(2);
        assert_eq!(slot.probe("__rs_bot.n").unwrap(), 1);
        assert_eq!(slot.state(), RunState::Running);
        assert!(slot.last_error().is_none());
        assert_eq!(
            slot.lifecycle_receipt(),
            None,
            "fresh Start clears the receipt"
        );
        slot.stop();
        assert_eq!(
            slot.lifecycle_receipt(),
            None,
            "operator Stop is not a script-requested Stopped receipt"
        );
    }

    #[cfg(feature = "load")]
    #[test]
    fn script_stop_receipt_bounds_utf8_reason() {
        let reason = "🙂".repeat(100);
        let source = format!(
            r#"
import {{ ScriptRunner }} from '../../runtime/ScriptRunner.js';
export default class T extends LoopingBot {{
  loop() {{ ScriptRunner.stop({reason:?}); }}
}}
"#
        );
        let mut slot = SlotScript::new();
        slot.start_load_with_loadouts(source.into(), LoadShape::CompatClass, vec![], &[])
            .unwrap();
        let input = crate::isolate_fb::tests::empty_input(1);
        slot.encode_snapshot_delta(&input, false);
        slot.load.as_ref().unwrap().on_game_tick(1);
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut logs = Vec::new();
        while slot.state() == RunState::Running && Instant::now() < deadline {
            logs.extend(slot.drain_logs());
            std::thread::sleep(Duration::from_millis(5));
        }
        let receipt = slot.lifecycle_receipt().unwrap_or_else(|| {
            panic!(
                "script Stop receipt; state={:?} logs={logs:?}",
                slot.state()
            )
        });
        assert_eq!(receipt.state, ScriptTerminalState::Stopped);
        assert!(receipt.reason.len() <= 256);
        assert!(receipt.reason.chars().all(|ch| ch == '🙂'));
    }

    #[cfg(feature = "load")]
    #[test]
    fn stop_releases_snapshot_storage_and_restart_emits_keyframe() {
        let mut slot = SlotScript::new();
        slot.start_compiled(Box::new(Noop), None).unwrap();
        let text = "x".repeat(1024 * 1024);
        let mut input = crate::isolate_fb::tests::empty_input(1);
        input.chat_text = Some(&text);
        let first = slot.encode_snapshot_delta(&input, false);
        assert!(first.len() > text.len());
        slot.store_last_world_id(Some(123));
        slot.last_error = Some("retained diagnostic".into());
        slot.pending_logs.push("retained log".into());
        slot.pause();
        assert!(slot.has_snapshot_fingerprint());
        assert_eq!(slot.last_world_id(), Some(123));
        slot.resume();
        let delta = slot.encode_snapshot_delta(&input, false);
        assert!(delta.len() < 1024);
        slot.stop();
        assert!(!slot.has_snapshot_fingerprint());
        assert_eq!(slot.last_world_id(), None);
        assert_eq!(std::mem::take(&mut slot.ipc).into_backing_capacity(), 0);
        assert_eq!(slot.last_error.as_deref(), Some("retained diagnostic"));
        assert_eq!(slot.pending_logs, ["retained log"]);
        slot.start_compiled(Box::new(Noop), None).unwrap();
        assert_eq!(slot.encode_snapshot_delta(&input, false), first);
        // The earlier owned packet remains intact after reuse and Stop.
        assert!(crate::isolate_fb::SnapshotReader::from_bytes(&first).is_ok());
    }

    #[test]
    fn on_random_defaults_to_host_and_override_claims_handle() {
        use api::random::{DetectedRandom, RandomClaim, RandomKind};

        struct ClaimHandle;
        impl Script for ClaimHandle {
            fn name(&self) -> &str {
                "claim-handle"
            }
            fn tick(&mut self, _ctx: &mut ScriptCtx<'_>) {}
            fn on_random(&mut self, _ev: &DetectedRandom) -> RandomClaim {
                RandomClaim::Handle
            }
        }

        let ev = DetectedRandom {
            kind: RandomKind::Dialog,
            name: "genie".to_string(),
            ours: true,
            npc_index: Some(0),
        };

        // Default: Host.
        let mut s = SlotScript::new();
        s.start_compiled(Box::new(Noop), None).unwrap();
        assert_eq!(s.on_random(&ev), RandomClaim::Host);

        // Override: Handle.
        s.stop();
        s.start_compiled(Box::new(ClaimHandle), None).unwrap();
        assert_eq!(s.on_random(&ev), RandomClaim::Handle);

        // Paused: Host — the knock only fires while Running.
        s.pause();
        assert_eq!(s.on_random(&ev), RandomClaim::Host);

        // Idle (stopped): Host.
        s.stop();
        assert_eq!(s.on_random(&ev), RandomClaim::Host);
    }

    #[test]
    fn ticks_counts_dispatched_ticks_since_start() {
        let mut s = SlotScript::new();
        s.start_compiled(Box::new(Noop), None).unwrap();
        let mut d = NullDriver::default();
        s.on_game_tick(&mut ScriptCtx {
            driver: &mut d,
            tick: 1,
            here: None,
            walk: None,
            walk_with: None,
            inv: None,
            snapshot: None,
            obj_names: None,
            compiled: crate::ctx::CompiledTick::default(),
        });
        s.on_game_tick(&mut ScriptCtx {
            driver: &mut d,
            tick: 2,
            here: None,
            walk: None,
            walk_with: None,
            inv: None,
            snapshot: None,
            obj_names: None,
            compiled: crate::ctx::CompiledTick::default(),
        });
        assert_eq!(s.ticks, 2);

        // Paused ticks do not count.
        s.pause();
        s.on_game_tick(&mut ScriptCtx {
            driver: &mut d,
            tick: 3,
            here: None,
            walk: None,
            walk_with: None,
            inv: None,
            snapshot: None,
            obj_names: None,
            compiled: crate::ctx::CompiledTick::default(),
        });
        assert_eq!(s.ticks, 2);

        // A fresh Start resets the counter.
        s.stop();
        s.start_compiled(Box::new(Noop), None).unwrap();
        s.on_game_tick(&mut ScriptCtx {
            driver: &mut d,
            tick: 4,
            here: None,
            walk: None,
            walk_with: None,
            inv: None,
            snapshot: None,
            obj_names: None,
            compiled: crate::ctx::CompiledTick::default(),
        });
        assert_eq!(s.ticks, 1);
    }

    #[test]
    fn pending_withdraw_x_pause_freezes_while_stop_and_reconnect_abort() {
        let mut slot = SlotScript::new();
        slot.start_compiled(Box::new(Noop), None).unwrap();
        slot.set_pending_withdraw_x(Some(PendingWithdrawX::waiting_dialog(2, 7, 0, 7, 3)));
        assert_eq!(
            slot.pending_withdraw_x().unwrap().remaining,
            Duration::from_millis(3000)
        );

        slot.pause();
        let paused = slot
            .pending_withdraw_x()
            .expect("Pause retains pending work");
        assert!(paused.deadline.is_none(), "Pause freezes monotonic time");
        slot.resume();
        assert!(
            slot.pending_withdraw_x().unwrap().deadline.is_some(),
            "Resume restores the remaining deadline"
        );

        slot.stop();
        assert!(
            slot.pending_withdraw_x().is_none(),
            "Stop aborts pending work"
        );

        slot.start_compiled(Box::new(Noop), None).unwrap();
        slot.set_pending_withdraw_x(Some(PendingWithdrawX::waiting_dialog(2, 7, 0, 7, 3)));
        let before_reset = slot.withdraw_x_result();
        slot.reset_session_work();
        assert!(
            slot.pending_withdraw_x().is_none(),
            "reconnect/session reset aborts pending work"
        );
        assert_eq!(
            slot.withdraw_x_result(),
            (before_reset.0.wrapping_add(1), false),
            "session reset publishes an explicit abort even if a later bank reuses the generation"
        );

        slot.set_pending_withdraw_x(Some(PendingWithdrawX::waiting_load_dialog(
            2, 20, 1, 0, 20, 3,
        )));
        let before_load_reset = slot.withdraw_load_result();
        slot.reset_session_work();
        assert_eq!(
            slot.withdraw_load_result(),
            (before_load_reset.0.wrapping_add(1), false),
            "withdrawLoad gets the same explicit abort on generation-reusing session reset"
        );

        let settlement = PendingWithdrawX::waiting_dialog(2, 7, 0, 7, 3).waiting_settlement();
        assert_eq!(settlement.remaining, Duration::from_millis(4000));

        let mut expired = PendingWithdrawX::waiting_dialog(2, 7, 0, 7, 3);
        expired.deadline = Some(Instant::now() - Duration::from_millis(1));
        assert!(expired.expired(), "expiry uses monotonic wall time");
    }

    #[test]
    fn pending_bank_op_pause_freezes_while_stop_and_reconnect_abort() {
        let mut slot = SlotScript::new();
        slot.start_compiled(Box::new(Noop), None).unwrap();
        slot.set_pending_bank_op(Some(PendingBankOp::new(
            PendingBankOpKind::Deposit,
            1,
            3,
            0,
            7,
        )));
        assert_eq!(
            slot.pending_bank_op().unwrap().remaining,
            Duration::from_millis(2000)
        );

        slot.pause();
        assert!(
            slot.pending_bank_op().unwrap().deadline.is_none(),
            "Pause freezes the ordinary bank deadline"
        );
        slot.resume();
        assert!(slot.pending_bank_op().unwrap().deadline.is_some());

        slot.stop();
        assert!(slot.pending_bank_op().is_none(), "Stop drops old-slot work");

        slot.start_compiled(Box::new(Noop), None).unwrap();
        slot.set_pending_bank_op(Some(PendingBankOp::new(
            PendingBankOpKind::Withdraw,
            1,
            20,
            0,
            7,
        )));
        assert_eq!(
            slot.pending_bank_op().unwrap().remaining,
            Duration::from_millis(4000)
        );
        let before = slot.bank_op_result();
        slot.reset_session_work();
        assert!(slot.pending_bank_op().is_none());
        assert_eq!(slot.bank_op_result(), (before.0.wrapping_add(1), false));
    }

    #[cfg(feature = "load")]
    #[test]
    fn fenced_settings_reject_stale_identity_generation_and_unchanged_bag() {
        let mut slot = SlotScript::new();
        slot.start_compiled(Box::new(Noop), None).unwrap();
        slot.attach_source_identity("catalog:ChickenKiller");
        let gen = slot.runtime_generation();
        let mut bag = serde_json::Map::new();
        bag.insert("x".into(), serde_json::json!(1));
        assert!(slot.post_settings_bag_fenced(&bag, "catalog:ChickenKiller", gen));
        assert!(
            !slot.post_settings_bag_fenced(&bag, "catalog:ChickenKiller", gen),
            "unchanged bag is not reposted"
        );
        assert!(!slot.post_settings_bag_fenced(&bag, "catalog:Other", gen));
        assert!(!slot.post_settings_bag_fenced(&bag, "catalog:ChickenKiller", gen.wrapping_add(1)));
        slot.stop();
        assert!(slot.source_identity().is_none());
        assert_ne!(slot.runtime_generation(), gen);
        assert!(!slot.post_settings_bag_fenced(&bag, "catalog:ChickenKiller", gen));
    }

    #[test]
    fn stop_clears_identity_and_bumps_runtime_generation() {
        let mut slot = SlotScript::new();
        slot.start_compiled(Box::new(Noop), None).unwrap();
        slot.attach_source_identity("file:shared.ts");
        let gen = slot.runtime_generation();
        slot.stop();
        assert!(slot.source_identity().is_none());
        assert_eq!(slot.state(), RunState::Idle);
        assert_ne!(
            slot.runtime_generation(),
            gen,
            "Stop must invalidate the previous execution generation"
        );
    }

    #[cfg(feature = "load")]
    #[test]
    fn slot_stop_delivers_final_logs_exactly_once() {
        let mut slot = SlotScript::new();
        slot.start_load(
            "export default class T extends LoopingBot {
            loop() {}
            onStop() { this.log('stopped-ok'); }
        }"
            .into(),
            LoadShape::CompatClass,
            vec![],
        )
        .unwrap();
        slot.load.as_ref().unwrap().on_game_tick(1);
        let _ = slot.probe("1");
        slot.stop();
        assert_eq!(slot.state(), RunState::Idle);
        let logs = slot.take_pending_logs();
        let hits = logs.iter().filter(|l| l.contains("stopped-ok")).count();
        assert_eq!(hits, 1, "exactly one onStop log after take: {logs:?}");
        slot.stop();
        let again = slot.take_pending_logs();
        assert!(
            again.iter().all(|l| !l.contains("stopped-ok")),
            "second stop must not rerun the hook: {again:?}"
        );
    }

    #[cfg(feature = "load")]
    #[test]
    fn self_stop_runs_hook_once_then_slot_stop_does_not() {
        let mut slot = SlotScript::new();
        slot.start_load(
            r#"
import { ScriptRunner } from '../../runtime/ScriptRunner.js';
export default class T extends LoopingBot {
    loop() { ScriptRunner.stop('done'); }
    onStop() { this.log('stopped-ok'); }
}
"#
            .into(),
            LoadShape::CompatClass,
            vec![],
        )
        .unwrap();
        slot.load.as_ref().unwrap().on_game_tick(1);
        let deadline = Instant::now() + Duration::from_secs(5);
        while slot.state() == RunState::Running && Instant::now() < deadline {
            let _ = slot.drain_logs();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(slot.state(), RunState::Idle);
        let logs = slot.take_pending_logs();
        let hits = logs.iter().filter(|l| l.contains("stopped-ok")).count();
        assert_eq!(hits, 1, "self-stop onStop once: {logs:?}");
        assert!(logs.iter().any(|l| l.contains("script requested stop")));
        slot.stop();
        let again = slot.take_pending_logs();
        assert!(
            again.iter().all(|l| !l.contains("stopped-ok")),
            "join after self-stop then Drop must not rerun: {again:?}"
        );
    }

    #[cfg(feature = "load")]
    #[test]
    fn watchdog_restart_folds_onstop_logs_into_pending() {
        let mut slot = SlotScript::new();
        slot.start_load(
            "export default class T extends LoopingBot {
            onStart() { globalThis.__gen = (globalThis.__gen || 0) + 1; }
            loop() { globalThis.__n = (globalThis.__n || 0) + 1; }
            onStop() { this.log('stopped-ok'); }
        }"
            .into(),
            LoadShape::CompatClass,
            vec![],
        )
        .unwrap();
        slot.load.as_ref().unwrap().on_game_tick(1);
        let _ = slot.probe("1");
        slot.restart_load_from_identity(Instant::now())
            .expect("restart from identity");
        let logs = slot.take_pending_logs();
        assert!(
            logs.iter().any(|l| l.contains("stopped-ok")),
            "dying isolate onStop must land in pending_logs: {logs:?}"
        );
        slot.load.as_ref().unwrap().on_game_tick(1);
        assert_eq!(slot.probe("__gen").unwrap(), 1, "new isolate onStart runs");
        slot.stop();
    }

    #[cfg(feature = "load")]
    #[test]
    fn restart_still_refuses_pause() {
        let mut slot = SlotScript::new();
        slot.start_load(
            "export default class T extends LoopingBot { loop() {} onStop() { this.log('stopped-ok'); } }"
                .into(),
            LoadShape::CompatClass,
            vec![],
        )
        .unwrap();
        slot.pause();
        let err = slot.restart_load_from_identity(Instant::now()).unwrap_err();
        assert!(
            err.contains("not running") || err.contains("pause") || err.contains("frozen"),
            "{err}"
        );
        slot.stop();
    }

    /// A compiled card that queues one walk per tick onto the ctx's sink —
    /// the queue the slot parks there — and nothing else.
    #[cfg(feature = "load")]
    #[derive(Default)]
    struct Walker;

    #[cfg(feature = "load")]
    impl Script for Walker {
        fn name(&self) -> &str {
            "Walker"
        }

        fn tick(&mut self, ctx: &mut ScriptCtx<'_>) {
            if let Some(sink) = ctx.compiled.interacts.as_mut() {
                sink.push(crate::shim::InteractReq::Walk {
                    x: 3,
                    z: 4,
                    level: 0,
                    allow_teleports: false,
                    allow_wilderness: false,
                    allow_bank_fetch: false,
                    request_id: 0,
                });
            }
        }
    }

    /// The walk `Walker` queues, for comparing whole requests.
    #[cfg(feature = "load")]
    fn walker_walk() -> crate::shim::InteractReq {
        crate::shim::InteractReq::Walk {
            x: 3,
            z: 4,
            level: 0,
            allow_teleports: false,
            allow_wilderness: false,
            allow_bank_fetch: false,
            request_id: 0,
        }
    }

    /// A ctx with no views wired and nothing parked: a compiled tick over it
    /// is the slot's own queue, installed by the slot.
    #[cfg(feature = "load")]
    fn compiled_ctx<'a>(
        driver: &'a mut dyn api::interact::Driver,
        selected: Option<&'a api::game_data::SelectedGameData>,
    ) -> ScriptCtx<'a> {
        ScriptCtx {
            driver,
            tick: 1,
            here: None,
            walk: None,
            walk_with: None,
            inv: None,
            snapshot: None,
            obj_names: None,
            compiled: crate::CompiledTick {
                selected,
                hold: false,
                interacts: None,
            },
        }
    }

    #[cfg(feature = "load")]
    #[test]
    fn a_compiled_tick_queues_onto_the_slot_and_one_drain_takes_it() {
        let mut slot = SlotScript::new();
        slot.start_compiled(Box::new(Walker), None).unwrap();
        let mut d = NullDriver::default();
        slot.on_game_tick(&mut compiled_ctx(&mut d, None));
        assert_eq!(
            slot.drain_interacts(),
            vec![walker_walk()],
            "the compiled card's verbs ride the slot's own drain"
        );
        assert!(
            slot.drain_interacts().is_empty(),
            "the drain takes the queue, it never replays it"
        );

        // A paused card is not ticked, so it queues nothing.
        slot.pause();
        slot.on_game_tick(&mut compiled_ctx(&mut d, None));
        assert!(slot.drain_interacts().is_empty());

        // Stop drops the instance and the queue it had not spent.
        slot.resume();
        slot.on_game_tick(&mut compiled_ctx(&mut d, None));
        slot.stop();
        assert!(
            slot.drain_interacts().is_empty(),
            "Stop must not leak a dead card's requests into the next Start"
        );
    }

    #[cfg(feature = "load")]
    #[test]
    fn a_compiled_start_pins_the_selected_facts_and_stop_clears_them() {
        let data =
            api::game_data::for_revision(client::io::ClientRevision::R274).expect("selected data");
        let mut slot = SlotScript::new();
        assert!(slot.compiled_game_data().is_none());
        slot.start_compiled(Box::new(Noop), Some(Arc::clone(&data)))
            .unwrap();
        assert!(
            slot.compiled_game_data().is_some(),
            "the Start pin rides out to the ctx"
        );
        slot.stop();
        assert!(
            slot.compiled_game_data().is_none(),
            "a stopped card keeps no pin"
        );
        slot.start_compiled(Box::new(Noop), None).unwrap();
        assert!(
            slot.compiled_game_data().is_none(),
            "a Start with no pin is what a compiled identify fails closed on"
        );
    }

    #[cfg(feature = "load")]
    #[test]
    fn the_pump_freezes_and_aborts_the_compiled_clue_machine() {
        let data =
            api::game_data::for_revision(client::io::ClientRevision::R274).expect("selected data");
        // The machine's own identify decides what is held: a selected
        // membership row with a positive count.
        let held_id = data
            .trails()
            .expect("trails")
            .rows
            .iter()
            .find(|row| row.role == "clue")
            .expect("a selected clue row")
            .id;
        let mut slot = SlotScript::new();
        slot.start_compiled(
            Box::new(crate::sherlock::Sherlock::default()),
            Some(Arc::clone(&data)),
        )
        .unwrap();
        let begin = crate::clue::dispatch(
            Some(&data),
            &serde_json::json!({ "op": "begin", "generation": 0, "held": [[held_id, 1]] }),
        );
        assert_eq!(begin["kind"], "token", "{begin}");
        let token = begin["token"].as_u64().expect("token");
        let next = || {
            serde_json::json!({
                "op": "next",
                "token": token,
                "generation": 0,
                "held": [[held_id, 1]],
            })
        };

        // Running and unfrozen: the machine posts its own gate question.
        slot.sync_compiled_clue(false);
        assert_eq!(
            crate::clue::dispatch(Some(&data), &next())["kind"],
            "callback.enabled"
        );

        // Operator Pause, on a frame that dispatches no tick at all: the live
        // token waits instead, so the pause cannot burn the session's clock.
        slot.pause();
        slot.sync_compiled_clue(false);
        assert_eq!(crate::clue::dispatch(Some(&data), &next())["kind"], "wait");
        slot.resume();
        slot.sync_compiled_clue(false);
        assert_eq!(
            crate::clue::dispatch(Some(&data), &next())["kind"],
            "callback.enabled",
            "a thaw resumes the same session"
        );

        // The guardian's hold freezes the same live session the same way.
        slot.sync_compiled_clue(true);
        assert_eq!(crate::clue::dispatch(Some(&data), &next())["kind"], "wait");
        slot.sync_compiled_clue(false);

        // Stop is marked wherever it ran and applied on this thread: the live
        // token is gone, and the next Start begins a fresh session.
        slot.stop();
        slot.sync_compiled_clue(false);
        let after = crate::clue::dispatch(Some(&data), &next());
        assert_eq!(after["kind"], "aborted", "{after}");
        let again = crate::clue::dispatch(
            Some(&data),
            &serde_json::json!({ "op": "begin", "generation": 0, "held": [[held_id, 1]] }),
        );
        assert_eq!(again["kind"], "token", "{again}");
        assert_ne!(
            again["token"].as_u64(),
            Some(token),
            "the aborted session's token is not reused"
        );
    }

    /// The two resets are not each other. A connection boundary
    /// (`reset_session_work`) aborts the compiled clue machine's live step and
    /// its token and keeps what the session still owes — the Entrana strip list
    /// the reclaim reads — while operator Stop is a fresh task instance and
    /// clears it with the step.
    #[cfg(feature = "load")]
    #[test]
    fn a_session_reset_keeps_the_clue_strip_list_and_a_stop_clears_it() {
        let data =
            api::game_data::for_revision(client::io::ClientRevision::R274).expect("selected data");
        // The Entrana-box proof row, and a worn name the frozen matcher folds.
        const ENTRANA: i32 = 3579;
        const HELM: i32 = 1163;
        let mut slot = SlotScript::new();
        slot.start_compiled(
            Box::new(crate::sherlock::Sherlock::default()),
            Some(Arc::clone(&data)),
        )
        .unwrap();
        let begin = crate::clue::dispatch(
            Some(&data),
            &serde_json::json!({ "op": "begin", "generation": 0, "held": [[ENTRANA, 1]] }),
        );
        assert_eq!(begin["kind"], "token", "{begin}");
        let token = begin["token"].as_u64().expect("token");
        // The landed gate first: enabled, the report, the row's own status.
        let next = |extra: serde_json::Value| {
            let mut call = serde_json::json!({
                "op": "next",
                "token": token,
                "generation": 0,
                "held": [[ENTRANA, 1]],
            });
            for (key, value) in extra.as_object().expect("extra") {
                call[key] = value.clone();
            }
            crate::clue::dispatch(Some(&data), &call)
        };
        assert_eq!(next(serde_json::json!({}))["kind"], "callback.enabled");
        assert_eq!(
            next(serde_json::json!({ "resume": true }))["kind"],
            "callback.log"
        );
        assert_eq!(next(serde_json::json!({}))["kind"], "callback.setStatus");
        // One strip step: the worn restricted row goes off, and the name is
        // listed for the reclaim.
        let strip = next(serde_json::json!({
            "equipment": [{ "id": HELM, "name": "Rune full helm", "count": 1, "slot": 0 }],
        }));
        assert_eq!(strip["kind"], "unequip", "{strip}");
        let owns = |data: &Arc<api::game_data::SelectedGameData>| {
            crate::clue::dispatch(Some(data), &serde_json::json!({ "op": "ownsEquipment" }))["owns"]
                == true
        };
        assert!(owns(&data), "the strip listed the name");

        // The connection boundary, applied the way the pump applies it.
        slot.reset_session_work();
        slot.sync_compiled_clue(false);
        let dead = crate::clue::dispatch(
            Some(&data),
            &serde_json::json!({
                "op": "next",
                "token": token,
                "generation": 0,
                "held": [[ENTRANA, 1]],
            }),
        );
        assert_eq!(dead["kind"], "aborted", "the boundary kills the step");
        assert!(
            owns(&data),
            "and keeps the list the reclaim still owes after a relog"
        );

        // Operator Stop: the fresh instance starts the session over.
        slot.stop();
        slot.sync_compiled_clue(false);
        assert!(!owns(&data), "Stop clears the strip list with the step");
    }
}
