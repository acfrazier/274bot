//! Wilderness zones from the r274 `wilderness_zones.dbrow` table: the
//! world AABB pairs `in_wilderness` tests against (the surface band plus
//! the underground dungeon band).

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
/// [`crate::router::find`] refuses to enter wilderness tiles; searches
/// with `FindOptions::allow_wilderness` may.
pub fn in_wilderness(t: WorldTile) -> bool {
    ZONES.iter().any(|z| z.contains(t))
}

#[cfg(test)]
mod tests {
    use super::in_wilderness;
    use api::snapshot::WorldTile;

    #[test]
    fn lumbridge_is_not_wilderness() {
        assert!(!in_wilderness(WorldTile {
            x: 3222,
            z: 3218,
            level: 0
        }));
    }

    #[test]
    fn wildy_ditch_north_is_wilderness() {
        assert!(in_wilderness(WorldTile {
            x: 3100,
            z: 3525,
            level: 0
        }));
    }

    #[test]
    fn surface_zone_edges_and_levels_are_inclusive() {
        // Decoded `0_46_55_0_0`–`3_52_99_63_63`: x 2944..3391,
        // z 3520..6399, level 0..=3.
        assert!(in_wilderness(WorldTile {
            x: 2944,
            z: 3520,
            level: 0
        }));
        assert!(in_wilderness(WorldTile {
            x: 3391,
            z: 6399,
            level: 3
        }));
        assert!(!in_wilderness(WorldTile {
            x: 3392,
            z: 3520,
            level: 0
        }));
        assert!(!in_wilderness(WorldTile {
            x: 2944,
            z: 3519,
            level: 0
        }));
    }

    #[test]
    fn underground_band_covers_the_dungeon_row() {
        // Decoded `0_46_155_0_0`–`0_52_199_63_63`: level 0,
        // x 2944..3391, z 9920..12799.
        assert!(in_wilderness(WorldTile {
            x: 3100,
            z: 10000,
            level: 0
        }));
        assert!(in_wilderness(WorldTile {
            x: 3391,
            z: 12799,
            level: 0
        }));
        assert!(!in_wilderness(WorldTile {
            x: 3100,
            z: 9919,
            level: 0
        }));
        assert!(!in_wilderness(WorldTile {
            x: 3100,
            z: 12800,
            level: 0
        }));
    }
}
