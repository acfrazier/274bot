//! Rust-owned composition for the compatibility combat keep list.

use api::game_data::{GameItem, SelectedGameData};
use serde_json::Value;
use std::collections::HashSet;

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct CombatKeepOptions {
    pub(crate) food: String,
    pub(crate) style: String,
    pub(crate) spell: String,
    pub(crate) ammo: String,
    pub(crate) weapon: String,
    pub(crate) extra: Vec<String>,
}

pub(crate) fn from_args(args: &[Value]) -> CombatKeepOptions {
    let Some(object) = args.first().and_then(Value::as_object) else {
        return CombatKeepOptions::default();
    };
    let string = |field| {
        object
            .get(field)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    };
    let extra = object
        .get("extra")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    CombatKeepOptions {
        food: string("food"),
        style: string("style"),
        spell: string("spell"),
        ammo: string("ammo"),
        weapon: string("weapon"),
        extra,
    }
}

pub(crate) fn combat_keep_names(
    data: &SelectedGameData,
    options: &CombatKeepOptions,
) -> Vec<String> {
    let mut keep = food_forms(data.items(), &options.food);
    keep.extend(options.extra.iter().cloned());
    if options.style == "mage" && !options.spell.is_empty() {
        if let Some(spell) = data
            .spells()
            .iter()
            .find(|spell| spell.name == options.spell)
        {
            keep.extend(spell.runes.iter().map(|rune| rune.name.clone()));
        }
    }
    if options.style == "range" && !options.ammo.is_empty() {
        keep.push(options.ammo.clone());
    }
    if !options.weapon.is_empty() {
        keep.push(options.weapon.clone());
    }
    keep
}

pub(crate) fn food_forms(items: &[GameItem], food: &str) -> Vec<String> {
    let key = food.trim().to_lowercase();
    let Some(alias) = items
        .iter()
        .find(|item| {
            item.name
                .as_deref()
                .is_some_and(|name| name.eq_ignore_ascii_case(&key))
        })
        .and_then(|item| item.alias.as_deref())
    else {
        return vec![key];
    };
    if alias.starts_with("partial_")
        || alias.starts_with("half_")
        || alias.starts_with("half_a_")
        || alias.starts_with("half_an_")
        || alias.ends_with("_slice")
        || alias.starts_with("cert_")
    {
        return vec![key];
    }

    let aliases = [
        alias.to_string(),
        format!("partial_{alias}"),
        format!("{alias}_slice"),
    ];
    let mut seen = HashSet::new();
    items
        .iter()
        .filter(|item| {
            item.alias
                .as_ref()
                .is_some_and(|candidate| aliases.contains(candidate))
        })
        .filter_map(|item| item.name.as_deref())
        .map(str::to_lowercase)
        .filter(|name| seen.insert(name.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use client::io::ClientRevision;

    fn data(revision: ClientRevision) -> std::sync::Arc<SelectedGameData> {
        api::game_data::for_revision(revision).expect("selected game data")
    }

    fn options(food: &str, style: &str) -> CombatKeepOptions {
        CombatKeepOptions {
            food: food.into(),
            style: style.into(),
            ..CombatKeepOptions::default()
        }
    }

    #[test]
    fn selected_food_forms_and_caller_order_match_both_revisions() {
        for revision in [ClientRevision::R274, ClientRevision::R289] {
            let data = data(revision);
            let mut full = options("Cake", "melee");
            full.extra = vec!["Coins".into()];
            full.weapon = "Rune scimitar".into();
            assert_eq!(
                combat_keep_names(&data, &full),
                [
                    "cake",
                    "2/3 cake",
                    "slice of cake",
                    "Coins",
                    "Rune scimitar"
                ]
            );
            assert_eq!(
                combat_keep_names(&data, &options("2/3 cake", "melee")),
                ["2/3 cake"]
            );
            assert_eq!(
                combat_keep_names(&data, &options("NotAFoodXYZ", "melee")),
                ["notafoodxyz"]
            );
        }
    }

    #[test]
    fn mage_keeps_all_exact_spell_runes_without_staff_subtraction() {
        for revision in [ClientRevision::R274, ClientRevision::R289] {
            let data = data(revision);
            let mut mage = options("Lobster", "mage");
            mage.spell = "Wind Strike".into();
            assert_eq!(
                combat_keep_names(&data, &mage),
                ["lobster", "Mind rune", "Air rune"]
            );

            mage.style = "melee".into();
            assert_eq!(combat_keep_names(&data, &mage), ["lobster"]);
            mage.style = "mage".into();
            mage.spell = "wind strike".into();
            assert_eq!(combat_keep_names(&data, &mage), ["lobster"]);
            mage.spell = "Unknown spell".into();
            assert_eq!(combat_keep_names(&data, &mage), ["lobster"]);
        }
    }

    #[test]
    fn ranged_ammo_and_duplicates_preserve_canonical_order() {
        for revision in [ClientRevision::R274, ClientRevision::R289] {
            let data = data(revision);
            let mut range = options("Cake", "range");
            range.extra = vec!["Coins".into(), "cake".into(), "Coins".into()];
            range.ammo = "Iron arrow".into();
            range.weapon = "Maple shortbow".into();
            assert_eq!(
                combat_keep_names(&data, &range),
                [
                    "cake",
                    "2/3 cake",
                    "slice of cake",
                    "Coins",
                    "cake",
                    "Coins",
                    "Iron arrow",
                    "Maple shortbow"
                ]
            );

            range.style = "melee".into();
            range.ammo.clear();
            range.weapon.clear();
            assert_eq!(
                combat_keep_names(&data, &range),
                [
                    "cake",
                    "2/3 cake",
                    "slice of cake",
                    "Coins",
                    "cake",
                    "Coins"
                ]
            );
        }
    }

    #[test]
    fn moss_shape_never_infers_cargo_or_equipment() {
        for revision in [ClientRevision::R274, ClientRevision::R289] {
            let data = data(revision);
            let mut moss = options("Lobster", "melee");
            moss.extra = vec!["Coins".into()];
            let keep = combat_keep_names(&data, &moss);
            assert_eq!(keep, ["lobster", "Coins"]);
            assert!(!keep.iter().any(|name| name == "Big bones"));
            assert!(!keep.iter().any(|name| name == "Rune scimitar"));
        }
    }

    #[test]
    fn bound_arguments_accept_supported_strings_and_arrays() {
        let args = [serde_json::json!({
            "food": "Cake",
            "style": "range",
            "spell": "Wind Strike",
            "ammo": "Iron arrow",
            "weapon": "Maple shortbow",
            "extra": ["Coins", 1, "Rope"]
        })];
        assert_eq!(
            from_args(&args),
            CombatKeepOptions {
                food: "Cake".into(),
                style: "range".into(),
                spell: "Wind Strike".into(),
                ammo: "Iron arrow".into(),
                weapon: "Maple shortbow".into(),
                extra: vec!["Coins".into(), "Rope".into()],
            }
        );
    }
}
