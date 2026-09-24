//! V8 marshalling for the step-machine host (`crate::machine`): the two
//! natives behind `_kernel.js` `runMachine`. `__rs2b0t_machine_start(family,
//! args)` begins a machine and returns its start envelope;
//! `__rs2b0t_machine_take(handle)` is the parked wait's poll (`undefined`
//! while the machine runs). Neither sends anything to the host: machine ops
//! join the tick's InteractReq batch in Rust.

use crate::machine::{self, Outcome, Started, Take};
use rustyscript::deno_core::serde_v8;
use rustyscript::Runtime;

pub(super) fn install(runtime: &mut Runtime) -> Result<(), String> {
    let context = runtime.deno_runtime().main_context();
    let mut scope = runtime.deno_runtime().handle_scope();
    let global = context.open(&mut scope).global(&mut scope);
    let start = v8::Function::new(&mut scope, start_callback)
        .ok_or_else(|| "machine start fn".to_string())?;
    let take = v8::Function::new(&mut scope, take_callback)
        .ok_or_else(|| "machine take fn".to_string())?;
    for (name, func) in [
        ("__rs2b0t_machine_start", start),
        ("__rs2b0t_machine_take", take),
    ] {
        let key = v8::String::new(&mut scope, name).ok_or_else(|| format!("{name} name"))?;
        global
            .set(&mut scope, key.into(), func.into())
            .ok_or_else(|| format!("{name} set"))?;
    }
    Ok(())
}

fn start_callback(
    scope: &mut v8::HandleScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let family = args.get(0).to_rust_string_lossy(scope);
    let at = js_queue_len(scope);
    let started = match serde_v8::from_v8::<serde_json::Value>(scope, args.get(1)) {
        Ok(value) => machine::start(&family, value, at),
        Err(e) => Started::Refused(format!("{family} arguments: {e}")),
    };
    let envelope = match started {
        Started::Running(handle) => {
            let handle = v8::Number::new(scope, handle as f64);
            envelope(scope, "running", "handle", handle.into())
        }
        Started::Settled(outcome) => outcome_envelope(scope, outcome),
        Started::Refused(reason) => {
            let reason = string(scope, &reason);
            envelope(scope, "refused", "reason", reason)
        }
    };
    if let Some(envelope) = envelope {
        rv.set(envelope);
    }
}

fn take_callback(
    scope: &mut v8::HandleScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let handle = args.get(0).number_value(scope).unwrap_or(0.0) as machine::Handle;
    if let Take::Settled(outcome) = machine::take(handle) {
        if let Some(envelope) = outcome_envelope(scope, outcome) {
            rv.set(envelope);
        }
    }
}

fn outcome_envelope<'s>(
    scope: &mut v8::HandleScope<'s>,
    outcome: Outcome,
) -> Option<v8::Local<'s, v8::Value>> {
    match outcome {
        Outcome::Done(value) => {
            let value = serde_v8::to_v8(scope, &value).ok()?;
            envelope(scope, "done", "value", value)
        }
        Outcome::Aborted(why) => {
            let reason = string(scope, why.as_str());
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
) -> Option<v8::Local<'s, v8::Value>> {
    let obj = v8::Object::new(scope);
    let kind_key = v8::String::new(scope, "kind")?;
    let kind = string(scope, kind);
    obj.set(scope, kind_key.into(), kind)?;
    let key = v8::String::new(scope, field)?;
    obj.set(scope, key.into(), value)?;
    Some(obj.into())
}

fn string<'s>(scope: &mut v8::HandleScope<'s>, s: &str) -> v8::Local<'s, v8::Value> {
    v8::String::new(scope, s).map_or_else(|| v8::undefined(scope).into(), Into::into)
}

/// Length of `__rs2b0t_host.interact`: where the caller's ops sit in the
/// tick batch when a begin emits.
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
