//! Arrival detection: standing on the destination, or adjacent to a solid
//! (unwalkable) destination such as a door or wall tile.

use crate::tile::{chebyshev, Tile};
/// Native walk arrival rule. `Reach` preserves the traditional requirement
/// that the destination itself be reachable (or a solid target be approached);
/// `Area` accepts any standable tile in the requested Chebyshev radius.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ArrivalKind {
    #[default]
    Reach,
    Area,
}

/// Area arrival has no path-to-centre requirement. The caller supplies
/// current, loaded-scene standability; packed route goals are not proof.
pub fn arrived_in_area(
    here: api::snapshot::WorldTile,
    centre: api::snapshot::WorldTile,
    radius: i32,
    standable: impl FnOnce(api::snapshot::WorldTile) -> bool,
) -> bool {
    let Ok(radius) = u32::try_from(radius) else {
        return false;
    };
    here.level == centre.level
        && here.x.abs_diff(centre.x).max(here.z.abs_diff(centre.z)) <= radius
        && standable(here)
}

/// True when the bot has arrived at `dest`: either standing on it, or
/// standing one tile away on the same level while `dest` is solid (so the
/// bot cannot step onto it).
pub fn arrived(here: Tile, dest: Tile, dest_walkable: bool) -> bool {
    here == dest || (!dest_walkable && chebyshev(here, dest) == 1 && here.level == dest.level)
}

#[cfg(test)]
#[path = "arrival_tests.rs"]
mod tests;
