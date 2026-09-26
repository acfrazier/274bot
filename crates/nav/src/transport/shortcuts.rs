use super::*;

// ---------------------------------------------------------------------------
// Agility shortcuts (m8aq `resolveShortcutPlacements` port).
// ---------------------------------------------------------------------------

/// Agility shortcut edges for the three locs m8aq models (`fullstyle`,
/// `watchshortcut`, `castlecrumbly`), plus the `stat(agility) < N` level the
/// scripts declare as the skill requirement.
pub(super) fn shortcut_edges(
    content_root: &Path,
    ids: &HashMap<String, i32>,
    positions: &HashMap<i32, Vec<Placement>>,
    loc_defs: &LocDefs,
    graph: &mut TransportGraph,
    skipped: &mut HashMap<&'static str, usize>,
) {
    let reqs = shortcut_agility_reqs(content_root);
    type PlacementMaker = fn(&Placement) -> Vec<WorldTile>;
    let makers: [(&str, PlacementMaker); 4] = [
        ("fullstyle", fullstyle_dests),
        ("watchshortcut", watchshortcut_dests),
        ("castlecrumbly", castlecrumbly_dests),
        ("balancing_ledge3", balancing_ledge3_dests),
    ];
    for (loc_name, dests) in makers {
        let Some(&id) = ids.get(loc_name) else {
            continue;
        };
        let Some(_def) = loc_defs.loc(id) else {
            continue;
        };
        let Some(extra) = extra_ticks(loc_name) else {
            bump(
                skipped,
                SKIP_UNPRICED,
                positions.get(&id).map_or(0, Vec::len),
            );
            continue;
        };
        let ticks = 1 + extra;
        let skill_req = reqs
            .get(loc_name)
            .map(|level| vec![(SKILL_AGILITY, *level)])
            .unwrap_or_default();
        let Some(placements) = positions.get(&id) else {
            continue;
        };
        for loc in placements {
            let at = WorldTile {
                x: loc.x,
                z: loc.z,
                level: loc.level,
            };
            for to in dests(loc) {
                if !in_world_box(&to) {
                    bump(skipped, SKIP_DEST_OUTSIDE, 1);
                    continue;
                }
                graph.edges.push(TransportEdge {
                    kind: TransportKind::AgilityShortcut,
                    at,
                    to,
                    loc_id: id,
                    option: 1,
                    ticks,
                    dir: None,
                    open_loc_id: None,
                    skill_req: skill_req.clone(),
                    item_req: vec![],
                    quest_req: vec![],
                    varp_req: vec![],
                    worn_req: vec![],
                    members_req: false,
                    wildy_cap: None,
                });
            }
        }
    }
}

/// Directed `island_rope_swing` hops from `shortcuts.loc` `start_coord` /
/// `end_coord`. Packed `at` is the standable start, not the loc origin.
/// Absolute params only emit when a placement footprint joins that start
/// — a far mapsquare copy cannot invent a remotely usable edge.
pub(super) fn island_rope_edges(
    content_root: &Path,
    ids: &HashMap<String, i32>,
    positions: &HashMap<i32, Vec<Placement>>,
    loc_defs: &LocDefs,
    graph: &mut TransportGraph,
    skipped: &mut HashMap<&'static str, usize>,
) {
    let path = content_root
        .join("scripts")
        .join("skill_agility")
        .join("configs")
        .join("shortcuts.loc");
    let Ok(text) = fs::read_to_string(&path) else {
        bump(skipped, SKIP_ISLAND_ROPE_CONFIG, 1);
        return;
    };
    for leaf in island_rope_leaves(&text) {
        emit_island_rope_leaf(&leaf, ids, positions, loc_defs, graph, skipped);
    }
}

pub(super) struct IslandRopeLeaf {
    name: String,
    start: Option<WorldTile>,
    end: Option<WorldTile>,
    width: i32,
    length: i32,
}

pub(super) fn island_rope_leaves(text: &str) -> Vec<IslandRopeLeaf> {
    let mut out = Vec::new();
    let mut cur_name: Option<String> = None;
    let mut category: Option<String> = None;
    let mut start = None;
    let mut end = None;
    let mut width = 1;
    let mut length = 1;
    let mut flush = |name: &str,
                     category: &Option<String>,
                     start: Option<WorldTile>,
                     end: Option<WorldTile>,
                     width: i32,
                     length: i32| {
        if category.as_deref() == Some(ISLAND_ROPE_CATEGORY) {
            out.push(IslandRopeLeaf {
                name: name.to_string(),
                start,
                end,
                width,
                length,
            });
        }
    };
    for raw in text.lines() {
        let line = raw.trim();
        if let Some(name) = config_header(line) {
            if let Some(prev) = cur_name.take() {
                flush(&prev, &category, start, end, width, length);
            }
            cur_name = Some(name.to_string());
            category = None;
            start = None;
            end = None;
            width = 1;
            length = 1;
            continue;
        }
        if cur_name.is_none() {
            continue;
        }
        if let Some(value) = line.strip_prefix("category=") {
            category = Some(value.trim().to_string());
        } else if let Some(value) = line.strip_prefix("width=") {
            if let Ok(n) = value.trim().parse::<i32>() {
                width = n;
            }
        } else if let Some(value) = line.strip_prefix("length=") {
            if let Ok(n) = value.trim().parse::<i32>() {
                length = n;
            }
        } else if let Some(rest) = line.strip_prefix("param=") {
            if let Some((key, value)) = rest.split_once(',') {
                let tile =
                    coord_literal(value.trim()).map(|(level, x, z)| WorldTile { level, x, z });
                match key.trim() {
                    "start_coord" => start = tile,
                    "end_coord" => end = tile,
                    _ => {}
                }
            }
        }
    }
    if let Some(prev) = cur_name {
        flush(&prev, &category, start, end, width, length);
    }
    out
}

pub(super) fn placement_joins(loc: &Placement, start: WorldTile, width: i32, length: i32) -> bool {
    if loc.level != start.level {
        return false;
    }
    let w = width.max(1);
    let l = length.max(1);
    let (fw, fl) = if loc.angle == 1 || loc.angle == 3 {
        (l, w)
    } else {
        (w, l)
    };
    let max_x = loc.x + fw - 1;
    let max_z = loc.z + fl - 1;
    start.x >= loc.x && start.x <= max_x && start.z >= loc.z && start.z <= max_z
}

pub(super) fn emit_island_rope_leaf(
    leaf: &IslandRopeLeaf,
    ids: &HashMap<String, i32>,
    positions: &HashMap<i32, Vec<Placement>>,
    loc_defs: &LocDefs,
    graph: &mut TransportGraph,
    skipped: &mut HashMap<&'static str, usize>,
) {
    let Some(&id) = ids.get(&leaf.name) else {
        return;
    };
    let Some(_def) = loc_defs.loc(id) else {
        return;
    };
    let Some(extra) = extra_ticks(&leaf.name) else {
        bump(
            skipped,
            SKIP_UNPRICED,
            positions.get(&id).map_or(1, Vec::len),
        );
        return;
    };
    let (Some(start), Some(end)) = (leaf.start, leaf.end) else {
        bump(skipped, SKIP_ISLAND_ROPE_PARAMS, 1);
        return;
    };
    if !in_world_box(&start) || !in_world_box(&end) {
        bump(skipped, SKIP_DEST_OUTSIDE, 1);
        return;
    }
    let Some(placements) = positions.get(&id) else {
        bump(skipped, SKIP_ISLAND_ROPE_JOIN, 1);
        return;
    };
    let mut joined = false;
    for loc in placements {
        if placement_joins(loc, start, leaf.width, leaf.length) {
            joined = true;
        } else {
            bump(skipped, SKIP_ISLAND_ROPE_JOIN, 1);
        }
    }
    if !joined {
        return;
    }
    let skill_req = if leaf.name == ISLAND_ROPE_RETURN {
        vec![]
    } else {
        vec![(SKILL_AGILITY, 10)]
    };
    graph.edges.push(TransportEdge {
        kind: TransportKind::AgilityShortcut,
        at: start,
        to: end,
        loc_id: id,
        option: 1,
        ticks: 1 + extra,
        dir: None,
        open_loc_id: None,
        skill_req,
        item_req: vec![],
        quest_req: vec![],
        varp_req: vec![],
        worn_req: vec![],
        members_req: false,
        wildy_cap: None,
    });
}

pub(super) fn fullstyle_dests(loc: &Placement) -> Vec<WorldTile> {
    let east_west = loc.angle == 0 || loc.angle == 2;
    let (a, b) = if east_west {
        (
            WorldTile {
                level: loc.level,
                x: loc.x,
                z: loc.z + 1,
            },
            WorldTile {
                level: loc.level,
                x: loc.x,
                z: loc.z - 1,
            },
        )
    } else {
        (
            WorldTile {
                level: loc.level,
                x: loc.x + 1,
                z: loc.z,
            },
            WorldTile {
                level: loc.level,
                x: loc.x - 1,
                z: loc.z,
            },
        )
    };
    vec![a, b]
}

pub(super) fn watchshortcut_dests(loc: &Placement) -> Vec<WorldTile> {
    vec![WorldTile {
        level: loc.level,
        x: loc.x,
        z: loc.z + 3,
    }]
}

pub(super) fn castlecrumbly_dests(loc: &Placement) -> Vec<WorldTile> {
    vec![WorldTile {
        level: loc.level,
        x: loc.x + 1,
        z: loc.z,
    }]
}

/// Yanille dungeon ledge (`agility_dungeon.rs2` `balancing_ledge3`):
/// `coordz(loc_coord) > 9518` walks south to `0_40_148_20_40` (2580,9512);
/// else north to `0_40_148_20_48` (2580,9520). Dual placements, one
/// directed hop each. `at` is the loc tile (mid-ledge, blocked); take-off
/// is the standable start tile one step away (`INTERACT_RADIUS` 1).
pub(super) fn balancing_ledge3_dests(loc: &Placement) -> Vec<WorldTile> {
    let to_z = if loc.z > 9518 { 9512 } else { 9520 };
    vec![WorldTile {
        level: loc.level,
        x: loc.x,
        z: to_z,
    }]
}

/// The `stat(agility) < N` level each `[oploc1,<name>]` block declares.
pub(super) fn shortcut_agility_reqs(content_root: &Path) -> HashMap<String, i32> {
    let mut out = HashMap::new();
    for dir in [
        content_root
            .join("scripts")
            .join("skill_agility")
            .join("scripts"),
        content_root
            .join("scripts")
            .join("areas")
            .join("area_yanille")
            .join("scripts"),
    ] {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for ent in entries.flatten() {
            let path = ent.path();
            if path.extension().and_then(|s| s.to_str()) != Some("rs2") {
                continue;
            }
            let Ok(text) = fs::read_to_string(&path) else {
                continue;
            };
            let mut current: Option<String> = None;
            for raw in text.lines() {
                let line = raw.trim();
                if line.starts_with('[') {
                    current = None;
                    if let Some((a, b)) = script_header(line) {
                        if oploc_option(a).is_some() {
                            current = Some(b.to_string());
                        }
                    }
                    continue;
                }
                let Some(name) = &current else {
                    continue;
                };
                if let Some(level) = agility_level_req(line) {
                    out.entry(name.clone()).or_insert(level);
                }
            }
        }
    }
    out
}

/// `stat(agility) < N` in one line → `N`.
pub(super) fn agility_level_req(line: &str) -> Option<i32> {
    let i = line.find("stat(agility)")?;
    let rest = &line[i + "stat(agility)".len()..];
    let rest = rest.trim_start().strip_prefix('<')?.trim_start();
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    digits.parse().ok()
}
