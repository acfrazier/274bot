//! Live: N wall channels — at most one head (fat `Client`), every other
//! slot a lean channel (`Lean`, no World). Print peak process RSS.
//!
//! One N per process (peak RSS is process-lifetime):
//! `LIVE=1 RSS_N=1 cargo test -p e2e --test channel_ladder -- --ignored --test-threads=1 --nocapture`

mod common;

use std::thread;
use std::time::{Duration, Instant};

use common::{fail, live, options};
use host_play::{run_channels, sample_process, Play};
use vault::{Profile, ProfileSettings, Vault};

fn parse_rss_n_from(raw: Option<&str>) -> Result<usize, &'static str> {
    match raw {
        None => Ok(1),
        Some("1") => Ok(1),
        Some("2") => Ok(2),
        Some("4") => Ok(4),
        Some("50") => Ok(50),
        _ => Err("channel_ladder: RSS_N must be 1, 2, 4, or 50"),
    }
}

fn ladder_profiles(n: usize) -> Vec<Profile> {
    let dir = std::env::temp_dir().join(format!(
        "274bot-channel-ladder-{}-{}",
        std::process::id(),
        n
    ));
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
            uid: 274_000_200 + i as i32,
            settings: ProfileSettings::default(),
        };
        vault.upsert(p.clone()).unwrap();
        out.push(p);
    }
    out
}

/// Poll until every channel is up: the one head must be `ingame` with
/// scene 2 (it builds a real scene); a lean channel counts once its cold
/// login granted (`ingame` — the thin snapshot only ever reaches
/// `scene_state` 1 on REBUILD_NORMAL, never 2).
fn wait_all_up(play: &Play, n: usize) {
    let timeout = Duration::from_secs(300);
    let deadline = Instant::now() + timeout;
    loop {
        let statuses = play.statuses();
        let ready = statuses
            .iter()
            .filter(|s| if s.lean { s.ingame } else { s.ingame && s.scene_state == 2 })
            .count();
        if ready >= n {
            return;
        }
        if Instant::now() >= deadline {
            fail(&format!(
                "channel_ladder: {ready}/{n} channel(s) up after 300s; \
                 statuses: {statuses:?}"
            ));
        }
        thread::sleep(Duration::from_millis(250));
    }
}

#[test]
#[ignore = "requires a local 274 engine and LIVE=1"]
fn live_channel_ladder_lean_wall() {
    if !live() {
        return;
    }
    let n = match parse_rss_n_from(std::env::var("RSS_N").ok().as_deref()) {
        Ok(n) => n,
        Err(msg) => fail(msg),
    };
    // At most one head: r0 is the fat Client, r1..r{n-1} are lean channels.
    let play = run_channels(&options(), ladder_profiles(n), 1);
    wait_all_up(&play, n);
    // Give the wall a beat to settle before sampling peak RSS.
    thread::sleep(Duration::from_secs(10));
    let rows = play.statuses();
    let (rss, _) = sample_process();
    if rss == 0 {
        fail("channel_ladder: rss=0");
    }
    let heads = rows.iter().filter(|s| !s.lean).count();
    let leanes = rows.iter().filter(|s| s.lean).count();
    let lean_paint_sum = rows.iter().filter(|s| s.lean).map(|s| s.paint_n).sum::<u64>();
    if lean_paint_sum > 0 {
        fail("channel_ladder: lean channels painted");
    }
    println!(
        "channel_ladder n={n} rss={rss} heads={heads} leanes={leanes} lean_paint_sum={lean_paint_sum}"
    );
    println!("PASS: channel_ladder n={n} rss={rss}");
}

// keep parse_rss unit tests below the live test

#[test]
fn parse_rss_n_default_and_allowed() {
    assert_eq!(parse_rss_n_from(None).unwrap(), 1);
    assert_eq!(parse_rss_n_from(Some("1")).unwrap(), 1);
    assert_eq!(parse_rss_n_from(Some("2")).unwrap(), 2);
    assert_eq!(parse_rss_n_from(Some("4")).unwrap(), 4);
    assert_eq!(parse_rss_n_from(Some("50")).unwrap(), 50);
}

#[test]
fn parse_rss_n_rejects_bad() {
    let err = "channel_ladder: RSS_N must be 1, 2, 4, or 50";
    assert_eq!(parse_rss_n_from(Some("")).unwrap_err(), err);
    assert_eq!(parse_rss_n_from(Some("3")).unwrap_err(), err);
    assert_eq!(parse_rss_n_from(Some("one")).unwrap_err(), err);
}
