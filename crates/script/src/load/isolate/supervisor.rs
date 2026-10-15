//! One process deadline and retirement owner, independent of the game reactor.
//! Only finished OS threads are joined. A timeout never releases a live thread.

use super::teardown::{ExecutionInterrupt, InterruptedExecution, TeardownPhase, TeardownState};
use super::{LoadIsolate, LoadIsolateError, JOIN_TIMEOUT};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Condvar, LazyLock, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const QUARANTINE_CAP: usize = 4;
const ISOLATE_CAP: usize = 128;

enum DeadlineKind {
    Execution { id: u64, owner: ExecutionInterrupt },
    Hook,
}

struct Deadline {
    teardown: Arc<Mutex<TeardownState>>,
    handle: v8::IsolateHandle,
    at: Instant,
    kind: DeadlineKind,
}

struct Retirement {
    isolate: LoadIsolate,
    done: Sender<Vec<String>>,
    deadline: Instant,
    validation: bool,
}

struct Quarantined {
    handle: JoinHandle<()>,
    validation_done: Option<(Sender<Vec<String>>, Vec<String>)>,
}

#[derive(Default)]
struct Registry {
    deadlines: Vec<Deadline>,
    active: usize,
    retiring: Vec<Retirement>,
    quarantined: Vec<Quarantined>,
    /// Raw Drop may revoke a live isolate without a waiting slot. It remains
    /// supervised and charged rather than becoming an untracked detached thread.
    uncredited: Vec<JoinHandle<()>>,
}

pub(super) struct Supervisor {
    state: Mutex<Registry>,
    changed: Condvar,
    alive: AtomicBool,
}

static SUPERVISOR: LazyLock<Result<Arc<Supervisor>, String>> = LazyLock::new(|| {
    let service = Arc::new(Supervisor {
        state: Mutex::new(Registry::default()),
        changed: Condvar::new(),
        alive: AtomicBool::new(false),
    });
    let worker = Arc::clone(&service);
    let (ready_tx, ready_rx) = mpsc::sync_channel(0);
    std::thread::Builder::new()
        .name("js-deadline-supervisor".into())
        .spawn(move || {
            worker.alive.store(true, Ordering::Release);
            let _ = ready_tx.send(());
            worker.run();
        })
        .map_err(|error| error.to_string())?;
    // A delayed worker must not become orphaned after the caller's bounded
    // ready wait: retain its sole owner and refuse admission while `alive`
    // is false, rather than discarding the service and its thread.
    let _ = ready_rx.recv_timeout(Duration::from_secs(1));
    Ok(service)
});

pub(super) fn get() -> Result<&'static Arc<Supervisor>, LoadIsolateError> {
    super::super::platform::require_v8_platform()?;
    SUPERVISOR
        .as_ref()
        .map_err(|error| LoadIsolateError::Supervisor(error.clone()))
}

pub(super) fn worker_count() -> usize {
    get().map_or(0, |service| {
        usize::from(service.alive.load(Ordering::Acquire))
    })
}

struct WorkerAlive<'a>(&'a AtomicBool);

impl Drop for WorkerAlive<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

/// The reservation stays in the isolate thread through V8 destruction.
/// Neither timeout nor dropping a public handle releases this credit.
pub(super) struct Permit(Arc<Supervisor>);

impl Drop for Permit {
    fn drop(&mut self) {
        let mut registry = self.0.state.lock().unwrap();
        registry.active -= 1;
        self.0.changed.notify_one();
    }
}

impl Supervisor {
    /// A full quarantine refuses all new JS, including validation. Uncredited
    /// retirees still count against the finite process isolate reservation.
    pub(super) fn admit(self: &Arc<Self>) -> Result<Permit, LoadIsolateError> {
        if !self.alive.load(Ordering::Acquire) {
            return Err(LoadIsolateError::Supervisor(
                "supervisor worker stopped".into(),
            ));
        }
        let mut registry = self.state.lock().unwrap();
        Self::reap(&mut registry);
        if registry.quarantined.len() == QUARANTINE_CAP {
            return Err(LoadIsolateError::QuarantineFull);
        }
        if registry.active >= ISOLATE_CAP {
            return Err(LoadIsolateError::IsolateCapacity);
        }
        registry.active += 1;
        Ok(Permit(Arc::clone(self)))
    }

    pub(super) fn arm_execution(
        &self,
        teardown: Arc<Mutex<TeardownState>>,
        handle: v8::IsolateHandle,
        id: u64,
        owner: ExecutionInterrupt,
        at: Instant,
    ) -> Result<(), LoadIsolateError> {
        if !self.alive.load(Ordering::Acquire) {
            return Err(LoadIsolateError::Supervisor(
                "supervisor worker stopped".into(),
            ));
        }
        let mut registry = self.state.lock().unwrap();
        if registry.deadlines.len() >= ISOLATE_CAP * 2 {
            return Err(LoadIsolateError::SupervisorCapacity);
        }
        registry.deadlines.push(Deadline {
            teardown,
            handle,
            at,
            kind: DeadlineKind::Execution { id, owner },
        });
        self.changed.notify_one();
        Ok(())
    }

    pub(super) fn arm_hook(
        &self,
        teardown: Arc<Mutex<TeardownState>>,
        handle: v8::IsolateHandle,
        at: Instant,
    ) -> Result<(), LoadIsolateError> {
        if !self.alive.load(Ordering::Acquire) {
            return Err(LoadIsolateError::Supervisor(
                "supervisor worker stopped".into(),
            ));
        }
        let mut registry = self.state.lock().unwrap();
        if registry.deadlines.len() >= ISOLATE_CAP * 2 {
            return Err(LoadIsolateError::SupervisorCapacity);
        }
        registry.deadlines.push(Deadline {
            teardown,
            handle,
            at,
            kind: DeadlineKind::Hook,
        });
        self.changed.notify_one();
        Ok(())
    }

    pub(super) fn retire(&self, isolate: LoadIsolate, done: Sender<Vec<String>>, validation: bool) {
        let mut registry = self.state.lock().unwrap();
        registry.retiring.push(Retirement {
            isolate,
            done,
            deadline: Instant::now() + JOIN_TIMEOUT,
            validation,
        });
        self.changed.notify_one();
    }

    pub(super) fn retain_uncredited(&self, handle: JoinHandle<()>) {
        let mut registry = self.state.lock().unwrap();
        registry.uncredited.push(handle);
        self.changed.notify_one();
    }

    fn reap(registry: &mut Registry) {
        let mut i = 0;
        while i < registry.quarantined.len() {
            if registry.quarantined[i].handle.is_finished() {
                let row = registry.quarantined.swap_remove(i);
                let _ = row.handle.join();
                if let Some((done, lines)) = row.validation_done {
                    let _ = done.send(lines);
                }
            } else {
                i += 1;
            }
        }
        let mut i = 0;
        while i < registry.uncredited.len() {
            if registry.uncredited[i].is_finished() {
                let _ = registry.uncredited.swap_remove(i).join();
            } else {
                i += 1;
            }
        }
    }

    fn run(self: Arc<Self>) {
        self.alive.store(true, Ordering::Release);
        let _alive = WorkerAlive(&self.alive);
        loop {
            let mut registry = self.state.lock().unwrap();
            let now = Instant::now();
            Self::reap(&mut registry);
            let mut i = 0;
            while i < registry.deadlines.len() {
                if registry.deadlines[i].at > now {
                    i += 1;
                    continue;
                }
                let entry = registry.deadlines.swap_remove(i);
                let mut state = entry.teardown.lock().unwrap();
                match entry.kind {
                    DeadlineKind::Execution { id, owner } => {
                        if state.phase == TeardownPhase::Running
                            && state.execution_id == id
                            && state.execution_active
                            && state.execution_interrupt.is_none()
                        {
                            let owner = state.execution_deadline_owner.take().unwrap_or(owner);
                            if owner != ExecutionInterrupt::Pause || state.pause_requested {
                                state.execution_interrupt = Some(InterruptedExecution {
                                    owner,
                                    consumed: false,
                                });
                                entry.handle.terminate_execution();
                            }
                        }
                        if state.execution_id == id {
                            state.execution_deadline_armed = false;
                        }
                    }
                    DeadlineKind::Hook => {
                        if state.phase == TeardownPhase::Hook && !state.interrupt_issued {
                            state.interrupt_issued = true;
                            entry.handle.terminate_execution();
                        }
                    }
                }
            }
            let mut i = 0;
            while i < registry.retiring.len() {
                let row = &mut registry.retiring[i];
                // A setup handle can arrive after Stop. Poll without blocking,
                // then terminate before onStop can take ownership of V8.
                let _ = row.isolate.resolve_setup(None);
                if row
                    .isolate
                    .handle
                    .as_ref()
                    .is_some_and(JoinHandle::is_finished)
                {
                    let mut row = registry.retiring.swap_remove(i);
                    if let Some(handle) = row.isolate.handle.take() {
                        let _ = handle.join();
                    }
                    row.isolate.pump_logs();
                    let lines = std::mem::take(&mut *row.isolate.logs.lock().unwrap());
                    let _ = row.done.send(lines);
                } else if now >= row.deadline && registry.quarantined.len() < QUARANTINE_CAP {
                    let mut row = registry.retiring.swap_remove(i);
                    // A gameplay slot releases J only with this credited A
                    // transfer. Validation keeps its permit until real join.
                    row.isolate.pump_logs();
                    let mut lines = std::mem::take(&mut *row.isolate.logs.lock().unwrap());
                    lines.push("JS isolate quarantined after join deadline".into());
                    if let Some(handle) = row.isolate.handle.take() {
                        if row.validation {
                            registry.quarantined.push(Quarantined {
                                handle,
                                validation_done: Some((row.done, lines)),
                            });
                        } else {
                            registry.quarantined.push(Quarantined {
                                handle,
                                validation_done: None,
                            });
                            let _ = row.done.send(lines);
                        }
                    }
                } else {
                    i += 1;
                }
            }
            let until = registry
                .deadlines
                .iter()
                .map(|entry| entry.at)
                .min()
                .unwrap_or_else(|| now + Duration::from_millis(5));
            let duration = until
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(5));
            let _guard = self.changed.wait_timeout(registry, duration).unwrap();
        }
    }
}
