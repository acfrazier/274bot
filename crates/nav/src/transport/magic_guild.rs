use super::*;

/// Named Magic Guild doors (`magicguild_door_l` / `_r`). Not inherited
/// closed gates — `[oploc1,magicguild_door_*]` in `magic_guild.rs2` is a
/// loc-specific opener, so [`inherited_closed_gates`] refuses them.
/// `~check_axis_locactive` + `stat(magic) < N` gates only the entering
/// crossing (`door_open` / the loc's facing); the exit arm is ungated.
pub(super) const MAGICGUILD_DOOR_LEFT: &str = "magicguild_door_l";
pub(super) const MAGICGUILD_DOOR_RIGHT: &str = "magicguild_door_r";
pub(super) const MAGICGUILD_OPEN_LABEL: &str = "open_mageguild_door";

pub(super) fn magicguild_door_edges(
    content_root: &Path,
    ids: &HashMap<String, i32>,
    positions: &HashMap<i32, Vec<Placement>>,
    graph: &mut TransportGraph,
    collision: &WorldCollision,
    skipped: &mut HashMap<&'static str, usize>,
) {
    let script_path = content_root
        .join("scripts")
        .join("areas")
        .join("area_yanille")
        .join("scripts")
        .join("magic_guild.rs2");
    let applicable = packed_declared_names(ids, &[MAGICGUILD_DOOR_LEFT, MAGICGUILD_DOOR_RIGHT]);
    if !script_path.is_file() {
        bump(skipped, SKIP_MAGICGUILD_SCRIPT, applicable);
        return;
    }
    let Some(level) = magicguild_entering_magic_level(content_root) else {
        bump(skipped, SKIP_MAGICGUILD_SCRIPT, applicable);
        return;
    };
    let admitted = magicguild_door_open_ids(content_root, ids);
    let admitted_ids: HashSet<i32> = admitted.keys().copied().collect();
    bump_unadmitted_packed_ids(
        skipped,
        SKIP_MAGICGUILD_CONFIG,
        ids,
        &[MAGICGUILD_DOOR_LEFT, MAGICGUILD_DOOR_RIGHT],
        &admitted_ids,
    );
    if admitted.is_empty() {
        return;
    }
    for (&id, &open) in &admitted {
        let edge_start = graph.edges.len();
        let Some(ps) = positions.get(&id) else {
            bump(skipped, SKIP_MAGICGUILD_DOOR, 1);
            continue;
        };
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
                    skill_req: if dir == angle_dir {
                        vec![(SKILL_MAGIC, level)]
                    } else {
                        vec![]
                    },
                    item_req: vec![],
                    quest_req: vec![],
                    varp_req: vec![],
                    worn_req: vec![],
                    members_req: false,
                });
            }
        }
        if graph.edges.len() == edge_start {
            bump(skipped, SKIP_MAGICGUILD_DOOR, 1);
        }
    }
}

/// `stat(magic) < N` on the entering arm of `open_mageguild_door`. Both
/// loc-specific `[oploc1,magicguild_door_*]` handlers must jump there.
pub(super) fn magicguild_entering_magic_level(content_root: &Path) -> Option<i32> {
    let path = content_root
        .join("scripts")
        .join("areas")
        .join("area_yanille")
        .join("scripts")
        .join("magic_guild.rs2");
    let text = fs::read_to_string(path).ok()?;
    if !magicguild_oploc_jumps_to_opener(&text) {
        return None;
    }
    let body = label_body_raw(&text, MAGICGUILD_OPEN_LABEL)?;
    let flat: String = body.chars().filter(|c| !c.is_whitespace()).collect();
    if !flat.contains("~check_axis_locactive(coord)") {
        return None;
    }
    if !flat.contains("~open_and_close_double_door2($entering,") {
        return None;
    }
    let needle = "if($entering=true&stat(magic)<";
    let rest = flat.split_once(needle)?.1;
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    let level: i32 = digits.parse().ok()?;
    (level > 0).then_some(level)
}

pub(super) fn magicguild_oploc_jumps_to_opener(text: &str) -> bool {
    let mut seen_left = false;
    let mut seen_right = false;
    for raw in text.lines() {
        let line = raw.trim();
        let line = match line.find("//") {
            Some(i) => line[..i].trim(),
            None => line,
        };
        if let Some(rest) = line.strip_prefix("[oploc1,magicguild_door_l]") {
            seen_left = magicguild_opener_jump(rest);
        } else if let Some(rest) = line.strip_prefix("[oploc1,magicguild_door_r]") {
            seen_right = magicguild_opener_jump(rest);
        }
    }
    seen_left && seen_right
}

pub(super) fn magicguild_opener_jump(rest: &str) -> bool {
    let flat: String = rest.chars().filter(|c| !c.is_whitespace()).collect();
    flat.starts_with(&format!("@{MAGICGUILD_OPEN_LABEL}(")) && flat.ends_with(");")
}

pub(super) fn magicguild_door_open_ids(content_root: &Path, ids: &HashMap<String, i32>) -> HashMap<i32, i32> {
    let path = content_root
        .join("scripts")
        .join("areas")
        .join("area_yanille")
        .join("configs")
        .join("magic_guild")
        .join("magic_guild.loc");
    let Ok(text) = fs::read_to_string(path) else {
        return HashMap::new();
    };
    let opens = parse_door_open_ids(&text, ids);
    let mut out = HashMap::new();
    for name in [MAGICGUILD_DOOR_LEFT, MAGICGUILD_DOOR_RIGHT] {
        let Some(&id) = ids.get(name) else {
            continue;
        };
        if !named_loc_has_open(&text, name) {
            continue;
        }
        let Some(&open) = opens.get(&id) else {
            continue;
        };
        out.insert(id, open);
    }
    out
}
