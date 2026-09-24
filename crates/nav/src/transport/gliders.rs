use super::*;

// ---------------------------------------------------------------------------
// Gnome gliders: the 2004 Gnome Air network (fixed platform table).
// ---------------------------------------------------------------------------

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

/// Glider edges from the fixed platform table: the hub to every pad, and
/// back from the round-trip pads. `calc_glidervar` permits only hub↔pad
/// flights; Lemanto Andra is one-way. The pilot's completed Grand Tree gate
/// is linked to its observable quest journal row. `%grandtree` is not
/// transmitted, so no varp-only alternative is emitted.
pub(super) fn glider_edges(content_root: &Path, graph: &mut TransportGraph) {
    let Ok(script) =
        fs::read_to_string(content_root.join("scripts/areas/area_gnome/scripts/gnome_glider.rs2"))
    else {
        return;
    };
    let Some((_, _, body)) = script_blocks(&script)
        .into_iter()
        .find(|(op, name, _)| op == "opnpc1" && name == "gnomepilot")
    else {
        return;
    };
    if !body
        .lines()
        .any(|line| line.trim() == "if(%grandtree = ^grandtree_complete & map_members = ^true) {")
        || !body.lines().any(|line| line.contains("gnome_pilot_glider"))
    {
        return;
    }
    let journal = JournalLinks::from_content(content_root);
    let Some(complete) = journal.constant("grandtree_complete") else {
        return;
    };
    let Some(quest) = journal.completed_name("grandtree", complete) else {
        return;
    };
    for (pad, round_trip) in GLIDER_PADS {
        push_glider_flight(graph, GLIDER_HUB, *pad, quest);
        if *round_trip {
            push_glider_flight(graph, *pad, GLIDER_HUB, quest);
        }
    }
}

pub(super) fn push_glider_flight(
    graph: &mut TransportGraph,
    at: WorldTile,
    to: WorldTile,
    quest: &str,
) {
    graph.edges.push(TransportEdge {
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
        quest_req: vec![quest.to_string()],
        varp_req: vec![],
        worn_req: vec![],
        members_req: false,
    });
}
