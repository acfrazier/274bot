use super::*;
use crate::quester::compile::{compile_path, CompileContext, PredicateContext, StepOutcome};
use crate::quester::loadouts::LoadoutOverlay;
use crate::quester::progress::CompiledProgress;
use api::game_data::SelectedGameData;
use api::quest_facts::QuestCatalog;
use api::quest_progress::EvidenceStamp;
use api::selected::{ClientRevision, FactKey, RunKey, Truth};
use api::snapshot::{GameSnapshot, QuestListStatus, QuestStatusView, SnapshotView};
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

fn fixture_compile_context<'a>(
    data: &'a SelectedGameData,
    quests: &'a QuestCatalog,
    progress: &'a CompiledProgress,
    areas: &'a HashMap<String, Vec<[i32; 5]>>,
    loadouts: &'a LoadoutOverlay,
    recipes: &'a HashMap<String, Vec<crate::quester::families::CompiledAcquireStep>>,
    path: &'a FactKey,
) -> CompileContext<'a> {
    CompileContext {
        path,
        progress,
        selected: data,
        quests,
        gathering: None,
        bank: None,
        bank_items: &[],
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
    let predicate =
        compile_end_predicate(&serde_json::json!({ "end": "aborted_unattackable" }), &cx).unwrap();
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
        assert_eq!(predicate.evaluate(&context), Truth::Unknown);
    });
}
