use super::*;
use rustyscript::RuntimeOptions;

const CHILD_ENV: &str = "SCRIPT_QUEST_JUNK_SHAPE_TEST_CHILD";
const TEST_NAME: &str =
    "materialize::bank_quest_junk_v8::tests::quest_junk_rows_have_fast_properties_and_bound_v8_heap";

/// A full bank (288 rows, the selected capacity) of distinct table ids, so each
/// call materializes one finding per matching table row.
const SETUP: &str = r#"
globalThis.__table = __rs2b0t_quest_junk_table();
globalThis.__banked = [...new Set(__table.map((row) => row.id))].slice(0, 288).map((id) => ({ id }));
globalThis.__find = () => __rs2b0t_find_quest_junk(__banked, () => 'complete');
globalThis.__fast = () => {
    const fast = new Function('value', 'return %HasFastProperties(value)');
    return __table.every(fast) && __find().every(fast);
};
globalThis.__repeat = (count) => {
    let kept = 0;
    for (let i = 0; i < count; i++) kept += __find().length;
    return kept;
};
"#;

fn setup() -> Runtime {
    crate::runtime::platform::bootstrap_test_v8_platform();
    let mut runtime = Runtime::new(RuntimeOptions::default()).unwrap();
    cb::install_ops(&mut runtime).unwrap();
    install(&mut runtime).unwrap();
    runtime.eval::<()>(SETUP).unwrap();
    runtime
}

fn collected_used_heap(runtime: &mut Runtime) -> usize {
    let isolate = runtime.deno_runtime().v8_isolate();
    isolate.low_memory_notification();
    isolate.get_heap_statistics().used_heap_size()
}

/// Run `name` alone in a child process so `--allow-natives-syntax` is set
/// before this process's platform starts.
fn in_child(name: &str) -> bool {
    if std::env::var_os(CHILD_ENV).is_some() {
        return true;
    }
    let output = std::process::Command::new(std::env::current_exe().expect("test executable"))
        .args(["--exact", name, "--nocapture"])
        .env(CHILD_ENV, "1")
        .output()
        .expect("run isolated V8-native-syntax test");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success() && stdout.contains("1 passed"),
        "isolated V8 test failed\nstdout: {stdout}\nstderr: {stderr}"
    );
    false
}

#[test]
fn quest_junk_rows_have_fast_properties_and_bound_v8_heap() {
    if !in_child(TEST_NAME) {
        return;
    }
    v8::V8::set_flags_from_string("--allow-natives-syntax");
    let mut runtime = setup();
    assert!(
        runtime.eval::<bool>("__fast()").unwrap(),
        "table rows and findings are built natively with fast properties"
    );
    let rows = runtime.eval::<usize>("__find().length").unwrap();
    let banked = runtime.eval::<usize>("__banked.length").unwrap();
    assert!(
        rows >= banked,
        "every banked id matches at least one table row"
    );
    runtime.eval::<usize>("__repeat(1)").unwrap();
    let baseline = collected_used_heap(&mut runtime);
    let kept = runtime.eval::<usize>("__repeat(1000)").unwrap();
    assert_eq!(kept, rows * 1000);
    let after = collected_used_heap(&mut runtime);
    let growth = after.saturating_sub(baseline);
    // Findings are transient: after collection nothing per call is retained.
    // Allow 64 bytes per table row for V8's own transition and IC caches.
    let limit = 64 * rows + 16_384;
    println!("quest junk V8 heap rows={rows} baseline={baseline} after={after} growth={growth} limit={limit}");
    assert!(
        growth <= limit,
        "collected V8 heap grew {growth} bytes; limit {limit}"
    );
}

#[test]
fn oversized_banked_lists_are_rejected_before_any_callback() {
    let mut runtime = setup();
    let rejected: bool = runtime
        .eval(
            r#"(() => {
                let calls = 0;
                try {
                    __rs2b0t_find_quest_junk(Array.from({ length: 289 }, () => ({ id: __table[0].id })), () => { calls++; return 'complete'; });
                    return false;
                } catch (e) {
                    return e instanceof RangeError && calls === 0;
                }
            })()"#,
        )
        .unwrap();
    assert!(rejected);
    let accepted: bool = runtime
        .eval(
            r#"(() => {
                const rows = Array.from({ length: 288 }, () => ({ id: __table[0].id }));
                const found = __rs2b0t_find_quest_junk(rows, () => 'started');
                return found.length >= 1 && found.every((row) => row.droppable === false);
            })()"#,
        )
        .unwrap();
    assert!(accepted, "exactly the selected capacity is accepted");
}
