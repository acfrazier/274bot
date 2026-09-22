//! Typed local V8 marshalling for the clue row helper.
//! Not a rustyscript `register_function` JSON op and not a clue machine.

use crate::supply_v2;
use rustyscript::Runtime;
use serde_json::Value;

const PENDING: &str = "__pending__";

pub(super) fn install(runtime: &mut Runtime) -> Result<(), String> {
    let context = runtime.deno_runtime().main_context();
    let mut scope = runtime.deno_runtime().handle_scope();
    let global = context.open(&mut scope).global(&mut scope);
    let name = v8::String::new(&mut scope, "__rs2b0t_clue_facts_v2")
        .ok_or_else(|| "clue name".to_string())?;
    let func = v8::Function::new(&mut scope, clue_callback).ok_or_else(|| "clue fn".to_string())?;
    global
        .set(&mut scope, name.into(), func.into())
        .ok_or_else(|| "clue set".to_string())?;
    Ok(())
}

fn clue_callback(
    scope: &mut v8::HandleScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    match run_clue(scope, &args) {
        Ok(value) => rv.set(value),
        Err(err) if err == PENDING => {}
        Err(err) => match helper_err(scope, &err) {
            Ok(value) => rv.set(value),
            Err(_) => rv.set(v8::null(scope).into()),
        },
    }
}

fn run_clue<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: &v8::FunctionCallbackArguments,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let op = args.get(0);
    if !op.is_string() {
        return Err("invalid-args".into());
    }
    let op = js_to_string(scope, op)?;
    match op.as_str() {
        "row" => clue_row_op(scope, args.get(1)),
        _ => Err("invalid-args".into()),
    }
}

fn clue_row_op<'s>(
    scope: &mut v8::HandleScope<'s>,
    input: v8::Local<v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    // The pin is settled before the data slot, and the family before the key.
    let (id, alias) = clue_pin(scope, input)?;
    let Some(data) = supply_v2::selected_data() else {
        return Err("missing-selected-data".into());
    };
    let pin = match id {
        Some(id) => api::clue_facts::CluePin::Id(id),
        None => api::clue_facts::CluePin::Alias(&alias),
    };
    match api::clue_facts::clue_row(data.trails(), pin) {
        Ok(value) => {
            let value = materialize(scope, &value)?;
            helper_ok(scope, value)
        }
        Err(error) => Err(error.into()),
    }
}

/// Exactly one pin. Neither or both, a non-object, an array, a non-string alias,
/// or an id that is not a real `i32` is invalid-args. The alias buffer is empty
/// for an id pin, and a present `{ id: 0 }` is a pin.
fn clue_pin(
    scope: &mut v8::HandleScope,
    input: v8::Local<v8::Value>,
) -> Result<(Option<i32>, String), String> {
    if input.is_null() || input.is_undefined() || input.is_array() || !input.is_object() {
        return Err("invalid-args".into());
    }
    let id_set = has_own(scope, input, "id")?;
    let alias_set = has_own(scope, input, "alias")?;
    if id_set == alias_set {
        return Err("invalid-args".into());
    }
    if id_set {
        // `is_int32` is false for "3554", 3554.5, a bigint, a boxed number, and
        // negative zero: this pin is never converted out of a string or a double.
        let value = field(scope, input, "id")?;
        if !value.is_int32() {
            return Err("invalid-args".into());
        }
        let id = value
            .int32_value(scope)
            .ok_or_else(|| "invalid-args".to_string())?;
        return Ok((Some(id), String::new()));
    }
    let value = field(scope, input, "alias")?;
    if value.is_null() || !value.is_string() {
        return Err("invalid-args".into());
    }
    Ok((None, js_to_string(scope, value)?))
}

fn has_own(
    scope: &mut v8::HandleScope,
    value: v8::Local<v8::Value>,
    name: &str,
) -> Result<bool, String> {
    let obj = value
        .to_object(scope)
        .ok_or_else(|| "invalid-args".to_string())?;
    let key = v8::String::new(scope, name).ok_or_else(|| "string".to_string())?;
    obj.has_own_property(scope, key.into())
        .ok_or_else(|| PENDING.to_string())
}

fn field<'s>(
    scope: &mut v8::HandleScope<'s>,
    value: v8::Local<v8::Value>,
    name: &str,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let obj = value
        .to_object(scope)
        .ok_or_else(|| "invalid-args".to_string())?;
    let key = v8::String::new(scope, name).ok_or_else(|| "string".to_string())?;
    obj.get(scope, key.into())
        .ok_or_else(|| PENDING.to_string())
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
