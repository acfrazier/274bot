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
        clock.set_freeze(true, false);
        let frozen_at = clock.frozen_at.expect("captured on freeze");
        assert_eq!(clock.now(), frozen_at);
        assert_eq!(clock.now(), frozen_at);
    }

    #[test]
    fn hold_is_equivalent_to_pause_for_frozen() {
        let mut pause = InstantTaskClock::new();
        let mut hold = InstantTaskClock::new();
        pause.set_freeze(true, false);
        hold.set_freeze(false, true);
        assert!(pause.frozen());
        assert!(hold.frozen());
        let pause_at = pause.frozen_at.expect("pause captured");
        let hold_at = hold.frozen_at.expect("hold captured");
        assert_eq!(pause.now(), pause_at);
        assert_eq!(hold.now(), hold_at);
    }

    #[test]
    fn thaw_with_none_deadline_is_a_noop() {
        let mut clock = InstantTaskClock::new();
        clock.set_freeze(true, false);
        assert!(clock.frozen_at.is_some());
        clock.set_freeze(false, false);
        assert!(!clock.frozen());
        assert!(clock.deadline.is_none());
        assert!(clock.frozen_at.is_none());
        assert!(!clock.bound_reached());
    }

    #[test]
    fn thaw_reclaims_deadline_between_observed_clock_bounds() {
        let mut clock = InstantTaskClock::new();
        let window = Duration::from_millis(500);
        let before = Instant::now();
        clock.deadline = Some(before + window);
        let original = clock.deadline.expect("seeded deadline");

        clock.set_freeze(true, false);
        let after_freeze = Instant::now();
        let frozen_at = clock.frozen_at.expect("frozen");
        assert!(frozen_at >= before);
        assert!(frozen_at <= after_freeze);

        let before_thaw = Instant::now();
        clock.set_freeze(false, false);
        let after_thaw = Instant::now();

        let gap_min = before_thaw.saturating_duration_since(frozen_at);
        let gap_max = after_thaw.saturating_duration_since(frozen_at);
        let reclaimed = clock.deadline.expect("reclaimed deadline");
        assert!(reclaimed >= original + gap_min);
        assert!(reclaimed <= original + gap_max);
        assert!(clock.frozen_at.is_none());
        assert!(!clock.frozen());
    }

    #[test]
    fn overlapping_pause_and_hold_reclaim_once_from_first_freeze() {
        let mut clock = InstantTaskClock::new();
        let before = Instant::now();
        clock.deadline = Some(before + Duration::from_millis(100));
        let original_deadline = clock.deadline;

        clock.set_freeze(true, false);
        let captured = clock.frozen_at;

        clock.set_freeze(true, true);
        assert_eq!(clock.frozen_at, captured);
        assert_eq!(clock.deadline, original_deadline);

        clock.set_freeze(false, true);
        assert!(clock.frozen());
        assert_eq!(clock.frozen_at, captured);
        assert_eq!(clock.deadline, original_deadline);

        let frozen_at = captured.expect("still frozen from first edge");
        let before_final_thaw = Instant::now();
        clock.set_freeze(false, false);
        let after_final_thaw = Instant::now();

        assert!(!clock.frozen());
        assert!(clock.frozen_at.is_none());
        let gap_min = before_final_thaw.saturating_duration_since(frozen_at);
        let gap_max = after_final_thaw.saturating_duration_since(frozen_at);
        let expected_base = original_deadline.expect("original");
        let reclaimed = clock.deadline.expect("reclaimed once");
        assert!(reclaimed >= expected_base + gap_min);
        assert!(reclaimed <= expected_base + gap_max);
    }

    #[test]
    fn arm_while_frozen_uses_frozen_at_not_wall_clock() {
        let mut clock = InstantTaskClock::new();
        clock.set_freeze(true, false);
        let frozen_at = clock.frozen_at.expect("captured");
        clock.arm(250);
        assert_eq!(clock.deadline, Some(frozen_at + Duration::from_millis(250)));
        assert!(!clock.bound_reached());
        clock.deadline = Some(frozen_at);
        assert!(clock.bound_reached());
    }

    #[test]
    fn bound_reached_is_false_when_deadline_is_none() {
        let mut clock = InstantTaskClock::new();
        clock.set_freeze(true, false);
        assert!(!clock.bound_reached());
        clock.set_freeze(false, false);
        assert!(!clock.bound_reached());
    }

    #[test]
    fn bound_reached_stays_true_when_already_due_while_frozen() {
        let mut clock = InstantTaskClock::new();
        clock.deadline = Some(Instant::now() - Duration::from_millis(1));
        clock.set_freeze(true, false);
        assert!(clock.bound_reached());
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
