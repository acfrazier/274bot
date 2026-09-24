use super::*;

/// Named Ranging Guild door (`ranging_guild_door`). Not an inherited
/// closed gate and not a Magic Guild `door_far_side` copy: the 289 opener
/// is a shape-9 diagonal wall that `~forcemove`s to a stand then
/// `p_teleport`s relative to that stand. `at` is the origin stand, `to`
/// the teleport dest; `dir` and `open_loc_id` stay `None` (the 3-tick
/// `loc_1532` add is visual, not a swing leaf).
pub(super) const RANGINGGUILD_DOOR_NAME: &str = "ranging_guild_door";
pub(super) const RANGINGGUILD_DOOR_ID: i32 = 2514;
pub(super) const RANGINGGUILD_DOOR_SHAPE: i32 = 9; // LocShape::WALL_DIAGONAL
pub(super) const RANGINGGUILD_EXIT_HALFPLANE: &str =
    "coordx(coord)>coordx(loc_coord)|coordz(coord)<coordz(loc_coord)";
pub(super) const RANGINGGUILD_EXIT_FORCE: (i32, i32, i32) = (1, 0, -1);
pub(super) const RANGINGGUILD_EXIT_TELE: (i32, i32, i32) = (-2, 0, 2);
pub(super) const RANGINGGUILD_ENTER_FORCE: (i32, i32, i32) = (-1, 0, 1);
pub(super) const RANGINGGUILD_ENTER_TELE: (i32, i32, i32) = (2, 0, -2);

pub(super) enum RangingLocResolve {
    Resolved(i32),
    NotApplicable,
    Skip(&'static str),
}

pub(super) fn rangingguild_door_edges(
    content_root: &Path,
    ids: &HashMap<String, i32>,
    positions: &HashMap<i32, Vec<Placement>>,
    graph: &mut TransportGraph,
    skipped: &mut HashMap<&'static str, usize>,
) {
    let loc_id = match rangingguild_resolve_loc_id(content_root, ids) {
        RangingLocResolve::Resolved(id) => id,
        RangingLocResolve::NotApplicable => return,
        RangingLocResolve::Skip(reason) => {
            bump(skipped, reason, RANGINGGUILD_DECLARED_PAIR);
            return;
        }
    };
    let Some((exit_force, exit_tele, enter_force, enter_tele)) =
        rangingguild_parse_opener(content_root)
    else {
        bump(
            skipped,
            SKIP_RANGINGGUILD_SCRIPT,
            RANGINGGUILD_DECLARED_PAIR,
        );
        return;
    };
    let Some(placement) = rangingguild_unique_placement(positions, loc_id) else {
        bump(
            skipped,
            SKIP_RANGINGGUILD_PLACEMENT,
            RANGINGGUILD_DECLARED_PAIR,
        );
        return;
    };
    let loc = WorldTile {
        x: placement.x,
        z: placement.z,
        level: placement.level,
    };
    let enter_at = rangingguild_apply(loc, enter_force);
    let enter_to = rangingguild_apply(enter_at, enter_tele);
    let exit_at = rangingguild_apply(loc, exit_force);
    let exit_to = rangingguild_apply(exit_at, exit_tele);
    for (at, to, skill_req) in [
        (enter_at, enter_to, vec![(SKILL_RANGED, 40)]),
        (exit_at, exit_to, vec![]),
    ] {
        graph.edges.push(TransportEdge {
            kind: TransportKind::Door,
            at,
            to,
            loc_id,
            option: 1,
            ticks: 1,
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
}

pub(super) fn rangingguild_resolve_loc_id(
    content_root: &Path,
    ids: &HashMap<String, i32>,
) -> RangingLocResolve {
    let Some(&id) = ids.get(RANGINGGUILD_DOOR_NAME) else {
        return RangingLocResolve::NotApplicable;
    };
    if id != RANGINGGUILD_DOOR_ID {
        return RangingLocResolve::Skip(SKIP_RANGINGGUILD_PACK);
    }
    let path = content_root
        .join("scripts")
        .join("minigames")
        .join("game_ranging")
        .join("configs")
        .join("ranging.loc");
    let Ok(text) = fs::read_to_string(&path) else {
        return RangingLocResolve::Skip(SKIP_RANGINGGUILD_CONFIG);
    };
    if named_loc_has_open(&text, RANGINGGUILD_DOOR_NAME) {
        RangingLocResolve::Resolved(id)
    } else {
        RangingLocResolve::Skip(SKIP_RANGINGGUILD_CONFIG)
    }
}

pub(super) fn rangingguild_unique_placement(
    positions: &HashMap<i32, Vec<Placement>>,
    loc_id: i32,
) -> Option<&Placement> {
    let ps = positions.get(&loc_id)?;
    let [placement] = ps.as_slice() else {
        return None;
    };
    (placement.level == 0 && placement.shape == RANGINGGUILD_DOOR_SHAPE && placement.angle == 0)
        .then_some(placement)
}

pub(super) fn rangingguild_apply(tile: WorldTile, (dx, d_level, dz): (i32, i32, i32)) -> WorldTile {
    WorldTile {
        x: tile.x + dx,
        z: tile.z + dz,
        level: tile.level + d_level,
    }
}

pub(super) type RangingGuildOpener = (
    (i32, i32, i32),
    (i32, i32, i32),
    (i32, i32, i32),
    (i32, i32, i32),
);

pub(super) fn rangingguild_parse_opener(content_root: &Path) -> Option<RangingGuildOpener> {
    let script = fs::read_to_string(
        content_root
            .join("scripts")
            .join("minigames")
            .join("game_ranging")
            .join("scripts")
            .join("ranging_guild_door.rs2"),
    )
    .ok()?;
    let mut blocks = script_blocks(&script)
        .into_iter()
        .filter(|(op, name, _)| op == "oploc1" && name == RANGINGGUILD_DOOR_NAME);
    let (_, _, body) = blocks.next()?;
    if blocks.next().is_some() {
        return None;
    }
    let body = rangingguild_strip_line_comments(&body);
    let (cond, exit_arm, enter_arm) = rangingguild_split_leading_if(&body)?;
    if rangingguild_flatten(&cond) != RANGINGGUILD_EXIT_HALFPLANE {
        return None;
    }
    let exit_flat = rangingguild_flatten(&exit_arm);
    let enter_flat = rangingguild_flatten(&enter_arm);
    if exit_flat.contains("stat(ranged)") {
        return None;
    }
    if !rangingguild_forcemove_before_teleport(&exit_flat)
        || !rangingguild_first_return_after_teleport(&exit_flat)
    {
        return None;
    }
    let exit_force = rangingguild_unique_delta(&exit_arm, "forcemove", "loc_coord")?;
    let exit_tele = rangingguild_unique_delta(&exit_arm, "p_teleport", "coord")?;
    if exit_force != RANGINGGUILD_EXIT_FORCE || exit_tele != RANGINGGUILD_EXIT_TELE {
        return None;
    }
    if rangingguild_stat_ranged_lt(&enter_arm) != Some(40)
        || !rangingguild_enter_gate_before_crossing(&enter_flat)
    {
        return None;
    }
    let enter_force = rangingguild_unique_delta(&enter_arm, "forcemove", "loc_coord")?;
    let enter_tele = rangingguild_unique_delta(&enter_arm, "p_teleport", "coord")?;
    if enter_force != RANGINGGUILD_ENTER_FORCE || enter_tele != RANGINGGUILD_ENTER_TELE {
        return None;
    }
    Some((exit_force, exit_tele, enter_force, enter_tele))
}

/// Unique deltas do not encode call order. The 289 crossing is forcemove
/// then teleport; a swapped body is an unsupported form, not a hop.
pub(super) fn rangingguild_forcemove_before_teleport(flat: &str) -> bool {
    let Some(force) = flat.find("forcemove(") else {
        return false;
    };
    let Some(tele) = flat.find("p_teleport(") else {
        return false;
    };
    force < tele
}

/// Exit `return;` is the arm terminator after the teleport, not an
/// arbitrary earlier substring.
pub(super) fn rangingguild_first_return_after_teleport(flat: &str) -> bool {
    let Some(ret) = flat.find("return;") else {
        return false;
    };
    let Some(tele) = flat.find("p_teleport(") else {
        return false;
    };
    ret > tele
}

/// Enter must refuse below 40 before the crossing: `if (stat(ranged) < 40)
/// { … return; }` then forcemove then teleport. A bare `stat(ranged)<40`
/// or a return between the two calls is unsupported.
pub(super) fn rangingguild_enter_gate_before_crossing(flat: &str) -> bool {
    let Some(gate) = flat.find("if(stat(ranged)<40){") else {
        return false;
    };
    let Some(force) = flat.find("forcemove(") else {
        return false;
    };
    let Some(tele) = flat.find("p_teleport(") else {
        return false;
    };
    if gate >= force || force >= tele {
        return false;
    }
    flat[gate..force].contains("return;") && !flat[force..tele].contains("return;")
}

pub(super) fn rangingguild_strip_line_comments(text: &str) -> String {
    let mut out = String::new();
    for raw in text.lines() {
        let line = match raw.find("//") {
            Some(i) => raw[..i].trim(),
            None => raw.trim(),
        };
        if !line.is_empty() {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

pub(super) fn rangingguild_flatten(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

pub(super) fn rangingguild_split_leading_if(text: &str) -> Option<(String, String, String)> {
    let start = text.find(|c: char| !c.is_whitespace())?;
    let rest = &text[start..];
    if !rest.starts_with("if") {
        return None;
    }
    let after_if = start + 2;
    if let Some(c) = text[after_if..].chars().next() {
        if c.is_ascii_alphanumeric() || c == '_' {
            return None;
        }
    }
    let paren = text[after_if..].find('(').map(|i| after_if + i)?;
    if !text[after_if..paren].chars().all(char::is_whitespace) {
        return None;
    }
    let cond_close = rangingguild_match(text, paren, '(', ')')?;
    let after_cond = cond_close + 1;
    let brace = text[after_cond..].find('{').map(|i| after_cond + i)?;
    if !text[after_cond..brace].chars().all(char::is_whitespace) {
        return None;
    }
    let body_close = rangingguild_match(text, brace, '{', '}')?;
    Some((
        text[paren + 1..cond_close].to_string(),
        text[brace + 1..body_close].to_string(),
        text[body_close + 1..].to_string(),
    ))
}

pub(super) fn rangingguild_match(
    text: &str,
    open_idx: usize,
    open: char,
    close: char,
) -> Option<usize> {
    let mut depth = 0i32;
    for (i, ch) in text[open_idx..].char_indices() {
        if ch == open {
            depth += 1;
        } else if ch == close {
            depth -= 1;
            if depth == 0 {
                return Some(open_idx + i);
            }
        }
    }
    None
}

pub(super) fn rangingguild_unique_delta(
    arm: &str,
    name: &str,
    base: &str,
) -> Option<(i32, i32, i32)> {
    let calls = call_args_all(arm, name);
    if calls.len() != 1 {
        return None;
    }
    let args = &calls[0];
    if args.len() != 1 {
        return None;
    }
    rangingguild_movecoord_delta(&args[0], base)
}

pub(super) fn rangingguild_movecoord_delta(expr: &str, base: &str) -> Option<(i32, i32, i32)> {
    let args = call_args(expr, "movecoord")?;
    if args.len() != 4 {
        return None;
    }
    let got = args[0].trim();
    let got = got.strip_suffix("()").unwrap_or(got);
    if got != base {
        return None;
    }
    Some((
        int_or_null(&args[1])?,
        int_or_null(&args[2])?,
        int_or_null(&args[3])?,
    ))
}

pub(super) fn rangingguild_stat_ranged_lt(arm: &str) -> Option<i32> {
    let flat = rangingguild_flatten(arm);
    let mut rest = flat.as_str();
    let mut found = None;
    while let Some((_, after)) = rest.split_once("stat(ranged)<") {
        if after.starts_with('=') {
            return None;
        }
        let digits: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
        let level: i32 = digits.parse().ok()?;
        if found.is_some() {
            return None;
        }
        found = Some(level);
        rest = after;
    }
    if flat.matches("stat(ranged)").count() != 1 {
        return None;
    }
    found
}
