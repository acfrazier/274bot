//! Borrowing family queries: look up views from the last rebuild without
//! allocating a new world copy.

use crate::snapshot::NpcView;

/// The view for a slot index, if that slot was live in the last rebuild.
pub fn npc_by_index(npcs: &[NpcView], index: usize) -> Option<&NpcView> {
    npcs.iter().find(|view| view.index == index)
}

/// Live views standing on a tile.
pub fn npcs_at<'a>(npcs: &'a [NpcView], x: i32, z: i32) -> impl Iterator<Item = &'a NpcView> {
    npcs.iter().filter(move |view| view.x == x && view.z == z)
}
