//! Fresh-account post-relog tutorial reseed wielding a bow to the bow tab.
//!
//! Regression for this fixture class: closing the character-design kit queues
//! `tutorial=1`, and at `tutorial <= 400` the content suppresses weapon
//! combat-tab updates, so a bow worn without a fresh post-relog `tutorial
//! 1000` confirm never sends `IF_SETTAB 1764` (`combat_bow`). The cell uses
//! the shared [`scenario::tutorial::PostRelogTutorial`] helper (baseline
//! captured after the relog, fresh same-session `getvar` reply required) and
//! then proves the worn shortbow binds combat tab root 1764.
//!
//! Run with `LIVE=1`, `BOT_CPU=1`, `BOT_NAV_BUILD=skip`,
//! `BOT_LIVE_NAME_PREFIX` (1-4 chars), `WORLD_NAV_PACK`, `WORLD_ENGINE_DIR`,
//! `BOT_CACHE_DIR` (writable clone), `LIVE_EVIDENCE_DIR`, and
//! `WORLD_GAME_PORT`/`WORLD_HTTP_PORT` pointing at the shared engine.

use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use api::interact;
use api::snapshot::GameSnapshot;
use host::Pump;
use host_play::{ProfileOptions, SharedClientTemplate};

const SHORTBOW: i32 = 841;
const COMBAT_BOW_ROOT: i32 = 1764;

fn env_port(name: &str, fallback: u16) -> u16 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(fallback)
}

fn selected_template() -> std::sync::Arc<SharedClientTemplate> {
    let nav_pack = PathBuf::from(std::env::var_os("WORLD_NAV_PACK").expect("WORLD_NAV_PACK"));
    let engine_dir = PathBuf::from(std::env::var_os("WORLD_ENGINE_DIR").expect("WORLD_ENGINE_DIR"));
    let options = ProfileOptions {
        profile: Some("local-289".into()),
        revision: Some("289".into()),
        host: Some("127.0.0.1".into()),
        asset_host: Some("127.0.0.1".into()),
        port: Some(env_port("WORLD_GAME_PORT", 44594)),
        http_port: Some(env_port("WORLD_HTTP_PORT", 1080)),
        nav_pack: Some(nav_pack),
        nav_flags: std::env::var_os("WORLD_NAV_FLAGS").map(PathBuf::from),
        engine_dir: Some(engine_dir),
        cache_dir: std::env::var_os("BOT_CACHE_DIR").map(PathBuf::from),
        ..ProfileOptions::default()
    };
    let profile = options
        .resolve(None)
        .expect("resolve explicit local R289 profile")
        .bind()
        .expect("bind explicit local R289 profile");
    SharedClientTemplate::load(profile).expect("load R289 template")
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
    panic!("[TUTORIAL-RESEED-BOW-FAIL] {label}: timed out");
}

fn inventory_tab_available(snapshot: &GameSnapshot) -> bool {
    snapshot
        .side_tabs()
        .iter()
        .any(|tab| tab.index == 3 && tab.available)
}

#[test]
#[ignore = "requires LIVE=1, BOT_CPU=1, BOT_NAV_BUILD=skip, BOT_LIVE_NAME_PREFIX, WORLD_NAV_PACK/WORLD_ENGINE_DIR/BOT_CACHE_DIR/LIVE_EVIDENCE_DIR and the shared 289 engine"]
fn live_tutorial_reseed_wields_bow() {
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"));
    assert_eq!(std::env::var("BOT_CPU").as_deref(), Ok("1"));
    assert_eq!(std::env::var("BOT_NAV_BUILD").as_deref(), Ok("skip"));
    let prefix = std::env::var("BOT_LIVE_NAME_PREFIX").expect("BOT_LIVE_NAME_PREFIX");
    assert!((1..=4).contains(&prefix.len()), "prefix must be 1-4 chars");
    let evidence_root =
        PathBuf::from(std::env::var_os("LIVE_EVIDENCE_DIR").expect("LIVE_EVIDENCE_DIR"));

    let template = selected_template();
    let serial = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_millis();
    let mut entropy = serial ^ ((std::process::id() as u128) << 64);
    let alphabet = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let mut token = [b'0'; 6];
    for digit in &mut token {
        *digit = alphabet[(entropy % alphabet.len() as u128) as usize];
        entropy /= alphabet.len() as u128;
    }
    let token = String::from_utf8(token.to_vec()).expect("ascii account token");
    let username = format!("{prefix}{token}_0");
    assert!(username.len() <= 12, "engine account name exceeds 12 chars");
    let uid = (serial % 1_000_000_000) as i32;

    let mut client = template
        .prepare_client(uid, true)
        .expect("prepare disposable R289 client");
    client.maininit();
    assert!(!client.error_loading, "R289 client asset init failed");
    let mut snapshot = GameSnapshot::new();
    let mut pump = Pump::new();
    assert!(
        interact::login(&mut client, &username, &username, false),
        "fresh-account login failed"
    );
    wait_for(
        &mut client,
        &mut snapshot,
        &mut pump,
        Duration::from_secs(120),
        "initial scene",
        |s| s.ingame() && s.attached() && s.scene_state() == 2 && s.tile().is_some(),
    );

    // Fresh accounts need the off-island tutorial skip and a cold
    // logout/login before the inventory side tab binds.
    let island_tile = snapshot.tile();
    interact::mainland_hop(&mut client);
    assert!(interact::cheat(&mut client, "getvar tutorial").is_sent());
    wait_for(
        &mut client,
        &mut snapshot,
        &mut pump,
        Duration::from_secs(30),
        "tutorial skip acknowledgement",
        |s| s.tile() != island_tile && scenario::tutorial::tutorial_confirmed(s, 0),
    );
    let ifaces = std::sync::Arc::clone(&client.ifaces);
    assert!(interact::logout(&mut client, &ifaces), "logout after skip");
    wait_for(
        &mut client,
        &mut snapshot,
        &mut pump,
        Duration::from_secs(30),
        "tutorial-skip logout",
        |s| !s.ingame(),
    );
    assert!(
        interact::login(&mut client, &username, &username, false),
        "cold login after tutorial skip"
    );
    wait_for(
        &mut client,
        &mut snapshot,
        &mut pump,
        Duration::from_secs(120),
        "scene after tutorial-skip login",
        |s| s.ingame() && s.attached() && s.scene_state() == 2 && s.tile().is_some(),
    );
    wait_for(
        &mut client,
        &mut snapshot,
        &mut pump,
        Duration::from_secs(120),
        "inventory tab after tutorial-skip relog",
        |s| s.ingame() && s.attached() && s.scene_state() == 2 && inventory_tab_available(s),
    );

    // Shared post-relog reseed with a fresh same-session confirm. The
    // baseline is captured after the relog session is ready and before the
    // reseed send, so a pre-relog line cannot satisfy the wait.
    let baseline = scenario::tutorial::chat_baseline(&snapshot);
    let reseed = scenario::tutorial::PostRelogTutorial::new(baseline);
    reseed.send_reseed(&mut client);
    let deadline = Instant::now() + scenario::tutorial::TUTORIAL_POST_RELOG_DEADLINE;
    loop {
        pump_once(&mut client, &mut snapshot, &mut pump);
        match reseed.check(&snapshot) {
            Ok(true) => {
                println!("{}", scenario::tutorial::confirmation_log());
                break;
            }
            Ok(false) => {
                if Instant::now() >= deadline {
                    panic!(
                        "[TUTORIAL-RESEED-BOW-FAIL] tutorial reseed confirm: {}",
                        reseed.failure_message()
                    );
                }
            }
            Err(error) => {
                panic!("[TUTORIAL-RESEED-BOW-FAIL] tutorial reseed confirm: {error}");
            }
        }
        thread::sleep(Duration::from_millis(20));
    }

    // Wield a requirement-free shortbow and prove the bow combat tab binds.
    assert!(interact::cheat(&mut client, "give shortbow 1").is_sent());
    wait_for(
        &mut client,
        &mut snapshot,
        &mut pump,
        Duration::from_secs(30),
        "shortbow grant",
        |s| s.inv_count(SHORTBOW) == 1,
    );
    let result = interact::Interactions::new(&snapshot, &mut client).wear(SHORTBOW);
    assert!(
        matches!(&result, interact::SendResult::Sent { .. }),
        "could not wield the fixture shortbow: {result:?}"
    );
    wait_for(
        &mut client,
        &mut snapshot,
        &mut pump,
        Duration::from_secs(30),
        "worn shortbow",
        |s| {
            s.equipment()
                .iter()
                .any(|item| item.def.id == SHORTBOW && item.count > 0)
        },
    );
    wait_for(
        &mut client,
        &mut snapshot,
        &mut pump,
        Duration::from_secs(30),
        "bow combat tab 1764",
        |s| {
            s.side_tabs()
                .iter()
                .any(|tab| tab.index == 0 && tab.root_component_id == COMBAT_BOW_ROOT)
        },
    );
    let combat_root = snapshot
        .side_tabs()
        .iter()
        .find(|tab| tab.index == 0)
        .map(|tab| tab.root_component_id)
        .unwrap_or(-1);
    println!(
        "worn bow observed: side_icon[0]={} snapshot_root={} tab1764={}",
        client.side_icon[0],
        combat_root,
        combat_root == COMBAT_BOW_ROOT
    );
    assert_eq!(
        client.side_icon[0], COMBAT_BOW_ROOT,
        "bow wear must bind side_icon[0] to combat_bow 1764"
    );
    assert_eq!(
        combat_root, COMBAT_BOW_ROOT,
        "bow wear must bind the combat tab root to combat_bow 1764"
    );
    println!("bow tab confirmed: root 1764 (combat_bow) with bow worn");

    let receipt = serde_json::json!({
        "proof": "fresh-account post-relog tutorial reseed wields shortbow to combat_bow 1764",
        "account": &username,
        "helper": scenario::tutorial::confirmation_log(),
        "side_icon_0": client.side_icon[0],
        "snapshot_root": combat_root,
    });
    std::fs::create_dir_all(&evidence_root).expect("evidence dir");
    std::fs::write(
        evidence_root.join("tutorial-reseed-bow-receipt-R2.json"),
        serde_json::to_string_pretty(&receipt).expect("receipt json"),
    )
    .expect("write bow receipt");
    println!("PASS tutorial-reseed-bow account={username}");
}
