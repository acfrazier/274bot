//! Live proof of content-derived Black Knights Fortress disguise doors and push-wall routing.
//! The account is seeded outside the fortress; every ingress movement uses the production router and Traveller.
//! Phoenix journal proof and Black Arm refusal also exercise the real Brimhaven kitchen door.
use std::cell::Cell;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::task::Poll;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use api::game_data::{self, WEARPOS_HAT, WEARPOS_TORSO};
use api::interact::{self, Interactions, SendResult};
use api::local_state::{
    AccountKey, ContentScope, DomainEdit, DomainIdentity, DomainLease, ErrorCode, LocalStatePort,
    StoreVersion, SubmitReceipt, WriteSettlement,
};
use api::prereq_knowledge::{self, Fact, Observation, SessionMemory};
use api::quest_progress::{EvidenceProvider, EvidenceStamp, QuestListEvidenceProvider};
use api::selected::{ClientRevision, Truth};
use api::snapshot::{GameSnapshot, ReadContext, SnapshotView, WorldTile};
use client::client::Client;
use host::Pump;
use host_play::{mint_live_entries, mint_live_names, ProfileOptions, SharedClientTemplate};
use nav_executor::{TravelEvent, TravelOptions, TravelOutcome, Traveller};
use nav_planner::quest_gates::{QuestEvidence, QuestFamilyId};
use nav_planner::router::{find_with, FindOptions, Leg, Route, RouteError};
use nav_planner::transport::{TransportEdge, TransportKind};
use nav_planner::world::NavWorld;
use nav_planner::WorldState;
use parking_lot::Mutex;
use script::local_state::{SlotState, StatePort};
use script::native::{NativeTick, PrereqAllow, Script, ScriptFailure, ScriptFlow};
use script::quest_journal::{JournalMachine, ROOT_289, TITLE_289};
use script::quester::prereq::run::{JournalAttempt, PrereqBudget, PrereqRun, RunEntry};
use serde::Serialize;
use serde_json::{json, Value};
use vault::{Profile, ProfileSettings};

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
    x: 3016,
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
        port: Some(host_play::live_ports::game_port()),
        http_port: Some(host_play::live_ports::http_port()),
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

/// Capture the actual CPU framebuffer as PNG with a matching JSON snapshot.
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
    let mut rgb = Vec::with_capacity(pixels.pixels.len() * 3);
    for pixel in pixels.pixels {
        rgb.extend_from_slice(&[
            ((pixel >> 16) & 0xff) as u8,
            ((pixel >> 8) & 0xff) as u8,
            (pixel & 0xff) as u8,
        ]);
    }
    client.draw = false;
    let file = fs::File::create(evidence.join(format!("{stem}.png")))
        .map_err(|error| format!("create {stem}.png: {error}"))?;
    let mut encoder = png::Encoder::new(file, pixels.width as u32, pixels.height as u32);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|error| error.to_string())?;
    writer
        .write_image_data(&rgb)
        .map_err(|error| error.to_string())?;
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
        Leg::Transport { edge } if edge.loc_id == id && edge.at == at => {
            Some(edge.as_ref().clone())
        }
        _ => None,
    })
}

fn logout_and_wait(client: &mut Client, snapshot: &mut GameSnapshot, pump: &mut Pump) -> bool {
    if !snapshot.ingame() {
        return true;
    }
    let ifaces = Arc::clone(&client.ifaces);
    interact::logout(client, &ifaces)
        && wait_for(client, snapshot, pump, Duration::from_secs(30), |s| {
            !s.ingame()
        })
}

fn attempt_for(attempts: &[TransportAttemptRecord], id: i32) -> Option<&TransportAttemptRecord> {
    attempts.iter().find(|attempt| {
        attempt.expected_id == id && attempt.actual_id == id && attempt.refusal.is_none()
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
    let evidence_root = required_path("NAV_QUEST_DOORS_EVIDENCE");
    let serial = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock")
        .as_millis();
    let timestamp = format!("{serial}Z");
    let (account, password) = mint_live_entries(&mint_live_names(1))
        .into_iter()
        .next()
        .expect("mint one owned live account");
    let evidence = evidence_root.join(format!("NAV-QUEST-DOORS-1_fortress_{account}_{timestamp}"));
    fs::create_dir_all(&evidence).expect("create per-account evidence directory");
    let receipt_path = evidence.join(format!("{timestamp}_receipt.json"));
    let start_stem = format!("{timestamp}_00-start");
    let end_stem = format!("{timestamp}_01-end");
    let failure_stem = format!("{timestamp}_FAIL-route-proof");
    let mut receipt = json!({
        "scenario": "black-knights-fortress-disguise-and-secret-wall",
        "account": account,
        "server": {"host": "127.0.0.1", "game_port": host_play::live_ports::game_port(), "http_port": host_play::live_ports::http_port()},
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
    assert!(
        world.collision.standable(DESTINATION),
        "the proof destination must be a real stand, not the adjacent ladder's blocked anchor"
    );
    let identity = profile.nav_identity().expect("navigation identity");
    let data =
        game_data::for_revision(client::io::ClientRevision::R289).expect("selected R289 game data");
    let helm = data
        .item_by_alias(HELM_ALIAS)
        .expect("selected bronze helm fact");
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
        "pack_format": nav_planner::pack::FORMAT_ID,
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
    assert!(
        interact::login(&mut client, &account, &password, false),
        "local login refused"
    );
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
    assert!(
        logout_and_wait(&mut client, &mut snapshot, &mut pump),
        "tutorial relog-out failed"
    );
    assert!(
        interact::login(&mut client, &account, &password, false),
        "relogin refused"
    );
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
    // Account-local placement near the Wilderness opens the game's warning.
    // Close it through the ordinary native modal operation before navigation.
    let modals = snapshot.modals();
    if modals.main != -1 || modals.side != -1 || modals.chat != -1 || modals.tutorial != -1 {
        assert!(
            matches!(
                Interactions::new(&snapshot, &mut client).close_modal(),
                SendResult::Sent { .. }
            ),
            "native seed-warning dismissal refused"
        );
        assert!(
            wait_for(&mut client, &mut snapshot, &mut pump, SETUP_TIMEOUT, |s| {
                let modals = s.modals();
                s.ingame()
                    && s.scene_state() == 2
                    && modals.main == -1
                    && modals.side == -1
                    && modals.chat == -1
                    && modals.tutorial == -1
            }),
            "seed-warning dismissal did not settle"
        );
    }

    let inv_only_state =
        WorldState::from_snapshot(&snapshot).with_map_members(profile.map_members());
    let inventory_only_result = find_route(&world, ORIGIN, INVENTORY_ONLY_TARGET, &inv_only_state);
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
        &start_stem,
        json!({"phase":"inventory-only-at-origin", "route_probe":format!("{inventory_only_result:?}")}),
    );
    receipt["start_capture"] = json!(start_capture
        .as_ref()
        .map(|_| format!("{start_stem}.png"))
        .map_err(Clone::clone));
    write_receipt(&receipt_path, &receipt);

    let mut setup_failure = start_capture.as_ref().err().cloned();
    for id in [helm.id, chainbody.id] {
        if setup_failure.is_some() {
            break;
        }
        let sent = matches!(
            Interactions::new(&snapshot, &mut client).wear(id),
            SendResult::Sent { .. }
        );
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
                    let mut expected = [helm.id, chainbody.id];
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
        route_error =
            Some("selected items were not both observed worn in their data-selected slots".into());
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
    let arrival_scene_ready =
        snapshot.ingame() && snapshot.attached() && snapshot.scene_state() == 2;
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
    let end_capture = save_frame(
        &mut client,
        &snapshot,
        &evidence,
        &end_stem,
        end_detail.clone(),
    );
    receipt["end_capture"] = json!(end_capture
        .as_ref()
        .map(|_| format!("{end_stem}.png"))
        .map_err(Clone::clone));
    if !arrived || !guard_server_crossing || !secret_wall_server_crossing || !fresh_tile_progress {
        let failure_capture =
            save_frame(&mut client, &snapshot, &evidence, &failure_stem, end_detail);
        receipt["failure_capture"] = json!(failure_capture
            .as_ref()
            .map(|_| format!("{failure_stem}.png"))
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
    println!(
        "{}",
        serde_json::to_string_pretty(&receipt).expect("print receipt")
    );

    assert!(
        fresh_account_baseline,
        "account was not fresh before seeding: {receipt}"
    );
    assert!(
        inventory_only_no_path,
        "inventory-only disguise opened a route: {receipt}"
    );
    assert!(
        both_items_worn,
        "both data-selected disguise items were not worn: {receipt}"
    );
    assert!(
        strict_all_gate,
        "guard route lacks the strict conjunction gate: {receipt}"
    );
    assert!(
        route_uses_guard_door,
        "route did not use bkfortressdoor1 at the reported tile: {receipt}"
    );
    assert!(
        route_uses_secret_wall,
        "route did not use the first secret push-wall: {receipt}"
    );
    assert!(
        guard_server_crossing,
        "no server-settled progress across the guard door: {receipt}"
    );
    assert!(
        secret_wall_server_crossing,
        "no server-settled progress across the secret wall: {receipt}"
    );
    assert!(
        attempts_in_order,
        "guard and secret-wall attempts were absent or out of route order: {receipt}"
    );
    assert!(
        fresh_tile_progress,
        "Traveller produced no fresh tile progress: {receipt}"
    );
    assert!(
        arrived,
        "Traveller did not reach the selected inside destination: {receipt}"
    );
    assert!(
        arrival_scene_ready,
        "arrival was not ingame with scene_state==2: {receipt}"
    );
    assert!(
        start_capture.is_ok(),
        "start capture was not preserved: {receipt}"
    );
    assert!(
        end_capture.is_ok(),
        "end capture was not preserved: {receipt}"
    );
    assert!(
        logout_observed,
        "successful explicit logout was not observed: {receipt}"
    );
}

fn run_brimhaven_black_arm_kitchen_refusal() {
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"));
    assert_eq!(std::env::var("BOT_LIVE_NAME_PREFIX").as_deref(), Ok("npr2"));
    let evidence = required_path("NAV_KITCHEN_EVIDENCE");
    fs::create_dir_all(&evidence).unwrap();
    let serial = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let (account, password) = mint_live_entries(&mint_live_names(1)).pop().unwrap();
    let mut options = live_profile_options();
    options.port = Some(47594);
    options.http_port = Some(3080);
    let profile = options.resolve(None).unwrap().bind().unwrap();
    let template = SharedClientTemplate::load(profile).unwrap();
    let world = template.world().unwrap();
    let edge = world
        .graph
        .edges
        .iter()
        .find(|edge| {
            edge.loc_id == 2628
                && edge
                    .crossing_id()
                    .is_some_and(|crossing| crossing.takeoff.z == 3190)
        })
        .expect("qualified kitchen door")
        .clone();
    let crossing = edge.crossing_id().expect("kitchen door crossing identity");
    let mut client = template
        .prepare_client((serial % i32::MAX as u128) as i32, true)
        .unwrap();
    client.draw = false;
    client.maininit();
    assert!(!client.error_loading);
    assert!(interact::login(&mut client, &account, &password, false));
    let mut snapshot = GameSnapshot::new();
    let mut pump = Pump::new();
    let result = (|| -> Result<Value, String> {
        if !wait_for(&mut client, &mut snapshot, &mut pump, SETUP_TIMEOUT, |s| {
            s.ingame() && s.attached() && s.scene_state() == 2
        }) {
            return Err("initial scene timeout".into());
        }
        interact::mainland_hop(&mut client);
        assert!(interact::cheat(&mut client, "getvar tutorial").is_sent());
        if !wait_for(&mut client, &mut snapshot, &mut pump, SETUP_TIMEOUT, |s| {
            s.chat_lines()
                .iter()
                .any(|line| line.text.contains("get tutorial: 1000"))
        }) {
            return Err("tutorial seed timeout".into());
        }
        if !logout_and_wait(&mut client, &mut snapshot, &mut pump) {
            return Err("tutorial logout timeout".into());
        }
        if !interact::login(&mut client, &account, &password, false) {
            return Err("tutorial relog refused".into());
        }
        if !wait_for(&mut client, &mut snapshot, &mut pump, SETUP_TIMEOUT, |s| {
            s.ingame() && s.attached() && s.scene_state() == 2
        }) {
            return Err("relogin scene timeout".into());
        }
        // Account-local seeds only. Never create, replace, or open the world loc.
        // quest_hero.constant:4 and quest.constant:7-8 define these stages.
        let hero = 15;
        let gang = 0;
        let black_arm = 4;
        for (varp, value) in [
            ("tutorial", scenario::tutorial::TUTORIAL_COMPLETE),
            ("heroquest", hero),
            ("phoenixgang", gang),
            ("blackarmgang", black_arm),
        ] {
            let mark = snapshot
                .chat_lines()
                .iter()
                .map(|line| line.sequence)
                .max()
                .unwrap_or(0);
            for command in [format!("setvar {varp} {value}"), format!("getvar {varp}")] {
                if !interact::cheat(&mut client, &command).is_sent() {
                    return Err(format!("seed refused: {command}"));
                }
            }
            let expected = format!("get {varp}: {value}");
            if !wait_for(&mut client, &mut snapshot, &mut pump, SETUP_TIMEOUT, |s| {
                s.chat_lines()
                    .iter()
                    .any(|line| line.sequence > mark && line.text.contains(&expected))
            }) {
                return Err(format!("missing fresh server witness: {expected}"));
            }
        }
        interact::seed_at(
            &mut client,
            crossing.takeoff.level,
            crossing.takeoff.x,
            crossing.takeoff.z,
        );
        if !wait_for(&mut client, &mut snapshot, &mut pump, SETUP_TIMEOUT, |s| {
            s.tile()
                == Some((
                    crossing.takeoff.x,
                    crossing.takeoff.z,
                    crossing.takeoff.level,
                ))
                && s.scene_state() == 2
                && s.locs()
                    .iter()
                    .any(|loc| loc.id == 2628 && loc.tile == crossing.at)
        }) {
            return Err("kitchen stand or real door absent".into());
        }
        let before_tick = snapshot.tick();
        let mark = snapshot
            .chat_lines()
            .iter()
            .map(|line| line.sequence)
            .max()
            .unwrap_or(0);
        let loc = snapshot
            .locs()
            .iter()
            .find(|loc| loc.id == 2628 && loc.tile == crossing.at)
            .unwrap();
        let sent = matches!(
            Interactions::new(&snapshot, &mut client).interact(
                api::interact::OpTarget::Loc(loc),
                api::interact::ActionSpec::Label("Open".into()),
            ),
            SendResult::Sent { .. }
        );
        if !sent {
            return Err("real Open interaction refused by client".into());
        }
        let settled = wait_for(
            &mut client,
            &mut snapshot,
            &mut pump,
            Duration::from_secs(20),
            |s| {
                s.tick() > before_tick
                    && s.chat_lines().iter().any(|line| {
                        line.sequence > mark
                            && line.text.contains("This door seems to be locked...")
                    })
            },
        );
        let refused = snapshot.chat_lines().iter().any(|line| {
            line.sequence > mark && line.text.contains("This door seems to be locked...")
        });
        let remained = snapshot.tile()
            == Some((
                crossing.takeoff.x,
                crossing.takeoff.z,
                crossing.takeoff.level,
            ));
        let correct = settled && refused && remained;
        let receipt = json!({
            "account": account,
            "scenario": "black-arm-gang-refusal",
            "server": "Engine L through chardonnay 127.0.0.1:47594",
            "seeded": {
                "heroquest_188": hero,
                "phoenixgang_146": gang,
                "blackarmgang_145": black_arm,
            },
            "seed_authority": "fresh server getvar replies",
            "crossing": crossing,
            "before_tick": before_tick,
            "after_tick": snapshot.tick(),
            "after_tile": snapshot.tile(),
            "fresh_refusal": refused,
            "correct": correct,
            "chat": snapshot
                .chat_lines()
                .iter()
                .map(|line| (&line.text, line.sequence))
                .collect::<Vec<_>>(),
        });
        if !correct {
            return Err(format!("wrong kitchen result: {receipt}"));
        }
        Ok(receipt)
    })();
    let logout = logout_and_wait(&mut client, &mut snapshot, &mut pump);
    if !logout {
        client.logout();
    }
    let receipt = json!({ "result": result, "logout_observed": logout });
    write_receipt(
        &evidence.join(format!("kitchen-black-arm-{account}-{serial}.json")),
        &receipt,
    );
    println!("KITCHEN_LIVE {receipt}");
    assert!(logout, "live account must log out");
    assert!(result.is_ok(), "kitchen live proof failed: {receipt}");
}

struct NoStore;

impl LocalStatePort for NoStore {
    fn lease(&self, identity: &DomainIdentity) -> Result<DomainLease, ErrorCode> {
        Err(ErrorCode::NotReady {
            domain: identity.domain(),
        })
    }

    fn submit(&self, edit: DomainEdit) -> SubmitReceipt {
        SubmitReceipt::Refused(ErrorCode::Busy {
            domain: edit.identity().domain(),
        })
    }

    fn observe(&self, _: api::identity::RequestId) -> Option<WriteSettlement> {
        None
    }
}

impl StatePort for NoStore {
    fn lease_version(&self, identity: &DomainIdentity) -> Result<StoreVersion, ErrorCode> {
        self.lease(identity).map(|lease| lease.version())
    }
}

fn evidence_stamp_json(stamp: EvidenceStamp) -> Value {
    json!({
        "run": {
            "slot": stamp.run.slot,
            "run": stamp.run.run,
            "session": stamp.run.session,
        },
        "tick": stamp.tick,
        "sequence": stamp.sequence,
    })
}

fn journal_attempt_receipt(
    before: Option<JournalAttempt>,
    after: Option<JournalAttempt>,
    stamp: EvidenceStamp,
    error: Option<&str>,
    completed: bool,
) -> Option<Value> {
    if before.is_none() && after.is_none() {
        return None;
    }
    let previous = before.map_or(0, |attempt| attempt.attempts_started);
    let current = after.map_or(0, |attempt| attempt.attempts_started);
    let started = previous.max(current);
    let (event, attempt, failure) = if current > previous {
        ("started", current, None)
    } else if let Some(error) = error {
        if before.is_some_and(|attempt| attempt.owner_active) {
            ("failed", started, Some(error))
        } else {
            ("begin_failed", started + 1, Some(error))
        }
    } else if completed {
        ("completed", started, None)
    } else if before.is_some_and(|attempt| attempt.owner_active)
        && after.is_some_and(|attempt| !attempt.owner_active && attempt.retry_after.is_some())
    {
        // The runner sets retry_after only for this exact native failure.
        ("failed", started, Some("journal modal timeout"))
    } else {
        return None;
    };
    Some(json!({
        "attempt": attempt,
        "event": event,
        "observed": evidence_stamp_json(stamp),
        "failure": failure,
        "retry_after_tick": after.and_then(|attempt| attempt.retry_after),
    }))
}

#[test]
fn journal_attempt_receipts_keep_timeout_retry_and_last_error() {
    let stamp = |tick| EvidenceStamp {
        run: api::selected::RunKey {
            slot: 1,
            run: 1,
            session: 1,
        },
        tick,
        sequence: tick,
    };
    let first = JournalAttempt {
        attempts_started: 1,
        owner_active: true,
        retry_after: None,
    };
    let retry = JournalAttempt {
        attempts_started: 1,
        owner_active: false,
        retry_after: Some(6),
    };
    let second = JournalAttempt {
        attempts_started: 2,
        owner_active: true,
        retry_after: Some(6),
    };
    let started = journal_attempt_receipt(None, Some(first), stamp(1), None, false).unwrap();
    assert_eq!(started["event"], "started");
    assert_eq!(started["attempt"], 1);
    let timeout = journal_attempt_receipt(Some(first), Some(retry), stamp(6), None, false).unwrap();
    assert_eq!(timeout["event"], "failed");
    assert_eq!(timeout["failure"], "journal modal timeout");
    assert_eq!(timeout["observed"]["tick"], 6);
    assert!(journal_attempt_receipt(Some(retry), Some(retry), stamp(6), None, false).is_none());
    let restarted =
        journal_attempt_receipt(Some(retry), Some(second), stamp(7), None, false).unwrap();
    assert_eq!(restarted["event"], "started");
    assert_eq!(restarted["attempt"], 2);
    assert_eq!(restarted["retry_after_tick"], 6);
    let failure = journal_attempt_receipt(
        Some(second),
        None,
        stamp(8),
        Some("the native action failed: journal click refused"),
        false,
    )
    .unwrap();
    assert_eq!(failure["event"], "failed");
    assert_eq!(failure["attempt"], 2);
    assert_eq!(
        failure["failure"],
        "the native action failed: journal click refused"
    );
    let begin_failed = journal_attempt_receipt(
        Some(retry),
        Some(retry),
        stamp(7),
        Some("journal facts are not prepared"),
        false,
    )
    .unwrap();
    assert_eq!(begin_failed["event"], "begin_failed");
    assert_eq!(begin_failed["attempt"], 2);
    let completed = journal_attempt_receipt(Some(second), None, stamp(9), None, true).unwrap();
    assert_eq!(completed["event"], "completed");
    assert_eq!(completed["attempt"], 2);
}

#[derive(Default)]
struct NativePrereqReport {
    finished: bool,
    attempts: Vec<Value>,
    journal_attempts: Vec<Value>,
    unsupported: Vec<Value>,
    journal_pages: Vec<LiveJournalPage>,
    error: Option<String>,
    current_stamp: Option<EvidenceStamp>,
}

struct KitchenPrereqScript {
    remaining: Vec<api::prereq_steps::ActionableGate>,
    active: Option<(api::prereq_steps::ActionableGate, PrereqRun)>,
    budget: PrereqBudget,
    report: Arc<Mutex<NativePrereqReport>>,
    open_page: Option<EvidenceStamp>,
}
impl Script for KitchenPrereqScript {
    fn tick(&mut self, tick: &mut NativeTick<'_>) -> Result<ScriptFlow, ScriptFailure> {
        let stamp = tick.cx.evidence();
        {
            let snapshot = tick.cx.snapshot();
            if let Some(page) = snapshot.journal_widgets(ROOT_289, TITLE_289) {
                let acquired = page.stamp;
                let mut report = self.report.lock();
                if report
                    .journal_pages
                    .iter()
                    .all(|captured| captured.acquired != acquired)
                {
                    report.journal_pages.push(LiveJournalPage {
                        title: page.value.title.to_string(),
                        lines: page.value.lines().map(str::to_owned).collect(),
                        acquired,
                        closed: None,
                    });
                }
                self.open_page = Some(acquired);
            } else if let Some(acquired) = self.open_page {
                if let Some(modal) = snapshot
                    .main_modal()
                    .filter(|modal| modal.value.root == -1 && modal.value.texts.is_empty())
                {
                    if let Some(page) = self
                        .report
                        .lock()
                        .journal_pages
                        .iter_mut()
                        .find(|page| page.acquired == acquired)
                    {
                        page.closed = Some(modal.stamp);
                    }
                    self.open_page = None;
                }
            }
        }
        let Some(state) = tick.state else {
            let mut report = self.report.lock();
            report.finished = true;
            report.error = Some("native prerequisite tick has no session state".into());
            report.current_stamp = Some(stamp);
            return Ok(ScriptFlow::Complete);
        };

        if let Some((gate, mut run)) = self.active.take() {
            let before = run.journal_attempt();
            let result = run.poll(tick.actions, &mut tick.cx, Some(state), &mut self.budget);
            let after = run.journal_attempt();
            let error = match &result {
                Poll::Ready(Err(error)) => Some(format!("{error:?}")),
                _ => None,
            };
            if let Some(mut receipt) = journal_attempt_receipt(
                before,
                after,
                stamp,
                error.as_deref(),
                matches!(&result, Poll::Ready(Ok(_))),
            ) {
                receipt["gate"] = json!(&gate);
                self.report.lock().journal_attempts.push(receipt);
            }
            match result {
                Poll::Pending => {
                    self.active = Some((gate, run));
                    return Ok(ScriptFlow::Continue);
                }
                Poll::Ready(Ok(done)) => {
                    self.report.lock().attempts.push(json!({
                        "gate": gate,
                        "crossed": done.crossed,
                    }));
                    if done.crossed {
                        let mut report = self.report.lock();
                        report.finished = true;
                        report.error =
                            Some("journal prerequisite unexpectedly crossed the door".into());
                        report.current_stamp = Some(stamp);
                        report.unsupported =
                            self.remaining.iter().map(|gate| json!(gate)).collect();
                        return Ok(ScriptFlow::Complete);
                    }
                }
                Poll::Ready(Err(_)) => {
                    let mut report = self.report.lock();
                    report.finished = true;
                    report.error = error;
                    report.current_stamp = Some(stamp);
                    report.unsupported = self.remaining.iter().map(|gate| json!(gate)).collect();
                    return Ok(ScriptFlow::Complete);
                }
            }
        }

        let entry = RunEntry::NeedsStep;
        let Some(gate) = PrereqRun::first_supported(&self.remaining, &entry, &tick.cx) else {
            let mut report = self.report.lock();
            report.finished = true;
            report.current_stamp = Some(stamp);
            report.unsupported = self.remaining.iter().map(|gate| json!(gate)).collect();
            return Ok(ScriptFlow::Complete);
        };
        self.remaining.retain(|candidate| candidate != &gate);
        self.active = Some((
            gate.clone(),
            PrereqRun::begin(gate, entry, PrereqAllow::default()),
        ));
        Ok(ScriptFlow::Continue)
    }
}

#[derive(Clone)]
struct LiveJournalPage {
    title: String,
    lines: Vec<String>,
    acquired: EvidenceStamp,
    closed: Option<EvidenceStamp>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum KitchenFrameCommand {
    Idle,
    Open,
    Logout,
}

struct KitchenFrameControl {
    command: KitchenFrameCommand,
    crossing: api::prereq_steps::CrossingId,
    crossing_pending: Option<(u32, i32)>,
    crossing_result: Option<Value>,
    logout_sent: bool,
    logout_observed: bool,
}

impl KitchenFrameControl {
    fn new(crossing: api::prereq_steps::CrossingId) -> Self {
        Self {
            command: KitchenFrameCommand::Idle,
            crossing,
            crossing_pending: None,
            crossing_result: None,
            logout_sent: false,
            logout_observed: false,
        }
    }
}

fn observe_kitchen_frame(
    client: &mut Client,
    username: &str,
    account: &str,
    input: host_play::SlotFrameInput,
    snapshot: &mut GameSnapshot,
    control: &Mutex<KitchenFrameControl>,
) {
    if username != account {
        return;
    }
    client.draw = false;
    snapshot.rebuild(client);
    let mut control = control.lock();

    if let Some((before_tick, mark)) = control.crossing_pending {
        let destination = (
            control.crossing.to.x,
            control.crossing.to.z,
            control.crossing.to.level,
        );
        let arrived = snapshot.tile() == Some(destination);
        let refusal = snapshot.chat_lines().iter().find(|line| {
            line.sequence > mark && line.text.contains("This door seems to be locked...")
        });
        if snapshot.tick() > before_tick && (arrived || refusal.is_some()) {
            control.crossing_result = Some(json!({
                "before_tick": before_tick,
                "after_tick": snapshot.tick(),
                "before_tile": control.crossing.takeoff,
                "after_tile": snapshot.tile(),
                "refused": refusal.is_some(),
                "refusal": refusal.map(|line| (&line.text, line.sequence)),
                "correct": arrived && refusal.is_none(),
                "chat": snapshot.chat_lines().iter()
                    .map(|line| (&line.text, line.sequence))
                    .collect::<Vec<_>>(),
            }));
            control.crossing_pending = None;
        }
    }

    match control.command {
        KitchenFrameCommand::Idle => {}
        KitchenFrameCommand::Open => {
            if input.hold
                || input.host_move_owned
                || snapshot.tile()
                    != Some((
                        control.crossing.takeoff.x,
                        control.crossing.takeoff.z,
                        control.crossing.takeoff.level,
                    ))
            {
                return;
            }
            let Some(loc) = snapshot
                .locs()
                .iter()
                .find(|loc| loc.id == 2628 && loc.tile == control.crossing.at)
            else {
                control.crossing_result = Some(json!({
                    "correct": false,
                    "error": "real kitchen door absent at planned crossing",
                    "tick": snapshot.tick(),
                    "tile": snapshot.tile(),
                }));
                control.command = KitchenFrameCommand::Idle;
                return;
            };
            let mark = snapshot
                .chat_lines()
                .iter()
                .map(|line| line.sequence)
                .max()
                .unwrap_or(0);
            let before_tick = snapshot.tick();
            let sent = matches!(
                Interactions::new(snapshot, client).interact(
                    api::interact::OpTarget::Loc(loc),
                    api::interact::ActionSpec::Label("Open".into()),
                ),
                SendResult::Sent { .. }
            );
            if sent {
                control.crossing_pending = Some((before_tick, mark));
            } else {
                control.crossing_result = Some(json!({
                    "correct": false,
                    "error": "real Open interaction refused by client",
                    "tick": before_tick,
                    "tile": snapshot.tile(),
                }));
            }
            control.command = KitchenFrameCommand::Idle;
        }
        KitchenFrameCommand::Logout => {
            if snapshot.ingame() && !control.logout_sent {
                let ifaces = Arc::clone(&client.ifaces);
                if !interact::logout(client, &ifaces) {
                    client.logout();
                }
                control.logout_sent = true;
            } else if control.logout_sent && !snapshot.ingame() {
                control.logout_observed = true;
            }
        }
    }
}

#[derive(Clone)]
struct SessionAndOrdinaryEvidence {
    memory: SessionMemory,
    ordinary: QuestListEvidenceProvider,
}

impl EvidenceProvider for SessionAndOrdinaryEvidence {
    fn observe_gate(
        &self,
        gate: &api::selected::QuestGate,
        required_after: EvidenceStamp,
    ) -> Option<Observation> {
        Some(prereq_knowledge::compose(
            gate,
            required_after,
            &self.memory,
            None,
            &self.ordinary,
        ))
    }
}

fn wait_for_play_tile(
    play: &host_play::Play,
    account: &str,
    tile: WorldTile,
    timeout: Duration,
) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if play.statuses().into_iter().any(|row| {
            row.username == account
                && row.ingame
                && row.scene_state == 2
                && row.tile_x == tile.x
                && row.tile_z == tile.z
                && row.tile_level == tile.level
        }) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn wait_for_native_prereq(play: &host_play::Play, account: &str, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if play
            .script_lifecycle_receipt(account)
            .is_some_and(|receipt| {
                matches!(
                    receipt.state,
                    script::ScriptTerminalState::Completed | script::ScriptTerminalState::Failed
                )
            })
        {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn wait_for_kitchen_crossing(
    control: &Mutex<KitchenFrameControl>,
    timeout: Duration,
) -> Option<Value> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(result) = control.lock().crossing_result.clone() {
            return Some(result);
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn page_receipt(page: &LiveJournalPage) -> Value {
    json!({
        "title": &page.title,
        "text": &page.lines,
        "acquired": evidence_stamp_json(page.acquired),
        "closed": page.closed.map(evidence_stamp_json),
    })
}

fn fact_receipt(fact: &Fact) -> Value {
    let source = match &fact.source {
        api::prereq_knowledge::FactSource::Page { row, line } => {
            json!({"kind": "page", "row": row, "rule": line})
        }
        api::prereq_knowledge::FactSource::Crossing {
            loc_id,
            option,
            passed,
        } => json!({"kind": "crossing", "loc_id": loc_id, "option": option, "passed": passed}),
        api::prereq_knowledge::FactSource::Refusal { row, entry } => {
            json!({"kind": "refusal", "row": row, "entry": entry})
        }
    };
    json!({
        "quest": &fact.quest,
        "signal": &fact.signal,
        "possible": fact.possible,
        "after": fact.after,
        "refused": fact.refused,
        "source": source,
        "observed": evidence_stamp_json(fact.observed),
    })
}
fn seed_phoenix_kitchen_account(
    template: &SharedClientTemplate,
    uid: i32,
    account: &str,
    password: &str,
    crossing: api::prereq_steps::CrossingId,
) -> (Value, bool, Option<String>) {
    let mut client = match template.prepare_client(uid, true) {
        Ok(client) => client,
        Err(error) => {
            return (json!({"error": error.clone()}), false, Some(error));
        }
    };
    client.draw = false;
    client.maininit();
    let mut snapshot = GameSnapshot::new();
    let mut pump = Pump::new();
    let seed_result = (|| -> Result<Value, String> {
        if client.error_loading {
            return Err("seed client failed to load".into());
        }
        if !interact::login(&mut client, account, password, false) {
            return Err("seed account login refused".into());
        }
        if !wait_for(&mut client, &mut snapshot, &mut pump, SETUP_TIMEOUT, |s| {
            s.ingame() && s.attached() && s.scene_state() == 2
        }) {
            return Err("initial seed scene timeout".into());
        }
        interact::mainland_hop(&mut client);
        if !interact::cheat(&mut client, "getvar tutorial").is_sent() {
            return Err("tutorial seed query refused".into());
        }
        if !wait_for(&mut client, &mut snapshot, &mut pump, SETUP_TIMEOUT, |s| {
            s.chat_lines()
                .iter()
                .any(|line| line.text.contains("get tutorial: 1000"))
        }) {
            return Err("tutorial seed query timeout".into());
        }
        if !logout_and_wait(&mut client, &mut snapshot, &mut pump) {
            return Err("tutorial seed logout timeout".into());
        }
        if !interact::login(&mut client, account, password, false) {
            return Err("tutorial seed relog refused".into());
        }
        if !wait_for(&mut client, &mut snapshot, &mut pump, SETUP_TIMEOUT, |s| {
            s.ingame() && s.attached() && s.scene_state() == 2
        }) {
            return Err("seed relog scene timeout".into());
        }

        // The Phoenix journal branch requires Black Arm to remain unjoined.
        // Black Arm stage 4 stays the independent refusal case.
        let seeded = [
            ("tutorial", scenario::tutorial::TUTORIAL_COMPLETE),
            ("heroquest", 3),
            ("phoenixgang", 10),
            ("blackarmgang", 0),
        ];
        for (varp, value) in seeded {
            let mark = snapshot
                .chat_lines()
                .iter()
                .map(|line| line.sequence)
                .max()
                .unwrap_or(0);
            for command in [format!("setvar {varp} {value}"), format!("getvar {varp}")] {
                if !interact::cheat(&mut client, &command).is_sent() {
                    return Err(format!("seed refused: {command}"));
                }
            }
            let expected = format!("get {varp}: {value}");
            if !wait_for(&mut client, &mut snapshot, &mut pump, SETUP_TIMEOUT, |s| {
                s.chat_lines()
                    .iter()
                    .any(|line| line.sequence > mark && line.text.contains(&expected))
            }) {
                return Err(format!("missing fresh server witness: {expected}"));
            }
        }

        interact::seed_at(
            &mut client,
            crossing.takeoff.level,
            crossing.takeoff.x,
            crossing.takeoff.z,
        );
        if !wait_for(&mut client, &mut snapshot, &mut pump, SETUP_TIMEOUT, |s| {
            s.tile()
                == Some((
                    crossing.takeoff.x,
                    crossing.takeoff.z,
                    crossing.takeoff.level,
                ))
                && s.scene_state() == 2
                && s.locs()
                    .iter()
                    .any(|loc| loc.id == 2628 && loc.tile == crossing.at)
        }) {
            return Err("kitchen stand or real door absent after account seed".into());
        }
        Ok(json!({
            "seeded": {
                "heroquest_188": 3,
                "phoenixgang_146": 10,
                "blackarmgang_145": 0,
                "tutorial": scenario::tutorial::TUTORIAL_COMPLETE,
            },
            "seed_authority": "fresh server getvar replies",
            "seed_tile": snapshot.tile(),
            "seed_tick": snapshot.tick(),
            "crossing": crossing,
        }))
    })();

    let logout = logout_and_wait(&mut client, &mut snapshot, &mut pump);
    if !logout {
        client.logout();
    }
    let error = seed_result.as_ref().err().cloned();
    let detail = json!({
        "result": seed_result.as_ref().ok(),
        "error": error,
        "logout_observed": logout,
    });
    (detail, logout, error)
}

struct PhoenixPlayResult {
    detail: Value,
    logout_observed: bool,
    error: Option<String>,
}

struct PhoenixPlaySetup<'a> {
    template: Arc<SharedClientTemplate>,
    selected: Arc<api::game_data::SelectedGameData>,
    world: Arc<NavWorld>,
    edge: TransportEdge,
    account: &'a str,
    password: &'a str,
    uid: i32,
    map_members: bool,
}

fn run_phoenix_native_and_crossing(
    setup: PhoenixPlaySetup<'_>,
    receipt_path: &Path,
    receipt: &mut Value,
) -> PhoenixPlayResult {
    let PhoenixPlaySetup {
        template,
        selected,
        world,
        edge,
        account,
        password,
        uid,
        map_members,
    } = setup;
    let crossing = edge.crossing_id().expect("qualified kitchen crossing");
    let snapshot = Arc::new(Mutex::new(GameSnapshot::new()));
    let control = Arc::new(Mutex::new(KitchenFrameControl::new(crossing)));
    let frame_snapshot = Arc::clone(&snapshot);
    let frame_control = Arc::clone(&control);
    let frame_account = account.to_owned();
    let vault_profile = Profile {
        username: account.to_owned(),
        password: password.to_owned().into(),
        uid,
        settings: ProfileSettings::default(),
    };
    let mut play = match host_play::run_with_template(
        template,
        false,
        vec![vault_profile],
        |_| (None, None),
        move |client, username, input| {
            let mut snapshot = frame_snapshot.lock();
            observe_kitchen_frame(
                client,
                username,
                &frame_account,
                input,
                &mut snapshot,
                &frame_control,
            );
        },
    ) {
        Ok(play) => play,
        Err(error) => {
            return PhoenixPlayResult {
                detail: json!({"startup_error": &error}),
                logout_observed: false,
                error: Some(format!("start Engine Q Play: {error}")),
            };
        }
    };
    play.focus(account);

    let slot_state = Arc::new(SlotState::new(
        Arc::new(NoStore),
        match AccountKey::try_new("local-289", account) {
            Ok(account) => account,
            Err(error) => {
                control.lock().command = KitchenFrameCommand::Logout;
                let deadline = Instant::now() + Duration::from_secs(30);
                while !control.lock().logout_observed && Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(50));
                }
                return PhoenixPlayResult {
                    detail: json!({"state_error": format!("{error:?}")}),
                    logout_observed: control.lock().logout_observed,
                    error: Some(format!("invalid slot account key: {error:?}")),
                };
            }
        },
        match ContentScope::try_new(ClientRevision::R289, "nav-prereq-phoenix-journal") {
            Ok(scope) => scope,
            Err(error) => {
                control.lock().command = KitchenFrameCommand::Logout;
                let deadline = Instant::now() + Duration::from_secs(30);
                while !control.lock().logout_observed && Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(50));
                }
                return PhoenixPlayResult {
                    detail: json!({"state_error": format!("{error:?}")}),
                    logout_observed: control.lock().logout_observed,
                    error: Some(format!("invalid slot content scope: {error:?}")),
                };
            }
        },
    ));

    let proof = (|| -> Result<Value, String> {
        if !wait_for_play_tile(&play, account, crossing.takeoff, Duration::from_secs(90)) {
            return Err(format!(
                "Engine Q account did not log in at kitchen stand: {:?}",
                play.statuses()
                    .into_iter()
                    .find(|row| row.username == account)
            ));
        }
        let candidates = edge
            .quest_gates
            .as_ref()
            .ok_or_else(|| "qualified kitchen edge has no quest gates".to_owned())?
            .gates()
            .iter()
            .cloned()
            .map(|gate| api::prereq_steps::ActionableGate {
                gate,
                truth: Truth::Unknown,
                leg: 0,
                crossing,
            })
            .collect::<Vec<_>>();
        let report = Arc::new(Mutex::new(NativePrereqReport::default()));
        play.script_start_handle()
            .start_test_script_with_local_state(
                account,
                Box::new(KitchenPrereqScript {
                    remaining: candidates,
                    active: None,
                    budget: PrereqBudget::default(),
                    report: Arc::clone(&report),
                    open_page: None,
                }),
                Some(Arc::clone(&selected)),
                Arc::clone(&slot_state),
            )
            .map_err(|error| format!("start production PrereqRun: {error:?}"))?;
        if !wait_for_native_prereq(&play, account, Duration::from_secs(120)) {
            let memory = slot_state.prereq_snapshot();
            let report_guard = report.lock();
            receipt["prerequisite_and_crossing"] = json!({
                "partial": {
                    "driver": "script::quester::prereq::run::PrereqRun",
                    "attempts": &report_guard.attempts,
                    "journal_attempts": &report_guard.journal_attempts,
                    "unsupported_gates": &report_guard.unsupported,
                    "session_memory": memory.facts().iter().map(fact_receipt).collect::<Vec<_>>(),
                    "journal_pages": report_guard.journal_pages.iter().map(page_receipt).collect::<Vec<_>>(),
                },
                "error": "production prerequisite journal run timed out",
            });
            write_receipt(receipt_path, receipt);
            drop(report_guard);
            play.script_stop(account);
            return Err("production prerequisite journal run timed out".into());
        }
        let lifecycle = play.script_lifecycle_receipt(account);
        let (
            attempts,
            journal_attempts,
            unsupported,
            pages,
            report_error,
            report_finished,
            current_stamp,
        ) = {
            let report = report.lock();
            (
                report.attempts.clone(),
                report.journal_attempts.clone(),
                report.unsupported.clone(),
                report.journal_pages.clone(),
                report.error.clone(),
                report.finished,
                report.current_stamp,
            )
        };

        let memory = slot_state.prereq_snapshot();
        let page_facts = memory
            .facts()
            .iter()
            .filter(|fact| matches!(&fact.source, api::prereq_knowledge::FactSource::Page { .. }))
            .collect::<Vec<_>>();
        let mut detail = json!({
            "driver": "script::quester::prereq::run::PrereqRun",
            "journal_machine": std::any::type_name::<JournalMachine>(),
            "attempts": &attempts,
            "journal_attempts": &journal_attempts,
            "unsupported_gates": &unsupported,
            "session_memory": memory.facts().iter().map(fact_receipt).collect::<Vec<_>>(),
            "journal_pages": pages.iter().map(page_receipt).collect::<Vec<_>>(),
            "runner_stamp": current_stamp.map(evidence_stamp_json),
            "lifecycle_state": lifecycle.as_ref().map(|receipt| format!("{:?}", receipt.state)),
            "read_count": pages.len(),
            "close_count": pages.iter().filter(|page| page.closed.is_some()).count(),
        });
        receipt["prerequisite_and_crossing"] = json!({"proof": detail.clone()});
        write_receipt(receipt_path, receipt);

        let lifecycle = lifecycle
            .ok_or_else(|| "production prerequisite lifecycle receipt absent".to_owned())?;
        if lifecycle.state != script::ScriptTerminalState::Completed {
            return Err(format!(
                "production prerequisite script ended in {:?}",
                lifecycle.state
            ));
        }
        if let Some(error) = report_error {
            return Err(format!("production PrereqRun refused: {error}"));
        }
        if !report_finished {
            return Err("production prerequisite script ended without a settled report".into());
        }
        let current_stamp = current_stamp
            .ok_or_else(|| "production prerequisite report lacks its live stamp".to_owned())?;
        if attempts.len() != 2 || pages.len() != attempts.len() {
            return Err(format!(
                "expected one Hero and one Shield journal read, got attempts={} pages={}",
                attempts.len(),
                pages.len()
            ));
        }
        if pages.iter().any(|page| match page.closed {
            None => true,
            Some(closed) => {
                closed.run != page.acquired.run
                    || (closed.tick, closed.sequence)
                        <= (page.acquired.tick, page.acquired.sequence)
            }
        }) {
            return Err("a production journal read lacks a later close stamp".into());
        }
        if page_facts.len() != attempts.len() {
            return Err(format!(
                "expected one session Page fact per journal read, got facts={} reads={}",
                page_facts.len(),
                attempts.len()
            ));
        }
        for page in &pages {
            let normalized = page.title.strip_prefix("@dre@").unwrap_or(&page.title);
            if !["Heroes' Quest", "Shield of Arrav"]
                .iter()
                .any(|expected| normalized.trim().eq_ignore_ascii_case(expected))
            {
                return Err(format!(
                    "unexpected production journal page: {}",
                    page.title
                ));
            }
        }
        let has_page = |expected: &str| {
            pages.iter().any(|page| {
                let normalized = page.title.strip_prefix("@dre@").unwrap_or(&page.title);
                normalized.trim().eq_ignore_ascii_case(expected)
            })
        };
        if !has_page("Heroes' Quest") || !has_page("Shield of Arrav") {
            return Err(
                "the prerequisite attempts did not read both Hero and Shield journals".into(),
            );
        }
        let floor = page_facts
            .iter()
            .map(|fact| fact.observed)
            .min_by_key(|stamp| (stamp.tick, stamp.sequence))
            .ok_or_else(|| "production journal pages recorded no session facts".to_owned())?;
        let identity = selected
            .quest_family_identity()
            .ok_or_else(|| "selected quest family identity is unavailable".to_owned())?;
        let family = QuestFamilyId::new(identity.sha256, identity.schema);
        if world.graph.quest_family != Some(family) {
            return Err("navigation pack and selected quest family identities differ".into());
        }
        let quests = selected
            .try_quests()
            .ok_or_else(|| "selected quest catalog was not prepared".to_owned())?;
        let snapshot_guard = snapshot.lock();
        let ordinary = QuestListEvidenceProvider::from_snapshot(
            &quests,
            Some(identity),
            Some(identity),
            SnapshotView::new(Some(&snapshot_guard), current_stamp),
        );
        let evidence = QuestEvidence::new(
            Arc::new(SessionAndOrdinaryEvidence {
                memory: memory.clone(),
                ordinary: ordinary.clone(),
            }),
            family,
            floor,
        );
        let state = WorldState::from_snapshot(&snapshot_guard)
            .with_map_members(map_members)
            .with_quest_evidence(evidence);
        let quest_truth = state.quest_gates(&edge);
        let gate_checks = edge
            .quest_gates
            .as_ref()
            .expect("edge quest gates checked above")
            .gates()
            .iter()
            .map(|gate| {
                json!({
                    "gate": gate,
                    "ordinary": format!("{:?}", ordinary.test_gate(gate, floor)),
                    "combined": format!("{:?}", prereq_knowledge::compose(
                        gate, floor, &memory, None, &ordinary
                    ).truth),
                })
            })
            .collect::<Vec<_>>();
        if quest_truth != Truth::True {
            return Err(format!(
                "journal session memory did not prove every kitchen gate: {gate_checks:?}"
            ));
        }
        let route = find_route(&world, crossing.takeoff, crossing.to, &state)
            .map_err(|error| format!("planner route failed after journal evidence: {error:?}"))?;
        let planned = planned_edge(&route, 2628, crossing.at)
            .ok_or_else(|| "planner route omitted kitchen loc 2628".to_owned())?;
        if planned.crossing_id() != Some(crossing) {
            return Err("planner selected a different directed kitchen crossing".into());
        }
        let plan = json!({
            "gate_truth": format!("{quest_truth:?}"),
            "gate_checks": gate_checks,
            "loc_id": planned.loc_id,
            "crossing": planned.crossing_id(),
            "route_legs": route.legs.iter().map(|leg| match leg {
                Leg::Walk { tiles } => json!({"kind": "walk", "tiles": tiles}),
                Leg::Transport { edge } => json!({
                    "kind": "transport",
                    "loc_id": edge.loc_id,
                    "at": edge.at,
                    "to": edge.to,
                }),
            }).collect::<Vec<_>>(),
        });
        detail["plan"] = plan;
        receipt["prerequisite_and_crossing"] = json!({"proof": detail.clone()});
        write_receipt(receipt_path, receipt);
        drop(snapshot_guard);

        control.lock().command = KitchenFrameCommand::Open;
        let crossing_result = wait_for_kitchen_crossing(&control, Duration::from_secs(30))
            .unwrap_or_else(|| {
                let snapshot = snapshot.lock();
                json!({
                    "correct": false,
                    "error": "timed out waiting for real kitchen crossing",
                    "tick": snapshot.tick(),
                    "tile": snapshot.tile(),
                })
            });
        detail["movement"] = crossing_result.clone();
        receipt["prerequisite_and_crossing"] = json!({"proof": detail.clone()});
        write_receipt(receipt_path, receipt);
        if crossing_result["correct"] != Value::Bool(true) {
            return Err(format!("real kitchen crossing failed: {crossing_result}"));
        }
        Ok(detail)
    })();

    if proof.is_err() && play.script_lifecycle_receipt(account).is_none() {
        play.script_stop(account);
    }
    control.lock().command = KitchenFrameCommand::Logout;
    let deadline = Instant::now() + Duration::from_secs(30);
    while !control.lock().logout_observed && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    let logout_observed = control.lock().logout_observed;
    let (detail, error) = match proof {
        Ok(proof) => (
            json!({"proof": proof, "logout_observed": logout_observed}),
            None,
        ),
        Err(error) => (
            json!({
                "partial": receipt["prerequisite_and_crossing"].clone(),
                "error": error.clone(),
                "logout_observed": logout_observed,
            }),
            Some(error),
        ),
    };
    drop(play);
    PhoenixPlayResult {
        detail,
        logout_observed,
        error,
    }
}
fn run_brimhaven_phoenix_kitchen_journal() {
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"));
    assert_eq!(std::env::var("BOT_LIVE_NAME_PREFIX").as_deref(), Ok("npr3"));
    let evidence = required_path("NAV_KITCHEN_EVIDENCE");
    fs::create_dir_all(&evidence).unwrap();
    let serial = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let (account, password) = mint_live_entries(&mint_live_names(1)).pop().unwrap();
    let uid = (serial % i32::MAX as u128) as i32;
    let receipt_path = evidence.join(format!("kitchen-phoenix-{account}-{serial}.json"));
    let mut receipt = json!({
        "account": account,
        "server": "Engine Q at 127.0.0.1:44694",
        "seed": null,
        "prerequisite_and_crossing": null,
        "logout_observed": false,
    });
    write_receipt(&receipt_path, &receipt);

    let mut seed_logout = false;
    let mut play_logout = false;
    let setup = (|| -> Result<(), String> {
        let mut options = live_profile_options();
        options.port = Some(44694);
        options.http_port = Some(1180);
        options.catalog_root = Some(PathBuf::from(
            "/Users/acfrazier/experiments/lostcity-289/content",
        ));
        let profile = options
            .resolve(None)
            .map_err(|error| format!("resolve Engine Q profile: {error:?}"))?
            .bind_runtime()
            .map_err(|error| format!("bind Engine Q profile: {error:?}"))?;
        let selected = profile
            .game_data()
            .ok_or_else(|| "Engine Q selected game data unavailable".to_owned())?;
        let template = SharedClientTemplate::load(Arc::clone(&profile))
            .map_err(|error| format!("load Engine Q client template: {error:?}"))?;
        if selected.try_quests().is_none() || selected.try_prereq_steps().is_none() {
            return Err("Engine Q profile did not prepare quest and prerequisite catalogs".into());
        }
        let world = template
            .world()
            .ok_or_else(|| "qualified WORLD_NAV_PACK is unavailable".to_owned())?;
        let edge = world
            .graph
            .edges
            .iter()
            .find(|edge| {
                edge.loc_id == 2628
                    && edge
                        .crossing_id()
                        .is_some_and(|crossing| crossing.takeoff.z == 3190)
            })
            .ok_or_else(|| "qualified kitchen door loc 2628 is unavailable".to_owned())?
            .clone();
        let crossing = edge
            .crossing_id()
            .ok_or_else(|| "qualified kitchen edge has no directed crossing".to_owned())?;

        let (seed_detail, observed_logout, seed_error) =
            seed_phoenix_kitchen_account(&template, uid, &account, &password, crossing);
        seed_logout = observed_logout;
        receipt["seed"] = seed_detail;
        receipt["logout_observed"] = json!(seed_logout);
        write_receipt(&receipt_path, &receipt);
        if let Some(error) = seed_error {
            return Err(error);
        }
        if !seed_logout {
            return Err("seed account did not log out before Engine Q Play".into());
        }

        let play_result = run_phoenix_native_and_crossing(
            PhoenixPlaySetup {
                template,
                selected,
                world,
                edge,
                account: &account,
                password: &password,
                uid,
                map_members: profile.map_members(),
            },
            &receipt_path,
            &mut receipt,
        );
        play_logout = play_result.logout_observed;
        receipt["prerequisite_and_crossing"] = play_result.detail;
        receipt["logout_observed"] = json!(seed_logout && play_logout);
        write_receipt(&receipt_path, &receipt);
        if let Some(error) = play_result.error {
            return Err(error);
        }
        if !play_logout {
            return Err("Engine Q account did not log out after journal proof".into());
        }
        Ok(())
    })();
    let error = setup.err();
    let logout_observed = seed_logout && play_logout;
    receipt["logout_observed"] = json!(logout_observed);
    receipt["result"] = match &error {
        Some(error) => json!({"ok": false, "error": error}),
        None => json!({"ok": true}),
    };
    write_receipt(&receipt_path, &receipt);
    println!("KITCHEN_LIVE {receipt}");
    assert!(
        logout_observed,
        "live Phoenix account must log out: {receipt}"
    );
    assert!(
        error.is_none(),
        "Phoenix journal live proof failed: {receipt}"
    );
}
#[test]
#[ignore = "requires Engine Q at 127.0.0.1:44694/http1180, LIVE=1, WORLD_NAV_PACK, BOT_LIVE_NAME_PREFIX=npr3, and NAV_KITCHEN_EVIDENCE"]
fn live_brimhaven_phoenix_kitchen_crossing() {
    run_brimhaven_phoenix_kitchen_journal();
}

#[test]
#[ignore = "requires Engine L, LIVE=1, qualified WORLD_NAV_PACK, own npr2 prefix and NAV_KITCHEN_EVIDENCE"]
fn live_brimhaven_black_arm_kitchen_refusal() {
    run_brimhaven_black_arm_kitchen_refusal();
}
