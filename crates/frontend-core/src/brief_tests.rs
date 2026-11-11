use super::*;
use crate::log::{global, LogScope, LogView};

/// Install the process log store as the `api::hostlog` sink. Every test that
/// publishes a brief calls this first: the pointer is shown only once a sink
/// is installed, and tests run in any order.
fn sink() -> &'static crate::log::LogStore {
    global()
}

/// Every message in `slot`'s ring, oldest first.
fn slot_lines(slot: &str) -> Vec<String> {
    let mut view = LogView::new(LogScope::slot(slot));
    global().refresh(&mut view);
    view.rows()
        .iter()
        .map(|row| row.message().to_owned())
        .collect()
}

/// The 99-zone detail of the live refusal that first showed the problem.
fn zone_items() -> Vec<String> {
    (0..99)
        .map(|i| {
            format!(
                "Ogre (L53) at 2567,{} level 0 [ogre@2567,{},0]",
                3000 + i,
                3000 + i
            )
        })
        .collect()
}

#[test]
fn brief_summary_is_cut_by_characters_not_bytes() {
    sink();
    // 100 three-byte characters, then a four-byte one at character 101.
    let text = format!("{}😀tail", "日".repeat(99));
    let brief = Brief::new(&text);
    let summary = brief.summary();
    assert!(summary.chars().count() <= SUMMARY_MAX, "{summary}");
    assert!(summary.ends_with('…'), "{summary}");
    assert!(!summary.contains('\u{FFFD}'));
    assert!(summary.starts_with("日日日"));

    // The cut lands on the four-byte character itself.
    let text = format!("{}😀😀tail", "a".repeat(98));
    let summary = Brief::new(&text).summary().to_owned();
    assert_eq!(summary.chars().count(), SUMMARY_MAX, "{summary}");
    assert!(summary.ends_with("😀…"), "{summary}");

    // With the pointer a shown line is at most LINE_MAX characters and
    // 448 bytes: one packed log line holds it.
    let line = Brief::new("😀".repeat(300))
        .with_detail(["x".to_owned()])
        .publish(&LogTarget::command(Some("brief-bytes")));
    assert!(line.chars().count() <= LINE_MAX, "{}", line.chars().count());
    assert!(line.len() <= LINE_MAX * 4, "{} bytes", line.len());
    assert!(line.ends_with(LOG_POINTER), "{line}");
}

#[test]
fn brief_detail_is_packed_into_lines_under_the_log_cap_and_loses_nothing() {
    sink();
    let slot = "brief-pack";
    let items = zone_items();
    let long = items.join("; ");
    assert!(long.len() > 4_000, "{} bytes", long.len());
    let line = Brief::new("Blocked by 99 danger zones")
        .with_detail(items.clone())
        .publish(&LogTarget::walk(Some(slot)));
    assert_eq!(line, format!("Blocked by 99 danger zones{LOG_POINTER}"));

    let lines = slot_lines(slot);
    assert!(
        lines.len() > 1 && lines.len() <= 16,
        "{} lines",
        lines.len()
    );
    let mut rejoined = String::new();
    for (index, entry) in lines.iter().enumerate() {
        assert!(
            entry.len() <= api::hostlog::LINE_BYTES,
            "{} bytes",
            entry.len()
        );
        assert!(!entry.ends_with('…'), "the store cut a line: {entry}");
        let body = entry
            .strip_prefix("walk detail: ")
            .expect("every line carries the head");
        let marker = format!("[{}/{}] ", index + 1, lines.len());
        rejoined.push_str(body.strip_prefix(&marker).expect("numbered line"));
    }
    assert_eq!(rejoined, long, "the log holds the whole detail");
    for item in &items {
        let key = item.rsplit('[').next().unwrap_or_default();
        assert_eq!(
            lines.iter().filter(|l| l.contains(key)).count(),
            1,
            "{key} appears once"
        );
    }
}

#[test]
fn brief_from_text_keeps_the_first_line_and_logs_the_rest_once() {
    sink();
    let slot = "brief-from-text";
    let text = format!(
        "load failed: {}\nsecond line {}",
        "ä".repeat(150),
        "z".repeat(700)
    );
    let line = Brief::from_text(&text).publish(&LogTarget::command(Some(slot)));
    assert!(line.chars().count() <= LINE_MAX, "{line}");
    assert!(
        !line.contains('\n') && !line.contains("second line"),
        "{line}"
    );
    assert!(
        line.starts_with("load failed: ää") && line.ends_with("… (see log)"),
        "{line}"
    );

    let lines = slot_lines(slot);
    assert!(lines.len() > 1);
    let joined: String = lines
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let body = entry.strip_prefix("detail: ").expect("head");
            let marker = format!("[{}/{}] ", index + 1, lines.len());
            body.strip_prefix(&marker).expect("marker").to_owned()
        })
        .collect();
    assert_eq!(joined, text.replace('\n', " "), "the whole text, once");

    // A published line is one short line: a second pass keeps it and logs
    // nothing more.
    let again = Brief::from_text(&line).publish(&LogTarget::command(Some(slot)));
    assert_eq!(again, line);
    assert_eq!(slot_lines(slot).len(), lines.len());
}

#[test]
fn a_short_text_is_kept_and_has_no_pointer() {
    sink();
    let slot = "brief-short";
    let line = Brief::from_text("Stop: no bot to act on").publish(&LogTarget::command(Some(slot)));
    assert_eq!(line, "Stop: no bot to act on");
    assert!(slot_lines(slot).is_empty());
    // 112 characters is still one line; 113 is not.
    let at_limit = "x".repeat(LINE_MAX);
    assert_eq!(
        Brief::from_text(&at_limit).publish(&LogTarget::command(Some(slot))),
        at_limit
    );
    let over = "x".repeat(LINE_MAX + 1);
    let cut = Brief::from_text(&over).publish(&LogTarget::command(Some(slot)));
    assert!(
        cut.chars().count() <= LINE_MAX && cut.ends_with(LOG_POINTER),
        "{cut}"
    );
}

/// The pointer follows the in-app ring, not the session file: with file
/// logging off (the default) the detail is in the ring and the line says so.
#[test]
fn the_pointer_is_shown_with_session_file_logging_off() {
    sink();
    let log = global();
    assert!(!log.file_open(), "no session file in this test");
    let slot = "brief-no-file";
    let line = Brief::new("Blocked by 2 danger zones")
        .with_detail(["a [a@1,1,0]".to_owned(), "b [b@2,2,0]".to_owned()])
        .publish(&LogTarget::walk(Some(slot)));
    assert!(line.ends_with(LOG_POINTER), "{line}");
    assert_eq!(slot_lines(slot).len(), 1, "the detail is in the ring");

    // A category outside the slot-log allowlist never reaches the ring, so
    // the line makes no promise.
    let line = Brief::new("trace only")
        .with_detail(["x".to_owned()])
        .publish(&LogTarget::trace_only(Some(slot)));
    assert_eq!(line, "trace only");
    // A producer that logged the detail itself only points.
    let line = Brief::new("Blocked")
        .logged_by_producer()
        .publish(&LogTarget::walk(Some(slot)));
    assert_eq!(line, format!("Blocked{LOG_POINTER}"));
    assert_eq!(
        slot_lines(slot).len(),
        1,
        "logged_by_producer writes nothing"
    );
}

#[test]
fn notice_line_keeps_the_first_line_in_the_cap_and_borrows_when_short() {
    assert!(matches!(notice_line("short"), Cow::Borrowed("short")));
    assert_eq!(notice_line("first\nsecond"), "first…");
    assert_eq!(notice_line("first\r\nsecond"), "first…");
    assert_eq!(notice_line("\nfirst"), "first");
    let long = "é".repeat(300);
    let line = notice_line(&long);
    assert_eq!(line.chars().count(), LINE_MAX);
    assert!(line.ends_with('…'));
    let multi = format!("{}\nmore", "é".repeat(LINE_MAX));
    assert!(notice_line(&multi).chars().count() <= LINE_MAX);
    assert!(notice_line(&multi).ends_with('…'));
}

/// One fixture per long producer of the notice census: whatever the producer
/// says, the line a front end shows is one line of at most `LINE_MAX`
/// characters.
#[test]
fn every_notice_producer_fits_the_line_bound() {
    sink();
    use host_play::walk_map::{ActionError, BlockedZones};
    let fits = |label: &str, line: &str| {
        assert!(
            line.chars().count() <= LINE_MAX,
            "{label}: {} chars: {line}",
            line.chars().count()
        );
        assert!(!line.contains(['\n', '\r']), "{label}: {line}");
    };

    // A walk refused for 99 danger zones (6,360 characters of detail).
    let items: Vec<String> = (0..99)
        .map(|i| {
            format!(
                "Ogre (L53) at 2567,{} level 0 [ogre@2567,{},0]",
                3000 + i,
                3000 + i
            )
        })
        .collect();
    let error = crate::walk_map::WalkActionError::from_host(ActionError::BlockedByZones {
        zones: Some(BlockedZones::new(items)),
    });
    let line = error.brief().publish(&LogTarget::walk(Some("census-walk")));
    fits("99 zones", &line);
    assert!(
        !line.contains("ogre@") && line.contains("99 danger zones"),
        "{line}"
    );

    // A refusal of any other owner, however long and however many lines.
    for (label, text) in [
        (
            "50 bots",
            format!(
                "Reload: {}",
                (0..50)
                    .map(|i| format!("bot-{i:02}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ),
        (
            "300-character path",
            format!("settings: write failed: {}", "dir/".repeat(75)),
        ),
        ("two lines", "first line\nsecond line".to_owned()),
        ("wide characters", "日本語".repeat(100)),
    ] {
        let line = Brief::from_text(&text).publish(&LogTarget::command(Some("census-text")));
        fits(label, &line);
    }

    // A parked script's 6,360-character block reason, on both panes.
    let reason = format!(
        "blocked by danger zones: {}",
        "Ogre (L53) at 2567,3046 level 0 [ogre@2567,3046,0], ".repeat(120)
    );
    assert!(reason.chars().count() > 6_000);
    let pane = crate::views::ReasonLine::new(&reason).as_str().into_owned();
    assert!(pane.chars().count() <= SUMMARY_MAX, "{pane}");
    fits("script reason", &pane);

    // The last guard: any direct write.
    fits("direct write", &notice_line(&reason));
}
