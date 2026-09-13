//! Host-owned Ent NPC identity and supplied-array tile match.
//!
//! Public contract is revision-independent on the selected 274/289 caches:
//! `npc.pack` maps 444..=452 to `macro_ent_*` and 453 to `suit_of_armour`
//! on both pinned content trees (`000c19997e07206131bcb3c884265840efce416d`
//! and `92649430fcbc83538d8c4367ecb96cee1a67a944`, recorded in
//! `crates/api/data/game-data/manifest.json`). Lifetime 60 is the
//! `npc_add(..., 60)` / `loc_del(60)` duration in
//! `scripts/macro events/scripts/woodcutting/macro_event_ent.rs2` on both
//! trees. Generated `SelectedGameData` item rows do not carry NPC ids;
//! these constants are the public binding, not a copied JS table.

use crate::snapshot::WorldTile;

/// Ent NPC type ids from selected `npc.pack` (`macro_ent_tree1`..=`macro_ent_magic`).
/// 453 is `suit_of_armour` and is not an Ent.
pub const ENT_NPC_IDS: &[i32] = &[444, 445, 446, 447, 448, 449, 450, 451, 452];

/// First id after the Ent pack; `suit_of_armour` on both selected caches.
pub const SUIT_OF_ARMOUR_NPC_ID: i32 = 453;

/// Ent despawn duration in ticks (`npc_add` duration in `macro_event_ent.rs2`).
pub const ENT_LIFE_TICKS: i32 = 60;

/// True when `id` is one of the selected-cache Ent NPC types.
pub fn is_ent_npc_id(id: i32) -> bool {
    ENT_NPC_IDS.contains(&id)
}

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
    use crate::game_data::for_revision;
    use client::io::ClientRevision;

    #[test]
    fn ids_cover_pack_and_exclude_suit_of_armour() {
        assert_eq!(ENT_NPC_IDS, &[444, 445, 446, 447, 448, 449, 450, 451, 452]);
        assert_eq!(ENT_NPC_IDS.len(), 9);
        assert_eq!(ENT_LIFE_TICKS, 60);
        assert!(!is_ent_npc_id(443));
        assert!(is_ent_npc_id(444));
        assert!(is_ent_npc_id(452));
        assert!(!is_ent_npc_id(SUIT_OF_ARMOUR_NPC_ID));
        assert!(!is_ent_npc_id(0));
        assert!(!is_ent_npc_id(-1));
    }

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

    #[test]
    fn selected_caches_load_and_share_the_revision_independent_ent_contract() {
        let data_274 = for_revision(ClientRevision::R274).expect("274 game data");
        let data_289 = for_revision(ClientRevision::R289).expect("289 game data");
        assert_eq!(data_274.revision(), 274);
        assert_eq!(data_289.revision(), 289);
        assert_ne!(data_274.cache_id(), data_289.cache_id());
        assert!(data_274.item_by_alias("rune_platebody").is_some());
        assert!(data_289.item_by_alias("rune_platebody").is_some());
        assert_eq!(ENT_NPC_IDS, &[444, 445, 446, 447, 448, 449, 450, 451, 452]);
        assert_eq!(ENT_LIFE_TICKS, 60);
        assert!(!is_ent_npc_id(SUIT_OF_ARMOUR_NPC_ID));
    }
}
