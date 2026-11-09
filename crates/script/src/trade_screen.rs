//! Shared player-trade screen lifecycle. Both native and Load callers retain
//! their own offer policy; this core owns the bounded offer/confirm/close waits.
//!
//! The waits are measured in a [`Moment`]. The native Quester counts observed
//! game ticks ([`Tick`], one per `PLAYER_INFO`); the Load compat adapter keeps
//! the frozen script's wall clock ([`Instant`]).
use std::time::{Duration, Instant};

/// Selected tradeside:inv component, shared by native and Load callers.
pub const OFFER_INV: i32 = 3322;

/// Load: nominal wait for the screen after an offer accept.
pub const TRADE_OFFER_WAIT_MS: u64 = 5_000;
/// Load: nominal wait for the screen after a confirm accept.
pub const TRADE_CONFIRM_WAIT_MS: u64 = 8_000;
/// Load: how long a closed screen must stay closed to count as settled.
pub const TRADE_CLOSE_DEBOUNCE_MS: u64 = 600;

// The two accept waits below, and the close grace in `quester/families/partner.rs`,
// are policy budgets for the partner bot's input, not content constants: the
// engine does not bound when the other account clicks. Each is at least the
// maximum observed on the real engine plus one margin, stated once for all
// three: margin = 4 ticks (2.4 s), two more exchanges of the two-tick length an
// exchange takes (the partner's click, then the server cycle that applies it).
// A value that already covers maximum + margin stays; a larger one only delays
// the failure of a trade that is stuck. Ticks on which either account is held
// are not counted (`EligibleClock` in `partner.rs`), so a hold is not a slow
// partner.
//
// Measurement (Engine L, `host.quester-partner-trade`, 12 paired trades, 24 sides,
// 1-minute host load 5 to 46 on 32 cores; the table and runs are
// `/Volumes/dev-scratch/274bot-evidence/TRADE-CONFIRM-SCREEN/r2/measure-table.md`):
// after our offer accept the confirm screen shows on the observed tick that is 1
// or 2 ticks later (maximum 2); after our final accept the screen is first seen
// closed 1 or 2 ticks later (maximum 2), and 3 once the one-tick debounce is
// counted (maximum 3).

/// Native: policy budget, in observed game ticks, for the partner's offer accept
/// to open the confirm screen after our own offer accept. Observed maximum 2,
/// plus the margin of 4, is 6; 9 covers it.
pub const TRADE_OFFER_WAIT_TICKS: u64 = 9;
/// Native: policy budget, in observed game ticks, for the partner's final accept
/// to close the screen, seen closed and debounced, after our own final accept.
/// Observed maximum 3, plus the margin of 4, is 7; 14 covers it.
pub const TRADE_CONFIRM_WAIT_TICKS: u64 = 14;
/// Native: observed game ticks a closed screen must stay closed. The server
/// writes a modal change in one tick's packets (`if_openmain(tradeconfirm)` and
/// `if_close` at `trade.rs2:147`, `:271`), so a screen that is still closed one
/// `PLAYER_INFO` later is closed.
pub const TRADE_CLOSE_DEBOUNCE_TICKS: u64 = 1;

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

/// One native predicate returned by `stableClosedPoll`.
///
/// An explicit clock keeps the frozen millisecond comparison. Production calls
/// use posted game ticks and require one full inactive tick.
#[derive(Debug, Clone, Copy)]
pub struct StableClosedPoll {
    inactive_since_ms: f64,
    inactive_since_tick: f64,
}

impl Default for StableClosedPoll {
    fn default() -> Self {
        Self {
            inactive_since_ms: -1.0,
            inactive_since_tick: -1.0,
        }
    }
}

impl StableClosedPoll {
    pub fn from_baselines(inactive_since_ms: f64, inactive_since_tick: f64) -> Self {
        Self {
            inactive_since_ms,
            inactive_since_tick,
        }
    }

    pub fn baselines(self) -> (f64, f64) {
        (self.inactive_since_ms, self.inactive_since_tick)
    }

    pub fn reset(&mut self) {
        self.inactive_since_ms = -1.0;
        self.inactive_since_tick = -1.0;
    }

    pub fn poll_at(&mut self, screen: ScreenKind, now_ms: f64, min_ms: f64) -> bool {
        if screen.active() {
            self.reset();
            return false;
        }
        if self.inactive_since_ms < 0.0 {
            self.inactive_since_ms = now_ms;
        }
        now_ms - self.inactive_since_ms >= min_ms
    }

    pub fn poll_tick(&mut self, screen: ScreenKind, tick: f64, min_ms: f64) -> bool {
        if screen.active() {
            self.reset();
            return false;
        }
        if self.inactive_since_tick < 0.0 {
            self.inactive_since_tick = tick;
        }
        tick - self.inactive_since_tick >= Self::observed_ticks_for_min_ms(min_ms)
    }

    fn observed_ticks_for_min_ms(min_ms: f64) -> f64 {
        (min_ms / 600.0).ceil().max(1.0)
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
    fn tick_clock_covers_non_default_minimum_ms() {
        let mut poll = StableClosedPoll::default();
        assert!(!poll.poll_tick(ScreenKind::Closed, 7.0, 1_201.0));
        assert!(!poll.poll_tick(ScreenKind::Closed, 9.0, 1_201.0));
        assert!(poll.poll_tick(ScreenKind::Closed, 10.0, 1_201.0));
    }

    #[test]
    fn expiry_is_not_a_transfer_and_offer_precedes_confirm() {
        let now = Instant::now();
        let mut driver = ScreenDriver::after_confirm();
        assert_eq!(driver.poll(ScreenKind::Offer, now, true), WaitEnd::TimedOut);
        assert_eq!(ScreenKind::observed(true, true), ScreenKind::Offer);
    }

    #[test]
    fn native_waits_are_the_measured_policy_budgets_in_observed_game_ticks() {
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
