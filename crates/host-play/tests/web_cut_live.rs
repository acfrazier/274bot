//! Live proof that knife-on-web and worn slash-weapon web crossings follow
//! the pinned R289 content on a fresh, tutorial-unlocked account.
//!
//! The Mage Arena segment is planned with a local R289 v16 pack and ends just
//! beyond the second web at 3093/3957.
//! Run with `LIVE=1`, `BOT_CPU=1`, `BOT_NAV_BUILD=skip`,
//! `BOT_LIVE_NAME_PREFIX` (1-4 chars), `WORLD_NAV_PACK`, `WORLD_ENGINE_DIR`,
//! and `LIVE_EVIDENCE_DIR` set to the WEB-CUT-2 evidence root.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use api::interact;
use api::snapshot::{GameSnapshot, WorldTile};
use host::Pump;
use host_play::{ProfileOptions, ServerProfile, SharedClientTemplate};
use nav::router::{find_with, FindOptions, Leg, Route};
use nav::transport::TransportKind;
use nav::traveller::{TravelEvent, TravelOptions, TravelOutcome, Traveller};
use nav::world::NavWorld;
use nav::WorldState;
use serde_json::{json, Value};

const MAGE_ARENA_ORIGIN: WorldTile = WorldTile {
    x: 3097,
    z: 3957,
    level: 0,
};
const ROUTE_DESTINATION: WorldTile = WorldTile {
    x: 3092,
    z: 3957,
    level: 0,
};
const WEB_LOC: i32 = 733;
const SLASHED_WEB_LOC: i32 = 734;
const KNIFE: i32 = 946;
const BRONZE_SCIMITAR: i32 = 1321;
const WEB_TARGETS: [(i32, i32); 2] = [(3095, 3957), (3093, 3957)];
const WEB_FAILURE: &str = "You fail to cut through it.";

fn selected() -> (Arc<ServerProfile>, Arc<SharedClientTemplate>) {
    let options = ProfileOptions {
        profile: Some("local-289".into()),
        revision: Some("289".into()),
        host: Some("127.0.0.1".into()),
        port: Some(45594),
        asset_host: Some("127.0.0.1".into()),
        http_port: Some(2080),
        nav_pack: Some(PathBuf::from(
            std::env::var("WORLD_NAV_PACK")
                .expect("WORLD_NAV_PACK must name the worktree v16 pack"),
        )),
        nav_flags: std::env::var_os("WORLD_NAV_FLAGS").map(PathBuf::from),
        engine_dir: Some(PathBuf::from(
            std::env::var("WORLD_ENGINE_DIR").expect("WORLD_ENGINE_DIR must name local R289"),
        )),
        cache_dir: std::env::var_os("WORLD_CACHE_DIR").map(PathBuf::from),
        ..ProfileOptions::default()
    };
    let profile = options
        .resolve(None)
        .expect("resolve explicit local R289 profile")
        .bind()
        .expect("bind explicit local R289 profile");
    assert_eq!(profile.profile_class(), host_play::ProfileClass::Local);
    assert_eq!(profile.client().game_host(), "127.0.0.1");
    assert_eq!(profile.client().game_port(), 45594);
    assert_eq!(profile.client().asset_port(), 2080);
    let template = SharedClientTemplate::load(Arc::clone(&profile))
        .expect("load selected R289 profile and v16 pack");
    assert!(
        template.world().is_some(),
        "the selected v16 pack must load"
    );
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
    let chat: Vec<_> = snapshot
        .chat_lines()
        .iter()
        .take(6)
        .map(|line| line.text.clone())
        .collect();
    let exit = client.take_session_exit_reason();
    panic!(
        "[WEB-CUT-LIVE-FAIL] {label}: timed out at {:?}; client=(ingame={}, scene={}, tab3={}, login_screen={}, reconnect={:?}, logout_timer={}, response_seen={}, exit={exit:?}); snapshot=(ingame={}, attached={}, scene={}, tab3_available={}, inventory={}, chat={chat:?})",
        snapshot.tile(),
        client.ingame,
        client.scene_state,
        client.side_icon[3],
        client.loginscreen,
        client.last_login_reconnect,
        client.logout_timer,
        client.last_response.is_some(),
        snapshot.ingame(),
        snapshot.attached(),
        snapshot.scene_state(),
        inventory_tab_available(snapshot),
        snapshot.inventory_size(),
    );
}

fn inventory_tab_available(snapshot: &GameSnapshot) -> bool {
    snapshot
        .side_tabs()
        .iter()
        .any(|tab| tab.index == 3 && tab.available)
}

fn login_fresh(
    template: &SharedClientTemplate,
    username: &str,
    uid: i32,
) -> (client::client::Client, GameSnapshot, Pump) {
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
        interact::login(&mut client, username, username, false),
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

    // Fresh accounts need the off-island tutorial skip and a cold logout/login before the
    // inventory side tab is bound into snapshots.
    let tutorial_island_tile = snapshot.tile();
    interact::mainland_hop(&mut client);
    assert!(interact::cheat(&mut client, "getvar tutorial").is_sent());
    wait_for(
        &mut client,
        &mut snapshot,
        &mut pump,
        Duration::from_secs(30),
        "tutorial skip acknowledgement",
        |s| {
            s.tile() != tutorial_island_tile
                && s.chat_lines().iter().any(|line| {
                    line.text
                        .to_ascii_lowercase()
                        .contains("get tutorial: 1000")
                })
        },
    );
    let ifaces = Arc::clone(&client.ifaces);
    assert!(
        interact::logout(&mut client, &ifaces),
        "logout after tutorial skip"
    );
    wait_for(
        &mut client,
        &mut snapshot,
        &mut pump,
        Duration::from_secs(30),
        "tutorial-skip logout",
        |s| !s.ingame(),
    );
    assert!(
        interact::login(&mut client, username, username, false),
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
    (client, snapshot, pump)
}

fn prepare_loadout(
    client: &mut client::client::Client,
    snapshot: &mut GameSnapshot,
    pump: &mut Pump,
    wield_slash_weapon: bool,
) {
    assert!(inventory_tab_available(snapshot));
    assert!(interact::cheat(client, "~clearinv").is_sent());
    wait_for(
        client,
        snapshot,
        pump,
        Duration::from_secs(30),
        "empty inventory",
        |s| inventory_tab_available(s) && s.inventory().is_empty(),
    );
    assert!(interact::cheat(client, "give knife 1").is_sent());
    wait_for(
        client,
        snapshot,
        pump,
        Duration::from_secs(30),
        "knife grant",
        |s| s.inv_count(KNIFE) == 1,
    );
    if wield_slash_weapon {
        assert!(interact::cheat(client, "give bronze_scimitar 1").is_sent());
        wait_for(
            client,
            snapshot,
            pump,
            Duration::from_secs(30),
            "slash weapon grant",
            |s| s.inv_count(BRONZE_SCIMITAR) == 1,
        );
        let result = interact::Interactions::new(snapshot, client).wear(BRONZE_SCIMITAR);
        assert!(
            matches!(&result, interact::SendResult::Sent { .. }),
            "could not wield the fixture slash weapon: {result:?}"
        );
        wait_for(
            client,
            snapshot,
            pump,
            Duration::from_secs(30),
            "worn slash weapon",
            |s| {
                s.equipment()
                    .iter()
                    .any(|item| item.def.id == BRONZE_SCIMITAR && item.count > 0)
            },
        );
    }
    assert_eq!(
        snapshot.inventory().len(),
        1,
        "loadout has only the knife in inventory"
    );
    assert_eq!(snapshot.inventory()[0].def.id, KNIFE);
    assert_eq!(snapshot.inventory()[0].count, 1);
    assert_eq!(snapshot.inv_count(KNIFE), 1);
    if wield_slash_weapon {
        assert!(snapshot
            .equipment()
            .iter()
            .any(|item| item.def.id == BRONZE_SCIMITAR && item.count > 0));
    } else {
        assert!(snapshot
            .equipment()
            .iter()
            .all(|item| { item.def.id != 1277 && item.def.id != BRONZE_SCIMITAR }));
    }
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
        "teleport to Mage Arena web route",
        |s| s.ingame() && s.scene_state() == 2 && s.tile() == Some((tile.x, tile.z, tile.level)),
    );
}

#[derive(Clone, Copy)]
struct WebLocObservation {
    tick: u32,
    ids: [Option<i32>; 2],
}

fn web_loc_ids(snapshot: &GameSnapshot) -> [Option<i32>; 2] {
    WEB_TARGETS.map(|target| {
        snapshot
            .locs()
            .iter()
            .find(|loc| {
                loc.tile.level == MAGE_ARENA_ORIGIN.level && (loc.tile.x, loc.tile.z) == target
            })
            .map(|loc| loc.id)
    })
}

fn web_ids_json(ids: [Option<i32>; 2]) -> Value {
    json!(WEB_TARGETS
        .iter()
        .zip(ids)
        .map(|(target, loc_id)| json!({
            "target": [target.0, target.1],
            "loc_id": loc_id,
        }))
        .collect::<Vec<_>>())
}

fn observation_web_id(observation: &WebLocObservation, target: (i32, i32)) -> Option<i32> {
    let index = WEB_TARGETS
        .iter()
        .position(|candidate| *candidate == target)
        .expect("known Mage Arena web");
    observation.ids[index]
}

fn wait_for_closed_webs(
    client: &mut client::client::Client,
    snapshot: &mut GameSnapshot,
    pump: &mut Pump,
) -> Value {
    let start = Instant::now();
    let deadline = start + Duration::from_secs(180);
    let start_tick = snapshot.tick();
    let start_ids = web_loc_ids(snapshot);
    let mut last_ids = start_ids;
    let mut slashed_since = start_ids.map(|id| (id == Some(SLASHED_WEB_LOC)).then_some(start_tick));

    loop {
        let tick = snapshot.tick();
        let ids = web_loc_ids(snapshot);
        let waited_for_revert = slashed_since
            .iter()
            .all(|since| since.is_none_or(|since| tick.saturating_sub(since) >= 100));
        if ids.iter().all(|id| *id == Some(WEB_LOC)) && waited_for_revert {
            return json!({
                "start_tick": start_tick,
                "start_webs": web_ids_json(start_ids),
                "ready_tick": tick,
                "ready_webs": web_ids_json(ids),
                "slashed_since_ticks": slashed_since,
                "ticks_waited": tick.saturating_sub(start_tick),
            });
        }

        pump_once(client, snapshot, pump);
        let tick = snapshot.tick();
        let ids = web_loc_ids(snapshot);
        for index in 0..WEB_TARGETS.len() {
            if ids[index] == Some(SLASHED_WEB_LOC) && last_ids[index] != Some(SLASHED_WEB_LOC) {
                slashed_since[index] = Some(tick);
            }
        }
        last_ids = ids;
        assert!(
            Instant::now() < deadline,
            "Mage Arena webs did not return to loc 733 after any observed 100-tick slash window; tick={tick}, webs={}",
            web_ids_json(ids)
        );
        thread::sleep(Duration::from_millis(20));
    }
}

fn attempt_target(attempt: &Value) -> (i32, i32) {
    let target = attempt["target"].as_array().expect("attempt target tuple");
    (
        target[0].as_i64().expect("target x") as i32,
        target[1].as_i64().expect("target z") as i32,
    )
}

fn attempt_tick(attempt: &Value) -> u32 {
    attempt["tick"].as_u64().expect("attempt tick") as u32
}

fn validate_attempt_scene(
    attempts: &[Value],
    observations: &[WebLocObservation],
) -> (Vec<Value>, Vec<Value>) {
    let attempt_scene: Vec<_> = attempts
        .iter()
        .map(|attempt| {
            let target = attempt_target(attempt);
            let tick = attempt_tick(attempt);
            let observation = observations
                .iter()
                .find(|observation| observation.tick == tick)
                .expect("recorded scene immediately before each transport attempt");
            let loc_id = observation_web_id(observation, target);
            assert_eq!(
                loc_id,
                Some(WEB_LOC),
                "an attempt only counts when its pre-send scene shows the uncut web: {attempt:?}"
            );
            json!({
                "tick": tick,
                "target": [target.0, target.1],
                "web_loc_id_before_send": loc_id,
                "outgoing_packet": attempt["outgoing_packet"],
            })
        })
        .collect();

    let loc_changes = WEB_TARGETS
        .iter()
        .map(|&target| {
            let last_attempt_tick = attempts
                .iter()
                .filter(|attempt| attempt_target(attempt) == target)
                .map(attempt_tick)
                .max()
                .expect("an outgoing packet for each web");
            let observation = observations
                .iter()
                .find(|observation| {
                    observation.tick > last_attempt_tick
                        && observation_web_id(observation, target) == Some(SLASHED_WEB_LOC)
                })
                .expect("observe loc_change to slashed web after the outgoing packet");
            json!({
                "target": [target.0, target.1],
                "from_loc_id": WEB_LOC,
                "outgoing_attempt_tick": last_attempt_tick,
                "to_loc_id": SLASHED_WEB_LOC,
                "observed_tick": observation.tick,
            })
        })
        .collect();
    (attempt_scene, loc_changes)
}

fn route_webs(
    world: &NavWorld,
    profile: &ServerProfile,
    snapshot: &GameSnapshot,
    wield_slash_weapon: bool,
) -> (Route, Vec<(i32, i32, i32)>) {
    let state = WorldState::from_snapshot(snapshot).with_map_members(profile.map_members());
    let route = find_with(
        &world.collision,
        &world.graph,
        MAGE_ARENA_ORIGIN,
        ROUTE_DESTINATION,
        FindOptions {
            allow_wilderness: true,
            ..FindOptions::default()
        },
        &state,
    )
    .unwrap_or_else(|error| panic!("Mage Arena route failed with loadout {state:?}: {error:?}"));
    let webs: Vec<_> = route
        .legs
        .iter()
        .filter_map(|leg| match leg {
            Leg::Transport { edge }
                if edge.kind == TransportKind::Door
                    && edge.loc_id == WEB_LOC
                    && edge.open_loc_id == Some(SLASHED_WEB_LOC) =>
            {
                Some((edge.at.x, edge.at.z, edge.option))
            }
            _ => None,
        })
        .collect();
    let expected_option = if wield_slash_weapon { 1 } else { 0 };
    assert_eq!(
        webs,
        vec![
            (WEB_TARGETS[0].0, WEB_TARGETS[0].1, expected_option),
            (WEB_TARGETS[1].0, WEB_TARGETS[1].1, expected_option),
        ],
        "the planned Mage Arena route must use both webs with the content-selected op"
    );
    for edge in route.legs.iter().filter_map(|leg| match leg {
        Leg::Transport { edge }
            if edge.kind == TransportKind::Door
                && edge.loc_id == WEB_LOC
                && edge.open_loc_id == Some(SLASHED_WEB_LOC) =>
        {
            Some(edge)
        }
        _ => None,
    }) {
        if wield_slash_weapon {
            assert_eq!(edge.option, 1, "wielded slash weapon selects oploc1");
        } else {
            assert_eq!(edge.option, 0, "knife loadout selects oplocu");
            assert!(
                edge.item_req.iter().any(|&(item_id, _)| item_id == KNIFE),
                "knife route edge must carry the knife item requirement"
            );
        }
    }
    (route, webs)
}

fn follow_webs(
    client: &mut client::client::Client,
    snapshot: &mut GameSnapshot,
    pump: &mut Pump,
    world: &NavWorld,
    route: Route,
) -> (
    TravelOutcome,
    Vec<Value>,
    Vec<String>,
    Vec<WebLocObservation>,
) {
    let mut traveller = Traveller::new();
    let mut attempts = Vec::new();
    let mut web_observations = Vec::new();

    let mut options = TravelOptions {
        close_enough: 0,
        edges: Some(world.graph.edges.as_slice()),
        on_event: Some(Box::new(|event: TravelEvent| {
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
                if *expected_id == WEB_LOC {
                    let outgoing_packet = match *option {
                        0 => json!({
                            "action": "oplocu",
                            "item_id": KNIFE,
                            "target_loc_id": actual_id,
                            "target_tile": [target.x, target.z, target.level],
                        }),
                        1 => json!({
                            "action": "oploc1",
                            "target_loc_id": actual_id,
                            "target_tile": [target.x, target.z, target.level],
                        }),
                        option => json!({
                            "action": format!("oploc{option}"),
                            "target_loc_id": actual_id,
                            "target_tile": [target.x, target.z, target.level],
                        }),
                    };
                    attempts.push(json!({
                        "tick": tick,
                        "kind": format!("{kind:?}"),
                        "expected_id": expected_id,
                        "actual_id": actual_id,
                        "target": [target.x, target.z, target.level],
                        "option": option,
                        "refusal": refusal.as_ref().map(|reason| format!("{reason:?}")),
                        "outgoing_packet": outgoing_packet,
                    }));
                }
            }
        })),
        ..TravelOptions::default()
    };
    let mut observed_failures = Vec::new();
    let mut last_chat_sequence = snapshot
        .chat_lines()
        .iter()
        .map(|line| line.sequence)
        .max()
        .unwrap_or(0);
    let deadline = Instant::now() + Duration::from_secs(180);
    let mut last_tick = None;
    let outcome = loop {
        pump_once(client, snapshot, pump);
        let latest_chat_sequence = snapshot
            .chat_lines()
            .iter()
            .map(|line| line.sequence)
            .max()
            .unwrap_or(last_chat_sequence);
        for line in snapshot
            .chat_lines()
            .iter()
            .filter(|line| line.sequence > last_chat_sequence)
        {
            if line.text.contains(WEB_FAILURE) {
                observed_failures.push(line.text.clone());
            }
        }
        last_chat_sequence = latest_chat_sequence;
        if snapshot.ingame() && snapshot.scene_state() == 2 {
            let tick = snapshot.tick();
            if last_tick != Some(tick) {
                last_tick = Some(tick);
                web_observations.push(WebLocObservation {
                    tick,
                    ids: web_loc_ids(snapshot),
                });
                if let Some(outcome) =
                    traveller.follow(client, snapshot, route.clone(), &mut options)
                {
                    break outcome;
                }
            }
        }
        assert!(
            Instant::now() < deadline,
            "Mage Arena web follow timed out at {:?}; failures={observed_failures:?}",
            snapshot.tile()
        );
        thread::sleep(Duration::from_millis(20));
    };
    drop(options);
    (outcome, attempts, observed_failures, web_observations)
}

/// Render the arrival frame (needs the client) and hand the pixels plus the
/// JSON receipt to a background evidence writer for encode and write, then
/// wait bounded for the files before returning. Names and contents match the
/// old synchronous capture; the RGB layout is preserved byte-for-byte.
fn save_capture(
    root: &Path,
    scenario: &str,
    account: &str,
    receipt: &Value,
    client: &mut client::client::Client,
) -> PathBuf {
    use host_play::evidence_writer::{
        EvidenceRequest, EvidenceSidecar, EvidenceWriter, PngColor, DEFAULT_FLUSH_WAIT,
        DEFAULT_QUEUE_BOUND,
    };
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_millis();
    let run_dir = root.join(format!("web_cut_{scenario}_{account}_{millis}"));
    fs::create_dir_all(&run_dir).expect("create per-account evidence folder");
    let stem = format!("{millis}_01-cross-both-webs");
    let render_start = Instant::now();
    let mut renderer = client::render::Renderer::new_prefer(client.config.lowmem, false);
    let client::render::backend::FrameOutput::PixMap(pixels) = renderer.mainredraw(client) else {
        panic!("CPU arrival capture returned a GPU frame");
    };
    println!(
        "capture-render site=web_cut {stem} {}x{} render_ms={:.1}",
        pixels.width,
        pixels.height,
        render_start.elapsed().as_secs_f64() * 1000.0,
    );
    let writer = EvidenceWriter::new(DEFAULT_QUEUE_BOUND);
    let job = writer.submit(EvidenceRequest {
        png_path: run_dir.join(format!("{stem}.png")),
        width: std::convert::TryFrom::try_from(pixels.width).expect("positive pixmap width"),
        height: std::convert::TryFrom::try_from(pixels.height).expect("positive pixmap height"),
        pixels: pixels.pixels,
        color: PngColor::Rgb,
        sidecar: Some(EvidenceSidecar {
            path: run_dir.join(format!("{stem}.json")),
            receipt: receipt.clone(),
            patch_error: None,
        }),
    });
    match writer.wait(&job, DEFAULT_FLUSH_WAIT) {
        Some(outcome) if outcome.error.is_none() => {}
        Some(outcome) => panic!(
            "write web-cut evidence {}: {}",
            run_dir.display(),
            outcome.error.unwrap_or_else(|| "unknown".into()),
        ),
        None => panic!(
            "web-cut evidence writer did not finish before its deadline: {}",
            run_dir.display(),
        ),
    }
    println!("LIVE_EVIDENCE {}", run_dir.display());
    run_dir
}

fn logout(client: &mut client::client::Client, snapshot: &mut GameSnapshot, pump: &mut Pump) {
    let ifaces = Arc::clone(&client.ifaces);
    assert!(
        interact::logout(client, &ifaces),
        "logout disposable account"
    );
    wait_for(
        client,
        snapshot,
        pump,
        Duration::from_secs(30),
        "disposable account logout",
        |s| !s.ingame(),
    );
}

#[test]
#[ignore = "requires local R289, the selected nav pack, and LIVE=1"]
fn live_fresh_accounts_cross_both_mage_arena_webs_with_content_actions() {
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"));
    assert_eq!(std::env::var("BOT_CPU").as_deref(), Ok("1"));
    assert_eq!(std::env::var("BOT_NAV_BUILD").as_deref(), Ok("skip"));
    let prefix = std::env::var("BOT_LIVE_NAME_PREFIX").expect("BOT_LIVE_NAME_PREFIX");
    assert!((1..=4).contains(&prefix.len()), "prefix must be 1-4 chars");
    let evidence_root =
        PathBuf::from(std::env::var_os("LIVE_EVIDENCE_DIR").expect("LIVE_EVIDENCE_DIR"));

    let (profile, template) = selected();
    let world = template.world().expect("selected nav world");
    let identity = profile.nav_identity().expect("selected nav identity");
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

    for (slot, scenario, wield_slash_weapon) in
        [(0, "knife-only", false), (1, "slash-equipped", true)]
    {
        let username = format!("{prefix}{token}_{slot}");
        assert!(username.len() <= 12, "engine account name exceeds 12 chars");
        let uid = ((serial % 1_000_000_000) as i32).saturating_add(slot);
        let (mut client, mut snapshot, mut pump) = login_fresh(&template, &username, uid);
        prepare_loadout(&mut client, &mut snapshot, &mut pump, wield_slash_weapon);
        tele_to(&mut client, &mut snapshot, &mut pump, MAGE_ARENA_ORIGIN);
        let preflight = wait_for_closed_webs(&mut client, &mut snapshot, &mut pump);
        let (route, webs) = route_webs(&world, &profile, &snapshot, wield_slash_weapon);
        assert_eq!(
            snapshot.tile(),
            Some((
                MAGE_ARENA_ORIGIN.x,
                MAGE_ARENA_ORIGIN.z,
                MAGE_ARENA_ORIGIN.level
            ))
        );

        let (outcome, attempts, failures, observations) =
            follow_webs(&mut client, &mut snapshot, &mut pump, &world, route.clone());
        assert_eq!(
            outcome,
            TravelOutcome::Arrived {
                at: route.dest
            },
            "{scenario}: both live web cuts must reach the far side; attempts={attempts:?}; failures={failures:?}"
        );
        assert!(
            snapshot.ingame() && snapshot.attached() && snapshot.scene_state() == 2,
            "{scenario}: arrival must be a fresh in-game scene"
        );
        assert_eq!(
            snapshot.tile(),
            Some((route.dest.x, route.dest.z, route.dest.level))
        );
        let expected_option = if wield_slash_weapon { 1 } else { 0 };
        assert_eq!(
            attempts.len(),
            failures.len() + WEB_TARGETS.len(),
            "every observed 50% fail must authorize exactly one retry; no timed retry is allowed"
        );
        assert!(
            attempts.iter().all(|attempt| {
                attempt["option"] == expected_option
                    && attempt["refusal"].is_null()
                    && attempt["expected_id"] == WEB_LOC
                    && attempt["actual_id"] == WEB_LOC
            }),
            "{scenario}: live attempts must send the selected action to loc 733: {attempts:?}"
        );
        let attempted_targets: Vec<_> = attempts
            .iter()
            .map(|attempt| {
                let target = attempt["target"].as_array().expect("target tuple");
                (
                    target[0].as_i64().unwrap() as i32,
                    target[1].as_i64().unwrap() as i32,
                )
            })
            .collect();
        for target in WEB_TARGETS {
            assert!(
                attempted_targets.contains(&target),
                "{scenario}: did not interact with web at {target:?}: {attempts:?}"
            );
        }

        let expected_action = if expected_option == 0 {
            "oplocu"
        } else {
            "oploc1"
        };
        assert!(
            attempts.iter().all(|attempt| {
                attempt["outgoing_packet"]["action"] == expected_action
                    && (expected_option != 0 || attempt["outgoing_packet"]["item_id"] == KNIFE)
            }),
            "{scenario}: outgoing loc-op packets must match the content action: {attempts:?}"
        );
        let (attempt_scene, observed_loc_changes) =
            validate_attempt_scene(&attempts, &observations);
        let receipt = json!({
            "request": "WEB-CUT-2",
            "scenario": scenario,
            "account": &username,
            "revision": 289,
            "nav_format": nav::pack::FORMAT_ID,
            "nav_sha256": identity.nav_sha256.clone(),
            "nav_pack": std::env::var("WORLD_NAV_PACK").expect("WORLD_NAV_PACK"),
            "engine_dir": std::env::var("WORLD_ENGINE_DIR").expect("WORLD_ENGINE_DIR"),
            "map_members": profile.map_members(),
            "origin": [MAGE_ARENA_ORIGIN.x, MAGE_ARENA_ORIGIN.z, MAGE_ARENA_ORIGIN.level],
            "route_destination": [ROUTE_DESTINATION.x, ROUTE_DESTINATION.z, ROUTE_DESTINATION.level],
            "web_route": format!("{route:?}"),
            "selected_webs": webs,
            "live_attempts": attempts,
            "web_preflight": preflight,
            "attempt_scene": attempt_scene,
            "observed_loc_changes": observed_loc_changes,
            "observed_failure_messages": failures,
            "outcome": format!("{outcome:?}"),
            "arrival": snapshot.tile(),
            "ingame": snapshot.ingame(),
            "attached": snapshot.attached(),
            "scene_state": snapshot.scene_state(),
            "inventory": snapshot.inventory().iter().map(|item| (item.def.id, item.count)).collect::<Vec<_>>(),
            "equipment": snapshot.equipment().iter().map(|item| (item.def.id, item.count)).collect::<Vec<_>>(),
        });
        let run_dir = save_capture(&evidence_root, scenario, &username, &receipt, &mut client);
        println!("PASS: web_cut_live {scenario} {receipt}");
        // Each account reads the shared web state before starting its route.
        // A slashed state is only accepted after at least 100 observed ticks.
        logout(&mut client, &mut snapshot, &mut pump);
        assert!(run_dir.starts_with(&evidence_root));
    }
}
