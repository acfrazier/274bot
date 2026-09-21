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

use api::game_data::GameItem;
use serde_json::Value;

/// Thrown dart when `weapon` matches a selected `*_dart` name; otherwise bow-shaped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RangeLoadout {
    pub weapon: String,
    pub projectile: String,
    pub thrown: bool,
}

/// First selected item whose alias ends `_dart` and whose name matches
/// `weapon.trim().to_lowercase()`. Unknown weapons, including Dragon dart,
/// stay bow-shaped: raw weapon, raw ammo, `thrown: false`.
pub fn range_loadout_of<'a, I>(items: I, weapon: &str, ammo: &str) -> RangeLoadout
where
    I: IntoIterator<Item = (&'a str, &'a str)>,
{
    let wanted = weapon.trim().to_lowercase();
    for (alias, name) in items {
        if alias.ends_with("_dart") && name.to_lowercase() == wanted {
            return RangeLoadout {
                weapon: name.to_string(),
                projectile: name.to_string(),
                thrown: true,
            };
        }
    }
    RangeLoadout {
        weapon: weapon.to_string(),
        projectile: ammo.to_string(),
        thrown: false,
    }
}

pub fn range_loadout_of_items(items: &[GameItem], weapon: &str, ammo: &str) -> RangeLoadout {
    range_loadout_of(
        items.iter().filter_map(|item| {
            Some((item.alias.as_deref()?, item.name.as_deref()?))
        }),
        weapon,
        ammo,
    )
}

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
    fn dart_shape_uses_canonical_name_and_unknown_stays_bow() {
        let items = [("bronze_dart", "Bronze dart"), ("maple_shortbow", "Maple shortbow")];
        assert_eq!(
            range_loadout_of(items, "Bronze dart", "Iron arrow"),
            RangeLoadout {
                weapon: "Bronze dart".into(),
                projectile: "Bronze dart".into(),
                thrown: true,
            }
        );
        assert_eq!(
            range_loadout_of(items, "  BRONZE DART ", "Iron arrow").weapon,
            "Bronze dart"
        );
        assert_eq!(
            range_loadout_of(items, "Maple shortbow", "Iron arrow"),
            RangeLoadout {
                weapon: "Maple shortbow".into(),
                projectile: "Iron arrow".into(),
                thrown: false,
            }
        );
        assert_eq!(
            range_loadout_of(items, "Dragon dart", "Iron arrow"),
            RangeLoadout {
                weapon: "Dragon dart".into(),
                projectile: "Iron arrow".into(),
                thrown: false,
            }
        );
        assert_eq!(
            range_loadout_of(items, "", "Iron arrow").projectile,
            "Iron arrow"
        );
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
