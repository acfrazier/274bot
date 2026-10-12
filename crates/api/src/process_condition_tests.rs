use super::*;
use crate::identity::{ProcessIncarnation, ProcessOrigin, RequestIdMinter, SlotKey};
use std::time::{Duration, Instant};

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
    let incarnation = ProcessIncarnation::mint();
    let mut ledger = ProcessConditions::new(incarnation);
    assert_eq!(ledger.snapshot().count(), 0);
    assert!(ledger
        .latched(ProcessServiceCode::ReactorUnavailable)
        .is_none());

    let first = condition(
        incarnation,
        ProcessServiceCode::ReactorUnavailable,
        ConditionCause::Panicked,
    );
    ledger.publish(first.clone()).unwrap();
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
fn a_condition_from_another_incarnation_is_never_current_health() {
    let current = ProcessIncarnation::mint();
    let previous = ProcessIncarnation::mint();
    let mut ledger = ProcessConditions::new(current);
    let stale = condition(
        previous,
        ProcessServiceCode::ResolverUnavailable,
        ConditionCause::ProgressLost,
    );
    assert_eq!(ledger.publish(stale), Err(PublishError::StaleIncarnation));
    assert!(ledger.retry_allowed(ProcessCapability::NameResolution));
    assert!(ledger
        .latched(ProcessServiceCode::ResolverUnavailable)
        .is_none());
    assert_eq!(ledger.incarnation(), current);
}

#[test]
fn admission_refuses_only_affected_operations_and_keeps_their_identity() {
    let incarnation = ProcessIncarnation::mint();
    let mut ledger = ProcessConditions::new(incarnation);
    let slot = SlotKey::from_incarnation(4).unwrap();
    let request = RequestIdMinter::new().mint().unwrap();
    let scope = Scope::Slot(slot);

    assert_eq!(
        ledger.admit(ProcessCapability::Login, Some(request), scope),
        Ok(())
    );
    ledger
        .publish(condition(
            incarnation,
            ProcessServiceCode::ReactorUnavailable,
            ConditionCause::ProgressLost,
        ))
        .unwrap();

    let refusal = ledger
        .admit(ProcessCapability::Login, Some(request), scope)
        .unwrap_err();
    assert_eq!(
        refusal.condition(),
        ConditionId::new(incarnation, ProcessServiceCode::ReactorUnavailable)
    );
    assert_eq!(refusal.request(), Some(request));
    assert_eq!(refusal.scope(), scope);
    assert_eq!(refusal.capability(), ProcessCapability::Login);
    assert_eq!(refusal.recoverability(), Recoverability::RestartRequired);
    assert_eq!(
        refusal.operator_text(),
        ProcessServiceCode::ReactorUnavailable.operator_text()
    );
    assert!(refusal
        .to_string()
        .starts_with("Game transport is unavailable."));

    assert_eq!(
        ledger.admit(ProcessCapability::NameResolution, None, Scope::Process),
        Ok(()),
        "an unrelated service continues"
    );
    assert_eq!(
        ledger.admit(ProcessCapability::LocalOnly, Some(request), scope),
        Ok(())
    );
    assert_eq!(
        ledger.admit(ProcessCapability::BackgroundWork, None, Scope::Process),
        Ok(())
    );
    assert!(
        !ledger.retry_allowed(ProcessCapability::Reconnect),
        "no automatic retry until restart"
    );
    assert!(ledger.retry_allowed(ProcessCapability::BackgroundWork));
}

#[test]
fn a_failure_report_settles_custody_but_acknowledges_no_retirement() {
    let incarnation = ProcessIncarnation::mint();
    let request = RequestIdMinter::new().mint().unwrap();
    let scope = Scope::Process;
    let mut custody = StrandedCustody::new(request, scope);
    assert!(custody.charged());
    assert_eq!(custody.terminal(), None);

    let condition = ConditionId::new(incarnation, ProcessServiceCode::WorkAdmissionUnavailable);
    let terminal = custody.fail_unavailable(condition).unwrap();
    assert_eq!(terminal.condition(), condition);
    assert_eq!(terminal.request(), request);
    assert_eq!(terminal.scope(), scope);
    assert_eq!(
        terminal.operator_text(),
        ProcessServiceCode::WorkAdmissionUnavailable.operator_text()
    );
    assert_eq!(custody.terminal(), Some(terminal));
    assert!(custody.charged(), "the logical failure released nothing");
    assert_eq!(
        custody.fail_unavailable(condition).unwrap_err(),
        CustodyError::AlreadyFailed,
        "one logical terminal; a late report cannot rewrite it"
    );

    let still_charged = custody.release().unwrap_err();
    assert!(
        still_charged.charged(),
        "no deadline or report stands in for the worker's return"
    );
    custody = still_charged;
    custody.worker_returned();
    assert!(!custody.charged());
    assert_eq!(
        custody.terminal(),
        Some(terminal),
        "a late return cannot replace the terminal"
    );
    custody.release().unwrap();
}

#[test]
fn diagnostics_are_bounded_on_char_boundaries_and_do_not_drive_behaviour() {
    let long = "é".repeat(200);
    let diagnostic = Diagnostic::bounded(&long);
    assert!(diagnostic.as_str().len() <= DIAGNOSTIC_MAX_BYTES);
    assert_eq!(diagnostic.as_str().len(), DIAGNOSTIC_MAX_BYTES);
    assert!(diagnostic.as_str().chars().all(|c| c == 'é'));
    assert_eq!(Diagnostic::bounded("short").as_str(), "short");

    let incarnation = ProcessIncarnation::mint();
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
