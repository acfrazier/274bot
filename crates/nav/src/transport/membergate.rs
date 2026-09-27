use super::*;

// ---------------------------------------------------------------------------
// Canonical membergate family (membergatel / membergater).
// ---------------------------------------------------------------------------

pub(super) const MEMBERGATE_LEFT: &str = "membergatel";
pub(super) const MEMBERGATE_RIGHT: &str = "membergater";
pub(super) const MEMBERGATE_LEFT_CLOSED: &str = "door_left_closed";
pub(super) const MEMBERGATE_RIGHT_CLOSED: &str = "door_right_closed";
pub(super) const MEMBERGATE_LEFT_OPENED: &str = "door_left_opened";
pub(super) const MEMBERGATE_RIGHT_OPENED: &str = "door_right_opened";
pub(super) const MEMBERGATE_LEFT_OPEN: &str =
    "if(map_members=^false){mes(^mes_members_gate);return;}~open_double_doors_left(500,door_right_closed,loc_param(open_sound));";
pub(super) const MEMBERGATE_RIGHT_OPEN: &str =
    "if(map_members=^false){mes(^mes_members_gate);return;}~open_double_doors_right(500,door_left_closed,loc_param(open_sound));";

/// Closed membergate block as read from `doubledoors.loc`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct MembergateDef {
    left: bool,
    pub(super) op_open: bool,
    pub(super) category_ok: bool,
    pub(super) open: Option<i32>,
}

/// Open-leaf provenance for `loc_1560` / `loc_1561`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct MembergateOpenLeaf {
    left: bool,
    op_close: bool,
    category_ok: bool,
}

/// Dedicated `membergatel`/`membergater` crossings. Not `parse_door_config`,
/// not fence-gate inheritance, not a generic double-door interpreter.
/// Both directions, `members_req`, `open_double_doors_*` morph.
pub(super) fn membergate_edges(
    content_root: &Path,
    ids: &HashMap<String, i32>,
    graph: &mut TransportGraph,
    skipped: &mut HashMap<&'static str, usize>,
    collision: &WorldCollision,
) {
    let Some(&left_id) = ids.get(MEMBERGATE_LEFT) else {
        return;
    };
    let Some(&right_id) = ids.get(MEMBERGATE_RIGHT) else {
        return;
    };
    let handlers = membergate_handlers(content_root);
    let (defs, open_leaves, conflicted) = membergate_loc_defs(content_root, ids);
    let positions = loc_positions(content_root);

    let mut admitted: HashMap<i32, i32> = HashMap::new();
    for (id, left) in [(left_id, true), (right_id, false)] {
        if conflicted.contains(&id) {
            bump(skipped, SKIP_MEMBERGATE_CONFLICT, 1);
            continue;
        }
        match handlers.get(&left) {
            Some(MembergateHandler::Ok) => {}
            Some(MembergateHandler::Conflict) => {
                bump(skipped, SKIP_MEMBERGATE_CONFLICT, 1);
                continue;
            }
            _ => {
                bump(skipped, SKIP_MEMBERGATE_HANDLER, 1);
                continue;
            }
        }
        let Some(def) = defs.get(&id) else {
            bump(skipped, SKIP_MEMBERGATE_SHAPE, 1);
            continue;
        };
        if def.left != left || !def.op_open || !def.category_ok {
            bump(skipped, SKIP_MEMBERGATE_SHAPE, 1);
            continue;
        }
        let Some(open) = def.open else {
            bump(skipped, SKIP_MEMBERGATE_STAGE, 1);
            continue;
        };
        let Some(leaf) = open_leaves.get(&open) else {
            bump(skipped, SKIP_MEMBERGATE_STAGE, 1);
            continue;
        };
        if leaf.left != left || !leaf.op_close || !leaf.category_ok {
            bump(skipped, SKIP_MEMBERGATE_STAGE, 1);
            continue;
        }
        admitted.insert(id, open);
    }
    if admitted.len() != 2 {
        return;
    }

    type Xyz = (i32, i32, i32);
    let mut placed: HashMap<Xyz, Vec<Xyz>> = HashMap::new();
    for &id in admitted.keys() {
        let Some(ps) = positions.get(&id) else {
            continue;
        };
        for p in ps {
            placed
                .entry((p.x, p.z, p.level))
                .or_default()
                .push((id, p.shape, p.angle));
        }
    }

    for (&id, &open) in &admitted {
        let Some(ps) = positions.get(&id) else {
            continue;
        };
        let left = id == left_id;
        for p in ps {
            if p.level != 0 || p.shape != 0 {
                continue;
            }
            let Some(angle_dir) = door_dir(p.angle) else {
                continue;
            };
            let at = WorldTile {
                x: p.x,
                z: p.z,
                level: p.level,
            };
            let pair = membergate_pair_tile(at, angle_dir, !left);
            let paired = placed
                .get(&(pair.x, pair.z, pair.level))
                .is_some_and(|rows| {
                    rows.iter().any(|&(oid, shape, angle)| {
                        oid != id && admitted.contains_key(&oid) && shape == 0 && angle == p.angle
                    })
                });
            if !paired {
                bump(skipped, SKIP_MEMBERGATE_PAIR, 1);
                continue;
            }
            for dir in [angle_dir, opposite(angle_dir)] {
                let Some(to) = door_far_side(at, dir, collision) else {
                    continue;
                };
                graph.edges.push(TransportEdge {
                    kind: TransportKind::Door,
                    at,
                    to,
                    loc_id: id,
                    option: 1,
                    ticks: 1,
                    dir: Some(dir),
                    open_loc_id: Some(open),
                    skill_req: vec![],
                    item_req: vec![],
                    quest_req: vec![],
                    varp_req: vec![],
                    worn_req: vec![],
                    members_req: true,
                    wildy_cap: None,
                });
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MembergateHandler {
    Ok,
    Invalid,
    Conflict,
}

/// Loc-specific `[oploc1,membergatel|membergater]` bodies under `scripts`.
/// Same-line and next-line forms. Duplicate identical canonical bodies are
/// fine; a second different body is a conflict. Extra guards / a different
/// opener fail closed.
pub(super) fn membergate_handlers(content_root: &Path) -> HashMap<bool, MembergateHandler> {
    let mut bodies: HashMap<bool, Vec<String>> = HashMap::new();
    visit_rs2(&content_root.join("scripts"), &mut |text| {
        for (left, body) in membergate_oploc_handlers(text) {
            bodies.entry(left).or_default().push(normalized_body(&body));
        }
    });
    let mut out = HashMap::new();
    for left in [true, false] {
        let expected = if left {
            MEMBERGATE_LEFT_OPEN
        } else {
            MEMBERGATE_RIGHT_OPEN
        };
        let Some(found) = bodies.get(&left) else {
            continue;
        };
        let mut distinct = found.clone();
        distinct.sort();
        distinct.dedup();
        out.insert(
            left,
            if distinct.len() > 1 {
                MembergateHandler::Conflict
            } else if distinct.first().map(String::as_str) == Some(expected) {
                MembergateHandler::Ok
            } else {
                MembergateHandler::Invalid
            },
        );
    }
    out
}

pub(super) fn membergate_oploc_handlers(text: &str) -> Vec<(bool, String)> {
    let mut out = Vec::new();
    let mut cur: Option<(bool, String)> = None;
    for raw in text.lines() {
        let line = match raw.find("//") {
            Some(i) => &raw[..i],
            None => raw,
        };
        let line = line.trim();
        if let Some((header, body)) = line.strip_prefix('[').and_then(|l| l.split_once(']')) {
            if let Some(done) = cur.take() {
                out.push(done);
            }
            let (op, name) = match header.split_once(',') {
                Some(parts) => parts,
                None => continue,
            };
            if op.trim() != "oploc1" {
                continue;
            }
            let left = match name.trim() {
                MEMBERGATE_LEFT => true,
                MEMBERGATE_RIGHT => false,
                _ => continue,
            };
            cur = Some((left, body.to_string()));
        } else if let Some((_, body)) = cur.as_mut() {
            body.push('\n');
            body.push_str(line);
        }
    }
    if let Some(done) = cur {
        out.push(done);
    }
    out
}

pub(super) fn membergate_loc_defs(
    content_root: &Path,
    ids: &HashMap<String, i32>,
) -> (
    HashMap<i32, MembergateDef>,
    HashMap<i32, MembergateOpenLeaf>,
    HashSet<i32>,
) {
    let path = content_root
        .join("scripts")
        .join("doors")
        .join("configs")
        .join("doubledoors.loc");
    let mut defs: HashMap<i32, MembergateDef> = HashMap::new();
    let mut open_leaves: HashMap<i32, MembergateOpenLeaf> = HashMap::new();
    let mut conflicted: HashSet<i32> = HashSet::new();
    let Ok(text) = fs::read_to_string(&path) else {
        return (defs, open_leaves, conflicted);
    };
    let mut cur_name: Option<String> = None;
    let mut op1: Option<String> = None;
    let mut category: Option<String> = None;
    let mut open: Option<Option<i32>> = None;
    let mut fields_conflicted = false;
    let flush = |name: &str,
                 op1: &Option<String>,
                 category: &Option<String>,
                 open: Option<Option<i32>>,
                 fields_conflicted: bool,
                 defs: &mut HashMap<i32, MembergateDef>,
                 open_leaves: &mut HashMap<i32, MembergateOpenLeaf>,
                 conflicted: &mut HashSet<i32>| {
        let Some(id) = loc_pack_id(name, ids) else {
            return;
        };
        let closed_left = ids.get(MEMBERGATE_LEFT).copied() == Some(id);
        let closed_right = ids.get(MEMBERGATE_RIGHT).copied() == Some(id);
        let open_left = loc_pack_id("loc_1560", ids) == Some(id);
        let open_right = loc_pack_id("loc_1561", ids) == Some(id);
        if closed_left || closed_right {
            if closed_left == closed_right {
                conflicted.insert(id);
                defs.remove(&id);
                return;
            }
            let left = closed_left;
            let want = if left {
                MEMBERGATE_LEFT_CLOSED
            } else {
                MEMBERGATE_RIGHT_CLOSED
            };
            let def = MembergateDef {
                left,
                op_open: !fields_conflicted && op1.as_deref() == Some("Open"),
                category_ok: !fields_conflicted && category.as_deref() == Some(want),
                open: (!fields_conflicted).then_some(open).flatten().flatten(),
            };
            match defs.get(&id) {
                Some(prev) if *prev != def => {
                    defs.remove(&id);
                    conflicted.insert(id);
                }
                Some(_) => {}
                None if conflicted.contains(&id) => {}
                None => {
                    defs.insert(id, def);
                }
            }
        } else if open_left || open_right {
            if open_left == open_right {
                conflicted.insert(id);
                open_leaves.remove(&id);
                return;
            }
            let left = open_left;
            let want = if left {
                MEMBERGATE_LEFT_OPENED
            } else {
                MEMBERGATE_RIGHT_OPENED
            };
            let leaf = MembergateOpenLeaf {
                left,
                op_close: !fields_conflicted && op1.as_deref() == Some("Close"),
                category_ok: !fields_conflicted && category.as_deref() == Some(want),
            };
            match open_leaves.get(&id) {
                Some(prev) if *prev != leaf => {
                    open_leaves.remove(&id);
                    conflicted.insert(id);
                }
                Some(_) => {}
                None if conflicted.contains(&id) => {}
                None => {
                    open_leaves.insert(id, leaf);
                }
            }
        }
    };
    for raw in text.lines() {
        let line = raw.trim();
        if let Some(name) = config_header(line) {
            if let Some(prev) = cur_name.take() {
                flush(
                    &prev,
                    &op1,
                    &category,
                    open,
                    fields_conflicted,
                    &mut defs,
                    &mut open_leaves,
                    &mut conflicted,
                );
            }
            cur_name = Some(name.to_string());
            op1 = None;
            category = None;
            open = None;
            fields_conflicted = false;
            continue;
        }
        if cur_name.is_none() {
            continue;
        }
        if let Some(value) = line.strip_prefix("op1=") {
            let value = value.trim().to_string();
            if op1.as_ref().is_some_and(|previous| previous != &value) {
                fields_conflicted = true;
            } else {
                op1 = Some(value);
            }
        } else if let Some(value) = line.strip_prefix("category=") {
            let value = value.trim().to_string();
            if category.as_ref().is_some_and(|previous| previous != &value) {
                fields_conflicted = true;
            } else {
                category = Some(value);
            }
        } else if let Some(rest) = line.strip_prefix("param=") {
            if let Some((key, value)) = rest.split_once(',') {
                if key.trim() == "next_loc_stage" {
                    let value = stage_open_loc_id(value.trim(), ids);
                    if open.is_some_and(|previous| previous != value) {
                        fields_conflicted = true;
                    } else {
                        open = Some(value);
                    }
                }
            }
        }
    }
    if let Some(prev) = cur_name {
        flush(
            &prev,
            &op1,
            &category,
            open,
            fields_conflicted,
            &mut defs,
            &mut open_leaves,
            &mut conflicted,
        );
    }
    (defs, open_leaves, conflicted)
}

/// Paired counterpart tile from `door_close` for `wall_straight`.
/// Left uses `door_close`; right uses the negated offset.
pub(super) fn membergate_pair_tile(at: WorldTile, angle_dir: DoorDir, right: bool) -> WorldTile {
    let (dx, dz) = match angle_dir {
        DoorDir::W => (0, 1),
        DoorDir::N => (1, 0),
        DoorDir::E => (0, -1),
        DoorDir::S => (-1, 0),
    };
    let sign = if right { -1 } else { 1 };
    WorldTile {
        x: at.x + sign * dx,
        z: at.z + sign * dz,
        level: at.level,
    }
}
