use api::identity::{
    Cancellation, Event, ProcessIncarnation, ProcessTime, RequestId, RequestIdMinter, Scope,
    Settlement, SlotKey, SlotKeyMinter,
};
use api::lifecycle::{
    Intent, IntentMailbox, LifecycleFact, MemoryMode, SessionEventOut, WorldChoice,
};
use api::process_condition::{
    ConditionCause, ConditionId, ProcessCapability, ProcessConditions, ProcessServiceCode,
    ProcessServiceCondition,
};

use super::*;
use crate::operations::ActionKind;
use crate::selection::ProfileIdentity;

struct Fixture {
    requests: RequestIdMinter,
    slots: SlotKeyMinter,
    incarnation: ProcessIncarnation,
    conditions: ProcessConditions,
    book: CommandBook,
}

impl Fixture {
    fn new() -> Self {
        let incarnation = ProcessIncarnation::mint();
        Self {
            requests: RequestIdMinter::new(),
            slots: SlotKeyMinter::new(),
            incarnation,
            conditions: ProcessConditions::new(incarnation),
            book: CommandBook::new(),
        }
    }

    fn slot(&self) -> SlotKey {
        self.slots.mint().unwrap()
    }

    fn open(&mut self, command: OperatorCommand) -> RequestId {
        let request = self.requests.mint().unwrap();
        self.book.open(request, command).unwrap();
        request
    }

    /// Open `command`, admit `who` on `slot` and, for a mailbox row, post
    /// it to `mailbox` and record the receipt: what T3.3's dispatch does.
    fn dispatch(
        &mut self,
        command: OperatorCommand,
        who: ProfileIdentity,
        slot: SlotKey,
        mailbox: &mut IntentMailbox,
    ) -> RequestId {
        let request = self.open(command);
        let admission = self
            .book
            .admit(request, who, Scope::Slot(slot), &self.conditions)
            .unwrap();
        if admission == Admission::Accepted {
            if let Some(intent) = command.intent() {
                let receipt = mailbox.post(intent);
                self.book.posted(request, who, slot, receipt).unwrap();
            }
        }
        request
    }

    fn latch(&mut self, code: ProcessServiceCode) -> ConditionId {
        let id = ConditionId::new(self.incarnation, code);
        self.conditions
            .publish(ProcessServiceCondition::new(
                id,
                ConditionCause::Panicked,
                ProcessTime::ZERO,
                None,
            ))
            .unwrap();
        id
    }

    fn terminal(
        &self,
        request: RequestId,
        who: ProfileIdentity,
    ) -> Option<&Settlement<Completion, CommandFailure>> {
        self.book
            .report(request)
            .unwrap()
            .member(who)
            .unwrap()
            .terminal()
    }
}

fn applied(slot: SlotKey, mailbox: &IntentMailbox) -> Event<SessionEventOut> {
    Event::new(
        Scope::Slot(slot),
        None,
        SessionEventOut::IntentApplied {
            generation: mailbox.snapshot().generation(),
        },
    )
}

const ALICE: ProfileIdentity = ProfileIdentity::uid(1);
const BOB: ProfileIdentity = ProfileIdentity::uid(2);
const DONE: Settlement<Completion, CommandFailure> = Settlement::Completed(Completion::Done);

#[test]
fn a_stale_receipt_generation_never_settles_a_newer_pending_report() {
    let mut f = Fixture::new();
    let slot = f.slot();
    let mut mailbox = IntentMailbox::new(false, WorldChoice::Automatic);

    let logout = f.dispatch(OperatorCommand::Logout, ALICE, slot, &mut mailbox);
    let ack_of_logout = applied(slot, &mailbox);
    let login = f.dispatch(OperatorCommand::Login, ALICE, slot, &mut mailbox);

    // The newer Log in superseded the older pending Log out (today
    // `session.rs:958-960`).
    assert_eq!(
        f.terminal(logout, ALICE),
        Some(&Settlement::Cancelled(Cancellation::Superseded))
    );
    // The slot's late acknowledgement of the Log out generation must not
    // complete the newer Log in.
    assert_eq!(f.book.observe(&ack_of_logout), 0);
    assert_eq!(f.terminal(login, ALICE), None);

    // Its own generation settles it, exactly once.
    let ack_of_login = applied(slot, &mailbox);
    assert_eq!(f.book.observe(&ack_of_login), 1);
    assert_eq!(f.terminal(login, ALICE), Some(&DONE));
    assert_eq!(f.book.observe(&ack_of_login), 0);
    assert_eq!(
        f.book.settle(
            login,
            ALICE,
            Settlement::Cancelled(Cancellation::Operator),
            None
        ),
        Err(CommandError::AlreadySettled {
            request: login,
            profile: ALICE
        })
    );
    assert_eq!(f.terminal(login, ALICE), Some(&DONE));
}

#[test]
fn a_later_acknowledgement_settles_earlier_posts_of_its_slot_only() {
    let mut f = Fixture::new();
    let (one, two) = (f.slot(), f.slot());
    let mut first = IntentMailbox::new(false, WorldChoice::Automatic);
    let mut second = IntentMailbox::new(false, WorldChoice::Automatic);

    let login = f.dispatch(OperatorCommand::Login, ALICE, one, &mut first);
    let other = f.dispatch(OperatorCommand::Login, BOB, two, &mut second);
    // A newer post that does not supersede the Log in (auto-login policy).
    first.post(Intent::AutoLogin(true));

    // Generations are per mailbox: slot two acknowledging a number equal to
    // slot one's must not settle slot one's member.
    assert_eq!(f.book.observe(&applied(two, &second)), 1);
    assert_eq!(f.terminal(login, ALICE), None);
    assert_eq!(f.terminal(other, BOB), Some(&DONE));

    // An owner that applied a later generation applied the earlier one too.
    assert_eq!(f.book.observe(&applied(one, &first)), 1);
    assert_eq!(f.terminal(login, ALICE), Some(&DONE));
}

#[test]
fn all_lists_every_row_once() {
    let mut seen = Vec::new();
    for command in OperatorCommand::ALL {
        let row = std::mem::discriminant(&command);
        assert!(!seen.contains(&row), "{command:?} listed twice");
        seen.push(row);
    }
}

#[test]
fn every_intent_applied_row_advances_its_mailbox_generation() {
    // A row settled by IntentApplied whose post did not advance the
    // generation would be settled by the acknowledgement of an older post.
    for command in OperatorCommand::ALL {
        if command.settles() != Settles::IntentApplied {
            continue;
        }
        let intent = command
            .intent()
            .unwrap_or_else(|| panic!("{command:?} settles on IntentApplied but posts no row"));
        for online in [false, true] {
            let mut mailbox = IntentMailbox::new(true, WorldChoice::Automatic);
            if online {
                mailbox.observe(LifecycleFact::LoginSucceeded);
            }
            let before = mailbox.snapshot().generation();
            let receipt = mailbox.post(intent);
            assert!(
                receipt.generation() > before,
                "{command:?} (online {online}) did not advance the generation"
            );
        }
    }
}

#[test]
fn every_current_operation_kind_has_a_command_row() {
    // Exhaustive over today's report kinds (`operations.rs:20-34`): a new
    // kind fails to compile here until it gets a row.
    fn every_kind(kind: ActionKind) -> ActionKind {
        match kind {
            ActionKind::Select
            | ActionKind::Load
            | ActionKind::Remove
            | ActionKind::Login
            | ActionKind::Logout
            | ActionKind::ScriptStart
            | ActionKind::ScriptPause
            | ActionKind::ScriptResume
            | ActionKind::ScriptStop
            | ActionKind::SaveProfile
            | ActionKind::DeleteProfile
            | ActionKind::SyncSettings => kind,
        }
    }
    let kinds = [
        ActionKind::Select,
        ActionKind::Load,
        ActionKind::Remove,
        ActionKind::Login,
        ActionKind::Logout,
        ActionKind::ScriptStart,
        ActionKind::ScriptPause,
        ActionKind::ScriptResume,
        ActionKind::ScriptStop,
        ActionKind::SaveProfile,
        ActionKind::DeleteProfile,
        ActionKind::SyncSettings,
    ];
    for kind in kinds.map(every_kind) {
        assert!(
            OperatorCommand::ALL
                .iter()
                .any(|command| command.action_kind() == Some(kind)),
            "{kind:?} has no OperatorCommand row"
        );
    }
}

#[test]
fn the_memory_relog_reports_as_todays_logout_operation() {
    // `request_memory_relog` opens a Logout op (`session.rs:1110`) and the
    // Remove of a slot cancels only the pending Log in (`session.rs:1214`).
    assert_eq!(
        OperatorCommand::RelogNow.action_kind(),
        Some(ActionKind::Logout)
    );
    assert!(OperatorCommand::Remove.supersedes(OperatorCommand::Login));
    assert!(!OperatorCommand::Remove.supersedes(OperatorCommand::Logout));
    assert!(OperatorCommand::Logout.supersedes(OperatorCommand::Login));
    assert!(OperatorCommand::Login.supersedes(OperatorCommand::Logout));
    assert!(!OperatorCommand::StopScript.supersedes(OperatorCommand::Login));
}

#[test]
fn supersession_is_per_member() {
    let mut f = Fixture::new();
    let (one, two) = (f.slot(), f.slot());
    let mut first = IntentMailbox::new(false, WorldChoice::Automatic);
    let mut second = IntentMailbox::new(false, WorldChoice::Automatic);

    let login_all = f.open(OperatorCommand::Login);
    for (who, slot, mailbox) in [(ALICE, one, &mut first), (BOB, two, &mut second)] {
        f.book
            .admit(login_all, who, Scope::Slot(slot), &f.conditions)
            .unwrap();
        let receipt = mailbox.post(Intent::Login);
        f.book.posted(login_all, who, slot, receipt).unwrap();
    }
    f.dispatch(OperatorCommand::Logout, BOB, two, &mut second);

    assert_eq!(f.terminal(login_all, ALICE), None);
    assert_eq!(
        f.terminal(login_all, BOB),
        Some(&Settlement::Cancelled(Cancellation::Superseded))
    );
    assert!(!f.book.report(login_all).unwrap().is_settled());
}

#[test]
fn a_withdrawn_login_settles_by_why_it_was_withdrawn() {
    let mut f = Fixture::new();
    let slot = f.slot();
    let mut mailbox = IntentMailbox::new(false, WorldChoice::World(5));
    let login = f.dispatch(OperatorCommand::Login, ALICE, slot, &mut mailbox);

    // While still wanted, the snapshot settles nothing.
    assert_eq!(f.book.observe_intent(slot, &mailbox.snapshot()), 0);

    // The world was refused: the login is held until the operator changes
    // something, a typed failure rather than a cancellation.
    mailbox.observe(LifecycleFact::WorldPreferenceFailed);
    assert_eq!(f.book.observe_intent(slot, &mailbox.snapshot()), 1);
    assert_eq!(
        f.terminal(login, ALICE),
        Some(&Settlement::Failed(CommandFailure::Failed(
            FailureKind::WorldRejected
        )))
    );

    // A new Log in lifts the hold; the repeat guard then latches it off,
    // which cancels (today `arm.login_latched()`, `session.rs:1641-1644`).
    let again = f.dispatch(OperatorCommand::Login, ALICE, slot, &mut mailbox);
    mailbox.observe(LifecycleFact::RepeatGuardTripped {
        count: 3,
        window_seconds: 60,
    });
    assert_eq!(f.book.observe_intent(slot, &mailbox.snapshot()), 1);
    assert_eq!(
        f.terminal(again, ALICE),
        Some(&Settlement::Cancelled(Cancellation::Superseded))
    );

    // A Log in posted after the latch rearms: the snapshot no longer
    // withdraws it.
    let third = f.dispatch(OperatorCommand::Login, ALICE, slot, &mut mailbox);
    assert_eq!(f.book.observe_intent(slot, &mailbox.snapshot()), 0);
    assert_eq!(f.terminal(third, ALICE), None);
}

#[test]
fn a_retired_slot_completes_withdrawals_and_invalidates_the_rest() {
    let mut f = Fixture::new();
    let (gone, kept) = (f.slot(), f.slot());
    let mut mailbox = IntentMailbox::new(false, WorldChoice::Automatic);
    let mut other = IntentMailbox::new(false, WorldChoice::Automatic);
    mailbox.observe(LifecycleFact::LoginSucceeded);

    // Remove closes the slot's mailbox lifetime, so it is posted last.
    let pause = f.dispatch(OperatorCommand::PauseScript, ALICE, gone, &mut mailbox);
    let remove = f.dispatch(OperatorCommand::Remove, ALICE, gone, &mut mailbox);
    let login = f.dispatch(OperatorCommand::Login, BOB, kept, &mut other);
    let start = f.open(OperatorCommand::StartScript);
    f.book
        .admit(start, ALICE, Scope::Slot(gone), &f.conditions)
        .unwrap();

    assert_eq!(f.book.slot_retired(gone), 3);
    assert_eq!(f.terminal(remove, ALICE), Some(&DONE));
    assert_eq!(
        f.terminal(pause, ALICE),
        Some(&Settlement::Completed(Completion::Skipped(
            SkipReason::NoRunningScript
        )))
    );
    assert_eq!(
        f.terminal(start, ALICE),
        Some(&Settlement::Cancelled(Cancellation::ScopeInvalidated))
    );
    assert_eq!(f.terminal(login, BOB), None);
}

#[test]
fn each_condition_refuses_exactly_the_commands_that_need_it() {
    for code in ProcessServiceCode::ALL {
        let mut f = Fixture::new();
        let condition = f.latch(code);
        for command in OperatorCommand::ALL {
            let slot = f.slot();
            let request = f.open(command);
            let admission = f
                .book
                .admit(request, ALICE, Scope::Slot(slot), &f.conditions)
                .unwrap();
            let needed = command
                .capabilities()
                .iter()
                .copied()
                .find(|&capability| code.affects(capability));
            match needed {
                Some(capability) if !command.retirement_dependent() => {
                    assert_eq!(
                        admission,
                        Admission::Refused {
                            condition,
                            capability
                        },
                        "{code:?} must refuse {command:?}"
                    );
                    let member = f.book.report(request).unwrap().member(ALICE).unwrap();
                    assert_eq!(member.pending(), None, "a refusal is never Pending");
                    assert_eq!(member.scope(), Scope::Slot(slot));
                    let Some(Settlement::Failed(failure)) = member.terminal() else {
                        panic!("{command:?} refused without a Failed terminal");
                    };
                    assert_eq!(failure.condition(), Some(condition));
                    assert_eq!(failure.operator_text(), Some(code.operator_text()));
                }
                _ => {
                    assert_eq!(
                        admission,
                        Admission::Accepted,
                        "{code:?} refused {command:?}"
                    );
                    let member = f.book.report(request).unwrap().member(ALICE).unwrap();
                    assert_eq!(member.pending(), Some(Pending::Accepted));
                }
            }
        }
    }
}

#[test]
fn every_code_refuses_some_command_and_spares_profile_edits() {
    for code in ProcessServiceCode::ALL {
        assert!(
            OperatorCommand::ALL
                .iter()
                .any(|command| !command.retirement_dependent()
                    && command.capabilities().iter().any(|&c| code.affects(c))),
            "no command is refused by {code:?}"
        );
        for local in [
            OperatorCommand::SaveProfile,
            OperatorCommand::Select,
            OperatorCommand::SetMark(true),
            OperatorCommand::SetNavPreference(crate::NavPreference::AllowTeleports, false),
        ] {
            assert!(!local.capabilities().iter().any(|&c| code.affects(c)));
        }
    }
    assert!(OperatorCommand::Login
        .capabilities()
        .contains(&ProcessCapability::Login));
    assert!(OperatorCommand::StartScript
        .capabilities()
        .contains(&ProcessCapability::BackgroundWork));
}

#[test]
fn logout_stop_and_remove_wait_for_retirement_through_an_outage() {
    let mut f = Fixture::new();
    let slot = f.slot();
    let mut mailbox = IntentMailbox::new(true, WorldChoice::Automatic);
    mailbox.observe(LifecycleFact::LoginSucceeded);
    let reactor = f.latch(ProcessServiceCode::ReactorUnavailable);

    let logout = f.dispatch(OperatorCommand::Logout, ALICE, slot, &mut mailbox);
    let stop = f.dispatch(OperatorCommand::StopScript, ALICE, slot, &mut mailbox);
    let remove = f.dispatch(OperatorCommand::Remove, BOB, slot, &mut mailbox);

    // Admitted, not refused, and not turned into a failure or a retry.
    assert_eq!(f.book.fail_unavailable(&f.conditions), 0);
    for (request, who) in [(logout, ALICE), (stop, ALICE), (remove, BOB)] {
        assert_eq!(f.terminal(request, who), None);
    }
    // Their status exposes the blocking condition while they wait.
    assert_eq!(
        f.book.blocked_by(logout, ALICE, &f.conditions),
        Some(reactor)
    );
    assert_eq!(f.book.blocked_by(stop, ALICE, &f.conditions), None);
    // Completion still needs the real retirement predicate.
    assert_eq!(f.book.slot_retired(slot), 3);
    assert_eq!(f.terminal(logout, ALICE), Some(&DONE));
    assert_eq!(f.book.blocked_by(logout, ALICE, &f.conditions), None);
}

#[test]
fn an_outage_fails_accepted_work_once_with_its_identity() {
    let mut f = Fixture::new();
    let slot = f.slot();
    let mut mailbox = IntentMailbox::new(false, WorldChoice::Automatic);
    let start = f.dispatch(OperatorCommand::StartScript, ALICE, slot, &mut mailbox);
    let login = f.dispatch(OperatorCommand::Login, BOB, slot, &mut mailbox);
    let save = f.dispatch(OperatorCommand::SaveProfile, BOB, slot, &mut mailbox);

    let work = f.latch(ProcessServiceCode::WorkAdmissionUnavailable);
    assert_eq!(f.book.fail_unavailable(&f.conditions), 1);
    let Some(Settlement::Failed(CommandFailure::Unavailable(terminal))) = f.terminal(start, ALICE)
    else {
        panic!("Start did not fail Unavailable");
    };
    assert_eq!(terminal.condition(), work);
    assert_eq!(terminal.request(), start);
    assert_eq!(terminal.scope(), Scope::Slot(slot));
    // Unaffected commands continue under their owners.
    assert_eq!(f.terminal(login, BOB), None);
    assert_eq!(f.terminal(save, BOB), None);

    // Late owner publication cannot replace the terminal.
    assert!(f.book.settle(start, ALICE, DONE, None).is_err());
    assert_eq!(f.book.fail_unavailable(&f.conditions), 0);
    assert!(matches!(
        f.terminal(start, ALICE),
        Some(Settlement::Failed(CommandFailure::Unavailable(_)))
    ));
}

#[test]
fn command_identity_is_a_unique_host_request() {
    let mut f = Fixture::new();
    let isolate = RequestId::isolate(7).unwrap();
    assert_eq!(
        f.book.open(isolate, OperatorCommand::Login),
        Err(CommandError::IsolateRequest(isolate))
    );
    let request = f.open(OperatorCommand::Login);
    assert_eq!(
        f.book.open(request, OperatorCommand::Logout),
        Err(CommandError::DuplicateRequest(request))
    );
    let slot = f.slot();
    f.book
        .admit(request, ALICE, Scope::Slot(slot), &f.conditions)
        .unwrap();
    assert_eq!(
        f.book
            .admit(request, ALICE, Scope::Slot(slot), &f.conditions),
        Err(CommandError::DuplicateMember {
            request,
            profile: ALICE
        })
    );
    let mut mailbox = IntentMailbox::new(false, WorldChoice::Automatic);
    let receipt = mailbox.post(Intent::MemoryMode(MemoryMode::Low));
    let save = f.open(OperatorCommand::SaveProfile);
    f.book
        .admit(save, ALICE, Scope::Slot(slot), &f.conditions)
        .unwrap();
    assert_eq!(
        f.book.posted(save, ALICE, slot, receipt),
        Err(CommandError::NoIntentRow(save))
    );
}

#[test]
fn settled_history_is_bounded_and_pending_reports_are_never_evicted() {
    let mut f = Fixture::new();
    let slot = f.slot();
    let pending = f.open(OperatorCommand::Login);
    f.book
        .admit(pending, ALICE, Scope::Slot(slot), &f.conditions)
        .unwrap();
    let mut settled = Vec::new();
    for _ in 0..SETTLED_REPORT_CAP + 5 {
        let request = f.open(OperatorCommand::Select);
        f.book
            .admit(request, ALICE, Scope::Slot(slot), &f.conditions)
            .unwrap();
        f.book.settle(request, ALICE, DONE, None).unwrap();
        settled.push(request);
    }
    assert!(f.book.report(pending).is_some());
    assert_eq!(
        f.book.reports().filter(|r| r.is_settled()).count(),
        SETTLED_REPORT_CAP
    );
    assert!(f.book.report(settled[0]).is_none());
    assert!(f.book.report(*settled.last().unwrap()).is_some());
}
