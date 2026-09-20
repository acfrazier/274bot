//! Shared Instant pause/hold deadline clock for load-gated task runtimes.
//!
//! One instance per task. Freeze captures `Instant::now`; thaw adds
//! `saturating_duration_since` to `deadline` only. Session `watchdog` is
//! isolate recovery and is not this type. Abort/reset must clear `deadline`
//! only — pause/hold/`frozen_at` stay on the instance.

use std::time::{Duration, Instant};

pub(crate) struct InstantTaskClock {
    pub(crate) paused: bool,
    pub(crate) held: bool,
    pub(crate) frozen_at: Option<Instant>,
    pub(crate) deadline: Option<Instant>,
}

impl InstantTaskClock {
    pub(crate) const fn new() -> Self {
        Self {
            paused: false,
            held: false,
            frozen_at: None,
            deadline: None,
        }
    }

    pub(crate) fn frozen(&self) -> bool {
        self.paused || self.held
    }

    pub(crate) fn now(&self) -> Instant {
        self.frozen_at.unwrap_or_else(Instant::now)
    }

    pub(crate) fn set_freeze(&mut self, paused: bool, held: bool) {
        let was_frozen = self.frozen();
        self.paused = paused;
        self.held = held;
        let frozen = self.frozen();
        if !was_frozen && frozen {
            self.frozen_at = Some(Instant::now());
        } else if was_frozen && !frozen {
            if let Some(at) = self.frozen_at.take() {
                if let Some(deadline) = self.deadline.as_mut() {
                    *deadline += Instant::now().saturating_duration_since(at);
                }
            }
        }
    }

    pub(crate) fn arm(&mut self, window: u64) {
        self.deadline = Some(self.now() + Duration::from_millis(window));
    }

    pub(crate) fn bound_reached(&self) -> bool {
        self.deadline.is_some_and(|deadline| self.now() >= deadline)
    }

    #[cfg(test)]
    fn set_freeze_at(&mut self, paused: bool, held: bool, now: Instant) {
        let was_frozen = self.frozen();
        self.paused = paused;
        self.held = held;
        let frozen = self.frozen();
        if !was_frozen && frozen {
            self.frozen_at = Some(now);
        } else if was_frozen && !frozen {
            if let Some(at) = self.frozen_at.take() {
                if let Some(deadline) = self.deadline.as_mut() {
                    *deadline += now.saturating_duration_since(at);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_clock_is_unfrozen_with_no_deadline() {
        let clock = InstantTaskClock::new();
        assert!(!clock.frozen());
        assert!(!clock.paused);
        assert!(!clock.held);
        assert!(clock.frozen_at.is_none());
        assert!(clock.deadline.is_none());
        assert!(!clock.bound_reached());
    }

    #[test]
    fn now_while_frozen_returns_frozen_at() {
        let mut clock = InstantTaskClock::new();
        let frozen_at = Instant::now();
        clock.set_freeze_at(true, false, frozen_at);
        assert_eq!(clock.now(), frozen_at);
        assert_eq!(clock.now(), clock.frozen_at.expect("captured"));
        assert_eq!(clock.now(), frozen_at);
    }

    #[test]
    fn hold_is_equivalent_to_pause_for_frozen() {
        let mut pause = InstantTaskClock::new();
        let mut hold = InstantTaskClock::new();
        let at = Instant::now();
        pause.set_freeze_at(true, false, at);
        hold.set_freeze_at(false, true, at);
        assert!(pause.frozen());
        assert!(hold.frozen());
        assert_eq!(pause.now(), hold.now());
    }

    #[test]
    fn thaw_with_none_deadline_is_a_noop() {
        let mut clock = InstantTaskClock::new();
        let t0 = Instant::now();
        let t1 = t0 + Duration::from_millis(250);
        clock.set_freeze_at(true, false, t0);
        clock.set_freeze_at(false, false, t1);
        assert!(!clock.frozen());
        assert!(clock.deadline.is_none());
        assert!(clock.frozen_at.is_none());
        assert!(!clock.bound_reached());
    }

    #[test]
    fn freeze_then_thaw_reclaims_deadline_by_exact_gap() {
        let mut clock = InstantTaskClock::new();
        let t0 = Instant::now();
        let window = Duration::from_millis(100);
        let frozen_for = Duration::from_millis(250);
        clock.deadline = Some(t0 + window);
        clock.set_freeze_at(true, false, t0);
        clock.set_freeze_at(false, false, t0 + frozen_for);
        assert_eq!(clock.deadline, Some(t0 + window + frozen_for));
        assert!(clock.frozen_at.is_none());
        assert!(!clock.frozen());
    }

    #[test]
    fn overlapping_pause_and_hold_reclaim_once_from_first_freeze() {
        let mut clock = InstantTaskClock::new();
        let t0 = Instant::now();
        let window = Duration::from_millis(100);
        clock.deadline = Some(t0 + window);
        clock.set_freeze_at(true, false, t0);
        let captured = clock.frozen_at;
        clock.set_freeze_at(true, true, t0 + Duration::from_millis(40));
        assert_eq!(clock.frozen_at, captured);
        clock.set_freeze_at(false, true, t0 + Duration::from_millis(80));
        assert!(clock.frozen());
        assert_eq!(clock.frozen_at, captured);
        assert_eq!(clock.deadline, Some(t0 + window));
        let thaw = t0 + Duration::from_millis(250);
        clock.set_freeze_at(false, false, thaw);
        assert!(!clock.frozen());
        assert_eq!(
            clock.deadline,
            Some(t0 + window + Duration::from_millis(250))
        );
        assert!(clock.frozen_at.is_none());
    }

    #[test]
    fn arm_while_frozen_uses_frozen_at_not_wall_clock() {
        let mut clock = InstantTaskClock::new();
        let frozen_at = Instant::now();
        clock.set_freeze_at(true, false, frozen_at);
        clock.arm(250);
        assert_eq!(clock.deadline, Some(frozen_at + Duration::from_millis(250)));
        assert!(!clock.bound_reached());
        clock.deadline = Some(frozen_at);
        assert!(clock.bound_reached());
    }

    #[test]
    fn bound_reached_is_false_when_deadline_is_none() {
        let mut clock = InstantTaskClock::new();
        clock.set_freeze_at(true, false, Instant::now());
        assert!(!clock.bound_reached());
        clock.set_freeze(false, false);
        assert!(!clock.bound_reached());
    }

    #[test]
    fn production_set_freeze_captures_now_for_later_reads() {
        let mut clock = InstantTaskClock::new();
        clock.set_freeze(true, false);
        let first = clock.now();
        let second = clock.now();
        assert_eq!(first, second);
        assert_eq!(first, clock.frozen_at.expect("captured"));
        clock.set_freeze(false, true);
        assert_eq!(clock.now(), first);
        clock.set_freeze(false, false);
        assert!(!clock.frozen());
        assert!(clock.frozen_at.is_none());
    }
}
