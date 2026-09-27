use super::*;

/// Slashable webs (`bigweb_slashable` / loc 733): two edges per
/// far-side dir — `oplocu` with an unequippable knife (`option` 0,
/// `item_req`), and `oploc1` Slash (`option` 1) when any
/// `slashattack_anim` blade is worn (`worn_req`, any-of). `loc_change`
/// to `bigweb_slashed`. Same crossing shape as a door so the traveller
/// trolls the 50% slash fail. Wilderness placements pack too; [`crate::router::find`]
/// still refuses wildy tiles unless the search opts in.
pub(super) const WEB_TICKS: i32 = 2;
/// `oplocu`: use the first `item_req` obj on the loc (the knife).
pub(super) const WEB_USE_OPTION: i32 = 0;

pub(super) fn web_edges(
    content_root: &Path,
    ids: &HashMap<String, i32>,
    positions: &HashMap<i32, Vec<Placement>>,
    graph: &mut TransportGraph,
    collision: &WorldCollision,
    skipped: &mut HashMap<&'static str, usize>,
) {
    let Some(&closed) = ids.get("bigweb_slashable") else {
        return;
    };
    let open_id = ids.get("bigweb_slashed").copied();
    let objs = obj_ids_by_name(content_root);
    let knife = objs.get("knife").copied().unwrap_or(946);
    let slash_blades: Vec<i32> = slash_weapon_ids(content_root, &objs)
        .into_iter()
        .filter(|&id| id != knife)
        .collect();
    let Some(placements) = positions.get(&closed) else {
        return;
    };
    for p in placements {
        if p.level != 0 {
            continue;
        }
        let Some(dir) = door_dir(p.angle) else {
            continue;
        };
        let at = WorldTile {
            x: p.x,
            z: p.z,
            level: p.level,
        };
        for dir in [dir, opposite(dir)] {
            let Some(to) = web_far_side(at, dir, collision) else {
                bump(skipped, SKIP_WEB_NO_FAR, 1);
                continue;
            };
            graph.edges.push(TransportEdge {
                kind: TransportKind::Door,
                at,
                to,
                loc_id: closed,
                option: WEB_USE_OPTION,
                ticks: WEB_TICKS,
                dir: Some(dir),
                open_loc_id: open_id,
                skill_req: vec![],
                item_req: vec![(knife, 1)],
                quest_req: vec![],
                varp_req: vec![],
                worn_req: vec![],
                members_req: false,
                wildy_cap: None,
            });
            if !slash_blades.is_empty() {
                graph.edges.push(TransportEdge {
                    kind: TransportKind::Door,
                    at,
                    to,
                    loc_id: closed,
                    option: 1,
                    ticks: WEB_TICKS,
                    dir: Some(dir),
                    open_loc_id: open_id,
                    skill_req: vec![],
                    item_req: vec![],
                    quest_req: vec![],
                    varp_req: vec![],
                    worn_req: slash_blades.clone(),
                    members_req: false,
                    wildy_cap: None,
                });
            }
        }
    }
}

/// Obj ids whose `.obj` block sets `slashattack_anim` to something other
/// than `human_unarmedpunch` — the same test `~slash_checker` uses on the
/// worn right hand. Unnamed pack rows are skipped.
pub(super) fn slash_weapon_ids(content_root: &Path, objs: &HashMap<String, i32>) -> Vec<i32> {
    let mut names: HashSet<String> = HashSet::new();
    let mut stack = vec![content_root.join("scripts")];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for ent in entries.flatten() {
            let path = ent.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().and_then(|s| s.to_str()) != Some("obj") {
                continue;
            }
            let Ok(text) = fs::read_to_string(&path) else {
                continue;
            };
            let mut current: Option<String> = None;
            for raw in text.lines() {
                let line = raw.trim();
                if let Some(name) = config_header(line) {
                    current = Some(name.to_string());
                    continue;
                }
                let Some(name) = current.as_deref() else {
                    continue;
                };
                let Some(anim) = line.strip_prefix("param=slashattack_anim,") else {
                    continue;
                };
                if anim.trim() != "human_unarmedpunch" {
                    names.insert(name.to_string());
                }
            }
        }
    }
    let mut ids: Vec<i32> = names.iter().filter_map(|n| objs.get(n).copied()).collect();
    ids.sort_unstable();
    ids.dedup();
    ids
}

// Web footprint traversal retains its existing behavior in this wall-door
// correction; webs are not shape-0 wall doors and need separate validation.
pub(super) fn web_far_side(
    at: WorldTile,
    dir: DoorDir,
    collision: &WorldCollision,
) -> Option<WorldTile> {
    let (dx, dz) = match dir {
        DoorDir::N => (0, 1),
        DoorDir::S => (0, -1),
        DoorDir::E => (1, 0),
        DoorDir::W => (-1, 0),
    };
    let (mut x, mut z) = (at.x + dx, at.z + dz);
    loop {
        let t = WorldTile {
            x,
            z,
            level: at.level,
        };
        if collision.standable(t) {
            return Some(t);
        }
        if x < collision.origin.x
            || z < collision.origin.z
            || (x - collision.origin.x) >= collision.width as i32
            || (z - collision.origin.z) >= collision.height as i32
        {
            return None;
        }
        x += dx;
        z += dz;
    }
}
