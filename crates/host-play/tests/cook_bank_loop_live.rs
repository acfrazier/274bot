//! Ignored live regression cells for the shipped Cook's Assistant Path.
//!
//! The fixed primary cell starts a fresh maxed account with no combat loadout
//! and the three ingredients in its bank. It must fetch them in exactly one
//! preparation bank-open generation and complete without deaths. The held
//! cell requires zero preparation visits; the neither cell requires one empty
//! bank scan before the real bundled acquisition recipes run. Every cell uses
//! the unchanged bundled `cook.json` with native Quester and never spawns a
//! world target. Evidence, including captured host session logs, is saved under
//! `LIVE_EVIDENCE_DIR`.
//! Bank intent is witnessed while closed and bound to the same Cook run's next
//! session generation. Snapshot wakes can settle the scan before this observer
//! samples the modal, so current status at opening is recorded separately.
//!
//! Run from the repository root against Engine A (the parent prepares the
//! owned writable APFS cache clone first):
//!
//! Use this worktree's current `target/debug/nav/289/274bot.navpack` and
//! matching `274bot.navflags`; borrowed older-format packs fail closed.
//!
//! ```text
//! HOME=/Volumes/dev-scratch/274bot-evidence/COOK-BANK-LOOP/home CARGO_HOME=/Users/acfrazier/.cargo RUSTUP_HOME=/Users/acfrazier/.rustup \
//! ENGINE_DIR=/Users/acfrazier/experiments/lostcity-289/engine BOT_NAV_SNAPSHOT_ROOT=/Volumes/dev-scratch/274bot-evidence/WALK-GUARD/cache-snapshots \
//! LIVE=1 BOT_CPU=1 BOT_NAV_BUILD=skip BOT_LIVE_NAME_PREFIX=ck WORLD_GAME_PORT=44594 WORLD_HTTP_PORT=1080 \
//! WORLD_ENGINE_DIR=/Users/acfrazier/experiments/lostcity-289/engine WORLD_NAV_PACK="$PWD/target/debug/nav/289/274bot.navpack" \
//! NAV_FLAGS="$PWD/target/debug/nav/289/274bot.navflags" RS2B0T=/Users/acfrazier/experiments/rs2b0t \
//! BOT_CACHE_DIR=<owned-writable-APFS-clone-prepared-for-this-run> LIVE_EVIDENCE_DIR=/Volumes/dev-scratch/274bot-evidence/COOK-BANK-LOOP \
//! cargo test -p host-play --test cook_bank_loop_live --features "live-harness test-support" -- --ignored --nocapture --test-threads=1
//! ```

#![cfg(all(feature = "live-harness", feature = "test-support"))]

#[path = "common/quester_live.rs"]
mod quester_live;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use api::game_data::SelectedGameData;
use api::hostlog::{Record, Sink};
use api::quest_facts::QuestCatalog;
use api::selected::{ClientRevision, RunKey};
use api::snapshot::{GameSnapshot, ItemView, WorldTile};
use client::client::skill::Skill;
use quester_live::{Cell, Mode, ObserveFamily, StartFamily};
use scenario::{Proof, Step, StepKind, Wait};
use script::native::{ScriptStatus, StatusValue};
use script::quester::compile::compile_path;
use script::quester::path::PathDocument;
use script::quester::runner::Quester;
use serde::Serialize;
use serde_json::{json, Value};

const COOK_PATH_JSON: &str = include_str!("../../script/paths/289/cook.json");
const COOK_START: WorldTile = WorldTile {
    x: 3209,
    z: 3215,
    level: 0,
};
const INGREDIENTS: [(&str, i32); 3] = [("egg", 1), ("bucket_milk", 1), ("pot_flour", 1)];
const LIVE_DEADLINE: Duration = Duration::from_secs(1800);
const PREPARATION_STEPS: [&str; 3] = ["egg", "milk", "flour"];
const ROOT_BEGIN_LINES: [(&str, &str, &str); 2] =
    [("cook:0", "start", "talk"), ("cook:1", "hand-in", "talk")];
const ROOT_SETTLED_STEPS: [(&str, &str); 2] = [("start", "talk"), ("hand-in", "talk")];
const ACQUISITION_CHILD_LINES: [(&str, &str); 3] = [
    ("acquire:egg", "take-egg"),
    ("acquire:milk", "take-bucket"),
    ("acquire:flour", "take-pot"),
];

static LIVE_CELL_LOCK: Mutex<()> = Mutex::new(());
static SESSION_LOGS: LazyLock<Mutex<Vec<SessionLogLine>>> =
    LazyLock::new(|| Mutex::new(Vec::new()));
static SESSION_LOG_SINK_INSTALLED: LazyLock<bool> =
    LazyLock::new(|| api::hostlog::install_sink(&SESSION_LOG_SINK));

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FixtureCase {
    Banked,
    Held,
    Neither,
}

impl FixtureCase {
    fn name(self) -> &'static str {
        match self {
            Self::Banked => "banked",
            Self::Held => "held",
            Self::Neither => "neither",
        }
    }

    fn held_items(self) -> &'static [(&'static str, i32)] {
        match self {
            Self::Held => &INGREDIENTS,
            Self::Banked | Self::Neither => &[],
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct SessionLogLine {
    slot: Option<String>,
    tick: Option<u32>,
    source: String,
    level: String,
    message: String,
}

struct SessionLogSink;
static SESSION_LOG_SINK: SessionLogSink = SessionLogSink;

impl Sink for SessionLogSink {
    fn record(&self, record: &Record<'_>) {
        let line = SessionLogLine {
            slot: record.slot.map(str::to_owned),
            tick: record.tick,
            source: record.source.label().to_owned(),
            level: record.level.label().to_owned(),
            message: record.message.to_owned(),
        };
        let mut lines = SESSION_LOGS
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        lines.push(line);
    }
}

fn session_log_store() -> &'static Mutex<Vec<SessionLogLine>> {
    assert!(
        *SESSION_LOG_SINK_INSTALLED,
        "could not install live session-log sink; another sink owns this process"
    );
    &SESSION_LOGS
}

fn evidence_directory(case: FixtureCase) -> Result<PathBuf, String> {
    let root = std::env::var_os("LIVE_EVIDENCE_DIR")
        .map(PathBuf::from)
        .ok_or_else(|| "LIVE_EVIDENCE_DIR is required".to_owned())?;
    if !root.is_absolute() {
        return Err(format!(
            "LIVE_EVIDENCE_DIR must be absolute: {}",
            root.display()
        ));
    }
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("clock: {error}"))?
        .as_nanos();
    let path = root
        .join("cook-bank-loop")
        .join(format!("{}-{nonce}", case.name()));
    std::fs::create_dir_all(&path)
        .map_err(|error| format!("create evidence directory {}: {error}", path.display()))?;
    Ok(path)
}

fn item_id(selected: &SelectedGameData, alias: &str) -> Result<i32, String> {
    selected
        .item_by_alias(alias)
        .map(|item| item.id)
        .ok_or_else(|| format!("selected R289 data has no item alias {alias:?}"))
}

fn item_counts(items: &[ItemView], ids: &[i32; 3]) -> [i32; 3] {
    std::array::from_fn(|index| {
        items
            .iter()
            .filter(|item| item.def.id == ids[index] && item.count > 0)
            .map(|item| item.count)
            .sum()
    })
}

fn validate_maxed_start(
    snapshot: &GameSnapshot,
    case: FixtureCase,
    ids: [i32; 3],
) -> Result<Value, String> {
    if !snapshot.ingame() || snapshot.scene_state() != 2 {
        return Err("Start fixture is not ingame with scene_state == 2".into());
    }
    if snapshot.tile() != Some((COOK_START.x, COOK_START.z, COOK_START.level)) {
        return Err(format!(
            "Start fixture is not at the Cook stand: {:?}",
            snapshot.tile()
        ));
    }
    if snapshot.bank_component_id() >= 0 {
        return Err("Start fixture must have the bank closed".into());
    }

    if !snapshot.quest_statuses().iter().any(|row| {
        row.name.trim().eq_ignore_ascii_case("Cook's Assistant")
            && row.status() == api::snapshot::QuestListStatus::NotStarted
    }) {
        return Err("Start fixture did not publish Cook's Assistant as not started".into());
    }

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
            .ok_or_else(|| format!("Start fixture did not publish the {skill} skill"))?;
        if stat.base != 99 || stat.effective != 99 {
            return Err(format!(
                "Start fixture {skill} expected base/effective 99, observed {}/{}",
                stat.base, stat.effective
            ));
        }
        observed_stats.push(json!({
            "skill": skill,
            "base": stat.base,
            "effective": stat.effective
        }));
    }

    let observed_ingredients = item_counts(snapshot.inventory(), &ids);
    let expected_ingredients = match case {
        FixtureCase::Banked | FixtureCase::Neither => [0, 0, 0],
        FixtureCase::Held => [1, 1, 1],
    };
    if observed_ingredients != expected_ingredients {
        return Err(format!(
            "Start fixture {:?} inventory expected {expected_ingredients:?}, observed {observed_ingredients:?}",
            case
        ));
    }
    let extras: Vec<_> = snapshot
        .inventory()
        .iter()
        .filter(|item| item.count > 0 && !ids.contains(&item.def.id))
        .map(|item| (item.def.id, item.count))
        .collect();
    if !extras.is_empty() {
        return Err(format!(
            "Start fixture contains unplanned inventory: {extras:?}"
        ));
    }
    let equipment: Vec<_> = snapshot
        .equipment()
        .iter()
        .filter(|item| item.count > 0)
        .map(|item| item.def.id)
        .collect();
    if !equipment.is_empty() {
        return Err(format!(
            "Start fixture has a combat/equipment loadout: {equipment:?}"
        ));
    }

    Ok(json!({
        "fixture_case": case.name(),
        "all_skills_99": true,
        "skills": observed_stats,
        "ingredient_ids": ids,
        "inventory_ingredient_counts": observed_ingredients,
        "equipment_ids": equipment,
        "bank_open": false,
        "tile": [COOK_START.x, COOK_START.z, COOK_START.level],
    }))
}

fn perform_command(name: &'static str, command: String, proof: Proof) -> Step {
    Step {
        name,
        kind: StepKind::Perform {
            send: Box::new(move |client, _| api::interact::cheat(client, &command).is_sent()),
        },
        wait: Wait {
            arm: proof,
            budget_ticks: 100,
        },
    }
}

fn build_case(case: FixtureCase) -> Result<(Cell, StartFamily, [i32; 3]), String> {
    let selected = api::game_data::for_revision(ClientRevision::R289)
        .map_err(|error| format!("selected R289 game data: {error}"))?;
    let quests = Arc::new(
        QuestCatalog::from_identity(selected.quest_identity())
            .expect("selected R289 quest catalog"),
    );
    let document: PathDocument = serde_json::from_str(COOK_PATH_JSON)
        .map_err(|error| format!("decode bundled cook.json: {error}"))?;
    if document.id.0.as_ref() != "cook" {
        return Err(format!(
            "bundled Cook Path identity changed: {}",
            document.id.0
        ));
    }
    let header = document
        .quest
        .as_ref()
        .ok_or("bundled Cook Path has no quest header")?;
    if header.owns_inventory {
        return Err("bundled Cook Path unexpectedly owns the inventory".into());
    }
    let quest_identity = selected
        .quest_identity()
        .ok_or("selected R289 quest identity is unavailable")?;
    let identity = quest_identity
        .rows
        .iter()
        .find(|row| row.id == "cook")
        .ok_or("selected R289 quest identity has no Cook row")?;
    let ids = [
        item_id(&selected, "egg")?,
        item_id(&selected, "bucket_milk")?,
        item_id(&selected, "pot_flour")?,
    ];

    let name = match case {
        FixtureCase::Banked => "live-cook-bank-loop-banked",
        FixtureCase::Held => "live-cook-bank-loop-held",
        FixtureCase::Neither => "live-cook-bank-loop-neither",
    };
    if identity.varp != "cookquest" {
        return Err(
            "selected Cook identity does not match the canonical fresh-account seed".into(),
        );
    }
    // The unchanged Cook Path uses quest-tab colour for cook:0 and has no
    // stage-zero varp hint. Reuse its existing explicit fresh-start scenario
    // instead of fabricating a hint for the generic authored-stage builder.
    let mut scenario = scenario::quester_stage(
        name,
        "Cook's Assistant",
        "cookquest",
        0,
        case.held_items(),
        COOK_START,
    );
    scenario.settings.deadline = LIVE_DEADLINE;
    let mut seed_commands = vec![
        "setvar tutorial 1000".to_owned(),
        "getvar tutorial".to_owned(),
        "setvar cookquest 0".to_owned(),
        "~clearinv".to_owned(),
    ];
    seed_commands.extend(
        case.held_items()
            .iter()
            .map(|(alias, qty)| format!("give {alias} {qty}")),
    );
    seed_commands.push(api::interact::tele_args(
        COOK_START.level,
        COOK_START.x,
        COOK_START.z,
    ));

    let relog_index = scenario
        .steps
        .iter()
        .position(|step| step.name == "relog so the quest tab colour matches the seeded varp")
        .ok_or("Cook fixture has no post-seed relog")?;
    let mut pre_relog = Vec::new();
    let mut inserted_seed_commands = Vec::new();
    if case == FixtureCase::Banked {
        for (alias, _) in INGREDIENTS {
            let command = format!("givebank {alias} 1");
            inserted_seed_commands.push(command.clone());
            pre_relog.push(perform_command(
                "seed one Cook ingredient in the bank",
                command,
                Proof::SideTabAvailable { index: 3 },
            ));
        }
    }
    for (index, skill) in Skill::names
        .iter()
        .enumerate()
        .filter(|(index, _)| Skill::used[*index])
    {
        let skill_id = i32::try_from(index).map_err(|_| "skill index overflow")?;
        let command = format!("setstat {skill} 99");
        inserted_seed_commands.push(command.clone());
        pre_relog.push(perform_command(
            "max one skill before Quester Start",
            command,
            Proof::Stat {
                id: skill_id,
                min: 99,
            },
        ));
    }
    scenario.steps.splice(relog_index..relog_index, pre_relog);
    let final_relog = scenario
        .steps
        .iter()
        .rposition(|step| matches!(step.kind, StepKind::Relog))
        .ok_or("Cook fixture has no final relog")?;
    scenario.steps.insert(
        final_relog + 1,
        Step {
            name: "reseed tutorial after Cook fixture relog",
            kind: StepKind::Repeat {
                send: Box::new(|client, _| {
                    api::interact::cheat(client, scenario::tutorial::TUTORIAL_SETVAR);
                    api::interact::cheat(client, scenario::tutorial::TUTORIAL_GETVAR);
                    true
                }),
            },
            wait: Wait {
                arm: Proof::FreshTutorial,
                budget_ticks: 200,
            },
        },
    );

    let maxed_gate =
        Box::new(move |snapshot: &GameSnapshot| validate_maxed_start(snapshot, case, ids).is_ok());
    let start_index = scenario
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .ok_or("Cook fixture has no native Start boundary")?;
    let return_to_stand = api::interact::tele_args(COOK_START.level, COOK_START.x, COOK_START.z);
    scenario.steps.insert(
        start_index,
        perform_command(
            "return to the Cook stand after fixture relog",
            return_to_stand.clone(),
            Proof::Arrived {
                x: COOK_START.x,
                z: COOK_START.z,
                level: COOK_START.level,
            },
        ),
    );
    scenario.steps.insert(
        start_index + 1,
        Step {
            name: "prove maxed no-loadout Cook fixture before Start",
            kind: StepKind::Await {
                evidence:
                    "all skills exactly 99, expected ingredients, empty equipment and Cook stand",
                ready: maxed_gate,
            },
            wait: Wait {
                arm: Proof::SideTabAvailable { index: 3 },
                budget_ticks: 200,
            },
        },
    );

    seed_commands.extend(inserted_seed_commands);
    seed_commands.extend([
        scenario::tutorial::TUTORIAL_SETVAR.to_owned(),
        scenario::tutorial::TUTORIAL_GETVAR.to_owned(),
    ]);
    seed_commands.push(return_to_stand);

    let observe_ids = ids;
    let observe_commands = seed_commands;
    let observe_start = Box::new(move |snapshot: &GameSnapshot| {
        let mut receipt = validate_maxed_start(snapshot, case, observe_ids)?;
        receipt["path_identity"] = json!({
            "id": "cook",
            "source": "crates/script/paths/289/cook.json",
            "owns_inventory": false,
            "modified": false,
        });
        receipt["pre_start_seed_commands"] = json!(observe_commands);
        Ok(receipt)
    });

    let start = production_start(Arc::clone(&selected), quests);
    Ok((
        Cell {
            quest: "cook",
            display: "Cook's Assistant",
            label: name.to_owned(),
            scenario,
            start_settings: serde_json::Map::from_iter([("quests".into(), json!(["cook"]))]),
            mode: Mode::Clean,
            observe_start: Some(observe_start),
        },
        start,
        ids,
    ))
}

fn production_start(selected: Arc<SelectedGameData>, quests: Arc<QuestCatalog>) -> StartFamily {
    Box::new(move |handle, account, banks| {
        let path = api::selected::FamilyPreparation::run({
            let selected = Arc::clone(&selected);
            let quests = Arc::clone(&quests);
            move |cap| compile_path(COOK_PATH_JSON.as_bytes(), &selected, &quests, cap)
        })
        .map_err(|error| format!("prepare bundled Cook Path: {error}"))?
        .join()
        .map_err(|_| "bundled Cook Path compilation panicked".to_owned())?
        .map_err(|error| format!("compile bundled Cook Path: {error:?}"))?;
        if path.id.0.as_ref() != "cook" {
            return Err(format!("native Cook Start compiled Path {:?}", path.id.0));
        }
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
            .map_err(|error| format!("start native Quester on bundled Cook Path: {error:?}"))
    })
}

fn status_text<'a>(status: Option<&'a ScriptStatus>, key: &str) -> Option<&'a str> {
    status?
        .fields
        .iter()
        .find(|field| field.key == key)
        .and_then(|field| match &field.value {
            StatusValue::Text(value) => Some(value.as_ref()),
            _ => None,
        })
}

fn quest_complete(snapshot: &GameSnapshot) -> bool {
    snapshot.quest_statuses().iter().any(|row| {
        row.name.trim().eq_ignore_ascii_case("Cook's Assistant")
            && row.status() == api::snapshot::QuestListStatus::Complete
    })
}

fn snapshot_status(status: Option<&ScriptStatus>) -> Value {
    let Some(status) = status else {
        return Value::Null;
    };
    json!({
        "phase": format!("{:?}", status.phase),
        "stage": status_text(Some(status), "stage"),
        "step_id": status_text(Some(status), "step_id"),
        "child_recipe_id": status_text(Some(status), "child_recipe_id"),
        "child_step_id": status_text(Some(status), "child_step_id"),
        "action_state": status_text(Some(status), "action_state"),
        "colour": status_text(Some(status), "colour"),
    })
}

#[derive(Debug, Clone, Serialize)]
struct BankSession {
    generation: u64,
    stage_at_open: Option<String>,
    step_at_open: Option<String>,
    child_recipe_at_open: Option<String>,
    child_step_at_open: Option<String>,
    action_state_at_open: Option<String>,
    opening_provision: Value,
    bank_loaded_seen: bool,
    first_loaded_bank_counts: Option<[i32; 3]>,
    minimum_loaded_bank_counts: Option<[i32; 3]>,
    maximum_loaded_bank_counts: Option<[i32; 3]>,
    inventory_counts_at_open: [i32; 3],
}

impl BankSession {
    fn new(
        generation: u64,
        snapshot: &GameSnapshot,
        status: Option<&ScriptStatus>,
        preparation: &ScriptStatus,
        ids: &[i32; 3],
    ) -> Self {
        Self {
            generation,
            stage_at_open: status_text(status, "stage").map(str::to_owned),
            step_at_open: status_text(status, "step_id").map(str::to_owned),
            child_recipe_at_open: status_text(status, "child_recipe_id").map(str::to_owned),
            child_step_at_open: status_text(status, "child_step_id").map(str::to_owned),
            action_state_at_open: status_text(status, "action_state").map(str::to_owned),
            opening_provision: json!({
                "expected_generation": generation,
                "run": {
                    "slot": preparation.run.slot,
                    "run": preparation.run.run,
                    "session": preparation.run.session,
                },
                "status": snapshot_status(Some(preparation)),
            }),
            bank_loaded_seen: false,
            first_loaded_bank_counts: None,
            minimum_loaded_bank_counts: None,
            maximum_loaded_bank_counts: None,
            inventory_counts_at_open: item_counts(snapshot.inventory(), ids),
        }
    }

    fn observe(&mut self, snapshot: &GameSnapshot, ids: &[i32; 3]) {
        if !snapshot.bank_loaded() {
            return;
        }
        let counts = item_counts(snapshot.bank(), ids);
        self.bank_loaded_seen = true;
        self.first_loaded_bank_counts.get_or_insert(counts);
        self.minimum_loaded_bank_counts = Some(match self.minimum_loaded_bank_counts {
            Some(previous) => std::array::from_fn(|index| previous[index].min(counts[index])),
            None => counts,
        });
        self.maximum_loaded_bank_counts = Some(match self.maximum_loaded_bank_counts {
            Some(previous) => std::array::from_fn(|index| previous[index].max(counts[index])),
            None => counts,
        });
    }
}

struct CookWitness {
    case: FixtureCase,
    ids: [i32; 3],
    completed_seen: bool,
    completion_tile: Option<(i32, i32, i32)>,
    all_ingredients_seen_before_completion: Option<Value>,
    seen_open_generations: HashSet<u64>,
    preparation_generations: HashSet<u64>,
    active_generation: Option<u64>,
    pending_preparation: Option<(u64, ScriptStatus)>,
    bank_sessions: Vec<BankSession>,
    observations: u64,
}

impl CookWitness {
    fn new(case: FixtureCase, ids: [i32; 3]) -> Self {
        Self {
            case,
            ids,
            completed_seen: false,
            completion_tile: None,
            all_ingredients_seen_before_completion: None,
            seen_open_generations: HashSet::new(),
            preparation_generations: HashSet::new(),
            active_generation: None,
            pending_preparation: None,
            bank_sessions: Vec::new(),
            observations: 0,
        }
    }

    fn preparation_step_is_related(status: Option<&ScriptStatus>) -> bool {
        if !status.is_some_and(|status| {
            status.phase == script::native::NativePhase::Working
                && status.card == script::CompiledId("Quester")
        }) || status_text(status, "quest_id") != Some("cook")
            || status_text(status, "action_state") != Some("banking")
        {
            return false;
        }
        match (status_text(status, "stage"), status_text(status, "step_id")) {
            (Some("cook:0"), Some("start")) => true,
            (Some("cook:1"), Some(step)) => PREPARATION_STEPS.contains(&step),
            _ => false,
        }
    }

    fn observe(
        &mut self,
        snapshot: &GameSnapshot,
        status: Option<&ScriptStatus>,
        lifecycle: Option<&script::ScriptLifecycleReceipt>,
    ) -> Result<Option<Value>, String> {
        self.observations = self.observations.saturating_add(1);
        let inventory = item_counts(snapshot.inventory(), &self.ids);
        let quest_is_green = quest_complete(snapshot);
        let complete_now = quest_is_green
            || status_text(status, "colour") == Some("complete")
            || status_text(status, "stage") == Some("cook:2");
        if complete_now && !self.completed_seen {
            self.completion_tile = Some(
                snapshot
                    .tile()
                    .ok_or("Cook completion observed without a player tile")?,
            );
            self.completed_seen = true;
        }
        if !self.completed_seen
            && inventory.iter().all(|count| *count >= 1)
            && self.all_ingredients_seen_before_completion.is_none()
        {
            self.all_ingredients_seen_before_completion = Some(json!({
                "inventory_counts": inventory,
                "status": snapshot_status(status),
                "tick": snapshot.tick(),
            }));
        }

        if self.completed_seen {
            let expected = self
                .completion_tile
                .ok_or("Cook completion has no witnessed player tile")?;
            let observed = snapshot
                .tile()
                .ok_or("player tile disappeared after Cook completion")?;
            if observed != expected {
                return Err(format!(
                    "player moved after Cook completion: {expected:?} -> {observed:?}"
                ));
            }
            if matches!(
                status_text(status, "action_state"),
                Some("banking" | "walking")
            ) {
                return Err(format!(
                    "post-completion bank or movement action observed: {:?}",
                    snapshot_status(status)
                ));
            }
        }

        let bank_open = snapshot.bank_component_id() >= 0;
        if !bank_open {
            self.active_generation = None;
            self.pending_preparation = if Self::preparation_step_is_related(status) {
                snapshot
                    .bank_session_generation()
                    .checked_add(1)
                    .zip(status.cloned())
            } else {
                None
            };
        } else {
            let generation = snapshot.bank_session_generation();
            if generation == 0 {
                return Err("bank opened without a nonzero bank-session generation".into());
            }
            if self.active_generation != Some(generation) {
                if self.completed_seen {
                    return Err(format!(
                        "new bank-open generation {generation} began after Cook completion"
                    ));
                }
                if !self.seen_open_generations.insert(generation) {
                    return Err(format!(
                        "bank session generation {generation} reopened without a new generation"
                    ));
                }
                let preparation = self.pending_preparation.take().filter(|(expected, prior)| {
                    *expected == generation
                        && status.is_some_and(|current| {
                            current.run == prior.run
                                && current.card == prior.card
                                && current.phase == script::native::NativePhase::Working
                                && status_text(Some(current), "quest_id") == Some("cook")
                                && status_text(Some(current), "stage")
                                    == status_text(Some(prior), "stage")
                                && status_text(Some(current), "step_id")
                                    == status_text(Some(prior), "step_id")
                        })
                });
                let Some((_, preparation)) = preparation else {
                    return Err(format!(
                        "pre-completion bank session opened without the same Cook run's \
                         related preparation intent for generation {generation}: {:?}",
                        snapshot_status(status)
                    ));
                };
                self.preparation_generations.insert(generation);
                self.bank_sessions.push(BankSession::new(
                    generation,
                    snapshot,
                    status,
                    &preparation,
                    &self.ids,
                ));
                self.active_generation = Some(generation);
            }
            let session = self
                .bank_sessions
                .last_mut()
                .filter(|session| session.generation == generation)
                .ok_or_else(|| format!("open bank generation {generation} has no witness"))?;
            session.observe(snapshot, &self.ids);
        }

        let completed_lifecycle = lifecycle
            .is_some_and(|receipt| receipt.state == script::ScriptTerminalState::Completed);
        if !quest_is_green || !completed_lifecycle {
            return Ok(None);
        }
        self.finish(status, lifecycle).map(Some)
    }
    fn finish(
        &self,
        status: Option<&ScriptStatus>,
        lifecycle: Option<&script::ScriptLifecycleReceipt>,
    ) -> Result<Value, String> {
        if !self.completed_seen {
            return Err("native Quester ended without observed Cook completion".into());
        }
        if !lifecycle.is_some_and(|receipt| receipt.state == script::ScriptTerminalState::Completed)
        {
            return Err("Cook completion lacks a durable Completed lifecycle receipt".into());
        }
        if self.all_ingredients_seen_before_completion.is_none() {
            return Err(
                "all three Cook ingredients were never observed in inventory before completion"
                    .into(),
            );
        }
        match self.case {
            FixtureCase::Banked => {
                if self.preparation_generations.len() != 1 {
                    return Err(format!(
                        "banked Cook ingredients needed {} preparation bank sessions; expected exactly one",
                        self.preparation_generations.len()
                    ));
                }
                let session = self
                    .bank_sessions
                    .first()
                    .ok_or("banked fixture has no observed bank session")?;
                for (index, (alias, _)) in INGREDIENTS.iter().enumerate() {
                    let maximum = session.maximum_loaded_bank_counts.unwrap_or([0; 3]);
                    let minimum = session.minimum_loaded_bank_counts.unwrap_or([i32::MAX; 3]);
                    if maximum[index] < 1 || minimum[index] >= maximum[index] {
                        return Err(format!(
                            "banked ingredient {alias} was not observed in a loaded bank and decremented during preparation"
                        ));
                    }
                }
            }
            FixtureCase::Held => {
                if !self.preparation_generations.is_empty() {
                    return Err(format!(
                        "held Cook ingredients caused {} pre-completion bank sessions",
                        self.preparation_generations.len()
                    ));
                }
            }
            FixtureCase::Neither => {
                if self.preparation_generations.len() != 1 {
                    return Err(format!(
                        "neither fixture required {} preparation bank sessions; expected one empty-stock scan",
                        self.preparation_generations.len()
                    ));
                }
                let session = self
                    .bank_sessions
                    .first()
                    .ok_or("neither fixture has no observed bank scan")?;
                if !session.bank_loaded_seen
                    || session.maximum_loaded_bank_counts != Some([0, 0, 0])
                {
                    return Err(format!(
                        "neither fixture bank scan was not a loaded empty-stock observation: {session:?}"
                    ));
                }
            }
        }
        Ok(json!({
            "fixture_case": self.case.name(),
            "path_identity": {
                "id": "cook",
                "source": "crates/script/paths/289/cook.json",
                "owns_inventory": false,
                "modified": false,
            },
            "quest_complete": true,
            "native_phase": status.map(|status| format!("{:?}", status.phase)),
            "lifecycle": lifecycle.map(|receipt| json!({
                "runtime_generation": receipt.runtime_generation,
                "state": receipt.state,
                "tick": receipt.tick,
                "reason": receipt.reason,
            })),
            "completion_tile": self.completion_tile,
            "all_ingredients_seen_before_completion": self.all_ingredients_seen_before_completion,
            "preparation_bank_open_generations": sorted_generations(&self.preparation_generations),
            "bank_open_sessions": self.bank_sessions,
            "post_completion_bank_or_movement": false,
            "observations": self.observations,
        }))
    }
}

fn sorted_generations(generations: &HashSet<u64>) -> Vec<u64> {
    let mut values = generations.iter().copied().collect::<Vec<_>>();
    values.sort_unstable();
    values
}

fn make_observer(case: FixtureCase, ids: [i32; 3]) -> ObserveFamily {
    let mut witness = CookWitness::new(case, ids);
    Box::new(move |snapshot, status, lifecycle| witness.observe(snapshot, status, lifecycle))
}

fn session_log_lines_for_account(
    lines: &[SessionLogLine],
    account: Option<&str>,
) -> Vec<SessionLogLine> {
    let prefix = std::env::var("BOT_LIVE_NAME_PREFIX").unwrap_or_default();
    lines
        .iter()
        .filter(|line| {
            line.slot.as_deref().is_some_and(|slot| match account {
                Some(account) => slot == account,
                None => !prefix.is_empty() && slot.starts_with(&prefix),
            })
        })
        .cloned()
        .collect()
}

fn save_run_evidence(
    directory: &Path,
    result: &Result<Value, String>,
    logs: &[SessionLogLine],
) -> Result<(), String> {
    let account = result
        .as_ref()
        .ok()
        .and_then(|receipt| receipt.get("account"))
        .and_then(Value::as_str);
    let account_logs = session_log_lines_for_account(logs, account);
    let mut text_log = String::new();
    let mut jsonl = String::new();
    for line in &account_logs {
        use std::fmt::Write as _;
        writeln!(
            text_log,
            "[tick={}] [{}:{}] {}",
            line.tick
                .map_or_else(|| "?".to_owned(), |tick| tick.to_string()),
            line.source,
            line.level,
            line.message
        )
        .map_err(|error| error.to_string())?;
        jsonl.push_str(&serde_json::to_string(line).map_err(|error| error.to_string())?);
        jsonl.push('\n');
    }
    std::fs::write(directory.join("session.log"), text_log)
        .map_err(|error| format!("write session.log: {error}"))?;
    std::fs::write(directory.join("session.jsonl"), jsonl)
        .map_err(|error| format!("write session.jsonl: {error}"))?;
    let result_text = match result {
        Ok(receipt) => json!({"result": "passed", "receipt": receipt}),
        Err(error) => json!({"result": "failed", "error": error}),
    };
    std::fs::write(
        directory.join("live-proof.json"),
        serde_json::to_vec_pretty(&json!({
            "result": result_text,
            "session_log": directory.join("session.log"),
            "captured_account_log_records": account_logs.len(),
        }))
        .map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("write live-proof.json: {error}"))?;
    Ok(())
}

fn run_case(case: FixtureCase) {
    let _serial = LIVE_CELL_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let log_store = session_log_store();
    log_store
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clear();
    let evidence = evidence_directory(case)
        .unwrap_or_else(|error| panic!("{} evidence setup failed: {error}", case.name()));
    let (cell, start, ids) = build_case(case)
        .unwrap_or_else(|error| panic!("{} Cook fixture setup failed: {error}", case.name()));
    let result = quester_live::run_family(cell, start, make_observer(case, ids));
    let logs = log_store
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone();
    save_run_evidence(&evidence, &result, &logs).unwrap_or_else(|error| {
        panic!(
            "{} evidence write failed: {error}; path={}",
            case.name(),
            evidence.display()
        )
    });
    let receipt = result.unwrap_or_else(|error| {
        panic!(
            "{} Cook bank-loop live proof failed: {error}; evidence={}",
            case.name(),
            evidence.display()
        )
    });
    assert_eq!(
        receipt["starts"].as_u64(),
        Some(1),
        "one explicit native Start"
    );
    assert_eq!(receipt["deaths"].as_u64(), Some(0), "zero deaths");
    assert_eq!(receipt["quest"].as_str(), Some("cook"));
    assert_eq!(receipt["family_receipt"]["quest_complete"], json!(true));
    assert_eq!(
        receipt["family_receipt"]["path_identity"]["id"],
        json!("cook")
    );
    assert_eq!(
        receipt["family_receipt"]["path_identity"]["owns_inventory"],
        json!(false)
    );

    let account = receipt["account"]
        .as_str()
        .unwrap_or_else(|| panic!("{} result has no live account", case.name()));
    let account_logs = session_log_lines_for_account(&logs, Some(account));
    for (stage, step, kind) in ROOT_BEGIN_LINES {
        let expected = format!("quester cook: stage {stage} step {step} ({kind}) begin");
        assert!(
            account_logs.iter().any(|line| line.message == expected),
            "missing root-step begin line {expected:?}; session log: {}",
            evidence.join("session.log").display()
        );
    }
    for (step, kind) in ROOT_SETTLED_STEPS {
        let expected = format!("step {step} ({kind}) settled");
        assert!(
            account_logs
                .iter()
                .any(|line| line.message.contains(&expected)),
            "missing root-step settled event {expected:?}; session log: {}",
            evidence.join("session.log").display()
        );
    }
    assert!(
        account_logs
            .iter()
            .any(|line| line.message == "quester cook: finish"),
        "missing terminal Quester finish trace; session log: {}",
        evidence.join("session.log").display()
    );
    assert!(
        !account_logs.iter().any(|line| {
            line.message.contains("step hand-in (talk) failed:")
                || line.message.contains("step hand-in (talk) begin failed:")
        }),
        "hand-in failed even though the quest completed; session log: {}",
        evidence.join("session.log").display()
    );
    match case {
        FixtureCase::Neither => {
            for (recipe, child) in ACQUISITION_CHILD_LINES {
                assert!(
                    account_logs.iter().any(|line| {
                        line.message.starts_with("quester cook: stage ")
                            && line
                                .message
                                .contains(&format!("recipe {recipe} child {child} begin"))
                    }),
                    "missing real acquisition child begin for {recipe}/{child}; session log: {}",
                    evidence.join("session.log").display()
                );
            }
        }
        FixtureCase::Banked | FixtureCase::Held => {
            for (recipe, child) in ACQUISITION_CHILD_LINES {
                assert!(
                    !account_logs.iter().any(|line| {
                        line.message.contains(&format!(
                            "recipe {recipe} child {child} begin"
                        ))
                    }),
                    "{} fixture unexpectedly ran acquisition child {recipe}/{child}; session log: {}",
                    case.name(),
                    evidence.join("session.log").display()
                );
            }
        }
    }
    println!(
        "Cook bank-loop {:?} passed; receipt={}; session_log={}",
        case,
        receipt,
        evidence.join("session.log").display()
    );
}

#[test]
fn original_cook_fixtures_build_without_a_synthetic_stage_zero_varp_hint() {
    for case in [FixtureCase::Banked, FixtureCase::Held, FixtureCase::Neither] {
        let (cell, _, _) = build_case(case).expect("original Cook fixture must be constructible");
        assert_eq!(cell.scenario.settings.deadline, LIVE_DEADLINE);
        assert_eq!(cell.start_settings["quests"], json!(["cook"]));
        let gate = cell
            .scenario
            .steps
            .iter()
            .position(|step| step.name == "prove maxed no-loadout Cook fixture before Start")
            .unwrap();
        let start = cell
            .scenario
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap();
        assert!(gate < start);
        assert!(
            matches!(cell.scenario.steps[gate - 1].kind, StepKind::Perform { .. }),
            "the fixture must re-establish its stand after the last relog, before its Start gate",
        );
        assert_eq!(
            cell.scenario.steps[..start]
                .iter()
                .filter(|step| matches!(step.kind, StepKind::Relog))
                .count(),
            2,
        );
        let final_relog = cell.scenario.steps[..start]
            .iter()
            .rposition(|step| matches!(step.kind, StepKind::Relog))
            .unwrap();
        assert!(matches!(
            cell.scenario.steps[final_relog + 1].kind,
            StepKind::Repeat { .. }
        ));
        assert_eq!(
            cell.scenario.steps[final_relog + 1].wait.arm,
            Proof::FreshTutorial,
            "Cook must confirm a fresh tutorial reseed after the final relog",
        );
    }
}

#[cfg(test)]
mod witness_lifecycle_tests {
    use super::*;
    use api::obj_names::ItemDefView;
    use api::snapshot::{ItemActionFamily, ItemContainer, QuestStatusView};

    const IDS: [i32; 3] = [101, 102, 103];
    const TILE: WorldTile = WorldTile {
        x: 3209,
        z: 3215,
        level: 0,
    };

    fn snapshot(green: bool) -> GameSnapshot {
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_tile(TILE);
        snapshot.seed_inventory(
            IDS.iter()
                .enumerate()
                .map(|(slot, id)| ItemView {
                    def: ItemDefView {
                        id: *id,
                        name: Some(format!("ingredient-{id}")),
                        stackable: false,
                        members: false,
                        base_value: 0,
                        noted: false,
                        certificate_link: -1,
                        certificate_template: -1,
                    },
                    container: ItemContainer::Inventory,
                    action_family: ItemActionFamily::Held,
                    slot: slot as i32,
                    count: 1,
                    actions: Vec::new(),
                    component_id: -1,
                })
                .collect(),
            28,
        );
        snapshot.seed_quest_statuses(
            if green {
                vec![QuestStatusView {
                    component_id: 1,
                    name: "Cook's Assistant".into(),
                    colour: 0x00F800,
                }]
            } else {
                Vec::new()
            },
            true,
        );
        snapshot
    }

    fn receipt(state: script::ScriptTerminalState) -> script::ScriptLifecycleReceipt {
        script::ScriptLifecycleReceipt {
            runtime_generation: 7,
            state,
            tick: 42,
            reason: "test terminal".into(),
        }
    }

    fn witness_with_prior_ingredients() -> CookWitness {
        let mut witness = CookWitness::new(FixtureCase::Held, IDS);
        assert_eq!(witness.observe(&snapshot(false), None, None).unwrap(), None);
        assert!(witness.all_ingredients_seen_before_completion.is_some());
        witness
    }

    fn preparing_status(step: &str, action: &str) -> ScriptStatus {
        ScriptStatus {
            run: RunKey {
                slot: 1,
                run: 1,
                session: 1,
            },
            card: script::CompiledId("Quester"),
            phase: script::native::NativePhase::Working,
            active_settings: 0,
            pending_settings: None,
            fields: [
                ("quest_id", "cook"),
                ("stage", "cook:1"),
                ("step_id", step),
                ("action_state", action),
                ("colour", "in_progress"),
            ]
            .map(|(key, value)| script::native::StatusField {
                key,
                label: key,
                value: StatusValue::Text(Arc::from(value)),
            })
            .into(),
            failure: None,
        }
    }

    fn pending_bank() -> (CookWitness, GameSnapshot) {
        let mut witness = CookWitness::new(FixtureCase::Neither, IDS);
        let snapshot = snapshot(false);
        witness
            .observe(&snapshot, Some(&preparing_status("egg", "banking")), None)
            .unwrap();
        assert!(witness.pending_preparation.is_some());
        (witness, snapshot)
    }

    fn open_bank(snapshot: &mut GameSnapshot) {
        snapshot.seed_bank_observation(5382, 1, Some(Vec::new()), Vec::new());
    }

    #[test]
    fn same_frame_scan_settlement_preserves_causal_opening_proof() {
        let (mut witness, mut snapshot) = pending_bank();
        open_bank(&mut snapshot);
        assert_eq!(
            witness
                .observe(&snapshot, Some(&preparing_status("egg", "working")), None)
                .unwrap(),
            None
        );
        assert!(witness.pending_preparation.is_none());
        let session = &witness.bank_sessions[0];
        assert_eq!(session.generation, 1);
        assert_eq!(session.action_state_at_open.as_deref(), Some("working"));
        assert_eq!(
            session.opening_provision["status"]["action_state"],
            "banking"
        );
        assert_eq!(session.opening_provision["run"]["run"], 1);
        assert!(session.bank_loaded_seen);
    }

    #[test]
    fn modal_without_observed_preparation_is_not_a_cook_bank_session() {
        let mut witness = CookWitness::new(FixtureCase::Neither, IDS);
        let mut snapshot = snapshot(false);
        open_bank(&mut snapshot);
        assert!(witness
            .observe(&snapshot, Some(&preparing_status("egg", "working")), None)
            .unwrap_err()
            .contains("related preparation intent"));
    }

    #[test]
    fn closed_non_banking_observation_clears_stale_preparation() {
        let (mut witness, mut snapshot) = pending_bank();
        let status = preparing_status("egg", "working");
        witness.observe(&snapshot, Some(&status), None).unwrap();
        assert!(witness.pending_preparation.is_none());
        open_bank(&mut snapshot);
        assert!(witness.observe(&snapshot, Some(&status), None).is_err());
    }

    #[test]
    fn preparation_cannot_cross_native_run_or_step_or_blocked_boundary() {
        for boundary in ["run", "step", "blocked", "card"] {
            let (mut witness, mut snapshot) = pending_bank();
            let mut status = preparing_status("egg", "working");
            match boundary {
                "run" => status.run.run += 1,
                "step" => status = preparing_status("hand-in", "working"),
                "blocked" => status.phase = script::native::NativePhase::Blocked,
                "card" => status.card = script::CompiledId("Other"),
                _ => unreachable!(),
            }
            open_bank(&mut snapshot);
            assert!(
                witness.observe(&snapshot, Some(&status), None).is_err(),
                "{boundary} must not inherit the earlier Cook bank intent"
            );
        }
    }

    #[test]
    fn preparation_is_bound_to_exactly_the_next_session_generation() {
        let (mut witness, mut snapshot) = pending_bank();
        open_bank(&mut snapshot);
        snapshot.seed_bank_observation(-1, 2, None, Vec::new());
        open_bank(&mut snapshot);
        assert_eq!(snapshot.bank_session_generation(), 3);
        assert!(witness
            .observe(&snapshot, Some(&preparing_status("egg", "working")), None)
            .is_err());
    }

    #[test]
    fn consumed_preparation_cannot_authorize_another_opening() {
        let (mut witness, mut snapshot) = pending_bank();
        let status = preparing_status("egg", "working");
        open_bank(&mut snapshot);
        witness.observe(&snapshot, Some(&status), None).unwrap();
        snapshot.seed_bank_observation(-1, 2, None, Vec::new());
        witness.observe(&snapshot, Some(&status), None).unwrap();
        open_bank(&mut snapshot);
        assert!(witness.observe(&snapshot, Some(&status), None).is_err());
        assert_eq!(witness.bank_sessions.len(), 1);
    }

    #[test]
    fn earlier_preparation_never_authorizes_a_post_completion_opening() {
        let (mut witness, _) = pending_bank();
        let mut snapshot = snapshot(true);
        let status = preparing_status("egg", "working");
        witness.observe(&snapshot, Some(&status), None).unwrap();
        assert!(witness.completed_seen);
        open_bank(&mut snapshot);
        assert!(witness
            .observe(&snapshot, Some(&status), None)
            .unwrap_err()
            .contains("after Cook completion"));
    }

    #[test]
    fn native_status_none_completed_lifecycle_and_green_quest_succeed() {
        let mut witness = witness_with_prior_ingredients();
        let result = witness
            .observe(
                &snapshot(true),
                None,
                Some(&receipt(script::ScriptTerminalState::Completed)),
            )
            .unwrap()
            .expect("durable completion and green quest tab should finish Cook proof");

        assert!(result["native_phase"].is_null());
        assert_eq!(result["lifecycle"]["state"], "completed");
        assert_eq!(result["quest_complete"], true);
    }

    #[test]
    fn completed_lifecycle_without_green_quest_remains_pending() {
        let mut witness = witness_with_prior_ingredients();
        assert_eq!(
            witness
                .observe(
                    &snapshot(false),
                    None,
                    Some(&receipt(script::ScriptTerminalState::Completed)),
                )
                .unwrap(),
            None
        );
        assert!(!witness.completed_seen);
    }

    #[test]
    fn green_quest_without_completed_lifecycle_never_succeeds() {
        for lifecycle in [
            None,
            Some(script::ScriptTerminalState::Stopped),
            Some(script::ScriptTerminalState::Failed),
            Some(script::ScriptTerminalState::Cancelled),
        ] {
            let mut witness = witness_with_prior_ingredients();
            let lifecycle = lifecycle.map(receipt);
            assert_eq!(
                witness
                    .observe(&snapshot(true), None, lifecycle.as_ref())
                    .unwrap(),
                None
            );
            assert!(witness.completed_seen);
        }
    }

    #[test]
    fn green_quest_guards_movement_before_completed_lifecycle_arrives() {
        let mut witness = witness_with_prior_ingredients();
        let mut posted = snapshot(true);
        assert_eq!(witness.observe(&posted, None, None).unwrap(), None);
        posted.seed_tile(WorldTile {
            x: TILE.x + 1,
            ..TILE
        });
        let error = witness
            .observe(
                &posted,
                None,
                Some(&receipt(script::ScriptTerminalState::Completed)),
            )
            .unwrap_err();
        assert!(error.contains("player moved after Cook completion"));
    }
}

#[test]
#[ignore = "requires Engine A and the live environment documented at the top of this file"]
fn live_cook_banked_ingredients_use_one_preparation_bank_session() {
    run_case(FixtureCase::Banked);
}

#[test]
#[ignore = "requires Engine A and the live environment documented at the top of this file"]
fn live_cook_held_ingredients_need_no_preparation_bank_session() {
    run_case(FixtureCase::Held);
}

#[test]
#[ignore = "requires Engine A and the live environment documented at the top of this file"]
fn live_cook_neither_ingredients_are_acquired_from_real_content() {
    run_case(FixtureCase::Neither);
}
