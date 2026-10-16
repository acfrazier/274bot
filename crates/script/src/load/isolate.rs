//! LoadIsolate: rustyscript V8 on its own thread (feature `load` only).

mod supervisor;
mod teardown;
mod thread;
mod validation;
pub use supervisor::{capacity_status as isolate_capacity_status, IsolateCapacityStatus};
pub use validation::{ValidationBatch, ValidationError};

pub use teardown::TeardownProof;
use teardown::{TeardownPhase, TeardownState};
use thread::isolate_main;
#[cfg(test)]
use thread::{stamp_mouse_gesture_identities, MouseGestureIdentities};

use super::bindings::wire_runtime;
use super::shape::LoadShape;
use super::snapshot::{
    dispatch_native_events, key_string, materialize_settings_bag, materialize_snapshot,
};
use super::{loadout_v8, machine_v8, paint_chrome, paint_jive, reach_query, run_policy_v8, shape};
use rustyscript::{json_args, Runtime, RuntimeOptions};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// A failed isolate may poison a shared lock before the supervisor can
/// retire it. Keep its state available for teardown rather than propagating
/// that isolate-local failure to the process-wide supervisor.
fn lock_after_isolate_failure<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poison| poison.into_inner())
}

/// Failures before an isolate thread has been created.
#[derive(Debug, thiserror::Error)]
pub enum LoadIsolateError {
    /// The embedder did not initialize V8 at its process root.
    #[error("V8 platform is not bootstrapped: call script::bootstrap_v8_platform() on main before creating any JavaScript runtime worker threads")]
    PlatformNotBootstrapped,
    #[error("V8 platform bootstrap must run on the process main thread")]
    PlatformNotMainThread,
    #[error("shared isolate bridge: {0}")]
    Bridge(String),
    #[error("JS deadline supervisor: {0}")]
    Supervisor(String),
    #[error("JS deadline supervisor is at capacity")]
    SupervisorCapacity,
    #[error("JS quarantine full (A={quarantined}, retiring J={retiring_gameplay}, retiring V={retiring_validation}); wait for actual isolate join")]
    QuarantineFull {
        quarantined: usize,
        retiring_gameplay: usize,
        retiring_validation: usize,
    },
    #[error("JS isolate admission at process cap (active={active}, retiring J={retiring_gameplay}, retiring V={retiring_validation})")]
    IsolateCapacity {
        active: usize,
        retiring_gameplay: usize,
        retiring_validation: usize,
    },
    /// The operating system refused to create the isolate thread.
    #[error("isolate thread: {0}")]
    ThreadSpawn(#[source] std::io::Error),
}

static NEXT_PAINT_GENERATION: AtomicU64 = AtomicU64::new(1);
static LIVE_RUNTIMES: AtomicUsize = AtomicUsize::new(0);

/// V8 runtimes currently owned by isolate threads. Tests use this to prove a
/// cancelled non-yielding startup was destroyed rather than merely detached.
#[doc(hidden)]
#[cfg(test)]
pub fn live_runtime_count() -> usize {
    LIVE_RUNTIMES.load(std::sync::atomic::Ordering::Acquire)
}

/// Runaway bound shared by the dispatch watchdog and Pause's one-shot.
/// One normal game tick lets legitimate slow work finish while still
/// bounding a non-yielding execution when Pause stops future dispatch.
const EXECUTION_WATCHDOG_HORIZON: Duration = Duration::from_millis(600);
/// The exactly-once onStop getter/body/log drain keeps its independent,
/// deliberately short teardown budget.
const ON_STOP_DEADLINE: Duration = Duration::from_millis(50);
/// `in_flight` tick id for a live `recoveryAnchor` eval (not a game tick).
const RECOVERY_ANCHOR_TICK: u64 = u64::MAX;
/// Hard stop for yielding JS (rustyscript `RuntimeOptions.timeout`).
const RUNTIME_TIMEOUT: Duration = Duration::from_millis(50);
/// How long setup waits for V8 creation and startup evaluation before the
/// owning isolate handle interrupts it.
const SETUP_TIMEOUT: Duration = Duration::from_secs(10);
/// Validation uses a shorter independent deadline because it is a
/// throwaway candidate check, not a live Start.
const VALIDATION_TIMEOUT: Duration = Duration::from_secs(2);
/// How long `join` waits for the isolate thread after Stop + terminate
/// before abandoning it: a stuck isolate must never freeze the caller.
const JOIN_TIMEOUT: Duration = Duration::from_secs(2);
/// The explicit stack reservation of each admitted isolate thread.
const ISOLATE_STACK: usize = 2 * 1024 * 1024;
/// Heap cap for the isolate (~64 MB, the brief's number).
const MAX_HEAP: usize = 64 * 1024 * 1024;
/// Bounded native pairing metadata for mouse gestures produced by one
/// isolate. Overflow invalidates every unmatched pair rather than letting
/// a later up inherit a newer gesture's identity.
const MAX_MOUSE_GESTURES: usize = 32;
/// Commands the host may leave unconsumed in the isolate's channel before
/// it stops queueing ticks and snapshot deltas (about 19 s of game ticks at
/// one snapshot plus one tick each, well past the setup deadline). Past it
/// the isolate thread is wedged: a queued tick would only be skipped as
/// stale, and a dropped delta is recovered by the slot's next keyframe.
/// Operator commands (Pause, Stop, probes, paint input) are never dropped.
const MAX_QUEUED_COMMANDS: usize = 64;

struct SnapshotMessage {
    bytes: Vec<u8>,
    #[cfg(feature = "memory-profile")]
    _lease: crate::memory_profile::SnapshotLease,
}
#[derive(Debug, Clone, PartialEq)]
pub struct QueuedInteract {
    /// Legacy gameplay or lifecycle request.
    pub req: crate::shim::InteractReq,
    /// Push-time walk outcome sequence shared by this physical batch.
    pub observed_walk_outcome_seq: u64,
}
/// Wait facts that a guardian-held seat batch carries as one exact summary.
///
/// The watchdog adds one for each enqueue and subtracts one, saturating at
/// zero, for each settle, in event order (`watchdog.rs` `on_wait_enqueued`
/// and `on_wait_settled`). Any sequence of those steps maps a starting count
/// `c` to `max(c + d, m)` with `d <= m` and `m >= 0`. This type keeps that map
/// in its normal form: `settles` saturating subtractions, then `enqueues`
/// additions (`m = enqueues`, `d = enqueues - settles`). The summary has a
/// fixed size for any hold length.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct HeldWaitFacts {
    /// Leading `WaitSettled` steps (`m - d`).
    pub settles: u64,
    /// Trailing `WaitEnqueued` steps (`m`).
    pub enqueues: u64,
    /// At least one wait fact was folded. Each fact stamps the scheduler.
    pub stamped: bool,
}

impl HeldWaitFacts {
    /// Fold one `WaitEnqueued` after the summarized facts.
    pub fn enqueue(&mut self) {
        self.enqueues = self.enqueues.saturating_add(1);
        self.stamped = true;
    }

    /// Fold one `WaitSettled` after the summarized facts. A settle after an
    /// enqueue cancels it exactly: `max(x + 1 - 1, 0) = x` for `x >= 0`.
    pub fn settle(&mut self) {
        if self.enqueues > 0 {
            self.enqueues -= 1;
        } else {
            self.settles = self.settles.saturating_add(1);
        }
        self.stamped = true;
    }

    /// The summary of these facts followed by `later`.
    pub fn then(self, later: Self) -> Self {
        let cancelled = self.enqueues.min(later.settles);
        Self {
            settles: self.settles.saturating_add(later.settles - cancelled),
            enqueues: (self.enqueues - cancelled).saturating_add(later.enqueues),
            stamped: self.stamped || later.stamped,
        }
    }

    /// The rows that reproduce the summarized facts on the watchdog: the
    /// settles, then the enqueues. When the facts cancel to the identity map,
    /// one enqueue and settle pair keeps the scheduler stamp and leaves the
    /// count unchanged.
    pub fn replay(self) -> impl Iterator<Item = crate::shim::InteractReq> {
        let pair = u64::from(self.stamped && self.settles == 0 && self.enqueues == 0);
        std::iter::repeat_n(
            crate::shim::InteractReq::WaitSettled,
            row_count(self.settles),
        )
        .chain(std::iter::repeat_n(
            crate::shim::InteractReq::WaitEnqueued,
            row_count(self.enqueues + pair),
        ))
        .chain(std::iter::repeat_n(
            crate::shim::InteractReq::WaitSettled,
            row_count(pair),
        ))
    }

    /// Number of rows that [`Self::replay`] yields.
    pub fn rows(&self) -> u64 {
        let pair = u64::from(self.stamped && self.settles == 0 && self.enqueues == 0);
        self.settles
            .saturating_add(self.enqueues)
            .saturating_add(2 * pair)
    }
}

fn row_count(rows: u64) -> usize {
    usize::try_from(rows).unwrap_or(usize::MAX)
}

#[derive(Debug)]
pub struct QueuedSeatBatch {
    /// The isolate session-work generation that produced this batch.
    pub generation: u64,
    /// Push-time walk outcome sequence shared by this physical batch.
    pub observed_walk_outcome_seq: u64,
    /// Decoded typed controls and gameplay rows.
    pub batch: crate::isolate_fb::DecodedSeatBatch,
    /// Local machine phases and terminals from this same producer turn.
    pub(crate) machine_events: Vec<crate::machine::MachineEvent>,
    /// Wait facts folded out of `batch.reqs` while the seat was held.
    pub held_waits: HeldWaitFacts,
}

impl QueuedSeatBatch {
    /// Rows this batch carries, with each folded wait fact counted as one row.
    pub fn rows(&self) -> usize {
        self.batch
            .reqs
            .len()
            .saturating_add(usize::try_from(self.held_waits.rows()).unwrap_or(usize::MAX))
    }
}

impl std::borrow::Borrow<crate::shim::InteractReq> for QueuedInteract {
    fn borrow(&self) -> &crate::shim::InteractReq {
        &self.req
    }
}

enum IsolateCmd {
    Tick {
        tick: u64,
        generation: u64,
        input_identity: u64,
        evidence_sequence: u64,
        wait_only: bool,
    },
    /// A session boundary. `keep_work`: hold the script's work for the next
    /// logged-in session instead of ending it.
    ResetSession {
        keep_work: bool,
        /// The work generation this reset opened: only the reset of the
        /// current generation restates the parked waits.
        generation: u64,
        /// The slot-owned RunKey for the new connection/session.
        execution: Option<api::identity::ExecutionId>,
        /// Walk rows discarded before host delivery. Their live waits must
        /// request the route again through the new session's seat.
        undelivered_walks: Vec<u64>,
    },
    /// An old-generation tick finished after the reset had already drained
    /// the host queue. Re-arm its discarded wait on the isolate thread.
    RearmDiscardedWalk(u64),
    /// The F0 owner is ready after a kept boundary: re-stamp pending starts
    /// with its session without resetting the isolate's retained work.
    AlignExecution(api::identity::ExecutionId),
    /// Latch manual movement before a queued Pause, even if the snapshot is pending.
    ManualWalkTakeover(u64),
    /// The host's FlatBuffer snapshot blob (schema: `crates/script/
    /// schema/isolate.fbs`), decoded on the isolate thread into the
    /// JS object the Game/Inventory/Skills/EventSignal shims read
    /// before the next dispatched tick. Never a JSON string.
    Snapshot(SnapshotMessage),
    /// Play-owned BroadcastChannel deliveries, encoded as an InteractBatch
    /// on the existing FlatBuffer wire.
    Channel(Vec<u8>),
    /// Merged operator settings bag for the prelude's `this.settings.*`,
    /// built on the isolate thread as V8 values (never JSON text).
    Settings(serde_json::Map<String, serde_json::Value>),
    /// Available loadouts. Rust keeps them for `selectedLoadout`.
    Loadouts(Vec<api::loadout::Loadout>),
    /// The slot's recovery hints, which outlive this isolate.
    RecoveryHints(std::sync::Arc<super::RecoveryHintsCell>),
    Pause,
    Resume,
    /// One-shot script-local paint button, tagged with the isolate
    /// work generation so a stale overlay cannot land on a later script.
    PaintClick {
        id: String,
        generation: u64,
    },
    /// Persistent strip/rail/tabs selection, tagged with the isolate
    /// work generation so a stale overlay cannot land on a later script.
    PaintSelect {
        key: String,
        name: String,
        generation: u64,
    },
    /// Generation-bound recoveryAnchor sample. Evaluated on this
    /// thread with the shared one-game-tick runaway bound; the reply is a
    /// FlatBuffer interact.
    RecoveryAnchor {
        generation: u64,
    },
    Probe(String, Sender<Result<serde_json::Value, String>>),
    /// Isolate teardown. `invoke_hook` is true for operator join and
    /// self-stop; raw Drop sends false so a dying isolate never runs
    /// `onStop` into a dropped rx.
    Stop {
        invoke_hook: bool,
    },
}

/// The isolate thread's end of the command channel. Every receive
/// releases one slot of the host's [`MAX_QUEUED_COMMANDS`] backlog.
struct CmdQueue {
    rx: Receiver<IsolateCmd>,
    queued: std::sync::Arc<AtomicUsize>,
}

impl CmdQueue {
    fn recv(&self) -> Option<IsolateCmd> {
        let cmd = self.rx.recv().ok()?;
        self.queued
            .fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
        Some(cmd)
    }

    fn try_recv(&self) -> Option<IsolateCmd> {
        let cmd = self.rx.try_recv().ok()?;
        self.queued
            .fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
        Some(cmd)
    }
}

enum ThreadMsg {
    Log(String),
    /// The isolate thread processed the session reset that opened
    /// `generation`: every tick of the previous connection has finished.
    SessionReset {
        generation: u64,
    },
    /// A diagnostic emitted while processing one tick. The host keeps this
    /// separate from user-authored log lines so a later user message cannot
    /// clear or create the slot's active tick error.
    TickError {
        tick: u64,
        generation: u64,
        message: String,
    },
    /// A watchdog/Pause/session terminate cut script JS. The owning slot
    /// must recreate the isolate; in-place single-flight recovery is unsafe.
    ScriptCut,
    /// The tick's shim interact queue (`__rs2b0t_host.interact`), a
    /// FlatBuffer `InteractBatch` of [`crate::shim::InteractReq`]s
    /// forwarded after the tick's JS finished (parked or not).
    Interact {
        bytes: Vec<u8>,
        generation: u64,
        observed_walk_outcome_seq: u64,
    },
    /// The tick's typed I-seat producer batch, decoded only on the host side.
    SeatBatch {
        bytes: Vec<u8>,
        generation: u64,
        observed_walk_outcome_seq: u64,
        machine_events: Vec<crate::machine::MachineEvent>,
    },
    IgnoredRandoms(Vec<String>),
    /// The tick whose work has fully finished. `report_errors` is false for
    /// operator Pause cancellation: termination is not a script diagnostic.
    Completed {
        tick: u64,
        generation: u64,
        successful: bool,
        report_errors: bool,
    },
    /// ScriptRunner.stop ended this isolate with its bounded script reason.
    ScriptStopped {
        tick: u64,
        reason: String,
    },
    /// ScriptRunner.stop ended this isolate lifetime.
    Stopped,
    /// A non-tick isolate command finished; clear matching `in_flight`.
    InFlightDone {
        generation: u64,
        tick: u64,
    },
    /// The tick's recorded paint frame. Both ends are this crate in this
    /// process, so the frame crosses as the typed value the recorder built:
    /// no FlatBuffer encode/verify/decode, and every reader shares one frame.
    Paint(std::sync::Arc<crate::shim::ScriptPaint>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TickOutcome {
    Success {
        tick: u64,
        generation: u64,
    },
    Error {
        tick: u64,
        generation: u64,
        message: String,
    },
}

/// One JS bot running in its own rustyscript/V8 isolate. Spawned only
/// on Start; the Runtime lives on the thread and is reached through a
/// command channel, so the host never blocks on JS. Ticks run on
/// observed game-tick edges; stale ticks are skipped.
pub struct LoadIsolate {
    #[cfg(feature = "memory-profile")]
    counters: std::sync::Arc<crate::memory_profile::Counters>,
    #[cfg(feature = "memory-profile")]
    last_completed: std::sync::atomic::AtomicU64,
    #[cfg(feature = "memory-profile")]
    dispatched: std::sync::atomic::AtomicU64,
    work_generation: std::sync::Arc<std::sync::atomic::AtomicU64>,
    walking_ownership: std::sync::Arc<crate::machine::WalkingOwnership>,
    /// Rust-only Load journal lease shared with the isolate thread. Native
    /// v2 tick cards and compatibility cards use the same journal adapter.
    compat_journal: Option<std::sync::Arc<crate::quest_journal::CompatJournalLease>>,
    /// Host-owned identity of forwarded paint frames. Unique per spawn
    /// and bumped on session reset so a stale overlay generation cannot
    /// match a later isolate that advertises the same button id. Shared
    /// with the isolate thread, which stamps the frame it forwards.
    paint_generation: std::sync::Arc<std::sync::atomic::AtomicU64>,
    stopped: std::sync::atomic::AtomicBool,
    /// One bounded, non-consuming terminal receipt for ScriptRunner.stop.
    script_stop: Mutex<Option<ScriptStopReceipt>>,
    tx: Sender<IsolateCmd>,
    /// Commands sent on `tx` and not yet received by the isolate thread.
    /// Bounds the channel: see [`MAX_QUEUED_COMMANDS`].
    queued: std::sync::Arc<AtomicUsize>,
    /// The latest snapshot post was refused; the next tick is refused with
    /// it, keeping each tick paired with its scene.
    snapshot_refused: AtomicBool,
    rx: Mutex<Receiver<ThreadMsg>>,
    logs: Mutex<Vec<String>>,
    /// Typed diagnostics collected while each tick runs. Kept separate from
    /// user-authored `Log` messages so slot state cannot be driven by text.
    tick_errors: Mutex<HashMap<(u64, u64), Vec<String>>>,
    /// Completed tick outcomes waiting for the owning slot to consume them.
    tick_outcomes: Mutex<Vec<TickOutcome>>,
    /// Generation with a queued active-error outcome. Successful ticks are
    /// reported only while this is set, avoiding an allocation per tick.
    tick_outcome_error_generation: Mutex<Option<u64>>,
    /// Set when the isolate confirms that a terminate cut script JS.
    /// The owning slot consumes this once and recreates the runtime.
    script_cut: AtomicBool,
    /// Interact requests forwarded by the tick thread (the shim
    /// `Bank`/`Banking` queue), drained by the host like logs.
    interacts: Mutex<Vec<QueuedInteract>>,
    /// Watchdog lifecycle facts are separate from gameplay interactions.
    lifecycle: Mutex<Vec<crate::shim::InteractReq>>,
    /// Ordered typed seat batches, each retaining its source generation and
    /// one push-time walk outcome stamp for the physical wire batch.
    seat_batches: Mutex<VecDeque<QueuedSeatBatch>>,
    /// AbortWalk release rows from the dropped connection, held until the
    /// isolate has processed the reconnect reset and then drained normally.
    held_releases: Mutex<Vec<QueuedInteract>>,
    /// A reconnect holds release rows from stale generation batches until
    /// the reset fence is processed; gameplay rows are discarded.
    holding_releases: AtomicBool,
    /// The generation of a reconnect reset the isolate thread has not
    /// processed yet, so stale release rows are not exposed before then.
    held_fence: Mutex<Option<u64>>,
    /// The latest paint frame the tick thread forwarded (the
    /// [`crate::shim::ScriptPaint`] the recorder built after each tick),
    /// shared with every reader instead of copied per read.
    paint: Mutex<Option<std::sync::Arc<crate::shim::ScriptPaint>>>,
    /// The bot instance's random-ignore list, forwarded by the tick
    /// thread whenever it changes (same source as the old probe path).
    ignored_randoms: Mutex<Vec<String>>,
    handle: Option<JoinHandle<()>>,
    /// Thread-safe handle used to terminate a runaway tick from this
    /// side of the channel. Empty until [`LoadIsolate::poll_ready`] sees
    /// setup finish. The terminate stays armed until the isolate thread
    /// has returned from the tick and clears it there (a cancel from this
    /// side would race the interrupt and make it a no-op).
    terminate: OnceLock<v8::IsolateHandle>,
    /// Setup receiver plus deadline; resolved from the slot observe.
    setup: Mutex<SetupState>,
    /// The tick currently being dispatched and when it was sent; the
    /// thread clears it when the tick completes.
    in_flight: Mutex<Option<(u64, u64, Instant)>>,
    /// Shared Stop/`onStop` phase. Join, Drop, and the isolate thread
    /// serialize tick-unwind vs hook vs Done under this mutex.
    teardown: std::sync::Arc<Mutex<TeardownState>>,
    /// Per-isolate invoke/finish/worker evidence. Clone before Drop.
    proof: TeardownProof,
}

/// ScriptRunner.stop receipt copied off the isolate thread. This is a
/// single bounded value, independent of the panel's destructive log queue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptStopReceipt {
    pub tick: u64,
    pub reason: String,
}

/// Non-blocking isolate boot result. Commands already queue until setup
/// finishes, so the host never waits on V8 creation on the caller thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ready {
    Pending,
    Ready,
    Failed(String),
}

enum SetupMessage {
    /// Published immediately after Runtime creation, before any
    /// user-influenced startup evaluation.
    Interrupt(v8::IsolateHandle),
    /// Final setup result after wiring the runtime and loading the module.
    Ready(Result<(), String>),
}

enum SetupState {
    Pending {
        rx: Receiver<SetupMessage>,
        deadline: Instant,
    },
    Ready,
    Failed(String),
}

/// The only production gameplay construction path is the slot-owned factory.
/// UI and control callers cannot create an isolate with this crate-private type.
pub(crate) struct GameplayFactory;

impl GameplayFactory {
    pub(crate) fn spawn_with_content(
        js: String,
        shape: LoadShape,
        siblings: Vec<(String, String)>,
        game_data: Option<std::sync::Arc<api::game_data::SelectedGameData>>,
        named_banks: std::sync::Arc<api::named_banks::NamedBankFacts>,
        execution: Option<api::identity::ExecutionId>,
    ) -> Result<LoadIsolate, LoadIsolateError> {
        LoadIsolate::spawn_inner(js, shape, siblings, game_data, named_banks, execution)
    }
}

impl LoadIsolate {
    /// Spawn the isolate thread with already-cached JS (no transpile).
    /// Returns as soon as the thread is created; V8 setup is resolved by
    /// [`LoadIsolate::poll_ready`]. Missing process-root
    /// [`crate::bootstrap_v8_platform`] or thread-create failure is returned
    /// synchronously, before any runtime worker is started.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn spawn(
        js: String,
        shape: LoadShape,
        siblings: Vec<(String, String)>,
    ) -> Result<Self, LoadIsolateError> {
        Self::spawn_inner(
            js,
            shape,
            siblings,
            None,
            std::sync::Arc::new(api::named_banks::NamedBankFacts::empty()),
            None,
        )
    }

    /// Spawn a test isolate that owns a native seat execution.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn spawn_with_execution(
        js: String,
        shape: LoadShape,
        siblings: Vec<(String, String)>,
        execution: api::identity::ExecutionId,
    ) -> Result<Self, LoadIsolateError> {
        Self::spawn_inner(
            js,
            shape,
            siblings,
            None,
            std::sync::Arc::new(api::named_banks::NamedBankFacts::empty()),
            Some(execution),
        )
    }
    /// Spawn with immutable selected-revision facts. The Arc is shared
    /// until the isolate's one startup publication is complete.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn spawn_with_game_data(
        js: String,
        shape: LoadShape,
        siblings: Vec<(String, String)>,
        game_data: std::sync::Arc<api::game_data::SelectedGameData>,
    ) -> Result<Self, LoadIsolateError> {
        Self::spawn_inner(
            js,
            shape,
            siblings,
            Some(game_data),
            std::sync::Arc::new(api::named_banks::NamedBankFacts::empty()),
            None,
        )
    }

    /// Spawn with selected-revision facts and named aliases.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn spawn_with_content(
        js: String,
        shape: LoadShape,
        siblings: Vec<(String, String)>,
        game_data: Option<std::sync::Arc<api::game_data::SelectedGameData>>,
        named_banks: std::sync::Arc<api::named_banks::NamedBankFacts>,
    ) -> Result<Self, LoadIsolateError> {
        Self::spawn_inner(js, shape, siblings, game_data, named_banks, None)
    }

    /// Queue a whole refresh on the single process ValidationSeat. The
    /// receiver owns the original cards, so the UI can fence the result.
    pub fn validate_prepared_batch(
        cards: Vec<super::library::PreparedCard>,
    ) -> Result<ValidationBatch, ValidationError> {
        validation::validate_batch(cards)
    }

    fn spawn_inner(
        js: String,
        shape: LoadShape,
        siblings: Vec<(String, String)>,
        game_data: Option<std::sync::Arc<api::game_data::SelectedGameData>>,
        named_banks: std::sync::Arc<api::named_banks::NamedBankFacts>,
        execution: Option<api::identity::ExecutionId>,
    ) -> Result<Self, LoadIsolateError> {
        super::platform::require_v8_platform()?;
        let supervisor = supervisor::get()?;
        let permit = supervisor.admit()?;
        let runtime_handle = super::runtime_bridge::handle()?;
        let (tx, rx) = mpsc::channel::<IsolateCmd>();
        let (setup_tx, setup_rx) = mpsc::channel::<SetupMessage>();
        let (msg_tx, msg_rx) = mpsc::channel::<ThreadMsg>();
        #[cfg(feature = "memory-profile")]
        let counters = crate::memory_profile::registered();
        #[cfg(feature = "memory-profile")]
        let thread_counters = counters.clone();
        let work_generation = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
        let thread_generation = work_generation.clone();
        let compat_journal = (shape != LoadShape::Reject)
            .then(|| std::sync::Arc::new(crate::quest_journal::CompatJournalLease::default()));
        let thread_compat_journal = compat_journal.clone();
        let teardown = std::sync::Arc::new(Mutex::new(TeardownState::new()));
        let thread_teardown = teardown.clone();
        let proof = TeardownProof::new();
        let thread_proof = proof.inner.clone();
        let paint_generation = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(
            NEXT_PAINT_GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        ));
        let thread_paint_generation = paint_generation.clone();
        let queued = std::sync::Arc::new(AtomicUsize::new(0));
        let thread_queued = queued.clone();
        let walking_ownership = std::sync::Arc::new(crate::machine::WalkingOwnership::default());
        let thread_walking_ownership = std::sync::Arc::clone(&walking_ownership);
        let handle = std::thread::Builder::new()
            .name("js-isolate".into())
            .stack_size(ISOLATE_STACK)
            .spawn(move || {
                let _permit = permit;
                isolate_main(
                    js,
                    shape,
                    siblings,
                    game_data,
                    named_banks,
                    runtime_handle,
                    CmdQueue {
                        rx,
                        queued: thread_queued,
                    },
                    msg_tx,
                    setup_tx,
                    thread_generation,
                    thread_walking_ownership,
                    thread_compat_journal,
                    thread_paint_generation,
                    thread_teardown,
                    thread_proof,
                    execution,
                    #[cfg(feature = "memory-profile")]
                    thread_counters,
                )
            })
            .map_err(LoadIsolateError::ThreadSpawn)?;
        Ok(LoadIsolate {
            #[cfg(feature = "memory-profile")]
            counters,
            #[cfg(feature = "memory-profile")]
            last_completed: std::sync::atomic::AtomicU64::new(0),
            #[cfg(feature = "memory-profile")]
            dispatched: std::sync::atomic::AtomicU64::new(0),
            work_generation,
            walking_ownership,
            compat_journal,
            paint_generation,
            stopped: std::sync::atomic::AtomicBool::new(false),
            script_stop: Mutex::new(None),
            tx,
            queued,
            snapshot_refused: AtomicBool::new(false),
            rx: Mutex::new(msg_rx),
            logs: Mutex::new(Vec::new()),
            tick_errors: Mutex::new(HashMap::new()),
            tick_outcomes: Mutex::new(Vec::new()),
            tick_outcome_error_generation: Mutex::new(None),
            script_cut: AtomicBool::new(false),
            interacts: Mutex::new(Vec::new()),
            seat_batches: Mutex::new(VecDeque::new()),
            lifecycle: Mutex::new(Vec::new()),
            held_releases: Mutex::new(Vec::new()),
            holding_releases: AtomicBool::new(false),
            held_fence: Mutex::new(None),
            paint: Mutex::new(None),
            ignored_randoms: Mutex::new(Vec::new()),
            handle: Some(handle),
            terminate: OnceLock::new(),
            setup: Mutex::new(SetupState::Pending {
                rx: setup_rx,
                deadline: Instant::now() + SETUP_TIMEOUT,
            }),
            in_flight: Mutex::new(None),
            teardown,
            proof,
        })
    }

    /// Post the host's FlatBuffer snapshot blob into the isolate: the
    /// buffer is decoded on the isolate thread into the JS object on
    /// the host handle (`__rs2b0t_host.snapshot`) before the next
    /// dispatched tick, so the Game/Inventory/Skills/EventSignal shims
    /// read the fields the host observed this PLAYER_INFO. Only these
    /// fields are copied — no World clone. Commands are serialized on
    /// the isolate thread, so a post followed by
    /// [`LoadIsolate::on_game_tick`] reaches JS in that order.
    ///
    /// `false`: the isolate thread has left [`MAX_QUEUED_COMMANDS`]
    /// unconsumed and the post was dropped. A dropped delta leaves the
    /// isolate's copy behind, so the caller's next post must be a keyframe,
    /// and the next tick is refused too: it would run against the scene the
    /// dropped post was meant to replace.
    pub fn post_snapshot(&self, bytes: Vec<u8>) -> bool {
        let refused = self.backlogged();
        self.snapshot_refused
            .store(refused, std::sync::atomic::Ordering::Release);
        if refused {
            return false;
        }
        let message = SnapshotMessage {
            #[cfg(feature = "memory-profile")]
            _lease: crate::memory_profile::SnapshotLease::new(
                self.counters.clone(),
                bytes.len(),
                bytes.capacity(),
            ),
            bytes,
        };
        self.send(IsolateCmd::Snapshot(message));
        true
    }

    /// The isolate thread has not consumed [`MAX_QUEUED_COMMANDS`] commands.
    fn backlogged(&self) -> bool {
        self.queued.load(std::sync::atomic::Ordering::Acquire) >= MAX_QUEUED_COMMANDS
    }

    /// Queue one command, counted until the isolate thread receives it.
    /// `false`: the thread is gone and nothing will receive it.
    fn send(&self, cmd: IsolateCmd) -> bool {
        self.queued
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        if self.tx.send(cmd).is_ok() {
            return true;
        }
        self.queued
            .fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
        false
    }

    /// Post available loadouts before subsequent tick commands.
    pub fn post_loadouts(&self, loadouts: &[api::loadout::Loadout]) {
        self.send(IsolateCmd::Loadouts(loadouts.to_vec()));
    }
    /// Deliver a broker batch without allowing it to grow the isolate command
    /// queue without bound. `false` means the recipient is stale or backlogged.
    pub fn post_channel_events(&self, bytes: Vec<u8>) -> bool {
        if self.backlogged()
            || self.stopped.load(std::sync::atomic::Ordering::Acquire)
            || self.teardown_blocks_dispatch()
        {
            return false;
        }
        self.send(IsolateCmd::Channel(bytes))
    }

    /// Use the slot's frozen `RecoveryHints` for this isolate, so a watchdog
    /// restart finds what the previous run latched.
    pub fn post_recovery_hints(&self, hints: std::sync::Arc<super::RecoveryHintsCell>) {
        self.send(IsolateCmd::RecoveryHints(hints));
    }

    /// Post the merged operator settings bag (schema defaults + panel/TUI
    /// overrides + optional scenario inject). The prelude's
    /// `this.settings.*` reads `__rs2b0t_host.settingsBag`.
    pub fn post_settings_bag(&self, bag: &serde_json::Map<String, serde_json::Value>) {
        self.send(IsolateCmd::Settings(bag.clone()));
    }

    /// Dispatch one observed game tick to the isolate. The previous
    /// tick is checked against [`EXECUTION_WATCHDOG_HORIZON`]: an execution
    /// still running past it is interrupted and logged, and its stale ticks
    /// are skipped.
    pub fn on_game_tick(&self, snap_tick: u64) {
        self.on_game_tick_at(snap_tick, 0);
    }

    /// Dispatch one observed game tick, tagging produced mouse rows with
    /// the native permit identity that was live at production.
    pub fn on_game_tick_at(&self, snap_tick: u64, input_identity: u64) {
        self.dispatch_tick(snap_tick, input_identity, 0, false);
    }

    /// Recheck parked waits against a changed snapshot without advancing
    /// game-tick listeners, machines, or the script's loop.
    /// `evidence_sequence` comes from actual producer field changes; native
    /// input identity remains the separate interaction authority fence.
    pub fn on_snapshot_change_at(
        &self,
        snap_tick: u64,
        input_identity: u64,
        evidence_sequence: u64,
    ) {
        self.dispatch_tick(snap_tick, input_identity, evidence_sequence, true);
    }

    /// Record a producer delta that carries changed evidence.
    pub(crate) fn record_snapshot_evidence_change(&self, changed: bool) {
        if changed {
            let mut state = self.teardown.lock().unwrap();
            state.snapshot_evidence_sequence = state.snapshot_evidence_sequence.wrapping_add(1);
        }
    }

    /// Current producer evidence generation, independent of native identity.
    pub(crate) fn snapshot_evidence_sequence(&self) -> u64 {
        self.teardown.lock().unwrap().snapshot_evidence_sequence
    }

    fn dispatch_tick(
        &self,
        snap_tick: u64,
        input_identity: u64,
        evidence_sequence: u64,
        wait_only: bool,
    ) {
        // Before Ready no tick has started, so none can be over budget.
        let ready = self.poll_ready() == Ready::Ready;
        self.pump_logs();
        if self.stopped.load(std::sync::atomic::Ordering::Acquire)
            || self.teardown_blocks_dispatch()
        {
            return;
        }
        let generation = self
            .work_generation
            .load(std::sync::atomic::Ordering::Acquire);
        // A wedged thread would only skip this tick as stale, and a tick
        // whose snapshot was refused would run on the previous scene: do
        // not queue it, and keep the oldest unfinished tick in flight for
        // the budget.
        let backlogged = self
            .snapshot_refused
            .swap(false, std::sync::atomic::Ordering::AcqRel)
            || self.backlogged();
        self.interrupt_slow_execution(ready);
        if !backlogged {
            *self.in_flight.lock().unwrap() = Some((generation, snap_tick, Instant::now()));
        }
        if backlogged {
            return;
        }
        #[cfg(feature = "memory-profile")]
        self.dispatched
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.send(IsolateCmd::Tick {
            tick: snap_tick,
            generation,
            input_identity,
            evidence_sequence,
            wait_only,
        });
    }

    /// Queue a game tick directly, without sampling or arming the host
    /// watchdog. Slow-completion regression tests place these behind the
    /// first tick command to exercise the isolate thread's stale-window
    /// bookkeeping; watchdog interruption has its own tests.
    #[cfg(feature = "test-hooks")]
    #[doc(hidden)]
    pub fn enqueue_tick_without_watchdog_for_test(&self, snap_tick: u64) {
        let generation = self
            .work_generation
            .load(std::sync::atomic::Ordering::Acquire);
        #[cfg(feature = "memory-profile")]
        self.dispatched
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.send(IsolateCmd::Tick {
            tick: snap_tick,
            generation,
            input_identity: 0,
            evidence_sequence: 0,
            wait_only: false,
        });
    }

    /// Interrupt the currently active execution against its own start time,
    /// never the cadence of later host dispatches. If it is still inside its
    /// horizon, arm one execution-scoped sleeper for the remaining time.
    fn interrupt_slow_execution(&self, ready: bool) {
        if !ready {
            return;
        }
        let active = {
            let st = self.teardown.lock().unwrap();
            match (
                st.phase,
                st.execution_active,
                st.execution_interrupt,
                st.execution_started,
            ) {
                (TeardownPhase::Running, true, None, Some(started)) => {
                    Some((st.execution_id, st.execution_tick, started.elapsed()))
                }
                _ => None,
            }
        };
        let Some((execution_id, tick, elapsed)) = active else {
            return;
        };
        if elapsed <= EXECUTION_WATCHDOG_HORIZON {
            self.arm_active_execution_deadline(teardown::ExecutionInterrupt::Watchdog);
            return;
        }
        if self.fire_execution_interrupt(execution_id, teardown::ExecutionInterrupt::Watchdog) {
            let line = if tick == RECOVERY_ANCHOR_TICK {
                format!("interrupted slow recoveryAnchor ({elapsed:?})")
            } else {
                format!("interrupted slow tick {tick} ({elapsed:?})")
            };
            self.logs.lock().unwrap().push(line);
        }
    }

    /// Arm the independent process supervisor against the execution's original
    /// start. Changing Pause/reset ownership never extends that deadline.
    fn arm_active_execution_deadline(&self, owner: teardown::ExecutionInterrupt) {
        let deadline = {
            let mut st = self.teardown.lock().unwrap();
            let (Some(started), Some(handle)) = (st.execution_started, self.terminate.get()) else {
                return;
            };
            if st.phase != TeardownPhase::Running
                || !st.execution_active
                || st.execution_interrupt.is_some()
            {
                return;
            }
            if owner != teardown::ExecutionInterrupt::Watchdog
                || st.execution_deadline_owner.is_none()
            {
                st.execution_deadline_owner = Some(owner);
            }
            if st.execution_deadline_armed {
                return;
            }
            st.execution_deadline_armed = true;
            (
                st.execution_id,
                started + EXECUTION_WATCHDOG_HORIZON,
                handle.clone(),
            )
        };
        let (id, at, handle) = deadline;
        #[cfg(any(test, feature = "test-hooks"))]
        let injected = self.teardown.lock().unwrap().fail_execution_registration;
        #[cfg(not(any(test, feature = "test-hooks")))]
        let injected: Option<teardown::DeadlineRefusal> = None;
        let registration = if let Some(reason) = injected {
            Err(match reason {
                teardown::DeadlineRefusal::Capacity => LoadIsolateError::SupervisorCapacity,
                teardown::DeadlineRefusal::Stopped => {
                    LoadIsolateError::Supervisor("supervisor worker stopped".into())
                }
            })
        } else {
            supervisor::get().and_then(|service| {
                service.arm_execution(self.teardown.clone(), handle, id, owner, at)
            })
        };
        if let Err(error) = registration {
            // No independent owner: interrupt this exact execution immediately.
            // Pause still allows healthy work when registration succeeds; without
            // an independent bound, it cannot promise an unbounded completion.
            let reason = match error {
                LoadIsolateError::SupervisorCapacity => teardown::DeadlineRefusal::Capacity,
                _ => teardown::DeadlineRefusal::Stopped,
            };
            self.fire_execution_interrupt(id, teardown::ExecutionInterrupt::NoDeadline(reason));
            self.logs.lock().unwrap().push(error.to_string());
        }
    }

    #[cfg(feature = "memory-profile")]
    pub fn memory_metrics(&self) -> serde_json::Value {
        self.counters.snapshot()
    }
    #[cfg(feature = "memory-profile")]
    pub fn memory_metrics_handle(&self) -> std::sync::Arc<crate::memory_profile::Counters> {
        self.counters.clone()
    }

    /// Read cached counters only; do not pump messages or probe JS.
    #[cfg(feature = "memory-profile")]
    pub fn memory_progress(&self) -> serde_json::Value {
        use std::sync::atomic::Ordering::Relaxed;
        serde_json::json!({"dispatched":self.dispatched.load(Relaxed),
            "last_completed_tick":self.last_completed.load(Relaxed),
            "paint":self.paint.lock().unwrap().as_ref().map(|p|serde_json::json!({"title":p.title,"lines":p.lines})),
            "in_flight":self.in_flight.lock().unwrap().as_ref().map(|(_,tick,t)|(*tick,t.elapsed().as_millis()))})
    }

    /// Park future tick dispatch. Pause intent shares the execution lock with
    /// entry, so a queued tick cannot enter after this call. Healthy work that
    /// already owns execution is allowed to finish when its independent deadline
    /// is registered; otherwise it is cut immediately rather than run unbounded.
    pub fn pause(&self) {
        let ready = self.poll_ready() == Ready::Ready;
        self.pump_logs();
        self.interrupt_slow_execution(ready);
        {
            let mut st = self.teardown.lock().unwrap();
            if st.phase == TeardownPhase::Running {
                st.pause_requested = true;
            }
        }
        self.arm_active_execution_deadline(teardown::ExecutionInterrupt::Pause);
        self.send(IsolateCmd::Pause);
    }
    /// Active walking family/composer ownership. Reconnects do not carry
    /// gameplay requests from the previous session.
    pub fn live_walking_operation(&self) -> bool {
        self.walking_ownership.live()
    }

    /// Latch manual movement before a queued Pause can resume parked machine rows.
    pub(crate) fn note_manual_walk_takeover(&self, user_move_intent_seq: u64) {
        self.walking_ownership.note_takeover(user_move_intent_seq);
        self.send(IsolateCmd::ManualWalkTakeover(user_move_intent_seq));
    }

    /// Re-arm tick dispatch after [`LoadIsolate::pause`].
    pub fn resume(&self) {
        self.teardown.lock().unwrap().pause_requested = false;
        self.send(IsolateCmd::Resume);
    }

    /// Queue a one-shot paint-button id for the current work generation.
    /// Consumed on the next paint that advertises that id; dropped on
    /// pause, generation skip, or an unadvertised leftover after paint.
    pub fn paint_click(&self, id: &str) {
        if id.is_empty() {
            return;
        }
        let generation = self
            .work_generation
            .load(std::sync::atomic::Ordering::Acquire);
        self.send(IsolateCmd::PaintClick {
            id: id.to_string(),
            generation,
        });
    }

    /// Store a strip/rail/tabs selection for the current work generation.
    /// Rejected on pause or generation skip; kept across paints until
    /// [`LoadIsolate::reset_session_work`].
    pub fn paint_select(&self, key: &str, name: &str) {
        if key.is_empty() || name.is_empty() {
            return;
        }
        let generation = self
            .work_generation
            .load(std::sync::atomic::Ordering::Acquire);
        self.send(IsolateCmd::PaintSelect {
            key: key.to_string(),
            name: name.to_string(),
            generation,
        });
    }

    /// Ask the isolate thread to evaluate `recoveryAnchor()` once for
    /// this work generation. Non-blocking: the reply arrives as a
    /// generation-tagged interact (`recovery-anchor` / `recovery-anchor-none`).
    pub fn request_recovery_anchor(&self) {
        let ready = self.poll_ready() == Ready::Ready;
        if self.stopped.load(std::sync::atomic::Ordering::Acquire)
            || self.teardown_blocks_dispatch()
        {
            return;
        }
        let generation = self
            .work_generation
            .load(std::sync::atomic::Ordering::Acquire);
        self.interrupt_slow_execution(ready);
        *self.in_flight.lock().unwrap() = Some((generation, RECOVERY_ANCHOR_TICK, Instant::now()));
        self.send(IsolateCmd::RecoveryAnchor { generation });
    }

    /// Evaluate `expr` in the isolate's global scope and return its
    /// JSON value (test/status read-back; e.g. `"__rs_bot.n"`).
    pub fn probe(&self, expr: &str) -> Result<serde_json::Value, String> {
        let (tx, rx) = mpsc::channel::<Result<serde_json::Value, String>>();
        if !self.send(IsolateCmd::Probe(expr.to_string(), tx)) {
            return Err("probe: isolate thread gone".into());
        }
        rx.recv_timeout(Duration::from_secs(10))
            .map_err(|e| format!("probe: {e}"))?
    }

    /// The bot instance's random-ignore list (`inst.ignoredRandoms?.()`
    /// on `__rs_bot`, default `[]`): read on the isolate thread each tick
    /// and cached here when it changes (see [`ThreadMsg::IgnoredRandoms`]).
    /// A throwing / non-array method and a native `tick`-shaped card (no
    /// instance) fail closed to `[]`. No probe round-trip.
    pub fn ignored_randoms(&self) -> Vec<String> {
        self.pump_logs();
        self.ignored_randoms.lock().unwrap().clone()
    }

    /// Match the host knock against the cache without cloning its list. This
    /// mirrors `RandomEvents.isIgnored`: Unicode lowercase, no whitespace trim.
    pub(crate) fn ignores_random(&self, name: &str) -> bool {
        self.pump_logs();
        self.ignored_randoms.lock().unwrap().iter().any(|ignored| {
            ignored
                .chars()
                .flat_map(char::to_lowercase)
                .eq(name.chars().flat_map(char::to_lowercase))
        })
    }

    /// Drain the isolate's log lines (tick errors, slow/interrupted
    /// ticks).
    pub fn drain_logs(&self) -> Vec<String> {
        self.pump_logs();
        std::mem::take(&mut *self.logs.lock().unwrap())
    }
    /// Consume the isolate's one cut notification. Only the owning slot calls
    /// this; a standalone isolate remains stopped at the cut continuation.
    pub(crate) fn take_script_cut(&self) -> bool {
        self.pump_logs();
        self.script_cut
            .swap(false, std::sync::atomic::Ordering::AcqRel)
    }

    /// Drain typed outcomes already folded by the caller's regular
    /// [`Self::pump_logs`] pass. User-authored log lines never enter this
    /// queue.
    pub(crate) fn drain_tick_outcomes(&self) -> Vec<TickOutcome> {
        std::mem::take(&mut *self.tick_outcomes.lock().unwrap())
    }

    /// Cached terminal state, refreshed by the regular log drain.
    pub fn stopped(&self) -> bool {
        self.stopped.load(std::sync::atomic::Ordering::Acquire)
    }

    /// Cached ScriptRunner.stop receipt. Reading it never drains logs.
    pub fn script_stop_receipt(&self) -> Option<ScriptStopReceipt> {
        self.pump_logs();
        self.script_stop.lock().unwrap().clone()
    }

    /// Per-isolate teardown evidence. Clone before `join`/`Drop`.
    #[doc(hidden)]
    pub fn teardown_proof(&self) -> TeardownProof {
        self.proof.clone()
    }

    /// Whether the isolate thread owns the current interruptible tick phase.
    #[doc(hidden)]
    pub fn execution_active(&self) -> bool {
        self.teardown.lock().unwrap().execution_active
    }

    /// Monotonic identity and activity of the latest interruptible execution.
    /// The identity remains after completion so lifecycle tests can observe
    /// executions too short to sample while active.
    #[doc(hidden)]
    pub fn execution_sequence(&self) -> (u64, bool) {
        let st = self.teardown.lock().unwrap();
        (st.execution_id, st.execution_active)
    }

    /// Number of per-tick diagnostics still awaiting their matching
    /// completion. Lifecycle regression seam; steady state is zero.
    #[doc(hidden)]
    pub fn pending_tick_error_count(&self) -> usize {
        self.pump_logs();
        self.tick_errors.lock().unwrap().len()
    }

    /// Isolate-scoped seam: delay after the shared deadline is armed and
    /// before getter/body. Proves a late completion cannot cancel it.
    #[doc(hidden)]
    pub fn delay_onstop_after_deadline_arm(&self, delay: Duration) {
        self.teardown.lock().unwrap().hook_entry_delay = Some(delay);
    }

    /// Isolate-scoped seam: refuse supervisor hook registration.
    #[doc(hidden)]
    pub fn fail_onstop_deadline_registration(&self) {
        self.teardown.lock().unwrap().fail_hook_registration = true;
    }

    /// Isolate-scoped deadline refusal seam for capacity/stopped diagnostics.
    #[doc(hidden)]
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn fail_execution_deadline_registration(&self, stopped: bool) {
        self.teardown.lock().unwrap().fail_execution_registration = Some(if stopped {
            teardown::DeadlineRefusal::Stopped
        } else {
            teardown::DeadlineRefusal::Capacity
        });
    }

    /// Give a success-path unit test enough scheduling headroom while
    /// retaining a bounded hook deadline.
    #[cfg(test)]
    pub(crate) fn set_onstop_timeout_for_test(&self, timeout: Duration) {
        self.teardown.lock().unwrap().test_hook_timeout = Some(timeout);
    }

    /// Drain current-generation interactions and any AbortWalk releases
    /// retained across a kept reconnect. Prior-session gameplay is discarded.
    pub fn drain_interacts_stamped(&self) -> Vec<QueuedInteract> {
        self.pump_logs();
        let mut interacts = std::mem::take(&mut *self.interacts.lock().unwrap());
        if self.held_fence.lock().unwrap().is_none() {
            self.holding_releases
                .store(false, std::sync::atomic::Ordering::Release);
            let mut releases = std::mem::take(&mut *self.held_releases.lock().unwrap());
            releases.append(&mut interacts);
            interacts = releases;
        }
        interacts
    }

    /// Current private session-work generation. It is independent of the
    /// slot's run/session stamp and fences isolate-owned output batches.
    pub fn generation(&self) -> u64 {
        self.work_generation
            .load(std::sync::atomic::Ordering::Acquire)
    }

    /// Drain ordered typed seat batches. Callers may restore an unconsumed
    /// suffix after an interrupted host-side dispatch.
    pub fn drain_seat_batches(&self) -> Vec<QueuedSeatBatch> {
        self.pump_logs();
        self.seat_batches.lock().unwrap().drain(..).collect()
    }

    #[cfg(feature = "test-hooks")]
    pub fn seat_batch_backlog(&self) -> usize {
        self.pump_logs();
        self.seat_batches.lock().unwrap().len()
    }

    /// Restore an unconsumed batch prefix ahead of anything that arrived
    /// afterward. Generation and per-batch walk stamps are never rewritten.
    pub(crate) fn restore_seat_batches(&self, mut drained: Vec<QueuedSeatBatch>) {
        self.pump_logs();
        let mut queued = self.seat_batches.lock().unwrap();
        drained.extend(queued.drain(..));
        *queued = drained.into();
    }

    /// Diagnostics/tests only: this drops each row's enqueue stamp and MUST NOT
    /// be used for production walking dispatch.
    pub fn drain_interacts(&self) -> Vec<crate::shim::InteractReq> {
        self.drain_interacts_stamped()
            .into_iter()
            .map(|queued| queued.req)
            .collect()
    }

    /// Put a host-drained batch back in front of requests that arrived
    /// afterward. Used when an operator Pause wins the host's final
    /// dispatch fence: no verb is lost, and Resume observes original order.
    pub(crate) fn restore_interacts_stamped(&self, mut drained: Vec<QueuedInteract>) {
        let mut queued = self.interacts.lock().unwrap();
        drained.append(&mut queued);
        *queued = drained;
    }

    /// Diagnostics/tests only: reconstructs zero stamps. Production restore
    /// MUST use `restore_interacts_stamped`.
    pub(crate) fn restore_interacts(&self, drained: Vec<crate::shim::InteractReq>) {
        self.restore_interacts_stamped(
            drained
                .into_iter()
                .map(|req| QueuedInteract {
                    req,
                    observed_walk_outcome_seq: 0,
                })
                .collect(),
        );
    }

    /// Drop queued canvas mouse rows so Pause/logout cannot replay them.
    pub fn discard_mouse_interacts(&self) {
        self.pump_logs();
        self.interacts
            .lock()
            .unwrap()
            .retain(|queued| !matches!(&queued.req, crate::shim::InteractReq::Mouse { .. }));
    }

    /// Drain generation-matched watchdog lifecycle facts (`note-progress`,
    /// loop/wait settle, recoveryAnchor replies) from the same FlatBuffer
    /// batch. Game interacts stay on [`LoadIsolate::drain_interacts`].
    pub fn drain_lifecycle(&self) -> Vec<crate::shim::InteractReq> {
        self.pump_logs();
        std::mem::take(&mut *self.lifecycle.lock().unwrap())
    }

    /// The session ended deliberately (operator logout, Stop, or removal):
    /// discard work from the previous connection, including batches that an
    /// already running tick has not forwarded yet, and end the script's
    /// in-flight machine rows and task runtimes (their awaits settle
    /// `aborted`). Script state and parked Execution waits survive; the next
    /// snapshot is posted before a new tick. Returns the new work
    /// generation. An active error transfers to that generation so the
    /// first completed clean loop can recover it.
    pub fn reset_session_work(&self, execution: Option<api::identity::ExecutionId>) -> u64 {
        self.reset_session(false, execution)
    }

    /// The connection dropped unexpectedly: discard the connection's gameplay
    /// work like [`LoadIsolate::reset_session_work`], but hold the script's
    /// parked awaits, machine rows, task runtimes and clocks until the next
    /// logged-in session's first tick. AbortWalk releases remain deliverable;
    /// gameplay requests from the dropped session are never replayed.
    pub fn reconnect_session_work(&self, execution: Option<api::identity::ExecutionId>) -> u64 {
        self.reset_session(true, execution)
    }

    /// A host slot discarded a walk after it left the isolate but before
    /// dispatch. The kept script's owning wait must request it in the new
    /// session; the old host-side row must not be replayed.
    pub(crate) fn rearm_host_discarded_walk(&self, token: u64) {
        self.send(IsolateCmd::RearmDiscardedWalk(token));
    }

    pub(crate) fn compat_journal_paint_hidden(&self, now: Instant) -> bool {
        let Some(lease) = self.compat_journal.as_ref() else {
            return false;
        };
        let generation = self
            .work_generation
            .load(std::sync::atomic::Ordering::Acquire);
        lease.live(generation, now)
    }

    fn revoke_compat_journal(&self) {
        if let Some(lease) = &self.compat_journal {
            lease.revoke();
        }
    }

    /// Re-key the pending machine starts after the real host session gate
    /// admits a kept Load run. This command is ordered before the next tick.
    pub(crate) fn align_execution(&self, execution: api::identity::ExecutionId) {
        self.send(IsolateCmd::AlignExecution(execution));
    }

    fn reset_session(&self, keep_work: bool, execution: Option<api::identity::ExecutionId>) -> u64 {
        if let Some(lease) = &self.compat_journal {
            lease.revoke();
            lease.set_host_generation(u64::MAX);
        }
        // No game ticks arrive while disconnected, so preserve the active
        // execution's existing runaway horizon before clearing `in_flight`.
        self.arm_active_execution_deadline(teardown::ExecutionInterrupt::SessionReset);
        // Fold output sent under the ended generation: a kept reconnect
        // retains AbortWalk releases only; gameplay rows are discarded.
        self.pump_logs();
        let had_active_error = self.tick_outcome_error_generation.lock().unwrap().is_some();
        let (generation, undelivered_walks) = {
            let mut interacts = self.interacts.lock().unwrap();
            let mut seat_batches = self.seat_batches.lock().unwrap();
            let undelivered_walks: Vec<_> = if keep_work {
                interacts
                    .iter()
                    .map(|row| &row.req)
                    .chain(
                        seat_batches
                            .iter()
                            .flat_map(|batch| batch.batch.reqs.iter().map(|row| &row.request)),
                    )
                    .filter_map(discarded_walk_id)
                    .take(HELD_RELEASES)
                    .collect()
            } else {
                Vec::new()
            };
            let generation = self
                .work_generation
                .fetch_add(1, std::sync::atomic::Ordering::AcqRel)
                .wrapping_add(1);
            self.paint_generation.store(
                NEXT_PAINT_GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
                std::sync::atomic::Ordering::Release,
            );
            {
                let mut held = self.held_releases.lock().unwrap();
                if keep_work {
                    hold_walk_releases(&mut held, interacts.drain(..));
                    for batch in seat_batches.drain(..) {
                        let observed_walk_outcome_seq = batch.observed_walk_outcome_seq;
                        hold_walk_releases(
                            &mut held,
                            batch.batch.reqs.into_iter().map(move |row| QueuedInteract {
                                req: row.request,
                                observed_walk_outcome_seq,
                            }),
                        );
                    }
                } else {
                    held.clear();
                    seat_batches.clear();
                }
            }
            self.holding_releases
                .store(keep_work, std::sync::atomic::Ordering::Release);
            *self.held_fence.lock().unwrap() = keep_work.then_some(generation);
            interacts.clear();
            self.lifecycle.lock().unwrap().clear();
            (generation, undelivered_walks)
        };
        if let Some(lease) = &self.compat_journal {
            lease.set_host_generation(generation);
        }
        self.tick_errors.lock().unwrap().clear();
        *self.tick_outcome_error_generation.lock().unwrap() =
            had_active_error.then_some(generation);
        self.tick_outcomes.lock().unwrap().clear();
        {
            // Re-stamp the held frame with the new generation: an overlay that
            // captured the pre-reset generation no longer matches it, so a
            // click or select carried over from that frame fails closed until
            // the next forwarded frame. One clone per reset, not per tick.
            let generation = self
                .paint_generation
                .load(std::sync::atomic::Ordering::Acquire);
            let mut slot = self.paint.lock().unwrap();
            if let Some(paint) = slot.as_ref().filter(|p| p.generation != generation) {
                let mut restamped = (**paint).clone();
                restamped.generation = generation;
                *slot = Some(std::sync::Arc::new(restamped));
            }
        }
        *self.in_flight.lock().unwrap() = None;
        self.send(IsolateCmd::ResetSession {
            keep_work,
            generation,
            execution,
            undelivered_walks,
        });
        generation
    }

    /// The latest recorded paint frame (the tick thread forwards the
    /// recorder's frame after every tick that painted, shared, not
    /// copied). `None` when the script has not painted yet. No probe
    /// round-trip — the host reads this every frame.
    pub fn paint(&self) -> Option<std::sync::Arc<crate::shim::ScriptPaint>> {
        self.pump_logs();
        self.paint.lock().unwrap().clone()
    }

    /// Resolve setup without blocking. Called from the slot's per-frame
    /// observe. Commands already queue until this returns [`Ready::Ready`].
    pub fn poll_ready(&self) -> Ready {
        self.resolve_setup(None)
    }

    fn resolve_setup(&self, block_until: Option<Instant>) -> Ready {
        let mut setup = lock_after_isolate_failure(&self.setup);
        let (rx, deadline) = match &*setup {
            SetupState::Ready => return Ready::Ready,
            SetupState::Failed(e) => return Ready::Failed(e.clone()),
            SetupState::Pending { rx, deadline } => (rx, *deadline),
        };
        let outcome = loop {
            let received = match block_until {
                None => rx
                    .try_recv()
                    .map_err(|e| matches!(e, mpsc::TryRecvError::Disconnected)),
                Some(until) => rx
                    .recv_timeout(
                        until
                            .min(deadline)
                            .saturating_duration_since(Instant::now()),
                    )
                    .map_err(|e| matches!(e, mpsc::RecvTimeoutError::Disconnected)),
            };
            match received {
                Ok(SetupMessage::Interrupt(handle)) => {
                    // Setup owns this handle independently of the final
                    // Ready/Failed result. A startup eval can now be
                    // interrupted even if it never returns to send Ready.
                    let _ = self.terminate.set(handle);
                    let st = lock_after_isolate_failure(&self.teardown);
                    if st.phase == TeardownPhase::UnwindingTick {
                        // Serialized with Hook entry: join can never fire
                        // this startup/tick interrupt after onStop begins.
                        self.fire_terminate();
                    }
                }
                Ok(SetupMessage::Ready(Ok(()))) => {
                    // A tick queued before setup finished starts running
                    // now: measure its budget from actual V8 readiness, not
                    // from the send time.
                    if let Some(entry) = lock_after_isolate_failure(&self.in_flight).as_mut() {
                        entry.2 = Instant::now();
                    }
                    break Ready::Ready;
                }
                Ok(SetupMessage::Ready(Err(e))) => break Ready::Failed(e),
                Err(true) => {
                    break Ready::Failed(format!(
                        "isolate init: {}",
                        mpsc::RecvTimeoutError::Disconnected
                    ));
                }
                Err(false) => {
                    let timed_out = Instant::now() >= deadline
                        || block_until.is_some_and(|until| Instant::now() >= until);
                    if !timed_out {
                        return Ready::Pending;
                    }
                    break Ready::Failed(format!(
                        "isolate init: {}",
                        mpsc::RecvTimeoutError::Timeout
                    ));
                }
            }
        };
        *setup = match &outcome {
            Ready::Ready => SetupState::Ready,
            Ready::Failed(e) => SetupState::Failed(e.clone()),
            Ready::Pending => unreachable!("pending returns above"),
        };
        drop(setup);
        if matches!(outcome, Ready::Failed(_)) {
            // A setup deadline or wire failure is still an owned runtime.
            // Serialize termination with Hook entry so a concurrent Stop
            // cannot turn this into an onStop interrupt.
            self.fire_before_hook();
        }
        outcome
    }

    fn fire_terminate(&self) {
        if let Some(handle) = self.terminate.get() {
            handle.terminate_execution();
        }
    }

    /// Fire an eval interrupt only for the exact execution the caller
    /// inspected. The isolate-side finish/cancel uses this same lock, so a
    /// delayed watchdog observation cannot terminate a later tick or onStop.
    fn fire_execution_interrupt(
        &self,
        execution_id: u64,
        owner: teardown::ExecutionInterrupt,
    ) -> bool {
        let mut st = self.teardown.lock().unwrap();
        if st.phase != TeardownPhase::Running
            || !st.execution_active
            || st.execution_id != execution_id
            || st.execution_interrupt.is_some()
        {
            return false;
        }
        st.execution_interrupt = Some(teardown::InterruptedExecution {
            owner,
            // Refused supervision is a terminal cut even if the V8-entry
            // fence has not consumed the terminate request yet. Otherwise a
            // Pause before entry could suppress both the cut and its cause.
            consumed: matches!(owner, teardown::ExecutionInterrupt::NoDeadline(_)),
        });
        self.fire_terminate();
        true
    }

    /// Terminate setup/tick failure while it is still before Hook.
    fn fire_before_hook(&self) {
        let st = lock_after_isolate_failure(&self.teardown);
        if matches!(
            st.phase,
            TeardownPhase::Running | TeardownPhase::UnwindingTick
        ) {
            self.fire_terminate();
        }
    }

    /// Transfer Stop and actual join custody to the one process supervisor.
    /// The slot receives a receipt only after join or credited quarantine.
    pub fn join_detached(self, done: Sender<Vec<String>>) {
        self.retire(true, false, done);
    }

    fn retire(self, invoke_hook: bool, validation: bool, done: Sender<Vec<String>>) {
        self.revoke_compat_journal();
        self.send(IsolateCmd::Stop { invoke_hook });
        {
            let mut st = lock_after_isolate_failure(&self.teardown);
            if st.phase == TeardownPhase::Running {
                st.phase = TeardownPhase::UnwindingTick;
            }
            if st.phase == TeardownPhase::UnwindingTick {
                self.fire_terminate();
            }
        }
        supervisor::get()
            .expect("isolate creation initialized the supervisor")
            .retire(self, done, validation);
    }

    /// Synchronous test/embedder stop uses the same process retirement owner.
    /// A failed retiree stays owned after this bounded wait returns.
    pub fn join(self) -> Vec<String> {
        self.join_inner()
    }

    fn join_inner(self) -> Vec<String> {
        let (tx, rx) = mpsc::channel();
        self.retire(true, false, tx);
        rx.recv_timeout(SETUP_TIMEOUT + JOIN_TIMEOUT + Duration::from_secs(1))
            .unwrap_or_else(|_| vec!["JS isolate still retiring under process supervision".into()])
    }

    fn teardown_blocks_dispatch(&self) -> bool {
        matches!(
            self.teardown.lock().unwrap().phase,
            TeardownPhase::Hook | TeardownPhase::Done
        )
    }

    /// Fold completed ticks and thread log lines into local state.
    /// `logs` and `in_flight` are never held together: the thread also
    /// takes them in this same order (`logs` -> `in_flight` would let a
    /// slow-tick interrupt deadlock against `on_game_tick`), so each
    /// message is folded under its own lock.
    fn pump_logs(&self) {
        loop {
            let msg = {
                let rx = lock_after_isolate_failure(&self.rx);
                rx.try_recv().ok()
            };
            let Some(msg) = msg else {
                break;
            };
            match msg {
                ThreadMsg::Log(line) => lock_after_isolate_failure(&self.logs).push(line),
                ThreadMsg::TickError {
                    tick,
                    generation,
                    message,
                } => {
                    self.revoke_compat_journal();
                    lock_after_isolate_failure(&self.tick_errors)
                        .entry((tick, generation))
                        .or_default()
                        .push(message);
                }
                ThreadMsg::ScriptCut => {
                    self.revoke_compat_journal();
                    self.script_cut
                        .store(true, std::sync::atomic::Ordering::Release);
                }
                ThreadMsg::ScriptStopped { tick, reason } => {
                    self.revoke_compat_journal();
                    *lock_after_isolate_failure(&self.script_stop) =
                        Some(ScriptStopReceipt { tick, reason });
                }
                ThreadMsg::SessionReset { generation } => {
                    let mut fence = lock_after_isolate_failure(&self.held_fence);
                    if *fence == Some(generation) {
                        *fence = None;
                    }
                }
                ThreadMsg::Stopped => {
                    self.revoke_compat_journal();
                    self.stopped
                        .store(true, std::sync::atomic::Ordering::Release);
                    *lock_after_isolate_failure(&self.in_flight) = None;
                    lock_after_isolate_failure(&self.interacts).clear();
                    lock_after_isolate_failure(&self.seat_batches).clear();
                    lock_after_isolate_failure(&self.lifecycle).clear();
                }
                ThreadMsg::Interact {
                    bytes,
                    generation,
                    observed_walk_outcome_seq,
                } => {
                    let mut interacts = lock_after_isolate_failure(&self.interacts);
                    if generation
                        != self
                            .work_generation
                            .load(std::sync::atomic::Ordering::Acquire)
                    {
                        // Prior-generation gameplay never reaches the host;
                        // only an AbortWalk release is retained for the relog.
                        if self
                            .holding_releases
                            .load(std::sync::atomic::Ordering::Acquire)
                        {
                            if let Ok(reqs) = crate::isolate_fb::decode_interact_batch(&bytes) {
                                for token in reqs.iter().filter_map(discarded_walk_id) {
                                    self.send(IsolateCmd::RearmDiscardedWalk(token));
                                }
                                hold_walk_releases(
                                    &mut lock_after_isolate_failure(&self.held_releases),
                                    reqs.into_iter().map(|req| QueuedInteract {
                                        req,
                                        observed_walk_outcome_seq,
                                    }),
                                );
                            }
                        }
                        continue;
                    }
                    match crate::isolate_fb::decode_interact_batch(&bytes) {
                        Ok(reqs) => {
                            let mut lifecycle = lock_after_isolate_failure(&self.lifecycle);
                            for req in reqs {
                                if req.is_watchdog_lifecycle() {
                                    lifecycle.push(req);
                                } else {
                                    interacts.push(QueuedInteract {
                                        req,
                                        observed_walk_outcome_seq,
                                    });
                                }
                            }
                        }
                        Err(e) => {
                            lock_after_isolate_failure(&self.logs).push(format!("interact: {e}"))
                        }
                    }
                }
                ThreadMsg::SeatBatch {
                    bytes,
                    generation,
                    observed_walk_outcome_seq,
                    machine_events,
                } => {
                    if generation
                        != self
                            .work_generation
                            .load(std::sync::atomic::Ordering::Acquire)
                    {
                        if self
                            .holding_releases
                            .load(std::sync::atomic::Ordering::Acquire)
                        {
                            if let Ok(batch) = crate::isolate_fb::decode_seat_interact_batch(&bytes)
                            {
                                for token in batch
                                    .reqs
                                    .iter()
                                    .filter_map(|row| discarded_walk_id(&row.request))
                                {
                                    self.send(IsolateCmd::RearmDiscardedWalk(token));
                                }
                                hold_walk_releases(
                                    &mut lock_after_isolate_failure(&self.held_releases),
                                    batch.reqs.into_iter().map(|row| QueuedInteract {
                                        req: row.request,
                                        observed_walk_outcome_seq,
                                    }),
                                );
                            }
                        }
                        continue;
                    }
                    match crate::isolate_fb::decode_seat_interact_batch(&bytes) {
                        Ok(batch) => {
                            lock_after_isolate_failure(&self.seat_batches).push_back(
                                QueuedSeatBatch {
                                    generation,
                                    observed_walk_outcome_seq,
                                    batch,
                                    machine_events,
                                    held_waits: HeldWaitFacts::default(),
                                },
                            );
                        }
                        Err(error) => lock_after_isolate_failure(&self.logs)
                            .push(format!("seat interact: {error}")),
                    }
                }
                ThreadMsg::Paint(frame) => {
                    // A frame forwarded before a session reset arrives with the
                    // generation it was built under; install it under the
                    // current one, so an overlay from before the reset cannot
                    // match it (the wire decoder used to re-stamp here too).
                    let generation = self
                        .paint_generation
                        .load(std::sync::atomic::Ordering::Acquire);
                    let frame = if frame.generation == generation {
                        frame
                    } else {
                        let mut restamped = (*frame).clone();
                        restamped.generation = generation;
                        std::sync::Arc::new(restamped)
                    };
                    *lock_after_isolate_failure(&self.paint) = Some(frame);
                }
                ThreadMsg::IgnoredRandoms(list) => {
                    *lock_after_isolate_failure(&self.ignored_randoms) = list;
                }
                ThreadMsg::Completed {
                    tick,
                    generation,
                    successful,
                    report_errors,
                } => {
                    let error =
                        lock_after_isolate_failure(&self.tick_errors).remove(&(tick, generation));
                    if report_errors {
                        if let Some(messages) = error.as_ref() {
                            let mut logs = lock_after_isolate_failure(&self.logs);
                            logs.extend(
                                messages
                                    .iter()
                                    .map(|message| format!("tick {tick}: {message}")),
                            );
                        }
                    }
                    let current = generation
                        == self
                            .work_generation
                            .load(std::sync::atomic::Ordering::Acquire);
                    if current {
                        let mut active =
                            lock_after_isolate_failure(&self.tick_outcome_error_generation);
                        let outcome = match error {
                            Some(messages) if report_errors => {
                                *active = Some(generation);
                                Some(TickOutcome::Error {
                                    tick,
                                    generation,
                                    message: messages.join("; "),
                                })
                            }
                            None if successful && *active == Some(generation) => {
                                *active = None;
                                Some(TickOutcome::Success { tick, generation })
                            }
                            _ => None,
                        };
                        if let Some(outcome) = outcome {
                            lock_after_isolate_failure(&self.tick_outcomes).push(outcome);
                        }
                    }
                    if !current {
                        continue;
                    }
                    let mut in_flight = lock_after_isolate_failure(&self.in_flight);
                    #[cfg(feature = "memory-profile")]
                    self.last_completed
                        .fetch_max(tick, std::sync::atomic::Ordering::Relaxed);
                    if in_flight.is_some_and(|(g, t, _)| g == generation && t <= tick) {
                        *in_flight = None;
                    }
                }
                ThreadMsg::InFlightDone { generation, tick } => {
                    if generation
                        != self
                            .work_generation
                            .load(std::sync::atomic::Ordering::Acquire)
                    {
                        continue;
                    }
                    let mut in_flight = lock_after_isolate_failure(&self.in_flight);
                    if in_flight.is_some_and(|(g, t, _)| g == generation && t == tick) {
                        *in_flight = None;
                    }
                }
            }
        }
    }
}

impl Drop for LoadIsolate {
    fn drop(&mut self) {
        self.revoke_compat_journal();
        // Revoke the handle's authority even if its owner omitted join.
        // A live thread remains charged to the supervisor until real exit.
        {
            let mut st = lock_after_isolate_failure(&self.teardown);
            if st.phase != TeardownPhase::Done {
                st.phase = TeardownPhase::Done;
            }
        }
        let _ = self.poll_ready();
        self.send(IsolateCmd::Stop { invoke_hook: false });
        self.fire_terminate();
        if let Some(handle) = self.handle.take() {
            if handle.is_finished() {
                let _ = handle.join();
            } else {
                supervisor::get()
                    .expect("isolate creation initialized the supervisor")
                    .retain_uncredited(handle);
            }
        }
    }
}

fn discarded_walk_id(req: &crate::shim::InteractReq) -> Option<u64> {
    match req {
        crate::shim::InteractReq::Walk { request_id, .. }
        | crate::shim::InteractReq::WalkNear { request_id, .. }
            if *request_id != 0 =>
        {
            Some(*request_id)
        }
        _ => None,
    }
}

/// AbortWalk releases a kept reconnect must drain to the host. Gameplay
/// requests from the prior generation are never carried into the new session.
const HELD_RELEASES: usize = api::script_contract::MAX_WALK_RELEASES;

fn hold_walk_releases(
    held: &mut Vec<QueuedInteract>,
    reqs: impl IntoIterator<Item = QueuedInteract>,
) {
    for queued in reqs {
        let crate::shim::InteractReq::AbortWalk { request_id } = &queued.req else {
            continue;
        };
        if held.iter().any(|held| {
            matches!(
                &held.req,
                crate::shim::InteractReq::AbortWalk { request_id: held_id }
                    if held_id == request_id
            )
        }) {
            continue;
        }
        if held.len() < HELD_RELEASES {
            held.push(queued);
        }
    }
}

#[cfg(test)]
#[path = "isolate_tests.rs"]
mod tests;
#[cfg(test)]
mod kept_reconnect_release_tests {
    use super::*;
    use crate::shim::InteractReq;

    #[test]
    fn stale_gameplay_cannot_evict_a_kept_abort_walk_release() {
        let release = QueuedInteract {
            req: InteractReq::AbortWalk { request_id: 42 },
            observed_walk_outcome_seq: 7,
        };
        let mut held = Vec::new();
        hold_walk_releases(&mut held, [release.clone()]);
        let stale_gameplay = (0..HELD_RELEASES * 2).map(|id| QueuedInteract {
            req: InteractReq::Walk {
                x: id as i32,
                z: id as i32,
                level: 0,
                allow_teleports: false,
                allow_wilderness: false,
                allow_bank_fetch: false,
                request_id: id as u64 + 1,
                avoid: Vec::new(),
                cross: Vec::new(),
            },
            observed_walk_outcome_seq: 3,
        });

        hold_walk_releases(&mut held, stale_gameplay);

        assert_eq!(held, vec![release]);
    }
}
