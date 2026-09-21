//! JSON `__rs2b0t_supply_v2` dispatch for NativeApi HelperResult methods.

use crate::escape_runes::{self, EscapeRunesError};
use crate::food_policy::{self, FoodHealOutcome, InvItemName};
use crate::keep_list::{self, CombatKeepOptions};
use api::game_data::SelectedGameData;
use serde_json::{json, Value};
use std::cell::RefCell;
use std::sync::Arc;

thread_local! {
    static GAME_DATA: RefCell<Option<Arc<SelectedGameData>>> = const { RefCell::new(None) };
}

pub fn configure(data: Option<Arc<SelectedGameData>>) {
    GAME_DATA.with(|slot| *slot.borrow_mut() = data);
}

pub(crate) fn selected_data() -> Option<Arc<SelectedGameData>> {
    GAME_DATA.with(|slot| slot.borrow().clone())
}

fn helper_ok(value: Value) -> Value {
    json!({ "ok": true, "value": value })
}

fn helper_err(error: &str) -> Value {
    json!({ "ok": false, "error": error })
}

pub fn dispatch(args: &[Value]) -> Value {
    let payload = args.first().and_then(Value::as_object);
    let Some(payload) = payload else {
        return helper_err("invalid-args");
    };
    let op = payload.get("op").and_then(Value::as_str).unwrap_or("");
    let input = payload.get("input").unwrap_or(&Value::Null);
    match op {
        "foodCount" => food_count_v2(input),
        "foodHealAmount" => food_heal_v2(input),
        "combatKeepNames" => combat_keep_names_v2(input),
        "runesPerCast" => runes_per_cast_v2(input),
        "escapeRunesFor" => escape_runes_v2(input),
        _ => helper_err("invalid-args"),
    }
}

fn food_count_v2(input: &Value) -> Value {
    let Some(items) = input.get("items").and_then(Value::as_array) else {
        return helper_err("invalid-args");
    };
    let Some(food_name) = input.get("foodName").and_then(Value::as_str) else {
        return helper_err("invalid-args");
    };
    let Some(data) = selected_data() else {
        return helper_err("missing-selected-data");
    };
    let mut rows = Vec::new();
    for row in items {
        if row.is_null() {
            continue;
        }
        let Some(obj) = row.as_object() else {
            continue;
        };
        let name = obj.get("name").and_then(|v| match v {
            Value::Null => None,
            Value::String(s) => Some(s.clone()),
            other => Some(other.to_string()),
        });
        rows.push(InvItemName { name });
    }
    let count = food_policy::food_count(Some(data.as_ref()), &rows, food_name);
    helper_ok(json!(count))
}

fn food_heal_v2(input: &Value) -> Value {
    let Some(food_name) = input.get("foodName").and_then(Value::as_str) else {
        return helper_err("invalid-args");
    };
    match food_policy::food_heal_amount(selected_data().as_deref(), food_name) {
        FoodHealOutcome::Ok(heal) => helper_ok(json!(heal)),
        FoodHealOutcome::MissingSelected => helper_err("missing-selected-data"),
        FoodHealOutcome::UnknownFood => helper_err("unknown-food"),
    }
}

fn combat_keep_names_v2(input: &Value) -> Value {
    let Some(data) = selected_data() else {
        return helper_err("missing-selected-data");
    };
    let Some(obj) = input.as_object() else {
        return helper_err("invalid-args");
    };
    if !obj.get("food").is_some_and(|v| v.is_string()) {
        return helper_err("invalid-args");
    }
    let read_string = |field: &str| -> Result<String, Value> {
        match obj.get(field) {
            None | Some(Value::Null) => Ok(String::new()),
            Some(Value::String(s)) => Ok(s.clone()),
            Some(_) => Err(helper_err("invalid-args")),
        }
    };
    let extra = match obj.get("extra") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(rows)) => {
            let mut out = Vec::with_capacity(rows.len());
            for row in rows {
                let Some(s) = row.as_str() else {
                    return helper_err("invalid-args");
                };
                out.push(s.to_string());
            }
            out
        }
        Some(_) => return helper_err("invalid-args"),
    };
    let options = match (
        read_string("food"),
        read_string("style"),
        read_string("spell"),
        read_string("ammo"),
        read_string("weapon"),
    ) {
        (Ok(food), Ok(style), Ok(spell), Ok(ammo), Ok(weapon)) => CombatKeepOptions {
            food,
            style,
            spell,
            ammo,
            weapon,
            extra,
        },
        (Err(v), _, _, _, _) | (_, Err(v), _, _, _) | (_, _, Err(v), _, _) | (_, _, _, Err(v), _)
        | (_, _, _, _, Err(v)) => return v,
    };
    helper_ok(json!(keep_list::combat_keep_names(
        data.as_ref(),
        &options
    )))
}

fn runes_per_cast_v2(input: &Value) -> Value {
    let Some(data) = selected_data() else {
        return helper_err("missing-selected-data");
    };
    let Some(spell_name) = input.get("spellName").and_then(Value::as_str) else {
        return helper_err("invalid-args");
    };
    let Some(wielded) = input.get("wielded").and_then(Value::as_array) else {
        return helper_err("invalid-args");
    };
    let mut wielded_names = Vec::with_capacity(wielded.len());
    for row in wielded {
        let Some(s) = row.as_str() else {
            return helper_err("invalid-args");
        };
        wielded_names.push(s.to_string());
    }
    match data.as_ref().runes_per_cast(spell_name, &wielded_names) {
        None => helper_ok(Value::Null),
        Some(costs) => helper_ok(json!(costs
            .into_iter()
            .map(|cost| json!({ "rune": cost.rune, "count": cost.count }))
            .collect::<Vec<_>>())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use client::io::ClientRevision;

    #[test]
    fn dispatch_food_count_empty_inv() {
        configure(api::game_data::for_revision(ClientRevision::R274).ok().map(Arc::from));
        let out = dispatch(&[json!({
            "op": "foodCount",
            "input": { "items": [], "foodName": "Shark" }
        })]);
        assert_eq!(out["ok"], true);
        assert_eq!(out["value"], 0);
    }
}

fn escape_runes_v2(input: &Value) -> Value {
    let Some(id) = input.get("id").and_then(Value::as_str) else {
        return helper_err("invalid-args");
    };
    match escape_runes::escape_runes_for_optional(selected_data().as_deref(), id) {
        Ok(fact) => helper_ok(json!({
            "runes": fact.runes.iter().map(|r| json!({ "rune": r.rune, "count": r.count })).collect::<Vec<_>>(),
            "level": fact.level,
            "label": fact.label,
        })),
        Err(EscapeRunesError::MissingSelected) => helper_err("missing-selected-data"),
        Err(EscapeRunesError::UnknownId) => helper_err("unknown-id"),
    }
}
