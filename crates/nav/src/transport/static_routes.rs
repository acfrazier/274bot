use super::*;

/// Destination-ship disembark: `oploc1` `Cross` on the `_gangplank_disembark`
/// loc (`gangplank.rs2`), landing on the dock. Live Port Sarim → Musa
/// stalls on the Musa deck if this hop is folded into the Boat edge —
/// `set_sail` only telejumps onto the ship.
#[derive(Debug, Clone, Copy)]
pub(super) struct DisembarkPlank {
    loc_id: i32,
    at: WorldTile,
    to: WorldTile,
}

/// One 2004 boat journey: talk to the dock NPC at `at`, sail onto the
/// destination ship (`to` = the `set_sail` deck tile). `plank` is the
/// boat-side gangplank off that ship; Shanks `set_sail_cairn` lands on
/// the dock with `plank: None`. `at` is the NPC spawn, never the origin
/// gangplank (board planks refuse until the sailor is spoken to).
#[derive(Debug, Clone, Copy)]
pub(super) struct BoatRoute {
    /// npc.pack id of the dock NPC who starts the journey.
    npc: i32,
    at: WorldTile,
    to: WorldTile,
    plank: Option<DisembarkPlank>,
    /// `set_sail` / `set_sail_cairn` `p_delay` only (plank ticks are on
    /// the disembark edge).
    ticks: i32,
    /// `(obj id, count)` fare the journey charges, if any.
    fare: Option<(i32, i32)>,
    /// `(varp id, min value)` quest gate, if any.
    varp_req: Option<(i32, i32)>,
}

/// The 2004 boat journeys: `~set_sail(` landings from area scripts, NPC
/// tiles from jm2, disembark locs from `gangplank.loc` / loc.pack (jm2
/// placements). Board planks (`*_on`) are not packed — they mes and
/// refuse until the sailor is spoken to.
pub(super) const GANGPLANK_TICKS: i32 = 2;

pub(super) const BOAT_ROUTES: &[BoatRoute] = &[
    // Seaman Thresnor (npc 378) on the Port Sarim pier (m47_50): lands on
    // the Karamja ship (2956,3143,1); `sarimshipplank_off` (loc 2082 at
    // 2956,3144,1, m46_49) Cross to the Karamja dock (2956,3146,0).
    BoatRoute {
        npc: 378,
        at: WorldTile {
            x: 3026,
            z: 3217,
            level: 0,
        },
        to: WorldTile {
            x: 2956,
            z: 3143,
            level: 1,
        },
        plank: Some(DisembarkPlank {
            loc_id: 2082,
            at: WorldTile {
                x: 2956,
                z: 3144,
                level: 1,
            },
            to: WorldTile {
                x: 2956,
                z: 3146,
                level: 0,
            },
        }),
        ticks: 7,
        fare: Some((995, 30)),
        varp_req: None,
    },
    // Customs officer (npc 380) at Musa Point (m46_49): lands on the Port
    // Sarim ship (3032,3217,1); `karamjashipplank_off` (loc 2084 at
    // 3031,3217,1, m47_50) to the Port Sarim dock (3029,3217,0).
    BoatRoute {
        npc: 380,
        at: WorldTile {
            x: 2955,
            z: 3146,
            level: 0,
        },
        to: WorldTile {
            x: 3032,
            z: 3217,
            level: 1,
        },
        plank: Some(DisembarkPlank {
            loc_id: 2084,
            at: WorldTile {
                x: 3031,
                z: 3217,
                level: 1,
            },
            to: WorldTile {
                x: 3029,
                z: 3217,
                level: 0,
            },
        }),
        ticks: 7,
        fare: Some((995, 30)),
        varp_req: None,
    },
    // Customs officer (npc 380) at the Brimhaven dock: lands on the
    // Ardougne ship (2683,3268,1); `brimhavenshipplank_off` (loc 2086 at
    // 2683,3269,1, m41_51) to the Ardougne dock (2683,3271,0).
    BoatRoute {
        npc: 380,
        at: WorldTile {
            x: 2772,
            z: 3231,
            level: 0,
        },
        to: WorldTile {
            x: 2683,
            z: 3268,
            level: 1,
        },
        plank: Some(DisembarkPlank {
            loc_id: 2086,
            at: WorldTile {
                x: 2683,
                z: 3269,
                level: 1,
            },
            to: WorldTile {
                x: 2683,
                z: 3271,
                level: 0,
            },
        }),
        ticks: 7,
        fare: Some((995, 30)),
        varp_req: None,
    },
    // Captain Barnaby (npc 381) at the Ardougne dock: lands on the
    // Brimhaven ship (2775,3234,1); `ardougneshipplank_off` (loc 2088 at
    // 2774,3234,1, m43_50) to the Brimhaven dock (2772,3234,0).
    BoatRoute {
        npc: 381,
        at: WorldTile {
            x: 2679,
            z: 3275,
            level: 0,
        },
        to: WorldTile {
            x: 2775,
            z: 3234,
            level: 1,
        },
        plank: Some(DisembarkPlank {
            loc_id: 2088,
            at: WorldTile {
                x: 2774,
                z: 3234,
                level: 1,
            },
            to: WorldTile {
                x: 2772,
                z: 3234,
                level: 0,
            },
        }),
        ticks: 7,
        fare: Some((995, 30)),
        varp_req: None,
    },
    // Monk of Entrana (shipmonk, npc 657): lands on the Entrana ship
    // (2834,3331,1); `ship_from_entrana_off` (loc 2415 at 2834,3333,1,
    // m44_52) to the Entrana dock (2834,3335,0). Delay 13.
    BoatRoute {
        npc: 657,
        at: WorldTile {
            x: 3049,
            z: 3235,
            level: 0,
        },
        to: WorldTile {
            x: 2834,
            z: 3331,
            level: 1,
        },
        plank: Some(DisembarkPlank {
            loc_id: 2415,
            at: WorldTile {
                x: 2834,
                z: 3333,
                level: 1,
            },
            to: WorldTile {
                x: 2834,
                z: 3335,
                level: 0,
            },
        }),
        ticks: 13,
        fare: None,
        varp_req: None,
    },
    // Monk of Entrana (shipmonk2, npc 658): lands on the Port Sarim ship
    // (3048,3231,1); `ship_to_entrana_off` (loc 2413 at 3048,3232,1,
    // m47_50) to the Port Sarim dock (3048,3234,0). Delay 14.
    BoatRoute {
        npc: 658,
        at: WorldTile {
            x: 2835,
            z: 3336,
            level: 0,
        },
        to: WorldTile {
            x: 3048,
            z: 3231,
            level: 1,
        },
        plank: Some(DisembarkPlank {
            loc_id: 2413,
            at: WorldTile {
                x: 3048,
                z: 3232,
                level: 1,
            },
            to: WorldTile {
                x: 3048,
                z: 3234,
                level: 0,
            },
        }),
        ticks: 14,
        fare: None,
        varp_req: None,
    },
    // Captain Shanks (npc 518) on the deck of the Lady of the Waves (m43_46):
    // `set_sail_cairn` lands directly on the Khazard dock
    // (`0_41_49_56_14`), no destination gangplank. Delay 9. Gated on Shilo
    // Village complete (`%zombiequeen >= ^zombiequeen_complete`).
    BoatRoute {
        npc: 518,
        at: WorldTile {
            x: 2763,
            z: 2961,
            level: 1,
        },
        to: WorldTile {
            x: 2680,
            z: 3150,
            level: 0,
        },
        plank: None,
        ticks: 9,
        fare: None,
        varp_req: Some((116, 15)),
    },
    // Captain Shanks (npc 518) → Port Sarim (`0_47_50_39_35`). Delay 15.
    BoatRoute {
        npc: 518,
        at: WorldTile {
            x: 2763,
            z: 2961,
            level: 1,
        },
        to: WorldTile {
            x: 3047,
            z: 3235,
            level: 0,
        },
        plank: None,
        ticks: 15,
        fare: None,
        varp_req: Some((116, 15)),
    },
];

/// Boat edges from the explicit 2004 route table: one `Talk-to` edge per
/// journey (NPC tile → `set_sail` deck), plus a loc-backed disembark
/// hop (`Cross` on the boat-side gangplank → dock). Kind is Ladder: a
/// level-changing loc op, not an NPC.
pub(super) fn boat_edges(graph: &mut TransportGraph) {
    for r in BOAT_ROUTES {
        graph.edges.push(TransportEdge {
            kind: TransportKind::Boat,
            at: r.at,
            to: r.to,
            loc_id: r.npc,
            option: 1,
            ticks: r.ticks,
            dir: None,
            open_loc_id: None,
            skill_req: vec![],
            item_req: r.fare.map(|(id, n)| vec![(id, n)]).unwrap_or_default(),
            quest_req: vec![],
            varp_req: r.varp_req.map(|v| vec![v]).unwrap_or_default(),
            worn_req: vec![],
            members_req: false,
        });
        if let Some(p) = r.plank {
            graph.edges.push(TransportEdge {
                kind: TransportKind::Ladder,
                at: p.at,
                to: p.to,
                loc_id: p.loc_id,
                option: 1,
                ticks: GANGPLANK_TICKS,
                dir: None,
                open_loc_id: None,
                skill_req: vec![],
                item_req: vec![],
                quest_req: vec![],
                varp_req: vec![],
                worn_req: vec![],
                members_req: false,
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Shilo↔Brimhaven cart: the 2004 route pair (`TransportKind::Npc`).
// ---------------------------------------------------------------------------

/// One Shilo↔Brimhaven cart journey: `at` the cart driver NPC's spawn tile
/// (jm2 `==== NPC ====` placement, id resolved through `pack/npc.pack`),
/// `to` the destination cart tile the script's `p_teleport(` literal lands
/// on. The whole hop is one `Talk-to` (`opnpc1`), and the scripts carry no
/// `p_delay`, so `ticks` is the 1 op base like the spirit trees.
#[derive(Debug, Clone, Copy)]
pub(super) struct CartRoute {
    /// npc.pack id of the cart driver who starts the journey.
    npc: i32,
    at: WorldTile,
    to: WorldTile,
    /// `(obj id, count)` fare: coins (`obj.pack` 995), count = the
    /// `calc_shilocart_cost` clamp cap.
    fare: Option<(i32, i32)>,
    /// The quest journal name gating the journey, if any.
    quest: Option<&'static str>,
}

/// The 2004 cart journeys: destinations from the `p_teleport(` calls in
/// `content/scripts/areas/area_brimhaven/scripts/hajedy.rs2` /
/// `content/scripts/areas/area_shilo/scripts/vigroy.rs2`, origin tiles from
/// the `==== NPC ====` placements in `content/maps/*.jm2`, and ids from
/// `pack/npc.pack`. The fare is `calc_shilocart_cost` in both scripts:
/// `(coins carried * 5) / 100`, clamped to 10–200 coins — the table keeps
/// the 200 cap. Hajedy refuses the ride until Shilo Village is complete
/// (`%zombiequeen >= ^zombiequeen_complete`); Vigroy's block carries no
/// gate.
pub(super) const CART_ROUTES: &[CartRoute] = &[
    // Hajedy (brimhavencartdriver, npc 510) by the Brimhaven cart
    // (m43_50 local (27,11) = 2779,3211): `p_teleport(0_44_46_18_7)`
    // lands at the Shilo Village cart (2834,2951).
    CartRoute {
        npc: 510,
        at: WorldTile {
            x: 2779,
            z: 3211,
            level: 0,
        },
        to: WorldTile {
            x: 2834,
            z: 2951,
            level: 0,
        },
        fare: Some((995, 200)),
        quest: Some("Shilo Village"),
    },
    // Vigroy (shilocartdriver, npc 511) at the Shilo Village cart
    // (m44_46 local (18,10) = 2834,2954): `p_teleport(0_43_50_24_14)`
    // lands at the Brimhaven cart (2776,3214).
    CartRoute {
        npc: 511,
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
        fare: Some((995, 200)),
        quest: None,
    },
];

/// Cart edges from the 2004 route table: one `Talk-to` edge per journey,
/// keyed from the cart driver NPC's tile.
pub(super) fn cart_edges(graph: &mut TransportGraph) {
    for r in CART_ROUTES {
        graph.edges.push(TransportEdge {
            kind: TransportKind::Npc,
            at: r.at,
            to: r.to,
            loc_id: r.npc,
            option: 1,
            ticks: 1,
            dir: None,
            open_loc_id: None,
            skill_req: vec![],
            item_req: r.fare.map(|(id, n)| vec![(id, n)]).unwrap_or_default(),
            quest_req: r.quest.map(|q| vec![q.to_string()]).unwrap_or_default(),
            varp_req: vec![],
            worn_req: vec![],
            members_req: false,
        });
    }
}
