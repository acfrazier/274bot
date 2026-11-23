use super::*;
use std::borrow::Cow;
use std::io::Write as _;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

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
#[cfg(feature = "test-support")]
fn nav_trace_requires_a_sink_specific_test_opt_in() {
    struct TraceCapture {
        enabled: bool,
        lines: Mutex<Vec<String>>,
    }
    impl Sink for TraceCapture {
        fn record(&self, record: &Record<'_>) {
            self.lines.lock().unwrap().push(record.message.to_owned());
        }

        fn test_nav_trace_enabled(&self) -> bool {
            self.enabled
        }
    }
    for enabled in [false, true] {
        let capture = TraceCapture {
            enabled,
            lines: Mutex::new(Vec::new()),
        };
        for category in [Category::NavTrace, Category::InteractTrace] {
            if let Some(sink) = routed_sink(&capture, category) {
                sink.record(&Record {
                    slot: Some("alice"),
                    tick: Some(7),
                    source: category.source(),
                    level: Level::Debug,
                    message: category.tag(),
                });
            }
        }
        let expected: &[&str] = if enabled { &["nav-trace"] } else { &[] };
        assert_eq!(*capture.lines.lock().unwrap(), expected);
    }
    assert!(!Category::NavTrace.slot_log());
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
fn debug_and_always_stderr_errors_use_the_redacted_writer_path() {
    install();
    register_secret("redaction-token");
    let old_debug = DEBUG.load(Ordering::Relaxed);
    set_debug(true);
    std::thread::spawn(|| {
        bind_slot("route-gina");
        crate::host_log!(Category::NavTrace, Level::Debug, "debug redaction-token");
        crate::host_log!(
            stderr;
            Category::Login,
            Level::Error,
            "error redaction-token"
        );
        assert_eq!(
            STDERR.with(|lines| lines.borrow().clone()),
            [
                "[nav-trace route-gina] debug ***",
                "[login route-gina] error ***"
            ]
        );
    })
    .join()
    .unwrap();
    set_debug(old_debug);

    assert_eq!(
        lines_for("route-gina")
            .into_iter()
            .map(|line| line.4)
            .collect::<Vec<_>>(),
        ["error ***"]
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

/// Writes into a shared buffer; the first write fails when `fail_first` is set.
struct SharedWriter {
    fail_first: bool,
    bytes: Arc<Mutex<Vec<u8>>>,
}

impl SharedWriter {
    fn new(fail_first: bool) -> (Self, Arc<Mutex<Vec<u8>>>) {
        let bytes = Arc::new(Mutex::new(Vec::new()));
        (
            Self {
                fail_first,
                bytes: Arc::clone(&bytes),
            },
            bytes,
        )
    }
}

impl std::io::Write for SharedWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if std::mem::take(&mut self.fail_first) {
            return Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe));
        }
        self.bytes.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// An output whose writer thread writes into `writer`.
fn output_writing_to(mut writer: impl std::io::Write + Send + 'static) -> StderrOutput {
    StderrOutput::start_with(move |rx, counters| {
        run_stderr_writer(rx, counters, || {}, |text| write_line(&mut writer, text));
    })
}

const LONG_WAIT: Duration = Duration::from_secs(5);

#[test]
fn a_failed_debug_write_is_dropped_instead_of_panicking() {
    let (writer, bytes) = SharedWriter::new(true);
    let output = output_writing_to(writer);
    output.enqueue(format_args!("[nav-trace slot] debug first"));
    // The writer finishes the first entry before it takes the second.
    wait_until(|| output.counters.pending.load(Ordering::Acquire) == 0);
    output.enqueue(format_args!("[nav-trace slot] debug second"));
    assert_eq!(output.shutdown(LONG_WAIT), 0);
    assert_eq!(output.dropped(), 1);
    assert_eq!(&*bytes.lock().unwrap(), b"[nav-trace slot] debug second\n");
}

fn wait_until(mut done: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + LONG_WAIT;
    while !done() {
        assert!(
            std::time::Instant::now() < deadline,
            "condition not reached"
        );
        std::thread::yield_now();
    }
}

#[test]
fn stderr_lock_is_available_while_the_process_writer_waits() {
    let (output, receiver) = StderrOutput::channel();
    let counters = Arc::clone(&output.counters);
    let (waiting_tx, waiting_rx) = std::sync::mpsc::sync_channel(1);
    let worker = std::thread::spawn(move || {
        let mut waiting_tx = Some(waiting_tx);
        run_process_stderr_writer(receiver, counters, || {
            if let Some(waiting_tx) = waiting_tx.take() {
                let _ = waiting_tx.send(());
            }
        });
    });
    waiting_rx
        .recv_timeout(LONG_WAIT)
        .expect("process writer did not reach its idle wait");

    let (probe_tx, probe_rx) = std::sync::mpsc::sync_channel(1);
    let probe = std::thread::spawn(move || {
        eprintln!("stderr lock probe");
        let mut stderr = std::io::stderr();
        let _ = stderr.flush();
        let _ = writeln!(stderr, "stderr write probe");
        let _ = probe_tx.send(());
    });
    let probe_returned = probe_rx.recv_timeout(LONG_WAIT).is_ok();
    output.shutdown(LONG_WAIT);
    probe.join().unwrap();
    worker.join().unwrap();
    assert!(
        probe_returned,
        "the idle process stderr writer held the standard stderr lock"
    );
}

/// The text of `entry` with its `[i/n] ` markers removed and its lines joined.
fn unnumbered(entry: &Entry) -> String {
    let n = entry.lines;
    entry
        .text
        .lines()
        .enumerate()
        .map(|(index, line)| {
            let marker = format!("[{}/{n}] ", index + 1);
            line.strip_prefix(&marker)
                .unwrap_or_else(|| panic!("line {index} lacks marker {marker}: {line:?}"))
                .to_owned()
        })
        .collect()
}

#[test]
fn a_text_that_fits_one_line_is_not_numbered_and_a_longer_one_is_split_not_cut() {
    let fits = "a".repeat(STDERR_LINE_BYTES);
    let entry = format_entry(format_args!("{fits}")).unwrap();
    assert_eq!((entry.lines, entry.text.as_str()), (1, fits.as_str()));

    // One byte over: two numbered lines, and every byte survives.
    let over = "b".repeat(STDERR_LINE_BYTES + 1);
    let entry = format_entry(format_args!("{over}")).unwrap();
    assert_eq!(entry.lines, 2);
    assert!(entry
        .text
        .lines()
        .all(|line| line.len() <= STDERR_LINE_BYTES));
    assert!(entry.text.starts_with("[1/2] "));
    assert_eq!(unnumbered(&entry), over);

    // Multi-byte characters stay whole at every cut.
    for unit in ["é", "€", "😀"] {
        let text = unit.repeat(2_500);
        let entry = format_entry(format_args!("[nav-trace] {text}")).unwrap();
        assert!(entry.lines > 1);
        assert_eq!(entry.text.lines().count(), entry.lines);
        assert!(entry
            .text
            .lines()
            .all(|line| line.len() <= STDERR_LINE_BYTES));
        assert_eq!(unnumbered(&entry), format!("[nav-trace] {text}"));
    }
}

#[test]
fn the_largest_entry_fits_its_line_budget_and_a_larger_one_is_refused_whole() {
    for unit in ["a", "é", "€", "😀"] {
        let text = unit.repeat(STDERR_ENTRY_BYTES / unit.len());
        assert!(text.len() <= STDERR_ENTRY_BYTES);
        let entry = format_entry(format_args!("{text}")).expect("an entry at the cap");
        assert!(
            entry.lines <= STDERR_ENTRY_LINES,
            "{unit}: {} lines",
            entry.lines
        );
        assert_eq!(unnumbered(&entry), text);
    }

    let (output, _receiver) = StderrOutput::channel();
    let too_long = "x".repeat(STDERR_ENTRY_BYTES + 1);
    output.enqueue(format_args!("{too_long}"));
    assert_eq!(output.dropped(), 1, "a refused entry counts once");
    assert_eq!(output.counters.queued.load(Ordering::Acquire), 0);
    assert_eq!(output.counters.pending.load(Ordering::Acquire), 0);
}

#[test]
fn a_split_entry_holds_all_its_lines_or_is_refused_whole() {
    let (output, _receiver) = StderrOutput::channel();
    let three_lines = "z".repeat(3 * STDERR_PIECE_BYTES - 10);
    output.enqueue(format_args!("{three_lines}"));
    assert_eq!(output.counters.queued.load(Ordering::Acquire), 3);
    assert_eq!(output.counters.pending.load(Ordering::Acquire), 1);

    for _ in 0..STDERR_QUEUE_LINES - 4 {
        output.enqueue(format_args!("filler"));
    }
    assert_eq!(
        output.counters.queued.load(Ordering::Acquire),
        STDERR_QUEUE_LINES - 1
    );
    // One line is free, two are needed: nothing stays reserved.
    let two_lines = "y".repeat(STDERR_LINE_BYTES + 1);
    output.enqueue(format_args!("{two_lines}"));
    assert_eq!(output.dropped(), 1);
    assert_eq!(
        output.counters.queued.load(Ordering::Acquire),
        STDERR_QUEUE_LINES - 1
    );
    assert_eq!(
        output.counters.pending.load(Ordering::Acquire),
        STDERR_QUEUE_LINES - 3
    );
}

#[test]
fn a_split_entry_reaches_the_sink_in_one_piece_between_other_entries() {
    let (writer, bytes) = SharedWriter::new(false);
    let output = output_writing_to(writer);
    let long = "w".repeat(2 * STDERR_PIECE_BYTES);
    output.enqueue(format_args!("before"));
    output.enqueue(format_args!("{long}"));
    output.enqueue(format_args!("after"));
    assert_eq!(output.shutdown(LONG_WAIT), 0);
    let written = String::from_utf8(bytes.lock().unwrap().clone()).unwrap();
    let lines: Vec<&str> = written.lines().collect();
    assert_eq!(lines.len(), 4);
    assert_eq!(lines[0], "before");
    assert!(lines[1].starts_with("[1/2] ") && lines[2].starts_with("[2/2] "));
    assert_eq!(lines[3], "after");
    assert_eq!(output.dropped(), 0);
}

#[test]
fn stderr_queue_allocation_and_size_stay_within_the_process_bound() {
    assert_eq!(STDERR_QUEUE_LINES, 256);
    assert_eq!(STDERR_LINE_BYTES, 1_024);
    assert_eq!(
        std::mem::size_of::<String>(),
        3 * std::mem::size_of::<usize>()
    );
    assert!(std::mem::size_of::<StderrOutput>() <= 128);
    assert!(std::mem::size_of::<Entry>() <= 32);

    let init = allocation_counter::measure(|| {
        let (output, receiver) = StderrOutput::channel();
        drop(output);
        drop(receiver);
    });
    assert!(
        init.bytes_total <= 16 * 1024,
        "fixed queue metadata allocated {} bytes",
        init.bytes_total
    );

    let (output, _receiver) = StderrOutput::channel();
    let queued = allocation_counter::measure(|| {
        for index in 0..STDERR_QUEUE_LINES {
            output.enqueue(format_args!("line {index}"));
        }
    });
    let max_line_storage = STDERR_QUEUE_LINES * STDERR_LINE_BYTES;
    assert_eq!(queued.count_total, STDERR_QUEUE_LINES as u64);
    assert_eq!(queued.bytes_total, max_line_storage as u64);
    assert_eq!(queued.bytes_current, max_line_storage as i64);
    assert_eq!(
        output.counters.queued.load(Ordering::Acquire),
        STDERR_QUEUE_LINES
    );
    assert_eq!(output.dropped(), 0);

    let full_queue_drop = allocation_counter::measure(|| {
        output.enqueue(format_args!("one more"));
    });
    assert_eq!(full_queue_drop.count_total, 0);
    assert_eq!(full_queue_drop.bytes_total, 0);
    assert_eq!(output.dropped(), 1);
}

#[test]
fn a_closed_stderr_queue_counts_the_refused_entry() {
    let (output, receiver) = StderrOutput::channel();
    drop(receiver);
    output.enqueue(format_args!("closed"));
    assert_eq!(output.dropped(), 1);
    assert_eq!(output.counters.queued.load(Ordering::Acquire), 0);
    assert_eq!(output.counters.pending.load(Ordering::Acquire), 0);
    output.shutdown(LONG_WAIT);
    output.enqueue(format_args!("after shutdown"));
    assert_eq!(output.dropped(), 2);
}

#[test]
fn stderr_queue_submission_drops_instead_of_waiting_for_sender_lock() {
    let (output, _receiver) = StderrOutput::channel();
    let output = Arc::new(output);
    let locked_sender = output.tx.lock();
    let thread_output = Arc::clone(&output);
    let (completed_tx, completed_rx) = std::sync::mpsc::sync_channel(1);
    let worker = std::thread::spawn(move || {
        thread_output.enqueue(format_args!("contended"));
        let _ = completed_tx.send(());
    });

    let completed = completed_rx
        .recv_timeout(Duration::from_millis(100))
        .is_ok();
    drop(locked_sender);
    worker.join().unwrap();
    assert!(completed, "slot submission waited on the sender lock");
    assert_eq!(output.dropped(), 1);
}

#[test]
fn shutdown_drains_every_accepted_entry_when_the_sink_is_writable() {
    let (writer, bytes) = SharedWriter::new(false);
    let output = output_writing_to(writer);
    let long = "l".repeat(STDERR_LINE_BYTES + 1);
    output.enqueue(format_args!("first"));
    output.enqueue(format_args!("{long}"));
    output.enqueue(format_args!("second"));
    assert_eq!(output.shutdown(LONG_WAIT), 0, "nothing is lost at exit");
    let written = String::from_utf8(bytes.lock().unwrap().clone()).unwrap();
    assert!(written.starts_with("first\n[1/2] "));
    assert!(written.ends_with("\nsecond\n"));
    assert_eq!(output.dropped(), 0);
    assert_eq!(output.counters.pending.load(Ordering::Acquire), 0);
    assert_eq!(output.counters.queued.load(Ordering::Acquire), 0);
    // A second shutdown is a no-op.
    assert_eq!(output.shutdown(LONG_WAIT), 0);
}

#[test]
fn a_failed_write_is_counted_once_per_entry_and_does_not_panic_the_worker() {
    let output = output_writing_to(FullDisk);
    let long = "l".repeat(STDERR_LINE_BYTES + 1);
    output.enqueue(format_args!("first"));
    output.enqueue(format_args!("{long}"));
    assert_eq!(output.shutdown(LONG_WAIT), 0);
    assert_eq!(output.dropped(), 2);
}

struct BlockingWriter {
    entered: std::sync::mpsc::Sender<()>,
    release: std::sync::mpsc::Receiver<()>,
    blocked_once: bool,
}

impl std::io::Write for BlockingWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if !self.blocked_once {
            self.blocked_once = true;
            let _ = self.entered.send(());
            let _ = self.release.recv();
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn shutdown_gives_up_on_a_stalled_sink_within_its_wait_and_counts_the_lost_entries() {
    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let output = output_writing_to(BlockingWriter {
        entered: entered_tx,
        release: release_rx,
        blocked_once: false,
    });
    output.enqueue(format_args!("blocked in the sink"));
    entered_rx
        .recv_timeout(LONG_WAIT)
        .expect("stderr writer did not reach the blocked sink");
    // Two more entries, one of them split, wait behind the blocked write.
    let long = "q".repeat(STDERR_LINE_BYTES + 1);
    output.enqueue(format_args!("queued behind it"));
    output.enqueue(format_args!("{long}"));
    assert_eq!(output.dropped(), 0);

    let started = std::time::Instant::now();
    let lost = output.shutdown(Duration::from_millis(50));
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "shutdown waited for the blocked sink: {:?}",
        started.elapsed()
    );
    assert_eq!(lost, 3, "the blocked entry and the two queued entries");
    assert_eq!(output.dropped(), 3);

    // The sink recovers: the abandoned thread still drains and ends.
    let _ = release_tx.send(());
    wait_until(|| output.counters.pending.load(Ordering::Acquire) == 0);
}

#[test]
fn reading_stderr_drop_count_does_not_start_the_writer() {
    assert!(std::sync::LazyLock::get(&STDERR_OUTPUT).is_none());
    assert_eq!(stderr_dropped(), 0);
    assert!(std::sync::LazyLock::get(&STDERR_OUTPUT).is_none());
}
