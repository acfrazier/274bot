// Task 1: `SlotScript` want_run/is_up state machine. A compiled `Script`
// ticks only while the slot is Running and `want_run`; operator Pause and
// the not-`is_up` gate both land in Paused, but only operator Pause clears
// `want_run`. `Counter::tick` never sends, so the driver only records.

use api::interact::Driver;
use api::prot::Out;
use script::{RunState, Script, ScriptCtx, SlotScript};

/// Outbound writes a driver receives, as recorded by the stub.
#[derive(Debug, Clone, PartialEq, Eq)]
enum OutByte {
    Enc(i32),
    P1(i32),
    P2(i32),
    P4(i32),
    Jstr(String),
}

#[derive(Default)]
struct OutSink(Vec<OutByte>);

impl Out for OutSink {
    fn p1_enc(&mut self, opcode: i32) {
        self.0.push(OutByte::Enc(opcode));
    }
    fn p1(&mut self, value: i32) {
        self.0.push(OutByte::P1(value));
    }
    fn p2(&mut self, value: i32) {
        self.0.push(OutByte::P2(value));
    }
    fn p4(&mut self, value: i32) {
        self.0.push(OutByte::P4(value));
    }
    fn pjstr(&mut self, s: &str) {
        self.0.push(OutByte::Jstr(s.to_string()));
    }
}

/// Recording driver stub: accepts every call, never sends.
#[derive(Default)]
struct Rec {
    out: OutSink,
}

impl Driver for Rec {
    fn set_menu(&mut self, _slot: i32, _action: i32, _a: i32, _b: i32, _c: i32) {}
    fn do_action(&mut self, _slot: i32) -> bool {
        true
    }
    fn try_move(
        &mut self,
        _src_x: i32,
        _src_z: i32,
        _dx: i32,
        _dz: i32,
        _try_nearest: bool,
        _loc_width: i32,
        _loc_length: i32,
        _loc_angle: i32,
        _loc_shape: i32,
        _forceapproach: i32,
        _type: i32,
    ) -> bool {
        true
    }
    fn local_route(&self) -> Option<(i32, i32)> {
        None
    }
    fn build_base(&self) -> (i32, i32) {
        (0, 0)
    }
    fn loc_typecode(&self, _scene_x: i32, _scene_z: i32) -> Option<i32> {
        None
    }
    fn out(&mut self) -> &mut dyn Out {
        &mut self.out
    }
    fn login(&mut self, _username: &str, _password: &str, _reconnect: bool) -> bool {
        true
    }
}

/// A compiled script whose `tick` never sends; `n` counts dispatched ticks.
struct Counter {
    n: u32,
    name: String,
}

impl Script for Counter {
    fn name(&self) -> &str {
        &self.name
    }
    fn tick(&mut self, _ctx: &mut ScriptCtx<'_>) {
        self.n += 1;
    }
}

#[test]
fn idle_has_no_script_and_tick_is_noop() {
    let mut s = SlotScript::new();
    assert_eq!(s.state(), RunState::Idle);
    assert!(!s.want_run);
    assert!(s.last_error().is_none());
    let mut d = Rec::default();
    let mut ctx = ScriptCtx {
        driver: &mut d,
        tick: 0,
        here: None,
        walk: None,
        walk_with: None,
        inv: None,
        obj_names: None,
    };
    s.on_game_tick(&mut ctx);
    assert_eq!(s.state(), RunState::Idle);
}

#[test]
fn start_pause_resume_stop() {
    let mut s = SlotScript::new();
    s.start_compiled(Box::new(Counter {
        n: 0,
        name: "c".into(),
    }))
    .unwrap();
    assert_eq!(s.state(), RunState::Running);
    assert!(s.want_run);
    s.pause();
    assert_eq!(s.state(), RunState::Paused);
    assert!(!s.want_run);
    s.resume();
    assert_eq!(s.state(), RunState::Running);
    s.stop();
    assert_eq!(s.state(), RunState::Idle);
    assert!(!s.want_run);
}

#[test]
fn start_while_active_refuses() {
    let mut s = SlotScript::new();
    s.start_compiled(Box::new(Counter {
        n: 0,
        name: "a".into(),
    }))
    .unwrap();
    let err = s
        .start_compiled(Box::new(Counter {
            n: 0,
            name: "b".into(),
        }))
        .unwrap_err();
    assert!(err.contains("active") || err.contains("Stop"));
}

#[test]
fn not_is_up_skips_tick_keeps_instance_auto_resumes() {
    let mut s = SlotScript::new();
    s.start_compiled(Box::new(Counter {
        n: 0,
        name: "c".into(),
    }))
    .unwrap();
    s.on_is_up(false);
    assert_eq!(s.state(), RunState::Paused);
    assert!(
        s.want_run,
        "offline pause is the is_up gate, not operator Pause"
    );
    let mut d = Rec::default();
    let mut ctx = ScriptCtx {
        driver: &mut d,
        tick: 1,
        here: None,
        walk: None,
        walk_with: None,
        inv: None,
        obj_names: None,
    };
    s.on_game_tick(&mut ctx); // must not panic; skip
    s.on_is_up(true);
    assert_eq!(s.state(), RunState::Running);
}

#[test]
fn operator_pause_survives_login() {
    let mut s = SlotScript::new();
    s.start_compiled(Box::new(Counter {
        n: 0,
        name: "c".into(),
    }))
    .unwrap();
    s.pause();
    s.on_is_up(false);
    s.on_is_up(true);
    assert_eq!(s.state(), RunState::Paused);
    assert!(!s.want_run);
}

/// `on_game_tick` dispatches only while Running && want_run; Paused and
/// Idle skip the script, and `tick` never sends.
#[test]
fn game_tick_dispatches_only_while_running() {
    let ticks = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
    struct Probe {
        ticks: std::sync::Arc<std::sync::atomic::AtomicU32>,
    }
    impl Script for Probe {
        fn name(&self) -> &str {
            "probe"
        }
        fn tick(&mut self, _ctx: &mut ScriptCtx<'_>) {
            self.ticks
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }

    let mut s = SlotScript::new();
    s.start_compiled(Box::new(Probe {
        ticks: ticks.clone(),
    }))
    .unwrap();
    let mut d = Rec::default();

    s.on_game_tick(&mut ScriptCtx {
        driver: &mut d,
        tick: 1,
        here: None,
        walk: None,
        walk_with: None,
        inv: None,
        obj_names: None,
    });
    assert_eq!(ticks.load(std::sync::atomic::Ordering::Relaxed), 1);

    // Operator Pause: tick skipped, instance kept.
    s.pause();
    s.on_game_tick(&mut ScriptCtx {
        driver: &mut d,
        tick: 2,
        here: None,
        walk: None,
        walk_with: None,
        inv: None,
        obj_names: None,
    });
    assert_eq!(ticks.load(std::sync::atomic::Ordering::Relaxed), 1);

    // Resume: tick dispatched again.
    s.resume();
    s.on_game_tick(&mut ScriptCtx {
        driver: &mut d,
        tick: 3,
        here: None,
        walk: None,
        walk_with: None,
        inv: None,
        obj_names: None,
    });
    assert_eq!(ticks.load(std::sync::atomic::Ordering::Relaxed), 2);

    // Stop: instance gone, tick skipped.
    s.stop();
    s.on_game_tick(&mut ScriptCtx {
        driver: &mut d,
        tick: 4,
        here: None,
        walk: None,
        walk_with: None,
        inv: None,
        obj_names: None,
    });
    assert_eq!(ticks.load(std::sync::atomic::Ordering::Relaxed), 2);

    // The stub received no outbound writes.
    assert!(d.out.0.is_empty());
}

/// A panicking `tick` is caught: slot goes Error with a message, the
/// instance is dropped (no further ticks, no auto-resume), and a fresh
/// Start is allowed from Error.
#[test]
fn panicking_tick_sets_error_and_drops_instance() {
    struct Panic;
    impl Script for Panic {
        fn name(&self) -> &str {
            "panic"
        }
        fn tick(&mut self, _ctx: &mut ScriptCtx<'_>) {
            panic!("boom");
        }
    }

    let mut s = SlotScript::new();
    s.start_compiled(Box::new(Panic)).unwrap();
    let mut d = Rec::default();
    let mut ctx = ScriptCtx {
        driver: &mut d,
        tick: 1,
        here: None,
        walk: None,
        walk_with: None,
        inv: None,
        obj_names: None,
    };
    s.on_game_tick(&mut ctx);

    assert_eq!(s.state(), RunState::Error);
    let err = s.last_error().expect("panic must set last_error");
    assert!(err.contains("boom"), "unexpected error: {err}");
    assert!(!s.want_run, "a dead instance no longer wants to run");

    // Instance is gone: is_up cannot resurrect, and further ticks no-op.
    s.on_is_up(true);
    assert_eq!(s.state(), RunState::Error);
    s.on_game_tick(&mut ScriptCtx {
        driver: &mut d,
        tick: 2,
        here: None,
        walk: None,
        walk_with: None,
        inv: None,
        obj_names: None,
    });
    assert_eq!(s.state(), RunState::Error);

    // Start from Error is allowed and clears the error.
    assert!(s
        .start_compiled(Box::new(Counter {
            n: 0,
            name: "c".into()
        }))
        .is_ok());
    assert_eq!(s.state(), RunState::Running);
    assert!(s.last_error().is_none());
}

/// `on_stop` is a teardown hook: it runs exactly once, on `stop`, before
/// the instance is dropped.
#[test]
fn stop_runs_on_stop_hook() {
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
    struct Teardown {
        calls: std::sync::Arc<std::sync::atomic::AtomicU32>,
    }
    impl Script for Teardown {
        fn name(&self) -> &str {
            "teardown"
        }
        fn tick(&mut self, _ctx: &mut ScriptCtx<'_>) {}
        fn on_stop(&mut self) {
            self.calls
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }

    let mut s = SlotScript::new();
    s.start_compiled(Box::new(Teardown {
        calls: calls.clone(),
    }))
    .unwrap();
    s.stop();
    assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 1);
}
