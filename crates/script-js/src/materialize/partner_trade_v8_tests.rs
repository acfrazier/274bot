use super::*;
use rustyscript::{Module, RuntimeOptions};

const CHILD_ENV: &str = "SCRIPT_STABLE_CLOSED_POLL_V8_TEST_CHILD";
const TEST_NAME: &str =
    "partner_trade_v8::tests::stable_closed_poll_wrapper_has_fast_properties_and_bounded_heap";
const CLOSURE_COUNT: usize = 512;
const REPEAT_COUNT: usize = 1_000;
// 512 wrapped predicates grew 108,776 bytes; allow 8 KiB for V8/allocator variance.
const RETAINED_HEAP_LIMIT: usize = 116_968;
const TEST_KERNEL: &str = r#"
export function notImpl() { throw new Error('not used by stableClosedPoll tests'); }
export function runMachine() { throw new Error('not used by stableClosedPoll tests'); }
"#;

fn load_trade_shim(runtime: &mut Runtime) {
    let kernel = Module::new("/rs2b0t/bot/shim/_kernel.js", TEST_KERNEL);
    let drive = Module::new(
        "/rs2b0t/bot/api/drive/drive_partner_trade.js",
        include_str!("../shim/drive_partner_trade.js"),
    );
    let entry = Module::new(
        "/rs2b0t/bot/api/drive/partner_trade_test.js",
        r#"
import { stableClosedPoll, tradeScreenState } from './drive_partner_trade.js';
globalThis.__stableClosedPoll = stableClosedPoll;
globalThis.__tradeScreenState = tradeScreenState;
"#,
    );
    runtime.load_modules(&entry, vec![&kernel, &drive]).unwrap();
}

fn setup() -> Runtime {
    crate::platform::bootstrap_test_v8_platform();
    let mut runtime = Runtime::new(RuntimeOptions::default()).unwrap();
    install(&mut runtime).unwrap();
    load_trade_shim(&mut runtime);
    runtime
        .eval::<()>(
            r#"
            globalThis.__makePredicates = count => Array.from(
                { length: count },
                () => __stableClosedPoll(600),
            );
            globalThis.__dropPredicates = () => { globalThis.__predicates = null; };
            "#,
        )
        .unwrap();
    runtime
}

fn post_trade_screen(tick: u64, offer: bool, confirm: bool) {
    crate::observed::post(tick, |post| {
        post.session(true)
            .trade_offer_open(offer)
            .trade_confirm_open(confirm);
    });
}

#[test]
fn stable_closed_poll_reads_scene_and_calls_clock_once_per_closed_poll() {
    crate::observed::on_reset();
    let mut runtime = setup();
    post_trade_screen(10, true, false);
    runtime
        .eval::<()>(
            r#"
            globalThis.__nativePartnerTrade = globalThis.__rs2b0t_partner_trade;
            globalThis.__nativePartnerCalls = 0;
            globalThis.__nativePollCalls = 0;
            globalThis.__rs2b0t_partner_trade = (...args) => {
                globalThis.__nativePartnerCalls++;
                const result = __nativePartnerTrade(...args);
                if (args[0] === 'stableClosedPoll' && typeof result === 'function') {
                    return (...pollArgs) => {
                        globalThis.__nativePollCalls++;
                        return result(...pollArgs);
                    };
                }
                return result;
            };
            globalThis.__clockCalls = 0;
            globalThis.__clockNow = 1000;
            globalThis.__poll = __stableClosedPoll(1200, () => {
                globalThis.__clockCalls++;
                return globalThis.__clockNow;
            });
            globalThis.__pollFactoryCalls = globalThis.__nativePartnerCalls;
            globalThis.__nativePartnerCalls = 0;
            "#,
        )
        .unwrap();

    assert_eq!(
        runtime.eval::<String>("__tradeScreenState()").unwrap(),
        "offer"
    );
    assert_eq!(runtime.eval::<u32>("__nativePartnerCalls").unwrap(), 1);
    assert_eq!(runtime.eval::<u32>("__pollFactoryCalls").unwrap(), 1);
    runtime
        .eval::<()>("globalThis.__nativePartnerCalls = 0")
        .unwrap();
    assert!(!runtime.eval::<bool>("__poll()").unwrap());
    assert_native_poll_once(&mut runtime);
    assert_eq!(runtime.eval::<u32>("__clockCalls").unwrap(), 0);

    post_trade_screen(20, false, false);
    assert!(!runtime.eval::<bool>("__poll()").unwrap());
    assert_native_poll_once(&mut runtime);
    assert_eq!(runtime.eval::<u32>("__clockCalls").unwrap(), 1);
    runtime.eval::<()>("globalThis.__clockNow = 2100").unwrap();
    assert!(!runtime.eval::<bool>("__poll()").unwrap());
    assert_native_poll_once(&mut runtime);
    runtime.eval::<()>("globalThis.__clockNow = 2200").unwrap();
    assert!(runtime.eval::<bool>("__poll()").unwrap());
    assert_native_poll_once(&mut runtime);
    assert_eq!(runtime.eval::<u32>("__clockCalls").unwrap(), 3);

    post_trade_screen(21, false, true);
    assert_eq!(
        runtime.eval::<String>("__tradeScreenState()").unwrap(),
        "confirm"
    );
    assert_eq!(runtime.eval::<u32>("__nativePartnerCalls").unwrap(), 1);
    runtime
        .eval::<()>("globalThis.__nativePartnerCalls = 0")
        .unwrap();
    assert!(!runtime.eval::<bool>("__poll()").unwrap());
    assert_native_poll_once(&mut runtime);
    assert_eq!(runtime.eval::<u32>("__clockCalls").unwrap(), 3);

    post_trade_screen(22, false, false);
    runtime.eval::<()>("globalThis.__clockNow = 5000").unwrap();
    assert!(!runtime.eval::<bool>("__poll()").unwrap());
    assert_native_poll_once(&mut runtime);
    runtime.eval::<()>("globalThis.__clockNow = 6200").unwrap();
    assert!(runtime.eval::<bool>("__poll()").unwrap());
    assert_native_poll_once(&mut runtime);
    assert_eq!(
        runtime.eval::<String>("__tradeScreenState()").unwrap(),
        "closed"
    );
    assert_eq!(runtime.eval::<u32>("__nativePartnerCalls").unwrap(), 1);
    runtime
        .eval::<()>("globalThis.__nativePartnerCalls = 0")
        .unwrap();

    post_trade_screen(30, false, false);
    runtime
        .eval::<()>("globalThis.__tickPoll = __stableClosedPoll(600)")
        .unwrap();
    assert_eq!(runtime.eval::<u32>("__nativePartnerCalls").unwrap(), 1);
    runtime
        .eval::<()>("globalThis.__nativePartnerCalls = 0")
        .unwrap();
    assert!(!runtime.eval::<bool>("__tickPoll()").unwrap());
    assert_native_poll_once(&mut runtime);
    post_trade_screen(31, false, false);
    assert!(runtime.eval::<bool>("__tickPoll()").unwrap());
    assert_native_poll_once(&mut runtime);
}

fn assert_native_poll_once(runtime: &mut Runtime) {
    assert_eq!(runtime.eval::<u32>("__nativePartnerCalls").unwrap(), 0);
    assert_eq!(runtime.eval::<u32>("__nativePollCalls").unwrap(), 1);
    runtime
        .eval::<()>("globalThis.__nativePollCalls = 0")
        .unwrap();
}

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

fn collected_used_heap(runtime: &mut Runtime) -> usize {
    let isolate = runtime.deno_runtime().v8_isolate();
    isolate.low_memory_notification();
    isolate.low_memory_notification();
    isolate.get_heap_statistics().used_heap_size()
}

fn fast_properties(
    scope: &mut v8::HandleScope,
    predicate: v8::Local<v8::Function>,
    value: v8::Local<v8::Value>,
) -> bool {
    let receiver = v8::undefined(scope).into();
    predicate
        .call(scope, receiver, &[value])
        .expect("V8 fast-properties intrinsic returns a value")
        .is_true()
}

#[test]
fn stable_closed_poll_wrapper_has_fast_properties_and_bounded_heap() {
    if !in_child(TEST_NAME) {
        return;
    }
    v8::V8::set_flags_from_string("--allow-natives-syntax");
    let mut runtime = setup();
    runtime
        .eval::<()>(
            "globalThis.__hasFastProperties = new Function('value', 'return %HasFastProperties(value)')",
        )
        .unwrap();
    let baseline = collected_used_heap(&mut runtime);
    runtime
        .eval::<()>("globalThis.__predicates = __makePredicates(512)")
        .unwrap();

    let context = runtime.deno_runtime().main_context();
    let mut scope = runtime.deno_runtime().handle_scope();
    let global = context.open(&mut scope).global(&mut scope);
    let name = v8::String::new(&mut scope, "__predicates").unwrap();
    let predicates =
        v8::Local::<v8::Array>::try_from(global.get(&mut scope, name.into()).unwrap()).unwrap();
    let name = v8::String::new(&mut scope, "__hasFastProperties").unwrap();
    let has_fast =
        v8::Local::<v8::Function>::try_from(global.get(&mut scope, name.into()).unwrap()).unwrap();
    assert_eq!(predicates.length() as usize, CLOSURE_COUNT);
    for index in 0..predicates.length() {
        let predicate = predicates
            .get_index(&mut scope, index)
            .expect("predicate array entry");
        let predicate = v8::Local::<v8::Function>::try_from(predicate).unwrap();
        assert!(
            fast_properties(&mut scope, has_fast, predicate.into()),
            "shim wrapper {index} has fast properties"
        );
    }
    drop(scope);

    let after_kept = collected_used_heap(&mut runtime);
    let retained_growth = after_kept.saturating_sub(baseline);
    let retained_limit = RETAINED_HEAP_LIMIT;
    println!(
        "stableClosedPoll retained wrappers={CLOSURE_COUNT} baseline={baseline} after={after_kept} growth={retained_growth} limit={retained_limit}"
    );
    assert!(
        retained_growth <= retained_limit,
        "retaining {CLOSURE_COUNT} stableClosedPoll wrappers grew V8 heap by {retained_growth} bytes; limit {retained_limit}"
    );

    runtime.eval::<()>("__dropPredicates()").unwrap();
    collected_used_heap(&mut runtime);
    runtime
        .eval::<()>("globalThis.__predicates = __makePredicates(1000); __dropPredicates()")
        .unwrap();
    collected_used_heap(&mut runtime);
    let warmed = collected_used_heap(&mut runtime);
    runtime
        .eval::<()>("globalThis.__predicates = __makePredicates(1000); __dropPredicates()")
        .unwrap();
    let after_repeat = collected_used_heap(&mut runtime);
    let transient_growth = after_repeat.saturating_sub(warmed);
    let transient_limit = 65_536;
    println!(
        "stableClosedPoll transient wrappers={REPEAT_COUNT} baseline={warmed} after={after_repeat} growth={transient_growth} limit={transient_limit}"
    );
    assert!(
        transient_growth <= transient_limit,
        "collected creation of {REPEAT_COUNT} stableClosedPoll wrappers grew V8 heap by {transient_growth} bytes; limit {transient_limit}"
    );
}
