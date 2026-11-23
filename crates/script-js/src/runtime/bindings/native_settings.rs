use rustyscript::Runtime;
use v8::{self, Local};

const LIST_READER: &str = "__rs2b0t_native_settings_list";
const TILE_READER: &str = "__rs2b0t_native_settings_tile";

/// Install typed readers over the existing host settings bag. The adapter
/// reads the current bag on every call and returns copies of compound values.
pub(super) fn install(runtime: &mut Runtime) -> Result<(), String> {
    let context = runtime.deno_runtime().main_context();
    let mut scope = runtime.deno_runtime().handle_scope();
    let global = context.open(&mut scope).global(&mut scope);
    let data = key_data(&mut scope)?;

    let list_name = v8::String::new(&mut scope, LIST_READER)
        .ok_or_else(|| "NativeSettings.list: create function name".to_string())?;
    let list = v8::Function::builder(list_setting)
        .data(data.into())
        .build(&mut scope)
        .ok_or_else(|| "NativeSettings.list: create callback".to_string())?;
    if !global
        .set(&mut scope, list_name.into(), list.into())
        .unwrap_or(false)
    {
        return Err("NativeSettings.list: install callback".to_string());
    }

    let tile_name = v8::String::new(&mut scope, TILE_READER)
        .ok_or_else(|| "NativeSettings.tile: create function name".to_string())?;
    let tile = v8::Function::builder(tile_setting)
        .data(data.into())
        .build(&mut scope)
        .ok_or_else(|| "NativeSettings.tile: create callback".to_string())?;
    if !global
        .set(&mut scope, tile_name.into(), tile.into())
        .unwrap_or(false)
    {
        return Err("NativeSettings.tile: install callback".to_string());
    }
    Ok(())
}

/// Callback data stores stable property keys once per isolate. The script only
/// passes a setting name and optional fallback; the Rust adapter owns access to
/// the host's current bag.
fn key_data<'s>(scope: &mut v8::HandleScope<'s>) -> Result<Local<'s, v8::Array>, String> {
    let keys = ["__rs2b0t_host", "settingsBag", "x", "z", "level"];
    let data = v8::Array::new(scope, keys.len() as i32);
    for (index, key) in keys.iter().enumerate() {
        let value = v8::String::new(scope, key)
            .ok_or_else(|| format!("NativeSettings: create `{key}` key"))?;
        if !data
            .set_index(scope, index as u32, value.into())
            .unwrap_or(false)
        {
            return Err(format!("NativeSettings: store `{key}` key"));
        }
    }
    Ok(data)
}

fn bag_setting<'s>(
    scope: &mut v8::HandleScope<'s>,
    data: Local<'s, v8::Value>,
    name: Local<'s, v8::Value>,
) -> Option<Local<'s, v8::Value>> {
    if !name.is_string() {
        return None;
    }
    let keys = Local::<v8::Array>::try_from(data).ok()?;
    let host_key = keys.get_index(scope, 0)?;
    let settings_key = keys.get_index(scope, 1)?;
    let global = scope.get_current_context().global(scope);
    let host = global.get(scope, host_key)?.to_object(scope)?;
    let bag = host.get(scope, settings_key)?.to_object(scope)?;
    bag.get(scope, name)
}

fn copy_string_array<'s>(
    scope: &mut v8::HandleScope<'s>,
    value: Local<'s, v8::Value>,
) -> Option<Local<'s, v8::Array>> {
    let source = Local::<v8::Array>::try_from(value).ok()?;
    let length = source.length();
    let capacity = i32::try_from(length).ok()?;
    let copy = v8::Array::new(scope, capacity);
    for index in 0..length {
        let item = source.get_index(scope, index)?;
        if !item.is_string() || !copy.set_index(scope, index, item).unwrap_or(false) {
            return None;
        }
    }
    Some(copy)
}

fn list_setting<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue,
) {
    let current = bag_setting(scope, args.data(), args.get(0));
    let value = current.and_then(|value| copy_string_array(scope, value));
    let value = value
        .or_else(|| copy_string_array(scope, args.get(1)))
        .unwrap_or_else(|| v8::Array::new(scope, 0));
    rv.set(value.into());
}

fn integer_field<'s>(
    scope: &mut v8::HandleScope<'s>,
    object: Local<'s, v8::Object>,
    key: Local<'s, v8::Value>,
) -> Option<i32> {
    let value = object.get(scope, key)?;
    if !value.is_number() {
        return None;
    }
    let number = value.number_value(scope)?;
    if !number.is_finite()
        || number.fract() != 0.0
        || number < f64::from(i32::MIN)
        || number > f64::from(i32::MAX)
    {
        return None;
    }
    Some(number as i32)
}

fn set_tile_field<'s>(
    scope: &mut v8::HandleScope<'s>,
    tile: Local<'s, v8::Object>,
    key: Local<'s, v8::Value>,
    value: i32,
) -> Option<()> {
    let key = Local::<v8::String>::try_from(key).ok()?;
    let value = v8::Integer::new(scope, value);
    tile.create_data_property(scope, key.into(), value.into())
        .filter(|created| *created)
        .map(|_| ())
}

fn copy_tile<'s>(
    scope: &mut v8::HandleScope<'s>,
    value: Local<'s, v8::Value>,
    data: Local<'s, v8::Value>,
) -> Option<Local<'s, v8::Object>> {
    if !value.is_object() || value.is_array() || value.is_function() {
        return None;
    }
    let keys = Local::<v8::Array>::try_from(data).ok()?;
    let source = value.to_object(scope)?;
    let x_key = keys.get_index(scope, 2)?;
    let z_key = keys.get_index(scope, 3)?;
    let level_key = keys.get_index(scope, 4)?;
    let x = integer_field(scope, source, x_key)?;
    let z = integer_field(scope, source, z_key)?;
    let level = integer_field(scope, source, level_key)?;
    if !(0..=3).contains(&level) {
        return None;
    }

    let tile = v8::Object::new(scope);
    set_tile_field(scope, tile, x_key, x)?;
    set_tile_field(scope, tile, z_key, z)?;
    set_tile_field(scope, tile, level_key, level)?;
    Some(tile)
}

fn tile_setting<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue,
) {
    let data = args.data();
    let current = bag_setting(scope, data, args.get(0));
    let value = current.and_then(|value| copy_tile(scope, value, data));
    let value = value.or_else(|| copy_tile(scope, args.get(1), data));
    match value {
        Some(value) => rv.set(value.into()),
        None => rv.set(v8::null(scope).into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustyscript::{Runtime, RuntimeOptions};

    const CHILD_ENV: &str = "SCRIPT_NATIVE_SETTINGS_TEST_CHILD";
    const FAST_TEST_NAME: &str = "runtime::bindings::native_settings::tests::settings_results_have_fast_properties";
    const HEAP_TEST_NAME: &str = "runtime::bindings::native_settings::tests::settings_reads_have_bounded_v8_heap";

    fn run_in_child(name: &str) -> bool {
        if std::env::var_os(CHILD_ENV).is_some() {
            return true;
        }
        let output = std::process::Command::new(std::env::current_exe().expect("test executable"))
            .args(["--exact", name, "--nocapture"])
            .env(CHILD_ENV, "1")
            .output()
            .expect("run isolated NativeSettings V8 test");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            output.status.success() && stdout.contains("1 passed"),
            "isolated NativeSettings V8 test failed\nstdout: {stdout}\nstderr: {stderr}"
        );
        false
    }

    fn setup() -> Runtime {
        crate::runtime::platform::bootstrap_test_v8_platform();
        let mut runtime = Runtime::new(RuntimeOptions::default()).expect("create V8 runtime");
        install(&mut runtime).expect("install NativeSettings adapters");
        runtime
            .eval::<()>(
                r#"globalThis.__rs2b0t_host = { settingsBag: {
                    names: ['alice', 'bob'],
                    home: { x: 3200, z: 3201, level: 1 },
                } };"#,
            )
            .expect("install settings bag");
        runtime
    }

    fn collected_used_heap(runtime: &mut Runtime) -> usize {
        let isolate = runtime.deno_runtime().v8_isolate();
        isolate.low_memory_notification();
        isolate.get_heap_statistics().used_heap_size()
    }

    #[test]
    fn settings_results_have_fast_properties() {
        if !run_in_child(FAST_TEST_NAME) {
            return;
        }
        v8::V8::set_flags_from_string("--allow-natives-syntax");
        let mut runtime = setup();
        let fast = runtime
            .eval::<bool>(
                r#"(() => {
                    const hasFastProperties = new Function('value', 'return %HasFastProperties(value)');
                    const tile = globalThis.__rs2b0t_native_settings_tile('home', null);
                    const names = globalThis.__rs2b0t_native_settings_list('names');
                    return hasFastProperties(tile) && Array.isArray(names) && names.length === 2;
                })()"#,
            )
            .expect("inspect NativeSettings result shapes");
        assert!(fast, "NativeSettings returns fast-property tiles and arrays");
    }

    #[test]
    fn settings_reads_have_bounded_v8_heap() {
        if !run_in_child(HEAP_TEST_NAME) {
            return;
        }
        let mut runtime = setup();
        runtime
            .eval::<()>(
                r#"globalThis.__repeatSettingsReads = (count) => {
                    let total = 0;
                    for (let i = 0; i < count; i++) {
                        total += globalThis.__rs2b0t_native_settings_list('names').length;
                        total += globalThis.__rs2b0t_native_settings_tile('home', null).x;
                    }
                    return total;
                };"#,
            )
            .expect("install repeated settings reads");
        assert!(runtime.eval::<usize>("__repeatSettingsReads(100)").unwrap() > 0);
        let baseline = collected_used_heap(&mut runtime);
        const REPEATS: usize = 1_000;
        let total = runtime
            .eval::<usize>(&format!("__repeatSettingsReads({REPEATS})"))
            .expect("repeat copied settings reads");
        assert_eq!(total, REPEATS * (2 + 3_200));
        let after = collected_used_heap(&mut runtime);
        let growth = after.saturating_sub(baseline);
        let limit = 64 * 1024;
        println!(
            "NativeSettings V8 heap repeats={REPEATS} baseline={baseline} after={after} growth={growth} limit={limit}"
        );
        assert!(
            growth <= limit,
            "collected V8 heap grew {growth} bytes; limit {limit}"
        );
    }
}
