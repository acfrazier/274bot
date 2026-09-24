use super::*;

/// Per-placement outcome resolution, m8aq-style: a landing, or a skip reason.
#[derive(Debug)]
pub(super) enum Outcome {
    Landing(Landing),
    Skipped(&'static str),
}

/// How the script moves the player (m8aq `Landing`).
#[derive(Debug)]
pub(super) enum Landing {
    Abs { level: i32, x: i32, z: i32 },
    LocDelta { dx: i32, d_level: i32, dz: i32 },
    FromLevel { d: i32 },
    FromZ { d: i32 },
}

/// A parsed `[oplocN,name]` script block: destinations keyed by the loc's
/// packed coord, the loc's angle, or a fallback (m8aq `ScriptRule`).
#[derive(Debug, Default)]
pub(super) struct ScriptRule {
    pub(super) by_loc_coord: HashMap<i32, Outcome>,
    by_angle: HashMap<i32, Outcome>,
    pub(super) fallback: Option<Outcome>,
}

pub(super) enum Guard {
    Coord(i32),
    Angle(i32),
    Default,
    Unknown,
}

pub(super) enum SwitchOn {
    Coord,
    Angle,
    Unknown,
}

/// Port of m8aq `parseScript`: walk a `ladders.rs2`/`stairs.rs2` text and
/// fill `out` with one rule per `[oplocN,name]` block, recording landing/
/// skip outcomes under the coord/angle/fallback guard in scope.
pub(super) fn parse_script(
    text: &str,
    kind: TransportKind,
    out: &mut HashMap<(String, i32), (TransportKind, ScriptRule)>,
) {
    let mut rule_key: Option<(String, i32)> = None;
    let mut aliases: HashSet<String> = HashSet::new();
    let mut guard: Option<Guard> = None;
    let mut guard_brace: i32 = -1;
    let mut switch_on: Option<SwitchOn> = None;
    let mut switch_brace: i32 = -1;
    let mut depth: i32 = 0;
    let mut last_if_was_coord = false;

    for raw in text.lines() {
        let line = match raw.find("//") {
            Some(i) => raw[..i].trim(),
            None => raw.trim(),
        };
        if line.is_empty() {
            continue;
        }

        if let Some((a, b)) = script_header(line) {
            rule_key = oploc_option(a).map(|option| (b.to_string(), option));
            if let Some(key) = &rule_key {
                out.insert(key.clone(), (kind, ScriptRule::default()));
            }
            aliases.clear();
            guard = None;
            guard_brace = -1;
            switch_on = None;
            switch_brace = -1;
            depth = 0;
            continue;
        }
        let Some(key) = rule_key.as_ref() else {
            continue;
        };

        if let Some(alias) = def_coord_alias(line) {
            aliases.insert(alias);
            continue;
        }

        let before = depth;
        let mut body: Option<&str> = Some(line);

        let sw = switch_kind(line);
        let case = case_parts(line);
        let else_if = line.starts_with("} else if (");
        let else_line = !else_if && line.starts_with("} else {");
        let if_line = if_coord_target(line);

        if let Some((kind, target)) = sw {
            switch_on = Some(match (kind, target.as_str()) {
                (SwitchKind::Int, "loc_angle") => SwitchOn::Angle,
                (SwitchKind::Coord, t) if t == "loc_coord" || aliases.contains(t) => {
                    SwitchOn::Coord
                }
                _ => SwitchOn::Unknown,
            });
            switch_brace = before;
            guard = None;
            guard_brace = -1;
            body = None;
        } else if let Some((key, rest)) = case {
            guard = Some(if key == "default" {
                Guard::Default
            } else {
                match switch_on {
                    Some(SwitchOn::Coord) => coord_literal(key)
                        .map(|(level, x, z)| Guard::Coord(pack_coord(level, x, z)))
                        .unwrap_or(Guard::Unknown),
                    Some(SwitchOn::Angle) => key
                        .parse::<i32>()
                        .map(Guard::Angle)
                        .unwrap_or(Guard::Unknown),
                    _ => Guard::Unknown,
                }
            });
            guard_brace = -1;
            body = Some(rest);
        } else if else_if {
            guard = Some(Guard::Unknown);
            body = None;
        } else if else_line {
            guard = Some(if last_if_was_coord {
                Guard::Default
            } else {
                Guard::Unknown
            });
            body = None;
        } else if line.starts_with("if") && line.contains('(') {
            last_if_was_coord = if_line
                .as_ref()
                .is_some_and(|(t, _)| t == "loc_coord" || aliases.contains(t));
            guard = Some(if last_if_was_coord {
                if_line
                    .as_ref()
                    .and_then(|(_, lit)| coord_literal(lit))
                    .map(|(level, x, z)| Guard::Coord(pack_coord(level, x, z)))
                    .unwrap_or(Guard::Unknown)
            } else {
                Guard::Unknown
            });
            guard_brace = before;
            body = None;
        }

        if let Some(b) = body {
            if !b.is_empty() {
                if let Some(outcome) = parse_statement(b) {
                    record(out, key, &guard, outcome);
                }
            }
        }

        depth += line.matches('{').count() as i32 - line.matches('}').count() as i32;

        if switch_on.is_some() && depth <= switch_brace {
            switch_on = None;
            guard = None;
            guard_brace = -1;
        } else if guard.is_some() && guard_brace >= 0 && depth <= guard_brace {
            guard = None;
            guard_brace = -1;
        }
    }
}

/// First-wins record under the current guard (m8aq `record`).
pub(super) fn record(
    out: &mut HashMap<(String, i32), (TransportKind, ScriptRule)>,
    key: &(String, i32),
    guard: &Option<Guard>,
    outcome: Outcome,
) {
    let Some((_, rule)) = out.get_mut(key) else {
        return;
    };
    match guard {
        Some(Guard::Coord(packed)) => {
            rule.by_loc_coord.entry(*packed).or_insert(outcome);
        }
        Some(Guard::Angle(n)) => {
            rule.by_angle.entry(*n).or_insert(outcome);
        }
        // m8aq `record`: a `default` guard or a null guard (an unguarded
        // statement) both land in the fallback (`guard?.kind !== 'unknown'`);
        // only an `unknown` guard drops the outcome.
        Some(Guard::Default) | None => {
            if rule.fallback.is_none() {
                rule.fallback = Some(outcome);
            }
        }
        Some(Guard::Unknown) => {}
    }
}

/// A statement line's transport outcome (m8aq `parseStatement`).
pub(super) fn parse_statement(line: &str) -> Option<Outcome> {
    for fn_name in ["p_telejump", "p_teleport", "~climb_ladder"] {
        if let Some(args) = call_args(line, fn_name) {
            if !args.is_empty() {
                return Some(parse_landing(&args[0]));
            }
        }
    }
    if line.contains("p_choice2_header") {
        return Some(Outcome::Skipped(SKIP_DIALOG));
    }
    if let Some(name) = label_name(line) {
        return match name {
            "stair_options" | "ladder_options" => Some(Outcome::Skipped(SKIP_DIALOG)),
            "unhandled_stairs" | "unhandled_ladder" => None,
            _ => Some(Outcome::Skipped(SKIP_HANDOFF)),
        };
    }
    None
}

/// A landing expression: a coordinate literal or a `movecoord` call (m8aq
/// `parseLanding`).
pub(super) fn parse_landing(expr: &str) -> Outcome {
    if let Some((level, x, z)) = coord_literal(expr) {
        return Outcome::Landing(Landing::Abs { level, x, z });
    }
    let Some(mv) = call_args(expr, "movecoord") else {
        return Outcome::Skipped(SKIP_UNPARSED);
    };
    if mv.len() != 4 {
        return Outcome::Skipped(SKIP_UNPARSED);
    }
    let (Some(dx), Some(d_level), Some(dz)) = (
        int_or_null(&mv[1]),
        int_or_null(&mv[2]),
        int_or_null(&mv[3]),
    ) else {
        return Outcome::Skipped(SKIP_RANDOM);
    };
    let base = mv[0].trim();
    let base = base.strip_suffix("()").unwrap_or(base);
    if base == "loc_coord" {
        return Outcome::Landing(Landing::LocDelta { dx, d_level, dz });
    }
    if base == "coord" {
        if dx == 0 && dz == 0 {
            return Outcome::Landing(Landing::FromLevel { d: d_level });
        }
        if dx == 0 && d_level == 0 && dz.abs() == CELLAR_SHIFT {
            return Outcome::Landing(Landing::FromZ { d: dz });
        }
        return Outcome::Skipped(SKIP_PLAYER_RELATIVE);
    }
    if let Some((level, x, z)) = coord_literal(base) {
        return Outcome::Landing(Landing::Abs {
            level: level + d_level,
            x: x + dx,
            z: z + dz,
        });
    }
    Outcome::Skipped(SKIP_UNPARSED)
}

/// Ladder/stairs edges (m8aq `resolvePlacements` — one edge per placement,
/// `at` the loc tile, `to` the resolved landing).
pub(super) fn ladder_stair_edges(
    content_root: &Path,
    ids: &HashMap<String, i32>,
    positions: &HashMap<i32, Vec<Placement>>,
    loc_defs: &LocDefs,
    graph: &mut TransportGraph,
    skipped: &mut HashMap<&'static str, usize>,
) {
    let scripts = content_root
        .join("scripts")
        .join("ladders+stairs")
        .join("scripts");
    let mut rules: HashMap<(String, i32), (TransportKind, ScriptRule)> = HashMap::new();
    for (file, kind) in [
        ("ladders.rs2", TransportKind::Ladder),
        ("stairs.rs2", TransportKind::Stairs),
    ] {
        if let Ok(text) = fs::read_to_string(scripts.join(file)) {
            parse_script(&text, kind, &mut rules);
        }
    }

    let mut keys: Vec<_> = rules.keys().cloned().collect();
    keys.sort();
    for (loc_name, option) in keys {
        let Some(&id) = ids.get(&loc_name) else {
            continue;
        };
        let Some(_def) = loc_defs.loc(id) else {
            continue;
        };
        let Some(extra) = extra_ticks(&loc_name) else {
            bump(
                skipped,
                SKIP_UNPRICED,
                positions.get(&id).map_or(0, Vec::len),
            );
            continue;
        };
        let ticks = 1 + extra;
        let (kind, rule) = &rules[&(loc_name, option)];
        let Some(placements) = positions.get(&id) else {
            continue;
        };
        for loc in placements {
            let at = pack_coord(loc.level, loc.x, loc.z);
            let outcome = rule
                .by_loc_coord
                .get(&at)
                .or_else(|| rule.by_angle.get(&loc.angle))
                .or(rule.fallback.as_ref());
            let Some(outcome) = outcome else {
                bump(skipped, SKIP_NO_RULE, 1);
                continue;
            };
            match outcome {
                Outcome::Skipped(reason) => bump(skipped, reason, 1),
                Outcome::Landing(landing) => {
                    let at = WorldTile {
                        x: loc.x,
                        z: loc.z,
                        level: loc.level,
                    };
                    let to = landing_tile(landing, loc, &at);
                    if !in_world_box(&to) {
                        bump(skipped, SKIP_DEST_OUTSIDE, 1);
                        continue;
                    }
                    graph.edges.push(TransportEdge {
                        kind: *kind,
                        at,
                        to,
                        loc_id: id,
                        option,
                        ticks,
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
        }
    }
}

/// Closed trapdoor placements (`trapdoors.rs2`) plus already-open leaves.
/// `oploc1,trapdoor` only `loc_change`s to the open loc; `oploc1,trapdoor_open`
/// `p_telejump`s `coord() ± 6400`. One edge per closed map placement, dest
/// baked from the loc tile (live landing is the player's tile).
pub(super) fn trapdoor_edges(
    content_root: &Path,
    ids: &HashMap<String, i32>,
    positions: &HashMap<i32, Vec<Placement>>,
    graph: &mut TransportGraph,
    skipped: &mut HashMap<&'static str, usize>,
) {
    let path = content_root
        .join("scripts")
        .join("general_use")
        .join("scripts")
        .join("trapdoors.rs2");
    let Ok(text) = fs::read_to_string(&path) else {
        return;
    };
    let mut rules: HashMap<(String, i32), (TransportKind, ScriptRule)> = HashMap::new();
    parse_script(&text, TransportKind::Ladder, &mut rules);
    for (closed_name, open_name) in [
        ("trapdoor", "trapdoor_open"),
        ("trapdoor_level1", "trapdoor_open_level1"),
    ] {
        let Some(&closed_id) = ids.get(closed_name) else {
            continue;
        };
        let open_id = ids.get(open_name).copied();
        let Some((_, open_rule)) = rules.get(&(open_name.to_string(), 1)) else {
            continue;
        };
        let Some(Outcome::Landing(landing)) = open_rule.fallback.as_ref() else {
            continue;
        };
        let Some(extra) = extra_ticks(closed_name).or_else(|| extra_ticks(open_name)) else {
            bump(
                skipped,
                SKIP_UNPRICED,
                positions.get(&closed_id).map_or(0, Vec::len),
            );
            continue;
        };
        let ticks = 1 + extra;
        let mut seen = HashSet::new();
        if let Some(placements) = positions.get(&closed_id) {
            for loc in placements {
                let at = WorldTile {
                    x: loc.x,
                    z: loc.z,
                    level: loc.level,
                };
                let to = landing_tile(landing, loc, &at);
                if !in_world_box(&to) {
                    bump(skipped, SKIP_DEST_OUTSIDE, 1);
                    continue;
                }
                seen.insert(at);
                graph.edges.push(TransportEdge {
                    kind: TransportKind::Ladder,
                    at,
                    to,
                    loc_id: closed_id,
                    option: 1,
                    ticks,
                    dir: None,
                    open_loc_id: open_id,
                    skill_req: vec![],
                    item_req: vec![],
                    quest_req: vec![],
                    varp_req: vec![],
                    worn_req: vec![],
                    members_req: false,
                });
            }
        }
        let Some(open_id) = open_id else {
            continue;
        };
        let Some(placements) = positions.get(&open_id) else {
            continue;
        };
        for loc in placements {
            let at = WorldTile {
                x: loc.x,
                z: loc.z,
                level: loc.level,
            };
            if !seen.insert(at) {
                continue;
            }
            let to = landing_tile(landing, loc, &at);
            if !in_world_box(&to) {
                bump(skipped, SKIP_DEST_OUTSIDE, 1);
                continue;
            }
            graph.edges.push(TransportEdge {
                kind: TransportKind::Ladder,
                at,
                to,
                loc_id: open_id,
                option: 1,
                ticks,
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
}

/// The `to` tile for a landing, per placement (m8aq `resolvePlacements`
/// dest + `landingOf`).
pub(super) fn landing_tile(landing: &Landing, loc: &Placement, at: &WorldTile) -> WorldTile {
    match *landing {
        Landing::Abs { level, x, z } => WorldTile { level, x, z },
        Landing::LocDelta { dx, d_level, dz } => WorldTile {
            level: loc.level + d_level,
            x: loc.x + dx,
            z: loc.z + dz,
        },
        Landing::FromLevel { d } => WorldTile {
            level: at.level + d,
            x: at.x,
            z: at.z,
        },
        Landing::FromZ { d } => WorldTile {
            level: at.level,
            x: at.x,
            z: at.z + d,
        },
    }
}
