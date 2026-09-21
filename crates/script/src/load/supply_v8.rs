//! Typed local V8 marshalling for v1 food shim helpers.

use crate::food_policy::{self, FoodHealOutcome, InvItemName};
use crate::supply_v2;
use rustyscript::Runtime;

pub(super) fn install(runtime: &mut Runtime) -> Result<(), String> {
    let context = runtime.deno_runtime().main_context();
    let mut scope = runtime.deno_runtime().handle_scope();
    let global = context.open(&mut scope).global(&mut scope);
    let count_name = v8::String::new(&mut scope, "__rs2b0t_food_count")
        .ok_or_else(|| "supply name food_count".to_string())?;
    let count_fn = v8::Function::new(&mut scope, food_count_v1_callback)
        .ok_or_else(|| "supply fn food_count".to_string())?;
    global
        .set(&mut scope, count_name.into(), count_fn.into())
        .ok_or_else(|| "supply set food_count".to_string())?;
    let heal_name = v8::String::new(&mut scope, "__rs2b0t_food_heal_amount")
        .ok_or_else(|| "supply name food_heal".to_string())?;
    let heal_fn = v8::Function::new(&mut scope, food_heal_v1_callback)
        .ok_or_else(|| "supply fn food_heal".to_string())?;
    global
        .set(&mut scope, heal_name.into(), heal_fn.into())
        .ok_or_else(|| "supply set food_heal".to_string())?;
    Ok(())
}

fn food_count_v1_callback(
    scope: &mut v8::HandleScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let count = run_food_count(scope, &args);
    rv.set(
        v8::Integer::new(scope, i32::try_from(count).unwrap_or(i32::MAX)).into(),
    );
}

fn food_heal_v1_callback(
    scope: &mut v8::HandleScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let obj = v8::Object::new(scope);
    match run_food_heal(scope, &args) {
        Some(heal) => {
            let ok = v8::Boolean::new(scope, true);
            let key_ok = v8::String::new(scope, "ok").expect("key");
            let _ = obj.set(scope, key_ok.into(), ok.into());
            let value = v8::Integer::new(scope, heal);
            let key_value = v8::String::new(scope, "value").expect("key");
            let _ = obj.set(scope, key_value.into(), value.into());
        }
        None => {
            let ok = v8::Boolean::new(scope, false);
            let key_ok = v8::String::new(scope, "ok").expect("key");
            let _ = obj.set(scope, key_ok.into(), ok.into());
        }
    }
    rv.set(obj.into());
}

fn run_food_count(scope: &mut v8::HandleScope, args: &v8::FunctionCallbackArguments) -> usize {
    let mut items = Vec::new();
    let value = args.get(0);
    if let Some(obj) = value.to_object(scope) {
        if let Ok(arr) = v8::Local::<v8::Array>::try_from(obj) {
            let len = arr.length();
            for i in 0..len {
                let Some(item) = arr.get_index(scope, i) else {
                    continue;
                };
                if item.is_null_or_undefined() || !item.is_object() {
                    continue;
                }
                let name = item
                    .to_object(scope)
                    .and_then(|obj| {
                        let key = v8::String::new(scope, "name")?;
                        obj.get(scope, key.into())
                    })
                    .map(|v| {
                        if v.is_null_or_undefined() {
                            None
                        } else {
                            Some(js_string_arg(scope, v))
                        }
                    })
                    .unwrap_or(None);
                items.push(InvItemName { name });
            }
        }
    }
    let food_name = js_string_arg(scope, args.get(1));
    food_policy::food_count(
        supply_v2::selected_data().as_deref(),
        &items,
        &food_name,
    )
}

fn run_food_heal(scope: &mut v8::HandleScope, args: &v8::FunctionCallbackArguments) -> Option<i32> {
    let food_name = js_string_arg(scope, args.get(0));
    match food_policy::food_heal_amount(supply_v2::selected_data().as_deref(), &food_name) {
        FoodHealOutcome::Ok(heal) => Some(heal),
        _ => None,
    }
}

fn js_string_arg(scope: &mut v8::HandleScope, value: v8::Local<v8::Value>) -> String {
    if value.is_null_or_undefined() {
        return String::new();
    }
    value
        .to_string(scope)
        .map(|s| s.to_rust_string_lossy(scope))
        .unwrap_or_default()
}
