//! LoadIsolate: rustyscript V8 on its own thread (feature `load` only).

use super::bindings::wire_runtime;
use super::shape::LoadShape;
use super::snapshot::{
    dispatch_native_events, materialize_settings_bag, materialize_snapshot,
};
use rustyscript::{json_args, Runtime, RuntimeOptions};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Mutex, Once};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

static NEXT_PAINT_GENERATION: AtomicU64 = AtomicU64::new(1);

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

enum ThreadMsg {
    Log(String),
    /// The tick's shim interact queue (`__rs2b0t_host.interact`), a
    /// FlatBuffer `InteractBatch` of [`crate::shim::InteractReq`]s
    /// forwarded after the tick's JS finished (parked or not).
    Interact {
        bytes: Vec<u8>,
        generation: u64,
    },
    /// The tick's recorded paint frame (`Paint.begin` … `end()` on the
    /// host handle), a FlatBuffer `Paint` forwarded for the script
    /// paint views. The host reads the latest frame off the handle
    /// without a probe round-trip. Null frames are not forwarded — a
    /// script that stops painting keeps its last frame. Never a JSON
    /// value on this channel.
    Paint(Vec<u8>),
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
    /// match a later isolate that advertises the same button id.
    paint_generation: std::sync::atomic::AtomicU64,
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
    /// The latest paint frame the tick thread forwarded (a
    /// [`crate::shim::ScriptPaint`] decoded off the host handle after
    /// each tick), read by the script paint views.
    paint: Mutex<Option<crate::shim::ScriptPaint>>,
    /// The bot instance's random-ignore list, forwarded by the tick
    /// thread after each tick (same source as the old probe path).
    ignored_randoms: Mutex<Vec<String>>,
    handle: Option<JoinHandle<()>>,
    /// Thread-safe handle used to terminate a runaway tick from this
    /// side of the channel. The terminate stays armed until the isolate
    /// thread has returned from the tick and clears it there (a cancel
    /// from this side would race the interrupt and make it a no-op).
    terminate: v8::IsolateHandle,
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

impl LoadIsolate {
    /// Spawn the isolate thread with already-cached JS (no transpile).
    /// Fails with a message when the source cannot be wired.
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
        let paint_generation =
            NEXT_PAINT_GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
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
                    thread_teardown,
                    thread_proof,
                    #[cfg(feature = "memory-profile")]
                    thread_counters,
                )
            })
            .map_err(|e| format!("isolate thread: {e}"))?;
        let terminate = match setup_rx.recv_timeout(Duration::from_secs(10)) {
            Ok(Ok(handle)) => handle,
            Ok(Err(e)) => return Err(e),
            Err(e) => return Err(format!("isolate init: {e}")),
        };
        Ok(LoadIsolate {
            #[cfg(feature = "memory-profile")]
            counters,
            #[cfg(feature = "memory-profile")]
            last_completed: std::sync::atomic::AtomicU64::new(0),
            #[cfg(feature = "memory-profile")]
            dispatched: std::sync::atomic::AtomicU64::new(0),
            work_generation,
            paint_generation: std::sync::atomic::AtomicU64::new(paint_generation),
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
            terminate,
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
                .filter(|(_, _, started)| started.elapsed() > SLOW_TICK)
                .map(|(_, tick, started)| (*tick, started.elapsed()));
            *in_flight = Some((generation, snap_tick, Instant::now()));
            over
        };
        if let Some((tick, elapsed)) = interrupted {
            // Leave the terminate armed until the isolate thread has
            // returned from the tick (it cancels there); an immediate
            // cancel would race the interrupt and make this a no-op.
            self.terminate.terminate_execution();
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
        self.pump_logs();
        if self.teardown_blocks_dispatch() {
            let _ = self.tx.send(IsolateCmd::Pause);
            return;
        }
        let over = self
            .in_flight
            .lock()
            .unwrap()
            .map(|(_, _, started)| started.elapsed() > SLOW_TICK)
            .unwrap_or(false);
        if over {
            // No cancel here: the isolate thread clears the terminate
            // itself once it has returned from the interrupted tick.
            self.terminate.terminate_execution();
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
                .filter(|(_, _, started)| started.elapsed() > SLOW_TICK)
                .map(|(_, tick, started)| (*tick, started.elapsed()));
            *in_flight = Some((generation, RECOVERY_ANCHOR_TICK, Instant::now()));
            over
        };
        if let Some((tick, elapsed)) = interrupted {
            self.terminate.terminate_execution();
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
        if let Some(paint) = self.paint.lock().unwrap().as_mut() {
            paint.generation = self
                .paint_generation
                .load(std::sync::atomic::Ordering::Acquire);
        }
        *self.in_flight.lock().unwrap() = None;
        let _ = self.tx.send(IsolateCmd::ResetSession);
    }

    /// The latest recorded paint frame (the tick thread forwards the
    /// host handle's `paint` record after every tick that painted).
    /// `None` when the script has not painted yet. No probe
    /// round-trip — the host reads this every frame.
    pub fn paint(&self) -> Option<crate::shim::ScriptPaint> {
        self.pump_logs();
        self.paint.lock().unwrap().clone()
    }

    /// Stop the isolate: tell the thread to exit, interrupt a live tick
    /// so Stop can be processed, and wait for the thread (the Runtime
    /// is dropped there). The wait is bounded by [`Self::JOIN_TIMEOUT`]:
    /// a stuck isolate is abandoned (thread detached) so Stop can never
    /// freeze the panel. Returns leftover log lines (including `onStop`
    /// / `this.log` drained during teardown) after `pump_logs`.
    ///
    /// Tick interruption is claimed only while phase is `Running`. Once
    /// the isolate has entered the hook, only that hook's 50 ms
    /// one-shot may terminate; join never samples `in_flight` to decide.
    pub fn join(mut self) -> Vec<String> {
        let _ = self.tx.send(IsolateCmd::Stop { invoke_hook: true });
        {
            let mut st = self.teardown.lock().unwrap();
            if st.phase == TeardownPhase::Running {
                st.phase = TeardownPhase::UnwindingTick;
                self.terminate.terminate_execution();
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
            }
            // else: abandoned — dropping the handle detaches the thread,
            // which exits on its own once the interrupt lands.
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
                    *self.script_stop.lock().unwrap() =
                        Some(ScriptStopReceipt { tick, reason });
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
                ThreadMsg::Paint(bytes) => {
                    // Decode the FlatBuffer paint frame (no JSON).
                    match crate::isolate_fb::decode_paint(&bytes) {
                        Ok(mut paint) => {
                            paint.generation = self
                                .paint_generation
                                .load(std::sync::atomic::Ordering::Acquire);
                            let mut slot = self.paint.lock().unwrap();
                            if slot.as_ref() != Some(&paint) {
                                *slot = Some(paint);
                            }
                        }
                        Err(e) => self.logs.lock().unwrap().push(format!("paint: {e}")),
                    }
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
        let _ = self.tx.send(IsolateCmd::Stop { invoke_hook: false });
        let mut st = self.teardown.lock().unwrap();
        match st.phase {
            TeardownPhase::Done => {}
            TeardownPhase::Running | TeardownPhase::UnwindingTick | TeardownPhase::Hook => {
                st.phase = TeardownPhase::Done;
                st.cancel.take();
                self.terminate.terminate_execution();
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
    teardown: std::sync::Arc<Mutex<TeardownState>>,
    proof: std::sync::Arc<TeardownProofInner>,
    #[cfg(feature = "memory-profile")] counters: std::sync::Arc<
        crate::memory_profile::Counters,
    >,
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
        teardown,
        proof,
        v2_native,
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

/// Drain the native recorder and compose it with user Paint.end (if any).
fn compose_forwarded_paint(
    runtime: &mut Runtime,
) -> Result<crate::shim::ScriptPaint, rustyscript::Error> {
    let user: Option<crate::shim::ScriptPaint> =
        runtime.eval("globalThis.__rs2b0t_host.paint || null")?;
    Ok(crate::canvas::compose_paint(user))
}
fn forward_paint_if_changed(
    ipc: &mut crate::isolate_fb::IsolateBuf,
    out: &Sender<ThreadMsg>,
    last: &mut Option<crate::shim::ScriptPaint>,
    frame: crate::shim::ScriptPaint,
) {
    if last.as_ref() == Some(&frame) {
        return;
    }
    *last = Some(frame.clone());
    let _ = out.send(ThreadMsg::Paint(ipc.encode_paint(&frame)));
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
    teardown: std::sync::Arc<Mutex<TeardownState>>,
    proof: std::sync::Arc<TeardownProofInner>,
    v2_native: bool,
    #[cfg(feature = "memory-profile")] counters: std::sync::Arc<
        crate::memory_profile::Counters,
    >,
) {
    let _finish = TickLoopFinish(proof.clone());
    #[cfg(feature = "memory-profile")]
    let mut last_heap_sample = None::<Instant>;
    let mut paused = false;
    let mut pending: Option<IsolateCmd> = None;
    // Host-owned hold gate (SEC-004): set from the posted FlatBuffer
    // snapshot, never from a JS-writable `__rs2b0t_host.hold`.
    let mut host_hold = false;
    let mut event_producer = crate::events::NativeEventProducer::new();
    // One reusable encode buffer for this V8 isolate: interact batch
    // and paint frames share it (`reset` between messages).
    let mut ipc = crate::isolate_fb::IsolateBuf::new();
    let mut last_forwarded_paint: Option<crate::shim::ScriptPaint> = None;
    let mut mouse_gestures = MouseGestureIdentities::default();
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
                // Decode the posted FlatBuffer and materialise the JS
                // object the shim reads on the host handle. A
                // malformed blob is logged, never fatal.
                match crate::isolate_fb::SnapshotReader::from_bytes(&bytes.bytes) {
                    Ok(snap) => {
                        crate::bank_open::on_snapshot(&snap);
                        crate::cake_stall::on_snapshot(&snap);
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
                        crate::autocast::on_snapshot(&snap);
                        crate::prayer::on_snapshot(&snap);
                        crate::teleport::on_snapshot(&snap);
                        crate::shop::on_snapshot(&snap);
                        crate::hunt_fight::on_snapshot(&snap);
                        crate::hunt_lair::on_snapshot(&snap);
                        crate::hunt_leave::on_snapshot(&snap);
                        crate::hunt_key::on_snapshot(&snap);
                        crate::hunt_cell::on_snapshot(&snap);
                        crate::hunt_bank::on_snapshot(&snap);
                        crate::production::on_snapshot(&snap);
                        crate::dialog::on_snapshot(&snap);
                        crate::modals::on_snapshot(&snap);
                        crate::quest_journal::on_snapshot(&snap);
                        crate::clue::on_snapshot(&snap);
                        crate::reach::on_snapshot(&snap);
                        crate::line_of_sight::on_snapshot(&snap);
                        crate::fire::on_snapshot(&snap);
                        crate::trade::on_snapshot(&snap);
                        crate::drive_partner_trade::on_snapshot(&snap);
                        if snap.has_hold() {
                            host_hold = snap.hold();
                            crate::periodic_bank::on_hold(host_hold);
                            crate::bank_open::on_hold(host_hold);
                            crate::cake_stall::on_hold(host_hold);
                            crate::walk_wait::on_hold(host_hold);
                            crate::inspect_wait::on_hold(host_hold);
                            crate::death_recovery::on_hold(host_hold);
                            crate::autocast::on_hold(host_hold);
                            crate::prayer::on_hold(host_hold);
                            crate::special::on_hold(host_hold);
                            crate::teleport::on_hold(host_hold);
                            crate::shop::on_hold(host_hold);
                            crate::hunt_fight::on_hold(host_hold);
                            crate::hunt_lair::on_hold(host_hold);
                            crate::hunt_leave::on_hold(host_hold);
                            crate::hunt_key::on_hold(host_hold);
                            crate::hunt_cell::on_hold(host_hold);
                            crate::hunt_bank::on_hold(host_hold);
                            crate::production::on_hold(host_hold);
                            crate::dialog::on_hold(host_hold);
                            crate::modals::on_hold(host_hold);
                            crate::quest_journal::on_hold(host_hold);
                            crate::clue::on_hold(host_hold);
                            crate::reach::on_hold(host_hold);
                            crate::fire::on_hold(host_hold);
                            crate::trade::on_hold(host_hold);
                            crate::drive_partner_trade::on_hold(host_hold);
                        }
                        if let Err(e) = materialize_snapshot(&mut runtime, &snap, host_hold) {
                            let _ = out.send(ThreadMsg::Log(format!("snapshot: {e}")));
                        } else {
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
                let start = Instant::now();
                let observed = event_producer.take_eligible();
                if let Some(diag) = observed.diagnostic {
                    let _ = out.send(ThreadMsg::Log(diag));
                }
                deliver_native_events(&mut runtime, &observed.events, &out);
                // Guardian hold: skip `loop()` AND skip resolving
                // parked conds (time waits too) — the wait stays parked
                // until the hold lifts. Still call `onPaint` so status
                // rows keep updating. Pause already freezes above.
                if host_hold {
                    // Paint-only tick: no loop, no pump. Use `__rs_bot`
                    // (global); module-local `inst` is not visible here.
                    let _ = runtime.eval::<()>(&format!("globalThis.__rs2b0t_host.tick = {n}"));
                    if !v2_native {
                        let _ = runtime.eval::<()>("globalThis.__rs2b0t_call_on_paint()");
                    }
                    let _ = runtime.eval::<()>("if (typeof globalThis.__rs2b0t_flush_native_events === 'function') globalThis.__rs2b0t_flush_native_events()");
                    let _ = runtime.block_on_event_loop(
                        rustyscript::deno_core::PollEventLoopOptions::default(),
                        Some(Duration::from_millis(10)),
                    );
                    if v2_native {
                        let rows: Result<
                            Vec<crate::shim::MaybeInteractReq>,
                            rustyscript::Error,
                        > = runtime.eval("globalThis.__rs2b0t_host.interact || []");
                        let lifecycle: Vec<crate::shim::InteractReq> = rows
                            .unwrap_or_default()
                            .into_iter()
                            .filter_map(|row| match row {
                                crate::shim::MaybeInteractReq::Req(
                                    req @ crate::shim::InteractReq::LoopSettled,
                                ) => Some(req),
                                _ => None,
                            })
                            .collect();
                        if !lifecycle.is_empty() {
                            let _ = out.send(ThreadMsg::Interact {
                                bytes: ipc.encode_interact_batch(&lifecycle),
                                generation,
                            });
                        }
                    }
                    match compose_forwarded_paint(&mut runtime) {
                        Ok(frame) => {
                            forward_paint_if_changed(
                                &mut ipc,
                                &out,
                                &mut last_forwarded_paint,
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
                // A parked Execution wait: settle it (cond / due tick /
                // due time) so the loop's continuation runs — never call
                // `loop()` again while parked. Otherwise start a fresh
                // tick. `__rs2b0t_pump` is async and awaited through the
                // event loop, so the resolved wait's continuation (which
                // may re-park or complete the tick) lands here.
                let parked = runtime
                    .eval::<bool>(
                        "!!(globalThis.__rs2b0t_host && globalThis.__rs2b0t_host.parked)",
                    )
                    .unwrap_or(false);
                let v2_pending = v2_native
                    && runtime
                        .eval::<bool>("!!globalThis.__rs_v2_tick_pending")
                        .unwrap_or(false);
                let result: Result<(), rustyscript::Error> = if parked {
                    // Pump settles the wait (and may re-park), then
                    // paints. Drain so await + onPaint + loop
                    // continuation land on this tick before paint
                    // forward.
                    runtime.call_function(None, "__rs2b0t_pump", json_args!(n))
                } else if v2_pending {
                    // Rust-owned v2 single-flight: do not re-enter tick
                    // while a previous returned Promise is pending.
                    // Snapshot posts still merge; this only skips tick.
                    Ok(())
                } else {
                    // `__rs_tick` is a synchronous entry that returns
                    // immediately (parked or not), so this cannot hang on
                    // a wait.
                    runtime.call_function_immediate(None, "__rs_tick", json_args!(n))
                };
                // Eligible NativeTick only: pause, generation mismatch,
                // and guardian hold already `continue` above. Advance an
                // admitted prayer private pump once per tick, including
                // a sync caller that did not return the Promise.
                // Before: pump ran only on `v2_pending` (and reset).
                // After: one no-op-if-absent call here, then the same
                // single event-loop drain as before — not a second 10ms
                // framework and not a duplicate drain.
                if v2_native {
                    let _ = runtime.eval::<()>(
                        "if (typeof globalThis.__rs_prayer_pump === 'function') globalThis.__rs_prayer_pump()",
                    );
                }
                let _ = runtime.block_on_event_loop(
                    rustyscript::deno_core::PollEventLoopOptions::default(),
                    Some(Duration::from_millis(10)),
                );
                if parked {
                    let _ = runtime.eval::<()>("if (typeof globalThis.__rs2b0t_flush_native_events === 'function') globalThis.__rs2b0t_flush_native_events()");
                }
                // The host may have armed `terminate_execution` to
                // interrupt a slow tick; clear it now that the tick's
                // JS frames have fully unwound. This is the only cancel
                // point — canceling from the host would race the
                // interrupt and make it a no-op.
                runtime
                    .deno_runtime()
                    .v8_isolate()
                    .cancel_terminate_execution();
                let elapsed = start.elapsed();
                #[cfg(feature = "memory-profile")]
                counters.tick(elapsed);
                if let Err(e) = result {
                    let _ = out.send(ThreadMsg::Log(format!("tick {n}: {e}")));
                }
                // Async errors (a cond that throws, a rejected wait)
                // surface on the runner's catch instead of throwing the
                // tick; fold them into the log like sync tick errors.
                let async_err: Option<String> = runtime
                    .eval("(() => { const e = globalThis.__rs2b0t_host.lastError; if (e) { globalThis.__rs2b0t_host.lastError = null; return e; } return null; })()")
                    .unwrap_or(None);
                if let Some(e) = async_err {
                    let _ = out.send(ThreadMsg::Log(format!("tick {n}: {e}")));
                }
                // `LoopingBot.log` / `this.log` push onto the host
                // handle; fold them into the isolate log so BOT_DEBUG
                // and the panel can see script-side lines.
                let bot_log: Result<Vec<String>, rustyscript::Error> =
                    runtime.eval(DRAIN_BOT_LOG);
                if let Ok(rows) = bot_log {
                    for line in rows {
                        let _ = out.send(ThreadMsg::Log(line));
                    }
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
                let rows: Result<Vec<crate::shim::MaybeInteractReq>, rustyscript::Error> =
                    runtime.eval("globalThis.__rs2b0t_host.interact || []");
                let mut reqs: Vec<crate::shim::InteractReq> = rows
                    .unwrap_or_default()
                    .into_iter()
                    .filter_map(|row| match row {
                        crate::shim::MaybeInteractReq::Req(req) => Some(req),
                        crate::shim::MaybeInteractReq::Skip(_) => None,
                    })
                    .collect();
                stamp_mouse_gesture_identities(&mut reqs, input_identity, &mut mouse_gestures);
                crate::inspect_wait::filter_public_inspect_wire(&mut reqs);
                let (enqueued, settled) = take_wait_facts(&mut runtime);
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
                // onPaint is sync; the async runner may still be parked
                // in onStart/loop. Invoke it here so the forward always
                // sees this tick's frame (or the catch/placeholder).
                if !v2_native {
                    let _ = runtime.eval::<()>("globalThis.__rs2b0t_call_on_paint()");
                }
                match compose_forwarded_paint(&mut runtime) {
                    Ok(frame) => {
                        forward_paint_if_changed(
                            &mut ipc,
                            &out,
                            &mut last_forwarded_paint,
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
                        let _ = out
                            .send(ThreadMsg::Log(format!("skipped stale ticks -> {latest}")));
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
                crate::periodic_bank::on_reset();
                crate::bank_open::on_reset();
                crate::cake_stall::on_reset();
                crate::walk_wait::on_reset();
                crate::inspect_wait::on_reset();
                crate::death_recovery::on_reset();
                crate::autocast::on_reset();
                crate::prayer::on_reset();
                crate::special::on_reset();
                crate::teleport::on_reset();
                crate::shop::on_reset();
                crate::hunt_fight::on_reset();
                crate::hunt_lair::on_reset();
                crate::hunt_leave::on_reset();
                crate::hunt_key::on_reset();
                crate::hunt_cell::on_reset();
                crate::hunt_bank::on_reset();
                crate::production::on_reset();
                crate::dialog::on_reset();
                crate::modals::on_reset();
                crate::quest_journal::on_reset();
                crate::clue::on_reset();
                crate::reach::on_reset();
                crate::line_of_sight::on_reset();
                crate::fire::on_reset();
                crate::trade::on_reset();
                crate::drive_partner_trade::on_reset();
                event_producer.reset();
                if v2_native {
                    let _ = runtime.eval::<()>(
                        "if (typeof globalThis.__rs_v2_reset_session === 'function') globalThis.__rs_v2_reset_session()",
                    );
                    let _ = runtime.eval::<()>(
                        "if (typeof globalThis.__rs_prayer_pump === 'function') globalThis.__rs_prayer_pump()",
                    );
                    let _ = runtime.block_on_event_loop(
                        rustyscript::deno_core::PollEventLoopOptions::default(),
                        Some(Duration::from_millis(10)),
                    );
                }
                let _ = runtime.eval::<()>("globalThis.__rs2b0t_host.interact = []");
                clear_unconsumed_paint_click(&mut runtime);
                super::paint_chrome::reset();
                super::paint_jive::reset();
            }
            IsolateCmd::Pause => {
                paused = true;
                let _ = event_producer.set_paused(true);
                crate::periodic_bank::on_pause();
                crate::bank_open::on_pause();
                crate::cake_stall::on_pause();
                crate::walk_wait::on_pause();
                crate::inspect_wait::on_pause();
                crate::death_recovery::on_pause();
                crate::autocast::on_pause();
                crate::prayer::on_pause();
                crate::special::on_pause();
                crate::teleport::on_pause();
                crate::shop::on_pause();
                crate::hunt_fight::on_pause();
                crate::hunt_lair::on_pause();
                crate::hunt_leave::on_pause();
                crate::hunt_key::on_pause();
                crate::hunt_cell::on_pause();
                crate::hunt_bank::on_pause();
                crate::production::on_pause();
                crate::dialog::on_pause();
                crate::modals::on_pause();
                crate::quest_journal::on_pause();
                crate::clue::on_pause();
                crate::reach::on_pause();
                crate::fire::on_pause();
                crate::trade::on_pause();
                crate::drive_partner_trade::on_pause();
                clear_unconsumed_paint_click(&mut runtime);
            }
            IsolateCmd::Resume => {
                paused = false;
                let _ = event_producer.set_paused(false);
                crate::periodic_bank::on_resume();
                crate::bank_open::on_resume();
                crate::cake_stall::on_resume();
                crate::walk_wait::on_resume();
                crate::inspect_wait::on_resume();
                crate::death_recovery::on_resume();
                crate::autocast::on_resume();
                crate::prayer::on_resume();
                crate::special::on_resume();
                crate::teleport::on_resume();
                crate::shop::on_resume();
                crate::hunt_fight::on_resume();
                crate::hunt_lair::on_resume();
                crate::hunt_leave::on_resume();
                crate::hunt_key::on_resume();
                crate::hunt_cell::on_resume();
                crate::hunt_bank::on_resume();
                crate::production::on_resume();
                crate::dialog::on_resume();
                crate::modals::on_resume();
                crate::quest_journal::on_resume();
                crate::clue::on_resume();
                crate::reach::on_resume();
                crate::fire::on_resume();
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
                    Some((x, z, level)) => {
                        crate::shim::InteractReq::RecoveryAnchor { x, z, level }
                    }
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
        globalThis.__release = () => {
            globalThis.__rs2b0t_host.parked = false;
            resolve();
        };
        globalThis.__rs2b0t_host.parked = true;
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
}
