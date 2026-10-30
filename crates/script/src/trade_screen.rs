//! Shared player-trade screen lifecycle. Both native and Load callers retain
//! their own offer policy; this core owns the bounded offer/confirm/close waits.
//!
//! The waits are measured in a [`Moment`]. The native Quester counts observed
//! game ticks ([`Tick`], one per `PLAYER_INFO`); the Load compat adapter keeps
//! the frozen script's wall clock ([`Instant`]).
use crate::bank::ops::observed_ticks_for_millis;
use std::time::{Duration, Instant};

/// Selected tradeside:inv component, shared by native and Load callers.
pub const OFFER_INV: i32 = 3322;

/// Load: nominal wait for the screen after an offer accept.
pub const TRADE_OFFER_WAIT_MS: u64 = 5_000;
/// Load: nominal wait for the screen after a confirm accept.
pub const TRADE_CONFIRM_WAIT_MS: u64 = 8_000;
/// Load: how long a closed screen must stay closed to count as settled.
pub const TRADE_CLOSE_DEBOUNCE_MS: u64 = 600;

/// Native: observed game ticks to wait for the confirm screen after our offer
/// accept. The wait ends on the partner account's accept click through the pair
/// barrier, so no content signal bounds it: the nominal 5 000 ms over the 600 ms
/// server tick is 8.33, rounded up to 9.
pub const TRADE_OFFER_WAIT_TICKS: u64 = observed_ticks_for_millis(TRADE_OFFER_WAIT_MS);
/// Native: observed game ticks to wait for the screen to close after our final
/// accept. It also ends on the partner's click: the nominal 8 000 ms over 600 ms
/// is 13.33, rounded up to 14.
pub const TRADE_CONFIRM_WAIT_TICKS: u64 = observed_ticks_for_millis(TRADE_CONFIRM_WAIT_MS);
/// Native: observed game ticks a closed screen must stay closed. The server
/// writes a modal change in one tick's packets (`if_openmain(tradeconfirm)` and
/// `if_close` at `trade.rs2:147`, `:271`), so a screen that is still closed one
/// `PLAYER_INFO` later is closed (the nominal 600 ms over 600 ms is 1).
pub const TRADE_CLOSE_DEBOUNCE_TICKS: u64 = observed_ticks_for_millis(TRADE_CLOSE_DEBOUNCE_MS);

/// A point on the clock a trade wait runs on.
pub trait Moment: Copy {
    /// Whether a screen closed since `since` has now stayed closed long enough.
    fn debounced_since(self, since: Self) -> bool;
}

impl Moment for Instant {
    fn debounced_since(self, since: Self) -> bool {
        self.saturating_duration_since(since) >= Duration::from_millis(TRADE_CLOSE_DEBOUNCE_MS)
    }
}

/// An observed game tick (`EvidenceStamp::tick`, one per `PLAYER_INFO`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tick(pub u64);

impl Moment for Tick {
    fn debounced_since(self, since: Self) -> bool {
        self.0.saturating_sub(since.0) >= TRADE_CLOSE_DEBOUNCE_TICKS
    }
}

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
#[derive(Debug)]
pub struct CloseDriver<M: Moment = Instant> {
    inactive_since: Option<M>,
}

impl<M: Moment> Default for CloseDriver<M> {
    fn default() -> Self {
        Self {
            inactive_since: None,
        }
    }
}

impl<M: Moment> CloseDriver<M> {
    pub fn poll(&mut self, screen: ScreenKind, now: M) -> bool {
        if screen.active() {
            self.inactive_since = None;
            return false;
        }
        let since = *self.inactive_since.get_or_insert(now);
        now.debounced_since(since)
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
pub struct ScreenDriver<M: Moment = Instant> {
    confirm_expected: bool,
    closes: CloseDriver<M>,
}

impl<M: Moment> ScreenDriver<M> {
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

    pub fn poll(&mut self, screen: ScreenKind, now: M, expired: bool) -> WaitEnd {
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
        assert_eq!(
            driver.poll(ScreenKind::Closed, now, false),
            WaitEnd::Waiting
        );
        assert_eq!(
            driver.poll(ScreenKind::Confirm, now + Duration::from_millis(300), false),
            WaitEnd::Confirm
        );
        let mut driver = ScreenDriver::after_confirm();
        assert_eq!(
            driver.poll(ScreenKind::Closed, now, false),
            WaitEnd::Waiting
        );
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

    #[test]
    fn native_waits_are_observed_game_ticks_rounded_up_from_the_nominal_duration() {
        assert_eq!(TRADE_OFFER_WAIT_TICKS, 9);
        assert_eq!(TRADE_CONFIRM_WAIT_TICKS, 14);
        assert_eq!(TRADE_CLOSE_DEBOUNCE_TICKS, 1);
    }

    #[test]
    fn tick_debounce_needs_a_later_observed_tick_and_a_confirm_screen_cancels_it() {
        let mut driver = ScreenDriver::<Tick>::after_confirm();
        assert_eq!(
            driver.poll(ScreenKind::Closed, Tick(10), false),
            WaitEnd::Waiting,
            "closed in the tick it was first seen is not yet settled"
        );
        assert_eq!(
            driver.poll(ScreenKind::Closed, Tick(10), false),
            WaitEnd::Waiting,
            "a second look in the same tick adds no time"
        );
        assert_eq!(
            driver.poll(ScreenKind::Confirm, Tick(11), false),
            WaitEnd::Waiting
        );
        assert_eq!(
            driver.poll(ScreenKind::Closed, Tick(12), false),
            WaitEnd::Waiting
        );
        assert_eq!(
            driver.poll(ScreenKind::Closed, Tick(13), false),
            WaitEnd::Closed
        );
        let mut offer = ScreenDriver::<Tick>::after_offer();
        assert_eq!(
            offer.poll(ScreenKind::Closed, Tick(20), false),
            WaitEnd::Waiting
        );
        assert_eq!(
            offer.poll(ScreenKind::Confirm, Tick(20), false),
            WaitEnd::Confirm
        );
        assert_eq!(
            offer.poll(ScreenKind::Offer, Tick(30), true),
            WaitEnd::TimedOut
        );
    }
}
