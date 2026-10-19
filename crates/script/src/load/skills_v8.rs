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
    let stats = stats(scope)?;
    let unused_wanted = v8::undefined(scope).into();
    let position = find_index(scope, stats, name, unused_wanted, true)?;
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
    let base = cb::call_method(
        scope,
        skills,
        "level",
        &[hitpoints],
        "Skills.level",
    )?;
    let zero = cb::num(scope, 0.0);
    if cb::le(scope, base, zero)? {
        return Err(cb::not_impl(scope, "Skills.hpFraction"));
    }

    let effective = cb::call_method(
        scope,
        skills,
        "effective",
        &[hitpoints],
        "Skills.effective",
    )?;
    cb::div(scope, effective, base)
}

fn stats<'s>(
    scope: &mut v8::HandleScope<'s>,
) -> cb::JsResult<'s, v8::Local<'s, v8::Value>> {
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
        let lowered = cb::call_method(
            scope,
            row_name,
            "toLowerCase",
            &[],
            "row.name.toLowerCase",
        )?;

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
