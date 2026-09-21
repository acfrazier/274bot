//! Typed local V8 marshalling for the eight loadout/potion v2 helpers.
//! Not a rustyscript `register_function` JSON op.

use std::collections::BTreeMap;

use crate::boost_potions::{
    self, PotionLevel, PotionPlan, PotionToSipError, PlannedCarry, BOOST_FLOOR,
};
use crate::loadout_plan::{self, LoadoutCarry, LoadoutInput};
use crate::ranged;
use crate::supply_v2;
use rustyscript::Runtime;

const PENDING: &str = "__pending__";

pub(super) fn install(runtime: &mut Runtime) -> Result<(), String> {
    let context = runtime.deno_runtime().main_context();
    let mut scope = runtime.deno_runtime().handle_scope();
    let global = context.open(&mut scope).global(&mut scope);
    let name = v8::String::new(&mut scope, "__rs2b0t_loadout_v2")
        .ok_or_else(|| "loadout name v2".to_string())?;
    let func = v8::Function::new(&mut scope, loadout_v2_callback)
        .ok_or_else(|| "loadout fn v2".to_string())?;
    global
        .set(&mut scope, name.into(), func.into())
        .ok_or_else(|| "loadout set v2".to_string())?;
    Ok(())
}

fn loadout_v2_callback(
    scope: &mut v8::HandleScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    match run_loadout_v2(scope, &args) {
        Ok(value) => rv.set(value),
        Err(err) if err == PENDING => {}
        Err(err) => match helper_err(scope, &err) {
            Ok(value) => rv.set(value),
            Err(_) => rv.set(v8::null(scope).into()),
        },
    }
}

fn run_loadout_v2<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: &v8::FunctionCallbackArguments,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let op = js_to_string(scope, args.get(0))?;
    let input = args.get(1);
    match op.as_str() {
        "foodOf" => v2_food_of(scope, input),
        "gearOf" => v2_gear_of(scope, input),
        "suppliesOf" => v2_supplies_of(scope, input),
        "weaponOf" => v2_weapon_of(scope, input),
        "rangeLoadoutOf" => v2_range_loadout_of(scope, input),
        "boostFaded" => v2_boost_faded(scope, input),
        "plannedPotions" => v2_planned_potions(scope, input),
        "potionToSip" => v2_potion_to_sip(scope, input),
        _ => Err("invalid-args".into()),
    }
}

fn v2_food_of<'s>(
    scope: &mut v8::HandleScope<'s>,
    input: v8::Local<v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let fallback = required_string_field(scope, input, "fallback")?;
    let loadout = required_loadout(scope, input)?;
    let Some(data) = supply_v2::selected_data() else {
        return Err("missing-selected-data".into());
    };
    let value = loadout_plan::food_of_input(Some(data.as_ref()), loadout.as_ref(), &fallback);
    let s = v8_str(scope, &value)?;
    helper_ok(scope, s)
}

fn v2_gear_of<'s>(
    scope: &mut v8::HandleScope<'s>,
    input: v8::Local<v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let loadout = required_loadout(scope, input)?;
    let names = loadout_plan::gear_of_input(loadout.as_ref());
    let arr = string_array(scope, &names)?;
    helper_ok(scope, arr)
}

fn v2_supplies_of<'s>(
    scope: &mut v8::HandleScope<'s>,
    input: v8::Local<v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let loadout = required_loadout(scope, input)?;
    let rows = loadout_plan::supplies_of_input(loadout.as_ref());
    let arr = v8::Array::new(scope, rows.len() as i32);
    for (i, row) in rows.iter().enumerate() {
        let obj = v8::Object::new(scope);
        let item = v8_str(scope, &row.item)?;
        set_key(scope, obj, "item", item);
        let qty = v8::Number::new(scope, f64::from(row.qty));
        set_key(scope, obj, "qty", qty.into());
        arr.set_index(scope, i as u32, obj.into())
            .ok_or_else(|| "array".to_string())?;
    }
    helper_ok(scope, arr.into())
}

fn v2_weapon_of<'s>(
    scope: &mut v8::HandleScope<'s>,
    input: v8::Local<v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let loadout = required_loadout(scope, input)?;
    let fallback = optional_string_or_null(scope, input, "fallback")?;
    match loadout_plan::weapon_of_input(loadout.as_ref(), fallback.as_deref()) {
        Some(name) => {
            let s = v8_str(scope, &name)?;
            helper_ok(scope, s)
        }
        None => {
            let null = v8::null(scope);
            helper_ok(scope, null.into())
        }
    }
}

fn v2_range_loadout_of<'s>(
    scope: &mut v8::HandleScope<'s>,
    input: v8::Local<v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    if !field_is_string(scope, input, "weapon") || !field_is_string(scope, input, "ammo") {
        return Err("invalid-args".into());
    }
    let weapon = field_string(scope, input, "weapon")?;
    let ammo = field_string(scope, input, "ammo")?;
    let Some(data) = supply_v2::selected_data() else {
        return Err("missing-selected-data".into());
    };
    let row = ranged::range_loadout_of_items(data.items(), &weapon, &ammo);
    let obj = v8::Object::new(scope);
    let weapon = v8_str(scope, &row.weapon)?;
    set_key(scope, obj, "weapon", weapon);
    let projectile = v8_str(scope, &row.projectile)?;
    set_key(scope, obj, "projectile", projectile);
    let thrown = v8::Boolean::new(scope, row.thrown);
    set_key(scope, obj, "thrown", thrown.into());
    helper_ok(scope, obj.into())
}

fn v2_boost_faded<'s>(
    scope: &mut v8::HandleScope<'s>,
    input: v8::Local<v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let base = required_finite_field(scope, input, "base")?;
    let effective = required_finite_field(scope, input, "effective")?;
    let floor = match optional_number_field(scope, input, "floor")? {
        None => BOOST_FLOOR,
        Some(n) => loadout_plan::finite_number(n).ok_or_else(|| "invalid-args".to_string())?,
    };
    let faded = boost_potions::boost_faded(
        boost_potions::JsNumber::Finite(base),
        boost_potions::JsNumber::Finite(effective),
        boost_potions::JsNumber::Finite(floor),
    );
    let value = v8::Boolean::new(scope, faded);
    helper_ok(scope, value.into())
}

fn v2_planned_potions<'s>(
    scope: &mut v8::HandleScope<'s>,
    input: v8::Local<v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let carry_val = field(scope, input, "carry")?;
    if !js_array_is_array(scope, carry_val) {
        return Err("invalid-args".into());
    }
    let carry = parse_planned_carry(scope, carry_val)?;
    let plans = boost_potions::planned_potions(&carry);
    let arr = materialize_plans(scope, &plans)?;
    helper_ok(scope, arr)
}

fn v2_potion_to_sip<'s>(
    scope: &mut v8::HandleScope<'s>,
    input: v8::Local<v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let plans_val = field(scope, input, "plans")?;
    let held_val = field(scope, input, "held")?;
    let levels_val = field(scope, input, "levels")?;
    if !js_array_is_array(scope, plans_val)
        || !js_array_is_array(scope, held_val)
        || !js_array_is_array(scope, levels_val)
    {
        return Err("invalid-args".into());
    }
    let plans = parse_plans(scope, plans_val)?;
    let held = parse_held(scope, held_val)?;
    let levels = parse_levels(scope, levels_val)?;
    match boost_potions::potion_to_sip(&plans, &held, &levels) {
        Ok(None) => {
            let null = v8::null(scope);
            helper_ok(scope, null.into())
        }
        Ok(Some(plan)) => {
            let value = materialize_plan(scope, &plan)?;
            helper_ok(scope, value)
        }
        Err(PotionToSipError::InvalidArgs) => Err("invalid-args".into()),
        Err(PotionToSipError::MissingObservation) => Err("missing-observation".into()),
    }
}

fn required_loadout(
    scope: &mut v8::HandleScope,
    input: v8::Local<v8::Value>,
) -> Result<Option<LoadoutInput>, String> {
    if !input.is_object() || input.is_null() || input.is_undefined() {
        return Err("invalid-args".into());
    }
    let value = field(scope, input, "loadout")?;
    if value.is_undefined() {
        return Err("invalid-args".into());
    }
    if value.is_null() {
        return Ok(None);
    }
    Ok(Some(parse_loadout(scope, value)?))
}

fn parse_loadout(
    scope: &mut v8::HandleScope,
    value: v8::Local<v8::Value>,
) -> Result<LoadoutInput, String> {
    if !is_plain_object(value) {
        return Err("invalid-args".into());
    }
    let name = optional_string_field(scope, value, "name")?;
    let worn_val = field(scope, value, "worn")?;
    let worn = if worn_val.is_undefined() || worn_val.is_null() {
        BTreeMap::new()
    } else {
        parse_worn(scope, worn_val)?
    };
    let carry_val = field(scope, value, "carry")?;
    let carry = if carry_val.is_undefined() || carry_val.is_null() {
        None
    } else {
        Some(parse_loadout_carry(scope, carry_val)?)
    };
    let unassigned_val = field(scope, value, "unassigned")?;
    let unassigned = if unassigned_val.is_undefined() || unassigned_val.is_null() {
        None
    } else {
        if !js_array_is_array(scope, unassigned_val) {
            return Err("invalid-args".into());
        }
        Some(required_string_array(scope, unassigned_val)?)
    };
    Ok(LoadoutInput {
        name,
        worn,
        carry,
        unassigned,
    })
}

fn parse_worn(
    scope: &mut v8::HandleScope,
    value: v8::Local<v8::Value>,
) -> Result<BTreeMap<String, String>, String> {
    if !is_plain_object(value) {
        return Err("invalid-args".into());
    }
    let keys = object_keys(scope, value)?;
    let mut worn = BTreeMap::new();
    for key in keys {
        let item = field(scope, value, &key)?;
        if !item.is_string() {
            return Err("invalid-args".into());
        }
        worn.insert(key, js_to_string(scope, item)?);
    }
    Ok(worn)
}

fn parse_loadout_carry(
    scope: &mut v8::HandleScope,
    value: v8::Local<v8::Value>,
) -> Result<Vec<LoadoutCarry>, String> {
    if !js_array_is_array(scope, value) {
        return Err("invalid-args".into());
    }
    let obj = value.to_object(scope).ok_or_else(|| "invalid-args".to_string())?;
    let len = array_len(scope, obj)?;
    let mut out = Vec::with_capacity(len as usize);
    for i in 0..len {
        let row = index_at(scope, obj, i)?;
        if !is_plain_object(row) {
            return Err("invalid-args".into());
        }
        if !field_is_string(scope, row, "item") {
            return Err("invalid-args".into());
        }
        let item = field_string(scope, row, "item")?;
        let qty = match optional_number_field(scope, row, "qty")? {
            None => None,
            Some(n) => Some(loadout_plan::positive_u32(n).ok_or_else(|| "invalid-args".to_string())?),
        };
        out.push(LoadoutCarry { item, qty });
    }
    Ok(out)
}

fn parse_planned_carry(
    scope: &mut v8::HandleScope,
    value: v8::Local<v8::Value>,
) -> Result<Vec<PlannedCarry>, String> {
    let obj = value.to_object(scope).ok_or_else(|| "invalid-args".to_string())?;
    let len = array_len(scope, obj)?;
    let mut out = Vec::with_capacity(len as usize);
    for i in 0..len {
        let row = index_at(scope, obj, i)?;
        if !is_plain_object(row) || !field_is_string(scope, row, "item") {
            return Err("invalid-args".into());
        }
        let qty_val = field(scope, row, "qty")?;
        if qty_val.is_undefined() || !qty_val.is_number() {
            return Err("invalid-args".into());
        }
        let qty_n = qty_val
            .number_value(scope)
            .ok_or_else(|| "invalid-args".to_string())?;
        let qty = loadout_plan::positive_u32(qty_n).ok_or_else(|| "invalid-args".to_string())?;
        out.push(PlannedCarry {
            item: field_string(scope, row, "item")?,
            qty,
        });
    }
    Ok(out)
}

fn parse_plans(
    scope: &mut v8::HandleScope,
    value: v8::Local<v8::Value>,
) -> Result<Vec<PotionPlan>, String> {
    let obj = value.to_object(scope).ok_or_else(|| "invalid-args".to_string())?;
    let len = array_len(scope, obj)?;
    let mut out = Vec::with_capacity(len as usize);
    for i in 0..len {
        let row = index_at(scope, obj, i)?;
        if !is_plain_object(row)
            || !field_is_string(scope, row, "skill")
            || !field_is_string(scope, row, "flask")
        {
            return Err("invalid-args".into());
        }
        let doses_val = field(scope, row, "doses")?;
        if !js_array_is_array(scope, doses_val) {
            return Err("invalid-args".into());
        }
        let want_val = field(scope, row, "want")?;
        if !want_val.is_number() {
            return Err("invalid-args".into());
        }
        let want_n = want_val
            .number_value(scope)
            .ok_or_else(|| "invalid-args".to_string())?;
        let want = loadout_plan::positive_u32(want_n).ok_or_else(|| "invalid-args".to_string())?;
        out.push(PotionPlan {
            skill: field_string(scope, row, "skill")?,
            short: optional_string_field(scope, row, "short")?,
            flask: field_string(scope, row, "flask")?,
            doses: required_string_array(scope, doses_val)?,
            want,
        });
    }
    Ok(out)
}

fn parse_held(scope: &mut v8::HandleScope, value: v8::Local<v8::Value>) -> Result<Vec<f64>, String> {
    let obj = value.to_object(scope).ok_or_else(|| "invalid-args".to_string())?;
    let len = array_len(scope, obj)?;
    let mut out = Vec::with_capacity(len as usize);
    for i in 0..len {
        let row = index_at(scope, obj, i)?;
        if !row.is_number() {
            return Err("invalid-args".into());
        }
        let n = row
            .number_value(scope)
            .ok_or_else(|| "invalid-args".to_string())?;
        out.push(loadout_plan::held_count(n).ok_or_else(|| "invalid-args".to_string())?);
    }
    Ok(out)
}

fn parse_levels(
    scope: &mut v8::HandleScope,
    value: v8::Local<v8::Value>,
) -> Result<Vec<PotionLevel>, String> {
    let obj = value.to_object(scope).ok_or_else(|| "invalid-args".to_string())?;
    let len = array_len(scope, obj)?;
    let mut out = Vec::with_capacity(len as usize);
    for i in 0..len {
        let row = index_at(scope, obj, i)?;
        if !is_plain_object(row) || !field_is_string(scope, row, "skill") {
            return Err("invalid-args".into());
        }
        out.push(PotionLevel {
            skill: field_string(scope, row, "skill")?,
            base: required_finite_field(scope, row, "base")?,
            effective: required_finite_field(scope, row, "effective")?,
        });
    }
    Ok(out)
}

fn materialize_plans<'s>(
    scope: &mut v8::HandleScope<'s>,
    plans: &[PotionPlan],
) -> Result<v8::Local<'s, v8::Value>, String> {
    let arr = v8::Array::new(scope, plans.len() as i32);
    for (i, plan) in plans.iter().enumerate() {
        let row = materialize_plan(scope, plan)?;
        arr.set_index(scope, i as u32, row)
            .ok_or_else(|| "array".to_string())?;
    }
    Ok(arr.into())
}

fn materialize_plan<'s>(
    scope: &mut v8::HandleScope<'s>,
    plan: &PotionPlan,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let obj = v8::Object::new(scope);
    let skill = v8_str(scope, &plan.skill)?;
    set_key(scope, obj, "skill", skill);
    if let Some(short) = &plan.short {
        let short = v8_str(scope, short)?;
        set_key(scope, obj, "short", short);
    }
    let flask = v8_str(scope, &plan.flask)?;
    set_key(scope, obj, "flask", flask);
    let doses = string_array(scope, &plan.doses)?;
    set_key(scope, obj, "doses", doses);
    let want = v8::Number::new(scope, f64::from(plan.want));
    set_key(scope, obj, "want", want.into());
    Ok(obj.into())
}

fn string_array<'s>(
    scope: &mut v8::HandleScope<'s>,
    names: &[String],
) -> Result<v8::Local<'s, v8::Value>, String> {
    let arr = v8::Array::new(scope, names.len() as i32);
    for (i, name) in names.iter().enumerate() {
        let s = v8_str(scope, name)?;
        arr.set_index(scope, i as u32, s)
            .ok_or_else(|| "array".to_string())?;
    }
    Ok(arr.into())
}

fn required_string_field(
    scope: &mut v8::HandleScope,
    value: v8::Local<v8::Value>,
    name: &str,
) -> Result<String, String> {
    if !field_is_string(scope, value, name) {
        return Err("invalid-args".into());
    }
    field_string(scope, value, name)
}

fn required_finite_field(
    scope: &mut v8::HandleScope,
    value: v8::Local<v8::Value>,
    name: &str,
) -> Result<f64, String> {
    let v = field(scope, value, name)?;
    if !v.is_number() {
        return Err("invalid-args".into());
    }
    let n = v
        .number_value(scope)
        .ok_or_else(|| "invalid-args".to_string())?;
    loadout_plan::finite_number(n).ok_or_else(|| "invalid-args".to_string())
}

fn optional_number_field(
    scope: &mut v8::HandleScope,
    value: v8::Local<v8::Value>,
    name: &str,
) -> Result<Option<f64>, String> {
    let v = field(scope, value, name)?;
    if v.is_undefined() {
        return Ok(None);
    }
    if !v.is_number() {
        return Err("invalid-args".into());
    }
    v.number_value(scope)
        .map(Some)
        .ok_or_else(|| "invalid-args".to_string())
}

fn optional_string_field(
    scope: &mut v8::HandleScope,
    value: v8::Local<v8::Value>,
    name: &str,
) -> Result<Option<String>, String> {
    let v = field(scope, value, name)?;
    if v.is_undefined() || v.is_null() {
        return Ok(None);
    }
    if !v.is_string() {
        return Err("invalid-args".into());
    }
    Ok(Some(js_to_string(scope, v)?))
}

fn optional_string_or_null(
    scope: &mut v8::HandleScope,
    value: v8::Local<v8::Value>,
    name: &str,
) -> Result<Option<String>, String> {
    optional_string_field(scope, value, name)
}

fn is_plain_object(value: v8::Local<v8::Value>) -> bool {
    value.is_object() && !value.is_null() && !value.is_undefined() && !value.is_array()
}

fn object_keys(scope: &mut v8::HandleScope, value: v8::Local<v8::Value>) -> Result<Vec<String>, String> {
    let context = scope.get_current_context();
    let global = context.global(scope);
    let Some(object_key) = v8::String::new(scope, "Object") else {
        return Err("string".into());
    };
    let Some(object_val) = global.get(scope, object_key.into()) else {
        return Err(PENDING.into());
    };
    let Some(object_obj) = object_val.to_object(scope) else {
        return Err("invalid-args".into());
    };
    let Some(keys_key) = v8::String::new(scope, "keys") else {
        return Err("string".into());
    };
    let Some(fn_val) = object_obj.get(scope, keys_key.into()) else {
        return Err(PENDING.into());
    };
    let Ok(func) = v8::Local::<v8::Function>::try_from(fn_val) else {
        return Err("invalid-args".into());
    };
    let Some(result) = func.call(scope, object_val, &[value]) else {
        return Err(PENDING.into());
    };
    required_string_array(scope, result)
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
    obj.get(scope, key.into()).ok_or_else(|| PENDING.to_string())
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

fn required_string_array(
    scope: &mut v8::HandleScope,
    value: v8::Local<v8::Value>,
) -> Result<Vec<String>, String> {
    let obj = value.to_object(scope).ok_or_else(|| "invalid-args".to_string())?;
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

fn js_to_string(scope: &mut v8::HandleScope, value: v8::Local<v8::Value>) -> Result<String, String> {
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
    let err = v8_str(scope, error)?;
    set_key(scope, obj, "error", err);
    Ok(obj.into())
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
