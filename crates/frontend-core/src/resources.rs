//! The process resource meter shared by the panel and the TUI. The
//! operator session owns the one sampler and runs it from `poll` at most
//! once per [`SAMPLE_PERIOD`]: never per row, per bot or per frame. Every
//! value says whether it is still measuring, measured, not measurable here,
//! or failed; memory is the whole process's, never a per-bot figure.

use std::time::{Duration, Instant};

use host_play::Play;

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
        }
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

/// Read this process's CPU time and memory.
pub fn probe_process() -> ProcessProbe {
    if !cfg!(any(target_os = "macos", target_os = "linux", windows)) {
        return ProcessProbe::Unsupported;
    }
    let (peak, cpu_seconds) = host_play::sample_process();
    // The host sampler's failure sentinel is `(0, 0.0)`.
    if peak == 0 && cpu_seconds == 0.0 {
        return ProcessProbe::Failed;
    }
    ProcessProbe::Sampled {
        cpu_seconds,
        resident: host_play::current_resident_bytes(),
        peak,
    }
}

const NOT_MEASURED_HERE: &str = "not measured on this platform";
const NO_LIVE_SLOTS: &str = "no live slots";
const PROBE_FAILED: &str = "process sample failed";

/// The meter state kept between samples.
pub(crate) struct Resources {
    probe: fn() -> ProcessProbe,
    cores: u32,
    last_sample: Option<Instant>,
    last_cpu: Option<(Instant, f64)>,
    last_traffic: Option<(Instant, u64, usize)>,
    view: ResourceView,
    generation: u64,
}

impl Default for Resources {
    fn default() -> Self {
        Self::with_probe(probe_process)
    }
}

impl Resources {
    pub(crate) fn with_probe(probe: fn() -> ProcessProbe) -> Self {
        Self {
            probe,
            cores: std::thread::available_parallelism()
                .map_or(1, |n| n.get() as u32)
                .max(1),
            last_sample: None,
            last_cpu: None,
            last_traffic: None,
            view: ResourceView::default(),
            generation: 0,
        }
    }

    pub(crate) fn view(&self) -> &ResourceView {
        &self.view
    }

    /// Moves whenever a sample changed the view.
    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    /// Sample once per [`SAMPLE_PERIOD`]; a call before that does nothing.
    pub(crate) fn poll(&mut self, now: Instant, play: Option<&Play>, selected: Option<&str>) {
        if self
            .last_sample
            .is_some_and(|last| now.saturating_duration_since(last) < SAMPLE_PERIOD)
        {
            return;
        }
        self.last_sample = Some(now);
        let (mut bots, mut ingame, mut background, mut traffic_sum) = (0, 0, 0, 0u64);
        if let Some(play) = play {
            play.for_each_live_slot(|slot| {
                bots += 1;
                ingame += usize::from(slot.ingame);
                background += usize::from(selected != Some(slot.name));
                traffic_sum = traffic_sum.wrapping_add(slot.traffic_bytes);
            });
        }
        let traffic = self.traffic(now, traffic_sum, bots);
        let (cpu, ram) = self.process(now);
        let view = ResourceView {
            bots,
            ingame,
            background,
            cpu,
            ram,
            traffic,
        };
        if view != self.view {
            self.view = view;
            self.generation += 1;
        }
    }

    fn traffic(&mut self, now: Instant, sum: u64, slots: usize) -> Metric {
        let previous = self.last_traffic.replace((now, sum, slots));
        if slots == 0 {
            return Metric::Unavailable(NO_LIVE_SLOTS);
        }
        let Some((then, sum0, slots0)) = previous else {
            return Metric::Measuring;
        };
        let dt = now.saturating_duration_since(then).as_secs_f64();
        // A worker came or went, or a counter restarted: re-baseline
        // rather than report a false rate.
        if dt <= 0.0 || slots != slots0 || sum < sum0 {
            return Metric::Measuring;
        }
        Metric::Available(format_rate((sum - sum0) as f64 / dt))
    }

    fn process(&mut self, now: Instant) -> (Metric, Metric) {
        match (self.probe)() {
            ProcessProbe::Unsupported => {
                self.last_cpu = None;
                (
                    Metric::Unavailable(NOT_MEASURED_HERE),
                    Metric::Unavailable(NOT_MEASURED_HERE),
                )
            }
            ProcessProbe::Failed => {
                self.last_cpu = None;
                (
                    Metric::Error(PROBE_FAILED.into()),
                    Metric::Error(PROBE_FAILED.into()),
                )
            }
            ProcessProbe::Sampled {
                cpu_seconds,
                resident,
                peak,
            } => {
                let cpu = match self.last_cpu.replace((now, cpu_seconds)) {
                    Some((then, cpu0)) => cpu_from_delta(
                        cpu_seconds - cpu0,
                        now.saturating_duration_since(then).as_secs_f64(),
                        self.cores,
                    ),
                    None => Metric::Measuring,
                };
                let ram = match resident {
                    Some(resident) => Metric::Available(format!(
                        "{} process, peak {}",
                        format_bytes(resident),
                        format_bytes(peak)
                    )),
                    None => Metric::Available(format!("peak {} process", format_bytes(peak))),
                };
                (cpu, ram)
            }
        }
    }
}

/// CPU use from CPU and wall deltas: busy cores and their share of all
/// `cores`. `Measuring` when no wall time passed.
pub fn cpu_from_delta(cpu_secs: f64, wall_secs: f64, cores: u32) -> Metric {
    if wall_secs <= 0.0 {
        return Metric::Measuring;
    }
    let busy = cpu_secs / wall_secs;
    let percent = 100.0 * busy / f64::from(cores.max(1));
    Metric::Available(format!("{busy:.1} cores ({percent:.0}% of {cores})"))
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
