//! Ignored headless live proof for the Al Kharid toll-gate hop on the local
//! R289 engine.
//!
//! WalkArm follows Lumbridge ↔ Al Kharid through the border gate (loc
//! 2882/2883 at (3268,3227)/(3268,3228)). The gate's Open starts the
//! border-guard `p_choice3` dialogue; the hop must answer the content's
//! "Yes, ok." pay option, not stall. A constructed hop without coins must
//! refuse instead of paying. Run with the shared local engine:
//!
//! ```text
//! HOME="$TMPDIR/274bot-tollg-home" LIVE=1 WORLD_NAV_PACK=/path/to/274bot.navpack \
//!   cargo test -p host-play --test toll_gate_live -- --ignored --nocapture --test-threads=1
//! ```

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use api::interact;
use api::snapshot::{GameSnapshot, WorldTile};
use host::Pump;
use host_play::{
    arm_walk_on, step_walk_arm_follow, ProfileOptions, SharedClientTemplate, WalkArms,
};
use nav::bank_fetch::BankRows;
use nav::router::{FindOptions, Leg, Route};
use nav::tile::Tile;
use nav::transport::{DoorDir, TransportKind};
use nav::traveller::{TravelOptions, TravelOutcome, Traveller};
use nav::world::NavWorld;
use nav::WorldState;
use serde_json::json;

const LUMBRIDGE: WorldTile = WorldTile {
    x: 3222,
    z: 3218,
    level: 0,
};
/// Al Kharid side of the border, a few tiles east of the gate.
const AL_KHARID: WorldTile = WorldTile {
    x: 3275,
    z: 3227,
    level: 0,
};
const GATE_WEST: WorldTile = WorldTile {
    x: 3264,
    z: 3227,
    level: 0,
};
const GATE_EAST: WorldTile = WorldTile {
    x: 3272,
    z: 3227,
    level: 0,
};
const TOLL_LEFT: i32 = 2882;
const TOLL_RIGHT: i32 = 2883;
const COINS: i32 = 995;

fn save_receipt(label: &str, receipt: &serde_json::Value) {
    let Some(dir) = std::env::var_os("LIVE_EVIDENCE_DIR") else {
        return;
    };
    let dir = PathBuf::from(dir);
    fs::create_dir_all(&dir).expect("create LIVE_EVIDENCE_DIR");
    let path = dir.join(format!("toll-gate-{label}.json"));
    let bytes = serde_json::to_vec_pretty(receipt).expect("serialize receipt");
    fs::write(path, bytes).expect("write receipt");
}

fn env_port(name: &str, default: u16) -> u16 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn selected() -> (Arc<host_play::ServerProfile>, Arc<SharedClientTemplate>) {
    let nav_pack = PathBuf::from(
        std::env::var("WORLD_NAV_PACK")
            .expect("WORLD_NAV_PACK must point at the local R289 navigation pack"),
    );
    // The shared-engine convention: WORLD_GAME_PORT/WORLD_HTTP_PORT select
    // another local engine; the defaults are this fixture's original ones.
    let port = env_port("WORLD_GAME_PORT", 45594);
    let http_port = env_port("WORLD_HTTP_PORT", 2080);
    let options = ProfileOptions {
        profile: Some("local-289".into()),
        revision: Some("289".into()),
        host: Some("127.0.0.1".into()),
        port: Some(port),
        asset_host: Some("127.0.0.1".into()),
        http_port: Some(http_port),
        nav_pack: Some(nav_pack),
        nav_flags: std::env::var_os("WORLD_NAV_FLAGS").map(PathBuf::from),
        engine_dir: std::env::var_os("WORLD_ENGINE_DIR")
            .or_else(|| std::env::var_os("BOT_NAV_ENGINE_DIR"))
            .map(PathBuf::from),
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
    assert_eq!(profile.client().game_port(), port);
    assert_eq!(profile.client().asset_port(), http_port);
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
    interact::seed_at(client, tile.level, tile.x, tile.z);
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
        |s| s.inv_count(COINS) >= 10,
    );
}

fn login_fresh(
    template: &SharedClientTemplate,
    prefix: &str,
) -> (String, client::client::Client, GameSnapshot, Pump) {
    let serial = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_millis();
    let uid = (serial % 100_000_000) as i32;
    let name = format!("{prefix}{uid:08}");
    let mut client = template
        .prepare_client(uid, true)
        .expect("prepare disposable R289 client");
    client.set_draw(false);
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
    (name, client, snapshot, pump)
}

fn route_uses_toll(route: &Route) -> bool {
    route.legs.iter().any(|leg| {
        matches!(
            leg,
            Leg::Transport { edge }
                if edge.kind == TransportKind::Door
                    && (edge.loc_id == TOLL_LEFT || edge.loc_id == TOLL_RIGHT)
        )
    })
}

fn walk_arm_to(
    client: &mut client::client::Client,
    snapshot: &mut GameSnapshot,
    pump: &mut Pump,
    world: &NavWorld,
    name: &str,
    from_to: (WorldTile, WorldTile),
    label: &str,
) {
    let (from, dest) = from_to;
    assert_eq!(
        snapshot.tile(),
        Some((from.x, from.z, from.level)),
        "{label}: source tile"
    );
    assert!(
        snapshot.inv_count(COINS) >= 10,
        "{label}: need 10+ coins, have {}",
        snapshot.inv_count(COINS)
    );
    let state = WorldState::from_snapshot(snapshot).with_map_members(true);
    let arms = WalkArms::default();
    let admission = host_play::admission::Admission::manual(
        FindOptions::default(),
        host_play::admission::capture(snapshot, state.map_members, Default::default(), false),
        0,
        host_play::WalkGlobals::default(),
    );
    let route = arm_walk_on(
        world,
        Tile {
            x: from.x,
            z: from.z,
            level: from.level,
        },
        Tile {
            x: dest.x,
            z: dest.z,
            level: dest.level,
        },
        FindOptions::default(),
        &state,
        &BankRows::default(),
        admission,
        &arms,
        Some(name),
    )
    .unwrap_or_else(|_| panic!("{label}: NoPath {from:?}->{dest:?}"))
    .route;
    assert!(
        route_uses_toll(&route),
        "{label}: route must take a toll-gate Door hop, got {route:?}"
    );
    let arm = Arc::clone(&arms.lock().expect("walk arms")[name]);
    let start_tick = snapshot.tick();
    let deadline = Instant::now() + Duration::from_secs(180);
    let mut last_tick = None;
    let terminal_here = loop {
        pump_once(client, snapshot, pump);
        let tick = snapshot.tick();
        if last_tick != Some(tick) {
            last_tick = Some(tick);
            if let Some(here) = snapshot.tile() {
                let mut arm = arm.lock().expect("walk arm");
                step_walk_arm_follow(
                    client,
                    snapshot,
                    &mut arm,
                    Some(world),
                    here,
                    state.map_members,
                    Some(name),
                );
                if arm.route.is_none() {
                    break here;
                }
            }
        }
        assert!(
            Instant::now() < deadline,
            "{label}: WalkArm timed out at {:?} tick={}",
            snapshot.tile(),
            snapshot.tick()
        );
        thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(
        terminal_here,
        (dest.x, dest.z, dest.level),
        "{label}: WalkTo terminal before arrival; tick={} start_tick={} coins={} chat={:?} options={:?}",
        snapshot.tick(),
        start_tick,
        snapshot.inv_count(COINS),
        snapshot.chat(),
        snapshot
            .chat_options()
            .iter()
            .map(|o| o.text.as_str())
            .collect::<Vec<_>>()
    );
    assert!(
        snapshot.ingame() && snapshot.attached() && snapshot.scene_state() == 2,
        "{label}: arrival must remain ingame/scene2"
    );
    let receipt = json!({
        "from": from,
        "to": dest,
        "fresh_arrival": snapshot.tile(),
        "ticks": snapshot.tick().saturating_sub(start_tick),
        "coins": snapshot.inv_count(COINS),
        "ingame": snapshot.ingame(),
        "scene_state": snapshot.scene_state(),
    });
    save_receipt(label, &receipt);
    println!("PASS: toll_gate_live {label} {receipt}");
}

#[test]
#[ignore = "requires the shared local R289 engine/nav pack and LIVE=1"]
fn toll_gate_live_walk_both_ways() {
    if std::env::var("LIVE").as_deref() != Ok("1") {
        return;
    }
    let (_profile, template) = selected();
    let world = template.world().expect("selected nav world");
    let (name, mut client, mut snapshot, mut pump) = login_fresh(&template, "toll");

    tele_to(&mut client, &mut snapshot, &mut pump, LUMBRIDGE);
    give_coins(&mut client, &mut snapshot, &mut pump);
    walk_arm_to(
        &mut client,
        &mut snapshot,
        &mut pump,
        world.as_ref(),
        &name,
        (LUMBRIDGE, AL_KHARID),
        "lumbridge-to-al-kharid",
    );

    for n in 1..=3 {
        tele_to(&mut client, &mut snapshot, &mut pump, GATE_WEST);
        walk_arm_to(
            &mut client,
            &mut snapshot,
            &mut pump,
            world.as_ref(),
            &name,
            (GATE_WEST, GATE_EAST),
            &format!("west-to-east-{n}"),
        );
        walk_arm_to(
            &mut client,
            &mut snapshot,
            &mut pump,
            world.as_ref(),
            &name,
            (GATE_EAST, GATE_WEST),
            &format!("east-to-west-{n}"),
        );
    }
    println!("PASS: toll_gate_live WalkArm both ways x3 plus Lumbridge→Al Kharid for {name}");
    if client.ingame {
        client.logout();
    }
}

#[test]
#[ignore = "requires the shared local R289 engine/nav pack and LIVE=1"]
fn toll_gate_live_refuses_without_coins() {
    if std::env::var("LIVE").as_deref() != Ok("1") {
        return;
    }
    let (_profile, template) = selected();
    let world = template.world().expect("selected nav world");
    let (name, mut client, mut snapshot, mut pump) = login_fresh(&template, "tln");

    // v11 reverse geometry lands eastbound on the loc (`to == at`, dir E),
    // not past it (`to.x > at.x` is the old far-side packing).
    let edge = world
        .graph
        .edges
        .iter()
        .find(|edge| {
            edge.kind == TransportKind::Door
                && (edge.loc_id == TOLL_LEFT || edge.loc_id == TOLL_RIGHT)
                && edge.dir == Some(DoorDir::E)
                // NAV-FARES bakes the 10-coin fare as consumed, not held.
                && edge.consumed_req.iter().any(|(id, n)| *id == COINS && *n >= 10)
        })
        .cloned()
        .expect("paid eastbound Al Kharid toll Door edge");
    let stand = WorldTile {
        x: edge.at.x - 1,
        z: edge.at.z,
        level: 0,
    };
    interact::cheat(&mut client, "~clearinv");
    tele_to(&mut client, &mut snapshot, &mut pump, stand);
    wait_for(
        &mut client,
        &mut snapshot,
        &mut pump,
        Duration::from_secs(15),
        "empty inventory",
        |s| s.inv_count(COINS) == 0,
    );
    assert_eq!(snapshot.inv_count(COINS), 0);

    let dest = edge.to;
    let route = Route {
        dest,
        ticks: 1.0,
        legs: vec![Leg::Transport {
            edge: Box::new(edge),
        }],
    };
    let mut traveller = Traveller::new();
    let mut options = TravelOptions {
        close_enough: 0,
        edges: Some(world.graph.edges.as_slice()),
        ..TravelOptions::default()
    };
    let deadline = Instant::now() + Duration::from_secs(90);
    let mut last_tick = None;
    let outcome = loop {
        pump_once(&mut client, &mut snapshot, &mut pump);
        let tick = snapshot.tick();
        if last_tick != Some(tick) {
            last_tick = Some(tick);
            if let Some(outcome) =
                traveller.follow(&mut client, &snapshot, route.clone(), &mut options)
            {
                break outcome;
            }
        }
        assert!(
            Instant::now() < deadline,
            "no-coins hop timed out at {:?} chat={:?} options={:?}",
            snapshot.tile(),
            snapshot.chat(),
            snapshot
                .chat_options()
                .iter()
                .map(|o| o.text.as_str())
                .collect::<Vec<_>>()
        );
        thread::sleep(Duration::from_millis(20));
    };
    drop(options);
    match &outcome {
        // The traveller's fare recheck names the consumed item and the fare.
        TravelOutcome::Blocked { detail, .. }
            if detail.contains(&format!("needs 10 of item {COINS} to pay the fare")) => {}
        other => panic!(
            "expected Blocked(coins) without paying, got {other:?}; tile={:?} options={:?}",
            snapshot.tile(),
            snapshot
                .chat_options()
                .iter()
                .map(|o| o.text.as_str())
                .collect::<Vec<_>>()
        ),
    }
    assert_eq!(
        snapshot.tile(),
        Some((stand.x, stand.z, stand.level)),
        "no-coins refusal must not cross the gate"
    );
    let receipt = json!({
        "account": name,
        "stand": stand,
        "dest": dest,
        "outcome": format!("{outcome:?}"),
        "coins": snapshot.inv_count(COINS),
        "tile": snapshot.tile(),
    });
    save_receipt("no-coins", &receipt);
    println!("PASS: toll_gate_live no-coins refusal {receipt}");
    if client.ingame {
        client.logout();
    }
}
