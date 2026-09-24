//! Typed V8 binding for RunManager's per-session host policy overlay.
//! This is a local V8 callback, not a rustyscript JSON op.

use super::callback_v8::{self, JsResult, Throw};
use api::run_policy::{RunEnergyMin, RunPolicyOverride, RunPolicyOverrideCell};
use rustyscript::Runtime;
use std::ffi::c_void;
use std::sync::Arc;

const RUN_OVERRIDE: &str = "__rs2b0t_run_override";
const ORIGINAL_GET: &str = "originalGet";
const RUN_OVERRIDE_FUNCTION: &str = "runOverrideFunction";

struct RunPolicyCellSlot {
    _cell: Arc<RunPolicyOverrideCell>,
}

pub(super) fn install(
    runtime: &mut Runtime,
    run_policy_override: Arc<RunPolicyOverrideCell>,
) -> Result<(), String> {
    let isolate = runtime.deno_runtime().v8_isolate();
    if isolate.get_slot::<RunPolicyCellSlot>().is_some() {
        return Err("run-policy cell is already installed".into());
    }
    isolate.set_slot(RunPolicyCellSlot {
        _cell: Arc::clone(&run_policy_override),
    });

    let context = runtime.deno_runtime().main_context();
    let mut scope = runtime.deno_runtime().handle_scope();
    let global = context.open(&mut scope).global(&mut scope);
    let rustyscript_name = v8::String::new(&mut scope, "rustyscript")
        .ok_or_else(|| "run-policy rustyscript name".to_string())?;
    let rustyscript_value = global
        .get(&mut scope, rustyscript_name.into())
        .ok_or_else(|| "run-policy rustyscript object".to_string())?;
    let rustyscript_object = v8::Local::<v8::Object>::try_from(rustyscript_value)
        .map_err(|_| "run-policy rustyscript object".to_string())?;
    let functions_name = v8::String::new(&mut scope, "functions")
        .ok_or_else(|| "run-policy functions name".to_string())?;
    let functions_value = rustyscript_object
        .get(&mut scope, functions_name.into())
        .ok_or_else(|| "run-policy functions proxy".to_string())?;
    let functions_proxy = v8::Local::<v8::Proxy>::try_from(functions_value)
        .map_err(|_| "run-policy functions proxy".to_string())?;
    let handler_value = functions_proxy.get_handler(&mut scope);
    let handler = v8::Local::<v8::Object>::try_from(handler_value)
        .map_err(|_| "run-policy functions handler".to_string())?;
    let get_name =
        v8::String::new(&mut scope, "get").ok_or_else(|| "run-policy get name".to_string())?;
    let original_get_value = handler
        .get(&mut scope, get_name.into())
        .ok_or_else(|| "run-policy original functions getter".to_string())?;
    let original_get = v8::Local::<v8::Function>::try_from(original_get_value)
        .map_err(|_| "run-policy original functions getter".to_string())?;

    let cell_data = v8::External::new(&mut scope, Arc::as_ptr(&run_policy_override) as *mut c_void);
    let run_override_function = v8::Function::builder(run_override_callback)
        .data(cell_data.into())
        .build(&mut scope)
        .ok_or_else(|| "run-policy override function".to_string())?;
    let callback_data = v8::Object::new(&mut scope);
    let original_get_name = v8::String::new(&mut scope, ORIGINAL_GET)
        .ok_or_else(|| "run-policy callback original name".to_string())?;
    if callback_data.set(&mut scope, original_get_name.into(), original_get.into()) != Some(true) {
        return Err("run-policy callback original getter".into());
    }
    let run_override_name = v8::String::new(&mut scope, RUN_OVERRIDE_FUNCTION)
        .ok_or_else(|| "run-policy callback override name".to_string())?;
    if callback_data.set(
        &mut scope,
        run_override_name.into(),
        run_override_function.into(),
    ) != Some(true)
    {
        return Err("run-policy callback override function".into());
    }
    let functions_get = v8::Function::builder(functions_get_callback)
        .data(callback_data.into())
        .build(&mut scope)
        .ok_or_else(|| "run-policy functions getter".to_string())?;
    if handler.set(&mut scope, get_name.into(), functions_get.into()) != Some(true) {
        return Err("run-policy functions getter install".into());
    }
    Ok(())
}

fn functions_get_callback<'s, 'cb>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'cb>,
) {
    let data = match v8::Local::<v8::Object>::try_from(args.data()) {
        Ok(data) => data,
        Err(_) => {
            throw_type_error(
                scope,
                rv,
                "run-policy functions getter has invalid callback data",
            );
            return;
        }
    };
    let name = args.get(1);
    if name.is_string() && name.to_rust_string_lossy(scope) == RUN_OVERRIDE {
        let key = match v8::String::new(scope, RUN_OVERRIDE_FUNCTION) {
            Some(key) => key,
            None => {
                throw_type_error(scope, rv, "run-policy override function key");
                return;
            }
        };
        match data.get(scope, key.into()) {
            Some(value) => rv.set(value),
            None => throw_type_error(scope, rv, "run-policy override function missing"),
        }
        return;
    }

    let key = match v8::String::new(scope, ORIGINAL_GET) {
        Some(key) => key,
        None => {
            throw_type_error(scope, rv, "run-policy original getter key");
            return;
        }
    };
    let original_get = match data
        .get(scope, key.into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    {
        Some(function) => function,
        None => {
            throw_type_error(scope, rv, "run-policy original getter missing");
            return;
        }
    };
    if let Some(value) =
        original_get.call(scope, args.this().into(), &[args.get(0), name, args.get(2)])
    {
        rv.set(value);
    }
}

fn run_override_callback<'s, 'cb>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'cb>,
) {
    let external = match v8::Local::<v8::External>::try_from(args.data()) {
        Ok(external) => external,
        Err(_) => {
            throw_type_error(scope, rv, "run-policy override has invalid cell data");
            return;
        }
    };
    let cell = external.value().cast::<RunPolicyOverrideCell>();
    if cell.is_null() {
        throw_type_error(scope, rv, "run-policy override has a null cell");
        return;
    }
    // SAFETY: `install` stores an Arc to this cell in the V8 isolate slot, so
    // the pointer remains valid for the lifetime of this function's context.
    let cell = unsafe { &*cell };
    match run_policy_override(scope, args.get(0)) {
        Ok(policy) => {
            cell.set(policy);
            rv.set(v8::undefined(scope).into());
        }
        Err(error) => callback_v8::finish(scope, rv, Err(error)),
    }
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
    Ok(Some(RunPolicyOverride {
        run_auto,
        energy_min,
    }))
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

fn throw_type_error<'s, 'cb>(
    scope: &mut v8::HandleScope<'s>,
    rv: v8::ReturnValue<'cb>,
    message: &str,
) {
    let error: Throw<'s> = callback_v8::type_error(scope, message);
    callback_v8::finish(scope, rv, Err(error));
}
