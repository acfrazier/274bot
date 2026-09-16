//! Pure ranged supply predicate shared by RockCrab, MossGiant, and Brimhaven.
//!
//! Frozen source (`api/combat/ranged.ts`):
//! `rangeSupplyEmpty(equipped, carried, ground) =>
//!     equipped <= 0 && carried <= 0 && ground <= 0`.
//!
//! Ownership stays in `script` (shared helper). Callers still compute the three
//! counts; this module only answers the depletion predicate. No host wire,
//! scene scan, or loadout policy.
//!
//! Exported JS marshals with native `Number` before the JSON bridge (which
//! would otherwise turn undefined/NaN/±Infinity into null). This module keeps
//! the `<= 0` decision and accepts finite JSON numbers, null→0, missing→NaN,
//! and IEEE string tags from that shim.

use serde_json::Value;

/// True when no projectile remains equipped, carried, or on the ground.
///
/// Uses IEEE relational `<= 0.0` (same outcome as JS Number `<=` for finite
/// values and NaN). Does not saturate into integers.
pub fn range_supply_empty(equipped: f64, carried: f64, ground: f64) -> bool {
    equipped <= 0.0 && carried <= 0.0 && ground <= 0.0
}

/// Decode one bridge arg into the f64 operand the predicate compares.
///
/// Exported shim path: JS `Number(value)` then finite number or `"NaN"` /
/// `"Infinity"` / `"-Infinity"`. Direct bind path still sees:
/// - missing → NaN (undefined-like; `NaN <= 0` is false)
/// - null → 0.0 (`null <= 0` is true)
/// - JSON number → as f64 (non-finite never arrives via JSON number)
/// - IEEE string tags → parse
/// - other → NaN (number signature; no object fidelity claim)
pub fn bridge_relational_number(value: Option<&Value>) -> f64 {
    match value {
        None => f64::NAN,
        Some(Value::Null) => 0.0,
        Some(Value::Number(n)) => n.as_f64().unwrap_or(f64::NAN),
        Some(Value::String(s)) => s.parse::<f64>().unwrap_or(f64::NAN),
        Some(_) => f64::NAN,
    }
}

/// Isolate binding entry: three positional JSON args → frozen predicate.
pub fn range_supply_empty_args(args: &[Value]) -> bool {
    range_supply_empty(
        bridge_relational_number(args.first()),
        bridge_relational_number(args.get(1)),
        bridge_relational_number(args.get(2)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn depletion_only_when_all_three_sources_are_empty() {
        assert!(range_supply_empty(0.0, 0.0, 0.0));
        assert!(!range_supply_empty(1.0, 0.0, 0.0));
        assert!(!range_supply_empty(0.0, 1.0, 0.0));
        assert!(!range_supply_empty(0.0, 0.0, 1.0));
        assert!(!range_supply_empty(2.0, 3.0, 4.0));
    }

    #[test]
    fn negatives_and_zero_count_as_empty_like_js_le() {
        // equipped <= 0 is true for negatives; all three empty → depleted.
        assert!(range_supply_empty(-1.0, -2.0, -0.0));
        assert!(range_supply_empty(-0.5, 0.0, 0.0));
        assert!(!range_supply_empty(0.1, 0.0, 0.0));
    }

    #[test]
    fn nan_blocks_depletion_like_js() {
        assert!(!range_supply_empty(f64::NAN, 0.0, 0.0));
        assert!(!range_supply_empty(0.0, f64::NAN, 0.0));
        assert!(!range_supply_empty(0.0, 0.0, f64::NAN));
    }

    #[test]
    fn infinity_edges_match_js_le() {
        assert!(!range_supply_empty(f64::INFINITY, 0.0, 0.0));
        assert!(range_supply_empty(f64::NEG_INFINITY, 0.0, 0.0));
        assert!(!range_supply_empty(0.0, f64::INFINITY, 0.0));
        assert!(range_supply_empty(
            0.0,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY
        ));
    }

    #[test]
    fn large_counts_do_not_saturate_to_empty() {
        // i32::MAX+ path and large f64 must stay non-empty (not saturating i32).
        assert!(!range_supply_empty((i32::MAX as f64) + 1.0, 0.0, 0.0));
        assert!(!range_supply_empty(1e20, 0.0, 0.0));
    }

    #[test]
    fn json_bridge_covers_caller_shapes_null_missing_and_ieee_tags() {
        assert!(range_supply_empty_args(&[json!(0), json!(0), json!(0)]));
        assert!(!range_supply_empty_args(&[json!(1), json!(0), json!(0)]));
        assert!(!range_supply_empty_args(&[json!(0), json!(1), json!(0)]));
        assert!(!range_supply_empty_args(&[json!(0), json!(0), json!(1)]));
        // null ToNumber → 0; three nulls deplete.
        assert!(range_supply_empty_args(&[
            Value::Null,
            Value::Null,
            Value::Null
        ]));
        // missing args → NaN path → not depleted.
        assert!(!range_supply_empty_args(&[]));
        assert!(!range_supply_empty_args(&[json!(0)]));
        // fractional JSON number.
        assert!(!range_supply_empty_args(&[json!(0.5), json!(0), json!(0)]));
        assert!(range_supply_empty_args(&[json!(-1), json!(0), json!(0)]));
        // Shim-encoded non-finites (JSON cannot carry NaN/±Infinity as numbers).
        assert!(!range_supply_empty_args(&[
            json!("NaN"),
            json!(0),
            json!(0)
        ]));
        assert!(!range_supply_empty_args(&[
            json!("Infinity"),
            json!(0),
            json!(0)
        ]));
        assert!(range_supply_empty_args(&[
            json!("-Infinity"),
            json!(0),
            json!(0)
        ]));
    }
}
