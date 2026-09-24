//! Wilderness zones for searches whose graph has no packed rules.
//!
//! A baked pack carries content-derived zones on
//! [`crate::transport::WildernessRules`]; the router prefers those when
//! present. This table is the empty-graph / e2e fallback, decoded from the
//! r274 `wilderness_zones.dbrow` pairs (the surface band plus the
//! underground dungeon band).
use api::snapshot::WorldTile;

/// One inclusive world AABB of a zone row.
struct Zone {
    x1: i32,
    z1: i32,
    x2: i32,
    z2: i32,
    level1: i32,
    level2: i32,
}

impl Zone {
    fn contains(&self, t: WorldTile) -> bool {
        t.x >= self.x1
            && t.x <= self.x2
            && t.z >= self.z1
            && t.z <= self.z2
            && t.level >= self.level1
            && t.level <= self.level2
    }
}

/// The two `wilderness_zones.dbrow` pairs, decoded from
/// `level_mx_mz_lx_lz` (`x = mx*64+lx`, `z = mz*64+lz`):
/// surface `0_46_55_0_0`–`3_52_99_63_63` → (2944,3520,0)–(3391,6399,3);
/// underground `0_46_155_0_0`–`0_52_199_63_63` → (2944,9920,0)–(3391,12799,0).
const ZONES: [Zone; 2] = [
    Zone {
        x1: 2944,
        z1: 3520,
        x2: 3391,
        z2: 6399,
        level1: 0,
        level2: 3,
    },
    Zone {
        x1: 2944,
        z1: 9920,
        x2: 3391,
        z2: 12799,
        level1: 0,
        level2: 0,
    },
];

/// Whether `t` lies inside any wilderness zone (inclusive edges). Default
/// [`crate::router::find`] refuses to enter wilderness tiles when the
/// search graph has no packed zones; searches with
/// `FindOptions::allow_wilderness` may. Packed graphs use
/// [`crate::transport::WildernessRules::contains`] instead.
pub fn in_wilderness(t: WorldTile) -> bool {
    ZONES.iter().any(|z| z.contains(t))
}

#[cfg(test)]
#[path = "wilderness_tests.rs"]
mod tests;
