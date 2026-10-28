use super::*;
use crate::native::{WalkCancelReason, WalkEnd};
use crate::quester::compile::{
    compile_uncached_for_test, decode_args, CompileContext, PredicateContext, StepOutcome,
};
use crate::quester::loadouts::LoadoutOverlay;
use crate::quester::progress::CompiledProgress;
use api::game_data::SelectedGameData;
use api::quest_facts::QuestCatalog;
use api::quest_progress::EvidenceStamp;
use api::selected::{ClientRevision, FactKey, RunKey, Truth};
use api::snapshot::{
    ChatLineView, GameSnapshot, GroundItemView, LocLayer, LocView, QuestListStatus,
    QuestStatusView, SnapshotView,
};
use std::collections::HashMap;
use std::sync::Arc;

fn report(end: CombatEnd) -> CombatReport {
    CombatReport {
        end,
        evidence: EvidenceStamp {
            run: RunKey {
                slot: 1,
                run: 1,
                session: 1,
            },
            tick: 12,
            sequence: 5,
        },
        engaged: None,
        engaged_npc_type: -1,
        ticks: 1,
        swings: 0,
        casts: 0,
        damage_taken: 0,
        food: 0,
        prayer_doses: 0,
        boost_doses: 0,
        antifire_doses: 0,
        hits_while_protected: 0,
        protect_switches: 0,
        intruders: 0,
        ammo_pickups: 0,
        restorations: 0,
        locked_ticks: 0,
        multi_op_plans: 0,
        melee_mode_fallback: None,
        flick_resets: 0,
        flick_misses: 0,
        flick_fallback: false,
    }
}

#[test]
fn open_tactic_defaults_auto_retaliate_on() {
    let args: TacticArgs = serde_json::from_value(serde_json::json!({
        "kind": "open",
        "style": "melee",
        "engage_radius": 6
    }))
    .unwrap();
    assert!(args.auto_retaliate);

    let args: TacticArgs = serde_json::from_value(serde_json::json!({
        "kind": "open",
        "style": "melee",
        "engage_radius": 6,
        "auto_retaliate": false
    }))
    .unwrap();
    assert!(!args.auto_retaliate);
}

#[test]
fn ranged_tactic_maps_each_ranged_style_and_defaults_to_rapid() {
    let tactic = |ranged_style: Option<&str>| {
        let mut value = serde_json::json!({
            "kind": "open",
            "style": "ranged",
            "engage_radius": 6
        });
        if let Some(ranged_style) = ranged_style {
            value["ranged_style"] = ranged_style.into();
        }
        serde_json::from_value::<TacticArgs>(value)
            .unwrap()
            .ranged_style
    };

    assert_eq!(tactic(None), RangedMode::Rapid);
    assert_eq!(tactic(Some("rapid")), RangedMode::Rapid);
    assert_eq!(tactic(Some("accurate")), RangedMode::Accurate);
    assert_eq!(tactic(Some("long_range")), RangedMode::LongRange);
}

#[test]
fn ranged_tactic_rejects_a_melee_mode() {
    assert!(validate_style_options(Style::Ranged, Some(MeleeMode::Accurate)).is_err());
    assert!(validate_style_options(Style::Melee, Some(MeleeMode::Accurate)).is_ok());
}

#[test]
fn mage_family_maps_selected_spell_order_and_explicit_fallback() {
    let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let quests = QuestCatalog::from_identity(data.quest_identity()).unwrap();
    let progress = CompiledProgress {
        binding: FactKey::new("journal:mage_mapper"),
        role: None,
        colour_not_started: FactKey::new("mage_mapper:0"),
        colour_in_progress: FactKey::new("mage_mapper:1"),
        colour_complete: FactKey::new("mage_mapper:2"),
        stage_keys: Arc::from([]),
        rules: Arc::from([]),
        flags: Arc::from([]),
        monotonic: false,
    };
    let areas = HashMap::new();
    let loadouts = LoadoutOverlay::new(Arc::from([]), Arc::from([]));
    let recipes = HashMap::new();
    let path = FactKey::new("mage_mapper");
    let cx = fixture_compile_context(
        &data, &quests, &progress, &areas, &loadouts, &recipes, &path,
    );
    let mut args = serde_json::json!({
        "target": {"npc": "delrith", "pick": "nearest", "not_targeting_others": true},
        "tactic": {"kind": "open", "style": "mage", "engage_radius": 6},
        "lost_radius": 12,
        "kill_budget_ticks": 100,
        "spells": ["fire_bolt", "Wind Strike"],
        "fallback_spells": true,
        "cross": ["draynor-jail-guards"],
        "guard": "protect",
        "finish": {"npc": "delrith_weakened", "prefer": ["Gabindo"], "max_ticks": 100},
        "loot": [{"obj": "bones", "qty": 25}],
        "until": {"Any": []},
        "win": {"All": []}
    });
    compile(serde_json::from_value(args.clone()).unwrap(), &cx).unwrap();
    let plan = compile_plan(serde_json::from_value(args.clone()).unwrap(), &cx).unwrap();
    assert_eq!(plan.request.style, Style::Mage);
    assert!(plan.request.fallback_spells);
    let order = plan.request.spells.as_ref().unwrap();
    assert_eq!(order.len(), 2);
    assert_eq!(order[0].alias.as_ref(), "magic_spell_fire_bolt");
    assert_eq!(order[1].alias.as_ref(), "magic_spell_wind_strike");
    assert_eq!(plan.cross.as_ref(), &[Arc::from("draynor-jail-guards")]);
    assert!(plan.protect);
    assert!(plan.until.is_some());
    assert!(plan.win.is_some());
    let finish = plan.finish.as_ref().unwrap();
    assert_eq!(
        finish.npc_type,
        data.npc_by_config("delrith_weakened").unwrap().id
    );
    assert_eq!(finish.options.prefer[0].as_ref(), "Gabindo");
    assert_eq!(finish.max_ticks, 100);
    assert_eq!(plan.loot.len(), 1);
    assert_eq!(plan.loot[0].id, data.item_by_alias("bones").unwrap().id);
    assert_eq!(plan.loot[0].qty, 25);

    args["spells"] = serde_json::Value::Null;
    args.as_object_mut().unwrap().remove("fallback_spells");
    let automatic = compile_plan(serde_json::from_value(args.clone()).unwrap(), &cx).unwrap();
    assert!(automatic.request.spells.is_none());
    assert!(!automatic.request.fallback_spells);
    args["spells"] = serde_json::json!([]);
    assert_eq!(
        compile(serde_json::from_value(args.clone()).unwrap(), &cx)
            .err()
            .unwrap()
            .code
            .as_ref(),
        "invalid-combat-spells"
    );
    args["spells"] = serde_json::json!(["not_a_selected_spell"]);
    assert_eq!(
        compile(serde_json::from_value(args.clone()).unwrap(), &cx)
            .err()
            .unwrap()
            .code
            .as_ref(),
        "unresolved-combat-spell"
    );
    args["spells"] = serde_json::json!(["fire_bolt"]);
    args["tactic"]["style"] = serde_json::json!("melee");
    assert_eq!(
        compile(serde_json::from_value(args).unwrap(), &cx)
            .err()
            .unwrap()
            .code
            .as_ref(),
        "unsupported-combat-spells"
    );
}

#[test]
fn authored_combat_walk_permissions_reach_return_and_abort_requests() {
    let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let quests = QuestCatalog::from_identity(data.quest_identity()).unwrap();
    let progress = CompiledProgress {
        binding: FactKey::new("journal:combat_walk_policy"),
        role: None,
        colour_not_started: FactKey::new("combat_walk_policy:0"),
        colour_in_progress: FactKey::new("combat_walk_policy:1"),
        colour_complete: FactKey::new("combat_walk_policy:2"),
        stage_keys: Arc::from([]),
        rules: Arc::from([]),
        flags: Arc::from([]),
        monotonic: false,
    };
    let areas = HashMap::new();
    let loadouts = LoadoutOverlay::new(Arc::from([]), Arc::from([]));
    let recipes = HashMap::new();
    let path = FactKey::new("combat_walk_policy");
    let cx = CompileContext {
        path: &path,
        kind: crate::quester::path::PathKind::Quest,
        pair: None,
        progress: &progress,
        selected: &data,
        quests: &quests,
        gathering: None,
        bank: None,
        bank_required: false,
        keep_ids: &[],
        areas: &areas,
        loadouts: &loadouts,
        loadout_carry: crate::quester::compile::empty_loadout_carry(),
        recipes: &recipes,
    };
    let stand = api::WorldTile {
        x: 3125,
        z: 3246,
        level: 0,
    };
    let args = serde_json::json!({
        "target": {"npc": "jailguard", "pick": "nearest", "not_targeting_others": true},
        "tactic": {"kind": "open", "style": "melee", "engage_radius": 12},
        "stand": {"tile": [3125, 3246, 0], "source": "native Prince live return-walk regression"},
        "lost_radius": 16,
        "kill_budget_ticks": 400,
        "until": {"Any": []},
        "cross": ["draynor-jail-guards"],
        "guard": "protect"
    });
    for cross in [false, true] {
        for guard in [
            None,
            Some(serde_json::Value::Null),
            Some(serde_json::json!("")),
            Some(serde_json::json!("protect")),
        ] {
            let protect = guard
                .as_ref()
                .is_some_and(|value| value.as_str() == Some("protect"));
            let mut input = args.clone();
            if !cross {
                input.as_object_mut().unwrap().remove("cross");
            }
            if let Some(guard) = guard {
                input["guard"] = guard;
            } else {
                input.as_object_mut().unwrap().remove("guard");
            }
            let plan = compile(decode_args::<CombatArgs>(&input).unwrap(), &cx).unwrap();
            for aborted in [false, true] {
                // Drive the real compiled plan and native combat machine, not
                // a CombatRun with its permission fields assigned by the test.
                let mut snapshot = GameSnapshot::new();
                snapshot.seed_ingame(2);
                snapshot.seed_world(api::snapshot::WorldStateView::default());
                snapshot.seed_local_player(super::super::tests::local_player(stand));
                snapshot.seed_inventory(vec![], 28);
                snapshot.seed_equipment(vec![]);
                snapshot.seed_players(vec![]);
                snapshot.seed_projectiles(vec![]);
                snapshot.seed_chat_lines(vec![]);
                snapshot.seed_hitmarks(api::snapshot::HitmarksView {
                    marks: [api::snapshot::HitmarkView {
                        value: 0,
                        kind: 0,
                        cycle: 0,
                    }; 4],
                    loop_cycle: 0,
                });
                snapshot.seed_varps(
                    (0..api::prayer::PRAYER_COUNT)
                        .map(|index| api::snapshot::VarpView {
                            index: api::prayer::PRAYER_VARP0 + index as i32,
                            value: 0,
                        })
                        .chain([api::snapshot::VarpView {
                            index: crate::combat::OPTION_NODEF,
                            value: 0,
                        }])
                        .collect(),
                );
                snapshot.seed_stats(
                    (0..25)
                        .map(|index| api::snapshot::StatView {
                            index,
                            name: String::new(),
                            effective: if index == 5 {
                                43
                            } else if aborted && index == 3 {
                                1
                            } else {
                                40
                            },
                            base: if index == 5 { 43 } else { 40 },
                            xp: 0,
                            used: api::snapshot::stat_used(index as usize),
                        })
                        .collect(),
                );
                let row = data.npc_by_config("jailguard").unwrap();
                let at = api::WorldTile {
                    x: stand.x + 1,
                    ..stand
                };
                snapshot.seed_npcs(vec![api::snapshot::NpcView {
                    index: 7,
                    r#type: Some(row.id as usize),
                    name: row.display.clone(),
                    actions: vec![Some("Attack".into())],
                    tile: at,
                    distance: 1,
                    animation: -1,
                    animation_frame: 0,
                    pose_animation: -1,
                    orientation: 0,
                    target_orientation: 0,
                    overhead_text: None,
                    spot_animation: -1,
                    spot_animation_stamp: -1,
                    health: 10,
                    total_health: 10,
                    face_entity: -1,
                    target: aborted.then_some(api::snapshot::ActorTargetView {
                        kind: api::snapshot::ActorKind::Player,
                        index: 0,
                    }),
                    moving: false,
                    running: false,
                    in_combat: aborted,
                    level: 1,
                    size: 1,
                    network: at,
                    x: 0,
                    z: 0,
                    yaw: 0,
                }]);
                let mut ledger = None;
                let mut run =
                    with_step_context(&snapshot, &mut ledger, 1, |cx| plan.begin(cx).unwrap());
                if !aborted {
                    snapshot.seed_npcs(vec![]);
                }
                let expected = if aborted {
                    CombatEnd::Aborted(AbortReason::Unprotected(crate::combat::Unprotected::NoFood))
                } else {
                    CombatEnd::TargetGone
                };
                // TargetGone has a three-tick disappearance grace. The
                // low-HP, empty-inventory attacker fixture aborts immediately.
                for tick in 2..=if aborted { 2 } else { 5 } {
                    assert!(
                        with_step_context(&snapshot, &mut ledger, tick, |cx| run.poll(cx))
                            .is_pending()
                    );
                }
                let receipt = run
                    .in_flight_outcome()
                    .and_then(|outcome| outcome.receipt.as_ref())
                    .and_then(|receipt| receipt.as_any().downcast_ref::<CombatReceipt>())
                    .expect("native combat must publish its report before the owned walk");
                assert_eq!(receipt.report.end, expected);
                let outbox = &ledger.as_ref().unwrap().outbox;
                assert_eq!(
                    outbox.len(),
                    1,
                    "{expected:?}: cross={cross}, protect={protect}"
                );
                let crate::native::HostEffect::Walk(request) = &outbox[0].effect else {
                    panic!("the compiled combat transition must submit a native walk");
                };
                assert_eq!(request.protect, protect, "{expected:?}: cross={cross}");
                assert_eq!(
                    request.food_guard,
                    aborted && !protect,
                    "only an unprotected abort walk requests the food-only guard"
                );
                if aborted {
                    assert_eq!(request.allow.prayer, protect);
                    assert!(request.allow.food);
                }
                if cross {
                    assert_eq!(request.cross.as_ref(), &[Arc::from("draynor-jail-guards")]);
                } else {
                    assert!(request.cross.is_empty(), "{expected:?}: protect={protect}");
                }
                if !aborted {
                    assert_eq!(request.target, stand);
                }
                if aborted {
                    // The attacker stands beside the stand, so the stand is
                    // inside its chase envelope and the walk names goal tiers
                    // beyond it.
                    assert_eq!(request.arrival, nav_planner::contract::ArrivalKind::Area);
                    assert_eq!(request.radius, 0);
                    assert!(request.candidate_goals.is_some());
                } else {
                    assert_eq!(request.radius, 1);
                }
            }
        }
    }
    let mut invalid = args;
    invalid["guard"] = serde_json::json!("off");
    assert_eq!(
        compile(decode_args::<CombatArgs>(&invalid).unwrap(), &cx)
            .err()
            .unwrap()
            .code
            .as_ref(),
        "invalid-args"
    );
}

#[test]
fn combat_finish_and_mixed_loot_compile_selected_configs_and_quantities() {
    let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let quests = QuestCatalog::from_identity(data.quest_identity()).unwrap();
    let progress = CompiledProgress {
        binding: FactKey::new("journal:combat_finish"),
        role: None,
        colour_not_started: FactKey::new("combat_finish:0"),
        colour_in_progress: FactKey::new("combat_finish:1"),
        colour_complete: FactKey::new("combat_finish:2"),
        stage_keys: Arc::from([]),
        rules: Arc::from([]),
        flags: Arc::from([]),
        monotonic: false,
    };
    let areas = HashMap::new();
    let loadouts = LoadoutOverlay::new(Arc::from([]), Arc::from([]));
    let recipes = HashMap::new();
    let path = FactKey::new("combat_finish_loot");
    let cx = fixture_compile_context(
        &data, &quests, &progress, &areas, &loadouts, &recipes, &path,
    );
    let prefer = "Carlem Aber Camerinthum Purchai Gabindo";
    let args = serde_json::json!({
        "target": {"npc": "delrith", "pick": "nearest", "not_targeting_others": true},
        "tactic": {"kind": "open", "style": "melee", "engage_radius": 6},
        "lost_radius": 12,
        "kill_budget_ticks": 100,
        "finish": {
            "npc": "delrith_weakened",
            "prefer": [prefer],
            "choose": 4,
            "line_rules": [{
                "when_line": "Choose the incantation",
                "choose": "Option two"
            }],
            "strict": true,
            "max_ticks": 100
        },
        "loot": ["white_bead", {"obj": "bones", "qty": 25}]
    });

    compile_json(&args, &cx).unwrap();
    let parsed: CombatArgs = decode_args(&args).unwrap();
    let target = compile_target(parsed.target, &cx).unwrap();
    let finish = compile_finish(parsed.finish.unwrap(), &target, &cx).unwrap();
    let loot = compile_loot(&parsed.loot, &cx).unwrap();

    let weakened = data.npc_by_config("delrith_weakened").unwrap();
    assert_eq!(finish.npc_type, weakened.id);
    assert_eq!(
        finish.npc_name.as_ref(),
        weakened.display.as_deref().unwrap()
    );
    assert_eq!(finish.options.prefer[0].as_ref(), prefer);
    assert_eq!(finish.options.choose, Some(4));
    assert_eq!(
        finish.options.line_rules[0].when_line.as_ref(),
        "Choose the incantation"
    );
    assert_eq!(finish.options.line_rules[0].choose.as_ref(), "Option two");
    assert!(finish.options.strict);
    assert_eq!(finish.max_ticks, 100);

    let white_bead = data.item_by_alias("white_bead").unwrap();
    let bones = data.item_by_alias("bones").unwrap();
    assert_eq!(loot.len(), 2);
    assert_eq!((loot[0].id, loot[0].qty), (white_bead.id, 1));
    assert_eq!(loot[0].name.as_ref(), white_bead.name.as_deref().unwrap());
    assert_eq!((loot[1].id, loot[1].qty), (bones.id, 25));
    assert_eq!(loot[1].name.as_ref(), bones.name.as_deref().unwrap());

    for bad_qty in [serde_json::json!(0), serde_json::json!(-1)] {
        let mut invalid = args.clone();
        invalid["loot"] = serde_json::json!([{"obj": "bones", "qty": bad_qty}]);
        assert!(compile_json(&invalid, &cx).is_err());
    }
    for bad_choose in [serde_json::json!(0), serde_json::json!(-1)] {
        let mut invalid = args.clone();
        invalid["finish"]["choose"] = bad_choose;
        assert!(compile_json(&invalid, &cx).is_err());
    }
    for bad_rule in [
        serde_json::json!([{"when_line": "", "choose": "Option two"}]),
        serde_json::json!([{"when_line": "Choose the incantation", "choose": " "}]),
    ] {
        let mut invalid = args.clone();
        invalid["finish"]["line_rules"] = bad_rule;
        assert!(compile_json(&invalid, &cx).is_err());
    }
    let mut invalid = args;
    invalid["finish"]["max_ticks"] = serde_json::json!(0);
    assert!(compile_json(&invalid, &cx).is_err());
}
fn fixture_compile_context<'a>(
    data: &'a SelectedGameData,
    quests: &'a QuestCatalog,
    progress: &'a CompiledProgress,
    areas: &'a HashMap<String, Vec<[i32; 5]>>,
    loadouts: &'a LoadoutOverlay,
    recipes: &'a HashMap<String, Arc<[crate::quester::families::CompiledAcquireStep]>>,
    path: &'a FactKey,
) -> CompileContext<'a> {
    CompileContext {
        path,
        kind: crate::quester::path::PathKind::Quest,
        pair: None,
        progress,
        selected: data,
        quests,
        gathering: None,
        bank: None,
        bank_required: false,
        keep_ids: &[],
        areas,
        loadouts,
        loadout_carry: crate::quester::compile::empty_loadout_carry(),
        recipes,
    }
}

fn compile_json(
    args: &serde_json::Value,
    cx: &CompileContext<'_>,
) -> Result<Arc<dyn crate::quester::compile::StepPlan>, crate::quester::compile::CompileError> {
    let args = decode_args::<CombatArgs>(args)?;
    compile(args, cx)
}

#[test]
fn imp_and_melee_paths_resolve_observed_quest_colour_on_r289() {
    let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let quests = QuestCatalog::from_identity(data.quest_identity()).unwrap();
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    snapshot.seed_quest_statuses(
        vec![QuestStatusView {
            name: "Imp Catcher".into(),
            component_id: 0,
            colour: 0xf80000,
        }],
        true,
    );
    let view = SnapshotView::new(
        Some(&snapshot),
        EvidenceStamp {
            run: RunKey {
                slot: 1,
                run: 1,
                session: 1,
            },
            tick: 1,
            sequence: 1,
        },
    );
    for bytes in [
        &include_bytes!("../../../paths/289/imp.json")[..],
        &include_bytes!("../../../paths/289/fixtures/combat_melee_upkeep.json")[..],
        &include_bytes!("../../../paths/289/fixtures/combat_melee_food_only.json")[..],
        &include_bytes!("../../../paths/289/fixtures/combat_unattackable.json")[..],
    ] {
        let path =
            compile_uncached_for_test(&serde_json::from_slice(bytes).unwrap(), &data, &quests)
                .unwrap();
        assert_eq!(
            crate::quester::progress::quest_colour(&path, &quests, view),
            Some(QuestListStatus::NotStarted),
        );
    }
}

#[test]
fn combat_end_predicate_distinguishes_unattackable_report() {
    let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let quests = QuestCatalog::from_identity(data.quest_identity()).unwrap();
    let progress = CompiledProgress {
        binding: FactKey::new("journal:imp"),
        role: None,
        colour_not_started: FactKey::new("imp:0"),
        colour_in_progress: FactKey::new("imp:1"),
        colour_complete: FactKey::new("imp:2"),
        stage_keys: Arc::from([]),
        rules: Arc::from([]),
        flags: Arc::from([]),
        monotonic: false,
    };
    let areas = HashMap::new();
    let loadouts = LoadoutOverlay::new(Arc::from([]), Arc::from([]));
    let recipes = HashMap::new();
    let path = FactKey::new("combat_unattackable");
    let cx = fixture_compile_context(
        &data, &quests, &progress, &areas, &loadouts, &recipes, &path,
    );
    let args = serde_json::from_value::<CombatEndArgs>(
        serde_json::json!({ "end": "aborted_unattackable" }),
    )
    .unwrap();
    let predicate = compile_end_predicate(args, &cx).unwrap();
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    let mut ledger = None;
    let aborted = report(CombatEnd::Aborted(AbortReason::Unattackable));
    let outcome = StepOutcome {
        progress: None,
        evidence: aborted.evidence,
        receipt: Some(Arc::new(CombatReceipt {
            report: aborted,
            target_gone_restarts: 0,
        })),
    };
    super::super::tests::with_tick(&snapshot, &mut ledger, 12, |tick| {
        let context = PredicateContext {
            cx: &tick.cx,
            pairs: tick.pairs,
            quests: &quests,
            progress: &[],
            required_after: tick.cx.evidence(),
            chat_since: 0,
            outcome: Some(&outcome),
        };
        assert_eq!(predicate.evaluate(&context), Truth::True);

        let killed = report(CombatEnd::Killed);
        let other = StepOutcome {
            progress: None,
            evidence: killed.evidence,
            receipt: Some(Arc::new(CombatReceipt {
                report: killed,
                target_gone_restarts: 0,
            })),
        };
        let context = PredicateContext {
            cx: &tick.cx,
            pairs: tick.pairs,
            quests: &quests,
            progress: &[],
            required_after: tick.cx.evidence(),
            chat_since: 0,
            outcome: Some(&other),
        };
        assert_eq!(predicate.evaluate(&context), Truth::False);
        let context = PredicateContext {
            cx: &tick.cx,
            pairs: tick.pairs,
            quests: &quests,
            progress: &[],
            required_after: tick.cx.evidence(),
            chat_since: 0,
            outcome: None,
        };
        assert_eq!(predicate.evaluate(&context), Truth::False);
    });
}

#[test]
fn unattackable_approach_is_not_reselected_after_observed_arrival() {
    let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let quests = QuestCatalog::from_identity(data.quest_identity()).unwrap();
    let path = compile_uncached_for_test(
        &serde_json::from_slice(include_bytes!(
            "../../../paths/289/fixtures/combat_unattackable.json"
        ))
        .unwrap(),
        &data,
        &quests,
    )
    .unwrap();
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    snapshot.seed_players(Vec::new());
    super::super::tests::seed_dialogue_combat(&mut snapshot, false);
    let mut local = snapshot.local_player().unwrap().clone();
    let mut ledger = None;
    for (x, expected_kind) in [(3110, "walk"), (3109, "combat")] {
        let here = api::WorldTile {
            x,
            z: 3346,
            level: 0,
        };
        local.player.actor.tile = here;
        local.player.network = here;
        snapshot.seed_local_player(local.clone());
        super::super::tests::with_tick(&snapshot, &mut ledger, x as u64, |tick| {
            let context = PredicateContext {
                cx: &tick.cx,
                pairs: tick.pairs,
                quests: &quests,
                progress: &[],
                required_after: tick.cx.evidence(),
                chat_since: 0,
                outcome: None,
            };
            let crate::quester::select::SelectionDecision::Selected(selected) =
                crate::quester::select::select(&path, 0, 0, &context)
            else {
                panic!("the observed safe/arrived tile must select its next real family");
            };
            assert_eq!(selected.step.kind.as_ref(), expected_kind);
        });
    }
}

#[test]
fn unattackable_walk_out_is_selected_after_aborted_unattackable() {
    let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let quests = QuestCatalog::from_identity(data.quest_identity()).unwrap();
    let path = compile_uncached_for_test(
        &serde_json::from_slice(include_bytes!(
            "../../../paths/289/fixtures/combat_unattackable.json"
        ))
        .unwrap(),
        &data,
        &quests,
    )
    .unwrap();
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    snapshot.seed_players(Vec::new());
    super::super::tests::seed_dialogue_combat(&mut snapshot, false);
    let mut local = snapshot.local_player().unwrap().clone();
    local.player.actor.tile = api::WorldTile {
        x: 3109,
        z: 3346,
        level: 0,
    };
    snapshot.seed_local_player(local);
    let aborted = report(CombatEnd::Aborted(AbortReason::Unattackable));
    let outcome = StepOutcome {
        progress: None,
        evidence: aborted.evidence,
        receipt: Some(Arc::new(CombatReceipt {
            report: aborted,
            target_gone_restarts: 0,
        })),
    };
    let mut ledger = None;
    super::super::tests::with_tick(&snapshot, &mut ledger, 40, |tick| {
        let context = PredicateContext {
            cx: &tick.cx,
            pairs: tick.pairs,
            quests: &quests,
            progress: &[],
            required_after: tick.cx.evidence(),
            chat_since: 0,
            outcome: Some(&outcome),
        };
        let crate::quester::select::SelectionDecision::Selected(selected) =
            crate::quester::select::select(&path, 0, 0, &context)
        else {
            panic!("Aborted(Unattackable) must select the Path walk-out step");
        };
        assert_eq!(
            selected.step.id.0.as_ref(),
            "walk-out-after-unattackable",
            "the caller-owned Quester walk step must run after Unattackable"
        );
    });
}

fn unattackable_outcome() -> StepOutcome {
    let aborted = report(CombatEnd::Aborted(AbortReason::Unattackable));
    StepOutcome {
        progress: None,
        evidence: aborted.evidence,
        receipt: Some(Arc::new(CombatReceipt {
            report: aborted,
            target_gone_restarts: 0,
        })),
    }
}

#[test]
fn unattackable_approach_stays_skipped_after_abort_even_when_leaving_the_tree() {
    let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let quests = QuestCatalog::from_identity(data.quest_identity()).unwrap();
    let path = compile_uncached_for_test(
        &serde_json::from_slice(include_bytes!(
            "../../../paths/289/fixtures/combat_unattackable.json"
        ))
        .unwrap(),
        &data,
        &quests,
    )
    .unwrap();
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    snapshot.seed_players(Vec::new());
    super::super::tests::seed_dialogue_combat(&mut snapshot, false);
    let mut local = snapshot.local_player().unwrap().clone();
    local.player.actor.tile = api::WorldTile {
        x: 3100,
        z: 3300,
        level: 0,
    };
    snapshot.seed_local_player(local);
    let outcome = unattackable_outcome();
    let mut ledger = None;
    super::super::tests::with_tick(&snapshot, &mut ledger, 50, |tick| {
        let context = PredicateContext {
            cx: &tick.cx,
            pairs: tick.pairs,
            quests: &quests,
            progress: &[],
            required_after: tick.cx.evidence(),
            chat_since: 0,
            outcome: Some(&outcome),
        };
        let crate::quester::select::SelectionDecision::Selected(selected) =
            crate::quester::select::select(&path, 0, 0, &context)
        else {
            panic!("leaving the tree after Unattackable must keep the caller walk-out selected");
        };
        assert_eq!(
            selected.step.id.0.as_ref(),
            "walk-out-after-unattackable",
            "sticky combat_end must skip approach once the abort report exists"
        );
    });
}

#[test]
fn unattackable_walk_out_is_skipped_once_the_caller_has_arrived() {
    let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let quests = QuestCatalog::from_identity(data.quest_identity()).unwrap();
    let path = compile_uncached_for_test(
        &serde_json::from_slice(include_bytes!(
            "../../../paths/289/fixtures/combat_unattackable.json"
        ))
        .unwrap(),
        &data,
        &quests,
    )
    .unwrap();
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    snapshot.seed_players(Vec::new());
    super::super::tests::seed_dialogue_combat(&mut snapshot, false);
    let mut local = snapshot.local_player().unwrap().clone();
    let here = api::WorldTile {
        x: 3093,
        z: 3243,
        level: 0,
    };
    local.player.actor.tile = here;
    local.player.network = here;
    snapshot.seed_local_player(local);
    let outcome = unattackable_outcome();
    let mut ledger = None;
    super::super::tests::with_tick(&snapshot, &mut ledger, 60, |tick| {
        let context = PredicateContext {
            cx: &tick.cx,
            pairs: tick.pairs,
            quests: &quests,
            progress: &[],
            required_after: tick.cx.evidence(),
            chat_since: 0,
            outcome: Some(&outcome),
        };
        assert!(
            matches!(
                crate::quester::select::select(&path, 0, 0, &context),
                crate::quester::select::SelectionDecision::Exhausted
            ),
            "arrived walk-out after Unattackable must not loop the walk step"
        );
    });
}

struct NeverStop;

impl PredicatePlan for NeverStop {
    fn evaluate(&self, _: &PredicateContext<'_, '_>) -> Truth {
        Truth::False
    }
}

fn combat_test_run(
    target: Target,
    stand: Option<api::WorldTile>,
    until: Option<Arc<dyn PredicatePlan>>,
    loot: Vec<LootItem>,
) -> CombatRun {
    let selected = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let tables = CombatTables::build(selected).unwrap();
    let request = CombatRequest {
        target,
        stand,
        lost_radius: 12,
        ..CombatRequest::default()
    };
    CombatRun {
        request: Arc::new(request),
        tables,
        cross: Arc::from([]),
        protect: false,
        until,
        win: None,
        loot: Arc::from(loot),
        action: None,
        phase: Phase::Combat,
        loot_index: 0,
        last_report: None,
        last_outcome: None,
        target_gone_restarts: 0,
        manual_takeover_watch: crate::native::ManualTakeoverWatch::default(),
        raised_prayers: crate::combat::RaisedPrayers::empty(),
        finish: None,
        finish_target_index: None,
        finish_ticks_elapsed: 0,
        finish_last_tick: None,
        trace_event: None,
        abort_escape: AbortEscape::new(),
    }
}
pub(crate) fn policy_s2_run_for_runner(cx: &mut StepContext<'_, '_>) -> Box<dyn StepRun> {
    let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let mut run = combat_test_run(imp_target(&data), None, None, Vec::new());
    run.begin_combat(cx).unwrap();
    Box::new(run)
}

/// The real `CombatPlan::begin`: it stamps the walk outcome sequence the step
/// began at, so a takeover before the begin cannot end the step.
pub(crate) fn imp_plan_run_for_runner(cx: &mut StepContext<'_, '_>) -> Box<dyn StepRun> {
    let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let run = combat_test_run(imp_target(&data), None, None, Vec::new());
    CombatPlan {
        request: run.request,
        tables: run.tables,
        cross: run.cross,
        protect: false,
        until: None,
        win: None,
        loot: run.loot,
        finish: None,
    }
    .begin(cx)
    .unwrap()
}

#[test]
fn manual_takeover_watch_ends_the_combat_step_after_begin() {
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    let mut ledger = None;
    let at_begin = publish_walk_outcome(7, WalkCancelReason::None);
    let mut run = with_step_context_at_walk_outcome(&snapshot, &mut ledger, 10, at_begin, |cx| {
        imp_plan_run_for_runner(cx)
    });
    assert!(
        with_step_context_at_walk_outcome(&snapshot, &mut ledger, 11, at_begin, |cx| run.poll(cx))
            .is_pending(),
        "no takeover yet"
    );
    // The machine's own walk already arrived: the host moves only the
    // sequence and names the takeover's outcome UserInput.
    let takeover = publish_walk_outcome(8, WalkCancelReason::UserInput);
    let result =
        with_step_context_at_walk_outcome(&snapshot, &mut ledger, 12, takeover, |cx| run.poll(cx));
    assert!(matches!(result, Poll::Ready(Err(ActionError::UserInput))));
}

#[test]
fn manual_takeover_watch_ignores_takeover_before_begin() {
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    let mut ledger = None;
    // An earlier step's takeover is the latest observed outcome at begin.
    let earlier = publish_walk_outcome(8, WalkCancelReason::UserInput);
    let mut run = with_step_context_at_walk_outcome(&snapshot, &mut ledger, 10, earlier, |cx| {
        imp_plan_run_for_runner(cx)
    });
    for tick in 11..=13 {
        let result =
            with_step_context_at_walk_outcome(&snapshot, &mut ledger, tick, earlier, |cx| {
                run.poll(cx)
            });
        assert!(
            !matches!(result, Poll::Ready(Err(ActionError::UserInput))),
            "a stale takeover ended the step at tick {tick}"
        );
    }
    // A later outcome that is not a takeover does not end it either.
    let later = publish_walk_outcome(9, WalkCancelReason::None);
    let result =
        with_step_context_at_walk_outcome(&snapshot, &mut ledger, 14, later, |cx| run.poll(cx));
    assert!(!matches!(result, Poll::Ready(Err(ActionError::UserInput))));
}

pub(crate) fn no_food_abort_run_for_runner(
    cx: &mut StepContext<'_, '_>,
    stand: api::WorldTile,
) -> Box<dyn StepRun> {
    let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let warlord = data.npc_by_config("khazard_warlord").unwrap();
    let mut run = combat_test_run(
        Target::Npc {
            types: Arc::from([warlord.id]),
            pick: Pick::Nearest,
            not_targeting_others: true,
        },
        Some(stand),
        Some(Arc::new(NeverStop)),
        Vec::new(),
    );
    let mut aborted = report(CombatEnd::Aborted(AbortReason::Unprotected(
        crate::combat::Unprotected::NoFood,
    )));
    aborted.engaged = Some(crate::combat::ActorRef {
        kind: api::snapshot::ActorKind::Npc,
        index: 0,
    });
    aborted.engaged_npc_type = warlord.id;

    assert!(
        run.on_combat_report(aborted, cx).is_pending(),
        "a boss abort must keep the real CombatRun alive while its walk settles"
    );
    Box::new(run)
}

fn imp_target(data: &SelectedGameData) -> Target {
    Target::Npc {
        types: Arc::from([data.npc_by_config("imp").unwrap().id]),
        pick: Pick::Random,
        not_targeting_others: true,
    }
}

fn finish_test_run(data: &SelectedGameData, max_ticks: u32) -> CombatRun {
    let original = data.npc_by_config("delrith").unwrap();
    let weakened = data.npc_by_config("delrith_weakened").unwrap();
    let mut run = combat_test_run(
        Target::Npc {
            types: Arc::from([original.id]),
            pick: Pick::Nearest,
            not_targeting_others: true,
        },
        None,
        None,
        Vec::new(),
    );
    run.finish = Some(FinishConfig {
        npc_type: weakened.id,
        npc_name: Arc::from(weakened.display.as_deref().unwrap()),
        original_npc_types: Arc::from([original.id]),
        options: DialogueOptions {
            prefer: Arc::from([Arc::from("Carlem Aber Camerinthum Purchai Gabindo")]),
            strict: true,
            ..DialogueOptions::default()
        },
        max_ticks,
    });
    run
}

fn start_finish_wait(run: &mut CombatRun, tick: u64) {
    run.phase = Phase::FinishWait;
    run.finish_target_index = Some(42);
    run.finish_ticks_elapsed = 0;
    run.finish_last_tick = Some(tick);
}

fn finish_npc(index: usize, npc_type: i32) -> api::snapshot::NpcView {
    api::snapshot::NpcView {
        index,
        r#type: usize::try_from(npc_type).ok(),
        name: None,
        actions: vec![Some("Attack".into())],
        tile: api::WorldTile {
            x: 3254,
            z: 3401,
            level: 0,
        },
        distance: 1,
        animation: -1,
        animation_frame: 0,
        pose_animation: -1,
        orientation: 0,
        target_orientation: 0,
        overhead_text: None,
        spot_animation: -1,
        spot_animation_stamp: -1,
        health: 10,
        total_health: 10,
        face_entity: -1,
        target: None,
        moving: false,
        running: false,
        in_combat: true,
        level: 1,
        size: 1,
        network: api::WorldTile {
            x: 3254,
            z: 3401,
            level: 0,
        },
        x: 0,
        z: 0,
        yaw: 0,
    }
}

fn seed_finish_target(snapshot: &mut GameSnapshot, index: Option<usize>, in_combat: bool) {
    let mut local = super::super::tests::local_player(api::WorldTile {
        x: 3253,
        z: 3401,
        level: 0,
    });
    local.player.actor.in_combat = in_combat;
    local.player.actor.target = index.map(|index| api::snapshot::ActorTargetView {
        kind: api::snapshot::ActorKind::Npc,
        index,
    });
    snapshot.seed_local_player(local);
}

fn inventory_item(id: i32, name: &str, count: i32) -> api::snapshot::ItemView {
    api::snapshot::ItemView {
        def: api::obj_names::ItemDefView {
            id,
            name: Some(name.to_owned()),
            stackable: false,
            members: false,
            base_value: 1,
            noted: false,
            certificate_link: -1,
            certificate_template: -1,
        },
        container: api::snapshot::ItemContainer::Inventory,
        action_family: api::snapshot::ItemActionFamily::Held,
        slot: 0,
        count,
        actions: vec![],
        component_id: 3214,
    }
}

fn ground_item(id: i32, name: &str, tile: api::WorldTile, distance: i32) -> GroundItemView {
    GroundItemView {
        def: api::obj_names::ItemDefView {
            id,
            name: Some(name.to_owned()),
            stackable: false,
            members: false,
            base_value: 1,
            noted: false,
            certificate_link: -1,
            certificate_template: -1,
        },
        count: 1,
        actions: vec![Some("Take".into())],
        tile,
        distance,
    }
}

fn with_step_context<R>(
    snapshot: &GameSnapshot,
    ledger: &mut Option<Box<crate::native::ledger::Ledger>>,
    tick: u64,
    f: impl FnOnce(&mut StepContext<'_, '_>) -> R,
) -> R {
    with_step_context_at_walk_outcome(snapshot, ledger, tick, WalkOutcomeFact::default(), f)
}

fn with_step_context_at_walk_outcome<R>(
    snapshot: &GameSnapshot,
    ledger: &mut Option<Box<crate::native::ledger::Ledger>>,
    tick: u64,
    outcome: WalkOutcomeFact,
    f: impl FnOnce(&mut StepContext<'_, '_>) -> R,
) -> R {
    super::super::tests::with_tick(snapshot, ledger, tick, |native| {
        native.cx.observed_walk_outcome_seq = outcome.seq;
        native.cx.observed_walk_cancel_reason = outcome.cancel_reason;
        let quests = QuestCatalog::empty();
        let required_after = native.cx.evidence();
        let banks = Arc::new(api::named_banks::NamedBankFacts::empty());
        f(&mut StepContext {
            tick: native,
            quests: &quests,
            progress: &[],
            required_after,
            banks: &banks,
            choices: &crate::quester::choices::QuestChoices::default(),
        })
    })
}

/// The host's walk outcome as the slot observes it (`observe_walk_outcome`).
#[derive(Debug, Clone, Copy, Default)]
struct WalkOutcomeFact {
    seq: u64,
    cancel_reason: WalkCancelReason,
}

fn publish_walk_outcome(seq: u64, cancel_reason: WalkCancelReason) -> WalkOutcomeFact {
    WalkOutcomeFact { seq, cancel_reason }
}

#[test]
fn boss_abort_blocks_but_m4_unattackable_report_completes_for_walkout() {
    let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let target = imp_target(&data);
    let never_stop = || Arc::new(NeverStop) as Arc<dyn PredicatePlan>;
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    let mut ledger = None;

    let mut boss = combat_test_run(target, None, Some(never_stop()), Vec::new());
    let result = with_step_context(&snapshot, &mut ledger, 12, |cx| {
        boss.on_combat_report(report(CombatEnd::Aborted(AbortReason::Unattackable)), cx)
    });
    assert!(matches!(result, Poll::Ready(Err(ActionError::Blocked(_)))));
    assert_eq!(
        boss.last_report.map(|report| report.end),
        Some(CombatEnd::Aborted(AbortReason::Unattackable))
    );

    let mut m4 = combat_test_run(
        Target::Attacker {
            npcs: true,
            players: false,
        },
        None,
        Some(never_stop()),
        Vec::new(),
    );
    let result = with_step_context(&snapshot, &mut ledger, 13, |cx| {
        m4.on_combat_report(report(CombatEnd::Aborted(AbortReason::Unattackable)), cx)
    });
    let Poll::Ready(Ok(outcome)) = result else {
        panic!("M4 Unattackable must complete so the caller walk-out can run");
    };
    let receipt = outcome
        .receipt
        .as_ref()
        .and_then(|receipt| receipt.as_any().downcast_ref::<CombatReceipt>())
        .expect("M4 must retain its Combat report");
    assert_eq!(
        receipt.report.end,
        CombatEnd::Aborted(AbortReason::Unattackable)
    );
}

type TestLedger = Option<Box<crate::native::ledger::Ledger>>;

/// The recorded M4 geometry (DIAG-M4-1027): the player at (3109,3346) one tile
/// east of the static hostile tree at (3108,3346).
const M4_HERE: api::WorldTile = api::WorldTile {
    x: 3109,
    z: 3346,
    level: 0,
};
const M4_TREE: api::WorldTile = api::WorldTile {
    x: 3108,
    z: 3346,
    level: 0,
};
/// The tile 17 tiles east of the player, which the old escape chose. A dead
/// tree thicket leaves it no legal Reach arrival goal.
const M4_THICKET: api::WorldTile = api::WorldTile {
    x: 3126,
    z: 3346,
    level: 0,
};
/// Chase envelope of the tree: `2 * maxrange + attackrange` is 14, and the
/// escape tiers count tiles beyond it from 16 on.
const M4_ENVELOPE: i32 = 15;

/// Open ground around the recorded tiles (x 3069..3150, z 3306..3387) with
/// every tile for which `blocked` holds closed.
fn m4_grid(blocked: impl Fn(i32, i32) -> bool) -> nav_planner::collision::WorldCollision {
    let origin = api::WorldTile {
        x: 3069,
        z: 3306,
        level: 0,
    };
    let size = 81usize;
    let mut flags = vec![0u32; 4 * size * size];
    let block = client::dash3d::CollisionFlag::SQ_BLOCKED as u32;
    for dz in 0..size {
        for dx in 0..size {
            let (x, z) = (origin.x + dx as i32, origin.z + dz as i32);
            if blocked(x, z) {
                flags[dz * size + dx] = block;
            }
        }
    }
    let (walk, packed) = nav_planner::collision::pack_walk(&flags);
    nav_planner::collision::WorldCollision {
        origin,
        width: size,
        height: size,
        walk,
        blocked: packed,
        flags: None,
    }
}

/// The thicket blocks every tile of (3125..=3127, 3345..=3347) except the
/// diagonal (3127,3347), the only standable tile of the 3-by-3 area around the
/// old destination.
fn m4_collision() -> nav_planner::collision::WorldCollision {
    m4_grid(|x, z| {
        (3125..=3127).contains(&x) && (3345..=3347).contains(&z) && (x, z) != (3127, 3347)
    })
}

/// The player stands in a corridor that runs 6 tiles north and then 37 tiles
/// east, walled in on every side. The first tiles outside the tree's chase
/// envelope lie 21 steps away, on no straight line from the player.
fn corridor_collision() -> nav_planner::collision::WorldCollision {
    m4_grid(|x, z| {
        let walled = (3107..=3146).contains(&x) && (3338..=3354).contains(&z);
        let corridor =
            (x == 3109 && (3340..=3352).contains(&z)) || (z == 3352 && (3109..=3146).contains(&x));
        walled && !corridor
    })
}

/// Open ground with a closed room around `stand`: the stand and its
/// neighbours are standable, and no step leads into them.
fn pocket_collision(stand: api::WorldTile) -> nav_planner::collision::WorldCollision {
    m4_grid(move |x, z| (x - stand.x).abs().max((z - stand.z).abs()) == 2)
}

fn with_step_context_collision<R>(
    snapshot: &GameSnapshot,
    collision: nav_planner::contract::CollisionView<'_>,
    ledger: &mut TestLedger,
    tick: u64,
    f: impl FnOnce(&mut StepContext<'_, '_>) -> R,
) -> R {
    super::super::tests::with_tick_collision(snapshot, collision, ledger, tick, |native| {
        let quests = QuestCatalog::empty();
        let required_after = native.cx.evidence();
        let banks = Arc::new(api::named_banks::NamedBankFacts::empty());
        f(&mut StepContext {
            tick: native,
            quests: &quests,
            progress: &[],
            required_after,
            banks: &banks,
            choices: &crate::quester::choices::QuestChoices::default(),
        })
    })
}

struct WalkOut {
    request_id: u64,
    target: api::WorldTile,
    food_guard: bool,
    allow_food: bool,
    arrival: nav_planner::contract::ArrivalKind,
    radius: u16,
    goals: Option<crate::native::WalkGoals>,
}

/// The walk the run requested last, when its newest outbox entry is a walk.
fn pending_walk(ledger: &TestLedger) -> Option<WalkOut> {
    let action = ledger.as_ref()?.outbox.last()?;
    let crate::native::HostEffect::Walk(request) = &action.effect else {
        return None;
    };
    Some(WalkOut {
        request_id: action.request_id.get(),
        target: request.target,
        food_guard: request.food_guard,
        allow_food: request.allow.food,
        arrival: request.arrival,
        radius: request.radius,
        goals: request.candidate_goals.clone(),
    })
}

fn last_walk(ledger: &TestLedger) -> WalkOut {
    pending_walk(ledger).expect("the abort must have requested a native walk")
}

/// The walk's goals. The adapter names exact Area goals and the host router
/// owns reachability, as in the Gatherer's observation walks.
fn goals_of(walk: &WalkOut) -> &crate::native::WalkGoals {
    assert_eq!(
        walk.arrival,
        nav_planner::contract::ArrivalKind::Area,
        "the escape must name exact Area goals for the shared router"
    );
    assert_eq!(walk.radius, 0, "exact goals need no arrival margin");
    walk.goals
        .as_ref()
        .expect("the escape must hand the router exact goal tiers")
}

/// A tile where the walk can arrive: its first goal, or its target.
fn arrival_goal(walk: &WalkOut) -> api::WorldTile {
    walk.goals
        .as_ref()
        .and_then(|goals| goals.as_slice().first().copied())
        .unwrap_or(walk.target)
}

/// The router's choice for `walk` over the baked collision it routes on: the
/// nearest goal of the first tier with a goal on the player's component, or,
/// for a single-target Reach walk, the nearest legal arrival goal on it. This
/// stands in for the host router, so a test can tell whether the first walk
/// of an escape can route at all.
fn routed_goal(
    collision: &nav_planner::collision::WorldCollision,
    lease: &nav_planner::contract::NavLease,
    walk: &WalkOut,
) -> Option<api::WorldTile> {
    let reachable = nav_planner::paint::flood_components(collision, &[M4_HERE]).swap_remove(0);
    let nearest = |tiles: Vec<api::WorldTile>| {
        tiles
            .into_iter()
            .filter(|tile| lease.collision().standable(*tile) && reachable.contains(tile))
            .min_by_key(|tile| tile_distance(M4_HERE, *tile))
    };
    match &walk.goals {
        Some(goals) => {
            nearest(goals.preferred().to_vec()).or_else(|| nearest(goals.fallback().to_vec()))
        }
        None => {
            let radius = i32::from(walk.radius);
            let spec = nav_planner::contract::ArrivalSpec::reach(walk.radius);
            nearest(
                (-radius..=radius)
                    .flat_map(|dx| {
                        (-radius..=radius).map(move |dz| api::WorldTile {
                            x: walk.target.x + dx,
                            z: walk.target.z + dz,
                            level: walk.target.level,
                        })
                    })
                    .filter(|goal| lease.collision().arrived(*goal, walk.target, spec) == Ok(true))
                    .collect(),
            )
        }
    }
}

/// The host's terminal receipt for the walk the run requested last.
fn post_walk_receipt(
    ledger: &mut TestLedger,
    tick: u64,
    end: WalkEnd,
    detail: Option<&str>,
    goal: Option<api::WorldTile>,
) {
    let request_id = last_walk(ledger).request_id;
    let ledger = ledger.as_mut().unwrap();
    ledger.outbox.clear();
    ledger.walk = Some(WalkReceipt {
        goal,
        request_id,
        evidence: EvidenceStamp {
            run: RunKey {
                slot: 1,
                run: 1,
                session: 1,
            },
            tick,
            sequence: tick,
        },
        end,
        blocked: None,
        detail: detail.map(Arc::from),
        refusal: None,
        assessment: None,
        escape: None,
    });
}

fn post_walk_end(ledger: &mut TestLedger, tick: u64, end: WalkEnd, detail: Option<&str>) {
    post_walk_receipt(ledger, tick, end, detail, None);
}

/// Fail every escape walk the run requests at `tick`, as the router does when
/// no goal of a request routes. The pass spends its requests, and then the
/// guarded hold begins.
fn fail_escape_walks_until_hold(
    run: &mut CombatRun,
    snapshot: &GameSnapshot,
    ledger: &mut TestLedger,
    tick: u64,
) -> Vec<Vec<api::WorldTile>> {
    let mut goal_sets = Vec::new();
    for _ in 0..=escape::ESCAPE_PASS_ATTEMPTS {
        let Some(walk) = pending_walk(ledger) else {
            return goal_sets;
        };
        if let Some(goals) = walk.goals {
            goal_sets.push(goals.as_slice().to_vec());
        }
        post_walk_end(ledger, tick, WalkEnd::Failed, Some("route fixture failure"));
        assert!(with_step_context(snapshot, ledger, tick, |cx| run.poll(cx)).is_pending());
    }
    panic!("the escape pass must end in the guarded hold");
}

fn m4_run_with_stand(stand: Option<api::WorldTile>) -> CombatRun {
    combat_test_run(
        Target::Attacker {
            npcs: true,
            players: false,
        },
        stand,
        Some(Arc::new(NeverStop)),
        Vec::new(),
    )
}

fn m4_run() -> CombatRun {
    m4_run_with_stand(None)
}

fn m4_report(data: &SelectedGameData) -> CombatReport {
    let mut aborted = report(CombatEnd::Aborted(AbortReason::Unattackable));
    aborted.engaged = Some(crate::combat::ActorRef {
        kind: api::snapshot::ActorKind::Npc,
        index: 4328,
    });
    aborted.engaged_npc_type = data.npc_by_config("nasty_tree").unwrap().id;
    aborted
}

/// HP 8, low Prayer, lobsters when `food`, and the static tree (type 152, no
/// operations) attacking when `attacked`.
fn seed_m4_scene(
    snapshot: &mut GameSnapshot,
    here: api::WorldTile,
    attacked: bool,
    attack_animation: i32,
    food: bool,
) {
    seed_m4_scene_at(snapshot, here, M4_TREE, attacked, attack_animation, food);
}

fn seed_m4_scene_at(
    snapshot: &mut GameSnapshot,
    here: api::WorldTile,
    tree_tile: api::WorldTile,
    attacked: bool,
    attack_animation: i32,
    food: bool,
) {
    let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let tree = data.npc_by_config("nasty_tree").unwrap();
    let lobster = data.item_by_alias("lobster").unwrap();
    let mut attackers = Vec::new();
    if attacked {
        let mut npc = aborting_npc(4328, tree.id, tree_tile);
        npc.actions = Vec::new();
        npc.animation = attack_animation;
        attackers.push(npc);
    }
    seed_abort_scene(
        snapshot,
        here,
        8,
        attackers,
        food.then_some(lobster.id),
        food.then_some("Lobster"),
    );
    snapshot.seed_stats(
        (0..25)
            .map(|index| api::snapshot::StatView {
                index,
                name: String::new(),
                effective: if index == 3 { 8 } else { 40 },
                base: match index {
                    3 => 40,
                    5 => 1,
                    _ => 40,
                },
                xp: 0,
                used: api::snapshot::stat_used(index as usize),
            })
            .collect(),
    );
}

/// The M4 abort report, polled once with the tree attacking.
fn start_m4_abort(
    run: &mut CombatRun,
    lease: &nav_planner::contract::NavLease,
    snapshot: &GameSnapshot,
    ledger: &mut TestLedger,
) {
    let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
    assert!(
        with_step_context_collision(snapshot, lease.collision(), ledger, 12, |cx| {
            run.on_combat_report(m4_report(&data), cx)
        })
        .is_pending()
    );
}

#[test]
fn m4_escape_hands_the_router_exact_standable_goals_in_one_tier() {
    let collision = m4_collision();
    let lease = crate::gatherer::select::tests::test_nav_lease_for_collision(&collision);
    let mut run = m4_run();
    let mut snapshot = GameSnapshot::new();
    seed_m4_scene(
        &mut snapshot,
        M4_HERE,
        true,
        live_attack_animation(&run),
        true,
    );
    let mut ledger = None;
    start_m4_abort(&mut run, &lease, &snapshot, &mut ledger);

    let first = last_walk(&ledger);
    let goals = goals_of(&first);
    assert!(
        first.food_guard,
        "low Prayer: the escape runs under the food guard"
    );
    assert!(!goals.preferred().is_empty() && goals.fallback().is_empty());
    for goal in goals.as_slice() {
        assert!(lease.collision().standable(*goal), "{goal:?} is standable");
    }
    assert!(
        !goals.as_slice().contains(&M4_THICKET),
        "the dead-tree thicket tile is not a standable goal"
    );
    for goal in goals.preferred() {
        assert!(
            tile_distance(*goal, M4_TREE) > M4_ENVELOPE,
            "{goal:?} lies outside the chase envelope"
        );
    }
    let routed = routed_goal(&collision, &lease, &first).expect("the router finds a goal");
    assert!(tile_distance(routed, M4_TREE) > M4_ENVELOPE);
}

#[test]
fn m4_escape_keeps_a_goal_beyond_a_closed_door_in_the_live_flood() {
    let here = api::WorldTile {
        x: 3000,
        z: 3000,
        level: 0,
    };
    let stand = api::WorldTile {
        x: here.x + 5,
        ..here
    };
    let door_x = here.x + 3;
    let mut reach = open_flood(here, 8);
    // The closed door blocks the only route into the flood region to its east.
    for x in door_x..reach.base_x + reach.width {
        for z in reach.base_z..reach.base_z + reach.height {
            let index = ((x - reach.base_x) * reach.height + z - reach.base_z) as usize;
            let word = index / u32::BITS as usize;
            let mask = !(1u32 << (index % u32::BITS as usize));
            if x == door_x {
                reach.walkable[word] &= mask;
            }
            reach.reachable[word] &= mask;
            reach.reachable_adj[word] &= mask;
            reach.exact_rank[index] = u16::MAX;
            reach.adjacent_rank[index] = u16::MAX;
            reach.step[index] = 0;
        }
    }
    let door = api::WorldTile { x: door_x, ..here };
    assert!(!crate::reach_route::reach_bit(
        &reach,
        &reach.walkable,
        door
    ));
    assert!(
        crate::reach_route::nearest_depth(&reach, std::slice::from_ref(&stand)).is_none(),
        "the closed door keeps the goal outside the live flood"
    );

    let geometry = escape::EscapeGeometry {
        here,
        threat: None,
        stand: Some(stand),
        chase_range: 0,
    };
    let goals = escape::AbortEscape::new()
        .next_goals(&geometry, None, Some(&reach))
        .expect("the route planner receives the goal beyond the closed door");
    assert!(
        goals.goals.preferred().contains(&stand),
        "the live flood does not remove a goal the router can evaluate"
    );
}

#[test]
fn m4_failed_escape_walk_replans_one_tier_lower_before_food_runs_out() {
    let collision = m4_collision();
    let lease = crate::gatherer::select::tests::test_nav_lease_for_collision(&collision);
    let mut run = m4_run();
    let mut snapshot = GameSnapshot::new();
    seed_m4_scene(
        &mut snapshot,
        M4_HERE,
        true,
        live_attack_animation(&run),
        true,
    );
    let mut ledger = None;
    start_m4_abort(&mut run, &lease, &snapshot, &mut ledger);
    let first = last_walk(&ledger);

    post_walk_end(
        &mut ledger,
        13,
        WalkEnd::Failed,
        Some("route fixture failure"),
    );
    assert!(
        with_step_context_collision(&snapshot, lease.collision(), &mut ledger, 13, |cx| {
            run.poll(cx)
        })
        .is_pending()
    );
    assert!(
        matches!(&run.phase, Phase::WalkingOutAfterAbort),
        "a failed first walk starts another guarded escape, not a hold that eats in place"
    );
    let second = last_walk(&ledger);
    let (first_goals, second_goals) = (goals_of(&first), goals_of(&second));
    assert!(
        first_goals.fallback().is_empty() && second_goals.fallback().is_empty(),
        "each request sends one tier"
    );
    assert!(
        second_goals
            .as_slice()
            .iter()
            .all(|goal| !first_goals.as_slice().contains(goal)),
        "a retry uses goals from a distinct tier"
    );
    assert!(
        second.food_guard,
        "food protection continues on the retry while food remains"
    );

    // The pass has no tier left: the next failure starts the guarded hold.
    post_walk_end(
        &mut ledger,
        14,
        WalkEnd::Failed,
        Some("route fixture failure"),
    );
    assert!(
        with_step_context_collision(&snapshot, lease.collision(), &mut ledger, 14, |cx| {
            run.poll(cx)
        })
        .is_pending()
    );
    assert!(matches!(&run.phase, Phase::HoldingAfterAbort));
}

#[test]
fn m4_safe_escape_completes_with_the_unattackable_receipt_and_selects_the_caller_walk() {
    let collision = m4_collision();
    let lease = crate::gatherer::select::tests::test_nav_lease_for_collision(&collision);
    let mut run = m4_run();
    let attack_animation = live_attack_animation(&run);
    let mut snapshot = GameSnapshot::new();
    seed_m4_scene(&mut snapshot, M4_HERE, true, attack_animation, true);
    let mut ledger = None;
    start_m4_abort(&mut run, &lease, &snapshot, &mut ledger);
    let first = last_walk(&ledger);
    let escape = routed_goal(&collision, &lease, &first).expect("the router finds a goal");

    // The walk arrives and no attacker remains in view.
    post_walk_receipt(&mut ledger, 13, WalkEnd::Arrived, None, Some(escape));
    let mut safe = GameSnapshot::new();
    seed_m4_scene(&mut safe, escape, false, attack_animation, true);
    let result =
        with_step_context_collision(&safe, lease.collision(), &mut ledger, 13, |cx| run.poll(cx));
    let Poll::Ready(Ok(outcome)) = result else {
        panic!(
            "a proved safe escape must complete the M4 step, got {}",
            describe_poll(&result)
        );
    };
    let receipt = outcome
        .receipt
        .as_ref()
        .and_then(|receipt| receipt.as_any().downcast_ref::<CombatReceipt>())
        .expect("the completed step retains its Combat receipt");
    assert_eq!(
        receipt.report.end,
        CombatEnd::Aborted(AbortReason::Unattackable)
    );
    assert_caller_walk_out_selected(&outcome, escape);
}

#[test]
fn m4_escape_that_still_sees_the_attacker_holds_then_completes_when_it_is_gone() {
    let collision = m4_collision();
    let lease = crate::gatherer::select::tests::test_nav_lease_for_collision(&collision);
    let mut run = m4_run();
    let attack_animation = live_attack_animation(&run);
    let mut snapshot = GameSnapshot::new();
    seed_m4_scene(&mut snapshot, M4_HERE, true, attack_animation, true);
    let mut ledger = None;
    start_m4_abort(&mut run, &lease, &snapshot, &mut ledger);
    // The walk arrives while the tree's last attack animation is still live.
    post_walk_end(&mut ledger, 13, WalkEnd::Arrived, None);
    assert!(with_step_context(&snapshot, &mut ledger, 13, |cx| run.poll(cx)).is_pending());
    assert!(matches!(&run.phase, Phase::HoldingAfterAbort));

    let mut safe = GameSnapshot::new();
    seed_m4_scene(&mut safe, M4_HERE, false, attack_animation, true);
    let mut finished = None;
    for tick in 14..=20 {
        if let Poll::Ready(result) = with_step_context(&safe, &mut ledger, tick, |cx| run.poll(cx))
        {
            finished = Some(result);
            break;
        }
    }
    let Some(Ok(outcome)) = finished else {
        panic!(
            "the threat-free hold must complete the M4 step, got {}",
            describe_poll(&finished.map_or(Poll::Pending, Poll::Ready))
        );
    };
    assert!(outcome.receipt.is_some_and(|receipt| {
        receipt
            .as_any()
            .downcast_ref::<CombatReceipt>()
            .is_some_and(|receipt| {
                receipt.report.end == CombatEnd::Aborted(AbortReason::Unattackable)
            })
    }));
}

fn describe_poll(poll: &Poll<Result<StepOutcome, ActionError>>) -> String {
    match poll {
        Poll::Pending => "still pending".to_owned(),
        Poll::Ready(Ok(_)) => "a completed step".to_owned(),
        Poll::Ready(Err(error)) => format!("the error {error:?}"),
    }
}

/// An exit that needs a walk beyond every straight line and local window: the
/// router receives goals on the far side of the corridor, and the first walk
/// routes to a tile outside the chase envelope. The old planner tried eight
/// rays from the player, found only tiles inside the envelope, and so chose a
/// tile a few steps up the corridor.
#[test]
fn m4_escape_reaches_an_exit_beyond_any_ray_or_local_window() {
    let collision = corridor_collision();
    let lease = crate::gatherer::select::tests::test_nav_lease_for_collision(&collision);
    let mut run = m4_run();
    let mut snapshot = GameSnapshot::new();
    seed_m4_scene(
        &mut snapshot,
        M4_HERE,
        true,
        live_attack_animation(&run),
        true,
    );
    let mut ledger = None;
    start_m4_abort(&mut run, &lease, &snapshot, &mut ledger);

    let first = last_walk(&ledger);
    let routed = routed_goal(&collision, &lease, &first)
        .expect("a goal of the first escape request routes from the corridor");
    assert!(
        tile_distance(routed, M4_TREE) > M4_ENVELOPE,
        "{routed:?} must leave the chase envelope, not stop inside the corridor"
    );
    assert!(
        tile_distance(routed, M4_HERE) > 12,
        "the exit is a long walk, outside any ray from the player"
    );
}

/// An authored stand can be standable but disconnected from the player. With
/// no live flood, the planner still sends that exact tier to the route planner.
#[test]
fn m4_disconnected_authored_stand_is_still_sent_to_the_router() {
    let stand = api::WorldTile {
        x: 3109,
        z: 3329,
        level: 0,
    };
    for (name, collision) in [
        ("closed room", pocket_collision(stand)),
        ("open ground", m4_collision()),
    ] {
        let lease = crate::gatherer::select::tests::test_nav_lease_for_collision(&collision);
        let mut run = m4_run_with_stand(Some(stand));
        let mut snapshot = GameSnapshot::new();
        seed_m4_scene(
            &mut snapshot,
            M4_HERE,
            true,
            live_attack_animation(&run),
            true,
        );
        let mut ledger = None;
        start_m4_abort(&mut run, &lease, &snapshot, &mut ledger);

        let first = last_walk(&ledger);
        let goals = goals_of(&first);
        assert!(goals.fallback().is_empty());
        assert!(
            goals.preferred().contains(&stand),
            "{name}: the authored stand remains in the request"
        );
        if name == "open ground" {
            let routed = routed_goal(&collision, &lease, &first)
                .unwrap_or_else(|| panic!("{name}: the route planner finds the open stand"));
            assert!(
                tile_distance(routed, stand) <= 1,
                "{name}: a reachable authored stand wins, got {routed:?}"
            );
        } else {
            assert!(
                routed_goal(&collision, &lease, &first).is_none(),
                "{name}: the route planner rejects the disconnected stand"
            );
            post_walk_end(
                &mut ledger,
                13,
                WalkEnd::Failed,
                Some("the authored stand has no route"),
            );
            let retried =
                with_step_context_collision(&snapshot, lease.collision(), &mut ledger, 13, |cx| {
                    run.poll(cx)
                });
            assert!(
                retried.is_pending(),
                "{name}: a disconnected stand must not finish the escape"
            );
            let next = last_walk(&ledger);
            assert!(
                goals_of(&next)
                    .preferred()
                    .iter()
                    .all(|goal| tile_distance(*goal, M4_TREE) > M4_ENVELOPE),
                "{name}: after a disconnected stand the run sends its next Beyond tier"
            );
            let routed = routed_goal(&collision, &lease, &next)
                .unwrap_or_else(|| panic!("{name}: the next Beyond tier must route"));
            assert!(
                tile_distance(routed, M4_TREE) > M4_ENVELOPE,
                "{name}: the route leaves the attack envelope, got {routed:?}"
            );
        }
    }
}

/// The hold ends without food and the exhausted escape walk begins. Returns
/// the run, its ledger, the snapshot and the earlier request goals.
fn m4_exhausted_escape_walk(
    lease: &nav_planner::contract::NavLease,
    move_tree: bool,
) -> (
    CombatRun,
    TestLedger,
    GameSnapshot,
    Vec<Vec<api::WorldTile>>,
) {
    let mut run = m4_run();
    let mut snapshot = GameSnapshot::new();
    seed_m4_scene(
        &mut snapshot,
        M4_HERE,
        true,
        live_attack_animation(&run),
        false,
    );
    let mut ledger = None;
    let mut earlier_goals = Vec::new();
    start_m4_abort(&mut run, lease, &snapshot, &mut ledger);
    while pending_walk(&ledger).is_some() {
        let failed = last_walk(&ledger);
        earlier_goals.push(goals_of(&failed).as_slice().to_vec());
        post_walk_end(
            &mut ledger,
            13,
            WalkEnd::Failed,
            Some("route fixture failure"),
        );
        assert!(
            with_step_context_collision(&snapshot, lease.collision(), &mut ledger, 13, |cx| {
                run.poll(cx)
            })
            .is_pending()
        );
    }
    assert!(matches!(&run.phase, Phase::HoldingAfterAbort));
    assert!(
        !earlier_goals.is_empty() && earlier_goals.len() <= escape::ESCAPE_PASS_ATTEMPTS,
        "the first pass requests each available escape tier once"
    );
    if move_tree {
        let moved_tree = api::WorldTile {
            x: M4_TREE.x + 1,
            ..M4_TREE
        };
        seed_m4_scene_at(
            &mut snapshot,
            M4_HERE,
            moved_tree,
            true,
            live_attack_animation(&run),
            false,
        );
    }
    for tick in 14..=40 {
        assert!(with_step_context_collision(
            &snapshot,
            lease.collision(),
            &mut ledger,
            tick,
            |cx| { run.poll(cx) }
        )
        .is_pending());
        if pending_walk(&ledger).is_some_and(|walk| !walk.allow_food) {
            if move_tree {
                let moved_again = api::WorldTile {
                    x: M4_TREE.x + 2,
                    ..M4_TREE
                };
                seed_m4_scene_at(
                    &mut snapshot,
                    M4_HERE,
                    moved_again,
                    true,
                    live_attack_animation(&run),
                    false,
                );
            }
            return (run, ledger, snapshot, earlier_goals);
        }
    }
    panic!("the hold must run out of supplies and start the exhausted escape");
}

/// Arrival is not safety: the exhausted pass has neither prayer nor food, and
/// the next request uses a distinct tier. An arrival with the attacker still
/// live plans that tier. The step completes only when no attacker is observed.
#[test]
fn exhausted_m4_escape_arrival_with_a_live_attacker_replans_and_completes_only_when_safe() {
    let collision = m4_collision();
    let lease = crate::gatherer::select::tests::test_nav_lease_for_collision(&collision);
    let (mut run, mut ledger, live, mut previous) = m4_exhausted_escape_walk(&lease, true);
    let first = last_walk(&ledger);
    assert!(!first.food_guard && !first.allow_food);
    assert!(
        !previous.is_empty(),
        "the exhausted pass follows failed walks"
    );
    assert!(
        previous.iter().all(|goals| {
            goals_of(&first)
                .as_slice()
                .iter()
                .all(|goal| !goals.contains(goal))
        }),
        "the exhausted pass does not repeat a goal from the food-supported pass"
    );
    previous.push(goals_of(&first).as_slice().to_vec());
    let goal = arrival_goal(&first);

    post_walk_receipt(&mut ledger, 41, WalkEnd::Arrived, None, Some(goal));
    let polled =
        with_step_context_collision(&live, lease.collision(), &mut ledger, 41, |cx| run.poll(cx));
    assert!(
        polled.is_pending(),
        "an arrival under a live attacker must not complete the step, got {}",
        describe_poll(&polled)
    );
    let second = last_walk(&ledger);
    assert_ne!(second.request_id, first.request_id);
    assert!(goals_of(&first).fallback().is_empty() && goals_of(&second).fallback().is_empty());
    assert!(
        !previous.is_empty(),
        "the attacker moved after the first retry"
    );
    assert!(
        previous.iter().all(|goals| {
            goals_of(&second)
                .as_slice()
                .iter()
                .all(|goal| !goals.contains(goal))
        }),
        "the moving attacker cannot make either pass repeat any earlier goal"
    );

    let mut safe = GameSnapshot::new();
    seed_m4_scene(&mut safe, goal, false, live_attack_animation(&run), false);
    post_walk_receipt(&mut ledger, 42, WalkEnd::Arrived, None, Some(goal));
    let result =
        with_step_context_collision(&safe, lease.collision(), &mut ledger, 42, |cx| run.poll(cx));
    let Poll::Ready(Ok(outcome)) = result else {
        panic!(
            "an arrival with no attacker in view completes, got {}",
            describe_poll(&result)
        );
    };
    assert!(outcome.receipt.is_some_and(|receipt| {
        receipt
            .as_any()
            .downcast_ref::<CombatReceipt>()
            .is_some_and(|receipt| {
                receipt.report.end == CombatEnd::Aborted(AbortReason::Unattackable)
            })
    }));
}

#[test]
fn stationary_m4_tree_gets_untried_goals_after_food_runs_out() {
    let collision = m4_collision();
    let lease = crate::gatherer::select::tests::test_nav_lease_for_collision(&collision);
    let (_, ledger, _, previous) = m4_exhausted_escape_walk(&lease, false);
    let exhausted = last_walk(&ledger);
    assert!(!exhausted.food_guard && !exhausted.allow_food);
    assert!(
        previous.iter().all(|goals| {
            goals_of(&exhausted)
                .as_slice()
                .iter()
                .all(|goal| !goals.contains(goal))
        }),
        "the stationary tree's exhausted request uses only untried goals"
    );
}

#[test]
fn exhausted_m4_escape_that_never_loses_the_attacker_ends_with_the_unsafe_arrival() {
    let collision = m4_collision();
    let lease = crate::gatherer::select::tests::test_nav_lease_for_collision(&collision);
    let (mut run, mut ledger, live, _) = m4_exhausted_escape_walk(&lease, true);
    let mut tick = 41;
    let result = loop {
        let goal = arrival_goal(&last_walk(&ledger));
        post_walk_receipt(&mut ledger, tick, WalkEnd::Arrived, None, Some(goal));
        let polled =
            with_step_context_collision(&live, lease.collision(), &mut ledger, tick, |cx| {
                run.poll(cx)
            });
        if let Poll::Ready(result) = polled {
            break result;
        }
        tick += 1;
        assert!(
            tick <= 41 + escape::ESCAPE_PASS_ATTEMPTS as u64,
            "the exhausted pass must end"
        );
    };
    assert!(
        matches!(&result, Err(ActionError::Blocked(reason)) if reason.contains("without proof")),
        "the step ends with the unsafe arrival, never a completion: {}",
        describe_poll(&Poll::Ready(result))
    );
}

#[test]
fn escape_goals_never_exceed_the_router_cap_and_requests_use_distinct_tiers() {
    let mut pass = escape::AbortEscape::new();
    let stand = api::WorldTile {
        x: M4_TREE.x + 16,
        ..M4_TREE
    };
    let geometry = escape::EscapeGeometry {
        here: M4_HERE,
        threat: Some(M4_TREE),
        stand: Some(stand),
        chase_range: 14,
    };
    let mut request_goals: Vec<Vec<api::WorldTile>> = Vec::new();
    while let Some(next) = pass.next_goals(&geometry, None, None) {
        assert!(next.goals.as_slice().len() <= crate::native::WalkGoals::MAX);
        assert!(
            next.goals.fallback().is_empty(),
            "each request sends one tier"
        );
        let goals = next.goals.as_slice().to_vec();
        assert!(
            request_goals
                .iter()
                .all(|previous| { goals.iter().all(|goal| !previous.contains(goal)) }),
            "each retry names goals from a distinct tier"
        );
        request_goals.push(goals);
    }
    assert_eq!(request_goals.len(), escape::ESCAPE_PASS_ATTEMPTS);
    assert!(
        request_goals[1].len() > crate::native::WalkGoals::MAX / 2,
        "the Beyond tier must use more than the old half-cap"
    );
    assert!(
        request_goals[0].contains(&stand),
        "the authored stand is the first preference"
    );
}

#[test]
fn moving_attacker_cannot_repeat_an_earlier_failed_escape_goal() {
    let mut pass = escape::AbortEscape::new();
    let before_move = escape::EscapeGeometry {
        here: api::WorldTile {
            x: 3002,
            z: 3000,
            level: 0,
        },
        threat: Some(api::WorldTile {
            x: 3000,
            z: 3000,
            level: 0,
        }),
        stand: None,
        chase_range: 4,
    };
    let failed = pass
        .next_goals(&before_move, None, None)
        .expect("the initial geometry offers an escape region");
    let mut earlier_goals = vec![failed.goals.as_slice().to_vec()];
    let repeated_under_new_geometry = api::WorldTile {
        x: 3006,
        z: 3000,
        level: 0,
    };
    assert!(
        failed
            .goals
            .as_slice()
            .contains(&repeated_under_new_geometry),
        "the first Beyond region includes the moving-attacker probe tile"
    );

    let after_move = escape::EscapeGeometry {
        threat: Some(api::WorldTile {
            x: 3001,
            z: 3000,
            level: 0,
        }),
        ..before_move
    };
    let next = pass
        .next_goals(&after_move, None, None)
        .expect("fresh goals remain after the attacker moves");
    assert!(
        !next.goals.as_slice().contains(&repeated_under_new_geometry),
        "the failed goal is excluded even when the moving threat changes its region"
    );
    assert!(
        earlier_goals.iter().all(|goals| {
            next.goals
                .as_slice()
                .iter()
                .all(|goal| !goals.contains(goal))
        }),
        "successive requests in one escape have no repeated tiles"
    );
    earlier_goals.push(next.goals.as_slice().to_vec());

    pass.begin_pass();
    let after_supply_exhaustion = pass
        .next_goals(&after_move, None, None)
        .expect("a moving attacker exposes fresh goals after supply exhaustion");
    assert!(
        earlier_goals.iter().all(|goals| {
            after_supply_exhaustion
                .goals
                .as_slice()
                .iter()
                .all(|goal| !goals.contains(goal))
        }),
        "the second pass keeps the first pass's tile exclusions"
    );
}

#[test]
fn escape_without_a_named_attacker_offers_only_the_authored_stand() {
    let stand = api::WorldTile {
        x: 3109,
        z: 3329,
        level: 0,
    };
    let geometry = |stand| escape::EscapeGeometry {
        here: M4_HERE,
        threat: None,
        stand,
        chase_range: 0,
    };
    let mut pass = escape::AbortEscape::new();
    let next = pass
        .next_goals(&geometry(Some(stand)), None, None)
        .expect("the stand is the only candidate");
    assert!(next.goals.as_slice().contains(&stand) && next.goals.fallback().is_empty());
    assert!(pass
        .next_goals(&geometry(Some(stand)), None, None)
        .is_none());
    assert!(escape::AbortEscape::new()
        .next_goals(&geometry(None), None, None)
        .is_none());
}

#[test]
fn escape_stand_inside_the_chase_envelope_is_not_the_first_preference() {
    let near_stand = api::WorldTile {
        x: M4_TREE.x - 5,
        ..M4_TREE
    };
    let geometry = escape::EscapeGeometry {
        here: M4_HERE,
        threat: Some(M4_TREE),
        stand: Some(near_stand),
        chase_range: 14,
    };
    let first = escape::AbortEscape::new()
        .next_goals(&geometry, None, None)
        .expect("tiers beyond the envelope remain");
    for goal in first.goals.preferred() {
        assert!(tile_distance(*goal, M4_TREE) > M4_ENVELOPE);
    }
}

#[test]
fn escape_goals_on_the_reference_pack_exclude_the_dead_tree_thicket() {
    let Some(_pack) =
        crate::ctx::test_support::local_asset_path("NAV_PACK", "", "real 289 nav pack")
    else {
        return;
    };
    let world = crate::gatherer::select::test_real_nav_world();
    let lease = crate::gatherer::select::tests::test_real_nav_lease();
    let reachable =
        nav_planner::paint::flood_components(&world.collision, &[M4_HERE]).swap_remove(0);
    let geometry = escape::EscapeGeometry {
        here: M4_HERE,
        threat: Some(M4_TREE),
        stand: None,
        chase_range: 14,
    };
    let first = escape::AbortEscape::new()
        .next_goals(&geometry, Some(lease.collision()), None)
        .expect("the reference pack leaves standable tiles around the tree");
    assert!(!first.goals.as_slice().contains(&M4_THICKET));
    for goal in first.goals.as_slice() {
        assert!(lease.collision().standable(*goal), "{goal:?}");
    }
    assert!(
        first
            .goals
            .preferred()
            .iter()
            .any(|goal| reachable.contains(goal)),
        "a goal beyond the chase envelope is reachable on the baked component"
    );
}

/// The Path's `walk-out-after-unattackable` step is the next step selected
/// from the completed step's outcome.
fn assert_caller_walk_out_selected(outcome: &StepOutcome, here: api::WorldTile) {
    let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let quests = QuestCatalog::from_identity(data.quest_identity()).unwrap();
    let path = compile_uncached_for_test(
        &serde_json::from_slice(include_bytes!(
            "../../../paths/289/fixtures/combat_unattackable.json"
        ))
        .unwrap(),
        &data,
        &quests,
    )
    .unwrap();
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    snapshot.seed_players(Vec::new());
    super::super::tests::seed_dialogue_combat(&mut snapshot, false);
    let mut local = snapshot.local_player().unwrap().clone();
    local.player.actor.tile = here;
    local.player.network = here;
    snapshot.seed_local_player(local);
    let mut ledger = None;
    super::super::tests::with_tick(&snapshot, &mut ledger, 40, |tick| {
        let context = PredicateContext {
            cx: &tick.cx,
            pairs: tick.pairs,
            quests: &quests,
            progress: &[],
            required_after: tick.cx.evidence(),
            chat_since: 0,
            outcome: Some(outcome),
        };
        let crate::quester::select::SelectionDecision::Selected(selected) =
            crate::quester::select::select(&path, 0, 0, &context)
        else {
            panic!("the completed step must select the Path walk-out step");
        };
        assert_eq!(
            selected.step.id.0.as_ref(),
            "walk-out-after-unattackable",
            "the caller-owned walk runs after the safe escape"
        );
    });
}

#[test]
fn aborted_walk_user_input_and_failure_preserve_the_terminal_report() {
    let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let mut aborted = report(CombatEnd::Aborted(AbortReason::Unprotected(
        crate::combat::Unprotected::NoFood,
    )));
    aborted.engaged = Some(crate::combat::ActorRef {
        kind: api::snapshot::ActorKind::Npc,
        index: 4,
    });
    aborted.engaged_npc_type = 477;
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    let mut ledger = None;

    for end in [WalkEnd::UserInput, WalkEnd::Failed] {
        let mut run = combat_test_run(
            imp_target(&data),
            None,
            Some(Arc::new(NeverStop)),
            Vec::new(),
        );
        run.phase = Phase::WalkingOutAfterAbort;
        run.last_report = Some(aborted);
        run.refresh_outcome();
        let user_input = end == WalkEnd::UserInput;
        let result = with_step_context(&snapshot, &mut ledger, 14, |cx| {
            run.on_abort_walk(
                WalkReceipt {
                    goal: None,
                    request_id: 7,
                    evidence: aborted.evidence,
                    end,
                    blocked: None,
                    detail: None,
                    refusal: None,
                    assessment: None,
                    escape: None,
                },
                cx,
            )
        });

        if user_input {
            assert!(matches!(result, Poll::Ready(Err(ActionError::UserInput))));
        } else {
            assert!(matches!(result, Poll::Ready(Err(ActionError::Blocked(_)))));
        }
        assert!(run.action.is_none());
        assert_eq!(run.target_gone_restarts, 0);
        let retained = run
            .last_outcome
            .as_ref()
            .and_then(|outcome| outcome.receipt.as_ref())
            .and_then(|receipt| receipt.as_any().downcast_ref::<CombatReceipt>())
            .expect("walk terminal must not replace the combat receipt");
        assert_eq!(retained.report.end, aborted.end);
        assert_eq!(retained.report.evidence, aborted.evidence);
        assert_eq!(retained.report.engaged, aborted.engaged);
        assert_eq!(retained.report.engaged_npc_type, aborted.engaged_npc_type);
    }
}

fn aborting_npc(index: usize, npc_type: i32, tile: api::WorldTile) -> api::snapshot::NpcView {
    let mut npc = finish_npc(index, npc_type);
    npc.tile = tile;
    npc.network = tile;
    npc.target = Some(api::snapshot::ActorTargetView {
        kind: api::snapshot::ActorKind::Player,
        index: 0,
    });
    npc.in_combat = true;
    npc
}

fn seed_abort_scene(
    snapshot: &mut GameSnapshot,
    here: api::WorldTile,
    hp: i32,
    attackers: Vec<api::snapshot::NpcView>,
    food_id: Option<i32>,
    food_name: Option<&str>,
) {
    snapshot.seed_ingame(2);
    snapshot.seed_world(api::snapshot::WorldStateView::default());
    let mut local = super::super::tests::local_player(here);
    local.player.actor.health = hp;
    local.player.actor.total_health = 40;
    local.player.actor.in_combat = !attackers.is_empty();
    local.player.actor.target = attackers.first().map(|npc| api::snapshot::ActorTargetView {
        kind: api::snapshot::ActorKind::Npc,
        index: npc.index,
    });
    snapshot.seed_local_player(local);
    snapshot.seed_inventory(
        food_id
            .zip(food_name)
            .map(|(id, name)| vec![inventory_item(id, name, 2)])
            .unwrap_or_default(),
        28,
    );
    snapshot.seed_equipment(vec![]);
    snapshot.seed_players(vec![]);
    snapshot.seed_projectiles(vec![]);
    snapshot.seed_chat_lines(vec![]);
    snapshot.seed_hitmarks(api::snapshot::HitmarksView {
        marks: [api::snapshot::HitmarkView {
            value: 0,
            kind: 0,
            cycle: 0,
        }; 4],
        loop_cycle: 0,
    });
    snapshot.seed_varps(
        (0..api::prayer::PRAYER_COUNT)
            .map(|index| api::snapshot::VarpView {
                index: api::prayer::PRAYER_VARP0 + index as i32,
                value: 0,
            })
            .chain([api::snapshot::VarpView {
                index: crate::combat::OPTION_NODEF,
                value: 0,
            }])
            .collect(),
    );
    snapshot.seed_stats(
        (0..25)
            .map(|index| api::snapshot::StatView {
                index,
                name: String::new(),
                effective: match index {
                    3 => hp,
                    5 => 43,
                    _ => 40,
                },
                base: if index == 5 { 43 } else { 40 },
                xp: 0,
                used: api::snapshot::stat_used(index as usize),
            })
            .collect(),
    );
    snapshot.seed_npcs(attackers);
}

fn guard_request_from_outbox(
    ledger: &Option<Box<crate::native::ledger::Ledger>>,
) -> crate::native::WalkRequest {
    let crate::native::HostEffect::Walk(request) =
        &ledger.as_ref().unwrap().outbox.last().unwrap().effect
    else {
        panic!("combat abort must emit its native escape walk");
    };
    let mut guard_request = super::reach::walk_request(
        request.target,
        request.radius,
        request.loc_id,
        request.required_after,
    );
    guard_request.protect = request.protect;
    guard_request.food_guard = request.food_guard;
    guard_request.allow = request.allow;
    guard_request
}

#[test]
fn prep_abort_without_engaged_actor_uses_heaviest_live_threat_and_eats_as_hp_falls() {
    use crate::combat::{facts, GuardOp, WalkGuard};

    let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let heavy = data.npc_by_config("khazard_warlord").unwrap();
    let light = data
        .npc_names()
        .unwrap()
        .rows
        .iter()
        .find(|row| facts::npc_max_hit(row) == Some(4))
        .expect("selected content includes a known-hit lower threat");
    assert!(facts::npc_max_hit(heavy).unwrap() > facts::npc_max_hit(light).unwrap());
    let lobster = data.item_by_alias("lobster").unwrap();
    let here = api::WorldTile {
        x: 3000,
        z: 3000,
        level: 0,
    };
    let mut run = combat_test_run(
        imp_target(&data),
        None,
        Some(Arc::new(NeverStop)),
        Vec::new(),
    );
    let attack_animation = run
        .tables
        .selected()
        .style_seqs()
        .first()
        .expect("selected combat data includes attack animations")
        .seq_id;
    let mut attackers = vec![
        aborting_npc(
            7,
            heavy.id,
            api::WorldTile {
                x: here.x + 1,
                ..here
            },
        ),
        aborting_npc(
            8,
            light.id,
            api::WorldTile {
                x: here.x - 1,
                ..here
            },
        ),
    ];
    for attacker in &mut attackers {
        attacker.animation = attack_animation;
    }
    let mut snapshot = GameSnapshot::new();
    seed_abort_scene(
        &mut snapshot,
        here,
        40,
        attackers,
        Some(lobster.id),
        lobster.name.as_deref(),
    );
    run.protect = true;
    let aborted = report(CombatEnd::Aborted(AbortReason::PrepFailed(
        crate::combat::PrepItem::Ammo,
    )));
    let mut ledger = None;

    assert!(with_step_context(&snapshot, &mut ledger, 12, |cx| {
        run.on_combat_report(aborted, cx)
    })
    .is_pending());
    let request = guard_request_from_outbox(&ledger);
    assert!(request.protect, "abort walks must arm WalkGuard protection");
    assert!(
        request.allow.prayer,
        "abort walk must retain the step's prayer permission"
    );
    assert!(
        request.allow.food,
        "abort WalkGuard must retain food permission"
    );
    assert!(
        request.target.x < here.x,
        "the heavier Warlord must be the retreat reference"
    );

    let mut guard = with_step_context(&snapshot, &mut ledger, 13, |cx| {
        WalkGuard::begin_with(&request, &cx.tick.cx.snapshot(), Arc::clone(&run.tables))
            .expect("Prayer-43 abort walk admits WalkGuard")
    });
    let first = with_step_context(&snapshot, &mut ledger, 13, |cx| {
        let view = cx.tick.cx.snapshot();
        let op = guard.tick(
            &view,
            cx.tick.cx.condition(),
            crate::survival::CurePoison::WhenNeeded,
        );
        if let Some(op) = op.as_ref() {
            guard.admitted(op, &view);
        }
        op
    });
    assert!(matches!(first, Some(GuardOp::IfButton { .. })));

    let mut attackers = vec![
        aborting_npc(
            7,
            heavy.id,
            api::WorldTile {
                x: here.x + 1,
                ..here
            },
        ),
        aborting_npc(
            8,
            light.id,
            api::WorldTile {
                x: here.x - 1,
                ..here
            },
        ),
    ];
    for attacker in &mut attackers {
        attacker.animation = attack_animation;
    }
    seed_abort_scene(
        &mut snapshot,
        here,
        1,
        attackers,
        Some(lobster.id),
        lobster.name.as_deref(),
    );
    let falling_hp = with_step_context(&snapshot, &mut ledger, 14, |cx| {
        guard.tick(
            &cx.tick.cx.snapshot(),
            cx.tick.cx.condition(),
            crate::survival::CurePoison::WhenNeeded,
        )
    });
    assert!(matches!(falling_hp, Some(GuardOp::Eat { .. })));
}

/// First listed attacker wins a weight tie. The old unknown max hit was that tie.
fn first_strictly_heavier(weights: &[i32]) -> usize {
    weights.iter().enumerate().skip(1).fold(
        0,
        |best, (index, weight)| {
            if *weight > weights[best] {
                index
            } else {
                best
            }
        },
    )
}

#[test]
fn retreat_fallback_picks_a_cow_over_a_first_listed_chicken() {
    use crate::combat::facts;

    let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let chicken = data.npc_by_config("chicken").unwrap();
    let cow = data.npc_by_config("cow").unwrap();
    assert_eq!(facts::npc_max_hit(chicken), Some(0));
    assert_eq!(facts::npc_max_hit(cow), Some(1));
    let chicken_weight = i32::from(facts::npc_max_hit(chicken).unwrap()) * 10;
    let cow_weight = i32::from(facts::npc_max_hit(cow).unwrap()) * 10;
    assert_eq!((chicken_weight, cow_weight), (0, 10));
    // The chicken is listed first. Old unknown weights were 0 and 0, so the
    // chicken stayed the reference and the retreat went the other way.
    assert_eq!(first_strictly_heavier(&[0, 0]), 0);
    assert_eq!(first_strictly_heavier(&[chicken_weight, cow_weight]), 1);

    let here = api::WorldTile {
        x: 3000,
        z: 3000,
        level: 0,
    };
    let mut run = combat_test_run(
        imp_target(&data),
        None,
        Some(Arc::new(NeverStop)),
        Vec::new(),
    );
    let attack_animation = run
        .tables
        .selected()
        .style_seqs()
        .first()
        .expect("selected combat data includes attack animations")
        .seq_id;
    let mut attackers = vec![
        aborting_npc(
            7,
            chicken.id,
            api::WorldTile {
                x: here.x - 1,
                ..here
            },
        ),
        aborting_npc(
            8,
            cow.id,
            api::WorldTile {
                x: here.x + 1,
                ..here
            },
        ),
    ];
    for attacker in &mut attackers {
        attacker.animation = attack_animation;
    }
    let mut snapshot = GameSnapshot::new();
    seed_abort_scene(&mut snapshot, here, 40, attackers, None, None);
    let aborted = report(CombatEnd::Aborted(AbortReason::PrepFailed(
        crate::combat::PrepItem::Ammo,
    )));
    let mut ledger = None;
    assert!(with_step_context(&snapshot, &mut ledger, 12, |cx| {
        run.on_combat_report(aborted, cx)
    })
    .is_pending());
    let request = guard_request_from_outbox(&ledger);
    assert!(
        request.target.x < here.x,
        "retreat must run from the cow. The old unknown weight keeps the first-listed chicken and walks the other way"
    );
}

#[test]
fn aborted_walk_to_authored_stand_arms_walk_guard() {
    let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let warlord = data.npc_by_config("khazard_warlord").unwrap();
    let here = api::WorldTile {
        x: 3000,
        z: 3000,
        level: 0,
    };
    let stand = api::WorldTile {
        x: 3200,
        z: 3200,
        level: 0,
    };
    let mut snapshot = GameSnapshot::new();
    seed_abort_scene(
        &mut snapshot,
        here,
        40,
        vec![aborting_npc(
            7,
            warlord.id,
            api::WorldTile {
                x: here.x + 1,
                ..here
            },
        )],
        None,
        None,
    );
    let mut run = combat_test_run(
        imp_target(&data),
        Some(stand),
        Some(Arc::new(NeverStop)),
        Vec::new(),
    );
    run.protect = true;
    let mut aborted = report(CombatEnd::Aborted(AbortReason::PrepFailed(
        crate::combat::PrepItem::Ammo,
    )));
    aborted.engaged = Some(crate::combat::ActorRef {
        kind: api::snapshot::ActorKind::Npc,
        index: 7,
    });
    aborted.engaged_npc_type = warlord.id;
    let mut ledger = None;

    assert!(with_step_context(&snapshot, &mut ledger, 12, |cx| {
        run.on_combat_report(aborted, cx)
    })
    .is_pending());
    let crate::native::HostEffect::Walk(request) =
        &ledger.as_ref().unwrap().outbox.last().unwrap().effect
    else {
        panic!("combat abort must emit its native stand walk");
    };
    assert_eq!(request.target, stand);
    assert!(request.protect);
    assert!(request.allow.food);
    assert!(request.allow.prayer);
}

#[test]
fn prep_abort_without_live_attacker_keeps_legacy_blocked_handoff() {
    let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let here = api::WorldTile {
        x: 3000,
        z: 3000,
        level: 0,
    };
    let mut snapshot = GameSnapshot::new();
    seed_abort_scene(&mut snapshot, here, 40, vec![], None, None);
    let mut run = combat_test_run(
        imp_target(&data),
        None,
        Some(Arc::new(NeverStop)),
        Vec::new(),
    );
    let mut ledger = None;

    let result = with_step_context(&snapshot, &mut ledger, 12, |cx| {
        run.on_combat_report(
            report(CombatEnd::Aborted(AbortReason::PrepFailed(
                crate::combat::PrepItem::Ammo,
            ))),
            cx,
        )
    });
    assert!(matches!(result, Poll::Ready(Err(ActionError::Blocked(_)))));
    assert!(ledger
        .as_ref()
        .is_none_or(|ledger| ledger.outbox.is_empty()));
}

#[test]
fn prep_abort_escapes_attack_animation_without_health_bar() {
    let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let warlord = data.npc_by_config("khazard_warlord").unwrap();
    let here = api::WorldTile {
        x: 3000,
        z: 3000,
        level: 0,
    };
    let mut run = combat_test_run(
        imp_target(&data),
        None,
        Some(Arc::new(NeverStop)),
        Vec::new(),
    );
    let attack_animation = run
        .tables
        .selected()
        .style_seqs()
        .first()
        .expect("selected combat data includes attack animations")
        .seq_id;
    let mut attacker = aborting_npc(
        7,
        warlord.id,
        api::WorldTile {
            x: here.x + 1,
            ..here
        },
    );
    attacker.in_combat = false;
    attacker.animation = attack_animation;
    attacker.health = 0;
    attacker.total_health = 0;
    let mut snapshot = GameSnapshot::new();
    seed_abort_scene(&mut snapshot, here, 40, vec![attacker], None, None);
    let mut ledger = None;

    assert!(with_step_context(&snapshot, &mut ledger, 12, |cx| {
        run.on_combat_report(
            report(CombatEnd::Aborted(AbortReason::PrepFailed(
                crate::combat::PrepItem::Ammo,
            ))),
            cx,
        )
    })
    .is_pending());
    let crate::native::HostEffect::Walk(request) =
        &ledger.as_ref().unwrap().outbox.last().unwrap().effect
    else {
        panic!("an attacking NPC without a health bar must still trigger escape");
    };
    assert!(request.target.x < here.x);
}

#[test]
fn abort_retreat_direction_uses_network_tiles_not_rendered_poses() {
    let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let warlord = data.npc_by_config("khazard_warlord").unwrap();
    let here = api::WorldTile {
        x: 3000,
        z: 3000,
        level: 0,
    };
    for engaged in [false, true] {
        let mut run = combat_test_run(
            imp_target(&data),
            None,
            Some(Arc::new(NeverStop)),
            Vec::new(),
        );
        let attack_animation = run
            .tables
            .selected()
            .style_seqs()
            .first()
            .expect("selected combat data includes attack animations")
            .seq_id;
        // The Warlord's packet-time tile is east of the player, while its
        // interpolated rendered pose still lags to the west.
        let mut attacker = aborting_npc(
            7,
            warlord.id,
            api::WorldTile {
                x: here.x + 1,
                ..here
            },
        );
        attacker.tile = api::WorldTile {
            x: here.x - 1,
            z: here.z + 1,
            ..here
        };
        attacker.animation = attack_animation;
        let mut snapshot = GameSnapshot::new();
        seed_abort_scene(&mut snapshot, here, 40, vec![attacker], None, None);
        let mut aborted = report(CombatEnd::Aborted(AbortReason::PrepFailed(
            crate::combat::PrepItem::Ammo,
        )));
        if engaged {
            aborted.engaged = Some(crate::combat::ActorRef {
                kind: api::snapshot::ActorKind::Npc,
                index: 7,
            });
            aborted.engaged_npc_type = warlord.id;
        }
        let mut ledger = None;

        assert!(with_step_context(&snapshot, &mut ledger, 12, |cx| {
            run.on_combat_report(aborted, cx)
        })
        .is_pending());
        let crate::native::HostEffect::Walk(request) =
            &ledger.as_ref().unwrap().outbox.last().unwrap().effect
        else {
            panic!("a live attacker must trigger the abort walk");
        };
        assert!(
            request.target.x < here.x,
            "engaged={engaged}: retreat must run west, away from the network tile"
        );
        assert_eq!(
            request.target.z, here.z,
            "engaged={engaged}: the rendered z offset must not steer the retreat"
        );
    }
}

#[test]
fn failed_abort_walk_holds_protection_at_high_hp_while_attacker_is_live() {
    let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let warlord = data.npc_by_config("khazard_warlord").unwrap();
    let lobster = data.item_by_alias("lobster").unwrap();
    let here = api::WorldTile {
        x: 3000,
        z: 3000,
        level: 0,
    };
    let mut run = combat_test_run(
        imp_target(&data),
        None,
        Some(Arc::new(NeverStop)),
        Vec::new(),
    );
    run.protect = true;
    let attack_animation = run
        .tables
        .selected()
        .style_seqs()
        .first()
        .expect("selected combat data includes attack animations")
        .seq_id;
    let mut attacker = aborting_npc(
        7,
        warlord.id,
        api::WorldTile {
            x: here.x + 1,
            ..here
        },
    );
    attacker.animation = attack_animation;
    let mut snapshot = GameSnapshot::new();
    seed_abort_scene(
        &mut snapshot,
        here,
        39,
        vec![attacker],
        Some(lobster.id),
        lobster.name.as_deref(),
    );
    let mut ledger = None;
    assert!(with_step_context(&snapshot, &mut ledger, 12, |cx| {
        run.on_combat_report(
            report(CombatEnd::Aborted(AbortReason::PrepFailed(
                crate::combat::PrepItem::Ammo,
            ))),
            cx,
        )
    })
    .is_pending());
    let _ = fail_escape_walks_until_hold(&mut run, &snapshot, &mut ledger, 13);

    let (protect_varp, protect_button_com) = run
        .tables
        .prayer(crate::combat::tables::PrayerRole::Protect, 2)
        .map(|prayer| (prayer.varp, prayer.button_com))
        .expect("selected content includes Protect from Melee");
    assert!(with_step_context(&snapshot, &mut ledger, 14, |cx| run.poll(cx)).is_pending());
    let raised = ledger.as_ref().unwrap().outbox.iter().find_map(|action| {
        matches!(
            action.effect,
            crate::native::HostEffect::Interaction(crate::shim::InteractReq::IfButton {
                component_id
            }) if component_id == protect_button_com
        )
        .then_some(action.request_id.get())
    });
    let Some(raised) = raised else {
        panic!("the abort hold must raise its allowed protect prayer");
    };
    let protect_clicks = ledger
        .as_ref()
        .unwrap()
        .outbox
        .iter()
        .filter(|action| {
            matches!(
                action.effect,
                crate::native::HostEffect::Interaction(crate::shim::InteractReq::IfButton {
                    component_id
                }) if component_id == protect_button_com
            )
        })
        .count();
    assert_eq!(
        protect_clicks, 1,
        "the hold must not immediately retire protection while the attacker is live"
    );

    snapshot.seed_varps(
        (0..api::prayer::PRAYER_COUNT)
            .map(|index| {
                let varp = api::prayer::PRAYER_VARP0 + index as i32;
                api::snapshot::VarpView {
                    index: varp,
                    value: i32::from(varp == protect_varp),
                }
            })
            .chain([api::snapshot::VarpView {
                index: crate::combat::OPTION_NODEF,
                value: 0,
            }])
            .collect(),
    );
    ledger.as_mut().unwrap().outbox.clear();
    ledger.as_mut().unwrap().interaction = Some(crate::native::InteractionReceipt {
        request_id: raised,
        evidence: EvidenceStamp {
            run: RunKey {
                slot: 1,
                run: 1,
                session: 1,
            },
            tick: 14,
            sequence: 14,
        },
        accepted: true,
        chat_since: 0,
    });
    for tick in 15..=26 {
        assert!(
            with_step_context(&snapshot, &mut ledger, tick, |cx| run.poll(cx)).is_pending(),
            "high HP and a long-running live threat must not end the hold at tick {tick}"
        );
    }
    assert!(
        ledger.as_ref().unwrap().outbox.iter().all(|action| {
            !matches!(
                action.effect,
                crate::native::HostEffect::Interaction(crate::shim::InteractReq::IfButton {
                    component_id
                }) if component_id == protect_button_com
            )
        }),
        "protection must not be dropped while the attacker remains live"
    );

    // Cancelling the hold hands its owned protection to the runner's cleanup.
    assert!(
        StepRun::prayer_cleanup(&run).contains(protect_varp),
        "the live hold reports its observed protect prayer as owned"
    );
    let mut actions = crate::native::NativeActions { _private: () };
    run.cancel(&mut actions);
    assert!(run.action.is_none());
    assert!(
        StepRun::prayer_cleanup(&run).contains(protect_varp),
        "hold cancellation must retain the protect prayer for release"
    );
}

#[test]
fn failed_abort_walk_does_not_retry_named_tiles_after_food_exhaustion() {
    let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let warlord = data.npc_by_config("khazard_warlord").unwrap();
    let here = api::WorldTile {
        x: 3000,
        z: 3000,
        level: 0,
    };
    let mut run = combat_test_run(
        imp_target(&data),
        None,
        Some(Arc::new(NeverStop)),
        Vec::new(),
    );
    let attack_animation = run
        .tables
        .selected()
        .style_seqs()
        .first()
        .expect("selected combat data includes attack animations")
        .seq_id;
    let mut attacker = aborting_npc(
        7,
        warlord.id,
        api::WorldTile {
            x: here.x + 1,
            ..here
        },
    );
    attacker.animation = attack_animation;
    let mut snapshot = GameSnapshot::new();
    seed_abort_scene(&mut snapshot, here, 1, vec![attacker], None, None);
    let mut ledger = None;
    assert!(with_step_context(&snapshot, &mut ledger, 12, |cx| {
        run.on_combat_report(
            report(CombatEnd::Aborted(AbortReason::PrepFailed(
                crate::combat::PrepItem::Ammo,
            ))),
            cx,
        )
    })
    .is_pending());
    let earlier_goal_sets = fail_escape_walks_until_hold(&mut run, &snapshot, &mut ledger, 13);
    assert!(
        !earlier_goal_sets.is_empty(),
        "the guarded pass names goals before food runs out"
    );
    assert!(with_step_context(&snapshot, &mut ledger, 14, |cx| run.poll(cx)).is_pending());
    assert!(with_step_context(&snapshot, &mut ledger, 15, |cx| run.poll(cx)).is_pending());
    let exhausted = last_walk(&ledger);
    assert!(!exhausted.food_guard && !exhausted.allow_food);
    assert!(
        earlier_goal_sets.iter().all(|goals| {
            goals_of(&exhausted)
                .as_slice()
                .iter()
                .all(|goal| !goals.contains(goal))
        }),
        "food exhaustion starts a request with only untried tiles"
    );
}

/// Every escape walk fails. A pass makes at most one request per destination
/// tier that still has an untried goal: the authored stand, tiles beyond the
/// chase envelope, and farther-away tiles. Failed tiles remain excluded when
/// food runs out and a second pass starts. Fixed pass counters bound retries,
/// but exhaustion ends earlier when no new tile can be named.
#[test]
fn failed_escape_recovery_is_bounded_and_preserves_the_last_failure() {
    let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let warlord = data.npc_by_config("khazard_warlord").unwrap();
    let here = api::WorldTile {
        x: 3000,
        z: 3000,
        level: 0,
    };
    let far_stand = api::WorldTile {
        x: 3000,
        z: 3200,
        level: 0,
    };
    for (stand, pass) in [(Some(far_stand), 3), (None, 2)] {
        let mut run = combat_test_run(
            imp_target(&data),
            stand,
            Some(Arc::new(NeverStop)),
            Vec::new(),
        );
        let mut attacker = aborting_npc(
            7,
            warlord.id,
            api::WorldTile {
                x: here.x + 1,
                ..here
            },
        );
        attacker.animation = live_attack_animation(&run);
        let mut snapshot = GameSnapshot::new();
        seed_abort_scene(&mut snapshot, here, 1, vec![attacker], None, None);
        let mut ledger = None;
        assert!(with_step_context(&snapshot, &mut ledger, 12, |cx| {
            run.on_combat_report(
                report(CombatEnd::Aborted(AbortReason::PrepFailed(
                    crate::combat::PrepItem::Ammo,
                ))),
                cx,
            )
        })
        .is_pending());

        let mut walks = Vec::new();
        let mut tick = 13;
        let result = loop {
            if pending_walk(&ledger).is_some() {
                walks.push(last_walk(&ledger));
                let detail = format!("route fixture failure {}", walks.len());
                post_walk_end(&mut ledger, tick, WalkEnd::Failed, Some(&detail));
            } else {
                // The guarded hold runs on until the food is gone.
                tick += 1;
            }
            if let Poll::Ready(result) =
                with_step_context(&snapshot, &mut ledger, tick, |cx| run.poll(cx))
            {
                break result;
            }
            assert!(
                walks.len() <= 2 * escape::ESCAPE_PASS_ATTEMPTS && tick < 100,
                "the recovery must end: {} walks by tick {tick}",
                walks.len()
            );
        };

        let first_pass_count = walks.iter().take_while(|walk| walk.food_guard).count();
        assert_eq!(
            first_pass_count, pass,
            "the guarded pass requests each available tier"
        );
        let exhausted_count = walks.len() - first_pass_count;
        assert!(
            exhausted_count > 0 && exhausted_count <= pass,
            "stand={stand:?}: the exhausted pass requests only tiers with untried goals"
        );
        let mut earlier_goal_sets = Vec::<Vec<api::WorldTile>>::new();
        for walk in &walks {
            let goals = goals_of(walk).as_slice();
            assert!(
                earlier_goal_sets
                    .iter()
                    .all(|previous| { goals.iter().all(|goal| !previous.contains(goal)) }),
                "a request does not repeat a tile named by an earlier failed request"
            );
            earlier_goal_sets.push(goals.to_vec());
        }
        for (index, walk) in walks.iter().enumerate() {
            assert_eq!(
                walk.food_guard,
                index < pass,
                "walk {index}: only the first pass runs under the food guard"
            );
        }
        for (pass_index, chunk) in walks.chunks(pass).enumerate() {
            let preferred = chunk
                .iter()
                .map(|walk| goals_of(walk).preferred().to_vec())
                .collect::<Vec<_>>();
            for (index, tier) in preferred.iter().enumerate() {
                assert!(
                    !preferred[..index].contains(tier),
                    "a pass names each tier as preferred once"
                );
            }
            assert_eq!(
                chunk[0]
                    .goals
                    .as_ref()
                    .is_some_and(|goals| goals.preferred().contains(&far_stand)),
                stand.is_some() && pass_index == 0,
                "the authored stand leads only the first pass when the Path names one"
            );
        }
        let last = format!("route fixture failure {}", walks.len());
        assert!(
            matches!(&result, Err(ActionError::Blocked(reason)) if reason.as_ref() == last),
            "the step must end with the last walk failure"
        );
        assert!(ledger.as_ref().unwrap().outbox.is_empty());
    }
}

/// An open posted flood of `radius` tiles around `here`: every step is clear
/// and a tile's rank is its Chebyshev distance from `here`, so its walking
/// depth equals its rank.
fn open_flood(here: api::WorldTile, radius: i32) -> api::query::ReachQueryView {
    let side = 2 * radius + 1;
    let n = (side * side) as usize;
    let mut exact_rank = vec![0u16; n];
    for lx in 0..side {
        for lz in 0..side {
            exact_rank[(lx * side + lz) as usize] =
                (lx - radius).abs().max((lz - radius).abs()) as u16;
        }
    }
    api::query::ReachQueryView {
        available: true,
        base_x: here.x - radius,
        base_z: here.z - radius,
        level: here.level,
        width: side,
        height: side,
        walkable: vec![u32::MAX; n.div_ceil(32)],
        reachable: vec![u32::MAX; n.div_ceil(32)],
        reachable_adj: vec![u32::MAX; n.div_ceil(32)],
        exact_rank,
        adjacent_rank: vec![0; n],
        step: vec![u8::MAX; n],
        canlight: Vec::new(),
    }
}

fn with_step_context_reach<R>(
    snapshot: &GameSnapshot,
    reach: &api::query::ReachQueryView,
    ledger: &mut TestLedger,
    tick: u64,
    f: impl FnOnce(&mut StepContext<'_, '_>) -> R,
) -> R {
    super::super::tests::with_tick_reach(snapshot, reach, ledger, tick, |native| {
        let quests = QuestCatalog::empty();
        let required_after = native.cx.evidence();
        let banks = Arc::new(api::named_banks::NamedBankFacts::empty());
        f(&mut StepContext {
            tick: native,
            quests: &quests,
            progress: &[],
            required_after,
            banks: &banks,
            choices: &crate::quester::choices::QuestChoices::default(),
        })
    })
}

/// The walking depth of an escape walk in an [`open_flood`] around `here`:
/// the Chebyshev distance to the nearest goal of its preferred tier.
fn open_flood_depth(here: api::WorldTile, walk: &WalkOut) -> u64 {
    goals_of(walk)
        .preferred()
        .iter()
        .map(|goal| tile_distance(here, *goal))
        .min()
        .expect("a tier has goals") as u64
}

/// The host never posts a terminal receipt for any escape walk, so only the
/// observed-tick budget can end one. Each walk is abandoned after its flood
/// depth plus the slack, the next tier starts while the attacker is live, and
/// the whole recovery ends within the budgets of its requests.
#[test]
fn escape_walks_that_never_settle_end_on_their_observed_tick_budget() {
    const FLOOD_RADIUS: i32 = 70;
    let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let warlord = data.npc_by_config("khazard_warlord").unwrap();
    let here = api::WorldTile {
        x: 3000,
        z: 3000,
        level: 0,
    };
    let far_stand = api::WorldTile {
        x: 3000,
        z: 3060,
        level: 0,
    };
    let reach = open_flood(here, FLOOD_RADIUS);
    for (stand, pass) in [(Some(far_stand), 3), (None, 2)] {
        let mut run = combat_test_run(
            imp_target(&data),
            stand,
            Some(Arc::new(NeverStop)),
            Vec::new(),
        );
        let mut attacker = aborting_npc(
            7,
            warlord.id,
            api::WorldTile {
                x: here.x + 1,
                ..here
            },
        );
        attacker.animation = live_attack_animation(&run);
        let mut snapshot = GameSnapshot::new();
        seed_abort_scene(&mut snapshot, here, 1, vec![attacker], None, None);
        let mut ledger = None;
        assert!(
            with_step_context_reach(&snapshot, &reach, &mut ledger, 12, |cx| {
                run.on_combat_report(
                    report(CombatEnd::Aborted(AbortReason::PrepFailed(
                        crate::combat::PrepItem::Ammo,
                    ))),
                    cx,
                )
            })
            .is_pending()
        );

        // Each entry is the tick a walk was requested and the walk.
        let mut walks = vec![(12, last_walk(&ledger))];
        let mut tick = 12;
        let bound = 12 + 2 * pass as u64 * (FLOOD_RADIUS as u64 + escape::ESCAPE_SLACK_TICKS) + 20;
        let result = loop {
            tick += 1;
            if let Poll::Ready(result) =
                with_step_context_reach(&snapshot, &reach, &mut ledger, tick, |cx| run.poll(cx))
            {
                break result;
            }
            if let Some(walk) = pending_walk(&ledger) {
                if walk.request_id != walks.last().unwrap().1.request_id {
                    walks.push((tick, walk));
                }
            }
            assert!(
                tick <= bound,
                "stand={stand:?}: the recovery must end by tick {bound}, with {} walks",
                walks.len()
            );
        };

        let first_pass_count = walks.iter().take_while(|(_, walk)| walk.food_guard).count();
        assert_eq!(
            first_pass_count, pass,
            "the guarded pass requests each available tier"
        );
        assert!(
            walks.len() > first_pass_count && walks.len() <= 2 * pass,
            "stand={stand:?}: the exhausted pass requests only tiers with untried goals"
        );
        for chunk in walks.chunks(pass) {
            for pair in chunk.windows(2) {
                assert_eq!(
                    pair[1].0 - pair[0].0,
                    open_flood_depth(here, &pair[0].1) + escape::ESCAPE_SLACK_TICKS,
                    "a walk is abandoned after its flood depth plus the slack, in observed ticks"
                );
            }
            let preferred = chunk
                .iter()
                .map(|(_, walk)| goals_of(walk).preferred().to_vec())
                .collect::<Vec<_>>();
            for (index, tier) in preferred.iter().enumerate() {
                assert!(
                    !preferred[..index].contains(tier),
                    "each stalled walk is followed by a request for the next tier"
                );
            }
        }
        for (index, (_, walk)) in walks.iter().enumerate() {
            assert_eq!(walk.food_guard, index < pass, "walk {index}");
        }
        assert!(
            matches!(
                &result,
                Err(ActionError::Blocked(reason)) if reason.as_ref() == ABORT_ESCAPE_STALLED_REASON
            ),
            "the step ends with the stalled-walk failure: {}",
            describe_poll(&Poll::Ready(result))
        );
    }
}

/// Without a measured depth (no flood, or none of the goals in it) the walk
/// has no tick budget and no clock. It ends by its receipt or by the existing
/// ten-minute walk deadline (`FIX-ABORT-ESCAPE/r4/MAIN-RULING.md`).
#[test]
fn an_unmeasured_escape_walk_has_no_tick_budget() {
    let geometry = escape::EscapeGeometry {
        here: M4_HERE,
        threat: Some(M4_TREE),
        stand: None,
        chase_range: 14,
    };
    let no_flood = escape::AbortEscape::new()
        .next_goals(&geometry, None, None)
        .expect("tiers beyond the envelope remain");
    assert_eq!(no_flood.budget_ticks, None);
    let small = open_flood(M4_HERE, 1);
    let no_goal_in_flood = escape::AbortEscape::new()
        .next_goals(&geometry, None, Some(&small))
        .expect("tiers beyond the envelope remain");
    assert_eq!(no_goal_in_flood.budget_ticks, None);

    let mut escape = escape::AbortEscape::new();
    escape.start_walk(10, no_flood.budget_ticks);
    assert!(
        !escape.walk_expired(10 + 10_000_000),
        "a walk with no tick budget never expires on observed ticks"
    );
}

/// A guarded escape walk that never settles, with food in the pack and the
/// hitpoints above the eat line when it starts. The attacker keeps hitting,
/// the walk guard keeps eating while the walk waits, the observed-tick budget
/// ends the walk with food and hitpoints left, the next tier is requested
/// under the same guard, and the whole first pass stays inside the sum of its
/// budgets, after which the hold takes over.
#[test]
fn a_stalled_guarded_escape_walk_keeps_eating_and_ends_with_supplies_left() {
    use crate::combat::{facts, GuardOp, WalkGuard};
    use crate::survival::CurePoison;

    /// An attack lands every this many ticks.
    const ATTACK_TICKS: u64 = 4;
    const FULL_HP: i32 = 40;
    const FLOOD_RADIUS: i32 = 70;

    let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let warlord = data.npc_by_config("khazard_warlord").unwrap();
    let lobster = data.item_by_alias("lobster").unwrap();
    let lobster_name = lobster.name.clone().expect("lobster has a name");
    let hit = i32::from(facts::npc_max_hit(warlord).expect("the warlord's max hit is known"));
    let here = api::WorldTile {
        x: 3000,
        z: 3000,
        level: 0,
    };
    let mut run = combat_test_run(
        imp_target(&data),
        None,
        Some(Arc::new(NeverStop)),
        Vec::new(),
    );
    let heal = run.tables.food(lobster.id).expect("lobster is food").heal;
    let attack_animation = live_attack_animation(&run);
    let reach = open_flood(here, FLOOD_RADIUS);
    let scene = |snapshot: &mut GameSnapshot, hp: i32, food: i32| {
        let mut attacker = aborting_npc(
            7,
            warlord.id,
            api::WorldTile {
                x: here.x + 1,
                ..here
            },
        );
        attacker.animation = attack_animation;
        seed_abort_scene(snapshot, here, hp, vec![attacker], None, None);
        snapshot.seed_inventory(
            if food > 0 {
                vec![inventory_item(lobster.id, &lobster_name, food)]
            } else {
                Vec::new()
            },
            28,
        );
    };

    let (mut hp, mut food, mut eats) = (FULL_HP, 28, 0u32);
    let mut snapshot = GameSnapshot::new();
    scene(&mut snapshot, hp, food);
    let mut ledger = None;
    assert!(
        with_step_context_reach(&snapshot, &reach, &mut ledger, 12, |cx| {
            run.on_combat_report(
                report(CombatEnd::Aborted(AbortReason::PrepFailed(
                    crate::combat::PrepItem::Ammo,
                ))),
                cx,
            )
        })
        .is_pending()
    );
    let guard_for_last_walk =
        |run: &CombatRun, snapshot: &GameSnapshot, ledger: &mut TestLedger, tick: u64| {
            let request = guard_request_from_outbox(ledger);
            assert!(request.food_guard && !request.protect, "a food-only guard");
            with_step_context_reach(snapshot, &reach, ledger, tick, |cx| {
                WalkGuard::begin_food_only_with(
                    &request,
                    &cx.tick.cx.snapshot(),
                    Arc::clone(&run.tables),
                )
                .expect("the escape walk admits the food guard")
            })
        };
    let mut guard = guard_for_last_walk(&run, &snapshot, &mut ledger, 12);

    // Each entry: tick requested, the walk, meals eaten so far, food left.
    let mut walks = vec![(12u64, last_walk(&ledger), eats, food)];
    let mut tick = 12u64;
    let budget_sum = |walks: &[(u64, WalkOut, u32, i32)]| {
        walks
            .iter()
            .map(|(_, walk, _, _)| open_flood_depth(here, walk) + escape::ESCAPE_SLACK_TICKS)
            .sum::<u64>()
    };
    let handoff = loop {
        tick += 1;
        if tick.is_multiple_of(ATTACK_TICKS) {
            hp -= hit;
        }
        assert!(hp > 0, "the bot dies at tick {tick} with {food} food left");
        scene(&mut snapshot, hp, food);
        let op = with_step_context_reach(&snapshot, &reach, &mut ledger, tick, |cx| {
            let view = cx.tick.cx.snapshot();
            let op = guard.tick(&view, cx.tick.cx.condition(), CurePoison::WhenNeeded);
            if let Some(op) = op.as_ref() {
                guard.admitted(op, &view);
            }
            op
        });
        if tick == 13 {
            assert!(op.is_none(), "the first tick is above the eat line: {op:?}");
        }
        if matches!(op, Some(GuardOp::Eat { .. })) {
            eats += 1;
            food -= 1;
            hp = (hp + heal).min(FULL_HP);
        }
        let polled =
            with_step_context_reach(&snapshot, &reach, &mut ledger, tick, |cx| run.poll(cx));
        assert!(
            polled.is_pending(),
            "tick {tick}: {}",
            describe_poll(&polled)
        );
        if matches!(run.phase, Phase::HoldingAfterAbort) {
            break tick;
        }
        if let Some(walk) = pending_walk(&ledger) {
            if walk.request_id != walks.last().unwrap().1.request_id {
                walks.push((tick, walk, eats, food));
                guard = guard_for_last_walk(&run, &snapshot, &mut ledger, tick);
            }
        }
        assert!(
            tick <= 12 + budget_sum(&walks) + 40,
            "the first pass must end within its budgets: {} walks at tick {tick}",
            walks.len()
        );
    };

    assert_eq!(walks.len(), 2, "one request for each tier without a stand");
    let (first_tick, first, _, _) = &walks[0];
    let (second_tick, second, eats_at_switch, food_at_switch) = &walks[1];
    assert_eq!(
        second_tick - first_tick,
        open_flood_depth(here, first) + escape::ESCAPE_SLACK_TICKS,
        "the stalled walk ends on its observed-tick budget"
    );
    assert!(
        *eats_at_switch >= 2,
        "the guard keeps eating while the walk waits: {eats_at_switch} meals"
    );
    assert!(
        *food_at_switch > 0,
        "the next tier starts while food remains"
    );
    assert!(
        first.food_guard && second.food_guard,
        "both walks are guarded"
    );
    assert!(
        goals_of(first).fallback().is_empty() && goals_of(second).fallback().is_empty(),
        "each request sends one tier"
    );
    assert!(
        goals_of(second)
            .preferred()
            .iter()
            .all(|goal| !goals_of(first).preferred().contains(goal)),
        "the retry uses different goals"
    );
    assert_eq!(
        handoff,
        12 + budget_sum(&walks),
        "the pass ends on the sum of its observed-tick budgets, then the hold starts"
    );
    assert!(hp > 0 && food > 0, "supplies remain at the hand-off");
}

/// With a live flood, the budget is the walking depth to the nearest goal of
/// the preferred tier plus the slack, far below the router's bound.
#[test]
fn a_measured_escape_walk_is_abandoned_after_its_flood_depth_plus_the_slack() {
    let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let warlord = data.npc_by_config("khazard_warlord").unwrap();
    let here = api::WorldTile {
        x: 3000,
        z: 3000,
        level: 0,
    };
    let mut run = combat_test_run(
        imp_target(&data),
        None,
        Some(Arc::new(NeverStop)),
        Vec::new(),
    );
    let mut attacker = aborting_npc(
        7,
        warlord.id,
        api::WorldTile {
            x: here.x + 1,
            ..here
        },
    );
    attacker.animation = live_attack_animation(&run);
    let mut snapshot = GameSnapshot::new();
    seed_abort_scene(&mut snapshot, here, 1, vec![attacker], None, None);
    let reach = open_flood(here, 70);
    let mut ledger = None;
    assert!(
        with_step_context_reach(&snapshot, &reach, &mut ledger, 12, |cx| {
            run.on_combat_report(
                report(CombatEnd::Aborted(AbortReason::PrepFailed(
                    crate::combat::PrepItem::Ammo,
                ))),
                cx,
            )
        })
        .is_pending()
    );
    let first = last_walk(&ledger);
    let depth = open_flood_depth(here, &first);
    let budget = depth + escape::ESCAPE_SLACK_TICKS;
    assert!(depth > 0, "the budget is measured");

    let mut tick = 12;
    loop {
        tick += 1;
        assert!(
            with_step_context_reach(&snapshot, &reach, &mut ledger, tick, |cx| run.poll(cx))
                .is_pending()
        );
        if last_walk(&ledger).request_id != first.request_id {
            break;
        }
        assert!(tick <= 12 + budget, "the walk must expire");
    }
    assert_eq!(
        tick,
        12 + budget,
        "the walk is abandoned after its flood depth plus the slack"
    );
    let second = last_walk(&ledger);
    assert!(goals_of(&second).fallback().is_empty());
    assert!(
        goals_of(&second)
            .preferred()
            .iter()
            .all(|goal| !goals_of(&first).preferred().contains(goal)),
        "the next request chooses a distinct tier"
    );
    assert!(second.food_guard, "the food guard continues");
}

/// Abort with the engaged Warlord live, then fail the abort walk so the hold
/// starts. Returns the run, its ledger and the abort tile.
fn engaged_abort_hold_after_failed_walk(
    snapshot: &mut GameSnapshot,
    after_walk: Vec<api::snapshot::NpcView>,
) -> (
    CombatRun,
    Option<Box<crate::native::ledger::Ledger>>,
    api::WorldTile,
) {
    let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let warlord = data.npc_by_config("khazard_warlord").unwrap();
    let here = api::WorldTile {
        x: 3000,
        z: 3000,
        level: 0,
    };
    let mut run = combat_test_run(
        imp_target(&data),
        None,
        Some(Arc::new(NeverStop)),
        Vec::new(),
    );
    let mut engaged = aborting_npc(
        7,
        warlord.id,
        api::WorldTile {
            x: here.x + 1,
            ..here
        },
    );
    engaged.animation = live_attack_animation(&run);
    seed_abort_scene(snapshot, here, 1, vec![engaged], None, None);
    let mut aborted = report(CombatEnd::Aborted(AbortReason::PrepFailed(
        crate::combat::PrepItem::Ammo,
    )));
    aborted.engaged = Some(crate::combat::ActorRef {
        kind: api::snapshot::ActorKind::Npc,
        index: 7,
    });
    aborted.engaged_npc_type = warlord.id;
    let mut ledger = None;
    assert!(with_step_context(snapshot, &mut ledger, 12, |cx| {
        run.on_combat_report(aborted, cx)
    })
    .is_pending());
    let _ = fail_escape_walks_until_hold(&mut run, snapshot, &mut ledger, 13);
    seed_abort_scene(snapshot, here, 1, after_walk, None, None);
    (run, ledger, here)
}

fn live_attack_animation(run: &CombatRun) -> i32 {
    run.tables
        .selected()
        .style_seqs()
        .first()
        .expect("selected combat data includes attack animations")
        .seq_id
}

#[test]
fn exhausted_escape_runs_from_a_new_attacker_when_the_engaged_actor_is_gone() {
    let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
    // A lower-range replacement at the vanished Warlord's tile isolates the
    // range fact: the plan must use the replacement, not the old report.
    let escape_envelope = |maxrange: i32, attackrange: i32| {
        maxrange
            .saturating_mul(2)
            .saturating_add(attackrange)
            .max(0)
            .saturating_add(1)
    };
    let warlord = data.npc_by_config("khazard_warlord").unwrap();
    let warlord_row = data
        .npc_name(warlord.id)
        .expect("selected content includes the engaged Warlord");
    let old_envelope = escape_envelope(warlord_row.maxrange, warlord_row.attackrange);
    let new_attacker_id = data
        .npc_names()
        .unwrap()
        .rows
        .iter()
        .find(|row| {
            crate::combat::facts::npc_max_hit(row) == Some(4)
                && escape_envelope(row.maxrange, row.attackrange) < old_envelope
        })
        .map(|row| row.id)
        .expect("selected content includes a lower-hit NPC with a narrower chase envelope");
    let mut snapshot = GameSnapshot::new();
    let new_attacker_tile = api::WorldTile {
        x: 3001,
        z: 3000,
        level: 0,
    };
    let mut newcomer = aborting_npc(9, new_attacker_id, new_attacker_tile);
    // A lower-hit attacker occupies the vanished Warlord's former tile.
    let probe_run = combat_test_run(imp_target(&data), None, None, Vec::new());
    newcomer.animation = live_attack_animation(&probe_run);
    let (mut run, mut ledger, here) =
        engaged_abort_hold_after_failed_walk(&mut snapshot, vec![newcomer]);
    for tick in 14..=16 {
        let polled = with_step_context(&snapshot, &mut ledger, tick, |cx| run.poll(cx));
        assert!(
            polled.is_pending(),
            "tick {tick}: the exhausted hold must escape, not park"
        );
    }
    let walks = ledger
        .as_ref()
        .unwrap()
        .outbox
        .iter()
        .filter_map(|action| match &action.effect {
            crate::native::HostEffect::Walk(request) => Some(request.target),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(walks.len(), 1, "exactly one exhausted escape walk");
    assert!(
        tile_distance(walks[0], new_attacker_tile) > tile_distance(here, new_attacker_tile),
        "the escape moves farther from the new attacker"
    );
    let new_attacker = data
        .npc_name(new_attacker_id)
        .expect("selected content includes the new attacker's range facts");
    let chase_envelope = escape_envelope(new_attacker.maxrange, new_attacker.attackrange);
    let distance = tile_distance(walks[0], new_attacker_tile);
    assert!(
        distance > chase_envelope,
        "the plan leaves the new attacker's chase envelope of {chase_envelope} tiles"
    );
    assert!(
        distance <= chase_envelope.saturating_add(1),
        "distance {distance} is not within the first ring beyond new envelope {chase_envelope}; old envelope is {old_envelope}"
    );
}

#[test]
fn exhausted_hold_without_any_attacker_parks_only_after_the_threat_free_horizon() {
    let mut snapshot = GameSnapshot::new();
    let (mut run, mut ledger, _) = engaged_abort_hold_after_failed_walk(&mut snapshot, vec![]);
    let mut ended = None;
    for tick in 14..=20 {
        let polled = with_step_context(&snapshot, &mut ledger, tick, |cx| run.poll(cx));
        if let Poll::Ready(result) = polled {
            ended = Some((tick, result));
            break;
        }
    }
    let Some((tick, Err(ActionError::Blocked(reason)))) = ended else {
        panic!("the hold must park with Blocked once the threat-free horizon passes");
    };
    assert_eq!(&*reason, super::ABORT_HOLD_SAFE_REASON);
    assert!(
        tick >= 16,
        "three threat-free ticks (14-16) precede the park, got {tick}"
    );
    assert!(
        ledger
            .as_ref()
            .unwrap()
            .outbox
            .iter()
            .all(|action| !matches!(action.effect, crate::native::HostEffect::Walk(_))),
        "no escape walk runs without a live attacker"
    );
}

#[test]
fn target_gone_walks_to_stand_and_rebegins_only_after_arrival() {
    let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let stand = api::WorldTile {
        x: 2632,
        z: 3222,
        level: 0,
    };
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    let mut ledger = None;
    let mut run = combat_test_run(
        imp_target(&data),
        Some(stand),
        Some(Arc::new(NeverStop)),
        Vec::new(),
    );

    assert!(with_step_context(&snapshot, &mut ledger, 12, |cx| {
        run.on_combat_report(report(CombatEnd::TargetGone), cx)
    })
    .is_pending());
    assert_eq!(run.target_gone_restarts, 0);
    assert!(matches!(&run.phase, Phase::ReturningToStand));
    assert!(matches!(run.action.as_ref(), Some(Action::Walk(_))));
    assert!(matches!(
        &ledger.as_ref().unwrap().outbox.last().unwrap().effect,
        crate::native::HostEffect::Walk(request)
            if request.target == stand && request.radius == 1
    ));

    let result = with_step_context(&snapshot, &mut ledger, 13, |cx| {
        run.on_return_walk(
            WalkReceipt {
                goal: None,
                request_id: 1,
                evidence: cx.tick.cx.evidence(),
                end: WalkEnd::Arrived,
                blocked: None,
                detail: None,
                refusal: None,
                assessment: None,
                escape: None,
            },
            cx,
        )
    });
    assert!(result.is_pending());
    assert_eq!(run.target_gone_restarts, 1);
    assert!(matches!(&run.phase, Phase::Combat));
    assert!(matches!(run.action.as_ref(), Some(Action::Combat(_))));
    let receipt = run
        .last_outcome
        .as_ref()
        .and_then(|outcome| outcome.receipt.as_ref())
        .and_then(|receipt| receipt.as_any().downcast_ref::<CombatReceipt>())
        .expect("restarted combat must retain the TargetGone report");
    assert_eq!(receipt.target_gone_restarts, 1);
}

#[test]
fn return_and_abort_walks_use_only_authored_protection() {
    let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let stand = api::WorldTile {
        x: 3125,
        z: 3246,
        level: 0,
    };
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    snapshot.seed_local_player(super::super::tests::local_player(api::WorldTile {
        x: 3112,
        z: 3242,
        level: 0,
    }));
    snapshot.seed_stats(
        (0..25)
            .map(|index| api::snapshot::StatView {
                index,
                name: String::new(),
                effective: if index == 5 { 43 } else { 40 },
                base: if index == 5 { 43 } else { 40 },
                xp: 0,
                used: api::snapshot::stat_used(index as usize),
            })
            .collect(),
    );
    for (cross, protect) in [(false, false), (true, false), (false, true), (true, true)] {
        for end in [
            CombatEnd::TargetGone,
            CombatEnd::Aborted(AbortReason::Unprotected(crate::combat::Unprotected::NoFood)),
        ] {
            let mut ledger = None;
            let mut run = combat_test_run(
                imp_target(&data),
                Some(stand),
                Some(Arc::new(NeverStop)),
                Vec::new(),
            );
            if cross {
                run.cross = Arc::from([Arc::from("draynor-jail-guards")]);
            }
            run.protect = protect;
            let aborting = matches!(&end, CombatEnd::Aborted(_));
            assert!(with_step_context(&snapshot, &mut ledger, 12, |cx| {
                run.on_combat_report(report(end), cx)
            })
            .is_pending());
            let crate::native::HostEffect::Walk(request) =
                &ledger.as_ref().unwrap().outbox.last().unwrap().effect
            else {
                panic!("the combat-owned transition must submit a native walk");
            };
            assert_eq!(request.target, stand);
            if aborting {
                assert_stand_goals(request, stand);
            } else {
                assert_eq!(request.radius, 1);
            }
            assert_eq!(request.protect, protect);
            assert_eq!(request.food_guard, aborting && !protect);
            if aborting {
                assert_eq!(request.allow.prayer, protect);
                assert!(request.allow.food);
            }
            if cross {
                assert_eq!(request.cross.as_ref(), &[Arc::from("draynor-jail-guards")]);
            } else {
                assert!(request.cross.is_empty());
            }
        }
    }
}

/// An abort walk to the authored stand names the stand and its neighbours
/// within one tile as exact Area goals, and the router picks among them.
fn assert_stand_goals(request: &crate::native::WalkRequest, stand: api::WorldTile) {
    assert_eq!(request.arrival, nav_planner::contract::ArrivalKind::Area);
    assert_eq!(request.radius, 0);
    let goals = request
        .candidate_goals
        .as_ref()
        .expect("the abort walk names exact goals");
    assert!(goals.preferred().contains(&stand));
    assert!(goals
        .as_slice()
        .iter()
        .all(|goal| tile_distance(*goal, stand) <= 1));
}

#[test]
fn target_gone_walk_without_observed_arrival_blocks_reengagement() {
    let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let stand = api::WorldTile {
        x: 2632,
        z: 3222,
        level: 0,
    };
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    let mut ledger = None;
    let mut run = combat_test_run(
        imp_target(&data),
        Some(stand),
        Some(Arc::new(NeverStop)),
        Vec::new(),
    );
    assert!(with_step_context(&snapshot, &mut ledger, 12, |cx| {
        run.on_combat_report(report(CombatEnd::TargetGone), cx)
    })
    .is_pending());

    let result = with_step_context(&snapshot, &mut ledger, 13, |cx| {
        run.on_return_walk(
            WalkReceipt {
                goal: None,
                request_id: 1,
                evidence: cx.tick.cx.evidence(),
                end: WalkEnd::RouteEnded,
                blocked: None,
                detail: None,
                refusal: None,
                assessment: None,
                escape: None,
            },
            cx,
        )
    });
    assert!(matches!(result, Poll::Ready(Err(ActionError::Blocked(_)))));
    assert_eq!(run.target_gone_restarts, 0);
    assert!(run.action.is_none());
}

#[test]
fn combat_walk_mappers_preserve_typed_evidence_without_reengaging() {
    let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let gates: Arc<[api::selected::QuestGate]> = Arc::from([api::selected::QuestGate::Complete(
        FactKey(Arc::from("combat-walk-gate")),
    )]);
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    let mut ledger = None;
    for abort in [false, true] {
        let mut run = combat_test_run(
            imp_target(&data),
            None,
            Some(Arc::new(NeverStop)),
            Vec::new(),
        );
        let result = with_step_context(&snapshot, &mut ledger, 13, |cx| {
            let receipt = WalkReceipt {
                goal: None,
                request_id: 1,
                evidence: cx.tick.cx.evidence(),
                end: WalkEnd::NeedsEvidence(Arc::clone(&gates)),
                blocked: None,
                detail: None,
                refusal: None,
                assessment: None,
                escape: None,
            };
            if abort {
                run.on_abort_walk(receipt, cx)
            } else {
                run.on_return_walk(receipt, cx)
            }
        });
        let Poll::Ready(Err(error)) = result else {
            panic!("unproven walk cannot restart combat");
        };
        assert_eq!(format!("{error:?}"), format!("NeedsEvidence({gates:?})"));
        assert_eq!(run.target_gone_restarts, 0);
        assert!(run.action.is_none());
    }
}

#[test]
fn target_gone_walk_user_input_parks_without_reengaging() {
    let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let stand = api::WorldTile {
        x: 2632,
        z: 3222,
        level: 0,
    };
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    let mut ledger = None;
    let mut run = combat_test_run(
        imp_target(&data),
        Some(stand),
        Some(Arc::new(NeverStop)),
        Vec::new(),
    );
    assert!(with_step_context(&snapshot, &mut ledger, 12, |cx| {
        run.on_combat_report(report(CombatEnd::TargetGone), cx)
    })
    .is_pending());

    let result = with_step_context(&snapshot, &mut ledger, 13, |cx| {
        run.on_return_walk(
            WalkReceipt {
                goal: None,
                request_id: 1,
                evidence: cx.tick.cx.evidence(),
                end: WalkEnd::UserInput,
                blocked: None,
                detail: None,
                refusal: None,
                assessment: None,
                escape: None,
            },
            cx,
        )
    });
    assert!(matches!(result, Poll::Ready(Err(ActionError::UserInput))));
    assert_eq!(run.target_gone_restarts, 0);
    assert!(matches!(&run.phase, Phase::ReturningToStand));
    assert!(run.action.is_none());
}

#[test]
fn manual_takeover_watch_covers_return_and_loot_sublegs() {
    let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let stand = api::WorldTile {
        x: 2632,
        z: 3222,
        level: 0,
    };
    let mut run = combat_test_run(
        imp_target(&data),
        Some(stand),
        Some(Arc::new(NeverStop)),
        vec![
            LootItem {
                id: 1,
                name: Arc::from("First drop"),
                qty: 1,
            },
            LootItem {
                id: 2,
                name: Arc::from("Second drop"),
                qty: 1,
            },
        ],
    );
    let outcome = publish_walk_outcome(40, WalkCancelReason::None);

    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    snapshot.seed_inventory(Vec::new(), 28);
    let mut ledger = None;
    with_step_context_at_walk_outcome(&snapshot, &mut ledger, 11, outcome, |cx| {
        run.manual_takeover_watch = crate::native::ManualTakeoverWatch::begin(&cx.tick.cx);
    });
    assert!(
        with_step_context_at_walk_outcome(&snapshot, &mut ledger, 12, outcome, |cx| {
            run.on_combat_report(report(CombatEnd::TargetGone), cx)
        })
        .is_pending()
    );

    let outcome = publish_walk_outcome(41, WalkCancelReason::None);
    assert!(
        with_step_context_at_walk_outcome(&snapshot, &mut ledger, 13, outcome, |cx| {
            run.on_return_walk(
                WalkReceipt {
                    goal: None,
                    request_id: 1,
                    evidence: cx.tick.cx.evidence(),
                    end: WalkEnd::Arrived,
                    blocked: None,
                    detail: None,
                    refusal: None,
                    assessment: None,
                    escape: None,
                },
                cx,
            )
        })
        .is_pending()
    );
    assert_eq!(run.target_gone_restarts, 1);

    assert!(
        with_step_context_at_walk_outcome(&snapshot, &mut ledger, 14, outcome, |cx| {
            run.on_combat_report(report(CombatEnd::Killed), cx)
        })
        .is_pending()
    );
    assert!(
        with_step_context_at_walk_outcome(&snapshot, &mut ledger, 15, outcome, |cx| {
            run.poll(cx)
        })
        .is_pending()
    );
    assert_eq!(run.loot_index, 1);
    assert!(matches!(run.action.as_ref(), Some(Action::Loot(_))));

    let outcome = publish_walk_outcome(42, WalkCancelReason::UserInput);
    let result =
        with_step_context_at_walk_outcome(&snapshot, &mut ledger, 16, outcome, |cx| run.poll(cx));
    assert!(matches!(result, Poll::Ready(Err(ActionError::UserInput))));
    assert_eq!(
        run.loot_index, 1,
        "manual movement must not start the next loot leg"
    );
    assert!(run.action.is_none());
    assert!(ledger
        .as_ref()
        .unwrap()
        .outbox
        .iter()
        .all(|action| { !matches!(&action.effect, crate::native::HostEffect::Interaction(_)) }));
}

#[test]
fn killed_report_enters_loot_and_takes_the_observed_drop() {
    let selected = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let item = selected.item_by_alias("white_bead").unwrap();
    let item_id = item.id;
    let item_name: Arc<str> = Arc::from(item.name.as_deref().unwrap_or("White bead"));
    let stand = api::WorldTile {
        x: 3253,
        z: 3401,
        level: 0,
    };
    let mut run = combat_test_run(
        imp_target(&selected),
        Some(stand),
        Some(Arc::new(NeverStop)),
        vec![LootItem {
            id: item_id,
            name: item_name.clone(),
            qty: 1,
        }],
    );
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    snapshot.seed_inventory(Vec::new(), 28);
    super::super::tests::seed_dialogue_combat(&mut snapshot, false);
    snapshot.seed_ground_items(vec![api::snapshot::GroundItemView {
        def: api::obj_names::ItemDefView {
            id: item_id,
            name: Some(item_name.to_string()),
            stackable: false,
            members: false,
            base_value: 1,
            noted: false,
            certificate_link: -1,
            certificate_template: -1,
        },
        count: 1,
        actions: vec![Some("Take".into())],
        tile: stand,
        distance: 0,
    }]);
    let mut ledger = None;
    assert!(with_step_context(&snapshot, &mut ledger, 12, |cx| {
        run.on_combat_report(report(CombatEnd::Killed), cx)
    })
    .is_pending());
    assert!(matches!(&run.phase, Phase::Loot));
    assert!(with_step_context(&snapshot, &mut ledger, 13, |cx| run.poll(cx)).is_pending());
    assert!(matches!(run.action.as_ref(), Some(Action::Loot(_))));
    assert!(matches!(
        &ledger.as_ref().unwrap().outbox.last().unwrap().effect,
        crate::native::HostEffect::Interaction(crate::shim::InteractReq::Obj {
            x,
            z,
            id: Some(id),
            action,
            ..
        }) if *x == stand.x && *z == stand.z && *id == item_id && action == "Take"
    ));
}

#[test]
fn same_tick_loot_settle_defers_next_item_without_skipping_it() {
    let selected = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let first = selected.item_by_alias("white_bead").unwrap();
    let second = selected.item_by_alias("bones").unwrap();
    let stand = api::WorldTile {
        x: 3253,
        z: 3401,
        level: 0,
    };
    let loot: Vec<_> = [first, second]
        .into_iter()
        .map(|item| LootItem {
            id: item.id,
            name: Arc::from(item.name.as_deref().unwrap()),
            qty: 1,
        })
        .collect();
    let mut run = combat_test_run(imp_target(&selected), Some(stand), None, loot);
    run.phase = Phase::Loot;
    let snapshot = || {
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_inventory(Vec::new(), 28);
        snapshot.seed_ground_items(
            [first, second]
                .into_iter()
                .map(|item| api::snapshot::GroundItemView {
                    def: super::super::tests::def(item.id, item.name.as_deref().unwrap()),
                    count: 1,
                    actions: vec![Some("Take".into())],
                    tile: stand,
                    distance: 0,
                })
                .collect(),
        );
        snapshot
    };
    let initial = snapshot();
    let mut later = snapshot();
    later.seed_inventory(
        vec![inventory_item(first.id, first.name.as_deref().unwrap(), 1)],
        28,
    );
    let mut ledger = None;
    super::super::tests::with_tick_snapshots(&initial, &later, &mut ledger, 1, |native, later| {
        let quests = QuestCatalog::empty();
        let banks = Arc::new(api::named_banks::NamedBankFacts::empty());
        let choices = crate::quester::choices::QuestChoices::default();
        let required_after = native.cx.evidence();
        let mut cx = StepContext {
            tick: native,
            quests: &quests,
            progress: &[],
            required_after,
            banks: &banks,
            choices: &choices,
        };
        assert!(run.poll(&mut cx).is_pending());
        assert_eq!(run.loot_index, 1);
        assert_eq!(cx.tick.cx.ledger.as_ref().unwrap().outbox.len(), 1);
        cx.tick.cx.snapshot = api::snapshot::SnapshotView::new(Some(later), required_after);
        for _ in 0..3 {
            assert!(run.poll(&mut cx).is_pending());
            assert_eq!(
                run.loot_index, 1,
                "denied loot admission must retain the second item"
            );
        }
        assert!(cx.tick.cx.ledger.as_ref().unwrap().outbox.is_empty());
    });
    assert!(with_step_context(&later, &mut ledger, 2, |cx| run.poll(cx)).is_pending());
    assert_eq!(run.loot_index, 2);
    assert!(matches!(
        &ledger.as_ref().unwrap().outbox.last().unwrap().effect,
        crate::native::HostEffect::Interaction(crate::shim::InteractReq::Obj {
            id: Some(id),
            name: Some(name),
            ..
        }) if *id == second.id && name == second.name.as_deref().unwrap()
    ));
}

#[test]
fn combat_loot_reaches_below_quantity_and_skips_at_the_requested_count() {
    let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let bones = data.item_by_alias("bones").unwrap();
    let bead = data.item_by_alias("white_bead").unwrap();
    let bones_name = bones.name.as_deref().unwrap();
    let bead_name = bead.name.as_deref().unwrap();
    let bones_tile = api::WorldTile {
        x: 3253,
        z: 3401,
        level: 0,
    };
    let bead_tile = api::WorldTile {
        x: 3254,
        z: 3401,
        level: 0,
    };

    for (held_bones, expected_index, expected_tile, expected_id) in
        [(1, 1, bones_tile, bones.id), (25, 2, bead_tile, bead.id)]
    {
        let mut run = combat_test_run(
            imp_target(&data),
            None,
            Some(Arc::new(NeverStop)),
            vec![
                LootItem {
                    id: bones.id,
                    name: Arc::from(bones_name),
                    qty: 25,
                },
                LootItem {
                    id: bead.id,
                    name: Arc::from(bead_name),
                    qty: 1,
                },
            ],
        );
        run.phase = Phase::Loot;
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_inventory(vec![inventory_item(bones.id, bones_name, held_bones)], 28);
        super::super::tests::seed_dialogue_combat(&mut snapshot, false);
        snapshot.seed_ground_items(vec![
            ground_item(bones.id, bones_name, bones_tile, 0),
            ground_item(bead.id, bead_name, bead_tile, 1),
        ]);
        let mut ledger = None;

        assert!(with_step_context(&snapshot, &mut ledger, 1, |cx| run.poll(cx)).is_pending());
        assert_eq!(run.loot_index, expected_index);
        assert!(matches!(run.action.as_ref(), Some(Action::Loot(_))));
        assert!(matches!(
            &ledger.as_ref().unwrap().outbox.last().unwrap().effect,
            crate::native::HostEffect::Interaction(crate::shim::InteractReq::Obj {
                x, z, id: Some(id), action, ..
            }) if *x == expected_tile.x && *z == expected_tile.z && *id == expected_id && action == "Take"
        ));
    }
}

#[test]
fn lifecycle_followups_unreachable_loot_walk_skips_to_the_next_item() {
    let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let here = api::WorldTile {
        x: 3200,
        z: 3200,
        level: 0,
    };
    let door = api::WorldTile {
        x: 3203,
        z: 3200,
        level: 0,
    };
    let first_tile = api::WorldTile {
        x: 3205,
        z: 3200,
        level: 0,
    };
    let second_tile = api::WorldTile {
        x: 3201,
        z: 3201,
        level: 0,
    };
    let mut run = combat_test_run(
        imp_target(&data),
        None,
        Some(Arc::new(NeverStop)),
        vec![
            LootItem {
                id: 1,
                name: Arc::from("First drop"),
                qty: 1,
            },
            LootItem {
                id: 2,
                name: Arc::from("Second drop"),
                qty: 1,
            },
        ],
    );
    let ground = |id, name: &str, tile, distance| GroundItemView {
        def: api::obj_names::ItemDefView {
            id,
            name: Some(name.to_owned()),
            stackable: false,
            members: false,
            base_value: 1,
            noted: false,
            certificate_link: -1,
            certificate_template: -1,
        },
        count: 1,
        actions: vec![Some("Take".into())],
        tile,
        distance,
    };
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    snapshot.seed_inventory(Vec::new(), 28);
    snapshot.seed_local_player(super::super::tests::local_player(here));
    snapshot.seed_ground_items(vec![
        ground(1, "First drop", first_tile, 5),
        ground(2, "Second drop", second_tile, 2),
    ]);
    snapshot.seed_locs(vec![LocView {
        id: 7,
        name: Some("Door".into()),
        actions: vec![Some("Open".into())],
        tile: door,
        distance: 3,
        typecode: 0,
        info: 0,
        description: None,
        layer: LocLayer::GroundDecoration,
        shape: -1,
        angle: 0,
        width: 1,
        length: 1,
        footprint_width: 1,
        footprint_length: 1,
        block_walk: false,
        block_range: false,
        active: true,
        animation: -1,
        map_function: -1,
        map_scene: -1,
        force_approach: 0,
    }]);
    snapshot.seed_chat_lines(vec![]);
    let mut ledger = None;
    assert!(with_step_context(&snapshot, &mut ledger, 12, |cx| {
        run.on_combat_report(report(CombatEnd::Killed), cx)
    })
    .is_pending());
    assert!(with_step_context(&snapshot, &mut ledger, 13, |cx| run.poll(cx)).is_pending());
    assert!(matches!(
        &ledger.as_ref().unwrap().outbox.last().unwrap().effect,
        crate::native::HostEffect::Interaction(crate::shim::InteractReq::Obj {
            x,
            z,
            id: Some(id),
            action,
            ..
        }) if *x == first_tile.x && *z == first_tile.z && *id == 1 && action == "Take"
    ));

    // The failed item click identifies the closed door; Reach's real recovery
    // path emits a walk to that door before this terminal route receipt.
    ledger.as_mut().unwrap().outbox.clear();
    snapshot.seed_chat_lines(vec![ChatLineView {
        type_: 0,
        username: None,
        text: "I can't reach that".into(),
        sequence: 1,
    }]);
    assert!(with_step_context(&snapshot, &mut ledger, 14, |cx| run.poll(cx)).is_pending());
    let request_id = match &ledger.as_ref().unwrap().outbox.last().unwrap().effect {
        crate::native::HostEffect::Walk(request) => {
            assert_eq!(request.target, door);
            ledger
                .as_ref()
                .unwrap()
                .outbox
                .last()
                .unwrap()
                .request_id
                .get()
        }
        _ => panic!("Reach must walk to the closed door to recover the loot path"),
    };
    ledger.as_mut().unwrap().outbox.clear();
    ledger.as_mut().unwrap().walk = Some(crate::native::WalkReceipt {
        goal: None,
        request_id,
        evidence: EvidenceStamp {
            run: RunKey {
                slot: 1,
                run: 1,
                session: 1,
            },
            tick: 15,
            sequence: 15,
        },
        end: WalkEnd::Failed,
        blocked: None,
        detail: Some(Arc::from("door recovery route unreachable")),
        refusal: None,
        assessment: None,
        escape: None,
    });

    assert!(with_step_context(&snapshot, &mut ledger, 15, |cx| run.poll(cx)).is_pending());
    assert_eq!(run.loot_index, 2, "the unreachable item is skipped");
    assert!(matches!(run.action.as_ref(), Some(Action::Loot(_))));
    assert!(matches!(
        &ledger.as_ref().unwrap().outbox.last().unwrap().effect,
        crate::native::HostEffect::Interaction(crate::shim::InteractReq::Obj {
            x,
            z,
            id: Some(id),
            action,
            ..
        }) if *x == second_tile.x && *z == second_tile.z && *id == 2 && action == "Take"
    ));
}

#[test]
fn combat_finish_requires_the_same_local_npc_slot_to_transform() {
    let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let original = data.npc_by_config("delrith").unwrap().id;
    let weakened = data.npc_by_config("delrith_weakened").unwrap().id;
    let mut run = finish_test_run(&data, 100);
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    snapshot.seed_inventory(Vec::new(), 28);
    seed_finish_target(&mut snapshot, Some(42), true);
    snapshot.seed_npcs(vec![finish_npc(42, original), finish_npc(99, weakened)]);
    let mut ledger = None;

    with_step_context(&snapshot, &mut ledger, 1, |cx| {
        run.begin_combat(cx).unwrap()
    });
    assert!(with_step_context(&snapshot, &mut ledger, 2, |cx| run.poll(cx)).is_pending());
    assert_eq!(run.finish_target_index, Some(42));

    snapshot.seed_npcs(vec![finish_npc(42, original), finish_npc(99, weakened)]);
    assert!(with_step_context(&snapshot, &mut ledger, 3, |cx| run.poll(cx)).is_pending());
    assert!(matches!(&run.phase, Phase::Combat));

    seed_finish_target(&mut snapshot, None, true);
    snapshot.seed_npcs(vec![finish_npc(42, weakened), finish_npc(99, weakened)]);
    ledger.as_mut().unwrap().outbox.clear();
    assert!(with_step_context(&snapshot, &mut ledger, 4, |cx| run.poll(cx)).is_pending());
    assert!(matches!(&run.phase, Phase::FinishWait));
    assert!(run.action.is_none());
    assert!(ledger.as_ref().unwrap().outbox.is_empty());
}

#[test]
fn combat_finish_rebinds_before_accepting_an_old_slot_transform() {
    let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let original = data.npc_by_config("delrith").unwrap().id;
    let weakened = data.npc_by_config("delrith_weakened").unwrap().id;
    let mut run = finish_test_run(&data, 100);
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    snapshot.seed_npcs(vec![finish_npc(42, original), finish_npc(53, original)]);
    seed_finish_target(&mut snapshot, Some(42), true);
    let mut ledger = None;
    assert!(!with_step_context(&snapshot, &mut ledger, 1, |cx| run
        .observe_finish_transform(cx)));
    assert_eq!(run.finish_target_index, Some(42));

    snapshot.seed_npcs(vec![finish_npc(42, weakened), finish_npc(53, original)]);
    seed_finish_target(&mut snapshot, Some(53), true);
    assert!(!with_step_context(&snapshot, &mut ledger, 2, |cx| run
        .observe_finish_transform(cx)));
    assert_eq!(
        run.finish_target_index,
        Some(53),
        "the old slot is a finish decoy"
    );

    snapshot.seed_npcs(vec![finish_npc(42, weakened), finish_npc(53, weakened)]);
    seed_finish_target(&mut snapshot, None, false);
    assert!(with_step_context(&snapshot, &mut ledger, 3, |cx| run
        .observe_finish_transform(cx)));
    assert_eq!(
        run.finish_target_index,
        Some(53),
        "same-child target loss preserves its transform proof"
    );
}

#[test]
fn combat_finish_restart_discards_the_previous_child_pin() {
    let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let original = data.npc_by_config("delrith").unwrap().id;
    let weakened = data.npc_by_config("delrith_weakened").unwrap().id;
    let mut run = finish_test_run(&data, 100);
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    snapshot.seed_inventory(Vec::new(), 28);
    snapshot.seed_npcs(vec![finish_npc(42, original), finish_npc(53, original)]);
    seed_finish_target(&mut snapshot, Some(42), true);
    let mut ledger = None;
    with_step_context(&snapshot, &mut ledger, 1, |cx| {
        run.begin_combat(cx).unwrap()
    });
    assert!(!with_step_context(&snapshot, &mut ledger, 2, |cx| run
        .observe_finish_transform(cx)));
    assert_eq!(run.finish_target_index, Some(42));

    drop(run.action.take());
    snapshot.seed_npcs(vec![finish_npc(42, weakened), finish_npc(53, original)]);
    seed_finish_target(&mut snapshot, None, false);
    assert!(
        with_step_context(&snapshot, &mut ledger, 3, |cx| run.rebegin_combat(cx, true))
            .is_pending()
    );
    assert_eq!(run.finish_target_index, None);
    assert!(!with_step_context(&snapshot, &mut ledger, 4, |cx| run
        .observe_finish_transform(cx)));
    seed_finish_target(&mut snapshot, Some(53), true);
    assert!(!with_step_context(&snapshot, &mut ledger, 5, |cx| run
        .observe_finish_transform(cx)));
    assert_eq!(run.finish_target_index, Some(53));
}

#[test]
fn combat_finish_waits_for_quiet_ready_chat_then_drives_continuation() {
    let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let mut run = finish_test_run(&data, 100);
    start_finish_wait(&mut run, 0);
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    snapshot.seed_chat_modal(-1, vec![]);
    snapshot.seed_chat_options(vec![], -1);
    super::super::tests::seed_dialogue_combat(&mut snapshot, true);
    let mut ledger = None;

    assert!(with_step_context(&snapshot, &mut ledger, 1, |cx| run.poll(cx)).is_pending());
    assert!(
        run.action.is_none(),
        "combat must be observed quiet before driving"
    );
    super::super::tests::seed_dialogue_combat(&mut snapshot, false);
    assert!(with_step_context(&snapshot, &mut ledger, 2, |cx| run.poll(cx)).is_pending());
    assert!(run.action.is_none(), "a closed chat is not ready");

    snapshot.seed_chat_modal(4882, vec!["Choose the incantation".into()]);
    snapshot.seed_chat_options(
        vec![
            api::snapshot::ChatOptionView {
                component_id: 11,
                text: "Option one".into(),
            },
            api::snapshot::ChatOptionView {
                component_id: 12,
                text: "Option two".into(),
            },
            api::snapshot::ChatOptionView {
                component_id: 13,
                text: "Option three".into(),
            },
            api::snapshot::ChatOptionView {
                component_id: 14,
                text: "Carlem Aber Camerinthum Purchai Gabindo".into(),
            },
        ],
        -1,
    );
    // The finish Dialogue begun on a ready page answers on its begin tick
    // (TICK-FIX #14, C-DIALOGUE-ADOPT).
    assert!(with_step_context(&snapshot, &mut ledger, 3, |cx| run.poll(cx)).is_pending());
    assert!(matches!(run.action.as_ref(), Some(Action::Dialogue(_))));
    assert!(matches!(
        &ledger.as_ref().unwrap().outbox.last().unwrap().effect,
        crate::native::HostEffect::Interaction(crate::shim::InteractReq::Answer { option: 4 })
    ));

    assert!(with_step_context(&snapshot, &mut ledger, 4, |cx| run.poll(cx)).is_pending());
    snapshot.seed_chat_modal(4883, vec!["The demon is weakened.".into()]);
    snapshot.seed_chat_options(vec![], 4899);
    // A new chat root acknowledges the answer with its page complete: the
    // Continue goes out on the ack tick (TICK-FIX #14, engine resume).
    assert!(with_step_context(&snapshot, &mut ledger, 5, |cx| run.poll(cx)).is_pending());
    assert!(matches!(
        &ledger.as_ref().unwrap().outbox.last().unwrap().effect,
        crate::native::HostEffect::Interaction(crate::shim::InteractReq::ContinueDialog {
            component_id: None
        })
    ));
    assert!(with_step_context(&snapshot, &mut ledger, 6, |cx| run.poll(cx)).is_pending());
    assert!(with_step_context(&snapshot, &mut ledger, 7, |cx| run.poll(cx)).is_pending());
    assert!(matches!(
        &ledger.as_ref().unwrap().outbox.last().unwrap().effect,
        crate::native::HostEffect::Interaction(crate::shim::InteractReq::ContinueDialog {
            component_id: None
        })
    ));

    snapshot.seed_chat_modal(-1, vec![]);
    snapshot.seed_chat_options(vec![], -1);
    assert!(
        with_step_context(&snapshot, &mut ledger, 8, |cx| run.poll(cx)).is_pending(),
        "finish observes the closed chat and starts the end gap on that tick"
    );
    let completed = with_step_context(&snapshot, &mut ledger, 9, |cx| run.poll(cx));
    let Poll::Ready(Ok(outcome)) = completed else {
        panic!("only a completed shared continuation may settle finish");
    };
    let receipt = outcome.receipt.as_ref().unwrap();
    let finish = receipt.as_any().downcast_ref::<FinishReceipt>().unwrap();
    assert_eq!(finish.npc_index, 42);
    assert!(receipt.as_any().downcast_ref::<CombatReceipt>().is_none());
    assert!(!ledger.as_ref().unwrap().outbox.iter().any(|row| {
        matches!(
            &row.effect,
            crate::native::HostEffect::Interaction(crate::shim::InteractReq::Npc {
                action, ..
            }) if action.eq_ignore_ascii_case("Talk-to")
        )
    }));
}

#[test]
fn killed_report_keeps_demon_incantation_under_finish_dialogue() {
    let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let original = data.npc_by_config("delrith").unwrap().id;
    let weakened = data.npc_by_config("delrith_weakened").unwrap().id;
    let mut run = finish_test_run(&data, 100);
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    snapshot.seed_inventory(Vec::new(), 28);
    seed_finish_target(&mut snapshot, Some(42), true);
    snapshot.seed_npcs(vec![finish_npc(42, original)]);
    let mut ledger = None;

    with_step_context(&snapshot, &mut ledger, 1, |cx| {
        run.begin_combat(cx).unwrap()
    });
    assert!(with_step_context(&snapshot, &mut ledger, 2, |cx| run.poll(cx)).is_pending());
    assert_eq!(run.finish_target_index, Some(42));

    seed_finish_target(&mut snapshot, None, false);
    snapshot.seed_npcs(vec![finish_npc(42, weakened)]);
    assert!(with_step_context(&snapshot, &mut ledger, 3, |cx| {
        run.on_combat_report(report(CombatEnd::Killed), cx)
    })
    .is_pending());
    assert!(matches!(&run.phase, Phase::FinishWait));
    assert!(run.action.is_none());
    assert_eq!(run.finish_target_index, Some(42));

    assert!(with_step_context(&snapshot, &mut ledger, 4, |cx| run.poll(cx)).is_pending());
    assert!(matches!(&run.phase, Phase::FinishWait));
    assert!(!ledger.as_ref().unwrap().outbox.iter().any(|row| {
        matches!(
            &row.effect,
            crate::native::HostEffect::Interaction(crate::shim::InteractReq::Answer { .. })
        )
    }));
    snapshot.seed_chat_modal(2480, vec!["Select an Option".into()]);
    snapshot.seed_chat_options(
        vec![
            api::snapshot::ChatOptionView {
                component_id: 11,
                text: "Carlem Gabindo Purchai Zaree Camerinthum".into(),
            },
            api::snapshot::ChatOptionView {
                component_id: 12,
                text: "Purchai Zaree Gabindo Carlem Camerinthum".into(),
            },
            api::snapshot::ChatOptionView {
                component_id: 13,
                text: "Purchai Camerinthum Aber Gabindo Carlem".into(),
            },
            api::snapshot::ChatOptionView {
                component_id: 14,
                text: "Carlem Aber Camerinthum Purchai Gabindo".into(),
            },
        ],
        -1,
    );
    assert!(with_step_context(&snapshot, &mut ledger, 5, |cx| run.poll(cx)).is_pending());
    assert!(matches!(&run.phase, Phase::FinishDialogue));
    assert_eq!(run.finish_ticks_elapsed, 2);
    assert!(run.finish_ticks_elapsed <= u64::from(run.finish.as_ref().unwrap().max_ticks));
    let answers: Vec<_> = ledger
        .as_ref()
        .unwrap()
        .outbox
        .iter()
        .filter_map(|row| match &row.effect {
            crate::native::HostEffect::Interaction(crate::shim::InteractReq::Answer { option }) => {
                Some(*option)
            }
            _ => None,
        })
        .collect();
    assert_eq!(answers, vec![4]);
}

#[test]
fn combat_finish_tick_budget_covers_the_dialogue_and_interruption_blocks() {
    let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let mut run = finish_test_run(&data, 2);
    start_finish_wait(&mut run, 0);
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    snapshot.seed_chat_modal(4882, vec!["Open continuation".into()]);
    snapshot.seed_chat_options(vec![], -1);
    super::super::tests::seed_dialogue_combat(&mut snapshot, false);
    let mut ledger = None;

    assert!(with_step_context(&snapshot, &mut ledger, 1, |cx| run.poll(cx)).is_pending());
    assert!(matches!(run.action.as_ref(), Some(Action::Dialogue(_))));
    assert!(with_step_context(&snapshot, &mut ledger, 2, |cx| run.poll(cx)).is_pending());
    let timeout = with_step_context(&snapshot, &mut ledger, 3, |cx| run.poll(cx));
    assert!(matches!(
        timeout,
        Poll::Ready(Err(ActionError::Blocked(reason))) if reason.contains("tick budget")
    ));

    let mut interrupted = finish_test_run(&data, 100);
    start_finish_wait(&mut interrupted, 0);
    snapshot.seed_chat_modal(4882, vec!["Open continuation".into()]);
    snapshot.seed_chat_options(vec![], -1);
    super::super::tests::seed_dialogue_combat(&mut snapshot, false);
    let mut interrupted_ledger = None;
    assert!(
        with_step_context(&snapshot, &mut interrupted_ledger, 1, |cx| {
            interrupted.poll(cx)
        })
        .is_pending()
    );
    snapshot.seed_chat_modal(-1, vec![]);
    snapshot.seed_chat_options(vec![], -1);
    super::super::tests::seed_dialogue_combat(&mut snapshot, true);
    let result = with_step_context(&snapshot, &mut interrupted_ledger, 2, |cx| {
        interrupted.poll(cx)
    });
    assert!(matches!(
        result,
        Poll::Ready(Err(ActionError::CombatInterrupted))
    ));
}

#[test]
fn combat_finish_uses_strict_current_page_line_rules() {
    let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let make_options = || {
        super::super::compile_dialogue_options(super::super::DialogueOptionsDocument {
            prefer: Vec::new(),
            choose: None,
            line_rules: vec![super::super::LineRuleDocument {
                when_line: "Choose the incantation".into(),
                choose: "Rule answer".into(),
            }],
            strict: true,
            gap_ticks: None,
        })
        .unwrap()
    };

    let mut run = finish_test_run(&data, 100);
    run.finish.as_mut().unwrap().options = make_options();
    start_finish_wait(&mut run, 0);
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    snapshot.seed_chat_modal(4882, vec!["Choose the incantation".into()]);
    snapshot.seed_chat_options(
        vec![
            api::snapshot::ChatOptionView {
                component_id: 11,
                text: "Fallback answer".into(),
            },
            api::snapshot::ChatOptionView {
                component_id: 12,
                text: "Rule answer".into(),
            },
        ],
        -1,
    );
    super::super::tests::seed_dialogue_combat(&mut snapshot, false);
    let mut ledger = None;
    // The ready finish page is answered on the tick its Dialogue begins
    // (TICK-FIX #14).
    assert!(with_step_context(&snapshot, &mut ledger, 1, |cx| run.poll(cx)).is_pending());
    assert!(matches!(
        &ledger.as_ref().unwrap().outbox.last().unwrap().effect,
        crate::native::HostEffect::Interaction(crate::shim::InteractReq::Answer { option: 2 })
    ));
    assert!(with_step_context(&snapshot, &mut ledger, 2, |cx| run.poll(cx)).is_pending());

    let mut unmatched = finish_test_run(&data, 100);
    unmatched.finish.as_mut().unwrap().options = make_options();
    start_finish_wait(&mut unmatched, 0);
    let mut unmatched_snapshot = GameSnapshot::new();
    unmatched_snapshot.seed_ingame(2);
    unmatched_snapshot.seed_chat_modal(4882, vec!["A different current page".into()]);
    unmatched_snapshot.seed_chat_options(
        vec![
            api::snapshot::ChatOptionView {
                component_id: 11,
                text: "Fallback answer".into(),
            },
            api::snapshot::ChatOptionView {
                component_id: 12,
                text: "Rule answer".into(),
            },
        ],
        -1,
    );
    super::super::tests::seed_dialogue_combat(&mut unmatched_snapshot, false);
    let mut unmatched_ledger = None;
    let failed = with_step_context(&unmatched_snapshot, &mut unmatched_ledger, 1, |cx| {
        unmatched.poll(cx)
    });
    assert!(matches!(
        failed,
        Poll::Ready(Err(ActionError::Blocked(reason))) if reason.contains("combat finish dialogue failed")
    ));
    assert!(
        !unmatched_ledger.as_ref().unwrap().outbox.iter().any(|row| {
            matches!(
                &row.effect,
                crate::native::HostEffect::Interaction(crate::shim::InteractReq::Answer { .. })
            )
        })
    );
}

#[test]
fn combat_finish_budget_counts_evidence_tick_deltas_across_wait_and_dialogue() {
    let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let mut run = finish_test_run(&data, 2);
    run.finish.as_mut().unwrap().options = DialogueOptions::default();
    start_finish_wait(&mut run, 10);
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    snapshot.seed_chat_modal(4882, vec!["A continuation page".into()]);
    snapshot.seed_chat_options(
        vec![api::snapshot::ChatOptionView {
            component_id: 11,
            text: "Continue with this".into(),
        }],
        -1,
    );
    super::super::tests::seed_dialogue_combat(&mut snapshot, false);
    let mut ledger = None;

    assert!(with_step_context(&snapshot, &mut ledger, 11, |cx| run.poll(cx)).is_pending());
    assert!(matches!(&run.phase, Phase::FinishDialogue));
    assert_eq!(run.finish_ticks_elapsed, 1);
    assert!(with_step_context(&snapshot, &mut ledger, 11, |cx| run.poll(cx)).is_pending());
    assert_eq!(
        run.finish_ticks_elapsed, 1,
        "a repeated poll in the same game tick spends no budget"
    );
    assert!(with_step_context(&snapshot, &mut ledger, 12, |cx| run.poll(cx)).is_pending());
    assert_eq!(run.finish_ticks_elapsed, 2);

    let timeout = with_step_context(&snapshot, &mut ledger, 15, |cx| run.poll(cx));
    assert!(matches!(
        timeout,
        Poll::Ready(Err(ActionError::Blocked(reason))) if reason.contains("tick budget")
    ));
    assert_eq!(
        run.finish_ticks_elapsed, 5,
        "a three-tick evidence jump spends all three ticks"
    );
}

#[test]
fn combat_finish_budget_ignores_duplicate_polls_and_counts_tick_jumps() {
    let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let mut run = finish_test_run(&data, 2);
    run.phase = Phase::FinishWait;
    run.finish_target_index = Some(42);
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    let mut ledger = None;
    for _ in 0..4 {
        assert!(
            with_step_context(&snapshot, &mut ledger, 1, |cx| run.poll(cx)).is_pending(),
            "duplicate polls must not spend a game-tick budget"
        );
    }
    assert!(
        matches!(
            with_step_context(&snapshot, &mut ledger, 4, |cx| run.poll(cx)),
            Poll::Ready(Err(ActionError::Blocked(reason))) if reason.contains("tick budget")
        ),
        "the jump from game tick 1 to 4 must spend three ticks"
    );
}

#[test]
fn combat_finish_strict_menu_refusals_block_consistently() {
    let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
    for prefer in [Arc::from([]), Arc::from([Arc::from("Unmatched answer")])] {
        let mut run = finish_test_run(&data, 100);
        run.finish.as_mut().unwrap().options = DialogueOptions {
            prefer,
            strict: true,
            ..Default::default()
        };
        start_finish_wait(&mut run, 0);
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_chat_modal(4882, vec!["Choose an answer.".into()]);
        snapshot.seed_chat_options(
            vec![api::snapshot::ChatOptionView {
                component_id: 11,
                text: "Not the authored answer".into(),
            }],
            -1,
        );
        super::super::tests::seed_dialogue_combat(&mut snapshot, false);
        let mut ledger = None;
        assert!(
            matches!(
                with_step_context(&snapshot, &mut ledger, 1, |cx| run.poll(cx)),
                Poll::Ready(Err(ActionError::Blocked(reason)))
                    if reason.contains("combat finish dialogue failed")
            ),
            "strict refusal must park rather than replay combat"
        );
        assert!(ledger
            .as_ref()
            .is_none_or(|ledger| ledger.outbox.is_empty()));
    }
}

#[test]
fn combat_kit_carry_keeps_compiled_alias_ids() {
    let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let quests = QuestCatalog::from_identity(data.quest_identity()).unwrap();
    let coins = data.item_by_alias("coins").unwrap();
    let bones = data.item_by_alias("bones").unwrap();
    assert_eq!(
        coins.id, 995,
        "authored `coins` is the stackable coin object"
    );
    // The shared display names collide: resolving them by name picks another object.
    assert_eq!(
        data.resolve_item_name("Coins").unwrap().id,
        617,
        "display `Coins` collides with the unstackable object"
    );
    assert_ne!(
        data.resolve_item_name("Bones").unwrap().id,
        bones.id,
        "display `Bones` collides with another bone object"
    );
    // The overlay row holds display names, as the compiled Path rows do; the
    // carry map holds the alias-resolved identities native consumers use.
    let row = api::loadout::Loadout::new("kitpath/cash")
        .with_carry("Coins", 1000)
        .with_carry("Bones", 5);
    let loadouts = LoadoutOverlay::new(Arc::from([]), Arc::from([row]));
    let carry: Arc<[crate::quester::compile::CompiledCarry]> = Arc::from(vec![
        crate::quester::compile::CompiledCarry {
            item: crate::native_bank::BankItem {
                id: coins.id,
                name: Arc::from(coins.name.as_deref().unwrap()),
            },
            qty: 1000,
            stackable: coins.stackable,
            latch_index: 0,
        },
        crate::quester::compile::CompiledCarry {
            item: crate::native_bank::BankItem {
                id: bones.id,
                name: Arc::from(bones.name.as_deref().unwrap()),
            },
            qty: 5,
            stackable: bones.stackable,
            latch_index: 1,
        },
    ]);
    let mut loadout_carry = HashMap::new();
    loadout_carry.insert(Arc::from("kitpath/cash"), Arc::clone(&carry));
    let progress = CompiledProgress {
        binding: FactKey::new("journal:kitpath"),
        role: None,
        colour_not_started: FactKey::new("kitpath:0"),
        colour_in_progress: FactKey::new("kitpath:1"),
        colour_complete: FactKey::new("kitpath:2"),
        stage_keys: Arc::from([]),
        rules: Arc::from([]),
        flags: Arc::from([]),
        monotonic: false,
    };
    let areas = HashMap::new();
    let recipes = HashMap::new();
    let path = FactKey::new("kitpath");
    let cx = CompileContext {
        path: &path,
        kind: crate::quester::path::PathKind::Quest,
        pair: None,
        progress: &progress,
        selected: &data,
        quests: &quests,
        gathering: None,
        bank: None,
        bank_required: false,
        keep_ids: &[],
        areas: &areas,
        loadouts: &loadouts,
        loadout_carry: &loadout_carry,
        recipes: &recipes,
    };
    let args = serde_json::json!({
        "target": {"npc": "jailguard", "pick": "nearest", "not_targeting_others": true},
        "tactic": {"kind": "open", "style": "melee", "engage_radius": 12},
        "lost_radius": 16,
        "kill_budget_ticks": 400,
        "loadout": "cash",
        "until": {"Any": []}
    });
    let plan = compile_plan(serde_json::from_value(args).unwrap(), &cx).unwrap();
    let kit = plan.request.kit.as_ref().expect("kit compiles");
    assert_eq!(
        kit.carry.as_ref(),
        &[(coins.id, 1000), (bones.id, 5)],
        "kit carry keeps the compiled alias identities, not the display-name picks"
    );
}
#[cfg(test)]
mod abort_hold_cure_policy_tests {
    use super::*;
    use crate::native::HostEffect;
    use crate::quester::families::tests::{def, local_player, with_tick};
    use api::selected::ClientRevision;
    use api::snapshot::{
        GameSnapshot, HitmarkView, HitmarksView, ItemActionFamily, ItemContainer, ItemView,
        StatView, VarpView, WorldStateView,
    };
    use std::sync::Arc;
    use std::task::Poll;

    fn held_item(id: i32, name: &str, action: &str, slot: i32) -> ItemView {
        ItemView {
            def: def(id, name),
            container: ItemContainer::Inventory,
            action_family: ItemActionFamily::Held,
            slot,
            count: 1,
            actions: vec![Some(action.into())],
            component_id: 3214,
        }
    }

    fn snapshot_with_bite_and_cure(tables: &CombatTables) -> GameSnapshot {
        let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
        let lobster = data.item_by_alias("lobster").unwrap();
        let antipoison = tables
            .potion(crate::combat::tables::PotionKind::Antipoison)
            .unwrap();
        let antipoison_id = antipoison.doses[0].unwrap().id;
        let at = api::WorldTile {
            x: 2600,
            z: 3200,
            level: 0,
        };
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_world(WorldStateView {
            map_base_x: 2560,
            map_base_z: 3160,
            members: true,
            ..WorldStateView::default()
        });
        snapshot.seed_local_player(local_player(at));
        snapshot.seed_stats(
            (0..25)
                .map(|index| StatView {
                    index,
                    name: String::new(),
                    effective: if index == 3 {
                        5
                    } else if index == 5 {
                        43
                    } else {
                        40
                    },
                    base: if index == 5 { 43 } else { 40 },
                    xp: 0,
                    used: api::snapshot::stat_used(index as usize),
                })
                .collect(),
        );
        snapshot.seed_varps(
            data.prayers()
                .iter()
                .map(|row| VarpView {
                    index: row.varp,
                    value: 0,
                })
                .collect(),
        );
        snapshot.seed_inventory(
            vec![
                held_item(lobster.id, lobster.name.as_deref().unwrap(), "Eat", 0),
                held_item(antipoison_id, "Antipoison", "Drink", 1),
            ],
            28,
        );
        snapshot.seed_equipment(Vec::new());
        snapshot.seed_npcs(Vec::new());
        snapshot.seed_players(Vec::new());
        snapshot.seed_projectiles(Vec::new());
        snapshot.seed_side_tabs(Vec::new(), 0);
        snapshot.seed_hitmarks(HitmarksView {
            marks: [HitmarkView {
                value: 0,
                kind: 0,
                cycle: 0,
            }; 4],
            loop_cycle: 0,
        });
        snapshot
    }

    #[test]
    fn abort_hold_respects_never_policy_when_antipoison_and_bite_are_due() {
        let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
        let tables = Arc::new(CombatTables::build(Arc::clone(&data)).unwrap());
        let snapshot = snapshot_with_bite_and_cure(&tables);
        let mut ledger_state = None;
        let handle = with_tick(&snapshot, &mut ledger_state, 0, |native| {
            native
                .actions
                .begin::<AbortHold>(
                    AbortHoldArgs {
                        tables: Arc::clone(&tables),
                        allow_prayer: false,
                    },
                    &mut native.cx,
                )
                .unwrap()
        });

        let outcome = with_tick(&snapshot, &mut ledger_state, 1, |native| {
            native.cx.condition = Some(api::condition::BotCondition {
                tick: 1,
                session: api::condition::SessionMark::Live,
                hp: 5,
                hp_max: 40,
                hp_known: true,
                poison: api::condition::PoisonState::Onset {
                    since: 1,
                    q: Some(11),
                    hit_since_line: false,
                },
                cures: api::condition::CureDoses {
                    antipoison: 1,
                    ..Default::default()
                },
                ..Default::default()
            });
            native.cx.cure_poison = crate::survival::CurePoison::Never;
            native.actions.poll(&handle, &mut native.cx)
        });
        assert!(matches!(outcome, Poll::Pending));
        let outbox = &ledger_state.as_ref().unwrap().outbox;
        assert_eq!(outbox.len(), 1);
        assert!(matches!(
            &outbox[0].effect,
            HostEffect::Interaction(InteractReq::Held { name, action, .. })
                if name == "Lobster" && action == "Eat"
        ));
        assert!(outbox.iter().all(|entry| !matches!(
            &entry.effect,
            HostEffect::Interaction(InteractReq::Held { action, .. }) if action == "Drink"
        )));
    }
}
