//! Paint helper and paint-controls descriptors keep fast properties on the
//! real recorder path. Host controls are recorded without a per-frame copy.
//!
//! This is a `harness = false` executable so `--allow-natives-syntax` can be
//! set before any V8 platform exists (as in `entity_row_shape.rs`). The bot
//! calls the real `PaintFrame.stepper` and `ScriptRunner.paintControls`
//! helpers from its `onPaint`; the probe reads the records that `Paint.end`
//! wrote for the host.

use script_js::{LoadIsolate, LoadShape};

const BOT_SOURCE: &str = r#"
import { Paint } from '../../paint/Paint.js';
import { ScriptRunner } from '../../runtime/ScriptRunner.js';
export default class PaintControlsShape extends LoopingBot {
    loop() {}
    onPaint() {
        const p = Paint.begin(null, {});
        p.title('Helper');
        p.row('status');
        p.stepper('mode', 'Mode', ['one', 'two'], 'one');
        p.buttons([{ id: 'gobank', label: 'Go bank' }]);
        p.gap();
        ScriptRunner.paintControls(p);
        p.end();
        globalThis.__previous = globalThis.__current;
        globalThis.__current = globalThis.__rs2b0t_host.paint;
    }
}
"#;

const READ_FACTS: &str = r#"(() => {
    const fast = new Function('o', 'return %HasFastProperties(o)');
    const facts = (o) => ({
        fast: fast(o),
        keys: Object.keys(o),
        frozen: Object.isFrozen(o),
    });
    const previous = globalThis.__previous;
    const current = globalThis.__current;
    return {
        record: facts(current),
        buttons: current.buttons.map(facts),
        ids: current.buttons.map((b) => b.id),
        sameObject: current.buttons.map((b, i) => b === previous.buttons[i]),
    };
})()"#;

fn probe(iso: &LoadIsolate, expr: &str) -> serde_json::Value {
    iso.probe(expr)
        .unwrap_or_else(|error| panic!("probe failed: {error}"))
}

fn main() {
    // Must run before V8 exists in this process: the isolated executable
    // exists precisely to keep this flag off every other test binary.
    v8::V8::set_flags_from_string("--allow-natives-syntax");
    script_js::bootstrap_v8_platform().expect("process-main V8 bootstrap");
    let iso = LoadIsolate::spawn(BOT_SOURCE.into(), LoadShape::CompatClass, vec![])
        .expect("spawn paint-controls isolate");
    for tick in 1..=2 {
        iso.on_game_tick(tick);
        probe(&iso, "true");
    }
    let logs = iso.drain_logs();
    assert!(
        logs.iter().all(|line| !line.contains("not impl")),
        "the helper frame must not throw: {logs:?}"
    );
    let facts = probe(&iso, READ_FACTS);

    assert_eq!(
        facts["record"]["fast"], true,
        "the recorded paint frame must have fast properties: {facts}"
    );
    assert_eq!(
        facts["ids"],
        serde_json::json!([
            "step:mode:prev",
            "step:mode:mid",
            "step:mode:next",
            "gobank",
            script::paint::HOST_PAUSE_CONTROL_ID,
            script::paint::HOST_STOP_CONTROL_ID
        ]),
        "{facts}"
    );
    for (index, button) in facts["buttons"]
        .as_array()
        .expect("buttons")
        .iter()
        .enumerate()
    {
        assert_eq!(
            button["fast"], true,
            "button {index} must have fast properties: {facts}"
        );
        assert_eq!(
            button["keys"],
            serde_json::json!(["id", "label"]),
            "button {index} field order: {facts}"
        );
    }
    // Stepper descriptors are native-created and rebuilt per frame. The
    // script descriptor is copied; the host controls are the frozen cache.
    assert_eq!(
        facts["sameObject"],
        serde_json::json!([false, false, false, false, true, true]),
        "the stepper and script descriptors are new while host controls are shared: {facts}"
    );
    for index in 0..4 {
        assert_eq!(
            facts["buttons"][index]["frozen"], false,
            "button {index} is an ordinary descriptor: {facts}"
        );
    }
    assert_eq!(facts["buttons"][4]["frozen"], true, "{facts}");
    assert_eq!(facts["buttons"][5]["frozen"], true, "{facts}");
    iso.join();
    println!("paint_controls_shape: ok {facts}");
}
