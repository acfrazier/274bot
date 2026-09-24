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
//!   call `opts.onDeath()` on a new death; after a finished run, clear the
//!   latch and call `opts.onRecovered()` when frozen `near` holds; answer
//!   due.
//! - `__rs2b0t_next_withdraw_chunk(need)`: frozen `nextWithdrawChunk`
//!   ([`crate::bank_withdraw::next_chunk`]), `null` or `{ kind, count }` /
//!   `{ kind, op }`.

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
    )?;
    cb::install(runtime, "__rs2b0t_next_withdraw_chunk", next_withdraw_chunk)
}

fn next_withdraw_chunk<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    use crate::bank_withdraw::{next_chunk, Chunk};
    let result = cb::number(scope, args.get(0)).and_then(|need| match next_chunk(need) {
        None => Ok(v8::null(scope).into()),
        Some(Chunk::X(count)) => {
            let kind = cb::string(scope, "x");
            let count = cb::num(scope, count);
            cb::object(scope, &[("kind", kind), ("count", count)])
        }
        Some(Chunk::Op(op)) => {
            let kind = cb::string(scope, "op");
            let op = cb::string(scope, op);
            cb::object(scope, &[("kind", kind), ("op", op)])
        }
    });
    cb::finish(scope, rv, result);
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
    use crate::death_recovery as death;
    let observed = death::observe();
    if observed.died {
        call_option(scope, opts, "onDeath")?;
    }
    if let Some(home) = observed.check_from {
        // Frozen `near(home, this.opts.anchor, this.opts.radius ?? 6)`:
        // `a.level === b.level && Math.abs(a.x - b.x) <= r && …`.
        let anchor = cb::get(scope, opts, "anchor")?;
        let radius = cb::get(scope, opts, "radius")?;
        let radius = if radius.is_null_or_undefined() {
            f64::from(death::DEFAULT_RADIUS)
        } else {
            cb::number(scope, radius)?
        };
        let level = cb::get(scope, anchor, "level")?;
        let level = level
            .is_number()
            .then(|| level.number_value(scope).unwrap_or(f64::NAN));
        let near = if level == Some(f64::from(home.level)) {
            let x = cb::get(scope, anchor, "x")?;
            let x = cb::number(scope, x)?;
            if (f64::from(home.x) - x).abs() <= radius {
                let z = cb::get(scope, anchor, "z")?;
                let z = cb::number(scope, z)?;
                death::near(home, level, x, z, radius)
            } else {
                false
            }
        } else {
            false
        };
        if near {
            death::recover();
            call_option(scope, opts, "onRecovered")?;
        }
    }
    Ok(death::due())
}

fn death_recovery_validate<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = death_due(scope, args.get(0)).map(|due| v8::Boolean::new(scope, due).into());
    cb::finish(scope, rv, result);
}
