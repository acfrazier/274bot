use super::*;
use crate::native::WalkEnd;
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
        bank_items: &[],
        keep_ids: &[],
        areas: &areas,
        loadouts: &loadouts,
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
                assert_eq!(request.radius, 1);
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
        bank_items: &[],
        keep_ids: &[],
        areas,
        loadouts,
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
    let loadouts = LoadoutOverlay::from_default_store(
        Arc::<[crate::loadouts_store::Loadout]>::from(Vec::new()),
    );
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
            bank: &crate::quester::bank_memo::BankMemo::default(),
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
            bank: &crate::quester::bank_memo::BankMemo::default(),
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
            bank: &crate::quester::bank_memo::BankMemo::default(),
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
        local.player.actor.tile = api::WorldTile {
            x,
            z: 3346,
            level: 0,
        };
        snapshot.seed_local_player(local.clone());
        super::super::tests::with_tick(&snapshot, &mut ledger, x as u64, |tick| {
            let context = PredicateContext {
                cx: &tick.cx,
                pairs: tick.pairs,
                quests: &quests,
                progress: &[],
                required_after: tick.cx.evidence(),
                chat_since: 0,
                bank: &crate::quester::bank_memo::BankMemo::default(),
                outcome: None,
            };
            let crate::quester::select::SelectionDecision::Selected(selected) =
                crate::quester::select::select(&path, 0, &context)
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
            bank: &crate::quester::bank_memo::BankMemo::default(),
            outcome: Some(&outcome),
        };
        let crate::quester::select::SelectionDecision::Selected(selected) =
            crate::quester::select::select(&path, 0, &context)
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
            bank: &crate::quester::bank_memo::BankMemo::default(),
            outcome: Some(&outcome),
        };
        let crate::quester::select::SelectionDecision::Selected(selected) =
            crate::quester::select::select(&path, 0, &context)
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
    local.player.actor.tile = api::WorldTile {
        x: 3093,
        z: 3243,
        level: 0,
    };
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
            bank: &crate::quester::bank_memo::BankMemo::default(),
            outcome: Some(&outcome),
        };
        assert!(
            matches!(
                crate::quester::select::select(&path, 0, &context),
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
        walk_outcome_seq_at_begin: 0,
        raised_prayers: crate::combat::RaisedPrayers::empty(),
        finish: None,
        finish_target_index: None,
        finish_ticks_elapsed: 0,
        finish_last_tick: None,
        trace_event: None,
    }
}
pub(crate) fn policy_s2_run_for_runner(cx: &mut StepContext<'_, '_>) -> Box<dyn StepRun> {
    let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let mut run = combat_test_run(imp_target(&data), None, None, Vec::new());
    run.begin_combat(cx).unwrap();
    Box::new(run)
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
            choose: Some(4),
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
    with_step_context_at_walk_seq(snapshot, ledger, tick, 0, f)
}

fn with_step_context_at_walk_seq<R>(
    snapshot: &GameSnapshot,
    ledger: &mut Option<Box<crate::native::ledger::Ledger>>,
    tick: u64,
    walk_outcome_seq: u64,
    f: impl FnOnce(&mut StepContext<'_, '_>) -> R,
) -> R {
    super::super::tests::with_tick(snapshot, ledger, tick, |native| {
        native.cx.observed_walk_outcome_seq = walk_outcome_seq;
        let quests = QuestCatalog::empty();
        let required_after = native.cx.evidence();
        let bank = crate::quester::bank_memo::BankMemo::default();
        let banks = Arc::new(api::named_banks::NamedBankFacts::empty());
        f(&mut StepContext {
            tick: native,
            quests: &quests,
            progress: &[],
            required_after,
            bank: &bank,
            banks: &banks,
            choices: &crate::quester::choices::QuestChoices::default(),
        })
    })
}

fn publish_walk_outcome(seq: u64, cancel_reason: crate::isolate_fb::WalkCancelReason) {
    crate::observed::replace(seq, true, |post| {
        post.walk_outcome(crate::observed::WalkOutcome {
            seq,
            generation: 1,
            request_id: 1,
            failed: cancel_reason == crate::isolate_fb::WalkCancelReason::UserInput,
            tile: crate::observed::Tile {
                x: 3200,
                z: 3200,
                level: 0,
            },
            radius: 0,
            allow_teleports: false,
            blocked: false,
        })
        .walk_outcome_cancel_reason(cancel_reason);
    });
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
                    request_id: 7,
                    evidence: aborted.evidence,
                    end,
                    blocked: None,
                    detail: None,
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
        let op = guard.tick(&view);
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
        guard.tick(&cx.tick.cx.snapshot())
    });
    assert!(matches!(falling_hp, Some(GuardOp::Eat { .. })));
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
    let request_id = ledger
        .as_ref()
        .unwrap()
        .outbox
        .last()
        .unwrap()
        .request_id
        .get();
    ledger.as_mut().unwrap().outbox.clear();
    ledger.as_mut().unwrap().walk = Some(WalkReceipt {
        request_id,
        evidence: EvidenceStamp {
            run: RunKey {
                slot: 1,
                run: 1,
                session: 1,
            },
            tick: 13,
            sequence: 13,
        },
        end: WalkEnd::Failed,
        blocked: None,
        detail: Some(Arc::from("route fixture failure")),
    });
    assert!(with_step_context(&snapshot, &mut ledger, 13, |cx| run.poll(cx)).is_pending());

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
fn failed_abort_walk_escapes_once_when_food_is_exhausted() {
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
    let failed_walk = ledger
        .as_ref()
        .unwrap()
        .outbox
        .last()
        .unwrap()
        .request_id
        .get();
    ledger.as_mut().unwrap().outbox.clear();
    ledger.as_mut().unwrap().walk = Some(WalkReceipt {
        request_id: failed_walk,
        evidence: EvidenceStamp {
            run: RunKey {
                slot: 1,
                run: 1,
                session: 1,
            },
            tick: 13,
            sequence: 13,
        },
        end: WalkEnd::Failed,
        blocked: None,
        detail: Some(Arc::from("route fixture failure")),
    });
    assert!(with_step_context(&snapshot, &mut ledger, 13, |cx| run.poll(cx)).is_pending());
    assert!(with_step_context(&snapshot, &mut ledger, 14, |cx| run.poll(cx)).is_pending());
    assert!(with_step_context(&snapshot, &mut ledger, 15, |cx| run.poll(cx)).is_pending());

    let outbox = &ledger.as_ref().unwrap().outbox;
    assert_eq!(
        outbox.len(),
        1,
        "supply exhaustion must start one escape walk"
    );
    let crate::native::HostEffect::Walk(request) = &outbox[0].effect else {
        panic!("supply exhaustion must escape with one native walk");
    };
    assert!(!request.protect);
    assert!(
        !request.food_guard,
        "the exhausted escape walk is unguarded"
    );
    assert!(!request.allow.prayer);
    assert!(!request.allow.food);
    let escape_walk = outbox[0].request_id.get();
    ledger.as_mut().unwrap().outbox.clear();
    ledger.as_mut().unwrap().walk = Some(WalkReceipt {
        request_id: escape_walk,
        evidence: EvidenceStamp {
            run: RunKey {
                slot: 1,
                run: 1,
                session: 1,
            },
            tick: 16,
            sequence: 16,
        },
        end: WalkEnd::Arrived,
        blocked: None,
        detail: None,
    });
    assert!(matches!(
        with_step_context(&snapshot, &mut ledger, 16, |cx| run.poll(cx)),
        Poll::Ready(Err(ActionError::Blocked(reason))) if reason.contains("escape")
    ));
    assert!(ledger.as_ref().unwrap().outbox.is_empty());
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
    let failed_walk = ledger
        .as_ref()
        .unwrap()
        .outbox
        .last()
        .unwrap()
        .request_id
        .get();
    ledger.as_mut().unwrap().outbox.clear();
    ledger.as_mut().unwrap().walk = Some(WalkReceipt {
        request_id: failed_walk,
        evidence: EvidenceStamp {
            run: RunKey {
                slot: 1,
                run: 1,
                session: 1,
            },
            tick: 13,
            sequence: 13,
        },
        end: WalkEnd::Failed,
        blocked: None,
        detail: Some(Arc::from("route fixture failure")),
    });
    assert!(with_step_context(snapshot, &mut ledger, 13, |cx| run.poll(cx)).is_pending());
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
    let wolf_like = data
        .npc_names()
        .unwrap()
        .rows
        .iter()
        .find(|row| crate::combat::facts::npc_max_hit(row) == Some(4))
        .expect("selected content includes a known-hit lower threat");
    let mut snapshot = GameSnapshot::new();
    let mut newcomer = aborting_npc(
        9,
        wolf_like.id,
        api::WorldTile {
            x: 3000,
            z: 2999,
            level: 0,
        },
    );
    // The live R2 shape: the Warlord row is gone; another NPC attacks.
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
    assert_eq!(walks[0].x, here.x, "no reference to the vanished Warlord");
    assert!(
        walks[0].z > here.z,
        "the escape runs north, away from the new attacker"
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
                request_id: 1,
                evidence: cx.tick.cx.evidence(),
                end: WalkEnd::Arrived,
                blocked: None,
                detail: None,
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
            assert_eq!(request.radius, 1);
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
                request_id: 1,
                evidence: cx.tick.cx.evidence(),
                end: WalkEnd::RouteEnded,
                blocked: None,
                detail: None,
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
                request_id: 1,
                evidence: cx.tick.cx.evidence(),
                end: WalkEnd::NeedsEvidence(Arc::clone(&gates)),
                blocked: None,
                detail: None,
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
                request_id: 1,
                evidence: cx.tick.cx.evidence(),
                end: WalkEnd::UserInput,
                blocked: None,
                detail: None,
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
fn manual_movement_baseline_covers_return_and_loot_sublegs() {
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
    run.walk_outcome_seq_at_begin = 40;
    publish_walk_outcome(40, crate::isolate_fb::WalkCancelReason::None);

    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    snapshot.seed_inventory(Vec::new(), 28);
    let mut ledger = None;
    assert!(
        with_step_context_at_walk_seq(&snapshot, &mut ledger, 12, 40, |cx| {
            run.on_combat_report(report(CombatEnd::TargetGone), cx)
        })
        .is_pending()
    );

    publish_walk_outcome(41, crate::isolate_fb::WalkCancelReason::None);
    assert!(
        with_step_context_at_walk_seq(&snapshot, &mut ledger, 13, 41, |cx| {
            run.on_return_walk(
                WalkReceipt {
                    request_id: 1,
                    evidence: cx.tick.cx.evidence(),
                    end: WalkEnd::Arrived,
                    blocked: None,
                    detail: None,
                },
                cx,
            )
        })
        .is_pending()
    );
    assert_eq!(run.target_gone_restarts, 1);

    assert!(
        with_step_context_at_walk_seq(&snapshot, &mut ledger, 14, 41, |cx| {
            run.on_combat_report(report(CombatEnd::Killed), cx)
        })
        .is_pending()
    );
    assert!(
        with_step_context_at_walk_seq(&snapshot, &mut ledger, 15, 41, |cx| { run.poll(cx) })
            .is_pending()
    );
    assert_eq!(run.loot_index, 1);
    assert!(matches!(run.action.as_ref(), Some(Action::Loot(_))));

    publish_walk_outcome(42, crate::isolate_fb::WalkCancelReason::UserInput);
    let result = with_step_context_at_walk_seq(&snapshot, &mut ledger, 16, 42, |cx| run.poll(cx));
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
            action,
            ..
        }) if *x == stand.x && *z == stand.z && action == "Take"
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

    for (held_bones, expected_index, expected_tile) in [(1, 1, bones_tile), (25, 2, bead_tile)] {
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
                x, z, action, ..
            }) if *x == expected_tile.x && *z == expected_tile.z && action == "Take"
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
            action,
            ..
        }) if *x == first_tile.x && *z == first_tile.z && action == "Take"
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
    });

    assert!(with_step_context(&snapshot, &mut ledger, 15, |cx| run.poll(cx)).is_pending());
    assert_eq!(run.loot_index, 2, "the unreachable item is skipped");
    assert!(matches!(run.action.as_ref(), Some(Action::Loot(_))));
    assert!(matches!(
        &ledger.as_ref().unwrap().outbox.last().unwrap().effect,
        crate::native::HostEffect::Interaction(crate::shim::InteractReq::Obj {
            x,
            z,
            action,
            ..
        }) if *x == second_tile.x && *z == second_tile.z && action == "Take"
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
    assert!(with_step_context(&snapshot, &mut ledger, 3, |cx| run.poll(cx)).is_pending());
    assert!(matches!(run.action.as_ref(), Some(Action::Dialogue(_))));
    assert!(ledger.as_ref().unwrap().outbox.is_empty());

    assert!(with_step_context(&snapshot, &mut ledger, 4, |cx| run.poll(cx)).is_pending());
    assert!(matches!(
        &ledger.as_ref().unwrap().outbox.last().unwrap().effect,
        crate::native::HostEffect::Interaction(crate::shim::InteractReq::Answer { option: 4 })
    ));
    snapshot.seed_chat_modal(4883, vec!["The demon is weakened.".into()]);
    snapshot.seed_chat_options(vec![], 4899);
    assert!(with_step_context(&snapshot, &mut ledger, 5, |cx| run.poll(cx)).is_pending());
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
    for tick in 8..13 {
        assert!(
            with_step_context(&snapshot, &mut ledger, tick, |cx| run.poll(cx)).is_pending(),
            "finish must wait for the shared four-tick dialogue gap"
        );
    }
    let completed = with_step_context(&snapshot, &mut ledger, 13, |cx| run.poll(cx));
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
        Poll::Ready(Err(ActionError::Blocked(reason))) if reason.contains("interrupted")
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
    assert!(with_step_context(&snapshot, &mut ledger, 1, |cx| run.poll(cx)).is_pending());
    assert!(with_step_context(&snapshot, &mut ledger, 2, |cx| run.poll(cx)).is_pending());
    assert!(matches!(
        &ledger.as_ref().unwrap().outbox.last().unwrap().effect,
        crate::native::HostEffect::Interaction(crate::shim::InteractReq::Answer { option: 2 })
    ));

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
    assert!(
        with_step_context(&unmatched_snapshot, &mut unmatched_ledger, 1, |cx| {
            unmatched.poll(cx)
        })
        .is_pending()
    );
    let failed = with_step_context(&unmatched_snapshot, &mut unmatched_ledger, 2, |cx| {
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
        assert!(with_step_context(&snapshot, &mut ledger, 1, |cx| run.poll(cx)).is_pending());
        assert!(
            matches!(
                with_step_context(&snapshot, &mut ledger, 2, |cx| run.poll(cx)),
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
