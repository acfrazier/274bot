use super::*;

/// A front end's need, as a test names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Need {
    FocusedBot,
}

type Clock = ErrorExpiry<Need>;

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
    let mut clock = Clock::default();
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
    let mut clock = Clock::default();
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
    let mut clock = Clock::default();
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
    let mut clock = Clock::default();
    let mut error = shown("Copy settings: focus a bot first");
    clock.waits_for(Need::FocusedBot, "Copy settings: focus a bot first");
    clock.observe(&mut error, 1, base);
    assert_eq!(clock.waiting_need(), Some(&Need::FocusedBot));

    assert!(!clock.resolve(&mut error, |_| false), "the need is not met");
    assert_eq!(error.as_deref(), Some("Copy settings: focus a bot first"));
    assert!(clock.resolve(&mut error, |need| *need == Need::FocusedBot));
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
    let mut clock = Clock::default();
    clock.waits_for(Need::FocusedBot, "script: no focused profile");
    let mut error = shown("settings: write failed");
    assert!(!clock.resolve(&mut error, |_| true));
    assert_eq!(error.as_deref(), Some("settings: write failed"));
    assert_eq!(clock.waiting_need(), None, "the stale need is dropped");
}

/// Every line of the process ring that holds `needle`, oldest first.
fn ring_lines(scope: crate::log::LogScope, needle: char) -> Vec<String> {
    let mut view = crate::log::LogView::new(scope);
    crate::log::global().refresh(&mut view);
    view.rows()
        .iter()
        .map(|row| row.message().to_owned())
        .filter(|message| message.contains(needle))
        .collect()
}

/// Strip the head and the `[i/n] ` marker from a group's lines and join them.
fn rejoin(lines: &[String], head: &str) -> String {
    lines
        .iter()
        .enumerate()
        .map(|(index, line)| {
            let body = line
                .strip_prefix(head)
                .expect("every line starts with the head");
            let marker = format!("[{}/{}] ", index + 1, lines.len());
            body.strip_prefix(&marker).expect("numbered").to_owned()
        })
        .collect()
}

#[test]
fn observe_logs_an_over_cap_multibyte_message_in_full_once_across_many_pumps() {
    use crate::log::LogScope;
    // The sink must be installed before the first line is written.
    crate::log::global();
    // 2-, 3- and 4-byte characters, 6,000 characters, one marker character
    // that no other test writes.
    let text: String = "ζ日😀 abc; ".chars().cycle().take(6_000).collect();
    assert!(text.len() > 9_000, "{} bytes", text.len());
    let base = Instant::now();
    let mut clock = Clock::default();
    let mut error = Some(text.clone());
    for pump in 0..100 {
        assert!(!clock.observe(&mut error, 0, at(base, pump / 20)));
    }
    assert_eq!(
        error.as_deref(),
        Some(text.as_str()),
        "the stored text is unchanged"
    );

    let lines = ring_lines(LogScope::Process, 'ζ');
    assert!(lines.len() > 1, "{} lines", lines.len());
    for line in &lines {
        assert!(
            line.len() <= api::hostlog::LINE_BYTES,
            "{} bytes",
            line.len()
        );
        assert!(!line.ends_with('…'), "the store cut a line: {line}");
    }
    assert_eq!(
        rejoin(&lines, "notice shortened: "),
        text,
        "one group holds the whole text"
    );

    // A published brief of the same text is one short line: the guard does not
    // log it again, and the detail is in the ring once, written by `publish`.
    let slot = "expiry-from-text";
    let shown = crate::Brief::from_text(&text).publish(&crate::LogTarget::command(Some(slot)));
    let mut error = Some(shown);
    clock.observe(&mut error, 0, at(base, 30));
    clock.observe(&mut error, 0, at(base, 31));
    assert_eq!(
        ring_lines(LogScope::Process, 'ζ'),
        lines,
        "the guard logged nothing for the short line"
    );
    let slot_lines = ring_lines(LogScope::slot(slot), 'ζ');
    assert_eq!(rejoin(&slot_lines, "detail: "), text, "one group, once");
}

#[test]
fn a_one_line_message_in_the_cap_is_not_logged_by_the_guard() {
    crate::log::global();
    let before = ring_lines(crate::log::LogScope::Process, 'η').len();
    let mut clock = Clock::default();
    let mut error = shown(&"η".repeat(crate::LINE_MAX));
    clock.observe(&mut error, 0, Instant::now());
    assert_eq!(ring_lines(crate::log::LogScope::Process, 'η').len(), before);
}
