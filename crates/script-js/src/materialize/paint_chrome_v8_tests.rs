use super::*;
use rustyscript::{Runtime, RuntimeOptions};

const CHILD_ENV: &str = "SCRIPT_PAINT_STEPPER_SHAPE_TEST_CHILD";
const TEST_NAME: &str =
    "paint_chrome::tests::stepper_descriptors_keep_fast_properties_and_heap_growth_is_bounded";

const SETUP: &str = r#"
globalThis.__draw = click => {
    const record = { lines: [], buttons: [] };
    const host = { paintClick: click };
    __rs2b0t_paint_chrome('begin', 'chatbox', null);
    const picked = __rs2b0t_paint_chrome(
        'stepper', record, 'mode', 'Mode', ['one', 'two'], 'one', 0, host
    );
    return { record, picked, host };
};
globalThis.__last = __draw(null);
globalThis.__fast = () => {
    const fast = new Function('value', 'return %HasFastProperties(value)');
    const buttons = __last.record.buttons;
    return buttons.length === 3
        && buttons.every(button => fast(button))
        && buttons.every(button => Object.keys(button).join(',') === 'id,label');
};
globalThis.__retainedButtons = [];
globalThis.__repeat = count => {
    for (let index = 0; index < count; index++) {
        globalThis.__last = __draw('step:mode:next');
        globalThis.__retainedButtons.push(...globalThis.__last.record.buttons);
    }
    return globalThis.__retainedButtons.length;
};
globalThis.__title = click => {
    const host = { paintClick: click };
    __rs2b0t_paint_chrome('title', host);
    return host.paintClick;
};
globalThis.__makeTemporaryPaintStates = count => {
    for (let index = 0; index < count; index++) {
        const owner = {};
        __rs2b0t_paint_chrome('stateNew', owner);
        __rs2b0t_paint_chrome('stateSet', owner, 'temporary', String(index));
    }
    return count;
};
globalThis.__stateIsolation = () => {
    const left = {};
    const right = {};
    __rs2b0t_paint_chrome('stateNew', left);
    __rs2b0t_paint_chrome('stateNew', right);
    __rs2b0t_paint_chrome('stateSet', left, 'mode', 'left');
    __rs2b0t_paint_chrome('stateSet', right, 'mode', 'right');
    return [
        __rs2b0t_paint_chrome('stateGet', left, 'mode', ''),
        __rs2b0t_paint_chrome('stateGet', right, 'mode', ''),
    ].join(':');
};
globalThis.__stateOwner = {};
__rs2b0t_paint_chrome('stateNew', __stateOwner);
__rs2b0t_paint_chrome('stateSet', __stateOwner, 'persist', 'before reset');
"#;

fn setup() -> Runtime {
    crate::platform::bootstrap_test_v8_platform();
    let mut runtime = Runtime::new(RuntimeOptions::default()).unwrap();
    reset();
    install(&mut runtime).unwrap();
    runtime.eval::<()>(SETUP).unwrap();
    runtime
}

fn collected_used_heap(runtime: &mut Runtime) -> usize {
    let isolate = runtime.deno_runtime().v8_isolate();
    isolate.low_memory_notification();
    isolate.low_memory_notification();
    isolate.get_heap_statistics().used_heap_size()
}

fn in_child(body: impl FnOnce()) {
    if std::env::var_os(CHILD_ENV).is_none() {
        let output = std::process::Command::new(std::env::current_exe().expect("test executable"))
            .args(["--exact", TEST_NAME, "--nocapture"])
            .env(CHILD_ENV, "1")
            .output()
            .expect("run isolated paint stepper shape test");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            output.status.success() && stdout.contains("1 passed"),
            "isolated paint stepper test failed\nstdout: {stdout}\nstderr: {stderr}"
        );
        return;
    }
    v8::V8::set_flags_from_string("--allow-natives-syntax");
    body();
}

#[test]
fn stepper_descriptors_keep_fast_properties_and_heap_growth_is_bounded() {
    in_child(|| {
        let mut runtime = setup();
        assert!(runtime.eval::<bool>("__fast()").unwrap());
        assert_eq!(
            runtime
                .eval::<String>("__draw('step:mode:next').picked")
                .unwrap(),
            "two"
        );
        assert!(
            runtime
                .eval::<bool>("__draw('step:mode:next').host.paintClick === null")
                .unwrap(),
            "the next click is consumed by Rust"
        );
        assert_eq!(
            runtime
                .eval::<String>("__draw('step:mode:mid').host.paintClick")
                .unwrap(),
            "step:mode:mid",
            "the display-only middle click remains pending"
        );

        let baseline = collected_used_heap(&mut runtime);
        let iterations = 1_000;
        let rows = iterations * 3;
        let repeat = format!("__repeat({iterations})");
        assert_eq!(runtime.eval::<usize>(&repeat).unwrap(), rows);
        assert_eq!(
            runtime.eval::<usize>("__retainedButtons.length").unwrap(),
            rows,
            "the heap measurement retains every generated button"
        );
        let after = collected_used_heap(&mut runtime);
        let growth = after.saturating_sub(baseline);
        // 3,000 retained buttons grew 381,552 B on aubaine; 25% headroom covers V8 variance.
        let limit = 381_552 + 381_552 / 4;
        println!("paint stepper V8 heap rows={rows} iterations={iterations} baseline={baseline} after={after} growth={growth} limit={limit}");
        assert!(
            growth <= limit,
            "collected V8 heap grew {growth} bytes; limit {limit}"
        );

        assert_eq!(runtime.eval::<String>("__stateIsolation()").unwrap(), "left:right");
        let state_baseline = collected_used_heap(&mut runtime);
        let state_count = 1_000;
        let create_states = format!("__makeTemporaryPaintStates({state_count})");
        let rust = allocation_counter::measure(|| {
            assert_eq!(runtime.eval::<usize>(&create_states).unwrap(), state_count);
        });
        let state_after = collected_used_heap(&mut runtime);
        let state_growth = state_after.saturating_sub(state_baseline);
        // 1,000 temporary stores grew 291,240 B on aubaine; 25% headroom covers V8 variance.
        let state_limit = 291_240 + 291_240 / 4;
        println!("paint state stores V8 heap states={state_count} baseline={state_baseline} after={state_after} growth={state_growth} limit={state_limit} rust={rust:?}");
        assert!(
            state_growth <= state_limit,
            "collected V8 heap grew {state_growth} bytes for temporary PaintState stores; limit {state_limit}"
        );
        assert!(
            rust.bytes_current <= 4 * 1024,
            "Rust retained {rust:?} after temporary PaintState stores"
        );

        assert!(runtime
            .eval::<bool>("__title('paint:toggle') === null")
            .unwrap());
        assert_eq!(
            runtime.eval::<String>("__title('another')").unwrap(),
            "another"
        );
        reset();
        assert_eq!(
            runtime
                .eval::<String>(
                    "__rs2b0t_paint_chrome('stateGet', __stateOwner, 'persist', 'fallback')"
                )
                .unwrap(),
            "fallback"
        );
        runtime
            .eval::<()>("__rs2b0t_paint_chrome('stateSet', __stateOwner, 'persist', 'after reset')")
            .unwrap();
        assert_eq!(
            runtime
                .eval::<String>(
                    "__rs2b0t_paint_chrome('stateGet', __stateOwner, 'persist', 'fallback')"
                )
                .unwrap(),
            "after reset"
        );
    });
}
