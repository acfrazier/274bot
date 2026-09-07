use client::core::{world::LevelHeightmaps, World};
use client::dash3d::{Decor, Ground, GroundDecor, GroundObject, GroundStamp, QuickGround, Square, Wall};
use std::ptr::NonNull;
use std::mem::size_of;

#[repr(C)]
struct RetainedSquareCore {
    level: i32,
    x: i32,
    z: i32,
    original_level: i32,
    sprites: [u32; 5],
    sprite_span: [i32; 5],
    quick_ground: Option<QuickGround>,
    ground: Option<NonNull<Ground>>,
    wall: Option<NonNull<Wall>>,
    decor: Option<NonNull<Decor>>,
    ground_decor: Option<NonNull<GroundDecor>>,
    ground_object: Option<NonNull<GroundObject>>,
    linked_square: Option<u32>,
    sprite_count: i32,
    sprite_spans: i32,
    model_stamp: i32,
}

fn world(levels: i32, x: i32, z: i32) -> World {
    let heights = vec![vec![vec![0; (z + 1) as usize]; (x + 1) as usize]; levels as usize];
    World::new(heights, z, levels, x)
}

fn linked_count(tile: &Square) -> usize {
    1 + tile.linked_square.as_deref().map(linked_count).unwrap_or(0)
}

fn linked_stamp_count(tile: &Square) -> usize {
    usize::from(tile.overlay_stamp.is_some())
        + tile
            .linked_square
            .as_deref()
            .map(linked_stamp_count)
            .unwrap_or(0)
}

fn scene_a() -> World {
    let mut w = world(4, 8, 8);
    for &(level, x, z) in &[(0, 1, 1), (1, 1, 1), (2, 2, 2), (3, 6, 6), (1, 4, 5)] {
        w.set_ground(level, x, z, 1, 0, -1, 10, 11, 12, 13, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10);
    }
    w.push_down(1, 1);
    w
}

fn scene_b() -> World {
    let mut w = world(4, 8, 8);
    for &(level, x, z) in &[(0, 0, 0), (0, 1, 0), (0, 2, 0), (2, 3, 3), (2, 3, 4), (2, 4, 3), (3, 7, 7)] {
        w.set_ground(level, x, z, 2, 1, 5, 20, 21, 22, 23, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20);
    }
    w.push_down(3, 3);
    w
}

fn summarize(name: &str, w: &World, levels: usize, x: usize, z: usize) {
    let mut occupied = 0usize;
    let mut linked = 0usize;
    let mut stamps = 0usize;
    for l in 0..levels {
        for ix in 0..x {
            for iz in 0..z {
                if let Some(tile) = w.square(l as i32, ix as i32, iz as i32) {
                    occupied += 1;
                    linked += linked_count(tile);
                    stamps += linked_stamp_count(tile);
                }
            }
        }
    }
    let grid_slots = levels * x * z;
    let tile_payload = linked * size_of::<Square>();
    let stamp_payload = stamps * size_of::<GroundStamp>();
    let old_payload = tile_payload + stamp_payload;
    let arena = stamps * size_of::<GroundStamp>();
    let indices = linked * size_of::<u32>();
    let free_list = stamps * size_of::<u32>();
    // Three render flags as bits, plus five i32 render bookkeeping values and
    // one i32 fill stamp: a deliberately conservative side-array sketch.
    let per_head_side_arrays = linked * (1 + 6 * size_of::<i32>());
    let retained_core = linked * size_of::<RetainedSquareCore>();
    let replacement = arena + indices + free_list + retained_core + per_head_side_arrays;
    println!("{name} occupied_tiles={occupied} linked_tiles={linked} stamps={stamps} grid_slots={grid_slots} old_tile_payload={tile_payload} old_stamp_payload={stamp_payload} old_total_payload={old_payload} arena={arena} indices={indices} free_list={free_list} retained_core={retained_core} per_head_side_arrays={per_head_side_arrays} replacement_total={replacement} delta={}", old_payload as isize - replacement as isize);
}

fn main() {
    println!("pointer={} usize={} square={} retained_core={} ground_stamp={} option_stamp={} level_heightmaps={} quick_ground={} ground_ptr={} wall_ptr={} decor_ptr={} ground_decor_ptr={} ground_object_ptr={}", size_of::<*const ()>(), size_of::<usize>(), size_of::<Square>(), size_of::<RetainedSquareCore>(), size_of::<GroundStamp>(), size_of::<Option<Box<GroundStamp>>>(), size_of::<LevelHeightmaps>(), size_of::<QuickGround>(), size_of::<Option<NonNull<Ground>>>(), size_of::<Option<NonNull<Wall>>>(), size_of::<Option<NonNull<Decor>>>(), size_of::<Option<NonNull<GroundDecor>>>(), size_of::<Option<NonNull<GroundObject>>>());
    summarize("scene_a", &scene_a(), 4, 8, 8);
    summarize("scene_b", &scene_b(), 4, 8, 8);
}
