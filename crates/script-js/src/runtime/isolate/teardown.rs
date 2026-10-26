use super::*;

/// Host/isolate coordination for Stop vs `onStop`. Transitions are
/// taken under the mutex so join cannot terminate after the hook
/// starts, and the hook's 50 ms one-shot cannot fire into Done.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum TeardownPhase {
    Running,
    UnwindingTick,
    Hook,
    Done,
}

/// Owner of a terminate request aimed at the current tick/recovery eval.
/// Ownership selects the diagnostic only; every consumed request follows the
/// same cut-and-recreate policy.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum ExecutionInterrupt {
    Watchdog,
    Pause,
    SessionReset,
    NoDeadline(DeadlineRefusal),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum DeadlineRefusal {
    Capacity,
    Stopped,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct InterruptedExecution {
    pub(super) owner: ExecutionInterrupt,
    /// Set by the V8-entry fence when the call consumed termination. A
    /// refused independent deadline is terminal even before entry and is
    /// marked here at refusal; later stale requests are cancelled, not cuts.
    pub(super) consumed: bool,
}

/// Teardown coordination and an independent producer evidence sequence.
/// Hook and supervisor decisions use the same mutex.
pub(super) struct TeardownState {
    pub(super) phase: TeardownPhase,
    pub(super) deadline: Option<Instant>,
    pub(super) interrupt_issued: bool,
    /// Isolate-scoped test seam for the shared supervisor.
    /// Isolate-scoped test seam: sleep after the deadline owner is armed
    /// and before getter/body, without a process-global switch.
    pub(super) hook_entry_delay: Option<Duration>,
    /// Test seam: pretend the supervisor cannot accept a hook deadline.
    pub(super) fail_hook_registration: bool,
    /// Test-only wider budget for success-path hook assertions.
    #[cfg(any(test, feature = "test-hooks"))]
    pub(super) test_hook_timeout: Option<Duration>,
    /// Isolate-local fault seam; no process-global supervisor state is changed.
    #[cfg(any(test, feature = "test-hooks"))]
    pub(super) fail_execution_registration: Option<DeadlineRefusal>,
    /// An interruptible tick/recovery eval is currently inside V8. Stop,
    /// Pause, connection reset, and the dispatch watchdog may terminate only
    /// while this is true.
    pub(super) execution_active: bool,
    /// Identity, owning tick, and start time of the active execution.
    /// Deadlines capture the identity so they cannot terminate later work.
    pub(super) execution_id: u64,
    pub(super) execution_tick: u64,
    pub(super) execution_generation: u64,
    pub(super) execution_started: Option<Instant>,
    /// An active execution has at most one registration with the shared
    /// deadline supervisor; Pause/reset may replace its diagnostic owner.
    pub(super) execution_deadline_armed: bool,
    pub(super) execution_deadline_owner: Option<ExecutionInterrupt>,
    /// Pause intent published synchronously by the host. It blocks the next
    /// entry but does not halt healthy work that already owns execution.
    pub(super) pause_requested: bool,
    /// A terminate armed for the active eval and not yet cancelled by the
    /// isolate thread.
    pub(super) execution_interrupt: Option<InterruptedExecution>,
    /// Producer evidence generation. Independent of native input identity
    /// and execution lifecycle; shared only to publish the host's delta key.
    pub(super) snapshot_evidence_sequence: u64,
}

impl TeardownState {
    pub(super) fn new() -> Self {
        Self {
            phase: TeardownPhase::Running,
            deadline: None,
            interrupt_issued: false,
            hook_entry_delay: None,
            fail_hook_registration: false,
            #[cfg(any(test, feature = "test-hooks"))]
            fail_execution_registration: None,
            #[cfg(any(test, feature = "test-hooks"))]
            test_hook_timeout: None,
            execution_active: false,
            execution_id: 0,
            execution_tick: 0,
            execution_generation: 0,
            execution_started: None,
            execution_deadline_armed: false,
            execution_deadline_owner: None,
            pause_requested: false,
            execution_interrupt: None,
            snapshot_evidence_sequence: 0,
        }
    }
}

/// Per-isolate teardown evidence. Survives dropping the public handle so
/// raw Drop can wait for the isolate thread to consume Stop before
/// asserting no hook / no leftover deadline worker.
#[doc(hidden)]
#[derive(Clone)]
pub struct TeardownProof {
    pub(super) inner: std::sync::Arc<TeardownProofInner>,
}

pub(super) struct TeardownProofInner {
    pub(super) invoked: AtomicBool,
    pub(super) finished: AtomicBool,
}

impl TeardownProof {
    pub(super) fn new() -> Self {
        Self {
            inner: std::sync::Arc::new(TeardownProofInner {
                invoked: AtomicBool::new(false),
                finished: AtomicBool::new(false),
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
    /// The independent process supervisor, including after this isolate exits.
    pub fn supervisor_workers(&self) -> usize {
        super::supervisor::worker_count()
    }
}

pub(super) struct TickLoopFinish(pub(super) std::sync::Arc<TeardownProofInner>);
impl Drop for TickLoopFinish {
    fn drop(&mut self) {
        self.0
            .finished
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

/// A machine pass must stop driving once teardown or a budget interrupt has
/// claimed the active execution. Pause intent alone blocks only the next
/// execution; healthy in-budget work already running is allowed to finish.
pub(super) fn machines_halted(teardown: &Mutex<TeardownState>) -> bool {
    let st = teardown.lock().unwrap();
    st.phase != TeardownPhase::Running || st.execution_interrupt.is_some()
}

/// Claim the next tick/recovery eval. This closes the race where Pause sees no
/// active eval immediately before the isolate thread enters a queued one.
pub(super) fn begin_interruptible_execution(
    teardown: &Mutex<TeardownState>,
    tick: u64,
    generation: u64,
) -> bool {
    let mut st = teardown.lock().unwrap();
    if st.phase != TeardownPhase::Running || st.pause_requested {
        return false;
    }
    st.execution_id = st.execution_id.wrapping_add(1);
    st.execution_tick = tick;
    st.execution_generation = generation;
    st.execution_started = Some(Instant::now());
    st.execution_active = true;
    st.execution_deadline_armed = false;
    st.execution_deadline_owner = None;
    true
}

/// Finish the active eval and cancel its terminate while holding the same lock
/// Pause/watchdog use to fire it. They therefore cannot fire after this cancel
/// and leave termination armed for a later eval or `onStop`.
pub(super) fn finish_interruptible_execution(
    runtime: &mut Runtime,
    teardown: &Mutex<TeardownState>,
) -> Option<InterruptedExecution> {
    let mut st = teardown.lock().unwrap();
    st.execution_active = false;
    st.execution_started = None;
    st.execution_deadline_armed = false;
    st.execution_deadline_owner = None;
    let interrupt = st.execution_interrupt.take();
    runtime
        .deno_runtime()
        .v8_isolate()
        .cancel_terminate_execution();
    interrupt
}

pub(super) fn enter_teardown_hook(teardown: &std::sync::Arc<Mutex<TeardownState>>) -> bool {
    let mut st = teardown.lock().unwrap();
    match st.phase {
        TeardownPhase::Hook | TeardownPhase::Done => false,
        TeardownPhase::Running | TeardownPhase::UnwindingTick => {
            st.phase = TeardownPhase::Hook;
            #[cfg(any(test, feature = "test-hooks"))]
            let hook_timeout = st.test_hook_timeout.unwrap_or(ON_STOP_DEADLINE);
            #[cfg(not(any(test, feature = "test-hooks")))]
            let hook_timeout = ON_STOP_DEADLINE;
            st.deadline = Some(Instant::now() + hook_timeout);
            st.interrupt_issued = false;
            true
        }
    }
}

pub(super) fn finish_teardown_hook(teardown: &std::sync::Arc<Mutex<TeardownState>>) {
    let mut st = teardown.lock().unwrap();
    st.phase = TeardownPhase::Done;
}

pub(super) fn drain_bot_log(runtime: &mut Runtime, out: &Sender<ThreadMsg>) {
    let bot_log: Result<Vec<String>, rustyscript::Error> =
        super::super::lineage_v8::call_function_immediate(
            runtime,
            None,
            "__rs2b0t_drain_log",
            json_args!(),
        );
    if let Ok(rows) = bot_log {
        for line in rows {
            let _ = out.send(ThreadMsg::Log(line));
        }
    }
}

pub(super) fn take_hook_entry_delay(
    teardown: &std::sync::Arc<Mutex<TeardownState>>,
) -> Option<Duration> {
    teardown.lock().unwrap().hook_entry_delay.take()
}

/// Register the hook's one absolute deadline on the process supervisor.
/// The phase check and eventual terminate share the teardown mutex.
pub(super) fn arm_hook_deadline(
    runtime: &mut Runtime,
    teardown: &std::sync::Arc<Mutex<TeardownState>>,
) -> bool {
    let deadline = {
        let st = teardown.lock().unwrap();
        if st.phase != TeardownPhase::Hook || st.fail_hook_registration {
            return false;
        }
        st.deadline
            .unwrap_or_else(|| Instant::now() + ON_STOP_DEADLINE)
    };
    runtime
        .deno_runtime()
        .v8_isolate()
        .cancel_terminate_execution();
    let handle = runtime.deno_runtime().v8_isolate().thread_safe_handle();
    if super::supervisor::get()
        .and_then(|service| service.arm_hook(teardown.clone(), handle, deadline))
        .is_err()
    {
        return false;
    }
    if let Some(delay) = take_hook_entry_delay(teardown) {
        std::thread::sleep(delay);
    }
    true
}

pub(super) fn complete_teardown_without_hook(
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

/// Exactly-once isolate-thread teardown. The shared supervisor enforces
/// the 50 ms hook-entry budget across getter, body and log drain.
///
/// Fail closed: if no deadline owner can be created, skip user
/// getter/body/drain, emit a native diagnostic, and finish cleanup.
pub(super) fn teardown_once(
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
    if !arm_hook_deadline(runtime, teardown) {
        complete_teardown_without_hook(
            runtime,
            out,
            teardown,
            Some("onStop skipped: no deadline owner"),
        );
        return;
    }
    proof
        .invoked
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let threw: Result<Option<String>, rustyscript::Error> =
        super::super::lineage_v8::call_function_immediate(
            runtime,
            None,
            "__rs2b0t_invoke_on_stop",
            json_args!(),
        );
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
    let _ = super::super::lineage_v8::eval::<()>(
        runtime,
        "if (globalThis.__rs2b0t_host) globalThis.__rs2b0t_host.interact = []",
    );
    finish_teardown_hook(teardown);
    runtime
        .deno_runtime()
        .v8_isolate()
        .cancel_terminate_execution();
    let _ = out.send(ThreadMsg::Stopped);
}
