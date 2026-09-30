//! First step whose `skip_if` is not `True`.
use super::compile::{CompiledPath, CompiledStep, PredicateContext};
use api::selected::Truth;

pub struct Selection<'a> {
    pub step: &'a CompiledStep,
    pub index: usize,
    pub prelude: bool,
}

/// Prelude first, then the current sequence. `Unknown` does not skip.
pub fn select<'a>(
    path: &'a CompiledPath,
    sequence: usize,
    cx: &PredicateContext<'_, '_>,
) -> Option<Selection<'a>> {
    for (index, step) in path.prelude.iter().enumerate() {
        if step.skip_if.evaluate(cx) != Truth::True {
            return Some(Selection {
                step,
                index,
                prelude: true,
            });
        }
    }
    let sequence = path.sequences.get(sequence)?;
    for (index, step) in sequence.steps.iter().enumerate() {
        if step.skip_if.evaluate(cx) != Truth::True {
            return Some(Selection {
                step,
                index,
                prelude: false,
            });
        }
    }
    None
}

pub fn sequence_for_stage(path: &CompiledPath, stage: &str) -> Option<usize> {
    path.sequences
        .iter()
        .position(|sequence| sequence.stage.0.as_ref() == stage)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::ledger::TickBudget;
    use crate::native::{ActionContext, RetainedMemory};
    use crate::quester::compile::{compile_uncached_for_test, decode_cook, PredicateContext};
    use crate::quester::path::PredicateDocument;
    use api::game_data::SelectedGameData;
    use api::quest_facts::QuestCatalog;
    use api::quest_progress::EvidenceStamp;
    use api::selected::{ClientRevision, RunKey};
    use api::snapshot::{GameSnapshot, SnapshotView};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    fn selected() -> Arc<SelectedGameData> {
        api::game_data::for_revision(ClientRevision::R289).unwrap()
    }

    fn quests(data: &SelectedGameData) -> QuestCatalog {
        QuestCatalog::from_identity(data.quest_identity()).unwrap_or_else(|_| QuestCatalog::empty())
    }

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
    fn any_empty_selects_and_unknown_skip_still_selects_on_empty_snapshot() {
        let data = selected();
        let pin = data.selected_pin().unwrap();
        let quests = quests(&data);
        let document = decode_cook().unwrap();
        let compiled = compile_uncached_for_test(&document, &data, &quests).unwrap();
        let snapshot = GameSnapshot::new();
        let view = SnapshotView::new(Some(&snapshot), stamp());
        let mut retained = RetainedMemory::default();
        let mut ledger = None;
        let mut budget = TickBudget::default();
        let cx = ActionContext {
            evidence: stamp(),
            pin: &pin,
            snapshot: view,
            retained: &mut retained,
            action_id: 0,
            active_now: Duration::ZERO,
            wall_now: Instant::now(),
            ledger: &mut ledger,
            budget: &mut budget,
            eligible: true,
        };
        let pred = PredicateContext {
            cx: &cx,
            quests: &quests,
            progress: &[],
            required_after: stamp(),
            chat_since: 0,
            outcome: None,
        };
        let picked = select(&compiled, 0, &pred).expect("never-skip start");
        assert_eq!(picked.step.id.0.as_ref(), "start");
        assert!(!picked.prelude);

        let mut unknown = decode_cook().unwrap();
        unknown.roles[0].sequences[0].steps[0].skip_if = PredicateDocument::Fact {
            kind: "has_item".into(),
            version: 1,
            args: serde_json::json!({"obj": "egg"}),
        };
        let compiled = compile_uncached_for_test(&unknown, &data, &quests).unwrap();
        let picked = select(&compiled, 0, &pred).expect("unknown skip_if still selects");
        assert_eq!(picked.step.id.0.as_ref(), "start");
    }
}
