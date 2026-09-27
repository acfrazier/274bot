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
