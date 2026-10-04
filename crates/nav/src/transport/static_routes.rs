use super::*;

// ---------------------------------------------------------------------------
// Boats: the 2004 dock-NPC journeys (explicit route table). Teleports are
// the any-tile layer (see `teleport_edges` in `teleports`).
// ---------------------------------------------------------------------------

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

/// One of the two Entrana board crossings modeled in this slice.
#[derive(Debug, Clone, Copy)]
struct BoardPlank {
    loc_id: i32,
    at: WorldTile,
    to: WorldTile,
}

/// One 2004 boat journey: talk to the dock NPC at `at`, sail onto the
/// destination ship (`to` = the `set_sail` deck tile). `plank` is the
/// boat-side gangplank off that ship; Shanks `set_sail_cairn` lands on
/// the dock with `plank: None`. `at` is the NPC spawn, never the origin
/// gangplank.
#[derive(Debug, Clone, Copy)]
pub(super) struct BoatRoute {
    /// npc.pack id of the dock NPC who starts the journey.
    npc: i32,
    at: WorldTile,
    to: WorldTile,
    plank: Option<DisembarkPlank>,
    /// `set_sail` / `set_sail_cairn` `p_delay` only (gangplank ticks are on
    /// their loc edges).
    ticks: i32,
    /// `(obj id, count)` fare the journey charges, if any.
    fare: Option<(i32, i32)>,
    /// Raw `(varp id, min value)` quest gate. The pack binder replaces
    /// non-transmitted varps with proven completed journal names.
    varp_req: Option<(i32, i32)>,
}

/// The 2004 boat journeys: `~set_sail(` landings from area scripts, NPC
/// tiles from jm2, and disembark locs from `gangplank.loc` / loc.pack.
/// The two Entrana board planks are included below; the other four board
/// planks are intentionally out of scope, not treated as movement refusals.
pub(super) const GANGPLANK_TICKS: i32 = 2;

/// Entrana `*_on` crossings are the only board planks without a loc-level
/// `board_message` or quest gate. At the map's north-facing loc angle,
/// `p_teleport` shifts one tile south before `p_telejump` shifts two more
/// south and one level up.
const UNGUARDED_BOARD_PLANKS: &[BoardPlank] = &[
    BoardPlank {
        loc_id: 2412, // ship_to_entrana_on
        at: WorldTile {
            x: 3048,
            z: 3233,
            level: 0,
        },
        to: WorldTile {
            x: 3048,
            z: 3230,
            level: 1,
        },
    },
    BoardPlank {
        loc_id: 2414, // ship_from_entrana_on
        at: WorldTile {
            x: 2834,
            z: 3334,
            level: 0,
        },
        to: WorldTile {
            x: 2834,
            z: 3331,
            level: 1,
        },
    },
];

/// Store each gangplank's script displacement relative to the player's
/// `coord`: `p_teleport` shifts one tile, then `p_telejump` shifts two more
/// from the updated coordinate. `to` is the canonical landing from the loc
/// anchor.
fn gangplank_edge(loc_id: i32, at: WorldTile, to: WorldTile) -> TransportEdge {
    TransportEdge {
        kind: TransportKind::Ladder,
        player_delta: Some(WorldTile {
            x: (to.x - at.x).signum() * 3,
            z: (to.z - at.z).signum() * 3,
            level: to.level - at.level,
        }),
        at,
        to,
        loc_id,
        option: 1,
        ticks: GANGPLANK_TICKS,
        dir: None,
        open_loc_id: None,
        skill_req: vec![],
        item_req: vec![],
        consumed_req: vec![],
        item_returns: vec![],
        quest_req: vec![],
        varp_req: vec![],
        worn_req: vec![],
        members_req: false,
        wildy_cap: None,
        quest_gates: None,
    }
}

pub(super) const BOAT_ROUTES: &[BoatRoute] = &[
    // Port Sarim → Musa: Talk-to lands on the Karamja ship (2956,3143,1);
    // loc 2082 at (2956,3144,1) has canonical landing (2956,3147,0).
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
                z: 3147,
                level: 0,
            },
        }),
        ticks: 7,
        fare: Some((995, 30)),
        varp_req: None,
    },
    // Musa → Port Sarim: Talk-to lands on the Port Sarim ship; loc 2084 at
    // (3031,3217,1) has canonical anchor landing (3028,3217,0).
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
                x: 3028,
                z: 3217,
                level: 0,
            },
        }),
        ticks: 7,
        fare: Some((995, 30)),
        varp_req: None,
    },
    // Brimhaven → Ardougne: Talk-to lands on the Ardougne ship; loc 2086 at
    // (2683,3269,1) has canonical anchor landing (2683,3272,0).
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
                z: 3272,
                level: 0,
            },
        }),
        ticks: 7,
        fare: Some((995, 30)),
        varp_req: None,
    },
    // Ardougne → Brimhaven: Talk-to lands on the Brimhaven ship; loc 2088 at
    // (2774,3234,1) has canonical anchor landing (2771,3234,0).
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
                x: 2771,
                z: 3234,
                level: 0,
            },
        }),
        ticks: 7,
        fare: Some((995, 30)),
        varp_req: None,
    },
    // Port Sarim → Entrana: Talk-to lands on the Entrana ship; loc 2415 at
    // (2834,3333,1) has canonical anchor landing (2834,3336,0). Delay 13.
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
                z: 3336,
                level: 0,
            },
        }),
        ticks: 13,
        fare: None,
        varp_req: None,
    },
    // Entrana → Port Sarim: Talk-to lands on the Port Sarim ship; loc 2413 at
    // (3048,3232,1) has canonical anchor landing (3048,3235,0). Delay 14.
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
                z: 3235,
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
/// journey (NPC tile → `set_sail` deck), plus loc-backed gangplank crossings.
/// Kind is Ladder: these are level-changing loc ops, not NPCs.
pub(super) fn boat_edges(
    graph: &mut TransportGraph,
    gates: &ObservableGates,
    audit: &mut VarpGateAudit,
) {
    for r in BOAT_ROUTES {
        gates.admit_edge(
            graph,
            TransportEdge {
                kind: TransportKind::Boat,
                player_delta: None,
                at: r.at,
                to: r.to,
                loc_id: r.npc,
                option: 1,
                ticks: r.ticks,
                dir: None,
                open_loc_id: None,
                skill_req: vec![],
                item_req: vec![],
                consumed_req: r.fare.map(|(id, n)| vec![(id, n)]).unwrap_or_default(),
                item_returns: vec![],
                quest_req: vec![],
                varp_req: r.varp_req.map(|v| vec![v]).unwrap_or_default(),
                worn_req: vec![],
                members_req: false,
                wildy_cap: None,
                quest_gates: None,
            },
            audit,
        );
        if let Some(p) = r.plank {
            graph.edges.push(gangplank_edge(p.loc_id, p.at, p.to));
        }
    }

    // Locs 2081, 2083, 2085, and 2087 are intentionally omitted from this
    // slice: their handler moves the player before printing `board_message`.
    for p in UNGUARDED_BOARD_PLANKS {
        graph.edges.push(gangplank_edge(p.loc_id, p.at, p.to));
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
    /// `(obj id, count)` minimum fare: the live percentage is evaluated
    /// against the carried balance by [`cart_fare`].
    fare: Option<(i32, i32)>,
    /// Whether the content's Zombie Queen completion gates this direction.
    requires_zombie_queen: bool,
}

/// The 2004 cart journeys: destinations from the `p_teleport(` calls in
/// `content/scripts/areas/area_brimhaven/scripts/hajedy.rs2` /
/// `content/scripts/areas/area_shilo/scripts/vigroy.rs2`, origin tiles from
/// the `==== NPC ====` placements in `content/maps/*.jm2`, and ids from
/// `pack/npc.pack`. Both routes use `calc_shilocart_cost`:
/// `(coins carried * 5) / 100`, clamped to 10–200 coins — packed supply
/// holds the 10-coin minimum, not the maximum. Hajedy's completion gate is
/// resolved to the quest journal's display name from current content;
/// Vigroy's block carries no gate.
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
        fare: Some((995, 10)),
        requires_zombie_queen: true,
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
        fare: Some((995, 10)),
        requires_zombie_queen: false,
    },
];

/// The live content's `calc_shilocart_cost`, for these exact route-table
/// edges only. A static packed minimum must not become a fixed debit.
pub(super) fn cart_fare(edge: &TransportEdge, id: i32, carried: i32) -> Option<i32> {
    if edge.kind != TransportKind::Npc {
        return None;
    }
    CART_ROUTES
        .iter()
        .any(|route| {
            edge.kind == TransportKind::Npc
                && edge.loc_id == route.npc
                && edge.at == route.at
                && edge.to == route.to
                && route.fare.is_some_and(|(coin, _)| coin == id)
        })
        .then(|| ((i64::from(carried.max(0)) * 5) / 100).clamp(10, 200) as i32)
}

/// Cart edges from the 2004 route table: one `Talk-to` edge per journey,
/// keyed from the cart driver NPC's tile.
pub(super) fn cart_edges(
    graph: &mut TransportGraph,
    zombie_queen_name: Option<&str>,
    skipped: &mut HashMap<&'static str, usize>,
) {
    for r in CART_ROUTES {
        let quest_req = if r.requires_zombie_queen {
            let Some(name) = zombie_queen_name else {
                bump(skipped, SKIP_HAJEDY_JOURNAL_NAME, 1);
                continue;
            };
            vec![name.to_string()]
        } else {
            Vec::new()
        };
        graph.edges.push(TransportEdge {
            kind: TransportKind::Npc,
            player_delta: None,
            at: r.at,
            to: r.to,
            loc_id: r.npc,
            option: 1,
            ticks: 1,
            dir: None,
            open_loc_id: None,
            skill_req: vec![],
            item_req: vec![],
            consumed_req: r.fare.map(|(id, n)| vec![(id, n)]).unwrap_or_default(),
            item_returns: vec![],
            quest_req,
            varp_req: vec![],
            worn_req: vec![],
            members_req: false,
            wildy_cap: None,
            quest_gates: None,
        });
    }
}
