use super::ledger::{Slice, SliceOutcome};
use super::*;
use std::cell::Cell;
use std::num::{NonZeroU32, NonZeroU64, NonZeroUsize};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::identity::{
    Cancellation, Deadline, Fence, Notice, Notifier, ProcessIncarnation, ProcessTime,
    PublishSequence, RequestIdMinter, Scope, Settlement, WorkIdentity,
};
use crate::process_condition::{
    ConditionCause, ConditionId, ProcessConditions, ProcessServiceCode, ProcessServiceCondition,
};

fn bounds(jobs: usize, bytes: usize) -> AdmissionBounds {
    AdmissionBounds::new(
        NonZeroUsize::new(jobs).unwrap(),
        NonZeroUsize::new(bytes).unwrap(),
        NonZeroU32::new(8).unwrap(),
    )
}

fn bounds_full(jobs: usize, bytes: usize, handles: u32) -> AdmissionBounds {
    AdmissionBounds::new(
        NonZeroUsize::new(jobs).unwrap(),
        NonZeroUsize::new(bytes).unwrap(),
        NonZeroU32::new(handles).unwrap(),
    )
}

fn incarnation(n: u64) -> ProcessIncarnation {
    ProcessIncarnation::from_start_nonce(NonZeroU64::new(n).unwrap())
}

struct Ids {
    minter: RequestIdMinter,
}

impl Ids {
    fn new() -> Self {
        Self {
            minter: RequestIdMinter::new(),
        }
    }

    fn identity(&self, incarnation: ProcessIncarnation) -> WorkIdentity {
        WorkIdentity::new(
            self.minter.mint().unwrap(),
            incarnation,
            Scope::Process,
            None,
        )
        .unwrap()
    }
}

/// What a family spec derives the charge and revision from.
#[derive(Debug, Clone, Copy)]
struct TestInput {
    bytes: usize,
    scratch: usize,
    handles: u32,
    revision: u64,
}

fn input(bytes: usize) -> TestInput {
    TestInput {
        bytes,
        scratch: 0,
        handles: 0,
        revision: 0,
    }
}

fn input_handles(bytes: usize, handles: u32) -> TestInput {
    TestInput {
        handles,
        ..input(bytes)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
struct TestJobError(&'static str);

#[test]
fn contained_job_panic_has_honest_operator_text() {
    let error: JobError<TestJobError> = JobError::FailedInternally;
    assert_eq!(error.to_string(), "background work job failed internally");
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TestPhase {
    Search,
    Commit,
}

impl WorkPhase for TestPhase {
    fn index(self) -> u16 {
        match self {
            Self::Search => 0,
            Self::Commit => 1,
        }
    }
}

macro_rules! spec {
    ($name:ident, $family:expr) => {
        struct $name;

        impl WorkFamilySpec for $name {
            const FAMILY: WorkFamily = $family;
            type Input = TestInput;
            type Output = u32;
            type Error = TestJobError;
            type Phase = TestPhase;

            fn revision(input: &TestInput) -> InputRevision {
                InputRevision::new(input.revision)
            }

            fn charge(input: &TestInput) -> WorkCharge {
                WorkCharge::new(input.bytes, input.scratch, 0, input.handles)
            }

            fn map_cancellation(reason: Cancellation) -> Cancellation {
                reason
            }
        }
    };
}

spec!(RouteSpec, WorkFamily::Route);
spec!(BakeSpec, WorkFamily::MapBake);
spec!(InspectSpec, WorkFamily::Inspection);
spec!(CardSpec, WorkFamily::CardPreparation);

fn empty_conditions(incarnation: ProcessIncarnation) -> ProcessConditions {
    ProcessConditions::new(incarnation)
}

fn budget(now: Instant, units: u32, ms: u64) -> SliceBudget {
    SliceBudget::new(
        NonZeroU32::new(units).unwrap(),
        Deadline::after(now, Duration::from_millis(ms)),
    )
}

struct AdjClock {
    now: Cell<Instant>,
}

impl AdjClock {
    fn new(now: Instant) -> Self {
        Self {
            now: Cell::new(now),
        }
    }

    fn advance(&self, d: Duration) {
        self.now.set(self.now.get() + d);
    }
}

impl MonotonicClock for AdjClock {
    fn now(&self) -> Instant {
        self.now.get()
    }
}

/// Units the job in custody produced, observable after custody transfer.
type Log = Arc<Mutex<Vec<u32>>>;

fn log() -> Log {
    Arc::new(Mutex::new(Vec::new()))
}

fn produced(log: &Log) -> Vec<u32> {
    log.lock().unwrap().clone()
}

#[derive(Debug)]
struct CountJob {
    next: u32,
    target: u32,
    log: Log,
    forbidden: Option<ForbiddenRetain>,
    retained_bytes: usize,
    ignore_yield: bool,
    ignore_cancel: bool,
    multi: bool,
    report: bool,
}

impl CountJob {
    fn new(target: u32) -> Self {
        Self {
            next: 0,
            target,
            log: log(),
            forbidden: None,
            retained_bytes: 0,
            ignore_yield: false,
            ignore_cancel: false,
            multi: false,
            report: false,
        }
    }

    fn logged(mut self, log: &Log) -> Self {
        self.log = Arc::clone(log);
        self
    }
}

impl CooperativeJob for CountJob {
    type Output = u32;
    type Error = TestJobError;
    type Phase = TestPhase;

    fn step(&mut self, ctx: &mut StepContext<'_, TestPhase>) -> StepOutcome<u32, TestJobError> {
        match ctx.checkpoint() {
            Checkpoint::Cancel(_) if !self.ignore_cancel => return StepOutcome::Yielded,
            Checkpoint::Yield if !self.ignore_yield => return StepOutcome::Yielded,
            Checkpoint::Cancel(_) | Checkpoint::Yield | Checkpoint::Continue => {}
        }
        loop {
            let after = ctx.consume_unit();
            self.log.lock().unwrap().push(self.next);
            self.next += 1;
            if self.report {
                // An earlier report in the same slice is replaced.
                ctx.publish_progress(TestPhase::Search, 0, None);
                ctx.publish_progress(TestPhase::Commit, self.next, Some(self.target));
            }
            if self.next >= self.target {
                return StepOutcome::Completed(self.next);
            }
            match after {
                Checkpoint::Cancel(_) if !self.ignore_cancel => return StepOutcome::Yielded,
                Checkpoint::Yield if !self.ignore_yield => return StepOutcome::Yielded,
                Checkpoint::Continue if self.multi => continue,
                Checkpoint::Continue | Checkpoint::Yield | Checkpoint::Cancel(_) => {
                    if self.ignore_yield {
                        let _ = ctx.consume_unit();
                    }
                    return StepOutcome::Yielded;
                }
            }
        }
    }

    fn retained(&self) -> RetainedState {
        match self.forbidden {
            Some(kind) => RetainedState::forbidden(kind),
            None => RetainedState::clean(self.retained_bytes),
        }
    }
}

fn submit<F>(
    ledger: &mut AdmissionLedger,
    identity: WorkIdentity,
    conditions: &ProcessConditions,
    input: TestInput,
    job: CountJob,
) -> Result<WorkTicket<F>, Refused<CountJob>>
where
    F: WorkFamilySpec<Input = TestInput, Output = u32, Error = TestJobError, Phase = TestPhase>,
{
    ledger.try_submit::<F, CountJob>(identity, &input, job, conditions)
}

fn settle_route(
    ledger: &mut AdmissionLedger,
    ticket: &WorkTicket<RouteSpec>,
) -> Settlement<u32, JobError<TestJobError>> {
    ledger.worker_returned(ticket.identity()).unwrap();
    ledger.consume(ticket).unwrap()
}

#[test]
fn every_family_has_typed_owners_phases_and_terminals() {
    assert_eq!(WorkFamily::ALL.len(), 8);
    for family in WorkFamily::ALL {
        assert!(!family.producer().label().is_empty());
        assert!(!family.consumers().is_empty());
        assert!(!family.interruptible_unit().is_empty());
        assert!(!family.phases().is_empty());
        assert!(!family.result_name().is_empty());
        assert!(!family.error_name().is_empty());
        assert!(!family.cancellation_mapping().is_empty());
        assert!(!family.charge_rule().is_empty());
        assert!(!family.revision_rule().is_empty());
    }
    assert_eq!(WorkFamily::Route.producer(), Track::T5);
    assert_eq!(WorkFamily::Route.consumers(), &[Track::T1, Track::T2]);
    assert_eq!(
        WorkFamily::MapBake.phases(),
        &[
            "ReadingCache",
            "DerivingPois",
            "BakingPlane",
            "BuildingZoomLevels",
            "Publishing",
        ]
    );
    assert_eq!(MapBakePhase::ALL.len(), 5);
    for (expected, phase) in (0u16..).zip(MapBakePhase::ALL) {
        assert_eq!(WorkPhase::index(phase), expected);
    }
    assert_eq!(MapBakeStatus::ALL.len(), 6);
    assert!(WorkFamily::MapBake
        .cancellation_mapping()
        .contains("MapJobStatus::Paused"));
    assert_eq!(
        MapBakeStatus::cancellation_observed_as(),
        MapBakeStatus::Paused
    );
}

#[test]
fn multi_unit_slice_yields_exactly_at_allowance() {
    let cancel = SharedCancellation::new();
    let now = Instant::now();
    let clock = AdjClock::new(now);
    let mut ctx = StepContext::<TestPhase>::new(&clock, budget(now, 3, 50), &cancel);
    assert_eq!(ctx.checkpoint(), Checkpoint::Continue);
    assert_eq!(ctx.consume_unit(), Checkpoint::Continue);
    assert_eq!(ctx.consume_unit(), Checkpoint::Continue);
    assert_eq!(ctx.consume_unit(), Checkpoint::Yield);
    assert_eq!(ctx.units_this_slice(), 3);
    assert!(!ctx.exceeded_allowance());
}

#[test]
fn deadline_expires_mid_slice_under_a_fake_clock() {
    let cancel = SharedCancellation::new();
    let now = Instant::now();
    let clock = AdjClock::new(now);
    let mut ctx = StepContext::<TestPhase>::new(&clock, budget(now, 8, 50), &cancel);
    assert_eq!(ctx.consume_unit(), Checkpoint::Continue);
    clock.advance(Duration::from_millis(80));
    assert_eq!(ctx.checkpoint(), Checkpoint::Yield);
}

#[test]
fn system_clock_is_a_live_clock_not_a_snapshot() {
    let clock = SystemClock;
    let first = clock.now();
    assert!(clock.now() >= first);
    // A deadline already in the past expires at the first checkpoint even
    // though the budget was built from an earlier instant.
    let cancel = SharedCancellation::new();
    let mut ctx = StepContext::<TestPhase>::new(
        &clock,
        SliceBudget::new(NonZeroU32::new(8).unwrap(), Deadline::at(first)),
        &cancel,
    );
    assert_eq!(ctx.checkpoint(), Checkpoint::Yield);
}

#[test]
fn cancellation_registered_between_units_is_observed() {
    let cancel = SharedCancellation::new();
    let now = Instant::now();
    let clock = AdjClock::new(now);
    let mut ctx = StepContext::<TestPhase>::new(&clock, budget(now, 8, 50), &cancel);
    assert_eq!(ctx.consume_unit(), Checkpoint::Continue);
    cancel.register(Cancellation::Operator).unwrap();
    assert_eq!(ctx.checkpoint(), Checkpoint::Cancel(Cancellation::Operator));
    assert_eq!(
        cancel.register(Cancellation::Shutdown),
        Err(Cancellation::Operator),
        "the first registration stands"
    );
    assert_eq!(ctx.checkpoint(), Checkpoint::Cancel(Cancellation::Operator));
}

#[test]
fn compliant_multi_unit_slice_is_accepted_by_the_ledger() {
    let ids = Ids::new();
    let incarnation = incarnation(1);
    let conditions = empty_conditions(incarnation);
    let mut ledger = AdmissionLedger::new(incarnation, bounds(4, 1024));
    let log = log();
    let mut job = CountJob::new(10).logged(&log);
    job.multi = true;
    let ticket: WorkTicket<RouteSpec> = submit(
        &mut ledger,
        ids.identity(incarnation),
        &conditions,
        input(16),
        job,
    )
    .unwrap();
    let now = Instant::now();
    let clock = AdjClock::new(now);
    assert_eq!(
        ledger
            .drive(ticket.identity(), &clock, budget(now, 3, 50))
            .unwrap(),
        Some(JobState::Yielded)
    );
    assert_eq!(produced(&log), vec![0, 1, 2]);
}

#[test]
fn yield_resume_preserves_the_cursor_and_does_not_repeat_a_unit() {
    let ids = Ids::new();
    let incarnation = incarnation(1);
    let conditions = empty_conditions(incarnation);
    let mut ledger = AdmissionLedger::new(incarnation, bounds(4, 1024));
    let log = log();
    let ticket: WorkTicket<RouteSpec> = submit(
        &mut ledger,
        ids.identity(incarnation),
        &conditions,
        input(16),
        CountJob::new(3).logged(&log),
    )
    .unwrap();
    let now = Instant::now();
    let clock = AdjClock::new(now);
    let slice = budget(now, 1, 50);
    assert_eq!(
        ledger.drive(ticket.identity(), &clock, slice).unwrap(),
        Some(JobState::Yielded)
    );
    assert_eq!(produced(&log), vec![0]);
    assert_eq!(
        ledger.drive(ticket.identity(), &clock, slice).unwrap(),
        Some(JobState::Yielded)
    );
    assert_eq!(produced(&log), vec![0, 1]);
    assert_eq!(
        ledger.drive(ticket.identity(), &clock, slice).unwrap(),
        None
    );
    assert_eq!(produced(&log), vec![0, 1, 2]);
    match settle_route(&mut ledger, &ticket) {
        Settlement::Completed(3) => {}
        other => panic!("expected Completed(3), got {other:?}"),
    }
}

#[test]
fn checkpoint_honours_yield_before_success_and_cancellation() {
    let ids = Ids::new();
    let incarnation = incarnation(1);
    let conditions = empty_conditions(incarnation);
    let mut ledger = AdmissionLedger::new(incarnation, bounds(2, 1024));
    let log = log();
    let ticket: WorkTicket<BakeSpec> = submit(
        &mut ledger,
        ids.identity(incarnation),
        &conditions,
        input(8),
        CountJob::new(2).logged(&log),
    )
    .unwrap();
    let now = Instant::now();
    let expired = budget(now, 1, 0);
    let later = AdjClock::new(now + Duration::from_millis(1));
    assert_eq!(
        ledger.drive(ticket.identity(), &later, expired).unwrap(),
        Some(JobState::Yielded)
    );
    assert!(produced(&log).is_empty());
    assert_eq!(
        ledger.cancel(ticket, Cancellation::Operator),
        CancelOutcome::Requested
    );
    let clock = AdjClock::new(now);
    assert_eq!(
        ledger
            .drive(ticket.identity(), &clock, budget(now, 1, 50))
            .unwrap(),
        None
    );
    ledger.worker_returned(ticket.identity()).unwrap();
    match ledger.consume(&ticket).unwrap() {
        Settlement::Cancelled(Cancellation::Operator) => {}
        other => panic!("expected Cancelled, got {other:?}"),
    }
}

#[test]
fn cancellation_before_publication_wins_and_publication_before_cancellation_sticks() {
    let ids = Ids::new();
    let incarnation = incarnation(1);
    let conditions = empty_conditions(incarnation);
    let mut ledger = AdmissionLedger::new(incarnation, bounds(2, 1024));
    let now = Instant::now();
    let clock = AdjClock::new(now);

    let mut late = CountJob::new(1);
    late.ignore_cancel = true;
    let ticket: WorkTicket<RouteSpec> = submit(
        &mut ledger,
        ids.identity(incarnation),
        &conditions,
        input(8),
        late,
    )
    .unwrap();
    assert_eq!(
        ledger.cancel(ticket, Cancellation::Operator),
        CancelOutcome::Requested
    );
    assert_eq!(
        ledger.cancel(ticket, Cancellation::Shutdown),
        CancelOutcome::Requested,
        "cancel is idempotent"
    );
    assert_eq!(ledger.cancellation(&ticket), Some(Cancellation::Operator));
    assert_eq!(
        ledger
            .drive(ticket.identity(), &clock, budget(now, 1, 50))
            .unwrap(),
        None
    );
    match settle_route(&mut ledger, &ticket) {
        Settlement::Cancelled(Cancellation::Operator) => {}
        other => panic!("late success must not publish, got {other:?}"),
    }

    let ticket: WorkTicket<RouteSpec> = submit(
        &mut ledger,
        ids.identity(incarnation),
        &conditions,
        input(8),
        CountJob::new(1),
    )
    .unwrap();
    assert_eq!(
        ledger
            .drive(ticket.identity(), &clock, budget(now, 1, 50))
            .unwrap(),
        None
    );
    assert_eq!(
        ledger.cancel(ticket, Cancellation::Operator),
        CancelOutcome::AlreadyTerminal
    );
    match settle_route(&mut ledger, &ticket) {
        Settlement::Completed(1) => {}
        other => panic!("already-published completion must stick, got {other:?}"),
    }
}

#[test]
fn cancelled_job_never_publishes_a_result() {
    let ids = Ids::new();
    let incarnation = incarnation(1);
    let conditions = empty_conditions(incarnation);
    let mut ledger = AdmissionLedger::new(incarnation, bounds(1, 1024));
    let mut job = CountJob::new(1);
    job.ignore_cancel = true;
    let ticket: WorkTicket<InspectSpec> = submit(
        &mut ledger,
        ids.identity(incarnation),
        &conditions,
        input(8),
        job,
    )
    .unwrap();
    ledger.cancel(ticket, Cancellation::Shutdown);
    assert_eq!(ledger.cancellation(&ticket), Some(Cancellation::Shutdown));
    let now = Instant::now();
    let clock = AdjClock::new(now);
    ledger
        .drive(ticket.identity(), &clock, budget(now, 1, 50))
        .unwrap();
    ledger.worker_returned(ticket.identity()).unwrap();
    match ledger.borrow_terminal(&ticket).unwrap() {
        Some(Settlement::Cancelled(Cancellation::Shutdown)) => {}
        other => panic!("expected Cancelled(Shutdown), got {other:?}"),
    }
}

#[test]
fn a_job_that_holds_forbidden_state_across_a_yield_is_rejected() {
    let ids = Ids::new();
    let incarnation = incarnation(1);
    let conditions = empty_conditions(incarnation);
    let mut ledger = AdmissionLedger::new(incarnation, bounds(1, 1024));
    let mut job = CountJob::new(4);
    job.forbidden = Some(ForbiddenRetain::OtherOwnerResource);
    let ticket: WorkTicket<CardSpec> = submit(
        &mut ledger,
        ids.identity(incarnation),
        &conditions,
        input(8),
        job,
    )
    .unwrap();
    let now = Instant::now();
    let clock = AdjClock::new(now);
    assert_eq!(
        ledger
            .drive(ticket.identity(), &clock, budget(now, 1, 50))
            .unwrap(),
        None
    );
    ledger.worker_returned(ticket.identity()).unwrap();
    match ledger.consume(&ticket).unwrap() {
        Settlement::Failed(JobError::Cooperation(CooperativeViolation::ForbiddenRetain(
            ForbiddenRetain::OtherOwnerResource,
        ))) => {}
        other => panic!("expected forbidden-retain failure, got {other:?}"),
    }
}

#[test]
fn ignoring_yield_fails_the_job_with_ignored_yield() {
    let ids = Ids::new();
    let incarnation = incarnation(1);
    let conditions = empty_conditions(incarnation);
    let mut ledger = AdmissionLedger::new(incarnation, bounds(1, 1024));
    let mut job = CountJob::new(8);
    job.ignore_yield = true;
    let ticket: WorkTicket<RouteSpec> = submit(
        &mut ledger,
        ids.identity(incarnation),
        &conditions,
        input(8),
        job,
    )
    .unwrap();
    let now = Instant::now();
    let clock = AdjClock::new(now);
    assert_eq!(
        ledger
            .drive(ticket.identity(), &clock, budget(now, 1, 50))
            .unwrap(),
        None
    );
    match settle_route(&mut ledger, &ticket) {
        Settlement::Failed(JobError::Cooperation(CooperativeViolation::IgnoredYield)) => {}
        other => panic!("expected IgnoredYield, got {other:?}"),
    }
}

#[test]
fn retaining_more_than_the_charge_fails_with_charge_exceeded() {
    let ids = Ids::new();
    let incarnation = incarnation(1);
    let conditions = empty_conditions(incarnation);
    let mut ledger = AdmissionLedger::new(incarnation, bounds(1, 1024));
    let mut job = CountJob::new(4);
    job.retained_bytes = 9;
    let ticket: WorkTicket<RouteSpec> = submit(
        &mut ledger,
        ids.identity(incarnation),
        &conditions,
        input(8),
        job,
    )
    .unwrap();
    let now = Instant::now();
    let clock = AdjClock::new(now);
    assert_eq!(
        ledger
            .drive(ticket.identity(), &clock, budget(now, 1, 50))
            .unwrap(),
        None,
        "a yield that retains 9 bytes against an 8-byte charge is a terminal"
    );
    match settle_route(&mut ledger, &ticket) {
        Settlement::Failed(JobError::Cooperation(CooperativeViolation::ChargeExceeded)) => {}
        other => panic!("expected ChargeExceeded, got {other:?}"),
    }
}

#[test]
fn wrong_scope_revision_and_duplicate_consume_cannot_install_or_reclaim() {
    let ids = Ids::new();
    let incarnation = incarnation(1);
    let conditions = empty_conditions(incarnation);
    let mut ledger = AdmissionLedger::new(incarnation, bounds(2, 1024));
    let id = ids.identity(incarnation);
    let ticket: WorkTicket<RouteSpec> =
        submit(&mut ledger, id, &conditions, input(8), CountJob::new(1)).unwrap();
    let mut fence = Fence::new();
    fence.arm(ticket);
    let stale = WorkTicket::<RouteSpec>::new(id, InputRevision::new(1));
    assert!(fence.admit(stale).is_err());
    assert_eq!(
        ledger.cancel(stale, Cancellation::Operator),
        CancelOutcome::StaleIdentity
    );

    let now = Instant::now();
    let clock = AdjClock::new(now);
    ledger
        .drive(ticket.identity(), &clock, budget(now, 1, 50))
        .unwrap();
    settle_route(&mut ledger, &ticket);
    match ledger.consume(&ticket) {
        Err(LedgerError::AlreadyConsumed | LedgerError::StaleIdentity) => {}
        other => panic!("duplicate consume must not reclaim another ticket, got {other:?}"),
    }
    assert!(!ledger.is_charged(&ticket));
}

#[test]
fn a_ticket_of_another_family_is_stale_for_the_same_job() {
    let ids = Ids::new();
    let incarnation = incarnation(1);
    let conditions = empty_conditions(incarnation);
    let mut ledger = AdmissionLedger::new(incarnation, bounds(2, 1024));
    let id = ids.identity(incarnation);
    let route: WorkTicket<RouteSpec> =
        submit(&mut ledger, id, &conditions, input(8), CountJob::new(1)).unwrap();
    let now = Instant::now();
    let clock = AdjClock::new(now);
    ledger.drive(id, &clock, budget(now, 1, 50)).unwrap();
    let forged = WorkTicket::<BakeSpec>::new(id, InputRevision::FIRST);
    assert_eq!(forged.family(), WorkFamily::MapBake);
    assert_eq!(
        ledger.cancel(forged, Cancellation::Operator),
        CancelOutcome::StaleIdentity
    );
    assert_eq!(
        ledger.consume(&forged).unwrap_err(),
        LedgerError::StaleIdentity
    );
    assert!(ledger.borrow_terminal(&route).unwrap().is_some());
    assert!(ledger.is_charged(&route));
}

#[test]
fn saturated_ledger_returns_full_and_hands_the_job_back() {
    let ids = Ids::new();
    let incarnation = incarnation(1);
    let conditions = empty_conditions(incarnation);
    let mut ledger = AdmissionLedger::new(incarnation, bounds(1, 32));
    let first = ids.identity(incarnation);
    let first_ticket: WorkTicket<RouteSpec> =
        submit(&mut ledger, first, &conditions, input(16), CountJob::new(1)).unwrap();
    let second = ids.identity(incarnation);
    let log = log();
    let mut job = CountJob::new(7).logged(&log);
    job.multi = true;
    let refused = submit::<RouteSpec>(&mut ledger, second, &conditions, input(16), job)
        .expect_err("expected Full");
    assert_eq!(refused.refusal(), SubmissionRefusal::Full);
    assert_eq!(refused.job().target, 7);
    let (refusal, mut job) = refused.into_parts();
    assert_eq!(refusal, SubmissionRefusal::Full);
    assert!(ledger.borrow_terminal(&first_ticket).unwrap().is_none());
    assert_eq!(ledger.charged_jobs(), 1);
    assert!(!ledger.is_charged(&WorkTicket::<RouteSpec>::new(second, InputRevision::FIRST)));
    // The caller still owns the job: it can run it elsewhere later.
    let cancel = SharedCancellation::new();
    let now = Instant::now();
    let clock = AdjClock::new(now);
    let mut ctx = StepContext::<TestPhase>::new(&clock, budget(now, 8, 50), &cancel);
    assert_eq!(job.step(&mut ctx), StepOutcome::Completed(7));
    assert_eq!(produced(&log).len(), 7);
}

#[test]
fn byte_and_handle_caps_return_full() {
    let ids = Ids::new();
    let incarnation = incarnation(1);
    let conditions = empty_conditions(incarnation);
    let mut bytes = AdmissionLedger::new(incarnation, bounds_full(8, 16, 8));
    submit::<CardSpec>(
        &mut bytes,
        ids.identity(incarnation),
        &conditions,
        input(16),
        CountJob::new(1),
    )
    .unwrap();
    match submit::<CardSpec>(
        &mut bytes,
        ids.identity(incarnation),
        &conditions,
        input(1),
        CountJob::new(1),
    ) {
        Err(refused) if refused.refusal() == SubmissionRefusal::Full => {}
        other => panic!("expected byte Full, got {other:?}"),
    }

    let mut handles = AdmissionLedger::new(incarnation, bounds_full(8, 1024, 1));
    submit::<RouteSpec>(
        &mut handles,
        ids.identity(incarnation),
        &conditions,
        input_handles(8, 1),
        CountJob::new(1),
    )
    .unwrap();
    match submit::<RouteSpec>(
        &mut handles,
        ids.identity(incarnation),
        &conditions,
        input_handles(8, 1),
        CountJob::new(1),
    ) {
        Err(refused) if refused.refusal() == SubmissionRefusal::Full => {}
        other => panic!("expected handle Full, got {other:?}"),
    }
}

#[test]
fn declared_navigation_custody_does_not_consume_background_bytes() {
    let ids = Ids::new();
    let incarnation = incarnation(1);
    let conditions = empty_conditions(incarnation);
    let mut ledger = AdmissionLedger::new(incarnation, bounds_full(16, 16, 8));
    for _ in 0..10 {
        submit::<RouteSpec>(
            &mut ledger,
            ids.identity(incarnation),
            &conditions,
            input(1024),
            CountJob::new(1),
        )
        .unwrap();
    }
    submit::<CardSpec>(
        &mut ledger,
        ids.identity(incarnation),
        &conditions,
        input(16),
        CountJob::new(1),
    )
    .unwrap();
    let refused = submit::<CardSpec>(
        &mut ledger,
        ids.identity(incarnation),
        &conditions,
        input(1),
        CountJob::new(1),
    )
    .expect_err("the background byte allowance still applies");
    assert_eq!(refused.refusal(), SubmissionRefusal::Full);
    assert_eq!(ledger.charged_jobs(), 11);
    assert_eq!(ledger.charged_bytes(), 10 * 1024 + 16);
}

#[test]
fn an_unrepresentable_charge_is_invalid_argument_not_saturated() {
    let ids = Ids::new();
    let incarnation = incarnation(1);
    let conditions = empty_conditions(incarnation);
    let mut ledger = AdmissionLedger::new(incarnation, bounds(8, usize::MAX));
    let overflow = TestInput {
        bytes: usize::MAX,
        scratch: 1,
        handles: 0,
        revision: 0,
    };
    assert_eq!(WorkCharge::new(usize::MAX, 1, 0, 0).total_bytes(), None);
    match submit::<RouteSpec>(
        &mut ledger,
        ids.identity(incarnation),
        &conditions,
        overflow,
        CountJob::new(1),
    ) {
        Err(refused) if refused.refusal() == SubmissionRefusal::InvalidArgument => {}
        other => panic!("expected InvalidArgument, got {other:?}"),
    }
    assert_eq!(ledger.charged_jobs(), 0);
    // Representable alone, unrepresentable summed with the ledger.
    submit::<RouteSpec>(
        &mut ledger,
        ids.identity(incarnation),
        &conditions,
        input(usize::MAX),
        CountJob::new(1),
    )
    .unwrap();
    match submit::<RouteSpec>(
        &mut ledger,
        ids.identity(incarnation),
        &conditions,
        input(1),
        CountJob::new(1),
    ) {
        Err(refused) if refused.refusal() == SubmissionRefusal::InvalidArgument => {}
        other => panic!("expected InvalidArgument on ledger overflow, got {other:?}"),
    }
}

#[test]
fn unconsumed_terminal_holds_capacity() {
    let ids = Ids::new();
    let incarnation = incarnation(1);
    let conditions = empty_conditions(incarnation);
    let mut ledger = AdmissionLedger::new(incarnation, bounds(1, 1024));
    let ticket: WorkTicket<RouteSpec> = submit(
        &mut ledger,
        ids.identity(incarnation),
        &conditions,
        input(8),
        CountJob::new(1),
    )
    .unwrap();
    let now = Instant::now();
    let clock = AdjClock::new(now);
    ledger
        .drive(ticket.identity(), &clock, budget(now, 1, 50))
        .unwrap();
    ledger.worker_returned(ticket.identity()).unwrap();
    match submit::<RouteSpec>(
        &mut ledger,
        ids.identity(incarnation),
        &conditions,
        input(8),
        CountJob::new(1),
    ) {
        Err(refused) if refused.refusal() == SubmissionRefusal::Full => {}
        other => panic!("unconsumed terminal must hold the job slot, got {other:?}"),
    }
    ledger.consume(&ticket).unwrap();
    submit::<RouteSpec>(
        &mut ledger,
        ids.identity(incarnation),
        &conditions,
        input(8),
        CountJob::new(1),
    )
    .unwrap();
}

#[test]
fn released_job_id_is_not_reused() {
    let ids = Ids::new();
    let incarnation = incarnation(1);
    let conditions = empty_conditions(incarnation);
    let mut ledger = AdmissionLedger::new(incarnation, bounds(2, 1024));
    let identity = ids.identity(incarnation);
    let ticket: WorkTicket<RouteSpec> = submit(
        &mut ledger,
        identity,
        &conditions,
        input(8),
        CountJob::new(1),
    )
    .unwrap();
    let now = Instant::now();
    let clock = AdjClock::new(now);
    ledger
        .drive(ticket.identity(), &clock, budget(now, 1, 50))
        .unwrap();
    settle_route(&mut ledger, &ticket);
    match submit::<RouteSpec>(
        &mut ledger,
        identity,
        &conditions,
        input(8),
        CountJob::new(1),
    ) {
        Err(refused) if refused.refusal() == SubmissionRefusal::InvalidArgument => {}
        other => panic!("released job id must not be reused, got {other:?}"),
    }
}

#[test]
fn abandoned_ticket_retains_credit_until_the_worker_returns() {
    let ids = Ids::new();
    let incarnation = incarnation(1);
    let conditions = empty_conditions(incarnation);
    let mut ledger = AdmissionLedger::new(incarnation, bounds(2, 1024));
    let ticket: WorkTicket<RouteSpec> = submit(
        &mut ledger,
        ids.identity(incarnation),
        &conditions,
        input(32),
        CountJob::new(1),
    )
    .unwrap();
    assert_eq!(ledger.charged_bytes(), 32);
    assert_eq!(
        ledger.abandon(ticket, Cancellation::Operator),
        CancelOutcome::Requested
    );
    assert!(
        ledger.is_charged(&ticket),
        "abandon must not free credit before the worker returns"
    );
    assert_eq!(ledger.charged_bytes(), 32);
    ledger.worker_returned(ticket.identity()).unwrap();
    assert!(
        !ledger.is_charged(&ticket),
        "credit frees only after the worker returns"
    );
    assert_eq!(ledger.charged_bytes(), 0);
}

#[test]
fn progress_is_stamped_by_the_service_and_never_takes_terminal_custody() {
    let ids = Ids::new();
    let incarnation = incarnation(1);
    let conditions = empty_conditions(incarnation);
    let mut ledger = AdmissionLedger::new(incarnation, bounds(1, 1024));
    let id = ids.identity(incarnation);
    let mut job = CountJob::new(2);
    job.report = true;
    let ticket: WorkTicket<BakeSpec> = submit(&mut ledger, id, &conditions, input(8), job).unwrap();
    assert!(ledger.progress(&ticket).is_none());
    let now = Instant::now();
    let clock = AdjClock::new(now);
    ledger
        .drive(ticket.identity(), &clock, budget(now, 1, 50))
        .unwrap();
    let first = ledger.progress(&ticket).unwrap().clone();
    assert_eq!(first.identity(), id);
    assert_eq!(first.sequence(), 1);
    assert_eq!(
        first.phase(),
        TestPhase::Commit.index(),
        "the later report of a slice replaces the earlier one"
    );
    assert_eq!(first.completed(), 1);
    assert_eq!(first.total(), Some(2));
    ledger
        .drive(ticket.identity(), &clock, budget(now, 1, 50))
        .unwrap();
    let second = ledger.progress(&ticket).unwrap();
    assert_eq!(second.sequence(), 2);
    assert_eq!(second.completed(), 2);
    ledger.worker_returned(ticket.identity()).unwrap();
    match ledger.consume(&ticket).unwrap() {
        Settlement::Completed(2) => {}
        other => panic!("progress must not steal the terminal, got {other:?}"),
    }
}

#[test]
fn unavailable_condition_refuses_submit_and_keeps_identity() {
    let ids = Ids::new();
    let incarnation = incarnation(1);
    let mut conditions = ProcessConditions::new(incarnation);
    conditions
        .publish(ProcessServiceCondition::new(
            ConditionId::new(incarnation, ProcessServiceCode::WorkAdmissionUnavailable),
            ConditionCause::ProgressLost,
            ProcessTime::ZERO,
            None,
        ))
        .unwrap();
    let mut ledger = AdmissionLedger::new(incarnation, bounds(2, 1024));
    let id = ids.identity(incarnation);
    match submit::<RouteSpec>(&mut ledger, id, &conditions, input(8), CountJob::new(3)) {
        Err(refused) => match refused.refusal() {
            SubmissionRefusal::Unavailable(refusal) => {
                assert_eq!(refusal.request(), Some(id.job()));
                assert_eq!(refusal.scope(), Scope::Process);
                assert_eq!(
                    refusal.condition().code(),
                    ProcessServiceCode::WorkAdmissionUnavailable
                );
                assert_eq!(refused.into_job().target, 3, "the job comes back");
            }
            other => panic!("expected Unavailable, got {other:?}"),
        },
        Ok(ticket) => panic!("expected Unavailable, got {ticket:?}"),
    }
    assert_eq!(ledger.charged_jobs(), 0);
}

#[test]
fn isolate_request_ids_are_not_work_ids() {
    use crate::identity::{IdentityError, RequestId, RequestNamespace};
    use crate::selected::RunKey;

    let isolate = RequestId::isolate(1).unwrap();
    assert_eq!(isolate.namespace(), RequestNamespace::Isolate);
    let run = RunKey {
        slot: 1,
        run: 1,
        session: 0,
    };
    match WorkIdentity::new(isolate, incarnation(1), Scope::Run(run), None) {
        Err(IdentityError::JobIdNotHostMinted { .. }) => {}
        other => panic!("expected JobIdNotHostMinted, got {other:?}"),
    }
}

#[test]
fn foreign_incarnation_is_invalid_authority() {
    let conditions = empty_conditions(incarnation(1));
    let mut ledger = AdmissionLedger::new(incarnation(1), bounds(1, 1024));
    let id = Ids::new().identity(incarnation(2));
    match submit::<RouteSpec>(&mut ledger, id, &conditions, input(8), CountJob::new(1)) {
        Err(refused) if refused.refusal() == SubmissionRefusal::InvalidAuthority => {}
        other => panic!("expected InvalidAuthority, got {other:?}"),
    }
    // A conditions ledger of another incarnation is no authority either.
    let foreign = empty_conditions(incarnation(2));
    let id = Ids::new().identity(incarnation(1));
    match submit::<RouteSpec>(&mut ledger, id, &foreign, input(8), CountJob::new(1)) {
        Err(refused) if refused.refusal() == SubmissionRefusal::InvalidAuthority => {}
        other => panic!("expected InvalidAuthority for foreign conditions, got {other:?}"),
    }
}

#[test]
fn stopping_refuses_new_work() {
    let ids = Ids::new();
    let incarnation = incarnation(1);
    let conditions = empty_conditions(incarnation);
    let mut ledger = AdmissionLedger::new(incarnation, bounds(1, 1024));
    ledger.begin_stop();
    match submit::<RouteSpec>(
        &mut ledger,
        ids.identity(incarnation),
        &conditions,
        input(8),
        CountJob::new(1),
    ) {
        Err(refused) if refused.refusal() == SubmissionRefusal::Stopping => {}
        other => panic!("expected Stopping, got {other:?}"),
    }
}

#[test]
fn charge_and_revision_come_from_the_spec() {
    let ids = Ids::new();
    let incarnation = incarnation(1);
    let conditions = empty_conditions(incarnation);
    let mut ledger = AdmissionLedger::new(incarnation, bounds(2, 1024));
    let id = ids.identity(incarnation);
    let spec_input = TestInput {
        bytes: 24,
        scratch: 8,
        handles: 2,
        revision: 5,
    };
    let ticket: WorkTicket<RouteSpec> =
        submit(&mut ledger, id, &conditions, spec_input, CountJob::new(1)).unwrap();
    assert_eq!(ticket.identity(), id);
    assert_eq!(ticket.family(), WorkFamily::Route);
    assert_eq!(ticket.revision(), RouteSpec::revision(&spec_input));
    assert_eq!(ticket.revision(), InputRevision::new(5));
    assert_eq!(
        ledger.charged_bytes(),
        RouteSpec::charge(&spec_input).total_bytes().unwrap()
    );
    assert_eq!(ledger.charged_bytes(), 32);
    assert_eq!(ledger.charged_handles(), 2);
    assert_eq!(ledger.job_state(&ticket), Some(JobState::Queued));
}

#[test]
fn yielded_charges_survive_until_consume_and_worker_return() {
    let ids = Ids::new();
    let incarnation = incarnation(1);
    let conditions = empty_conditions(incarnation);
    let mut ledger = AdmissionLedger::new(incarnation, bounds(2, 1024));
    let ticket: WorkTicket<RouteSpec> = submit(
        &mut ledger,
        ids.identity(incarnation),
        &conditions,
        input(24),
        CountJob::new(2),
    )
    .unwrap();
    let now = Instant::now();
    let clock = AdjClock::new(now);
    ledger
        .drive(ticket.identity(), &clock, budget(now, 1, 50))
        .unwrap();
    assert_eq!(ledger.job_state(&ticket), Some(JobState::Yielded));
    assert_eq!(ledger.charged_bytes(), 24);
    ledger
        .drive(ticket.identity(), &clock, budget(now, 1, 50))
        .unwrap();
    assert!(ledger.is_charged(&ticket));
    ledger.worker_returned(ticket.identity()).unwrap();
    assert!(ledger.is_charged(&ticket));
    ledger.consume(&ticket).unwrap();
    assert!(!ledger.is_charged(&ticket));
}

#[test]
fn consume_before_worker_return_keeps_credit_charged() {
    let ids = Ids::new();
    let incarnation = incarnation(1);
    let conditions = empty_conditions(incarnation);
    let mut ledger = AdmissionLedger::new(incarnation, bounds(2, 1024));
    let ticket: WorkTicket<RouteSpec> = submit(
        &mut ledger,
        ids.identity(incarnation),
        &conditions,
        input(24),
        CountJob::new(1),
    )
    .unwrap();
    let now = Instant::now();
    let clock = AdjClock::new(now);
    ledger
        .drive(ticket.identity(), &clock, budget(now, 1, 50))
        .unwrap();
    match ledger.consume(&ticket).unwrap() {
        Settlement::Completed(1) => {}
        other => panic!("expected Completed(1), got {other:?}"),
    }
    assert!(
        ledger.is_charged(&ticket),
        "a consumed terminal does not free credit before the worker returns"
    );
    assert_eq!(ledger.charged_bytes(), 24);
    assert_eq!(ledger.charged_jobs(), 1);
    ledger.worker_returned(ticket.identity()).unwrap();
    assert!(!ledger.is_charged(&ticket));
    assert_eq!(ledger.charged_bytes(), 0);
}

#[test]
fn fail_unavailable_publishes_and_retains_charge() {
    let ids = Ids::new();
    let incarnation = incarnation(1);
    let conditions = empty_conditions(incarnation);
    let mut ledger = AdmissionLedger::new(incarnation, bounds(2, 1024));
    let ticket: WorkTicket<RouteSpec> = submit(
        &mut ledger,
        ids.identity(incarnation),
        &conditions,
        input(16),
        CountJob::new(1),
    )
    .unwrap();
    let wrong = ConditionId::new(incarnation, ProcessServiceCode::ReactorUnavailable);
    match ledger.fail_unavailable(ticket.identity(), wrong) {
        Err(LedgerError::WrongCondition) => {}
        other => panic!("expected WrongCondition, got {other:?}"),
    }
    let condition = ConditionId::new(incarnation, ProcessServiceCode::WorkAdmissionUnavailable);
    ledger
        .fail_unavailable(ticket.identity(), condition)
        .unwrap();
    match ledger.fail_unavailable(ticket.identity(), condition) {
        Err(LedgerError::AlreadyTerminal) => {}
        other => panic!("second fail_unavailable must not rewrite, got {other:?}"),
    }
    assert!(ledger.is_charged(&ticket));
    let now = Instant::now();
    let clock = AdjClock::new(now);
    assert_eq!(
        ledger
            .drive(ticket.identity(), &clock, budget(now, 1, 50))
            .unwrap(),
        None
    );
    match ledger.borrow_terminal(&ticket).unwrap() {
        Some(Settlement::Failed(JobError::Unavailable(terminal))) => {
            assert_eq!(terminal.condition(), condition);
        }
        other => panic!("late completion must not replace Unavailable, got {other:?}"),
    }
    ledger.worker_returned(ticket.identity()).unwrap();
    assert!(ledger.is_charged(&ticket));
    match ledger.consume(&ticket).unwrap() {
        Settlement::Failed(JobError::Unavailable(_)) => {}
        other => panic!("expected Unavailable terminal, got {other:?}"),
    }
    assert!(!ledger.is_charged(&ticket));
}

#[test]
fn fail_unavailable_from_another_incarnation_is_wrong_condition() {
    let ids = Ids::new();
    let here = incarnation(1);
    let conditions = empty_conditions(here);
    let mut ledger = AdmissionLedger::new(here, bounds(2, 1024));
    let ticket: WorkTicket<RouteSpec> = submit(
        &mut ledger,
        ids.identity(here),
        &conditions,
        input(16),
        CountJob::new(1),
    )
    .unwrap();
    let foreign = ConditionId::new(incarnation(2), ProcessServiceCode::WorkAdmissionUnavailable);
    match ledger.fail_unavailable(ticket.identity(), foreign) {
        Err(LedgerError::WrongCondition) => {}
        other => panic!("expected WrongCondition for a foreign incarnation, got {other:?}"),
    }
    assert!(ledger.borrow_terminal(&ticket).unwrap().is_none());
    assert_eq!(ledger.job_state(&ticket), Some(JobState::Queued));
}

#[test]
fn cancel_then_outage_keeps_the_cancellation_terminal() {
    let ids = Ids::new();
    let incarnation = incarnation(1);
    let conditions = empty_conditions(incarnation);
    let mut ledger = AdmissionLedger::new(incarnation, bounds(2, 1024));
    let ticket: WorkTicket<RouteSpec> = submit(
        &mut ledger,
        ids.identity(incarnation),
        &conditions,
        input(16),
        CountJob::new(1),
    )
    .unwrap();
    assert_eq!(
        ledger.cancel(ticket, Cancellation::Operator),
        CancelOutcome::Requested
    );
    let condition = ConditionId::new(incarnation, ProcessServiceCode::WorkAdmissionUnavailable);
    match ledger.fail_unavailable(ticket.identity(), condition) {
        Err(LedgerError::AlreadyTerminal) => {}
        other => panic!("the cancellation selected first must win, got {other:?}"),
    }
    match ledger.borrow_terminal(&ticket).unwrap() {
        Some(Settlement::Cancelled(Cancellation::Operator)) => {}
        other => panic!("expected Cancelled(Operator), got {other:?}"),
    }
    assert!(ledger.is_charged(&ticket));
    ledger.worker_returned(ticket.identity()).unwrap();
    match ledger.consume(&ticket).unwrap() {
        Settlement::Cancelled(Cancellation::Operator) => {}
        other => panic!("expected Cancelled(Operator), got {other:?}"),
    }
    assert!(!ledger.is_charged(&ticket));

    // The other order: an outage selected first cannot be cancelled away.
    let ticket: WorkTicket<RouteSpec> = submit(
        &mut ledger,
        ids.identity(incarnation),
        &conditions,
        input(16),
        CountJob::new(1),
    )
    .unwrap();
    ledger
        .fail_unavailable(ticket.identity(), condition)
        .unwrap();
    assert_eq!(
        ledger.cancel(ticket, Cancellation::Operator),
        CancelOutcome::AlreadyTerminal
    );
    match ledger.borrow_terminal(&ticket).unwrap() {
        Some(Settlement::Failed(JobError::Unavailable(_))) => {}
        other => panic!("expected the Unavailable terminal to stand, got {other:?}"),
    }
}

/// What the notifier saw at the moment of each notice: the notice itself
/// and the three publications as read inside `notify`.
#[derive(Debug, Clone)]
struct Observed {
    notice: PublishSequence,
    capacity: (PublishSequence, WorkCapacity),
    progress: (PublishSequence, Option<WorkProgress>),
    terminal: (PublishSequence, Option<WorkIdentity>),
}

struct Reader {
    publications: Arc<WorkPublications>,
    seen: Mutex<Vec<Observed>>,
}

impl Notifier for Reader {
    fn notify(&self, notice: Notice) {
        assert_eq!(notice.scope(), Scope::Process);
        let observed = Observed {
            notice: notice.sequence(),
            capacity: self.publications.capacity().read(|seq, v| (seq, *v)),
            progress: self.publications.progress().read(|seq, v| (seq, v.clone())),
            terminal: self.publications.terminal().read(|seq, v| (seq, *v)),
        };
        self.seen.lock().unwrap().push(observed);
    }
}

/// What the worker wake saw: one entry per wake, holding what the watched
/// job's live cancellation source read inside `wake`.
#[derive(Default)]
struct WakeLog {
    watched: Mutex<Option<Arc<SharedCancellation>>>,
    wakes: Mutex<Vec<Option<Cancellation>>>,
}

impl WakeLog {
    fn watch(&self, source: Arc<SharedCancellation>) {
        *self.watched.lock().unwrap() = Some(source);
    }

    fn wakes(&self) -> Vec<Option<Cancellation>> {
        self.wakes.lock().unwrap().clone()
    }
}

impl WorkerWake for WakeLog {
    fn wake(&self) {
        let read = self
            .watched
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|source| source.cancellation());
        self.wakes.lock().unwrap().push(read);
    }
}

#[test]
fn capacity_progress_and_terminal_notify_after_publication() {
    let ids = Ids::new();
    let incarnation = incarnation(1);
    let conditions = empty_conditions(incarnation);
    let publications = Arc::new(WorkPublications::new(Scope::Process));
    let reader = Arc::new(Reader {
        publications: Arc::clone(&publications),
        seen: Mutex::new(Vec::new()),
    });
    let mut ledger = AdmissionLedger::with_notifier(
        incarnation,
        bounds(2, 1024),
        Arc::clone(&publications),
        Arc::clone(&reader) as Arc<dyn Notifier>,
        Arc::new(WakeLog::default()),
    );
    let id = ids.identity(incarnation);
    let mut job = CountJob::new(1);
    job.report = true;
    let ticket: WorkTicket<RouteSpec> =
        submit(&mut ledger, id, &conditions, input(8), job).unwrap();
    assert!(std::ptr::eq(ledger.publications(), &*publications));

    // 1. Submission: the capacity notice carries the sequence the reader saw
    //    inside notify, and the state it read already counted the job.
    {
        let seen = reader.seen.lock().unwrap();
        assert_eq!(seen.len(), 1, "submit publishes capacity once");
        let first = &seen[0];
        assert_eq!(first.notice, first.capacity.0);
        assert_eq!(first.capacity.0.get(), 1);
        assert_eq!(first.capacity.1.jobs(), 1);
        assert_eq!(first.capacity.1.bytes(), 8);
        assert!(first.progress.1.is_none());
        assert!(first.terminal.1.is_none());
    }

    // 2. One slice completes the job: progress is published (and noticed)
    //    before the terminal is published (and noticed).
    let now = Instant::now();
    let clock = AdjClock::new(now);
    assert_eq!(
        ledger
            .drive(ticket.identity(), &clock, budget(now, 1, 50))
            .unwrap(),
        None
    );
    {
        let seen = reader.seen.lock().unwrap();
        assert_eq!(seen.len(), 3, "drive publishes progress then terminal");
        let progress = &seen[1];
        assert_eq!(progress.notice, progress.progress.0);
        assert_eq!(progress.progress.0.get(), 1);
        let snapshot = progress
            .progress
            .1
            .as_ref()
            .expect("progress readable in notify");
        assert_eq!(snapshot.identity(), id);
        assert_eq!(snapshot.sequence(), 1);
        assert_eq!(snapshot.completed(), 1);
        assert!(
            progress.terminal.1.is_none(),
            "progress is noticed before the terminal exists"
        );
        let terminal = &seen[2];
        assert_eq!(terminal.notice, terminal.terminal.0);
        assert_eq!(terminal.terminal.0.get(), 1);
        assert_eq!(terminal.terminal.1, Some(id));
    }
    assert!(matches!(
        ledger.borrow_terminal(&ticket).unwrap(),
        Some(Settlement::Completed(1))
    ));

    // 3. Return and consumption release the slot: capacity is published
    //    again, and the reader sees the empty ledger inside notify.
    ledger.worker_returned(ticket.identity()).unwrap();
    assert_eq!(
        reader.seen.lock().unwrap().len(),
        3,
        "return alone frees nothing"
    );
    ledger.consume(&ticket).unwrap();
    {
        let seen = reader.seen.lock().unwrap();
        assert_eq!(seen.len(), 4, "release publishes capacity");
        let last = &seen[3];
        assert_eq!(last.notice, last.capacity.0);
        assert_eq!(last.capacity.0.get(), 2);
        assert_eq!(last.capacity.1, WorkCapacity::EMPTY);
    }
    assert_eq!(publications.capacity().sequence().get(), 2);
    assert_eq!(publications.progress().sequence().get(), 1);
    assert_eq!(publications.terminal().sequence().get(), 1);
}

#[test]
fn an_outage_terminal_is_noticed_through_the_terminal_publication() {
    let ids = Ids::new();
    let incarnation = incarnation(1);
    let conditions = empty_conditions(incarnation);
    let publications = Arc::new(WorkPublications::new(Scope::Process));
    let reader = Arc::new(Reader {
        publications: Arc::clone(&publications),
        seen: Mutex::new(Vec::new()),
    });
    let wake = Arc::new(WakeLog::default());
    let mut ledger = AdmissionLedger::with_notifier(
        incarnation,
        bounds(2, 1024),
        Arc::clone(&publications),
        Arc::clone(&reader) as Arc<dyn Notifier>,
        Arc::clone(&wake) as Arc<dyn WorkerWake>,
    );
    let id = ids.identity(incarnation);
    let ticket: WorkTicket<RouteSpec> =
        submit(&mut ledger, id, &conditions, input(8), CountJob::new(1)).unwrap();
    let condition = ConditionId::new(incarnation, ProcessServiceCode::WorkAdmissionUnavailable);
    ledger
        .fail_unavailable(ticket.identity(), condition)
        .unwrap();
    let seen = reader.seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    let terminal = &seen[1];
    assert_eq!(terminal.notice, terminal.terminal.0);
    assert_eq!(terminal.terminal.1, Some(id));
    assert!(
        wake.wakes().is_empty(),
        "an outage settles a gone worker; it is not a cancellation wake"
    );
}

// --- Round 4: live cancellation seam, arbitration, spec identity, wake, phases ---

struct Quiet;

impl Notifier for Quiet {
    fn notify(&self, _notice: Notice) {}
}

fn ledger_with_wake(
    incarnation: ProcessIncarnation,
    bounds: AdmissionBounds,
    wake: &Arc<WakeLog>,
) -> AdmissionLedger {
    AdmissionLedger::with_notifier(
        incarnation,
        bounds,
        Arc::new(WorkPublications::new(Scope::Process)),
        Arc::new(Quiet),
        Arc::clone(wake) as Arc<dyn WorkerWake>,
    )
}

#[test]
fn a_shared_cancellation_source_is_revoked_from_another_thread() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<SharedCancellation>();
    let now = Instant::now();
    let cancel = SharedCancellation::new();
    let clock = AdjClock::new(now);
    let mut ctx = StepContext::<TestPhase>::new(&clock, budget(now, 2, 100), &cancel);
    assert_eq!(ctx.consume_unit(), Checkpoint::Continue);
    std::thread::scope(|scope| {
        scope.spawn(|| cancel.register(Cancellation::Operator).unwrap());
    });
    assert_eq!(ctx.checkpoint(), Checkpoint::Cancel(Cancellation::Operator));
}

/// What the worker side of a gated slice reports to the controlling test
/// thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GateEvent {
    /// The job finished unit `n` and parked on its release.
    UnitDone(u32),
    /// The slice's step returned — a success, a failure or an entry Yield
    /// alike — so the controller never waits on a channel whose only
    /// sender is a live job with no event left to send.
    SliceFinished,
}

/// Parks after every unit — including after its last unit, before the
/// pre-success checkpoint — so the test thread can register a revocation
/// through the ledger while the slice is out on a worker thread. A park
/// ends only when the controller releases it or drops the release sender;
/// no timer is involved, so a slow controller cannot fail a correct job.
/// After the controller's final release the sender is gone, and a park
/// the protocol did not expect (a seam that hid the revocation) observes
/// the disconnection at once and ends the step with a family failure.
#[derive(Debug)]
struct GateJob {
    next: u32,
    target: u32,
    log: Log,
    events: Sender<GateEvent>,
    proceed: Receiver<()>,
    seen: Arc<Mutex<Vec<Checkpoint>>>,
}

impl CooperativeJob for GateJob {
    type Output = u32;
    type Error = TestJobError;
    type Phase = TestPhase;

    fn step(&mut self, ctx: &mut StepContext<'_, TestPhase>) -> StepOutcome<u32, TestJobError> {
        loop {
            let decision = ctx.checkpoint();
            self.seen.lock().unwrap().push(decision);
            match decision {
                Checkpoint::Cancel(_) | Checkpoint::Yield => return StepOutcome::Yielded,
                Checkpoint::Continue => {}
            }
            if self.next >= self.target {
                // Obligation 7: the checkpoint above ran before this success.
                return StepOutcome::Completed(self.next);
            }
            ctx.consume_unit();
            self.log.lock().unwrap().push(self.next);
            self.next += 1;
            self.events.send(GateEvent::UnitDone(self.next)).unwrap();
            if self.proceed.recv().is_err() {
                return StepOutcome::Failed(TestJobError("parked after the final release"));
            }
        }
    }

    fn retained(&self) -> RetainedState {
        RetainedState::clean(0)
    }
}

/// The park releases of a [`GateJob`]. The controller owns them, releases
/// each park it expects, and drops this after the final one: a later park
/// sees the disconnection immediately instead of waiting on wall time.
struct Releases(Sender<()>);

impl Releases {
    fn release(&self) {
        self.0.send(()).unwrap();
    }
}

/// The controller's ends of a [`GateJob`]'s protocol, consumed while one
/// slice is out: the event receiver, the sender the worker wrapper reports
/// [`GateEvent::SliceFinished`] through, and the park releases. Both
/// senders travel to the worker thread, so a worker that dies disconnects
/// the event channel rather than leaving the controller waiting.
struct GateEnds {
    events: Receiver<GateEvent>,
    finished: Sender<GateEvent>,
    releases: Releases,
}

/// The test thread's observations of a [`GateJob`].
struct Gate {
    seen: Arc<Mutex<Vec<Checkpoint>>>,
    log: Log,
}

impl Gate {
    fn new(target: u32) -> (Self, GateEnds, GateJob) {
        let (events_tx, events_rx) = std::sync::mpsc::channel();
        let (proceed_tx, proceed_rx) = std::sync::mpsc::channel();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = log();
        let job = GateJob {
            next: 0,
            target,
            log: Arc::clone(&log),
            events: events_tx.clone(),
            proceed: proceed_rx,
            seen: Arc::clone(&seen),
        };
        (
            Self { seen, log },
            GateEnds {
                events: events_rx,
                finished: events_tx,
                releases: Releases(proceed_tx),
            },
            job,
        )
    }

    fn seen(&self) -> Vec<Checkpoint> {
        self.seen.lock().unwrap().clone()
    }
}

/// Step `slice` once on a worker thread while `control` runs here as the
/// controller. The worker builds its own controlled clock, which never
/// advances: no deadline expires on its own and nothing in the
/// interleaving depends on wall-clock scheduling. After the step returns,
/// whatever it returned, the worker reports [`GateEvent::SliceFinished`],
/// and this helper requires that to be the event after `control`'s last
/// one, so a unit the controller did not expect fails here, immediately.
/// `control` owns the park releases and lets them drop when it is done.
fn step_gated<T>(
    mut slice: Slice,
    budget: SliceBudget,
    now: Instant,
    ends: GateEnds,
    control: impl FnOnce(&Receiver<GateEvent>, Releases) -> T,
) -> (Slice, SliceOutcome, T) {
    let GateEnds {
        events,
        finished,
        releases,
    } = ends;
    std::thread::scope(|scope| {
        let worker = scope.spawn(move || {
            let clock = AdjClock::new(now);
            let result = slice.step(&clock, budget);
            // Nobody listens only if the controller already failed.
            let _ = finished.send(GateEvent::SliceFinished);
            (slice, result)
        });
        let kept = control(&events, releases);
        assert_eq!(
            events.recv().unwrap(),
            GateEvent::SliceFinished,
            "a unit after the controller's last expected event is a park nobody released"
        );
        let (slice, result) = worker.join().unwrap();
        (slice, result, kept)
    })
}

/// Dispatch `ticket`'s slice to a worker thread, let it finish `units`
/// units, register `reason` through the ledger while the slice is still
/// out and parked, release the worker, and settle. Every assertion about
/// the registration runs after the worker is released, and the final
/// release drops the release sender, so a seam defect fails the test at
/// the next event rather than deadlocking the scope or waiting on a timer.
fn revoke_mid_step(
    ledger: &mut AdmissionLedger,
    ticket: WorkTicket<RouteSpec>,
    ends: GateEnds,
    wake: &WakeLog,
    units: u32,
    reason: Cancellation,
) -> Option<JobState> {
    let slice = ledger.dispatch(ticket.identity()).unwrap().unwrap();
    wake.watch(slice.cancellation_source());
    let now = Instant::now();
    let (slice, result, (redispatch, outcome, wakes)) =
        step_gated(slice, budget(now, 8, 100), now, ends, |events, releases| {
            for done in 1..=units {
                assert_eq!(events.recv().unwrap(), GateEvent::UnitDone(done));
                releases.release();
            }
            assert_eq!(events.recv().unwrap(), GateEvent::UnitDone(units + 1));
            // The slice is out and parked before its next checkpoint.
            let redispatch = ledger.dispatch(ticket.identity());
            let outcome = ledger.cancel(ticket, reason);
            let wakes = wake.wakes();
            releases.release();
            // `releases` drops here: the final release. A park after it —
            // a seam that hid the revocation — sees disconnection at once.
            (redispatch, outcome, wakes)
        });
    assert_eq!(redispatch.unwrap_err(), LedgerError::WorkerNotReturned);
    assert_eq!(outcome, CancelOutcome::Requested);
    assert_eq!(
        wakes,
        vec![Some(reason)],
        "the worker is woken after the revocation is registered"
    );
    ledger.settle(slice, result).unwrap()
}

#[test]
fn a_revocation_registered_through_the_ledger_mid_step_is_observed_at_the_next_checkpoint() {
    let ids = Ids::new();
    let incarnation = incarnation(1);
    let conditions = empty_conditions(incarnation);
    let wake = Arc::new(WakeLog::default());
    let mut ledger = ledger_with_wake(incarnation, bounds(2, 1024), &wake);
    let (gate, ends, job) = Gate::new(3);
    let id = ids.identity(incarnation);
    let ticket = ledger
        .try_submit::<RouteSpec, GateJob>(id, &input(8), job, &conditions)
        .unwrap();
    assert_eq!(
        revoke_mid_step(&mut ledger, ticket, ends, &wake, 0, Cancellation::Operator),
        None,
        "the slice settles as a terminal"
    );
    assert_eq!(produced(&gate.log), vec![0], "no unit after the revocation");
    assert_eq!(
        gate.seen(),
        vec![
            Checkpoint::Continue,
            Checkpoint::Cancel(Cancellation::Operator)
        ]
    );
    match ledger.borrow_terminal(&ticket).unwrap() {
        Some(Settlement::Cancelled(Cancellation::Operator)) => {}
        other => panic!("expected Cancelled(Operator), got {other:?}"),
    }
    assert!(ledger.dispatch(id).unwrap().is_none());
    match settle_route(&mut ledger, &ticket) {
        Settlement::Cancelled(Cancellation::Operator) => {}
        other => panic!("expected Cancelled(Operator), got {other:?}"),
    }
}

#[test]
fn a_revocation_registered_before_the_success_checkpoint_cancels_instead_of_publishing() {
    let ids = Ids::new();
    let incarnation = incarnation(1);
    let conditions = empty_conditions(incarnation);
    let wake = Arc::new(WakeLog::default());
    let mut ledger = ledger_with_wake(incarnation, bounds(2, 1024), &wake);
    let (gate, ends, job) = Gate::new(1);
    let ticket = ledger
        .try_submit::<RouteSpec, GateJob>(ids.identity(incarnation), &input(8), job, &conditions)
        .unwrap();
    assert_eq!(
        revoke_mid_step(&mut ledger, ticket, ends, &wake, 0, Cancellation::Shutdown),
        None
    );
    assert_eq!(
        produced(&gate.log),
        vec![0],
        "the last unit ran; the success it would have returned did not publish"
    );
    assert_eq!(
        gate.seen(),
        vec![
            Checkpoint::Continue,
            Checkpoint::Cancel(Cancellation::Shutdown)
        ],
        "the pre-success checkpoint observed the revocation"
    );
    match settle_route(&mut ledger, &ticket) {
        Settlement::Cancelled(Cancellation::Shutdown) => {}
        other => panic!("expected Cancelled(Shutdown), got {other:?}"),
    }
}

#[test]
fn an_entry_yield_reports_the_finished_slice_without_a_unit_event() {
    let ids = Ids::new();
    let incarnation = incarnation(1);
    let conditions = empty_conditions(incarnation);
    let mut ledger = AdmissionLedger::new(incarnation, bounds(2, 1024));
    let (gate, ends, job) = Gate::new(1);
    let ticket = ledger
        .try_submit::<RouteSpec, GateJob>(ids.identity(incarnation), &input(8), job, &conditions)
        .unwrap();
    let slice = ledger.dispatch(ticket.identity()).unwrap().unwrap();
    let now = Instant::now();
    // The deadline has passed when the worker enters: the first checkpoint
    // yields before any unit, so the job neither parks nor reports a unit.
    // The controller expects nothing; `step_gated`'s wait ends on the
    // finished event, not on a unit the job will never send.
    let expired = SliceBudget::new(NonZeroU32::new(8).unwrap(), Deadline::at(now));
    let (slice, result, ()) = step_gated(slice, expired, now, ends, |_, _| {});
    assert_eq!(gate.seen(), vec![Checkpoint::Yield]);
    assert!(produced(&gate.log).is_empty(), "no unit before the yield");
    assert_eq!(
        ledger.settle(slice, result).unwrap(),
        Some(JobState::Yielded),
        "an entry yield is an ordinary yield"
    );
    assert_eq!(ledger.job_state(&ticket), Some(JobState::Yielded));
    // The job is intact and steps again: revoked, its next entry
    // checkpoint observes the revocation.
    assert_eq!(
        ledger.cancel(ticket, Cancellation::Operator),
        CancelOutcome::Requested
    );
    assert_eq!(
        ledger
            .drive(ticket.identity(), &AdjClock::new(now), budget(now, 8, 100))
            .unwrap(),
        None
    );
    assert_eq!(
        gate.seen(),
        vec![
            Checkpoint::Yield,
            Checkpoint::Cancel(Cancellation::Operator)
        ]
    );
    match settle_route(&mut ledger, &ticket) {
        Settlement::Cancelled(Cancellation::Operator) => {}
        other => panic!("expected Cancelled(Operator), got {other:?}"),
    }
}

#[test]
fn a_park_after_the_final_release_fails_at_once_and_is_reported() {
    let ids = Ids::new();
    let incarnation = incarnation(1);
    let conditions = empty_conditions(incarnation);
    let mut ledger = AdmissionLedger::new(incarnation, bounds(2, 1024));
    let (gate, ends, job) = Gate::new(2);
    let ticket = ledger
        .try_submit::<RouteSpec, GateJob>(ids.identity(incarnation), &input(8), job, &conditions)
        .unwrap();
    let slice = ledger.dispatch(ticket.identity()).unwrap().unwrap();
    let now = Instant::now();
    // The controller expects one park and makes its release the final
    // one without registering anything, so the job — correctly — goes on
    // to a second unit and parks again. That park finds the release
    // sender gone: the step ends at once with the job's own failure, and
    // the unexpected unit is reported rather than silently waited on.
    let (slice, result, ()) =
        step_gated(slice, budget(now, 8, 100), now, ends, |events, releases| {
            assert_eq!(events.recv().unwrap(), GateEvent::UnitDone(1));
            releases.release();
            drop(releases);
            assert_eq!(
                events.recv().unwrap(),
                GateEvent::UnitDone(2),
                "the park nobody will release is reported"
            );
        });
    assert_eq!(
        gate.seen(),
        vec![Checkpoint::Continue, Checkpoint::Continue]
    );
    assert_eq!(produced(&gate.log), vec![0, 1]);
    assert_eq!(ledger.settle(slice, result).unwrap(), None);
    match settle_route(&mut ledger, &ticket) {
        Settlement::Failed(JobError::Family(TestJobError("parked after the final release"))) => {}
        other => panic!("expected the park failure, got {other:?}"),
    }
}

/// Fails with a family error at its first step, whatever its checkpoint
/// said: a late family failure.
#[derive(Debug)]
struct FailJob(&'static str);

impl CooperativeJob for FailJob {
    type Output = u32;
    type Error = TestJobError;
    type Phase = TestPhase;

    fn step(&mut self, ctx: &mut StepContext<'_, TestPhase>) -> StepOutcome<u32, TestJobError> {
        let _ = ctx.checkpoint();
        StepOutcome::Failed(TestJobError(self.0))
    }

    fn retained(&self) -> RetainedState {
        RetainedState::clean(0)
    }
}

#[test]
fn a_selected_cancellation_is_not_replaced_by_a_late_family_failure() {
    let ids = Ids::new();
    let incarnation = incarnation(1);
    let conditions = empty_conditions(incarnation);
    let mut ledger = AdmissionLedger::new(incarnation, bounds(2, 1024));
    let ticket = ledger
        .try_submit::<RouteSpec, FailJob>(
            ids.identity(incarnation),
            &input(8),
            FailJob("late cleanup failure"),
            &conditions,
        )
        .unwrap();
    assert_eq!(
        ledger.cancel(ticket, Cancellation::Operator),
        CancelOutcome::Requested
    );
    let now = Instant::now();
    assert_eq!(
        ledger
            .drive(ticket.identity(), &AdjClock::new(now), budget(now, 2, 100))
            .unwrap(),
        None
    );
    match ledger.borrow_terminal(&ticket).unwrap() {
        Some(Settlement::Cancelled(Cancellation::Operator)) => {}
        other => panic!("the selected cancellation must stand, got {other:?}"),
    }
    match settle_route(&mut ledger, &ticket) {
        Settlement::Cancelled(Cancellation::Operator) => {}
        other => panic!("expected Cancelled(Operator), got {other:?}"),
    }
}

#[test]
fn a_published_family_failure_is_not_rewritten_by_a_later_cancellation() {
    let ids = Ids::new();
    let incarnation = incarnation(1);
    let conditions = empty_conditions(incarnation);
    let mut ledger = AdmissionLedger::new(incarnation, bounds(2, 1024));
    let ticket = ledger
        .try_submit::<RouteSpec, FailJob>(
            ids.identity(incarnation),
            &input(8),
            FailJob("late cleanup failure"),
            &conditions,
        )
        .unwrap();
    let now = Instant::now();
    assert_eq!(
        ledger
            .drive(ticket.identity(), &AdjClock::new(now), budget(now, 2, 100))
            .unwrap(),
        None
    );
    assert_eq!(
        ledger.cancel(ticket, Cancellation::Operator),
        CancelOutcome::AlreadyTerminal
    );
    assert_eq!(
        ledger.cancellation(&ticket),
        None,
        "a late request selects nothing"
    );
    match ledger.borrow_terminal(&ticket).unwrap() {
        Some(Settlement::Failed(JobError::Family(TestJobError("late cleanup failure")))) => {}
        other => panic!("the published failure must stand, got {other:?}"),
    }
    match settle_route(&mut ledger, &ticket) {
        Settlement::Failed(JobError::Family(TestJobError("late cleanup failure"))) => {}
        other => panic!("expected the family failure, got {other:?}"),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
enum DecodeError {
    #[error("png exceeds {0} bytes")]
    Oversized(usize),
}

/// A second family with its own result, error and phase types.
struct DecodeSpec;

impl WorkFamilySpec for DecodeSpec {
    const FAMILY: WorkFamily = WorkFamily::WalkMapDecode;
    type Input = TestInput;
    type Output = String;
    type Error = DecodeError;
    type Phase = WalkMapDecodePhase;

    fn revision(input: &TestInput) -> InputRevision {
        InputRevision::new(input.revision)
    }

    fn charge(input: &TestInput) -> WorkCharge {
        WorkCharge::new(input.bytes, input.scratch, 0, input.handles)
    }

    fn map_cancellation(reason: Cancellation) -> Cancellation {
        reason
    }
}

#[derive(Debug)]
struct TextJob {
    text: &'static str,
    oversized: Option<usize>,
}

impl CooperativeJob for TextJob {
    type Output = String;
    type Error = DecodeError;
    type Phase = WalkMapDecodePhase;

    fn step(
        &mut self,
        ctx: &mut StepContext<'_, WalkMapDecodePhase>,
    ) -> StepOutcome<String, DecodeError> {
        if let Checkpoint::Cancel(_) | Checkpoint::Yield = ctx.checkpoint() {
            return StepOutcome::Yielded;
        }
        ctx.consume_unit();
        ctx.publish_progress(WalkMapDecodePhase::Publish, 1, Some(1));
        match self.oversized {
            Some(bytes) => StepOutcome::Failed(DecodeError::Oversized(bytes)),
            None => StepOutcome::Completed(self.text.to_owned()),
        }
    }

    fn retained(&self) -> RetainedState {
        RetainedState::clean(0)
    }
}

#[test]
fn families_with_distinct_result_error_and_phase_types_share_one_ledger() {
    let ids = Ids::new();
    let incarnation = incarnation(1);
    let conditions = empty_conditions(incarnation);
    let mut ledger = AdmissionLedger::new(incarnation, bounds(4, 1024));
    let mut counting = CountJob::new(1);
    counting.report = true;
    let route: WorkTicket<RouteSpec> = submit(
        &mut ledger,
        ids.identity(incarnation),
        &conditions,
        input(8),
        counting,
    )
    .unwrap();
    let decode = ledger
        .try_submit::<DecodeSpec, TextJob>(
            ids.identity(incarnation),
            &input(8),
            TextJob {
                text: "decoded",
                oversized: None,
            },
            &conditions,
        )
        .unwrap();
    let failing = ledger
        .try_submit::<DecodeSpec, TextJob>(
            ids.identity(incarnation),
            &input(8),
            TextJob {
                text: "",
                oversized: Some(70_000),
            },
            &conditions,
        )
        .unwrap();
    let now = Instant::now();
    let clock = AdjClock::new(now);
    for job in [route.identity(), decode.identity(), failing.identity()] {
        assert_eq!(
            ledger.drive(job, &clock, budget(now, 2, 100)).unwrap(),
            None
        );
        ledger.worker_returned(job).unwrap();
    }
    assert_eq!(
        ledger.progress(&route).unwrap().phase(),
        TestPhase::Commit.index(),
        "route progress is in route phases"
    );
    assert_eq!(
        ledger.progress(&decode).unwrap().phase(),
        WalkMapDecodePhase::Publish.index(),
        "decode progress is in decode phases"
    );
    assert_eq!(WalkMapDecodePhase::Publish.index(), 2);
    assert!(
        WalkMapDecodePhase::Publish.index() > TestPhase::Commit.index(),
        "a decode phase index is not a route phase index"
    );
    // A route ticket composed for the decode job's identity is stale: the
    // spec, not only the identity, is checked.
    let forged = WorkTicket::<RouteSpec>::new(decode.identity(), decode.revision());
    assert_eq!(
        ledger.borrow_terminal(&forged).unwrap_err(),
        LedgerError::StaleIdentity
    );
    assert_eq!(
        ledger.consume(&decode).unwrap(),
        Settlement::Completed("decoded".to_owned())
    );
    assert_eq!(
        ledger.consume(&failing).unwrap(),
        Settlement::Failed(JobError::Family(DecodeError::Oversized(70_000)))
    );
    assert_eq!(ledger.consume(&route).unwrap(), Settlement::Completed(1));
    assert_eq!(ledger.charged_jobs(), 0);
}

/// The same runtime family as [`RouteSpec`] with another output type: a
/// second spec that registration does not forbid.
struct RouteAliasSpec;

impl WorkFamilySpec for RouteAliasSpec {
    const FAMILY: WorkFamily = WorkFamily::Route;
    type Input = TestInput;
    type Output = String;
    type Error = TestJobError;
    type Phase = TestPhase;

    fn revision(input: &TestInput) -> InputRevision {
        InputRevision::new(input.revision)
    }

    fn charge(input: &TestInput) -> WorkCharge {
        WorkCharge::new(input.bytes, input.scratch, 0, input.handles)
    }

    fn map_cancellation(reason: Cancellation) -> Cancellation {
        reason
    }
}

#[test]
fn a_different_spec_of_the_same_family_is_stale_for_every_ticket_operation() {
    let ids = Ids::new();
    let incarnation = incarnation(1);
    let conditions = empty_conditions(incarnation);
    let mut ledger = AdmissionLedger::new(incarnation, bounds(2, 1024));
    let mut job = CountJob::new(1);
    job.report = true;
    let ticket: WorkTicket<RouteSpec> = submit(
        &mut ledger,
        ids.identity(incarnation),
        &conditions,
        input(8),
        job,
    )
    .unwrap();
    let alias = WorkTicket::<RouteAliasSpec>::new(ticket.identity(), ticket.revision());
    assert_eq!(
        alias.family(),
        ticket.family(),
        "same WorkFamily, other spec"
    );

    // Pending: the alias cannot cancel, abandon or see the accepted job.
    assert_eq!(
        ledger.cancel(alias, Cancellation::Operator),
        CancelOutcome::StaleIdentity
    );
    assert_eq!(
        ledger.cancellation(&ticket),
        None,
        "the job was not cancelled"
    );
    assert!(!ledger.is_charged(&alias));
    assert!(ledger.progress(&alias).is_none());
    let now = Instant::now();
    assert_eq!(
        ledger
            .drive(ticket.identity(), &AdjClock::new(now), budget(now, 2, 100))
            .unwrap(),
        None
    );
    assert!(ledger.progress(&ticket).is_some());
    assert!(ledger.progress(&alias).is_none());
    ledger.worker_returned(ticket.identity()).unwrap();

    // Terminal: the alias cannot borrow, consume or discard it.
    assert_eq!(
        ledger.borrow_terminal(&alias).unwrap_err(),
        LedgerError::StaleIdentity
    );
    assert_eq!(
        ledger.consume(&alias).unwrap_err(),
        LedgerError::StaleIdentity
    );
    assert_eq!(
        ledger.abandon(alias, Cancellation::Operator),
        CancelOutcome::StaleIdentity
    );
    assert!(
        ledger.is_charged(&ticket),
        "the accepted ticket's terminal was not discarded"
    );
    match ledger.borrow_terminal(&ticket).unwrap() {
        Some(Settlement::Completed(1)) => {}
        other => panic!("expected the accepted terminal, got {other:?}"),
    }
    assert_eq!(ledger.consume(&ticket).unwrap(), Settlement::Completed(1));
    assert!(!ledger.is_charged(&ticket));
}

#[test]
fn cancel_wakes_the_worker_once_per_recorded_revocation() {
    let ids = Ids::new();
    let incarnation = incarnation(1);
    let conditions = empty_conditions(incarnation);
    let wake = Arc::new(WakeLog::default());
    let mut ledger = ledger_with_wake(incarnation, bounds(4, 1024), &wake);
    let now = Instant::now();
    let clock = AdjClock::new(now);

    // A dispatched job: the worker holds its live source while the
    // service registers the revocation.
    let a: WorkTicket<RouteSpec> = submit(
        &mut ledger,
        ids.identity(incarnation),
        &conditions,
        input(8),
        CountJob::new(1),
    )
    .unwrap();
    let mut slice = ledger.dispatch(a.identity()).unwrap().unwrap();
    wake.watch(slice.cancellation_source());
    assert!(
        wake.wakes().is_empty(),
        "submission and dispatch wake nothing"
    );
    assert_eq!(
        ledger.cancel(a, Cancellation::Operator),
        CancelOutcome::Requested
    );
    assert_eq!(
        wake.wakes(),
        vec![Some(Cancellation::Operator)],
        "one wake, after the source holds the revocation"
    );
    assert_eq!(
        ledger.cancel(a, Cancellation::Shutdown),
        CancelOutcome::Requested,
        "idempotent"
    );
    assert_eq!(
        ledger.abandon(a, Cancellation::Shutdown),
        CancelOutcome::Requested
    );
    let stale = WorkTicket::<RouteSpec>::new(a.identity(), InputRevision::new(7));
    assert_eq!(
        ledger.cancel(stale, Cancellation::Operator),
        CancelOutcome::StaleIdentity
    );
    assert_eq!(
        wake.wakes().len(),
        1,
        "a repeated, abandoning or stale request records nothing and wakes nothing"
    );
    let result = slice.step(&clock, budget(now, 2, 100));
    assert_eq!(ledger.settle(slice, result).unwrap(), None);
    assert_eq!(
        ledger.cancel(a, Cancellation::Operator),
        CancelOutcome::AlreadyTerminal
    );
    assert_eq!(wake.wakes().len(), 1, "a terminal request wakes nothing");

    // Abandon carries cancel's wake obligation.
    let b: WorkTicket<RouteSpec> = submit(
        &mut ledger,
        ids.identity(incarnation),
        &conditions,
        input(8),
        CountJob::new(1),
    )
    .unwrap();
    let slice = ledger.dispatch(b.identity()).unwrap().unwrap();
    wake.watch(slice.cancellation_source());
    assert_eq!(
        ledger.abandon(b, Cancellation::ScopeInvalidated),
        CancelOutcome::Requested
    );
    assert_eq!(
        wake.wakes(),
        vec![
            Some(Cancellation::Operator),
            Some(Cancellation::ScopeInvalidated)
        ]
    );

    // A job already at its terminal is not woken by a late request.
    let c: WorkTicket<RouteSpec> = submit(
        &mut ledger,
        ids.identity(incarnation),
        &conditions,
        input(8),
        CountJob::new(1),
    )
    .unwrap();
    assert_eq!(
        ledger
            .drive(c.identity(), &clock, budget(now, 2, 100))
            .unwrap(),
        None
    );
    assert_eq!(
        ledger.cancel(c, Cancellation::Operator),
        CancelOutcome::AlreadyTerminal
    );
    assert_eq!(wake.wakes().len(), 2);
}

/// Two slices out at once, each offered the other's outcome: the ledger
/// refuses both and nothing — custody, progress, terminal, charge —
/// changes. Same spec, so only the identity binding can tell them apart.
#[test]
fn settle_refuses_another_jobs_outcome_under_the_same_spec() {
    let ids = Ids::new();
    let incarnation = incarnation(1);
    let conditions = empty_conditions(incarnation);
    let mut ledger = AdmissionLedger::new(incarnation, bounds(2, 1024));
    let a: WorkTicket<RouteSpec> = submit(
        &mut ledger,
        ids.identity(incarnation),
        &conditions,
        input(8),
        CountJob::new(1),
    )
    .unwrap();
    let mut two = CountJob::new(2);
    two.multi = true;
    two.report = true;
    let b: WorkTicket<RouteSpec> = submit(
        &mut ledger,
        ids.identity(incarnation),
        &conditions,
        input(8),
        two,
    )
    .unwrap();
    let now = Instant::now();
    let clock = AdjClock::new(now);
    let mut slice_a = ledger.dispatch(a.identity()).unwrap().unwrap();
    let mut slice_b = ledger.dispatch(b.identity()).unwrap().unwrap();
    let result_a = slice_a.step(&clock, budget(now, 4, 100));
    let result_b = slice_b.step(&clock, budget(now, 4, 100));
    assert_eq!(
        ledger.settle(slice_a, result_b).unwrap_err(),
        LedgerError::StaleIdentity,
        "b's outcome cannot settle a"
    );
    assert_eq!(
        ledger.settle(slice_b, result_a).unwrap_err(),
        LedgerError::StaleIdentity,
        "a's outcome cannot settle b"
    );
    assert_eq!(ledger.borrow_terminal(&a).unwrap(), None);
    assert_eq!(ledger.borrow_terminal(&b).unwrap(), None);
    assert_eq!(
        ledger.publications().progress().sequence().get(),
        0,
        "b's progress report was not published under either job"
    );
    assert_eq!(
        ledger.dispatch(a.identity()).unwrap_err(),
        LedgerError::WorkerNotReturned,
        "a's custody is still out"
    );
    assert_eq!(
        ledger.dispatch(b.identity()).unwrap_err(),
        LedgerError::WorkerNotReturned,
        "b's custody is still out"
    );
    assert_eq!(ledger.charged_jobs(), 2);
}

/// The same exchange across families: a decode outcome offered for the
/// route slice would install a `String` terminal under a `u32` ticket.
#[test]
fn settle_refuses_another_familys_outcome() {
    let ids = Ids::new();
    let incarnation = incarnation(1);
    let conditions = empty_conditions(incarnation);
    let mut ledger = AdmissionLedger::new(incarnation, bounds(2, 1024));
    let route: WorkTicket<RouteSpec> = submit(
        &mut ledger,
        ids.identity(incarnation),
        &conditions,
        input(8),
        CountJob::new(1),
    )
    .unwrap();
    let decode = ledger
        .try_submit::<DecodeSpec, TextJob>(
            ids.identity(incarnation),
            &input(8),
            TextJob {
                text: "decoded",
                oversized: None,
            },
            &conditions,
        )
        .unwrap();
    let now = Instant::now();
    let clock = AdjClock::new(now);
    let mut slice_route = ledger.dispatch(route.identity()).unwrap().unwrap();
    let mut slice_decode = ledger.dispatch(decode.identity()).unwrap().unwrap();
    let result_route = slice_route.step(&clock, budget(now, 4, 100));
    let result_decode = slice_decode.step(&clock, budget(now, 4, 100));
    assert_eq!(
        ledger.settle(slice_route, result_decode).unwrap_err(),
        LedgerError::StaleIdentity
    );
    assert_eq!(
        ledger.settle(slice_decode, result_route).unwrap_err(),
        LedgerError::StaleIdentity
    );
    assert_eq!(ledger.borrow_terminal(&route).unwrap(), None);
    assert_eq!(ledger.borrow_terminal(&decode).unwrap(), None);
    assert_eq!(ledger.publications().progress().sequence().get(), 0);
    assert_eq!(
        ledger.dispatch(route.identity()).unwrap_err(),
        LedgerError::WorkerNotReturned
    );
    assert_eq!(
        ledger.dispatch(decode.identity()).unwrap_err(),
        LedgerError::WorkerNotReturned
    );
}

/// A slice stepped twice offers its first step's outcome: that outcome is
/// of an earlier step of this very dispatch, and is refused like a foreign
/// one.
#[test]
fn settle_refuses_an_earlier_steps_outcome_of_the_same_slice() {
    let ids = Ids::new();
    let incarnation = incarnation(1);
    let conditions = empty_conditions(incarnation);
    let mut ledger = AdmissionLedger::new(incarnation, bounds(2, 1024));
    let ticket: WorkTicket<RouteSpec> = submit(
        &mut ledger,
        ids.identity(incarnation),
        &conditions,
        input(8),
        CountJob::new(2),
    )
    .unwrap();
    let now = Instant::now();
    let clock = AdjClock::new(now);
    let mut slice = ledger.dispatch(ticket.identity()).unwrap().unwrap();
    let first = slice.step(&clock, budget(now, 4, 100));
    let _latest = slice.step(&clock, budget(now, 4, 100));
    assert_eq!(
        ledger.settle(slice, first).unwrap_err(),
        LedgerError::StaleIdentity
    );
    assert_eq!(ledger.borrow_terminal(&ticket).unwrap(), None);
    assert_eq!(ledger.job_state(&ticket), Some(JobState::Executing));
    assert_eq!(
        ledger.dispatch(ticket.identity()).unwrap_err(),
        LedgerError::WorkerNotReturned
    );
}
