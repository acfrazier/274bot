//! Shared player-trade screen lifecycle. Both native and Load callers retain
//! their own offer policy; this core owns the bounded offer/confirm/close waits.
use std::time::{Duration, Instant};

pub const TRADE_OFFER_WAIT_MS: u64 = 5_000;
pub const TRADE_CONFIRM_WAIT_MS: u64 = 8_000;
pub const TRADE_CLOSE_DEBOUNCE_MS: u64 = 600;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenKind {
    Offer,
    Confirm,
    Closed,
}

impl ScreenKind {
    pub fn observed(offer: bool, confirm: bool) -> Self {
        if offer {
            Self::Offer
        } else if confirm {
            Self::Confirm
        } else {
            Self::Closed
        }
    }

    pub fn active(self) -> bool {
        self != Self::Closed
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Offer => "offer",
            Self::Confirm => "confirm",
            Self::Closed => "closed",
        }
    }
}

/// An inactive post is not yet a settled trade. A subsequent active post
/// cancels the debounce, including a confirm screen appearing after offer.
#[derive(Debug, Default)]
pub struct CloseDriver {
    inactive_since: Option<Instant>,
}

impl CloseDriver {
    pub fn poll(&mut self, screen: ScreenKind, now: Instant) -> bool {
        if screen.active() {
            self.inactive_since = None;
            return false;
        }
        let since = *self.inactive_since.get_or_insert(now);
        now.saturating_duration_since(since)
            >= Duration::from_millis(TRADE_CLOSE_DEBOUNCE_MS)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitEnd {
    Waiting,
    Confirm,
    Closed,
    TimedOut,
}

/// One shared driver for the screen following an accepted offer or
/// confirmation. A caller must still prove its own inventory postconditions.
#[derive(Debug)]
pub struct ScreenDriver {
    confirm_expected: bool,
    closes: CloseDriver,
}

impl ScreenDriver {
    pub fn after_offer() -> Self {
        Self {
            confirm_expected: true,
            closes: CloseDriver::default(),
        }
    }

    pub fn after_confirm() -> Self {
        Self {
            confirm_expected: false,
            closes: CloseDriver::default(),
        }
    }

    pub fn poll(&mut self, screen: ScreenKind, now: Instant, expired: bool) -> WaitEnd {
        if self.confirm_expected && screen == ScreenKind::Confirm {
            return WaitEnd::Confirm;
        }
        if self.closes.poll(screen, now) {
            return WaitEnd::Closed;
        }
        if expired {
            WaitEnd::TimedOut
        } else {
            WaitEnd::Waiting
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delayed_confirmation_resets_close_debounce() {
        let now = Instant::now();
        let mut driver = ScreenDriver::after_offer();
        assert_eq!(driver.poll(ScreenKind::Closed, now, false), WaitEnd::Waiting);
        assert_eq!(
            driver.poll(ScreenKind::Confirm, now + Duration::from_millis(300), false),
            WaitEnd::Confirm
        );
        let mut driver = ScreenDriver::after_confirm();
        assert_eq!(driver.poll(ScreenKind::Closed, now, false), WaitEnd::Waiting);
        assert_eq!(
            driver.poll(ScreenKind::Confirm, now + Duration::from_millis(400), false),
            WaitEnd::Waiting
        );
        assert_eq!(
            driver.poll(ScreenKind::Closed, now + Duration::from_millis(700), false),
            WaitEnd::Waiting
        );
        assert_eq!(
            driver.poll(ScreenKind::Closed, now + Duration::from_millis(1300), false),
            WaitEnd::Closed
        );
    }

    #[test]
    fn expiry_is_not_a_transfer_and_offer_precedes_confirm() {
        let now = Instant::now();
        let mut driver = ScreenDriver::after_confirm();
        assert_eq!(driver.poll(ScreenKind::Offer, now, true), WaitEnd::TimedOut);
        assert_eq!(ScreenKind::observed(true, true), ScreenKind::Offer);
    }
}
