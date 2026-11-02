//! Live memory qualification: N headless Clients using the selected hosted WSS
//! profile, each with one tiny Load card. Emits process RSS, Rust allocator,
//! V8 heap, task/fd and network counters for the matched native-tls comparator.
//!
//! One N per process (peak RSS is process-lifetime). Use `BOT_SERVER_PROFILE`
//! to select the checked 289 WSS profile override and `SSL_CERT_FILE` /
//! `SSL_CERT_DIR` for its isolated private CA. The candidate installs that CA
//! through the adapter's test-support platform-verifier path; origin/dev's
//! native-tls path reads the same `SSL_CERT_FILE`.
//!
//! `ENGINE_DIR`, `WORLD_ENGINE_DIR`, `WORLD_NAV_PACK`, `BOT_CACHE_DIR`,
//! `BOT_GAME_PORT`, and `BOT_HTTP_PORT` point at the local Engine A proxy
//! inputs. `LIVE=1 RSS_LOAD=1 RSS_N=10 BOT_LIVE_NAME_PREFIX=<unique1-4char>
//! cargo test -p host-play --features memory-profile,soak-observers --test
//! rss_ladder -- --ignored --test-threads=1 --nocapture`. Accepted N: 1, 2, 4,
//! 10. The prefix owns all minted accounts (12-char engine limit).

#[cfg(feature = "memory-profile")]
use std::alloc::{GlobalAlloc, Layout};
#[cfg(feature = "memory-profile")]
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

#[cfg(feature = "memory-profile")]
static QUALIFICATION_RUST_LIVE_BYTES: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "memory-profile")]
static QUALIFICATION_RUST_PEAK_BYTES: AtomicU64 = AtomicU64::new(0);

#[cfg(feature = "memory-profile")]
struct QualificationAllocator;

#[cfg(feature = "memory-profile")]
#[global_allocator]
static QUALIFICATION_ALLOCATOR: QualificationAllocator = QualificationAllocator;

#[cfg(feature = "memory-profile")]
unsafe impl GlobalAlloc for QualificationAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the allocator contract is supplied by GlobalAlloc's caller;
        // the exact layout is forwarded to the existing benchmark allocator.
        let pointer = unsafe { host_play::memory::BENCHMARK_ALLOCATOR.alloc(layout) };
        if !pointer.is_null() {
            note_rust_allocation(layout.size());
        }
        pointer
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the allocator contract is supplied by GlobalAlloc's caller;
        // the exact layout is forwarded to the existing benchmark allocator.
        let pointer = unsafe { host_play::memory::BENCHMARK_ALLOCATOR.alloc_zeroed(layout) };
        if !pointer.is_null() {
            note_rust_allocation(layout.size());
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: pointer/layout are the matching allocation pair required by
        // GlobalAlloc and are forwarded unchanged to the existing allocator.
        unsafe { host_play::memory::BENCHMARK_ALLOCATOR.dealloc(pointer, layout) };
        QUALIFICATION_RUST_LIVE_BYTES.fetch_sub(layout.size() as u64, Relaxed);
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        // SAFETY: pointer/layout are the matching allocation pair required by
        // GlobalAlloc and are forwarded unchanged to the existing allocator.
        let replacement =
            unsafe { host_play::memory::BENCHMARK_ALLOCATOR.realloc(pointer, layout, size) };
        if !replacement.is_null() {
            let old_size = layout.size() as u64;
            let new_size = size as u64;
            if new_size >= old_size {
                note_rust_allocation((new_size - old_size) as usize);
            } else {
                QUALIFICATION_RUST_LIVE_BYTES.fetch_sub(old_size - new_size, Relaxed);
            }
        }
        replacement
    }
}

#[cfg(feature = "memory-profile")]
fn note_rust_allocation(size: usize) {
    let live = QUALIFICATION_RUST_LIVE_BYTES
        .fetch_add(size as u64, Relaxed)
        .saturating_add(size as u64);
    QUALIFICATION_RUST_PEAK_BYTES.fetch_max(live, Relaxed);
}

#[cfg(feature = "memory-profile")]
fn current_rust_live_bytes() -> Option<u64> {
    Some(QUALIFICATION_RUST_LIVE_BYTES.load(Relaxed))
}

#[cfg(not(feature = "memory-profile"))]
fn current_rust_live_bytes() -> Option<u64> {
    None
}

#[cfg(feature = "memory-profile")]
fn reset_rust_peak_bytes() -> Option<u64> {
    let live = QUALIFICATION_RUST_LIVE_BYTES.load(Relaxed);
    QUALIFICATION_RUST_PEAK_BYTES.store(live, Relaxed);
    Some(live)
}

#[cfg(not(feature = "memory-profile"))]
fn reset_rust_peak_bytes() -> Option<u64> {
    None
}

#[cfg(feature = "memory-profile")]
fn current_rust_peak_bytes() -> Option<u64> {
    Some(QUALIFICATION_RUST_PEAK_BYTES.load(Relaxed))
}

#[cfg(not(feature = "memory-profile"))]
fn current_rust_peak_bytes() -> Option<u64> {
    None
}

#[cfg(feature = "memory-profile")]
fn rust_allocator_counts() -> Option<(u64, u64, u64)> {
    (!cfg!(feature = "memory-profile-no-alloc")).then(host_play::memory::rust_allocator_counts)
}

#[cfg(not(feature = "memory-profile"))]
fn rust_allocator_counts() -> Option<(u64, u64, u64)> {
    None
}

#[cfg(feature = "memory-profile")]
fn v8_heap_metrics() -> (u64, u64, Option<u64>, Option<u64>) {
    let samples = script_js::memory_profile::snapshots();
    let active = samples
        .iter()
        .filter(|sample| sample["v8_live"] == 1)
        .count() as u64;
    let sampled = samples
        .iter()
        .filter(|sample| {
            sample["v8_live"] == 1 && sample["v8_heap_samples"].as_u64().unwrap_or(0) > 0
        })
        .count() as u64;
    let sum = |key: &str| {
        samples
            .iter()
            .filter_map(|sample| sample[key].as_u64())
            .sum::<u64>()
    };
    let complete = active > 0 && sampled == active;
    (
        active,
        sampled,
        complete.then(|| sum("v8_used_bytes")),
        complete.then(|| sum("v8_total_bytes")),
    )
}

#[cfg(not(feature = "memory-profile"))]
fn v8_heap_metrics() -> (u64, u64, Option<u64>, Option<u64>) {
    (0, 0, None, None)
}

fn isolated_ca() -> (std::path::PathBuf, String) {
    use sha2::Digest;

    let home = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| fail("rss_ladder: isolated HOME is required"));
    let home = std::fs::canonicalize(home)
        .unwrap_or_else(|error| fail(&format!("rss_ladder: canonicalize HOME: {error}")));
    let ca = std::env::var_os("SSL_CERT_FILE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| fail("rss_ladder: SSL_CERT_FILE must name the isolated private CA"));
    let ca = std::fs::canonicalize(ca)
        .unwrap_or_else(|error| fail(&format!("rss_ladder: canonicalize SSL_CERT_FILE: {error}")));
    if !ca.starts_with(&home) || !ca.is_file() {
        fail("rss_ladder: SSL_CERT_FILE must be a file inside the isolated HOME");
    }
    let cert_dir = std::env::var_os("SSL_CERT_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| fail("rss_ladder: SSL_CERT_DIR must remain inside isolated HOME"));
    let cert_dir = std::fs::canonicalize(cert_dir)
        .unwrap_or_else(|error| fail(&format!("rss_ladder: canonicalize SSL_CERT_DIR: {error}")));
    if !cert_dir.starts_with(&home) {
        fail("rss_ladder: SSL_CERT_DIR must remain inside isolated HOME");
    }
    let ca_bytes = std::fs::read(&ca)
        .unwrap_or_else(|error| fail(&format!("rss_ladder: read private CA: {error}")));
    let ca_sha256 = format!("{:x}", sha2::Sha256::digest(&ca_bytes));

    #[cfg(feature = "soak-observers")]
    {
        use rustls::pki_types::{pem::PemObject, CertificateDer};

        let certificates = CertificateDer::pem_file_iter(&ca)
            .unwrap_or_else(|error| fail(&format!("rss_ladder: parse private CA: {error}")))
            .collect::<Result<Vec<_>, _>>()
            .unwrap_or_else(|error| {
                fail(&format!("rss_ladder: read private CA certificate: {error}"))
            });
        if certificates.len() != 1 {
            fail(&format!(
                "rss_ladder: private CA must contain exactly one certificate, got {}",
                certificates.len()
            ));
        }
        let config =
            client_adapter::tls_handshake::build_test_client_config_with_roots(&certificates)
                .unwrap_or_else(|error| {
                    fail(&format!(
                        "rss_ladder: build private-CA WSS verifier: {error}"
                    ))
                });
        client_adapter::service::NetworkService::install_test_tls_config(config).unwrap_or_else(
            |error| {
                fail(&format!(
                    "rss_ladder: install private-CA WSS verifier: {error}"
                ))
            },
        );
    }

    (ca, ca_sha256)
}

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
        play.maintain_services(Instant::now());
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
        thread::sleep(Duration::from_millis(100));
    }
}

// This executable must initialize protected V8 on process main, before Play
// creates any worker. A libtest worker cannot satisfy that production contract.
fn main() {
    if std::env::args().any(|arg| arg == "--list") {
        println!("live_rss_ladder_headless: test");
        println!("parse_rss_n_default_and_allowed: test");
        println!("parse_rss_n_rejects_bad: test");
        return;
    }
    parse_rss_n_default_and_allowed();
    parse_rss_n_rejects_bad();
    if live() {
        script_js::bootstrap_v8_platform().expect("process-main V8 bootstrap");
        live_rss_ladder_headless();
    } else {
        println!("PASS: rss_ladder argument checks; live measurement requires LIVE=1");
    }
}

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
    // Use the selected WSS profile override, not the legacy TCP PlayOptions
    // transport. The candidate's private root is installed before the process
    // network starts; origin/dev's native-tls path consumes the same SSL_CERT_FILE.
    let (_ca_path, ca_sha256) = isolated_ca();
    let profile_name = std::env::var("BOT_SERVER_PROFILE")
        .unwrap_or_else(|_| fail("rss_ladder: BOT_SERVER_PROFILE must select the WSS profile"));
    let home = std::path::PathBuf::from(std::env::var_os("HOME").expect("throwaway HOME"));
    let http_port = std::env::var("BOT_HTTP_PORT")
        .ok()
        .and_then(|port| port.parse().ok())
        .unwrap_or(2080);
    let selection = ProfileOptions {
        profile: Some(profile_name.clone()),
        revision: Some("289".into()),
        host: Some(opts.host.clone()),
        asset_host: Some(opts.host.clone()),
        port: Some(opts.port),
        http_port: Some(http_port),
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
    .unwrap_or_else(|error| fail(&format!("rss_ladder: resolve WSS profile: {error}")));
    if selection.profile_class() != host_play::ProfileClass::Remote
        || selection.transport() != client::Transport::Wss
        || selection.game_host() != opts.host.as_str()
        || selection.game_port() != opts.port
    {
        fail("rss_ladder: selected profile override is not the configured remote WSS endpoint");
    }
    // Accepted G2a caller ordering: start and retain the process network before
    // cache preparation; this is not a product-path workaround.
    #[cfg(feature = "soak-observers")]
    let _network_driver = host_play::PlayComposition::start().unwrap_or_else(|error| {
        fail(&format!(
            "rss_ladder: start process network before WSS cache preparation: {error}"
        ))
    });
    let template = selection
        .prepare_template()
        .unwrap_or_else(|error| fail(&format!("rss_ladder: prepare WSS template: {error}")));
    let profiles = ladder_profiles(n);
    let allocation_baseline = rust_allocator_counts();
    let rust_live_baseline = current_rust_live_bytes();
    reset_rust_peak_bytes();

    let play = run_with_template(
        template,
        false,
        profiles,
        |_| (None, None),
        |c, _, _hold| c.set_draw(false),
    )
    .expect("start checked Play");
    wait_all_scene2(&play, n);
    let load = std::env::var("RSS_LOAD").as_deref() == Ok("1");
    #[cfg(feature = "js")]
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
            play.maintain_services(Instant::now());
            if Instant::now() >= until {
                fail(&format!(
                    "rss_ladder: {n} Load scripts did not all reach Running"
                ));
            }
            thread::sleep(Duration::from_millis(50));
        }
    }
    #[cfg(not(feature = "js"))]
    if load {
        fail("RSS_LOAD requires host-play's `js` feature");
    }
    // Give the wall a beat to settle before sampling peak RSS and the
    // independent Rust/V8 domains.
    common::maintain_for(&play, Duration::from_secs(10));
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
    let tcp_s = tcp
        .map(|count| count.to_string())
        .unwrap_or_else(|| "?".into());
    let current = host_play::current_resident_bytes();
    let current_s = current
        .map(|value| value.to_string())
        .unwrap_or_else(|| "?".into());
    let fds = fd_count();
    let fds_s = fds
        .map(|count| count.to_string())
        .unwrap_or_else(|| "?".into());
    let (tasks, tasks_hist) = task_census()
        .map(|(total, hist)| (Some(total), format!(" tasks_hist=[{hist}]")))
        .unwrap_or_else(|| (None, String::new()));
    let tasks_s = tasks
        .map(|total| total.to_string())
        .unwrap_or_else(|| "?".into());
    let allocation_after = rust_allocator_counts();
    let allocation_delta = allocation_after
        .zip(allocation_baseline)
        .map(|(after, before)| {
            (
                after.0.saturating_sub(before.0),
                after.1.saturating_sub(before.1),
            )
        });
    let rust_live_after = current_rust_live_bytes();
    let rust_peak = current_rust_peak_bytes();
    let rust_peak_delta = rust_peak
        .zip(rust_live_baseline)
        .map(|(peak, baseline)| peak.saturating_sub(baseline));
    let (v8_active_isolates, v8_sampled_isolates, v8_used_bytes, v8_total_bytes) =
        v8_heap_metrics();
    let ready_slots = play
        .statuses()
        .iter()
        .filter(|status| status.ingame && status.scene_state == 2)
        .count();
    let receipt = serde_json::json!({
        "schema": "t1.4.g5e-memory-ladder.v1",
        "n": n,
        "load": load,
        "ready_slots": ready_slots,
        "profile": profile_name,
        "profile_class": "remote",
        "transport": "wss",
        "game_host": opts.host,
        "game_port": opts.port,
        "asset_host": opts.host,
        "http_port": http_port,
        "private_ca_sha256": ca_sha256,
        "rss_peak_bytes": rss,
        "resident_current_bytes": current,
        "rust_allocations_since_play": allocation_delta.map(|delta| delta.0),
        "rust_allocated_bytes_since_play": allocation_delta.map(|delta| delta.1),
        "rust_live_bytes": rust_live_after,
        "rust_live_bytes_before_play": rust_live_baseline,
        "rust_live_peak_bytes": rust_peak,
        "rust_live_peak_delta_bytes": rust_peak_delta,
        "v8_active_isolates": v8_active_isolates,
        "v8_sampled_isolates": v8_sampled_isolates,
        "v8_used_bytes": v8_used_bytes,
        "v8_total_bytes": v8_total_bytes,
        "ondemand_workers": workers,
        "wss_tcp_connections": tcp,
        "tasks": tasks,
        "fds": fds,
        "bytes_in": bytes_in,
        "bytes_out": bytes_out,
        "allocation_counting": allocation_after.is_some(),
        "wss_private_ca_installed": true,
        "measurement_scope": "whole process; per-WSS slope is calculated by the matched N ladder",
    });
    println!(
        "G5E_MEMORY_RECEIPT {}",
        serde_json::to_string(&receipt).expect("serialize memory ladder receipt")
    );
    println!(
        "rss_ladder n={n} load={load} rss={rss} current={current_s} ondemand={workers} tcp={tcp_s} tasks={tasks_s}{tasks_hist} fds={fds_s} bytes_in={bytes_in} bytes_out={bytes_out}"
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
    println!("PASS: rss_ladder n={n} load={load} rss={rss} current={current_s} ondemand={workers} tcp={tcp_s} tasks={tasks_s} fds={fds_s}");
}

fn parse_rss_n_default_and_allowed() {
    assert_eq!(parse_rss_n_from(None).unwrap(), 1);
    assert_eq!(parse_rss_n_from(Some("1")).unwrap(), 1);
    assert_eq!(parse_rss_n_from(Some("2")).unwrap(), 2);
    assert_eq!(parse_rss_n_from(Some("4")).unwrap(), 4);
    assert_eq!(parse_rss_n_from(Some("10")).unwrap(), 10);
}

fn parse_rss_n_rejects_bad() {
    let err = "rss_ladder: RSS_N must be 1, 2, 4, or 10";
    assert_eq!(parse_rss_n_from(Some("")).unwrap_err(), err);
    assert_eq!(parse_rss_n_from(Some("3")).unwrap_err(), err);
    assert_eq!(parse_rss_n_from(Some("50")).unwrap_err(), err);
    assert_eq!(parse_rss_n_from(Some("one")).unwrap_err(), err);
}
