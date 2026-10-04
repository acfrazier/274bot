//! Vampire Slayer qualification over the shared fixture and live runner.
#![cfg(feature = "live-harness")]
#[path = "common/f2pd_fixture.rs"]
mod fixture;
#[path = "common/quester_live.rs"]
mod quester_live;

use api::snapshot::WorldTile;
use quester_live::{Cell, Mode};

fn cell(label: &str, stage: &str, mode: Mode) -> Cell {
    fixture::cell(fixture::Case {
        quest: "vampire",
        display: "Vampire Slayer",
        label,
        stage,
        loadout: "melee",
        items: &[("coins", 200)],
        stand: WorldTile {
            x: 3098,
            z: 3268,
            level: 0,
        },
        mode,
        fixture_varp: None,
    })
}

#[test]
#[ignore = "requires LIVE=1 and the shared local 289 engine"]
fn vampire_stage_0() {
    quester_live::run(cell("stage-0", "vampire:0", fixture::stage("vampire:1"))).unwrap();
}
#[test]
#[ignore = "requires LIVE=1 and the shared local 289 engine"]
fn vampire_stage_1() {
    quester_live::run(cell("stage-1", "vampire:1", fixture::stage("vampire:2"))).unwrap();
}
#[test]
#[ignore = "requires LIVE=1 and the shared local 289 engine"]
fn vampire_stage_2() {
    // A pre-completed fixture is skipped by the native queue. Starting at
    // stage 2 and waiting for lifecycle completion exercises stage 3's exit.
    let receipt = quester_live::run(cell(
        "stage-2",
        "vampire:2",
        Mode::Stage { expect: Vec::new() },
    ))
    .unwrap();
    let x = receipt["last_tile"][0].as_i64().unwrap();
    let z = receipt["last_tile"][1].as_i64().unwrap();
    assert_eq!(receipt["last_tile"][2], 0);
    assert!((x - 3115).abs() <= 2 && (z - 3357).abs() <= 2);
}
#[test]
#[ignore = "requires LIVE=1 and the shared local 289 engine"]
fn vampire_clean() {
    quester_live::run(cell("clean", "vampire:0", Mode::Clean)).unwrap();
}
#[test]
#[ignore = "requires LIVE=1 and the shared local 289 engine"]
fn vampire_resume() {
    quester_live::run(cell(
        "resume",
        "vampire:0",
        Mode::Restart {
            at: ["vampire:1".into(), "vampire:2".into()],
        },
    ))
    .unwrap();
}
#[test]
#[ignore = "requires LIVE=1 and the shared local 289 engine"]
fn vampire_death() {
    quester_live::run(cell(
        "death",
        "vampire:2",
        Mode::Death {
            at: "vampire:2".into(),
            step: "kill-count-draynor".into(),
        },
    ))
    .unwrap();
}

#[test]
fn vampire_path_compiles_against_selected_content() {
    let selected = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
    let quests = api::quest_facts::QuestCatalog::from_identity(selected.quest_identity()).unwrap();
    let document: script::quester::path::PathDocument =
        serde_json::from_str(script::quester::compile::VAMPIRE_JSON).unwrap();
    script::quester::compile::compile_uncached_for_test(&document, &selected, &quests).unwrap();
}
