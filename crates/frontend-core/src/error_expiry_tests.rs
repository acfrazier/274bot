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

/// An action that resolves the message clears it at once; the next message
/// gets the whole timeout, not what was left of the old one.
#[test]
fn a_resolving_action_clears_the_message_early_and_resets_the_clock() {
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
