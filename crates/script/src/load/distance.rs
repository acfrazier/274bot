//! Typed V8 marshalling for `Tile.distanceTo` and shim distance callers.
//!
//! Frozen `Tile.distanceTo` (`Tile.ts`): Chebyshev on the plane,
//! `1_000_000 + xz` across planes. Computed in `i64` so any script-supplied
//! `i32` tile cannot overflow.

use api::query::tile_distance_to;
use api::snapshot::WorldTile;
use rustyscript::Runtime;

pub(super) fn install(runtime: &mut Runtime) -> Result<(), String> {
    let context = runtime.deno_runtime().main_context();
    let mut scope = runtime.deno_runtime().handle_scope();
    let global = context.open(&mut scope).global(&mut scope);
    let name = v8::String::new(&mut scope, "__rs2b0t_distance")
        .ok_or_else(|| "distance name".to_string())?;
    let func = v8::Function::new(&mut scope, distance_callback)
        .ok_or_else(|| "distance fn".to_string())?;
    global
        .set(&mut scope, name.into(), func.into())
        .ok_or_else(|| "distance set".to_string())?;
    Ok(())
}

fn distance_callback(
    scope: &mut v8::HandleScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    match run_distance(scope, &args) {
        Ok(value) => rv.set(value),
        Err(err) => {
            let msg = match v8::String::new(scope, &err) {
                Some(s) => s,
                None => {
                    rv.set(v8::null(scope).into());
                    return;
                }
            };
            let exc = v8::Exception::error(scope, msg);
            scope.throw_exception(exc);
        }
    }
}

fn run_distance<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: &v8::FunctionCallbackArguments,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let from = required_tile(scope, args.get(0), "from")?;
    let to = required_tile(scope, args.get(1), "to")?;
    let distance = tile_distance_to(from, to);
    Ok(v8::Number::new(scope, distance as f64).into())
}

fn required_tile(
    scope: &mut v8::HandleScope,
    value: v8::Local<v8::Value>,
    side: &str,
) -> Result<WorldTile, String> {
    if value.is_null() || value.is_undefined() || !value.is_object() {
        return Err(format!(
            "invalid tile distance: {side} must be a Tile-like object"
        ));
    }
    Ok(WorldTile {
        x: required_i32_field(scope, value, side, "x")?,
        z: required_i32_field(scope, value, side, "z")?,
        level: match optional_field(scope, value, "level")? {
            None => 0,
            Some(v) if v.is_null() || v.is_undefined() => 0,
            Some(v) => required_i32(scope, v, side, "level")?,
        },
    })
}

fn optional_field<'s>(
    scope: &mut v8::HandleScope<'s>,
    value: v8::Local<v8::Value>,
    name: &str,
) -> Result<Option<v8::Local<'s, v8::Value>>, String> {
    let obj = value
        .to_object(scope)
        .ok_or_else(|| "invalid tile distance: not an object".to_string())?;
    let key = v8::String::new(scope, name).ok_or_else(|| "string".to_string())?;
    Ok(obj.get(scope, key.into()))
}

fn required_i32_field(
    scope: &mut v8::HandleScope,
    value: v8::Local<v8::Value>,
    side: &str,
    field: &str,
) -> Result<i32, String> {
    let v = optional_field(scope, value, field)?
        .ok_or_else(|| format!("invalid tile distance: {side}.{field} must be an integer"))?;
    required_i32(scope, v, side, field)
}

fn required_i32(
    scope: &mut v8::HandleScope,
    value: v8::Local<v8::Value>,
    side: &str,
    field: &str,
) -> Result<i32, String> {
    if !value.is_number() {
        return Err(format!(
            "invalid tile distance: {side}.{field} must be an integer"
        ));
    }
    let n = value.number_value(scope).ok_or_else(|| {
        format!("invalid tile distance: {side}.{field} must be an integer")
    })?;
    if !n.is_finite() || n.fract() != 0.0 {
        return Err(format!(
            "invalid tile distance: {side}.{field} must be an integer"
        ));
    }
    if n < (i32::MIN as f64) || n > (i32::MAX as f64) {
        return Err(format!(
            "invalid tile distance: {side}.{field} must be an integer in {}..={}",
            i32::MIN,
            i32::MAX
        ));
    }
    Ok(n as i32)
}
