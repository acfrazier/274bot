//! Static `area_allow_loc_add` subset baked as the `274L` canlight sidecar.
//!
//! The bit means the immutable content would allow a loc add: the tile is
//! on a loaded mapsquare, outside every `bank_zones` AABB, has no baked
//! engine FLOOR / FLOOR_DECORATION / LOC analogue (`WR_GRND` /
//! `WALK_SCENERY`), and is not covered by a static **active** map-loc
//! footprint (MAP_LOCADDUNSAFE rotation). It is not walkable, not a
//! handwritten bank box, and not a complete live predicate. Live loc
//! add/del/morph, open doors, F2P members tiles, and multi-tile live
//! footprints stay residual. Static map locs are a conservative baseline:
//! a later removal can leave a tile excluded.

use api::snapshot::WorldTile;

use crate::collision::WorldCollision;

/// Content-root-relative bank_zones dbrow both revisions use.
pub const BANK_ZONES_REL: &str = "scripts/skill_firemaking/configs/bank_zones.dbrow";

const LEVELS: i32 = 4;

/// Walk-buffer index of `tile`, `None` outside the grid or on unknown levels.
pub fn canlight_cell_index(collision: &WorldCollision, tile: WorldTile) -> Option<usize> {
    if !(0..LEVELS).contains(&tile.level) {
        return None;
    }
    let lx = tile.x - collision.origin.x;
    let lz = tile.z - collision.origin.z;
    if lx < 0 || lz < 0 {
        return None;
    }
    let (lx, lz) = (lx as usize, lz as usize);
    if lx >= collision.width || lz >= collision.height {
        return None;
    }
    Some(tile.level as usize * collision.width * collision.height + lz * collision.width + lx)
}

/// Whether `tile` is set in a canlight bitset.
pub fn canlight_at(bits: &[u64], collision: &WorldCollision, tile: WorldTile) -> bool {
    let Some(idx) = canlight_cell_index(collision, tile) else {
        return false;
    };
    bits.get(idx / 64)
        .is_some_and(|word| word & (1 << (idx % 64)) != 0)
}

#[cfg(test)]
#[path = "canlight_tests.rs"]
mod tests;
