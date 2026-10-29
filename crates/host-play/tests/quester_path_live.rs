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
                .map_err(|error| error.to_string())
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

fn campaign_cells(manifest_path: &std::path::Path) -> Result<Vec<Cell>, String> {
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
        .map_err(|error| error.to_string())?;
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
        Ok(Cell {
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
        })
    }).collect()
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
    assert!(std::env::var_os("QUEST_CAMPAIGN_TICKS").is_some());
    assert!(
        api::hostlog::install_sink(&CAMPAIGN_LOG_SINK),
        "campaign needs its own session log sink"
    );
    let manifest = PathBuf::from(
        std::env::var_os("QUEST_CAMPAIGN_MANIFEST").expect("QUEST_CAMPAIGN_MANIFEST"),
    );
    let cells = campaign_cells(&manifest).unwrap_or_else(|error| panic!("{error}"));
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
    let logs = CAMPAIGN_LOGS.lock().unwrap();
    let mut file = std::fs::File::create(root.join("session.jsonl")).unwrap();
    use std::io::Write;
    for line in logs.iter() {
        writeln!(file, "{line}").unwrap();
    }
    println!("QUEST-CAMPAIGN result={:?}", result.is_ok());
    result.unwrap();
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
