//! Meter values over a real [`Play`] (no server contact): every value is
//! measured, still measuring, not measurable here, or failed — never a
//! fabricated zero.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use host_play::{Play, PlayOptions, SlotArm, SlotStatus};

use super::*;

fn empty_play() -> Play {
    host_play::run_with_io(
        &PlayOptions {
            host: "127.0.0.1".into(),
            port: 43594,
            cache_dir: "/tmp".into(),
            lowmem: true,
            mainland: false,
        },
        vec![],
        |_| (None, None),
        |_, _, _| {},
    )
}

fn unsupported() -> ProcessProbe {
    ProcessProbe::Unsupported
}

fn failing() -> ProcessProbe {
    ProcessProbe::Failed
}

static CPU_READS: AtomicUsize = AtomicUsize::new(0);

/// Half a CPU second per read; no current-resident figure.
fn peak_only() -> ProcessProbe {
    let n = CPU_READS.fetch_add(1, Ordering::SeqCst) + 1;
    ProcessProbe::Sampled {
        cpu_seconds: n as f64 * 0.5,
        resident: None,
        peak: 3 << 30,
    }
}

fn live(play: &mut Play, name: &str, bytes: u64) {
    if play.arm(name).is_none() {
        play.attach_arm(name, SlotArm::new(1, true));
    }
    let mut rows = play.statuses.lock().unwrap();
    match rows.iter_mut().find(|row| row.username == name) {
        Some(row) => row.bytes_in = bytes,
        None => rows.push(SlotStatus {
            username: name.into(),
            bytes_in: bytes,
            ..SlotStatus::default()
        }),
    }
}

#[test]
fn a_platform_without_a_sampler_and_a_failed_read_say_so() {
    let now = Instant::now();
    let mut meter = Resources::with_probe(unsupported);
    meter.poll(now, None, None);
    let view = meter.view();
    assert_eq!(view.cpu, Metric::Unavailable(NOT_MEASURED_HERE));
    assert_eq!(view.ram, Metric::Unavailable(NOT_MEASURED_HERE));
    assert_eq!(
        view.traffic,
        Metric::Unavailable(NO_LIVE_SLOTS),
        "no workers: no rate to measure, not 0 B/s"
    );

    let mut meter = Resources::with_probe(failing);
    meter.poll(now, None, None);
    assert_eq!(meter.view().cpu, Metric::Error(PROBE_FAILED.into()));
    assert_eq!(meter.view().ram, Metric::Error(PROBE_FAILED.into()));
}

#[test]
fn cpu_measures_after_two_reads_and_memory_names_its_definition() {
    let start = Instant::now();
    let mut meter = Resources::with_probe(peak_only);
    meter.poll(start, None, None);
    assert_eq!(meter.view().cpu, Metric::Measuring);
    assert_eq!(
        meter.view().ram,
        Metric::Available("peak 3.00 GB process".into())
    );
    meter.poll(start + Duration::from_millis(500), None, None);
    assert_eq!(meter.view().cpu, Metric::Measuring, "not due yet");
    meter.poll(start + Duration::from_secs(1), None, None);
    match &meter.view().cpu {
        Metric::Available(text) => assert!(text.starts_with("0.5 cores"), "{text}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn traffic_is_a_rate_over_a_stable_set_of_live_workers() {
    let mut play = empty_play();
    let start = Instant::now();
    let at = |secs: u64| start + Duration::from_secs(secs);
    let mut meter = Resources::with_probe(unsupported);
    live(&mut play, "alice", 1_000);
    meter.poll(at(0), Some(&play), Some("alice"));
    assert_eq!(meter.view().traffic, Metric::Measuring, "one sample");
    assert_eq!((meter.view().bots, meter.view().background), (1, 0));

    live(&mut play, "alice", 3_048);
    meter.poll(at(1), Some(&play), Some("alice"));
    assert_eq!(meter.view().traffic, Metric::Available("2.0 KB/s".into()));

    live(&mut play, "bob", 0);
    meter.poll(at(2), Some(&play), Some("alice"));
    assert_eq!(
        meter.view().traffic,
        Metric::Measuring,
        "a worker joined: re-baseline"
    );
    assert_eq!((meter.view().bots, meter.view().background), (2, 1));

    live(&mut play, "alice", 0);
    meter.poll(at(3), Some(&play), Some("alice"));
    assert_eq!(
        meter.view().traffic,
        Metric::Measuring,
        "a counter restarted with its session: re-baseline"
    );
    live(&mut play, "bob", 512);
    meter.poll(at(4), Some(&play), Some("alice"));
    assert_eq!(meter.view().traffic, Metric::Available("512 B/s".into()));
}

/// On the platforms with a sampler, this process's own read yields CPU
/// time, a peak and a current resident size.
#[cfg(any(target_os = "macos", target_os = "linux", windows))]
#[test]
fn this_process_reads_its_own_counters() {
    match probe_process() {
        ProcessProbe::Sampled {
            cpu_seconds,
            resident,
            peak,
        } => {
            assert!(cpu_seconds >= 0.0 && peak > 0);
            assert!(resident.is_some_and(|r| r > 0), "{resident:?}");
        }
        other => panic!("{other:?}"),
    }
}
