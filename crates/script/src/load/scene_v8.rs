//! Typed local V8 marshalling for v2 `sceneLocs` / `sceneNpcs` / `questStatus`.
//! Not a rustyscript `register_function` JSON op and not a journal machine.

use crate::scene_query::{self, QuestStatusError, Region, SceneProjection, SceneQueryError};
use rustyscript::Runtime;

const PENDING: &str = "__pending__";

pub(super) fn install(runtime: &mut Runtime) -> Result<(), String> {
    let context = runtime.deno_runtime().main_context();
    let mut scope = runtime.deno_runtime().handle_scope();
    let global = context.open(&mut scope).global(&mut scope);
    let scene_name = v8::String::new(&mut scope, "__rs2b0t_scene_query")
        .ok_or_else(|| "scene name".to_string())?;
    let scene_fn =
        v8::Function::new(&mut scope, scene_callback).ok_or_else(|| "scene fn".to_string())?;
    global
        .set(&mut scope, scene_name.into(), scene_fn.into())
        .ok_or_else(|| "scene set".to_string())?;
    let quest_name = v8::String::new(&mut scope, "__rs2b0t_quest_status")
        .ok_or_else(|| "quest status name".to_string())?;
    let quest_fn = v8::Function::new(&mut scope, quest_callback)
        .ok_or_else(|| "quest status fn".to_string())?;
    global
        .set(&mut scope, quest_name.into(), quest_fn.into())
        .ok_or_else(|| "quest status set".to_string())?;
    Ok(())
}

fn scene_callback(
    scope: &mut v8::HandleScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    match run_scene(scope, &args) {
        Ok(value) => rv.set(value),
        Err(err) if err == PENDING => {}
        Err(err) => match helper_err(scope, &err) {
            Ok(value) => rv.set(value),
            Err(_) => rv.set(v8::null(scope).into()),
        },
    }
}

fn quest_callback(
    scope: &mut v8::HandleScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    match run_quest(scope, &args) {
        Ok(value) => rv.set(value),
        Err(err) if err == PENDING => {}
        Err(err) => match helper_err(scope, &err) {
            Ok(value) => rv.set(value),
            Err(_) => rv.set(v8::null(scope).into()),
        },
    }
}

fn run_scene<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: &v8::FunctionCallbackArguments,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let kind = js_to_string(scope, args.get(0))?;
    let input = args.get(1);
    match kind.as_str() {
        "locs" => scene_locs(scope, input),
        "npcs" => scene_npcs(scope, input),
        _ => helper_err(scope, SceneQueryError::InvalidArgs.as_str()),
    }
}

fn run_quest<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: &v8::FunctionCallbackArguments,
) -> Result<v8::Local<'s, v8::Value>, String> {
    quest_status(scope, args.get(0))
}

fn scene_locs<'s>(
    scope: &mut v8::HandleScope<'s>,
    input: v8::Local<v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let obj = object_arg(scope, input)?;
    let ids_v = field(scope, obj, "ids")?;
    if ids_v.is_null_or_undefined() {
        return helper_err(scope, SceneQueryError::MissingIds.as_str());
    }
    let ids = int_array(scope, ids_v)?;
    if ids.is_empty() {
        return helper_err(scope, SceneQueryError::MissingIds.as_str());
    }
    let limit = limit_field(scope, obj)?;
    let region = region_arg(scope, obj)?;
    match scene_query::scene_locs(&ids, limit, region) {
        Ok(hit) => materialize_projection(scope, &hit),
        Err(err) => helper_err(scope, err.as_str()),
    }
}

fn scene_npcs<'s>(
    scope: &mut v8::HandleScope<'s>,
    input: v8::Local<v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let obj = object_arg(scope, input)?;
    let types_v = field(scope, obj, "types")?;
    let types = int_array(scope, types_v)?;
    if types.is_empty() {
        return helper_err(scope, SceneQueryError::InvalidArgs.as_str());
    }
    let actions_v = field(scope, obj, "actions")?;
    let actions = string_array(scope, actions_v)?;
    if actions.is_empty() || actions.iter().any(|action| action.is_empty()) {
        return helper_err(scope, SceneQueryError::InvalidArgs.as_str());
    }
    let limit = limit_field(scope, obj)?;
    let region = region_arg(scope, obj)?;
    match scene_query::scene_npcs(&types, &actions, limit, region) {
        Ok(hit) => materialize_projection(scope, &hit),
        Err(err) => helper_err(scope, err.as_str()),
    }
}
fn quest_status<'s>(
    scope: &mut v8::HandleScope<'s>,
    input: v8::Local<v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let obj = object_arg(scope, input)?;
    if has_own(scope, obj, "id")? {
        return helper_err(scope, QuestStatusError::InvalidArgs.as_str());
    }
    let name_v = field(scope, obj, "name")?;
    if !name_v.is_string() {
        return helper_err(scope, QuestStatusError::InvalidArgs.as_str());
    }
    let name = js_to_string(scope, name_v)?;
    match scene_query::quest_status(&name) {
        Ok(hit) => {
            let value = v8::Object::new(scope);
            let status = v8_str(scope, &hit.status)?;
            set_key(scope, value, "status", status);
            let seq = v8::Number::new(scope, hit.as_of_sequence as f64);
            set_key(scope, value, "as_of_sequence", seq.into());
            helper_ok(scope, value.into())
        }
        Err(err) => helper_err(scope, err.as_str()),
    }
}
fn object_arg<'s>(
    scope: &mut v8::HandleScope<'s>,
    input: v8::Local<v8::Value>,
) -> Result<v8::Local<'s, v8::Object>, String> {
    if input.is_null_or_undefined() || !input.is_object() || input.is_array() {
        return Err(SceneQueryError::InvalidArgs.as_str().into());
    }
    input
        .to_object(scope)
        .ok_or_else(|| SceneQueryError::InvalidArgs.as_str().to_string())
}

fn int_array<'s>(
    scope: &mut v8::HandleScope<'s>,
    value: v8::Local<'s, v8::Value>,
) -> Result<Vec<i32>, String> {
    if !value.is_array() {
        return Err(SceneQueryError::InvalidArgs.as_str().into());
    }
    let arr = value
        .to_object(scope)
        .ok_or_else(|| SceneQueryError::InvalidArgs.as_str().to_string())?;
    let len = array_len(scope, arr)?;
    let mut out = Vec::with_capacity(len as usize);
    for i in 0..len {
        let item = index_at(scope, arr, i)?;
        out.push(required_i32(scope, item)?);
    }
    Ok(out)
}

fn string_array<'s>(
    scope: &mut v8::HandleScope<'s>,
    value: v8::Local<'s, v8::Value>,
) -> Result<Vec<String>, String> {
    if value.is_null_or_undefined() || !value.is_array() {
        return Err(SceneQueryError::InvalidArgs.as_str().into());
    }
    let arr = value
        .to_object(scope)
        .ok_or_else(|| SceneQueryError::InvalidArgs.as_str().to_string())?;
    let len = array_len(scope, arr)?;
    let mut out = Vec::with_capacity(len as usize);
    for i in 0..len {
        let item = index_at(scope, arr, i)?;
        if !item.is_string() {
            return Err(SceneQueryError::InvalidArgs.as_str().into());
        }
        out.push(js_to_string(scope, item)?);
    }
    Ok(out)
}

fn limit_field<'s>(
    scope: &mut v8::HandleScope<'s>,
    obj: v8::Local<'s, v8::Object>,
) -> Result<i32, String> {
    let value = field(scope, obj, "limit")?;
    let n = required_i32(scope, value)?;
    if !(1..=scene_query::SCENE_LIMIT_MAX).contains(&n) {
        return Err(SceneQueryError::InvalidArgs.as_str().into());
    }
    Ok(n)
}

fn region_arg<'s>(
    scope: &mut v8::HandleScope<'s>,
    obj: v8::Local<'s, v8::Object>,
) -> Result<Option<Region>, String> {
    if !has_own(scope, obj, "region")? {
        return Ok(None);
    }
    let value = field(scope, obj, "region")?;
    if value.is_null_or_undefined() || !value.is_object() || value.is_array() {
        return Err(SceneQueryError::InvalidArgs.as_str().into());
    }
    let region = value
        .to_object(scope)
        .ok_or_else(|| SceneQueryError::InvalidArgs.as_str().to_string())?;
    let min_x = required_i32_field(scope, region, "min_x")?;
    let min_z = required_i32_field(scope, region, "min_z")?;
    let max_x = required_i32_field(scope, region, "max_x")?;
    let max_z = required_i32_field(scope, region, "max_z")?;
    let level = required_i32_field(scope, region, "level")?;
    Ok(Some(Region {
        min_x,
        min_z,
        max_x,
        max_z,
        level,
    }))
}

fn required_i32_field<'s>(
    scope: &mut v8::HandleScope<'s>,
    obj: v8::Local<'s, v8::Object>,
    name: &str,
) -> Result<i32, String> {
    let v = field(scope, obj, name)?;
    required_i32(scope, v)
}

fn required_i32<'s>(
    scope: &mut v8::HandleScope<'s>,
    value: v8::Local<'s, v8::Value>,
) -> Result<i32, String> {
    if !value.is_number() {
        return Err(SceneQueryError::InvalidArgs.as_str().into());
    }
    let n = value
        .number_value(scope)
        .ok_or_else(|| PENDING.to_string())?;
    if !n.is_finite() || n.fract() != 0.0 {
        return Err(SceneQueryError::InvalidArgs.as_str().into());
    }
    if n < (i32::MIN as f64) || n > (i32::MAX as f64) {
        return Err(SceneQueryError::InvalidArgs.as_str().into());
    }
    Ok(n as i32)
}

fn array_len<'s>(
    scope: &mut v8::HandleScope<'s>,
    obj: v8::Local<'s, v8::Object>,
) -> Result<u32, String> {
    let key = v8::String::new(scope, "length").ok_or_else(|| "string".to_string())?;
    let Some(len) = obj.get(scope, key.into()) else {
        return Err(PENDING.into());
    };
    let n = len
        .number_value(scope)
        .ok_or_else(|| SceneQueryError::InvalidArgs.as_str().to_string())?;
    if !n.is_finite() || n < 0.0 {
        return Err(SceneQueryError::InvalidArgs.as_str().into());
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
    obj: v8::Local<'s, v8::Object>,
    name: &str,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let key = v8::String::new(scope, name).ok_or_else(|| "string".to_string())?;
    obj.get(scope, key.into())
        .ok_or_else(|| PENDING.to_string())
}

fn has_own<'s>(
    scope: &mut v8::HandleScope<'s>,
    obj: v8::Local<'s, v8::Object>,
    name: &str,
) -> Result<bool, String> {
    let key = v8::String::new(scope, name).ok_or_else(|| "string".to_string())?;
    obj.has_own_property(scope, key.into())
        .ok_or_else(|| PENDING.to_string())
}

fn js_to_string(
    scope: &mut v8::HandleScope,
    value: v8::Local<v8::Value>,
) -> Result<String, String> {
    match value.to_string(scope) {
        Some(s) => Ok(s.to_rust_string_lossy(scope)),
        None => Err(PENDING.into()),
    }
}
fn materialize_projection<'s>(
    scope: &mut v8::HandleScope<'s>,
    hit: &SceneProjection,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let value = v8::Object::new(scope);
    let seq = v8::Number::new(scope, hit.as_of_sequence as f64);
    set_key(scope, value, "as_of_sequence", seq.into());
    let scene = v8::Object::new(scope);
    let available = v8::Boolean::new(scope, hit.scene.available);
    set_key(scope, scene, "available", available.into());
    set_num(scope, scene, "base_x", hit.scene.base_x);
    set_num(scope, scene, "base_z", hit.scene.base_z);
    set_num(scope, scene, "level", hit.scene.level);
    set_num(scope, scene, "width", hit.scene.width);
    set_num(scope, scene, "height", hit.scene.height);
    set_key(scope, value, "scene", scene.into());
    let rows = v8::Array::new(scope, hit.rows.len() as i32);
    for (i, row) in hit.rows.iter().enumerate() {
        let obj = v8::Object::new(scope);
        set_num(scope, obj, "id", row.id);
        set_num(scope, obj, "x", row.x);
        set_num(scope, obj, "z", row.z);
        set_num(scope, obj, "level", row.level);
        let actions = v8::Array::new(scope, row.actions.len() as i32);
        for (j, action) in row.actions.iter().enumerate() {
            let text = v8_str(scope, action)?;
            actions
                .set_index(scope, j as u32, text)
                .ok_or_else(|| PENDING.to_string())?;
        }
        set_key(scope, obj, "actions", actions.into());
        rows.set_index(scope, i as u32, obj.into())
            .ok_or_else(|| PENDING.to_string())?;
    }
    set_key(scope, value, "rows", rows.into());
    let truncated = v8::Boolean::new(scope, hit.truncated);
    set_key(scope, value, "truncated", truncated.into());
    helper_ok(scope, value.into())
}

fn set_num<'s>(scope: &mut v8::HandleScope<'s>, obj: v8::Local<'s, v8::Object>, key: &str, n: i32) {
    let num = v8::Number::new(scope, n as f64);
    set_key(scope, obj, key, num.into());
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
    let msg = v8::String::new(scope, error).ok_or_else(|| "string".to_string())?;
    set_key(scope, obj, "error", msg.into());
    Ok(obj.into())
}

fn v8_str<'s>(
    scope: &mut v8::HandleScope<'s>,
    s: &str,
) -> Result<v8::Local<'s, v8::Value>, String> {
    Ok(v8::String::new(scope, s)
        .ok_or_else(|| "string".to_string())?
        .into())
}

fn set_key<'s>(
    scope: &mut v8::HandleScope<'s>,
    obj: v8::Local<'s, v8::Object>,
    key: &str,
    value: v8::Local<'s, v8::Value>,
) {
    if let Some(k) = v8::String::new(scope, key) {
        obj.set(scope, k.into(), value);
    }
}
