//! Shared live-harness helpers for the `e2e` tests.
//!
//! Every test is `#[ignore]` and additionally returns early unless `LIVE=1`,
//! so the default `cargo test` (and even `-- --ignored` without the env
//! var) stays green with no engine running. With `LIVE=1` against the local
//! engine a failure prints `FAIL: ...` and exits 1 (rs2b0t e2e style).
#![allow(dead_code)] // each test binary uses a subset of the helpers

use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

use host_play::{Play, PlayOptions};
use scenario::ScenarioRunner;
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
/// distinct uids. The engine auto-registers unknown accounts. Accepts
/// `&str` or `String` entries (live twins mint per-run usernames).
pub fn temp_vault<S: AsRef<str>>(entries: &[(S, S)]) -> PathBuf {
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
                username: user.as_ref().into(),
                password: pass.as_ref().into(),
                uid: 274_000_001 + i as i32,
                settings: ProfileSettings::default(),
            })
            .unwrap();
    }
    path
}

pub fn profiles<S: AsRef<str>>(entries: &[(S, S)]) -> Vec<Profile> {
    let path = temp_vault(entries);
    let vault = Vault::unlock(&path, "bot").unwrap();
    entries
        .iter()
        .map(|(user, _)| vault.get(user.as_ref()).unwrap().clone())
        .collect()
}

/// Mint `n` per-run usernames for a scenario's seed, arm `runner` with
/// them (so its per-frame hooks drive the minted slots), and return the
/// `(user, pass)` vault entries (password = user, the auto-registration
/// convention). The engine auto-registers unknown names, so a live twin
/// never logs into the shared `test` save; player saves accumulate under
/// the engine's `player/` dir — wipe it to reset.
pub fn mint_seed(runner: &mut ScenarioRunner, n: usize) -> Vec<(String, String)> {
    let names = host_play::mint_live_names(n);
    runner.set_live_names(&names);
    names.into_iter().map(|u| (u.clone(), u)).collect()
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

/// Poll until `want` slots show `!ingame` (clean IF logout) or `fail`.
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
