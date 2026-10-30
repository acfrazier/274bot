//! Engine Q route proof for the authored Merlin stage consent.
//! This cell uses the shared Quester pump, default combat stats, and no items.
//! The success boundary is the observed keep second floor, not Mordred's death.
#![cfg(all(feature = "live-harness", feature = "test-support"))]

#[path = "common/quester_live.rs"]
mod quester_live;

use api::snapshot::GameSnapshot;
use quester_live::{Cell, Mode};
use scenario::{Proof, Step, StepKind, Wait};
use serde_json::{json, Map, Value};
use std::time::Duration;

fn command(name: &'static str, command: String, arm: Proof) -> Step {
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

fn crossing_observation(snapshot: &GameSnapshot) -> Value {
    json!({
        "tick": snapshot.tick(),
        "tile": snapshot.tile(),
        "stats": snapshot.stats().iter().filter(|stat| [1, 3].contains(&stat.index))
            .map(|stat| json!({"index": stat.index, "base": stat.base,
                "effective": stat.effective})).collect::<Vec<_>>(),
        "inventory": snapshot.inventory().iter().filter(|item| item.count > 0)
            .map(|item| json!({"id": item.def.id, "count": item.count})).collect::<Vec<_>>()
    })
}

fn default_crossing_start(snapshot: &GameSnapshot) -> Result<Value, String> {
    if snapshot.tile() != Some((2778, 3401, 0)) {
        return Err(format!(
            "unexpected keep crossing start: {:?}",
            snapshot.tile()
        ));
    }
    for (index, expected) in [(1, 1), (3, 10)] {
        let stat = snapshot
            .stats()
            .iter()
            .find(|stat| stat.index == index)
            .ok_or_else(|| format!("crossing stat {index} is unobserved"))?;
        if stat.base != expected || stat.effective != expected {
            return Err(format!(
                "crossing stat {index}: expected {expected}, got {}/{}",
                stat.base, stat.effective
            ));
        }
    }
    if snapshot.inventory().iter().any(|item| item.count > 0)
        || snapshot.equipment().iter().any(|item| item.count > 0)
    {
        return Err("the default crossing cell must not seed items or equipment".into());
    }
    Ok(
        json!({"cell": "Merlin declared keep stairs", "seed": ["setvar arthur 3", "~clearinv", "tele 0 2778 3401", "minme", "~clearinv worn"],
        "blanket_danger": false, "world_speed_command": false,
        "observed": crossing_observation(snapshot)}),
    )
}

#[test]
#[ignore = "requires Engine Q, CPU renderer, live catalog, and evidence directory"]
fn merlin_declared_keep_stairs() {
    assert!(
        std::env::var_os("QUESTER_TICK_MS").is_none(),
        "the shared Engine Q speed must remain unchanged"
    );
    let final_proof = Proof::Arrived {
        x: 2769,
        z: 3403,
        level: 2,
    };
    let mut scenario = scenario::quester_stage_with_watch(
        "merlin-declared-keep-stairs",
        "Merlin's Crystal",
        "arthur",
        3,
        &[],
        api::WorldTile {
            x: 2778,
            z: 3401,
            level: 0,
        },
        400,
    );
    let start = scenario
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .expect("the shared stage builder supplies the native Start");
    scenario.steps.splice(
        start..start,
        [
            command(
                "reset the crossing to default player stats",
                "minme".into(),
                Proof::StatAtMost { id: 1, max: 1 },
            ),
            command(
                "remove fixture equipment",
                "~clearinv worn".into(),
                Proof::SideTabAvailable { index: 3 },
            ),
            Step {
                name: "observe the exact default crossing fixture",
                kind: StepKind::Await {
                    evidence: "default Hitpoints 10, Defence 1, no items, keep approach",
                    ready: Box::new(|snapshot| default_crossing_start(snapshot).is_ok()),
                    summary: None,
                },
                wait: Wait {
                    arm: Proof::SideTabAvailable { index: 3 },
                    budget_ticks: 100,
                },
            },
        ],
    );
    let watch = scenario
        .steps
        .last_mut()
        .expect("the shared stage builder supplies a watch");
    watch.name = "observe the native declared-zone stair crossing";
    watch.wait.arm = final_proof;
    scenario.proof = final_proof;
    scenario.settings.deadline = Duration::from_secs(180);
    let cell = Cell {
        quest: "arthur",
        display: "Merlin's Crystal",
        label: "merlin-declared-keep-stairs".into(),
        scenario,
        start_settings: Map::from_iter([
            ("quests".into(), json!(["arthur"])),
            ("include_draft_guides".into(), json!(true)),
        ]),
        mode: Mode::Stage {
            expect: vec!["arthur:4".into()],
        },
        observe_start: Some(Box::new(default_crossing_start)),
    };
    quester_live::run_observed(
        cell,
        Box::new(|snapshot, _, _| {
            if snapshot
                .stats()
                .iter()
                .any(|stat| stat.index == 3 && stat.effective <= 0)
            {
                return Err("the keep crossing killed the default-stat player".into());
            }
            Ok(snapshot
                .tile()
                .filter(|(x, z, level)| {
                    *level == 2 && (*x - 2769).abs() <= 3 && (*z - 3403).abs() <= 3
                })
                .map(|_| crossing_observation(snapshot)))
        }),
        Box::new(|snapshot, _| Some(crossing_observation(snapshot))),
    )
    .unwrap();
}
