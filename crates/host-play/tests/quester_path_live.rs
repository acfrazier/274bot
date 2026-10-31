//! Generic Path live smoke: one qualification-profile cell for a content quest id.
//!
//! This is the permanent form of the PATHS-OVERNIGHT throwaway runner. It
//! runs the Path id in `QUESTER_PATH` from the embedded release index, or
//! from `QUESTER_PATH_DIR` through the existing `FolderSource` registry.
//! Optional account seeds come from the `QUESTER_SEEDS` JSON file.
//! Stage mode checks the expected stage after Start. Clean mode checks terminal
//! completion without seeding the target Path's stage.
//! Path-terminal mode checks that the runner completes without completing the
//! quest.
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
//!   QUESTER_SUSTAIN_RUN=1 QUESTER_SEEDS=<evidence-root>/seeds/cook.json \
//!   cargo test \
//!     -p host-play --features "live-harness test-support" \
//!     --test quester_path_live live_quester_path_smoke -- --ignored --nocapture --test-threads=1
//! ```
//! `QUESTER_TICK_MS` changes world-wide tick speed. Leave it unset for this
//! live check. The host receipt records the observed tick mean.
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
//! The required `stage` field seeds the account in `stage` and `path_terminal`
//! modes. Clean mode ignores it and starts from clean quest state.
//!
//! For a focused terminal Path probe, set `mode` to `path_terminal`. The live
//! cell passes when the script ends, even if the quest remains in progress.
//!
//! For campaign seed plans, the optional `gap` object names a partial-guide
//! boundary: `{"step_id":"GAP-peer-riddle-ui","park_reason":"wait exhausted"}`.
//! For a `no step for stage` gap it also names the boundary `stage`.
//! Campaign parks require the published native Blocked phase. No-step gaps
//! also check uncapped status transitions and the authored boundary predicate
//! against the final CPU capture, so the bounded text trace is not sole proof.
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
use scenario::quester::{
    FixtureLoadout, QuestFixtureProfile, QuesterPathSeeds, QuesterStage, TestProfile,
};
use script::quester::registry;
use std::path::PathBuf;
use std::time::Duration;

/// Fixed whole-cell deadline (fixture seed plus quest). Not env-configurable:
/// a smoke that cannot Start and settle inside it fails instead of burning
/// the shared engine. 45 minutes covers the long Paths (Prince Ali Rescue
/// ran about 31 minutes overnight).
const DEADLINE: Duration = Duration::from_secs(2700);

/// Resolve the harness row and use the qualification profile it declares.
/// This also allows Base60 Paths such as Elemental Workshop.
fn smoke_row(quest: &str) -> Result<QuestFixtureProfile, String> {
    scenario::quester::fixture_row(quest)
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
) -> Result<Cell, String> {
    let (mode, stage) = match seeds.mode.as_deref().unwrap_or("stage") {
        "stage" => (
            Mode::Stage {
                expect: seeds.expect.clone(),
            },
            Some(seeds.stage.as_str()),
        ),
        "path_terminal" => (Mode::PathTerminal, Some(seeds.stage.as_str())),
        "clean" => (Mode::Clean, None),
        other => return Err(format!("unknown smoke mode {other:?}")),
    };
    let spec = PathCell {
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
    };
    let mut cell = if row.quest == "blackarmgang" {
        blackarmgang_role_smoke_cell(spec)?
    } else {
        quester_live::path_cell(spec)?
    };
    cell.scenario.settings.deadline = DEADLINE;
    Ok(cell)
}

/// The early Phoenix Path stage is single-account even though the complete
/// Shield of Arrav Path declares a partner role.
fn blackarmgang_role_smoke_cell(spec: PathCell<'_>) -> Result<Cell, String> {
    let stage = spec
        .stage
        .ok_or("blackarmgang smoke needs an explicit role stage")?;
    let (path, _) = scenario::quester::load_quester_path_document(spec.quest)?;
    let selected = api::game_data::for_revision(api::selected::ClientRevision::R289)?;
    let identity = selected
        .quest_identity()
        .and_then(|facts| facts.rows.iter().find(|identity| identity.id == spec.quest))
        .ok_or_else(|| format!("no quest identity row for {}", spec.quest))?;
    let mut fixture = scenario::quester::quester_role_stage(
        QuesterStage {
            name: spec.quest,
            quest_display: spec.display,
            path: &path,
            identity,
            selected: &selected,
            stage,
            loadout: spec.loadout,
            extra_items: spec.extra_items,
            stand: spec.stand,
        },
        script::quester::pair::Gang::Phoenix,
        &identity.varp,
        spec.seed_vars,
    )?;
    if !spec.before_relog.is_empty() {
        let relog = fixture
            .scenario
            .steps
            .iter()
            .rposition(|step| matches!(step.kind, scenario::StepKind::Relog))
            .ok_or("blackarmgang role fixture has no final relog")?;
        fixture
            .scenario
            .steps
            .splice(relog..relog, spec.before_relog);
    }
    let seed = fixture.seed;
    Ok(Cell {
        quest: spec.quest,
        display: spec.display,
        label: spec.label,
        scenario: fixture.scenario,
        start_settings: fixture.start_settings,
        mode: spec.mode,
        observe_start: Some(Box::new(move |snapshot| seed.observe_start(snapshot))),
    })
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
    let cell = smoke_cell(row, &seeds, loadout, &extra_items, &seed_vars)
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
                    "quest_points_varp": plan["qp_varp_id"],
                    "quest_points": plan["qp_varp_id"].as_i64()
                        .and_then(|index| i32::try_from(index).ok())
                        .and_then(|index| snapshot.varps().iter().find(|row| row.index == index))
                        .map(|row| row.value),
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
    occurrences: u32,
}

fn parse_root_step_trace(quest: &str, message: &str) -> Option<RootStepTrace> {
    let prefix = format!("quester {quest}: stage ");
    let (stage, detail) = message.strip_prefix(&prefix)?.split_once(" step ")?;
    let (label, event) = detail.rsplit_once(") ")?;
    let (step_id, kind) = label.rsplit_once(" (")?;
    if stage.is_empty() || step_id.is_empty() || kind.is_empty() {
        return None;
    }
    let repetition = event.rsplit_once(" (repeated ").and_then(|(base, count)| {
        let count = count
            .strip_suffix(" additional times)")?
            .parse::<u32>()
            .ok()
            .filter(|count| *count > 0)?;
        Some((base, count))
    });
    let (event, occurrences, repeated) =
        repetition.map_or((event, 1, false), |(base, count)| (base, count, true));
    if event != "begin" && event != "settled" && !event.starts_with("failed:") {
        return None;
    }
    Some(RootStepTrace {
        stage: stage.to_owned(),
        step_id: step_id.to_owned(),
        kind: kind.to_owned(),
        event: event.to_owned(),
        repeated,
        occurrences,
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
    if !receipts.is_empty() {
        let (document, _) = scenario::quester::load_quester_path_document(quest)?;
        for actor in receipts.values_mut() {
            let role = document
                .roles
                .iter()
                .find(|role| {
                    role.role.as_ref().map(|role| role.0.as_ref()) == actor["role"].as_str()
                })
                .ok_or("gap evidence has no matching authored role")?;
            let steps = |steps: &[script::quester::path::StepDocument]| {
                steps
                    .iter()
                    .map(|step| {
                        serde_json::json!({
                            "id": step.id, "kind": step.kind,
                            "settle": step.settle, "skip_if": step.skip_if,
                            "max_attempts": step.max_attempts,
                        })
                    })
                    .collect::<Vec<_>>()
            };
            let status_path = actor["status_log"]
                .as_str()
                .ok_or("gap receipt has no status log")?;
            let statuses = std::fs::read_to_string(status_path)
                .map_err(|error| format!("{status_path}: {error}"))?
                .lines()
                .map(serde_json::from_str::<serde_json::Value>)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| format!("{status_path}: {error}"))?;
            let capture_path = std::path::Path::new(
                actor["captures"]
                    .as_array()
                    .and_then(|captures| captures.last())
                    .and_then(serde_json::Value::as_str)
                    .ok_or("gap receipt has no final capture")?,
            )
            .with_extension("json");
            let end_state: serde_json::Value = serde_json::from_slice(
                &std::fs::read(&capture_path)
                    .map_err(|error| format!("{}: {error}", capture_path.display()))?,
            )
            .map_err(|error| format!("{}: {error}", capture_path.display()))?;
            // Keep raw receipts immutable. These separately sourced witnesses
            // establish no-step settlements and counted content misses.
            actor["gap_state_evidence"] = serde_json::json!({
                "status_log": status_path,
                "end_capture": capture_path,
                "statuses": statuses,
                "end_state": end_state,
                "guide": {
                    "quest": document.id,
                    "areas": document.quest.as_ref().map(|quest| &quest.areas),
                    "prelude": steps(&role.prelude),
                    "sequences": role.sequences.iter().map(|sequence| serde_json::json!({
                        "stage": sequence.stage, "steps": steps(&sequence.steps),
                    })).collect::<Vec<_>>(),
                },
            });
        }
    }
    Ok(receipts)
}

#[derive(serde::Serialize)]
struct CountedContentMiss<'a> {
    stage: &'a str,
    step_id: &'a str,
    kind: &'a str,
    attempt: u64,
    max_attempts: u64,
    tick: u64,
}

fn counted_content_misses<'a>(
    quest: &str,
    receipt: &'a serde_json::Value,
) -> Option<Vec<CountedContentMiss<'a>>> {
    let Some(evidence) = receipt.get("gap_state_evidence") else {
        return Some(Vec::new());
    };
    let guide = &evidence["guide"];
    let end = &evidence["end_state"];
    let final_run = receipt["final_status"]["run"].as_str()?;
    if guide["quest"].as_str() != Some(quest)
        || end["account"] != receipt["account"]
        || end["label"].as_str() != Some("end-fail")
        || end["status"] != receipt["final_status"]
    {
        return None;
    }
    let mut misses = Vec::new();
    let mut counts = std::collections::BTreeMap::new();
    let mut last_tick = 0;
    for row in evidence["statuses"].as_array()? {
        let tick = row["tick"].as_u64()?;
        if tick < last_tick || tick > end["tick"].as_u64()? {
            return None;
        }
        last_tick = tick;
        let status = &row["status"];
        let fields = &status["fields"];
        if fields["quest_id"].as_str() != Some(quest) {
            continue;
        }
        if status["run"].as_str() != Some(final_run) {
            return None;
        }
        if fields["step_attempt_outcome"].as_str() != Some("content_miss") {
            continue;
        }
        let stage = fields["stage"].as_str()?;
        let step_id = fields["step_id"].as_str()?;
        let mut authored = guide["sequences"]
            .as_array()?
            .iter()
            .filter(|sequence| sequence["stage"].as_str() == Some(stage))
            .filter_map(|sequence| sequence["steps"].as_array())
            .flatten()
            .chain(guide["prelude"].as_array()?.iter())
            .filter(|step| step["id"].as_str() == Some(step_id));
        let step = authored.next()?;
        if authored.next().is_some() {
            return None;
        }
        let max_attempts = step["max_attempts"].as_u64()?;
        let attempt = fields["step_attempts"].as_u64()?;
        if attempt == 0 || attempt > max_attempts {
            return None;
        }
        // A fresh native step scope starts at zero. Repeated publications of
        // one receipt are not additional attempts; every new count must be +1.
        let previous = counts.entry((stage, step_id)).or_insert(0_u64);
        if attempt == *previous {
            continue;
        }
        if attempt != previous.checked_add(1)? {
            return None;
        }
        *previous = attempt;
        misses.push(CountedContentMiss {
            stage,
            step_id,
            kind: step["kind"].as_str()?,
            attempt,
            max_attempts,
            tick,
        });
    }
    Some(misses)
}

fn counted_miss_status(
    fields: &serde_json::Value,
    tick: u64,
    misses: &[CountedContentMiss<'_>],
) -> bool {
    fields["step_attempt_outcome"].as_str() == Some("content_miss")
        && misses.iter().any(|miss| {
            fields["stage"].as_str() == Some(miss.stage)
                && fields["step_id"].as_str() == Some(miss.step_id)
                && fields["step_attempts"].as_u64() == Some(miss.attempt)
                && tick >= miss.tick
        })
}

struct StepSettlement<'a> {
    begin: &'a RootStepTrace,
    settled: bool,
    begins: u64,
    failures: u64,
}

fn prior_started_steps_settled(
    events: &[(usize, RootStepTrace)],
    boundary: usize,
    status_steps: &[serde_json::Value],
    misses: &[CountedContentMiss<'_>],
) -> Option<Vec<serde_json::Value>> {
    let mut steps = std::collections::BTreeMap::<(&str, &str), StepSettlement<'_>>::new();
    for (index, event) in events {
        let key = (event.stage.as_str(), event.step_id.as_str());
        // Native repeat summaries flush at the park, after a wait gap began.
        // They can only extend an original step observed before the boundary.
        if *index >= boundary && (!event.repeated || !steps.contains_key(&key)) {
            continue;
        }
        match event.event.as_str() {
            "begin" => {
                if let Some(step) = steps.get_mut(&key) {
                    if step.begin.kind != event.kind
                        || (!event.repeated && (step.settled || step.begins != step.failures))
                    {
                        return None;
                    }
                    step.begins = step.begins.checked_add(u64::from(event.occurrences))?;
                } else {
                    if event.repeated {
                        return None;
                    }
                    steps.insert(
                        key,
                        StepSettlement {
                            begin: event,
                            settled: false,
                            begins: 1,
                            failures: 0,
                        },
                    );
                }
            }
            "failed: content miss" => {
                let step = steps.get_mut(&key)?;
                if step.begin.kind != event.kind
                    || (event.repeated && step.failures == 0)
                    || (!event.repeated
                        && (step.settled || step.begins != step.failures.checked_add(1)?))
                {
                    return None;
                }
                step.failures = step.failures.checked_add(u64::from(event.occurrences))?;
            }
            outcome if outcome.starts_with("failed:") => return None,
            "settled" => {
                // Progress can advance the journal stage before this event.
                let mut pending = steps.iter().filter(|(_, step)| {
                    !step.settled
                        && step.begin.step_id == event.step_id
                        && step.begin.kind == event.kind
                });
                let key = if let Some((key, _)) = pending.next() {
                    if pending.next().is_some() {
                        return None;
                    }
                    *key
                } else if event.repeated
                    && steps.values().any(|step| {
                        step.settled
                            && step.begin.step_id == event.step_id
                            && step.begin.kind == event.kind
                    })
                {
                    continue;
                } else {
                    return None;
                };
                steps.get_mut(&key)?.settled = true;
            }
            _ => {}
        }
    }
    for step in steps.values_mut() {
        let matching = misses.iter().filter(|miss| {
            miss.stage == step.begin.stage
                && miss.step_id == step.begin.step_id
                && miss.kind == step.begin.kind
        });
        let mut total = 0;
        for miss in matching {
            total += 1;
            if step.begins > miss.max_attempts {
                return None;
            }
        }
        if total != step.failures || step.begins != step.failures.checked_add(1)? {
            return None;
        }
        if !step.settled {
            if !status_steps.iter().any(|status| {
                status["stage"].as_str() == Some(step.begin.stage.as_str())
                    && status["step_id"].as_str() == Some(step.begin.step_id.as_str())
                    && status["kind"].as_str() == Some(step.begin.kind.as_str())
                    && status["settled"].as_bool() == Some(true)
            }) {
                return None;
            }
            step.settled = true;
        }
    }
    if misses.iter().any(|miss| {
        !steps
            .get(&(miss.stage, miss.step_id))
            .is_some_and(|step| step.begin.kind == miss.kind && miss.attempt <= step.failures)
    }) {
        return None;
    }
    Some(
        steps
            .into_values()
            .map(|step| {
                serde_json::json!({
                    "stage": step.begin.stage,
                    "step_id": step.begin.step_id,
                    "kind": step.begin.kind,
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
/// Check stable facts actually present in the CPU capture, not arbitrary Path
/// predicates. Missing observations, fresh-chat predicates and unsupported
/// facts stay Unknown. Empty predicates cannot establish a boundary.
fn captured_gap_fact(
    predicate: &serde_json::Value,
    state: &serde_json::Value,
    guide: &serde_json::Value,
    selected: &api::game_data::SelectedGameData,
) -> api::selected::Truth {
    use api::selected::Truth;
    for (key, all) in [("All", true), ("Any", false)] {
        if let Some(items) = predicate[key].as_array() {
            if items.is_empty() {
                return Truth::Unknown;
            }
            let truths = items
                .iter()
                .map(|item| captured_gap_fact(item, state, guide, selected));
            return if all {
                Truth::all(truths)
            } else {
                Truth::any(truths)
            };
        }
    }
    if let Some(inner) = predicate.get("Not") {
        return !captured_gap_fact(inner, state, guide, selected);
    }
    let fact = &predicate["Fact"];
    if fact["version"].as_u64() != Some(1) {
        return Truth::Unknown;
    }
    let args = &fact["args"];
    let answer = (|| match fact["kind"].as_str()? {
        "has_item" | "item_count_at_least" => {
            let id = selected.item_by_alias(args["obj"].as_str()?)?.id;
            let count = state["inventory"]
                .as_array()?
                .iter()
                .try_fold(0i64, |sum, row| {
                    Some(sum.saturating_add(if row[0].as_i64()? == i64::from(id) {
                        row[1].as_i64()?
                    } else {
                        0
                    }))
                })?;
            let minimum = if fact["kind"] == "has_item" {
                1
            } else {
                args["qty"].as_i64()?
            };
            Some(minimum > 0 && count >= minimum)
        }
        "worn" => {
            let id = selected.item_by_alias(args["obj"].as_str()?)?.id;
            state["equipment"]
                .as_array()?
                .iter()
                .try_fold(false, |worn, item| {
                    let observed = item.as_i64()?;
                    Some(worn || observed == i64::from(id))
                })
        }
        "near" => {
            let [hx, hz, hl] = state["tile"].as_array()?.as_slice() else {
                return None;
            };
            let [tx, tz, tl] = args["tile"].as_array()?.as_slice() else {
                return None;
            };
            let radius = args["radius"].as_i64()?;
            Some(
                radius >= 0
                    && hl.as_i64()? == tl.as_i64()?
                    && hx.as_i64()?.abs_diff(tx.as_i64()?) <= radius as u64
                    && hz.as_i64()?.abs_diff(tz.as_i64()?) <= radius as u64,
            )
        }
        "in_area" => {
            let [x, z, level] = state["tile"].as_array()?.as_slice() else {
                return None;
            };
            let (x, z, level) = (x.as_i64()?, z.as_i64()?, level.as_i64()?);
            let boxes = guide["areas"][args["area"].as_str()?]["boxes"].as_array()?;
            let mut inside = false;
            for bounds in boxes {
                inside |= x >= bounds[0].as_i64()?
                    && x <= bounds[2].as_i64()?
                    && z >= bounds[1].as_i64()?
                    && z <= bounds[3].as_i64()?
                    && level == bounds[4].as_i64()?;
            }
            Some(inside)
        }
        "stage_in" if args["quest"] == guide["quest"] => {
            let stage = state["status"]["fields"]["quest"]["stage"]
                .as_str()
                .or_else(|| state["status"]["fields"]["stage"].as_str())?;
            Some(
                args["any"]
                    .as_array()?
                    .iter()
                    .any(|candidate| candidate.as_str() == Some(stage)),
            )
        }
        _ => None,
    })();
    match answer {
        Some(true) => Truth::True,
        Some(false) => Truth::False,
        None => Truth::Unknown,
    }
}

fn no_step_state_proof(
    gap: &CampaignGap,
    quest: &str,
    account: &str,
    receipt: &serde_json::Value,
    misses: &[CountedContentMiss<'_>],
) -> Option<serde_json::Value> {
    let evidence = receipt.get("gap_state_evidence")?;
    let guide = &evidence["guide"];
    let end = &evidence["end_state"];
    let final_run = receipt["final_status"]["run"].as_str()?;
    if guide["quest"].as_str() != Some(quest)
        || end["account"].as_str() != Some(account)
        || end["label"].as_str() != Some("end-fail")
        || end["status"]["run"].as_str() != Some(final_run)
        || end["status"] != receipt["final_status"]
    {
        return None;
    }
    let mut prior = Vec::<serde_json::Value>::new();
    let mut last_key = None;
    let mut last_tick = 0;
    let mut anchor = None;
    for row in evidence["statuses"].as_array()? {
        let tick = row["tick"].as_u64()?;
        if tick < last_tick {
            return None;
        }
        last_tick = tick;
        let status = &row["status"];
        let fields = &status["fields"];
        if fields["quest_id"].as_str() != Some(quest) {
            continue;
        }
        let content_miss = counted_miss_status(fields, tick, misses);
        if status["run"].as_str() != Some(final_run)
            || (!content_miss
                && (fields
                    .get("last_failure")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|failure| !failure.is_empty() && failure != gap.park_reason)
                    || fields
                        .get("attempts")
                        .is_some_and(|attempts| attempts.as_u64() != Some(0))
                    || fields.get("step_attempt_outcome").is_some()))
        {
            return None;
        }
        // A journal reread resets the displayed index to zero. It is not a
        // new begin, and neither is the published no-step Blocked status.
        if fields["needs_read"].as_str() != Some("False")
            || !matches!(
                fields["action_state"].as_str(),
                Some("working" | "walking" | "talking" | "fighting" | "banking")
            )
        {
            continue;
        }
        if fields["attempts"].as_u64() != Some(0) && !content_miss {
            return None;
        }
        let sequence_index = usize::try_from(fields["sequence"].as_u64()?).ok()?;
        let step_index = usize::try_from(fields["step_index"].as_u64()?).ok()?;
        let stage = fields["stage"].as_str()?;
        let step_id = fields["step_id"].as_str()?;
        let sequence = guide["sequences"].as_array()?.get(sequence_index)?;
        if sequence["stage"].as_str() != Some(stage) {
            return None;
        }
        let step = sequence["steps"]
            .as_array()?
            .get(step_index)
            .filter(|step| step["id"].as_str() == Some(step_id))
            .or_else(|| {
                guide["prelude"]
                    .as_array()?
                    .get(step_index)
                    .filter(|step| step["id"].as_str() == Some(step_id))
            })?;
        let key = (stage, step_id, sequence_index, step_index);
        if last_key == Some(key) {
            continue;
        }
        if prior.iter().any(|previous| {
            previous["stage"].as_str() == Some(stage)
                && previous["step_id"].as_str() == Some(step_id)
        }) {
            return None;
        }
        // The runner can only leave an active step for a later selection
        // after settling it. Failures publish last_failure/attempts and are
        // rejected above; repeats and the bounded trace are checked too.
        prior.push(serde_json::json!({
            "stage": stage, "step_id": step_id, "kind": step["kind"],
            "settled": true, "source": "uncapped status transition",
        }));
        last_key = Some(key);
        anchor = Some((stage, step_id, step, tick));
    }
    let (stage, step_id, step, begin_tick) = anchor?;
    if Some(stage) != gap.stage.as_deref()
        || step_id != gap.step_id
        || last_tick <= begin_tick
        || end["tick"].as_u64()? < last_tick
    {
        return None;
    }
    let selected = api::game_data::for_revision(api::selected::ClientRevision::R289).ok()?;
    let predicate = if step["settle"] == serde_json::json!({"All": []}) {
        &step["skip_if"]
    } else {
        &step["settle"]
    };
    if captured_gap_fact(predicate, end, guide, &selected) != api::selected::Truth::True {
        return None;
    }
    Some(serde_json::json!({
        "prior_steps": prior,
        "status_log": evidence["status_log"],
        "end_capture": evidence["end_capture"],
        "boundary_predicate": predicate,
        "end_state": {
            "tick": end["tick"], "tile": end["tile"], "inventory": end["inventory"],
            "equipment": end["equipment"], "chat": end["chat"],
        },
    }))
}

fn prove_actor_gap(
    gap: &CampaignGap,
    quest: &str,
    account: &str,
    messages: &[String],
    receipt: &serde_json::Value,
) -> Option<serde_json::Value> {
    // Both receipt shapes use the actor's cross-run total for `deaths`.
    if receipt["account"].as_str() != Some(account)
        || receipt["quest"].as_str() != Some(quest)
        || receipt["quest_complete"].as_bool() != Some(false)
        || receipt["deaths"].as_i64() != Some(0)
        || receipt
            .get("death_count_total_across_runs")
            .is_some_and(|deaths| deaths.as_i64() != Some(0))
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
    let final_stage = receipt["stages_seen"].as_array()?.last()?.as_str()?;
    if final_status["phase"].as_str() != Some("Blocked")
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

    let trace_truncated = messages
        .iter()
        .any(|message| message == "quester trace truncated after the event limit");
    if (trace_truncated && gap.park_reason != "no step for stage")
        || messages
            .iter()
            .any(|message| message == "quester terminal trace event was truncated")
        || messages
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
    let misses = counted_content_misses(quest, receipt)?;
    if root_events.iter().any(|(index, event)| {
        *index >= context_index
            || (event.repeated
                && (event.event == "begin" || event.event.starts_with("failed:"))
                && !misses.iter().any(|miss| {
                    miss.stage == event.stage
                        && miss.step_id == event.step_id
                        && miss.kind == event.kind
                }))
    }) {
        return None;
    }

    let state_proof = if gap.park_reason == "no step for stage" {
        Some(no_step_state_proof(gap, quest, account, receipt, &misses)?)
    } else {
        None
    };
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
            prior_started_steps_settled(&root_events, target_begin.0, &[], &misses)?
        }
        "no step for stage" => {
            let expected_stage = gap.stage.as_deref()?;
            if final_stage != expected_stage {
                return None;
            }
            let state = state_proof.as_ref()?;
            let state_steps = state["prior_steps"].as_array()?;
            let mut prior = prior_started_steps_settled(
                &root_events,
                context_index,
                if trace_truncated { state_steps } else { &[] },
                &misses,
            )?;
            let anchor_begin = root_events.iter().find(|(index, event)| {
                *index < context_index
                    && event.stage == expected_stage
                    && event.step_id == gap.step_id
                    && event.event == "begin"
                    && !event.repeated
            });
            if (!trace_truncated && anchor_begin.is_none())
                || anchor_begin.is_some_and(|(begin, _)| {
                    root_events.iter().any(|(index, event)| {
                        *index > *begin
                            && event.event == "begin"
                            && !event.repeated
                            && (event.stage != expected_stage || event.step_id != gap.step_id)
                    })
                })
            {
                return None;
            }
            for step in state_steps {
                if !prior.iter().any(|previous| {
                    previous["stage"] == step["stage"] && previous["step_id"] == step["step_id"]
                }) {
                    prior.push(step.clone());
                }
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
        "state_evidence": state_proof,
        "counted_content_misses": misses,
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

    #[test]
    fn tick_mean_uses_only_in_game_tick_advances() {
        let start = std::time::Instant::now();
        let mut mean = quester_live::TickMean::default();
        mean.observe_at(10, true, start);
        mean.observe_at(11, true, start + Duration::from_millis(600));
        mean.observe_at(0, false, start + Duration::from_millis(900));
        mean.observe_at(800, true, start + Duration::from_secs(5));
        mean.observe_at(801, true, start + Duration::from_millis(5_600));

        assert_eq!(mean.mean_ms(), Some(600.0));
    }

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

    #[test]
    fn arthur_fixture_seeds_minimum_combat_stats() {
        let row = smoke_row("arthur").unwrap();
        let path =
            serde_json::from_str(include_str!("../../script/paths/289/arthur.json")).unwrap();
        let selected = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
        let fixture =
            scenario::quester::build_quester_path_fixture(scenario::quester::QuesterPathFixture {
                name: "arthur-floor",
                quest_display: row.display,
                path: &path,
                selected: &selected,
                stage: Some("arthur:3"),
                loadout: None,
                extra_items: &[],
                seed_vars: &[],
                stand: WorldTile {
                    x: 2769,
                    z: 3403,
                    level: 2,
                },
                before_relog: Vec::new(),
            })
            .unwrap();
        let combat_stats: Vec<_> = fixture
            .seed
            .stats
            .iter()
            .filter(|stat| stat.skill <= 6)
            .map(|stat| (stat.skill, stat.level))
            .collect();
        assert_eq!(
            combat_stats,
            [
                (0, 40),
                (1, 40),
                (2, 40),
                (3, 40),
                (4, 40),
                (5, 43),
                (6, 40)
            ]
        );
    }

    fn cook_cell(seeds: &QuesterPathSeeds) -> Cell {
        let row = smoke_row("cook").unwrap();
        assert_eq!(row.display, "Cook's Assistant");
        let loadout = seeds.fixture_loadout().unwrap();
        let extra_items = seeds.extra_item_refs();
        let seed_vars = seeds.seed_var_refs();
        smoke_cell(row, seeds, loadout, &extra_items, &seed_vars).unwrap()
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
        let cell = smoke_cell(row, &seeds, loadout, &extra_items, &seed_vars).unwrap();
        assert!(matches!(cell.mode, Mode::Clean));
        assert!(!cell.start_settings.contains_key("allow_danger_zones"));
        assert!(!cell
            .scenario
            .steps
            .iter()
            .any(|step| step.name == "reset authored content stage"));
    }

    #[test]
    fn path_terminal_smoke_uses_the_seeded_stage() {
        let _registry_guard = PATH_REGISTRY_LOCK.lock().unwrap();
        let mut seeds = parse_seeds(COOK_SEEDS).unwrap();
        seeds.mode = Some("path_terminal".into());
        let cell = cook_cell(&seeds);
        assert!(matches!(cell.mode, Mode::PathTerminal));
        assert!(cell.start_settings.contains_key("quests"));
        assert!(cell
            .scenario
            .steps
            .iter()
            .any(|step| step.name == "reset authored content stage"));
    }

    #[test]
    fn production_path_smoke_seed_fixtures_build_embedded_stages() {
        let _registry_guard = PATH_REGISTRY_LOCK.lock().unwrap();
        for (quest, stage, text) in [
            (
                "sheep",
                "sheep:1",
                include_str!("fixtures/quester-path/sheep-spin.json"),
            ),
            (
                "prince",
                "prince:20",
                include_str!("fixtures/quester-path/prince-production.json"),
            ),
            (
                "blackarmgang",
                "p:8",
                include_str!("fixtures/quester-path/blackarm-jonny.json"),
            ),
            (
                "arthur",
                "arthur:3",
                include_str!("fixtures/quester-path/arthur-mordred.json"),
            ),
            (
                "itexam",
                "itexam:first",
                include_str!("fixtures/quester-path/itexam-thieve.json"),
            ),
            (
                "priest",
                "priest:0",
                include_str!("fixtures/quester-path/priest-start.json"),
            ),
        ] {
            let seeds = parse_seeds(text).unwrap();
            assert_eq!(seeds.stage, stage);
            let row = smoke_row(quest).unwrap();
            let loadout = seeds.fixture_loadout().unwrap();
            let extra_items = seeds.extra_item_refs();
            let seed_vars = seeds.seed_var_refs();
            let cell = smoke_cell(row, &seeds, loadout, &extra_items, &seed_vars).unwrap();
            assert_eq!(cell.quest, quest);
            match &cell.mode {
                Mode::Stage { expect } => assert_eq!(expect, &seeds.expect),
                other => panic!("expected fresh stage mode, found {other:?}"),
            }
            assert_eq!(cell.scenario.settings.deadline, DEADLINE);
            assert_eq!(cell.scenario.settings.start_script, Some("Quester"));
            let stage_seed = cell
                .scenario
                .steps
                .iter()
                .position(|step| step.name == "reset authored content stage")
                .expect("fixture seeds its authored starting stage");
            let relog = cell
                .scenario
                .steps
                .iter()
                .rposition(|step| matches!(step.kind, scenario::StepKind::Relog))
                .expect("fixture relogs after seeding");
            let stand = cell
                .scenario
                .steps
                .iter()
                .position(|step| step.name == "stand at the authored sequence")
                .expect("fixture stands before Start");
            assert!(stage_seed < relog && relog < stand);

            match quest {
                "blackarmgang" => {
                    assert_eq!(row.profile, TestProfile::Base40);
                    assert_eq!(seeds.expect, vec!["p:9".to_owned()]);
                    // p:8 has a journal predicate, not a varp hint. The content
                    // constant phoenixgang_spoken_straven is 8, before joining.
                    assert_eq!(seed_vars, [("phoenixgang", 8)]);
                    assert_eq!(cell.start_settings["gang"], "phoenix");
                    assert_ne!(seeds.stage, seeds.expect[0]);
                    assert!(!seeds
                        .extra_items
                        .iter()
                        .any(|(item, _)| item == "intelligence_report"));
                }
                "arthur" => {
                    assert_eq!(row.profile, TestProfile::Base40);
                    assert_eq!(seeds.expect, vec!["arthur:4".to_owned()]);
                    assert!(seeds.extra_items.is_empty());
                    assert!(seeds.seed_vars.is_empty());
                    assert!(seeds.before_relog.is_empty());
                    assert_ne!(seeds.stage, seeds.expect[0]);
                }
                "itexam" => {
                    assert_eq!(row.profile, TestProfile::Base40);
                    assert_eq!(seeds.expect, vec!["itexam:second".to_owned()]);
                    assert!(seeds.seed_vars.is_empty());
                    assert!(seeds.before_relog.is_empty());
                    assert!(seeds
                        .extra_items
                        .iter()
                        .any(|(item, _)| item == "rock_sample2"));
                    assert!(seeds
                        .extra_items
                        .iter()
                        .any(|(item, _)| item == "rock_sample3"));
                    assert!(!seeds
                        .extra_items
                        .iter()
                        .any(|(item, _)| item == "rock_sample1"));
                    assert_ne!(seeds.stage, seeds.expect[0]);
                }
                _ => {}
            }
        }
    }

    #[test]
    fn itexam_fixture_starts_thieving_at_the_authored_minimum() {
        let _registry_guard = PATH_REGISTRY_LOCK.lock().unwrap();
        let seeds = parse_seeds(include_str!("fixtures/quester-path/itexam-thieve.json")).unwrap();
        let row = smoke_row("itexam").unwrap();
        let path =
            serde_json::from_str(include_str!("../../script/paths/289/itexam.json")).unwrap();
        let selected = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
        let extra_items = seeds.extra_item_refs();
        let seed_vars = seeds.seed_var_refs();
        let fixture =
            scenario::quester::build_quester_path_fixture(scenario::quester::QuesterPathFixture {
                name: "itexam-thieve",
                quest_display: row.display,
                path: &path,
                selected: &selected,
                stage: Some(&seeds.stage),
                loadout: seeds.fixture_loadout().unwrap(),
                extra_items: &extra_items,
                seed_vars: &seed_vars,
                stand: WorldTile {
                    x: seeds.stand[0],
                    z: seeds.stand[1],
                    level: seeds.stand[2],
                },
                before_relog: seeds.before_relog_steps(),
            })
            .unwrap();
        let thieving_skill = client::client::skill::Skill::names
            .iter()
            .position(|name| name.eq_ignore_ascii_case("thieving"))
            .unwrap() as u8;
        let thieving = fixture
            .seed
            .stats
            .iter()
            .find(|minimum| minimum.skill == thieving_skill)
            .unwrap();
        assert_eq!(thieving.level, 25);
    }

    #[test]
    fn smoke_row_uses_each_quest_profile() {
        assert!(smoke_row("not-a-quest").is_err());
        assert_eq!(smoke_row("dragon").unwrap().profile, TestProfile::Base60);
        assert_eq!(
            smoke_row("elemental_workshop").unwrap().profile,
            TestProfile::Base60
        );
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
        assert!(smoke_cell(row, &seeds, None, &empty, &empty).is_err());
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
            "stages_seen": [stage],
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
                "run": "RunKey { slot: 1, run: 1, session: 1 }",
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

    fn no_step_receipt() -> serde_json::Value {
        let mut actor = actor_receipt("first-skipped-step", "no step for stage", STAGE);
        let near = serde_json::json!({
            "Fact": {"kind": "near", "version": 1, "args": {"tile": [3200,3200,0], "radius": 0}}
        });
        actor["gap_state_evidence"] = serde_json::json!({
            "status_log": "recorded/status.jsonl",
            "end_capture": "recorded/end-fail.json",
            "guide": {
                "quest": QUEST, "areas": {}, "prelude": [],
                "sequences": [{
                    "stage": STAGE,
                    "steps": [
                        {"id": "first-skipped-step", "kind": "equip", "skip_if": near, "settle": near},
                        {"id": PRIOR_STEP, "kind": "walk", "skip_if": {"Any": []}, "settle": near},
                        {"id": "later-step", "kind": "perform", "skip_if": near, "settle": near},
                    ],
                }],
            },
            "statuses": [
                {"tick": 40, "status": {
                    "run": actor["final_status"]["run"], "phase": "Working",
                    "fields": {
                        "quest_id": QUEST, "stage": STAGE, "step_id": PRIOR_STEP,
                        "sequence": 0, "step_index": 1, "needs_read": "False",
                        "action_state": "walking", "attempts": 0,
                    },
                }},
                {"tick": 42, "status": actor["final_status"]},
            ],
            "end_state": {
                "account": ACCOUNT, "label": "end-fail", "tick": 43,
                "status": actor["final_status"], "tile": [3200,3200,0],
                "inventory": [], "equipment": [], "chat": [],
            },
        });
        actor
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

    fn recorded_viking_packet() -> serde_json::Value {
        // Oracle inputs and native messages from QUEST-CAMPAIGN/viking/gap-1,
        // without changing its missing death alias or lean Blocked status.
        serde_json::from_str(include_str!("fixtures/campaign-viking-gap-1.json")).unwrap()
    }

    #[test]
    fn recorded_viking_gap_packet_proves_advancing_settlements() {
        let packet = recorded_viking_packet();
        let gap = campaign_gap(&serde_json::json!({"gap": packet["gap"]}))
            .unwrap()
            .unwrap();
        let actor = packet["receipt"].clone();
        assert!(actor.get("death_count_total_across_runs").is_none());
        assert!(actor["final_status"]["fields"].get("quest").is_none());
        let account = actor["account"].as_str().unwrap().to_owned();
        let receipts = std::collections::BTreeMap::from([(account, actor)]);
        let result = Err(packet["error"].as_str().unwrap().to_owned());
        let proof = campaign_gap_proof(
            &result,
            &[Some(gap)],
            QUEST,
            packet["logs"].as_array().unwrap(),
            &receipts,
        )
        .expect("the recorded clean Viking boundary must be proven");
        assert_eq!(proof["stage"], "viking:peer");
        assert_eq!(
            proof["accounts"][0]["prior_steps"],
            serde_json::json!([
                {"stage": "viking:0", "step_id": "ask-brundt-to-start", "kind": "talk", "settled": true},
                {"stage": "viking:1", "step_id": "accept-peer-trial", "kind": "talk", "settled": true},
                {"stage": "viking:peer", "step_id": "walk-peer-door", "kind": "walk", "settled": true},
            ])
        );
    }

    #[test]
    fn recorded_viking_gap_keeps_both_death_counter_guards() {
        let packet = recorded_viking_packet();
        let gap = campaign_gap(&serde_json::json!({"gap": packet["gap"]}))
            .unwrap()
            .unwrap();
        let account = packet["receipt"]["account"].as_str().unwrap();
        let messages = campaign_trace_messages(QUEST, packet["logs"].as_array().unwrap());
        let messages = &messages[account];
        let mut actor = packet["receipt"].clone();
        actor["death_count_total_across_runs"] = serde_json::json!(0);
        assert!(prove_actor_gap(&gap, QUEST, account, messages, &actor).is_some());
        for (key, value) in [
            ("deaths", serde_json::json!(1)),
            ("death_count_total_across_runs", serde_json::json!(1)),
            ("deaths", serde_json::Value::Null),
        ] {
            let mut actor = packet["receipt"].clone();
            actor[key] = value;
            assert!(
                prove_actor_gap(&gap, QUEST, account, messages, &actor).is_none(),
                "{key} must retain its death guard"
            );
        }
    }

    #[test]
    fn advancing_settlement_requires_one_pending_begin_even_with_reused_ids() {
        let begin_0 = "quester viking: stage viking:0 step reused (talk) begin";
        let begin_1 = "quester viking: stage viking:1 step reused (talk) begin";
        let settle_1 = "quester viking: stage viking:1 step reused (talk) settled";
        let settle_peer = "quester viking: stage viking:peer step reused (talk) settled";
        let events = |messages: &[&str]| {
            messages
                .iter()
                .enumerate()
                .map(|(index, message)| (index, parse_root_step_trace(QUEST, message).unwrap()))
                .collect::<Vec<_>>()
        };
        let ordered = events(&[begin_0, settle_1, begin_1, settle_peer]);
        let proof = prior_started_steps_settled(&ordered, usize::MAX, &[], &[])
            .expect("sequential reused IDs have unambiguous advancing settlements");
        assert_eq!(proof.len(), 2);
        assert_eq!(proof[0]["stage"], "viking:0");
        assert_eq!(proof[1]["stage"], "viking:1");
        let ambiguous = events(&[begin_0, begin_1, settle_1, settle_peer]);
        assert!(prior_started_steps_settled(&ambiguous, usize::MAX, &[], &[]).is_none());
    }

    fn receipts(actor: serde_json::Value) -> std::collections::BTreeMap<String, serde_json::Value> {
        std::collections::BTreeMap::from([(ACCOUNT.to_owned(), actor)])
    }

    fn error_result() -> Result<serde_json::Value, String> {
        Err(format!("{ACCOUNT} Quester parked/blocked: phase Blocked"))
    }

    fn counted_miss_packet(
        no_step: bool,
    ) -> (CampaignGap, Vec<serde_json::Value>, serde_json::Value) {
        let mut actor = no_step_receipt();
        let mut evidence = actor["gap_state_evidence"].take();
        let gap = if no_step {
            CampaignGap {
                step_id: PRIOR_STEP.into(),
                park_reason: "no step for stage".into(),
                stage: Some(STAGE.into()),
            }
        } else {
            actor = actor_receipt(GAP_STEP, "wait exhausted", STAGE);
            wait_gap()
        };
        let authored = &mut evidence["guide"]["sequences"][0]["steps"][1];
        authored["kind"] = serde_json::json!("interact");
        authored["max_attempts"] = serde_json::json!(3);
        let row = |tick, attempt| {
            let mut fields = serde_json::json!({
                "quest_id": QUEST, "stage": STAGE, "step_id": PRIOR_STEP,
                "sequence": 0, "step_index": 1, "needs_read": "False",
                "action_state": "working", "attempts": attempt,
                "step_attempts": attempt,
            });
            if attempt > 0 {
                fields["step_attempt_outcome"] = serde_json::json!("content_miss");
                fields["last_failure"] = serde_json::json!("content miss");
            }
            serde_json::json!({
                "tick": tick,
                "status": {
                    "run": actor["final_status"]["run"], "phase": "Working", "fields": fields,
                },
            })
        };
        evidence["statuses"] = serde_json::json!([
            row(10, 0), row(11, 1), row(12, 1), row(13, 2),
            {"tick": 42, "status": actor["final_status"]},
        ]);
        evidence["end_state"]["status"] = actor["final_status"].clone();
        actor["gap_state_evidence"] = evidence;
        let mut logs = vec![
            log(&format!("quester {QUEST}: run start stage={STAGE}")),
            log(&format!(
                "quester {QUEST}: stage {STAGE} step {PRIOR_STEP} (interact) begin"
            )),
            log(&format!(
                "quester {QUEST}: stage {STAGE} step {PRIOR_STEP} (interact) failed: content miss"
            )),
            log(&format!(
                "quester {QUEST}: stage {STAGE} step {PRIOR_STEP} (interact) settled"
            )),
        ];
        if !no_step {
            logs.extend([
                log(&format!(
                    "quester {QUEST}: stage {STAGE} step {GAP_STEP} (wait) begin"
                )),
                log(&format!(
                    "quester {QUEST}: stage {STAGE} step {GAP_STEP} (wait) failed: wait exhausted"
                )),
            ]);
        }
        // The native trace emits each distinct line once, then flushes these
        // additional-occurrence summaries immediately before the park.
        logs.extend([
            log(&format!(
                "quester {QUEST}: stage {STAGE} step {PRIOR_STEP} (interact) begin (repeated 2 additional times)"
            )),
            log(&format!(
                "quester {QUEST}: stage {STAGE} step {PRIOR_STEP} (interact) failed: content miss (repeated 1 additional times)"
            )),
            log(&format!(
                "quester {QUEST}: park context step={} child_recipe= child=",
                if no_step { "first-skipped-step" } else { GAP_STEP }
            )),
            log(&format!("quester {QUEST}: park: {}", gap.park_reason)),
        ]);
        (gap, logs, actor)
    }

    #[test]
    fn counted_content_misses_within_budget_prove_both_gap_kinds() {
        for no_step in [false, true] {
            let (gap, logs, actor) = counted_miss_packet(no_step);
            let proof = campaign_gap_proof(
                &error_result(),
                &[Some(gap)],
                QUEST,
                &logs,
                &receipts(actor),
            )
            .expect("two native content misses followed by settlement fit a three-attempt budget");
            let misses = proof["accounts"][0]["counted_content_misses"]
                .as_array()
                .unwrap();
            assert_eq!(
                misses.len(),
                2,
                "duplicate status publications are not new attempts"
            );
            assert_eq!(misses[0]["attempt"], 1);
            assert_eq!(misses[1]["attempt"], 2);
            assert_eq!(misses[1]["max_attempts"], 3);
        }
    }

    #[test]
    fn counted_content_misses_do_not_waive_other_failure_classes() {
        for no_step in [false, true] {
            for failure in [
                "dialogue open failed",
                "walk failed",
                "transport failed",
                "harness failed",
                "infrastructure failed",
                "step settle timeout",
            ] {
                let (gap, mut logs, actor) = counted_miss_packet(no_step);
                logs[2] = log(&format!(
                    "quester {QUEST}: stage {STAGE} step {PRIOR_STEP} (interact) failed: {failure}"
                ));
                assert!(
                    campaign_gap_proof(
                        &error_result(),
                        &[Some(gap)],
                        QUEST,
                        &logs,
                        &receipts(actor)
                    )
                    .is_none(),
                    "{failure} must not inherit a content-miss allowance"
                );
            }
        }
    }

    #[test]
    fn counted_content_misses_require_exact_native_and_authored_authority() {
        for no_step in [false, true] {
            for mutation in [
                "missing evidence",
                "missing budget",
                "over budget",
                "no successful attempt left",
                "missing outcome",
                "unknown outcome",
                "missing counter",
                "wrong run",
                "wrong step",
                "wrong stage",
                "counter jump",
                "counter reversal",
                "missing failure event",
                "unaccounted begin",
                "extra content miss",
                "wrong root kind",
            ] {
                let (gap, mut logs, mut actor) = counted_miss_packet(no_step);
                let evidence = &mut actor["gap_state_evidence"];
                match mutation {
                    "missing evidence" => *evidence = serde_json::Value::Null,
                    "missing budget" => evidence["guide"]["sequences"][0]["steps"][1]["max_attempts"] =
                        serde_json::Value::Null,
                    "over budget" => evidence["guide"]["sequences"][0]["steps"][1]["max_attempts"] =
                        serde_json::json!(1),
                    "no successful attempt left" =>
                        evidence["guide"]["sequences"][0]["steps"][1]["max_attempts"] =
                            serde_json::json!(2),
                    "missing outcome" | "unknown outcome" | "missing counter" | "wrong run"
                    | "wrong step" | "wrong stage" => {
                        for row in &mut evidence["statuses"].as_array_mut().unwrap()[1..=3] {
                            let status = &mut row["status"];
                            match mutation {
                                "missing outcome" => {
                                    status["fields"].as_object_mut().unwrap()
                                        .remove("step_attempt_outcome");
                                }
                                "unknown outcome" => status["fields"]["step_attempt_outcome"] =
                                    serde_json::json!("operational_failure"),
                                "missing counter" => status["fields"]["step_attempts"] =
                                    serde_json::Value::Null,
                                "wrong run" => status["run"] = serde_json::json!("another run"),
                                "wrong step" => status["fields"]["step_id"] =
                                    serde_json::json!("later-step"),
                                "wrong stage" => status["fields"]["stage"] =
                                    serde_json::json!("viking:0"),
                                _ => unreachable!(),
                            }
                        }
                    }
                    "counter jump" => {
                        evidence["guide"]["sequences"][0]["steps"][1]["max_attempts"] =
                            serde_json::json!(4);
                        evidence["statuses"][3]["status"]["fields"]["step_attempts"] =
                            serde_json::json!(3);
                    }
                    "counter reversal" => evidence["statuses"][3]["status"]["fields"]["step_attempts"] =
                        serde_json::json!(0),
                    "missing failure event" => { logs.remove(2); }
                    "unaccounted begin" => logs.push(log(&format!(
                        "quester {QUEST}: stage {STAGE} step {PRIOR_STEP} (interact) begin"
                    ))),
                    "extra content miss" => {
                        let mut extra = evidence["statuses"][3].clone();
                        extra["tick"] = serde_json::json!(14);
                        extra["status"]["fields"]["step_attempts"] = serde_json::json!(3);
                        evidence["statuses"].as_array_mut().unwrap().insert(4, extra);
                    }
                    "wrong root kind" => logs[2] = log(&format!(
                        "quester {QUEST}: stage {STAGE} step {PRIOR_STEP} (walk) failed: content miss"
                    )),
                    _ => unreachable!(),
                }
                assert!(
                    campaign_gap_proof(
                        &error_result(),
                        &[Some(gap)],
                        QUEST,
                        &logs,
                        &receipts(actor)
                    )
                    .is_none(),
                    "{mutation} must fail closed (no_step={no_step})"
                );
            }
        }
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
            log("quester viking: park context step=first-skipped-step child_recipe= child="),
            log("quester viking: park: no step for stage"),
        ];
        let proof = campaign_gap_proof(
            &error_result(),
            &[Some(gap)],
            QUEST,
            &logs,
            &receipts(no_step_receipt()),
        )
        .expect("no-step gap should prove its exact stage boundary");
        assert_eq!(proof["step_id"], PRIOR_STEP);
        assert_eq!(proof["stage"], STAGE);
        assert!(proof["prior_steps_settled"].as_bool().unwrap());
        assert_eq!(
            proof["accounts"][0]["park_context_step"],
            "first-skipped-step"
        );
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
            log("quester viking: park context step=first-skipped-step child_recipe= child="),
            log("quester viking: park: no step for stage"),
        ];
        assert!(campaign_gap_proof(
            &error_result(),
            &[Some(gap)],
            QUEST,
            &logs,
            &receipts(no_step_receipt()),
        )
        .is_none());
    }

    #[test]
    fn no_step_gap_without_a_trace_cap_still_requires_its_anchor_trace() {
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
            &receipts(no_step_receipt()),
        )
        .is_none());
    }

    #[test]
    fn capped_no_step_gap_uses_uncapped_status_and_captured_goal() {
        let gap = CampaignGap {
            step_id: PRIOR_STEP.into(),
            park_reason: "no step for stage".into(),
            stage: Some(STAGE.into()),
        };
        let logs = vec![
            log("quester viking: run start stage=viking:8"),
            log("quester viking: stage viking:8 step walk-peer-door (walk) begin"),
            log("quester trace truncated after the event limit"),
            log("quester viking: park context step=first-skipped-step child_recipe= child="),
            log("quester viking: park: no step for stage"),
        ];
        let actor = no_step_receipt();
        assert!(campaign_gap_proof(
            &error_result(),
            &[Some(gap.clone())],
            QUEST,
            &logs,
            &receipts(actor.clone()),
        )
        .is_some());
        let mut after_cap_logs = logs.clone();
        after_cap_logs.remove(1);
        assert!(
            campaign_gap_proof(
                &error_result(),
                &[Some(gap.clone())],
                QUEST,
                &after_cap_logs,
                &receipts(actor.clone()),
            )
            .is_some(),
            "a fully capped anchor still has uncapped status and end-state proof"
        );
        for mutation in [
            "missing statuses",
            "missing capture",
            "wrong tile",
            "malformed tile",
            "failure",
            "retry",
            "later begin",
            "wrong run",
            "vacuous goal",
        ] {
            let mut bad = actor.clone();
            let evidence = &mut bad["gap_state_evidence"];
            match mutation {
                "missing statuses" => evidence["statuses"] = serde_json::json!([]),
                "missing capture" => evidence["end_state"] = serde_json::Value::Null,
                "wrong tile" => evidence["end_state"]["tile"] = serde_json::json!([3201, 3200, 0]),
                "malformed tile" => evidence["end_state"]["tile"] = serde_json::json!([3200]),
                "failure" => {
                    evidence["statuses"][0]["status"]["fields"]["last_failure"] =
                        "expected dialogue did not open".into()
                }
                "retry" => evidence["statuses"][0]["status"]["fields"]["attempts"] = 1.into(),
                "later begin" => {
                    let mut later = evidence["statuses"][0].clone();
                    later["tick"] = 41.into();
                    later["status"]["fields"]["step_id"] = "later-step".into();
                    later["status"]["fields"]["step_index"] = 2.into();
                    evidence["statuses"]
                        .as_array_mut()
                        .unwrap()
                        .insert(1, later);
                }
                "wrong run" => evidence["statuses"][0]["status"]["run"] = "another run".into(),
                "vacuous goal" => {
                    evidence["guide"]["sequences"][0]["steps"][1]["settle"] =
                        serde_json::json!({"All": []})
                }
                _ => unreachable!(),
            }
            assert!(
                campaign_gap_proof(
                    &error_result(),
                    &[Some(gap.clone())],
                    QUEST,
                    &logs,
                    &receipts(bad),
                )
                .is_none(),
                "{mutation}"
            );
        }
        for message in [
            "quester viking: stage viking:8 step earlier (talk) failed: expected dialogue did not open",
            "quester viking: stage viking:8 step earlier (talk) begin (repeated 1 additional times)",
            "quester terminal trace event was truncated",
        ] {
            let mut bad_logs = logs.clone();
            bad_logs.insert(1, log(message));
            assert!(campaign_gap_proof(
                &error_result(), &[Some(gap.clone())], QUEST, &bad_logs, &receipts(actor.clone()),
            ).is_none(), "{message}");
        }
    }

    #[test]
    fn recorded_sheep_status_and_capture_prove_the_anchor_but_not_a_working_final_phase() {
        let packet: serde_json::Value =
            serde_json::from_str(include_str!("fixtures/campaign-sheepherder-gap-2.json")).unwrap();
        let gap = campaign_gap(&serde_json::json!({"gap": packet["gap"]}))
            .unwrap()
            .unwrap();
        let actor = &packet["receipt"];
        let account = actor["account"].as_str().unwrap();
        let state = no_step_state_proof(&gap, "sheepherder", account, actor, &[])
            .expect("the real uncapped statuses and worn prod establish the boundary");
        assert_eq!(state["prior_steps"].as_array().unwrap().len(), 7);
        assert_eq!(
            state["prior_steps"].as_array().unwrap().last().unwrap()["step_id"],
            "wield-prod"
        );
        assert_eq!(actor["final_status"]["phase"], "Working");
        let messages = campaign_trace_messages("sheepherder", packet["logs"].as_array().unwrap());
        assert!(
            prove_actor_gap(&gap, "sheepherder", account, &messages[account], actor).is_none(),
            "old receipts must not be silently promoted to a later native phase"
        );
    }

    #[test]
    fn recorded_murder_retry_remains_unclean_despite_end_state_and_trace_cap() {
        let packet: serde_json::Value =
            serde_json::from_str(include_str!("fixtures/campaign-murder-gap-2.json")).unwrap();
        let gap = campaign_gap(&serde_json::json!({"gap": packet["gap"]}))
            .unwrap()
            .unwrap();
        let actor = &packet["receipt"];
        let account = actor["account"].as_str().unwrap();
        assert!(actor["gap_state_evidence"]["end_state"]["inventory"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row[0] == 1814));
        let messages = campaign_trace_messages("murder", packet["logs"].as_array().unwrap());
        assert!(messages[account]
            .iter()
            .any(|message| message == "quester trace truncated after the event limit"));
        assert!(messages[account]
            .iter()
            .any(|message| message.contains("begin (repeated")));
        assert!(no_step_state_proof(&gap, "murder", account, actor, &[]).is_none());
        assert!(prove_actor_gap(&gap, "murder", account, &messages[account], actor).is_none());
    }
}
