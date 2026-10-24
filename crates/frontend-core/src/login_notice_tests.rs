use std::time::Duration;

use super::*;
use crate::ERROR_MESSAGE_TIMEOUT as TIMEOUT;

const REFUSED: &str = "Error connecting to server. (connect: Connection refused)";

fn failure(at: Instant) -> Option<LoginFailure<'static>> {
    Some(LoginFailure {
        lifetime: 1,
        at,
        text: REFUSED,
    })
}

fn shown(notice: &Option<String>) -> Option<&str> {
    notice.as_deref()
}

/// The notice keeps the failure through the retry's other states (no
/// failure published, not logged in) and clears at the shared timeout, not
/// a frame before.
#[test]
fn a_notice_stands_through_state_changes_until_the_timeout() {
    let base = Instant::now();
    let mut clock = LoginNotice::default();
    let mut notice = None;
    assert!(clock.update(&mut notice, failure(base), false, base));
    let line = format!("login {REFUSED}");
    assert_eq!(shown(&notice), Some(line.as_str()));
    // Connecting, a queue wait, logged out: the host cleared its error.
    assert!(!clock.update(&mut notice, None, false, base + TIMEOUT / 2));
    assert!(!clock.update(
        &mut notice,
        None,
        false,
        base + TIMEOUT - Duration::from_millis(1)
    ));
    assert_eq!(
        shown(&notice),
        Some(line.as_str()),
        "just before the timeout"
    );
    assert!(clock.update(&mut notice, None, false, base + TIMEOUT));
    assert_eq!(notice, None, "gone at the timeout");
}

/// A newer failure with the same text is a new occurrence: it restarts the
/// clock. The same failure seen again (same Error-phase start) does not.
#[test]
fn a_newer_failure_with_the_same_text_restarts_the_clock() {
    let base = Instant::now();
    let mut clock = LoginNotice::default();
    let mut notice = None;
    clock.update(&mut notice, failure(base), false, base);
    // The same failure on a later poll: no renewal.
    assert!(!clock.update(
        &mut notice,
        failure(base),
        false,
        base + Duration::from_secs(10)
    ));
    // The retry fails again 10 s later with the same words.
    let again = base + Duration::from_secs(10);
    assert!(clock.update(&mut notice, failure(again), false, again));
    clock.update(&mut notice, None, false, base + TIMEOUT);
    assert!(notice.is_some(), "the second failure restarted the 15 s");
    clock.update(
        &mut notice,
        None,
        false,
        again + TIMEOUT - Duration::from_millis(1),
    );
    assert!(notice.is_some());
    assert!(clock.update(&mut notice, None, false, again + TIMEOUT));
    assert_eq!(notice, None);
}

/// A newer failure with other text replaces the shown one, and a later
/// failure after the notice expired shows again.
#[test]
fn a_newer_failure_replaces_and_a_failure_after_expiry_shows_again() {
    let base = Instant::now();
    let mut clock = LoginNotice::default();
    let mut notice = None;
    clock.update(&mut notice, failure(base), false, base);
    let later = base + Duration::from_secs(3);
    let refused = LoginFailure {
        lifetime: 1,
        at: later,
        text: "code 3: Invalid username or password.",
    };
    assert!(clock.update(&mut notice, Some(refused), false, later));
    assert_eq!(
        shown(&notice),
        Some("login code 3: Invalid username or password.")
    );
    assert!(clock.update(&mut notice, None, false, later + TIMEOUT));
    assert_eq!(notice, None);
    let next = later + TIMEOUT + Duration::from_secs(5);
    assert!(clock.update(&mut notice, failure(next), false, next));
    assert!(notice.is_some(), "a new failure after expiry shows again");
}

/// A successful login resolves the notice at once, and a failure the row
/// still shows while logged in never raises one.
#[test]
fn logging_in_resolves_the_notice() {
    let base = Instant::now();
    let mut clock = LoginNotice::default();
    let mut notice = None;
    clock.update(&mut notice, failure(base), false, base);
    assert!(clock.update(&mut notice, None, true, base + Duration::from_secs(2)));
    assert_eq!(notice, None, "the login resolved it before the timeout");
    let next = base + Duration::from_secs(4);
    assert!(!clock.update(&mut notice, failure(next), true, next));
    assert_eq!(notice, None);
}

/// A bot that never failed a login keeps both slots empty, so it costs no
/// allocation; the first failure fills them, and later updates move the
/// text through its box without replacing the box.
#[test]
fn the_slots_fill_on_the_first_failure_and_keep_their_box() {
    let base = Instant::now();
    let mut state = None;
    let mut text = None;
    assert!(!update_slot(&mut state, &mut text, None, false, base));
    assert!(!update_slot(&mut state, &mut text, None, true, base));
    assert!(
        state.is_none() && text.is_none(),
        "no failure, no allocation"
    );
    assert!(update_slot(
        &mut state,
        &mut text,
        failure(base),
        false,
        base
    ));
    let held: *const String = &**text.as_ref().expect("the failure raised a notice");
    assert!(state.is_some());
    let later = base + Duration::from_secs(1);
    assert!(!update_slot(
        &mut state,
        &mut text,
        failure(base),
        false,
        later
    ));
    assert!(
        std::ptr::eq(held, &**text.as_ref().unwrap()),
        "text kept its box"
    );
    assert!(update_slot(&mut state, &mut text, None, true, later));
    assert!(text.is_none(), "a login clears the row's notice");
    assert!(
        state.is_some(),
        "the clock stays so a stale failure cannot return"
    );
    assert!(!update_slot(
        &mut state,
        &mut text,
        failure(base),
        false,
        later
    ));
    assert!(text.is_none());
}
