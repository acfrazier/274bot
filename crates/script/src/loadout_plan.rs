//! Typed loadout planning helpers shared by v1 JSON wrappers and v2 V8.
//!
//! These functions recommend food/gear/supplies/weapon names. They do not
//! drink, equip, withdraw, or invent a snapshot loadout field.

use std::collections::BTreeMap;

use api::game_data::SelectedGameData;

use crate::loadouts_store::{self, CarryEntry};

/// One caller-supplied carry row after typed validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadoutCarry {
    pub item: String,
    /// Present only when the caller supplied `qty`. Omitted defaults to 1.
    pub qty: Option<u32>,
}

/// Caller-supplied loadout. Not a snapshot field.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LoadoutInput {
    pub name: Option<String>,
    pub worn: BTreeMap<String, String>,
    pub carry: Option<Vec<LoadoutCarry>>,
    pub unassigned: Option<Vec<String>>,
}

/// Finite `n` that is a positive integer `<= u32::MAX`.
pub fn positive_u32(n: f64) -> Option<u32> {
    if n.is_finite() && n.fract() == 0.0 && n >= 1.0 && n <= f64::from(u32::MAX) {
        Some(n as u32)
    } else {
        None
    }
}

/// Finite observation used by boost arithmetic. Negatives are kept.
pub fn finite_number(n: f64) -> Option<f64> {
    n.is_finite().then_some(n)
}

/// Held pack count: finite and `>= 0`. Fractions are allowed.
pub fn held_count(n: f64) -> Option<f64> {
    (n.is_finite() && n >= 0.0).then_some(n)
}

/// First `fixed_food_heals` carry name, or `None` when no selected match.
///
/// Names are compared as supplied (no trim), matching the current v1 helper.
pub fn food_of_name<'a, I>(data: Option<&SelectedGameData>, names: I) -> Option<String>
where
    I: IntoIterator<Item = &'a str>,
{
    let data = data?;
    for name in names {
        if let Some((known, _)) = data
            .fixed_food_heals()
            .find(|(known, _)| known.eq_ignore_ascii_case(name))
        {
            return Some(known.to_string());
        }
    }
    None
}

/// First edible carry name, otherwise `fallback`.
pub fn food_of<'a, I>(data: Option<&SelectedGameData>, names: I, fallback: &str) -> String
where
    I: IntoIterator<Item = &'a str>,
{
    food_of_name(data, names).unwrap_or_else(|| fallback.to_string())
}

pub fn food_of_input(
    data: Option<&SelectedGameData>,
    loadout: Option<&LoadoutInput>,
    fallback: &str,
) -> String {
    let names = loadout
        .and_then(|row| row.carry.as_ref())
        .into_iter()
        .flatten()
        .map(|row| row.item.as_str());
    food_of(data, names, fallback)
}

pub fn gear_of_input(loadout: Option<&LoadoutInput>) -> Vec<String> {
    let Some(loadout) = loadout else {
        return Vec::new();
    };
    loadouts_store::project_gear(
        |slot| loadout.worn.get(slot).map(String::as_str),
        loadout.unassigned.iter().flatten().map(String::as_str),
    )
}

pub fn supplies_of_input(loadout: Option<&LoadoutInput>) -> Vec<CarryEntry> {
    let Some(loadout) = loadout else {
        return Vec::new();
    };
    let Some(carry) = loadout.carry.as_ref() else {
        return Vec::new();
    };
    carry
        .iter()
        .filter_map(|row| {
            let qty = row.qty.unwrap_or(1);
            loadouts_store::carry_entry(&row.item, qty)
        })
        .collect()
}

pub fn weapon_of_input(loadout: Option<&LoadoutInput>, fallback: Option<&str>) -> Option<String> {
    let righthand = loadout
        .and_then(|row| row.worn.get("righthand"))
        .map(String::as_str);
    loadouts_store::project_weapon(righthand, fallback)
}

#[cfg(test)]
mod tests {
    use super::*;
    use client::io::ClientRevision;

    #[test]
    fn food_of_uses_fixed_heals_not_ambiguous_eat() {
        let data = api::game_data::for_revision(ClientRevision::R274).unwrap();
        assert_eq!(
            food_of(
                Some(data.as_ref()),
                ["Lobster", "Prayer potion(4)", "Cabbage"],
                "Trout"
            ),
            "Lobster"
        );
        assert_eq!(
            food_of(Some(data.as_ref()), ["Cabbage", "Ugthanki kebab"], "Trout"),
            "Trout"
        );
        assert_eq!(food_of(None, ["Lobster"], "Trout"), "Trout");
    }

    #[test]
    fn quantity_limits_reject_zero_fraction_and_overflow() {
        assert_eq!(positive_u32(1.0), Some(1));
        assert_eq!(positive_u32(f64::from(u32::MAX)), Some(u32::MAX));
        assert_eq!(positive_u32(0.0), None);
        assert_eq!(positive_u32(1.5), None);
        assert_eq!(positive_u32(-3.0), None);
        assert_eq!(positive_u32(f64::INFINITY), None);
        assert_eq!(positive_u32((u32::MAX as f64) + 1.0), None);
        assert_eq!(held_count(0.5), Some(0.5));
        assert_eq!(held_count(-0.0), Some(0.0));
        assert_eq!(held_count(-1.0), None);
        assert_eq!(finite_number(-7.0), Some(-7.0));
        assert_eq!(finite_number(f64::NAN), None);
    }
}
