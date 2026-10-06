//! Generic Path live smoke: one Base40 cell for a content quest id.
//!
//! This is the permanent form of the PATHS-OVERNIGHT throwaway runner. It
//! runs the Path id in `QUESTER_PATH` from the embedded release index, or
//! from `QUESTER_PATH_DIR` through the existing `FolderSource` registry.
//! Optional account seeds come from the `QUESTER_SEEDS` JSON file. The cell
//! runs under a fixed deadline; expected stage settling or terminal completion
//! passes after Start.
//!
//! Run from the repository root against Engine A (the parent prepares the
//! owned writable APFS cache clone first), with a throwaway HOME:
//!
//! ```text
//! env HOME="$(mktemp -d)" LIVE=1 BOT_CPU=1 BOT_LIVE_NAME_PREFIX=qh BOT_NAV_BUILD=skip \
//!   WORLD_GAME_PORT=44594 WORLD_HTTP_PORT=1080 WORLD_NAV_PACK=<274bot.navpack> \
//!   WORLD_ENGINE_DIR=<engine-A-dir> RS2B0T=<catalog-root> \
//!   BOT_CACHE_DIR=<owned-writable-APFS-cache-clone> LIVE_EVIDENCE_DIR=<evidence-root> \
//!   QUESTER_PATH=cook QUESTER_SEEDS=<evidence-root>/seeds/cook.json \
//!   cargo test -p host-play --test quester_path_live --features "live-harness test-support" \
//!     live_quester_path_smoke -- --ignored --nocapture --test-threads=1
//! ```
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
/// the shared engine.
const DEADLINE: Duration = Duration::from_secs(1500);

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
) -> Result<Cell, String> {
    let mode = match seeds.mode.as_deref().unwrap_or("stage") {
        "stage" => Mode::Stage {
            expect: seeds.expect.clone(),
        },
        "clean" => Mode::Clean,
        other => return Err(format!("unknown smoke mode {other:?}")),
    };
    let mut cell = quester_live::path_cell(PathCell {
        quest: row.quest,
        display: row.display,
        label: "path-smoke".to_owned(),
        stage: seeds.stage.as_str(),
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
    cell.scenario.settings.deadline = DEADLINE;
    Ok(cell)
}

#[test]
#[ignore = "requires LIVE=1 and the shared local 289 engine; see common/quester_live.rs"]
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
    let result = quester_live::run(cell);
    println!("PATH-SMOKE {quest} ({source}): {:?}", result.is_ok());
    registry::set_source(before);
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

    fn parse_seeds(text: &str) -> Result<QuesterPathSeeds, String> {
        scenario::quester::parse_quester_path_seeds(text, "QUESTER_SEEDS")
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
    fn smoke_row_rejects_unknown_quests_and_non_base40_profiles() {
        assert!(smoke_row("not-a-quest").is_err());
        assert!(smoke_row("dragon").is_err());
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
