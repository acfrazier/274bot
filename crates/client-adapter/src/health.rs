//! Reactor progress and one-shot process availability observation.
//!
//! Initial OS trust/verifier calls run on the dedicated handshake lane, so
//! their elapsed time is never interpreted as reactor loss. An actual reactor
//! stall, start failure, or panic latches permanently until process restart.

use std::io;
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub const REACTOR_POLL_MAX: Duration = Duration::from_millis(100);
pub const REACTOR_STALL_LIMIT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum ReactorLoss {
    StartFailed,
    Panicked,
    ProgressLost,
}

impl ReactorLoss {
    fn code(self) -> u8 {
        match self {
            Self::StartFailed => 1,
            Self::Panicked => 2,
            Self::ProgressLost => 3,
        }
    }

    fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::StartFailed),
            2 => Some(Self::Panicked),
            3 => Some(Self::ProgressLost),
            _ => None,
        }
    }
}
/// A committed record stays accepted when its wake fails. Health reports
/// this diagnostic without latching a process-wide reactor failure.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct WakeFailure {
    pub total: u64,
    pub kind: io::ErrorKind,
    pub raw_os_error: Option<i32>,
}

/// `epoch` is one fixed process incarnation origin. A single reactor writer
/// updates progress; composition checkers serialize their observation window.
#[derive(Debug)]
pub struct ReactorHealth {
    epoch: Instant,
    progress_ns: AtomicU64,
    graceful_close_timeouts: AtomicU64,
    graceful_close_drain_errors: AtomicU64,
    graceful_close_shutdown_drops: AtomicU64,
    checker: Mutex<CheckerWindow>,
    wake_failure: Mutex<Option<WakeFailure>>,
    latched: AtomicU8,
}

#[derive(Debug, Default)]
struct CheckerWindow {
    last_turn: Option<Instant>,
    progress_ns: u64,
    observed_stale: Duration,
}

impl ReactorHealth {
    pub fn new(epoch: Instant) -> Self {
        Self {
            epoch,
            progress_ns: AtomicU64::new(0),
            graceful_close_timeouts: AtomicU64::new(0),
            graceful_close_drain_errors: AtomicU64::new(0),
            graceful_close_shutdown_drops: AtomicU64::new(0),
            checker: Mutex::new(CheckerWindow::default()),
            wake_failure: Mutex::new(None),
            latched: AtomicU8::new(0),
        }
    }

    /// Reactor marks poll entry/exit and each finite turn. Conversion
    /// overflow is a local service loss, never silently wrapped time.
    pub fn progress(&self, at: Instant) {
        let nanos = at.saturating_duration_since(self.epoch).as_nanos();
        let Ok(nanos) = u64::try_from(nanos) else {
            self.latch(ReactorLoss::ProgressLost);
            return;
        };
        self.progress_ns.store(nanos, Ordering::Release);
    }

    /// Count a bounded session close that reached its flush or peer-drain
    /// deadline. This does not make the process network service unavailable.
    pub fn record_graceful_close_timeout(&self) {
        self.graceful_close_timeouts.fetch_add(1, Ordering::Relaxed);
    }

    pub fn graceful_close_timeouts(&self) -> u64 {
        self.graceful_close_timeouts.load(Ordering::Relaxed)
    }
    /// Count a peer reset or fatal TLS alert while discarding a stopped
    /// session's transport tail. A clean peer close is not an error.
    pub fn record_graceful_close_drain_error(&self) {
        self.graceful_close_drain_errors
            .fetch_add(1, Ordering::Relaxed);
    }

    pub fn graceful_close_drain_errors(&self) -> u64 {
        self.graceful_close_drain_errors.load(Ordering::Relaxed)
    }

    /// Count each unfinished drain that service shutdown retires.
    pub fn record_graceful_close_shutdown_drop(&self) {
        self.graceful_close_shutdown_drops
            .fetch_add(1, Ordering::Relaxed);
    }

    pub fn graceful_close_shutdown_drops(&self) -> u64 {
        self.graceful_close_shutdown_drops.load(Ordering::Relaxed)
    }


    pub fn record_wake_failure(&self, error: &io::Error) {
        let mut last = self
            .wake_failure
            .lock()
            .expect("wake failure mutex is not poisoned");
        let total = last.map_or(1, |failure| {
            failure
                .total
                .checked_add(1)
                .expect("wake failure count exhausted")
        });
        *last = Some(WakeFailure {
            total,
            kind: error.kind(),
            raw_os_error: error.raw_os_error(),
        });
    }

    pub fn wake_failure(&self) -> Option<WakeFailure> {
        *self
            .wake_failure
            .lock()
            .expect("wake failure mutex is not poisoned")
    }

    /// The checker must observe five seconds without reactor progress. A gap
    /// longer than two reactor poll intervals is not continuous observation:
    /// even a sub-limit process pause restarts the window.
    pub fn maintain(&self, now: Instant) -> Option<ReactorLoss> {
        if let Some(loss) = self.loss() {
            return Some(loss);
        }
        let mut checker = self.checker.lock().expect("checker window is not poisoned");
        if checker.last_turn.is_some_and(|last| now <= last) {
            return self.loss();
        }
        let progress = self.progress_ns.load(Ordering::Acquire);
        let gap = checker
            .last_turn
            .map(|last| now.saturating_duration_since(last));
        if progress != checker.progress_ns || gap.is_none_or(|gap| gap > REACTOR_POLL_MAX * 2) {
            checker.observed_stale = Duration::ZERO;
        } else if let Some(gap) = gap {
            checker.observed_stale += gap;
        }
        checker.progress_ns = progress;
        checker.last_turn = Some(now);
        if checker.observed_stale >= REACTOR_STALL_LIMIT {
            // Do not latch a progress mark published during this check.
            if self.progress_ns.load(Ordering::Acquire) == progress {
                self.latch(ReactorLoss::ProgressLost);
            } else {
                checker.observed_stale = Duration::ZERO;
            }
        }
        self.loss()
    }

    /// Called only for OS thread-start failure or an uncaught fixed service
    /// worker/reactor panic; returned construction errors stay connection-local.
    pub fn latch(&self, cause: ReactorLoss) {
        let _ = self
            .latched
            .compare_exchange(0, cause.code(), Ordering::AcqRel, Ordering::Acquire);
    }

    pub fn loss(&self) -> Option<ReactorLoss> {
        ReactorLoss::from_code(self.latched.load(Ordering::Acquire))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_five_second_reactor_stall_latches_once() {
        let base = Instant::now();
        let health = ReactorHealth::new(base);
        health.progress(base);
        assert_eq!(health.maintain(base), None);
        for tenth in 1..50 {
            assert_eq!(
                health.maintain(base + Duration::from_millis(tenth * 100)),
                None
            );
        }
        assert_eq!(
            health.maintain(
                (base + REACTOR_STALL_LIMIT)
                    .checked_sub(Duration::from_nanos(1))
                    .expect("stall test window exceeds one nanosecond"),
            ),
            None
        );
        assert_eq!(
            health.maintain(base + REACTOR_STALL_LIMIT),
            Some(ReactorLoss::ProgressLost)
        );
        health.progress(base + REACTOR_STALL_LIMIT + Duration::from_secs(20));
        assert_eq!(
            health.maintain(base + Duration::from_secs(30)),
            Some(ReactorLoss::ProgressLost)
        );
    }

    #[test]
    fn process_pause_restarts_stall_window_but_reactor_stall_does_not() {
        let base = Instant::now();
        let health = ReactorHealth::new(base);
        health.progress(base);
        assert_eq!(health.maintain(base), None);
        let resumed = base + Duration::from_secs(20);
        assert_eq!(health.maintain(resumed), None);
        for tenth in 1..=40 {
            assert_eq!(
                health.maintain(resumed + Duration::from_millis(tenth * 100)),
                None
            );
        }
        assert_eq!(health.maintain(resumed + REACTOR_STALL_LIMIT), None);

        let stalled = ReactorHealth::new(base);
        stalled.progress(base);
        assert_eq!(stalled.maintain(base), None);
        for tenth in 1..=50 {
            let now = base + Duration::from_millis(tenth * 100);
            assert_eq!(
                stalled.maintain(now),
                (tenth == 50).then_some(ReactorLoss::ProgressLost)
            );
        }
    }

    #[test]
    fn sub_limit_process_pause_does_not_count_as_observed_stall() {
        use std::sync::{Arc, Barrier};

        let base = Instant::now();
        let health = Arc::new(ReactorHealth::new(base));
        health.progress(base);
        for tick in 0..=5 {
            assert_eq!(
                health.maintain(base + Duration::from_millis(tick * 20)),
                None
            );
        }
        health.progress(base + Duration::from_millis(100));
        for tick in 6..=15 {
            assert_eq!(
                health.maintain(base + Duration::from_millis(tick * 10)),
                None
            );
        }
        // The reactor mark is 50 ms older than the last checker turn.
        // Both checkers resume after a 4.97 s process-wide pause.
        let resumed = base + Duration::from_millis(5120);
        let barrier = Arc::new(Barrier::new(3));
        std::thread::scope(|scope| {
            for _ in 0..2 {
                let barrier = Arc::clone(&barrier);
                let health = Arc::clone(&health);
                scope.spawn(move || {
                    barrier.wait();
                    assert_eq!(health.maintain(resumed), None);
                });
            }
            barrier.wait();
        });
        assert_eq!(health.loss(), None);
        // Continuous checks after the pause still catch a real stalled reactor.
        for tenth in 1..=50 {
            assert_eq!(
                health.maintain(resumed + Duration::from_millis(tenth * 100)),
                (tenth == 50).then_some(ReactorLoss::ProgressLost)
            );
        }
    }

    #[test]
    fn start_failure_or_panic_never_turns_into_progress_lost() {
        let base = Instant::now();
        let health = ReactorHealth::new(base);
        health.latch(ReactorLoss::StartFailed);
        health.latch(ReactorLoss::Panicked);
        assert_eq!(
            health.maintain(base + Duration::from_secs(900)),
            Some(ReactorLoss::StartFailed)
        );
        let other = ReactorHealth::new(base);
        other.latch(ReactorLoss::Panicked);
        assert_eq!(other.maintain(base), Some(ReactorLoss::Panicked));
    }

    #[test]
    fn frequent_owner_turns_do_not_mistake_worker_hold_for_stall() {
        let base = Instant::now();
        let health = ReactorHealth::new(base);
        for tick in 0..=9000 {
            let now = base + Duration::from_millis(tick * 100);
            health.progress(now);
            assert_eq!(health.maintain(now), None);
        }
    }
}
