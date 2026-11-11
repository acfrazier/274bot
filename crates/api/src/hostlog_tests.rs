use super::*;
use std::borrow::Cow;
use std::sync::Mutex;

type Captured = (Option<String>, Option<u32>, Source, Level, String);

struct Capture(Mutex<Vec<Captured>>);

impl Sink for Capture {
    fn record(&self, r: &Record<'_>) {
        self.0.lock().unwrap().push((
            r.slot.map(str::to_string),
            r.tick,
            r.source,
            r.level,
            r.message.to_string(),
        ));
    }
}

static CAPTURE: Capture = Capture(Mutex::new(Vec::new()));

fn install() {
    install_sink(&CAPTURE);
}

fn lines_for(slot: &str) -> Vec<Captured> {
    CAPTURE
        .0
        .lock()
        .unwrap()
        .iter()
        .filter(|c| c.0.as_deref() == Some(slot))
        .cloned()
        .collect()
}

#[test]
fn allowlisted_categories_reach_the_sink_and_traces_never_do() {
    install();
    std::thread::spawn(|| {
        bind_slot("route-alice");
        set_tick(41);
        for category in [
            Category::Lifecycle,
            Category::Login,
            Category::ScriptLifecycle,
            Category::Watchdog,
            Category::RandomEvent,
            Category::NavEvent,
            Category::BankOp,
            Category::FrameStats,
            Category::NavTrace,
            Category::InteractTrace,
            Category::ScriptTrace,
            Category::Echo,
        ] {
            crate::host_log!(category, Level::Info, "{}", category.tag());
        }
    })
    .join()
    .unwrap();
    let got: Vec<String> = lines_for("route-alice").into_iter().map(|c| c.4).collect();
    assert_eq!(
        got,
        ["host", "login", "script", "watchdog", "random", "nav", "bank"],
        "per-frame and per-tick trace categories stay off the slot log"
    );
}

#[test]
fn a_bound_slot_thread_stamps_slot_tick_and_source() {
    install();
    std::thread::spawn(|| {
        bind_slot("route-bob");
        set_tick(7);
        crate::host_log!(Category::Watchdog, Level::Warn, "hung loop (10s)");
        crate::host_log!(
            Category::Login,
            Level::Info,
            slot = "route-carol",
            "explicit"
        );
    })
    .join()
    .unwrap();
    assert_eq!(
        lines_for("route-bob"),
        [(
            Some("route-bob".into()),
            Some(7),
            Source::Watchdog,
            Level::Warn,
            "hung loop (10s)".into()
        )]
    );
    let carol = lines_for("route-carol");
    assert_eq!(carol.len(), 1, "an explicit slot overrides the bound one");
    assert_eq!(carol[0].2, Source::Login);
}

#[test]
fn trace_arguments_are_not_evaluated_while_debug_is_off() {
    install();
    if debug_enabled() {
        return; // BOT_DEBUG=1 in the environment: traces print by design.
    }
    let mut evaluated = false;
    let mut probe = || {
        evaluated = true;
        "x"
    };
    crate::host_log!(Category::NavTrace, Level::Debug, "{}", probe());
    assert!(
        !evaluated,
        "a stderr-only category must not format while off"
    );
}

#[test]
fn overlapping_and_nested_secrets_never_leave_a_fragment() {
    // Registered shortest first: a sequential replace would store "***22".
    register_secret("sekret");
    register_secret("sekret22");
    register_secret("wxyz");
    register_secret("yzab");
    assert_eq!(
        redact("login rejected password sekret22"),
        "login rejected password ***"
    );
    assert_eq!(redact("sekret and sekret22."), "*** and ***.");
    assert_eq!(
        redact("key=wxyzab!"),
        "key=***!",
        "overlapping secrets merge"
    );
    assert!(matches!(redact("nothing here"), Cow::Borrowed(_)));
}

#[test]
fn short_passwords_and_passwords_equal_to_the_name_are_redacted_too() {
    register_secret("q7");
    register_secret("route-dora");
    assert_eq!(redact("pw=q7;"), "pw=***;");
    assert_eq!(redact("route-dora logged in"), "*** logged in");
}

#[test]
fn stderr_and_the_sink_only_ever_see_the_redacted_line() {
    install();
    register_secret("hunter2hunter");
    std::thread::spawn(|| {
        bind_slot("route-erin");
        crate::host_log!(
            stderr;
            Category::Login,
            Level::Error,
            "handshake with hunter2hunter refused"
        );
        let stderr = STDERR.with(|lines| lines.borrow().clone());
        assert_eq!(stderr, ["[login route-erin] handshake with *** refused"]);
    })
    .join()
    .unwrap();
    assert_eq!(
        lines_for("route-erin")
            .into_iter()
            .map(|c| c.4)
            .collect::<Vec<_>>(),
        ["handshake with *** refused"]
    );
}

#[test]
fn any_thread_reads_a_slots_current_tick() {
    install();
    let (tx, rx) = std::sync::mpsc::channel();
    let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
    let worker = std::thread::spawn(move || {
        bind_slot("route-fay");
        tx.send(()).unwrap();
        done_rx.recv().unwrap();
        set_tick(314);
        tx.send(()).unwrap();
        done_rx.recv().unwrap();
    });
    rx.recv().unwrap();
    assert_eq!(
        slot_tick("route-fay"),
        None,
        "no tick before the first observation"
    );
    done_tx.send(()).unwrap();
    rx.recv().unwrap();
    assert_eq!(slot_tick("route-fay"), Some(314));
    // A line for that slot logged on another thread carries its tick.
    crate::host_log!(
        Category::ScriptLifecycle,
        Level::Info,
        slot = "route-fay",
        "start load"
    );
    assert_eq!(lines_for("route-fay")[0].1, Some(314));
    done_tx.send(()).unwrap();
    worker.join().unwrap();
}

/// A stderr that refuses every write, as a redirected stderr on a full disk does.
struct FullDisk;

impl std::io::Write for FullDisk {
    fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
        Err(std::io::Error::from(std::io::ErrorKind::StorageFull))
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Err(std::io::Error::from(std::io::ErrorKind::StorageFull))
    }
}

#[test]
fn a_failed_debug_write_is_dropped_instead_of_panicking() {
    // `eprintln!` panics here; a panic while a caller holds the shared nav
    // lock poisoned it for every slot (T14-POISON-DIAG).
    write_line(&mut FullDisk, format_args!("[nav-trace slot] walk-send"));

    let mut out = Vec::new();
    write_line(&mut out, format_args!("[nav-trace slot] walk-send"));
    assert_eq!(out, b"[nav-trace slot] walk-send\n");
}

/// Strip the `[i/n] ` marker of `line`, checking it numbers line `index` of
/// `count`.
fn unmarked(line: &str, index: usize, count: usize) -> &str {
    let marker = format!("[{}/{count}] ", index + 1);
    line.strip_prefix(&marker)
        .unwrap_or_else(|| panic!("line {index} of {count} starts with {marker:?}: {line:?}"))
}

#[test]
fn pack_splits_on_char_boundaries_and_loses_nothing() {
    // Each fixture puts one multibyte character across the 470-byte payload
    // limit (480 bytes minus the widest marker): its first byte is at 469, so
    // the limit falls inside the character.
    for (name, wide) in [("2-byte", 'é'), ("3-byte", '日'), ("4-byte", '😀')] {
        let mut text = "a".repeat(469);
        text.push(wide);
        text.push_str(&"b".repeat(500));
        text.push(wide);
        let lines = pack(&[text.as_str()]);
        assert!(lines.len() > 1, "{name}: the text needs several lines");
        let mut rejoined = String::new();
        for (index, line) in lines.iter().enumerate() {
            assert!(line.len() <= PACK_BYTES, "{name}: {} bytes", line.len());
            assert!(!line.contains('\u{FFFD}'), "{name}: no replacement char");
            rejoined.push_str(unmarked(line, index, lines.len()));
        }
        assert_eq!(rejoined, text, "{name}: nothing is lost or reordered");
    }
}

#[test]
fn pack_gives_one_unmarked_line_when_the_text_fits_and_nothing_for_no_items() {
    assert_eq!(pack(&["alpha", "beta"]), ["alpha; beta"]);
    assert_eq!(
        pack(&["line one\nline two\ttab"]),
        ["line one line two tab"]
    );
    assert!(pack::<&str>(&[]).is_empty());
}

#[test]
fn pack_ends_a_line_after_an_item_so_items_stay_whole() {
    // 60 items of 20 bytes join to 1,318 bytes: several lines, each cut at
    // the last whole item that fits.
    let items: Vec<String> = (0..60)
        .map(|i| format!("item-{i:02}-xxxxxxxxxxxx"))
        .collect();
    let lines = pack(&items);
    assert!(lines.len() > 1);
    let mut rejoined = String::new();
    for (index, line) in lines.iter().enumerate() {
        let payload = unmarked(line, index, lines.len());
        assert!(
            payload.ends_with("; ") || index + 1 == lines.len(),
            "a line ends on an item boundary: {payload:?}"
        );
        rejoined.push_str(payload);
    }
    assert_eq!(rejoined, items.join("; "));
    for item in &items {
        assert_eq!(
            lines.iter().filter(|l| l.contains(item.as_str())).count(),
            1,
            "{item} appears once"
        );
    }
}

#[test]
fn emit_packed_writes_every_line_in_order_with_its_head_under_the_store_cap() {
    install();
    let items: Vec<String> = (0..99)
        .map(|i| format!("ogre {i:02} at (2567,3046,0) [ogre@2567,3046,{i}]"))
        .collect();
    let head = "WalkTo detail ";
    std::thread::spawn(move || {
        emit_packed(
            Emit {
                category: Category::NavEvent,
                level: Level::Info,
                slot: Some("route-gus"),
                always_stderr: false,
            },
            head,
            &items,
        );
    })
    .join()
    .unwrap();
    let lines: Vec<String> = lines_for("route-gus").into_iter().map(|c| c.4).collect();
    assert!(
        lines.len() > 1 && lines.len() <= 16,
        "{} lines",
        lines.len()
    );
    let mut rejoined = String::new();
    for (index, line) in lines.iter().enumerate() {
        assert!(line.len() <= LINE_BYTES, "{} bytes", line.len());
        let body = line
            .strip_prefix(head)
            .expect("every line starts with the head");
        rejoined.push_str(unmarked(body, index, lines.len()));
    }
    let expected: Vec<String> = (0..99)
        .map(|i| format!("ogre {i:02} at (2567,3046,0) [ogre@2567,3046,{i}]"))
        .collect();
    assert_eq!(rejoined, expected.join("; "));
}

#[test]
fn the_in_app_log_check_follows_the_sink_and_the_allowlist_not_debug_or_files() {
    install();
    assert!(logged(Category::Lifecycle) && logged(Category::NavEvent));
    assert!(
        !logged(Category::NavTrace) && !logged(Category::Echo),
        "stderr-only categories never reach the ring, even with debug on"
    );
}

#[test]
fn cut_chars_counts_characters_ends_with_an_ellipsis_and_borrows_when_it_fits() {
    assert!(matches!(cut_chars("short", 100), Cow::Borrowed("short")));
    assert!(matches!(cut_chars("abcde", 5), Cow::Borrowed("abcde")));
    assert_eq!(cut_chars("abcdef", 5), "abcd…");
    // A 4-byte character at the cut stays whole: 100 characters, not bytes.
    let wide = "😀".repeat(150);
    let cut = cut_chars(&wide, SUMMARY_CHARS);
    assert_eq!(cut.chars().count(), SUMMARY_CHARS);
    assert!(cut.ends_with('…') && cut.starts_with("😀"));
}
