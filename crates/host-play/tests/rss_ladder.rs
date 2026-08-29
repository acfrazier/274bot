//! Live: N headless Clients, every slot `set_draw(false)`, print peak RSS.
//!
//! The old `paint_n`/`skip_n`/`game_draw_enters` asserts are gone with the
//! `SlotStatus` draw counters (M2); the surviving regression is the RSS
//! ladder across N headless `Client` slots.
//!
//! One N per process (peak RSS is process-lifetime):
//! `LIVE=1 RSS_N=1 cargo test -p host-play --test rss_ladder -- --ignored --test-threads=1 --nocapture`

mod common;

use std::thread;
use std::time::{Duration, Instant};

use common::{fail, live, options};
use host_play::{run_with_io, sample_process, Play};
use vault::{Profile, ProfileSettings, Vault};

fn parse_rss_n_from(raw: Option<&str>) -> Result<usize, &'static str> {
    match raw {
        None => Ok(1),
        Some("1") => Ok(1),
        Some("2") => Ok(2),
        Some("4") => Ok(4),
        _ => Err("rss_ladder: RSS_N must be 1, 2, or 4"),
    }
}

fn ladder_profiles(n: usize) -> Vec<Profile> {
    let dir = std::env::temp_dir().join(format!("274bot-rss-ladder-{}-{}", std::process::id(), n));
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

fn wait_all_scene2(play: &Play, n: usize) {
    let timeout = Duration::from_secs(180);
    let deadline = Instant::now() + timeout;
    loop {
        let statuses = play.statuses();
        let ready = statuses
            .iter()
            .filter(|s| s.ingame && s.scene_state == 2)
            .count();
        if ready >= n {
            return;
        }
        if Instant::now() >= deadline {
            fail(&format!(
                "rss_ladder: {ready}/{n} slot(s) ingame scene 2 after 180s"
            ));
        }
        thread::sleep(Duration::from_millis(250));
    }
}

#[test]
#[ignore = "requires a local 274 engine and LIVE=1"]
fn live_rss_ladder_all_null() {
    if !live() {
        return;
    }
    let n = match parse_rss_n_from(std::env::var("RSS_N").ok().as_deref()) {
        Ok(n) => n,
        Err(msg) => fail(msg),
    };
    let play = run_with_io(
        &options(),
        ladder_profiles(n),
        |_| (None, None),
        |c, _| c.set_draw(false),
    );
    wait_all_scene2(&play, n);
    // Give the wall a beat to settle before sampling peak RSS.
    thread::sleep(Duration::from_secs(10));
    let (rss, _) = sample_process();
    if rss == 0 {
        fail("rss_ladder: rss=0");
    }
    let (bytes_in, bytes_out): (u64, u64) = play
        .statuses()
        .iter()
        .fold((0, 0), |(i, o), s| (i + s.bytes_in, o + s.bytes_out));
    println!("rss_ladder n={n} rss={rss} bytes_in={bytes_in} bytes_out={bytes_out}");
    println!("PASS: rss_ladder n={n} rss={rss}");
}

// keep parse_rss unit tests from Task 3 below this

#[test]
fn parse_rss_n_default_and_allowed() {
    assert_eq!(parse_rss_n_from(None).unwrap(), 1);
    assert_eq!(parse_rss_n_from(Some("1")).unwrap(), 1);
    assert_eq!(parse_rss_n_from(Some("2")).unwrap(), 2);
    assert_eq!(parse_rss_n_from(Some("4")).unwrap(), 4);
}

#[test]
fn parse_rss_n_rejects_bad() {
    let err = "rss_ladder: RSS_N must be 1, 2, or 4";
    assert_eq!(parse_rss_n_from(Some("")).unwrap_err(), err);
    assert_eq!(parse_rss_n_from(Some("3")).unwrap_err(), err);
    assert_eq!(parse_rss_n_from(Some("50")).unwrap_err(), err);
    assert_eq!(parse_rss_n_from(Some("one")).unwrap_err(), err);
}
