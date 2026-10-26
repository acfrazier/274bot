//! V8 conversion for the shared typed gathering-location helpers.
//!
//! The shim only maps public names to this callback. Location rows remain
//! caller-owned V8 values, and each exported helper makes one native call.

use super::callback_v8::{self as cb, Flow, JsResult};
use rustyscript::Runtime;
use script::gather_locations::{
    self, Axis, Resolution, Resolver, DEFAULT_BOOTH_NAME, DEFAULT_BOOTH_OP, MAP_SQUARE,
};
use std::ops::ControlFlow;

const BOOTH_NAME: i32 = 0;
const BOOTH_OP: i32 = 1;
const MAP_SQUARE_OP: i32 = 2;
const SAME_MAP_SQUARE: i32 = 3;
const LOCATION_OPTIONS: i32 = 4;
const BOOTH_FIELDS: i32 = 5;
const RESOLVE: i32 = 6;

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
    let operation = args.get(0);
    if !operation.is_int32() {
        return Err(cb::not_impl(scope, "gathering location helper"));
    }
    match operation.int32_value(scope).unwrap_or(-1) {
        BOOTH_NAME => Ok(cb::string(scope, DEFAULT_BOOTH_NAME)),
        BOOTH_OP => Ok(cb::string(scope, DEFAULT_BOOTH_OP)),
        MAP_SQUARE_OP => Ok(cb::num(scope, MAP_SQUARE)),
        SAME_MAP_SQUARE => {
            let same = same_map_square(scope, args.get(1), args.get(2))?;
            Ok(v8::Boolean::new(scope, same).into())
        }
        LOCATION_OPTIONS => location_options(scope, args.get(1)),
        BOOTH_FIELDS => booth_fields(scope, args.get(1)),
        RESOLVE => resolve_gathering_location(scope, args.get(1), args.get(2), args.get(3)),
        _ => Err(cb::not_impl(scope, "gathering location helper")),
    }
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
    gather_locations::same_square_with(a_level.strict_equals(b_level), |axis| {
        let coordinate = match axis {
            Axis::X => "x",
            Axis::Z => "z",
        };
        let a_chunk = floor_chunk(scope, a, coordinate)?;
        let b_chunk = floor_chunk(scope, b, coordinate)?;
        Ok(a_chunk.strict_equals(b_chunk))
    })
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
    let prefix = gather_locations::OPTION_PREFIX.map(|name| cb::string(scope, name));
    let output = v8::Array::new_with_elements(scope, &prefix);
    let map = cb::get(scope, table, "map")?;
    let Some(map_callback) = v8::Function::builder(map_name_callback).build(scope) else {
        return Err(cb::type_error(
            scope,
            "could not create gathering map callback",
        ));
    };
    let map_callback: v8::Local<v8::Value> = map_callback.into();
    let mapped = cb::call(scope, map, table, &[map_callback], "map")?;
    let iterator = cb::ForOf::open(scope, mapped, "location options")?;
    let mut index = prefix.len() as u32;

    loop {
        let flow = {
            let inner = &mut v8::EscapableHandleScope::new(scope);
            let flow = match iterator.step(inner) {
                Ok(Some(value)) => Ok(Flow::Continue(Some(value))),
                Ok(None) => Ok(Flow::Break(None)),
                Err(error) => Err(error),
            };
            cb::iteration(inner, flow)
        }?;
        match flow {
            Flow::Continue(Some(value)) => {
                if index == u32::MAX {
                    let error = cb::type_error(scope, "gathering options exceed V8 array length");
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
    let name = if location.is_null_or_undefined() {
        None
    } else {
        let value = cb::get(scope, location, "boothName")?;
        (!value.is_null_or_undefined()).then_some(value)
    };
    let booth_name = gather_locations::booth_name(name, |default| cb::string(scope, default));
    let op = if location.is_null_or_undefined() {
        None
    } else {
        let value = cb::get(scope, location, "boothOp")?;
        (!value.is_null_or_undefined()).then_some(value)
    };
    let booth_op = gather_locations::booth_op(op, |default| cb::string(scope, default));
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

    let mode = resolution_mode(scope, normalized);
    let found = gather_locations::resolve_with(
        mode,
        &mut ForeignResolver {
            scope,
            start_tile,
            table,
            normalized,
        },
    )?;
    Ok(found.unwrap_or_else(|| v8::null(scope).into()))
}

fn resolution_mode(scope: &mut v8::HandleScope, normalized: v8::Local<v8::Value>) -> Resolution {
    let Ok(text) = v8::Local::<v8::String>::try_from(normalized) else {
        return Resolution::Named;
    };
    let length = text.length();
    if length > gather_locations::MAX_ALIAS_LEN {
        return Resolution::Named;
    }
    let mut units = [0_u16; gather_locations::MAX_ALIAS_LEN];
    let units = &mut units[..length];
    text.write_v2(scope, 0, units, v8::WriteFlags::empty());
    gather_locations::resolution(|alias| units.iter().copied().eq(alias.bytes().map(u16::from)))
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

struct ForeignPool<'s> {
    rows: v8::Local<'s, v8::Value>,
    index: f64,
}

struct ForeignResolver<'a, 's> {
    scope: &'a mut v8::HandleScope<'s>,
    start_tile: v8::Local<'s, v8::Value>,
    table: v8::Local<'s, v8::Value>,
    normalized: v8::Local<'s, v8::Value>,
}

impl<'s> Resolver for ForeignResolver<'_, 's> {
    type Pool = ForeignPool<'s>;
    type Row = v8::Local<'s, v8::Value>;
    type Error = cb::Throw<'s>;

    fn pool(&mut self, local_only: bool) -> JsResult<'s, Self::Pool> {
        let rows = if local_only {
            filtered_same_square(self.scope, self.table, self.start_tile)?
        } else {
            self.table
        };
        Ok(ForeignPool { rows, index: 1.0 })
    }

    fn first(&mut self, pool: &mut Self::Pool) -> JsResult<'s, Option<(Self::Row, f64)>> {
        let length = cb::get(self.scope, pool.rows, "length")?;
        if length.strict_equals(cb::num(self.scope, 0.0)) {
            return Ok(None);
        }
        let row = cb::get_index(self.scope, pool.rows, 0)?;
        let spot = cb::get(self.scope, row, "spot")?;
        let distance = bank_distance(self.scope, self.start_tile, spot)?;
        Ok(Some((row, distance)))
    }

    fn next(
        &mut self,
        pool: &mut Self::Pool,
        prefer: impl FnOnce(f64) -> bool,
    ) -> JsResult<'s, ControlFlow<(), Option<Self::Row>>> {
        let inner = &mut v8::EscapableHandleScope::new(self.scope);
        let result = (|| {
            let length = cb::get(inner, pool.rows, "length")?;
            let index_value = cb::num(inner, pool.index);
            if !cb::lt(inner, index_value, length)? {
                return Ok(Flow::Break(None));
            }
            let row = cb::get_index(inner, pool.rows, pool.index as u32)?;
            let spot = cb::get(inner, row, "spot")?;
            let distance = bank_distance(inner, self.start_tile, spot)?;
            Ok(Flow::Continue(prefer(distance).then_some(row)))
        })();
        match cb::iteration(inner, result)? {
            Flow::Continue(candidate) => {
                pool.index += 1.0;
                Ok(ControlFlow::Continue(candidate))
            }
            Flow::Break(_) => Ok(ControlFlow::Break(())),
        }
    }

    fn find_named(&mut self) -> JsResult<'s, Option<Self::Row>> {
        find_named_row(self.scope, self.table, self.normalized)
    }
}

/// Read the frozen bankDistance operands in order. `sub` performs V8
/// ToPrimitive/ToNumeric, then the shared typed core applies `hypot` to both
/// deltas. No coordinate object or row is copied into a Rust collection.
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
    Ok(gather_locations::distance_delta(dx, dz))
}

fn find_named_row<'s>(
    scope: &mut v8::HandleScope<'s>,
    table: v8::Local<'s, v8::Value>,
    normalized: v8::Local<'s, v8::Value>,
) -> JsResult<'s, Option<v8::Local<'s, v8::Value>>> {
    let find = cb::get(scope, table, "find")?;
    let Some(predicate) = v8::Function::builder(find_named_callback)
        .data(normalized)
        .build(scope)
    else {
        return Err(cb::type_error(
            scope,
            "could not create gathering find callback",
        ));
    };
    let predicate: v8::Local<v8::Value> = predicate.into();
    let found = cb::call(scope, find, table, &[predicate], "find")?;
    Ok((!found.is_null_or_undefined()).then_some(found))
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

#[cfg(test)]
mod tests {
    use super::*;
    use rustyscript::RuntimeOptions;

    const GATHERING_SETUP: &str = r#"
const from = { x: 64, z: 64, level: 0 };
const rows = [
    { name: 'Far', spot: { x: 74, z: 64, level: 0 } },
    { name: 'Cross plane', spot: { x: 67, z: 68, level: 1 } },
    { name: 'Local', spot: { x: 70, z: 72, level: 0 }, boothName: 'Chest', boothOp: 'Bank' },
];
const call = (operation, ...args) => __rs2b0t_gathering_locations(operation, ...args);
globalThis.checkGathering = () => {
    const options = call(4, rows);
    const defaults = call(5, null);
    const custom = call(5, rows[2]);
    return call(0) === 'Bank booth' &&
        call(1) === 'Use-quickly' &&
        call(2) === 64 &&
        call(3, from, rows[0].spot) &&
        !call(3, from, rows[1].spot) &&
        options.length === 8 && options[5] === 'Far' && options[7] === 'Local' &&
        defaults.boothName === 'Bank booth' && defaults.boothOp === 'Use-quickly' &&
        custom.boothName === 'Chest' && custom.boothOp === 'Bank' &&
        call(6, 'Use Closest', from, rows) === rows[1] &&
        call(6, 'Auto', from, rows) === rows[0] &&
        call(6, ' cross plane ', from, rows) === rows[1] &&
        call(6, 'Use Start Position', from, rows) === null &&
        call(6, 'Use Custom Position', from, rows) === null &&
        call(6, 'unknown', from, rows) === null;
};
"#;

    fn setup() -> Runtime {
        crate::platform::bootstrap_test_v8_platform();
        let mut runtime = Runtime::new(RuntimeOptions::default()).unwrap();
        cb::install_ops(&mut runtime).unwrap();
        install(&mut runtime).unwrap();
        runtime.eval::<()>(GATHERING_SETUP).unwrap();
        runtime
    }

    #[test]
    fn gathering_callbacks_bound_cold_bridge_cache_and_keep_zero_warm_rust_heap() {
        let mut runtime = setup();
        let context = runtime.deno_runtime().main_context();
        let mut scope = runtime.deno_runtime().handle_scope();
        let global = context.open(&mut scope).global(&mut scope);
        let key = cb::string(&mut scope, "checkGathering");
        let function =
            v8::Local::<v8::Function>::try_from(global.get(&mut scope, key).unwrap()).unwrap();
        let receiver = v8::undefined(&mut scope).into();
        let cold = allocation_counter::measure(|| {
            assert!(function.call(&mut scope, receiver, &[]).unwrap().is_true());
        });
        let warm = allocation_counter::measure(|| {
            for _ in 0..1_000 {
                assert!(function.call(&mut scope, receiver, &[]).unwrap().is_true());
            }
        });
        println!("Gathering seven-helper callbacks cold={cold:?}; warm1000={warm:?}");
        // One existing rusty_v8 ScopeData cache cell; the independent control
        // below attributes the charge without invoking a Gathering helper.
        assert!(cold.count_total <= 1);
        assert!(cold.count_current <= 1);
        assert!(cold.count_max <= 1);
        assert!(cold.bytes_total <= 112);
        assert!(cold.bytes_current <= 112);
        assert!(cold.bytes_max <= 112);
        assert_eq!(warm, allocation_counter::AllocationInfo::default());
    }

    fn scope_cache_control<'s>(
        scope: &mut v8::HandleScope<'s>,
        args: v8::FunctionCallbackArguments<'s>,
        mut rv: v8::ReturnValue,
    ) {
        let first = &mut v8::TryCatch::new(scope);
        let second = &mut v8::TryCatch::new(first);
        let third = &mut v8::TryCatch::new(second);
        if args.get(0).is_true() {
            let fourth = &mut v8::TryCatch::new(third);
            rv.set(v8::Boolean::new(fourth, true).into());
        } else {
            rv.set(v8::Boolean::new(third, true).into());
        }
    }

    #[test]
    fn gathering_cold_charge_reuses_an_independent_scope_cache_cell() {
        let mut runtime = setup();
        cb::install(&mut runtime, "__scope_cache_control", scope_cache_control).unwrap();
        runtime
            .eval::<()>(
                r#"
globalThis.checkPureJs = () => true;
globalThis.checkThreeScopes = () => __scope_cache_control(false);
globalThis.checkFourScopes = () => __scope_cache_control(true);
"#,
            )
            .unwrap();
        let context = runtime.deno_runtime().main_context();
        let mut scope = runtime.deno_runtime().handle_scope();
        let global = context.open(&mut scope).global(&mut scope);
        let key = cb::string(&mut scope, "checkPureJs");
        let pure_js =
            v8::Local::<v8::Function>::try_from(global.get(&mut scope, key).unwrap()).unwrap();
        let key = cb::string(&mut scope, "checkThreeScopes");
        let three_scopes =
            v8::Local::<v8::Function>::try_from(global.get(&mut scope, key).unwrap()).unwrap();
        let key = cb::string(&mut scope, "checkFourScopes");
        let four_scopes =
            v8::Local::<v8::Function>::try_from(global.get(&mut scope, key).unwrap()).unwrap();
        let key = cb::string(&mut scope, "checkGathering");
        let gathering =
            v8::Local::<v8::Function>::try_from(global.get(&mut scope, key).unwrap()).unwrap();
        let receiver = v8::undefined(&mut scope).into();
        let pure = allocation_counter::measure(|| {
            assert!(pure_js.call(&mut scope, receiver, &[]).unwrap().is_true());
        });
        let cold = allocation_counter::measure(|| {
            assert!(three_scopes
                .call(&mut scope, receiver, &[])
                .unwrap()
                .is_true());
        });
        let gathering_after_control = allocation_counter::measure(|| {
            assert!(gathering.call(&mut scope, receiver, &[]).unwrap().is_true());
        });
        let extra_depth = allocation_counter::measure(|| {
            assert!(four_scopes
                .call(&mut scope, receiver, &[])
                .unwrap()
                .is_true());
        });
        let warm = allocation_counter::measure(|| {
            for _ in 0..1_000 {
                assert!(four_scopes
                    .call(&mut scope, receiver, &[])
                    .unwrap()
                    .is_true());
                assert!(gathering.call(&mut scope, receiver, &[]).unwrap().is_true());
            }
        });
        println!(
            "Gathering scope-cache control pure={pure:?}; three scopes={cold:?}; \
             gathering after control={gathering_after_control:?}; \
             extra scope={extra_depth:?}; warm1000={warm:?}"
        );
        assert_eq!(pure, allocation_counter::AllocationInfo::default());
        assert_eq!(cold.count_total, 1);
        assert_eq!(cold.count_current, 1);
        assert_eq!(cold.bytes_total, 112);
        assert_eq!(cold.bytes_total, extra_depth.bytes_total);
        assert_eq!(
            gathering_after_control,
            allocation_counter::AllocationInfo::default()
        );
        assert_eq!(extra_depth.count_total, 1);
        assert_eq!(extra_depth.count_current, 1);
        assert_eq!(warm, allocation_counter::AllocationInfo::default());
    }
}
