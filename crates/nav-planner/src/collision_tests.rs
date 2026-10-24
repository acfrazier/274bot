use super::*;

#[test]
fn attach_flags_then_drop_leaves_walk_intact() {
    let flags = vec![CollisionFlag::W_N as u32; 4];
    let (walk, blocked) = pack_walk(&flags);
    let mut c = WorldCollision {
        origin: WorldTile {
            x: 0,
            z: 0,
            level: 0,
        },
        width: 1,
        height: 1,
        walk,
        blocked,
        flags: None,
    };
    let w = c.walkable_word(0, 0, 0);
    c.attach_flags(flags);
    assert!(c.flags.is_some());
    assert_eq!(c.walkable_word(0, 0, 0), w);
    c.drop_flags();
    assert!(c.flags.is_none());
    assert_eq!(c.walkable_word(0, 0, 0), w);
}

/// A level-0 world at (3200,3200) with one flag word per tile. The
/// bake is one plane per level, so planes 1..=3 stay empty. The flags
/// sidecar stays loaded so `flag()` reads the raw words.
fn flag_world(flags: Vec<u32>) -> WorldCollision {
    let plane = flags.len();
    let mut padded = vec![0u32; 4 * plane];
    padded[..plane].copy_from_slice(&flags);
    let (walk, blocked) = pack_walk(&padded);
    WorldCollision {
        origin: WorldTile {
            x: 3200,
            z: 3200,
            level: 0,
        },
        width: plane,
        height: 1,
        walk,
        blocked,
        flags: Some(padded),
    }
}

#[test]
fn nearest_walkable_at_grid_margin_keeps_the_blind_neighbor() {
    let wc = flag_world(vec![0]);
    let door = WorldTile {
        x: 3200,
        z: 3200,
        level: 0,
    };
    let outside = wc.nearest_walkable(door, -1, 0);

    assert_eq!(
        outside,
        WorldTile {
            x: 3199,
            z: 3200,
            level: 0,
        }
    );
    assert!(
        !wc.walkable(outside),
        "the edge fallback is a blind neighbour outside the baked grid"
    );
}

#[test]
fn face_flag_tile_is_standable_but_not_walkable() {
    let wc = flag_world(vec![CollisionFlag::W_N as u32]);
    let t = WorldTile {
        x: 3200,
        z: 3200,
        level: 0,
    };
    // A directional face flag (the closed door's W_N) does not block
    // standing, but the blanket walkable() still rejects the tile.
    assert!(wc.standable(t));
    assert!(!wc.walkable(t));
}

#[test]
fn footprint_and_ground_blocks_are_not_standable() {
    let wc = flag_world(vec![
        CollisionFlag::WR_GRND as u32,
        CollisionFlag::WALK_SCENERY as u32,
        0,
    ]);
    let t = |x: i32| WorldTile {
        x: 3200 + x,
        z: 3200,
        level: 0,
    };
    // A ground block and a scenery footprint disqualify standing just
    // like walking.
    assert!(!wc.standable(t(0)));
    assert!(!wc.walkable(t(0)));
    assert!(!wc.standable(t(1)));
    assert!(!wc.walkable(t(1)));
    // A clear tile stays both standable and walkable.
    assert!(wc.standable(t(2)));
    assert!(wc.walkable(t(2)));
}

#[test]
fn range_face_flags_do_not_disqualify_standing() {
    // V_N (a blockrange wall's range stamp) is a face flag like W_N:
    // standable, never a footprint block.
    let wc = flag_world(vec![CollisionFlag::V_N as u32]);
    let t = WorldTile {
        x: 3200,
        z: 3200,
        level: 0,
    };
    assert!(wc.standable(t));
}

#[test]
fn standable_rejects_out_of_grid() {
    let wc = flag_world(vec![0u32]);
    assert!(!wc.standable(WorldTile {
        x: 3199,
        z: 3200,
        level: 0
    }));
    // The bake is one plane per level; other levels are their own
    // empty planes (empty off-level used to look like "no flags" and
    // flood upstairs).
    assert!(wc.standable(WorldTile {
        x: 3200,
        z: 3200,
        level: 1
    }));
}

#[test]
fn off_level_is_an_empty_plane_not_level0_reuse() {
    let wc = flag_world(vec![CollisionFlag::WALK_SCENERY as u32]);
    let l0 = WorldTile {
        x: 3200,
        z: 3200,
        level: 0,
    };
    let l1 = WorldTile {
        x: 3200,
        z: 3200,
        level: 1,
    };
    // The level-1 plane is its own empty plane: the L0 cell is never
    // reused (the old `let _ = level` is gone).
    assert_eq!(wc.flag(l1.x, l1.z, l1.level), 0);
    assert_ne!(wc.flag(l0.x, l0.z, l0.level), 0);
    assert_eq!(wc.walkable_word(l1.x, l1.z, l1.level), 0);
    // An empty plane has no walk-block flags, so it walks and stands.
    assert!(wc.walkable(l1));
    assert!(wc.standable(l1));
    // Out-of-range levels are empty, never a reuse or a panic.
    assert_eq!(wc.flag(l1.x, l1.z, 4), 0);
    assert_eq!(wc.walkable_word(l1.x, l1.z, -1), 0);
    assert!(!wc.walkable(WorldTile {
        x: 3200,
        z: 3200,
        level: 4
    }));
}

#[test]
fn derive_walkable_does_not_seal_a_face_flag_from_every_side() {
    let wc = flag_world(vec![CollisionFlag::W_W as u32]);
    let t = WorldTile {
        x: 3200,
        z: 3200,
        level: 0,
    };
    let word = wc.walkable_word(t.x, t.z, t.level);
    assert_eq!(word & CollisionFlag::W_W as u32, CollisionFlag::W_W as u32);
    assert_eq!(
        word & SQ_BLOCKED,
        0,
        "a west wall must not inject SQ_BLOCKED"
    );
}
