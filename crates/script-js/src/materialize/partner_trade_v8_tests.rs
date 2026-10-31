use super::*;
use rustyscript::RuntimeOptions;

const CHILD_ENV: &str = "SCRIPT_STABLE_CLOSED_POLL_V8_TEST_CHILD";
const TEST_NAME: &str =
    "partner_trade_v8::tests::stable_closed_poll_state_has_fast_properties_and_bounded_heap";
const PRIVATE_STATE_KEY: &str = "rs2b0t.trade.stableClosedPoll.state";
const CLOSURE_COUNT: usize = 512;
const REPEAT_COUNT: usize = 1_000;

fn setup() -> Runtime {
    crate::platform::bootstrap_test_v8_platform();
    let mut runtime = Runtime::new(RuntimeOptions::default()).unwrap();
    install(&mut runtime).unwrap();
    runtime
        .eval::<()>(
            r#"
            globalThis.__makePredicates = count => Array.from(
                { length: count },
                () => __rs2b0t_partner_trade('stableClosedPoll', 600),
            );
            globalThis.__dropPredicates = () => { globalThis.__predicates = null; };
            globalThis.__hasFastProperties = new Function(
                'value',
                'return %HasFastProperties(value)',
            );
            "#,
        )
        .unwrap();
    runtime
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
fn stable_closed_poll_state_has_fast_properties_and_bounded_heap() {
    if !in_child(TEST_NAME) {
        return;
    }
    v8::V8::set_flags_from_string("--allow-natives-syntax");
    let mut runtime = setup();
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
    let key_name = v8::String::new(&mut scope, PRIVATE_STATE_KEY).unwrap();
    let private_key = v8::Private::for_api(&mut scope, Some(key_name));

    assert_eq!(predicates.length() as usize, CLOSURE_COUNT);
    for index in 0..predicates.length() {
        let predicate = predicates
            .get_index(&mut scope, index)
            .expect("predicate array entry");
        let predicate = v8::Local::<v8::Function>::try_from(predicate).unwrap();
        assert!(
            fast_properties(&mut scope, has_fast, predicate.into()),
            "native predicate {index} has fast properties"
        );
        let state = predicate
            .get_private(&mut scope, private_key)
            .expect("native predicate owns its private state");
        let state = state
            .to_object(&mut scope)
            .expect("private state is an object");
        assert!(
            fast_properties(&mut scope, has_fast, state.into()),
            "native predicate state {index} has fast properties"
        );
    }
    drop(scope);

    let after_kept = collected_used_heap(&mut runtime);
    let retained_growth = after_kept.saturating_sub(baseline);
    // The native callback and its per-predicate state measured about 724 B each.
    let retained_limit = CLOSURE_COUNT * 768 + 32_768;
    println!(
        "stableClosedPoll retained closures={CLOSURE_COUNT} baseline={baseline} after={after_kept} growth={retained_growth} limit={retained_limit}"
    );
    assert!(
        retained_growth <= retained_limit,
        "retaining {CLOSURE_COUNT} native predicates grew V8 heap by {retained_growth} bytes; limit {retained_limit}"
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
        "stableClosedPoll transient closures={REPEAT_COUNT} baseline={warmed} after={after_repeat} growth={transient_growth} limit={transient_limit}"
    );
    assert!(
        transient_growth <= transient_limit,
        "collected creation of {REPEAT_COUNT} native predicates grew V8 heap by {transient_growth} bytes; limit {transient_limit}"
    );
}
