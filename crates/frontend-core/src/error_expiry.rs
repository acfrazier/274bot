//! How long a front end keeps its one error message on screen. The panel's
//! red line and the terminal app's banner are the same rule: a message
//! stays until an action that resolves it clears it, a newer message
//! replaces it, or [`ERROR_MESSAGE_TIMEOUT`] passes without either. This is
//! a UI timer over wall-clock time, not game time.

use std::time::{Duration, Instant};

/// How long an error message stays after it appears (or is replaced by a
/// newer one) when nothing clears it first.
pub const ERROR_MESSAGE_TIMEOUT: Duration = Duration::from_secs(15);

/// The expiry clock of one front end's error message. The front end keeps
/// its message in its own `Option<String>` and calls [`Self::observe`] once
/// per frame (or tick); this type only remembers which message it has seen
/// and since when.
#[derive(Debug, Default)]
pub struct ErrorExpiry {
    seen: Option<String>,
    since: Option<Instant>,
    refusals: u64,
}

impl ErrorExpiry {
    /// Look at `error` at `now`. A message that differs from the one seen
    /// before, or that appears with a new refusal behind it (`refusals`,
    /// [`crate::OperatorSession::refusals`]: the same text refused again),
    /// starts its timeout. A cleared message resets the clock. A message
    /// that has stood for [`ERROR_MESSAGE_TIMEOUT`] is cleared. Returns
    /// whether it cleared the message.
    pub fn observe(&mut self, error: &mut Option<String>, refusals: u64, now: Instant) -> bool {
        let refused_again = std::mem::replace(&mut self.refusals, refusals) != refusals;
        let Some(text) = error.as_deref() else {
            self.seen = None;
            self.since = None;
            return false;
        };
        if self.seen.as_deref() != Some(text) || refused_again {
            match &mut self.seen {
                Some(seen) => {
                    seen.clear();
                    seen.push_str(text);
                }
                None => self.seen = Some(text.to_owned()),
            }
            self.since = Some(now);
            return false;
        }
        if self
            .since
            .is_some_and(|since| now.saturating_duration_since(since) >= ERROR_MESSAGE_TIMEOUT)
        {
            *error = None;
            self.seen = None;
            self.since = None;
            return true;
        }
        false
    }

    /// When the message under watch expires, for a front end that sleeps
    /// between frames. `None` while no message is shown.
    pub fn expires_at(&self) -> Option<Instant> {
        self.since.map(|since| since + ERROR_MESSAGE_TIMEOUT)
    }
}

#[cfg(test)]
#[path = "error_expiry_tests.rs"]
mod tests;
