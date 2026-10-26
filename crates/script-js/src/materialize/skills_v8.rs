//! Local V8 access to the compat Skills methods over the posted stats snapshot.
//!
//! This reads the already-applied V8 snapshot directly. It is not an
//! isolate-to-host transport and retains no per-isolate or per-call Rust state.

use super::callback_v8 as cb;
use rustyscript::Runtime;

const HELPER: &str = "__rs2b0t_skills";

const INDEX: i32 = 0;
const XP: i32 = 1;
const LEVEL: i32 = 2;
const EFFECTIVE: i32 = 3;
const HP_FRACTION: i32 = 4;

pub(super) fn install(runtime: &mut Runtime) -> Result<(), String> {
    cb::install(runtime, HELPER, skills_callback)
}

fn skills_callback<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = run(scope, &args);
    cb::finish(scope, rv, result);
}

fn run<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> cb::JsResult<'s, v8::Local<'s, v8::Value>> {
    let operation = args.get(0);
    if !operation.is_int32() {
        return Err(cb::not_impl(scope, "Skills"));
    }
    let operation = operation.int32_value(scope).unwrap_or(-1);
    match operation {
        INDEX => index(scope, args.get(1)),
        XP => stat(scope, args.get(1), "Skills.xp", "xp"),
        LEVEL => stat(scope, args.get(1), "Skills.level", "base"),
        EFFECTIVE => stat(scope, args.get(1), "Skills.effective", "effective"),
        HP_FRACTION => hp_fraction(scope, args.get(1)),
        _ => Err(cb::not_impl(scope, "Skills")),
    }
}

fn index<'s>(
    scope: &mut v8::HandleScope<'s>,
    name: v8::Local<'s, v8::Value>,
) -> cb::JsResult<'s, v8::Local<'s, v8::Value>> {
    // The shim's `String(name).toLowerCase()` runs before it reads stats.
    let wanted = lower_string(scope, name)?;
    let stats = stats(scope)?;
    let unused_name = v8::undefined(scope).into();
    find_index(scope, stats, unused_name, wanted, false)
}

fn stat<'s>(
    scope: &mut v8::HandleScope<'s>,
    name: v8::Local<'s, v8::Value>,
    operation: &'static str,
    field: &'static str,
) -> cb::JsResult<'s, v8::Local<'s, v8::Value>> {
    // `mustStat` runs `stats().findIndex(...)` and then calls `stats()` again
    // for the row. Keep the second snapshot read, even when it differs.
    let rows = stats(scope)?;
    let unused_wanted = v8::undefined(scope).into();
    let position = find_index(scope, rows, name, unused_wanted, true)?;
    let missing = cb::num(scope, -1.0);
    if position.strict_equals(missing) {
        return Err(cb::not_impl(scope, operation));
    }

    let fresh_stats = stats(scope)?;
    let row = cb::catching(scope, |s| fresh_stats.to_object(s)?.get(s, position))?;
    if !cb::truthy(scope, row) {
        return Err(cb::not_impl(scope, operation));
    }

    // The old shim reads this field once for typeof and again for its return.
    // The second getter result is intentionally returned without revalidation.
    if !cb::get(scope, row, field)?.is_number() {
        return Err(cb::not_impl(scope, operation));
    }
    cb::get(scope, row, field)
}

fn hp_fraction<'s>(
    scope: &mut v8::HandleScope<'s>,
    skills: v8::Local<'s, v8::Value>,
) -> cb::JsResult<'s, v8::Local<'s, v8::Value>> {
    let hitpoints = cb::string(scope, "hitpoints");
    let base = cb::call_method(scope, skills, "level", &[hitpoints], "Skills.level")?;
    let zero = cb::num(scope, 0.0);
    if cb::le(scope, base, zero)? {
        return Err(cb::not_impl(scope, "Skills.hpFraction"));
    }

    let effective = cb::call_method(scope, skills, "effective", &[hitpoints], "Skills.effective")?;
    cb::div(scope, effective, base)
}

fn stats<'s>(scope: &mut v8::HandleScope<'s>) -> cb::JsResult<'s, v8::Local<'s, v8::Value>> {
    // Mirror `_kernel.js`: host() || {}, then snapshot || {}, then stats || [].
    let global = scope.get_current_context().global(scope);
    let host = cb::get(scope, global.into(), "__rs2b0t_host")?;
    let host = if cb::truthy(scope, host) {
        host
    } else {
        v8::Object::new(scope).into()
    };
    let snapshot = cb::get(scope, host, "snapshot")?;
    let snapshot = if cb::truthy(scope, snapshot) {
        snapshot
    } else {
        v8::Object::new(scope).into()
    };
    let stats = cb::get(scope, snapshot, "stats")?;
    if cb::truthy(scope, stats) {
        Ok(stats)
    } else {
        Ok(v8::Array::new(scope, 0).into())
    }
}

fn lower_string<'s>(
    scope: &mut v8::HandleScope<'s>,
    value: v8::Local<'s, v8::Value>,
) -> cb::JsResult<'s, v8::Local<'s, v8::Value>> {
    // Use the actual global String call (not abstract ToString): String(Symbol)
    // is legal, and a script may observe or replace the global function.
    let global = scope.get_current_context().global(scope);
    let string = cb::get(scope, global.into(), "String")?;
    let undefined = v8::undefined(scope).into();
    let converted = cb::call(scope, string, undefined, &[value], "String")?;
    cb::call_method(
        scope,
        converted,
        "toLowerCase",
        &[],
        "String(name).toLowerCase",
    )
}

fn find_index<'s>(
    scope: &mut v8::HandleScope<'s>,
    stats: v8::Local<'s, v8::Value>,
    name: v8::Local<'s, v8::Value>,
    wanted: v8::Local<'s, v8::Value>,
    stringify_per_row: bool,
) -> cb::JsResult<'s, v8::Local<'s, v8::Value>> {
    let stringify_per_row = v8::Boolean::new(scope, stringify_per_row).into();
    let data = cb::object(
        scope,
        &[
            ("name", name),
            ("wanted", wanted),
            ("stringifyPerRow", stringify_per_row),
        ],
    )?;
    let Some(predicate) = v8::Function::builder(skill_row_matches)
        .data(data)
        .build(scope)
    else {
        return Err(cb::type_error(scope, "could not create Skills predicate"));
    };
    cb::call_method(
        scope,
        stats,
        "findIndex",
        &[predicate.into()],
        "stats.findIndex",
    )
}

fn skill_row_matches<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = (|| -> cb::JsResult<'s, bool> {
        let row = args.get(0);
        if !cb::truthy(scope, row) {
            return Ok(false);
        }

        // Both property reads are intentional: the JS expression uses one for
        // typeof and a fresh second read before calling toLowerCase.
        if !cb::get(scope, row, "name")?.is_string() {
            return Ok(false);
        }
        let row_name = cb::get(scope, row, "name")?;
        let lowered = cb::call_method(scope, row_name, "toLowerCase", &[], "row.name.toLowerCase")?;

        let data = args.data();
        let stringify_per_row = cb::get(scope, data, "stringifyPerRow")?;
        let wanted = if cb::truthy(scope, stringify_per_row) {
            let name = cb::get(scope, data, "name")?;
            lower_string(scope, name)?
        } else {
            cb::get(scope, data, "wanted")?
        };
        Ok(lowered.strict_equals(wanted))
    })();
    let result = match result {
        Ok(matches) => Ok(v8::Boolean::new(scope, matches).into()),
        Err(error) => Err(error),
    };
    cb::finish(scope, rv, result);
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustyscript::RuntimeOptions;

    const SKILLS_SETUP: &str = r#"
globalThis.__rs2b0t_host = { snapshot: { stats: [
    { name: 'attack', xp: 123, base: 50, effective: 60 },
    { name: 'hitpoints', xp: 456, base: 10, effective: 8 },
] } };
const skills = {
    level: name => __rs2b0t_skills(2, name),
    effective: name => __rs2b0t_skills(3, name),
};
globalThis.checkSkills = () =>
    __rs2b0t_skills(0, 'ATTACK') === 0 &&
    __rs2b0t_skills(1, 'attack') === 123 &&
    __rs2b0t_skills(2, 'attack') === 50 &&
    __rs2b0t_skills(3, 'attack') === 60 &&
    __rs2b0t_skills(4, skills) === 0.8;
"#;

    fn scope_cache_control<'s>(
        scope: &mut v8::HandleScope<'s>,
        args: v8::FunctionCallbackArguments<'s>,
        mut rv: v8::ReturnValue,
    ) {
        let first = &mut v8::TryCatch::new(scope);
        let second = &mut v8::TryCatch::new(first);
        let third = &mut v8::TryCatch::new(second);
        let fourth = &mut v8::TryCatch::new(third);
        let fifth = &mut v8::TryCatch::new(fourth);
        if args.get(0).is_true() {
            let sixth = &mut v8::TryCatch::new(fifth);
            rv.set(v8::Boolean::new(sixth, true).into());
        } else {
            rv.set(v8::Boolean::new(fifth, true).into());
        }
    }

    #[test]
    fn cold_native_bridge_charge_is_a_reusable_per_depth_scope_cache() {
        crate::platform::bootstrap_test_v8_platform();
        let mut runtime = Runtime::new(RuntimeOptions::default()).unwrap();
        cb::install_ops(&mut runtime).unwrap();
        install(&mut runtime).unwrap();
        runtime.eval::<()>(SKILLS_SETUP).unwrap();
        cb::install(&mut runtime, "__scope_cache_control", scope_cache_control).unwrap();
        runtime
            .eval::<()>(
                r#"
globalThis.checkPureJs = () => true;
globalThis.checkFiveScopes = () => __scope_cache_control(false);
globalThis.checkSixScopes = () => __scope_cache_control(true);
"#,
            )
            .unwrap();
        let context = runtime.deno_runtime().main_context();
        let mut scope = runtime.deno_runtime().handle_scope();
        let global = context.open(&mut scope).global(&mut scope);
        let key = cb::string(&mut scope, "checkPureJs");
        let pure_js =
            v8::Local::<v8::Function>::try_from(global.get(&mut scope, key).unwrap()).unwrap();
        let key = cb::string(&mut scope, "checkFiveScopes");
        let five_scopes =
            v8::Local::<v8::Function>::try_from(global.get(&mut scope, key).unwrap()).unwrap();
        let key = cb::string(&mut scope, "checkSixScopes");
        let six_scopes =
            v8::Local::<v8::Function>::try_from(global.get(&mut scope, key).unwrap()).unwrap();
        let key = cb::string(&mut scope, "checkSkills");
        let skills =
            v8::Local::<v8::Function>::try_from(global.get(&mut scope, key).unwrap()).unwrap();
        let receiver = v8::undefined(&mut scope).into();
        let pure = allocation_counter::measure(|| {
            assert!(pure_js.call(&mut scope, receiver, &[]).unwrap().is_true());
        });
        let cold = allocation_counter::measure(|| {
            assert!(five_scopes
                .call(&mut scope, receiver, &[])
                .unwrap()
                .is_true());
        });
        let skills_after_control = allocation_counter::measure(|| {
            assert!(skills.call(&mut scope, receiver, &[]).unwrap().is_true());
        });
        let extra_depth = allocation_counter::measure(|| {
            assert!(six_scopes
                .call(&mut scope, receiver, &[])
                .unwrap()
                .is_true());
        });
        let warm = allocation_counter::measure(|| {
            for _ in 0..1_000 {
                assert!(six_scopes
                    .call(&mut scope, receiver, &[])
                    .unwrap()
                    .is_true());
                assert!(skills.call(&mut scope, receiver, &[]).unwrap().is_true());
            }
        });
        println!(
            "Native scope-cache control pure={pure:?}; five scopes={cold:?}; \
             skills after control={skills_after_control:?}; \
             extra scope={extra_depth:?}; warm1000={warm:?}"
        );
        assert_eq!(pure, allocation_counter::AllocationInfo::default());
        assert_eq!(cold.count_total, 3);
        assert_eq!(cold.bytes_total, 3 * extra_depth.bytes_total);
        assert_eq!(
            skills_after_control,
            allocation_counter::AllocationInfo::default()
        );
        assert_eq!(extra_depth.count_total, 1);
        assert_eq!(extra_depth.count_current, 1);
        assert_eq!(extra_depth.bytes_current, extra_depth.bytes_total as i64);
        assert_eq!(warm, allocation_counter::AllocationInfo::default());
    }

    #[test]
    fn skills_callbacks_bound_cold_bridge_cache_and_keep_zero_warm_rust_heap() {
        crate::platform::bootstrap_test_v8_platform();
        let mut runtime = Runtime::new(RuntimeOptions::default()).unwrap();
        cb::install_ops(&mut runtime).unwrap();
        install(&mut runtime).unwrap();
        runtime.eval::<()>(SKILLS_SETUP).unwrap();
        let context = runtime.deno_runtime().main_context();
        let mut scope = runtime.deno_runtime().handle_scope();
        let global = context.open(&mut scope).global(&mut scope);
        let key = cb::string(&mut scope, "checkSkills");
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
        println!("Skills five-method native callbacks cold={cold:?}; warm1000={warm:?}");
        // rusty_v8 137.2.1 caches one boxed ScopeData per maximum native-scope
        // depth until isolate disposal. The independent control above incurs
        // the same three 112-byte cache cells before any Skills call, after
        // which Skills adds none. Keep that cold bridge charge inside the
        // measurement, rather than hiding it in an unmeasured warm-up.
        assert!(cold.count_total <= 3, "cold native bridge allocations");
        assert!(cold.count_current <= 3, "cold retained bridge allocations");
        assert!(cold.count_max <= 3, "cold peak bridge allocations");
        assert!(cold.bytes_total <= 336, "cold native bridge bytes");
        assert!(cold.bytes_current <= 336, "cold retained bridge bytes");
        assert!(cold.bytes_max <= 336, "cold peak bridge bytes");
        assert_eq!(warm, allocation_counter::AllocationInfo::default());
    }
}
