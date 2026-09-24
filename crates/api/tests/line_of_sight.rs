// Frozen LOS geometry and v2/v1 query order. Not a Reach flood stand-in.

use api::line_of_sight::{
    has_line_of_sight_local, line_of_sight_v1, line_of_sight_v2, CollisionQuery, Footprint,
    LineOfSightError, SCENE_SIZE,
};
use api::snapshot::WorldTile;
use client::dash3d::CollisionFlag;
use std::sync::Arc;

fn open_scene(w: i32, h: i32) -> Vec<i32> {
    vec![0; (w * h) as usize]
}

fn flags_at<'a>(grid: &'a [i32], w: i32, h: i32) -> impl Fn(i32, i32) -> Option<i32> + 'a {
    move |lx, lz| {
        if lx < 0 || lz < 0 || lx >= w || lz >= h {
            return None;
        }
        Some(grid[(lx * h + lz) as usize])
    }
}

fn set(grid: &mut [i32], h: i32, lx: i32, lz: i32, flag: i32) {
    grid[(lx * h + lz) as usize] = flag;
}

fn one(lx: i32, lz: i32) -> Footprint {
    Footprint { lx, lz, size: 1 }
}

fn tile(x: i32, z: i32, level: i32) -> WorldTile {
    WorldTile { x, z, level }
}

fn query(
    available: bool,
    flags: &[i32],
    base_x: i32,
    base_z: i32,
    level: i32,
    w: i32,
    h: i32,
) -> CollisionQuery {
    CollisionQuery {
        available,
        base_x,
        base_z,
        level,
        width: w,
        height: h,
        flags: Arc::from(flags),
    }
}

#[test]
fn frozen_open_row_col_diag() {
    let g = open_scene(16, 16);
    let at = flags_at(&g, 16, 16);
    assert!(has_line_of_sight_local(&at, one(1, 1), one(8, 1)));
    assert!(has_line_of_sight_local(&at, one(1, 1), one(1, 8)));
    assert!(has_line_of_sight_local(&at, one(1, 1), one(6, 6)));
    assert!(has_line_of_sight_local(&at, one(8, 8), one(2, 3)));
}

#[test]
fn frozen_self_tile_true_before_flags() {
    let mut g = open_scene(16, 16);
    set(&mut g, 16, 3, 3, CollisionFlag::WALK_SCENERY);
    let at = flags_at(&g, 16, 16);
    assert!(has_line_of_sight_local(&at, one(3, 3), one(3, 3)));
}

#[test]
fn frozen_asymmetric_projectile_vs_walk_only_walls() {
    let mut g = open_scene(16, 16);
    set(&mut g, 16, 5, 1, CollisionFlag::V_W);
    let at = flags_at(&g, 16, 16);
    assert!(!has_line_of_sight_local(&at, one(1, 1), one(8, 1)));

    let mut g = open_scene(16, 16);
    set(&mut g, 16, 5, 1, CollisionFlag::W_W);
    let at = flags_at(&g, 16, 16);
    assert!(has_line_of_sight_local(&at, one(1, 1), one(8, 1)));

    let mut g = open_scene(16, 16);
    set(&mut g, 16, 1, 5, CollisionFlag::V_S);
    let at = flags_at(&g, 16, 16);
    assert!(!has_line_of_sight_local(&at, one(1, 1), one(1, 8)));

    let mut g = open_scene(16, 16);
    set(&mut g, 16, 1, 5, CollisionFlag::V_N);
    let at = flags_at(&g, 16, 16);
    assert!(!has_line_of_sight_local(&at, one(1, 8), one(1, 1)));
}

#[test]
fn frozen_vis_scenery_blocks_walk_scenery_does_not() {
    let mut g = open_scene(16, 16);
    set(&mut g, 16, 5, 1, CollisionFlag::VIS_SCENERY);
    let at = flags_at(&g, 16, 16);
    assert!(!has_line_of_sight_local(&at, one(1, 1), one(8, 1)));

    let mut g = open_scene(16, 16);
    set(&mut g, 16, 5, 1, CollisionFlag::WALK_SCENERY);
    let at = flags_at(&g, 16, 16);
    assert!(has_line_of_sight_local(&at, one(1, 1), one(8, 1)));

    let mut g = open_scene(16, 16);
    set(&mut g, 16, 4, 4, CollisionFlag::VIS_SCENERY);
    let at = flags_at(&g, 16, 16);
    assert!(!has_line_of_sight_local(&at, one(1, 1), one(6, 6)));
}

#[test]
fn frozen_dest_rock_visible_start_scenery_blind() {
    let mut g = open_scene(16, 16);
    set(&mut g, 16, 8, 1, CollisionFlag::VIS_SCENERY);
    let at = flags_at(&g, 16, 16);
    assert!(has_line_of_sight_local(&at, one(1, 1), one(8, 1)));

    let mut g = open_scene(16, 16);
    set(&mut g, 16, 1, 1, CollisionFlag::WALK_SCENERY);
    let at = flags_at(&g, 16, 16);
    assert!(!has_line_of_sight_local(&at, one(1, 1), one(8, 1)));
}

#[test]
fn frozen_scene_edge_opaque() {
    let g = open_scene(16, 16);
    let at = flags_at(&g, 16, 16);
    assert!(!has_line_of_sight_local(&at, one(14, 1), one(20, 1)));
}

#[test]
fn frozen_size3_nearest_corner() {
    let body = Footprint {
        lx: 3,
        lz: 3,
        size: 3,
    };
    let mut g = open_scene(16, 16);
    set(&mut g, 16, 4, 4, CollisionFlag::VIS_SCENERY);
    let at = flags_at(&g, 16, 16);
    assert!(has_line_of_sight_local(&at, one(8, 8), body));

    let mut g = open_scene(16, 16);
    set(&mut g, 16, 6, 6, CollisionFlag::VIS_SCENERY);
    let at = flags_at(&g, 16, 16);
    assert!(!has_line_of_sight_local(&at, one(8, 8), body));

    let mut g = open_scene(16, 16);
    set(&mut g, 16, 7, 4, CollisionFlag::VIS_SCENERY);
    let at = flags_at(&g, 16, 16);
    assert!(!has_line_of_sight_local(&at, one(9, 4), body));

    let mut g = open_scene(16, 16);
    set(&mut g, 16, 2, 4, CollisionFlag::VIS_SCENERY);
    let at = flags_at(&g, 16, 16);
    assert!(has_line_of_sight_local(&at, one(9, 4), body));
}

#[test]
fn frozen_size4_viewer_side() {
    let body = Footprint {
        lx: 5,
        lz: 5,
        size: 4,
    };
    let mut g = open_scene(16, 16);
    set(&mut g, 16, 3, 3, CollisionFlag::VIS_SCENERY);
    let at = flags_at(&g, 16, 16);
    assert!(!has_line_of_sight_local(&at, one(1, 1), body));

    let mut g = open_scene(16, 16);
    set(&mut g, 16, 7, 7, CollisionFlag::VIS_SCENERY);
    let at = flags_at(&g, 16, 16);
    assert!(has_line_of_sight_local(&at, one(1, 1), body));
}

#[test]
fn t15_flood_reachable_vis_scenery_is_los_false() {
    let mut g = open_scene(16, 16);
    set(&mut g, 16, 5, 1, CollisionFlag::VIS_SCENERY);
    let q = query(true, &g, 3200, 3200, 0, 16, 16);
    let got = line_of_sight_v2(Some(&q), tile(3201, 3201, 0), tile(3208, 3201, 0), Some(1));
    assert_eq!(got, Ok(false));
}

#[test]
fn v2_invalid_args_independent_of_absence() {
    let absent = query(false, &[], 0, 0, 0, 0, 0);
    assert_eq!(
        line_of_sight_v2(Some(&absent), tile(1, 1, 0), tile(1, 1, 0), Some(0)),
        Err(LineOfSightError::InvalidArgs)
    );
    assert_eq!(
        line_of_sight_v2(Some(&absent), tile(1, 1, 0), tile(1, 1, 0), Some(105)),
        Err(LineOfSightError::InvalidArgs)
    );
    assert_eq!(
        line_of_sight_v2(
            Some(&absent),
            tile(1, 1, 0),
            tile(1, 1, 0),
            Some(SCENE_SIZE + 1)
        ),
        Err(LineOfSightError::InvalidArgs)
    );
    assert_eq!(
        line_of_sight_v2(None, tile(1, 1, 0), tile(2, 2, 1), Some(1)),
        Ok(false),
        "different plane is ok:true value:false even without family"
    );
}

#[test]
fn v2_same_plane_absent_is_missing_observation() {
    assert_eq!(
        line_of_sight_v2(None, tile(1, 1, 0), tile(2, 2, 0), Some(1)),
        Err(LineOfSightError::MissingObservation)
    );
    let absent = query(false, &[], 0, 0, 0, 0, 0);
    assert_eq!(
        line_of_sight_v2(Some(&absent), tile(1, 1, 0), tile(2, 2, 0), None),
        Err(LineOfSightError::MissingObservation)
    );
}

#[test]
fn v2_published_level_mismatch_is_missing_observation() {
    let g = open_scene(8, 8);
    let q = query(true, &g, 3200, 3200, 1, 8, 8);
    assert_eq!(
        line_of_sight_v2(Some(&q), tile(3200, 3200, 0), tile(3201, 3200, 0), Some(1)),
        Err(LineOfSightError::MissingObservation)
    );
}

#[test]
fn v2_origin_out_of_scene_false_edge_footprint_not_rejected() {
    let g = open_scene(104, 104);
    let q = query(true, &g, 0, 0, 0, 104, 104);
    assert_eq!(
        line_of_sight_v2(Some(&q), tile(-1, 50, 0), tile(10, 50, 0), Some(1)),
        Ok(false)
    );
    assert_eq!(
        line_of_sight_v2(Some(&q), tile(100, 50, 0), tile(102, 50, 0), Some(4)),
        Ok(true),
        "root: far footprint may extend past 104 when origins and sampled ray stay valid"
    );
}

#[test]
fn v2_self_tile_true_only_after_both_origins_in_scene() {
    let g = open_scene(8, 8);
    let q = query(true, &g, 3200, 3200, 0, 8, 8);
    assert_eq!(
        line_of_sight_v2(Some(&q), tile(3203, 3203, 0), tile(3203, 3203, 0), Some(1)),
        Ok(true)
    );
    assert_eq!(
        line_of_sight_v2(Some(&q), tile(4000, 4000, 0), tile(4000, 4000, 0), Some(1)),
        Ok(false)
    );
}

#[test]
fn v1_bounded_compat() {
    let g = open_scene(8, 8);
    let q = query(true, &g, 3200, 3200, 0, 8, 8);
    assert!(line_of_sight_v1(
        Some(&q),
        tile(3201, 3201, 0),
        tile(3204, 3201, 0),
        None
    ));
    assert!(!line_of_sight_v1(
        Some(&q),
        tile(3201, 3201, 0),
        tile(3204, 3201, 0),
        Some(0)
    ));
    assert!(!line_of_sight_v1(
        Some(&q),
        tile(3201, 3201, 0),
        tile(3204, 3201, 0),
        Some(105)
    ));
    assert!(!line_of_sight_v1(
        None,
        tile(3201, 3201, 0),
        tile(3204, 3201, 0),
        Some(1)
    ));
    assert!(!line_of_sight_v1(
        Some(&q),
        tile(3201, 3201, 0),
        tile(3201, 3201, 1),
        Some(1)
    ));
}

/// Chosen fixture: LOS to network SW disagrees with LOS to rendered SW.
/// Not a universal "wrong coords fail" rule.
#[test]
fn network_sw_and_rendered_sw_are_not_interchangeable_for_los() {
    let mut g = open_scene(16, 16);
    set(&mut g, 16, 3, 1, CollisionFlag::VIS_SCENERY);
    let q = query(true, &g, 3200, 3200, 0, 16, 16);
    let spot = tile(3201, 3201, 0);
    let network = tile(3205, 3201, 0);
    let rendered = tile(3201, 3205, 0);
    let network_los = line_of_sight_v2(Some(&q), spot, network, Some(4));
    let rendered_los = line_of_sight_v2(Some(&q), spot, rendered, Some(4));
    assert_eq!(network_los, Ok(false));
    assert_eq!(rendered_los, Ok(true));
    assert_ne!(network_los, rendered_los);
}
