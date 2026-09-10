//! Controlled step-5 world/guardian cells.
//!
//! This is intentionally an ignored, source-only harness.  It binds one
//! selected local revision, prepares a disposable account on the mainland,
//! and then drives the existing ScenarioRunner/Traveller or Guardian.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use api::interact::{self, Interactions, SendResult};
use api::snapshot::GameSnapshot;
use client::client::Client;
use host::{Guardian, Pump};
use host_play::{ProfileOptions, SharedClientTemplate};
use scenario::{RunnerStatus, ScenarioRunner};
use serde_json::json;
use vault::ProfileSettings;

const MAINLAND: (i32, i32, i32) = (3220, 3212, 0);
const DOOR: (i32, i32, i32) = (2816, 3438, 0);
const DOOR_CLOSED: i32 = 1530;
const DOOR_OPEN: i32 = 1531;
const LAMP_OBJ: i32 = 2528;
const LAMP_IF_ROOT: i32 = 2808;
const STRENGTH: i32 = 2;

fn required(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} is required"))
}

fn selected() -> (Arc<host_play::ServerProfile>, Arc<SharedClientTemplate>) {
    let revision = required("WORLD_REVISION");
    assert!(matches!(revision.as_str(), "274" | "289"));
    let nav_pack = PathBuf::from(required("WORLD_NAV_PACK"));
    let port = if revision == "289" { 44594 } else { 43594 };
    let http_port = if revision == "289" { 1080 } else { 80 };
    let options = ProfileOptions {
        profile: Some(format!("local-{revision}")),
        revision: Some(revision),
        host: Some("127.0.0.1".into()),
        asset_host: Some("127.0.0.1".into()),
        port: Some(port),
        http_port: Some(http_port),
        nav_pack: Some(nav_pack),
        nav_flags: std::env::var_os("WORLD_NAV_FLAGS").map(PathBuf::from),
        engine_dir: std::env::var_os("WORLD_ENGINE_DIR").map(PathBuf::from),
        ..ProfileOptions::default()
    };
    let profile = options
        .resolve(None)
        .expect("profile resolve")
        .bind()
        .expect("profile bind");
    assert_eq!(profile.client().target(), client::BotTarget::Local);
    assert_eq!(profile.client().game_host(), "127.0.0.1");
    let template =
        SharedClientTemplate::load(Arc::clone(&profile)).expect("selected template/world load");
    assert!(
        template.world().is_some(),
        "selected template.world() is required"
    );
    println!(
        "{}",
        json!({
            "phase": "identity",
            "revision": profile.revision().as_i32(),
            "profile": profile.label(),
            "cache_id": profile.cache_id(),
            "nav_pack": profile.nav_pack(),
        })
    );
    (profile, template)
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
    phase: &str,
    ready: impl Fn(&GameSnapshot) -> bool,
) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        pump_once(client, snapshot, pump);
        if ready(snapshot) {
            println!("{}", json!({"phase": phase, "tile": snapshot.tile()}));
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!(
        "{phase}: readiness predicate failed; tile={:?}",
        snapshot.tile()
    );
}

fn logout_and_relogin(
    client: &mut Client,
    snapshot: &mut GameSnapshot,
    pump: &mut Pump,
    name: &str,
) {
    let ifaces = Arc::clone(&client.ifaces);
    assert!(
        interact::logout(client, &ifaces),
        "selected cache has no logout control"
    );
    wait_for(
        client,
        snapshot,
        pump,
        Duration::from_secs(30),
        "logout",
        |s| !s.ingame(),
    );
    assert!(interact::login(client, name, name, false), "relogin failed");
    wait_for(
        client,
        snapshot,
        pump,
        Duration::from_secs(90),
        "relogin",
        |s| s.ingame() && s.attached() && s.scene_state() == 2 && s.tile().is_some(),
    );
}

fn prepare_mainland(client: &mut Client, snapshot: &mut GameSnapshot, pump: &mut Pump, name: &str) {
    client.draw = false;
    client.maininit();
    assert!(!client.error_loading, "asset initialization failed");
    assert!(interact::login(client, name, name, false), "login failed");
    wait_for(
        client,
        snapshot,
        pump,
        Duration::from_secs(90),
        "initial-scene2",
        |s| s.ingame() && s.attached() && s.scene_state() == 2,
    );
    interact::seed_at(client, MAINLAND.2, MAINLAND.0, MAINLAND.1);
    wait_for(
        client,
        snapshot,
        pump,
        Duration::from_secs(30),
        "mainland-seed",
        |s| s.ingame() && s.scene_state() == 2 && s.tile() == Some(MAINLAND),
    );
    logout_and_relogin(client, snapshot, pump, name);
    assert_eq!(
        snapshot.tile(),
        Some(MAINLAND),
        "relogin did not retain mainland seed"
    );
    println!(
        "{}",
        json!({"phase": "baseline-after-preparation", "tile": snapshot.tile()})
    );
}

fn door(snapshot: &GameSnapshot) -> Option<(api::snapshot::WorldTile, i32)> {
    snapshot
        .locs()
        .iter()
        .filter(|loc| {
            loc.tile.level == DOOR.2
                && (loc.tile.x - DOOR.0).abs().max((loc.tile.z - DOOR.1).abs()) <= 3
                && (loc.id == DOOR_CLOSED || loc.id == DOOR_OPEN)
        })
        .min_by_key(|loc| (loc.tile.x - DOOR.0).abs().max((loc.tile.z - DOOR.1).abs()))
        .map(|loc| (loc.tile, loc.id))
}

fn prepare_door(client: &mut Client, snapshot: &mut GameSnapshot, pump: &mut Pump) {
    // Reuse the production nav_door tele fixture, but do not use its closer
    // companion. This puts the single account at the outside stand.
    assert!(interact::cheat(client, "tele 0,43,53,61,44"));
    wait_for(
        client,
        snapshot,
        pump,
        Duration::from_secs(30),
        "door-outside",
        |s| s.tile() == Some((2813, 3436, 0)),
    );
    let before = door(snapshot);
    if before.is_some_and(|(_, id)| id == DOOR_OPEN) {
        let (tile, _) = before.expect("open door identity");
        assert!(interact::op_loc(client, tile.x, tile.z, DOOR_OPEN));
        wait_for(
            client,
            snapshot,
            pump,
            Duration::from_secs(30),
            "door-close",
            |s| door(s).is_some_and(|(_, id)| id == DOOR_CLOSED),
        );
    }
    assert_eq!(
        door(snapshot).map(|(_, id)| id),
        Some(DOOR_CLOSED),
        "door was not closed before baseline"
    );
    println!(
        "{}",
        json!({"phase": "door-baseline", "door": door(snapshot), "tile": snapshot.tile()})
    );
}

fn run_nav(
    case: &str,
    template: Arc<SharedClientTemplate>,
    profile: Arc<host_play::ServerProfile>,
) {
    let revision = profile.revision().as_i32();
    let serial = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let name = format!("w{}{}", revision, serial % 100_000_000);
    let mut client = template
        .prepare_client((serial % 1_000_000_000) as i32, true)
        .expect("prepare selected client");
    let mut snapshot = GameSnapshot::new();
    let mut pump = Pump::new();
    prepare_mainland(&mut client, &mut snapshot, &mut pump, &name);

    let mut scenario = scenario::get(case).expect("registered navigation scenario");
    scenario.settings.require_mainland_base = true;
    if case == "nav_door" {
        scenario.seed.profiles.truncate(1);
        scenario.companions.clear();
        scenario.settings.full_rate = false;
        prepare_door(&mut client, &mut snapshot, &mut pump);
        // prepare_mainland already proved the exact mainland landing and
        // logout/relogin. The runner's legacy x >= 3000 seed heuristic
        // cannot be applied after the fixture tele to Catherby (x = 2813).
        scenario.settings.require_mainland_base = false;
        // The first production step is only the outside tele; preparation
        // above already established that fixture, so proof starts at Follow.
        scenario.steps.remove(0);
    } else {
        scenario.settings.nav.engine_speed_ms = None;
        println!("{{\"phase\":\"nav_full-fixture\",\"engine_speed_ms\":null}}");
    }
    let mut runner = ScenarioRunner::with_world(scenario, template.world());
    runner.set_live_names(std::slice::from_ref(&name));
    runner.set_shot_sink(Box::new(|_, _| {}));

    let outer = if case == "nav_full" {
        Duration::from_secs(400)
    } else {
        Duration::from_secs(180)
    };
    let deadline = Instant::now() + outer;
    let mut before_door = door(&snapshot);
    let mut opened = false;
    let mut arrival = false;
    loop {
        pump_once(&mut client, &mut snapshot, &mut pump);
        if case == "nav_door" {
            let live_door = door(&snapshot);
            opened |= live_door.is_some_and(|(_, id)| id == DOOR_OPEN);
            before_door = before_door.or(live_door);
            arrival |= snapshot
                .tile()
                .is_some_and(|t| t.0 == 2817 && t.1 >= 3443 && t.2 == 0);
        }
        runner.tick(&mut client);
        match runner.status() {
            RunnerStatus::Passed => {
                if case == "nav_door" {
                    assert!(opened, "route did not cause observed door opening");
                    assert!(arrival, "inside arrival was not observed");
                    println!(
                        "PASS: nav_door: {}",
                        json!({"before_door": before_door, "after_door": door(&snapshot), "traveller_runner": runner.evidence()})
                    );
                } else {
                    println!(
                        "PASS: nav_full: {}",
                        json!({"engine_speed_ms": null, "evidence": runner.evidence()})
                    );
                }
                break;
            }
            RunnerStatus::Failed(msg) => panic!("{case}: {msg}; evidence={:?}", runner.evidence()),
            _ => assert!(
                Instant::now() < deadline,
                "{case}: outer timeout; evidence={:?}",
                runner.evidence()
            ),
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    if client.ingame {
        client.logout();
    }
}

fn has_lamp(snapshot: &GameSnapshot) -> bool {
    snapshot
        .inv()
        .iter()
        .any(|(id, count)| *id == LAMP_OBJ && *count > 0)
}

fn strength_xp(snapshot: &GameSnapshot) -> Option<i32> {
    snapshot
        .stats()
        .iter()
        .find(|s| s.index == STRENGTH)
        .map(|s| s.xp)
}

fn run_guardian(template: Arc<SharedClientTemplate>, profile: Arc<host_play::ServerProfile>) {
    let serial = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let name = format!("g{}{}", profile.revision().as_i32(), serial % 100_000_000);
    let mut client = template
        .prepare_client((serial % 1_000_000_000) as i32, true)
        .expect("prepare selected client");
    let mut snapshot = GameSnapshot::new();
    let mut pump = Pump::new();
    prepare_mainland(&mut client, &mut snapshot, &mut pump, &name);
    assert!(
        interact::cheat(&mut client, "give macro_genilamp"),
        "local give command refused"
    );
    wait_for(
        &mut client,
        &mut snapshot,
        &mut pump,
        Duration::from_secs(30),
        "lamp-inventory",
        has_lamp,
    );
    let inventory_before = snapshot.inv().to_vec();
    let xp_before = strength_xp(&snapshot).expect("strength XP baseline");
    let settings = ProfileSettings {
        random_events: true,
        lamp_auto: true,
        lamp_skill: "strength".into(),
        ..Default::default()
    };
    let mut guardian = Guardian::new();
    let end = Instant::now() + Duration::from_secs(30);
    let mut saw_hold = false;
    let mut saw_interface = false;
    let mut saw_consumed = false;
    let mut saw_xp_gain = false;
    let mut resumed_from = None;
    let mut walk_sent = false;
    let mut walk_target = None;
    while Instant::now() < end {
        pump_once(&mut client, &mut snapshot, &mut pump);
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let status = guardian.tick(&mut client, &snapshot, &settings, now_ms, None);
        saw_hold |= status.hold && status.ours;
        saw_interface |= snapshot.modals().main == LAMP_IF_ROOT;
        saw_consumed |= !has_lamp(&snapshot);
        saw_xp_gain |= strength_xp(&snapshot).is_some_and(|xp| xp > xp_before);
        if saw_consumed && !status.hold && saw_xp_gain && !walk_sent {
            resumed_from = snapshot.tile();
            let from = resumed_from.expect("post-resolution tile");
            let target = (from.0 + 1, from.1, from.2);
            walk_target = Some(target);
            match Interactions::new(&snapshot, &mut client).walk(api::snapshot::WorldTile {
                x: target.0,
                z: target.1,
                level: target.2,
            }) {
                SendResult::Sent { .. } => walk_sent = true,
                SendResult::Refused { reason, .. } => {
                    panic!("post-resolution walk refused: {reason:?}")
                }
            }
        }
        if walk_sent
            && snapshot.tile() == walk_target
            && saw_hold
            && saw_interface
            && saw_consumed
            && saw_xp_gain
        {
            println!(
                "PASS: guardian_lamp: {}",
                json!({"inventory_before": inventory_before, "xp_before": xp_before, "interface": saw_interface, "hold": saw_hold, "consumed": saw_consumed, "xp_gain": saw_xp_gain, "resumed_from": resumed_from, "walk_to": walk_target})
            );
            if client.ingame {
                client.logout();
            }
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("guardian_lamp incomplete: inventory_before={inventory_before:?} xp_before={xp_before} hold={saw_hold} interface={saw_interface} consumed={saw_consumed} xp_gain={saw_xp_gain} walk_sent={walk_sent}");
}

#[test]
#[ignore = "requires LIVE=1, WORLD_REVISION, WORLD_CASE, WORLD_NAV_PACK and local engine"]
fn world_boundary_live() {
    if std::env::var("LIVE").as_deref() != Ok("1") {
        return;
    }
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let case = required("WORLD_CASE");
        assert!(matches!(
            case.as_str(),
            "nav_full" | "nav_door" | "guardian_lamp"
        ));
        let (profile, template) = selected();
        match case.as_str() {
            "nav_full" | "nav_door" => run_nav(&case, template, profile),
            "guardian_lamp" => run_guardian(template, profile),
            _ => unreachable!(),
        }
    }));
    if let Err(error) = result {
        eprintln!("FAIL: world_boundary_live: {error:?}");
        std::process::exit(1);
    }
}
