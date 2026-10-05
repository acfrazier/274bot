//! Ignored live proof cells for Quester's shared Path action families.
//!
//! Each cell uses `common/quester_live.rs` for its host pump, retained-cache
//! copy, fresh local account, status log, and paired CPU PNG/JSON evidence.
//! Only the tested account is seeded before the native Quester Path starts.
//!
//! Run one cell at a time with Engine A already running and the cache source
//! kept immutable. The common runner documents all required environment.
//!
//! ```text
//! LIVE=1 BOT_CPU=1 BOT_LIVE_NAME_PREFIX=qn WORLD_GAME_PORT=44594 WORLD_HTTP_PORT=1080 WORLD_NAV_PACK=<nav-pack> WORLD_ENGINE_DIR=<engine-A-dir> RS2B0T=<catalog-root> BOT_CACHE_DIR=/Volumes/dev-scratch/274bot-evidence/WALK-GUARD/cache-snapshots/37214163f1e6ceca LIVE_EVIDENCE_DIR=/Volumes/dev-scratch/274bot-evidence/QUESTER-FAMILIES-1 cargo test -p host-play --test quester_families_live --features "live-harness test-support" live_quester_families_expected_combat -- --ignored --nocapture --test-threads=1
//! ```
#![cfg(all(feature = "live-harness", feature = "test-support"))]

#[path = "common/quester_live.rs"]
mod quester_live;

use std::sync::Arc;
use std::time::Duration;

use api::game_data::SelectedGameData;
use api::quest_facts::QuestCatalog;
use api::selected::{ClientRevision, FactKey, RunKey};
use api::snapshot::{ActorKind, ActorTargetView, GameSnapshot, WorldTile};
use quester_live::{Cell, Mode, ObserveFamily, StartFamily};
use scenario::quester::{quester_stage, FixtureLoadout, QuesterStage};
use scenario::{Proof, Step, StepKind, Wait};
use script::native::{NativePhase, ScriptStatus, StatusValue};
use script::quester::compile::compile_path;
use script::quester::path::{
    PathDocument, PredicateDocument, ProgressColourDocument, ProgressDocument,
    ProgressRuleDocument, QuestBankDocument, QuestLoadoutDocument, SequenceDocument, StepDocument,
};
use script::quester::runner::Quester;
use serde_json::{json, Value};

const COOK_PATH_JSON: &str = include_str!("../../script/paths/289/cook.json");
const CAPTAIN_TAUNT: &str = "Very well, if you're challenging me, let's get on with it!";
const BLACKCOG_SOURCE: &str = "live Clock Tower spawn observed at [2613,9639,0] before Start";
const DESERT_ITEMS: &[(&str, i32)] = &[("lobster", 6)];

const DEATH_IOU_ITEMS: &[(&str, i32)] = &[("death_iou", 1)];
const CLOCK_TOWER_ITEMS: &[(&str, i32)] = &[("bucket_water", 1)];
const MAIN_DOCUMENT_ITEMS: &[(&str, i32)] = &[("piratemessage", 1), ("the_shield_of_arrav", 1)];

const DESERT_STAND: WorldTile = WorldTile {
    x: 3270,
    z: 3029,
    level: 0,
};
const PRIEST_DOOR_STAND: WorldTile = WorldTile {
    x: 3406,
    z: 3488,
    level: 0,
};
const SAFE_STAND: WorldTile = WorldTile {
    x: 3209,
    z: 3215,
    level: 0,
};
const BLACKCOG_TILE: WorldTile = WorldTile {
    x: 2613,
    z: 9639,
    level: 0,
};

fn live_data() -> (Arc<SelectedGameData>, Arc<QuestCatalog>) {
    let selected = api::game_data::for_revision(ClientRevision::R289).expect("selected R289 data");
    let quests = Arc::new(
        QuestCatalog::from_identity(selected.quest_identity()).expect("selected quest catalog"),
    );
    (selected, quests)
}

fn quest_varp(selected: &SelectedGameData, quest: &str) -> (i32, String) {
    let row = selected
        .quest_identity()
        .expect("selected R289 quest identity")
        .rows
        .iter()
        .find(|row| row.id.eq_ignore_ascii_case(quest))
        .unwrap_or_else(|| panic!("selected quest identity does not include {quest:?}"));
    (row.varp_id, row.varp.clone())
}

fn item_id(selected: &SelectedGameData, alias: &str) -> i32 {
    selected
        .item_by_alias(alias)
        .unwrap_or_else(|| panic!("selected R289 items do not include {alias:?}"))
        .id
}

fn insert_target_quest_seed(
    fixture: &mut scenario::quester::QuesterFixture,
    varp: &str,
    stage: i32,
) {
    let command = format!("setvar {varp} {stage}");
    let relog = fixture
        .scenario
        .steps
        .iter()
        .position(|step| step.name == "relog after permanent quest stage and loadout seed")
        .expect("path-backed fixture has a post-seed relog");
    let step_command = command.clone();
    fixture.scenario.steps.insert(
        relog,
        Step {
            name: "stage target quest before relog",
            kind: StepKind::Perform {
                send: Box::new(move |client, _| {
                    api::interact::cheat(client, &step_command).is_sent()
                }),
            },
            wait: Wait {
                arm: Proof::SideTabAvailable { index: 3 },
                budget_ticks: 100,
            },
        },
    );
    let teleport = fixture
        .seed_commands
        .len()
        .checked_sub(1)
        .expect("path-backed fixture records its teleport seed");
    fixture.seed_commands.insert(teleport, command);
}

fn make_cell(
    name: &'static str,
    (quest, display): (&'static str, &'static str),
    quest_stage: i32,
    items: &'static [(&'static str, i32)],
    stand: WorldTile,
    loadout: Option<FixtureLoadout<'_>>,
    selected: Arc<SelectedGameData>,
) -> (Cell, scenario::quester::QuesterSeed, Vec<String>) {
    // Cook is a Base40 profile-table key; the exact target quest state is
    // staged separately so every cell still begins at its authored target.
    let path = family_path(
        "cook",
        "Cook Base40 fixture root",
        "cook:0",
        Vec::new(),
        false,
    );
    let identity = selected
        .quest_identity()
        .expect("selected R289 quest identity")
        .rows
        .iter()
        .find(|row| row.id == "cook")
        .expect("selected R289 Cook identity");
    let mut fixture = quester_stage(QuesterStage {
        name,
        quest_display: "Cook's Assistant",
        path: &path,
        identity,
        selected: &selected,
        stage: "cook:0",
        loadout,
        extra_items: items,
        stand,
    })
    .unwrap_or_else(|error| panic!("build Cook-profile family fixture: {error}"));
    fixture.scenario.settings.deadline = Duration::from_secs(300);
    fixture
        .start_settings
        .insert("quests".into(), json!([quest]));
    if quest.eq_ignore_ascii_case("cook") {
        assert_eq!(quest_stage, 0, "Cook-profile path already seeds stage zero");
    } else {
        let (_, target_varp) = quest_varp(&selected, quest);
        insert_target_quest_seed(&mut fixture, &target_varp, quest_stage);
    }
    let seed = fixture.seed;
    let seed_commands = fixture.seed_commands;
    (
        Cell {
            quest,
            display,
            label: name.to_owned(),
            scenario: fixture.scenario,
            start_settings: fixture.start_settings,
            mode: Mode::Stage { expect: Vec::new() },
            observe_start: None,
        },
        seed,
        seed_commands,
    )
}

fn family_path(
    path_id: &str,
    display_name: &str,
    stage: &str,
    steps: Vec<StepDocument>,
    terminal: bool,
) -> PathDocument {
    // Keep a released R289 Path envelope and replace only its action sequence.
    let mut document: PathDocument =
        serde_json::from_str(COOK_PATH_JSON).expect("decode released Cook Path source");
    document.id = FactKey::new(path_id);
    document.display_name = display_name.to_owned();
    document.required.clear();
    document.tested_stats = None;
    let quest = document.quest.as_mut().expect("released Cook quest header");
    quest.members = false;
    quest.quest_points = 0;
    quest.requirements.clear();
    quest.items.clear();
    quest.acquire.clear();
    quest.bank = QuestBankDocument::Nearest(script::quester::path::NearestBankDocument::Nearest);
    quest.coin_float = 0;
    quest.loadouts.clear();
    quest.loadouts.insert(
        "family_melee".to_owned(),
        QuestLoadoutDocument {
            worn: [
                ("hat".to_owned(), "iron_full_helm".to_owned()),
                ("torso".to_owned(), "iron_platebody".to_owned()),
                ("legs".to_owned(), "iron_platelegs".to_owned()),
                ("lefthand".to_owned(), "iron_kiteshield".to_owned()),
                ("righthand".to_owned(), "iron_scimitar".to_owned()),
            ]
            .into_iter()
            .collect(),
            carry: Vec::new(),
        },
    );
    quest.areas.clear();
    quest.tools.clear();
    quest.owns_inventory = true;
    let stage_key = FactKey::new(stage);
    let role = document.roles.first_mut().expect("released Cook role");
    role.progress_binding = FactKey::new(&format!("journal:{path_id}"));
    role.progress = Some(ProgressDocument {
        colour: ProgressColourDocument {
            not_started: stage_key.clone(),
            in_progress: stage_key.clone(),
            complete: stage_key.clone(),
        },
        rules: vec![ProgressRuleDocument {
            stage: stage_key.clone(),
            all: Vec::new(),
            any: Vec::new(),
            not: Vec::new(),
            varp: Some(0),
        }],
        flags: Vec::new(),
        monotonic: false,
    });
    role.prelude.clear();
    role.sequences = vec![SequenceDocument {
        stage: FactKey::new(stage),
        required: Vec::new(),
        terminal,
        recovery_entry: None,
        steps,
    }];
    document
}

fn step(
    id: &str,
    kind: &str,
    args: Value,
    advances: bool,
    skip_if: PredicateDocument,
    settle: PredicateDocument,
) -> StepDocument {
    StepDocument {
        id: FactKey::new(id),
        kind: kind.to_owned(),
        version: 1,
        args,
        comment: None,
        advances: Some(advances),
        skip_if,
        settle,
    }
}

fn fact(kind: &str, args: Value) -> PredicateDocument {
    PredicateDocument::Fact {
        kind: kind.to_owned(),
        version: 1,
        args,
    }
}

fn empty_any() -> PredicateDocument {
    PredicateDocument::Any(Vec::new())
}

fn empty_all() -> PredicateDocument {
    PredicateDocument::All(Vec::new())
}

fn family_start(
    document: PathDocument,
    selected: Arc<SelectedGameData>,
    quests: Arc<QuestCatalog>,
) -> StartFamily {
    Box::new(move |handle, account, banks| {
        let bytes = serde_json::to_vec(&document).map_err(|error| error.to_string())?;
        let path = compile_path(&bytes, &selected, &quests)
            .map_err(|error| format!("compile inline family Path: {error:?}"))?;
        let script = Quester::new(
            RunKey {
                slot: 0,
                run: 0,
                session: 0,
            },
            path,
            Arc::clone(&selected),
            Arc::clone(&quests),
            Arc::clone(banks),
        );
        handle
            .start_test_script(account, Box::new(script), Some(Arc::clone(&selected)))
            .map(|_| ())
            .map_err(|error| format!("start inline family Path: {error:?}"))
    })
}

fn tile(snapshot: &GameSnapshot) -> Option<WorldTile> {
    snapshot
        .tile()
        .map(|(x, z, level)| WorldTile { x, z, level })
}

fn within(here: WorldTile, target: WorldTile, radius: i32) -> bool {
    here.level == target.level
        && (here.x - target.x).abs() <= radius
        && (here.z - target.z).abs() <= radius
}

fn item_count(snapshot: &GameSnapshot, id: i32) -> i32 {
    snapshot
        .inventory()
        .iter()
        .filter(|item| item.def.id == id)
        .map(|item| item.count)
        .sum()
}

fn stat_base(snapshot: &GameSnapshot, name: &str) -> Option<i32> {
    snapshot
        .stats()
        .iter()
        .find(|stat| stat.name.eq_ignore_ascii_case(name))
        .map(|stat| stat.base)
}

fn varp_value(snapshot: &GameSnapshot, id: i32) -> Option<i32> {
    snapshot
        .varps()
        .iter()
        .find(|varp| varp.index == id)
        .map(|varp| varp.value)
}

fn require_item(
    snapshot: &GameSnapshot,
    id: i32,
    expected: i32,
    alias: &str,
) -> Result<(), String> {
    let actual = item_count(snapshot, id);
    if actual != expected {
        return Err(format!(
            "fixture expected {expected} {alias}, observed {actual}"
        ));
    }
    Ok(())
}

fn inventory_receipt(snapshot: &GameSnapshot) -> Vec<Value> {
    snapshot
        .inventory()
        .iter()
        .map(|item| json!({"id": item.def.id, "name": &item.def.name, "count": item.count}))
        .collect()
}

fn dialogue_texts(snapshot: &GameSnapshot) -> Vec<String> {
    snapshot
        .chat_options()
        .iter()
        .map(|option| option.text.as_str())
        .chain(snapshot.chat_lines().iter().map(|line| line.text.as_str()))
        .map(str::to_owned)
        .collect()
}

fn remember_dialogue_texts(snapshot: &GameSnapshot, seen: &mut Vec<String>) {
    for text in dialogue_texts(snapshot) {
        if !seen.iter().any(|old| old.eq_ignore_ascii_case(&text)) {
            seen.push(text);
        }
    }
}

fn saw_text(seen: &[String], expected: &str) -> bool {
    seen.iter().any(|text| text.eq_ignore_ascii_case(expected))
}

fn status_text<'a>(status: &'a ScriptStatus, key: &str) -> Option<&'a str> {
    status
        .fields
        .iter()
        .find(|field| field.key == key)
        .and_then(|field| match &field.value {
            StatusValue::Text(value) => Some(value.as_ref()),
            _ => None,
        })
}

fn native_status_receipt(status: Option<&ScriptStatus>) -> Value {
    match status {
        Some(status) => json!({
            "phase": format!("{:?}", status.phase),
            "step_id": status_text(status, "step_id"),
            "failure": status.failure.as_ref().map(|failure| format!("{failure:?}")),
        }),
        None => Value::Null,
    }
}

fn common_observe_start(
    snapshot: &GameSnapshot,
    expected_tile: WorldTile,
) -> Result<WorldTile, String> {
    let here = tile(snapshot).ok_or("pre-Start snapshot has no local tile")?;
    if !within(here, expected_tile, 2) {
        return Err(format!(
            "fixture stood at {here:?}, expected near {expected_tile:?}"
        ));
    }
    Ok(here)
}

#[test]
#[ignore = "requires LIVE=1 and the already-running shared local Engine A"]
fn live_quester_families_expected_combat() {
    let (selected, quests) = live_data();
    let captain_id = selected
        .npc_by_config("desertminingcaptain")
        .expect("selected R289 desert mining captain config")
        .id;
    let (quest_varp_id, _) = quest_varp(&selected, "desertrescue");
    let (mut cell, seed, seed_commands) = make_cell(
        "live-quester-families-expected-combat",
        ("desertrescue", "The Tourist Trap"),
        3,
        DESERT_ITEMS,
        DESERT_STAND,
        Some(FixtureLoadout::Path("family_melee")),
        Arc::clone(&selected),
    );
    let start_seed = seed;
    let start_selected = Arc::clone(&selected);
    cell.observe_start = Some(Box::new(move |snapshot| {
        let seed_receipt = start_seed.observe_start(snapshot)?;
        let here = common_observe_start(snapshot, DESERT_STAND)?;
        if quest_varp_id >= 0 && varp_value(snapshot, quest_varp_id) != Some(3) {
            return Err(format!(
                "desertrescue fixture varp {quest_varp_id} was {:?}, expected 3",
                varp_value(snapshot, quest_varp_id)
            ));
        }
        let local = snapshot
            .local_player()
            .ok_or("pre-Start combat snapshot has no local player")?;
        if local.player.actor.in_combat {
            return Err("combat fixture began in combat before the talk step".into());
        }
        let captain_target_id = usize::try_from(captain_id)
            .map_err(|error| format!("invalid captain type id: {error}"))?;
        Ok(json!({
            "fixture": "desertrescue:3",
            "profile_seed": seed_receipt,
            "seed_commands": seed_commands,
            "tile": here,
            "quest_varp": [quest_varp_id, varp_value(snapshot, quest_varp_id)],
            "local_in_combat_before_talk": local.player.actor.in_combat,
            "captain_type_id": captain_target_id,
            "stats": [
                ["attack", stat_base(snapshot, "attack")],
                ["strength", stat_base(snapshot, "strength")],
                ["defence", stat_base(snapshot, "defence")],
                ["hitpoints", stat_base(snapshot, "hitpoints")],
                ["prayer", stat_base(snapshot, "prayer")],
            ],
            "inventory": inventory_receipt(snapshot),
            "selected_revision": start_selected.revision().to_string(),
        }))
    }));

    let in_combat = fact("in_combat", json!({}));
    let document = family_path(
        "cook",
        "Expected combat family fixture",
        "cook:0",
        vec![
            step(
                "captain-handoff",
                "talk",
                json!({
                    "npc": "desertminingcaptain",
                    "prefer": [
                        "Wow! A real captain!",
                        "I'd love to work for a tough guy like you!",
                        "Can't I do something for a strong Captain like you?",
                        "Sorry Sir, I don't think I can do that.",
                        "It's a funny captain who can't fight his own battles!"
                    ],
                    "expect_combat": {"npc": "desertminingcaptain"}
                }),
                false,
                in_combat.clone(),
                in_combat.clone(),
            ),
            step(
                "safe-combat-wait",
                "wait",
                json!({"until": in_combat, "max_ticks": 10_000}),
                false,
                empty_any(),
                in_combat,
            ),
        ],
        false,
    );
    let start = family_start(document, Arc::clone(&selected), Arc::clone(&quests));
    let mut dialogue_seen = Vec::new();
    let mut saw_wait_step = false;
    let observe: ObserveFamily = Box::new(move |snapshot, status| {
        if let Some(status) = status {
            saw_wait_step |= status_text(status, "step_id") == Some("safe-combat-wait");
        }
        remember_dialogue_texts(snapshot, &mut dialogue_seen);
        if !saw_wait_step || !saw_text(&dialogue_seen, CAPTAIN_TAUNT) {
            return Ok(None);
        }
        let Some(local) = snapshot.local_player() else {
            return Ok(None);
        };
        let local_index = local.player.index;
        let Some(target) = local.player.actor.target else {
            return Ok(None);
        };
        if !local.player.actor.in_combat || target.kind != ActorKind::Npc {
            return Ok(None);
        }
        let Some(captain) = snapshot.npcs().iter().find(|npc| {
            npc.r#type == Some(captain_id as usize)
                && npc.index == target.index
                && npc.target
                    == Some(ActorTargetView {
                        kind: ActorKind::Player,
                        index: local_index,
                    })
        }) else {
            return Ok(None);
        };
        Ok(Some(json!({
            "handoff_witness": "talk step completed; exact captain targets the local player",
            "talk_step_completed": saw_wait_step,
            "target_config": "desertminingcaptain",
            "npc_type_id": captain_id,
            "npc_index": captain.index,
            "local_player_index": local_index,
            "local_in_combat": local.player.actor.in_combat,
            "local_target": target,
            "captain_target": captain.target,
            "captain_in_combat_diagnostic": captain.in_combat,
            "captain_health_diagnostic": captain.health,
            "taunt_followup": CAPTAIN_TAUNT,
            "dialogue_text_seen": &dialogue_seen,
            "native_status": native_status_receipt(status),
        })))
    });
    quester_live::run_family(cell, start, observe)
        .unwrap_or_else(|error| panic!("expected combat family live proof failed: {error}"));
}

#[test]
#[ignore = "requires LIVE=1 and the already-running shared local Engine A"]
fn live_quester_families_loc_dialogue() {
    let (selected, quests) = live_data();
    let (quest_varp_id, _) = quest_varp(&selected, "priestperil");
    let door_id = selected
        .loc_by_config("priestperiltempledoorl")
        .expect("selected R289 Priest in Peril temple door config")
        .id;
    let (mut cell, seed, seed_commands) = make_cell(
        "live-quester-families-loc-dialogue",
        ("priestperil", "Priest in Peril"),
        1,
        &[],
        PRIEST_DOOR_STAND,
        None,
        Arc::clone(&selected),
    );
    let start_seed = seed;
    cell.observe_start = Some(Box::new(move |snapshot| {
        let seed_receipt = start_seed.observe_start(snapshot)?;
        let here = common_observe_start(snapshot, PRIEST_DOOR_STAND)?;
        if quest_varp_id >= 0 && varp_value(snapshot, quest_varp_id) != Some(1) {
            return Err(format!(
                "priestperil fixture varp {quest_varp_id} was {:?}, expected 1",
                varp_value(snapshot, quest_varp_id)
            ));
        }
        let door = snapshot
            .locs()
            .iter()
            .find(|loc| loc.id == door_id)
            .ok_or("pre-Start snapshot did not observe priestperiltempledoorl")?;
        if door.distance > 2
            || !door
                .actions
                .iter()
                .flatten()
                .any(|action| action.eq_ignore_ascii_case("Knock-at"))
        {
            return Err(format!(
                "priestperiltempledoorl was not a nearby Knock-at target: {door:?}"
            ));
        }
        Ok(json!({
            "fixture": "priestperil:1",
            "profile_seed": seed_receipt,
            "seed_commands": seed_commands,
            "tile": here,
            "quest_varp": [quest_varp_id, varp_value(snapshot, quest_varp_id)],
            "loc": {"config": "priestperiltempledoorl", "id": door.id, "name": &door.name,
                    "tile": door.tile, "distance": door.distance, "actions": &door.actions},
        }))
    }));
    let document = family_path(
        "cook",
        "Priest in Peril loc dialogue family fixture",
        "cook:0",
        vec![step(
            "knock-temple-door",
            "interact",
            json!({
                "target": {"loc": "priestperiltempledoorl"},
                "op": "Knock-at",
                "anchor": {
                    "tile": [3406, 3488, 0],
                    "source": "R289 Priest in Peril door witness at [3406,3488,0]"
                },
                "radius": 2,
                "wait_if_missing": true,
                "dialogue": {
                    "prefer": ["Roald sent me to check on Drezel.", "Sure."]
                }
            }),
            false,
            empty_any(),
            empty_all(),
        )],
        true,
    );
    let start = family_start(document, Arc::clone(&selected), Arc::clone(&quests));
    let mut dialogue_seen = Vec::new();
    let observe: ObserveFamily = Box::new(move |snapshot, status| {
        remember_dialogue_texts(snapshot, &mut dialogue_seen);
        let value = varp_value(snapshot, quest_varp_id);
        if value != Some(2)
            || !saw_text(&dialogue_seen, "Roald sent me to check on Drezel.")
            || !saw_text(&dialogue_seen, "Sure.")
        {
            return Ok(None);
        }
        Ok(Some(json!({
            "family_step": "interact",
            "target_config": "priestperiltempledoorl",
            "operation": "Knock-at",
            "preferred_dialogue_choices_observed": [
                "Roald sent me to check on Drezel.", "Sure."
            ],
            "dialogue_text_seen": &dialogue_seen,
            "quest_varp": [quest_varp_id, value],
            "native_status": native_status_receipt(status),
        })))
    });
    quester_live::run_family(cell, start, observe)
        .unwrap_or_else(|error| panic!("loc dialogue family live proof failed: {error}"));
}

#[test]
#[ignore = "requires LIVE=1 and the already-running shared local Engine A"]
fn live_quester_families_held() {
    let (selected, quests) = live_data();
    let iou_id = item_id(&selected, "death_iou");
    let combination_id = item_id(&selected, "death_combination");
    let (mut cell, seed, seed_commands) = make_cell(
        "live-quester-families-held-death-iou",
        ("cook", "Cook's Assistant"),
        0,
        DEATH_IOU_ITEMS,
        SAFE_STAND,
        None,
        Arc::clone(&selected),
    );
    let start_seed = seed;
    cell.observe_start = Some(Box::new(move |snapshot| {
        let seed_receipt = start_seed.observe_start(snapshot)?;
        let here = common_observe_start(snapshot, SAFE_STAND)?;
        require_item(snapshot, iou_id, 1, "death_iou")?;
        require_item(snapshot, combination_id, 0, "death_combination")?;
        if varp_value(snapshot, script::combat::OPTION_NODEF) != Some(0) {
            return Err(
                "held fixture requires observed auto-retaliate enabled before Start".into(),
            );
        }
        Ok(json!({
            "fixture": "held Death IOU read",
            "profile_seed": seed_receipt,
            "seed_commands": seed_commands,
            "tile": here,
            "held_target": {"config": "death_iou", "id": iou_id, "count": item_count(snapshot, iou_id)},
            "result_before": {"config": "death_combination", "id": combination_id,
                              "count": item_count(snapshot, combination_id)},
            "inventory": inventory_receipt(snapshot),
        }))
    }));
    let document = family_path(
        "cook",
        "Death IOU held read family fixture",
        "cook:0",
        vec![
            step(
                "disable-auto-retaliate",
                "setting",
                json!({"retaliate": false}),
                false,
                PredicateDocument::Any(vec![
                    fact("has_item", json!({"obj": "death_combination"})),
                    fact("retaliate", json!({"retaliate": false})),
                ]),
                fact("retaliate", json!({"retaliate": false})),
            ),
            step(
                "read-death-iou",
                "interact",
                json!({
                    "target": {"held": "death_iou"},
                    "op": "Read",
                    "dialogue": "continue"
                }),
                false,
                fact("has_item", json!({"obj": "death_combination"})),
                fact("has_item", json!({"obj": "death_combination"})),
            ),
            step(
                "enable-auto-retaliate",
                "setting",
                json!({"retaliate": true}),
                false,
                fact("retaliate", json!({"retaliate": true})),
                fact("retaliate", json!({"retaliate": true})),
            ),
        ],
        true,
    );
    let start = family_start(document, Arc::clone(&selected), Arc::clone(&quests));
    let mut saw_retaliate_off = false;
    let observe: ObserveFamily = Box::new(move |snapshot, status| {
        let retaliate = varp_value(snapshot, script::combat::OPTION_NODEF);
        saw_retaliate_off |= retaliate == Some(1);
        let source_count = item_count(snapshot, iou_id);
        let result_count = item_count(snapshot, combination_id);
        let path_completed = status.is_some_and(|status| status.phase == NativePhase::Complete);
        if !path_completed
            || source_count != 0
            || result_count != 1
            || !saw_retaliate_off
            || retaliate != Some(0)
            || snapshot.modals().main != -1
            || snapshot.modals().chat != -1
        {
            return Ok(None);
        }
        Ok(Some(json!({
            "family_step": "interact",
            "target": {"held": "death_iou", "id": iou_id},
            "operation": "Read",
            "result": {"config": "death_combination", "id": combination_id,
                       "count": result_count},
            "source_count_after": source_count,
            "setting": {"observed_off": saw_retaliate_off, "option_nodef_after": retaliate},
            "main_modal_after": snapshot.modals().main,
            "native_status": native_status_receipt(status),
        })))
    });
    quester_live::run_family(cell, start, observe)
        .unwrap_or_else(|error| panic!("held item family live proof failed: {error}"));
}

#[test]
#[ignore = "requires LIVE=1 and the already-running shared local Engine A"]
fn live_quester_families_ground_use_on() {
    let (selected, quests) = live_data();
    let water_id = item_id(&selected, "bucket_water");
    let cog_id = item_id(&selected, "blackcog");
    let (cog_varp_id, cog_varp) = quest_varp(&selected, "cog");
    assert_eq!(
        cog_varp, "~get_cog_progress",
        "selected Clock Tower varp alias"
    );
    let (mut cell, seed, seed_commands) = make_cell(
        "live-quester-families-ground-use-on",
        ("cog", "Clock Tower"),
        1,
        CLOCK_TOWER_ITEMS,
        BLACKCOG_TILE,
        None,
        Arc::clone(&selected),
    );
    let start_seed = seed;
    let observe_selected = Arc::clone(&selected);
    cell.observe_start = Some(Box::new(move |snapshot| {
        let seed_receipt = start_seed.observe_start(snapshot)?;
        let here = common_observe_start(snapshot, BLACKCOG_TILE)?;
        require_item(snapshot, water_id, 1, "bucket_water")?;
        require_item(snapshot, cog_id, 0, "blackcog")?;
        // Selected R289 exports blackcog's exact item identity but no
        // source-map spawn row. Fail closed unless the real pre-Start scene
        // snapshot proves that exact item at the authored tile and exposes Take.
        let ground = snapshot
            .ground_items()
            .iter()
            .find(|item| item.def.id == cog_id && item.tile == BLACKCOG_TILE)
            .ok_or("pre-Start live scene lacks blackcog at [2613,9639,0]")?;
        if !ground
            .actions
            .iter()
            .flatten()
            .any(|action| action.eq_ignore_ascii_case("Take"))
        {
            return Err(format!(
                "blackcog at {:?} did not expose Take: {:?}",
                ground.tile, ground.actions
            ));
        }
        Ok(json!({
            "fixture": "cog:1",
            "profile_seed": seed_receipt,
            "seed_commands": seed_commands,
            "tile": here,
            "quest_varp": [cog_varp_id, if cog_varp_id >= 0 { varp_value(snapshot, cog_varp_id) } else { None }],
            "bucket_water": {"id": water_id, "count": item_count(snapshot, water_id)},
            "target": {"config": "blackcog", "id": ground.def.id, "count": ground.count,
                       "tile": ground.tile, "distance": ground.distance, "actions": &ground.actions},
            "target_source": BLACKCOG_SOURCE,
            "selected_revision": observe_selected.revision().to_string(),
        }))
    }));
    let document = family_path(
        "cook",
        "Clock Tower ground use-on family fixture",
        "cook:0",
        vec![
            step(
                "use-water-on-blackcog",
                "use_on",
                json!({
                    "item": "bucket_water",
                    "target": {
                        "ground": "blackcog",
                        "tile": [2613, 9639, 0],
                        "source": BLACKCOG_SOURCE
                    },
                    "dialogue": "continue"
                }),
                false,
                empty_any(),
                empty_all(),
            ),
            step(
                "take-blackcog",
                "interact",
                json!({
                    "target": {"ground": "blackcog"},
                    "op": "Take",
                    "anchor": {
                        "tile": [2613, 9639, 0],
                        "source": BLACKCOG_SOURCE
                    },
                    "radius": 2,
                    "wait_if_missing": true
                }),
                false,
                fact("has_item", json!({"obj": "blackcog"})),
                fact("has_item", json!({"obj": "blackcog"})),
            ),
        ],
        true,
    );
    let start = family_start(document, Arc::clone(&selected), Arc::clone(&quests));
    let mut saw_take_step = false;
    let observe: ObserveFamily = Box::new(move |snapshot, status| {
        if let Some(status) = status {
            saw_take_step |= status_text(status, "step_id") == Some("take-blackcog");
        }
        let held_cog = item_count(snapshot, cog_id);
        if !saw_take_step || held_cog != 1 {
            return Ok(None);
        }
        Ok(Some(json!({
            "family_steps": ["use-water-on-blackcog", "take-blackcog"],
            "use_on": {
                "item": "bucket_water",
                "target": {"ground": "blackcog", "id": cog_id,
                           "tile": BLACKCOG_TILE, "source": BLACKCOG_SOURCE},
                "dialogue": "continue"
            },
            "take_result": {"config": "blackcog", "id": cog_id, "count": held_cog},
            "bucket_water_after": item_count(snapshot, water_id),
            "native_status": native_status_receipt(status),
        })))
    });
    quester_live::run_family(cell, start, observe)
        .unwrap_or_else(|error| panic!("ground use-on family live proof failed: {error}"));
}

#[test]
#[ignore = "requires LIVE=1 and the already-running shared local Engine A"]
fn live_quester_families_main_documents() {
    let (selected, quests) = live_data();
    let ui = *selected
        .dialogue_ui()
        .expect("selected main-document controls");
    // Ordinary quest sequences reselect from observable skip predicates.
    // These informational reads do not change inventory or quest state, so
    // prove each family action in its own cell instead of inventing a cursor.
    for (label, alias, root, fragments) in [
        (
            "live-quester-families-main-scroll",
            "piratemessage",
            ui.scroll_root,
            &["Visit the city of the White Knights"][..],
        ),
        (
            "live-quester-families-main-book",
            "the_shield_of_arrav",
            ui.book_root,
            &["Arrav is probably", "year 143", "This tactic did not work"][..],
        ),
    ] {
        let id = item_id(&selected, alias);
        let (mut cell, seed, seed_commands) = make_cell(
            label,
            ("cook", "Cook's Assistant"),
            0,
            MAIN_DOCUMENT_ITEMS,
            SAFE_STAND,
            None,
            Arc::clone(&selected),
        );
        cell.observe_start = Some(Box::new(move |snapshot| {
            let seed_receipt = seed.observe_start(snapshot)?;
            let here = common_observe_start(snapshot, SAFE_STAND)?;
            require_item(snapshot, id, 1, alias)?;
            if snapshot.modals().main != -1 || snapshot.modals().chat != -1 {
                return Err(
                    "main-document fixture requires both modals closed before Start".into(),
                );
            }
            Ok(json!({
                "fixture": alias,
                "profile_seed": seed_receipt,
                "seed_commands": seed_commands,
                "tile": here,
                "inventory": inventory_receipt(snapshot),
            }))
        }));
        let document = family_path(
            "cook",
            "Selected main-document continuation fixture",
            "cook:0",
            vec![step(
                "read-document",
                "interact",
                json!({
                    "target": {"held": alias},
                    "op": "Read",
                    "dialogue": "continue"
                }),
                false,
                empty_any(),
                empty_all(),
            )],
            false,
        );
        let start = family_start(document, Arc::clone(&selected), Arc::clone(&quests));
        let mut pages_seen = vec![false; fragments.len()];
        let mut saw_root = false;
        let observe: ObserveFamily = Box::new(move |snapshot, status| {
            let main = snapshot.modals().main;
            if main == root {
                saw_root = true;
                for widget in snapshot.widgets().iter().filter(|widget| {
                    widget.root == api::snapshot::WidgetRoot::Main
                        && widget.root_component_id == root
                        && !widget.hidden
                }) {
                    if let Some(text) = &widget.text {
                        for (seen, fragment) in pages_seen.iter_mut().zip(fragments) {
                            *seen |= text.contains(fragment);
                        }
                    }
                }
            }
            if !saw_root
                || !pages_seen.iter().all(|seen| *seen)
                || main != -1
                || snapshot.modals().chat != -1
                || item_count(snapshot, id) != 1
                || !status.is_some_and(|status| {
                    status.failure.is_none()
                        && status_text(status, "step_id") == Some("read-document")
                })
            {
                return Ok(None);
            }
            Ok(Some(json!({
                "family_step": "read-document",
                "target": {"held": alias, "id": id},
                "source": [
                    "quests/quest_hunt/scripts/pirate_message.rs2:23-27",
                    "quests/quest_blackarmgang/scripts/arrav_book.rs2:2-18",
                    "general/scripts/book.rs2:31-56"
                ],
                "main_root_observed": root,
                "page_fragments": fragments,
                "pages_observed": pages_seen,
                "main_modal_after": main,
                "chat_modal_after": snapshot.modals().chat,
                "native_status": native_status_receipt(status),
            })))
        });
        quester_live::run_family(cell, start, observe).unwrap_or_else(|error| {
            panic!("{alias} main-document family live proof failed: {error}")
        });
    }
}
