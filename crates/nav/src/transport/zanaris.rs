use super::*;

// ---------------------------------------------------------------------------
// The Zanaris shed door (`quest_zanaris.rs2`): a worn-item teleport door.
// ---------------------------------------------------------------------------

/// The Zanaris shed door ticks: OP_BASE 1, the door block's `p_delay(1)`,
/// and the `player_teleport_normal` cast `p_delay(2)` (the whole Open
/// channel; the shimmer `mes` and the open anim add no delay).
pub(super) const ZANARIS_DOOR_TICKS: i32 = 4;

/// The Zanaris shed door edge from `scripts/quests/quest_zanaris/scripts/
/// quest_zanaris.rs2`'s `[oploc1,zanarisdoor]` block: the door opens
/// (`~open_and_close_door2(loc_1532, $entering, door_open)` —
/// `open_loc_id` the `loc_1532` open leaf) and, approached from the
/// outside, teleports through to Zanaris
/// (`~player_teleport_normal(0_50_149_20_56)` = (3220,9592)) when the
/// player wears the Dramen staff (`inv_total(worn, dramen_staff) > 0` →
/// `worn_req`) and is a member (`map_members = ^true` — the members flag
/// the bot host already tracks in WorldState, so nothing extra is stored).
/// The Lost City quest varp (`%zanaris`) gates the content, carried as the
/// quest name. One edge per placement (a single m50_49 placement at the
/// Lumbridge swamp shed): `at` the door loc tile, `to` the Zanaris
/// landing. No other Zanaris locs derive — no fairy rings, no Entrana
/// dungeon magic door, no `zanarismagicdoor`/`zanarismarketdoor`/
/// `zanarisladderout` hops.
pub(super) fn zanaris_door_edges(
    content_root: &Path,
    ids: &HashMap<String, i32>,
    positions: &HashMap<i32, Vec<Placement>>,
    graph: &mut TransportGraph,
    skipped: &mut HashMap<&'static str, usize>,
) {
    let Ok(script) = fs::read_to_string(
        content_root
            .join("scripts")
            .join("quests")
            .join("quest_zanaris")
            .join("scripts")
            .join("quest_zanaris.rs2"),
    ) else {
        if ids.contains_key("zanarisdoor") {
            bump(skipped, SKIP_ZANARIS_SOURCE, ZANARIS_DECLARED_ROUTES);
        }
        return;
    };
    let Some((_, name, body)) = script_blocks(&script)
        .into_iter()
        .find(|(op, name, _)| op.as_str() == "oploc1" && name.as_str() == "zanarisdoor")
    else {
        if ids.contains_key("zanarisdoor") {
            bump(skipped, SKIP_ZANARIS_SOURCE, ZANARIS_DECLARED_ROUTES);
        }
        return;
    };
    let Some(&loc_id) = ids.get(&name) else {
        return;
    };
    // The open leaf: `~open_and_close_door2(loc_1532, $entering, …)`.
    let open_loc_id = call_args(&body, "open_and_close_door2")
        .and_then(|args| args.first().cloned())
        .and_then(|leaf| {
            leaf.trim()
                .strip_prefix("loc_")
                .and_then(|n| n.parse::<i32>().ok())
        });
    // The teleport landing: `~player_teleport_normal(0_50_149_20_56)`.
    let Some(to) = call_args(&body, "player_teleport_normal")
        .and_then(|args| args.first().cloned())
        .and_then(|dest| coord_literal(&dest))
        .map(|(level, x, z)| WorldTile { x, z, level })
    else {
        bump(skipped, SKIP_ZANARIS_ROUTE, ZANARIS_DECLARED_ROUTES);
        return;
    };
    // The Dramen staff id (`pack/obj.pack`); a missing pack skips the
    // door instead of faking an id.
    let Some(&staff_id) = obj_ids_by_name(content_root).get("dramen_staff") else {
        bump(skipped, SKIP_ZANARIS_ROUTE, ZANARIS_DECLARED_ROUTES);
        return;
    };
    let Some(placements) = positions.get(&loc_id) else {
        bump(skipped, SKIP_ZANARIS_ROUTE, ZANARIS_DECLARED_ROUTES);
        return;
    };
    let edge_start = graph.edges.len();
    for loc in placements {
        if loc.level != 0 || loc.shape != 0 {
            continue;
        }
        graph.edges.push(TransportEdge {
            kind: TransportKind::Door,
            at: WorldTile {
                x: loc.x,
                z: loc.z,
                level: loc.level,
            },
            to,
            loc_id,
            option: 1, // Open (oploc1)
            ticks: ZANARIS_DOOR_TICKS,
            dir: None,
            open_loc_id,
            skill_req: vec![],
            item_req: vec![],
            quest_req: vec!["Lost City".to_string()],
            varp_req: vec![],
            worn_req: vec![staff_id],
            members_req: false,
            wildy_cap: None,
        });
    }
    if graph.edges.len() == edge_start {
        bump(skipped, SKIP_ZANARIS_ROUTE, ZANARIS_DECLARED_ROUTES);
    }
}
