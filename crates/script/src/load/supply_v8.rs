//! Typed local V8 marshalling for supply helpers (v1 food + five v2 methods).
//! Not a rustyscript `register_function` JSON op.

use crate::escape_runes::{self, EscapeRunesError, EscapeRunesFact, RuneCost};
use crate::food_policy::{self, FoodHealOutcome, InvItemName};
use crate::keep_list::{self, CombatKeepOptions};
use crate::supply_v2;
use rustyscript::Runtime;

const PENDING: &str = "__pending__";

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
    let v2_name = v8::String::new(&mut scope, "__rs2b0t_supply_v2")
        .ok_or_else(|| "supply name v2".to_string())?;
    let v2_fn = v8::Function::new(&mut scope, supply_v2_callback)
        .ok_or_else(|| "supply fn v2".to_string())?;
    global
        .set(&mut scope, v2_name.into(), v2_fn.into())
        .ok_or_else(|| "supply set v2".to_string())?;
    Ok(())
}

fn food_count_v1_callback(
    scope: &mut v8::HandleScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    match run_food_count_v1(scope, &args) {
        Ok(count) => {
            rv.set(v8::Integer::new(scope, i32::try_from(count).unwrap_or(i32::MAX)).into())
        }
        Err(err) if err == PENDING => {}
        Err(err) => throw_error(scope, &err),
    }
}

fn food_heal_v1_callback(
    scope: &mut v8::HandleScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let food_name = match js_to_string(scope, args.get(0)) {
        Ok(name) => name,
        Err(err) if err == PENDING => return,
        Err(err) => {
            throw_error(scope, &err);
            return;
        }
    };
    let obj = v8::Object::new(scope);
    match food_policy::food_heal_amount(supply_v2::selected_data().as_deref(), &food_name) {
        FoodHealOutcome::Ok(heal) => {
            let ok = v8::Boolean::new(scope, true);
            set_key(scope, obj, "ok", ok.into());
            let value = v8::Integer::new(scope, heal);
            set_key(scope, obj, "value", value.into());
        }
        FoodHealOutcome::MissingSelected => {
            let ok = v8::Boolean::new(scope, false);
            set_key(scope, obj, "ok", ok.into());
            let reason = v8_str(scope, supply_v2::GAME_DATA_UNAVAILABLE)
                .unwrap_or_else(|_| v8::undefined(scope).into());
            set_key(scope, obj, "reason", reason);
        }
        FoodHealOutcome::UnknownFood => {
            let ok = v8::Boolean::new(scope, false);
            set_key(scope, obj, "ok", ok.into());
        }
    }
    rv.set(obj.into());
}

fn supply_v2_callback(
    scope: &mut v8::HandleScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    match run_supply_v2(scope, &args) {
        Ok(value) => rv.set(value),
        Err(err) if err == PENDING => {}
        Err(err) => match helper_err(scope, &err) {
            Ok(value) => rv.set(value),
            Err(_) => rv.set(v8::null(scope).into()),
        },
    }
}

fn run_food_count_v1(
    scope: &mut v8::HandleScope,
    args: &v8::FunctionCallbackArguments,
) -> Result<usize, String> {
    let items_val = args.get(0);
    if !js_array_is_array(scope, items_val) {
        return Ok(0);
    }
    let items = parse_v1_items(scope, items_val)?;
    if items.is_empty() {
        return Ok(0);
    }
    let food_name = js_to_string(scope, args.get(1))?;
    Ok(food_policy::food_count(
        supply_v2::selected_data().as_deref(),
        &items,
        &food_name,
    ))
}

fn run_supply_v2<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: &v8::FunctionCallbackArguments,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let op = js_to_string(scope, args.get(0))?;
    let input = args.get(1);
    match op.as_str() {
        "foodCount" => v2_food_count(scope, input),
        "foodHealAmount" => v2_food_heal(scope, input),
        "combatKeepNames" => v2_combat_keep_names(scope, input),
        "runesPerCast" => v2_runes_per_cast(scope, input),
        "escapeRunesFor" => v2_escape_runes(scope, input),
        _ => Err("invalid-args".into()),
    }
}

fn v2_food_count<'s>(
    scope: &mut v8::HandleScope<'s>,
    input: v8::Local<v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let items_val = field(scope, input, "items")?;
    if !js_array_is_array(scope, items_val) {
        return Err("invalid-args".into());
    }
    if !field_is_string(scope, input, "foodName") {
        return Err("invalid-args".into());
    }
    let food_name = field_string(scope, input, "foodName")?;
    let items = parse_v2_items(scope, items_val)?;
    let Some(data) = supply_v2::selected_data() else {
        return Err("missing-selected-data".into());
    };
    let count = food_policy::food_count(Some(data.as_ref()), &items, &food_name);
    let n = v8::Integer::new(scope, i32::try_from(count).unwrap_or(i32::MAX));
    helper_ok(scope, n.into())
}

fn v2_food_heal<'s>(
    scope: &mut v8::HandleScope<'s>,
    input: v8::Local<v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    if !field_is_string(scope, input, "foodName") {
        return Err("invalid-args".into());
    }
    let food_name = field_string(scope, input, "foodName")?;
    match food_policy::food_heal_amount(supply_v2::selected_data().as_deref(), &food_name) {
        FoodHealOutcome::Ok(heal) => {
            let n = v8::Integer::new(scope, heal);
            helper_ok(scope, n.into())
        }
        FoodHealOutcome::MissingSelected => Err("missing-selected-data".into()),
        FoodHealOutcome::UnknownFood => Err("unknown-food".into()),
    }
}

fn v2_combat_keep_names<'s>(
    scope: &mut v8::HandleScope<'s>,
    input: v8::Local<v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let Some(data) = supply_v2::selected_data() else {
        return Err("missing-selected-data".into());
    };
    if !field_is_string(scope, input, "food") {
        return Err("invalid-args".into());
    }
    let options = CombatKeepOptions {
        food: field_string(scope, input, "food")?,
        style: optional_string(scope, input, "style")?,
        spell: optional_string(scope, input, "spell")?,
        ammo: optional_string(scope, input, "ammo")?,
        weapon: optional_string(scope, input, "weapon")?,
        extra: optional_string_array(scope, input, "extra")?,
    };
    let names = keep_list::combat_keep_names(data.as_ref(), &options);
    let arr = v8::Array::new(scope, names.len() as i32);
    for (i, name) in names.iter().enumerate() {
        let s = v8_str(scope, name)?;
        arr.set_index(scope, i as u32, s)
            .ok_or_else(|| "array set".to_string())?;
    }
    helper_ok(scope, arr.into())
}

fn v2_runes_per_cast<'s>(
    scope: &mut v8::HandleScope<'s>,
    input: v8::Local<v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let Some(data) = supply_v2::selected_data() else {
        return Err("missing-selected-data".into());
    };
    if !field_is_string(scope, input, "spellName") {
        return Err("invalid-args".into());
    }
    let spell_name = field_string(scope, input, "spellName")?;
    let wielded_val = field(scope, input, "wielded")?;
    if !js_array_is_array(scope, wielded_val) {
        return Err("invalid-args".into());
    }
    let wielded = required_string_array(scope, wielded_val)?;
    match data.as_ref().runes_per_cast(&spell_name, &wielded) {
        None => {
            let null = v8::null(scope);
            helper_ok(scope, null.into())
        }
        Some(costs) => {
            let mapped: Vec<RuneCost> = costs
                .into_iter()
                .map(|row| RuneCost {
                    rune: row.rune,
                    count: row.count,
                })
                .collect();
            let value = materialize_rune_costs(scope, &mapped)?;
            helper_ok(scope, value)
        }
    }
}

fn v2_escape_runes<'s>(
    scope: &mut v8::HandleScope<'s>,
    input: v8::Local<v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    if !field_is_string(scope, input, "id") {
        return Err("invalid-args".into());
    }
    let id = field_string(scope, input, "id")?;
    match escape_runes::escape_runes_for_optional(supply_v2::selected_data().as_deref(), &id) {
        Ok(fact) => {
            let value = materialize_escape(scope, &fact)?;
            helper_ok(scope, value)
        }
        Err(EscapeRunesError::MissingSelected) => Err("missing-selected-data".into()),
        Err(EscapeRunesError::UnknownId) => Err("unknown-id".into()),
    }
}

fn parse_v1_items(
    scope: &mut v8::HandleScope,
    value: v8::Local<v8::Value>,
) -> Result<Vec<InvItemName>, String> {
    let Some(obj) = value.to_object(scope) else {
        return Ok(Vec::new());
    };
    let len = array_len(scope, obj)?;
    let mut out = Vec::new();
    for i in 0..len {
        let item = index_at(scope, obj, i)?;
        if js_is_falsy(scope, item) {
            continue;
        }
        out.push(InvItemName {
            name: item_name(scope, item)?,
        });
    }
    Ok(out)
}

fn parse_v2_items(
    scope: &mut v8::HandleScope,
    value: v8::Local<v8::Value>,
) -> Result<Vec<InvItemName>, String> {
    let Some(obj) = value.to_object(scope) else {
        return Err("invalid-args".into());
    };
    let len = array_len(scope, obj)?;
    let mut out = Vec::with_capacity(len as usize);
    for i in 0..len {
        let item = index_at(scope, obj, i)?;
        if item.is_null_or_undefined() {
            return Err("invalid-args".into());
        }
        if !item.is_object() {
            return Err("invalid-args".into());
        }
        out.push(InvItemName {
            name: item_name(scope, item)?,
        });
    }
    Ok(out)
}

fn item_name(
    scope: &mut v8::HandleScope,
    item: v8::Local<v8::Value>,
) -> Result<Option<String>, String> {
    let Some(obj) = item.to_object(scope) else {
        return Ok(None);
    };
    let key = v8::String::new(scope, "name").ok_or_else(|| "string".to_string())?;
    let Some(name) = obj.get(scope, key.into()) else {
        return Err(PENDING.into());
    };
    if name.is_null_or_undefined() {
        return Ok(None);
    }
    Ok(Some(js_to_string(scope, name)?))
}

fn js_is_falsy(scope: &mut v8::HandleScope, value: v8::Local<v8::Value>) -> bool {
    if value.is_null_or_undefined() || value.is_false() {
        return true;
    }
    if value.is_number() {
        let n = value.number_value(scope).unwrap_or(f64::NAN);
        return n == 0.0 || n.is_nan();
    }
    if value.is_string() {
        return value.to_rust_string_lossy(scope).is_empty();
    }
    false
}

fn js_array_is_array(scope: &mut v8::HandleScope, value: v8::Local<v8::Value>) -> bool {
    if value.is_array() {
        return true;
    }
    let context = scope.get_current_context();
    let global = context.global(scope);
    let Some(array_key) = v8::String::new(scope, "Array") else {
        return false;
    };
    let Some(array_val) = global.get(scope, array_key.into()) else {
        return false;
    };
    let Some(array_obj) = array_val.to_object(scope) else {
        return false;
    };
    let Some(is_array_key) = v8::String::new(scope, "isArray") else {
        return false;
    };
    let Some(fn_val) = array_obj.get(scope, is_array_key.into()) else {
        return false;
    };
    let Ok(func) = v8::Local::<v8::Function>::try_from(fn_val) else {
        return false;
    };
    let Some(result) = func.call(scope, array_val, &[value]) else {
        return false;
    };
    result.boolean_value(scope)
}

fn array_len(scope: &mut v8::HandleScope, obj: v8::Local<v8::Object>) -> Result<u32, String> {
    let key = v8::String::new(scope, "length").ok_or_else(|| "string".to_string())?;
    let Some(len) = obj.get(scope, key.into()) else {
        return Err(PENDING.into());
    };
    let n = len
        .number_value(scope)
        .ok_or_else(|| "invalid-args".to_string())?;
    if !n.is_finite() || n < 0.0 {
        return Err("invalid-args".into());
    }
    Ok(n as u32)
}

fn index_at<'s>(
    scope: &mut v8::HandleScope<'s>,
    obj: v8::Local<'s, v8::Object>,
    i: u32,
) -> Result<v8::Local<'s, v8::Value>, String> {
    obj.get_index(scope, i).ok_or_else(|| PENDING.to_string())
}

fn field<'s>(
    scope: &mut v8::HandleScope<'s>,
    value: v8::Local<v8::Value>,
    name: &str,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let obj = value
        .to_object(scope)
        .ok_or_else(|| "invalid-args".to_string())?;
    let key = v8::String::new(scope, name).ok_or_else(|| "string".to_string())?;
    obj.get(scope, key.into())
        .ok_or_else(|| PENDING.to_string())
}

fn field_is_string(scope: &mut v8::HandleScope, value: v8::Local<v8::Value>, name: &str) -> bool {
    field(scope, value, name).is_ok_and(|v| v.is_string())
}

fn field_string(
    scope: &mut v8::HandleScope,
    value: v8::Local<v8::Value>,
    name: &str,
) -> Result<String, String> {
    let v = field(scope, value, name)?;
    js_to_string(scope, v)
}

fn optional_string(
    scope: &mut v8::HandleScope,
    value: v8::Local<v8::Value>,
    name: &str,
) -> Result<String, String> {
    let v = field(scope, value, name)?;
    if v.is_undefined() || v.is_null() {
        return Ok(String::new());
    }
    if !v.is_string() {
        return Err("invalid-args".into());
    }
    js_to_string(scope, v)
}

fn optional_string_array(
    scope: &mut v8::HandleScope,
    value: v8::Local<v8::Value>,
    name: &str,
) -> Result<Vec<String>, String> {
    let v = field(scope, value, name)?;
    if v.is_undefined() || v.is_null() {
        return Ok(Vec::new());
    }
    if !js_array_is_array(scope, v) {
        return Err("invalid-args".into());
    }
    required_string_array(scope, v)
}

fn required_string_array(
    scope: &mut v8::HandleScope,
    value: v8::Local<v8::Value>,
) -> Result<Vec<String>, String> {
    let obj = value
        .to_object(scope)
        .ok_or_else(|| "invalid-args".to_string())?;
    let len = array_len(scope, obj)?;
    let mut out = Vec::with_capacity(len as usize);
    for i in 0..len {
        let item = index_at(scope, obj, i)?;
        if !item.is_string() {
            return Err("invalid-args".into());
        }
        out.push(js_to_string(scope, item)?);
    }
    Ok(out)
}

fn js_to_string(
    scope: &mut v8::HandleScope,
    value: v8::Local<v8::Value>,
) -> Result<String, String> {
    if value.is_null() {
        return Ok("null".into());
    }
    if value.is_undefined() {
        return Ok("undefined".into());
    }
    match value.to_string(scope) {
        Some(s) => Ok(s.to_rust_string_lossy(scope)),
        None => Err(PENDING.into()),
    }
}

fn helper_ok<'s>(
    scope: &mut v8::HandleScope<'s>,
    value: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let obj = v8::Object::new(scope);
    let ok = v8::Boolean::new(scope, true);
    set_key(scope, obj, "ok", ok.into());
    set_key(scope, obj, "value", value);
    Ok(obj.into())
}

fn helper_err<'s>(
    scope: &mut v8::HandleScope<'s>,
    error: &str,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let obj = v8::Object::new(scope);
    let ok = v8::Boolean::new(scope, false);
    set_key(scope, obj, "ok", ok.into());
    let error = if error == "missing-selected-data" {
        supply_v2::GAME_DATA_UNAVAILABLE
    } else {
        error
    };
    let err = v8_str(scope, error)?;
    set_key(scope, obj, "error", err);
    Ok(obj.into())
}

fn materialize_escape<'s>(
    scope: &mut v8::HandleScope<'s>,
    fact: &EscapeRunesFact,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let obj = v8::Object::new(scope);
    let runes = materialize_rune_costs(scope, &fact.runes)?;
    set_key(scope, obj, "runes", runes);
    let level = v8::Integer::new(scope, fact.level);
    set_key(scope, obj, "level", level.into());
    let label = v8_str(scope, &fact.label)?;
    set_key(scope, obj, "label", label);
    Ok(obj.into())
}

fn materialize_rune_costs<'s>(
    scope: &mut v8::HandleScope<'s>,
    costs: &[RuneCost],
) -> Result<v8::Local<'s, v8::Value>, String> {
    let arr = v8::Array::new(scope, costs.len() as i32);
    for (i, cost) in costs.iter().enumerate() {
        let row = v8::Object::new(scope);
        let rune = v8_str(scope, &cost.rune)?;
        set_key(scope, row, "rune", rune);
        let count = v8::Integer::new(scope, cost.count);
        set_key(scope, row, "count", count.into());
        arr.set_index(scope, i as u32, row.into())
            .ok_or_else(|| "array".to_string())?;
    }
    Ok(arr.into())
}

fn v8_str<'s>(
    scope: &mut v8::HandleScope<'s>,
    s: &str,
) -> Result<v8::Local<'s, v8::Value>, String> {
    v8::String::new(scope, s)
        .map(|v| v.into())
        .ok_or_else(|| "string".to_string())
}

fn set_key(
    scope: &mut v8::HandleScope,
    obj: v8::Local<v8::Object>,
    key: &str,
    value: v8::Local<v8::Value>,
) {
    if let Some(k) = v8::String::new(scope, key) {
        let _ = obj.set(scope, k.into(), value);
    }
}

fn throw_error(scope: &mut v8::HandleScope, message: &str) {
    if let Some(s) = v8::String::new(scope, message) {
        let exc = v8::Exception::error(scope, s);
        scope.throw_exception(exc);
    }
}
