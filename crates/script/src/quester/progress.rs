//! Colour-only stage read and live nav evidence. Journal is S1j.
use super::compile::CompiledPath;
use api::quest_facts::QuestCatalog;
use api::quest_progress::{EvidenceProvider, EvidenceStamp};
use api::selected::{FactKey, QuestGate, Truth};
use api::snapshot::{QuestListStatus, SnapshotView};

pub fn colour_stage(
    path: &CompiledPath,
    quests: &QuestCatalog,
    snapshot: SnapshotView<'_>,
) -> Option<FactKey> {
    let facts = quests.quest(path.id.0.as_ref()).ok()?;
    let rows = snapshot.quest_statuses()?;
    let row = rows
        .value
        .iter()
        .find(|row| row.name.eq_ignore_ascii_case(facts.display.as_ref()))?;
    match row.status() {
        QuestListStatus::NotStarted => Some(path.colour_not_started.clone()),
        QuestListStatus::Complete => Some(path.colour_complete.clone()),
        QuestListStatus::InProgress => {
            let id = path.id.0.as_ref();
            Some(FactKey::new(&format!("{id}:1")))
        }
        QuestListStatus::Unknown => None,
    }
}

/// Live evidence: `Complete` from tab colour; `Window` from a seeded
/// transmitted varp table only. Path `varp` hints are never consulted.
#[derive(Clone)]
pub struct LiveEvidence {
    pub colours: Vec<(FactKey, QuestListStatus)>,
    pub varps: Vec<(FactKey, i32)>,
    pub stamp: EvidenceStamp,
}

impl LiveEvidence {
    pub fn from_snapshot(
        snapshot: SnapshotView<'_>,
        quests: &QuestCatalog,
        stamp: EvidenceStamp,
    ) -> Self {
        let mut colours = Vec::new();
        if let Some(rows) = snapshot.quest_statuses() {
            for row in rows.value {
                if let Some(facts) = quests.by_display(&row.name) {
                    colours.push((facts.id.clone(), row.status()));
                }
            }
        }
        Self {
            colours,
            varps: Vec::new(),
            stamp,
        }
    }
}

impl EvidenceProvider for LiveEvidence {
    fn test_gate(&self, gate: &QuestGate, required_after: EvidenceStamp) -> Truth {
        if !self.stamp.meets(required_after) {
            return Truth::Unknown;
        }
        match gate {
            QuestGate::Complete(quest) => match self
                .colours
                .iter()
                .find(|(id, _)| id == quest)
                .map(|(_, status)| *status)
            {
                Some(QuestListStatus::Complete) => Truth::True,
                Some(_) => Truth::False,
                None => Truth::Unknown,
            },
            QuestGate::Window(window) => {
                let Some((_, value)) = self.varps.iter().find(|(id, _)| id == &window.signal)
                else {
                    return Truth::Unknown;
                };
                window.values.test(&api::selected::InclusiveRange {
                    min: Some(*value),
                    max: Some(*value),
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::quest_progress::EvidenceStamp;
    use api::selected::{FactKey, InclusiveRange, RunKey, StageWindow};
    use api::snapshot::QuestListStatus;

    fn stamp() -> EvidenceStamp {
        EvidenceStamp {
            run: RunKey {
                slot: 1,
                run: 1,
                session: 1,
            },
            tick: 1,
            sequence: 1,
        }
    }

    #[test]
    fn complete_from_seeded_colour_and_window_from_transmitted_varp_only() {
        let evidence = LiveEvidence {
            colours: vec![(FactKey::new("cook"), QuestListStatus::Complete)],
            varps: vec![(FactKey::new("misc_quest"), 3)],
            stamp: stamp(),
        };
        assert_eq!(
            evidence.test_gate(&QuestGate::Complete(FactKey::new("cook")), stamp()),
            Truth::True
        );
        assert_eq!(
            evidence.test_gate(&QuestGate::Complete(FactKey::new("imp")), stamp()),
            Truth::Unknown
        );
        let window = QuestGate::Window(StageWindow {
            quest: FactKey::new("misc"),
            signal: FactKey::new("misc_quest"),
            values: InclusiveRange {
                min: Some(3),
                max: Some(3),
            },
        });
        assert_eq!(evidence.test_gate(&window, stamp()), Truth::True);
        let other = QuestGate::Window(StageWindow {
            quest: FactKey::new("cook"),
            signal: FactKey::new("cookquest"),
            values: InclusiveRange {
                min: Some(1),
                max: Some(1),
            },
        });
        assert_eq!(evidence.test_gate(&other, stamp()), Truth::Unknown);
    }
}
