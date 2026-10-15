//! Typed V8 marshalling for tile-distance shim callers.
//!
//! Frozen `Tile.distanceTo` (`Tile.ts`) is Chebyshev on the plane and
//! `1_000_000 + xz` across planes. Entity-query `withinOf` uses the same
//! calculation without the level penalty. Script-supplied coordinates are JS
//! numbers, computed in `f64` the same way `Math.abs` would.

use super::callback_v8::{self as cb, Callback};
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
    let clue_tiles_to_name = v8::String::new(&mut scope, "__rs2b0t_clue_tiles_to")
        .ok_or_else(|| "clue tiles-to name".to_string())?;
    let clue_tiles_to = v8::Function::new(&mut scope, tiles_to_callback)
        .ok_or_else(|| "clue tiles-to fn".to_string())?;
    global
        .set(&mut scope, clue_tiles_to_name.into(), clue_tiles_to.into())
        .ok_or_else(|| "clue tiles-to set".to_string())?;
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
    let planar = args.get(2).boolean_value(scope);
    Ok(v8::Number::new(scope, js_tile_distance(from, to, planar)).into())
}

fn tiles_to_callback<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = (|| {
        let target = args.get(0);
        if !cb::truthy(scope, target) {
            return Ok(v8::null(scope).into());
        }
        let world_tile = Callback::plain(scope, args.get(1), "worldTile");
        let here = world_tile.call(scope, &[])?;
        if !cb::truthy(scope, here) {
            return Ok(v8::null(scope).into());
        }
        let from = required_tile(scope, here, "here").map_err(|e| cb::type_error(scope, &e))?;
        let to = required_tile(scope, target, "target").map_err(|e| cb::type_error(scope, &e))?;
        Ok(v8::Number::new(scope, js_tile_distance(from, to, true)).into())
    })();
    cb::finish(scope, rv, result);
}

#[derive(Clone, Copy)]
struct JsTile {
    x: f64,
    z: f64,
    level: f64,
}

fn js_tile_distance(from: JsTile, to: JsTile, ignore_level: bool) -> f64 {
    let (dx, dz) = ((from.x - to.x).abs(), (from.z - to.z).abs());
    // `Math.max` propagates NaN; `f64::max` would drop it.
    let planar = if dx.is_nan() || dz.is_nan() {
        f64::NAN
    } else {
        dx.max(dz)
    };
    if !ignore_level && from.level != to.level {
        1_000_000.0 + planar
    } else {
        planar
    }
}

fn required_tile(
    scope: &mut v8::HandleScope,
    value: v8::Local<v8::Value>,
    side: &str,
) -> Result<JsTile, String> {
    if value.is_null() || value.is_undefined() || !value.is_object() {
        return Err(format!(
            "invalid tile distance: {side} must be a Tile-like object"
        ));
    }
    Ok(JsTile {
        x: required_number_field(scope, value, side, "x")?,
        z: required_number_field(scope, value, side, "z")?,
        level: match optional_field(scope, value, "level")? {
            None => 0.0,
            Some(v) if v.is_null() || v.is_undefined() => 0.0,
            Some(v) => required_number(scope, v, side, "level")?,
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

fn required_number_field(
    scope: &mut v8::HandleScope,
    value: v8::Local<v8::Value>,
    side: &str,
    field: &str,
) -> Result<f64, String> {
    let v = optional_field(scope, value, field)?
        .ok_or_else(|| format!("invalid tile distance: {side}.{field} must be a number"))?;
    required_number(scope, v, side, field)
}

fn required_number(
    scope: &mut v8::HandleScope,
    value: v8::Local<v8::Value>,
    side: &str,
    field: &str,
) -> Result<f64, String> {
    if !value.is_number() {
        return Err(format!(
            "invalid tile distance: {side}.{field} must be a number"
        ));
    }
    value
        .number_value(scope)
        .ok_or_else(|| format!("invalid tile distance: {side}.{field} must be a number"))
}
