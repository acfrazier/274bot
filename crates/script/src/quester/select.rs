//! Ordered, tri-valued selection: only a proven `False` may start a step.
use super::compile::{CompiledPath, CompiledStep, PredicateContext};
use api::selected::Truth;

pub struct Selection<'a> {
    pub step: &'a CompiledStep,
    pub index: usize,
    pub prelude: bool,
}

pub enum SelectionDecision<'a> {
    Selected(Selection<'a>),
    Unknown,
    Exhausted,
}

/// Prelude first, then the current sequence. Unknown blocks lower priorities.
pub fn select<'a>(
    path: &'a CompiledPath,
    sequence: usize,
    cx: &PredicateContext<'_, '_>,
) -> SelectionDecision<'a> {
    for (prelude, steps) in [
        (true, path.prelude.as_slice()),
        (
            false,
            path.sequences
                .get(sequence)
                .map_or(&[], |seq| seq.steps.as_slice()),
        ),
    ] {
        for (index, step) in steps.iter().enumerate() {
            match step.skip_if.evaluate(cx) {
                Truth::True => {}
                Truth::Unknown => return SelectionDecision::Unknown,
                Truth::False => {
                    return SelectionDecision::Selected(Selection {
                        step,
                        index,
                        prelude,
                    })
                }
            }
        }
    }
    SelectionDecision::Exhausted
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
    fn any_empty_selects_and_unknown_skip_waits_on_empty_snapshot() {
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
        let bank = crate::quester::bank_memo::BankMemo::default();
        let pred = PredicateContext {
            cx: &cx,
            quests: &quests,
            progress: &[],
            required_after: stamp(),
            chat_since: 0,
            outcome: None,
            bank: &bank,
        };
        let SelectionDecision::Selected(picked) = select(&compiled, 0, &pred) else {
            panic!("never-skip start must select");
        };
        assert_eq!(picked.step.id.0.as_ref(), "start");
        assert!(!picked.prelude);

        let mut unknown = decode_cook().unwrap();
        unknown.roles[0].sequences[0].steps[0].skip_if = PredicateDocument::Fact {
            kind: "has_item".into(),
            version: 1,
            args: serde_json::json!({"obj": "egg"}),
        };
        let compiled = compile_uncached_for_test(&unknown, &data, &quests).unwrap();
        assert!(
            matches!(select(&compiled, 0, &pred), SelectionDecision::Unknown),
            "unknown inventory must wait, never select an action on login"
        );
    }
}
