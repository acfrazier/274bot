//! Content-derived transport graph: doors, ladders, stairs, agility
//! shortcuts, boats, and magic teleports as directed transport edges built
//! from the Server's own content — `scripts/{doors,ladders+stairs,
//! interface_boat,skill_magic,skill_agility}`, `pack/loc.pack`, and the
//! `maps/*.jm2` loc placements — instead of a hand-authored table.
//!
//! The ladder/stairs parsing is a port of m8aq `apiv2/nav/transports.ts`
//! (`resolvePlacements`: `p_telejump`/`p_teleport`/`~climb_ladder` +
//! `movecoord`/coordinate literals under `switch_coord`/`switch_int` guards);
//! agility shortcuts port `resolveShortcutPlacements`. Doors reuse
//! [`crate::pack`]'s door-config + jm2 `LOC` → `DoorEdge` logic. Boats and
//! teleport spells cannot be represented faithfully as edges here (a boat's
//! origin is the dock NPC's tile, a teleport spell is cast from anywhere, and
//! an edge needs a concrete `from` tile), so they are counted and skipped on
//! stderr, never faked.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use api::obj_names::LocDefs;
use api::snapshot::WorldTile;

use crate::pack::{parse_door_config, parse_mapsquare_jm2};

/// The kinds of transport edge this graph derives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TransportKind {
    /// A wall door: both sides of the loc, traversed with the `Open` op.
    Door,
    /// A ladder placement (climb up/down per the script's op).
    Ladder,
    /// A staircase placement.
    Stairs,
    /// A ship/boat journey (origin is the dock).
    Boat,
    /// A magic teleport spell (destination is the spell's landing).
    Teleport,
    /// An agility shortcut (stile, wall climb, …).
    AgilityShortcut,
}

/// One directed transport hop: stand on `from`, use `option` on the loc
/// `loc_id`, arrive at `to` after `ticks`. Requirement vectors are `(skill
/// id, level)` / `(item id, count)` pairs, spell/quest names, and `(varp,
/// value)` pairs, filled from what the source scripts/defs declare (empty
/// when the source declares nothing).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransportEdge {
    pub kind: TransportKind,
    pub from: WorldTile,
    pub to: WorldTile,
    pub loc_id: i32,
    pub option: i32,
    pub ticks: i32,
    pub skill_req: Vec<(i32, i32)>,
    pub item_req: Vec<(i32, i32)>,
    pub quest_req: Vec<String>,
    pub varp_req: Vec<(i32, i32)>,
}

/// All transport edges, indexed by origin tile (`graph.from[tile]` lists
/// indexes into [`TransportGraph::edges`]).
#[derive(Debug, Default)]
pub struct TransportGraph {
    pub edges: Vec<TransportEdge>,
    pub from: HashMap<WorldTile, Vec<usize>>,
}

/// Derive the transport graph from `content_root` (the Server content tree:
/// `scripts/`, `pack/loc.pack`, `maps/*.jm2`) plus the client loc defs.
///
/// Doors come from `scripts/doors/configs/*.loc` + the jm2 LOC placements;
/// ladders/stairs from `scripts/ladders+stairs/scripts/*.rs2`; agility
/// shortcuts from `scripts/skill_agility/scripts/*.rs2`. Placements and
/// destinations that resolve are emitted as edges from the standing tiles
/// around each loc (no walkability filter — the router applies the
/// collision map). Rows that do not resolve are counted per reason on
/// stderr; boats and teleport spells are skipped with a reason (no
/// content-derivable origin tile), never faked.
pub fn derive_transports(content_root: &Path, loc_defs: &LocDefs) -> TransportGraph {
    let mut graph = TransportGraph::default();
    let mut skipped: HashMap<&'static str, usize> = HashMap::new();

    let ids = loc_ids_by_name(content_root);
    let positions = loc_positions(content_root);

    door_edges(content_root, &mut graph, &mut skipped);
    ladder_stair_edges(content_root, &ids, &positions, loc_defs, &mut graph, &mut skipped);
    shortcut_edges(content_root, &ids, &positions, loc_defs, &mut graph, &mut skipped);
    boat_skip(content_root, &mut skipped);
    teleport_skip(content_root, &mut skipped);

    for (i, e) in graph.edges.iter().enumerate() {
        graph.from.entry(e.from).or_default().push(i);
    }

    report(content_root, &graph, &skipped);
    graph
}

// ---------------------------------------------------------------------------
// Skip reasons (m8aq strings kept verbatim where they exist there).
// ---------------------------------------------------------------------------

const SKIP_NO_RULE: &str = "no rule for this placement (script reports it unhandled)";
const SKIP_PLAYER_RELATIVE: &str = "player-relative destination with a horizontal shift";
const SKIP_DIALOG: &str = "destination is behind a dialog";
const SKIP_HANDOFF: &str = "destination handed to another script";
const SKIP_RANDOM: &str = "destination is randomised";
const SKIP_UNPARSED: &str = "destination expression not understood";
const SKIP_DEST_OUTSIDE: &str = "destination outside the grid box";
const SKIP_UNPRICED: &str = "no measured tick cost for this loc name";
const SKIP_SQUARE: &str = "unparseable mapsquare (no MAP section)";
const SKIP_NO_DOOR_CONFIGS: &str = "no door configs parsed under scripts/doors/configs";
const SKIP_BOAT_NO_ORIGIN: &str = "boat journey has no content-derivable origin tile (dock NPC coords)";
const SKIP_TELEPORT_NO_ORIGIN: &str = "teleport spell has no fixed origin (cast from anywhere)";

/// m8aq `types.ts` world box: every reachable 2004 tile. Destinations
/// outside it are skipped (m8aq's `idxOf` returns -1 there).
const LEVELS: i32 = 4;
const X0: i32 = 1856;
const X1: i32 = 3648;
const Z0: i32 = 1280;
const Z1: i32 = 10368;
/// `ladder_cellar`'s +6400/-6400 z shift (m8aq `CELLAR_SHIFT`).
const CELLAR_SHIFT: i32 = 6400;
/// Standard RS2 skill id (Server `PlayerStat`).
const SKILL_AGILITY: i32 = 16;

fn in_world_box(t: &WorldTile) -> bool {
    (0..LEVELS).contains(&t.level) && (X0..X1).contains(&t.x) && (Z0..Z1).contains(&t.z)
}

/// m8aq `packCoord`.
fn pack_coord(level: i32, x: i32, z: i32) -> i32 {
    ((level & 0x3) << 28) | ((x & 0x3fff) << 14) | (z & 0x3fff)
}

/// m8aq `costs.ts` `BY_NAME` extras (ladders/stairs/shortcuts relevant to the
/// parsed scripts). A loc name absent here is unpriced and skipped, like
/// m8aq's `SKIP_UNPRICED`. Edge ticks = `1` (m8aq `opBase`) + extra.
const EXTRA_TICKS: &[(&str, i32)] = &[
    // ladders.rs2: two ticks for a climb, one for a shipladder / wizard tower.
    ("ship_ladder", 2),
    ("ship_laddertop", 2),
    ("laddertop", 2),
    ("ladder", 2),
    ("laddermiddle", 2),
    ("laddertop_directional", 2),
    ("ladder_directional", 2),
    ("ladder_cellar", 2),
    ("ladder_from_cellar", 2),
    ("ladder_from_cellar_directional", 2),
    ("ladder_cellar_inside_down", 2),
    ("phoenixladder", 2),
    ("grandtree_laddermiddle", 2),
    ("laddertop_norim", 2),
    ("shipladder_angled", 1),
    ("shipladder_top_angled", 1),
    ("wizards_tower_laddertop", 1),
    ("wizards_tower_ladder", 1),
    // stairs.rs2.
    ("stairs", 1),
    ("stairstop", 1),
    ("spookystairs", 1),
    ("spookystairstop", 1),
    ("stairs_cellar", 1),
    ("loc_1734", 1),
    ("loc_1736", 1),
    ("outdoorstairs_wooden_bottom", 1),
    ("cryptstairsdown", 1),
    ("cryptstairsup", 1),
    ("board_game_stairs_top", 1),
    ("board_game_stairs_base", 1),
    ("board_game_stairs_grey_all", 1),
    ("board_game_stairs_grey_top", 1),
    ("board_game_stairs_grey_base", 1),
    ("board_game_stairs_grey_base2", 1),
    ("yanillestairsdown", 0),
    ("yanillestairsup", 0),
    ("spiralstairs", 0),
    ("spiralstairsmiddle", 0),
    ("spiralstairstop", 0),
    ("spiralstairs_wooden", 0),
    ("spiralstairstop_wooden", 0),
    ("balance40up", 0),
    ("woodenstairs", 0),
    ("woodenstairstop", 0),
    // agility shortcuts.
    ("fullstyle", 1),
    ("watchshortcut", 0),
    ("castlecrumbly", 2),
];

fn extra_ticks(name: &str) -> Option<i32> {
    EXTRA_TICKS
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, extra)| *extra)
}

fn bump(skipped: &mut HashMap<&'static str, usize>, reason: &'static str, n: usize) {
    if n > 0 {
        *skipped.entry(reason).or_default() += n;
    }
}

fn report(
    content_root: &Path,
    graph: &TransportGraph,
    skipped: &HashMap<&'static str, usize>,
) {
    let mut by_kind: HashMap<TransportKind, usize> = HashMap::new();
    for e in &graph.edges {
        *by_kind.entry(e.kind).or_default() += 1;
    }
    eprintln!(
        "derive_transports({}): {} edges ({} doors, {} ladders, {} stairs, {} agility shortcuts); {} skipped rows",
        content_root.display(),
        graph.edges.len(),
        by_kind.get(&TransportKind::Door).copied().unwrap_or(0),
        by_kind.get(&TransportKind::Ladder).copied().unwrap_or(0),
        by_kind.get(&TransportKind::Stairs).copied().unwrap_or(0),
        by_kind.get(&TransportKind::AgilityShortcut).copied().unwrap_or(0),
        skipped.values().sum::<usize>(),
    );
    let mut reasons: Vec<_> = skipped.keys().collect();
    reasons.sort();
    for r in reasons {
        eprintln!("derive_transports: skipped {}: {}", skipped[r], r);
    }
}

// ---------------------------------------------------------------------------
// Content reads.
// ---------------------------------------------------------------------------

/// One loc placement read from a jm2 file (all levels).
struct Placement {
    id: i32,
    angle: i32,
    level: i32,
    x: i32,
    z: i32,
}

/// `pack/loc.pack` id→name lines → name → id (m8aq `locIdsByName`).
fn loc_ids_by_name(content_root: &Path) -> HashMap<String, i32> {
    let mut out = HashMap::new();
    let Ok(text) = fs::read_to_string(content_root.join("pack").join("loc.pack")) else {
        return out;
    };
    for line in text.lines() {
        let Some((id, name)) = line.split_once('=') else {
            continue;
        };
        let Ok(id) = id.trim().parse::<i32>() else {
            continue;
        };
        let name = name.trim();
        if id >= 0 && !name.is_empty() {
            out.insert(name.to_string(), id);
        }
    }
    out
}

/// All jm2 loc placements grouped by id (m8aq `locPositions`).
fn loc_positions(content_root: &Path) -> HashMap<i32, Vec<Placement>> {
    let mut out: HashMap<i32, Vec<Placement>> = HashMap::new();
    let Ok(entries) = fs::read_dir(content_root.join("maps")) else {
        return out;
    };
    for ent in entries.flatten() {
        let path = ent.path();
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        let Some((mx, mz)) = mapsquare_coords(name) else {
            continue;
        };
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        for p in parse_jm2_locs(&text, mx, mz) {
            out.entry(p.id).or_default().push(p);
        }
    }
    out
}

/// `m<x>_<z>.jm2` → `(x, z)`.
fn mapsquare_coords(name: &str) -> Option<(i32, i32)> {
    let rest = name.strip_prefix('m')?.strip_suffix(".jm2")?;
    let (x, z) = rest.split_once('_')?;
    Some((x.parse().ok()?, z.parse().ok()?))
}

/// Every `LOC` placement in a jm2 text (all levels), in absolute coords.
fn parse_jm2_locs(text: &str, mx: i32, mz: i32) -> Vec<Placement> {
    let mut out = Vec::new();
    let mut in_loc = false;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(name) = crate::pack::section(line) {
            in_loc = name == "LOC";
            continue;
        }
        if !in_loc {
            continue;
        }
        let Some((coords, data)) = line.split_once(':') else {
            continue;
        };
        let mut c = coords.split_whitespace();
        let (Some(level), Some(x), Some(z)) = (
            c.next().and_then(|t| t.parse::<i32>().ok()),
            c.next().and_then(|t| t.parse::<i32>().ok()),
            c.next().and_then(|t| t.parse::<i32>().ok()),
        ) else {
            continue;
        };
        if c.next().is_some() {
            continue;
        }
        let mut d = data.split_whitespace();
        let Some(id) = d.next().and_then(|t| t.parse::<i32>().ok()) else {
            continue;
        };
        // Same token layout and defaults as m8aq `readMapsquare`: the second
        // token is the shape (unused for edges), the third the angle.
        d.next();
        let angle: i32 = d.next().and_then(|t| t.parse().ok()).unwrap_or(0);
        out.push(Placement {
            id,
            angle,
            level,
            x: mx * 64 + x,
            z: mz * 64 + z,
        });
    }
    out
}

// ---------------------------------------------------------------------------
// Doors.
// ---------------------------------------------------------------------------

/// Door edges from `scripts/doors/configs/*.loc` openable ids + the jm2 LOC
/// placements, reusing [`parse_door_config`] and [`parse_mapsquare_jm2`]
/// (the existing jm2 `LOC` → `DoorEdge` bake). Both directions are emitted
/// (the same `Open` op works either way); `option` 1 is the `Open` op.
fn door_edges(
    content_root: &Path,
    graph: &mut TransportGraph,
    skipped: &mut HashMap<&'static str, usize>,
) {
    let configs = content_root.join("scripts").join("doors").join("configs");
    let mut door_ids = HashSet::new();
    let mut configs_read = 0;
    if let Ok(entries) = fs::read_dir(&configs) {
        for ent in entries.flatten() {
            let path = ent.path();
            if path.extension().and_then(|s| s.to_str()) != Some("loc") {
                continue;
            }
            if let Ok(text) = fs::read_to_string(&path) {
                door_ids.extend(parse_door_config(&text));
                configs_read += 1;
            }
        }
    }
    if configs_read == 0 {
        bump(skipped, SKIP_NO_DOOR_CONFIGS, 1);
        return;
    }
    // Passable locs only affect walk stamping, never the door list.
    let passable = HashSet::new();
    let Ok(entries) = fs::read_dir(content_root.join("maps")) else {
        return;
    };
    for ent in entries.flatten() {
        let path = ent.path();
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        let Some((mx, mz)) = mapsquare_coords(name) else {
            continue;
        };
        match parse_mapsquare_jm2(&path, mx, mz, &door_ids, &passable) {
            Ok(sq) => {
                for d in sq.doors {
                    graph.edges.push(TransportEdge {
                        kind: TransportKind::Door,
                        from: to_world(d.from),
                        to: to_world(d.to),
                        loc_id: d.loc_id,
                        option: 1,
                        ticks: 1,
                        skill_req: vec![],
                        item_req: vec![],
                        quest_req: vec![],
                        varp_req: vec![],
                    });
                }
            }
            Err(_) => bump(skipped, SKIP_SQUARE, 1),
        }
    }
}

fn to_world(t: crate::tile::Tile) -> WorldTile {
    WorldTile {
        x: t.x,
        z: t.z,
        level: t.level,
    }
}

// ---------------------------------------------------------------------------
// Ladders and stairs (m8aq `resolvePlacements` port).
// ---------------------------------------------------------------------------

/// Per-placement outcome resolution, m8aq-style: a landing, or a skip reason.
#[derive(Debug)]
enum Outcome {
    Landing(Landing),
    Skipped(&'static str),
}

/// How the script moves the player (m8aq `Landing`).
#[derive(Debug)]
enum Landing {
    Abs { level: i32, x: i32, z: i32 },
    LocDelta { dx: i32, d_level: i32, dz: i32 },
    FromLevel { d: i32 },
    FromZ { d: i32 },
}

/// A parsed `[oplocN,name]` script block: destinations keyed by the loc's
/// packed coord, the loc's angle, or a fallback (m8aq `ScriptRule`).
#[derive(Debug, Default)]
struct ScriptRule {
    by_loc_coord: HashMap<i32, Outcome>,
    by_angle: HashMap<i32, Outcome>,
    fallback: Option<Outcome>,
}

enum Guard {
    Coord(i32),
    Angle(i32),
    Default,
    Unknown,
}

enum SwitchOn {
    Coord,
    Angle,
    Unknown,
}

enum SwitchKind {
    Coord,
    Int,
}

/// Port of m8aq `parseScript`: walk a `ladders.rs2`/`stairs.rs2` text and
/// fill `out` with one rule per `[oplocN,name]` block, recording landing/
/// skip outcomes under the coord/angle/fallback guard in scope.
fn parse_script(
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
                (SwitchKind::Coord, t) if t == "loc_coord" || aliases.contains(t) => SwitchOn::Coord,
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
fn record(
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
        Some(Guard::Default) => {
            if rule.fallback.is_none() {
                rule.fallback = Some(outcome);
            }
        }
        Some(Guard::Unknown) | None => {}
    }
}

/// A statement line's transport outcome (m8aq `parseStatement`).
fn parse_statement(line: &str) -> Option<Outcome> {
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
fn parse_landing(expr: &str) -> Outcome {
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

/// Ladder/stairs edges (m8aq `resolvePlacements` + the standing-tile fan-out
/// of `buildTransportTable`, minus the collision-grid filters).
fn ladder_stair_edges(
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
        let Some(def) = loc_defs.loc(id) else {
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
        let (width, length) = (def.width.max(1), def.length.max(1));
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
                    for from in standing_tiles(loc, width, length) {
                        let to = landing_tile(landing, loc, &from);
                        if !in_world_box(&to) {
                            bump(skipped, SKIP_DEST_OUTSIDE, 1);
                            continue;
                        }
                        graph.edges.push(TransportEdge {
                            kind: *kind,
                            from,
                            to,
                            loc_id: id,
                            option,
                            ticks,
                            skill_req: vec![],
                            item_req: vec![],
                            quest_req: vec![],
                            varp_req: vec![],
                        });
                    }
                }
            }
        }
    }
}

/// The walkable-looking tiles around a loc footprint (m8aq
/// `standingTiles`); the router filters them against the collision map.
fn standing_tiles(loc: &Placement, width: i32, length: i32) -> Vec<WorldTile> {
    let turned = loc.angle == 1 || loc.angle == 3;
    let w = if turned { length } else { width };
    let l = if turned { width } else { length };
    let mut out = Vec::new();
    for dx in 0..w {
        out.push(WorldTile {
            level: loc.level,
            x: loc.x + dx,
            z: loc.z - 1,
        });
        out.push(WorldTile {
            level: loc.level,
            x: loc.x + dx,
            z: loc.z + l,
        });
    }
    for dz in 0..l {
        out.push(WorldTile {
            level: loc.level,
            x: loc.x - 1,
            z: loc.z + dz,
        });
        out.push(WorldTile {
            level: loc.level,
            x: loc.x + w,
            z: loc.z + dz,
        });
    }
    out
}

/// The `to` tile for a landing, per placement / standing tile (m8aq
/// `resolvePlacements` dest + `landingOf`).
fn landing_tile(landing: &Landing, loc: &Placement, from: &WorldTile) -> WorldTile {
    match *landing {
        Landing::Abs { level, x, z } => WorldTile { level, x, z },
        Landing::LocDelta { dx, d_level, dz } => WorldTile {
            level: loc.level + d_level,
            x: loc.x + dx,
            z: loc.z + dz,
        },
        Landing::FromLevel { d } => WorldTile {
            level: from.level + d,
            x: from.x,
            z: from.z,
        },
        Landing::FromZ { d } => WorldTile {
            level: from.level,
            x: from.x,
            z: from.z + d,
        },
    }
}

// ---------------------------------------------------------------------------
// Agility shortcuts (m8aq `resolveShortcutPlacements` port).
// ---------------------------------------------------------------------------

/// Agility shortcut edges for the three locs m8aq models (`fullstyle`,
/// `watchshortcut`, `castlecrumbly`), plus the `stat(agility) < N` level the
/// scripts declare as the skill requirement.
fn shortcut_edges(
    content_root: &Path,
    ids: &HashMap<String, i32>,
    positions: &HashMap<i32, Vec<Placement>>,
    loc_defs: &LocDefs,
    graph: &mut TransportGraph,
    skipped: &mut HashMap<&'static str, usize>,
) {
    let reqs = shortcut_agility_reqs(content_root);
    let makers: [(&str, fn(&Placement) -> Vec<WorldTile>); 3] = [
        ("fullstyle", fullstyle_dests),
        ("watchshortcut", watchshortcut_dests),
        ("castlecrumbly", castlecrumbly_dests),
    ];
    for (loc_name, dests) in makers {
        let Some(&id) = ids.get(loc_name) else {
            continue;
        };
        let Some(def) = loc_defs.loc(id) else {
            continue;
        };
        let Some(extra) = extra_ticks(loc_name) else {
            bump(skipped, SKIP_UNPRICED, positions.get(&id).map_or(0, Vec::len));
            continue;
        };
        let (width, length) = (def.width.max(1), def.length.max(1));
        let ticks = 1 + extra;
        let skill_req = reqs
            .get(loc_name)
            .map(|level| vec![(SKILL_AGILITY, *level)])
            .unwrap_or_default();
        let Some(placements) = positions.get(&id) else {
            continue;
        };
        for loc in placements {
            for to in dests(loc) {
                if !in_world_box(&to) {
                    bump(skipped, SKIP_DEST_OUTSIDE, 1);
                    continue;
                }
                for from in standing_tiles(loc, width, length) {
                    graph.edges.push(TransportEdge {
                        kind: TransportKind::AgilityShortcut,
                        from,
                        to,
                        loc_id: id,
                        option: 1,
                        ticks,
                        skill_req: skill_req.clone(),
                        item_req: vec![],
                        quest_req: vec![],
                        varp_req: vec![],
                    });
                }
            }
        }
    }
}

fn fullstyle_dests(loc: &Placement) -> Vec<WorldTile> {
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

fn watchshortcut_dests(loc: &Placement) -> Vec<WorldTile> {
    vec![WorldTile {
        level: loc.level,
        x: loc.x,
        z: loc.z + 3,
    }]
}

fn castlecrumbly_dests(loc: &Placement) -> Vec<WorldTile> {
    vec![WorldTile {
        level: loc.level,
        x: loc.x + 1,
        z: loc.z,
    }]
}

/// The `stat(agility) < N` level each `[oploc1,<name>]` block declares.
fn shortcut_agility_reqs(content_root: &Path) -> HashMap<String, i32> {
    let mut out = HashMap::new();
    let scripts = content_root
        .join("scripts")
        .join("skill_agility")
        .join("scripts");
    let Ok(entries) = fs::read_dir(&scripts) else {
        return out;
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
    out
}

/// `stat(agility) < N` in one line → `N`.
fn agility_level_req(line: &str) -> Option<i32> {
    let i = line.find("stat(agility)")?;
    let rest = &line[i + "stat(agility)".len()..];
    let rest = rest.trim_start().strip_prefix('<')?.trim_start();
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    digits.parse().ok()
}

// ---------------------------------------------------------------------------
// Boats and teleports: counted, not faked.
// ---------------------------------------------------------------------------

/// `interface_boat` only defines the `set_sail` proc; the journeys with
/// literal destinations live in `scripts/areas/*` (customs officers,
/// sailors) and key off the dock NPC's tile, so no content-derivable origin.
fn boat_skip(content_root: &Path, skipped: &mut HashMap<&'static str, usize>) {
    let mut n = 0usize;
    visit_rs2(&content_root.join("scripts"), &mut |text| {
        n += text.matches("~set_sail(").count();
        n += text.matches("~set_sail_cairn(").count();
    });
    bump(skipped, SKIP_BOAT_NO_ORIGIN, n);
}

/// Teleport spells declare a landing (`data=tele_coord`) and requirements in
/// `skill_magic/configs/magic_spells.dbrow`, but are cast from anywhere, so
/// they have no single origin tile to key an edge on.
fn teleport_skip(content_root: &Path, skipped: &mut HashMap<&'static str, usize>) {
    let path = content_root
        .join("scripts")
        .join("skill_magic")
        .join("configs")
        .join("magic_spells.dbrow");
    let Ok(text) = fs::read_to_string(&path) else {
        return;
    };
    let n = text
        .lines()
        .filter(|l| l.trim().starts_with("data=tele_coord,"))
        .count();
    bump(skipped, SKIP_TELEPORT_NO_ORIGIN, n);
}

fn visit_rs2(dir: &Path, cb: &mut impl FnMut(&str)) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for ent in entries.flatten() {
        let path = ent.path();
        if path.is_dir() {
            visit_rs2(&path, cb);
        } else if path.extension().and_then(|s| s.to_str()) == Some("rs2") {
            if let Ok(text) = fs::read_to_string(&path) {
                cb(&text);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Script text helpers (m8aq regexes ported without a regex dependency).
// ---------------------------------------------------------------------------

/// `[a,b]` where both parts are identifiers.
fn script_header(line: &str) -> Option<(&str, &str)> {
    let inner = line.strip_prefix('[')?.strip_suffix(']')?;
    let (a, b) = inner.split_once(',')?;
    let word = |s: &str| !s.is_empty() && s.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_');
    if !word(a) || !word(b) {
        return None;
    }
    Some((a, b))
}

/// `oplocN` → `N`.
fn oploc_option(header: &str) -> Option<i32> {
    let rest = header.strip_prefix("oploc")?;
    if rest.is_empty() || !rest.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    rest.parse().ok()
}

/// `def_coord $name = loc_coord[()]` → `$name`.
fn def_coord_alias(line: &str) -> Option<String> {
    let (lhs, rhs) = line.split_once('=')?;
    let lhs = lhs.trim();
    let (kw, name) = lhs.split_once(char::is_whitespace)?;
    if kw != "def_coord" {
        return None;
    }
    let name = name.trim();
    if !name.starts_with('$') || name.len() == 1 {
        return None;
    }
    if !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'$' || b == b'_') {
        return None;
    }
    if !rhs.trim_start().starts_with("loc_coord") {
        return None;
    }
    Some(name.to_string())
}

/// `switch_(coord|int) (target) {` → `(kind, target)`.
fn switch_kind(line: &str) -> Option<(SwitchKind, String)> {
    let rest = line.strip_prefix("switch_")?;
    let (kind, rest) = if let Some(r) = rest.strip_prefix("coord") {
        (SwitchKind::Coord, r)
    } else if let Some(r) = rest.strip_prefix("int") {
        (SwitchKind::Int, r)
    } else {
        return None;
    };
    let after = rest.as_bytes().first();
    if !matches!(after, None | Some(b' ') | Some(b'\t') | Some(b'(')) {
        return None;
    }
    let inner = rest.trim_start().strip_prefix('(')?.split(')').next()?;
    Some((kind, inner.trim().to_string()))
}

/// `case <key> : <body>` with a `default`/coord-literal/int key.
fn case_parts(line: &str) -> Option<(&str, &str)> {
    let rest = line.strip_prefix("case")?;
    let rest = rest.trim_start();
    let (key, body) = rest.split_once(':')?;
    let key = key.trim();
    if !case_key_valid(key) {
        return None;
    }
    Some((key, body.trim()))
}

fn case_key_valid(key: &str) -> bool {
    if key == "default" {
        return true;
    }
    if key.is_empty() {
        return false;
    }
    let parts: Vec<&str> = key.split('_').collect();
    let digit_part = |p: &str| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit());
    if parts.len() == 1 {
        return digit_part(parts[0]);
    }
    parts.len() == 5 && parts.iter().all(|p| digit_part(p))
}

/// `if (target = <5-part coord literal>)` → `(target, literal)`.
fn if_coord_target(line: &str) -> Option<(String, String)> {
    let rest = line.strip_prefix("if")?;
    let rest = rest.trim_start();
    let inner = rest.strip_prefix('(')?.split(')').next()?;
    let (target, value) = inner.split_once('=')?;
    let target = target.trim();
    if target.is_empty()
        || !target
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'$')
    {
        return None;
    }
    let value = value.trim();
    let parts: Vec<&str> = value.split('_').collect();
    if parts.len() != 5
        || !parts
            .iter()
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
    {
        return None;
    }
    Some((target.to_string(), value.to_string()))
}

/// `@name` preceded by start/whitespace/`:` → `name`.
fn label_name(line: &str) -> Option<&str> {
    for (i, ch) in line.char_indices() {
        if ch != '@' {
            continue;
        }
        let prev_ok = i == 0 || matches!(line.as_bytes()[i - 1], b' ' | b'\t' | b':');
        if !prev_ok {
            continue;
        }
        let rest = &line[i + 1..];
        let end = rest
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(rest.len());
        if end > 0 {
            return Some(&rest[..end]);
        }
    }
    None
}

/// Top-level args of `name(...)` in `text`, or None (m8aq `callArgs`).
fn call_args(text: &str, name: &str) -> Option<Vec<String>> {
    let needle = format!("{name}(");
    let at = text.find(&needle)?;
    if at > 0 {
        let prev = text.as_bytes()[at - 1];
        if prev.is_ascii_alphanumeric() || prev == b'_' {
            return None;
        }
    }
    let after = &text[at + needle.len()..];
    let mut args = Vec::new();
    // Depth starts at 1: the call's own `(` was consumed by the needle.
    let mut depth = 1i32;
    let mut start = 0usize;
    for (i, ch) in after.char_indices() {
        match ch {
            '(' => depth += 1,
            ',' if depth == 1 => {
                args.push(after[start..i].trim().to_string());
                start = i + 1;
            }
            ')' => {
                depth -= 1;
                if depth == 0 {
                    args.push(after[start..i].trim().to_string());
                    return Some(args);
                }
            }
            _ => {}
        }
    }
    None
}

/// `L_XHI_ZHI_XLO_ZLO` → `(level, x, z)` with `x = XHI<<6|XLO` (m8aq
/// `parseCoordLiteral`).
fn coord_literal(text: &str) -> Option<(i32, i32, i32)> {
    let t = text.trim();
    let mut parts = t.split('_');
    let level = parts.next()?;
    let x_hi = parts.next()?;
    let z_hi = parts.next()?;
    let x_lo = parts.next()?;
    let z_lo = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if !(digits(level) && digits(x_hi) && digits(z_hi) && digits(x_lo) && digits(z_lo)) {
        return None;
    }
    Some((
        level.parse().ok()?,
        (x_hi.parse::<i32>().ok()? << 6) | x_lo.parse::<i32>().ok()?,
        (z_hi.parse::<i32>().ok()? << 6) | z_lo.parse::<i32>().ok()?,
    ))
}

/// `-?\d+` (m8aq `intOrNull`).
fn int_or_null(text: &str) -> Option<i32> {
    let t = text.trim();
    let digits = t.strip_prefix('-').unwrap_or(t);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    t.parse().ok()
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::*;
    use client::config::LocType;

    /// A throwaway content root written on demand, removed on drop.
    struct Fixture {
        root: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let root = std::env::temp_dir().join(format!(
                "nav-transport-fixture-{}-{n}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).unwrap();
            Fixture { root }
        }

        fn write(&self, rel: &str, text: &str) {
            let path = self.root.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }

        fn path(&self) -> &Path {
            &self.root
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn loc_defs(entries: &[(i32, i32, i32)]) -> LocDefs {
        let locs: Vec<LocType> = entries
            .iter()
            .map(|&(id, width, length)| LocType {
                id,
                width,
                length,
                ..Default::default()
            })
            .collect();
        LocDefs::from_locs(&locs)
    }

    #[test]
    fn derive_transports_pins_catherby_door_and_a_ladder() {
        let fx = Fixture::new();
        fx.write("pack/loc.pack", "1530=loc_1530\n1747=ladder\n");
        fx.write(
            "scripts/doors/configs/doors.loc",
            "[loc_1530]\nname=Door\nop1=Open\ncategory=door_closed\n",
        );
        fx.write(
            "maps/m44_53.jm2",
            "\
==== MAP ====
0 0 45: h1 o6 u50
0 0 46: h1 o6 u50
0 0 47: h1 o6 u50
0 10 10: h1 o6 u50
==== LOC ====
0 0 46: 1530 0 1
0 10 10: 1747 0 0
",
        );
        fx.write(
            "scripts/ladders+stairs/scripts/ladders.rs2",
            "\
[oploc1,ladder]
p_arrivedelay;
switch_coord (loc_coord) {
    case 0_44_53_10_10 : ~climb_ladder(1_44_54_10_12, true);
    case default : ~climb_ladder(movecoord(coord(), 0, 1, 0), true);
}
",
        );
        let defs = loc_defs(&[(1530, 1, 1), (1747, 1, 1)]);
        let graph = derive_transports(fx.path(), &defs);

        // The Catherby door (loc 1530 @ 2816,3438,0, angle 1): one edge per
        // side, both directions, `Open` op 1, one tick.
        let doors: Vec<_> = graph
            .edges
            .iter()
            .filter(|e| e.kind == TransportKind::Door && e.loc_id == 1530)
            .collect();
        assert_eq!(doors.len(), 2);
        for d in &doors {
            assert_eq!(d.option, 1);
            assert_eq!(d.ticks, 1);
        }
        let south = WorldTile {
            x: 2816,
            z: 3437,
            level: 0,
        };
        let north = WorldTile {
            x: 2816,
            z: 3439,
            level: 0,
        };
        let fwd = doors
            .iter()
            .find(|d| d.from == south)
            .expect("Catherby south→north door");
        assert_eq!(fwd.to, north);
        let rev = doors
            .iter()
            .find(|d| d.from == north)
            .expect("Catherby reverse neighbour");
        assert_eq!(rev.to, south);

        // One ladder placement (id 1747 @ 2826,3402,0) climbing to
        // (1,2826,3468): one edge per standing tile.
        let ladders: Vec<_> = graph
            .edges
            .iter()
            .filter(|e| e.kind == TransportKind::Ladder && e.loc_id == 1747)
            .collect();
        assert_eq!(ladders.len(), 4);
        let landing = WorldTile {
            x: 2826,
            z: 3468,
            level: 1,
        };
        let standing = [
            WorldTile {
                x: 2826,
                z: 3401,
                level: 0,
            },
            WorldTile {
                x: 2826,
                z: 3403,
                level: 0,
            },
            WorldTile {
                x: 2825,
                z: 3402,
                level: 0,
            },
            WorldTile {
                x: 2827,
                z: 3402,
                level: 0,
            },
        ];
        for l in &ladders {
            assert_eq!(l.to, landing);
            assert!(standing.contains(&l.from), "unexpected from {:?}", l.from);
            assert_eq!(l.option, 1);
            assert_eq!(l.ticks, 3); // op base 1 + ladder extra 2
            assert!(l.skill_req.is_empty());
        }

        // The from-index keys both edges' origins.
        assert_eq!(graph.from[&south].len(), 1);
        assert_eq!(graph.edges[graph.from[&south][0]].to, north);
        let stand = WorldTile {
            x: 2826,
            z: 3401,
            level: 0,
        };
        assert_eq!(graph.from[&stand].len(), 1);
        assert_eq!(graph.edges[graph.from[&stand][0]].to, landing);
    }

    #[test]
    fn derive_transports_pins_watchshortcut_agility_req() {
        let fx = Fixture::new();
        fx.write("pack/loc.pack", "2298=watchshortcut\n");
        fx.write(
            "maps/m44_53.jm2",
            "\
==== MAP ====
0 5 5: h1 o6 u50
==== LOC ====
0 5 5: 2298 10 0
",
        );
        fx.write(
            "scripts/skill_agility/scripts/shortcuts.rs2",
            "\
[oploc1,watchshortcut]
if(stat(agility) < 5) {
    ~mesbox(\"You need an Agility level of 5 to climb the wall.\");
    return;
}
p_telejump(movecoord(loc_coord, 0, 0, 3));
",
        );
        let defs = loc_defs(&[(2298, 1, 1)]);
        let graph = derive_transports(fx.path(), &defs);

        let edges: Vec<_> = graph
            .edges
            .iter()
            .filter(|e| e.kind == TransportKind::AgilityShortcut && e.loc_id == 2298)
            .collect();
        assert_eq!(edges.len(), 4);
        let landing = WorldTile {
            x: 2821,
            z: 3400,
            level: 0,
        };
        for e in &edges {
            assert_eq!(e.to, landing);
            assert_eq!(e.option, 1);
            assert_eq!(e.ticks, 1); // op base 1 + watchshortcut extra 0
            assert_eq!(e.skill_req, vec![(SKILL_AGILITY, 5)]);
        }
    }

    #[test]
    fn parse_landing_handles_movecoord_forms() {
        assert!(matches!(
            parse_landing("0_48_49_32_26"),
            Outcome::Landing(Landing::Abs {
                level: 0,
                x: 3104,
                z: 3162
            })
        ));
        assert!(matches!(
            parse_landing("movecoord(coord(), 0, 1, 0)"),
            Outcome::Landing(Landing::FromLevel { d: 1 })
        ));
        assert!(matches!(
            parse_landing("movecoord(coord, 0, 0, 6400)"),
            Outcome::Landing(Landing::FromZ { d: 6400 })
        ));
        assert!(matches!(
            parse_landing("movecoord(loc_coord, 2, 1, 0)"),
            Outcome::Landing(Landing::LocDelta {
                dx: 2,
                d_level: 1,
                dz: 0
            })
        ));
        // A horizontal shift relative to the player is skipped, not faked.
        assert!(matches!(
            parse_landing("movecoord(coord, 0, 1, -4)"),
            Outcome::Skipped(SKIP_PLAYER_RELATIVE)
        ));
        assert!(matches!(
            parse_landing("movecoord(1_34_77_30_5, $randomX, 0, $randomZ)"),
            Outcome::Skipped(SKIP_RANDOM)
        ));
        assert!(matches!(
            parse_landing("movecoord(0_45_55_19_44, 0, 1, 0)"),
            Outcome::Landing(Landing::Abs { level: 1, .. })
        ));
    }

    #[test]
    fn parse_statement_classifies_handoffs_and_dialogs() {
        assert!(matches!(
            parse_statement("@ladder_options(movecoord(coord(), 0, 1, 0), movecoord(coord(), 0, -1, 0));"),
            Some(Outcome::Skipped(SKIP_DIALOG))
        ));
        assert!(matches!(
            parse_statement("@stair_options(2_50_50_5_9, 0_50_50_5_9);"),
            Some(Outcome::Skipped(SKIP_DIALOG))
        ));
        assert!(matches!(
            parse_statement("@unhandled_stairs(loc_coord);"),
            None
        ));
        assert!(matches!(
            parse_statement("@ladder_to_dwarf_remains;"),
            Some(Outcome::Skipped(SKIP_HANDOFF))
        ));
        assert!(matches!(
            parse_statement("def_int $option = ~p_choice2_header(\"Climb Up.\", 1, \"Climb Down.\", 2, \"Climb up or down the ladder?\");"),
            Some(Outcome::Skipped(SKIP_DIALOG))
        ));
        assert!(matches!(parse_statement("p_arrivedelay;"), None));
    }

    #[test]
    fn parse_script_picks_out_coord_cases_and_fallbacks() {
        let mut rules = HashMap::new();
        parse_script(
            "\
[oploc1,laddertop]
p_arrivedelay;
switch_coord (loc_coord) {
    case 2_47_54_17_57 : ~climb_ladder(1_47_54_17_58, false); // black knights fortress ladder
    case default : ~climb_ladder(movecoord(coord(), 0, -1, 0), false);
}
",
            TransportKind::Ladder,
            &mut rules,
        );
        let (kind, rule) = rules.get(&("laddertop".to_string(), 1)).unwrap();
        assert_eq!(*kind, TransportKind::Ladder);
        // 2_47_54_17_57 -> level 2, x=47<<6|17=3025, z=54<<6|57=3513.
        let packed = pack_coord(2, 3025, 3513);
        match rule.by_loc_coord.get(&packed) {
            Some(Outcome::Landing(Landing::Abs { level: 1, x, z })) => {
                assert_eq!(*x, 3025);
                assert_eq!(*z, 3514);
            }
            other => panic!("unexpected case outcome: {other:?}"),
        }
        assert!(matches!(
            rule.fallback,
            Some(Outcome::Landing(Landing::FromLevel { d: -1 }))
        ));
    }

    #[test]
    fn derive_transports_skips_script_names_missing_from_pack() {
        let fx = Fixture::new();
        fx.write("pack/loc.pack", "");
        fx.write(
            "scripts/ladders+stairs/scripts/ladders.rs2",
            "\
[oploc1,some_unknown_ladder]
p_arrivedelay;
~climb_ladder(movecoord(coord(), 0, 1, 0), true);
",
        );
        fx.write(
            "maps/m44_53.jm2",
            "\
==== MAP ====
0 0 0: h1 o6 u50
==== LOC ====
0 0 0: 1747 0 0
",
        );
        let defs = loc_defs(&[(1747, 1, 1)]);
        let graph = derive_transports(fx.path(), &defs);
        assert!(graph.edges.is_empty());
    }

    #[test]
    fn derive_transports_without_door_configs_emits_no_door_edges() {
        let fx = Fixture::new();
        fx.write("pack/loc.pack", "1530=loc_1530\n");
        fx.write(
            "maps/m44_53.jm2",
            "\
==== MAP ====
0 0 46: h1 o6 u50
==== LOC ====
0 0 46: 1530 0 1
",
        );
        let defs = loc_defs(&[(1530, 1, 1)]);
        let graph = derive_transports(fx.path(), &defs);
        assert!(graph
            .edges
            .iter()
            .all(|e| e.kind != TransportKind::Door));
    }
}
