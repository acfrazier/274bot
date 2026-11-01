//! Ignored Engine L proof that a native Walk returns its no-route receipt.
//!
//! This test does not move or seed the account. It uses a fresh account with
//! no Dusty key and asks for the Taverley dungeon cell behind that gate.

#![cfg(all(feature = "live-harness", feature = "test-support"))]

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::task::Poll;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use api::snapshot::{GameSnapshot, WorldTile};
use host_play::{ProfileOptions, SharedClientTemplate};
use script::native::walk::Walk;
use script::native::{
    ActionError, ActionHandle, NativeTick, Script, ScriptFailure, ScriptFlow, WalkBit, WalkEnd,
    WalkOptions, WalkReceipt, WalkRequest,
};
use serde_json::{json, Value};
use vault::{Profile, ProfileSettings};

const DUSTY_KEY_ID: i32 = 1590;
const TARGET: WorldTile = WorldTile {
    x: 2875,
    z: 9799,
    level: 0,
};
const RADIUS: u16 = 5;
// The admitted v17 debug regression finished in 8.46 s on chardonnay
// (WALK-NO-ROUTE/repro-after.log): 15 ticks at Engine L's 600 ms cadence
// (engine/src/engine/World.ts:120), plus one tick for receipt delivery.
const MAX_ROUTE_TICKS: u32 = 16;
const ENGINE_L_GAME_PORT: u16 = 47594;
const ENGINE_L_HTTP_PORT: u16 = 3080;
const SETUP_TIMEOUT: Duration = Duration::from_secs(180);

#[derive(Debug, Default)]
struct LiveState {
    latest_tick: Option<u32>,
    ingame: bool,
    scene_state: i32,
    player_tile: Option<WorldTile>,
    dusty_key_count: Option<i32>,
    request_tick: Option<u32>,
    request_tile: Option<WorldTile>,
    dusty_key_count_at_request: Option<i32>,
    last_poll_tick: Option<u32>,
    end_tick: Option<u32>,
    result: Option<Result<WalkReceipt, ActionError>>,
    start_error: Option<String>,
}

struct SharedLiveState {
    state: Mutex<LiveState>,
    changed: Condvar,
}

impl SharedLiveState {
    fn new() -> Self {
        Self {
            state: Mutex::new(LiveState::default()),
            changed: Condvar::new(),
        }
    }
}

struct NativeNoRouteScript {
    shared: Arc<SharedLiveState>,
    handle: Option<ActionHandle<Walk>>,
}

impl Script for NativeNoRouteScript {
    fn tick(&mut self, tick: &mut NativeTick<'_>) -> Result<ScriptFlow, ScriptFailure> {
        if self.handle.is_none() {
            {
                let mut state = self.shared.state.lock().expect("live state");
                if state.request_tick.is_some() {
                    state.start_error = Some("native Walk lost its action handle".into());
                    self.shared.changed.notify_all();
                    return Ok(ScriptFlow::Complete);
                }
                let Some(observed_tick) = state.latest_tick else {
                    state.start_error = Some("native Walk has no observed request tick".into());
                    self.shared.changed.notify_all();
                    return Ok(ScriptFlow::Complete);
                };
                state.request_tick = Some(observed_tick);
                state.request_tile = state.player_tile;
                state.dusty_key_count_at_request = state.dusty_key_count;
            }
            self.shared.changed.notify_all();

            let request = WalkRequest {
                target: TARGET,
                candidate_goals: None,
                loc_id: None,
                radius: RADIUS,
                arrival: nav_planner::contract::ArrivalKind::Reach,
                options: WalkOptions {
                    allow_teleports: WalkBit::Allow,
                    allow_wilderness: WalkBit::Allow,
                    allow_danger_zones: WalkBit::Allow,
                    ..WalkOptions::default()
                },
                required_after: tick.cx.evidence(),
                evidence: None,
                cross: Box::default(),
                avoid: Box::default(),
                protect: false,
                food_guard: false,
                allow: Default::default(),
            };
            match tick.actions.begin::<Walk>(request, &mut tick.cx) {
                Ok(handle) => self.handle = Some(handle),
                Err(error) => {
                    let mut state = self.shared.state.lock().expect("live state");
                    state.start_error = Some(format!("native Walk begin: {error:?}"));
                    self.shared.changed.notify_all();
                    return Ok(ScriptFlow::Complete);
                }
            }
        }

        let Some(handle) = &self.handle else {
            return Ok(ScriptFlow::Continue);
        };
        match tick.actions.poll(handle, &mut tick.cx) {
            Poll::Pending => {
                let mut state = self.shared.state.lock().expect("live state");
                state.last_poll_tick = state.latest_tick;
                self.shared.changed.notify_all();
                Ok(ScriptFlow::Continue)
            }
            Poll::Ready(result) => {
                let mut state = self.shared.state.lock().expect("live state");
                state.last_poll_tick = state.latest_tick;
                state.end_tick = state.latest_tick;
                state.result = Some(result);
                self.handle = None;
                self.shared.changed.notify_all();
                Ok(ScriptFlow::Complete)
            }
        }
    }
}

fn required_path(name: &str) -> PathBuf {
    let value = std::env::var_os(name).unwrap_or_else(|| panic!("{name} is required"));
    let path = PathBuf::from(value);
    assert!(
        path.is_absolute(),
        "{name} must be absolute: {}",
        path.display()
    );
    path
}

fn required_port(name: &str) -> u16 {
    std::env::var(name)
        .unwrap_or_else(|_| panic!("{name} is required"))
        .parse()
        .unwrap_or_else(|error| panic!("{name} must be a port number: {error}"))
}

fn copy_dir(source: &Path, destination: &Path) -> Result<(), String> {
    fs::create_dir(destination)
        .map_err(|error| format!("create {}: {error}", destination.display()))?;
    for entry in
        fs::read_dir(source).map_err(|error| format!("read {}: {error}", source.display()))?
    {
        let entry =
            entry.map_err(|error| format!("read entry in {}: {error}", source.display()))?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let kind = entry
            .file_type()
            .map_err(|error| format!("inspect {}: {error}", source_path.display()))?;
        if kind.is_dir() {
            copy_dir(&source_path, &destination_path)?;
        } else if kind.is_file() {
            fs::copy(&source_path, &destination_path).map_err(|error| {
                format!(
                    "copy {} to {}: {error}",
                    source_path.display(),
                    destination_path.display()
                )
            })?;
        } else {
            return Err(format!(
                "unsupported cache entry type: {}",
                source_path.display()
            ));
        }
    }
    Ok(())
}

fn tile_json(tile: Option<WorldTile>) -> Value {
    tile.map_or(
        Value::Null,
        |tile| json!({"x": tile.x, "z": tile.z, "level": tile.level}),
    )
}
fn within_radius(from: WorldTile, target: WorldTile, radius: u16) -> bool {
    from.level == target.level
        && (from.x - target.x).abs() <= i32::from(radius)
        && (from.z - target.z).abs() <= i32::from(radius)
}

fn wait_for_setup(play: &host_play::Play, shared: &SharedLiveState, account: &str) -> LiveState {
    let deadline = Instant::now() + SETUP_TIMEOUT;
    let mut state = shared.state.lock().expect("live state");
    loop {
        let status_ready = play
            .statuses()
            .iter()
            .any(|status| status.username == account && status.ingame && status.scene_state == 2);
        if state.ingame && state.scene_state == 2 && state.player_tile.is_some() && status_ready {
            return LiveState {
                latest_tick: state.latest_tick,
                ingame: state.ingame,
                scene_state: state.scene_state,
                player_tile: state.player_tile,
                dusty_key_count: state.dusty_key_count,
                ..LiveState::default()
            };
        }
        let now = Instant::now();
        assert!(
            now < deadline,
            "account did not enter a live scene: {state:?}"
        );
        let (next, timeout) = shared
            .changed
            .wait_timeout(state, deadline.saturating_duration_since(now))
            .expect("live state wait");
        state = next;
        assert!(!timeout.timed_out(), "account did not enter a live scene");
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RouteWait {
    Receipt,
    TickBackstop,
    SessionEnded,
}

fn wait_for_route(shared: &SharedLiveState) -> RouteWait {
    let mut state = shared.state.lock().expect("live state");
    loop {
        if state.result.is_some() || state.start_error.is_some() {
            return RouteWait::Receipt;
        }
        if !state.ingame || state.scene_state != 2 {
            return RouteWait::SessionEnded;
        }
        if let (Some(request), Some(last_poll)) = (state.request_tick, state.last_poll_tick) {
            if last_poll.wrapping_sub(request) > MAX_ROUTE_TICKS {
                return RouteWait::TickBackstop;
            }
        }
        state = shared.changed.wait(state).expect("live route wait");
    }
}

fn write_receipt(path: &Path, receipt: &Value) {
    let bytes = serde_json::to_vec_pretty(receipt).expect("serialize live receipt");
    let mut file = fs::File::create(path)
        .unwrap_or_else(|error| panic!("create evidence {}: {error}", path.display()));
    file.write_all(&bytes)
        .unwrap_or_else(|error| panic!("write evidence {}: {error}", path.display()));
    file.sync_all()
        .unwrap_or_else(|error| panic!("flush evidence {}: {error}", path.display()));
    println!("walk-no-route-evidence={}", path.display());
}

fn normalized_detail(detail: &str) -> String {
    detail.to_ascii_lowercase().replace(['_', '-'], " ")
}
fn quester_action_result_json(result: &Result<WalkReceipt, ActionError>) -> Value {
    match result {
        Ok(receipt) => match receipt.clone().into_arrival() {
            Ok(_) => json!({"kind": "arrived"}),
            Err(ActionError::Blocked(detail)) => {
                json!({"kind": "blocked", "detail": detail.as_ref()})
            }
            Err(error) => json!({"kind": "action_error", "error": format!("{error:?}")}),
        },
        Err(error) => json!({"kind": "action_error", "error": format!("{error:?}")}),
    }
}

#[test]
#[ignore = "requires LIVE=1, BOT_CPU=1, BOT_LIVE_NAME_PREFIX, Engine L ports 47594/3080, WORLD_ENGINE_DIR, WORLD_NAV_PACK, WORLD_NAV_FLAGS, WORLD_CACHE_DIR, RS2B0T and LIVE_EVIDENCE_DIR"]
fn live_scorpcatcher_no_key_walk_returns_no_path_with_detail_within_observed_ticks() {
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"));
    assert_eq!(std::env::var("BOT_CPU").as_deref(), Ok("1"));
    let game_port = required_port("WORLD_GAME_PORT");
    let http_port = required_port("WORLD_HTTP_PORT");
    assert_eq!(game_port, ENGINE_L_GAME_PORT, "use the Engine L tunnel");
    assert_eq!(
        http_port, ENGINE_L_HTTP_PORT,
        "use the Engine L HTTP tunnel"
    );
    let evidence_root = required_path("LIVE_EVIDENCE_DIR");
    fs::create_dir_all(&evidence_root).expect("create evidence root");
    let serial = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock")
        .as_nanos();
    let names = host_play::mint_live_names(1);
    let account = names.first().expect("one minted account").clone();
    let (username, password) = host_play::mint_live_entries(&names)
        .pop()
        .expect("one minted credential");
    assert_eq!(username, account);
    let run_dir = evidence_root.join(format!("walk-no-route-{account}-{serial}"));
    fs::create_dir(&run_dir).expect("create run evidence directory");
    let cache_source = required_path("WORLD_CACHE_DIR");
    let cache_dir = run_dir.join("cache");
    copy_dir(&cache_source, &cache_dir).unwrap_or_else(|error| panic!("copy live cache: {error}"));

    let engine_dir = required_path("WORLD_ENGINE_DIR");
    let nav_pack = required_path("WORLD_NAV_PACK");
    let nav_flags = required_path("WORLD_NAV_FLAGS");
    let catalog_root = required_path("RS2B0T");
    let options = ProfileOptions {
        profile: Some("local-289".into()),
        revision: Some("289".into()),
        host: Some("127.0.0.1".into()),
        asset_host: Some("127.0.0.1".into()),
        port: Some(game_port),
        http_port: Some(http_port),
        engine_dir: Some(engine_dir),
        cache_dir: Some(cache_dir),
        unpack_dir: Some(run_dir.join("client-unpack")),
        nav_pack: Some(nav_pack),
        nav_flags: Some(nav_flags),
        vault_path: Some(run_dir.join("vault")),
        catalog_root: Some(catalog_root),
        ..ProfileOptions::default()
    };
    let profile = options
        .resolve(None)
        .expect("resolve Engine L profile")
        .bind_runtime()
        .expect("bind Engine L profile");
    let template = SharedClientTemplate::load(Arc::clone(&profile)).expect("load profile template");
    assert!(
        template.world().is_some(),
        "the v17 navigation pack must load"
    );

    let uid = i32::try_from(serial % i32::MAX as u128).expect("bounded uid");
    let profile = Profile {
        username: account.clone(),
        password: password.into(),
        uid,
        settings: ProfileSettings::default(),
    };
    let shared = Arc::new(SharedLiveState::new());
    let frame_shared = Arc::clone(&shared);
    let frame_account = account.clone();
    let play = host_play::run_with_template(
        template,
        false,
        vec![profile],
        |_| (None, None),
        move |client, username, _| {
            if username != frame_account {
                return;
            }
            let mut snapshot = GameSnapshot::new();
            snapshot.rebuild(client);
            let tile = snapshot
                .tile()
                .map(|(x, z, level)| WorldTile { x, z, level });
            let mut state = frame_shared.state.lock().expect("live state");
            let changed = state.latest_tick != Some(snapshot.tick())
                || state.ingame != client.ingame
                || state.scene_state != client.scene_state
                || state.player_tile != tile;
            state.latest_tick = Some(snapshot.tick());
            state.ingame = client.ingame;
            state.scene_state = client.scene_state;
            state.player_tile = tile;
            state.dusty_key_count = Some(snapshot.inv_count(DUSTY_KEY_ID));
            drop(state);
            if changed {
                frame_shared.changed.notify_all();
            }
        },
    )
    .expect("start one-account live Play");

    let setup = wait_for_setup(&play, &shared, &account);
    let origin = setup.player_tile.expect("ready setup has a tile");
    let dusty_key_count = setup.dusty_key_count.expect("ready setup has inventory");
    let setup_receipt = json!({
        "phase": "ready-before-walk",
        "account": account,
        "request": {"target": tile_json(Some(TARGET)), "radius": RADIUS},
        "origin": tile_json(Some(origin)),
        "observed_tick": setup.latest_tick,
        "scene_state": setup.scene_state,
        "dusty_key_id": DUSTY_KEY_ID,
        "dusty_key_count": dusty_key_count,
        "world_engine_dir": std::env::var("WORLD_ENGINE_DIR").expect("WORLD_ENGINE_DIR"),
        "world_nav_pack": std::env::var("WORLD_NAV_PACK").expect("WORLD_NAV_PACK"),
    });
    write_receipt(&run_dir.join("setup.json"), &setup_receipt);
    assert_eq!(
        dusty_key_count, 0,
        "fresh account must not hold the Dusty key"
    );
    assert!(
        !within_radius(origin, TARGET, RADIUS),
        "the live account must start outside the requested arrival radius"
    );

    let handle = play.script_start_handle();
    let run = handle
        .start_test_script(
            &account,
            Box::new(NativeNoRouteScript {
                shared: Arc::clone(&shared),
                handle: None,
            }),
            None,
        )
        .expect("start native no-route probe");
    assert_eq!(play.script_native_run(&account), Some(run));
    play.wake(&account);

    let wait = wait_for_route(&shared);
    let state = shared.state.lock().expect("live state");
    let result_json = state.result.as_ref().map(|result| match result {
        Ok(receipt) => json!({
            "kind": "walk_receipt",
            "request_id": receipt.request_id,
            "typed_end": format!("{:?}", receipt.end),
            "detail": receipt.detail.as_deref(),
            "failure_detail": receipt.failure_detail().as_deref(),
        }),
        Err(error) => json!({
            "kind": "action_error",
            "error": format!("{error:?}"),
        }),
    });
    let quester_action_result = state.result.as_ref().map(quester_action_result_json);

    let observed_ticks = state
        .request_tick
        .zip(state.end_tick)
        .map(|(start, end)| end.wrapping_sub(start));
    let report = json!({
        "phase": "native-walk-terminal",
        "wait": format!("{wait:?}"),
        "account": account,
        "request": {
            "kind": "Reach",
            "origin": tile_json(state.request_tile),
            "target": tile_json(Some(TARGET)),
            "radius": RADIUS,
            "dusty_key_id": DUSTY_KEY_ID,
            "dusty_key_count": state.dusty_key_count_at_request,
            "allow_teleports": true,
            "allow_wilderness": true,
            "allow_danger_zones": true,
        },
        "request_tick": state.request_tick,
        "end_tick": state.end_tick,
        "last_script_poll_tick": state.last_poll_tick,
        "observed_game_ticks": observed_ticks,
        "max_route_ticks": MAX_ROUTE_TICKS,
        "result": result_json,
        "quester_action_result": quester_action_result,
        "start_error": state.start_error,
    });
    write_receipt(&run_dir.join("receipt.json"), &report);
    drop(state);

    assert_eq!(
        wait,
        RouteWait::Receipt,
        "route ended before a native receipt"
    );
    let state = shared.state.lock().expect("live state");
    assert!(
        state.start_error.is_none(),
        "native Walk start failed: {:?}",
        state.start_error
    );
    let result = state.result.as_ref().expect("native Walk receipt");
    let receipt = result
        .as_ref()
        .expect("native Walk must return a typed receipt");
    assert_eq!(
        receipt.end,
        WalkEnd::NoPath,
        "native route must end as NoPath"
    );
    let detail = receipt
        .detail
        .as_deref()
        .expect("NoPath must retain route detail");
    assert!(!detail.trim().is_empty(), "NoPath detail must not be empty");
    let reason = normalized_detail(detail);
    assert!(
        reason.contains("dusty key"),
        "the real map must report the missing Dusty key, not a generic cutoff: {detail:?}"
    );
    let quester_error = receipt
        .clone()
        .into_arrival()
        .expect_err("NoPath cannot satisfy the Quester Walk arrival");
    match quester_error {
        ActionError::Blocked(reason) => assert_eq!(reason.as_ref(), detail),
        other => panic!("NoPath must reach Quester as Blocked(detail), got {other:?}"),
    }
    let ticks = state
        .request_tick
        .zip(state.end_tick)
        .map(|(start, end)| end.wrapping_sub(start))
        .expect("request and terminal ticks are recorded");
    assert!(
        ticks <= MAX_ROUTE_TICKS,
        "NoPath arrived after {ticks} observed game ticks"
    );
    assert_eq!(state.dusty_key_count_at_request, Some(0));
}
