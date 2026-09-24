//! Typed V8 marshalling for v1 `meleeWeapons.js`: one native call per export
//! (`__rs2b0t_melee_weapon('best' | 'known', …)`), deciding in
//! `crate::melee_weapons` over the selected `melee_weapons` family.

use crate::melee_weapons::{self, WeaponPick};
use rustyscript::Runtime;

const PENDING: &str = "__pending__";

pub(super) fn install(runtime: &mut Runtime) -> Result<(), String> {
    let context = runtime.deno_runtime().main_context();
    let mut scope = runtime.deno_runtime().handle_scope();
    let global = context.open(&mut scope).global(&mut scope);
    let name = v8::String::new(&mut scope, "__rs2b0t_melee_weapon")
        .ok_or_else(|| "melee weapon name".to_string())?;
    let func = v8::Function::new(&mut scope, melee_weapon_callback)
        .ok_or_else(|| "melee weapon fn".to_string())?;
    global
        .set(&mut scope, name.into(), func.into())
        .ok_or_else(|| "melee weapon set".to_string())?;
    Ok(())
}

fn melee_weapon_callback(
    scope: &mut v8::HandleScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    match run(scope, &args) {
        Ok(Some(name)) => match v8::String::new(scope, &name) {
            Some(s) => rv.set(s.into()),
            None => rv.set(v8::null(scope).into()),
        },
        Ok(None) => rv.set(v8::null(scope).into()),
        Err(err) if err == PENDING => {}
        Err(err) => throw_error(scope, &err),
    }
}

/// `best`: (available, attack, preferStab, unusable[]); `known`: (names).
fn run(
    scope: &mut v8::HandleScope,
    args: &v8::FunctionCallbackArguments,
) -> Result<Option<String>, String> {
    let op = args.get(0).to_rust_string_lossy(scope);
    let export = match op.as_str() {
        "best" => "bestMeleeWeapon",
        "known" => "knownMeleeWeapon",
        _ => return Err(format!("not impl: meleeWeapons.{op}")),
    };
    let data = crate::supply_v2::selected_data();
    let family = data
        .as_deref()
        .and_then(|data| data.equipment_names())
        .map(|facts| facts.melee_weapons.as_slice())
        .ok_or_else(|| format!("not impl: {export}: selected melee_weapons facts absent"))?;
    if op == "known" {
        let names = string_array(scope, args.get(1))?;
        return Ok(melee_weapons::known_melee_weapon(family, &names));
    }
    let available = string_array(scope, args.get(1))?;
    let attack = args.get(2).number_value(scope).unwrap_or(f64::NAN);
    let prefer_stab = args.get(3).boolean_value(scope);
    let unusable = string_array(scope, args.get(4))?;
    Ok(melee_weapons::best_melee_weapon(
        family,
        &available,
        &WeaponPick {
            attack,
            prefer_stab,
            unusable: &unusable,
        },
    ))
}

/// Array elements coerced with `String(x)`; anything else is empty.
fn string_array(
    scope: &mut v8::HandleScope,
    value: v8::Local<v8::Value>,
) -> Result<Vec<String>, String> {
    let Ok(array) = v8::Local::<v8::Array>::try_from(value) else {
        return Ok(Vec::new());
    };
    let mut out = Vec::with_capacity(array.length() as usize);
    for i in 0..array.length() {
        let item = array
            .get_index(scope, i)
            .ok_or_else(|| PENDING.to_string())?;
        let text = item.to_string(scope).ok_or_else(|| PENDING.to_string())?;
        out.push(text.to_rust_string_lossy(scope));
    }
    Ok(out)
}

fn throw_error(scope: &mut v8::HandleScope, message: &str) {
    if let Some(s) = v8::String::new(scope, message) {
        let exc = v8::Exception::error(scope, s);
        scope.throw_exception(exc);
    }
}
