//! Shared planner fixtures and source-boundary checks for test builds.

use std::sync::Arc;

use api::prereq_knowledge::Observation;
use api::quest_progress::{EvidenceProvider, EvidenceStamp};
use api::selected::{FactKey, InclusiveRange, QuestGate, RunKey, StageWindow, Truth};

use crate::quest_gates::QuestEvidence;

pub mod source_scan;

/// A stand-in for the runner's pinned provider: the values fresh evidence
/// still allows per `(quest, signal)` and journal completion per quest,
/// observed at `stamp`. Windows are decided by the shared
/// [`InclusiveRange::test`], as the catalog decides them.
pub struct Resolved {
    pub stamp: EvidenceStamp,
    pub signals: Vec<(&'static str, &'static str, InclusiveRange)>,
    pub complete: Vec<(&'static str, bool)>,
}

impl EvidenceProvider for Resolved {
    fn observe_gate(&self, gate: &QuestGate, required_after: EvidenceStamp) -> Option<Observation> {
        if !self.stamp.meets(required_after) {
            return None;
        }
        let truth = match gate {
            QuestGate::Complete(quest) => self
                .complete
                .iter()
                .find(|(key, _)| *key == &*quest.0)
                .map(|&(_, done)| if done { Truth::True } else { Truth::False }),
            QuestGate::Window(window) => self
                .signals
                .iter()
                .find(|(quest, signal, _)| {
                    *quest == &*window.quest.0 && *signal == &*window.signal.0
                })
                .map(|(_, _, possible)| window.values.test(possible)),
        }?;
        Some(Observation {
            truth,
            observed: Some(self.stamp),
        })
    }
}

pub fn stamp(tick: u64) -> EvidenceStamp {
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

pub fn range(min: Option<i32>, max: Option<i32>) -> InclusiveRange {
    InclusiveRange { min, max }
}

pub fn window(quest: &str, signal: &str, min: Option<i32>, max: Option<i32>) -> QuestGate {
    QuestGate::Window(StageWindow {
        quest: FactKey::new(quest),
        signal: FactKey::new(signal),
        values: range(min, max),
    })
}

/// Evidence observed at tick 10 against a floor of tick 5, leaving
/// `possible` for one `tbwt`/`tbwt_main` signal.
pub fn tbwt_evidence(possible: InclusiveRange) -> QuestEvidence {
    QuestEvidence::new(
        Arc::new(Resolved {
            stamp: stamp(10),
            signals: vec![("tbwt", "tbwt_main", possible)],
            complete: vec![],
        }),
        stamp(5),
    )
}

#[test]
fn resolved_provider_preserves_its_observation_stamp() {
    let provider = Resolved {
        stamp: stamp(10),
        signals: vec![("quest", "signal", range(Some(3), Some(3)))],
        complete: vec![],
    };
    let gate = window("quest", "signal", Some(1), Some(5));
    assert_eq!(
        provider.observe_gate(&gate, stamp(5)),
        Some(Observation {
            truth: Truth::True,
            observed: Some(stamp(10)),
        })
    );
    assert_eq!(provider.test_gate(&gate, stamp(5)), Truth::True);
    assert_eq!(provider.observe_gate(&gate, stamp(11)), None);
    assert_eq!(
        provider.observe_gate(&window("other", "signal", Some(1), Some(5)), stamp(5)),
        None
    );
}
