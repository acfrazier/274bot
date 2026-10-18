//! Per-uid script runner. `SlotScript` owns at most one compiled `Script`
//! and gates it on operator intent (`want_run`) and client presence
//! (`on_is_up`). `tick` runs on the caller's pump at a game-tick edge and
//! must return; panics are caught, never abort the process.

#[cfg(feature = "load")]
#[path = "slot/api.rs"]
mod api_seat;
#[cfg(feature = "load")]
#[path = "slot/bank.rs"]
mod bank_seat;
pub(crate) mod compiled;
mod pending;
use crate::native::{Interrupt, RetainedMemory, ScriptFailure, ScriptFlow, StopReason};
pub use compiled::{prepare_config, CompiledDelivery};
use compiled::{CompiledRun, Preparation};

pub use pending::{PendingBankOp, PendingBankOpKind, PendingWithdrawX, PendingWithdrawXPhase};

use crate::native::seat::{Admission, EmissionSource, GameplaySeat};
use api::identity::{ExecutionId, Fence, RequestId, RequestIdMinter};
use api::script_contract::RequestStamp;
use std::collections::VecDeque;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::ctx::ScriptCtx;
#[cfg(feature = "load")]
use crate::isolate_fb::{DecodedSeatBatch, IsolateBuf, SeatStartArgs, SnapshotFingerprint};
#[cfg(feature = "load")]
use crate::load::{GameplayFactory, LoadIsolate, LoadShape, Ready};
use crate::watchdog::{ProgressWatchdog, Tile as WatchdogTile, WatchdogAction};
use api::native_input::NativeInputAuthority;
use api::random::{DetectedRandom, RandomClaim};
use serde::Serialize;

// The seat's host ids occupy the high host range. Other process services
// currently mint from HOST_FIRST; no separate seat instance can reuse an id.
static SEAT_REQUEST_IDS: RequestIdMinter = match RequestIdMinter::starting_at(1 << 63) {
    Ok(ids) => ids,
    Err(_) => panic!("host request namespace is too small"),
};

enum SeatEffect {
    Native(crate::native::HostAction),
    #[cfg(feature = "load")]
    Interact(crate::load::QueuedInteract),
}

/// Read-only, allocation-free counters for this slot's armed gameplay seat.
/// Late controls are expected duplicates, not user-visible failures.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SlotMetrics {
    pub dropped: u64,
    pub late_controls: u64,
}

/// Test-support read-only receipt for the real slot-owned seat.
#[cfg(feature = "test-hooks")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeatProbe {
    pub held: bool,
    pub dropped: u64,
    pub late_controls: u64,
    pub live: usize,
    pub results: usize,
    pub reserved_releases: usize,
    pub mirror_revokes_consumed: u64,
}

/// The host outcome id uses the producer's namespace. Native ledger walks
/// reserve a distinct seat id from Load walks with the same numeric id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeatWalkTerminal {
    Native(RequestId),
    Load(RequestId),
}
/// Native ledger request tokens start at one and are scoped to the current
/// run. Keep their seat release handles in the lower half of the host range;
/// the slot's compiled owner minter starts in the upper half.
fn native_walk_id(raw: u64) -> Option<RequestId> {
    (raw > 0 && raw < RequestId::HOST_FIRST)
        .then(|| RequestId::from_raw(RequestId::HOST_FIRST + raw))
        .flatten()
}

fn seat_walk_id(req: &crate::shim::InteractReq, owner: Option<RequestId>) -> Option<RequestId> {
    match req {
        crate::shim::InteractReq::Walk { request_id, .. }
        | crate::shim::InteractReq::WalkNear { request_id, .. } => {
            RequestId::from_raw(*request_id).or(owner)
        }
        // WalkTo is one immediate click, and WalkNearestBank has no
        // correlated AbortWalk handle to reserve.
        _ => None,
    }
}

/// Most lane rows, plus one row for each replacement key, that one guardian
/// hold carries before the fold refuses a later `ChannelPost`: the
/// gameplay/lifecycle capacity of one seat take (§28.5 line 179). The fold
/// refuses no other row. Resume pages the folded batch into the seat
/// (`SlotScript::consume_held_fold`), so `GameplaySeat::take` drops none of
/// its rows, however many opens, closes and inspections it carries.
#[cfg(feature = "load")]
const HELD_LIFECYCLE_ROWS: usize = api::script_contract::MAX_ENCODED_GAMEPLAY;

/// Lifecycle rows of which the host applies only the latest value. The fold
/// keeps one row for each key, the newest, and appends these rows after the
/// ordered lane.
#[cfg(feature = "load")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HeldReplacement {
    /// `script_interact.rs` writes the last `SetCameraYaw` of a drain.
    CameraYaw,
    /// `feed_watchdog` keeps the last anchor reply of a drain.
    RecoveryAnchor,
    /// The host keeps the highest acknowledged inspect sequence. The
    /// isolate acknowledges only an advanced sequence, so the last is highest.
    InspectAck,
    /// The host ignores an isolate-sent `ChannelMessage`.
    ChannelMessage,
    /// The host ignores an isolate-sent `ChannelStatus`.
    ChannelStatus,
    /// Stamps both watchdog clocks with the host frame time.
    NoteProgress,
    /// Stamps the scheduler clock with the host frame time.
    LoopSettled,
}

#[cfg(feature = "load")]
impl HeldReplacement {
    const KEYS: usize = 7;

    fn of(req: &crate::shim::InteractReq) -> Option<Self> {
        use crate::shim::InteractReq;
        Some(match req {
            InteractReq::SetCameraYaw { .. } => Self::CameraYaw,
            InteractReq::RecoveryAnchor { .. } | InteractReq::RecoveryAnchorNone => {
                Self::RecoveryAnchor
            }
            InteractReq::InspectAck { .. } => Self::InspectAck,
            InteractReq::ChannelMessage { .. } => Self::ChannelMessage,
            InteractReq::ChannelStatus { .. } => Self::ChannelStatus,
            InteractReq::NoteProgress => Self::NoteProgress,
            InteractReq::LoopSettled => Self::LoopSettled,
            _ => return None,
        })
    }
}

/// Status sent to a script whose BroadcastChannel post the hold refused.
#[cfg(feature = "load")]
const HELD_CHANNEL_POST_REFUSED: &str = "BroadcastChannel post dropped while the session was held";

fn seat_interact_class(
    req: &crate::shim::InteractReq,
    owner: Option<RequestId>,
) -> (
    api::script_contract::EmissionClass,
    Option<api::script_contract::Release>,
) {
    use api::script_contract::{EmissionClass, Release};
    match req {
        crate::shim::InteractReq::AbortWalk { request_id } => (
            EmissionClass::Release,
            RequestId::from_raw(*request_id)
                .or(owner)
                .map(Release::AbortWalk),
        ),
        crate::shim::InteractReq::GatherStop { .. } => {
            (EmissionClass::Release, Some(Release::GatherStop))
        }
        crate::shim::InteractReq::CombatStop { .. } => {
            (EmissionClass::Release, Some(Release::CombatStop))
        }
        _ if req.is_game() => (EmissionClass::Gameplay, None),
        _ => (EmissionClass::Lifecycle, None),
    }
}

/// Rows that `GameplaySeat::take` moves before its 141 other rows: the
/// releases and the one run-policy control. A held tick of a scene load, of
/// the welcome modal or of a watchdog recovery keeps these rows (operator
/// decision 2026-10-16: aborts are never dropped, and the newest run policy
/// is delivered after the hold).
fn seat_reserved_row(req: &crate::shim::InteractReq) -> bool {
    matches!(
        req,
        crate::shim::InteractReq::AbortWalk { .. }
            | crate::shim::InteractReq::GatherStop { .. }
            | crate::shim::InteractReq::CombatStop { .. }
            | crate::shim::InteractReq::RunPolicyOverride { .. }
    )
}

/// A second guardian hold folds over a retained batch whose pages still
/// wait, and whose controls the seat has not applied. Keep those controls
/// ahead of the newer batch's controls, in order, so each control class
/// applies in the order of the waiting and then the newer controls. The isolate repeats
/// every unacknowledged control in each batch, and keeps one control for
/// each key, so a newer control with the key of a kept control is that
/// control's repeat. Applied after it, the repeat is a duplicate or a late
/// control, so it is carried once:
/// - a start by its id (`Host::carry_typed_start` replaces the arguments of
///   a pending start, and ids are never reused),
/// - a decision by its request and prompt sequence,
/// - a cancel by its target (`Host::queue_cancel` raises the reason of a
///   pending cancel),
/// - a mirror revoke by its owner (`validate_mirror_revokes` refuses one
///   owner twice),
/// - a terminal machine event by its id and kind.
///
/// A machine phase is the current value of its id, which the isolate
/// replaces (`Host::queue_machine_event`). It repeats only the last kept
/// phase of its id. A phase that returns to an earlier value is a new
/// transition and is kept. The watermark is the higher of the two, which
/// the seat's monotone watermark gives when it applies both in order. Rows
/// fold as for every held batch.
///
/// Accepted limit (R8-M1, T13B-R10, T13B-MERGE): the order is kept inside
/// each control class, not between classes. `apply_seat_batch` applies
/// every start before every machine event. If one fold holds a terminal
/// event of request A from an older batch and a new start of request B from
/// a newer batch, resume tries B before A ends. The seat then refuses a
/// Session B as busy, and the script sees `Refused(Busy)` for B. Serial
/// apply would end A first and admit B. The fold does not change.
///
/// The sequence is reachable through a callback start, but it is rare. It
/// needs one guardian hold and both conditions below
/// (`TRACK-T1/T1.3b/review-sol-final/review.md`, "Decision 3"):
/// - The isolate emits the terminal of A before the guardian hold, and the
///   host does not drain that batch before the hold starts. While the
///   guardian holds, the seat does not complete or start a native:
///   `GameplaySeat::complete` and `GameplaySeat::start` return early
///   (`native/seat.rs:790-793`, `:1016-1018`). The isolate's machines do not
///   step (`machine::pass` returns while the host is held), so no native
///   gets a new terminal during the hold. The test
///   `guardian_hold_neither_completes_nor_starts_a_native` asserts this.
/// - An `onPaint` or event callback starts B during the hold. The script's
///   `loop()` does not run during the hold (`load/isolate/thread.rs:1709`).
///
/// A fold from two guardian holds does not occur: one player has at most
/// one active random event (`macro_events.rs2:73-75`), and the next general
/// event waits for the 500-tick event timer (`login.rs2:40`,
/// `macro_events.rs2:2-12`). So a second guardian hold does not start while
/// the pages of the first fold wait.
#[cfg(feature = "load")]
fn carry_waiting_controls(
    waiting: &mut crate::load::QueuedSeatBatch,
    newer: &mut crate::load::QueuedSeatBatch,
) {
    use crate::machine::{MachineEvent, MachineEventKind};
    fn ahead<T>(newer: &mut Vec<T>, waiting: &mut Vec<T>, repeats: impl Fn(&[T], &T) -> bool) {
        let later = std::mem::replace(newer, std::mem::take(waiting));
        for control in later {
            if !repeats(newer, &control) {
                newer.push(control);
            }
        }
    }
    fn event_repeats(kept: &[MachineEvent], event: &MachineEvent) -> bool {
        match event.kind {
            MachineEventKind::Phase(_) => kept
                .iter()
                .rev()
                .find(|kept| kept.id == event.id && matches!(kept.kind, MachineEventKind::Phase(_)))
                .is_some_and(|kept| kept.kind == event.kind),
            kind => kept
                .iter()
                .any(|kept| kept.id == event.id && kept.kind == kind),
        }
    }
    newer.batch.consumed_terminal_seq = newer
        .batch
        .consumed_terminal_seq
        .max(waiting.batch.consumed_terminal_seq);
    ahead(
        &mut newer.batch.starts,
        &mut waiting.batch.starts,
        |kept, start| kept.iter().any(|kept| kept.id == start.id),
    );
    ahead(
        &mut newer.batch.decisions,
        &mut waiting.batch.decisions,
        |kept, decision| {
            kept.iter()
                .any(|kept| kept.of == decision.of && kept.sequence == decision.sequence)
        },
    );
    ahead(
        &mut newer.batch.cancels,
        &mut waiting.batch.cancels,
        |kept, cancel| kept.iter().any(|kept| kept.of == cancel.of),
    );
    ahead(
        &mut newer.batch.mirror_revokes,
        &mut waiting.batch.mirror_revokes,
        |kept, revoke| kept.iter().any(|kept| kept.owner == revoke.owner),
    );
    ahead(
        &mut newer.machine_events,
        &mut waiting.machine_events,
        event_repeats,
    );
}

#[cfg(feature = "load")]
fn seat_live_details<'a>(
    seat: &'a GameplaySeat<SeatEffect>,
    api: Option<&api_seat::ApiSeat>,
    compiled: Option<&CompiledRun>,
    compiled_owner: Option<RequestId>,
) -> [crate::isolate_fb::SeatLiveDetails<'a>; api::script_contract::MAX_LIVE] {
    use crate::isolate_fb::{SeatLiveDetails, SeatPromptInput};
    use crate::native::NativePhase;
    let unused_id = seat.live().first().map_or_else(
        || RequestId::isolate(1).expect("isolate namespace"),
        |entry| entry.id,
    );
    let mut details = [SeatLiveDetails {
        id: unused_id,
        phase: NativePhase::Working,
        prompt: None,
    }; api::script_contract::MAX_LIVE];
    for (detail, entry) in details.iter_mut().zip(seat.live()) {
        detail.id = entry.id;
        let prompt = seat.prompt_payload(entry.id);
        detail.prompt = prompt.map(|prompt| SeatPromptInput {
            sequence: prompt.seq,
            point: prompt.point.member(),
            args: &prompt.args,
        });
        detail.phase = if prompt.is_some() {
            NativePhase::Waiting
        } else {
            seat.machine_phase(entry.id)
                .or_else(|| api.and_then(|api| api.phase_for(entry.id)))
                .or_else(|| {
                    (compiled_owner == Some(entry.id)).then(|| {
                        compiled
                            .and_then(|run| run.output.status.as_ref().map(|status| status.phase))
                            .unwrap_or(NativePhase::Working)
                    })
                })
                .unwrap_or(NativePhase::Working)
        };
    }
    details
}

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
    pub loadouts: Arc<[api::loadout::Loadout]>,
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
    /// One bounded native Start retained across temporary Full admission.
    pending_compiled_start: Option<compiled::PendingCompiledStart>,
    /// A watchdog restart awaiting capacity keeps its current instance until
    /// the replacement job is accepted on a later slot turn.
    pending_compiled_restart: bool,
    /// One native owner for compiled cards and Load; the payload enum only
    /// selects an existing downstream dispatcher, never a second scheduler.
    seat: Option<Box<GameplaySeat<SeatEffect>>>,
    seat_compiled_owner: Option<RequestId>,
    seat_fence: Fence<ExecutionId>,
    seat_delivery: Vec<SeatEffect>,
    seat_native_delivery: VecDeque<crate::native::HostAction>,
    #[cfg(feature = "load")]
    seat_interact_delivery: Vec<crate::load::QueuedInteract>,
    /// Guardian hold is separate from the shared seat-held state.
    guardian_held: bool,
    /// The slot loop holds the frame for a reason that is not the guardian:
    /// the scene is not ready, the welcome modal is up, or a session boundary
    /// applies. That is a held tick (`28-seat-sg.md:64`), not the guardian's
    /// `SessionHeld` (`28-seat-sg.md:65`), so the seat is not frozen.
    seat_host_held: bool,
    /// Walk aborts that the script sent while the frame was held, oldest
    /// first, for the first frame after the hold (operator decision
    /// 2026-10-16). One for each walk request, at most `MAX_WALK_RELEASES`.
    #[cfg(feature = "load")]
    held_aborts: Vec<crate::load::QueuedInteract>,
    /// The newest run-policy update that the script sent while the frame was
    /// held, for the first frame after the hold. The outer option is "an
    /// update was kept". The inner option is replace versus clear.
    #[cfg(feature = "load")]
    held_policy: Option<Option<api::run_policy::RunPolicyOverride>>,
    /// A missing host session also freezes the shared seat.
    seat_session_held: bool,
    /// Physical Walk terminals deferred until guardian hold lifts.
    held_walk_terminals: Vec<RequestId>,
    /// Watchdog facts already filtered and dispatched by the owner seat,
    /// consumed by the host's lifecycle drain rather than the game driver.
    #[cfg(feature = "load")]
    seat_lifecycle: Vec<crate::shim::InteractReq>,
    /// Channels that already received the held-post refusal status in the
    /// current hold. Cleared when the folded batch is consumed after resume.
    #[cfg(feature = "load")]
    held_channel_refusals: Vec<u64>,
    #[cfg(feature = "load")]
    api: Option<Box<api_seat::ApiSeat>>,
    #[cfg(feature = "load")]
    bank: Option<Box<bank_seat::BankSeat>>,
    retained: Option<Arc<std::sync::Mutex<RetainedMemory>>>,
    native_runtime: crate::native::ledger::Runtime,
    quest_pairs: Option<Arc<dyn crate::quester::pair::QuestPairPort>>,
    /// The slot's local-state capability (T5.2), bound by the composition
    /// root beside the quest-pair port; `None` when the composition opened
    /// no state service for this slot.
    local_state: Option<Arc<crate::local_state::SlotState>>,
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
            pending_compiled_start: None,
            pending_compiled_restart: false,
            seat: None,
            seat_compiled_owner: None,
            seat_fence: Fence::new(),
            seat_delivery: Vec::new(),
            seat_native_delivery: VecDeque::new(),
            #[cfg(feature = "load")]
            seat_interact_delivery: Vec::new(),
            guardian_held: false,
            seat_host_held: false,
            #[cfg(feature = "load")]
            held_aborts: Vec::new(),
            #[cfg(feature = "load")]
            held_policy: None,
            seat_session_held: false,
            held_walk_terminals: Vec::new(),
            #[cfg(feature = "load")]
            seat_lifecycle: Vec::new(),
            #[cfg(feature = "load")]
            held_channel_refusals: Vec::new(),
            #[cfg(feature = "load")]
            api: None,
            #[cfg(feature = "load")]
            bank: None,
            retained: None,
            native_runtime: Default::default(),
            quest_pairs: None,
            local_state: None,
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

    /// Install the slot's local-state capability (T5.2): the process
    /// service plus the account and content identity this slot reads and
    /// writes under. Native cards receive it on every tick
    /// ([`crate::native::NativeTick::state`]).
    pub fn bind_local_state(&mut self, state: Arc<crate::local_state::SlotState>) {
        self.local_state = Some(state);
    }

    /// The bound local-state capability, if any.
    pub fn local_state(&self) -> Option<&Arc<crate::local_state::SlotState>> {
        self.local_state.as_ref()
    }
    /// Align the compiled execution with the actual host session gate, not
    /// `work_epoch` (which also advances for transient down observations).
    /// Called while the host holds the gate borrow across the native tick.
    pub fn align_seat_session(&mut self, ready: Option<api::identity::SessionKey>) {
        let Some(session) = ready.filter(|key| key.slot().incarnation().get() == self.incarnation)
        else {
            self.seat_session_held = true;
            if let Some(seat) = &mut self.seat {
                seat.hold(true);
            }
            return;
        };
        if let Some(run) = self.compiled.as_mut() {
            if run.run.session != session.epoch() {
                run.rekey_session(session.epoch());
            }
        }
        let rekeyed = self.seat.as_mut().and_then(|seat| {
            (seat.run().session != session.epoch()).then(|| {
                seat.reconnect(session.epoch());
                seat.run()
            })
        });
        if let Some(run) = rekeyed {
            self.discard_seat_gameplay_delivery();
            #[cfg(not(feature = "load"))]
            let _ = run;
            #[cfg(feature = "load")]
            self.align_api_session(run.session);
            #[cfg(feature = "load")]
            if let Some(isolate) = &self.load {
                // The local work epoch can have advanced on transient down
                // observations before F0 mints this session. The carrier's
                // pending Starts must use the owner's actual execution key.
                isolate.align_execution(ExecutionId::first(run));
            }
        }
        if self.seat_compiled_owner.is_none() {
            if let Some(run) = self.compiled.as_ref() {
                let key = run.run;
                let card = run.config.card();
                self.admit_compiled_owner(key, card);
            }
        }
        self.seat_session_held = false;
        if let Some(seat) = &mut self.seat {
            seat.hold(self.guardian_held);
        }
        self.flush_held_walk_terminal();
        if let Some(run) = self.seat.as_ref().map(|seat| seat.run()) {
            self.seat_fence.arm(ExecutionId::first(run));
        }
    }

    /// Guardian authority precedes every request foreground. Called even
    /// when the host suppresses the compiled tick itself.
    pub fn seat_guardian_hold(&mut self, held: bool) {
        self.guardian_held = held;
        if let Some(seat) = &mut self.seat {
            seat.hold(held || self.seat_session_held);
        }
        if held {
            self.discard_seat_gameplay_delivery();
        } else {
            self.flush_held_walk_terminal();
        }
    }

    /// The slot loop's frame hold that is not the guardian's: the scene is
    /// not ready, the welcome modal is up, or a session boundary applies.
    /// Only the guardian freezes the seat (`28-seat-sg.md:65`, §27 §6 of
    /// `27-decision-r5.md`). This hold is a held tick (`28-seat-sg.md:64`):
    /// the seat keeps applying controls, and gameplay of the held frames is
    /// dropped. The operator decision of 2026-10-16 amends that row: a held
    /// tick keeps `AbortWalk` and the newest `RunPolicyOverride` for the
    /// first frame after the hold. That frame dispatches the aborts in the
    /// order the script sent them, and the kept policy applies in the same
    /// frame ([`Self::seat_held_tick`], [`Self::keep_held_aborts`]). The
    /// drain applies a `GatherStop` or `CombatStop` at once, so the order
    /// across these kinds is not kept. A watchdog recovery hold keeps the
    /// same rows. Accepted limit (T13B-MERGE): if the host observes a scene
    /// load in one frame only, no drain runs while this flag is set. A batch
    /// that the isolate emitted before the load then keeps its gameplay, and
    /// the first frame after the load delivers it. This path did the same
    /// before T1.3b.
    pub fn seat_host_hold(&mut self, held: bool) {
        self.seat_host_held = held;
    }

    fn flush_held_walk_terminal(&mut self) {
        if self.guardian_held || self.seat_session_held {
            return;
        }
        if self.held_walk_terminals.is_empty() {
            return;
        }
        if let Some(seat) = &mut self.seat {
            for id in self.held_walk_terminals.drain(..) {
                seat.walk_terminal(id);
            }
        } else {
            self.held_walk_terminals.clear();
        }
    }

    /// Snapshot the seat's bounded counters without a test-only capability or
    /// an isolate round trip. An unarmed slot has no seat events to report.
    pub fn slot_metrics(&self) -> SlotMetrics {
        self.seat
            .as_ref()
            .map_or(SlotMetrics::default(), |seat| SlotMetrics {
                dropped: seat.dropped(),
                late_controls: seat.late_controls(),
            })
    }

    /// Read-only seat state for the ignored live and test-support cells.
    #[cfg(feature = "test-hooks")]
    pub fn seat_probe(&self) -> Option<SeatProbe> {
        let seat = self.seat.as_ref()?;
        Some(SeatProbe {
            held: seat.held(),
            dropped: seat.dropped(),
            late_controls: seat.late_controls(),
            live: seat.live().len(),
            results: seat.results().len(),
            reserved_releases: seat.reserved_releases(),
            mirror_revokes_consumed: seat.mirror_revokes_consumed(),
        })
    }
    /// The host test rig derives its independent session gate from the same
    /// incarnation bound by `script_slot_or_insert`, not from a seat result.
    #[cfg(feature = "test-hooks")]
    pub fn slot_key_for_test(&self) -> Option<api::identity::SlotKey> {
        api::identity::SlotKey::from_incarnation(self.incarnation)
    }

    /// A non-guardian held tick discards one-shot script effects. Guardian
    /// SessionHeld takes a different path: it keeps reserved releases. A
    /// held tick of a frame that the slot loop holds (the scene is not
    /// ready, the welcome modal is up) or of a watchdog recovery keeps the
    /// abort releases and the newest run policy too (operator decision
    /// 2026-10-16, [`Self::seat_host_hold`]). The seat keeps at most one
    /// release row for each reserved release slot and one policy row
    /// (`GameplaySeat::held_tick_keeping_releases`).
    pub fn seat_held_tick(&mut self) {
        let keep = self.seat_host_held || self.watchdog.holds_script_actions();
        let Some(seat) = self.seat.as_mut() else {
            return;
        };
        if seat.held() {
            return;
        }
        if keep {
            seat.held_tick_keeping_releases();
        } else {
            seat.held_tick();
        }
        let abort = |req: &crate::shim::InteractReq| keep && seat_reserved_row(req);
        let abort_action = |action: &crate::native::HostAction| {
            let crate::native::HostEffect::Interaction(req) = &action.effect else {
                return false;
            };
            abort(req)
        };
        let mut dropped = self.seat_native_delivery.len();
        self.seat_native_delivery.retain(abort_action);
        dropped -= self.seat_native_delivery.len();
        #[cfg(feature = "load")]
        {
            dropped += self.seat_interact_delivery.len() + self.compiled_interacts.len();
            self.seat_interact_delivery
                .retain(|queued| abort(&queued.req));
            let mut at = 0;
            let stamps = &mut self.compiled_interact_outcome_seqs;
            self.compiled_interacts.retain(|req| {
                let kept = abort(req);
                if kept {
                    at += 1;
                } else {
                    stamps.remove(at);
                }
                kept
            });
            dropped -= self.seat_interact_delivery.len() + self.compiled_interacts.len();
        }
        if let Some(ledger) = self.native_runtime.ledger.as_mut() {
            dropped += ledger.outbox.len();
            ledger.outbox.retain(abort_action);
            dropped -= ledger.outbox.len();
        }
        seat.count_dropped(dropped);
    }

    fn discard_seat_gameplay_delivery(&mut self) {
        let Some(seat) = &mut self.seat else {
            return;
        };
        let native_before = self.seat_native_delivery.len();
        self.seat_native_delivery.retain(|action| {
            let keep = match &action.effect {
                crate::native::HostEffect::Interaction(req) => {
                    seat_interact_class(req, None).0
                        != api::script_contract::EmissionClass::Gameplay
                }
                _ => false,
            };
            if !keep {
                let walk = match &action.effect {
                    crate::native::HostEffect::Walk(_) => native_walk_id(action.request_id.get()),
                    crate::native::HostEffect::Interaction(req) => seat_walk_id(req, None),
                    _ => None,
                };
                if let Some(id) = walk {
                    seat.discard_unarmed_walk(id);
                }
            }
            keep
        });
        let native_dropped = native_before - self.seat_native_delivery.len();
        #[cfg(feature = "load")]
        let dropped = {
            let interact_before = self.seat_interact_delivery.len();
            self.seat_interact_delivery.retain(|queued| {
                let keep = seat_interact_class(&queued.req, None).0
                    != api::script_contract::EmissionClass::Gameplay;
                if !keep {
                    if let Some(id) = seat_walk_id(&queued.req, None) {
                        seat.discard_unarmed_walk(id);
                    }
                }
                keep
            });
            native_dropped + interact_before - self.seat_interact_delivery.len()
        };
        #[cfg(not(feature = "load"))]
        let dropped = native_dropped;
        seat.count_dropped(dropped);
    }

    fn arm_seat(&mut self, run: api::selected::RunKey) {
        self.seat_fence.arm(ExecutionId::first(run));
        let mut seat = GameplaySeat::new(run);
        seat.hold(self.guardian_held || self.seat_session_held);
        self.seat = Some(Box::new(seat));
        self.held_walk_terminals.clear();
        self.seat_compiled_owner = None;
        self.seat_delivery.clear();
        self.seat_native_delivery.clear();
        #[cfg(feature = "load")]
        self.seat_interact_delivery.clear();
        #[cfg(feature = "load")]
        self.held_aborts.clear();
        #[cfg(feature = "load")]
        {
            self.held_policy = None;
        }
        #[cfg(feature = "load")]
        self.seat_lifecycle.clear();
        #[cfg(feature = "load")]
        self.held_channel_refusals.clear();
    }

    fn arm_compiled_seat(&mut self, run: api::selected::RunKey, card: crate::CompiledId) {
        if run.slot_key().is_none() {
            return;
        }
        self.arm_seat(run);
        self.admit_compiled_owner(run, card);
    }

    fn admit_compiled_owner(&mut self, run: api::selected::RunKey, card: crate::CompiledId) {
        if run.slot_key().is_none() {
            return;
        }
        let kind = match card.0 {
            "Gatherer" => crate::native::request::NativeKind::Gather,
            "Quester" => crate::native::request::NativeKind::Quester,
            #[cfg(feature = "load")]
            "Sherlock" => crate::native::request::NativeKind::Clue,
            #[cfg(any(test, feature = "test-hooks"))]
            "test" => crate::native::request::NativeKind::BankNearest,
            _ => panic!("unregistered compiled seat family: {}", card.0),
        };
        let id = SEAT_REQUEST_IDS.mint().expect("host request id exhausted");
        let stamp = RequestStamp::new(id, None, None, ExecutionId::first(run))
            .expect("compiled run is bound");
        let admitted = self
            .seat
            .as_mut()
            .expect("armed native seat")
            .start(stamp, kind, false)
            .expect("new seat has a request credit");
        assert_eq!(admitted, Admission::Admitted);
        self.seat_compiled_owner = Some(id);
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
    /// at Load) with no operator loadouts posted. Same state gating as
    /// [`SlotScript::start_compiled`]. The composition root posts the
    /// operator's rows through [`SlotScript::start_load_with_loadouts`]
    /// from its local-state lease; nothing here opens a store.
    #[cfg(feature = "load")]
    pub fn start_load(
        &mut self,
        source: String,
        shape: LoadShape,
        siblings: Vec<(String, String)>,
    ) -> Result<(), String> {
        self.start_load_with_loadouts(source, shape, siblings, &[])
    }

    /// Start with explicit loadouts, avoiding operator filesystem inputs in
    /// disposable live fixtures. Commands are posted before the first tick.
    #[cfg(feature = "load")]
    pub fn start_load_with_loadouts(
        &mut self,
        source: String,
        shape: LoadShape,
        siblings: Vec<(String, String)>,
        loadouts: &[api::loadout::Loadout],
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
        loadouts: &[api::loadout::Loadout],
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
        loadouts: &[api::loadout::Loadout],
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
        self.bank_runtime_boundary();
        #[cfg(test)]
        if std::mem::take(&mut self.fail_spawn_for_test) {
            return Err("isolate thread: test-injected spawn failure".to_string());
        }
        let run_generation = self
            .runtime_generation
            .checked_add(u64::from(bump))
            .ok_or_else(|| "run generation exhausted".to_string())?;
        let execution = ExecutionId::first(api::selected::RunKey {
            slot: self.incarnation,
            run: run_generation,
            session: self.work_epoch,
        });
        let isolate = GameplayFactory::spawn_with_content(
            identity.source.to_string(),
            identity.shape,
            identity.siblings.iter().cloned().collect(),
            identity.game_data.clone(),
            Arc::clone(&identity.named_banks),
            Some(execution),
        )
        .map_err(|error| error.to_string())?;
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
            self.runtime_generation = run_generation;
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
        loadouts: &[api::loadout::Loadout],
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
                        if self.incarnation != 0 {
                            self.arm_seat(api::selected::RunKey {
                                slot: self.incarnation,
                                run: self.runtime_generation,
                                session: self.work_epoch,
                            });
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
        let native = self.compiled.is_some()
            || self.preparing.is_some()
            || self.pending_compiled_start.is_some();
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
        self.pending_compiled_start = None;
        self.pending_compiled_restart = false;
        self.held_walk_terminals.clear();
        if let Some(mut seat) = self.seat.take() {
            seat.stop();
        }
        self.seat_fence.revoke();
        self.seat_compiled_owner = None;
        self.seat_delivery.clear();
        self.seat_native_delivery.clear();
        #[cfg(feature = "load")]
        self.seat_interact_delivery.clear();
        #[cfg(feature = "load")]
        self.held_aborts.clear();
        #[cfg(feature = "load")]
        {
            self.held_policy = None;
        }
        #[cfg(feature = "load")]
        self.seat_lifecycle.clear();
        #[cfg(feature = "load")]
        self.held_channel_refusals.clear();
        #[cfg(feature = "load")]
        self.teardown_api(reason);
        #[cfg(feature = "load")]
        self.bank_runtime_boundary();
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
                self.complete_withdraw_x(false);
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
        #[cfg(feature = "load")]
        let discarded_walks = if reconnect {
            let seat_walks = self
                .seat
                .as_ref()
                .into_iter()
                .flat_map(|seat| seat.undelivered_walk_ids().map(|id| id.get()));
            let delivered_walks = self
                .seat_interact_delivery
                .iter()
                .filter_map(|queued| seat_walk_id(&queued.req, None).map(|id| id.get()));
            let mut ids = Vec::with_capacity(api::script_contract::MAX_WALK_RELEASES);
            for id in seat_walks.chain(delivered_walks) {
                if ids.len() == api::script_contract::MAX_WALK_RELEASES {
                    break;
                }
                if !ids.contains(&id) {
                    ids.push(id);
                }
            }
            ids
        } else {
            Vec::new()
        };
        // The same run continues in a new session: evidence, owners and
        // receipts stamped before the boundary cannot meet a fence after it.
        if let Some(run) = self.compiled.as_mut() {
            run.rekey_session(self.work_epoch);
        }
        if let Some(seat) = &mut self.seat {
            if reconnect {
                seat.reconnect(self.work_epoch);
            } else {
                let mut run = seat.run();
                run.session = self.work_epoch;
                seat.reset(run);
                self.seat_compiled_owner = None;
            }
        }
        if reconnect {
            self.discard_seat_gameplay_delivery();
        } else {
            self.seat_delivery.clear();
            self.seat_native_delivery.clear();
            #[cfg(feature = "load")]
            self.seat_interact_delivery.clear();
            #[cfg(feature = "load")]
            self.held_aborts.clear();
            #[cfg(feature = "load")]
            {
                self.held_policy = None;
            }
        }
        #[cfg(feature = "load")]
        self.seat_lifecycle.clear();
        #[cfg(feature = "load")]
        self.held_channel_refusals.clear();
        self.seat_fence.revoke();
        #[cfg(feature = "load")]
        self.api_session_boundary(reconnect);
        #[cfg(feature = "load")]
        self.bank_session_boundary(reconnect);
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
            let execution = ExecutionId::first(api::selected::RunKey {
                slot: self.incarnation,
                run: self.runtime_generation,
                session: self.work_epoch,
            });
            let carried_error = self.active_tick_error_generation.is_some();
            let held = reconnect && self.load.is_some();
            if let Some(isolate) = &self.load {
                let generation = if held {
                    isolate.reconnect_session_work(Some(execution))
                } else {
                    isolate.reset_session_work(Some(execution))
                };
                if held {
                    for token in &discarded_walks {
                        isolate.rearm_host_discarded_walk(*token);
                    }
                }
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
    pub fn post_loadouts(&mut self, loadouts: &[api::loadout::Loadout]) {
        if let Some(isolate) = &self.load {
            isolate.post_loadouts(loadouts);
        }
        if let Some(identity) = &mut self.load_identity {
            identity.loadouts = Arc::from(loadouts.to_vec());
        }
    }

    /// Start a JS Load isolate with `loadouts` posted and optionally post
    /// the operator settings bag.
    #[cfg(feature = "load")]
    pub fn start_load_with_settings(
        &mut self,
        source: String,
        shape: LoadShape,
        bag: Option<&serde_json::Map<String, serde_json::Value>>,
        siblings: Vec<(String, String)>,
        loadouts: &[api::loadout::Loadout],
    ) -> Result<(), String> {
        self.start_load_with_settings_and_game_data(
            source,
            shape,
            bag,
            siblings,
            loadouts,
            None,
            std::sync::Arc::new(api::named_banks::NamedBankFacts::empty()),
        )
    }

    /// Start with settings, the operator's loadout rows and the immutable
    /// facts selected by the owning Play.
    #[cfg(feature = "load")]
    #[allow(clippy::too_many_arguments)] // the composition's start inputs, each owned by a different seam
    pub fn start_load_with_settings_and_game_data(
        &mut self,
        source: String,
        shape: LoadShape,
        bag: Option<&serde_json::Map<String, serde_json::Value>>,
        siblings: Vec<(String, String)>,
        loadouts: &[api::loadout::Loadout],
        game_data: Option<std::sync::Arc<api::game_data::SelectedGameData>>,
        named_banks: std::sync::Arc<api::named_banks::NamedBankFacts>,
    ) -> Result<(), String> {
        self.start_load_with_settings_and_game_data_typed(
            source,
            shape,
            bag,
            siblings,
            loadouts,
            game_data,
            named_banks,
        )
        .map_err(|e| e.to_string())
    }

    /// Typed initial-start boundary; refuses active slots before evaluating
    /// source. `loadouts` are the operator's rows the composition leased
    /// from the local-state service: a `loadout` setting in `bag` must
    /// name one of them, or the start is refused before anything spawns.
    #[cfg(feature = "load")]
    #[allow(clippy::too_many_arguments)] // the composition's start inputs, each owned by a different seam
    pub fn start_load_with_settings_and_game_data_typed(
        &mut self,
        source: String,
        shape: LoadShape,
        bag: Option<&serde_json::Map<String, serde_json::Value>>,
        siblings: Vec<(String, String)>,
        loadouts: &[api::loadout::Loadout],
        game_data: Option<std::sync::Arc<api::game_data::SelectedGameData>>,
        named_banks: std::sync::Arc<api::named_banks::NamedBankFacts>,
    ) -> Result<(), StartLoadError> {
        if let Some(bag) = bag {
            let wanted = bag.get("loadout").map(|value| value.as_str().unwrap_or(""));
            crate::loadouts_store::resolve_script_loadout_setting(loadouts, wanted)
                .map(|_| ())
                .map_err(|error| StartLoadError::Refused(error.to_string()))?;
        }
        self.start_load_with_loadouts_and_game_data_typed(
            source,
            shape,
            siblings,
            loadouts,
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
    /// next observe is a delta. The actual changed-field mask advances the
    /// isolate evidence sequence without cloning the fingerprint.
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
        native.api_combat = self
            .api
            .as_ref()
            .and_then(|seat| seat.combat_page.as_deref());
        native.api_combat_outcome = self
            .api
            .as_ref()
            .and_then(|seat| seat.combat_terminal.as_ref());
        native.api_progress = self
            .api
            .as_ref()
            .and_then(|seat| seat.progress_page.as_ref());
        let details = self.seat.as_ref().map(|seat| {
            seat_live_details(
                seat,
                self.api.as_deref(),
                self.compiled.as_deref(),
                self.seat_compiled_owner,
            )
        });
        let mut bank_values = [crate::isolate_fb::SeatResultValue {
            id: RequestId::isolate(1).expect("isolate namespace"),
            value: &[],
        }; api::script_contract::MAX_RESULTS];
        let bank_values_len = self
            .bank
            .as_ref()
            .map_or(0, |bank| bank.result_values(&mut bank_values));
        if let (Some(seat), Some(details)) = (self.seat.as_ref(), details.as_ref()) {
            native.seat_page = Some(
                crate::isolate_fb::SeatPageInput::new(
                    seat.live(),
                    seat.results(),
                    &details[..seat.live().len()],
                    seat.mirror_revokes_consumed(),
                )
                .expect("slot-owned seat page invariant")
                .with_result_values(&bank_values[..bank_values_len])
                .expect("slot-owned bank result invariant"),
            );
        }
        let (bytes, fp, evidence_changed) =
            self.ipc.encode_snapshot_delta_with_native_and_evidence(
                self.last_snapshot.as_ref(),
                input,
                native,
                force_banks,
            );
        self.last_snapshot = Some(fp);
        if let Some(isolate) = self.load.as_ref() {
            isolate.record_snapshot_evidence_change(evidence_changed);
        }
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
        native.api_combat = self
            .api
            .as_ref()
            .and_then(|seat| seat.combat_page.as_deref());
        native.api_combat_outcome = self
            .api
            .as_ref()
            .and_then(|seat| seat.combat_terminal.as_ref());
        native.api_progress = self
            .api
            .as_ref()
            .and_then(|seat| seat.progress_page.as_ref());
        let details = self.seat.as_ref().map(|seat| {
            seat_live_details(
                seat,
                self.api.as_deref(),
                self.compiled.as_deref(),
                self.seat_compiled_owner,
            )
        });
        let mut bank_values = [crate::isolate_fb::SeatResultValue {
            id: RequestId::isolate(1).expect("isolate namespace"),
            value: &[],
        }; api::script_contract::MAX_RESULTS];
        let bank_values_len = self
            .bank
            .as_ref()
            .map_or(0, |bank| bank.result_values(&mut bank_values));
        if let (Some(seat), Some(details)) = (self.seat.as_ref(), details.as_ref()) {
            native.seat_page = Some(
                crate::isolate_fb::SeatPageInput::new(
                    seat.live(),
                    seat.results(),
                    &details[..seat.live().len()],
                    seat.mirror_revokes_consumed(),
                )
                .expect("slot-owned seat page invariant")
                .with_result_values(&bank_values[..bank_values_len])
                .expect("slot-owned bank result invariant"),
            );
        }
        let (bytes, fp, evidence_changed) = self.ipc.encode_snapshot_wake_with_native_and_evidence(
            self.last_snapshot.as_mut(),
            input,
            native,
            force_banks,
            preserve_inv,
        );
        self.last_snapshot = Some(fp);
        if let Some(isolate) = self.load.as_ref() {
            isolate.record_snapshot_evidence_change(evidence_changed);
        }
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

    /// Live work a manual movement takes over: a walking operation, or a
    /// Gather or Combat API session with or without a walk in flight.
    pub fn manual_input_owner(&self) -> bool {
        #[cfg(feature = "load")]
        if self.api_session_takes_manual_input() {
            return true;
        }
        self.live_walking_operation()
    }

    pub fn note_manual_walk_takeover(&mut self, intent_seq: u64, tick: u64) {
        // A deliberate takeover is gameplay progress, even if the human
        // clicked an unwalkable tile and no player-info movement follows.
        self.watchdog.stamp_gameplay(Instant::now());
        #[cfg(feature = "load")]
        if let Some(isolate) = &self.load {
            isolate.note_manual_walk_takeover(intent_seq);
        }
        #[cfg(feature = "load")]
        self.api_user_input();
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

    /// Only observations update the walk stamp. A matching physical terminal
    /// retires its seat walk-release slot once; guardian hold defers that edge.
    pub fn observe_walk_outcome(&mut self, seq: u64, terminal: Option<SeatWalkTerminal>) -> bool {
        let changed = self.native_runtime.observed_walk_outcome_seq != seq;
        self.native_runtime.observed_walk_outcome_seq = seq;
        if changed {
            let id = match terminal {
                Some(SeatWalkTerminal::Native(raw)) => native_walk_id(raw.get()),
                Some(SeatWalkTerminal::Load(id)) => Some(id),
                None => None,
            };
            if let Some(id) = id {
                if self.guardian_held {
                    if !self.held_walk_terminals.contains(&id)
                        && self.held_walk_terminals.len() < api::script_contract::MAX_WALK_RELEASES
                    {
                        self.held_walk_terminals.push(id);
                    }
                } else if let Some(seat) = &mut self.seat {
                    seat.walk_terminal(id);
                }
            }
        }
        changed
    }

    fn capture_native_seat_rows(&mut self) {
        let Some(seat) = self.seat.as_mut() else {
            return;
        };
        let active_native_walk = self
            .native_runtime
            .ledger
            .as_ref()
            .and_then(|ledger| ledger.owner.as_ref())
            .and_then(|owner| owner.active_walk())
            .and_then(|request| native_walk_id(request.get()));
        seat.retire_inactive_native_walks(active_native_walk);
        #[cfg(feature = "load")]
        let bank_owner = self.bank.as_ref().and_then(|bank| bank.action_owner);
        #[cfg(not(feature = "load"))]
        let bank_owner = None;
        let owner = bank_owner
            .or(self.seat_compiled_owner)
            .or_else(|| seat.session());
        let source = owner.map_or(EmissionSource::Root, EmissionSource::Owner);
        let Some(ledger) = self.native_runtime.ledger.as_mut() else {
            return;
        };
        for action in ledger.outbox.drain(..) {
            if !action.live() {
                seat.count_dropped(1);
                continue;
            }
            let walk = match &action.effect {
                crate::native::HostEffect::Walk(_) => native_walk_id(action.request_id.get()),
                crate::native::HostEffect::Interaction(req) => seat_walk_id(req, owner),
                _ => None,
            };
            if matches!(&action.effect, crate::native::HostEffect::Walk(_)) && walk.is_none() {
                seat.count_dropped(1);
                continue;
            }
            let (class, release) = match &action.effect {
                crate::native::HostEffect::Interaction(crate::shim::InteractReq::AbortWalk {
                    request_id,
                }) => (
                    api::script_contract::EmissionClass::Release,
                    native_walk_id(*request_id)
                        .or(owner)
                        .map(api::script_contract::Release::AbortWalk),
                ),
                crate::native::HostEffect::Interaction(req) => seat_interact_class(req, owner),
                crate::native::HostEffect::Walk(_)
                | crate::native::HostEffect::BankPick(_)
                | crate::native::HostEffect::AssessWalk(_) => {
                    (api::script_contract::EmissionClass::Gameplay, None)
                }
            };
            let effect = SeatEffect::Native(action);
            let result = match walk {
                Some(id) => seat.push_walk(effect, source, id),
                None => seat.push(effect, class, source, release),
            };
            if let Err(error) = result {
                self.pending_logs
                    .push(format!("seat native emission refused: {error:?}"));
            }
        }
    }

    #[cfg(feature = "load")]
    fn capture_compiled_seat_interacts(&mut self) {
        let Some(seat) = self.seat.as_mut() else {
            return;
        };
        let owner = self.seat_compiled_owner.or_else(|| seat.session());
        let source = owner.map_or(EmissionSource::Root, EmissionSource::Owner);
        let reqs = &mut self.compiled_interacts;
        let stamps = &mut self.compiled_interact_outcome_seqs;
        debug_assert_eq!(reqs.len(), stamps.len());
        for (req, observed_walk_outcome_seq) in reqs.drain(..).zip(stamps.drain(..)) {
            let (class, release) = seat_interact_class(&req, owner);
            let walk = seat_walk_id(&req, owner);
            let reserved_policy = matches!(req, crate::shim::InteractReq::RunPolicyOverride { .. });
            let effect = SeatEffect::Interact(crate::load::QueuedInteract {
                req,
                observed_walk_outcome_seq,
            });
            let result = match walk {
                Some(id) if class == api::script_contract::EmissionClass::Gameplay => {
                    seat.push_walk(effect, source, id)
                }
                _ if reserved_policy => seat.push_policy(effect, source),
                _ => seat.push(effect, class, source, release),
            };
            if let Err(error) = result {
                self.pending_logs
                    .push(format!("seat compiled interaction refused: {error:?}"));
            }
        }
    }

    fn dispatch_seat_rows(&mut self) {
        let Some(seat) = self.seat.as_mut() else {
            return;
        };
        seat.take();
        seat.dispatch_into(&mut self.seat_delivery);
        for row in self.seat_delivery.drain(..) {
            match row {
                SeatEffect::Native(action) => self.seat_native_delivery.push_back(action),
                #[cfg(feature = "load")]
                SeatEffect::Interact(interact)
                    if matches!(
                        &interact.req,
                        crate::shim::InteractReq::NoteProgress
                            | crate::shim::InteractReq::LoopSettled
                            | crate::shim::InteractReq::WaitEnqueued
                            | crate::shim::InteractReq::WaitSettled
                            | crate::shim::InteractReq::RecoveryAnchor { .. }
                            | crate::shim::InteractReq::RecoveryAnchorNone
                    ) =>
                {
                    self.seat_lifecycle.push(interact.req);
                }
                #[cfg(feature = "load")]
                SeatEffect::Interact(interact) => self.seat_interact_delivery.push(interact),
            }
        }
    }

    /// A row captured before ordinary Done retains its immutable lease until
    /// the host's final dispatch fence for this frame. Error/Stop never use
    /// this path; they revoke the owner and clear the seat.
    pub fn completed_seat_dispatch_open(&self) -> bool {
        self.state == RunState::Idle
            && !self.want_run
            && self.lifecycle_receipt.as_ref().is_some_and(|receipt| {
                receipt.runtime_generation == self.runtime_generation
                    && receipt.state == ScriptTerminalState::Completed
            })
            && self
                .native_runtime
                .ledger
                .as_ref()
                .and_then(|ledger| ledger.owner.as_ref())
                .is_some_and(|owner| owner.live())
    }

    /// Queued native work the host may dispatch now. Work queued before a
    /// watchdog recovery hold waits for it to end; Blocked revoked its work.
    pub fn has_native_actions(&self) -> bool {
        if let Some(seat) = &self.seat {
            let completed = self.completed_seat_dispatch_open();
            return completed
                || (self.dispatch_open()
                    && (!self.seat_native_delivery.is_empty() || seat.has_rows()));
        }
        self.native_runtime
            .ledger
            .as_ref()
            .is_some_and(|ledger| ledger.outbox.iter().any(|action| action.live()))
            && self.dispatch_open()
    }

    /// The host calls this only while holding the slot's final dispatch fence.
    /// Keep the bounded outbox's allocation for the next observed tick.
    pub fn take_native_action(&mut self) -> Option<crate::native::HostAction> {
        if !self.dispatch_open() && !self.completed_seat_dispatch_open() {
            return None;
        }
        if self.seat.is_some() {
            let completed = self.completed_seat_dispatch_open();
            self.dispatch_seat_rows();
            while let Some(action) = self.seat_native_delivery.pop_front() {
                if action.live() {
                    return Some(action);
                }
                if let Some(seat) = &mut self.seat {
                    seat.count_dropped(1);
                }
            }
            if completed {
                self.native_runtime.revoke();
            }
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
            let request_id = receipt.request_id;
            ledger.walk = Some(receipt);
            // The native ledger uses a separate seat id from a Load walk
            // with the same request number. Its accepted terminal is the
            // authority for freeing that reservation, even when the host has
            // already armed a newer route.
            if let Some(id) = native_walk_id(request_id) {
                if self.guardian_held {
                    if !self.held_walk_terminals.contains(&id)
                        && self.held_walk_terminals.len() < api::script_contract::MAX_WALK_RELEASES
                    {
                        self.held_walk_terminals.push(id);
                    }
                } else if let Some(seat) = &mut self.seat {
                    seat.walk_terminal(id);
                }
            }
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
        if self.seat.is_some() {
            self.dispatch_seat_rows();
            return std::mem::take(&mut self.seat_interact_delivery)
                .into_iter()
                .map(|queued| queued.req)
                .collect();
        }
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

    /// One physical isolate batch is the custody unit: its generation and
    /// observed-walk stamp cannot be borrowed from a later producer tick.
    #[cfg(feature = "load")]
    fn consume_seat_batch(
        &mut self,
        batch: DecodedSeatBatch,
        observed_walk_outcome_seq: u64,
        machine_events: &[crate::machine::MachineEvent],
    ) {
        if self.admit_seat_batch(&batch) {
            self.apply_seat_batch(batch, observed_walk_outcome_seq, machine_events);
        }
    }

    /// Preflight a batch before any of its rows reaches the seat. A
    /// malformed batch is dropped whole, and its rows are counted.
    #[cfg(feature = "load")]
    fn admit_seat_batch(&mut self, batch: &DecodedSeatBatch) -> bool {
        let Some(seat) = self.seat.as_ref() else {
            self.pending_logs
                .push("seat batch without an armed seat".into());
            return false;
        };
        let run = seat.run();
        let Some(execution) = self.seat_fence.current() else {
            self.pending_logs
                .push("seat batch without an execution fence".into());
            return false;
        };
        if batch.starts.iter().any(|start| {
            start.run != run.run
                || start.session != run.session
                || RequestStamp::new(start.id, start.parent, None, execution).is_err()
        }) {
            self.pending_logs
                .push("dropped malformed seat batch: start run or stamp".into());
            self.seat
                .as_mut()
                .expect("armed")
                .count_dropped(batch.reqs.len());
            return false;
        }
        if let Err(error) = seat
            .validate_mirror_revokes(batch.mirror_revokes.iter().map(|row| (row.owner, row.seq)))
        {
            // An ordered connection cannot skip an unacknowledged sequence:
            // the adapter carries a contiguous prefix until page receipt.
            self.pending_logs.push(format!(
                "dropped malformed seat batch: mirror sequence {error:?}"
            ));
            self.seat
                .as_mut()
                .expect("armed")
                .count_dropped(batch.reqs.len());
            return false;
        }
        true
    }

    /// Apply an admitted batch: push its rows, then its controls and
    /// starts, then take and dispatch.
    #[cfg(feature = "load")]
    fn apply_seat_batch(
        &mut self,
        mut batch: DecodedSeatBatch,
        observed_walk_outcome_seq: u64,
        machine_events: &[crate::machine::MachineEvent],
    ) {
        let execution = self
            .seat_fence
            .current()
            .expect("an admitted batch has an execution fence");
        // The isolate emitted these rows before this host batch applied its
        // terminal controls. Pin push-time authority now; watermark, controls
        // and starts then settle or witness the rows before Take/Dispatch.
        self.push_seat_rows(
            batch.reqs.drain(..),
            &batch.starts,
            observed_walk_outcome_seq,
        );
        let seat = self.seat.as_mut().expect("armed");
        seat.watermark(batch.consumed_terminal_seq);
        for cancel in &batch.cancels {
            if !seat.live().iter().any(|entry| entry.id == cancel.of)
                && !seat.results().iter().any(|result| result.id == cancel.of)
                && batch.starts.iter().any(|start| start.id == cancel.of)
            {
                // The cancellation is part of the carried pending start.
                continue;
            }
            if let Err(error) = seat.cancel(cancel.of, cancel.reason) {
                self.pending_logs
                    .push(format!("seat cancel refused: {error:?}"));
            }
        }
        for revoke in &batch.mirror_revokes {
            if let Err(error) = seat.apply_mirror_revoke(revoke.owner, revoke.seq) {
                self.pending_logs
                    .push(format!("seat mirror revoke refused: {error:?}"));
            }
        }
        for decision in &mut batch.decisions {
            if seat.decide_with_answer(
                decision.of,
                decision.sequence,
                std::mem::take(&mut decision.answer),
            ) == crate::native::seat::SeatDecisionStatus::Invalid
            {
                self.pending_logs.push(format!(
                    "seat decision did not match a live prompt: {} seq {}",
                    decision.of.get(),
                    decision.sequence.get()
                ));
            }
        }
        self.stop_cancelled_api_work();
        self.stop_cancelled_bank_work();
        for start in &mut batch.starts {
            let stamp = RequestStamp::new(start.id, start.parent, None, execution)
                .expect("start stamp was preflighted");
            let cancelled = batch.cancels.iter().any(|cancel| {
                cancel.of == start.id
                    && cancel.reason == api::script_contract::CallbackCancelReason::ParentAborted
            });
            let admission = match self
                .seat
                .as_mut()
                .expect("armed")
                .start(stamp, start.kind, cancelled)
            {
                Ok(admission) => admission,
                Err(error) => {
                    self.pending_logs
                        .push(format!("seat start refused: {error:?}"));
                    break;
                }
            };
            match admission {
                Admission::Admitted => {
                    if batch.cancels.iter().any(|cancel| {
                        cancel.of == start.id
                            && cancel.reason
                                == api::script_contract::CallbackCancelReason::HookThrew
                    }) {
                        if let Err(error) = self.seat.as_mut().expect("armed").cancel(
                            start.id,
                            api::script_contract::CallbackCancelReason::HookThrew,
                        ) {
                            self.pending_logs
                                .push(format!("seat hook throw refused: {error:?}"));
                        }
                        continue;
                    }
                    match std::mem::replace(&mut start.args, SeatStartArgs::Empty) {
                        SeatStartArgs::Gather(settings) => {
                            self.start_api_gather(start.id.get(), &Arc::new(settings));
                        }
                        SeatStartArgs::Combat(request) => {
                            self.start_api_combat(start.id.get(), &Arc::new(request));
                        }
                        SeatStartArgs::QuestProgress(name) => {
                            self.start_api_progress(start.id.get(), &name);
                        }
                        SeatStartArgs::Bank(request) => {
                            self.start_bank(start.id, request);
                        }
                        SeatStartArgs::Empty => {}
                    }
                }
                Admission::Deferred => break,
                Admission::Refused(_)
                | Admission::Cancelled
                | Admission::Duplicate
                | Admission::Stale => {}
            }
        }
        for event in machine_events {
            use crate::machine::MachineEventKind;
            let seat = self.seat.as_mut().expect("armed");
            let result = match event.kind {
                MachineEventKind::Phase(phase) => seat.set_machine_phase(event.id, phase),
                MachineEventKind::Done => seat.complete(event.id, false),
                MachineEventKind::Failed => seat.complete(event.id, true),
                MachineEventKind::Aborted(reason) => seat.revoke(event.id, reason),
                MachineEventKind::ManualWalk => seat.manual_walk(event.id),
            };
            if let Err(error) = result {
                self.pending_logs
                    .push(format!("seat machine event refused: {error:?}"));
            }
        }
        self.dispatch_seat_rows();
        self.settle_api_seat_terminals();
    }

    /// Pin each row's push-time authority and queue it in the seat. A row
    /// whose lineage names a start of `starts` that the seat has not
    /// admitted yet has conditional custody until that start is admitted.
    #[cfg(feature = "load")]
    fn push_seat_rows(
        &mut self,
        rows: impl Iterator<Item = crate::isolate_fb::SeatInteractDecoded>,
        starts: &[crate::isolate_fb::SeatStartInput],
        observed_walk_outcome_seq: u64,
    ) {
        use api::script_contract::{AuthorityStatus, EmissionClass};
        for row in rows {
            let source = match (row.lineage, row.mirror_owner) {
                (None, None) => EmissionSource::Root,
                (lineage, mirror) => {
                    let pending = lineage.and_then(|id| {
                        let seat = self.seat.as_ref().expect("armed");
                        (!seat.live().iter().any(|entry| entry.id == id)
                            && !seat.results().iter().any(|result| result.id == id)
                            && !matches!(
                                seat.lookup(id),
                                AuthorityStatus::Revoked | AuthorityStatus::Expired
                            ))
                        .then(|| starts.iter().find(|start| start.id == id))
                        .flatten()
                    });
                    match (lineage, mirror, pending) {
                        (Some(id), None, Some(start)) => EmissionSource::Pending {
                            owner: id,
                            parent: start.parent,
                            kind: start.kind,
                        },
                        (Some(id), Some(owner), Some(start)) => EmissionSource::PendingMirror {
                            owner,
                            pending: id,
                            parent: start.parent,
                            kind: start.kind,
                        },
                        (Some(id), _, None)
                            if !self
                                .seat
                                .as_ref()
                                .expect("armed")
                                .live()
                                .iter()
                                .any(|entry| entry.id == id)
                                && !self
                                    .seat
                                    .as_ref()
                                    .expect("armed")
                                    .results()
                                    .iter()
                                    .any(|result| result.id == id)
                                && !matches!(
                                    self.seat.as_ref().expect("armed").lookup(id),
                                    AuthorityStatus::Revoked | AuthorityStatus::Expired
                                ) =>
                        {
                            EmissionSource::InvalidExternal(id)
                        }
                        (Some(id), None, None) => EmissionSource::Owner(id),
                        (parent, Some(owner), None) => EmissionSource::Mirror { owner, parent },
                        (None, Some(_), Some(_)) => {
                            unreachable!("a pending start requires a lineage id")
                        }
                        (None, None, _) => unreachable!("root handled above"),
                    }
                }
            };
            let (class, release) =
                seat_interact_class(&row.request, row.lineage.or(row.mirror_owner));
            let walk = seat_walk_id(&row.request, row.mirror_owner.or(row.lineage));
            let reserved_policy = matches!(
                row.request,
                crate::shim::InteractReq::RunPolicyOverride { .. }
            );
            let effect = SeatEffect::Interact(crate::load::QueuedInteract {
                req: row.request,
                observed_walk_outcome_seq,
            });
            let result = match walk {
                Some(id) if class == EmissionClass::Gameplay => self
                    .seat
                    .as_mut()
                    .expect("armed")
                    .push_walk(effect, source, id),
                _ if reserved_policy => self
                    .seat
                    .as_mut()
                    .expect("armed")
                    .push_policy(effect, source),
                _ => self
                    .seat
                    .as_mut()
                    .expect("armed")
                    .push(effect, class, source, release),
            };
            if let Err(error) = result {
                self.pending_logs
                    .push(format!("seat isolate emission refused: {error:?}"));
            }
        }
    }

    #[cfg(feature = "load")]
    fn compact_held_seat_batches(
        &mut self,
        mut batches: Vec<crate::load::QueuedSeatBatch>,
    ) -> Vec<crate::load::QueuedSeatBatch> {
        let Some(mut latest) = batches.pop() else {
            return batches;
        };
        // The isolate repeats unacknowledged starts, cancels, mirror revokes,
        // machine events and terminal watermarks. The newest batch supersedes
        // those pages. A retained batch that still waits for its pages
        // (`held_fold`) is different: the seat has not applied its controls,
        // so they stay first, in order (`carry_waiting_controls`). The order
        // between control classes (every start before every machine event at
        // resume) is an accepted limit: `carry_waiting_controls` states the
        // condition and why guardian-only holds do not reach it.
        // `SessionHeld` discards gameplay only (§28.4), so the fold bounds
        // the other rows by exact summaries:
        // - Releases are never dropped. A repeated release of the same target
        //   and id is idempotent and is carried once. Walk releases keep the
        //   16-handle reservation (§28.5).
        // - Replacements keep the newest value of each key.
        // - Wait facts fold into one exact `HeldWaitFacts` summary.
        // - Channel and inspect rows stay in one ordered lane. Only a
        //   `ChannelPost` that does not fit one seat take is refused; the
        //   sender gets a channel status and the refusal is counted.
        // The folded batch is releases, the policy row, the lane, then the
        // replacements. Resume pages it into the seat
        // (`consume_held_fold`), so the replacements land after the lane.
        use crate::shim::InteractReq;
        for waiting in batches.iter_mut().rev().filter(|batch| batch.held_fold) {
            carry_waiting_controls(waiting, &mut latest);
        }
        let mut releases: Vec<crate::isolate_fb::SeatInteractDecoded> = Vec::new();
        let mut walk_releases = 0;
        let mut lane: Vec<crate::isolate_fb::SeatInteractDecoded> = Vec::new();
        let mut replacements: Vec<crate::isolate_fb::SeatInteractDecoded> = Vec::new();
        let mut policy = None;
        let mut waits = crate::load::HeldWaitFacts::default();
        let mut refused_posts: Vec<u64> = Vec::new();
        let mut dropped = 0;
        for batch in batches.iter_mut().chain(std::iter::once(&mut latest)) {
            waits = waits.then(std::mem::take(&mut batch.held_waits));
            for row in batch.batch.reqs.drain(..) {
                if let Some(key) = HeldReplacement::of(&row.request) {
                    match replacements
                        .iter_mut()
                        .find(|kept| HeldReplacement::of(&kept.request) == Some(key))
                    {
                        Some(kept) => *kept = row,
                        None => replacements.push(row),
                    }
                    continue;
                }
                match &row.request {
                    InteractReq::AbortWalk { .. }
                    | InteractReq::GatherStop { .. }
                    | InteractReq::CombatStop { .. } => {
                        if releases.iter().any(|kept| kept.request == row.request) {
                            continue;
                        }
                        if matches!(row.request, InteractReq::AbortWalk { .. }) {
                            if walk_releases == api::script_contract::MAX_WALK_RELEASES {
                                dropped += 1;
                                continue;
                            }
                            walk_releases += 1;
                        }
                        releases.push(row);
                    }
                    InteractReq::RunPolicyOverride { .. } => policy = Some(row),
                    InteractReq::WaitEnqueued => waits.enqueue(),
                    InteractReq::WaitSettled => waits.settle(),
                    InteractReq::ChannelPost { channel_id, .. }
                        if lane.len() + HeldReplacement::KEYS >= HELD_LIFECYCLE_ROWS =>
                    {
                        dropped += 1;
                        if !refused_posts.contains(channel_id) {
                            refused_posts.push(*channel_id);
                        }
                    }
                    _ if seat_interact_class(&row.request, row.lineage.or(row.mirror_owner)).0
                        == api::script_contract::EmissionClass::Lifecycle =>
                    {
                        lane.push(row);
                    }
                    // Gameplay normally leaves before the fold. A row that
                    // still reaches it is discarded here, and counted.
                    _ => dropped += 1,
                }
            }
        }
        if let Some(seat) = &mut self.seat {
            seat.count_dropped(dropped);
        }
        self.notify_held_channel_refusals(refused_posts);
        releases.extend(policy);
        releases.extend(lane);
        releases.extend(replacements);
        latest.batch.reqs = releases;
        latest.held_waits = waits;
        latest.held_fold = true;
        vec![latest]
    }

    /// Hand the retained guardian-hold batch to the seat in pages. A page
    /// holds no more non-reserved rows than the next `take` moves (§28.5
    /// line 179), so `take` clears none of them. Releases and the policy
    /// row lead the folded batch, and they go to the seat with the first
    /// page whatever the room: `take` moves reserved rows before its 141
    /// other rows, so they never cause its clear, and a release that frees
    /// a walk slot is never withheld behind a queue that waits for that
    /// slot (§28.5 lines 177-183). The replacements close the batch and go
    /// with the last page, after the lane. The seat takes and dispatches
    /// each page before the next is pushed. The batch's controls apply
    /// with the last page, after all of its rows are pushed, as for one
    /// batch.
    ///
    /// An early page's take also moves rows that already wait in the seat,
    /// before the batch's controls apply. The retained rows are lifecycle
    /// and release rows, whose dispatch does not read those controls.
    /// Native gameplay captured before this drain relies on the host's
    /// final authority fence (`authority.live()` in
    /// `host-play/src/script_observe.rs`), which runs after the drain.
    ///
    /// Returns the rest of the batch when the seat has no room for its
    /// next page in this frame. The caller keeps it, and every later batch
    /// behind it, for the next frame.
    #[cfg(feature = "load")]
    fn consume_held_fold(
        &mut self,
        mut queued: crate::load::QueuedSeatBatch,
    ) -> Option<crate::load::QueuedSeatBatch> {
        if !self.admit_seat_batch(&queued.batch) {
            return None;
        }
        loop {
            let room = self.seat.as_ref().map_or(0, |seat| seat.take_room());
            let rows = &queued.batch.reqs;
            let unreserved = rows
                .iter()
                .filter(|row| !seat_reserved_row(&row.request))
                .count();
            if unreserved <= room {
                self.apply_seat_batch(
                    queued.batch,
                    queued.observed_walk_outcome_seq,
                    &queued.machine_events,
                );
                return None;
            }
            let replacements = rows
                .iter()
                .rev()
                .take_while(|row| HeldReplacement::of(&row.request).is_some())
                .count();
            // Every reserved row goes with this page; only the non-reserved
            // rows wait for room. The fold puts all reserved rows first.
            let page = room.min(unreserved - replacements);
            let mut cut = 0;
            let mut paged = 0;
            while let Some(row) = rows.get(cut) {
                if !seat_reserved_row(&row.request) {
                    if paged == page {
                        break;
                    }
                    paged += 1;
                }
                cut += 1;
            }
            if cut == 0 {
                return Some(queued);
            }
            self.push_seat_rows(
                queued.batch.reqs.drain(..cut),
                &queued.batch.starts,
                queued.observed_walk_outcome_seq,
            );
            self.dispatch_seat_rows();
        }
    }

    /// Tell each script channel whose post the hold refused, once in each
    /// hold, through the broker's status delivery path.
    #[cfg(feature = "load")]
    fn notify_held_channel_refusals(&mut self, refused: Vec<u64>) {
        let mut statuses = Vec::new();
        for channel_id in refused {
            if self.held_channel_refusals.contains(&channel_id) {
                continue;
            }
            self.held_channel_refusals.push(channel_id);
            statuses.push(crate::shim::InteractReq::ChannelStatus {
                channel_id,
                message: HELD_CHANNEL_POST_REFUSED.into(),
            });
        }
        if !statuses.is_empty() {
            let _ = self.post_channel_events(crate::isolate_fb::encode_interact_batch(&statuses));
        }
    }

    /// Hand a held batch's wait summary to the watchdog lane before the seat
    /// consumes the batch. Wait facts carry no authority and never dispatch,
    /// so they skip the seat's row limit, as `dispatch_seat_rows` routes
    /// them. The replay gives the watchdog count that the held facts give in
    /// event order, for every starting count. The hold has ended, so the
    /// channel refusal notices of the hold end too.
    #[cfg(feature = "load")]
    fn release_held_fold(&mut self, batch: &mut crate::load::QueuedSeatBatch) {
        self.held_channel_refusals.clear();
        let waits = std::mem::take(&mut batch.held_waits);
        self.seat_lifecycle
            .reserve(usize::try_from(waits.rows()).unwrap_or(usize::MAX));
        self.seat_lifecycle.extend(waits.replay());
    }

    /// A Paused guardian frame still receives isolate batches. Fold their
    /// repeated control pages before the host resumes ordinary dispatch.
    #[cfg(feature = "load")]
    pub fn retain_held_seat_batches(&mut self) {
        if !self.seat.as_ref().is_some_and(|seat| seat.held()) {
            return;
        }
        let Some(isolate) = &self.load else {
            return;
        };
        let generation = isolate.generation();
        let batches = isolate.drain_seat_batches();
        if batches.is_empty() {
            return;
        }
        let run = self.seat.as_ref().expect("held seat").run();
        let mut carried = Vec::new();
        for mut queued in batches {
            if queued.generation != generation
                || queued
                    .batch
                    .starts
                    .iter()
                    .any(|start| start.run != run.run || start.session != run.session)
            {
                self.seat
                    .as_mut()
                    .expect("held seat")
                    .count_dropped(queued.rows());
                continue;
            }
            let before = queued.batch.reqs.len();
            queued.batch.reqs.retain(|row| {
                seat_interact_class(&row.request, row.lineage.or(row.mirror_owner)).0
                    != api::script_contract::EmissionClass::Gameplay
            });
            self.seat
                .as_mut()
                .expect("held seat")
                .count_dropped(before - queued.batch.reqs.len());
            carried.push(queued);
        }
        let carried = self.compact_held_seat_batches(carried);
        self.load
            .as_ref()
            .expect("batch producer")
            .restore_seat_batches(carried);
    }

    #[cfg(all(feature = "load", feature = "test-hooks"))]
    pub fn seat_batch_backlog(&self) -> usize {
        self.load
            .as_ref()
            .map_or(0, LoadIsolate::seat_batch_backlog)
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
        let batches = self
            .load
            .as_ref()
            .map_or_else(Vec::new, LoadIsolate::drain_seat_batches);
        let mut carried = Vec::new();
        // Set when the retained guardian-hold batch still has pages that
        // the seat cannot take in this frame.
        let mut paging = false;
        // Reserved rows of batches that a watchdog recovery hold drops.
        let mut watchdog_kept = Vec::new();
        for mut queued in batches {
            let current_generation = self.load.as_ref().expect("batch producer").generation();
            if queued.generation != current_generation {
                self.pending_logs
                    .push("dropped stale seat batch generation".into());
                if let Some(seat) = self.seat.as_mut() {
                    seat.count_dropped(queued.rows());
                }
                continue;
            }
            let run = self.seat.as_ref().map(|seat| seat.run());
            if run.is_none_or(|run| {
                queued
                    .batch
                    .starts
                    .iter()
                    .any(|start| start.run != run.run || start.session != run.session)
            }) {
                self.pending_logs
                    .push("dropped malformed seat batch run stamp".into());
                if let Some(seat) = self.seat.as_mut() {
                    seat.count_dropped(queued.rows());
                }
                continue;
            }
            if self.seat.as_ref().is_some_and(|seat| seat.held()) {
                let before = queued.batch.reqs.len();
                queued.batch.reqs.retain(|row| {
                    seat_interact_class(&row.request, row.lineage.or(row.mirror_owner)).0
                        != api::script_contract::EmissionClass::Gameplay
                });
                self.seat
                    .as_mut()
                    .expect("held seat")
                    .count_dropped(before - queued.batch.reqs.len());
                // The carrier still owns starts, controls, watermark and
                // releases. Guardian-held gameplay is never replayed.
                carried.push(queued);
                continue;
            }
            if self.seat_host_held {
                // A held tick of a frame that the slot loop holds
                // (`28-seat-sg.md:64`): the frame's gameplay is dropped and
                // counted, never dispatched after the hold. The seat is not
                // frozen, so the batch's controls and other rows apply now.
                let before = queued.batch.reqs.len();
                queued.batch.reqs.retain(|row| {
                    seat_interact_class(&row.request, row.lineage.or(row.mirror_owner)).0
                        != api::script_contract::EmissionClass::Gameplay
                });
                if let Some(seat) = self.seat.as_mut() {
                    seat.count_dropped(before - queued.batch.reqs.len());
                }
                if queued.held_fold {
                    // The guardian's retained pages wait for the hold to
                    // end, and later batches queue behind them.
                    paging = true;
                    carried.push(queued);
                    continue;
                }
            }
            if self.watchdog.holds_script_actions() {
                self.seat_held_tick();
                // A watchdog recovery hold keeps the batch's controls for
                // after the hold and drops its other rows. Operator decision
                // 2026-10-16 (aborts are never dropped): the releases and the
                // run-policy row join this frame's rows in the order the
                // script sent them. As for a held tick of a scene load, the
                // drain applies a Gather or Combat Stop at once, and
                // `keep_held_aborts` keeps the walk aborts and the newest
                // policy for the first frame after the hold.
                let mut dropped = queued.rows();
                let observed_walk_outcome_seq = queued.observed_walk_outcome_seq;
                for row in queued.batch.reqs.drain(..) {
                    if seat_reserved_row(&row.request) {
                        dropped -= 1;
                        watchdog_kept.push(crate::load::QueuedInteract {
                            req: row.request,
                            observed_walk_outcome_seq,
                        });
                    }
                }
                queued.held_waits = crate::load::HeldWaitFacts::default();
                self.seat.as_mut().expect("armed").count_dropped(dropped);
                carried.push(queued);
                continue;
            }
            if !self.dispatch_open() || paging {
                // Closed dispatch keeps the batch. So do retained pages that
                // wait: later batches queue behind them, in order.
                carried.push(queued);
                continue;
            }
            self.release_held_fold(&mut queued);
            if queued.held_fold {
                if let Some(rest) = self.consume_held_fold(queued) {
                    paging = true;
                    carried.push(rest);
                }
                continue;
            }
            self.consume_seat_batch(
                queued.batch,
                queued.observed_walk_outcome_seq,
                &queued.machine_events,
            );
        }
        if !carried.is_empty() {
            let carried = if self.seat.as_ref().is_some_and(|seat| seat.held()) {
                self.compact_held_seat_batches(carried)
            } else {
                carried
            };
            self.load
                .as_ref()
                .expect("batch producer")
                .restore_seat_batches(carried);
        }
        if self.dispatch_open() || self.completed_seat_dispatch_open() {
            self.dispatch_seat_rows();
        }
        let mut reqs = self.drain_interacts_stamped();
        if self.dispatch_open() || self.completed_seat_dispatch_open() {
            reqs.extend(std::mem::take(&mut self.seat_interact_delivery));
        }
        reqs.extend(watchdog_kept);
        let mut policy = None;
        reqs.retain(|queued| match &queued.req {
            crate::shim::InteractReq::RunPolicyOverride { policy: update } => {
                policy = Some(*update);
                false
            }
            crate::shim::InteractReq::GatherStop { .. }
            | crate::shim::InteractReq::CombatStop { .. } => {
                self.consume_api_release(&queued.req);
                false
            }
            crate::shim::InteractReq::GatherRun { .. }
            | crate::shim::InteractReq::CombatFight { .. }
            | crate::shim::InteractReq::ProgressRead { .. } => {
                self.pending_logs
                    .push("dropped malformed legacy API start outside typed seat".into());
                if let Some(seat) = self.seat.as_mut() {
                    seat.count_dropped(1);
                }
                false
            }
            _ => true,
        });
        self.keep_held_aborts(&mut policy, &mut reqs);
        (policy, reqs, self.api_owns_foreground())
    }

    /// While the frame is held, the host dispatches no script row: the slot
    /// loop's hold closes the `up && !hold` gate (`script_observe.rs`), and
    /// a watchdog recovery hold drops the drained rows unless an API session
    /// owns the foreground. A walk abort or a run-policy update that reached
    /// this drain would be lost. Operator decision 2026-10-16 amends
    /// `28-seat-sg.md:64`: aborts are never dropped. Keep each walk abort
    /// here, in order, and the newest policy update, and give them to the
    /// first frame after the hold: the aborts in front of that frame's rows,
    /// and the policy as that frame's update unless the frame has a newer
    /// one. `GatherStop` and `CombatStop` never reach this point: the drain
    /// applies them in the slot at once (`consume_api_release`). The seat
    /// reserves one release slot for each walk (`MAX_WALK_RELEASES`), so this
    /// keeps one abort for each walk request and at most
    /// `MAX_WALK_RELEASES`. The drain counts a further abort as dropped.
    #[cfg(feature = "load")]
    fn keep_held_aborts(
        &mut self,
        policy: &mut Option<Option<api::run_policy::RunPolicyOverride>>,
        reqs: &mut Vec<crate::load::QueuedInteract>,
    ) {
        let held = self.guardian_held
            || self.seat_host_held
            || (self.watchdog.holds_script_actions() && !self.api_owns_foreground());
        if !held {
            if !self.held_aborts.is_empty() {
                reqs.splice(0..0, self.held_aborts.drain(..));
            }
            // A policy update of this frame is newer than the kept one.
            let kept = self.held_policy.take();
            if policy.is_none() {
                *policy = kept;
            }
            return;
        }
        if let Some(update) = policy.take() {
            // A policy row that still waits in the seat was sent before
            // this one. Only the newest is kept.
            if let Some(seat) = self.seat.as_mut() {
                seat.discard_policy();
            }
            self.held_policy = Some(update);
        }
        let mut dropped = 0;
        let kept = &mut self.held_aborts;
        reqs.retain(|queued| {
            if !matches!(queued.req, crate::shim::InteractReq::AbortWalk { .. }) {
                return true;
            }
            // Only aborts are kept, so an equal row is the same walk's abort.
            if !kept.iter().any(|held| held.req == queued.req) {
                if kept.len() < api::script_contract::MAX_WALK_RELEASES {
                    kept.push(queued.clone());
                } else {
                    dropped += 1;
                }
            }
            false
        });
        if let Some(seat) = self.seat.as_mut() {
            seat.count_dropped(dropped);
        }
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
    pub fn drain_lifecycle(&mut self) -> Vec<crate::shim::InteractReq> {
        let mut facts = self
            .load
            .as_ref()
            .map_or_else(Vec::new, LoadIsolate::drain_lifecycle);
        facts.append(&mut self.seat_lifecycle);
        facts
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

    /// Record a fresh player-info tick observed from game packet input.
    pub fn note_game_packet_tick(&mut self, now: Instant) {
        self.watchdog.stamp_game_packet_tick(now);
    }

    #[cfg(feature = "load")]
    fn duel_fight_pen_anchor_unreachable(player: WatchdogTile, anchor: WatchdogTile) -> bool {
        let pen_index = |tile: WatchdogTile| {
            crate::duel::pen_at(crate::observed::Tile {
                x: tile.x,
                z: tile.z,
                level: tile.level,
            })
        };
        let Some(player_pen) = pen_index(player) else {
            return false;
        };
        Some(player_pen) != pen_index(anchor)
    }

    #[cfg(feature = "load")]
    fn hold_fight_pen_recovery_if_needed(
        &mut self,
        player: Option<WatchdogTile>,
        anchor: Option<WatchdogTile>,
    ) -> Option<WatchdogAction> {
        let (Some(player), Some(anchor)) = (player, anchor) else {
            return None;
        };
        if !self.load_active() || !Self::duel_fight_pen_anchor_unreachable(player, anchor) {
            return None;
        }
        let action = self.watchdog.hold_in_fight_pen(anchor);
        if action != WatchdogAction::HoldInFightPen {
            return None;
        }
        self.pending_logs.push(
            "watchdog: holding recovery in a Duel Arena fight pen; the recovery anchor is outside the pen"
                .into(),
        );
        Some(action)
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
        #[cfg(feature = "load")]
        if self.load_active() {
            if let (Some(anchor), Some((x, z, level))) =
                (self.watchdog.held_fight_pen_anchor(), here)
            {
                let player = WatchdogTile { x, z, level };
                if !Self::duel_fight_pen_anchor_unreachable(player, anchor) {
                    self.watchdog.resume_after_fight_pen(now);
                }
            }
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
            #[cfg(feature = "load")]
            if let Some(action) = self.hold_fight_pen_recovery_if_needed(player, reply) {
                return action;
            }
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
            // `api.gather` has a separate recovery-anchor path; keep its
            // Duel Arena fight-pen check consistent with lifecycle anchors.
            if let Some(anchor) = self.api_recovery_anchor() {
                let player = here.map(|(x, z, level)| WatchdogTile { x, z, level });
                if let Some(action) = self.hold_fight_pen_recovery_if_needed(player, anchor) {
                    return action;
                }
                return self.watchdog.on_anchor(now, player, anchor);
            }
        }
        match action {
            WatchdogAction::WarnHungLoop => self
                .pending_logs
                .push("watchdog: hung loop (10s, no scheduler progress)".into()),
            WatchdogAction::WarnNoGamePackets => self
                .pending_logs
                .push("watchdog: no game packets for 10 s".into()),
            _ => {}
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
            let evidence_sequence = isolate.snapshot_evidence_sequence();
            let input_identity = self.native_input.lock().identity();
            isolate.on_snapshot_change_at(ctx.tick, input_identity, evidence_sequence);
            self.tick_api(ctx);
            self.tick_bank(ctx);
            self.capture_native_seat_rows();
            self.settle_api_seat_terminals();
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
            if ctx.compiled.hold {
                self.seat_held_tick();
                return;
            }
            self.tick_api(ctx);
            self.tick_bank(ctx);
            self.capture_native_seat_rows();
            self.settle_api_seat_terminals();
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
                let service_authority = ctx.compiled.service_gate.and_then(|gate| {
                    self.seat_fence.current().map(|execution| {
                        crate::native::ServiceAuthority::new(execution, &self.seat_fence, gate)
                    })
                });
                let (result, interacts) = run.tick(
                    ctx,
                    &mut retained,
                    &mut self.native_runtime,
                    self.quest_pairs.as_deref(),
                    self.local_state.as_deref(),
                    compiled::SeatTickPorts {
                        authority: service_authority,
                        prompt: self
                            .seat
                            .as_mut()
                            .map(|seat| seat.as_mut() as &mut dyn crate::native::SeatPromptPort),
                    },
                );
                #[cfg(feature = "load")]
                {
                    ctx.compiled.interacts = interacts;
                }
                #[cfg(not(feature = "load"))]
                let _ = interacts;
                run.finish_tick();
                result
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
        self.capture_native_seat_rows();
        #[cfg(feature = "load")]
        self.capture_compiled_seat_interacts();
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
                if let Some(id) = self.seat_compiled_owner.take() {
                    if let Some(seat) = &mut self.seat {
                        let _ = seat.complete(id, false);
                    }
                }
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
                api::host_log!(
                    api::hostlog::Category::ScriptLifecycle,
                    api::hostlog::Level::Info,
                    "script requested stop tick={} reason={}",
                    receipt.tick,
                    receipt.reason
                );
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
        self.pending_compiled_start = None;
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

#[cfg(test)]
#[path = "slot_tests.rs"]
mod tests;

#[cfg(all(test, feature = "load"))]
#[path = "slot/seat_wire_tests.rs"]
mod seat_wire_tests;

#[cfg(all(test, feature = "load"))]
#[path = "slot/seat_family_tests.rs"]
mod seat_family_tests;
