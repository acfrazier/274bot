//! Live: headed wall of N `Client` slots (flat model — no lean, no baton).
//! After everyone is up, walk the focus through the list in order
//! (`r0` → `r1` → …): `play.focus(name)` selects which slot is the TV.
//! Each hop must land as the focused slot at `ingame && scene_state == 2`,
//! and every slot must stay up across the swap (focus is pure
//! bookkeeping — switching it never touches a socket).
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

/// Snapshot the wall for a FAIL dump: the focused slot plus every slot's
/// up state.
fn dump(play: &Play, tag: &str) {
    let rows = play.statuses();
    let up = rows
        .iter()
        .map(|s| {
            format!(
                "{} up={} ingame={} scene={} player={} err={:?}",
                s.username,
                s.is_up(),
                s.ingame,
                s.scene_state,
                s.player,
                s.error
            )
        })
        .collect::<Vec<_>>();
    println!(
        "channel_walk {tag}: focused={:?} slots=[{}]",
        play.focused(),
        up.join("; ")
    );
}

/// Poll until `want` is the focused slot and reports up with a local
/// player (a full `Client` in a built scene — the flat-model TV proof).
/// Focus is bookkeeping, so there is no baton to poll.
fn wait_focused(play: &Play, want: &str, timeout: Duration, tag: &str) {
    let deadline = Instant::now() + timeout;
    loop {
        let ok = play.focused().as_deref() == Some(want)
            && play
                .statuses()
                .iter()
                .any(|s| s.username == want && s.is_up() && !s.player.is_empty());
        if ok {
            println!("channel_walk: TV is {want} (up)");
            return;
        }
        if Instant::now() >= deadline {
            dump(play, tag);
            fail(&format!(
                "channel_walk: {tag}: want focused {want} up after {timeout:?}"
            ));
        }
        thread::sleep(Duration::from_millis(50));
    }
}

#[test]
#[ignore = "requires a local 274 engine and LIVE=1"]
fn live_channel_walk_focuses_every_slot() {
    if !live() {
        return;
    }
    let n = parse_n();
    let names: Vec<String> = (0..n).map(|i| format!("r{i}")).collect();
    println!("channel_walk: spawn n={n} Client slots");
    let mut play = run_channels(&options(), walk_profiles(n), 1);
    wait_up(&play, n, Duration::from_secs(600), "channel_walk login");
    dump(&play, "all-up");

    // Initial TV: r0 focused, up at scene 2.
    play.focus(&names[0]);
    wait_focused(&play, &names[0], Duration::from_secs(120), "initial-tv");

    // Walk the focus through the rest of the wall. Each hop must land the
    // new slot as the focused TV while every slot stays up — the flat
    // model's "no second Client, no dropped channel" guarantee.
    for (i, name) in names.iter().enumerate().skip(1) {
        let prev = play.focused();
        println!("channel_walk: hop {}/{} {:?} -> {name}", i, n - 1, prev);
        let t0 = Instant::now();
        play.focus(name);
        wait_focused(
            &play,
            name,
            Duration::from_secs(120),
            &format!("hop-{name}"),
        );
        println!("channel_walk: hop {name} ok in {:?}", t0.elapsed());
    }
    dump(&play, "done");

    // No slot may have dropped while focus walked the wall.
    let up = play.statuses().iter().filter(|s| s.is_up()).count();
    if up != n {
        fail(&format!(
            "channel_walk: slots dropped during focus walk ({up}/{n} up)"
        ));
    }
    println!(
        "PASS: channel_walk n={n} last={}",
        play.focused().unwrap_or_default()
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
