//! Live proof of content-derived Black Knights Fortress disguise doors and push-wall routing.
//! The account is seeded outside the fortress; every ingress movement uses the production router and Traveller.
use std::cell::{Cell, RefCell};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use api::game_data::{self, WEARPOS_HAT, WEARPOS_TORSO};
use api::interact::{self, Interactions, SendResult};
use api::snapshot::{GameSnapshot, ReadContext, WorldTile};
use client::client::Client;
use host::Pump;
use host_play::{ProfileOptions, SharedClientTemplate};
use nav::router::{find_with, FindOptions, Leg, Route, RouteError};
use nav::transport::{TransportEdge, TransportKind};
use nav::traveller::{TravelEvent, TravelOptions, TravelOutcome, Traveller};
use nav::world::NavWorld;
use nav::WorldState;
use serde::Serialize;
use serde_json::{json, Value};

const ORIGIN: WorldTile = WorldTile {
    x: 3016,
    z: 3512,
    level: 0,
};
const INVENTORY_ONLY_TARGET: WorldTile = WorldTile {
    x: 3016,
    z: 3516,
    level: 0,
};
const GUARD_DOOR_AT: WorldTile = WorldTile {
    x: 3016,
    z: 3514,
    level: 0,
};
const FIRST_SECRET_WALL_AT: WorldTile = WorldTile {
    x: 3016,
    z: 3517,
    level: 0,
};
const DESTINATION: WorldTile = WorldTile {
    x: 3015,
    z: 3519,
    level: 0,
};
const GUARD_CONFIG: &str = "bkfortressdoor1";
const SECRET_WALL_CONFIG: &str = "bksecretdoor";
const HELM_ALIAS: &str = "bronze_med_helm";
const CHAINBODY_ALIAS: &str = "iron_chainbody";
const SETUP_TIMEOUT: Duration = Duration::from_secs(90);
const TRAVEL_TIMEOUT: Duration = Duration::from_secs(180);

#[derive(Debug, Clone, Serialize)]
struct TileProgress {
    tick: u32,
    tile: WorldTile,
}

#[derive(Debug, Clone, Serialize)]
struct TransportAttemptRecord {
    tick: u32,
    kind: String,
    expected_id: i32,
    actual_id: i32,
    target: WorldTile,
    option: i32,
    refusal: Option<String>,
    tile_at_attempt: Option<WorldTile>,
}

fn required_path(name: &str) -> PathBuf {
    PathBuf::from(std::env::var_os(name).unwrap_or_else(|| panic!("{name} is required")))
}

fn live_profile_options() -> ProfileOptions {
    ProfileOptions {
        profile: Some("local-289".into()),
        revision: Some("289".into()),
        host: Some("127.0.0.1".into()),
        asset_host: Some("127.0.0.1".into()),
        port: Some(44594),
        http_port: Some(1080),
        cache_dir: Some(required_path("BOT_CACHE_DIR")),
        unpack_dir: Some(required_path("BOT_NAV_SNAPSHOT_ROOT")),
        engine_dir: Some(required_path("WORLD_ENGINE_DIR")),
        nav_pack: Some(required_path("WORLD_NAV_PACK")),
        ..ProfileOptions::default()
    }
}

fn pump_once(client: &mut Client, snapshot: &mut GameSnapshot, pump: &mut Pump) {
    client.mainloop();
    host::publish_snapshot(snapshot, client, pump.drain_client(client));
}

fn wait_for(
    client: &mut Client,
    snapshot: &mut GameSnapshot,
    pump: &mut Pump,
    timeout: Duration,
    ready: impl Fn(&GameSnapshot) -> bool,
) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        pump_once(client, snapshot, pump);
        if ready(snapshot) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn inventory_count(snapshot: &GameSnapshot, id: i32) -> i32 {
    snapshot
        .inv()
        .iter()
        .filter(|&&(item_id, count)| item_id == id && count > 0)
        .map(|&(_, count)| count)
        .sum()
}

fn equipment_count(snapshot: &GameSnapshot, id: i32) -> i32 {
    snapshot
        .equipment()
        .iter()
        .filter(|item| item.def.id == id && item.count > 0)
        .map(|item| item.count)
        .sum()
}

fn equipment_slot(snapshot: &GameSnapshot, id: i32) -> Option<i32> {
    snapshot
        .equipment()
        .iter()
        .find(|item| item.def.id == id && item.count > 0)
        .map(|item| item.slot)
}

fn equipment_rows(snapshot: &GameSnapshot) -> Vec<Value> {
    snapshot
        .equipment()
        .iter()
        .filter(|item| item.count > 0)
        .map(|item| {
            json!({
                "id": item.def.id,
                "name": item.def.name,
                "count": item.count,
                "slot": item.slot,
            })
        })
        .collect()
}

fn write_receipt(path: &Path, receipt: &Value) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create evidence directory");
    }
    fs::write(
        path,
        serde_json::to_vec_pretty(receipt).expect("serialize receipt"),
    )
    .expect("write receipt");
}

/// Capture the CPU framebuffer as a portable pixmap with a matching JSON snapshot.
/// The operator may convert the PPM to PNG without rerunning the live cell.
fn save_frame(
    client: &mut Client,
    snapshot: &GameSnapshot,
    evidence: &Path,
    stem: &str,
    detail: Value,
) -> Result<(), String> {
    client.draw = true;
    let mut renderer = client::render::Renderer::new_prefer(client.config.lowmem, false);
    let output = renderer.mainredraw(client);
    let client::render::backend::FrameOutput::PixMap(pixels) = output else {
        client.draw = false;
        return Err("BOT_CPU=1 did not produce a CPU pixmap".into());
    };
    let mut ppm = format!("P6\n{} {}\n255\n", pixels.width, pixels.height).into_bytes();
    ppm.reserve(pixels.pixels.len() * 3);
    for pixel in pixels.pixels {
        ppm.extend_from_slice(&[
            ((pixel >> 16) & 0xff) as u8,
            ((pixel >> 8) & 0xff) as u8,
            (pixel & 0xff) as u8,
        ]);
    }
    client.draw = false;
    fs::write(evidence.join(format!("{stem}.ppm")), ppm)
        .map_err(|error| format!("write {stem}.ppm: {error}"))?;
    let paired = json!({
        "ingame": snapshot.ingame(),
        "attached": snapshot.attached(),
        "scene_state": snapshot.scene_state(),
        "tick": snapshot.tick(),
        "tile": snapshot.tile(),
        "inventory": snapshot.inv(),
        "equipment": equipment_rows(snapshot),
        "stats": snapshot.stats(),
        "detail": detail,
    });
    fs::write(
        evidence.join(format!("{stem}.json")),
        serde_json::to_vec_pretty(&paired).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("write {stem}.json: {error}"))?;
    Ok(())
}

fn find_route(
    world: &NavWorld,
    from: WorldTile,
    to: WorldTile,
    state: &WorldState,
) -> Result<Route, RouteError> {
    find_with(
        &world.collision,
        &world.graph,
        from,
        to,
        FindOptions {
            allow_teleports: false,
            allow_wilderness: false,
            allow_bank_fetch: false,
            ..FindOptions::default()
        },
        state,
    )
}

fn planned_edge(route: &Route, id: i32, at: WorldTile) -> Option<TransportEdge> {
    route.legs.iter().find_map(|leg| match leg {
        Leg::Transport { edge } if edge.loc_id == id && edge.at == at => Some(edge.as_ref().clone()),
        _ => None,
    })
}

fn logout_and_wait(
    client: &mut Client,
    snapshot: &mut GameSnapshot,
    pump: &mut Pump,
) -> bool {
    if !snapshot.ingame() {
        return true;
    }
    let ifaces = Arc::clone(&client.ifaces);
    interact::logout(client, &ifaces)
        && wait_for(client, snapshot, pump, Duration::from_secs(30), |s| !s.ingame())
}

fn attempt_for<'a>(
    attempts: &'a [TransportAttemptRecord],
    id: i32,
) -> Option<&'a TransportAttemptRecord> {
    attempts.iter().find(|attempt| {
        attempt.expected_id == id
            && attempt.actual_id == id
            && attempt.refusal.is_none()
    })
}

fn endpoint_observed_after(
    progress: &[TileProgress],
    attempt: &TransportAttemptRecord,
    edge: &TransportEdge,
) -> bool {
    progress
        .iter()
        .any(|step| step.tick > attempt.tick && step.tile == edge.to)
}

#[test]
#[ignore = "requires LIVE=1, BOT_CPU=1, disposable HOME, BOT_CACHE_DIR, BOT_NAV_SNAPSHOT_ROOT, WORLD_ENGINE_DIR, WORLD_NAV_PACK, NAV_QUEST_DOORS_EVIDENCE, and BOT_LIVE_NAME_PREFIX=nd"]
fn live_fortress_disguise_and_secret_wall_use_content_navigation() {
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"));
    assert_eq!(std::env::var("BOT_CPU").as_deref(), Ok("1"));
    assert!(std::env::var_os("HOME").is_some(), "set a disposable HOME");
    let prefix = std::env::var("BOT_LIVE_NAME_PREFIX").expect("BOT_LIVE_NAME_PREFIX");
    assert_eq!(prefix, "nd", "use this cell's account namespace");
    let evidence = required_path("NAV_QUEST_DOORS_EVIDENCE");
    fs::create_dir_all(&evidence).expect("create evidence directory");
    let receipt_path = evidence.join("receipt.json");
    let serial = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock")
        .as_millis();
    let account = format!("{prefix}{:09}", serial % 1_000_000_000);
    let mut receipt = json!({
        "scenario": "black-knights-fortress-disguise-and-secret-wall",
        "account": account,
        "server": {"host": "127.0.0.1", "game_port": 44594, "http_port": 1080},
        "profile": "local-289",
        "origin": ORIGIN,
        "inventory_only_target": INVENTORY_ONLY_TARGET,
        "guard_door_at": GUARD_DOOR_AT,
        "first_secret_wall_at": FIRST_SECRET_WALL_AT,
        "destination": DESTINATION,
        "engine_loaded": false,
        "tutorial_mainland_ready": false,
        "fresh_account_baseline": false,
        "inventory_only_no_path": false,
        "both_items_worn": false,
        "strict_all_gate": false,
        "route_uses_guard_door": false,
        "route_uses_secret_wall": false,
        "guard_server_crossing": false,
        "secret_wall_server_crossing": false,
        "fresh_tile_progress": false,
        "arrival_scene_ready": false,
        "logout_observed": false,
    });
    write_receipt(&receipt_path, &receipt);

    let profile = live_profile_options()
        .resolve(None)
        .expect("resolve Engine A R289 profile")
        .bind()
        .expect("bind Engine A R289 profile");
    let template = SharedClientTemplate::load(Arc::clone(&profile)).expect("client template");
    let world = template.world().expect("explicit WORLD_NAV_PACK");
    let identity = profile.nav_identity().expect("navigation identity");
    let data = game_data::for_revision(client::io::ClientRevision::R289)
        .expect("selected R289 game data");
    let helm = data.item_by_alias(HELM_ALIAS).expect("selected bronze helm fact");
    let chainbody = data
        .item_by_alias(CHAINBODY_ALIAS)
        .expect("selected chainbody fact");
    let guard = data
        .loc_by_config(GUARD_CONFIG)
        .expect("selected fortress guard-door fact");
    let secret_wall = data
        .loc_by_config(SECRET_WALL_CONFIG)
        .expect("selected secret-wall fact");
    assert_eq!(helm.id, 1139);
    assert_eq!(chainbody.id, 1101);
    assert_eq!(helm.wear_position, WEARPOS_HAT);
    assert_eq!(chainbody.wear_position, WEARPOS_TORSO);
    assert_eq!(guard.id, 2337);
    assert_eq!(secret_wall.id, 2341);
    receipt["engine_loaded"] = json!(true);
    receipt["nav_identity"] = json!({
        "pack_format": nav::pack::FORMAT_ID,
        "nav_sha256": identity.nav_sha256,
    });
    receipt["selected_facts"] = json!({
        "helmet": {"alias": HELM_ALIAS, "id": helm.id, "wear_position": helm.wear_position},
        "chainbody": {"alias": CHAINBODY_ALIAS, "id": chainbody.id, "wear_position": chainbody.wear_position},
        "guard": {"config": guard.config, "id": guard.id},
        "secret_wall": {"config": secret_wall.config, "id": secret_wall.id},
    });
    write_receipt(&receipt_path, &receipt);

    let mut client = template
        .prepare_client((serial % i32::MAX as u128) as i32, true)
        .expect("prepare CPU client");
    client.draw = false;
    client.maininit();
    assert!(!client.error_loading, "client failed to load");
    assert!(interact::login(&mut client, &account, &account, false), "local login refused");
    let mut snapshot = GameSnapshot::new();
    let mut pump = Pump::new();
    assert!(
        wait_for(&mut client, &mut snapshot, &mut pump, SETUP_TIMEOUT, |s| {
            s.ingame() && s.attached() && s.scene_state() == 2
        }),
        "initial account scene did not become ready: {:?}",
        snapshot.tile()
    );

    // Match the ordinary fresh-account tutorial hop and relog; no route hop is
    // used after the one account-local placement at ORIGIN.
    interact::mainland_hop(&mut client);
    assert!(interact::cheat(&mut client, "getvar tutorial").is_sent());
    assert!(
        wait_for(&mut client, &mut snapshot, &mut pump, SETUP_TIMEOUT, |s| {
            s.chat_lines()
                .iter()
                .any(|line| line.text.contains("get tutorial: 1000"))
        }),
        "tutorial skip was not acknowledged"
    );
    assert!(logout_and_wait(&mut client, &mut snapshot, &mut pump), "tutorial relog-out failed");
    assert!(interact::login(&mut client, &account, &account, false), "relogin refused");
    assert!(
        wait_for(&mut client, &mut snapshot, &mut pump, SETUP_TIMEOUT, |s| {
            s.ingame() && s.attached() && s.scene_state() == 2
        }),
        "post-tutorial account scene did not become ready"
    );
    receipt["tutorial_mainland_ready"] = json!(true);

    let fresh_account_baseline = inventory_count(&snapshot, helm.id) == 0
        && inventory_count(&snapshot, chainbody.id) == 0
        && equipment_count(&snapshot, helm.id) == 0
        && equipment_count(&snapshot, chainbody.id) == 0;
    receipt["fresh_account_baseline"] = json!(fresh_account_baseline);
    receipt["baseline"] = json!({
        "tile": snapshot.tile(),
        "inventory": snapshot.inv(),
        "equipment": equipment_rows(&snapshot),
        "scene_state": snapshot.scene_state(),
    });
    write_receipt(&receipt_path, &receipt);

    for skill in [
        "attack",
        "strength",
        "defence",
        "hitpoints",
        "ranged",
        "magic",
        "prayer",
    ] {
        assert!(
            interact::cheat(&mut client, &format!("setstat {skill} 40")).is_sent(),
            "setstat {skill} refused"
        );
    }
    assert!(interact::cheat(&mut client, &format!("give {HELM_ALIAS} 1")).is_sent());
    assert!(interact::cheat(&mut client, &format!("give {CHAINBODY_ALIAS} 1")).is_sent());
    interact::seed_at(&mut client, ORIGIN.level, ORIGIN.x, ORIGIN.z);
    assert!(
        wait_for(&mut client, &mut snapshot, &mut pump, SETUP_TIMEOUT, |s| {
            s.ingame()
                && s.attached()
                && s.scene_state() == 2
                && ReadContext::new(s).world_tile() == Some(ORIGIN)
                && inventory_count(s, helm.id) == 1
                && inventory_count(s, chainbody.id) == 1
                && WorldState::from_snapshot(s).combat_level == Some(51)
        }),
        "account seed timed out: tile={:?}, inventory={:?}, combat={:?}",
        snapshot.tile(),
        snapshot.inv(),
        WorldState::from_snapshot(&snapshot).combat_level
    );

    let inv_only_state = WorldState::from_snapshot(&snapshot).with_map_members(profile.map_members());
    let inventory_only_worn_empty =
        equipment_count(&snapshot, helm.id) == 0 && equipment_count(&snapshot, chainbody.id) == 0;
    let inventory_only_no_path = inventory_only_worn_empty
        && inventory_count(&snapshot, helm.id) == 1
        && inventory_count(&snapshot, chainbody.id) == 1
        && matches!(&inventory_only_result, Err(RouteError::NoPath));
    receipt["inventory_only_probe"] = json!({
        "from": ORIGIN,
        "to": INVENTORY_ONLY_TARGET,
        "inventory": snapshot.inv(),
        "worn_is_empty": inventory_only_worn_empty,
        "result": format!("{inventory_only_result:?}"),
        "no_path": inventory_only_no_path,
    });
    receipt["inventory_only_no_path"] = json!(inventory_only_no_path);
    let start_capture = save_frame(
        &mut client,
        &snapshot,
        &evidence,
        "00-start",
        json!({"phase":"inventory-only-at-origin", "route_probe":format!("{inventory_only_result:?}")}),
    );
    receipt["start_capture"] = json!(start_capture.as_ref().map(|_| "00-start.ppm").map_err(Clone::clone));
    write_receipt(&receipt_path, &receipt);

    let mut setup_failure = start_capture.as_ref().err().cloned();
    for id in [helm.id, chainbody.id] {
        if setup_failure.is_some() {
            break;
        }
        let sent = matches!(Interactions::new(&snapshot, &mut client).wear(id), SendResult::Sent { .. });
        if !sent {
            setup_failure = Some(format!("production Interactions::wear({id}) was refused"));
            break;
        }
        if !wait_for(&mut client, &mut snapshot, &mut pump, SETUP_TIMEOUT, |s| {
            s.ingame() && s.scene_state() == 2 && equipment_count(s, id) == 1
        }) {
            setup_failure = Some(format!("wearing selected item {id} timed out"));
        }
    }

    let both_items_worn = equipment_count(&snapshot, helm.id) == 1
        && equipment_count(&snapshot, chainbody.id) == 1
        && equipment_slot(&snapshot, helm.id) == Some(helm.wear_position)
        && equipment_slot(&snapshot, chainbody.id) == Some(chainbody.wear_position);
    receipt["both_items_worn"] = json!(both_items_worn);
    receipt["worn_equipment"] = json!(equipment_rows(&snapshot));

    let mut route_for_execution = None;
    let mut guard_edge = None;
    let mut wall_edge = None;
    let mut strict_all_gate = false;
    let mut route_uses_guard_door = false;
    let mut route_uses_secret_wall = false;
    let mut route_error = setup_failure;
    if route_error.is_none() && both_items_worn {
        let state = WorldState::from_snapshot(&snapshot).with_map_members(profile.map_members());
        match find_route(&world, ORIGIN, DESTINATION, &state) {
            Ok(route) => {
                let found_guard = planned_edge(&route, guard.id, GUARD_DOOR_AT);
                let found_wall = planned_edge(&route, secret_wall.id, FIRST_SECRET_WALL_AT);
                route_uses_guard_door = found_guard
                    .as_ref()
                    .is_some_and(|edge| edge.kind == TransportKind::Door);
                route_uses_secret_wall = found_wall
                    .as_ref()
                    .is_some_and(|edge| edge.kind == TransportKind::Door);
                strict_all_gate = found_guard.as_ref().is_some_and(|edge| {
                    let mut expected = vec![helm.id, chainbody.id];
                    expected.sort_unstable();
                    edge.kind == TransportKind::Door
                        && edge.worn_all_req == expected
                        && edge.worn_req.is_empty()
                });
                receipt["planned_route"] = json!({
                    "result": "found",
                    "legs": route.legs.iter().map(|leg| format!("{leg:?}")).collect::<Vec<_>>(),
                    "guard_edge": found_guard.as_ref().map(|edge| json!({
                        "at": edge.at, "to": edge.to, "loc_id": edge.loc_id,
                        "kind": format!("{:?}", edge.kind),
                        "worn_all_req": edge.worn_all_req,
                        "worn_req": edge.worn_req,
                    })),
                    "secret_wall_edge": found_wall.as_ref().map(|edge| json!({
                        "at": edge.at, "to": edge.to, "loc_id": edge.loc_id,
                        "kind": format!("{:?}", edge.kind),
                    })),
                    "strict_all_gate": strict_all_gate,
                    "uses_guard_door": route_uses_guard_door,
                    "uses_secret_wall": route_uses_secret_wall,
                });
                guard_edge = found_guard;
                wall_edge = found_wall;
                if route_uses_guard_door && route_uses_secret_wall && strict_all_gate {
                    route_for_execution = Some(route);
                } else {
                    route_error = Some("planned route did not prove the content guard and secret wall with the exact all-worn gate".into());
                }
            }
            Err(error) => {
                route_error = Some(format!("worn route search failed: {error:?}"));
                receipt["planned_route"] = json!({"result":"error", "error":format!("{error:?}")});
            }
        }
    } else if route_error.is_none() {
        route_error = Some("selected items were not both observed worn in their data-selected slots".into());
    }
    receipt["strict_all_gate"] = json!(strict_all_gate);
    receipt["route_uses_guard_door"] = json!(route_uses_guard_door);
    receipt["route_uses_secret_wall"] = json!(route_uses_secret_wall);
    if let Some(error) = route_error.as_ref() {
        receipt["route_error"] = json!(error);
    }
    write_receipt(&receipt_path, &receipt);

    let mut progress = Vec::new();
    let mut attempts = Vec::new();
    let mut travel_events = Vec::new();
    let mut leg_events = Vec::new();
    let mut travel_outcome = None;
    let route_start_tile = ReadContext::new(&snapshot).world_tile();
    let route_start_tick = snapshot.tick();
    if let Some(route) = route_for_execution {
        let mut traveller = Traveller::new();
        let observed_tile = Cell::new(route_start_tile);
        let mut options = TravelOptions {
            close_enough: 0,
            edges: Some(&world.graph.edges),
            on_event: Some(Box::new(|event| {
                if let TravelEvent::TransportAttempt {
                    tick,
                    kind,
                    expected_id,
                    actual_id,
                    target,
                    option,
                    refusal,
                } = &event
                {
                    attempts.push(TransportAttemptRecord {
                        tick: *tick,
                        kind: format!("{kind:?}"),
                        expected_id: *expected_id,
                        actual_id: *actual_id,
                        target: *target,
                        option: *option,
                        refusal: (*refusal).map(|reason| format!("{reason:?}")),
                        tile_at_attempt: observed_tile.get(),
                    });
                }
                travel_events.push(format!("{event:?}"));
            })),
            on_leg: Some(Box::new(|leg, phase| {
                leg_events.push(format!("{phase:?}: {leg:?}"));
            })),
            ..TravelOptions::default()
        };
        if let Some(tile) = route_start_tile {
            progress.push(TileProgress {
                tick: route_start_tick,
                tile,
            });
        }
        let deadline = Instant::now() + TRAVEL_TIMEOUT;
        let mut last_tick = None;
        let mut last_tile = route_start_tile;
        loop {
            pump_once(&mut client, &mut snapshot, &mut pump);
            let here = ReadContext::new(&snapshot).world_tile();
            observed_tile.set(here);
            if here != last_tile {
                if let Some(tile) = here {
                    progress.push(TileProgress {
                        tick: snapshot.tick(),
                        tile,
                    });
                }
                last_tile = here;
            }
            if last_tick != Some(snapshot.tick()) {
                last_tick = Some(snapshot.tick());
                if let Some(outcome) =
                    traveller.follow(&mut client, &snapshot, route.clone(), &mut options)
                {
                    travel_outcome = Some(outcome);
                    break;
                }
            }
            if Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        drop(options);
    }

    let successful_guard_attempt = guard_edge
        .as_ref()
        .and_then(|edge| attempt_for(&attempts, edge.loc_id));
    let successful_wall_attempt = wall_edge
        .as_ref()
        .and_then(|edge| attempt_for(&attempts, edge.loc_id));
    let guard_server_crossing = match (guard_edge.as_ref(), successful_guard_attempt) {
        (Some(edge), Some(attempt)) => {
            attempt.target == edge.at && endpoint_observed_after(&progress, attempt, edge)
        }
        _ => false,
    };
    let secret_wall_server_crossing = match (wall_edge.as_ref(), successful_wall_attempt) {
        (Some(edge), Some(attempt)) => {
            attempt.target == edge.at && endpoint_observed_after(&progress, attempt, edge)
        }
        _ => false,
    };
    let attempts_in_order = match (successful_guard_attempt, successful_wall_attempt) {
        (Some(guard_attempt), Some(wall_attempt)) => guard_attempt.tick < wall_attempt.tick,
        _ => false,
    };
    let fresh_tile_progress = progress
        .iter()
        .any(|step| step.tick > route_start_tick && Some(step.tile) != route_start_tile);
    let arrived = matches!(
        &travel_outcome,
        Some(TravelOutcome::Arrived { at }) if *at == DESTINATION
    );
    let arrival_scene_ready = snapshot.ingame() && snapshot.attached() && snapshot.scene_state() == 2;
    receipt["travel"] = json!({
        "route_start_tile": route_start_tile,
        "route_start_tick": route_start_tick,
        "tile_progress": progress,
        "transport_attempts": attempts,
        "events": travel_events,
        "legs": leg_events,
        "outcome": format!("{travel_outcome:?}"),
        "final_tile": snapshot.tile(),
        "ingame": snapshot.ingame(),
        "attached": snapshot.attached(),
        "scene_state": snapshot.scene_state(),
        "guard_attempt_tick": successful_guard_attempt.map(|attempt| attempt.tick),
        "secret_wall_attempt_tick": successful_wall_attempt.map(|attempt| attempt.tick),
        "attempts_in_order": attempts_in_order,
    });
    receipt["guard_server_crossing"] = json!(guard_server_crossing);
    receipt["secret_wall_server_crossing"] = json!(secret_wall_server_crossing);
    receipt["fresh_tile_progress"] = json!(fresh_tile_progress);
    receipt["arrival_scene_ready"] = json!(arrival_scene_ready);
    receipt["arrived"] = json!(arrived);

    let end_detail = json!({
        "phase": "route-end-before-logout",
        "outcome": format!("{travel_outcome:?}"),
        "destination": DESTINATION,
        "guard_server_crossing": guard_server_crossing,
        "secret_wall_server_crossing": secret_wall_server_crossing,
        "fresh_tile_progress": fresh_tile_progress,
        "route_error": route_error,
    });
    let end_capture = save_frame(&mut client, &snapshot, &evidence, "01-end", end_detail.clone());
    receipt["end_capture"] = json!(end_capture.as_ref().map(|_| "01-end.ppm").map_err(Clone::clone));
    if !arrived || !guard_server_crossing || !secret_wall_server_crossing || !fresh_tile_progress {
        let failure_capture = save_frame(
            &mut client,
            &snapshot,
            &evidence,
            "FAIL-route-proof",
            end_detail,
        );
        receipt["failure_capture"] = json!(failure_capture
            .as_ref()
            .map(|_| "FAIL-route-proof.ppm")
            .map_err(Clone::clone));
    }
    write_receipt(&receipt_path, &receipt);

    let logout_observed = logout_and_wait(&mut client, &mut snapshot, &mut pump);
    if !logout_observed {
        client.logout();
    }
    receipt["logout_observed"] = json!(logout_observed);
    receipt["post_logout"] = json!({
        "ingame": snapshot.ingame(),
        "scene_state": snapshot.scene_state(),
        "tile": snapshot.tile(),
    });
    write_receipt(&receipt_path, &receipt);
    println!("{}", serde_json::to_string_pretty(&receipt).expect("print receipt"));

    assert!(fresh_account_baseline, "account was not fresh before seeding: {receipt}");
    assert!(inventory_only_no_path, "inventory-only disguise opened a route: {receipt}");
    assert!(both_items_worn, "both data-selected disguise items were not worn: {receipt}");
    assert!(strict_all_gate, "guard route lacks the strict conjunction gate: {receipt}");
    assert!(route_uses_guard_door, "route did not use bkfortressdoor1 at the reported tile: {receipt}");
    assert!(route_uses_secret_wall, "route did not use the first secret push-wall: {receipt}");
    assert!(guard_server_crossing, "no server-settled progress across the guard door: {receipt}");
    assert!(secret_wall_server_crossing, "no server-settled progress across the secret wall: {receipt}");
    assert!(attempts_in_order, "guard and secret-wall attempts were absent or out of route order: {receipt}");
    assert!(fresh_tile_progress, "Traveller produced no fresh tile progress: {receipt}");
    assert!(arrived, "Traveller did not reach the selected inside destination: {receipt}");
    assert!(arrival_scene_ready, "arrival was not ingame with scene_state==2: {receipt}");
    assert!(start_capture.is_ok(), "start capture was not preserved: {receipt}");
    assert!(end_capture.is_ok(), "end capture was not preserved: {receipt}");
    assert!(logout_observed, "successful explicit logout was not observed: {receipt}");
}
