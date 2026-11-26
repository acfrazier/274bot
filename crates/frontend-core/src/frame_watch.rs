//! Frame-stall watchdog shared by the panel and the TUI.
//!
//! Each armed frame stores one timestamp. In debug builds a watcher that sees
//! no heartbeat for 5 s (or a caller-supplied threshold) writes one session-log
//! line naming the stalled frame and that frame thread's stack, once per stall.
//! Release builds do not start a watcher and do not store a timestamp.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex, Once};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use api::host_log;
use api::hostlog::{Category, Level};

/// Production stall limit. Tests pass a shorter threshold to [`FrameWatch::start`].
pub const FRAME_STALL: Duration = Duration::from_secs(5);

static ARMED: AtomicBool = AtomicBool::new(false);
static START: LazyLock<Instant> = LazyLock::new(Instant::now);

fn mono_ns() -> u64 {
    START.elapsed().as_nanos().try_into().unwrap_or(u64::MAX)
}

/// Start the debug watcher for `frame`. Release builds do nothing.
/// The panel and TUI binaries call this once before their frame loops.
pub fn arm(frame: &'static str) {
    #[cfg(debug_assertions)]
    {
        let _watch = production(frame);
        ARMED.store(true, Ordering::Release);
    }
    #[cfg(not(debug_assertions))]
    {
        let _ = frame;
    }
}

/// One heartbeat for `frame`. A no-op until [`arm`], and always a no-op in
/// release builds.
pub fn beat(frame: &'static str) {
    #[cfg(debug_assertions)]
    {
        if ARMED.load(Ordering::Acquire) {
            production(frame).beat();
        }
    }
    #[cfg(not(debug_assertions))]
    {
        let _ = frame;
    }
}

#[cfg(debug_assertions)]
fn production(frame: &'static str) -> &'static FrameWatch {
    static PANEL: LazyLock<FrameWatch> = LazyLock::new(|| FrameWatch::start("panel", FRAME_STALL));
    static TUI: LazyLock<FrameWatch> = LazyLock::new(|| FrameWatch::start("tui", FRAME_STALL));
    match frame {
        "tui" => &TUI,
        _ => &PANEL,
    }
}

struct Heart {
    started_ns: u64,
    last_ns: AtomicU64,
    thread: AtomicU64,
    reported_at: AtomicU64,
    stop: AtomicBool,
}

/// One named frame heartbeat and, in debug builds, its stall watcher.
pub struct FrameWatch {
    #[cfg(debug_assertions)]
    heart: std::sync::Arc<Heart>,
    #[cfg(debug_assertions)]
    join: Mutex<Option<JoinHandle<()>>>,
}

impl FrameWatch {
    /// `threshold` is the stall limit. Production uses [`FRAME_STALL`].
    pub fn start(name: &'static str, threshold: Duration) -> Self {
        #[cfg(debug_assertions)]
        {
            let heart = std::sync::Arc::new(Heart {
                started_ns: mono_ns().max(1),
                last_ns: AtomicU64::new(0),
                thread: AtomicU64::new(0),
                reported_at: AtomicU64::new(0),
                stop: AtomicBool::new(false),
            });
            let watch = std::sync::Arc::clone(&heart);
            let poll = poll_interval(threshold);
            let threshold_ns = duration_ns(threshold).max(1);
            let handle = thread::Builder::new()
                .name(format!("frame-watch-{name}"))
                .spawn(move || {
                    while !watch.stop.load(Ordering::Acquire) {
                        report_if_stalled(name, threshold, threshold_ns, &watch);
                        thread::park_timeout(poll);
                    }
                })
                .expect("frame watcher thread");
            Self {
                heart,
                join: Mutex::new(Some(handle)),
            }
        }
        #[cfg(not(debug_assertions))]
        {
            let _ = (name, threshold);
            Self {}
        }
    }

    /// Store this thread's heartbeat. One atomic store.
    pub fn beat(&self) {
        #[cfg(debug_assertions)]
        {
            self.heart.thread.store(current_thread(), Ordering::Relaxed);
            self.heart
                .last_ns
                .store(mono_ns().max(1), Ordering::Release);
        }
    }
}

#[cfg(debug_assertions)]
fn duration_ns(threshold: Duration) -> u64 {
    threshold.as_nanos().try_into().unwrap_or(u64::MAX)
}

#[cfg(debug_assertions)]
fn poll_interval(threshold: Duration) -> Duration {
    let quarter = threshold / 4;
    if quarter < Duration::from_millis(5) {
        Duration::from_millis(5)
    } else if quarter > Duration::from_millis(250) {
        Duration::from_millis(250)
    } else {
        quarter
    }
}

#[cfg(debug_assertions)]
fn report_if_stalled(name: &'static str, threshold: Duration, threshold_ns: u64, heart: &Heart) {
    let last = heart.last_ns.load(Ordering::Acquire);
    let native = heart.thread.load(Ordering::Acquire);
    if last == 0 || native == 0 {
        let age = mono_ns().saturating_sub(heart.started_ns);
        if age < threshold_ns {
            return;
        }
        if heart.reported_at.swap(heart.started_ns, Ordering::AcqRel) == heart.started_ns {
            return;
        }
        log_stall(name, threshold, age, native);
        return;
    }
    let age = mono_ns().saturating_sub(last);
    if age < threshold_ns {
        return;
    }
    if heart.reported_at.swap(last, Ordering::AcqRel) == last {
        return;
    }
    log_stall(name, threshold, age, native);
}

#[cfg(debug_assertions)]
fn log_stall(name: &'static str, threshold: Duration, age_ns: u64, native: u64) {
    let age = Duration::from_nanos(age_ns);
    host_log!(
        stderr;
        Category::Watchdog,
        Level::Warn,
        "frame stall: {name} no heartbeat for {}ms (limit {}ms)",
        age.as_millis(),
        threshold.as_millis()
    );
    match capture_stack(native) {
        Some(frames) if !frames.is_empty() => {
            for frame in frames {
                host_log!(
                    stderr;
                    Category::Watchdog,
                    Level::Warn,
                    "frame stall stack: {name} {frame}"
                );
            }
        }
        _ => {
            host_log!(
                stderr;
                Category::Watchdog,
                Level::Warn,
                "frame stall stack: {name} unavailable thread={native:#x}"
            );
        }
    }
}

#[cfg(debug_assertions)]
fn current_thread() -> u64 {
    #[cfg(any(target_vendor = "apple", all(target_os = "linux", target_env = "gnu")))]
    {
        // SAFETY: pthread_self returns this thread's id and does not
        // dereference caller memory.
        unsafe { libc::pthread_self() as u64 }
    }
    #[cfg(not(any(target_vendor = "apple", all(target_os = "linux", target_env = "gnu"))))]
    {
        1
    }
}

#[cfg(debug_assertions)]
fn capture_stack(native: u64) -> Option<Vec<String>> {
    #[cfg(any(target_vendor = "apple", all(target_os = "linux", target_env = "gnu")))]
    {
        capture_stack_signal(native)
    }
    #[cfg(not(any(target_vendor = "apple", all(target_os = "linux", target_env = "gnu"))))]
    {
        let _ = native;
        None
    }
}

#[cfg(all(
    debug_assertions,
    any(target_vendor = "apple", all(target_os = "linux", target_env = "gnu"))
))]
fn capture_stack_signal(native: u64) -> Option<Vec<String>> {
    let _guard = CAPTURE_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    install_handler();
    CAPTURE_READY.store(false, Ordering::Release);
    FRAME_COUNT.store(0, Ordering::Release);
    // SAFETY: `native` was stored from pthread_self on the frame thread.
    // The handler only writes the capture buffer.
    let rc = unsafe { libc::pthread_kill(native as libc::pthread_t, libc::SIGUSR2) };
    if rc != 0 {
        return None;
    }
    let deadline = Instant::now() + Duration::from_millis(50);
    while !CAPTURE_READY.load(Ordering::Acquire) {
        if Instant::now() >= deadline {
            return None;
        }
        thread::sleep(Duration::from_millis(1));
    }
    let count = FRAME_COUNT.load(Ordering::Acquire).min(MAX_FRAMES);
    let mut addrs = Vec::with_capacity(count);
    for slot in FRAME_PTRS.iter().take(count) {
        let addr = slot.load(Ordering::Acquire);
        if addr != 0 {
            addrs.push(addr);
        }
    }
    Some(symbolize(&addrs))
}

#[cfg(all(
    debug_assertions,
    any(target_vendor = "apple", all(target_os = "linux", target_env = "gnu"))
))]
fn symbolize(addrs: &[usize]) -> Vec<String> {
    if addrs.is_empty() {
        return Vec::new();
    }
    let ptrs: Vec<*mut std::ffi::c_void> = addrs
        .iter()
        .map(|addr| *addr as *mut std::ffi::c_void)
        .collect();
    // SAFETY: `ptrs` holds the captured instruction pointers. backtrace_symbols
    // allocates one table; free releases that table after the strings are copied.
    unsafe {
        let symbols = libc::backtrace_symbols(ptrs.as_ptr(), ptrs.len() as libc::c_int);
        if symbols.is_null() {
            return addrs.iter().map(|addr| format!("{addr:#x}")).collect();
        }
        let mut lines = Vec::with_capacity(addrs.len());
        for (index, addr) in addrs.iter().copied().enumerate() {
            let text = std::ffi::CStr::from_ptr(*symbols.add(index))
                .to_string_lossy()
                .replace(['\n', '\r', '\t'], " ");
            let line = if text.contains("0x") {
                text
            } else {
                format!("{addr:#x} {text}")
            };
            lines.push(truncate_line(&line));
        }
        libc::free(symbols.cast());
        lines
    }
}

#[cfg(all(
    debug_assertions,
    any(target_vendor = "apple", all(target_os = "linux", target_env = "gnu"))
))]
fn truncate_line(text: &str) -> String {
    text.chars().take(180).collect()
}

#[cfg(all(
    debug_assertions,
    any(target_vendor = "apple", all(target_os = "linux", target_env = "gnu"))
))]
fn install_handler() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        // SAFETY: the handler calls backtrace and stores addresses in
        // lock-free atomics. It does not allocate.
        unsafe {
            let mut action: libc::sigaction = std::mem::zeroed();
            action.sa_sigaction = on_usr2 as *const () as libc::sighandler_t;
            action.sa_flags = libc::SA_RESTART;
            libc::sigemptyset(&mut action.sa_mask);
            libc::sigaction(libc::SIGUSR2, &action, std::ptr::null_mut());
        }
    });
}

#[cfg(all(
    debug_assertions,
    any(target_vendor = "apple", all(target_os = "linux", target_env = "gnu"))
))]
const MAX_FRAMES: usize = 48;

#[cfg(all(
    debug_assertions,
    any(target_vendor = "apple", all(target_os = "linux", target_env = "gnu"))
))]
static FRAME_PTRS: [std::sync::atomic::AtomicUsize; MAX_FRAMES] =
    [const { std::sync::atomic::AtomicUsize::new(0) }; MAX_FRAMES];

#[cfg(all(
    debug_assertions,
    any(target_vendor = "apple", all(target_os = "linux", target_env = "gnu"))
))]
static FRAME_COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

#[cfg(all(
    debug_assertions,
    any(target_vendor = "apple", all(target_os = "linux", target_env = "gnu"))
))]
static CAPTURE_READY: AtomicBool = AtomicBool::new(false);

#[cfg(all(
    debug_assertions,
    any(target_vendor = "apple", all(target_os = "linux", target_env = "gnu"))
))]
static CAPTURE_LOCK: Mutex<()> = Mutex::new(());

#[cfg(all(
    debug_assertions,
    any(target_vendor = "apple", all(target_os = "linux", target_env = "gnu"))
))]
extern "C" fn on_usr2(_signal: libc::c_int) {
    let mut buf = [std::ptr::null_mut::<std::ffi::c_void>(); MAX_FRAMES];
    // SAFETY: backtrace is the signal-safe unwind this handler exists to call.
    // The buffer is stack memory owned by this invocation.
    let count = unsafe { libc::backtrace(buf.as_mut_ptr(), MAX_FRAMES as libc::c_int) };
    let count = usize::try_from(count).unwrap_or(0).min(MAX_FRAMES);
    for (index, frame) in buf.iter().take(count).enumerate() {
        FRAME_PTRS[index].store(*frame as usize, Ordering::Relaxed);
    }
    FRAME_COUNT.store(count, Ordering::Release);
    CAPTURE_READY.store(true, Ordering::Release);
}

#[cfg(debug_assertions)]
impl Drop for FrameWatch {
    fn drop(&mut self) {
        self.heart.stop.store(true, Ordering::Release);
        if let Some(handle) = self
            .join
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        {
            handle.thread().unpark();
            let _ = handle.join();
        }
    }
}

#[cfg(all(test, debug_assertions))]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use super::FrameWatch;

    #[test]
    fn blocked_frame_writes_stall_and_stack_to_the_session_log() {
        let _store = crate::log::global();
        let watch = Arc::new(FrameWatch::start(
            "frame-stall-probe",
            Duration::from_millis(30),
        ));
        let stop = Arc::new(AtomicBool::new(false));
        let frame = {
            let watch = Arc::clone(&watch);
            let stop = Arc::clone(&stop);
            std::thread::Builder::new()
                .name("frame-stall-probe".to_owned())
                .spawn(move || {
                    watch.beat();
                    while !stop.load(Ordering::Acquire) {
                        std::thread::park_timeout(Duration::from_millis(20));
                    }
                })
                .expect("blocked frame thread")
        };
        let deadline = Instant::now() + Duration::from_millis(400);
        let mut saw_stall = false;
        let mut saw_stack = false;
        while Instant::now() < deadline {
            let mut view = crate::log::LogView::new(crate::log::LogScope::Process);
            crate::log::global().refresh(&mut view);
            for row in view.rows() {
                let message = row.message();
                if message.contains("frame stall: frame-stall-probe") {
                    saw_stall = true;
                }
                if message.contains("frame stall stack: frame-stall-probe")
                    && message.contains("0x")
                {
                    saw_stack = true;
                }
            }
            if saw_stall && saw_stack {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        stop.store(true, Ordering::Release);
        frame.thread().unpark();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = frame.join();
            let _ = done_tx.send(());
        });
        let _ = done_rx.recv_timeout(Duration::from_millis(200));
        drop(watch);
        assert!(saw_stall, "session log missing the stall line");
        assert!(saw_stack, "session log missing the frame stack");
    }
}
