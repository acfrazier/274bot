//! Live Lumbridge-stairs WalkTo proof through the native map arm.
//! Requires an explicitly authorized local engine/pack and writes its receipt even on route failure.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use api::interact;
use api::snapshot::GameSnapshot;
use client::client::Client;
use host::Pump;
use host_play::walk_map::{ActionKind, FocusToken, MapContext, MapModel};
use host_play::{ProfileOptions, SharedClientTemplate, SlotArm, WalkArms};
use nav::bank_fetch::BankRows;
use nav::map::identity::Digest;
use nav::router::FindOptions;
use nav::tile::Tile;
use nav::WorldState;
use serde_json::{json, Value};

const ORIGIN: (i32, i32, i32) = (3205, 3206, 0);
const STAIRS_ID: i32 = 1738;
const WHEEL_GOAL: Tile = Tile {
    x: 3209,
    z: 3212,
    level: 1,
};
// This wheel tile is blocked as a stand; the adjacent reachable stand is the
// native map Walk target and must be operable on the wheel.
const DESTINATION: Tile = Tile {
    x: 3209,
    z: 3213,
    level: 1,
};

fn pump_once(client: &mut Client, snapshot: &mut GameSnapshot, pump: &mut Pump) {
    client.mainloop();
    host::publish_snapshot(snapshot, client, pump.drain_client(client));
}

fn wait_for(
    client: &mut Client,
    snapshot: &mut GameSnapshot,
    pump: &mut Pump,
    ready: impl Fn(&GameSnapshot) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        pump_once(client, snapshot, pump);
        if ready(snapshot) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "scene readiness timed out at {:?}",
            snapshot.tile()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn write_receipt(path: &std::path::Path, receipt: &Value) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create receipt directory");
    }
    fs::write(
        path,
        serde_json::to_vec_pretty(receipt).expect("serialize receipt"),
    )
    .expect("write stairs live receipt");
    println!(
        "{}",
        serde_json::to_string_pretty(receipt).expect("print receipt")
    );
}

fn env_port(name: &str, default: u16) -> u16 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

#[test]
#[ignore = "requires LIVE=1, NAV_STAIRS_PACK, WORLD_ENGINE_DIR and NAV_STAIRS_RECEIPT; BOT_LIVE_NAME_PREFIX defaults to ns"]
fn live_lumbridge_stairs_walk_arm_reaches_operable_stand() {
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"));
    let prefix = std::env::var("BOT_LIVE_NAME_PREFIX").unwrap_or_else(|_| "ns".into());
    assert!(
        !prefix.is_empty(),
        "BOT_LIVE_NAME_PREFIX must be non-empty so generated accounts stay agent-owned"
    );
    let receipt_path =
        PathBuf::from(std::env::var_os("NAV_STAIRS_RECEIPT").expect("NAV_STAIRS_RECEIPT"));
    let serial = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock")
        .as_millis();
    let engine_dir = PathBuf::from(std::env::var_os("WORLD_ENGINE_DIR").expect("WORLD_ENGINE_DIR"));
    // The shared-engine convention: WORLD_GAME_PORT/WORLD_HTTP_PORT select
    // another local engine; the defaults are this fixture's original ones.
    let port = env_port("WORLD_GAME_PORT", 45594);
    let http_port = env_port("WORLD_HTTP_PORT", 2080);
    let options = ProfileOptions {
        profile: Some("local-289".into()),
        revision: Some("289".into()),
        host: Some("127.0.0.1".into()),
        asset_host: Some("127.0.0.1".into()),
        port: Some(port),
        http_port: Some(http_port),
        engine_dir: Some(engine_dir),
        nav_pack: Some(PathBuf::from(
            std::env::var_os("NAV_STAIRS_PACK").expect("NAV_STAIRS_PACK"),
        )),
        ..ProfileOptions::default()
    };
    let profile = options
        .resolve(None)
        .expect("resolve local-289 profile")
        .bind()
        .expect("bind profile");
    let name = format!("{prefix}{}", serial % 1_000_000_000);
    let template =
        SharedClientTemplate::load(Arc::clone(&profile)).expect("load shared client template");
    let world = template.world().expect("bundled world");
    let identity = profile.nav_identity().expect("navigation identity");
    let context = MapContext {
        focus: Some(FocusToken::capture(&SlotArm::new(1, false))),
        nav: Digest::from_hex(&identity.nav_sha256).expect("navigation digest"),
        overlay: None,
        generation: 1,
    };
    let client_id = (serial % 1_000_000_000) as i32;
    let mut client = template
        .prepare_client(client_id, true)
        .expect("prepare client");
    client.draw = false;
    client.maininit();
    assert!(!client.error_loading, "client failed to load");
    assert!(
        interact::login(&mut client, &name, &name, false),
        "local login refused"
    );
    let mut snapshot = GameSnapshot::new();
    let mut pump = Pump::new();
    wait_for(&mut client, &mut snapshot, &mut pump, |s| {
        s.ingame() && s.attached() && s.scene_state() == 2
    });

    // Tutorial seeding places the player on the near-side ground tile; no route hop is used.
    interact::seed_at(&mut client, ORIGIN.2, ORIGIN.0, ORIGIN.1);
    wait_for(&mut client, &mut snapshot, &mut pump, |s| {
        s.ingame() && s.attached() && s.scene_state() == 2 && s.tile() == Some(ORIGIN)
    });
    let initial_chat_sequence = snapshot
        .chat_lines()
        .iter()
        .map(|line| line.sequence)
        .max()
        .unwrap_or(0);
    let stairs = snapshot.locs().iter().find(|loc| loc.id == STAIRS_ID);
    let scene = snapshot.scene();
    let stairs_snapshot = stairs.map(|loc| {
        let from = api::snapshot::WorldTile {
            x: ORIGIN.0,
            z: ORIGIN.1,
            level: ORIGIN.2,
        };
        let operable = api::query::loc_approach::operable_tiles(loc, scene).unwrap_or_default();
        let scene_query = api::query::SceneQuery::new(scene, Some(from));
        let reachable_operable: Vec<_> = operable
            .iter()
            .copied()
            .filter(|tile| scene_query.can_reach(*tile, &api::query::SceneReachOptions::default()))
            .collect();
        json!({
            "id": loc.id, "tile": loc.tile, "name": loc.name,
            "footprint": [loc.footprint_width, loc.footprint_length],
            "force_approach": loc.force_approach, "angle": loc.angle, "shape": loc.shape,
            "actions": loc.actions,
            "can_operate_from_origin": api::query::loc_approach::can_operate_from(loc, scene, from),
            "operable_tiles": operable, "reachable_operable_tiles": reachable_operable,
        })
    });
    let origin_scene_diagnostic = json!({
        "scene_available": scene.available, "base": [scene.base_x, scene.base_z], "level": scene.level,
        "dimensions": [scene.width, scene.height], "origin": ORIGIN,
        "origin_in_scene": ORIGIN.0 >= scene.base_x && ORIGIN.1 >= scene.base_z
            && ORIGIN.0 < scene.base_x + scene.width && ORIGIN.1 < scene.base_z + scene.height,
    });

    let mut receipt = json!({
        "scenario": "lumbridge-stairs-loc1738", "account": name, "profile": "local-289",
        "server": {"host": "127.0.0.1", "game_port": port, "http_port": http_port},
        "origin": ORIGIN, "wheel_goal": [WHEEL_GOAL.x, WHEEL_GOAL.z, WHEEL_GOAL.level],
        "destination_stand": [DESTINATION.x, DESTINATION.z, DESTINATION.level],
        "destination_rationale": "wheel tile is blocked; route to adjacent operable stand",
        "scene_state": snapshot.scene_state(), "ingame": snapshot.ingame(),
        "stairs": stairs_snapshot, "scene_diagnostic": origin_scene_diagnostic,
        "route_legs": Value::Null, "route_ticks": Value::Null, "route_error": Value::Null,
        "chat": [], "tile_changes": [], "terminal_arm_outcome": Value::Null,
        "transport_attempt_state": "not exposed by the production WalkArm follow hook",
        "arrival": false,
    });

    let route_result = (|| {
        let mut model = MapModel::default();
        model.bind(context);
        let selected = model.select_tile(&world, DESTINATION);
        receipt["map_selected_tile"] = json!(selected.map(|tile| (tile.x, tile.z, tile.level)));
        let command = model.confirm(
            ActionKind::Walk,
            &context,
            Some(Tile {
                x: ORIGIN.0,
                z: ORIGIN.1,
                level: ORIGIN.2,
            }),
            FindOptions::default(),
        );
        let command = match command {
            Ok(command) => command,
            Err(error) => return Err(format!("map confirm: {error:?}")),
        };
        receipt["map_command_destination"] = json!({
            "x": command.destination().x,
            "z": command.destination().z,
            "level": command.destination().level,
        });
        let state = WorldState::from_snapshot(&snapshot).with_map_members(profile.map_members());
        let arms = WalkArms::default();
        let admission = host_play::admission::Admission::manual(
            command.options(),
            host_play::admission::capture(&snapshot, state.map_members, Default::default(), false),
            0,
            host_play::WalkGlobals::default(),
        );
        let route = match command.walk_on(
            &world,
            &context,
            &name,
            &state,
            &BankRows::default(),
            admission,
            &arms,
        ) {
            Ok(route) => route.route,
            Err(error) => return Err(format!("walk_on: {error:?}")),
        };
        receipt["route_legs"] = json!(format!("{:?}", route.legs));
        receipt["route_ticks"] = json!(route.ticks);
        let arm = Arc::clone(&arms.lock().expect("walk arms")[&name]);
        if route.dest
            != (api::snapshot::WorldTile {
                x: DESTINATION.x,
                z: DESTINATION.z,
                level: DESTINATION.level,
            })
        {
            return Err(format!(
                "native map changed the exact operable stand to {:?}",
                route.dest
            ));
        }
        let deadline = Instant::now() + Duration::from_secs(90);
        let mut last_tick = None;
        let mut last_tile = snapshot.tile();
        loop {
            pump_once(&mut client, &mut snapshot, &mut pump);
            if Some(snapshot.tick()) != last_tick {
                last_tick = Some(snapshot.tick());
                if let Some(here) = snapshot.tile() {
                    if Some(here) != last_tile {
                        receipt["tile_changes"]
                            .as_array_mut()
                            .expect("tile change array")
                            .push(json!({"tick":snapshot.tick(), "tile":here}));
                        last_tile = Some(here);
                    }
                    let terminal_step = host_play::step_walk_arm_follow(
                        &mut client,
                        &snapshot,
                        &mut arm.lock().expect("walk arm"),
                        Some(&world),
                        here,
                        state.map_members,
                        Some(&name),
                    );
                    let route_active = arm.lock().expect("walk arm").route.is_some();
                    if !route_active {
                        let arrived = here == (DESTINATION.x, DESTINATION.z, DESTINATION.level);
                        receipt["terminal_arm_outcome"] = json!({
                            "terminal_step_reported": terminal_step,
                            "route_cleared": true,
                            "tile": here,
                            "classified_arrival": arrived,
                        });
                        receipt["arrival"] = json!(arrived);
                        break;
                    }
                }
            }
            if snapshot.ingame() && snapshot.scene_state() == 2 {
                for line in snapshot
                    .chat_lines()
                    .iter()
                    .filter(|line| line.sequence > initial_chat_sequence)
                {
                    let lines = receipt["chat"].as_array_mut().expect("chat array");
                    if !lines.iter().any(|saved| saved["sequence"] == line.sequence) {
                        lines.push(json!({"sequence": line.sequence, "text": line.text, "type": line.type_, "username": line.username}));
                    }
                }
            }
            if Instant::now() >= deadline {
                receipt["terminal_arm_outcome"] = json!({"timeout": true, "route_still_active": arm.lock().expect("walk arm").route.is_some(), "tile": snapshot.tile()});
                return Err(format!("WalkArm timeout at {:?}", snapshot.tile()));
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        Ok(())
    })();

    // Collect the final chat ring as well, then persist evidence before any acceptance assertion.
    for line in snapshot
        .chat_lines()
        .iter()
        .filter(|line| line.sequence > initial_chat_sequence)
    {
        let lines = receipt["chat"].as_array_mut().expect("chat array");
        if !lines.iter().any(|saved| saved["sequence"] == line.sequence) {
            lines.push(json!({"sequence": line.sequence, "text": line.text, "type": line.type_, "username": line.username}));
        }
    }
    if let Err(error) = &route_result {
        receipt["route_error"] = json!(error);
    }
    receipt["final_tile"] = json!(snapshot.tile());
    receipt["final_scene_state"] = json!(snapshot.scene_state());
    receipt["final_ingame"] = json!(snapshot.ingame());
    let final_stand = snapshot
        .tile()
        .map(|(x, z, level)| api::snapshot::WorldTile { x, z, level });
    let final_wheel_operability = snapshot
        .locs()
        .iter()
        .find(|loc| {
            loc.tile.x == WHEEL_GOAL.x
                && loc.tile.z == WHEEL_GOAL.z
                && loc.tile.level == WHEEL_GOAL.level
        })
        .map(|loc| {
            final_stand.and_then(|stand| {
                api::query::loc_approach::can_operate_from(loc, snapshot.scene(), stand)
            })
        });
    receipt["wheel_loc_id_at_goal"] = json!(snapshot
        .locs()
        .iter()
        .find(|loc| loc.tile.x == WHEEL_GOAL.x
            && loc.tile.z == WHEEL_GOAL.z
            && loc.tile.level == WHEEL_GOAL.level)
        .map(|loc| loc.id));
    receipt["wheel_can_operate_from_final_stand"] = json!(final_wheel_operability);
    if route_result.is_ok() {
        receipt["wheel_stand_reached_by_production_follow"] = json!(
            receipt["arrival"] == true
                && receipt["final_tile"]
                    == json!([DESTINATION.x, DESTINATION.z, DESTINATION.level])
        );
    }
    write_receipt(&receipt_path, &receipt);
    client.logout();

    assert!(
        stairs_snapshot.is_some(),
        "stairs loc 1738 was absent from origin snapshot"
    );
    assert!(
        receipt["route_error"].is_null(),
        "route setup/follow failed: {}",
        receipt["route_error"]
    );
    assert_eq!(
        receipt["final_scene_state"], 2,
        "scene not ready at completion"
    );
    assert_eq!(
        receipt["final_ingame"], true,
        "client left game before completion"
    );
    assert_eq!(
        receipt["final_tile"],
        json!([DESTINATION.x, DESTINATION.z, DESTINATION.level]),
        "WalkArm did not arrive on exact reachable stand tile"
    );
    assert_eq!(
        receipt["arrival"], true,
        "WalkArm route terminated before exact stand destination"
    );
    assert_eq!(
        receipt["wheel_can_operate_from_final_stand"], true,
        "final stand cannot operate the wheel loc"
    );
}
