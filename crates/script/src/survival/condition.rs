//! Derive compact condition facts from one complete snapshot.

use api::condition::{
    BotCondition, ConditionError, CureDoses, FoodSummary, PoisonState, SessionMark,
};
use api::snapshot::GameSnapshot;

use crate::combat::tables::{CombatTables, FoodFact};

/// Derive the local player's condition from a complete snapshot and selected tables.
///
/// The caller owns poison state and replaces the returned unknown default with
/// its session reducer state. This function reads inventory rows without copying them.
pub fn derive(
    snapshot: &GameSnapshot,
    tables: &CombatTables,
) -> Result<BotCondition, ConditionError> {
    if snapshot.local_player().is_none() {
        return Err(ConditionError::NoLocalPlayer);
    }
    if snapshot.inventory_packet_generation().is_none() {
        return Err(ConditionError::InventoryUnavailable);
    }

    let mut hitpoints = snapshot.stats().iter().filter(|row| row.index == 3);
    let row = hitpoints.next().ok_or(ConditionError::StatsUnavailable)?;
    if hitpoints.next().is_some() || !row.used {
        return Err(ConditionError::StatsUnavailable);
    }
    let hp = u8::try_from(row.effective).map_err(|_| ConditionError::StatsUnavailable)?;
    let hp_max = u8::try_from(row.base).map_err(|_| ConditionError::StatsUnavailable)?;

    let mut condition = BotCondition {
        tick: snapshot.tick(),
        session: if hp == 0 {
            SessionMark::Dead
        } else {
            SessionMark::Live
        },
        hp,
        hp_max,
        hp_known: true,
        poison: PoisonState::Unknown {
            since: snapshot.tick() as u16,
        },
        food: FoodSummary::default(),
        cures: CureDoses::default(),
    };

    let selected = tables.selected();
    let fruit_id = selected
        .item_by_alias("macro_triffidfruit")
        .map(|item| item.id);
    let mut first_food_kind = None;
    for item in snapshot.inventory() {
        if item.count <= 0 {
            continue;
        }
        let count = item.count as u32;
        if let Some(food) = tables.food(item.def.id).filter(|food| ordinary(food)) {
            let count_u8 = u8::try_from(count).unwrap_or(u8::MAX);
            condition.food.ordinary_count = condition.food.ordinary_count.saturating_add(count_u8);
            let heal = u64::try_from(food.heal).unwrap_or(0);
            let total = u64::from(count).saturating_mul(heal);
            condition.food.ordinary_heal_total = condition
                .food
                .ordinary_heal_total
                .saturating_add(u16::try_from(total).unwrap_or(u16::MAX));
            let heal = u8::try_from(food.heal).unwrap_or(u8::MAX);
            condition.food.best_heal = condition.food.best_heal.max(heal);
            condition.food.smallest_heal = if condition.food.smallest_heal == 0 {
                heal
            } else {
                condition.food.smallest_heal.min(heal)
            };
            if first_food_kind.is_some_and(|id| id != item.def.id) {
                condition.food.more_kinds = true;
            } else {
                first_food_kind = Some(item.def.id);
            }
            if combo_food(food) {
                condition.food.combo_count = condition.food.combo_count.saturating_add(count_u8);
            }
        }

        if fruit_id == Some(item.def.id) {
            condition.cures.strange_fruit = condition
                .cures
                .strange_fruit
                .saturating_add(u8::try_from(count).unwrap_or(u8::MAX));
        }

        for fact in selected
            .consumption_facts()
            .iter()
            .filter(|fact| fact.item.id == item.def.id)
        {
            let Some(doses) = fact.dose_count.filter(|doses| *doses > 0) else {
                continue;
            };
            let total_doses = count.saturating_mul(u32::from(doses));
            let held = u8::try_from(total_doses).unwrap_or(u8::MAX);
            match fact.dose_family.as_deref() {
                Some("potion_antipoison") => {
                    condition.cures.antipoison = condition.cures.antipoison.saturating_add(held);
                }
                Some("potion_2antipoison") => {
                    condition.cures.super_antipoison =
                        condition.cures.super_antipoison.saturating_add(held);
                }
                _ => {}
            }
        }
    }

    Ok(condition)
}

fn ordinary(food: &FoodFact) -> bool {
    food.eat_delay_arg == Some(2)
        && food.message_delay.is_none_or(|delay| delay <= 0)
        && food.heal > 0
}

fn combo_food(food: &FoodFact) -> bool {
    food.eat_delay_arg
        .zip(food.skill_delay_arg)
        .is_some_and(|(eat_delay, skill_delay)| skill_delay < eat_delay)
}
