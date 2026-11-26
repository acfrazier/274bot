//! Memory and startup gates of slice L7 (ADR 0023 item 32, Promotion
//! Condition 2) for the stated workload of `support/l7_workload.luau`.
//!
//! The heap gate drives one product run through 1,010 trips (20,200 ticks)
//! of the workload and bounds the VM heap, the Rust live bytes of the run
//! (mlua takes the VM heap from the Rust allocator,
//! `mlua-0.12.2/src/memory.rs:1-5`) and what a Stop leaves behind. The start
//! gate bounds the cost of one Start: the manifest read, which makes no VM,
//! and the spawn, which compiles the source, makes the VM and runs the load
//! entry.
//!
//! Each limit is a measured value plus the headroom of the project memory
//! rules: a heap total gets 5 %, a total above 1 MiB gets `min(1 %, 64 KiB)`,
//! both rounded up to 8 bytes. A count of allocations gets 5 %, at least 8.

pub mod support;
#[path = "support/workload.rs"]
pub mod workload;

use std::time::{Duration, Instant};

use script::surface::SurfaceRun;
use script_luau::read_manifest;
use support::{spawn_with, stop};
use workload::{used_bytes, Answered, Driver, ENTRIES_PER_TRIP, TICKS_PER_TRIP};

/// The stop threshold of the VM (ADR 0023 item 12).
const SOFT_CAP_BYTES: u64 = 16 * 1024 * 1024;
const WARM_UP_TRIPS: u64 = 10;
const TRIPS: u64 = 1_000;

fn start() -> Box<dyn SurfaceRun> {
    spawn_with(workload::SOURCE, Some(workload::settings())).expect("the workload starts")
}

/// The measured value plus the heap-total headroom of the project memory
/// rules: 5 %, rounded up to 8 bytes. These are heaps of one run. The peak
/// values also depend on the clock: the Luau collector sets its next trigger
/// from the measured allocation rate and mark time
/// (`luau0-src-0.22.0+luau740`, `VM/src/lgc.cpp:1281-1297`), so the peak of
/// one run differs from the next. Each measured value below is the highest
/// of 20 runs, 10 on each platform.
const fn with_heap_headroom(measured: u64) -> u64 {
    (measured + measured.div_ceil(20)).div_ceil(8) * 8
}

// Measured on 2026-11-26 on merlot (macOS 27, arm64) and chardonnay (Linux
// x86_64), debug test builds; the L7 report has the runs.
/// VM used bytes after the load entry and its full collection (the same in
/// each run).
const LOAD_USED_MEASURED: u64 = 888_088;
/// The highest VM used bytes after any of the 20,200 ticks (1,150,928 to
/// 1,201,040).
const PEAK_USED_MEASURED: u64 = 1_201_040;
/// Rust live bytes of a started run, the VM heap included (macOS 895,550,
/// Linux 895,390).
const SPAWN_LIVE_MEASURED: u64 = 895_550;
/// The highest Rust live bytes of the run in the 20,200 ticks (1,168,970 to
/// 1,219,537).
const RUN_PEAK_MEASURED: u64 = 1_219_537;
/// Rust allocations of one spawn (macOS 472, Linux 470).
const SPAWN_ALLOCATIONS_MEASURED: u64 = 472;
/// The peak Rust bytes of a manifest read (the same on both).
const MANIFEST_PEAK_MEASURED: u64 = 8_430;
/// The median spawn took 1.5 to 3.1 ms (debug test build). The limit is
/// the load deadline of the product (50 ms, ADR 0023 operator answer 2):
/// twenty times the measured value, so a slow two-core runner passes and a
/// spawn that has become as slow as the deadline that it feeds fails.
const SPAWN_MEDIAN_LIMIT: Duration = Duration::from_millis(50);

#[test]
fn stated_workload_heap_stays_bounded_far_below_the_cap() {
    let mut started = None;
    let spawn = allocation_counter::measure(|| started = Some(start()));
    let mut run = started.expect("spawned run");
    let load_used = used_bytes(&*run);

    let mut driver = Driver::new();
    let mut warm_used = 0;
    let mut peak_used = load_used;
    let mut low_used = u64::MAX;
    let drive = allocation_counter::measure(|| {
        for _ in 0..WARM_UP_TRIPS * TICKS_PER_TRIP {
            driver.step(&mut *run);
            peak_used = peak_used.max(used_bytes(&*run));
        }
        warm_used = used_bytes(&*run);
        for _ in 0..TRIPS * TICKS_PER_TRIP {
            driver.step(&mut *run);
            let used = used_bytes(&*run);
            peak_used = peak_used.max(used);
            low_used = low_used.min(used);
        }
    });
    let end_used = used_bytes(&*run);
    let answered = driver.answered;
    let ticks = (WARM_UP_TRIPS + TRIPS) * TICKS_PER_TRIP;

    let teardown = allocation_counter::measure(|| drop(stop(run)));
    let spawn_live = u64::try_from(spawn.bytes_current).expect("a started run holds bytes");
    let run_live = u64::try_from(spawn.bytes_current + drive.bytes_current)
        .expect("a running run holds bytes");
    let run_peak = spawn_live + drive.bytes_max;
    println!(
        "L7_HEAP {}",
        serde_json::json!({
            "ticks": ticks,
            "answered": format!("{answered:?}"),
            "vm_used_after_load": load_used,
            "vm_used_after_warm_up": warm_used,
            "vm_used_low": low_used,
            "vm_used_peak": peak_used,
            "vm_used_end": end_used,
            "soft_cap": SOFT_CAP_BYTES,
            "peak_share_of_cap": peak_used as f64 / SOFT_CAP_BYTES as f64,
            "rust_live_after_spawn": spawn.bytes_current,
            "rust_peak_in_spawn": spawn.bytes_max,
            "rust_live_change_in_drive": drive.bytes_current,
            "rust_peak_in_drive": drive.bytes_max,
            "rust_allocations_in_drive": drive.count_total,
            "rust_live_of_run": run_live,
            "rust_peak_of_run": run_peak,
            "rust_live_change_in_stop": teardown.bytes_current,
        })
    );

    // The driver took the script through every phase of each trip.
    let trips = ticks / TICKS_PER_TRIP;
    assert_eq!(
        answered,
        Answered {
            walks: 4 * trips,
            fights: 4 * trips,
            banks: 3 * trips,
            pickups: 4 * trips,
            logs: trips * ENTRIES_PER_TRIP / 100,
        },
        "the workload went through each phase of each trip"
    );

    let load_limit = with_heap_headroom(LOAD_USED_MEASURED);
    assert!(
        load_used <= load_limit,
        "VM used bytes after load {load_used} > {load_limit}"
    );
    let peak_limit = with_heap_headroom(PEAK_USED_MEASURED);
    assert!(
        peak_used <= peak_limit,
        "VM used bytes peak {peak_used} > {peak_limit} over {ticks} ticks"
    );
    // The workload peaks at 7 % of the cap: the script keeps 7/8 of the cap
    // at least for its own data.
    assert!(peak_used * 8 <= SOFT_CAP_BYTES);
    let spawn_limit = with_heap_headroom(SPAWN_LIVE_MEASURED);
    assert!(
        spawn_live <= spawn_limit,
        "Rust live bytes of a started run {spawn_live} > {spawn_limit}"
    );
    let run_limit = with_heap_headroom(RUN_PEAK_MEASURED);
    assert!(
        run_peak <= run_limit,
        "Rust live bytes of a run peaked at {run_peak} > {run_limit} in {ticks} ticks"
    );
    // A Stop returns every byte that the spawn and the ticks kept.
    assert_eq!(
        teardown.bytes_current,
        -(run_live as i64),
        "Stop left {} bytes",
        run_live as i64 + teardown.bytes_current
    );
}

#[test]
fn stated_workload_start_cost_stays_within_the_measured_bound() {
    // The picker reads the manifest without a VM: its peak is a small part
    // of a VM.
    let manifest = allocation_counter::measure(|| {
        let read = read_manifest(workload::SOURCE).expect("workload manifest");
        assert_eq!(read.manifest().name(), "L7 Cow Fighter");
    });

    let mut started = None;
    let spawn = allocation_counter::measure(|| started = Some(start()));
    drop(started);

    let mut samples: Vec<Duration> = (0..15)
        .map(|_| {
            let begin = Instant::now();
            let run = start();
            let elapsed = begin.elapsed();
            drop(run);
            elapsed
        })
        .collect();
    samples.sort_unstable();
    let median = samples[samples.len() / 2];
    println!(
        "L7_START {}",
        serde_json::json!({
            "manifest_rust_peak": manifest.bytes_max,
            "manifest_rust_allocations": manifest.count_total,
            "spawn_rust_peak": spawn.bytes_max,
            "spawn_rust_live": spawn.bytes_current,
            "spawn_rust_allocations": spawn.count_total,
            "spawn_ms_min": samples[0].as_secs_f64() * 1e3,
            "spawn_ms_median": median.as_secs_f64() * 1e3,
            "spawn_ms_max": samples[samples.len() - 1].as_secs_f64() * 1e3,
        })
    );

    let manifest_limit = with_heap_headroom(MANIFEST_PEAK_MEASURED);
    assert!(
        manifest.bytes_max <= manifest_limit,
        "manifest read peak {} > {manifest_limit}",
        manifest.bytes_max
    );
    assert!(
        manifest.bytes_max * 16 <= spawn.bytes_max,
        "the manifest read is not a VM"
    );
    let allocation_limit =
        SPAWN_ALLOCATIONS_MEASURED + SPAWN_ALLOCATIONS_MEASURED.div_ceil(20).max(8);
    assert!(
        spawn.count_total <= allocation_limit,
        "a spawn made {} Rust allocations > {allocation_limit}",
        spawn.count_total
    );
    assert!(
        median <= SPAWN_MEDIAN_LIMIT,
        "median spawn {median:?} > {SPAWN_MEDIAN_LIMIT:?}"
    );
}

/// QS12: one Start from a file, timed by phase, in a new process. The L7
/// wrapper runs it with the file and this test binary in and out of the page
/// cache of the storage under test, so the first Start of a process pages in
/// the Luau compiler and VM code from that storage.
#[test]
#[ignore = "QS12 measurement cell; the L7 wrapper runs it once per process and reads its JSON line"]
fn qs12_first_start_from_file() {
    let path = std::env::var_os("L7_LUAU_FILE").expect("L7_LUAU_FILE names the workload file");
    let begin = Instant::now();
    let source = std::fs::read_to_string(path).expect("read the workload file");
    let read = begin.elapsed();
    let manifest_begin = Instant::now();
    let manifest = read_manifest(&source).expect("workload manifest");
    let manifest_time = manifest_begin.elapsed();
    assert_eq!(manifest.manifest().name(), "L7 Cow Fighter");
    let spawn_begin = Instant::now();
    let mut run = spawn_with(&source, Some(workload::settings())).expect("the workload starts");
    let spawn_time = spawn_begin.elapsed();
    let tick_begin = Instant::now();
    Driver::new().step(&mut *run);
    let tick_time = tick_begin.elapsed();
    let first_tick_done = begin.elapsed();
    let second_begin = Instant::now();
    let second = spawn_with(&source, Some(workload::settings())).expect("second start");
    let second_spawn = second_begin.elapsed();
    drop(second);
    println!(
        "QS12 {}",
        serde_json::json!({
            "bytes": source.len(),
            "read_us": read.as_secs_f64() * 1e6,
            "manifest_us": manifest_time.as_secs_f64() * 1e6,
            "spawn_us": spawn_time.as_secs_f64() * 1e6,
            "first_tick_us": tick_time.as_secs_f64() * 1e6,
            "start_to_first_tick_us": first_tick_done.as_secs_f64() * 1e6,
            "second_spawn_us": second_spawn.as_secs_f64() * 1e6,
        })
    );
}
