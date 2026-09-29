//! Meter values over a real [`Play`] (no server contact): every value is
//! measured, still measuring, not measurable here, or failed — never a
//! fabricated zero or a rate across different workers — and the OS probe
//! never runs on the polling thread.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread::ThreadId;
use std::time::{Duration, Instant};

use host_play::{Play, PlayOptions, SlotArm, SlotStatus};

use super::*;

fn empty_play() -> Play {
    host_play::run_with_io(
        &PlayOptions {
            host: "127.0.0.1".into(),
            transport: host_play::Transport::Tcp,
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

/// Keep polling at `at` (so no new sample is due) until the probe thread's
/// result shows, or fail after a few seconds.
fn settle(
    meter: &mut Resources,
    at: Instant,
    play: Option<&Play>,
    done: impl Fn(&ResourceView) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        meter.poll(at, play, None);
        if done(meter.view()) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "no probe result: {:?}",
            meter.view()
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn unsupported() -> ProcessProbe {
    ProcessProbe::Unsupported
}

fn failing() -> ProcessProbe {
    ProcessProbe::Failed
}

#[test]
fn a_platform_without_a_sampler_and_a_failed_read_say_so() {
    let now = Instant::now();
    let mut meter = Resources::with_probe(unsupported);
    settle(&mut meter, now, None, |view| view.cpu != Metric::Measuring);
    let view = meter.view();
    assert_eq!(view.cpu, Metric::Unavailable(NOT_MEASURED_HERE));
    assert_eq!(view.ram, Metric::Unavailable(NOT_MEASURED_HERE));
    assert_eq!(
        view.traffic,
        Metric::Unavailable(NO_LIVE_SLOTS),
        "no workers: no rate to measure, not 0 B/s"
    );
    assert_eq!(view.brief, "cpu - ram - net -", "the header says so too");

    let mut meter = Resources::with_probe(failing);
    settle(&mut meter, now, None, |view| view.cpu != Metric::Measuring);
    assert_eq!(meter.view().cpu, Metric::Error(PROBE_FAILED.into()));
    assert_eq!(meter.view().ram, Metric::Error(PROBE_FAILED.into()));
    assert_eq!(meter.view().brief, "cpu err ram err net -");
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

#[test]
fn cpu_measures_after_two_reads_and_memory_names_its_definition() {
    let start = Instant::now();
    let mut meter = Resources::with_probe(peak_only);
    settle(&mut meter, start, None, |view| {
        view.ram != Metric::Measuring
    });
    assert_eq!(
        meter.view().cpu,
        Metric::Measuring,
        "a rate needs two reads"
    );
    assert_eq!(
        meter.view().ram,
        Metric::Available("peak 3.00 GB process".into())
    );
    assert_eq!(meter.view().brief, "cpu … ram peak 3.00 GB net -");
    meter.poll(start + Duration::from_millis(500), None, None);
    assert_eq!(CPU_READS.load(Ordering::SeqCst), 1, "not due yet");
    settle(&mut meter, start + Duration::from_secs(1), None, |view| {
        view.cpu != Metric::Measuring
    });
    assert!(matches!(meter.view().cpu, Metric::Available(_)));
    let brief = &meter.view().brief;
    assert!(
        brief.starts_with("cpu ") && brief.ends_with("% ram peak 3.00 GB net -"),
        "the header shows the CPU share: {brief}"
    );
    // Half a CPU second over one wall second is half a core.
    assert_eq!(
        cpu_from_delta(0.5, 1.0, 4),
        Some(("0.5 cores (12% of 4)".into(), "12%".into()))
    );
}

/// Read after the peak, the current size can come out above it; the peak
/// shown never does.
fn grown_after_peak() -> ProcessProbe {
    ProcessProbe::Sampled {
        cpu_seconds: 1.0,
        resident: Some(365 << 20),
        peak: 364 << 20,
    }
}

#[test]
fn the_peak_shown_is_never_below_the_current_size() {
    let mut meter = Resources::with_probe(grown_after_peak);
    settle(&mut meter, Instant::now(), None, |view| {
        view.ram != Metric::Measuring
    });
    assert_eq!(
        meter.view().ram,
        Metric::Available("365.0 MB process, peak 365.0 MB".into())
    );
}

/// Publish `name`'s stream counter, starting its worker when it has none.
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

/// A rate is only taken over the same workers and streams: a replaced
/// worker, a restarted counter or a new stream re-baseline, even when the
/// number of workers or the summed bytes would hide it.
#[test]
fn traffic_is_a_rate_over_the_same_workers_and_streams() {
    let mut play = empty_play();
    let start = Instant::now();
    let at = |secs: u64| start + Duration::from_secs(secs);
    let mut meter = Resources::with_probe(unsupported);
    let sample = |meter: &mut Resources, play: &Play, secs| {
        meter.poll(at(secs), Some(play), Some("alice"));
        meter.view().traffic.clone()
    };
    live(&mut play, "alice", 1_000);
    live(&mut play, "bob", 1_000);
    assert_eq!(
        sample(&mut meter, &play, 0),
        Metric::Measuring,
        "one sample"
    );
    assert_eq!((meter.view().bots, meter.view().background), (2, 1));

    live(&mut play, "alice", 2_024);
    live(&mut play, "bob", 2_024);
    assert_eq!(
        sample(&mut meter, &play, 1),
        Metric::Available("2.0 KB/s".into())
    );

    // bob's worker is replaced; the count stays two.
    play.attach_arm("bob", SlotArm::new(2, true));
    live(&mut play, "bob", 9_000);
    assert_eq!(sample(&mut meter, &play, 2), Metric::Measuring);
    assert_eq!(meter.view().bots, 2);
    live(&mut play, "alice", 2_536);
    live(&mut play, "bob", 9_000);
    assert_eq!(
        sample(&mut meter, &play, 3),
        Metric::Available("512 B/s".into())
    );

    // alice's counter restarts while bob grows more: the sum still grows.
    live(&mut play, "alice", 0);
    live(&mut play, "bob", 20_000);
    assert_eq!(sample(&mut meter, &play, 4), Metric::Measuring);

    // alice's session changed and its new stream already passed the old
    // count before the sample: the stream, not the count, tells.
    live(&mut play, "alice", 100);
    live(&mut play, "bob", 20_000);
    assert_eq!(
        sample(&mut meter, &play, 5),
        Metric::Available("100 B/s".into())
    );
    {
        let mut rows = play.statuses.lock().unwrap();
        let alice = rows.iter_mut().find(|row| row.username == "alice").unwrap();
        alice.stream_epoch += 1;
        alice.bytes_in = 5_000;
    }
    assert_eq!(sample(&mut meter, &play, 6), Metric::Measuring);
}

static PROBE_THREAD: parking_lot::Mutex<Option<ThreadId>> = parking_lot::Mutex::new(None);
static GATE_OPEN: AtomicBool = AtomicBool::new(false);

/// Records its thread, then holds until the test opens the gate (bounded,
/// so a failure cannot hang the suite).
fn gated() -> ProcessProbe {
    *PROBE_THREAD.lock() = Some(std::thread::current().id());
    let deadline = Instant::now() + Duration::from_secs(10);
    while !GATE_OPEN.load(Ordering::SeqCst) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(1));
    }
    ProcessProbe::Sampled {
        cpu_seconds: 2.0,
        resident: Some(10 << 20),
        peak: 20 << 20,
    }
}

/// A poll that starts a sample returns while the probe is still running
/// (on the meter's own thread); a later poll picks the result up.
#[test]
fn the_probe_runs_off_the_polling_thread() {
    let mut meter = Resources::with_probe(gated);
    let now = Instant::now();
    meter.poll(now, None, None);
    let deadline = Instant::now() + Duration::from_secs(5);
    while PROBE_THREAD.lock().is_none() {
        assert!(Instant::now() < deadline, "the probe never started");
        std::thread::sleep(Duration::from_millis(1));
    }
    // The probe is blocked on the gate, yet polls keep returning.
    for _ in 0..10 {
        meter.poll(now, None, None);
    }
    assert_eq!(meter.view().ram, Metric::Measuring, "not finished yet");
    assert_ne!(*PROBE_THREAD.lock(), Some(std::thread::current().id()));
    GATE_OPEN.store(true, Ordering::SeqCst);
    settle(&mut meter, now, None, |view| view.ram != Metric::Measuring);
    assert_eq!(
        meter.view().ram,
        Metric::Available("10.0 MB process, peak 20.0 MB".into())
    );
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
