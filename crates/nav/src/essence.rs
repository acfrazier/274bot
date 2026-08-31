//! The Rune Essence mine's per-slot return latch. Entering the mine
//! through wizard N records N; the mine's exit portal may only return
//! to N. The session lives on the [`crate::traveller::Traveller`]; the
//! router synthesizes a session-gated return hop from it (the
//! `[oploc1,blankrunestone_exit_portal]` portal teleports to the
//! wizard's `^essence_mine_to_<wizard>` anchor). The pack carries entry
//! edges only (wizard → mine pad) — the return is never packed, so
//! `find` without a session treats the enclosed mine as a sealed dead
//! end, never a corridor between arbitrary overworld tiles.

use api::snapshot::WorldTile;

use crate::transport::{TransportEdge, TransportKind};

/// The latched essence-mine session: the wizard the player entered
/// through and the overworld tile the mine exit portal returns to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EssenceSession {
    pub wizard_npc: i32,
    pub return_tile: WorldTile,
}

/// The mine exit portal (`blankrunestone_exit_portal`, loc 2492)
/// placements inside the enclosure — the `==== LOC ====` rows of
/// `maps/m45_75.jm2`: the interact targets the session return hop is
/// relaxed from. The mine carries four of them; the router relaxes all
/// four and the route's leg names the one it uses.
pub const ESSENCE_MINE_PORTALS: &[WorldTile] = &[
    WorldTile {
        x: 2885,
        z: 4850,
        level: 0,
    },
    WorldTile {
        x: 2889,
        z: 4813,
        level: 0,
    },
    WorldTile {
        x: 2932,
        z: 4854,
        level: 0,
    },
    WorldTile {
        x: 2933,
        z: 4815,
        level: 0,
    },
];

/// The mine exit portal loc id (`pack/loc.pack`
/// `blankrunestone_exit_portal`).
pub const ESSENCE_MINE_PORTAL_LOC_ID: i32 = 2492;

/// Exit-portal teleport ticks: OP_BASE 1 + the `p_delay(1)` in
/// `[oploc1,blankrunestone_exit_portal]`.
pub const ESSENCE_MINE_EXIT_TICKS: i32 = 2;

/// The exit hop's arrival tolerance: the portal teleports to
/// `map_findsquare(anchor, 0, 2, lineofwalk)` — a random standable tile
/// within chebyshev 2 of the wizard's anchor, never the anchor exactly.
pub const ESSENCE_MINE_EXIT_ARRIVE_RADIUS: i32 = 2;

/// The session for entering the mine through wizard `wizard_npc`; `None`
/// when the npc is not an essence wizard.
pub fn essence_session_for_wizard(wizard_npc: i32) -> Option<EssenceSession> {
    essence_return_anchor(wizard_npc).map(|return_tile| EssenceSession {
        wizard_npc,
        return_tile,
    })
}

/// Whether the edge is a packed essence-mine entry hop: an Npc edge whose
/// loc id is an essence wizard. Only these latch the session on arrival.
pub fn is_essence_entry_edge(edge: &TransportEdge) -> bool {
    edge.kind == TransportKind::Npc && essence_return_anchor(edge.loc_id).is_some()
}

/// Whether the tile is inside the Rune Essence mine enclosure (mapsquare
/// m45_75, the cave the 22 `essence_mine_teleports` coords land in). The
/// entry hop's arrival accepts any tile in here — the landing is
/// randomised, never the pad exactly.
pub fn in_essence_mine(t: WorldTile) -> bool {
    t.level == 0 && (2880..2944).contains(&t.x) && (4800..4864).contains(&t.z)
}

/// The session-gated return hop: `oploc1` the mine exit portal at `at`,
/// teleporting to the entry wizard's overworld anchor. Synthesized by the
/// router from the live session — never packed.
pub fn essence_return_edge(at: WorldTile, session: &EssenceSession) -> TransportEdge {
    TransportEdge {
        kind: TransportKind::EssenceExit,
        at,
        to: session.return_tile,
        loc_id: ESSENCE_MINE_PORTAL_LOC_ID,
        option: 1,
        ticks: ESSENCE_MINE_EXIT_TICKS,
        dir: None,
        open_loc_id: None,
        skill_req: vec![],
        item_req: vec![],
        quest_req: vec![],
        varp_req: vec![],
        worn_req: vec![],
    }
}

/// `^essence_mine_to_<wizard>` return anchors from
/// `content/scripts/skill_runecraft/configs/runecraft.constant`
/// (`lvl_mx_mz_lx_lz`): the tile the mine exit portal teleports the
/// player to after entering through that wizard.
fn essence_return_anchor(wizard_npc: i32) -> Option<WorldTile> {
    match wizard_npc {
        // aubury: 0_50_53_53_9
        553 => Some(WorldTile {
            x: 3253,
            z: 3401,
            level: 0,
        }),
        // head_wizard (Sedridor): 0_48_149_34_36
        300 => Some(WorldTile {
            x: 3106,
            z: 9572,
            level: 0,
        }),
        // guild_wizard (Distentor): 0_40_48_31_14
        462 => Some(WorldTile {
            x: 2591,
            z: 3086,
            level: 0,
        }),
        // ardounge_wizard (Cromperty): 0_41_51_60_58
        844 => Some(WorldTile {
            x: 2684,
            z: 3322,
            level: 0,
        }),
        // gnome_brimstail: 0_37_153_22_18
        171 => Some(WorldTile {
            x: 2390,
            z: 9810,
            level: 0,
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_latches_the_entry_wizards_return_anchor() {
        let aubury = essence_session_for_wizard(553).expect("aubury is a wizard");
        assert_eq!(aubury.wizard_npc, 553);
        assert_eq!(
            aubury.return_tile,
            WorldTile {
                x: 3253,
                z: 3401,
                level: 0
            },
            "aubury's `^essence_mine_to_aubury` anchor"
        );
        let sedridor = essence_session_for_wizard(300).expect("sedridor is a wizard");
        assert_eq!(
            sedridor.return_tile,
            WorldTile {
                x: 3106,
                z: 9572,
                level: 0
            }
        );
        assert_eq!(
            essence_session_for_wizard(7),
            None,
            "a cart driver latches nothing"
        );
    }

    #[test]
    fn entry_edges_are_the_wizard_npc_hops_only() {
        let mut entry = essence_return_edge(
            ESSENCE_MINE_PORTALS[0],
            &essence_session_for_wizard(553).unwrap(),
        );
        entry.kind = TransportKind::Npc;
        entry.loc_id = 553; // the wizard npc, like `essence_mine_edges`
        entry.to = WorldTile {
            x: 2912,
            z: 4833,
            level: 0,
        };
        assert!(is_essence_entry_edge(&entry));
        let cart = TransportEdge {
            kind: TransportKind::Npc,
            at: WorldTile {
                x: 2834,
                z: 2954,
                level: 0,
            },
            to: WorldTile {
                x: 2776,
                z: 3214,
                level: 0,
            },
            loc_id: 511,
            option: 1,
            ticks: 1,
            dir: None,
            open_loc_id: None,
            skill_req: vec![],
            item_req: vec![],
            quest_req: vec![],
            varp_req: vec![],
            worn_req: vec![],
        };
        assert!(
            !is_essence_entry_edge(&cart),
            "a cart driver is not an entry hop"
        );
    }

    #[test]
    fn the_return_edge_targets_only_the_session_wizard() {
        let aubury = essence_session_for_wizard(553).unwrap();
        for &portal in ESSENCE_MINE_PORTALS {
            let edge = essence_return_edge(portal, &aubury);
            assert_eq!(edge.kind, TransportKind::EssenceExit);
            assert_eq!(edge.loc_id, ESSENCE_MINE_PORTAL_LOC_ID);
            assert_eq!(edge.option, 1, "the exit is `oploc1`");
            assert_eq!(
                edge.to, aubury.return_tile,
                "returns to the entry wizard only"
            );
        }
        let sedridor = essence_session_for_wizard(300).unwrap();
        let edge = essence_return_edge(ESSENCE_MINE_PORTALS[0], &sedridor);
        assert_eq!(edge.to, sedridor.return_tile);
        assert_ne!(
            edge.to, aubury.return_tile,
            "a different wizard returns elsewhere"
        );
    }

    #[test]
    fn mine_enclosure_covers_the_landings_and_portals() {
        // The four portal placements and the pad sit inside the enclosure;
        // a tile outside the cave walls does not.
        for &p in ESSENCE_MINE_PORTALS {
            assert!(in_essence_mine(p), "portal at {p:?} inside the mine");
        }
        assert!(in_essence_mine(WorldTile {
            x: 2912,
            z: 4833,
            level: 0
        }));
        assert!(in_essence_mine(WorldTile {
            x: 2896,
            z: 4809,
            level: 0
        }));
        assert!(!in_essence_mine(WorldTile {
            x: 3253,
            z: 3401,
            level: 0
        }));
        assert!(!in_essence_mine(WorldTile {
            x: 2879,
            z: 4833,
            level: 0
        }));
        assert!(!in_essence_mine(WorldTile {
            x: 2912,
            z: 4833,
            level: 1
        }));
    }
}
