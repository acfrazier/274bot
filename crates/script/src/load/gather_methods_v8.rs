//! Typed local V8 marshalling for gather query helpers.
//! Not a rustyscript `register_function` JSON op and not a prayer machine.

use crate::supply_v2;
use rustyscript::Runtime;
use serde_json::Value;

const PENDING: &str = "__pending__";

/// The same numeric gate as the scene helpers' `sceneLimitOk`.
const SCENE_LIMIT_MAX: i32 = 64;

pub(super) fn install(runtime: &mut Runtime) -> Result<(), String> {
    let context = runtime.deno_runtime().main_context();
    let mut scope = runtime.deno_runtime().handle_scope();
    let global = context.open(&mut scope).global(&mut scope);
    let name = v8::String::new(&mut scope, "__rs2b0t_gather_methods_v2")
        .ok_or_else(|| "gather name".to_string())?;
    let func =
        v8::Function::new(&mut scope, gather_callback).ok_or_else(|| "gather fn".to_string())?;
    global
        .set(&mut scope, name.into(), func.into())
        .ok_or_else(|| "gather set".to_string())?;
    Ok(())
}

fn gather_callback(
    scope: &mut v8::HandleScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    match run_gather(scope, &args) {
        Ok(value) => rv.set(value),
        Err(err) if err == PENDING => {}
        Err(err) => match helper_err(scope, &err) {
            Ok(value) => rv.set(value),
            Err(_) => rv.set(v8::null(scope).into()),
        },
    }
}

fn run_gather<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: &v8::FunctionCallbackArguments,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let op = args.get(0);
    if !op.is_string() {
        return Err("invalid-args".into());
    }
    let op = js_to_string(scope, op)?;
    match op.as_str() {
        "gatherMethods" => gather_methods_op(scope, args.get(1)),
        "gatherResource" => gather_resource_op(scope, args.get(1)),
        "gatherPlacements" => gather_placements_op(scope, args.get(1)),
        _ => Err("invalid-args".into()),
    }
}

fn gather_methods_op<'s>(
    scope: &mut v8::HandleScope<'s>,
    input: v8::Local<v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let skill = optional_skill(scope, input)?;
    let Some(data) = supply_v2::selected_data() else {
        return Err("missing-selected-data".into());
    };
    match api::gather_methods::gather_methods(data.gather_methods(), skill.as_deref()) {
        Ok(value) => {
            let value = materialize(scope, &value)?;
            helper_ok(scope, value)
        }
        Err(error) => Err(error.into()),
    }
}

fn gather_resource_op<'s>(
    scope: &mut v8::HandleScope<'s>,
    input: v8::Local<v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let name = required_name(scope, input)?;
    let Some(data) = supply_v2::selected_data() else {
        return Err("missing-selected-data".into());
    };
    match api::gather_methods::gather_resource(data.gather_methods(), &name) {
        Ok(value) => {
            let value = materialize(scope, &value)?;
            helper_ok(scope, value)
        }
        Err(error) => Err(error.into()),
    }
}

/// The public wrapper's gates run first, so a bad call never reads the pin.
/// A present object with no `region` key is `missing-region` before resource,
/// limit, or region shape; a present-but-bad region is `invalid-args`.
fn gather_placements_op<'s>(
    scope: &mut v8::HandleScope<'s>,
    input: v8::Local<v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let request = placement_request(scope, input)?;
    let Some(data) = supply_v2::selected_data() else {
        return Err("missing-selected-data".into());
    };
    match api::gather_methods::gather_placements(
        data.gather_placements(),
        data.gather_methods(),
        &request.resource,
        &request.region,
        request.limit,
    ) {
        Ok(value) => {
            let value = materialize(scope, &value)?;
            helper_ok(scope, value)
        }
        Err(error) => Err(error.into()),
    }
}

struct PlacementRequest {
    resource: String,
    region: api::gather_methods::SceneRegionInput,
    limit: usize,
}

/// `limit` is 1..=64, the same numeric gate as the scene helpers, and never
/// clamped. `is_int32` is false for a string, a fraction, a bigint, a boxed
/// number, and negative zero, so each region field and the limit are integers
/// or `invalid-args`. Extra keys are ignored.
fn placement_request(
    scope: &mut v8::HandleScope,
    input: v8::Local<v8::Value>,
) -> Result<PlacementRequest, String> {
    if input.is_null() || input.is_undefined() || input.is_array() || !input.is_object() {
        return Err("invalid-args".into());
    }
    if !has_own(scope, input, "region")? {
        return Err("missing-region".into());
    }
    let resource = field(scope, input, "resource")?;
    if !resource.is_string() {
        return Err("invalid-args".into());
    }
    let resource = js_to_string(scope, resource)?;
    let limit = required_i32(scope, input, "limit")?;
    if !(1..=SCENE_LIMIT_MAX).contains(&limit) {
        return Err("invalid-args".into());
    }
    let region = field(scope, input, "region")?;
    if region.is_null() || region.is_undefined() || region.is_array() || !region.is_object() {
        return Err("invalid-args".into());
    }
    let region = api::gather_methods::SceneRegionInput {
        min_x: required_i32(scope, region, "min_x")?,
        min_z: required_i32(scope, region, "min_z")?,
        max_x: required_i32(scope, region, "max_x")?,
        max_z: required_i32(scope, region, "max_z")?,
        level: required_i32(scope, region, "level")?,
    };
    Ok(PlacementRequest {
        resource,
        region,
        limit: limit as usize,
    })
}

fn optional_skill(
    scope: &mut v8::HandleScope,
    input: v8::Local<v8::Value>,
) -> Result<Option<String>, String> {
    if input.is_null() || input.is_undefined() || !input.is_object() {
        return Err("invalid-args".into());
    }
    let skill = field(scope, input, "skill")?;
    if skill.is_undefined() {
        return Ok(None);
    }
    if skill.is_null() || !skill.is_string() {
        return Err("invalid-args".into());
    }
    Ok(Some(js_to_string(scope, skill)?))
}

fn required_name(
    scope: &mut v8::HandleScope,
    input: v8::Local<v8::Value>,
) -> Result<String, String> {
    if input.is_null() || input.is_undefined() || !input.is_object() {
        return Err("invalid-args".into());
    }
    let name = field(scope, input, "name")?;
    if name.is_null() || name.is_undefined() || !name.is_string() {
        return Err("invalid-args".into());
    }
    js_to_string(scope, name)
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

/// A required real `i32`. Absent is `invalid-args`, not a default.
fn required_i32(
    scope: &mut v8::HandleScope,
    input: v8::Local<v8::Value>,
    name: &str,
) -> Result<i32, String> {
    if !has_own(scope, input, name)? {
        return Err("invalid-args".into());
    }
    let value = field(scope, input, name)?;
    if !value.is_int32() {
        return Err("invalid-args".into());
    }
    value
        .int32_value(scope)
        .ok_or_else(|| "invalid-args".to_string())
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
