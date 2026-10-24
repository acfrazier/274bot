//! Shared planner fixtures and source-boundary checks for test builds.

use std::num::NonZeroU16;
use std::sync::Arc;

use api::quest_progress::{EvidenceProvider, EvidenceStamp};
use api::selected::{FactKey, InclusiveRange, QuestGate, RunKey, StageWindow, Truth};

use crate::quest_gates::{QuestEvidence, QuestFamilyId};

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

pub fn family(byte: u8) -> QuestFamilyId {
    family_schema(byte, 1)
}

pub fn family_schema(byte: u8, schema: u16) -> QuestFamilyId {
    QuestFamilyId::new([byte; 32], NonZeroU16::new(schema).expect("Quest fixture schemas must be nonzero"))
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

/// Evidence for family 1, observed at tick 10 against a floor of tick 5,
/// leaving `possible` for one `tbwt`/`tbwt_main` signal.
pub fn tbwt_evidence(possible: InclusiveRange) -> QuestEvidence {
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
