//! Trail pack budgeting: pure slot arithmetic over the caller's own numbers.
//! Nothing here reads `host().snapshot`, `trails()`, `items()`, equipment, the
//! bank, `identify_step`, or `clue_row`, so a missing page or family is not an
//! error of this method.
//!
//! Frozen source of truth: `packPlan.ts` in
//! `release-0.1.8/reference/rs2b0t-beecd9126b/` (`trailFoodTarget`,
//! `teleportRuneTarget`, `weaponNeeded`, `casketRewardSlots`). Where the older
//! preflight arithmetic disagrees with that file, the file controls. The four
//! helpers widen to `i64`: `perCast = i32::MAX` publishes `42949672940`, not a
//! wrapped or saturated `i32`.
//!
//! The V8 wrapper is the only production caller.

use serde_json::{json, Value};

/// Hosts size `foodWithdraw()` for sustained combat (20+), which fills the pack
/// and starves the trail kit, the runes especially. Applied inside `food` only:
/// it is not a published field.
pub const TRAIL_FOOD_CAP: i64 = 10;

/// Sextant + watch + chart, fetched after banking when the bank had none.
pub const COORD_TOOL_SLOTS: i64 = 3;

/// Casts of each catalogued teleport a trail carries runes for. Runes stack, so
/// a bigger cast budget costs no extra slot: `food` carries no rune-slot term.
pub const TELEPORT_CASTS: i64 = 20;

/// The caller asked for food and the pack cannot spare a slot: the whole
/// result, with no value. The frozen `trailFoodTarget` returns `0` for the same
/// arithmetic and its bank loop reads that as "withdraw nothing"; the public
/// envelope names the state instead of publishing `ok` food `0`.
pub const NO_ROOM: &str = "no-room";

/// The caller's four budget numbers. An absent field is already `0`: the wire
/// rejects a negative or non-`i32` value instead of clamping it.
pub struct PackBudget {
    pub host_want: i32,
    pub held_food: i32,
    pub free_slots: i32,
    pub reserve_slots: i32,
}

/// The caller's fields as presence, not truthiness. `Some` publishes the field
/// even when it cannot change the output (`perCast: 0`, `weaponName: ""`,
/// `casketAlias: ""`); the wrapper settles presence with `hasOwnProperty`.
pub struct PackPlanInput<'a> {
    pub budget: PackBudget,
    pub per_cast: Option<i32>,
    pub weapon_name: Option<&'a str>,
    pub weapon_in_backpack: bool,
    pub weapon_equipped: bool,
    pub casket_alias: Option<&'a str>,
}

/// The frozen `trailFoodTarget`, widened so a huge room cannot wrap into a
/// false `no-room`: `i32::MAX + i32::MAX` is `4294967294`, not `-2`. Every
/// valid input still yields `0..=10`.
pub fn trail_food_target(budget: &PackBudget) -> i64 {
    let capped = i64::from(budget.host_want).min(TRAIL_FOOD_CAP);
    let spare = (i64::from(budget.free_slots) - i64::from(budget.reserve_slots)).max(0);
    let room = i64::from(budget.held_food) + spare;
    capped.min(room).max(0)
}

/// The frozen `teleportRuneTarget`: `perCast * 20`. `2147483647` is
/// `42949672940`; a present `0` is a target of `0`, not an omitted field.
pub fn teleport_rune_target(per_cast: i32) -> i64 {
    i64::from(per_cast) * TELEPORT_CASTS
}

/// The frozen `weaponNeeded`: a weapon already worn or in the pack is a weapon
/// we have, and an empty name never withdraws one.
pub fn weapon_needed(weapon_name: &str, in_backpack: bool, equipped: bool) -> bool {
    !weapon_name.is_empty() && !in_backpack && !equipped
}

/// The frozen `casketRewardSlots`: hard before medium, and an unrecognised
/// alias still reserves the easy-tier minimum. A count over the landed 71
/// casket aliases is evidence, not a lookup: nothing here reads the family.
pub fn casket_reward_slots(casket_alias: &str) -> i64 {
    if casket_alias.contains("_hard_") {
        6
    } else if casket_alias.contains("_medium_") {
        5
    } else {
        4
    }
}

/// The whole published value, or [`NO_ROOM`]: `coordToolSlots`, `teleportCasts`,
/// and `food` always, and `runeTarget`, `weaponNeeded`, and `rewardSlots` only
/// when the caller asked for them. Nothing outside those six fields is
/// published, and no field is ever `null`.
pub fn pack_plan(input: &PackPlanInput<'_>) -> Result<Value, &'static str> {
    let food = trail_food_target(&input.budget);
    if input.budget.host_want > 0 && food == 0 {
        return Err(NO_ROOM);
    }
    let mut value = json!({
        "coordToolSlots": COORD_TOOL_SLOTS,
        "teleportCasts": TELEPORT_CASTS,
        "food": food,
    });
    if let Some(per_cast) = input.per_cast {
        value["runeTarget"] = json!(teleport_rune_target(per_cast));
    }
    if let Some(weapon_name) = input.weapon_name {
        value["weaponNeeded"] = json!(weapon_needed(
            weapon_name,
            input.weapon_in_backpack,
            input.weapon_equipped,
        ));
    }
    if let Some(casket_alias) = input.casket_alias {
        value["rewardSlots"] = json!(casket_reward_slots(casket_alias));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn budget(host_want: i32, held_food: i32, free_slots: i32, reserve_slots: i32) -> PackBudget {
        PackBudget {
            host_want,
            held_food,
            free_slots,
            reserve_slots,
        }
    }

    /// A bare call: the four numbers, with nothing optional asked for.
    fn plan(
        host_want: i32,
        held_food: i32,
        free_slots: i32,
        reserve_slots: i32,
    ) -> Result<Value, &'static str> {
        pack_plan(&PackPlanInput {
            budget: budget(host_want, held_food, free_slots, reserve_slots),
            per_cast: None,
            weapon_name: None,
            weapon_in_backpack: false,
            weapon_equipped: false,
            casket_alias: None,
        })
    }

    fn food(value: &Result<Value, &'static str>) -> i64 {
        value.as_ref().expect("ok plan")["food"]
            .as_i64()
            .expect("food")
    }

    /// The frozen `trailFoodTarget` cases, with the frozen 20-food default.
    #[test]
    fn food_matches_the_frozen_trail_food_target_cases() {
        assert_eq!(trail_food_target(&budget(20, 0, 22, 0)), TRAIL_FOOD_CAP);
        assert_eq!(trail_food_target(&budget(6, 0, 22, 0)), 6);
        assert_eq!(
            trail_food_target(&budget(20, 0, 5, COORD_TOOL_SLOTS as i32)),
            2
        );
        assert_eq!(trail_food_target(&budget(20, 8, 0, 0)), 8);
        assert_eq!(trail_food_target(&budget(20, 12, 0, 0)), TRAIL_FOOD_CAP);
        // The whole point: a 20-food host leaves room for the teleport runes.
        assert!(
            (28 - 6) - trail_food_target(&budget(20, 0, 28 - 6, 0)) >= 5,
            "the frozen rune-room case"
        );
    }

    #[test]
    fn no_room_is_the_whole_result_under_a_positive_host_want() {
        assert_eq!(plan(20, 0, 0, 0), Err(NO_ROOM));
        assert_eq!(plan(20, 0, 2, 3), Err(NO_ROOM));
        // A caller who wants no food, and a rune-only call, are not no-room.
        assert_eq!(food(&plan(0, 0, 0, 0)), 0);
        assert_eq!(food(&plan(20, 8, 0, 0)), 8);
        assert_eq!(food(&plan(20, 12, 0, 0)), TRAIL_FOOD_CAP);
        assert_eq!(food(&plan(6, 0, 22, 0)), 6);
        assert_eq!(food(&plan(20, 0, 5, 3)), 2);
        assert_eq!(food(&plan(20, 0, 22, 0)), TRAIL_FOOD_CAP);
    }

    /// The widening lock: two products disagree with a wrapping or saturating
    /// `i32`, and a huge room is not a false `no-room`.
    #[test]
    fn the_arithmetic_widens_past_i32() {
        assert_eq!(teleport_rune_target(i32::MAX), 42_949_672_940);
        assert_ne!(teleport_rune_target(i32::MAX), -20);
        assert_ne!(teleport_rune_target(i32::MAX), i64::from(i32::MAX));
        let value = pack_plan(&PackPlanInput {
            budget: budget(20, i32::MAX, i32::MAX, 0),
            per_cast: Some(i32::MAX),
            weapon_name: None,
            weapon_in_backpack: false,
            weapon_equipped: false,
            casket_alias: None,
        })
        .expect("huge room is ok");
        assert_eq!(value["food"], TRAIL_FOOD_CAP);
        assert_eq!(value["runeTarget"], 42_949_672_940_i64);
    }

    #[test]
    fn rune_target_is_twenty_casts_and_a_present_zero_is_a_target() {
        assert_eq!(teleport_rune_target(1), 20);
        assert_eq!(teleport_rune_target(3), 60);
        assert_eq!(teleport_rune_target(0), 0);
        let value = pack_plan(&PackPlanInput {
            budget: budget(0, 0, 0, 0),
            per_cast: Some(0),
            weapon_name: Some(""),
            weapon_in_backpack: false,
            weapon_equipped: false,
            casket_alias: Some(""),
        })
        .expect("rune-only call");
        assert_eq!(value["runeTarget"], 0);
        assert_eq!(value["weaponNeeded"], false);
        assert_eq!(value["rewardSlots"], 4);
    }

    #[test]
    fn weapon_needed_matches_the_frozen_cases() {
        assert!(!weapon_needed("Rune scimitar", false, true));
        assert!(!weapon_needed("Rune scimitar", true, false));
        assert!(weapon_needed("Rune scimitar", false, false));
        assert!(!weapon_needed("", false, false));
        // Nothing is trimmed, so `" "` is a name.
        assert!(weapon_needed(" ", false, false));
    }

    #[test]
    fn reward_slots_check_hard_before_medium_and_never_fold_case() {
        assert_eq!(casket_reward_slots("trail_clue_easy_map001_casket"), 4);
        assert_eq!(casket_reward_slots("trail_clue_medium_sextant004_casket"), 5);
        assert_eq!(casket_reward_slots("trail_clue_hard_sextant001_casket"), 6);
        assert_eq!(casket_reward_slots("mystery_casket"), 4);
        assert_eq!(casket_reward_slots(""), 4);
        assert_eq!(casket_reward_slots("trail_clue_HARD_x_casket"), 4);
        assert_eq!(casket_reward_slots("trail_clue_MEDIUM_x_casket"), 4);
    }

    #[test]
    fn only_asked_for_fields_are_published() {
        let bare = plan(0, 0, 0, 0).expect("packPlan({})");
        assert_eq!(
            bare,
            json!({ "coordToolSlots": 3, "teleportCasts": 20, "food": 0 })
        );
        assert_eq!(
            bare.as_object().map(|value| value.len()),
            Some(3),
            "nothing else is published"
        );
        let asked = pack_plan(&PackPlanInput {
            budget: budget(6, 0, 22, 0),
            per_cast: Some(3),
            weapon_name: Some("Rune scimitar"),
            weapon_in_backpack: true,
            weapon_equipped: false,
            casket_alias: Some("trail_clue_medium_sextant004_casket"),
        })
        .expect("ok plan");
        assert_eq!(
            asked,
            json!({
                "coordToolSlots": 3,
                "teleportCasts": 20,
                "food": 6,
                "runeTarget": 60,
                "weaponNeeded": false,
                "rewardSlots": 5,
            })
        );
    }
}
