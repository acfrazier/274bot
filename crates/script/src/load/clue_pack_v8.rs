//! Typed local V8 marshalling for the trail pack-plan, hard-kit, and keep
//! helpers. Not a rustyscript `register_function` JSON op and not a clue
//! machine.

use rustyscript::Runtime;
use serde_json::Value;

const PENDING: &str = "__pending__";
const INVALID_ARGS: &str = "invalid-args";

pub(super) fn install(runtime: &mut Runtime) -> Result<(), String> {
    let context = runtime.deno_runtime().main_context();
    let mut scope = runtime.deno_runtime().handle_scope();
    let global = context.open(&mut scope).global(&mut scope);
    let name = v8::String::new(&mut scope, "__rs2b0t_clue_pack_v2")
        .ok_or_else(|| "clue pack name".to_string())?;
    let func = v8::Function::new(&mut scope, clue_pack_callback)
        .ok_or_else(|| "clue pack fn".to_string())?;
    global
        .set(&mut scope, name.into(), func.into())
        .ok_or_else(|| "clue pack set".to_string())?;
    Ok(())
}

fn clue_pack_callback(
    scope: &mut v8::HandleScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    match run_clue_pack(scope, &args) {
        Ok(value) => rv.set(value),
        Err(err) if err == PENDING => {}
        Err(err) => match helper_err(scope, &err) {
            Ok(value) => rv.set(value),
            Err(_) => rv.set(v8::null(scope).into()),
        },
    }
}

fn run_clue_pack<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: &v8::FunctionCallbackArguments,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let op = args.get(0);
    if !op.is_string() {
        return Err(INVALID_ARGS.into());
    }
    let op = js_to_string(scope, op)?;
    match op.as_str() {
        "packPlan" => pack_plan_op(scope, args.get(1)),
        "hardKit" => hard_kit_op(scope, args.get(1)),
        "keep" => keep_op(scope, args.get(1)),
        _ => Err(INVALID_ARGS.into()),
    }
}

/// The caller's own numbers, and nothing else: no snapshot, no page, no
/// inventory, no equipment, no bank, and no trail row. Presence is
/// `has_own_property`, so a present `null`, a wrong type, a fraction, a
/// negative, `-0`, a string number, a bigint, or a boxed `Number` is
/// `invalid-args` rather than the omitted default. Extra keys are ignored.
fn pack_plan_op<'s>(
    scope: &mut v8::HandleScope<'s>,
    input: v8::Local<v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    if input.is_null() || input.is_undefined() || input.is_array() || !input.is_object() {
        return Err(INVALID_ARGS.into());
    }
    let weapon_name = optional_text(scope, input, "weaponName")?;
    let casket_alias = optional_text(scope, input, "casketAlias")?;
    let plan = api::clue_pack::PackPlanInput {
        budget: api::clue_pack::PackBudget {
            host_want: count(scope, input, "hostWant")?,
            held_food: count(scope, input, "heldFood")?,
            free_slots: count(scope, input, "freeSlots")?,
            reserve_slots: count(scope, input, "reserveSlots")?,
        },
        per_cast: optional_count(scope, input, "perCast")?,
        weapon_name: weapon_name.as_deref(),
        weapon_in_backpack: flag(scope, input, "weaponInBackpack")?,
        weapon_equipped: flag(scope, input, "weaponEquipped")?,
        casket_alias: casket_alias.as_deref(),
    };
    match api::clue_pack::pack_plan(&plan) {
        Ok(value) => {
            let value = materialize(scope, &value)?;
            helper_ok(scope, value)
        }
        Err(error) => Err(error.into()),
    }
}

/// The caller's own kit facts, and nothing else: no quest tab, no snapshot, no
/// inventory, no equipment, and no bank. Every field is required here, so an
/// omitted `attack` is `invalid-args` rather than the status `attack` and an
/// omitted `lostCity` is `invalid-args` rather than `lost-city`. Extra keys are
/// ignored, on the input and on each item.
fn hard_kit_op<'s>(
    scope: &mut v8::HandleScope<'s>,
    input: v8::Local<v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    if input.is_null() || input.is_undefined() || input.is_array() || !input.is_object() {
        return Err(INVALID_ARGS.into());
    }
    let attack = required_count(scope, input, "attack")?;
    let lost_city = required_flag(scope, input, "lostCity")?;
    let items = required_items(scope, input, "items")?;
    let kit = api::clue_pack::HardKitInput {
        attack,
        lost_city,
        items: &items,
    };
    match api::clue_pack::hard_clue_kit(&kit) {
        Ok(value) => {
            let value = materialize(scope, &value)?;
            helper_ok(scope, value)
        }
        Err(error) => Err(error.into()),
    }
}

/// The caller's own display name, and the caller's own additive names:
/// nothing else. No snapshot, no `snapshot.inv` page, no bank page, no
/// equipment, and no trail family. `name` is required, so `keep({})` is
/// `invalid-args` rather than a missing name; `extra` is optional and only
/// adds. Extra input keys are ignored, and a kept name is never an error.
fn keep_op<'s>(
    scope: &mut v8::HandleScope<'s>,
    input: v8::Local<v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    if input.is_null() || input.is_undefined() || input.is_array() || !input.is_object() {
        return Err(INVALID_ARGS.into());
    }
    let name = required_text(scope, input, "name")?;
    let extra = optional_text_list(scope, input, "extra")?;
    let value = materialize(scope, &api::clue_pack::keep_clue_kit(&name, &extra))?;
    helper_ok(scope, value)
}

/// A required field. Absent is `invalid-args`: the `count()` default of `0`
/// would turn an omitted `attack` into the status `attack`.
fn required_count(
    scope: &mut v8::HandleScope,
    input: v8::Local<v8::Value>,
    name: &str,
) -> Result<i32, String> {
    match optional_count(scope, input, name)? {
        Some(value) => Ok(value),
        None => Err(INVALID_ARGS.into()),
    }
}

/// A required string, including `""`. Absent is `invalid-args`: the
/// `optional_text` default of `None` would turn an omitted `name` into a
/// missing name rather than an argument error, and a present non-string is
/// never coerced.
fn required_text(
    scope: &mut v8::HandleScope,
    input: v8::Local<v8::Value>,
    name: &str,
) -> Result<String, String> {
    match optional_text(scope, input, name)? {
        Some(value) => Ok(value),
        None => Err(INVALID_ARGS.into()),
    }
}

/// A required real `i32`. Unlike a count it may be negative: an item id that
/// matches no dose, dagger, or shark is a miss in the frozen reduce, not an
/// argument error. `is_int32` is false for a string, a fraction, a bigint, a
/// boxed number, and negative zero.
fn required_i32(
    scope: &mut v8::HandleScope,
    input: v8::Local<v8::Value>,
    name: &str,
) -> Result<i32, String> {
    if !has_own(scope, input, name)? {
        return Err(INVALID_ARGS.into());
    }
    let value = field(scope, input, name)?;
    if !value.is_int32() {
        return Err(INVALID_ARGS.into());
    }
    value
        .int32_value(scope)
        .ok_or_else(|| INVALID_ARGS.to_string())
}

/// A required boolean. Absent is `invalid-args`: the `flag()` default of
/// `false` would turn an omitted `lostCity` into the status `lost-city`, and
/// `1`/`0` are not booleans here either.
fn required_flag(
    scope: &mut v8::HandleScope,
    input: v8::Local<v8::Value>,
    name: &str,
) -> Result<bool, String> {
    if !has_own(scope, input, name)? {
        return Err(INVALID_ARGS.into());
    }
    let value = field(scope, input, name)?;
    if !value.is_boolean() {
        return Err(INVALID_ARGS.into());
    }
    Ok(value.boolean_value(scope))
}

/// A required array of kit rows. An element that is not an object, or whose
/// `id` or `count` is missing, `null`, or the wrong type, is `invalid-args`:
/// the frozen reduce reads those two fields only, so an extra `worn`, `slot`,
/// or `name` key is ignored rather than refused.
fn required_items(
    scope: &mut v8::HandleScope,
    input: v8::Local<v8::Value>,
    name: &str,
) -> Result<Vec<api::clue_pack::HardKitItem>, String> {
    if !has_own(scope, input, name)? {
        return Err(INVALID_ARGS.into());
    }
    let value = field(scope, input, name)?;
    if !value.is_array() {
        return Err(INVALID_ARGS.into());
    }
    let rows = v8::Local::<v8::Array>::try_from(value).map_err(|_| INVALID_ARGS.to_string())?;
    let mut items = Vec::with_capacity(rows.length() as usize);
    for index in 0..rows.length() {
        let row = rows
            .get_index(scope, index)
            .ok_or_else(|| PENDING.to_string())?;
        if row.is_null() || row.is_undefined() || row.is_array() || !row.is_object() {
            return Err(INVALID_ARGS.into());
        }
        items.push(api::clue_pack::HardKitItem {
            id: required_i32(scope, row, "id")?,
            count: required_count(scope, row, "count")?,
        });
    }
    Ok(items)
}

/// An absent field is `0`. The same real-`i32` check as `clue.row`, plus the
/// non-negative rule: a present value that is not one is `invalid-args`, never
/// the default and never clamped.
fn count(
    scope: &mut v8::HandleScope,
    input: v8::Local<v8::Value>,
    name: &str,
) -> Result<i32, String> {
    match optional_count(scope, input, name)? {
        Some(value) => Ok(value),
        None => Ok(0),
    }
}

fn optional_count(
    scope: &mut v8::HandleScope,
    input: v8::Local<v8::Value>,
    name: &str,
) -> Result<Option<i32>, String> {
    if !has_own(scope, input, name)? {
        return Ok(None);
    }
    let value = field(scope, input, name)?;
    if !value.is_int32() {
        return Err(INVALID_ARGS.into());
    }
    let value = value
        .int32_value(scope)
        .ok_or_else(|| INVALID_ARGS.to_string())?;
    if value < 0 {
        return Err(INVALID_ARGS.into());
    }
    Ok(Some(value))
}

/// An absent field is `false`; a present non-boolean is `invalid-args`. `1` and
/// `0` are not booleans here.
fn flag(
    scope: &mut v8::HandleScope,
    input: v8::Local<v8::Value>,
    name: &str,
) -> Result<bool, String> {
    if !has_own(scope, input, name)? {
        return Ok(false);
    }
    let value = field(scope, input, name)?;
    if !value.is_boolean() {
        return Err(INVALID_ARGS.into());
    }
    Ok(value.boolean_value(scope))
}

/// A present string, including `""`. Absent publishes no field, and a present
/// non-string is `invalid-args`: nothing here coerces a number to a string.
fn optional_text(
    scope: &mut v8::HandleScope,
    input: v8::Local<v8::Value>,
    name: &str,
) -> Result<Option<String>, String> {
    if !has_own(scope, input, name)? {
        return Ok(None);
    }
    let value = field(scope, input, name)?;
    if !value.is_string() {
        return Err(INVALID_ARGS.into());
    }
    Ok(Some(js_to_string(scope, value)?))
}

/// An optional list of caller names. Absent is empty; a present `null`, a
/// non-array, or any non-string element (a hole included) is `invalid-args`.
/// An empty array is a present empty list, and `""` is a present entry.
fn optional_text_list(
    scope: &mut v8::HandleScope,
    input: v8::Local<v8::Value>,
    name: &str,
) -> Result<Vec<String>, String> {
    if !has_own(scope, input, name)? {
        return Ok(Vec::new());
    }
    let value = field(scope, input, name)?;
    if !value.is_array() {
        return Err(INVALID_ARGS.into());
    }
    let rows = v8::Local::<v8::Array>::try_from(value).map_err(|_| INVALID_ARGS.to_string())?;
    let mut entries = Vec::with_capacity(rows.length() as usize);
    for index in 0..rows.length() {
        let entry = rows
            .get_index(scope, index)
            .ok_or_else(|| PENDING.to_string())?;
        if !entry.is_string() {
            return Err(INVALID_ARGS.into());
        }
        entries.push(js_to_string(scope, entry)?);
    }
    Ok(entries)
}

fn has_own(
    scope: &mut v8::HandleScope,
    value: v8::Local<v8::Value>,
    name: &str,
) -> Result<bool, String> {
    let obj = value
        .to_object(scope)
        .ok_or_else(|| INVALID_ARGS.to_string())?;
    let key = v8::String::new(scope, name).ok_or_else(|| "string".to_string())?;
    obj.has_own_property(scope, key.into())
        .ok_or_else(|| PENDING.to_string())
}

fn field<'s>(
    scope: &mut v8::HandleScope<'s>,
    value: v8::Local<v8::Value>,
    name: &str,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let obj = value
        .to_object(scope)
        .ok_or_else(|| INVALID_ARGS.to_string())?;
    let key = v8::String::new(scope, name).ok_or_else(|| "string".to_string())?;
    obj.get(scope, key.into())
        .ok_or_else(|| PENDING.to_string())
}

fn js_to_string(
    scope: &mut v8::HandleScope,
    value: v8::Local<v8::Value>,
) -> Result<String, String> {
    match value.to_string(scope) {
        Some(text) => Ok(text.to_rust_string_lossy(scope)),
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
    let error = v8_str(scope, error)?;
    set_key(scope, obj, "error", error);
    Ok(obj.into())
}

fn materialize<'s>(
    scope: &mut v8::HandleScope<'s>,
    value: &Value,
) -> Result<v8::Local<'s, v8::Value>, String> {
    match value {
        Value::Null => Ok(v8::null(scope).into()),
        Value::Bool(flag) => Ok(v8::Boolean::new(scope, *flag).into()),
        Value::Number(number) => {
            // Widened for `runeTarget`: an integral value past `i32` is
            // published as its exact double (`i32::MAX * 20` is a safe
            // integer), not refused and not truncated.
            match number.as_i64().map(i32::try_from) {
                Some(Ok(integer)) => Ok(v8::Integer::new(scope, integer).into()),
                _ => {
                    let float = number.as_f64().ok_or_else(|| "number".to_string())?;
                    Ok(v8::Number::new(scope, float).into())
                }
            }
        }
        Value::String(text) => v8_str(scope, text),
        Value::Array(items) => {
            let arr = v8::Array::new(scope, i32::try_from(items.len()).unwrap_or(i32::MAX));
            for (index, item) in items.iter().enumerate() {
                let value = materialize(scope, item)?;
                arr.set_index(scope, index as u32, value)
                    .ok_or_else(|| "array".to_string())?;
            }
            Ok(arr.into())
        }
        Value::Object(map) => {
            let obj = v8::Object::new(scope);
            for (key, item) in map {
                let value = materialize(scope, item)?;
                set_key(scope, obj, key, value);
            }
            Ok(obj.into())
        }
    }
}

fn v8_str<'s>(
    scope: &mut v8::HandleScope<'s>,
    text: &str,
) -> Result<v8::Local<'s, v8::Value>, String> {
    v8::String::new(scope, text)
        .map(|value| value.into())
        .ok_or_else(|| "string".to_string())
}

fn set_key(
    scope: &mut v8::HandleScope,
    obj: v8::Local<v8::Object>,
    key: &str,
    value: v8::Local<v8::Value>,
) {
    if let Some(key) = v8::String::new(scope, key) {
        let _ = obj.set(scope, key.into(), value);
    }
}
