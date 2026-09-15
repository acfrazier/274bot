//! Script-side Ent lookup over a caller-supplied NPC list.
//!
//! Ent identity (ids, lifetime) is revision-independent and stays shared in
//! `api::ent`; this helper answers "is an Ent standing on this tile" from the
//! supplied rows only. It never scans the posted scene or copies the world,
//! and it is not the guardian `tree spirit` evade.

use api::ent::is_ent_npc_id;
use api::snapshot::WorldTile;

/// True when a supplied NPC row is an Ent standing on `tile`.
///
/// Uses the caller-supplied `(id, tile)` list only. Does not read the
/// current posted scene or copy the world.
pub fn ent_npc_on_tile<I>(npcs: I, tile: WorldTile) -> bool
where
    I: IntoIterator<Item = (i32, WorldTile)>,
{
    npcs.into_iter()
        .any(|(id, npc_tile)| is_ent_npc_id(id) && npc_tile == tile)
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::ent::SUIT_OF_ARMOUR_NPC_ID;

    #[test]
    fn supplied_array_matches_exact_tile_not_world_scan() {
        let tree = WorldTile {
            x: 3087,
            z: 3234,
            level: 0,
        };
        let neighbour = WorldTile {
            x: 3088,
            z: 3234,
            level: 0,
        };
        let other_plane = WorldTile {
            x: 3087,
            z: 3234,
            level: 1,
        };
        assert!(ent_npc_on_tile([(444, tree)], tree));
        assert!(!ent_npc_on_tile([(444, tree)], neighbour));
        assert!(!ent_npc_on_tile([(444, tree)], other_plane));
        assert!(!ent_npc_on_tile([(443, tree)], tree));
        assert!(!ent_npc_on_tile([(SUIT_OF_ARMOUR_NPC_ID, tree)], tree));
        assert!(!ent_npc_on_tile(std::iter::empty(), tree));
        assert!(ent_npc_on_tile(
            [(443, tree), (452, tree), (444, neighbour)],
            tree
        ));
        assert!(!ent_npc_on_tile(
            [(444, neighbour), (445, other_plane)],
            tree
        ));
    }
}
