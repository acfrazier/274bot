//! One bot's current login notice: the newest failed login, shown apart
//! from the bot's live state. The host clears its error when the next
//! attempt starts, and the slot can then show Connecting, a queue wait or
//! logged out; none of those resolves the failure, so the notice outlives
//! them. It follows the front ends' one message rule
//! ([`crate::error_expiry`]): a successful login resolves it, a newer
//! failed login replaces it (also with the same text: each failure is its
//! own occurrence), or it clears after [`crate::ERROR_MESSAGE_TIMEOUT`].
//!
//! This is not the row's last error ([`crate::FleetRow::error`]): that is
//! history, kept until the bot is ready, goes offline or starts a new
//! worker lifetime.

use std::fmt::Write as _;
use std::time::Instant;

use crate::error_expiry::ErrorExpiry;

/// What resolves a login notice: the bot logged in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LoginNoticeNeed {
    LoggedIn,
}

/// One failed login as the host published it: the worker lifetime, when
/// its Error phase began, and its text (already capped).
#[derive(Debug, Clone, Copy)]
pub(crate) struct LoginFailure<'a> {
    pub lifetime: u64,
    pub at: Instant,
    pub text: &'a str,
}

/// The clock of one bot's login notice. The notice text itself lives on
/// the bot's projected row ([`crate::FleetRow::login_notice`]).
#[derive(Debug, Default)]
pub(crate) struct LoginNotice {
    clock: ErrorExpiry<LoginNoticeNeed>,
    /// The failure the notice shows: lifetime and Error-phase start.
    shown: Option<(u64, Instant)>,
    /// Failures seen so far: a new one restarts the clock even when its
    /// text equals the last.
    failures: u64,
}

impl LoginNotice {
    /// Bring `notice` up to date at `now`. `failure` is the host's current
    /// failed login, if any; `logged_in` says the bot has a logged-in
    /// session. Returns whether `notice` changed.
    pub(crate) fn update(
        &mut self,
        notice: &mut Option<String>,
        failure: Option<LoginFailure<'_>>,
        logged_in: bool,
        now: Instant,
    ) -> bool {
        let mut changed = false;
        if let Some(failure) = failure.filter(|_| !logged_in) {
            let identity = (failure.lifetime, failure.at);
            if self.shown != Some(identity) {
                self.shown = Some(identity);
                self.failures += 1;
                let line = notice.get_or_insert_with(String::new);
                line.clear();
                let _ = write!(line, "login {}", failure.text);
                self.clock.waits_for(LoginNoticeNeed::LoggedIn, line);
                changed = true;
            }
        }
        changed |= self.clock.resolve(notice, |need| match need {
            LoginNoticeNeed::LoggedIn => logged_in,
        });
        changed |= self.clock.observe(notice, self.failures, now);
        changed
    }
}

/// [`LoginNotice::update`] for a bot's boxed slots: `state` (its clock,
/// created by its first failed login) and `text` (the row's boxed notice).
/// A bot that never failed a login and shows no notice stays as it was: two
/// empty pointers, no allocation. The text moves out of its box and back
/// without copying.
#[expect(
    clippy::box_collection,
    reason = "the row's one-pointer slot; a bare `String` is 24 bytes on every row"
)]
pub(crate) fn update_slot(
    state: &mut Option<Box<LoginNotice>>,
    text: &mut Option<Box<String>>,
    failure: Option<LoginFailure<'_>>,
    logged_in: bool,
    now: Instant,
) -> bool {
    let failing = failure.is_some() && !logged_in;
    if state.is_none() && text.is_none() && !failing {
        return false;
    }
    let state = state.get_or_insert_with(Box::default);
    let mut line = text.as_mut().map(|held| std::mem::take(&mut **held));
    let changed = state.update(&mut line, failure, logged_in, now);
    match (line, text.as_mut()) {
        (Some(line), Some(held)) => **held = line,
        (Some(line), None) => *text = Some(Box::new(line)),
        (None, _) => *text = None,
    }
    changed
}

#[cfg(test)]
#[path = "login_notice_tests.rs"]
mod tests;
