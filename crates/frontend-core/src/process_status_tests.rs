use std::time::{Duration, Instant};

use api::identity::{ProcessIncarnation, ProcessOrigin, RequestIdMinter, Scope, SlotKeyMinter};
use api::process_condition::{
    ConditionCause, ConditionId, ProcessConditions, ProcessServiceCode, ProcessServiceCondition,
    PublishError,
};

use super::*;
use crate::command::{Admission, CommandBook, OperatorCommand};
use crate::selection::ProfileIdentity;

fn condition(
    incarnation: ProcessIncarnation,
    code: ProcessServiceCode,
    cause: ConditionCause,
    after: Duration,
) -> ProcessServiceCondition {
    let start = Instant::now();
    let detected = ProcessOrigin::at(start).elapsed(start + after);
    ProcessServiceCondition::new(ConditionId::new(incarnation, code), cause, detected, None)
}

#[test]
fn no_condition_means_no_status_and_nothing_refused() {
    let conditions = ProcessConditions::new(ProcessIncarnation::mint());
    let status = ProcessStatus::new(&conditions);
    assert!(!status.restart_required());
    assert_eq!(status.lines().count(), 0);
    for command in OperatorCommand::ALL {
        assert!(status.refusing(command).is_none());
        assert!(status.retry_allowed(command));
    }
}

#[test]
fn each_condition_maps_to_its_status_line_and_refusals() {
    let cases = [
        (
            ProcessServiceCode::ReactorUnavailable,
            OperatorCommand::Login,
            OperatorCommand::StartScript,
        ),
        (
            ProcessServiceCode::ResolverUnavailable,
            OperatorCommand::Login,
            OperatorCommand::StartScript,
        ),
        (
            ProcessServiceCode::WorkAdmissionUnavailable,
            OperatorCommand::StartScript,
            OperatorCommand::Login,
        ),
    ];
    for (code, refused, unaffected) in cases {
        let incarnation = ProcessIncarnation::mint();
        let mut conditions = ProcessConditions::new(incarnation);
        conditions
            .publish(condition(
                incarnation,
                code,
                ConditionCause::ProgressLost,
                Duration::from_secs(3),
            ))
            .unwrap();
        let status = ProcessStatus::new(&conditions);
        assert!(status.restart_required());
        let lines: Vec<_> = status.lines().collect();
        assert_eq!(lines.len(), 1);
        let line = lines[0];
        assert_eq!(line.code(), code);
        assert_eq!(line.id(), ConditionId::new(incarnation, code));
        assert_eq!(line.cause(), ConditionCause::ProgressLost);
        assert_eq!(line.operator_text(), code.operator_text());
        assert!(line
            .operator_text()
            .contains("Restart the application process."));

        let refusal = status.refusing(refused).expect("affected command refused");
        assert_eq!(refusal.id(), line.id());
        assert!(!status.retry_allowed(refused));
        assert!(
            status.refusing(unaffected).is_none(),
            "{code:?} refused {unaffected:?}"
        );
        assert!(status.retry_allowed(unaffected));
        // Retirement-dependent commands are never refused.
        for retire in [
            OperatorCommand::Logout,
            OperatorCommand::StopScript,
            OperatorCommand::Remove,
            OperatorCommand::Quit,
        ] {
            assert!(status.refusing(retire).is_none());
        }
    }
}

#[test]
fn the_status_and_the_command_report_agree_and_the_status_outlives_the_report() {
    // The surfaces read the same two projections: the persistent status and
    // the command's report. They must name the same condition and text, and
    // settling or evicting the command must not clear the status.
    let incarnation = ProcessIncarnation::mint();
    let mut conditions = ProcessConditions::new(incarnation);
    conditions
        .publish(condition(
            incarnation,
            ProcessServiceCode::ReactorUnavailable,
            ConditionCause::StartFailed,
            Duration::ZERO,
        ))
        .unwrap();
    let mut book = CommandBook::new();
    let requests = RequestIdMinter::new();
    let slot = SlotKeyMinter::new().mint().unwrap();
    let request = requests.mint().unwrap();
    book.open(request, OperatorCommand::Login).unwrap();
    let admission = book
        .admit(
            request,
            ProfileIdentity::uid(9),
            Scope::Slot(slot),
            &conditions,
        )
        .unwrap();

    let status = ProcessStatus::new(&conditions);
    let line = status.refusing(OperatorCommand::Login).unwrap();
    let Admission::Refused { condition, .. } = admission else {
        panic!("Login admitted under a latched reactor condition");
    };
    assert_eq!(condition, line.id());
    let report = book.report(request).unwrap();
    assert_eq!(report.command(), OperatorCommand::Login);
    let failure = match report.members()[0].terminal() {
        Some(api::identity::Settlement::Failed(failure)) => *failure,
        other => panic!("unexpected terminal {other:?}"),
    };
    assert_eq!(failure.operator_text(), Some(line.operator_text()));

    // A later command, a hidden banner or a settled report change nothing:
    // the ledger has no clear operation and the status still shows it.
    for _ in 0..crate::command::SETTLED_REPORT_CAP + 1 {
        let other = requests.mint().unwrap();
        book.open(other, OperatorCommand::Select).unwrap();
    }
    assert!(ProcessStatus::new(&conditions).restart_required());
    assert_eq!(ProcessStatus::new(&conditions).lines().count(), 1);
}

#[test]
fn a_stale_incarnation_never_reaches_the_status() {
    let current = ProcessIncarnation::mint();
    let previous = ProcessIncarnation::mint();
    let mut conditions = ProcessConditions::new(current);
    assert_eq!(
        conditions.publish(condition(
            previous,
            ProcessServiceCode::WorkAdmissionUnavailable,
            ConditionCause::Panicked,
            Duration::ZERO,
        )),
        Err(PublishError::StaleIncarnation)
    );
    let status = ProcessStatus::new(&conditions);
    assert!(!status.restart_required());
    assert!(status.refusing(OperatorCommand::StartScript).is_none());
}

#[test]
fn all_three_conditions_show_three_lines_in_code_order() {
    let incarnation = ProcessIncarnation::mint();
    let mut conditions = ProcessConditions::new(incarnation);
    for code in ProcessServiceCode::ALL.into_iter().rev() {
        conditions
            .publish(condition(
                incarnation,
                code,
                ConditionCause::Panicked,
                Duration::ZERO,
            ))
            .unwrap();
    }
    let codes: Vec<_> = ProcessStatus::new(&conditions)
        .lines()
        .map(StatusLine::code)
        .collect();
    assert_eq!(codes, ProcessServiceCode::ALL);
}
