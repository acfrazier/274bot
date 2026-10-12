use std::num::NonZeroU64;

use api::identity::{
    Cancellation, Event, ProcessIncarnation, ProcessTime, RequestId, RequestIdMinter, Scope,
    SessionKey, Settlement, SlotKey, SlotKeyMinter,
};
use api::lifecycle::{
    Intent, IntentMailbox, LifecycleFact, MailboxError, MailboxGeneration, MemoryMode,
    SessionEndCause, SessionEventOut, WorldChoice,
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
        let incarnation = ProcessIncarnation::from_start_nonce(NonZeroU64::MIN);
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

    fn open(&mut self, command: OperatorCommand, targets: &[(Subject, Scope)]) -> RequestId {
        let request = self.requests.mint().unwrap();
        self.book
            .open(request, command, targets, &self.conditions)
            .unwrap();
        request
    }

    /// The one-member application command `command`.
    fn open_app(&mut self, command: OperatorCommand) -> RequestId {
        self.open(command, &[(Subject::Application, Scope::Process)])
    }

    /// Open `command` for `who` on `slot` and, for an accepted mailbox row,
    /// post it to `mailbox` and record the receipt or the refusal: what
    /// T3.3's dispatch does.
    fn dispatch(
        &mut self,
        command: OperatorCommand,
        who: Subject,
        slot: SlotKey,
        mailbox: &mut IntentMailbox,
    ) -> RequestId {
        let request = self.open(command, &[(who, Scope::Slot(slot))]);
        let accepted = self.member(request, who).pending().is_some();
        if let (true, Some(intent)) = (accepted, command.intent()) {
            match mailbox.post(intent) {
                Ok(receipt) => self.book.posted(request, who, slot, receipt).unwrap(),
                Err(error) => self.book.post_refused(request, who, slot, error).unwrap(),
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

    fn member(&self, request: RequestId, who: Subject) -> &MemberReport {
        self.book.report(request).unwrap().member(who).unwrap()
    }

    fn terminal(
        &self,
        request: RequestId,
        who: Subject,
    ) -> Option<&Settlement<Completion, CommandFailure>> {
        self.member(request, who).terminal()
    }

    fn settled_count(&self) -> usize {
        self.book.reports().filter(|r| r.is_settled()).count()
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

fn unavailable(
    slot: SlotKey,
    capability: ProcessCapability,
    condition: ConditionId,
) -> Event<SessionEventOut> {
    Event::new(
        Scope::Slot(slot),
        None,
        SessionEventOut::Unavailable {
            capability,
            condition,
        },
    )
}

fn unavailable_terminal(
    terminal: Option<&Settlement<Completion, CommandFailure>>,
) -> UnavailableTerminal {
    match terminal {
        Some(Settlement::Failed(CommandFailure::Unavailable(terminal))) => *terminal,
        other => panic!("expected a Failed/Unavailable terminal, got {other:?}"),
    }
}

fn online(auto_login: bool) -> IntentMailbox {
    let mut mailbox = IntentMailbox::new(auto_login, WorldChoice::Automatic);
    mailbox.observe(LifecycleFact::LoginSucceeded).unwrap();
    mailbox
}

const ALICE: Subject = Subject::Profile(ProfileIdentity::uid(1));
const BOB: Subject = Subject::Profile(ProfileIdentity::uid(2));
const CAROL: Subject = Subject::Profile(ProfileIdentity::uid(3));
const DONE: Settlement<Completion, CommandFailure> = Settlement::Completed(Completion::Done);
const SUPERSEDED: Settlement<Completion, CommandFailure> =
    Settlement::Cancelled(Cancellation::Superseded);

// --- Vocabulary columns ---

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

/// One row's independently specified columns: (row, subject, settles,
/// capability needed at admission, today's report).
type Columns = (
    OperatorCommand,
    SubjectKind,
    Settles,
    Option<ProcessCapability>,
    Option<ActionKind>,
);

/// The application decisions the r1 review found missing (D7 / `00-contract.md:52`:
/// fleet model, dirty-form workflow, catalogue preparation and loadout draft
/// are core-owned), each with its independently specified columns.
#[test]
fn the_core_owned_application_decisions_have_their_frozen_columns() {
    use OperatorCommand as C;
    use ProcessCapability as P;
    use Settles::{AtDispatch, Owner};
    use SubjectKind::{Application, Profile};
    let rows: [Columns; 16] = [
        (C::SetFleetFilter, Application, AtDispatch, None, None),
        (
            C::SetFleetSort(FleetOrder::Name),
            Application,
            AtDispatch,
            None,
            None,
        ),
        (C::SelectScriptCard, Profile, AtDispatch, None, None),
        (
            C::TranspileAll,
            Application,
            Owner,
            Some(P::BackgroundWork),
            None,
        ),
        (C::DeferCatalogImport, Application, Owner, None, None),
        (C::OpenProfileForm, Profile, AtDispatch, None, None),
        (C::OpenNewProfileForm, Application, AtDispatch, None, None),
        (
            C::EditProfileDraft(ProfileDraftField::Password),
            Application,
            AtDispatch,
            None,
            None,
        ),
        (C::KeepEditing, Application, AtDispatch, None, None),
        (
            C::CloseProfileForm(FormClose::Window),
            Application,
            AtDispatch,
            None,
            None,
        ),
        (
            C::MarkTutorialSkipped,
            Profile,
            Owner,
            None,
            Some(ActionKind::SaveProfile),
        ),
        (C::SelectLoadout, Application, AtDispatch, None, None),
        (
            C::EditLoadoutDraft(LoadoutField::Name),
            Application,
            AtDispatch,
            None,
            None,
        ),
        (C::CopyEquipmentToLoadout, Profile, AtDispatch, None, None),
        (C::CreateLoadout, Application, Owner, None, None),
        (C::DuplicateLoadout, Application, Owner, None, None),
    ];
    for (row, subject, settles, capability, kind) in rows {
        assert!(
            OperatorCommand::ALL
                .iter()
                .any(|c| std::mem::discriminant(c) == std::mem::discriminant(&row)),
            "{row:?} missing from ALL"
        );
        assert_eq!(row.subject(), subject, "{row:?} subject");
        assert_eq!(row.settles(), settles, "{row:?} settlement");
        assert_eq!(
            row.capabilities(),
            capability.as_slice(),
            "{row:?} capability"
        );
        assert_eq!(row.action_kind(), kind, "{row:?} report kind");
        // None of them is an intent-mailbox row or waits on retirement.
        assert_eq!(row.intent(), None, "{row:?} posts an intent");
        assert!(!row.retirement_dependent());
    }
    // A Browse pick is not an assignment: only Assign writes durably.
    assert_eq!(OperatorCommand::AssignScript.settles(), Owner);
    // Loadout create/duplicate are durable writes like Save and Delete, not
    // draft edits; each is its own row.
    for durable in [
        C::CreateLoadout,
        C::DuplicateLoadout,
        C::SaveLoadout,
        C::DeleteLoadout,
    ] {
        assert_eq!(durable.settles(), Owner, "{durable:?}");
    }
    // TutSkip's network half needs the transport; its durable half does not.
    assert_eq!(C::DebugCommand.capabilities(), [P::NetworkEffect]);
}

#[test]
fn every_row_names_one_subject_kind_and_the_book_enforces_it() {
    // Every mailbox row targets profiles: a slot belongs to a profile.
    for command in OperatorCommand::ALL {
        if command.intent().is_some() {
            assert_eq!(command.subject(), SubjectKind::Profile, "{command:?}");
        }
    }
    let mut f = Fixture::new();
    let slot = f.slot();
    let request = f.requests.mint().unwrap();
    // A profile row refuses the application subject, and the reverse.
    assert_eq!(
        f.book.open(
            request,
            OperatorCommand::SelectScriptCard,
            &[(Subject::Application, Scope::Process)],
            &f.conditions
        ),
        Err(CommandError::WrongSubject {
            request,
            subject: Subject::Application,
            scope: Scope::Process
        })
    );
    assert_eq!(
        f.book.open(
            request,
            OperatorCommand::SetFleetSort(FleetOrder::Status),
            &[(ALICE, Scope::Slot(slot))],
            &f.conditions
        ),
        Err(CommandError::WrongSubject {
            request,
            subject: ALICE,
            scope: Scope::Slot(slot)
        })
    );
    // The application member lives in the process scope.
    assert_eq!(
        f.book.open(
            request,
            OperatorCommand::KeepEditing,
            &[(Subject::Application, Scope::Slot(slot))],
            &f.conditions
        ),
        Err(CommandError::WrongSubject {
            request,
            subject: Subject::Application,
            scope: Scope::Slot(slot)
        })
    );
    assert_eq!(
        f.book
            .open(request, OperatorCommand::Login, &[], &f.conditions),
        Err(CommandError::NoMembers(request))
    );
    // None of the refusals opened anything; the identity is still free.
    assert_eq!(f.book.reports().count(), 0);
    f.book
        .open(
            request,
            OperatorCommand::KeepEditing,
            &[(Subject::Application, Scope::Process)],
            &f.conditions,
        )
        .unwrap();
}

#[test]
fn tutskip_is_two_commands_whose_terminals_are_independent() {
    // `app.rs:2100-2104`: the cheat and the durable mark. Under a transport
    // outage the cheat is refused while the mark is still written; a failed
    // write of the mark does not touch the cheat's terminal.
    let mut f = Fixture::new();
    let slot = f.slot();
    let reactor = f.latch(ProcessServiceCode::ReactorUnavailable);
    let cheat = f.open(OperatorCommand::DebugCommand, &[(ALICE, Scope::Slot(slot))]);
    let mark = f.open(
        OperatorCommand::MarkTutorialSkipped,
        &[(ALICE, Scope::Slot(slot))],
    );
    assert_eq!(
        f.terminal(cheat, ALICE),
        Some(&Settlement::Failed(CommandFailure::Refused {
            condition: reactor,
            capability: ProcessCapability::NetworkEffect
        }))
    );
    assert_eq!(f.member(mark, ALICE).pending(), Some(Pending::Accepted));
    assert_eq!(f.book.fail_unavailable(&f.conditions), 0);
    let write_failed = Settlement::Failed(CommandFailure::Failed(FailureKind::WriteFailed));
    f.book
        .settle(mark, ALICE, write_failed.clone(), Some("disk full"))
        .unwrap();
    assert_eq!(f.terminal(mark, ALICE), Some(&write_failed));
    assert_eq!(f.member(mark, ALICE).detail(), Some("disk full"));
    assert!(matches!(
        f.terminal(cheat, ALICE),
        Some(Settlement::Failed(CommandFailure::Refused { .. }))
    ));
}

#[test]
fn profile_form_and_loadout_decisions_settle_by_their_own_rules() {
    let mut f = Fixture::new();
    // A form retarget that finds unsaved edits stages the decision instead.
    let open = f.open(OperatorCommand::OpenProfileForm, &[(BOB, Scope::Process)]);
    let staged = Settlement::Completed(Completion::Skipped(SkipReason::DecisionStaged));
    f.book.settle(open, BOB, staged.clone(), None).unwrap();
    assert_eq!(f.terminal(open, BOB), Some(&staged));
    // Keep editing withdraws it; it is a command with a terminal, not a no-op.
    let keep = f.open_app(OperatorCommand::KeepEditing);
    assert_eq!(
        f.member(keep, Subject::Application).pending(),
        Some(Pending::Accepted)
    );
    f.book
        .settle(keep, Subject::Application, DONE, None)
        .unwrap();
    // A loadout create waits for its durable write and reports its failure.
    let create = f.open_app(OperatorCommand::CreateLoadout);
    assert!(!f.book.report(create).unwrap().is_settled());
    let failed = Settlement::Failed(CommandFailure::Failed(FailureKind::WriteFailed));
    f.book
        .settle(create, Subject::Application, failed.clone(), None)
        .unwrap();
    assert_eq!(f.terminal(create, Subject::Application), Some(&failed));
    // Duplicate with nothing selected is skipped, typed.
    let duplicate = f.open_app(OperatorCommand::DuplicateLoadout);
    let nothing = Settlement::Completed(Completion::Skipped(SkipReason::NothingSelected));
    f.book
        .settle(duplicate, Subject::Application, nothing.clone(), None)
        .unwrap();
    assert_eq!(f.terminal(duplicate, Subject::Application), Some(&nothing));
}

// --- Process conditions: independent capability/consumer matrix ---

/// The rows each code refuses at admission, listed independently of
/// `capabilities()` (§26 §3.1 affected capabilities over the current
/// rows): transport-dependent rows for the reactor, unresolved logins for
/// the resolver, background-job rows for work admission. Retirement rows
/// (Logout, Remove, Stop, Quit) are never refused.
fn refused_by(code: ProcessServiceCode) -> Vec<OperatorCommand> {
    use OperatorCommand as C;
    match code {
        ProcessServiceCode::ReactorUnavailable => vec![
            C::Login,
            C::RelogNow,
            C::SwitchWorld(1),
            C::ScriptPaintInput,
            C::WalkTo,
            C::GroupWalk,
            C::Teleport,
            C::DebugCommand,
            C::GameInput,
        ],
        ProcessServiceCode::ResolverUnavailable => vec![C::Login, C::RelogNow, C::SwitchWorld(1)],
        ProcessServiceCode::WorkAdmissionUnavailable => vec![
            C::StartScript,
            C::AssignAndRestart,
            C::SaveScriptSettings,
            C::ApplySettingsToMarked,
            C::RestartForSettings,
            C::LoadScriptFile,
            C::ReloadScripts,
            C::ConfirmReload,
            C::ImportCatalog,
            C::TranspileAll,
            C::WalkTo,
            C::GroupWalk,
            C::BakeMapNow,
            C::ReloadQuesterPaths,
        ],
    }
}

fn target_of(f: &Fixture, command: OperatorCommand) -> (Subject, Scope) {
    match command.subject() {
        SubjectKind::Profile => (ALICE, Scope::Slot(f.slot())),
        SubjectKind::Application => (Subject::Application, Scope::Process),
    }
}

#[test]
fn each_code_refuses_exactly_its_independent_row_set() {
    for code in ProcessServiceCode::ALL {
        let mut f = Fixture::new();
        let condition = f.latch(code);
        let expected = refused_by(code);
        for command in OperatorCommand::ALL {
            let target = target_of(&f, command);
            let request = f.open(command, &[target]);
            let member = f.member(request, target.0);
            if expected.contains(&command) {
                assert_eq!(member.pending(), None, "{code:?} must refuse {command:?}");
                assert_eq!(member.scope(), target.1, "a refusal keeps its scope");
                let Some(Settlement::Failed(failure @ CommandFailure::Refused { .. })) =
                    member.terminal()
                else {
                    panic!("{command:?} refused without a Refused terminal");
                };
                assert_eq!(failure.condition(), Some(condition));
                assert_eq!(failure.operator_text(), Some(code.operator_text()));
            } else {
                assert_eq!(
                    member.pending(),
                    Some(Pending::Accepted),
                    "{code:?} refused {command:?}"
                );
            }
        }
    }
}

#[test]
fn only_a_resolved_login_stops_needing_the_resolver() {
    use ProcessCapability as P;
    for command in OperatorCommand::ALL {
        let resolved = command.needs(Stage::AddressResolved);
        assert_eq!(command.needs(Stage::Admitted), command.capabilities());
        if matches!(
            command,
            OperatorCommand::Login | OperatorCommand::RelogNow | OperatorCommand::SwitchWorld(_)
        ) {
            assert_eq!(resolved, [P::NewConnection, P::Login, P::ResolvedAddress]);
        } else {
            assert_eq!(resolved, command.capabilities(), "{command:?}");
        }
    }
}

#[test]
fn a_resolver_outage_fails_an_unresolved_login_and_spares_a_resolved_one() {
    let mut f = Fixture::new();
    let (unresolved_slot, resolved_slot) = (f.slot(), f.slot());
    let mut unresolved_box = IntentMailbox::new(false, WorldChoice::Automatic);
    let mut resolved_box = IntentMailbox::new(false, WorldChoice::Automatic);
    let unresolved = f.dispatch(
        OperatorCommand::Login,
        ALICE,
        unresolved_slot,
        &mut unresolved_box,
    );
    let resolved = f.dispatch(
        OperatorCommand::Login,
        BOB,
        resolved_slot,
        &mut resolved_box,
    );
    // The slot owner resolved BOB's handshake address for the attempt it
    // read at the current generation; a later Log in on that slot was not
    // covered by it.
    let through = resolved_box.snapshot().generation();
    assert_eq!(f.book.address_resolved(resolved_slot, through), 1);
    assert_eq!(f.book.address_resolved(resolved_slot, through), 0);
    assert_eq!(f.member(resolved, BOB).stage(), Stage::AddressResolved);
    assert_eq!(f.member(unresolved, ALICE).stage(), Stage::Admitted);

    let resolver = f.latch(ProcessServiceCode::ResolverUnavailable);
    assert_eq!(f.book.fail_unavailable(&f.conditions), 1);
    let terminal = unavailable_terminal(f.terminal(unresolved, ALICE));
    assert_eq!(terminal.condition(), resolver);
    assert_eq!(terminal.request(), unresolved);
    assert_eq!(terminal.scope(), Scope::Slot(unresolved_slot));
    // The resolved handshake continues, and its status shows nothing.
    assert_eq!(f.terminal(resolved, BOB), None);
    assert_eq!(f.book.blocked_by(resolved, BOB, &f.conditions), None);
    // A new, unresolved login is still refused at admission.
    let again = f.open(OperatorCommand::Login, &[(BOB, Scope::Slot(resolved_slot))]);
    assert!(matches!(
        f.terminal(again, BOB),
        Some(Settlement::Failed(CommandFailure::Refused {
            capability: ProcessCapability::NameResolution,
            ..
        }))
    ));
    // The resolved login still needs the transport.
    let reactor = f.latch(ProcessServiceCode::ReactorUnavailable);
    assert_eq!(
        f.book.blocked_by(resolved, BOB, &f.conditions),
        Some(reactor)
    );
    assert_eq!(f.book.fail_unavailable(&f.conditions), 1);
    assert_eq!(
        unavailable_terminal(f.terminal(resolved, BOB)).condition(),
        reactor
    );
    // Each member got exactly one terminal.
    assert_eq!(
        unavailable_terminal(f.terminal(unresolved, ALICE)).condition(),
        resolver
    );
}

#[test]
fn all_three_codes_fail_accepted_work_once_with_its_identity_and_spare_the_rest() {
    let cases = [
        (
            ProcessServiceCode::ReactorUnavailable,
            OperatorCommand::Teleport,
        ),
        (
            ProcessServiceCode::ResolverUnavailable,
            OperatorCommand::Login,
        ),
        (
            ProcessServiceCode::WorkAdmissionUnavailable,
            OperatorCommand::StartScript,
        ),
    ];
    for (code, affected) in cases {
        let mut f = Fixture::new();
        let slot = f.slot();
        let mut mailbox = IntentMailbox::new(false, WorldChoice::Automatic);
        let work = f.dispatch(affected, ALICE, slot, &mut mailbox);
        let save = f.open(OperatorCommand::SaveProfile, &[(BOB, Scope::Process)]);
        let select = f.open(OperatorCommand::Select, &[(BOB, Scope::Slot(slot))]);
        let stop = f.dispatch(OperatorCommand::StopScript, BOB, slot, &mut mailbox);

        let condition = f.latch(code);
        assert_eq!(f.book.fail_unavailable(&f.conditions), 1, "{code:?}");
        let terminal = unavailable_terminal(f.terminal(work, ALICE));
        assert_eq!(terminal.condition(), condition);
        assert_eq!(terminal.request(), work);
        assert_eq!(terminal.scope(), Scope::Slot(slot));
        assert_eq!(terminal.operator_text(), code.operator_text());
        // Unaffected local work and the retirement command continue.
        for (request, who) in [(save, BOB), (select, BOB), (stop, BOB)] {
            assert_eq!(
                f.terminal(request, who),
                None,
                "{code:?} touched {request:?}"
            );
        }
        // Late publication cannot replace the terminal; a second pass
        // settles nothing.
        assert_eq!(
            f.book.settle(work, ALICE, DONE, None),
            Err(CommandError::AlreadySettled {
                request: work,
                subject: ALICE
            })
        );
        assert_eq!(f.book.fail_unavailable(&f.conditions), 0);
        assert_eq!(
            unavailable_terminal(f.terminal(work, ALICE)).condition(),
            condition
        );
    }
}

#[test]
fn logout_stop_and_remove_wait_for_retirement_through_an_outage() {
    let mut f = Fixture::new();
    let slot = f.slot();
    let mut mailbox = online(true);
    let reactor = f.latch(ProcessServiceCode::ReactorUnavailable);

    let logout = f.dispatch(OperatorCommand::Logout, ALICE, slot, &mut mailbox);
    let stop = f.dispatch(OperatorCommand::StopScript, ALICE, slot, &mut mailbox);
    let remove = f.dispatch(OperatorCommand::Remove, BOB, slot, &mut mailbox);

    // Admitted, not refused, and not turned into a failure or a retry, by
    // the ledger or by the slot owner's own Unavailable event.
    assert_eq!(f.book.fail_unavailable(&f.conditions), 0);
    assert_eq!(
        f.book.observe(&unavailable(
            slot,
            ProcessCapability::NetworkEffect,
            reactor
        )),
        0
    );
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
fn the_slot_owners_unavailable_event_settles_its_members_once() {
    let mut f = Fixture::new();
    let (slot, other_slot) = (f.slot(), f.slot());
    let mut mailbox = online(false);
    let mut other_box = IntentMailbox::new(false, WorldChoice::Automatic);
    let reactor = f.latch(ProcessServiceCode::ReactorUnavailable);
    // These commands were accepted before the reactor latched: admit them
    // under an empty ledger, then restore the latched one.
    let ledger = std::mem::replace(&mut f.conditions, ProcessConditions::new(f.incarnation));
    let relog = f.dispatch(OperatorCommand::RelogNow, ALICE, slot, &mut mailbox);
    let logout = f.dispatch(OperatorCommand::Logout, BOB, slot, &mut mailbox);
    let start = f.dispatch(OperatorCommand::StartScript, ALICE, slot, &mut mailbox);
    let elsewhere = f.dispatch(OperatorCommand::Login, CAROL, other_slot, &mut other_box);
    f.conditions = ledger;

    // The slot cannot start the relog's new connection.
    let event = unavailable(slot, ProcessCapability::NewConnection, reactor);
    assert_eq!(f.book.observe(&event), 1);
    let terminal = unavailable_terminal(f.terminal(relog, ALICE));
    assert_eq!(terminal.condition(), reactor);
    assert_eq!(terminal.request(), relog);
    assert_eq!(terminal.scope(), Scope::Slot(slot));
    // The Log out keeps its precedence, the Start does not need a new
    // connection, and another slot's login is untouched.
    assert_eq!(f.terminal(logout, BOB), None);
    assert_eq!(f.terminal(start, ALICE), None);
    assert_eq!(f.terminal(elsewhere, CAROL), None);
    // No double settlement: the same event, the ledger fan-out or a late
    // acknowledgement leave the relog's terminal as it is.
    assert_eq!(f.book.observe(&event), 0);
    assert_eq!(
        f.book.observe(&applied(slot, &mailbox)),
        1,
        "only the Log out"
    );
    assert_eq!(f.terminal(logout, BOB), Some(&DONE));
    assert_eq!(
        f.book.fail_unavailable(&f.conditions),
        1,
        "only the other slot's login"
    );
    assert_eq!(
        unavailable_terminal(f.terminal(elsewhere, CAROL)).condition(),
        reactor
    );
    assert_eq!(
        unavailable_terminal(f.terminal(relog, ALICE)).condition(),
        reactor
    );
    assert_eq!(
        unavailable_terminal(f.terminal(relog, ALICE)).request(),
        relog
    );
}

#[test]
fn a_session_lost_to_an_outage_settles_no_command() {
    let mut f = Fixture::new();
    let slot = f.slot();
    let mut mailbox = online(false);
    let logout = f.dispatch(OperatorCommand::Logout, ALICE, slot, &mut mailbox);
    let reactor = ConditionId::new(f.incarnation, ProcessServiceCode::ReactorUnavailable);
    let session = SessionKey::first(slot);
    let ended = Event::new(
        Scope::Session(session),
        None,
        SessionEventOut::SessionEnded {
            cause: SessionEndCause::ServiceUnavailable { condition: reactor },
            resumable: false,
        },
    );
    assert_eq!(f.book.observe(&ended), 0);
    assert!(f.member(logout, ALICE).pending().is_some());
    // Its acknowledgement still completes it.
    assert_eq!(f.book.observe(&applied(slot, &mailbox)), 1);
}

// --- Binding and acknowledgement ---

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
    assert_eq!(f.terminal(logout, ALICE), Some(&SUPERSEDED));
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
            subject: ALICE
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
    first.post(Intent::AutoLogin(true)).unwrap();

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
        for is_online in [false, true] {
            let mut mailbox = if is_online {
                online(true)
            } else {
                IntentMailbox::new(true, WorldChoice::Automatic)
            };
            let before = mailbox.snapshot().generation();
            let receipt = mailbox.post(intent).unwrap();
            assert!(
                receipt.generation() > before,
                "{command:?} (online {is_online}) did not advance the generation"
            );
        }
    }
}

/// The r1 review's first baseline probe (H2): a member admitted on its
/// current slot must never be rebound to another slot's lifetime, so that
/// slot's acknowledgement cannot settle it.
#[test]
fn review_reposting_cannot_rebind_current_member_to_stale_slot() {
    let mut f = Fixture::new();
    let (current, stale) = (f.slot(), f.slot());
    let mut mailbox = IntentMailbox::new(false, WorldChoice::Automatic);
    let mut old = IntentMailbox::new(false, WorldChoice::Automatic);
    let login = f.dispatch(OperatorCommand::Login, ALICE, current, &mut mailbox);
    let stale_receipt = old.post(Intent::Login).unwrap();
    assert_eq!(
        f.book.posted(login, ALICE, stale, stale_receipt),
        Err(CommandError::AlreadyPosted {
            request: login,
            subject: ALICE
        })
    );
    assert_eq!(f.member(login, ALICE).scope(), Scope::Slot(current));
    assert_eq!(f.book.observe(&applied(stale, &old)), 0);
    assert_eq!(f.terminal(login, ALICE), None);
}

#[test]
fn a_post_binds_its_member_once_to_the_admitted_slot() {
    let mut f = Fixture::new();
    let (current, stale) = (f.slot(), f.slot());
    let mut mailbox = IntentMailbox::new(false, WorldChoice::Automatic);
    let mut old = IntentMailbox::new(false, WorldChoice::Automatic);
    let earlier = f.dispatch(OperatorCommand::Logout, ALICE, current, &mut mailbox);

    // A first post naming another slot is refused and changes nothing: the
    // member stays Accepted and nothing is superseded.
    let login = f.open(OperatorCommand::Login, &[(ALICE, Scope::Slot(current))]);
    let wrong = old.post(Intent::Login).unwrap();
    assert_eq!(
        f.book.posted(login, ALICE, stale, wrong),
        Err(CommandError::WrongSlot {
            request: login,
            subject: ALICE,
            slot: stale
        })
    );
    assert_eq!(f.member(login, ALICE).pending(), Some(Pending::Accepted));
    assert_eq!(
        f.terminal(earlier, ALICE),
        None,
        "a refused post superseded"
    );
    assert_eq!(f.book.observe(&applied(stale, &old)), 0);

    // The valid post binds it and only then supersedes.
    let receipt = mailbox.post(Intent::Login).unwrap();
    f.book.posted(login, ALICE, current, receipt).unwrap();
    assert_eq!(f.terminal(earlier, ALICE), Some(&SUPERSEDED));
    let bound = Pending::Posted {
        slot: current,
        generation: receipt.generation(),
    };
    // A second post, on the same slot with a newer or the same receipt, or
    // a refused-post report, is refused; the binding never moves.
    let newer = mailbox.post(Intent::AutoLogin(true)).unwrap();
    for attempt in [
        f.book.posted(login, ALICE, current, newer),
        f.book.posted(login, ALICE, current, receipt),
        f.book.posted(login, ALICE, stale, wrong),
        f.book
            .post_refused(login, ALICE, current, MailboxError::Closed),
    ] {
        assert_eq!(
            attempt,
            Err(CommandError::AlreadyPosted {
                request: login,
                subject: ALICE
            })
        );
        assert_eq!(f.member(login, ALICE).pending(), Some(bound));
    }
    // A member admitted without a slot cannot be posted at all.
    let unbound = f.open(OperatorCommand::Login, &[(BOB, Scope::Process)]);
    assert_eq!(
        f.book.posted(unbound, BOB, current, receipt),
        Err(CommandError::WrongSlot {
            request: unbound,
            subject: BOB,
            slot: current
        })
    );
    // A non-mailbox row records no post.
    let save = f.open(
        OperatorCommand::SaveProfile,
        &[(ALICE, Scope::Slot(current))],
    );
    assert_eq!(
        f.book.posted(save, ALICE, current, receipt),
        Err(CommandError::NoIntentRow(save))
    );
}

#[test]
fn an_owner_report_cannot_forge_a_condition_terminal() {
    let mut f = Fixture::new();
    let slot = f.slot();
    let start = f.open(OperatorCommand::StartScript, &[(ALICE, Scope::Slot(slot))]);
    let condition = ConditionId::new(f.incarnation, ProcessServiceCode::WorkAdmissionUnavailable);
    for forged in [
        CommandFailure::Refused {
            condition,
            capability: ProcessCapability::BackgroundWork,
        },
        CommandFailure::Unavailable(UnavailableTerminal::new(condition, start, Scope::Process)),
    ] {
        assert_eq!(
            f.book
                .settle(start, ALICE, Settlement::Failed(forged), None),
            Err(CommandError::ConditionTerminal {
                request: start,
                subject: ALICE
            })
        );
    }
    assert_eq!(f.member(start, ALICE).pending(), Some(Pending::Accepted));
}

// --- Supersession and r2 mailbox obligations ---

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

    let login_all = f.open(
        OperatorCommand::Login,
        &[(ALICE, Scope::Slot(one)), (BOB, Scope::Slot(two))],
    );
    for (who, slot, mailbox) in [(ALICE, one, &mut first), (BOB, two, &mut second)] {
        let receipt = mailbox.post(Intent::Login).unwrap();
        f.book.posted(login_all, who, slot, receipt).unwrap();
    }
    f.dispatch(OperatorCommand::Logout, BOB, two, &mut second);

    assert_eq!(f.terminal(login_all, ALICE), None);
    assert_eq!(f.terminal(login_all, BOB), Some(&SUPERSEDED));
    assert!(!f.book.report(login_all).unwrap().is_settled());
}

#[test]
fn a_stronger_row_supersedes_a_pending_turnover_and_its_old_acknowledgement_is_stale() {
    let mut f = Fixture::new();
    let slot = f.slot();
    let mut mailbox = online(true);
    let relog = f.dispatch(OperatorCommand::RelogNow, ALICE, slot, &mut mailbox);
    assert!(mailbox.snapshot().turnover_pending());
    let ack_of_relog = applied(slot, &mailbox);
    let logout = f.dispatch(OperatorCommand::Logout, ALICE, slot, &mut mailbox);
    // The mailbox dropped the turnover's login half, and the book cancelled
    // the relog member it would have completed.
    assert!(!mailbox.snapshot().turnover_pending());
    assert_eq!(f.terminal(relog, ALICE), Some(&SUPERSEDED));
    // The relog generation's late acknowledgement completes neither.
    assert_eq!(f.book.observe(&ack_of_relog), 0);
    assert_eq!(f.terminal(logout, ALICE), None);
    assert_eq!(f.book.observe(&applied(slot, &mailbox)), 1);
    assert_eq!(f.terminal(logout, ALICE), Some(&DONE));

    // Offline, Relog now is a plain login: no turnover half, and its own
    // acknowledgement completes it.
    let offline_slot = f.slot();
    let mut offline = IntentMailbox::new(false, WorldChoice::Automatic);
    let plain = f.dispatch(OperatorCommand::RelogNow, BOB, offline_slot, &mut offline);
    assert!(!offline.snapshot().turnover_pending());
    assert_eq!(f.book.observe(&applied(offline_slot, &offline)), 1);
    assert_eq!(f.terminal(plain, BOB), Some(&DONE));

    // Remove supersedes a pending turnover too, and closes the lifetime.
    let removed_slot = f.slot();
    let mut removed = online(false);
    let turnover = f.dispatch(OperatorCommand::RelogNow, ALICE, removed_slot, &mut removed);
    let remove = f.dispatch(OperatorCommand::Remove, ALICE, removed_slot, &mut removed);
    assert_eq!(f.terminal(turnover, ALICE), Some(&SUPERSEDED));
    assert!(!removed.snapshot().turnover_pending());
    assert!(removed.snapshot().closed());
    assert_eq!(f.terminal(remove, ALICE), None);
}

#[test]
fn a_closed_lifetime_refuses_later_rows_and_leaves_book_and_mailbox_intact() {
    let mut f = Fixture::new();
    let slot = f.slot();
    let mut mailbox = online(false);
    let remove = f.dispatch(OperatorCommand::Remove, ALICE, slot, &mut mailbox);
    let closed = mailbox.snapshot();
    assert!(closed.closed());

    // A login after Remove: the mailbox refuses the row and its scope is
    // gone. A Log out and a Stop are refused too but keep waiting for the
    // slot's real retirement.
    let login = f.dispatch(OperatorCommand::Login, BOB, slot, &mut mailbox);
    let logout = f.dispatch(OperatorCommand::Logout, BOB, slot, &mut mailbox);
    let stop = f.dispatch(OperatorCommand::StopScript, ALICE, slot, &mut mailbox);
    assert_eq!(
        f.terminal(login, BOB),
        Some(&Settlement::Cancelled(Cancellation::ScopeInvalidated))
    );
    assert_eq!(f.member(logout, BOB).pending(), Some(Pending::Accepted));
    assert_eq!(f.member(stop, ALICE).pending(), Some(Pending::Accepted));
    // The refused rows changed nothing in the mailbox, and a refused post
    // superseded nothing.
    assert_eq!(mailbox.snapshot(), closed);
    assert_eq!(f.terminal(remove, ALICE), None);
    // A repeated Remove is accepted by the closed lifetime and changes
    // nothing; its generation is the closing one, already acknowledged.
    let again = f.dispatch(OperatorCommand::Remove, BOB, slot, &mut mailbox);
    assert_eq!(mailbox.snapshot(), closed);
    assert_eq!(
        f.book.observe(&applied(slot, &mailbox)),
        0,
        "Remove settles on retirement"
    );
    // Retirement completes every withdrawal, once.
    assert_eq!(f.book.slot_retired(slot), 4);
    for (request, who) in [(remove, ALICE), (again, BOB), (logout, BOB), (stop, ALICE)] {
        assert_eq!(f.terminal(request, who), Some(&DONE));
    }
    assert_eq!(f.book.slot_retired(slot), 0);
}

#[test]
fn an_exhausted_mailbox_fails_its_member_and_supersedes_nothing() {
    let mut f = Fixture::new();
    let slot = f.slot();
    let mut mailbox = IntentMailbox::new(false, WorldChoice::Automatic);
    let login = f.dispatch(OperatorCommand::Login, ALICE, slot, &mut mailbox);
    let logout = f.open(OperatorCommand::Logout, &[(ALICE, Scope::Slot(slot))]);
    f.book
        .post_refused(
            logout,
            ALICE,
            slot,
            MailboxError::Exhausted(MailboxGeneration::Cancellation),
        )
        .unwrap();
    assert_eq!(
        f.terminal(logout, ALICE),
        Some(&Settlement::Failed(CommandFailure::Failed(
            FailureKind::IntentExhausted
        )))
    );
    assert_eq!(f.terminal(login, ALICE), None, "a refused post superseded");
    assert_eq!(f.book.observe(&applied(slot, &mailbox)), 1);
    assert_eq!(f.terminal(login, ALICE), Some(&DONE));
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
    mailbox
        .observe(LifecycleFact::WorldPreferenceFailed)
        .unwrap();
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
    mailbox
        .observe(LifecycleFact::RepeatGuardTripped {
            count: 3,
            window_seconds: 60,
        })
        .unwrap();
    assert_eq!(f.book.observe_intent(slot, &mailbox.snapshot()), 1);
    assert_eq!(f.terminal(again, ALICE), Some(&SUPERSEDED));

    // A Log in posted after the latch rearms: the snapshot no longer
    // withdraws it.
    let third = f.dispatch(OperatorCommand::Login, ALICE, slot, &mut mailbox);
    assert_eq!(f.book.observe_intent(slot, &mailbox.snapshot()), 0);
    assert_eq!(f.terminal(third, ALICE), None);
}

#[test]
fn a_withdrawn_snapshot_settles_only_its_own_slots_members() {
    let mut f = Fixture::new();
    let (latched, healthy) = (f.slot(), f.slot());
    let mut latched_box = IntentMailbox::new(false, WorldChoice::Automatic);
    let mut healthy_box = IntentMailbox::new(false, WorldChoice::Automatic);
    let kept = f.dispatch(OperatorCommand::Login, BOB, healthy, &mut healthy_box);
    let withdrawn = f.dispatch(OperatorCommand::Login, ALICE, latched, &mut latched_box);
    latched_box
        .observe(LifecycleFact::RepeatGuardTripped {
            count: 3,
            window_seconds: 60,
        })
        .unwrap();
    // The other slot's member was posted at an older generation than this
    // snapshot's; only the slot fence keeps it out.
    assert!(healthy_box.snapshot().generation() < latched_box.snapshot().generation());
    assert_eq!(f.book.observe_intent(latched, &latched_box.snapshot()), 1);
    assert_eq!(f.terminal(withdrawn, ALICE), Some(&SUPERSEDED));
    assert_eq!(f.terminal(kept, BOB), None);
}

// --- Retirement and terminal precedence ---

#[test]
fn a_retired_slot_completes_withdrawals_and_invalidates_the_rest() {
    let mut f = Fixture::new();
    let (gone, kept) = (f.slot(), f.slot());
    let mut mailbox = online(false);
    let mut other = IntentMailbox::new(false, WorldChoice::Automatic);

    // Remove closes the slot's mailbox lifetime, so it is posted last.
    let pause = f.dispatch(OperatorCommand::PauseScript, ALICE, gone, &mut mailbox);
    let remove = f.dispatch(OperatorCommand::Remove, ALICE, gone, &mut mailbox);
    let login = f.dispatch(OperatorCommand::Login, BOB, kept, &mut other);
    let start = f.open(OperatorCommand::StartScript, &[(ALICE, Scope::Slot(gone))]);

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
fn retirement_never_replaces_an_earlier_terminal() {
    let mut f = Fixture::new();
    let (slot, other) = (f.slot(), f.slot());
    let mut mailbox = IntentMailbox::new(false, WorldChoice::Automatic);
    // Five commands, each with a member on `slot` already settled by a
    // different path and a member on another slot still pending, so every
    // report stays open while `slot` retires.
    let both = |who| [(who, Scope::Slot(slot)), (CAROL, Scope::Slot(other))];
    let post = |f: &mut Fixture, request, who, mailbox: &mut IntentMailbox, intent| {
        let receipt = mailbox.post(intent).unwrap();
        f.book.posted(request, who, slot, receipt).unwrap();
    };
    // Acknowledgement.
    let login = f.open(OperatorCommand::Login, &both(ALICE));
    post(&mut f, login, ALICE, &mut mailbox, Intent::Login);
    assert_eq!(f.book.observe(&applied(slot, &mailbox)), 1);
    // Owner rejection.
    let start = f.open(OperatorCommand::StartScript, &both(ALICE));
    post(&mut f, start, ALICE, &mut mailbox, Intent::StartScript);
    let rejected = Settlement::Failed(CommandFailure::Rejected(Rejection::StartRefused));
    f.book.settle(start, ALICE, rejected.clone(), None).unwrap();
    // Dispatch result.
    let select = f.open(OperatorCommand::Select, &both(BOB));
    f.book.settle(select, BOB, DONE, None).unwrap();
    // The slot owner's Unavailable event.
    let walk = f.open(OperatorCommand::GroupWalk, &both(BOB));
    let work = ConditionId::new(f.incarnation, ProcessServiceCode::WorkAdmissionUnavailable);
    let event = unavailable(slot, ProcessCapability::BackgroundWork, work);
    assert_eq!(f.book.observe(&event), 1);
    // Operator cancellation.
    let pause = f.open(OperatorCommand::PauseScript, &both(BOB));
    post(&mut f, pause, BOB, &mut mailbox, Intent::PauseScript);
    let cancelled = Settlement::Cancelled(Cancellation::Operator);
    f.book.settle(pause, BOB, cancelled.clone(), None).unwrap();

    // Retirement (twice) touches none of them.
    assert_eq!(f.book.slot_retired(slot), 0);
    assert_eq!(f.book.slot_retired(slot), 0);
    assert_eq!(f.terminal(login, ALICE), Some(&DONE));
    assert_eq!(f.terminal(start, ALICE), Some(&rejected));
    assert_eq!(f.terminal(select, BOB), Some(&DONE));
    assert_eq!(f.terminal(pause, BOB), Some(&cancelled));
    assert_eq!(
        unavailable_terminal(f.terminal(walk, BOB)).condition(),
        work
    );
    // The other slot's members are still pending; its own retirement
    // settles exactly them.
    for request in [login, start, select, walk, pause] {
        assert_eq!(f.terminal(request, CAROL), None);
    }
    assert_eq!(f.book.slot_retired(other), 5);
}

#[test]
fn an_outage_terminal_survives_retirement_and_late_acknowledgement() {
    let mut f = Fixture::new();
    let slot = f.slot();
    let mut mailbox = IntentMailbox::new(false, WorldChoice::Automatic);
    let login = f.dispatch(OperatorCommand::Login, ALICE, slot, &mut mailbox);
    let reactor = f.latch(ProcessServiceCode::ReactorUnavailable);
    assert_eq!(f.book.fail_unavailable(&f.conditions), 1);
    assert_eq!(f.book.observe(&applied(slot, &mailbox)), 0);
    assert_eq!(f.book.observe_intent(slot, &mailbox.snapshot()), 0);
    assert_eq!(f.book.slot_retired(slot), 0);
    assert_eq!(f.book.slot_retired(slot), 0);
    assert_eq!(
        unavailable_terminal(f.terminal(login, ALICE)).condition(),
        reactor
    );
}

// --- Identity and bounded history ---

#[test]
fn command_identity_is_a_unique_host_request() {
    let mut f = Fixture::new();
    let slot = f.slot();
    let target = [(ALICE, Scope::Slot(slot))];
    let isolate = RequestId::isolate(7).unwrap();
    assert_eq!(
        f.book
            .open(isolate, OperatorCommand::Login, &target, &f.conditions),
        Err(CommandError::IsolateRequest(isolate))
    );
    let request = f.open(OperatorCommand::Login, &target);
    assert_eq!(
        f.book
            .open(request, OperatorCommand::Logout, &target, &f.conditions),
        Err(CommandError::DuplicateRequest(request))
    );
    let fresh = f.requests.mint().unwrap();
    assert_eq!(
        f.book.open(
            fresh,
            OperatorCommand::Login,
            &[(ALICE, Scope::Slot(slot)), (ALICE, Scope::Slot(slot))],
            &f.conditions
        ),
        Err(CommandError::DuplicateMember {
            request: fresh,
            subject: ALICE
        })
    );
    assert!(f.book.report(fresh).is_none());
    let mut mailbox = IntentMailbox::new(false, WorldChoice::Automatic);
    let receipt = mailbox.post(Intent::MemoryMode(MemoryMode::Low)).unwrap();
    assert_eq!(
        f.book.posted(request, BOB, slot, receipt),
        Err(CommandError::UnknownMember {
            request,
            subject: BOB
        })
    );
    assert_eq!(
        f.book.posted(fresh, ALICE, slot, receipt),
        Err(CommandError::UnknownRequest(fresh))
    );
}

#[test]
fn settled_history_is_bounded_and_pending_reports_are_never_evicted() {
    let mut f = Fixture::new();
    let slot = f.slot();
    let pending = f.open(OperatorCommand::Login, &[(ALICE, Scope::Slot(slot))]);
    let mut settled = Vec::new();
    for _ in 0..SETTLED_REPORT_CAP + 5 {
        let request = f.open(OperatorCommand::Select, &[(ALICE, Scope::Slot(slot))]);
        f.book.settle(request, ALICE, DONE, None).unwrap();
        settled.push(request);
    }
    assert!(f.book.report(pending).is_some());
    assert_eq!(f.settled_count(), SETTLED_REPORT_CAP);
    assert!(f.book.report(settled[0]).is_none());
    assert!(f.book.report(*settled.last().unwrap()).is_some());
}

/// The r1 review's second baseline probe (M1): many pending reports that
/// finish later must not grow the settled history past its cap.
#[test]
fn review_settled_history_is_bounded_when_many_pending_members_finish() {
    let mut f = Fixture::new();
    let slot = f.slot();
    let requests: Vec<_> = (0..SETTLED_REPORT_CAP + 5)
        .map(|_| f.open(OperatorCommand::Select, &[(ALICE, Scope::Slot(slot))]))
        .collect();
    // Pending work is never evicted, however much of it there is.
    assert_eq!(f.book.reports().count(), SETTLED_REPORT_CAP + 5);
    for request in &requests {
        f.book.settle(*request, ALICE, DONE, None).unwrap();
        assert!(f.settled_count() <= SETTLED_REPORT_CAP);
    }
    assert_eq!(f.settled_count(), SETTLED_REPORT_CAP);
    // The newest terminals are the ones kept.
    for (index, request) in requests.iter().enumerate() {
        assert_eq!(
            f.book.report(*request).is_some(),
            index >= 5,
            "report {index}"
        );
    }
}

#[test]
fn the_history_keeps_the_most_recently_settled_reports_on_every_terminal_path() {
    let mut f = Fixture::new();
    let slot = f.slot();
    let mut mailbox = IntentMailbox::new(false, WorldChoice::Automatic);
    // An old report that settles last outlives older terminals.
    let old_login = f.dispatch(OperatorCommand::Login, ALICE, slot, &mut mailbox);
    let early: Vec<_> = (0..SETTLED_REPORT_CAP)
        .map(|_| {
            let request = f.open(OperatorCommand::Select, &[(BOB, Scope::Slot(slot))]);
            f.book.settle(request, BOB, DONE, None).unwrap();
            request
        })
        .collect();
    assert_eq!(f.book.observe(&applied(slot, &mailbox)), 1);
    assert_eq!(f.settled_count(), SETTLED_REPORT_CAP);
    assert_eq!(f.terminal(old_login, ALICE), Some(&DONE));
    assert!(f.book.report(early[0]).is_none());
    assert!(f.book.report(early[1]).is_some());

    // Refusals at open, outage fan-out and retirement settle in bulk; the
    // cap holds after each.
    f.latch(ProcessServiceCode::ReactorUnavailable);
    for _ in 0..SETTLED_REPORT_CAP {
        f.open(OperatorCommand::Teleport, &[(ALICE, Scope::Slot(slot))]);
    }
    assert_eq!(f.settled_count(), SETTLED_REPORT_CAP);
    let ledger = std::mem::replace(&mut f.conditions, ProcessConditions::new(f.incarnation));
    let walks: Vec<_> = (0..SETTLED_REPORT_CAP + 3)
        .map(|_| f.open(OperatorCommand::WalkTo, &[(ALICE, Scope::Slot(slot))]))
        .collect();
    let starts: Vec<_> = (0..SETTLED_REPORT_CAP + 3)
        .map(|_| f.open(OperatorCommand::StartScript, &[(BOB, Scope::Slot(slot))]))
        .collect();
    f.conditions = ledger;
    assert_eq!(
        f.book.fail_unavailable(&f.conditions),
        SETTLED_REPORT_CAP + 3
    );
    assert_eq!(f.settled_count(), SETTLED_REPORT_CAP);
    assert!(f.book.report(*walks.last().unwrap()).is_some());
    assert!(
        starts.iter().all(|r| f.book.report(*r).is_some()),
        "pending evicted"
    );
    assert_eq!(f.book.slot_retired(slot), SETTLED_REPORT_CAP + 3);
    assert_eq!(f.settled_count(), SETTLED_REPORT_CAP);
    assert!(f.book.report(*starts.last().unwrap()).is_some());
    assert!(f.book.report(*walks.last().unwrap()).is_none());
}
