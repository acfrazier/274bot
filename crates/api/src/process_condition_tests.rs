use super::*;
use crate::identity::{
    Cancellation, ExecutionId, ProcessIncarnation, ProcessOrigin, RequestIdMinter, SessionKey,
    Settlement, SlotKey, Terminal, TerminalError,
};
use crate::lifecycle::{
    Acknowledgement, HoldReason, Intent, IntentMailbox, LifecycleFact, LoginIntent, LoginLatch,
    MailboxError, RunEffect, SessionEndCause, SessionEventOut, SessionGate, WorldChoice,
};
use crate::selected::RunKey;
use std::num::NonZeroU64;
use std::time::{Duration, Instant};

fn incarnation(nonce: u64) -> ProcessIncarnation {
    ProcessIncarnation::from_start_nonce(NonZeroU64::new(nonce).unwrap())
}

fn condition(
    incarnation: ProcessIncarnation,
    code: ProcessServiceCode,
    cause: ConditionCause,
) -> ProcessServiceCondition {
    let start = Instant::now();
    ProcessServiceCondition::new(
        ConditionId::new(incarnation, code),
        cause,
        ProcessOrigin::at(start).elapsed(start + Duration::from_millis(1500)),
        None,
    )
}

/// One capability each code affects, and one every code leaves alone.
fn affected_capability(code: ProcessServiceCode) -> ProcessCapability {
    match code {
        ProcessServiceCode::ReactorUnavailable => ProcessCapability::Login,
        ProcessServiceCode::ResolverUnavailable => ProcessCapability::NameResolution,
        ProcessServiceCode::WorkAdmissionUnavailable => ProcessCapability::BackgroundWork,
    }
}

#[test]
fn every_code_carries_its_exact_restart_text_and_classification() {
    let texts: Vec<&str> = ProcessServiceCode::ALL
        .iter()
        .map(|code| code.operator_text())
        .collect();
    assert_eq!(
        texts,
        [
            "Game transport is unavailable. Restart the application process. Retrying login or restarting a script will not restore transport.",
            "Name resolution is unavailable. Restart the application process. Retrying the request or restarting a script will not restore name resolution.",
            "Background work is unavailable. Restart the application process. Retrying the job or restarting a script will not restore background work.",
        ]
    );
    for code in ProcessServiceCode::ALL {
        assert!(code.restart_required());
        assert_eq!(code.recoverability(), Recoverability::RestartRequired);
        assert_ne!(code.recoverability(), Recoverability::TransientCapacity);
        assert_eq!(code.to_string(), code.code());
    }
    assert_eq!(
        ProcessServiceCode::ALL.map(ProcessServiceCode::code),
        [
            "ReactorUnavailable",
            "ResolverUnavailable",
            "WorkAdmissionUnavailable"
        ]
    );
}

#[test]
fn affected_and_unaffected_capabilities_follow_the_code() {
    use ProcessCapability as Cap;
    use ProcessServiceCode as Code;
    let affected = |code: Code| -> Vec<Cap> {
        Cap::ALL
            .into_iter()
            .filter(|capability| code.affects(*capability))
            .collect()
    };
    assert_eq!(
        affected(Code::ReactorUnavailable),
        [
            Cap::NewConnection,
            Cap::Login,
            Cap::Reconnect,
            Cap::NetworkEffect
        ]
    );
    assert_eq!(affected(Code::ResolverUnavailable), [Cap::NameResolution]);
    assert_eq!(
        affected(Code::WorkAdmissionUnavailable),
        [Cap::BackgroundWork]
    );
    for code in Code::ALL {
        assert!(
            !code.affects(Cap::LocalOnly) && !code.affects(Cap::ResolvedAddress),
            "{code}: local work and already-resolved addresses never need the failed service"
        );
    }
}

#[test]
fn conditions_latch_once_per_code_per_incarnation_until_exit() {
    let incarnation = incarnation(1);
    let mut ledger = ProcessConditions::new(incarnation);
    assert_eq!(ledger.snapshot().count(), 0);
    assert!(ledger
        .latched(ProcessServiceCode::ReactorUnavailable)
        .is_none());

    ledger
        .publish(condition(
            incarnation,
            ProcessServiceCode::ReactorUnavailable,
            ConditionCause::Panicked,
        ))
        .unwrap();
    let again = condition(
        incarnation,
        ProcessServiceCode::ReactorUnavailable,
        ConditionCause::ProgressLost,
    );
    assert_eq!(
        ledger.publish(again),
        Err(PublishError::AlreadyLatched(
            ProcessServiceCode::ReactorUnavailable
        ))
    );
    assert_eq!(
        ledger
            .latched(ProcessServiceCode::ReactorUnavailable)
            .map(ProcessServiceCondition::cause),
        Some(ConditionCause::Panicked),
        "the first publication stands"
    );

    for code in [
        ProcessServiceCode::ResolverUnavailable,
        ProcessServiceCode::WorkAdmissionUnavailable,
    ] {
        ledger
            .publish(condition(incarnation, code, ConditionCause::StartFailed))
            .unwrap();
    }
    assert_eq!(ledger.snapshot().count(), MAX_CONDITIONS);
    assert_eq!(MAX_CONDITIONS, 3, "T3 borrows at most three entries");
    let codes: Vec<ProcessServiceCode> = ledger
        .snapshot()
        .map(ProcessServiceCondition::code)
        .collect();
    assert_eq!(codes, ProcessServiceCode::ALL);
    for condition in ledger.snapshot() {
        assert!(condition.restart_required());
        assert_eq!(condition.id().incarnation(), incarnation);
    }
}

#[test]
fn every_code_is_sticky_and_a_condition_from_another_incarnation_is_never_current_health() {
    for code in ProcessServiceCode::ALL {
        let current = incarnation(2);
        let previous = incarnation(1);
        let mut ledger = ProcessConditions::new(current);
        let stale = condition(previous, code, ConditionCause::ProgressLost);
        assert_eq!(
            ledger.publish(stale),
            Err(PublishError::StaleIncarnation),
            "{code}"
        );
        assert!(ledger.retry_allowed(affected_capability(code)), "{code}");
        assert!(ledger.latched(code).is_none(), "{code}");
        assert_eq!(ledger.incarnation(), current);

        ledger
            .publish(condition(current, code, ConditionCause::ProgressLost))
            .unwrap();
        // Nothing a consumer does clears it: a second report, a finished
        // request or a fresh look at the ledger all see the same entry.
        assert_eq!(
            ledger.publish(condition(current, code, ConditionCause::Panicked)),
            Err(PublishError::AlreadyLatched(code))
        );
        assert!(ledger.latched(code).is_some(), "{code}: latched until exit");
        assert!(!ledger.retry_allowed(affected_capability(code)), "{code}");
        assert!(
            ledger
                .admit(affected_capability(code), None, Scope::Process)
                .is_err(),
            "{code}: still refused"
        );
    }
}

#[test]
fn every_code_refuses_only_affected_operations_and_keeps_their_identity() {
    let slot = SlotKey::from_incarnation(4).unwrap();
    let scope = Scope::Slot(slot);
    for code in ProcessServiceCode::ALL {
        let incarnation = incarnation(3);
        let mut ledger = ProcessConditions::new(incarnation);
        let request = RequestIdMinter::new().mint().unwrap();
        let condition_id = ConditionId::new(incarnation, code);
        for capability in ProcessCapability::ALL {
            assert_eq!(
                ledger.admit(capability, Some(request), scope),
                Ok(()),
                "{code}: everything is admitted before the outage"
            );
        }
        ledger
            .publish(condition(incarnation, code, ConditionCause::ProgressLost))
            .unwrap();

        for capability in ProcessCapability::ALL {
            let decision = ledger.admit(capability, Some(request), scope);
            if code.affects(capability) {
                let AdmissionRefusal::Unavailable(refusal) =
                    decision.expect_err("affected capability is refused");
                assert_eq!(refusal.condition(), condition_id, "{code} {capability:?}");
                assert_eq!(refusal.request(), Some(request));
                assert_eq!(refusal.scope(), scope);
                assert_eq!(refusal.capability(), capability);
                assert_eq!(refusal.recoverability(), Recoverability::RestartRequired);
                assert_eq!(refusal.operator_text(), code.operator_text());
                assert!(
                    refusal.to_string().starts_with(code.operator_text()),
                    "{code}: the refusal carries the exact text"
                );
                assert!(AdmissionRefusal::Unavailable(refusal)
                    .to_string()
                    .starts_with(code.operator_text()));
                assert!(
                    !ledger.retry_allowed(capability),
                    "{code} {capability:?}: no automatic retry until restart"
                );
            } else {
                assert_eq!(
                    decision,
                    Ok(()),
                    "{code} {capability:?}: an unrelated service continues"
                );
                assert!(ledger.retry_allowed(capability));
            }
        }
        assert_eq!(
            ledger.admit(ProcessCapability::LocalOnly, None, Scope::Process),
            Ok(()),
            "{code}: local work on a slot or the process needs no failed service"
        );
    }
}

#[test]
fn every_code_settles_custody_logically_but_acknowledges_no_retirement() {
    for code in ProcessServiceCode::ALL {
        let incarnation = incarnation(5);
        let request = RequestIdMinter::new().mint().unwrap();
        let scope = Scope::Process;
        let mut custody = StrandedCustody::new(request, scope);
        assert!(custody.charged());
        assert_eq!(custody.terminal(), None);
        assert_eq!(custody.request(), request);
        assert_eq!(custody.scope(), scope);

        let condition = ConditionId::new(incarnation, code);
        let terminal = custody.fail_unavailable(condition).unwrap();
        assert_eq!(terminal.condition(), condition, "{code}");
        assert_eq!(terminal.request(), request);
        assert_eq!(terminal.scope(), scope);
        assert_eq!(terminal.operator_text(), code.operator_text());
        assert_eq!(custody.terminal(), Some(terminal));
        assert!(
            custody.charged(),
            "{code}: the logical failure released nothing"
        );
        assert_eq!(
            custody.fail_unavailable(condition),
            Err(CustodyError::AlreadyFailed),
            "{code}: one logical terminal; a late report cannot rewrite it"
        );

        // No deadline, report or display refresh stands in for the return:
        // the refusal hands the custody back, still charged.
        let still_charged = match custody.release() {
            Err(CustodyError::WorkerStillCharged(custody)) => custody,
            other => panic!("{code}: expected the custody back, got {other:?}"),
        };
        assert!(still_charged.charged());
        assert_eq!(still_charged.terminal(), Some(terminal));
        let mut custody = still_charged;
        let refused = custody.release().unwrap_err();
        assert!(
            refused.to_string().contains("still charged"),
            "{code}: the reason names the retained worker"
        );
        let CustodyError::WorkerStillCharged(returned) = refused else {
            panic!("{code}: a refused release keeps the custody");
        };
        custody = returned;
        custody.worker_returned();
        assert!(!custody.charged());
        assert_eq!(
            custody.terminal(),
            Some(terminal),
            "{code}: a late return cannot replace the terminal"
        );
        custody.release().unwrap();
    }
}

/// §26 §3.2 "Logout/Stop/Remove intent precedes or races the condition":
/// the frozen intent precedence stands, and a preserved login intent is
/// refused at admission rather than retried.
#[test]
fn intent_precedence_survives_an_outage_and_the_retry_is_refused_not_scheduled() {
    let incarnation = incarnation(6);
    let slot = SlotKey::from_incarnation(9).unwrap();
    let scope = Scope::Slot(slot);
    let mut ledger = ProcessConditions::new(incarnation);
    let reactor = ConditionId::new(incarnation, ProcessServiceCode::ReactorUnavailable);

    // Logout precedes the condition: it stays a logout, never a fresh login.
    let mut logout = IntentMailbox::new(true, WorldChoice::Automatic);
    let login = logout.login_command(false).unwrap();
    logout.observe(LifecycleFact::LoginSucceeded).unwrap();
    assert_eq!(logout.acknowledge_login(login), Acknowledgement::Applied);
    logout.post(Intent::Logout).unwrap();
    let press = logout.logout_command(true).unwrap();
    ledger
        .publish(condition(
            incarnation,
            ProcessServiceCode::ReactorUnavailable,
            ConditionCause::Panicked,
        ))
        .unwrap();
    assert_eq!(
        ledger.admit(ProcessCapability::NetworkEffect, None, scope),
        Err(AdmissionRefusal::Unavailable(UnavailableRefusal {
            condition: reactor,
            capability: ProcessCapability::NetworkEffect,
            request: None,
            scope,
        })),
        "the clean logout press needs network authority the session lost"
    );
    assert!(
        logout.snapshot().logout_wanted(),
        "blocked: the Logout is not complete while its session is live"
    );
    // The reactor ends the session (`SessionEndCause::ServiceUnavailable`):
    // the exit is the Logout's completion predicate and settles its press.
    logout
        .observe(LifecycleFact::SessionExited {
            clean: false,
            resumable: false,
        })
        .unwrap();
    assert!(!logout.snapshot().logout_wanted(), "the Logout is complete");
    assert_eq!(
        logout.acknowledge_logout(press),
        Acknowledgement::Stale,
        "a press read before the exit settles nothing afterwards"
    );
    assert_eq!(logout.snapshot().latch(), Some(LoginLatch::OperatorLogout));
    assert!(
        logout.login_command(false).is_none(),
        "an outage never turns Logout into a fresh login"
    );

    // Remove after the condition is latched: the lifetime closes; no retry.
    let mut removed = IntentMailbox::new(true, WorldChoice::Automatic);
    removed.post(Intent::RemoveSlot).unwrap();
    assert!(removed.snapshot().closed());
    assert!(removed.login_command(false).is_none());
    assert_eq!(removed.post(Intent::Login), Err(MailboxError::Closed));
    assert!(
        !ledger.retry_allowed(ProcessCapability::Login),
        "and the ledger would refuse the retry anyway"
    );

    // A preserved login intent under the outage: refused at admission with
    // the condition, the intent kept, nothing scheduled.
    let mut wanted = IntentMailbox::new(true, WorldChoice::Automatic);
    let login = wanted.login_command(false).unwrap();
    wanted.observe(LifecycleFact::LoginSucceeded).unwrap();
    wanted.acknowledge_login(login);
    wanted
        .observe(LifecycleFact::SessionExited {
            clean: false,
            resumable: false,
        })
        .unwrap();
    assert!(wanted.relogs_after_drop(), "the intent is preserved");
    let relog = wanted.login_command(false).unwrap();
    let AdmissionRefusal::Unavailable(refusal) = ledger
        .admit(ProcessCapability::Login, None, scope)
        .unwrap_err();
    assert_eq!(refusal.condition(), reactor);
    let projected = SessionEventOut::Unavailable {
        capability: refusal.capability(),
        condition: refusal.condition(),
    };
    assert_eq!(
        projected,
        SessionEventOut::Unavailable {
            capability: ProcessCapability::Login,
            condition: reactor
        },
        "the slot projects the refusal with its condition"
    );
    assert!(
        wanted.attempt_is_current(relog, 0),
        "the refusal is not a cancellation: the intent still stands"
    );
    assert_eq!(wanted.snapshot().login(), LoginIntent::Auto);
    assert_eq!(wanted.snapshot().latch(), None, "no latch is invented");
    assert!(!ledger.retry_allowed(ProcessCapability::Reconnect));
    assert!(!ledger.retry_allowed(ProcessCapability::Login));

    // The resolver going down leaves an established session alone.
    let resolver = ProcessConditions::new(incarnation);
    assert_eq!(
        resolver.admit(ProcessCapability::NetworkEffect, None, scope),
        Ok(())
    );
    let mut resolver = resolver;
    resolver
        .publish(condition(
            incarnation,
            ProcessServiceCode::ResolverUnavailable,
            ConditionCause::StartFailed,
        ))
        .unwrap();
    assert_eq!(
        resolver.admit(ProcessCapability::NetworkEffect, None, scope),
        Ok(()),
        "an established session needs no new resolution"
    );
    assert_eq!(
        resolver.admit(ProcessCapability::ResolvedAddress, None, scope),
        Ok(())
    );
    assert!(resolver
        .admit(ProcessCapability::NameResolution, None, scope)
        .is_err());
}

/// §26 §3.2 "Reactor becomes unavailable with a live session": the session
/// projects the condition instead of Ready or recovery, keeps run identity
/// and hold semantics, and invents no Start or reconnect.
#[test]
fn a_live_session_losing_its_reactor_projects_the_condition_not_readiness() {
    let incarnation = incarnation(7);
    let slot = SlotKey::from_incarnation(11).unwrap();
    let session = SessionKey::new(slot, 1);
    let reactor = ConditionId::new(incarnation, ProcessServiceCode::ReactorUnavailable);
    let mut gate = SessionGate::new(slot);
    gate.begin(session).unwrap();
    gate.observe(session, &SessionEventOut::SceneReady).unwrap();
    gate.observe(
        session,
        &SessionEventOut::Held {
            why: HoldReason::Guardian,
        },
    )
    .unwrap();
    assert!(gate.held_by(HoldReason::Guardian));

    let end = SessionEventOut::SessionEnded {
        cause: SessionEndCause::ServiceUnavailable { condition: reactor },
        resumable: false,
    };
    gate.observe(session, &end).unwrap();
    assert_eq!(gate.ready(), None, "network authority is invalidated");
    assert_eq!(gate.current(), None);
    assert!(
        gate.observe(session, &SessionEventOut::SceneReady).is_err(),
        "no automatic recovery projects Ready again"
    );
    let SessionEventOut::SessionEnded { cause, resumable } = end else {
        unreachable!()
    };
    assert!(!resumable, "no reconnect is invented");
    assert!(!cause.is_clean());
    assert!(
        !cause.counts_toward_repeat_guard(),
        "the outage is not a repeat-guard strike"
    );

    // The mailbox keeps the intent (run identity and hold semantics are the
    // seat's); the ledger refuses the reconnect the intent would schedule.
    let mut mailbox = IntentMailbox::new(true, WorldChoice::Automatic);
    let login = mailbox.login_command(false).unwrap();
    mailbox.observe(LifecycleFact::LoginSucceeded).unwrap();
    mailbox.acknowledge_login(login);
    mailbox
        .observe(LifecycleFact::SessionExited {
            clean: false,
            resumable: false,
        })
        .unwrap();
    assert!(!mailbox.reconnects(), "a fresh login, never a reconnect");
    assert!(mailbox.snapshot().login_wanted(), "intent preserved");
    let mut ledger = ProcessConditions::new(incarnation);
    ledger
        .publish(condition(
            incarnation,
            ProcessServiceCode::ReactorUnavailable,
            ConditionCause::ProgressLost,
        ))
        .unwrap();
    assert!(ledger
        .admit(ProcessCapability::Login, None, Scope::Session(session))
        .is_err());
    assert!(ledger
        .admit(ProcessCapability::NewConnection, None, Scope::Slot(slot))
        .is_err());
    assert_eq!(
        ledger.admit(ProcessCapability::LocalOnly, None, Scope::Slot(slot)),
        Ok(()),
        "local work on the slot continues"
    );
}

#[test]
fn diagnostics_are_bounded_on_char_boundaries_and_do_not_drive_behaviour() {
    let long = "é".repeat(200);
    let diagnostic = Diagnostic::bounded(&long);
    assert!(diagnostic.as_str().len() <= DIAGNOSTIC_MAX_BYTES);
    assert_eq!(diagnostic.as_str().len(), DIAGNOSTIC_MAX_BYTES);
    assert!(diagnostic.as_str().chars().all(|c| c == 'é'));
    assert_eq!(Diagnostic::bounded("short").as_str(), "short");

    let incarnation = incarnation(8);
    let with = ProcessServiceCondition::new(
        ConditionId::new(incarnation, ProcessServiceCode::ResolverUnavailable),
        ConditionCause::StartFailed,
        ProcessTime::ZERO,
        Some(Diagnostic::bounded("getaddrinfo failed")),
    );
    let without = ProcessServiceCondition::new(
        ConditionId::new(incarnation, ProcessServiceCode::ResolverUnavailable),
        ConditionCause::StartFailed,
        ProcessTime::ZERO,
        None,
    );
    assert_eq!(with.diagnostic(), Some("getaddrinfo failed"));
    assert_eq!(without.diagnostic(), None);
    assert_eq!(with.id(), without.id());
    assert_eq!(with.operator_text(), without.operator_text());
    let record = with.to_string();
    assert!(record.contains("code=ResolverUnavailable"));
    assert!(record.contains("cause=start_failed"));
    assert!(record.contains("detected_at_ms=0"));
    assert!(record.contains("restart_required=true"));
    assert!(record.contains("getaddrinfo failed"));
    assert!(!without.to_string().contains("diagnostic="));
}

/// A slot with a live session and auto-login on.
fn live_mailbox() -> IntentMailbox {
    let mut mailbox = IntentMailbox::new(true, WorldChoice::Automatic);
    let login = mailbox.login_command(false).unwrap();
    mailbox.observe(LifecycleFact::LoginSucceeded).unwrap();
    assert_eq!(mailbox.acknowledge_login(login), Acknowledgement::Applied);
    mailbox
}

/// The capability an accepted in-flight run request needs from each
/// service.
fn in_flight_capability(code: ProcessServiceCode) -> ProcessCapability {
    match code {
        ProcessServiceCode::ReactorUnavailable => ProcessCapability::NetworkEffect,
        ProcessServiceCode::ResolverUnavailable => ProcessCapability::NameResolution,
        ProcessServiceCode::WorkAdmissionUnavailable => ProcessCapability::BackgroundWork,
    }
}

/// §26 §3.2: a retirement-dependent command (Stop, Logout, Remove) cannot
/// report Completed while the stranded worker is charged. Its blocked status
/// reads the latched condition and the retained custody; no deadline,
/// terminal or report stands in for the worker's return, after which the
/// command may complete.
fn completion_waits_for_the_real_return(
    ledger: &ProcessConditions,
    capability: ProcessCapability,
    condition: ConditionId,
    custody: StrandedCustody,
) {
    let mut report = Terminal::<Settlement<(), UnavailableTerminal>>::new();
    let mut custody = match custody.release() {
        Err(CustodyError::WorkerStillCharged(custody)) => custody,
        other => panic!("the charge outlives the logical terminal: {other:?}"),
    };
    assert!(report.is_pending(), "not Completed before its predicate");
    assert_eq!(
        ledger.blocks(capability).map(ProcessServiceCondition::id),
        Some(condition),
        "the blocked status names the condition"
    );
    assert!(custody.charged(), "and the retained retirement");
    custody.worker_returned();
    custody.release().unwrap();
    report.publish(Settlement::Completed(())).unwrap();
    assert!(report.borrow().is_some_and(Settlement::is_completed));
}

/// §26 §3.2 "Logout/Stop/Remove intent precedes or races the condition",
/// Stop with an affected outage: the outage already selected the accepted
/// request's terminal, Stop keeps its own meaning (it ends the run, implies
/// no logout and arms no login), the run's work is not retried, and the
/// stranded charge outlives the consumed terminal.
#[test]
fn stop_under_an_affected_outage_keeps_the_outage_terminal_and_schedules_nothing() {
    for code in ProcessServiceCode::ALL {
        let incarnation = incarnation(8);
        let condition_id = ConditionId::new(incarnation, code);
        let run = RunKey {
            slot: 12,
            run: 1,
            session: 1,
        };
        let scope = Scope::Execution(ExecutionId::first(run));
        let capability = in_flight_capability(code);
        let mut ledger = ProcessConditions::new(incarnation);
        let request = RequestIdMinter::new().mint().unwrap();
        assert_eq!(
            ledger.admit(capability, Some(request), scope),
            Ok(()),
            "{code}: accepted before the outage"
        );
        let mut custody = StrandedCustody::new(request, scope);
        let mut result = Terminal::<Settlement<(), UnavailableTerminal>>::new();
        let mut mailbox = live_mailbox();

        ledger
            .publish(condition(incarnation, code, ConditionCause::ProgressLost))
            .unwrap();
        let terminal = custody.fail_unavailable(condition_id).unwrap();
        result.publish(Settlement::Failed(terminal)).unwrap();

        let before = mailbox.snapshot();
        let receipt = mailbox.post(Intent::StopScript).unwrap();
        assert_eq!(Intent::StopScript.meaning().run(), RunEffect::Terminate);
        assert!(
            !receipt.invalidated_attempt(),
            "{code}: Stop fires no login attempt"
        );
        let after = mailbox.snapshot();
        assert_eq!(
            after.login(),
            before.login(),
            "{code}: Stop implies no logout and arms no login"
        );
        assert_eq!(after.latch(), None);
        assert!(!after.logout_wanted());
        assert!(mailbox.logout_command(true).is_none());

        assert_eq!(
            custody.cancel(Cancellation::Operator),
            Err(CustodyError::AlreadyFailed),
            "{code}: the outage selected the terminal first"
        );
        assert_eq!(
            result.publish(Settlement::Cancelled(Cancellation::Operator)),
            Err(TerminalError::AlreadyPublished)
        );
        assert_eq!(result.borrow(), Some(&Settlement::Failed(terminal)));
        assert_eq!(
            (terminal.request(), terminal.scope(), terminal.condition()),
            (request, scope, condition_id),
            "{code}: the terminal keeps the request's own identity"
        );

        let AdmissionRefusal::Unavailable(refusal) =
            ledger.admit(capability, None, Scope::Run(run)).unwrap_err();
        assert_eq!(
            refusal.condition(),
            condition_id,
            "{code}: Stop is no retry"
        );
        assert!(!ledger.retry_allowed(capability));

        assert!(!result.consume().unwrap().is_completed());
        assert!(
            custody.charged(),
            "{code}: consuming the terminal released nothing"
        );
        completion_waits_for_the_real_return(&ledger, capability, condition_id, custody);
    }
}

/// §26 §3.2 terminal race, both orders, for Logout, Stop and Remove on a
/// live session whose accepted network request is stranded by the reactor:
/// whichever of the intent's cancellation and the outage's Failed/Unavailable
/// is selected first is the one immutable terminal; the intent keeps its
/// precedence (no fresh login, no retry); a Logout or Remove stays blocked
/// (logout still wanted) while its session is live and completes its session
/// side only at the exit; and the stranded charge outlives the consumed
/// terminal.
#[test]
fn an_intent_and_an_outage_select_one_terminal_in_either_order() {
    for (intent, reason) in [
        (Intent::Logout, Cancellation::ScopeInvalidated),
        (Intent::StopScript, Cancellation::Operator),
        (Intent::RemoveSlot, Cancellation::Shutdown),
    ] {
        for intent_first in [true, false] {
            let label = format!(
                "{intent:?} {} the outage",
                if intent_first { "before" } else { "after" }
            );
            let incarnation = incarnation(9);
            let reactor = ConditionId::new(incarnation, ProcessServiceCode::ReactorUnavailable);
            let slot = SlotKey::from_incarnation(13).unwrap();
            let scope = Scope::Session(SessionKey::new(slot, 1));
            let capability = ProcessCapability::NetworkEffect;
            let mut ledger = ProcessConditions::new(incarnation);
            let request = RequestIdMinter::new().mint().unwrap();
            ledger.admit(capability, Some(request), scope).unwrap();
            let mut custody = StrandedCustody::new(request, scope);
            let mut result = Terminal::<Settlement<(), UnavailableTerminal>>::new();
            let mut mailbox = live_mailbox();
            let outage = |ledger: &mut ProcessConditions| {
                ledger
                    .publish(condition(
                        incarnation,
                        ProcessServiceCode::ReactorUnavailable,
                        ConditionCause::Panicked,
                    ))
                    .unwrap();
            };

            let selected = if intent_first {
                mailbox.post(intent).unwrap();
                custody.cancel(reason).unwrap();
                result.publish(Settlement::Cancelled(reason)).unwrap();
                outage(&mut ledger);
                assert_eq!(
                    custody.fail_unavailable(reactor),
                    Err(CustodyError::AlreadyCancelled(reason)),
                    "{label}"
                );
                assert_eq!(
                    result.publish(Settlement::Failed(UnavailableTerminal::new(
                        reactor, request, scope
                    ))),
                    Err(TerminalError::AlreadyPublished)
                );
                assert_eq!(custody.terminal(), None, "{label}");
                assert_eq!(custody.cancellation(), Some(reason));
                Settlement::Cancelled(reason)
            } else {
                outage(&mut ledger);
                let terminal = custody.fail_unavailable(reactor).unwrap();
                result.publish(Settlement::Failed(terminal)).unwrap();
                mailbox.post(intent).unwrap();
                assert_eq!(
                    custody.cancel(reason),
                    Err(CustodyError::AlreadyFailed),
                    "{label}"
                );
                assert_eq!(
                    result.publish(Settlement::Cancelled(reason)),
                    Err(TerminalError::AlreadyPublished)
                );
                assert_eq!(custody.cancellation(), None, "{label}");
                assert_eq!(
                    (terminal.request(), terminal.scope(), terminal.condition()),
                    (request, scope, reactor),
                    "{label}"
                );
                Settlement::Failed(terminal)
            };
            assert_eq!(
                result.borrow(),
                Some(&selected),
                "{label}: one immutable terminal"
            );

            let snapshot = mailbox.snapshot();
            match intent {
                Intent::Logout => {
                    assert_eq!(snapshot.latch(), Some(LoginLatch::OperatorLogout));
                    assert_eq!(snapshot.login(), LoginIntent::Withdrawn);
                }
                Intent::StopScript => {
                    assert_eq!(snapshot.login(), LoginIntent::Auto, "Stop is no logout");
                    assert_eq!(snapshot.latch(), None);
                }
                _ => {
                    assert!(snapshot.closed());
                    assert_eq!(mailbox.post(Intent::Login), Err(MailboxError::Closed));
                }
            }
            let owes_press = intent != Intent::StopScript;
            assert_eq!(snapshot.logout_wanted(), owes_press, "{label}");
            if owes_press {
                let AdmissionRefusal::Unavailable(refusal) = ledger
                    .admit(capability, None, Scope::Slot(slot))
                    .unwrap_err();
                assert_eq!(
                    SessionEventOut::Unavailable {
                        capability: refusal.capability(),
                        condition: refusal.condition(),
                    },
                    SessionEventOut::Unavailable {
                        capability,
                        condition: reactor
                    },
                    "{label}: the press is refused and the slot projects the condition"
                );
                assert!(
                    mailbox.snapshot().logout_wanted(),
                    "{label}: blocked, not complete, while the session is live"
                );
            }

            // The reactor ends the live session.
            let end = SessionEndCause::ServiceUnavailable { condition: reactor };
            assert!(!end.is_clean());
            assert!(!end.counts_toward_repeat_guard());
            mailbox
                .observe(LifecycleFact::SessionExited {
                    clean: false,
                    resumable: false,
                })
                .unwrap();
            assert!(
                !mailbox.snapshot().logout_wanted(),
                "{label}: the exit completes the session side"
            );
            if intent == Intent::StopScript {
                assert!(
                    mailbox.login_command(false).is_some(),
                    "{label}: the login intent is preserved"
                );
                assert!(ledger
                    .admit(ProcessCapability::Login, None, Scope::Slot(slot))
                    .is_err());
            } else {
                assert!(
                    mailbox.login_command(false).is_none(),
                    "{label}: never a fresh login"
                );
            }
            assert!(!ledger.retry_allowed(ProcessCapability::Login));
            assert!(!ledger.retry_allowed(capability));

            assert_eq!(result.consume().unwrap(), selected);
            assert!(
                custody.charged(),
                "{label}: consuming the terminal released nothing"
            );
            completion_waits_for_the_real_return(&ledger, capability, reactor, custody);
        }
    }
}
