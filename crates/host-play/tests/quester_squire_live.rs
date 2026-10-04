//! The Knight's Sword (`squire`) live qualification cells. Ignored unless LIVE=1;
//! environment and evidence layout in `common/quester_live.rs`.
#![cfg(feature = "live-harness")]

#[path = "common/quester_live.rs"]
mod quester_live;

use api::snapshot::WorldTile;
use quester_live::{Cell, Mode};
use scenario::{ScriptInjectValue, ScriptSettingInject};

const QUEST: &str = "squire";
const DISPLAY: &str = "The Knight's Sword";
const SETTINGS: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "quests",
    value: ScriptInjectValue::StrList(&[QUEST]),
}];

fn cell(
    label: &str,
    stage: i32,
    stand: WorldTile,
    items: &'static [(&'static str, i32)],
    mode: Mode,
) -> Cell {
    let mut scenario =
        scenario::quester_stage("quester_squire", DISPLAY, "squire", stage, items, stand);
    let seed = scenario
        .steps
        .iter()
        .position(|step| step.name == "reset quest stage")
        .unwrap();
    scenario.steps.insert(
        seed,
        scenario::Step {
            name: "stage the squire skill profile",
            kind: scenario::StepKind::Perform {
                send: Box::new(|c, _| {
                    for cmd in [
                        "setstat mining 15",
                        "setstat smithing 15",
                        "setstat cooking 10",
                    ] {
                        let _ = api::interact::cheat(c, cmd);
                    }
                    true
                }),
            },
            wait: scenario::Wait {
                arm: scenario::Proof::Stat { id: 14, min: 15 },
                budget_ticks: 80,
            },
        },
    );
    scenario.settings.script_settings_inject = Some(SETTINGS);
    Cell {
        quest: QUEST,
        display: DISPLAY,
        label: label.to_owned(),
        start_settings: scenario::settings_inject_map(Some(SETTINGS)).unwrap(),
        scenario,
        mode,
        observe_start: None,
    }
}

fn stage(next: &str) -> Mode {
    Mode::Stage {
        expect: vec![next.to_owned()],
    }
}

const THURGO: WorldTile = WorldTile {
    x: 3001,
    z: 3146,
    level: 0,
};
const SQUIRE: WorldTile = WorldTile {
    x: 2977,
    z: 3344,
    level: 0,
};

#[test]
#[ignore = "requires LIVE=1 and the shared local 289 engine; see common/quester_live.rs"]
fn squire_stage_3_thurgo_sword() {
    quester_live::run(cell("stage-squire-3", 3, THURGO, &[], stage("squire:4"))).unwrap();
}

#[test]
#[ignore = "requires LIVE=1 and the shared local 289 engine; see common/quester_live.rs"]
fn squire_stage_4_ask_picture() {
    quester_live::run(cell("stage-squire-4", 4, SQUIRE, &[], stage("squire:5"))).unwrap();
}
