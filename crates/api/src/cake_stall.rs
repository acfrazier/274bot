//! Host-owned East Ardougne Baker stall pins for the bounded cake mapping.
//!
//! Selected 274/289 headed loc type 2561 is `Baker's stall` / `Steal from`
//! at `(2667,3310,0)`, 2×2 `block_walk`. A second same-id stall at
//! `(2655,3311,0)` is not this card's target. LIVE occupancy proves stand
//! `(2668,3312,0)` is walkable. Unknown profiles post nothing extra.

use crate::snapshot::WorldTile;

const fn t(x: i32, z: i32) -> WorldTile {
    WorldTile { x, z, level: 0 }
}

/// Selected Baker stall: loc identity plus stand/flee tiles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BakerStall {
    pub loc_id: i32,
    pub name: &'static str,
    pub op: &'static str,
    pub stall: WorldTile,
    pub stand: WorldTile,
    pub stand_alt: WorldTile,
    pub flee: WorldTile,
}

/// Posted selected-world Baker stall. Same pins on 274 and 289.
pub const BAKER_STALL: BakerStall = BakerStall {
    loc_id: 2561,
    name: "Baker's stall",
    op: "Steal from",
    stall: t(2667, 3310),
    stand: t(2668, 3312),
    stand_alt: t(2669, 3310),
    flee: t(2655, 3298),
};

/// Stall food names counted for restock. Chocolate cake is not a stall food.
pub const CAKE_ITEM_NAMES: &[&str] = &["Cake", "Bread", "Chocolate slice"];

/// Neighborhood around the selected stall tile used to reject the other stall.
pub const TARGET_RADIUS: i32 = 3;

/// One posted loc row considered for stall selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StallLoc<'a> {
    pub id: i32,
    pub name: Option<&'a str>,
    pub x: i32,
    pub z: i32,
    pub level: i32,
    pub distance: i32,
    pub actions: &'a [&'a str],
}

fn names_match(actual: Option<&str>, want: &str) -> bool {
    actual
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .is_some_and(|name| name.eq_ignore_ascii_case(want.trim()))
}

fn has_op(actions: &[&str], want: &str) -> bool {
    let want = want.trim();
    actions.iter().any(|action| {
        let action = action.trim();
        !action.is_empty() && action != "hidden" && action.eq_ignore_ascii_case(want)
    })
}

fn chebyshev(a: WorldTile, x: i32, z: i32, level: i32) -> i32 {
    if a.level != level {
        return i32::MAX;
    }
    (a.x - x).abs().max((a.z - z).abs())
}

/// Pick the matching Baker stall: name, steal op, selected loc id, and
/// neighborhood of the posted stall tile. Closest *qualifying* row wins.
/// Missing facts yield no target.
pub fn select_baker_stall<'a>(
    facts: Option<&BakerStall>,
    locs: &'a [StallLoc<'a>],
) -> Option<&'a StallLoc<'a>> {
    let facts = facts?;
    locs.iter()
        .filter(|loc| {
            loc.id == facts.loc_id
                && names_match(loc.name, facts.name)
                && has_op(loc.actions, facts.op)
                && chebyshev(facts.stall, loc.x, loc.z, loc.level) <= TARGET_RADIUS
        })
        .min_by_key(|loc| loc.distance)
}

/// Restock is owed only when the pack can still take stall food.
pub fn needs_cake_restock(carried: i32, target: Option<i32>, pack_full: bool) -> bool {
    if pack_full {
        return false;
    }
    carried < target.unwrap_or(1)
}

/// Exact stall-food names; chocolate cake does not match chocolate slice.
pub fn counts_as_stall_food(name: &str) -> bool {
    CAKE_ITEM_NAMES
        .iter()
        .any(|want| name.trim().eq_ignore_ascii_case(want))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loc<'a>(
        id: i32,
        name: Option<&'a str>,
        x: i32,
        z: i32,
        distance: i32,
        actions: &'a [&'a str],
    ) -> StallLoc<'a> {
        StallLoc {
            id,
            name,
            x,
            z,
            level: 0,
            distance,
            actions,
        }
    }

    #[test]
    fn selected_pins_match_headed_274_289_loc_and_live_stand() {
        assert_eq!(BAKER_STALL.loc_id, 2561);
        assert_eq!(BAKER_STALL.name, "Baker's stall");
        assert_eq!(BAKER_STALL.op, "Steal from");
        assert_eq!(BAKER_STALL.stall, t(2667, 3310));
        assert_eq!(BAKER_STALL.stand, t(2668, 3312));
        assert_eq!(BAKER_STALL.stand_alt, t(2669, 3310));
        assert_eq!(BAKER_STALL.flee, t(2655, 3298));
        assert_eq!(CAKE_ITEM_NAMES, ["Cake", "Bread", "Chocolate slice"]);
    }

    #[test]
    fn closer_door_does_not_beat_farther_qualifying_stall() {
        let steal = ["Steal from"];
        let open = ["Open"];
        let locs = [
            loc(1530, Some("Door"), 2668, 3312, 0, &open),
            loc(2561, Some("Baker's stall"), 2667, 3310, 2, &steal),
        ];
        let hit = select_baker_stall(Some(&BAKER_STALL), &locs).expect("stall");
        assert_eq!(hit.id, 2561);
        assert_eq!((hit.x, hit.z), (2667, 3310));
    }

    #[test]
    fn other_same_id_stall_is_not_the_selected_target() {
        let steal = ["Steal from"];
        let locs = [
            loc(2561, Some("Baker's stall"), 2655, 3311, 1, &steal),
            loc(2561, Some("Baker's stall"), 2667, 3310, 11, &steal),
        ];
        let hit = select_baker_stall(Some(&BAKER_STALL), &locs).expect("market stall");
        assert_eq!((hit.x, hit.z, hit.distance), (2667, 3310, 11));
    }

    #[test]
    fn depleted_or_wrong_op_or_missing_facts_select_nothing() {
        let examine = ["Examine"];
        let steal = ["Steal from"];
        let depleted = [loc(2561, Some("Baker's stall"), 2667, 3310, 1, &examine)];
        assert!(select_baker_stall(Some(&BAKER_STALL), &depleted).is_none());
        let wrong_name = [loc(2561, Some("Bakery stall"), 2667, 3310, 1, &steal)];
        assert!(select_baker_stall(Some(&BAKER_STALL), &wrong_name).is_none());
        let wrong_id = [loc(99, Some("Baker's stall"), 2667, 3310, 1, &steal)];
        assert!(select_baker_stall(Some(&BAKER_STALL), &wrong_id).is_none());
        assert!(select_baker_stall(None, &depleted).is_none());
        assert!(select_baker_stall(Some(&BAKER_STALL), &[]).is_none());
    }

    #[test]
    fn restock_honors_capacity_and_requested_count() {
        assert!(needs_cake_restock(0, Some(1), false));
        assert!(!needs_cake_restock(0, Some(1), true));
        assert!(!needs_cake_restock(0, Some(0), false));
        assert!(!needs_cake_restock(4, Some(4), false));
        assert!(needs_cake_restock(0, None, false));
        assert!(counts_as_stall_food("chocolate slice"));
        assert!(counts_as_stall_food("Cake"));
        assert!(!counts_as_stall_food("Chocolate cake"));
        assert!(!counts_as_stall_food("Bread roll"));
    }
}
