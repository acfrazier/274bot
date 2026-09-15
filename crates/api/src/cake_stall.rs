//! Host-owned East Ardougne Baker stall pins for the bounded cake mapping.
//!
//! Selected 274/289 headed loc type 2561 is `Baker's stall` / `Steal from`
//! at `(2667,3310,0)`, 2×2 `block_walk`. A second same-id stall at
//! `(2655,3311,0)` is not this card's target. LIVE occupancy proves stand
//! `(2668,3312,0)` is walkable. Unknown profiles post nothing extra.
//!
//! These are posted facts only: stall selection, restock and stall-food
//! predicates live with their script consumer in `script::cake_stall`.

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
