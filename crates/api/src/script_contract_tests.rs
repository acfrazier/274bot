use super::*;
use crate::identity::{SlotKey, Terminal};

fn id(raw: u64) -> RequestId {
    RequestId::isolate(raw).unwrap()
}
fn stamp(raw: u64) -> RequestStamp {
    let run = RunKey {
        slot: 1,
        run: 2,
        session: 3,
    };
    RequestStamp::new(id(raw), None, None, ExecutionId::first(run)).unwrap()
}

#[test]
fn foreign_lineage_retains_its_diagnostic_and_uses_only_the_innermost_guard() {
    assert_eq!(Lineage::Foreign.parent(Some(id(7))), Some(id(7)));
    assert_eq!(Lineage::Foreign.parent(None), None);
    assert_eq!(Lineage::Root.parent(Some(id(7))), None);
    assert_eq!(Lineage::Token(id(5)).parent(Some(id(7))), Some(id(5)));
}
#[test]
fn captured_authority_is_bounded_and_cannot_invent_a_session_or_cycle() {
    let authority =
        CapturedAuthority::new(id(5), &[id(4), id(3), id(2), id(1)], Some(id(1))).unwrap();
    assert_eq!(authority.owner(), id(5));
    assert_eq!(
        authority.ancestors().collect::<Vec<_>>(),
        vec![id(4), id(3), id(2), id(1)]
    );
    assert_eq!(authority.session(), Some(id(1)));
    assert_eq!(
        CapturedAuthority::new(id(5), &[id(5)], None),
        Err(ContractError::CyclicLineage)
    );
    assert_eq!(
        CapturedAuthority::new(id(5), &[id(4), id(4)], None),
        Err(ContractError::CyclicLineage)
    );
    assert_eq!(
        CapturedAuthority::new(id(5), &[id(4)], Some(id(1))),
        Err(ContractError::SessionOutsideLineage)
    );
    assert_eq!(
        CapturedAuthority::new(id(6), &[id(5), id(4), id(3), id(2), id(1)], None),
        Err(ContractError::LineageDepth)
    );
}
#[test]
fn completion_is_not_revocation_and_manual_walk_uses_a_separate_boundary_bit() {
    let done: Outcome<bool, ()> = Outcome::done(false);
    let failed: Outcome<bool, ()> = Outcome::failed(());
    let refused: Outcome<bool, ()> = Outcome::refused(Refusal::Busy);
    assert!(!done.revokes_authority());
    assert!(!failed.revokes_authority());
    assert!(!refused.revokes_authority());
    assert!(matches!(
        done.settlement(),
        Settlement::Completed(Completion::Done(false))
    ));
    for reason in [
        AbortReason::Reset,
        AbortReason::Superseded,
        AbortReason::Terminated,
        AbortReason::UserInput,
        AbortReason::ParentAborted,
    ] {
        let outcome: Outcome<(), ()> = Outcome::aborted(reason);
        assert_eq!(outcome.abort_reason(), Some(reason));
        assert!(
            matches!(outcome.settlement(), Settlement::Cancelled(value) if *value == reason.cancellation())
        );
        assert!(outcome.revokes_authority());
    }
    assert_eq!(
        dispatch_disposition(
            EmissionClass::Gameplay,
            None,
            None,
            AdmissionWitness::Admitted,
            EmissionProof::Proven,
            AuthorityBoundary::Revoked,
            false
        ),
        DispatchDisposition::Drop
    );
}
#[test]
fn guardian_and_foreground_are_separate_and_controls_are_never_filtered() {
    for foreground in [None, Some(id(1))] {
        for boundary in [AuthorityBoundary::Preserved, AuthorityBoundary::Revoked] {
            for witness in [AdmissionWitness::Pending, AdmissionWitness::Admitted] {
                assert_eq!(
                    dispatch_disposition(
                        EmissionClass::Host,
                        None,
                        foreground,
                        witness,
                        EmissionProof::InvalidExternal,
                        boundary,
                        true
                    ),
                    DispatchDisposition::Dispatch
                );
                for class in [EmissionClass::Release, EmissionClass::Lifecycle] {
                    assert_eq!(
                        dispatch_disposition(
                            class,
                            None,
                            foreground,
                            witness,
                            EmissionProof::InvalidExternal,
                            boundary,
                            false
                        ),
                        DispatchDisposition::Dispatch
                    );
                    assert_eq!(
                        dispatch_disposition(
                            class,
                            None,
                            foreground,
                            witness,
                            EmissionProof::InvalidExternal,
                            boundary,
                            true
                        ),
                        DispatchDisposition::Retain
                    );
                }
                assert_eq!(
                    dispatch_disposition(
                        EmissionClass::Gameplay,
                        Some(id(1)),
                        foreground,
                        witness,
                        EmissionProof::Proven,
                        boundary,
                        true
                    ),
                    DispatchDisposition::Drop
                );
            }
        }
    }
    assert_eq!(
        dispatch_disposition(
            EmissionClass::Gameplay,
            Some(id(1)),
            Some(id(1)),
            AdmissionWitness::Admitted,
            EmissionProof::Proven,
            AuthorityBoundary::Preserved,
            false
        ),
        DispatchDisposition::Dispatch
    );
    assert_eq!(
        dispatch_disposition(
            EmissionClass::Gameplay,
            Some(id(1)),
            Some(id(2)),
            AdmissionWitness::Admitted,
            EmissionProof::Proven,
            AuthorityBoundary::Preserved,
            false
        ),
        DispatchDisposition::Drop
    );
    assert_eq!(
        dispatch_disposition(
            EmissionClass::Gameplay,
            Some(id(1)),
            None,
            AdmissionWitness::Admitted,
            EmissionProof::Proven,
            AuthorityBoundary::Preserved,
            false
        ),
        DispatchDisposition::Dispatch
    );
    assert_eq!(
        dispatch_disposition(
            EmissionClass::Gameplay,
            None,
            None,
            AdmissionWitness::Pending,
            EmissionProof::Proven,
            AuthorityBoundary::Preserved,
            false
        ),
        DispatchDisposition::Drop
    );
    assert_eq!(
        dispatch_disposition(
            EmissionClass::Gameplay,
            None,
            None,
            AdmissionWitness::Admitted,
            EmissionProof::ExpiredLookup,
            AuthorityBoundary::Preserved,
            false
        ),
        DispatchDisposition::Drop
    );
    assert_eq!(
        dispatch_disposition(
            EmissionClass::Gameplay,
            None,
            None,
            AdmissionWitness::Admitted,
            EmissionProof::InvalidExternal,
            AuthorityBoundary::Preserved,
            false
        ),
        DispatchDisposition::ProtocolDefect
    );
    assert_eq!(
        dispatch_disposition(
            EmissionClass::Gameplay,
            Some(id(1)),
            Some(id(1)),
            AdmissionWitness::Admitted,
            EmissionProof::InvalidExternal,
            AuthorityBoundary::Preserved,
            true
        ),
        DispatchDisposition::ProtocolDefect,
        "a hold must not hide an invalid-external protocol defect"
    );
}

#[test]
fn boundary_delivery_revocation_and_release_custody_are_exhaustive() {
    use Boundary::*;
    const ALL: [Boundary; 14] = [
        Done,
        Failed,
        HookThrew,
        ParentAborted,
        Superseded,
        ManualWalk,
        ManualCombat,
        GatherSignal,
        Stop,
        Reset,
        KeptReconnect,
        Pause,
        GuardianHold,
        HeldTick,
    ];
    for boundary in ALL {
        match boundary {
            Done | Failed | HookThrew | ParentAborted | Superseded | ManualWalk | ManualCombat => {
                assert_eq!(boundary.delivery(), BoundaryDelivery::RetainedTerminal);
            }
            Reset => assert_eq!(boundary.delivery(), BoundaryDelivery::LocalReset),
            Stop => assert_eq!(boundary.delivery(), BoundaryDelivery::SilentStop),
            GatherSignal | KeptReconnect | Pause | GuardianHold | HeldTick => {
                assert_eq!(boundary.delivery(), BoundaryDelivery::Hold);
            }
        }
        assert_eq!(
            boundary.revokes_descendants(),
            matches!(
                boundary,
                ParentAborted | Superseded | ManualWalk | ManualCombat | Stop | Reset
            )
        );
        assert_eq!(
            boundary.keeps_queued_releases(),
            !matches!(boundary, Stop | Reset | HeldTick),
            "{boundary:?} release custody"
        );
    }
}
#[test]
fn pages_enforce_combined_credits_and_sequence_order_not_just_two_separate_caps() {
    let live: Vec<_> = (1..=MAX_LIVE)
        .map(|raw| {
            LiveStatus::<_, ()>::new(stamp(raw as u64), ChildOrigin::Started, "running", 0, None)
                .unwrap()
        })
        .collect();
    let results: Vec<_> = (1..=17)
        .map(|raw| {
            ResultRecord::new(
                id(100 + raw),
                TerminalSequence::from_raw(raw).unwrap(),
                Outcome::<(), ()>::done(()),
            )
        })
        .collect();
    assert!(StatusPage::new(&live, &results[..16]).is_ok());
    assert_eq!(
        StatusPage::new(&live, &results).unwrap_err(),
        ContractError::ResultCredits
    );
    let reversed = [
        ResultRecord::new(
            id(20),
            TerminalSequence::from_raw(2).unwrap(),
            Outcome::<(), ()>::done(()),
        ),
        ResultRecord::new(id(21), TerminalSequence::FIRST, Outcome::done(())),
    ];
    assert_eq!(
        StatusPage::new(&[] as &[LiveStatus<(), ()>], &reversed).unwrap_err(),
        ContractError::TerminalOrder
    );
    let duplicate = [
        ResultRecord::new(id(20), TerminalSequence::FIRST, Outcome::<(), ()>::done(())),
        ResultRecord::new(id(21), TerminalSequence::FIRST, Outcome::done(())),
    ];
    assert_eq!(
        StatusPage::new(&[] as &[LiveStatus<(), ()>], &duplicate).unwrap_err(),
        ContractError::TerminalOrder
    );
}
#[test]
fn prompt_is_page_state_and_a_terminal_transfers_once_through_the_shared_cell() {
    let live = LiveStatus::new(
        stamp(1),
        ChildOrigin::Started,
        "awaiting-sustain",
        0,
        Some(Prompt::new(DecisionSequence::FIRST, "sustain", ())),
    )
    .unwrap();
    assert_eq!(live.phase(), &"awaiting-sustain");
    assert_eq!(live.prompt().unwrap().sequence(), DecisionSequence::FIRST);
    assert_eq!(
        LiveStatus::new(
            stamp(1),
            ChildOrigin::Started,
            "awaiting-sustain",
            1,
            Some(Prompt::new(DecisionSequence::FIRST, "sustain", ()))
        )
        .unwrap_err(),
        ContractError::DecisionOrder
    );
    let mut terminal = Terminal::new();
    terminal
        .publish(ResultRecord::new(
            id(1),
            TerminalSequence::FIRST,
            Outcome::<bool, ()>::done(true),
        ))
        .unwrap();
    assert_eq!(
        terminal.borrow().unwrap().sequence(),
        TerminalSequence::FIRST
    );
    let record = terminal.consume().unwrap();
    assert!(matches!(
        record.into_outcome().into_settlement().0,
        Settlement::Completed(Completion::Done(true))
    ));
    assert!(terminal.consume().is_err());
}
#[test]
fn stamps_reuse_f0_execution_and_scope_and_sequences_never_wrap() {
    let header = stamp(1);
    assert_eq!(
        header.session().slot(),
        SlotKey::from_incarnation(1).unwrap()
    );
    assert_eq!(header.scope(), Scope::Execution(header.execution()));
    let mut fence = crate::identity::Fence::new();
    fence.arm(header.execution());
    assert!(fence.admit(header.execution()).is_ok());
    fence.arm(header.execution().next_generation().unwrap());
    assert!(fence.admit(header.execution()).is_err());
    assert_eq!(
        TerminalSequence::from_raw(u64::MAX).unwrap().next(),
        Err(ContractError::SequenceExhausted)
    );
    assert_eq!(
        DecisionSequence::from_raw(u32::MAX).unwrap().next(),
        Err(ContractError::SequenceExhausted)
    );
    assert!(TerminalSequence::from_raw(0).is_none());
    assert!(DecisionSequence::from_raw(0).is_none());
}
