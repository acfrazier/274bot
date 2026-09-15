// Scene-window crop of the shared 274L canlight plane: world index is
// level*width*height + z*width + x; posted Reach bits are lx*height + lz.

use api::query::{
    pack_canlight_u32, pack_reach_query_plane, CanlightPlane, ReachQueryView, SceneQuery,
};
use api::snapshot::{SceneView, WorldTile};

fn world_canlight_bit(level: i32, width: i32, height: i32, x: i32, z: i32) -> usize {
    (level as usize) * (width as usize) * (height as usize)
        + (z as usize) * (width as usize)
        + x as usize
}

fn open_scene() -> SceneView {
    SceneView {
        available: true,
        base_x: 3200,
        base_z: 3200,
        level: 0,
        width: 104,
        height: 104,
        collision_flags: vec![0; 104 * 104],
    }
}

#[test]
fn pack_canlight_crops_transposed_index_with_nonzero_origin_and_plane() {
    let origin_x = 100;
    let origin_z = 200;
    let world_w = 5;
    let world_h = 3;
    let level = 1;
    let scene_x = 102;
    let scene_z = 201;
    let scene_w = 3;
    let scene_h = 2;
    let lit_x = 103;
    let lit_z = 202;
    let cells: usize = 4 * world_w as usize * world_h as usize;
    let mut bits = vec![0u64; cells.div_ceil(64)];
    let idx = world_canlight_bit(level, world_w, world_h, lit_x - origin_x, lit_z - origin_z);
    bits[idx / 64] |= 1u64 << (idx % 64);

    let plane = CanlightPlane {
        bits: &bits,
        origin_x,
        origin_z,
        width: world_w,
        height: world_h,
    };
    let words = pack_canlight_u32(scene_x, scene_z, level, scene_w, scene_h, Some(plane));
    assert_eq!(
        words.len(),
        (scene_w as usize * scene_h as usize).div_ceil(32)
    );
    let lit = WorldTile {
        x: lit_x,
        z: lit_z,
        level,
    };
    assert!(
        ReachQueryView::bit_at(&words, scene_w, scene_h, scene_x, scene_z, level, lit),
        "posted window must light the world cell after x/z transposition"
    );
    let neighbor = WorldTile {
        x: lit_x,
        z: lit_z - 1,
        level,
    };
    assert!(
        !ReachQueryView::bit_at(&words, scene_w, scene_h, scene_x, scene_z, level, neighbor),
        "adjacent posted cell must stay unset"
    );
    assert!(pack_canlight_u32(scene_x, scene_z, level, scene_w, scene_h, None).is_empty());
    let zeros = pack_canlight_u32(
        scene_x,
        scene_z,
        level,
        scene_w,
        scene_h,
        Some(CanlightPlane {
            bits: &vec![0u64; bits.len()],
            origin_x,
            origin_z,
            width: world_w,
            height: world_h,
        }),
    );
    assert_eq!(zeros.len(), words.len());
    assert!(
        zeros.iter().all(|w| *w == 0),
        "valid all-zero stays present"
    );
}

#[test]
fn pack_reach_query_plane_posts_cropped_canlight_and_empty_when_missing() {
    let mut scene = open_scene();
    scene.level = 2;
    let player = WorldTile {
        x: 3205,
        z: 3205,
        level: 2,
    };
    let sq = SceneQuery::new(&scene, Some(player));
    let flood = sq.flood_reach().expect("player in scene floods");
    let origin_x = 3100;
    let origin_z = 3100;
    let world_w = 200;
    let world_h = 200;
    let cells: usize = 4 * world_w as usize * world_h as usize;
    let mut bits = vec![0u64; cells.div_ceil(64)];
    let lit = WorldTile {
        x: 3205,
        z: 3206,
        level: 2,
    };
    let idx = world_canlight_bit(2, world_w, world_h, lit.x - origin_x, lit.z - origin_z);
    bits[idx / 64] |= 1u64 << (idx % 64);
    let plane = CanlightPlane {
        bits: &bits,
        origin_x,
        origin_z,
        width: world_w,
        height: world_h,
    };
    let view = pack_reach_query_plane(&scene, Some(&flood), Some(plane));
    assert!(view.available);
    assert!(!view.canlight.is_empty());
    assert!(ReachQueryView::bit_at(
        &view.canlight,
        view.width,
        view.height,
        view.base_x,
        view.base_z,
        view.level,
        lit,
    ));
    assert!(!ReachQueryView::bit_at(
        &view.canlight,
        view.width,
        view.height,
        view.base_x,
        view.base_z,
        view.level,
        player,
    ));
    let missing = pack_reach_query_plane(&scene, Some(&flood), None);
    assert!(missing.canlight.is_empty());
    assert_eq!(
        pack_reach_query_plane(&scene, None, Some(plane)),
        ReachQueryView::unavailable()
    );
}
