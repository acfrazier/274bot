use super::*;

// ---------------------------------------------------------------------------
// Edgeville brass-key hut door.
// ---------------------------------------------------------------------------

pub(super) const BRASS_KEY_DOOR_NAME: &str = "brasskeydoor";
pub(super) const BRASS_KEY_NAME: &str = "edgevilledungeonkey";
pub(super) const BRASS_KEY_OPEN_LABEL: &str = "open_edgeville_dungeon_door";
pub(super) const BRASS_KEY_LOCKED_HANDLER: &str = r#"mes("The door is locked.");"#;
pub(super) const BRASS_KEY_USE_HANDLER: &str = r#"
switch_obj(last_useitem) {
    case edgevilledungeonkey : @open_edgeville_dungeon_door;
    case default : ~displaymessage(^dm_default);
}
"#;
pub(super) const BRASS_KEY_OPEN_HANDLER: &str = r#"
if (inv_total(inv, edgevilledungeonkey) > 0) {
    mes("You unlock the door.");
    sound_synth(locked, 1, 0);
    def_coord $loc_coord = loc_coord;
    def_int $angle = loc_angle;
    def_locshape $shape = loc_shape;
    def_loc $replacement = loc_param(next_loc_stage);
    def_int $x;
    def_int $z;
    $x, $z = ~door_open($angle, loc_shape);
    def_boolean $entering = ~check_axis(coord, $loc_coord, $angle);
    def_coord $dest = $loc_coord;
    if ($entering = true) {
        if (coord ! $loc_coord) {
            p_delay(0);
            p_teleport($loc_coord);
            sound_synth(door_open, 1, 0);
            p_delay(1);
        } else {
            p_delay(0);
            sound_synth(door_open, 1, 0);
        }
        $dest = movecoord($loc_coord, $x, 0, $z);
    } else {
        p_delay(0);
        sound_synth(door_open, 1, 0);
    }
    p_teleport($dest);
    loc_add($loc_coord, inviswall, $angle, $shape, 3);
    loc_add(movecoord($loc_coord, $x, 0, $z), $replacement, modulo(add($angle, 1), 4), $shape, 3);
} else {
    mes("The door is locked.");
}
"#;

/// The Edgeville surface-hut door is not a generic `Open` door:
/// `[oploc1,brasskeydoor]` is locked, while the canonical `oplocu` handler
/// admits only `edgevilledungeonkey`, verifies it is still held, teleports
/// across, and temporarily replaces the closed leaf with `inviswall` plus
/// the configured `next_loc_stage` leaf at `door_open(angle, shape)`.
///
/// The two edges preserve those source endpoints. Entering starts at the
/// closed placement and lands on the replacement tile; leaving starts at
/// that replacement tile and lands on the original placement. This differs
/// deliberately from the generic-door `at ± 1` pair. Every alias, handler,
/// config, placement shape, and endpoint must resolve or the family is
/// omitted.
pub(super) fn brass_key_door_edges(
    content_root: &Path,
    ids: &HashMap<String, i32>,
    positions: &HashMap<i32, Vec<Placement>>,
    graph: &mut TransportGraph,
    skipped: &mut HashMap<&'static str, usize>,
    collision: &WorldCollision,
) {
    let Some(&door_id) = ids.get(BRASS_KEY_DOOR_NAME) else {
        bump(skipped, SKIP_BRASS_KEY_SOURCE, 1);
        return;
    };
    let objs = obj_ids_by_name(content_root);
    let Some(&key_id) = objs.get(BRASS_KEY_NAME) else {
        bump(skipped, SKIP_BRASS_KEY_SOURCE, 1);
        return;
    };
    let Some(open_id) = brass_key_open_loc_id(content_root, ids, door_id) else {
        bump(skipped, SKIP_BRASS_KEY_SOURCE, 1);
        return;
    };
    if !brass_key_handler_matches(content_root) {
        bump(skipped, SKIP_BRASS_KEY_SOURCE, 1);
        return;
    }
    let Some(placements) = positions.get(&door_id) else {
        return;
    };
    for placement in placements {
        if placement.level != 0 || placement.shape != 0 {
            bump(skipped, SKIP_BRASS_KEY_SHAPE, 1);
            continue;
        }
        let Some(open_dir) = door_dir(placement.angle) else {
            bump(skipped, SKIP_BRASS_KEY_SHAPE, 1);
            continue;
        };
        let closed = WorldTile {
            x: placement.x,
            z: placement.z,
            level: placement.level,
        };
        let Some(replacement) = door_far_side(closed, open_dir, collision) else {
            bump(skipped, SKIP_BRASS_KEY_ENDPOINT, 1);
            continue;
        };
        if !collision.standable(closed) {
            bump(skipped, SKIP_BRASS_KEY_ENDPOINT, 1);
            continue;
        }
        for (at, to, dir) in [
            (closed, replacement, open_dir),
            (replacement, closed, opposite(open_dir)),
        ] {
            graph.edges.push(TransportEdge {
                kind: TransportKind::Door,
                at,
                to,
                loc_id: door_id,
                option: 0,
                ticks: 1,
                dir: Some(dir),
                open_loc_id: Some(open_id),
                skill_req: vec![],
                item_req: vec![(key_id, 1)],
                quest_req: vec![],
                varp_req: vec![],
                worn_req: vec![],
                members_req: false,
                wildy_cap: None,
            });
        }
    }
}

/// Resolve the brass-key door's configured replacement leaf. Duplicate
/// identical declarations are harmless; a conflicting declaration omits the
/// family.
pub(super) fn brass_key_open_loc_id(
    content_root: &Path,
    ids: &HashMap<String, i32>,
    door_id: i32,
) -> Option<i32> {
    let mut found = None;
    let mut conflicted = false;
    visit_loc_configs(&content_root.join("scripts"), &mut |text| {
        if !parse_door_config_ids(text, ids).contains(&door_id) {
            return;
        }
        let Some(open) = parse_door_open_ids(text, ids).get(&door_id).copied() else {
            conflicted = true;
            return;
        };
        if found.is_some_and(|previous| previous != open) {
            conflicted = true;
        } else {
            found = Some(open);
        }
    });
    (!conflicted).then_some(found).flatten()
}

/// Exact selected-content handler check. Whitespace and comments may move,
/// but the locked normal op, key-only item-use switch, non-consuming
/// inventory guard, source teleport, and temporary replacement sequence may
/// not change silently.
pub(super) fn brass_key_handler_matches(content_root: &Path) -> bool {
    let path = content_root
        .join("scripts")
        .join("areas")
        .join("area_edgeville")
        .join("scripts")
        .join("edgeville_dungeon.rs2");
    let Ok(text) = fs::read_to_string(path) else {
        return false;
    };
    let handler_matches = |op: &str, name: &str, expected: &str| {
        let bodies = selected_script_bodies(&text, op, name);
        let mut matching = bodies.iter().map(String::as_str);
        let Some(body) = matching.next() else {
            return false;
        };
        matching.next().is_none() && normalized_body(body) == normalized_body(expected)
    };
    handler_matches("oploc1", BRASS_KEY_DOOR_NAME, BRASS_KEY_LOCKED_HANDLER)
        && handler_matches("oplocu", BRASS_KEY_DOOR_NAME, BRASS_KEY_USE_HANDLER)
        && handler_matches("label", BRASS_KEY_OPEN_LABEL, BRASS_KEY_OPEN_HANDLER)
}

/// Bodies of one selected `[op,name]` block, stopping at the next header
/// even when that next header has an inline body. `script_blocks` retains
/// its historical next-line-only behavior; the brass-key source ends with
/// inline odd-wall handlers that must not be mistaken for label content.
pub(super) fn selected_script_bodies(
    text: &str,
    selected_op: &str,
    selected_name: &str,
) -> Vec<String> {
    let mut out = Vec::new();
    let mut selected = false;
    let mut body = String::new();
    for raw in text.lines() {
        let header_line = raw
            .split_once("//")
            .map_or(raw, |(before, _)| before)
            .trim();
        if let Some(rest) = header_line.strip_prefix('[') {
            if let Some((header, inline)) = rest.split_once(']') {
                if selected {
                    out.push(std::mem::take(&mut body));
                }
                let mut parts = header.split(',').map(str::trim);
                selected = parts.next() == Some(selected_op)
                    && parts.next() == Some(selected_name)
                    && parts.next().is_none();
                if selected && !inline.trim().is_empty() {
                    body.push_str(inline.trim());
                    body.push('\n');
                }
                continue;
            }
        }
        if selected {
            body.push_str(raw);
            body.push('\n');
        }
    }
    if selected {
        out.push(body);
    }
    out
}
