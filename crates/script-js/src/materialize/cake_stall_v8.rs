//! Direct V8 binding for cake-stall facts held by the Rust snapshot observer.
//!
//! These synchronous helpers use the same native callback boundary as the other
//! typed v8 helpers. They do not register a rustyscript JSON function or copy
//! the inventory through a JS/Rust JSON value.

use super::callback_v8::{self as cb, JsResult};
use rustyscript::Runtime;

pub(super) fn install(runtime: &mut Runtime) -> Result<(), String> {
    cb::install(runtime, "__rs2b0t_cake_stall", cake_stall)
}

fn cake_stall<'s>(
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
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    match args.get(0).to_rust_string_lossy(scope).as_str() {
        "count" => Ok(v8::Integer::new(scope, crate::cake_stall::carried_cakes()).into()),
        "needs_restock" => {
            let target = args.get(1);
            let target = if target.is_null() || target.is_undefined() {
                None
            } else {
                let value = cb::number(scope, target)?;
                (value.is_finite() && value.fract() == 0.0).then_some(value as i32)
            };
            Ok(v8::Boolean::new(
                scope,
                crate::cake_stall::needs_cake_restock_from_snapshot(target),
            )
            .into())
        }
        "data" => cake_data(scope),
        "classify_steal" => classify_steal(scope, args.get(1)),
        "should_reset" => should_reset(scope, args.get(1)),
        _ => Err(cb::not_impl(scope, "cakeStall")),
    }
}

fn cake_data<'s>(scope: &mut v8::HandleScope<'s>) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let items: Vec<_> = script::cake_stall::CAKE_ITEMS
        .iter()
        .map(|name| cb::string(scope, name))
        .collect();
    let items = v8::Array::new_with_elements(scope, &items).into();
    let lockout_ticks = cb::num(scope, script::cake_stall::LOCKOUT_TICKS as f64);
    let reset_after_refusals = cb::num(scope, f64::from(script::cake_stall::RESET_AFTER_REFUSALS));
    cb::object(
        scope,
        &[
            ("cakeItems", items),
            ("lockoutTicks", lockout_ticks),
            ("resetAfterRefusals", reset_after_refusals),
        ],
    )
}

fn classify_steal<'s>(
    scope: &mut v8::HandleScope<'s>,
    signals: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let gained = cb::get(scope, signals, "gained")?;
    let gained = cb::truthy(scope, gained);
    let combat = if gained {
        false
    } else {
        let combat = cb::get(scope, signals, "combat")?;
        cb::truthy(scope, combat)
    };
    let lockout_seen = if gained || combat {
        false
    } else {
        let lockout_seen = cb::get(scope, signals, "lockoutSeen")?;
        cb::truthy(scope, lockout_seen)
    };
    let attempt_seen = if gained || combat || lockout_seen {
        false
    } else {
        let attempt_seen = cb::get(scope, signals, "attemptSeen")?;
        cb::truthy(scope, attempt_seen)
    };
    Ok(cb::string(
        scope,
        script::cake_stall::classify_steal(gained, combat, lockout_seen, attempt_seen),
    ))
}

fn should_reset<'s>(
    scope: &mut v8::HandleScope<'s>,
    consecutive_refusals: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let consecutive_refusals = cb::number(scope, consecutive_refusals)?;
    Ok(v8::Boolean::new(
        scope,
        script::cake_stall::should_reset(consecutive_refusals),
    )
    .into())
}
