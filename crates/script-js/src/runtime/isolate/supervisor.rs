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
    awaiting_setup: bool,
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
    /// Rows temporarily owned by the worker outside this lock still count
    /// as retiring in concurrent capacity/status snapshots.
    processing_gameplay: usize,
    processing_validation: usize,
    /// Raw Drop may revoke a live isolate without a waiting slot. It remains
    /// supervised and charged rather than becoming an untracked detached thread.
    uncredited: Vec<JoinHandle<()>>,
}

/// Physical process census. Retiring gameplay/validation rows retain their J/V
/// charge; quarantined rows hold an A credit until their OS thread joins.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IsolateCapacityStatus {
    pub active: usize,
    pub retiring_gameplay: usize,
    pub retiring_validation: usize,
    pub quarantined: usize,
    pub quarantined_validation: usize,
    pub uncredited: usize,
}

impl Registry {
    fn status(&self) -> IsolateCapacityStatus {
        IsolateCapacityStatus {
            active: self.active,
            retiring_gameplay: self.retiring.iter().filter(|row| !row.validation).count()
                + self.processing_gameplay,
            retiring_validation: self.retiring.iter().filter(|row| row.validation).count()
                + self.processing_validation,
            quarantined: self.quarantined.len(),
            quarantined_validation: self
                .quarantined
                .iter()
                .filter(|row| row.validation_done.is_some())
                .count(),
            uncredited: self.uncredited.len(),
        }
    }

    /// `None` parks until a producer notifies; only live retirement custody
    /// needs the 5 ms join probe. A registered deadline uses its absolute time.
    fn wait_budget(&self) -> Option<Duration> {
        let until = self.deadlines.iter().map(|entry| entry.at).min();
        let retiring = !self.retiring.is_empty()
            || !self.quarantined.is_empty()
            || !self.uncredited.is_empty();
        match (until, retiring) {
            (None, false) => None,
            (Some(at), false) => Some(at.saturating_duration_since(Instant::now())),
            (Some(at), true) => Some(
                at.saturating_duration_since(Instant::now())
                    .min(Duration::from_millis(5)),
            ),
            (None, true) => Some(Duration::from_millis(5)),
        }
    }
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

/// Snapshot of physical JS admission and retirement, including rows whose
/// logical Stop has completed but whose isolate thread has not joined.
pub fn capacity_status() -> Result<IsolateCapacityStatus, LoadIsolateError> {
    let service = get()?;
    let registry = service
        .state
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    Ok(registry.status())
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
        let mut registry = self
            .0
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
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
        let mut registry = self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        Self::reap(&mut registry);
        if registry.quarantined.len() == QUARANTINE_CAP {
            let status = registry.status();
            return Err(LoadIsolateError::QuarantineFull {
                quarantined: status.quarantined,
                retiring_gameplay: status.retiring_gameplay,
                retiring_validation: status.retiring_validation,
            });
        }
        if registry.active >= ISOLATE_CAP {
            let status = registry.status();
            return Err(LoadIsolateError::IsolateCapacity {
                active: status.active,
                retiring_gameplay: status.retiring_gameplay,
                retiring_validation: status.retiring_validation,
            });
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
        let mut registry = self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
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
        let mut registry = self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
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

    fn retirement_deadline(setup: &super::SetupState, now: Instant) -> Instant {
        match setup {
            super::SetupState::Pending { deadline, .. } => *deadline + JOIN_TIMEOUT,
            _ => now + JOIN_TIMEOUT,
        }
    }

    pub(super) fn retire(&self, isolate: LoadIsolate, done: Sender<Vec<String>>, validation: bool) {
        // Setup retains its original 10-second deadline, followed by the
        // 2-second join allowance. A ready isolate needs only join.
        let setup = isolate
            .setup
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let awaiting_setup = matches!(*setup, super::SetupState::Pending { .. });
        let deadline = Self::retirement_deadline(&setup, Instant::now());
        drop(setup);
        let mut registry = self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        registry.retiring.push(Retirement {
            isolate,
            done,
            deadline,
            awaiting_setup,
            validation,
        });
        self.changed.notify_one();
    }

    pub(super) fn retain_uncredited(&self, handle: JoinHandle<()>) {
        let mut registry = self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
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
        let mut expired = Vec::new();
        let mut retiring = Vec::new();
        let mut still_retiring = Vec::new();
        let mut credited = Vec::new();
        let mut receipts = Vec::new();
        loop {
            let mut registry = self
                .state
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            let now = Instant::now();
            Self::reap(&mut registry);
            // Both deadline and retirement work leave the process registry
            // before taking isolate locks or dropping an isolate.
            expired.clear();
            let mut i = 0;
            while i < registry.deadlines.len() {
                if registry.deadlines[i].at <= now {
                    expired.push(registry.deadlines.swap_remove(i));
                } else {
                    i += 1;
                }
            }
            drop(registry);
            for entry in expired.drain(..) {
                let mut state = entry
                    .teardown
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner());
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
            let mut registry = self
                .state
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            std::mem::swap(&mut retiring, &mut registry.retiring);
            registry.processing_gameplay = retiring.iter().filter(|row| !row.validation).count();
            registry.processing_validation = retiring.len() - registry.processing_gameplay;
            // Only this worker credits quarantines. Reserve the available
            // slots locally; concurrent reaping can only increase capacity.
            let mut quarantine_room = QUARANTINE_CAP - registry.quarantined.len();
            drop(registry);

            // The scratch vectors retain their allocations across join probes.
            let now = Instant::now();
            for mut row in retiring.drain(..) {
                // A setup handle can arrive after Stop. Poll without blocking,
                // then terminate before onStop can take ownership of V8.
                let setup = row.isolate.resolve_setup(None);
                if row.awaiting_setup && setup != super::Ready::Pending {
                    row.awaiting_setup = false;
                    row.deadline = row.deadline.min(Instant::now() + JOIN_TIMEOUT);
                }
                if row
                    .isolate
                    .handle
                    .as_ref()
                    .is_some_and(JoinHandle::is_finished)
                {
                    if let Some(handle) = row.isolate.handle.take() {
                        let _ = handle.join();
                    }
                    row.isolate.pump_logs();
                    let lines = std::mem::take(
                        &mut *row
                            .isolate
                            .logs
                            .lock()
                            .unwrap_or_else(|poison| poison.into_inner()),
                    );
                    receipts.push((row.done, lines));
                    // Drop the LoadIsolate here, before reacquiring registry.
                } else if now >= row.deadline && quarantine_room != 0 {
                    // A gameplay slot releases J only with this credited A
                    // transfer. Validation keeps its permit until real join.
                    row.isolate.pump_logs();
                    let mut lines = std::mem::take(
                        &mut *row
                            .isolate
                            .logs
                            .lock()
                            .unwrap_or_else(|poison| poison.into_inner()),
                    );
                    lines.push("JS isolate quarantined after join deadline".into());
                    if let Some(handle) = row.isolate.handle.take() {
                        quarantine_room -= 1;
                        if row.validation {
                            credited.push(Quarantined {
                                handle,
                                validation_done: Some((row.done, lines)),
                            });
                        } else {
                            credited.push(Quarantined {
                                handle,
                                validation_done: None,
                            });
                            receipts.push((row.done, lines));
                        }
                    }
                } else {
                    still_retiring.push(row);
                }
            }
            let mut registry = self
                .state
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            registry.processing_gameplay = 0;
            registry.processing_validation = 0;
            registry.retiring.append(&mut still_retiring);
            registry.quarantined.append(&mut credited);
            drop(registry);
            // A receipt is authoritative only after capacity and quarantine
            // state reflect it. Never send while holding the process registry.
            for (done, lines) in receipts.drain(..) {
                let _ = done.send(lines);
            }
            let registry = self
                .state
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            if let Some(duration) = registry.wait_budget() {
                drop(
                    self.changed
                        .wait_timeout(registry, duration)
                        .unwrap_or_else(|poison| poison.into_inner()),
                );
            } else {
                drop(
                    self.changed
                        .wait(registry)
                        .unwrap_or_else(|poison| poison.into_inner()),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_registry_parks_until_notification() {
        let registry = Registry::default();
        assert_eq!(registry.wait_budget(), None);
        assert_eq!(registry.status().quarantined, 0);
        assert_eq!(registry.status().retiring_validation, 0);
    }

    #[test]
    fn setup_stop_preserves_original_setup_deadline_plus_join() {
        let now = Instant::now();
        let (tx, rx) = mpsc::channel();
        let setup = super::super::SetupState::Pending {
            rx,
            deadline: now + super::super::SETUP_TIMEOUT,
        };
        assert_eq!(
            Supervisor::retirement_deadline(&setup, now),
            now + super::super::SETUP_TIMEOUT + JOIN_TIMEOUT
        );
        drop(tx);
        assert_eq!(
            Supervisor::retirement_deadline(&super::super::SetupState::Ready, now),
            now + JOIN_TIMEOUT
        );
    }

    #[test]
    fn quarantined_validation_receipt_waits_for_physical_thread_join() {
        let (release, held) = mpsc::channel();
        let handle = std::thread::spawn(move || held.recv().unwrap());
        let (done, receipt) = mpsc::channel();
        let mut registry = Registry::default();
        registry.quarantined.push(Quarantined {
            handle,
            validation_done: Some((done, vec!["joined validator".into()])),
        });
        assert_eq!(registry.status().quarantined_validation, 1);
        Supervisor::reap(&mut registry);
        assert!(matches!(receipt.try_recv(), Err(mpsc::TryRecvError::Empty)));
        release.send(()).unwrap();
        while !registry.quarantined[0].handle.is_finished() {
            std::thread::yield_now();
        }
        Supervisor::reap(&mut registry);
        assert_eq!(receipt.recv().unwrap(), vec!["joined validator"]);
        assert_eq!(registry.status().quarantined_validation, 0);
    }

    #[test]
    fn isolate_capacity_refuses_without_uncharging_retirees() {
        let service = Arc::new(Supervisor {
            state: Mutex::new(Registry {
                active: ISOLATE_CAP,
                ..Registry::default()
            }),
            changed: Condvar::new(),
            alive: AtomicBool::new(true),
        });
        assert!(matches!(
            service.admit(),
            Err(LoadIsolateError::IsolateCapacity {
                active: ISOLATE_CAP,
                ..
            })
        ));
        assert_eq!(service.state.lock().unwrap().status().active, ISOLATE_CAP);
    }

    #[test]
    fn poisoned_isolate_deadline_does_not_poison_process_registry() {
        let isolate = LoadIsolate::spawn(
            "export function tick() {}".into(),
            super::super::LoadShape::NativeTick,
            vec![],
        )
        .unwrap();
        assert_eq!(isolate.probe("1").unwrap(), 1);
        assert_eq!(isolate.poll_ready(), super::super::Ready::Ready);
        let teardown = Arc::new(Mutex::new(TeardownState::new()));
        let poisoned = Arc::clone(&teardown);
        let _ = std::thread::spawn(move || {
            let _guard = poisoned.lock().unwrap();
            panic!("isolate-local lock fault");
        })
        .join();
        get()
            .unwrap()
            .arm_hook(
                Arc::clone(&teardown),
                isolate.terminate.get().unwrap().clone(),
                Instant::now(),
            )
            .unwrap();
        let until = Instant::now() + Duration::from_secs(2);
        loop {
            let guard = get().unwrap().state.lock().unwrap();
            let pending = guard
                .deadlines
                .iter()
                .any(|deadline| Arc::ptr_eq(&deadline.teardown, &teardown));
            drop(guard);
            if !pending {
                break;
            }
            assert!(
                Instant::now() < until,
                "poisoned deadline was not processed"
            );
            std::thread::yield_now();
        }
        assert_eq!(worker_count(), 1);
        let peer = LoadIsolate::spawn(
            "export function tick() {}".into(),
            super::super::LoadShape::NativeTick,
            vec![],
        )
        .unwrap();
        assert_eq!(peer.probe("1").unwrap(), 1);
        peer.join();
        isolate.join();
    }

    #[test]
    fn poisoned_retiree_does_not_stop_process_supervisor() {
        const CHILD: &str = "SCRIPT_POISONED_RETIREE_CHILD";
        const TEST_NAME: &str =
            "isolate::supervisor::tests::poisoned_retiree_does_not_stop_process_supervisor";
        if std::env::var_os(CHILD).is_none() {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", TEST_NAME, "--nocapture"])
                .env(CHILD, "1")
                .output()
                .unwrap();
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(
                output.status.success(),
                "poisoned retirement child failed\nstdout: {stdout}\nstderr: {stderr}"
            );
            assert!(
                stdout.contains(&format!("test {TEST_NAME} ... ok")) && stdout.contains("1 passed"),
                "poisoned retirement child did not run exactly once\nstdout: {stdout}\nstderr: {stderr}"
            );
            return;
        }

        let isolate = LoadIsolate::spawn(
            "export function tick() {}".into(),
            super::super::LoadShape::NativeTick,
            vec![],
        )
        .unwrap();
        assert_eq!(isolate.probe("1").unwrap(), 1);
        assert_eq!(isolate.poll_ready(), super::super::Ready::Ready);
        let poisoned = Arc::clone(&isolate.teardown);
        assert!(std::thread::spawn(move || {
            let _guard = poisoned.lock().unwrap();
            panic!("poison a real isolate teardown");
        })
        .join()
        .is_err());
        let (done, receipt) = mpsc::channel();
        isolate.send(super::super::IsolateCmd::Stop { invoke_hook: false });
        get().unwrap().retire(isolate, done, false);
        let _lines = receipt
            .recv_timeout(Duration::from_secs(3))
            .expect("poisoned isolate must complete retirement");
        // The receipt is the authority boundary: every row and processing
        // count must already be published in the registry when it arrives.
        let registry = get()
            .unwrap()
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        assert!(registry.retiring.is_empty());
        assert_eq!(registry.processing_gameplay, 0);
        drop(registry);
        assert_eq!(worker_count(), 1, "one poisoned isolate cannot stop JS");
        let peer = LoadIsolate::spawn(
            "export function tick() {}".into(),
            super::super::LoadShape::NativeTick,
            vec![],
        )
        .unwrap();
        assert_eq!(peer.probe("1").unwrap(), 1);
        peer.join();
    }
}
