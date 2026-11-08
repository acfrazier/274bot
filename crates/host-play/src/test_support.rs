use std::sync::Arc;

use api::prereq_knowledge::Observation;
use api::quest_progress::{EvidenceProvider, EvidenceStamp};
use api::selected::{FactKey, InclusiveRange, QuestGate, RunKey, StageWindow, Truth};
use nav_planner::quest_gates::QuestEvidence;

struct Resolved {
    stamp: EvidenceStamp,
    signals: Vec<(&'static str, &'static str, InclusiveRange)>,
    complete: Vec<(&'static str, bool)>,
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

/// Evidence observed at tick 10 against a floor of tick 5, with
/// `possible` for the `tbwt`/`tbwt_main` signal.
pub(crate) fn tbwt_evidence(possible: InclusiveRange) -> QuestEvidence {
    QuestEvidence::new(
        Arc::new(Resolved {
            stamp: stamp(10),
            signals: vec![("tbwt", "tbwt_main", possible)],
            complete: vec![],
        }),
        stamp(5),
    )
}
