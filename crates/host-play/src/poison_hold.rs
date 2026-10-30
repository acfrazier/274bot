/// The route tile where an assessed poison hold must stop the follow.
#[derive(Debug, Default)]
pub(crate) struct PoisonHold {
    pub(crate) until_index: u16,
    pub(crate) wait: Option<HoldWait>,
    pub(crate) stand_sent: bool,
}

/// The single observed-tick window this route attempt may wait.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct HoldWait {
    pub(crate) started: u16,
    pub(crate) deadline: u16,
}

impl HoldWait {
    pub(crate) fn start(started: u16, remaining: u8) -> Self {
        Self {
            started,
            deadline: started.wrapping_add(u16::from(remaining)),
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
}

#[cfg(test)]
#[path = "poison_hold_integration_tests.rs"]
mod integration_tests;
