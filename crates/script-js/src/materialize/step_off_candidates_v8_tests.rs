use super::*;
use rustyscript::deno_core::v8;
use rustyscript::RuntimeOptions;

const CHILD_ENV: &str = "SCRIPT_STEP_OFF_CANDIDATES_V8_TEST_CHILD";
const TEST_NAME: &str =
    "materialize::step_off_candidates_v8::tests::rows_have_fast_properties_and_bounded_v8_heap";
const SETUP: &str = r#"
globalThis.__stepRows = () => __rs2b0t_step_off_candidates(10, 20, 2, 6, 18);
globalThis.__fastStepRows = () => {
    const fast = new Function('value', 'return %HasFastProperties(value)');
    const rows = __stepRows();
    return Array.isArray(rows) && rows.length === 8
        && rows.every(row => fast(row) && row.x !== undefined && row.z !== undefined && row.level === 2);
};
globalThis.__repeatStepRows = count => {
    let countRows = 0;
    for (let index = 0; index < count; index++) countRows += __stepRows().length;
    return countRows;
};
"#;

fn setup() -> Runtime {
    crate::runtime::platform::bootstrap_test_v8_platform();
    let mut runtime = Runtime::new(RuntimeOptions::default()).unwrap();
    install(&mut runtime).unwrap();
    runtime.eval::<()>(SETUP).unwrap();
    runtime
}

fn collected_used_heap(runtime: &mut Runtime) -> usize {
    let isolate = runtime.deno_runtime().v8_isolate();
    isolate.low_memory_notification();
    isolate.get_heap_statistics().used_heap_size()
}

#[test]
fn rows_have_fast_properties_and_bounded_v8_heap() {
    if std::env::var_os(CHILD_ENV).is_none() {
        let output = std::process::Command::new(std::env::current_exe().expect("test executable"))
            .args(["--exact", TEST_NAME, "--nocapture"])
            .env(CHILD_ENV, "1")
            .output()
            .expect("run isolated V8 native-syntax test");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            output.status.success() && stdout.contains("1 passed"),
            "isolated V8 test failed\nstdout: {stdout}\nstderr: {stderr}"
        );
        return;
    }
    v8::V8::set_flags_from_string("--allow-natives-syntax");
    let mut runtime = setup();
    assert!(runtime.eval::<bool>("__fastStepRows()").unwrap());
    // The native walk contract always returns the eight adjacent compass
    // tiles, independent of the caller's distance or the coordinates.
    runtime.eval::<usize>("__repeatStepRows(1000)").unwrap();
    let baseline = collected_used_heap(&mut runtime);
    assert_eq!(
        runtime.eval::<usize>("__repeatStepRows(1000)").unwrap(),
        8_000
    );
    let after = collected_used_heap(&mut runtime);
    let growth = after.saturating_sub(baseline);
    let limit = 16_384;
    println!(
        "step-off V8 heap rows=8 baseline={baseline} after={after} growth={growth} limit={limit}"
    );
    assert!(
        growth <= limit,
        "collected V8 heap grew {growth} bytes; limit {limit}"
    );
}
