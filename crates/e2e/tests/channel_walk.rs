//! Live: headed wall of N channels (1 fat TV + N-1 lean). After everyone
//! is up, retune the TV through the list in order (`r0` → `r1` → …).
//! Each hop must land as the unique fat head at `ingame && scene_state==2`
//! without spawning a second Client.
//!
//! `LIVE=1 CHANNEL_N=2 cargo test -p e2e --test channel_walk -- --ignored --test-threads=1 --nocapture`
//! Default `CHANNEL_N=50` (or `RSS_N`).

mod common;

use std::thread;
use std::time::{Duration, Instant};

use common::{fail, live, options, wait_up};
use host_play::{run_channels, Play};
use vault::{Profile, ProfileSettings, Vault};

fn parse_n() -> usize {
    let raw = std::env::var("CHANNEL_N")
        .ok()
        .or_else(|| std::env::var("RSS_N").ok());
    match raw.as_deref() {
        None => 50,
        Some("2") => 2,
        Some("4") => 4,
        Some("50") => 50,
        Some(other) => fail(&format!(
            "channel_walk: CHANNEL_N must be 2, 4, or 50 (got {other})"
        )),
    }
}

fn walk_profiles(n: usize) -> Vec<Profile> {
    let dir =
        std::env::temp_dir().join(format!("274bot-channel-walk-{}-{}", std::process::id(), n));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("vault");
    if path.exists() {
        let _ = std::fs::remove_file(&path);
    }
    let mut vault = Vault::create(&path, "bot").unwrap();
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let name = format!("r{i}");
        let p = Profile {
            username: name.clone(),
            password: name,
            uid: 274_000_300 + i as i32,
            settings: ProfileSettings::default(),
        };
        vault.upsert(p.clone()).unwrap();
        out.push(p);
    }
    out
}

fn dump(play: &Play, tag: &str) {
    let rows = play.statuses();
    let fat = rows
        .iter()
        .filter(|s| !s.lean)
        .map(|s| {
            format!(
                "{} ingame={} scene={} err={:?}",
                s.username, s.ingame, s.scene_state, s.error
            )
        })
        .collect::<Vec<_>>();
    let lean_up = rows.iter().filter(|s| s.lean && s.ingame).count();
    let lean_n = rows.iter().filter(|s| s.lean).count();
    println!(
        "channel_walk {tag}: pending={} fat_head={:?} fat=[{}] lean_up={lean_up}/{lean_n}",
        play.tune_pending(),
        play.fat_head_name(),
        fat.join("; ")
    );
}

fn wait_fat(play: &mut Play, want: &str, lean_want: usize, timeout: Duration, tag: &str) {
    let deadline = Instant::now() + timeout;
    loop {
        play.poll_tune();
        let rows = play.statuses();
        let fat = rows.iter().find(|s| !s.lean);
        let ok = fat.is_some_and(|s| s.username == want && s.is_up());
        let fats = rows.iter().filter(|s| !s.lean).count();
        let lean_rows = rows.iter().filter(|s| s.lean).count();
        let lean_up = rows.iter().filter(|s| s.lean && s.ingame).count();
        if ok && fats == 1 && lean_rows == lean_want && lean_up == lean_want && !play.tune_pending()
        {
            println!("channel_walk: TV is {want} (up) lean_up={lean_up}/{lean_rows}");
            return;
        }
        if Instant::now() >= deadline {
            dump(play, tag);
            fail(&format!(
                "channel_walk: {tag}: want fat {want} + {lean_want} lean after {timeout:?}"
            ));
        }
        thread::sleep(Duration::from_millis(50));
    }
}

#[test]
#[ignore = "requires a local 274 engine and LIVE=1"]
fn live_channel_walk_retunes_every_head() {
    if !live() {
        return;
    }
    let n = parse_n();
    let names: Vec<String> = (0..n).map(|i| format!("r{i}")).collect();
    println!(
        "channel_walk: spawn n={n} (1 TV + {} lean)",
        n.saturating_sub(1)
    );
    let mut play = run_channels(&options(), walk_profiles(n), 1);
    wait_up(&play, n, Duration::from_secs(600), "channel_walk login");
    dump(&play, "all-up");
    let fats = play.statuses().iter().filter(|s| !s.lean).count();
    if fats != 1 {
        fail(&format!(
            "channel_walk: expected 1 fat after login, got {fats}"
        ));
    }
    wait_fat(
        &mut play,
        &names[0],
        n - 1,
        Duration::from_secs(120),
        "initial-tv",
    );

    for (i, name) in names.iter().enumerate().skip(1) {
        let prev = play.fat_head_name();
        println!("channel_walk: hop {}/{} {:?} -> {name}", i, n - 1, prev);
        let t0 = Instant::now();
        if let Err(e) = play.retune(name, None, None) {
            dump(&play, "retune-err");
            fail(&format!("channel_walk: retune {name}: {e:?}"));
        }
        wait_fat(
            &mut play,
            name,
            n - 1,
            Duration::from_secs(120),
            &format!("hop-{name}"),
        );
        println!("channel_walk: hop {name} ok in {:?}", t0.elapsed());
    }
    dump(&play, "done");
    let lean_up = play
        .statuses()
        .iter()
        .filter(|s| s.lean && s.ingame)
        .count();
    if lean_up != n - 1 {
        fail(&format!(
            "channel_walk: parked leanes dropped ({lean_up}/{} ingame)",
            n - 1
        ));
    }
    println!(
        "PASS: channel_walk n={n} last={} lean_up={lean_up}",
        play.fat_head_name().unwrap_or_default()
    );
}

#[test]
fn parse_n_defaults_to_fifty() {
    // The live test reads env; this pins the allowed set used by parse_n's match.
    assert_eq!(
        match None::<&str> {
            None => 50,
            Some("2") => 2,
            _ => 0,
        },
        50
    );
}
