//! Rust-owned composition for the compatibility combat keep list.

use api::game_data::GameItem;
#[cfg(feature = "load")]
use api::game_data::SelectedGameData;
#[cfg(feature = "load")]
use serde_json::Value;

#[cfg(feature = "load")]
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct CombatKeepOptions {
    pub(crate) food: String,
    pub(crate) style: String,
    pub(crate) spell: String,
    pub(crate) ammo: String,
    pub(crate) weapon: String,
    pub(crate) extra: Vec<String>,
}

#[cfg(feature = "load")]
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

#[cfg(feature = "load")]
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

/// The alias of the whole food `key` names, when that item has forms: the
/// first item named `key` (ASCII case-insensitive) whose alias is not itself
/// a partial form or a bank note.
fn parent_alias<'a>(items: &'a [GameItem], key: &str) -> Option<&'a str> {
    let alias = items
        .iter()
        .find(|item| {
            item.name
                .as_deref()
                .is_some_and(|name| name.eq_ignore_ascii_case(key))
        })?
        .alias
        .as_deref()?;
    let is_form = ["partial_", "half_", "cert_"]
        .iter()
        .any(|prefix| alias.starts_with(prefix))
        || alias.ends_with("_slice");
    (!is_form).then_some(alias)
}

/// Frozen `FOOD_FORMS` (`api/combat/food.ts:7-17`) by the selected aliases:
/// the whole food, `2/3 cake` / `slice of cake` (`partial_`, `_slice`), the
/// `chocolate slice` of `chocolate_cake`, `1/2 … pizza` (`half_`) and
/// `half a/an … pie` (`half_a_`, `half_an_`).
fn is_form_alias(parent: &str, candidate: &str) -> bool {
    candidate == parent
        || ["partial_", "half_", "half_a_", "half_an_"]
            .iter()
            .any(|prefix| candidate.strip_prefix(prefix) == Some(parent))
        || candidate
            .strip_suffix("_slice")
            .is_some_and(|stem| stem == parent || parent.strip_suffix("_cake") == Some(stem))
}

/// The display names of `parent`'s forms, in item order.
fn form_names<'a>(items: &'a [GameItem], parent: &'a str) -> impl Iterator<Item = &'a str> {
    items.iter().filter_map(move |item| {
        let alias = item.alias.as_deref()?;
        is_form_alias(parent, alias)
            .then_some(item.name.as_deref())
            .flatten()
    })
}

/// Frozen `foodForms(foodName)`: the lowercased names of every form of the
/// named food, else the lowercased name alone.
pub(crate) fn food_forms(items: &[GameItem], food: &str) -> Vec<String> {
    let key = food.trim();
    let Some(parent) = parent_alias(items, key) else {
        return vec![key.to_lowercase()];
    };
    let mut forms: Vec<String> = Vec::new();
    for name in form_names(items, parent) {
        let name = name.to_lowercase();
        if !forms.contains(&name) {
            forms.push(name);
        }
    }
    forms
}

/// The forms of one food, for matching item names without allocating.
/// Aliases are unique and seven patterns name a form, so seven names hold
/// every form.
pub(crate) struct FoodForms<'a> {
    names: [&'a str; 7],
    len: usize,
}

impl<'a> FoodForms<'a> {
    pub(crate) fn new(items: &'a [GameItem], food: &'a str) -> Self {
        let key = food.trim();
        let mut forms = Self {
            names: [""; 7],
            len: 0,
        };
        match parent_alias(items, key) {
            Some(parent) => {
                for name in form_names(items, parent).take(forms.names.len()) {
                    forms.names[forms.len] = name;
                    forms.len += 1;
                }
            }
            None => {
                forms.names[0] = key;
                forms.len = 1;
            }
        }
        forms
    }

    /// Frozen `isFoodItem(name, foodName)` (`food.ts:74-76`).
    pub(crate) fn contains(&self, name: &str) -> bool {
        self.names[..self.len]
            .iter()
            .any(|form| form.eq_ignore_ascii_case(name))
    }
}

#[cfg(all(test, feature = "load"))]
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
