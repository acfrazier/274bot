//! WalkTo map-cache footprint receipt: bake a demand against a scratch cache
//! root and sample the process physical footprint and malloc statistics
//! before the open, during the bake, at Ready and after the close.
//!
//! cargo run --release -p host-play --example map_footprint -- \
//!   <shared profile flags> --cache-root DIR [--catalogue] [--cycles N] [--pause]
//!   [--interrupt-after SECS]
//!
//! `--cache-root` must be a scratch directory (an empty one is a cold bake;
//! cycles after the first reopen the published cache, i.e. warm opens); the
//! operator `~/.274bot/map-cache` is never touched. `--catalogue` measures
//! the TUI's catalogue-only demand. `--interrupt-after` first closes a bake
//! after SECS, so the first cycle measures the resume. `--pause` stops after
//! the baseline and after each close until a line arrives on stdin, so an
//! outside tool (`footprint -p`, `vmmap --summary`) can snapshot the
//! process. One JSON line per sample on stdout. macOS reports
//! `proc_pid_rusage` v4 physical footprint and `malloc_zone_statistics`;
//! other platforms print RSS only.
use host_play::map_cache::{MapCacheRoot, MapDemand, MapDemandManager, MapJobStatus};
use host_play::{map_profile_descriptor, NativeMapProducer};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Default)]
struct Sample {
    footprint: u64,
    malloc_in_use: u64,
    malloc_allocated: u64,
}

#[cfg(target_os = "macos")]
fn sample() -> Sample {
    unsafe {
        let mut info: libc::rusage_info_v4 = std::mem::zeroed();
        let rc = libc::proc_pid_rusage(
            libc::getpid(),
            libc::RUSAGE_INFO_V4,
            &mut info as *mut libc::rusage_info_v4 as *mut libc::rusage_info_t,
        );
        let mut stats: libc::malloc_statistics_t = std::mem::zeroed();
        // A null zone sums every registered zone.
        libc::malloc_zone_statistics(std::ptr::null_mut(), &mut stats);
        Sample {
            footprint: if rc == 0 { info.ri_phys_footprint } else { 0 },
            malloc_in_use: stats.size_in_use as u64,
            malloc_allocated: stats.size_allocated as u64,
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn sample() -> Sample {
    let rss = std::fs::read_to_string("/proc/self/statm")
        .ok()
        .and_then(|s| s.split_whitespace().nth(1)?.parse::<u64>().ok())
        .map(|pages| pages * 4096)
        .unwrap_or(0);
    Sample {
        footprint: rss,
        ..Sample::default()
    }
}

fn mib(bytes: i128) -> f64 {
    bytes as f64 / 1_048_576.0
}

/// Sample until the footprint moves less than 0.5 MiB over two seconds
/// (bounded at 20 s), so deferred frees don't read as a bake delta.
fn settled() -> Sample {
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut window = vec![sample()];
    loop {
        std::thread::sleep(Duration::from_millis(250));
        window.push(sample());
        if window.len() > 8 {
            window.remove(0);
        }
        let min = window.iter().map(|s| s.footprint).min().unwrap_or(0);
        let max = window.iter().map(|s| s.footprint).max().unwrap_or(0);
        if (window.len() == 8 && max - min < 512 * 1024) || Instant::now() > deadline {
            return *window.last().unwrap_or(&Sample::default());
        }
    }
}

fn emit(phase: &str, base: Sample, now: Sample, extra: &str) {
    println!(
        "{{\"phase\":\"{phase}\",\"footprint_mib\":{:.2},\"footprint_delta_mib\":{:.2},\"malloc_in_use_delta_bytes\":{},\"malloc_allocated_delta_mib\":{:.2}{extra}}}",
        mib(i128::from(now.footprint)),
        mib(i128::from(now.footprint) - i128::from(base.footprint)),
        i128::from(now.malloc_in_use) - i128::from(base.malloc_in_use),
        mib(i128::from(now.malloc_allocated) - i128::from(base.malloc_allocated)),
    );
}

fn run() -> Result<(), String> {
    let (options, rest) = host_play::parse_profile_args(std::env::args().skip(1))?;
    let mut cache_root = None;
    let mut demand = MapDemand::Images;
    let mut cycles = 1u32;
    let mut pauses = false;
    let mut interrupt = None;
    let mut rest = rest.into_iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--cache-root" => cache_root = rest.next().map(PathBuf::from),
            "--catalogue" => demand = MapDemand::CatalogueOnly,
            "--pause" => pauses = true,
            "--interrupt-after" => {
                interrupt = Some(Duration::from_secs_f64(
                    rest.next()
                        .and_then(|n| n.parse().ok())
                        .ok_or("--interrupt-after needs seconds")?,
                ))
            }
            "--cycles" => {
                cycles = rest
                    .next()
                    .and_then(|n| n.parse().ok())
                    .ok_or("--cycles needs a count")?
            }
            other => return Err(format!("unknown argument {other}")),
        }
    }
    let cache_root = cache_root.ok_or("--cache-root DIR is required (scratch directory)")?;
    let profile = options.resolve(None)?.bind_runtime()?;
    let descriptor = map_profile_descriptor(&profile).map_err(|e| e.to_string())?;
    let manager = MapDemandManager::new(
        MapCacheRoot::from_root(cache_root),
        Arc::new(NativeMapProducer::new()),
    );
    let pause = |phase: &str| {
        if pauses {
            eprintln!("map_footprint: paused {phase} pid {}", std::process::id());
            let _ = std::io::stdin().read_line(&mut String::new());
        }
    };
    // stdout allocates its line buffer on the first print; do that before the
    // baseline so it doesn't read as a map allocation.
    println!("{{\"phase\":\"start\",\"pid\":{}}}", std::process::id());
    let base = settled();
    emit("baseline", base, base, "");
    pause("baseline");
    if let Some(after) = interrupt {
        // A close mid-bake: drop the only consumer, which cancels the job;
        // the first cycle below then measures the resume.
        let handle = manager
            .request(descriptor.clone(), demand)
            .map_err(|e| e.to_string())?;
        std::thread::sleep(after);
        let status = format!("{:?}", handle.status());
        drop(handle);
        std::thread::sleep(Duration::from_secs(1));
        manager.reap();
        emit(
            "interrupted",
            base,
            sample(),
            &format!(",\"status_at_close\":{status:?}"),
        );
    }
    for cycle in 0..cycles {
        let started = Instant::now();
        let handle = manager
            .request(descriptor.clone(), demand)
            .map_err(|e| e.to_string())?;
        let mut peak = base;
        loop {
            let now = sample();
            if now.footprint > peak.footprint {
                peak = now;
            }
            match handle.status() {
                MapJobStatus::Ready => break,
                MapJobStatus::Failed(message) => return Err(message),
                MapJobStatus::Paused | MapJobStatus::Cancelled => return Err("bake stopped".into()),
                MapJobStatus::Queued | MapJobStatus::Running(_) => {}
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let elapsed = started.elapsed().as_secs_f64();
        emit(
            "peak",
            base,
            peak,
            &format!(",\"cycle\":{cycle},\"elapsed_s\":{elapsed:.2}"),
        );
        let ready = handle.ready().map_err(|e| e.to_string())?;
        emit("ready", base, sample(), &format!(",\"cycle\":{cycle}"));
        drop(ready);
        drop(handle);
        manager.reap();
        std::thread::sleep(Duration::from_secs(1));
        manager.reap();
        emit("closed", base, settled(), &format!(",\"cycle\":{cycle}"));
        pause("closed");
    }
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("map_footprint: {error}");
        std::process::exit(2);
    }
}
