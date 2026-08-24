//! Shared live-harness helpers for the `e2e` tests.
//!
//! Every test is `#[ignore]` and additionally returns early unless `LIVE=1`,
//! so the default `cargo test` (and even `-- --ignored` without the env
//! var) stays green with no engine running. With `LIVE=1` against the local
//! engine a failure prints `FAIL: ...` and exits 1 (rs2b0t e2e style).
#![allow(dead_code)] // each test binary uses a subset of the helpers

use std::path::PathBuf;
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

use host_play::{Play, PlayOptions, SlotStatus};

static HEARTBEAT: Mutex<Option<Instant>> = Mutex::new(None);

fn short_status(statuses: &[SlotStatus]) -> String {
    statuses
        .iter()
        .map(|s| {
            format!(
                "{} lean={} in={} sc={} q={}/{} err={:?}",
                s.username,
                s.lean,
                s.ingame,
                s.scene_state,
                s.queue_position,
                s.queue_total,
                s.error
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn debug_heartbeat(ready: usize, want: usize) -> bool {
    let _ = (ready, want);
    let mut last = HEARTBEAT.lock().unwrap();
    match *last {
        Some(t) if t.elapsed() < Duration::from_secs(10) => false,
        _ => {
            *last = Some(Instant::now());
            true
        }
    }
}
use vault::{Profile, ProfileSettings, Vault};

/// rs2b0t-style harness failure: print and exit 1.
pub fn fail(msg: &str) -> ! {
    eprintln!("FAIL: {msg}");
    std::process::exit(1);
}

/// `LIVE=1` gates the ignored tests.
pub fn live() -> bool {
    std::env::var("LIVE").as_deref() == Ok("1")
}

/// A throwaway encrypted vault holding `user`/`pass` profiles with stable
/// distinct uids. The engine auto-registers unknown accounts.
pub fn temp_vault(entries: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "274bot-e2e-{}-{}",
        std::process::id(),
        entries.len()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("vault");
    if path.exists() {
        std::fs::remove_file(&path).unwrap();
    }
    let mut vault = Vault::create(&path, "bot").unwrap();
    for (i, (user, pass)) in entries.iter().enumerate() {
        vault
            .upsert(Profile {
                username: (*user).into(),
                password: (*pass).into(),
                uid: 274_000_001 + i as i32,
                settings: ProfileSettings::default(),
            })
            .unwrap();
    }
    path
}

pub fn profiles(entries: &[(&str, &str)]) -> Vec<Profile> {
    let path = temp_vault(entries);
    let vault = Vault::unlock(&path, "bot").unwrap();
    entries
        .iter()
        .map(|(user, _)| vault.get(user).unwrap().clone())
        .collect()
}

/// Engine + cache defaults (matches `client-play` / the operator syntax).
pub fn options() -> PlayOptions {
    let home = std::env::var("HOME").unwrap();
    PlayOptions {
        host: "127.0.0.1".into(),
        port: 43594,
        cache_dir: format!("{home}/experiments/Server/engine/data/pack/client"),
        lowmem: true,
        mainland: false,
    }
}

/// Poll until `want` channels are up (fat: scene 2, lean: ingame).
pub fn wait_up(play: &Play, want: usize, timeout: Duration, case: &str) {
    let deadline = Instant::now() + timeout;
    loop {
        let statuses = play.statuses();
        let ready = statuses.iter().filter(|s| s.is_up()).count();
        if ready >= want {
            println!("{case}: {ready}/{want} up");
            return;
        }
        if Instant::now() >= deadline {
            fail(&format!(
                "{case}: {ready}/{want} up after {timeout:?}; statuses: {statuses:?}"
            ));
        }
        if debug_heartbeat(ready, want) {
            println!(
                "{case}: waiting {ready}/{want} up; {}",
                short_status(&statuses)
            );
        }
        thread::sleep(Duration::from_millis(250));
    }
}

/// Poll `play` until `want` slots show `ingame && scene_state == 2` or the
/// timeout fires (then `fail`).
pub fn wait_ingame(play: &Play, want: usize, timeout: Duration, case: &str) {
    let deadline = Instant::now() + timeout;
    loop {
        let statuses = play.statuses();
        let ready = statuses
            .iter()
            .filter(|s| s.ingame && s.scene_state == 2)
            .count();
        if ready >= want {
            println!("PASS: {case}: {ready} slot(s) ingame with scene 2");
            return;
        }
        if Instant::now() >= deadline {
            fail(&format!(
                "{case}: {ready}/{want} slot(s) ingame scene 2 after {timeout:?}; \
                 statuses: {statuses:?}"
            ));
        }
        thread::sleep(Duration::from_millis(250));
    }
}

/// Sorted login-handshake start times (for the not-simultaneous assert).
pub fn login_starts(statuses: &[SlotStatus]) -> Vec<Instant> {
    let mut starts: Vec<Instant> = statuses.iter().filter_map(|s| s.login_started).collect();
    starts.sort();
    starts
}

/// Poll `play` until `want` slots show `!ingame` (a clean IF logout put
/// them back on the title) or the timeout fires (then `fail` — a dirty
/// disconnect is a FAIL, not a skip).
pub fn wait_logged_out(play: &Play, want: usize, timeout: Duration, case: &str) {
    let deadline = Instant::now() + timeout;
    loop {
        let statuses = play.statuses();
        let out = statuses.iter().filter(|s| !s.ingame).count();
        if out >= want {
            println!("PASS: {case}: {out} slot(s) on the title after logout");
            return;
        }
        if Instant::now() >= deadline {
            fail(&format!(
                "{case}: {out}/{want} slot(s) left the game after {timeout:?}; \
                 statuses: {statuses:?}"
            ));
        }
        thread::sleep(Duration::from_millis(250));
    }
}

/// A title-hold assert: for `duration`, every slot must stay `!ingame`
/// (armed slots must not handshake on their own). Fails on the first
/// ingame status.
pub fn assert_none_ingame_for(play: &Play, duration: Duration, case: &str) {
    let deadline = Instant::now() + duration;
    loop {
        let statuses = play.statuses();
        let ingame = statuses.iter().filter(|s| s.ingame).count();
        if ingame > 0 {
            fail(&format!(
                "{case}: {ingame} slot(s) went ingame during the {duration:?} title hold; \
                 statuses: {statuses:?}"
            ));
        }
        if Instant::now() >= deadline {
            println!("PASS: {case}: no slot ingame for {duration:?}");
            return;
        }
        thread::sleep(Duration::from_millis(250));
    }
}
