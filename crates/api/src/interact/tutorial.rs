//! Post-relog tutorial reseed for fresh live accounts.
//!
//! Closing the character-design kit queues `tutorial=1`
//! (`content/scripts/tutorial/scripts/tutorial.rs2:133-141`), and at
//! `tutorial <= 400` the content suppresses weapon combat-tab updates
//! (`appearance.rs2:116-118`). A fixture that proves `tutorial 1000` before
//! the initial relog can therefore reach the mainland with `tutorial 1`
//! again, and any later weapon wield fails for a fixture reason.
//!
//! The shared fix is: after the initial relog (so the kit-close queue has
//! already run), send `setvar tutorial 1000` + `getvar tutorial` in the new
//! session and require a **fresh same-session** `get tutorial: 1000` reply
//! before the cell continues. The wait has a fixed wall-clock deadline and
//! a clear failure message.

use std::time::{Duration, Instant};

use crate::interact::{cheat, Driver};
use crate::snapshot::GameSnapshot;

/// Completed-tutorial value every fresh live account reseeds to.
pub const TUTORIAL_COMPLETE: i32 = 1000;
/// Cheat that assigns the completed value.
pub const TUTORIAL_SETVAR: &str = "setvar tutorial 1000";
/// Cheat whose reply proves the assigned value.
pub const TUTORIAL_GETVAR: &str = "getvar tutorial";
/// Chat needle a `Proof::Chat` waits for when it must see the same proof.
pub const TUTORIAL_CHAT_NEEDLE: &str = "get tutorial: 1000";
/// Fixed wall-clock bound for the post-relog confirmation.
///
/// Pre-relog proofs race the kit-close queue; only a fresh reply in the
/// final session proves the durable value. Sixty seconds covers cheat
/// round-trips on the shared engine without letting a wedged fixture hang
/// the cell's own (longer) preparation deadline.
pub const TUTORIAL_POST_RELOG_DEADLINE: Duration = Duration::from_secs(60);
/// Log line cells emit (via [`confirmation_log`]) once the fresh reply lands,
/// so live logs prove the helper ran.
pub const TUTORIAL_CONFIRM_LOG: &str =
    "post-relog tutorial confirmed: get tutorial: 1000 (fresh same-session getvar after relog)";

/// Parse an engine `::getvar` reply line, case-insensitively.
///
/// Accepts `get tutorial: 1000` with surrounding whitespace and any ASCII
/// case (`GET TUTORIAL: 1000`). Returns the value only when the name is
/// `tutorial`; any other varp or malformed line returns `None`.
pub fn parse_tutorial_reply(text: &str) -> Option<i32> {
    let rest = text.trim().strip_prefix("get ").or_else(|| {
        // Case-insensitive `get ` prefix without allocating on the hot path
        // more than once: only lowercased when the fast check misses.
        let lower = text.trim().to_ascii_lowercase();
        lower.strip_prefix("get ").map(|_| &text.trim()[4..])
    })?;
    // `rest` keeps the original case for the value; split name/value on the
    // last colon so `get tutorial : 1000` still parses.
    let (name, value) = rest.rsplit_once(':')?;
    if !name.trim().eq_ignore_ascii_case("tutorial") {
        return None;
    }
    value.trim().parse().ok()
}

/// Newest chat `sequence` in `snapshot`, or `0` when no chat has posted.
///
/// Capture this immediately after the relog session is ready (ingame,
/// scene 2, side-tab bound) and **before** sending the reseed: only a reply
/// with a strictly larger sequence proves the new session's value. The
/// client clears `chat_text` on login but keeps bumping `chat_seq`, so a
/// pre-relog `1000` line can linger in a manually retained snapshot with an
/// old sequence and must not satisfy the check.
pub fn chat_baseline(snapshot: &GameSnapshot) -> i32 {
    snapshot
        .chat_lines()
        .iter()
        .map(|line| line.sequence)
        .max()
        .unwrap_or(0)
}

/// Whether `snapshot` holds a fresh post-relog `tutorial 1000` proof.
///
/// Fresh means either:
/// - a chat line with `sequence > baseline` parsing to 1000, or
/// - a chat-modal text parsing to 1000 (the modal is cleared on login, so
///   any present proof arrived after the relog).
pub fn tutorial_confirmed(snapshot: &GameSnapshot, baseline: i32) -> bool {
    if snapshot.chat_lines().iter().any(|line| {
        line.sequence > baseline && parse_tutorial_reply(&line.text) == Some(TUTORIAL_COMPLETE)
    }) {
        return true;
    }
    snapshot
        .chat_modal_texts()
        .iter()
        .any(|text| parse_tutorial_reply(text) == Some(TUTORIAL_COMPLETE))
}

/// The line a cell logs once [`PostRelogTutorial::check`] first returns
/// `Ok(true)`. Shared so every live log proves the same helper ran.
pub fn confirmation_log() -> &'static str {
    TUTORIAL_CONFIRM_LOG
}

/// Post-relog reseed state: baseline + fixed deadline.
///
/// Typical use in a per-frame fixture, after the relog session is ready:
///
/// ```text
/// // once, when WaitRelog first holds:
/// let mut reseed = PostRelogTutorial::new(chat_baseline(&snapshot));
/// reseed.send_reseed(client); // setvar tutorial 1000 + getvar tutorial
/// // every frame until Seed:
/// match reseed.check(&snapshot) {
///     Ok(true) => { println!("{}", confirmation_log()); /* continue */ }
///     Ok(false) => {} // keep waiting
///     Err(error) => { /* fail the prerequisite with error */ }
/// }
/// ```
#[derive(Debug, Clone)]
pub struct PostRelogTutorial {
    baseline: i32,
    started: Instant,
    deadline: Duration,
}

impl PostRelogTutorial {
    /// New reseed wait: `baseline` from [`chat_baseline`] captured after the
    /// relog session is ready and before [`PostRelogTutorial::send_reseed`].
    pub fn new(baseline: i32) -> Self {
        Self {
            baseline,
            started: Instant::now(),
            deadline: TUTORIAL_POST_RELOG_DEADLINE,
        }
    }

    /// Override the deadline (tests and unusually slow engines).
    pub fn with_deadline(mut self, deadline: Duration) -> Self {
        self.deadline = deadline;
        self
    }

    /// Constructor with an explicit start time, for deadline tests without
    /// sleeping.
    pub fn with_started(baseline: i32, started: Instant, deadline: Duration) -> Self {
        Self {
            baseline,
            started,
            deadline,
        }
    }

    /// The chat baseline this wait requires a strictly newer reply than.
    pub fn baseline(&self) -> i32 {
        self.baseline
    }

    /// The fixed deadline this wait fails against.
    pub fn deadline(&self) -> Duration {
        self.deadline
    }

    /// Time since the wait started.
    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    /// Send the post-relog reseed: `setvar tutorial 1000` then
    /// `getvar tutorial` in the new session. Call once after the relog
    /// session is ready; the kit-close queue has already run there.
    pub fn send_reseed<D: Driver + ?Sized>(&self, client: &mut D) {
        let _ = cheat(client, TUTORIAL_SETVAR);
        let _ = cheat(client, TUTORIAL_GETVAR);
    }

    /// Whether `snapshot` already holds the fresh proof.
    pub fn is_confirmed(&self, snapshot: &GameSnapshot) -> bool {
        tutorial_confirmed(snapshot, self.baseline)
    }

    /// Poll the wait: `Ok(true)` when the fresh proof has landed,
    /// `Ok(false)` while still inside the deadline, `Err` with a clear
    /// fixture message once the fixed deadline passes without proof.
    pub fn check(&self, snapshot: &GameSnapshot) -> Result<bool, String> {
        if self.is_confirmed(snapshot) {
            return Ok(true);
        }
        if self.elapsed() >= self.deadline {
            return Err(self.failure_message());
        }
        Ok(false)
    }

    /// Clear failure message for the deadline path. Names the expected
    /// fresh reply, the bound, and the kit-close rollback it guards.
    pub fn failure_message(&self) -> String {
        format!(
            "fresh-account tutorial confirm failed: no fresh 'get tutorial: {}' within {:?} after the initial relog \
             (closing the character-design kit queues tutorial=1; weapon combat tabs stay suppressed until tutorial>400)",
            TUTORIAL_COMPLETE,
            self.deadline,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::ChatLineView;

    fn line(text: &str, sequence: i32) -> ChatLineView {
        ChatLineView {
            type_: 0,
            username: None,
            text: text.to_owned(),
            sequence,
        }
    }

    #[test]
    fn parses_tutorial_replies_case_insensitively() {
        assert_eq!(parse_tutorial_reply("get tutorial: 1000"), Some(1000));
        assert_eq!(parse_tutorial_reply("GET TUTORIAL: 1000"), Some(1000));
        assert_eq!(parse_tutorial_reply("  get  tutorial : 1  "), Some(1));
        assert_eq!(parse_tutorial_reply("get tutorial:0"), Some(0));
        assert_eq!(parse_tutorial_reply("hello"), None);
        assert_eq!(parse_tutorial_reply("get coins: 25"), None);
        assert_eq!(parse_tutorial_reply("get tutorial: lots"), None);
    }

    #[test]
    fn confirmed_needs_a_reply_newer_than_the_relog_baseline() {
        let mut snapshot = GameSnapshot::new();
        // Stale pre-relog proof at the baseline sequence must not satisfy.
        snapshot.seed_chat_lines(vec![line("get tutorial: 1000", 7)]);
        assert!(!tutorial_confirmed(&snapshot, 7));
        assert!(!tutorial_confirmed(&snapshot, 8));
        // A strictly newer 1000 reply in the same session satisfies.
        snapshot.seed_chat_lines(vec![
            line("get tutorial: 1000", 7),
            line("get tutorial: 1000", 8),
        ]);
        assert!(tutorial_confirmed(&snapshot, 7));
        // A newer line with the rolled-back value does not.
        snapshot.seed_chat_lines(vec![line("get tutorial: 1", 9)]);
        assert!(!tutorial_confirmed(&snapshot, 7));
        // Chat-modal proof (cleared on login) satisfies without a sequence.
        snapshot.seed_chat_lines(vec![]);
        snapshot.seed_chat_modal(1, vec!["get tutorial: 1000".to_owned()]);
        assert!(tutorial_confirmed(&snapshot, 99));
    }

    #[test]
    fn baseline_is_the_newest_chat_sequence() {
        let mut snapshot = GameSnapshot::new();
        assert_eq!(chat_baseline(&snapshot), 0);
        snapshot.seed_chat_lines(vec![line("a", 3), line("b", 9), line("c", 5)]);
        assert_eq!(chat_baseline(&snapshot), 9);
    }

    #[test]
    fn deadline_failure_names_the_fresh_reply_and_the_bound() {
        let snapshot = GameSnapshot::new();
        let started = Instant::now() - TUTORIAL_POST_RELOG_DEADLINE - Duration::from_secs(1);
        let wait = PostRelogTutorial::with_started(0, started, TUTORIAL_POST_RELOG_DEADLINE);
        assert!(!wait.is_confirmed(&snapshot));
        let error = wait
            .check(&snapshot)
            .expect_err("expired wait without proof must fail");
        assert!(
            error.contains("get tutorial: 1000"),
            "failure must name the expected reply: {error}"
        );
        assert!(
            error.contains("relog"),
            "failure must point at the post-relog ordering: {error}"
        );
        assert!(
            error.contains("60s") || error.contains("60"),
            "failure must state the fixed bound: {error}"
        );
    }

    #[test]
    fn waiting_inside_the_deadline_is_not_a_failure() {
        let snapshot = GameSnapshot::new();
        let wait = PostRelogTutorial::new(0);
        assert_eq!(wait.check(&snapshot), Ok(false));
    }

    #[test]
    fn fresh_reply_before_the_deadline_succeeds() {
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_chat_lines(vec![line("get tutorial: 1000", 11)]);
        let wait = PostRelogTutorial::new(10);
        assert_eq!(wait.check(&snapshot), Ok(true));
    }
}
