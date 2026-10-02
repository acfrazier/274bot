//! Persistent ignored LIVE regressions for panel manual movement takeover.
//!
//! Input enters through `Session::capture_tx`, the same sender used by the
//! production panel capture path. The second test exercises both M5's paused
//! carry and the post-Resume takeover window with the production script APIs.

use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use host::InputEv;
use nav::router::FindOptions;
use nav::tile::Tile;
use nav::world::NavWorld;
use nav::WorldState;

use crate::session::Session;

const LOCAL_GAME_PORT: u16 = 45_594;
const LOCAL_ASSET_PORT: u16 = 2_080;
const SCENE_WAIT: Duration = Duration::from_secs(150);
const WALK_WAIT: Duration = Duration::from_secs(90);
const TERMINAL_WAIT: Duration = Duration::from_secs(30);
const SETTLE_WAIT: Duration = Duration::from_secs(5);
const MINIMAP_CLICK: (i32, i32) = (628, 83);

struct LiveInputs {
    cache: PathBuf,
    engine: PathBuf,
    nav_pack: PathBuf,
    evidence: PathBuf,
}

fn live_inputs() -> Result<LiveInputs, String> {
    assert_eq!(
        env::var("LIVE").as_deref(),
        Ok("1"),
        "run ignored LIVE tests with LIVE=1"
    );
    assert_eq!(env::var("BOT_NAV_BUILD").as_deref(), Ok("skip"));
    assert_eq!(env::var("BOT_CPU").as_deref(), Ok("1"));
    // These ignored LIVE cases run serially in their own test process.
    if env::var_os("BOT_LIVE_NAME_PREFIX").is_none() {
        env::set_var("BOT_LIVE_NAME_PREFIX", "mc");
    }

    let evidence = PathBuf::from(
        env::var_os("MANUAL_LIVE_EVIDENCE").ok_or("MANUAL_LIVE_EVIDENCE is required")?,
    );
    if !evidence.is_absolute() {
        return Err("MANUAL_LIVE_EVIDENCE must be an absolute external evidence directory".into());
    }
    fs::create_dir_all(&evidence).map_err(|error| format!("evidence directory: {error}"))?;
    let evidence = evidence
        .canonicalize()
        .map_err(|error| format!("evidence directory: {error}"))?;

    // The caller owns throwaway isolation: accept any supplied HOME and only
    // require the copied decoded cache beneath it.
    let home = PathBuf::from(env::var_os("HOME").ok_or("throwaway HOME is required")?);
    let home = home
        .canonicalize()
        .map_err(|error| format!("HOME: {error}"))?;

    let cache = home.join(".274bot/unpack-289");
    if !cache.is_dir() {
        return Err(format!(
            "copied decoded R289 cache is missing: {}",
            cache.display()
        ));
    }
    let engine =
        PathBuf::from(env::var_os("WORLD_ENGINE_DIR").ok_or("WORLD_ENGINE_DIR is required")?);
    if !engine.is_dir() {
        return Err(format!(
            "WORLD_ENGINE_DIR is not a directory: {}",
            engine.display()
        ));
    }
    let nav_pack =
        PathBuf::from(env::var_os("WORLD_NAV_PACK").ok_or("WORLD_NAV_PACK is required")?);
    if !nav_pack.is_file() {
        return Err(format!(
            "WORLD_NAV_PACK is not a file: {}",
            nav_pack.display()
        ));
    }
    Ok(LiveInputs {
        cache,
        engine,
        nav_pack,
        evidence,
    })
}

fn prime_local_cache(cache: &Path) -> Result<(), String> {
    fs::create_dir_all(cache).map_err(|error| error.to_string())?;
    let checksums = client::client::Client::get_jag_checksums_for(
        client::Transport::Tcp,
        "127.0.0.1",
        LOCAL_ASSET_PORT,
    )
    .map_err(str::to_string)?;
    for (index, file) in [
        "title",
        "config",
        "interface",
        "media",
        "versionlist",
        "textures",
        "wordenc",
        "sounds",
    ]
    .iter()
    .enumerate()
    {
        client::client::Client::get_jag_file_for(
            client::Transport::Tcp,
            cache.to_str().ok_or("cache path is not UTF-8")?,
            "127.0.0.1",
            LOCAL_ASSET_PORT,
            file,
            index + 1,
            &checksums,
        )
        .ok_or_else(|| format!("local R289 cache archive {file} is unavailable"))?;
    }
    Ok(())
}

fn stamp() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .to_string()
}

fn player_tile(session: &Session, name: &str) -> Result<Tile, String> {
    let states = session.nav_states.lock().unwrap();
    let tile = states
        .get(name)
        .and_then(|row| row.0.local_player())
        .map(|player| player.player.actor.tile)
        .ok_or("panel player tile is unavailable")?;
    Ok(Tile {
        x: tile.x,
        z: tile.z,
        level: tile.level,
    })
}

fn reachable_destination(world: &NavWorld, here: Tile) -> Result<Tile, String> {
    let scratch = Arc::new(Mutex::new(HashMap::new()));
    for distance in [40, 32, 24, 16] {
        for (dx, dz) in [(0, distance), (distance, 0), (0, -distance), (-distance, 0)] {
            let dest = Tile {
                x: here.x + dx,
                z: here.z + dz,
                level: here.level,
            };
            if host_play::arm_walk_on(
                world,
                here,
                dest,
                FindOptions {
                    allow_wilderness: true,
                    ..FindOptions::default()
                },
                &WorldState::empty(),
                &[],
                &scratch,
                None,
            )
            .is_ok_and(|route| {
                // A nearby tile can require a long detour around the castle.
                // Keep the real walk within the unchanged 60-second await
                // budget, without transports or a deadline-only false result.
                matches!(
                    route.legs.as_slice(),
                    [nav::router::Leg::Walk { tiles }] if tiles.len() <= 48
                )
            }) {
                return Ok(dest);
            }
        }
    }
    Err("no reachable live long-walk destination near the seeded mainland tile".into())
}

fn walk_source(destination: Tile) -> String {
    traversal_source(destination, "walkTo")
}

fn traversal_source(destination: Tile, method: &str) -> String {
    format!(
        r#"import {{ Traversal }} from '../../api/walking/Traversal.js';
        export default class ManualClickProof extends LoopingBot {{
            recoveryAnchor() {{ return {{ x: {}, z: {}, level: {} }}; }}
            async loop() {{
                if (this.started) return;
                this.started = true;
                globalThis.__manual_walk_result = null;
                globalThis.__manual_terminal_count = 0;
                globalThis.__manual_walk_result = await Traversal.{}(
                    {{ x: {}, z: {}, level: {} }}, {{ radius: 0, timeoutMs: 60000 }});
                globalThis.__manual_terminal_count++;
            }}
        }}"#,
        destination.x,
        destination.z,
        destination.level,
        method,
        destination.x,
        destination.z,
        destination.level
    )
}

fn v2_walk_source(destination: Tile) -> String {
    format!(
        r#"export const apiVersion = 2;
        export function tick(api) {{
            if (!globalThis.__manual_started) {{
                globalThis.__manual_started = true;
                globalThis.__manual_walk_result = null;
                globalThis.__manual_terminal_count = 0;
                api.request({{ op: 'walk', x: {}, z: {}, level: {}, request_id: 9001 }});
            }} else if (api.snapshot.walk_outcome_request_id === 9001
                && api.snapshot.walk_outcome_cancel_reason === 'user-input'
                && !globalThis.__manual_terminal_count) {{
                globalThis.__manual_v2_reason = api.snapshot.walk_outcome_cancel_reason;
                globalThis.__manual_walk_result = !api.snapshot.walk_outcome_failed;
                globalThis.__manual_terminal_count++;
            }}
        }}"#,
        destination.x, destination.z, destination.level
    )
}

fn prepare_session(inputs: &LiveInputs, pause_owner: bool) -> Result<(Session, String), String> {
    prepare_session_n(inputs, pause_owner, 1)
}

fn prepare_session_n(
    inputs: &LiveInputs,
    pause_owner: bool,
    count: usize,
) -> Result<(Session, String), String> {
    prime_local_cache(&inputs.cache)?;
    let mut session = Session::with_instance(host_play::InstancePermit::SkipLock);
    session.persist_ui = false;
    session.ui.nav.pause_script_on_manual_walk_abort = pause_owner;
    session.configure_profile(host_play::ProfileOptions {
        profile: Some("local-289".into()),
        revision: Some("289".into()),
        host: Some("127.0.0.1".into()),
        asset_host: Some("127.0.0.1".into()),
        port: Some(LOCAL_GAME_PORT),
        http_port: Some(LOCAL_ASSET_PORT),
        engine_dir: Some(inputs.engine.clone()),
        nav_pack: Some(inputs.nav_pack.clone()),
        cache_dir: Some(inputs.cache.clone()),
        unpack_dir: Some(inputs.cache.clone()),
        ..host_play::ProfileOptions::default()
    })?;
    session.bind_profile()?;
    let mut scenario =
        scenario::get("render_smoke").ok_or("render_smoke scenario is unavailable")?;
    scenario.seed.mainland = true;
    scenario.seed.profiles.resize(count, ("test2", "test2"));
    session.live_prepare_script(scenario)?;
    session.set_capture(true);
    session.set_renderer(true);

    let deadline = Instant::now() + SCENE_WAIT;
    let name = loop {
        session.pump_status();
        if session
            .statuses
            .iter()
            .filter(|row| {
                row.ingame
                    && row.scene_state == 2
                    && player_tile(&session, &row.username)
                        .is_ok_and(|tile| (tile.x, tile.z, tile.level) == (3220, 3220, 0))
            })
            .count()
            < count
        {
            if Instant::now() >= deadline {
                return Err("panel companion scene-ready deadline".into());
            }
            std::thread::sleep(Duration::from_millis(20));
            continue;
        }
        if let Some(row) = session.statuses.iter().find(|row| {
            row.ingame
                && row.scene_state == 2
                && session
                    .nav_states
                    .lock()
                    .unwrap()
                    .get(&row.username)
                    .and_then(|state| state.0.local_player())
                    .is_some_and(|player| {
                        let tile = player.player.actor.tile;
                        (tile.x, tile.z, tile.level) == (3220, 3220, 0)
                    })
        }) {
            break row.username.clone();
        }
        if Instant::now() >= deadline {
            return Err(format!("panel scene-ready deadline: {:?}", session.error));
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    session.select(&name);
    session.set_capture(true);
    Ok((session, name))
}

fn current_probe(session: &Session, name: &str) -> Result<serde_json::Value, String> {
    let mut proof = session
        .core
        .play()
        .ok_or("Play disappeared")?
        .manual_click_live_probe(name);
    let tile = player_tile(session, name)?;
    proof["player_tile"] = serde_json::json!([tile.x, tile.z, tile.level]);
    if let Some((snapshot, _)) = session.nav_states.lock().unwrap().get(name) {
        proof["menu_entries"] = serde_json::json!(snapshot.menu_entries());
        proof["npcs"] = serde_json::json!(snapshot.npcs());
    }
    let arm = session.travellers.lock().unwrap().get(name).cloned();
    if let Some(arm) = arm {
        let arm = arm.lock().unwrap();
        proof["walk_arm"] = serde_json::json!({
            "route_generation": arm.route_generation,
            "destination": arm.queued_tile().map(|tile| [tile.x, tile.z, tile.level]),
            "active": arm.route.is_some() || arm.bank_fetch.is_some(),
            "follow_aim": arm.traveller.current_aim().map(|tile| [tile.x, tile.z, tile.level]),
        });
    }
    Ok(proof)
}

fn wait_for_armed(
    session: &mut Session,
    name: &str,
    initial: Tile,
) -> Result<serde_json::Value, String> {
    let deadline = Instant::now() + WALK_WAIT;
    loop {
        session.pump_status();
        let proof = current_probe(session, name)?;
        if player_tile(session, name).is_ok_and(|tile| tile != initial)
            && proof["nav"]["route"] == true
            && proof["nav"]["request_id"]
                .as_u64()
                .is_some_and(|id| id != 0)
        {
            return Ok(proof);
        }
        if Instant::now() >= deadline {
            return Err(format!("script route arm deadline: {proof}"));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn start_walk(session: &Session, name: &str) -> Result<(), String> {
    let here = player_tile(session, name)?;
    let world = session
        .core
        .play()
        .and_then(host_play::Play::world)
        .ok_or("live navigation world is unavailable")?;
    let destination = reachable_destination(&world, here)?;
    session
        .core
        .play()
        .ok_or("Play disappeared")?
        .script_start_load(
            name,
            walk_source(destination),
            script::LoadShape::CompatClass,
            None,
            vec![],
        )?;
    Ok(())
}

fn capture(
    session: &Session,
    dir: &Path,
    stamp: &str,
    label: &str,
    proof: &serde_json::Value,
) -> Result<(), String> {
    let pixels = session.focused_pixels().ok_or("panel CPU frame mailbox")?;
    if pixels.generation() == 0 {
        return Err("panel has not rendered a CPU frame".into());
    }
    let raw: Vec<u8> = pixels
        .snapshot()
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect();
    fs::write(dir.join(format!("{stamp}_{label}.argb")), raw).map_err(|error| error.to_string())?;
    let receipt = serde_json::json!({
        "surface": "panel headless CPU game pane",
        "width": 765,
        "height": 503,
        "frame_generation": pixels.generation(),
        "proof": proof,
    });
    fs::write(
        dir.join(format!("{stamp}_{label}.json")),
        serde_json::to_vec_pretty(&receipt).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())
}

fn send_click(session: &Session, button: i32, x: i32, y: i32) -> Result<(), String> {
    let sender = session
        .capture_tx
        .as_ref()
        .ok_or("production panel capture sender is unavailable")?;
    sender
        .send(InputEv::Move { x, y })
        .map_err(|error| error.to_string())?;
    sender
        .send(InputEv::Down { button, x, y })
        .map_err(|error| error.to_string())?;
    sender.send(InputEv::Up).map_err(|error| error.to_string())
}

fn send_minimap_click(session: &Session) -> Result<(), String> {
    send_click(session, 1, MINIMAP_CLICK.0, MINIMAP_CLICK.1)
}

fn wait_for_user_input(session: &mut Session, name: &str) -> Result<serde_json::Value, String> {
    let deadline = Instant::now() + TERMINAL_WAIT;
    loop {
        session.pump_status();
        let proof = current_probe(session, name)?;
        if proof["nav"]["reason"] == "UserInput"
            && proof["script"]["walk_result"] == false
            && proof["script"]["terminal_count"] == 1
        {
            return Ok(proof);
        }
        if Instant::now() >= deadline {
            return Err(format!("manual UserInput receipt deadline: {proof}"));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn wait_for_walk_success(session: &mut Session, name: &str) -> Result<serde_json::Value, String> {
    let deadline = Instant::now() + WALK_WAIT;
    loop {
        session.pump_status();
        let proof = current_probe(session, name)?;
        if proof["script"]["terminal_count"] == 1 {
            return Ok(proof);
        }
        if Instant::now() >= deadline {
            return Err(format!("resumed carried walk arrival deadline: {proof}"));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn wait_for_pause_carry(session: &Session, name: &str) -> Result<serde_json::Value, String> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let proof = current_probe(session, name)?;
        if proof["script"]["run_state"] == "Paused" && proof["nav"]["carry"] == true {
            return Ok(proof);
        }
        if Instant::now() >= deadline {
            return Err(format!("Pause did not retain a carried walk: {proof}"));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn assert_correlated_user_input(before: &serde_json::Value, after: &serde_json::Value) {
    assert_eq!(after["nav"]["reason"], "UserInput");
    assert_eq!(
        after["nav"]["outcome_request_id"],
        before["nav"]["request_id"]
    );
    assert_eq!(
        after["nav"]["outcome_generation"],
        before["nav"]["route_generation"]
    );
    assert_eq!(
        after["nav"]["outcome_seq"].as_u64(),
        before["nav"]["outcome_seq"].as_u64().map(|seq| seq + 1)
    );
    assert_eq!(
        after["nav"]["user_move_intent_seq"].as_u64(),
        before["nav"]["user_move_intent_seq"]
            .as_u64()
            .map(|seq| seq + 1)
    );
    assert_eq!(after["nav"]["blocked"], false);
    for field in [
        "armed",
        "route",
        "worker",
        "pending",
        "requested",
        "bank_fetch",
        "carry",
    ] {
        assert_eq!(after["nav"][field], false, "leftover {field}");
    }
}

fn prepare_evidence(
    inputs: &LiveInputs,
    label: &str,
    name: &str,
) -> Result<(String, PathBuf), String> {
    let stamp = stamp();
    let dir = inputs
        .evidence
        .join(format!("manual-click-c_{label}_{name}_{stamp}"));
    fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    Ok((stamp, dir))
}

fn run_manual_click(pause_owner: bool, recovery: bool, v2: bool) -> Result<(), String> {
    let inputs = live_inputs()?;
    let (mut session, name) = prepare_session(&inputs, pause_owner)?;
    let initial = player_tile(&session, &name)?;
    let world = session
        .core
        .play()
        .and_then(host_play::Play::world)
        .ok_or("live navigation world")?;
    let destination = reachable_destination(&world, initial)?;
    let (source, shape) = if v2 {
        (v2_walk_source(destination), script::LoadShape::NativeTick)
    } else {
        (walk_source(destination), script::LoadShape::CompatClass)
    };
    session
        .core
        .play()
        .ok_or("Play disappeared")?
        .script_start_load(&name, source, shape, None, vec![])?;
    let before = wait_for_armed(&mut session, &name, initial)?;
    let (stamp, dir) = prepare_evidence(
        &inputs,
        &format!(
            "panel-{}-{}-{}",
            if pause_owner { "on" } else { "off" },
            if recovery { "recovery" } else { "walk" },
            if v2 { "v2" } else { "compat" }
        ),
        &name,
    )?;
    capture(&session, &dir, &stamp, "01-armed", &before)?;

    // Both controls enter via the production capture channel, not a host op.
    for (label, button, x, y) in [
        ("02-tab-control", 1, 600, 190),
        ("02-right-click-control", 2, 256, 180),
    ] {
        send_click(&session, button, x, y)?;
        let until = Instant::now() + Duration::from_millis(200);
        while Instant::now() < until {
            session.pump_status();
            std::thread::sleep(Duration::from_millis(20));
        }
        let controls = current_probe(&session, &name)?;
        assert_eq!(
            controls["nav"]["user_move_intent_seq"],
            before["nav"]["user_move_intent_seq"]
        );
        assert_eq!(controls["nav"]["request_id"], before["nav"]["request_id"]);
        assert_eq!(
            controls["nav"]["route"], true,
            "unrelated input must keep following"
        );
        capture(&session, &dir, &stamp, label, &controls)?;
    }

    if recovery {
        let deadline = Instant::now() + TERMINAL_WAIT;
        let mut last_watchdog = String::new();
        let mut transition = 0;
        loop {
            let here = player_tile(&session, &name)?;
            session
                .core
                .play()
                .ok_or("Play disappeared")?
                .manual_click_live_age_gameplay(
                    &name,
                    api::snapshot::WorldTile {
                        x: here.x,
                        z: here.z,
                        level: here.level,
                    },
                )?;
            session.pump_status();
            let proof = current_probe(&session, &name)?;
            let state = format!(
                "{}/{}",
                proof["script"]["watchdog_state"], proof["script"]["runtime_generation"],
            );
            if state != last_watchdog {
                transition += 1;
                capture(
                    &session,
                    &dir,
                    &stamp,
                    &format!("03-watchdog-{transition}"),
                    &proof,
                )?;
                println!("recovery transition account={name} proof={proof}");
                last_watchdog = state;
            }
            if proof["script"]["recovering_anchor"].is_array()
                && proof["nav"]["route"] == true
                && proof["nav"]["request_id"] == 0
            {
                capture(&session, &dir, &stamp, "03-watchdog-recovery", &proof)?;
                break;
            }
            if Instant::now() >= deadline {
                return Err(format!("real recovery arm deadline: {proof}"));
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    // With the world menu still open, this off-row click also walks on minimap.
    send_minimap_click(&session)?;
    if pause_owner {
        let deadline = Instant::now() + TERMINAL_WAIT;
        loop {
            session.pump_status();
            let proof = current_probe(&session, &name)?;
            if proof["script"]["run_state"] == "Paused" && proof["nav"]["reason"] == "UserInput" {
                assert_correlated_user_input(&before, &proof);
                assert!(proof["script"]["recovering_anchor"].is_null());
                capture(&session, &dir, &stamp, "04-takeover-paused", &proof)?;
                break;
            }
            if Instant::now() >= deadline {
                return Err(format!("owner Pause deadline: {proof}"));
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        session
            .core
            .play()
            .ok_or("Play disappeared")?
            .script_resume(&name);
    }
    let after = wait_for_user_input(&mut session, &name)?;
    assert_correlated_user_input(&before, &after);
    assert_eq!(after["script"]["run_state"], "Running");
    assert!(after["script"]["recovering_anchor"].is_null());
    if v2 {
        assert_eq!(after["script"]["v2_reason"], "user-input");
    }
    capture(&session, &dir, &stamp, "05-terminal", &after)?;
    let stable_until = Instant::now() + SETTLE_WAIT;
    while Instant::now() < stable_until {
        session.pump_status();
        std::thread::sleep(Duration::from_millis(20));
    }
    let settled = current_probe(&session, &name)?;
    assert_eq!(
        settled["nav"], after["nav"],
        "no old automatic follow after takeover"
    );
    assert_eq!(
        settled["script"]["runtime_generation"],
        before["script"]["runtime_generation"]
    );
    assert_eq!(settled["script"]["terminal_count"], 1);
    assert_eq!(settled["script"]["rearm_pending"], false);
    assert!(settled["script"]["recovering_anchor"].is_null());
    capture(&session, &dir, &stamp, "06-settled", &settled)?;
    println!("PASS panel pause={pause_owner} recovery={recovery} v2={v2} account={name} before={before} after={after} settled={settled}");
    if let Some(play) = session.core.play() {
        play.script_stop(&name);
    }
    session.core.set_play(None);
    Ok(())
}

fn hover_frame(session: &mut Session, x: i32, y: i32) -> Result<(), String> {
    let frame = session
        .focused_pixels()
        .ok_or("panel CPU frame mailbox")?
        .generation();
    session
        .capture_tx
        .as_ref()
        .ok_or("panel capture sender")?
        .send(InputEv::Move { x, y })
        .map_err(|error| error.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(5);
    while session
        .focused_pixels()
        .is_none_or(|pixels| pixels.generation() < frame + 2)
    {
        session.pump_status();
        if Instant::now() >= deadline {
            return Err("hover paint deadline".into());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    session.pump_status();
    Ok(())
}

fn target_menu_matches(proof: &serde_json::Value, target: &str) -> bool {
    let Some(entries) = proof["menu_entries"].as_array() else {
        return false;
    };
    if target == "npc" {
        proof["npcs"].as_array().into_iter().flatten().any(|npc| {
            npc["name"].as_str().is_some_and(|name| {
                !name.is_empty()
                    && entries
                        .iter()
                        .any(|entry| entry.as_str().is_some_and(|text| text.contains(name)))
            })
        })
    } else {
        entries
            .iter()
            .any(|entry| entry.as_str().is_some_and(|text| text.contains("Bush")))
    }
}

fn scene_candidate_points(proof: &serde_json::Value, target: &str) -> Vec<(i32, i32)> {
    if target != "npc" {
        return [180, 220, 140, 260, 100, 300, 60]
            .into_iter()
            .flat_map(|y| {
                [128, 208, 88, 168, 248, 328, 408, 488, 48, 288, 368, 448]
                    .into_iter()
                    .map(move |x| (x, y))
            })
            .collect();
    }
    let mut boxes = Vec::new();
    for bounds in proof["script"]["npc_boxes"]
        .as_array()
        .into_iter()
        .flatten()
    {
        let mut rect: Option<(i32, i32, i32, i32)> = None;
        for point in bounds["points"].as_array().into_iter().flatten() {
            let (Some(x), Some(y)) = (point["x"].as_i64(), point["y"].as_i64()) else {
                continue;
            };
            let (x, y) = (x as i32, y as i32);
            rect = Some(match rect {
                Some((x0, x1, y0, y1)) => (x0.min(x), x1.max(x), y0.min(y), y1.max(y)),
                None => (x, x, y, y),
            });
        }
        if let Some((x0, x1, y0, y1)) = rect {
            let (x0, x1, y0, y1) = (x0.max(4), x1.min(515), y0.max(4), y1.min(337));
            if x0 < x1 && y0 < y1 {
                boxes.push((x0, x1, y0, y1));
            }
        }
    }
    boxes.sort_by_key(|&(x0, x1, y0, y1)| std::cmp::Reverse((x1 - x0) * (y1 - y0)));
    let mut points = Vec::new();
    for (x0, x1, y0, y1) in boxes {
        // Projection boxes contain empty space as well as model triangles.
        // Try body points, prioritizing the largest visible actor.
        for dy in [1, 2, 3] {
            for dx in [1, 2, 3] {
                points.push((x0 + (x1 - x0) * dx / 4, y0 + (y1 - y0) * dy / 4));
            }
        }
    }
    points
}

fn run_scene_target_capture(target: &str) -> Result<(), String> {
    let inputs = live_inputs()?;
    let (mut session, name) = prepare_session(&inputs, false)?;
    let initial = player_tile(&session, &name)?;
    start_walk(&session, &name)?;
    let before = wait_for_armed(&mut session, &name, initial)?;
    let (stamp, dir) = prepare_evidence(&inputs, &format!("panel-{target}"), &name)?;
    capture(&session, &dir, &stamp, "01-armed", &before)?;
    let mut selected = None;
    let deadline = Instant::now() + Duration::from_secs(25);
    for attempt in 0..32 {
        let proof = current_probe(&session, &name)?;
        let candidates = scene_candidate_points(&proof, target);
        let Some(&(x, y)) = candidates.get(attempt % candidates.len().max(1)) else {
            return Err(format!(
                "no projected {target} candidate in the live scene: {proof}"
            ));
        };
        hover_frame(&mut session, x, y)?;
        let hovered = current_probe(&session, &name)?;
        assert_eq!(hovered["nav"]["route"], true, "hover must not cancel");
        assert_eq!(
            hovered["nav"]["user_move_intent_seq"],
            before["nav"]["user_move_intent_seq"]
        );
        // Right-click freezes the actual model/loc pick. Read the open menu
        // after a server observation, not stale rows from the previous hover.
        send_click(&session, 2, x, y)?;
        hover_frame(&mut session, x, y)?;
        std::thread::sleep(Duration::from_millis(650));
        session.pump_status();
        let menu = current_probe(&session, &name)?;
        assert_eq!(menu["nav"]["route"], true, "right-click must not cancel");
        assert_eq!(
            menu["nav"]["user_move_intent_seq"],
            before["nav"]["user_move_intent_seq"]
        );
        if target_menu_matches(&menu, target) {
            selected = Some((x, y, menu));
            break;
        }
        // Mouse-out dismisses a rejected world menu without a human Down.
        hover_frame(&mut session, 750, 480)?;
        if Instant::now() >= deadline {
            break;
        }
    }
    let (x, y, menu) = selected.ok_or_else(|| {
        format!(
            "no real {target} world menu in the live scene: {}",
            current_probe(&session, &name).unwrap()
        )
    })?;
    assert_eq!(menu["nav"]["route"], true, "right-click must not cancel");
    capture(&session, &dir, &stamp, "03-target-world-menu", &menu)?;
    let entries = menu["menu_entries"].as_array().ok_or("open menu rows")?;
    let count = entries.len() as i32;
    let row = entries
        .iter()
        .position(|entry| {
            entry.as_str().is_some_and(|text| match target {
                "cancel" => text == "Cancel",
                "npc" => {
                    text.starts_with("Examine")
                        && menu["npcs"].as_array().into_iter().flatten().any(|npc| {
                            npc["name"]
                                .as_str()
                                .is_some_and(|name| !name.is_empty() && text.contains(name))
                        })
                }
                _ => text.starts_with("Examine") && text.contains("Bush"),
            })
        })
        .ok_or("selected actor/loc/conservative menu row")? as i32;
    // These applet coordinates follow the client's 334px viewport placement,
    // including its 21px clamp and 31px first-row baseline.
    let top = (y - 4).min(334 - (count * 15 + 21));
    send_click(&session, 1, x, top + 4 + 31 + (count - 1 - row) * 15)?;
    let after = wait_for_user_input(&mut session, &name)?;
    assert_correlated_user_input(&before, &after);
    assert_eq!(after["script"]["run_state"], "Running");
    capture(&session, &dir, &stamp, "04-user-input", &after)?;
    let stable_until = Instant::now() + SETTLE_WAIT;
    while Instant::now() < stable_until {
        session.pump_status();
        std::thread::sleep(Duration::from_millis(20));
    }
    let settled = current_probe(&session, &name)?;
    assert_eq!(settled["nav"], after["nav"], "no old target walk replay");
    assert_eq!(settled["script"]["terminal_count"], 1);
    assert_eq!(settled["script"]["rearm_pending"], false);
    capture(&session, &dir, &stamp, "05-stable", &settled)?;
    println!("PASS scene target={target} account={name} before={before} menu={menu} after={after} settled={settled}");
    session.core.set_play(None);
    Ok(())
}

#[test]
#[ignore = "requires LIVE=1, local R289 engine/nav pack and a throwaway HOME"]
fn live_manual_click_panel_npc_loc_and_cancel() {
    for target in ["npc", "loc", "cancel"] {
        run_scene_target_capture(target).unwrap();
    }
}

fn run_pause_click_resume() -> Result<(), String> {
    let inputs = live_inputs()?;
    let (mut session, name) = prepare_session(&inputs, false)?;
    let (stamp, dir) = prepare_evidence(&inputs, "panel-f2-pause-carry", &name)?;

    // M5 control: a click while operator-paused increments host intent but
    // does not cancel the carried compat walk. No snapshot is posted by this
    // harness while Paused; the normal slot pump owns publication.
    let initial = player_tile(&session, &name)?;
    start_walk(&session, &name)?;
    let before_pause_click = wait_for_armed(&mut session, &name, initial)?;
    capture(
        &session,
        &dir,
        &stamp,
        "01-walking-before-pause",
        &before_pause_click,
    )?;
    session
        .core
        .play()
        .ok_or("Play disappeared")?
        .script_pause(&name);
    let paused = wait_for_pause_carry(&session, &name)?;
    assert_eq!(paused["script"]["walk_result"], serde_json::Value::Null);
    assert_eq!(paused["script"]["terminal_count"], 0);
    capture(&session, &dir, &stamp, "02-paused-carry", &paused)?;

    send_minimap_click(&session)?;
    let deadline = Instant::now() + TERMINAL_WAIT;
    let paused_click = loop {
        session.pump_status();
        let proof = current_probe(&session, &name)?;
        if proof["nav"]["user_move_intent_seq"].as_u64()
            == paused["nav"]["user_move_intent_seq"]
                .as_u64()
                .map(|seq| seq + 1)
        {
            break proof;
        }
        if Instant::now() >= deadline {
            return Err(format!("paused manual-input observation deadline: {proof}"));
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(paused_click["script"]["run_state"], "Paused");
    assert_eq!(
        paused_click["script"]["walk_result"],
        serde_json::Value::Null
    );
    assert_eq!(paused_click["script"]["terminal_count"], 0);
    assert_eq!(
        paused_click["nav"]["carry"], true,
        "paused click dropped the carry"
    );
    assert_eq!(paused_click["nav"]["route"], false);
    assert_eq!(
        paused_click["nav"]["outcome_seq"],
        paused["nav"]["outcome_seq"]
    );
    assert_ne!(paused_click["nav"]["reason"], "UserInput");
    capture(
        &session,
        &dir,
        &stamp,
        "03-click-while-paused",
        &paused_click,
    )?;

    session
        .core
        .play()
        .ok_or("Play disappeared")?
        .script_resume(&name);
    let arrived = wait_for_walk_success(&mut session, &name)?;
    capture(
        &session,
        &dir,
        &stamp,
        "04-resumed-carry-terminal",
        &arrived,
    )?;
    println!("F2 resumed terminal account={name} proof={arrived}");
    assert_eq!(
        arrived["script"]["walk_result"], true,
        "paused-period carry did not arrive: {arrived}"
    );
    assert_eq!(arrived["script"]["terminal_count"], 1);
    assert_ne!(arrived["nav"]["reason"], "UserInput");
    assert_eq!(arrived["nav"]["carry"], false);

    let stable_until = Instant::now() + SETTLE_WAIT;
    while Instant::now() < stable_until {
        session.pump_status();
        std::thread::sleep(Duration::from_millis(20));
    }
    let arrived_settled = current_probe(&session, &name)?;
    assert_eq!(arrived_settled["nav"], arrived["nav"]);
    assert_eq!(arrived_settled["script"]["terminal_count"], 1);
    capture(
        &session,
        &dir,
        &stamp,
        "05-arrival-settled",
        &arrived_settled,
    )?;

    // Grok's resume-then-click window: enqueue Resume, then send the actual
    // panel capture click without an intervening UI pump. The carried walk's
    // original request must receive one correlated UserInput terminal.
    if let Some(play) = session.core.play() {
        play.script_stop(&name);
    }
    let second_initial = player_tile(&session, &name)?;
    start_walk(&session, &name)?;
    let before_resume_click = wait_for_armed(&mut session, &name, second_initial)?;
    capture(
        &session,
        &dir,
        &stamp,
        "06-second-walk-armed",
        &before_resume_click,
    )?;
    session
        .core
        .play()
        .ok_or("Play disappeared")?
        .script_pause(&name);
    let second_paused = wait_for_pause_carry(&session, &name)?;
    assert_eq!(second_paused["nav"]["carry"], true);
    session
        .core
        .play()
        .ok_or("Play disappeared")?
        .script_resume(&name);
    assert_eq!(
        session
            .core
            .play()
            .ok_or("Play disappeared")?
            .script_state(&name),
        script::RunState::Running
    );
    send_minimap_click(&session)?;
    let after_resume_click = wait_for_user_input(&mut session, &name)?;
    assert_correlated_user_input(&before_resume_click, &after_resume_click);
    capture(
        &session,
        &dir,
        &stamp,
        "07-click-after-resume-receipt",
        &after_resume_click,
    )?;

    let stable_until = Instant::now() + SETTLE_WAIT;
    while Instant::now() < stable_until {
        session.pump_status();
        std::thread::sleep(Duration::from_millis(20));
    }
    let after_resume_settled = current_probe(&session, &name)?;
    assert_eq!(after_resume_settled["nav"], after_resume_click["nav"]);
    assert_eq!(after_resume_settled["script"]["terminal_count"], 1);
    capture(
        &session,
        &dir,
        &stamp,
        "08-click-after-resume-settled",
        &after_resume_settled,
    )?;
    println!(
        "PASS panel F2 account={name} paused_click={paused_click} arrived={arrived_settled} resume_click={after_resume_click} settled={after_resume_settled}"
    );
    if let Some(play) = session.core.play() {
        play.script_stop(&name);
    }
    session.core.set_play(None);
    Ok(())
}

#[test]
#[ignore = "requires LIVE=1, local R289 engine/nav pack and a throwaway HOME"]
fn live_manual_click_panel_cpu() {
    run_manual_click(false, false, false).unwrap();
}

#[test]
#[ignore = "requires LIVE=1, local R289 engine/nav pack and a throwaway HOME"]
fn live_pause_manual_click_resume_carry() {
    run_pause_click_resume().unwrap();
}

#[test]
#[ignore = "requires LIVE=1, local R289 engine/nav pack and a throwaway HOME"]
fn live_manual_click_panel_pause_on() {
    run_manual_click(true, false, false).unwrap();
}

#[test]
#[ignore = "requires LIVE=1, local R289 engine/nav pack and a throwaway HOME"]
fn live_manual_click_panel_recovery_pause_resume() {
    run_manual_click(true, true, false).unwrap();
}

#[test]
#[ignore = "requires LIVE=1, local R289 engine/nav pack and a throwaway HOME"]
fn live_manual_click_panel_recovery_pause_off() {
    run_manual_click(false, true, false).unwrap();
}

#[test]
#[ignore = "requires LIVE=1, local R289 engine/nav pack and a throwaway HOME"]
fn live_manual_click_panel_v2_pause_off() {
    run_manual_click(false, false, true).unwrap();
}

fn run_group_capture() -> Result<(), String> {
    let inputs = live_inputs()?;
    let (mut session, name) = prepare_session_n(&inputs, true, 2)?;
    let peer = session
        .statuses
        .iter()
        .find(|row| row.username != name)
        .ok_or("missing ready group companion")?
        .username
        .clone();
    let world = session
        .core
        .play()
        .and_then(host_play::Play::world)
        .ok_or("live navigation world")?;
    let (stamp, dir) = prepare_evidence(&inputs, "panel-group", &name)?;
    // The seeded fountain tile is blocked in the map picker. Complete a
    // short real script walk to its existing normalized standable selection
    // before either group arm exists; no teleport or snapshot is fabricated.
    for member in [&name, &peer] {
        session.select(member);
        let here = player_tile(&session, member)?;
        let destination = session
            .select_picker_tile(&world, here)
            .ok_or("standable group fixture tile")?;
        session
            .core
            .play()
            .ok_or("Play disappeared")?
            .script_start_load(
                member,
                walk_source(destination),
                script::LoadShape::CompatClass,
                None,
                vec![],
            )?;
        let deadline = Instant::now() + WALK_WAIT;
        loop {
            session.pump_status();
            let proof = current_probe(&session, member)?;
            if proof["script"]["walk_result"] == true
                && player_tile(&session, member)? == destination
            {
                capture(
                    &session,
                    &dir,
                    &stamp,
                    &format!("00-{member}-standable"),
                    &proof,
                )?;
                break;
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "group fixture walk did not arrive at {destination:?}: {proof}"
                ));
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        session
            .core
            .play()
            .ok_or("Play disappeared")?
            .script_stop(member);
    }
    session.select(&name);
    for (trial, x, y) in [
        ("floor", 256, 180),
        ("minimap", MINIMAP_CLICK.0, MINIMAP_CLICK.1),
    ] {
        let before_here = player_tile(&session, &name)?;
        let peer_here = player_tile(&session, &peer)?;
        let destination = reachable_destination(&world, peer_here)?;
        session.refresh_walk_send();
        session.walk_send_all_eligible();
        let selected = session
            .select_picker_tile(&world, destination)
            .ok_or("group destination has no walkable map selection")?;
        assert!(
            session.confirm_picker_group_walk(&world),
            "group arm: {:?}",
            session.error
        );
        let before = current_probe(&session, &name)?;
        let peer_before = current_probe(&session, &peer)?;
        assert_eq!(before["walk_arm"]["active"], true);
        assert_eq!(peer_before["walk_arm"]["active"], true);
        let selected = serde_json::json!([selected.x, selected.z, selected.level]);
        assert_eq!(before["walk_arm"]["destination"], selected);
        assert_eq!(peer_before["walk_arm"]["destination"], selected);
        capture(
            &session,
            &dir,
            &stamp,
            &format!("{trial}-01-group-armed"),
            &serde_json::json!({"focused": before, "peer": peer_before}),
        )?;
        let control_until = Instant::now() + Duration::from_secs(2);
        while Instant::now() < control_until {
            session.pump_status();
            std::thread::sleep(Duration::from_millis(20));
        }
        let control = current_probe(&session, &name)?;
        let peer_control = current_probe(&session, &peer)?;
        assert_eq!(
            control["nav"]["user_move_intent_seq"],
            before["nav"]["user_move_intent_seq"]
        );
        assert_eq!(
            peer_control["nav"]["user_move_intent_seq"],
            peer_before["nav"]["user_move_intent_seq"]
        );
        capture(
            &session,
            &dir,
            &stamp,
            &format!("{trial}-01-no-click-control"),
            &serde_json::json!({"focused": control, "peer": peer_control}),
        )?;
        println!("group no-click control trial={trial} focused={control} peer={peer_control}");
        send_click(&session, 1, x, y)?;
        let deadline = Instant::now() + TERMINAL_WAIT;
        let after = loop {
            session.pump_status();
            let proof = current_probe(&session, &name)?;
            if proof["walk_arm"]["active"] == false {
                break proof;
            }
            if Instant::now() >= deadline {
                return Err(format!("group {trial} takeover deadline: {proof}"));
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        let peer_after = current_probe(&session, &peer)?;
        assert_eq!(
            after["walk_arm"]["route_generation"].as_u64(),
            before["walk_arm"]["route_generation"]
                .as_u64()
                .map(|generation| generation + 1)
        );
        for field in ["route_generation", "destination", "active", "follow_aim"] {
            assert_eq!(
                peer_after["walk_arm"][field], peer_control["walk_arm"][field],
                "one member's gesture must not change the companion's {field}"
            );
        }
        assert_eq!(peer_after["nav"], peer_control["nav"]);
        assert_ne!(peer_after["nav"]["reason"], "UserInput");
        capture(
            &session,
            &dir,
            &stamp,
            &format!("{trial}-02-member-cancelled"),
            &serde_json::json!({"focused": after, "peer": peer_after}),
        )?;
        let stable_until = Instant::now() + Duration::from_secs(2);
        while Instant::now() < stable_until {
            session.pump_status();
            std::thread::sleep(Duration::from_millis(20));
        }
        let settled = current_probe(&session, &name)?;
        let peer_settled = current_probe(&session, &peer)?;
        println!("group isolation trial={trial} focused={settled} peer={peer_settled}");
        assert_eq!(
            settled["walk_arm"], after["walk_arm"],
            "cancelled group member must not rearm"
        );
        assert_eq!(
            peer_settled["walk_arm"], peer_control["walk_arm"],
            "the companion retains its active arm, generation, destination and aim"
        );
        assert_eq!(peer_settled["nav"], peer_control["nav"]);
        assert_ne!(peer_settled["nav"]["reason"], "UserInput");
        if trial == "minimap" {
            assert_ne!(
                player_tile(&session, &name)?,
                before_here,
                "human minimap movement is retained"
            );
        }
        capture(
            &session,
            &dir,
            &stamp,
            &format!("{trial}-03-human-and-peer-isolation"),
            &serde_json::json!({"focused": settled, "peer": peer_settled}),
        )?;
        println!("PASS group trial={trial} focused={name} peer={peer} before={before} after={after} settled={settled} peer_settled={peer_settled}");
    }
    session.core.set_play(None);
    Ok(())
}

#[test]
#[ignore = "requires LIVE=1, two local R289 accounts/nav pack and a throwaway HOME"]
fn live_manual_click_panel_group_capture() {
    run_group_capture().unwrap();
}

#[test]
#[ignore = "requires LIVE=1, local R289 engine/nav pack and a throwaway HOME"]
fn live_manual_click_panel_route_less_resilient() {
    let inputs = live_inputs().unwrap();
    let (mut session, name) = prepare_session(&inputs, false).unwrap();
    let here = player_tile(&session, &name).unwrap();
    let world = session
        .core
        .play()
        .and_then(host_play::Play::world)
        .unwrap();
    // A real blocked terrain target makes the normal baked → scene ladder
    // enter a live walking phase with no host route.
    let destination = (12..=24)
        .flat_map(|distance| [(0, distance), (distance, 0), (0, -distance), (-distance, 0)])
        .map(|(dx, dz)| Tile {
            x: here.x + dx,
            z: here.z + dz,
            level: here.level,
        })
        .find(|tile| {
            !world.collision.walkable(api::snapshot::WorldTile {
                x: tile.x,
                z: tile.z,
                level: tile.level,
            })
        })
        .expect("a blocked terrain tile near the seeded scene");
    session
        .core
        .play()
        .unwrap()
        .script_start_load(
            &name,
            traversal_source(destination, "walkResilient"),
            script::LoadShape::CompatClass,
            None,
            vec![],
        )
        .unwrap();
    let (stamp, dir) = prepare_evidence(&inputs, "panel-route-less-resilient", &name).unwrap();
    let deadline = Instant::now() + TERMINAL_WAIT;
    let before = loop {
        session.pump_status();
        let proof = current_probe(&session, &name).unwrap();
        if proof["script"]["live_walking_operation"] == true
            && proof["script"]["terminal_count"] == 0
            && proof["nav"]["failed"] == true
            && proof["nav"]["route"] == false
            && proof["nav"]["worker"] == false
            && proof["nav"]["pending"] == false
        {
            break proof;
        }
        assert!(
            Instant::now() < deadline,
            "route-less ladder deadline: {proof}"
        );
        std::thread::sleep(Duration::from_millis(20));
    };
    capture(
        &session,
        &dir,
        &stamp,
        "01-live-family-no-host-route",
        &before,
    )
    .unwrap();
    send_minimap_click(&session).unwrap();
    let deadline = Instant::now() + TERMINAL_WAIT;
    let after = loop {
        session.pump_status();
        let proof = current_probe(&session, &name).unwrap();
        if proof["script"]["walk_result"] == false && proof["script"]["terminal_count"] == 1 {
            break proof;
        }
        assert!(
            Instant::now() < deadline,
            "route-less takeover deadline: {proof}"
        );
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(after["script"]["run_state"], "Running");
    assert_eq!(
        after["nav"]["takeover_watermark"],
        after["nav"]["outcome_seq"]
    );
    assert_eq!(
        after["nav"]["user_move_intent_seq"].as_u64(),
        before["nav"]["user_move_intent_seq"]
            .as_u64()
            .map(|seq| seq + 1)
    );
    assert_eq!(
        after["nav"]["outcome_request_id"], before["nav"]["outcome_request_id"],
        "no fabricated route-less receipt"
    );
    capture(
        &session,
        &dir,
        &stamp,
        "02-route-less-false-terminal",
        &after,
    )
    .unwrap();
    let stable_until = Instant::now() + SETTLE_WAIT;
    while Instant::now() < stable_until {
        session.pump_status();
        std::thread::sleep(Duration::from_millis(20));
    }
    let settled = current_probe(&session, &name).unwrap();
    assert_eq!(
        settled["nav"], after["nav"],
        "no automatic resilient recovery or next leg"
    );
    assert_eq!(settled["script"]["terminal_count"], 1);
    assert_eq!(
        settled["script"]["runtime_generation"],
        before["script"]["runtime_generation"]
    );
    capture(&session, &dir, &stamp, "03-route-less-stable", &settled).unwrap();
    println!(
        "PASS route-less resilient account={name} before={before} after={after} settled={settled}"
    );
    session.core.set_play(None);
}
