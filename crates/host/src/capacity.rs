//! Host capacity: whether this machine gives the bots CPU time when they
//! need it.
//!
//! Each slot thread owns one [`SlotCapacity`] (atomics only), registered
//! once when the thread binds and unregistered when it unbinds. The frame
//! path writes only that slot's atomics: no shared lock, no hash, no heap.
//! The operator session calls [`CapacityMonitor::evaluate`] at UI cadence;
//! that is the only place that walks every slot.
//!
//! A service's delay is **scheduling delay**: how long the slot thread
//! was kept waiting for a CPU since its previous service, not how long
//! its own work took.
//! - Linux: the thread's run-queue wait (`/proc/thread-self/schedstat`,
//!   field 2: nanoseconds runnable but not running), one `pread` per
//!   service. The bot's own work, its sleeps and its blocking on other
//!   threads (the script isolate, locks, the GPU) are not run-queue wait,
//!   so a 2 s script tick on an idle machine adds nothing. A timer or
//!   packet wake that waits for a busy CPU, and preemption in the middle of
//!   a tick, add all of it, on the 20 ms frame path and the idle park alike.
//! - Elsewhere (macOS, Windows, or a Linux kernel without schedstat): how
//!   far past its chosen deadline a timed wait returned (the 20 ms frame
//!   sleep or the park bound). Own work is excluded by construction (a
//!   frame that overran its 20 ms does not sleep, so it has no deadline to
//!   miss). Preemption inside a tick and a packet wake that is late but
//!   still before the park bound are not seen there.

use std::cell::RefCell;
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use api::host_log;
use api::hostlog::{Category, Level};

/// Sliding window that counts late services.
pub const WINDOW: Duration = Duration::from_secs(10);
/// Warning stays up until this long without a delay of [`CALM`] or more.
pub const CLEAR_AFTER: Duration = Duration::from_secs(30);
/// A service delayed this long counts as late.
pub const LATE: Duration = Duration::from_millis(600);
/// Delays at or above this keep the warning up (reset the calm timer).
pub const CALM: Duration = Duration::from_millis(300);
/// A bot needs this many late services in [`WINDOW`] to count.
pub const MIN_LATE_SERVICES: u32 = 3;
/// Floor on how many late bots trip the warning.
pub const MIN_LATE_BOTS: usize = 3;
/// Late bots as a percentage of active bots, with [`MIN_LATE_BOTS`] as floor.
pub const LATE_BOT_PERCENT: usize = 10;

/// Delays of [`CALM`] or more each slot keeps. A bot can have at most about
/// `WINDOW / CALM` of them in a window, and only [`MIN_LATE_SERVICES`] late
/// ones decide whether it counts.
const RING: usize = 16;
/// Low bits of a packed sample: delay ms (or tick count).
const LOW_BITS: u32 = 24;
const LOW_MASK: u64 = (1 << LOW_BITS) - 1;
/// Shown figures are rounded to this, so the warning changes only when the
/// line would.
const SHOWN_STEP_MS: u64 = 10;

const WINDOW_MS: u64 = WINDOW.as_millis() as u64;
const CLEAR_AFTER_MS: u64 = CLEAR_AFTER.as_millis() as u64;
const LATE_MS: u64 = LATE.as_millis() as u64;

/// Whether this bot counts toward the fleet threshold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ServiceClass {
    /// In game, not latched, not paused, hearing the server: counted.
    Active = 0,
    /// The script watchdog has seen no game packets for 10 s: the server
    /// is quiet, which is not this host's load.
    Quiet = 1,
    /// Title, logging in, or login-latched: ignored.
    Parked = 2,
    /// Operator-paused script: ignored.
    Paused = 3,
    /// Frozen or stopping script: ignored.
    Suspended = 4,
}

impl ServiceClass {
    fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::Active,
            1 => Self::Quiet,
            3 => Self::Paused,
            4 => Self::Suspended,
            _ => Self::Parked,
        }
    }
}

/// How the front end is drawing, for the warning line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RenderPolicyLabel {
    /// Panel default: only the focused bot rasters.
    #[default]
    OnlySelected,
    /// Every wall member rasters (render-all).
    RenderAll,
    /// TUI / headless: no game raster.
    Headless,
}

impl RenderPolicyLabel {
    /// End-user label in the warning line.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OnlySelected => "only selected",
            Self::RenderAll => "render-all",
            Self::Headless => "headless",
        }
    }
}

/// Snapshot shown while the host is behind. Figures are rounded to 10 ms.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CapacityWarning {
    late_bots: u16,
    active_bots: u16,
    threshold: u16,
    p95_delay: Option<Duration>,
    tick_period: Option<Duration>,
    render: RenderPolicyLabel,
}

impl CapacityWarning {
    /// A warning snapshot; durations are rounded to 10 ms.
    pub fn new(
        late_bots: u16,
        active_bots: u16,
        threshold: u16,
        p95_delay: Option<Duration>,
        tick_period: Option<Duration>,
        render: RenderPolicyLabel,
    ) -> Self {
        let round = |d: Duration| {
            let ms = (d.as_millis() as u64 + SHOWN_STEP_MS / 2) / SHOWN_STEP_MS * SHOWN_STEP_MS;
            Duration::from_millis(ms)
        };
        Self {
            late_bots,
            active_bots,
            threshold,
            p95_delay: p95_delay.map(round),
            tick_period: tick_period.map(round),
            render,
        }
    }

    /// Active bots with at least [`MIN_LATE_SERVICES`] late services in
    /// [`WINDOW`].
    pub fn late_bots(&self) -> u16 {
        self.late_bots
    }

    /// Active bots (parked, paused, suspended and quiet bots excluded).
    pub fn active_bots(&self) -> u16 {
        self.active_bots
    }

    /// `max(3, 10% of active)`, rounded up.
    pub fn threshold(&self) -> u16 {
        self.threshold
    }

    /// p95 of the delays of [`CALM`] or more in [`WINDOW`] across active
    /// bots; `None` when there were none.
    pub fn p95_delay(&self) -> Option<Duration> {
        self.p95_delay
    }

    /// Median time per game tick the active bots processed, measured
    /// between real `PLAYER_INFO` ticks over 10–20 s; `None` without ticks.
    pub fn tick_period(&self) -> Option<Duration> {
        self.tick_period
    }

    /// What the front end is rendering.
    pub fn render(&self) -> RenderPolicyLabel {
        self.render
    }
}

/// Result of one evaluate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CapacityEval {
    /// No warning.
    Clear,
    /// Every bot that would count hears no game packets: the server is
    /// quiet, so the host is not blamed.
    Silence,
    /// Show the warning.
    Warn(CapacityWarning),
}

/// One slot's record, written only by its own slot thread.
struct SlotCapacity {
    class: AtomicU8,
    /// `(at_ms + 1) << 24 | delay_ms`; 0 is empty.
    samples: [AtomicU64; RING],
    /// `(at_ms + 1) << 24 | ticks mod 2^24` at the newest game tick; 0 none.
    tick: AtomicU64,
}

impl SlotCapacity {
    fn new() -> Self {
        Self {
            class: AtomicU8::new(ServiceClass::Parked as u8),
            samples: std::array::from_fn(|_| AtomicU64::new(0)),
            tick: AtomicU64::new(0),
        }
    }

    fn class(&self) -> ServiceClass {
        ServiceClass::from_u8(self.class.load(Ordering::Relaxed))
    }
}

fn pack(at_ms: u64, low: u64) -> u64 {
    ((at_ms + 1) << LOW_BITS) | low.min(LOW_MASK)
}

fn unpack(packed: u64) -> Option<(u64, u64)> {
    (packed != 0).then(|| ((packed >> LOW_BITS) - 1, packed & LOW_MASK))
}

/// Where a slot's scheduling delay comes from.
enum WaitSource {
    /// Cumulative run-queue wait of this thread.
    #[cfg(target_os = "linux")]
    RunQueue(RunQueueStat),
    /// The slot loop's timed-wait overshoot.
    Overshoot,
    /// A test's cumulative run-queue wait.
    #[cfg(test)]
    Injected(std::rc::Rc<std::cell::Cell<Duration>>),
}

impl WaitSource {
    /// Run-queue wait where the kernel keeps it; overshoot elsewhere. Call
    /// on the slot thread (the Linux file is that thread's).
    fn for_this_thread() -> Self {
        #[cfg(target_os = "linux")]
        if let Some(stat) = RunQueueStat::open() {
            log_source_once("run-queue wait (Linux schedstat)");
            return Self::RunQueue(stat);
        }
        log_source_once("timed-wake overshoot");
        Self::Overshoot
    }

    /// Cumulative run-queue wait; `None` means use the overshoot.
    fn waited(&self) -> Option<Duration> {
        match self {
            #[cfg(target_os = "linux")]
            Self::RunQueue(stat) => stat.waited(),
            Self::Overshoot => None,
            #[cfg(test)]
            Self::Injected(total) => Some(total.get()),
        }
    }
}

fn log_source_once(source: &str) {
    static LOGGED: std::sync::Once = std::sync::Once::new();
    LOGGED.call_once(|| {
        host_log!(
            Category::Lifecycle,
            Level::Info,
            "host capacity: measuring each bot's {source}"
        );
    });
}

/// This thread's `/proc/thread-self/schedstat`, kept open: one `pread`
/// per read, no allocation.
#[cfg(target_os = "linux")]
struct RunQueueStat(std::fs::File);

#[cfg(target_os = "linux")]
impl RunQueueStat {
    /// `None` when the file is missing or the kernel does not keep the
    /// counters (older kernels then report `0 0 0`; a thread that is running
    /// has been switched in at least once). The CPU-time field is no test:
    /// it can still read 0 on a thread that has only just started.
    fn open() -> Option<Self> {
        let stat = Self(std::fs::File::open("/proc/thread-self/schedstat").ok()?);
        let [_, _, slices] = stat.read()?;
        (slices > 0).then_some(stat)
    }

    fn read(&self) -> Option<[u64; 3]> {
        use std::os::unix::fs::FileExt;
        let mut buf = [0u8; 80];
        let n = self.0.read_at(&mut buf, 0).ok()?;
        parse_schedstat(&buf[..n])
    }

    fn waited(&self) -> Option<Duration> {
        self.read().map(|[_, wait, _]| Duration::from_nanos(wait))
    }
}

/// `"<cpu ns> <run-queue wait ns> <timeslices>\n"`.
#[cfg(any(target_os = "linux", test))]
fn parse_schedstat(bytes: &[u8]) -> Option<[u64; 3]> {
    let mut fields = bytes
        .split(|b| b.is_ascii_whitespace())
        .filter(|field| !field.is_empty())
        .map(|field| {
            field.iter().try_fold(0u64, |acc, &b| {
                b.is_ascii_digit()
                    .then_some(())
                    .and_then(|()| acc.checked_mul(10)?.checked_add(u64::from(b - b'0')))
            })
        });
    Some([fields.next()??, fields.next()??, fields.next()??])
}

/// The slot thread's writer: its own [`SlotCapacity`] plus the state only
/// this thread reads. Dropping it unregisters the slot.
struct SlotRecorder {
    monitor: Arc<CapacityMonitor>,
    slot: Arc<SlotCapacity>,
    source: WaitSource,
    class: ServiceClass,
    /// The previous service was Active: the delay since then counts only
    /// when that one and this one both were.
    counted: bool,
    /// Cumulative run-queue wait at the previous service.
    waited: Option<Duration>,
    cursor: usize,
    ticks: u64,
}

impl SlotRecorder {
    fn new(monitor: &Arc<CapacityMonitor>, source: WaitSource) -> Self {
        Self {
            monitor: Arc::clone(monitor),
            slot: monitor.register(),
            source,
            class: ServiceClass::Parked,
            counted: false,
            waited: None,
            cursor: 0,
            ticks: 0,
        }
    }

    /// Leaving Active drops the slot's samples, so delays from before a
    /// pause, a park or a relog never count after it. Any change also skips
    /// the next service: its delay accrued under the old class.
    fn set_class(&mut self, class: ServiceClass) {
        if class == self.class {
            return;
        }
        if self.class == ServiceClass::Active {
            for cell in &self.slot.samples {
                cell.store(0, Ordering::Relaxed);
            }
        }
        self.class = class;
        self.counted = false;
        self.slot.class.store(class as u8, Ordering::Relaxed);
    }

    /// The slot loop starts (after login): re-baseline, count nothing until
    /// observe reports Active twice in a row.
    fn begin(&mut self, ticks: u64) {
        self.set_class(ServiceClass::Parked);
        self.counted = false;
        self.waited = self.source.waited();
        self.ticks = ticks;
    }

    /// One service at `at`. `overshoot` is how far past its timed deadline
    /// the loop resumed (used only without a run-queue clock); `ticks` is
    /// the client's cumulative `PLAYER_INFO` count.
    fn record(&mut self, at: Instant, overshoot: Duration, ticks: u64) {
        let delay = match self.source.waited() {
            Some(total) => {
                let delay = self
                    .waited
                    .map_or(Duration::ZERO, |prev| total.saturating_sub(prev));
                self.waited = Some(total);
                delay
            }
            None => overshoot,
        };
        let active = self.class == ServiceClass::Active;
        if active && self.counted && delay >= CALM {
            let delay_ms = u64::try_from(delay.as_millis()).unwrap_or(u64::MAX);
            self.slot.samples[self.cursor]
                .store(pack(self.monitor.ms(at), delay_ms), Ordering::Relaxed);
            self.cursor = (self.cursor + 1) % RING;
        }
        self.counted = active;
        if ticks != self.ticks {
            self.ticks = ticks;
            self.slot.tick.store(
                pack(self.monitor.ms(at), ticks & LOW_MASK),
                Ordering::Relaxed,
            );
        }
    }
}

impl Drop for SlotRecorder {
    fn drop(&mut self) {
        self.monitor.unregister(&self.slot);
    }
}

thread_local! {
    static RECORDER: RefCell<Option<SlotRecorder>> = const { RefCell::new(None) };
}

fn with_recorder(f: impl FnOnce(&mut SlotRecorder)) {
    RECORDER.with(|cell| {
        if let Some(recorder) = cell.borrow_mut().as_mut() {
            f(recorder);
        }
    });
}

/// Register this slot thread with a play's monitor (once, at thread start).
pub fn bind_capacity(monitor: &Arc<CapacityMonitor>) {
    let recorder = SlotRecorder::new(monitor, WaitSource::for_this_thread());
    RECORDER.with(|cell| *cell.borrow_mut() = Some(recorder));
}

/// Bind this thread with an injected run-queue clock (`Some`) or the
/// overshoot fallback (`None`).
#[cfg(test)]
pub(crate) fn bind_capacity_for_test(
    monitor: &Arc<CapacityMonitor>,
    waited: Option<std::rc::Rc<std::cell::Cell<Duration>>>,
) {
    let source = waited.map_or(WaitSource::Overshoot, WaitSource::Injected);
    let recorder = SlotRecorder::new(monitor, source);
    RECORDER.with(|cell| *cell.borrow_mut() = Some(recorder));
}

/// Unregister this slot thread (thread exit).
pub fn unbind_capacity() {
    let recorder = RECORDER.with(|cell| cell.borrow_mut().take());
    drop(recorder);
}

/// This thread's class from the latest observe.
pub fn set_service_class(class: ServiceClass) {
    with_recorder(|recorder| recorder.set_class(class));
}

pub(crate) fn begin_services(ticks: u64) {
    with_recorder(|recorder| recorder.begin(ticks));
}

pub(crate) fn end_services() {
    with_recorder(|recorder| recorder.set_class(ServiceClass::Parked));
}

pub(crate) fn record_service(at: Instant, overshoot: Duration, ticks: u64) {
    with_recorder(|recorder| recorder.record(at, overshoot, ticks));
}

/// Per-play registry of slot records and the warning's hysteresis.
pub struct CapacityMonitor {
    epoch: Instant,
    registry: Mutex<Registry>,
}

#[derive(Default)]
struct Registry {
    slots: Vec<Entry>,
    warning: bool,
    /// Newest delay of [`CALM`] or more seen from an active bot, ms.
    last_unrest: Option<u64>,
    /// Reused buffers: no allocation per evaluate once warm.
    delays: Vec<u64>,
    periods: Vec<u64>,
}

/// Evaluator-side state per slot: tick marks `WINDOW` apart, so the tick
/// period spans 10–20 s of real ticks.
struct Entry {
    slot: Arc<SlotCapacity>,
    base: u64,
    mid: u64,
}

impl Entry {
    fn reset_ticks(&mut self) {
        self.base = 0;
        self.mid = 0;
    }

    /// ms per processed game tick, between real tick instants.
    fn tick_period(&mut self, now_ms: u64) -> Option<u64> {
        let tick = self.slot.tick.load(Ordering::Relaxed);
        let Some((at, count)) = unpack(tick) else {
            self.reset_ticks();
            return None;
        };
        if now_ms.saturating_sub(at) > WINDOW_MS {
            self.reset_ticks();
            return None;
        }
        let Some((mid_at, _)) = unpack(self.mid) else {
            self.base = tick;
            self.mid = tick;
            return None;
        };
        if at.saturating_sub(mid_at) >= WINDOW_MS {
            self.base = self.mid;
            self.mid = tick;
        }
        let (base_at, base_count) = unpack(self.base)?;
        let ticks = count.wrapping_sub(base_count) & LOW_MASK;
        (ticks > 0 && at > base_at).then(|| (at - base_at) / ticks)
    }
}

impl Default for CapacityMonitor {
    fn default() -> Self {
        Self::new()
    }
}

impl CapacityMonitor {
    /// Empty registry.
    pub fn new() -> Self {
        Self {
            epoch: Instant::now(),
            registry: Mutex::new(Registry::default()),
        }
    }

    fn ms(&self, at: Instant) -> u64 {
        u64::try_from(at.saturating_duration_since(self.epoch).as_millis()).unwrap_or(u64::MAX)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Registry> {
        self.registry.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn register(&self) -> Arc<SlotCapacity> {
        let slot = Arc::new(SlotCapacity::new());
        self.lock().slots.push(Entry {
            slot: Arc::clone(&slot),
            base: 0,
            mid: 0,
        });
        slot
    }

    fn unregister(&self, slot: &Arc<SlotCapacity>) {
        self.lock()
            .slots
            .retain(|entry| !Arc::ptr_eq(&entry.slot, slot));
    }

    /// Slot threads currently registered.
    pub fn registered(&self) -> usize {
        self.lock().slots.len()
    }

    /// Every stored delay (ms) across registered slots, sorted.
    #[cfg(test)]
    pub(crate) fn recorded_delays(&self) -> Vec<u64> {
        let mut delays: Vec<u64> = self
            .lock()
            .slots
            .iter()
            .flat_map(|entry| entry.slot.samples.iter())
            .filter_map(|cell| unpack(cell.load(Ordering::Relaxed)).map(|(_, delay)| delay))
            .collect();
        delays.sort_unstable();
        delays
    }

    /// Decide whether to show the warning. Call at UI cadence; `render` is
    /// display-only.
    pub fn evaluate(&self, now: Instant, render: RenderPolicyLabel) -> CapacityEval {
        let now_ms = self.ms(now);
        let mut guard = self.lock();
        let Registry {
            slots,
            warning,
            last_unrest,
            delays,
            periods,
        } = &mut *guard;
        delays.clear();
        periods.clear();
        let (mut active, mut quiet, mut late_bots) = (0usize, 0usize, 0usize);
        for entry in slots.iter_mut() {
            match entry.slot.class() {
                ServiceClass::Active => {}
                ServiceClass::Quiet => {
                    quiet += 1;
                    entry.reset_ticks();
                    continue;
                }
                _ => {
                    entry.reset_ticks();
                    continue;
                }
            }
            active += 1;
            let mut lates = 0u32;
            for cell in &entry.slot.samples {
                let Some((at, delay_ms)) = unpack(cell.load(Ordering::Relaxed)) else {
                    continue;
                };
                if last_unrest.is_none_or(|newest| at > newest) {
                    *last_unrest = Some(at);
                }
                if now_ms.saturating_sub(at) > WINDOW_MS {
                    continue;
                }
                delays.push(delay_ms);
                if delay_ms >= LATE_MS {
                    lates += 1;
                }
            }
            if lates >= MIN_LATE_SERVICES {
                late_bots += 1;
            }
            if let Some(period) = entry.tick_period(now_ms) {
                periods.push(period);
            }
        }
        if active == 0 {
            *warning = false;
            return if quiet > 0 {
                CapacityEval::Silence
            } else {
                CapacityEval::Clear
            };
        }
        let threshold = MIN_LATE_BOTS.max(active.div_ceil(LATE_BOT_PERCENT));
        let calm = last_unrest.is_none_or(|at| now_ms.saturating_sub(at) >= CLEAR_AFTER_MS);
        *warning = late_bots >= threshold || (*warning && !calm);
        if !*warning {
            return CapacityEval::Clear;
        }
        let p95 = (!delays.is_empty()).then(|| {
            delays.sort_unstable();
            // Nearest rank: the smallest delay at or above 95% of them.
            delays[(delays.len() * 95).div_ceil(100) - 1]
        });
        let tick = (!periods.is_empty()).then(|| {
            periods.sort_unstable();
            periods[periods.len() / 2]
        });
        let count = |n: usize| u16::try_from(n).unwrap_or(u16::MAX);
        CapacityEval::Warn(CapacityWarning::new(
            count(late_bots),
            count(active),
            count(threshold),
            p95.map(Duration::from_millis),
            tick.map(Duration::from_millis),
            render,
        ))
    }
}

#[cfg(test)]
#[path = "capacity_tests.rs"]
mod tests;
