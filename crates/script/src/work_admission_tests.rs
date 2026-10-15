use super::*;
use api::identity::{Cancellation, ProcessIncarnation, Scope, Settlement};
use api::process_condition::ProcessConditions;
use api::work::{
    Checkpoint, CooperativeJob, CooperativeViolation, ForbiddenRetain, InputRevision, JobError,
    MapBakePhase, RetainedState, StepContext, StepOutcome, SubmissionRefusal, WorkFamily,
    WorkFamilySpec, WorkProgress, WorkTicket,
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
    const FAMILY: WorkFamily = WorkFamily::MapBake;
    type Input = TestInput;
    type Output = u32;
    type Error = TestError;
    type Phase = MapBakePhase;

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
    J: CooperativeJob<Output = u32, Error = TestError, Phase = MapBakePhase>,
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
    type Phase = MapBakePhase;

    fn step(&mut self, context: &mut StepContext<'_, Self::Phase>) -> StepOutcome<u32, TestError> {
        self.trace.lock().push(self.id);
        let call = self.calls.fetch_add(1, Ordering::AcqRel) + 1;
        match context.checkpoint() {
            Checkpoint::Continue => {}
            Checkpoint::Yield => return StepOutcome::Yielded,
            Checkpoint::Cancel(_) => return StepOutcome::Completed(u32::from(self.id)),
        }
        context.publish_progress(
            MapBakePhase::BakingPlane,
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
        assert_eq!(progress.phase(), MapBakePhase::BakingPlane.index());
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

#[test]
fn admission_bounds_refuse_without_taking_job_custody() {
    let shared = new_service(Arc::new(ManualClock::new()));
    let conditions = conditions(&shared);
    let trace = Arc::new(Mutex::new(Vec::new()));
    let mut service = shared.lock();

    for (charge, expected) in [
        (
            WorkCharge::new(MAX_WORK_BYTES, 0, 0, 0),
            SubmissionRefusal::Full,
        ),
        (
            WorkCharge::new(0, 0, 0, MAX_OPERATION_HANDLES + 1),
            SubmissionRefusal::Full,
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
    type Phase = MapBakePhase;

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
    type Phase = MapBakePhase;

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
    type Phase = MapBakePhase;

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

struct QualifiedProfileJob {
    clock: Arc<ManualClock>,
    stage: u8,
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
                self.clock.advance(Duration::from_millis(20));
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
fn a_declared_gather_decode_gets_only_its_first_prepare_allowance() {
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
                QualifiedProfileJob { clock, stage: 0 },
                &conditions,
                QualifiedUnit::GatheringFamilyDecode,
            )
            .unwrap_or_else(|refused| panic!("qualified profile refused: {:?}", refused.refusal()))
    };
    WorkService::wait_terminal(&shared, &ticket).expect("qualified job settles");
    assert!(matches!(
        shared.lock().consume(&ticket).expect("consume profile"),
        Settlement::Failed(JobError::Cooperation(CooperativeViolation::UnboundedSlice))
    ));
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
        shared.lock().consume(&process_ticket).expect("consume profile"),
        Settlement::Failed(JobError::Cooperation(CooperativeViolation::UnboundedSlice))
    ));
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
    type Phase = MapBakePhase;

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
        Err(WorkShutdownError::WorkerNotReturned)
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
