//! Native sequencing for the published `TreeBot` contract.

use rustyscript::deno_core::v8;
use rustyscript::Runtime;

use crate::materialize::callback_v8::{self as cb, Flow, JsResult, Poll};

/// Install the native TreeBot sequencing callback into one runtime.
pub(crate) fn install(runtime: &mut Runtime) -> Result<(), String> {
    cb::install(runtime, "__rs2b0t_tree_loop", tree_loop)
}

fn tree_loop<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = run(scope, args.get(0));
    cb::finish(scope, rv, result);
}

fn run<'s>(
    scope: &mut v8::HandleScope<'s>,
    bot: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    if !scene_ready_or_log(scope, bot)? {
        return Ok(v8::undefined(scope).into());
    }

    let root = cb::call_method(scope, bot, "root", &[], "TreeBot.root")?;
    let global = scope.get_current_context().global(scope);
    let constructor = cb::get(scope, global.into(), "__rs2b0t_branch_task_ctor")?;
    let constructor = v8::Local::<v8::Object>::try_from(constructor)
        .map_err(|_| cb::type_error(scope, "BranchTask is not a constructor"))?;
    let validate_key = v8::String::new(scope, "validate")
        .ok_or_else(|| cb::type_error(scope, "TreeBot validate key"))?;
    let success_key = v8::String::new(scope, "success")
        .ok_or_else(|| cb::type_error(scope, "TreeBot success key"))?;
    let failure_key = v8::String::new(scope, "failure")
        .ok_or_else(|| cb::type_error(scope, "TreeBot failure key"))?;

    let poll = Poll::default();
    let mut node = root;
    let leaf = loop {
        let flow = {
            let mut iteration_scope = v8::EscapableHandleScope::new(scope);
            let result = iterate(
                &mut iteration_scope,
                node,
                constructor,
                validate_key.into(),
                success_key.into(),
                failure_key.into(),
                &poll,
            );
            cb::iteration(&mut iteration_scope, result)?
        };
        match flow {
            Flow::Continue(Some(next)) => node = next,
            Flow::Break(Some(leaf)) => break leaf,
            Flow::Continue(None) | Flow::Break(None) => {
                return Err(cb::type_error(scope, "TreeBot sequencing lost its node"));
            }
        }
    };

    cb::call_method(scope, leaf, "execute", &[], "LeafTask.execute")
}

fn iterate<'s>(
    scope: &mut v8::HandleScope<'s>,
    node: v8::Local<'s, v8::Value>,
    constructor: v8::Local<'s, v8::Object>,
    validate_key: v8::Local<'s, v8::Value>,
    success_key: v8::Local<'s, v8::Value>,
    failure_key: v8::Local<'s, v8::Value>,
    poll: &Poll,
) -> JsResult<'s, Flow<'s>> {
    poll.check(scope)?;
    let branch = cb::catching(scope, |scope| node.instance_of(scope, constructor))?;
    if !branch {
        return Ok(Flow::Break(Some(node)));
    }

    let valid = call_cached(scope, node, validate_key, "BranchTask.validate")?;
    let next = if cb::truthy(scope, valid) {
        call_cached(scope, node, success_key, "BranchTask.success")?
    } else {
        call_cached(scope, node, failure_key, "BranchTask.failure")?
    };
    Ok(Flow::Continue(Some(next)))
}

fn call_cached<'s>(
    scope: &mut v8::HandleScope<'s>,
    target: v8::Local<'s, v8::Value>,
    key: v8::Local<'s, v8::Value>,
    label: &str,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let function = cb::catching(scope, |scope| target.to_object(scope)?.get(scope, key))?;
    cb::call(scope, function, target, &[], label)
}

fn scene_ready_or_log<'s>(
    scope: &mut v8::HandleScope<'s>,
    bot: v8::Local<'s, v8::Value>,
) -> JsResult<'s, bool> {
    let global = scope.get_current_context().global(scope);
    let host = cb::get(scope, global.into(), "__rs2b0t_host")?;
    let snapshot = cb::get(scope, host, "snapshot")?;
    let ingame = cb::get(scope, snapshot, "ingame")?;
    let scene_state = cb::get(scope, snapshot, "scene_state")?;
    if ingame.is_true() && scene_state.is_number() && scene_state.number_value(scope) == Some(2.0) {
        return Ok(true);
    }

    let state = if scene_state.is_number() {
        scene_state.number_value(scope).unwrap_or(0.0)
    } else {
        0.0
    };
    let last = cb::get(scope, bot, "lastSceneWaitLogAt")?;
    let performance = cb::get(scope, global.into(), "performance")?;
    let now = cb::call_method(scope, performance, "now", &[], "performance.now")?;
    let elapsed = cb::sub(scope, now, last)?;
    let threshold = v8::Number::new(scope, 2000.0).into();
    if cb::gt(scope, elapsed, threshold)? {
        let key = cb::string(scope, "lastSceneWaitLogAt");
        let target = v8::Local::<v8::Object>::try_from(bot)
            .map_err(|_| cb::type_error(scope, "TreeBot is not an object"))?;
        let stored = cb::catching(scope, |scope| target.set(scope, key, now))?;
        if !stored {
            return Err(cb::type_error(
                scope,
                "Cannot assign to read only property 'lastSceneWaitLogAt'",
            ));
        }
        let message = format!("waiting for scene (state={state}) before tree");
        let message = cb::string(scope, &message);
        cb::call_method(scope, bot, "log", &[message], "TreeBot.log")?;
    }
    Ok(false)
}

#[cfg(all(test, feature = "memory-profile"))]
mod tests {
    use rustyscript::deno_core::v8;
    use rustyscript::{Module, Runtime, RuntimeOptions};

    const CHILD_ENV: &str = "SCRIPT_COMPAT_V1_BOT_SHAPE_TEST_CHILD";
    const TEST_NAME: &str =
        "runtime::bindings::tree_bot::tests::bot_objects_have_fast_properties_and_bounded_heap";

    fn setup() -> Runtime {
        crate::runtime::platform::bootstrap_test_v8_platform();
        let mut runtime = Runtime::new(RuntimeOptions::default()).unwrap();
        runtime.eval::<()>(crate::shim::PRELUDE).unwrap();
        super::install(&mut runtime).unwrap();
        let bot = Module::new(crate::shim::BOT_MODULE, include_str!("../../shim/bot.js"));
        let main = Module::new(
            "/compat_v1_bot_shape_test.js",
            format!(
                r#"
import {{ LoopingBot, TaskBot, TreeBot, BranchTask, LeafTask }} from '{}';
const fast = new Function('value', 'return %HasFastProperties(value)');
const values = () => [
    new LoopingBot(),
    new TaskBot(),
    new TreeBot(),
    new BranchTask(),
    new LeafTask()
];
globalThis.__bot_fast = () => values().every(fast);
globalThis.__bot_repeat = count => {{
    const rows = [];
    for (let i = 0; i < count; i++) rows.push(...values());
    globalThis.__bot_last = rows;
    return rows.length;
}};
globalThis.__bot_drop = () => {{ globalThis.__bot_last = null; }};
"#,
                crate::shim::BOT_MODULE
            ),
        );
        runtime.load_modules(&main, vec![&bot]).unwrap();
        runtime
    }

    fn collected_used_heap(runtime: &mut Runtime) -> usize {
        let isolate = runtime.deno_runtime().v8_isolate();
        isolate.low_memory_notification();
        isolate.get_heap_statistics().used_heap_size()
    }

    fn in_child() -> bool {
        if std::env::var_os(CHILD_ENV).is_some() {
            return true;
        }
        let output = std::process::Command::new(std::env::current_exe().expect("test executable"))
            .args(["--exact", TEST_NAME, "--nocapture"])
            .env(CHILD_ENV, "1")
            .output()
            .expect("run isolated V8 native-syntax test");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            output.status.success() && stdout.contains("1 passed"),
            "isolated V8 bot test failed\nstdout: {stdout}\nstderr: {stderr}"
        );
        false
    }

    #[test]
    fn bot_objects_have_fast_properties_and_bounded_heap() {
        if !in_child() {
            return;
        }
        v8::V8::set_flags_from_string("--allow-natives-syntax");
        let mut runtime = setup();
        assert!(runtime.eval::<bool>("__bot_fast()").unwrap());
        assert_eq!(runtime.eval::<usize>("__bot_repeat(1)").unwrap(), 5);
        runtime.eval::<()>("__bot_drop()").unwrap();
        let baseline = collected_used_heap(&mut runtime);
        assert_eq!(runtime.eval::<usize>("__bot_repeat(1000)").unwrap(), 5000);
        runtime.eval::<()>("__bot_drop()").unwrap();
        let after = collected_used_heap(&mut runtime);
        let growth = after.saturating_sub(baseline);
        let limit = 16_384;
        println!(
            "compat v1 Bot V8 heap baseline={baseline} after={after} growth={growth} limit={limit}"
        );
        assert!(
            growth <= limit,
            "collected Bot object heap grew {growth} bytes; limit {limit}"
        );
    }
}
