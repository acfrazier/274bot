//! Native boost-potion descriptors, planning and sip selection.
//!
//! The frozen `bot/api/combat/boostPotions` helper holds script-owned policy:
//! which dose forms exist, which flask the bank run draws, and whether a boost
//! is due another dose. That policy lives here; the shim only marshals
//! arguments, runs the one caller iteration or callback a step asks for, and
//! returns the original objects.
//!
//! [`dispatch`] backs the `__rs2b0t_boost_potions_step` binding:
//!
//! ```text
//! op "table"  -> the descriptor rows, the floor and the drained flask name
//! op "faded"  -> `boostFaded(base, effective[, floor])`
//! op "plan"   -> `plannedPotions(carry)`, one step per dose comparison
//! op "sip"    -> `potionToSip(state)`, one step per callback
//! ```
//!
//! Every step is stateless, so a callback that re-enters a helper cannot disturb
//! the selection in flight, and no caller object, carry row, plan or callback
//! answer crosses the rustyscript bridge: the shim reports a single bounded
//! observation per step (the item key the frozen comparison reads at that dose,
//! level numbers, held truthiness).
//!
//! JS number semantics stay at the marshaling boundary. The shim converts the
//! values it reads with `Number()` — the same coercion the frozen arithmetic
//! would apply — and tags the three non-finite results with `String(value)`,
//! because no JSON number can carry them. The threshold arithmetic below is
//! then the frozen IEEE-754 arithmetic.

use serde_json::{json, Value};

/// A JS number at the marshaling boundary.
///
/// Finite values cross the isolate bridge as JSON numbers; `NaN`, `Infinity`
/// and `-Infinity` have no JSON form, so the shim sends what `String(value)`
/// produces for them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum JsNumber {
    Finite(f64),
    NaN,
    Infinity,
    NegInfinity,
}

impl JsNumber {
    /// The value a `Number()` conversion produced.
    pub fn of(value: f64) -> Self {
        if value.is_nan() {
            JsNumber::NaN
        } else if value == f64::INFINITY {
            JsNumber::Infinity
        } else if value == f64::NEG_INFINITY {
            JsNumber::NegInfinity
        } else {
            JsNumber::Finite(value)
        }
    }

    /// This value as a Rust `f64` (`NaN` stays `NaN`).
    pub fn get(self) -> f64 {
        match self {
            JsNumber::Finite(value) => value,
            JsNumber::NaN => f64::NAN,
            JsNumber::Infinity => f64::INFINITY,
            JsNumber::NegInfinity => f64::NEG_INFINITY,
        }
    }

    fn from_json(value: &Value) -> Option<Self> {
        match value {
            Value::Number(number) => number.as_f64().map(JsNumber::Finite),
            Value::String(tag) => match tag.as_str() {
                "NaN" => Some(JsNumber::NaN),
                "Infinity" => Some(JsNumber::Infinity),
                "-Infinity" => Some(JsNumber::NegInfinity),
                _ => None,
            },
            _ => None,
        }
    }
}

/// Dose counts, high to low, of every dose form the table lists.
const DOSE_COUNTS: [u32; 4] = [4, 3, 2, 1];

/// Index into [`DOSE_COUNTS`] of the dose form drawn when the loadout names none.
const DEFAULT_DOSE: usize = 1;

/// Flasks to carry per trip when the loadout names no dose for a potion.
pub const DEFAULT_WANT: i64 = 1;

/// One super-potion row: the skill the dose lifts, the paint label kept short
/// enough for a three-column row, and the dose-name stem.
pub struct BoostPotion {
    pub skill: &'static str,
    pub short: &'static str,
    stem: &'static str,
}

impl BoostPotion {
    /// Every dose form, high to low, so a part-used flask still counts as one in
    /// the pack.
    pub fn doses(&self) -> Vec<String> {
        (0..DOSE_COUNTS.len())
            .filter_map(|dose| self.dose_name(dose))
            .collect()
    }

    /// The dose form drawn when the loadout names none.
    pub fn flask(&self) -> String {
        self.dose_name(DEFAULT_DOSE).unwrap_or_default()
    }

    /// The canonical name of dose `dose` (0 is the four-dose flask).
    pub fn dose_name(&self, dose: usize) -> Option<String> {
        DOSE_COUNTS
            .get(dose)
            .map(|count| format!("{}({count})", self.stem))
    }

    /// Whether dose `dose` is the one `key` names, case-insensitively.
    ///
    /// `key` is the carried item the shim read at the frozen comparison's own
    /// right operand for this dose, already trimmed and lowercased.
    pub fn dose_matches(&self, dose: usize, key: &str) -> bool {
        self.dose_name(dose)
            .is_some_and(|name| name.to_lowercase() == key)
    }
}

pub const SUPER_ATTACK: BoostPotion = BoostPotion {
    skill: "attack",
    short: "Att",
    stem: "Super attack",
};

pub const SUPER_STRENGTH: BoostPotion = BoostPotion {
    skill: "strength",
    short: "Str",
    stem: "Super strength",
};

/// Checked in this order; attack wins a tick both could use.
pub const BOOST_POTIONS: [&BoostPotion; 2] = [&SUPER_ATTACK, &SUPER_STRENGTH];

/// What the last dose leaves behind.
pub const EMPTY_VIAL: &str = "Vial";

/// Share of the base level the boost may decay to before another dose is worth
/// its tick.
pub const BOOST_FLOOR: f64 = 0.1;

/// Whether the boost has decayed back into the floor band.
///
/// `base > 0 && effective - base >= 0 && effective - base <= floor * base`: a
/// boost sitting exactly on the threshold is due, and a level drained *below*
/// its base is not a decayed boost, because a super potion restores none of it.
pub fn boost_faded(base: JsNumber, effective: JsNumber, floor: JsNumber) -> bool {
    let (base, effective, floor) = (base.get(), effective.get(), floor.get());
    let boost = effective - base;
    base > 0.0 && boost >= 0.0 && boost <= floor * base
}

// --- step helpers --------------------------------------------------------------

/// An unsupported or malformed step stays an explicit `notImpl` error in the shim.
fn feature_error(feature: &str) -> Value {
    json!({ "kind": "error", "feature": feature })
}

fn potion_at(payload: &Value) -> Option<&'static BoostPotion> {
    let index = payload.get("potion").and_then(Value::as_i64)?;
    usize::try_from(index)
        .ok()
        .and_then(|index| BOOST_POTIONS.get(index).copied())
}

/// The descriptor rows the shim exports, the floor and the drained flask name.
/// The frozen module builds these from its own literals; here they are the one
/// source, so the exported descriptors, the plan defaults and the sip threshold
/// cannot drift apart.
fn table() -> Value {
    json!({
        "kind": "table",
        "floor": BOOST_FLOOR,
        "empty_vial": EMPTY_VIAL,
        "potions": BOOST_POTIONS
            .iter()
            .map(|potion| json!({
                "skill": potion.skill,
                "short": potion.short,
                "flask": potion.flask(),
                "doses": potion.doses(),
            }))
            .collect::<Vec<Value>>(),
    })
}

/// `boostFaded(base, effective[, floor])`: an absent `floor` is the frozen
/// default parameter, not a caller value.
fn faded_step(payload: &Value) -> Value {
    let Some(base) = payload.get("base").and_then(JsNumber::from_json) else {
        return feature_error("boostPotions.boostFaded");
    };
    let Some(effective) = payload.get("effective").and_then(JsNumber::from_json) else {
        return feature_error("boostPotions.boostFaded");
    };
    let floor = match payload.get("floor") {
        None => JsNumber::Finite(BOOST_FLOOR),
        Some(value) => match JsNumber::from_json(value) {
            Some(floor) => floor,
            None => return feature_error("boostPotions.boostFaded"),
        },
    };
    json!({ "kind": "value", "value": boost_faded(base, effective, floor) })
}

/// One step of `plannedPotions(carry)`.
fn plan_step(payload: &Value) -> Value {
    match payload.get("what").and_then(Value::as_str) {
        Some("round") => plan_round(payload),
        Some("entry") => plan_entry(payload),
        Some("exhausted") => plan_exhausted(payload),
        _ => feature_error("boostPotions.plannedPotions"),
    }
}

/// The table's own order picks the potion to plan: the row after the potion the
/// caller just planned, or the end of the table.
fn plan_round(payload: &Value) -> Value {
    let next = match payload.get("done_potion") {
        None => 0,
        Some(value) => match value.as_i64() {
            Some(done) => done.saturating_add(1),
            None => return feature_error("boostPotions.plannedPotions"),
        },
    };
    match usize::try_from(next)
        .ok()
        .filter(|index| *index < BOOST_POTIONS.len())
    {
        Some(index) => json!({ "kind": "scan", "potion": index }),
        None => json!({ "kind": "done" }),
    }
}

/// One dose comparison of `potion.doses.find(...)`, in dose order.
///
/// The reply asks for the item the frozen callback reads at that comparison —
/// the frozen `find` re-reads `entry.item.trim().toLowerCase()` once per dose,
/// so a carried getter is read exactly as often here. The first dose that
/// matches names the flask to withdraw; a row that never matches keeps the scan
/// going to the next entry.
fn plan_entry(payload: &Value) -> Value {
    let Some(potion) = potion_at(payload) else {
        return feature_error("boostPotions.plannedPotions");
    };
    let dose = match payload.get("dose") {
        None => return json!({ "kind": "compare", "dose": 0 }),
        Some(value) => match value.as_i64().and_then(|dose| usize::try_from(dose).ok()) {
            Some(dose) => dose,
            None => return feature_error("boostPotions.plannedPotions"),
        },
    };
    let Some(item) = payload.get("item").and_then(Value::as_str) else {
        return feature_error("boostPotions.plannedPotions");
    };
    if dose >= DOSE_COUNTS.len() {
        return feature_error("boostPotions.plannedPotions");
    }
    if potion.dose_matches(dose, item) {
        return match potion.dose_name(dose) {
            Some(flask) => json!({ "kind": "accept", "flask": flask }),
            None => feature_error("boostPotions.plannedPotions"),
        };
    }
    if dose + 1 < DOSE_COUNTS.len() {
        json!({ "kind": "compare", "dose": dose + 1 })
    } else {
        json!({ "kind": "next" })
    }
}

/// The scan ran out of carried entries: the table's own flask and count.
fn plan_exhausted(payload: &Value) -> Value {
    let Some(potion) = potion_at(payload) else {
        return feature_error("boostPotions.plannedPotions");
    };
    json!({ "kind": "fallback", "flask": potion.flask(), "want": DEFAULT_WANT })
}

/// One step of `potionToSip(state)`.
///
/// The shim reports one observation per call: `reached` (the caller's own
/// iteration produced a plan), then the `levels` answer for that plan's skill,
/// then whether the pack holds that potion. The reply is the next callback to
/// run, the next iteration, the selected plan or the no-sip outcome.
///
/// Native owns the levels-before-held order and the first-match exit, so a
/// later plan's callbacks are never requested and a plan the caller already
/// reported is never re-decided.
fn sip_step(payload: &Value) -> Value {
    match payload.get("reached").and_then(Value::as_bool) {
        Some(true) => {}
        Some(false) => return json!({ "kind": "none" }),
        None => return feature_error("boostPotions.potionToSip"),
    }
    let (base, effective) = match (payload.get("base"), payload.get("effective")) {
        (None, None) => return json!({ "kind": "levels" }),
        (base, effective) => {
            let (Some(base), Some(effective)) = (
                base.and_then(JsNumber::from_json),
                effective.and_then(JsNumber::from_json),
            ) else {
                return feature_error("boostPotions.potionToSip");
            };
            (base, effective)
        }
    };
    let held_ok = match payload.get("held_ok") {
        None => return json!({ "kind": "held" }),
        Some(value) => match value.as_bool() {
            Some(held_ok) => held_ok,
            None => return feature_error("boostPotions.potionToSip"),
        },
    };
    if held_ok && boost_faded(base, effective, JsNumber::Finite(BOOST_FLOOR)) {
        json!({ "kind": "hit" })
    } else {
        json!({ "kind": "next" })
    }
}

/// One step of a boost-potion helper (`__rs2b0t_boost_potions_step`).
///
/// Stateless by construction, so a `levels` or `held` callback that re-enters a
/// helper cannot disturb the selection in flight. An unknown op or a malformed
/// observation is an explicit feature error rather than a silent answer.
pub fn dispatch(payload: &Value) -> Value {
    match payload.get("op").and_then(Value::as_str) {
        Some("table") => table(),
        Some("faded") => faded_step(payload),
        Some("plan") => plan_step(payload),
        Some("sip") => sip_step(payload),
        _ => feature_error("boostPotions"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(payload: Value) -> Value {
        dispatch(&payload)
    }

    fn plan(what: &str, payload: Value) -> Value {
        let mut payload = payload;
        payload["op"] = json!("plan");
        payload["what"] = json!(what);
        dispatch(&payload)
    }

    #[test]
    fn the_table_is_the_frozen_descriptor_rows_in_order() {
        let table = step(json!({ "op": "table" }));
        assert_eq!(table["floor"], json!(0.1));
        assert_eq!(table["empty_vial"], json!("Vial"));
        assert_eq!(
            table["potions"],
            json!([
                {
                    "skill": "attack",
                    "short": "Att",
                    "flask": "Super attack(3)",
                    "doses": [
                        "Super attack(4)",
                        "Super attack(3)",
                        "Super attack(2)",
                        "Super attack(1)"
                    ],
                },
                {
                    "skill": "strength",
                    "short": "Str",
                    "flask": "Super strength(3)",
                    "doses": [
                        "Super strength(4)",
                        "Super strength(3)",
                        "Super strength(2)",
                        "Super strength(1)"
                    ],
                }
            ]),
            "attack is checked before strength"
        );
        assert_eq!(BOOST_FLOOR, 0.1);
        assert_eq!(EMPTY_VIAL, "Vial");
        assert_eq!(SUPER_ATTACK.short, "Att");
        assert_eq!(SUPER_STRENGTH.short, "Str");
    }

    #[test]
    fn boost_faded_is_the_floor_band_and_never_a_drained_level() {
        let faded = |base: f64, effective: f64| {
            boost_faded(
                JsNumber::of(base),
                JsNumber::of(effective),
                JsNumber::Finite(BOOST_FLOOR),
            )
        };
        assert!(faded(70.0, 70.0), "an unboosted skill is due its first sip");
        assert!(faded(70.0, 77.0), "a boost exactly on the tenth is due");
        assert!(!faded(70.0, 78.0), "one level above the tenth is not");
        assert!(!faded(70.0, 85.0), "a fresh super potion is not faded");
        assert!(faded(1.0, 1.0), "a tenth of one rounds to nothing");
        assert!(!faded(1.0, 2.0));
        assert!(!faded(0.0, 0.0), "an unread skill never asks for a dose");
        assert!(!faded(70.0, 60.0), "a drained skill is not a decayed boost");
    }

    #[test]
    fn boost_faded_keeps_the_caller_floor_and_non_finite_numbers() {
        assert!(boost_faded(
            JsNumber::Finite(70.0),
            JsNumber::Finite(77.0),
            JsNumber::Finite(0.2)
        ));
        assert!(!boost_faded(
            JsNumber::Finite(70.0),
            JsNumber::Finite(77.0),
            JsNumber::Finite(0.0)
        ));
        assert!(!boost_faded(
            JsNumber::NaN,
            JsNumber::NaN,
            JsNumber::Finite(BOOST_FLOOR)
        ));
        assert!(!boost_faded(
            JsNumber::Finite(70.0),
            JsNumber::Infinity,
            JsNumber::Finite(BOOST_FLOOR)
        ));
        assert!(!boost_faded(
            JsNumber::Infinity,
            JsNumber::Finite(85.0),
            JsNumber::Finite(BOOST_FLOOR)
        ));
        assert!(
            boost_faded(
                JsNumber::Finite(70.0),
                JsNumber::Infinity,
                JsNumber::Infinity
            ),
            "the frozen arithmetic keeps `Infinity <= Infinity * 70` due"
        );
    }

    #[test]
    fn the_faded_op_defaults_an_absent_floor_and_reports_a_value() {
        assert_eq!(
            step(json!({ "op": "faded", "base": 70, "effective": 77 })),
            json!({ "kind": "value", "value": true }),
            "an omitted floor is the frozen default parameter"
        );
        assert_eq!(
            step(json!({ "op": "faded", "base": 70, "effective": 78 })),
            json!({ "kind": "value", "value": false })
        );
        assert_eq!(
            step(json!({ "op": "faded", "base": 70, "effective": 77, "floor": 0.05 })),
            json!({ "kind": "value", "value": false })
        );
        assert_eq!(
            step(json!({ "op": "faded", "base": 70, "effective": "Infinity" })),
            json!({ "kind": "value", "value": false }),
            "a tagged non-finite value is still the caller's number"
        );
        assert_eq!(
            step(json!({ "op": "faded", "base": 70, "effective": 77, "floor": null })),
            json!({ "kind": "error", "feature": "boostPotions.boostFaded" }),
            "a null floor is not a caller number"
        );
        assert_eq!(
            step(json!({ "op": "faded", "effective": 77 })),
            json!({ "kind": "error", "feature": "boostPotions.boostFaded" })
        );
    }

    #[test]
    fn a_dose_matches_only_its_lowercased_canonical_name() {
        assert!(SUPER_ATTACK.dose_matches(2, "super attack(2)"));
        assert_eq!(
            SUPER_ATTACK.dose_name(2).as_deref(),
            Some("Super attack(2)")
        );
        assert_eq!(
            SUPER_ATTACK.doses(),
            vec![
                "Super attack(4)",
                "Super attack(3)",
                "Super attack(2)",
                "Super attack(1)"
            ]
        );
        assert!(!SUPER_ATTACK.dose_matches(2, "super attack(5)"));
        assert!(!SUPER_ATTACK.dose_matches(2, "Super attack(2)"));
        assert!(!SUPER_ATTACK.dose_matches(0, "super attack(2)"));
        assert!(!SUPER_ATTACK.dose_matches(4, "super attack(5)"));
        assert!(SUPER_STRENGTH.dose_matches(0, "super strength(4)"));
        assert!(
            !SUPER_ATTACK.dose_matches(0, "super strength(4)"),
            "a dose form belongs to its own potion"
        );
    }

    #[test]
    fn a_round_names_the_next_table_potion_or_the_end() {
        assert_eq!(
            plan("round", json!({})),
            json!({ "kind": "scan", "potion": 0 })
        );
        assert_eq!(
            plan("round", json!({ "done_potion": 0 })),
            json!({ "kind": "scan", "potion": 1 })
        );
        assert_eq!(
            plan("round", json!({ "done_potion": 1 })),
            json!({ "kind": "done" }),
            "the table has two rows"
        );
        assert_eq!(
            plan("round", json!({ "done_potion": 7 })),
            json!({ "kind": "done" })
        );
        assert_eq!(
            plan("round", json!({ "done_potion": "0" })),
            json!({ "kind": "error", "feature": "boostPotions.plannedPotions" })
        );
    }

    #[test]
    fn an_entry_compares_its_doses_in_order_and_accepts_the_first_match() {
        assert_eq!(
            plan("entry", json!({ "potion": 0 })),
            json!({ "kind": "compare", "dose": 0 }),
            "the dose table's own order is the comparison order"
        );
        assert_eq!(
            plan(
                "entry",
                json!({ "potion": 0, "dose": 0, "item": "super attack(2)" })
            ),
            json!({ "kind": "compare", "dose": 1 })
        );
        assert_eq!(
            plan(
                "entry",
                json!({ "potion": 0, "dose": 1, "item": "super attack(2)" })
            ),
            json!({ "kind": "compare", "dose": 2 })
        );
        assert_eq!(
            plan(
                "entry",
                json!({ "potion": 0, "dose": 2, "item": "super attack(2)" })
            ),
            json!({ "kind": "accept", "flask": "Super attack(2)" }),
            "the match names that dose form's flask"
        );
        assert_eq!(
            plan(
                "entry",
                json!({ "potion": 1, "dose": 3, "item": "super strength(1)" })
            ),
            json!({ "kind": "accept", "flask": "Super strength(1)" })
        );
        assert_eq!(
            plan(
                "entry",
                json!({ "potion": 0, "dose": 3, "item": "lobster" })
            ),
            json!({ "kind": "next" }),
            "a row that never matches ends this entry's comparisons"
        );
        assert_eq!(
            plan(
                "entry",
                json!({ "potion": 0, "dose": 0, "item": "Super attack(4)" })
            ),
            json!({ "kind": "compare", "dose": 1 }),
            "the shim owns the trim and lowercase the frozen comparison does"
        );
        assert_eq!(
            plan(
                "entry",
                json!({ "potion": 9, "dose": 0, "item": "super attack(4)" })
            ),
            json!({ "kind": "error", "feature": "boostPotions.plannedPotions" })
        );
        assert_eq!(
            plan(
                "entry",
                json!({ "potion": 0, "dose": 4, "item": "super attack(4)" })
            ),
            json!({ "kind": "error", "feature": "boostPotions.plannedPotions" }),
            "there are four dose forms"
        );
        assert_eq!(
            plan("entry", json!({ "potion": 0, "dose": 0 })),
            json!({ "kind": "error", "feature": "boostPotions.plannedPotions" })
        );
        assert_eq!(
            plan(
                "entry",
                json!({ "potion": 0, "dose": "0", "item": "super attack(4)" })
            ),
            json!({ "kind": "error", "feature": "boostPotions.plannedPotions" })
        );
    }

    #[test]
    fn an_exhausted_scan_falls_back_to_the_table_default() {
        assert_eq!(
            plan("exhausted", json!({ "potion": 0 })),
            json!({ "kind": "fallback", "flask": "Super attack(3)", "want": 1 })
        );
        assert_eq!(
            plan("exhausted", json!({ "potion": 1 })),
            json!({ "kind": "fallback", "flask": "Super strength(3)", "want": 1 })
        );
        assert_eq!(
            plan("exhausted", json!({})),
            json!({ "kind": "error", "feature": "boostPotions.plannedPotions" })
        );
    }

    fn sip(payload: Value) -> Value {
        let mut payload = payload;
        payload["op"] = json!("sip");
        dispatch(&payload)
    }

    #[test]
    fn sip_asks_levels_then_held_and_decides_the_first_due_plan() {
        assert_eq!(sip(json!({ "reached": true })), json!({ "kind": "levels" }));
        assert_eq!(
            sip(json!({ "reached": true, "base": 70, "effective": 77 })),
            json!({ "kind": "held" }),
            "held is asked only after the caller's levels answer"
        );
        assert_eq!(
            sip(json!({ "reached": true, "base": 70, "effective": 77, "held_ok": true })),
            json!({ "kind": "hit" })
        );
        assert_eq!(
            sip(json!({ "reached": true, "base": 70, "effective": 78, "held_ok": true })),
            json!({ "kind": "next" })
        );
        assert_eq!(
            sip(json!({ "reached": true, "base": 70, "effective": 77, "held_ok": false })),
            json!({ "kind": "next" }),
            "an empty pack never selects its plan"
        );
        assert_eq!(
            sip(json!({ "reached": true, "base": 70, "effective": 60, "held_ok": true })),
            json!({ "kind": "next" }),
            "a drained skill is not a decayed boost"
        );
        assert_eq!(
            sip(json!({ "reached": false })),
            json!({ "kind": "none" }),
            "an exhausted caller iteration sips nothing"
        );
    }

    #[test]
    fn sip_rejects_a_malformed_observation_instead_of_repeating_a_step() {
        assert_eq!(
            sip(json!({})),
            json!({ "kind": "error", "feature": "boostPotions.potionToSip" })
        );
        assert_eq!(
            sip(json!({ "reached": true, "base": 70 })),
            json!({ "kind": "error", "feature": "boostPotions.potionToSip" })
        );
        assert_eq!(
            sip(json!({ "reached": true, "base": 70, "effective": null })),
            json!({ "kind": "error", "feature": "boostPotions.potionToSip" })
        );
        assert_eq!(
            sip(json!({ "reached": true, "base": 70, "effective": 77, "held_ok": 1 })),
            json!({ "kind": "error", "feature": "boostPotions.potionToSip" })
        );
        assert_eq!(
            sip(json!({ "reached": "yes" })),
            json!({ "kind": "error", "feature": "boostPotions.potionToSip" })
        );
    }

    #[test]
    fn unknown_ops_and_missing_payloads_are_an_explicit_feature_error() {
        assert_eq!(
            step(json!({ "op": "nope" })),
            json!({ "kind": "error", "feature": "boostPotions" })
        );
        assert_eq!(
            step(Value::Null),
            json!({ "kind": "error", "feature": "boostPotions" })
        );
        assert_eq!(
            step(json!({ "op": "plan" })),
            json!({ "kind": "error", "feature": "boostPotions.plannedPotions" })
        );
    }

    #[test]
    fn js_numbers_round_trip_every_ieee_754_state() {
        assert_eq!(JsNumber::of(1.5).get(), 1.5);
        assert_eq!(JsNumber::of(f64::NAN), JsNumber::NaN);
        assert_eq!(JsNumber::of(f64::INFINITY), JsNumber::Infinity);
        assert_eq!(JsNumber::of(f64::NEG_INFINITY), JsNumber::NegInfinity);
        assert!(JsNumber::NaN.get().is_nan());
        assert_eq!(
            JsNumber::from_json(&json!("-Infinity")),
            Some(JsNumber::NegInfinity)
        );
        assert_eq!(JsNumber::from_json(&json!("later")), None);
        assert_eq!(JsNumber::from_json(&json!(true)), None);
        assert_eq!(JsNumber::from_json(&Value::Null), None);
        assert_eq!(JsNumber::from_json(&json!(2)), Some(JsNumber::Finite(2.0)));
    }
}
