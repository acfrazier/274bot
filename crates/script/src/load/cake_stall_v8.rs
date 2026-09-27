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
        _ => Err(cb::not_impl(scope, "cakeStall")),
    }
}
