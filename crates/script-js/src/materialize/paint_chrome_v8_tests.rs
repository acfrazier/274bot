use super::*;
use rustyscript::{Runtime, RuntimeOptions};

const CHILD_ENV: &str = "SCRIPT_PAINT_STEPPER_SHAPE_TEST_CHILD";
const TEST_NAME: &str = "materialize::paint_chrome::tests::stepper_descriptors_keep_fast_properties_and_heap_growth_is_bounded";

const SETUP: &str = r#"
globalThis.__draw = click => {
    const record = { lines: [], buttons: [] };
    __rs2b0t_paint_chrome('begin', 'chatbox', null);
    const picked = __rs2b0t_paint_chrome(
        'stepper', record, 'mode', 'Mode', ['one', 'two'], 'one', 0, click
    );
    return { record, picked };
};
globalThis.__last = __draw(null);
globalThis.__fast = () => {
    const fast = new Function('value', 'return %HasFastProperties(value)');
    const buttons = __last.record.buttons;
    return buttons.length === 3
        && buttons.every(button => fast(button))
        && buttons.every(button => Object.keys(button).join(',') === 'id,label');
};
globalThis.__repeat = count => {
    for (let index = 0; index < count; index++) {
        globalThis.__last = __draw('step:mode:next');
    }
    return count * globalThis.__last.record.buttons.length;
};
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

        let baseline = collected_used_heap(&mut runtime);
        let iterations = 1_000;
        let rows = iterations * 3;
        assert_eq!(
            runtime
                .eval::<usize>(&format!("__repeat({iterations})"))
                .unwrap(),
            rows
        );
        let after = collected_used_heap(&mut runtime);
        let growth = after.saturating_sub(baseline);
        let limit = 64 * 1024;
        println!("paint stepper V8 heap rows={rows} iterations={iterations} baseline={baseline} after={after} growth={growth} limit={limit}");
        assert!(
            growth <= limit,
            "collected V8 heap grew {growth} bytes; limit {limit}"
        );
    });
}
