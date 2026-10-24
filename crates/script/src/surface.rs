use crate::interact::{InteractReq, QueuedInteract};
use crate::native::seat_rows::QueuedSeatBatch;
use crate::native::snapshot_source::{BankSelectionInput, NativeFactsInput, SnapshotInput};
use crate::native::{recovery_hints::RecoveryHintsCell, SettingsBag};
use crate::paint::ScriptPaint;
use api::identity::ExecutionId;
use std::{any::Any, sync::{mpsc::Sender, Arc}, time::Instant};

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

/// One running surface. The slot owns lifecycle transitions, and `stop`
/// transfers the run to its retirement worker.
pub trait SurfaceRun: Send {
    // setup, identity, session
    /// Return the current setup state.
    fn poll_ready(&self) -> SurfaceReady;
    /// Return the run generation.
    fn generation(&self) -> u64;
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
