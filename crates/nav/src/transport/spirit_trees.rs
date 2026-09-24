use super::*;

/// Spirit-tree edges from `scripts/areas/area_gnome/scripts/spirit_tree.rs2`
/// plus the same folder's `spirit_tree.constant`: each `[oploc1,<loc>]`
/// block lists its destinations as `^…_tree` constants (a `$end_pos = ^…`
/// assignment on every `case` line, or a direct `@spirit_tree_tele(^…)`
/// call for the young tree's single destination). One directed edge per
/// tree loc placement per destination: `at` the tree loc tile (jm2
/// placement, like every loc-backed edge), `to` the destination constant's
/// tile, `Talk-to` op 1, one tick. `varp_req` carries the quest gate the
/// block checks (`%grandtree` / `%treequest` complete values, the same
/// varps the gliders gate on); the members check in `spirit_tree_tele` is
/// not a varp and is left off until WorldState.
pub(super) fn spirit_tree_edges(
    content_root: &Path,
    ids: &HashMap<String, i32>,
    positions: &HashMap<i32, Vec<Placement>>,
    graph: &mut TransportGraph,
    skipped: &mut HashMap<&'static str, usize>,
) {
    let Ok(script) = fs::read_to_string(
        content_root
            .join("scripts")
            .join("areas")
            .join("area_gnome")
            .join("scripts")
            .join("spirit_tree.rs2"),
    ) else {
        return;
    };
    let Ok(constants) = fs::read_to_string(
        content_root
            .join("scripts")
            .join("areas")
            .join("area_gnome")
            .join("configs")
            .join("spirit_tree.constant"),
    ) else {
        return;
    };
    // `^name` → the tree's tile (`0_mx_mz_lx_lz`, decoded like every other
    // coord literal).
    let mut tree_dests: HashMap<String, WorldTile> = HashMap::new();
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
                tree_dests.insert(name.to_string(), WorldTile { x, z, level });
            }
        }
    }
    let all_consts = script_constants(content_root);
    let varps = varp_ids_by_name(content_root);

    for (op, name, body) in script_blocks(&script) {
        if op != "oploc1" {
            continue;
        }
        let Some(&loc_id) = ids.get(&name) else {
            continue;
        };
        let Some(placements) = positions.get(&loc_id) else {
            continue;
        };
        let mut dests = Vec::new();
        for const_name in spirit_tree_dest_names(&body) {
            if let Some(to) = tree_dests.get(&const_name) {
                dests.push(*to);
            }
        }
        if dests.is_empty() {
            bump(skipped, SKIP_SPIRIT_NO_DEST, 1);
            continue;
        }
        let varp_req = spirit_tree_gate(&body)
            .and_then(|(varp, complete)| {
                let varp_id = varps.get(&varp)?;
                let value = all_consts.get(&complete)?;
                Some(vec![(*varp_id, *value)])
            })
            .unwrap_or_default();
        for loc in placements {
            let at = WorldTile {
                x: loc.x,
                z: loc.z,
                level: loc.level,
            };
            for to in &dests {
                graph.edges.push(TransportEdge {
                    kind: TransportKind::SpiritTree,
                    at,
                    to: *to,
                    loc_id,
                    option: 1,
                    ticks: SPIRIT_TREE_TICKS,
                    dir: None,
                    open_loc_id: None,
                    skill_req: vec![],
                    item_req: vec![],
                    quest_req: vec![],
                    varp_req: varp_req.clone(),
                    worn_req: vec![],
                    members_req: false,
                });
            }
        }
    }
}

/// The `^<const>` destination names a spirit-tree `[oploc1,…]` block lists:
/// the `$end_pos = ^…` assignments on `case` lines and direct
/// `@spirit_tree_tele(^…)` calls. The block's initial `def_coord $end_pos =
/// ^…` default is the tree's own tile (overridden by every case), never a
/// destination.
pub(super) fn spirit_tree_dest_names(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    for raw in body.lines() {
        let line = raw.trim();
        if line.starts_with("case") {
            if let Some(i) = line.find("$end_pos = ^") {
                if let Some(name) = const_token(&line[i + "$end_pos = ^".len()..]) {
                    out.push(name.to_string());
                }
            }
        }
        if let Some(i) = line.find("spirit_tree_tele(^") {
            if let Some(name) = const_token(&line[i + "spirit_tree_tele(^".len()..]) {
                out.push(name.to_string());
            }
        }
    }
    out
}

/// The leading identifier token (alphanumerics + `_`).
pub(super) fn const_token(rest: &str) -> Option<&str> {
    let end = rest
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(rest.len());
    if end > 0 {
        Some(&rest[..end])
    } else {
        None
    }
}

/// The `(varp name, complete constant)` gate a spirit-tree block declares
/// (`if(%<varp> ! ^<complete>)` — the tree refuses to talk until the quest
/// is done), or `None` for an un-gated block.
pub(super) fn spirit_tree_gate(body: &str) -> Option<(String, String)> {
    for raw in body.lines() {
        let line = raw.trim();
        let Some(rest) = line.strip_prefix("if(%") else {
            continue;
        };
        let varp = const_token(rest)?;
        let rest = rest[varp.len()..].trim_start().strip_prefix('!')?;
        let complete = const_token(rest.trim_start().strip_prefix('^')?)?;
        if !varp.is_empty() {
            return Some((varp.to_string(), complete.to_string()));
        }
    }
    None
}
