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

fn default_crossing_start(snapshot: &GameSnapshot, hitpoints: i32) -> Result<Value, String> {
    if snapshot.tile() != Some((2778, 3401, 0)) {
        return Err(format!(
            "unexpected keep crossing start: {:?}",
            snapshot.tile()
        ));
    }
    for (index, expected) in [(1, 1), (3, hitpoints)] {
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
    Ok(json!({"cell": "Merlin keep stairs", "hitpoints": hitpoints,
        "seed": ["setvar arthur 3", "~clearinv", "tele 0 2778 3401", "minme", "~clearinv worn"],
        "quester_card_blanket_danger": false, "world_speed_command": false,
        "observed": crossing_observation(snapshot)}))
}

/// Request only: the guide declares the crossing, the global setting is off, so
/// survival admission enforces the 1% bound. The player is raised to Hitpoints
/// 99 so the route passes the bound and the walk is admitted.
#[test]
#[ignore = "requires Engine Q, CPU renderer, live catalog, and evidence directory"]
fn merlin_declared_keep_stairs() {
    run_keep_stairs(false, REQUEST_ONLY_HITPOINTS);
}

/// The global Danger routing setting overrides everything, Quester included:
/// the same crossing with no declaration in the guide and the setting on. The
/// player keeps the default 10 Hitpoints: the bound is informational and the
/// walk proceeds.
#[test]
#[ignore = "requires Engine Q, CPU renderer, live catalog, and evidence directory"]
fn merlin_keep_stairs_global_override_without_declaration() {
    run_keep_stairs(true, DEFAULT_HITPOINTS);
}

const DEFAULT_HITPOINTS: i32 = 10;
const REQUEST_ONLY_HITPOINTS: i32 = 99;

fn strip_declarations(value: &mut Value) {
    match value {
        Value::Object(map) => {
            if let Some(cross) = map.get_mut("cross").filter(|cross| cross.is_array()) {
                *cross = json!([]);
            }
            map.values_mut().for_each(strip_declarations);
        }
        Value::Array(items) => items.iter_mut().for_each(strip_declarations),
        _ => {}
    }
}

/// Serve Arthur from a folder whose stages and steps declare no crossings.
fn serve_arthur_without_declarations() {
    let (document, _) = scenario::quester::load_quester_path_document("arthur")
        .expect("the bundled Arthur Path loads");
    let mut value = serde_json::to_value(&document).expect("Arthur serializes");
    strip_declarations(&mut value);
    let folder = std::path::PathBuf::from(
        std::env::var_os("LIVE_EVIDENCE_DIR").expect("LIVE_EVIDENCE_DIR is required"),
    )
    .join("undeclared-arthur");
    std::fs::create_dir_all(&folder).expect("create the undeclared Arthur folder");
    std::fs::write(
        folder.join("arthur.json"),
        serde_json::to_vec_pretty(&value).expect("Arthur re-serializes"),
    )
    .expect("write the undeclared Arthur Path");
    scenario::quester::reload_quester_path_source("arthur", Some(&folder))
        .expect("the folder serves Arthur");
    let (served, source) =
        scenario::quester::load_quester_path_document("arthur").expect("the folder Arthur loads");
    assert_eq!(
        source, "folder",
        "Arthur must come from the stripped folder"
    );
    assert!(
        served
            .roles
            .iter()
            .flat_map(|role| &role.sequences)
            .all(|sequence| sequence.cross.is_empty()),
        "the served guide declares no stage crossings"
    );
}

fn run_keep_stairs(global_override_without_declaration: bool, hitpoints: i32) {
    assert!(
        std::env::var_os("QUESTER_TICK_MS").is_none(),
        "the shared Engine Q speed must remain unchanged"
    );
    let final_proof = Proof::Arrived {
        x: 2769,
        z: 3403,
        level: 2,
    };
    let label = if global_override_without_declaration {
        "merlin-global-override-undeclared-keep-stairs"
    } else {
        "merlin-declared-keep-stairs"
    };
    let source_before = script::quester::registry::source();
    if global_override_without_declaration {
        serve_arthur_without_declarations();
    }
    let mut scenario = scenario::quester_stage_with_watch(
        label,
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
    let mut fixture = vec![
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
    ];
    if hitpoints != DEFAULT_HITPOINTS {
        fixture.push(command(
            "raise Hitpoints for the request-only crossing",
            format!("setstat hitpoints {hitpoints}"),
            Proof::Stat {
                id: 3,
                min: hitpoints,
            },
        ));
    }
    scenario.steps.splice(
        start..start,
        fixture.into_iter().chain([Step {
            name: "observe the exact crossing fixture",
            kind: StepKind::Await {
                evidence: "default Hitpoints 10, Defence 1, no items, keep approach",
                ready: Box::new(move |snapshot| {
                    default_crossing_start(snapshot, hitpoints).is_ok()
                }),
                summary: None,
            },
            wait: Wait {
                arm: Proof::SideTabAvailable { index: 3 },
                budget_ticks: 100,
            },
        }]),
    );
    let watch = scenario
        .steps
        .last_mut()
        .expect("the shared stage builder supplies a watch");
    watch.name = if global_override_without_declaration {
        "observe the native undeclared stair crossing under the global override"
    } else {
        "observe the native declared-zone stair crossing"
    };
    watch.wait.arm = final_proof;
    scenario.proof = final_proof;
    scenario.settings.deadline = Duration::from_secs(180);
    // The global override is a user setting: with it on, the walk needs no
    // declaration at all.
    scenario.settings.nav.allow_danger_zones = global_override_without_declaration;
    let cell = Cell {
        quest: "arthur",
        display: "Merlin's Crystal",
        label: label.into(),
        scenario,
        start_settings: Map::from_iter([
            ("quests".into(), json!(["arthur"])),
            ("include_draft_guides".into(), json!(true)),
        ]),
        mode: Mode::Stage {
            expect: vec!["arthur:4".into()],
        },
        observe_start: Some(Box::new(move |snapshot| {
            default_crossing_start(snapshot, hitpoints)
        })),
    };
    let outcome = quester_live::run_observed(
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
    );
    script::quester::registry::set_source(source_before);
    outcome.unwrap();
}
