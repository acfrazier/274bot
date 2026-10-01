//! Typed local V8 marshalling for released quest Path queries.
//! Not a rustyscript `register_function` JSON op.

use rustyscript::Runtime;

pub(super) fn install(runtime: &mut Runtime) -> Result<(), String> {
    let context = runtime.deno_runtime().main_context();
    let mut scope = runtime.deno_runtime().handle_scope();
    let global = context.open(&mut scope).global(&mut scope);
    let name = v8::String::new(&mut scope, "__rs2b0t_progress_methods_v2")
        .ok_or_else(|| "progress name".to_string())?;
    let func = v8::Function::new(&mut scope, progress_callback)
        .ok_or_else(|| "progress fn".to_string())?;
    global
        .set(&mut scope, name.into(), func.into())
        .ok_or_else(|| "progress set".to_string())?;
    Ok(())
}

fn progress_callback(
    scope: &mut v8::HandleScope,
    _args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    match quest_paths(scope) {
        Ok(value) => rv.set(value),
        Err(error) => match helper_err(scope, &error) {
            Ok(value) => rv.set(value),
            Err(_) => rv.set(v8::null(scope).into()),
        },
    }
}

fn quest_paths<'s>(
    scope: &mut v8::HandleScope<'s>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let rows = crate::quester::card::released_paths();
    let length = i32::try_from(rows.len()).map_err(|_| "quest-paths".to_string())?;
    let values = v8::Array::new(scope, length);
    for (index, row) in rows.iter().enumerate() {
        let index = u32::try_from(index).map_err(|_| "quest-paths".to_string())?;
        let stages_length =
            i32::try_from(row.stages.len()).map_err(|_| "quest-paths".to_string())?;
        let stages = v8::Array::new(scope, stages_length);
        for (stage_index, stage) in row.stages.iter().enumerate() {
            let stage_index = u32::try_from(stage_index).map_err(|_| "quest-paths".to_string())?;
            let stage = v8_str(scope, stage)?;
            stages
                .set_index(scope, stage_index, stage)
                .ok_or_else(|| "quest-paths".to_string())?;
        }

        let value = v8::Object::new(scope);
        let id = v8_str(scope, &row.id)?;
        set_key(scope, value, "id", id);
        let display = v8_str(scope, &row.display)?;
        set_key(scope, value, "display", display);
        let journal = v8::Boolean::new(scope, row.journal);
        set_key(scope, value, "journal", journal.into());
        set_key(scope, value, "stages", stages.into());
        values
            .set_index(scope, index, value.into())
            .ok_or_else(|| "quest-paths".to_string())?;
    }

    let rows = helper_ok(scope, values.into())?;
    Ok(rows)
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
