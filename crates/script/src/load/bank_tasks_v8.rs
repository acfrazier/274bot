//! TaskBot `validate` for PeriodicBank and DeathRecovery as one native
//! call each. The shim passes the caller's options object; Rust calls its
//! functions in the frozen order through the one callback path and reads
//! the scene for the rest.
//!
//! - `__rs2b0t_periodic_bank_validate(opts)`: frozen `PeriodicBank.validate`
//!   (`strategy()`, combat, the failure backoff, then `strategy()`,
//!   `countLoot()`, `itemsThreshold()`, `minutesThreshold()` into
//!   `shouldBankNow`). An absent option keeps the shim's default (`'off'`,
//!   0, 15, 10).
//! - `__rs2b0t_death_recovery_validate(opts)`: observe the posted chat,
//!   call `opts.onDeath()` on a new death, answer due.

use super::callback_v8::{self as cb, Callback, JsResult};
use crate::periodic_bank::{
    minutes_since_last_bank, parse_bank_strategy, should_bank_now, suppressed,
};
use rustyscript::Runtime;

pub(super) fn install(runtime: &mut Runtime) -> Result<(), String> {
    cb::install(
        runtime,
        "__rs2b0t_periodic_bank_validate",
        periodic_bank_validate,
    )?;
    cb::install(
        runtime,
        "__rs2b0t_death_recovery_validate",
        death_recovery_validate,
    )
}

/// `opts[key]()` when it is a function (receiver `opts`), else `None`.
fn call_option<'s>(
    scope: &mut v8::HandleScope<'s>,
    opts: v8::Local<'s, v8::Value>,
    key: &'static str,
) -> JsResult<'s, Option<v8::Local<'s, v8::Value>>> {
    let func = cb::get(scope, opts, key)?;
    if !func.is_function() {
        return Ok(None);
    }
    Callback::method(func, opts, key).call(scope, &[]).map(Some)
}

fn number_option<'s>(
    scope: &mut v8::HandleScope<'s>,
    opts: v8::Local<'s, v8::Value>,
    key: &'static str,
    absent: f64,
) -> JsResult<'s, f64> {
    match call_option(scope, opts, key)? {
        Some(value) => cb::number(scope, value),
        None => Ok(absent),
    }
}

/// The strategy string, or `None` for a non-string or an absent option.
fn strategy<'s>(
    scope: &mut v8::HandleScope<'s>,
    opts: v8::Local<'s, v8::Value>,
) -> JsResult<'s, Option<String>> {
    Ok(call_option(scope, opts, "strategy")?
        .filter(|value| value.is_string())
        .map(|value| value.to_rust_string_lossy(scope)))
}

fn periodic_due<'s>(
    scope: &mut v8::HandleScope<'s>,
    opts: v8::Local<'s, v8::Value>,
) -> JsResult<'s, bool> {
    // The shim's absent strategy is 'off'.
    let first = match call_option(scope, opts, "strategy")? {
        None => return Ok(false),
        Some(value) => value,
    };
    let off = first.is_string() && first.to_rust_string_lossy(scope) == "off";
    let in_combat = crate::observed::with(|scene| scene.since_login().in_combat().unwrap_or(false));
    if off || in_combat || suppressed() {
        return Ok(false);
    }
    let strategy = parse_bank_strategy(strategy(scope, opts)?.as_deref().unwrap_or("off"));
    let loot = number_option(scope, opts, "countLoot", 0.0)?;
    let minutes = minutes_since_last_bank();
    let items_threshold = number_option(scope, opts, "itemsThreshold", 15.0)?;
    let minutes_threshold = number_option(scope, opts, "minutesThreshold", 10.0)?;
    Ok(should_bank_now(
        strategy,
        loot,
        items_threshold,
        minutes,
        minutes_threshold,
    ))
}

fn periodic_bank_validate<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = periodic_due(scope, args.get(0)).map(|due| v8::Boolean::new(scope, due).into());
    cb::finish(scope, rv, result);
}

fn death_due<'s>(
    scope: &mut v8::HandleScope<'s>,
    opts: v8::Local<'s, v8::Value>,
) -> JsResult<'s, bool> {
    let (due, died) = crate::death_recovery::validate();
    if died {
        call_option(scope, opts, "onDeath")?;
    }
    Ok(due)
}

fn death_recovery_validate<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = death_due(scope, args.get(0)).map(|due| v8::Boolean::new(scope, due).into());
    cb::finish(scope, rv, result);
}
