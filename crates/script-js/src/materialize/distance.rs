//! Typed V8 marshalling for world tiles and shared tile-distance callers.
//!
//! Frozen `Tile.distanceTo` (`Tile.ts`) is Chebyshev on the plane and
//! `1_000_000 + xz` across planes. Entity-query `withinOf` uses the same
//! calculation without the level penalty. Script-supplied coordinates are JS
//! numbers, computed in `f64` the same way `Math.abs` would.

use super::callback_v8::{self as cb, Callback};
use rustyscript::Runtime;

pub(super) fn install(runtime: &mut Runtime) -> Result<(), String> {
    cb::install(runtime, "__rs2b0t_tile", tile_callback)?;
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

/// Query a native posted tile without allocating an echoed Tile-like object.
pub(super) fn planar_from_posted<'s>(
    scope: &mut v8::HandleScope<'s>,
    from: crate::observed::Tile,
    to: v8::Local<'s, v8::Value>,
) -> cb::JsResult<'s, v8::Local<'s, v8::Value>> {
    let to = cb::catching(scope, |scope| match required_tile(scope, to, "to") {
        Ok(tile) => Some(tile),
        Err(error) => {
            let message =
                v8::String::new(scope, &error).unwrap_or_else(|| v8::String::empty(scope));
            let error = v8::Exception::error(scope, message);
            scope.throw_exception(error);
            None
        }
    })?;
    let from = JsTile {
        x: f64::from(from.x),
        z: f64::from(from.z),
        level: f64::from(from.level),
    };
    Ok(cb::num(scope, js_tile_distance(from, to, true)))
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

fn tile_callback<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = run_tile(scope, &args);
    cb::finish(scope, rv, result);
}

fn run_tile<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> cb::JsResult<'s, v8::Local<'s, v8::Value>> {
    let operation = args.get(0);
    if !operation.is_int32() {
        return Err(cb::type_error(scope, "invalid tile operation"));
    }
    match operation.int32_value(scope) {
        Some(0) => copy_tile(scope, args.get(1), args.get(2)),
        Some(1) => {
            let source = args.get(2);
            let x = cb::get(scope, source, "x")?;
            let x = cb::add(scope, x, args.get(3))?;
            let z = cb::get(scope, source, "z")?;
            let z = cb::add(scope, z, args.get(4))?;
            let level = cb::get(scope, source, "level")?;
            cb::construct(scope, args.get(1), &[x, z, level], "Tile")
        }
        Some(2) => {
            for field in ["x", "z", "level"] {
                let left = cb::get(scope, args.get(1), field)?;
                let right = cb::get(scope, args.get(2), field)?;
                if !left.strict_equals(right) {
                    return Ok(v8::Boolean::new(scope, false).into());
                }
            }
            Ok(v8::Boolean::new(scope, true).into())
        }
        Some(3) => {
            // Keep V8's UTF-16 strings and ToString semantics, including
            // conversion order and Symbol/@@toPrimitive exceptions.
            let mut text = cb::string(scope, "(");
            for (field, suffix) in [("x", ", "), ("z", ", "), ("level", ")")] {
                let value = cb::get(scope, args.get(1), field)?;
                let value = cb::catching(scope, |s| value.to_string(s))?;
                text = cb::add(scope, text, value.into())?;
                let suffix = cb::string(scope, suffix);
                text = cb::add(scope, text, suffix)?;
            }
            Ok(text)
        }
        Some(4) => posted_tile(scope, args.get(1), args.get(2)),
        _ => Err(cb::type_error(scope, "invalid tile operation")),
    }
}

fn copy_tile<'s>(
    scope: &mut v8::HandleScope<'s>,
    constructor: v8::Local<'s, v8::Value>,
    source: v8::Local<'s, v8::Value>,
) -> cb::JsResult<'s, v8::Local<'s, v8::Value>> {
    let x = cb::get(scope, source, "x")?;
    let z = cb::get(scope, source, "z")?;
    let level = cb::get(scope, source, "level")?;
    let level = if level.is_null_or_undefined() {
        cb::num(scope, 0.0)
    } else {
        level
    };
    cb::construct(scope, constructor, &[x, z, level], "Tile")
}

fn posted_tile<'s>(
    scope: &mut v8::HandleScope<'s>,
    constructor: v8::Local<'s, v8::Value>,
    value: v8::Local<'s, v8::Value>,
) -> cb::JsResult<'s, v8::Local<'s, v8::Value>> {
    if !value.is_object() || value.is_function() {
        return Ok(v8::null(scope).into());
    }
    for (field, maximum) in [("x", 16_383.0), ("z", 16_383.0), ("level", 3.0)] {
        // Read each occurrence separately: script getters may change values
        // or throw between the safe-integer and bound comparisons.
        if field == "level" && cb::get(scope, value, field)?.is_undefined() {
            continue;
        }
        let coordinate = cb::get(scope, value, field)?;
        if !is_safe_integer(scope, coordinate) {
            return Ok(v8::null(scope).into());
        }
        let coordinate = cb::get(scope, value, field)?;
        let zero = cb::num(scope, 0.0);
        if cb::lt(scope, coordinate, zero)? {
            return Ok(v8::null(scope).into());
        }
        let coordinate = cb::get(scope, value, field)?;
        let maximum = cb::num(scope, maximum);
        if cb::gt(scope, coordinate, maximum)? {
            return Ok(v8::null(scope).into());
        }
    }
    let constructor_object = v8::Local::<v8::Object>::try_from(constructor)
        .map_err(|_| cb::type_error(scope, "Tile is not a constructor"))?;
    if cb::catching(scope, |s| value.instance_of(s, constructor_object))? {
        Ok(value)
    } else {
        copy_tile(scope, constructor, value)
    }
}

fn is_safe_integer(scope: &mut v8::HandleScope, value: v8::Local<v8::Value>) -> bool {
    value.is_number()
        && value.number_value(scope).is_some_and(|number| {
            number.is_finite() && number.fract() == 0.0 && number.abs() <= 9_007_199_254_740_991.0
        })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_distance_keeps_planar_level_and_nan_semantics_without_heap_state() {
        assert_eq!(std::mem::size_of::<JsTile>(), 24);
        assert!(!std::mem::needs_drop::<JsTile>());
        let from = JsTile {
            x: 10.0,
            z: 20.0,
            level: 2.0,
        };
        let to = JsTile {
            x: 13.0,
            z: 16.0,
            level: 1.0,
        };
        assert_eq!(js_tile_distance(from, to, false), 1_000_004.0);
        assert_eq!(js_tile_distance(from, to, true), 4.0);
        assert!(js_tile_distance(
            JsTile {
                x: f64::NAN,
                ..from
            },
            to,
            false
        )
        .is_nan());
        let measured = allocation_counter::measure(|| {
            for _ in 0..1_000 {
                std::hint::black_box(js_tile_distance(from, to, false));
            }
        });
        eprintln!("world distance core: JsTile=24B warm_1000={measured:?}");
        assert_eq!(measured.count_total, 0);
        assert_eq!(measured.bytes_current, 0);
    }
}
