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
//! caller callback with the answer, and feeds that answer back. Every
//! candidate order, use/wield gate, comparison, short circuit and thrown
//! feature error is decided here. Callbacks are never precomputed: a step
//! only ever names the one tool the existing early-exit selector would reach.
//!
//! Callback answers the shim marshals back keep their JS meaning: counts and
//! levels arrive as numbers (or the canonical `"NaN"` / `"±Infinity"` tokens
//! the shim substitutes for non-finite numbers), truthiness answers arrive as
//! booleans, because `bestFrom` compares `=== true` while `hasAllTools` and
//! `hasToolReq` use JS truthiness.

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

// --- posted-row decode -------------------------------------------------------

/// One row of the posted `content.gather_tools` table, as the shim sees it.
struct PostedRow<'a> {
    name: &'a str,
    use_skill: Option<&'a str>,
    use_level: Option<&'a Value>,
    wield_attack: Option<&'a Value>,
}

impl PostedRow<'_> {
    /// JS `tool.use_skill !== 'mining' || Number(level) >= (tool.use_level ?? 0)`.
    fn usable_at(&self, level: f64) -> bool {
        if self.use_skill != Some("mining") {
            return true;
        }
        level >= self.use_level.map(num).unwrap_or(0.0)
    }

    /// JS `tool.wield_attack == null || Number(attack) >= tool.wield_attack`.
    fn wieldable_at(&self, attack: f64) -> bool {
        match self.wield_attack {
            None => true,
            Some(need) => attack >= num(need),
        }
    }
}

fn posted_rows<'a>(facts: Option<&'a Value>, kind: &str) -> Vec<PostedRow<'a>> {
    facts
        .and_then(|facts| facts.get(kind))
        .and_then(Value::as_array)
        .map(|rows| rows.iter().filter_map(posted_row).collect())
        .unwrap_or_default()
}

/// The shim's row filter: an object with a non-blank string name and numeric id.
fn posted_row(row: &Value) -> Option<PostedRow<'_>> {
    let name = row.get("name")?.as_str()?;
    if name.trim().is_empty() {
        return None;
    }
    if !row.get("id").is_some_and(Value::is_number) {
        return None;
    }
    Some(PostedRow {
        name,
        use_skill: row.get("use_skill").and_then(Value::as_str),
        use_level: row.get("use_level").filter(|value| !value.is_null()),
        wield_attack: row.get("wield_attack").filter(|value| !value.is_null()),
    })
}

// --- JS value semantics the shim used to apply ---------------------------------

/// JS `Number(value)` for the JSON values the shim can send: missing is `NaN`
/// (`Number(undefined)`), `null` is 0, booleans are 1/0, numbers pass through,
/// and strings (including the canonical `"NaN"` / `"Infinity"` tokens) parse as
/// decimal JS numbers. Anything else is `NaN`.
fn num(value: &Value) -> f64 {
    match value {
        Value::Null => 0.0,
        Value::Bool(true) => 1.0,
        Value::Bool(false) => 0.0,
        Value::Number(number) => number.as_f64().unwrap_or(f64::NAN),
        Value::String(text) => number_from_str(text),
        Value::Array(_) | Value::Object(_) => f64::NAN,
    }
}

fn num_opt(value: Option<&Value>) -> f64 {
    value.map(num).unwrap_or(f64::NAN)
}

fn number_from_str(raw: &str) -> f64 {
    let text = raw.trim();
    if text.is_empty() {
        return 0.0;
    }
    let (sign, digits) = match text.strip_prefix('-') {
        Some(rest) => (-1.0, rest),
        None => (1.0, text.strip_prefix('+').unwrap_or(text)),
    };
    match digits.to_ascii_lowercase().as_str() {
        "infinity" => return sign * f64::INFINITY,
        "nan" => return f64::NAN,
        _ => {}
    }
    let decimal = !digits.is_empty()
        && digits
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'.' | b'e' | b'E' | b'+' | b'-'))
        && digits.bytes().any(|byte| byte.is_ascii_digit());
    if !decimal {
        return f64::NAN;
    }
    digits
        .parse::<f64>()
        .map(|value| sign * value)
        .unwrap_or(f64::NAN)
}

/// JS truthiness for JSON values (missing values are falsy).
fn truthy(value: &Value) -> bool {
    match value {
        Value::Null | Value::Bool(false) => false,
        Value::Bool(true) => true,
        Value::Number(number) => number.as_f64().is_some_and(|value| value != 0.0),
        Value::String(text) => !text.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

fn truthy_opt(value: Option<&Value>) -> bool {
    value.is_some_and(truthy)
}

/// JS `Number(value) || 0`.
fn zeroed(value: f64) -> f64 {
    if value == 0.0 || value.is_nan() {
        0.0
    } else {
        value
    }
}

/// JS `Math.min` (NaN propagates, unlike Rust's `f64::min`).
fn js_min(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else {
        a.min(b)
    }
}

fn probe(index: i64, what: &str, name: Value) -> Value {
    json!({"kind": "probe", "index": index, "what": what, "name": name})
}

fn done(value: Value) -> Value {
    json!({"kind": "done", "value": value})
}

/// An unsupported case stays an explicit `notImpl` error in the shim.
fn feature_error(feature: &str) -> Value {
    json!({"kind": "error", "feature": feature})
}

// --- step dispatch --------------------------------------------------------------

/// One step of a tool decision (`__rs2b0t_tool_step`).
///
/// The shim passes the decision inputs plus the answer to the previous probe;
/// the reply is the next probe, a final value, or an explicit feature error.
/// Stateless by construction, so a callback that re-enters a tool decision
/// cannot disturb the decision in flight.
pub fn dispatch(payload: &Value) -> Value {
    match payload.get("op").and_then(Value::as_str) {
        Some("best") => best_step(payload),
        Some("can_wield") => can_wield_value(payload),
        Some("has_all") => has_all_step(payload),
        Some("has_req") => has_req_step(payload),
        Some("restock") => restock_step(payload),
        _ => feature_error("Tools"),
    }
}

/// `bestAxe` / `bestPickaxe`: first best-first candidate whose use gate passes
/// and whose `available(name) === true`.
fn best_step(payload: &Value) -> Value {
    let kind = payload.get("kind").and_then(Value::as_str).unwrap_or("");
    let level = num_opt(payload.get("level"));
    let facts = payload.get("facts");
    let index = payload.get("index").and_then(Value::as_i64).unwrap_or(-1);
    let accepted = matches!(payload.get("accepted"), Some(Value::Bool(true)));
    let rows = posted_rows(facts, kind);
    let gated: Vec<&PostedRow> = rows.iter().filter(|row| row.usable_at(level)).collect();
    if accepted {
        return match usize::try_from(index).ok().and_then(|i| gated.get(i)) {
            Some(row) => json!({"kind": "done", "name": row.name}),
            None => json!({"kind": "none"}),
        };
    }
    let next = index + 1;
    match usize::try_from(next).ok().and_then(|i| gated.get(i)) {
        Some(row) => json!({"kind": "probe", "index": next, "name": row.name}),
        None => json!({"kind": "none"}),
    }
}

/// `canWieldTool`: exact-name lookup over axes then pickaxes, Attack gate.
fn can_wield_value(payload: &Value) -> Value {
    let facts = payload.get("facts");
    let want = payload
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    let attack = num_opt(payload.get("attack"));
    let wieldable = ["axes", "pickaxes"]
        .iter()
        .flat_map(|kind| posted_rows(facts, kind))
        .find(|row| row.name == want)
        .is_some_and(|row| row.wieldable_at(attack));
    json!({"kind": "value", "value": wieldable})
}

/// `hasAllTools`: `.every` over the requirements, in order, with the existing
/// tiered error and short circuit. `mode` picks the callback the shim owns:
/// `inventory` compares an inventory count against `min`, `skill` uses the
/// skill-level result's truthiness.
fn has_all_step(payload: &Value) -> Value {
    let mode = payload
        .get("mode")
        .and_then(Value::as_str)
        .unwrap_or("skill");
    let skill_fn = matches!(payload.get("skill_fn"), Some(Value::Bool(true)));
    let mut index = payload
        .get("index")
        .and_then(Value::as_i64)
        .unwrap_or(0)
        .max(0);
    let mut count = payload
        .get("count")
        .filter(|value| !value.is_null())
        .map(num);
    let mut ok = payload.get("ok").and_then(Value::as_bool);
    let Some(reqs) = payload.get("reqs").and_then(Value::as_array) else {
        return done(Value::Bool(false));
    };
    loop {
        let Some(req) = reqs.get(index as usize) else {
            return done(Value::Bool(true));
        };
        let name = req.get("name").filter(|value| truthy(value));
        if mode == "inventory" {
            if truthy(req) && req.get("kind").and_then(Value::as_str) == Some("tiered") {
                return feature_error("Tools.hasAllTools");
            }
            if !truthy(req) || name.is_none() {
                return done(Value::Bool(false));
            }
            let Some(count) = count else {
                return probe(index, "inv", name.cloned().unwrap_or(Value::Null));
            };
            let min = req
                .get("min")
                .filter(|value| !value.is_null())
                .map(num)
                .unwrap_or(1.0);
            if !(count >= min) {
                return done(Value::Bool(false));
            }
        } else {
            if !truthy(req) || name.is_none() {
                return done(Value::Bool(false));
            }
            if !skill_fn {
                return done(Value::Bool(false));
            }
            let Some(ok) = ok else {
                return probe(index, "skill", name.cloned().unwrap_or(Value::Null));
            };
            if !ok {
                return done(Value::Bool(false));
            }
        }
        index += 1;
        count = None;
        ok = None;
    }
}

/// `hasToolReq(available, req)`: JS-truthiness of `req && req.name && available(req.name)`.
fn has_req_step(payload: &Value) -> Value {
    let req = payload.get("req");
    let name = req
        .filter(|req| truthy(req))
        .and_then(|req| req.get("name"));
    if !truthy_opt(name) {
        return done(Value::Bool(false));
    }
    match payload.get("ok").and_then(Value::as_bool) {
        Some(ok) => done(Value::Bool(ok)),
        None => probe(-1, "available", name.cloned().unwrap_or(Value::Null)),
    }
}

/// `toolRestockPlan`: the tinderbox-only plan the shim implements today, with
/// the same per-requirement validation error and callback order (`invCount`
/// then `bankCount`, and only when the pack shortfall is positive).
fn restock_step(payload: &Value) -> Value {
    let mut index = payload
        .get("index")
        .and_then(Value::as_i64)
        .unwrap_or(0)
        .max(0);
    let mut count = payload
        .get("count")
        .filter(|value| !value.is_null())
        .map(num);
    let mut bank = payload
        .get("bank")
        .filter(|value| !value.is_null())
        .map(num);
    let Some(reqs) = payload.get("reqs").and_then(Value::as_array) else {
        return feature_error("Tools.toolRestockPlan");
    };
    loop {
        let Some(req) = reqs.get(index as usize) else {
            return json!({"kind": "done"});
        };
        if !truthy(req) || req.get("kind").and_then(Value::as_str) == Some("tiered") {
            return feature_error("Tools.toolRestockPlan");
        }
        let Some(name) = req.get("name").and_then(Value::as_str) else {
            return feature_error("Tools.toolRestockPlan");
        };
        if name.to_lowercase() != "tinderbox" {
            return feature_error("Tools.toolRestockPlan");
        }
        let Some(count_now) = count else {
            return probe(index, "inv", Value::String(name.to_string()));
        };
        let have = zeroed(count_now);
        let min = req
            .get("min")
            .filter(|value| !value.is_null())
            .map(num)
            .unwrap_or(1.0);
        let target = req
            .get("restock")
            .filter(|value| !value.is_null())
            .map(num)
            .unwrap_or(min);
        let need = target - have;
        if need <= 0.0 {
            index += 1;
            count = None;
            bank = None;
            continue;
        }
        let Some(bank_now) = bank else {
            return probe(index, "bank", Value::String(name.to_string()));
        };
        let available = zeroed(bank_now);
        if available <= 0.0 {
            index += 1;
            count = None;
            bank = None;
            continue;
        }
        return json!({
            "kind": "emit",
            "index": index + 1,
            "step": {
                "name": name,
                "qty": js_min(need, available),
                "equip": req.get("equip") == Some(&Value::Bool(true)),
            },
        });
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

    fn facts() -> Value {
        api::gather_tools::content_json_value()
    }

    fn best_call(kind: &str, level: Value, index: i64, accepted: bool, facts: &Value) -> Value {
        dispatch(&json!({
            "op": "best",
            "kind": kind,
            "level": level,
            "facts": facts,
            "index": index,
            "accepted": accepted,
        }))
    }

    #[test]
    fn step_owns_candidate_order_gates_and_acceptance() {
        let facts = facts();
        // Level 30: rune (41) and adamant (31) never reach a callback.
        assert_eq!(
            best_call("pickaxes", json!(30), -1, false, &facts),
            json!({"kind": "probe", "index": 0, "name": "Mithril pickaxe"})
        );
        assert_eq!(
            best_call("pickaxes", json!(30), 0, false, &facts),
            json!({"kind": "probe", "index": 1, "name": "Steel pickaxe"})
        );
        assert_eq!(
            best_call("pickaxes", json!(30), 1, true, &facts),
            json!({"kind": "done", "name": "Steel pickaxe"})
        );
        assert_eq!(
            best_call("pickaxes", json!(30), 3, false, &facts),
            json!({"kind": "none"})
        );
        // Axes have no use gate: level is not consulted at all.
        assert_eq!(
            best_call("axes", json!(1), -1, false, &facts),
            json!({"kind": "probe", "index": 0, "name": "Rune axe"})
        );
        assert_eq!(
            best_call("axes", json!("NaN"), -1, false, &facts),
            json!({"kind": "probe", "index": 0, "name": "Rune axe"})
        );
        assert_eq!(
            best_call("pickaxes", json!("NaN"), -1, false, &facts),
            json!({"kind": "none"})
        );
        // Number(null) is 0: bronze and iron mining rows stay reachable.
        assert_eq!(
            best_call("pickaxes", Value::Null, -1, false, &facts),
            json!({"kind": "probe", "index": 0, "name": "Iron pickaxe"})
        );
        // Accepted with no probed candidate is a shim-state error, not a hit.
        assert_eq!(
            best_call("axes", json!(1), -1, true, &facts),
            json!({"kind": "none"})
        );
    }

    #[test]
    fn step_fails_closed_without_posted_rows_and_survives_odd_rows() {
        for facts in [
            Value::Null,
            json!({}),
            json!({"axes": null}),
            json!({"axes": "nope"}),
            json!({"axes": []}),
            json!({"axes": [{"name": "  ", "id": 1}, {"name": "X"}]}),
            json!({"axes": [{"name": "X", "id": "1"}]}),
        ] {
            assert_eq!(
                best_call("axes", json!(99), -1, false, &facts),
                json!({"kind": "none"}),
                "{facts}"
            );
            assert_eq!(
                dispatch(
                    &json!({"op": "can_wield", "name": "Steel axe", "attack": 99, "facts": facts})
                ),
                json!({"kind": "value", "value": false}),
                "{facts}"
            );
        }
        let one = json!({"axes": [{"name": "Bronze axe", "id": 1351, "use_skill": null, "use_level": null, "wield_attack": null}]});
        assert_eq!(
            dispatch(
                &json!({"op": "can_wield", "name": "  Bronze axe  ", "attack": "NaN", "facts": one})
            ),
            json!({"kind": "value", "value": true}),
            "tutorial bronze ignores an unknown Attack"
        );
    }

    #[test]
    fn can_wield_step_reads_posted_rows_and_attack_gate() {
        let facts = facts();
        let wield = |name: &str, attack: Value| {
            dispatch(&json!({"op": "can_wield", "name": name, "attack": attack, "facts": facts}))
        };
        assert_eq!(
            wield("Steel pickaxe", json!(5)),
            json!({"kind": "value", "value": true})
        );
        assert_eq!(
            wield("Steel pickaxe", json!(4)),
            json!({"kind": "value", "value": false})
        );
        assert_eq!(
            wield("Steel axe", json!("5")),
            json!({"kind": "value", "value": true})
        );
        assert_eq!(
            wield("Black axe", json!(9)),
            json!({"kind": "value", "value": false})
        );
        assert_eq!(
            wield("Black axe", json!(10)),
            json!({"kind": "value", "value": true})
        );
        assert_eq!(
            wield("  Bronze pickaxe ", json!(0)),
            json!({"kind": "value", "value": true})
        );
        assert_eq!(
            wield("Dragon pickaxe", json!(99)),
            json!({"kind": "value", "value": false})
        );
        assert_eq!(
            wield("", json!(99)),
            json!({"kind": "value", "value": false})
        );
        assert_eq!(
            wield("steel pickaxe", json!(99)),
            json!({"kind": "value", "value": false})
        );
        assert_eq!(
            wield("Steel pickaxe", json!("NaN")),
            json!({"kind": "value", "value": false})
        );
        assert_eq!(
            wield("Steel pickaxe", json!("Infinity")),
            json!({"kind": "value", "value": true})
        );
    }

    #[test]
    fn has_all_steps_probe_in_order_and_short_circuit() {
        let reqs = json!([{"name": "Tinderbox", "min": 2}, {"name": "Hammer"}]);
        let step = |index: i64, count: Value| {
            dispatch(&json!({
                "op": "has_all", "mode": "inventory", "reqs": reqs, "index": index, "count": count,
            }))
        };
        assert_eq!(
            step(0, Value::Null),
            json!({"kind": "probe", "index": 0, "what": "inv", "name": "Tinderbox"})
        );
        // Shortfall keeps the caller from ever asking about the next requirement.
        assert_eq!(step(0, json!(1)), json!({"kind": "done", "value": false}));
        assert_eq!(
            step(0, json!(2)),
            json!({"kind": "probe", "index": 1, "what": "inv", "name": "Hammer"})
        );
        assert_eq!(step(1, json!(1)), json!({"kind": "done", "value": true}));
        assert_eq!(
            step(1, json!("NaN")),
            json!({"kind": "done", "value": false}),
            "a NaN count never satisfies a min"
        );
        assert_eq!(step(2, Value::Null), json!({"kind": "done", "value": true}));
        assert_eq!(
            step(0, json!(1)),
            json!({"kind": "done", "value": false}),
            "min defaults to 1"
        );
        let tiered = dispatch(&json!({
            "op": "has_all", "mode": "inventory",
            "reqs": [{"kind": "tiered", "skill": "mining"}], "index": 0,
        }));
        assert_eq!(
            tiered,
            json!({"kind": "error", "feature": "Tools.hasAllTools"})
        );
        let nameless = dispatch(&json!({
            "op": "has_all", "mode": "inventory", "reqs": [{"min": 3}], "index": 0,
        }));
        assert_eq!(nameless, json!({"kind": "done", "value": false}));
        let empty =
            dispatch(&json!({"op": "has_all", "mode": "inventory", "reqs": [], "index": 0}));
        assert_eq!(empty, json!({"kind": "done", "value": true}));
    }

    #[test]
    fn has_all_skill_mode_needs_a_skill_callback_and_truthy_results() {
        let reqs = json!([{"name": "Tinderbox"}]);
        let step = |skill_fn: bool, ok: Value| {
            dispatch(&json!({
                "op": "has_all", "mode": "skill", "reqs": reqs, "index": 0,
                "skill_fn": skill_fn, "ok": ok,
            }))
        };
        assert_eq!(
            step(false, Value::Null),
            json!({"kind": "done", "value": false})
        );
        assert_eq!(
            step(true, Value::Null),
            json!({"kind": "probe", "index": 0, "what": "skill", "name": "Tinderbox"})
        );
        assert_eq!(
            step(true, json!(false)),
            json!({"kind": "done", "value": false})
        );
        assert_eq!(
            step(true, json!(true)),
            json!({"kind": "done", "value": true})
        );
    }

    #[test]
    fn has_req_only_probes_a_truthy_name_and_keeps_js_truthiness() {
        let step =
            |req: Value, ok: Value| dispatch(&json!({"op": "has_req", "req": req, "ok": ok}));
        assert_eq!(
            step(Value::Null, Value::Null),
            json!({"kind": "done", "value": false})
        );
        assert_eq!(
            step(json!({"name": ""}), Value::Null),
            json!({"kind": "done", "value": false})
        );
        assert_eq!(
            step(json!({}), Value::Null),
            json!({"kind": "done", "value": false})
        );
        assert_eq!(
            step(json!({"name": "Tinderbox"}), Value::Null),
            json!({"kind": "probe", "index": -1, "what": "available", "name": "Tinderbox"})
        );
        // Truthiness, not `=== true` like bestFrom: the shim marshals `!!value`.
        assert_eq!(
            step(json!({"name": "Tinderbox"}), json!(false)),
            json!({"kind": "done", "value": false})
        );
        assert_eq!(
            step(json!({"name": "Tinderbox"}), json!(true)),
            json!({"kind": "done", "value": true})
        );
    }

    #[test]
    fn restock_steps_probe_inv_then_bank_and_keep_the_explicit_error() {
        let reqs = json!([{"name": "Tinderbox", "min": 2, "restock": 5, "equip": true}]);
        let step = |index: i64, count: Value, bank: Value| {
            dispatch(&json!({
                "op": "restock", "reqs": reqs, "index": index, "count": count, "bank": bank,
            }))
        };
        assert_eq!(
            step(0, Value::Null, Value::Null),
            json!({"kind": "probe", "index": 0, "what": "inv", "name": "Tinderbox"})
        );
        // Pack already at target: bankCount is never called.
        assert_eq!(step(0, json!(5), Value::Null), json!({"kind": "done"}));
        assert_eq!(
            step(0, json!(1), Value::Null),
            json!({"kind": "probe", "index": 0, "what": "bank", "name": "Tinderbox"})
        );
        assert_eq!(step(0, json!(1), json!(0)), json!({"kind": "done"}));
        assert_eq!(
            step(0, json!(1), json!(10)),
            json!({"kind": "emit", "index": 1, "step": {"name": "Tinderbox", "qty": 4.0, "equip": true}})
        );
        assert_eq!(
            step(0, json!("NaN"), json!(1)),
            json!({"kind": "emit", "index": 1, "step": {"name": "Tinderbox", "qty": 1.0, "equip": true}}),
            "an unreadable count behaves like an empty pack"
        );
        for reqs in [
            json!([{"name": "Hammer"}]),
            json!([{"kind": "tiered", "name": "Tinderbox"}]),
            json!([Value::Null]),
            json!([{"name": 7}]),
        ] {
            let error = dispatch(&json!({"op": "restock", "reqs": reqs, "index": 0}));
            assert_eq!(
                error,
                json!({"kind": "error", "feature": "Tools.toolRestockPlan"}),
                "{reqs}"
            );
        }
        let done = dispatch(&json!({"op": "restock", "reqs": [], "index": 0}));
        assert_eq!(done, json!({"kind": "done"}));
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
