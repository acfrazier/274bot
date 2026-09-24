use super::*;

/// The Grand Tree glider hub (Ta Quir Priw): `^ta_quir_priw =
/// 3_38_54_33_45` in `scripts/areas/area_gnome/configs/glider.constant`
/// (the Gnome pilot spawns one tile west).
pub(super) const GLIDER_HUB: WorldTile = WorldTile {
    x: 2465,
    z: 3501,
    level: 3,
};

/// The four glider pads and their platforms, decoded from `glider.constant`
/// (`^gandius = 0_46_46_27_25` → (2971,2969), `^sindarpos = 0_44_54_34_41`,
/// `^lemanto_andra = 0_51_53_56_38`, `^kar_hewo = 0_51_50_20_11`). The
/// second field is whether the pad has a return flight to the hub.
pub(super) const GLIDER_PADS: &[(WorldTile, bool)] = &[
    (
        WorldTile {
            x: 2971,
            z: 2969,
            level: 0,
        },
        true,
    ), // Gandius (Gnome Stronghold)
    (
        WorldTile {
            x: 2850,
            z: 3497,
            level: 0,
        },
        true,
    ), // Sindarpos (Al Kharid)
    (
        WorldTile {
            x: 3320,
            z: 3430,
            level: 0,
        },
        false,
    ), // Lemanto Andra (Varrock): one-way
    (
        WorldTile {
            x: 3284,
            z: 3211,
            level: 0,
        },
        true,
    ), // Kar-Hewo (Karamja)
];

/// Gnome pilot (npc.pack 170): the `Talk-to` target at every platform.
pub(super) const GNOME_PILOT: i32 = 170;

/// The glider quest gate: the pilot offers Gnome Air only once the Grand
/// Tree quest is complete (`%grandtree >= ^grandtree_complete`, varp 150
/// = 160 in `scripts/areas/area_gnome/scripts/gnome_glider.rs2`'s
/// `[opnpc1,gnomepilot]` block). Live `WorldState` may prove that as the
/// varp **or** as the green journal row (the kit waits `QuestDone`);
/// missing varps fail closed, so each flight packs both proofs.
pub(super) const GLIDER_QUEST_REQ: (i32, i32) = (150, 160);
pub(super) const GLIDER_QUEST_NAME: &str = "The Grand Tree";

/// Glider edges from the fixed platform table: the hub to every pad, and
/// back from the round-trip pads. `calc_glidervar` in `gnome_glider.rs2`
/// allows only hub↔pad flights (pad↔pad shows "You can't go there at the
/// moment."), and has no lemanto_andra → hub pair, so Lemanto Andra is
/// one-way. The flight is a `p_delay(3)` + teleport on top of the
/// `Talk-to` op.
pub(super) fn glider_edges(graph: &mut TransportGraph) {
    for (pad, round_trip) in GLIDER_PADS {
        push_glider_flight(graph, GLIDER_HUB, *pad);
        if *round_trip {
            push_glider_flight(graph, *pad, GLIDER_HUB);
        }
    }
}

pub(super) fn push_glider_flight(graph: &mut TransportGraph, at: WorldTile, to: WorldTile) {
    graph.edges.push(glider_edge(at, to, true));
    graph.edges.push(glider_edge(at, to, false));
}

pub(super) fn glider_edge(at: WorldTile, to: WorldTile, varp_gate: bool) -> TransportEdge {
    TransportEdge {
        kind: TransportKind::Glider,
        at,
        to,
        loc_id: GNOME_PILOT,
        option: 1,
        ticks: 4,
        dir: None,
        open_loc_id: None,
        skill_req: vec![],
        item_req: vec![],
        quest_req: if varp_gate {
            vec![]
        } else {
            vec![GLIDER_QUEST_NAME.to_string()]
        },
        varp_req: if varp_gate {
            vec![GLIDER_QUEST_REQ]
        } else {
            vec![]
        },
        worn_req: vec![],
        members_req: false,
    }
}
