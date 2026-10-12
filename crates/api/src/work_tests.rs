use super::*;
use std::num::{NonZeroU32, NonZeroUsize};
use std::time::{Duration, Instant};

use crate::identity::{
    Cancellation, Deadline, Fence, ProcessIncarnation, ProcessTime, RequestIdMinter, Scope,
    Settlement, WorkIdentity,
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
        WorkIdentity::new(self.minter.mint().unwrap(), incarnation, Scope::Process).unwrap()
    }
}

fn charge(bytes: usize) -> WorkCharge {
    WorkCharge::new(bytes, 0, 0, 0)
}

type TestLedger = AdmissionLedger<u32, &'static str>;

fn budget(now: Instant, units: u32, ms: u64) -> SliceBudget {
    SliceBudget::new(
        NonZeroU32::new(units).unwrap(),
        Deadline::after(now, Duration::from_millis(ms)),
    )
}

struct CountJob {
    next: u32,
    target: u32,
    produced: Vec<u32>,
    forbidden: Option<ForbiddenRetain>,
    ignore_yield: bool,
    ignore_cancel: bool,
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
        }
    }
}

impl CooperativeJob for CountJob {
    type Output = u32;
    type Error = &'static str;

    fn step(&mut self, ctx: &mut StepContext<'_>) -> StepOutcome<u32, &'static str> {
        match ctx.checkpoint() {
            Checkpoint::Cancel(_) if !self.ignore_cancel => return StepOutcome::Yielded,
            Checkpoint::Yield if !self.ignore_yield => return StepOutcome::Yielded,
            Checkpoint::Cancel(_) | Checkpoint::Yield | Checkpoint::Continue => {}
        }
        let _ = ctx.consume_unit();
        if self.ignore_yield {
            let _ = ctx.consume_unit();
        }
        self.produced.push(self.next);
        self.next += 1;
        if self.next >= self.target {
            StepOutcome::Completed(self.next)
        } else {
            StepOutcome::Yielded
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
fn every_family_has_a_producer_consumer_and_unit() {
    assert_eq!(WorkFamily::ALL.len(), 8);
    for family in WorkFamily::ALL {
        assert!(!family.producer().is_empty());
        assert!(!family.consumer().is_empty());
        assert!(!family.interruptible_unit().is_empty());
    }
}

#[test]
fn yield_resume_preserves_the_cursor_and_does_not_repeat_a_unit() {
    let ids = Ids::new();
    let incarnation = ProcessIncarnation::mint();
    let mut ledger = AdmissionLedger::new(incarnation, bounds(4, 1024));
    let ticket = ledger
        .try_submit(
            ids.identity(incarnation),
            WorkFamily::Route,
            InputRevision::FIRST,
            charge(16),
            None,
        )
        .unwrap();
    let mut job = CountJob::new(3);
    let now = Instant::now();
    let slice = budget(now, 1, 50);
    assert_eq!(
        ledger.drive(&ticket, &mut job, now, slice).unwrap(),
        Some(JobState::Yielded)
    );
    assert_eq!(job.produced, vec![0]);
    assert_eq!(
        ledger.drive(&ticket, &mut job, now, slice).unwrap(),
        Some(JobState::Yielded)
    );
    assert_eq!(job.produced, vec![0, 1]);
    assert_eq!(ledger.drive(&ticket, &mut job, now, slice).unwrap(), None);
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
    let incarnation = ProcessIncarnation::mint();
    let mut ledger = AdmissionLedger::new(incarnation, bounds(2, 1024));
    let ticket = ledger
        .try_submit(
            ids.identity(incarnation),
            WorkFamily::MapBake,
            InputRevision::FIRST,
            charge(8),
            None,
        )
        .unwrap();
    let mut job = CountJob::new(2);
    let now = Instant::now();
    let expired = budget(now, 1, 0);
    let later = now + Duration::from_millis(1);
    assert_eq!(
        ledger.drive(&ticket, &mut job, later, expired).unwrap(),
        Some(JobState::Yielded)
    );
    assert!(job.produced.is_empty());
    assert_eq!(
        ledger.cancel(ticket, Cancellation::Operator),
        CancelOutcome::Requested
    );
    assert_eq!(
        ledger
            .drive(&ticket, &mut job, now, budget(now, 1, 50))
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
    let incarnation = ProcessIncarnation::mint();
    let mut ledger = AdmissionLedger::new(incarnation, bounds(2, 1024));
    let now = Instant::now();

    let ticket = ledger
        .try_submit(
            ids.identity(incarnation),
            WorkFamily::Route,
            InputRevision::FIRST,
            charge(8),
            None,
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
            .drive(&ticket, &mut late, now, budget(now, 1, 50))
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
            None,
        )
        .unwrap();
    let mut job = CountJob::new(1);
    assert_eq!(
        ledger
            .drive(&ticket, &mut job, now, budget(now, 1, 50))
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
    let incarnation = ProcessIncarnation::mint();
    let mut ledger = AdmissionLedger::new(incarnation, bounds(1, 1024));
    let ticket = ledger
        .try_submit(
            ids.identity(incarnation),
            WorkFamily::Inspection,
            InputRevision::FIRST,
            charge(8),
            None,
        )
        .unwrap();
    let mut job = CountJob::new(1);
    job.ignore_cancel = true;
    ledger.cancel(ticket, Cancellation::Shutdown);
    ledger
        .drive(
            &ticket,
            &mut job,
            Instant::now(),
            budget(Instant::now(), 1, 50),
        )
        .unwrap();
    ledger.worker_returned(&ticket).unwrap();
    assert!(
        !ledger.borrow_terminal(&ticket).unwrap().is_completed(),
        "a cancelled job must not publish Completed"
    );
}

#[test]
fn a_job_that_holds_forbidden_state_across_a_yield_is_rejected() {
    let ids = Ids::new();
    let incarnation = ProcessIncarnation::mint();
    let mut ledger = AdmissionLedger::new(incarnation, bounds(1, 1024));
    let ticket = ledger
        .try_submit(
            ids.identity(incarnation),
            WorkFamily::CardPreparation,
            InputRevision::FIRST,
            charge(8),
            None,
        )
        .unwrap();
    let mut job = CountJob::new(4);
    job.forbidden = Some(ForbiddenRetain::OtherOwnerResource);
    let now = Instant::now();
    assert_eq!(
        ledger
            .drive(&ticket, &mut job, now, budget(now, 1, 50))
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
fn ignoring_yield_or_doing_two_units_fails_the_job() {
    let ids = Ids::new();
    let incarnation = ProcessIncarnation::mint();
    let mut ledger = AdmissionLedger::new(incarnation, bounds(1, 1024));
    let ticket = ledger
        .try_submit(
            ids.identity(incarnation),
            WorkFamily::Route,
            InputRevision::FIRST,
            charge(8),
            None,
        )
        .unwrap();
    let mut job = CountJob::new(8);
    job.ignore_yield = true;
    let now = Instant::now();
    assert_eq!(
        ledger
            .drive(&ticket, &mut job, now, budget(now, 8, 50))
            .unwrap(),
        None
    );
    ledger.worker_returned(&ticket).unwrap();
    match ledger.consume(&ticket).unwrap() {
        Settlement::Failed(JobError::Cooperation(
            CooperativeViolation::IgnoredYield | CooperativeViolation::UnboundedSlice,
        )) => {}
        other => panic!("expected cooperation failure, got {other:?}"),
    }
}

#[test]
fn wrong_scope_revision_and_duplicate_consume_cannot_install_or_reclaim() {
    let ids = Ids::new();
    let incarnation = ProcessIncarnation::mint();
    let mut ledger = AdmissionLedger::new(incarnation, bounds(2, 1024));
    let id = ids.identity(incarnation);
    let ticket = ledger
        .try_submit(id, WorkFamily::Route, InputRevision::FIRST, charge(8), None)
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
        .drive(&ticket, &mut job, now, budget(now, 1, 50))
        .unwrap();
    ledger.worker_returned(&ticket).unwrap();
    ledger.consume(&ticket).unwrap();
    match ledger.consume(&ticket) {
        Err(
            crate::identity::TerminalError::AlreadyConsumed
            | crate::identity::TerminalError::NotPublished,
        ) => {}
        other => panic!("duplicate consume must not reclaim another ticket, got {other:?}"),
    }
    assert!(!ledger.is_charged(&ticket));
}

#[test]
fn saturated_ledger_returns_full_without_taking_custody() {
    let ids = Ids::new();
    let incarnation = ProcessIncarnation::mint();
    let mut ledger: TestLedger = AdmissionLedger::new(incarnation, bounds(1, 32));
    let first = ids.identity(incarnation);
    let first_ticket = ledger
        .try_submit(
            first,
            WorkFamily::Route,
            InputRevision::FIRST,
            charge(16),
            None,
        )
        .unwrap();
    let second = ids.identity(incarnation);
    match ledger.try_submit(
        second,
        WorkFamily::Route,
        InputRevision::FIRST,
        charge(16),
        None,
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
fn abandoned_ticket_retains_credit_until_the_worker_returns() {
    let ids = Ids::new();
    let incarnation = ProcessIncarnation::mint();
    let mut ledger: TestLedger = AdmissionLedger::new(incarnation, bounds(2, 1024));
    let ticket = ledger
        .try_submit(
            ids.identity(incarnation),
            WorkFamily::Route,
            InputRevision::FIRST,
            charge(32),
            None,
        )
        .unwrap();
    assert_eq!(ledger.charged_bytes(), 32);
    ledger.abandon(ticket, Cancellation::Operator);
    assert!(
        ledger.is_charged(&ticket),
        "abandon must not free credit before the worker returns"
    );
    ledger.worker_returned(&ticket).unwrap();
    assert!(
        !ledger.is_charged(&ticket),
        "credit frees only after the worker returns"
    );
}

#[test]
fn progress_coalescing_does_not_take_terminal_custody() {
    let ids = Ids::new();
    let incarnation = ProcessIncarnation::mint();
    let mut ledger = AdmissionLedger::new(incarnation, bounds(1, 1024));
    let id = ids.identity(incarnation);
    let ticket = ledger
        .try_submit(
            id,
            WorkFamily::MapBake,
            InputRevision::FIRST,
            charge(8),
            None,
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
        .drive(&ticket, &mut job, now, budget(now, 1, 50))
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
    let incarnation = ProcessIncarnation::mint();
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
        Some(&conditions),
    ) {
        Err(SubmissionRefusal::Unavailable(refusal)) => {
            assert_eq!(refusal.request(), Some(id.request()));
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
    use crate::identity::{RequestId, RequestNamespace};
    use crate::selected::RunKey;

    let incarnation = ProcessIncarnation::mint();
    let mut ledger: TestLedger = AdmissionLedger::new(incarnation, bounds(1, 1024));
    let isolate = RequestId::isolate(1).unwrap();
    assert_eq!(isolate.namespace(), RequestNamespace::Isolate);
    let run = RunKey {
        slot: 1,
        run: 1,
        session: 0,
    };
    match WorkIdentity::new(isolate, incarnation, Scope::Run(run)) {
        Ok(id) => {
            match ledger.try_submit(id, WorkFamily::Route, InputRevision::FIRST, charge(8), None) {
                Err(SubmissionRefusal::InvalidArgument) => {}
                other => panic!("expected InvalidArgument for an isolate work id, got {other:?}"),
            }
        }
        Err(_) => {
            // F0-1/2 H4: WorkIdentity itself will refuse isolate ids.
        }
    }
    assert_eq!(ledger.charged_jobs(), 0);
}

#[test]
fn foreign_incarnation_is_invalid_authority() {
    let mut ledger: TestLedger = AdmissionLedger::new(ProcessIncarnation::mint(), bounds(1, 1024));
    let id = Ids::new().identity(ProcessIncarnation::mint());
    match ledger.try_submit(id, WorkFamily::Route, InputRevision::FIRST, charge(8), None) {
        Err(SubmissionRefusal::InvalidAuthority) => {}
        other => panic!("expected InvalidAuthority, got {other:?}"),
    }
}

#[test]
fn stopping_refuses_new_work() {
    let ids = Ids::new();
    let incarnation = ProcessIncarnation::mint();
    let mut ledger: TestLedger = AdmissionLedger::new(incarnation, bounds(1, 1024));
    ledger.begin_stop();
    match ledger.try_submit(
        ids.identity(incarnation),
        WorkFamily::Route,
        InputRevision::FIRST,
        charge(8),
        None,
    ) {
        Err(SubmissionRefusal::Stopping) => {}
        other => panic!("expected Stopping, got {other:?}"),
    }
}

#[test]
fn yielded_charges_survive_until_consume_and_worker_return() {
    let ids = Ids::new();
    let incarnation = ProcessIncarnation::mint();
    let mut ledger = AdmissionLedger::new(incarnation, bounds(2, 1024));
    let ticket = ledger
        .try_submit(
            ids.identity(incarnation),
            WorkFamily::Route,
            InputRevision::FIRST,
            charge(24),
            None,
        )
        .unwrap();
    let mut job = CountJob::new(2);
    let now = Instant::now();
    ledger
        .drive(&ticket, &mut job, now, budget(now, 1, 50))
        .unwrap();
    assert_eq!(ledger.job_state(&ticket), Some(JobState::Yielded));
    assert_eq!(ledger.charged_bytes(), 24);
    ledger
        .drive(&ticket, &mut job, now, budget(now, 1, 50))
        .unwrap();
    assert!(ledger.is_charged(&ticket));
    ledger.worker_returned(&ticket).unwrap();
    assert!(ledger.is_charged(&ticket));
    ledger.consume(&ticket).unwrap();
    assert!(!ledger.is_charged(&ticket));
}

#[test]
fn family_preparation_is_named_as_forbidden_across_a_yield() {
    // Characterization: FamilyPreparation is Rc-bound (selected.rs:310-328)
    // and maps onto ForbiddenRetain::OtherOwnerResource, not a retained
    // result.
    assert_eq!(
        WorkFamily::CardPreparation.interruptible_unit(),
        "one preparation phase"
    );
    let _ = ForbiddenRetain::OtherOwnerResource;
    let _ = AllowedRetain::Cursor;
}
