//! LoadIsolate: rustyscript V8 on its own thread (feature `load` only).

mod teardown;
mod thread;

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
use super::{loadout_v8, machine_v8, paint_chrome, paint_jive, reach_query, shape, snapshot};
use rustyscript::{json_args, Runtime, RuntimeOptions};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Mutex, Once, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

static NEXT_PAINT_GENERATION: AtomicU64 = AtomicU64::new(1);
static LIVE_RUNTIMES: AtomicUsize = AtomicUsize::new(0);
static ABANDONED_ISOLATES: Mutex<Vec<JoinHandle<()>>> = Mutex::new(Vec::new());

fn reap_finished_abandoned(list: &mut Vec<JoinHandle<()>>) {
    let mut i = 0;
    while i < list.len() {
        if list[i].is_finished() {
            let handle = list.remove(i);
            let _ = handle.join();
        } else {
            i += 1;
        }
    }
}

fn retain_abandoned_thread(handle: JoinHandle<()>) {
    let mut list = ABANDONED_ISOLATES.lock().unwrap();
    reap_finished_abandoned(&mut list);
    list.push(handle);
}

/// Isolates whose `join` hit [`JOIN_TIMEOUT`] and were not dropped silently.
#[doc(hidden)]
#[allow(dead_code)]
pub fn abandoned_isolate_count() -> usize {
    let mut list = ABANDONED_ISOLATES.lock().unwrap();
    reap_finished_abandoned(&mut list);
    list.len()
}

/// V8 runtimes currently owned by isolate threads. Tests use this to prove a
/// cancelled non-yielding startup was destroyed rather than merely detached.
#[doc(hidden)]
#[cfg(test)]
pub fn live_runtime_count() -> usize {
    LIVE_RUNTIMES.load(std::sync::atomic::Ordering::Acquire)
}

/// Per-tick budget: ticks taking longer than this are interrupted and
/// logged, and stale ticks are skipped.
const SLOW_TICK: Duration = Duration::from_millis(50);
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

enum IsolateCmd {
    Tick {
        tick: u64,
        generation: u64,
        input_identity: u64,
    },
    ResetSession,
    /// The host's FlatBuffer snapshot blob (schema: `crates/script/
    /// schema/isolate.fbs`), decoded on the isolate thread into the
    /// JS object the Game/Inventory/Skills/EventSignal shims read
    /// before the next dispatched tick. Never a JSON string.
    Snapshot(SnapshotMessage),
    /// Merged operator settings bag for the prelude's `this.settings.*`,
    /// built on the isolate thread as V8 values (never JSON text).
    Settings(serde_json::Map<String, serde_json::Value>),
    /// Available loadouts. Rust keeps them for `selectedLoadout`.
    Loadouts(Vec<crate::loadouts_store::Loadout>),
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
    /// thread with the 50 ms budget; the reply is a FlatBuffer interact.
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
    /// A diagnostic emitted while processing one tick. The host keeps this
    /// separate from user-authored log lines so a later user message cannot
    /// clear or create the slot's active tick error.
    TickError {
        tick: u64,
        generation: u64,
        message: String,
    },
    /// The tick's shim interact queue (`__rs2b0t_host.interact`), a
    /// FlatBuffer `InteractBatch` of [`crate::shim::InteractReq`]s
    /// forwarded after the tick's JS finished (parked or not).
    Interact {
        bytes: Vec<u8>,
        generation: u64,
    },
    /// The bot instance's `ignoredRandoms()` list, read on the isolate
    /// thread each tick and sent only when it changed; the host caches it
    /// (no probe).
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
    /// Interact requests forwarded by the tick thread (the shim
    /// `Bank`/`Banking` queue), drained by the host like logs.
    interacts: Mutex<Vec<crate::shim::InteractReq>>,
    /// Watchdog lifecycle facts from the same FlatBuffer batch.
    lifecycle: Mutex<Vec<crate::shim::InteractReq>>,
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

impl LoadIsolate {
    /// Spawn the isolate thread with already-cached JS (no transpile).
    /// Returns as soon as the thread is created; V8 setup is resolved by
    /// [`LoadIsolate::poll_ready`]. Thread-create failure is the only
    /// synchronous error.
    pub fn spawn(
        js: String,
        shape: LoadShape,
        siblings: Vec<(String, String)>,
    ) -> Result<Self, String> {
        Self::spawn_inner(
            js,
            shape,
            siblings,
            None,
            std::sync::Arc::new(api::named_banks::NamedBankFacts::empty()),
            std::sync::Arc::new(api::run_policy::RunPolicyOverrideCell::new()),
        )
    }

    /// Spawn with immutable selected-revision facts. The Arc is shared
    /// until the isolate's one startup publication is complete.
    pub fn spawn_with_game_data(
        js: String,
        shape: LoadShape,
        siblings: Vec<(String, String)>,
        game_data: std::sync::Arc<api::game_data::SelectedGameData>,
    ) -> Result<Self, String> {
        Self::spawn_inner(
            js,
            shape,
            siblings,
            Some(game_data),
            std::sync::Arc::new(api::named_banks::NamedBankFacts::empty()),
            std::sync::Arc::new(api::run_policy::RunPolicyOverrideCell::new()),
        )
    }

    /// Spawn with selected-revision facts, named aliases, and the script
    /// slot's shared run-policy cell.
    pub fn spawn_with_content(
        js: String,
        shape: LoadShape,
        siblings: Vec<(String, String)>,
        game_data: Option<std::sync::Arc<api::game_data::SelectedGameData>>,
        named_banks: std::sync::Arc<api::named_banks::NamedBankFacts>,
        run_policy_override: std::sync::Arc<api::run_policy::RunPolicyOverrideCell>,
    ) -> Result<Self, String> {
        Self::spawn_inner(
            js,
            shape,
            siblings,
            game_data,
            named_banks,
            run_policy_override,
        )
    }

    /// Evaluate and instantiate the candidate in a throwaway Runtime
    /// using the same [`wire_runtime`] path Start uses. Old isolates
    /// are not touched. Fails on top-level throw, missing export, or
    /// unresolvable sibling.
    pub fn validate_source(
        js: &str,
        shape: LoadShape,
        siblings: &[(String, String)],
    ) -> Result<(), String> {
        let source = js.to_owned();
        let siblings = siblings.to_vec();
        std::thread::Builder::new()
            .name("js-validate".into())
            .spawn(move || {
                let isolate = Self::spawn(source, shape, siblings)?;
                let result = isolate.resolve_setup(Some(Instant::now() + VALIDATION_TIMEOUT));
                let result = match result {
                    Ready::Ready => Ok(()),
                    Ready::Failed(error) => Err(error),
                    Ready::Pending => unreachable!("bounded setup wait cannot remain pending"),
                };
                let _ = isolate.join_without_hook();
                result
            })
            .map_err(|e| format!("js validation thread: {e}"))?
            .join()
            .map_err(|_| "js validation thread panicked".to_string())?
    }

    fn spawn_inner(
        js: String,
        shape: LoadShape,
        siblings: Vec<(String, String)>,
        game_data: Option<std::sync::Arc<api::game_data::SelectedGameData>>,
        named_banks: std::sync::Arc<api::named_banks::NamedBankFacts>,
        run_policy_override: std::sync::Arc<api::run_policy::RunPolicyOverrideCell>,
    ) -> Result<Self, String> {
        ensure_platform();
        let (tx, rx) = mpsc::channel::<IsolateCmd>();
        let (setup_tx, setup_rx) = mpsc::channel::<SetupMessage>();
        let (msg_tx, msg_rx) = mpsc::channel::<ThreadMsg>();
        #[cfg(feature = "memory-profile")]
        let counters = crate::memory_profile::registered();
        #[cfg(feature = "memory-profile")]
        let thread_counters = counters.clone();
        let work_generation = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
        let thread_generation = work_generation.clone();
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
        let handle = std::thread::Builder::new()
            .name("js-isolate".into())
            .spawn(move || {
                isolate_main(
                    js,
                    shape,
                    siblings,
                    game_data,
                    named_banks,
                    run_policy_override,
                    CmdQueue {
                        rx,
                        queued: thread_queued,
                    },
                    msg_tx,
                    setup_tx,
                    thread_generation,
                    thread_paint_generation,
                    thread_teardown,
                    thread_proof,
                    #[cfg(feature = "memory-profile")]
                    thread_counters,
                )
            })
            .map_err(|e| format!("isolate thread: {e}"))?;
        Ok(LoadIsolate {
            #[cfg(feature = "memory-profile")]
            counters,
            #[cfg(feature = "memory-profile")]
            last_completed: std::sync::atomic::AtomicU64::new(0),
            #[cfg(feature = "memory-profile")]
            dispatched: std::sync::atomic::AtomicU64::new(0),
            work_generation,
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
            interacts: Mutex::new(Vec::new()),
            lifecycle: Mutex::new(Vec::new()),
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
    pub fn post_loadouts(&self, loadouts: &[crate::loadouts_store::Loadout]) {
        self.send(IsolateCmd::Loadouts(loadouts.to_vec()));
    }

    /// Post the merged operator settings bag (schema defaults + panel/TUI
    /// overrides + optional scenario inject). The prelude's
    /// `this.settings.*` reads `__rs2b0t_host.settingsBag`.
    pub fn post_settings_bag(&self, bag: &serde_json::Map<String, serde_json::Value>) {
        self.send(IsolateCmd::Settings(bag.clone()));
    }

    /// Dispatch one observed game tick to the isolate. The previous
    /// tick is checked against the budget: still running past
    /// [`SLOW_TICK`] is interrupted and logged, and its stale ticks are
    /// skipped.
    pub fn on_game_tick(&self, snap_tick: u64) {
        self.on_game_tick_at(snap_tick, 0);
    }

    /// Dispatch one observed game tick, tagging produced mouse rows with
    /// the native permit identity that was live at production.
    pub fn on_game_tick_at(&self, snap_tick: u64, input_identity: u64) {
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
        });
    }

    /// Interrupt and report the currently active execution when its host
    /// dispatch has exceeded the tick budget. `in_flight` can outlive the
    /// corresponding V8 entry, so execution ownership is the final fence.
    fn interrupt_slow_execution(&self, ready: bool) {
        if !ready {
            return;
        }
        let over = self
            .in_flight
            .lock()
            .unwrap()
            .as_ref()
            .filter(|(_, _, started)| started.elapsed() > SLOW_TICK)
            .map(|(_, tick, started)| (*tick, started.elapsed()));
        let Some((tick, elapsed)) = over else {
            return;
        };
        if self.fire_execution_interrupt(teardown::ExecutionInterrupt::Watchdog) {
            let line = if tick == RECOVERY_ANCHOR_TICK {
                format!("interrupted slow recoveryAnchor ({elapsed:?})")
            } else {
                format!("interrupted slow tick {tick} ({elapsed:?})")
            };
            self.logs.lock().unwrap().push(line);
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

    /// Park tick dispatch. Pause publishes intent under the execution lock,
    /// terminating only the eval that is actually active. A queued eval sees
    /// the intent before entry; a completed eval cannot be terminated after
    /// its isolate-side cancel.
    pub fn pause(&self) {
        let ready = self.poll_ready() == Ready::Ready;
        self.pump_logs();
        self.interrupt_slow_execution(ready);
        {
            let mut st = self.teardown.lock().unwrap();
            if st.phase == TeardownPhase::Running {
                st.pause_requested = true;
                if st.execution_active
                    && st.execution_interrupt != Some(teardown::ExecutionInterrupt::Watchdog)
                {
                    st.execution_interrupt = Some(teardown::ExecutionInterrupt::Pause);
                    self.fire_terminate();
                }
            }
        }
        self.send(IsolateCmd::Pause);
    }

    /// Re-arm tick dispatch after [`LoadIsolate::pause`].
    pub fn resume(&self) {
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
        let interrupted = {
            let mut in_flight = self.in_flight.lock().unwrap();
            let over = in_flight
                .as_ref()
                .filter(|(_, _, started)| ready && started.elapsed() > SLOW_TICK)
                .map(|(_, tick, started)| (*tick, started.elapsed()));
            *in_flight = Some((generation, RECOVERY_ANCHOR_TICK, Instant::now()));
            over
        };
        if let Some((tick, elapsed)) = interrupted {
            if self.fire_execution_interrupt(teardown::ExecutionInterrupt::Watchdog) {
                let line = if tick == RECOVERY_ANCHOR_TICK {
                    format!("interrupted slow recoveryAnchor ({elapsed:?})")
                } else {
                    format!("interrupted slow tick {tick} ({elapsed:?})")
                };
                self.logs.lock().unwrap().push(line);
            }
        }
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

    /// Drain the isolate's log lines (tick errors, slow/interrupted
    /// ticks).
    pub fn drain_logs(&self) -> Vec<String> {
        self.pump_logs();
        std::mem::take(&mut *self.logs.lock().unwrap())
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

    /// Whether the isolate thread currently owns an interruptible user
    /// execution phase. Tests use this as a scheduling barrier rather than
    /// guessing with a sleep.
    #[doc(hidden)]
    pub fn execution_active(&self) -> bool {
        self.teardown.lock().unwrap().execution_active
    }

    /// Isolate-scoped seam: delay after the deadline owner is armed and
    /// before getter/body. Proves a late cancel cannot drop the one-shot.
    #[doc(hidden)]
    pub fn delay_onstop_after_deadline_arm(&self, delay: Duration) {
        self.teardown.lock().unwrap().hook_entry_delay = Some(delay);
    }

    /// Isolate-scoped seam: deadline-thread spawn fails for this isolate.
    #[doc(hidden)]
    pub fn fail_onstop_deadline_spawn(&self) {
        self.teardown.lock().unwrap().fail_deadline_spawn = true;
    }

    /// Give a success-path unit test enough scheduling headroom while
    /// retaining a bounded hook deadline.
    #[cfg(test)]
    pub(crate) fn set_onstop_timeout_for_test(&self, timeout: Duration) {
        self.teardown.lock().unwrap().test_hook_timeout = Some(timeout);
    }

    /// Drain the interact requests the tick's shim queued
    /// (`__rs2b0t_host.interact`), forwarded by the tick thread in
    /// tick order. The host dispatches them through the slot Driver;
    /// a malformed entry is logged and dropped, never fatal.
    pub fn drain_interacts(&self) -> Vec<crate::shim::InteractReq> {
        self.pump_logs();
        std::mem::take(&mut *self.interacts.lock().unwrap())
    }

    /// Put a host-drained batch back in front of requests that arrived
    /// afterward. Used when an operator Pause wins the host's final
    /// dispatch fence: no verb is lost, and Resume observes original order.
    pub(crate) fn restore_interacts(&self, mut drained: Vec<crate::shim::InteractReq>) {
        let mut queued = self.interacts.lock().unwrap();
        drained.append(&mut queued);
        *queued = drained;
    }

    /// Drop queued canvas mouse rows so Pause/logout cannot replay them.
    pub fn discard_mouse_interacts(&self) {
        self.pump_logs();
        self.interacts
            .lock()
            .unwrap()
            .retain(|req| !matches!(req, crate::shim::InteractReq::Mouse { .. }));
    }

    /// Drain generation-matched watchdog lifecycle facts (`note-progress`,
    /// loop/wait settle, recoveryAnchor replies) from the same FlatBuffer
    /// batch. Game interacts stay on [`LoadIsolate::drain_interacts`].
    pub fn drain_lifecycle(&self) -> Vec<crate::shim::InteractReq> {
        self.pump_logs();
        std::mem::take(&mut *self.lifecycle.lock().unwrap())
    }

    /// Discard work from the previous connection, including batches that
    /// an already running tick has not forwarded yet. Script state and
    /// parked waits survive; the next snapshot is posted before a new tick.
    pub fn reset_session_work(&self) {
        {
            let mut interacts = self.interacts.lock().unwrap();
            self.work_generation
                .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
            self.paint_generation.store(
                NEXT_PAINT_GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
                std::sync::atomic::Ordering::Release,
            );
            interacts.clear();
            self.lifecycle.lock().unwrap().clear();
        }
        self.tick_errors.lock().unwrap().clear();
        *self.tick_outcome_error_generation.lock().unwrap() = None;
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
        self.send(IsolateCmd::ResetSession);
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
        let mut setup = self.setup.lock().unwrap();
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
                    let st = self.teardown.lock().unwrap();
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
                    if let Some(entry) = self.in_flight.lock().unwrap().as_mut() {
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

    /// Fire an eval interrupt only while the isolate thread owns an active
    /// interruptible execution. The isolate-side finish/cancel uses this same
    /// lock, so no terminate can leak into a later tick or `onStop`.
    fn fire_execution_interrupt(&self, owner: teardown::ExecutionInterrupt) -> bool {
        let mut st = self.teardown.lock().unwrap();
        if st.phase != TeardownPhase::Running || !st.execution_active {
            return false;
        }
        st.execution_interrupt = Some(owner);
        self.fire_terminate();
        true
    }

    /// Terminate setup/tick failure while it is still before Hook.
    fn fire_before_hook(&self) {
        let st = self.teardown.lock().unwrap();
        if matches!(
            st.phase,
            TeardownPhase::Running | TeardownPhase::UnwindingTick
        ) {
            self.fire_terminate();
        }
    }

    /// Stop without blocking the caller. `join` (onStop hook plus the 2 s
    /// cap) runs on a reaper thread that reports leftover logs.
    pub fn join_detached(self, done: Sender<Vec<String>>) {
        let _ = std::thread::Builder::new()
            .name("isolate-reaper".into())
            .spawn(move || {
                let _ = done.send(self.join());
            });
    }

    /// Stop the isolate: tell the thread to exit, interrupt a live tick
    /// so Stop can be processed, and wait for the thread (the Runtime
    /// is dropped there). The wait is bounded by [`JOIN_TIMEOUT`]:
    /// a stuck isolate is abandoned (thread tracked, not dropped) so Stop
    /// can never freeze the panel. Returns leftover log lines (including
    /// `onStop` / `this.log` drained during teardown) after `pump_logs`.
    ///
    /// Tick interruption is claimed only while phase is `Running`. Once
    /// the isolate has entered the hook, only that hook's 50 ms
    /// one-shot may terminate; join never samples `in_flight` to decide.
    /// A join during setup first waits (bounded by the setup deadline) for
    /// the terminate handle, so a tick queued before Ready can still be
    /// interrupted; the reaper, never the UI, owns that wait.
    pub fn join(self) -> Vec<String> {
        self.join_inner(true)
    }

    /// Validation owns a throwaway isolate but must not invoke user
    /// `onStop`; it only needs the runtime to finish and release.
    fn join_without_hook(self) -> Vec<String> {
        self.join_inner(false)
    }

    fn join_inner(mut self, invoke_hook: bool) -> Vec<String> {
        self.send(IsolateCmd::Stop { invoke_hook });
        {
            let mut st = self.teardown.lock().unwrap();
            if st.phase == TeardownPhase::Running {
                st.phase = TeardownPhase::UnwindingTick;
            }
            if st.phase == TeardownPhase::UnwindingTick {
                // Fire immediately when poll_ready already consumed the
                // startup handle. Otherwise resolve_setup fires as soon as
                // that independently owned handle arrives.
                self.fire_terminate();
            }
        }
        let _ = self.resolve_setup(Some(Instant::now() + SETUP_TIMEOUT));
        if let Some(handle) = self.handle.take() {
            let deadline = Instant::now() + JOIN_TIMEOUT;
            while !handle.is_finished() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(5));
                self.pump_logs();
            }
            if handle.is_finished() {
                let _ = handle.join();
            } else {
                retain_abandoned_thread(handle);
            }
        }
        self.pump_logs();
        std::mem::take(&mut *self.logs.lock().unwrap())
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
                let rx = self.rx.lock().unwrap();
                rx.try_recv().ok()
            };
            let Some(msg) = msg else {
                break;
            };
            match msg {
                ThreadMsg::Log(line) => self.logs.lock().unwrap().push(line),
                ThreadMsg::TickError {
                    tick,
                    generation,
                    message,
                } => {
                    self.tick_errors
                        .lock()
                        .unwrap()
                        .entry((tick, generation))
                        .or_default()
                        .push(message);
                }
                ThreadMsg::ScriptStopped { tick, reason } => {
                    *self.script_stop.lock().unwrap() = Some(ScriptStopReceipt { tick, reason });
                }
                ThreadMsg::Stopped => {
                    self.stopped
                        .store(true, std::sync::atomic::Ordering::Release);
                    *self.in_flight.lock().unwrap() = None;
                    self.interacts.lock().unwrap().clear();
                    self.lifecycle.lock().unwrap().clear();
                }
                ThreadMsg::Interact { bytes, generation } => {
                    let mut interacts = self.interacts.lock().unwrap();
                    if generation
                        != self
                            .work_generation
                            .load(std::sync::atomic::Ordering::Acquire)
                    {
                        continue;
                    }
                    match crate::isolate_fb::decode_interact_batch(&bytes) {
                        Ok(reqs) => {
                            let mut lifecycle = self.lifecycle.lock().unwrap();
                            for req in reqs {
                                if req.is_watchdog_lifecycle() {
                                    lifecycle.push(req);
                                } else {
                                    interacts.push(req);
                                }
                            }
                        }
                        Err(e) => self.logs.lock().unwrap().push(format!("interact: {e}")),
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
                    *self.paint.lock().unwrap() = Some(frame);
                }
                ThreadMsg::IgnoredRandoms(list) => {
                    *self.ignored_randoms.lock().unwrap() = list;
                }
                ThreadMsg::Completed {
                    tick,
                    generation,
                    successful,
                    report_errors,
                } => {
                    let error = self.tick_errors.lock().unwrap().remove(&(tick, generation));
                    if report_errors {
                        if let Some(messages) = error.as_ref() {
                            let mut logs = self.logs.lock().unwrap();
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
                        let mut active = self.tick_outcome_error_generation.lock().unwrap();
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
                            self.tick_outcomes.lock().unwrap().push(outcome);
                        }
                    }
                    if !current {
                        continue;
                    }
                    let mut in_flight = self.in_flight.lock().unwrap();
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
                    let mut in_flight = self.in_flight.lock().unwrap();
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
        // Best-effort: unblock a stuck tick and close the channel; the
        // thread exits and drops its Runtime by itself (no join here,
        // and no cancel — the thread clears the terminate once the tick
        // has returned). After a successful join the hook is Done: do
        // not re-interrupt a completed teardown.
        {
            let mut st = self.teardown.lock().unwrap();
            match st.phase {
                TeardownPhase::Done => {}
                TeardownPhase::Running | TeardownPhase::UnwindingTick | TeardownPhase::Hook => {
                    st.phase = TeardownPhase::Done;
                    st.cancel.take();
                }
            }
        }
        let _ = self.poll_ready();
        self.send(IsolateCmd::Stop { invoke_hook: false });
        self.fire_terminate();
        if let Some(handle) = self.handle.take() {
            if handle.is_finished() {
                let _ = handle.join();
            } else {
                retain_abandoned_thread(handle);
            }
        }
    }
}

/// Initialize the V8 platform once, on the caller's thread (Start and
/// Load both run here, and the isolate threads are spawned by it).
fn ensure_platform() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        rustyscript::init_platform(1, true);
    });
}

#[cfg(test)]
#[path = "isolate_tests.rs"]
mod tests;
