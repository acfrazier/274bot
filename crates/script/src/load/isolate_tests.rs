use super::*;

#[test]
fn startup_non_yielding_module_has_bounded_join() {
    let isolate = LoadIsolate::spawn(
        "while (true) {}".into(),
        LoadShape::NativeTick,
        vec![],
    )
    .unwrap();
    let started = Instant::now();
    isolate.join();
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "startup evaluation outlived its owner: {:?}",
        started.elapsed()
    );
}

#[test]
fn validate_non_yielding_module_is_bounded() {
    const CHILD: &str = "SCRIPT_VALIDATE_CHILD";
    if std::env::var_os(CHILD).is_some() {
        let result = LoadIsolate::validate_source(
            "while (true) {}".into(),
            LoadShape::NativeTick,
            &[],
        );
        assert!(result.is_err(), "non-yielding source must be rejected");
        return;
    }

    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "load::isolate::tests::validate_non_yielding_module_is_bounded",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success(), "validation child failed: {status}");
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            panic!("validation child did not return before the deadline");
        }
        std::thread::yield_now();
    }
}

#[test]
fn pause_interrupts_a_newly_started_runaway_tick() {
    const CHILD: &str = "SCRIPT_PAUSE_CHILD";
    if std::env::var_os(CHILD).is_some() {
        let iso = LoadIsolate::spawn(
            "export function tick() { while (true) {} }".into(),
            LoadShape::NativeTick,
            vec![],
        )
        .unwrap();
        let ready_deadline = Instant::now() + Duration::from_secs(2);
        loop {
            match iso.poll_ready() {
                Ready::Pending => {
                    assert!(Instant::now() < ready_deadline, "isolate setup did not settle");
                    std::thread::yield_now();
                }
                Ready::Ready => break,
                Ready::Failed(error) => panic!("isolate setup failed: {error}"),
            }
        }
        iso.on_game_tick(1);
        iso.pause();
        let finish_deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let _ = iso.drain_logs();
            if iso.in_flight.lock().unwrap().is_none() {
                iso.join();
                return;
            }
            assert!(
                Instant::now() < finish_deadline,
                "Pause did not terminate the active tick"
            );
            std::thread::yield_now();
        }
    }

    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "load::isolate::tests::pause_interrupts_a_newly_started_runaway_tick",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success(), "pause child failed: {status}");
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            panic!("pause child did not return before the deadline");
        }
        std::thread::yield_now();
    }
}


#[test]
fn reset_rejects_a_tick_queued_with_the_previous_session_generation() {
    let iso = LoadIsolate::spawn(
        "export function tick(api) { globalThis.n = (globalThis.n || 0) + 1; }".into(),
        LoadShape::NativeTick,
        vec![],
    )
    .unwrap();
    iso.reset_session_work();
    // A sender captured this tick before reset but enqueued it late.
    assert!(iso.send(IsolateCmd::Tick {
        tick: 1,
        generation: 0,
        input_identity: 0,
    }));
    assert_eq!(
        iso.probe("globalThis.n || 0").unwrap(),
        serde_json::json!(0)
    );
    iso.on_game_tick(2);
    assert_eq!(iso.probe("globalThis.n").unwrap(), serde_json::json!(1));
    iso.join();
}

#[test]
fn shim_prelude_defines_globals_for_compat_fixture() {
    ensure_platform();
    let mut runtime = Runtime::new(RuntimeOptions::default()).unwrap();
    runtime.eval::<()>(crate::shim::PRELUDE).unwrap();
    let t: bool = runtime.eval("typeof defineBot === 'function'").unwrap();
    assert!(t);
    let t: bool = runtime.eval("typeof TaskBot === 'function'").unwrap();
    assert!(t);
    let t: bool = runtime.eval("typeof TreeBot === 'function'").unwrap();
    assert!(t);
    let t: bool = runtime.eval("typeof LoopingBot === 'function'").unwrap();
    assert!(t);
    let t: bool = runtime.eval("typeof __rs2b0t_host === 'object'").unwrap();
    assert!(t);
    // defineBot validates { name, create } instead of no-op'ing.
    let err: bool = runtime
        .eval("(() => { try { defineBot({}); return false; } catch { return true; } })()")
        .unwrap();
    assert!(err, "defineBot throws without a name/create pair");
}

#[test]
fn prelude_canvas_keyboard_queues_key_rows_and_blocks_mouse_layout() {
    ensure_platform();
    let mut runtime = Runtime::new(RuntimeOptions::default()).unwrap();
    runtime.eval::<()>(crate::shim::PRELUDE).unwrap();
    let report: serde_json::Value = runtime
        .eval(
            r#"
(() => {
const canvas = document.getElementById('canvas');
const other = document.getElementById('other');
canvas.dispatchEvent(new KeyboardEvent('keydown', {key: '2', code: '2'}));
canvas.dispatchEvent(new KeyboardEvent('keyup', {key: '2', code: '2'}));
let mouseCtor = true;
try { new MouseEvent('mousedown'); } catch { mouseCtor = false; }
let mouseDispatch = '';
try { canvas.dispatchEvent(new MouseEvent('mousedown')); }
catch (e) { mouseDispatch = String((e && e.message) || e); }
let layout = '';
try { canvas.getBoundingClientRect(); }
catch (e) { layout = String((e && e.message) || e); }
const frozen = new MouseEvent('mousedown', {clientX: 382.5, clientY: 251.5});
canvas.dispatchEvent(frozen);
let unknown = '';
try { canvas.dispatchEvent(new MouseEvent('mousemove')); }
catch (e) { unknown = String((e && e.message) || e); }
return {
    canvas: canvas !== null && typeof canvas === 'object',
    other: other,
    interact: globalThis.__rs2b0t_host.interact,
    mouseCtor,
    mouseDispatch,
    layout,
    frozen: {x: frozen.clientX, y: frozen.clientY, button: frozen.button},
    unknown,
};
})()
"#,
        )
        .unwrap();
    assert_eq!(report["canvas"], true);
    assert!(report["other"].is_null());
    assert_eq!(
        report["interact"],
        serde_json::json!([
            {"op": "key", "down": true, "key": "2", "code": "2"},
            {"op": "key", "down": false, "key": "2", "code": "2"},
            {"op": "mouse", "down": true, "x": 0, "y": 0, "button": 0},
            {"op": "mouse", "down": true, "x": 382.5, "y": 251.5, "button": 0},
        ])
    );
    assert_eq!(report["mouseCtor"], true);
    assert_eq!(report["mouseDispatch"], "");
    assert!(
        report["layout"]
            .as_str()
            .is_some_and(|s| s.contains("BLOCKED: missing getBoundingClientRect")),
        "{report:?}"
    );
    assert_eq!(report["frozen"]["x"], 382.5);
    assert_eq!(report["frozen"]["y"], 251.5);
    assert_eq!(report["frozen"]["button"], 0);
    assert!(
        report["unknown"]
            .as_str()
            .is_some_and(|s| s.contains("BLOCKED: missing mouse")),
        "{report:?}"
    );
}

#[test]
fn canvas_keyboard_producer_round_trips_fb_and_drops_stale_generation() {
    let iso = LoadIsolate::spawn(
        r#"
export default class T extends LoopingBot {
loop() {
    const canvas = document.getElementById('canvas');
    canvas.dispatchEvent(new KeyboardEvent('keydown', {key: '2', code: '2'}));
    canvas.dispatchEvent(new KeyboardEvent('keyup', {key: '2', code: '2'}));
}
}
"#
        .into(),
        LoadShape::CompatClass,
        vec![],
    )
    .unwrap();
    iso.on_game_tick(1);
    iso.probe("true").unwrap();
    let reqs = iso.drain_interacts();
    assert!(
        reqs.iter().any(|req| matches!(
            req,
            crate::shim::InteractReq::Key {
                down: true,
                key,
                ..
            } if key == "2"
        )),
        "{reqs:?}"
    );
    assert!(
        reqs.iter().any(|req| matches!(
            req,
            crate::shim::InteractReq::Key {
                down: false,
                key,
                ..
            } if key == "2"
        )),
        "{reqs:?}"
    );

    iso.on_game_tick(2);
    iso.probe("true").unwrap();
    iso.reset_session_work();
    let stale = iso.drain_interacts();
    assert!(
        stale
            .iter()
            .all(|req| !matches!(req, crate::shim::InteractReq::Key { .. })),
        "stale generation must not deliver keys: {stale:?}"
    );
    iso.join();
}

#[test]
fn canvas_mouse_producer_round_trips_fb_and_drops_stale_generation() {
    let iso = LoadIsolate::spawn(
        r#"
export default class T extends LoopingBot {
loop() {
    const canvas = document.getElementById('canvas');
    canvas.dispatchEvent(new MouseEvent('mousedown', {clientX: 382.5, clientY: 251.5}));
    canvas.dispatchEvent(new MouseEvent('mouseup', {clientX: 382.5, clientY: 251.5}));
    canvas.dispatchEvent(new KeyboardEvent('keydown', {key: '2', code: '2'}));
}
}
"#
        .into(),
        LoadShape::CompatClass,
        vec![],
    )
    .unwrap();
    iso.on_game_tick(1);
    iso.probe("true").unwrap();
    let reqs = iso.drain_interacts();
    assert!(
        reqs.iter().any(|req| matches!(
            req,
            crate::shim::InteractReq::Mouse {
                down: true,
                x,
                y,
                button: 0,
                ..
            } if (*x - 382.5).abs() < 1e-9 && (*y - 251.5).abs() < 1e-9
        )),
        "{reqs:?}"
    );
    assert!(
        reqs.iter()
            .any(|req| matches!(req, crate::shim::InteractReq::Mouse { down: false, .. })),
        "{reqs:?}"
    );
    assert!(
        reqs.iter().any(|req| matches!(
            req,
            crate::shim::InteractReq::Key { down: true, key, .. } if key == "2"
        )),
        "{reqs:?}"
    );

    iso.on_game_tick(2);
    iso.probe("true").unwrap();
    iso.reset_session_work();
    let stale = iso.drain_interacts();
    assert!(
        stale
            .iter()
            .all(|req| !matches!(req, crate::shim::InteractReq::Mouse { .. })),
        "stale generation must not deliver mouse: {stale:?}"
    );
    iso.join();
}

#[test]
fn canvas_mouse_production_stamps_input_identity() {
    let iso = LoadIsolate::spawn(
        r#"
export default class T extends LoopingBot {
loop() {
    const canvas = document.getElementById('canvas');
    canvas.dispatchEvent(new MouseEvent('mousedown', {clientX: 382.5, clientY: 251.5}));
}
}
"#
        .into(),
        LoadShape::CompatClass,
        vec![],
    )
    .unwrap();
    iso.on_game_tick_at(1, 42);
    iso.probe("true").unwrap();
    let reqs = iso.drain_interacts();
    assert!(
        reqs.iter().any(|req| matches!(
            req,
            crate::shim::InteractReq::Mouse {
                down: true,
                identity: 42,
                ..
            }
        )),
        "{reqs:?}"
    );
    iso.join();
}

#[test]
fn root_mouse_parked_up_keeps_revoked_gesture_identity() {
    let iso = LoadIsolate::spawn(
        r#"
export default class T extends LoopingBot {
async loop() {
    if (globalThis.__started) return;
    globalThis.__started = true;
    const canvas = document.getElementById('canvas');
    canvas.dispatchEvent(new MouseEvent('mousedown', {clientX: 100, clientY: 100}));
    await new Promise((resolve) => {
        globalThis.__release = resolve;
    });
    canvas.dispatchEvent(new MouseEvent('mouseup', {clientX: 100, clientY: 100}));
}
}
"#
        .into(),
        LoadShape::CompatClass,
        vec![],
    )
    .unwrap();

    iso.on_game_tick_at(1, 42);
    iso.probe("true").unwrap();
    let first = iso.drain_interacts();
    assert!(
        first.iter().any(|req| matches!(
            req,
            crate::shim::InteractReq::Mouse {
                down: true,
                identity: 42,
                ..
            }
        )),
        "{first:?}"
    );

    iso.pause();
    iso.resume();
    iso.probe("globalThis.__release(); true").unwrap();
    iso.on_game_tick_at(2, 43);
    iso.probe("true").unwrap();
    let second = iso.drain_interacts();
    assert!(
        second.iter().any(|req| matches!(
            req,
            crate::shim::InteractReq::Mouse {
                down: false,
                identity: 42,
                ..
            }
        )),
        "parked old up was restamped: {second:?}"
    );
    iso.join();
}

#[test]
fn execution_delay_ticks_mouse_up_keeps_down_identity() {
    let iso = LoadIsolate::spawn(
        r#"
import { Execution } from '../../api/execution/Execution.js';
export default class T extends LoopingBot {
async loop() {
    if (globalThis.__started) return;
    globalThis.__started = true;
    const canvas = document.getElementById('canvas');
    canvas.dispatchEvent(new MouseEvent('mousedown', {clientX: 100, clientY: 100}));
    await Execution.delayTicks(1);
    canvas.dispatchEvent(new MouseEvent('mouseup', {clientX: 100, clientY: 100}));
}
}
"#
        .into(),
        LoadShape::CompatClass,
        vec![],
    )
    .unwrap();

    iso.on_game_tick_at(1, 42);
    iso.probe("true").unwrap();
    let first = iso.drain_interacts();
    assert!(first.iter().any(|req| matches!(
        req,
        crate::shim::InteractReq::Mouse {
            down: true,
            identity: 42,
            ..
        }
    )));

    iso.pause();
    iso.resume();
    iso.on_game_tick_at(2, 43);
    iso.probe("true").unwrap();
    let second = iso.drain_interacts();
    assert!(
        second.iter().any(|req| matches!(
            req,
            crate::shim::InteractReq::Mouse {
                down: false,
                identity: 42,
                ..
            }
        )),
        "{second:?}"
    );
    iso.join();
}

#[test]
fn mouse_gesture_identity_overflow_fails_closed() {
    let mouse = |down| crate::shim::InteractReq::Mouse {
        down,
        x: 10.0,
        y: 10.0,
        button: 0,
        identity: 99,
    };
    let mut gestures = MouseGestureIdentities::default();
    let mut downs: Vec<_> = (0..=MAX_MOUSE_GESTURES).map(|_| mouse(true)).collect();
    stamp_mouse_gesture_identities(&mut downs, 7, &mut gestures);
    assert!(downs
        .iter()
        .all(|req| matches!(req, crate::shim::InteractReq::Mouse { identity: 7, .. })));
    assert!(gestures.pairs.is_empty());
    assert_eq!(gestures.unpairable, 33);

    let mut ups: Vec<_> = (0..=MAX_MOUSE_GESTURES).map(|_| mouse(false)).collect();
    stamp_mouse_gesture_identities(&mut ups, 7, &mut gestures);
    assert!(ups
        .iter()
        .all(|req| matches!(req, crate::shim::InteractReq::Mouse { identity: 0, .. })));
    assert_eq!(gestures.unpairable, 0);
}

#[test]
fn canvas_mouse_malformed_rows_keep_sibling_key_and_center() {
    let iso = LoadIsolate::spawn(
        r#"
export default class T extends LoopingBot {
loop() {
    const canvas = document.getElementById('canvas');
    const h = globalThis.__rs2b0t_host;
    h.interact = h.interact || [];
    h.interact.push({op:'mouse', down:true, x:{}, y:10, button:0});
    h.interact.push({op:'mouse', down:true, x:[1], y:10, button:0});
    h.interact.push({op:'mouse', down:true, x: Number.NaN, y:10, button:0});
    h.interact.push({op:'mouse', down:true, x:100, y:100, button: 4294967296});
    h.interact.push({op:'mouse', down:true, x:100, y:100, button: 1.5});
    h.interact.push({op:'mouse', down:true, x:100, y:100, button: null});
    h.interact.push([1, 2, 3]);
    canvas.dispatchEvent(new KeyboardEvent('keydown', {key: '2', code: '2'}));
    canvas.dispatchEvent(new MouseEvent('mousedown', {clientX: 382.5, clientY: 251.5}));
    canvas.dispatchEvent(new MouseEvent('mouseup', {clientX: 382.5, clientY: 251.5}));
}
}
"#
        .into(),
        LoadShape::CompatClass,
        vec![],
    )
    .unwrap();
    iso.on_game_tick(1);
    iso.probe("true").unwrap();
    let reqs = iso.drain_interacts();
    assert!(
        reqs.iter().any(|req| matches!(
            req,
            crate::shim::InteractReq::Key { down: true, key, .. } if key == "2"
        )),
        "sibling key dropped: {reqs:?}"
    );
    assert!(
        reqs.iter().any(|req| matches!(
            req,
            crate::shim::InteractReq::Mouse {
                down: true,
                x,
                y,
                button: 0,
                ..
            } if (*x - 382.5).abs() < 1e-9 && (*y - 251.5).abs() < 1e-9
        )),
        "valid center down dropped: {reqs:?}"
    );
    assert!(
        reqs.iter()
            .any(|req| matches!(req, crate::shim::InteractReq::Mouse { down: false, .. })),
        "valid center up dropped: {reqs:?}"
    );
    iso.join();
}

/// One posted tick whose only event-relevant fact is a prayer xp table:
/// an `xp` above the previous tick's value is one `SkillXp` event.
fn xp_input(tick: u64, inv_size: i32, xp: i32) -> Vec<u8> {
    let stats = [crate::isolate_fb::StatInput {
        index: 5,
        name: "prayer",
        xp,
        base: 2,
        effective: 2,
    }];
    let mut input = crate::isolate_fb::tests::empty_input(tick);
    input.inv_size = inv_size;
    input.stats = &stats;
    crate::isolate_fb::encode_snapshot(&input)
}

fn xp_event(skill: i32) -> crate::events::NativeEvent {
    crate::events::NativeEvent::SkillXp {
        skill,
        name: format!("skill{skill}"),
        xp: skill,
        delta: 1,
    }
}

fn staged_writes(iso: &LoadIsolate) -> serde_json::Value {
    iso.probe(
        "({ticks: globalThis.__ticks || 0, invSize: globalThis.__rs2b0t_host.snapshot.inv_size, \
             staged: typeof globalThis.__staged_writes, \
             pending: typeof globalThis.__rs2b0t_pending_native_event_batch})",
    )
    .unwrap()
}

/// Undelivered events in the compat runner's queue (0 when it is unset).
fn queue_length(iso: &LoadIsolate) -> Option<i64> {
    iso.probe(
            "(() => { const q = globalThis.__rs2b0t_pending_native_event_batch; return q ? q.length : 0; })()",
        )
        .unwrap()
        .as_i64()
}

/// A shape without an events API must not pay for events: the producer
/// never diffs a posted table (no diagnostic either), and the dispatcher
/// never builds or stages a batch. `__staged_writes` counts every write
/// the dispatcher would make to its staging global — the hand-off, plus
/// its own clear.
#[test]
fn native_tick_shape_builds_and_stages_no_native_events() {
    let iso = LoadIsolate::spawn(
        r#"
let staged = null;
Object.defineProperty(globalThis, '__rs2b0t_native_event_batch', {
    configurable: true,
    get() { return staged; },
    set(v) {
        globalThis.__staged_writes = (globalThis.__staged_writes || 0) + 1;
        staged = v;
    },
});
export function tick(api) { globalThis.__ticks = (globalThis.__ticks || 0) + 1; }
"#
        .into(),
        LoadShape::NativeTick,
        vec![],
    )
    .unwrap();
    // An invalid inv_size is a producer diagnostic for a consumer; this
    // shape has none, so not even the diff may run.
    iso.post_snapshot(xp_input(1, 28, 0));
    iso.on_game_tick(1);
    iso.post_snapshot(xp_input(2, 99, 1));
    iso.on_game_tick(2);
    let report = staged_writes(&iso);
    assert_eq!(report["ticks"], 2, "both ticks reached the module");
    assert_eq!(report["invSize"], 99, "the snapshot materialised");
    assert_eq!(
        report["staged"], "undefined",
        "no event batch may be built for a shape without an events API"
    );
    assert_eq!(
        report["pending"], "undefined",
        "no event queue may be created for a shape without an events API"
    );
    let logs = iso.drain_logs();
    assert!(
        !logs.iter().any(|line| line.contains("inventory events")),
        "an unconsumed shape must not diff posted tables: {logs:?}"
    );
    iso.join();
}

/// The compat queue is the only staging buffer between the dispatcher and
/// the runner's per-tick drain. It must stay bounded when that drain
/// stalls, and the bound must drop the oldest events — the drain delivers
/// in order, so the newest are the ones still worth running.
#[test]
fn compat_event_queue_is_capped_and_keeps_the_newest_events() {
    ensure_platform();
    let mut runtime = Runtime::new(RuntimeOptions::default()).unwrap();
    let first: Vec<_> = (0..300).map(xp_event).collect();
    dispatch_native_events(&mut runtime, &first).unwrap();
    let staged: Option<serde_json::Value> = runtime
        .eval("globalThis.__rs2b0t_native_event_batch ?? null")
        .unwrap();
    assert!(staged.is_none(), "the append consumes the staged batch");
    let capped: i64 = runtime
        .eval("globalThis.__rs2b0t_pending_native_event_batch.length")
        .unwrap();
    assert_eq!(capped, 256, "one oversized batch is capped at 256");

    let second: Vec<_> = (300..310).map(xp_event).collect();
    dispatch_native_events(&mut runtime, &second).unwrap();
    let report: Vec<i64> = runtime
            .eval("(() => { const q = globalThis.__rs2b0t_pending_native_event_batch; return [q.length, q[0].payload.skill, q[q.length - 1].payload.skill]; })()")
            .unwrap();
    assert_eq!(report[0], 256, "a second batch stays capped: {report:?}");
    assert_eq!(
        report[1], 54,
        "the oldest events are trimmed first: {report:?}"
    );
    assert_eq!(report[2], 309, "the newest event is retained: {report:?}");
}

/// ResetSession drops the batches the previous connection staged: the
/// compat runner must not replay them, and the next session must not
/// start with the trim window already full.
#[test]
fn reset_session_clears_the_pending_native_event_queue() {
    let iso = LoadIsolate::spawn(
        r#"
export default class T extends LoopingBot {
loop() {
    // Stall the runner's drain so the queue keeps what Rust staged.
    globalThis.__rs2b0t_flush_native_events = () => {};
}
}
"#
        .into(),
        LoadShape::CompatClass,
        vec![],
    )
    .unwrap();
    iso.post_snapshot(xp_input(1, 28, 0));
    iso.on_game_tick(1);
    assert_eq!(
        queue_length(&iso),
        Some(0),
        "the first table only seeds the producer"
    );
    iso.post_snapshot(xp_input(2, 28, 1));
    iso.on_game_tick(2);
    assert_eq!(
        queue_length(&iso),
        Some(1),
        "the xp diff reached the compat queue"
    );
    iso.reset_session_work();
    let cleared = iso
        .probe("globalThis.__rs2b0t_pending_native_event_batch === null")
        .unwrap();
    assert_eq!(
        cleared,
        serde_json::json!(true),
        "ResetSession must clear the undelivered queue"
    );
    iso.join();
}

fn wait_ready(iso: &LoadIsolate) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match iso.poll_ready() {
            Ready::Ready => return,
            Ready::Failed(e) => panic!("setup failed: {e}"),
            Ready::Pending if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(5));
            }
            Ready::Pending => panic!("setup timed out"),
        }
    }
}

#[test]
fn spawn_returns_before_setup_and_poll_ready_becomes_ready() {
    let t0 = Instant::now();
    let iso = LoadIsolate::spawn(
        "export function tick(api) {}".into(),
        LoadShape::NativeTick,
        vec![],
    )
    .unwrap();
    assert!(
        t0.elapsed() < Duration::from_millis(500),
        "spawn must not wait on V8: {:?}",
        t0.elapsed()
    );
    wait_ready(&iso);
    assert_eq!(iso.poll_ready(), Ready::Ready);
    iso.join();
}

#[test]
fn poll_ready_surfaces_a_wire_failure() {
    let iso = LoadIsolate::spawn(
        "not valid javascript!!!!".into(),
        LoadShape::NativeTick,
        vec![],
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let err = loop {
        match iso.poll_ready() {
            Ready::Failed(e) => break e,
            Ready::Ready => panic!("invalid source must not become Ready"),
            Ready::Pending if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(5));
            }
            Ready::Pending => panic!("setup neither failed nor finished"),
        }
    };
    assert!(!err.is_empty(), "{err}");
    iso.join();
}

#[test]
fn join_detached_returns_immediately_and_delivers_onstop_logs() {
    let iso = LoadIsolate::spawn(
        "export default class T extends LoopingBot {
            loop() {}
            onStop() { this.log('stopped-ok'); }
        }"
        .into(),
        LoadShape::CompatClass,
        vec![],
    )
    .unwrap();
    wait_ready(&iso);
    iso.set_onstop_timeout_for_test(Duration::from_secs(1));
    iso.on_game_tick(1);
    let _ = iso.probe("1");
    let (tx, rx) = mpsc::channel();
    let t0 = Instant::now();
    iso.join_detached(tx);
    assert!(
        t0.elapsed() < Duration::from_millis(200),
        "join_detached blocked: {:?}",
        t0.elapsed()
    );
    let logs = rx
        .recv_timeout(Duration::from_secs(5))
        .expect("reaper logs");
    assert!(
        logs.iter().any(|l| l.contains("stopped-ok")),
        "onStop log missing: {logs:?}"
    );
    let _ = abandoned_isolate_count();
}

fn machine_tick(iso: &LoadIsolate, n: u64) {
    iso.on_game_tick(n);
    let _ = iso.probe("true");
}

fn if_button(id: i32) -> crate::shim::InteractReq {
    crate::shim::InteractReq::IfButton { component_id: id }
}

fn spawn_machine_card(body: &str) -> LoadIsolate {
    let src = format!(
        "import {{ runMachine, queue }} from '../../shim/_kernel.js';\n\
             import {{ Execution }} from '../../api/execution/Execution.js';\n\
             export default class T extends LoopingBot {{\n\
             async loop() {{\n\
             if (globalThis.__did) return;\n\
             globalThis.__did = true;\n\
             {body}\n\
             }}\n}}\n"
    );
    LoadIsolate::spawn(src, LoadShape::CompatClass, vec![]).unwrap()
}

#[test]
fn machine_ops_join_the_batch_in_order_and_the_await_settles_once() {
    let iso = spawn_machine_card(
        "globalThis.__settles = 0;
             queue({ op: 'if-button', component_id: 1 });
             const run = runMachine('probe', { button: 10, steps: 2 });
             queue({ op: 'if-button', component_id: 2 });
             const out = await run;
             globalThis.__settles += 1;
             globalThis.__out = out;
             globalThis.__at = globalThis.__rs2b0t_host.tick;",
    );
    machine_tick(&iso, 1);
    assert_eq!(
        iso.drain_interacts(),
        vec![if_button(1), if_button(10), if_button(2)],
        "begin ops sit where the caller started the machine"
    );
    machine_tick(&iso, 2);
    assert_eq!(iso.drain_interacts(), vec![if_button(11)]);
    machine_tick(&iso, 3);
    assert_eq!(iso.drain_interacts(), vec![if_button(12)]);
    assert_eq!(iso.probe("globalThis.__settles").unwrap(), 0);
    for n in 4..=6 {
        machine_tick(&iso, n);
    }
    assert_eq!(iso.probe("globalThis.__settles").unwrap(), 1);
    assert_eq!(
        iso.probe("globalThis.__out").unwrap(),
        serde_json::json!({ "kind": "done", "value": 2 })
    );
    assert_eq!(
        iso.probe("globalThis.__at").unwrap(),
        4,
        "the completing step settles in the same tick's pump"
    );
    assert!(
        iso.drain_interacts().is_empty(),
        "a finished row emits nothing"
    );
    iso.join();
}

#[test]
fn reset_session_aborts_a_running_machine_and_settles_its_await() {
    let iso = spawn_machine_card(
        "globalThis.__out = await runMachine('probe', { button: 10, steps: 5 });",
    );
    machine_tick(&iso, 1);
    assert_eq!(iso.drain_interacts(), vec![if_button(10)]);
    iso.reset_session_work();
    machine_tick(&iso, 2);
    assert_eq!(
        iso.probe("globalThis.__out").unwrap(),
        serde_json::json!({ "kind": "aborted", "reason": "reset" })
    );
    machine_tick(&iso, 3);
    assert!(
        iso.drain_interacts().is_empty(),
        "an aborted row never steps"
    );
    iso.join();
}

#[test]
fn concurrent_machines_settle_independently() {
    let iso = spawn_machine_card(
        "const stamp = (p) => p.then((out) => ({ out, at: globalThis.__rs2b0t_host.tick }));
             globalThis.__both = await Promise.all([
                 stamp(runMachine('probe', { button: 100, steps: 1 })),
                 stamp(runMachine('probe', { button: 200, steps: 3 })),
             ]);",
    );
    for n in 1..=6 {
        machine_tick(&iso, n);
    }
    assert_eq!(
        iso.probe("globalThis.__both").unwrap(),
        serde_json::json!([
            { "out": { "kind": "done", "value": 1 }, "at": 3 },
            { "out": { "kind": "done", "value": 3 }, "at": 5 },
        ])
    );
    iso.join();
}

#[test]
fn refused_and_immediate_starts_resolve_without_a_wait() {
    let iso = spawn_machine_card(
        "globalThis.__outs = [
                 await runMachine('probe', { button: 1, steps: 1, refuse: true }),
                 await runMachine('probe', { button: 2, steps: 0 }),
                 await runMachine('nowhere', {}),
             ];",
    );
    machine_tick(&iso, 1);
    let outs = iso.probe("globalThis.__outs").unwrap();
    assert_eq!(
        outs[0],
        serde_json::json!({ "kind": "refused", "reason": "probe refused" })
    );
    assert_eq!(outs[1], serde_json::json!({ "kind": "done", "value": 0 }));
    assert_eq!(outs[2]["kind"], "refused");
    assert_eq!(iso.drain_interacts(), vec![if_button(2)]);
    iso.join();
}

// Frozen timing: a callback sees the tick it runs in, `delayTicks(1)`
// from a tick-2 callback resumes in tick 3's pump, and the row resumes
// (and here ends, settling the await) in tick 3 as well.
#[test]
fn a_machine_calls_sync_and_async_script_callbacks_in_order() {
    let iso = spawn_machine_card(
        "globalThis.__calls = [];
             const at = () => globalThis.__rs2b0t_host.tick;
             const hooks = {
                 tag: 'H',
                 sync(n, s) {
                     globalThis.__calls.push(['sync', n, s, this.tag, at()]);
                     queue({ op: 'if-button', component_id: 900 });
                     return n + 1;
                 },
                 async later(v) {
                     globalThis.__calls.push(['later', v, at()]);
                     await Execution.delayTicks(1);
                     globalThis.__calls.push(['later-resumed', v, at()]);
                     return v * 10;
                 },
                 mark() {
                     globalThis.__calls.push(['mark', at()]);
                     return 'm';
                 },
             };
             globalThis.__out = await runMachine('hooked', { emit: 500 }, hooks);
             globalThis.__at = at();",
    );
    machine_tick(&iso, 1);
    assert!(iso.drain_interacts().is_empty());
    machine_tick(&iso, 2);
    assert_eq!(
        iso.drain_interacts(),
        vec![if_button(900), if_button(501)],
        "a step's op follows the row its callback queued"
    );
    assert_eq!(
        iso.probe("globalThis.__calls").unwrap(),
        serde_json::json!([["sync", 1, "a", "H", 2], ["later", 2, 2]]),
        "callbacks see the tick they run in; delayTicks(1) has not elapsed"
    );
    assert_eq!(
        iso.probe("globalThis.__out ?? null").unwrap(),
        serde_json::Value::Null
    );
    machine_tick(&iso, 3);
    assert_eq!(
        iso.probe("globalThis.__calls").unwrap(),
        serde_json::json!([
            ["sync", 1, "a", "H", 2],
            ["later", 2, 2],
            ["later-resumed", 2, 3],
            ["mark", 3],
        ]),
        "the row resumes in the tick the wait settled"
    );
    assert_eq!(iso.drain_interacts(), vec![if_button(502), if_button(503)]);
    assert_eq!(
        iso.probe("globalThis.__out").unwrap(),
        serde_json::json!({ "kind": "done", "value": [2, 20, "m"] })
    );
    assert_eq!(iso.probe("globalThis.__at").unwrap(), 3);
    iso.join();
}

// N1(a): a callback that only awaits microtasks answers in the call's
// own checkpoint, so all of a burst's awaited calls run in one tick
// (up to the per-row budget), like a frozen `for` with `await`s.
#[test]
fn microtask_only_awaits_answer_in_the_same_step() {
    const B: usize = crate::machine::CALLS_PER_TICK;
    let iso = spawn_machine_card(&format!(
        "globalThis.__ticks = [];
             globalThis.__out = await runMachine('burst', {{ calls: {} }}, {{
                 async each(i) {{
                     await null;
                     await Promise.resolve();
                     globalThis.__ticks.push(globalThis.__rs2b0t_host.tick);
                     return i;
                 }},
             }});
             globalThis.__at = globalThis.__rs2b0t_host.tick;",
        B + 8
    ));
    machine_tick(&iso, 1);
    machine_tick(&iso, 2);
    let ticks: Vec<u64> = serde_json::from_value(iso.probe("globalThis.__ticks").unwrap()).unwrap();
    assert_eq!(ticks.len(), B, "the budget bounds tick 2");
    assert!(ticks.iter().all(|&t| t == 2), "{ticks:?}");
    machine_tick(&iso, 3);
    let ticks: Vec<u64> = serde_json::from_value(iso.probe("globalThis.__ticks").unwrap()).unwrap();
    assert_eq!(ticks.len(), B + 8);
    assert!(ticks[B..].iter().all(|&t| t == 3), "{ticks:?}");
    assert_eq!(iso.probe("globalThis.__out.value.length").unwrap(), B + 8);
    assert_eq!(iso.probe("globalThis.__at").unwrap(), 3);
    iso.join();
}

// N1(b): a callback promise the pump settles resumes its row after the
// pump in the same tick, and the budget spans both passes: 6 calls
// before the wait, the rest of the budget after it in tick 2, the last
// 8 in tick 3.
#[test]
fn a_pump_settled_callback_resumes_its_row_in_the_same_tick() {
    const B: usize = crate::machine::CALLS_PER_TICK;
    let iso = spawn_machine_card(&format!(
        "globalThis.__ticks = [];
             const each = (i) => {{
                 globalThis.__ticks.push(globalThis.__rs2b0t_host.tick);
                 return i === 5 ? Execution.delayTicks(0).then(() => i) : i;
             }};
             globalThis.__out = await runMachine('burst', {{ calls: {} }}, {{ each }});",
        B + 8
    ));
    machine_tick(&iso, 1);
    machine_tick(&iso, 2);
    let ticks: Vec<u64> = serde_json::from_value(iso.probe("globalThis.__ticks").unwrap()).unwrap();
    assert_eq!(
        ticks,
        vec![2; B],
        "6 calls, the pump, then the rest of the budget"
    );
    machine_tick(&iso, 3);
    let ticks: Vec<u64> = serde_json::from_value(iso.probe("globalThis.__ticks").unwrap()).unwrap();
    assert_eq!(ticks[B..], [3; 8]);
    assert_eq!(
        iso.probe("globalThis.__out.value[5]").unwrap(),
        5,
        "the resumed row read the settled value"
    );
    iso.join();
}

// N2: presence is read once, at start; null/undefined are absent.
#[test]
fn hook_presence_is_read_once_at_start() {
    let iso = spawn_machine_card(
        "globalThis.__reads = 0;
             const hooks = {
                 a() {},
                 b: null,
                 get c() { globalThis.__reads += 1; return () => 1; },
             };
             globalThis.__out = await runMachine('present', {}, hooks);",
    );
    machine_tick(&iso, 1);
    assert_eq!(
        iso.probe("globalThis.__out").unwrap(),
        serde_json::json!({ "kind": "done", "value": [true, false, true, false] })
    );
    assert_eq!(iso.probe("globalThis.__reads").unwrap(), 1);
    iso.join();
}

// N3: rows script code queued between ticks (a probe here, the
// recovery anchor in production) stay ahead of the next step's ops.
#[test]
fn step_ops_follow_rows_queued_before_the_tick() {
    let iso = spawn_machine_card(
        "globalThis.__out = await runMachine('probe', { button: 10, steps: 1 });",
    );
    machine_tick(&iso, 1);
    assert_eq!(iso.drain_interacts(), vec![if_button(10)]);
    let _ =
        iso.probe("globalThis.__rs2b0t_host.interact.push({ op: 'if-button', component_id: 7 })");
    machine_tick(&iso, 2);
    assert_eq!(iso.drain_interacts(), vec![if_button(7), if_button(11)]);
    iso.join();
}

// N5: a row whose callback starts a newer row of its exclusive family
// stops driving at once: no further op, and it settles `superseded`.
#[test]
fn a_row_that_supersedes_itself_mid_step_stops_at_once() {
    let iso = spawn_machine_card(
        "globalThis.__out = await runMachine('solo-hooked', {}, {
                 again() {
                     globalThis.__next = runMachine('solo-hooked', { quiet: true }, {
                         again: () => 0,
                     });
                     return 1;
                 },
             });",
    );
    machine_tick(&iso, 1);
    machine_tick(&iso, 2);
    assert!(
        iso.drain_interacts().is_empty(),
        "the superseded row emits nothing after its callback"
    );
    machine_tick(&iso, 3);
    assert_eq!(
        iso.probe("globalThis.__out").unwrap(),
        serde_json::json!({ "kind": "aborted", "reason": "superseded" })
    );
    iso.join();
}

#[test]
fn a_throwing_or_rejecting_callback_fails_the_machine() {
    let iso = spawn_machine_card(
        "const boom = new Error('boom');
             const late = { code: 7 };
             const settle = (p) => p.then(
                 (out) => ({ ok: out }),
                 (e) => ({ err: String(e && e.message), same: e === boom || e === late }),
             );
             globalThis.__outs = await Promise.all([
                 settle(runMachine('hooked', {}, { sync() { throw boom; } })),
                 settle(runMachine('hooked', {}, {
                     sync: (n) => n,
                     async later() { throw late; },
                 })),
                 settle(runMachine('hooked', {}, { sync: 5 })),
                 settle(runMachine('hooked', {})),
             ]);",
    );
    machine_tick(&iso, 1);
    machine_tick(&iso, 2);
    let outs = iso.probe("globalThis.__outs").unwrap();
    assert_eq!(
        outs[0],
        serde_json::json!({ "err": "boom", "same": true }),
        "the await rejects with the very value the callback threw"
    );
    assert_eq!(
        outs[1],
        serde_json::json!({ "err": "undefined", "same": true }),
        "and with the very value an async callback rejected with"
    );
    assert_eq!(
        outs[2],
        serde_json::json!({ "err": "sync is not a function", "same": false })
    );
    assert!(
        outs[3]["err"]
            .as_str()
            .is_some_and(|e| e.contains("reading 'sync'")),
        "a missing hooks object throws at start: {outs:?}"
    );
    let logs = iso.drain_logs();
    assert!(
        logs.iter().all(|l| !l.contains("Uncaught")),
        "an observed rejection is not unhandled: {logs:?}"
    );
    iso.join();
}

#[test]
fn reset_and_stop_release_a_machine_waiting_on_a_callback_promise() {
    let src = "globalThis.__out = await runMachine('hooked', {}, {
                 sync: (n) => n,
                 later: () => new Promise(() => {}),
             });";
    let iso = spawn_machine_card(src);
    machine_tick(&iso, 1);
    machine_tick(&iso, 2);
    iso.reset_session_work();
    machine_tick(&iso, 3);
    assert_eq!(
        iso.probe("globalThis.__out").unwrap(),
        serde_json::json!({ "kind": "aborted", "reason": "reset" })
    );
    iso.join();

    // Stop with the callback and its promise still held: the rows drop
    // before the isolate (a V8 handle outliving it would abort here).
    let iso = spawn_machine_card(src);
    machine_tick(&iso, 1);
    machine_tick(&iso, 2);
    iso.join();
}

#[test]
fn join_stops_a_machine_before_a_later_spinning_callback() {
    // The first callback spins until join's terminate ends it; the
    // machine keeps going on a throw, so without the claim re-check its
    // next callback would spin on past join.
    let iso = spawn_machine_card(
        "globalThis.__out = await runMachine('burst', { calls: 3, keep: true }, {
                 each() { for (;;) {} },
             });",
    );
    let proof = iso.teardown_proof();
    machine_tick(&iso, 1);
    iso.on_game_tick(2);
    std::thread::sleep(Duration::from_millis(200));
    let t0 = Instant::now();
    iso.join();
    assert!(
        proof.finished() && t0.elapsed() < JOIN_TIMEOUT,
        "join abandoned the isolate after {:?}",
        t0.elapsed()
    );
}

#[test]
fn join_runs_no_paint_after_a_machine_callback_absorbed_its_terminate() {
    // The callback absorbs join's terminate; the tick then cancels the
    // terminate. An ungated onPaint after that would spin past join.
    let src = "import { runMachine } from '../../shim/_kernel.js';\n\
             export default class T extends LoopingBot {\n\
             onPaint() { if (globalThis.__spin) for (;;) {} }\n\
             async loop() {\n\
             if (globalThis.__did) return;\n\
             globalThis.__did = true;\n\
             await runMachine('burst', { calls: 3, keep: true }, {\n\
                 each() { globalThis.__spin = true; for (;;) {} },\n\
             });\n\
             }\n}\n";
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    let proof = iso.teardown_proof();
    machine_tick(&iso, 1);
    iso.on_game_tick(2);
    std::thread::sleep(Duration::from_millis(200));
    let t0 = Instant::now();
    iso.join();
    assert!(
        proof.finished() && t0.elapsed() < JOIN_TIMEOUT,
        "join abandoned the isolate after {:?}",
        t0.elapsed()
    );
}

#[test]
fn machines_step_normally_again_after_a_watchdog_fire() {
    // A callback spins on tick 2; dispatching tick 3 fires the
    // watchdog. Once that tick's cancel clears the mark, a machine
    // started afterwards must step every tick and complete.
    let iso = spawn_machine_card(
        "globalThis.__first = await runMachine('burst', { calls: 1, keep: true }, {
                 each() { for (;;) {} },
             });
             globalThis.__startedAt = globalThis.__rs2b0t_host.tick;
             globalThis.__out = await runMachine('probe', { button: 10, steps: 1 });
             globalThis.__at = globalThis.__rs2b0t_host.tick;",
    );
    machine_tick(&iso, 1);
    iso.on_game_tick(2);
    std::thread::sleep(Duration::from_millis(200));
    machine_tick(&iso, 3);
    assert_eq!(
        iso.probe("globalThis.__first").unwrap(),
        serde_json::json!({ "kind": "aborted", "reason": "terminated" })
    );
    let (started, observed): (u64, u64) = serde_json::from_value(
        iso.probe("[globalThis.__startedAt, globalThis.__rs2b0t_host.tick]")
            .unwrap(),
    )
    .unwrap();
    let first_batch = iso.drain_interacts();
    match first_batch.as_slice() {
        [begin] if *begin == if_button(10) => {
            assert_eq!(observed, started, "the probe observed the start tick");
            machine_tick(&iso, started + 1);
            assert_eq!(
                iso.drain_interacts(),
                vec![if_button(11)],
                "the first tick after the start steps the machine"
            );
        }
        [begin, step] if *begin == if_button(10) && *step == if_button(11) => {
            assert_eq!(
                observed,
                started + 1,
                "the second interaction must come from an extra observed tick"
            );
        }
        _ => panic!("the recovered machine must start once and step at most once: {first_batch:?}"),
    }
    machine_tick(&iso, started + 2);
    assert_eq!(
        iso.probe("globalThis.__out").unwrap(),
        serde_json::json!({ "kind": "done", "value": 1 })
    );
    assert_eq!(iso.probe("globalThis.__at").unwrap(), started + 2);
    iso.join();
}

#[test]
fn a_watchdog_terminate_in_a_callbacks_microtasks_ends_the_row() {
    // The continuation, not the callback, spins; the watchdog of the
    // next dispatch terminates it inside the call's checkpoint. The row
    // must end there, not call on as if the callback had returned.
    let iso = spawn_machine_card(
        "globalThis.__calls = 0;
             globalThis.__out = await runMachine('burst', { calls: 3, keep: true }, {
                 each(i) {
                     globalThis.__calls++;
                     if (i === 0) Promise.resolve().then(() => { for (;;) {} });
                     return i;
                 },
             });",
    );
    machine_tick(&iso, 1);
    iso.on_game_tick(2);
    std::thread::sleep(Duration::from_millis(200));
    machine_tick(&iso, 3);
    machine_tick(&iso, 4);
    assert_eq!(iso.probe("globalThis.__calls").unwrap(), 1);
    assert_eq!(
        iso.probe("globalThis.__out").unwrap(),
        serde_json::json!({ "kind": "aborted", "reason": "terminated" })
    );
    iso.join();
}

#[test]
fn a_callback_may_start_another_machine_mid_step() {
    let iso = spawn_machine_card(
        "globalThis.__out = await runMachine('hooked', { emit: 500 }, {
                 sync(n) {
                     globalThis.__inner = runMachine('probe', { button: 700, steps: 1 });
                     return n;
                 },
                 later: (v) => v,
                 mark: () => null,
             });
             globalThis.__innerOut = await globalThis.__inner;",
    );
    machine_tick(&iso, 1);
    machine_tick(&iso, 2);
    assert_eq!(
        iso.drain_interacts(),
        vec![
            if_button(700),
            if_button(501),
            if_button(502),
            if_button(503)
        ],
        "the inner begin op lands where its callback ran"
    );
    for n in 3..=5 {
        machine_tick(&iso, n);
    }
    assert_eq!(
        iso.drain_interacts(),
        vec![if_button(701)],
        "the inner row is first stepped on the next tick"
    );
    assert_eq!(iso.probe("globalThis.__out.kind").unwrap(), "done");
    assert_eq!(
        iso.probe("globalThis.__innerOut").unwrap(),
        serde_json::json!({ "kind": "done", "value": 1 })
    );
    iso.join();
}
