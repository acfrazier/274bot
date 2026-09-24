//! V8 marshalling for the step-machine host (`crate::machine`).
//!
//! - `__rs2b0t_machine_start(family, args, hooks)` holds the family's
//!   declared callbacks off `hooks`, begins the machine and returns its
//!   start envelope.
//! - `__rs2b0t_machine_take(handle)` is the parked wait's poll
//!   (`undefined` while the machine runs).
//! - [`step`] is the isolate thread's per-tick step, with script callbacks
//!   invoked through the one callback path (`callback_v8`) and returned
//!   promises held and their state polled once per tick.
//!
//! Nothing here sends to the host: machine ops join the tick's
//! InteractReq batch in Rust.

use super::callback_v8::{self, Callback, HeldCallback, Throw};
use crate::machine::{self, Called, Outcome, Pending, Reply, Started, Take, Thrown};
use rustyscript::deno_core::error::JsError;
use rustyscript::deno_core::serde_v8;
use rustyscript::Runtime;
use serde_json::Value;

pub(super) fn install(runtime: &mut Runtime) -> Result<(), String> {
    callback_v8::install(runtime, "__rs2b0t_machine_start", start_callback)?;
    callback_v8::install(runtime, "__rs2b0t_machine_take", take_callback)
}

/// Step every live machine once, before this tick's JS.
pub(super) fn step(runtime: &mut Runtime) {
    machine::step(&mut RuntimeJs(runtime));
}

fn start_callback<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let family = args.get(0).to_rust_string_lossy(scope);
    let hooks = match hold_hooks(scope, &family, args.get(2)) {
        Ok(hooks) => hooks,
        Err(throw) => return callback_v8::finish(scope, rv, Err(throw)),
    };
    let at = js_queue_len(scope);
    let started = match serde_v8::from_v8::<Value>(scope, args.get(1)) {
        Ok(value) => machine::start(&family, value, hooks, at),
        Err(e) => Started::Refused(format!("{family} arguments: {e}")),
    };
    let envelope = match started {
        Started::Running(handle) => {
            let handle = callback_v8::num(scope, handle as f64);
            envelope(scope, "running", "handle", handle)
        }
        Started::Settled(outcome) => outcome_envelope(scope, outcome),
        Started::Refused(reason) => {
            let reason = callback_v8::string(scope, &reason);
            envelope(scope, "refused", "reason", reason)
        }
    };
    callback_v8::finish(scope, rv, envelope);
}

/// `hooks[name]` for each declared callback, called as `hooks.name(...)`.
/// A family without callbacks never reads `hooks`.
fn hold_hooks<'s>(
    scope: &mut v8::HandleScope<'s>,
    family: &str,
    hooks: v8::Local<'s, v8::Value>,
) -> Result<Vec<HeldCallback>, Throw<'s>> {
    let names = machine::callbacks_of(family).unwrap_or_default();
    let mut held = Vec::with_capacity(names.len());
    for &name in names {
        let func = callback_v8::get(scope, hooks, name)?;
        held.push(Callback::method(func, hooks, name).hold(scope));
    }
    Ok(held)
}

fn take_callback<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let handle = args.get(0).number_value(scope).unwrap_or(0.0) as machine::Handle;
    if let Take::Settled(outcome) = machine::take(handle) {
        let envelope = outcome_envelope(scope, outcome);
        callback_v8::finish(scope, rv, envelope);
    }
}

fn outcome_envelope<'s>(
    scope: &mut v8::HandleScope<'s>,
    outcome: Outcome,
) -> callback_v8::JsResult<'s, v8::Local<'s, v8::Value>> {
    match outcome {
        Outcome::Done(value) => {
            let value = serde_v8::to_v8(scope, &value)
                .map_err(|e| callback_v8::type_error(scope, &format!("machine value: {e}")))?;
            envelope(scope, "done", "value", value)
        }
        Outcome::Failed(thrown) => {
            let error = match thrown.value() {
                Some(value) => v8::Local::new(scope, value),
                None => {
                    let message = v8::String::new(scope, thrown.message())
                        .unwrap_or_else(|| v8::String::empty(scope));
                    v8::Exception::error(scope, message)
                }
            };
            envelope(scope, "failed", "error", error)
        }
        Outcome::Aborted(why) => {
            let reason = callback_v8::string(scope, why.as_str());
            envelope(scope, "aborted", "reason", reason)
        }
    }
}

/// `{ kind, [field]: value }`.
fn envelope<'s>(
    scope: &mut v8::HandleScope<'s>,
    kind: &str,
    field: &str,
    value: v8::Local<'s, v8::Value>,
) -> callback_v8::JsResult<'s, v8::Local<'s, v8::Value>> {
    let kind = callback_v8::string(scope, kind);
    callback_v8::object(scope, &[("kind", kind), (field, value)])
}

/// Length of `__rs2b0t_host.interact`: where machine ops sit in the tick
/// batch relative to the rows JS queued.
fn js_queue_len(scope: &mut v8::HandleScope) -> usize {
    let context = scope.get_current_context();
    let global = context.global(scope);
    let Some(host_key) = v8::String::new(scope, "__rs2b0t_host") else {
        return 0;
    };
    let Some(host) = global
        .get(scope, host_key.into())
        .and_then(|host| host.to_object(scope))
    else {
        return 0;
    };
    let Some(interact_key) = v8::String::new(scope, "interact") else {
        return 0;
    };
    host.get(scope, interact_key.into())
        .and_then(|rows| v8::Local::<v8::Array>::try_from(rows).ok())
        .map_or(0, |rows| rows.length() as usize)
}

/// The isolate runtime as the machine host's script side.
struct RuntimeJs<'r>(&'r mut Runtime);

impl machine::Js for RuntimeJs<'_> {
    fn queue_len(&mut self) -> usize {
        let mut scope = self.0.deno_runtime().handle_scope();
        js_queue_len(&mut scope)
    }

    fn call(&mut self, hook: Option<&HeldCallback>, args: &[Value]) -> Called {
        let Some(hook) = hook else {
            return Called::Settled(Reply::Threw(Thrown::new("undeclared machine callback")));
        };
        let scope = &mut self.0.deno_runtime().handle_scope();
        let callback = hook.open(scope);
        let mut argv = Vec::with_capacity(args.len());
        for arg in args {
            match serde_v8::to_v8(scope, arg) {
                Ok(value) => argv.push(value),
                Err(e) => {
                    let failure = Thrown::new(format!("callback argument: {e}"));
                    return Called::Settled(Reply::Threw(failure));
                }
            }
        }
        let value = match callback.call(scope, &argv) {
            Ok(value) => value,
            Err(Throw::Value(exception)) => {
                return Called::Settled(Reply::Threw(thrown(scope, exception)));
            }
            Err(Throw::Terminated) => {
                return Called::Settled(Reply::Threw(Thrown::new("execution terminated")));
            }
        };
        if let Ok(promise) = v8::Local::<v8::Promise>::try_from(value) {
            // The host observes the rejection; it is not an unhandled one.
            if let Some(noop) = v8::Function::new(
                scope,
                |_: &mut v8::HandleScope, _: v8::FunctionCallbackArguments, _: v8::ReturnValue| {},
            ) {
                promise.catch(scope, noop);
            }
            return Called::Pending(v8::Global::new(scope, promise));
        }
        Called::Settled(settled_value(scope, value))
    }

    fn poll(&mut self, pending: &Pending) -> Option<Reply> {
        let scope = &mut self.0.deno_runtime().handle_scope();
        let promise = v8::Local::new(scope, pending);
        match promise.state() {
            v8::PromiseState::Pending => None,
            v8::PromiseState::Fulfilled => {
                let value = promise.result(scope);
                Some(settled_value(scope, value))
            }
            v8::PromiseState::Rejected => {
                let reason = promise.result(scope);
                Some(Reply::Threw(thrown(scope, reason)))
            }
        }
    }
}

fn settled_value(scope: &mut v8::HandleScope, value: v8::Local<v8::Value>) -> Reply {
    match serde_v8::from_v8::<Value>(scope, value) {
        Ok(value) => Reply::Value(value),
        Err(e) => Reply::Threw(Thrown::new(format!("callback result: {e}"))),
    }
}

/// The thrown value itself, with its message (`new Error('x')` → `x`,
/// else V8's text).
fn thrown(scope: &mut v8::HandleScope, value: v8::Local<v8::Value>) -> Thrown {
    let error = JsError::from_v8_exception(scope, value);
    let message = error.message.unwrap_or(error.exception_message);
    Thrown::js(message, v8::Global::new(scope, value))
}
