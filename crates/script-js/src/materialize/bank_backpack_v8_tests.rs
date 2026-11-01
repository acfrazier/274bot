use super::*;
use crate::observed::ItemRow;
use rustyscript::{Module, RuntimeOptions};
use std::rc::Rc;

const CHILD_ENV: &str = "SCRIPT_NORMAL_BACKPACK_SHAPE_TEST_CHILD";
const TEST_NAME: &str =
    "bank_backpack_v8::tests::normal_backpack_rows_have_fast_properties_and_bound_v8_heap";

const BANK_KERNEL: &str = r#"
export function runMachine() { throw new Error('unused'); }
export function snap() { return {}; }
export function queue() { return {}; }
export function proxy(_name, methods) { return methods; }
"#;
const BANK_ENTRY: &str = r#"
import '/rs2b0t/bot/shim/_kernel.js';
import { Bank } from '/rs2b0t/bot/api/bank/Bank.js';
import { reader } from '/rs2b0t/bot/adapter/ClientAdapter.js';
globalThis.__BankForTest = Bank;
globalThis.__ReaderForTest = reader;
"#;

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
globalThis.__bankFrozen = () => {
    const rows = __BankForTest.items();
    return Array.isArray(rows) && Object.isFrozen(rows)
        && rows.every((row) => Array.isArray(row.ops)
            && Object.isFrozen(row) && Object.isFrozen(row.ops));
};
globalThis.__bankFast = () => {
    const fast = new Function('value', 'return %HasFastProperties(value)');
    const rows = __BankForTest.items();
    return Array.isArray(rows) && rows.length === 288
        && __bankFrozen() && rows.every(fast);
};
globalThis.__bankRepeat = (count) => {
    let kept = 0;
    for (let i = 0; i < count; i++) kept += __BankForTest.items().length;
    return kept;
};
globalThis.__readerBankFrozen = () => {
    const rows = __ReaderForTest.bankItems();
    return Array.isArray(rows) && Object.isFrozen(rows)
        && rows.every((row) => Array.isArray(row.ops)
            && Object.isFrozen(row) && Object.isFrozen(row.ops));
};
globalThis.__readerBankFast = () => {
    const fast = new Function('value', 'return %HasFastProperties(value)');
    const rows = __ReaderForTest.bankItems();
    return Array.isArray(rows) && rows.length === 288
        && __readerBankFrozen() && rows.every(fast);
};
globalThis.__readerBankRepeat = (count) => {
    let kept = 0;
    for (let i = 0; i < count; i++) kept += __ReaderForTest.bankItems().length;
    return kept;
};
const bankStrictWritesThrow = new Function('rows', `
    'use strict';
    const t = (operation) => {
        try { operation(); return false; }
        catch (error) { return error instanceof TypeError; }
    };
    return {
        push: t(() => rows.push(null)),
        index: t(() => { rows[0] = null; }),
        sort: t(() => rows.sort(() => 0)),
        row: t(() => { rows[0].count = 999; }),
        ops: t(() => rows[0].ops.push('Drop')),
        cert: t(() => { rows[0].cert = 1; }),
    };
`);
globalThis.__bankWritesThrow = () => {
    const rows = __BankForTest.items();
    return Array.isArray(rows) && Object.isFrozen(rows)
        && Object.isFrozen(rows[0])
        && Object.values(bankStrictWritesThrow(rows)).every(Boolean);
};
globalThis.__readerBankWritesThrow = () => {
    const rows = __ReaderForTest.bankItems();
    return Array.isArray(rows) && Object.isFrozen(rows)
        && Object.isFrozen(rows[0])
        && Object.values(bankStrictWritesThrow(rows)).every(Boolean);
};
"#;

fn load_bank_api(runtime: &mut Runtime) {
    let bank = Module::new(
        "/rs2b0t/bot/api/bank/Bank.js",
        include_str!("../shim/bank.js"),
    );
    let adapter = Module::new(
        "/rs2b0t/bot/adapter/ClientAdapter.js",
        include_str!("../shim/client_adapter.js"),
    );
    let kernel = Module::new("/rs2b0t/bot/shim/_kernel.js", BANK_KERNEL);
    let entry = Module::new("/bank_backpack_test.js", BANK_ENTRY);
    // Both shims import `_kernel.js`; register it before the importers.
    runtime
        .load_modules(&entry, vec![&kernel, &bank, &adapter])
        .unwrap();
}

fn setup() -> Runtime {
    crate::platform::bootstrap_test_v8_platform();
    let mut runtime = Runtime::new(RuntimeOptions::default()).unwrap();
    cb::install_ops(&mut runtime).unwrap();
    install(&mut runtime).unwrap();
    super::super::bank_tasks_v8::install(&mut runtime).unwrap();

    load_bank_api(&mut runtime);
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
    // A warm-up window first: V8's own transition and IC caches settle there,
    // so the measured window sees only what the rows themselves retain.
    runtime.eval::<usize>("__repeat(1000)").unwrap();
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

const BANK_FAST_CHILD_ENV: &str = "SCRIPT_BANK_ITEMS_FAST_TEST_CHILD";
const BANK_FAST_TEST_NAME: &str =
    "bank_backpack_v8::tests::bank_288_item_rows_have_fast_properties";
const BANK_HEAP_CHILD_ENV: &str = "SCRIPT_BANK_ITEMS_HEAP_TEST_CHILD";
const BANK_HEAP_TEST_NAME: &str =
    "bank_backpack_v8::tests::bank_288_item_rows_have_bounded_v8_heap";

/// A full 288-row bank (`interface_bank/configs/bank.constant:1`).
fn full_bank() -> Vec<ItemRow> {
    let ops: Rc<[Rc<str>]> = [
        "Withdraw-1",
        "Withdraw-5",
        "Withdraw-10",
        "Withdraw-All",
        "Withdraw-X",
    ]
    .map(Rc::from)
    .into();
    (0..288)
        .map(|slot| ItemRow {
            id: 1000 + slot,
            count: slot + 1,
            name: Some(Rc::from("Logs")),
            ops: Rc::clone(&ops),
            slot: Some(slot),
            component_id: Some(5382),
            ..ItemRow::default()
        })
        .collect()
}

fn post_bank(
    tick: u64,
    rows: Vec<ItemRow>,
    open: bool,
    bank_generation: u64,
    snapshot_generation: i64,
) {
    crate::observed::post(tick, |post| {
        post.ingame(true)
            .bank(rows)
            .bank_open(open)
            .bank_generation(bank_generation)
            .bank_snapshot_generation(snapshot_generation);
    });
}

fn post_bank_without_snapshot_generation(tick: u64, rows: Vec<ItemRow>, bank_generation: u64) {
    crate::observed::post(tick, |post| {
        post.ingame(true)
            .bank(rows)
            .bank_open(true)
            .bank_generation(bank_generation);
    });
}

fn run_bank_v8_test_child(environment: &str, test_name: &str) -> bool {
    if std::env::var_os(environment).is_some() {
        return true;
    }
    let output = std::process::Command::new(std::env::current_exe().expect("test executable"))
        .args(["--exact", test_name, "--nocapture"])
        .env(environment, "1")
        .output()
        .expect("run isolated V8-native-syntax test");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success() && stdout.contains("1 passed"),
        "isolated V8 test failed\nstdout: {stdout}\nstderr: {stderr}"
    );
    print!("{stdout}");
    false
}

#[test]
fn bank_288_item_rows_have_fast_properties() {
    if !run_bank_v8_test_child(BANK_FAST_CHILD_ENV, BANK_FAST_TEST_NAME) {
        return;
    }
    v8::V8::set_flags_from_string("--allow-natives-syntax");
    crate::observed::on_reset();
    let mut runtime = setup();
    post_bank(1, full_bank(), true, 1, 1);
    assert!(
        runtime
            .eval::<bool>("__bankFast() && __readerBankFast()")
            .unwrap(),
        "288 frozen native bank rows have fast properties"
    );
    crate::observed::on_reset();
}

#[test]
fn bank_288_item_rows_have_bounded_v8_heap() {
    if !run_bank_v8_test_child(BANK_HEAP_CHILD_ENV, BANK_HEAP_TEST_NAME) {
        return;
    }
    v8::V8::set_flags_from_string("--allow-natives-syntax");
    crate::observed::on_reset();
    let mut runtime = setup();
    post_bank(1, full_bank(), true, 1, 1);
    assert_eq!(
        runtime
            .eval::<usize>("__bankRepeat(5) + __readerBankRepeat(5)")
            .unwrap(),
        2_880
    );
    let baseline = collected_used_heap(&mut runtime);
    const REPEATS: usize = 20;
    let repeated = runtime
        .eval::<usize>(&format!(
            "__bankRepeat({REPEATS}) + __readerBankRepeat({REPEATS})"
        ))
        .unwrap();
    assert_eq!(repeated, 2 * 288 * REPEATS);
    let after = collected_used_heap(&mut runtime);
    let growth = after.saturating_sub(baseline);
    // Repeated reads reuse both frozen arrays for this bank snapshot generation.
    let limit = 16_384;
    println!(
        "bank items V8 heap rows=288 repeats={REPEATS} baseline={baseline} \
         after={after} growth={growth} limit={limit}"
    );
    assert!(
        growth <= limit,
        "collected V8 heap grew {growth} bytes; limit {limit}"
    );
    crate::observed::on_reset();
}

/// `Bank.items()` keeps the legacy `noted`/`cert` projection; the client
/// reader keeps its open-bank `InvItemSnapshot` projection. Both are frozen
/// and reused by generation, while reader rows filter missing slots/negative
/// ids and the legacy snapshot preserves every posted row.
#[test]
fn bank_items_project_open_bank_rows_and_refresh_by_generation() {
    crate::observed::on_reset();
    let mut runtime = setup();
    let ops: Rc<[Rc<str>]> = ["Withdraw-1"].map(Rc::from).into();
    let rows = vec![
        ItemRow {
            id: 2399,
            count: 1,
            name: Some(Rc::from("Key")),
            ops: Rc::clone(&ops),
            noted: true,
            cert: 105,
            slot: Some(0),
            component_id: Some(5382),
        },
        ItemRow {
            id: 995,
            count: 40,
            name: None,
            noted: false,
            cert: 106,
            slot: Some(1),
            component_id: None,
            ..ItemRow::default()
        },
        ItemRow {
            id: 1511,
            count: 1,
            slot: None,
            ..ItemRow::default()
        },
        ItemRow {
            id: -1,
            count: 1,
            slot: Some(3),
            ..ItemRow::default()
        },
    ];
    post_bank_without_snapshot_generation(1, rows.clone(), 10);
    runtime
        .eval::<()>(
            "globalThis.__unversionedBankItems = __BankForTest.items(); \
             globalThis.__unversionedReaderBankItems = __ReaderForTest.bankItems()",
        )
        .unwrap();
    assert!(runtime
        .eval::<bool>(
            "__BankForTest.items() !== __unversionedBankItems \
             && __ReaderForTest.bankItems() !== __unversionedReaderBankItems",
        )
        .unwrap());

    post_bank(2, rows.clone(), true, 10, 20);
    assert_eq!(
        runtime
            .eval::<String>("JSON.stringify(__BankForTest.items())")
            .unwrap(),
        r#"[{"name":"Key","count":1,"id":2399,"slot":0,"comId":5382,"ops":["Withdraw-1"],"noted":true,"cert":105},{"name":null,"count":40,"id":995,"slot":1,"comId":-1,"ops":[],"noted":false,"cert":106},{"name":null,"count":1,"id":1511,"slot":-1,"comId":-1,"ops":[],"noted":false,"cert":0},{"name":null,"count":1,"id":-1,"slot":3,"comId":-1,"ops":[],"noted":false,"cert":0}]"#
    );
    assert_eq!(
        runtime
            .eval::<String>("JSON.stringify(__ReaderForTest.bankItems())")
            .unwrap(),
        r#"[{"slot":0,"id":2399,"name":"Key","count":1,"noted":true,"ops":["Withdraw-1"],"comId":5382},{"slot":1,"id":995,"name":null,"count":40,"noted":false,"ops":[],"comId":-1}]"#
    );
    assert!(runtime
        .eval::<bool>("__bankFrozen() && __readerBankFrozen()")
        .unwrap());
    assert!(runtime
        .eval::<bool>("__bankWritesThrow() && __readerBankWritesThrow()")
        .unwrap());

    runtime
        .eval::<()>(
            "globalThis.__firstBankItems = __BankForTest.items(); \
             globalThis.__firstReaderBankItems = __ReaderForTest.bankItems()",
        )
        .unwrap();
    assert!(runtime
        .eval::<bool>(
            "__BankForTest.items() === __firstBankItems \
             && __ReaderForTest.bankItems() === __firstReaderBankItems \
             && __firstBankItems !== __firstReaderBankItems",
        )
        .unwrap());

    // The review probe posted twice more with identical generations. Both
    // public arrays must retain their exact identities across those posts.
    post_bank(3, rows.clone(), true, 10, 20);
    post_bank(4, rows.clone(), true, 10, 20);
    assert!(runtime
        .eval::<bool>(
            "__BankForTest.items() === __firstBankItems \
             && __ReaderForTest.bankItems() === __firstReaderBankItems",
        )
        .unwrap());
    assert!(runtime
        .eval::<bool>("__bankWritesThrow() && __readerBankWritesThrow()")
        .unwrap());

    post_bank(5, rows.clone(), true, 10, 21);
    assert!(runtime
        .eval::<bool>(
            "__BankForTest.items() !== __firstBankItems \
             && __ReaderForTest.bankItems() !== __firstReaderBankItems",
        )
        .unwrap());
    runtime
        .eval::<()>(
            "globalThis.__newBankItems = __BankForTest.items(); \
             globalThis.__newReaderBankItems = __ReaderForTest.bankItems()",
        )
        .unwrap();
    post_bank(6, rows, true, 11, 21);
    assert!(runtime
        .eval::<bool>(
            "__BankForTest.items() !== __newBankItems \
             && __ReaderForTest.bankItems() !== __newReaderBankItems",
        )
        .unwrap());

    post_bank(7, Vec::new(), false, 12, -1);
    assert_eq!(
        runtime
            .eval::<String>("JSON.stringify(__BankForTest.items())")
            .unwrap(),
        "[]"
    );
    assert_eq!(
        runtime
            .eval::<String>("JSON.stringify(__ReaderForTest.bankItems())")
            .unwrap(),
        "[]"
    );
    assert!(runtime
        .eval::<bool>("__bankFrozen() && __readerBankFrozen()")
        .unwrap());
    runtime
        .eval::<()>(
            "globalThis.__closedBankItems = __BankForTest.items(); \
             globalThis.__closedReaderBankItems = __ReaderForTest.bankItems()",
        )
        .unwrap();
    assert!(runtime
        .eval::<bool>(
            "__BankForTest.items() !== __closedBankItems \
             && __ReaderForTest.bankItems() === __closedReaderBankItems",
        )
        .unwrap());
    crate::observed::on_reset();
}
