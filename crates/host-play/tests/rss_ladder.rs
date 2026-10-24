//! Live: N headless Clients, every slot `set_draw(false)`, print peak RSS
//! plus OnDemand worker count, TCP to the engine port, and (Linux) task/fd
//! census for the D1 footprint comparison.
//!
//! The old `paint_n`/`skip_n`/`game_draw_enters` asserts are gone with the
//! `SlotStatus` draw counters (M2); the surviving regression is the RSS
//! ladder across N headless `Client` slots.
//!
//! One N per process (peak RSS is process-lifetime). `RSS_LOAD=1` starts one
//! tiny Load card per headless slot, measuring J=N and the JS singleton slope.
//! Use the checked 289 template: `ENGINE_DIR`, `WORLD_ENGINE_DIR`,
//! `WORLD_NAV_PACK`, `BOT_CACHE_DIR`, and `BOT_GAME_PORT=44594` point at the
//! local Engine A inputs. `LIVE=1 RSS_LOAD=1 RSS_N=10
//! BOT_LIVE_NAME_PREFIX=<unique1-4char> cargo test -p host-play --test
//! rss_ladder -- --ignored --test-threads=1 --nocapture`. Accepted N:
//! 1, 2, 4, 10. The prefix owns all minted accounts (12-char engine limit).

mod common;

use std::thread;
use std::time::{Duration, Instant};

use client::io::OnDemand;
use common::{fail, live, options};
use host::debug_enabled;
use host_play::{count_tcp_to, run_with_template, sample_process, Play, ProfileOptions};
use vault::{Profile, ProfileSettings, Vault};

fn parse_rss_n_from(raw: Option<&str>) -> Result<usize, &'static str> {
    match raw {
        None => Ok(1),
        Some("1") => Ok(1),
        Some("2") => Ok(2),
        Some("4") => Ok(4),
        Some("10") => Ok(10),
        _ => Err("rss_ladder: RSS_N must be 1, 2, 4, or 10"),
    }
}

fn ladder_profiles(n: usize) -> Vec<Profile> {
    let prefix = std::env::var("BOT_LIVE_NAME_PREFIX").unwrap_or_default();
    if prefix.is_empty() {
        fail("rss_ladder: set a unique BOT_LIVE_NAME_PREFIX for this live run");
    }
    // mint_live_names keeps every name within the engine's 12-char limit.
    let names = host_play::mint_live_names(n);
    if names.len() != n {
        fail("rss_ladder: failed to mint every live account");
    }
    for name in &names {
        if !name.starts_with(&prefix) {
            fail(&format!(
                "rss_ladder: minted {name:?} outside BOT_LIVE_NAME_PREFIX={prefix:?}"
            ));
        }
        if name.len() > 12 {
            fail(&format!(
                "rss_ladder: minted {name:?} exceeds the 12-char engine limit"
            ));
        }
    }
    let credentials = host_play::mint_live_entries(&names);
    let dir = std::env::temp_dir().join(format!("274bot-rss-ladder-{}-{}", std::process::id(), n));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("vault");
    if path.exists() {
        let _ = std::fs::remove_file(&path);
    }
    let mut vault = Vault::create(&path, "test-passphrase-01").unwrap();
    let mut out = Vec::with_capacity(n);
    for (i, (username, password)) in credentials.into_iter().enumerate() {
        let p = Profile {
            username,
            password: password.into(),
            uid: 274_000_200 + i as i32,
            settings: ProfileSettings::default(),
        };
        vault.upsert(p.clone()).unwrap();
        out.push(p);
    }
    out
}

/// Open fd count (Linux `/proc/self/fd`); `None` elsewhere or on read failure.
fn fd_count() -> Option<usize> {
    #[cfg(target_os = "linux")]
    {
        Some(std::fs::read_dir("/proc/self/fd").ok()?.count())
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

/// Live thread-task count plus a compact `comm=count` histogram (Linux
/// `/proc/self/task`); `None` elsewhere or on read failure. D1 uses the
/// counts to tell per-slot thread/fd growth apart from per-slot heap.
fn task_census() -> Option<(usize, String)> {
    #[cfg(target_os = "linux")]
    {
        let mut hist = std::collections::BTreeMap::<String, usize>::new();
        let tasks = std::fs::read_dir("/proc/self/task").ok()?;
        let mut total = 0usize;
        for task in tasks {
            let task = task.ok()?;
            let comm = std::fs::read_to_string(task.path().join("comm"))
                .unwrap_or_else(|_| "?\n".into())
                .trim()
                .to_owned();
            *hist.entry(comm).or_default() += 1;
            total += 1;
        }
        let summary = hist
            .iter()
            .map(|(name, count)| format!("{name}={count}"))
            .collect::<Vec<_>>()
            .join(",");
        Some((total, summary))
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
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
#[ignore = "requires LIVE=1, Engine A, revision-289 template and throwaway HOME"]
fn live_rss_ladder_headless() {
    if !live() {
        return;
    }
    let n = match parse_rss_n_from(std::env::var("RSS_N").ok().as_deref()) {
        Ok(n) => n,
        Err(msg) => fail(msg),
    };
    let mut opts = options();
    opts.port = std::env::var("BOT_GAME_PORT")
        .ok()
        .and_then(|port| port.parse().ok())
        .unwrap_or(opts.port);
    if let Some(cache) = std::env::var_os("BOT_CACHE_DIR") {
        opts.cache_dir = std::path::PathBuf::from(cache)
            .to_string_lossy()
            .into_owned();
    }
    // Use the same revision-289 checked template as the product and the
    // Script API live cells. Direct PlayOptions uses the legacy 274 wire
    // profile, which cannot log into Engine A even with the correct port.
    let home = std::path::PathBuf::from(std::env::var_os("HOME").expect("throwaway HOME"));
    let template = ProfileOptions {
        profile: Some("local-289".into()),
        revision: Some("289".into()),
        host: Some(opts.host.clone()),
        asset_host: Some(opts.host.clone()),
        port: Some(opts.port),
        http_port: Some(
            std::env::var("BOT_HTTP_PORT")
                .ok()
                .and_then(|port| port.parse().ok())
                .unwrap_or(2080),
        ),
        cache_dir: Some((&opts.cache_dir).into()),
        unpack_dir: Some(home.join(".274bot/unpack-289")),
        vault_path: Some(home.join("vault")),
        nav_pack: Some(
            std::env::var_os("WORLD_NAV_PACK")
                .expect("WORLD_NAV_PACK")
                .into(),
        ),
        engine_dir: Some(
            std::env::var_os("WORLD_ENGINE_DIR")
                .expect("WORLD_ENGINE_DIR")
                .into(),
        ),
        ..ProfileOptions::default()
    }
    .resolve(None)
    .expect("local-289 profile")
    .prepare_template()
    .expect("cached revision-289 template");
    let play = run_with_template(
        template,
        false,
        ladder_profiles(n),
        |_| (None, None),
        |c, _, _hold| c.set_draw(false),
    )
    .expect("start checked Play");
    wait_all_scene2(&play, n);
    let load = std::env::var("RSS_LOAD").as_deref() == Ok("1");
    if load {
        let names = play
            .statuses()
            .iter()
            .map(|status| status.username.clone())
            .collect::<Vec<_>>();
        for name in &names {
            play.script_start_load(
                name,
                "export function tick(api) {}".into(),
                script_js::LoadShape::NativeTick,
                None,
                vec![],
            )
            .unwrap_or_else(|error| fail(&format!("rss_ladder Load {name}: {error}")));
        }
        let until = Instant::now() + Duration::from_secs(30);
        while names
            .iter()
            .any(|name| play.script_state(name) != script::RunState::Running)
        {
            if Instant::now() >= until {
                fail(&format!(
                    "rss_ladder: {n} Load scripts did not all reach Running"
                ));
            }
            thread::sleep(Duration::from_millis(50));
        }
    }
    // Give the wall a beat to settle before sampling peak RSS.
    thread::sleep(Duration::from_secs(10));
    let (rss, _) = sample_process();
    if rss == 0 {
        fail("rss_ladder: rss=0");
    }
    let workers = OnDemand::live_workers_for(&opts.host, opts.port);
    let tcp = count_tcp_to(&opts.host, opts.port);
    if debug_enabled() {
        eprintln!(
            "rss_ladder debug: live_workers_for({}:{})={workers} tcp={tcp:?}",
            opts.host, opts.port
        );
    }
    let (bytes_in, bytes_out): (u64, u64) = play
        .statuses()
        .iter()
        .fold((0, 0), |(i, o), s| (i + s.bytes_in, o + s.bytes_out));
    let tcp_s = tcp.map(|c| c.to_string()).unwrap_or_else(|| "?".into());
    let current = host_play::current_resident_bytes().unwrap_or(0);
    let fds = fd_count()
        .map(|c| c.to_string())
        .unwrap_or_else(|| "?".into());
    let (tasks_s, tasks_hist) = task_census()
        .map(|(total, hist)| (total.to_string(), format!(" tasks_hist=[{hist}]")))
        .unwrap_or_else(|| ("?".into(), String::new()));
    println!(
        "rss_ladder n={n} load={load} rss={rss} current={current} ondemand={workers} tcp={tcp_s} tasks={tasks_s}{tasks_hist} fds={fds} bytes_in={bytes_in} bytes_out={bytes_out}"
    );
    // Connection contract: one OnDemand worker for every N. Do not fail on RSS size.
    if workers != 1 {
        fail(&format!(
            "rss_ladder: want 1 OnDemand worker for {}:{}, got {workers}",
            opts.host, opts.port
        ));
    }
    if let Some(tcp) = tcp {
        if tcp > n + 1 {
            fail(&format!(
                "rss_ladder: tcp={tcp} to {}:{} > n+1 (game+ondemand)",
                opts.host, opts.port
            ));
        }
    }
    println!("PASS: rss_ladder n={n} load={load} rss={rss} current={current} ondemand={workers} tcp={tcp_s} tasks={tasks_s} fds={fds}");
}

// keep parse_rss unit tests from Task 3 below this

#[test]
fn parse_rss_n_default_and_allowed() {
    assert_eq!(parse_rss_n_from(None).unwrap(), 1);
    assert_eq!(parse_rss_n_from(Some("1")).unwrap(), 1);
    assert_eq!(parse_rss_n_from(Some("2")).unwrap(), 2);
    assert_eq!(parse_rss_n_from(Some("4")).unwrap(), 4);
    assert_eq!(parse_rss_n_from(Some("10")).unwrap(), 10);
}

#[test]
fn parse_rss_n_rejects_bad() {
    let err = "rss_ladder: RSS_N must be 1, 2, 4, or 10";
    assert_eq!(parse_rss_n_from(Some("")).unwrap_err(), err);
    assert_eq!(parse_rss_n_from(Some("3")).unwrap_err(), err);
    assert_eq!(parse_rss_n_from(Some("50")).unwrap_err(), err);
    assert_eq!(parse_rss_n_from(Some("one")).unwrap_err(), err);
}
