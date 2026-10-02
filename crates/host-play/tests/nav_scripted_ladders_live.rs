//! Fresh local-289 account proof of content-derived Mage Arena cellar routing.
use std::cell::Cell;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use api::interact;
use api::snapshot::{GameSnapshot, WorldTile};
use client::client::Client;
use host::Pump;
use host_play::{ProfileOptions, SharedClientTemplate};
use nav::router::{find_with, FindOptions, Leg};
use nav::traveller::{TravelEvent, TravelOptions, TravelOutcome, Traveller};
use serde_json::json;

const ORIGIN: WorldTile = WorldTile {
    x: 3092,
    z: 3957,
    level: 0,
};
const BANK: WorldTile = WorldTile {
    x: 2534,
    z: 4713,
    level: 0,
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
            "setup timed out at {:?}",
            snapshot.tile()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
#[ignore = "requires LIVE=1, WORLD_ENGINE_DIR, WORLD_NAV_PACK, NAV_SCRIPTED_LADDERS_RECEIPT and disposable HOME"]
fn live_mage_arena_webs_cellar_and_gundai_arrive() {
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"));
    let serial = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let prefix = std::env::var("BOT_LIVE_NAME_PREFIX").unwrap_or_else(|_| "ml".into());
    let name = format!("{prefix}{}", serial % 1_000_000_000);
    let receipt_path =
        PathBuf::from(std::env::var_os("NAV_SCRIPTED_LADDERS_RECEIPT").expect("receipt path"));
    let profile = ProfileOptions {
        profile: Some("local-289".into()),
        revision: Some("289".into()),
        host: Some("127.0.0.1".into()),
        asset_host: Some("127.0.0.1".into()),
        port: Some(45594),
        http_port: Some(2080),
        engine_dir: Some(PathBuf::from(
            std::env::var_os("WORLD_ENGINE_DIR").expect("WORLD_ENGINE_DIR"),
        )),
        nav_pack: Some(PathBuf::from(
            std::env::var_os("WORLD_NAV_PACK").expect("WORLD_NAV_PACK"),
        )),
        ..ProfileOptions::default()
    }
    .resolve(None)
    .expect("local profile")
    .bind()
    .expect("bound local profile");
    assert_eq!(profile.profile_class(), host_play::ProfileClass::Local);
    let template = SharedClientTemplate::load(Arc::clone(&profile)).expect("client template");
    let world = template.world().expect("explicit world pack");
    let mut client = template
        .prepare_client((serial % 1_000_000_000) as i32, true)
        .unwrap();
    client.draw = false;
    client.maininit();
    assert!(!client.error_loading);
    assert!(interact::login(&mut client, &name, &name, false));
    let mut snapshot = GameSnapshot::new();
    let mut pump = Pump::new();
    wait_for(&mut client, &mut snapshot, &mut pump, |s| {
        s.ingame() && s.attached() && s.scene_state() == 2
    });
    assert!(interact::cheat(&mut client, "setvar tutorial 1000").is_sent());
    assert!(interact::cheat(&mut client, "getvar tutorial").is_sent());
    wait_for(&mut client, &mut snapshot, &mut pump, |s| {
        s.chat_lines()
            .iter()
            .any(|line| line.text.contains("get tutorial: 1000"))
    });
    let ifaces = Arc::clone(&client.ifaces);
    assert!(interact::logout(&mut client, &ifaces));
    wait_for(&mut client, &mut snapshot, &mut pump, |s| !s.ingame());
    assert!(interact::login(&mut client, &name, &name, false));
    wait_for(&mut client, &mut snapshot, &mut pump, |s| {
        s.ingame() && s.scene_state() == 2
    });
    // Reuse the gatherer fixture's local setup cheats; the only placement is
    // the entrance inside the webs, never a cellar/route-hop teleport.
    interact::seed_at(&mut client, ORIGIN.level, ORIGIN.x, ORIGIN.z);
    for skill in [
        "attack",
        "strength",
        "defence",
        "hitpoints",
        "ranged",
        "magic",
        "prayer",
    ] {
        assert!(interact::cheat(&mut client, &format!("setstat {skill} 99")).is_sent());
    }
    wait_for(&mut client, &mut snapshot, &mut pump, |s| {
        s.ingame()
            && s.scene_state() == 2
            && s.tile() == Some((ORIGIN.x, ORIGIN.z, ORIGIN.level))
            && nav::WorldState::from_snapshot(s)
                .combat_level
                .is_some_and(|level| level >= 100)
    });
    let state = nav::WorldState::from_snapshot(&snapshot).with_map_members(profile.map_members());
    let route = find_with(
        &world.collision,
        &world.graph,
        ORIGIN,
        BANK,
        FindOptions {
            allow_wilderness: true,
            allow_teleports: false,
            allow_bank_fetch: false,
            ..FindOptions::default()
        },
        &state,
    )
    .expect("cellar route");
    assert!(route
        .legs
        .iter()
        .any(|leg| matches!(leg, Leg::Transport { edge } if edge.loc_id == 2871)));
    let cellar_edge = route
        .legs
        .iter()
        .find_map(|leg| match leg {
            Leg::Transport { edge } if edge.loc_id == 2871 => Some(edge.clone()),
            _ => None,
        })
        .unwrap();
    let mut traveller = Traveller::new();
    let mut events = Vec::new();
    let mut tiles = Vec::new();
    let mut last_tile = None;
    let mut saw_cellar_attempt = false;
    let mut legs = Vec::new();
    let observed_tile = Cell::new(None);
    let cellar_sent_tile = Cell::new(None);
    let mut cellar_landing = None;
    let mut options = TravelOptions {
        close_enough: 0,
        edges: Some(&world.graph.edges),
        on_event: Some(Box::new(|event| {
            if matches!(
                &event,
                TravelEvent::TransportAttempt {
                    expected_id: 2871,
                    refusal: None,
                    ..
                }
            ) {
                saw_cellar_attempt = true;
                cellar_sent_tile.set(observed_tile.get());
            }
            events.push(format!("{event:?}"));
        })),
        on_leg: Some(Box::new(|leg, phase| {
            legs.push(format!("{phase:?}: {leg:?}"))
        })),
        ..TravelOptions::default()
    };
    let deadline = Instant::now() + Duration::from_secs(180);
    let mut last_tick = None;
    let outcome = loop {
        pump_once(&mut client, &mut snapshot, &mut pump);
        let here = api::snapshot::ReadContext::new(&snapshot).world_tile();
        observed_tile.set(here);
        if cellar_landing.is_none() {
            if let (Some(sent), Some(here)) = (cellar_sent_tile.get(), here) {
                if (here.x - sent.x).abs().max((here.z - sent.z).abs()) > 64 {
                    cellar_landing = Some(here);
                }
            }
        }
        if snapshot.tile() != last_tile {
            last_tile = snapshot.tile();
            tiles.push(json!({"tick": snapshot.tick(), "tile": last_tile}));
        }
        if last_tick != Some(snapshot.tick()) {
            last_tick = Some(snapshot.tick());
            if let Some(outcome) =
                traveller.follow(&mut client, &snapshot, route.clone(), &mut options)
            {
                break Some(outcome);
            }
        }
        if Instant::now() >= deadline {
            break None;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    drop(options);
    let gundai = snapshot.npcs().iter().find(|npc| npc.r#type == Some(902));
    let gundai_observed =
        gundai.map(|npc| json!({"id": npc.r#type, "name": npc.name, "tile": npc.tile}));
    let receipt = json!({"account": name, "origin": ORIGIN, "bank_stand": BANK,
        "combat_level": state.combat_level, "route": format!("{route:?}"), "events": events, "legs": legs, "tiles": tiles,
        "outcome": format!("{outcome:?}"), "cellar_attempt": saw_cellar_attempt,
        "cellar_edge": {"at": cellar_edge.at, "to": cellar_edge.to, "loc_id": cellar_edge.loc_id, "option": cellar_edge.option, "ticks": cellar_edge.ticks},
        "cellar_sent_tile": cellar_sent_tile.get(), "cellar_landing": cellar_landing,
        "final_tile": snapshot.tile(), "ingame": snapshot.ingame(), "scene_state": snapshot.scene_state(),
        "gundai": gundai_observed});
    std::fs::create_dir_all(receipt_path.parent().expect("receipt parent")).unwrap();
    std::fs::write(receipt_path, serde_json::to_vec_pretty(&receipt).unwrap()).unwrap();
    println!("{}", serde_json::to_string_pretty(&receipt).unwrap());
    client.logout();
    assert!(
        matches!(outcome, Some(TravelOutcome::Arrived { at }) if at == BANK),
        "{receipt}"
    );
    assert!(saw_cellar_attempt, "{receipt}");
    assert!(gundai.is_some(), "{receipt}");
    let sent = cellar_sent_tile
        .get()
        .expect("cellar transport attempt records its takeoff");
    let delta = cellar_edge
        .player_delta
        .expect("cellar edge uses a player-relative landing");
    let expected_landing = WorldTile {
        x: sent.x.checked_add(delta.x).expect("cellar x landing fits"),
        z: sent.z.checked_add(delta.z).expect("cellar z landing fits"),
        level: sent
            .level
            .checked_add(delta.level)
            .expect("cellar level landing fits"),
    };
    assert_eq!(cellar_landing, Some(expected_landing), "{receipt}");
    assert_eq!(receipt["scene_state"], 2);
}

/// A fresh account is staged on the east stand of the Viking seer ladder.
/// The route and live settlement must both use the actual take-off tile, not
/// the blocked ladder-top tile at the loc anchor.
#[test]
#[ignore = "requires LIVE=1, WORLD_ENGINE_DIR, WORLD_NAV_PACK, NAV_RELATIVE_LADDER_RECEIPT and disposable HOME"]
fn live_viking_seer_ladder_uses_player_relative_landing() {
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"));
    let prefix = std::env::var("BOT_LIVE_NAME_PREFIX").unwrap_or_else(|_| "nr".into());
    assert!(!prefix.is_empty());
    let serial = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let name = format!("{prefix}{}", serial % 1_000_000_000);
    let receipt_path =
        PathBuf::from(std::env::var_os("NAV_RELATIVE_LADDER_RECEIPT").expect("receipt path"));
    let origin = WorldTile {
        x: 2632,
        z: 3663,
        level: 0,
    };
    let destination = WorldTile { level: 2, ..origin };
    let delta = WorldTile {
        x: 0,
        z: 0,
        level: 2,
    };
    let profile = ProfileOptions {
        profile: Some("local-289".into()),
        revision: Some("289".into()),
        host: Some("127.0.0.1".into()),
        asset_host: Some("127.0.0.1".into()),
        port: Some(45594),
        http_port: Some(2080),
        engine_dir: Some(PathBuf::from(
            std::env::var_os("WORLD_ENGINE_DIR").expect("WORLD_ENGINE_DIR"),
        )),
        nav_pack: Some(PathBuf::from(
            std::env::var_os("WORLD_NAV_PACK").expect("WORLD_NAV_PACK"),
        )),
        ..ProfileOptions::default()
    }
    .resolve(None)
    .expect("local profile")
    .bind()
    .expect("bound local profile");
    let template = SharedClientTemplate::load(Arc::clone(&profile)).expect("client template");
    let world = template.world().expect("explicit world pack");
    let ladder_edge = world
        .graph
        .edges
        .iter()
        .find(|edge| edge.loc_id == 4163)
        .expect("Viking seer ladder edge in packed world");
    assert_eq!(ladder_edge.player_delta, Some(delta));
    assert_eq!(
        ladder_edge.at,
        WorldTile {
            x: 2631,
            z: 3663,
            level: 0,
        }
    );
    let route = find_with(
        &world.collision,
        &world.graph,
        origin,
        destination,
        FindOptions::default(),
        &nav::WorldState::empty().with_map_members(profile.map_members()),
    )
    .expect("non-anchor Viking seer ladder route");
    let planned_edge = route
        .legs
        .iter()
        .find_map(|leg| match leg {
            Leg::Transport { edge } if edge.loc_id == 4163 => Some(edge.clone()),
            _ => None,
        })
        .expect("route crosses Viking seer ladder");
    assert_eq!(planned_edge.to, destination);

    let mut client = template
        .prepare_client((serial % 1_000_000_000) as i32, true)
        .unwrap();
    client.draw = false;
    client.maininit();
    assert!(!client.error_loading);
    assert!(interact::login(&mut client, &name, &name, false));
    let mut snapshot = GameSnapshot::new();
    let mut pump = Pump::new();
    wait_for(&mut client, &mut snapshot, &mut pump, |s| {
        s.ingame() && s.attached() && s.scene_state() == 2
    });
    // The new account is staged at the real non-anchor take-off stand; this
    // ladder has no quest, skill, or varp gate in its content handler.
    interact::seed_at(&mut client, origin.level, origin.x, origin.z);
    wait_for(&mut client, &mut snapshot, &mut pump, |s| {
        s.ingame()
            && s.scene_state() == 2
            && api::snapshot::ReadContext::new(s).world_tile() == Some(origin)
    });
    let mut traveller = Traveller::new();
    let observed_tile = Cell::new(None);
    let sent_tile = Cell::new(None);
    let mut saw_attempt = false;
    let mut observed_landing = None;
    let mut events = Vec::new();
    let mut options = TravelOptions {
        close_enough: 0,
        edges: Some(&world.graph.edges),
        on_event: Some(Box::new(|event| {
            if matches!(
                &event,
                TravelEvent::TransportAttempt {
                    expected_id: 4163,
                    refusal: None,
                    ..
                }
            ) {
                saw_attempt = true;
                sent_tile.set(observed_tile.get());
            }
            events.push(format!("{event:?}"));
        })),
        ..TravelOptions::default()
    };
    let deadline = Instant::now() + Duration::from_secs(180);
    let mut last_tick = None;
    let outcome = loop {
        pump_once(&mut client, &mut snapshot, &mut pump);
        let here = api::snapshot::ReadContext::new(&snapshot).world_tile();
        observed_tile.set(here);
        if observed_landing.is_none() {
            if let (Some(sent), Some(here)) = (sent_tile.get(), here) {
                if here != sent {
                    observed_landing = Some(here);
                }
            }
        }
        if last_tick != Some(snapshot.tick()) {
            last_tick = Some(snapshot.tick());
            if let Some(outcome) =
                traveller.follow(&mut client, &snapshot, route.clone(), &mut options)
            {
                break Some(outcome);
            }
        }
        if Instant::now() >= deadline {
            break None;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    drop(options);
    let sent = sent_tile.get();
    let expected_landing = sent.map(|sent| WorldTile {
        x: sent.x.checked_add(delta.x).expect("landing x fits"),
        z: sent.z.checked_add(delta.z).expect("landing z fits"),
        level: sent
            .level
            .checked_add(delta.level)
            .expect("landing level fits"),
    });
    let receipt = json!({
        "account": name,
        "loc_id": 4163,
        "origin": origin,
        "destination": destination,
        "delta": delta,
        "route": format!("{route:?}"),
        "planned_edge": {"at": planned_edge.at, "to": planned_edge.to, "player_delta": planned_edge.player_delta},
        "events": events,
        "attempt": saw_attempt,
        "sent_tile": sent,
        "observed_landing": observed_landing,
        "expected_landing": expected_landing,
        "outcome": format!("{outcome:?}"),
        "final_tile": snapshot.tile(),
        "ingame": snapshot.ingame(),
        "scene_state": snapshot.scene_state()
    });
    if let Some(parent) = receipt_path.parent() {
        std::fs::create_dir_all(parent).expect("receipt directory");
    }
    std::fs::write(
        &receipt_path,
        serde_json::to_vec_pretty(&receipt).expect("serialize receipt"),
    )
    .expect("write receipt");
    println!("{}", serde_json::to_string_pretty(&receipt).unwrap());
    client.logout();
    assert!(
        matches!(outcome, Some(TravelOutcome::Arrived { at }) if at == destination),
        "{receipt}"
    );
    assert!(saw_attempt, "{receipt}");
    assert_eq!(sent, Some(origin), "{receipt}");
    assert_eq!(observed_landing, expected_landing, "{receipt}");
    assert_eq!(expected_landing, Some(destination), "{receipt}");
    assert_eq!(
        snapshot.tile(),
        Some((destination.x, destination.z, destination.level))
    );
    assert_eq!(snapshot.scene_state(), 2);
}
