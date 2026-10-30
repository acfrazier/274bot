use super::*;
use crate::observed::ItemRow;
use rustyscript::RuntimeOptions;
use std::rc::Rc;

const CHILD_ENV: &str = "SCRIPT_NORMAL_BACKPACK_SHAPE_TEST_CHILD";
const TEST_NAME: &str =
    "bank_backpack_v8::tests::normal_backpack_rows_have_fast_properties_and_bound_v8_heap";

const SETUP: &str = r#"
globalThis.__fast = () => {
    const fast = new Function('value', 'return %HasFastProperties(value)');
    const rows = __rs2b0t_bank_normal_backpack();
    return Array.isArray(rows) && rows.length === 28 && rows.every(fast);
};
globalThis.__repeat = (count) => {
    let kept = 0;
    for (let i = 0; i < count; i++) kept += __rs2b0t_bank_normal_backpack().length;
    return kept;
};
"#;

fn setup() -> Runtime {
    crate::platform::bootstrap_test_v8_platform();
    let mut runtime = Runtime::new(RuntimeOptions::default()).unwrap();
    cb::install_ops(&mut runtime).unwrap();
    install(&mut runtime).unwrap();
    runtime.eval::<()>(SETUP).unwrap();
    runtime
}

fn full_pack() -> Vec<ItemRow> {
    (0..28)
        .map(|slot| ItemRow {
            id: 1511,
            count: 1,
            name: Some(Rc::from("Logs")),
            slot: Some(slot),
            component_id: Some(3214),
            ..ItemRow::default()
        })
        .collect()
}

fn post(tick: u64, rows: Vec<ItemRow>, size: i32, ready: bool, bank_open: bool) {
    crate::observed::post(tick, |post| {
        post.ingame(true)
            .inv(rows)
            .inv_size(size)
            .inv_ready(ready)
            .bank_open(bank_open);
    });
}

fn collected_used_heap(runtime: &mut Runtime) -> usize {
    let isolate = runtime.deno_runtime().v8_isolate();
    isolate.low_memory_notification();
    isolate.get_heap_statistics().used_heap_size()
}

#[test]
fn normal_backpack_rows_have_fast_properties_and_bound_v8_heap() {
    if std::env::var_os(CHILD_ENV).is_none() {
        let output = std::process::Command::new(std::env::current_exe().expect("test executable"))
            .args(["--exact", TEST_NAME, "--nocapture"])
            .env(CHILD_ENV, "1")
            .output()
            .expect("run isolated V8-native-syntax test");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            output.status.success() && stdout.contains("1 passed"),
            "isolated V8 test failed\nstdout: {stdout}\nstderr: {stderr}"
        );
        return;
    }
    v8::V8::set_flags_from_string("--allow-natives-syntax");
    crate::observed::on_reset();
    let mut runtime = setup();
    post(1, full_pack(), 28, true, false);
    assert!(
        runtime.eval::<bool>("__fast()").unwrap(),
        "28 natively built rows with fast properties"
    );
    runtime.eval::<usize>("__repeat(1)").unwrap();
    let baseline = collected_used_heap(&mut runtime);
    assert_eq!(runtime.eval::<usize>("__repeat(1000)").unwrap(), 28_000);
    let after = collected_used_heap(&mut runtime);
    let growth = after.saturating_sub(baseline);
    // Rows are transient; allow V8's own transition/IC caches only.
    let limit = 16_384;
    println!("normal backpack V8 heap rows=28 baseline={baseline} after={after} growth={growth} limit={limit}");
    assert!(
        growth <= limit,
        "collected V8 heap grew {growth} bytes; limit {limit}"
    );
    crate::observed::on_reset();
}

#[test]
fn normal_backpack_is_null_unless_closed_ready_and_28_slots() {
    crate::observed::on_reset();
    let mut runtime = setup();
    post(1, full_pack(), 28, false, false);
    assert!(runtime
        .eval::<bool>("__rs2b0t_bank_normal_backpack() === null")
        .unwrap());
    post(2, full_pack(), 28, true, true);
    assert!(runtime
        .eval::<bool>("__rs2b0t_bank_normal_backpack() === null")
        .unwrap());
    post(3, Vec::new(), 0, true, false);
    assert!(runtime
        .eval::<bool>("__rs2b0t_bank_normal_backpack() === null")
        .unwrap());
    post(4, Vec::new(), 28, true, false);
    assert!(runtime
        .eval::<bool>("JSON.stringify(__rs2b0t_bank_normal_backpack()) === '[]'")
        .unwrap());
    crate::observed::on_reset();
}
