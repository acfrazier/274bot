use super::*;

// ---------------------------------------------------------------------------
// Wilderness levers: the Ardougne↔wilderness teleport pair.
// ---------------------------------------------------------------------------

/// Wilderness lever edges from `scripts/areas/area_ardougne_east/scripts/
/// wilderness_lever.rs2` plus the folder's `wilderness_lever.constant`:
/// each `[oploc1,<loc>]` block's `~player_teleport_normal(^…_coord)` call
/// resolves through the constant's 5-part coord literal. One directed edge
/// per lever loc placement: `at` the lever loc tile (jm2 placement, like
/// every loc-backed edge), `to` the constant's tile, `Pull` op 1, two
/// ticks. Kind stays [`TransportKind::Door`] (the pack wire already
/// carries it; no version bump). The Ardougne→wilderness landing is inside
/// the wilderness zone, so the router only relaxes that edge under
/// `FindOptions::allow_wilderness`; the wilderness→Ardougne landing is not
/// and is always legal. The `%warning_wilderness_teleport_lever` confirm
/// dialog is execute, not search, and carries no edge.
pub(super) fn lever_edges(
    content_root: &Path,
    ids: &HashMap<String, i32>,
    positions: &HashMap<i32, Vec<Placement>>,
    graph: &mut TransportGraph,
    skipped: &mut HashMap<&'static str, usize>,
) {
    let dir = content_root
        .join("scripts")
        .join("areas")
        .join("area_ardougne_east");
    let applicable = wilderness_lever_applicable(ids);
    let Ok(script) = fs::read_to_string(dir.join("scripts").join("wilderness_lever.rs2")) else {
        bump(skipped, SKIP_LEVER_SOURCE, applicable);
        return;
    };
    let Ok(constants) = fs::read_to_string(dir.join("configs").join("wilderness_lever.constant"))
    else {
        bump(skipped, SKIP_LEVER_SOURCE, applicable);
        return;
    };
    // `^name` → the teleport tile (`0_mx_mz_lx_lz`, decoded like every
    // other coord literal).
    let mut lever_dests: HashMap<String, WorldTile> = HashMap::new();
    for raw in constants.lines() {
        let line = raw.trim();
        let Some(rest) = line.strip_prefix('^') else {
            continue;
        };
        let Some((name, coord)) = rest.split_once('=') else {
            continue;
        };
        if let Some((level, x, z)) = coord_literal(coord) {
            let name = name.trim();
            if !name.is_empty() {
                lever_dests.insert(name.to_string(), WorldTile { x, z, level });
            }
        }
    }

    let mut visited_oploc1 = HashSet::new();
    for (op, name, body) in script_blocks(&script) {
        if op != "oploc1" {
            continue;
        }
        visited_oploc1.insert(name.clone());
        let edge_start = graph.edges.len();
        let Some(&loc_id) = ids.get(&name) else {
            continue;
        };
        let Some(placements) = positions.get(&loc_id) else {
            bump(skipped, SKIP_LEVER_ROUTE, 1);
            continue;
        };
        let mut tos = Vec::new();
        for args in call_args_all(&body, "~player_teleport_normal") {
            let Some(dest) = args.first().and_then(|a| a.trim().strip_prefix('^')) else {
                continue;
            };
            if let Some(to) = lever_dests.get(dest) {
                tos.push(*to);
            }
        }
        for loc in placements {
            let at = WorldTile {
                x: loc.x,
                z: loc.z,
                level: loc.level,
            };
            for to in &tos {
                graph.edges.push(TransportEdge {
                    kind: TransportKind::Door,
                    at,
                    to: *to,
                    loc_id,
                    option: 1, // Pull (oploc1)
                    ticks: LEVER_TICKS,
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
        if graph.edges.len() == edge_start {
            bump(skipped, SKIP_LEVER_ROUTE, 1);
        }
    }
    // Packed declared names whose `[oploc1,name]` block never appeared.
    // Visited names already used SKIP_LEVER_ROUTE on zero-emission.
    for name in WILDERNESS_LEVER_LOC_NAMES {
        if ids.contains_key(*name) && !visited_oploc1.contains(*name) {
            bump(skipped, SKIP_LEVER_ROUTE, 1);
        }
    }
}
