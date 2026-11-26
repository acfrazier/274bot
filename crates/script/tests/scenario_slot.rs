use script::slot::{
    RetirementTicket, RunState, SlotExecutionObservation, SlotExecutionRef, StopAuthorization,
};
use script::surface::extension::{
    ExtensionEnd, ExtensionEndCause, ExtensionHookFailure, RunBinding,
};

fn ticket() -> RetirementTicket {
    RetirementTicket {
        execution: SlotExecutionRef {
            slot_incarnation: 9,
            runtime_generation: 17,
            start_serial: 4,
        },
        authorization: StopAuthorization {
            plan_id: 31,
            step_ordinal: 6,
            stop_ordinal: 2,
        },
        after_generation: 18,
    }
}

fn idle(ticket: RetirementTicket) -> SlotExecutionObservation {
    SlotExecutionObservation {
        state: RunState::Idle,
        slot_incarnation: ticket.execution.slot_incarnation,
        start_serial: ticket.execution.start_serial,
        runtime_generation: 18,
        execution: None,
        retirement_ticket: Some(ticket),
        extension_end: None,
    }
}

#[test]
fn retirement_ticket_matches_only_its_retained_idle_lineage() {
    let ticket = ticket();
    let mut observation = idle(ticket);

    assert!(ticket.matches_idle(&observation));
    assert_eq!(ticket, observation.retirement_ticket.unwrap());
    assert_eq!(
        ticket.execution,
        observation.retirement_ticket.unwrap().execution
    );

    observation.state = RunState::Running;
    assert!(!ticket.matches_idle(&observation));
    observation.state = RunState::Idle;

    observation.execution = Some(ticket.execution);
    assert!(!ticket.matches_idle(&observation));
    observation.execution = None;

    observation.runtime_generation += 1;
    assert!(!ticket.matches_idle(&observation));
    observation.runtime_generation -= 1;

    observation.start_serial += 1;
    assert!(!ticket.matches_idle(&observation));
    observation.start_serial -= 1;

    observation.slot_incarnation += 1;
    assert!(!ticket.matches_idle(&observation));
}

#[test]
fn clean_end_requires_the_expected_cause_binding_and_no_hook_error() {
    let binding = RunBinding::new(9, 17, 44);
    let mut end = ExtensionEnd::new(binding, ExtensionEndCause::Stopped);
    end.vm_dropped = true;
    assert!(end.is_clean_end(binding, ExtensionEndCause::Stopped));
    assert!(!end.is_clean_end(binding, ExtensionEndCause::Completed));
    assert!(!end.is_clean_end(RunBinding::new(9, 18, 44), ExtensionEndCause::Stopped));

    end.cause = ExtensionEndCause::Completed;
    assert!(end.is_clean_end(binding, ExtensionEndCause::Completed));
    assert!(!end.is_clean_end(binding, ExtensionEndCause::Stopped));

    end.hook_failure = Some(ExtensionHookFailure::new("luau-script", "onStop failed"));
    assert!(!end.is_clean_end(binding, ExtensionEndCause::Completed));
}
