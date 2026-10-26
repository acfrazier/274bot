use api::identity::{ExecutionId, RequestId};
use api::script_contract::{CallbackCancelReason, DecisionSequence, Refusal, RequestStamp};
use script::native::request::NativeKind;
use script::native::seat::SeatTerminal;
use script::native::seat_rows::{
    DecodedSeatBatch, SeatMachineEvent as MachineEvent, SeatMachineEventKind as MachineEventKind,
    SeatStartArgs, SeatStartInput,
};
use script::slot::{SlotMetrics, SlotScript};
use script_js::wire::{
    decode_seat_interact_batch, encode_seat_interact_batch, SeatCancelInput, SeatDecisionRef,
    SeatInteractBatchInput, SeatStartArgsRef, SeatStartRef,
};

fn id(raw: u64) -> RequestId {
    RequestId::isolate(raw).unwrap()
}

fn slot() -> SlotScript {
    let mut slot = SlotScript::new();
    slot.bind_incarnation(1);
    slot.arm_seat_for_test(api::selected::RunKey {
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

/// §28.7 case 30: the host's JS wire decoder and slot consumer receive fifty
/// separately encoded late copies of each control after a real result receipt.
#[test]
fn fifty_late_control_copies_retire_in_slot_metrics_without_logs() {
    for cancel in [false, true] {
        let mut slot = slot();
        let mut admitted = batch();
        admitted.starts.push(start(1, NativeKind::BankNearest));
        slot.consume_seat_batch_for_test(admitted, 0, &[]);
        slot.consume_seat_batch_for_test(
            batch(),
            0,
            &[MachineEvent {
                id: id(1),
                kind: MachineEventKind::Done,
                admitted: true,
            }],
        );
        // The fresh seat has published exactly one terminal.
        let seq = slot.seat_for_test().unwrap().results()[0].seq;
        let mut acknowledged = batch();
        acknowledged.consumed_terminal_seq = seq;
        slot.consume_seat_batch_for_test(acknowledged, 0, &[]);
        assert!(slot.seat_for_test().unwrap().results().is_empty());
        let logs_before = slot.pending_log_len_for_test();
        let decisions = [SeatDecisionRef {
            of: id(1),
            sequence: DecisionSequence::FIRST,
            answer: &[],
        }];
        let cancels = [SeatCancelInput {
            of: id(1),
            reason: CallbackCancelReason::HookThrew,
        }];

        for _ in 0..50 {
            let input = SeatInteractBatchInput {
                consumed_terminal_seq: seq,
                starts: &[],
                decisions: if cancel { &[] } else { decisions.as_slice() },
                cancels: if cancel { cancels.as_slice() } else { &[] },
                mirror_revokes: &[],
                reqs: &[],
            };
            let wire = encode_seat_interact_batch(&input).expect("encode duplicate control");
            let duplicate = decode_seat_interact_batch(&wire).expect("decode duplicate control");
            slot.consume_seat_batch_for_test(duplicate, 0, &[]);
        }
        let seat = slot.seat_for_test().unwrap();
        assert_eq!(seat.late_controls(), 50);
        assert!(seat.live().is_empty());
        assert!(seat.results().is_empty());
        assert_eq!(slot.pending_log_len_for_test(), logs_before);
        assert_eq!(
            slot.slot_metrics(),
            SlotMetrics {
                dropped: seat.dropped(),
                late_controls: 50,
            },
            "50 encoded late copies must be visible without the test-support probe"
        );
        #[cfg(feature = "test-hooks")]
        assert_eq!(slot.seat_probe().unwrap().late_controls, 50);
    }
}

/// §28.7 case 20: seven actual encoded starts exercise four distinct
/// capability roots, nested parentage, and the projected fifth-root refusal.
#[test]
fn nested_four_root_wire_starts_publish_the_fifth_root_refusal() {
    let mut slot = slot();
    let execution = ExecutionId::first(slot.seat_for_test().unwrap().run());
    let bank = script::bank::request::BankSeatArgs::new(
        script::bank::request::BankSeatOperation::BankNearest,
    );
    let starts = [
        (1, None, NativeKind::Prayer),
        (2, None, NativeKind::Autocast),
        (3, None, NativeKind::BankNearest),
        (4, None, NativeKind::PeriodicBank),
        (5, Some(1), NativeKind::Teleport),
        (6, Some(5), NativeKind::Walk),
        (7, None, NativeKind::Modals),
    ]
    .map(|(raw, parent, kind)| SeatStartRef {
        stamp: RequestStamp::new(id(raw), parent.map(id), None, execution)
            .expect("current seat execution"),
        kind,
        args: if kind == NativeKind::BankNearest {
            SeatStartArgsRef::Bank(&bank)
        } else {
            SeatStartArgsRef::Empty
        },
    });
    let input = SeatInteractBatchInput {
        consumed_terminal_seq: 0,
        starts: &starts,
        decisions: &[],
        cancels: &[],
        mirror_revokes: &[],
        reqs: &[],
    };
    let wire = encode_seat_interact_batch(&input).expect("encode seven ordered starts");
    let decoded = decode_seat_interact_batch(&wire).expect("decode seven ordered starts");
    slot.consume_seat_batch_for_test(decoded, 0, &[]);

    let seat = slot.seat_for_test().unwrap();
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
