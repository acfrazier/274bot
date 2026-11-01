//! `loc_can_operate` against the engine size-1 reach strategy
//! (`lostcity-289/engine/src/engine/routefinder/ReachStrategy.ts`) and the
//! client `CollisionMap::test_wall` / `test_w_decor`.
//!
//! The engine oracle below is a literal transliteration of `reachWall1`
//! (`ReachStrategy.ts:98-173`) and `reachWallDecor1`
//! (`ReachStrategy.ts:254-280`) over the engine's own flag values
//! (`flags.ts`). The client words use the client `CollisionFlag` layout. The
//! two layouts agree on the wall bits (`0x1..=0x80`), `LOC` (`0x100`) and
//! `FLOOR` (`0x200000`), which are the only bits a standable tile can carry.

use api::query::loc_approach::{
    loc_candidate_stands, loc_can_operate, loc_reach_strategy, LocOperateQuery, LocReachStrategy,
};
use api::snapshot::WorldTile;
use client::dash3d::{CollisionFlag, CollisionMap};

// Engine `flags.ts` values.
const E_WALL_NORTH: u32 = 0x2;
const E_WALL_EAST: u32 = 0x8;
const E_WALL_SOUTH: u32 = 0x20;
const E_WALL_WEST: u32 = 0x80;
const E_BLOCK_WEST: u32 = 0x240108;
const E_BLOCK_EAST: u32 = 0x240180;
const E_BLOCK_SOUTH: u32 = 0x240102;
const E_BLOCK_NORTH: u32 = 0x240120;

fn tile(x: i32, z: i32) -> WorldTile {
    WorldTile { x, z, level: 0 }
}

/// `ReachStrategy.reachWall1` with `dest` at the origin and `src` at
/// `(sx, sz)` relative to it. Angles: 0 west, 1 north, 2 east, 3 south.
fn engine_wall1(flags: u32, sx: i32, sz: i32, shape: u8, angle: u8) -> bool {
    let open = |mask: u32| flags & mask == 0;
    match shape {
        0 => match angle {
            0 => {
                (sx == -1 && sz == 0)
                    || (sx == 0 && sz == 1 && open(E_BLOCK_NORTH))
                    || (sx == 0 && sz == -1 && open(E_BLOCK_SOUTH))
            }
            1 => {
                (sx == 0 && sz == 1)
                    || (sx == -1 && sz == 0 && open(E_BLOCK_WEST))
                    || (sx == 1 && sz == 0 && open(E_BLOCK_EAST))
            }
            2 => {
                (sx == 1 && sz == 0)
                    || (sx == 0 && sz == 1 && open(E_BLOCK_NORTH))
                    || (sx == 0 && sz == -1 && open(E_BLOCK_SOUTH))
            }
            _ => {
                (sx == 0 && sz == -1)
                    || (sx == -1 && sz == 0 && open(E_BLOCK_WEST))
                    || (sx == 1 && sz == 0 && open(E_BLOCK_EAST))
            }
        },
        2 => match angle {
            0 => {
                (sx == -1 && sz == 0)
                    || (sx == 0 && sz == 1)
                    || (sx == 1 && sz == 0 && open(E_BLOCK_EAST))
                    || (sx == 0 && sz == -1 && open(E_BLOCK_SOUTH))
            }
            1 => {
                (sx == -1 && sz == 0 && open(E_BLOCK_WEST))
                    || (sx == 0 && sz == 1)
                    || (sx == 1 && sz == 0)
                    || (sx == 0 && sz == -1 && open(E_BLOCK_SOUTH))
            }
            2 => {
                (sx == -1 && sz == 0 && open(E_BLOCK_WEST))
                    || (sx == 0 && sz == 1 && open(E_BLOCK_NORTH))
                    || (sx == 1 && sz == 0)
                    || (sx == 0 && sz == -1)
            }
            _ => {
                (sx == -1 && sz == 0)
                    || (sx == 0 && sz == 1 && open(E_BLOCK_NORTH))
                    || (sx == 1 && sz == 0 && open(E_BLOCK_EAST))
                    || (sx == 0 && sz == -1)
            }
        },
        9 => {
            (sx == 0 && sz == 1 && flags & E_WALL_SOUTH == 0)
                || (sx == 0 && sz == -1 && flags & E_WALL_NORTH == 0)
                || (sx == -1 && sz == 0 && flags & E_WALL_EAST == 0)
                || (sx == 1 && sz == 0 && flags & E_WALL_WEST == 0)
        }
        // Shapes 1 and 3 have no branch: `reachWall1` returns false.
        _ => false,
    }
}

/// `ReachStrategy.reachWallDecor1`, including `alteredRotation`.
fn engine_wall_decor1(flags: u32, sx: i32, sz: i32, shape: u8, angle: u8) -> bool {
    let open = |mask: u32| flags & mask == 0;
    match shape {
        6 | 7 => {
            let rotation = if shape == 7 { (angle + 2) & 3 } else { angle };
            match rotation {
                0 => {
                    (sx == 1 && sz == 0 && open(E_WALL_WEST))
                        || (sx == 0 && sz == -1 && open(E_WALL_NORTH))
                }
                1 => {
                    (sx == -1 && sz == 0 && open(E_WALL_EAST))
                        || (sx == 0 && sz == -1 && open(E_WALL_NORTH))
                }
                2 => {
                    (sx == -1 && sz == 0 && open(E_WALL_EAST))
                        || (sx == 0 && sz == 1 && open(E_WALL_SOUTH))
                }
                _ => {
                    (sx == 1 && sz == 0 && open(E_WALL_WEST))
                        || (sx == 0 && sz == 1 && open(E_WALL_SOUTH))
                }
            }
        }
        8 => {
            (sx == 0 && sz == 1 && open(E_WALL_SOUTH))
                || (sx == 0 && sz == -1 && open(E_WALL_NORTH))
                || (sx == -1 && sz == 0 && open(E_WALL_EAST))
                || (sx == 1 && sz == 0 && open(E_WALL_WEST))
        }
        // Shapes 4 and 5 have no branch: `reachWallDecor1` returns false.
        _ => false,
    }
}

fn engine_reached(flags: u32, sx: i32, sz: i32, shape: u8, angle: u8) -> bool {
    if sx == 0 && sz == 0 {
        return true;
    }
    match shape {
        0..=3 | 9 => engine_wall1(flags, sx, sz, shape, angle),
        4..=8 => engine_wall_decor1(flags, sx, sz, shape, angle),
        _ => false,
    }
}

/// Every wall-bit combination, with and without the `LOC` and `FLOOR` bits.
fn flag_words() -> impl Iterator<Item = (u32, i32)> {
    (0u32..256).flat_map(|low| {
        [0u32, 0x100, 0x20_0000, 0x20_0100]
            .into_iter()
            .map(move |high| (low | high, (low | high) as i32))
    })
}

fn ours(shape: u8, angle: u8, sx: i32, sz: i32, flags: i32) -> Option<bool> {
    loc_can_operate(LocOperateQuery {
        origin: tile(100, 100),
        from: tile(100 + sx, 100 + sz),
        shape,
        angle,
        width: 1,
        length: 1,
        force_approach: 0,
        flags,
    })
}

#[test]
fn walls_and_decorations_match_the_engine_for_every_shape_rotation_and_flag_word() {
    let mut checked = 0usize;
    for shape in 0u8..=9 {
        for angle in 0u8..4 {
            for sx in -2..=2 {
                for sz in -2..=2 {
                    for (engine_flags, client_flags) in flag_words() {
                        let expected = engine_reached(engine_flags, sx, sz, shape, angle);
                        assert_eq!(
                            ours(shape, angle, sx, sz, client_flags),
                            Some(expected),
                            "shape {shape} angle {angle} stand ({sx},{sz}) flags {engine_flags:#x}",
                        );
                        checked += 1;
                    }
                }
            }
        }
    }
    assert_eq!(checked, 10 * 4 * 25 * 1024);
}

#[test]
fn walls_and_decorations_match_the_client_collision_map() {
    let mut map = CollisionMap::new();
    let mut checked = 0usize;
    for shape in 0u8..=9 {
        for angle in 0u8..4 {
            for sx in -1..=1 {
                for sz in -1..=1 {
                    for (_, client_flags) in flag_words() {
                        map.flags[(50 + sx) as usize][(50 + sz) as usize] = client_flags;
                        let expected = if (4..=8).contains(&shape) {
                            map.test_w_decor(50 + sx, 50 + sz, 50, 50, i32::from(shape), i32::from(angle))
                        } else {
                            map.test_wall(50 + sx, 50 + sz, 50, 50, i32::from(shape), i32::from(angle))
                        };
                        assert_eq!(
                            ours(shape, angle, sx, sz, client_flags),
                            Some(expected),
                            "shape {shape} angle {angle} stand ({sx},{sz}) flags {client_flags:#x}",
                        );
                        checked += 1;
                    }
                }
            }
        }
    }
    assert_eq!(checked, 10 * 4 * 9 * 1024);
}

/// The stands the reviewers named. Loc 4642 (shape 5, angle 1) at
/// (2205,4943,0) once recorded (2205,4944,0), the tile on the far side of its
/// wall decoration. Loc 3927 (shape 4) once recorded the facing neighbour in
/// each rotation. The engine has no branch for either shape.
#[test]
fn straight_wall_decorations_have_only_the_origin_as_a_stand() {
    let open = 0;
    let named = [
        (5u8, 1u8, tile(2205, 4943), tile(2205, 4944)),
        (5, 1, tile(2210, 4943), tile(2210, 4944)),
        (4, 0, tile(2313, 9656), tile(2312, 9656)),
        (4, 1, tile(2313, 9657), tile(2313, 9658)),
        (4, 3, tile(2314, 9656), tile(2314, 9655)),
        (4, 2, tile(2314, 9657), tile(2315, 9657)),
    ];
    for (shape, angle, origin, stand) in named {
        let query = |from| LocOperateQuery {
            origin,
            from,
            shape,
            angle,
            width: 1,
            length: 1,
            force_approach: 0,
            flags: open,
        };
        assert_eq!(loc_can_operate(query(stand)), Some(false), "{origin:?} {stand:?}");
        assert_eq!(loc_can_operate(query(origin)), Some(true), "{origin:?}");
    }
}

/// Shapes 1 and 3 (diagonal and square wall corners) reach only the origin.
#[test]
fn wall_corner_shapes_have_only_the_origin_as_a_stand() {
    for shape in [1u8, 3] {
        for angle in 0u8..4 {
            for (dx, dz) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                assert_eq!(ours(shape, angle, dx, dz, 0), Some(false), "{shape} {angle}");
            }
            assert_eq!(ours(shape, angle, 0, 0, 0), Some(true));
        }
    }
}

/// Shape 7 rotates by two (`alteredRotation`), so it tests the opposite pair
/// of shape 6 at the same angle.
#[test]
fn diagonal_decoration_shape_seven_uses_the_altered_rotation() {
    for angle in 0u8..4 {
        let six: Vec<(i32, i32)> = [(-1, 0), (1, 0), (0, -1), (0, 1)]
            .into_iter()
            .filter(|&(dx, dz)| ours(6, angle, dx, dz, 0) == Some(true))
            .collect();
        let seven: Vec<(i32, i32)> = [(-1, 0), (1, 0), (0, -1), (0, 1)]
            .into_iter()
            .filter(|&(dx, dz)| ours(7, (angle + 2) & 3, dx, dz, 0) == Some(true))
            .collect();
        assert_eq!(six.len(), 2, "angle {angle}");
        assert_eq!(six, seven, "angle {angle}");
    }
}

/// A blocked side removes the stand: a wall flag on the stand tile's face
/// toward the origin refuses the interaction (the `PL_WALK_*` and `W_*` tests).
#[test]
fn a_wall_face_between_stand_and_origin_refuses_the_stand() {
    // Shape 0 angle 1: the wall runs along the north edge, so the open
    // neighbours are north (free) and west/east (face tests).
    assert_eq!(ours(0, 1, 0, 1, CollisionFlag::PL_WALK_N), Some(true));
    assert_eq!(ours(0, 1, -1, 0, CollisionFlag::PL_WALK_E), Some(false));
    assert_eq!(ours(0, 1, -1, 0, 0), Some(true));
    assert_eq!(ours(0, 1, 1, 0, CollisionFlag::PL_WALK_W), Some(false));
    // Shape 9 tests the diagonal face bits.
    assert_eq!(ours(9, 0, 0, 1, CollisionFlag::W_S), Some(false));
    assert_eq!(ours(9, 0, 0, 1, 0), Some(true));
}

#[test]
fn candidates_cover_every_engine_stand_and_unsupported_shapes_have_none() {
    let origin = tile(100, 100);
    for shape in 0u8..=9 {
        for angle in 0u8..4 {
            let candidates = loc_candidate_stands(origin, shape, angle, 1, 1).unwrap();
            for sx in -2..=2 {
                for sz in -2..=2 {
                    assert!(
                        !engine_reached(0, sx, sz, shape, angle)
                            || candidates.contains(&tile(100 + sx, 100 + sz)),
                        "shape {shape} angle {angle} stand ({sx},{sz}) is not a candidate"
                    );
                }
            }
        }
    }
    for shape in [12u8, 13, 21] {
        assert_eq!(loc_reach_strategy(shape), LocReachStrategy::Unsupported);
        assert!(loc_candidate_stands(origin, shape, 0, 1, 1).is_none());
        assert_eq!(ours(shape, 0, 1, 0, 0), None);
    }
}
