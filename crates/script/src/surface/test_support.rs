use super::{
    PostedSnapshotFacts, ScriptStopReceipt, SurfaceError, SurfaceFactory, SurfaceKind,
    SurfaceReady, SurfaceRun, SurfaceStart, TickOutcome,
};
use crate::interact::{InteractReq, QueuedInteract};
use crate::native::recovery_hints::RecoveryHintsCell;
use crate::native::seat_rows::QueuedSeatBatch;
use crate::native::snapshot_source::{NativeFactsInput, SnapshotInput};
use crate::native::SettingsBag;
use crate::paint::ScriptPaint;
use api::identity::ExecutionId;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

/// A runtime-neutral card accepted by the core stub factory.
#[derive(Debug)]
pub struct StubSurfaceCard;

impl super::SurfaceCard for StubSurfaceCard {
    fn kind(&self) -> SurfaceKind {
        SurfaceKind::Js
    }

    fn family(&self) -> Option<super::SurfaceFamily> {
        None
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// One call made on a [`StubSurfaceRun`]. The fixture retains operation
/// arguments where tests may need to verify what the slot sent.
#[derive(Clone, Debug, PartialEq)]
pub enum StubSurfaceOperation {
    /// The slot queried setup readiness.
    PollReady,
    /// The slot queried the generation.
    Generation,
    /// The slot queried execution activity.
    ExecutionActive,
    /// The slot queried the execution sequence.
    ExecutionSequence,
    /// The slot aligned the execution identity.
    AlignExecution(ExecutionId),
    /// The slot reset session work.
    ResetSessionWork(Option<ExecutionId>),
    /// The slot reconnected session work.
    ReconnectSessionWork(Option<ExecutionId>),
    /// The slot paused the surface.
    Pause,
    /// The slot resumed the surface.
    Resume,
    /// The slot transferred the run to retirement.
    Stop,
    /// The slot queried the terminal state.
    Stopped,
    /// The slot queried the script stop receipt.
    ScriptStopReceipt,
    /// The slot consumed a script-cut request.
    TakeScriptCut,
    /// The slot posted a snapshot delta.
    PostSnapshotDelta { tick: u64, force_banks: bool },
    /// The slot posted a snapshot wake.
    PostSnapshotWake {
        tick: u64,
        force_banks: bool,
        preserve_inv: bool,
    },
    /// The slot posted channel events.
    PostChannelEvents(Vec<InteractReq>),
    /// The slot posted a settings bag.
    PostSettingsBag(SettingsBag),
    /// The slot posted loadouts.
    PostLoadouts(Vec<api::loadout::Loadout>),
    /// The slot reported a game tick.
    OnGameTickAt { tick: u64, input_identity: u64 },
    /// The slot reported a snapshot change.
    OnSnapshotChangeAt { tick: u64, input_identity: u64 },
    /// The slot drained stamped interactions.
    DrainInteractsStamped,
    /// The slot restored stamped interactions.
    RestoreInteractsStamped { count: usize },
    /// The slot drained seat batches.
    DrainSeatBatches,
    /// The slot restored seat batches.
    RestoreSeatBatches { count: usize },
    /// The slot drained lifecycle interactions.
    DrainLifecycle,
    /// The slot drained tick outcomes.
    DrainTickOutcomes,
    /// The slot discarded queued mouse interactions.
    DiscardMouseInteracts { count: usize },
    /// The slot rearmed a host-discarded walk.
    RearmHostDiscardedWalk { token: u64 },
    /// The slot requested a recovery anchor.
    RequestRecoveryAnchor,
    /// The slot queried the live walking-operation state.
    LiveWalkingOperation,
    /// The slot recorded a manual walk takeover.
    NoteManualWalkTakeover { user_move_intent_seq: u64 },
    /// The slot queried the paint frame.
    Paint,
    /// The slot sent a paint click.
    PaintClick { id: String },
    /// The slot selected a paint entry.
    PaintSelect { key: String, name: String },
    /// The slot queried journal-paint visibility.
    JournalPaintHidden { now: Instant },
    /// The slot evaluated a probe expression.
    Probe { expr: String },
    /// The slot drained log lines.
    DrainLogs,
    /// The slot queried ignored random names.
    IgnoredRandoms,
    /// The slot queried one ignored random name.
    IgnoresRandom { name: String },
    /// The slot queried memory metrics.
    MemoryMetrics,
    /// The slot queried memory progress.
    MemoryProgress,
    /// The slot queried the seat-batch backlog through test hooks.
    SeatBatchBacklog,
    /// The slot queried the retained fold-batch count through test hooks.
    HeldFoldLen,
}

/// Shareable controls and observations for one run created by
/// [`StubSurfaceFactory`]. Each handle belongs to exactly one run.
#[derive(Clone)]
pub struct StubSurfaceHandle {
    state: Arc<StubSurfaceState>,
}

impl StubSurfaceHandle {
    /// Return the start inputs retained by this run.
    pub fn start(&self) -> SurfaceStart {
        self.state.start.clone()
    }

    /// Return the run's current execution identity.
    pub fn execution(&self) -> ExecutionId {
        *lock(&self.state.execution)
    }

    /// Return the exact recovery-hints allocation passed to this run.
    pub fn recovery_hints(&self) -> Arc<RecoveryHintsCell> {
        Arc::clone(&self.state.recovery_hints)
    }

    /// Add stamped interactions to the run's drain queue.
    pub fn queue_interacts(&self, rows: impl IntoIterator<Item = QueuedInteract>) {
        lock(&self.state.interacts).extend(rows);
    }

    /// Add native seat batches to the run's drain queue.
    pub fn queue_seat_batches(&self, batches: impl IntoIterator<Item = QueuedSeatBatch>) {
        lock(&self.state.seat_batches).extend(batches);
    }

    /// Add lifecycle interactions to the run's drain queue.
    pub fn queue_lifecycle(&self, rows: impl IntoIterator<Item = InteractReq>) {
        lock(&self.state.lifecycle).extend(rows);
    }

    /// Add tick outcomes to the run's drain queue.
    pub fn queue_tick_outcomes(&self, outcomes: impl IntoIterator<Item = TickOutcome>) {
        lock(&self.state.tick_outcomes).extend(outcomes);
    }

    /// Add lines to the run's log queue.
    pub fn queue_logs(&self, lines: impl IntoIterator<Item = String>) {
        lock(&self.state.logs).extend(lines);
    }

    /// Set the readiness returned by [`SurfaceRun::poll_ready`].
    pub fn set_readiness(&self, readiness: SurfaceReady) {
        *lock(&self.state.readiness) = readiness;
    }

    /// Set terminal state and the optional script stop receipt.
    pub fn set_terminal(&self, stopped: bool, receipt: Option<ScriptStopReceipt>) {
        self.state.stopped.store(stopped, Ordering::Release);
        *lock(&self.state.stop_receipt) = receipt;
    }

    /// Hold the real retirement receipt until [`Self::finish_stop`].
    /// Call this before the slot transfers the run to retirement.
    pub fn defer_stop_receipt(&self) {
        lock(&self.state.retirement).deferred = true;
    }

    /// Release a held retirement receipt once, preserving its captured logs.
    pub fn finish_stop(&self) -> bool {
        let pending = {
            let mut retirement = lock(&self.state.retirement);
            retirement.deferred = false;
            retirement.pending.take()
        };
        pending.is_some_and(|(done, logs)| done.send(logs).is_ok())
    }

    /// Set whether [`SurfaceRun::take_script_cut`] will report a pending cut.
    pub fn set_script_cut(&self, pending: bool) {
        self.state.script_cut.store(pending, Ordering::Release);
    }

    /// Set the result returned by [`SurfaceRun::probe`].
    pub fn set_probe_response(&self, response: Result<serde_json::Value, String>) {
        *lock(&self.state.probe_response) = response;
    }

    /// Set facts returned by [`SurfaceRun::posted_snapshot_facts`].
    pub fn set_posted_facts(&self, facts: Option<PostedSnapshotFacts>) {
        *lock(&self.state.posted_facts) = facts;
    }

    /// Return how many allocation-free fact reads the run has served.
    pub fn posted_facts_reads(&self) -> usize {
        self.state.posted_facts_reads.load(Ordering::Relaxed)
    }

    /// Set the acceptance result for delta and wake snapshot posts.
    pub fn set_snapshot_acceptance(&self, delta: bool, wake: bool) {
        self.state
            .snapshot_delta_accepts
            .store(delta, Ordering::Release);
        self.state
            .snapshot_wake_accepts
            .store(wake, Ordering::Release);
    }

    /// Set the acceptance result for channel-event posts.
    pub fn set_channel_events_acceptance(&self, accepted: bool) {
        self.state
            .channel_events_accept
            .store(accepted, Ordering::Release);
    }

    /// Set the generation returned by [`SurfaceRun::generation`].
    pub fn set_generation(&self, generation: u64) {
        self.state.generation.store(generation, Ordering::Release);
    }

    /// Set the execution-active response.
    pub fn set_execution_active(&self, active: bool) {
        self.state.execution_active.store(active, Ordering::Release);
    }

    /// Set the execution-sequence response.
    pub fn set_execution_sequence(&self, sequence: (u64, bool)) {
        *lock(&self.state.execution_sequence) = sequence;
    }

    /// Set the return values for session reset and reconnect calls.
    pub fn set_session_work_results(&self, reset: u64, reconnect: u64) {
        self.state
            .reset_session_work_result
            .store(reset, Ordering::Release);
        self.state
            .reconnect_session_work_result
            .store(reconnect, Ordering::Release);
    }

    /// Set whether a walking operation is active.
    pub fn set_live_walking_operation(&self, active: bool) {
        self.state
            .live_walking_operation
            .store(active, Ordering::Release);
    }

    /// Return whether the run is paused.
    pub fn is_paused(&self) -> bool {
        self.state.paused.load(Ordering::Acquire)
    }

    /// Set the paint frame returned by [`SurfaceRun::paint`].
    pub fn set_paint(&self, paint: Option<Arc<ScriptPaint>>) {
        *lock(&self.state.paint) = paint;
    }

    /// Set the result returned by [`SurfaceRun::journal_paint_hidden`].
    pub fn set_journal_paint_hidden(&self, hidden: bool) {
        self.state
            .journal_paint_hidden
            .store(hidden, Ordering::Release);
    }

    /// Replace the queued log lines used by `stop`'s retirement completion.
    pub fn set_retirement_logs(&self, lines: impl IntoIterator<Item = String>) {
        *lock(&self.state.retirement_logs) = lines.into_iter().collect();
    }

    /// Replace the names returned by [`SurfaceRun::ignored_randoms`].
    pub fn set_ignored_randoms(&self, names: Vec<String>) {
        *lock(&self.state.ignored_randoms) = names;
    }

    /// Set the optional result returned by [`SurfaceRun::memory_metrics`].
    pub fn set_memory_metrics(&self, metrics: Option<serde_json::Value>) {
        *lock(&self.state.memory_metrics) = metrics;
    }

    /// Set the value returned by [`SurfaceRun::memory_progress`].
    pub fn set_memory_progress(&self, progress: serde_json::Value) {
        *lock(&self.state.memory_progress) = progress;
    }

    /// Return whether the slot called `stop` on this run.
    pub fn stop_called(&self) -> bool {
        self.state.stop_called.load(Ordering::Acquire)
    }

    /// Return the run's stopped response without recording a surface call.
    pub fn is_stopped(&self) -> bool {
        self.state.stopped.load(Ordering::Acquire)
    }

    /// Return a snapshot of the operations recorded for this run.
    pub fn operations(&self) -> Vec<StubSurfaceOperation> {
        lock(&self.state.operations).clone()
    }

    /// Drain and return the operations recorded for this run.
    pub fn take_operations(&self) -> Vec<StubSurfaceOperation> {
        std::mem::take(&mut *lock(&self.state.operations))
    }

    /// Return the manual-movement sequence most recently passed to the run.
    pub fn manual_walk_intent_sequence(&self) -> u64 {
        self.state
            .manual_walk_intent_sequence
            .load(Ordering::Acquire)
    }
}

/// A configurable `SurfaceRun` used by core slot tests.
pub struct StubSurfaceRun {
    state: Arc<StubSurfaceState>,
}

impl StubSurfaceRun {
    /// Create a run that retains the supplied start, execution, and exact
    /// recovery-hints allocation.
    pub fn new(
        start: SurfaceStart,
        execution: ExecutionId,
        recovery_hints: Arc<RecoveryHintsCell>,
    ) -> Self {
        Self {
            state: Arc::new(StubSurfaceState::new(start, execution, recovery_hints)),
        }
    }

    /// Return a shareable handle to this run's queues and responses.
    pub fn handle(&self) -> StubSurfaceHandle {
        StubSurfaceHandle {
            state: Arc::clone(&self.state),
        }
    }
}

impl SurfaceRun for StubSurfaceRun {
    fn poll_ready(&self) -> SurfaceReady {
        record(&self.state, StubSurfaceOperation::PollReady);
        lock(&self.state.readiness).clone()
    }

    fn generation(&self) -> u64 {
        record(&self.state, StubSurfaceOperation::Generation);
        self.state.generation.load(Ordering::Acquire)
    }

    fn execution_active(&self) -> bool {
        record(&self.state, StubSurfaceOperation::ExecutionActive);
        self.state.execution_active.load(Ordering::Acquire)
    }

    fn execution_sequence(&self) -> (u64, bool) {
        record(&self.state, StubSurfaceOperation::ExecutionSequence);
        *lock(&self.state.execution_sequence)
    }

    fn align_execution(&self, execution: ExecutionId) {
        record(&self.state, StubSurfaceOperation::AlignExecution(execution));
        *lock(&self.state.execution) = execution;
    }

    fn reset_session_work(&mut self, execution: Option<ExecutionId>) -> u64 {
        record(
            &self.state,
            StubSurfaceOperation::ResetSessionWork(execution),
        );
        *lock(&self.state.posted_facts) = None;
        self.state.reset_session_work_result.load(Ordering::Acquire)
    }

    fn reconnect_session_work(&mut self, execution: Option<ExecutionId>) -> u64 {
        record(
            &self.state,
            StubSurfaceOperation::ReconnectSessionWork(execution),
        );
        *lock(&self.state.posted_facts) = None;
        self.state
            .reconnect_session_work_result
            .load(Ordering::Acquire)
    }

    fn pause(&self) {
        record(&self.state, StubSurfaceOperation::Pause);
        self.state.paused.store(true, Ordering::Release);
    }

    fn resume(&self) {
        record(&self.state, StubSurfaceOperation::Resume);
        self.state.paused.store(false, Ordering::Release);
    }

    fn stop(self: Box<Self>, done: Sender<Vec<String>>) {
        record(&self.state, StubSurfaceOperation::Stop);
        self.state.stop_called.store(true, Ordering::Release);
        self.state.stopped.store(true, Ordering::Release);
        let retirement_logs = std::mem::take(&mut *lock(&self.state.retirement_logs));
        let mut retirement = lock(&self.state.retirement);
        if retirement.deferred {
            retirement.pending = Some((done, retirement_logs));
        } else {
            drop(retirement);
            let _ = done.send(retirement_logs);
        }
    }

    fn stopped(&self) -> bool {
        record(&self.state, StubSurfaceOperation::Stopped);
        self.state.stopped.load(Ordering::Acquire)
    }

    fn script_stop_receipt(&self) -> Option<ScriptStopReceipt> {
        record(&self.state, StubSurfaceOperation::ScriptStopReceipt);
        lock(&self.state.stop_receipt).clone()
    }

    fn take_script_cut(&self) -> bool {
        record(&self.state, StubSurfaceOperation::TakeScriptCut);
        self.state.script_cut.swap(false, Ordering::AcqRel)
    }

    fn post_snapshot_delta(
        &mut self,
        input: &SnapshotInput<'_>,
        _native: NativeFactsInput<'_>,
        force_banks: bool,
    ) -> bool {
        record(
            &self.state,
            StubSurfaceOperation::PostSnapshotDelta {
                tick: input.tick,
                force_banks,
            },
        );
        let accepted = self.state.snapshot_delta_accepts.load(Ordering::Acquire);
        if !accepted {
            *lock(&self.state.posted_facts) = None;
        }
        accepted
    }

    fn post_snapshot_wake(
        &mut self,
        input: &SnapshotInput<'_>,
        _native: NativeFactsInput<'_>,
        force_banks: bool,
        preserve_inv: bool,
    ) -> bool {
        record(
            &self.state,
            StubSurfaceOperation::PostSnapshotWake {
                tick: input.tick,
                force_banks,
                preserve_inv,
            },
        );
        let accepted = self.state.snapshot_wake_accepts.load(Ordering::Acquire);
        if !accepted {
            *lock(&self.state.posted_facts) = None;
        }
        accepted
    }

    fn posted_snapshot_facts(&self) -> Option<PostedSnapshotFacts> {
        self.state
            .posted_facts_reads
            .fetch_add(1, Ordering::Relaxed);
        *lock(&self.state.posted_facts)
    }

    fn post_channel_events(&self, events: &[InteractReq]) -> bool {
        record(
            &self.state,
            StubSurfaceOperation::PostChannelEvents(events.to_vec()),
        );
        self.state.channel_events_accept.load(Ordering::Acquire)
    }

    fn post_settings_bag(&self, bag: &SettingsBag) {
        record(
            &self.state,
            StubSurfaceOperation::PostSettingsBag(bag.clone()),
        );
    }

    fn post_loadouts(&self, loadouts: &[api::loadout::Loadout]) {
        record(
            &self.state,
            StubSurfaceOperation::PostLoadouts(loadouts.to_vec()),
        );
    }

    fn on_game_tick_at(&self, snap_tick: u64, input_identity: u64) {
        record(
            &self.state,
            StubSurfaceOperation::OnGameTickAt {
                tick: snap_tick,
                input_identity,
            },
        );
    }

    fn on_snapshot_change_at(&self, snap_tick: u64, input_identity: u64) {
        record(
            &self.state,
            StubSurfaceOperation::OnSnapshotChangeAt {
                tick: snap_tick,
                input_identity,
            },
        );
    }

    fn drain_interacts_stamped(&self) -> Vec<QueuedInteract> {
        record(&self.state, StubSurfaceOperation::DrainInteractsStamped);
        std::mem::take(&mut *lock(&self.state.interacts))
    }

    fn restore_interacts_stamped(&self, mut drained: Vec<QueuedInteract>) {
        let count = drained.len();
        record(
            &self.state,
            StubSurfaceOperation::RestoreInteractsStamped { count },
        );
        let mut queued = lock(&self.state.interacts);
        drained.append(&mut queued);
        *queued = drained;
    }

    fn drain_seat_batches(&self) -> Vec<QueuedSeatBatch> {
        record(&self.state, StubSurfaceOperation::DrainSeatBatches);
        std::mem::take(&mut *lock(&self.state.seat_batches))
    }

    fn restore_seat_batches(&self, mut drained: Vec<QueuedSeatBatch>) {
        let count = drained.len();
        record(
            &self.state,
            StubSurfaceOperation::RestoreSeatBatches { count },
        );
        let mut queued = lock(&self.state.seat_batches);
        drained.append(&mut queued);
        *queued = drained;
    }

    fn drain_lifecycle(&self) -> Vec<InteractReq> {
        record(&self.state, StubSurfaceOperation::DrainLifecycle);
        std::mem::take(&mut *lock(&self.state.lifecycle))
    }

    fn drain_tick_outcomes(&self) -> Vec<TickOutcome> {
        record(&self.state, StubSurfaceOperation::DrainTickOutcomes);
        std::mem::take(&mut *lock(&self.state.tick_outcomes))
    }

    fn discard_mouse_interacts(&self) {
        let mut interacts = lock(&self.state.interacts);
        let old_len = interacts.len();
        interacts.retain(|row| !matches!(row.req, InteractReq::Mouse { .. }));
        let discarded = old_len - interacts.len();
        drop(interacts);
        record(
            &self.state,
            StubSurfaceOperation::DiscardMouseInteracts { count: discarded },
        );
    }

    fn rearm_host_discarded_walk(&self, token: u64) {
        record(
            &self.state,
            StubSurfaceOperation::RearmHostDiscardedWalk { token },
        );
    }

    fn request_recovery_anchor(&self) {
        record(&self.state, StubSurfaceOperation::RequestRecoveryAnchor);
    }

    fn live_walking_operation(&self) -> bool {
        record(&self.state, StubSurfaceOperation::LiveWalkingOperation);
        self.state.live_walking_operation.load(Ordering::Acquire)
    }

    fn note_manual_walk_takeover(&self, user_move_intent_seq: u64) {
        record(
            &self.state,
            StubSurfaceOperation::NoteManualWalkTakeover {
                user_move_intent_seq,
            },
        );
        self.state
            .manual_walk_intent_sequence
            .store(user_move_intent_seq, Ordering::Release);
    }

    fn paint(&self) -> Option<Arc<ScriptPaint>> {
        record(&self.state, StubSurfaceOperation::Paint);
        lock(&self.state.paint).as_ref().map(Arc::clone)
    }

    fn paint_click(&self, id: &str) {
        record(
            &self.state,
            StubSurfaceOperation::PaintClick { id: id.to_owned() },
        );
    }

    fn paint_select(&self, key: &str, name: &str) {
        record(
            &self.state,
            StubSurfaceOperation::PaintSelect {
                key: key.to_owned(),
                name: name.to_owned(),
            },
        );
    }

    fn journal_paint_hidden(&self, now: Instant) -> bool {
        record(
            &self.state,
            StubSurfaceOperation::JournalPaintHidden { now },
        );
        self.state.journal_paint_hidden.load(Ordering::Acquire)
    }

    fn probe(&self, expr: &str) -> Result<serde_json::Value, SurfaceError> {
        record(
            &self.state,
            StubSurfaceOperation::Probe {
                expr: expr.to_owned(),
            },
        );
        match lock(&self.state.probe_response).clone() {
            Ok(value) => Ok(value),
            Err(message) => Err(SurfaceError::Probe(message)),
        }
    }

    fn drain_logs(&self) -> Vec<String> {
        record(&self.state, StubSurfaceOperation::DrainLogs);
        std::mem::take(&mut *lock(&self.state.logs))
    }

    fn ignored_randoms(&self) -> Vec<String> {
        record(&self.state, StubSurfaceOperation::IgnoredRandoms);
        lock(&self.state.ignored_randoms).clone()
    }

    fn ignores_random(&self, name: &str) -> bool {
        record(
            &self.state,
            StubSurfaceOperation::IgnoresRandom {
                name: name.to_owned(),
            },
        );
        let folded_name = name.to_lowercase();
        lock(&self.state.ignored_randoms)
            .iter()
            .any(|ignored| ignored.to_lowercase() == folded_name)
    }

    fn memory_metrics(&self) -> Option<serde_json::Value> {
        record(&self.state, StubSurfaceOperation::MemoryMetrics);
        lock(&self.state.memory_metrics).clone()
    }

    fn memory_progress(&self) -> serde_json::Value {
        record(&self.state, StubSurfaceOperation::MemoryProgress);
        lock(&self.state.memory_progress).clone()
    }

    #[cfg(feature = "test-hooks")]
    fn seat_batch_backlog(&self) -> usize {
        let backlog = lock(&self.state.seat_batches).len();
        record(&self.state, StubSurfaceOperation::SeatBatchBacklog);
        backlog
    }

    #[cfg(feature = "test-hooks")]
    fn held_fold_len(&self) -> usize {
        let retained = lock(&self.state.seat_batches)
            .iter()
            .filter(|batch| batch.held_fold)
            .count();
        record(&self.state, StubSurfaceOperation::HeldFoldLen);
        retained
    }

    #[cfg(any(test, feature = "test-hooks"))]
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

/// A factory that creates a distinct stub run for every successful spawn.
/// It retains no recovery-hints cell. Each run keeps the exact cell supplied
/// by its caller, so slot ownership and restart identity remain observable.
#[derive(Default)]
pub struct StubSurfaceFactory {
    runs: Mutex<Vec<StubSurfaceHandle>>,
    next_spawn_errors: Mutex<VecDeque<String>>,
    spawn_attempts: AtomicUsize,
}

impl StubSurfaceFactory {
    /// Create an empty stub factory.
    pub fn new() -> Self {
        Self::default()
    }

    /// Make the next spawn fail with `message`.
    pub fn fail_next_spawn(&self, message: impl Into<String>) {
        lock(&self.next_spawn_errors).push_back(message.into());
    }

    /// Return the number of spawn attempts, including failed attempts.
    pub fn spawn_attempts(&self) -> usize {
        self.spawn_attempts.load(Ordering::Acquire)
    }

    /// Return handles for every successful run, in spawn order.
    pub fn runs(&self) -> Vec<StubSurfaceHandle> {
        lock(&self.runs).clone()
    }

    /// Return the successful run at `index`, if one exists.
    pub fn run(&self, index: usize) -> Option<StubSurfaceHandle> {
        lock(&self.runs).get(index).cloned()
    }

    /// Return the most recently spawned run, if one exists.
    pub fn last_run(&self) -> Option<StubSurfaceHandle> {
        lock(&self.runs).last().cloned()
    }
}

impl SurfaceFactory for StubSurfaceFactory {
    fn kind(&self) -> SurfaceKind {
        SurfaceKind::Js
    }

    fn spawn(
        &self,
        start: &SurfaceStart,
        execution: ExecutionId,
        recovery_hints: Arc<RecoveryHintsCell>,
    ) -> Result<Box<dyn SurfaceRun>, SurfaceError> {
        self.spawn_attempts.fetch_add(1, Ordering::Relaxed);
        if start.card.kind() != self.kind() {
            return Err(SurfaceError::ForeignCard(self.kind()));
        }
        if let Some(message) = lock(&self.next_spawn_errors).pop_front() {
            return Err(SurfaceError::Spawn(message));
        }
        let run = StubSurfaceRun::new(start.clone(), execution, recovery_hints);
        lock(&self.runs).push(run.handle());
        Ok(Box::new(run))
    }
}

#[derive(Default)]
struct StubRetirement {
    deferred: bool,
    pending: Option<(Sender<Vec<String>>, Vec<String>)>,
}

struct StubSurfaceState {
    start: SurfaceStart,
    execution: Mutex<ExecutionId>,
    recovery_hints: Arc<RecoveryHintsCell>,
    readiness: Mutex<SurfaceReady>,
    generation: AtomicU64,
    execution_active: AtomicBool,
    execution_sequence: Mutex<(u64, bool)>,
    reset_session_work_result: AtomicU64,
    reconnect_session_work_result: AtomicU64,
    paused: AtomicBool,
    stopped: AtomicBool,
    stop_called: AtomicBool,
    stop_receipt: Mutex<Option<ScriptStopReceipt>>,
    script_cut: AtomicBool,
    snapshot_delta_accepts: AtomicBool,
    snapshot_wake_accepts: AtomicBool,
    posted_facts: Mutex<Option<PostedSnapshotFacts>>,
    posted_facts_reads: AtomicUsize,
    channel_events_accept: AtomicBool,
    interacts: Mutex<Vec<QueuedInteract>>,
    seat_batches: Mutex<Vec<QueuedSeatBatch>>,
    lifecycle: Mutex<Vec<InteractReq>>,
    tick_outcomes: Mutex<Vec<TickOutcome>>,
    logs: Mutex<Vec<String>>,
    retirement_logs: Mutex<Vec<String>>,
    retirement: Mutex<StubRetirement>,
    live_walking_operation: AtomicBool,
    manual_walk_intent_sequence: AtomicU64,
    paint: Mutex<Option<Arc<ScriptPaint>>>,
    journal_paint_hidden: AtomicBool,
    probe_response: Mutex<Result<serde_json::Value, String>>,
    ignored_randoms: Mutex<Vec<String>>,
    memory_metrics: Mutex<Option<serde_json::Value>>,
    memory_progress: Mutex<serde_json::Value>,
    operations: Mutex<Vec<StubSurfaceOperation>>,
}

impl StubSurfaceState {
    fn new(
        start: SurfaceStart,
        execution: ExecutionId,
        recovery_hints: Arc<RecoveryHintsCell>,
    ) -> Self {
        Self {
            start,
            execution: Mutex::new(execution),
            recovery_hints,
            readiness: Mutex::new(SurfaceReady::Pending),
            generation: AtomicU64::new(0),
            execution_active: AtomicBool::new(false),
            execution_sequence: Mutex::new((0, false)),
            reset_session_work_result: AtomicU64::new(0),
            reconnect_session_work_result: AtomicU64::new(0),
            paused: AtomicBool::new(false),
            stopped: AtomicBool::new(false),
            stop_called: AtomicBool::new(false),
            stop_receipt: Mutex::new(None),
            script_cut: AtomicBool::new(false),
            snapshot_delta_accepts: AtomicBool::new(true),
            snapshot_wake_accepts: AtomicBool::new(true),
            posted_facts: Mutex::new(None),
            posted_facts_reads: AtomicUsize::new(0),
            channel_events_accept: AtomicBool::new(true),
            interacts: Mutex::new(Vec::new()),
            seat_batches: Mutex::new(Vec::new()),
            lifecycle: Mutex::new(Vec::new()),
            tick_outcomes: Mutex::new(Vec::new()),
            logs: Mutex::new(Vec::new()),
            retirement_logs: Mutex::new(Vec::new()),
            retirement: Mutex::new(StubRetirement::default()),
            live_walking_operation: AtomicBool::new(false),
            manual_walk_intent_sequence: AtomicU64::new(0),
            paint: Mutex::new(None),
            journal_paint_hidden: AtomicBool::new(false),
            probe_response: Mutex::new(Ok(serde_json::Value::Null)),
            ignored_randoms: Mutex::new(Vec::new()),
            memory_metrics: Mutex::new(None),
            memory_progress: Mutex::new(serde_json::json!({})),
            operations: Mutex::new(Vec::new()),
        }
    }
}

fn record(state: &StubSurfaceState, operation: StubSurfaceOperation) {
    lock(&state.operations).push(operation);
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
