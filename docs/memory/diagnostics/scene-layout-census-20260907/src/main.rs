use client::core::{world::LevelHeightmaps, World};
use client::dash3d::{GroundStamp, Square};
use std::mem::size_of;

fn world(levels: i32, x: i32, z: i32) -> World {
    let heights = vec![vec![vec![0; (z + 1) as usize]; (x + 1) as usize]; levels as usize];
    World::new(heights, z, levels, x)
}

fn linked_count(tile: &Square) -> usize {
    1 + tile.linked_square.as_deref().map(linked_count).unwrap_or(0)
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
                    stamps += usize::from(tile.overlay_stamp.is_some());
                }
            }
        }
    }
    let grid_slots = levels * x * z;
    let tile_payload = occupied * size_of::<Square>();
    let stamp_payload = stamps * size_of::<GroundStamp>();
    let old_payload = tile_payload + stamp_payload;
    let arena = stamps * size_of::<GroundStamp>();
    let indices = occupied * size_of::<u32>();
    let free_list = stamps * size_of::<u32>();
    // Three render flags as bits, plus five i32 render bookkeeping values and
    // one i32 fill stamp: a deliberately conservative side-array sketch.
    let per_head_side_arrays = occupied * (1 + 6 * size_of::<i32>());
    let replacement = arena + indices + free_list + per_head_side_arrays;
    println!("{name} occupied_tiles={occupied} linked_tiles={linked} stamps={stamps} grid_slots={grid_slots} old_tile_payload={tile_payload} old_stamp_payload={stamp_payload} old_total_payload={old_payload} arena={arena} indices={indices} free_list={free_list} per_head_side_arrays={per_head_side_arrays} replacement_total={replacement} delta={}", old_payload as isize - replacement as isize);
}

fn main() {
    println!("pointer={} usize={} square={} ground_stamp={} option_stamp={} level_heightmaps={}", size_of::<*const ()>(), size_of::<usize>(), size_of::<Square>(), size_of::<GroundStamp>(), size_of::<Option<Box<GroundStamp>>>(), size_of::<LevelHeightmaps>());
    summarize("scene_a", &scene_a(), 4, 8, 8);
    summarize("scene_b", &scene_b(), 4, 8, 8);
}
