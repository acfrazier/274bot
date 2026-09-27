//! The process resource meter shared by the panel and the TUI. The
//! operator session owns the one meter and polls it once per frame; the
//! meter samples at most once per [`SAMPLE_PERIOD`] by the poll's clock,
//! never per row, per bot or per frame. The OS process probe (system calls,
//! `/proc` reads) runs on the meter's own thread, so a poll only asks for
//! a probe and picks up the latest finished one; the fleet side (live
//! workers, traffic) is one in-memory pass over the host rows. Every value
//! says whether it is still measuring, measured, not measurable here, or
//! failed; memory is the whole process's, never a per-bot figure.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use host_play::Play;
use parking_lot::Mutex;

/// How often the meter samples.
pub const SAMPLE_PERIOD: Duration = Duration::from_secs(1);

/// One meter value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Metric {
    /// A rate that needs a second sample (or a re-baseline) first.
    Measuring,
    Available(String),
    /// Not measurable here; the reason says why.
    Unavailable(&'static str),
    Error(String),
}

impl Metric {
    /// The value, or the reason it has none.
    pub fn text(&self) -> &str {
        match self {
            Self::Measuring => "measuring…",
            Self::Available(text) | Self::Error(text) => text,
            Self::Unavailable(reason) => reason,
        }
    }
}

/// What the meter shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceView {
    /// Live worker lifetimes: logged-out, queued and connecting workers
    /// count; ended ones do not.
    pub bots: usize,
    pub ingame: usize,
    /// Live workers other than the selected bot.
    pub background: usize,
    /// Process CPU: busy cores and their share of all cores.
    pub cpu: Metric,
    /// Process memory: the current resident set and the lifetime peak.
    pub ram: Metric,
    /// Summed stream bytes of every live worker, per second.
    pub traffic: Metric,
    /// The three values on one narrow line for a header: `cpu 12% ram
    /// 263.9 MB net 1.2 KB/s`. A value still measuring reads `…`, one with
    /// nothing to measure `-` and a failed one `err`.
    pub brief: String,
}

impl Default for ResourceView {
    fn default() -> Self {
        Self {
            bots: 0,
            ingame: 0,
            background: 0,
            cpu: Metric::Measuring,
            ram: Metric::Measuring,
            traffic: Metric::Measuring,
            brief: String::from("cpu … ram … net …"),
        }
    }
}

/// One value as a narrow header shows it: `figure` when measured, else
/// `…` (measuring), `-` (nothing to measure) or `err`.
fn brief_text<'a>(metric: &'a Metric, figure: &'a str) -> &'a str {
    match metric {
        Metric::Measuring => "…",
        Metric::Available(_) => figure,
        Metric::Unavailable(_) => "-",
        Metric::Error(_) => "err",
    }
}

/// One read of the process counters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ProcessProbe {
    /// This platform has no process sampler.
    Unsupported,
    /// The OS call failed.
    Failed,
    Sampled {
        /// Cumulative user + system CPU seconds.
        cpu_seconds: f64,
        /// Current resident bytes, when the platform reports them.
        resident: Option<u64>,
        /// Lifetime peak resident bytes.
        peak: u64,
    },
}

/// Read this process's CPU time and memory. The current resident size is
/// read first, so the peak read after it already covers it.
pub fn probe_process() -> ProcessProbe {
    if !cfg!(any(target_os = "macos", target_os = "linux", windows)) {
        return ProcessProbe::Unsupported;
    }
    let resident = host_play::current_resident_bytes();
    let (peak, cpu_seconds) = host_play::sample_process();
    // The host sampler's failure sentinel is `(0, 0.0)`.
    if peak == 0 && cpu_seconds == 0.0 {
        return ProcessProbe::Failed;
    }
    ProcessProbe::Sampled {
        cpu_seconds,
        resident,
        peak,
    }
}

const NOT_MEASURED_HERE: &str = "not measured on this platform";
const NO_LIVE_SLOTS: &str = "no live slots";
const PROBE_FAILED: &str = "process sample failed";

/// CPU and memory as the probe thread last measured them, with the short
/// figures a narrow header shows while they are available.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ProcessMetrics {
    cpu: Metric,
    ram: Metric,
    /// The CPU share of all cores (`12%`).
    cpu_figure: String,
    /// The current resident size (`263.9 MB`), else the peak.
    ram_figure: String,
}

impl ProcessMetrics {
    /// Both values in one state that has no figure.
    fn without_figures(value: Metric) -> Self {
        Self {
            cpu: value.clone(),
            ram: value,
            cpu_figure: String::new(),
            ram_figure: String::new(),
        }
    }
}

/// What the poll and the probe thread share.
struct ProbeShared {
    probe: fn() -> ProcessProbe,
    /// Set by the poll to ask for one probe; the thread takes it.
    requested: AtomicBool,
    stop: AtomicBool,
    /// Probes finished so far; the poll reads `latest` when it moves.
    finished: AtomicU64,
    latest: Mutex<ProcessMetrics>,
}

/// The probe thread: waits to be asked, runs the OS probe and turns
/// successive reads into CPU and memory values.
fn probe_thread(shared: Arc<ProbeShared>) {
    let cores = thread::available_parallelism()
        .map_or(1, |n| n.get() as u32)
        .max(1);
    let mut last_cpu = None;
    loop {
        if shared.stop.load(Ordering::Acquire) {
            return;
        }
        if !shared.requested.swap(false, Ordering::AcqRel) {
            thread::park();
            continue;
        }
        let taken = Instant::now();
        let probe = std::panic::catch_unwind(shared.probe).unwrap_or(ProcessProbe::Failed);
        let metrics = process_metrics(probe, taken, &mut last_cpu, cores);
        *shared.latest.lock() = metrics;
        shared.finished.fetch_add(1, Ordering::Release);
    }
}

/// CPU from the previous read (`last_cpu`, updated here) and memory from
/// this one. The peak shown is never below the current size: a lifetime
/// peak below it is not a valid value.
fn process_metrics(
    probe: ProcessProbe,
    taken: Instant,
    last_cpu: &mut Option<(Instant, f64)>,
    cores: u32,
) -> ProcessMetrics {
    match probe {
        ProcessProbe::Unsupported => {
            *last_cpu = None;
            ProcessMetrics::without_figures(Metric::Unavailable(NOT_MEASURED_HERE))
        }
        ProcessProbe::Failed => {
            *last_cpu = None;
            ProcessMetrics::without_figures(Metric::Error(PROBE_FAILED.into()))
        }
        ProcessProbe::Sampled {
            cpu_seconds,
            resident,
            peak,
        } => {
            let cpu = last_cpu
                .replace((taken, cpu_seconds))
                .and_then(|(then, cpu0)| {
                    cpu_from_delta(
                        cpu_seconds - cpu0,
                        taken.saturating_duration_since(then).as_secs_f64(),
                        cores,
                    )
                });
            let (cpu, cpu_figure) = match cpu {
                Some((text, figure)) => (Metric::Available(text), figure),
                None => (Metric::Measuring, String::new()),
            };
            let (ram, ram_figure) = match resident {
                Some(resident) => {
                    let now = format_bytes(resident);
                    let ram = format!("{now} process, peak {}", format_bytes(peak.max(resident)));
                    (Metric::Available(ram), now)
                }
                None => {
                    let peak = format!("peak {}", format_bytes(peak));
                    (Metric::Available(format!("{peak} process")), peak)
                }
            };
            ProcessMetrics {
                cpu,
                ram,
                cpu_figure,
                ram_figure,
            }
        }
    }
}

/// One live worker's traffic counter at a sample.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SlotTraffic {
    lifetime: u64,
    stream: u32,
    bytes: u64,
}

/// Per-worker traffic baselines: a rate is only taken over the same
/// workers and the same streams, so a replaced worker or a restarted
/// counter re-baselines instead of producing a false rate.
#[derive(Debug, Default)]
struct Traffic {
    at: Option<Instant>,
    /// The last sample, sorted by lifetime.
    last: Vec<SlotTraffic>,
    /// This sample being gathered (then swapped with `last`).
    current: Vec<SlotTraffic>,
}

impl Traffic {
    fn rate(&mut self, now: Instant) -> Metric {
        self.current.sort_unstable_by_key(|slot| slot.lifetime);
        std::mem::swap(&mut self.last, &mut self.current);
        let (sample, before) = (&self.last, &self.current);
        let previous = self.at.replace(now);
        if sample.is_empty() {
            return Metric::Unavailable(NO_LIVE_SLOTS);
        }
        let Some(then) = previous else {
            return Metric::Measuring;
        };
        let dt = now.saturating_duration_since(then).as_secs_f64();
        if dt <= 0.0 || sample.len() != before.len() {
            return Metric::Measuring;
        }
        let mut bytes = 0u64;
        for (now, then) in sample.iter().zip(before) {
            if now.lifetime != then.lifetime || now.stream != then.stream || now.bytes < then.bytes
            {
                return Metric::Measuring;
            }
            bytes = bytes.saturating_add(now.bytes - then.bytes);
        }
        Metric::Available(format_rate(bytes as f64 / dt))
    }
}

/// The meter state kept by the poll.
pub(crate) struct Resources {
    shared: Arc<ProbeShared>,
    /// The probe thread, started by the first sample.
    worker: Option<JoinHandle<()>>,
    /// A probe was asked for and has not finished.
    in_flight: bool,
    /// `ProbeShared::finished` as last taken.
    seen: u64,
    last_sample: Option<Instant>,
    traffic: Traffic,
    view: ResourceView,
    /// [`ProcessMetrics`]' figures behind `view.cpu` and `view.ram`.
    cpu_figure: String,
    ram_figure: String,
    generation: u64,
}

impl Default for Resources {
    fn default() -> Self {
        Self::with_probe(probe_process)
    }
}

impl Drop for Resources {
    fn drop(&mut self) {
        // The thread exits at its next wake; nothing here waits for it.
        self.shared.stop.store(true, Ordering::Release);
        if let Some(worker) = &self.worker {
            worker.thread().unpark();
        }
    }
}

impl Resources {
    pub(crate) fn with_probe(probe: fn() -> ProcessProbe) -> Self {
        Self {
            shared: Arc::new(ProbeShared {
                probe,
                requested: AtomicBool::new(false),
                stop: AtomicBool::new(false),
                finished: AtomicU64::new(0),
                latest: Mutex::new(ProcessMetrics::without_figures(Metric::Measuring)),
            }),
            worker: None,
            in_flight: false,
            seen: 0,
            last_sample: None,
            traffic: Traffic::default(),
            view: ResourceView::default(),
            cpu_figure: String::new(),
            ram_figure: String::new(),
            generation: 0,
        }
    }

    pub(crate) fn view(&self) -> &ResourceView {
        &self.view
    }

    /// Moves whenever the view changed.
    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    /// Take a finished process probe (one atomic load when there is none)
    /// and, once per [`SAMPLE_PERIOD`], ask for the next one and count the
    /// live workers and their traffic.
    pub(crate) fn poll(&mut self, now: Instant, play: Option<&Play>, selected: Option<&str>) {
        let mut changed = self.take_process_metrics();
        let due = self
            .last_sample
            .is_none_or(|last| now.saturating_duration_since(last) >= SAMPLE_PERIOD);
        if due {
            self.last_sample = Some(now);
            changed |= self.request_probe();
            changed |= self.sample_fleet(now, play, selected);
        }
        if changed {
            self.write_brief();
            self.generation += 1;
        }
    }

    /// Rebuild the narrow header line from the current values (into its
    /// own buffer).
    fn write_brief(&mut self) {
        use std::fmt::Write as _;
        let view = &mut self.view;
        view.brief.clear();
        let _ = write!(
            view.brief,
            "cpu {} ram {} net {}",
            brief_text(&view.cpu, &self.cpu_figure),
            brief_text(&view.ram, &self.ram_figure),
            brief_text(&view.traffic, view.traffic.text()),
        );
    }

    fn take_process_metrics(&mut self) -> bool {
        let finished = self.shared.finished.load(Ordering::Acquire);
        if finished == self.seen {
            return false;
        }
        self.seen = finished;
        self.in_flight = false;
        let latest = self.shared.latest.lock();
        let changed = self.view.cpu != latest.cpu || self.view.ram != latest.ram;
        if changed {
            self.view.cpu.clone_from(&latest.cpu);
            self.view.ram.clone_from(&latest.ram);
            self.cpu_figure.clone_from(&latest.cpu_figure);
            self.ram_figure.clone_from(&latest.ram_figure);
        }
        changed
    }

    /// Ask the probe thread for one probe (starting it the first time).
    /// A probe still running is not asked for again. Returns whether the
    /// view changed (the thread could not be started or has stopped).
    fn request_probe(&mut self) -> bool {
        if self
            .worker
            .as_ref()
            .is_some_and(|worker| worker.is_finished())
        {
            self.worker = None;
            self.in_flight = false;
            return self.set_process_error("resource meter thread stopped".into());
        }
        if self.in_flight {
            return false;
        }
        if self.worker.is_none() {
            let shared = Arc::clone(&self.shared);
            match thread::Builder::new()
                .name("resource-meter".into())
                .spawn(move || probe_thread(shared))
            {
                Ok(worker) => self.worker = Some(worker),
                Err(error) => {
                    return self.set_process_error(format!("resource meter thread: {error}"))
                }
            }
        }
        self.shared.requested.store(true, Ordering::Release);
        if let Some(worker) = &self.worker {
            worker.thread().unpark();
        }
        self.in_flight = true;
        false
    }

    fn set_process_error(&mut self, error: String) -> bool {
        let metric = Metric::Error(error);
        let changed = self.view.cpu != metric || self.view.ram != metric;
        self.view.cpu.clone_from(&metric);
        self.view.ram = metric;
        changed
    }

    /// Count the live workers and take their traffic rate: one pass over
    /// the host rows, no system calls.
    fn sample_fleet(&mut self, now: Instant, play: Option<&Play>, selected: Option<&str>) -> bool {
        let (mut bots, mut ingame, mut background) = (0, 0, 0);
        let current = &mut self.traffic.current;
        current.clear();
        if let Some(play) = play {
            play.for_each_live_slot(|slot| {
                bots += 1;
                ingame += usize::from(slot.ingame);
                background += usize::from(selected != Some(slot.name));
                current.push(SlotTraffic {
                    lifetime: slot.lifetime,
                    stream: slot.stream,
                    bytes: slot.traffic_bytes,
                });
            });
        }
        let traffic = self.traffic.rate(now);
        let view = &mut self.view;
        let changed = (view.bots, view.ingame, view.background) != (bots, ingame, background)
            || view.traffic != traffic;
        view.bots = bots;
        view.ingame = ingame;
        view.background = background;
        view.traffic = traffic;
        changed
    }
}

/// CPU use from CPU and wall deltas: busy cores and their share of all
/// `cores` (`0.5 cores (12% of 4)`), then the share alone for a narrow
/// header (`12%`). `None` (still measuring) when no wall time passed.
pub fn cpu_from_delta(cpu_secs: f64, wall_secs: f64, cores: u32) -> Option<(String, String)> {
    if wall_secs <= 0.0 {
        return None;
    }
    let busy = cpu_secs / wall_secs;
    let percent = 100.0 * busy / f64::from(cores.max(1));
    Some((
        format!("{busy:.1} cores ({percent:.0}% of {cores})"),
        format!("{percent:.0}%"),
    ))
}

/// Bytes in the nearest unit: B under 1 KB, then KB, MB, GB.
pub fn format_bytes(bytes: u64) -> String {
    let b = bytes as f64;
    let kb = 1024.0;
    let mb = kb * 1024.0;
    let gb = mb * 1024.0;
    if b < kb {
        format!("{b:.0} B")
    } else if b < mb {
        format!("{:.0} KB", b / kb)
    } else if b < gb {
        format!("{:.1} MB", b / mb)
    } else {
        format!("{:.2} GB", b / gb)
    }
}

fn format_rate(bytes_per_sec: f64) -> String {
    let kb = 1024.0;
    let mb = kb * 1024.0;
    if bytes_per_sec < kb {
        format!("{bytes_per_sec:.0} B/s")
    } else if bytes_per_sec < mb {
        format!("{:.1} KB/s", bytes_per_sec / kb)
    } else {
        format!("{:.1} MB/s", bytes_per_sec / mb)
    }
}

/// `"{n} bots ({ingame} running)"`, singular `bot` for one.
pub fn format_bots(n: usize, ingame: usize) -> String {
    let noun = if n == 1 { "bot" } else { "bots" };
    format!("{n} {noun} ({ingame} running)")
}

/// The background-bot count: `1 other`, `3 others`.
pub fn format_background(n: usize) -> String {
    if n == 1 {
        "1 other".into()
    } else {
        format!("{n} others")
    }
}

/// The background-bots notice body: other profiles keep running, with the
/// live meter.
pub fn background_ack_text(others: usize, view: &ResourceView) -> String {
    format!(
        "Other profiles keep running in the background ({}). See the resource section for live cost: {} · cpu {} · ram {}.",
        format_background(others),
        format_bots(view.bots, view.ingame),
        view.cpu.text(),
        view.ram.text(),
    )
}

#[cfg(test)]
#[path = "resources_tests.rs"]
mod tests;
