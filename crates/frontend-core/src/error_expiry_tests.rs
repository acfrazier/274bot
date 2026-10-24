use super::*;

fn at(base: Instant, seconds: u64) -> Instant {
    base + Duration::from_secs(seconds)
}

fn shown(text: &str) -> Option<String> {
    Some(text.to_owned())
}

/// The message goes by itself after the timeout, not before.
#[test]
fn a_message_expires_after_the_timeout_with_no_action() {
    let base = Instant::now();
    let mut clock = ErrorExpiry::default();
    let mut error = shown("Stop: no bot to act on");
    assert!(!clock.observe(&mut error, 0, base));
    assert!(!clock.observe(&mut error, 0, at(base, 14)));
    assert_eq!(error.as_deref(), Some("Stop: no bot to act on"));
    assert_eq!(clock.expires_at(), Some(at(base, 15)));
    assert!(clock.observe(&mut error, 0, at(base, 15)));
    assert_eq!(error, None);
    assert_eq!(clock.expires_at(), None);
}

/// A message cleared by the front end (the action that fixed it) resets the
/// clock; the next message gets the whole timeout, not what was left.
#[test]
fn a_cleared_message_resets_the_clock() {
    let base = Instant::now();
    let mut clock = ErrorExpiry::default();
    let mut error = shown("settings: write failed");
    clock.observe(&mut error, 0, base);
    error = None;
    assert!(!clock.observe(&mut error, 0, at(base, 5)));
    assert_eq!(clock.expires_at(), None);
    error = shown("settings: write failed");
    clock.observe(&mut error, 0, at(base, 12));
    assert!(!clock.observe(&mut error, 0, at(base, 26)));
    assert!(clock.observe(&mut error, 0, at(base, 27)));
}

/// A newer message replaces the old one and restarts the timeout, whether its
/// text differs or it is the same refusal again.
#[test]
fn a_newer_message_restarts_the_timer() {
    let base = Instant::now();
    let mut clock = ErrorExpiry::default();
    let mut error = shown("first");
    clock.observe(&mut error, 0, base);
    error = shown("second");
    assert!(!clock.observe(&mut error, 0, at(base, 10)));
    assert!(!clock.observe(&mut error, 0, at(base, 24)));
    assert_eq!(error.as_deref(), Some("second"));
    // The same text refused again, 14 s later, starts over.
    assert!(!clock.observe(&mut error, 1, at(base, 24)));
    assert!(!clock.observe(&mut error, 1, at(base, 38)));
    assert!(clock.observe(&mut error, 1, at(base, 39)));
    assert_eq!(error, None);
}

/// A message that waits for a need clears when the need holds, and not before;
/// the next message gets the whole timeout.
#[test]
fn a_message_waiting_for_a_need_clears_when_the_need_holds() {
    let base = Instant::now();
    let mut clock = ErrorExpiry::default();
    let mut error = shown("Copy settings: focus a bot first");
    clock.waits_for(ErrorNeed::FocusedBot, "Copy settings: focus a bot first");
    clock.observe(&mut error, 1, base);
    assert_eq!(clock.waiting_need(), Some(ErrorNeed::FocusedBot));

    assert!(!clock.resolve(&mut error, |_| false), "the need is not met");
    assert_eq!(error.as_deref(), Some("Copy settings: focus a bot first"));
    assert!(clock.resolve(&mut error, |need| need == ErrorNeed::FocusedBot));
    assert_eq!(error, None);
    assert_eq!(clock.waiting_need(), None);
    assert!(!clock.observe(&mut error, 1, at(base, 5)));
    assert_eq!(clock.expires_at(), None, "the clock restarted");

    error = shown("next");
    clock.observe(&mut error, 1, at(base, 12));
    assert!(!clock.observe(&mut error, 1, at(base, 26)));
    assert!(clock.observe(&mut error, 1, at(base, 27)));
}

/// A newer message with other text is not cleared by the need of an older one.
#[test]
fn a_need_never_clears_a_different_message() {
    let mut clock = ErrorExpiry::default();
    clock.waits_for(ErrorNeed::FocusedBot, "script: no focused profile");
    let mut error = shown("settings: write failed");
    assert!(!clock.resolve(&mut error, |_| true));
    assert_eq!(error.as_deref(), Some("settings: write failed"));
    assert_eq!(clock.waiting_need(), None, "the stale need is dropped");
}
