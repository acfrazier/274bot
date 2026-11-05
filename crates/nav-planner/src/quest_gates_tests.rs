use std::mem::size_of;
use std::sync::Arc;

use api::selected::{FactKey, QuestGate, Truth};

use super::{QuestEvidence, QuestGateError, QuestGates};
use crate::test_support::{range, stamp, tbwt_evidence, window, Resolved};
use crate::transport::TransportEdge;

fn gates(list: Vec<QuestGate>) -> QuestGates {
    QuestGates::new(list).expect("valid gates")
}

#[test]
fn shared_gates_keeps_the_canonical_gate_allocation() {
    let gates = gates(vec![QuestGate::Complete(FactKey::new("arrav"))]);
    let first = gates.shared_gates();
    let second = gates.shared_gates();
    let clone = gates.clone();
    let cloned = clone.shared_gates();
    assert!(Arc::ptr_eq(&first, &second));
    assert!(Arc::ptr_eq(&first, &cloned));
    assert_eq!(first.as_ref(), gates.gates());
}

#[test]
fn quest_gate_layout_stays_compact() {
    let canonical: Arc<[QuestGate]> = vec![QuestGate::Complete(FactKey::new("arrav"))].into();
    let mut shared = None;
    let allocation = allocation_counter::measure(|| {
        shared = Some(Arc::new(super::GateSet {
            gates: Arc::clone(&canonical),
        }));
    });
    assert_eq!(allocation.count_total, 1, "one allocation per shared set");
    eprintln!(
        "quest gate layout QuestGates={} OptionQuestGates={} TransportEdge={} GateSet={} GateSetAllocation={}",
        size_of::<QuestGates>(),
        size_of::<Option<QuestGates>>(),
        size_of::<TransportEdge>(),
        size_of::<super::GateSet>(),
        allocation.bytes_current,
    );
    assert!(shared.is_some(), "the measurement retains the shared set");
    assert_eq!(
        size_of::<QuestGates>(),
        size_of::<usize>(),
        "QuestGates is one shared thin pointer"
    );
    assert_eq!(
        size_of::<Option<QuestGates>>(),
        size_of::<usize>(),
        "Option<QuestGates> uses the Arc null niche"
    );
    // The former 56-byte Option<QuestGates> added 48 bytes to every edge.
    assert!(
        size_of::<TransportEdge>() <= 344,
        "TransportEdge should stay within 8 bytes of the measured 336-byte layout"
    );
}

#[test]
fn equality_window_holds_only_when_every_possible_value_is_its_value() {
    let exact = gates(vec![window("tbwt", "tbwt_main", Some(3), Some(3))]);
    let truth = |min, max| exact.test(Some(&tbwt_evidence(range(min, max))));
    assert_eq!(truth(Some(3), Some(3)), Truth::True);
    assert_eq!(
        truth(Some(3), Some(4)),
        Truth::Unknown,
        "4 is still possible"
    );
    assert_eq!(
        truth(Some(2), Some(3)),
        Truth::Unknown,
        "2 is still possible"
    );
    assert_eq!(truth(Some(4), Some(6)), Truth::False);
    assert_eq!(truth(None, Some(2)), Truth::False);
    assert_eq!(truth(Some(0), None), Truth::Unknown, "unbounded evidence");
}

#[test]
fn upper_only_window_holds_up_to_and_including_its_bound() {
    let before = gates(vec![window("tbwt", "tbwt_main", None, Some(5))]);
    let truth = |min, max| before.test(Some(&tbwt_evidence(range(min, max))));
    assert_eq!(truth(Some(0), Some(5)), Truth::True);
    assert_eq!(truth(None, Some(5)), Truth::True);
    assert_eq!(truth(Some(5), Some(5)), Truth::True);
    assert_eq!(truth(Some(5), Some(6)), Truth::Unknown);
    assert_eq!(truth(Some(6), None), Truth::False);
}

#[test]
fn missing_stale_or_unresolved_evidence_never_authorizes() {
    let exact = gates(vec![window("tbwt", "tbwt_main", Some(3), Some(3))]);
    let proven = Arc::new(Resolved {
        stamp: stamp(10),
        signals: vec![("tbwt", "tbwt_main", range(Some(3), Some(3)))],
        complete: vec![],
    });
    let with = |floor| QuestEvidence::new(proven.clone(), floor);
    assert_eq!(exact.test(Some(&with(stamp(5)))), Truth::True);
    assert_eq!(exact.test(None), Truth::Unknown, "no evidence");
    assert_eq!(
        exact.test(Some(&with(stamp(11)))),
        Truth::Unknown,
        "evidence older than the causal floor"
    );
    let other_signal = gates(vec![window("tbwt", "tbwt_side", Some(3), Some(3))]);
    assert_eq!(
        other_signal.test(Some(&with(stamp(5)))),
        Truth::Unknown,
        "a signal the evidence does not resolve"
    );
}

#[test]
fn every_gate_on_an_edge_must_hold() {
    let evidence = QuestEvidence::new(
        Arc::new(Resolved {
            stamp: stamp(10),
            signals: vec![
                ("tbwt", "tbwt_main", range(Some(3), Some(3))),
                ("heroes", "heroes_main", range(Some(1), Some(4))),
            ],
            complete: vec![("arrav", true), ("dragon", false)],
        }),
        stamp(5),
    );
    let exact = window("tbwt", "tbwt_main", Some(3), Some(3));
    let undecided = window("heroes", "heroes_main", Some(2), Some(2));
    let disproven = window("tbwt", "tbwt_main", Some(5), None);
    let test = |list| gates(list).test(Some(&evidence));
    assert_eq!(
        test(vec![
            exact.clone(),
            QuestGate::Complete(FactKey::new("arrav"))
        ]),
        Truth::True
    );
    assert_eq!(test(vec![exact.clone(), undecided.clone()]), Truth::Unknown);
    assert_eq!(test(vec![undecided, disproven]), Truth::False);
    assert_eq!(
        test(vec![exact, QuestGate::Complete(FactKey::new("dragon"))]),
        Truth::False
    );
}

#[test]
fn gate_lists_are_validated_and_canonical() {
    let empty: Vec<QuestGate> = vec![];
    assert_eq!(QuestGates::new(empty), Err(QuestGateError::Empty));
    assert_eq!(
        QuestGates::new([QuestGate::Complete(FactKey::new(""))]),
        Err(QuestGateError::EmptyKey)
    );
    assert_eq!(
        QuestGates::new([window("tbwt", "", Some(1), Some(1))]),
        Err(QuestGateError::EmptyKey)
    );
    assert_eq!(
        QuestGates::new([window("tbwt", "tbwt_main", Some(4), Some(3))]),
        Err(QuestGateError::EmptyWindow)
    );
    assert_eq!(
        QuestGates::new([window("tbwt", "tbwt_main", None, None)]),
        Err(QuestGateError::Unbounded)
    );
    let a = window("tbwt", "tbwt_main", Some(3), Some(3));
    let b = QuestGate::Complete(FactKey::new("arrav"));
    let forward = gates(vec![a.clone(), b.clone(), a.clone()]);
    let backward = gates(vec![b, a]);
    assert_eq!(
        forward, backward,
        "order and repeats do not change the gates"
    );
    assert_eq!(forward.gates().len(), 2);
}
