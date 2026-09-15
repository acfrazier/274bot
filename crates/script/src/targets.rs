//! Native `chooseTarget` traversal over the caller's nearest-first candidates.
//!
//! The frozen helper is a `for...of` first-hit scan over a caller-supplied
//! iterable:
//!
//! ```text
//! for (const c of candidatesNearestFirst) {
//!     if (reachable(c)) return { target: c, blocked: null };
//! }
//! return { target: null, blocked: candidatesNearestFirst[0] ?? null };
//! ```
//!
//! [`dispatch`] backs the `__rs2b0t_target_step` binding and owns that
//! traversal: which candidate is visited, that the first truthy `reachable`
//! answer is the hit, that `reachable` is never called past the hit, and that
//! the blocked fallback only follows exhaustion. The shim keeps the caller's
//! iterator and the element it last read, runs `next()` / `reachable(value)` /
//! `candidatesNearestFirst[0]` exactly when a step asks for it, and reports
//! back only `done` / truthiness booleans — no caller object, candidate,
//! callback answer or collected reachability list crosses the rustyscript
//! bridge.
//!
//! The `for...of` engine details stay in JS because the shim still drives the
//! caller's real iterator: live length, mutation, holes read as `undefined`,
//! callback receiver/argument/order, reentrancy and the iterator close on an
//! early exit are unchanged. Truthiness is the shim's `!!` (never
//! `=== true`), so a truthy non-`true` answer is still a hit.

use serde_json::{json, Value};

/// One step of a `chooseTarget` decision (`__rs2b0t_target_step`).
///
/// The shim reports the bounded answer to the previous action: `{op:"choose"}`
/// starts, `{done}` reports what the caller's iterator returned, and
/// `{probed}` reports the truthiness of `reachable(value)` for the element that
/// iterator last produced. The reply is the next action — `next` (advance the
/// caller's iterator), `probe` (call `reachable` on the current element), `hit`
/// (return the current element) or `exhausted` (read the blocked fallback).
///
/// Stateless by construction, so a `reachable` callback that re-enters
/// `chooseTarget` cannot disturb the decision in flight. An unknown op is an
/// explicit feature error rather than a silent fallback.
pub fn dispatch(input: &Value) -> Value {
    match input.get("op").and_then(Value::as_str) {
        Some("choose") => choose_step(input),
        _ => json!({ "kind": "notImpl", "reason": "unknown op" }),
    }
}

/// First-hit state machine: a reported probe answer decides hit versus advance;
/// otherwise an exhausted iterator step is the only path to the fallback.
fn choose_step(input: &Value) -> Value {
    if let Some(probed) = input.get("probed").and_then(Value::as_bool) {
        return if probed {
            json!({ "kind": "hit" })
        } else {
            json!({ "kind": "next" })
        };
    }
    match input.get("done").and_then(Value::as_bool) {
        None => json!({ "kind": "next" }),
        Some(false) => json!({ "kind": "probe" }),
        Some(true) => json!({ "kind": "exhausted" }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(payload: Value) -> Value {
        dispatch(&payload)
    }

    #[test]
    fn start_asks_for_the_first_iterator_step() {
        assert_eq!(step(json!({ "op": "choose" })), json!({ "kind": "next" }));
    }

    #[test]
    fn an_iterator_value_is_probed_and_a_done_step_falls_back() {
        assert_eq!(
            step(json!({ "op": "choose", "done": false })),
            json!({ "kind": "probe" })
        );
        assert_eq!(
            step(json!({ "op": "choose", "done": true })),
            json!({ "kind": "exhausted" }),
            "exhaustion is the only path to the blocked fallback"
        );
    }

    #[test]
    fn the_first_truthy_probe_is_the_hit_and_a_falsy_one_advances() {
        assert_eq!(
            step(json!({ "op": "choose", "probed": true })),
            json!({ "kind": "hit" })
        );
        assert_eq!(
            step(json!({ "op": "choose", "probed": false })),
            json!({ "kind": "next" }),
            "a falsy answer never ends the scan"
        );
    }

    #[test]
    fn a_probe_answer_outranks_a_stale_iterator_read() {
        assert_eq!(
            step(json!({ "op": "choose", "done": false, "probed": true })),
            json!({ "kind": "hit" })
        );
        assert_eq!(
            step(json!({ "op": "choose", "done": false, "probed": false })),
            json!({ "kind": "next" }),
            "the value the iterator already produced is not read twice"
        );
        assert_eq!(
            step(json!({ "op": "choose", "done": true, "probed": true })),
            json!({ "kind": "hit" }),
            "a hit reported before the exhaustion step still wins"
        );
    }

    #[test]
    fn a_non_boolean_observation_is_not_a_decision() {
        assert_eq!(
            step(json!({ "op": "choose", "probed": "yes", "done": false })),
            json!({ "kind": "probe" }),
            "only a JS boolean decides; the shim owns the truthiness conversion"
        );
        assert_eq!(
            step(json!({ "op": "choose", "done": 1 })),
            json!({ "kind": "next" }),
            "a missing iterator answer starts the scan"
        );
    }

    #[test]
    fn unknown_ops_are_an_explicit_feature_error() {
        assert_eq!(
            step(json!({ "op": "nope" })),
            json!({ "kind": "notImpl", "reason": "unknown op" })
        );
        assert_eq!(
            step(Value::Null),
            json!({ "kind": "notImpl", "reason": "unknown op" })
        );
    }
}
