//! Trail pack budgeting, hard-kit status, and the trail bank-stop keep
//! predicate: pure arithmetic and name matching over the caller's own facts.
//! Nothing here reads `host().snapshot`, `trails()`, `items()`,
//! equipment, the bank, `questStatus`, `identify_step`, or `clue_row`, so a
//! missing page or family is not an error of any method.
//!
//! Frozen source of truth: `packPlan.ts` (`trailFoodTarget`,
//! `teleportRuneTarget`, `weaponNeeded`, `casketRewardSlots`), `hardClueKit.ts`
//! (`superantiDoses`, `hardClueKit`), and the `isKeep` predicate of
//! `SolveClue.ts` (`bankFirst`) in
//! `release-0.1.9/reference/rs2b0t-00d39a17e0/`. Where the older preflight
//! arithmetic disagrees with those files, the files control. The four pack
//! helpers widen to `i64`: `perCast = i32::MAX` publishes `42949672940`, not a
//! wrapped or saturated `i32`. The dose and shark sums widen the same way, so a
//! huge stack cannot wrap into a false failure.
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

/// Frozen `hardClueKit` minimums and identities. The kit check reads the
/// caller's facts only: the quest tab, skills, equipment, and the bank stay
/// with the caller who resolved them.
pub const MIN_ATTACK: i32 = 60;

/// Frozen `DDS_IDS`: the poisoned dragon dagger and the unpoisoned local one.
pub const DDS_IDS: [i32; 2] = [1231, 1215];

/// Frozen `SUPERANTI`: `(id, doses)` of the four superantipoison sizes.
/// Ordinary antipoison (`2446`) is not one of them and contributes no dose.
pub const SUPERANTI: [(i32, i64); 4] = [(2448, 4), (181, 3), (183, 2), (185, 1)];

/// Frozen `SHARK_ID`: the only food this check counts.
pub const SHARK_ID: i32 = 385;

/// Frozen `MIN_SHARKS`: fifteen, however the stacks are split.
pub const MIN_SHARKS: i64 = 15;

const READY: &str = "ready";
const ATTACK: &str = "attack";
const LOST_CITY: &str = "lost-city";
const DDS: &str = "dds";
const SUPERANTIPOISON: &str = "superantipoison";
const SHARKS: &str = "sharks";

/// One kit row. The frozen reduce reads `id` and `count` only: a `worn`,
/// `slot`, or `name` key is not an input and is not an error.
pub struct HardKitItem {
    pub id: i32,
    pub count: i32,
}

/// The caller's kit facts as required fields. `attack` is non-negative and
/// `lost_city` is already resolved: the wrapper rejects a missing or
/// wrong-typed field before anything here compares, so an omitted `attack` is
/// never the status `attack` and an omitted `lostCity` is never `lost-city`.
pub struct HardKitInput<'a> {
    pub attack: i32,
    pub lost_city: bool,
    pub items: &'a [HardKitItem],
}

/// The frozen `superantiDoses`, widened: `1073741824` of `2448` is `4294967296`
/// doses, so it is not a wrapped `0`.
pub fn superanti_doses(items: &[HardKitItem]) -> i64 {
    items
        .iter()
        .map(|item| {
            let doses = SUPERANTI
                .iter()
                .find(|(id, _)| *id == item.id)
                .map_or(0, |(_, doses)| *doses);
            i64::from(item.count) * doses
        })
        .sum()
}

/// The frozen shark reduce, widened: two `i32::MAX` stacks are `4294967294`,
/// so they are not a wrapped `-2` under the minimum.
pub fn shark_count(items: &[HardKitItem]) -> i64 {
    items
        .iter()
        .filter(|item| item.id == SHARK_ID)
        .map(|item| i64::from(item.count))
        .sum()
}

/// The frozen `hardClueKit` status, first failure wins: `attack`, then
/// `lost-city`, then `dds`, then `superantipoison`, then `sharks`, else the one
/// ok value `{ status: "ready" }`. A failure is the whole result, with no
/// value: neither sum is ever published.
pub fn hard_clue_kit(input: &HardKitInput<'_>) -> Result<Value, &'static str> {
    if input.attack < MIN_ATTACK {
        return Err(ATTACK);
    }
    if !input.lost_city {
        return Err(LOST_CITY);
    }
    if !input
        .items
        .iter()
        .any(|item| DDS_IDS.contains(&item.id) && item.count > 0)
    {
        return Err(DDS);
    }
    if superanti_doses(input.items) == 0 {
        return Err(SUPERANTIPOISON);
    }
    if shark_count(input.items) < MIN_SHARKS {
        return Err(SHARKS);
    }
    Ok(json!({ "status": READY }))
}

/// Frozen `isKeep` identities, after the frozen ASCII lower: `SPADE_NAME`
/// (`Spade`), the ship-fare `Coins`, the Shantay pass, and the `coordItems`
/// trio. Six names the frozen predicate matches outright, in its own order.
pub const KEEP_NAMES: [&str; 6] = [
    "spade",
    "coins",
    "shantay pass",
    "sextant",
    "watch",
    "chart",
];

/// Frozen `isKeep` substrings, matched anywhere in the lowered name. The
/// breadth is itself frozen: the predicate is the substring, not a scroll, a
/// casket, or a trail-family membership row.
pub const KEEP_SUBSTRINGS: [&str; 2] = ["clue", "casket"];

/// The frozen `isKeep` deposit predicate: the six identities, either
/// substring, or the caller's own additive names by ASCII equality. Nothing is
/// trimmed, so `" spade"` is not `spade`, and no name is food: `Shark` is
/// deposited unless the caller asked for it. `extra` cannot un-keep, and a
/// miss is `{ keep: false }` rather than an error. The Entrana veto and the
/// caller-gathered sets (`protectedNames`, weapon, teleport kit, jungle, row
/// items) stay with the caller who resolved them.
pub fn keep_clue_kit(name: &str, extra: &[String]) -> Value {
    let lowered = name.to_ascii_lowercase();
    let keep = KEEP_NAMES.contains(&lowered.as_str())
        || KEEP_SUBSTRINGS
            .iter()
            .any(|substring| lowered.contains(substring))
        || extra
            .iter()
            .any(|entry| entry.eq_ignore_ascii_case(&lowered));
    json!({ "keep": keep })
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
        assert_eq!(
            casket_reward_slots("trail_clue_medium_sextant004_casket"),
            5
        );
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

    fn kit_items(rows: &[(i32, i32)]) -> Vec<HardKitItem> {
        rows.iter()
            .map(|(id, count)| HardKitItem {
                id: *id,
                count: *count,
            })
            .collect()
    }

    fn kit_status(
        attack: i32,
        lost_city: bool,
        rows: &[(i32, i32)],
    ) -> Result<Value, &'static str> {
        let items = kit_items(rows);
        hard_clue_kit(&HardKitInput {
            attack,
            lost_city,
            items: &items,
        })
    }

    fn ready() -> Result<Value, &'static str> {
        Ok(json!({ "status": "ready" }))
    }

    /// The frozen status order: first failure wins, and that failure is the
    /// whole result.
    #[test]
    fn hard_kit_reports_the_first_frozen_failure() {
        assert_eq!(
            kit_status(60, true, &[(1231, 1), (185, 1), (385, 15)]),
            ready()
        );
        // Attack 60 is enough, `attack: 0` is a number rather than a miss, and
        // attack is decided before Lost City.
        assert_eq!(
            kit_status(0, true, &[(1231, 1), (185, 1), (385, 15)]),
            Err(ATTACK)
        );
        assert_eq!(kit_status(59, false, &[]), Err(ATTACK));
        assert_eq!(kit_status(60, false, &[]), Err(LOST_CITY));
        // An empty or unrelated pack is `dds`, not an empty success.
        assert_eq!(kit_status(60, true, &[]), Err(DDS));
        assert_eq!(kit_status(60, true, &[(1305, 1)]), Err(DDS));
        // A DDS id with no charge is not a dagger we hold.
        assert_eq!(kit_status(60, true, &[(1231, 0)]), Err(DDS));
        assert_eq!(kit_status(60, true, &[(1215, 1)]), Err(SUPERANTIPOISON));
        // Ordinary antipoison is not superantipoison.
        assert_eq!(
            kit_status(60, true, &[(1231, 1), (2446, 4)]),
            Err(SUPERANTIPOISON)
        );
        // One dose is enough, whatever its size, so every catalogued id is in
        // the table.
        for id in [2448, 181, 183, 185] {
            assert_eq!(
                kit_status(60, true, &[(1231, 1), (id, 1), (385, 15)]),
                ready(),
                "{id}"
            );
        }
        // Fourteen sharks fail, fifteen pass however they are split, and other
        // food is not sharks.
        assert_eq!(
            kit_status(60, true, &[(1231, 1), (185, 1), (385, 14)]),
            Err(SHARKS)
        );
        assert_eq!(
            kit_status(60, true, &[(1231, 1), (185, 1), (385, 7), (385, 8)]),
            ready()
        );
        assert_eq!(
            kit_status(60, true, &[(1231, 1), (185, 1), (391, 30)]),
            Err(SHARKS)
        );
    }

    /// The widening lock on both sums: a wrapped `i32` product of one legal
    /// count is a false `superantipoison`, and a wrapped `i32` pair of stacks
    /// is a false `sharks`.
    #[test]
    fn hard_kit_sums_widen_past_i32() {
        assert_eq!(
            superanti_doses(&kit_items(&[(2448, 1), (181, 1), (183, 1), (185, 1)])),
            10
        );
        assert_eq!(superanti_doses(&kit_items(&[(2446, 1), (385, 15)])), 0);
        assert_eq!(
            superanti_doses(&kit_items(&[(2448, 1_073_741_824)])),
            4_294_967_296
        );
        assert_eq!(
            shark_count(&kit_items(&[(385, i32::MAX), (385, i32::MAX)])),
            4_294_967_294
        );
        assert_eq!(
            kit_status(60, true, &[(1231, 1), (2448, 1_073_741_824), (385, 15)]),
            ready()
        );
        assert_eq!(
            kit_status(
                60,
                true,
                &[(1231, 1), (185, 1), (385, i32::MAX), (385, i32::MAX)]
            ),
            ready()
        );
    }

    fn keep(name: &str, extra: &[&str]) -> bool {
        let extra: Vec<String> = extra.iter().map(|entry| (*entry).to_string()).collect();
        keep_clue_kit(name, &extra)["keep"].as_bool().expect("keep")
    }

    /// The six frozen identities, matched after the ASCII lower rather than
    /// folded as Unicode and compared exactly rather than by substring.
    #[test]
    fn keep_matches_the_six_frozen_identities() {
        for name in [
            "spade",
            "Spade",
            "SPADE",
            "coins",
            "Coins",
            "shantay pass",
            "Shantay pass",
            "SHANTAY PASS",
            "sextant",
            "Sextant",
            "watch",
            "Watch",
            "chart",
            "Chart",
        ] {
            assert!(keep(name, &[]), "{name}");
        }
        // Identity is equality: a longer name containing one of the six is not
        // kept by the constant clause.
        for name in [
            "Spade handle",
            "coins pouch",
            "Ring of coins",
            "Sextant stand",
            "Watchtower",
            "Shanty pass",
            "chart table",
            "spades",
        ] {
            assert!(!keep(name, &[]), "{name}");
        }
    }

    /// The frozen breadth: either substring, anywhere, whatever else the name
    /// says. Not an exact scroll, an exact casket, or a membership row.
    #[test]
    fn keep_takes_the_frozen_substrings_wherever_they_land() {
        for name in [
            "clue",
            "Clue scroll",
            "Clue scroll (hard)",
            "trail_clue_hard_sextant028",
            "unclued",
            "casket",
            "Casket",
            "Pirate casket",
            "trail_clue_easy_map001_casket",
        ] {
            assert!(keep(name, &[]), "{name}");
        }
        // Food is never implied, and a scroll-adjacent word is not a scroll.
        for name in [
            "Shark",
            "sharks",
            "Lobster",
            "Rune scimitar",
            "Rope",
            "Machete",
            "Radimus notes",
            "",
        ] {
            assert!(!keep(name, &[]), "{name}");
        }
    }

    /// The caller's own names are additive: ASCII equality, never a substring,
    /// and never able to un-keep a constant or a substring hit. A miss is
    /// `{ keep: false }`, not an error.
    #[test]
    fn keep_extra_is_additive_ascii_equality() {
        assert!(keep("Rune scimitar", &["Rune scimitar"]));
        assert!(keep("RUNE SCIMITAR", &["rune scimitar"]));
        assert!(keep("rune scimitar", &["RUNE SCIMITAR"]));
        // The frozen Entrana veto is not this helper's: a restricted name the
        // caller asked for is kept.
        assert!(keep("Bronze platebody", &["Bronze platebody"]));
        // Equality, not substring, and no widening.
        assert!(!keep("Rune scimitar", &["Rune"]));
        assert!(!keep("Super spade", &["spade"]));
        assert!(!keep("Bronze platebody", &["platebody"]));
        assert!(!keep("Rope", &["Ropes"]));
        // Additive only: extra cannot un-keep a constant or a substring.
        assert!(keep("Clue scroll", &["Shark"]));
        assert!(keep("spade", &[]));
        // `""` is a present entry, and it matches the empty name exactly.
        assert!(keep("", &[""]));
        assert!(keep("Shark", &["Shark"]));
        assert!(keep("Shark", &["shark"]));
    }

    /// Nothing is trimmed, and the fold is ASCII, so a Unicode-only case pair
    /// stays apart.
    #[test]
    fn keep_does_not_trim_and_folds_ascii_only() {
        for name in [
            " spade",
            "spade ",
            " spade ",
            " coins",
            "shantay pass ",
            "Sextant\t",
        ] {
            assert!(!keep(name, &[]), "{name}");
        }
        // `İ` (U+0130) folds to `i̇` under Unicode `toLowerCase`, which is not
        // this lock's fold, and `ı` is not the ASCII `i`.
        assert!(keep("İ", &["İ"]));
        assert!(!keep("İ", &["i̇"]));
        assert!(!keep("ı", &["I"]));
    }

    /// The whole published value, and only it: `keep` is a boolean, so a miss
    /// is `ok` rather than an error.
    #[test]
    fn keep_publishes_the_one_boolean_key() {
        assert_eq!(keep_clue_kit("Spade", &[]), json!({ "keep": true }));
        assert_eq!(keep_clue_kit("Shark", &[]), json!({ "keep": false }));
        assert_eq!(
            keep_clue_kit("", &[]).as_object().map(|value| value.len()),
            Some(1)
        );
    }
}
