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
