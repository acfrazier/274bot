//! Generic Path live smoke: one Base40 cell for a content quest id.
//!
//! This is the permanent form of the PATHS-OVERNIGHT throwaway runner. It
//! runs the Path id in `QUESTER_PATH` from the embedded release index, or
//! from `QUESTER_PATH_DIR` through the existing `FolderSource` registry.
//! Optional account seeds come from the `QUESTER_SEEDS` JSON file.
//! Stage mode checks the expected stage after Start. Clean mode checks terminal
//! completion without seeding the target Path's stage.
//! Set `QUESTER_ALLOW_DANGER_ZONES=1` to run a separate cell with the Quester
//! card's `allow_danger_zones` setting enabled; unset keeps the default.
//! The cell runs under a fixed deadline.
//!
//! Run from the repository root against an explicitly selected local 289
//! engine, with a throwaway HOME containing the `local-289q` profile:
//!
//! ```text
//! env HOME=<throwaway-home> LIVE=1 BOT_CPU=1 BOT_LIVE_NAME_PREFIX=qh \
//!   BOT_NAV_BUILD=skip BOT_SERVER_PROFILE=local-289q \
//!   WORLD_GAME_PORT=44694 WORLD_HTTP_PORT=1180 QUESTER_LIVE_KEEP_HOME=1 \
//!   WORLD_NAV_PACK=<274bot.navpack> WORLD_ENGINE_DIR=<engine-q-dir> \
//!   RS2B0T=<catalog-root> BOT_CACHE_DIR=<owned-writable-APFS-cache-clone> \
//!   LIVE_EVIDENCE_DIR=<evidence-root> QUESTER_PATH=cook \
//!   QUESTER_TICK_MS=300 QUESTER_SUSTAIN_RUN=1 QUESTER_SEEDS=<evidence-root>/seeds/cook.json \
//!   cargo test \
//!     -p host-play --features "live-harness test-support" \
//!     --test quester_path_live live_quester_path_smoke -- --ignored --nocapture --test-threads=1
//! ```
//! `QUESTER_TICK_MS` changes world-wide tick speed; use it only on isolated
//! Engine Q, never on shared Engine A or the builder. Run without it to keep
//! the world's current tick rate.
//! For a custom local profile, omit `members` so Engine Q's `world.json`
//! remains the source of world membership.
//!
//! Seeds file shape (`seeds/cook.json`):
//!
//! ```json
//! {
//!   "stage": "cook:0",
//!   "stand": [3209, 3215, 0],
//!   "loadout": null,
//!   "extra_items": [],
//!   "seed_vars": [["cookquest", 0]],
//!   "before_relog": [],
//!   "expect": ["cook:1"],
//!   "mode": "stage"
//! }
//! ```
//!
//! For clean completion, set `mode` to `clean` and `expect` to `[]`.
//! The `stage` field stays required by the shared schema, but the fixture does
//! not apply it to the account.
//!
//! For campaign seed plans, the optional `gap` object names a partial-guide
//! boundary: `{"step_id":"GAP-peer-riddle-ui","park_reason":"wait exhausted"}`.
//! For a `no step for stage` gap it also names the boundary `stage`.
//!
//! The generic smoke fields above are separate from campaign `gap` metadata.
//!
//! `seed_vars` names the stage varp for Paths with no progress varp hint
//! (released Cook); `before_relog` holds raw fixture cheats such as
//! `givebank` stock and prerequisite quest flags. Only the tested account is
//! seeded before Start: no NPCs, locs or chance-drop waits.
#![cfg(all(feature = "live-harness", feature = "test-support"))]

#[path = "common/quester_live.rs"]
mod quester_live;

use api::snapshot::WorldTile;
use quester_live::{Cell, Mode, PathCell};
use scenario::quester::{FixtureLoadout, QuestFixtureProfile, QuesterPathSeeds, TestProfile};
use script::quester::registry;
use std::path::PathBuf;
use std::time::Duration;

/// Fixed whole-cell deadline (fixture seed plus quest). Not env-configurable:
/// a smoke that cannot Start and settle inside it fails instead of burning
/// the shared engine. 45 minutes covers the long Paths (Prince Ali Rescue
/// ran about 31 minutes overnight).
const DEADLINE: Duration = Duration::from_secs(2700);

/// The harness row for an env-driven quest: static id, display and profile,
/// so the cell needs no allocation to satisfy fixture lifetimes. Refuses
/// quests the profile table moved off Base40.
fn smoke_row(quest: &str) -> Result<QuestFixtureProfile, String> {
    let row = scenario::quester::fixture_row(quest)?;
    if row.profile != TestProfile::Base40 {
        return Err(format!("{quest} is not a Base40 smoke quest"));
    }
    Ok(row)
}

/// Select the same embedded or folder-backed source the headed entry uses.
fn resolve_source(quest: &str) -> Result<&'static str, String> {
    let folder = std::env::var_os("QUESTER_PATH_DIR").map(PathBuf::from);
    scenario::quester::reload_quester_path_source(quest, folder.as_deref())
}

fn smoke_cell(
    row: QuestFixtureProfile,
    seeds: &QuesterPathSeeds,
    loadout: Option<FixtureLoadout<'_>>,
    extra_items: &[(&str, i32)],
    seed_vars: &[(&str, i32)],
    allow_danger_zones: bool,
) -> Result<Cell, String> {
    let (mode, stage) = match seeds.mode.as_deref().unwrap_or("stage") {
        "stage" => (
            Mode::Stage {
                expect: seeds.expect.clone(),
            },
            Some(seeds.stage.as_str()),
        ),
        "clean" => (Mode::Clean, None),
        other => return Err(format!("unknown smoke mode {other:?}")),
    };
    let mut cell = quester_live::path_cell(PathCell {
        quest: row.quest,
        display: row.display,
        label: "path-smoke".to_owned(),
        stage,
        loadout,
        extra_items,
        seed_vars,
        stand: WorldTile {
            x: seeds.stand[0],
            z: seeds.stand[1],
            level: seeds.stand[2],
        },
        mode,
        before_relog: seeds.before_relog_steps(),
    })?;
    if allow_danger_zones {
        cell.start_settings
            .insert("allow_danger_zones".into(), true.into());
    }
    cell.scenario.settings.deadline = DEADLINE;
    Ok(cell)
}

fn allow_danger_zones_from_env() -> Result<bool, String> {
    match std::env::var("QUESTER_ALLOW_DANGER_ZONES") {
        Ok(value) if value == "1" => Ok(true),
        Ok(value) if value == "0" => Ok(false),
        Ok(_) => Err("QUESTER_ALLOW_DANGER_ZONES must be 0 or 1".into()),
        Err(std::env::VarError::NotPresent) => Ok(false),
        Err(error) => Err(format!("QUESTER_ALLOW_DANGER_ZONES: {error}")),
    }
}

#[test]
#[ignore = "requires LIVE=1 and an explicitly selected local 289 engine; see common/quester_live.rs"]
fn live_quester_path_smoke() {
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"));
    let quest = std::env::var("QUESTER_PATH").expect("QUESTER_PATH names the Path");
    let seeds_path = PathBuf::from(std::env::var_os("QUESTER_SEEDS").expect("QUESTER_SEEDS"));
    let text = std::fs::read_to_string(&seeds_path)
        .unwrap_or_else(|error| panic!("read {}: {error}", seeds_path.display()));
    let seeds = scenario::quester::parse_quester_path_seeds(&text, "QUESTER_SEEDS")
        .unwrap_or_else(|error| panic!("{error}"));
    let row = smoke_row(&quest).unwrap_or_else(|error| panic!("{error}"));
    let before = registry::source();
    let source = resolve_source(&quest).unwrap_or_else(|error| panic!("{error}"));
    let loadout = seeds
        .fixture_loadout()
        .unwrap_or_else(|error| panic!("{error}"));
    let extra_items = seeds.extra_item_refs();
    let seed_vars = seeds.seed_var_refs();
    let allow_danger_zones =
        allow_danger_zones_from_env().unwrap_or_else(|error| panic!("{error}"));
    let cell = smoke_cell(
        row,
        &seeds,
        loadout,
        &extra_items,
        &seed_vars,
        allow_danger_zones,
    )
    .unwrap_or_else(|error| panic!("build {quest} smoke cell from {source}: {error}"));
    let result = quester_live::run_quester_path(cell);
    println!("PATH-SMOKE {quest} ({source}): {:?}", result.is_ok());
    registry::set_source(before);
    result.unwrap();
}

/// Campaign plans use the quest-watch translator, not qualification profiles.
#[derive(serde::Deserialize)]
struct CampaignManifest {
    accounts: Vec<CampaignAccount>,
}

#[derive(serde::Deserialize)]
struct CampaignAccount {
    quest: String,
    plan: PathBuf,
    gang: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct CampaignGap {
    step_id: String,
    park_reason: String,
    #[serde(default)]
    stage: Option<String>,
}

struct CampaignEntry {
    cell: Cell,
    gap: Option<CampaignGap>,
}

static CAMPAIGN_LOGS: std::sync::Mutex<Vec<serde_json::Value>> = std::sync::Mutex::new(Vec::new());
struct CampaignLogSink;
static CAMPAIGN_LOG_SINK: CampaignLogSink = CampaignLogSink;

impl api::hostlog::Sink for CampaignLogSink {
    fn record(&self, record: &api::hostlog::Record<'_>) {
        CAMPAIGN_LOGS.lock().unwrap().push(serde_json::json!({
            "account": record.slot,
            "tick": record.tick,
            "source": record.source.label(),
            "level": record.level.label(),
            "message": record.message,
        }));
    }
}

fn campaign_catalog() -> Result<&'static api::quest_facts::QuestCatalog, String> {
    static CATALOG: std::sync::LazyLock<
        Result<std::sync::Arc<api::quest_facts::QuestCatalog>, String>,
    > = std::sync::LazyLock::new(|| {
        api::selected::FamilyPreparation::run(|worker| {
            let selected = api::game_data::for_revision(api::selected::ClientRevision::R289)?;
            selected
                .prepare_quests(worker)
                .map_err(|error| format!("{error:?}"))
        })
        .map_err(|error| format!("{error:?}"))?
        .join()
        .map_err(|error| format!("{error:?}"))?
    });
    CATALOG
        .as_ref()
        .map(|catalog| catalog.as_ref())
        .map_err(Clone::clone)
}

fn campaign_gap(plan: &serde_json::Value) -> Result<Option<CampaignGap>, String> {
    let Some(value) = plan.get("gap") else {
        return Ok(None);
    };
    let gap: CampaignGap =
        serde_json::from_value(value.clone()).map_err(|error| format!("campaign gap: {error}"))?;
    if gap.step_id.is_empty() {
        return Err("campaign gap step_id must not be empty".into());
    }
    match gap.park_reason.as_str() {
        "wait exhausted" if gap.stage.as_ref().is_none_or(|stage| !stage.is_empty()) => {}
        "no step for stage" if gap.stage.as_ref().is_some_and(|stage| !stage.is_empty()) => {}
        "wait exhausted" => return Err("wait gap stage must be non-empty when supplied".into()),
        "no step for stage" => return Err("no-step gap requires a non-empty stage".into()),
        _ => return Err("campaign gap park_reason is not an approved native park reason".into()),
    }
    Ok(Some(gap))
}

fn campaign_cells(manifest_path: &std::path::Path) -> Result<Vec<CampaignEntry>, String> {
    let manifest: CampaignManifest =
        serde_json::from_slice(&std::fs::read(manifest_path).map_err(|error| error.to_string())?)
            .map_err(|error| error.to_string())?;
    if !(1..=2).contains(&manifest.accounts.len()) {
        return Err("campaign requires one account, or two reciprocal partner roles".into());
    }
    let quest = &manifest.accounts[0].quest;
    if manifest
        .accounts
        .iter()
        .any(|account| &account.quest != quest)
    {
        return Err("campaign runs exactly one guide per command".into());
    }
    let folder = std::env::var_os("QUESTER_PATH_DIR")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or("campaign requires absolute QUESTER_PATH_DIR (folder mode)")?;
    if scenario::quester::reload_quester_path_source(quest, Some(&folder))? != "folder" {
        return Err("campaign must load its guide from the staging folder".into());
    }
    let row = campaign_catalog()?
        .quest(quest)
        .map_err(|error| format!("{error:?}"))?;
    let (path, _) = scenario::quester::load_quester_path_document(quest)?;
    if path.partner.is_some() != (manifest.accounts.len() == 2) {
        return Err("campaign account count does not match the guide's partner declaration".into());
    }
    if manifest.accounts.len() == 2
        && (manifest.accounts[0].gang.as_deref() != Some("phoenix")
            || manifest.accounts[1].gang.as_deref() != Some("blackarm"))
    {
        return Err("campaign partner roles must be ordered phoenix then blackarm".into());
    }
    manifest.accounts.iter().map(|account| {
        let plan_path = manifest_path.parent().ok_or("manifest has no parent")?.join(&account.plan);
        let text = std::fs::read_to_string(&plan_path).map_err(|error| error.to_string())?;
        let plan: serde_json::Value = serde_json::from_str(&text).map_err(|error| error.to_string())?;
        if plan["quest"].as_str() != Some(quest) {
            return Err("campaign seed plan quest does not match its guide".into());
        }
        let gap = campaign_gap(&plan)?;
        let mut scenario = scenario::quest_watch::quest_campaign_scenario(&text)?;
        scenario.settings.deadline = DEADLINE;
        let mut settings = serde_json::Map::from_iter([
            ("quests".into(), serde_json::json!([quest])),
            ("allow_teleports".into(), true.into()),
            ("allow_wilderness".into(), true.into()),
            ("allow_danger_zones".into(), true.into()),
        ]);
        if let Some(gang) = &account.gang {
            settings.insert("gang".into(), gang.clone().into());
        }
        Ok(CampaignEntry {
            cell: Cell {
                quest: row.id.0.as_ref(),
                display: row.display.as_ref(),
                label: "campaign".into(),
                scenario,
                start_settings: settings,
                mode: Mode::Clean,
                observe_start: Some(Box::new(move |snapshot| Ok(serde_json::json!({
                    "seed_plan": plan,
                    "tick": snapshot.tick(),
                    "tile": snapshot.tile(),
                    "inventory": snapshot.inventory().iter().filter(|item| item.count > 0)
                        .map(|item| [item.def.id, item.count]).collect::<Vec<_>>(),
                    "stats": snapshot.stats().iter().map(|stat|
                        serde_json::json!({"index": stat.index, "base": stat.base, "effective": stat.effective})
                    ).collect::<Vec<_>>(),
                })))),
            },
            gap,
        })
    }).collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RootStepTrace {
    stage: String,
    step_id: String,
    kind: String,
    event: String,
    repeated: bool,
}

fn parse_root_step_trace(quest: &str, message: &str) -> Option<RootStepTrace> {
    let prefix = format!("quester {quest}: stage ");
    let (stage, detail) = message.strip_prefix(&prefix)?.split_once(" step ")?;
    let (label, event) = detail.rsplit_once(") ")?;
    let (step_id, kind) = label.rsplit_once(" (")?;
    if stage.is_empty() || step_id.is_empty() || kind.is_empty() {
        return None;
    }
    let (event, repeated) = match event.rsplit_once(" (repeated ") {
        Some((base, count))
            if count
                .strip_suffix(" additional times)")
                .and_then(|count| count.parse::<u32>().ok())
                .is_some_and(|count| count > 0) =>
        {
            (base, true)
        }
        _ => (event, false),
    };
    if event != "begin" && event != "settled" && !event.starts_with("failed:") {
        return None;
    }
    Some(RootStepTrace {
        stage: stage.to_owned(),
        step_id: step_id.to_owned(),
        kind: kind.to_owned(),
        event: event.to_owned(),
        repeated,
    })
}

fn campaign_trace_messages(
    quest: &str,
    logs: &[serde_json::Value],
) -> std::collections::BTreeMap<String, Vec<String>> {
    let prefix = format!("quester {quest}:");
    let mut by_account = std::collections::BTreeMap::<String, Vec<String>>::new();
    for record in logs {
        let (Some(account), Some(message)) =
            (record["account"].as_str(), record["message"].as_str())
        else {
            continue;
        };
        if message.starts_with(&prefix)
            || message == "quester trace truncated after the event limit"
            || message == "quester terminal trace event was truncated"
        {
            by_account
                .entry(account.to_owned())
                .or_default()
                .push(message.to_owned());
        }
    }
    by_account
}

fn insert_actor_receipt(
    receipts: &mut std::collections::BTreeMap<String, serde_json::Value>,
    actor: &serde_json::Value,
) -> Result<(), String> {
    let account = actor["account"]
        .as_str()
        .ok_or("campaign actor receipt has no account")?;
    if let Some(existing) = receipts.get(account) {
        if existing != actor {
            return Err(format!("conflicting campaign receipts for {account}"));
        }
    } else {
        receipts.insert(account.to_owned(), actor.clone());
    }
    Ok(())
}

fn campaign_actor_receipts(
    evidence_root: &std::path::Path,
    quest: &str,
) -> Result<std::collections::BTreeMap<String, serde_json::Value>, String> {
    let quest_root = evidence_root.join(quest);
    let entries = match std::fs::read_dir(&quest_root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(std::collections::BTreeMap::new());
        }
        Err(error) => return Err(error.to_string()),
    };
    let mut receipts = std::collections::BTreeMap::new();
    for entry in entries {
        let entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path().join("receipt.json");
        if !path.is_file() {
            continue;
        }
        let receipt: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).map_err(|error| error.to_string())?)
                .map_err(|error| format!("{}: {error}", path.display()))?;
        if let Some(roles) = receipt["roles"].as_array() {
            for actor in roles {
                insert_actor_receipt(&mut receipts, actor)?;
            }
        } else if receipt["account"].is_string() {
            insert_actor_receipt(&mut receipts, &receipt)?;
        }
    }
    Ok(receipts)
}

#[derive(Default)]
struct StepSettlement {
    stage: String,
    step_id: String,
    kind: String,
    started: bool,
    settled: bool,
}

fn prior_started_steps_settled(
    events: &[(usize, RootStepTrace)],
    boundary: usize,
) -> Option<Vec<serde_json::Value>> {
    let mut steps = std::collections::BTreeMap::<(String, String), StepSettlement>::new();
    for (_, event) in events.iter().filter(|entry| entry.0 < boundary) {
        let key = (event.stage.clone(), event.step_id.clone());
        let step = steps.entry(key).or_insert_with(|| StepSettlement {
            stage: event.stage.clone(),
            step_id: event.step_id.clone(),
            kind: event.kind.clone(),
            ..StepSettlement::default()
        });
        if event.kind != step.kind {
            return None;
        }
        if event.repeated && (event.event == "begin" || event.event.starts_with("failed:")) {
            return None;
        }
        match event.event.as_str() {
            "begin" => {
                if step.started {
                    return None;
                }
                step.started = true;
            }
            "settled" if step.started => step.settled = true,
            "settled" => return None,
            failed if failed.starts_with("failed:") && step.started => step.settled = false,
            failed if failed.starts_with("failed:") => return None,
            _ => {}
        }
    }
    if steps.values().any(|step| !step.started || !step.settled) {
        return None;
    }
    Some(
        steps
            .into_values()
            .map(|step| {
                serde_json::json!({
                    "stage": step.stage,
                    "step_id": step.step_id,
                    "kind": step.kind,
                    "settled": step.settled,
                })
            })
            .collect(),
    )
}

fn park_context_step<'a>(quest: &str, message: &'a str) -> Option<&'a str> {
    let prefix = format!("quester {quest}: park context step=");
    message
        .strip_prefix(&prefix)?
        .split_whitespace()
        .next()
        .filter(|step| !step.is_empty())
}

fn prove_actor_gap(
    gap: &CampaignGap,
    quest: &str,
    account: &str,
    messages: &[String],
    receipt: &serde_json::Value,
) -> Option<serde_json::Value> {
    if receipt["account"].as_str() != Some(account)
        || receipt["quest"].as_str() != Some(quest)
        || receipt["quest_complete"].as_bool() != Some(false)
        || receipt["deaths"].as_i64() != Some(0)
        || receipt["death_count_total_across_runs"].as_i64() != Some(0)
        || receipt["death_sent"].as_bool() != Some(false)
        || receipt["death_injections"].as_i64() != Some(0)
        || receipt["death_observed"].as_bool() != Some(false)
        || !receipt["error"].is_null()
        || !receipt["capture_error"].is_null()
        || receipt["end_captured"].as_bool() != Some(true)
    {
        return None;
    }

    let final_status = &receipt["final_status"];
    let fields = final_status.get("fields")?;
    let final_stage = fields["quest"]["stage"].as_str()?;
    if final_status["phase"].as_str() != Some("Blocked")
        || fields["quest_id"].as_str() != Some(quest)
        || fields["action_state"].as_str() != Some("blocked")
        || fields["block_reason"].as_str() != Some(gap.park_reason.as_str())
    {
        return None;
    }
    if gap
        .stage
        .as_deref()
        .is_some_and(|expected_stage| expected_stage != final_stage)
    {
        return None;
    }

    if messages.iter().any(|message| {
        message == "quester trace truncated after the event limit"
            || message == "quester terminal trace event was truncated"
    }) || messages
        .iter()
        .filter(|message| message.starts_with(&format!("quester {quest}: run start ")))
        .count()
        != 1
    {
        return None;
    }
    let contexts = messages
        .iter()
        .enumerate()
        .filter_map(|(index, message)| {
            park_context_step(quest, message).map(|step| (index, step.to_owned()))
        })
        .collect::<Vec<_>>();
    let parks = messages
        .iter()
        .enumerate()
        .filter_map(|(index, message)| {
            message
                .strip_prefix(&format!("quester {quest}: park: "))
                .map(|reason| (index, reason.to_owned()))
        })
        .collect::<Vec<_>>();
    let [(context_index, context_step)] = contexts.as_slice() else {
        return None;
    };
    let context_index = *context_index;
    let context_step = context_step.as_str();
    let [(park_index, park_reason)] = parks.as_slice() else {
        return None;
    };
    let park_index = *park_index;
    let park_reason = park_reason.as_str();
    if context_index >= park_index || park_reason != gap.park_reason.as_str() {
        return None;
    }

    let root_events = messages
        .iter()
        .enumerate()
        .filter_map(|(index, message)| {
            parse_root_step_trace(quest, message).map(|event| (index, event))
        })
        .collect::<Vec<_>>();
    if root_events.iter().any(|(index, event)| {
        *index < context_index
            && event.repeated
            && (event.event == "begin" || event.event.starts_with("failed:"))
    }) {
        return None;
    }

    let prior_steps = match gap.park_reason.as_str() {
        "wait exhausted" => {
            let target_stage = final_stage;
            let target_begin = root_events.iter().find(|entry| {
                entry.0 < context_index
                    && entry.1.stage == target_stage
                    && entry.1.step_id == gap.step_id
                    && entry.1.kind == "wait"
                    && entry.1.event == "begin"
                    && !entry.1.repeated
            })?;
            let target_failure = root_events
                .iter()
                .filter(|event| {
                    event.0 > target_begin.0
                        && event.0 < context_index
                        && event.1.stage == target_stage
                        && event.1.step_id == gap.step_id
                        && event.1.event == "failed: wait exhausted"
                        && !event.1.repeated
                })
                .collect::<Vec<_>>();
            if target_failure.len() != 1
                || root_events.iter().any(|(index, event)| {
                    *index < context_index
                        && event.stage == target_stage
                        && event.step_id == gap.step_id
                        && event.event == "settled"
                })
                || context_step != gap.step_id.as_str()
                || fields["step_id"].as_str() != Some(gap.step_id.as_str())
                || fields["last_failure"].as_str() != Some("wait exhausted")
                || target_failure[0].0 >= park_index
                || root_events.iter().any(|(index, event)| {
                    *index > target_begin.0
                        && *index < context_index
                        && !event.repeated
                        && !(event.stage == target_stage
                            && event.step_id == gap.step_id
                            && event.event == "failed: wait exhausted")
                })
            {
                return None;
            }
            prior_started_steps_settled(&root_events, target_begin.0)?
        }
        "no step for stage" => {
            let expected_stage = gap.stage.as_deref()?;
            if final_stage != expected_stage
                || context_step != "unknown"
                || fields["step_id"].as_str() != Some("")
            {
                return None;
            }
            let prior = prior_started_steps_settled(&root_events, context_index)?;
            let stage_events = root_events
                .iter()
                .filter(|event| {
                    event.0 < context_index && event.1.stage == expected_stage && !event.1.repeated
                })
                .collect::<Vec<_>>();
            let last_begin = stage_events
                .iter()
                .rev()
                .find(|(_, event)| event.event == "begin")?;
            let last_settled = stage_events
                .iter()
                .rev()
                .find(|(_, event)| event.event == "settled")?;
            if last_begin.1.step_id != gap.step_id
                || last_settled.1.step_id != gap.step_id
                || last_begin.1.kind != last_settled.1.kind
                || root_events
                    .iter()
                    .any(|(index, event)| *index > last_settled.0 && event.event == "begin")
                || !prior.iter().any(|step| {
                    step["stage"].as_str() == Some(expected_stage)
                        && step["step_id"].as_str() == Some(gap.step_id.as_str())
                        && step["settled"].as_bool() == Some(true)
                })
            {
                return None;
            }
            prior
        }
        _ => return None,
    };

    Some(serde_json::json!({
        "account": account,
        "stage": final_stage,
        "step_id": gap.step_id,
        "park_reason": gap.park_reason,
        "park_context_step": context_step,
        "park_trace": messages[park_index],
        "prior_steps_settled": true,
        "prior_steps": prior_steps,
        "step_observations": receipt["step_observations"],
        "final_status": final_status,
    }))
}

fn campaign_gap_proof(
    result: &Result<serde_json::Value, String>,
    gaps: &[Option<CampaignGap>],
    quest: &str,
    logs: &[serde_json::Value],
    receipts: &std::collections::BTreeMap<String, serde_json::Value>,
) -> Option<serde_json::Value> {
    let error = match result {
        Err(error) if error.contains("Quester parked/blocked:") => error,
        _ => return None,
    };
    let gap = gaps.first()?.as_ref()?;
    if gaps.iter().any(|candidate| candidate.as_ref() != Some(gap)) {
        return None;
    }
    let messages = campaign_trace_messages(quest, logs);
    if messages.len() != gaps.len() {
        return None;
    }
    let mut actors = Vec::with_capacity(gaps.len());
    for (account, account_messages) in messages {
        let receipt = receipts.get(&account)?;
        actors.push(prove_actor_gap(
            gap,
            quest,
            &account,
            &account_messages,
            receipt,
        )?);
    }
    Some(serde_json::json!({
        "step_id": gap.step_id,
        "stage": gap.stage,
        "park_reason": gap.park_reason,
        "prior_steps_settled": true,
        "accounts": actors,
        "runner_error": error,
    }))
}

#[test]
#[ignore = "QUEST-CAMPAIGN: exact authored seeds, Engine Q, staging folder and observed-tick deadline"]
fn live_quester_campaign() {
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"));
    assert_eq!(std::env::var("WORLD_GAME_PORT").as_deref(), Ok("44694"));
    assert!(
        std::env::var_os("QUESTER_TICK_MS").is_none(),
        "campaign must not change or restore the shared Engine Q tick speed"
    );
    assert_eq!(
        std::env::var("QUESTER_SPEED_RESTORE").as_deref(),
        Ok("0"),
        "campaign must explicitly disable restoration of the shared Engine Q speed"
    );
    assert!(std::env::var_os("QUEST_CAMPAIGN_TICKS").is_some());
    assert!(
        api::hostlog::install_sink(&CAMPAIGN_LOG_SINK),
        "campaign needs its own session log sink"
    );
    let manifest = PathBuf::from(
        std::env::var_os("QUEST_CAMPAIGN_MANIFEST").expect("QUEST_CAMPAIGN_MANIFEST"),
    );
    let campaign = campaign_cells(&manifest).unwrap_or_else(|error| panic!("{error}"));
    let quest = campaign[0].cell.quest;
    let (cells, gaps): (Vec<_>, Vec<_>) = campaign
        .into_iter()
        .map(|entry| (entry.cell, entry.gap))
        .unzip();
    let result = if cells.len() == 2 {
        let roles: [Cell; 2] = cells.try_into().unwrap_or_else(|_| unreachable!());
        quester_live::run_pair(quester_live::PairCell {
            roles,
            mode: quester_live::PairMode::Clean,
        })
    } else {
        quester_live::run(cells.into_iter().next().unwrap())
    };
    let root = PathBuf::from(std::env::var_os("LIVE_EVIDENCE_DIR").expect("LIVE_EVIDENCE_DIR"));
    let logs = std::mem::take(&mut *CAMPAIGN_LOGS.lock().unwrap());
    let mut file = std::fs::File::create(root.join("session.jsonl")).unwrap();
    use std::io::Write;
    for line in &logs {
        writeln!(file, "{line}").unwrap();
    }
    drop(file);

    let receipt_scan = campaign_actor_receipts(&root, quest);
    let gap_proof = receipt_scan
        .as_ref()
        .ok()
        .and_then(|receipts| campaign_gap_proof(&result, &gaps, quest, &logs, receipts));
    let mut outcome = match &result {
        Ok(receipt) => serde_json::json!({"state": "pass", "receipt": receipt}),
        Err(error) => serde_json::json!({"state": "failed", "error": error}),
    };
    if let Some(proof) = gap_proof {
        outcome["state"] = serde_json::json!("gap-reached");
        outcome["step_id"] = proof["step_id"].clone();
        outcome["stage"] = proof["stage"].clone();
        outcome["park_reason"] = proof["park_reason"].clone();
        outcome["prior_steps_settled"] = proof["prior_steps_settled"].clone();
        outcome["proof"] = proof;
    }
    if let Err(error) = receipt_scan {
        outcome["receipt_scan_error"] = serde_json::json!(error);
    }
    std::fs::write(
        root.join("campaign-outcome.json"),
        serde_json::to_vec_pretty(&outcome).unwrap(),
    )
    .unwrap();
    println!(
        "QUEST-CAMPAIGN state={} runner_passed={}",
        outcome["state"].as_str().unwrap_or("failed"),
        result.is_ok()
    );
    if outcome["state"] == "failed" {
        match result {
            Err(error) => panic!("QUEST-CAMPAIGN failed: {error}"),
            Ok(_) => panic!("QUEST-CAMPAIGN reported failure without a runner error"),
        }
    }
}
#[cfg(test)]
mod smoke_tests {
    use super::*;
    use std::sync::Mutex;

    static PATH_REGISTRY_LOCK: Mutex<()> = Mutex::new(());

    const COOK_SEEDS: &str = r#"{
        "stage": "cook:0",
        "stand": [3209, 3215, 0],
        "loadout": null,
        "extra_items": [],
        "seed_vars": [["cookquest", 0]],
        "before_relog": [],
        "expect": ["cook:1"],
        "mode": "stage"
    }"#;

    const COG_CLEAN_SEEDS: &str = r#"{
        "stage": "cog:0",
        "stand": [2569, 3249, 0],
        "loadout": null,
        "extra_items": [],
        "seed_vars": [],
        "before_relog": [],
        "expect": [],
        "mode": "clean"
    }"#;

    fn parse_seeds(text: &str) -> Result<QuesterPathSeeds, String> {
        scenario::quester::parse_quester_path_seeds(text, "QUESTER_SEEDS")
    }

    fn cook_cell(seeds: &QuesterPathSeeds) -> Cell {
        let row = smoke_row("cook").unwrap();
        assert_eq!(row.display, "Cook's Assistant");
        let loadout = seeds.fixture_loadout().unwrap();
        let extra_items = seeds.extra_item_refs();
        let seed_vars = seeds.seed_var_refs();
        smoke_cell(row, seeds, loadout, &extra_items, &seed_vars, false).unwrap()
    }

    #[test]
    fn smoke_seeds_parse_and_build_the_embedded_cook_cell() {
        let _registry_guard = PATH_REGISTRY_LOCK.lock().unwrap();
        let seeds = parse_seeds(COOK_SEEDS).unwrap();
        assert_eq!(seeds.stage, "cook:0");
        let cell = cook_cell(&seeds);
        assert_eq!(cell.quest, "cook");
        assert_eq!(cell.scenario.settings.deadline, DEADLINE);
        assert!(cell.scenario.settings.start_script == Some("Quester"));
        let names: Vec<&str> = cell.scenario.steps.iter().map(|step| step.name).collect();
        let relog = cell
            .scenario
            .steps
            .iter()
            .rposition(|step| matches!(step.kind, scenario::StepKind::Relog))
            .unwrap();
        let stand = names
            .iter()
            .position(|name| *name == "stand at the authored sequence")
            .unwrap();
        assert!(relog < stand, "cook stand follows the relog");
        assert!(cell
            .scenario
            .steps
            .iter()
            .any(|step| step.name == "reset authored content stage"));
    }

    #[test]
    fn clean_smoke_does_not_seed_target_progress() {
        let _registry_guard = PATH_REGISTRY_LOCK.lock().unwrap();
        let seeds = parse_seeds(COG_CLEAN_SEEDS).unwrap();
        let row = smoke_row("cog").unwrap();
        let loadout = seeds.fixture_loadout().unwrap();
        let extra_items = seeds.extra_item_refs();
        let seed_vars = seeds.seed_var_refs();
        let cell = smoke_cell(row, &seeds, loadout, &extra_items, &seed_vars, false).unwrap();
        assert!(matches!(cell.mode, Mode::Clean));
        assert!(!cell.start_settings.contains_key("allow_danger_zones"));
        assert!(!cell
            .scenario
            .steps
            .iter()
            .any(|step| step.name == "reset authored content stage"));
    }

    #[test]
    fn danger_route_permission_is_opt_in() {
        let _registry_guard = PATH_REGISTRY_LOCK.lock().unwrap();
        let seeds = parse_seeds(COG_CLEAN_SEEDS).unwrap();
        let row = smoke_row("cog").unwrap();
        let extra_items = seeds.extra_item_refs();
        let seed_vars = seeds.seed_var_refs();
        let default = smoke_cell(
            row,
            &seeds,
            seeds.fixture_loadout().unwrap(),
            &extra_items,
            &seed_vars,
            false,
        )
        .unwrap();
        assert!(!default.start_settings.contains_key("allow_danger_zones"));
        let allowed = smoke_cell(
            row,
            &seeds,
            seeds.fixture_loadout().unwrap(),
            &extra_items,
            &seed_vars,
            true,
        )
        .unwrap();
        assert_eq!(
            allowed.start_settings.get("allow_danger_zones"),
            Some(&serde_json::Value::Bool(true))
        );
    }

    #[test]
    fn smoke_row_rejects_unknown_quests_and_non_base40_profiles() {
        assert!(smoke_row("not-a-quest").is_err());
        assert!(smoke_row("dragon").is_err());
        assert_eq!(smoke_row("cog").unwrap().profile, TestProfile::Base40);
        assert_eq!(smoke_row("cook").unwrap().display, "Cook's Assistant");
    }

    #[test]
    fn smoke_seeds_reject_an_unknown_mode_kit_or_shape() {
        let _registry_guard = PATH_REGISTRY_LOCK.lock().unwrap();
        assert!(parse_seeds(r#"{"stage": "cook:0", "stand": [0, 0]}"#).is_err());
        let mut seeds = parse_seeds(COOK_SEEDS).unwrap();
        seeds.mode = Some("speedrun".into());
        assert!(seeds.fixture_loadout().is_ok());
        let row = smoke_row("cook").unwrap();
        let empty: Vec<(&str, i32)> = Vec::new();
        assert!(smoke_cell(row, &seeds, None, &empty, &empty, false).is_err());
        let seeds = parse_seeds(
            &COOK_SEEDS.replace(r#""loadout": null"#, r#""loadout": {"standard": "plate"}"#),
        )
        .unwrap();
        assert!(seeds.fixture_loadout().is_err());
    }

    #[test]
    fn smoke_cell_serves_a_folder_cook_through_the_registry() {
        let _registry_guard = PATH_REGISTRY_LOCK.lock().unwrap();
        let seeds = parse_seeds(COOK_SEEDS).unwrap();
        // Baseline from the embedded index, before any folder is published.
        let embedded_stats = cook_cell(&seeds)
            .scenario
            .steps
            .iter()
            .filter(|step| step.name == "stage qualification skill")
            .count();
        let root = std::env::temp_dir().join(format!(
            "274bot-path-smoke-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        // Folder-only marker: the embedded Cook carries no skill
        // requirements, so one extra crafting gate must survive into the
        // cell as exactly one more "stage qualification skill" step. An
        // embedded-only read would still build the baseline cell and fail
        // the count below.
        let mut folder_cook: serde_json::Value =
            serde_json::from_slice(script::quester::compile::path_bytes("cook").unwrap()).unwrap();
        folder_cook["quest"]["requirements"] = serde_json::json!([{
            "id": "crafting",
            "kind": {"Skill": {"skill": "crafting", "level": 31}},
            "at": "Start",
            "source": "folder smoke marker",
        }]);
        std::fs::write(
            root.join("cook.json"),
            serde_json::to_vec(&folder_cook).unwrap(),
        )
        .unwrap();
        let before = registry::source();
        // Restore the previous source on every exit; the shared registry is
        // process-global and another test may read its bundled snapshot.
        struct SourceGuard(Option<registry::FolderSource>);
        impl Drop for SourceGuard {
            fn drop(&mut self) {
                if let Some(source) = self.0.take() {
                    registry::set_source(source);
                }
            }
        }
        let _guard = SourceGuard(Some(before));
        assert_eq!(
            scenario::quester::reload_quester_path_source("cook", Some(&root)).unwrap(),
            "folder"
        );
        let (path, source) = scenario::quester::load_quester_path_document("cook").unwrap();
        assert_eq!(source, "folder");
        assert_eq!(path.id.0.as_ref(), "cook");
        // Build the cell while the folder source is still the snapshot
        // `path_cell` reads; restoring first would test the embedded cell.
        assert_eq!(
            registry::snapshot().path_source("cook").label(),
            "folder",
            "the folder source must stay published across the cell build"
        );
        let cell = cook_cell(&seeds);
        assert_eq!(cell.quest, "cook");
        let folder_stats = cell
            .scenario
            .steps
            .iter()
            .filter(|step| step.name == "stage qualification skill")
            .count();
        assert_eq!(
            folder_stats,
            embedded_stats + 1,
            "the folder-only crafting gate must survive into the cell"
        );
        std::fs::remove_dir_all(&root).ok();
    }
}

#[test]
fn campaign_identity_does_not_require_a_qualification_profile() {
    let catalog = campaign_catalog().unwrap();
    assert_eq!(
        catalog.quest("scorpcatcher").unwrap().display.as_ref(),
        "Scorpion Catcher"
    );
    assert_eq!(
        catalog.quest("priest").unwrap().display.as_ref(),
        "The Restless Ghost"
    );
    assert!(catalog.quest("not-a-quest").is_err());
}

#[cfg(test)]
mod campaign_gap_tests {
    use super::*;

    const QUEST: &str = "viking";
    const ACCOUNT: &str = "qh12345678_0";
    const STAGE: &str = "viking:8";
    const GAP_STEP: &str = "GAP-peer-riddle-ui";
    const PRIOR_STEP: &str = "walk-peer-door";

    fn log(message: &str) -> serde_json::Value {
        serde_json::json!({"account": ACCOUNT, "message": message})
    }

    fn wait_gap() -> CampaignGap {
        CampaignGap {
            step_id: GAP_STEP.into(),
            park_reason: "wait exhausted".into(),
            stage: Some(STAGE.into()),
        }
    }

    fn actor_receipt(step_id: &str, reason: &str, stage: &str) -> serde_json::Value {
        serde_json::json!({
            "account": ACCOUNT,
            "quest": QUEST,
            "quest_complete": false,
            "deaths": 0,
            "death_count_total_across_runs": 0,
            "death_sent": false,
            "death_injections": 0,
            "death_observed": false,
            "error": null,
            "capture_error": null,
            "end_captured": true,
            "step_observations": [{
                "run": "RunKey { slot: 1, run: 1, session: 1 }",
                "tick": 42,
                "stage": stage,
                "sequence": 0,
                "step_index": 1,
                "remaining_steps": 1,
                "step_id": step_id,
                "child_step_id": "",
                "action_state": "blocked",
                "waiting_for": null,
                "owned_combat": null,
                "inventory": [],
            }],
            "final_status": {
                "phase": "Blocked",
                "fields": {
                    "quest_id": QUEST,
                    "step_id": step_id,
                    "action_state": "blocked",
                    "block_reason": reason,
                    "last_failure": reason,
                    "quest": {"stage": stage, "rule": null, "complete": "Unknown"},
                },
                "failure": "Some(parked)"
            }
        })
    }

    fn wait_trace() -> Vec<serde_json::Value> {
        vec![
            log("quester viking: run start stage=viking:0"),
            log("quester viking: stage viking:0 step walk-peer-door (walk) begin"),
            log("quester viking: stage viking:0 step walk-peer-door (walk) settled"),
            log("quester viking: stage viking:8 step GAP-peer-riddle-ui (wait) begin"),
            log("quester viking: stage viking:8 step GAP-peer-riddle-ui (wait) failed: wait exhausted"),
            log("quester viking: park context step=GAP-peer-riddle-ui child_recipe= child="),
            log("quester viking: park: wait exhausted"),
        ]
    }

    fn receipts(actor: serde_json::Value) -> std::collections::BTreeMap<String, serde_json::Value> {
        std::collections::BTreeMap::from([(ACCOUNT.to_owned(), actor)])
    }

    fn error_result() -> Result<serde_json::Value, String> {
        Err(format!("{ACCOUNT} Quester parked/blocked: phase Blocked"))
    }

    #[test]
    fn gap_declaration_is_optional_and_limited_to_native_park_reasons() {
        assert!(campaign_gap(&serde_json::json!({})).unwrap().is_none());
        let wait = serde_json::json!({
            "gap": {"step_id": GAP_STEP, "park_reason": "wait exhausted"}
        });
        assert_eq!(
            campaign_gap(&wait).unwrap(),
            Some(CampaignGap {
                stage: None,
                ..wait_gap()
            })
        );
        assert!(campaign_gap(&serde_json::json!({
            "gap": {"step_id": GAP_STEP, "park_reason": "no step for stage"}
        }))
        .is_err());
        assert!(campaign_gap(&serde_json::json!({
            "gap": {"step_id": GAP_STEP, "park_reason": "unrelated failure"}
        }))
        .is_err());
    }

    #[test]
    fn gap_requires_exact_declared_step_reason_and_settled_prior_steps() {
        let gap = wait_gap();
        let logs = wait_trace();
        let actor = actor_receipt(GAP_STEP, "wait exhausted", STAGE);
        let proof = campaign_gap_proof(
            &error_result(),
            &[Some(gap)],
            QUEST,
            &logs,
            &receipts(actor),
        )
        .expect("a declared timeout gap with settled prior steps should be proven");
        assert_eq!(proof["step_id"], GAP_STEP);
        assert_eq!(proof["park_reason"], "wait exhausted");
        assert!(proof["prior_steps_settled"].as_bool().unwrap());
        assert_eq!(
            proof["accounts"][0]["prior_steps"][0]["step_id"],
            PRIOR_STEP
        );
        assert!(proof["accounts"][0]["prior_steps"][0]["settled"]
            .as_bool()
            .unwrap());
    }

    #[test]
    fn gap_rejects_a_wrong_step() {
        let gap = CampaignGap {
            step_id: "GAP-some-other-step".into(),
            ..wait_gap()
        };
        assert!(campaign_gap_proof(
            &error_result(),
            &[Some(gap)],
            QUEST,
            &wait_trace(),
            &receipts(actor_receipt(GAP_STEP, "wait exhausted", STAGE)),
        )
        .is_none());
    }

    #[test]
    fn gap_rejects_a_wrong_reason() {
        let mut logs = wait_trace();
        logs[6] = log("quester viking: park: no step for stage");
        assert!(campaign_gap_proof(
            &error_result(),
            &[Some(wait_gap())],
            QUEST,
            &logs,
            &receipts(actor_receipt(GAP_STEP, "wait exhausted", STAGE)),
        )
        .is_none());
    }

    #[test]
    fn gap_rejects_an_unsettled_earlier_step() {
        let mut logs = wait_trace();
        logs.remove(2);
        assert!(campaign_gap_proof(
            &error_result(),
            &[Some(wait_gap())],
            QUEST,
            &logs,
            &receipts(actor_receipt(GAP_STEP, "wait exhausted", STAGE)),
        )
        .is_none());
    }

    #[test]
    fn complete_guide_failure_does_not_become_a_gap_without_declaration() {
        assert!(campaign_gap_proof(
            &error_result(),
            &[None],
            QUEST,
            &wait_trace(),
            &receipts(actor_receipt(GAP_STEP, "wait exhausted", STAGE)),
        )
        .is_none());
    }

    #[test]
    fn gap_rejects_death_and_unrelated_failures() {
        let mut actor = actor_receipt(GAP_STEP, "wait exhausted", STAGE);
        actor["deaths"] = serde_json::json!(1);
        assert!(campaign_gap_proof(
            &error_result(),
            &[Some(wait_gap())],
            QUEST,
            &wait_trace(),
            &receipts(actor),
        )
        .is_none());
        assert!(campaign_gap_proof(
            &Err("unrelated failure".into()),
            &[Some(wait_gap())],
            QUEST,
            &wait_trace(),
            &receipts(actor_receipt(GAP_STEP, "wait exhausted", STAGE)),
        )
        .is_none());
    }

    #[test]
    fn no_step_gap_requires_stage_and_a_settled_boundary_anchor() {
        let gap = CampaignGap {
            step_id: PRIOR_STEP.into(),
            park_reason: "no step for stage".into(),
            stage: Some(STAGE.into()),
        };
        // Later conditional guide steps are skipped, so they leave no root trace.
        let logs = vec![
            log("quester viking: run start stage=viking:8"),
            log("quester viking: stage viking:8 step walk-peer-door (walk) begin"),
            log("quester viking: stage viking:8 step walk-peer-door (walk) settled"),
            log("quester viking: park context step=unknown child_recipe= child="),
            log("quester viking: park: no step for stage"),
        ];
        let proof = campaign_gap_proof(
            &error_result(),
            &[Some(gap)],
            QUEST,
            &logs,
            &receipts(actor_receipt("", "no step for stage", STAGE)),
        )
        .expect("no-step gap should prove its exact stage boundary");
        assert_eq!(proof["step_id"], PRIOR_STEP);
        assert_eq!(proof["stage"], STAGE);
        assert!(proof["prior_steps_settled"].as_bool().unwrap());
        assert_eq!(proof["accounts"][0]["park_context_step"], "unknown");
        assert_eq!(
            proof["accounts"][0]["prior_steps"][0]["step_id"],
            PRIOR_STEP
        );
    }

    #[test]
    fn no_step_gap_rejects_a_later_started_root_step() {
        let gap = CampaignGap {
            step_id: PRIOR_STEP.into(),
            park_reason: "no step for stage".into(),
            stage: Some(STAGE.into()),
        };
        let logs = vec![
            log("quester viking: run start stage=viking:8"),
            log("quester viking: stage viking:8 step walk-peer-door (walk) begin"),
            log("quester viking: stage viking:8 step walk-peer-door (walk) settled"),
            log("quester viking: stage viking:8 step later-step (perform) begin"),
            log("quester viking: stage viking:8 step later-step (perform) settled"),
            log("quester viking: park context step=unknown child_recipe= child="),
            log("quester viking: park: no step for stage"),
        ];
        assert!(campaign_gap_proof(
            &error_result(),
            &[Some(gap)],
            QUEST,
            &logs,
            &receipts(actor_receipt("", "no step for stage", STAGE)),
        )
        .is_none());
    }

    #[test]
    fn no_step_gap_does_not_accept_unknown_context_without_its_anchor_trace() {
        let gap = CampaignGap {
            step_id: PRIOR_STEP.into(),
            park_reason: "no step for stage".into(),
            stage: Some(STAGE.into()),
        };
        let logs = vec![
            log("quester viking: run start stage=viking:8"),
            log("quester viking: park context step=unknown child_recipe= child="),
            log("quester viking: park: no step for stage"),
        ];
        assert!(campaign_gap_proof(
            &error_result(),
            &[Some(gap)],
            QUEST,
            &logs,
            &receipts(actor_receipt("", "no step for stage", STAGE)),
        )
        .is_none());
    }
}
