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
mod tests {
    use crate::arrival::arrived;
    use crate::tile::Tile;

    #[test]
    fn arrived_on_tile_or_adjacent_if_solid() {
        let a = Tile {
            x: 10,
            z: 10,
            level: 0,
        };
        assert!(arrived(a, a, true));
        assert!(!arrived(
            a,
            Tile {
                x: 12,
                z: 10,
                level: 0
            },
            true
        ));
        assert!(arrived(
            a,
            Tile {
                x: 10,
                z: 11,
                level: 0
            },
            false
        ));
        assert!(!arrived(
            a,
            Tile {
                x: 10,
                z: 11,
                level: 0
            },
            true
        ));
    }
}
