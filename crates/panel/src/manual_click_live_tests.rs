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
    assert_eq!(env::var("BOT_LIVE_NAME_PREFIX").as_deref(), Ok("mc"));

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

    let home = PathBuf::from(env::var_os("HOME").ok_or("throwaway HOME is required")?);
    let home = home
        .canonicalize()
        .map_err(|error| format!("HOME: {error}"))?;
    if home == evidence || !home.starts_with(&evidence) {
        return Err("HOME must be a disposable directory inside MANUAL_LIVE_EVIDENCE".into());
    }
    let marker = fs::read_to_string(home.join(".manual-click-b-throwaway-home"))
        .map_err(|error| format!("throwaway HOME marker: {error}"))?;
    if marker.trim() != "MANUAL-CLICK-B-2" {
        return Err("HOME marker must contain MANUAL-CLICK-B-2".into());
    }
    let temp = PathBuf::from(env::var_os("TMPDIR").ok_or("throwaway TMPDIR is required")?);
    let temp = temp
        .canonicalize()
        .map_err(|error| format!("TMPDIR: {error}"))?;
    if !temp.starts_with(&home) {
        return Err("TMPDIR must be inside the throwaway HOME".into());
    }

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
    format!(
        r#"import {{ Traversal }} from '../../api/walking/Traversal.js';
        export default class ManualClickProof extends LoopingBot {{
            async loop() {{
                if (this.started) return;
                this.started = true;
                globalThis.__manual_walk_result = null;
                globalThis.__manual_terminal_count = 0;
                globalThis.__manual_walk_result = await Traversal.walkTo(
                    {{ x: {}, z: {}, level: {} }}, {{ radius: 0, timeoutMs: 60000 }});
                globalThis.__manual_terminal_count++;
            }}
        }}"#,
        destination.x, destination.z, destination.level
    )
}

fn prepare_session(inputs: &LiveInputs) -> Result<(Session, String), String> {
    prime_local_cache(&inputs.cache)?;
    let mut session = Session::with_instance(host_play::InstancePermit::SkipLock);
    session.persist_ui = false;
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
    session.live_prepare_script(scenario)?;
    session.set_capture(true);
    session.set_renderer(true);

    let deadline = Instant::now() + SCENE_WAIT;
    let name = loop {
        session.pump_status();
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
        "request_id": "MANUAL-CLICK-B-2",
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

fn send_minimap_click(session: &Session) -> Result<(), String> {
    let sender = session
        .capture_tx
        .as_ref()
        .ok_or("production panel capture sender is unavailable")?;
    let (x, y) = MINIMAP_CLICK;
    sender
        .send(InputEv::Move { x, y })
        .map_err(|error| error.to_string())?;
    sender
        .send(InputEv::Down { button: 1, x, y })
        .map_err(|error| error.to_string())?;
    sender.send(InputEv::Up).map_err(|error| error.to_string())
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
        .join(format!("manual-click-b_{label}_{name}_{stamp}"));
    fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    Ok((stamp, dir))
}

fn run_manual_click() -> Result<(), String> {
    let inputs = live_inputs()?;
    let (mut session, name) = prepare_session(&inputs)?;
    let initial = player_tile(&session, &name)?;
    let world = session
        .core
        .play()
        .and_then(host_play::Play::world)
        .ok_or("live navigation world is unavailable")?;
    let destination = reachable_destination(&world, initial)?;
    session
        .core
        .play()
        .ok_or("Play disappeared")?
        .script_start_load(
            &name,
            walk_source(destination),
            script::LoadShape::CompatClass,
            None,
            vec![],
        )?;
    let before = wait_for_armed(&mut session, &name, initial)?;
    let (stamp, dir) = prepare_evidence(&inputs, "panel-cpu", &name)?;
    capture(&session, &dir, &stamp, "01-armed", &before)?;

    send_minimap_click(&session)?;
    let after = wait_for_user_input(&mut session, &name)?;
    assert_correlated_user_input(&before, &after);
    capture(&session, &dir, &stamp, "02-takeover", &after)?;

    let stable_until = Instant::now() + SETTLE_WAIT;
    while Instant::now() < stable_until {
        session.pump_status();
        std::thread::sleep(Duration::from_millis(20));
    }
    let settled = current_probe(&session, &name)?;
    assert_eq!(
        settled["nav"], after["nav"],
        "stale follow changed after takeover"
    );
    assert_eq!(settled["script"]["terminal_count"], 1);
    capture(&session, &dir, &stamp, "03-settled", &settled)?;
    println!("PASS panel CPU account={name} before={before} after={after} settled={settled}");
    if let Some(play) = session.core.play() {
        play.script_stop(&name);
    }
    session.core.set_play(None);
    Ok(())
}

fn run_pause_click_resume() -> Result<(), String> {
    let inputs = live_inputs()?;
    let (mut session, name) = prepare_session(&inputs)?;
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
#[ignore = "requires LIVE=1, local R289 engine/nav pack and a marked throwaway HOME"]
fn live_manual_click_panel_cpu() {
    run_manual_click().unwrap();
}

#[test]
#[ignore = "requires LIVE=1, local R289 engine/nav pack and a marked throwaway HOME"]
fn live_pause_manual_click_resume_carry() {
    run_pause_click_resume().unwrap();
}
