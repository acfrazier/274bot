//! Native gather-tool selection over the posted `api::gather_tools` rows.
//!
//! Pickaxe use is gated by the mining `levelrequire` row; axes have no
//! woodcutting use gate. Bronze is the tutorial wield path with no Attack
//! gate and unknown names cannot wield. The identity/use/wield rows stay in
//! `api::gather_tools` as posted facts — selection policy lives here, not in
//! the JS shim.
//!
//! [`dispatch`] backs the `__rs2b0t_tool_step` binding. The shim keeps the
//! synchronous exports (`bestAxe`, `bestPickaxe`, `canWieldTool`, `hasAllTools`,
//! `hasToolReq`, `toolRestockPlan`) but no longer owns any selection policy:
//! it starts a decision, asks this module what to do next, runs at most one
//! original field access / JS coercion / caller callback at that point, and
//! feeds the bounded answer back. Every candidate order, use/wield gate,
//! short circuit and thrown feature error is decided here. Callbacks and
//! object getters are never precomputed: a step only ever names the one
//! candidate, requirement field, or probe the existing early-exit selector
//! would reach.
//!
//! JS `Number()`, relational `>=`, `??`, `|| 0` and `Math.min` stay in the
//! shim at the original reached expression. Native never parses those values
//! and never receives an entire `reqs` / row object graph through serde_v8.

use api::gather_tools::{GatherTool, AXES, PICKAXES};
use serde_json::{json, Value};

fn meets_use(tool: &GatherTool, level: i32) -> bool {
    match (tool.use_skill, tool.use_level) {
        (Some("mining"), Some(need)) => level >= need,
        _ => true,
    }
}

/// First best-first pickaxe whose mining use level is met and `available`.
pub fn best_pickaxe(level: i32, mut available: impl FnMut(&str) -> bool) -> Option<&'static str> {
    PICKAXES
        .iter()
        .find_map(|tool| (meets_use(tool, level) && available(tool.name)).then_some(tool.name))
}

/// First best-first axe that `available` accepts. `level` is unused (no WC gate).
pub fn best_axe(_level: i32, mut available: impl FnMut(&str) -> bool) -> Option<&'static str> {
    AXES.iter()
        .find_map(|tool| available(tool.name).then_some(tool.name))
}

fn named(name: &str) -> Option<&'static GatherTool> {
    let name = name.trim();
    AXES.iter().chain(PICKAXES).find(|tool| tool.name == name)
}

/// Unknown names are unavailable. Bronze tutorial path has no Attack gate.
pub fn can_wield_tool(name: &str, attack: i32) -> bool {
    match named(name) {
        None => false,
        Some(tool) => match tool.wield_attack {
            None => true,
            Some(need) => attack >= need,
        },
    }
}

// --- step helpers --------------------------------------------------------------

fn i64_field(payload: &Value, key: &str, default: i64) -> i64 {
    payload.get(key).and_then(Value::as_i64).unwrap_or(default)
}

fn bool_field(payload: &Value, key: &str) -> Option<bool> {
    payload.get(key).and_then(Value::as_bool)
}

fn has_key(payload: &Value, key: &str) -> bool {
    bool_field(payload, key).unwrap_or(false)
}

fn row_index(payload: &Value) -> Option<i64> {
    payload.get("row_index").and_then(Value::as_i64)
}

fn row_name(payload: &Value) -> Option<&str> {
    payload
        .get("row")
        .and_then(|row| row.get("name"))
        .and_then(Value::as_str)
}

fn row_mining(payload: &Value) -> bool {
    payload
        .get("row")
        .and_then(|row| row.get("use_skill"))
        .and_then(Value::as_str)
        == Some("mining")
}

fn names(payload: &Value) -> &[Value] {
    payload
        .get("names")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn probe(index: i64, what: &str, name: Value) -> Value {
    json!({"kind": "probe", "index": index, "what": what, "name": name})
}

fn done(value: Value) -> Value {
    json!({"kind": "done", "value": value})
}

fn read(index: i64, what: &str) -> Value {
    json!({"kind": "read", "index": index, "what": what})
}

fn need_row(index: i64) -> Value {
    json!({"kind": "need_row", "index": index})
}

fn need_level(index: i64) -> Value {
    json!({"kind": "need_level", "index": index})
}

fn skip(index: i64) -> Value {
    json!({"kind": "skip", "index": index})
}

fn need_wield(index: i64) -> Value {
    json!({"kind": "need_wield", "index": index})
}

fn need_attack() -> Value {
    json!({"kind": "need_attack"})
}

fn emit(index: i64, name: &str) -> Value {
    json!({"kind": "emit", "index": index, "name": name})
}

/// An unsupported case stays an explicit `notImpl` error in the shim.
fn feature_error(feature: &str) -> Value {
    json!({"kind": "error", "feature": feature})
}

// --- step dispatch --------------------------------------------------------------

/// One step of a tool decision (`__rs2b0t_tool_step`).
///
/// The shim passes the current index plus only the bounded answer to the
/// previous read, gate, or callback. The reply is the next read, probe,
/// skip, emit, final value, or an explicit feature error. Stateless by
/// construction, so a callback that re-enters a tool decision cannot disturb
/// the decision in flight. Candidate indices are the original snapshot order,
/// never a rebuilt filtered list.
pub fn dispatch(payload: &Value) -> Value {
    match payload.get("op").and_then(Value::as_str) {
        Some("best") => best_step(payload),
        Some("can_wield") => can_wield_step(payload),
        Some("has_all") => has_all_step(payload),
        Some("has_req") => has_req_step(payload),
        Some("restock") => restock_step(payload),
        _ => feature_error("Tools"),
    }
}

/// `bestAxe` / `bestPickaxe`: walk the shim's `tools(kind)` snapshot in order.
/// Mining rows ask for a fresh JS `Number(level) >= (use_level ?? 0)` at each
/// reached gate; axes never request that conversion.
fn best_step(payload: &Value) -> Value {
    let index = i64_field(payload, "index", -1);
    let accepted = bool_field(payload, "accepted").unwrap_or(false);
    if accepted {
        return match row_name(payload) {
            Some(name) => json!({"kind": "done", "name": name}),
            None => json!({"kind": "none"}),
        };
    }
    let next = index + 1;
    let has_row = row_index(payload) == Some(next) && payload.get("row").is_some();
    if !has_row {
        if bool_field(payload, "has_next").unwrap_or(false) {
            return need_row(next);
        }
        return json!({"kind": "none"});
    }
    if row_mining(payload) {
        match bool_field(payload, "level_ok") {
            None => return need_level(next),
            Some(false) => return skip(next),
            Some(true) => {}
        }
    }
    match row_name(payload) {
        Some(name) => json!({"kind": "probe", "index": next, "name": name}),
        None => skip(next),
    }
}

/// `canWieldTool`: exact-name lookup over the shim snapshot, then Attack
/// conversion only for a found non-bronze row.
fn can_wield_step(payload: &Value) -> Value {
    let want = payload.get("name").and_then(Value::as_str).unwrap_or("");
    if want.is_empty() {
        return json!({"kind": "value", "value": false});
    }
    let found = names(payload).iter().position(|n| n.as_str() == Some(want));
    let Some(found) = found else {
        return json!({"kind": "value", "value": false});
    };
    let index = found as i64;
    match payload.get("wield") {
        None => need_wield(index),
        Some(Value::Null) => json!({"kind": "value", "value": true}),
        Some(_) => match bool_field(payload, "attack_ok") {
            None => need_attack(),
            Some(ok) => json!({"kind": "value", "value": ok}),
        },
    }
}

/// `hasAllTools`: `.every` over the requirements, in order, with the existing
/// tiered error and short circuit. Native never sees the `reqs` array; it
/// asks for one field of the current index, then one callback. A sparse hole
/// advances; an explicit falsy slot fails. Name truthiness is a JS boolean so
/// the live `r.name` is not serialized; the shim re-reads it at the probe.
fn has_all_step(payload: &Value) -> Value {
    let mode = payload
        .get("mode")
        .and_then(Value::as_str)
        .unwrap_or("skill");
    let inventory = mode == "inventory";
    let skill_fn = bool_field(payload, "skill_fn").unwrap_or(false);
    let index = i64_field(payload, "index", 0).max(0);
    let req_len = i64_field(payload, "req_len", 0).max(0);
    if index >= req_len {
        return done(Value::Bool(true));
    }
    if bool_field(payload, "hole").unwrap_or(false) {
        return json!({"kind": "advance", "index": index + 1});
    }
    let Some(present) = bool_field(payload, "present") else {
        return read(index, "present");
    };
    if !present {
        return done(Value::Bool(false));
    }
    if inventory {
        if !has_key(payload, "has_kind") {
            return read(index, "kind");
        }
        if payload.get("kind").and_then(Value::as_str) == Some("tiered") {
            return feature_error("Tools.hasAllTools");
        }
    }
    if !has_key(payload, "has_name") {
        return read(index, "name");
    }
    if !bool_field(payload, "name_ok").unwrap_or(false) {
        return done(Value::Bool(false));
    }
    if !inventory && !skill_fn {
        return done(Value::Bool(false));
    }
    match bool_field(payload, "ok") {
        None => {
            json!({"kind": "probe", "index": index, "what": if inventory { "inv" } else { "skill" }})
        }
        Some(false) => done(Value::Bool(false)),
        Some(true) => json!({"kind": "advance", "index": index + 1}),
    }
}

/// `hasToolReq(available, req)`: JS-truthiness of `req && req.name && available(req.name)`.
/// Native decides the short-circuit; the shim re-reads `req.name` at the probe.
fn has_req_step(payload: &Value) -> Value {
    let Some(present) = bool_field(payload, "present") else {
        return read(-1, "present");
    };
    if !present {
        return done(Value::Bool(false));
    }
    if !has_key(payload, "has_name") {
        return read(-1, "name");
    }
    if !bool_field(payload, "name_ok").unwrap_or(false) {
        return done(Value::Bool(false));
    }
    match bool_field(payload, "ok") {
        Some(ok) => done(Value::Bool(ok)),
        None => json!({"kind": "probe", "index": -1, "what": "available"}),
    }
}

/// `toolRestockPlan`: tinderbox-only plan, same validation error and callback
/// order (`invCount` then `bankCount`). Arithmetic and `Number()` stay in JS.
fn restock_step(payload: &Value) -> Value {
    let index = i64_field(payload, "index", 0).max(0);
    let req_len = i64_field(payload, "req_len", 0).max(0);
    if index >= req_len {
        return json!({"kind": "done"});
    }
    let Some(present) = bool_field(payload, "present") else {
        return read(index, "present");
    };
    if !present {
        return feature_error("Tools.toolRestockPlan");
    }
    if !has_key(payload, "has_kind") {
        return read(index, "kind");
    }
    if payload.get("kind").and_then(Value::as_str) == Some("tiered") {
        return feature_error("Tools.toolRestockPlan");
    }
    if !has_key(payload, "has_name") {
        return read(index, "name");
    }
    let Some(name) = payload.get("name").and_then(Value::as_str) else {
        return feature_error("Tools.toolRestockPlan");
    };
    if name.to_lowercase() != "tinderbox" {
        return feature_error("Tools.toolRestockPlan");
    }
    match bool_field(payload, "need_le_0") {
        None => return probe(index, "inv", Value::String(name.to_string())),
        Some(true) => return json!({"kind": "advance", "index": index + 1}),
        Some(false) => {}
    }
    match bool_field(payload, "avail_le_0") {
        None => probe(index, "bank", Value::String(name.to_string())),
        Some(true) => json!({"kind": "advance", "index": index + 1}),
        Some(false) => emit(index, name),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn best_pickaxe_uses_mining_level_and_available_only() {
        let steel = |name: &str| name == "Steel pickaxe";
        assert_eq!(best_pickaxe(30, steel), Some("Steel pickaxe"));
        assert_eq!(best_pickaxe(5, steel), None);
        assert_eq!(
            best_pickaxe(1, |name| name == "Bronze pickaxe"
                || name == "Steel pickaxe"),
            Some("Bronze pickaxe")
        );
        assert_eq!(best_pickaxe(30, |_| false), None);
        let mut skipped = Vec::new();
        assert_eq!(
            best_pickaxe(30, |name| {
                skipped.push(name.to_string());
                false
            }),
            None
        );
        assert_eq!(
            skipped,
            [
                "Mithril pickaxe",
                "Steel pickaxe",
                "Iron pickaxe",
                "Bronze pickaxe",
            ]
        );
        let mut calls = Vec::new();
        let hit = best_pickaxe(99, |name| {
            calls.push(name.to_string());
            name == "Steel pickaxe"
        });
        assert_eq!(hit, Some("Steel pickaxe"));
        assert_eq!(
            calls,
            [
                "Rune pickaxe",
                "Adamant pickaxe",
                "Mithril pickaxe",
                "Steel pickaxe",
            ]
        );
    }

    #[test]
    fn best_axe_has_no_woodcutting_gate_and_keeps_black() {
        assert_eq!(
            best_axe(1, |name| name == "Rune axe" || name == "Steel axe"),
            Some("Rune axe")
        );
        let mut calls = Vec::new();
        let hit = best_axe(1, |name| {
            calls.push(name.to_string());
            name == "Steel axe"
        });
        assert_eq!(hit, Some("Steel axe"));
        assert_eq!(
            calls,
            [
                "Rune axe",
                "Adamant axe",
                "Mithril axe",
                "Black axe",
                "Steel axe",
            ]
        );
        assert_eq!(best_axe(99, |name| name == "Bank-only mithril"), None);
    }

    #[test]
    fn can_wield_is_attack_gate_and_unknown_is_unavailable() {
        assert!(!can_wield_tool("Steel pickaxe", 1));
        assert!(can_wield_tool("Steel pickaxe", 5));
        assert!(!can_wield_tool("Steel axe", 1));
        assert!(can_wield_tool("Steel axe", 5));
        assert!(can_wield_tool("Bronze pickaxe", 0));
        assert!(can_wield_tool("Bronze axe", 0));
        assert!(can_wield_tool("Black axe", 10));
        assert!(!can_wield_tool("Black axe", 9));
        assert!(!can_wield_tool("Dragon pickaxe", 99));
        assert!(!can_wield_tool("", 99));
        assert!(!can_wield_tool("steel pickaxe", 99));
    }

    fn best_start(has_next: bool) -> Value {
        dispatch(&json!({"op": "best", "index": -1, "accepted": false, "has_next": has_next}))
    }

    #[test]
    fn step_owns_candidate_order_gates_and_acceptance() {
        assert_eq!(best_start(true), json!({"kind": "need_row", "index": 0}));
        assert_eq!(
            dispatch(&json!({
                "op": "best", "index": -1, "accepted": false, "has_next": true,
                "row_index": 0, "row": {"name": "Mithril pickaxe", "use_skill": "mining"},
            })),
            json!({"kind": "need_level", "index": 0}),
            "mining rows request a fresh level conversion"
        );
        assert_eq!(
            dispatch(&json!({
                "op": "best", "index": -1, "accepted": false, "has_next": true,
                "row_index": 0, "row": {"name": "Mithril pickaxe", "use_skill": "mining"},
                "level_ok": true,
            })),
            json!({"kind": "probe", "index": 0, "name": "Mithril pickaxe"})
        );
        assert_eq!(
            dispatch(&json!({
                "op": "best", "index": -1, "accepted": false, "has_next": true,
                "row_index": 0, "row": {"name": "Rune pickaxe", "use_skill": "mining"},
                "level_ok": false,
            })),
            json!({"kind": "skip", "index": 0}),
            "a failed mining gate does not probe and does not keep the conversion"
        );
        assert_eq!(
            dispatch(&json!({
                "op": "best", "index": 0, "accepted": true,
                "row_index": 0, "row": {"name": "Steel pickaxe", "use_skill": "mining"},
            })),
            json!({"kind": "done", "name": "Steel pickaxe"})
        );
        assert_eq!(
            dispatch(&json!({
                "op": "best", "index": 0, "accepted": false, "has_next": false,
            })),
            json!({"kind": "none"})
        );
        assert_eq!(
            dispatch(&json!({
                "op": "best", "index": -1, "accepted": false, "has_next": true,
                "row_index": 0, "row": {"name": "Rune axe", "use_skill": null},
            })),
            json!({"kind": "probe", "index": 0, "name": "Rune axe"}),
            "axes never request Number(level)"
        );
        assert_eq!(
            dispatch(&json!({"op": "best", "index": -1, "accepted": true, "has_next": true})),
            json!({"kind": "none"}),
            "an accepted answer with no probed candidate is not a hit"
        );
        assert_eq!(best_start(false), json!({"kind": "none"}));
    }

    #[test]
    fn can_wield_step_looks_up_then_asks_for_attack_only_when_needed() {
        let names = json!(["Steel pickaxe", "Bronze axe", "Steel axe"]);
        let wield = |name: &str, extra: Value| {
            let mut payload = json!({"op": "can_wield", "name": name, "names": names});
            if let Value::Object(extra) = extra {
                if let Value::Object(obj) = &mut payload {
                    obj.extend(extra);
                }
            }
            dispatch(&payload)
        };
        assert_eq!(
            wield("Dragon pickaxe", json!({})),
            json!({"kind": "value", "value": false}),
            "unknown names never request Attack"
        );
        assert_eq!(
            wield("", json!({})),
            json!({"kind": "value", "value": false})
        );
        assert_eq!(
            wield("Bronze axe", json!({})),
            json!({"kind": "need_wield", "index": 1})
        );
        assert_eq!(
            wield("Bronze axe", json!({"wield": Value::Null})),
            json!({"kind": "value", "value": true}),
            "bronze never requests Attack"
        );
        assert_eq!(
            wield("Steel axe", json!({"wield": 5})),
            json!({"kind": "need_attack"})
        );
        assert_eq!(
            wield("Steel axe", json!({"wield": 5, "attack_ok": true})),
            json!({"kind": "value", "value": true})
        );
        assert_eq!(
            wield("Steel axe", json!({"wield": 5, "attack_ok": false})),
            json!({"kind": "value", "value": false})
        );
        assert_eq!(
            wield("steel pickaxe", json!({})),
            json!({"kind": "value", "value": false})
        );
    }

    #[test]
    fn has_all_steps_probe_in_order_and_short_circuit() {
        assert_eq!(
            dispatch(&json!({"op": "has_all", "mode": "inventory", "index": 0, "req_len": 2})),
            json!({"kind": "read", "index": 0, "what": "present"})
        );
        assert_eq!(
            dispatch(&json!({
                "op": "has_all", "mode": "inventory", "index": 0, "req_len": 2,
                "present": true,
            })),
            json!({"kind": "read", "index": 0, "what": "kind"})
        );
        assert_eq!(
            dispatch(&json!({
                "op": "has_all", "mode": "inventory", "index": 0, "req_len": 2,
                "present": true, "has_kind": true, "kind": "tiered",
            })),
            json!({"kind": "error", "feature": "Tools.hasAllTools"})
        );
        assert_eq!(
            dispatch(&json!({
                "op": "has_all", "mode": "inventory", "index": 0, "req_len": 2,
                "present": true, "has_kind": true, "kind": "item",
            })),
            json!({"kind": "read", "index": 0, "what": "name"})
        );
        assert_eq!(
            dispatch(&json!({
                "op": "has_all", "mode": "inventory", "index": 0, "req_len": 2,
                "present": true, "has_kind": true, "has_name": true, "name_ok": true,
            })),
            json!({"kind": "probe", "index": 0, "what": "inv"})
        );
        assert_eq!(
            dispatch(&json!({
                "op": "has_all", "mode": "inventory", "index": 0, "req_len": 2,
                "present": true, "has_kind": true, "has_name": true, "name_ok": true,
                "ok": false,
            })),
            json!({"kind": "done", "value": false}),
            "an unmet requirement never advances"
        );
        assert_eq!(
            dispatch(&json!({
                "op": "has_all", "mode": "inventory", "index": 0, "req_len": 2,
                "present": true, "has_kind": true, "has_name": true, "name_ok": true,
                "ok": true,
            })),
            json!({"kind": "advance", "index": 1})
        );
        assert_eq!(
            dispatch(&json!({
                "op": "has_all", "mode": "inventory", "index": 0, "req_len": 2,
                "present": false,
            })),
            json!({"kind": "done", "value": false})
        );
        assert_eq!(
            dispatch(&json!({
                "op": "has_all", "mode": "inventory", "index": 0, "req_len": 2,
                "hole": true,
            })),
            json!({"kind": "advance", "index": 1}),
            "a sparse hole advances without failing"
        );
        assert_eq!(
            dispatch(&json!({
                "op": "has_all", "mode": "inventory", "index": 0, "req_len": 2,
                "present": true, "has_kind": true, "has_name": true, "name_ok": false,
            })),
            json!({"kind": "done", "value": false})
        );
        assert_eq!(
            dispatch(&json!({"op": "has_all", "mode": "inventory", "index": 2, "req_len": 2})),
            json!({"kind": "done", "value": true})
        );
        assert_eq!(
            dispatch(&json!({"op": "has_all", "mode": "inventory", "index": 0, "req_len": 0})),
            json!({"kind": "done", "value": true})
        );
    }

    #[test]
    fn has_all_skill_mode_needs_a_skill_callback_and_truthy_results() {
        let base = json!({
            "op": "has_all", "mode": "skill", "index": 0, "req_len": 1,
            "present": true, "has_name": true, "name_ok": true,
        });
        assert_eq!(
            dispatch(&json!({
                "op": "has_all", "mode": "skill", "index": 0, "req_len": 1,
                "present": true, "has_name": true, "name_ok": true,
                "skill_fn": false,
            })),
            json!({"kind": "done", "value": false})
        );
        let mut probe_payload = base.clone();
        probe_payload["skill_fn"] = json!(true);
        assert_eq!(
            dispatch(&probe_payload),
            json!({"kind": "probe", "index": 0, "what": "skill"})
        );
        let mut ok = base.clone();
        ok["skill_fn"] = json!(true);
        ok["ok"] = json!(true);
        assert_eq!(dispatch(&ok), json!({"kind": "advance", "index": 1}));
        let mut no = base;
        no["skill_fn"] = json!(true);
        no["ok"] = json!(false);
        assert_eq!(dispatch(&no), json!({"kind": "done", "value": false}));
    }

    #[test]
    fn has_req_only_probes_a_truthy_name_and_keeps_js_truthiness() {
        assert_eq!(
            dispatch(&json!({"op": "has_req"})),
            json!({"kind": "read", "index": -1, "what": "present"})
        );
        assert_eq!(
            dispatch(&json!({"op": "has_req", "present": false})),
            json!({"kind": "done", "value": false})
        );
        assert_eq!(
            dispatch(&json!({"op": "has_req", "present": true})),
            json!({"kind": "read", "index": -1, "what": "name"})
        );
        assert_eq!(
            dispatch(
                &json!({"op": "has_req", "present": true, "has_name": true, "name_ok": false})
            ),
            json!({"kind": "done", "value": false})
        );
        assert_eq!(
            dispatch(&json!({
                "op": "has_req", "present": true, "has_name": true, "name_ok": true,
            })),
            json!({"kind": "probe", "index": -1, "what": "available"})
        );
        assert_eq!(
            dispatch(&json!({
                "op": "has_req", "present": true, "has_name": true, "name_ok": true,
                "ok": true,
            })),
            json!({"kind": "done", "value": true})
        );
        assert_eq!(
            dispatch(&json!({
                "op": "has_req", "present": true, "has_name": true, "name_ok": true,
                "ok": false,
            })),
            json!({"kind": "done", "value": false})
        );
    }

    #[test]
    fn restock_steps_read_then_probe_inv_then_bank() {
        assert_eq!(
            dispatch(&json!({"op": "restock", "index": 0, "req_len": 1})),
            json!({"kind": "read", "index": 0, "what": "present"})
        );
        assert_eq!(
            dispatch(&json!({
                "op": "restock", "index": 0, "req_len": 1, "present": false,
            })),
            json!({"kind": "error", "feature": "Tools.toolRestockPlan"})
        );
        let named = json!({
            "op": "restock", "index": 0, "req_len": 1,
            "present": true, "has_kind": true, "has_name": true, "name": "Tinderbox",
        });
        assert_eq!(
            dispatch(&named),
            json!({"kind": "probe", "index": 0, "what": "inv", "name": "Tinderbox"})
        );
        let mut packed = named.clone();
        packed["need_le_0"] = json!(true);
        assert_eq!(
            dispatch(&packed),
            json!({"kind": "advance", "index": 1}),
            "a stocked pack never asks the bank"
        );
        let mut bank = named.clone();
        bank["need_le_0"] = json!(false);
        assert_eq!(
            dispatch(&bank),
            json!({"kind": "probe", "index": 0, "what": "bank", "name": "Tinderbox"})
        );
        let mut empty_bank = bank.clone();
        empty_bank["avail_le_0"] = json!(true);
        assert_eq!(
            dispatch(&empty_bank),
            json!({"kind": "advance", "index": 1})
        );
        let mut emit = bank;
        emit["avail_le_0"] = json!(false);
        assert_eq!(
            dispatch(&emit),
            json!({"kind": "emit", "index": 0, "name": "Tinderbox"})
        );
        assert_eq!(
            dispatch(&json!({
                "op": "restock", "index": 0, "req_len": 1,
                "present": true, "has_kind": true, "has_name": true, "name": "Hammer",
            })),
            json!({"kind": "error", "feature": "Tools.toolRestockPlan"})
        );
        assert_eq!(
            dispatch(&json!({
                "op": "restock", "index": 0, "req_len": 1,
                "present": true, "has_kind": true, "kind": "tiered",
            })),
            json!({"kind": "error", "feature": "Tools.toolRestockPlan"})
        );
        assert_eq!(
            dispatch(&json!({"op": "restock", "index": 1, "req_len": 1})),
            json!({"kind": "done"})
        );
        assert_eq!(
            dispatch(&json!({"op": "restock", "index": 0, "req_len": 0})),
            json!({"kind": "done"})
        );
    }

    #[test]
    fn unknown_ops_are_an_explicit_feature_error() {
        assert_eq!(
            dispatch(&json!({"op": "nope"})),
            json!({"kind": "error", "feature": "Tools"})
        );
        assert_eq!(
            dispatch(&Value::Null),
            json!({"kind": "error", "feature": "Tools"})
        );
    }
}
