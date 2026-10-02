//! Persistent ignored LIVE regressions for the production TUI Manual-walk path.
//!
//! These tests intentionally run inside `bin` so input is routed through
//! `TuiApp::on_key` and the same `dispatch` used by `tui-play`.

use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use nav::router::FindOptions;
use nav::tile::Tile;
use nav::world::NavWorld;
use nav::WorldState;
use ratatui::backend::TestBackend;
use ratatui::Terminal;

use super::{dispatch, temp_live_vault, TuiApp, TuiSession};

const LOCAL_GAME_PORT: u16 = 45_594;
const LOCAL_ASSET_PORT: u16 = 2_080;
const LIVE_WAIT: Duration = Duration::from_secs(150);
const WALK_WAIT: Duration = Duration::from_secs(40);
const TERMINAL_WAIT: Duration = Duration::from_secs(20);
const SETTLE_WAIT: Duration = Duration::from_secs(5);

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
            .is_ok()
            {
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
            recoveryAnchor() {{ return {{ x: {}, z: {}, level: {} }}; }}
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
        destination.x,
        destination.z,
        destination.level,
        destination.x,
        destination.z,
        destination.level
    )
}

fn run_manual_click(cols: u16, rows: u16, pause_owner: bool, recovery: bool) -> Result<(), String> {
    let inputs = live_inputs()?;
    prime_local_cache(&inputs.cache)?;
    let profile = host_play::ProfileOptions {
        profile: Some("local-289".into()),
        revision: Some("289".into()),
        host: Some("127.0.0.1".into()),
        asset_host: Some("127.0.0.1".into()),
        port: Some(LOCAL_GAME_PORT),
        http_port: Some(LOCAL_ASSET_PORT),
        engine_dir: Some(inputs.engine),
        nav_pack: Some(inputs.nav_pack),
        cache_dir: Some(inputs.cache.clone()),
        unpack_dir: Some(inputs.cache),
        ..host_play::ProfileOptions::default()
    }
    .resolve(None)?
    .bind()?;
    let template = host_play::SharedClientTemplate::load(profile)?;
    let names = host_play::mint_live_names(1);
    let credentials = host_play::mint_live_entries(&names);
    let name = names.first().ok_or("no minted live account")?.clone();
    let passphrase = host_play::live_vault_passphrase();
    let vault_path = temp_live_vault(&credentials, &passphrase);

    let mut session = TuiSession::new_bound(template, host_play::InstancePermit::SkipLock);
    session.options.mainland = true;
    session.persist_ui = false;
    session.unlock_at(&vault_path, &passphrase)?;
    session.set_pause_script_on_manual_walk_abort(pause_owner);
    if !session.load_and_login(&name) {
        return Err(format!("TUI load/login failed: {:?}", session.error));
    }
    session.focus(&name);
    let mut app = TuiApp::new("manual-click CPU live");
    app.pause_script_on_manual_walk_abort = pause_owner;
    let deadline = Instant::now() + LIVE_WAIT;
    loop {
        session.pump(&mut app);
        let scene_ready = session.core.play().is_some_and(|play| {
            play.statuses()
                .iter()
                .any(|status| status.username == name && status.ingame && status.scene_state == 2)
        });
        let seeded = app
            .here
            .is_some_and(|tile| (tile.x, tile.z, tile.level) == (3220, 3220, 0));
        if scene_ready && seeded {
            break;
        }
        if Instant::now() >= deadline {
            return Err(format!("TUI scene-ready deadline: {:?}", session.error));
        }
        std::thread::sleep(Duration::from_millis(20));
    }

    let here = app.here.ok_or("TUI player tile is unavailable")?;
    let route_from = nav::tile::Tile {
        x: here.x,
        z: here.z,
        level: here.level,
    };
    let world = session
        .core
        .play()
        .and_then(host_play::Play::world)
        .ok_or("live navigation world is unavailable")?;
    let destination = reachable_destination(&world, route_from)?;
    let play = session.core.play().ok_or("Play disappeared")?;
    play.script_start_load(
        &name,
        walk_source(destination),
        script::LoadShape::CompatClass,
        None,
        vec![],
    )?;

    let deadline = Instant::now() + WALK_WAIT;
    let before = loop {
        session.pump(&mut app);
        let proof = session
            .core
            .play()
            .ok_or("Play disappeared")?
            .manual_click_live_probe(&name);
        let moved = app.here.is_some_and(|tile| tile != here);
        if moved
            && proof["nav"]["route"] == true
            && proof["nav"]["request_id"]
                .as_u64()
                .is_some_and(|id| id != 0)
        {
            break proof;
        }
        if Instant::now() >= deadline {
            return Err(format!("script route arm deadline: {proof}"));
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let armed_here = app.here.ok_or("TUI armed tile is unavailable")?;
    let stamp = stamp();
    let dir = inputs.evidence.join(format!(
        "manual-click-c_tui-{cols}x{rows}-pause-{pause_owner}-recovery-{recovery}_{}_{}",
        name, stamp
    ));
    fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    capture(&mut app, cols, rows, &dir, &stamp, "01-armed", &before)?;

    if recovery {
        let deadline = Instant::now() + TERMINAL_WAIT;
        loop {
            let here = app.here.ok_or("TUI tile during recovery")?;
            session
                .core
                .play()
                .ok_or("Play disappeared")?
                .manual_click_live_age_gameplay(&name, here)?;
            session.pump(&mut app);
            let proof = session
                .core
                .play()
                .ok_or("Play disappeared")?
                .manual_click_live_probe(&name);
            if proof["script"]["recovering_anchor"].is_array()
                && proof["nav"]["route"] == true
                && proof["nav"]["request_id"] == 0
            {
                capture(
                    &mut app,
                    cols,
                    rows,
                    &dir,
                    &stamp,
                    "02-watchdog-recovery",
                    &proof,
                )?;
                break;
            }
            if Instant::now() >= deadline {
                return Err(format!("real recovery arm deadline: {proof}"));
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    app.show_screen(crate::Screen::Overview);
    let open_manual = app.on_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE));
    dispatch(&mut session, &mut app, open_manual);
    let step = app.on_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
    if !matches!(step, super::AppAction::WalkTile(_)) {
        return Err(format!(
            "production TUI Manual Up did not produce WalkTile: {step:?}"
        ));
    }
    dispatch(&mut session, &mut app, step);
    if pause_owner {
        let deadline = Instant::now() + TERMINAL_WAIT;
        loop {
            session.pump(&mut app);
            let proof = session
                .core
                .play()
                .ok_or("Play disappeared")?
                .manual_click_live_probe(&name);
            if proof["script"]["run_state"] == "Paused" && proof["nav"]["reason"] == "UserInput" {
                assert_correlated_user_input(&before, &proof);
                assert!(proof["script"]["recovering_anchor"].is_null());
                capture(
                    &mut app,
                    cols,
                    rows,
                    &dir,
                    &stamp,
                    "03-takeover-paused",
                    &proof,
                )?;
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

    let deadline = Instant::now() + TERMINAL_WAIT;
    let after = loop {
        session.pump(&mut app);
        let proof = session
            .core
            .play()
            .ok_or("Play disappeared")?
            .manual_click_live_probe(&name);
        if proof["script"]["walk_result"] == false && proof["script"]["terminal_count"] == 1 {
            break proof;
        }
        if Instant::now() >= deadline {
            return Err(format!("manual UserInput receipt deadline: {proof}"));
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_correlated_user_input(&before, &after);
    capture(&mut app, cols, rows, &dir, &stamp, "02-takeover", &after)?;

    let stable_until = Instant::now() + SETTLE_WAIT;
    while Instant::now() < stable_until {
        session.pump(&mut app);
        std::thread::sleep(Duration::from_millis(20));
    }
    let settled = session
        .core
        .play()
        .ok_or("Play disappeared")?
        .manual_click_live_probe(&name);
    assert_eq!(
        settled["nav"], after["nav"],
        "stale follow changed after takeover"
    );
    assert_eq!(settled["script"]["terminal_count"], 1);
    assert_eq!(settled["script"]["run_state"], "Running");
    assert_eq!(
        settled["script"]["runtime_generation"],
        before["script"]["runtime_generation"]
    );
    assert_eq!(settled["script"]["rearm_pending"], false);
    assert!(settled["script"]["recovering_anchor"].is_null());
    if app.here == Some(armed_here) {
        return Err("manual TUI step did not change the observed player tile".into());
    }
    capture(&mut app, cols, rows, &dir, &stamp, "03-settled", &settled)?;
    let close_manual = app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    dispatch(&mut session, &mut app, close_manual);
    let open_settings = app.on_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE));
    dispatch(&mut session, &mut app, open_settings);
    assert!(
        app.settings_state.open,
        "the real Overview settings key opens the popup"
    );
    for _ in 0..6 {
        app.on_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    }
    assert_eq!(
        app.settings_state.row, 6,
        "the added pause preference row is reachable"
    );
    capture(
        &mut app,
        cols,
        rows,
        &dir,
        &stamp,
        "04-settings-pause-row",
        &settled,
    )?;
    let prior = app.pause_script_on_manual_walk_abort;
    let toggle = app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    dispatch(&mut session, &mut app, toggle);
    session.pump(&mut app);
    assert_eq!(app.pause_script_on_manual_walk_abort, !prior);
    capture(
        &mut app,
        cols,
        rows,
        &dir,
        &stamp,
        "05-settings-toggle",
        &settled,
    )?;
    let restore = app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    dispatch(&mut session, &mut app, restore);
    session.pump(&mut app);
    println!(
        "PASS TUI {cols}x{rows} account={name} before={before} after={after} settled={settled} initial_here={here:?} armed_here={armed_here:?} final_here={:?}",
        app.here
    );
    if let Some(play) = session.core.play() {
        play.script_stop(&name);
    }
    session.core.set_play(None);
    Ok(())
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

fn capture(
    app: &mut TuiApp,
    cols: u16,
    rows: u16,
    dir: &Path,
    stamp: &str,
    label: &str,
    proof: &serde_json::Value,
) -> Result<(), String> {
    let mut terminal =
        Terminal::new(TestBackend::new(cols, rows)).map_err(|error| error.to_string())?;
    terminal
        .draw(|frame| app.draw(frame))
        .map_err(|error| error.to_string())?;
    let buffer = terminal.backend().buffer();
    let mut text = String::with_capacity(usize::from(cols) * usize::from(rows + 1));
    for y in 0..rows {
        for x in 0..cols {
            text.push_str(buffer[(x, y)].symbol());
        }
        text.push('\n');
    }
    fs::write(dir.join(format!("{stamp}_{label}.txt")), text).map_err(|error| error.to_string())?;
    let cells: Vec<_> = buffer
        .content
        .iter()
        .map(|cell| {
            serde_json::json!({
                "symbol": cell.symbol(),
                "fg": format!("{:?}", cell.fg),
                "bg": format!("{:?}", cell.bg),
            })
        })
        .collect();
    let receipt = serde_json::json!({
        "surface": "TUI",
        "cols": cols,
        "rows": rows,
        "proof": proof,
        "cells": cells,
    });
    fs::write(
        dir.join(format!("{stamp}_{label}.json")),
        serde_json::to_vec_pretty(&receipt).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())
}

#[test]
#[ignore = "requires LIVE=1, local R289 engine/nav pack and a throwaway HOME"]
fn live_manual_click_tui_120x40() {
    run_manual_click(120, 40, false, false).unwrap();
}

#[test]
#[ignore = "requires LIVE=1, local R289 engine/nav pack and a throwaway HOME"]
fn live_manual_click_tui_80x24() {
    run_manual_click(80, 24, false, false).unwrap();
}

#[test]
#[ignore = "requires LIVE=1, local R289 engine/nav pack and a throwaway HOME"]
fn live_manual_click_tui_recovery_pause_resume_120x40() {
    run_manual_click(120, 40, true, true).unwrap();
}

#[test]
#[ignore = "requires LIVE=1, local R289 engine/nav pack and a throwaway HOME"]
fn live_manual_click_tui_recovery_pause_resume_80x24() {
    run_manual_click(80, 24, true, true).unwrap();
}
