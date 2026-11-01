//! Sealed external-script input authority and the shared host envelope.
//!
//! This module defines no script loader or authoring binding. The loader's
//! admitted capability metadata remains the only source of an external permit.

use crate::interact::{InteractReq, QueuedInteract};
use crate::native::seat_rows::QueuedSeatBatch;
use crate::native::snapshot_source::{BankSelectionInput, NativeFactsInput, SnapshotInput};
use crate::native::{recovery_hints::RecoveryHintsCell, ActionContext, ActionError, SettingsBag};
use crate::paint::ScriptPaint;
use api::identity::ExecutionId;
use api::interact::server_ordered::ServerOrderedRequest;
use api::native_input::NativeInputAuthority;
use api::prot::Send as PacketSend;
use api::selected::RunKey;
use std::{
    any::Any,
    sync::{mpsc::Sender, Arc, Mutex, MutexGuard, OnceLock},
    time::Instant,
};

/// The runtime a card needs. This value selects its factory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SurfaceKind {
    /// JavaScript runtime.
    Js,
}

impl SurfaceKind {
    /// The number of surface kinds in the inline factory array.
    pub const COUNT: usize = 1;

    /// The index of this kind in the inline factory array.
    pub const fn index(self) -> usize {
        self as usize
    }
}

/// The API family that a running card uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SurfaceFamily {
    /// Version 1 of the JavaScript API.
    JsV1,
    /// Version 2 of the JavaScript API.
    JsV2,
}

/// Frozen inputs for one card. The surface crate builds the card and exposes
/// its concrete value through [`SurfaceCard::as_any`].
pub trait SurfaceCard: Any + Send + Sync + std::fmt::Debug {
    /// Return the runtime this card requires.
    fn kind(&self) -> SurfaceKind;

    /// Return the API family the card uses, if it has one.
    fn family(&self) -> Option<SurfaceFamily>;

    /// Return this card as `Any` for a factory-specific downcast.
    fn as_any(&self) -> &dyn Any;
}

/// Shared values that the slot retains when it restarts a run.
///
/// Cloning this value clones its five `Arc`s. It does not copy their contents.
#[derive(Clone, Debug)]
pub struct SurfaceStart {
    /// The surface-owned card.
    pub card: Arc<dyn SurfaceCard>,
    /// The selected settings bag, when present.
    pub settings_bag: Option<Arc<SettingsBag>>,
    /// The selected game data, when present.
    pub game_data: Option<Arc<api::game_data::SelectedGameData>>,
    /// Immutable named-bank facts for this start.
    pub named_banks: Arc<api::named_banks::NamedBankFacts>,
    /// Shared loadouts for this start.
    pub loadouts: Arc<[api::loadout::Loadout]>,
}

/// Creates one surface run for a card.
pub trait SurfaceFactory: Send + Sync {
    /// Return the runtime this factory serves.
    fn kind(&self) -> SurfaceKind;

    /// Spawn one run with the slot's shared recovery-hints cell.
    ///
    /// The slot passes the same `start` and `recovery_hints` values on every
    /// restart. The factory posts loadouts, recovery hints, and the settings
    /// bag before it returns, in that order.
    fn spawn(
        &self,
        start: &SurfaceStart,
        execution: ExecutionId,
        recovery_hints: Arc<RecoveryHintsCell>,
    ) -> Result<Box<dyn SurfaceRun>, SurfaceError>;
    /// Spawn with the slot's bound local-state capability when the surface
    /// can consume it. Existing factories retain their original entry point.
    fn spawn_with_state(
        &self,
        start: &SurfaceStart,
        execution: ExecutionId,
        recovery_hints: Arc<RecoveryHintsCell>,
        local_state: Option<Arc<crate::local_state::SlotState>>,
    ) -> Result<Box<dyn SurfaceRun>, SurfaceError> {
        let _ = local_state;
        self.spawn(start, execution, recovery_hints)
    }
}

/// Receipt returned when the script calls its stop operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptStopReceipt {
    /// The game tick at which the script stopped.
    pub tick: u64,
    /// The script's stop reason.
    pub reason: String,
}

/// The result of one script tick.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TickOutcome {
    /// The tick completed successfully.
    Success { tick: u64, generation: u64 },
    /// The tick failed with a message.
    Error {
        tick: u64,
        generation: u64,
        message: String,
    },
}

/// Readiness of a newly spawned surface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SurfaceReady {
    /// The surface has not completed setup.
    Pending,
    /// The surface can run.
    Ready,
    /// Setup failed with a diagnostic message.
    Failed(String),
}
/// Lifecycle projection exposed to a JavaScript run context.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RuntimeRunState {
    /// No run has started in this isolate.
    #[default]
    Idle,
    /// The run may execute, including while a host hold temporarily gates ticks.
    Running,
    /// The operator paused this run.
    Paused,
    /// The run is retiring.
    Stopping,
    /// The run ended without an error.
    Stopped,
    /// The run ended with an error.
    Crashed,
}

impl RuntimeRunState {
    /// The compatibility API state spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Running => "running",
            Self::Paused => "paused",
            Self::Stopping => "stopping",
            Self::Stopped => "stopped",
            Self::Crashed => "crashed",
        }
    }

    /// Whether this run has been cancelled or reached a terminal state.
    pub const fn aborted(self) -> bool {
        matches!(self, Self::Stopping | Self::Stopped | Self::Crashed)
    }
}

/// Slot-owned lifecycle requests submitted by the isolate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuntimeControlRequest {
    /// Pause the owning run.
    Pause,
    /// Resume the owning run after an operator pause.
    Resume,
}

/// Exact terminal activity result retained until the next activity start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeActivityOutcome {
    /// `finished`, `failed`, or `interrupted`.
    pub kind: String,
    /// The caller or child-provided reason.
    pub reason: String,
}

/// Bounded state for the single child activity owned by a script run.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RuntimeActivityState {
    /// Isolate-local identity of the active child.
    pub child_id: Option<u64>,
    /// Last terminal outcome, if any.
    pub outcome: Option<RuntimeActivityOutcome>,
    /// Whether one child callback is currently awaiting completion.
    pub in_flight: bool,
    /// Whether interruption was requested for this child.
    pub interrupted: bool,
    /// Whether `finish` was requested by the child.
    pub finish_requested: bool,
    /// Wall-clock eligibility bound in milliseconds.
    pub next_at_ms: f64,
    /// Server-tick eligibility bound.
    pub next_tick: f64,
    /// The actual active TaskBot name, when the child has one.
    pub active_task: Option<String>,
}

/// Run-fenced native state shared by the slot and its isolate.
#[derive(Clone, Debug, PartialEq)]
pub struct ScriptRuntimeSnapshot {
    /// Slot generation that owns this run.
    pub generation: u64,
    /// Current script lifecycle.
    pub state: RuntimeRunState,
    /// Native stop receipt reason, if one exists.
    pub stop_reason: Option<String>,
    /// Expected host-queue work that holds the loop watchdog.
    pub watchdog_hold: Option<String>,
    /// Stable source key attached by the slot.
    pub source_identity: Option<String>,
    /// The one activity owned by this run.
    pub activity: RuntimeActivityState,
    /// A control request awaiting the slot lifecycle callback.
    pub control_request: Option<RuntimeControlRequest>,
}

/// Run-fenced state shared by the slot and its V8 isolate.
///
/// The slot creates a fresh value for every isolate generation. The isolate
/// may project or update its owned child activity, but does not own script
/// lifecycle transitions.
pub struct ScriptRuntimeContext {
    state: Mutex<ScriptRuntimeSnapshot>,
    native_input: OnceLock<Arc<NativeInputAuthority>>,
}

impl ScriptRuntimeContext {
    /// Create context for one runtime generation.
    pub fn new(generation: u64) -> Self {
        Self {
            state: Mutex::new(ScriptRuntimeSnapshot {
                generation,
                state: RuntimeRunState::Idle,
                stop_reason: None,
                watchdog_hold: None,
                source_identity: None,
                activity: RuntimeActivityState::default(),
                control_request: None,
            }),
            native_input: OnceLock::new(),
        }
    }

    fn lock(&self) -> MutexGuard<'_, ScriptRuntimeSnapshot> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }

    /// Copy the current run and child state for an isolate callback.
    pub fn snapshot(&self) -> ScriptRuntimeSnapshot {
        self.lock().clone()
    }
    /// The slot-local authority shared with native input consumption.
    pub(crate) fn native_input(&self) -> &Arc<NativeInputAuthority> {
        self.native_input.get_or_init(NativeInputAuthority::new)
    }

    /// Bind the host-owned input authority before a run starts.
    pub(crate) fn bind_native_input(
        &self,
        authority: Arc<NativeInputAuthority>,
    ) -> Result<(), Arc<NativeInputAuthority>> {
        self.native_input.set(authority)
    }

    /// Create a fresh run context carrying the same slot-local input authority.
    pub(crate) fn with_native_input(generation: u64, authority: Arc<NativeInputAuthority>) -> Self {
        let context = Self::new(generation);
        let _ = context.native_input.set(authority);
        context
    }

    /// Preserve a context snapshot while replacing its slot-local authority.
    pub(crate) fn with_snapshot_and_native_input(
        snapshot: ScriptRuntimeSnapshot,
        authority: Arc<NativeInputAuthority>,
    ) -> Self {
        let context = Self {
            state: Mutex::new(snapshot),
            native_input: OnceLock::new(),
        };
        let _ = context.native_input.set(authority);
        context
    }
    /// Read the current state without copying the rest of the context.
    pub fn state(&self) -> RuntimeRunState {
        self.lock().state
    }

    /// Update the slot-owned lifecycle projection.
    pub fn set_state(&self, state: RuntimeRunState) {
        let mut current = self.lock();
        if current.state != state {
            current.state = state;
        }
        if state.aborted() {
            current.control_request = None;
        }
    }
    /// Queue a run-fenced pause request for the owning slot.
    pub fn request_pause(&self) -> bool {
        let mut current = self.lock();
        if current.state != RuntimeRunState::Running || current.control_request.is_some() {
            return false;
        }
        current.control_request = Some(RuntimeControlRequest::Pause);
        true
    }

    /// Queue a run-fenced resume request only after operator pause.
    pub fn request_resume(&self) -> bool {
        let mut current = self.lock();
        if current.state != RuntimeRunState::Paused || current.control_request.is_some() {
            return false;
        }
        current.control_request = Some(RuntimeControlRequest::Resume);
        true
    }

    /// Take one pending request at the slot lifecycle boundary.
    pub fn take_control_request(&self) -> Option<RuntimeControlRequest> {
        self.lock().control_request.take()
    }

    /// Retain the exact source identity attached to this run.
    pub fn set_source_identity(&self, identity: Option<String>) {
        let mut current = self.lock();
        if current.source_identity != identity {
            current.source_identity = identity;
        }
    }

    /// Retain the exact terminal reason from the slot lifecycle receipt.
    pub fn set_stop_reason(&self, reason: Option<String>) {
        let mut current = self.lock();
        if current.stop_reason != reason {
            current.stop_reason = reason;
        }
    }
    /// Read the current host-queue watchdog hold.
    pub fn watchdog_hold(&self) -> Option<String> {
        self.lock().watchdog_hold.clone()
    }

    /// Set the host-queue watchdog hold exposed by the run context.
    pub fn set_watchdog_hold(&self, hold: Option<String>) {
        let mut current = self.lock();
        if current.watchdog_hold != hold {
            current.watchdog_hold = hold;
        }
    }

    /// Register the one child after the native factory has created it.
    pub fn start_activity(&self, child_id: u64) -> Result<(), &'static str> {
        let mut current = self.lock();
        if current.state.aborted() {
            return Err("Script run has ended");
        }
        if current.activity.child_id.is_some() || current.activity.in_flight {
            return Err("An activity is already running");
        }
        current.activity = RuntimeActivityState {
            child_id: Some(child_id),
            ..RuntimeActivityState::default()
        };
        Ok(())
    }

    /// Whether `child_id` is still the run's current child.
    pub fn owns_activity(&self, child_id: u64) -> bool {
        self.lock().activity.child_id == Some(child_id)
    }

    /// Begin one eligible child loop turn.
    pub fn begin_activity_loop(&self, child_id: u64, tick: f64, now_ms: f64) -> bool {
        let mut current = self.lock();
        if current.state.aborted() {
            return false;
        }
        let activity = &mut current.activity;
        if activity.child_id != Some(child_id)
            || activity.in_flight
            || tick < activity.next_tick
            || now_ms < activity.next_at_ms
        {
            return false;
        }
        activity.in_flight = true;
        true
    }

    /// Record one child loop result and its next native eligibility bounds.
    pub fn settle_activity_loop(&self, child_id: u64, next_at_ms: f64, next_tick: f64) {
        let mut current = self.lock();
        let activity = &mut current.activity;
        if activity.child_id != Some(child_id) {
            return;
        }
        activity.in_flight = false;
        activity.next_at_ms = next_at_ms;
        activity.next_tick = next_tick;
    }

    /// Record child callback entry for the `start` turn.
    pub fn begin_activity_start(&self, child_id: u64) -> bool {
        let mut current = self.lock();
        if current.state.aborted() {
            return false;
        }
        let activity = &mut current.activity;
        if activity.child_id != Some(child_id) || activity.in_flight {
            return false;
        }
        activity.in_flight = true;
        true
    }

    /// Complete one child `onStart` turn.
    pub fn settle_activity_start(&self, child_id: u64) {
        let mut current = self.lock();
        if current.activity.child_id == Some(child_id) {
            current.activity.in_flight = false;
        }
    }

    /// Publish one actual active task name from the child.
    pub fn set_activity_task(&self, child_id: u64, name: Option<String>) {
        let mut current = self.lock();
        if current.activity.child_id == Some(child_id) {
            current.activity.active_task = name;
        }
    }

    /// Retain a terminal child result.
    pub fn set_activity_outcome(
        &self,
        child_id: u64,
        kind: String,
        reason: String,
        finish_requested: bool,
    ) {
        let mut current = self.lock();
        let activity = &mut current.activity;
        if activity.child_id == Some(child_id) {
            activity.finish_requested = finish_requested;
            activity.outcome = Some(RuntimeActivityOutcome { kind, reason });
        }
    }

    /// Publish child interruption and return whether its callback is pending.
    pub fn interrupt_activity(&self, child_id: Option<u64>, reason: String) -> Option<bool> {
        let mut current = self.lock();
        let activity = &mut current.activity;
        if activity.child_id != child_id {
            return None;
        }
        activity.interrupted = true;
        activity.outcome = Some(RuntimeActivityOutcome {
            kind: "interrupted".into(),
            reason,
        });
        Some(activity.in_flight)
    }

    /// Release the child identity after its single cleanup path.
    pub fn clear_activity(&self, child_id: u64) {
        let mut current = self.lock();
        let activity = &mut current.activity;
        if activity.child_id == Some(child_id) {
            activity.child_id = None;
            activity.in_flight = false;
            activity.finish_requested = false;
            activity.active_task = None;
        }
    }
}

/// The identifiers the host compares for the latest and previous route
/// inspection results.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RouteInspectIdentities {
    /// `(sequence, generation, request id)` for the latest result.
    pub latest: (u64, u64, u64),
    /// `(sequence, generation, request id)` for the previous result.
    pub prev: (u64, u64, u64),
    /// The replaced latest request id.
    pub replaced_id: u64,
    /// The replaced previous request id.
    pub replaced_prev_id: u64,
    /// The three refused request ids.
    pub refused: [u64; 3],
}

/// The allocation-free subset of the last accepted snapshot that the host
/// compares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PostedSnapshotFacts {
    /// Sequence of the latest withdraw-X result.
    pub withdraw_x_result_seq: u64,
    /// Sequence of the latest bank-operation result.
    pub bank_op_result_seq: u64,
    /// Sequence of the latest walk outcome.
    pub walk_outcome_seq: u64,
    /// Bank-selection identity from the accepted snapshot.
    pub bank_selection: BankSelectionInput,
    /// Route-inspection identities from the accepted snapshot.
    pub route_inspect: RouteInspectIdentities,
}

/// A surface startup or operation failure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SurfaceError {
    /// The run could not start.
    #[error("{0}")]
    Spawn(String),
    /// A probe request failed.
    #[error("{0}")]
    Probe(String),
    /// No bound factory serves this card's runtime.
    #[error("no surface factory for {0:?}")]
    NoFactory(SurfaceKind),
    /// The factory received a card that it did not build.
    #[error("surface card is not a {0:?} card")]
    ForeignCard(SurfaceKind),
}

/// A queued host-owned client frame whose isolate acknowledgement is awaited
/// without holding the owning slot mutex.
pub trait ClientFrameDispatch: Send + Sync {
    /// Wait for the isolate to finish this frame and return whether it
    /// acknowledged delivery.
    fn wait_for_ack(&self) -> bool;
}

/// One running surface. The slot owns lifecycle transitions, and `stop`
/// transfers the run to its retirement worker.
pub trait SurfaceRun: Send {
    // setup, identity, session
    /// Return the current setup state.
    fn poll_ready(&self) -> SurfaceReady;
    /// Return the run generation.
    fn generation(&self) -> u64;
    /// Bind the slot-owned run context. Non-JavaScript surfaces need no
    /// isolate projection and retain the default no-op.
    fn bind_runtime_context(&mut self, _context: Arc<ScriptRuntimeContext>) {}
    /// Return whether script execution is active.
    fn execution_active(&self) -> bool;
    /// Return the execution sequence and its activity flag.
    fn execution_sequence(&self) -> (u64, bool);
    /// Align the run with the slot's current execution identity.
    fn align_execution(&self, execution: ExecutionId);
    /// Clear posted snapshot facts and discard session work.
    fn reset_session_work(&mut self, execution: Option<ExecutionId>) -> u64;
    /// Clear posted snapshot facts and reconnect session work.
    fn reconnect_session_work(&mut self, execution: Option<ExecutionId>) -> u64;

    // operator control and terminal facts
    /// Pause script execution.
    fn pause(&self);
    /// Resume script execution.
    fn resume(&self);
    /// Queue one client frame and return its independently waitable receipt.
    /// Hosts must drop their slot lock before waiting for the receipt.
    fn begin_client_frame(&mut self) -> Option<Arc<dyn ClientFrameDispatch>> {
        None
    }
    /// Notify this run of one host-owned client main-loop frame.
    ///
    /// The host invokes this after each 20 ms client loop, independently of
    /// game-tick delivery. Implementations that do not expose frame events
    /// keep the default no-op.
    fn on_client_frame(&mut self) -> bool {
        self.begin_client_frame()
            .is_some_and(|dispatch| dispatch.wait_for_ack())
    }
    /// Return whether this run has requested the lazy projection loc list.
    fn loc_projection_wanted(&self) -> bool {
        false
    }
    /// Transfer this run to retirement and send its logs to `done`.
    fn stop(self: Box<Self>, done: Sender<Vec<String>>);
    /// Return whether the script has stopped.
    fn stopped(&self) -> bool;
    /// Return the script's stop receipt, if one exists.
    fn script_stop_receipt(&self) -> Option<ScriptStopReceipt>;
    /// Consume and return the pending script-cut request.
    fn take_script_cut(&self) -> bool;

    // posts. The slot prepares native pages before it calls these methods.
    /// Post a snapshot delta. `false` means the surface refused the post.
    fn post_snapshot_delta(
        &mut self,
        input: &SnapshotInput<'_>,
        native: NativeFactsInput<'_>,
        force_banks: bool,
    ) -> bool;
    /// Post a snapshot wake. `false` means the surface refused the post.
    fn post_snapshot_wake(
        &mut self,
        input: &SnapshotInput<'_>,
        native: NativeFactsInput<'_>,
        force_banks: bool,
        preserve_inv: bool,
    ) -> bool;
    /// Copy the host-compared facts from the last accepted snapshot.
    fn posted_snapshot_facts(&self) -> Option<PostedSnapshotFacts>;
    /// Post channel events and report whether the surface accepted them.
    fn post_channel_events(&self, events: &[InteractReq]) -> bool;
    /// Post the current settings bag.
    fn post_settings_bag(&self, bag: &SettingsBag);
    /// Post the current loadouts.
    fn post_loadouts(&self, loadouts: &[api::loadout::Loadout]);
    /// Report a game tick and its input identity.
    fn on_game_tick_at(&self, snap_tick: u64, input_identity: u64);
    /// Report a snapshot change and its input identity.
    fn on_snapshot_change_at(&self, snap_tick: u64, input_identity: u64);

    // decoded effects
    /// Drain stamped interactions queued by the surface.
    fn drain_interacts_stamped(&self) -> Vec<QueuedInteract>;
    /// Restore stamped interactions to the surface queue.
    fn restore_interacts_stamped(&self, drained: Vec<QueuedInteract>);
    /// Drain queued native seat batches.
    fn drain_seat_batches(&self) -> Vec<QueuedSeatBatch>;
    /// Restore native seat batches to the surface queue.
    fn restore_seat_batches(&self, drained: Vec<QueuedSeatBatch>);
    /// Drain queued lifecycle interactions.
    fn drain_lifecycle(&self) -> Vec<InteractReq>;
    /// Drain queued tick outcomes.
    fn drain_tick_outcomes(&self) -> Vec<TickOutcome>;
    /// Discard queued mouse interactions.
    fn discard_mouse_interacts(&self);
    /// Rearm a host-discarded walk with its token.
    fn rearm_host_discarded_walk(&self, token: u64);
    /// Request a recovery anchor from the surface.
    fn request_recovery_anchor(&self);

    // walking facts
    /// Return whether a walking operation is active.
    fn live_walking_operation(&self) -> bool;
    /// Record the sequence of a manual walk takeover.
    fn note_manual_walk_takeover(&self, user_move_intent_seq: u64);

    // paint, probe, logs, randoms
    /// Return the latest paint frame, if one exists.
    fn paint(&self) -> Option<Arc<ScriptPaint>>;
    /// Send a paint-button click to the surface.
    fn paint_click(&self, id: &str);
    /// Select a paint item by key and name.
    fn paint_select(&self, key: &str, name: &str);
    /// Return whether the compatibility journal paint is hidden at `now`.
    fn journal_paint_hidden(&self, now: Instant) -> bool;
    /// Evaluate a probe expression.
    fn probe(&self, expr: &str) -> Result<serde_json::Value, SurfaceError>;
    /// Drain log lines.
    fn drain_logs(&self) -> Vec<String>;
    /// Return the ignored random names.
    fn ignored_randoms(&self) -> Vec<String>;
    /// Return whether `name` is in the ignored-random list.
    fn ignores_random(&self, name: &str) -> bool;

    // metrics
    /// Return memory metrics when available.
    fn memory_metrics(&self) -> Option<serde_json::Value>;
    /// Return memory progress.
    fn memory_progress(&self) -> serde_json::Value;

    /// Expose the concrete run only to unit and integration test hooks.
    #[cfg(any(test, feature = "test-hooks"))]
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any;

    /// Return the queued seat-batch count for test hooks.
    #[cfg(feature = "test-hooks")]
    fn seat_batch_backlog(&self) -> usize;

    /// Return the retained fold-batch count, not the number of folded rows.
    #[cfg(feature = "test-hooks")]
    fn held_fold_len(&self) -> usize;
}

#[cfg(any(test, feature = "test-hooks"))]
pub mod test_support;

/// One owned request, with no public constructor for advanced packet input.
pub struct ServerOrderedEnvelope {
    payload: Payload,
}

enum Payload {
    Content(ServerOrderedRequest),
    Advanced(AdvancedPacketOps),
}

/// A borrowed payload is dispatch data, not authority to emit another request.
pub enum ServerOrderedPayload<'a> {
    Content(&'a ServerOrderedRequest),
    Advanced(&'a [PacketSend]),
}

impl ServerOrderedEnvelope {
    pub(crate) fn content(request: ServerOrderedRequest) -> Self {
        Self {
            payload: Payload::Content(request),
        }
    }

    pub fn payload(&self) -> ServerOrderedPayload<'_> {
        match &self.payload {
            Payload::Content(request) => ServerOrderedPayload::Content(request),
            Payload::Advanced(request) => ServerOrderedPayload::Advanced(&request.operations),
        }
    }

    pub fn debug_catalog(&self) -> Option<&api::debug_commands::DebugCatalog> {
        match &self.payload {
            Payload::Content(_) => None,
            Payload::Advanced(request) => request.debug.as_deref(),
        }
    }
}

/// An external packet batch can be constructed only within this surface.
///
/// ```compile_fail,E0451
/// let _ = script::surface::AdvancedPacketOps { operations: Vec::new(), debug: None };
/// ```
pub struct AdvancedPacketOps {
    operations: Vec<PacketSend>,
    debug: Option<Arc<api::debug_commands::DebugCatalog>>,
}

/// A permit binds the external entry run and session admitted by its loader.
/// Native machines receive neither this permit nor a packet context.
///
/// ```compile_fail,E0451
/// let _ = script::surface::ScriptPacketOpsPermit { run: todo!(), debug: None };
/// ```
pub struct ScriptPacketOpsPermit {
    run: RunKey,
    debug: Option<Arc<api::debug_commands::DebugCatalog>>,
}

/// The external surface can emit packet input with its sealed live permit.
/// Built-in machines receive only [`ActionContext`].
///
/// ```compile_fail,E0599
/// fn native(cx: &mut script::native::ActionContext<'_>, packets: Vec<api::prot::Send>) {
///     cx.send_batch(packets);
/// }
/// ```
pub struct ScriptPacketOpsContext<'context, 'frame> {
    context: &'context mut ActionContext<'frame>,
    permit: ScriptPacketOpsPermit,
}

impl ScriptPacketOpsContext<'_, '_> {
    /// Queue one bounded packet batch. This does not prove server acceptance.
    pub fn send_batch(&mut self, operations: Vec<PacketSend>) -> Result<u64, ActionError> {
        if self.context.run() != self.permit.run {
            return Err(ActionError::Stale);
        }
        ServerOrderedRequest::validate_advanced_shape(self.context.pin.revision, &operations)
            .map_err(ActionError::ServerOrdered)?;
        self.context
            .emit_server_ordered_envelope(ServerOrderedEnvelope {
                payload: Payload::Advanced(AdvancedPacketOps {
                    operations,
                    debug: self.permit.debug.as_ref().map(Arc::clone),
                }),
            })
    }
}

/// Integration fixtures can exercise the real envelope without a script loader.
/// This constructor is absent from every production feature lane.
#[cfg(any(test, feature = "test-hooks"))]
pub fn packet_context_for_test<'context, 'frame>(
    context: &'context mut ActionContext<'frame>,
) -> ScriptPacketOpsContext<'context, 'frame> {
    let run = context.run();
    ScriptPacketOpsContext {
        context,
        permit: ScriptPacketOpsPermit { run, debug: None },
    }
}

/// Integration fixtures can bind the same optional catalog as an external loader.
#[cfg(any(test, feature = "test-hooks"))]
pub fn packet_context_for_test_with_debug<'context, 'frame>(
    context: &'context mut ActionContext<'frame>,
    debug: Arc<api::debug_commands::DebugCatalog>,
) -> ScriptPacketOpsContext<'context, 'frame> {
    let run = context.run();
    ScriptPacketOpsContext {
        context,
        permit: ScriptPacketOpsPermit {
            run,
            debug: Some(debug),
        },
    }
}
