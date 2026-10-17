use super::*;
use api::identity::{Cancellation, ProcessIncarnation, Scope, Settlement};
use api::process_condition::ProcessConditions;
use api::work::{
    Checkpoint, CooperativeJob, CooperativeViolation, ForbiddenRetain, InputRevision, JobError,
    RetainedState, StepContext, StepOutcome, SubmissionRefusal, WorkFamily, WorkFamilySpec,
    WorkProgress, WorkTicket,
};
use std::num::NonZeroU64;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::time::{Duration, Instant};

static NEXT_TEST_NONCE: AtomicU64 = AtomicU64::new(1);

#[derive(Debug)]
struct ManualClock(Mutex<Instant>);

impl ManualClock {
    fn new() -> Self {
        Self(Mutex::new(Instant::now()))
    }

    fn advance(&self, by: Duration) {
        let mut now = self.0.lock();
        *now += by;
    }
}

impl MonotonicClock for ManualClock {
    fn now(&self) -> Instant {
        *self.0.lock()
    }
}

#[derive(Clone, Copy)]
struct TestInput {
    charge: WorkCharge,
}

#[derive(Clone, Copy)]
struct TestSpec;

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
#[error("test job failed")]
struct TestError;

impl WorkFamilySpec for TestSpec {
    const FAMILY: WorkFamily = WorkFamily::CardPreparation;
    type Input = TestInput;
    type Output = u32;
    type Error = TestError;
    type Phase = PreparationPhase;

    fn revision(_input: &Self::Input) -> InputRevision {
        InputRevision::FIRST
    }

    fn charge(input: &Self::Input) -> WorkCharge {
        input.charge
    }

    fn map_cancellation(reason: Cancellation) -> Cancellation {
        reason
    }
}

fn new_service(clock: Arc<ManualClock>) -> Arc<Mutex<WorkService>> {
    let nonce = NEXT_TEST_NONCE.fetch_add(1, Ordering::Relaxed);
    let incarnation = ProcessIncarnation::from_start_nonce(
        NonZeroU64::new(nonce).expect("test nonce is nonzero"),
    );
    let clock: Arc<dyn MonotonicClock + Send + Sync> = clock;
    Arc::new(Mutex::new(WorkService::build(incarnation, clock)))
}

fn conditions(shared: &Arc<Mutex<WorkService>>) -> ProcessConditions {
    ProcessConditions::new(shared.lock().incarnation())
}

fn submit<J>(
    shared: &Arc<Mutex<WorkService>>,
    input: &TestInput,
    job: J,
    conditions: &ProcessConditions,
) -> WorkTicket<TestSpec>
where
    J: CooperativeJob<Output = u32, Error = TestError, Phase = PreparationPhase>,
{
    let mut service = shared.lock();
    let identity = service
        .mint_identity(Scope::Process, None)
        .expect("test identity mints");
    match service.try_submit::<TestSpec, J>(identity, input, job, conditions) {
        Ok(ticket) => ticket,
        Err(refused) => panic!("unexpected test refusal: {:?}", refused.refusal()),
    }
}

fn wait_for_jobs(shared: &Arc<Mutex<WorkService>>, expected: usize) {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let (notifier, epoch, jobs) = {
            let service = shared.lock();
            service.observe_health();
            let epoch = service.publication_epoch();
            let jobs = service.capacity().jobs();
            (Arc::clone(&service.shared.notifier), epoch, jobs)
        };
        if jobs == expected {
            return;
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(
            !remaining.is_zero(),
            "capacity did not reach {expected} jobs"
        );
        notifier.wait_for_change(epoch, remaining);
    }
}
#[test]
fn publication_wait_closes_change_before_and_during_wait_races() {
    let shared = new_service(Arc::new(ManualClock::new()));
    let notifier = Arc::clone(&shared.lock().shared.notifier);

    let observed = shared.lock().publication_epoch();
    notifier.signal_change();
    assert_eq!(
        WorkService::wait_for_publication_change(&shared, observed, Duration::ZERO),
        PublicationWaitOutcome::Changed
    );

    let observed = shared.lock().publication_epoch();
    let gate = notifier.gate.lock();
    let waiting = Arc::clone(&shared);
    let waiter = std::thread::spawn(move || {
        WorkService::wait_for_publication_change(&waiting, observed, Duration::from_secs(1))
    });
    let signaling = Arc::clone(&notifier);
    let signaler = std::thread::spawn(move || signaling.signal_change());
    drop(gate);
    assert_eq!(
        waiter.join().expect("publication wait returned"),
        PublicationWaitOutcome::Changed
    );
    signaler.join().expect("publication signal returned");
    WorkService::shutdown(&shared).expect("service shuts down");
}

#[test]
fn publication_wait_times_out_or_reports_shutdown_during_worker_outage() {
    let shared = new_service(Arc::new(ManualClock::new()));
    let observed = shared.lock().publication_epoch();
    assert_eq!(
        WorkService::wait_for_publication_change(&shared, observed, Duration::from_millis(1)),
        PublicationWaitOutcome::TimedOut
    );
    {
        shared.lock().begin_shutdown();
    }
    assert_eq!(
        WorkService::wait_for_publication_change(&shared, observed, Duration::ZERO),
        PublicationWaitOutcome::Unavailable
    );
    WorkService::shutdown(&shared).expect("service shuts down");

    let nonce = NEXT_TEST_NONCE.fetch_add(1, Ordering::Relaxed);
    let incarnation = ProcessIncarnation::from_start_nonce(
        NonZeroU64::new(nonce).expect("test nonce is nonzero"),
    );
    let unavailable =
        WorkService::build_with_spawn(incarnation, Arc::new(api::work::SystemClock), |_| {
            Err(std::io::Error::other("injected worker start failure"))
        });
    let unavailable = Arc::new(Mutex::new(unavailable));
    let observed = unavailable.lock().publication_epoch();
    assert_eq!(
        WorkService::wait_for_publication_change(&unavailable, observed, Duration::ZERO),
        PublicationWaitOutcome::TimedOut,
        "a background-worker outage does not stop request-thread publications"
    );
    WorkService::shutdown(&unavailable).expect("unavailable service shuts down");
}

struct QueueJob {
    id: u8,
    trace: Arc<Mutex<Vec<u8>>>,
    calls: Arc<AtomicUsize>,
    yields: usize,
    gate: Option<(SyncSender<()>, Receiver<()>)>,
}

impl QueueJob {
    fn new(
        id: u8,
        trace: Arc<Mutex<Vec<u8>>>,
        yields: usize,
        gate: Option<(SyncSender<()>, Receiver<()>)>,
    ) -> Self {
        Self {
            id,
            trace,
            calls: Arc::new(AtomicUsize::new(0)),
            yields,
            gate,
        }
    }
}

impl CooperativeJob for QueueJob {
    type Output = u32;
    type Error = TestError;
    type Phase = PreparationPhase;

    fn step(&mut self, context: &mut StepContext<'_, Self::Phase>) -> StepOutcome<u32, TestError> {
        self.trace.lock().push(self.id);
        let call = self.calls.fetch_add(1, Ordering::AcqRel) + 1;
        match context.checkpoint() {
            Checkpoint::Continue => {}
            Checkpoint::Yield => return StepOutcome::Yielded,
            Checkpoint::Cancel(_) => return StepOutcome::Completed(u32::from(self.id)),
        }
        context.publish_progress(
            PreparationPhase::Prepare,
            call as u32,
            Some((self.yields + 1) as u32),
        );
        match context.consume_unit() {
            Checkpoint::Continue => {}
            Checkpoint::Yield => return StepOutcome::Yielded,
            Checkpoint::Cancel(_) => return StepOutcome::Completed(u32::from(self.id)),
        }
        if let Some((entered, release)) = self.gate.take() {
            entered.send(()).expect("test observes the gated step");
            release
                .recv_timeout(Duration::from_secs(2))
                .expect("test releases the gated step");
        }
        if call <= self.yields {
            StepOutcome::Yielded
        } else {
            StepOutcome::Completed(u32::from(self.id))
        }
    }

    fn retained(&self) -> RetainedState {
        RetainedState::clean(0)
    }
}

#[test]
fn worker_start_failure_latches_typed_unavailable_without_replacement() {
    assert_eq!(
        WorkStartError::OsRandom.to_string(),
        ProcessServiceCode::WorkAdmissionUnavailable.operator_text()
    );
    let incarnation = ProcessIncarnation::from_start_nonce(NonZeroU64::new(79).unwrap());
    let clock: Arc<dyn MonotonicClock + Send + Sync> = Arc::new(ManualClock::new());
    let service = WorkService::build_with_spawn(incarnation, clock, |_| {
        Err(std::io::Error::other("injected worker start failure"))
    });
    let condition_id = {
        let conditions = service.conditions();
        let condition = conditions
            .latched(ProcessServiceCode::WorkAdmissionUnavailable)
            .expect("start failure is a process condition");
        assert_eq!(condition.cause(), ConditionCause::StartFailed);
        condition.id()
    };
    assert!(!service.worker_available());
    let shared = Arc::new(Mutex::new(service));
    let input = TestInput {
        charge: WorkCharge::new(0, 0, 0, 0),
    };
    let trace = Arc::new(Mutex::new(Vec::new()));
    let accepted_conditions = conditions(&shared);
    let mut service = shared.lock();
    let identity = service.mint_identity(Scope::Process, None).unwrap();
    let refused = service
        .try_submit::<TestSpec, _>(
            identity,
            &input,
            QueueJob::new(79, Arc::clone(&trace), 0, None),
            &accepted_conditions,
        )
        .expect_err("failed start never accepts work");
    assert!(matches!(
        refused.refusal(),
        SubmissionRefusal::Unavailable(_)
    ));
    assert_eq!(
        service.health(),
        Some(condition_id),
        "startup condition remains sticky"
    );
    assert!(trace.lock().is_empty(), "no fallback executed the job");
    drop(service);
    WorkService::shutdown(&shared).expect("there is no worker to retire");
}

#[test]
fn fifo_round_robin_runs_short_jobs_before_a_long_jobs_next_slice() {
    let shared = new_service(Arc::new(ManualClock::new()));
    let conditions = conditions(&shared);
    let input = TestInput {
        charge: WorkCharge::new(32, 16, 16, 1),
    };
    let trace = Arc::new(Mutex::new(Vec::new()));
    let (entered_tx, entered_rx) = mpsc::sync_channel(0);
    let (release_tx, release_rx) = mpsc::sync_channel(0);
    let long = QueueJob::new(1, Arc::clone(&trace), 1, Some((entered_tx, release_rx)));
    let long_calls = Arc::clone(&long.calls);
    let long_ticket = submit(&shared, &input, long, &conditions);
    entered_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("long job enters its first slice");

    let first_short = submit(
        &shared,
        &input,
        QueueJob::new(2, Arc::clone(&trace), 0, None),
        &conditions,
    );
    let second_short = submit(
        &shared,
        &input,
        QueueJob::new(3, Arc::clone(&trace), 0, None),
        &conditions,
    );
    release_tx.send(()).expect("release long job's first slice");

    WorkService::wait_terminal(&shared, &long_ticket).expect("long job completes");
    assert_eq!(*trace.lock(), [1, 2, 3, 1]);
    assert_eq!(long_calls.load(Ordering::Acquire), 2);
    {
        let service = shared.lock();
        assert_eq!(service.capacity().jobs(), 3);
        let progress: &WorkProgress = service.progress(&long_ticket).expect("long progress");
        assert_eq!(progress.sequence(), 2);
        assert_eq!(progress.phase(), PreparationPhase::Prepare.index());
        assert_eq!(progress.completed(), 2);
        assert_eq!(progress.total(), Some(2));
    }

    for ticket in [&long_ticket, &first_short, &second_short] {
        WorkService::wait_terminal(&shared, ticket).expect("short job completes");
    }
    {
        let mut service = shared.lock();
        assert!(matches!(
            service.consume(&long_ticket).expect("consume long result"),
            Settlement::Completed(1)
        ));
        assert!(matches!(
            service
                .consume(&first_short)
                .expect("consume first short result"),
            Settlement::Completed(2)
        ));
        assert!(matches!(
            service
                .consume(&second_short)
                .expect("consume second short result"),
            Settlement::Completed(3)
        ));
    }
    wait_for_jobs(&shared, 0);
}

struct SlowQueuedJob;

impl CooperativeJob for SlowQueuedJob {
    type Output = u32;
    type Error = TestError;
    type Phase = PreparationPhase;

    fn step(&mut self, context: &mut StepContext<'_, Self::Phase>) -> StepOutcome<u32, TestError> {
        if !matches!(context.checkpoint(), Checkpoint::Continue) {
            return StepOutcome::Yielded;
        }
        std::thread::park_timeout(Duration::from_millis(100));
        StepOutcome::Completed(9)
    }

    fn retained(&self) -> RetainedState {
        RetainedState::clean(0)
    }
}

#[test]
fn a_healthy_busy_worker_does_not_time_out_a_queued_receipt_after_five_seconds() {
    let shared = new_service(Arc::new(ManualClock::new()));
    let conditions = conditions(&shared);
    let input = TestInput {
        charge: WorkCharge::new(32, 16, 16, 0),
    };
    let trace = Arc::new(Mutex::new(Vec::new()));
    let (entered_tx, entered_rx) = mpsc::sync_channel(0);
    let (release_tx, release_rx) = mpsc::sync_channel(0);
    let first = submit(
        &shared,
        &input,
        QueueJob::new(1, Arc::clone(&trace), 0, Some((entered_tx, release_rx))),
        &conditions,
    );
    entered_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("first job enters worker");

    let queued = (0..62)
        .map(|_| submit(&shared, &input, SlowQueuedJob, &conditions))
        .collect::<Vec<_>>();
    let last = submit(
        &shared,
        &input,
        QueueJob::new(2, Arc::clone(&trace), 0, None),
        &conditions,
    );
    let (wait_tx, wait_rx) = mpsc::sync_channel(1);
    let waiter_service = Arc::clone(&shared);
    let waiter_ticket = last;
    let waiter = std::thread::spawn(move || {
        wait_tx
            .send(WorkService::wait_terminal(&waiter_service, &waiter_ticket))
            .expect("test receives queued result");
    });
    release_tx.send(()).expect("release first job");

    assert!(
        matches!(
            wait_rx.recv_timeout(Duration::from_millis(5200)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ),
        "a busy but healthy worker must not report WorkerNotReturned at five seconds"
    );
    wait_rx
        .recv_timeout(Duration::from_secs(3))
        .expect("queued receipt settles after the healthy worker drains")
        .expect("waiting for a healthy worker succeeds");
    waiter.join().expect("receipt waiter exits");
    let mut service = shared.lock();
    assert!(matches!(
        service.consume(&first),
        Ok(Settlement::Completed(1))
    ));
    for ticket in queued {
        assert!(matches!(
            service.consume(&ticket),
            Ok(Settlement::Completed(9))
        ));
    }
    assert!(matches!(
        service.consume(&last),
        Ok(Settlement::Completed(2))
    ));
}

/// A short, ordinary Start with no qualified decode allowance.
#[derive(Debug)]
struct ShortProfileJob;

impl CooperativeJob for ShortProfileJob {
    type Output = u32;
    type Error = TestError;
    type Phase = PreparationPhase;

    fn step(&mut self, context: &mut StepContext<'_, Self::Phase>) -> StepOutcome<u32, TestError> {
        match context.checkpoint() {
            Checkpoint::Cancel(_) | Checkpoint::Yield => StepOutcome::Yielded,
            Checkpoint::Continue => StepOutcome::Completed(5),
        }
    }

    fn retained(&self) -> RetainedState {
        RetainedState::clean(0)
    }
}

#[test]
fn maximal_catalogue_and_native_sized_start_both_complete() {
    let clock = Arc::new(ManualClock::new());
    let shared = new_service(Arc::clone(&clock));
    let conditions = conditions(&shared);
    let trace = Arc::new(Mutex::new(Vec::new()));
    let (entered_tx, entered_rx) = mpsc::sync_channel(0);
    let (release_tx, release_rx) = mpsc::sync_channel(0);
    let catalogue = submit(
        &shared,
        &TestInput {
            charge: WorkCharge::new(64 << 20, 32 << 20, 32 << 20, 0),
        },
        QueueJob::new(1, Arc::clone(&trace), 0, Some((entered_tx, release_rx))),
        &conditions,
    );
    entered_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("catalogue is in flight");
    let start = {
        let mut service = shared.lock();
        let identity = service.mint_identity(Scope::Process, None).unwrap();
        service
            .try_submit::<ProfileSpec, _>(
                identity,
                &TestInput {
                    charge: WorkCharge::new(4096, 2 << 20, 2 << 20, 0),
                },
                ShortProfileJob,
                &conditions,
            )
            .expect("normal Start remains admissible beside maximal catalogue")
    };
    release_tx.send(()).expect("release catalogue");
    WorkService::wait_terminal(&shared, &catalogue).expect("catalogue settles");
    WorkService::wait_terminal(&shared, &start).expect("Start settles");
    let mut service = shared.lock();
    assert!(matches!(
        service.consume(&catalogue),
        Ok(Settlement::Completed(1))
    ));
    assert!(matches!(
        service.consume(&start),
        Ok(Settlement::Completed(5))
    ));
}

#[test]
fn progress_remains_latest_after_sixty_four_updates() {
    let shared = new_service(Arc::new(ManualClock::new()));
    let conditions = conditions(&shared);
    let ticket = submit(
        &shared,
        &TestInput {
            charge: WorkCharge::new(0, 0, 0, 0),
        },
        QueueJob::new(8, Arc::new(Mutex::new(Vec::new())), 80, None),
        &conditions,
    );
    WorkService::wait_terminal(&shared, &ticket).expect("long job settles");
    let mut service = shared.lock();
    assert_eq!(service.progress(&ticket).unwrap().sequence(), 81);
    service.publications().progress().read(|_, value| {
        let latest = value.as_ref().expect("latest progress is published");
        assert_eq!(latest.identity(), ticket.identity());
        assert_eq!(latest.sequence(), 81);
    });
    assert!(matches!(
        service.consume(&ticket),
        Ok(Settlement::Completed(8))
    ));
}

#[test]
fn replaceable_progress_over_capacity_does_not_fail_the_job() {
    let shared = new_service(Arc::new(ManualClock::new()));
    let conditions = conditions(&shared);
    let yields = MAX_PROGRESS_SNAPSHOTS_PER_JOB + 1024;
    let ticket = submit(
        &shared,
        &TestInput {
            charge: WorkCharge::new(0, 0, 0, 0),
        },
        QueueJob::new(8, Arc::new(Mutex::new(Vec::new())), yields, None),
        &conditions,
    );
    WorkService::wait_terminal(&shared, &ticket).expect("job settles after progress fills");
    let mut service = shared.lock();
    assert_eq!(
        service.progress(&ticket).unwrap().sequence(),
        MAX_PROGRESS_SNAPSHOTS_PER_JOB as u64,
    );
    assert_eq!(service.dropped_progress_reports(), 1025);
    eprintln!(
        "PROGRESS_POLICY immutable_snapshots={} progress_bytes_per_job={} service_bytes={} dropped={}",
        MAX_PROGRESS_SNAPSHOTS_PER_JOB,
        PROGRESS_BYTES_PER_JOB,
        MAX_WORK_BYTES,
        service.dropped_progress_reports(),
    );
    assert!(matches!(
        service.consume(&ticket),
        Ok(Settlement::Completed(8))
    ));
    assert!(service.worker_available());
}

#[test]
fn admission_bounds_refuse_without_taking_job_custody() {
    let shared = new_service(Arc::new(ManualClock::new()));
    let conditions = conditions(&shared);
    let trace = Arc::new(Mutex::new(Vec::new()));
    let mut service = shared.lock();

    for (charge, expected) in [
        (
            WorkCharge::new(MAX_WORK_BYTES, 0, 0, 0),
            SubmissionRefusal::InvalidArgument,
        ),
        (
            WorkCharge::new(0, 0, 0, MAX_OPERATION_HANDLES + 1),
            SubmissionRefusal::InvalidArgument,
        ),
        (
            WorkCharge::new(usize::MAX, 1, 0, 0),
            SubmissionRefusal::InvalidArgument,
        ),
    ] {
        let identity = service
            .mint_identity(Scope::Process, None)
            .expect("test identity mints");
        let input = TestInput { charge };
        let job = QueueJob::new(90, Arc::clone(&trace), 0, None);
        let refused = match service.try_submit::<TestSpec, _>(identity, &input, job, &conditions) {
            Ok(_) => panic!("oversized charge was accepted"),
            Err(refused) => refused,
        };
        let (refusal, job) = refused.into_parts();
        assert_eq!(refusal, expected);
        assert_eq!(job.id, 90);
    }
    assert_eq!(service.capacity().jobs(), 0);
    assert!(trace.lock().is_empty());
}

#[test]
fn unconsumed_terminals_hold_all_job_slots_until_consumed() {
    let shared = new_service(Arc::new(ManualClock::new()));
    let conditions = conditions(&shared);
    let input = TestInput {
        charge: WorkCharge::new(0, 0, 0, 0),
    };
    let trace = Arc::new(Mutex::new(Vec::new()));
    let mut tickets = Vec::with_capacity(MAX_WORK_JOBS);
    for id in 0..MAX_WORK_JOBS {
        tickets.push(submit(
            &shared,
            &input,
            QueueJob::new(id as u8, Arc::clone(&trace), 0, None),
            &conditions,
        ));
    }
    for ticket in &tickets {
        WorkService::wait_terminal(&shared, ticket).expect("accepted job settles");
    }
    assert_eq!(shared.lock().capacity().jobs(), MAX_WORK_JOBS);

    let mut service = shared.lock();
    let identity = service
        .mint_identity(Scope::Process, None)
        .expect("test identity mints");
    let overflow = QueueJob::new(100, Arc::clone(&trace), 0, None);
    let refused = match service.try_submit::<TestSpec, _>(identity, &input, overflow, &conditions) {
        Ok(_) => panic!("unconsumed terminal slots were reused"),
        Err(refused) => refused,
    };
    let (refusal, overflow) = refused.into_parts();
    assert_eq!(refusal, SubmissionRefusal::Full);
    assert_eq!(overflow.id, 100);
    for ticket in &tickets {
        assert!(matches!(
            service.consume(ticket).expect("consume accepted terminal"),
            Settlement::Completed(_)
        ));
    }
    drop(service);
    wait_for_jobs(&shared, 0);
}

/// The terminal's UnsafeCell is published once, borrowed immutably under
/// service custody, then moved out exactly once under exclusive custody.
#[test]
fn terminal_borrow_and_take_preserve_single_publication() {
    let shared = new_service(Arc::new(ManualClock::new()));
    let conditions = conditions(&shared);
    let trace = Arc::new(Mutex::new(Vec::new()));
    let input = TestInput {
        charge: WorkCharge::new(0, 0, 0, 0),
    };
    let ticket = submit(
        &shared,
        &input,
        QueueJob::new(37, trace, 0, None),
        &conditions,
    );
    WorkService::wait_terminal(&shared, &ticket).expect("job publishes one terminal");
    let mut service = shared.lock();
    let first = service
        .borrow_terminal(&ticket)
        .expect("matching ticket")
        .expect("published terminal");
    assert!(matches!(first, Settlement::Completed(37)));
    let second = service
        .borrow_terminal(&ticket)
        .expect("matching ticket")
        .expect("published terminal");
    assert!(std::ptr::eq(first, second));
    assert!(matches!(
        service.consume(&ticket).expect("single terminal transfer"),
        Settlement::Completed(37)
    ));
    assert!(
        service.consume(&ticket).is_err(),
        "no second terminal exists"
    );
}

#[test]
fn cancellation_selected_before_yield_return_is_terminal_without_requeue() {
    let shared = new_service(Arc::new(ManualClock::new()));
    let conditions = conditions(&shared);
    let input = TestInput {
        charge: WorkCharge::new(0, 0, 0, 0),
    };
    let trace = Arc::new(Mutex::new(Vec::new()));
    let (entered_tx, entered_rx) = mpsc::sync_channel(0);
    let (release_tx, release_rx) = mpsc::sync_channel(0);
    let job = QueueJob::new(4, Arc::clone(&trace), 3, Some((entered_tx, release_rx)));
    let calls = Arc::clone(&job.calls);
    let ticket = submit(&shared, &input, job, &conditions);
    entered_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("job pauses after its checkpoint");
    assert_eq!(
        shared.lock().cancel(ticket, Cancellation::Operator),
        CancelOutcome::Requested
    );
    release_tx.send(()).expect("release the yielded step");

    WorkService::wait_terminal(&shared, &ticket).expect("cancellation settles");
    {
        let mut service = shared.lock();
        assert!(matches!(
            service.consume(&ticket).expect("consume cancellation"),
            Settlement::Cancelled(Cancellation::Operator)
        ));
    }
    wait_for_jobs(&shared, 0);
    assert_eq!(calls.load(Ordering::Acquire), 1);
    assert_eq!(*trace.lock(), [4]);
}

#[test]
fn abandoning_an_active_job_keeps_credit_until_the_worker_returns() {
    let shared = new_service(Arc::new(ManualClock::new()));
    let conditions = conditions(&shared);
    let input = TestInput {
        charge: WorkCharge::new(0, 0, 0, 0),
    };
    let trace = Arc::new(Mutex::new(Vec::new()));
    let (entered_tx, entered_rx) = mpsc::sync_channel(0);
    let (release_tx, release_rx) = mpsc::sync_channel(0);
    let job = QueueJob::new(5, Arc::clone(&trace), 0, Some((entered_tx, release_rx)));
    let ticket = submit(&shared, &input, job, &conditions);
    entered_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("job enters its active step");
    assert_eq!(
        shared
            .lock()
            .abandon(ticket, Cancellation::ScopeInvalidated),
        CancelOutcome::Requested
    );
    assert_eq!(shared.lock().capacity().jobs(), 1);
    release_tx.send(()).expect("release the active step");
    WorkService::wait_terminal(&shared, &ticket).expect("abandoned job reaches cancellation");
    wait_for_jobs(&shared, 0);
    assert_eq!(*trace.lock(), [5]);
}

struct ExcessiveJob;

impl CooperativeJob for ExcessiveJob {
    type Output = u32;
    type Error = TestError;
    type Phase = PreparationPhase;

    fn step(&mut self, context: &mut StepContext<'_, Self::Phase>) -> StepOutcome<u32, TestError> {
        for _ in 0..=WORK_UNITS_PER_SLICE {
            let _ = context.consume_unit();
        }
        StepOutcome::Completed(8)
    }

    fn retained(&self) -> RetainedState {
        RetainedState::clean(0)
    }
}

struct ForbiddenRetainJob;

impl CooperativeJob for ForbiddenRetainJob {
    type Output = u32;
    type Error = TestError;
    type Phase = PreparationPhase;

    fn step(&mut self, context: &mut StepContext<'_, Self::Phase>) -> StepOutcome<u32, TestError> {
        let _ = context.checkpoint();
        StepOutcome::Yielded
    }

    fn retained(&self) -> RetainedState {
        RetainedState::forbidden(ForbiddenRetain::V8Handle)
    }
}

struct FailedJob;

impl CooperativeJob for FailedJob {
    type Output = u32;
    type Error = TestError;
    type Phase = PreparationPhase;

    fn step(&mut self, _context: &mut StepContext<'_, Self::Phase>) -> StepOutcome<u32, TestError> {
        StepOutcome::Failed(TestError)
    }

    fn retained(&self) -> RetainedState {
        RetainedState::clean(0)
    }
}

#[test]
fn cooperation_violations_and_family_errors_are_typed_terminals() {
    let shared = new_service(Arc::new(ManualClock::new()));
    let conditions = conditions(&shared);
    let input = TestInput {
        charge: WorkCharge::new(0, 0, 0, 0),
    };
    let excessive = submit(&shared, &input, ExcessiveJob, &conditions);
    let forbidden = submit(&shared, &input, ForbiddenRetainJob, &conditions);
    let failed = submit(&shared, &input, FailedJob, &conditions);

    for ticket in [&excessive, &forbidden, &failed] {
        WorkService::wait_terminal(&shared, ticket).expect("job settles");
    }
    {
        let mut service = shared.lock();
        assert!(matches!(
            service.consume(&excessive).expect("consume exceeded job"),
            Settlement::Failed(JobError::Cooperation(CooperativeViolation::IgnoredYield))
        ));
        assert!(matches!(
            service
                .consume(&forbidden)
                .expect("consume forbidden retain"),
            Settlement::Failed(JobError::Cooperation(
                CooperativeViolation::ForbiddenRetain(ForbiddenRetain::V8Handle)
            ))
        ));
        assert!(matches!(
            service.consume(&failed).expect("consume family error"),
            Settlement::Failed(JobError::Family(TestError))
        ));
    }
    assert!(
        shared.lock().worker_available(),
        "a returned violation cannot poison the process worker"
    );
    assert_eq!(shared.lock().health(), None);
    let trace = Arc::new(Mutex::new(Vec::new()));
    let healthy = submit(
        &shared,
        &input,
        QueueJob::new(9, trace.clone(), 0, None),
        &conditions,
    );
    WorkService::wait_terminal(&shared, &healthy).expect("next admitted job settles");
    assert!(matches!(
        shared
            .lock()
            .consume(&healthy)
            .expect("consume healthy job"),
        Settlement::Completed(9)
    ));
    assert_eq!(*trace.lock(), [9]);
    wait_for_jobs(&shared, 0);
}

struct PanickingJob;

impl CooperativeJob for PanickingJob {
    type Output = u32;
    type Error = TestError;
    type Phase = PreparationPhase;

    fn step(&mut self, _context: &mut StepContext<'_, Self::Phase>) -> StepOutcome<u32, TestError> {
        panic!("injected job panic")
    }

    fn retained(&self) -> RetainedState {
        RetainedState::clean(0)
    }
}

#[test]
fn panicking_job_fails_only_itself_and_worker_accepts_following_job() {
    let shared = new_service(Arc::new(ManualClock::new()));
    let conditions = conditions(&shared);
    let input = TestInput {
        charge: WorkCharge::new(0, 0, 0, 0),
    };
    let panicking = submit(&shared, &input, PanickingJob, &conditions);
    WorkService::wait_terminal(&shared, &panicking).expect("panic is terminal");
    let terminal = shared
        .lock()
        .consume(&panicking)
        .expect("contained panic settles one job");
    let Settlement::Failed(error) = terminal else {
        panic!("contained panic must be a failed job");
    };
    assert!(matches!(error, JobError::FailedInternally));
    assert_eq!(error.to_string(), "background work job failed internally");
    assert!(shared.lock().worker_available());
    assert_eq!(shared.lock().health(), None);
    let healthy = submit(
        &shared,
        &input,
        QueueJob::new(7, Arc::new(Mutex::new(Vec::new())), 0, None),
        &conditions,
    );
    WorkService::wait_terminal(&shared, &healthy).expect("following job completes");
    assert!(matches!(
        shared.lock().consume(&healthy),
        Ok(Settlement::Completed(7))
    ));
}

#[derive(Clone, Copy)]
struct ProfileSpec;

impl WorkFamilySpec for ProfileSpec {
    const FAMILY: WorkFamily = WorkFamily::ProfilePreparation;
    type Input = TestInput;
    type Output = u32;
    type Error = TestError;
    type Phase = PreparationPhase;

    fn revision(_input: &Self::Input) -> InputRevision {
        InputRevision::FIRST
    }

    fn charge(input: &Self::Input) -> WorkCharge {
        input.charge
    }

    fn map_cancellation(reason: Cancellation) -> Cancellation {
        reason
    }
}

#[derive(Debug)]
struct QualifiedProfileJob {
    clock: Arc<ManualClock>,
    stage: u8,
    last_duration: Duration,
}

impl CooperativeJob for QualifiedProfileJob {
    type Output = u32;
    type Error = TestError;
    type Phase = PreparationPhase;

    fn step(&mut self, context: &mut StepContext<'_, Self::Phase>) -> StepOutcome<u32, TestError> {
        assert!(matches!(context.checkpoint(), Checkpoint::Continue));
        match self.stage {
            0 => {
                context.publish_progress(PreparationPhase::Load, 1, Some(1));
                assert!(matches!(context.consume_unit(), Checkpoint::Continue));
                self.stage = 1;
                StepOutcome::Yielded
            }
            1 => {
                self.clock.advance(Duration::from_millis(20));
                context.publish_progress(PreparationPhase::Prepare, 1, Some(2));
                assert!(matches!(context.consume_unit(), Checkpoint::Continue));
                self.stage = 2;
                StepOutcome::Yielded
            }
            2 => {
                self.clock.advance(self.last_duration);
                StepOutcome::Completed(5)
            }
            _ => unreachable!("three bounded profile steps"),
        }
    }

    fn retained(&self) -> RetainedState {
        RetainedState::clean(0)
    }
}

#[test]
fn a_declared_gather_decode_gets_only_its_first_prepare_checkpoint() {
    let clock = Arc::new(ManualClock::new());
    let shared = new_service(Arc::clone(&clock));
    let conditions = conditions(&shared);
    let input = TestInput {
        charge: WorkCharge::new(0, 0, 0, 0),
    };
    let ticket = {
        let mut service = shared.lock();
        let slot = api::identity::SlotKey::from_incarnation(1).unwrap();
        let identity = service.mint_identity(Scope::Slot(slot), None).unwrap();
        service
            .try_submit_qualified::<ProfileSpec, _>(
                identity,
                &input,
                QualifiedProfileJob {
                    clock,
                    stage: 0,
                    last_duration: Duration::from_millis(20),
                },
                &conditions,
                QualifiedUnit::GatheringFamilyDecode,
            )
            .unwrap_or_else(|refused| panic!("qualified profile refused: {:?}", refused.refusal()))
    };
    WorkService::wait_terminal(&shared, &ticket).expect("qualified job settles");
    assert!(matches!(
        shared.lock().consume(&ticket).expect("consume profile"),
        Settlement::Completed(5)
    ));
    assert_eq!(shared.lock().slice_overruns(), 1);
    assert!(shared.lock().worker_available());
    assert_eq!(shared.lock().health(), None);
}

#[test]
fn a_qualified_family_unit_cannot_be_attached_to_an_unrelated_job() {
    let clock = Arc::new(ManualClock::new());
    let shared = new_service(Arc::clone(&clock));
    let conditions = conditions(&shared);
    let input = TestInput {
        charge: WorkCharge::new(0, 0, 0, 0),
    };
    let mut service = shared.lock();
    let process = service.mint_identity(Scope::Process, None).unwrap();
    let process_ticket = service
        .try_submit_qualified::<ProfileSpec, _>(
            process,
            &input,
            QualifiedProfileJob {
                clock: Arc::clone(&clock),
                stage: 0,
                last_duration: Duration::from_millis(20),
            },
            &conditions,
            QualifiedUnit::GatheringFamilyDecode,
        )
        .expect("process-scoped profile save prepares the same pinned gathering family");
    let slot = api::identity::SlotKey::from_incarnation(2).unwrap();
    let other_family = service.mint_identity(Scope::Slot(slot), None).unwrap();
    assert!(matches!(
        service.try_submit_qualified::<TestSpec, _>(
            other_family,
            &input,
            QueueJob::new(1, Arc::new(Mutex::new(Vec::new())), 0, None),
            &conditions,
            QualifiedUnit::GatheringFamilyDecode,
        ),
        Err(refused) if refused.refusal() == SubmissionRefusal::InvalidArgument
    ));
    assert_eq!(service.capacity().jobs(), 1);
    drop(service);
    WorkService::wait_terminal(&shared, &process_ticket).expect("process profile settles");
    assert!(matches!(
        shared
            .lock()
            .consume(&process_ticket)
            .expect("consume profile"),
        Settlement::Completed(5)
    ));
}

#[test]
fn pinned_quester_progress_compile_has_one_bounded_profile_prepare_unit() {
    for last_duration in [Duration::from_millis(1), Duration::from_millis(20)] {
        let clock = Arc::new(ManualClock::new());
        let shared = new_service(Arc::clone(&clock));
        let conditions = conditions(&shared);
        let ticket = {
            let mut service = shared.lock();
            let slot = api::identity::SlotKey::from_incarnation(3).unwrap();
            let identity = service.mint_identity(Scope::Slot(slot), None).unwrap();
            service
                .try_submit_qualified::<ProfileSpec, _>(
                    identity,
                    &TestInput {
                        charge: WorkCharge::new(0, 0, 0, 0),
                    },
                    QualifiedProfileJob {
                        clock,
                        stage: 0,
                        last_duration,
                    },
                    &conditions,
                    QualifiedUnit::QuesterProgressCompile,
                )
                .expect("pinned profile compilation is admitted")
        };
        WorkService::wait_terminal(&shared, &ticket).expect("profile preparation settles");
        assert!(matches!(
            shared.lock().consume(&ticket).expect("consume profile"),
            Settlement::Completed(5)
        ));
        assert_eq!(
            shared.lock().slice_overruns(),
            u64::from(last_duration >= WORK_SLICE)
        );
        assert!(shared.lock().worker_available());
    }
}

#[derive(Clone, Copy)]
struct CatalogueSpec;

impl WorkFamilySpec for CatalogueSpec {
    const FAMILY: WorkFamily = WorkFamily::CataloguePreparation;
    type Input = TestInput;
    type Output = u32;
    type Error = TestError;
    type Phase = PreparationPhase;

    fn revision(_input: &Self::Input) -> InputRevision {
        InputRevision::FIRST
    }

    fn charge(input: &Self::Input) -> WorkCharge {
        input.charge
    }

    fn map_cancellation(reason: Cancellation) -> Cancellation {
        reason
    }
}

#[derive(Debug)]
struct QuesterRegistryJob {
    clock: Arc<ManualClock>,
    stage: u8,
    seal_duration: Duration,
}

impl CooperativeJob for QuesterRegistryJob {
    type Output = u32;
    type Error = TestError;
    type Phase = PreparationPhase;

    fn step(&mut self, context: &mut StepContext<'_, Self::Phase>) -> StepOutcome<u32, TestError> {
        assert!(matches!(context.checkpoint(), Checkpoint::Continue));
        match self.stage {
            0 => {
                context.publish_progress(PreparationPhase::Load, 1, Some(1));
                self.stage = 1;
                StepOutcome::Yielded
            }
            1 | 2 => {
                self.clock.advance(Duration::from_millis(20));
                let completed = self.stage as u32;
                context.publish_progress(
                    PreparationPhase::Prepare,
                    completed,
                    (self.stage == 2).then_some(completed),
                );
                self.stage += 1;
                StepOutcome::Yielded
            }
            3 => {
                self.clock.advance(self.seal_duration);
                context.publish_progress(PreparationPhase::Seal, 1, Some(1));
                StepOutcome::Completed(5)
            }
            _ => unreachable!("three preparation actions and a seal"),
        }
    }

    fn retained(&self) -> RetainedState {
        RetainedState::clean(0)
    }
}

#[test]
fn bounded_quester_actions_each_qualify_but_seal_returns_to_ordinary_allowance() {
    for seal_duration in [Duration::from_millis(1), Duration::from_millis(20)] {
        let clock = Arc::new(ManualClock::new());
        let shared = new_service(Arc::clone(&clock));
        let conditions = conditions(&shared);
        let ticket = {
            let mut service = shared.lock();
            let identity = service.mint_identity(Scope::Process, None).unwrap();
            service
                .try_submit_qualified::<CatalogueSpec, _>(
                    identity,
                    &TestInput {
                        charge: WorkCharge::new(0, 0, 0, 0),
                    },
                    QuesterRegistryJob {
                        clock,
                        stage: 0,
                        seal_duration,
                    },
                    &conditions,
                    QualifiedUnit::QuesterRegistryAction,
                )
                .expect("catalogue Quester action is admitted")
        };
        WorkService::wait_terminal(&shared, &ticket).expect("registry preparation settles");
        assert!(matches!(
            shared.lock().consume(&ticket).expect("consume catalogue"),
            Settlement::Completed(5)
        ));
        assert_eq!(
            shared.lock().slice_overruns(),
            u64::from(seal_duration >= WORK_SLICE)
        );
        assert!(shared.lock().worker_available());
    }
}

#[derive(Debug)]
struct MissingProgressQualification {
    clock: Arc<ManualClock>,
    stage: u8,
    decisions: Arc<Mutex<Vec<Checkpoint>>>,
}

impl CooperativeJob for MissingProgressQualification {
    type Output = u32;
    type Error = TestError;
    type Phase = PreparationPhase;

    fn step(&mut self, context: &mut StepContext<'_, Self::Phase>) -> StepOutcome<u32, TestError> {
        match self.stage {
            0 => {
                context.publish_progress(PreparationPhase::Load, 1, Some(1));
                self.stage = 1;
                StepOutcome::Yielded
            }
            1 => {
                self.clock.advance(Duration::from_millis(20));
                self.decisions.lock().push(context.checkpoint());
                self.stage = 2;
                StepOutcome::Yielded
            }
            2 => {
                self.clock.advance(Duration::from_millis(20));
                self.decisions.lock().push(context.checkpoint());
                context.publish_progress(PreparationPhase::Prepare, 1, None);
                self.stage = 3;
                StepOutcome::Yielded
            }
            3 => StepOutcome::Completed(5),
            _ => unreachable!("bounded qualified probe"),
        }
    }

    fn retained(&self) -> RetainedState {
        RetainedState::clean(0)
    }
}

#[test]
fn no_progress_yield_does_not_renew_one_shot_qualified_allowance() {
    let clock = Arc::new(ManualClock::new());
    let shared = new_service(Arc::clone(&clock));
    let conditions = conditions(&shared);
    let decisions = Arc::new(Mutex::new(Vec::new()));
    let ticket = {
        let mut service = shared.lock();
        let identity = service.mint_identity(Scope::Process, None).unwrap();
        service
            .try_submit_qualified::<ProfileSpec, _>(
                identity,
                &TestInput {
                    charge: WorkCharge::new(0, 0, 0, 0),
                },
                MissingProgressQualification {
                    clock,
                    stage: 0,
                    decisions: Arc::clone(&decisions),
                },
                &conditions,
                QualifiedUnit::GatheringFamilyDecode,
            )
            .expect("one-shot pinned decode admitted")
    };
    WorkService::wait_terminal(&shared, &ticket).expect("one-shot unit settles");
    assert!(matches!(
        shared.lock().consume(&ticket),
        Ok(Settlement::Completed(5))
    ));
    assert_eq!(*decisions.lock(), [Checkpoint::Continue, Checkpoint::Yield]);
}

#[test]
fn registry_qualification_is_debited_without_progress_and_exhaustion_fails_one_job() {
    let clock = Arc::new(ManualClock::new());
    let shared = new_service(Arc::clone(&clock));
    let conditions = conditions(&shared);
    let decisions = Arc::new(Mutex::new(Vec::new()));
    let (entered_tx, entered_rx) = mpsc::sync_channel(0);
    let (release_tx, release_rx) = mpsc::sync_channel(0);
    let gate = submit(
        &shared,
        &TestInput {
            charge: WorkCharge::new(0, 0, 0, 0),
        },
        QueueJob::new(
            1,
            Arc::new(Mutex::new(Vec::new())),
            0,
            Some((entered_tx, release_rx)),
        ),
        &conditions,
    );
    entered_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("worker enters the gated first job");
    let ticket = {
        let mut service = shared.lock();
        let identity = service.mint_identity(Scope::Process, None).unwrap();
        let ticket = service
            .try_submit_qualified::<CatalogueSpec, _>(
                identity,
                &TestInput {
                    charge: WorkCharge::new(0, 0, 0, 0),
                },
                MissingProgressQualification {
                    clock,
                    stage: 0,
                    decisions: Arc::clone(&decisions),
                },
                &conditions,
                QualifiedUnit::QuesterRegistryAction,
            )
            .expect("bounded registry admitted");
        service
            .jobs
            .get(&ticket.job())
            .expect("queued registry entry")
            .control
            .lock()
            .qualified_dispatches = MAX_QUALIFIED_DISPATCHES_PER_JOB - 1;
        ticket
    };
    release_tx.send(()).expect("release gated worker");
    WorkService::wait_terminal(&shared, &ticket).expect("exhausted registry settles");
    assert!(matches!(
        shared.lock().consume(&ticket),
        Ok(Settlement::Failed(JobError::Cooperation(
            CooperativeViolation::UnboundedSlice
        )))
    ));
    assert_eq!(*decisions.lock(), [Checkpoint::Continue, Checkpoint::Yield]);
    WorkService::wait_terminal(&shared, &gate).expect("earlier gate settles");
    assert!(matches!(
        shared.lock().consume(&gate),
        Ok(Settlement::Completed(1))
    ));
    assert!(shared.lock().worker_available());
}

struct StrandedJob {
    entered: SyncSender<()>,
    resume: Receiver<()>,
    drop_entered: SyncSender<()>,
    drop_resume: Receiver<()>,
}

impl CooperativeJob for StrandedJob {
    type Output = u32;
    type Error = TestError;
    type Phase = PreparationPhase;

    fn step(&mut self, _context: &mut StepContext<'_, Self::Phase>) -> StepOutcome<u32, TestError> {
        self.entered.send(()).expect("test observes active worker");
        self.resume
            .recv_timeout(Duration::from_secs(2))
            .expect("test releases active worker");
        StepOutcome::Completed(9)
    }

    fn retained(&self) -> RetainedState {
        RetainedState::clean(0)
    }
}

impl Drop for StrandedJob {
    fn drop(&mut self) {
        let _ = self.drop_entered.send(());
        let _ = self.drop_resume.recv_timeout(Duration::from_secs(2));
    }
}

#[test]
fn stalled_worker_publishes_unavailable_before_return_without_releasing_credit() {
    let clock = Arc::new(ManualClock::new());
    let shared = new_service(Arc::clone(&clock));
    let conditions = conditions(&shared);
    let input = TestInput {
        charge: WorkCharge::new(4, 8, 16, 1),
    };
    let (entered_tx, entered_rx) = mpsc::sync_channel(0);
    let (resume_tx, resume_rx) = mpsc::sync_channel(0);
    let (drop_entered_tx, drop_entered_rx) = mpsc::sync_channel(0);
    let (drop_resume_tx, drop_resume_rx) = mpsc::sync_channel(0);
    let ticket = submit(
        &shared,
        &input,
        StrandedJob {
            entered: entered_tx,
            resume: resume_rx,
            drop_entered: drop_entered_tx,
            drop_resume: drop_resume_rx,
        },
        &conditions,
    );
    entered_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("worker enters the non-returning step");

    clock.advance(WORKER_STALL_LIMIT + Duration::from_millis(1));
    WorkService::wait_terminal(&shared, &ticket)
        .expect("health observer publishes outage terminal");
    let condition = {
        let mut service = shared.lock();
        let condition = service.health().expect("sticky work condition");
        assert!(matches!(
            service.borrow_terminal(&ticket).expect("borrow terminal"),
            Some(Settlement::Failed(JobError::Unavailable(terminal)))
                if terminal.condition() == condition
        ));
        assert_eq!(service.capacity().jobs(), 1);

        let identity = service
            .mint_identity(Scope::Process, None)
            .expect("next test identity mints");
        let refused = match service.try_submit::<TestSpec, _>(
            identity,
            &input,
            QueueJob::new(6, Arc::new(Mutex::new(Vec::new())), 0, None),
            &conditions,
        ) {
            Ok(_) => panic!("unavailable service accepted new work"),
            Err(refused) => refused,
        };
        match refused.refusal() {
            SubmissionRefusal::Unavailable(refusal) => {
                assert_eq!(refusal.condition(), condition);
            }
            refusal => panic!("unexpected refusal: {refusal:?}"),
        }
        assert!(matches!(
            service.consume(&ticket).expect("consume outage terminal"),
            Settlement::Failed(JobError::Unavailable(terminal))
                if terminal.condition() == condition
        ));
        assert_eq!(service.capacity().jobs(), 1);
        condition
    };

    resume_tx.send(()).expect("release the stranded step");
    drop_entered_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("job cleanup is held before worker-return accounting");
    let still_charged = shared.lock().capacity().jobs() == 1;
    drop_resume_tx
        .send(())
        .expect("allow the worker to record its actual return");
    wait_for_jobs(&shared, 0);
    assert!(still_charged);
    assert_eq!(shared.lock().health(), Some(condition));
}

#[test]
fn slow_step_returns_after_ten_seconds_without_latching_outage() {
    let clock = Arc::new(ManualClock::new());
    let shared = new_service(Arc::clone(&clock));
    let conditions = conditions(&shared);
    let input = TestInput {
        charge: WorkCharge::new(4, 8, 16, 1),
    };
    let trace = Arc::new(Mutex::new(Vec::new()));
    let (entered_tx, entered_rx) = mpsc::sync_channel(0);
    let (release_tx, release_rx) = mpsc::sync_channel(0);
    let ticket = submit(
        &shared,
        &input,
        QueueJob::new(10, Arc::clone(&trace), 0, Some((entered_tx, release_rx))),
        &conditions,
    );
    entered_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("worker enters the slow step");

    clock.advance(Duration::from_secs(10));
    let health_during_slow_step = {
        let service = shared.lock();
        service.observe_health();
        service.health()
    };

    release_tx.send(()).expect("release the slow step");
    WorkService::wait_terminal(&shared, &ticket).expect("slow step completes normally");
    let settlement = shared
        .lock()
        .consume(&ticket)
        .expect("consume completed slow step");
    wait_for_jobs(&shared, 0);

    assert!(health_during_slow_step.is_none());
    assert!(shared.lock().health().is_none());
    assert_eq!(settlement, Settlement::Completed(10));
    assert_eq!(*trace.lock(), [10]);
}

#[test]
fn stalled_worker_latches_at_exact_stall_limit() {
    let clock = Arc::new(ManualClock::new());
    let shared = new_service(Arc::clone(&clock));
    let conditions = conditions(&shared);
    let input = TestInput {
        charge: WorkCharge::new(4, 8, 16, 1),
    };
    let (entered_tx, entered_rx) = mpsc::sync_channel(0);
    let (resume_tx, resume_rx) = mpsc::sync_channel(0);
    let (drop_entered_tx, drop_entered_rx) = mpsc::sync_channel(0);
    let (drop_resume_tx, drop_resume_rx) = mpsc::sync_channel(0);
    let ticket = submit(
        &shared,
        &input,
        StrandedJob {
            entered: entered_tx,
            resume: resume_rx,
            drop_entered: drop_entered_tx,
            drop_resume: drop_resume_rx,
        },
        &conditions,
    );
    entered_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("worker enters the non-returning step");

    clock.advance(WORKER_STALL_LIMIT);
    shared.lock().observe_health();
    let condition = shared.lock().health();
    let outage_terminal = if let Some(condition) = condition {
        WorkService::wait_terminal(&shared, &ticket).expect("outage settles the ticket");
        let terminal = shared
            .lock()
            .consume(&ticket)
            .expect("consume outage terminal");
        matches!(
            terminal,
            Settlement::Failed(JobError::Unavailable(terminal))
                if terminal.condition() == condition
        )
    } else {
        false
    };

    resume_tx.send(()).expect("release the stalled step");
    drop_entered_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("job cleanup waits before worker-return accounting");
    drop_resume_tx
        .send(())
        .expect("allow worker-return accounting");
    if condition.is_none() {
        WorkService::wait_terminal(&shared, &ticket).expect("worker returns after release");
        assert_eq!(
            shared
                .lock()
                .consume(&ticket)
                .expect("consume completed ticket"),
            Settlement::Completed(9)
        );
    }
    wait_for_jobs(&shared, 0);

    assert!(
        condition.is_some(),
        "the worker outage must latch at the exact stall limit"
    );
    assert!(outage_terminal, "the latch must settle the accepted ticket");
}

#[test]
fn wait_terminal_detects_stalled_worker_within_recheck_interval() {
    let clock = Arc::new(ManualClock::new());
    let shared = new_service(Arc::clone(&clock));
    let conditions = conditions(&shared);
    let input = TestInput {
        charge: WorkCharge::new(4, 8, 16, 1),
    };
    let (entered_tx, entered_rx) = mpsc::sync_channel(0);
    let (resume_tx, resume_rx) = mpsc::sync_channel(0);
    let (drop_entered_tx, drop_entered_rx) = mpsc::sync_channel(0);
    let (drop_resume_tx, drop_resume_rx) = mpsc::sync_channel(0);
    let ticket = submit(
        &shared,
        &input,
        StrandedJob {
            entered: entered_tx,
            resume: resume_rx,
            drop_entered: drop_entered_tx,
            drop_resume: drop_resume_rx,
        },
        &conditions,
    );
    entered_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("worker enters the non-returning step");

    let (wait_entered_tx, wait_entered_rx) = mpsc::channel();
    shared
        .lock()
        .shared
        .notifier
        .hook_next_wait(wait_entered_tx);
    let waiter_ticket = WorkTicket::<TestSpec>::new(ticket.identity(), ticket.revision());
    let (wait_tx, wait_rx) = mpsc::channel();
    let waiter_shared = Arc::clone(&shared);
    let waiter = std::thread::spawn(move || {
        let result = WorkService::wait_terminal(&waiter_shared, &waiter_ticket);
        let _ = wait_tx.send(result);
    });
    let wait_hook_received = wait_entered_rx
        .recv_timeout(Duration::from_secs(10))
        .is_ok();
    let waiter_parked = if wait_hook_received {
        let notifier = Arc::clone(&shared.lock().shared.notifier);
        let _gate = notifier.gate.lock();
        true
    } else {
        false
    };

    // The fake clock jumps to the 60-second boundary while the caller waits.
    // The recheck interval is at most 5 s by design (1 s today); a 10 s budget
    // leaves margin for a heavily loaded host and still fails a recheck that
    // is missing or set far above the 5 s ceiling.
    let detection_bound = Duration::from_secs(10);
    let detection_started = Instant::now();
    clock.advance(WORKER_STALL_LIMIT);
    let wait_result = wait_rx.recv_timeout(detection_bound);
    let detected = matches!(wait_result, Ok(Ok(())));
    let detection_elapsed = detection_started.elapsed();
    let condition = shared.lock().health();
    let outage_terminal = if let Some(condition) = condition {
        let terminal = shared
            .lock()
            .consume(&ticket)
            .expect("consume outage terminal");
        matches!(
            terminal,
            Settlement::Failed(JobError::Unavailable(terminal))
                if terminal.condition() == condition
        )
    } else {
        false
    };

    resume_tx.send(()).expect("release the stalled step");
    drop_entered_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("job cleanup waits before worker-return accounting");
    drop_resume_tx
        .send(())
        .expect("allow worker-return accounting");
    if condition.is_none() {
        WorkService::wait_terminal(&shared, &ticket).expect("worker returns after release");
        assert_eq!(
            shared
                .lock()
                .consume(&ticket)
                .expect("consume completed ticket"),
            Settlement::Completed(9)
        );
    }
    wait_for_jobs(&shared, 0);
    waiter.join().expect("waiter thread joins");

    assert!(waiter_parked, "wait_terminal reaches its timed wait");
    assert!(
        detected,
        "wait_terminal must detect the outage within the recheck interval"
    );
    assert!(
        detection_elapsed <= detection_bound,
        "outage detection took {detection_elapsed:?}, over {detection_bound:?}"
    );
    assert!(condition.is_some(), "the service must latch the outage");
    assert!(
        outage_terminal,
        "the outage must settle the accepted ticket"
    );
}

fn wait_for_shutdown_start(shared: &Arc<Mutex<WorkService>>) {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let (notifier, epoch, stopping) = {
            let service = shared.lock();
            let epoch = service.publication_epoch();
            (
                Arc::clone(&service.shared.notifier),
                epoch,
                service.stopping,
            )
        };
        if stopping {
            return;
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(!remaining.is_zero(), "shutdown did not stop admission");
        notifier.wait_for_change(epoch, remaining);
    }
}

#[test]
fn orderly_shutdown_stops_admission_cancels_pending_and_joins_worker() {
    let shared = new_service(Arc::new(ManualClock::new()));
    let conditions = conditions(&shared);
    let input = TestInput {
        charge: WorkCharge::new(0, 0, 0, 0),
    };
    let trace = Arc::new(Mutex::new(Vec::new()));
    let (entered_tx, entered_rx) = mpsc::sync_channel(0);
    let (release_tx, release_rx) = mpsc::sync_channel(0);
    let ticket = submit(
        &shared,
        &input,
        QueueJob::new(7, Arc::clone(&trace), 3, Some((entered_tx, release_rx))),
        &conditions,
    );
    entered_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("worker enters before shutdown");
    let queued_ticket = submit(
        &shared,
        &input,
        QueueJob::new(8, Arc::clone(&trace), 0, None),
        &conditions,
    );

    let shutdown_shared = Arc::clone(&shared);
    let (shutdown_tx, shutdown_rx) = mpsc::sync_channel(0);
    let shutdown_thread = std::thread::spawn(move || {
        shutdown_tx
            .send(WorkService::shutdown(&shutdown_shared))
            .expect("test receives shutdown result");
    });
    wait_for_shutdown_start(&shared);

    let mut service = shared.lock();
    let identity = service
        .mint_identity(Scope::Process, None)
        .expect("test identity mints");
    let refused = match service.try_submit::<TestSpec, _>(
        identity,
        &input,
        QueueJob::new(9, Arc::clone(&trace), 0, None),
        &conditions,
    ) {
        Ok(_) => panic!("shutdown service accepted new work"),
        Err(refused) => refused,
    };
    let (refusal, job) = refused.into_parts();
    assert_eq!(refusal, SubmissionRefusal::Stopping);
    assert_eq!(job.id, 9);
    drop(service);

    release_tx.send(()).expect("release the cancelled step");
    assert_eq!(
        shutdown_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("shutdown joins the returned worker"),
        Ok(())
    );
    shutdown_thread.join().expect("shutdown caller exits");
    WorkService::wait_terminal(&shared, &ticket).expect("shutdown cancellation is terminal");
    WorkService::wait_terminal(&shared, &queued_ticket)
        .expect("queued shutdown cancellation is terminal");
    {
        let mut service = shared.lock();
        assert!(service.worker_joined);
        assert!(matches!(
            service
                .borrow_terminal(&ticket)
                .expect("borrow shutdown terminal"),
            Some(Settlement::Cancelled(Cancellation::Shutdown))
        ));
        assert_eq!(service.capacity().jobs(), 2);
        assert!(matches!(
            service
                .consume(&ticket)
                .expect("consume shutdown cancellation"),
            Settlement::Cancelled(Cancellation::Shutdown)
        ));
        assert_eq!(service.capacity().jobs(), 1);
        assert!(matches!(
            service
                .consume(&queued_ticket)
                .expect("consume queued shutdown cancellation"),
            Settlement::Cancelled(Cancellation::Shutdown)
        ));
        assert_eq!(service.capacity().jobs(), 0);
    }
    assert_eq!(*trace.lock(), [7]);
}

#[test]
fn shutdown_timeout_retains_credit_until_late_worker_return_and_join() {
    let clock = Arc::new(ManualClock::new());
    let shared = new_service(Arc::clone(&clock));
    let conditions = conditions(&shared);
    let input = TestInput {
        charge: WorkCharge::new(1, 2, 3, 1),
    };
    let trace = Arc::new(Mutex::new(Vec::new()));
    let (entered_tx, entered_rx) = mpsc::sync_channel(0);
    let (release_tx, release_rx) = mpsc::sync_channel(0);
    let ticket = submit(
        &shared,
        &input,
        QueueJob::new(9, Arc::clone(&trace), 3, Some((entered_tx, release_rx))),
        &conditions,
    );
    entered_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("worker enters before bounded shutdown");
    clock.advance(WORKER_STALL_LIMIT + Duration::from_millis(1));

    assert_eq!(
        WorkService::shutdown_with_timeout(&shared, Duration::from_millis(25)),
        Err(WorkShutdownError::ThreadsNotReturned {
            background_worker: true,
            request_threads: 0,
        })
    );
    let condition = {
        let mut service = shared.lock();
        let condition = service.health().expect("stalled worker latches health");
        assert!(matches!(
            service
                .borrow_terminal(&ticket)
                .expect("borrow cancel terminal"),
            Some(Settlement::Cancelled(Cancellation::Shutdown))
        ));
        assert_eq!(service.capacity().jobs(), 1);
        assert!(matches!(
            service
                .consume(&ticket)
                .expect("consume shutdown cancellation"),
            Settlement::Cancelled(Cancellation::Shutdown)
        ));
        assert_eq!(service.capacity().jobs(), 1);
        condition
    };

    release_tx.send(()).expect("release the stalled step");
    wait_for_jobs(&shared, 0);
    assert_eq!(shared.lock().health(), Some(condition));
    assert_eq!(
        WorkService::shutdown_with_timeout(&shared, Duration::from_secs(1)),
        Ok(())
    );
    assert!(shared.lock().worker_joined);
    assert_eq!(*trace.lock(), [9]);
}

#[path = "work_admission/cancellation_tests.rs"]
mod cancellation;

#[path = "work_admission/request_thread_tests.rs"]
mod request_thread;
