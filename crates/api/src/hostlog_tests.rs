use super::*;
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
