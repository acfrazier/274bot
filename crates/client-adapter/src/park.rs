//! Per-slot K2 Park: publication, capacity marker, and absolute waits.
//!
//! Producers publish bytes, flags, or reply cells *before* acquiring K2.
//! K2 orders an armed writer against a reactor drain. Neither side tests the
//! marker outside K2; notifications happen after releasing it. No socket I/O
//! or other owner lock is allowed in a Park predicate.

use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

#[cfg(debug_assertions)]
thread_local! {
    static K2_PREDICATE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// True only while a Park wait invokes predicates under K2. Resolver state
/// acquisition checks this in debug builds so a new nested lock fails fast.
#[cfg(debug_assertions)]
pub(crate) fn in_k2_predicate() -> bool {
    K2_PREDICATE.with(std::cell::Cell::get)
}

#[cfg(debug_assertions)]
struct K2PredicateGuard;

#[cfg(debug_assertions)]
impl K2PredicateGuard {
    fn enter() -> Self {
        K2_PREDICATE.with(|held| {
            debug_assert!(!held.get(), "nested K2 predicate");
            held.set(true);
        });
        Self
    }
}

#[cfg(debug_assertions)]
impl Drop for K2PredicateGuard {
    fn drop(&mut self) {
        K2_PREDICATE.with(|held| held.set(false));
    }
}

const MAX_HEALTH_WAIT: Duration = Duration::from_millis(100);

/// Exact connection and binding identity of a writer waiting for ring space.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WriterBinding {
    pub connection: u64,
    pub binding: u64,
}

#[derive(Debug, Default)]
struct ParkState {
    epoch: u64,
    writer: Option<WriterBinding>,
}

/// A slot-owned wait, shared by all transport producers that can wake it.
#[derive(Debug, Default)]
pub struct Park {
    state: Mutex<ParkState>,
    wake: Condvar,
}

/// The terminal/result precedence determined at the slot wait boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParkOutcome<T> {
    Ready(T),
    Unavailable,
    SelectedIntent,
    Revoked,
    Deadline,
}

impl Park {
    pub fn new() -> Self {
        Self::default()
    }

    /// Publish a previously committed result or revocation and wake the slot.
    pub fn publish(&self) {
        {
            let mut state = self.state.lock().expect("Park K2 state is not poisoned");
            state.epoch = state.epoch.wrapping_add(1);
        }
        self.wake.notify_all();
    }

    /// Select a token's one-shot revocation under K2. Its publication and
    /// every login read/write under-K2 check now have one linear order. Fire
    /// this only after releasing the intent/queue owner lock.
    pub fn publish_revocation(&self, fired: &std::sync::atomic::AtomicBool) {
        let changed = {
            let mut state = self.state.lock().expect("Park K2 state is not poisoned");
            if fired.swap(true, std::sync::atomic::Ordering::AcqRel) {
                false
            } else {
                state.epoch = state.epoch.wrapping_add(1);
                true
            }
        };
        if changed {
            self.wake.notify_all();
        }
    }

    /// The reactor calls this after freeing output ring space. The marker is
    /// read *under K2*, including the case where no writer was registered.
    /// In-game bindings do not need a per-drain login wake.
    pub fn publish_freed_space(&self, binding: WriterBinding) {
        let armed = {
            let mut state = self.state.lock().expect("Park K2 state is not poisoned");
            if state.writer == Some(binding) {
                state.epoch = state.epoch.wrapping_add(1);
                true
            } else {
                false
            }
        };
        if armed {
            self.wake.notify_all();
        }
    }

    /// Observe the K2 publication counter for deterministic tests/diagnostics.
    #[cfg(any(test, feature = "test-support"))]
    pub fn epoch(&self) -> u64 {
        self.state
            .lock()
            .expect("Park K2 state is not poisoned")
            .epoch
    }

    /// Check all predicates before accepting ready work, including on a fast
    /// path. A caller may invoke this without taking K2 only if readiness and
    /// every terminal predicate are atomic publications; `wait` rechecks K2.
    pub fn try_ready<T>(
        &self,
        deadline: Instant,
        now: impl FnOnce() -> Instant,
        unavailable: impl FnOnce() -> bool,
        selected_intent: impl FnOnce() -> bool,
        revoked: impl FnOnce() -> bool,
        ready: impl FnOnce() -> Option<T>,
    ) -> Option<ParkOutcome<T>> {
        Self::check(
            deadline,
            now,
            unavailable,
            selected_intent,
            revoked,
            ready,
            false,
        )
    }

    /// Wait for a generation-matched result. The absolute deadline never
    /// restarts on spurious wakes; health maintenance gets a turn every 100 ms.
    /// `receipt_first` is true *only* for a Seal detachment receipt published
    /// before the handshake deadline (finalization §2.1), not for game I/O.
    #[expect(
        clippy::too_many_arguments,
        reason = "Park checks ordered terminal predicates and readiness under K2"
    )]
    pub fn wait<T>(
        &self,
        deadline: Instant,
        mut now: impl FnMut() -> Instant,
        mut unavailable: impl FnMut() -> bool,
        mut selected_intent: impl FnMut() -> bool,
        mut revoked: impl FnMut() -> bool,
        mut ready: impl FnMut() -> Option<T>,
        receipt_first: bool,
        writer: Option<WriterBinding>,
    ) -> ParkOutcome<T> {
        let mut state = self.state.lock().expect("Park K2 state is not poisoned");
        if let Some(binding) = writer {
            assert!(state.writer.is_none() || state.writer == Some(binding));
            state.writer = Some(binding);
        }
        let result = loop {
            let outcome = {
                #[cfg(debug_assertions)]
                let _k2_predicate = K2PredicateGuard::enter();
                Self::check(
                    deadline,
                    &mut now,
                    &mut unavailable,
                    &mut selected_intent,
                    &mut revoked,
                    &mut ready,
                    receipt_first,
                )
            };
            if let Some(outcome) = outcome {
                break outcome;
            }
            let remaining = deadline.saturating_duration_since(now());
            let bound = remaining.min(MAX_HEALTH_WAIT);
            let (next, _) = self
                .wake
                .wait_timeout(state, bound)
                .expect("Park K2 state is not poisoned");
            state = next;
        };
        if let Some(binding) = writer {
            assert_eq!(state.writer, Some(binding));
            state.writer = None;
        }
        result
    }

    fn check<T>(
        deadline: Instant,
        now: impl FnOnce() -> Instant,
        unavailable: impl FnOnce() -> bool,
        selected_intent: impl FnOnce() -> bool,
        revoked: impl FnOnce() -> bool,
        ready: impl FnOnce() -> Option<T>,
        receipt_first: bool,
    ) -> Option<ParkOutcome<T>> {
        // A previously selected intent terminal cannot be overwritten by a
        // later service latch. Both precede byte/readiness inspection.
        if selected_intent() {
            return Some(ParkOutcome::SelectedIntent);
        }
        if unavailable() {
            return Some(ParkOutcome::Unavailable);
        }
        if revoked() {
            return Some(ParkOutcome::Revoked);
        }
        if receipt_first {
            // A timely positive Seal receipt beats a late wake. No other
            // operation may consume readiness after its deadline.
            if let Some(value) = ready() {
                return Some(ParkOutcome::Ready(value));
            }
            return (now() >= deadline).then_some(ParkOutcome::Deadline);
        }
        if now() >= deadline {
            return Some(ParkOutcome::Deadline);
        }
        ready().map(ParkOutcome::Ready)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{mpsc, Arc};

    #[test]
    fn deadline_equality_and_revocation_beat_ready_bytes() {
        let park = Park::new();
        let deadline = Instant::now();
        assert_eq!(
            park.try_ready(
                deadline,
                || deadline,
                || false,
                || false,
                || false,
                || Some(12)
            ),
            Some(ParkOutcome::Deadline)
        );
        assert_eq!(
            park.try_ready(
                deadline,
                || deadline,
                || false,
                || false,
                || true,
                || Some(12)
            ),
            Some(ParkOutcome::Revoked)
        );
    }

    #[test]
    fn predeadline_receipt_survives_late_slot_wake() {
        let park = Park::new();
        let deadline = Instant::now();
        let result = park.wait(
            deadline,
            || deadline + Duration::from_secs(3),
            || false,
            || false,
            || false,
            || Some(91),
            true,
            None,
        );
        assert_eq!(result, ParkOutcome::Ready(91));
    }

    #[test]
    fn space_publish_before_or_after_marker_never_loses_progress() {
        let park = Arc::new(Park::new());
        let binding = WriterBinding {
            connection: 3,
            binding: 7,
        };
        let space = Arc::new(AtomicBool::new(false));
        // Drain before arm: K2 recheck sees the published capacity.
        space.store(true, Ordering::Release);
        park.publish_freed_space(binding);
        assert_eq!(
            park.wait(
                Instant::now() + Duration::from_secs(1),
                Instant::now,
                || false,
                || false,
                || false,
                || space.load(Ordering::Acquire).then_some(()),
                false,
                Some(binding)
            ),
            ParkOutcome::Ready(())
        );

        space.store(false, Ordering::Release);
        let (armed, receiver) = mpsc::channel();
        let worker = Arc::clone(&park);
        let observed = Arc::clone(&space);
        let thread = std::thread::spawn(move || {
            worker.wait(
                Instant::now() + Duration::from_secs(1),
                Instant::now,
                || false,
                || false,
                || false,
                || {
                    armed.send(()).ok();
                    observed.load(Ordering::Acquire).then_some(())
                },
                false,
                Some(binding),
            )
        });
        receiver.recv().unwrap();
        space.store(true, Ordering::Release);
        park.publish_freed_space(binding);
        assert_eq!(thread.join().unwrap(), ParkOutcome::Ready(()));
    }
}
