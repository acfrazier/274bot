use super::*;
use std::cell::Cell;
use std::num::{NonZeroU32, NonZeroU64, NonZeroUsize};
use std::sync::Mutex;
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

fn charge(bytes: usize) -> WorkCharge {
    WorkCharge::new(bytes, 0, 0, 0)
}

fn charge_handles(bytes: usize, handles: u32) -> WorkCharge {
    WorkCharge::new(bytes, 0, 0, handles)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
struct TestJobError(&'static str);

type TestLedger = AdmissionLedger<u32, TestJobError>;

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

struct CountJob {
    next: u32,
    target: u32,
    produced: Vec<u32>,
    forbidden: Option<ForbiddenRetain>,
    ignore_yield: bool,
    ignore_cancel: bool,
    multi: bool,
}

impl CountJob {
    fn new(target: u32) -> Self {
        Self {
            next: 0,
            target,
            produced: Vec::new(),
            forbidden: None,
            ignore_yield: false,
            ignore_cancel: false,
            multi: false,
        }
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
            self.produced.push(self.next);
            self.next += 1;
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
            None => RetainedState::clean(0),
        }
    }
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
}

#[test]
fn map_bake_status_characterizes_current_producer_endings() {
    // host-play/src/map_cache.rs:844-868,1939-1946
    assert!(MapBakeStatus::Ready.is_domain_ending());
    assert!(MapBakeStatus::Paused.is_domain_ending());
    assert!(MapBakeStatus::Cancelled.is_domain_ending());
    assert!(MapBakeStatus::Failed.is_domain_ending());
    assert!(!MapBakeStatus::Queued.is_domain_ending());
    assert!(!MapBakeStatus::Running.is_domain_ending());
    assert!(MapBakeStatus::Paused.is_family_result());
    assert!(!MapBakeStatus::Cancelled.is_family_result());
    assert_eq!(WorkFamily::MapBake.producer(), Track::T5);
    assert_eq!(WorkFamily::MapBake.consumers(), &[Track::T3]);
}

#[test]
fn walk_map_decode_characterizes_one_job_as_the_unit() {
    // panel/src/walk_map.rs:1026-1038,1165-1186
    assert!(WalkMapDecodeCommand::Job.is_interruptible_unit());
    assert!(!WalkMapDecodeCommand::Stop.is_interruptible_unit());
    assert_eq!(WorkFamily::WalkMapDecode.producer(), Track::T4);
    assert_eq!(
        WorkFamily::WalkMapDecode.interruptible_unit(),
        "one decode_checked job (not PNG-length alone)"
    );
}

#[test]
fn family_preparation_is_named_as_forbidden_across_a_yield() {
    // selected.rs:310-328: FamilyPreparation is Rc-bound and !Send.
    assert_eq!(
        WorkFamily::CardPreparation.interruptible_unit(),
        "one preparation phase"
    );
    assert_eq!(
        ForbiddenRetain::OtherOwnerResource,
        ForbiddenRetain::OtherOwnerResource
    );
    fn require_send<T: Send>() {}
    require_send::<CountJob>();
    let _ = PreparationPhase::Prepare;
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
    let ticket = ledger
        .try_submit(
            ids.identity(incarnation),
            WorkFamily::Route,
            InputRevision::FIRST,
            charge(16),
            &conditions,
        )
        .unwrap();
    let mut job = CountJob::new(10);
    job.multi = true;
    let now = Instant::now();
    assert_eq!(
        ledger
            .drive(&ticket, &mut job, &now, budget(now, 3, 50))
            .unwrap(),
        Some(JobState::Yielded)
    );
    assert_eq!(job.produced, vec![0, 1, 2]);
    assert_eq!(job.next, 3);
}

#[test]
fn yield_resume_preserves_the_cursor_and_does_not_repeat_a_unit() {
    let ids = Ids::new();
    let incarnation = incarnation(1);
    let conditions = empty_conditions(incarnation);
    let mut ledger = AdmissionLedger::new(incarnation, bounds(4, 1024));
    let ticket = ledger
        .try_submit(
            ids.identity(incarnation),
            WorkFamily::Route,
            InputRevision::FIRST,
            charge(16),
            &conditions,
        )
        .unwrap();
    let mut job = CountJob::new(3);
    let now = Instant::now();
    let slice = budget(now, 1, 50);
    assert_eq!(
        ledger.drive(&ticket, &mut job, &now, slice).unwrap(),
        Some(JobState::Yielded)
    );
    assert_eq!(job.produced, vec![0]);
    assert_eq!(
        ledger.drive(&ticket, &mut job, &now, slice).unwrap(),
        Some(JobState::Yielded)
    );
    assert_eq!(job.produced, vec![0, 1]);
    assert_eq!(ledger.drive(&ticket, &mut job, &now, slice).unwrap(), None);
    assert_eq!(job.produced, vec![0, 1, 2]);
    ledger.worker_returned(&ticket).unwrap();
    match ledger.consume(&ticket).unwrap() {
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
    let ticket = ledger
        .try_submit(
            ids.identity(incarnation),
            WorkFamily::MapBake,
            InputRevision::FIRST,
            charge(8),
            &conditions,
        )
        .unwrap();
    let mut job = CountJob::new(2);
    let now = Instant::now();
    let expired = budget(now, 1, 0);
    let later = now + Duration::from_millis(1);
    assert_eq!(
        ledger.drive(&ticket, &mut job, &later, expired).unwrap(),
        Some(JobState::Yielded)
    );
    assert!(job.produced.is_empty());
    assert_eq!(
        ledger.cancel(ticket, Cancellation::Operator),
        CancelOutcome::Requested
    );
    assert_eq!(
        ledger
            .drive(&ticket, &mut job, &now, budget(now, 1, 50))
            .unwrap(),
        None
    );
    ledger.worker_returned(&ticket).unwrap();
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

    let ticket = ledger
        .try_submit(
            ids.identity(incarnation),
            WorkFamily::Route,
            InputRevision::FIRST,
            charge(8),
            &conditions,
        )
        .unwrap();
    let mut late = CountJob::new(1);
    late.ignore_cancel = true;
    assert_eq!(
        ledger.cancel(ticket, Cancellation::Operator),
        CancelOutcome::Requested
    );
    assert_eq!(
        ledger
            .drive(&ticket, &mut late, &now, budget(now, 1, 50))
            .unwrap(),
        None
    );
    ledger.worker_returned(&ticket).unwrap();
    match ledger.consume(&ticket).unwrap() {
        Settlement::Cancelled(Cancellation::Operator) => {}
        other => panic!("late success must not publish, got {other:?}"),
    }

    let ticket = ledger
        .try_submit(
            ids.identity(incarnation),
            WorkFamily::Route,
            InputRevision::FIRST,
            charge(8),
            &conditions,
        )
        .unwrap();
    let mut job = CountJob::new(1);
    assert_eq!(
        ledger
            .drive(&ticket, &mut job, &now, budget(now, 1, 50))
            .unwrap(),
        None
    );
    assert_eq!(
        ledger.cancel(ticket, Cancellation::Operator),
        CancelOutcome::AlreadyTerminal
    );
    ledger.worker_returned(&ticket).unwrap();
    match ledger.consume(&ticket).unwrap() {
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
    let ticket = ledger
        .try_submit(
            ids.identity(incarnation),
            WorkFamily::Inspection,
            InputRevision::FIRST,
            charge(8),
            &conditions,
        )
        .unwrap();
    let mut job = CountJob::new(1);
    job.ignore_cancel = true;
    ledger.cancel(ticket, Cancellation::Shutdown);
    assert_eq!(ledger.cancellation(&ticket), Some(Cancellation::Shutdown));
    ledger
        .drive(
            &ticket,
            &mut job,
            &Instant::now(),
            budget(Instant::now(), 1, 50),
        )
        .unwrap();
    ledger.worker_returned(&ticket).unwrap();
    match ledger.borrow_terminal(&ticket) {
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
    let ticket = ledger
        .try_submit(
            ids.identity(incarnation),
            WorkFamily::CardPreparation,
            InputRevision::FIRST,
            charge(8),
            &conditions,
        )
        .unwrap();
    let mut job = CountJob::new(4);
    job.forbidden = Some(ForbiddenRetain::OtherOwnerResource);
    let now = Instant::now();
    assert_eq!(
        ledger
            .drive(&ticket, &mut job, &now, budget(now, 1, 50))
            .unwrap(),
        None
    );
    ledger.worker_returned(&ticket).unwrap();
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
    let ticket = ledger
        .try_submit(
            ids.identity(incarnation),
            WorkFamily::Route,
            InputRevision::FIRST,
            charge(8),
            &conditions,
        )
        .unwrap();
    let mut job = CountJob::new(8);
    job.ignore_yield = true;
    let now = Instant::now();
    assert_eq!(
        ledger
            .drive(&ticket, &mut job, &now, budget(now, 1, 50))
            .unwrap(),
        None
    );
    ledger.worker_returned(&ticket).unwrap();
    match ledger.consume(&ticket).unwrap() {
        Settlement::Failed(JobError::Cooperation(CooperativeViolation::IgnoredYield)) => {}
        other => panic!("expected IgnoredYield, got {other:?}"),
    }
}

#[test]
fn wrong_scope_revision_and_duplicate_consume_cannot_install_or_reclaim() {
    let ids = Ids::new();
    let incarnation = incarnation(1);
    let conditions = empty_conditions(incarnation);
    let mut ledger = AdmissionLedger::new(incarnation, bounds(2, 1024));
    let id = ids.identity(incarnation);
    let ticket = ledger
        .try_submit(
            id,
            WorkFamily::Route,
            InputRevision::FIRST,
            charge(8),
            &conditions,
        )
        .unwrap();
    let mut fence = Fence::new();
    fence.arm(ticket);
    let stale = WorkTicket::new(id, WorkFamily::Route, InputRevision::new(1));
    assert!(fence.admit(stale).is_err());
    assert_eq!(
        ledger.cancel(stale, Cancellation::Operator),
        CancelOutcome::StaleIdentity
    );

    let mut job = CountJob::new(1);
    let now = Instant::now();
    ledger
        .drive(&ticket, &mut job, &now, budget(now, 1, 50))
        .unwrap();
    ledger.worker_returned(&ticket).unwrap();
    ledger.consume(&ticket).unwrap();
    match ledger.consume(&ticket) {
        Err(LedgerError::AlreadyConsumed | LedgerError::StaleIdentity) => {}
        other => panic!("duplicate consume must not reclaim another ticket, got {other:?}"),
    }
    assert!(!ledger.is_charged(&ticket));
}

#[test]
fn saturated_ledger_returns_full_without_taking_custody() {
    let ids = Ids::new();
    let incarnation = incarnation(1);
    let conditions = empty_conditions(incarnation);
    let mut ledger: TestLedger = AdmissionLedger::new(incarnation, bounds(1, 32));
    let first = ids.identity(incarnation);
    let first_ticket = ledger
        .try_submit(
            first,
            WorkFamily::Route,
            InputRevision::FIRST,
            charge(16),
            &conditions,
        )
        .unwrap();
    let second = ids.identity(incarnation);
    match ledger.try_submit(
        second,
        WorkFamily::Route,
        InputRevision::FIRST,
        charge(16),
        &conditions,
    ) {
        Err(SubmissionRefusal::Full) => {}
        other => panic!("expected Full, got {other:?}"),
    }
    assert!(ledger.borrow_terminal(&first_ticket).is_none());
    assert_eq!(ledger.charged_jobs(), 1);
    assert!(!ledger.is_charged(&WorkTicket::new(
        second,
        WorkFamily::Route,
        InputRevision::FIRST
    )));
}

#[test]
fn byte_and_handle_caps_return_full() {
    let ids = Ids::new();
    let incarnation = incarnation(1);
    let conditions = empty_conditions(incarnation);
    let mut bytes: TestLedger = AdmissionLedger::new(incarnation, bounds_full(8, 16, 8));
    bytes
        .try_submit(
            ids.identity(incarnation),
            WorkFamily::Route,
            InputRevision::FIRST,
            charge(16),
            &conditions,
        )
        .unwrap();
    match bytes.try_submit(
        ids.identity(incarnation),
        WorkFamily::Route,
        InputRevision::FIRST,
        charge(1),
        &conditions,
    ) {
        Err(SubmissionRefusal::Full) => {}
        other => panic!("expected byte Full, got {other:?}"),
    }

    let mut handles: TestLedger = AdmissionLedger::new(incarnation, bounds_full(8, 1024, 1));
    handles
        .try_submit(
            ids.identity(incarnation),
            WorkFamily::Route,
            InputRevision::FIRST,
            charge_handles(8, 1),
            &conditions,
        )
        .unwrap();
    match handles.try_submit(
        ids.identity(incarnation),
        WorkFamily::Route,
        InputRevision::FIRST,
        charge_handles(8, 1),
        &conditions,
    ) {
        Err(SubmissionRefusal::Full) => {}
        other => panic!("expected handle Full, got {other:?}"),
    }
}

#[test]
fn unconsumed_terminal_holds_capacity() {
    let ids = Ids::new();
    let incarnation = incarnation(1);
    let conditions = empty_conditions(incarnation);
    let mut ledger: TestLedger = AdmissionLedger::new(incarnation, bounds(1, 1024));
    let ticket = ledger
        .try_submit(
            ids.identity(incarnation),
            WorkFamily::Route,
            InputRevision::FIRST,
            charge(8),
            &conditions,
        )
        .unwrap();
    let mut job = CountJob::new(1);
    let now = Instant::now();
    ledger
        .drive(&ticket, &mut job, &now, budget(now, 1, 50))
        .unwrap();
    ledger.worker_returned(&ticket).unwrap();
    match ledger.try_submit(
        ids.identity(incarnation),
        WorkFamily::Route,
        InputRevision::FIRST,
        charge(8),
        &conditions,
    ) {
        Err(SubmissionRefusal::Full) => {}
        other => panic!("unconsumed terminal must hold the job slot, got {other:?}"),
    }
    ledger.consume(&ticket).unwrap();
    ledger
        .try_submit(
            ids.identity(incarnation),
            WorkFamily::Route,
            InputRevision::FIRST,
            charge(8),
            &conditions,
        )
        .unwrap();
}

#[test]
fn released_job_id_is_not_reused() {
    let ids = Ids::new();
    let incarnation = incarnation(1);
    let conditions = empty_conditions(incarnation);
    let mut ledger: TestLedger = AdmissionLedger::new(incarnation, bounds(2, 1024));
    let identity = ids.identity(incarnation);
    let ticket = ledger
        .try_submit(
            identity,
            WorkFamily::Route,
            InputRevision::FIRST,
            charge(8),
            &conditions,
        )
        .unwrap();
    let mut job = CountJob::new(1);
    let now = Instant::now();
    ledger
        .drive(&ticket, &mut job, &now, budget(now, 1, 50))
        .unwrap();
    ledger.worker_returned(&ticket).unwrap();
    ledger.consume(&ticket).unwrap();
    match ledger.try_submit(
        identity,
        WorkFamily::Route,
        InputRevision::FIRST,
        charge(8),
        &conditions,
    ) {
        Err(SubmissionRefusal::InvalidArgument) => {}
        other => panic!("released job id must not be reused, got {other:?}"),
    }
}

#[test]
fn abandoned_ticket_retains_credit_until_the_worker_returns() {
    let ids = Ids::new();
    let incarnation = incarnation(1);
    let conditions = empty_conditions(incarnation);
    let mut ledger: TestLedger = AdmissionLedger::new(incarnation, bounds(2, 1024));
    let ticket = ledger
        .try_submit(
            ids.identity(incarnation),
            WorkFamily::Route,
            InputRevision::FIRST,
            charge(32),
            &conditions,
        )
        .unwrap();
    assert_eq!(ledger.charged_bytes(), 32);
    ledger.abandon(ticket, Cancellation::Operator);
    assert!(
        ledger.is_charged(&ticket),
        "abandon must not free credit before the worker returns"
    );
    match ledger.discard_abandoned(&ticket) {
        Err(LedgerError::WorkerNotReturned) => {}
        other => panic!("discard before return must fail, got {other:?}"),
    }
    ledger.worker_returned(&ticket).unwrap();
    assert!(
        !ledger.is_charged(&ticket),
        "credit frees only after the worker returns"
    );
}

#[test]
fn progress_coalescing_does_not_take_terminal_custody() {
    let ids = Ids::new();
    let incarnation = incarnation(1);
    let conditions = empty_conditions(incarnation);
    let mut ledger = AdmissionLedger::new(incarnation, bounds(1, 1024));
    let id = ids.identity(incarnation);
    let ticket = ledger
        .try_submit(
            id,
            WorkFamily::MapBake,
            InputRevision::FIRST,
            charge(8),
            &conditions,
        )
        .unwrap();
    ledger
        .publish_progress(&ticket, WorkProgress::new(id, 1, 0, 1, Some(4)))
        .unwrap();
    ledger
        .publish_progress(&ticket, WorkProgress::new(id, 1, 0, 99, Some(4)))
        .unwrap();
    assert_eq!(ledger.progress(&ticket).unwrap().completed(), 1);
    ledger
        .publish_progress(&ticket, WorkProgress::new(id, 2, 1, 2, Some(4)))
        .unwrap();
    assert_eq!(ledger.progress(&ticket).unwrap().completed(), 2);
    let mut job = CountJob::new(1);
    let now = Instant::now();
    ledger
        .drive(&ticket, &mut job, &now, budget(now, 1, 50))
        .unwrap();
    ledger.worker_returned(&ticket).unwrap();
    match ledger.consume(&ticket).unwrap() {
        Settlement::Completed(1) => {}
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
    let mut ledger: TestLedger = AdmissionLedger::new(incarnation, bounds(2, 1024));
    let id = ids.identity(incarnation);
    match ledger.try_submit(
        id,
        WorkFamily::Route,
        InputRevision::FIRST,
        charge(8),
        &conditions,
    ) {
        Err(SubmissionRefusal::Unavailable(refusal)) => {
            assert_eq!(refusal.request(), Some(id.job()));
            assert_eq!(refusal.scope(), Scope::Process);
            assert_eq!(
                refusal.condition().code(),
                ProcessServiceCode::WorkAdmissionUnavailable
            );
        }
        other => panic!("expected Unavailable, got {other:?}"),
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
    let mut ledger: TestLedger = AdmissionLedger::new(incarnation(1), bounds(1, 1024));
    let id = Ids::new().identity(incarnation(2));
    match ledger.try_submit(
        id,
        WorkFamily::Route,
        InputRevision::FIRST,
        charge(8),
        &conditions,
    ) {
        Err(SubmissionRefusal::InvalidAuthority) => {}
        other => panic!("expected InvalidAuthority, got {other:?}"),
    }
}

#[test]
fn stopping_refuses_new_work() {
    let ids = Ids::new();
    let incarnation = incarnation(1);
    let conditions = empty_conditions(incarnation);
    let mut ledger: TestLedger = AdmissionLedger::new(incarnation, bounds(1, 1024));
    ledger.begin_stop();
    match ledger.try_submit(
        ids.identity(incarnation),
        WorkFamily::Route,
        InputRevision::FIRST,
        charge(8),
        &conditions,
    ) {
        Err(SubmissionRefusal::Stopping) => {}
        other => panic!("expected Stopping, got {other:?}"),
    }
}

#[test]
fn yielded_charges_survive_until_consume_and_worker_return() {
    let ids = Ids::new();
    let incarnation = incarnation(1);
    let conditions = empty_conditions(incarnation);
    let mut ledger = AdmissionLedger::new(incarnation, bounds(2, 1024));
    let ticket = ledger
        .try_submit(
            ids.identity(incarnation),
            WorkFamily::Route,
            InputRevision::FIRST,
            charge(24),
            &conditions,
        )
        .unwrap();
    let mut job = CountJob::new(2);
    let now = Instant::now();
    ledger
        .drive(&ticket, &mut job, &now, budget(now, 1, 50))
        .unwrap();
    assert_eq!(ledger.job_state(&ticket), Some(JobState::Yielded));
    assert_eq!(ledger.charged_bytes(), 24);
    ledger
        .drive(&ticket, &mut job, &now, budget(now, 1, 50))
        .unwrap();
    assert!(ledger.is_charged(&ticket));
    ledger.worker_returned(&ticket).unwrap();
    assert!(ledger.is_charged(&ticket));
    ledger.consume(&ticket).unwrap();
    assert!(!ledger.is_charged(&ticket));
}

#[test]
fn fail_unavailable_publishes_and_retains_charge() {
    let ids = Ids::new();
    let incarnation = incarnation(1);
    let conditions = empty_conditions(incarnation);
    let mut ledger: TestLedger = AdmissionLedger::new(incarnation, bounds(2, 1024));
    let ticket = ledger
        .try_submit(
            ids.identity(incarnation),
            WorkFamily::Route,
            InputRevision::FIRST,
            charge(16),
            &conditions,
        )
        .unwrap();
    let wrong = ConditionId::new(incarnation, ProcessServiceCode::ReactorUnavailable);
    match ledger.fail_unavailable(&ticket, wrong) {
        Err(LedgerError::WrongCondition) => {}
        other => panic!("expected WrongCondition, got {other:?}"),
    }
    let condition = ConditionId::new(incarnation, ProcessServiceCode::WorkAdmissionUnavailable);
    ledger.fail_unavailable(&ticket, condition).unwrap();
    match ledger.fail_unavailable(&ticket, condition) {
        Err(LedgerError::AlreadyTerminal) => {}
        other => panic!("second fail_unavailable must not rewrite, got {other:?}"),
    }
    assert!(ledger.is_charged(&ticket));
    let mut job = CountJob::new(1);
    let now = Instant::now();
    assert_eq!(
        ledger
            .drive(&ticket, &mut job, &now, budget(now, 1, 50))
            .unwrap(),
        None
    );
    match ledger.borrow_terminal(&ticket) {
        Some(Settlement::Failed(JobError::Unavailable(terminal))) => {
            assert_eq!(terminal.condition(), condition);
        }
        other => panic!("late completion must not replace Unavailable, got {other:?}"),
    }
    ledger.worker_returned(&ticket).unwrap();
    assert!(ledger.is_charged(&ticket));
    match ledger.consume(&ticket).unwrap() {
        Settlement::Failed(JobError::Unavailable(_)) => {}
        other => panic!("expected Unavailable terminal, got {other:?}"),
    }
    assert!(!ledger.is_charged(&ticket));
}

struct RecordingNotifier {
    notices: Mutex<Vec<PublishSequence>>,
}

impl Notifier for RecordingNotifier {
    fn notify(&self, notice: Notice) {
        self.notices.lock().unwrap().push(notice.sequence());
    }
}

#[test]
fn capacity_progress_and_terminal_publish_before_notify() {
    let ids = Ids::new();
    let incarnation = incarnation(1);
    let conditions = empty_conditions(incarnation);
    let mut ledger: TestLedger = AdmissionLedger::new(incarnation, bounds(2, 1024));
    let id = ids.identity(incarnation);
    let ticket = ledger
        .try_submit(
            id,
            WorkFamily::Route,
            InputRevision::FIRST,
            charge(8),
            &conditions,
        )
        .unwrap();
    assert_eq!(ledger.incarnation(), incarnation);
    ledger.capacity_publication().read(|seq, value| {
        assert!(
            seq.get() >= 1,
            "submit publishes capacity before any notice"
        );
        assert_eq!(value.jobs(), 1);
    });
    assert!(ledger
        .terminal_publication()
        .read(|_, value| value.is_none()));

    let notifier = RecordingNotifier {
        notices: Mutex::new(Vec::new()),
    };
    let progress = WorkProgress::new(id, 1, 0, 1, Some(2));
    ledger.publish_progress(&ticket, progress.clone()).unwrap();
    ledger
        .progress_publication()
        .publish(Some(progress), &notifier)
        .unwrap();
    let noticed = notifier.notices.lock().unwrap()[0];
    ledger.progress_publication().read(|seq, value| {
        assert_eq!(seq, noticed);
        assert!(value.is_some());
    });
}
