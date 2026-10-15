//! Process-wide cooperative work admission and its single FIFO worker.
//!
//! Ordinary G2b dispatches request a yield after 64 work units or 10 ms.
//! Returning late is reported, not treated as a failed operator action:
//! scheduler delay is not evidence that a job ignored a checkpoint. A
//! non-returning worker is detected separately; explicit unit/checkpoint
//! violations still fail only their job. Admission reserves job, byte,
//! operation-handle, and terminal custody.

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
    WorkCapacity, WorkCharge, WorkFamily, WorkFamilySpec, WorkPhase, WorkProgress,
    WorkPublications, WorkTicket, WorkerWake,
};
use parking_lot::{Condvar, Mutex, MutexGuard};
use rand::RngCore;

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

/// A worker that remains inside a step this long makes the service unavailable.
pub const WORKER_STALL_LIMIT: Duration = Duration::from_secs(5);
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

/// Failure to complete an orderly worker shutdown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum WorkShutdownError {
    /// The worker did not return before the shutdown bound.
    #[error("the WorkAdmission worker did not return before the shutdown bound")]
    WorkerNotReturned,
    /// The worker thread exited by panicking outside a job boundary.
    #[error("the WorkAdmission worker panicked before shutdown acknowledgement")]
    WorkerPanicked,
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
                latch_outage(&shared, ConditionCause::StartFailed, shared.clock.now());
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
            if now.saturating_duration_since(active.started) >= WORKER_STALL_LIMIT {
                latch_outage(&self.shared, ConditionCause::ProgressLost, now);
            }
        }
    }

    /// Wake the sole worker after a command has been published.
    pub fn wake(&self) {
        self.shared.wake_worker();
    }

    /// Stop admission, register `Shutdown` for every accepted job, and wait
    /// for the sole worker thread to return and be joined. Call without
    /// holding the service mutex; it is released during the bounded wait.
    ///
    /// `WorkerNotReturned` is not a retirement acknowledgement: the service,
    /// worker handle, and any active-job credit remain retained. Call again
    /// after the active step returns to join the worker.
    pub fn shutdown(shared: &Arc<parking_lot::Mutex<Self>>) -> Result<(), WorkShutdownError> {
        Self::shutdown_with_timeout(shared, WORKER_STALL_LIMIT)
    }

    fn shutdown_with_timeout(
        shared: &Arc<parking_lot::Mutex<Self>>,
        timeout: Duration,
    ) -> Result<(), WorkShutdownError> {
        let deadline = Instant::now() + timeout;
        {
            let mut service = shared.lock();
            service.begin_shutdown();
        }
        loop {
            let (notifier, epoch, exited, joined, panicked) = {
                let service = shared.lock();
                service.observe_health();
                let epoch = service.publication_epoch();
                (
                    Arc::clone(&service.shared.notifier),
                    epoch,
                    service.shared.exited.load(Ordering::Acquire),
                    service.worker_joined,
                    service.worker_panicked,
                )
            };
            if joined {
                return if panicked {
                    Err(WorkShutdownError::WorkerPanicked)
                } else {
                    Ok(())
                };
            }
            if exited {
                let worker = {
                    let mut service = shared.lock();
                    if service.worker_joined {
                        None
                    } else {
                        service._worker.take()
                    }
                };
                if let Some(worker) = worker {
                    let panicked = worker.join().is_err();
                    let mut service = shared.lock();
                    service.worker_joined = true;
                    service.worker_panicked = panicked;
                    service.shared.notifier.signal_change();
                    return if panicked {
                        Err(WorkShutdownError::WorkerPanicked)
                    } else {
                        Ok(())
                    };
                }
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                let service = shared.lock();
                service.observe_health();
                if service.worker_joined {
                    return if service.worker_panicked {
                        Err(WorkShutdownError::WorkerPanicked)
                    } else {
                        Ok(())
                    };
                }
                if service.shared.exited.load(Ordering::Acquire) && service._worker.is_some() {
                    drop(service);
                    continue;
                }
                return Err(WorkShutdownError::WorkerNotReturned);
            }
            notifier.wait_for_change(epoch, remaining);
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
    /// mutex is released before each condition-variable wait.
    pub fn wait_terminal<F: WorkFamilySpec>(
        shared: &Arc<parking_lot::Mutex<Self>>,
        ticket: &WorkTicket<F>,
    ) -> Result<(), LedgerError> {
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
                    return Err(LedgerError::AlreadyConsumed);
                }
                if !entry.terminal.is_pending() {
                    return Ok(());
                }
                (Arc::clone(&service.shared.notifier), epoch)
            };
            // Recheck health periodically, but never confuse a healthy busy
            // queue with a worker that failed to return from its active step.
            notifier.wait_for_change(epoch, WORKER_STALL_LIMIT);
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
            notifier.wait_for_change(epoch, WORKER_STALL_LIMIT);
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
        if let Some(worker) = self._worker.take() {
            self.worker_panicked = worker.join().is_err();
            self.worker_joined = true;
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
}

impl CapacityLedger {
    fn reserve(
        &mut self,
        bounds: AdmissionBounds,
        charge: WorkCharge,
    ) -> Result<(), SubmissionRefusal> {
        let bytes = charge
            .total_bytes()
            .and_then(|total| total.checked_add(PROGRESS_BYTES_PER_JOB))
            .ok_or(SubmissionRefusal::InvalidArgument)?;
        if bytes > bounds.max_bytes().get()
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
        if next_bytes > bounds.max_bytes().get() {
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
        Ok(())
    }

    fn release(&mut self, charge: WorkCharge) {
        let bytes = charge
            .total_bytes()
            .and_then(|total| total.checked_add(PROGRESS_BYTES_PER_JOB))
            .expect("only representable reserved charges are released");
        self.jobs -= 1;
        self.bytes -= bytes;
        self.handles -= charge.operation_handles();
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
            self.capacity.lock().release(entry.charge);
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
}

impl ServiceNotifier {
    fn wait_for_change(&self, observed: u64, timeout: Duration) {
        let mut gate = self.gate.lock();
        if self.epoch.load(Ordering::Acquire) == observed {
            self.changed.wait_for(&mut gate, timeout);
        }
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
    progress_sequence: u64,
    progress_dropped: bool,
    qualified_unit: Option<QualifiedUnit>,
    qualified_dispatches: usize,
}

struct JobEntry {
    identity: WorkIdentity,
    spec: TypeId,
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
}

impl JobEntry {
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
        latch_outage(&shared, ConditionCause::Panicked, now);
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
        run_entry(shared, entry);
    }
}

fn run_entry(shared: &Arc<WorkerShared>, entry: Arc<JobEntry>) {
    if !entry.terminal.is_pending() {
        drop_queued_entry(shared, &entry);
        return;
    }
    let mut job = match entry.job.lock().take() {
        Some(job) => job,
        None => {
            mark_returned(shared, &entry);
            return;
        }
    };
    let mut published = false;
    let skip_step = {
        let mut control = entry.control.lock();
        if shared.shutdown_requested.load(Ordering::Acquire) {
            let _ = request_cancel_locked(&entry, &mut control, Cancellation::Shutdown);
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
            shared.publish_terminal(&entry);
        }
        drop(job);
        mark_returned(shared, &entry);
        return;
    }
    let started = shared.clock.now();
    *shared.active.lock() = Some(ActiveStep {
        entry: entry.identity.job(),
        started,
        returned: false,
    });
    let (allowance, qualified) = entry.slice_allowance();
    let budget = SliceBudget::new(
        NonZeroU32::new(WORK_UNITS_PER_SLICE).expect("nonzero fixed quantum"),
        Deadline::after(started, allowance),
    );
    let step = catch_unwind(AssertUnwindSafe(|| {
        job.step(&*shared.clock, budget, &*entry.cancellation)
    }));
    if let Some(active) = shared.active.lock().as_mut() {
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
                "background work job {:?} failed internally (contained panic); only this job failed",
                entry.identity.job(),
            );
            let published = entry.publish_service_terminal(ServiceTerminal::FailedInternally);
            if published {
                shared.publish_terminal(&entry);
            }
            *shared.active.lock() = None;
            mark_returned(shared, &entry);
        }
        Ok((outcome, report)) => {
            process_step(
                shared,
                &entry,
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
            *shared.active.lock() = None;
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
    if elapsed >= timing.allowance {
        let count = shared.slice_overruns.fetch_add(1, Ordering::AcqRel) + 1;
        // Exponential reporting bounds log work if a busy host has many
        // delayed steps, while the exact count remains observable.
        if count.is_power_of_two() {
            api::host_log!(
                api::hostlog::Category::Lifecycle,
                api::hostlog::Level::Warn,
                "background work slice overrun count={count} elapsed={elapsed:?} allowance={:?}",
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
                                "background work job {:?} coalesced progress after immutable snapshot custody filled",
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

fn latch_outage(shared: &Arc<WorkerShared>, cause: ConditionCause, now: Instant) {
    let condition_id = ConditionId::new(
        shared.incarnation,
        ProcessServiceCode::WorkAdmissionUnavailable,
    );
    let queued = {
        let mut queue = shared.queue.lock();
        {
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
                let _ = health.publish(condition);
            }
        }
        shared.unavailable.store(true, Ordering::Release);
        queue.stopping = true;
        queue.jobs.drain(..).collect::<Vec<_>>()
    };
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
        let refusal = if identity.incarnation() != self.incarnation
            || conditions.incarnation() != self.incarnation
        {
            Some(SubmissionRefusal::InvalidAuthority)
        } else if let Some(unavailable) = self.refusal_for_unavailable(identity) {
            Some(unavailable)
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
        let mut queue = self.shared.queue.lock();
        if let Some(unavailable) = self.refusal_for_unavailable(identity) {
            return Err(api::work::Refused::new(unavailable, job));
        }
        if queue.stopping || self.stopping {
            return Err(api::work::Refused::new(SubmissionRefusal::Stopping, job));
        }
        if let Err(refusal) = self.shared.capacity.lock().reserve(self.bounds, charge) {
            return Err(api::work::Refused::new(refusal, job));
        }
        let entry = Arc::new(JobEntry {
            identity,
            spec: TypeId::of::<F>(),
            revision,
            charge,
            job: Mutex::new(Some(Box::new(TypedJob::<F, J> {
                job,
                family: std::marker::PhantomData,
            }))),
            control: Mutex::new(EntryControl {
                state: JobState::Queued,
                stranded: api::process_condition::StrandedCustody::new(
                    identity.job(),
                    identity.scope(),
                ),
                abandoned: false,
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
        });
        self.jobs.insert(identity.job(), Arc::clone(&entry));
        self.last_job = Some(identity.job());
        self.shared
            .registry
            .lock()
            .insert(identity.job(), Arc::downgrade(&entry));
        queue.jobs.push_back(Arc::clone(&entry));
        self.publish_capacity();
        drop(queue);
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
