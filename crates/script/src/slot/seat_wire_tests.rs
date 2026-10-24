use super::*;
use crate::interact::InteractReq;
use crate::native::request::NativeKind;
use crate::native::seat::SeatTerminal;
use crate::native::seat_rows::{
    DecodedSeatBatch, SeatCancelDecoded, SeatDecisionInput, SeatInteractDecoded, SeatMachineEvent,
    SeatMachineEventKind, SeatMirrorRevoke, SeatPageInput, SeatStartInput,
};
use api::script_contract::{AbortReason, CallbackCancelReason};

fn id(raw: u64) -> RequestId {
    RequestId::isolate(raw).unwrap()
}

fn slot() -> SlotScript {
    let mut slot = SlotScript::new();
    slot.bind_incarnation(1);
    slot.arm_seat(api::selected::RunKey {
        slot: 1,
        run: 7,
        session: 2,
    });
    slot
}

fn batch() -> DecodedSeatBatch {
    DecodedSeatBatch {
        consumed_terminal_seq: 0,
        starts: Vec::new(),
        decisions: Vec::new(),
        cancels: Vec::new(),
        mirror_revokes: Vec::new(),
        reqs: Vec::new(),
    }
}

fn start(raw: u64, kind: NativeKind) -> SeatStartInput {
    SeatStartInput {
        id: id(raw),
        parent: None,
        run: 7,
        session: 2,
        kind,
        args: SeatStartArgs::Empty,
    }
}

#[test]
fn typed_batch_admits_before_dispatch_and_carries_physical_walk_stamp() {
    let mut slot = slot();
    let mut input = batch();
    input.starts.push(start(1, NativeKind::Hunt));
    input.reqs.push(SeatInteractDecoded {
        lineage: Some(id(1)),
        mirror_owner: Some(id(90)),
        request: InteractReq::WalkTo {
            x: 10,
            z: 20,
            level: 0,
        },
    });
    slot.consume_seat_batch(input, 23, &[]);
    assert_eq!(slot.seat.as_ref().unwrap().live()[0].id, id(1));
    let row = slot.seat_interact_delivery.pop().expect("admitted walk");
    assert_eq!(row.observed_walk_outcome_seq, 23);
    assert!(matches!(row.req, InteractReq::WalkTo { x: 10, z: 20, .. }));
}

#[test]
fn typed_batch_rejects_run_or_mirror_gap_before_watermark_and_start() {
    let mut slot = slot();
    let mut wrong_run = batch();
    wrong_run.starts.push(SeatStartInput {
        run: 8,
        ..start(1, NativeKind::Hunt)
    });
    slot.consume_seat_batch(wrong_run, 0, &[]);
    assert!(slot.seat.as_ref().unwrap().live().is_empty());
    let mut gap = batch();
    gap.starts.push(start(2, NativeKind::Hunt));
    gap.mirror_revokes.push(SeatMirrorRevoke {
        owner: id(90),
        seq: 2,
    });
    slot.consume_seat_batch(gap, 0, &[]);
    assert!(slot.seat.as_ref().unwrap().live().is_empty());
    assert_eq!(slot.seat.as_ref().unwrap().mirror_revokes_consumed(), 0);
    assert!(slot
        .pending_logs
        .iter()
        .any(|line| line.contains("malformed seat batch")));
}

#[test]
fn typed_batch_pairs_parent_cancel_with_pending_start_and_excludes_gameplay() {
    let mut slot = slot();
    let mut input = batch();
    input.starts.push(start(1, NativeKind::Hunt));
    input.cancels.push(SeatCancelDecoded {
        of: id(1),
        reason: CallbackCancelReason::ParentAborted,
    });
    input.reqs.push(SeatInteractDecoded {
        lineage: Some(id(1)),
        mirror_owner: None,
        request: InteractReq::WalkTo {
            x: 1,
            z: 2,
            level: 0,
        },
    });
    slot.consume_seat_batch(input, 0, &[]);
    assert!(slot.seat.as_ref().unwrap().live().is_empty());
    assert_eq!(
        slot.seat.as_ref().unwrap().results()[0].outcome,
        SeatTerminal::Aborted(AbortReason::ParentAborted),
    );
    assert!(slot.seat_interact_delivery.is_empty());
    assert_eq!(slot.seat.as_ref().unwrap().dropped(), 1);
}

#[test]
fn typed_mirror_revoke_is_applied_before_that_batch_gameplay() {
    let mut slot = slot();
    let mut input = batch();
    input.mirror_revokes.push(SeatMirrorRevoke {
        owner: id(90),
        seq: 1,
    });
    input.reqs.push(SeatInteractDecoded {
        lineage: None,
        mirror_owner: Some(id(90)),
        request: InteractReq::WalkTo {
            x: 1,
            z: 2,
            level: 0,
        },
    });
    slot.consume_seat_batch(input, 0, &[]);
    assert_eq!(slot.seat.as_ref().unwrap().mirror_revokes_consumed(), 1);
    assert!(slot.seat_interact_delivery.is_empty());
    assert_eq!(slot.seat.as_ref().unwrap().dropped(), 1);
}

#[test]
fn typed_pending_hook_throw_completes_without_revoking_final_emission() {
    let mut slot = slot();
    let mut input = batch();
    input.starts.push(start(1, NativeKind::Hunt));
    input.cancels.push(SeatCancelDecoded {
        of: id(1),
        reason: CallbackCancelReason::HookThrew,
    });
    input.reqs.push(SeatInteractDecoded {
        lineage: Some(id(1)),
        mirror_owner: None,
        request: InteractReq::WalkTo {
            x: 4,
            z: 5,
            level: 0,
        },
    });
    slot.consume_seat_batch(input, 11, &[]);
    let seat = slot.seat.as_ref().unwrap();
    assert!(seat.live().is_empty());
    assert_eq!(seat.results()[0].outcome, SeatTerminal::Failed);
    assert!(!seat.results()[0].authority_revoked);
    assert!(matches!(
        slot.seat_interact_delivery.as_slice(),
        [crate::interact::QueuedInteract {
            req: InteractReq::WalkTo { x: 4, z: 5, .. },
            observed_walk_outcome_seq: 11,
        }]
    ));
}

/// §28.7 case 28: the same decoded unit returns a result credit, consumes a
/// decision, admits a child, then dispatches that child's own earlier push.
#[test]
fn typed_batch_watermark_decide_start_and_own_push_share_turn() {
    use api::script_contract::DecisionSequence;

    let mut slot = slot();
    let mut first = batch();
    first.starts.push(start(1, NativeKind::BankNearest));
    slot.consume_seat_batch(first, 0, &[]);
    slot.seat.as_mut().unwrap().complete(id(1), false).unwrap();
    assert_eq!(slot.seat.as_ref().unwrap().results()[0].seq, 1);

    let mut session = batch();
    session.starts.push(start(2, NativeKind::Hunt));
    slot.consume_seat_batch(session, 0, &[]);
    assert_eq!(slot.seat.as_mut().unwrap().prompt(id(2)).unwrap(), Some(1));

    let mut input = batch();
    input.consumed_terminal_seq = 1;
    input.decisions.push(SeatDecisionInput {
        of: id(2),
        sequence: DecisionSequence::FIRST,
        answer: vec![9],
    });
    input.starts.push(SeatStartInput {
        parent: Some(id(2)),
        ..start(3, NativeKind::BankNearest)
    });
    input.reqs.push(SeatInteractDecoded {
        lineage: Some(id(3)),
        mirror_owner: None,
        request: InteractReq::WalkTo {
            x: 9,
            z: 10,
            level: 0,
        },
    });
    slot.consume_seat_batch(input, 37, &[]);
    let seat = slot.seat.as_mut().unwrap();
    assert!(
        seat.results().is_empty(),
        "watermark returned the previous result credit"
    );
    assert!(seat
        .live()
        .iter()
        .any(|entry| entry.id == id(3) && entry.parent == Some(id(2))));
    assert_eq!(seat.take_answer(id(2)).unwrap().answer, [9]);
    assert!(matches!(
        slot.seat_interact_delivery.as_slice(),
        [crate::interact::QueuedInteract {
            req: InteractReq::WalkTo { x: 9, z: 10, .. },
            observed_walk_outcome_seq: 37,
        }]
    ));
}

/// §28.7 1/13/13b: the native seat owns prompt bytes, page state and answer
/// custody; the first callback I-family will supply a product hook producer.
#[test]
fn native_prompt_page_decision_and_late_copy_are_bounded_and_ordered() {
    use api::script_contract::{DecisionSequence, PROMPT_ARGS_MAX_BYTES};

    let point = crate::native::PromptPoint::from_contract_row(
        crate::native::abi::HOOK_CHARACTERIZATIONS
            .iter()
            .find(|row| row.member() == "sustain")
            .expect("awaited sustain contract row"),
    )
    .expect("awaited hook point");
    let mut slot = slot();
    let mut starts = batch();
    starts.starts.push(start(1, NativeKind::Hunt));
    slot.consume_seat_batch(starts, 0, &[]);
    let seat = slot.seat.as_mut().unwrap();
    assert_eq!(
        seat.prompt_with_details(id(1), point, &[1, 2, 3]),
        Ok(DecisionSequence::FIRST)
    );
    assert_eq!(
        seat.prompt_with_details(id(1), point, &[4]),
        Err(crate::native::seat::SeatError::AwaitingDecision)
    );
    {
        let details = seat_live_details(seat, None, None, None);
        let _page = SeatPageInput::new(
            seat.live(),
            seat.results(),
            &details[..seat.live().len()],
            0,
        )
        .expect("real seat prompt page");
        assert_eq!(seat.live().len(), 1);
        let prompt = details[0].prompt.expect("borrowed native prompt");
        assert_eq!(prompt.point, "sustain");
        assert_eq!(prompt.args, &[1, 2, 3]);
        assert_eq!(prompt.sequence.get(), 1);
    }
    assert_eq!(
        seat.prompt_with_details(id(1), point, &[0; PROMPT_ARGS_MAX_BYTES + 1]),
        Err(crate::native::seat::SeatError::PromptTooLarge)
    );

    let mut decided = batch();
    decided.decisions.push(SeatDecisionInput {
        of: id(1),
        sequence: DecisionSequence::FIRST,
        answer: vec![9, 8, 7],
    });
    slot.consume_seat_batch(decided, 0, &[]);
    let seat = slot.seat.as_mut().unwrap();
    assert_eq!(seat.live()[0].prompt, None);
    assert_eq!(seat.live()[0].decisions_consumed, 1);
    assert_eq!(
        seat.take_answer(id(1)).expect("native decision").answer,
        vec![9, 8, 7]
    );
    assert_eq!(
        seat.prompt_with_details(id(1), point, &[5; PROMPT_ARGS_MAX_BYTES]),
        Ok(DecisionSequence::FIRST.next().unwrap())
    );
    assert_eq!(
        seat.prompt_payload(id(1)).unwrap().args,
        [5; PROMPT_ARGS_MAX_BYTES]
    );
    let late_before = seat.late_controls();
    let logs_before = slot.pending_logs.len();
    let mut late = batch();
    late.decisions.push(SeatDecisionInput {
        of: id(1),
        sequence: DecisionSequence::FIRST,
        answer: vec![0],
    });
    slot.consume_seat_batch(late, 0, &[]);
    let seat = slot.seat.as_ref().unwrap();
    assert_eq!(seat.late_controls(), late_before + 1);
    assert_eq!(seat.live()[0].prompt, Some(2));
    assert_eq!(
        slot.pending_logs.len(),
        logs_before,
        "duplicate is counted, never logged"
    );
    let mut invalid = batch();
    invalid.decisions.push(SeatDecisionInput {
        of: id(1),
        sequence: DecisionSequence::from_raw(3).unwrap(),
        answer: vec![0],
    });
    slot.consume_seat_batch(invalid, 0, &[]);
    assert_eq!(slot.seat.as_ref().unwrap().live()[0].prompt, Some(2));
    assert_eq!(slot.seat.as_ref().unwrap().late_controls(), late_before + 1);
    assert!(slot
        .pending_logs
        .last()
        .unwrap()
        .contains("did not match a live prompt"));
}

/// §28.7 7/24: actual isolate machine transitions update the native page and
/// settle through the seat before a captured final emission dispatches.
#[test]
fn carried_machine_phase_and_terminal_events_preserve_final_row_authority() {
    use crate::native::seat_rows::{
        SeatMachineEvent as MachineEvent, SeatMachineEventKind as MachineEventKind,
    };
    use crate::native::NativePhase;

    let mut slot = slot();
    let mut admitted = batch();
    admitted.starts.push(start(1, NativeKind::Hunt));
    slot.consume_seat_batch(
        admitted,
        0,
        &[MachineEvent {
            id: id(1),
            kind: MachineEventKind::Phase(NativePhase::Waiting),
            admitted: true,
        }],
    );
    let seat = slot.seat.as_ref().unwrap();
    assert_eq!(seat.machine_phase(id(1)), Some(NativePhase::Waiting));
    let details = seat_live_details(seat, None, None, None);
    assert_eq!(details[0].phase, NativePhase::Waiting);

    let mut final_row = batch();
    final_row.reqs.push(SeatInteractDecoded {
        lineage: Some(id(1)),
        mirror_owner: None,
        request: InteractReq::WalkTo {
            x: 21,
            z: 22,
            level: 0,
        },
    });
    slot.consume_seat_batch(
        final_row,
        4,
        &[MachineEvent {
            id: id(1),
            kind: MachineEventKind::Done,
            admitted: true,
        }],
    );
    let seat = slot.seat.as_ref().unwrap();
    assert_eq!(seat.machine_phase(id(1)), None);
    assert_eq!(seat.results()[0].outcome, SeatTerminal::Done);
    assert!(matches!(
        slot.seat_interact_delivery.as_slice(),
        [crate::interact::QueuedInteract {
            req: InteractReq::WalkTo { x: 21, z: 22, .. },
            observed_walk_outcome_seq: 4,
        }]
    ));
}

/// §28.7 3/11/22: the one real consumer handles failed, revoked, and manual
/// Walk exits rather than leaving a completed JS machine live in the forest.
#[test]
fn carried_machine_failure_abort_and_takeover_settle_without_ghost_entries() {
    use crate::native::seat_rows::{
        SeatMachineEvent as MachineEvent, SeatMachineEventKind as MachineEventKind,
    };

    for (kind, expected) in [
        (MachineEventKind::Failed, SeatTerminal::Failed),
        (
            MachineEventKind::Aborted(AbortReason::ParentAborted),
            SeatTerminal::Aborted(AbortReason::ParentAborted),
        ),
        (MachineEventKind::ManualWalk, SeatTerminal::Done),
    ] {
        let mut slot = slot();
        let mut admitted = batch();
        admitted.starts.push(start(1, NativeKind::Walk));
        slot.consume_seat_batch(admitted, 0, &[]);
        slot.consume_seat_batch(
            batch(),
            0,
            &[MachineEvent {
                id: id(1),
                kind,
                admitted: true,
            }],
        );
        let seat = slot.seat.as_ref().unwrap();
        assert!(seat.live().is_empty());
        assert_eq!(seat.results()[0].outcome, expected);
        assert_eq!(
            seat.results()[0].authority_revoked,
            !matches!(kind, MachineEventKind::Failed),
        );
    }
}

#[test]
fn late_control_plain_batches_do_not_revive_terminal_or_log() {
    use api::script_contract::DecisionSequence;

    for cancel in [false, true] {
        let mut slot = slot();
        let mut admitted = batch();
        admitted.starts.push(start(1, NativeKind::BankNearest));
        slot.consume_seat_batch(admitted, 0, &[]);
        slot.consume_seat_batch(
            batch(),
            0,
            &[SeatMachineEvent {
                id: id(1),
                kind: SeatMachineEventKind::Done,
                admitted: true,
            }],
        );
        let seq = slot.seat.as_ref().unwrap().results()[0].seq;
        let mut acknowledged = batch();
        acknowledged.consumed_terminal_seq = seq;
        slot.consume_seat_batch(acknowledged, 0, &[]);
        assert!(slot.seat.as_ref().unwrap().results().is_empty());
        let logs_before = slot.pending_logs.len();

        for _ in 0..50 {
            let mut duplicate = batch();
            duplicate.consumed_terminal_seq = seq;
            if cancel {
                duplicate.cancels.push(SeatCancelDecoded {
                    of: id(1),
                    reason: CallbackCancelReason::HookThrew,
                });
            } else {
                duplicate.decisions.push(SeatDecisionInput {
                    of: id(1),
                    sequence: DecisionSequence::FIRST,
                    answer: Vec::new(),
                });
            }
            slot.consume_seat_batch(duplicate, 0, &[]);
        }
        let seat = slot.seat.as_ref().unwrap();
        assert_eq!(seat.late_controls(), 50);
        assert!(seat.live().is_empty());
        assert!(seat.results().is_empty());
        assert_eq!(slot.pending_logs.len(), logs_before);
        assert_eq!(
            slot.slot_metrics(),
            SlotMetrics {
                dropped: seat.dropped(),
                late_controls: 50,
            },
            "50 plain late copies are visible without reviving a terminal"
        );
    }
}

#[test]
fn plain_batch_starts_preserve_four_roots_nested_parents_and_busy_refusal() {
    use crate::bank::request::{BankSeatArgs, BankSeatOperation};
    use api::script_contract::Refusal;

    let mut slot = slot();
    let starts = [
        (1, None, NativeKind::Prayer),
        (2, None, NativeKind::Autocast),
        (3, None, NativeKind::BankNearest),
        (4, None, NativeKind::PeriodicBank),
        (5, Some(1), NativeKind::Teleport),
        (6, Some(5), NativeKind::Walk),
        (7, None, NativeKind::Modals),
    ]
    .into_iter()
    .map(|(raw, parent, kind)| SeatStartInput {
        id: id(raw),
        parent: parent.map(id),
        run: 7,
        session: 2,
        kind,
        args: if kind == NativeKind::BankNearest {
            SeatStartArgs::Bank(BankSeatArgs::new(BankSeatOperation::BankNearest))
        } else {
            SeatStartArgs::Empty
        },
    })
    .collect();
    slot.consume_seat_batch(
        DecodedSeatBatch {
            consumed_terminal_seq: 0,
            starts,
            decisions: Vec::new(),
            cancels: Vec::new(),
            mirror_revokes: Vec::new(),
            reqs: Vec::new(),
        },
        0,
        &[],
    );
    let seat = slot.seat.as_ref().unwrap();
    assert_eq!(seat.live().len(), 6);
    assert_eq!(
        seat.live()
            .iter()
            .filter(|entry| entry.parent.is_none())
            .count(),
        4
    );
    assert_eq!(
        seat.live()
            .iter()
            .find(|entry| entry.id == id(5))
            .unwrap()
            .parent,
        Some(id(1))
    );
    assert_eq!(
        seat.live()
            .iter()
            .find(|entry| entry.id == id(6))
            .unwrap()
            .parent,
        Some(id(5))
    );
    assert_eq!(seat.results().len(), 1);
    assert_eq!(seat.results()[0].id, id(7));
    assert_eq!(
        seat.results()[0].outcome,
        SeatTerminal::Refused(Refusal::Busy)
    );
}
