//! Ignored headless live proof for NPC-backed boat hops on the local R289 engine.
//!
//! The run uses one disposable `nhop*` account and drives the real
//! `Traveller::follow` loop rather than a scenario stub. It records
//! `TravelEvent` transport observations and requires exact fresh landing
//! snapshots for Port Sarim -> Musa, Musa -> Port Sarim, and Brimhaven ->
//! Ardougne. Run with the shared local engine and a matching nav pack:
//!
//! ```text
//! HOME="$TMPDIR/274bot-nhop-home" LIVE=1 WORLD_NAV_PACK=/path/to/274bot.navpack \
//!   cargo test -p host-play --test npc_hop_live -- --ignored --nocapture --test-threads=1
//! ```

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use api::interact;
use api::snapshot::{GameSnapshot, WorldTile};
use host::Pump;
use host_play::{ProfileOptions, ServerProfile, SharedClientTemplate};
use nav::router::{find_with, FindOptions, Leg};
use nav::transport::TransportKind;
use nav::traveller::{TravelEvent, TravelOptions, TravelOutcome, Traveller};
use nav::world::NavWorld;
use serde_json::json;

const PORT_SARIM_DOCK: WorldTile = WorldTile {
    x: 3029,
    z: 3217,
    level: 0,
};
const MUSA_DOCK: WorldTile = WorldTile {
    x: 2956,
    z: 3146,
    level: 0,
};
const BRIMHAVEN_STAND: WorldTile = WorldTile {
    x: 2772,
    z: 3234,
    level: 0,
};
const ARDOUGNE_DOCK: WorldTile = WorldTile {
    x: 2683,
    z: 3271,
    level: 0,
};
/// Persist a compact route receipt when the live runner is given an evidence
/// directory; stdout remains the primary receipt for ordinary cargo runs.
fn save_receipt(label: &str, receipt: &serde_json::Value) {
    let Some(dir) = std::env::var_os("LIVE_EVIDENCE_DIR") else {
        return;
    };
    let dir = PathBuf::from(dir);
    fs::create_dir_all(&dir).expect("create LIVE_EVIDENCE_DIR");
    let path = dir.join(format!("npc-hop-{label}.json"));
    let bytes = serde_json::to_vec_pretty(receipt).expect("serialize route receipt");
    fs::write(path, bytes).expect("write route receipt");
}

/// Render the same arrival state without creating a native window. PPM keeps
/// the live harness dependency-free; evidence collection can losslessly
/// convert this actual CPU framebuffer to PNG.
fn save_arrival_frame(label: &str, client: &mut client::client::Client) {
    let Some(dir) = std::env::var_os("LIVE_EVIDENCE_DIR") else {
        return;
    };
    let mut renderer = client::render::Renderer::new_prefer(client.config.lowmem, false);
    let client::render::backend::FrameOutput::PixMap(pixels) = renderer.mainredraw(client) else {
        panic!("CPU arrival capture returned a GPU frame");
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
    let path = PathBuf::from(dir).join(format!("npc-hop-{label}.ppm"));
    fs::write(path, ppm).expect("write actual arrival framebuffer");
}

fn selected() -> (Arc<ServerProfile>, Arc<SharedClientTemplate>) {
    let nav_pack = PathBuf::from(
        std::env::var("WORLD_NAV_PACK")
            .expect("WORLD_NAV_PACK must point at the local R289 navigation pack"),
    );
    let options = ProfileOptions {
        profile: Some("local-289".into()),
        revision: Some("289".into()),
        host: Some("127.0.0.1".into()),
        port: Some(45594),
        asset_host: Some("127.0.0.1".into()),
        http_port: Some(2080),
        nav_pack: Some(nav_pack),
        nav_flags: std::env::var_os("WORLD_NAV_FLAGS").map(PathBuf::from),
        engine_dir: std::env::var_os("WORLD_ENGINE_DIR").map(PathBuf::from),
        cache_dir: std::env::var_os("WORLD_CACHE_DIR").map(PathBuf::from),
        ..ProfileOptions::default()
    };
    let profile = options
        .resolve(None)
        .expect("R289 local profile resolve")
        .bind()
        .expect("R289 local profile bind");
    assert_eq!(profile.profile_class(), host_play::ProfileClass::Local);
    assert_eq!(profile.client().game_host(), "127.0.0.1");
    assert_eq!(profile.client().game_port(), 45594);
    assert_eq!(profile.client().asset_port(), 2080);
    let template = SharedClientTemplate::load(Arc::clone(&profile))
        .expect("selected R289 template and nav pack load");
    assert!(template.world().is_some(), "selected nav pack must load");
    (profile, template)
}

fn pump_once(client: &mut client::client::Client, snapshot: &mut GameSnapshot, pump: &mut Pump) {
    client.mainloop();
    host::publish_snapshot(snapshot, client, pump.drain_client(client));
}

fn wait_for(
    client: &mut client::client::Client,
    snapshot: &mut GameSnapshot,
    pump: &mut Pump,
    timeout: Duration,
    label: &str,
    ready: impl Fn(&GameSnapshot) -> bool,
) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        pump_once(client, snapshot, pump);
        if ready(snapshot) {
            return;
        }
        thread::sleep(Duration::from_millis(20));
    }
    panic!("{label}: timed out at {:?}", snapshot.tile());
}

fn tele_to(
    client: &mut client::client::Client,
    snapshot: &mut GameSnapshot,
    pump: &mut Pump,
    tile: WorldTile,
) {
    assert!(interact::cheat(client, &interact::tele_args(tile.level, tile.x, tile.z)).is_sent());
    wait_for(
        client,
        snapshot,
        pump,
        Duration::from_secs(30),
        "teleport",
        |s| s.ingame() && s.scene_state() == 2 && s.tile() == Some((tile.x, tile.z, tile.level)),
    );
}

fn give_coins(client: &mut client::client::Client, snapshot: &mut GameSnapshot, pump: &mut Pump) {
    assert!(interact::cheat(client, "give coins 200").is_sent());
    wait_for(
        client,
        snapshot,
        pump,
        Duration::from_secs(30),
        "coin grant",
        |s| s.inv_count(995) >= 30,
    );
}

fn follow_boat(
    client: &mut client::client::Client,
    snapshot: &mut GameSnapshot,
    pump: &mut Pump,
    world: &NavWorld,
    from: WorldTile,
    dest: WorldTile,
    label: &str,
) -> TravelOutcome {
    assert_eq!(
        snapshot.tile(),
        Some((from.x, from.z, from.level)),
        "{label}: source tile"
    );
    let state = nav::WorldState::from_snapshot(snapshot).with_map_members(true);
    let route = find_with(
        &world.collision,
        &world.graph,
        from,
        dest,
        FindOptions::default(),
        &state,
    )
    .unwrap_or_else(|error| {
        let nearby: Vec<_> = world
            .graph
            .edges
            .iter()
            .filter(|edge| {
                edge.kind == TransportKind::Boat
                    && (edge.at.x - from.x).abs() < 16
                    && (edge.at.z - from.z).abs() < 16
            })
            .collect();
        panic!("{label}: route {from:?}->{dest:?}: {error:?}; nearby boats={nearby:?}");
    });
    assert!(
        route
            .legs
            .iter()
            .any(|leg| matches!(leg, Leg::Transport { edge } if edge.kind == TransportKind::Boat)),
        "{label}: route must contain a Boat transport leg, got {route:?}"
    );
    let mut traveller = Traveller::new();
    let (outcome, events, saw_boat_state, saw_boat_attempt) = {
        let mut events = Vec::new();
        let mut saw_boat_state = false;
        let mut saw_boat_attempt = false;
        let mut options = TravelOptions {
            close_enough: 0,
            teleports: Some(world.graph.teleports.as_slice()),
            edges: Some(world.graph.edges.as_slice()),
            on_event: Some(Box::new(|event: TravelEvent| {
                match &event {
                    TravelEvent::TransportState { kind, .. } if *kind == TransportKind::Boat => {
                        saw_boat_state = true;
                    }
                    TravelEvent::TransportAttempt { kind, refusal, .. }
                        if *kind == TransportKind::Boat && refusal.is_none() =>
                    {
                        saw_boat_attempt = true;
                    }
                    _ => {}
                }
                events.push(format!("{event:?}"));
            })),
            ..TravelOptions::default()
        };
        let deadline = Instant::now() + Duration::from_secs(240);
        let mut last_tick = None;
        let outcome = loop {
            pump_once(client, snapshot, pump);
            let tick = snapshot.tick();
            if last_tick != Some(tick) {
                last_tick = Some(tick);
                if let Some(outcome) =
                    traveller.follow(client, snapshot, route.clone(), &mut options)
                {
                    break outcome;
                }
            }
            assert!(
                Instant::now() < deadline,
                "{label}: follow timed out at {:?}",
                snapshot.tile()
            );
            thread::sleep(Duration::from_millis(20));
        };
        drop(options);
        (outcome, events, saw_boat_state, saw_boat_attempt)
    };
    assert!(
        matches!(&outcome, TravelOutcome::Arrived { at } if *at == dest),
        "{label}: expected fresh exact arrival at {dest:?}, got {outcome:?}; events={events:?}"
    );
    assert!(
        snapshot.ingame() && snapshot.attached() && snapshot.scene_state() == 2,
        "{label}: arrival must remain ingame/scene2, got ingame={} attached={} scene={}",
        snapshot.ingame(),
        snapshot.attached(),
        snapshot.scene_state()
    );
    assert_eq!(
        snapshot.tile(),
        Some((dest.x, dest.z, dest.level)),
        "{label}: arrival snapshot must be fresh"
    );
    assert!(
        saw_boat_state,
        "{label}: TravelEvent evidence did not include Boat transport state: {events:?}"
    );
    assert!(
        saw_boat_attempt,
        "{label}: TravelEvent evidence did not include a successful Boat transport attempt: {events:?}"
    );
    let receipt = json!({
        "from": from,
        "to": dest,
        "outcome": format!("{outcome:?}"),
        "transport_event_count": events.len(),
        "boat_transport_state": saw_boat_state,
        "boat_transport_attempt": saw_boat_attempt,
        "fresh_arrival": snapshot.tile(),
        "ingame": snapshot.ingame(),
        "attached": snapshot.attached(),
        "scene_state": snapshot.scene_state(),
    });
    save_receipt(label, &receipt);
    save_arrival_frame(label, client);
    println!("PASS: npc_hop_live {label} {receipt}");
    outcome
}

#[test]
#[ignore = "requires the shared local R289 engine/nav pack and LIVE=1"]
fn npc_hop_live_boat_routes() {
    if std::env::var("LIVE").as_deref() != Ok("1") {
        return;
    }
    let (_profile, template) = selected();
    let world = template.world().expect("selected nav world");
    let serial = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_millis();
    let uid = (serial % 100_000_000) as i32;
    let name = format!("nhop{uid:08}");
    let mut client = template
        .prepare_client(uid, true)
        .expect("prepare disposable R289 client");
    client.set_draw(std::env::var_os("LIVE_EVIDENCE_DIR").is_some());
    client.maininit();
    assert!(
        !client.error_loading,
        "R289 client asset initialization failed"
    );
    let mut snapshot = GameSnapshot::new();
    let mut pump = Pump::new();
    assert!(
        interact::login(&mut client, &name, &name, false),
        "login failed"
    );
    wait_for(
        &mut client,
        &mut snapshot,
        &mut pump,
        Duration::from_secs(120),
        "initial scene",
        |s| s.ingame() && s.attached() && s.scene_state() == 2 && s.tile().is_some(),
    );

    // Three independent fresh-arrival proofs consume one shared 200-coin
    // grant: Port Sarim -> Musa, Musa -> Port Sarim, Brimhaven -> Ardougne.
    tele_to(&mut client, &mut snapshot, &mut pump, PORT_SARIM_DOCK);
    give_coins(&mut client, &mut snapshot, &mut pump);
    follow_boat(
        &mut client,
        &mut snapshot,
        &mut pump,
        world.as_ref(),
        PORT_SARIM_DOCK,
        MUSA_DOCK,
        "port-sarim-to-musa",
    );
    follow_boat(
        &mut client,
        &mut snapshot,
        &mut pump,
        world.as_ref(),
        MUSA_DOCK,
        PORT_SARIM_DOCK,
        "musa-to-port-sarim",
    );
    tele_to(&mut client, &mut snapshot, &mut pump, BRIMHAVEN_STAND);
    follow_boat(
        &mut client,
        &mut snapshot,
        &mut pump,
        world.as_ref(),
        BRIMHAVEN_STAND,
        ARDOUGNE_DOCK,
        "brimhaven-to-ardougne",
    );
    println!("PASS: npc_hop_live all required boat routes completed for {name}");
    if client.ingame {
        client.logout();
    }
}
