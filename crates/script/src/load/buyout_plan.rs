//! In-isolate ShopBuyout Rust helper.
//!
//! JS marshals primitive args into `__rs2b0t_buyout_plan`. This callback
//! builds a typed local request, runs the Rust planner, and materializes
//! the typed result into a V8 object. Not a FlatBuffer RPC: no isolate/host
//! buffer, no extra JSON op, no `__rs2b0t_shop` planner payload.

use crate::shop::{BuyoutPlanRequest, BuyoutPlanResult};
use rustyscript::Runtime;

pub(super) fn install(runtime: &mut Runtime) -> Result<(), String> {
    let context = runtime.deno_runtime().main_context();
    let mut scope = runtime.deno_runtime().handle_scope();
    let global = context.open(&mut scope).global(&mut scope);
    let name = v8::String::new(&mut scope, "__rs2b0t_buyout_plan")
        .ok_or_else(|| "buyout plan name".to_string())?;
    let func = v8::Function::new(&mut scope, buyout_plan_callback)
        .ok_or_else(|| "buyout plan function".to_string())?;
    global
        .set(&mut scope, name.into(), func.into())
        .ok_or_else(|| "buyout plan set".to_string())?;
    Ok(())
}

fn buyout_plan_callback(
    scope: &mut v8::HandleScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    match run(scope, &args) {
        Ok(value) => rv.set(value),
        Err(reason) => match fail_obj(scope, &reason) {
            Ok(value) => rv.set(value),
            Err(_) => rv.set(v8::null(scope).into()),
        },
    }
}

fn run<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: &v8::FunctionCallbackArguments,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let inv = js_string_arg(scope, args.get(0));
    let keeper = js_string_arg(scope, args.get(1));
    let coins = js_i64(scope, args.get(2));
    let stock_objs = js_string_array(scope, args.get(3))?;
    let stock_counts = js_i32_array(scope, args.get(4))?;
    if stock_objs.len() != stock_counts.len() {
        return Err("stock objs/counts length mismatch".into());
    }
    let chosen = js_string_array(scope, args.get(5))?;
    let req = BuyoutPlanRequest {
        inv,
        keeper,
        coins,
        stock: stock_objs.into_iter().zip(stock_counts).collect(),
        chosen,
    };
    let result = crate::shop::run_buyout_plan(&req);
    materialize_result(scope, &result)
}

fn materialize_result<'s>(
    scope: &mut v8::HandleScope<'s>,
    result: &BuyoutPlanResult,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let obj = v8::Object::new(scope);
    let ok = v8::Boolean::new(scope, result.ok);
    set(scope, obj, "ok", ok.into())?;
    let reason = js_string(scope, &result.reason)?;
    set(scope, obj, "reason", reason)?;
    let items = v8::Array::new(scope, result.items.len() as i32);
    for (i, row) in result.items.iter().enumerate() {
        let item = v8::Object::new(scope);
        let obj_name = js_string(scope, &row.obj)?;
        set(scope, item, "obj", obj_name)?;
        let name = js_string(scope, &row.name)?;
        set(scope, item, "name", name)?;
        let units = num(scope, f64::from(row.units));
        set(scope, item, "units", units)?;
        let est = num(scope, row.est_cost as f64);
        set(scope, item, "estCost", est)?;
        items
            .set_index(scope, i as u32, item.into())
            .ok_or_else(|| "v8 array set failed".to_string())?;
    }
    set(scope, obj, "items", items.into())?;
    Ok(obj.into())
}

fn fail_obj<'s>(
    scope: &mut v8::HandleScope<'s>,
    reason: &str,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let obj = v8::Object::new(scope);
    let ok = v8::Boolean::new(scope, false);
    set(scope, obj, "ok", ok.into())?;
    let reason = js_string(scope, reason)?;
    set(scope, obj, "reason", reason)?;
    let items = v8::Array::new(scope, 0);
    set(scope, obj, "items", items.into())?;
    Ok(obj.into())
}

fn js_string_arg(scope: &mut v8::HandleScope, value: v8::Local<v8::Value>) -> String {
    if value.is_null_or_undefined() {
        return String::new();
    }
    value
        .to_string(scope)
        .map(|s| s.to_rust_string_lossy(scope))
        .unwrap_or_default()
}

fn js_i64(scope: &mut v8::HandleScope, value: v8::Local<v8::Value>) -> i64 {
    value
        .number_value(scope)
        .filter(|n| n.is_finite())
        .map(|n| n.trunc() as i64)
        .unwrap_or(0)
}

fn js_string_array(
    scope: &mut v8::HandleScope,
    value: v8::Local<v8::Value>,
) -> Result<Vec<String>, String> {
    if value.is_null_or_undefined() {
        return Ok(Vec::new());
    }
    let arr = value
        .to_object(scope)
        .and_then(|o| v8::Local::<v8::Array>::try_from(o).ok())
        .ok_or_else(|| "expected string array".to_string())?;
    let len = arr.length();
    let mut out = Vec::with_capacity(len as usize);
    for i in 0..len {
        let item = arr
            .get_index(scope, i)
            .ok_or_else(|| "array get failed".to_string())?;
        out.push(js_string_arg(scope, item));
    }
    Ok(out)
}

fn js_i32_array(
    scope: &mut v8::HandleScope,
    value: v8::Local<v8::Value>,
) -> Result<Vec<i32>, String> {
    if value.is_null_or_undefined() {
        return Ok(Vec::new());
    }
    let arr = value
        .to_object(scope)
        .and_then(|o| v8::Local::<v8::Array>::try_from(o).ok())
        .ok_or_else(|| "expected number array".to_string())?;
    let len = arr.length();
    let mut out = Vec::with_capacity(len as usize);
    for i in 0..len {
        let item = arr
            .get_index(scope, i)
            .ok_or_else(|| "array get failed".to_string())?;
        let n = js_i64(scope, item).clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
        out.push(n);
    }
    Ok(out)
}

fn js_string<'s>(
    scope: &mut v8::HandleScope<'s>,
    s: &str,
) -> Result<v8::Local<'s, v8::Value>, String> {
    v8::String::new(scope, s)
        .map(|v| v.into())
        .ok_or_else(|| "v8 string alloc failed".to_string())
}

fn num<'s>(scope: &mut v8::HandleScope<'s>, n: f64) -> v8::Local<'s, v8::Value> {
    v8::Number::new(scope, n).into()
}

fn set<'s>(
    scope: &mut v8::HandleScope<'s>,
    obj: v8::Local<'s, v8::Object>,
    key: &str,
    value: v8::Local<'s, v8::Value>,
) -> Result<(), String> {
    let key = js_string(scope, key)?;
    obj.set(scope, key, value)
        .ok_or_else(|| "v8 object set failed".to_string())?;
    Ok(())
}
