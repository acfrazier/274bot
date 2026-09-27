//! Typed V8 binding for RunManager's per-session host policy overlay.
//! Coercion is synchronous; the resulting replacement joins the isolate's
//! next FlatBuffer interact batch.

use super::callback_v8::{self, JsResult};
use api::run_policy::{RunEnergyMin, RunPolicyOverride};
use rustyscript::Runtime;
use std::cell::Cell;

#[derive(Clone, Copy)]
enum PendingOverride {
    Empty,
    Replace(Option<RunPolicyOverride>),
}

thread_local! {
    static PENDING_OVERRIDE: Cell<PendingOverride> =
        const { Cell::new(PendingOverride::Empty) };
}

/// Install the direct `__rs2b0t_run_override` callback. Its pending value is
/// isolate-thread local and never shared with the host.
pub(super) fn install(runtime: &mut Runtime) -> Result<(), String> {
    clear_pending();
    callback_v8::install(runtime, "__rs2b0t_run_override", run_override_callback)
}

fn run_override_callback<'s, 'cb>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'cb>,
) {
    match run_policy_override(scope, args.get(0)) {
        Ok(policy) => {
            PENDING_OVERRIDE.with(|pending| pending.set(PendingOverride::Replace(policy)));
            rv.set(v8::undefined(scope).into());
        }
        Err(error) => callback_v8::finish(scope, rv, Err(error)),
    }
}

pub(super) fn take_pending() -> Option<crate::shim::InteractReq> {
    PENDING_OVERRIDE.with(|pending| match pending.replace(PendingOverride::Empty) {
        PendingOverride::Empty => None,
        PendingOverride::Replace(policy) => {
            Some(crate::shim::InteractReq::RunPolicyOverride { policy })
        }
    })
}

pub(super) fn clear_pending() {
    PENDING_OVERRIDE.with(|pending| pending.set(PendingOverride::Empty));
}

fn run_policy_override<'s>(
    scope: &mut v8::HandleScope<'s>,
    value: v8::Local<'s, v8::Value>,
) -> JsResult<'s, Option<RunPolicyOverride>> {
    if value.is_null() || value.is_undefined() {
        return Ok(None);
    }
    let object = callback_v8::catching(scope, |scope| value.to_object(scope))?;

    // Frozen RunManager.ts:26-38 reads energyMin first and computes
    // Math.max(0, Math.min(100, energyMin)); preserving NaN is required because
    // the frozen energy >= NaN comparison is always false.
    let energy_value = property(scope, object, "energyMin")?;
    let energy_min = if energy_value.is_null() || energy_value.is_undefined() {
        None
    } else {
        let number = callback_v8::number(scope, energy_value)?;
        Some(clamp_energy_min(number))
    };

    // Frozen RunManager.ts:81-98 checks `if (!policy.runAuto)`, so non-boolean
    // values use JavaScript truthiness rather than falling through to defaults.
    let run_auto_value = property(scope, object, "runAuto")?;
    let run_auto = if run_auto_value.is_null() || run_auto_value.is_undefined() {
        None
    } else {
        Some(callback_v8::truthy(scope, run_auto_value))
    };
    Ok(
        (run_auto.is_some() || energy_min.is_some()).then_some(RunPolicyOverride {
            run_auto,
            energy_min,
        }),
    )
}

fn clamp_energy_min(number: f64) -> RunEnergyMin {
    if number.is_nan() {
        return RunEnergyMin::NotANumber;
    }
    if number <= 0.0 {
        RunEnergyMin::Floor(0)
    } else if number >= 100.0 {
        RunEnergyMin::Floor(100)
    } else {
        // Client energy is integral, so ceil preserves the frozen `>= floor`
        // comparison for fractional values.
        RunEnergyMin::Floor(number.ceil() as i32)
    }
}

fn property<'s>(
    scope: &mut v8::HandleScope<'s>,
    object: v8::Local<'s, v8::Object>,
    name: &str,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let key = v8::String::new(scope, name)
        .ok_or_else(|| callback_v8::type_error(scope, "run-policy property key"))?;
    callback_v8::catching(scope, |scope| object.get(scope, key.into()))
}
