//! Generic Path live smoke: one Base40 cell for a content quest id.
//!
//! This is the permanent form of the PATHS-OVERNIGHT throwaway runner. It
//! runs the Path id in `QUESTER_PATH_QUEST` from the embedded release index,
//! or from a folder through the existing `FolderSource` registry when
//! `QUESTER_PATH_FOLDER` names one, with the Base40 qualification profile.
//! Account seeds come from the small JSON file in `QUESTER_PATH_SEEDS` and
//! the cell runs under a fixed deadline; completing the quest is optional,
//! settling into an expected stage (or the terminal) after Start passes.
//!
//! Run from the repository root against Engine A (the parent prepares the
//! owned writable APFS cache clone first), with a throwaway HOME:
//!
//! ```text
//! env HOME="$(mktemp -d)" LIVE=1 BOT_CPU=1 BOT_LIVE_NAME_PREFIX=qh BOT_NAV_BUILD=skip \
//!   WORLD_GAME_PORT=44594 WORLD_HTTP_PORT=1080 WORLD_NAV_PACK=<274bot.navpack> \
//!   WORLD_ENGINE_DIR=<engine-A-dir> RS2B0T=<catalog-root> \
//!   BOT_CACHE_DIR=<owned-writable-APFS-cache-clone> LIVE_EVIDENCE_DIR=<evidence-root> \
//!   QUESTER_PATH_QUEST=cook QUESTER_PATH_SEEDS=<evidence-root>/seeds/cook.json \
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

use api::selected::{ClientRevision, FamilyPreparation};
use api::snapshot::WorldTile;
use quester_live::{Cell, Mode, PathCell};
use scenario::quester::{FixtureLoadout, QuestFixtureProfile, StandardKit, TestProfile};
use script::quester::registry::{self, FolderSource};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

/// Fixed whole-cell deadline (fixture seed plus quest). Not env-configurable:
/// a smoke that cannot Start and settle inside it fails instead of burning
/// the shared engine.
const DEADLINE: Duration = Duration::from_secs(1500);

/// Seeds for one smoke cell, read from the `QUESTER_PATH_SEEDS` JSON file.
#[derive(Debug, serde::Deserialize)]
struct Seeds {
    stage: String,
    stand: [i32; 3],
    #[serde(default)]
    loadout: Option<SeedsLoadout>,
    #[serde(default)]
    extra_items: Vec<(String, i32)>,
    /// Explicit operator seeds for hint-less stages (released Cook: the
    /// stage varp and value). Recorded like every other cheat.
    #[serde(default)]
    seed_vars: Vec<(String, i32)>,
    /// Raw fixture cheats (`givebank` stock, prerequisite quest flags),
    /// inserted before the fixture's final relog. Never gameplay.
    #[serde(default)]
    before_relog: Vec<String>,
    /// Stages that pass the cell once observed after Start (a terminal
    /// completion also passes). Empty means completion only.
    #[serde(default)]
    expect: Vec<String>,
    /// `stage` (default) or `clean`.
    #[serde(default)]
    mode: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
enum SeedsLoadout {
    #[serde(rename = "path")]
    Path(String),
    #[serde(rename = "standard")]
    Standard(String),
}

fn parse_seeds(text: &str) -> Result<Seeds, String> {
    serde_json::from_str(text).map_err(|error| format!("parse QUESTER_PATH_SEEDS: {error}"))
}

fn seeds_loadout(loadout: Option<&SeedsLoadout>) -> Result<Option<FixtureLoadout<'_>>, String> {
    loadout
        .map(|loadout| match loadout {
            SeedsLoadout::Path(key) => Ok(FixtureLoadout::Path(key.as_str())),
            SeedsLoadout::Standard(style) => match style.as_str() {
                "melee" => Ok(FixtureLoadout::Standard(StandardKit::Melee)),
                "magic" => Ok(FixtureLoadout::Standard(StandardKit::Magic)),
                "ranged" => Ok(FixtureLoadout::Standard(StandardKit::Ranged)),
                other => Err(format!("unknown standard kit {other:?}")),
            },
        })
        .transpose()
}

fn seed_command(command: String) -> scenario::Step {
    scenario::Step {
        name: "smoke seed before final relog",
        kind: scenario::StepKind::Perform {
            send: Box::new(move |client, _| api::interact::cheat(client, &command).is_sent()),
        },
        wait: scenario::Wait {
            arm: scenario::Proof::SideTabAvailable { index: 3 },
            budget_ticks: 100,
        },
    }
}

/// The harness row for an env-driven quest: static id, display and profile,
/// so the cell needs no allocation to satisfy fixture lifetimes. Refuses
/// quests the profile table moved off Base40: the smoke is defined as a
/// Base40 qualification cell.
fn smoke_row(quest: &str) -> Result<QuestFixtureProfile, String> {
    let row = scenario::quester::fixture_row(quest)?;
    if row.profile != TestProfile::Base40 {
        return Err(format!("{quest} is not a Base40 smoke quest"));
    }
    Ok(row)
}

/// Path bytes plus where they came from. Loading a folder also publishes it
/// to the registry snapshot `path_cell` reads, shadowing the embedded index;
/// without a folder the bundled snapshot answers for the embedded index.
fn resolve_source(quest: &str) -> Result<&'static str, String> {
    if let Some(folder) = std::env::var_os("QUESTER_PATH_FOLDER") {
        let folder = PathBuf::from(folder);
        if !folder.is_absolute() || !folder.is_dir() {
            return Err(format!(
                "QUESTER_PATH_FOLDER must be an absolute directory: {}",
                folder.display()
            ));
        }
        let before = registry::source();
        registry::set_source(FolderSource {
            enabled: true,
            folder,
        });
        let selected = FamilyPreparation::run(|_| {
            api::game_data::for_revision(ClientRevision::R289).expect("selected 289 data")
        })
        .map_err(|error| format!("selected data worker: {error:?}"))?
        .join()
        .map_err(|error| format!("selected data worker: {error:?}"))?;
        let data = Arc::clone(&selected);
        let loaded = FamilyPreparation::run(move |worker| registry::reload(&data, worker))
            .map_err(|error| format!("folder reload worker: {error:?}"))?
            .join()
            .map_err(|error| format!("folder reload worker: {error:?}"))?
            .map_err(|error| format!("folder reload: {error:?}"))?;
        for diagnostic in loaded.diagnostics() {
            println!("PATH-DIAG {diagnostic}");
        }
        if loaded.bytes(quest).is_none() {
            registry::set_source(before);
            return Err(format!("{quest} is not served by the folder registry"));
        }
        // Keep the folder published for the run below; the caller restores
        // the previous source afterwards.
        return Ok(loaded.path_source(quest).label());
    }
    script::quester::compile::path_bytes(quest)
        .map(|_| "embedded")
        .ok_or_else(|| format!("{quest} is not an embedded release Path"))
}

fn smoke_cell(
    row: QuestFixtureProfile,
    seeds: &Seeds,
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
        before_relog: seeds
            .before_relog
            .iter()
            .cloned()
            .map(seed_command)
            .collect(),
    })?;
    cell.scenario.settings.deadline = DEADLINE;
    Ok(cell)
}

#[test]
#[ignore = "requires LIVE=1 and the shared local 289 engine; see common/quester_live.rs"]
fn live_quester_path_smoke() {
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"));
    let quest = std::env::var("QUESTER_PATH_QUEST").expect("QUESTER_PATH_QUEST names the Path");
    let seeds_path =
        PathBuf::from(std::env::var_os("QUESTER_PATH_SEEDS").expect("QUESTER_PATH_SEEDS"));
    let text = std::fs::read_to_string(&seeds_path)
        .unwrap_or_else(|error| panic!("read {}: {error}", seeds_path.display()));
    let seeds = parse_seeds(&text).unwrap_or_else(|error| panic!("{error}"));
    let row = smoke_row(&quest).unwrap_or_else(|error| panic!("{error}"));
    let before = registry::source();
    let source = resolve_source(&quest).unwrap_or_else(|error| panic!("{error}"));
    let loadout = seeds_loadout(seeds.loadout.as_ref()).unwrap_or_else(|error| panic!("{error}"));
    let extra_items: Vec<(&str, i32)> = seeds
        .extra_items
        .iter()
        .map(|(item, qty)| (item.as_str(), *qty))
        .collect();
    let seed_vars: Vec<(&str, i32)> = seeds
        .seed_vars
        .iter()
        .map(|(varp, value)| (varp.as_str(), *value))
        .collect();
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

    fn cook_cell(seeds: &Seeds) -> Cell {
        let row = smoke_row("cook").unwrap();
        assert_eq!(row.display, "Cook's Assistant");
        let loadout = seeds_loadout(seeds.loadout.as_ref()).unwrap();
        assert!(loadout.is_none());
        let extra_items: Vec<(&str, i32)> = seeds
            .extra_items
            .iter()
            .map(|(item, qty)| (item.as_str(), *qty))
            .collect();
        let seed_vars: Vec<(&str, i32)> = seeds
            .seed_vars
            .iter()
            .map(|(varp, value)| (varp.as_str(), *value))
            .collect();
        smoke_cell(row, seeds, loadout, &extra_items, &seed_vars).unwrap()
    }

    #[test]
    fn smoke_seeds_parse_and_build_the_embedded_cook_cell() {
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
        assert!(parse_seeds(r#"{"stage": "cook:0", "stand": [0, 0]}"#).is_err());
        let mut seeds = parse_seeds(COOK_SEEDS).unwrap();
        seeds.mode = Some("speedrun".into());
        assert!(seeds_loadout(seeds.loadout.as_ref()).is_ok());
        let row = smoke_row("cook").unwrap();
        let empty: Vec<(&str, i32)> = Vec::new();
        assert!(smoke_cell(row, &seeds, None, &empty, &empty).is_err());
        let seeds = parse_seeds(
            &COOK_SEEDS.replace(r#""loadout": null"#, r#""loadout": {"standard": "plate"}"#),
        )
        .unwrap();
        assert!(seeds_loadout(seeds.loadout.as_ref()).is_err());
    }

    #[test]
    fn smoke_cell_serves_a_folder_cook_through_the_registry() {
        let root = std::env::temp_dir().join(format!(
            "274bot-path-smoke-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("cook.json"),
            script::quester::compile::path_bytes("cook").unwrap(),
        )
        .unwrap();
        let before = registry::source();
        registry::set_source(FolderSource {
            enabled: true,
            folder: root.clone(),
        });
        let selected = FamilyPreparation::run(|_| {
            api::game_data::for_revision(ClientRevision::R289).expect("selected 289 data")
        })
        .unwrap()
        .join()
        .unwrap();
        let data = Arc::clone(&selected);
        let loaded = FamilyPreparation::run(move |worker| registry::reload(&data, worker))
            .unwrap()
            .join()
            .unwrap()
            .unwrap();
        let bytes = loaded.bytes("cook").expect("folder serves cook");
        assert_eq!(loaded.path_source("cook").label(), "folder");
        registry::set_source(before);
        let path: script::quester::path::PathDocument =
            serde_json::from_slice(bytes.as_ref()).unwrap();
        assert_eq!(path.id.0.as_ref(), "cook");
        let seeds = parse_seeds(COOK_SEEDS).unwrap();
        let cell = cook_cell(&seeds);
        assert_eq!(cell.quest, "cook");
        std::fs::remove_dir_all(&root).ok();
    }
}
