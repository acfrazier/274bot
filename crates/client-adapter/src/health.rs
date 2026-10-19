//! Reactor progress and one-shot process availability observation.
//!
//! Initial OS trust/verifier calls run on the dedicated handshake lane, so
//! their elapsed time is never interpreted as reactor loss. An actual reactor
//! stall, start failure, or panic latches permanently until process restart.

use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
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

/// `epoch` is one fixed process incarnation origin. A single reactor writer
/// updates progress; the composition owner samples it on independent turns.
#[derive(Debug)]
pub struct ReactorHealth {
    epoch: Instant,
    progress_ns: AtomicU64,
    progress_count: AtomicU64,
    latched: AtomicU8,
}

impl ReactorHealth {
    pub fn new(epoch: Instant) -> Self {
        Self {
            epoch,
            progress_ns: AtomicU64::new(0),
            progress_count: AtomicU64::new(0),
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
        self.progress_count.fetch_add(1, Ordering::Relaxed);
    }

    /// A driver independent of a status consumer calls this at least once
    /// per requested 100 ms interval; cheap due-checks may call it more often.
    pub fn maintain(&self, now: Instant) -> Option<ReactorLoss> {
        if let Some(loss) = self.loss() {
            return Some(loss);
        }
        let elapsed = now.saturating_duration_since(self.epoch);
        let observed = self.progress_ns.load(Ordering::Acquire);
        // The count distinguishes a reactor that never entered Poll from one
        // that made an initial t=0 progress mark.
        let stale = self.progress_count.load(Ordering::Acquire) == 0
            || elapsed.as_nanos().saturating_sub(u128::from(observed))
                >= REACTOR_STALL_LIMIT.as_nanos();
        if stale && elapsed >= REACTOR_STALL_LIMIT {
            self.latch(ReactorLoss::ProgressLost);
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
        assert_eq!(
            health.maintain(base + REACTOR_STALL_LIMIT - Duration::from_nanos(1)),
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
