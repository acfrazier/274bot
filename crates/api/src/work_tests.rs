use super::*;
use std::cell::Cell;
use std::num::{NonZeroU32, NonZeroU64, NonZeroUsize};
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

    fn step(&mut self, ctx: &mut StepContext<'_>) -> StepOutcome<u32, TestJobError> {
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
    F: WorkFamilySpec<Input = TestInput, Output = u32, Error = TestJobError>,
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
    let cancel = Cell::new(None);
    let now = Instant::now();
    let clock = AdjClock::new(now);
    let mut ctx = StepContext::new(&clock, budget(now, 3, 50), &cancel);
    assert_eq!(ctx.checkpoint(), Checkpoint::Continue);
    assert_eq!(ctx.consume_unit(), Checkpoint::Continue);
    assert_eq!(ctx.consume_unit(), Checkpoint::Continue);
    assert_eq!(ctx.consume_unit(), Checkpoint::Yield);
    assert_eq!(ctx.units_this_slice(), 3);
    assert!(!ctx.exceeded_allowance());
}

#[test]
fn deadline_expires_mid_slice_under_a_fake_clock() {
    let cancel = Cell::new(None);
    let now = Instant::now();
    let clock = AdjClock::new(now);
    let mut ctx = StepContext::new(&clock, budget(now, 8, 50), &cancel);
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
    let cancel = Cell::new(None);
    let mut ctx = StepContext::new(
        &clock,
        SliceBudget::new(NonZeroU32::new(8).unwrap(), Deadline::at(first)),
        &cancel,
    );
    assert_eq!(ctx.checkpoint(), Checkpoint::Yield);
}

#[test]
fn cancellation_registered_between_units_is_observed() {
    let cancel = Cell::new(None);
    let now = Instant::now();
    let clock = AdjClock::new(now);
    let mut ctx = StepContext::new(&clock, budget(now, 8, 50), &cancel);
    assert_eq!(ctx.consume_unit(), Checkpoint::Continue);
    cancel.set(Some(Cancellation::Operator));
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
    let cancel = Cell::new(None);
    let now = Instant::now();
    let clock = AdjClock::new(now);
    let mut ctx = StepContext::new(&clock, budget(now, 8, 50), &cancel);
    assert_eq!(job.step(&mut ctx), StepOutcome::Completed(7));
    assert_eq!(produced(&log).len(), 7);
}

#[test]
fn byte_and_handle_caps_return_full() {
    let ids = Ids::new();
    let incarnation = incarnation(1);
    let conditions = empty_conditions(incarnation);
    let mut bytes = AdmissionLedger::new(incarnation, bounds_full(8, 16, 8));
    submit::<RouteSpec>(
        &mut bytes,
        ids.identity(incarnation),
        &conditions,
        input(16),
        CountJob::new(1),
    )
    .unwrap();
    match submit::<RouteSpec>(
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
    let mut ledger = AdmissionLedger::with_notifier(
        incarnation,
        bounds(2, 1024),
        Arc::clone(&publications),
        Arc::clone(&reader) as Arc<dyn Notifier>,
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
}
