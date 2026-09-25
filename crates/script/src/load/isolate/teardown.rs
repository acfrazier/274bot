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

/// Phase, Hook-entry deadline, and at-most-one interrupt. Finish and the
/// one-shot worker decide under this same mutex.
pub(super) struct TeardownState {
    pub(super) phase: TeardownPhase,
    pub(super) deadline: Option<Instant>,
    pub(super) interrupt_issued: bool,
    /// Dropped at Done so a sleeping one-shot worker exits without terminate.
    pub(super) cancel: Option<Sender<()>>,
    /// Isolate-scoped test seam: sleep after the deadline owner is armed
    /// and before getter/body, without a process-global switch.
    pub(super) hook_entry_delay: Option<Duration>,
    /// Isolate-scoped test seam: pretend deadline-thread spawn failed.
    pub(super) fail_deadline_spawn: bool,
    /// Unit-test-only wider budget for success-path hook assertions.
    #[cfg(test)]
    pub(super) test_hook_timeout: Option<Duration>,
    /// The slow-tick watchdog armed a terminate the isolate thread has not
    /// cancelled yet. Set with the terminate, cleared with the cancel.
    pub(super) watchdog_fired: bool,
}

impl TeardownState {
    pub(super) fn new() -> Self {
        Self {
            phase: TeardownPhase::Running,
            deadline: None,
            interrupt_issued: false,
            cancel: None,
            hook_entry_delay: None,
            fail_deadline_spawn: false,
            #[cfg(test)]
            test_hook_timeout: None,
            watchdog_fired: false,
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
    pub(super) worker_live: AtomicUsize,
}

impl TeardownProof {
    pub(super) fn new() -> Self {
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

pub(super) struct DeadlineWorkerGuard {
    proof: std::sync::Arc<TeardownProofInner>,
}
impl Drop for DeadlineWorkerGuard {
    fn drop(&mut self) {
        self.proof
            .worker_live
            .fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
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

/// Join (or Drop) has claimed the isolate for Stop and armed a terminate
/// for the running tick. A tick phase must not start after that point: an
/// earlier call whose error is ignored may already have absorbed the
/// terminate, and a spinning `loop()` would then never be interrupted.
/// Join sets the phase before it fires the terminate, so a check made
/// after any absorbing call sees it.
pub(super) fn tick_claimed(teardown: &Mutex<TeardownState>) -> bool {
    teardown.lock().unwrap().phase != TeardownPhase::Running
}

/// A machine pass must stop driving: join claimed the tick, or the
/// watchdog armed a terminate. A terminate that ends a callback's microtask
/// continuation is consumed there and never reported to the caller, so the
/// armed flag is the only trace of it.
pub(super) fn machines_halted(teardown: &Mutex<TeardownState>) -> bool {
    let st = teardown.lock().unwrap();
    st.phase != TeardownPhase::Running || st.watchdog_fired
}

/// Cancel an armed terminate and forget the watchdog's, as one step.
pub(super) fn cancel_terminate(runtime: &mut Runtime, teardown: &Mutex<TeardownState>) {
    let mut st = teardown.lock().unwrap();
    st.watchdog_fired = false;
    runtime
        .deno_runtime()
        .v8_isolate()
        .cancel_terminate_execution();
}

pub(super) fn enter_teardown_hook(teardown: &std::sync::Arc<Mutex<TeardownState>>) -> bool {
    let mut st = teardown.lock().unwrap();
    match st.phase {
        TeardownPhase::Hook | TeardownPhase::Done => false,
        TeardownPhase::Running | TeardownPhase::UnwindingTick => {
            st.phase = TeardownPhase::Hook;
            #[cfg(test)]
            let hook_timeout = st.test_hook_timeout.unwrap_or(SLOW_TICK);
            #[cfg(not(test))]
            let hook_timeout = SLOW_TICK;
            st.deadline = Some(Instant::now() + hook_timeout);
            st.interrupt_issued = false;
            true
        }
    }
}

pub(super) fn finish_teardown_hook(teardown: &std::sync::Arc<Mutex<TeardownState>>) {
    let mut st = teardown.lock().unwrap();
    st.phase = TeardownPhase::Done;
    st.cancel.take();
}

pub(super) fn drain_bot_log(runtime: &mut Runtime, out: &Sender<ThreadMsg>) {
    let bot_log: Result<Vec<String>, rustyscript::Error> =
        runtime.call_function_immediate(None, "__rs2b0t_drain_log", json_args!());
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

/// One-shot worker spawned only at Hook entry. Sleeps until the
/// Hook-entry deadline, then issues at most one terminate while still
/// Hook, under the same mutex as finish. Cancelled by dropping `cancel`.
///
/// The old tick interrupt is cleared under the Hook lock *before* spawn
/// so a deschedule cannot let this worker fire and then be cancelled.
pub(super) fn arm_hook_deadline(
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

/// Exactly-once isolate-thread teardown. The 50 ms deadline is the
/// Instant captured at Hook entry; a one-shot worker (not a parked
/// per-bot thread) issues at most one interrupt under the same mutex
/// as finish. Getter, body, and log-drain share that budget.
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
