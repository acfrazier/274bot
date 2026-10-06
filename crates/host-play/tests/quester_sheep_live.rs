//! Live proof for the unchanged bundled Sheep Path, using the shared Quester lifecycle runner.
//!
//! The fixture deposits shears and wool through the real Draynor bank before
//! the final relog. At the one native Start the pack must be empty, the bank
//! closed, and Sheep visibly NotStarted. The common runner captures status,
//! CPU PNGs, JSON frame receipts, and a durable lifecycle receipt under
//! LIVE_EVIDENCE_DIR.
//!
//! ```text
//! LIVE=1 BOT_CPU=1 BOT_LIVE_NAME_PREFIX=ck WORLD_GAME_PORT=44594 WORLD_HTTP_PORT=1080 WORLD_NAV_PACK=<nav-pack> WORLD_ENGINE_DIR=<engine-A-dir> RS2B0T=<catalog-root> BOT_CACHE_DIR=<owned-writable-APFS-cache-clone> LIVE_EVIDENCE_DIR=<external-evidence-dir> cargo test -p host-play --test quester_sheep_live --features "live-harness test-support" live_quester_sheep_bank_loop -- --ignored --nocapture --test-threads=1
//! ```

#![cfg(all(feature = "live-harness", feature = "test-support"))]

#[path = "common/quester_live.rs"]
mod quester_live;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use api::game_data::SelectedGameData;
use api::interact::{ActionSpec, Interactions, OpTarget, SendResult};
use api::quest_facts::QuestCatalog;
use api::selected::{ClientRevision, RunKey};
use api::snapshot::{GameSnapshot, QuestListStatus, WorldTile};
use client::client::skill::Skill;
use quester_live::{Cell, Mode, ObserveFamily, StartFamily};
use scenario::{Proof, Step, StepKind, Wait};
use script::native::{NativePhase, ScriptStatus, StatusValue};
use script::quester::compile::compile_path;
use script::quester::path::PathDocument;
use script::quester::runner::Quester;
use serde_json::{json, Value};

const SHEEP_PATH_BYTES: &[u8] = include_bytes!("../../script/paths/289/sheep.json");
const SHEARS_QTY: i32 = 1;
const WOOL_QTY: i32 = 20;
const BALLS_OF_WOOL_QTY: i32 = 20;
const DRAYNOR_BANK_APPROACH: WorldTile = WorldTile {
    x: 3092,
    z: 3243,
    level: 0,
};
const DRAYNOR_BANK_BOOTH: WorldTile = WorldTile {
    x: 3091,
    z: 3243,
    level: 0,
};
const DRAYNOR_BANK_BOOTH_ID: i32 = 2213;
const SHEEP_START_STAND: WorldTile = WorldTile {
    x: 3197,
    z: 3266,
    level: 0,
};
const DEADLINE: Duration = Duration::from_secs(1800);

#[derive(Debug, Clone)]
struct BankSeedReceipt {
    tile: WorldTile,
    bank_loaded: bool,
    empty_bank_loaded: bool,
    session_generation: u64,
    snapshot_generation: Option<u64>,
    shears: i32,
    wool: i32,
    inventory_empty: bool,
}

#[derive(Debug, Default, Clone)]
struct BankSeedState {
    closed_generation: Option<u64>,
    empty_open_generation: Option<u64>,
    empty_open_receipt: Option<Value>,
    receipt: Option<BankSeedReceipt>,
}

struct SheepCase {
    cell: Cell,
    bank_seed: Arc<Mutex<BankSeedState>>,
    shears_id: i32,
    wool_id: i32,
    balls_of_wool_id: i32,
}

fn selected_data() -> Result<Arc<SelectedGameData>, String> {
    api::game_data::for_revision(ClientRevision::R289)
        .map_err(|error| format!("selected R289 game data: {error}"))
}

fn item_id(selected: &SelectedGameData, alias: &str) -> Result<i32, String> {
    selected
        .item_by_alias(alias)
        .map(|item| item.id)
        .ok_or_else(|| format!("selected R289 data has no item alias {alias:?}"))
}

fn item_count(items: &[api::snapshot::ItemView], id: i32) -> i32 {
    items
        .iter()
        .filter(|item| item.def.id == id && item.count > 0)
        .map(|item| item.count)
        .sum()
}

fn inventory_receipt(snapshot: &GameSnapshot) -> Vec<Value> {
    snapshot
        .inventory()
        .iter()
        .filter(|item| item.count > 0)
        .map(|item| json!({"id": item.def.id, "name": &item.def.name, "count": item.count}))
        .collect()
}

fn inventory_empty(snapshot: &GameSnapshot) -> bool {
    snapshot.inventory().iter().all(|item| item.count <= 0)
}

fn tile(snapshot: &GameSnapshot) -> Option<WorldTile> {
    snapshot
        .tile()
        .map(|(x, z, level)| WorldTile { x, z, level })
}

fn near(here: WorldTile, expected: WorldTile, radius: i32) -> bool {
    here.level == expected.level
        && (here.x - expected.x).abs() <= radius
        && (here.z - expected.z).abs() <= radius
}

fn tile_json(tile: WorldTile) -> Value {
    json!([tile.x, tile.z, tile.level])
}

fn sheep_quest_status(snapshot: &GameSnapshot) -> Option<QuestListStatus> {
    snapshot
        .quest_statuses()
        .iter()
        .find(|row| row.name.trim().eq_ignore_ascii_case("Sheep Shearer"))
        .map(|row| row.status())
}

fn status_field<'a>(status: &'a ScriptStatus, key: &str) -> Option<&'a StatusValue> {
    status
        .fields
        .iter()
        .find(|field| field.key == key)
        .map(|field| &field.value)
}

fn status_text<'a>(status: &'a ScriptStatus, key: &str) -> Option<&'a str> {
    match status_field(status, key)? {
        StatusValue::Text(value) => Some(value.as_ref()),
        _ => None,
    }
}

fn status_step(status: &ScriptStatus) -> Option<&str> {
    ["step_id", "child_step_id"]
        .into_iter()
        .filter_map(|key| status_text(status, key))
        .find(|step| !step.is_empty())
}

fn bank_deposit_all_operation(actions: &[Option<String>]) -> Option<i32> {
    actions
        .iter()
        .position(|action| {
            action.as_deref().is_some_and(|label| {
                let normalized: String = label
                    .to_ascii_lowercase()
                    .chars()
                    .filter(|character| {
                        !character.is_whitespace() && *character != '-' && *character != '_'
                    })
                    .collect();
                normalized.contains("deposit") && normalized.contains("all")
            })
        })
        .map(|slot| slot as i32 + 1)
}

fn bank_seed_receipt_json(receipt: &BankSeedReceipt) -> Value {
    json!({
        "tile": tile_json(receipt.tile),
        "bank_loaded": receipt.bank_loaded,
        "empty_bank_loaded_before_deposit": receipt.empty_bank_loaded,
        "bank_session_generation": receipt.session_generation,
        "bank_snapshot_generation": receipt.snapshot_generation,
        "bank": {"shears": receipt.shears, "wool": receipt.wool},
        "inventory_empty": receipt.inventory_empty,
        "seeded_before_start": true,
    })
}

#[derive(Debug, Clone)]
struct SheepStartFacts {
    inventory: Vec<(i32, i32)>,
    equipment: Vec<i32>,
    bank_open: bool,
    sheep: Option<QuestListStatus>,
    tile: Option<WorldTile>,
}

fn validate_start_gate(facts: &SheepStartFacts, bank_seed: &BankSeedReceipt) -> Result<(), String> {
    if !facts.inventory.is_empty() {
        return Err(format!(
            "Sheep Start inventory is not empty: {:?}",
            facts.inventory
        ));
    }
    if !facts.equipment.is_empty() {
        return Err(format!(
            "Sheep Start must have no combat/equipment loadout: {:?}",
            facts.equipment
        ));
    }
    if facts.bank_open {
        return Err("Sheep Start requires the bank modal to be closed".into());
    }
    if facts.sheep != Some(QuestListStatus::NotStarted) {
        return Err(format!(
            "Sheep Start requires observed NotStarted quest colour, got {:?}",
            facts.sheep
        ));
    }
    if facts.tile != Some(SHEEP_START_STAND) {
        return Err(format!(
            "Sheep Start requires the post-relog stand {SHEEP_START_STAND:?}, got {:?}",
            facts.tile
        ));
    }
    if !bank_seed.bank_loaded
        || !bank_seed.empty_bank_loaded
        || bank_seed.session_generation == 0
        || bank_seed.tile != DRAYNOR_BANK_APPROACH
        || bank_seed.shears != SHEARS_QTY
        || bank_seed.wool != WOOL_QTY
        || !bank_seed.inventory_empty
    {
        return Err(format!(
            "invalid pre-Start real-bank seed receipt: {bank_seed:?}"
        ));
    }
    Ok(())
}

fn validate_maxed_start(
    snapshot: &GameSnapshot,
    bank_seed: &BankSeedReceipt,
) -> Result<Value, String> {
    if !snapshot.ingame() || snapshot.scene_state() != 2 {
        return Err("Sheep Start fixture is not ingame with scene_state == 2".into());
    }
    let facts = SheepStartFacts {
        inventory: snapshot
            .inventory()
            .iter()
            .filter(|item| item.count > 0)
            .map(|item| (item.def.id, item.count))
            .collect(),
        equipment: snapshot
            .equipment()
            .iter()
            .filter(|item| item.count > 0)
            .map(|item| item.def.id)
            .collect(),
        bank_open: snapshot.bank_component_id() >= 0,
        sheep: sheep_quest_status(snapshot),
        tile: tile(snapshot),
    };
    validate_start_gate(&facts, bank_seed)?;

    let mut observed_stats = Vec::with_capacity(Skill::names.len());
    for (index, skill) in Skill::names
        .iter()
        .enumerate()
        .filter(|(index, _)| Skill::used[*index])
    {
        let stat = snapshot
            .stats()
            .iter()
            .find(|stat| stat.index == index as i32)
            .ok_or_else(|| format!("Sheep Start fixture did not publish the {skill} skill"))?;
        if stat.base != 99 || stat.effective != 99 {
            return Err(format!(
                "Sheep Start fixture {skill} expected base/effective 99, observed {}/{}",
                stat.base, stat.effective
            ));
        }
        observed_stats.push(json!({
            "skill": skill,
            "base": stat.base,
            "effective": stat.effective
        }));
    }
    Ok(json!({
        "fixture": "unchanged bundled sheep.json",
        "all_skills_99": true,
        "skills": observed_stats,
        "inventory": inventory_receipt(snapshot),
        "equipment": facts.equipment,
        "bank_open": facts.bank_open,
        "sheep_colour": facts.sheep.map(QuestListStatus::as_str),
        "tile": facts.tile.map(tile_json),
        "bank_seed": bank_seed_receipt_json(bank_seed),
    }))
}

fn step(name: &'static str, kind: StepKind, arm: Proof, budget_ticks: u32) -> Step {
    Step {
        name,
        kind,
        wait: Wait { arm, budget_ticks },
    }
}

fn perform_command(name: &'static str, command: String, arm: Proof) -> Step {
    Step {
        name,
        kind: StepKind::Perform {
            send: Box::new(move |client, _| api::interact::cheat(client, &command).is_sent()),
        },
        wait: Wait {
            arm,
            budget_ticks: 200,
        },
    }
}

fn sheep_bank_seed_steps(
    shears_id: i32,
    wool_id: i32,
    bank_seed: Arc<Mutex<BankSeedState>>,
) -> Vec<Step> {
    let mut steps = Vec::new();
    let bank_approach = DRAYNOR_BANK_APPROACH;
    let bank_booth = DRAYNOR_BANK_BOOTH;
    let teleport = api::interact::tele_args(bank_approach.level, bank_approach.x, bank_approach.z);
    steps.push(step(
        "teleport to the Draynor bank seed approach",
        StepKind::Perform {
            send: Box::new(move |client, _| api::interact::cheat(client, &teleport).is_sent()),
        },
        Proof::Arrived {
            x: bank_approach.x,
            z: bank_approach.z,
            level: bank_approach.level,
        },
        200,
    ));

    let closed_baseline = Arc::clone(&bank_seed);
    steps.push(step(
        "observe the closed bank and empty inventory before seeding",
        StepKind::Await {
            evidence: "closed bank session and empty inventory before real-bank seed",
            ready: Box::new(move |snapshot| {
                if !snapshot.ingame()
                    || snapshot.scene_state() != 2
                    || snapshot.bank_component_id() >= 0
                    || snapshot.bank_loaded()
                    || !inventory_empty(snapshot)
                {
                    return false;
                }
                let Ok(mut state) = closed_baseline.lock() else {
                    return false;
                };
                state.closed_generation = Some(snapshot.bank_session_generation());
                true
            }),
        },
        Proof::SideTabAvailable { index: 3 },
        200,
    ));

    steps.push(step(
        "open the exact Draynor bank booth for fixture seeding",
        StepKind::Perform {
            send: Box::new(move |client, snapshot| {
                if snapshot.bank_component_id() >= 0 && snapshot.bank_loaded() {
                    return true;
                }
                matches!(
                    Interactions::new(snapshot, client)
                        .open_booth_at(bank_booth, DRAYNOR_BANK_BOOTH_ID),
                    SendResult::Sent { .. }
                )
            }),
        },
        Proof::SideTabAvailable { index: 3 },
        200,
    ));

    let opened_state = Arc::clone(&bank_seed);
    steps.push(step(
        "observe a fresh empty Draynor bank load",
        StepKind::Await {
            evidence: "fresh open+loaded Draynor bank with no Sheep seed stock",
            ready: Box::new(move |snapshot| {
                if snapshot.bank_component_id() < 0 || !snapshot.bank_loaded() {
                    return false;
                }
                let Ok(mut state) = opened_state.lock() else {
                    return false;
                };
                let Some(closed_generation) = state.closed_generation else {
                    return false;
                };
                let generation = snapshot.bank_session_generation();
                let Some(snapshot_generation) = snapshot.bank_snapshot_generation() else {
                    return false;
                };
                if generation <= closed_generation
                    || tile(snapshot) != Some(DRAYNOR_BANK_APPROACH)
                    || item_count(snapshot.bank(), shears_id) != 0
                    || item_count(snapshot.bank(), wool_id) != 0
                    || !inventory_empty(snapshot)
                {
                    return false;
                }
                state.empty_open_generation = Some(generation);
                state.empty_open_receipt = Some(json!({
                    "event": "fresh-empty-bank-loaded-before-seed",
                    "tile": tile_json(DRAYNOR_BANK_APPROACH),
                    "bank_session_generation": generation,
                    "bank_snapshot_generation": snapshot_generation,
                    "bank_loaded": true,
                    "shears": 0,
                    "wool": 0,
                    "inventory": inventory_receipt(snapshot),
                }));
                true
            }),
        },
        Proof::SideTabAvailable { index: 3 },
        200,
    ));

    steps.push(step(
        "seed temporary fixture shears and wool before Start",
        StepKind::Perform {
            send: Box::new(move |client, _| {
                api::interact::cheat(client, "give shears 1").is_sent()
                    && api::interact::cheat(client, "give wool 20").is_sent()
            }),
        },
        Proof::SideTabAvailable { index: 3 },
        200,
    ));

    steps.push(step(
        "observe the temporary Sheep bank-seed inventory",
        StepKind::Await {
            evidence: "exact temporary shears+wool inventory at the loaded bank",
            ready: Box::new(move |snapshot| {
                snapshot.bank_component_id() >= 0
                    && snapshot.bank_loaded()
                    && item_count(snapshot.inventory(), shears_id) == SHEARS_QTY
                    && item_count(snapshot.inventory(), wool_id) == WOOL_QTY
                    && snapshot
                        .inventory()
                        .iter()
                        .filter(|item| item.count > 0)
                        .map(|item| item.count)
                        .sum::<i32>()
                        == SHEARS_QTY + WOOL_QTY
                    && item_count(snapshot.bank(), shears_id) == 0
                    && item_count(snapshot.bank(), wool_id) == 0
            }),
        },
        Proof::SideTabAvailable { index: 3 },
        200,
    ));

    steps.push(deposit_seed_item_step(
        "deposit fixture shears into the real bank",
        shears_id,
        SHEARS_QTY,
    ));
    steps.push(deposit_seed_item_step(
        "deposit fixture wool into the real bank",
        wool_id,
        WOOL_QTY,
    ));

    let seeded_state = Arc::clone(&bank_seed);
    steps.push(step(
        "observe exact loaded Sheep bank stock before closing",
        StepKind::Await {
            evidence: "loaded Draynor bank contains exactly the Sheep fixture stock",
            ready: Box::new(move |snapshot| {
                if snapshot.bank_component_id() < 0 || !snapshot.bank_loaded() {
                    return false;
                }
                let Ok(mut state) = seeded_state.lock() else {
                    return false;
                };
                let Some(empty_generation) = state.empty_open_generation else {
                    return false;
                };
                let generation = snapshot.bank_session_generation();
                let Some(snapshot_generation) = snapshot.bank_snapshot_generation() else {
                    return false;
                };
                let exact = generation == empty_generation
                    && tile(snapshot) == Some(DRAYNOR_BANK_APPROACH)
                    && item_count(snapshot.bank(), shears_id) == SHEARS_QTY
                    && item_count(snapshot.bank(), wool_id) == WOOL_QTY
                    && inventory_empty(snapshot);
                if exact {
                    state.receipt = Some(BankSeedReceipt {
                        tile: DRAYNOR_BANK_APPROACH,
                        bank_loaded: true,
                        empty_bank_loaded: state.empty_open_receipt.is_some(),
                        session_generation: generation,
                        snapshot_generation: Some(snapshot_generation),
                        shears: item_count(snapshot.bank(), shears_id),
                        wool: item_count(snapshot.bank(), wool_id),
                        inventory_empty: true,
                    });
                }
                exact
            }),
        },
        Proof::SideTabAvailable { index: 3 },
        300,
    ));

    steps.push(step(
        "close the Sheep fixture bank before the final relog",
        StepKind::Perform {
            send: Box::new(|client, snapshot| {
                snapshot.bank_component_id() < 0
                    || matches!(
                        Interactions::new(snapshot, client).close_modal(),
                        SendResult::Sent { .. }
                    )
            }),
        },
        Proof::BankClosed,
        200,
    ));

    steps
}

fn deposit_seed_item_step(name: &'static str, id: i32, quantity: i32) -> Step {
    step(
        name,
        StepKind::Repeat {
            send: Box::new(move |client, snapshot| {
                if snapshot.bank_component_id() < 0 || !snapshot.bank_loaded() {
                    return true;
                }
                let held = item_count(snapshot.inventory(), id);
                let banked = item_count(snapshot.bank(), id);
                if held == 0 {
                    return banked == quantity;
                }
                if held != quantity || banked != 0 {
                    return false;
                }
                let Some(item) = snapshot
                    .bank_side()
                    .iter()
                    .find(|item| item.def.id == id && item.count > 0)
                else {
                    return true;
                };
                let Some(operation) = bank_deposit_all_operation(&item.actions) else {
                    return true;
                };
                matches!(
                    Interactions::new(snapshot, client)
                        .interact(OpTarget::Item(item), ActionSpec::Operation(operation)),
                    SendResult::Sent { .. }
                )
            }),
        },
        Proof::BankItemId {
            id,
            count: quantity,
        },
        250,
    )
}

fn build_case() -> Result<SheepCase, String> {
    let bundled = script::quester::compile::path_bytes("sheep")
        .ok_or("release index has no bundled sheep Path")?;
    if bundled != SHEEP_PATH_BYTES {
        return Err("compiled bundled sheep Path differs from script/paths/289/sheep.json".into());
    }
    let path: PathDocument = serde_json::from_slice(bundled)
        .map_err(|error| format!("decode bundled sheep.json: {error}"))?;
    let header = path
        .quest
        .as_ref()
        .ok_or("bundled Sheep Path has no quest header")?;
    if path.id.0.as_ref() != "sheep" || header.owns_inventory {
        return Err("bundled Sheep Path identity or owns_inventory changed".into());
    }

    let selected = selected_data()?;
    let shears_id = item_id(&selected, "shears")?;
    let wool_id = item_id(&selected, "wool")?;
    let balls_of_wool_id = item_id(&selected, "ball_of_wool")?;
    let bank_seed = Arc::new(Mutex::new(BankSeedState::default()));
    let mut before_relog = sheep_bank_seed_steps(shears_id, wool_id, Arc::clone(&bank_seed));
    for (index, skill) in Skill::names
        .iter()
        .enumerate()
        .filter(|(index, _)| Skill::used[*index])
    {
        let skill_id = i32::try_from(index).map_err(|_| "skill index overflow")?;
        before_relog.push(perform_command(
            "max one skill before Sheep Start",
            format!("setstat {skill} 99"),
            Proof::Stat {
                id: skill_id,
                min: 99,
            },
        ));
    }

    let identity = selected
        .quest_identity()
        .and_then(|table| table.rows.iter().find(|row| row.id == "sheep"))
        .ok_or("selected R289 identity has no Sheep row")?;
    if identity.varp != "sheep" {
        return Err("selected Sheep identity differs from the canonical fresh seed".into());
    }
    // Sheep's unchanged Path uses quest-tab colour at stage zero, without a
    // varp hint. Use the same explicit fresh-stage builder as the Cook cell.
    let mut scenario = scenario::quester_stage(
        "live-quester-sheep-bank-loop",
        "Sheep Shearer",
        "sheep",
        0,
        &[],
        SHEEP_START_STAND,
    );
    scenario.settings.deadline = DEADLINE;
    let relog = scenario
        .steps
        .iter()
        .rposition(|step| matches!(step.kind, StepKind::Relog))
        .ok_or("Sheep fixture has no final relog")?;
    before_relog.insert(
        0,
        perform_command(
            "clear Sheep fixture equipment before Start",
            "~clearinv worn".into(),
            Proof::SideTabAvailable { index: 3 },
        ),
    );
    scenario.steps.splice(relog..relog, before_relog);
    let start = scenario
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .ok_or("Sheep fixture has no native Start")?;
    scenario.steps.insert(
        start,
        perform_command(
            "stand at the authored sequence",
            api::interact::tele_args(
                SHEEP_START_STAND.level,
                SHEEP_START_STAND.x,
                SHEEP_START_STAND.z,
            ),
            Proof::Arrived {
                x: SHEEP_START_STAND.x,
                z: SHEEP_START_STAND.z,
                level: SHEEP_START_STAND.level,
            },
        ),
    );
    let mut cell = Cell {
        quest: "sheep",
        display: "Sheep Shearer",
        label: "quester-sheep-bank-loop".to_owned(),
        scenario,
        start_settings: serde_json::Map::from_iter([("quests".into(), json!(["sheep"]))]),
        mode: Mode::Clean,
        observe_start: None,
    };
    let gate_index = start + 1;
    let gate_seed = Arc::clone(&bank_seed);
    cell.scenario.steps.insert(gate_index, Step {
        name: "prove maxed empty no-loadout Sheep fixture before Start",
        kind: StepKind::Await {
            evidence: "all skills 99, empty inventory, closed bank, Sheep NotStarted, real bank seed and post-relog stand",
            ready: Box::new(move |snapshot| {
                let Ok(state) = gate_seed.lock() else {
                    return false;
                };
                state
                    .receipt
                    .as_ref()
                    .is_some_and(|receipt| validate_maxed_start(snapshot, receipt).is_ok())
            }),
        },
        wait: Wait {
            arm: Proof::SideTabAvailable { index: 3 },
            budget_ticks: 200,
        },
    });

    let observed_seed = Arc::clone(&bank_seed);
    cell.observe_start = Some(Box::new(move |snapshot| {
        let bank_seed = observed_seed
            .lock()
            .map_err(|_| "Sheep bank seed witness poisoned")?
            .receipt
            .clone()
            .ok_or("no observed pre-Start bank seed receipt")?;
        let start_receipt = validate_maxed_start(snapshot, &bank_seed)?;
        let setup = observed_seed
            .lock()
            .map_err(|_| "Sheep bank seed witness poisoned")?;
        Ok(json!({
            "path_identity": {
                "id": "sheep",
                "source": "crates/script/paths/289/sheep.json",
                "owns_inventory": false,
                "modified": false,
            },
            "pre_start_bank_setup": setup.empty_open_receipt,
            "pre_start_bank_seed": bank_seed_receipt_json(&bank_seed),
            "start_fixture": start_receipt,
        }))
    }));

    Ok(SheepCase {
        cell,
        bank_seed,
        shears_id,
        wool_id,
        balls_of_wool_id,
    })
}

fn start_sheep(selected: Arc<SelectedGameData>, quests: Arc<QuestCatalog>) -> StartFamily {
    Box::new(move |handle, account, banks| {
        let bytes = script::quester::compile::path_bytes("sheep")
            .ok_or("release index has no bundled sheep Path")?;
        if bytes != SHEEP_PATH_BYTES {
            return Err("refusing to compile modified Sheep Path bytes".into());
        }
        let bytes = bytes.to_vec();
        let path = api::selected::FamilyPreparation::run({
            let selected = Arc::clone(&selected);
            let quests = Arc::clone(&quests);
            move |cap| compile_path(&bytes, &selected, &quests, cap)
        })
        .map_err(|error| format!("prepare bundled Sheep Path: {error}"))?
        .join()
        .map_err(|_| "bundled Sheep Path preparation panicked".to_owned())?
        .map_err(|error| format!("compile bundled Sheep Path: {error:?}"))?;
        let quester = Quester::new(
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
            .start_test_script(account, Box::new(quester), Some(Arc::clone(&selected)))
            .map(|_| ())
            .map_err(|error| format!("start bundled Sheep Quester: {error:?}"))
    })
}

#[derive(Debug, Default, Clone)]
struct SheepFrame {
    stage: Option<String>,
    step: Option<String>,
    phase: Option<NativePhase>,
    colour: Option<String>,
    action_state: Option<String>,
    provision: Option<String>,
    provision_item: Option<String>,
    block_reason: Option<String>,
    at_bank: bool,
    bank_open: bool,
    bank_loaded: bool,
    bank_generation: Option<u64>,
    bank_snapshot_generation: Option<u64>,
    bank_shears: i32,
    bank_wool: i32,
    inventory_shears: i32,
    inventory_wool: i32,
    inventory_balls: i32,
    green: bool,
    status: Value,
}

#[derive(Debug, Default)]
struct SheepGameplayProof {
    bank_loaded: Option<Value>,
    bank_generation: Option<u64>,
    provisioning_withdrawal: Option<Value>,
    spin_start: Option<Value>,
    spin: Option<Value>,
    hand_in: Option<Value>,
    green: Option<Value>,
    native_complete: Option<Value>,
    durable_completion: Option<Value>,
    max_balls: i32,
}

impl SheepGameplayProof {
    fn observe(
        &mut self,
        frame: &SheepFrame,
        lifecycle: Option<&script::ScriptLifecycleReceipt>,
    ) -> Result<(), String> {
        if frame.phase == Some(NativePhase::Blocked)
            || frame.action_state.as_deref() == Some("blocked")
            || frame.provision.as_deref() == Some("Blocked")
            || frame
                .provision_item
                .as_deref()
                .is_some_and(|item| item.starts_with("inventory capacity needs "))
            || frame
                .block_reason
                .as_deref()
                .is_some_and(|reason| !reason.is_empty())
        {
            return Err(format!("Quester parked/blocked during Sheep: {frame:?}"));
        }
        if frame.provision.as_deref() == Some("Spillover") {
            return Err(format!(
                "Quester attempted a capacity spillover/bank retreat during Sheep: {frame:?}"
            ));
        }
        if self.bank_loaded.is_some() {
            if frame.bank_open && frame.bank_generation != self.bank_generation {
                return Err(format!(
                    "Quester opened an additional bank generation/possible retreat: {frame:?}"
                ));
            }
            if self.provisioning_withdrawal.is_some()
                && frame.provision.as_deref() == Some("Scanning")
            {
                return Err(format!(
                    "Quester started another bank scan after provisioning withdrawal: {frame:?}"
                ));
            }
        }

        let sheep_stage = frame.stage.as_deref() == Some("sheep:1");
        let step_is = |id: &str| frame.step.as_deref() == Some(id);
        let provision_scanning =
            matches!(frame.provision.as_deref(), Some("Scanning" | "Withdrawing"));
        let authored_bank_scan =
            sheep_stage && (step_is("scan-bank") || step_is("withdraw-shears"));
        if sheep_stage && (step_is("spin") || step_is("hand-in")) && frame.bank_open {
            return Err(format!(
                "Sheep left the bank open during spinning or hand-in: {frame:?}"
            ));
        }
        if self.bank_loaded.is_none()
            && (provision_scanning || authored_bank_scan)
            && frame.at_bank
            && frame.bank_open
            && frame.bank_loaded
            && frame
                .bank_generation
                .is_some_and(|generation| generation > 0)
            && frame.bank_snapshot_generation.is_some()
            && frame.bank_shears == SHEARS_QTY
            && frame.bank_wool == WOOL_QTY
        {
            self.bank_generation = frame.bank_generation;
            self.bank_loaded = Some(json!({
                "event": "Quester loaded the real bank before withdrawal",
                "stage": frame.stage,
                "step": frame.step,
                "provision": frame.provision,
                "bank_loaded": frame.bank_loaded,
                "bank_session_generation": frame.bank_generation,
                "bank_snapshot_generation": frame.bank_snapshot_generation,
                "bank": {"shears": frame.bank_shears, "wool": frame.bank_wool},
                "status": frame.status,
            }));
        }
        if self.bank_loaded.is_some()
            && self.provisioning_withdrawal.is_none()
            && frame.provision.as_deref() == Some("Withdrawing")
            && frame.at_bank
            && frame.bank_open
            && frame.bank_loaded
            && frame.bank_generation == self.bank_generation
            && frame.inventory_shears == SHEARS_QTY
            && frame.inventory_wool == WOOL_QTY
            && frame.bank_shears == 0
            && frame.bank_wool == 0
        {
            self.provisioning_withdrawal = Some(json!({
                "event": "Quester provisioner withdrew banked shears and wool",
                "stage": frame.stage,
                "step": frame.step,
                "provision": frame.provision,
                "inventory": {"shears": frame.inventory_shears, "wool": frame.inventory_wool},
                "bank": {"shears": frame.bank_shears, "wool": frame.bank_wool},
                "bank_session_generation": frame.bank_generation,
                "status": frame.status,
            }));
        }
        if sheep_stage && step_is("spin") && self.provisioning_withdrawal.is_some() {
            if self.spin_start.is_none()
                && frame.inventory_wool == WOOL_QTY
                && frame.inventory_balls == 0
            {
                self.spin_start = Some(json!({
                    "event": "spin step began with the 20 banked wool and no finished balls",
                    "stage": frame.stage,
                    "step": frame.step,
                    "inventory": {
                        "shears": frame.inventory_shears,
                        "wool": frame.inventory_wool,
                        "balls_of_wool": frame.inventory_balls
                    },
                    "status": frame.status,
                }));
            }
            if self.spin_start.is_some() {
                self.max_balls = self.max_balls.max(frame.inventory_balls);
                if frame.inventory_balls > 0 && self.spin.is_none() {
                    self.spin = Some(json!({
                        "event": "observed spun balls of wool from the authored spinning-wheel step",
                        "stage": frame.stage,
                        "step": frame.step,
                        "inventory": {"wool": frame.inventory_wool, "balls_of_wool": frame.inventory_balls},
                        "status": frame.status,
                    }));
                }
            }
        }
        if self.spin_start.is_some() && sheep_stage {
            self.max_balls = self.max_balls.max(frame.inventory_balls);
        }
        if sheep_stage
            && step_is("hand-in")
            && frame.inventory_balls >= BALLS_OF_WOOL_QTY
            && self.hand_in.is_none()
        {
            self.hand_in = Some(json!({
                "event": "observed actual Farmer Fred hand-in step",
                "stage": frame.stage,
                "step": frame.step,
                "inventory": {"balls_of_wool": frame.inventory_balls},
                "status": frame.status,
            }));
        }
        if frame.green {
            self.green.get_or_insert_with(
                || json!({"event": "Sheep Shearer quest tab is green", "status": frame.status}),
            );
        }
        if frame.phase == Some(NativePhase::Complete)
            && frame.stage.as_deref() == Some("sheep:2")
            && frame.colour.as_deref() == Some("complete")
        {
            self.native_complete.get_or_insert_with(|| {
                json!({
                    "event": "Quester published completed Sheep progress",
                    "phase": "Complete",
                    "stage": frame.stage,
                    "colour": frame.colour,
                    "status": frame.status,
                })
            });
        }
        if let Some(lifecycle) = lifecycle {
            match lifecycle.state {
                script::ScriptTerminalState::Completed => {
                    self.durable_completion.get_or_insert_with(|| {
                        json!({
                            "runtime_generation": lifecycle.runtime_generation,
                            "state": lifecycle.state,
                            "tick": lifecycle.tick,
                            "reason": lifecycle.reason,
                        })
                    });
                }
                script::ScriptTerminalState::Failed
                | script::ScriptTerminalState::Cancelled
                | script::ScriptTerminalState::Stopped => {
                    return Err(format!(
                        "Sheep Quester ended {:?}: {lifecycle:?}",
                        lifecycle.state
                    ));
                }
            }
        }
        Ok(())
    }

    fn qualifies(&self) -> bool {
        self.bank_loaded.is_some()
            && self.provisioning_withdrawal.is_some()
            && self.spin_start.is_some()
            && self.spin.is_some()
            && self.max_balls >= BALLS_OF_WOOL_QTY
            && self.hand_in.is_some()
            && self.green.is_some()
            && self.durable_completion.is_some()
    }

    fn receipt(&self, bank_seed: &BankSeedReceipt) -> Value {
        json!({
            "pre_start_bank_seed": bank_seed_receipt_json(bank_seed),
            "bank_loading": self.bank_loaded,
            "provisioning_withdrawal": self.provisioning_withdrawal,
            "spin_start": self.spin_start,
            "spin": self.spin,
            "max_observed_balls_of_wool": self.max_balls,
            "hand_in": self.hand_in,
            "green_quest_tab": self.green,
            "native_completion": self.native_complete,
            "durable_lifecycle_completion": self.durable_completion,
            "no_parked_capacity_state": true,
            "no_additional_bank_retreat": true,
            "bank_session_generation": self.bank_generation,
        })
    }
}

fn sheep_frame(
    snapshot: &GameSnapshot,
    status: Option<&ScriptStatus>,
    shears_id: i32,
    wool_id: i32,
    balls_of_wool_id: i32,
) -> SheepFrame {
    let here = tile(snapshot);
    SheepFrame {
        stage: status.and_then(|status| quester_live::status_stage(status, "sheep")),
        step: status.and_then(status_step).map(str::to_owned),
        phase: status.map(|status| status.phase),
        colour: status
            .and_then(|status| status_text(status, "colour"))
            .map(str::to_owned),
        action_state: status
            .and_then(|status| status_text(status, "action_state"))
            .map(str::to_owned),
        provision: status
            .and_then(|status| status_text(status, "provision"))
            .map(str::to_owned),
        provision_item: status
            .and_then(|status| status_text(status, "provision_item"))
            .map(str::to_owned),
        block_reason: status
            .and_then(|status| status_text(status, "block_reason"))
            .map(str::to_owned),
        at_bank: here.is_some_and(|tile| near(tile, DRAYNOR_BANK_APPROACH, 8)),
        bank_open: snapshot.bank_component_id() >= 0,
        bank_loaded: snapshot.bank_loaded(),
        bank_generation: (snapshot.bank_component_id() >= 0)
            .then_some(snapshot.bank_session_generation()),
        bank_snapshot_generation: snapshot.bank_snapshot_generation(),
        bank_shears: item_count(snapshot.bank(), shears_id),
        bank_wool: item_count(snapshot.bank(), wool_id),
        inventory_shears: item_count(snapshot.inventory(), shears_id),
        inventory_wool: item_count(snapshot.inventory(), wool_id),
        inventory_balls: item_count(snapshot.inventory(), balls_of_wool_id),
        green: sheep_quest_status(snapshot) == Some(QuestListStatus::Complete),
        status: status.map(quester_live::status_json).unwrap_or(Value::Null),
    }
}

fn observe_sheep(
    bank_seed: Arc<Mutex<BankSeedState>>,
    shears_id: i32,
    wool_id: i32,
    balls_of_wool_id: i32,
) -> ObserveFamily {
    let mut proof = SheepGameplayProof::default();
    Box::new(move |snapshot, status, lifecycle| {
        if status.is_some_and(|status| status.card != script::CompiledId("Quester")) {
            return Err(format!(
                "Sheep observer saw foreign card status {:?}",
                status.map(|status| status.card)
            ));
        }
        proof.observe(
            &sheep_frame(snapshot, status, shears_id, wool_id, balls_of_wool_id),
            lifecycle,
        )?;
        if !proof.qualifies() {
            return Ok(None);
        }
        let bank_seed = bank_seed
            .lock()
            .map_err(|_| "Sheep bank seed witness poisoned")?
            .receipt
            .clone()
            .ok_or("Sheep gameplay completed without its pre-Start bank receipt")?;
        Ok(Some(proof.receipt(&bank_seed)))
    })
}

fn validate_live_receipt(receipt: &Value) -> Result<(), String> {
    if receipt.get("starts").and_then(Value::as_u64) != Some(1) {
        return Err(format!(
            "shared Quester runner did not prove exactly one Start: {receipt}"
        ));
    }
    if receipt.get("deaths").and_then(Value::as_u64) != Some(0) {
        return Err(format!(
            "shared Quester runner did not prove zero deaths: {receipt}"
        ));
    }
    let family = receipt
        .get("family_receipt")
        .filter(|value| value.is_object())
        .ok_or_else(|| {
            format!("shared lifecycle ended without the full Sheep gameplay witness: {receipt}")
        })?;
    for key in [
        "bank_loading",
        "provisioning_withdrawal",
        "spin_start",
        "spin",
        "hand_in",
        "green_quest_tab",
        "durable_lifecycle_completion",
    ] {
        if !family.get(key).is_some_and(Value::is_object) {
            return Err(format!("Sheep gameplay witness lacks {key}: {family}"));
        }
    }
    if family
        .get("pre_start_bank_seed")
        .and_then(|seed| seed.get("bank_loaded"))
        .and_then(Value::as_bool)
        != Some(true)
        || family
            .pointer("/pre_start_bank_seed/bank/shears")
            .and_then(Value::as_i64)
            != Some(i64::from(SHEARS_QTY))
        || family
            .pointer("/pre_start_bank_seed/bank/wool")
            .and_then(Value::as_i64)
            != Some(i64::from(WOOL_QTY))
        || family
            .pointer("/pre_start_bank_seed/inventory_empty")
            .and_then(Value::as_bool)
            != Some(true)
        || family
            .pointer("/pre_start_bank_seed/seeded_before_start")
            .and_then(Value::as_bool)
            != Some(true)
    {
        return Err(format!(
            "pre-Start bank seed receipt is not conclusive: {family}"
        ));
    }
    if family
        .pointer("/bank_loading/bank/shears")
        .and_then(Value::as_i64)
        != Some(i64::from(SHEARS_QTY))
        || family
            .pointer("/bank_loading/bank/wool")
            .and_then(Value::as_i64)
            != Some(i64::from(WOOL_QTY))
        || family
            .pointer("/provisioning_withdrawal/provision")
            .and_then(Value::as_str)
            != Some("Withdrawing")
        || family
            .pointer("/provisioning_withdrawal/inventory/shears")
            .and_then(Value::as_i64)
            != Some(i64::from(SHEARS_QTY))
        || family
            .pointer("/provisioning_withdrawal/inventory/wool")
            .and_then(Value::as_i64)
            != Some(i64::from(WOOL_QTY))
        || family
            .pointer("/provisioning_withdrawal/bank/shears")
            .and_then(Value::as_i64)
            != Some(0)
        || family
            .pointer("/provisioning_withdrawal/bank/wool")
            .and_then(Value::as_i64)
            != Some(0)
        || family
            .pointer("/bank_loading/bank_session_generation")
            .and_then(Value::as_u64)
            != family
                .pointer("/provisioning_withdrawal/bank_session_generation")
                .and_then(Value::as_u64)
        || !family
            .get("bank_session_generation")
            .and_then(Value::as_u64)
            .is_some_and(|generation| generation > 0)
        || family
            .get("bank_session_generation")
            .and_then(Value::as_u64)
            != family
                .pointer("/bank_loading/bank_session_generation")
                .and_then(Value::as_u64)
    {
        return Err(format!(
            "real Sheep bank loading/withdrawal proof failed: {family}"
        ));
    }
    if family.pointer("/spin_start/step").and_then(Value::as_str) != Some("spin")
        || family
            .pointer("/spin_start/inventory/wool")
            .and_then(Value::as_i64)
            != Some(i64::from(WOOL_QTY))
        || family
            .pointer("/spin_start/inventory/balls_of_wool")
            .and_then(Value::as_i64)
            != Some(0)
        || family.pointer("/spin/step").and_then(Value::as_str) != Some("spin")
        || family
            .get("max_observed_balls_of_wool")
            .and_then(Value::as_i64)
            != Some(i64::from(BALLS_OF_WOOL_QTY))
        || family.pointer("/hand_in/step").and_then(Value::as_str) != Some("hand-in")
        || family
            .pointer("/hand_in/inventory/balls_of_wool")
            .and_then(Value::as_i64)
            != Some(i64::from(BALLS_OF_WOOL_QTY))
        || family
            .get("no_parked_capacity_state")
            .and_then(Value::as_bool)
            != Some(true)
        || family
            .get("no_additional_bank_retreat")
            .and_then(Value::as_bool)
            != Some(true)
    {
        return Err(format!(
            "Sheep gameplay witness failed a terminal gate: {family}"
        ));
    }
    if family
        .pointer("/durable_lifecycle_completion/state")
        .and_then(Value::as_str)
        != Some("completed")
    {
        return Err(format!(
            "Sheep lifecycle witness is not Completed: {family}"
        ));
    }
    let lifecycle = receipt
        .get("lifecycle")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("shared runner omitted durable lifecycle receipt: {receipt}"))?;
    if !lifecycle.contains("Completed") {
        return Err(format!(
            "native lifecycle did not durably complete: {lifecycle}"
        ));
    }
    Ok(())
}

#[test]
#[ignore = "requires Engine A and the live environment documented at the top of this file"]
fn live_quester_sheep_bank_loop() {
    let case =
        build_case().unwrap_or_else(|error| panic!("build Sheep bank-loop fixture: {error}"));
    let selected = selected_data().expect("selected R289 game data");
    let quests = Arc::new(
        QuestCatalog::from_identity(selected.quest_identity())
            .expect("selected R289 quest catalog"),
    );
    let start = start_sheep(Arc::clone(&selected), quests);
    let observe = observe_sheep(
        Arc::clone(&case.bank_seed),
        case.shears_id,
        case.wool_id,
        case.balls_of_wool_id,
    );
    let receipt = quester_live::run_family(case.cell, start, observe)
        .unwrap_or_else(|error| panic!("Sheep bank-loop live proof failed: {error}"));
    validate_live_receipt(&receipt)
        .unwrap_or_else(|error| panic!("Sheep bank-loop terminal qualification failed: {error}"));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seeded_bank() -> BankSeedReceipt {
        BankSeedReceipt {
            tile: DRAYNOR_BANK_APPROACH,
            bank_loaded: true,
            empty_bank_loaded: true,
            session_generation: 1,
            snapshot_generation: Some(1),
            shears: SHEARS_QTY,
            wool: WOOL_QTY,
            inventory_empty: true,
        }
    }

    fn valid_start_facts() -> SheepStartFacts {
        SheepStartFacts {
            inventory: Vec::new(),
            equipment: Vec::new(),
            bank_open: false,
            sheep: Some(QuestListStatus::NotStarted),
            tile: Some(SHEEP_START_STAND),
        }
    }

    fn frame(stage: &str, step: &str) -> SheepFrame {
        SheepFrame {
            stage: Some(stage.to_owned()),
            step: Some(step.to_owned()),
            phase: Some(NativePhase::Working),
            colour: Some("in_progress".to_owned()),
            ..SheepFrame::default()
        }
    }

    fn completed_lifecycle() -> script::ScriptLifecycleReceipt {
        script::ScriptLifecycleReceipt {
            runtime_generation: 7,
            state: script::ScriptTerminalState::Completed,
            tick: 42,
            reason: "test terminal".into(),
        }
    }

    #[test]
    fn fixture_uses_unchanged_bundled_sheep_path_and_prestart_bank_seed() {
        let bytes = script::quester::compile::path_bytes("sheep").expect("bundled Sheep Path");
        assert_eq!(bytes, SHEEP_PATH_BYTES);
        let path: PathDocument = serde_json::from_slice(bytes).expect("decode bundled Sheep Path");
        assert_eq!(path.id.0.as_ref(), "sheep");
        assert!(
            !path
                .quest
                .as_ref()
                .expect("Sheep quest header")
                .owns_inventory
        );
        assert_eq!(
            path.quest.as_ref().expect("Sheep quest header").items.len(),
            3
        );

        let case = build_case().expect("build live fixture without running it");
        let cell = case.cell;
        assert_eq!(cell.quest, "sheep");
        assert_eq!(cell.display, "Sheep Shearer");
        assert_eq!(cell.start_settings.len(), 1);
        assert_eq!(cell.start_settings.get("quests"), Some(&json!(["sheep"])));
        assert_eq!(cell.scenario.settings.start_script, Some("Quester"));
        assert_eq!(cell.scenario.settings.deadline, DEADLINE);
        assert_eq!(
            cell.scenario
                .steps
                .iter()
                .filter(|step| matches!(step.kind, StepKind::StartScript))
                .count(),
            1
        );

        let start = cell
            .scenario
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .expect("one native Quester Start step");
        assert_eq!(
            cell.scenario.steps[start + 1].name,
            "watch the quest tab turn complete"
        );
        assert_eq!(cell.scenario.steps.len(), start + 2);
        let relog = cell
            .scenario
            .steps
            .iter()
            .rposition(|step| matches!(step.kind, StepKind::Relog))
            .expect("fixture's final relog");
        let bank_steps = [
            "teleport to the Draynor bank seed approach",
            "observe the closed bank and empty inventory before seeding",
            "open the exact Draynor bank booth for fixture seeding",
            "observe a fresh empty Draynor bank load",
            "seed temporary fixture shears and wool before Start",
            "observe the temporary Sheep bank-seed inventory",
            "deposit fixture shears into the real bank",
            "deposit fixture wool into the real bank",
            "observe exact loaded Sheep bank stock before closing",
            "close the Sheep fixture bank before the final relog",
        ];
        for name in bank_steps {
            let index = cell
                .scenario
                .steps
                .iter()
                .position(|step| step.name == name)
                .unwrap_or_else(|| panic!("missing fixture step {name:?}"));
            assert!(index < relog, "{name:?} must be pre-relog and pre-Start");
        }
        let maxed_stats = cell.scenario.steps[..relog]
            .iter()
            .filter(|step| step.name == "max one skill before Sheep Start")
            .count();
        assert_eq!(
            maxed_stats,
            Skill::used.iter().filter(|used| **used).count(),
            "ordinary maxed stats must be staged before the final relog"
        );
        let gate = cell
            .scenario
            .steps
            .iter()
            .position(|step| step.name == "prove maxed empty no-loadout Sheep fixture before Start")
            .expect("strong fixture gate");
        let stand = cell
            .scenario
            .steps
            .iter()
            .position(|step| step.name == "stand at the authored sequence")
            .expect("fixture stand step");
        assert!(relog < stand && stand < gate && gate < start);
    }

    #[test]
    fn start_gate_requires_empty_pack_closed_bank_not_started_sheep_no_loadout_and_seed() {
        let bank = seeded_bank();
        let mut facts = valid_start_facts();
        assert!(validate_start_gate(&facts, &bank).is_ok());

        facts.inventory.push((123, 1));
        assert!(validate_start_gate(&facts, &bank)
            .unwrap_err()
            .contains("inventory is not empty"));
        facts.inventory.clear();

        facts.equipment.push(456);
        assert!(validate_start_gate(&facts, &bank)
            .unwrap_err()
            .contains("no combat/equipment loadout"));
        facts.equipment.clear();

        facts.bank_open = true;
        assert!(validate_start_gate(&facts, &bank)
            .unwrap_err()
            .contains("bank modal to be closed"));
        facts.bank_open = false;

        facts.sheep = Some(QuestListStatus::InProgress);
        assert!(validate_start_gate(&facts, &bank)
            .unwrap_err()
            .contains("NotStarted"));
        facts.sheep = Some(QuestListStatus::NotStarted);

        facts.tile = None;
        assert!(validate_start_gate(&facts, &bank)
            .unwrap_err()
            .contains("post-relog stand"));
        facts.tile = Some(SHEEP_START_STAND);

        let mut bad_bank = bank.clone();
        bad_bank.wool = 19;
        assert!(validate_start_gate(&facts, &bad_bank)
            .unwrap_err()
            .contains("pre-Start real-bank seed"));
    }

    #[test]
    fn gameplay_witness_requires_bank_loading_provision_withdrawal_spin_handin_green_and_completion(
    ) {
        let mut proof = SheepGameplayProof::default();
        let mut bank = frame("sheep:0", "start");
        bank.provision = Some("Scanning".to_owned());
        bank.at_bank = true;
        bank.bank_open = true;
        bank.bank_loaded = true;
        bank.bank_generation = Some(2);
        bank.bank_snapshot_generation = Some(3);
        bank.bank_shears = SHEARS_QTY;
        bank.bank_wool = WOOL_QTY;
        proof.observe(&bank, None).unwrap();
        assert!(proof.bank_loaded.is_some());
        assert!(!proof.qualifies());

        let mut withdrawn = frame("sheep:0", "start");
        withdrawn.provision = Some("Withdrawing".to_owned());
        withdrawn.at_bank = true;
        withdrawn.bank_open = true;
        withdrawn.bank_loaded = true;
        withdrawn.bank_generation = Some(2);
        withdrawn.bank_shears = 0;
        withdrawn.bank_wool = 0;
        withdrawn.inventory_shears = SHEARS_QTY;
        withdrawn.inventory_wool = WOOL_QTY;
        proof.observe(&withdrawn, None).unwrap();
        assert!(proof.provisioning_withdrawal.is_some());

        let mut spin_start = frame("sheep:1", "spin");
        spin_start.inventory_shears = SHEARS_QTY;
        spin_start.inventory_wool = WOOL_QTY;
        proof.observe(&spin_start, None).unwrap();
        assert!(proof.spin_start.is_some());

        let mut spun = frame("sheep:1", "spin");
        spun.inventory_shears = SHEARS_QTY;
        spun.inventory_balls = BALLS_OF_WOOL_QTY;
        proof.observe(&spun, None).unwrap();
        assert!(proof.spin.is_some());
        assert_eq!(proof.max_balls, BALLS_OF_WOOL_QTY);

        let mut handed_in = frame("sheep:1", "hand-in");
        handed_in.inventory_balls = BALLS_OF_WOOL_QTY;
        proof.observe(&handed_in, None).unwrap();
        assert!(proof.hand_in.is_some());

        let mut complete = frame("sheep:2", "hand-in");
        complete.phase = Some(NativePhase::Complete);
        complete.colour = Some("complete".to_owned());
        complete.green = true;
        proof
            .observe(&complete, Some(&completed_lifecycle()))
            .unwrap();
        assert!(proof.qualifies());
    }

    #[test]
    fn gameplay_witness_fails_closed_on_capacity_park_or_retreat() {
        let mut proof = SheepGameplayProof::default();
        let mut parked = frame("sheep:1", "withdraw-shears");
        parked.provision = Some("Blocked".to_owned());
        parked.provision_item = Some("inventory capacity needs 1 additional safe slots".to_owned());
        assert!(proof
            .observe(&parked, None)
            .unwrap_err()
            .contains("parked/blocked"));

        let mut retreat = frame("sheep:1", "hand-in");
        retreat.provision = Some("Spillover".to_owned());
        assert!(proof
            .observe(&retreat, None)
            .unwrap_err()
            .contains("capacity spillover/bank retreat"));

        let mut proof = SheepGameplayProof {
            bank_loaded: Some(json!({})),
            bank_generation: Some(2),
            ..Default::default()
        };
        let mut reopened = frame("sheep:1", "hand-in");
        reopened.bank_open = true;
        reopened.bank_generation = Some(3);
        assert!(proof
            .observe(&reopened, None)
            .unwrap_err()
            .contains("additional bank generation"));
    }

    #[test]
    fn green_without_durable_completed_lifecycle_does_not_qualify() {
        let proof = SheepGameplayProof {
            bank_loaded: Some(json!({})),
            provisioning_withdrawal: Some(json!({})),
            spin_start: Some(json!({})),
            spin: Some(json!({})),
            max_balls: BALLS_OF_WOOL_QTY,
            hand_in: Some(json!({})),
            green: Some(json!({})),
            ..Default::default()
        };
        assert!(!proof.qualifies());
    }
}
