//! LoadIsolate: rustyscript V8 on its own thread (feature `load` only).

use super::bindings::wire_runtime;
use super::shape::LoadShape;
use super::snapshot::{dispatch_native_events, materialize_settings_bag, materialize_snapshot};
use rustyscript::{json_args, Runtime, RuntimeOptions};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Mutex, Once, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

static NEXT_PAINT_GENERATION: AtomicU64 = AtomicU64::new(1);
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

/// Per-tick budget: ticks taking longer than this are interrupted and
/// logged, and stale ticks are skipped.
const SLOW_TICK: Duration = Duration::from_millis(50);
/// `in_flight` tick id for a live `recoveryAnchor` eval (not a game tick).
const RECOVERY_ANCHOR_TICK: u64 = u64::MAX;
/// Hard stop for yielding JS (rustyscript `RuntimeOptions.timeout`).
const RUNTIME_TIMEOUT: Duration = Duration::from_millis(50);
/// How long `join` waits for the isolate thread after Stop + terminate
/// before abandoning it: a stuck isolate must never freeze the caller.
const JOIN_TIMEOUT: Duration = Duration::from_secs(2);
/// How long `poll_ready` waits for V8 creation, prelude, content eval
/// and module load before reporting the same timeout spawn used to.
const SETUP_TIMEOUT: Duration = Duration::from_secs(10);
/// Heap cap for the isolate (~64 MB, the brief's number).
const MAX_HEAP: usize = 64 * 1024 * 1024;
/// Bounded native pairing metadata for mouse gestures produced by one
/// isolate. Overflow invalidates every unmatched pair rather than letting
/// a later up inherit a newer gesture's identity.
const MAX_MOUSE_GESTURES: usize = 32;

#[derive(Default)]
struct MouseGestureIdentities {
    pairs: VecDeque<u64>,
    /// Number of leading ups that cannot be paired after overflow. The
    /// count is bounded storage and makes them fail closed at identity 0.
    unpairable: u64,
}

fn stamp_mouse_gesture_identities(
    reqs: &mut [crate::shim::InteractReq],
    input_identity: u64,
    gestures: &mut MouseGestureIdentities,
) {
    for req in reqs {
        let crate::shim::InteractReq::Mouse { down, identity, .. } = req else {
            continue;
        };
        if *down {
            *identity = input_identity;
            if gestures.unpairable != 0 {
                gestures.unpairable = gestures.unpairable.saturating_add(1);
            } else if gestures.pairs.len() >= MAX_MOUSE_GESTURES {
                gestures.unpairable = (gestures.pairs.len() as u64).saturating_add(1);
                gestures.pairs.clear();
            } else {
                gestures.pairs.push_back(input_identity);
            }
        } else if gestures.unpairable != 0 {
            gestures.unpairable -= 1;
            *identity = 0;
        } else {
            // FIFO is deliberate: if a second down precedes the first
            // up, that old up must retain the oldest gesture identity.
            *identity = gestures.pairs.pop_front().unwrap_or(0);
        }
    }
}

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
    /// Merged operator settings JSON for the prelude's `this.settings.*`.
    Settings(String),
    Loadouts(String),
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

/// Host/isolate coordination for Stop vs `onStop`. Transitions are
/// taken under the mutex so join cannot terminate after the hook
/// starts, and the hook's 50 ms one-shot cannot fire into Done.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum TeardownPhase {
    Running,
    UnwindingTick,
    Hook,
    Done,
}

/// Phase, Hook-entry deadline, and at-most-one interrupt. Finish and the
/// one-shot worker decide under this same mutex.
struct TeardownState {
    phase: TeardownPhase,
    deadline: Option<Instant>,
    interrupt_issued: bool,
    /// Dropped at Done so a sleeping one-shot worker exits without terminate.
    cancel: Option<Sender<()>>,
    /// Isolate-scoped test seam: sleep after the deadline owner is armed
    /// and before getter/body, without a process-global switch.
    hook_entry_delay: Option<Duration>,
    /// Isolate-scoped test seam: pretend deadline-thread spawn failed.
    fail_deadline_spawn: bool,
}

impl TeardownState {
    fn new() -> Self {
        Self {
            phase: TeardownPhase::Running,
            deadline: None,
            interrupt_issued: false,
            cancel: None,
            hook_entry_delay: None,
            fail_deadline_spawn: false,
        }
    }
}

/// Per-isolate teardown evidence. Survives dropping the public handle so
/// raw Drop can wait for the isolate thread to consume Stop before
/// asserting no hook / no leftover deadline worker.
#[doc(hidden)]
#[derive(Clone)]
pub struct TeardownProof {
    inner: std::sync::Arc<TeardownProofInner>,
}

struct TeardownProofInner {
    invoked: AtomicBool,
    finished: AtomicBool,
    worker_live: AtomicUsize,
}

impl TeardownProof {
    fn new() -> Self {
        Self {
            inner: std::sync::Arc::new(TeardownProofInner {
                invoked: AtomicBool::new(false),
                finished: AtomicBool::new(false),
                worker_live: AtomicUsize::new(0),
            }),
        }
    }

    pub fn invoked(&self) -> bool {
        self.inner.invoked.load(std::sync::atomic::Ordering::SeqCst)
    }

    pub fn finished(&self) -> bool {
        self.inner
            .finished
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    pub fn deadline_workers(&self) -> usize {
        self.inner
            .worker_live
            .load(std::sync::atomic::Ordering::SeqCst)
    }
}

struct DeadlineWorkerGuard {
    proof: std::sync::Arc<TeardownProofInner>,
}
impl Drop for DeadlineWorkerGuard {
    fn drop(&mut self) {
        self.proof
            .worker_live
            .fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

struct TickLoopFinish(std::sync::Arc<TeardownProofInner>);
impl Drop for TickLoopFinish {
    fn drop(&mut self) {
        self.0
            .finished
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

/// Join (or Drop) has claimed the isolate for Stop and armed a terminate
/// for the running tick. A tick phase must not start after that point: an
/// earlier call whose error is ignored may already have absorbed the
/// terminate, and a spinning `loop()` would then never be interrupted.
/// Join sets the phase before it fires the terminate, so a check made
/// after any absorbing call sees it.
fn tick_claimed(teardown: &Mutex<TeardownState>) -> bool {
    teardown.lock().unwrap().phase != TeardownPhase::Running
}

/// Record the eligible tick number on the host handle
/// (`__rs2b0t_host.tick`) before any of the tick's JS runs.
fn record_tick(runtime: &mut Runtime, n: u64) {
    let scope = &mut runtime.deno_runtime().handle_scope();
    let global = scope.get_current_context().global(scope);
    let (Some(host_key), Some(tick_key)) = (
        v8::String::new(scope, "__rs2b0t_host"),
        v8::String::new(scope, "tick"),
    ) else {
        return;
    };
    let Some(host) = global
        .get(scope, host_key.into())
        .and_then(|host| host.to_object(scope))
    else {
        return;
    };
    let tick = v8::Number::new(scope, n as f64);
    let _ = host.set(scope, tick_key.into(), tick.into());
}

/// Drops every machine row when the tick loop ends (Stop, script stop).
struct MachinesStop;
impl Drop for MachinesStop {
    fn drop(&mut self) {
        crate::machine::on_stop();
    }
}

enum ThreadMsg {
    Log(String),
    /// The tick's shim interact queue (`__rs2b0t_host.interact`), a
    /// FlatBuffer `InteractBatch` of [`crate::shim::InteractReq`]s
    /// forwarded after the tick's JS finished (parked or not).
    Interact {
        bytes: Vec<u8>,
        generation: u64,
    },
    /// The bot instance's `ignoredRandoms()` list, read on the isolate
    /// thread after the tick and cached on the host handle (no probe).
    IgnoredRandoms(Vec<String>),
    /// The highest tick the thread has fully processed (ran or skipped).
    Completed {
        tick: u64,
        generation: u64,
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
    rx: Mutex<Receiver<ThreadMsg>>,
    logs: Mutex<Vec<String>>,
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
    /// thread after each tick (same source as the old probe path).
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

enum SetupState {
    Pending {
        rx: Receiver<Result<v8::IsolateHandle, String>>,
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
        )
    }

    /// Spawn with selected-revision facts and already-resolved named
    /// bank aliases. Existing constructors post empty aliases.
    pub fn spawn_with_content(
        js: String,
        shape: LoadShape,
        siblings: Vec<(String, String)>,
        game_data: Option<std::sync::Arc<api::game_data::SelectedGameData>>,
        named_banks: std::sync::Arc<api::named_banks::NamedBankFacts>,
    ) -> Result<Self, String> {
        Self::spawn_inner(js, shape, siblings, game_data, named_banks)
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
        ensure_platform();
        let mut runtime = Runtime::new(RuntimeOptions {
            timeout: RUNTIME_TIMEOUT,
            max_heap_size: Some(MAX_HEAP),
            ..Default::default()
        })
        .map_err(|e| format!("js engine init: {e}"))?;
        wire_runtime(
            &mut runtime,
            js,
            shape,
            siblings,
            None,
            std::sync::Arc::new(api::named_banks::NamedBankFacts::empty()),
        )
    }

    fn spawn_inner(
        js: String,
        shape: LoadShape,
        siblings: Vec<(String, String)>,
        game_data: Option<std::sync::Arc<api::game_data::SelectedGameData>>,
        named_banks: std::sync::Arc<api::named_banks::NamedBankFacts>,
    ) -> Result<Self, String> {
        ensure_platform();
        let (tx, rx) = mpsc::channel::<IsolateCmd>();
        let (msg_tx, msg_rx) = mpsc::channel::<ThreadMsg>();
        let (setup_tx, setup_rx) = mpsc::channel::<Result<v8::IsolateHandle, String>>();
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
        let handle = std::thread::Builder::new()
            .name("js-isolate".into())
            .spawn(move || {
                isolate_main(
                    js,
                    shape,
                    siblings,
                    game_data,
                    named_banks,
                    rx,
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
            rx: Mutex::new(msg_rx),
            logs: Mutex::new(Vec::new()),
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
    pub fn post_snapshot(&self, bytes: Vec<u8>) {
        let message = SnapshotMessage {
            #[cfg(feature = "memory-profile")]
            _lease: crate::memory_profile::SnapshotLease::new(
                self.counters.clone(),
                bytes.len(),
                bytes.capacity(),
            ),
            bytes,
        };
        let _ = self.tx.send(IsolateCmd::Snapshot(message));
    }

    /// Post available loadouts before subsequent tick commands.
    pub fn post_loadouts(&self, loadouts: &[crate::loadouts_store::Loadout]) {
        let json = serde_json::to_string(loadouts).expect("serializable loadouts");
        let _ = self.tx.send(IsolateCmd::Loadouts(json));
    }

    /// Post the merged operator settings bag (schema defaults + panel/TUI
    /// overrides + optional scenario inject). The prelude's
    /// `this.settings.*` reads `__rs2b0t_host.settingsBag`.
    pub fn post_settings_bag(&self, bag: &serde_json::Map<String, serde_json::Value>) {
        let Ok(json) = serde_json::to_string(bag) else {
            return;
        };
        let _ = self.tx.send(IsolateCmd::Settings(json));
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
        #[cfg(feature = "memory-profile")]
        self.dispatched
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let generation = self
            .work_generation
            .load(std::sync::atomic::Ordering::Acquire);
        let interrupted = {
            let mut in_flight = self.in_flight.lock().unwrap();
            // The previous tick is still in flight (no `Completed`
            // folded yet) past the budget: interrupt it.
            let over = in_flight
                .as_ref()
                .filter(|(_, _, started)| ready && started.elapsed() > SLOW_TICK)
                .map(|(_, tick, started)| (*tick, started.elapsed()));
            *in_flight = Some((generation, snap_tick, Instant::now()));
            over
        };
        if let Some((tick, elapsed)) = interrupted {
            // Leave the terminate armed until the isolate thread has
            // returned from the tick (it cancels there); an immediate
            // cancel would race the interrupt and make this a no-op.
            self.fire_terminate();
            // `in_flight` was released before this lock, so the lock
            // order (never `in_flight` -> `logs`) holds everywhere.
            let line = if tick == RECOVERY_ANCHOR_TICK {
                format!("interrupted slow recoveryAnchor ({elapsed:?})")
            } else {
                format!("interrupted slow tick {tick} ({elapsed:?})")
            };
            self.logs.lock().unwrap().push(line);
        }
        let _ = self.tx.send(IsolateCmd::Tick {
            tick: snap_tick,
            generation,
            input_identity,
        });
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

    /// Park tick dispatch. A runaway tick is interrupted first so the
    /// thread returns to the command loop.
    pub fn pause(&self) {
        let ready = self.poll_ready() == Ready::Ready;
        self.pump_logs();
        if self.teardown_blocks_dispatch() {
            let _ = self.tx.send(IsolateCmd::Pause);
            return;
        }
        let over = self
            .in_flight
            .lock()
            .unwrap()
            .map(|(_, _, started)| ready && started.elapsed() > SLOW_TICK)
            .unwrap_or(false);
        if over {
            // No cancel here: the isolate thread clears the terminate
            // itself once it has returned from the interrupted tick.
            self.fire_terminate();
        }
        let _ = self.tx.send(IsolateCmd::Pause);
    }

    /// Re-arm tick dispatch after [`LoadIsolate::pause`].
    pub fn resume(&self) {
        let _ = self.tx.send(IsolateCmd::Resume);
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
        let _ = self.tx.send(IsolateCmd::PaintClick {
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
        let _ = self.tx.send(IsolateCmd::PaintSelect {
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
            self.fire_terminate();
            let line = if tick == RECOVERY_ANCHOR_TICK {
                format!("interrupted slow recoveryAnchor ({elapsed:?})")
            } else {
                format!("interrupted slow tick {tick} ({elapsed:?})")
            };
            self.logs.lock().unwrap().push(line);
        }
        let _ = self.tx.send(IsolateCmd::RecoveryAnchor { generation });
    }

    /// Evaluate `expr` in the isolate's global scope and return its
    /// JSON value (test/status read-back; e.g. `"__rs_bot.n"`).
    pub fn probe(&self, expr: &str) -> Result<serde_json::Value, String> {
        let (tx, rx) = mpsc::channel::<Result<serde_json::Value, String>>();
        self.tx
            .send(IsolateCmd::Probe(expr.to_string(), tx))
            .map_err(|e| e.to_string())?;
        rx.recv_timeout(Duration::from_secs(10))
            .map_err(|e| format!("probe: {e}"))?
    }

    /// The bot instance's random-ignore list (`inst.ignoredRandoms?.()`
    /// on `__rs_bot`, default `[]`): cached on the isolate thread
    /// after each tick (see [`ThreadMsg::IgnoredRandoms`]). A throwing /
    /// non-array method and a native `tick`-shaped card (no instance)
    /// fail closed to `[]`. No probe round-trip.
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

    /// Drain the interact requests the tick's shim queued
    /// (`__rs2b0t_host.interact`), forwarded by the tick thread in
    /// tick order. The host dispatches them through the slot Driver;
    /// a malformed entry is logged and dropped, never fatal.
    pub fn drain_interacts(&self) -> Vec<crate::shim::InteractReq> {
        self.pump_logs();
        std::mem::take(&mut *self.interacts.lock().unwrap())
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
            self.paint_generation
                .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
            interacts.clear();
            self.lifecycle.lock().unwrap().clear();
        }
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
        let _ = self.tx.send(IsolateCmd::ResetSession);
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
        // `Err(true)` is a disconnected channel; `Err(false)` nothing yet.
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
        let outcome = match received {
            Ok(Ok(handle)) => {
                let _ = self.terminate.set(handle);
                // A tick queued before setup finished starts running now:
                // its slow-tick budget is measured from here, not its send.
                if let Some(entry) = self.in_flight.lock().unwrap().as_mut() {
                    entry.2 = Instant::now();
                }
                Ready::Ready
            }
            Ok(Err(e)) => Ready::Failed(e),
            Err(true) => Ready::Failed(format!(
                "isolate init: {}",
                mpsc::RecvTimeoutError::Disconnected
            )),
            Err(false) if Instant::now() < deadline => return Ready::Pending,
            Err(false) => {
                Ready::Failed(format!("isolate init: {}", mpsc::RecvTimeoutError::Timeout))
            }
        };
        *setup = match &outcome {
            Ready::Ready => SetupState::Ready,
            Ready::Failed(e) => SetupState::Failed(e.clone()),
            Ready::Pending => unreachable!("pending returns above"),
        };
        outcome
    }

    fn fire_terminate(&self) {
        if let Some(handle) = self.terminate.get() {
            handle.terminate_execution();
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
    pub fn join(mut self) -> Vec<String> {
        let _ = self.tx.send(IsolateCmd::Stop { invoke_hook: true });
        let _ = self.resolve_setup(Some(Instant::now() + SETUP_TIMEOUT));
        {
            let mut st = self.teardown.lock().unwrap();
            if st.phase == TeardownPhase::Running {
                st.phase = TeardownPhase::UnwindingTick;
                self.fire_terminate();
            }
        }
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
        let mut msgs = Vec::new();
        {
            let rx = self.rx.lock().unwrap();
            while let Ok(msg) = rx.try_recv() {
                msgs.push(msg);
            }
        }
        for msg in msgs {
            match msg {
                ThreadMsg::Log(line) => self.logs.lock().unwrap().push(line),
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
                ThreadMsg::Completed { tick, generation } => {
                    let mut in_flight = self.in_flight.lock().unwrap();
                    if generation
                        != self
                            .work_generation
                            .load(std::sync::atomic::Ordering::Acquire)
                    {
                        continue;
                    }
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
        let _ = self.poll_ready();
        let _ = self.tx.send(IsolateCmd::Stop { invoke_hook: false });
        let mut st = self.teardown.lock().unwrap();
        match st.phase {
            TeardownPhase::Done => {}
            TeardownPhase::Running | TeardownPhase::UnwindingTick | TeardownPhase::Hook => {
                st.phase = TeardownPhase::Done;
                st.cancel.take();
                self.fire_terminate();
            }
        }
        drop(st);
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

/// Read the bot instance's ignore list on the isolate thread (no probe).
fn eval_ignored_randoms(runtime: &mut Runtime) -> Vec<String> {
    runtime
        .eval::<Vec<String>>(
            "(() => { const b = globalThis.__rs_bot; if (!b || typeof b.ignoredRandoms !== 'function') return []; const l = b.ignoredRandoms(); return Array.isArray(l) ? l.filter(x => typeof x === 'string') : []; })()",
        )
        .unwrap_or_default()
}

/// The isolate thread: create the Runtime, wire the module, hand the
/// thread-safe isolate handle back, then run the tick loop.
#[allow(clippy::too_many_arguments)] // channel endpoints plus optional diagnostics
fn isolate_main(
    source: String,
    shape: LoadShape,
    siblings: Vec<(String, String)>,
    game_data: Option<std::sync::Arc<api::game_data::SelectedGameData>>,
    named_banks: std::sync::Arc<api::named_banks::NamedBankFacts>,
    cmds: Receiver<IsolateCmd>,
    out: Sender<ThreadMsg>,
    setup: Sender<Result<v8::IsolateHandle, String>>,
    work_generation: std::sync::Arc<std::sync::atomic::AtomicU64>,
    paint_generation: std::sync::Arc<std::sync::atomic::AtomicU64>,
    teardown: std::sync::Arc<Mutex<TeardownState>>,
    proof: std::sync::Arc<TeardownProofInner>,
    #[cfg(feature = "memory-profile")] counters: std::sync::Arc<crate::memory_profile::Counters>,
) {
    #[cfg(feature = "memory-profile")]
    let _heap_lifetime = crate::memory_profile::HeapLifetime(counters.clone());
    let mut runtime = match Runtime::new(RuntimeOptions {
        timeout: RUNTIME_TIMEOUT,
        max_heap_size: Some(MAX_HEAP),
        ..Default::default()
    }) {
        Ok(runtime) => runtime,
        Err(e) => {
            let _ = setup.send(Err(format!("js engine init: {e}")));
            return;
        }
    };
    // Declared after `runtime`, so a failed wire (whose module code may
    // have started a machine) drops the rows before the isolate.
    let _machines = MachinesStop;
    #[cfg(feature = "memory-profile")]
    counters
        .heap_live
        .store(1, std::sync::atomic::Ordering::Relaxed);
    if let Err(e) = wire_runtime(
        &mut runtime,
        &source,
        shape,
        &siblings,
        game_data,
        named_banks,
    ) {
        let _ = setup.send(Err(e));
        return;
    }
    // The native-event consumer ships only with the compat runner
    // (`COMPAT_RUNNER` defines `__rs2b0t_flush_native_events`), and
    // `wire_runtime` has already evaluated the main module, so this read
    // is final for the isolate's life. Native shapes (`tick(api)` and
    // every v2 card) have no events API: building and staging a batch
    // for them would grow a queue nothing drains. A future v2
    // `api.on(...)` only has to define the flush global; delivery turns
    // back on here with no other change.
    let events_consumed = runtime
        .eval::<bool>("typeof globalThis.__rs2b0t_flush_native_events === 'function'")
        .unwrap_or(false);
    let v2_native = shape == LoadShape::NativeTick
        && matches!(
            super::shape::parse_declared_api_version(&source),
            Ok(Some(2))
        );
    let _ = runtime.eval::<serde_json::Value>(INSTALL_ON_STOP);
    let terminate = runtime.deno_runtime().v8_isolate().thread_safe_handle();
    let _ = setup.send(Ok(terminate));
    tick_loop(
        runtime,
        cmds,
        out,
        work_generation,
        paint_generation,
        teardown,
        proof,
        v2_native,
        events_consumed,
        matches!(shape, LoadShape::CompatDefineBot | LoadShape::CompatClass),
        #[cfg(feature = "memory-profile")]
        counters,
    );
}

fn deliver_native_events(
    runtime: &mut Runtime,
    events: &[crate::events::NativeEvent],
    out: &Sender<ThreadMsg>,
) {
    if events.is_empty() {
        return;
    }
    if let Err(e) = dispatch_native_events(runtime, events) {
        let _ = out.send(ThreadMsg::Log(format!("native events: {e}")));
    }
}

/// This tick's script paint frame: the native recorder composed with the
/// user `Paint.end` record. A compat bot that may not paint yet forwards
/// an empty frame instead — rs2b0t clears the script layer while
/// `paintBot` is null (`panel/Overlay.ts:43-58`).
fn paint_frame(
    runtime: &mut Runtime,
    script_paint: bool,
) -> Result<crate::shim::ScriptPaint, rustyscript::Error> {
    if !script_paint {
        return Ok(crate::shim::ScriptPaint::default());
    }
    let user: Option<crate::shim::ScriptPaint> =
        runtime.eval("globalThis.__rs2b0t_host.paint || null")?;
    Ok(crate::canvas::compose_paint(user))
}

/// rs2b0t `ScriptRunner.paintBot` (`runtime/ScriptRunner.ts:141-151`): a
/// compat bot's `onPaint` runs only after `onStart` completed
/// (`startupComplete`, set at `:220`) and while `loopReadyOrDetached()`
/// holds (`:38-59`): in game, scene state 2, a local tile, and stats
/// loaded — or detached, which here is an isolate that was never posted a
/// session (`ingame` absent), as rs2b0t treats an unattached reader.
/// rs2b0t's `statsReady` also requires each stat to arrive in this login.
/// The host empties posted stats at logout and on every session change
/// (`GameSnapshot::reset_session`), so the gate closes until the new
/// session's first `UPDATE_STAT`; this uses the `activeStatsReady` rule
/// (every used stat's base level above 0). The only gap left is a mix of
/// old and new values inside the same account's login stat burst: exact
/// per-slot parity would need a per-slot seen generation in the client.
/// The script-state term (running or paused) is implicit: the isolate
/// paints only on ticks it runs.
fn compat_may_paint(runner: &Runner) -> bool {
    runner.start_ok
        && crate::observed::with(|scene| {
            let scene = scene.latest();
            match scene.ingame() {
                None => true,
                Some(ingame) => {
                    ingame
                        && scene.scene_state() == Some(2)
                        && scene.here().is_some()
                        && scene.stats().is_some_and(|stats| stats.ready)
                }
            }
        })
}

/// Forward the recorder's frame when it differs from the last one sent,
/// stamped with the session it belongs to. The frame is built here, so it
/// crosses as the typed value the host reads — no codec round trip, and no
/// copy per reader.
fn forward_paint_if_changed(
    out: &Sender<ThreadMsg>,
    last: &mut Option<std::sync::Arc<crate::shim::ScriptPaint>>,
    paint_generation: &std::sync::atomic::AtomicU64,
    mut frame: crate::shim::ScriptPaint,
) {
    frame.generation = paint_generation.load(std::sync::atomic::Ordering::Acquire);
    if last.as_deref() == Some(&frame) {
        return;
    }
    let frame = std::sync::Arc::new(frame);
    // Remembered even when it is capped, so an over-cap frame is logged once
    // per change (the wire decoder used to drop it on the host side).
    *last = Some(std::sync::Arc::clone(&frame));
    if let Err(e) = crate::isolate_fb::cap_paint(&frame) {
        let _ = out.send(ThreadMsg::Log(format!("paint: {e}")));
        return;
    }
    let _ = out.send(ThreadMsg::Paint(frame));
}

/// Drop an unconsumed one-shot so a later paint cannot return a stale id.
fn clear_unconsumed_paint_click(runtime: &mut Runtime) {
    let _ = runtime.eval::<()>(
        "if (globalThis.__rs2b0t_host && globalThis.__rs2b0t_host.paintClick != null) { globalThis.__rs2b0t_host.paintClick = null; }",
    );
}

fn set_paint_click(runtime: &mut Runtime, id: &str) -> Result<(), String> {
    let json = serde_json::to_string(id).map_err(|e| e.to_string())?;
    runtime
        .eval::<()>(&format!("globalThis.__rs2b0t_host.paintClick = {json};"))
        .map_err(|e| e.to_string())
}

/// Drain Execution wait enqueue/settle counters. Each increment is a
/// real lifecycle fact, including settle+repark in the same pump.
fn take_wait_facts(runtime: &mut Runtime) -> (u32, u32) {
    let counts: Result<Vec<u32>, rustyscript::Error> = runtime.eval(
        "(() => { const h = globalThis.__rs2b0t_host || {}; const e = h.waitEnqueues | 0; const s = h.waitSettles | 0; h.waitEnqueues = 0; h.waitSettles = 0; return [e, s]; })()",
    );
    match counts.ok().as_deref() {
        Some([e, s, ..]) => (*e, *s),
        _ => (0, 0),
    }
}

fn append_wait_facts(reqs: &mut Vec<crate::shim::InteractReq>, enqueued: u32, settled: u32) {
    for _ in 0..enqueued {
        reqs.push(crate::shim::InteractReq::WaitEnqueued);
    }
    for _ in 0..settled {
        reqs.push(crate::shim::InteractReq::WaitSettled);
    }
}

/// Run this tick's event loop for up to 10 ms. An unhandled promise
/// rejection — an un-awaited shim call that failed — surfaces here once
/// as the drain's error; log it under the tick instead of dropping it.
fn drain_event_loop(runtime: &mut Runtime, out: &Sender<ThreadMsg>, n: u64) {
    if let Err(e) = runtime.block_on_event_loop(
        rustyscript::deno_core::PollEventLoopOptions::default(),
        Some(Duration::from_millis(10)),
    ) {
        let _ = out.send(ThreadMsg::Log(format!("tick {n}: {e}")));
    }
}

/// Log what the tick's callbacks left on the host handle, under the tick
/// number: the recorded error (a throwing onPaint or event callback, a
/// rejected v2 tick), then `LoopingBot.log` / `this.log` lines, so
/// BOT_DEBUG and the panel can see script-side lines.
fn forward_script_logs(runtime: &mut Runtime, out: &Sender<ThreadMsg>, n: u64) {
    let err: Option<String> = runtime
        .eval("(() => { const e = globalThis.__rs2b0t_host.lastError; if (e) { globalThis.__rs2b0t_host.lastError = null; return e; } return null; })()")
        .unwrap_or(None);
    if let Some(e) = err {
        let _ = out.send(ThreadMsg::Log(format!("tick {n}: {e}")));
    }
    let rows: Result<Vec<String>, rustyscript::Error> = runtime.eval(DRAIN_BOT_LOG);
    for line in rows.unwrap_or_default() {
        let _ = out.send(ThreadMsg::Log(line));
    }
}

/// The settle promise of a compat method invoker or a v1 native tick: it
/// fulfils `null` on success or the error text, and never rejects.
type Settle = rustyscript::js_value::Promise<Option<String>>;

/// Rust-owned single-flight for every non-v2 shape. Compat `onStart`
/// runs once and gates `loop()`; `loop()` (or a v1 native tick that
/// returned a promise) is never re-entered while its promise is pending.
/// Only the runner's own promise holds it: an Execution wait parked by a
/// listener or an un-awaited helper does not.
struct Runner {
    phase: Phase,
    /// `onStart` settled successfully — rs2b0t `ScriptRunner.startupComplete`
    /// (`ScriptRunner.ts:220`). Native shapes have no onStart: always set.
    start_ok: bool,
}

enum Phase {
    /// Compat card whose `onStart` has not been invoked.
    Unstarted,
    /// Compat `onStart` in flight.
    Starting(Settle),
    /// `onStart` failed during this tick. The first `loop()` waits for
    /// the next eligible tick, as the pre-F02 runner did.
    StartFailed,
    /// Nothing in flight: the next eligible tick invokes `loop()`/`tick`.
    Idle,
    /// `loop()` or the native tick in flight.
    Running(Settle),
}

impl Runner {
    fn new(compat: bool) -> Self {
        Self {
            phase: if compat {
                Phase::Unstarted
            } else {
                Phase::Idle
            },
            start_ok: !compat,
        }
    }

    /// `onStart` has settled, so its subscriptions exist.
    fn started(&self) -> bool {
        matches!(self.phase, Phase::Idle | Phase::Running(_))
    }

    /// Observe the in-flight promise; on settle log its error and go
    /// idle (`StartFailed` for a failed `onStart`). `true` when a
    /// `loop()`/tick fulfilled cleanly.
    fn poll(&mut self, runtime: &mut Runtime, out: &Sender<ThreadMsg>, n: u64) -> bool {
        let (state, is_loop) = match &self.phase {
            Phase::Starting(p) => (p.poll_promise(runtime), false),
            Phase::Running(p) => (p.poll_promise(runtime), true),
            Phase::Unstarted | Phase::StartFailed | Phase::Idle => return false,
        };
        let err = match state {
            std::task::Poll::Pending => return false,
            std::task::Poll::Ready(Ok(err)) => err,
            std::task::Poll::Ready(Err(e)) => Some(e.to_string()),
        };
        self.phase = if err.is_some() && !is_loop {
            Phase::StartFailed
        } else {
            Phase::Idle
        };
        self.start_ok |= !is_loop && err.is_none();
        match err {
            Some(e) => {
                let _ = out.send(ThreadMsg::Log(format!("tick {n}: {e}")));
                false
            }
            None => is_loop,
        }
    }
}

/// One eligible non-v2 tick, in the phase order the isolate owns (the
/// tick is already recorded and machines stepped): tick listeners, wait
/// settle, `onStart` once (compat), native events once started, then
/// `loop()`/`tick` when nothing is in flight. Sets `loop_settled` when a
/// compat `loop()` settle is observed here.
///
/// A running `loop()` is polled before any of these phases' JS runs: one
/// whose wait settles in this tick's pump (continuations run as each
/// call returns) finishes this tick, and the next `loop()` starts on the
/// next one — at most one `loop()` start per tick. A settling `onStart`
/// is re-polled after the pump so the first `loop()` follows a successful
/// one at once; after a failed one it starts on the next tick.
fn run_tick_phases(
    runtime: &mut Runtime,
    runner: &mut Runner,
    n: u64,
    compat: bool,
    events_consumed: bool,
    out: &Sender<ThreadMsg>,
    loop_settled: &mut bool,
    teardown: &Mutex<TeardownState>,
) -> Result<(), rustyscript::Error> {
    *loop_settled |= runner.poll(runtime, out, n) && compat;
    if let Phase::StartFailed = runner.phase {
        runner.phase = Phase::Idle;
    }
    // BotHost tick listeners, before any wait settles this tick. Absent
    // when the card never loaded BotHost.
    if tick_claimed(teardown) {
        return Ok(());
    }
    let _ =
        runtime.call_function_immediate::<()>(None, "__rs2b0t_fire_tick_listeners", json_args!());
    if tick_claimed(teardown) {
        return Ok(());
    }
    runtime.call_function_immediate::<()>(None, "__rs2b0t_pump", json_args!(n))?;
    if tick_claimed(teardown) {
        return Ok(());
    }
    match runner.phase {
        // An onStart a listener or a settled wait just finished lets the
        // first `loop()` run on this tick.
        Phase::Starting(_) => {
            runner.poll(runtime, out, n);
        }
        Phase::Unstarted => {
            // onStart is invoked exactly once: a failed call counts as a
            // failed onStart.
            runner.phase = Phase::StartFailed;
            let start: Settle =
                runtime.call_function_immediate(None, "__rs2b0t_compat_on_start", json_args!())?;
            runner.phase = Phase::Starting(start);
            // Microtasks run as the call returns, so a synchronous
            // onStart has settled here.
            runner.poll(runtime, out, n);
        }
        Phase::StartFailed | Phase::Idle | Phase::Running(_) => {}
    }
    if events_consumed && runner.started() && !tick_claimed(teardown) {
        runtime.call_function_immediate::<()>(
            None,
            "__rs2b0t_flush_native_events",
            json_args!(),
        )?;
    }
    if tick_claimed(teardown) {
        return Ok(());
    }
    if let Phase::Idle = runner.phase {
        if compat {
            let run: Settle =
                runtime.call_function_immediate(None, "__rs2b0t_compat_loop", json_args!())?;
            runner.phase = Phase::Running(run);
        } else {
            let run: Option<Settle> =
                runtime.call_function_immediate(None, "__rs_tick", json_args!(n))?;
            if let Some(run) = run {
                runner.phase = Phase::Running(run);
            }
        }
    }
    Ok(())
}

/// Sample recoveryAnchor on the isolate thread. Invalid/missing/throw → none.
fn eval_recovery_anchor(runtime: &mut Runtime) -> Option<(i32, i32, i32)> {
    let value: Result<Option<Vec<i32>>, rustyscript::Error> = runtime.eval(
        r#"(() => {
            try {
                const inst = globalThis.__rs_bot;
                if (!inst || typeof inst.recoveryAnchor !== 'function') return null;
                const a = inst.recoveryAnchor();
                if (!a || typeof a !== 'object' || Array.isArray(a)) return null;
                const x = a.x, z = a.z, level = a.level;
                if (!Number.isInteger(x) || !Number.isInteger(z) || !Number.isInteger(level)) return null;
                return [x, z, level];
            } catch (_) { return null; }
        })()"#,
    );
    match value {
        Ok(Some(coords)) if coords.len() >= 3 => Some((coords[0], coords[1], coords[2])),
        _ => None,
    }
}

const DRAIN_BOT_LOG: &str = "(() => { const h = globalThis.__rs2b0t_host; const rows = h && h.log; if (!Array.isArray(rows) || rows.length === 0) return []; h.log = []; return rows.map(String); })()";

const INSTALL_ON_STOP: &str = r#"void (globalThis.__rs2b0t_invoke_on_stop = function () {
  try {
const inst = globalThis.__rs_bot;
if (!inst || typeof inst.onStop !== 'function') return null;
inst.onStop();
return null;
  } catch (e) {
return String((e && (e.message || e.stack)) || e);
  }
}, globalThis.__rs2b0t_drain_log = function () {
  const h = globalThis.__rs2b0t_host;
  const rows = h && h.log;
  if (!Array.isArray(rows) || rows.length === 0) return [];
  h.log = [];
  return rows.map(String);
}, 0)"#;

fn enter_teardown_hook(teardown: &std::sync::Arc<Mutex<TeardownState>>) -> bool {
    let mut st = teardown.lock().unwrap();
    match st.phase {
        TeardownPhase::Hook | TeardownPhase::Done => false,
        TeardownPhase::Running | TeardownPhase::UnwindingTick => {
            st.phase = TeardownPhase::Hook;
            st.deadline = Some(Instant::now() + SLOW_TICK);
            st.interrupt_issued = false;
            true
        }
    }
}

fn finish_teardown_hook(teardown: &std::sync::Arc<Mutex<TeardownState>>) {
    let mut st = teardown.lock().unwrap();
    st.phase = TeardownPhase::Done;
    st.cancel.take();
}

fn drain_bot_log(runtime: &mut Runtime, out: &Sender<ThreadMsg>) {
    let bot_log: Result<Vec<String>, rustyscript::Error> =
        runtime.call_function_immediate(None, "__rs2b0t_drain_log", json_args!());
    if let Ok(rows) = bot_log {
        for line in rows {
            let _ = out.send(ThreadMsg::Log(line));
        }
    }
}

fn take_hook_entry_delay(teardown: &std::sync::Arc<Mutex<TeardownState>>) -> Option<Duration> {
    teardown.lock().unwrap().hook_entry_delay.take()
}

/// One-shot worker spawned only at Hook entry. Sleeps until the
/// Hook-entry deadline, then issues at most one terminate while still
/// Hook, under the same mutex as finish. Cancelled by dropping `cancel`.
///
/// The old tick interrupt is cleared under the Hook lock *before* spawn
/// so a deschedule cannot let this worker fire and then be cancelled.
fn arm_hook_deadline(
    runtime: &mut Runtime,
    teardown: &std::sync::Arc<Mutex<TeardownState>>,
    proof: &std::sync::Arc<TeardownProofInner>,
) -> Option<JoinHandle<()>> {
    let handle = runtime.deno_runtime().v8_isolate().thread_safe_handle();
    let (cancel_tx, cancel_rx) = mpsc::channel::<()>();
    let deadline = {
        let mut st = teardown.lock().unwrap();
        if st.phase != TeardownPhase::Hook {
            return None;
        }
        runtime
            .deno_runtime()
            .v8_isolate()
            .cancel_terminate_execution();
        if st.fail_deadline_spawn {
            return None;
        }
        st.cancel = Some(cancel_tx);
        st.deadline.unwrap_or_else(|| Instant::now() + SLOW_TICK)
    };
    let wd_teardown = teardown.clone();
    let wd_proof = proof.clone();
    match std::thread::Builder::new()
        .name("js-onstop-deadline".into())
        .spawn(move || {
            wd_proof
                .worker_live
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let _guard = DeadlineWorkerGuard { proof: wd_proof };
            let remaining = deadline.saturating_duration_since(Instant::now());
            match cancel_rx.recv_timeout(remaining) {
                Ok(()) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            }
            let mut st = wd_teardown.lock().unwrap();
            if st.phase == TeardownPhase::Hook && !st.interrupt_issued {
                st.interrupt_issued = true;
                handle.terminate_execution();
            }
        }) {
        Ok(h) => {
            if let Some(delay) = take_hook_entry_delay(teardown) {
                std::thread::sleep(delay);
            }
            Some(h)
        }
        Err(_) => {
            teardown.lock().unwrap().cancel.take();
            None
        }
    }
}

fn complete_teardown_without_hook(
    runtime: &mut Runtime,
    out: &Sender<ThreadMsg>,
    teardown: &std::sync::Arc<Mutex<TeardownState>>,
    diagnostic: Option<&str>,
) {
    if let Some(line) = diagnostic {
        let _ = out.send(ThreadMsg::Log(line.to_string()));
    }
    finish_teardown_hook(teardown);
    runtime
        .deno_runtime()
        .v8_isolate()
        .cancel_terminate_execution();
    let _ = out.send(ThreadMsg::Stopped);
}

/// Exactly-once isolate-thread teardown. The 50 ms deadline is the
/// Instant captured at Hook entry; a one-shot worker (not a parked
/// per-bot thread) issues at most one interrupt under the same mutex
/// as finish. Getter, body, and log-drain share that budget.
///
/// Fail closed: if no deadline owner can be created, skip user
/// getter/body/drain, emit a native diagnostic, and finish cleanup.
fn teardown_once(
    runtime: &mut Runtime,
    out: &Sender<ThreadMsg>,
    invoke_hook: bool,
    teardown: &std::sync::Arc<Mutex<TeardownState>>,
    proof: &std::sync::Arc<TeardownProofInner>,
) {
    if !enter_teardown_hook(teardown) {
        return;
    }
    if !invoke_hook {
        complete_teardown_without_hook(runtime, out, teardown, None);
        return;
    }
    let Some(worker) = arm_hook_deadline(runtime, teardown, proof) else {
        complete_teardown_without_hook(
            runtime,
            out,
            teardown,
            Some("onStop skipped: no deadline owner"),
        );
        return;
    };
    proof
        .invoked
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let threw: Result<Option<String>, rustyscript::Error> =
        runtime.call_function_immediate(None, "__rs2b0t_invoke_on_stop", json_args!());
    match threw {
        Ok(Some(msg)) => {
            let _ = out.send(ThreadMsg::Log(format!("onStop threw: {msg}")));
        }
        Ok(None) => {}
        Err(e) => {
            let _ = out.send(ThreadMsg::Log(format!("onStop threw: {e}")));
        }
    }
    drain_bot_log(runtime, out);
    let _ = runtime.deno_runtime().execute_script(
        "<onStop-clear-interact>",
        "if (globalThis.__rs2b0t_host) globalThis.__rs2b0t_host.interact = []",
    );
    finish_teardown_hook(teardown);
    runtime
        .deno_runtime()
        .v8_isolate()
        .cancel_terminate_execution();
    let _ = worker.join();
    let _ = out.send(ThreadMsg::Stopped);
}

fn script_stop_requested(runtime: &mut Runtime) -> bool {
    runtime
        .eval::<bool>("!!(globalThis.__rs2b0t_host && globalThis.__rs2b0t_host.stopRequested)")
        .unwrap_or(false)
}

const STOP_REASON_MAX_BYTES: usize = 256;

fn script_stop_reason(runtime: &mut Runtime) -> String {
    let mut reason = runtime
        .eval::<Option<String>>(
            "(() => { const h = globalThis.__rs2b0t_host; return h && typeof h.stopReason === 'string' ? h.stopReason : null; })()",
        )
        .unwrap_or(None)
        .unwrap_or_default();
    if reason.len() > STOP_REASON_MAX_BYTES {
        let mut end = STOP_REASON_MAX_BYTES;
        while !reason.is_char_boundary(end) {
            end -= 1;
        }
        reason.truncate(end);
    }
    reason
}

/// The tick loop: commands are serialized on this thread; ticks run
/// with a time budget, slow ticks are logged and stale queued ticks are
/// skipped, and errors never kill the isolate.
///
/// `events_consumed` is false for every native shape: the event producer
/// is then never observed and no batch is built, so the isolate pays
/// nothing for events nothing can receive.
///
/// The stale-skip drain consumes commands with an explicit match so a
/// non-Tick command (Pause/Resume/Probe/Stop/PaintClick) that arrives while ticks
/// are queued is stashed for the next iteration instead of being
/// dropped (a `while let Ok(IsolateCmd::Tick(..))` pattern would
/// swallow it).
fn tick_loop(
    mut runtime: Runtime,
    cmds: Receiver<IsolateCmd>,
    out: Sender<ThreadMsg>,
    work_generation: std::sync::Arc<std::sync::atomic::AtomicU64>,
    paint_generation: std::sync::Arc<std::sync::atomic::AtomicU64>,
    teardown: std::sync::Arc<Mutex<TeardownState>>,
    proof: std::sync::Arc<TeardownProofInner>,
    v2_native: bool,
    events_consumed: bool,
    compat: bool,
    #[cfg(feature = "memory-profile")] counters: std::sync::Arc<crate::memory_profile::Counters>,
) {
    let _finish = TickLoopFinish(proof.clone());
    // Locals drop before the `runtime` parameter: machine rows (and any
    // V8 handles they hold) never outlive the isolate.
    let _machines = MachinesStop;
    #[cfg(feature = "memory-profile")]
    let mut last_heap_sample = None::<Instant>;
    let mut paused = false;
    let mut pending: Option<IsolateCmd> = None;
    // Host-owned hold gate (SEC-004): set from the posted FlatBuffer
    // snapshot, never from a JS-writable `__rs2b0t_host.hold`.
    let mut host_hold = false;
    let mut event_producer = crate::events::NativeEventProducer::new();
    // One reusable encode buffer for this V8 isolate's interact batches
    // (`reset` between messages). Paint frames cross typed, not encoded.
    let mut ipc = crate::isolate_fb::IsolateBuf::new();
    let mut last_forwarded_paint: Option<std::sync::Arc<crate::shim::ScriptPaint>> = None;
    let mut mouse_gestures = MouseGestureIdentities::default();
    let mut runner = Runner::new(compat);
    loop {
        #[cfg(feature = "memory-profile")]
        if last_heap_sample.is_none_or(|t| t.elapsed() >= Duration::from_secs(1)) {
            use std::sync::atomic::Ordering::Relaxed;
            let heap = runtime.deno_runtime().v8_isolate().get_heap_statistics();
            counters
                .heap_used
                .store(heap.used_heap_size() as u64, Relaxed);
            counters
                .heap_total
                .store(heap.total_heap_size() as u64, Relaxed);
            counters.heap_updated_ms.store(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64,
                Relaxed,
            );
            counters.heap_samples.fetch_add(1, Relaxed);
            last_heap_sample = Some(Instant::now());
        }
        let cmd = match pending.take() {
            Some(cmd) => cmd,
            None => match cmds.recv() {
                Ok(cmd) => cmd,
                Err(_) => break,
            },
        };
        match cmd {
            IsolateCmd::Snapshot(bytes) => {
                // Decode the posted FlatBuffer once into the isolate
                // scene the step machines read, then materialise the JS
                // object the shim reads on the host handle. A malformed
                // blob is logged, never fatal.
                match crate::isolate_fb::SnapshotReader::from_bytes(&bytes.bytes) {
                    Ok(snap) => {
                        // Step machines read the scene at call time; the
                        // hooks below are the edge-triggered waits.
                        crate::observed::apply(&snap);
                        super::reach_query::apply(&snap);
                        crate::walk_wait::on_snapshot(&snap);
                        crate::inspect_wait::on_snapshot(&snap);
                        if let Some((seq, inspect_generation)) =
                            crate::inspect_wait::take_pending_ack()
                        {
                            let generation =
                                work_generation.load(std::sync::atomic::Ordering::Acquire);
                            let req = crate::shim::InteractReq::InspectAck {
                                seq,
                                generation: inspect_generation,
                            };
                            let _ = out.send(ThreadMsg::Interact {
                                bytes: ipc.encode_interact_batch(&[req]),
                                generation,
                            });
                        }
                        crate::dialog::on_snapshot();
                        crate::reach::on_snapshot(&snap);
                        if snap.has_hold() {
                            host_hold = snap.hold();
                            crate::periodic_bank::on_hold(host_hold);
                            crate::cake_stall::on_hold(host_hold);
                            crate::walk_wait::on_hold(host_hold);
                            crate::inspect_wait::on_hold(host_hold);
                            crate::death_recovery::on_hold(host_hold);
                            crate::machine::on_hold(host_hold);
                            crate::shop::on_hold(host_hold);
                            crate::hunt_fight::on_hold(host_hold);
                            crate::hunt_lair::on_hold(host_hold);
                            crate::hunt_leave::on_hold(host_hold);
                            crate::hunt_key::on_hold(host_hold);
                            crate::hunt_cell::on_hold(host_hold);
                            crate::hunt_bank::on_hold(host_hold);
                            crate::production::on_hold(host_hold);
                            crate::dialog::on_hold(host_hold);
                            crate::quest_journal::on_hold(host_hold);
                            crate::clue::on_hold(host_hold);
                            crate::reach::on_hold(host_hold);
                            crate::trade::on_hold(host_hold);
                            crate::drive_partner_trade::on_hold(host_hold);
                        }
                        if let Err(e) = materialize_snapshot(&mut runtime, &snap, host_hold) {
                            let _ = out.send(ThreadMsg::Log(format!("snapshot: {e}")));
                        } else if events_consumed {
                            let observed = event_producer.observe(&snap);
                            if let Some(diag) = observed.diagnostic {
                                let _ = out.send(ThreadMsg::Log(diag));
                            }
                        }
                    }
                    Err(e) => {
                        let _ = out.send(ThreadMsg::Log(format!("snapshot: {e}")));
                    }
                }
            }
            IsolateCmd::Loadouts(json) => {
                if let Err(e) =
                    runtime.eval::<()>(&format!("globalThis.__rs2b0t_host.loadouts = {json};"))
                {
                    let _ = out.send(ThreadMsg::Log(format!("loadouts: {e}")));
                }
            }
            IsolateCmd::Settings(json) => {
                if let Err(e) = materialize_settings_bag(&mut runtime, &json) {
                    let _ = out.send(ThreadMsg::Log(format!("settings: {e}")));
                }
            }
            IsolateCmd::Tick {
                tick: n,
                generation,
                input_identity,
            } => {
                if paused
                    || generation != work_generation.load(std::sync::atomic::Ordering::Acquire)
                {
                    continue;
                }
                // Stop is queued behind this tick and join has armed its
                // terminate for it: run no script, so that terminate
                // reaches the hook's cancel instead of an ignored call.
                if tick_claimed(&teardown) {
                    continue;
                }
                let start = Instant::now();
                // Every shape records the tick first, so machine callbacks,
                // listeners and waits all see this tick's number.
                record_tick(&mut runtime, n);
                // Step machines read the scene the Snapshot command just
                // applied; they run before the tick's other JS, and a
                // completion settles in this tick's pump. Join's claim is
                // re-checked between callbacks: one may absorb its terminate.
                super::machine_v8::step(&mut runtime, &|| tick_claimed(&teardown));
                if events_consumed {
                    let observed = event_producer.take_eligible();
                    if let Some(diag) = observed.diagnostic {
                        let _ = out.send(ThreadMsg::Log(diag));
                    }
                    deliver_native_events(&mut runtime, &observed.events, &out);
                }
                // Guardian hold: skip `loop()` AND skip resolving
                // parked conds (time waits too) — the wait stays parked
                // until the hold lifts. Still call `onPaint` so status
                // rows keep updating. Pause already freezes above.
                if host_hold {
                    // Paint-only tick: no loop, no pump. The single paint
                    // pass of a held tick. Use `__rs_bot` (global);
                    // module-local `inst` is not visible here.
                    let script_paint = !compat || compat_may_paint(&runner);
                    if !v2_native && script_paint {
                        let _ = runtime.eval::<()>("globalThis.__rs2b0t_call_on_paint()");
                    }
                    if events_consumed && runner.started() {
                        let _ = runtime.call_function_immediate::<()>(
                            None,
                            "__rs2b0t_flush_native_events",
                            json_args!(),
                        );
                    }
                    drain_event_loop(&mut runtime, &out, n);
                    forward_script_logs(&mut runtime, &out, n);
                    // Work that fulfils under hold is still scheduler
                    // progress; its gameplay is dropped below.
                    let mut lifecycle: Vec<crate::shim::InteractReq> = Vec::new();
                    if v2_native {
                        let rows: Result<Vec<crate::shim::MaybeInteractReq>, rustyscript::Error> =
                            runtime.eval("globalThis.__rs2b0t_host.interact || []");
                        lifecycle.extend(rows.unwrap_or_default().into_iter().filter_map(|row| {
                            match row {
                                crate::shim::MaybeInteractReq::Req(
                                    req @ crate::shim::InteractReq::LoopSettled,
                                ) => Some(req),
                                _ => None,
                            }
                        }));
                    } else if runner.poll(&mut runtime, &out, n) && compat {
                        lifecycle.push(crate::shim::InteractReq::LoopSettled);
                    }
                    if !lifecycle.is_empty() {
                        let _ = out.send(ThreadMsg::Interact {
                            bytes: ipc.encode_interact_batch(&lifecycle),
                            generation,
                        });
                    }
                    match paint_frame(&mut runtime, script_paint) {
                        Ok(frame) => {
                            forward_paint_if_changed(
                                &out,
                                &mut last_forwarded_paint,
                                &paint_generation,
                                frame,
                            );
                        }
                        Err(e) => {
                            let _ = out.send(ThreadMsg::Log(format!("paint eval: {e}")));
                        }
                    }
                    clear_unconsumed_paint_click(&mut runtime);
                    // Ownership boundary after callback eval + microtasks:
                    // drop public actions and cancel a terminate armed by
                    // a runaway listener so the next eligible tick recovers.
                    runtime
                        .deno_runtime()
                        .v8_isolate()
                        .cancel_terminate_execution();
                    let _ = runtime.eval::<()>(
                        "if (globalThis.__rs2b0t_host) globalThis.__rs2b0t_host.interact = []",
                    );
                    crate::machine::drop_ops();
                    let _ = out.send(ThreadMsg::IgnoredRandoms(eval_ignored_randoms(
                        &mut runtime,
                    )));
                    if script_stop_requested(&mut runtime) {
                        let _ = out.send(ThreadMsg::ScriptStopped {
                            tick: n,
                            reason: script_stop_reason(&mut runtime),
                        });
                        let _ = out.send(ThreadMsg::Completed {
                            tick: n,
                            generation,
                        });
                        let _ = out.send(ThreadMsg::Log(format!(
                            "script requested stop on tick {n}; isolate stopping"
                        )));
                        teardown_once(&mut runtime, &out, true, &teardown, &proof);
                        break;
                    }
                    let _ = out.send(ThreadMsg::Completed {
                        tick: n,
                        generation,
                    });
                    continue;
                }
                // Rust owns the tick phases and the single-flight. Every
                // shape settles its due Execution waits (the pump); v2
                // keeps its JS-flagged single-flight, every other shape
                // runs through the Rust `Runner`. Onward work lands in
                // the drain below.
                let mut loop_settled = false;
                let result: Result<(), rustyscript::Error> = if tick_claimed(&teardown) {
                    // A machine callback or native event absorbed join's
                    // terminate: the loop must not start after it.
                    Ok(())
                } else if v2_native {
                    let pumped =
                        runtime.call_function_immediate::<()>(None, "__rs2b0t_pump", json_args!(n));
                    // Do not re-enter tick while a previous returned
                    // Promise is pending. Snapshot posts still merge;
                    // this only skips tick.
                    let v2_pending = runtime
                        .eval::<bool>("!!globalThis.__rs_v2_tick_pending")
                        .unwrap_or(false);
                    let ticked = if v2_pending || tick_claimed(&teardown) {
                        Ok(())
                    } else {
                        runtime.call_function_immediate(None, "__rs_tick", json_args!(n))
                    };
                    pumped.and(ticked)
                } else {
                    run_tick_phases(
                        &mut runtime,
                        &mut runner,
                        n,
                        compat,
                        events_consumed,
                        &out,
                        &mut loop_settled,
                        &teardown,
                    )
                };
                // Eligible NativeTick only: pause, generation mismatch,
                // and guardian hold already `continue` above. The step
                // machines were stepped before this tick's JS; their
                // awaits settle in the pump below with every other wait.
                drain_event_loop(&mut runtime, &out, n);
                // The host may have armed `terminate_execution` to
                // interrupt a slow tick; clear it now that the tick's
                // JS frames have fully unwound. This is the only cancel
                // point — canceling from the host would race the
                // interrupt and make it a no-op.
                runtime
                    .deno_runtime()
                    .v8_isolate()
                    .cancel_terminate_execution();
                // A machine callback whose promise the pump (or the drain)
                // settled resumes its row in this tick, and a row that ends
                // here settles its await in this tick too.
                if !tick_claimed(&teardown) {
                    if let Err(e) =
                        super::machine_v8::resume(&mut runtime, &|| tick_claimed(&teardown))
                    {
                        let _ = out.send(ThreadMsg::Log(format!("tick {n}: machines: {e}")));
                    }
                }
                if !v2_native {
                    // A loop that finished in the drain frees the
                    // single-flight for the next tick.
                    loop_settled |= runner.poll(&mut runtime, &out, n) && compat;
                }
                let elapsed = start.elapsed();
                #[cfg(feature = "memory-profile")]
                counters.tick(elapsed);
                if let Err(e) = result {
                    let _ = out.send(ThreadMsg::Log(format!("tick {n}: {e}")));
                }
                // Forward the tick's shim interact queue (Bank/Banking
                // requests written to `__rs2b0t_host.interact`) to the
                // host, then clear it for the next tick. The queue is
                // evaluated only now, after the tick's JS (and any
                // parked continuation) has fully run, so a request
                // reaches the host exactly once. The queue is read
                // through the runtime's value bridge (v8 object walk,
                // not `JSON.parse`) and forwarded as a FlatBuffer
                // batch, not a stringified JSON document. Each row is
                // accepted or rejected locally so a malformed mouse
                // object cannot drop a sibling key.
                // Machine-emitted ops join the batch in Rust at the JS
                // queue position where they were emitted.
                let rows: Result<Vec<crate::shim::MaybeInteractReq>, rustyscript::Error> =
                    runtime.eval("globalThis.__rs2b0t_host.interact || []");
                let mut reqs = crate::machine::merge_ops(rows.unwrap_or_default());
                stamp_mouse_gesture_identities(&mut reqs, input_identity, &mut mouse_gestures);
                crate::inspect_wait::filter_public_inspect_wire(&mut reqs);
                let (enqueued, settled) = take_wait_facts(&mut runtime);
                if loop_settled {
                    reqs.push(crate::shim::InteractReq::LoopSettled);
                }
                append_wait_facts(&mut reqs, enqueued, settled);
                if !reqs.is_empty() {
                    let _ = out.send(ThreadMsg::Interact {
                        bytes: ipc.encode_interact_batch(&reqs),
                        generation,
                    });
                }
                let _ = runtime.eval::<()>("globalThis.__rs2b0t_host.interact = []");
                // Forward the tick's recorded paint frame
                // (`Paint.begin` … `end()` on the host handle) to the
                // host, so the script paint views read it without a
                // probe round-trip. Only non-empty frames are sent —
                // a tick that painted nothing leaves the last frame
                // in place (Stop drops the whole isolate). serde_v8
                // walks the v8 object into `ScriptPaint`; the channel
                // carries a FlatBuffer, never a `serde_json::Value`.
                // This is the tick's one onPaint pass: onPaint is sync
                // and never waits for `loop()`. A compat bot paints once
                // its onStart completed and the scene and stats are
                // ready (`compat_may_paint`), even while `loop()` is
                // parked; native shapes paint every tick.
                let script_paint = !compat || compat_may_paint(&runner);
                if !v2_native && script_paint {
                    let _ = runtime.eval::<()>("globalThis.__rs2b0t_call_on_paint()");
                }
                // After the paint pass, so a throwing onPaint is logged
                // under this tick.
                forward_script_logs(&mut runtime, &out, n);
                match paint_frame(&mut runtime, script_paint) {
                    Ok(frame) => {
                        forward_paint_if_changed(
                            &out,
                            &mut last_forwarded_paint,
                            &paint_generation,
                            frame,
                        );
                    }
                    Err(e) => {
                        let _ = out.send(ThreadMsg::Log(format!("paint eval: {e}")));
                    }
                }
                clear_unconsumed_paint_click(&mut runtime);
                let _ = out.send(ThreadMsg::IgnoredRandoms(eval_ignored_randoms(
                    &mut runtime,
                )));
                // ScriptRunner.stop signal: the script flags the host
                // handle. Fold the completed tick, log the stop, run
                // exactly-once onStop under the isolate-owned 50 ms
                // deadline, then break so the Runtime is dropped.
                if script_stop_requested(&mut runtime) {
                    let _ = out.send(ThreadMsg::ScriptStopped {
                        tick: n,
                        reason: script_stop_reason(&mut runtime),
                    });
                    let _ = out.send(ThreadMsg::Completed {
                        tick: n,
                        generation,
                    });
                    let _ = out.send(ThreadMsg::Log(format!(
                        "script requested stop on tick {n}; isolate stopping"
                    )));
                    teardown_once(&mut runtime, &out, true, &teardown, &proof);
                    break;
                }
                if elapsed > SLOW_TICK {
                    let _ = out.send(ThreadMsg::Log(format!("slow tick {n}: {elapsed:?}")));
                    // Skip stale queued ticks: a slow tick means the
                    // pump backed up, so only the newest matters.
                    let mut latest = n;
                    loop {
                        match cmds.try_recv() {
                            Ok(IsolateCmd::Tick {
                                tick: next,
                                generation: next_generation,
                                input_identity: _,
                            }) if next_generation == generation => latest = next,
                            Ok(other) => {
                                pending = Some(other);
                                break;
                            }
                            Err(_) => break,
                        }
                    }
                    if latest != n {
                        let _ =
                            out.send(ThreadMsg::Log(format!("skipped stale ticks -> {latest}")));
                    }
                    let _ = out.send(ThreadMsg::Completed {
                        tick: latest,
                        generation,
                    });
                } else {
                    let _ = out.send(ThreadMsg::Completed {
                        tick: n,
                        generation,
                    });
                }
            }
            IsolateCmd::ResetSession => {
                crate::observed::on_reset();
                super::reach_query::on_reset();
                crate::periodic_bank::on_reset();
                crate::cake_stall::on_reset();
                crate::walk_wait::on_reset();
                crate::inspect_wait::on_reset();
                crate::death_recovery::on_reset();
                crate::machine::on_reset();
                crate::shop::on_reset();
                crate::hunt_fight::on_reset();
                crate::hunt_lair::on_reset();
                crate::hunt_leave::on_reset();
                crate::hunt_key::on_reset();
                crate::hunt_cell::on_reset();
                crate::hunt_bank::on_reset();
                crate::production::on_reset();
                crate::dialog::on_reset();
                crate::quest_journal::on_reset();
                crate::clue::on_reset();
                crate::reach::on_reset();
                crate::trade::on_reset();
                crate::drive_partner_trade::on_reset();
                event_producer.reset();
                if events_consumed {
                    // The compat runner's queue is the only holder of
                    // events that were staged but not yet flushed. A
                    // reconnect must not replay them into the new
                    // session, and must not leave the trim window full.
                    let _ =
                        runtime.eval::<()>("globalThis.__rs2b0t_pending_native_event_batch = null");
                }
                if v2_native {
                    let _ = runtime.eval::<()>(
                        "if (typeof globalThis.__rs_v2_reset_session === 'function') globalThis.__rs_v2_reset_session()",
                    );
                    // Machine rows are gone: settle their awaits now, so a
                    // stale promise cannot take a later snapshot's facts.
                    if crate::machine::any_settled() {
                        let _ = runtime.call_function_immediate::<()>(
                            None,
                            "__rs2b0t_settle_machines",
                            json_args!(),
                        );
                    }
                    if let Err(e) = runtime.block_on_event_loop(
                        rustyscript::deno_core::PollEventLoopOptions::default(),
                        Some(Duration::from_millis(10)),
                    ) {
                        let _ = out.send(ThreadMsg::Log(format!("session reset: {e}")));
                    }
                }
                // Rows queued during the reset drain go, machine ops with
                // them.
                let _ = runtime.eval::<()>("globalThis.__rs2b0t_host.interact = []");
                crate::machine::drop_ops();
                clear_unconsumed_paint_click(&mut runtime);
                super::paint_chrome::reset();
                super::paint_jive::reset();
            }
            IsolateCmd::Pause => {
                paused = true;
                let _ = event_producer.set_paused(true);
                crate::periodic_bank::on_pause();
                crate::cake_stall::on_pause();
                crate::walk_wait::on_pause();
                crate::inspect_wait::on_pause();
                crate::death_recovery::on_pause();
                crate::machine::on_pause();
                crate::shop::on_pause();
                crate::hunt_fight::on_pause();
                crate::hunt_lair::on_pause();
                crate::hunt_leave::on_pause();
                crate::hunt_key::on_pause();
                crate::hunt_cell::on_pause();
                crate::hunt_bank::on_pause();
                crate::production::on_pause();
                crate::dialog::on_pause();
                crate::quest_journal::on_pause();
                crate::clue::on_pause();
                crate::reach::on_pause();
                crate::trade::on_pause();
                crate::drive_partner_trade::on_pause();
                clear_unconsumed_paint_click(&mut runtime);
            }
            IsolateCmd::Resume => {
                paused = false;
                let _ = event_producer.set_paused(false);
                crate::periodic_bank::on_resume();
                crate::cake_stall::on_resume();
                crate::walk_wait::on_resume();
                crate::inspect_wait::on_resume();
                crate::death_recovery::on_resume();
                crate::machine::on_resume();
                crate::shop::on_resume();
                crate::hunt_fight::on_resume();
                crate::hunt_lair::on_resume();
                crate::hunt_leave::on_resume();
                crate::hunt_key::on_resume();
                crate::hunt_cell::on_resume();
                crate::hunt_bank::on_resume();
                crate::production::on_resume();
                crate::dialog::on_resume();
                crate::quest_journal::on_resume();
                crate::clue::on_resume();
                crate::reach::on_resume();
                crate::trade::on_resume();
                crate::drive_partner_trade::on_resume();
            }
            IsolateCmd::PaintClick { id, generation } => {
                if paused
                    || generation != work_generation.load(std::sync::atomic::Ordering::Acquire)
                {
                    continue;
                }
                if let Err(e) = set_paint_click(&mut runtime, &id) {
                    let _ = out.send(ThreadMsg::Log(format!("paintClick: {e}")));
                }
            }
            IsolateCmd::PaintSelect {
                key,
                name,
                generation,
            } => {
                if paused
                    || generation != work_generation.load(std::sync::atomic::Ordering::Acquire)
                {
                    continue;
                }
                super::paint_chrome::store_select(&key, &name);
            }
            IsolateCmd::RecoveryAnchor { generation } => {
                // Pause / generation still reject. host_hold freezes
                // loop/pump (guardian or recovery) but must not skip
                // the async recoveryAnchor sample: OR-ing recovery into
                // snapshot.hold would otherwise stick SamplingAnchor.
                // Guardian freeze aborts sampling on the host before a
                // new request is posted.
                if paused
                    || generation != work_generation.load(std::sync::atomic::Ordering::Acquire)
                {
                    let _ = out.send(ThreadMsg::InFlightDone {
                        generation,
                        tick: RECOVERY_ANCHOR_TICK,
                    });
                    continue;
                }
                let start = Instant::now();
                let req = match eval_recovery_anchor(&mut runtime) {
                    Some((x, z, level)) => crate::shim::InteractReq::RecoveryAnchor { x, z, level },
                    None => crate::shim::InteractReq::RecoveryAnchorNone,
                };
                runtime
                    .deno_runtime()
                    .v8_isolate()
                    .cancel_terminate_execution();
                if start.elapsed() > SLOW_TICK {
                    let _ = out.send(ThreadMsg::Log(format!(
                        "slow recoveryAnchor: {:?}",
                        start.elapsed()
                    )));
                }
                if generation == work_generation.load(std::sync::atomic::Ordering::Acquire) {
                    let _ = out.send(ThreadMsg::Interact {
                        bytes: ipc.encode_interact_batch(&[req]),
                        generation,
                    });
                }
                let _ = out.send(ThreadMsg::InFlightDone {
                    generation,
                    tick: RECOVERY_ANCHOR_TICK,
                });
            }
            IsolateCmd::Probe(expr, reply) => {
                let value: Result<serde_json::Value, String> =
                    runtime.eval(expr).map_err(|e| e.to_string());
                let _ = reply.send(value);
            }
            IsolateCmd::Stop { invoke_hook } => {
                teardown_once(&mut runtime, &out, invoke_hook, &teardown, &proof);
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reset_rejects_a_tick_queued_with_the_previous_session_generation() {
        let iso = LoadIsolate::spawn(
            "export function tick(api) { globalThis.n = (globalThis.n || 0) + 1; }".into(),
            LoadShape::NativeTick,
            vec![],
        )
        .unwrap();
        iso.reset_session_work();
        // A sender captured this tick before reset but enqueued it late.
        iso.tx
            .send(IsolateCmd::Tick {
                tick: 1,
                generation: 0,
                input_identity: 0,
            })
            .unwrap();
        assert_eq!(
            iso.probe("globalThis.n || 0").unwrap(),
            serde_json::json!(0)
        );
        iso.on_game_tick(2);
        assert_eq!(iso.probe("globalThis.n").unwrap(), serde_json::json!(1));
        iso.join();
    }

    #[test]
    fn shim_prelude_defines_globals_for_compat_fixture() {
        ensure_platform();
        let mut runtime = Runtime::new(RuntimeOptions::default()).unwrap();
        runtime.eval::<()>(crate::shim::PRELUDE).unwrap();
        let t: bool = runtime.eval("typeof defineBot === 'function'").unwrap();
        assert!(t);
        let t: bool = runtime.eval("typeof TaskBot === 'function'").unwrap();
        assert!(t);
        let t: bool = runtime.eval("typeof TreeBot === 'function'").unwrap();
        assert!(t);
        let t: bool = runtime.eval("typeof LoopingBot === 'function'").unwrap();
        assert!(t);
        let t: bool = runtime.eval("typeof __rs2b0t_host === 'object'").unwrap();
        assert!(t);
        // defineBot validates { name, create } instead of no-op'ing.
        let err: bool = runtime
            .eval("(() => { try { defineBot({}); return false; } catch { return true; } })()")
            .unwrap();
        assert!(err, "defineBot throws without a name/create pair");
    }

    #[test]
    fn prelude_canvas_keyboard_queues_key_rows_and_blocks_mouse_layout() {
        ensure_platform();
        let mut runtime = Runtime::new(RuntimeOptions::default()).unwrap();
        runtime.eval::<()>(crate::shim::PRELUDE).unwrap();
        let report: serde_json::Value = runtime
            .eval(
                r#"
(() => {
const canvas = document.getElementById('canvas');
const other = document.getElementById('other');
canvas.dispatchEvent(new KeyboardEvent('keydown', {key: '2', code: '2'}));
canvas.dispatchEvent(new KeyboardEvent('keyup', {key: '2', code: '2'}));
let mouseCtor = true;
try { new MouseEvent('mousedown'); } catch { mouseCtor = false; }
let mouseDispatch = '';
try { canvas.dispatchEvent(new MouseEvent('mousedown')); }
catch (e) { mouseDispatch = String((e && e.message) || e); }
let layout = '';
try { canvas.getBoundingClientRect(); }
catch (e) { layout = String((e && e.message) || e); }
const frozen = new MouseEvent('mousedown', {clientX: 382.5, clientY: 251.5});
canvas.dispatchEvent(frozen);
let unknown = '';
try { canvas.dispatchEvent(new MouseEvent('mousemove')); }
catch (e) { unknown = String((e && e.message) || e); }
return {
    canvas: canvas !== null && typeof canvas === 'object',
    other: other,
    interact: globalThis.__rs2b0t_host.interact,
    mouseCtor,
    mouseDispatch,
    layout,
    frozen: {x: frozen.clientX, y: frozen.clientY, button: frozen.button},
    unknown,
};
})()
"#,
            )
            .unwrap();
        assert_eq!(report["canvas"], true);
        assert!(report["other"].is_null());
        assert_eq!(
            report["interact"],
            serde_json::json!([
                {"op": "key", "down": true, "key": "2", "code": "2"},
                {"op": "key", "down": false, "key": "2", "code": "2"},
                {"op": "mouse", "down": true, "x": 0, "y": 0, "button": 0},
                {"op": "mouse", "down": true, "x": 382.5, "y": 251.5, "button": 0},
            ])
        );
        assert_eq!(report["mouseCtor"], true);
        assert_eq!(report["mouseDispatch"], "");
        assert!(
            report["layout"]
                .as_str()
                .is_some_and(|s| s.contains("BLOCKED: missing getBoundingClientRect")),
            "{report:?}"
        );
        assert_eq!(report["frozen"]["x"], 382.5);
        assert_eq!(report["frozen"]["y"], 251.5);
        assert_eq!(report["frozen"]["button"], 0);
        assert!(
            report["unknown"]
                .as_str()
                .is_some_and(|s| s.contains("BLOCKED: missing mouse")),
            "{report:?}"
        );
    }

    #[test]
    fn canvas_keyboard_producer_round_trips_fb_and_drops_stale_generation() {
        let iso = LoadIsolate::spawn(
            r#"
export default class T extends LoopingBot {
loop() {
    const canvas = document.getElementById('canvas');
    canvas.dispatchEvent(new KeyboardEvent('keydown', {key: '2', code: '2'}));
    canvas.dispatchEvent(new KeyboardEvent('keyup', {key: '2', code: '2'}));
}
}
"#
            .into(),
            LoadShape::CompatClass,
            vec![],
        )
        .unwrap();
        iso.on_game_tick(1);
        iso.probe("true").unwrap();
        let reqs = iso.drain_interacts();
        assert!(
            reqs.iter().any(|req| matches!(
                req,
                crate::shim::InteractReq::Key {
                    down: true,
                    key,
                    ..
                } if key == "2"
            )),
            "{reqs:?}"
        );
        assert!(
            reqs.iter().any(|req| matches!(
                req,
                crate::shim::InteractReq::Key {
                    down: false,
                    key,
                    ..
                } if key == "2"
            )),
            "{reqs:?}"
        );

        iso.on_game_tick(2);
        iso.probe("true").unwrap();
        iso.reset_session_work();
        let stale = iso.drain_interacts();
        assert!(
            stale
                .iter()
                .all(|req| !matches!(req, crate::shim::InteractReq::Key { .. })),
            "stale generation must not deliver keys: {stale:?}"
        );
        iso.join();
    }

    #[test]
    fn canvas_mouse_producer_round_trips_fb_and_drops_stale_generation() {
        let iso = LoadIsolate::spawn(
            r#"
export default class T extends LoopingBot {
loop() {
    const canvas = document.getElementById('canvas');
    canvas.dispatchEvent(new MouseEvent('mousedown', {clientX: 382.5, clientY: 251.5}));
    canvas.dispatchEvent(new MouseEvent('mouseup', {clientX: 382.5, clientY: 251.5}));
    canvas.dispatchEvent(new KeyboardEvent('keydown', {key: '2', code: '2'}));
}
}
"#
            .into(),
            LoadShape::CompatClass,
            vec![],
        )
        .unwrap();
        iso.on_game_tick(1);
        iso.probe("true").unwrap();
        let reqs = iso.drain_interacts();
        assert!(
            reqs.iter().any(|req| matches!(
                req,
                crate::shim::InteractReq::Mouse {
                    down: true,
                    x,
                    y,
                    button: 0,
                    ..
                } if (*x - 382.5).abs() < 1e-9 && (*y - 251.5).abs() < 1e-9
            )),
            "{reqs:?}"
        );
        assert!(
            reqs.iter()
                .any(|req| matches!(req, crate::shim::InteractReq::Mouse { down: false, .. })),
            "{reqs:?}"
        );
        assert!(
            reqs.iter().any(|req| matches!(
                req,
                crate::shim::InteractReq::Key { down: true, key, .. } if key == "2"
            )),
            "{reqs:?}"
        );

        iso.on_game_tick(2);
        iso.probe("true").unwrap();
        iso.reset_session_work();
        let stale = iso.drain_interacts();
        assert!(
            stale
                .iter()
                .all(|req| !matches!(req, crate::shim::InteractReq::Mouse { .. })),
            "stale generation must not deliver mouse: {stale:?}"
        );
        iso.join();
    }

    #[test]
    fn canvas_mouse_production_stamps_input_identity() {
        let iso = LoadIsolate::spawn(
            r#"
export default class T extends LoopingBot {
loop() {
    const canvas = document.getElementById('canvas');
    canvas.dispatchEvent(new MouseEvent('mousedown', {clientX: 382.5, clientY: 251.5}));
}
}
"#
            .into(),
            LoadShape::CompatClass,
            vec![],
        )
        .unwrap();
        iso.on_game_tick_at(1, 42);
        iso.probe("true").unwrap();
        let reqs = iso.drain_interacts();
        assert!(
            reqs.iter().any(|req| matches!(
                req,
                crate::shim::InteractReq::Mouse {
                    down: true,
                    identity: 42,
                    ..
                }
            )),
            "{reqs:?}"
        );
        iso.join();
    }

    #[test]
    fn root_mouse_parked_up_keeps_revoked_gesture_identity() {
        let iso = LoadIsolate::spawn(
            r#"
export default class T extends LoopingBot {
async loop() {
    if (globalThis.__started) return;
    globalThis.__started = true;
    const canvas = document.getElementById('canvas');
    canvas.dispatchEvent(new MouseEvent('mousedown', {clientX: 100, clientY: 100}));
    await new Promise((resolve) => {
        globalThis.__release = resolve;
    });
    canvas.dispatchEvent(new MouseEvent('mouseup', {clientX: 100, clientY: 100}));
}
}
"#
            .into(),
            LoadShape::CompatClass,
            vec![],
        )
        .unwrap();

        iso.on_game_tick_at(1, 42);
        iso.probe("true").unwrap();
        let first = iso.drain_interacts();
        assert!(
            first.iter().any(|req| matches!(
                req,
                crate::shim::InteractReq::Mouse {
                    down: true,
                    identity: 42,
                    ..
                }
            )),
            "{first:?}"
        );

        iso.pause();
        iso.resume();
        iso.probe("globalThis.__release(); true").unwrap();
        iso.on_game_tick_at(2, 43);
        iso.probe("true").unwrap();
        let second = iso.drain_interacts();
        assert!(
            second.iter().any(|req| matches!(
                req,
                crate::shim::InteractReq::Mouse {
                    down: false,
                    identity: 42,
                    ..
                }
            )),
            "parked old up was restamped: {second:?}"
        );
        iso.join();
    }

    #[test]
    fn execution_delay_ticks_mouse_up_keeps_down_identity() {
        let iso = LoadIsolate::spawn(
            r#"
import { Execution } from '../../api/execution/Execution.js';
export default class T extends LoopingBot {
async loop() {
    if (globalThis.__started) return;
    globalThis.__started = true;
    const canvas = document.getElementById('canvas');
    canvas.dispatchEvent(new MouseEvent('mousedown', {clientX: 100, clientY: 100}));
    await Execution.delayTicks(1);
    canvas.dispatchEvent(new MouseEvent('mouseup', {clientX: 100, clientY: 100}));
}
}
"#
            .into(),
            LoadShape::CompatClass,
            vec![],
        )
        .unwrap();

        iso.on_game_tick_at(1, 42);
        iso.probe("true").unwrap();
        let first = iso.drain_interacts();
        assert!(first.iter().any(|req| matches!(
            req,
            crate::shim::InteractReq::Mouse {
                down: true,
                identity: 42,
                ..
            }
        )));

        iso.pause();
        iso.resume();
        iso.on_game_tick_at(2, 43);
        iso.probe("true").unwrap();
        let second = iso.drain_interacts();
        assert!(
            second.iter().any(|req| matches!(
                req,
                crate::shim::InteractReq::Mouse {
                    down: false,
                    identity: 42,
                    ..
                }
            )),
            "{second:?}"
        );
        iso.join();
    }

    #[test]
    fn mouse_gesture_identity_overflow_fails_closed() {
        let mouse = |down| crate::shim::InteractReq::Mouse {
            down,
            x: 10.0,
            y: 10.0,
            button: 0,
            identity: 99,
        };
        let mut gestures = MouseGestureIdentities::default();
        let mut downs: Vec<_> = (0..=MAX_MOUSE_GESTURES).map(|_| mouse(true)).collect();
        stamp_mouse_gesture_identities(&mut downs, 7, &mut gestures);
        assert!(downs
            .iter()
            .all(|req| matches!(req, crate::shim::InteractReq::Mouse { identity: 7, .. })));
        assert!(gestures.pairs.is_empty());
        assert_eq!(gestures.unpairable, 33);

        let mut ups: Vec<_> = (0..=MAX_MOUSE_GESTURES).map(|_| mouse(false)).collect();
        stamp_mouse_gesture_identities(&mut ups, 7, &mut gestures);
        assert!(ups
            .iter()
            .all(|req| matches!(req, crate::shim::InteractReq::Mouse { identity: 0, .. })));
        assert_eq!(gestures.unpairable, 0);
    }

    #[test]
    fn canvas_mouse_malformed_rows_keep_sibling_key_and_center() {
        let iso = LoadIsolate::spawn(
            r#"
export default class T extends LoopingBot {
loop() {
    const canvas = document.getElementById('canvas');
    const h = globalThis.__rs2b0t_host;
    h.interact = h.interact || [];
    h.interact.push({op:'mouse', down:true, x:{}, y:10, button:0});
    h.interact.push({op:'mouse', down:true, x:[1], y:10, button:0});
    h.interact.push({op:'mouse', down:true, x: Number.NaN, y:10, button:0});
    h.interact.push({op:'mouse', down:true, x:100, y:100, button: 4294967296});
    h.interact.push({op:'mouse', down:true, x:100, y:100, button: 1.5});
    h.interact.push({op:'mouse', down:true, x:100, y:100, button: null});
    h.interact.push([1, 2, 3]);
    canvas.dispatchEvent(new KeyboardEvent('keydown', {key: '2', code: '2'}));
    canvas.dispatchEvent(new MouseEvent('mousedown', {clientX: 382.5, clientY: 251.5}));
    canvas.dispatchEvent(new MouseEvent('mouseup', {clientX: 382.5, clientY: 251.5}));
}
}
"#
            .into(),
            LoadShape::CompatClass,
            vec![],
        )
        .unwrap();
        iso.on_game_tick(1);
        iso.probe("true").unwrap();
        let reqs = iso.drain_interacts();
        assert!(
            reqs.iter().any(|req| matches!(
                req,
                crate::shim::InteractReq::Key { down: true, key, .. } if key == "2"
            )),
            "sibling key dropped: {reqs:?}"
        );
        assert!(
            reqs.iter().any(|req| matches!(
                req,
                crate::shim::InteractReq::Mouse {
                    down: true,
                    x,
                    y,
                    button: 0,
                    ..
                } if (*x - 382.5).abs() < 1e-9 && (*y - 251.5).abs() < 1e-9
            )),
            "valid center down dropped: {reqs:?}"
        );
        assert!(
            reqs.iter()
                .any(|req| matches!(req, crate::shim::InteractReq::Mouse { down: false, .. })),
            "valid center up dropped: {reqs:?}"
        );
        iso.join();
    }

    /// One posted tick whose only event-relevant fact is a prayer xp table:
    /// an `xp` above the previous tick's value is one `SkillXp` event.
    fn xp_input(tick: u64, inv_size: i32, xp: i32) -> Vec<u8> {
        let stats = [crate::isolate_fb::StatInput {
            index: 5,
            name: "prayer",
            xp,
            base: 2,
            effective: 2,
        }];
        let mut input = crate::isolate_fb::tests::empty_input(tick);
        input.inv_size = inv_size;
        input.stats = &stats;
        crate::isolate_fb::encode_snapshot(&input)
    }

    fn xp_event(skill: i32) -> crate::events::NativeEvent {
        crate::events::NativeEvent::SkillXp {
            skill,
            name: format!("skill{skill}"),
            xp: skill,
            delta: 1,
        }
    }

    fn staged_writes(iso: &LoadIsolate) -> serde_json::Value {
        iso.probe(
            "({ticks: globalThis.__ticks || 0, invSize: globalThis.__rs2b0t_host.snapshot.inv_size, \
             staged: typeof globalThis.__staged_writes, \
             pending: typeof globalThis.__rs2b0t_pending_native_event_batch})",
        )
        .unwrap()
    }

    /// Undelivered events in the compat runner's queue (0 when it is unset).
    fn queue_length(iso: &LoadIsolate) -> Option<i64> {
        iso.probe(
            "(() => { const q = globalThis.__rs2b0t_pending_native_event_batch; return q ? q.length : 0; })()",
        )
        .unwrap()
        .as_i64()
    }

    /// A shape without an events API must not pay for events: the producer
    /// never diffs a posted table (no diagnostic either), and the dispatcher
    /// never builds or stages a batch. `__staged_writes` counts every write
    /// the dispatcher would make to its staging global — the hand-off, plus
    /// its own clear.
    #[test]
    fn native_tick_shape_builds_and_stages_no_native_events() {
        let iso = LoadIsolate::spawn(
            r#"
let staged = null;
Object.defineProperty(globalThis, '__rs2b0t_native_event_batch', {
    configurable: true,
    get() { return staged; },
    set(v) {
        globalThis.__staged_writes = (globalThis.__staged_writes || 0) + 1;
        staged = v;
    },
});
export function tick(api) { globalThis.__ticks = (globalThis.__ticks || 0) + 1; }
"#
            .into(),
            LoadShape::NativeTick,
            vec![],
        )
        .unwrap();
        // An invalid inv_size is a producer diagnostic for a consumer; this
        // shape has none, so not even the diff may run.
        iso.post_snapshot(xp_input(1, 28, 0));
        iso.on_game_tick(1);
        iso.post_snapshot(xp_input(2, 99, 1));
        iso.on_game_tick(2);
        let report = staged_writes(&iso);
        assert_eq!(report["ticks"], 2, "both ticks reached the module");
        assert_eq!(report["invSize"], 99, "the snapshot materialised");
        assert_eq!(
            report["staged"], "undefined",
            "no event batch may be built for a shape without an events API"
        );
        assert_eq!(
            report["pending"], "undefined",
            "no event queue may be created for a shape without an events API"
        );
        let logs = iso.drain_logs();
        assert!(
            !logs.iter().any(|line| line.contains("inventory events")),
            "an unconsumed shape must not diff posted tables: {logs:?}"
        );
        iso.join();
    }

    /// The compat queue is the only staging buffer between the dispatcher and
    /// the runner's per-tick drain. It must stay bounded when that drain
    /// stalls, and the bound must drop the oldest events — the drain delivers
    /// in order, so the newest are the ones still worth running.
    #[test]
    fn compat_event_queue_is_capped_and_keeps_the_newest_events() {
        ensure_platform();
        let mut runtime = Runtime::new(RuntimeOptions::default()).unwrap();
        let first: Vec<_> = (0..300).map(xp_event).collect();
        dispatch_native_events(&mut runtime, &first).unwrap();
        let staged: Option<serde_json::Value> = runtime
            .eval("globalThis.__rs2b0t_native_event_batch ?? null")
            .unwrap();
        assert!(staged.is_none(), "the append consumes the staged batch");
        let capped: i64 = runtime
            .eval("globalThis.__rs2b0t_pending_native_event_batch.length")
            .unwrap();
        assert_eq!(capped, 256, "one oversized batch is capped at 256");

        let second: Vec<_> = (300..310).map(xp_event).collect();
        dispatch_native_events(&mut runtime, &second).unwrap();
        let report: Vec<i64> = runtime
            .eval("(() => { const q = globalThis.__rs2b0t_pending_native_event_batch; return [q.length, q[0].payload.skill, q[q.length - 1].payload.skill]; })()")
            .unwrap();
        assert_eq!(report[0], 256, "a second batch stays capped: {report:?}");
        assert_eq!(
            report[1], 54,
            "the oldest events are trimmed first: {report:?}"
        );
        assert_eq!(report[2], 309, "the newest event is retained: {report:?}");
    }

    /// ResetSession drops the batches the previous connection staged: the
    /// compat runner must not replay them, and the next session must not
    /// start with the trim window already full.
    #[test]
    fn reset_session_clears_the_pending_native_event_queue() {
        let iso = LoadIsolate::spawn(
            r#"
export default class T extends LoopingBot {
loop() {
    // Stall the runner's drain so the queue keeps what Rust staged.
    globalThis.__rs2b0t_flush_native_events = () => {};
}
}
"#
            .into(),
            LoadShape::CompatClass,
            vec![],
        )
        .unwrap();
        iso.post_snapshot(xp_input(1, 28, 0));
        iso.on_game_tick(1);
        assert_eq!(
            queue_length(&iso),
            Some(0),
            "the first table only seeds the producer"
        );
        iso.post_snapshot(xp_input(2, 28, 1));
        iso.on_game_tick(2);
        assert_eq!(
            queue_length(&iso),
            Some(1),
            "the xp diff reached the compat queue"
        );
        iso.reset_session_work();
        let cleared = iso
            .probe("globalThis.__rs2b0t_pending_native_event_batch === null")
            .unwrap();
        assert_eq!(
            cleared,
            serde_json::json!(true),
            "ResetSession must clear the undelivered queue"
        );
        iso.join();
    }

    fn wait_ready(iso: &LoadIsolate) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match iso.poll_ready() {
                Ready::Ready => return,
                Ready::Failed(e) => panic!("setup failed: {e}"),
                Ready::Pending if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Ready::Pending => panic!("setup timed out"),
            }
        }
    }

    #[test]
    fn spawn_returns_before_setup_and_poll_ready_becomes_ready() {
        let t0 = Instant::now();
        let iso = LoadIsolate::spawn(
            "export function tick(api) {}".into(),
            LoadShape::NativeTick,
            vec![],
        )
        .unwrap();
        assert!(
            t0.elapsed() < Duration::from_millis(500),
            "spawn must not wait on V8: {:?}",
            t0.elapsed()
        );
        wait_ready(&iso);
        assert_eq!(iso.poll_ready(), Ready::Ready);
        iso.join();
    }

    #[test]
    fn poll_ready_surfaces_a_wire_failure() {
        let iso = LoadIsolate::spawn(
            "not valid javascript!!!!".into(),
            LoadShape::NativeTick,
            vec![],
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let err = loop {
            match iso.poll_ready() {
                Ready::Failed(e) => break e,
                Ready::Ready => panic!("invalid source must not become Ready"),
                Ready::Pending if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Ready::Pending => panic!("setup neither failed nor finished"),
            }
        };
        assert!(!err.is_empty(), "{err}");
        iso.join();
    }

    #[test]
    fn join_detached_returns_immediately_and_delivers_onstop_logs() {
        let iso = LoadIsolate::spawn(
            "export default class T extends LoopingBot {
            loop() {}
            onStop() { this.log('stopped-ok'); }
        }"
            .into(),
            LoadShape::CompatClass,
            vec![],
        )
        .unwrap();
        wait_ready(&iso);
        iso.on_game_tick(1);
        let _ = iso.probe("1");
        let (tx, rx) = mpsc::channel();
        let t0 = Instant::now();
        iso.join_detached(tx);
        assert!(
            t0.elapsed() < Duration::from_millis(200),
            "join_detached blocked: {:?}",
            t0.elapsed()
        );
        let logs = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("reaper logs");
        assert!(
            logs.iter().any(|l| l.contains("stopped-ok")),
            "onStop log missing: {logs:?}"
        );
        let _ = abandoned_isolate_count();
    }

    fn machine_tick(iso: &LoadIsolate, n: u64) {
        iso.on_game_tick(n);
        let _ = iso.probe("true");
    }

    fn if_button(id: i32) -> crate::shim::InteractReq {
        crate::shim::InteractReq::IfButton { component_id: id }
    }

    fn spawn_machine_card(body: &str) -> LoadIsolate {
        let src = format!(
            "import {{ runMachine, queue }} from '../../shim/_kernel.js';\n\
             import {{ Execution }} from '../../api/execution/Execution.js';\n\
             export default class T extends LoopingBot {{\n\
             async loop() {{\n\
             if (globalThis.__did) return;\n\
             globalThis.__did = true;\n\
             {body}\n\
             }}\n}}\n"
        );
        LoadIsolate::spawn(src, LoadShape::CompatClass, vec![]).unwrap()
    }

    #[test]
    fn machine_ops_join_the_batch_in_order_and_the_await_settles_once() {
        let iso = spawn_machine_card(
            "globalThis.__settles = 0;
             queue({ op: 'if-button', component_id: 1 });
             const run = runMachine('probe', { button: 10, steps: 2 });
             queue({ op: 'if-button', component_id: 2 });
             const out = await run;
             globalThis.__settles += 1;
             globalThis.__out = out;
             globalThis.__at = globalThis.__rs2b0t_host.tick;",
        );
        machine_tick(&iso, 1);
        assert_eq!(
            iso.drain_interacts(),
            vec![if_button(1), if_button(10), if_button(2)],
            "begin ops sit where the caller started the machine"
        );
        machine_tick(&iso, 2);
        assert_eq!(iso.drain_interacts(), vec![if_button(11)]);
        machine_tick(&iso, 3);
        assert_eq!(iso.drain_interacts(), vec![if_button(12)]);
        assert_eq!(iso.probe("globalThis.__settles").unwrap(), 0);
        for n in 4..=6 {
            machine_tick(&iso, n);
        }
        assert_eq!(iso.probe("globalThis.__settles").unwrap(), 1);
        assert_eq!(
            iso.probe("globalThis.__out").unwrap(),
            serde_json::json!({ "kind": "done", "value": 2 })
        );
        assert_eq!(
            iso.probe("globalThis.__at").unwrap(),
            4,
            "the completing step settles in the same tick's pump"
        );
        assert!(
            iso.drain_interacts().is_empty(),
            "a finished row emits nothing"
        );
        iso.join();
    }

    #[test]
    fn reset_session_aborts_a_running_machine_and_settles_its_await() {
        let iso = spawn_machine_card(
            "globalThis.__out = await runMachine('probe', { button: 10, steps: 5 });",
        );
        machine_tick(&iso, 1);
        assert_eq!(iso.drain_interacts(), vec![if_button(10)]);
        iso.reset_session_work();
        machine_tick(&iso, 2);
        assert_eq!(
            iso.probe("globalThis.__out").unwrap(),
            serde_json::json!({ "kind": "aborted", "reason": "reset" })
        );
        machine_tick(&iso, 3);
        assert!(
            iso.drain_interacts().is_empty(),
            "an aborted row never steps"
        );
        iso.join();
    }

    #[test]
    fn concurrent_machines_settle_independently() {
        let iso = spawn_machine_card(
            "const stamp = (p) => p.then((out) => ({ out, at: globalThis.__rs2b0t_host.tick }));
             globalThis.__both = await Promise.all([
                 stamp(runMachine('probe', { button: 100, steps: 1 })),
                 stamp(runMachine('probe', { button: 200, steps: 3 })),
             ]);",
        );
        for n in 1..=6 {
            machine_tick(&iso, n);
        }
        assert_eq!(
            iso.probe("globalThis.__both").unwrap(),
            serde_json::json!([
                { "out": { "kind": "done", "value": 1 }, "at": 3 },
                { "out": { "kind": "done", "value": 3 }, "at": 5 },
            ])
        );
        iso.join();
    }

    #[test]
    fn refused_and_immediate_starts_resolve_without_a_wait() {
        let iso = spawn_machine_card(
            "globalThis.__outs = [
                 await runMachine('probe', { button: 1, steps: 1, refuse: true }),
                 await runMachine('probe', { button: 2, steps: 0 }),
                 await runMachine('nowhere', {}),
             ];",
        );
        machine_tick(&iso, 1);
        let outs = iso.probe("globalThis.__outs").unwrap();
        assert_eq!(
            outs[0],
            serde_json::json!({ "kind": "refused", "reason": "probe refused" })
        );
        assert_eq!(outs[1], serde_json::json!({ "kind": "done", "value": 0 }));
        assert_eq!(outs[2]["kind"], "refused");
        assert_eq!(iso.drain_interacts(), vec![if_button(2)]);
        iso.join();
    }

    // Frozen timing: a callback sees the tick it runs in, `delayTicks(1)`
    // from a tick-2 callback resumes in tick 3's pump, and the row resumes
    // (and here ends, settling the await) in tick 3 as well.
    #[test]
    fn a_machine_calls_sync_and_async_script_callbacks_in_order() {
        let iso = spawn_machine_card(
            "globalThis.__calls = [];
             const at = () => globalThis.__rs2b0t_host.tick;
             const hooks = {
                 tag: 'H',
                 sync(n, s) {
                     globalThis.__calls.push(['sync', n, s, this.tag, at()]);
                     queue({ op: 'if-button', component_id: 900 });
                     return n + 1;
                 },
                 async later(v) {
                     globalThis.__calls.push(['later', v, at()]);
                     await Execution.delayTicks(1);
                     globalThis.__calls.push(['later-resumed', v, at()]);
                     return v * 10;
                 },
                 mark() {
                     globalThis.__calls.push(['mark', at()]);
                     return 'm';
                 },
             };
             globalThis.__out = await runMachine('hooked', { emit: 500 }, hooks);
             globalThis.__at = at();",
        );
        machine_tick(&iso, 1);
        assert!(iso.drain_interacts().is_empty());
        machine_tick(&iso, 2);
        assert_eq!(
            iso.drain_interacts(),
            vec![if_button(900), if_button(501)],
            "a step's op follows the row its callback queued"
        );
        assert_eq!(
            iso.probe("globalThis.__calls").unwrap(),
            serde_json::json!([["sync", 1, "a", "H", 2], ["later", 2, 2]]),
            "callbacks see the tick they run in; delayTicks(1) has not elapsed"
        );
        assert_eq!(
            iso.probe("globalThis.__out ?? null").unwrap(),
            serde_json::Value::Null
        );
        machine_tick(&iso, 3);
        assert_eq!(
            iso.probe("globalThis.__calls").unwrap(),
            serde_json::json!([
                ["sync", 1, "a", "H", 2],
                ["later", 2, 2],
                ["later-resumed", 2, 3],
                ["mark", 3],
            ]),
            "the row resumes in the tick the wait settled"
        );
        assert_eq!(iso.drain_interacts(), vec![if_button(502), if_button(503)]);
        assert_eq!(
            iso.probe("globalThis.__out").unwrap(),
            serde_json::json!({ "kind": "done", "value": [2, 20, "m"] })
        );
        assert_eq!(iso.probe("globalThis.__at").unwrap(), 3);
        iso.join();
    }

    // N1(a): a callback that only awaits microtasks answers in the call's
    // own checkpoint, so all of a burst's awaited calls run in one tick
    // (up to the per-row budget), like a frozen `for` with `await`s.
    #[test]
    fn microtask_only_awaits_answer_in_the_same_step() {
        let iso = spawn_machine_card(
            "globalThis.__ticks = [];
             globalThis.__out = await runMachine('burst', { calls: 40 }, {
                 async each(i) {
                     await null;
                     await Promise.resolve();
                     globalThis.__ticks.push(globalThis.__rs2b0t_host.tick);
                     return i;
                 },
             });
             globalThis.__at = globalThis.__rs2b0t_host.tick;",
        );
        machine_tick(&iso, 1);
        machine_tick(&iso, 2);
        let ticks: Vec<u64> =
            serde_json::from_value(iso.probe("globalThis.__ticks").unwrap()).unwrap();
        assert_eq!(ticks.len(), 32, "the budget bounds tick 2: {ticks:?}");
        assert!(ticks.iter().all(|&t| t == 2), "{ticks:?}");
        machine_tick(&iso, 3);
        let ticks: Vec<u64> =
            serde_json::from_value(iso.probe("globalThis.__ticks").unwrap()).unwrap();
        assert_eq!(ticks.len(), 40);
        assert!(ticks[32..].iter().all(|&t| t == 3), "{ticks:?}");
        assert_eq!(iso.probe("globalThis.__out.value.length").unwrap(), 40);
        assert_eq!(iso.probe("globalThis.__at").unwrap(), 3);
        iso.join();
    }

    // N1(b): a callback promise the pump settles resumes its row after the
    // pump in the same tick, and the budget spans both passes: 6 calls
    // before the wait, 26 after it in tick 2, the last 8 in tick 3.
    #[test]
    fn a_pump_settled_callback_resumes_its_row_in_the_same_tick() {
        let iso = spawn_machine_card(
            "globalThis.__ticks = [];
             const each = (i) => {
                 globalThis.__ticks.push(globalThis.__rs2b0t_host.tick);
                 return i === 5 ? Execution.delayTicks(0).then(() => i) : i;
             };
             globalThis.__out = await runMachine('burst', { calls: 40 }, { each });",
        );
        machine_tick(&iso, 1);
        machine_tick(&iso, 2);
        let ticks: Vec<u64> =
            serde_json::from_value(iso.probe("globalThis.__ticks").unwrap()).unwrap();
        assert_eq!(ticks, vec![2; 32], "6 calls, the pump, then 26 more");
        machine_tick(&iso, 3);
        let ticks: Vec<u64> =
            serde_json::from_value(iso.probe("globalThis.__ticks").unwrap()).unwrap();
        assert_eq!(ticks[32..], [3; 8]);
        assert_eq!(
            iso.probe("globalThis.__out.value[5]").unwrap(),
            5,
            "the resumed row read the settled value"
        );
        iso.join();
    }

    // N2: presence is read once, at start; null/undefined are absent.
    #[test]
    fn hook_presence_is_read_once_at_start() {
        let iso = spawn_machine_card(
            "globalThis.__reads = 0;
             const hooks = {
                 a() {},
                 b: null,
                 get c() { globalThis.__reads += 1; return () => 1; },
             };
             globalThis.__out = await runMachine('present', {}, hooks);",
        );
        machine_tick(&iso, 1);
        assert_eq!(
            iso.probe("globalThis.__out").unwrap(),
            serde_json::json!({ "kind": "done", "value": [true, false, true, false] })
        );
        assert_eq!(iso.probe("globalThis.__reads").unwrap(), 1);
        iso.join();
    }

    // N3: rows script code queued between ticks (a probe here, the
    // recovery anchor in production) stay ahead of the next step's ops.
    #[test]
    fn step_ops_follow_rows_queued_before_the_tick() {
        let iso = spawn_machine_card(
            "globalThis.__out = await runMachine('probe', { button: 10, steps: 1 });",
        );
        machine_tick(&iso, 1);
        assert_eq!(iso.drain_interacts(), vec![if_button(10)]);
        let _ = iso
            .probe("globalThis.__rs2b0t_host.interact.push({ op: 'if-button', component_id: 7 })");
        machine_tick(&iso, 2);
        assert_eq!(iso.drain_interacts(), vec![if_button(7), if_button(11)]);
        iso.join();
    }

    // N5: a row whose callback starts a newer row of its exclusive family
    // stops driving at once: no further op, and it settles `superseded`.
    #[test]
    fn a_row_that_supersedes_itself_mid_step_stops_at_once() {
        let iso = spawn_machine_card(
            "globalThis.__out = await runMachine('solo-hooked', {}, {
                 again() {
                     globalThis.__next = runMachine('solo-hooked', { quiet: true }, {
                         again: () => 0,
                     });
                     return 1;
                 },
             });",
        );
        machine_tick(&iso, 1);
        machine_tick(&iso, 2);
        assert!(
            iso.drain_interacts().is_empty(),
            "the superseded row emits nothing after its callback"
        );
        machine_tick(&iso, 3);
        assert_eq!(
            iso.probe("globalThis.__out").unwrap(),
            serde_json::json!({ "kind": "aborted", "reason": "superseded" })
        );
        iso.join();
    }

    #[test]
    fn a_throwing_or_rejecting_callback_fails_the_machine() {
        let iso = spawn_machine_card(
            "const boom = new Error('boom');
             const late = { code: 7 };
             const settle = (p) => p.then(
                 (out) => ({ ok: out }),
                 (e) => ({ err: String(e && e.message), same: e === boom || e === late }),
             );
             globalThis.__outs = await Promise.all([
                 settle(runMachine('hooked', {}, { sync() { throw boom; } })),
                 settle(runMachine('hooked', {}, {
                     sync: (n) => n,
                     async later() { throw late; },
                 })),
                 settle(runMachine('hooked', {}, { sync: 5 })),
                 settle(runMachine('hooked', {})),
             ]);",
        );
        machine_tick(&iso, 1);
        machine_tick(&iso, 2);
        let outs = iso.probe("globalThis.__outs").unwrap();
        assert_eq!(
            outs[0],
            serde_json::json!({ "err": "boom", "same": true }),
            "the await rejects with the very value the callback threw"
        );
        assert_eq!(
            outs[1],
            serde_json::json!({ "err": "undefined", "same": true }),
            "and with the very value an async callback rejected with"
        );
        assert_eq!(
            outs[2],
            serde_json::json!({ "err": "sync is not a function", "same": false })
        );
        assert!(
            outs[3]["err"]
                .as_str()
                .is_some_and(|e| e.contains("reading 'sync'")),
            "a missing hooks object throws at start: {outs:?}"
        );
        let logs = iso.drain_logs();
        assert!(
            logs.iter().all(|l| !l.contains("Uncaught")),
            "an observed rejection is not unhandled: {logs:?}"
        );
        iso.join();
    }

    #[test]
    fn reset_and_stop_release_a_machine_waiting_on_a_callback_promise() {
        let src = "globalThis.__out = await runMachine('hooked', {}, {
                 sync: (n) => n,
                 later: () => new Promise(() => {}),
             });";
        let iso = spawn_machine_card(src);
        machine_tick(&iso, 1);
        machine_tick(&iso, 2);
        iso.reset_session_work();
        machine_tick(&iso, 3);
        assert_eq!(
            iso.probe("globalThis.__out").unwrap(),
            serde_json::json!({ "kind": "aborted", "reason": "reset" })
        );
        iso.join();

        // Stop with the callback and its promise still held: the rows drop
        // before the isolate (a V8 handle outliving it would abort here).
        let iso = spawn_machine_card(src);
        machine_tick(&iso, 1);
        machine_tick(&iso, 2);
        iso.join();
    }

    #[test]
    fn join_stops_a_machine_before_a_later_spinning_callback() {
        // The first callback spins until join's terminate ends it; the
        // machine keeps going on a throw, so without the claim re-check its
        // next callback would spin on past join.
        let iso = spawn_machine_card(
            "globalThis.__out = await runMachine('burst', { calls: 3, keep: true }, {
                 each() { for (;;) {} },
             });",
        );
        let proof = iso.teardown_proof();
        machine_tick(&iso, 1);
        iso.on_game_tick(2);
        std::thread::sleep(Duration::from_millis(200));
        let t0 = Instant::now();
        iso.join();
        assert!(
            proof.finished() && t0.elapsed() < JOIN_TIMEOUT,
            "join abandoned the isolate after {:?}",
            t0.elapsed()
        );
    }

    #[test]
    fn a_callback_may_start_another_machine_mid_step() {
        let iso = spawn_machine_card(
            "globalThis.__out = await runMachine('hooked', { emit: 500 }, {
                 sync(n) {
                     globalThis.__inner = runMachine('probe', { button: 700, steps: 1 });
                     return n;
                 },
                 later: (v) => v,
                 mark: () => null,
             });
             globalThis.__innerOut = await globalThis.__inner;",
        );
        machine_tick(&iso, 1);
        machine_tick(&iso, 2);
        assert_eq!(
            iso.drain_interacts(),
            vec![
                if_button(700),
                if_button(501),
                if_button(502),
                if_button(503)
            ],
            "the inner begin op lands where its callback ran"
        );
        for n in 3..=5 {
            machine_tick(&iso, n);
        }
        assert_eq!(
            iso.drain_interacts(),
            vec![if_button(701)],
            "the inner row is first stepped on the next tick"
        );
        assert_eq!(iso.probe("globalThis.__out.kind").unwrap(), "done");
        assert_eq!(
            iso.probe("globalThis.__innerOut").unwrap(),
            serde_json::json!({ "kind": "done", "value": 1 })
        );
        iso.join();
    }
}
