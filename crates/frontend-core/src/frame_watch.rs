//! Frame-stall watchdog shared by the panel and the TUI.
//!
//! Each armed frame stores one timestamp. In debug builds a watcher that sees
//! no heartbeat for 5 s (or a caller-supplied threshold) writes one session-log
//! line naming the stalled frame and that frame thread's stack, once per stall.
//! Release builds do not start a watcher and do not store a timestamp.

#[cfg(debug_assertions)]
use std::cell::OnceCell;
#[cfg(debug_assertions)]
use std::fmt;
#[cfg(debug_assertions)]
use std::num::NonZeroU64;
#[cfg(debug_assertions)]
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
#[cfg(all(
    debug_assertions,
    any(target_vendor = "apple", all(target_os = "linux", target_env = "gnu"))
))]
use std::sync::Mutex;
#[cfg(debug_assertions)]
use std::sync::{Arc, LazyLock};
#[cfg(debug_assertions)]
use std::thread::{self, JoinHandle};
use std::time::Duration;
#[cfg(debug_assertions)]
use std::time::Instant;

#[cfg(debug_assertions)]
use api::host_log;
#[cfg(debug_assertions)]
use api::hostlog::{Category, Level};

/// Stall limit after the first heartbeat.
pub const FRAME_STALL: Duration = Duration::from_secs(5);

#[cfg(debug_assertions)]
static START: LazyLock<Instant> = LazyLock::new(Instant::now);

#[cfg(debug_assertions)]
fn mono_ns() -> u64 {
    START.elapsed().as_nanos().try_into().unwrap_or(u64::MAX)
}

/// Start the debug watcher for `frame` on the current thread. Release builds do nothing.
/// The panel and TUI binaries call this once before their frame loops.
pub fn arm(frame: &'static str) {
    #[cfg(debug_assertions)]
    {
        production(frame).with(|watch| {
            watch.get_or_init(|| FrameWatch::start_with_log(frame, FRAME_STALL, log_watchdog));
        });
    }
    #[cfg(not(debug_assertions))]
    {
        let _ = frame;
    }
}

/// Record a heartbeat for `frame` on the current thread. A no-op until [`arm`]
/// on this thread, and always a no-op in release builds.
pub fn beat(frame: &'static str) {
    #[cfg(debug_assertions)]
    {
        production(frame).with(|watch| {
            if let Some(watch) = watch.get() {
                watch.beat();
            }
        });
    }
    #[cfg(not(debug_assertions))]
    {
        let _ = frame;
    }
}

#[cfg(debug_assertions)]
fn production(frame: &'static str) -> &'static std::thread::LocalKey<OnceCell<FrameWatch>> {
    // Own the watcher on its frame thread so Drop joins the watcher before
    // the target pthread ID can expire.
    thread_local! {
        static PANEL: OnceCell<FrameWatch> = const { OnceCell::new() };
        static TUI: OnceCell<FrameWatch> = const { OnceCell::new() };
    }
    match frame {
        "tui" => &TUI,
        _ => &PANEL,
    }
}

#[cfg(debug_assertions)]
struct Heart {
    last_ns: AtomicU64,
    thread: AtomicU64,
    stop: AtomicBool,
}

/// One named frame heartbeat, owned by the thread it watches.
#[cfg(debug_assertions)]
struct FrameWatch {
    heart: Arc<Heart>,
    join: Option<JoinHandle<()>>,
    _thread_bound: std::marker::PhantomData<*const ()>,
}

#[cfg(debug_assertions)]
impl FrameWatch {
    fn start_with_log(
        name: &'static str,
        threshold: Duration,
        log: impl Fn(fmt::Arguments<'_>) + Send + 'static,
    ) -> Self {
        let heart = Arc::new(Heart {
            last_ns: AtomicU64::new(0),
            thread: AtomicU64::new(0),
            stop: AtomicBool::new(false),
        });
        let watch = Arc::clone(&heart);
        let poll = poll_interval(threshold);
        let threshold_ns = duration_ns(threshold).max(1);
        let handle = thread::Builder::new()
            .name(format!("frame-watch-{name}"))
            .spawn(move || {
                let mut reported_at = 0;
                while !watch.stop.load(Ordering::Acquire) {
                    report_if_stalled(
                        name,
                        threshold,
                        threshold_ns,
                        &watch,
                        &mut reported_at,
                        &log,
                    );
                    thread::park_timeout(poll);
                }
            })
            .expect("frame watcher thread");
        Self {
            heart,
            join: Some(handle),
            _thread_bound: std::marker::PhantomData,
        }
    }

    fn beat(&self) {
        if self.heart.last_ns.load(Ordering::Relaxed) == 0 {
            self.heart.thread.store(current_thread(), Ordering::Relaxed);
        }
        self.heart
            .last_ns
            .store(mono_ns().max(1), Ordering::Release);
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
fn report_if_stalled(
    name: &'static str,
    threshold: Duration,
    threshold_ns: u64,
    heart: &Heart,
    reported_at: &mut u64,
    log: &impl Fn(fmt::Arguments<'_>),
) {
    let last = heart.last_ns.load(Ordering::Acquire);
    if last == 0 || last == *reported_at {
        return;
    }
    let age = mono_ns().saturating_sub(last);
    if age < threshold_ns {
        return;
    }
    // The release of the first heartbeat publishes the pthread ID.
    let Some(native) = NonZeroU64::new(heart.thread.load(Ordering::Relaxed)) else {
        return;
    };
    *reported_at = last;
    log_stall(name, threshold, age, native, log);
}

#[cfg(debug_assertions)]
fn log_stall(
    name: &'static str,
    threshold: Duration,
    age_ns: u64,
    native: NonZeroU64,
    log: &impl Fn(fmt::Arguments<'_>),
) {
    let age = Duration::from_nanos(age_ns);
    log(format_args!(
        "frame stall: {name} no heartbeat for {}ms (limit {}ms)",
        age.as_millis(),
        threshold.as_millis()
    ));
    match capture_stack(native) {
        Some(frames) if !frames.is_empty() => {
            for frame in frames {
                log(format_args!("frame stall stack: {name} {frame}"));
            }
        }
        _ => {
            log(format_args!(
                "frame stall stack: {name} unavailable thread={native:#x}"
            ));
        }
    }
}

#[cfg(debug_assertions)]
fn log_watchdog(message: fmt::Arguments<'_>) {
    host_log!(stderr; Category::Watchdog, Level::Warn, "{message}");
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
fn capture_stack(native: NonZeroU64) -> Option<Vec<String>> {
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
fn capture_stack_signal(native: NonZeroU64) -> Option<Vec<String>> {
    let _guard = CAPTURE_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    static INSTALLED: LazyLock<bool> = LazyLock::new(install_handler);
    if !*INSTALLED {
        return None;
    }
    CAPTURE_READY.store(false, Ordering::Release);
    FRAME_COUNT.store(0, Ordering::Release);
    // SAFETY: beat recorded this nonzero ID from pthread_self on the frame
    // thread. That thread owns the watcher and joins it before exiting.
    // SIGUSR2 has the capture handler installed above.
    let rc = unsafe { libc::pthread_kill(native.get() as libc::pthread_t, libc::SIGUSR2) };
    if rc != 0 {
        return None;
    }
    let deadline = Instant::now() + Duration::from_secs(1);
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
fn install_handler() -> bool {
    #[cfg(target_os = "linux")]
    {
        // Warm up glibc's unwinder so it loads libgcc_s outside the handler.
        let mut frame = std::ptr::null_mut::<std::ffi::c_void>();
        // SAFETY: backtrace can write at most one pointer into `frame`.
        unsafe { libc::backtrace(&mut frame, 1) };
    }
    // SAFETY: sigemptyset initializes the mask. The action points to an
    // extern "C" signal handler, and no old action is requested.
    unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = on_usr2 as *const () as libc::sighandler_t;
        action.sa_flags = libc::SA_RESTART;
        libc::sigemptyset(&mut action.sa_mask);
        libc::sigaction(libc::SIGUSR2, &action, std::ptr::null_mut()) == 0
    }
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
    // SAFETY: `buf` has room for MAX_FRAMES pointers. On Linux installation
    // performs the initial unwind outside the handler. This best-effort
    // diagnostic does not assume backtrace is POSIX async-signal-safe.
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
        if let Some(handle) = self.join.take() {
            handle.thread().unpark();
            let _ = handle.join();
        }
    }
}

#[cfg(all(test, debug_assertions))]
mod tests {
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    use super::{duration_ns, report_if_stalled, FrameWatch};

    #[test]
    fn armed_watch_does_not_report_before_first_heartbeat() {
        let threshold = Duration::from_millis(10);
        let (send, log) = mpsc::channel();
        let check = send.clone();
        let watch = FrameWatch::start_with_log("startup-probe", threshold, move |message| {
            send.send(message.to_string()).unwrap();
        });
        thread::sleep(threshold);
        // Poll explicitly so this test does not depend on when the watcher runs.
        report_if_stalled(
            "startup-probe",
            threshold,
            duration_ns(threshold),
            &watch.heart,
            &mut 0,
            &|message| check.send(message.to_string()).unwrap(),
        );
        drop(watch);
        assert_eq!(log.try_recv(), Err(mpsc::TryRecvError::Empty));
    }

    #[test]
    #[cfg(any(target_vendor = "apple", all(target_os = "linux", target_env = "gnu")))]
    fn blocked_frame_writes_stall_and_stack_to_the_session_log() {
        let (send, log) = mpsc::channel();
        let (finish, blocked) = mpsc::channel::<()>();
        let frame = thread::spawn(move || {
            let watch = FrameWatch::start_with_log(
                "frame-stall-probe",
                Duration::from_millis(30),
                move |message| send.send(message.to_string()).unwrap(),
            );
            watch.beat();
            while blocked.try_recv() == Err(mpsc::TryRecvError::Empty) {
                thread::park_timeout(Duration::from_secs(1));
            }
        });
        let deadline = Duration::from_secs(3);
        let stall = log.recv_timeout(deadline);
        let first_frame = stall
            .as_ref()
            .ok()
            .and_then(|_| log.recv_timeout(deadline).ok());
        drop(finish);
        frame.thread().unpark();
        frame.join().expect("blocked frame thread");

        let stall = stall.expect("session log missing the stall line");
        assert!(
            stall.starts_with("frame stall: frame-stall-probe "),
            "{stall}"
        );
        let frames: Vec<_> = first_frame.into_iter().chain(log.try_iter()).collect();
        let captured = frames
            .iter()
            .filter_map(|line| line.strip_prefix("frame stall stack: frame-stall-probe "))
            .filter(|frame| !frame.starts_with("unavailable"))
            .filter_map(|frame| frame.rsplit_once("0x"))
            .filter_map(|(_, addr)| addr.split(|ch: char| !ch.is_ascii_hexdigit()).next())
            .filter_map(|hex| usize::from_str_radix(hex, 16).ok())
            .filter(|&addr| addr != 0)
            .count();
        assert!(
            captured >= 3,
            "expected real captured stack frames: {frames:?}"
        );
        #[cfg(target_vendor = "apple")]
        assert!(
            frames.iter().any(|frame| {
                frame.contains("park_timeout")
                    || frame.contains("blocked_frame_writes_stall_and_stack_to_the_session_log")
            }),
            "expected the parked frame thread's stack: {frames:?}"
        );
    }
}
