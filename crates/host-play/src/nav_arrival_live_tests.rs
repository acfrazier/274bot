//! Ignored live proof that a native Walk settles with its retained receipt.
//!
//! Run once per scenario with `NAV_ARRIVAL_SCENARIO=return` and
//! `NAV_ARRIVAL_SCENARIO=yew`; both runs require the copied cache and the
//! caller-supplied throwaway HOME under `BOT_EVIDENCE_DIR`.

use super::*;
use api::snapshot::{GameSnapshot, WorldTile};
use client::render::backend::FrameOutput;
use host::FrameBuf;
use script::native::walk::Walk;
use script::native::{
    ActionError, ActionHandle, NativeTick, Script, ScriptFailure, ScriptFlow, WalkEnd, WalkReceipt,
    WalkRequest,
};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::task::Poll;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use vault::{Profile, ProfileSettings};

const RETURN_ORIGIN: WorldTile = WorldTile {
    x: 3185,
    z: 3440,
    level: 0,
};
const RETURN_TARGET: WorldTile = WorldTile {
    x: 3017,
    z: 3170,
    level: 0,
};
const YEW_SCENE_SEED: WorldTile = WorldTile {
    x: 3150,
    z: 3230,
    level: 0,
};
const SETUP_TIMEOUT: Duration = Duration::from_secs(180);
const WALK_TIMEOUT: Duration = Duration::from_secs(12 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scenario {
    Return,
    Yew,
}

impl Scenario {
    fn from_env() -> Self {
        match std::env::var("NAV_ARRIVAL_SCENARIO").as_deref() {
            Ok("return") => Self::Return,
            Ok("yew") => Self::Yew,
            other => panic!("NAV_ARRIVAL_SCENARIO must be return or yew, got {other:?}"),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Return => "return",
            Self::Yew => "yew",
        }
    }

    fn seed(self) -> WorldTile {
        match self {
            Self::Return => RETURN_ORIGIN,
            Self::Yew => YEW_SCENE_SEED,
        }
    }
}

#[derive(Debug, Clone)]
struct WalkSpec {
    target: WorldTile,
    radius: u16,
    loc_id: Option<i32>,
    loc_name: Option<String>,
    loc_actions: Vec<String>,
}

impl WalkSpec {
    fn return_walk() -> Self {
        Self {
            target: RETURN_TARGET,
            radius: 12,
            loc_id: None,
            loc_name: None,
            loc_actions: Vec::new(),
        }
    }

    fn request(&self, required_after: api::quest_progress::EvidenceStamp) -> WalkRequest {
        WalkRequest {
            target: self.target,
            radius: self.radius,
            arrival: nav::arrival::ArrivalKind::Reach,
            loc_id: self.loc_id,
            options: script::FindOptions {
                allow_teleports: false,
                allow_wilderness: false,
                allow_bank_fetch: false,
            },
            required_after,
            evidence: None,
            cross: Box::default(),
            protect: false,
            allow: Default::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct SceneProof {
    ingame: bool,
    scene_state: i32,
    player_tile: Option<WorldTile>,
    target_loc_visible: bool,
    target_operable: Option<bool>,
}

#[derive(Default)]
struct LiveState {
    teleport_sent: bool,
    ready: bool,
    setup_error: Option<String>,
    target: Option<WalkSpec>,
    scene: SceneProof,
    walk_started: bool,
    start_error: Option<String>,
    receipt: Option<Result<WalkReceipt, ActionError>>,
}

struct NativeWalkScript {
    state: Arc<parking_lot::Mutex<LiveState>>,
    walk: WalkSpec,
    handle: Option<ActionHandle<Walk>>,
}

impl Script for NativeWalkScript {
    fn tick(&mut self, tick: &mut NativeTick<'_>) -> Result<ScriptFlow, ScriptFailure> {
        let begin = {
            let mut state = self.state.lock();
            if state.walk_started {
                false
            } else {
                state.walk_started = true;
                true
            }
        };
        if begin {
            match tick
                .actions
                .begin::<Walk>(self.walk.request(tick.cx.evidence()), &mut tick.cx)
            {
                Ok(handle) => self.handle = Some(handle),
                Err(error) => {
                    self.state.lock().start_error = Some(format!("native Walk begin: {error:?}"));
                }
            }
        }

        if let Some(handle) = &self.handle {
            if let Poll::Ready(result) = tick.actions.poll(handle, &mut tick.cx) {
                self.state.lock().receipt = Some(result);
                self.handle = None;
            }
        }
        Ok(ScriptFlow::Continue)
    }
}

#[derive(Clone)]
struct RouteSample {
    request_id: u64,
    generation: u64,
    route: nav::router::Route,
}

fn stamp_utc() -> String {
    format!(
        "{}Z",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_secs()
    )
}

fn absolute_env_path(name: &str) -> PathBuf {
    let path = PathBuf::from(
        std::env::var_os(name).unwrap_or_else(|| panic!("{name} must be explicitly supplied")),
    );
    assert!(
        path.is_absolute(),
        "{name} must be absolute: {}",
        path.display()
    );
    path
}

fn required_directory(name: &str, evidence_root: &Path) -> PathBuf {
    let supplied = absolute_env_path(name);
    let canonical = std::fs::canonicalize(&supplied)
        .unwrap_or_else(|error| panic!("resolve {name} {}: {error}", supplied.display()));
    assert!(
        canonical.is_dir(),
        "{name} must be a directory: {}",
        canonical.display()
    );
    assert!(
        canonical.starts_with(evidence_root),
        "{name} must resolve inside BOT_EVIDENCE_DIR to a throwaway copy: {}",
        canonical.display()
    );
    canonical
}

fn chebyshev(from: WorldTile, to: WorldTile) -> u32 {
    from.x.abs_diff(to.x).max(from.z.abs_diff(to.z))
}

/// The approximate Lumbridge point only opens the live scene. The destination
/// is always selected from the scene's loaded loc facts, never from a guessed id.
fn yew_walk(snapshot: &GameSnapshot, origin: WorldTile) -> Option<WalkSpec> {
    snapshot
        .locs()
        .iter()
        .filter_map(|loc| {
            let name = loc.name.as_deref()?;
            if !name.to_ascii_lowercase().contains("yew")
                || !loc
                    .actions
                    .iter()
                    .flatten()
                    .any(|action| action.eq_ignore_ascii_case("Chop down"))
            {
                return None;
            }
            let distance = chebyshev(origin, loc.tile);
            if distance <= 1 || distance > 52 {
                return None;
            }
            Some((
                distance,
                WalkSpec {
                    target: loc.tile,
                    radius: 1,
                    loc_id: Some(loc.id),
                    loc_name: Some(name.to_owned()),
                    loc_actions: loc.actions.iter().flatten().cloned().collect(),
                },
            ))
        })
        .max_by_key(|(distance, _)| *distance)
        .map(|(_, walk)| walk)
}

fn frame_hook(
    state: Arc<parking_lot::Mutex<LiveState>>,
    account: String,
    scenario: Scenario,
) -> impl Fn(&mut client::client::Client, &str, crate::SlotFrameInput) + Send + Sync + 'static {
    move |client, username, _hold| {
        if username != account {
            return;
        }
        client.set_draw(true);
        let mut snapshot = GameSnapshot::new();
        snapshot.rebuild(client);
        let tile = snapshot
            .tile()
            .map(|(x, z, level)| WorldTile { x, z, level });
        let seed = scenario.seed();
        let mut state = state.lock();
        if scenario == Scenario::Yew
            && state.target.is_none()
            && state.teleport_sent
            && tile == Some(seed)
            && client.ingame
            && client.scene_state == 2
        {
            state.target = yew_walk(&snapshot, seed);
        }
        state.scene = SceneProof {
            ingame: client.ingame,
            scene_state: client.scene_state,
            player_tile: tile,
            target_loc_visible: state.target.as_ref().is_some_and(|target| {
                target.loc_id.is_some_and(|id| {
                    snapshot
                        .locs()
                        .iter()
                        .any(|loc| loc.id == id && loc.tile == target.target)
                })
            }),
            target_operable: state.target.as_ref().and_then(|target| {
                let id = target.loc_id?;
                let loc = snapshot
                    .locs()
                    .iter()
                    .find(|loc| loc.id == id && loc.tile == target.target)?;
                tile.and_then(|from| {
                    api::query::loc_approach::can_operate_from(loc, snapshot.scene(), from)
                })
            }),
        };
        if !(client.ingame && client.scene_state == 2) || state.setup_error.is_some() {
            return;
        }
        if !state.teleport_sent {
            api::interact::seed_at(client, seed.level, seed.x, seed.z);
            state.teleport_sent = true;
            return;
        }
        if tile != Some(seed) {
            state.ready = false;
            return;
        }
        state.ready = state.target.is_some();
    }
}

fn slot_status(play: &Play, account: &str) -> Value {
    play.statuses()
        .into_iter()
        .find(|status| status.username == account)
        .map(|status| {
            json!({
                "username": status.username,
                "ingame": status.ingame,
                "scene_state": status.scene_state,
            })
        })
        .unwrap_or(Value::Null)
}

fn slot_is_ready(play: &Play, account: &str) -> bool {
    play.statuses()
        .iter()
        .any(|status| status.username == account && status.ingame && status.scene_state == 2)
}

fn wait_for_setup(
    play: &Play,
    account: &str,
    state: &Arc<parking_lot::Mutex<LiveState>>,
) -> Result<(), String> {
    let deadline = Instant::now() + SETUP_TIMEOUT;
    loop {
        let (ready, error) = {
            let state = state.lock();
            (state.ready, state.setup_error.clone())
        };
        if let Some(error) = error {
            return Err(error);
        }
        if ready && slot_is_ready(play, account) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "account did not reach the scenario seed in ingame scene_state 2 within {SETUP_TIMEOUT:?}"
            ));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn wait_frame_after(mailbox: &FrameBuf, generation: u64, timeout: Duration) -> Option<FrameOutput> {
    let deadline = Instant::now() + timeout;
    loop {
        if mailbox.generation() > generation {
            if let Some(frame) = mailbox.take() {
                return Some(frame);
            }
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn encode_png(frame: FrameOutput) -> Result<(Vec<u8>, u32, u32), String> {
    let (width, height, pixels) = match frame {
        FrameOutput::PixMap(pixmap) => (
            u32::try_from(pixmap.width).map_err(|error| error.to_string())?,
            u32::try_from(pixmap.height).map_err(|error| error.to_string())?,
            pixmap.pixels,
        ),
        FrameOutput::Texture(texture) => (texture.width, texture.height, texture.read_back()),
    };
    let pixel_count = (width as usize)
        .checked_mul(height as usize)
        .ok_or_else(|| "rendered frame dimensions overflow".to_string())?;
    if width == 0 || height == 0 || pixels.len() < pixel_count {
        return Err(format!(
            "rendered frame has invalid dimensions/data: {width}x{height}, {} pixels",
            pixels.len()
        ));
    }
    let mut rgba = Vec::with_capacity(pixel_count * 4);
    for pixel in pixels.into_iter().take(pixel_count) {
        rgba.extend_from_slice(&[
            ((pixel >> 16) & 0xff) as u8,
            ((pixel >> 8) & 0xff) as u8,
            (pixel & 0xff) as u8,
            0xff,
        ]);
    }
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(|error| error.to_string())?;
        writer
            .write_image_data(&rgba)
            .map_err(|error| error.to_string())?;
    }
    Ok((bytes, width, height))
}

fn route_sample(play: &Play, account: &str) -> Option<RouteSample> {
    let navs = play.navs.lock().unwrap();
    let bot = navs.get(account)?;
    Some(RouteSample {
        request_id: bot.route_request_id,
        generation: bot.route_generation,
        route: bot.route.clone()?,
    })
}

fn route_json(sample: Option<&RouteSample>) -> Value {
    let Some(sample) = sample else {
        return Value::Null;
    };
    let legs = sample
        .route
        .legs
        .iter()
        .map(|leg| match leg {
            nav::router::Leg::Walk { tiles } => json!({"kind": "walk", "tiles": tiles}),
            nav::router::Leg::Transport { edge } => json!({
                "kind": format!("{:?}", edge.kind),
                "at": edge.at,
                "to": edge.to,
                "loc_id": edge.loc_id,
                "option": edge.option,
                "ticks": edge.ticks,
            }),
        })
        .collect::<Vec<_>>();
    json!({
        "request_id": sample.request_id,
        "generation": sample.generation,
        "destination": sample.route.dest,
        "ticks": sample.route.ticks,
        "legs": legs,
    })
}

fn nav_cache_identity(play: &Play) -> Value {
    let Some(profile) = play.server_profile() else {
        return json!({"exposed": false, "reason": "Play has no bound server profile"});
    };
    let origin = match profile.nav_origin() {
        NavOrigin::Bundled { identity, path } => json!({
            "kind": "bundled",
            "path": path.display().to_string(),
            "identity": identity,
        }),
        NavOrigin::External { path } => json!({
            "kind": "external",
            "path": path.display().to_string(),
        }),
    };
    json!({
        "profile": profile.name(),
        "revision": format!("{:?}", profile.revision()),
        "cache_id": profile.cache_id(),
        "selected_game_data_status": profile.game_data_status().reason(),
        "nav_pack": profile.nav_pack().display().to_string(),
        "nav_origin": origin,
        "nav_manifest": profile.nav_identity(),
        "nav_availability": format!("{:?}", profile.nav_availability()),
        "nav_load_counters": format!("{:?}", profile.nav_load_counters()),
        "nav_world_loaded": play.world().is_some(),
    })
}

fn target_json(walk: Option<&WalkSpec>) -> Value {
    let Some(walk) = walk else {
        return Value::Null;
    };
    json!({
        "tile": walk.target,
        "radius": walk.radius,
        "loc_id": walk.loc_id,
        "loc_name": walk.loc_name,
        "loc_actions": walk.loc_actions,
        "loc_identity_source": walk.loc_id.map(|_| "live GameSnapshot::locs()"),
    })
}

fn scene_json(scene: SceneProof) -> Value {
    json!({
        "ingame": scene.ingame,
        "scene_state": scene.scene_state,
        "player_tile": scene.player_tile,
        "target_loc_visible": scene.target_loc_visible,
        "target_operable": scene.target_operable,
    })
}

fn receipt_json(result: Option<&Result<WalkReceipt, ActionError>>) -> Value {
    match result {
        Some(Ok(receipt)) => json!({
            "source": "retained result of polling native Walk action handle",
            "request_id": receipt.request_id,
            "evidence": {
                "run": {
                    "slot": receipt.evidence.run.slot,
                    "run": receipt.evidence.run.run,
                    "session": receipt.evidence.run.session,
                },
                "tick": receipt.evidence.tick,
                "sequence": receipt.evidence.sequence,
            },
            "end": format!("{:?}", receipt.end),
            "blocked_zones": receipt.blocked.as_deref().map(|zones| format!("{zones:?}")),
            "detail": receipt.detail.as_deref(),
        }),
        Some(Err(error)) => json!({
            "source": "native Walk action handle returned an error",
            "error": format!("{error:?}"),
        }),
        None => Value::Null,
    }
}

struct EvidenceContext<'a> {
    scenario: Scenario,
    account: &'a str,
    stamp: &'a str,
    play: &'a Play,
    nav_identity: &'a Value,
}

impl EvidenceContext<'_> {
    fn document(
        &self,
        phase: &str,
        target: Option<&WalkSpec>,
        scene: SceneProof,
        route: Option<&RouteSample>,
        result: Option<&Result<WalkReceipt, ActionError>>,
        extra_error: Option<&str>,
    ) -> Value {
        json!({
            "task": "NAV-ARRIVAL-1",
            "scenario": self.scenario.name(),
            "account": self.account,
            "timestamp_utc": self.stamp,
            "phase": phase,
            "client": {"bot_cpu": true, "live": true},
            "setup": {
                "seed_tile": self.scenario.seed(),
                "slot_status": slot_status(self.play, self.account),
                "scene": scene_json(scene),
            },
            "request": target.map(|walk| json!({
                "type": "native Walk",
                "target": walk.target,
                "radius": walk.radius,
                "loc_id": walk.loc_id,
                "allow_teleports": false,
                "allow_wilderness": false,
                "allow_bank_fetch": false,
                "target_identity": target_json(Some(walk)),
            })),
            "route": route_json(route),
            "native_walk_receipt": receipt_json(result),
            "nav_cache_identity": self.nav_identity,
            "harness_error": extra_error,
        })
    }
}

/// Writes a real rendered frame (when one is available) and its matching JSON
/// before the caller performs the corresponding assertion.
fn write_capture(
    run_dir: &Path,
    stamp: &str,
    step: &str,
    failed: bool,
    mut report: Value,
    mailbox: &FrameBuf,
) -> bool {
    let stem = if failed {
        format!("{stamp}_FAIL-{step}")
    } else {
        format!("{stamp}_{step}")
    };
    let png_path = run_dir.join(format!("{stem}.png"));
    let json_path = run_dir.join(format!("{stem}.json"));
    let generation = mailbox.generation();
    let encoded = wait_frame_after(mailbox, generation, Duration::from_secs(8)).map(encode_png);
    let (png_written, capture) = match encoded {
        Some(Ok((bytes, width, height))) => match std::fs::write(&png_path, bytes) {
            Ok(()) => (
                true,
                json!({
                    "frame_source": "host::FrameBuf actual rendered FrameOutput",
                    "png_file": png_path.file_name().and_then(|name| name.to_str()),
                    "width": width,
                    "height": height,
                    "png_written": true,
                }),
            ),
            Err(error) => (
                false,
                json!({
                    "frame_source": "host::FrameBuf actual rendered FrameOutput",
                    "png_file": png_path.file_name().and_then(|name| name.to_str()),
                    "png_written": false,
                    "png_error": error.to_string(),
                }),
            ),
        },
        Some(Err(error)) => (
            false,
            json!({
                "frame_source": "host::FrameBuf actual rendered FrameOutput",
                "png_written": false,
                "capture_error": error,
            }),
        ),
        None => (
            false,
            json!({
                "frame_source": "host::FrameBuf actual rendered FrameOutput",
                "png_written": false,
                "capture_error": "no rendered frame arrived before the capture deadline",
            }),
        ),
    };
    report["capture"] = capture;
    std::fs::write(
        &json_path,
        serde_json::to_vec_pretty(&report).expect("serialize NAV-ARRIVAL live evidence"),
    )
    .unwrap_or_else(|error| panic!("write evidence {}: {error}", json_path.display()));
    println!("nav-arrival-evidence-json={}", json_path.display());
    if png_written {
        println!("nav-arrival-evidence-png={}", png_path.display());
    }
    png_written
}

fn live_profile(name: String, password: String) -> Profile {
    Profile {
        username: name,
        password: password.into(),
        uid: 274_279_311,
        settings: ProfileSettings::default(),
    }
}

#[test]
#[ignore = "requires LIVE=1, BOT_CPU=1, and the local R289 engine/nav fixtures"]
fn live_native_walk_arrival_retains_arrived_receipt() {
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"), "requires LIVE=1");
    assert_eq!(
        std::env::var("BOT_CPU").as_deref(),
        Ok("1"),
        "requires BOT_CPU=1"
    );
    let scenario = Scenario::from_env();
    let supplied_root = absolute_env_path("BOT_EVIDENCE_DIR");
    std::fs::create_dir_all(&supplied_root).expect("create BOT_EVIDENCE_DIR");
    let evidence_root = std::fs::canonicalize(&supplied_root).expect("resolve BOT_EVIDENCE_DIR");
    let home = required_directory("HOME", &evidence_root);
    let copied_cache = required_directory("BOT_CACHE_DIR", &evidence_root);
    let nav_pack_supplied = absolute_env_path("WORLD_NAV_PACK");
    let nav_pack = std::fs::canonicalize(&nav_pack_supplied).unwrap_or_else(|error| {
        panic!(
            "resolve WORLD_NAV_PACK {}: {error}",
            nav_pack_supplied.display()
        )
    });
    assert!(nav_pack.is_file(), "WORLD_NAV_PACK must be a file");
    let engine_supplied = absolute_env_path("WORLD_ENGINE_DIR");
    let engine_dir = std::fs::canonicalize(&engine_supplied).unwrap_or_else(|error| {
        panic!(
            "resolve WORLD_ENGINE_DIR {}: {error}",
            engine_supplied.display()
        )
    });
    assert!(engine_dir.is_dir(), "WORLD_ENGINE_DIR must be a directory");

    let timestamp = stamp_utc();
    let names = mint_live_names(1);
    let (account, password) = mint_live_entries(&names)
        .into_iter()
        .next()
        .expect("mint one owned live account");
    let run_dir = evidence_root.join(format!(
        "NAV-ARRIVAL-1_{}_{}_{}",
        scenario.name(),
        account,
        timestamp
    ));
    std::fs::create_dir_all(&run_dir).expect("create per-account capture directory");

    let options = ProfileOptions {
        profile: Some("local-289".into()),
        revision: Some("289".into()),
        host: Some("127.0.0.1".into()),
        asset_host: Some("127.0.0.1".into()),
        port: Some(45594),
        http_port: Some(2080),
        engine_dir: Some(engine_dir),
        cache_dir: Some(copied_cache.clone()),
        unpack_dir: Some(copied_cache),
        vault_path: Some(home.join("vault-nav-arrival")),
        nav_pack: Some(nav_pack),
        ..ProfileOptions::default()
    };
    let template = options
        .resolve(None)
        .expect("resolve explicit local-289 profile")
        .prepare_template()
        .expect("prepare local-289 live template with copied cache");
    let state = Arc::new(parking_lot::Mutex::new(LiveState {
        target: (scenario == Scenario::Return).then(WalkSpec::return_walk),
        ..LiveState::default()
    }));
    let mailbox = FrameBuf::new();
    let frame_mailbox = Arc::clone(&mailbox);
    let frame_state = Arc::clone(&state);
    let frame_account = account.clone();
    let play = run_with_template(
        template,
        false,
        vec![live_profile(account.clone(), password)],
        move |username| {
            if username == frame_account {
                (None, Some(Arc::clone(&frame_mailbox)))
            } else {
                (None, None)
            }
        },
        frame_hook(frame_state, account.clone(), scenario),
    )
    .expect("start one-slot live Play");

    let setup_result = wait_for_setup(&play, &account, &state);
    if let Err(error) = &setup_result {
        state.lock().setup_error = Some(error.clone());
    }
    let (target, scene, setup_error) = {
        let state = state.lock();
        (state.target.clone(), state.scene, state.setup_error.clone())
    };
    let nav_identity = nav_cache_identity(&play);
    let evidence = EvidenceContext {
        scenario,
        account: &account,
        stamp: &timestamp,
        play: &play,
        nav_identity: &nav_identity,
    };
    let setup_ok = setup_result.is_ok()
        && setup_error.is_none()
        && target.is_some()
        && scene.ingame
        && scene.scene_state == 2
        && scene.player_tile == Some(scenario.seed())
        && (scenario != Scenario::Yew || scene.target_loc_visible)
        && slot_is_ready(&play, &account);
    let setup_report = evidence.document(
        "ready-before-native-script",
        target.as_ref(),
        scene,
        None,
        None,
        setup_error.as_deref(),
    );
    let setup_png = write_capture(
        &run_dir,
        &timestamp,
        if setup_ok { "01-origin" } else { "setup" },
        !setup_ok,
        setup_report,
        &mailbox,
    );
    assert!(
        setup_ok,
        "account must be ingame with scene_state 2 at {:?} before Walk: {:?}",
        scenario.seed(),
        setup_error
    );
    assert!(
        setup_png,
        "save the real pre-script scene PNG before walking"
    );
    let walk = target.expect("setup evidence checked a selected native Walk target");

    let start_handle = play.script_start_handle();
    match start_handle.start_test_script(
        &account,
        Box::new(NativeWalkScript {
            state: Arc::clone(&state),
            walk: walk.clone(),
            handle: None,
        }),
        None,
    ) {
        Ok(run) => {
            if play.script_native_run(&account) != Some(run) {
                state.lock().start_error = Some("installed native run key was not retained".into());
            }
            play.wake(&account);
        }
        Err(error) => state.lock().start_error = Some(error),
    }

    let deadline = Instant::now() + WALK_TIMEOUT;
    let mut route = None;
    let mut timed_out = false;
    loop {
        if route.is_none() {
            route = route_sample(&play, &account);
        }
        let (terminal, start_error) = {
            let state = state.lock();
            (state.receipt.is_some(), state.start_error.clone())
        };
        if terminal || start_error.is_some() {
            break;
        }
        if Instant::now() >= deadline {
            timed_out = true;
            state.lock().start_error = Some(format!("native Walk wait exceeded {WALK_TIMEOUT:?}"));
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    if let Some(sample) = route_sample(&play, &account) {
        route = Some(sample);
    }
    let (scene, result, start_error) = {
        let state = state.lock();
        (
            state.scene,
            state.receipt.clone(),
            state.start_error.clone(),
        )
    };
    let arrived = matches!(result.as_ref(), Some(Ok(receipt)) if receipt.end == WalkEnd::Arrived);
    let final_report = evidence.document(
        if arrived {
            "native-walk-terminal"
        } else {
            "native-walk-failure"
        },
        Some(&walk),
        scene,
        route.as_ref(),
        result.as_ref(),
        start_error.as_deref(),
    );
    let final_png = write_capture(
        &run_dir,
        &timestamp,
        "02-final-walk",
        !arrived || timed_out || start_error.is_some(),
        final_report,
        &mailbox,
    );

    assert!(
        final_png,
        "save the real final-scene PNG and matching receipt JSON"
    );
    assert!(
        !timed_out,
        "native Walk did not produce a retained terminal receipt: {result:?}"
    );
    assert!(
        start_error.is_none(),
        "native Walk script failed to start: {start_error:?}"
    );
    assert!(
        arrived,
        "the native Walk receipt must end at WalkEnd::Arrived, not another host/Traveller terminal: {result:?}"
    );
    assert!(
        scene.ingame && scene.scene_state == 2 && scene.player_tile.is_some(),
        "final evidence must be from a ready live scene: {scene:?}"
    );
    let end = scene.player_tile.expect("final scene tile asserted above");
    assert_eq!(end.level, walk.target.level, "arrival plane");
    if scenario == Scenario::Return {
        assert!(
            chebyshev(end, walk.target) <= u32::from(walk.radius),
            "Return ended outside the requested anchor radius"
        );
    }
    if scenario == Scenario::Yew {
        assert!(walk.loc_id.is_some() && walk.loc_name.is_some());
        assert_eq!(
            scene.target_operable,
            Some(true),
            "the radius-1 yew walk must end on a legal full-footprint stand"
        );
        println!(
            "nav-arrival-yew-fixture id={} name={:?} tile={:?} actions={:?}",
            walk.loc_id.expect("Yew target has selected loc id"),
            walk.loc_name,
            walk.target,
            walk.loc_actions
        );
    }
}
