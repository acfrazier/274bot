//! Native batching for the frozen pure `Shop.shopOpBatch` helper.
//!
//! Shop option clicks are USER_EVENT packets (`IfButton.ts:5`, `InvButton.ts:5`).
//! The engine allows five per player tick (`ClientGameProtCategory.ts:6`), and
//! `NetworkPlayer.ts:69,141-143` counts successfully handled USER_EVENT packets.
use super::callback_v8::{self as cb, JsResult};
use rustyscript::Runtime;

const USER_EVENT_OPS_PER_TICK: u32 = 5;
const SHOP_STEPS: [i32; 3] = [10, 5, 1];
const SELL_STACK_STEP: i32 = 10;

pub(super) fn install(runtime: &mut Runtime) -> Result<(), String> {
    cb::install(runtime, "__rs2b0t_shop_op_batch", shop_op_batch)
}

fn shop_op_batch<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = run(scope, &args);
    cb::finish(scope, rv, result);
}

fn run<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let ops = args.get(0);
    if ops.is_string() && ops.to_rust_string_lossy(scope) == "sellStackStep" {
        return Ok(cb::num(scope, f64::from(SELL_STACK_STEP)));
    }
    let ops = v8::Local::<v8::Array>::try_from(ops)
        .map_err(|_| cb::type_error(scope, "ops.some is not a function"))?;
    let verb = cb::to_string(scope, args.get(1))?;
    let mut remaining = args.get(2);
    let batch = v8::Array::new(scope, 0);
    let zero = cb::num(scope, 0.0);
    let mut count = 0;

    while count < USER_EVENT_OPS_PER_TICK && cb::gt(scope, remaining, zero)? {
        let mut selected = None;
        for size in SHOP_STEPS {
            let size_value = cb::num(scope, f64::from(size));
            if cb::le(scope, size_value, remaining)? && has_shop_op(scope, ops, &verb, size)? {
                selected = Some(size);
                break;
            }
        }
        let Some(size) = selected else {
            break;
        };
        let index = shop_op_index(scope, ops, &verb, size)?;
        let value = match index {
            Some(index) => v8::Number::new(scope, f64::from(index)).into(),
            None => v8::Number::new(scope, -1.0).into(),
        };
        cb::catching(scope, |s| batch.set_index(s, count, value))?;
        count += 1;
        let size_value = cb::num(scope, f64::from(size));
        remaining = cb::sub(scope, remaining, size_value)?;
    }
    Ok(batch.into())
}

fn has_shop_op<'s>(
    scope: &mut v8::HandleScope<'s>,
    ops: v8::Local<'s, v8::Array>,
    verb: &str,
    size: i32,
) -> JsResult<'s, bool> {
    let length = ops.length();
    let expected = format!("{verb} {size}");
    for index in 0..length {
        if !cb::catching(scope, |s| ops.has_index(s, index))? {
            continue;
        }
        let op = cb::get_index(scope, ops.into(), index)?;
        if shop_op_matches(scope, op, &expected)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn shop_op_index<'s>(
    scope: &mut v8::HandleScope<'s>,
    ops: v8::Local<'s, v8::Array>,
    verb: &str,
    size: i32,
) -> JsResult<'s, Option<u32>> {
    let length = ops.length();
    let expected = format!("{verb} {size}");
    for index in 0..length {
        let op = cb::get_index(scope, ops.into(), index)?;
        if shop_op_matches(scope, op, &expected)? {
            return Ok(Some(index));
        }
    }
    Ok(None)
}

fn shop_op_matches<'s>(
    scope: &mut v8::HandleScope<'s>,
    op: v8::Local<'s, v8::Value>,
    expected: &str,
) -> JsResult<'s, bool> {
    if op.is_null_or_undefined() {
        return Ok(false);
    }
    if !op.is_string() {
        return Err(cb::type_error(scope, "op.toLowerCase is not a function"));
    }
    Ok(op.to_rust_string_lossy(scope).to_lowercase() == expected)
}
