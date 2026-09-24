//! F12 paint: cross-language calls per `onPaint` pass.
//!
//! Every canvas member used to be its own `rustyscript.functions.__rs2b0t_canvas_*`
//! call, so a pass cost one JS->Rust crossing per op. The recorded pass must not
//! scale with the script's op count. This counts the real native calls by
//! wrapping the runtime's function table with a counting proxy.

use std::path::PathBuf;

use script::LoadIsolate;

mod common;

/// Wrap `rustyscript.functions` and the tape's direct global so every
/// `__rs2b0t_canvas*` call increments a global counter. Returns whether the
/// wrapper is installed.
const INSTALL_COUNTER: &str = r#"
(() => {
    const real = globalThis.rustyscript.functions;
    const counting = new Proxy({}, {
        get(target, name) {
            const fn = real[name];
            if (typeof fn !== 'function') return fn;
            if (typeof name === 'string' && name.indexOf('__rs2b0t_canvas') === 0) {
                return (...args) => {
                    globalThis.__canvasCalls += 1;
                    return fn(...args);
                };
            }
            return fn;
        },
    });
    globalThis.__canvasCalls = 0;
    globalThis.rustyscript = {
        functions: counting,
        async_functions: globalThis.rustyscript.async_functions,
    };
    const submit = globalThis.__rs2b0t_canvas_submit;
    globalThis.__rs2b0t_canvas_submit = (...args) => {
        globalThis.__canvasCalls += 1;
        return submit(...args);
    };
    return globalThis.rustyscript.functions === counting && typeof submit === 'function';
})()
"#;

fn spawn(src: &str) -> LoadIsolate {
    let shape = script::load::detect_shape(src);
    LoadIsolate::spawn(src.to_string(), shape, vec![]).expect("bot loads")
}

fn spawn_counting(src: &str) -> LoadIsolate {
    let iso = spawn(src);
    assert_eq!(
        iso.probe(INSTALL_COUNTER).unwrap(),
        serde_json::Value::Bool(true),
        "counting proxy installed"
    );
    iso
}

/// The compat paint gate holds until `onStart` finished and the scene is
/// ready, so ExampleBot's `onStart` needs a live-shaped snapshot.
fn spawn_counting_ingame(src: &str) -> LoadIsolate {
    let iso = spawn_counting(src);
    common::post_snapshot_input(&iso, &common::ingame_snapshot());
    iso
}

fn tick_no_paint(iso: &LoadIsolate, n: u64) {
    iso.on_game_tick(n);
    let _ = iso.probe("0");
}

/// One paint pass: reset the counter, tick, read it back.
fn pass_crossings(iso: &LoadIsolate, n: u64) -> i64 {
    let _ = iso.probe("globalThis.__canvasCalls = 0");
    iso.on_game_tick(n);
    let calls = iso
        .probe("globalThis.__canvasCalls")
        .expect("counter readable")
        .as_i64()
        .expect("counter is a number");
    assert!(iso.paint().is_some(), "tick {n} forwarded a paint frame");
    calls
}

fn examplebot_source() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ExampleBot.ts");
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let src = String::from_utf8(bytes).expect("ExampleBot utf-8");
    script::transpile_ts(&src).expect("transpile ExampleBot")
}

/// The pinned template: 2 style sets, a measure, a fillRect and a fillText.
#[test]
fn examplebot_paint_pass_stays_within_the_tape_budget() {
    let iso = spawn_counting_ingame(&examplebot_source());
    tick_no_paint(&iso, 1);
    let calls = pass_crossings(&iso, 2);
    println!("ExampleBot: {calls} canvas crossings per paint pass");
    assert!(
        (3..=5).contains(&calls),
        "ExampleBot canvas crossings per paint pass: {calls} (begin + measure + flush + done)"
    );
    let steady = pass_crossings(&iso, 3);
    assert_eq!(
        steady, calls,
        "a later pass costs the same: {steady} vs {calls}"
    );
    iso.join();
}

/// 280 recorded ops must not become 280 crossings.
#[test]
fn crossings_do_not_scale_with_recorded_ops() {
    let src = r#"
export default class Wide extends LoopingBot {
    onPaint(ctx) {
        ctx.font = '12px monospace';
        for (let i = 0; i < 40; i++) {
            ctx.fillStyle = i % 2 ? '#101010' : '#202020';
            ctx.fillRect(i, i, 4, 4);
            ctx.beginPath();
            ctx.moveTo(0, 0);
            ctx.lineTo(i, i);
            ctx.stroke();
        }
        ctx.fillText('wide', 8, 300);
    }
}
"#;
    let iso = spawn_counting(src);
    let calls = pass_crossings(&iso, 1);
    let frame = iso.paint().expect("wide paint");
    let ops = frame.canvas.len() as i64;
    println!("wide overlay: {ops} recorded canvas ops, {calls} canvas crossings");
    assert!(ops >= 80, "the pass recorded {ops} ops");
    assert!(
        calls <= 4,
        "{ops} recorded canvas ops cost {calls} crossings"
    );
    iso.join();
}
