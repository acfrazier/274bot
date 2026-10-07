//! The Knight's Sword (`squire`) live qualification cells. Ignored unless LIVE=1;
//! environment and evidence layout in `common/quester_live.rs`.
#![cfg(feature = "live-harness")]

#[path = "common/quester_live.rs"]
mod quester_live;

use api::snapshot::{GameSnapshot, WorldTile};
use quester_live::{Cell, Mode, ObserveTick};
use scenario::{ScriptInjectValue, ScriptSettingInject};
use serde_json::{json, Value};
use std::time::Duration;

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

const PORTRAIT_STAND: WorldTile = WorldTile {
    x: 2985,
    z: 3335,
    level: 2,
};
const SIR_VYVIN_TYPE: usize = 605;
const SQUIRE_VARP_ID: i32 = 122;
const KNIGHTS_PORTRAIT_ID: i32 = 666;
const VYVIN_CLEAR_STEP: &str = "wait-vyvin-clear";
const VYVIN_SEARCH_STEP: &str = "search-vyvin-cupboard";
const PORTRAIT_DEADLINE: Duration = Duration::from_secs(240);

fn room_observation(snapshot: &GameSnapshot, event: &str) -> Value {
    let player_tile = snapshot.tile();
    let sir_vyvin = snapshot
        .npcs()
        .iter()
        .filter(|npc| npc.r#type == Some(SIR_VYVIN_TYPE))
        .map(|npc| {
            let same_level = player_tile.is_some_and(|(_, _, level)| level == npc.tile.level);
            let chebyshev =
                player_tile.map(|(x, z, _)| (npc.tile.x - x).abs().max((npc.tile.z - z).abs()));
            json!({
                "npc_type": SIR_VYVIN_TYPE,
                "name": npc.name.as_deref(),
                "index": npc.index,
                "tile": {
                    "x": npc.tile.x,
                    "z": npc.tile.z,
                    "level": npc.tile.level,
                },
                "same_level": same_level,
                "chebyshev": chebyshev,
                "adjacent_radius_1": same_level && chebyshev.is_some_and(|distance| distance <= 1),
            })
        })
        .collect::<Vec<_>>();
    let sir_vyvin_adjacent = sir_vyvin
        .iter()
        .any(|npc| npc["adjacent_radius_1"].as_bool() == Some(true));
    json!({
        "event": event,
        "tick": snapshot.tick(),
        "player_tile": player_tile.map(|(x, z, level)| json!({"x": x, "z": z, "level": level})),
        "sir_vyvin_visible": !sir_vyvin.is_empty(),
        "sir_vyvin_adjacent_radius_1": sir_vyvin_adjacent,
        "sir_vyvin": sir_vyvin,
    })
}

fn observe_squire_room_start(snapshot: &GameSnapshot) -> Result<Value, String> {
    if snapshot.tile() != Some((PORTRAIT_STAND.x, PORTRAIT_STAND.z, PORTRAIT_STAND.level)) {
        return Err(format!(
            "stage-5 fixture stood at {:?}, expected {PORTRAIT_STAND:?}",
            snapshot.tile()
        ));
    }
    // Squire's permanent varp is not transmitted to the client. A pre-Start
    // getvar reply proves the server-side seed without trusting zeroed cache data.
    let varp_witness = snapshot
        .chat_lines()
        .iter()
        .find(|line| line.text == "get squire: 5")
        .ok_or("stage-5 fixture lacks the server's get squire: 5 witness")?;
    let inventory = snapshot
        .inventory()
        .iter()
        .map(|item| {
            json!({
                "id": item.def.id,
                "name": item.def.name.as_deref(),
                "count": item.count,
            })
        })
        .collect::<Vec<_>>();
    let portrait_count = snapshot
        .inventory()
        .iter()
        .filter(|item| item.def.id == KNIGHTS_PORTRAIT_ID)
        .map(|item| item.count)
        .sum::<i32>();
    if portrait_count != 0 {
        return Err(format!(
            "stage-5 fixture unexpectedly started with {portrait_count} Knights Portraits"
        ));
    }
    if snapshot.inventory().len() != 1
        || !snapshot
            .inventory()
            .iter()
            .any(|item| item.def.id == 995 && item.count == 300)
    {
        return Err(format!(
            "stage-5 fixture requires only the Path's 300-coin float: {inventory:?}"
        ));
    }
    let room = room_observation(snapshot, "start");
    Ok(json!({
        "event": "squire-stage-5-room-start",
        "quest_varp": {"index": SQUIRE_VARP_ID, "value": 5, "server_witness": varp_witness.text},
        "inventory": inventory,
        "knights_portrait_id": KNIGHTS_PORTRAIT_ID,
        "knights_portrait_count": portrait_count,
        "sir_vyvin_adjacent_at_start": room["sir_vyvin_adjacent_radius_1"],
        "room": room,
    }))
}

fn squire_wait_tick_observer() -> ObserveTick {
    let mut last_tick = None;
    Box::new(move |snapshot, status| {
        let status = status?;
        if quester_live::status_stage(status, QUEST).as_deref() != Some("squire:5") {
            return None;
        }
        let logged_status = quester_live::status_json(status);
        let fields = logged_status.get("fields")?;
        let step_id = fields.get("step_id").and_then(Value::as_str);
        let child_step_id = fields.get("child_step_id").and_then(Value::as_str);
        let is_clear_wait =
            step_id == Some(VYVIN_CLEAR_STEP) || child_step_id == Some(VYVIN_CLEAR_STEP);
        let is_search_wait =
            step_id == Some(VYVIN_SEARCH_STEP) || child_step_id == Some(VYVIN_SEARCH_STEP);
        let observed_step = if is_clear_wait {
            VYVIN_CLEAR_STEP
        } else if is_search_wait {
            VYVIN_SEARCH_STEP
        } else {
            return None;
        };
        let tick = snapshot.tick();
        if last_tick == Some(tick) {
            return None;
        }
        last_tick = Some(tick);
        Some(json!({
            "event": "squire-stage-5-wait-tick",
            "tick": tick,
            "step_id": observed_step,
            "status": logged_status,
            "room": room_observation(snapshot, "wait-tick"),
        }))
    })
}

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

#[test]
#[ignore = "requires LIVE=1 and the shared local 289 engine; see common/quester_live.rs"]
fn squire_stage_5_room_portrait() {
    let mut cell = cell(
        "stage-squire-5-room-portrait",
        5,
        PORTRAIT_STAND,
        &[("coins", 300)],
        stage("squire:6"),
    );
    // Observe the real empty bank before Start, as earlier quest stages do.
    // Otherwise the authored portrait withdrawal must leave the room to scan it.
    let stand_index = cell
        .scenario
        .steps
        .iter()
        .position(|step| step.name == "stand at the quest start")
        .unwrap();
    cell.scenario.steps.insert(
        stand_index + 1,
        scenario::Step {
            name: "prove the server-side Squire stage before Start",
            kind: scenario::StepKind::Perform {
                send: Box::new(|client, _| api::interact::cheat(client, "getvar squire").is_sent()),
            },
            wait: scenario::Wait {
                arm: scenario::Proof::Chat {
                    needle: "get squire: 5",
                },
                budget_ticks: 40,
            },
        },
    );
    cell.scenario.steps.splice(
        stand_index..stand_index,
        [
            scenario::Step {
                name: "stand at the real bank before the portrait fixture",
                kind: scenario::StepKind::Perform {
                    send: Box::new(|client, _| {
                        api::interact::cheat(client, &api::interact::tele_args(0, 3092, 3243))
                            .is_sent()
                    }),
                },
                wait: scenario::Wait {
                    arm: scenario::Proof::Arrived {
                        x: 3092,
                        z: 3243,
                        level: 0,
                    },
                    budget_ticks: 100,
                },
            },
            scenario::Step {
                name: "open the real bank before the portrait fixture",
                kind: scenario::StepKind::Perform {
                    send: Box::new(|client, snapshot| {
                        matches!(
                            api::interact::Interactions::new(snapshot, client).open_booth_at(
                                WorldTile {
                                    x: 3091,
                                    z: 3243,
                                    level: 0
                                },
                                2213
                            ),
                            api::interact::SendResult::Sent { .. }
                        )
                    }),
                },
                wait: scenario::Wait {
                    arm: scenario::Proof::SideTabAvailable { index: 3 },
                    budget_ticks: 100,
                },
            },
            scenario::Step {
                name: "observe no banked portrait before Start",
                kind: scenario::StepKind::Await {
                    evidence: "real bank loaded with no portrait",
                    ready: Box::new(|snapshot| {
                        snapshot.bank_component_id() >= 0
                            && snapshot.bank_loaded()
                            && !snapshot
                                .bank()
                                .iter()
                                .any(|item| item.def.id == KNIGHTS_PORTRAIT_ID && item.count > 0)
                    }),
                },
                wait: scenario::Wait {
                    arm: scenario::Proof::SideTabAvailable { index: 3 },
                    budget_ticks: 100,
                },
            },
            scenario::Step {
                name: "close the real bank before entering Vyvin's room",
                kind: scenario::StepKind::Perform {
                    send: Box::new(|client, snapshot| {
                        matches!(
                            api::interact::Interactions::new(snapshot, client).close_modal(),
                            api::interact::SendResult::Sent { .. }
                        )
                    }),
                },
                wait: scenario::Wait {
                    arm: scenario::Proof::BankClosed,
                    budget_ticks: 100,
                },
            },
        ],
    );
    cell.scenario.settings.deadline = PORTRAIT_DEADLINE;
    cell.observe_start = Some(Box::new(observe_squire_room_start));
    quester_live::run_observed(
        cell,
        Box::new(|snapshot, _, _| {
            let count = snapshot
                .inventory()
                .iter()
                .filter(|item| item.def.id == KNIGHTS_PORTRAIT_ID)
                .map(|item| item.count)
                .sum::<i32>();
            Ok((count > 0).then(|| {
                json!({
                    "portrait_count": count,
                    "tick": snapshot.tick(),
                    "room": room_observation(snapshot, "portrait-obtained"),
                })
            }))
        }),
        squire_wait_tick_observer(),
    )
    .unwrap();
}
