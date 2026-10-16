use super::*;

#[test]
fn missing_bootstrap_error_names_the_entrypoint() {
    let message = LoadIsolateError::PlatformNotBootstrapped.to_string();
    assert!(
        message.contains("script::bootstrap_v8_platform()"),
        "{message}"
    );
}

fn run_exact_child(test: &str, env: &str) {
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", test, "--nocapture"])
        .env(env, "1")
        .spawn()
        .unwrap();
    // Harness liveness bound only: the product deadline oracle is the inner
    // per-attempt `< 3s` assertion inside the child. The heaviest child does
    // three such attempts (~9s of owned work) plus test-binary spawn and V8
    // bootstrap, which stretches under machine load, so the outer bound stays
    // generous while still killing (never hanging on) a truly stuck child.
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success(), "{test} child failed: {status}");
            return;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            panic!("{test} child did not return before the deadline");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn startup_non_yielding_module_has_bounded_join() {
    const CHILD: &str = "SCRIPT_STARTUP_CHILD";
    if std::env::var_os(CHILD).is_some() {
        let baseline = live_runtime_count();
        for attempt in 1..=3 {
            let isolate =
                LoadIsolate::spawn("while (true) {}".into(), LoadShape::NativeTick, vec![])
                    .unwrap();
            let started = Instant::now();
            isolate.join();
            assert!(
                started.elapsed() < Duration::from_secs(3),
                "attempt {attempt}: startup evaluation outlived its owner: {:?}",
                started.elapsed()
            );
            assert_eq!(
                live_runtime_count(),
                baseline,
                "attempt {attempt}: joined startup left a live Runtime"
            );
        }
        return;
    }
    run_exact_child(
        "load::isolate::tests::startup_non_yielding_module_has_bounded_join",
        CHILD,
    );
}

#[test]
fn validate_non_yielding_module_is_bounded() {
    const CHILD: &str = "SCRIPT_VALIDATE_CHILD";
    if std::env::var_os(CHILD).is_some() {
        let baseline = live_runtime_count();
        for attempt in 1..=3 {
            let started = Instant::now();
            let card = super::super::library::PreparedCard {
                card: super::super::library::JsCard {
                    name: "runaway".into(),
                    path: "runaway.ts".into(),
                    shape: LoadShape::NativeTick,
                    origin: "while (true) {}".into(),
                    js: "while (true) {}".into(),
                    kind: crate::rs2b0t_registry::ScriptKind::NativeTick,
                    source: crate::rs2b0t_registry::ScriptSource::File,
                    sha256: String::new(),
                    description: String::new(),
                    category: String::new(),
                    tags: Vec::new(),
                    settings_schema: Vec::new(),
                    unloadable: None,
                    api_family: super::super::ApiFamily::Unversioned,
                },
                siblings: Vec::new(),
                fingerprint: String::new(),
            };
            let batch = LoadIsolate::validate_prepared_batch(vec![card]).unwrap();
            let result = loop {
                match batch.try_recv() {
                    Ok(rows) => break rows.into_iter().next().unwrap().1,
                    Err(mpsc::TryRecvError::Empty) => {
                        assert!(
                            started.elapsed() < Duration::from_secs(3),
                            "validation batch did not settle"
                        );
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(mpsc::TryRecvError::Disconnected) => {
                        panic!("validation coordinator disconnected")
                    }
                }
            };
            assert!(
                result.is_err(),
                "attempt {attempt}: non-yielding source must be rejected"
            );
            assert!(
                started.elapsed() < Duration::from_secs(3),
                "attempt {attempt}: validation exceeded its owned deadline: {:?}",
                started.elapsed()
            );
            assert_eq!(
                live_runtime_count(),
                baseline,
                "attempt {attempt}: validation left a live Runtime"
            );
        }
        return;
    }
    run_exact_child(
        "load::isolate::tests::validate_non_yielding_module_is_bounded",
        CHILD,
    );
}

#[test]
fn pause_interrupts_a_newly_started_runaway_tick() {
    const CHILD: &str = "SCRIPT_PAUSE_CHILD";
    if std::env::var_os(CHILD).is_some() {
        let iso = LoadIsolate::spawn(
            "export function tick(api) {
                globalThis.__runs = (globalThis.__runs || 0) + 1;
                globalThis.__lastTick = api.tick;
                if (api.tick === 1) while (true) {}
            }"
            .into(),
            LoadShape::NativeTick,
            vec![],
        )
        .unwrap();
        let ready_deadline = Instant::now() + Duration::from_secs(2);
        loop {
            match iso.poll_ready() {
                Ready::Pending => {
                    assert!(
                        Instant::now() < ready_deadline,
                        "isolate setup did not settle"
                    );
                    std::thread::sleep(Duration::from_millis(1));
                }
                Ready::Ready => break,
                Ready::Failed(error) => panic!("isolate setup failed: {error}"),
            }
        }
        iso.on_game_tick(1);
        let entered_deadline = Instant::now() + Duration::from_secs(2);
        while !iso.execution_active() {
            assert!(
                Instant::now() < entered_deadline,
                "runaway tick never became interruptible"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        iso.pause();
        let finish_deadline = Instant::now() + Duration::from_secs(2);
        let mut logs = Vec::new();
        loop {
            logs.extend(iso.drain_logs());
            if iso.in_flight.lock().unwrap().is_none() {
                break;
            }
            assert!(
                Instant::now() < finish_deadline,
                "Pause did not terminate the active tick"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(
            logs.iter().all(|line| !line.contains("Unknown error")),
            "operator cancellation is not a script error: {logs:?}"
        );
        iso.resume();
        iso.probe("true").unwrap();
        iso.on_game_tick(2);
        assert_eq!(
            iso.probe("globalThis.__lastTick").unwrap(),
            serde_json::json!(2),
            "the consumed Pause interrupt must not leak into the resumed tick"
        );
        iso.join();
        return;
    }
    run_exact_child(
        "load::isolate::tests::pause_interrupts_a_newly_started_runaway_tick",
        CHILD,
    );
}
#[test]
fn paint_generation_allocator_separates_reset_and_replacement() {
    const CHILD: &str = "SCRIPT_PAINT_GENERATION_CHILD";
    if std::env::var_os(CHILD).is_some() {
        let old = LoadIsolate::spawn(
            "export function tick() {}".into(),
            LoadShape::NativeTick,
            vec![],
        )
        .unwrap();
        let initial = old
            .paint_generation
            .load(std::sync::atomic::Ordering::Acquire);
        old.reset_session_work(None);
        let after_reset = old
            .paint_generation
            .load(std::sync::atomic::Ordering::Acquire);
        let replacement = LoadIsolate::spawn(
            "export function tick() {}".into(),
            LoadShape::NativeTick,
            vec![],
        )
        .unwrap();
        let replacement_generation = replacement
            .paint_generation
            .load(std::sync::atomic::Ordering::Acquire);
        assert_ne!(initial, after_reset);
        assert_ne!(
            after_reset, replacement_generation,
            "paint epochs must not collide across isolate lifetimes"
        );
        old.join();
        replacement.join();
        return;
    }

    run_exact_child(
        "load::isolate::tests::paint_generation_allocator_separates_reset_and_replacement",
        CHILD,
    );
}

#[test]
fn reset_rejects_a_tick_queued_with_the_previous_session_generation() {
    let iso = LoadIsolate::spawn(
        "export function tick(api) { globalThis.n = (globalThis.n || 0) + 1; }".into(),
        LoadShape::NativeTick,
        vec![],
    )
    .unwrap();
    iso.reset_session_work(None);
    // A sender captured this tick before reset but enqueued it late.
    assert!(iso.send(IsolateCmd::Tick {
        tick: 1,
        generation: 0,
        input_identity: 0,
        evidence_sequence: 0,
        wait_only: false,
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
    crate::load::platform::bootstrap_test_v8_platform();
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
fn enter_root_clears_cped_after_throw_without_changing_default_termination() {
    crate::load::platform::bootstrap_test_v8_platform();
    let mut runtime = Runtime::new(RuntimeOptions::default()).unwrap();
    super::super::lineage_v8::install(&mut runtime, false).unwrap();
    assert!(!runtime
        .deno_runtime()
        .v8_isolate()
        .is_execution_terminating());

    let failed = super::super::lineage_v8::enter_root(&mut runtime, |runtime| {
        {
            let scope = &mut runtime.deno_runtime().handle_scope();
            let poison = rustyscript::deno_core::v8::Object::new(scope);
            scope.set_continuation_preserved_embedder_data(poison.into());
        }
        runtime.eval::<()>("throw new Error('root-entry probe')")
    });
    assert!(failed.is_err(), "the guarded JavaScript entry must throw");
    assert!(!runtime
        .deno_runtime()
        .v8_isolate()
        .is_execution_terminating());

    let undefined = super::super::lineage_v8::enter_root(&mut runtime, |runtime| {
        let scope = &mut runtime.deno_runtime().handle_scope();
        scope
            .get_continuation_preserved_embedder_data()
            .is_undefined()
    });
    assert!(undefined, "the next root entry must observe undefined CPED");
    assert!(!runtime
        .deno_runtime()
        .v8_isolate()
        .is_execution_terminating());
}

#[test]
fn prelude_canvas_keyboard_queues_key_rows_and_blocks_mouse_layout() {
    crate::load::platform::bootstrap_test_v8_platform();
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
    iso.reset_session_work(None);
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
    iso.reset_session_work(None);
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
    crate::load::platform::bootstrap_test_v8_platform();
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
    iso.reset_session_work(None);
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
}

fn machine_tick(iso: &LoadIsolate, n: u64) {
    iso.on_game_tick(n);
    let _ = iso.probe("true");
}

#[test]
fn unbound_v2_guarded_queue_delivers_held_request() {
    let source = r#"export const apiVersion = 2;
export function tick(api) {
  api.request({ op: 'held', name: 'Bones', action: 'Bury' });
}"#;
    let iso = LoadIsolate::spawn(source.into(), LoadShape::NativeTick, vec![]).unwrap();
    wait_ready(&iso);
    let snapshot = crate::isolate_fb::tests::empty_input(1);
    iso.post_snapshot(crate::isolate_fb::encode_snapshot_with_native(
        &snapshot,
        crate::isolate_fb::NativeFactsInput::default(),
    ));
    machine_tick(&iso, 1);
    let reqs = iso.drain_interacts();
    assert!(
        reqs.iter().any(|req| matches!(
            req,
            crate::shim::InteractReq::Held { name, action, .. }
                if name == "Bones" && action == "Bury"
        )),
        "unbound native-v2 guarded queue lost Held request: {reqs:?}; logs={:?}; outcomes={:?}; queue={:?}",
        iso.drain_logs(),
        iso.drain_tick_outcomes(),
        iso.probe("globalThis.__rs2b0t_host?.interact?.length")
    );
    iso.join();
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
    iso.reset_session_work(None);
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

// The awaited callback starts at tick 2, is polled while pending at tick 3,
// and resumes the same row in tick 4 without a second invocation.
#[test]
fn a_machine_calls_sync_and_async_script_callbacks_in_order() {
    let iso = spawn_machine_card(
        "globalThis.__calls = [];
             globalThis.__laterInvocations = 0;
             const at = () => globalThis.__rs2b0t_host.tick;
             const hooks = {
                 tag: 'H',
                 sync(n, s) {
                     globalThis.__calls.push(['sync', n, s, this.tag, at()]);
                     queue({ op: 'if-button', component_id: 900 });
                     return n + 1;
                 },
                 async later(v) {
                     globalThis.__laterInvocations += 1;
                     globalThis.__calls.push(['later', v, at()]);
                     await Execution.delayTicks(2);
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
        "the awaited callback starts once and remains pending"
    );
    assert_eq!(iso.probe("globalThis.__laterInvocations").unwrap(), 1);
    assert_eq!(
        iso.probe("globalThis.__out ?? null").unwrap(),
        serde_json::Value::Null
    );
    machine_tick(&iso, 3);
    assert_eq!(
        iso.probe("globalThis.__calls").unwrap(),
        serde_json::json!([["sync", 1, "a", "H", 2], ["later", 2, 2]]),
        "a pending row is polled, not stepped or reinvoked"
    );
    assert_eq!(iso.probe("globalThis.__laterInvocations").unwrap(), 1);
    assert_eq!(
        iso.probe("globalThis.__out ?? null").unwrap(),
        serde_json::Value::Null
    );
    assert!(iso.drain_interacts().is_empty());
    machine_tick(&iso, 4);
    assert_eq!(
        iso.probe("globalThis.__calls").unwrap(),
        serde_json::json!([
            ["sync", 1, "a", "H", 2],
            ["later", 2, 2],
            ["later-resumed", 2, 4],
            ["mark", 4],
        ]),
        "the row resumes in the tick the wait settles"
    );
    assert_eq!(iso.probe("globalThis.__laterInvocations").unwrap(), 1);
    assert_eq!(iso.drain_interacts(), vec![if_button(502), if_button(503)]);
    assert_eq!(
        iso.probe("globalThis.__out").unwrap(),
        serde_json::json!({ "kind": "done", "value": [2, 20, "m"] })
    );
    assert_eq!(iso.probe("globalThis.__at").unwrap(), 4);
    iso.join();
}

#[test]
fn sync_step_promises_are_opaque_while_ask_promises_fail() {
    let iso = spawn_machine_card(
        "const settled = Promise.resolve(false);
             const pending = new Promise(() => {});
             const settle = (promise) => promise.then(
                 (out) => ({ ok: out }),
                 (error) => ({ err: String(error) }),
             );
             const run = (promise) => settle(runMachine('hooked', {}, {
                 sync: () => promise,
                 later(value) {
                     globalThis.__opaque.push({
                         object: typeof value === 'object' && value !== null,
                         promise: value instanceof Promise,
                         empty: Object.keys(value).length === 0,
                         same: value === promise,
                     });
                     return value;
                 },
                 mark: () => 'mark',
             }));
             globalThis.__opaque = [];
             globalThis.__outcomes = await Promise.all([
                 run(settled),
                 run(pending),
                 settle(runMachine('solo-asker', {}, {
                     get: () => Promise.resolve(false),
                 })),
             ]);",
    );
    machine_tick(&iso, 1);
    machine_tick(&iso, 2);
    let outcomes = iso.probe("globalThis.__outcomes").unwrap();
    for index in [0, 1] {
        assert_eq!(
            outcomes[index]["ok"],
            serde_json::json!({ "kind": "done", "value": [{}, {}, "mark"] }),
            "a sync Step::Call does not await a settled or pending Promise"
        );
    }
    assert_eq!(
        iso.probe("globalThis.__opaque").unwrap(),
        serde_json::json!([
            { "object": true, "promise": false, "empty": true, "same": false },
            { "object": true, "promise": false, "empty": true, "same": false },
        ])
    );
    assert!(
        outcomes[2]["err"]
            .as_str()
            .is_some_and(|error| error
                .contains("not impl: a synchronous machine callback returned a promise")),
        "Cx::ask rejects a Promise instead of returning an opaque value: {outcomes:?}"
    );
    iso.join();
}

#[test]
fn real_hunt_effect_notifications_preserve_order_and_multiplicity_before_decision() {
    let iso = spawn_machine_card(
        "globalThis.__events = [];
             globalThis.__out = await runMachine('notification-order', {}, {
                 countKill() {
                     globalThis.__events.push(['countKill']);
                 },
                 eatOnce() {
                     globalThis.__events.push(['decision']);
                     return 'continue';
                 },
             });",
    );
    machine_tick(&iso, 1);
    assert_eq!(
        iso.probe("globalThis.__events").unwrap(),
        serde_json::json!([["countKill"], ["countKill"], ["decision"],]),
        "each event notification is delivered exactly once, in order, before the decision"
    );
    assert_eq!(
        iso.probe("globalThis.__out").unwrap(),
        serde_json::json!({ "kind": "done", "value": "continue" })
    );
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
fn thrown_value_identity_and_detached_child_survival() {
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
                     async later() {
                         globalThis.__detached = runMachine('probe', { button: 70, steps: 2 });
                         throw late;
                     },
                 })),
                 settle(runMachine('hooked', {}, { sync: 5 })),
                 settle(runMachine('hooked', {})),
                settle(runMachine('solo-asker', {}, {
                    get() { throw boom; },
                })),
             ]);
             globalThis.__detachedOut = await globalThis.__detached;",
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
    assert_eq!(
        outs[4],
        serde_json::json!({ "err": "boom", "same": true }),
        "Cx::ask fails with the very value its callback threw"
    );
    assert_eq!(
        iso.probe("globalThis.__detachedOut ?? null").unwrap(),
        serde_json::Value::Null,
        "the child remains pending after its parent's awaited hook fails"
    );
    assert_eq!(
        iso.drain_interacts(),
        vec![if_button(70)],
        "the detached child started before the parent failed"
    );
    machine_tick(&iso, 3);
    assert_eq!(iso.drain_interacts(), vec![if_button(71)]);
    machine_tick(&iso, 4);
    assert_eq!(iso.drain_interacts(), vec![if_button(72)]);
    machine_tick(&iso, 5);
    assert!(iso.drain_interacts().is_empty());
    assert_eq!(
        iso.probe("globalThis.__detachedOut").unwrap(),
        serde_json::json!({ "kind": "done", "value": 2 }),
        "the detached row completes after its parent rejects"
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
    iso.reset_session_work(None);
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
    std::thread::sleep(Duration::from_millis(650));
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

// M-187: a top-level synchronous runMachine kick must not drain
// microtasks the caller queued before the call. Frozen order is B,A.
#[test]
fn a_synchronous_kick_does_not_run_unrelated_microtasks() {
    let iso = LoadIsolate::spawn(
        "import { runMachine } from '../../shim/_kernel.js';
             globalThis.runMachine = runMachine;
             export default class T extends LoopingBot {
             loop() {}
             }"
        .into(),
        LoadShape::CompatClass,
        vec![],
    )
    .unwrap();
    wait_ready(&iso);
    iso.probe(
        "(() => {
             globalThis.__order = [];
             Promise.resolve().then(() => globalThis.__order.push('A'));
             globalThis.runMachine('kicked', { done: true }, { each() { return 0; } });
             globalThis.__order.push('B');
             return globalThis.__order;
         })()",
    )
    .expect("kick probe");
    // The caller's turn has finished; drain whatever the kick left queued.
    machine_tick(&iso, 1);
    assert_eq!(
        iso.probe("globalThis.__order").unwrap(),
        serde_json::json!(["B", "A"]),
        "unrelated microtasks must run after the caller continues, not inside the kick"
    );
    iso.join();
}

#[test]
fn v2_journal_read_hides_paint_until_observed_close_and_reset() {
    use crate::isolate_fb::{IsolateBuf, MainModalTextsInput, NativeFactsInput, QuestStatusInput};
    use crate::shim::InteractReq;

    let iso = LoadIsolate::spawn(
        r#"export const apiVersion = 2;
           export async function tick(api) {
             if (globalThis.__started) return;
             globalThis.__started = true;
             const begin = api.questJournalBegin({ name: "Cook's Assistant" });
             globalThis.__begin = begin;
             if (begin.ok) globalThis.__out = await api.questJournalRun(begin.value);
           }"#
        .into(),
        LoadShape::NativeTick,
        vec![],
    )
    .unwrap();
    let rows = [QuestStatusInput {
        name: "Cook's Assistant",
        status: "notStarted",
        component_id: Some(1234),
    }];
    let mut buf = IsolateBuf::new();
    let mut post = |tick, root, texts: &[String]| {
        let mut snapshot = crate::isolate_fb::tests::empty_input(tick);
        snapshot.scene_state = 2;
        snapshot.side_tab = 2;
        snapshot.main_modal_id = root;
        let facts = NativeFactsInput {
            quest_statuses: Some(&rows),
            main_modal_texts: Some(MainModalTextsInput { root, texts }),
            ..Default::default()
        };
        let (bytes, _) = buf.encode_snapshot_delta_with_native(None, &snapshot, facts, false);
        iso.post_snapshot(bytes);
        machine_tick(&iso, tick);
    };
    post(1, -1, &[]);
    assert_eq!(iso.probe("globalThis.__begin.ok").unwrap(), true);
    assert_eq!(iso.drain_interacts(), vec![if_button(1234)]);
    assert!(
        iso.compat_journal_paint_hidden(Instant::now()),
        "the existing v2 journal consumer must hide before its row click"
    );
    let lines = ["@dre@The Cook's Quest".to_owned()];
    post(2, 77, &lines);
    assert_eq!(iso.drain_interacts(), vec![InteractReq::CloseModal]);
    assert!(
        iso.compat_journal_paint_hidden(Instant::now()),
        "paint lease was revoked after CloseModal; tick errors: {:?}; logs: {:?}",
        iso.tick_errors.lock().unwrap(),
        iso.drain_logs()
    );
    post(3, 77, &lines);
    assert!(iso.drain_interacts().is_empty());
    assert!(iso.compat_journal_paint_hidden(Instant::now()));
    post(4, -1, &[]);
    assert_eq!(
        iso.probe("globalThis.__out.value.lines").unwrap(),
        serde_json::json!(lines)
    );
    assert!(!iso.compat_journal_paint_hidden(Instant::now()));

    iso.reset_session_work(None);
    iso.probe("globalThis.__started = false; true").unwrap();
    post(5, -1, &[]);
    assert_eq!(iso.drain_interacts(), vec![if_button(1234)]);
    assert!(
        iso.compat_journal_paint_hidden(Instant::now()),
        "reset binds the new generation"
    );
    iso.reset_session_work(None);
    assert!(
        !iso.compat_journal_paint_hidden(Instant::now()),
        "reset clears before another tick"
    );
    iso.join();
}

fn post_execution_seat_page<'a>(
    isolate: &LoadIsolate,
    buffer: &mut crate::isolate_fb::IsolateBuf,
    tick: u64,
    live: &'a [crate::native::seat::SeatEntry],
    results: &'a [crate::native::seat::SeatResult],
    details: &'a [crate::isolate_fb::SeatLiveDetails<'a>],
    api_progress: Option<&'a crate::api_progress::ProgressPage>,
) {
    let page =
        crate::isolate_fb::SeatPageInput::new(live, results, details, 0).expect("valid seat page");
    let snapshot = crate::isolate_fb::tests::empty_input(tick);
    let bytes = buffer.encode_snapshot_with_native(
        &snapshot,
        crate::isolate_fb::NativeFactsInput {
            api_progress,
            seat_page: Some(page),
            ..Default::default()
        },
    );
    isolate.post_snapshot(bytes);
    machine_tick(isolate, tick);
}

#[test]
fn v2_execution_seat_start_waits_for_live_ack_and_repeats_terminal_watermark() {
    use crate::api_progress::ProgressPage;
    use crate::isolate_fb::{SeatInteractDecoded, SeatLiveDetails, SeatStartArgs};
    use crate::machine::MachineEventKind;
    use crate::native::request::NativeKind;
    use crate::native::{seat::SeatTerminal, NativePhase};
    use api::identity::ExecutionId;
    use api::script_contract::ChildOrigin;
    use api::selected::RunKey;

    fn assert_root_loop_settled(rows: &[SeatInteractDecoded]) {
        assert!(
            matches!(
                rows,
                [SeatInteractDecoded {
                    lineage: None,
                    mirror_owner: None,
                    request: crate::shim::InteractReq::LoopSettled,
                }]
            ),
            "a synchronous v2 tick reports one root loop settlement: {rows:?}"
        );
    }

    let source = r#"
export const apiVersion = 2;
let started = false;
globalThis.__questSettles = 0;
globalThis.__questResult = null;
export function tick(api) {
  if (started) return;
  started = true;
  api.questProgress({ quest: "Cook's Assistant" }).then((result) => {
    globalThis.__questSettles += 1;
    globalThis.__questResult = result;
  });
}
"#;
    let execution = ExecutionId::first(RunKey {
        slot: 13,
        run: 28,
        session: 0,
    });
    let isolate =
        LoadIsolate::spawn_with_execution(source.into(), LoadShape::NativeTick, vec![], execution)
            .unwrap();
    wait_ready(&isolate);

    let mut buffer = crate::isolate_fb::IsolateBuf::new();
    let empty: [crate::native::seat::SeatEntry; 0] = [];
    let no_results: [crate::native::seat::SeatResult; 0] = [];
    let no_details: [SeatLiveDetails<'_>; 0] = [];
    post_execution_seat_page(
        &isolate,
        &mut buffer,
        1,
        &empty,
        &no_results,
        &no_details,
        None,
    );
    assert!(isolate.drain_interacts_stamped().is_empty());
    let pending = isolate.drain_seat_batches();
    assert_eq!(pending.len(), 1, "the root start is carried immediately");
    let start = pending[0]
        .batch
        .starts
        .first()
        .expect("one typed quest-progress start");
    let id = start.id;
    assert_eq!(start.kind, NativeKind::QuestProgress);
    assert!(matches!(
        &start.args,
        SeatStartArgs::QuestProgress(name) if name == "Cook's Assistant"
    ));
    assert_eq!((start.run, start.session), (28, 0));
    assert_eq!(pending[0].batch.consumed_terminal_seq, 0);
    assert_eq!(
        pending[0].batch.reqs.len(),
        1,
        "the pending Promise parks exactly one watchdog wait"
    );
    assert!(matches!(
        &pending[0].batch.reqs[0],
        crate::isolate_fb::SeatInteractDecoded {
            lineage: None,
            mirror_owner: None,
            request: crate::shim::InteractReq::WaitEnqueued,
        }
    ));
    assert!(pending[0].machine_events.is_empty());
    assert_eq!(isolate.probe("globalThis.__questSettles").unwrap(), 0);

    // The owner retries the same start until its id appears in the live page.
    post_execution_seat_page(
        &isolate,
        &mut buffer,
        2,
        &empty,
        &no_results,
        &no_details,
        None,
    );
    assert!(isolate.drain_interacts_stamped().is_empty());
    let retry = isolate.drain_seat_batches();
    assert_eq!(retry.len(), 1);
    assert_eq!(retry[0].batch.starts.len(), 1);
    assert_eq!(retry[0].batch.starts[0].id, id);
    assert_root_loop_settled(&retry[0].batch.reqs);
    assert_eq!(isolate.probe("globalThis.__questSettles").unwrap(), 0);

    let live = [crate::native::seat::SeatEntry {
        id,
        parent: None,
        kind: NativeKind::QuestProgress,
        origin: ChildOrigin::Started,
        depth: 0,
        decisions_consumed: 0,
        prompt: None,
    }];
    let details = [SeatLiveDetails {
        id,
        phase: NativePhase::Working,
        prompt: None,
    }];
    let reading = ProgressPage::Reading { token: id.get() };
    post_execution_seat_page(
        &isolate,
        &mut buffer,
        3,
        &live,
        &no_results,
        &details,
        Some(&reading),
    );
    assert!(isolate.drain_interacts_stamped().is_empty());
    let admitted = isolate.drain_seat_batches();
    assert!(
        admitted
            .iter()
            .flat_map(|batch| &batch.batch.starts)
            .all(|start| start.id != id),
        "live acknowledgement retires the pending start retry"
    );
    assert_eq!(isolate.probe("globalThis.__questSettles").unwrap(), 0);

    // The host's retained API page completes the local machine and publishes
    // its terminal event; the seat's retained result is acknowledged below.
    let refused = ProgressPage::Refused {
        token: id.get(),
        reason: std::sync::Arc::from("fixture"),
    };
    post_execution_seat_page(
        &isolate,
        &mut buffer,
        4,
        &live,
        &no_results,
        &details,
        Some(&refused),
    );
    assert!(isolate.drain_interacts_stamped().is_empty());
    let terminal = isolate.drain_seat_batches();
    assert!(
        terminal.iter().any(|batch| {
            batch
                .machine_events
                .iter()
                .any(|event| event.id == id && event.kind == MachineEventKind::Done)
        }),
        "local terminal is published against the admitted native owner"
    );
    assert!(terminal
        .iter()
        .flat_map(|batch| &batch.batch.starts)
        .all(|start| start.id != id));
    assert_eq!(isolate.probe("globalThis.__questSettles").unwrap(), 1);
    assert_eq!(
        isolate.probe("globalThis.__questResult").unwrap(),
        serde_json::json!({
            "kind": "done",
            "value": { "end": "refused", "token": id.get(), "reason": "fixture" }
        })
    );

    let result = [crate::native::seat::SeatResult {
        id,
        seq: 1,
        outcome: SeatTerminal::Done,
        prior_parent: None,
        session: None,
        authority_revoked: false,
    }];
    for tick in [5, 6] {
        post_execution_seat_page(
            &isolate,
            &mut buffer,
            tick,
            &empty,
            &result,
            &no_details,
            Some(&refused),
        );
        assert!(isolate.drain_interacts_stamped().is_empty());
        let receipt = isolate.drain_seat_batches();
        assert_eq!(
            receipt.len(),
            1,
            "retained terminal is acked on tick {tick}"
        );
        assert_eq!(receipt[0].batch.consumed_terminal_seq, 1);
        assert!(receipt[0].batch.starts.is_empty());
        assert_root_loop_settled(&receipt[0].batch.reqs);
        assert!(receipt[0].machine_events.is_empty());
    }
    assert_eq!(isolate.probe("globalThis.__questSettles").unwrap(), 1);

    post_execution_seat_page(
        &isolate,
        &mut buffer,
        7,
        &empty,
        &no_results,
        &no_details,
        Some(&refused),
    );
    assert!(isolate.drain_interacts_stamped().is_empty());
    let after_retirement = isolate.drain_seat_batches();
    assert_eq!(
        after_retirement.len(),
        1,
        "the synchronous tick still reports its root loop settlement"
    );
    assert_eq!(
        after_retirement[0].batch.consumed_terminal_seq,
        1,
        "the highest consumed sequence remains monotonic on later lifecycle rows"
    );
    assert!(after_retirement[0].batch.starts.is_empty());
    assert_root_loop_settled(&after_retirement[0].batch.reqs);
    isolate.join();
}

#[test]
fn execution_owned_v8_caps_rows_and_reserves_controls() {
    use crate::isolate_fb::MAX_SEAT_GAMEPLAY_ROWS;
    use crate::shim::InteractReq;
    use api::identity::ExecutionId;
    use api::selected::RunKey;

    let source = r#"
import { Execution } from '../../api/execution/Execution.js';
import { RunManager } from '../../runtime/RunManager.js';
export const apiVersion = 2;
export function tick() {
  for (let i = 0; i < 300; i++) Execution.noteProgress();
  globalThis.__rs2b0t_host.interact.push({ op: 'abort-walk', request_id: 123 });
  RunManager.override({ runAuto: false, energyMin: 55 });
}
"#;
    let execution = ExecutionId::first(RunKey {
        slot: 14,
        run: 29,
        session: 0,
    });
    let isolate =
        LoadIsolate::spawn_with_execution(source.into(), LoadShape::NativeTick, vec![], execution)
            .unwrap();
    wait_ready(&isolate);

    let empty: [crate::native::seat::SeatEntry; 0] = [];
    let no_results: [crate::native::seat::SeatResult; 0] = [];
    let no_details: [crate::isolate_fb::SeatLiveDetails<'_>; 0] = [];
    let mut buffer = crate::isolate_fb::IsolateBuf::new();
    post_execution_seat_page(
        &isolate,
        &mut buffer,
        1,
        &empty,
        &no_results,
        &no_details,
        None,
    );

    assert!(
        isolate.drain_interacts_stamped().is_empty(),
        "execution-owned rows must not leak to the legacy interact drain"
    );
    let batches = isolate.drain_seat_batches();
    assert_eq!(
        batches.len(),
        1,
        "one owner batch carries the tick output; tick errors: {:?}; logs: {:?}",
        isolate.tick_errors.lock().unwrap(),
        isolate.drain_logs()
    );
    let requests = &batches[0].batch.reqs;
    assert_eq!(requests.len(), MAX_SEAT_GAMEPLAY_ROWS + 2);
    assert!(matches!(
        &requests[0].request,
        InteractReq::AbortWalk { request_id: 123 }
    ));
    assert!(matches!(
        &requests[1].request,
        InteractReq::RunPolicyOverride {
            policy: Some(policy),
        } if policy.run_auto == Some(false)
            && policy.energy_min == Some(api::run_policy::RunEnergyMin::Floor(55))
    ));
    assert_eq!(
        requests[2..]
            .iter()
            .filter(|row| matches!(&row.request, InteractReq::NoteProgress))
            .count(),
        MAX_SEAT_GAMEPLAY_ROWS,
        "the first 141 gameplay rows survive the owner cap"
    );
    assert!(requests[2..]
        .iter()
        .all(|row| matches!(&row.request, InteractReq::NoteProgress)));
    let logs = isolate.drain_logs();
    assert!(
        logs.iter()
            .any(|line| line.contains("dropped 159 over-cap seat interact rows")),
        "{logs:?}"
    );
    isolate.join();
}

#[test]
fn execution_owned_reset_restates_parked_waits_on_the_seat_lane() {
    use crate::shim::InteractReq;
    use api::identity::ExecutionId;
    use api::selected::RunKey;

    let source = r#"
import { Execution } from '../../api/execution/Execution.js';
export const apiVersion = 2;
let waiting = false;
export function tick() {
  if (waiting) return;
  waiting = true;
  void Execution.delayTicks(100);
}
"#;
    let execution = ExecutionId::first(RunKey {
        slot: 15,
        run: 30,
        session: 0,
    });
    let isolate =
        LoadIsolate::spawn_with_execution(source.into(), LoadShape::NativeTick, vec![], execution)
            .unwrap();
    wait_ready(&isolate);

    let empty: [crate::native::seat::SeatEntry; 0] = [];
    let no_results: [crate::native::seat::SeatResult; 0] = [];
    let no_details: [crate::isolate_fb::SeatLiveDetails<'_>; 0] = [];
    let mut buffer = crate::isolate_fb::IsolateBuf::new();
    post_execution_seat_page(
        &isolate,
        &mut buffer,
        1,
        &empty,
        &no_results,
        &no_details,
        None,
    );
    let first = isolate.drain_seat_batches();
    assert_eq!(
        first.len(),
        1,
        "initial parked wait is carried on the owner lane; tick errors: {:?}; logs: {:?}",
        isolate.tick_errors.lock().unwrap(),
        isolate.drain_logs()
    );
    assert_eq!(
        first[0]
            .batch
            .reqs
            .iter()
            .filter(|row| matches!(&row.request, InteractReq::WaitEnqueued))
            .count(),
        1
    );
    assert!(isolate.drain_interacts_stamped().is_empty());

    isolate.reset_session_work(Some(execution));
    assert_eq!(isolate.probe("true").unwrap(), serde_json::json!(true));
    assert!(isolate.drain_interacts_stamped().is_empty());
    let reset = isolate.drain_seat_batches();
    assert_eq!(
        reset.len(),
        1,
        "reset wait facts use the execution owner lane"
    );
    assert_eq!(reset[0].batch.reqs.len(), 1);
    assert!(matches!(
        &reset[0].batch.reqs[0].request,
        InteractReq::WaitEnqueued
    ));
    isolate.join();
}

#[test]
fn execution_owned_recovery_anchor_uses_the_seat_lane() {
    use crate::shim::InteractReq;
    use api::identity::ExecutionId;
    use api::selected::RunKey;

    let execution = ExecutionId::first(RunKey {
        slot: 16,
        run: 31,
        session: 0,
    });
    let isolate = LoadIsolate::spawn_with_execution(
        "export const apiVersion = 2; export function tick() {}".into(),
        LoadShape::NativeTick,
        vec![],
        execution,
    )
    .unwrap();
    wait_ready(&isolate);
    isolate
        .probe(
            "globalThis.__rs_bot = { recoveryAnchor() { return { x: 10, z: 20, level: 1 }; } }; true",
        )
        .unwrap();

    isolate.request_recovery_anchor();
    assert_eq!(isolate.probe("true").unwrap(), serde_json::json!(true));
    assert!(isolate.drain_interacts_stamped().is_empty());
    let batches = isolate.drain_seat_batches();
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].batch.reqs.len(), 1);
    assert!(matches!(
        &batches[0].batch.reqs[0].request,
        InteractReq::RecoveryAnchor {
            x: 10,
            z: 20,
            level: 1
        }
    ));
    isolate.join();
}

#[test]
fn execution_owned_inspect_ack_uses_the_seat_lane() {
    use crate::isolate_fb::{
        IsolateBuf, NativeFactsInput, RouteInspectFactsInput, RouteInspectTerminalInput,
    };
    use crate::shim::InteractReq;
    use api::identity::ExecutionId;
    use api::selected::RunKey;

    let execution = ExecutionId::first(RunKey {
        slot: 17,
        run: 32,
        session: 0,
    });
    let isolate = LoadIsolate::spawn_with_execution(
        "export const apiVersion = 2; export function tick() {}".into(),
        LoadShape::NativeTick,
        vec![],
        execution,
    )
    .unwrap();
    wait_ready(&isolate);

    let snapshot = crate::isolate_fb::tests::empty_input(1);
    let facts = NativeFactsInput {
        route_inspect: RouteInspectFactsInput {
            latest: RouteInspectTerminalInput {
                seq: 7,
                generation: 4,
                request_id: 88,
                ..Default::default()
            },
            ..Default::default()
        },
        ..Default::default()
    };
    let mut buffer = IsolateBuf::new();
    isolate.post_snapshot(buffer.encode_snapshot_with_native(&snapshot, facts));
    assert_eq!(isolate.probe("true").unwrap(), serde_json::json!(true));

    assert!(isolate.drain_interacts_stamped().is_empty());
    let batches = isolate.drain_seat_batches();
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].batch.reqs.len(), 1);
    assert!(matches!(
        &batches[0].batch.reqs[0].request,
        InteractReq::InspectAck {
            seq: 7,
            generation: 4
        }
    ));
    isolate.join();
}
