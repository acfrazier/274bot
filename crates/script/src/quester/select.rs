//! Ordered, tri-valued selection: only a proven `False` may start a step.
use super::compile::{CompiledPath, CompiledStep, PredicateContext};
use super::path::SequenceOrder;
use api::selected::Truth;

pub struct Selection<'a> {
    pub step: &'a CompiledStep,
    pub index: usize,
    pub prelude: bool,
}

pub enum SelectionDecision<'a> {
    Selected(Selection<'a>),
    Unknown(Selection<'a>),
    Exhausted,
}

/// Prelude first, then the current sequence. Unknown blocks lower priorities.
/// `cursor` is where an `Ordered` sequence resumes; other orders ignore it.
pub fn select<'a>(
    path: &'a CompiledPath,
    sequence: usize,
    cursor: usize,
    cx: &PredicateContext<'_, '_>,
) -> SelectionDecision<'a> {
    select_with_skips(path, sequence, cursor, cx, |_| {})
}

pub fn select_with_skips<'a>(
    path: &'a CompiledPath,
    sequence: usize,
    cursor: usize,
    cx: &PredicateContext<'_, '_>,
    mut on_skip: impl FnMut(&'a CompiledStep),
) -> SelectionDecision<'a> {
    for (index, step) in path.prelude.iter().enumerate() {
        match step.skip_if.evaluate(cx) {
            Truth::True => on_skip(step),
            Truth::Unknown => {
                return SelectionDecision::Unknown(Selection {
                    step,
                    index,
                    prelude: true,
                });
            }
            Truth::False => {
                return SelectionDecision::Selected(Selection {
                    step,
                    index,
                    prelude: true,
                });
            }
        }
    }
    let Some(sequence) = path.sequences.get(sequence) else {
        return SelectionDecision::Exhausted;
    };
    let nearest = sequence.order == SequenceOrder::Nearest;
    let first = match sequence.order {
        SequenceOrder::Ordered => cursor,
        SequenceOrder::Authored | SequenceOrder::Nearest => 0,
    };
    let here = nearest.then(|| cx.cx.snapshot().here()).flatten();
    let mut chosen = None;
    let mut best = (i64::MAX, i32::MAX);
    for (index, step) in sequence.steps.iter().enumerate().skip(first) {
        match step.skip_if.evaluate(cx) {
            Truth::True => on_skip(step),
            Truth::Unknown => {
                return SelectionDecision::Unknown(Selection {
                    step,
                    index,
                    prelude: false,
                });
            }
            Truth::False => {
                let candidate = Selection {
                    step,
                    index,
                    prelude: false,
                };
                if !nearest {
                    return SelectionDecision::Selected(candidate);
                }
                let (Some(here), Some(anchor)) = (here.as_ref(), step.plan.anchor()) else {
                    return SelectionDecision::Unknown(candidate);
                };
                let distance = (i64::from(here.value.x) - i64::from(anchor.x))
                    .abs()
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
    use crate::quester::families::tests::with_tick_bank;
    use crate::quester::path::{PathDocument, PredicateDocument};
    use crate::quester::probe::{known_empty_bank, progress_for_stage, Choice, Probe};
    use api::bank_memory::{BankMemory, Origin};
    use api::game_data::SelectedGameData;
    use api::obj_names::ItemDefView;
    use api::quest_facts::QuestCatalog;
    use api::quest_progress::{EvidenceStamp, QuestProgress};
    use api::selected::{ClientRevision, FactKey, RunKey, Truth};
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

    fn choice_for_stage(
        path: &CompiledPath,
        stage: &str,
        snapshot: &GameSnapshot,
        quests: &QuestCatalog,
        progress: &[QuestProgress],
        bank: &BankMemory,
    ) -> Choice {
        Probe {
            path,
            selected: &selected(),
            quests,
            progress,
            bank,
        }
        .choice(stage, 0, snapshot)
    }

    fn chosen(id: &str) -> Choice {
        Choice::Step(FactKey::new(id))
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
        let pred = PredicateContext {
            cx: &cx,
            pairs: None,
            quests: &quests,
            progress: &[],
            required_after: stamp(),
            chat_since: 0,
            outcome: None,
        };
        let SelectionDecision::Selected(picked) = select(&compiled, 0, 0, &pred) else {
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
            matches!(
                select(&compiled, 0, 0, &pred),
                SelectionDecision::Unknown(_)
            ),
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
        for (stage, _scan, acquire) in [
            ("romeojuliet:20", "scan-message-bank", "replace-message"),
            ("romeojuliet:50", "scan-potion-bank", "take-berries"),
        ] {
            let sequence = sequence_for_stage(&path, stage).unwrap();
            let mut ledger = None;
            let decide =
                |bank: &BankMemory, ledger: &mut Option<Box<crate::native::ledger::Ledger>>| {
                    with_tick_bank(&snapshot, Some(bank), ledger, 1, |tick| {
                        let pred = PredicateContext {
                            cx: &tick.cx,
                            pairs: tick.pairs,
                            quests: &quests,
                            progress: &[],
                            required_after: stamp(),
                            chat_since: 0,
                            outcome: None,
                        };
                        match select(&path, sequence, 0, &pred) {
                            SelectionDecision::Selected(picked) => {
                                Some(picked.step.id.0.to_string())
                            }
                            SelectionDecision::Unknown(_) => None,
                            SelectionDecision::Exhausted => {
                                panic!("romeo stage must resolve to a step")
                            }
                        }
                    })
                };
            assert_eq!(
                decide(&BankMemory::default(), &mut ledger),
                None,
                "an Unknown bank leaves the scan step undecided for the runner's provisioning scan"
            );
            assert_eq!(
                decide(&BankMemory::seeded(&[], Origin::Session), &mut ledger),
                Some(acquire.to_string())
            );
            assert_eq!(
                decide(&BankMemory::seeded(&[], Origin::Hint), &mut ledger),
                Some(acquire.to_string()),
                "a hint counts as known: the authored scan is skipped"
            );
        }
    }

    #[test]
    fn rune_stage_one_selects_talisman_recovery_and_package_steps_in_order() {
        let data = selected();
        let quests = quests(&data);
        let compiled = path(RUNE_MYSTERIES_JSON, &data, &quests);
        let pending = progress_for_stage(
            &compiled,
            &data,
            "runemysteries:1",
            &[
                ("runemysteries:talisman_pending", Truth::True, None),
                ("runemysteries:package_delivered", Truth::False, None),
            ],
        );
        let unknown_bank = BankMemory::default();
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
            chosen("deliver-talisman")
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
            chosen("deliver-talisman")
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
            Choice::Unknown(FactKey::new("scan-bank")),
            "an Unknown bank leaves the scan step undecided for the runner's provisioning scan"
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
            chosen("recover-duke")
        );

        let package = inventory_snapshot(&data, &[("research_package", 1)]);
        let package_pending = progress_for_stage(
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
            chosen("deliver-package")
        );

        let package_delivered = progress_for_stage(
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
            chosen("collect-notes")
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
            chosen("collect-notes"),
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
            chosen("deliver-notes")
        );
    }

    #[test]
    fn sheep_partial_products_use_journal_remaining_count_and_keep_full_supply_recoverable() {
        let data = selected();
        let quests = quests(&data);
        let compiled = path(SHEEP_JSON, &data, &quests);
        let sequence_progress = progress_for_stage(
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
            chosen("shear"),
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
            chosen("spin")
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
            chosen("spin"),
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
            chosen("hand-in")
        );

        let no_progress = inventory_snapshot(&data, &[("shears", 1), ("ball_of_wool", 8)]);
        assert!(
            matches!(
                choice_for_stage(&compiled, "sheep:1", &no_progress, &quests, &[], &bank),
                Choice::Unknown(_)
            ),
            "a partial inventory must wait for journal quantity evidence"
        );
        let full_supply = inventory_snapshot(&data, &[("shears", 1), ("ball_of_wool", 20)]);
        assert_eq!(
            choice_for_stage(&compiled, "sheep:1", &full_supply, &quests, &[], &bank,),
            chosen("hand-in"),
            "the full hand-in quantity remains usable before the first journal read"
        );
    }

    fn assert_nearest_approach_for_family(kind: &str, args: serde_json::Value) {
        let data = selected();
        let quests = quests(&data);
        let mut document: serde_json::Value =
            serde_json::from_str(crate::quester::compile::COOK_JSON).unwrap();
        document["roles"][0]["prelude"] = serde_json::json!([]);
        document["roles"][0]["sequences"][0]["order"] = serde_json::json!("nearest");
        document["roles"][0]["sequences"][0]["steps"] = serde_json::json!([
            ("far", [3227, 3300, 0], [3208, 3213, 0]),
            ("near", [3208, 3213, 0], [3227, 3300, 0]),
        ]
        .into_iter()
        .map(|(id, approach, exact_target)| {
            let mut args = args.clone();
            args["anchor"] = serde_json::json!({
                "tile": approach,
                "source": "nearest selection fixture"
            });
            if let Some(tile) = args.pointer_mut("/target/tile") {
                *tile = serde_json::json!(exact_target);
            }
            serde_json::json!({
                "id": id, "kind": kind, "version": 1, "args": args,
                "advances": false,
                "skip_if": {"Any": []}, "settle": {"All": []}
            })
        })
        .collect::<Vec<_>>());
        let document: PathDocument = serde_json::from_value(document).unwrap();
        let path = compile_uncached_for_test(&document, &data, &quests).unwrap();
        let mut snapshot = inventory_snapshot(&data, &[]);
        let bank = known_empty_bank();
        for (x, z, expected) in [(3208, 3213, "near"), (3227, 3300, "far")] {
            snapshot.seed_local_player(crate::quester::families::tests::local_player(
                api::WorldTile { x, z, level: 0 },
            ));
            assert_eq!(
                choice_for_stage(&path, "cook:0", &snapshot, &quests, &[], &bank),
                chosen(expected),
                "{kind} must select by its authored approach, not authored order or exact target"
            );
        }
    }

    #[test]
    fn nearest_uses_authored_approach_for_talk() {
        assert_nearest_approach_for_family("talk", serde_json::json!({"npc": "cook"}));
    }

    #[test]
    fn nearest_uses_authored_approach_for_interact() {
        assert_nearest_approach_for_family(
            "interact",
            serde_json::json!({
                "target": {
                    "loc": "priestperiltempledoorl",
                    "tile": [0, 0, 0],
                    "source": "nearest selection fixture"
                },
                "op": "Knock-at"
            }),
        );
    }

    #[test]
    fn nearest_uses_authored_approach_for_use_on() {
        assert_nearest_approach_for_family(
            "use_on",
            serde_json::json!({
                "item": "shears",
                "target": {
                    "ground": "wool",
                    "tile": [0, 0, 0],
                    "source": "nearest selection fixture"
                }
            }),
        );
    }

    #[test]
    fn nearest_reselects_completed_candidates_and_never_chooses_through_unknown_evidence() {
        use crate::native::ActionError;
        use crate::quester::compile::{PredicatePlan, StepContext, StepPlan, StepRun};
        struct Skip(Truth);
        impl PredicatePlan for Skip {
            fn evaluate(&self, _: &PredicateContext<'_, '_>) -> Truth {
                self.0
            }
        }
        struct Anchor(api::WorldTile);
        impl StepPlan for Anchor {
            fn begin(&self, _: &mut StepContext<'_, '_>) -> Result<Box<dyn StepRun>, ActionError> {
                Err(ActionError::Unavailable(Arc::from(
                    "selector fixture does not dispatch",
                )))
            }
            fn anchor(&self) -> Option<api::WorldTile> {
                Some(self.0)
            }
        }
        let step = |id: &str, x, z| CompiledStep {
            id: FactKey::new(id),
            kind: Arc::from("fixture"),
            comment: None,
            loadout: None,
            tactic: None,
            advances: false,
            skip_if: Arc::new(Skip(Truth::False)),
            skip_if_summary: Arc::from("never"),
            settle: Arc::new(Skip(Truth::True)),
            plan: Arc::new(Anchor(api::WorldTile { x, z, level: 0 })),
        };
        let data = selected();
        let quests = quests(&data);
        let mut path = compile_uncached_for_test(&decode_cook().unwrap(), &data, &quests).unwrap();
        let sequence = &mut Arc::get_mut(&mut path).unwrap().sequences[0];
        sequence.order = crate::quester::path::SequenceOrder::Nearest;
        sequence.steps = vec![step("far", 20, 0), step("near", 1, 0), step("tie", 0, 1)];
        let mut snapshot = inventory_snapshot(&data, &[]);
        snapshot.seed_local_player(crate::quester::families::tests::local_player(
            api::WorldTile {
                x: 0,
                z: 0,
                level: 0,
            },
        ));
        let bank = known_empty_bank();
        assert_eq!(
            choice_for_stage(&path, "cook:0", &snapshot, &quests, &[], &bank),
            chosen("near")
        );
        Arc::get_mut(&mut path).unwrap().sequences[0].steps[1].skip_if =
            Arc::new(Skip(Truth::True));
        assert_eq!(
            choice_for_stage(&path, "cook:0", &snapshot, &quests, &[], &bank),
            chosen("tie")
        );
        snapshot.seed_local_player(crate::quester::families::tests::local_player(
            api::WorldTile {
                x: 100,
                z: 0,
                level: 0,
            },
        ));
        assert_eq!(
            choice_for_stage(&path, "cook:0", &snapshot, &quests, &[], &bank),
            chosen("far")
        );
        Arc::get_mut(&mut path).unwrap().sequences[0].steps[2].skip_if =
            Arc::new(Skip(Truth::Unknown));
        assert_eq!(
            choice_for_stage(&path, "cook:0", &snapshot, &quests, &[], &bank),
            Choice::Unknown(FactKey::new("tie"))
        );
        let missing_here = inventory_snapshot(&data, &[]);
        assert_eq!(
            choice_for_stage(&path, "cook:0", &missing_here, &quests, &[], &bank),
            Choice::Unknown(FactKey::new("far"))
        );
        for step in &mut Arc::get_mut(&mut path).unwrap().sequences[0].steps {
            step.skip_if = Arc::new(Skip(Truth::True));
        }
        assert_eq!(
            choice_for_stage(&path, "cook:0", &missing_here, &quests, &[], &bank),
            Choice::Exhausted
        );
        Arc::get_mut(&mut path).unwrap().prelude = vec![step("authored-prelude", 1000, 0)];
        assert_eq!(
            choice_for_stage(&path, "cook:0", &snapshot, &quests, &[], &bank),
            chosen("authored-prelude")
        );
    }

    /// Five indistinguishable wait steps; `third_skip` replaces step 3's skip.
    fn five_step_path(order: &str, third_skip: serde_json::Value) -> Arc<CompiledPath> {
        let data = selected();
        let quests = quests(&data);
        let mut document: serde_json::Value =
            serde_json::from_str(crate::quester::compile::COOK_JSON).unwrap();
        document["roles"][0]["prelude"] = serde_json::json!([]);
        document["roles"][0]["sequences"][0]["order"] = serde_json::json!(order);
        document["roles"][0]["sequences"][0]["steps"] = serde_json::json!((1..=5)
            .map(|n| serde_json::json!({
                "id": format!("step-{n}"), "kind": "wait", "version": 1,
                "args": {"until": {"All": []}, "max_ticks": 10},
                "advances": false,
                "skip_if": if n == 3 { third_skip.clone() } else { serde_json::json!({"Any": []}) },
                "settle": {"All": []}
            }))
            .collect::<Vec<_>>());
        let document: PathDocument = serde_json::from_value(document).unwrap();
        compile_uncached_for_test(&document, &data, &quests).unwrap()
    }

    #[test]
    fn ordered_sequence_selects_from_the_cursor_and_jumps_proven_skips() {
        let data = selected();
        let quests = quests(&data);
        let bank = known_empty_bank();
        let empty = inventory_snapshot(&data, &[]);
        let path = five_step_path("ordered", serde_json::json!({"Any": []}));
        for (cursor, expected) in [(0, "step-1"), (1, "step-2"), (2, "step-3"), (4, "step-5")] {
            assert_eq!(
                Probe {
                    path: &path,
                    selected: &data,
                    quests: &quests,
                    progress: &[],
                    bank: &bank,
                }
                .choice("cook:0", cursor, &empty),
                chosen(expected),
                "cursor {cursor} resumes at its own step, never at step 1"
            );
        }
        assert_eq!(
            Probe {
                path: &path,
                selected: &data,
                quests: &quests,
                progress: &[],
                bank: &bank,
            }
            .choice("cook:0", 5, &empty),
            Choice::Exhausted,
            "a cursor past the last step is exhausted, not wrapped"
        );

        let egg_skip =
            serde_json::json!({"Fact": {"kind": "has_item", "version": 1, "args": {"obj": "egg"}}});
        let path = five_step_path("ordered", egg_skip);
        let egg = inventory_snapshot(&data, &[("egg", 1)]);
        let probe = Probe {
            path: &path,
            selected: &data,
            quests: &quests,
            progress: &[],
            bank: &bank,
        };
        assert_eq!(probe.choice("cook:0", 2, &egg), chosen("step-4"));
        assert_eq!(probe.choice("cook:0", 2, &empty), chosen("step-3"));
    }

    #[test]
    fn authored_sequence_ignores_the_cursor() {
        let data = selected();
        let quests = quests(&data);
        let bank = known_empty_bank();
        let empty = inventory_snapshot(&data, &[]);
        let path = five_step_path("authored", serde_json::json!({"Any": []}));
        let probe = Probe {
            path: &path,
            selected: &data,
            quests: &quests,
            progress: &[],
            bank: &bank,
        };
        for cursor in [0, 3, 5] {
            assert_eq!(probe.choice("cook:0", cursor, &empty), chosen("step-1"));
        }
    }

    /// PORT-S6-DRAFTS-R5 D1: the combination-door prelude steps are
    /// side-aware. The stateless prelude selector re-runs from index 0 on
    /// every selection, so a door step whose skip holds on both sides of the
    /// door is re-selected after crossing and oscillates: the R4 step skipped
    /// on the mansion-wide area (true in the hall too) with a vacuous settle,
    /// so `return-totem` was only reached at x >= 2644. This runs the real
    /// selector on the compiled `totem.json`: the stair-room step must fire
    /// only west of `combodoor`, the hall step only east of it before the
    /// totem is held, and `return-totem` from every hall tile. Fails on the
    /// R4 document (hall tiles select the stair-room door step there, and
    /// the hall step does not exist).
    #[test]
    fn totem_combo_door_steps_are_side_aware() {
        let data = selected();
        let quests = quests(&data);
        let bank = known_empty_bank();
        let json = include_str!("../../paths/289/totem.json");
        let compiled = path(json, &data, &quests);
        let snapshot_at = |x, z, level, items: &[(&str, i32)]| {
            let mut snapshot = inventory_snapshot(&data, items);
            let tile = api::WorldTile { x, z, level };
            snapshot.seed_tile(tile);
            snapshot.seed_local_player(crate::quester::families::tests::local_player(tile));
            snapshot
        };
        let totem = [("tribal_totem", 1)];
        // With the totem held (post search-chest, combination solved): down
        // the stairs on level 1, through the door in the stair room, and
        // straight home from every hall tile, including the door tile itself.
        for (name, x, z, level, expected) in [
            ("l1-chest-room", 2637, 3323, 1, "climb-down-stairs"),
            (
                "stair-landing",
                2631,
                3325,
                0,
                "open-combo-door-from-stair-room",
            ),
            (
                "stair-room-w33",
                2633,
                3323,
                0,
                "open-combo-door-from-stair-room",
            ),
            (
                "stair-room-w27",
                2627,
                3324,
                0,
                "open-combo-door-from-stair-room",
            ),
            ("hall-e34-door-tile", 2634, 3323, 0, "return-totem"),
            ("hall-e35", 2635, 3323, 0, "return-totem"),
            ("hall-teleport", 2638, 3321, 0, "return-totem"),
            ("hall-e40", 2640, 3322, 0, "return-totem"),
            ("hall-e43", 2643, 3322, 0, "return-totem"),
            ("outside-box-e46", 2646, 3322, 0, "return-totem"),
        ] {
            let progress = [progress_for_stage(
                &compiled,
                &data,
                "totem:4",
                &[("combo", Truth::True, None)],
            )];
            let probe = Probe {
                path: &compiled,
                selected: &data,
                quests: &quests,
                progress: &progress,
                bank: &bank,
            };
            assert_eq!(
                probe.choice("totem:4", 0, &snapshot_at(x, z, level, &totem)),
                chosen(expected),
                "{name} ({x},{z},{level}) with the totem must select {expected}"
            );
        }
        // Without the totem (stage-4 ascent): solve first, then through the
        // door from the hall side, then the stairs sequence takes over.
        for (name, x, z, combo, expected) in [
            (
                "hall-pre-combo",
                2638,
                3321,
                Truth::False,
                "solve-combination",
            ),
            (
                "hall-post-combo",
                2638,
                3321,
                Truth::True,
                "open-combo-door-from-hall",
            ),
            (
                "hall-e35-post-combo",
                2635,
                3323,
                Truth::True,
                "open-combo-door-from-hall",
            ),
            (
                "stair-room-post-combo",
                2631,
                3325,
                Truth::True,
                "disarm-trap",
            ),
        ] {
            let progress = [progress_for_stage(
                &compiled,
                &data,
                "totem:4",
                &[("combo", combo, None)],
            )];
            let probe = Probe {
                path: &compiled,
                selected: &data,
                quests: &quests,
                progress: &progress,
                bank: &bank,
            };
            assert_eq!(
                probe.choice("totem:4", 0, &snapshot_at(x, z, 0, &[])),
                chosen(expected),
                "{name} ({x},{z},0) without the totem must select {expected}"
            );
        }
    }
}
