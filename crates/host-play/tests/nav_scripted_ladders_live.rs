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
    let prefix = std::env::var("BOT_LIVE_NAME_PREFIX").unwrap_or_else(|_| "ms".into());
    assert!(prefix.starts_with("ms"), "use agent-owned ms accounts");
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
    assert_eq!(receipt["scene_state"], 2);
}
