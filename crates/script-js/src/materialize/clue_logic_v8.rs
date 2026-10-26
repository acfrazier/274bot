//! Typed local V8 marshalling for the held-step identify helper.
//! Not a rustyscript `register_function` JSON op and not a clue machine.

use crate::supply_v2;
use rustyscript::Runtime;
use serde_json::Value;

const PENDING: &str = "__pending__";

pub(super) fn install(runtime: &mut Runtime) -> Result<(), String> {
    let context = runtime.deno_runtime().main_context();
    let mut scope = runtime.deno_runtime().handle_scope();
    let global = context.open(&mut scope).global(&mut scope);
    let name = v8::String::new(&mut scope, "__rs2b0t_clue_logic_v2")
        .ok_or_else(|| "clue logic name".to_string())?;
    let func = v8::Function::new(&mut scope, clue_logic_callback)
        .ok_or_else(|| "clue logic fn".to_string())?;
    global
        .set(&mut scope, name.into(), func.into())
        .ok_or_else(|| "clue logic set".to_string())?;
    Ok(())
}

fn clue_logic_callback(
    scope: &mut v8::HandleScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    match run_clue_logic(scope, &args) {
        Ok(value) => rv.set(value),
        Err(err) if err == PENDING => {}
        Err(err) => match helper_err(scope, &err) {
            Ok(value) => rv.set(value),
            Err(_) => rv.set(v8::null(scope).into()),
        },
    }
}

fn run_clue_logic<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: &v8::FunctionCallbackArguments,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let op = args.get(0);
    if !op.is_string() {
        return Err("invalid-args".into());
    }
    let op = js_to_string(scope, op)?;
    match op.as_str() {
        "heldStep" => held_step_op(scope),
        _ => Err("invalid-args".into()),
    }
}

/// The wrapper's order: `missing-selected-data`, then the helper's family
/// token, then `none-held`, then the landed row. The page is the already
/// posted `host().snapshot.inv`; nothing here rebuilds the tab or accepts a
/// caller-supplied page.
fn held_step_op<'s>(scope: &mut v8::HandleScope<'s>) -> Result<v8::Local<'s, v8::Value>, String> {
    let Some(data) = supply_v2::selected_data() else {
        return Err(supply_v2::GAME_DATA_UNAVAILABLE.into());
    };
    let held = posted_page(scope)?;
    match api::clue_logic::identify_step(&held, data.trails()) {
        Ok(row) => {
            // Consumption of the landed row builder only: the ok value cannot
            // drift from `clue.row`. The id came from that table, so this
            // cannot publish `unknown-id`.
            let landed =
                api::clue_facts::clue_row(data.trails(), api::clue_facts::CluePin::Id(row.id))
                    .map_err(|error| error.to_string())?;
            let value = materialize(scope, &landed)?;
            helper_ok(scope, value)
        }
        Err(error) => Err(error.into()),
    }
}

/// The posted `(obj id, count)` page in posted order, from the host object the
/// materializer wrote — including a delta that kept the last rows. A missing
/// host handle, snapshot, or page is an empty held list, never a second
/// snapshot error token. A row that is not an `i32` pair cannot be held.
fn posted_page<'s>(scope: &mut v8::HandleScope<'s>) -> Result<Vec<(i32, i32)>, String> {
    let global = scope.get_current_context().global(scope);
    let snapshot = match optional_field(scope, global.into(), "__rs2b0t_host")? {
        Some(host) => optional_field(scope, host, "snapshot")?,
        None => None,
    };
    let page = match snapshot {
        Some(snapshot) => optional_field(scope, snapshot, "inv")?,
        None => None,
    };
    let Some(page) = page else {
        return Ok(Vec::new());
    };
    let Ok(array) = v8::Local::<v8::Array>::try_from(page) else {
        return Ok(Vec::new());
    };
    let mut held = Vec::with_capacity(array.length() as usize);
    for index in 0..array.length() {
        let row = array
            .get_index(scope, index)
            .ok_or_else(|| PENDING.to_string())?;
        if !row.is_object() || row.is_array() {
            continue;
        }
        let (Some(id), Some(count)) = (
            optional_field(scope, row, "id")?,
            optional_field(scope, row, "count")?,
        ) else {
            continue;
        };
        if let (Some(id), Some(count)) = (page_i32(scope, id), page_i32(scope, count)) {
            held.push((id, count));
        }
    }
    Ok(held)
}

/// The host writes page numbers as doubles. A finite integral value in `i32`
/// range is a pair member; anything else (a string, a bigint, a fraction, a
/// double out of range) is skipped the same way an unheld pair is.
fn page_i32(scope: &mut v8::HandleScope, value: v8::Local<v8::Value>) -> Option<i32> {
    if !value.is_number() {
        return None;
    }
    let number = value.number_value(scope)?;
    if !number.is_finite() || number.fract() != 0.0 {
        return None;
    }
    if number < i32::MIN as f64 || number > i32::MAX as f64 {
        return None;
    }
    Some(number as i32)
}

fn optional_field<'s>(
    scope: &mut v8::HandleScope<'s>,
    value: v8::Local<'s, v8::Value>,
    name: &str,
) -> Result<Option<v8::Local<'s, v8::Value>>, String> {
    let Some(obj) = value.to_object(scope) else {
        return Ok(None);
    };
    let key = v8::String::new(scope, name).ok_or_else(|| "string".to_string())?;
    let field = obj
        .get(scope, key.into())
        .ok_or_else(|| PENDING.to_string())?;
    if field.is_null_or_undefined() {
        return Ok(None);
    }
    Ok(Some(field))
}

fn js_to_string(
    scope: &mut v8::HandleScope,
    value: v8::Local<v8::Value>,
) -> Result<String, String> {
    match value.to_string(scope) {
        Some(text) => Ok(text.to_rust_string_lossy(scope)),
        None => Err(PENDING.into()),
    }
}

fn helper_ok<'s>(
    scope: &mut v8::HandleScope<'s>,
    value: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let obj = v8::Object::new(scope);
    let ok = v8::Boolean::new(scope, true);
    set_key(scope, obj, "ok", ok.into());
    set_key(scope, obj, "value", value);
    Ok(obj.into())
}

fn helper_err<'s>(
    scope: &mut v8::HandleScope<'s>,
    error: &str,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let obj = v8::Object::new(scope);
    let ok = v8::Boolean::new(scope, false);
    set_key(scope, obj, "ok", ok.into());
    let error = if error == "missing-selected-data" {
        supply_v2::GAME_DATA_UNAVAILABLE
    } else {
        error
    };
    let error = v8_str(scope, error)?;
    set_key(scope, obj, "error", error);
    Ok(obj.into())
}

fn materialize<'s>(
    scope: &mut v8::HandleScope<'s>,
    value: &Value,
) -> Result<v8::Local<'s, v8::Value>, String> {
    match value {
        Value::Null => Ok(v8::null(scope).into()),
        Value::Bool(flag) => Ok(v8::Boolean::new(scope, *flag).into()),
        Value::Number(number) => {
            if let Some(integer) = number.as_i64() {
                let integer = i32::try_from(integer).map_err(|_| "number".to_string())?;
                Ok(v8::Integer::new(scope, integer).into())
            } else {
                let float = number.as_f64().ok_or_else(|| "number".to_string())?;
                Ok(v8::Number::new(scope, float).into())
            }
        }
        Value::String(text) => v8_str(scope, text),
        Value::Array(items) => {
            let arr = v8::Array::new(scope, i32::try_from(items.len()).unwrap_or(i32::MAX));
            for (index, item) in items.iter().enumerate() {
                let value = materialize(scope, item)?;
                arr.set_index(scope, index as u32, value)
                    .ok_or_else(|| "array".to_string())?;
            }
            Ok(arr.into())
        }
        Value::Object(map) => {
            let obj = v8::Object::new(scope);
            for (key, item) in map {
                let value = materialize(scope, item)?;
                set_key(scope, obj, key, value);
            }
            Ok(obj.into())
        }
    }
}

fn v8_str<'s>(
    scope: &mut v8::HandleScope<'s>,
    text: &str,
) -> Result<v8::Local<'s, v8::Value>, String> {
    v8::String::new(scope, text)
        .map(|value| value.into())
        .ok_or_else(|| "string".to_string())
}

fn set_key(
    scope: &mut v8::HandleScope,
    obj: v8::Local<v8::Object>,
    key: &str,
    value: v8::Local<v8::Value>,
) {
    if let Some(key) = v8::String::new(scope, key) {
        let _ = obj.set(scope, key.into(), value);
    }
}
