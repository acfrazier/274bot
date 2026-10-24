//! Process-wide work admission, request threads and the background worker.
//!
//! Each registered request-thread family gets one short-lived thread per job.
//! Its yields continue on that thread. The background worker serves only the
//! families that have not migrated. One ledger retains identity, cancellation,
//! job slots, declared bytes, operation handles and unconsumed terminals.
//! Ordinary steps request a yield after 64 work units or 10 ms.

use std::any::{Any, TypeId};
use std::cell::UnsafeCell;
use std::collections::{HashMap, VecDeque};
use std::num::{NonZeroU32, NonZeroU64, NonZeroUsize};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock, OnceLock, Weak};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use api::identity::{
    Cancellation, Deadline, IdentityError, Notifier, ProcessIncarnation, ProcessOrigin,
    PublishSequence, RequestId, RequestIdMinter, Scope, Settlement, WorkIdentity,
};
use api::process_condition::{
    AdmissionRefusal, ConditionCause, ConditionId, ProcessCapability, ProcessConditions,
    ProcessServiceCode, ProcessServiceCondition,
};
use api::work::{
    AdmissionBounds, CancelOutcome, CooperativeJob, CooperativeViolation, JobError, JobSettlement,
    JobState, LedgerError, MonotonicClock, PreparationPhase, ProgressReport, RetainedState,
    SharedCancellation, SliceBudget, StepContext, StepOutcome, SubmissionRefusal, WorkAdmission,
    WorkByteAdmission, WorkCapacity, WorkCharge, WorkExecution, WorkFamily, WorkFamilySpec,
    WorkPhase, WorkProgress, WorkPublications, WorkTicket, WorkerWake,
};
use parking_lot::{Condvar, Mutex, MutexGuard};
use rand::RngCore;

/// Identity-bound cancellation custody for already-admitted work.
pub mod cancellation;

/// Work units allowed in one dispatch.
pub const WORK_UNITS_PER_SLICE: u32 = 64;
/// Soft elapsed-time checkpoint for ordinary dispatches. Returning after
/// this bound is reported as an overrun, not a cooperation failure.
pub const WORK_SLICE: Duration = Duration::from_millis(10);
/// One verify-and-decode of the pinned 274/289 gathering family (at most
/// 1,411,973 embedded input bytes). Only the first Profile Prepare step may
/// use this qualified allowance; later steps return to the ordinary quantum.
pub(crate) const GATHERING_DECODE_SLICE: Duration = Duration::from_millis(100);
/// One bounded Quester registry action (at most a 1 MiB document). Release
/// samples were 1.324 ms in T1.2 r2 and 1.99 ms in r2 review; the 250 ms
/// reporting allowance also clears the prior 110–136 ms debug observations.
/// The seal returns to the ordinary quantum.
pub(crate) const QUESTER_REGISTRY_SLICE: Duration = Duration::from_millis(250);
/// One bounded Quester Path profile compilation. The largest bundled Path
/// measured 6.197 ms in T1.2 r2 and 7.94 ms in r2 review (release), versus
/// 110–136 ms in the debug host probe. The 250 ms reporting allowance
/// leaves margin on both configurations.
pub(crate) const QUESTER_PROGRESS_SLICE: Duration = Duration::from_millis(250);

/// Closed, T1-internal declarations of the only longer units in this endpoint.
/// Callers cannot supply a duration or widen an arbitrary preparation family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum QualifiedUnit {
    GatheringFamilyDecode,
    QuesterRegistryAction,
    QuesterProgressCompile,
}

/// The shared background worker staying inside one step this long makes
/// background work unavailable. A request step this long only logs a warning.
/// Healthy disk I/O can stall for seconds (a 6.1 s fsync was measured).
/// The sticky latch catches a lost worker, not a slow one.
pub const WORKER_STALL_LIMIT: Duration = Duration::from_secs(60);
/// Maximum time shutdown waits for the worker to return and be joined.
const WORKER_SHUTDOWN_LIMIT: Duration = Duration::from_secs(5);
/// Interval between worker-health checks while waiting for work notifications.
const WORKER_HEALTH_RECHECK_INTERVAL: Duration = Duration::from_secs(1);
/// Warn when one active step takes longer than this to return.
const SLOW_STEP_WARNING_LIMIT: Duration = Duration::from_secs(5);
/// Maximum simultaneously accepted jobs, including unconsumed terminals.
const MAX_WORK_JOBS: usize = 64;
/// Two independently maximal 128 MiB preparations may coexist. Ordinary
/// catalogue and folder work charges their smaller measured footprint; the
/// second reservation is headroom, not permission for unbounded input.
const MAX_WORK_BYTES: usize = 256 * 1024 * 1024 + MAX_WORK_JOBS * PROGRESS_BYTES_PER_JOB;
/// WorkAdmission operation handles across all accepted jobs.
const MAX_OPERATION_HANDLES: u32 = 8;
/// Immutable borrowed snapshots cannot be overwritten while a reader holds
/// `WorkAdmission::progress`. Reserve bounded custody; after it fills, drop
/// later replaceable progress reports without failing the accepted job.
/// Cooperative preparations publish at most 1,027 snapshots regardless of
/// their dispatch count.
const MAX_PROGRESS_SNAPSHOTS_PER_JOB: usize = 32_768;
/// A qualified registry dispatch consumes one scheduler-owned action even
/// when the job yields without reporting progress. The 4,096-entry scan,
/// at most 512 candidates and 64 KiB reads of 1 MiB files fit beneath this
/// cap with margin; an arbitrary callback cannot renew a long allowance.
const MAX_QUALIFIED_DISPATCHES_PER_JOB: usize = MAX_PROGRESS_SNAPSHOTS_PER_JOB;
const PROGRESS_CHUNK: usize = 64;
const PROGRESS_BYTES_PER_JOB: usize = std::mem::size_of::<ProgressHistory>()
    + (MAX_PROGRESS_SNAPSHOTS_PER_JOB - PROGRESS_CHUNK)
        * std::mem::size_of::<OnceLock<WorkProgress>>()
    + (MAX_PROGRESS_SNAPSHOTS_PER_JOB / PROGRESS_CHUNK) * std::mem::size_of::<Box<ProgressBlock>>();
const TERMINAL_PENDING: u8 = 0;
const TERMINAL_PUBLISHED: u8 = 1;
const TERMINAL_CONSUMED: u8 = 2;

type Erased = Box<dyn Any + Send>;

/// Failure to create the process-wide worker service.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum WorkStartError {
    /// The OS CSPRNG failed while naming the process incarnation.
    #[error("{}", ProcessServiceCode::WorkAdmissionUnavailable.operator_text())]
    OsRandom,
}

/// Failure to wait for an admitted job's terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum WorkWaitError {
    #[error("work terminal ledger: {0}")]
    Ledger(#[from] LedgerError),
    /// The sole background worker cannot run queued work while it waits.
    #[error("the WorkAdmission worker cannot wait for its own pending background work")]
    WouldDeadlock,
}

/// Failure to complete an orderly worker shutdown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum WorkShutdownError {
    /// One or more execution threads did not return before the fixed deadline.
    #[error("WorkAdmission shutdown deadline: background_worker={background_worker}, request_threads={request_threads}")]
    ThreadsNotReturned {
        /// Whether the background worker still owns its thread.
        background_worker: bool,
        /// Number of request threads that have not returned.
        request_threads: usize,
    },
    /// The worker thread exited by panicking outside a job boundary.
    #[error("the WorkAdmission worker panicked before shutdown acknowledgement")]
    WorkerPanicked,
}
/// Result of one bounded wait for any WorkAdmission publication or lifecycle change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicationWaitOutcome {
    /// The notification epoch changed; the caller should reread its publication.
    Changed,
    /// No publication or lifecycle change occurred before the supplied timeout.
    TimedOut,
    /// WorkAdmission is shutting down or unavailable for this process incarnation.
    Unavailable,
}

static WORK_INITIALIZED: AtomicBool = AtomicBool::new(false);
static SHARED_WORK: LazyLock<Result<Arc<Mutex<WorkService>>, WorkStartError>> =
    LazyLock::new(|| {
        let incarnation = WorkService::draw_incarnation()?;
        let service = Arc::new(Mutex::new(WorkService::new(incarnation)));
        WORK_INITIALIZED.store(true, Ordering::Release);
        Ok(service)
    });

/// The process-wide WorkAdmission scheduler.
///
/// Hosted processes obtain the sole service through [`WorkService::shared`].
/// The explicit-incarnation builder is private to the service and tests.
pub struct WorkService {
    incarnation: ProcessIncarnation,
    ids: RequestIdMinter,
    bounds: AdmissionBounds,
    jobs: HashMap<RequestId, Arc<JobEntry>>,
    last_job: Option<RequestId>,
    stopping: bool,
    shared: Arc<WorkerShared>,
    _worker: Option<JoinHandle<()>>,
    request_threads: HashMap<RequestId, JoinHandle<()>>,
    worker_joined: bool,
    worker_panicked: bool,
}

impl std::fmt::Debug for WorkService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkService")
            .field("incarnation", &self.incarnation)
            .field("charged", &self.capacity())
            .field("stopping", &self.stopping)
            .field("health", &self.health())
            .finish_non_exhaustive()
    }
}

impl WorkService {
    fn new(incarnation: ProcessIncarnation) -> Self {
        Self::build(incarnation, Arc::new(api::work::SystemClock))
    }

    fn build(
        incarnation: ProcessIncarnation,
        clock: Arc<dyn MonotonicClock + Send + Sync>,
    ) -> Self {
        Self::build_with_spawn(incarnation, clock, |shared| {
            thread::Builder::new()
                .name("work-admission".to_owned())
                .spawn(move || worker_thread(shared))
        })
    }

    fn build_with_spawn(
        incarnation: ProcessIncarnation,
        clock: Arc<dyn MonotonicClock + Send + Sync>,
        spawn: impl FnOnce(Arc<WorkerShared>) -> std::io::Result<JoinHandle<()>>,
    ) -> Self {
        let origin = ProcessOrigin::at(clock.now());
        let bounds = AdmissionBounds::new(
            NonZeroUsize::new(MAX_WORK_JOBS).expect("nonzero fixed bound"),
            NonZeroUsize::new(MAX_WORK_BYTES).expect("nonzero fixed bound"),
            NonZeroU32::new(MAX_OPERATION_HANDLES).expect("nonzero fixed bound"),
        );
        let publications = Arc::new(WorkPublications::new(Scope::Process));
        let notifier = Arc::new(ServiceNotifier::default());
        let shared = Arc::new(WorkerShared {
            incarnation,
            origin,
            clock,
            queue: Mutex::new(WorkerQueue {
                jobs: VecDeque::with_capacity(MAX_WORK_JOBS),
                stopping: false,
            }),
            wake: Condvar::new(),
            active: Mutex::new(None),
            health: Mutex::new(ProcessConditions::new(incarnation)),
            capacity: Mutex::new(CapacityLedger::default()),
            registry: Mutex::new(HashMap::with_capacity(MAX_WORK_JOBS)),
            publications,
            notifier,
            slice_overruns: AtomicU64::new(0),
            progress_drops: AtomicU64::new(0),
            cancellation_noops: AtomicU64::new(0),
            shutdown_requested: AtomicBool::new(false),
            unavailable: AtomicBool::new(false),
            exited: AtomicBool::new(false),
        });
        let worker = match spawn(Arc::clone(&shared)) {
            Ok(worker) => Some(worker),
            Err(_) => {
                // The unavailable service keeps its process identity and
                // typed condition so every later caller receives the frozen
                // restart-required refusal, never a transient Full/start error.
                latch_outage(
                    &shared,
                    ConditionCause::StartFailed,
                    shared.clock.now(),
                    None,
                );
                None
            }
        };
        let worker_joined = worker.is_none();
        Self {
            incarnation,
            ids: RequestIdMinter::new(),
            bounds,
            jobs: HashMap::with_capacity(MAX_WORK_JOBS),
            last_job: None,
            stopping: false,
            shared,
            request_threads: HashMap::with_capacity(MAX_WORK_JOBS),
            _worker: worker,
            worker_joined,
            worker_panicked: false,
        }
    }

    /// Return the process-wide singleton, drawing its process nonce once from
    /// the OS CSPRNG (redrawing only if that draw is zero).
    pub fn shared() -> Result<Arc<Mutex<Self>>, WorkStartError> {
        SHARED_WORK.as_ref().map(Arc::clone).map_err(|error| *error)
    }

    /// Stop the process worker at an application exit without creating one
    /// for a run that never requested background work.
    pub fn shutdown_if_started() -> Result<(), WorkShutdownError> {
        if !WORK_INITIALIZED.load(Ordering::Acquire) {
            return Ok(());
        }
        let Ok(shared) = SHARED_WORK.as_ref() else {
            return Ok(());
        };
        Self::shutdown(shared)
    }

    fn draw_incarnation() -> Result<ProcessIncarnation, WorkStartError> {
        loop {
            let mut bytes = [0_u8; std::mem::size_of::<u64>()];
            rand::rngs::OsRng
                .try_fill_bytes(&mut bytes)
                .map_err(|_| WorkStartError::OsRandom)?;
            if let Some(nonce) = NonZeroU64::new(u64::from_ne_bytes(bytes)) {
                return Ok(ProcessIncarnation::from_start_nonce(nonce));
            }
        }
    }

    /// The process incarnation this service fences.
    pub const fn incarnation(&self) -> ProcessIncarnation {
        self.incarnation
    }

    /// Mint a host request ID and compose it with the requested scope and
    /// optional originating request.
    pub fn mint_identity(
        &mut self,
        scope: Scope,
        origin: Option<RequestId>,
    ) -> Result<WorkIdentity, IdentityError> {
        WorkIdentity::new(self.ids.mint()?, self.incarnation, scope, origin)
    }

    /// Read the service's latched process conditions. The guard is required
    /// because the worker health observer may publish a sticky outage.
    pub fn conditions(&self) -> MutexGuard<'_, ProcessConditions> {
        self.observe_health();
        self.shared.health.lock()
    }

    /// The WorkAdmission condition, if the worker has become unavailable.
    pub fn health(&self) -> Option<ConditionId> {
        self.observe_health();
        self.shared
            .health
            .lock()
            .latched(ProcessServiceCode::WorkAdmissionUnavailable)
            .map(|condition| condition.id())
    }

    /// Current reserved job, byte, and operation-handle capacity.
    pub fn capacity(&self) -> WorkCapacity {
        self.observe_health();
        self.shared.capacity.lock().snapshot()
    }

    /// The service's capacity/progress/terminal publications.
    pub fn publications(&self) -> &WorkPublications {
        self.observe_health();
        &self.shared.publications
    }

    /// Number of dispatches that returned after their soft wall-time
    /// checkpoint. Such delay never changes an accepted job's terminal.
    pub fn slice_overruns(&self) -> u64 {
        self.shared.slice_overruns.load(Ordering::Acquire)
    }

    /// Replaceable progress reports dropped after one job's immutable
    /// snapshot custody fills. Dropping them never changes its terminal.
    pub fn dropped_progress_reports(&self) -> u64 {
        self.shared.progress_drops.load(Ordering::Acquire)
    }

    /// Current internal notification epoch. Publication and shutdown state
    /// changes are recorded before advancing this epoch.
    pub fn publication_epoch(&self) -> u64 {
        self.shared.notifier.epoch.load(Ordering::Acquire)
    }

    /// Explicitly observe worker health without waiting for worker progress.
    pub fn observe_health(&self) {
        let now = self.shared.clock.now();
        let active = *self.shared.active.lock();
        if let Some(active) = active.filter(|active| !active.returned) {
            let elapsed = now.saturating_duration_since(active.started);
            if elapsed >= WORKER_STALL_LIMIT {
                latch_outage(
                    &self.shared,
                    ConditionCause::ProgressLost,
                    now,
                    Some(elapsed),
                );
            }
        }
        for entry in self.jobs.values() {
            if entry.family.policy().execution() != WorkExecution::RequestThread {
                continue;
            }
            let mut active = entry.request_active.lock();
            let Some(active) = active.as_mut().filter(|active| !active.returned) else {
                continue;
            };
            let elapsed = now.saturating_duration_since(active.started);
            if elapsed >= WORKER_STALL_LIMIT && !active.stall_warned {
                active.stall_warned = true;
                api::host_log!(
                    api::hostlog::Category::Lifecycle,
                    api::hostlog::Level::Warn,
                    "request work step stalled after {elapsed:?} family={:?} job={:?}",
                    entry.family,
                    entry.identity.job(),
                );
            }
        }
    }

    /// Wake the sole worker after a command has been published.
    pub fn wake(&self) {
        self.shared.wake_worker();
    }

    /// Stop admission and register `Shutdown` for every accepted job.
    /// Join returned threads until the fixed deadline, without holding the
    /// service mutex during a wait. Pump paths never call this method.
    ///
    /// `ThreadsNotReturned` is not a retirement acknowledgement. The service
    /// retains those handles and their custody until the threads return.
    /// A later call can finish their joins.
    pub fn shutdown(shared: &Arc<parking_lot::Mutex<Self>>) -> Result<(), WorkShutdownError> {
        Self::shutdown_with_timeout(shared, WORKER_SHUTDOWN_LIMIT)
    }

    fn shutdown_with_timeout(
        shared: &Arc<parking_lot::Mutex<Self>>,
        timeout: Duration,
    ) -> Result<(), WorkShutdownError> {
        let deadline = Instant::now() + timeout;
        shared.lock().begin_shutdown();
        loop {
            let (notifier, epoch) = {
                let mut service = shared.lock();
                service.observe_health();
                service.join_finished_threads();
                if service.worker_joined && service.request_threads.is_empty() {
                    return if service.worker_panicked {
                        Err(WorkShutdownError::WorkerPanicked)
                    } else {
                        Ok(())
                    };
                }
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Err(WorkShutdownError::ThreadsNotReturned {
                        background_worker: !service.worker_joined,
                        request_threads: service.request_threads.len(),
                    });
                }
                (
                    Arc::clone(&service.shared.notifier),
                    service.publication_epoch(),
                )
            };
            let remaining = deadline.saturating_duration_since(Instant::now());
            notifier.wait_for_change(epoch, remaining.min(Duration::from_millis(10)));
        }
    }

    // Only shutdown calls this method. Finished handles may be discarded by
    // pump cleanup, but pump paths never join a thread.
    fn join_finished_threads(&mut self) {
        if self._worker.as_ref().is_some_and(JoinHandle::is_finished) {
            let worker = self._worker.take().expect("finished worker handle");
            self.worker_panicked |= worker.join().is_err();
            self.worker_joined = true;
        }
        for (_, thread) in self
            .request_threads
            .extract_if(|_, thread| thread.is_finished())
        {
            self.worker_panicked |= thread.join().is_err();
        }
    }

    fn begin_shutdown(&mut self) {
        if self.stopping {
            return;
        }
        self.stopping = true;
        self.shared
            .shutdown_requested
            .store(true, Ordering::Release);
        for entry in self.jobs.values() {
            request_cancel(&self.shared, entry, Cancellation::Shutdown);
        }
        self.observe_health();
        {
            let mut queue = self.shared.queue.lock();
            queue.stopping = true;
        }
        self.shared.wake.notify_all();
        self.shared.notifier.signal_change();
    }

    /// Whether the worker thread has not yet reported an outage. A `true`
    /// result does not assert that a step is making progress; use `health`.
    pub fn worker_available(&self) -> bool {
        self.observe_health();
        !self.shared.unavailable.load(Ordering::Acquire)
    }
    /// Current accepted-job state for a matching identity-bound ticket.
    pub fn state<F: WorkFamilySpec>(&self, ticket: &WorkTicket<F>) -> Option<JobState> {
        self.observe_health();
        let entry = self
            .jobs
            .get(&ticket.job())
            .filter(|entry| entry.matches(ticket))?;
        if !entry.terminal.is_pending() {
            return None;
        }
        Some(entry.control.lock().state)
    }

    /// Wait for a ticket's terminal from a non-pump context. The service
    /// mutex is released before each condition-variable wait. A wait for
    /// pending background work from this service's worker fails immediately.
    pub fn wait_terminal<F: WorkFamilySpec>(
        shared: &Arc<parking_lot::Mutex<Self>>,
        ticket: &WorkTicket<F>,
    ) -> Result<(), WorkWaitError> {
        loop {
            let (notifier, epoch) = {
                let service = shared.lock();
                service.observe_health();
                let epoch = service.publication_epoch();
                let entry = service
                    .jobs
                    .get(&ticket.job())
                    .filter(|entry| entry.matches(ticket))
                    .ok_or(LedgerError::StaleIdentity)?;
                if entry.terminal.is_consumed() {
                    return Err(LedgerError::AlreadyConsumed.into());
                }
                if !entry.terminal.is_pending() {
                    return Ok(());
                }
                if entry.family.policy().execution() == WorkExecution::BackgroundWorker
                    && service
                        ._worker
                        .as_ref()
                        .is_some_and(|worker| worker.thread().id() == thread::current().id())
                {
                    return Err(WorkWaitError::WouldDeadlock);
                }
                (Arc::clone(&service.shared.notifier), epoch)
            };
            // Recheck health periodically, but never confuse a healthy busy
            // queue with a worker that failed to return from its active step.
            notifier.wait_for_change(epoch, WORKER_HEALTH_RECHECK_INTERVAL);
        }
    }

    /// Park only a non-pump caller until the bounded ledger changes. A Full
    /// refusal retains the producer's input; retry mints a fresh identity.
    pub fn wait_for_capacity_change(
        shared: &Arc<parking_lot::Mutex<Self>>,
        observed: PublishSequence,
    ) {
        loop {
            let (notifier, epoch) = {
                let service = shared.lock();
                if service.stopping
                    || !service.worker_available()
                    || service.shared.publications.capacity().sequence() != observed
                {
                    return;
                }
                (
                    Arc::clone(&service.shared.notifier),
                    service.publication_epoch(),
                )
            };
            notifier.wait_for_change(epoch, WORKER_HEALTH_RECHECK_INTERVAL);
        }
    }

    /// Wait once for a publication change without holding the service mutex.
    ///
    /// The notifier's gate closes the check-to-wait race: if the epoch changed
    /// before this call parks, the notifier returns immediately. `timeout`
    /// bounds every wait; callers must handle `TimedOut` and `Unavailable`.
    pub fn wait_for_publication_change(
        shared: &Arc<parking_lot::Mutex<Self>>,
        observed: u64,
        timeout: Duration,
    ) -> PublicationWaitOutcome {
        let (notifier, epoch) = {
            let service = shared.lock();
            if service.stopping {
                return PublicationWaitOutcome::Unavailable;
            }
            let epoch = service.publication_epoch();
            if epoch != observed {
                return PublicationWaitOutcome::Changed;
            }
            (Arc::clone(&service.shared.notifier), epoch)
        };

        let changed = notifier.wait_for_change(epoch, timeout);
        let service = shared.lock();
        if service.stopping {
            PublicationWaitOutcome::Unavailable
        } else if changed || service.publication_epoch() != observed {
            PublicationWaitOutcome::Changed
        } else {
            PublicationWaitOutcome::TimedOut
        }
    }

    #[cfg(feature = "test-hooks")]
    #[doc(hidden)]
    pub fn with_clock_for_tests(
        incarnation: ProcessIncarnation,
        clock: Arc<dyn MonotonicClock + Send + Sync>,
    ) -> Self {
        Self::build(incarnation, clock)
    }

    fn cleanup_released(&mut self) {
        self.request_threads
            .retain(|_, thread| !thread.is_finished());
        self.jobs.retain(|_, entry| {
            if entry.released.load(Ordering::Acquire) {
                self.shared.registry.lock().remove(&entry.identity.job());
                false
            } else {
                true
            }
        });
    }

    fn refusal_for_unavailable(&self, identity: WorkIdentity) -> Option<SubmissionRefusal> {
        let health = self.shared.health.lock();
        match health.admit(
            ProcessCapability::BackgroundWork,
            Some(identity.job()),
            identity.scope(),
        ) {
            Ok(()) => None,
            Err(AdmissionRefusal::Unavailable(refusal)) => {
                Some(SubmissionRefusal::Unavailable(refusal))
            }
        }
    }

    fn publish_capacity(&self) {
        let snapshot = self.shared.capacity.lock().snapshot();
        let _ = self
            .shared
            .publications
            .capacity()
            .publish(snapshot, &*self.shared.notifier);
    }
}

impl Drop for WorkService {
    fn drop(&mut self) {
        self.begin_shutdown();
        let deadline = Instant::now() + WORKER_SHUTDOWN_LIMIT;
        loop {
            self.join_finished_threads();
            if self.worker_joined && self.request_threads.is_empty() {
                return;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                api::host_log!(
                    api::hostlog::Category::Lifecycle,
                    api::hostlog::Level::Error,
                    "WorkAdmission shutdown deadline: background_worker={} request_threads={}",
                    !self.worker_joined,
                    self.request_threads.len(),
                );
                // Dropping a handle detaches it. Its thread still owns the
                // shared ledger and exact job custody until actual return.
                return;
            }
            let epoch = self.publication_epoch();
            self.shared
                .notifier
                .wait_for_change(epoch, remaining.min(Duration::from_millis(10)));
        }
    }
}

/// A snapshot of actual resource custody. `bytes` includes the fixed bounded
/// per-job progress history as well as the family-supplied charge.
#[derive(Default)]
struct CapacityLedger {
    jobs: usize,
    bytes: usize,
    handles: u32,
    bounded_bytes: usize,
}

impl CapacityLedger {
    fn reserve(
        &mut self,
        bounds: AdmissionBounds,
        charge: WorkCharge,
        byte_admission: WorkByteAdmission,
    ) -> Result<(), SubmissionRefusal> {
        let bytes = charge
            .total_bytes()
            .and_then(|total| total.checked_add(PROGRESS_BYTES_PER_JOB))
            .ok_or(SubmissionRefusal::InvalidArgument)?;
        if (byte_admission == WorkByteAdmission::Bounded && bytes > bounds.max_bytes().get())
            || charge.operation_handles() > bounds.max_handles().get()
        {
            // An impossible single-job charge is permanent, not Full.
            return Err(SubmissionRefusal::InvalidArgument);
        }
        if self.jobs >= bounds.max_jobs().get() {
            return Err(SubmissionRefusal::Full);
        }
        let next_bytes = self
            .bytes
            .checked_add(bytes)
            .ok_or(SubmissionRefusal::InvalidArgument)?;
        let next_bounded = match byte_admission {
            WorkByteAdmission::Bounded => self
                .bounded_bytes
                .checked_add(bytes)
                .ok_or(SubmissionRefusal::InvalidArgument)?,
            WorkByteAdmission::Declared => self.bounded_bytes,
        };
        if next_bounded > bounds.max_bytes().get() {
            return Err(SubmissionRefusal::Full);
        }
        let next_handles = self
            .handles
            .checked_add(charge.operation_handles())
            .ok_or(SubmissionRefusal::InvalidArgument)?;
        if next_handles > bounds.max_handles().get() {
            return Err(SubmissionRefusal::Full);
        }
        self.jobs += 1;
        self.bytes = next_bytes;
        self.handles = next_handles;
        self.bounded_bytes = next_bounded;
        Ok(())
    }

    fn release(&mut self, charge: WorkCharge, byte_admission: WorkByteAdmission) {
        let bytes = charge
            .total_bytes()
            .and_then(|total| total.checked_add(PROGRESS_BYTES_PER_JOB))
            .expect("only representable reserved charges are released");
        self.jobs -= 1;
        self.bytes -= bytes;
        self.handles -= charge.operation_handles();
        if byte_admission == WorkByteAdmission::Bounded {
            self.bounded_bytes -= bytes;
        }
    }

    fn snapshot(&self) -> WorkCapacity {
        WorkCapacity::new(self.jobs, self.bytes, self.handles)
    }
}

#[derive(Clone, Copy)]
struct ActiveStep {
    entry: RequestId,
    started: Instant,
    returned: bool,
    stall_warned: bool,
}

struct WorkerQueue {
    jobs: VecDeque<Arc<JobEntry>>,
    stopping: bool,
}

struct WorkerShared {
    incarnation: ProcessIncarnation,
    origin: ProcessOrigin,
    clock: Arc<dyn MonotonicClock + Send + Sync>,
    queue: Mutex<WorkerQueue>,
    wake: Condvar,
    active: Mutex<Option<ActiveStep>>,
    health: Mutex<ProcessConditions>,
    capacity: Mutex<CapacityLedger>,
    registry: Mutex<HashMap<RequestId, Weak<JobEntry>>>,
    publications: Arc<WorkPublications>,
    notifier: Arc<ServiceNotifier>,
    slice_overruns: AtomicU64,
    progress_drops: AtomicU64,
    cancellation_noops: AtomicU64,
    shutdown_requested: AtomicBool,
    unavailable: AtomicBool,
    exited: AtomicBool,
}

impl WorkerShared {
    fn wake_worker(&self) {
        self.wake.notify_one();
    }

    fn publish_terminal(&self, entry: &JobEntry) {
        let _ = self
            .publications
            .terminal()
            .publish(Some(entry.identity), &*self.notifier);
    }

    fn publish_progress(&self, progress: WorkProgress) {
        let _ = self
            .publications
            .progress()
            .publish(Some(progress), &*self.notifier);
    }

    fn publish_capacity(&self) {
        let snapshot = self.capacity.lock().snapshot();
        let _ = self
            .publications
            .capacity()
            .publish(snapshot, &*self.notifier);
    }

    fn release_once(&self, entry: &JobEntry) {
        if entry
            .released
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            self.capacity
                .lock()
                .release(entry.charge, entry.family.policy().bytes());
            self.publish_capacity();
            self.registry.lock().remove(&entry.identity.job());
        }
    }
}

#[derive(Default)]
struct ServiceNotifier {
    epoch: AtomicU64,
    gate: Mutex<()>,
    changed: Condvar,
    #[cfg(test)]
    wait_hook: Mutex<Option<std::sync::mpsc::Sender<()>>>,
}

impl ServiceNotifier {
    fn wait_for_change(&self, observed: u64, timeout: Duration) -> bool {
        let mut gate = self.gate.lock();
        if self.epoch.load(Ordering::Acquire) == observed {
            #[cfg(test)]
            if let Some(wait_hook) = self.wait_hook.lock().take() {
                let _ = wait_hook.send(());
            }
            self.changed.wait_for(&mut gate, timeout);
        }
        self.epoch.load(Ordering::Acquire) != observed
    }

    #[cfg(test)]
    fn hook_next_wait(&self, hook: std::sync::mpsc::Sender<()>) {
        *self.wait_hook.lock() = Some(hook);
    }

    fn signal_change(&self) {
        let _gate = self.gate.lock();
        self.epoch.fetch_add(1, Ordering::Release);
        self.changed.notify_all();
    }
}

impl Notifier for ServiceNotifier {
    fn notify(&self, _notice: api::identity::Notice) {
        self.signal_change();
    }
}

struct EntryControl {
    state: JobState,
    stranded: api::process_condition::StrandedCustody,
    abandoned: bool,
    cancellation_claimed: bool,
    progress_sequence: u64,
    progress_dropped: bool,
    qualified_unit: Option<QualifiedUnit>,
    qualified_dispatches: usize,
}

struct JobEntry {
    identity: WorkIdentity,
    spec: TypeId,
    family: WorkFamily,
    revision: api::work::InputRevision,
    charge: WorkCharge,
    job: Mutex<Option<Box<dyn ErasedJob>>>,
    control: Mutex<EntryControl>,
    cancellation: Arc<SharedCancellation>,
    terminal: TerminalCell,
    progress: ProgressHistory,
    settlement: fn(ServiceTerminal) -> Erased,
    map_cancellation: fn(Cancellation) -> Cancellation,
    returned: AtomicBool,
    released: AtomicBool,
    request_active: Mutex<Option<ActiveStep>>,
}

impl JobEntry {
    fn active_step<'a>(&'a self, shared: &'a WorkerShared) -> &'a Mutex<Option<ActiveStep>> {
        match self.family.policy().execution() {
            WorkExecution::BackgroundWorker => &shared.active,
            WorkExecution::RequestThread => &self.request_active,
        }
    }

    fn matches<F: WorkFamilySpec>(&self, ticket: &WorkTicket<F>) -> bool {
        self.identity == ticket.identity()
            && self.spec == TypeId::of::<F>()
            && self.revision == ticket.revision()
    }

    fn slice_allowance(&self) -> (Duration, Option<QualifiedUnit>) {
        let mut control = self.control.lock();
        let phase = self.progress.latest().map(|progress| progress.phase());
        match control.qualified_unit {
            Some(
                unit @ (QualifiedUnit::GatheringFamilyDecode
                | QualifiedUnit::QuesterProgressCompile),
            ) if phase == Some(PreparationPhase::Load.index())
                && control.qualified_dispatches == 0 =>
            {
                control.qualified_dispatches += 1;
                control.qualified_unit = None;
                let allowance = match unit {
                    QualifiedUnit::GatheringFamilyDecode => GATHERING_DECODE_SLICE,
                    QualifiedUnit::QuesterProgressCompile => QUESTER_PROGRESS_SLICE,
                    QualifiedUnit::QuesterRegistryAction => unreachable!("one-shot unit"),
                };
                (allowance, Some(unit))
            }
            Some(QualifiedUnit::QuesterRegistryAction)
                if matches!(phase, Some(phase) if phase == PreparationPhase::Load.index()
                    || phase == PreparationPhase::Prepare.index())
                    && control.qualified_dispatches < MAX_QUALIFIED_DISPATCHES_PER_JOB =>
            {
                control.qualified_dispatches += 1;
                (
                    QUESTER_REGISTRY_SLICE,
                    Some(QualifiedUnit::QuesterRegistryAction),
                )
            }
            _ => (WORK_SLICE, None),
        }
    }

    fn terminal(&self) -> Option<&Erased> {
        self.terminal.borrow()
    }

    fn publish_service_terminal(&self, terminal: ServiceTerminal) -> bool {
        let settlement = (self.settlement)(terminal);
        self.terminal.publish(settlement)
    }
}

/// A one-shot terminal cell. Once published, the worker never touches the
/// payload again. `WorkService::consume` has exclusive access to the service
/// and moves the payload out; readers only borrow while it remains published.
struct TerminalCell {
    value: UnsafeCell<Option<Erased>>,
    state: AtomicU8,
}

impl TerminalCell {
    const fn new() -> Self {
        Self {
            value: UnsafeCell::new(None),
            state: AtomicU8::new(TERMINAL_PENDING),
        }
    }

    fn publish(&self, value: Erased) -> bool {
        if self.state.load(Ordering::Acquire) != TERMINAL_PENDING {
            return false;
        }
        // SAFETY: every publisher holds the owning JobEntry's `control` mutex
        // and checks/sets the atomic state; this is the only write before the
        // Release store below. After that store, no worker reads or writes the
        // payload. A consumer requires `&mut WorkService`, so safe Rust cannot
        // overlap it with a borrow returned by `borrow_terminal`; it sets the
        // state to CONSUMED before dropping the service entry.
        unsafe { *self.value.get() = Some(value) };
        self.state.store(TERMINAL_PUBLISHED, Ordering::Release);
        true
    }

    fn borrow(&self) -> Option<&Erased> {
        if self.state.load(Ordering::Acquire) != TERMINAL_PUBLISHED {
            return None;
        }
        // SAFETY: the Release/Acquire pair makes the initialized immutable
        // payload visible. A published value is not changed by the worker;
        // only exclusive `WorkService::consume` may take it, which cannot
        // overlap this borrow under the frozen API's `&self`/`&mut self`
        // split and the composition mutex.
        unsafe { (&*self.value.get()).as_ref() }
    }

    fn is_pending(&self) -> bool {
        self.state.load(Ordering::Acquire) == TERMINAL_PENDING
    }

    fn is_consumed(&self) -> bool {
        self.state.load(Ordering::Acquire) == TERMINAL_CONSUMED
    }

    fn take(&self) -> Result<Erased, LedgerError> {
        match self.state.load(Ordering::Acquire) {
            TERMINAL_PENDING => return Err(LedgerError::NotPublished),
            TERMINAL_CONSUMED => return Err(LedgerError::AlreadyConsumed),
            TERMINAL_PUBLISHED => {}
            _ => unreachable!("terminal state is a closed three-value enum"),
        }
        // SAFETY: callers hold `&mut WorkService`, which excludes all API
        // borrows of this service. The worker treats the payload as immutable
        // after publication and only checks the atomic state. `JobEntry`'s
        // control mutex serialized this cell's one publication against every
        // competing publisher.
        let value =
            unsafe { (&mut *self.value.get()).take() }.expect("published terminal has a payload");
        self.state.store(TERMINAL_CONSUMED, Ordering::Release);
        Ok(value)
    }
}

// SAFETY: `TerminalCell`'s value is initialized once under JobEntry::control,
// published with Release/Acquire, and thereafter immutable except for the
// exclusive consumer operation documented above. The contained type is Send.
unsafe impl Sync for TerminalCell {}

type ProgressBlock = [OnceLock<WorkProgress>; PROGRESS_CHUNK];

struct ProgressHistory {
    first: ProgressBlock,
    overflow: Mutex<Vec<Box<ProgressBlock>>>,
    len: AtomicUsize,
    writer: Mutex<()>,
}

impl ProgressHistory {
    fn new() -> Self {
        Self {
            first: std::array::from_fn(|_| OnceLock::new()),
            overflow: Mutex::new(Vec::new()),
            len: AtomicUsize::new(0),
            writer: Mutex::new(()),
        }
    }

    fn append(&self, progress: WorkProgress) -> bool {
        let _writer = self.writer.lock();
        let len = self.len.load(Ordering::Relaxed);
        if len >= MAX_PROGRESS_SNAPSHOTS_PER_JOB {
            return false;
        }
        if len < PROGRESS_CHUNK {
            self.first[len].set(progress).expect("one progress writer");
        } else {
            let index = len - PROGRESS_CHUNK;
            let mut overflow = self.overflow.lock();
            if index / PROGRESS_CHUNK == overflow.len() {
                overflow.push(Box::new(std::array::from_fn(|_| OnceLock::new())));
            }
            overflow[index / PROGRESS_CHUNK][index % PROGRESS_CHUNK]
                .set(progress)
                .expect("one progress writer");
        }
        self.len.store(len + 1, Ordering::Release);
        true
    }

    fn latest(&self) -> Option<&WorkProgress> {
        let len = self.len.load(Ordering::Acquire).checked_sub(1)?;
        if len < PROGRESS_CHUNK {
            return self.first[len].get();
        }
        let index = len - PROGRESS_CHUNK;
        let overflow = self.overflow.lock();
        let value = overflow[index / PROGRESS_CHUNK][index % PROGRESS_CHUNK]
            .get()
            .expect("published length follows immutable slot");
        // SAFETY: the OnceLock lives in a heap block that never moves or
        // leaves this ProgressHistory. Its value never changes once set.
        // The WorkAdmission borrow cannot outlive its owning service/job.
        Some(unsafe { &*(value as *const WorkProgress) })
    }
}

enum ErasedStep {
    Yielded,
    Completed(Erased),
    Failed(Erased),
}

struct SliceReport {
    continued: bool,
    exceeded: bool,
    progress: Option<ProgressReport>,
}

trait ErasedJob: Send {
    fn step(
        &mut self,
        clock: &dyn MonotonicClock,
        budget: SliceBudget,
        cancellation: &dyn api::work::CancellationSource,
    ) -> (ErasedStep, SliceReport);
    fn retained(&self) -> RetainedState;
}

struct TypedJob<F, J> {
    job: J,
    family: std::marker::PhantomData<fn() -> F>,
}

impl<F, J> ErasedJob for TypedJob<F, J>
where
    F: WorkFamilySpec,
    J: CooperativeJob<Output = F::Output, Error = F::Error, Phase = F::Phase>,
{
    fn step(
        &mut self,
        clock: &dyn MonotonicClock,
        budget: SliceBudget,
        cancellation: &dyn api::work::CancellationSource,
    ) -> (ErasedStep, SliceReport) {
        let mut context = StepContext::<F::Phase>::new(clock, budget, cancellation);
        let outcome = match self.job.step(&mut context) {
            StepOutcome::Yielded => ErasedStep::Yielded,
            StepOutcome::Completed(value) => {
                ErasedStep::Completed(Box::new(
                    Settlement::<F::Output, JobError<F::Error>>::Completed(value),
                ))
            }
            StepOutcome::Failed(error) => {
                ErasedStep::Failed(Box::new(
                    Settlement::<F::Output, JobError<F::Error>>::Failed(JobError::Family(error)),
                ))
            }
        };
        let report = SliceReport {
            continued: context.continued_after_yield(),
            exceeded: context.exceeded_allowance(),
            progress: context.reported_progress(),
        };
        (outcome, report)
    }

    fn retained(&self) -> RetainedState {
        self.job.retained()
    }
}

enum ServiceTerminal {
    Cancelled(Cancellation),
    Cooperation(CooperativeViolation),
    FailedInternally,
    Unavailable(api::process_condition::UnavailableTerminal),
}

fn erase_service_terminal<F: WorkFamilySpec>(terminal: ServiceTerminal) -> Erased {
    let settlement: JobSettlement<F> = match terminal {
        ServiceTerminal::Cancelled(reason) => Settlement::Cancelled(reason),
        ServiceTerminal::Cooperation(violation) => {
            Settlement::Failed(JobError::Cooperation(violation))
        }
        ServiceTerminal::FailedInternally => Settlement::Failed(JobError::FailedInternally),
        ServiceTerminal::Unavailable(unavailable) => {
            Settlement::Failed(JobError::Unavailable(unavailable))
        }
    };
    Box::new(settlement)
}

struct WorkerExit<'a>(&'a WorkerShared);

impl Drop for WorkerExit<'_> {
    fn drop(&mut self) {
        self.0.exited.store(true, Ordering::Release);
        self.0.notifier.signal_change();
    }
}

fn worker_thread(shared: Arc<WorkerShared>) {
    let _exit = WorkerExit(&shared);
    let result = catch_unwind(AssertUnwindSafe(|| worker_loop(&shared)));
    if result.is_err() {
        let now = shared.clock.now();
        latch_outage(&shared, ConditionCause::Panicked, now, None);
        if let Some(active) = *shared.active.lock() {
            if let Some(entry) = shared
                .registry
                .lock()
                .get(&active.entry)
                .and_then(Weak::upgrade)
            {
                mark_returned(&shared, &entry);
            }
        }
    }
}

fn request_thread(shared: Arc<WorkerShared>, entry: Arc<JobEntry>) {
    let result = catch_unwind(AssertUnwindSafe(|| {
        while !entry.returned.load(Ordering::Acquire) {
            run_entry(&shared, &entry);
        }
    }));
    if result.is_err() {
        let job = entry.job.lock().take();
        let _ = catch_unwind(AssertUnwindSafe(|| drop(job)));
        let published = {
            let control = entry.control.lock();
            if let Some(reason) = control.stranded.cancellation() {
                entry.publish_service_terminal(ServiceTerminal::Cancelled(reason))
            } else {
                entry.publish_service_terminal(ServiceTerminal::FailedInternally)
            }
        };
        if published {
            shared.publish_terminal(&entry);
        }
        api::host_log!(
            api::hostlog::Category::Lifecycle,
            api::hostlog::Level::Error,
            "request work job {:?} failed internally (contained panic); only this job failed",
            entry.identity.job(),
        );
        *entry.request_active.lock() = None;
        mark_returned(&shared, &entry);
    }
    shared.notifier.signal_change();
}

fn worker_loop(shared: &Arc<WorkerShared>) {
    loop {
        let entry = {
            let mut queue = shared.queue.lock();
            while queue.jobs.is_empty() && !queue.stopping {
                shared.wake.wait(&mut queue);
            }
            if queue.jobs.is_empty() && queue.stopping {
                return;
            }
            queue.jobs.pop_front().expect("nonempty queue").clone()
        };
        if shared.unavailable.load(Ordering::Acquire) {
            drop_queued_entry(shared, &entry);
            continue;
        }
        run_entry(shared, &entry);
    }
}

fn executor_log_label(entry: &JobEntry) -> &'static str {
    match entry.family.policy().execution() {
        WorkExecution::BackgroundWorker => "background work",
        WorkExecution::RequestThread => "request work",
    }
}

fn run_entry(shared: &Arc<WorkerShared>, entry: &Arc<JobEntry>) {
    if !entry.terminal.is_pending() {
        drop_queued_entry(shared, entry);
        return;
    }
    let mut job = match entry.job.lock().take() {
        Some(job) => job,
        None => {
            mark_returned(shared, entry);
            return;
        }
    };
    let mut published = false;
    let skip_step = {
        let mut control = entry.control.lock();
        if shared.shutdown_requested.load(Ordering::Acquire) {
            let _ = request_cancel_locked(entry, &mut control, Cancellation::Shutdown);
        }
        if !entry.terminal.is_pending() {
            true
        } else if let Some(reason) = control.stranded.cancellation() {
            published = entry.publish_service_terminal(ServiceTerminal::Cancelled(reason));
            true
        } else {
            control.state = JobState::Executing;
            false
        }
    };
    if skip_step {
        if published {
            shared.publish_terminal(entry);
        }
        drop(job);
        mark_returned(shared, entry);
        return;
    }
    let started = shared.clock.now();
    *entry.active_step(shared).lock() = Some(ActiveStep {
        entry: entry.identity.job(),
        started,
        returned: false,
        stall_warned: false,
    });
    let (allowance, qualified) = entry.slice_allowance();
    let budget = SliceBudget::new(
        NonZeroU32::new(WORK_UNITS_PER_SLICE).expect("nonzero fixed quantum"),
        Deadline::after(started, allowance),
    );
    let step = catch_unwind(AssertUnwindSafe(|| {
        job.step(&*shared.clock, budget, &*entry.cancellation)
    }));
    if let Some(active) = entry.active_step(shared).lock().as_mut() {
        if active.entry == entry.identity.job() {
            active.returned = true;
        }
    }
    let returned_at = shared.clock.now();
    match step {
        Err(_) => {
            // A returned job panic is a failed job, not a lost worker.
            // Drop of its captured state is also inside the job boundary.
            let _ = catch_unwind(AssertUnwindSafe(|| drop(job)));
            api::host_log!(
                api::hostlog::Category::Lifecycle,
                api::hostlog::Level::Error,
                "{} job {:?} failed internally (contained panic); only this job failed",
                executor_log_label(entry),
                entry.identity.job(),
            );
            let published = entry.publish_service_terminal(ServiceTerminal::FailedInternally);
            if published {
                shared.publish_terminal(entry);
            }
            *entry.active_step(shared).lock() = None;
            mark_returned(shared, entry);
        }
        Ok((outcome, report)) => {
            process_step(
                shared,
                entry,
                job,
                outcome,
                report,
                StepTiming {
                    started,
                    returned_at,
                    allowance,
                    qualified,
                },
            );
            *entry.active_step(shared).lock() = None;
        }
    }
}

struct StepTiming {
    started: Instant,
    returned_at: Instant,
    allowance: Duration,
    qualified: Option<QualifiedUnit>,
}

fn process_step(
    shared: &Arc<WorkerShared>,
    entry: &Arc<JobEntry>,
    job: Box<dyn ErasedJob>,
    outcome: ErasedStep,
    report: SliceReport,
    timing: StepTiming,
) {
    let elapsed = timing.returned_at.saturating_duration_since(timing.started);
    let slow_step = elapsed > SLOW_STEP_WARNING_LIMIT;
    if slow_step {
        api::host_log!(
            api::hostlog::Category::Lifecycle,
            api::hostlog::Level::Warn,
            "{} step returned slowly family={:?} elapsed={elapsed:?}",
            executor_log_label(entry),
            entry.family,
        );
    }
    if elapsed >= timing.allowance {
        let count = shared.slice_overruns.fetch_add(1, Ordering::AcqRel) + 1;
        // Exponential reporting bounds log work if a busy host has many
        // delayed steps, while the exact count remains observable.
        // The slow-step warning already reports this step's delay. Suppress
        // the overrun warning to avoid two warnings for the same step.
        if count.is_power_of_two() && !slow_step {
            api::host_log!(
                api::hostlog::Category::Lifecycle,
                api::hostlog::Level::Warn,
                "{} slice overrun count={count} family={:?} elapsed={elapsed:?} allowance={:?}",
                executor_log_label(entry),
                entry.family,
                timing.allowance,
            );
        }
    }
    let mut violation = if report.continued {
        Some(CooperativeViolation::IgnoredYield)
    } else if report.exceeded {
        Some(CooperativeViolation::UnboundedSlice)
    } else {
        None
    };
    let mut published = false;
    let mut yielded = false;
    {
        let mut control = entry.control.lock();
        if control.qualified_unit == Some(QualifiedUnit::QuesterRegistryAction)
            && control.qualified_dispatches == MAX_QUALIFIED_DISPATCHES_PER_JOB
            && timing.qualified.is_none()
            && (report.progress.is_some_and(|progress| {
                matches!(
                    progress.phase(),
                    phase if phase == PreparationPhase::Load.index()
                        || phase == PreparationPhase::Prepare.index()
                )
            }) || matches!(&outcome, ErasedStep::Completed(_) | ErasedStep::Failed(_)))
        {
            // A returned registry action after the scheduler's finite
            // qualified-unit allowance exceeds the slice allowance; progress
            // totals cannot renew it. ChargeExceeded is reserved for bytes.
            violation = Some(CooperativeViolation::UnboundedSlice);
        }
        if shared.shutdown_requested.load(Ordering::Acquire) {
            let _ = request_cancel_locked(entry, &mut control, Cancellation::Shutdown);
        }
        if entry.terminal.is_pending() {
            if let Some(report) = report.progress {
                if control.qualified_unit == Some(QualifiedUnit::QuesterRegistryAction)
                    && report.phase() == PreparationPhase::Prepare.index()
                    && report.total() == Some(report.completed())
                {
                    // The cooperative Quester adapter publishes a known total
                    // only when its final Prepare action has completed. Seal
                    // must return to the ordinary allowance.
                    control.qualified_unit = None;
                }
                if let Some(sequence) = control.progress_sequence.checked_add(1) {
                    let progress = WorkProgress::new(
                        entry.identity,
                        sequence,
                        report.phase(),
                        report.completed(),
                        report.total(),
                    );
                    if entry.progress.append(progress.clone()) {
                        control.progress_sequence = sequence;
                        shared.publish_progress(progress);
                    } else {
                        shared.progress_drops.fetch_add(1, Ordering::AcqRel);
                        if !control.progress_dropped {
                            control.progress_dropped = true;
                            api::host_log!(
                                api::hostlog::Category::Lifecycle,
                                api::hostlog::Level::Warn,
                                "{} job {:?} dropped progress after immutable snapshot custody filled",
                                executor_log_label(entry),
                                entry.identity.job(),
                            );
                        }
                    }
                }
            }
        }
        if !entry.terminal.is_pending() {
            // A health observer may already have selected the unavailable
            // terminal. The returned step cannot rewrite it.
        } else if let Some(reason) = control.stranded.cancellation() {
            published = entry.publish_service_terminal(ServiceTerminal::Cancelled(reason));
        } else if let Some(violation) = violation {
            published = entry.publish_service_terminal(ServiceTerminal::Cooperation(violation));
        } else {
            match outcome {
                ErasedStep::Completed(settlement) | ErasedStep::Failed(settlement) => {
                    published = entry.terminal.publish(settlement);
                }
                ErasedStep::Yielded => {
                    let retained = job.retained();
                    let violation = if let Some(forbidden) = retained.forbidden_item() {
                        Some(CooperativeViolation::ForbiddenRetain(forbidden))
                    } else if entry
                        .charge
                        .total_bytes()
                        .is_none_or(|reserved| retained.bytes() > reserved)
                    {
                        Some(CooperativeViolation::ChargeExceeded)
                    } else {
                        None
                    };
                    if let Some(violation) = violation {
                        published =
                            entry.publish_service_terminal(ServiceTerminal::Cooperation(violation));
                    } else {
                        control.state = JobState::Yielded;
                        yielded = true;
                    }
                }
            }
        }
    }
    if published {
        shared.publish_terminal(entry);
    }
    if yielded {
        *entry.job.lock() = Some(job);
        let control = entry.control.lock();
        if control.stranded.cancellation().is_none() {
            if entry.family.policy().execution() == WorkExecution::RequestThread {
                return;
            }
            let mut queue = shared.queue.lock();
            if !queue.stopping && !shared.unavailable.load(Ordering::Acquire) {
                queue.jobs.push_back(Arc::clone(entry));
                shared.wake.notify_one();
                return;
            }
        }
        drop(control);
        drop_queued_entry(shared, entry);
        return;
    }
    drop(job);
    mark_returned(shared, entry);
}

fn drop_queued_entry(shared: &Arc<WorkerShared>, entry: &Arc<JobEntry>) {
    drop(entry.job.lock().take());
    let mut published = false;
    {
        let control = entry.control.lock();
        if entry.terminal.is_pending() {
            if let Some(reason) = control.stranded.cancellation() {
                published = entry.publish_service_terminal(ServiceTerminal::Cancelled(reason));
            } else if let Some(condition) = shared
                .health
                .lock()
                .latched(ProcessServiceCode::WorkAdmissionUnavailable)
                .map(|condition| condition.id())
            {
                let unavailable = api::process_condition::UnavailableTerminal::new(
                    condition,
                    entry.identity.job(),
                    entry.identity.scope(),
                );
                published =
                    entry
                        .terminal
                        .publish((entry.settlement)(ServiceTerminal::Unavailable(
                            unavailable,
                        )));
            } else if control.abandoned {
                published = entry
                    .publish_service_terminal(ServiceTerminal::Cancelled(Cancellation::Shutdown));
            }
        }
    }
    if published {
        shared.publish_terminal(entry);
    }
    mark_returned(shared, entry);
}

fn mark_returned(shared: &Arc<WorkerShared>, entry: &JobEntry) {
    let mut published = false;
    {
        let mut control = entry.control.lock();
        if entry.returned.swap(true, Ordering::AcqRel) {
            return;
        }
        control.stranded.worker_returned();
        if control.abandoned && entry.terminal.is_pending() {
            let reason = control
                .stranded
                .cancellation()
                .unwrap_or(Cancellation::Shutdown);
            published = entry.publish_service_terminal(ServiceTerminal::Cancelled(reason));
        }
    }
    if published {
        shared.publish_terminal(entry);
    }
    maybe_release(shared, entry);
}

fn maybe_release(shared: &Arc<WorkerShared>, entry: &JobEntry) {
    if !entry.returned.load(Ordering::Acquire) {
        return;
    }
    let abandoned = entry.control.lock().abandoned;
    if entry.terminal.is_consumed() || abandoned {
        shared.release_once(entry);
    }
}

fn request_cancel_locked(
    entry: &JobEntry,
    control: &mut EntryControl,
    reason: Cancellation,
) -> CancelOutcome {
    let reason = (entry.map_cancellation)(reason);
    if !entry.terminal.is_pending() {
        return CancelOutcome::AlreadyTerminal;
    }
    match control.stranded.cancel(reason) {
        Ok(()) => {
            entry
                .cancellation
                .register(reason)
                .expect("the live source mirrors the first custody selection");
            CancelOutcome::Requested
        }
        Err(api::process_condition::CustodyError::AlreadyCancelled(_)) => CancelOutcome::Requested,
        Err(api::process_condition::CustodyError::AlreadyFailed)
        | Err(api::process_condition::CustodyError::WorkerStillCharged(_)) => {
            CancelOutcome::AlreadyTerminal
        }
    }
}

fn request_cancel(
    shared: &Arc<WorkerShared>,
    entry: &Arc<JobEntry>,
    reason: Cancellation,
) -> CancelOutcome {
    let outcome = {
        let mut control = entry.control.lock();
        request_cancel_locked(entry, &mut control, reason)
    };
    if outcome == CancelOutcome::Requested {
        shared.wake_worker();
    }
    outcome
}

fn latch_outage(
    shared: &Arc<WorkerShared>,
    cause: ConditionCause,
    now: Instant,
    stalled_elapsed: Option<Duration>,
) {
    let condition_id = ConditionId::new(
        shared.incarnation,
        ProcessServiceCode::WorkAdmissionUnavailable,
    );
    let (queued, newly_latched) = {
        let mut queue = shared.queue.lock();
        let newly_latched = {
            let mut health = shared.health.lock();
            if health
                .latched(ProcessServiceCode::WorkAdmissionUnavailable)
                .is_none()
            {
                let condition = ProcessServiceCondition::new(
                    condition_id,
                    cause,
                    shared.origin.elapsed(now),
                    None,
                );
                health.publish(condition).is_ok()
            } else {
                false
            }
        };
        shared.unavailable.store(true, Ordering::Release);
        queue.stopping = true;
        (queue.jobs.drain(..).collect::<Vec<_>>(), newly_latched)
    };
    if newly_latched {
        if let Some(elapsed) = stalled_elapsed {
            api::host_log!(
                api::hostlog::Category::Lifecycle,
                api::hostlog::Level::Warn,
                "background work worker stalled after {elapsed:?}. The service is unavailable.",
            );
        }
    }
    let condition = shared
        .health
        .lock()
        .latched(ProcessServiceCode::WorkAdmissionUnavailable)
        .map(|condition| condition.id())
        .unwrap_or(condition_id);
    let entries: Vec<_> = shared
        .registry
        .lock()
        .values()
        .filter_map(Weak::upgrade)
        .collect();
    for entry in entries {
        if entry.family.policy().execution() == WorkExecution::RequestThread {
            continue;
        }
        let mut published = false;
        {
            let mut control = entry.control.lock();
            if entry.terminal.is_pending() {
                if let Some(reason) = control.stranded.cancellation() {
                    published = entry.publish_service_terminal(ServiceTerminal::Cancelled(reason));
                } else {
                    let terminal = api::process_condition::UnavailableTerminal::new(
                        condition,
                        entry.identity.job(),
                        entry.identity.scope(),
                    );
                    if control.stranded.fail_unavailable(condition).is_ok() {
                        published = entry
                            .terminal
                            .publish((entry.settlement)(ServiceTerminal::Unavailable(terminal)));
                    }
                }
            }
        }
        if published {
            shared.publish_terminal(&entry);
        }
    }
    for entry in queued {
        drop_queued_entry(shared, &entry);
    }
    shared.wake.notify_all();
}

impl WorkService {
    /// Admit a declared family unit without changing the F0 WorkAdmission API
    /// or the ordinary quantum of unrelated jobs.
    pub(crate) fn try_submit_qualified<F, J>(
        &mut self,
        identity: WorkIdentity,
        input: &F::Input,
        job: J,
        conditions: &ProcessConditions,
        unit: QualifiedUnit,
    ) -> Result<WorkTicket<F>, api::work::Refused<J>>
    where
        F: WorkFamilySpec,
        J: CooperativeJob<Output = F::Output, Error = F::Error, Phase = F::Phase>,
    {
        self.try_submit_inner(identity, input, job, conditions, Some(unit))
    }

    fn try_submit_inner<F, J>(
        &mut self,
        identity: WorkIdentity,
        input: &F::Input,
        job: J,
        conditions: &ProcessConditions,
        qualified_unit: Option<QualifiedUnit>,
    ) -> Result<WorkTicket<F>, api::work::Refused<J>>
    where
        F: WorkFamilySpec,
        J: CooperativeJob<Output = F::Output, Error = F::Error, Phase = F::Phase>,
    {
        self.try_submit_with_spawn(
            identity,
            input,
            job,
            conditions,
            qualified_unit,
            |builder, shared, entry| builder.spawn(move || request_thread(shared, entry)),
        )
    }

    fn try_submit_with_spawn<F, J>(
        &mut self,
        identity: WorkIdentity,
        input: &F::Input,
        job: J,
        conditions: &ProcessConditions,
        qualified_unit: Option<QualifiedUnit>,
        spawn: impl FnOnce(
            thread::Builder,
            Arc<WorkerShared>,
            Arc<JobEntry>,
        ) -> std::io::Result<JoinHandle<()>>,
    ) -> Result<WorkTicket<F>, api::work::Refused<J>>
    where
        F: WorkFamilySpec,
        J: CooperativeJob<Output = F::Output, Error = F::Error, Phase = F::Phase>,
    {
        if qualified_unit.is_some_and(|unit| {
            !matches!(
                (unit, F::FAMILY),
                (
                    QualifiedUnit::GatheringFamilyDecode | QualifiedUnit::QuesterProgressCompile,
                    WorkFamily::ProfilePreparation
                ) | (
                    QualifiedUnit::QuesterRegistryAction,
                    WorkFamily::CataloguePreparation | WorkFamily::ProfilePreparation
                )
            )
        }) {
            return Err(api::work::Refused::new(
                SubmissionRefusal::InvalidArgument,
                job,
            ));
        }
        self.observe_health();
        self.cleanup_released();
        let background = F::FAMILY.policy().execution() == WorkExecution::BackgroundWorker;
        let refusal = if identity.incarnation() != self.incarnation
            || conditions.incarnation() != self.incarnation
        {
            Some(SubmissionRefusal::InvalidAuthority)
        } else if background {
            self.refusal_for_unavailable(identity)
        } else {
            None
        };
        let refusal = if let Some(refusal) = refusal {
            Some(refusal)
        } else if let Err(AdmissionRefusal::Unavailable(refusal)) = conditions.admit(
            ProcessCapability::BackgroundWork,
            Some(identity.job()),
            identity.scope(),
        ) {
            Some(SubmissionRefusal::Unavailable(refusal))
        } else if self.stopping {
            Some(SubmissionRefusal::Stopping)
        } else if identity.job().namespace() != api::identity::RequestNamespace::Host
            || self.jobs.contains_key(&identity.job())
            || self.last_job.is_some_and(|last| identity.job() <= last)
        {
            Some(SubmissionRefusal::InvalidArgument)
        } else {
            None
        };
        if let Some(refusal) = refusal {
            return Err(api::work::Refused::new(refusal, job));
        }
        let charge = F::charge(input);
        let revision = F::revision(input);
        let policy = F::FAMILY.policy();
        let builder = match policy.execution() {
            WorkExecution::BackgroundWorker => None,
            WorkExecution::RequestThread => {
                let Some(settings) = job.request_thread() else {
                    return Err(api::work::Refused::new(
                        SubmissionRefusal::InvalidArgument,
                        job,
                    ));
                };
                let mut builder = thread::Builder::new().name(settings.name().to_owned());
                if let Some(stack_bytes) = settings.stack_bytes() {
                    builder = builder.stack_size(stack_bytes.get());
                }
                Some(builder)
            }
        };
        let mut queue = self.shared.queue.lock();
        if background {
            if let Some(unavailable) = self.refusal_for_unavailable(identity) {
                return Err(api::work::Refused::new(unavailable, job));
            }
        }
        if self.stopping || (background && queue.stopping) {
            return Err(api::work::Refused::new(SubmissionRefusal::Stopping, job));
        }
        if let Err(refusal) =
            self.shared
                .capacity
                .lock()
                .reserve(self.bounds, charge, policy.bytes())
        {
            return Err(api::work::Refused::new(refusal, job));
        }
        let entry = Arc::new(JobEntry {
            identity,
            spec: TypeId::of::<F>(),
            family: F::FAMILY,
            revision,
            charge,
            job: Mutex::new(None),
            control: Mutex::new(EntryControl {
                state: JobState::Queued,
                stranded: api::process_condition::StrandedCustody::new(
                    identity.job(),
                    identity.scope(),
                ),
                abandoned: false,
                cancellation_claimed: false,
                progress_sequence: 0,
                qualified_unit,
                qualified_dispatches: 0,
                progress_dropped: false,
            }),
            cancellation: Arc::new(SharedCancellation::new()),
            terminal: TerminalCell::new(),
            progress: ProgressHistory::new(),
            settlement: erase_service_terminal::<F>,
            map_cancellation: F::map_cancellation,
            returned: AtomicBool::new(false),
            released: AtomicBool::new(false),
            request_active: Mutex::new(None),
        });
        // A new request thread cannot obtain the job until registration is
        // complete. The original J stays with this caller if spawn fails.
        let mut start = entry.job.lock();
        if let Some(builder) = builder {
            match spawn(builder, Arc::clone(&self.shared), Arc::clone(&entry)) {
                Ok(thread) => {
                    self.request_threads.insert(identity.job(), thread);
                }
                Err(error) => {
                    self.shared.capacity.lock().release(charge, policy.bytes());
                    self.publish_capacity();
                    api::host_log!(
                        api::hostlog::Category::Lifecycle,
                        api::hostlog::Level::Warn,
                        "request work thread start failed for job {:?}: {error}",
                        identity.job(),
                    );
                    return Err(api::work::Refused::new(
                        SubmissionRefusal::ThreadStartFailed,
                        job,
                    ));
                }
            }
        }
        *start = Some(Box::new(TypedJob::<F, J> {
            job,
            family: std::marker::PhantomData,
        }));
        self.jobs.insert(identity.job(), Arc::clone(&entry));
        self.last_job = Some(identity.job());
        self.shared
            .registry
            .lock()
            .insert(identity.job(), Arc::downgrade(&entry));
        if policy.execution() == WorkExecution::BackgroundWorker {
            queue.jobs.push_back(Arc::clone(&entry));
        }
        self.publish_capacity();
        drop(queue);
        drop(start);
        self.shared.wake_worker();
        Ok(WorkTicket::new(identity, revision))
    }
}

impl WorkAdmission for WorkService {
    fn try_submit<F, J>(
        &mut self,
        identity: WorkIdentity,
        input: &F::Input,
        job: J,
        conditions: &ProcessConditions,
    ) -> Result<WorkTicket<F>, api::work::Refused<J>>
    where
        F: WorkFamilySpec,
        J: CooperativeJob<Output = F::Output, Error = F::Error, Phase = F::Phase>,
    {
        self.try_submit_inner(identity, input, job, conditions, None)
    }

    fn cancel<F: WorkFamilySpec>(
        &mut self,
        ticket: WorkTicket<F>,
        reason: Cancellation,
    ) -> CancelOutcome {
        self.observe_health();
        let Some(entry) = self
            .jobs
            .get(&ticket.job())
            .filter(|entry| entry.matches(&ticket))
            .cloned()
        else {
            return CancelOutcome::StaleIdentity;
        };
        request_cancel(&self.shared, &entry, reason)
    }

    fn abandon<F: WorkFamilySpec>(
        &mut self,
        ticket: WorkTicket<F>,
        reason: Cancellation,
    ) -> CancelOutcome {
        let outcome = self.cancel(ticket, reason);
        if let Some(entry) = self
            .jobs
            .get(&ticket.job())
            .filter(|entry| entry.matches(&ticket))
            .cloned()
        {
            entry.control.lock().abandoned = true;
            maybe_release(&self.shared, &entry);
        }
        self.cleanup_released();
        outcome
    }

    fn borrow_terminal<F: WorkFamilySpec>(
        &self,
        ticket: &WorkTicket<F>,
    ) -> Result<Option<&JobSettlement<F>>, LedgerError> {
        self.observe_health();
        let entry = self
            .jobs
            .get(&ticket.job())
            .filter(|entry| entry.matches(ticket))
            .ok_or(LedgerError::StaleIdentity)?;
        if entry.terminal.is_consumed() {
            return Err(LedgerError::AlreadyConsumed);
        }
        match entry.terminal() {
            None => Ok(None),
            Some(value) => value
                .downcast_ref::<JobSettlement<F>>()
                .map(Some)
                .ok_or(LedgerError::StaleIdentity),
        }
    }

    fn consume<F: WorkFamilySpec>(
        &mut self,
        ticket: &WorkTicket<F>,
    ) -> Result<JobSettlement<F>, LedgerError> {
        self.observe_health();
        let entry = self
            .jobs
            .get(&ticket.job())
            .filter(|entry| entry.matches(ticket))
            .cloned()
            .ok_or(LedgerError::StaleIdentity)?;
        if entry
            .terminal()
            .is_some_and(|value| !value.is::<JobSettlement<F>>())
        {
            return Err(LedgerError::StaleIdentity);
        }
        let erased = entry.terminal.take()?;
        let settlement = *erased
            .downcast::<JobSettlement<F>>()
            .map_err(|_| LedgerError::StaleIdentity)?;
        maybe_release(&self.shared, &entry);
        self.cleanup_released();
        Ok(settlement)
    }

    fn progress<F: WorkFamilySpec>(&self, ticket: &WorkTicket<F>) -> Option<&WorkProgress> {
        self.observe_health();
        let entry = self
            .jobs
            .get(&ticket.job())
            .filter(|entry| entry.matches(ticket))?;
        entry.progress.latest()
    }

    fn publications(&self) -> &WorkPublications {
        &self.shared.publications
    }
}

impl WorkerWake for WorkService {
    fn wake(&self) {
        self.shared.wake_worker();
    }
}

#[cfg(test)]
#[path = "work_admission_tests.rs"]
mod tests;
