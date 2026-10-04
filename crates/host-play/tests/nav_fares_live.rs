//! Seeded revision-289 manual WalkTo: chained tolls and native Shilo cart fares.
//! Uses only per-account setup cheats; never changes shared engine content.
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use api::interact;
use api::snapshot::GameSnapshot;
use client::client::Client;
use host::Pump;
use host_play::walk_map::{ActionError, ActionKind, FocusToken, MapContext, MapModel};
use host_play::{ProfileOptions, SharedClientTemplate, SlotArm, WalkArms};
use nav::map::identity::Digest;
use nav::router::{FindOptions, Leg};
use nav::tile::Tile;
use nav::WorldState;
use serde_json::{json, Value};

fn live_profile_options() -> ProfileOptions {
    ProfileOptions {
        profile: Some("local-289".into()),
        revision: Some("289".into()),
        host: Some("127.0.0.1".into()),
        asset_host: Some("127.0.0.1".into()),
        port: Some(44594),
        http_port: Some(1080),
        cache_dir: Some(PathBuf::from(
            std::env::var_os("BOT_CACHE_DIR").expect("disposable cache"),
        )),
        unpack_dir: Some(PathBuf::from(
            std::env::var_os("NAV_FARES_SNAPSHOT_ROOT").expect("explicit snapshot root"),
        )),
        engine_dir: Some(PathBuf::from(
            std::env::var_os("WORLD_ENGINE_DIR").expect("explicit engine"),
        )),
        nav_pack: Some(PathBuf::from(
            std::env::var_os("WORLD_NAV_PACK").expect("explicit pack"),
        )),
        ..ProfileOptions::default()
    }
}

const ORIGIN: Tile = Tile {
    x: 2663,
    z: 3302,
    level: 0,
};
const DESTINATION: Tile = Tile {
    x: 3028,
    z: 3217,
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
            "seed readiness: tile={:?}, inv={:?}, stats={:?}",
            snapshot.tile(),
            snapshot.inv(),
            snapshot.stats()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn coins(snapshot: &GameSnapshot, coin_id: i32) -> i32 {
    snapshot
        .inv()
        .iter()
        .filter(|&&(id, _)| id == coin_id)
        .map(|&(_, count)| count)
        .sum()
}

fn save_frame(client: &mut Client, snapshot: &GameSnapshot, dir: &Path, name: &str, detail: Value) {
    client.draw = true;
    let mut renderer = client::render::Renderer::new_prefer(client.config.lowmem, false);
    let client::render::backend::FrameOutput::PixMap(pixels) = renderer.mainredraw(client) else {
        panic!("CPU capture required");
    };
    let mut ppm = format!("P6\n{} {}\n255\n", pixels.width, pixels.height).into_bytes();
    for pixel in pixels.pixels {
        ppm.extend_from_slice(&[
            ((pixel >> 16) & 0xff) as u8,
            ((pixel >> 8) & 0xff) as u8,
            (pixel & 0xff) as u8,
        ]);
    }
    std::fs::write(dir.join(format!("{name}.ppm")), ppm).unwrap();
    std::fs::write(dir.join(format!("{name}.json")), serde_json::to_vec_pretty(&json!({
        "ingame":snapshot.ingame(), "attached":snapshot.attached(), "scene_state":snapshot.scene_state(),
        "tick":snapshot.tick(), "tile":snapshot.tile(), "inventory":snapshot.inv(),
        "stats":snapshot.stats(), "detail":detail,
    })).unwrap()).unwrap();
    client.draw = false;
}

#[test]
#[ignore = "LIVE=1, BOT_CPU=1, disposable HOME, WORLD_ENGINE_DIR, WORLD_NAV_PACK, BOT_CACHE_DIR, NAV_FARES_SNAPSHOT_ROOT and NAV_FARES_EVIDENCE required"]
fn live_manual_walk_two_fares_sixty_to_zero_and_thirty_refused() {
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"));
    assert_eq!(std::env::var("BOT_CPU").as_deref(), Ok("1"));
    let evidence =
        PathBuf::from(std::env::var_os("NAV_FARES_EVIDENCE").expect("explicit evidence"));
    std::fs::create_dir_all(&evidence).unwrap();
    let profile = live_profile_options()
        .resolve(None)
        .unwrap()
        .bind()
        .unwrap();
    let template = SharedClientTemplate::load(Arc::clone(&profile)).unwrap();
    let world = template.world().expect("explicit pack world");
    let identity = profile.nav_identity().unwrap();
    let data = api::game_data::for_revision(client::io::ClientRevision::R289).unwrap();
    // Map actions normally receive Play's world after its selected-facts bind.
    world.bind_named_bank_facts(&data).unwrap();
    let coin_id = data.item_by_alias("coins").expect("content coin alias").id;
    let prefix = std::env::var("BOT_LIVE_NAME_PREFIX").expect("explicit name prefix");
    assert_eq!(prefix, "nf");
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis()
        % 100_000_000;
    let names: [String; 2] = std::array::from_fn(|index| format!("{prefix}{stamp}{index}"));
    let mut cells = Vec::new();
    // One seeded cell exercises refusal first, then successful execution.
    for (slot, initial_coins) in [30, 60].into_iter().enumerate() {
        let account = &names[slot];
        let mut client = template.prepare_client(slot as i32 + 1, true).unwrap();
        client.draw = false;
        client.maininit();
        assert!(!client.error_loading);
        assert!(interact::login(&mut client, account, account, false));
        let mut snapshot = GameSnapshot::new();
        let mut pump = Pump::new();
        wait_for(&mut client, &mut snapshot, &mut pump, |s| {
            s.ingame() && s.attached() && s.scene_state() == 2
        });
        interact::mainland_hop(&mut client);
        assert!(interact::cheat(&mut client, "getvar tutorial").is_sent());
        wait_for(&mut client, &mut snapshot, &mut pump, |s| {
            s.chat_lines()
                .iter()
                .any(|line| line.text.contains("get tutorial: 1000"))
        });
        let ifaces = Arc::clone(&client.ifaces);
        assert!(interact::logout(&mut client, &ifaces));
        wait_for(&mut client, &mut snapshot, &mut pump, |s| !s.ingame());
        assert!(interact::login(&mut client, account, account, false));
        wait_for(&mut client, &mut snapshot, &mut pump, |s| {
            s.ingame() && s.scene_state() == 2
        });
        assert_eq!(
            coins(&snapshot, coin_id),
            0,
            "fresh account unexpectedly has coins"
        );
        for (skill, level) in [
            ("attack", 40),
            ("defence", 30),
            ("strength", 40),
            ("hitpoints", 26),
            ("ranged", 1),
            ("prayer", 1),
            ("magic", 1),
        ] {
            assert!(interact::cheat(&mut client, &format!("setstat {skill} {level}")).is_sent());
        }
        assert!(interact::cheat(&mut client, &format!("give coins {initial_coins}")).is_sent());
        interact::seed_at(&mut client, ORIGIN.level, ORIGIN.x, ORIGIN.z);
        wait_for(&mut client, &mut snapshot, &mut pump, |s| {
            s.ingame()
                && s.scene_state() == 2
                && s.tile() == Some((ORIGIN.x, ORIGIN.z, ORIGIN.level))
                && coins(s, coin_id) == initial_coins
                && WorldState::from_snapshot(s).combat_level == Some(40)
        });
        let state = WorldState::from_snapshot(&snapshot).with_map_members(profile.map_members());
        let context = MapContext {
            focus: Some(FocusToken::capture(&SlotArm::new(1, false))),
            nav: Digest::from_hex(&identity.nav_sha256).unwrap(),
            overlay: None,
            generation: 1,
        };
        let mut model = MapModel::default();
        model.bind(context);
        model.select_tile(&world, DESTINATION);
        let command = model
            .confirm(
                ActionKind::Walk,
                &context,
                Some(ORIGIN),
                FindOptions::default(),
            )
            .unwrap();
        let arms = WalkArms::default();
        let result = command.walk_on(&world, &context, account, &state, &[], &arms);
        if initial_coins == 30 {
            let error = result.unwrap_err();
            let message = error.to_string();
            save_frame(
                &mut client,
                &snapshot,
                &evidence,
                "00-thirty-refused",
                json!({"account":account,"error":message,"combat":state.combat_level}),
            );
            assert!(
                matches!(error, ActionError::InsufficientItems { .. }),
                "{message}"
            );
            assert!(
                message.to_lowercase().contains("30 more coins"),
                "{message}"
            );
            assert!(
                arms.lock().unwrap().get(account).is_none(),
                "refusal armed a walk"
            );
            assert_eq!(snapshot.tile(), Some((ORIGIN.x, ORIGIN.z, 0)));
            assert_eq!(coins(&snapshot, coin_id), 30);
            cells.push(json!({"account":account,"initial_coins":30,"result":"refused-before-boarding","message":message}));
            client.logout();
            continue;
        }
        let route = result.expect("sixty coins pays both ships");
        let ships: Vec<_> = route
            .legs
            .iter()
            .filter_map(|leg| match leg {
                Leg::Transport { edge } if edge.kind == nav::transport::TransportKind::Boat => {
                    Some(edge.loc_id)
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            ships,
            [381, 380],
            "must use Barnaby then Customs: {route:?}"
        );
        save_frame(
            &mut client,
            &snapshot,
            &evidence,
            "01-sixty-origin",
            json!({"account":account,"combat":state.combat_level,"route":format!("{route:?}")}),
        );
        let arm = Arc::clone(&arms.lock().unwrap()[account]);
        let deadline = Instant::now() + Duration::from_secs(420);
        let mut last_tick = None;
        let mut observations = Vec::new();
        let mut last = None;
        let mut brimhaven = false;
        let mut karamja = false;
        loop {
            pump_once(&mut client, &mut snapshot, &mut pump);
            let observed = (snapshot.tile(), coins(&snapshot, coin_id));
            if last != Some(observed) {
                observations.push(json!({"tick":snapshot.tick(),"tile":observed.0,"coins":observed.1,"scene":snapshot.scene_state()}));
                last = Some(observed);
            }
            if let Some((x, z, level)) = snapshot.tile() {
                if !brimhaven
                    && (2760..=2790).contains(&x)
                    && (3220..=3250).contains(&z)
                    && observed.1 == 30
                    && snapshot.scene_state() == 2
                {
                    brimhaven = true;
                    save_frame(
                        &mut client,
                        &snapshot,
                        &evidence,
                        "02-brimhaven-thirty",
                        json!({"account":account}),
                    );
                }
                karamja |= (2940..=2965).contains(&x)
                    && (3130..=3160).contains(&z)
                    && level == 0
                    && observed.1 == 30;
            }
            if last_tick != Some(snapshot.tick()) {
                last_tick = Some(snapshot.tick());
                if let Some(here) = snapshot.tile() {
                    let mut arm = arm.lock().unwrap();
                    host_play::step_walk_arm_follow(
                        &mut client,
                        &snapshot,
                        &mut arm,
                        Some(&world),
                        here,
                        profile.map_members(),
                        Some(account),
                    );
                    if arm.route.is_none() {
                        break;
                    }
                }
            }
            if Instant::now() >= deadline {
                save_frame(
                    &mut client,
                    &snapshot,
                    &evidence,
                    "FAIL-sixty-timeout",
                    json!({"account":account,"observations":observations}),
                );
                panic!(
                    "fare walk timed out: {:?}, {} coins",
                    snapshot.tile(),
                    coins(&snapshot, coin_id)
                );
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        save_frame(
            &mut client,
            &snapshot,
            &evidence,
            "03-port-sarim-zero",
            json!({"account":account,"observations":observations}),
        );
        let receipt = json!({"account":account,"initial_coins":60,"final_coins":coins(&snapshot,coin_id),
            "final_tile":snapshot.tile(),"ingame":snapshot.ingame(),"scene_state":snapshot.scene_state(),
            "brimhaven":brimhaven,"karamja":karamja,"ships":ships,"observations":observations});
        cells.push(receipt.clone());
        std::fs::write(evidence.join("receipt.json"), serde_json::to_vec_pretty(&json!({"pack_format":nav::pack::FORMAT_ID,"pack_sha256":identity.nav_sha256,"cells":cells})).unwrap()).unwrap();
        assert!(brimhaven && karamja, "{receipt}");
        assert_eq!(
            snapshot.tile(),
            Some((DESTINATION.x, DESTINATION.z, DESTINATION.level)),
            "{receipt}"
        );
        assert!(
            snapshot.ingame() && snapshot.scene_state() == 2,
            "{receipt}"
        );
        assert_eq!(coins(&snapshot, coin_id), 0, "{receipt}");
        client.logout();
    }
}

#[test]
#[ignore = "LIVE=1, BOT_CPU=1, disposable HOME, WORLD_ENGINE_DIR, WORLD_NAV_PACK, BOT_CACHE_DIR, NAV_FARES_SNAPSHOT_ROOT and NAV_FARES_EVIDENCE required"]
fn live_shilo_cart_ten_coins_to_zero() {
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"));
    assert_eq!(std::env::var("BOT_CPU").as_deref(), Ok("1"));
    let evidence_root =
        PathBuf::from(std::env::var_os("NAV_FARES_EVIDENCE").expect("explicit evidence"));
    let profile = live_profile_options()
        .resolve(None)
        .unwrap()
        .bind()
        .unwrap();
    let template = SharedClientTemplate::load(Arc::clone(&profile)).unwrap();
    let world = template.world().expect("explicit pack world");
    let identity = profile.nav_identity().unwrap();
    let data = api::game_data::for_revision(client::io::ClientRevision::R289).unwrap();
    world.bind_named_bank_facts(&data).unwrap();
    let coin_id = data.item_by_alias("coins").expect("content coin alias").id;
    let prefix = std::env::var("BOT_LIVE_NAME_PREFIX").expect("explicit name prefix");
    assert_eq!(prefix, "n1");
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis()
        % 100_000_000;
    let account = format!("{prefix}{stamp}");
    let evidence = evidence_root.join(format!("shilo-cart-{account}-{stamp}"));
    std::fs::create_dir_all(&evidence).unwrap();
    let mut client = template.prepare_client(1, true).unwrap();
    client.draw = false;
    client.maininit();
    assert!(!client.error_loading);
    assert!(interact::login(&mut client, &account, &account, false));
    let mut snapshot = GameSnapshot::new();
    let mut pump = Pump::new();
    wait_for(&mut client, &mut snapshot, &mut pump, |s| {
        s.ingame() && s.attached() && s.scene_state() == 2
    });
    interact::mainland_hop(&mut client);
    assert!(interact::cheat(&mut client, "getvar tutorial").is_sent());
    wait_for(&mut client, &mut snapshot, &mut pump, |s| {
        s.chat_lines()
            .iter()
            .any(|line| line.text.contains("get tutorial: 1000"))
    });
    let ifaces = Arc::clone(&client.ifaces);
    assert!(interact::logout(&mut client, &ifaces));
    wait_for(&mut client, &mut snapshot, &mut pump, |s| !s.ingame());
    assert!(interact::login(&mut client, &account, &account, false));
    wait_for(&mut client, &mut snapshot, &mut pump, |s| {
        s.ingame() && s.attached() && s.scene_state() == 2
    });
    assert_eq!(
        coins(&snapshot, coin_id),
        0,
        "fresh account has unexpected coins"
    );
    assert!(interact::cheat(&mut client, "give coins 10").is_sent());
    let origin = Tile {
        x: 2834,
        z: 2953,
        level: 0,
    };
    let destination = Tile {
        x: 2776,
        z: 3214,
        level: 0,
    };
    interact::seed_at(&mut client, origin.level, origin.x, origin.z);
    wait_for(&mut client, &mut snapshot, &mut pump, |s| {
        s.ingame()
            && s.scene_state() == 2
            && s.tile() == Some((origin.x, origin.z, origin.level))
            && coins(s, coin_id) == 10
    });
    let state = WorldState::from_snapshot(&snapshot).with_map_members(profile.map_members());
    let context = MapContext {
        focus: Some(FocusToken::capture(&SlotArm::new(1, false))),
        nav: Digest::from_hex(&identity.nav_sha256).unwrap(),
        overlay: None,
        generation: 1,
    };
    let mut model = MapModel::default();
    model.bind(context);
    model.select_tile(&world, destination);
    let command = model
        .confirm(
            ActionKind::Walk,
            &context,
            Some(origin),
            FindOptions::default(),
        )
        .unwrap();
    let arms = WalkArms::default();
    let route = command
        .walk_on(&world, &context, &account, &state, &[], &arms)
        .expect("native cart fare is ten coins");
    let carts: Vec<_> = route
        .legs
        .iter()
        .filter_map(|leg| match leg {
            Leg::Transport { edge } if edge.kind == nav::transport::TransportKind::Npc => {
                Some(edge.loc_id)
            }
            _ => None,
        })
        .collect();
    assert_eq!(carts, [511], "must take the real Vigroy cart: {route:?}");
    save_frame(
        &mut client,
        &snapshot,
        &evidence,
        "00-shilo-ten",
        json!({
            "account":account, "route":format!("{route:?}"), "pack_sha256":identity.nav_sha256,
        }),
    );
    let arm = Arc::clone(&arms.lock().unwrap()[&account]);
    let deadline = Instant::now() + Duration::from_secs(120);
    let mut last_tick = None;
    let mut observations = Vec::new();
    let mut last = None;
    loop {
        pump_once(&mut client, &mut snapshot, &mut pump);
        let observed = (
            snapshot.tile(),
            coins(&snapshot, coin_id),
            snapshot.scene_state(),
        );
        if last != Some(observed) {
            observations.push(json!({"tick":snapshot.tick(),"tile":observed.0,"coins":observed.1,"scene":snapshot.scene_state()}));
            last = Some(observed);
        }
        if last_tick != Some(snapshot.tick()) {
            last_tick = Some(snapshot.tick());
            if let Some(here) = snapshot.tile() {
                let mut arm = arm.lock().unwrap();
                host_play::step_walk_arm_follow(
                    &mut client,
                    &snapshot,
                    &mut arm,
                    Some(&world),
                    here,
                    profile.map_members(),
                    Some(&account),
                );
                if arm.route.is_none() && snapshot.scene_state() == 2 {
                    break;
                }
            }
        }
        if Instant::now() >= deadline {
            save_frame(
                &mut client,
                &snapshot,
                &evidence,
                "FAIL-cart-timeout",
                json!({"account":account,"observations":observations}),
            );
            panic!("cart timed out: {:?}", snapshot.tile());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let receipt = json!({
        "account":account,"initial_coins":10,"final_coins":coins(&snapshot,coin_id),
        "final_tile":snapshot.tile(),"ingame":snapshot.ingame(),"scene_state":snapshot.scene_state(),
        "carts":carts,"observations":observations,"pack_format":nav::pack::FORMAT_ID,"pack_sha256":identity.nav_sha256,
    });
    save_frame(
        &mut client,
        &snapshot,
        &evidence,
        "01-brimhaven-zero",
        receipt.clone(),
    );
    std::fs::write(
        evidence.join("receipt.json"),
        serde_json::to_vec_pretty(&receipt).unwrap(),
    )
    .unwrap();
    assert_eq!(
        snapshot.tile(),
        Some((destination.x, destination.z, destination.level)),
        "{receipt}"
    );
    assert_eq!(coins(&snapshot, coin_id), 0, "{receipt}");
    assert!(
        snapshot.ingame() && snapshot.scene_state() == 2,
        "{receipt}"
    );
    client.logout();
}
