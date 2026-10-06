//! Real map WalkTo and production WalkArm follow from the Ardougne shop pocket.
//! Requires LIVE=1 and NAV_ORIGIN_ENGINE; never reconfigures the shared engine.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use api::interact;
use api::snapshot::GameSnapshot;
use client::client::Client;
use host::Pump;
use host_play::walk_map::{ActionKind, FocusToken, MapContext, MapModel};
use host_play::{ProfileOptions, SharedClientTemplate, SlotArm, WalkArms};
use nav::map::identity::Digest;
use nav::router::{FindOptions, Leg};
use nav::tile::Tile;
use nav::transport::DoorDir;
use nav::WorldState;
use serde_json::json;

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
            "scene readiness: {:?}",
            snapshot.tile()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
#[ignore = "LIVE=1, NAV_ORIGIN_ENGINE and NAV_ORIGIN_PACK required; absence fails"]
fn live_walkto_from_ardougne_pocket_near_and_far() {
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"));
    let engine_dir =
        PathBuf::from(std::env::var_os("NAV_ORIGIN_ENGINE").expect("NAV_ORIGIN_ENGINE"));
    let options = ProfileOptions {
        profile: Some("local-289".into()),
        revision: Some("289".into()),
        host: Some("127.0.0.1".into()),
        asset_host: Some("127.0.0.1".into()),
        port: Some(45594),
        http_port: Some(2080),
        engine_dir: Some(engine_dir),
        nav_pack: Some(PathBuf::from(
            std::env::var_os("NAV_ORIGIN_PACK").expect("NAV_ORIGIN_PACK"),
        )),
        ..ProfileOptions::default()
    };
    let profile = options
        .resolve(None)
        .expect("resolve")
        .bind()
        .expect("bind");
    let template = SharedClientTemplate::load(Arc::clone(&profile)).expect("template");
    let world = template.world().expect("bundled world");
    let identity = profile.nav_identity().expect("bundled nav identity");
    println!(
        "{}",
        json!({"phase":"identity", "nav_sha256":identity.nav_sha256,
        "format":nav::pack::FORMAT_ID, "game_port":45594, "http_port":2080})
    );
    let serial = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let name = format!("navorigin{}", serial % 1_000);
    let mut client = template
        .prepare_client((serial % 1_000_000_000) as i32, true)
        .expect("client");
    client.draw = false;
    client.maininit();
    assert!(!client.error_loading);
    assert!(interact::login(&mut client, &name, &name, false));
    let mut snapshot = GameSnapshot::new();
    let mut pump = Pump::new();
    wait_for(&mut client, &mut snapshot, &mut pump, |s| {
        s.ingame() && s.attached() && s.scene_state() == 2
    });
    // Mainland preparation is the fixture's tutorial-skip cheat, not a route hop.
    interact::seed_at(&mut client, 0, 3220, 3212);
    wait_for(&mut client, &mut snapshot, &mut pump, |s| {
        s.scene_state() == 2 && s.tile() == Some((3220, 3212, 0))
    });

    let context = MapContext {
        focus: Some(FocusToken::capture(&SlotArm::new(1, false))),
        nav: Digest::from_hex(&identity.nav_sha256).unwrap(),
        overlay: None,
        generation: 1,
    };
    let arms = WalkArms::default();
    for destination in [
        Tile {
            x: 2661,
            z: 3301,
            level: 0,
        },
        Tile {
            x: 2724,
            z: 3484,
            level: 0,
        },
    ] {
        interact::seed_at(&mut client, 0, 2659, 3292);
        wait_for(&mut client, &mut snapshot, &mut pump, |s| {
            s.ingame() && s.scene_state() == 2 && s.tile() == Some((2659, 3292, 0))
        });
        println!(
            "{}",
            json!({"phase":"origin", "tile":snapshot.tile(), "scene":snapshot.scene_state(),
            "destination":[destination.x,destination.z,destination.level]})
        );
        let mut model = MapModel::default();
        model.bind(context);
        model.select_tile(&world, destination);
        let command = model
            .confirm(
                ActionKind::Walk,
                &context,
                Some(Tile {
                    x: 2659,
                    z: 3292,
                    level: 0,
                }),
                FindOptions::default(),
            )
            .expect("confirm WalkTo");
        let state = WorldState::from_snapshot(&snapshot);
        let admission = host_play::admission::Admission::manual(
            command.options(),
            host_play::admission::capture(&snapshot, state.map_members, Default::default(), false),
            0,
            host_play::WalkGlobals {
                allow_danger_zones: true,
                survivable_routing: false,
                ..Default::default()
            },
        );
        let route = command
            .walk_on(&world, &context, &name, &state, &[], admission, &arms)
            .expect("WalkTo route")
            .route;
        assert!(route
            .legs
            .iter()
            .any(|leg| matches!(leg, Leg::Transport { edge }
            if edge.loc_id == 1530 && edge.dir == Some(DoorDir::W) && edge.to == edge.at)));
        println!(
            "{}",
            json!({"phase":"route", "legs":format!("{:?}", route.legs), "ticks":route.ticks})
        );
        // WalkArms is the production std-mutex ABI; do not introduce a second lock type.
        let arm = Arc::clone(&arms.lock().expect("walk arms")[&name]);
        let deadline = Instant::now() + Duration::from_secs(180);
        let mut last_tick = None;
        let mut opened = false;
        loop {
            pump_once(&mut client, &mut snapshot, &mut pump);
            opened |= snapshot
                .locs()
                .iter()
                .any(|loc| loc.id == 1531 && loc.tile.x == 2657 && loc.tile.z == 3292);
            if last_tick != Some(snapshot.tick()) {
                last_tick = Some(snapshot.tick());
                if let Some(here) = snapshot.tile() {
                    let mut arm = arm.lock().expect("walk arm");
                    host_play::step_walk_arm_follow(
                        &mut client,
                        &snapshot,
                        &mut arm,
                        Some(&world),
                        here,
                        state.map_members,
                        Some(&name),
                    );
                    if arm.route.is_none() {
                        assert_eq!(
                            here,
                            (destination.x, destination.z, destination.level),
                            "WalkTo terminal before arrival"
                        );
                        println!(
                            "{}",
                            json!({"phase":"arrival", "tile":here, "scene":snapshot.scene_state(),
                            "opened_leaf_seen":opened, "tick":snapshot.tick()})
                        );
                        break;
                    }
                }
            }
            assert!(
                Instant::now() < deadline,
                "WalkTo timeout: {:?}",
                snapshot.tile()
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    client.logout();
}
