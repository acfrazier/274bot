//! Typed native V8 helpers for the generic gathering-location API.
//!
//! The shim only maps public names to this callback. Location rows remain
//! caller-owned V8 values, and each exported helper makes one native call.

use super::bank_locations_v8;
use super::callback_v8::{self as cb, Flow, JsResult};
use rustyscript::Runtime;

const DEFAULT_BOOTH_NAME: &str = "Bank booth";
const DEFAULT_BOOTH_OP: &str = "Use-quickly";
const MAP_SQUARE: f64 = 64.0;

const AUTO_LEGACY: &str = "Auto";
const USE_CLOSEST: &str = "Use Closest";
const USE_START_POSITION: &str = "Use Start Position";
const USE_CUSTOM_POSITION: &str = "Use Custom Position";
const NONE_LEGACY: &str = "None";

pub(super) fn install(runtime: &mut Runtime) -> Result<(), String> {
    cb::install(runtime, "__rs2b0t_gathering_locations", gathering_callback)
}

fn gathering_callback<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = dispatch(scope, &args);
    cb::finish(scope, rv, result);
}

fn dispatch<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let op = args.get(0);
    if op.strict_equals(cb::string(scope, "DEFAULT_BOOTH_NAME")) {
        return Ok(cb::string(scope, DEFAULT_BOOTH_NAME));
    }
    if op.strict_equals(cb::string(scope, "DEFAULT_BOOTH_OP")) {
        return Ok(cb::string(scope, DEFAULT_BOOTH_OP));
    }
    if op.strict_equals(cb::string(scope, "MAP_SQUARE")) {
        return Ok(cb::num(scope, MAP_SQUARE));
    }
    if op.strict_equals(cb::string(scope, "sameMapSquare")) {
        let same = same_map_square(scope, args.get(1), args.get(2))?;
        return Ok(v8::Boolean::new(scope, same).into());
    }
    if op.strict_equals(cb::string(scope, "locationOptions")) {
        return location_options(scope, args.get(1));
    }
    if op.strict_equals(cb::string(scope, "boothFields")) {
        return booth_fields(scope, args.get(1));
    }
    if op.strict_equals(cb::string(scope, "resolveGatheringLocation")) {
        return resolve_gathering_location(scope, args.get(1), args.get(2), args.get(3));
    }
    Err(cb::not_impl(scope, "gathering location helper"))
}

/// Frozen `sameMapSquare`: strict level equality, then floor-divided x and z
/// chunks. Read only the fields reached before the frozen `&&` short circuit.
fn same_map_square<'s>(
    scope: &mut v8::HandleScope<'s>,
    a: v8::Local<'s, v8::Value>,
    b: v8::Local<'s, v8::Value>,
) -> JsResult<'s, bool> {
    let a_level = cb::get(scope, a, "level")?;
    let b_level = cb::get(scope, b, "level")?;
    if !a_level.strict_equals(b_level) {
        return Ok(false);
    }

    let a_x = floor_chunk(scope, a, "x")?;
    let b_x = floor_chunk(scope, b, "x")?;
    if !a_x.strict_equals(b_x) {
        return Ok(false);
    }

    let a_z = floor_chunk(scope, a, "z")?;
    let b_z = floor_chunk(scope, b, "z")?;
    Ok(a_z.strict_equals(b_z))
}

fn floor_chunk<'s>(
    scope: &mut v8::HandleScope<'s>,
    tile: v8::Local<'s, v8::Value>,
    coordinate: &str,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let global = scope.get_current_context().global(scope);
    let math = cb::get(scope, global.into(), "Math")?;
    let floor = cb::get(scope, math, "floor")?;
    let value = cb::get(scope, tile, coordinate)?;
    let map_square = cb::num(scope, MAP_SQUARE);
    let quotient = cb::div(scope, value, map_square)?;
    cb::call(scope, floor, math, &[quotient], "floor")
}

/// Frozen `[...constants, ...table.map(location => location.name)]`.
pub(super) fn location_options<'s>(
    scope: &mut v8::HandleScope<'s>,
    table: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let prefix = [
        cb::string(scope, AUTO_LEGACY),
        cb::string(scope, USE_CLOSEST),
        cb::string(scope, USE_START_POSITION),
        cb::string(scope, USE_CUSTOM_POSITION),
        cb::string(scope, NONE_LEGACY),
    ];
    let output = v8::Array::new_with_elements(scope, &prefix);
    let map = cb::get(scope, table, "map")?;
    let Some(map_callback) = v8::Function::builder(map_name_callback).build(scope) else {
        return Err(cb::type_error(scope, "could not create gathering map callback"));
    };
    let map_callback: v8::Local<v8::Value> = map_callback.into();
    let mapped = cb::call(scope, map, table, &[map_callback], "map")?;
    let iterator = SpreadIterator::open(scope, mapped)?;
    let mut index = prefix.len() as u32;

    loop {
        let inner = &mut v8::EscapableHandleScope::new(scope);
        let flow = match iterator.step(inner) {
            Ok(Some(value)) => Ok(Flow::Continue(Some(value))),
            Ok(None) => Ok(Flow::Break(None)),
            Err(error) => Err(error),
        };
        match cb::iteration(inner, flow)? {
            Flow::Continue(Some(value)) => {
                if index == u32::MAX {
                    let error =
                        cb::type_error(scope, "gathering options exceed V8 array length");
                    return iterator.body(scope, Err(error));
                }
                let append = match cb::catching(scope, |s| output.set_index(s, index, value)) {
                    Ok(true) => Ok(()),
                    Ok(false) => Err(cb::type_error(scope, "could not append gathering option")),
                    Err(error) => Err(error),
                };
                iterator.body(scope, append)?;
                index += 1;
            }
            Flow::Break(_) => break,
            Flow::Continue(None) => unreachable!("gathering spread yielded no value"),
        }
    }
    Ok(output.into())
}

/// A no-allocation `GetIterator` path for the spread in `locationOptions`.
/// `callback_v8::ForOf` builds formatted error text while opening, which would
/// allocate a Rust `String` on this successful helper path.
struct SpreadIterator<'s> {
    iterator: v8::Local<'s, v8::Value>,
    next: v8::Local<'s, v8::Value>,
    poll: cb::Poll,
}

impl<'s> SpreadIterator<'s> {
    fn open(
        scope: &mut v8::HandleScope<'s>,
        iterable: v8::Local<'s, v8::Value>,
    ) -> JsResult<'s, Self> {
        if iterable.is_null_or_undefined() {
            return Err(cb::type_error(scope, "location options is not iterable"));
        }
        let symbol: v8::Local<v8::Value> = v8::Symbol::get_iterator(scope).into();
        let method = cb::catching(scope, |s| iterable.to_object(s)?.get(s, symbol))?;
        if !method.is_function() {
            return Err(cb::type_error(scope, "location options is not iterable"));
        }
        let iterator = cb::call(scope, method, iterable, &[], "Symbol.iterator")?;
        if !iterator.is_object() {
            return Err(cb::type_error(
                scope,
                "Result of the Symbol.iterator method is not an object",
            ));
        }
        let next = cb::get(scope, iterator, "next")?;
        Ok(Self {
            iterator,
            next,
            poll: cb::Poll::default(),
        })
    }

    fn step(
        &self,
        scope: &mut v8::HandleScope<'s>,
    ) -> JsResult<'s, Option<v8::Local<'s, v8::Value>>> {
        self.poll.check(scope)?;
        let result = cb::call(scope, self.next, self.iterator, &[], "iterator next")?;
        if !result.is_object() {
            return Err(cb::type_error(scope, "Iterator result is not an object"));
        }
        let done = cb::get(scope, result, "done")?;
        if cb::truthy(scope, done) {
            Ok(None)
        } else {
            cb::get(scope, result, "value").map(Some)
        }
    }

    fn body<T>(
        &self,
        scope: &mut v8::HandleScope<'s>,
        outcome: JsResult<'s, T>,
    ) -> JsResult<'s, T> {
        let thrown = match outcome {
            Ok(value) => return Ok(value),
            Err(cb::Throw::Terminated) => return Err(cb::Throw::Terminated),
            Err(thrown) => thrown,
        };
        match self.close(scope) {
            Err(cb::Throw::Terminated) => Err(cb::Throw::Terminated),
            _ => Err(thrown),
        }
    }

    fn close(&self, scope: &mut v8::HandleScope<'s>) -> JsResult<'s, ()> {
        let ret = cb::get(scope, self.iterator, "return")?;
        if ret.is_null_or_undefined() {
            return Ok(());
        }
        let result = cb::call(scope, ret, self.iterator, &[], "iterator return")?;
        if !result.is_object() {
            return Err(cb::type_error(scope, "Iterator result is not an object"));
        }
        Ok(())
    }
}


fn map_name_callback<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = cb::get(scope, args.get(0), "name");
    cb::finish(scope, rv, result);
}

/// Frozen `loc?.boothName ?? DEFAULT_BOOTH_NAME` and `loc?.boothOp ?? ...`.
fn booth_fields<'s>(
    scope: &mut v8::HandleScope<'s>,
    location: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let booth_name = if location.is_null_or_undefined() {
        cb::string(scope, DEFAULT_BOOTH_NAME)
    } else {
        let value = cb::get(scope, location, "boothName")?;
        if value.is_null_or_undefined() {
            cb::string(scope, DEFAULT_BOOTH_NAME)
        } else {
            value
        }
    };
    let booth_op = if location.is_null_or_undefined() {
        cb::string(scope, DEFAULT_BOOTH_OP)
    } else {
        let value = cb::get(scope, location, "boothOp")?;
        if value.is_null_or_undefined() {
            cb::string(scope, DEFAULT_BOOTH_OP)
        } else {
            value
        }
    };
    cb::object(scope, &[("boothName", booth_name), ("boothOp", booth_op)])
}

/// Frozen `setting.trim().toLowerCase()` and its aliases, pool, stable rank,
/// or first exact name match. A found row is returned as its original V8 value.
pub(super) fn resolve_gathering_location<'s>(
    scope: &mut v8::HandleScope<'s>,
    setting: v8::Local<'s, v8::Value>,
    start_tile: v8::Local<'s, v8::Value>,
    table: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let trim = cb::get(scope, setting, "trim")?;
    let trimmed = cb::call(scope, trim, setting, &[], "trim")?;
    let to_lower_case = cb::get(scope, trimmed, "toLowerCase")?;
    let normalized = cb::call(scope, to_lower_case, trimmed, &[], "toLowerCase")?;

    if normalized.strict_equals(cb::string(scope, "use custom position"))
        || normalized.strict_equals(cb::string(scope, "use start position"))
    {
        return Ok(v8::null(scope).into());
    }

    let closest = normalized.strict_equals(cb::string(scope, "use closest"));
    let auto = normalized.strict_equals(cb::string(scope, "auto"));
    if closest || auto {
        let pool = if auto {
            filtered_same_square(scope, table, start_tile)?
        } else {
            table
        };
        return nearest_row(scope, start_tile, pool);
    }

    find_named_row(scope, table, normalized)
}

fn filtered_same_square<'s>(
    scope: &mut v8::HandleScope<'s>,
    table: v8::Local<'s, v8::Value>,
    start_tile: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let filter = cb::get(scope, table, "filter")?;
    let Some(predicate) = v8::Function::builder(filter_same_square_callback)
        .data(start_tile)
        .build(scope)
    else {
        return Err(cb::type_error(
            scope,
            "could not create gathering filter callback",
        ));
    };
    let predicate: v8::Local<v8::Value> = predicate.into();
    cb::call(scope, filter, table, &[predicate], "filter")
}

fn filter_same_square_callback<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = (|| {
        let location = args.get(0);
        let spot = cb::get(scope, location, "spot")?;
        let same = same_map_square(scope, args.data(), spot)?;
        Ok(v8::Boolean::new(scope, same).into())
    })();
    cb::finish(scope, rv, result);
}

fn nearest_row<'s>(
    scope: &mut v8::HandleScope<'s>,
    start_tile: v8::Local<'s, v8::Value>,
    pool: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let length = cb::get(scope, pool, "length")?;
    if length.strict_equals(cb::num(scope, 0.0)) {
        return Ok(v8::null(scope).into());
    }

    let mut best = cb::get_index(scope, pool, 0)?;
    let best_spot = cb::get(scope, best, "spot")?;
    let mut best_distance = bank_distance(scope, start_tile, best_spot)?;
    let mut index = 1.0;

    loop {
        let length = cb::get(scope, pool, "length")?;
        let index_value = cb::num(scope, index);
        if !cb::lt(scope, index_value, length)? {
            break;
        }
        let location = cb::get_index(scope, pool, index as u32)?;
        let spot = cb::get(scope, location, "spot")?;
        let distance = bank_distance(scope, start_tile, spot)?;
        if distance < best_distance {
            best = location;
            best_distance = distance;
        }
        index += 1.0;
    }
    Ok(best)
}

/// Read the frozen bankDistance operands in order. `sub` performs V8
/// ToPrimitive/ToNumeric, then the existing native bank-distance owner applies
/// its shared `hypot` core to the two deltas.
fn bank_distance<'s>(
    scope: &mut v8::HandleScope<'s>,
    from: v8::Local<'s, v8::Value>,
    bank: v8::Local<'s, v8::Value>,
) -> JsResult<'s, f64> {
    let bank_x = cb::get(scope, bank, "x")?;
    let from_x = cb::get(scope, from, "x")?;
    let dx = cb::sub(scope, bank_x, from_x)?;

    let bank_z = cb::get(scope, bank, "z")?;
    let from_z = cb::get(scope, from, "z")?;
    let dz = cb::sub(scope, bank_z, from_z)?;

    let dx = cb::number(scope, dx)?;
    let dz = cb::number(scope, dz)?;
    Ok(bank_locations_v8::bank_distance_delta(dx, dz))
}

fn find_named_row<'s>(
    scope: &mut v8::HandleScope<'s>,
    table: v8::Local<'s, v8::Value>,
    normalized: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let find = cb::get(scope, table, "find")?;
    let Some(predicate) = v8::Function::builder(find_named_callback)
        .data(normalized)
        .build(scope)
    else {
        return Err(cb::type_error(scope, "could not create gathering find callback"));
    };
    let predicate: v8::Local<v8::Value> = predicate.into();
    let found = cb::call(scope, find, table, &[predicate], "find")?;
    if found.is_null_or_undefined() {
        Ok(v8::null(scope).into())
    } else {
        Ok(found)
    }
}

fn find_named_callback<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = (|| {
        let name = cb::get(scope, args.get(0), "name")?;
        let to_lower_case = cb::get(scope, name, "toLowerCase")?;
        let lowered = cb::call(scope, to_lower_case, name, &[], "toLowerCase")?;
        Ok(v8::Boolean::new(scope, lowered.strict_equals(args.data())).into())
    })();
    cb::finish(scope, rv, result);
}
