//! Rust-owned food count and fixed heal lookup for supply helpers.

use api::game_data::SelectedGameData;

use crate::keep_list;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvItemName {
    pub name: Option<String>,
}

/// Food form keys for `foodName`, using selected item catalog when present.
pub fn food_forms_for(data: Option<&SelectedGameData>, food_name: &str) -> Vec<String> {
    keep_list::food_forms(data.map_or(&[], |data| data.items()), food_name)
}

/// Frozen `foodCount(items, foodName)` (`food.ts:78-80`): the slots holding
/// any form of the food. Matches borrowed names; allocates nothing.
pub fn food_count(
    data: Option<&SelectedGameData>,
    items: &[InvItemName],
    food_name: &str,
) -> usize {
    let forms = keep_list::FoodForms::new(data.map_or(&[], |data| data.items()), food_name);
    items
        .iter()
        .filter(|row| forms.contains(row.name.as_deref().unwrap_or("")))
        .count()
}

pub fn food_heal_amount(data: Option<&SelectedGameData>, food_name: &str) -> FoodHealOutcome {
    let Some(data) = data else {
        return FoodHealOutcome::MissingSelected;
    };
    let key = food_name.trim();
    if key.is_empty() {
        return FoodHealOutcome::UnknownFood;
    }
    match data.fixed_food_heal(key) {
        Some(heal) => FoodHealOutcome::Ok(heal),
        None => FoodHealOutcome::UnknownFood,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FoodHealOutcome {
    Ok(i32),
    UnknownFood,
    MissingSelected,
}

/// Frozen `DEFAULT_FOOD_HEAL` (`api/combat/food.ts:67`).
pub const DEFAULT_FOOD_HEAL: i32 = 8;

/// Frozen `MIN_EAT_HP` (`api/combat/food.ts:64`): the #465 eat floor.
pub const MIN_EAT_HP: i32 = 5;

/// Frozen v1 `foodHealAmount` (`api/combat/food.ts:86-106`) over the
/// selected facts: an empty name is the default (`:88-90`); then the exact
/// fixed heal (`:91-94`); then the first fixed-heal food whose name contains
/// the key or is contained in it (`:100-104`); else the default (`:105`).
/// The heal table is the selected cache's, not the frozen hand-copied one.
/// The frozen `FOOD_FORMS` step (`:95-99`) gives a partial form its parent's
/// heal because the frozen table lacks some forms; every selected form row
/// (274 and 289) carries its own fixed heal, so the exact step answers it.
/// `None` only when the answer needs selected facts the host does not have.
pub fn frozen_food_heal_amount(data: Option<&SelectedGameData>, food_name: &str) -> Option<i32> {
    let key = food_name.trim();
    if key.is_empty() {
        return Some(DEFAULT_FOOD_HEAL);
    }
    let data = data?;
    if let Some(heal) = data.fixed_food_heal(key) {
        return Some(heal);
    }
    Some(
        data.fixed_food_heals()
            .find(|(name, _)| {
                contains_ignore_ascii_case(key, name) || contains_ignore_ascii_case(name, key)
            })
            .map_or(DEFAULT_FOOD_HEAL, |(_, heal)| heal),
    )
}

/// JS `hay.toLowerCase().includes(needle.toLowerCase())` for the ASCII item
/// names, without the copies.
fn contains_ignore_ascii_case(hay: &str, needle: &str) -> bool {
    needle.is_empty()
        || hay
            .as_bytes()
            .windows(needle.len())
            .any(|window| window.eq_ignore_ascii_case(needle.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use client::io::ClientRevision;

    fn data(revision: ClientRevision) -> std::sync::Arc<SelectedGameData> {
        api::game_data::for_revision(revision).expect("selected game data")
    }

    #[test]
    fn counts_slots_not_stack_qty_both_revisions() {
        for revision in [ClientRevision::R274, ClientRevision::R289] {
            let data = data(revision);
            let items = vec![
                InvItemName {
                    name: Some("Trout".into()),
                },
                InvItemName {
                    name: Some("Trout".into()),
                },
                InvItemName {
                    name: Some("Lobster".into()),
                },
            ];
            assert_eq!(food_count(Some(&data), &items, "Trout"), 2);
            assert_eq!(food_count(Some(&data), &items, "Lobster"), 1);
        }
    }

    #[test]
    fn empty_name_matches_empty_form_key() {
        let data = data(ClientRevision::R274);
        let items = vec![
            InvItemName { name: None },
            InvItemName {
                name: Some("".into()),
            },
        ];
        assert_eq!(food_count(Some(&data), &items, ""), 2);
    }
}
