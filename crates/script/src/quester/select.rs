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
    for (index, step) in path.prelude.iter().enumerate() {
        match step.skip_if.evaluate(cx) {
            Truth::True => {}
            Truth::Unknown => return SelectionDecision::Unknown,
            Truth::False => {
                return SelectionDecision::Selected(Selection { step, index, prelude: true });
            }
        }
    }
    let Some(sequence) = path.sequences.get(sequence) else {
        return SelectionDecision::Exhausted;
    };
    let nearest = sequence.order == super::path::SequenceOrder::Nearest;
    let here = nearest.then(|| cx.cx.snapshot().here()).flatten();
    let mut chosen = None;
    let mut best = (i64::MAX, i32::MAX);
    for (index, step) in sequence.steps.iter().enumerate() {
        match step.skip_if.evaluate(cx) {
            Truth::True => {}
            Truth::Unknown => return SelectionDecision::Unknown,
            Truth::False => {
                let candidate = Selection { step, index, prelude: false };
                if !nearest {
                    return SelectionDecision::Selected(candidate);
                }
                let (Some(here), Some(anchor)) = (here.as_ref(), step.plan.anchor()) else {
                    return SelectionDecision::Unknown;
                };
                let distance = (i64::from(here.value.x) - i64::from(anchor.x)).abs()
                    .max((i64::from(here.value.z) - i64::from(anchor.z)).abs());
                let score = (distance, (here.value.level - anchor.level).abs());
                if score < best {
                    best = score;
                    chosen = Some(candidate);
                }
            }
        }
    }
    chosen.map_or(SelectionDecision::Exhausted, SelectionDecision::Selected)
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
    use crate::quester::compile::{
        compile_uncached_for_test, decode_cook, PredicateContext, RUNE_MYSTERIES_JSON, SHEEP_JSON,
    };
    use crate::quester::path::{PathDocument, PredicateDocument};
    use api::game_data::SelectedGameData;
    use api::obj_names::ItemDefView;
    use api::quest_facts::QuestCatalog;
    use api::quest_progress::{EvidenceStamp, ProgressFlag, QuestProgress};
    use api::selected::{ClientRevision, FactKey, Knowledge, RunKey, Truth};
    use api::snapshot::{GameSnapshot, ItemActionFamily, ItemContainer, ItemView, SnapshotView};
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

    fn path(json: &str, data: &SelectedGameData, quests: &QuestCatalog) -> Arc<CompiledPath> {
        let document: PathDocument = serde_json::from_str(json).unwrap();
        compile_uncached_for_test(&document, data, quests).unwrap()
    }

    fn inventory_snapshot(data: &SelectedGameData, items: &[(&str, i32)]) -> GameSnapshot {
        let rows = items
            .iter()
            .enumerate()
            .map(|(slot, (alias, count))| {
                let item = data.item_by_alias(alias).unwrap();
                ItemView {
                    def: ItemDefView {
                        id: item.id,
                        name: Some((*alias).into()),
                        stackable: false,
                        members: false,
                        base_value: 1,
                        noted: false,
                        certificate_link: -1,
                        certificate_template: -1,
                    },
                    container: ItemContainer::Inventory,
                    action_family: ItemActionFamily::Held,
                    slot: slot as i32,
                    count: *count,
                    actions: Vec::new(),
                    component_id: 0,
                }
            })
            .collect();
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_inventory(rows, 28);
        snapshot.seed_equipment(Vec::new());
        snapshot
    }

    fn progress(
        path: &CompiledPath,
        data: &SelectedGameData,
        stage: &str,
        flags: &[(&str, Truth, Option<u32>)],
    ) -> QuestProgress {
        let stage = FactKey::new(stage);
        QuestProgress {
            quest: path.id.clone(),
            stage: Knowledge::Known(stage.clone()),
            complete: Truth::False,
            signals: Arc::from(Vec::new()),
            flags: Arc::from(
                flags
                    .iter()
                    .map(|(flag, truth, count)| ProgressFlag {
                        flag: FactKey::new(flag),
                        truth: *truth,
                        count: *count,
                    })
                    .collect::<Vec<_>>(),
            ),
            evidence: stamp(),
            binding: path.progress.binding.clone(),
            role: path.progress.role.clone(),
            rule: Knowledge::Known(stage),
            pin: data.selected_pin().unwrap(),
        }
    }

    #[derive(Debug, PartialEq, Eq)]
    enum Choice {
        Step(String),
        Unknown,
        Exhausted,
    }

    fn choice_for_stage(
        path: &CompiledPath,
        stage: &str,
        snapshot: &GameSnapshot,
        quests: &QuestCatalog,
        progress: &[QuestProgress],
        bank: &crate::quester::bank_memo::BankMemo,
    ) -> Choice {
        let sequence = sequence_for_stage(path, stage).unwrap();
        let mut ledger = None;
        crate::quester::families::tests::with_tick(snapshot, &mut ledger, 1, |tick| {
            let cx = PredicateContext { cx: &tick.cx, pairs: tick.pairs, quests,
            progress,
            required_after: stamp(),
            chat_since: 0,
            outcome: None,
            bank, };
            match select(path, sequence, &cx) {
                SelectionDecision::Selected(selected) => {
                    Choice::Step(selected.step.id.0.to_string())
                }
                SelectionDecision::Unknown => Choice::Unknown,
                SelectionDecision::Exhausted => Choice::Exhausted,
            }
        })
    }

    fn known_empty_bank() -> crate::quester::bank_memo::BankMemo {
        let mut bank = crate::quester::bank_memo::BankMemo::default();
        bank.update(&crate::native_bank::BankReceipt {
            counts: Vec::new(),
            complete: true,
        });
        bank
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
            observed_walk_outcome_seq: 0,
            active_now: Duration::ZERO,
            wall_now: Instant::now(),
            ledger: &mut ledger,
            budget: &mut budget,
            eligible: true,
        };
        let bank = crate::quester::bank_memo::BankMemo::default();
        let pred = PredicateContext { cx: &cx, pairs: None, quests: &quests,
        progress: &[],
        required_after: stamp(),
        chat_since: 0,
        outcome: None,
        bank: &bank, };
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

    #[test]
    fn romeo_known_empty_bank_advances_to_missing_item_acquisition() {
        let data = selected();
        let quests = quests(&data);
        let document =
            serde_json::from_str(crate::quester::compile::ROMEO_AND_JULIET_JSON).unwrap();
        let path = compile_uncached_for_test(&document, &data, &quests).unwrap();
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_inventory(vec![], 28);
        for (stage, scan, acquire) in [
            ("romeojuliet:20", "scan-message-bank", "replace-message"),
            ("romeojuliet:50", "scan-potion-bank", "take-berries"),
        ] {
            let sequence = sequence_for_stage(&path, stage).unwrap();
            let mut bank = crate::quester::bank_memo::BankMemo::default();
            let mut ledger = None;
            let selected_id =
                |bank: &crate::quester::bank_memo::BankMemo,
                 ledger: &mut Option<Box<crate::native::ledger::Ledger>>| {
                    crate::quester::families::tests::with_tick(&snapshot, ledger, 1, |tick| {
                        let pred = PredicateContext { cx: &tick.cx, pairs: tick.pairs, quests: &quests,
                        progress: &[],
                        required_after: stamp(),
                        chat_since: 0,
                        outcome: None,
                        bank, };
                        let SelectionDecision::Selected(picked) = select(&path, sequence, &pred)
                        else {
                            panic!("observed inventory and bank state must select a step");
                        };
                        picked.step.id.0.to_string()
                    })
                };
            assert_eq!(selected_id(&bank, &mut ledger), scan);
            bank.update(&crate::native_bank::BankReceipt {
                counts: vec![],
                complete: true,
            });
            assert_eq!(selected_id(&bank, &mut ledger), acquire);
        }
    }

    #[test]
    fn rune_stage_one_selects_talisman_recovery_and_package_steps_in_order() {
        let data = selected();
        let quests = quests(&data);
        let compiled = path(RUNE_MYSTERIES_JSON, &data, &quests);
        let pending = progress(
            &compiled,
            &data,
            "runemysteries:1",
            &[
                ("runemysteries:talisman_pending", Truth::True, None),
                ("runemysteries:package_delivered", Truth::False, None),
            ],
        );
        let unknown_bank = crate::quester::bank_memo::BankMemo::default();
        let known_bank = known_empty_bank();

        let talisman = inventory_snapshot(&data, &[("air_talisman", 1)]);
        assert_eq!(
            choice_for_stage(
                &compiled,
                "runemysteries:1",
                &talisman,
                &quests,
                std::slice::from_ref(&pending),
                &unknown_bank,
            ),
            Choice::Step("deliver-talisman".into())
        );
        assert_eq!(
            choice_for_stage(
                &compiled,
                "runemysteries:1",
                &talisman,
                &quests,
                std::slice::from_ref(&pending),
                &known_bank,
            ),
            Choice::Step("deliver-talisman".into())
        );

        let empty = inventory_snapshot(&data, &[]);
        assert_eq!(
            choice_for_stage(
                &compiled,
                "runemysteries:1",
                &empty,
                &quests,
                std::slice::from_ref(&pending),
                &unknown_bank,
            ),
            Choice::Step("scan-bank".into()),
            "a bank scan remains available when no quest item is held"
        );
        assert_eq!(
            choice_for_stage(
                &compiled,
                "runemysteries:1",
                &empty,
                &quests,
                std::slice::from_ref(&pending),
                &known_bank,
            ),
            Choice::Step("recover-duke".into())
        );

        let package = inventory_snapshot(&data, &[("research_package", 1)]);
        let package_pending = progress(
            &compiled,
            &data,
            "runemysteries:1",
            &[
                ("runemysteries:talisman_pending", Truth::False, None),
                ("runemysteries:package_delivered", Truth::False, None),
            ],
        );
        assert_eq!(
            choice_for_stage(
                &compiled,
                "runemysteries:1",
                &package,
                &quests,
                std::slice::from_ref(&package_pending),
                &unknown_bank,
            ),
            Choice::Step("deliver-package".into())
        );

        let package_delivered = progress(
            &compiled,
            &data,
            "runemysteries:1",
            &[
                ("runemysteries:talisman_pending", Truth::False, None),
                ("runemysteries:package_delivered", Truth::True, None),
            ],
        );
        assert_eq!(
            choice_for_stage(
                &compiled,
                "runemysteries:1",
                &empty,
                &quests,
                std::slice::from_ref(&package_delivered),
                &known_bank,
            ),
            Choice::Step("collect-notes".into())
        );
        assert_eq!(
            choice_for_stage(
                &compiled,
                "runemysteries:1",
                &empty,
                &quests,
                std::slice::from_ref(&package_delivered),
                &unknown_bank,
            ),
            Choice::Step("collect-notes".into()),
            "after delivery, obtain Aubury's notes before generic bank recovery"
        );

        let notes = inventory_snapshot(&data, &[("research_notes", 1)]);
        assert_eq!(
            choice_for_stage(
                &compiled,
                "runemysteries:1",
                &notes,
                &quests,
                std::slice::from_ref(&package_delivered),
                &known_bank,
            ),
            Choice::Step("deliver-notes".into())
        );
    }

    #[test]
    fn sheep_partial_products_use_journal_remaining_count_and_keep_full_supply_recoverable() {
        let data = selected();
        let quests = quests(&data);
        let compiled = path(SHEEP_JSON, &data, &quests);
        let sequence_progress = progress(
            &compiled,
            &data,
            "sheep:1",
            &[("sheep:balls_to_go", Truth::True, Some(12))],
        );
        let progress = [sequence_progress];
        let bank = known_empty_bank();

        let partial = inventory_snapshot(
            &data,
            &[("shears", 1), ("bronze_sword", 1), ("ball_of_wool", 8)],
        );
        assert_eq!(
            choice_for_stage(
                &compiled,
                "sheep:1",
                &partial,
                &quests,
                &progress,
                &bank,
            ),
            Choice::Step("shear".into()),
            "the 12 remaining balls minus 8 held balls requires only 4 new wool; the bronze sword is not a quest equip"
        );

        let ready_to_spin = inventory_snapshot(
            &data,
            &[
                ("shears", 1),
                ("bronze_sword", 1),
                ("ball_of_wool", 8),
                ("wool", 4),
            ],
        );
        assert_eq!(
            choice_for_stage(
                &compiled,
                "sheep:1",
                &ready_to_spin,
                &quests,
                &progress,
                &bank,
            ),
            Choice::Step("spin".into())
        );

        let mixed_partial =
            inventory_snapshot(&data, &[("shears", 1), ("ball_of_wool", 6), ("wool", 10)]);
        assert_eq!(
            choice_for_stage(
                &compiled,
                "sheep:1",
                &mixed_partial,
                &quests,
                &progress,
                &bank,
            ),
            Choice::Step("spin".into()),
            "existing wool plus held balls already meets the remaining shear target"
        );

        let enough_balls = inventory_snapshot(&data, &[("shears", 1), ("ball_of_wool", 12)]);
        assert_eq!(
            choice_for_stage(
                &compiled,
                "sheep:1",
                &enough_balls,
                &quests,
                &progress,
                &bank,
            ),
            Choice::Step("hand-in".into())
        );

        let no_progress = inventory_snapshot(&data, &[("shears", 1), ("ball_of_wool", 8)]);
        assert_eq!(
            choice_for_stage(&compiled, "sheep:1", &no_progress, &quests, &[], &bank,),
            Choice::Unknown,
            "a partial inventory must wait for journal quantity evidence"
        );
        let full_supply = inventory_snapshot(&data, &[("shears", 1), ("ball_of_wool", 20)]);
        assert_eq!(
            choice_for_stage(&compiled, "sheep:1", &full_supply, &quests, &[], &bank,),
            Choice::Step("hand-in".into()),
            "the full hand-in quantity remains usable before the first journal read"
        );
    }

    #[test]
    fn nearest_reselects_completed_candidates_and_never_chooses_through_unknown_evidence() {
        use crate::native::ActionError;
        use crate::quester::compile::{PredicatePlan, StepContext, StepPlan, StepRun};
        struct Skip(Truth);
        impl PredicatePlan for Skip {
            fn evaluate(&self, _: &PredicateContext<'_, '_>) -> Truth { self.0 }
        }
        struct Anchor(api::WorldTile);
        impl StepPlan for Anchor {
            fn begin(&self, _: &mut StepContext<'_, '_>) -> Result<Box<dyn StepRun>, ActionError> {
                Err(ActionError::Unavailable(Arc::from("selector fixture does not dispatch")))
            }
            fn anchor(&self) -> Option<api::WorldTile> { Some(self.0) }
        }
        let step = |id: &str, x, z| CompiledStep {
            id: FactKey::new(id), kind: Arc::from("fixture"), comment: None,
            loadout: None, tactic: None, advances: false,
            skip_if: Arc::new(Skip(Truth::False)), settle: Arc::new(Skip(Truth::True)),
            plan: Arc::new(Anchor(api::WorldTile { x, z, level: 0 })),
        };
        let data = selected();
        let quests = quests(&data);
        let mut path = compile_uncached_for_test(&decode_cook().unwrap(), &data, &quests).unwrap();
        let sequence = &mut Arc::get_mut(&mut path).unwrap().sequences[0];
        sequence.order = crate::quester::path::SequenceOrder::Nearest;
        sequence.steps = vec![step("far", 20, 0), step("near", 1, 0), step("tie", 0, 1)];
        let mut snapshot = inventory_snapshot(&data, &[]);
        snapshot.seed_tile(api::WorldTile { x: 0, z: 0, level: 0 });
        let bank = known_empty_bank();
        assert_eq!(choice_for_stage(&path, "cook:0", &snapshot, &quests, &[], &bank), Choice::Step("near".into()));
        Arc::get_mut(&mut path).unwrap().sequences[0].steps[1].skip_if = Arc::new(Skip(Truth::True));
        assert_eq!(choice_for_stage(&path, "cook:0", &snapshot, &quests, &[], &bank), Choice::Step("tie".into()));
        snapshot.seed_tile(api::WorldTile { x: 100, z: 0, level: 0 });
        assert_eq!(choice_for_stage(&path, "cook:0", &snapshot, &quests, &[], &bank), Choice::Step("far".into()));
        Arc::get_mut(&mut path).unwrap().sequences[0].steps[2].skip_if = Arc::new(Skip(Truth::Unknown));
        assert_eq!(choice_for_stage(&path, "cook:0", &snapshot, &quests, &[], &bank), Choice::Unknown);
        let missing_here = inventory_snapshot(&data, &[]);
        assert_eq!(choice_for_stage(&path, "cook:0", &missing_here, &quests, &[], &bank), Choice::Unknown);
        for step in &mut Arc::get_mut(&mut path).unwrap().sequences[0].steps {
            step.skip_if = Arc::new(Skip(Truth::True));
        }
        assert_eq!(choice_for_stage(&path, "cook:0", &missing_here, &quests, &[], &bank), Choice::Exhausted);
        Arc::get_mut(&mut path).unwrap().prelude = vec![step("authored-prelude", 1000, 0)];
        assert_eq!(choice_for_stage(&path, "cook:0", &snapshot, &quests, &[], &bank), Choice::Step("authored-prelude".into()));
    }
}
