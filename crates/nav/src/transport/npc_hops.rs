use super::*;

// ---------------------------------------------------------------------------
// Rune Mysteries essence mine: wizard entry teleports
// (`TransportKind::Npc`).
// ---------------------------------------------------------------------------

/// One essence-mine wizard journey: `at` the wizard NPC's placement tile
/// (jm2 `==== NPC ====` placement, id resolved through `pack/npc.pack`),
/// `to` the Rune Essence mine pad. The whole hop is the wizard's direct
/// teleport op — `[opnpc3,<name>]` calls `@teleport_to_essence_mine`, and
/// the `teleport_to_essence_mine` proc refuses below
/// `%runemysteries >= ^runemysteries_complete`, so the edge carries the
/// Rune Mysteries quest name.
#[derive(Debug, Clone, Copy)]
pub(super) struct EssenceWizard {
    /// npc.pack id of the wizard who opens the portal.
    npc: i32,
    at: WorldTile,
    /// The wizard's direct teleport op (the `[opnpcN,…]` block that calls
    /// `@teleport_to_essence_mine`).
    option: i32,
}

/// The 2004 essence-mine wizards: placement tiles from the `==== NPC ====`
/// entries in `content/maps/*.jm2`, ids from `pack/npc.pack`, and the
/// direct-teleport op from each wizard script (`[opnpc4,aubury]` vs the
/// others' `[opnpc3,…]`). The proc lands the player at a random
/// `essence_mine_teleports` coord inside the enclosed mine (m45_75) and
/// stores the wizard's `^essence_mine_to_<wizard>` return anchor for the
/// exit portal, so the entry `to` is the mine's walkable centre pad and
/// the executor accepts any landing in the mine.
pub(super) const ESSENCE_WIZARDS: &[EssenceWizard] = &[
    // Aubury (aubury, npc 553) in the Varrock rune shop (m50_53 local
    // (53,10)); `[opnpc4,aubury]`.
    EssenceWizard {
        npc: 553,
        at: WorldTile {
            x: 3253,
            z: 3402,
            level: 0,
        },
        option: 4,
    },
    // Sedridor (head_wizard, npc 300) in the Wizards' Tower cellar
    // (m48_149 local (31,35) — the 6400-cellar band of (3103,3171));
    // `[opnpc3,head_wizard]`.
    EssenceWizard {
        npc: 300,
        at: WorldTile {
            x: 3103,
            z: 9571,
            level: 0,
        },
        option: 3,
    },
    // Distentor (guild_wizard, npc 462) at the Magicians' Guild, Yanille
    // (m40_48 local (34,17)); `[opnpc3,guild_wizard]`.
    EssenceWizard {
        npc: 462,
        at: WorldTile {
            x: 2594,
            z: 3089,
            level: 0,
        },
        option: 3,
    },
    // Cromperty (ardounge_wizard, npc 844) in East Ardougne (m41_51
    // local (59,62)); `[opnpc3,ardounge_wizard]`.
    EssenceWizard {
        npc: 844,
        at: WorldTile {
            x: 2683,
            z: 3326,
            level: 0,
        },
        option: 3,
    },
    // Brimstail (gnome_brimstail, npc 171) in his cave (m37_153 local
    // (22,18) — the 6400-cellar band of (2390,3410));
    // `[opnpc3,gnome_brimstail]`.
    EssenceWizard {
        npc: 171,
        at: WorldTile {
            x: 2390,
            z: 9810,
            level: 0,
        },
        option: 3,
    },
];

/// The Rune Essence mine pad (m45_75 local (32,33)): the walkable centre
/// anchor the entry edges land on. The real landing is randomised among
/// the `essence_mine_teleports` enum coords, so the executor accepts any
/// landing inside the enclosed mine instead of this exact tile.
pub(super) const ESSENCE_MINE_PAD: WorldTile = WorldTile {
    x: 2912,
    z: 4833,
    level: 0,
};

/// Essence-mine entry edges from the fixed wizard table: one direct
/// teleport hop per wizard, landing on the mine pad. The return is not
/// packed — the mine exit portal's hop is synthesized per-slot from the
/// traveller's [`crate::essence::EssenceSession`], so the mine is never
/// a corridor between arbitrary overworld tiles. The gate is the quest
/// journal's row name ("Rune Mysteries Quest", green at
/// `%runemysteries >= ^runemysteries_complete`) — the same name
/// `WorldState::from_snapshot` reads from the quest tab; the
/// perm-scoped `%runemysteries` varp is never transmitted, so a
/// `varp_req` gate could never pass live.
pub(super) fn essence_mine_edges(graph: &mut TransportGraph) {
    for w in ESSENCE_WIZARDS {
        graph.edges.push(TransportEdge {
            kind: TransportKind::Npc,
            at: w.at,
            to: ESSENCE_MINE_PAD,
            loc_id: w.npc,
            option: w.option,
            ticks: ESSENCE_MINE_TICKS,
            dir: None,
            open_loc_id: None,
            skill_req: vec![],
            item_req: vec![],
            quest_req: vec!["Rune Mysteries Quest".to_string()],
            varp_req: vec![],
            worn_req: vec![],
            members_req: false,
            wildy_cap: None,
        });
    }
}

// ---------------------------------------------------------------------------
// Elkoy's Tree Gnome Village maze escorts (`TransportKind::Npc`).
// ---------------------------------------------------------------------------

/// One Elkoy escort journey: `at` the Elkoy NPC's placement tile (jm2
/// `==== NPC ====` placement, id resolved through `pack/npc.pack`), `to`
/// the coord the script's `p_telejump(` literal lands on. The whole hop
/// is one `Talk-to` (`opnpc1`) — the "Yes please."/"Can you show me
/// out…" choice is execute, never a search arm — and the scripts carry no
/// `p_delay`, so `ticks` is the 1 op base like the carts and spirit trees.
#[derive(Debug, Clone, Copy)]
pub(super) struct ElkoyEscort {
    /// npc.pack id of the Elkoy who escorts the player.
    npc: i32,
    at: WorldTile,
    to: WorldTile,
}

/// The 2004 Elkoy escorts: the two `p_telejump(` destinations from
/// `content/scripts/areas/area_gnome/scripts/elkoy.rs2` —
/// `^elkoy_maze_coord` (the maze-side `[opnpc1,elkoy]` escort into the
/// village) and `^elkoy_entrance_coord` (the village `[opnpc1,elkoy_village]`
/// escort out) — resolved through `content/scripts/quests/quest_tree/
/// configs/quest_tree.constant` (`0_39_49_8_56` → (2504,3192),
/// `0_39_49_19_23` → (2515,3159)); origin tiles from the `==== NPC ====`
/// placements in `content/maps/m39_49.jm2` (npc 473 elkoy at local
/// (8,55) = (2504,3191), one tile south of the entrance coord; npc 474
/// elkoy_village at local (18,23) = (2514,3159), one tile west of the maze
/// coord); ids from `pack/npc.pack`. The edges carry the Tree Gnome
/// Village journal name — `elkoy.rs2`'s `[opnpc1,…]` blocks gate on
/// `%treequest` at every stage. The traveller walks no maze tiles: the
/// hop lands straight on the village/entrance coord (the script's own
/// landing, never a snap).
pub(super) const ELKOY_ESCORTS: &[ElkoyEscort] = &[
    // elkoy (npc 473) by the maze entrance (m39_49 local (8,55)):
    // `p_telejump(^elkoy_maze_coord)` lands in the village (2515,3159).
    ElkoyEscort {
        npc: 473,
        at: WorldTile {
            x: 2504,
            z: 3191,
            level: 0,
        },
        to: WorldTile {
            x: 2515,
            z: 3159,
            level: 0,
        },
    },
    // elkoy_village (npc 474) in the village (m39_49 local (18,23)):
    // `p_telejump(^elkoy_entrance_coord)` lands at the maze entrance
    // (2504,3192).
    ElkoyEscort {
        npc: 474,
        at: WorldTile {
            x: 2514,
            z: 3159,
            level: 0,
        },
        to: WorldTile {
            x: 2504,
            z: 3192,
            level: 0,
        },
    },
];

/// Elkoy escort edges from the fixed 2004 route table: one `Talk-to` edge
/// per escort, keyed from the Elkoy NPC's tile.
pub(super) fn elkoy_edges(graph: &mut TransportGraph) {
    for e in ELKOY_ESCORTS {
        graph.edges.push(TransportEdge {
            kind: TransportKind::Npc,
            at: e.at,
            to: e.to,
            loc_id: e.npc,
            option: 1,
            ticks: 1,
            dir: None,
            open_loc_id: None,
            skill_req: vec![],
            item_req: vec![],
            quest_req: vec!["Tree Gnome Village".to_string()],
            varp_req: vec![],
            worn_req: vec![],
            members_req: false,
            wildy_cap: None,
        });
    }
}
