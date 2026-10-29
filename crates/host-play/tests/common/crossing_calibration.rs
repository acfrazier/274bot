//! No-food crossing measurements through the production Quester walk family.
//! The supplied folder Path contains ordered real walk targets and a bounded
//! observation hold. This proves route admission/survival, not quest completion.
use super::quester_live::{self, Cell, Mode};
use api::selected::RunKey;
use api::snapshot::{GameSnapshot, WorldTile};
use scenario::{Proof, Step, StepKind, Wait};
use script::native::{ScriptStatus, StatusValue};
use serde::{Deserialize, Serialize};
use serde_json::{Map, json};
use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Route {
    id: String,
    start: [i32; 3],
    targets: Vec<[i32; 3]>,
    cross: Vec<String>,
    defence: i32,
    sources: Vec<String>,
}

fn stat(snapshot: &GameSnapshot, index: i32) -> Option<(i32, i32)> {
    snapshot
        .stats()
        .iter()
        .find(|value| value.index == index)
        .map(|value| (value.base, value.effective))
}

fn step_id(status: &ScriptStatus) -> Option<&str> {
    match &status
        .fields
        .iter()
        .find(|field| field.key == "step_id")?
        .value
    {
        StatusValue::Text(value) => Some(value.as_ref()),
        _ => None,
    }
}

fn empty_kit(snapshot: &GameSnapshot) -> bool {
    snapshot.inventory().iter().all(|item| item.count == 0)
        && snapshot.equipment().iter().all(|item| item.count == 0)
}

fn fixture_matches(snapshot: &GameSnapshot, start: [i32; 3], hp: i32) -> bool {
    snapshot.ingame()
        && snapshot.scene_state() == 2
        && snapshot.tile() == Some((start[0], start[1], start[2]))
        && stat(snapshot, 1) == Some((1, 1))
        && stat(snapshot, 3) == Some((hp, hp))
        && empty_kit(snapshot)
}

fn command(name: &'static str, value: String, arm: Proof) -> Step {
    Step {
        name,
        kind: StepKind::Perform {
            send: Box::new(move |client, _| api::interact::cheat(client, &value).is_sent()),
        },
        wait: Wait {
            arm,
            budget_ticks: 100,
        },
    }
}

#[test]
#[ignore = "requires Engine Q, a cited ordered crossing Path, CPU capture and isolated HOME"]
fn live_quester_crossing_calibration() {
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"));
    assert_eq!(
        std::env::var("BOT_SERVER_PROFILE").as_deref(),
        Ok("local-289q")
    );
    assert_eq!(std::env::var("WORLD_GAME_PORT").as_deref(), Ok("44694"));
    assert!(std::env::var_os("QUESTER_TICK_MS").is_none());
    assert!(std::env::var_os("QUESTER_SUSTAIN_RUN").is_none());
    assert_ne!(
        std::env::var("QUESTER_ALLOW_DANGER_ZONES").as_deref(),
        Ok("1")
    );
    let route_path = PathBuf::from(std::env::var_os("QUEST_CROSSING_ROUTE").expect("route JSON"));
    let route: Route = serde_json::from_slice(&std::fs::read(&route_path).expect("read route"))
        .expect("strict route JSON");
    let hp: i32 = std::env::var("QUEST_CROSSING_HP")
        .expect("seeded HP")
        .parse()
        .expect("integer HP");
    let ticks: u64 = std::env::var("QUEST_CROSSING_TICKS")
        .expect("crossing tick bound")
        .parse()
        .expect("integer tick bound");
    assert!((1..=99).contains(&hp));
    assert!(ticks > 0);
    assert_eq!(route.defence, 1);
    assert!(!route.targets.is_empty());
    assert_eq!(route.targets.last(), Some(&route.start));
    assert!(route.targets.iter().any(|target| *target != route.start));
    assert!(!route.cross.is_empty() && route.cross.len() <= 8);
    assert!(!route.sources.is_empty());
    assert!(api::hostlog::install_sink(&super::CAMPAIGN_LOG_SINK));
    super::resolve_source("priest").expect("load the real ordered calibration Path");
    let start_tile = route.start;
    let mut scenario = scenario::quester_stage_with_watch(
        "quest-crossing-calibration",
        "The Restless Ghost",
        "priest",
        0,
        &[],
        WorldTile {
            x: start_tile[0],
            z: start_tile[1],
            level: start_tile[2],
        },
        ticks,
    );
    let start = scenario
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .expect("shared native Start");
    scenario.steps.splice(start..start, [
        command("reset to ordinary baseline stats", "minme".into(), Proof::StatAtMost { id: 1, max: 1 }),
        command("remove inventory before crossing", "~clearinv".into(), Proof::SideTabAvailable { index: 3 }),
        command("remove equipment before crossing", "~clearinv worn".into(), Proof::SideTabAvailable { index: 3 }),
        command("seed the candidate Hitpoints", format!("setstat hitpoints {hp}"), Proof::StatAtLeast { id: 3, min: hp }),
        Step {
            name: "prove exact no-food crossing fixture",
            kind: StepKind::Await {
                evidence: "observed full candidate HP, Defence 1, empty inventory/equipment and exact route origin",
                ready: Box::new(move |snapshot| fixture_matches(snapshot, start_tile, hp)),
                summary: None,
            },
            wait: Wait { arm: Proof::SideTabAvailable { index: 3 }, budget_ticks: 100 },
        },
    ]);
    scenario.settings.deadline =
        Duration::from_millis(ticks.saturating_add(400).saturating_mul(300));
    let label = format!("crossing-{}-hp-{hp}", route.id);
    let cell = Cell {
        quest: "priest",
        display: "The Restless Ghost",
        label,
        scenario,
        start_settings: Map::from_iter([
            ("quests".into(), json!(["priest"])),
            ("include_draft_guides".into(), json!(true)),
            ("allow_danger_zones".into(), json!(false)),
        ]),
        mode: Mode::Clean,
        observe_start: Some(Box::new(move |snapshot| {
            if !fixture_matches(snapshot, start_tile, hp) {
                return Err(
                    "candidate HP, Defence, empty kit or crossing origin changed before Start"
                        .into(),
                );
            }
            Ok(json!({"route": route, "hitpoints": hp, "defence": 1,
                "observed_tile": snapshot.tile(), "blanket_danger": false,
                "post_start_seed_commands": false, "world_speed_command": false}))
        })),
    };
    let mut pinned_run: Option<RunKey> = None;
    let mut lowest_hp = hp;
    let mut last_tick = None;
    let result = quester_live::run_observed(
        cell,
        Box::new(move |snapshot, status, _| {
            let Some((base, current)) = stat(snapshot, 3) else {
                return Ok(None);
            };
            if base != hp || stat(snapshot, 1) != Some((1, 1)) {
                return Err("the seeded HP base or Defence changed during the crossing".into());
            }
            if current <= 0 {
                return Err("the crossing killed the candidate player".into());
            }
            if !empty_kit(snapshot) {
                return Err("food, inventory or equipment appeared during the crossing".into());
            }
            lowest_hp = lowest_hp.min(current);
            let Some(status) = status else {
                return Ok(None);
            };
            if let Some(expected) = pinned_run {
                if status.run != expected {
                    return Err("the native run restarted during the crossing".into());
                }
            } else {
                pinned_run = Some(status.run);
            }
            let holding = step_id(status) == Some("observe-round-trip");
            if holding
                && snapshot.tile().is_some_and(|(x, z, level)| {
                    level == start_tile[2]
                        && (x - start_tile[0]).abs().max((z - start_tile[1]).abs()) <= 1
                })
            {
                return Ok(Some(
                    json!({"round_trip_complete": true, "seeded_hitpoints": hp,
                    "defence": 1, "minimum_observed_hitpoints": lowest_hp,
                    "ending_hitpoints": current, "ending_tile": snapshot.tile(),
                    "native_run": format!("{:?}", status.run), "no_food": true}),
                ));
            }
            Ok(None)
        }),
        Box::new(move |snapshot, status| {
            let tick = snapshot.tick();
            if last_tick == Some(tick) {
                return None;
            }
            last_tick = Some(tick);
            Some(json!({"tick": tick, "tile": snapshot.tile(),
                "hitpoints": stat(snapshot, 3), "defence": stat(snapshot, 1),
                "step_id": status.and_then(step_id),
                "run": status.map(|s| format!("{:?}", s.run))}))
        }),
    );
    let root = PathBuf::from(std::env::var_os("LIVE_EVIDENCE_DIR").expect("evidence directory"));
    let mut log = std::io::BufWriter::new(
        std::fs::File::create(root.join("session.jsonl")).expect("session log"),
    );
    for row in super::CAMPAIGN_LOGS
        .lock()
        .expect("captured product logs")
        .iter()
    {
        serde_json::to_writer(&mut log, row).expect("serialize product log");
        writeln!(log).expect("write product log newline");
    }
    log.flush().expect("flush product logs");
    result.expect("the candidate must produce a real admitted, surviving round trip");
}
