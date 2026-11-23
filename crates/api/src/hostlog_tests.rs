use super::*;
use std::borrow::Cow;
use std::io::Write as _;
use std::sync::Arc;
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

#[derive(Default)]
struct FailOnceWriter {
    failed_once: bool,
    bytes: Vec<u8>,
}

impl std::io::Write for FailOnceWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if !self.failed_once {
            self.failed_once = true;
            return Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn a_failed_debug_write_is_dropped_instead_of_panicking() {
    let (output, receiver) = StderrOutput::channel();
    let dropped = Arc::clone(&output.dropped);
    let queued = Arc::clone(&output.queued);
    let worker = std::thread::spawn(move || {
        let mut writer = FailOnceWriter::default();
        run_stderr_writer(
            receiver,
            dropped,
            queued,
            || {},
            |line| write_line(&mut writer, line),
        );
        writer
    });

    output.enqueue(format_args!("[nav-trace slot] debug first"));
    output.enqueue(format_args!("[nav-trace slot] debug second"));
    output.shutdown();
    let writer = worker.join().unwrap();
    assert_eq!(output.dropped(), 1);
    assert_eq!(writer.bytes, b"[nav-trace slot] debug second\n");
}

#[test]
fn stderr_lock_is_available_while_the_process_writer_waits() {
    let (output, receiver) = StderrOutput::channel();
    let dropped = Arc::clone(&output.dropped);
    let queued = Arc::clone(&output.queued);
    let (waiting_tx, waiting_rx) = std::sync::mpsc::sync_channel(1);
    let worker = std::thread::spawn(move || {
        let mut waiting_tx = Some(waiting_tx);
        run_process_stderr_writer(receiver, dropped, queued, || {
            if let Some(waiting_tx) = waiting_tx.take() {
                let _ = waiting_tx.send(());
            }
        });
    });
    waiting_rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("process writer did not reach its idle wait");

    let (probe_tx, probe_rx) = std::sync::mpsc::sync_channel(1);
    let probe = std::thread::spawn(move || {
        eprintln!("stderr lock probe");
        let mut stderr = std::io::stderr();
        let _ = stderr.flush();
        let _ = writeln!(stderr, "stderr write probe");
        let _ = probe_tx.send(());
    });
    let probe_returned = probe_rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .is_ok();
    output.shutdown();
    probe.join().unwrap();
    worker.join().unwrap();
    assert!(
        probe_returned,
        "the idle process stderr writer held the standard stderr lock"
    );
}

#[test]
fn stderr_lines_keep_utf8_and_never_exceed_the_byte_limit() {
    let line = format_stderr_line(format_args!("[nav-trace] {}", "é".repeat(1_000)));
    assert!(line.len() <= STDERR_LINE_BYTES);
    assert!(line.ends_with(TRUNCATION_MARKER));
    assert!(line.is_char_boundary(line.len()));
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
    assert_eq!(output.queued.load(Ordering::Acquire), STDERR_QUEUE_LINES);
    assert_eq!(output.dropped(), 0);

    let full_queue_drop = allocation_counter::measure(|| {
        output.enqueue(format_args!("one more"));
    });
    assert_eq!(full_queue_drop.count_total, 0);
    assert_eq!(full_queue_drop.bytes_total, 0);
    assert_eq!(output.dropped(), 1);
}

#[test]
fn a_closed_stderr_queue_counts_the_refused_line() {
    let (output, receiver) = StderrOutput::channel();
    drop(receiver);
    output.enqueue(format_args!("closed"));
    assert_eq!(output.dropped(), 1);
    output.shutdown();
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
        .recv_timeout(std::time::Duration::from_millis(100))
        .is_ok();
    drop(locked_sender);
    worker.join().unwrap();
    assert!(completed, "slot submission waited on the sender lock");
    assert_eq!(output.dropped(), 1);
}

struct CaptureWriter(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for CaptureWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn stderr_shutdown_drains_accepted_lines_when_the_sink_is_writable() {
    let (output, receiver) = StderrOutput::channel();
    let bytes = Arc::new(Mutex::new(Vec::new()));
    let mut writer = CaptureWriter(Arc::clone(&bytes));
    let dropped = Arc::clone(&output.dropped);
    let queued = Arc::clone(&output.queued);
    let worker = std::thread::spawn(move || {
        run_stderr_writer(
            receiver,
            dropped,
            queued,
            || {},
            |line| write_line(&mut writer, line),
        );
    });

    output.enqueue(format_args!("first"));
    output.enqueue(format_args!("second"));
    output.shutdown();
    worker.join().unwrap();
    assert_eq!(&*bytes.lock().unwrap(), b"first\nsecond\n");
    assert_eq!(output.dropped(), 0);
}

#[test]
fn stderr_io_failure_is_counted_and_does_not_panic_the_worker() {
    let (output, receiver) = StderrOutput::channel();
    let dropped = Arc::clone(&output.dropped);
    let queued = Arc::clone(&output.queued);
    let worker = std::thread::spawn(move || {
        let mut writer = FullDisk;
        run_stderr_writer(
            receiver,
            dropped,
            queued,
            || {},
            |line| write_line(&mut writer, line),
        );
    });
    output.enqueue(format_args!("first"));
    output.enqueue(format_args!("second"));
    output.shutdown();
    worker.join().unwrap();
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
            self.release.recv().unwrap();
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn shutdown_does_not_wait_for_a_blocked_stderr_pipe() {
    let (output, receiver) = StderrOutput::channel();
    let dropped = Arc::clone(&output.dropped);
    let queued = Arc::clone(&output.queued);
    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let (worker_done_tx, worker_done_rx) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        let mut writer = BlockingWriter {
            entered: entered_tx,
            release: release_rx,
            blocked_once: false,
        };
        run_stderr_writer(
            receiver,
            dropped,
            queued,
            || {},
            |line| write_line(&mut writer, line),
        );
        let _ = worker_done_tx.send(());
    });
    output.enqueue(format_args!("blocked"));
    let writer_entered = entered_rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .is_ok();

    let (shutdown_done_tx, shutdown_done_rx) = std::sync::mpsc::channel();
    let shutdown = std::thread::spawn(move || {
        output.shutdown();
        let _ = shutdown_done_tx.send(());
    });
    let shutdown_returned = shutdown_done_rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .is_ok();

    let _ = release_tx.send(());
    let worker_returned = worker_done_rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .is_ok();
    if shutdown_returned {
        shutdown.join().unwrap();
    } else {
        drop(shutdown);
    }
    if worker_returned {
        worker.join().unwrap();
    } else {
        drop(worker);
    }

    assert!(
        writer_entered,
        "stderr writer did not reach the blocked sink"
    );
    assert!(
        shutdown_returned,
        "shutdown waited for the blocked stderr sink"
    );
    assert!(
        worker_returned,
        "stderr worker did not exit after the blocked sink was released"
    );
}

#[test]
fn reading_stderr_drop_count_does_not_start_the_writer() {
    assert!(std::sync::LazyLock::get(&STDERR_OUTPUT).is_none());
    assert_eq!(stderr_dropped(), 0);
    assert!(std::sync::LazyLock::get(&STDERR_OUTPUT).is_none());
}
