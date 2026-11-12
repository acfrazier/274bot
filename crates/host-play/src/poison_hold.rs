use api::condition::{PoisonWindow, WindowReset};

/// The route tile where an assessed poison hold must stop the follow.
#[derive(Debug, Default)]
pub(crate) struct PoisonHold {
    pub(crate) until_index: u16,
    pub(crate) wait: Option<HoldWait>,
    pub(crate) stand_sent: bool,
}

/// The observed-tick wait, with one bounded recovery for a missed observation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct HoldWait {
    pub(crate) started: u16,
    pub(crate) deadline: u16,
    recovered_gap: bool,
}

impl HoldWait {
    pub(crate) fn start(started: u16, remaining: u8) -> Self {
        Self {
            started,
            deadline: started.wrapping_add(u16::from(remaining)),
            recovered_gap: false,
        }
    }

    pub(crate) fn recover_frame_gap(&mut self, now: u16, window: PoisonWindow) {
        if self.recovered_gap
            || !window.accepted
            || window.last_reset != WindowReset::FrameGap
            || self.elapsed(now) > self.deadline.wrapping_sub(self.started) + 1
        {
            return;
        }
        let deadline = now.wrapping_add(u16::from(window.remaining));
        let extension = deadline.wrapping_sub(self.deadline);
        // poison.rs2:29,72-75 needs 31 clean edges. Cover one missed deadline
        // update and one replacement window, never a second extension.
        if extension != 0 && extension <= u16::from(crate::slot_condition::CLEAN_TICKS) + 1 {
            self.deadline = deadline;
            self.recovered_gap = true;
        }
    }
    /// Short-window comparison that remains valid across `u16` wraparound.
    pub(crate) fn ended(self, now: u16) -> bool {
        now.wrapping_sub(self.deadline) < 0x8000
    }

    pub(crate) fn elapsed(self, now: u16) -> u16 {
        now.wrapping_sub(self.started)
    }

    pub(crate) fn waited(self, now: u16) -> u8 {
        self.elapsed(now).min(u16::from(u8::MAX)) as u8
    }

    pub(crate) fn remaining(self) -> u8 {
        self.deadline
            .wrapping_sub(self.started)
            .min(u16::from(u8::MAX)) as u8
    }
}

impl PoisonHold {
    /// Start the hold window once. A second publication cannot restart it.
    pub(crate) fn start_wait(&mut self, now: u16, remaining: u8) -> Option<HoldWait> {
        if self.wait.is_some() {
            return None;
        }
        let wait = HoldWait::start(now, remaining);
        self.wait = Some(wait);
        Some(wait)
    }
}

#[cfg(test)]
mod tests {
    use super::{HoldWait, PoisonHold};
    use crate::slot_condition::CLEAN_TICKS;

    #[test]
    fn fallback_window_is_31_ticks() {
        let mut hold = PoisonHold {
            until_index: 7,
            ..PoisonHold::default()
        };
        let wait = hold.start_wait(100, CLEAN_TICKS).unwrap();
        assert_eq!(wait.deadline.wrapping_sub(wait.started), 31);
        assert!(!wait.ended(130));
        assert!(wait.ended(131));
        assert_eq!(wait.waited(131), 31);
    }

    #[test]
    fn partial_window_uses_current_remainder_once() {
        let mut hold = PoisonHold::default();
        let first = hold.start_wait(10, 18).unwrap();
        assert_eq!(first.deadline, 28);
        assert_eq!(hold.start_wait(15, 7), None);
        assert_eq!(hold.wait, Some(first));
        assert!(!first.ended(27));
        assert!(first.ended(28));
    }

    #[test]
    fn deadline_comparison_wraps_across_u16_boundary() {
        let wait = HoldWait::start(65_530, 18);
        assert_eq!(wait.deadline, 12);
        assert!(!wait.ended(65_535));
        assert!(!wait.ended(11));
        assert!(wait.ended(12));
        assert_eq!(wait.waited(12), 18);
    }

    #[test]
    fn frame_gap_recovery_is_bounded_and_wraps() {
        use api::condition::{PoisonWindow, WindowReset};

        let gap = PoisonWindow {
            accepted: true,
            remaining: CLEAN_TICKS,
            last_reset: WindowReset::FrameGap,
        };
        for started in [0u16, 65_530] {
            let mut wait = HoldWait::start(started, CLEAN_TICKS);
            wait.recover_frame_gap(started.wrapping_add(32), gap);
            assert_eq!(wait.deadline, started.wrapping_add(63));
            wait.recover_frame_gap(started.wrapping_add(34), gap);
            assert_eq!(wait.deadline, started.wrapping_add(63));
            assert!(!wait.ended(started.wrapping_add(62)));
            assert!(wait.ended(started.wrapping_add(63)));
        }
    }

    #[test]
    fn stale_or_unaccepted_window_cannot_extend_the_wait() {
        use api::condition::{PoisonWindow, WindowReset};

        let mut wait = HoldWait::start(0, CLEAN_TICKS);
        wait.recover_frame_gap(
            3,
            PoisonWindow {
                accepted: false,
                remaining: CLEAN_TICKS,
                last_reset: WindowReset::FrameGap,
            },
        );
        assert_eq!(wait.deadline, 31);
        wait.recover_frame_gap(
            33,
            PoisonWindow {
                accepted: true,
                remaining: CLEAN_TICKS,
                last_reset: WindowReset::FrameGap,
            },
        );
        assert_eq!(
            wait.deadline, 31,
            "a late pump cannot restart an expired wait"
        );
    }

    #[test]
    fn hold_recovery_keeps_inline_storage_and_allocates_nothing() {
        use api::condition::{PoisonWindow, WindowReset};

        assert_eq!(std::mem::size_of::<HoldWait>(), 6);
        assert_eq!(std::mem::size_of::<Option<HoldWait>>(), 6);
        assert_eq!(std::mem::size_of::<PoisonHold>(), 10);
        let mut hold = PoisonHold::default();
        let measured = allocation_counter::measure(|| {
            hold.start_wait(0, CLEAN_TICKS);
            for now in 3..=1003 {
                hold.wait.as_mut().unwrap().recover_frame_gap(
                    now,
                    PoisonWindow {
                        accepted: true,
                        remaining: CLEAN_TICKS,
                        last_reset: WindowReset::FrameGap,
                    },
                );
            }
        });
        assert_eq!(measured.bytes_total, 0, "{measured:?}");
    }
}

#[cfg(test)]
#[path = "poison_hold_integration_tests.rs"]
mod integration_tests;
