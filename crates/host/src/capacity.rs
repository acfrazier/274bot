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
//! - macOS and Windows (and Linux without schedstat): time **off the CPU**
//!   between two services: wall time, minus the thread's own CPU time
//!   (`CLOCK_THREAD_CPUTIME_ID` / `GetThreadTimes`), minus the loop's own
//!   timed waits up to their deadlines (the 20 ms frame sleep, the park),
//!   minus waits the slot brackets with [`off_cpu_wait`]. What is left is
//!   time the thread could have run and didn't: a late timer wake and
//!   preemption in the middle of a tick, on both paths. The slot never
//!   waits for its script isolate, and its per-frame locks are held only
//!   briefly by other threads. The long blocks inside a service are
//!   bracketed: GPU set-up, and every socket call the client makes that
//!   can block (a send waiting for a slow network or server to take the
//!   bytes, a read, a close), so outbound backpressure is not counted as
//!   the host being late. Unlike run-queue wait, though, it keeps any
//!   blocking that is not bracketed: a page-in, or the client's shared GPU
//!   scene lock when many bots render. A packet wake that waited for a CPU
//!   before the park bound is inside the park, so it is not seen.
//! - Without a thread CPU clock: how far past its deadline a timed wait
//!   returned. Own work is excluded by construction (a frame that overran
//!   its 20 ms does not sleep, so it has no deadline to miss); preemption
//!   inside a tick is not seen.

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

/// Delays of [`CALM`] or more each slot keeps, for the p95 and the calm
/// timer. Each delay is time the thread spent waiting between its previous
/// service and this one, so the waits of successive services never
/// overlap: all but the oldest of the delays inside a window lie inside it,
/// and at most `WINDOW / CALM + 1` (34) fit. The ring holds every one.
const RING: usize = (WINDOW_MS / CALM.as_millis() as u64) as usize + 1;
/// Delays of [`LATE`] or more each slot keeps, apart from [`RING`], so no
/// shorter delay ever pushes one out: the newest [`MIN_LATE_SERVICES`] are
/// all a bot's count needs.
const LATES: usize = MIN_LATE_SERVICES as usize;
/// Low bits of a packed sample: delay ms (or tick count).
const LOW_BITS: u32 = 24;
const LOW_MASK: u64 = (1 << LOW_BITS) - 1;
/// Shown figures are rounded to this, so the warning changes only when the
/// line would.
const SHOWN_STEP_MS: u64 = 10;

const WINDOW_MS: u64 = WINDOW.as_millis() as u64;
const CLEAR_AFTER_MS: u64 = CLEAR_AFTER.as_millis() as u64;

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
    /// `(at_ms + 1) << 24 | delay_ms` per delay of [`CALM`] or more; 0 is
    /// empty.
    samples: [AtomicU64; RING],
    /// The same, per delay of [`LATE`] or more.
    lates: [AtomicU64; LATES],
    /// `(at_ms + 1) << 24 | ticks mod 2^24` at the newest game tick; 0 none.
    tick: AtomicU64,
    /// Every counted service's delay, for live receipts.
    #[cfg(feature = "test-support")]
    receipt: Mutex<std::collections::VecDeque<(u64, u64)>>,
}

impl SlotCapacity {
    fn new() -> Self {
        Self {
            class: AtomicU8::new(ServiceClass::Parked as u8),
            samples: std::array::from_fn(|_| AtomicU64::new(0)),
            lates: std::array::from_fn(|_| AtomicU64::new(0)),
            tick: AtomicU64::new(0),
            #[cfg(feature = "test-support")]
            receipt: Mutex::default(),
        }
    }

    fn class(&self) -> ServiceClass {
        ServiceClass::from_u8(self.class.load(Ordering::Relaxed))
    }

    fn clear(&self) {
        for cell in self.samples.iter().chain(&self.lates) {
            cell.store(0, Ordering::Relaxed);
        }
        #[cfg(feature = "test-support")]
        self.receipt
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
    }
}

fn pack(at_ms: u64, low: u64) -> u64 {
    ((at_ms + 1) << LOW_BITS) | low.min(LOW_MASK)
}

fn unpack(packed: u64) -> Option<(u64, u64)> {
    (packed != 0).then(|| ((packed >> LOW_BITS) - 1, packed & LOW_MASK))
}

/// What a slot reads at each service to find its scheduling delay.
enum Clock {
    /// Cumulative run-queue wait of this thread.
    #[cfg(target_os = "linux")]
    RunQueue(RunQueueStat),
    /// This thread's CPU time: the delay is the time off the CPU.
    ThreadCpu,
    /// None: the delay is the slot loop's timed-wait overshoot.
    Overshoot,
    /// A test's cumulative run-queue wait.
    #[cfg(test)]
    InjectedWait(std::rc::Rc<std::cell::Cell<Duration>>),
    /// A test's thread CPU time.
    #[cfg(test)]
    InjectedCpu(std::rc::Rc<std::cell::Cell<Duration>>),
}

/// One reading of a [`Clock`].
enum Reading {
    /// Cumulative run-queue wait.
    Waited(Duration),
    /// Cumulative CPU time of the thread.
    Cpu(Duration),
    /// Nothing to read: use the overshoot.
    None,
}

#[cfg(feature = "test-support")]
static FORCE_OFF_CPU: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Slot threads bound after this measure time off the CPU (the macOS and
/// Windows measure) even where Linux keeps run-queue wait, so that measure
/// runs live on Linux. Test builds only.
#[cfg(feature = "test-support")]
pub fn force_off_cpu_measure() {
    FORCE_OFF_CPU.store(true, Ordering::Relaxed);
}

impl Clock {
    /// Run-queue wait where the kernel keeps it; else the thread's CPU
    /// time; else nothing. Call on the slot thread (both are that
    /// thread's).
    fn for_this_thread() -> Self {
        #[cfg(feature = "test-support")]
        let forced = FORCE_OFF_CPU.load(Ordering::Relaxed);
        #[cfg(not(feature = "test-support"))]
        let forced = false;
        #[cfg(target_os = "linux")]
        if !forced {
            if let Some(stat) = RunQueueStat::open() {
                log_source_once("run-queue wait (Linux schedstat)");
                return Self::RunQueue(stat);
            }
        }
        if thread_cpu_time().is_some() {
            log_source_once(if forced {
                "time off the CPU (thread CPU clock; forced by a test)"
            } else {
                "time off the CPU (thread CPU clock)"
            });
            return Self::ThreadCpu;
        }
        log_source_once("timed-wake overshoot");
        Self::Overshoot
    }

    fn read(&self) -> Reading {
        let cpu = |time: Option<Duration>| time.map_or(Reading::None, Reading::Cpu);
        match self {
            #[cfg(target_os = "linux")]
            Self::RunQueue(stat) => stat.waited().map_or(Reading::None, Reading::Waited),
            Self::ThreadCpu => cpu(thread_cpu_time()),
            Self::Overshoot => Reading::None,
            #[cfg(test)]
            Self::InjectedWait(total) => Reading::Waited(total.get()),
            #[cfg(test)]
            Self::InjectedCpu(total) => cpu(Some(total.get())),
        }
    }

    /// This thread's CPU time, for a clock that measures time off the CPU;
    /// no read at all for the others.
    fn cpu(&self) -> Option<Duration> {
        match self {
            Self::ThreadCpu => thread_cpu_time(),
            #[cfg(test)]
            Self::InjectedCpu(total) => Some(total.get()),
            _ => None,
        }
    }
}

/// Time off the CPU: the `wall` time between two services that the thread
/// neither ran (`cpu`) nor spent in waits of its own (`own_waits`).
fn off_cpu(wall: Duration, cpu: Duration, own_waits: Duration) -> Duration {
    wall.saturating_sub(cpu).saturating_sub(own_waits)
}

/// CPU time this thread has run (user + system).
#[cfg(unix)]
#[allow(unsafe_code)]
fn thread_cpu_time() -> Option<Duration> {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `ts` is a valid out pointer; the clock id is a constant.
    let rc = unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut ts) };
    (rc == 0).then(|| {
        Duration::new(
            u64::try_from(ts.tv_sec).unwrap_or(0),
            u32::try_from(ts.tv_nsec).unwrap_or(0),
        )
    })
}

/// CPU time this thread has run (kernel + user, 100 ns units).
#[cfg(windows)]
#[allow(unsafe_code)]
fn thread_cpu_time() -> Option<Duration> {
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::Threading::{GetCurrentThread, GetThreadTimes};
    let zero = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let (mut created, mut exited, mut kernel, mut user) = (zero, zero, zero, zero);
    // SAFETY: the pseudo-handle names this thread and needs no closing; the
    // four out pointers are valid.
    let ok = unsafe {
        GetThreadTimes(
            GetCurrentThread(),
            &mut created,
            &mut exited,
            &mut kernel,
            &mut user,
        )
    };
    let ticks = |t: FILETIME| (u64::from(t.dwHighDateTime) << 32) | u64::from(t.dwLowDateTime);
    (ok != 0).then(|| Duration::from_nanos((ticks(kernel) + ticks(user)).saturating_mul(100)))
}

#[cfg(not(any(unix, windows)))]
fn thread_cpu_time() -> Option<Duration> {
    None
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

/// One slot-loop service, as the loop saw it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Service {
    /// When the loop resumed for it: the sample's time.
    pub(crate) at: Instant,
    /// When its tick returned.
    pub(crate) end: Instant,
    /// Wall time in the loop's own timed waits since the previous service
    /// (the frame sleep, the park), each counted up to its deadline.
    pub(crate) slept: Duration,
    /// How far past those deadlines the waits returned.
    pub(crate) overshoot: Duration,
    /// The client's cumulative `PLAYER_INFO` count.
    pub(crate) ticks: u64,
}

/// The slot thread's writer: its own [`SlotCapacity`] plus the state only
/// this thread reads. Dropping it unregisters the slot.
struct SlotRecorder {
    monitor: Arc<CapacityMonitor>,
    slot: Arc<SlotCapacity>,
    clock: Clock,
    class: ServiceClass,
    /// The previous service was Active: the delay since then counts only
    /// when that one and this one both were.
    counted: bool,
    /// When the previous service ended, and the clock's reading then.
    mark: Option<(Instant, Duration)>,
    /// Time off the CPU inside [`off_cpu_wait`] since the previous service.
    bracketed: Duration,
    cursor: usize,
    late_cursor: usize,
    ticks: u64,
}

impl SlotRecorder {
    fn new(monitor: &Arc<CapacityMonitor>, clock: Clock) -> Self {
        Self {
            monitor: Arc::clone(monitor),
            slot: monitor.register(),
            clock,
            class: ServiceClass::Parked,
            counted: false,
            mark: None,
            bracketed: Duration::ZERO,
            cursor: 0,
            late_cursor: 0,
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
            self.slot.clear();
        }
        self.class = class;
        self.counted = false;
        self.slot.class.store(class as u8, Ordering::Relaxed);
    }

    /// The slot loop starts at `at` (after login): re-baseline, count
    /// nothing until observe reports Active twice in a row.
    fn begin(&mut self, at: Instant, ticks: u64) {
        self.set_class(ServiceClass::Parked);
        self.counted = false;
        self.mark = match self.clock.read() {
            Reading::Waited(total) | Reading::Cpu(total) => Some((at, total)),
            Reading::None => None,
        };
        self.bracketed = Duration::ZERO;
        self.ticks = ticks;
    }

    /// Scheduling delay since the previous service (zero for the first).
    fn delay(&mut self, service: &Service) -> Duration {
        let bracketed = std::mem::take(&mut self.bracketed);
        let (reading, delay) = match self.clock.read() {
            Reading::Waited(total) => (
                Some(total),
                self.mark
                    .map_or(Duration::ZERO, |(_, prev)| total.saturating_sub(prev)),
            ),
            Reading::Cpu(total) => (
                Some(total),
                self.mark.map_or(Duration::ZERO, |(prev_end, prev)| {
                    off_cpu(
                        service.end.saturating_duration_since(prev_end),
                        total.saturating_sub(prev),
                        service.slept + bracketed,
                    )
                }),
            ),
            Reading::None => (None, service.overshoot),
        };
        self.mark = reading.map(|total| (service.end, total));
        delay
    }

    /// One service; returns its delay.
    fn record(&mut self, service: Service) -> Duration {
        let delay = self.delay(&service);
        let active = self.class == ServiceClass::Active;
        if active && self.counted {
            let at_ms = self.monitor.ms(service.at);
            let delay_ms = u64::try_from(delay.as_millis()).unwrap_or(u64::MAX);
            #[cfg(feature = "test-support")]
            {
                let mut receipt = self.slot.receipt.lock().unwrap_or_else(|e| e.into_inner());
                while receipt
                    .front()
                    .is_some_and(|&(at, _)| at_ms.saturating_sub(at) > WINDOW_MS)
                {
                    receipt.pop_front();
                }
                receipt.push_back((at_ms, delay_ms));
            }
            if delay >= CALM {
                self.slot.samples[self.cursor].store(pack(at_ms, delay_ms), Ordering::Relaxed);
                self.cursor = (self.cursor + 1) % RING;
            }
            if delay >= LATE {
                self.slot.lates[self.late_cursor].store(pack(at_ms, delay_ms), Ordering::Relaxed);
                self.late_cursor = (self.late_cursor + 1) % LATES;
            }
        }
        self.counted = active;
        if service.ticks != self.ticks {
            self.ticks = service.ticks;
            self.slot.tick.store(
                pack(self.monitor.ms(service.at), service.ticks & LOW_MASK),
                Ordering::Relaxed,
            );
        }
        delay
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

/// `f` on this thread's recorder, if it has one. Socket calls reach this
/// through the client's hook on any thread, at thread exit too, so a
/// recorder already torn down is simply none.
fn with_recorder<T>(f: impl FnOnce(&mut SlotRecorder) -> T) -> Option<T> {
    RECORDER
        .try_with(|cell| cell.borrow_mut().as_mut().map(f))
        .ok()
        .flatten()
}

/// This slot's CPU time, when it measures time off the CPU.
fn recorder_cpu() -> Option<Duration> {
    with_recorder(|recorder| recorder.clock.cpu()).flatten()
}

/// Run `wait`, a call the slot thread knows can block on the OS or another
/// thread for a long time (GPU initialisation, a socket call), and keep the
/// time it spent off the CPU there out of this slot's scheduling delay.
/// Only the time-off-the-CPU measure needs it: run-queue wait never counts
/// blocking, and timed-wake overshoot doesn't look inside a tick.
pub(crate) fn off_cpu_wait<R>(wait: impl FnOnce() -> R) -> R {
    let Some(cpu_before) = recorder_cpu() else {
        return wait();
    };
    let start = Instant::now();
    let out = wait();
    let wall = start.elapsed();
    if let Some(cpu_after) = recorder_cpu() {
        with_recorder(|recorder| {
            recorder.bracketed +=
                off_cpu(wall, cpu_after.saturating_sub(cpu_before), Duration::ZERO);
        });
    }
    out
}

/// The client's hook for its blocking socket calls: [`off_cpu_wait`].
fn transport_wait(wait: &mut dyn FnMut()) {
    off_cpu_wait(wait);
}

/// Bind this thread's recorder; the client's blocking socket calls are
/// bracketed from now on (the hook is process-wide and set once).
fn bind(recorder: SlotRecorder) {
    client::io::client_stream::set_transport_wait_hook(transport_wait);
    RECORDER.with(|cell| *cell.borrow_mut() = Some(recorder));
}

/// Register this slot thread with a play's monitor (once, at thread start).
pub fn bind_capacity(monitor: &Arc<CapacityMonitor>) {
    bind(SlotRecorder::new(monitor, Clock::for_this_thread()));
}

/// A test's stand-in for the slot thread's clock.
#[cfg(test)]
pub(crate) enum TestClock {
    /// No clock: the loop's overshoot.
    Overshoot,
    /// Cumulative run-queue wait.
    Waited(std::rc::Rc<std::cell::Cell<Duration>>),
    /// Cumulative thread CPU time.
    Cpu(std::rc::Rc<std::cell::Cell<Duration>>),
}

/// Bind this thread with a test's clock.
#[cfg(test)]
pub(crate) fn bind_capacity_for_test(monitor: &Arc<CapacityMonitor>, clock: TestClock) {
    let clock = match clock {
        TestClock::Overshoot => Clock::Overshoot,
        TestClock::Waited(total) => Clock::InjectedWait(total),
        TestClock::Cpu(total) => Clock::InjectedCpu(total),
    };
    bind(SlotRecorder::new(monitor, clock));
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
    with_recorder(|recorder| recorder.begin(Instant::now(), ticks));
}

pub(crate) fn end_services() {
    with_recorder(|recorder| recorder.set_class(ServiceClass::Parked));
}

pub(crate) fn record_service(service: Service) {
    with_recorder(|recorder| recorder.record(service));
}

/// One active bot's delay per counted service over the last [`WINDOW`],
/// for live receipts.
#[cfg(feature = "test-support")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ServiceDelays {
    /// Counted services in the window.
    pub services: usize,
    /// Of those, delayed [`LATE`] or more.
    pub late: usize,
    /// Nearest-rank 95th percentile delay.
    pub p95: Duration,
    /// Longest delay.
    pub max: Duration,
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

    /// Each active bot's delay per counted service over the last
    /// [`WINDOW`], for live receipts: the same delays the warning reads,
    /// short ones included.
    #[cfg(feature = "test-support")]
    pub fn service_delays(&self, now: Instant) -> Vec<ServiceDelays> {
        const LATE_MS: u64 = LATE.as_millis() as u64;
        let now_ms = self.ms(now);
        self.lock()
            .slots
            .iter()
            .filter(|entry| entry.slot.class() == ServiceClass::Active)
            .filter_map(|entry| {
                let receipt = entry.slot.receipt.lock().unwrap_or_else(|e| e.into_inner());
                let mut delays: Vec<u64> = receipt
                    .iter()
                    .filter(|&&(at, _)| now_ms.saturating_sub(at) <= WINDOW_MS)
                    .map(|&(_, delay)| delay)
                    .collect();
                delays.sort_unstable();
                let max = *delays.last()?;
                Some(ServiceDelays {
                    services: delays.len(),
                    late: delays.iter().filter(|&&delay| delay >= LATE_MS).count(),
                    p95: Duration::from_millis(delays[(delays.len() * 95).div_ceil(100) - 1]),
                    max: Duration::from_millis(max),
                })
            })
            .collect()
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
            for cell in &entry.slot.samples {
                let Some((at, delay_ms)) = unpack(cell.load(Ordering::Relaxed)) else {
                    continue;
                };
                if last_unrest.is_none_or(|newest| at > newest) {
                    *last_unrest = Some(at);
                }
                if now_ms.saturating_sub(at) <= WINDOW_MS {
                    delays.push(delay_ms);
                }
            }
            let lates = entry
                .slot
                .lates
                .iter()
                .filter_map(|cell| unpack(cell.load(Ordering::Relaxed)))
                .filter(|&(at, _)| now_ms.saturating_sub(at) <= WINDOW_MS)
                .count();
            if lates >= LATES {
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
