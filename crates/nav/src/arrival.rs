//! Arrival detection: standing on the destination, or adjacent to a solid
//! (unwalkable) destination such as a door or wall tile.

use crate::tile::{chebyshev, Tile};

/// True when the bot has arrived at `dest`: either standing on it, or
/// standing one tile away on the same level while `dest` is solid (so the
/// bot cannot step onto it).
pub fn arrived(here: Tile, dest: Tile, dest_walkable: bool) -> bool {
    here == dest || (!dest_walkable && chebyshev(here, dest) == 1 && here.level == dest.level)
}

#[cfg(test)]
#[path = "arrival_tests.rs"]
mod tests;
