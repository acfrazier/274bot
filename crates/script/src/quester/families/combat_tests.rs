use super::*;
use crate::native::WalkEnd;
use crate::quester::compile::{
    compile_path, decode_args, CompileContext, PredicateContext, StepOutcome,
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
fn combat_owned_walk_crossing_and_protection_compile_independently() {
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
    let cx = fixture_compile_context(
        &data, &quests, &progress, &areas, &loadouts, &recipes, &path,
    );
    let args = serde_json::json!({
        "target": {"npc": "jailguard", "pick": "nearest", "not_targeting_others": true},
        "tactic": {"kind": "open", "style": "melee", "engage_radius": 12},
        "stand": {"tile": [3125, 3246, 0], "source": "native Prince live return-walk regression"},
        "lost_radius": 16,
        "kill_budget_ticks": 400,
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
            let mut input = args.clone();
            if !cross {
                input.as_object_mut().unwrap().remove("cross");
            }
            if let Some(guard) = guard {
                input["guard"] = guard;
            } else {
                input.as_object_mut().unwrap().remove("guard");
            }
            compile(decode_args::<CombatArgs>(&input).unwrap(), &cx).unwrap();
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
        let path = compile_path(bytes, &data, &quests).unwrap();
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
    let path = compile_path(
        include_bytes!("../../../paths/289/fixtures/combat_unattackable.json"),
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
    let path = compile_path(
        include_bytes!("../../../paths/289/fixtures/combat_unattackable.json"),
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
    let path = compile_path(
        include_bytes!("../../../paths/289/fixtures/combat_unattackable.json"),
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
    let path = compile_path(
        include_bytes!("../../../paths/289/fixtures/combat_unattackable.json"),
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
        let result = run.on_abort_walk(WalkReceipt {
            request_id: 7,
            evidence: aborted.evidence,
            end,
            blocked: None,
            detail: None,
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
fn return_and_abort_walks_retain_only_the_combat_steps_declared_permissions() {
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
                run.on_abort_walk(receipt)
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
            },
            LootItem {
                id: 2,
                name: Arc::from("Second drop"),
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
            },
            LootItem {
                id: 2,
                name: Arc::from("Second drop"),
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
