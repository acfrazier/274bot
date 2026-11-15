//! The isolate's teardown protocol: one [`TeardownState`] under one mutex.
//!
//! Three actors meet here: the isolate thread (evals, the `onStop` hook), the
//! host slot thread (Pause, Resume, session reset, the dispatch watchdog, Stop,
//! raw Drop) and the process deadline supervisor (expired execution and hook
//! deadlines). Every decision is a transition of [`TeardownState`] taken under
//! the mutex, and every V8 termination request is issued or cancelled under
//! that same mutex. That is what keeps a terminate request from outliving the
//! eval it was armed for.
//!
//! The protocol is checked by `tools/js-teardown-model` (ADR 0017); the
//! conformance tests in `teardown_conformance_tests.rs` drive the functions of
//! this file through the model's traces. The four invariants:
//!
//! 1. The eval that armed a terminate request consumes it, or the isolate
//!    thread cancels it, and no later eval or `onStop` sees it.
//! 2. `onStop` runs at most once and only after no eval is active.
//! 3. The supervisor releases a capacity credit only after the OS thread
//!    joins (`supervisor.rs`).
//! 4. Pause never cuts an eval that already owns execution (inside its
//!    horizon) and always blocks the next tick entry.

use super::*;

/// What the teardown protocol asks of V8 from any thread.
pub(super) trait Terminate {
    fn terminate(&self);
}

impl Terminate for v8::IsolateHandle {
    fn terminate(&self) {
        let _ = self.terminate_execution();
    }
}

/// What the isolate thread asks of V8 to drop a pending termination request.
pub(super) trait CancelTerminate {
    fn cancel_terminate(&mut self);
}

impl CancelTerminate for Runtime {
    fn cancel_terminate(&mut self) {
        self.deno_runtime()
            .v8_isolate()
            .cancel_terminate_execution();
    }
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

/// An interruptible tick, recovery, frame or lifecycle eval that owns V8.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct Execution {
    /// Identity that a deadline captures, so that it cannot terminate later work.
    pub(super) id: u64,
    pub(super) tick: u64,
    /// The work generation that dispatched it.
    pub(super) generation: u64,
    pub(super) started: Instant,
    /// The one registration with the process supervisor, and the diagnostic
    /// owner of its cut. `Some` exactly while a deadline is registered for
    /// this eval; Pause or reset may replace the owner, never the time.
    pub(super) deadline: Option<ExecutionInterrupt>,
    /// A terminate armed for this eval and not yet cancelled by the isolate
    /// thread.
    pub(super) interrupt: Option<InterruptedExecution>,
}

/// Host/isolate coordination for Stop vs `onStop`.
///
/// `Hook` has no execution: `onStop` can only start after the eval is
/// finished, and the isolate thread that runs the hook is the thread that
/// runs the evals. `Hook` is entered once.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum TeardownState {
    Running {
        execution: Option<Execution>,
        /// Pause intent published synchronously by the host. It blocks the
        /// next tick entry but does not halt healthy work that already owns
        /// execution.
        pause_requested: bool,
    },
    /// Stop (or a failed setup) has begun. The eval that was active, if any,
    /// is being cut and still has to finish.
    UnwindingTick { execution: Option<Execution> },
    /// The `onStop` hook owns V8, under one absolute deadline.
    Hook { deadline: Instant, fired: bool },
    /// The hook finished, or raw Drop revoked the handle. After a Drop the
    /// eval that was running still finishes on the isolate thread, so its
    /// interrupt is kept for that finish.
    Done { execution: Option<Execution> },
}

/// Why an eval entry or a hook entry was refused.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum HookRefusal {
    /// The hook already ran or is running.
    Entered,
    /// An eval is still active. The isolate thread runs both, so this is
    /// unreachable; it is refused rather than overlapped.
    ExecutionActive,
}

/// What an expired execution deadline did.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum DeadlineVerdict {
    /// The eval was cut, with this owner.
    Cut(ExecutionInterrupt),
    /// A Pause deadline found Pause resumed: nothing is cut.
    PauseResumed,
    /// No live eval of that identity: nothing is cut.
    Stale,
}

impl TeardownState {
    pub(super) const fn new() -> Self {
        Self::Running {
            execution: None,
            pause_requested: false,
        }
    }

    pub(super) fn execution(&self) -> Option<&Execution> {
        match self {
            Self::Running { execution, .. }
            | Self::UnwindingTick { execution }
            | Self::Done { execution } => execution.as_ref(),
            Self::Hook { .. } => None,
        }
    }

    fn execution_mut(&mut self) -> Option<&mut Execution> {
        match self {
            Self::Running { execution, .. }
            | Self::UnwindingTick { execution }
            | Self::Done { execution } => execution.as_mut(),
            Self::Hook { .. } => None,
        }
    }

    /// The eval that a watchdog, Pause or reset may still act on: Running,
    /// and not yet cut.
    fn live_execution_mut(&mut self) -> Option<&mut Execution> {
        match self {
            Self::Running {
                execution: Some(execution),
                ..
            } if execution.interrupt.is_none() => Some(execution),
            _ => None,
        }
    }

    pub(super) fn is_running(&self) -> bool {
        matches!(self, Self::Running { .. })
    }

    pub(super) fn is_unwinding(&self) -> bool {
        matches!(self, Self::UnwindingTick { .. })
    }

    /// Stop, a failed setup or a revoked handle has ended the running phase.
    pub(super) fn is_stopping(&self) -> bool {
        !self.is_running()
    }

    /// Hook or Done: no new work may dispatch.
    pub(super) fn blocks_dispatch(&self) -> bool {
        matches!(self, Self::Hook { .. } | Self::Done { .. })
    }

    /// A machine pass must stop driving once teardown or a budget interrupt
    /// has claimed the active execution. Pause intent alone blocks only the
    /// next execution; healthy in-budget work already running finishes.
    pub(super) fn halted(&self) -> bool {
        !self.is_running() || self.interrupted()
    }

    pub(super) fn interrupted(&self) -> bool {
        self.execution()
            .is_some_and(|execution| execution.interrupt.is_some())
    }

    /// Enter an eval. Refused outside `Running`, while one is active, and for
    /// a tick or recovery eval while Pause intent is published.
    pub(super) fn begin_execution(
        &mut self,
        id: u64,
        tick: u64,
        generation: u64,
        now: Instant,
        honor_pause: bool,
    ) -> bool {
        let Self::Running {
            execution,
            pause_requested,
        } = self
        else {
            return false;
        };
        if execution.is_some() || (honor_pause && *pause_requested) {
            return false;
        }
        *execution = Some(Execution {
            id,
            tick,
            generation,
            started: now,
            deadline: None,
            interrupt: None,
        });
        true
    }

    /// Leave the active eval and return the terminate armed for it. The
    /// caller cancels V8's request under the same lock.
    pub(super) fn finish_execution(&mut self) -> Option<InterruptedExecution> {
        match self {
            Self::Running { execution, .. }
            | Self::UnwindingTick { execution }
            | Self::Done { execution } => {
                execution.take().and_then(|execution| execution.interrupt)
            }
            Self::Hook { .. } => None,
        }
    }

    /// Publish Pause intent. Only a running isolate parks.
    pub(super) fn request_pause(&mut self) {
        if let Self::Running {
            pause_requested, ..
        } = self
        {
            *pause_requested = true;
        }
    }

    pub(super) fn clear_pause(&mut self) {
        if let Self::Running {
            pause_requested, ..
        } = self
        {
            *pause_requested = false;
        }
    }

    pub(super) fn pause_requested(&self) -> bool {
        matches!(
            self,
            Self::Running {
                pause_requested: true,
                ..
            }
        )
    }

    /// The live eval, for the watchdog's first look.
    pub(super) fn probe(&self) -> Option<&Execution> {
        match self {
            Self::Running {
                execution: Some(execution),
                ..
            } if execution.interrupt.is_none() => Some(execution),
            _ => None,
        }
    }

    /// Register intent to cut the live eval at its own horizon. Returns the
    /// identity and start of the eval when the caller must register a
    /// deadline with the supervisor; `None` when one is already registered
    /// (only the diagnostic owner may change) or no live eval exists.
    pub(super) fn arm_deadline(&mut self, owner: ExecutionInterrupt) -> Option<(u64, Instant)> {
        let execution = self.live_execution_mut()?;
        let armed = execution.deadline.is_some();
        if owner != ExecutionInterrupt::Watchdog || execution.deadline.is_none() {
            execution.deadline = Some(owner);
        }
        (!armed).then_some((execution.id, execution.started))
    }

    /// Mark the live eval `id` cut by `owner`. The caller terminates V8 under
    /// the same lock when this returns true.
    pub(super) fn fire_interrupt(&mut self, id: u64, owner: ExecutionInterrupt) -> bool {
        let Some(execution) = self.live_execution_mut() else {
            return false;
        };
        if execution.id != id {
            return false;
        }
        execution.interrupt = Some(InterruptedExecution {
            owner,
            consumed: matches!(owner, ExecutionInterrupt::NoDeadline(_)),
        });
        true
    }

    /// An execution deadline of eval `id` has expired. The caller terminates
    /// V8 under the same lock on `Cut`.
    pub(super) fn execution_deadline_due(
        &mut self,
        id: u64,
        owner: ExecutionInterrupt,
    ) -> DeadlineVerdict {
        let pause_requested = self.pause_requested();
        let running = self.is_running();
        let verdict = match self.execution_mut() {
            Some(execution) if running && execution.id == id && execution.interrupt.is_none() => {
                let owner = execution.deadline.take().unwrap_or(owner);
                if owner != ExecutionInterrupt::Pause || pause_requested {
                    execution.interrupt = Some(InterruptedExecution {
                        owner,
                        consumed: false,
                    });
                    DeadlineVerdict::Cut(owner)
                } else {
                    DeadlineVerdict::PauseResumed
                }
            }
            _ => DeadlineVerdict::Stale,
        };
        if let Some(execution) = self.execution_mut() {
            if execution.id == id {
                execution.deadline = None;
            }
        }
        verdict
    }

    /// The V8-entry fence saw this call consume the termination.
    pub(super) fn mark_consumed(&mut self) {
        if let Some(interrupt) = self
            .execution_mut()
            .and_then(|execution| execution.interrupt.as_mut())
        {
            interrupt.consumed = true;
        }
    }

    /// Stop: end the running phase. Returns true when the caller must
    /// terminate the isolate now, which is while the tick unwinds and never
    /// once the hook owns V8.
    pub(super) fn begin_stop(&mut self) -> bool {
        if let Self::Running { execution, .. } = *self {
            *self = Self::UnwindingTick { execution };
        }
        self.is_unwinding()
    }

    /// Whether a failed setup may still terminate its isolate: Stop is not
    /// yet in the hook.
    pub(super) fn terminates_before_hook(&self) -> bool {
        matches!(self, Self::Running { .. } | Self::UnwindingTick { .. })
    }

    /// Raw Drop revokes the handle. A running hook keeps its deadline: the
    /// supervisor's hook deadline only fires in `Hook`, and a termination that
    /// Drop issues can be cancelled by the isolate thread before the body.
    pub(super) fn revoke(&mut self) {
        if let Self::Running { execution, .. } | Self::UnwindingTick { execution } = *self {
            *self = Self::Done { execution };
        }
    }

    /// Enter the `onStop` hook once, under one absolute deadline.
    pub(super) fn enter_hook(
        &mut self,
        now: Instant,
        timeout: Duration,
    ) -> Result<(), HookRefusal> {
        match self {
            Self::Hook { .. } | Self::Done { .. } => Err(HookRefusal::Entered),
            Self::Running {
                execution: Some(_), ..
            }
            | Self::UnwindingTick { execution: Some(_) } => Err(HookRefusal::ExecutionActive),
            Self::Running { .. } | Self::UnwindingTick { .. } => {
                *self = Self::Hook {
                    deadline: now + timeout,
                    fired: false,
                };
                Ok(())
            }
        }
    }

    /// The hook's absolute deadline, while the hook owns V8.
    pub(super) fn hook_deadline(&self) -> Option<Instant> {
        match self {
            Self::Hook { deadline, .. } => Some(*deadline),
            _ => None,
        }
    }

    /// The hook deadline has expired. True once, and only in `Hook`: the
    /// caller terminates V8 under the same lock.
    pub(super) fn hook_deadline_due(&mut self) -> bool {
        match self {
            Self::Hook { fired, .. } if !*fired => {
                *fired = true;
                true
            }
            _ => false,
        }
    }

    pub(super) fn finish_hook(&mut self) {
        *self = Self::Done { execution: None };
    }
}

/// Teardown state and the counters and test seams that share its mutex.
/// Hook and supervisor decisions use the same mutex.
pub(super) struct Teardown {
    pub(super) state: TeardownState,
    /// Execution ids issued. The identity outlives its execution so that
    /// lifecycle tests can observe executions too short to sample.
    pub(super) executions: u64,
    /// Producer evidence generation. Independent of native input identity
    /// and execution lifecycle; shared only to publish the host's delta key.
    pub(super) snapshot_evidence_sequence: u64,
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
    /// Test seam: `SeatPage` commands that passed the early gate and ran.
    #[cfg(feature = "test-hooks")]
    pub(super) executed_seat_pages: u64,
}

impl Teardown {
    pub(super) fn new() -> Self {
        Self {
            state: TeardownState::new(),
            executions: 0,
            snapshot_evidence_sequence: 0,
            hook_entry_delay: None,
            fail_hook_registration: false,
            #[cfg(any(test, feature = "test-hooks"))]
            test_hook_timeout: None,
            #[cfg(any(test, feature = "test-hooks"))]
            fail_execution_registration: None,
            #[cfg(feature = "test-hooks")]
            executed_seat_pages: 0,
        }
    }

    /// Enter an eval under a fresh identity. The identity is spent only when
    /// the entry is accepted.
    fn begin_execution(
        &mut self,
        tick: u64,
        generation: u64,
        now: Instant,
        honor_pause: bool,
    ) -> bool {
        let id = self.executions.wrapping_add(1);
        let entered = self
            .state
            .begin_execution(id, tick, generation, now, honor_pause);
        if entered {
            self.executions = id;
        }
        entered
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
pub(super) fn machines_halted(teardown: &Mutex<Teardown>) -> bool {
    teardown.lock().unwrap().state.halted()
}

fn begin_execution(
    teardown: &Mutex<Teardown>,
    tick: u64,
    generation: u64,
    now: Instant,
    honor_pause: bool,
) -> bool {
    teardown
        .lock()
        .unwrap()
        .begin_execution(tick, generation, now, honor_pause)
}

/// Claim the next tick/recovery eval. This closes the race where Pause sees no
/// active eval immediately before the isolate thread enters a queued one.
pub(super) fn begin_interruptible_execution(
    teardown: &Mutex<Teardown>,
    tick: u64,
    generation: u64,
    now: Instant,
) -> bool {
    begin_execution(teardown, tick, generation, now, true)
}

/// Client-frame listeners belong to BotHost, not the paused ScriptRunner
/// work loop, so they remain deliverable while the operator has paused it.
pub(super) fn begin_frame_execution(
    teardown: &Mutex<Teardown>,
    tick: u64,
    generation: u64,
    now: Instant,
) -> bool {
    begin_execution(teardown, tick, generation, now, false)
}

/// Pause/resume lifecycle callbacks are allowed while ScriptRunner work is
/// paused, but their synchronous JavaScript body still has an execution bound.
pub(super) fn begin_lifecycle_hook_execution(
    teardown: &Mutex<Teardown>,
    generation: u64,
    now: Instant,
) -> bool {
    begin_execution(teardown, super::LIFECYCLE_HOOK_TICK, generation, now, false)
}

/// Finish the active eval and cancel its terminate while holding the same lock
/// Pause/watchdog use to fire it. They therefore cannot fire after this cancel
/// and leave termination armed for a later eval or `onStop`.
pub(super) fn finish_interruptible_execution(
    v8: &mut impl CancelTerminate,
    teardown: &Mutex<Teardown>,
) -> Option<InterruptedExecution> {
    let mut st = teardown.lock().unwrap();
    let interrupt = st.state.finish_execution();
    v8.cancel_terminate();
    interrupt
}

/// Cut the live eval `id` now and terminate V8 under the same lock.
pub(super) fn fire_execution_interrupt(
    teardown: &Mutex<Teardown>,
    terminate: Option<&impl Terminate>,
    id: u64,
    owner: ExecutionInterrupt,
) -> bool {
    let mut st = teardown.lock().unwrap();
    if !st.state.fire_interrupt(id, owner) {
        return false;
    }
    if let Some(handle) = terminate {
        handle.terminate();
    }
    true
}

/// A deadline that arming asks the caller to register with the supervisor.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct DeadlineArm {
    pub(super) id: u64,
    pub(super) at: Instant,
}

/// First half of arming, under the lock: the deadline the caller must
/// register for the live eval, at the eval's original start plus the
/// horizon. `None` when one is registered (only the diagnostic owner may
/// change) or no live eval exists.
pub(super) fn begin_arm(
    teardown: &Mutex<Teardown>,
    owner: ExecutionInterrupt,
) -> Option<DeadlineArm> {
    let (id, started) = teardown.lock().unwrap().state.arm_deadline(owner)?;
    Some(DeadlineArm {
        id,
        at: started + EXECUTION_WATCHDOG_HORIZON,
    })
}

/// Second half: the supervisor's answer. A refused registration cuts the
/// eval at once, with no deadline, and returns the refusal.
pub(super) fn finish_arm(
    teardown: &Mutex<Teardown>,
    terminate: &impl Terminate,
    arm: DeadlineArm,
    registration: Result<(), LoadIsolateError>,
) -> Result<(), LoadIsolateError> {
    let Err(error) = registration else {
        return Ok(());
    };
    let reason = match error {
        LoadIsolateError::SupervisorCapacity => DeadlineRefusal::Capacity,
        _ => DeadlineRefusal::Stopped,
    };
    fire_execution_interrupt(
        teardown,
        Some(terminate),
        arm.id,
        ExecutionInterrupt::NoDeadline(reason),
    );
    Err(error)
}

/// Arm the independent process supervisor against the live eval's original
/// start. Changing Pause/reset ownership never extends that deadline.
pub(super) fn arm_active_execution_deadline(
    teardown: &Mutex<Teardown>,
    terminate: &impl Terminate,
    owner: ExecutionInterrupt,
    register: impl FnOnce(u64, Instant) -> Result<(), LoadIsolateError>,
) -> Result<(), LoadIsolateError> {
    let Some(arm) = begin_arm(teardown, owner) else {
        return Ok(());
    };
    finish_arm(teardown, terminate, arm, register(arm.id, arm.at))
}

/// The V8-entry fence. A cut eval is marked consumed when this call consumed
/// the termination; a request fired after the call returned is cancelled at
/// finish and the call stays uncut.
pub(super) fn note_interrupt_consumed(
    teardown: &Mutex<Teardown>,
    consumed_by_call: impl FnOnce() -> bool,
) {
    let interrupted = teardown.lock().unwrap().state.interrupted();
    if interrupted && consumed_by_call() {
        teardown.lock().unwrap().state.mark_consumed();
    }
}

/// The watchdog's first look at the live eval. It reads under the lock and
/// acts after it: the eval can finish in between, which the arm and the fire
/// both re-check.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum WatchdogLook {
    /// Inside the horizon: arm the independent deadline.
    Within,
    /// Past the horizon: cut now.
    Past {
        id: u64,
        tick: u64,
        elapsed: Duration,
    },
}

pub(super) fn watchdog_look(
    teardown: &Mutex<Teardown>,
    ready: bool,
    now: Instant,
) -> Option<WatchdogLook> {
    if !ready {
        return None;
    }
    let st = teardown.lock().unwrap();
    let execution = st.state.probe()?;
    let elapsed = now.saturating_duration_since(execution.started);
    Some(if elapsed <= EXECUTION_WATCHDOG_HORIZON {
        WatchdogLook::Within
    } else {
        WatchdogLook::Past {
            id: execution.id,
            tick: execution.tick,
            elapsed,
        }
    })
}

/// One execution or hook deadline of the process supervisor.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum DeadlineKind {
    Execution { id: u64, owner: ExecutionInterrupt },
    Hook,
}

/// The supervisor's expired deadline. The decision and the termination share
/// the teardown mutex, so a deadline never terminates a later eval, a Pause
/// that was resumed, or an isolate that has left the hook.
pub(super) fn run_deadline(
    teardown: &Mutex<Teardown>,
    kind: DeadlineKind,
    handle: &impl Terminate,
) {
    let mut st = teardown.lock().unwrap_or_else(|poison| poison.into_inner());
    match kind {
        DeadlineKind::Execution { id, owner } => {
            if let DeadlineVerdict::Cut(_) = st.state.execution_deadline_due(id, owner) {
                handle.terminate();
            }
        }
        DeadlineKind::Hook => {
            if st.state.hook_deadline_due() {
                handle.terminate();
            }
        }
    }
}

/// Stop, from the host: end the running phase and terminate the tick eval.
pub(super) fn begin_stop(teardown: &Mutex<Teardown>, terminate: Option<&impl Terminate>) {
    let mut st = lock_after_isolate_failure(teardown);
    if st.state.begin_stop() {
        if let Some(handle) = terminate {
            handle.terminate();
        }
    }
}

/// A failed setup terminates its isolate while Stop has not entered the hook.
pub(super) fn terminate_before_hook(
    teardown: &Mutex<Teardown>,
    terminate: Option<&impl Terminate>,
) {
    let st = lock_after_isolate_failure(teardown);
    if st.state.terminates_before_hook() {
        if let Some(handle) = terminate {
            handle.terminate();
        }
    }
}

/// Raw Drop: revoke the handle's authority. A live thread stays charged to
/// the supervisor until it really exits.
pub(super) fn revoke(teardown: &Mutex<Teardown>) {
    lock_after_isolate_failure(teardown).state.revoke();
}

pub(super) fn enter_teardown_hook(teardown: &Mutex<Teardown>, now: Instant) -> bool {
    let mut st = teardown.lock().unwrap();
    #[cfg(any(test, feature = "test-hooks"))]
    let hook_timeout = st.test_hook_timeout.unwrap_or(ON_STOP_DEADLINE);
    #[cfg(not(any(test, feature = "test-hooks")))]
    let hook_timeout = ON_STOP_DEADLINE;
    st.state.enter_hook(now, hook_timeout).is_ok()
}

pub(super) fn finish_teardown_hook(teardown: &Mutex<Teardown>) {
    teardown.lock().unwrap().state.finish_hook();
}

pub(super) fn drain_bot_log(runtime: &mut Runtime, out: &Sender<ThreadMsg>) {
    let bot_log: Result<Vec<String>, rustyscript::Error> =
        crate::materialize::lineage_v8::call_function_immediate(
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

pub(super) fn take_hook_entry_delay(teardown: &Mutex<Teardown>) -> Option<Duration> {
    teardown.lock().unwrap().hook_entry_delay.take()
}

/// The hook's one absolute deadline, while the hook owns V8. The phase check
/// and the eventual terminate share the teardown mutex.
pub(super) fn hook_deadline(teardown: &Mutex<Teardown>) -> Option<Instant> {
    let st = teardown.lock().unwrap();
    if st.fail_hook_registration {
        return None;
    }
    st.state.hook_deadline()
}

/// Register the hook's one absolute deadline on the process supervisor.
pub(super) fn arm_hook_deadline(
    runtime: &mut Runtime,
    teardown: &std::sync::Arc<Mutex<Teardown>>,
) -> bool {
    let Some(deadline) = hook_deadline(teardown) else {
        return false;
    };
    runtime.cancel_terminate();
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
    teardown: &std::sync::Arc<Mutex<Teardown>>,
    diagnostic: Option<&str>,
) {
    if let Some(line) = diagnostic {
        let _ = out.send(ThreadMsg::Log(line.to_string()));
    }
    finish_teardown_hook(teardown);
    runtime.cancel_terminate();
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
    teardown: &std::sync::Arc<Mutex<Teardown>>,
    proof: &std::sync::Arc<TeardownProofInner>,
) {
    if !enter_teardown_hook(teardown, Instant::now()) {
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
        crate::materialize::lineage_v8::call_function_immediate(
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
    let _ = crate::materialize::lineage_v8::eval::<()>(
        runtime,
        "if (globalThis.__rs2b0t_host) globalThis.__rs2b0t_host.interact = []",
    );
    finish_teardown_hook(teardown);
    runtime.cancel_terminate();
    let _ = out.send(ThreadMsg::Stopped);
}

#[cfg(test)]
#[path = "teardown_conformance_tests.rs"]
mod conformance_tests;
