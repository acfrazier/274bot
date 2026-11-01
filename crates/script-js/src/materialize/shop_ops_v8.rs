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
    cb::install(runtime, "__rs2b0t_shop_op_batch", shop_op_batch)?;
    cb::install(runtime, "__rs2b0t_shop_price", shop_price)
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

fn shop_price<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = run_price(scope, &args);
    cb::finish(scope, rv, result);
}

fn run_price<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let export = cb::to_string(scope, args.get(0))?;
    let price = match export.as_str() {
        "unitPrice" => {
            let item = args.get(1);
            let shop = args.get(2);
            let stock = cb::number(scope, args.get(3))?;
            let baseline_value = cb::get(scope, item, "baseline")?;
            let baseline = cb::number(scope, baseline_value)?;
            let delta_value = cb::get(scope, shop, "delta")?;
            let delta = cb::number(scope, delta_value)?;
            let sell_value = cb::get(scope, shop, "sell")?;
            let sell = cb::number(scope, sell_value)?;
            let cost_value = cb::get(scope, item, "cost")?;
            let cost = cb::number(scope, cost_value)?;
            match (
                price_i32(cost),
                price_i32(baseline),
                price_i32(sell),
                price_i32(delta),
                price_i32(stock),
            ) {
                (Some(cost), Some(baseline), Some(sell), Some(delta), Some(stock)) => {
                    api::shop_facts::unit_price_values(cost, baseline, sell, delta, stock) as f64
                }
                _ => f64::NAN,
            }
        }
        "shopBuyPrice" => {
            let cost = cb::number(scope, args.get(1))?;
            let bought = cb::number(scope, args.get(2))?;
            let sell = cb::number(scope, args.get(3))?;
            let delta = cb::number(scope, args.get(4))?;
            match (
                price_i32(cost),
                price_i32(bought),
                price_i32(sell),
                price_i32(delta),
            ) {
                (Some(cost), Some(bought), Some(sell), Some(delta)) => {
                    api::shop_facts::shop_buy_price(cost, bought, sell, delta) as f64
                }
                _ => f64::NAN,
            }
        }
        "buyoutCost" => {
            let cost = cb::number(scope, args.get(1))?;
            let stock = cb::number(scope, args.get(2))?;
            let sell = cb::number(scope, args.get(3))?;
            let delta = cb::number(scope, args.get(4))?;
            match (
                price_i32(cost),
                price_stock(stock),
                price_i32(sell),
                price_i32(delta),
            ) {
                (Some(cost), Some(stock), Some(sell), Some(delta)) => {
                    api::shop_facts::buyout_cost(cost, stock, sell, delta) as f64
                }
                _ => f64::NAN,
            }
        }
        _ => return Err(cb::type_error(scope, "unknown native shop price operation")),
    };
    Ok(cb::num(scope, price))
}

fn price_i32(value: f64) -> Option<i32> {
    (value.is_finite()
        && value.fract() == 0.0
        && value >= f64::from(i32::MIN)
        && value <= f64::from(i32::MAX))
    .then_some(value as i32)
}

fn price_stock(value: f64) -> Option<i32> {
    if !value.is_finite() {
        return None;
    }
    if value <= 0.0 {
        return Some(0);
    }
    Some(value.ceil().min(f64::from(i32::MAX)) as i32)
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
