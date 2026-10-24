use super::*;
use api::obj_names::LocDefs;
use client::config::LocType;
use std::fs;
use std::path::PathBuf;

/// A scratch directory for one fixture, removed on drop.
struct FixtureDir(PathBuf);

impl FixtureDir {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("274bot-nav-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        FixtureDir(dir)
    }
}

impl Drop for FixtureDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// A one-loc `LocDefs` table.
fn defs(locs: &[LocType]) -> LocDefs {
    LocDefs::from_locs(locs)
}
#[test]
fn collision_bake_marks_wall_and_door_blocked() {
    let fix = FixtureDir::new("wall-and-door");
    let text = "\
==== MAP ====
0 0 0: h1 o6 u48
0 0 1: f1 u48
0 0 2: h1 o6 u50
==== LOC ====
0 0 2: 1530 0 1
";
    fs::write(fix.0.join("m50_50.jm2"), text).unwrap();
    let locs = defs(&[LocType {
        id: 1530,
        blockwalk: true,
        ..LocType::default()
    }]);
    let mut door_ids = HashSet::new();
    door_ids.insert(1530);
    let wc = bake_from_maps(&fix.0, &locs, &door_ids).unwrap();
    // m50_50 local (0,0) is absolute (3200, 3200).
    let open = WorldTile {
        x: 3200,
        z: 3200,
        level: 0,
    };
    let blocked = WorldTile {
        x: 3200,
        z: 3201,
        level: 0,
    };
    let door = WorldTile {
        x: 3200,
        z: 3202,
        level: 0,
    };
    assert!(wc.walkable(open));
    assert!(!wc.walkable(blocked));
    assert!(!wc.walkable(door));
    // f1 stamps WR_GRND; the closed door stamps the W_N wall flag.
    assert_eq!(
        wc.flag(blocked.x, blocked.z, blocked.level) & CollisionFlag::WR_GRND as u32,
        CollisionFlag::WR_GRND as u32
    );
    assert_eq!(
        wc.flag(door.x, door.z, door.level) & CollisionFlag::W_N as u32,
        CollisionFlag::W_N as u32
    );
    // Outside the grid: no flags, not walkable. The bake is one plane
    // per level: this fixture has no level-1 content, so the level-1
    // plane is empty (walkable), never a level-0 reuse.
    assert_eq!(wc.flag(3199, 3200, 0), 0);
    assert_eq!(wc.flag(3200, 3200, 1), 0);
    assert_eq!(wc.flag(3200, 3200, 4), 0);
    assert!(wc.walkable(WorldTile {
        x: 3200,
        z: 3200,
        level: 1
    }));
    assert!(!wc.walkable(WorldTile {
        x: 3199,
        z: 3200,
        level: 0
    }));
}

#[test]
fn f1_is_wr_grnd_f2_link_below_is_not() {
    let fix = FixtureDir::new("f1-vs-f2");
    let text = "\
==== MAP ====
0 0 0: f1 u48
0 0 1: f2 u50
";
    fs::write(fix.0.join("m50_50.jm2"), text).unwrap();
    let wc = bake_from_maps(&fix.0, &defs(&[]), &HashSet::new()).unwrap();
    // f1 (bit 0, BLOCK) stamps WR_GRND; f2 (bit 1, LINK_BELOW) alone
    // never stamps ground.
    assert_ne!(wc.flag(3200, 3200, 0) & CollisionFlag::WR_GRND as u32, 0);
    assert_eq!(wc.flag(3200, 3201, 0), 0);
}

#[test]
fn blockwalk_no_wall_does_not_add_wall() {
    let fix = FixtureDir::new("no-wall-blockwalk-false");
    let text = "\
==== MAP ====
0 0 0: h1 o6 u48
==== LOC ====
0 0 0: 1234 0 0
";
    fs::write(fix.0.join("m50_50.jm2"), text).unwrap();
    let locs = defs(&[LocType {
        id: 1234,
        blockwalk: false,
        ..LocType::default()
    }]);
    let wc = bake_from_maps(&fix.0, &locs, &HashSet::new()).unwrap();
    // Shape 0 (wall straight) with blockwalk=false carries no collision.
    assert_eq!(wc.flag(3200, 3200, 0), 0);
}

#[test]
fn roof_with_blockwalk_stamps_add_loc_like_the_client() {
    let fix = FixtureDir::new("roof-blockwalk");
    let text = "\
==== MAP ====
0 0 0: h1 o6 u48
==== LOC ====
0 0 0: 1234 12 0
";
    fs::write(fix.0.join("m50_50.jm2"), text).unwrap();
    let locs = defs(&[LocType {
        id: 1234,
        width: 1,
        length: 1,
        blockwalk: true,
        ..LocType::default()
    }]);
    let wc = bake_from_maps(&fix.0, &locs, &HashSet::new()).unwrap();
    // The client `addLoc` roof branch (shape >= ROOF_STRAIGHT) stamps a
    // WALK_SCENERY footprint like a centrepiece.
    assert_ne!(
        wc.flag(3200, 3200, 0) & CollisionFlag::WALK_SCENERY as u32,
        0
    );
    assert!(!wc.walkable(WorldTile {
        x: 3200,
        z: 3200,
        level: 0
    }));
}

#[test]
fn ground_decor_needs_active_and_blockwalk() {
    let fix = FixtureDir::new("ground-decor-gates");
    let text = "\
==== MAP ====
0 0 0: h1 o6 u48
==== LOC ====
0 0 1: 1248 22 0
0 0 2: 559 22 0
";
    fs::write(fix.0.join("m50_50.jm2"), text).unwrap();
    let locs = defs(&[
        LocType {
            id: 1248,
            blockwalk: true,
            active: true,
            ..LocType::default()
        },
        LocType {
            id: 559,
            blockwalk: true,
            active: false,
            ..LocType::default()
        },
    ]);
    let wc = bake_from_maps(&fix.0, &locs, &HashSet::new()).unwrap();
    // Ground decor blocks via the client's `block_ground` (WR_GRND)
    // only when both blockwalk and active are set.
    assert_ne!(wc.flag(3200, 3201, 0) & CollisionFlag::WR_GRND as u32, 0);
    assert_eq!(wc.flag(3200, 3202, 0), 0);
}

#[test]
fn level1_map_block_stamps_plane1_not_level0() {
    let fix = FixtureDir::new("l1-map-block");
    let text = "\
==== MAP ====
1 0 0: f1 u48
";
    fs::write(fix.0.join("m50_50.jm2"), text).unwrap();
    let wc = bake_from_maps(&fix.0, &defs(&[]), &HashSet::new()).unwrap();
    // A level-1 MAP block stamps the level-1 plane, never level 0.
    assert_ne!(wc.flag(3200, 3200, 1) & CollisionFlag::WR_GRND as u32, 0);
    assert_eq!(wc.flag(3200, 3200, 0), 0);
}

#[test]
fn link_below_moves_a_level1_block_down_to_level0() {
    let fix = FixtureDir::new("link-below-down");
    let text = "\
==== MAP ====
1 0 0: f3 u48
";
    fs::write(fix.0.join("m50_50.jm2"), text).unwrap();
    let wc = bake_from_maps(&fix.0, &defs(&[]), &HashSet::new()).unwrap();
    // Client `finishBuild`: a level-1 BLOCK tile whose level-1 map
    // flags carry LINK_BELOW stamps `true_level = level - 1` (TS
    // 79-87), landing on level 0's grid.
    assert_ne!(wc.flag(3200, 3200, 0) & CollisionFlag::WR_GRND as u32, 0);
    assert_eq!(wc.flag(3200, 3200, 1) & CollisionFlag::WR_GRND as u32, 0);
}

#[test]
fn link_below_drops_a_level0_block_off_the_grid() {
    let fix = FixtureDir::new("link-below-off");
    let text = "\
==== MAP ====
0 0 0: f1 u48
1 0 0: f2 u48
";
    fs::write(fix.0.join("m50_50.jm2"), text).unwrap();
    let wc = bake_from_maps(&fix.0, &defs(&[]), &HashSet::new()).unwrap();
    // A level-0 BLOCK with LINK_BELOW on the level-1 map resolves to
    // true_level -1: the client never stamps a negative plane.
    assert_eq!(wc.flag(3200, 3200, 0), 0);
}

#[test]
fn link_below_shifts_a_level1_loc_down_to_level0() {
    let fix = FixtureDir::new("link-below-loc-shift");
    let text = "\
==== MAP ====
1 0 0: h1 o5 f2 u48
1 0 1: h1 o5 f2 u48
==== LOC ====
1 0 1: 994 0 3
";
    fs::write(fix.0.join("m50_50.jm2"), text).unwrap();
    let locs = defs(&[LocType {
        id: 994,
        width: 1,
        length: 1,
        blockwalk: true,
        ..LocType::default()
    }]);
    let wc = bake_from_maps(&fix.0, &locs, &HashSet::new()).unwrap();
    // Client `loadLocations`: LINK_BELOW on mapl[1] places a level-1
    // loc on the level-0 collision (the Lumbridge castle battlements
    // and the drawbridge walls stamp exactly this way). A south wall
    // stamps W_S on its tile and W_N one tile north.
    assert_ne!(wc.flag(3200, 3201, 0) & CollisionFlag::W_S as u32, 0);
    assert_ne!(wc.flag(3200, 3200, 0) & CollisionFlag::W_N as u32, 0);
    assert_eq!(wc.flag(3200, 3200, 1), 0);
}

#[test]
fn link_below_drops_a_level0_loc_off_the_grid() {
    let fix = FixtureDir::new("link-below-loc-drop");
    let text = "\
==== MAP ====
0 0 0: h1 f1 u48
1 0 0: h1 f2 u48
==== LOC ====
0 0 0: 1013 10 0
";
    fs::write(fix.0.join("m50_50.jm2"), text).unwrap();
    let locs = defs(&[LocType {
        id: 1013,
        width: 1,
        length: 1,
        blockwalk: true,
        ..LocType::default()
    }]);
    let wc = bake_from_maps(&fix.0, &locs, &HashSet::new()).unwrap();
    // A level-0 loc on a LINK_BELOW tile resolves to current_level -1:
    // the client never stamps a negative plane, so the loc vanishes
    // (the L0 BLOCK f1 also resolves below the grid).
    assert_eq!(wc.flag(3200, 3200, 0), 0);
}

#[test]
fn level1_loc_stamps_its_own_plane_not_level0() {
    let fix = FixtureDir::new("l1-loc");
    let text = "\
==== MAP ====
==== LOC ====
1 0 0: 1234 10 0
";
    fs::write(fix.0.join("m50_50.jm2"), text).unwrap();
    let locs = defs(&[LocType {
        id: 1234,
        width: 1,
        length: 1,
        blockwalk: true,
        ..LocType::default()
    }]);
    let wc = bake_from_maps(&fix.0, &locs, &HashSet::new()).unwrap();
    // A level-1 centrepiece stamps WALK_SCENERY on the level-1 plane;
    // level 0 stays clean (no squash).
    assert_ne!(
        wc.flag(3200, 3200, 1) & CollisionFlag::WALK_SCENERY as u32,
        0
    );
    assert_eq!(wc.flag(3200, 3200, 0), 0);
}

#[test]
fn level1_roof_stays_on_level1() {
    let fix = FixtureDir::new("l1-roof");
    let text = "\
==== MAP ====
==== LOC ====
1 0 0: 1234 12 0
";
    fs::write(fix.0.join("m50_50.jm2"), text).unwrap();
    let locs = defs(&[LocType {
        id: 1234,
        width: 1,
        length: 1,
        blockwalk: true,
        ..LocType::default()
    }]);
    let wc = bake_from_maps(&fix.0, &locs, &HashSet::new()).unwrap();
    // Roofs stamp their own level (ClientBuild `add_loc` on `level`):
    // an L1+ roof stays L1+, never pushed to the ground plane.
    assert_ne!(
        wc.flag(3200, 3200, 1) & CollisionFlag::WALK_SCENERY as u32,
        0
    );
    assert_eq!(wc.flag(3200, 3200, 0), 0);
}

#[test]
fn bake_skips_non_jm2_files_and_merges_the_bbox() {
    let fix = FixtureDir::new("bbox-merge");
    fs::write(fix.0.join("ignore.csv"), "metadata\n").unwrap();
    fs::write(
        fix.0.join("m50_50.jm2"),
        "==== MAP ====\n0 0 0: h1 o6 u48\n",
    )
    .unwrap();
    fs::write(
        fix.0.join("m52_52.jm2"),
        "==== MAP ====\n0 0 0: h1 o6 u48\n",
    )
    .unwrap();
    let wc = bake_from_maps(&fix.0, &defs(&[]), &HashSet::new()).unwrap();
    // Origin is the western/northern corner; both walkable tiles are in.
    assert_eq!(wc.origin.x, 3200);
    assert_eq!(wc.origin.z, 3200);
    assert_eq!(wc.width, 3 * SQUARE);
    assert_eq!(wc.height, 3 * SQUARE);
    assert!(wc.walkable(WorldTile {
        x: 3200,
        z: 3200,
        level: 0
    }));
    assert!(wc.walkable(WorldTile {
        x: 3328,
        z: 3328,
        level: 0
    }));
    // The gap between the two squares stays blocked.
    assert!(!wc.walkable(WorldTile {
        x: 3264,
        z: 3264,
        level: 0
    }));
}

#[test]
fn scenery_footprint_stamps_walk_scenery() {
    let fix = FixtureDir::new("scenery-footprint");
    let text = "\
==== MAP ====
0 0 0: h1 o6 u48
==== LOC ====
0 0 0: 1013 10 0
0 1 0: 602 10 1
";
    fs::write(fix.0.join("m50_50.jm2"), text).unwrap();
    let locs = defs(&[
        LocType {
            id: 1013,
            width: 2,
            length: 1,
            blockwalk: true,
            ..LocType::default()
        },
        LocType {
            id: 602,
            width: 2,
            length: 1,
            blockwalk: true,
            ..LocType::default()
        },
    ]);
    let wc = bake_from_maps(&fix.0, &locs, &HashSet::new()).unwrap();
    let t = |x: i32, z: i32| WorldTile { x, z, level: 0 };
    // Shape 10, angle 0 at (0,0): 2 wide in x -> (3200,3200) and
    // (3201,3200).
    assert!(!wc.walkable(t(3200, 3200)));
    // Shape 10, angle 1 (north) at (1,0): width/length swap -> 1 wide
    // in x, 2 long in z -> (3201,3200) and (3201,3201).
    assert!(!wc.walkable(t(3201, 3200)));
    assert!(!wc.walkable(t(3201, 3201)));
    // The tile west of both footprints is untouched and walkable.
    assert!(wc.walkable(t(3200, 3201)));
    assert_eq!(
        wc.flag(3201, 3200, 0) & CollisionFlag::WALK_SCENERY as u32,
        CollisionFlag::WALK_SCENERY as u32
    );
}

#[test]
fn diagonal_wall_and_ground_decor_stamp_their_flags() {
    let fix = FixtureDir::new("diag-and-decor");
    let text = "\
==== MAP ====
0 0 0: h1 o6 u48
==== LOC ====
0 0 0: 1013 9 0
0 0 1: 1248 22 0
0 0 2: 559 22 0
";
    fs::write(fix.0.join("m50_50.jm2"), text).unwrap();
    let locs = defs(&[
        LocType {
            id: 1013,
            width: 1,
            length: 1,
            blockwalk: true,
            ..LocType::default()
        },
        LocType {
            id: 1248,
            blockwalk: true,
            active: true,
            ..LocType::default()
        },
        LocType {
            id: 559,
            blockwalk: true,
            active: false,
            ..LocType::default()
        },
    ]);
    let wc = bake_from_maps(&fix.0, &locs, &HashSet::new()).unwrap();
    let t = |x: i32, z: i32| WorldTile { x, z, level: 0 };
    // Shape 9 (wall diagonal) is a scenery-footprint loc like the
    // client: WALK_SCENERY on its tile.
    assert!(!wc.walkable(t(3200, 3200)));
    // A blockwalk && active ground decor blocks via the client's
    // `block_ground` (WR_GRND), which is in the walk mask.
    assert_eq!(
        wc.flag(3200, 3201, 0) & CollisionFlag::WR_GRND as u32,
        CollisionFlag::WR_GRND as u32
    );
    assert!(!wc.walkable(t(3200, 3201)));
    // The inactive ground decor loc (same shape, `active` gates it) is
    // not stamped and stays walkable.
    assert_eq!(wc.flag(3200, 3202, 0), 0);
    assert!(wc.walkable(t(3200, 3202)));
}

#[test]
fn bake_rejects_mapsquare_without_a_map_section() {
    let fix = FixtureDir::new("no-map");
    fs::write(fix.0.join("m50_50.jm2"), "==== NPC ====\n0 0 0: 1234\n").unwrap();
    assert!(matches!(
        bake_from_maps(&fix.0, &defs(&[]), &HashSet::new()),
        Err(crate::GenerationError::Pack(PackError::BadLength(_)))
    ));
}

#[test]
fn bake_rejects_empty_maps_dir() {
    let fix = FixtureDir::new("empty");
    assert!(matches!(
        bake_from_maps(&fix.0, &defs(&[]), &HashSet::new()),
        Err(crate::GenerationError::Pack(PackError::BadLength(_)))
    ));
}
