use std::num::NonZeroU16;
use std::sync::Arc;

use api::quest_progress::{EvidenceProvider, EvidenceStamp};
use api::selected::{FactKey, InclusiveRange, QuestGate, RunKey, StageWindow, Truth};

use super::{QuestEvidence, QuestFamilyId, QuestFamilyMismatch, QuestGateError, QuestGates};
use crate::transport::TransportGraph;

/// A stand-in for the runner's pinned provider: the values fresh evidence
/// still allows per `(quest, signal)` and journal completion per quest,
/// observed at `stamp`. Windows are decided by the shared
/// [`InclusiveRange::test`], as the catalog decides them.
pub(crate) struct Resolved {
    pub(crate) stamp: EvidenceStamp,
    pub(crate) signals: Vec<(&'static str, &'static str, InclusiveRange)>,
    pub(crate) complete: Vec<(&'static str, bool)>,
}

impl EvidenceProvider for Resolved {
    fn test_gate(&self, gate: &QuestGate, required_after: EvidenceStamp) -> Truth {
        if !self.stamp.meets(required_after) {
            return Truth::Unknown;
        }
        match gate {
            QuestGate::Complete(quest) => self
                .complete
                .iter()
                .find(|(key, _)| *key == &*quest.0)
                .map_or(Truth::Unknown, |&(_, done)| {
                    if done {
                        Truth::True
                    } else {
                        Truth::False
                    }
                }),
            QuestGate::Window(window) => self
                .signals
                .iter()
                .find(|(quest, signal, _)| {
                    *quest == &*window.quest.0 && *signal == &*window.signal.0
                })
                .map_or(Truth::Unknown, |(_, _, possible)| {
                    window.values.test(possible)
                }),
        }
    }
}

pub(crate) fn family(byte: u8) -> QuestFamilyId {
    family_schema(byte, 1)
}

pub(crate) fn family_schema(byte: u8, schema: u16) -> QuestFamilyId {
    QuestFamilyId::new([byte; 32], NonZeroU16::new(schema).unwrap())
}

pub(crate) fn stamp(tick: u64) -> EvidenceStamp {
    EvidenceStamp {
        run: RunKey {
            slot: 1,
            run: 1,
            session: 1,
        },
        tick,
        sequence: 0,
    }
}

pub(crate) fn range(min: Option<i32>, max: Option<i32>) -> InclusiveRange {
    InclusiveRange { min, max }
}

pub(crate) fn window(quest: &str, signal: &str, min: Option<i32>, max: Option<i32>) -> QuestGate {
    QuestGate::Window(StageWindow {
        quest: FactKey::new(quest),
        signal: FactKey::new(signal),
        values: range(min, max),
    })
}

/// Evidence for family 1, observed at tick 10 against a floor of tick 5,
/// leaving `possible` for one `tbwt`/`tbwt_main` signal.
pub(crate) fn tbwt_evidence(possible: InclusiveRange) -> QuestEvidence {
    QuestEvidence::new(
        Arc::new(Resolved {
            stamp: stamp(10),
            signals: vec![("tbwt", "tbwt_main", possible)],
            complete: vec![],
        }),
        family(1),
        stamp(5),
    )
}

fn gates(list: Vec<QuestGate>) -> QuestGates {
    QuestGates::new(family(1), list).expect("valid gates")
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
fn missing_stale_or_foreign_evidence_never_authorizes() {
    let exact = gates(vec![window("tbwt", "tbwt_main", Some(3), Some(3))]);
    let proven = Resolved {
        stamp: stamp(10),
        signals: vec![("tbwt", "tbwt_main", range(Some(3), Some(3)))],
        complete: vec![],
    };
    let proven = Arc::new(proven);
    let with = |family, floor| QuestEvidence::new(proven.clone(), family, floor);
    assert_eq!(exact.test(Some(&with(family(1), stamp(5)))), Truth::True);
    assert_eq!(exact.test(None), Truth::Unknown, "no evidence");
    assert_eq!(
        exact.test(Some(&with(family(1), stamp(11)))),
        Truth::Unknown,
        "evidence older than the causal floor"
    );
    assert_eq!(
        exact.test(Some(&with(family(2), stamp(5)))),
        Truth::Unknown,
        "another quest family digest"
    );
    let other_schema = family_schema(1, 2);
    assert_eq!(
        exact.test(Some(&with(other_schema, stamp(5)))),
        Truth::Unknown,
        "another extractor schema"
    );
    let other_signal = gates(vec![window("tbwt", "tbwt_side", Some(3), Some(3))]);
    assert_eq!(
        other_signal.test(Some(&with(family(1), stamp(5)))),
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
        family(1),
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
    assert_eq!(
        QuestGates::new(family(1), empty),
        Err(QuestGateError::Empty)
    );
    assert_eq!(
        QuestGates::new(family(1), [QuestGate::Complete(FactKey::new(""))]),
        Err(QuestGateError::EmptyKey)
    );
    assert_eq!(
        QuestGates::new(family(1), [window("tbwt", "", Some(1), Some(1))]),
        Err(QuestGateError::EmptyKey)
    );
    assert_eq!(
        QuestGates::new(family(1), [window("tbwt", "tbwt_main", Some(4), Some(3))]),
        Err(QuestGateError::EmptyWindow)
    );
    assert_eq!(
        QuestGates::new(family(1), [window("tbwt", "tbwt_main", None, None)]),
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

#[test]
fn pack_admission_requires_the_manifest_quest_family() {
    let bound = TransportGraph {
        quest_family: Some(family(1)),
        ..TransportGraph::default()
    };
    assert_eq!(bound.admit_quest_family(&family(1)), Ok(()));
    assert_eq!(
        bound.admit_quest_family(&family(2)),
        Err(QuestFamilyMismatch {
            pack: Some(family(1)),
            expected: family(2),
        })
    );
    let newer_schema = family_schema(1, 2);
    assert!(bound.admit_quest_family(&newer_schema).is_err());
    let unbound = TransportGraph::default();
    assert_eq!(
        unbound.admit_quest_family(&family(1)),
        Err(QuestFamilyMismatch {
            pack: None,
            expected: family(1),
        })
    );
}
