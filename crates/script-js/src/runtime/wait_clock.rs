//! Execution's wait clock: isolate time that stands still while the script
//! is paused (operator Pause, or a reconnect the host relogs through).
//!
//! Frozen `ScriptContext.resume` shifts every parked waiter's `dueAt` and
//! `timeoutAt` by the paused span (`ScriptContext.ts:101-117`), so a
//! `delay` / `delayUntil` parked across a pause resumes with the time it had
//! left. Stopping the clock the waits are measured on is that same shift for
//! every wait at once. `performance.now()` stays wall time, as in frozen.
//!
//! One isolate per thread, so the paused span is thread-local.

use std::cell::Cell;
use std::time::{Duration, Instant};

thread_local! {
    /// Paused time already over.
    static PAUSED: Cell<Duration> = const { Cell::new(Duration::ZERO) };
    /// Start of the pause in progress.
    static PAUSED_SINCE: Cell<Option<Instant>> = const { Cell::new(None) };
}

/// Stop or restart the clock. Idempotent in both directions.
pub(crate) fn set_paused(paused: bool, now: Instant) {
    match (paused, PAUSED_SINCE.get()) {
        (true, None) => PAUSED_SINCE.set(Some(now)),
        (false, Some(since)) => {
            PAUSED.set(PAUSED.get() + now.saturating_duration_since(since));
            PAUSED_SINCE.set(None);
        }
        _ => {}
    }
}

/// Milliseconds since `start`, less every paused span.
pub(crate) fn now_ms(start: Instant, now: Instant) -> f64 {
    let current = PAUSED_SINCE
        .get()
        .map_or(Duration::ZERO, |since| now.saturating_duration_since(since));
    now.saturating_duration_since(start)
        .saturating_sub(PAUSED.get() + current)
        .as_millis() as f64
}
