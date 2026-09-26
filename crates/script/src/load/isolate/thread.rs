use super::teardown::*;
use super::*;

#[derive(Default)]
pub(super) struct MouseGestureIdentities {
    pub(super) pairs: VecDeque<u64>,
    /// Number of leading ups that cannot be paired after overflow. The
    /// count is bounded storage and makes them fail closed at identity 0.
    pub(super) unpairable: u64,
}

pub(super) fn stamp_mouse_gesture_identities(
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

/// A value Rust writes onto the host handle without running any JS.
enum HostValue<'a> {
    Number(f64),
    Str(&'a str),
    Null,
}

/// Set `__rs2b0t_host[key]` through V8 directly: typed, no script eval.
/// `false` when the host handle is missing or the write failed.
fn set_host_field(runtime: &mut Runtime, key: &str, value: HostValue<'_>) -> bool {
    let scope = &mut runtime.deno_runtime().handle_scope();
    let global = scope.get_current_context().global(scope);
    let (Ok(host_key), Ok(field)) = (key_string(scope, "__rs2b0t_host"), key_string(scope, key))
    else {
        return false;
    };
    let Some(host) = global
        .get(scope, host_key.into())
        .and_then(|host| host.to_object(scope))
    else {
        return false;
    };
    let value: v8::Local<v8::Value> = match value {
        HostValue::Number(n) => v8::Number::new(scope, n).into(),
        HostValue::Str(s) => match v8::String::new(scope, s) {
            Some(s) => s.into(),
            None => return false,
        },
        HostValue::Null => v8::null(scope).into(),
    };
    host.set(scope, field.into(), value).is_some()
}

/// Record the eligible tick number on the host handle
/// (`__rs2b0t_host.tick`) before any of the tick's JS runs.
fn record_tick(runtime: &mut Runtime, n: u64) {
    set_host_field(runtime, "tick", HostValue::Number(n as f64));
}

/// `!!globalThis[name]`, read through V8 without compiling a script.
fn global_flag(runtime: &mut Runtime, name: &str) -> bool {
    let scope = &mut runtime.deno_runtime().handle_scope();
    let global = scope.get_current_context().global(scope);
    let Ok(key) = key_string(scope, name) else {
        return false;
    };
    global
        .get(scope, key.into())
        .is_some_and(|value| value.boolean_value(scope))
}

/// Drops every machine row when the tick loop ends (Stop, script stop).
struct MachinesStop;
impl Drop for MachinesStop {
    fn drop(&mut self) {
        crate::machine::on_stop();
        crate::hunt::on_stop();
    }
}

/// Counts actual Runtime ownership, not merely live JoinHandles.
struct RuntimeLifetime;

impl RuntimeLifetime {
    fn enter() -> Self {
        LIVE_RUNTIMES.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        Self
    }
}

impl Drop for RuntimeLifetime {
    fn drop(&mut self) {
        LIVE_RUNTIMES.fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
    }
}

/// The isolate thread: create the Runtime, wire the module, hand the
/// thread-safe isolate handle back, then run the tick loop.
#[allow(clippy::too_many_arguments)] // channel endpoints plus optional diagnostics
pub(super) fn isolate_main(
    source: String,
    shape: LoadShape,
    siblings: Vec<(String, String)>,
    game_data: Option<std::sync::Arc<api::game_data::SelectedGameData>>,
    named_banks: std::sync::Arc<api::named_banks::NamedBankFacts>,
    run_policy_override: std::sync::Arc<api::run_policy::RunPolicyOverrideCell>,
    cmds: CmdQueue,
    out: Sender<ThreadMsg>,
    setup: Sender<SetupMessage>,
    work_generation: std::sync::Arc<std::sync::atomic::AtomicU64>,
    paint_generation: std::sync::Arc<std::sync::atomic::AtomicU64>,
    teardown: std::sync::Arc<Mutex<TeardownState>>,
    proof: std::sync::Arc<TeardownProofInner>,
    #[cfg(feature = "memory-profile")] counters: std::sync::Arc<crate::memory_profile::Counters>,
) {
    #[cfg(feature = "memory-profile")]
    let _heap_lifetime = crate::memory_profile::HeapLifetime(counters.clone());
    // Declared before `runtime` so the counter decrements only after the
    // Runtime destructor has completed.
    let _runtime_lifetime: RuntimeLifetime;
    let mut runtime = match Runtime::new(RuntimeOptions {
        timeout: RUNTIME_TIMEOUT,
        max_heap_size: Some(MAX_HEAP),
        ..Default::default()
    }) {
        Ok(runtime) => runtime,
        Err(e) => {
            let _ = setup.send(SetupMessage::Ready(Err(format!("js engine init: {e}"))));
            return;
        }
    };
    _runtime_lifetime = RuntimeLifetime::enter();
    let terminate = runtime.deno_runtime().v8_isolate().thread_safe_handle();
    let setup_handle = terminate.clone();
    let setup_failed = setup.send(SetupMessage::Interrupt(terminate)).is_err();
    {
        let st = teardown.lock().unwrap();
        if setup_failed || st.phase != TeardownPhase::Running {
            setup_handle.terminate_execution();
        }
    }
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
        run_policy_override,
    ) {
        let _ = setup.send(SetupMessage::Ready(Err(e)));
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
    let _ = runtime.eval::<serde_json::Value>(INSTALL_HOST_HOOKS);
    let _ = setup.send(SetupMessage::Ready(Ok(())));
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
    n: u64,
    generation: u64,
    teardown: &Mutex<TeardownState>,
) {
    if events.is_empty() {
        return;
    }
    if let Err(e) = call_interruptible(runtime, teardown, |runtime| {
        dispatch_native_events(runtime, events)
    }) {
        let _ = out.send(ThreadMsg::TickError {
            tick: n,
            generation,
            message: format!("native events: {e}"),
        });
    }
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
    n: u64,
    generation: u64,
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
        let _ = out.send(ThreadMsg::TickError {
            tick: n,
            generation,
            message: format!("paint: {e}"),
        });
        return;
    }
    let _ = out.send(ThreadMsg::Paint(frame));
}

/// Drop an unconsumed one-shot so a later paint cannot return a stale id.
fn clear_unconsumed_paint_click(runtime: &mut Runtime) {
    set_host_field(runtime, "paintClick", HostValue::Null);
}

/// What the tick's JS queued on the host handle, read and cleared by one
/// call (`__rs2b0t_take_tick_output`, no user code runs in it).
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct TickOutput {
    /// Each row read on its own, so no row can cost its siblings or the
    /// wait facts beside them.
    interact: Vec<crate::shim::QueuedRow>,
    /// `typeof` a non-array queue the script left in place of the array.
    queue: Option<String>,
    wait_enqueues: u32,
    wait_settles: u32,
}

/// The tick's taken interact queue and wait facts.
#[derive(Default)]
struct TickRows {
    rows: Vec<crate::shim::MaybeInteractReq>,
    /// `typeof` a non-array queue the script left in place of the array.
    queue: Option<String>,
    enqueued: u32,
    settled: u32,
}

impl TickRows {
    /// Name, under the tick, every row that will not reach the host
    /// because it is malformed — never dropped silently.
    fn log_rejected(&self, out: &Sender<ThreadMsg>, n: u64, generation: u64) {
        if let Some(kind) = &self.queue {
            let _ = out.send(ThreadMsg::TickError {
                tick: n,
                generation,
                message: format!("interact queue: dropped a {kind}, not an array"),
            });
        }
        for row in &self.rows {
            if let crate::shim::MaybeInteractReq::Skip(rejected) = row {
                let _ = out.send(ThreadMsg::TickError {
                    tick: n,
                    generation,
                    message: format!("dropped malformed interact row: {}", rejected.0),
                });
            }
        }
    }
}

/// Take and clear this tick's shim interact queue, and with `facts` its
/// Execution wait enqueue/settle counters (each increment is a real
/// lifecycle fact, including settle+repark in the same pump). A queue that
/// cannot be read at all is logged under the tick.
fn take_tick_output(
    runtime: &mut Runtime,
    out: &Sender<ThreadMsg>,
    n: u64,
    generation: u64,
    facts: bool,
    teardown: &Mutex<TeardownState>,
) -> TickRows {
    match call_interruptible(runtime, teardown, |runtime| {
        runtime.call_function_immediate::<TickOutput>(
            None,
            "__rs2b0t_take_tick_output",
            json_args!(facts),
        )
    }) {
        Ok(output) => TickRows {
            rows: output.interact.into_iter().map(|row| row.0).collect(),
            queue: output.queue,
            enqueued: output.wait_enqueues,
            settled: output.wait_settles,
        },
        Err(e) => {
            let _ = out.send(ThreadMsg::TickError {
                tick: n,
                generation,
                message: format!("interact queue: {e}"),
            });
            TickRows::default()
        }
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
fn drain_event_loop(
    runtime: &mut Runtime,
    out: &Sender<ThreadMsg>,
    n: u64,
    generation: u64,
    teardown: &Mutex<TeardownState>,
) {
    if let Err(e) = call_interruptible(runtime, teardown, |runtime| {
        runtime.block_on_event_loop(
            rustyscript::deno_core::PollEventLoopOptions::default(),
            Some(Duration::from_millis(10)),
        )
    }) {
        let _ = out.send(ThreadMsg::TickError {
            tick: n,
            generation,
            message: e.to_string(),
        });
    }
}

/// Run one pass of the event loop — the microtask checkpoint and ready ops
/// every script eval used to run as it returned — so continuations and
/// rejections queued by the call before it land in this tick. A rejection
/// is logged under the tick.
fn pump_event_loop(
    runtime: &mut Runtime,
    out: &Sender<ThreadMsg>,
    n: u64,
    generation: u64,
    teardown: &Mutex<TeardownState>,
) {
    if let Err(e) = call_interruptible(runtime, teardown, |runtime| {
        runtime.advance_event_loop(rustyscript::deno_core::PollEventLoopOptions::default())
    }) {
        let _ = out.send(ThreadMsg::TickError {
            tick: n,
            generation,
            message: e.to_string(),
        });
    }
}

/// What the tick's JS left on the host handle, read by the tick's one
/// after-tick call (`__rs2b0t_after_tick`).
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct AfterTick {
    /// The recorded error: a throwing onPaint or event callback, a rejected
    /// v2 tick.
    error: Option<String>,
    /// `LoopingBot.log` / `this.log` lines.
    log: Vec<String>,
    paint: PaintRecord,
    /// `None` when user code was not run (a claimed tick).
    ignored_randoms: Option<Vec<String>>,
}

/// The user `Paint.end` record. Read on its own terms: a malformed record
/// is logged without losing the rest of the after-tick read.
struct PaintRecord(Result<Option<crate::shim::ScriptPaint>, String>);

impl<'de> serde::Deserialize<'de> for PaintRecord {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(Self(
            Option::<crate::shim::ScriptPaint>::deserialize(d).map_err(|e| e.to_string()),
        ))
    }
}
/// Finish the tick's one paint/readback pass and forward changed host facts.
/// Arguments are the explicit tick identity plus the isolate-owned caches;
/// keeping them borrowed avoids rebuilding an aggregate on every tick.
#[allow(clippy::too_many_arguments)]
fn finish_tick(
    runtime: &mut Runtime,
    out: &Sender<ThreadMsg>,
    n: u64,
    generation: u64,
    run_paint: bool,
    script_paint: bool,
    claimed: bool,
    teardown: &Mutex<TeardownState>,
    last_paint: &mut Option<std::sync::Arc<crate::shim::ScriptPaint>>,
    paint_generation: &std::sync::atomic::AtomicU64,
    last_ignored: &mut Vec<String>,
) -> Option<String> {
    if run_paint {
        let _ = call_interruptible(runtime, teardown, |runtime| {
            runtime.call_function_immediate::<()>(None, "__rs2b0t_call_on_paint", json_args!())
        });
        pump_event_loop(runtime, out, n, generation, teardown);
    }
    let after: Result<AfterTick, rustyscript::Error> =
        call_interruptible(runtime, teardown, |runtime| {
            runtime.call_function_immediate(
                None,
                "__rs2b0t_after_tick",
                json_args!(script_paint, !claimed),
            )
        });
    let mut paint = None;
    match after {
        Ok(after) => {
            if let Some(e) = after.error {
                let _ = out.send(ThreadMsg::TickError {
                    tick: n,
                    generation,
                    message: e,
                });
            }
            for line in after.log {
                let _ = out.send(ThreadMsg::Log(line));
            }
            match after.paint.0 {
                Ok(user) => paint = Some(user),
                Err(e) if script_paint => {
                    let _ = out.send(ThreadMsg::TickError {
                        tick: n,
                        generation,
                        message: format!("paint eval: {e}"),
                    });
                }
                Err(_) => {}
            }
            if let Some(list) = after.ignored_randoms.filter(|list| list != last_ignored) {
                last_ignored.clone_from(&list);
                let _ = out.send(ThreadMsg::IgnoredRandoms(list));
            }
        }
        Err(e) => {
            let _ = out.send(ThreadMsg::TickError {
                tick: n,
                generation,
                message: format!("after tick: {e}"),
            });
        }
    }
    // A compat bot that may not paint yet forwards an empty frame instead —
    // rs2b0t clears the script layer while `paintBot` is null
    // (`panel/Overlay.ts:43-58`).
    let frame = if script_paint {
        paint.map(crate::canvas::compose_paint)
    } else {
        Some(crate::shim::ScriptPaint::default())
    };
    if let Some(frame) = frame {
        forward_paint_if_changed(out, n, generation, last_paint, paint_generation, frame);
    }
    if !claimed {
        pump_event_loop(runtime, out, n, generation, teardown);
    }
    requested_stop(runtime)
}

/// ScriptRunner.stop's flag and bounded reason, read off the host handle
/// through V8 (no script).
fn requested_stop(runtime: &mut Runtime) -> Option<String> {
    let scope = &mut runtime.deno_runtime().handle_scope();
    let global = scope.get_current_context().global(scope);
    let host_key = key_string(scope, "__rs2b0t_host").ok()?;
    let host = global.get(scope, host_key.into())?.to_object(scope)?;
    let flag = key_string(scope, "stopRequested").ok()?;
    if !host.get(scope, flag.into())?.boolean_value(scope) {
        return None;
    }
    let reason = key_string(scope, "stopReason")
        .ok()
        .and_then(|key| host.get(scope, key.into()))
        .filter(|reason| reason.is_string())
        .map(|reason| reason.to_rust_string_lossy(scope))
        .unwrap_or_default();
    Some(bounded_stop_reason(reason))
}

/// The settle promise of a compat method invoker or a v1 native tick: it
/// fulfils `null` on success or the error text, and never rejects.
type Settle = rustyscript::js_value::Promise<Option<String>>;

/// Record whether this V8 call consumed the active `TerminateExecution`.
/// rustyscript may return `Ok` after unwinding an interrupted event-loop
/// drive and may also cancel V8's termination flag. In that case a no-op
/// entry is the fence: it succeeds only when this call consumed the request.
/// If the host fired after the call returned, the no-op consumes the pending
/// request and this call remains uncut.
fn call_interruptible<T, E>(
    runtime: &mut Runtime,
    teardown: &Mutex<TeardownState>,
    call: impl FnOnce(&mut Runtime) -> Result<T, E>,
) -> Result<T, E> {
    let result = call(runtime);
    let direct_termination = runtime
        .deno_runtime()
        .v8_isolate()
        .is_execution_terminating();
    let interrupted = teardown.lock().unwrap().execution_interrupt.is_some();
    let consumed_here = interrupted && (direct_termination || runtime.eval::<()>("void 0").is_ok());
    if consumed_here {
        if let Some(interrupt) = &mut teardown.lock().unwrap().execution_interrupt {
            interrupt.consumed = true;
        }
    }
    result
}

struct PendingLifecycle {
    promise: Settle,
}

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
    Starting(PendingLifecycle),
    /// `onStart` failed during this tick. The first `loop()` waits for
    /// the next eligible tick, as the pre-F02 runner did.
    StartFailed,
    /// Nothing in flight: the next eligible tick invokes `loop()`/tick.
    Idle,
    /// `loop()` or the native tick in flight.
    Running(PendingLifecycle),
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
    fn poll(
        &mut self,
        runtime: &mut Runtime,
        out: &Sender<ThreadMsg>,
        n: u64,
        generation: u64,
    ) -> bool {
        let (state, is_loop) = match &self.phase {
            Phase::Starting(pending) => (pending.promise.poll_promise(runtime), false),
            Phase::Running(pending) => (pending.promise.poll_promise(runtime), true),
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
                let _ = out.send(ThreadMsg::TickError {
                    tick: n,
                    generation,
                    message: e,
                });
                false
            }
            None => is_loop,
        }
    }
}

fn report_interrupted_execution(
    interrupt: Option<InterruptedExecution>,
    out: &Sender<ThreadMsg>,
    tick: u64,
    generation: u64,
) {
    let Some(interrupt) = interrupt.filter(|interrupt| interrupt.consumed) else {
        return;
    };
    let owner = match interrupt.owner {
        ExecutionInterrupt::Watchdog => "watchdog",
        ExecutionInterrupt::Pause => "Pause deadline",
        ExecutionInterrupt::SessionReset => "session-reset deadline",
    };
    let _ = out.send(ThreadMsg::TickError {
        tick,
        generation,
        message: format!("runaway execution interrupted by {owner}"),
    });
    let _ = out.send(ThreadMsg::ScriptCut);
}

/// One eligible non-v2 tick, in the phase order the isolate owns (the
/// tick is already recorded and machines stepped): tick listeners, wait
/// settle, `onStart` once (compat), native events once started, then
/// `loop()`/`tick` when nothing is in flight. Sets `loop_settled` when a
/// compat `loop()` settle is observed here and `iteration_completed` for
/// every clean loop/native-tick completion.
///
/// A running `loop()` is polled before any of these phases' JS runs: one
/// whose wait settles in this tick's pump (continuations run as each
/// call returns) finishes this tick, and the next `loop()` starts on the
/// next one — at most one `loop()` start per tick. A settling `onStart`
/// is re-polled after the pump so the first `loop()` follows a successful
/// one at once; after a failed one it starts on the next tick.
#[allow(clippy::too_many_arguments)]
fn run_tick_phases(
    runtime: &mut Runtime,
    runner: &mut Runner,
    n: u64,
    generation: u64,
    compat: bool,
    events_consumed: bool,
    out: &Sender<ThreadMsg>,
    loop_settled: &mut bool,
    iteration_completed: &mut bool,
    teardown: &Mutex<TeardownState>,
) -> Result<(), rustyscript::Error> {
    let settled = runner.poll(runtime, out, n, generation);
    *iteration_completed |= settled;
    *loop_settled |= settled && compat;
    if let Phase::StartFailed = runner.phase {
        runner.phase = Phase::Idle;
    }
    // BotHost tick listeners, before any wait settles this tick. Absent
    // when the card never loaded BotHost.
    if machines_halted(teardown) {
        return Ok(());
    }
    let listener = call_interruptible(runtime, teardown, |runtime| {
        runtime.call_function_immediate::<()>(
            None,
            "__rs2b0t_fire_tick_listeners",
            json_args!(),
        )?;
        runtime.advance_event_loop(rustyscript::deno_core::PollEventLoopOptions::default())
    });
    let _ = listener;
    if machines_halted(teardown) {
        return Ok(());
    }
    call_interruptible(runtime, teardown, |runtime| {
        runtime.call_function_immediate::<()>(None, "__rs2b0t_pump", json_args!(n))
    })?;
    if machines_halted(teardown) {
        return Ok(());
    }
    match runner.phase {
        // An onStart a listener or a settled wait just finished lets the
        // first `loop()` run on this tick.
        Phase::Starting(_) => {
            runner.poll(runtime, out, n, generation);
        }
        Phase::Unstarted => {
            // onStart is invoked exactly once: a failed call counts as a
            // failed onStart.
            runner.phase = Phase::StartFailed;
            let start: Settle = call_interruptible(runtime, teardown, |runtime| {
                runtime.call_function_immediate(None, "__rs2b0t_compat_on_start", json_args!())
            })?;
            runner.phase = Phase::Starting(PendingLifecycle { promise: start });
            // Microtasks run as the call returns, so a synchronous
            // onStart has settled here.
            runner.poll(runtime, out, n, generation);
        }
        Phase::StartFailed | Phase::Idle | Phase::Running(_) => {}
    }
    if events_consumed && runner.started() && !machines_halted(teardown) {
        call_interruptible(runtime, teardown, |runtime| {
            runtime.call_function_immediate::<()>(
                None,
                "__rs2b0t_flush_native_events",
                json_args!(),
            )
        })?;
    }
    if machines_halted(teardown) {
        return Ok(());
    }
    if let Phase::Idle = runner.phase {
        if compat {
            let promise: Settle = call_interruptible(runtime, teardown, |runtime| {
                runtime.call_function_immediate(None, "__rs2b0t_compat_loop", json_args!())
            })?;
            runner.phase = Phase::Running(PendingLifecycle { promise });
        } else {
            let run: Option<Settle> = call_interruptible(runtime, teardown, |runtime| {
                runtime.call_function_immediate(None, "__rs_tick", json_args!(n))
            })?;
            if let Some(promise) = run {
                runner.phase = Phase::Running(PendingLifecycle { promise });
            } else {
                *iteration_completed = true;
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

/// Host-handle hooks the tick loop calls by name (no per-call script
/// compile): the exactly-once `onStop` body and its log drain, and the two
/// per-tick reads. `__rs2b0t_take_tick_output` runs no user code, so a
/// slow onPaint can never cost the tick's interact batch.
/// `__rs2b0t_after_tick` reads in the order the per-field evals did — the
/// recorded error, `this.log`, the paint record, the paint-click clear —
/// and only then calls the bot's `ignoredRandoms()`.
const INSTALL_HOST_HOOKS: &str = r#"void (globalThis.__rs2b0t_invoke_on_stop = function () {
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
}, globalThis.__rs2b0t_take_tick_output = function (facts) {
  const h = globalThis.__rs2b0t_host;
  if (!h) return { interact: [], queue: null, waitEnqueues: 0, waitSettles: 0 };
  const rows = h.interact;
  h.interact = [];
  let e = 0, s = 0;
  if (facts) {
    e = Math.max(0, h.waitEnqueues | 0);
    s = Math.max(0, h.waitSettles | 0);
    h.waitEnqueues = 0;
    h.waitSettles = 0;
  }
  const ok = Array.isArray(rows);
  return {
    interact: ok ? rows : [],
    queue: ok || rows == null ? null : typeof rows,
    waitEnqueues: e,
    waitSettles: s,
  };
}, globalThis.__rs2b0t_after_tick = function (readPaint, user) {
  const h = globalThis.__rs2b0t_host;
  let error = null, log = [], paint = null;
  if (h) {
    if (h.lastError) {
      error = String(h.lastError);
      h.lastError = null;
    }
    if (Array.isArray(h.log) && h.log.length > 0) {
      log = h.log.map(String);
      h.log = [];
    }
    if (readPaint) paint = h.paint || null;
    if (h.paintClick != null) h.paintClick = null;
  }
  let ignored = null;
  if (user) {
    ignored = [];
    try {
      const b = globalThis.__rs_bot;
      if (b && typeof b.ignoredRandoms === 'function') {
        const l = b.ignoredRandoms();
        if (Array.isArray(l)) ignored = l.filter((x) => typeof x === 'string');
      }
    } catch (_) {}
  }
  return { error, log, paint, ignoredRandoms: ignored };
}, 0)"#;

const STOP_REASON_MAX_BYTES: usize = 256;

/// ScriptRunner.stop's reason, cut to [`STOP_REASON_MAX_BYTES`] on a char
/// boundary.
fn bounded_stop_reason(mut reason: String) -> String {
    if reason.len() > STOP_REASON_MAX_BYTES {
        let mut end = STOP_REASON_MAX_BYTES;
        while !reason.is_char_boundary(end) {
            end -= 1;
        }
        reason.truncate(end);
    }
    reason
}

/// ScriptRunner.stop from the tick's JS: fold the completed tick, log the
/// stop, and run the exactly-once `onStop` teardown. The caller breaks.
fn stop_on_script_request(
    runtime: &mut Runtime,
    out: &Sender<ThreadMsg>,
    n: u64,
    generation: u64,
    reason: String,
    teardown: &std::sync::Arc<Mutex<TeardownState>>,
    proof: &std::sync::Arc<TeardownProofInner>,
) {
    let _ = out.send(ThreadMsg::ScriptStopped { tick: n, reason });
    let _ = out.send(ThreadMsg::Completed {
        tick: n,
        generation,
        successful: false,
        report_errors: true,
    });
    let _ = out.send(ThreadMsg::Log(format!(
        "script requested stop on tick {n}; isolate stopping"
    )));
    teardown_once(runtime, out, true, teardown, proof);
}

/// One queued command seen by a slow tick's stale-skip. The per-tick posts
/// (snapshot deltas, settings, loadouts and the ticks themselves) keep the
/// window open, and a queued tick of `generation` raises `latest`. Any other
/// command — Pause/Resume, a session reset, paint input, a probe, Stop —
/// ends it: a tick queued after one is a fresh dispatch, not backlog.
fn extends_stale_window(cmd: &IsolateCmd, generation: u64, latest: &mut u64) -> bool {
    match cmd {
        IsolateCmd::Tick {
            tick,
            generation: g,
            ..
        } => {
            if *g == generation {
                *latest = (*latest).max(*tick);
            }
            true
        }
        IsolateCmd::Snapshot(_) | IsolateCmd::Settings(_) | IsolateCmd::Loadouts(_) => true,
        _ => false,
    }
}

/// The tick loop: commands are serialized on this thread; ticks run
/// with a time budget, slow ticks are logged and stale queued ticks are
/// skipped, and errors never kill the isolate.
///
/// `events_consumed` is false for every native shape: the event producer
/// is then never observed and no batch is built, so the isolate pays
/// nothing for events nothing can receive.
///
/// After a slow tick the stale-skip drains the queued commands, in order,
/// into `pending` up to the end of the stale window
/// ([`extends_stale_window`]) and drops the window's ticks of that
/// generation. The window runs past snapshots: the host posts one before
/// every tick, so stopping at the first non-Tick would skip nothing. Every
/// other drained command then runs in its queued order.
#[allow(clippy::too_many_arguments)] // isolate loop owns queues, gens, teardown, and mode flags
fn tick_loop(
    mut runtime: Runtime,
    cmds: CmdQueue,
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
    let mut pending: VecDeque<IsolateCmd> = VecDeque::new();
    // Host-owned hold gate (SEC-004): set from the posted FlatBuffer
    // snapshot, never from a JS-writable `__rs2b0t_host.hold`.
    let mut host_hold = false;
    let mut event_producer = crate::events::NativeEventProducer::new();
    // One reusable encode buffer for this V8 isolate's interact batches
    // (`reset` between messages). Paint frames cross typed, not encoded.
    let mut ipc = crate::isolate_fb::IsolateBuf::new();
    let mut last_forwarded_paint: Option<std::sync::Arc<crate::shim::ScriptPaint>> = None;
    // The ignore list last forwarded; the host starts from the same empty
    // list, so a bot without one never sends it.
    let mut last_ignored_randoms: Vec<String> = Vec::new();
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
        let Some(cmd) = pending.pop_front().or_else(|| cmds.recv()) else {
            break;
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
                        crate::reach::on_snapshot(&snap);
                        if snap.has_hold() {
                            host_hold = snap.hold();
                            crate::periodic_bank::on_hold(host_hold);
                            crate::cake_stall::on_hold(host_hold);
                            crate::walk_wait::on_hold(host_hold);
                            crate::inspect_wait::on_hold(host_hold);
                            crate::machine::on_hold(host_hold);
                            crate::hunt_fight::on_hold(host_hold);
                            crate::hunt_lair::on_hold(host_hold);
                            crate::hunt_leave::on_hold(host_hold);
                            crate::hunt_key::on_hold(host_hold);
                            crate::hunt_cell::on_hold(host_hold);
                            crate::hunt_bank::on_hold(host_hold);
                            crate::quest_journal::on_hold(host_hold);
                            crate::clue::on_hold(host_hold);
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
            IsolateCmd::Loadouts(rows) => super::loadout_v8::post(rows),
            IsolateCmd::Settings(bag) => {
                if let Err(e) = materialize_settings_bag(&mut runtime, &bag) {
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
                    let _ = out.send(ThreadMsg::InFlightDone {
                        generation,
                        tick: n,
                    });
                    continue;
                }
                // Pause intent and execution entry share a lock: either Pause
                // terminates this active eval or this queued tick never enters
                // V8. Stop uses the same phase gate.
                if !begin_interruptible_execution(&teardown, n, generation) {
                    let _ = out.send(ThreadMsg::Completed {
                        tick: n,
                        generation,
                        successful: false,
                        report_errors: false,
                    });
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
                let _ = call_interruptible(&mut runtime, &teardown, |runtime| {
                    super::machine_v8::step(runtime, &|| machines_halted(&teardown));
                    Ok::<(), rustyscript::Error>(())
                });
                // A machine callback may absorb an interrupt. Finish and
                // cancel under the execution lock before returning to the
                // command loop, so it cannot leak into Resume or onStop.
                if machines_halted(&teardown) {
                    let interrupt = finish_interruptible_execution(&mut runtime, &teardown);
                    report_interrupted_execution(interrupt, &out, n, generation);
                    let _ = out.send(ThreadMsg::Completed {
                        tick: n,
                        generation,
                        successful: false,
                        report_errors: true,
                    });
                    continue;
                }
                if events_consumed {
                    let observed = event_producer.take_eligible();
                    if let Some(diag) = observed.diagnostic {
                        let _ = out.send(ThreadMsg::Log(diag));
                    }
                    // A machine callback may have absorbed join's terminate.
                    if !machines_halted(&teardown) {
                        deliver_native_events(
                            &mut runtime,
                            &observed.events,
                            &out,
                            n,
                            generation,
                            &teardown,
                        );
                    }
                }
                // Guardian hold: skip `loop()` AND skip resolving
                // parked conds (time waits too) — the wait stays parked
                // until the hold lifts. Still call `onPaint` so status
                // rows keep updating. Pause already freezes above.
                if host_hold {
                    // Paint-only tick: no loop, no pump. The single paint
                    // pass of a held tick. Use `__rs_bot` (global);
                    // module-local `inst` is not visible here.
                    // Pause/watchdog/Stop may have claimed execution through
                    // a machine or event callback: run no more user JS.
                    let halted = machines_halted(&teardown);
                    let script_paint = !compat || compat_may_paint(&runner);
                    if !v2_native && script_paint && !halted {
                        let _ = call_interruptible(&mut runtime, &teardown, |runtime| {
                            runtime.call_function_immediate::<()>(
                                None,
                                "__rs2b0t_call_on_paint",
                                json_args!(),
                            )
                        });
                        pump_event_loop(&mut runtime, &out, n, generation, &teardown);
                    }
                    if events_consumed && runner.started() && !halted {
                        let _ = call_interruptible(&mut runtime, &teardown, |runtime| {
                            runtime.call_function_immediate::<()>(
                                None,
                                "__rs2b0t_flush_native_events",
                                json_args!(),
                            )
                        });
                    }
                    if !halted {
                        drain_event_loop(&mut runtime, &out, n, generation, &teardown);
                    }
                    // Held gameplay rows are dropped. Execution remains owned
                    // through paint/readback so Pause cannot arm a terminate
                    // after an early cancel and leak it into Resume.
                    let output =
                        take_tick_output(&mut runtime, &out, n, generation, false, &teardown);
                    crate::machine::drop_ops();
                    let stop = finish_tick(
                        &mut runtime,
                        &out,
                        n,
                        generation,
                        false,
                        script_paint,
                        machines_halted(&teardown),
                        &teardown,
                        &mut last_forwarded_paint,
                        &paint_generation,
                        &mut last_ignored_randoms,
                    );
                    // Work that fulfils under hold is still scheduler
                    // progress; its gameplay is dropped above. Polled after
                    // the log read, so a failed loop logs after the tick's
                    // own lines, as it did.
                    let mut lifecycle: Vec<crate::shim::InteractReq> = if v2_native {
                        output
                            .rows
                            .into_iter()
                            .filter_map(|row| match row {
                                crate::shim::MaybeInteractReq::Req(
                                    req @ crate::shim::InteractReq::LoopSettled,
                                ) => Some(req),
                                _ => None,
                            })
                            .collect()
                    } else {
                        Vec::new()
                    };
                    if !v2_native && runner.poll(&mut runtime, &out, n, generation) && compat {
                        lifecycle.push(crate::shim::InteractReq::LoopSettled);
                    }
                    let interrupt = finish_interruptible_execution(&mut runtime, &teardown);
                    report_interrupted_execution(interrupt, &out, n, generation);
                    if !lifecycle.is_empty() {
                        let _ = out.send(ThreadMsg::Interact {
                            bytes: ipc.encode_interact_batch(&lifecycle),
                            generation,
                        });
                    }
                    if let Some(reason) = stop {
                        stop_on_script_request(
                            &mut runtime,
                            &out,
                            n,
                            generation,
                            reason,
                            &teardown,
                            &proof,
                        );
                        break;
                    }
                    let _ = out.send(ThreadMsg::Completed {
                        tick: n,
                        generation,
                        // A guardian-held frame painted but did not run the
                        // script loop, so it cannot clear an active error.
                        successful: false,
                        report_errors: true,
                    });
                    continue;
                }
                // Rust owns the tick phases and the single-flight. Every
                // shape settles its due Execution waits (the pump); v2
                // keeps its JS-flagged single-flight, every other shape
                // runs through the Rust `Runner`. Onward work lands in
                // the drain below.
                let mut loop_settled = false;
                let mut iteration_completed = false;
                let mut result: Result<(), rustyscript::Error> = if machines_halted(&teardown) {
                    // A callback or operator action claimed this execution:
                    // no later user phase may start through that boundary.
                    Ok(())
                } else if v2_native {
                    let pumped = call_interruptible(&mut runtime, &teardown, |runtime| {
                        runtime.call_function_immediate::<()>(None, "__rs2b0t_pump", json_args!(n))
                    });
                    // Do not re-enter tick while a previous returned
                    // Promise is pending. Snapshot posts still merge;
                    // this only skips tick.
                    let ticked = if global_flag(&mut runtime, "__rs_v2_tick_pending")
                        || machines_halted(&teardown)
                    {
                        Ok(())
                    } else {
                        call_interruptible(&mut runtime, &teardown, |runtime| {
                            runtime.call_function_immediate::<()>(None, "__rs_tick", json_args!(n))
                        })
                    };
                    pumped.and(ticked)
                } else {
                    run_tick_phases(
                        &mut runtime,
                        &mut runner,
                        n,
                        generation,
                        compat,
                        events_consumed,
                        &out,
                        &mut loop_settled,
                        &mut iteration_completed,
                        &teardown,
                    )
                };
                // Keep execution ownership through every remaining V8 phase.
                // Pause/watchdog and this thread's final cancel serialize on
                // the teardown mutex, so a stale host sample cannot arm
                // termination for the next tick.
                if !machines_halted(&teardown) {
                    let drained = call_interruptible(&mut runtime, &teardown, |runtime| {
                        runtime.block_on_event_loop(
                            rustyscript::deno_core::PollEventLoopOptions::default(),
                            Some(Duration::from_millis(10)),
                        )
                    });
                    if result.is_ok() {
                        result = drained;
                    }
                }
                // A machine callback whose promise the pump (or the drain)
                // settled resumes its row in this tick. Callback JS is
                // unrelated to the lifecycle single-flight.
                if !machines_halted(&teardown) {
                    let _ = call_interruptible(&mut runtime, &teardown, |runtime| {
                        super::machine_v8::resume_callbacks(runtime, &|| {
                            machines_halted(&teardown)
                        });
                        Ok::<(), rustyscript::Error>(())
                    });
                }
                // A row that ended here settles its machine wait in this tick.
                if !machines_halted(&teardown) {
                    let settled = call_interruptible(&mut runtime, &teardown, |runtime| {
                        super::machine_v8::settle_waits(runtime, &|| machines_halted(&teardown))?;
                        runtime.block_on_event_loop(
                            rustyscript::deno_core::PollEventLoopOptions::default(),
                            Some(Duration::from_millis(10)),
                        )
                    });
                    if let Err(e) = settled {
                        let _ = out.send(ThreadMsg::TickError {
                            tick: n,
                            generation,
                            message: format!("machines: {e}"),
                        });
                    }
                }
                if !v2_native && !machines_halted(&teardown) {
                    // A loop that finished in the drain frees the
                    // single-flight for the next tick.
                    let settled = runner.poll(&mut runtime, &out, n, generation);
                    iteration_completed |= settled;
                    loop_settled |= settled && compat;
                }
                let elapsed = start.elapsed();
                #[cfg(feature = "memory-profile")]
                counters.tick(elapsed);
                let result_error = result.err().map(|error| error.to_string());
                // Forward the tick's shim interact queue (Bank/Banking
                // requests written to `__rs2b0t_host.interact`) to the
                // host, cleared in the same call. The queue is taken only
                // now, after the tick's JS (and any parked continuation)
                // has fully run, so a request reaches the host exactly
                // once. The queue is read through the runtime's value
                // bridge (v8 object walk, not `JSON.parse`) and forwarded
                // as a FlatBuffer batch, not a stringified JSON document.
                // Each row is accepted or rejected locally so a malformed
                // mouse object cannot drop a sibling key.
                // Machine-emitted ops join the batch in Rust at the JS
                // queue position where they were emitted.
                let taken = take_tick_output(&mut runtime, &out, n, generation, true, &teardown);
                taken.log_rejected(&out, n, generation);
                let mut reqs = crate::machine::merge_ops(taken.rows);
                stamp_mouse_gesture_identities(&mut reqs, input_identity, &mut mouse_gestures);
                crate::inspect_wait::filter_public_inspect_wire(&mut reqs);
                if v2_native
                    && reqs
                        .iter()
                        .any(|req| matches!(req, crate::shim::InteractReq::LoopSettled))
                {
                    iteration_completed = true;
                }
                if loop_settled {
                    reqs.push(crate::shim::InteractReq::LoopSettled);
                }
                append_wait_facts(&mut reqs, taken.enqueued, taken.settled);
                if !reqs.is_empty() {
                    let _ = out.send(ThreadMsg::Interact {
                        bytes: ipc.encode_interact_batch(&reqs),
                        generation,
                    });
                }
                // The tick's one onPaint pass and everything else it left
                // on the host handle (`finish_tick`). onPaint is sync and
                // never waits for `loop()`. A compat bot paints once its
                // onStart completed and the scene and stats are ready
                // (`compat_may_paint`), even while `loop()` is parked;
                // native shapes paint every tick.
                let script_paint = !compat || compat_may_paint(&runner);
                // A claimed/paused tick starts no new onPaint work.
                let halted = machines_halted(&teardown);
                let stop = finish_tick(
                    &mut runtime,
                    &out,
                    n,
                    generation,
                    !v2_native && script_paint && !halted,
                    script_paint,
                    halted,
                    &teardown,
                    &mut last_forwarded_paint,
                    &paint_generation,
                    &mut last_ignored_randoms,
                );
                let interrupt = finish_interruptible_execution(&mut runtime, &teardown);
                let was_cut = interrupt.is_some_and(|interrupt| interrupt.consumed);
                if !was_cut {
                    if let Some(message) = result_error {
                        let _ = out.send(ThreadMsg::TickError {
                            tick: n,
                            generation,
                            message,
                        });
                    }
                }
                report_interrupted_execution(interrupt, &out, n, generation);
                // ScriptRunner.stop signal: the script flags the host
                // handle. Fold the completed tick, log the stop, run
                // exactly-once onStop under the isolate-owned 50 ms
                // deadline, then break so the Runtime is dropped.
                if let Some(reason) = stop {
                    stop_on_script_request(
                        &mut runtime,
                        &out,
                        n,
                        generation,
                        reason,
                        &teardown,
                        &proof,
                    );
                    break;
                }
                let mut latest = n;
                if elapsed > EXECUTION_WATCHDOG_HORIZON {
                    let _ = out.send(ThreadMsg::Log(format!("slow tick {n}: {elapsed:?}")));
                    // Skip stale queued ticks, but complete the tick that
                    // actually produced diagnostics. A separate in-flight
                    // acknowledgement clears the newest skipped host tick.
                    let mut window = pending
                        .iter()
                        .take_while(|cmd| extends_stale_window(cmd, generation, &mut latest))
                        .count();
                    if window == pending.len() {
                        while let Some(cmd) = cmds.try_recv() {
                            let open = extends_stale_window(&cmd, generation, &mut latest);
                            pending.push_back(cmd);
                            if !open {
                                break;
                            }
                            window += 1;
                        }
                    }
                    if latest != n {
                        let mut at = 0;
                        pending.retain(|cmd| {
                            at += 1;
                            at > window
                                || !matches!(cmd, IsolateCmd::Tick { generation: g, .. } if *g == generation)
                        });
                        let _ =
                            out.send(ThreadMsg::Log(format!("skipped stale ticks -> {latest}")));
                    }
                }
                let _ = out.send(ThreadMsg::Completed {
                    tick: n,
                    generation,
                    successful: iteration_completed && interrupt.is_none(),
                    report_errors: true,
                });
                if latest != n {
                    let _ = out.send(ThreadMsg::InFlightDone {
                        generation,
                        tick: latest,
                    });
                }
            }
            IsolateCmd::ResetSession => {
                crate::observed::on_reset();
                super::reach_query::on_reset();
                crate::cake_stall::on_reset();
                crate::walk_wait::on_reset();
                crate::inspect_wait::on_reset();
                crate::death_recovery::on_reset();
                crate::machine::on_reset();
                crate::hunt_fight::on_reset();
                crate::hunt_lair::on_reset();
                crate::hunt_leave::on_reset();
                crate::hunt_key::on_reset();
                crate::hunt_cell::on_reset();
                crate::hunt_bank::on_reset();
                crate::quest_journal::on_reset();
                crate::clue::on_reset();
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
                crate::machine::on_pause();
                crate::hunt_fight::on_pause();
                crate::hunt_lair::on_pause();
                crate::hunt_leave::on_pause();
                crate::hunt_key::on_pause();
                crate::hunt_cell::on_pause();
                crate::hunt_bank::on_pause();
                crate::quest_journal::on_pause();
                crate::clue::on_pause();
                clear_unconsumed_paint_click(&mut runtime);
            }
            IsolateCmd::Resume => {
                paused = false;
                let _ = event_producer.set_paused(false);
                crate::periodic_bank::on_resume();
                crate::cake_stall::on_resume();
                crate::walk_wait::on_resume();
                crate::inspect_wait::on_resume();
                crate::machine::on_resume();
                crate::hunt_fight::on_resume();
                crate::hunt_lair::on_resume();
                crate::hunt_leave::on_resume();
                crate::hunt_key::on_resume();
                crate::hunt_cell::on_resume();
                crate::hunt_bank::on_resume();
                crate::quest_journal::on_resume();
                crate::clue::on_resume();
            }
            IsolateCmd::PaintClick { id, generation } => {
                if paused
                    || generation != work_generation.load(std::sync::atomic::Ordering::Acquire)
                {
                    continue;
                }
                if !set_host_field(&mut runtime, "paintClick", HostValue::Str(&id)) {
                    let _ = out.send(ThreadMsg::Log("paintClick: no host handle".into()));
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
                if !begin_interruptible_execution(&teardown, RECOVERY_ANCHOR_TICK, generation) {
                    let _ = out.send(ThreadMsg::InFlightDone {
                        generation,
                        tick: RECOVERY_ANCHOR_TICK,
                    });
                    continue;
                }
                let start = Instant::now();
                let req = match call_interruptible(&mut runtime, &teardown, |runtime| {
                    Ok::<_, rustyscript::Error>(eval_recovery_anchor(runtime))
                })
                .unwrap_or(None)
                {
                    Some((x, z, level)) => crate::shim::InteractReq::RecoveryAnchor { x, z, level },
                    None => crate::shim::InteractReq::RecoveryAnchorNone,
                };
                let interrupt = finish_interruptible_execution(&mut runtime, &teardown);
                let pause_interrupted = interrupt.is_some_and(|interrupt| {
                    interrupt.consumed && interrupt.owner == ExecutionInterrupt::Pause
                });
                report_interrupted_execution(interrupt, &out, RECOVERY_ANCHOR_TICK, generation);
                if start.elapsed() > EXECUTION_WATCHDOG_HORIZON && !pause_interrupted {
                    let _ = out.send(ThreadMsg::Log(format!(
                        "slow recoveryAnchor: {:?}",
                        start.elapsed()
                    )));
                }
                if !pause_interrupted
                    && generation == work_generation.load(std::sync::atomic::Ordering::Acquire)
                {
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
