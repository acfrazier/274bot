//! Typed V8 marshalling for `api.lineOfSight` and v1 `Reachability.lineOfSight`.

use api::line_of_sight::LineOfSightError;
use api::snapshot::WorldTile;
use rustyscript::Runtime;

const PENDING: &str = "__pending__";

pub(super) fn install(runtime: &mut Runtime) -> Result<(), String> {
    runtime
        .eval::<()>(
            r#"
globalThis.__rs2b0t_flags_view = function (u8) {
  const ints = (u8 && u8.byteLength)
    ? new Int32Array(u8.buffer, u8.byteOffset, u8.byteLength >> 2)
    : new Int32Array(0);
  const length = ints.length;
  return {
    length: length,
    at: function (index) {
      if (typeof index !== 'number' || !Number.isFinite(index) || !Number.isInteger(index)) {
        return undefined;
      }
      if (index < 0 || index >= length) return undefined;
      return ints[index];
    }
  };
};
"#,
        )
        .map_err(|e| format!("flags view: {e}"))?;
    let context = runtime.deno_runtime().main_context();
    let mut scope = runtime.deno_runtime().handle_scope();
    let global = context.open(&mut scope).global(&mut scope);
    let name = v8::String::new(&mut scope, "__rs2b0t_line_of_sight")
        .ok_or_else(|| "los name".to_string())?;
    let func = v8::Function::new(&mut scope, los_callback)
        .ok_or_else(|| "los fn".to_string())?;
    global
        .set(&mut scope, name.into(), func.into())
        .ok_or_else(|| "los set".to_string())?;
    Ok(())
}

fn los_callback(
    scope: &mut v8::HandleScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    match run_los(scope, &args) {
        Ok(value) => rv.set(value),
        Err(err) if err == PENDING => {}
        Err(err) => match helper_err(scope, &err) {
            Ok(value) => rv.set(value),
            Err(_) => rv.set(v8::null(scope).into()),
        },
    }
}

fn run_los<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: &v8::FunctionCallbackArguments,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let kind = js_to_string(scope, args.get(0))?;
    match kind.as_str() {
        "v2" => v2(scope, args.get(1)),
        "v1" => v1(scope, args.get(1), args.get(2), args.get(3)),
        _ => Err("invalid-args".into()),
    }
}

fn v2<'s>(
    scope: &mut v8::HandleScope<'s>,
    input: v8::Local<v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    if !input.is_object() || input.is_null() || input.is_undefined() {
        return helper_err(scope, LineOfSightError::InvalidArgs.as_str());
    }
    let from_v = field(scope, input, "from")?;
    let from = required_tile(scope, from_v)?;
    let to_v = field(scope, input, "to")?;
    let to = required_tile(scope, to_v)?;
    let size = optional_size(scope, input)?;
    match crate::line_of_sight::query_v2(from, to, size) {
        Ok(v) => helper_ok_bool(scope, v),
        Err(e) => helper_err(scope, e.as_str()),
    }
}

fn v1<'s>(
    scope: &mut v8::HandleScope<'s>,
    from: v8::Local<v8::Value>,
    to: v8::Local<v8::Value>,
    size: v8::Local<v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let Some(from) = tile_or_none(scope, from)? else {
        return bool_val(scope, false);
    };
    let Some(to) = tile_or_none(scope, to)? else {
        return bool_val(scope, false);
    };
    let size = if size.is_undefined() {
        None
    } else if !size.is_number() {
        return bool_val(scope, false);
    } else {
        let n = size.number_value(scope).ok_or_else(|| PENDING.to_string())?;
        if !n.is_finite() || n.fract() != 0.0 {
            return bool_val(scope, false);
        }
        Some(n as i32)
    };
    bool_val(scope, crate::line_of_sight::query_v1(from, to, size))
}

fn required_tile(
    scope: &mut v8::HandleScope,
    value: v8::Local<v8::Value>,
) -> Result<WorldTile, String> {
    tile_or_none(scope, value)?.ok_or_else(|| LineOfSightError::InvalidArgs.as_str().to_string())
}

fn tile_or_none(
    scope: &mut v8::HandleScope,
    value: v8::Local<v8::Value>,
) -> Result<Option<WorldTile>, String> {
    if value.is_null() || value.is_undefined() || !value.is_object() {
        return Ok(None);
    }
    let x = required_i32_field(scope, value, "x")?;
    let z = required_i32_field(scope, value, "z")?;
    let level = required_i32_field(scope, value, "level")?;
    Ok(Some(WorldTile { x, z, level }))
}

fn optional_size(
    scope: &mut v8::HandleScope,
    input: v8::Local<v8::Value>,
) -> Result<Option<i32>, String> {
    let v = field(scope, input, "size")?;
    if v.is_undefined() {
        return Ok(None);
    }
    required_i32(scope, v).map(Some)
}

fn required_i32_field(
    scope: &mut v8::HandleScope,
    value: v8::Local<v8::Value>,
    name: &str,
) -> Result<i32, String> {
    let v = field(scope, value, name)?;
    required_i32(scope, v)
}

fn required_i32(scope: &mut v8::HandleScope, value: v8::Local<v8::Value>) -> Result<i32, String> {
    if !value.is_number() {
        return Err(LineOfSightError::InvalidArgs.as_str().into());
    }
    let n = value
        .number_value(scope)
        .ok_or_else(|| PENDING.to_string())?;
    if !n.is_finite() || n.fract() != 0.0 {
        return Err(LineOfSightError::InvalidArgs.as_str().into());
    }
    if n < (i32::MIN as f64) || n > (i32::MAX as f64) {
        return Err(LineOfSightError::InvalidArgs.as_str().into());
    }
    Ok(n as i32)
}

fn field<'s>(
    scope: &mut v8::HandleScope<'s>,
    value: v8::Local<v8::Value>,
    name: &str,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let obj = value
        .to_object(scope)
        .ok_or_else(|| LineOfSightError::InvalidArgs.as_str().to_string())?;
    let key = v8::String::new(scope, name).ok_or_else(|| "string".to_string())?;
    obj.get(scope, key.into())
        .ok_or_else(|| PENDING.to_string())
}

fn js_to_string(scope: &mut v8::HandleScope, value: v8::Local<v8::Value>) -> Result<String, String> {
    match value.to_string(scope) {
        Some(s) => Ok(s.to_rust_string_lossy(scope)),
        None => Err(PENDING.into()),
    }
}

fn helper_ok_bool<'s>(
    scope: &mut v8::HandleScope<'s>,
    value: bool,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let obj = v8::Object::new(scope);
    let ok = v8::Boolean::new(scope, true);
    set_key(scope, obj, "ok", ok.into());
    let value = v8::Boolean::new(scope, value);
    set_key(scope, obj, "value", value.into());
    Ok(obj.into())
}

fn helper_err<'s>(
    scope: &mut v8::HandleScope<'s>,
    error: &str,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let obj = v8::Object::new(scope);
    let ok = v8::Boolean::new(scope, false);
    set_key(scope, obj, "ok", ok.into());
    let msg = v8::String::new(scope, error).ok_or_else(|| "string".to_string())?;
    set_key(scope, obj, "error", msg.into());
    Ok(obj.into())
}

fn bool_val<'s>(
    scope: &mut v8::HandleScope<'s>,
    value: bool,
) -> Result<v8::Local<'s, v8::Value>, String> {
    Ok(v8::Boolean::new(scope, value).into())
}

fn set_key<'s>(
    scope: &mut v8::HandleScope<'s>,
    obj: v8::Local<'s, v8::Object>,
    key: &str,
    value: v8::Local<'s, v8::Value>,
) {
    if let Some(k) = v8::String::new(scope, key) {
        obj.set(scope, k.into(), value);
    }
}
