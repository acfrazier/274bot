//! Rust-owned food count and fixed heal lookup for supply helpers.

use api::game_data::SelectedGameData;

use crate::keep_list;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvItemName {
    pub name: Option<String>,
}

/// Food form keys for `foodName`, using selected item catalog when present.
pub fn food_forms_for(data: Option<&SelectedGameData>, food_name: &str) -> Vec<String> {
    let key = string_food_key(food_name);
    match data {
        Some(data) => keep_list::food_forms(data.items(), &key),
        None => vec![key],
    }
}

pub fn food_count(
    data: Option<&SelectedGameData>,
    items: &[InvItemName],
    food_name: &str,
) -> usize {
    let forms = food_forms_for(data, food_name);
    items
        .iter()
        .filter(|row| row_is_food_slot(row, &forms))
        .count()
}

fn row_is_food_slot(row: &InvItemName, forms: &[String]) -> bool {
    let name = row.name.as_deref().unwrap_or("");
    forms.contains(&name.to_lowercase())
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

fn string_food_key(food_name: &str) -> String {
    // Match v1 `String(foodName).trim().toLowerCase()` including odd coercions
    // handled at the JS marshal boundary before this runs.
    food_name.trim().to_lowercase()
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
