//! Content-derived transport graph: doors, ladders, stairs, agility
//! shortcuts, boats, gnome gliders, spirit trees, wilderness levers, the
//! Al Kharid toll / Shantay pass item gates, the Rune Mysteries
//! essence-mine wizard and Elkoy's Tree Gnome Village maze escort NPC
//! hops, and magic teleports as directed transport edges built from the
//! Server's own content — `scripts/{doors, ladders+stairs, interface_boat,
//! skill_magic, skill_agility}` and the Ardougne wilderness_lever pair,
//! `pack/loc.pack`, and the `maps/*.jm2` loc placements — instead of a
//! hand-authored table.
//!
//! The ladder/stairs parsing is a port of m8aq `apiv2/nav/transports.ts`
//! (`resolvePlacements`: `p_telejump`/`p_teleport`/`~climb_ladder` +
//! `movecoord`/coordinate literals under `switch_coord`/`switch_int` guards);
//! agility shortcuts port `resolveShortcutPlacements`. Doors derive two
//! edges per jm2 placement — `dir` and its opposite — from the door
//! configs + the baked collision (`at` = the loc tile, `to` each
//! direction's far-side walk-out, `open_loc_id` from the config's
//! `next_loc_stage`). Boats are
//! an explicit 2004 route table (dock NPC tile → post-gangplank dock tile,
//! mined from the `areas/*` `~set_sail(` call sites and the `==== NPC ====`
//! map placements), and gnome gliders a fixed platform table with their
//! quest gate. Teleports are the one any-tile layer: spell teleports
//! (`skill_magic/configs/magic_spells.dbrow`) and jewellery rubs
//! (`general/scripts/enchanted_jewellry/*.rs2`) have no fixed origin, so
//! they live in [`TransportGraph::teleports`] — usable from any tile, kept
//! out of the `at`-indexed edge set.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use api::obj_names::LocDefs;
use api::snapshot::WorldTile;

use crate::collision::WorldCollision;
use crate::pack::{parse_door_config, parse_door_config_ids, parse_door_open_ids};

/// The kinds of transport edge this graph derives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TransportKind {
    /// A wall door: both sides of the loc, traversed with the `Open` op.
    Door,
    /// A ladder placement (climb up/down per the script's op).
    Ladder,
    /// A staircase placement.
    Stairs,
    /// A ship/boat journey (origin is the dock NPC's tile).
    Boat,
    /// A magic teleport spell (destination is the spell's landing).
    Teleport,
    /// An agility shortcut (stile, wall climb, …).
    AgilityShortcut,
    /// A gnome-glider flight between two fixed platforms.
    Glider,
    /// A spirit-tree journey between a tree loc tile and a sibling tree's
    /// tile (the `^…_tree` destination constant).
    SpiritTree,
    /// An NPC-triggered transport hop (carts, essence-mine wizards,
    /// Elkoy's maze escorts).
    Npc,
}

/// The crossing direction of a door edge (step 2 derives it from the door
/// angle); `None` for every other edge kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DoorDir {
    N,
    E,
    S,
    W,
}

/// One directed transport hop: stand on or near `at`, use `option` on the
/// loc `loc_id`, arrive at `to` after `ticks`. `at` is the interact
/// target — the loc tile (door/ladder/stairs/agility/glider) or the
/// origin-leg NPC tile (boat); `to` is the arrival tile. `dir` is the
/// crossing direction for doors only (`None` for every other kind until
/// steps 3/4 fill them); `open_loc_id` the open leaf id a door config's
/// `next_loc_stage` declares (`None` when the config carries none).
/// Requirement vectors are `(skill id, level)` /
/// `(item id, count)` pairs, spell/quest names, and `(varp, value)` pairs,
/// filled from what the source scripts/defs declare (empty when the source
/// declares nothing). `worn_req` is the list of obj ids that must be
/// equipped (worn) to take the hop, read from the source's own `worn`
/// inventory checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransportEdge {
    pub kind: TransportKind,
    pub at: WorldTile,
    pub to: WorldTile,
    pub loc_id: i32,
    pub option: i32,
    pub ticks: i32,
    pub dir: Option<DoorDir>,
    pub open_loc_id: Option<i32>,
    pub skill_req: Vec<(i32, i32)>,
    pub item_req: Vec<(i32, i32)>,
    pub quest_req: Vec<String>,
    pub varp_req: Vec<(i32, i32)>,
    pub worn_req: Vec<i32>,
}

/// All transport edges, indexed by interact target (`graph.at[tile]` lists
/// indexes into [`TransportGraph::edges`]).
#[derive(Debug, Default)]
pub struct TransportGraph {
    pub edges: Vec<TransportEdge>,
    pub at: HashMap<WorldTile, Vec<usize>>,
    /// Any-tile teleport edges (spells + jewellery rubs), kept out of
    /// `edges`/`at` so the default [`crate::router::find`] never sees
    /// them. [`crate::router::find_allow_teleports`] unions them in from
    /// any node. `at` is a wire-only placeholder, never indexed.
    pub teleports: Vec<TransportEdge>,
}

/// Derive the transport graph from `content_root` (the Server content tree:
/// `scripts/`, `pack/loc.pack`, `maps/*.jm2`) plus the client loc defs,
/// and the baked whole-world [`WorldCollision`] (the door edges walk their
/// `to` far side out to a standable tile on it; door edges carry `dir` and
/// `open_loc_id`, every other kind keeps `dir: None`/`open_loc_id: None`).
///
/// Doors come from `scripts/doors/configs/*.loc` + the jm2 LOC placements;
/// ladders/stairs from `scripts/ladders+stairs/scripts/*.rs2`; agility
/// shortcuts from `scripts/skill_agility/scripts/*.rs2`. Placements and
/// destinations that resolve emit an edge — doors emit two per placement
/// (`dir` and its opposite, each with its own far-side walk-out); `at` the
/// loc tile, `to` the resolved landing (no walkability filter — the router
/// applies the collision map). Boats, gnome gliders, the Rune Mysteries
/// essence-mine wizards and Elkoy's maze escorts are the explicit 2004
/// route/placement tables below, and spirit trees the `area_gnome` network
/// (see `spirit_tree_edges`). Teleports (spells + jewellery rubs) are any-tile
/// edges and land in [`TransportGraph::teleports`], never in the `at`
/// index. Rows that do not resolve are counted per reason on stderr, never
/// faked.
pub fn derive_transports(
    content_root: &Path,
    loc_defs: &LocDefs,
    collision: &WorldCollision,
) -> TransportGraph {
    let mut graph = TransportGraph::default();
    let mut skipped: HashMap<&'static str, usize> = HashMap::new();

    let ids = loc_ids_by_name(content_root);
    let positions = loc_positions(content_root);

    door_edges(content_root, &ids, &mut graph, &mut skipped, collision);
    ladder_stair_edges(
        content_root,
        &ids,
        &positions,
        loc_defs,
        &mut graph,
        &mut skipped,
    );
    shortcut_edges(
        content_root,
        &ids,
        &positions,
        loc_defs,
        &mut graph,
        &mut skipped,
    );
    boat_edges(&mut graph);
    cart_edges(&mut graph);
    essence_mine_edges(&mut graph);
    elkoy_edges(&mut graph);
    glider_edges(&mut graph);
    spirit_tree_edges(content_root, &ids, &positions, &mut graph, &mut skipped);
    lever_edges(content_root, &ids, &positions, &mut graph);
    toll_edges(content_root, &ids, &positions, &mut graph, collision);
    zanaris_door_edges(content_root, &ids, &positions, &mut graph);
    teleport_edges(content_root, &mut graph, &mut skipped);

    for (i, e) in graph.edges.iter().enumerate() {
        graph.at.entry(e.at).or_default().push(i);
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
const SKIP_NO_DOOR_CONFIGS: &str = "no door configs parsed under scripts/doors/configs";
const SKIP_TELEPORT_BAD_DEST: &str = "teleport destination does not parse";
const SKIP_TELEPORT_UNRESOLVED_RUNE: &str = "teleport rune name not in pack/obj.pack";
const SKIP_TELEPORT_UNRESOLVED_ITEM: &str = "jewellery item name not in pack/obj.pack";
const SKIP_SPIRIT_NO_DEST: &str = "spirit tree block lists no resolvable destination";

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
/// Standard RS2 skill id for Magic (Server `PlayerStat`).
const SKILL_MAGIC: i32 = 6;
/// Teleport edges have no origin tile (cast/rubbed from anywhere); `at` is
/// a wire-only placeholder that is never indexed into
/// [`TransportGraph::at`].
const TELEPORT_PLACEHOLDER_AT: WorldTile = WorldTile {
    x: 0,
    z: 0,
    level: 0,
};
/// Spell teleport ticks: OP_BASE 1 + the `player_teleport_normal` cast
/// `p_delay(2)` (the spell's whole channel).
const SPELL_TELEPORT_TICKS: i32 = 3;
/// Jewellery rub teleport ticks: OP_BASE 1 + the rub script's `p_delay(1)`.
const JEWELLERY_TELEPORT_TICKS: i32 = 2;
/// Spirit-tree teleport ticks: OP_BASE 1 + the `spirit_tree_tele` label's
/// `p_delay(0)` (the tree's whole channel).
const SPIRIT_TREE_TICKS: i32 = 1;
/// Lever teleport ticks: OP_BASE 1 + the `p_delay(1)` + `p_delay(0)` in
/// `wilderness_lever.rs2` (the pull channel's whole channel; the once-only
/// warning dialog is execute, not search).
const LEVER_TICKS: i32 = 2;
/// Essence-mine wizard teleport ticks: OP_BASE 1 + the `p_delay(4)` in
/// `teleport_to_essence_mine` (the portal channel's whole channel).
const ESSENCE_MINE_TICKS: i32 = 5;

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

fn report(content_root: &Path, graph: &TransportGraph, skipped: &HashMap<&'static str, usize>) {
    let mut by_kind: HashMap<TransportKind, usize> = HashMap::new();
    for e in &graph.edges {
        *by_kind.entry(e.kind).or_default() += 1;
    }
    // Spells always carry the magic-level skill req; jewellery rubs never do.
    let spell_teles = graph
        .teleports
        .iter()
        .filter(|e| !e.skill_req.is_empty())
        .count();
    let jewel_teles = graph.teleports.len() - spell_teles;
    eprintln!(
        "derive_transports({}): {} edges ({} doors, {} ladders, {} stairs, {} agility shortcuts, {} boats, {} gliders, {} spirit trees, {} npc hops); {} teleports ({} spells, {} jewellery); {} skipped rows",
        content_root.display(),
        graph.edges.len(),
        by_kind.get(&TransportKind::Door).copied().unwrap_or(0),
        by_kind.get(&TransportKind::Ladder).copied().unwrap_or(0),
        by_kind.get(&TransportKind::Stairs).copied().unwrap_or(0),
        by_kind.get(&TransportKind::AgilityShortcut).copied().unwrap_or(0),
        by_kind.get(&TransportKind::Boat).copied().unwrap_or(0),
        by_kind.get(&TransportKind::Glider).copied().unwrap_or(0),
        by_kind.get(&TransportKind::SpiritTree).copied().unwrap_or(0),
        by_kind.get(&TransportKind::Npc).copied().unwrap_or(0),
        graph.teleports.len(),
        spell_teles,
        jewel_teles,
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
    shape: i32,
    angle: i32,
    level: i32,
    x: i32,
    z: i32,
}

/// `pack/loc.pack` id→name lines → name → id (m8aq `locIdsByName`).
fn loc_ids_by_name(content_root: &Path) -> HashMap<String, i32> {
    pack_ids_by_name(content_root, "loc.pack")
}

/// `pack/obj.pack` id→name lines → name → id (the spell-rune and jewellery
/// item id map).
fn obj_ids_by_name(content_root: &Path) -> HashMap<String, i32> {
    pack_ids_by_name(content_root, "obj.pack")
}

/// `pack/<file>` `id=name` lines → name → id.
fn pack_ids_by_name(content_root: &Path, file: &str) -> HashMap<String, i32> {
    let mut out = HashMap::new();
    let Ok(text) = fs::read_to_string(content_root.join("pack").join(file)) else {
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
        // token is the shape, the third the angle.
        let shape: i32 = d.next().and_then(|t| t.parse().ok()).unwrap_or(0);
        let angle: i32 = d.next().and_then(|t| t.parse().ok()).unwrap_or(0);
        out.push(Placement {
            id,
            shape,
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

/// Door edges from `scripts/doors/configs/*.loc` plus
/// `scripts/general_use/configs/gates.loc` (fence gates) openable ids +
/// the jm2 LOC placements, two edges per placement: `at` = the door loc
/// tile, `dir` =
/// the placement angle's wall orientation and its opposite (a door is
/// bidirectional), `to` = each direction's far-side tile (walking outward
/// from `at` in the wall's far direction until
/// [`WorldCollision::standable`] accepts one), `open_loc_id` = the
/// config's `param=next_loc_stage` open leaf. `option` 1 is the `Open` op;
/// each `to` is that crossing's arrival side, never a snap. Quest-gated
/// doors (`scripts/quests/*/configs/*.loc` and
/// `scripts/areas/*/configs/*.loc` named blocks) join the door set when
/// their `[oploc1,<name>]` open script declares a varp gate; the gate is
/// carried on every edge of the door (`varp_req`), never invented.
fn door_edges(
    content_root: &Path,
    ids: &HashMap<String, i32>,
    graph: &mut TransportGraph,
    skipped: &mut HashMap<&'static str, usize>,
    collision: &WorldCollision,
) {
    let configs = content_root.join("scripts").join("doors").join("configs");
    let mut door_ids = HashSet::new();
    let mut open_ids: HashMap<i32, i32> = HashMap::new();
    if let Ok(entries) = fs::read_dir(&configs) {
        for ent in entries.flatten() {
            let path = ent.path();
            if path.extension().and_then(|s| s.to_str()) != Some("loc") {
                continue;
            }
            if let Ok(text) = fs::read_to_string(&path) {
                door_ids.extend(parse_door_config(&text));
                open_ids.extend(parse_door_open_ids(&text, ids));
            }
        }
    }
    // Fence gates live outside the door configs dir: `gates.loc` under
    // `scripts/general_use/configs`. The closed gate categories count as
    // openable like `door_closed`, so the same parse collects them.
    let gates = content_root
        .join("scripts")
        .join("general_use")
        .join("configs")
        .join("gates.loc");
    if let Ok(text) = fs::read_to_string(&gates) {
        door_ids.extend(parse_door_config(&text));
        open_ids.extend(parse_door_open_ids(&text, ids));
    }
    let door_reqs = {
        let constants = script_constants(content_root);
        let varps = varp_ids_by_name(content_root);
        let door_names = door_config_names(content_root, ids);
        quest_door_reqs(content_root, &door_names, ids, &constants, &varps)
    };
    door_ids.extend(door_reqs.keys().copied());

    if door_ids.is_empty() {
        bump(skipped, SKIP_NO_DOOR_CONFIGS, 1);
        return;
    }
    let positions = loc_positions(content_root);
    for id in &door_ids {
        let Some(placements) = positions.get(id) else {
            continue;
        };
        for p in placements {
            // The collision bake (and its `standable`) is level 0 only.
            if p.level != 0 || p.shape != 0 {
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
            // A door is bidirectional: an edge in `dir` and one in its
            // opposite, each with its own far-side walk-out (a direction
            // whose far side never resolves yields no edge).
            for dir in [dir, opposite(dir)] {
                let Some(to) = door_far_side(at, dir, collision) else {
                    continue;
                };
                graph.edges.push(TransportEdge {
                    kind: TransportKind::Door,
                    at,
                    to,
                    loc_id: *id,
                    option: 1,
                    ticks: 1,
                    dir: Some(dir),
                    open_loc_id: open_ids.get(id).copied(),
                    skill_req: vec![],
                    item_req: vec![],
                    quest_req: vec![],
                    varp_req: door_reqs.get(id).cloned().unwrap_or_default(),
                    worn_req: vec![],
                });
            }
        }
    }
}

/// `DoorDir` for a placement angle (0=west, 1=north, 2=east, 3=south —
/// the [`client::dash3d::LocAngle`] order), `None` for any other angle.
fn door_dir(angle: i32) -> Option<DoorDir> {
    match angle {
        0 => Some(DoorDir::W),
        1 => Some(DoorDir::N),
        2 => Some(DoorDir::E),
        3 => Some(DoorDir::S),
        _ => None,
    }
}

/// The opposite crossing direction of a door edge's `dir`.
fn opposite(dir: DoorDir) -> DoorDir {
    match dir {
        DoorDir::N => DoorDir::S,
        DoorDir::E => DoorDir::W,
        DoorDir::S => DoorDir::N,
        DoorDir::W => DoorDir::E,
    }
}

/// The far-side tile of a door at `at`: walk outward in the wall's far
/// direction (`dir`: N→+z, S→-z, E→+x, W→-x) one tile at a time until
/// `collision.standable` accepts one. A door whose far side never becomes
/// standable inside the bake yields no edge.
fn door_far_side(at: WorldTile, dir: DoorDir, collision: &WorldCollision) -> Option<WorldTile> {
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

// ---------------------------------------------------------------------------
// Quest-gated doors (requirements read from the door's open script).
// ---------------------------------------------------------------------------

/// `[<name>]` config block header → the name.
fn config_header(line: &str) -> Option<&str> {
    let name = line.strip_prefix('[')?.strip_suffix(']')?;
    if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
        return None;
    }
    Some(name)
}

/// Every `^<name> = <int>` constant under `content/scripts` (the value map
/// the door scripts' `case ^…`/`if (%… >= ^…)` keys resolve through).
fn script_constants(content_root: &Path) -> HashMap<String, i32> {
    let mut out = HashMap::new();
    let mut pending = vec![content_root.join("scripts")];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for ent in entries.flatten() {
            let path = ent.path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().and_then(|s| s.to_str()) == Some("constant") {
                if let Ok(text) = fs::read_to_string(&path) {
                    for raw in text.lines() {
                        let line = raw.trim();
                        let Some(rest) = line.strip_prefix('^') else {
                            continue;
                        };
                        let Some((name, val)) = rest.split_once('=') else {
                            continue;
                        };
                        let name = name.trim();
                        let Ok(val) = val.trim().parse::<i32>() else {
                            continue;
                        };
                        if !name.is_empty() {
                            out.entry(name.to_string()).or_insert(val);
                        }
                    }
                }
            }
        }
    }
    out
}

/// `pack/varp.pack` `id=name` → name → id (like [`loc_ids_by_name`]).
fn varp_ids_by_name(content_root: &Path) -> HashMap<String, i32> {
    pack_ids_by_name(content_root, "varp.pack")
}

/// Every `[<name>]` block in a door config (`scripts/doors/configs/`,
/// `scripts/quests/`, `scripts/areas/`, `scripts/general_use/configs/`)
/// that can open (`op1=Open` or `category=door_closed`, the
/// [`parse_door_config`] rule) and resolves to a numeric loc id.
fn door_config_names(content_root: &Path, ids: &HashMap<String, i32>) -> HashSet<String> {
    let mut out = HashSet::new();
    let scripts = content_root.join("scripts");
    let mut pending = vec![
        scripts.join("doors").join("configs"),
        scripts.join("quests"),
        scripts.join("areas"),
        scripts.join("general_use").join("configs"),
    ];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for ent in entries.flatten() {
            let path = ent.path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().and_then(|s| s.to_str()) == Some("loc") {
                if let Ok(text) = fs::read_to_string(&path) {
                    let mut cur: Option<&str> = None;
                    let mut openable = false;
                    for raw in text.lines() {
                        let line = raw.trim();
                        if let Some(name) = config_header(line) {
                            if let Some(prev) = cur {
                                if openable && ids.contains_key(prev) {
                                    out.insert(prev.to_string());
                                }
                            }
                            cur = Some(name);
                            openable = false;
                        } else if cur.is_some()
                            && (line == "op1=Open" || line == "category=door_closed")
                        {
                            openable = true;
                        }
                    }
                    if let Some(name) = cur {
                        if openable && ids.contains_key(name) {
                            out.insert(name.to_string());
                        }
                    }
                }
            }
        }
    }
    out
}

/// Door loc id → its `(varp id, min value)` gate, read from the door's own
/// `[oploc1,<name>]` open script: a `switch_int(%<varp>)` whose opening
/// cases carry the open call, or an `if (%<varp> >= ^<const> [| …])` whose
/// arm opens. The generic `_door_closed` script opens freely and carries
/// nothing.
fn quest_door_reqs(
    content_root: &Path,
    door_names: &HashSet<String>,
    ids: &HashMap<String, i32>,
    constants: &HashMap<String, i32>,
    varps: &HashMap<String, i32>,
) -> HashMap<i32, Vec<(i32, i32)>> {
    let mut out: HashMap<i32, Vec<(i32, i32)>> = HashMap::new();
    visit_rs2(&content_root.join("scripts"), &mut |text| {
        for (op, name, block) in script_blocks(text) {
            if op != "oploc1" || !door_names.contains(&name) {
                continue;
            }
            let Some(&id) = ids.get(&name) else {
                continue;
            };
            let Some(gates) = script_varp_gate(&block, text, constants) else {
                continue;
            };
            let reqs = out.entry(id).or_default();
            for (varp, min_value) in gates {
                if let Some(&varp_id) = varps.get(&varp) {
                    reqs.push((varp_id, min_value));
                }
            }
        }
    });
    out
}

/// `[<a>,<b>]` blocks in a script text → `(a, b, body)`; the body is the
/// raw text until the next header. Headers may carry a trailing `//`
/// comment (the existing parsers' convention).
fn script_blocks(text: &str) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    let mut cur: Option<(String, String)> = None;
    let mut body = String::new();
    for raw in text.lines() {
        let line = raw.trim();
        let header_line = match line.find("//") {
            Some(i) => line[..i].trim(),
            None => line,
        };
        if let Some((a, b)) = script_header(header_line) {
            if let Some(prev) = cur.take() {
                out.push((prev.0, prev.1, std::mem::take(&mut body)));
            }
            cur = Some((a.to_string(), b.to_string()));
        } else if cur.is_some() {
            body.push_str(line);
            body.push('\n');
        }
    }
    if let Some((a, b)) = cur.take() {
        out.push((a, b, body));
    }
    out
}

/// The `(varp name, min value)` gates a door's open script declares: a
/// `switch_int(%<varp>)` whose opening cases carry the open call, or an
/// `if (%<varp> (>=|=) ^<const> & …)` whose arm opens (every ANDed varp
/// condition is carried).
fn script_varp_gate(
    block: &str,
    script_text: &str,
    constants: &HashMap<String, i32>,
) -> Option<Vec<(String, i32)>> {
    if let Some((varp, cases)) = switch_varp_cases(block) {
        let mut opens = Vec::new();
        for (keys, body) in cases {
            if !body_opens(&body, script_text) {
                continue;
            }
            for key in keys {
                if let Some(v) = case_value(&key, constants) {
                    opens.push(v);
                }
            }
        }
        if let Some(min) = opens.into_iter().min() {
            return Some(vec![(varp, min)]);
        }
    }
    if let Some((gates, arm)) = if_varp_gate(block, constants) {
        if body_opens(&arm, script_text) {
            return Some(gates);
        }
    }
    None
}

/// `^<const>` (resolved through the constants map) or a bare integer.
fn case_value(key: &str, constants: &HashMap<String, i32>) -> Option<i32> {
    let key = key.trim();
    if let Some(name) = key.strip_prefix('^') {
        constants.get(name).copied()
    } else {
        key.parse().ok()
    }
}

/// `switch_int (%<varp>) { case … }` from a block: the varp name and every
/// case's `(keys, body)`. `default` is kept as a key so a gate that only
/// opens on `default` can never match (it has no numeric keys).
fn switch_varp_cases(block: &str) -> Option<(String, Vec<(Vec<String>, String)>)> {
    let lines: Vec<&str> = block.lines().collect();
    let mut si = None;
    for (idx, raw) in lines.iter().enumerate() {
        let line = raw.trim();
        let Some(rest) = line.strip_prefix("switch_int") else {
            continue;
        };
        let rest = rest.trim_start().strip_prefix('(')?;
        let name = rest.split(')').next()?.trim();
        if let Some(name) = name.strip_prefix('%') {
            si = Some((idx, name.to_string()));
            break;
        }
    }
    let (start, varp) = si?;
    let mut cases: Vec<(Vec<String>, String)> = Vec::new();
    let mut depth = 0i32;
    let mut cur: Option<Vec<String>> = None;
    let mut body = String::new();
    for raw in lines.iter().skip(start + 1) {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        // A `case` at the switch's own brace depth starts a new case; the
        // switch's closing `}` at that depth ends the parse.
        if depth == 0 {
            if let Some((keys, rest)) = switch_case(line) {
                if let Some(prev) = cur.take() {
                    cases.push((prev, std::mem::take(&mut body)));
                }
                cur = Some(keys);
                if let Some(rest) = rest {
                    body.push_str(rest);
                    body.push('\n');
                }
                continue;
            }
            if line.starts_with('}') {
                break;
            }
        }
        if cur.is_some() {
            body.push_str(line);
            body.push('\n');
        }
        depth += line.matches('{').count() as i32 - line.matches('}').count() as i32;
    }
    if let Some(keys) = cur.take() {
        cases.push((keys, body));
    }
    Some((varp, cases))
}

/// `case <key, key, …> : <body>` → (keys, body). `default` is a key.
fn switch_case(line: &str) -> Option<(Vec<String>, Option<&str>)> {
    let rest = line.strip_prefix("case")?;
    let rest = rest.trim_start();
    let (keys, body) = rest.split_once(':')?;
    let keys: Vec<String> = keys
        .split(',')
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty())
        .collect();
    if keys.is_empty() {
        return None;
    }
    let body = body.trim();
    Some((keys, if body.is_empty() { None } else { Some(body) }))
}

/// `if (%<varp> (>=|=) ^<const> [& …]) { <arm> }` in a block: every
/// `(varp, const value)` condition and the arm body. The first matching
/// guard is read.
fn if_varp_gate(
    block: &str,
    constants: &HashMap<String, i32>,
) -> Option<(Vec<(String, i32)>, String)> {
    let bytes = block.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i..].starts_with(b"if") {
            let tail = &block[i + 2..];
            let rest = tail.trim_start();
            let ws = tail.len() - rest.len();
            if let Some(inner) = rest.strip_prefix('(') {
                if let Some(close) = inner.find(')') {
                    let head = inner[..close].trim();
                    let conds = varp_gate_consts(head);
                    if !conds.is_empty() {
                        let mut gates = Vec::new();
                        for (varp, cname) in conds {
                            if let Some(&value) = constants.get(&cname) {
                                gates.push((varp.to_string(), value));
                            }
                        }
                        if !gates.is_empty() {
                            let arm_from = i + 2 + ws + 1 + close + 1;
                            if let Some(arm) = balanced_arm(block, arm_from) {
                                return Some((gates, arm));
                            }
                        }
                    }
                }
            }
        }
        i += 1;
    }
    None
}

/// `%<varp> (>=|=) ^<const>` conditions in an if-head (ANDed together),
/// whitespace tolerant; clauses that are not varp-gated (e.g. a `|`
/// leaving-side term) are skipped.
fn varp_gate_consts(head: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for clause in head.split('&') {
        let clause = clause.trim();
        let (varp, cmp) = if let Some((a, b)) = clause.split_once(">=") {
            (a, b)
        } else if let Some((a, b)) = clause.split_once('=') {
            (a, b)
        } else {
            continue;
        };
        let varp = varp.trim();
        let Some(varp) = varp.strip_prefix('%') else {
            continue;
        };
        let cmp = cmp.trim();
        let Some(cmp) = cmp.strip_prefix('^') else {
            continue;
        };
        let end = cmp
            .find(|c: char| c.is_whitespace() || c == '|')
            .unwrap_or(cmp.len());
        let cname = &cmp[..end];
        if !cname.is_empty() {
            out.push((varp.to_string(), cname.to_string()));
        }
    }
    out
}

/// The balanced `{ … }` starting at or after `from`.
fn balanced_arm(block: &str, from: usize) -> Option<String> {
    let rest = &block[from.min(block.len())..];
    let start = rest.find('{')?;
    let mut depth = 1i32;
    for (off, ch) in rest[start + 1..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(rest[start..=start + 1 + off].to_string());
                }
            }
            _ => {}
        }
    }
    None
}

/// True when a case/if arm opens the door: it calls an `~open*` proc
/// directly, or calls a `@label` whose own body does.
fn body_opens(body: &str, script_text: &str) -> bool {
    if body.contains("open_and_close") || body.contains("~open_") {
        return true;
    }
    for label in body_labels(body) {
        if let Some(lb) = label_block(script_text, &label) {
            if body_opens(&lb, script_text) {
                return true;
            }
        }
    }
    false
}

/// `@name` tokens in a body.
fn body_labels(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    for part in body.split('@').skip(1) {
        let end = part
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(part.len());
        if end > 0 {
            out.push(part[..end].to_string());
        }
    }
    out
}

/// The body of `[label,<name>]` in a script text.
fn label_block(script_text: &str, name: &str) -> Option<String> {
    for (op, n, body) in script_blocks(script_text) {
        if op == "label" && n == name {
            return Some(body);
        }
    }
    None
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

/// Ladder/stairs edges (m8aq `resolvePlacements` — one edge per placement,
/// `at` the loc tile, `to` the resolved landing).
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
                    });
                }
            }
        }
    }
}

/// The `to` tile for a landing, per placement (m8aq `resolvePlacements`
/// dest + `landingOf`).
fn landing_tile(landing: &Landing, loc: &Placement, at: &WorldTile) -> WorldTile {
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
                });
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
// Boats: the 2004 dock-NPC journeys (explicit route table). Teleports are
// the any-tile layer (see `teleport_edges` below).
// ---------------------------------------------------------------------------

/// One 2004 boat journey: talk to the dock NPC at `at`, sail to the
/// destination dock, and walk off the destination gangplank. `at` is the
/// NPC's spawn tile (jm2 `==== NPC ====` section, id resolved through
/// `pack/npc.pack`), never the origin gangplank; `to` is the dock tile past
/// the destination gangplank, never a boat-interior/water tile — the
/// gangplank crossing is folded into `ticks` (= the
/// `~set_sail(`/`~set_sail_cairn(` `p_delay` + 2 crossing ticks).
#[derive(Debug, Clone, Copy)]
struct BoatRoute {
    /// npc.pack id of the dock NPC who starts the journey.
    npc: i32,
    at: WorldTile,
    to: WorldTile,
    /// `set_sail` delay + gangplank crossing; no crossing when the landing
    /// is a direct dock tile (`set_sail_cairn`).
    ticks: i32,
    /// `(obj id, count)` fare the journey charges, if any.
    fare: Option<(i32, i32)>,
    /// `(varp id, min value)` quest gate, if any.
    varp_req: Option<(i32, i32)>,
}

/// The 2004 boat journeys: destinations and delays from the `~set_sail(`/
/// `~set_sail_cairn(` call sites in `content/scripts/areas/*` (the customs
/// officers, sailors, monks of Entrana, and Captain Shanks), origin tiles
/// from the `==== NPC ====` placements in `content/maps/*.jm2`, destination
/// dock tiles from the `_gangplank_disembark` locs of
/// `content/scripts/general_use/configs/gangplank.loc` (crossed after the
/// journey lands on the boat), and ids from `pack/npc.pack`/`pack/obj.pack`/
/// `pack/varp.pack`.
const BOAT_ROUTES: &[BoatRoute] = &[
    // Seaman Thresnor (npc 378) on the Port Sarim pier (m47_50): lands on
    // the Karamja ship (2956,3143,1), then off `sarimshipplank_off`
    // (loc 2082, north-facing at 2956,3144,1) to the Karamja dock
    // (2956,3146,0). Delay 7 + 2.
    BoatRoute {
        npc: 378,
        at: WorldTile {
            x: 3026,
            z: 3217,
            level: 0,
        },
        to: WorldTile {
            x: 2956,
            z: 3146,
            level: 0,
        },
        ticks: 9,
        fare: Some((995, 30)),
        varp_req: None,
    },
    // Customs officer (npc 380) at Musa Point (m46_49): lands on the Port
    // Sarim ship (3032,3217,1), then off `karamjashipplank_off` (loc 2084,
    // west-facing at 3031,3217,1) to the Port Sarim dock (3029,3217,0).
    // Delay 7 + 2.
    BoatRoute {
        npc: 380,
        at: WorldTile {
            x: 2955,
            z: 3146,
            level: 0,
        },
        to: WorldTile {
            x: 3029,
            z: 3217,
            level: 0,
        },
        ticks: 9,
        fare: Some((995, 30)),
        varp_req: None,
    },
    // Customs officer (npc 380) at the Brimhaven dock (nearest spawn to the
    // rs2b0t-observed stand, 2772,3234): lands on the Ardougne ship
    // (2683,3268,1), then off `brimhavenshipplank_off` (loc 2086) to the
    // Ardougne dock (2683,3271,0). Delay 7 + 2.
    BoatRoute {
        npc: 380,
        at: WorldTile {
            x: 2772,
            z: 3231,
            level: 0,
        },
        to: WorldTile {
            x: 2683,
            z: 3271,
            level: 0,
        },
        ticks: 9,
        fare: Some((995, 30)),
        varp_req: None,
    },
    // Captain Barnaby (npc 381) at the Ardougne dock (m41_51): lands on the
    // Brimhaven ship (2775,3234,1), then off `ardougneshipplank_off`
    // (loc 2088) to the Brimhaven dock (2772,3234,0). Delay 7 + 2.
    BoatRoute {
        npc: 381,
        at: WorldTile {
            x: 2679,
            z: 3275,
            level: 0,
        },
        to: WorldTile {
            x: 2772,
            z: 3234,
            level: 0,
        },
        ticks: 9,
        fare: Some((995, 30)),
        varp_req: None,
    },
    // Monk of Entrana (shipmonk, npc 657) on the Port Sarim dock (nearest
    // spawn to the rs2b0t-observed stand, 3048,3236): lands on the Entrana
    // ship (2834,3331,1), then off `ship_from_entrana_off` (loc 2415) to the
    // Entrana dock (2834,3335,0). Delay 13 + 2.
    BoatRoute {
        npc: 657,
        at: WorldTile {
            x: 3049,
            z: 3235,
            level: 0,
        },
        to: WorldTile {
            x: 2834,
            z: 3335,
            level: 0,
        },
        ticks: 15,
        fare: None,
        varp_req: None,
    },
    // Monk of Entrana (shipmonk2, npc 658) on the Entrana dock (nearest
    // spawn to the rs2b0t-observed stand, 2834,3335): lands on the Port
    // Sarim ship (3048,3231,1), then off `ship_to_entrana_off` (loc 2413) to
    // the Port Sarim dock (3048,3234,0). Delay 14 + 2.
    BoatRoute {
        npc: 658,
        at: WorldTile {
            x: 2835,
            z: 3336,
            level: 0,
        },
        to: WorldTile {
            x: 3048,
            z: 3234,
            level: 0,
        },
        ticks: 16,
        fare: None,
        varp_req: None,
    },
    // Captain Shanks (npc 518) on the deck of the Lady of the Waves (m43_46):
    // `set_sail_cairn` lands directly on the Khazard dock
    // (`0_41_49_56_14`), no destination gangplank. Delay 9. Gated on Shilo
    // Village complete (`%zombiequeen >= ^zombiequeen_complete`).
    BoatRoute {
        npc: 518,
        at: WorldTile {
            x: 2763,
            z: 2961,
            level: 1,
        },
        to: WorldTile {
            x: 2680,
            z: 3150,
            level: 0,
        },
        ticks: 9,
        fare: None,
        varp_req: Some((116, 15)),
    },
    // Captain Shanks (npc 518) → Port Sarim (`0_47_50_39_35`). Delay 15.
    BoatRoute {
        npc: 518,
        at: WorldTile {
            x: 2763,
            z: 2961,
            level: 1,
        },
        to: WorldTile {
            x: 3047,
            z: 3235,
            level: 0,
        },
        ticks: 15,
        fare: None,
        varp_req: Some((116, 15)),
    },
];

/// Boat edges from the explicit 2004 route table: one `Talk-to` edge per
/// journey, keyed from the dock NPC's tile.
fn boat_edges(graph: &mut TransportGraph) {
    for r in BOAT_ROUTES {
        graph.edges.push(TransportEdge {
            kind: TransportKind::Boat,
            at: r.at,
            to: r.to,
            loc_id: r.npc,
            option: 1,
            ticks: r.ticks,
            dir: None,
            open_loc_id: None,
            skill_req: vec![],
            item_req: r.fare.map(|(id, n)| vec![(id, n)]).unwrap_or_default(),
            quest_req: vec![],
            varp_req: r.varp_req.map(|v| vec![v]).unwrap_or_default(),
            worn_req: vec![],
        });
    }
}

// ---------------------------------------------------------------------------
// Shilo↔Brimhaven cart: the 2004 route pair (`TransportKind::Npc`).
// ---------------------------------------------------------------------------

/// One Shilo↔Brimhaven cart journey: `at` the cart driver NPC's spawn tile
/// (jm2 `==== NPC ====` placement, id resolved through `pack/npc.pack`),
/// `to` the destination cart tile the script's `p_teleport(` literal lands
/// on. The whole hop is one `Talk-to` (`opnpc1`), and the scripts carry no
/// `p_delay`, so `ticks` is the 1 op base like the spirit trees.
#[derive(Debug, Clone, Copy)]
struct CartRoute {
    /// npc.pack id of the cart driver who starts the journey.
    npc: i32,
    at: WorldTile,
    to: WorldTile,
    /// `(obj id, count)` fare: coins (`obj.pack` 995), count = the
    /// `calc_shilocart_cost` clamp cap.
    fare: Option<(i32, i32)>,
    /// The quest journal name gating the journey, if any.
    quest: Option<&'static str>,
}

/// The 2004 cart journeys: destinations from the `p_teleport(` calls in
/// `content/scripts/areas/area_brimhaven/scripts/hajedy.rs2` /
/// `content/scripts/areas/area_shilo/scripts/vigroy.rs2`, origin tiles from
/// the `==== NPC ====` placements in `content/maps/*.jm2`, and ids from
/// `pack/npc.pack`. The fare is `calc_shilocart_cost` in both scripts:
/// `(coins carried * 5) / 100`, clamped to 10–200 coins — the table keeps
/// the 200 cap. Hajedy refuses the ride until Shilo Village is complete
/// (`%zombiequeen >= ^zombiequeen_complete`); Vigroy's block carries no
/// gate.
const CART_ROUTES: &[CartRoute] = &[
    // Hajedy (brimhavencartdriver, npc 510) by the Brimhaven cart
    // (m43_50 local (27,11) = 2779,3211): `p_teleport(0_44_46_18_7)`
    // lands at the Shilo Village cart (2834,2951).
    CartRoute {
        npc: 510,
        at: WorldTile {
            x: 2779,
            z: 3211,
            level: 0,
        },
        to: WorldTile {
            x: 2834,
            z: 2951,
            level: 0,
        },
        fare: Some((995, 200)),
        quest: Some("Shilo Village"),
    },
    // Vigroy (shilocartdriver, npc 511) at the Shilo Village cart
    // (m44_46 local (18,10) = 2834,2954): `p_teleport(0_43_50_24_14)`
    // lands at the Brimhaven cart (2776,3214).
    CartRoute {
        npc: 511,
        at: WorldTile {
            x: 2834,
            z: 2954,
            level: 0,
        },
        to: WorldTile {
            x: 2776,
            z: 3214,
            level: 0,
        },
        fare: Some((995, 200)),
        quest: None,
    },
];

/// Cart edges from the 2004 route table: one `Talk-to` edge per journey,
/// keyed from the cart driver NPC's tile.
fn cart_edges(graph: &mut TransportGraph) {
    for r in CART_ROUTES {
        graph.edges.push(TransportEdge {
            kind: TransportKind::Npc,
            at: r.at,
            to: r.to,
            loc_id: r.npc,
            option: 1,
            ticks: 1,
            dir: None,
            open_loc_id: None,
            skill_req: vec![],
            item_req: r.fare.map(|(id, n)| vec![(id, n)]).unwrap_or_default(),
            quest_req: r.quest.map(|q| vec![q.to_string()]).unwrap_or_default(),
            varp_req: vec![],
            worn_req: vec![],
        });
    }
}

// ---------------------------------------------------------------------------
// Rune Mysteries essence mine: wizard entry teleports
// (`TransportKind::Npc`).
// ---------------------------------------------------------------------------

/// One essence-mine wizard journey: `at` the wizard NPC's placement tile
/// (jm2 `==== NPC ====` placement, id resolved through `pack/npc.pack`),
/// `to` the Rune Essence mine pad. The whole hop is the wizard's direct
/// teleport op — `[opnpc3,<name>]` calls `@teleport_to_essence_mine`, and
/// the `teleport_to_essence_mine` proc refuses below
/// `%runemysteries >= ^runemysteries_complete`, so the edge carries the
/// Rune Mysteries quest name.
#[derive(Debug, Clone, Copy)]
struct EssenceWizard {
    /// npc.pack id of the wizard who opens the portal.
    npc: i32,
    at: WorldTile,
    /// The wizard's direct teleport op (the `[opnpcN,…]` block that calls
    /// `@teleport_to_essence_mine`).
    option: i32,
}

/// The 2004 essence-mine wizards: placement tiles from the `==== NPC ====`
/// entries in `content/maps/*.jm2`, ids from `pack/npc.pack`, and the
/// direct-teleport op from each wizard script (`[opnpc4,aubury]` vs the
/// others' `[opnpc3,…]`). The proc lands the player at a random
/// `essence_mine_teleports` coord inside the enclosed mine (m45_75) and
/// stores the wizard's `^essence_mine_to_<wizard>` return anchor for the
/// exit portal, so the entry `to` is the mine's walkable centre pad and
/// the executor accepts any landing in the mine.
const ESSENCE_WIZARDS: &[EssenceWizard] = &[
    // Aubury (aubury, npc 553) in the Varrock rune shop (m50_53 local
    // (53,10)); `[opnpc4,aubury]`.
    EssenceWizard {
        npc: 553,
        at: WorldTile {
            x: 3253,
            z: 3402,
            level: 0,
        },
        option: 4,
    },
    // Sedridor (head_wizard, npc 300) in the Wizards' Tower cellar
    // (m48_149 local (31,35) — the 6400-cellar band of (3103,3171));
    // `[opnpc3,head_wizard]`.
    EssenceWizard {
        npc: 300,
        at: WorldTile {
            x: 3103,
            z: 9571,
            level: 0,
        },
        option: 3,
    },
    // Distentor (guild_wizard, npc 462) at the Magicians' Guild, Yanille
    // (m40_48 local (34,17)); `[opnpc3,guild_wizard]`.
    EssenceWizard {
        npc: 462,
        at: WorldTile {
            x: 2594,
            z: 3089,
            level: 0,
        },
        option: 3,
    },
    // Cromperty (ardounge_wizard, npc 844) in East Ardougne (m41_51
    // local (59,62)); `[opnpc3,ardounge_wizard]`.
    EssenceWizard {
        npc: 844,
        at: WorldTile {
            x: 2683,
            z: 3326,
            level: 0,
        },
        option: 3,
    },
    // Brimstail (gnome_brimstail, npc 171) in his cave (m37_153 local
    // (22,18) — the 6400-cellar band of (2390,3410));
    // `[opnpc3,gnome_brimstail]`.
    EssenceWizard {
        npc: 171,
        at: WorldTile {
            x: 2390,
            z: 9810,
            level: 0,
        },
        option: 3,
    },
];

/// The Rune Essence mine pad (m45_75 local (32,33)): the walkable centre
/// anchor the entry edges land on. The real landing is randomised among
/// the `essence_mine_teleports` enum coords, so the executor accepts any
/// landing inside the enclosed mine instead of this exact tile.
const ESSENCE_MINE_PAD: WorldTile = WorldTile {
    x: 2912,
    z: 4833,
    level: 0,
};

/// Essence-mine entry edges from the fixed wizard table: one direct
/// teleport hop per wizard, landing on the mine pad.
fn essence_mine_edges(graph: &mut TransportGraph) {
    for w in ESSENCE_WIZARDS {
        graph.edges.push(TransportEdge {
            kind: TransportKind::Npc,
            at: w.at,
            to: ESSENCE_MINE_PAD,
            loc_id: w.npc,
            option: w.option,
            ticks: ESSENCE_MINE_TICKS,
            dir: None,
            open_loc_id: None,
            skill_req: vec![],
            item_req: vec![],
            quest_req: vec!["Rune Mysteries".to_string()],
            varp_req: vec![],
            worn_req: vec![],
        });
    }
}

// ---------------------------------------------------------------------------
// Elkoy's Tree Gnome Village maze escorts (`TransportKind::Npc`).
// ---------------------------------------------------------------------------

/// One Elkoy escort journey: `at` the Elkoy NPC's placement tile (jm2
/// `==== NPC ====` placement, id resolved through `pack/npc.pack`), `to`
/// the coord the script's `p_telejump(` literal lands on. The whole hop
/// is one `Talk-to` (`opnpc1`) — the "Yes please."/"Can you show me
/// out…" choice is execute, never a search arm — and the scripts carry no
/// `p_delay`, so `ticks` is the 1 op base like the carts and spirit trees.
#[derive(Debug, Clone, Copy)]
struct ElkoyEscort {
    /// npc.pack id of the Elkoy who escorts the player.
    npc: i32,
    at: WorldTile,
    to: WorldTile,
}

/// The 2004 Elkoy escorts: the two `p_telejump(` destinations from
/// `content/scripts/areas/area_gnome/scripts/elkoy.rs2` —
/// `^elkoy_maze_coord` (the maze-side `[opnpc1,elkoy]` escort into the
/// village) and `^elkoy_entrance_coord` (the village `[opnpc1,elkoy_village]`
/// escort out) — resolved through `content/scripts/quests/quest_tree/
/// configs/quest_tree.constant` (`0_39_49_8_56` → (2504,3192),
/// `0_39_49_19_23` → (2515,3159)); origin tiles from the `==== NPC ====`
/// placements in `content/maps/m39_49.jm2` (npc 473 elkoy at local
/// (8,55) = (2504,3191), one tile south of the entrance coord; npc 474
/// elkoy_village at local (18,23) = (2514,3159), one tile west of the maze
/// coord); ids from `pack/npc.pack`. The edges carry the Tree Gnome
/// Village journal name — `elkoy.rs2`'s `[opnpc1,…]` blocks gate on
/// `%treequest` at every stage. The traveller walks no maze tiles: the
/// hop lands straight on the village/entrance coord (the script's own
/// landing, never a snap).
const ELKOY_ESCORTS: &[ElkoyEscort] = &[
    // elkoy (npc 473) by the maze entrance (m39_49 local (8,55)):
    // `p_telejump(^elkoy_maze_coord)` lands in the village (2515,3159).
    ElkoyEscort {
        npc: 473,
        at: WorldTile {
            x: 2504,
            z: 3191,
            level: 0,
        },
        to: WorldTile {
            x: 2515,
            z: 3159,
            level: 0,
        },
    },
    // elkoy_village (npc 474) in the village (m39_49 local (18,23)):
    // `p_telejump(^elkoy_entrance_coord)` lands at the maze entrance
    // (2504,3192).
    ElkoyEscort {
        npc: 474,
        at: WorldTile {
            x: 2514,
            z: 3159,
            level: 0,
        },
        to: WorldTile {
            x: 2504,
            z: 3192,
            level: 0,
        },
    },
];

/// Elkoy escort edges from the fixed 2004 route table: one `Talk-to` edge
/// per escort, keyed from the Elkoy NPC's tile.
fn elkoy_edges(graph: &mut TransportGraph) {
    for e in ELKOY_ESCORTS {
        graph.edges.push(TransportEdge {
            kind: TransportKind::Npc,
            at: e.at,
            to: e.to,
            loc_id: e.npc,
            option: 1,
            ticks: 1,
            dir: None,
            open_loc_id: None,
            skill_req: vec![],
            item_req: vec![],
            quest_req: vec!["Tree Gnome Village".to_string()],
            varp_req: vec![],
            worn_req: vec![],
        });
    }
}

// ---------------------------------------------------------------------------
// Gnome gliders: the 2004 Gnome Air network (fixed platform table).
// ---------------------------------------------------------------------------

/// The Grand Tree glider hub (Ta Quir Priw): `^ta_quir_priw =
/// 3_38_54_33_45` in `scripts/areas/area_gnome/configs/glider.constant`
/// (the Gnome pilot spawns one tile west).
const GLIDER_HUB: WorldTile = WorldTile {
    x: 2465,
    z: 3501,
    level: 3,
};

/// The four glider pads and their platforms, decoded from `glider.constant`
/// (`^gandius = 0_46_46_27_25` → (2971,2969), `^sindarpos = 0_44_54_34_41`,
/// `^lemanto_andra = 0_51_53_56_38`, `^kar_hewo = 0_51_50_20_11`). The
/// second field is whether the pad has a return flight to the hub.
const GLIDER_PADS: &[(WorldTile, bool)] = &[
    (
        WorldTile {
            x: 2971,
            z: 2969,
            level: 0,
        },
        true,
    ), // Gandius (Gnome Stronghold)
    (
        WorldTile {
            x: 2850,
            z: 3497,
            level: 0,
        },
        true,
    ), // Sindarpos (Al Kharid)
    (
        WorldTile {
            x: 3320,
            z: 3430,
            level: 0,
        },
        false,
    ), // Lemanto Andra (Varrock): one-way
    (
        WorldTile {
            x: 3284,
            z: 3211,
            level: 0,
        },
        true,
    ), // Kar-Hewo (Karamja)
];

/// Gnome pilot (npc.pack 170): the `Talk-to` target at every platform.
const GNOME_PILOT: i32 = 170;

/// The glider quest gate: the pilot offers Gnome Air only once the Grand
/// Tree quest is complete (`%grandtree >= ^grandtree_complete`, varp 150
/// = 160 in `scripts/areas/area_gnome/scripts/gnome_glider.rs2`'s
/// `[opnpc1,gnomepilot]` block).
const GLIDER_QUEST_REQ: (i32, i32) = (150, 160);

/// Glider edges from the fixed platform table: the hub to every pad, and
/// back from the round-trip pads. `calc_glidervar` in `gnome_glider.rs2`
/// allows only hub↔pad flights (pad↔pad shows "You can't go there at the
/// moment."), and has no lemanto_andra → hub pair, so Lemanto Andra is
/// one-way. The flight is a `p_delay(3)` + teleport on top of the
/// `Talk-to` op.
fn glider_edges(graph: &mut TransportGraph) {
    for (pad, round_trip) in GLIDER_PADS {
        graph.edges.push(glider_edge(GLIDER_HUB, *pad));
        if *round_trip {
            graph.edges.push(glider_edge(*pad, GLIDER_HUB));
        }
    }
}

fn glider_edge(at: WorldTile, to: WorldTile) -> TransportEdge {
    TransportEdge {
        kind: TransportKind::Glider,
        at,
        to,
        loc_id: GNOME_PILOT,
        option: 1,
        ticks: 4,
        dir: None,
        open_loc_id: None,
        skill_req: vec![],
        item_req: vec![],
        quest_req: vec![],
        varp_req: vec![GLIDER_QUEST_REQ],
        worn_req: vec![],
    }
}

// ---------------------------------------------------------------------------
// Spirit trees: the `area_gnome` network (three script blocks, content-read).
// ---------------------------------------------------------------------------

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
fn spirit_tree_edges(
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
fn spirit_tree_dest_names(body: &str) -> Vec<String> {
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
fn const_token(rest: &str) -> Option<&str> {
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
fn spirit_tree_gate(body: &str) -> Option<(String, String)> {
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

// ---------------------------------------------------------------------------
// Wilderness levers: the Ardougne↔wilderness teleport pair.
// ---------------------------------------------------------------------------

/// Wilderness lever edges from `scripts/areas/area_ardougne_east/scripts/
/// wilderness_lever.rs2` plus the folder's `wilderness_lever.constant`:
/// each `[oploc1,<loc>]` block's `~player_teleport_normal(^…_coord)` call
/// resolves through the constant's 5-part coord literal. One directed edge
/// per lever loc placement: `at` the lever loc tile (jm2 placement, like
/// every loc-backed edge), `to` the constant's tile, `Pull` op 1, two
/// ticks. Kind stays [`TransportKind::Door`] (the pack wire already
/// carries it; no version bump). The Ardougne→wilderness landing is inside
/// the wilderness zone, so the router only relaxes that edge under
/// `FindOptions::allow_wilderness`; the wilderness→Ardougne landing is not
/// and is always legal. The `%warning_wilderness_teleport_lever` confirm
/// dialog is execute, not search, and carries no edge.
fn lever_edges(
    content_root: &Path,
    ids: &HashMap<String, i32>,
    positions: &HashMap<i32, Vec<Placement>>,
    graph: &mut TransportGraph,
) {
    let dir = content_root
        .join("scripts")
        .join("areas")
        .join("area_ardougne_east");
    let Ok(script) = fs::read_to_string(dir.join("scripts").join("wilderness_lever.rs2")) else {
        return;
    };
    let Ok(constants) = fs::read_to_string(dir.join("configs").join("wilderness_lever.constant"))
    else {
        return;
    };
    // `^name` → the teleport tile (`0_mx_mz_lx_lz`, decoded like every
    // other coord literal).
    let mut lever_dests: HashMap<String, WorldTile> = HashMap::new();
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
                lever_dests.insert(name.to_string(), WorldTile { x, z, level });
            }
        }
    }

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
        let mut tos = Vec::new();
        for args in call_args_all(&body, "~player_teleport_normal") {
            let Some(dest) = args.first().and_then(|a| a.trim().strip_prefix('^')) else {
                continue;
            };
            if let Some(to) = lever_dests.get(dest) {
                tos.push(*to);
            }
        }
        for loc in placements {
            let at = WorldTile {
                x: loc.x,
                z: loc.z,
                level: loc.level,
            };
            for to in &tos {
                graph.edges.push(TransportEdge {
                    kind: TransportKind::Door,
                    at,
                    to: *to,
                    loc_id,
                    option: 1, // Pull (oploc1)
                    ticks: LEVER_TICKS,
                    dir: None,
                    open_loc_id: None,
                    skill_req: vec![],
                    item_req: vec![],
                    quest_req: vec![],
                    varp_req: vec![],
                    worn_req: vec![],
                });
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Al Kharid border toll and the Shantay northbound hop (item-gated gates).
// ---------------------------------------------------------------------------

/// The Al Kharid border toll: 10 coins (`inv_del(inv, coins, 10)` in
/// `border_gate.rs2`'s `pass_toll_gate`, guarded by
/// `inv_total(inv, coins) < 10`).
const AL_KHARID_TOLL_COINS: i32 = 10;
/// The Shantay henge doorway's `to`: `p_teleport(0_51_48_40_46)`
/// (3304,3118) then `p_telejump(movecoord(coord,0,0,-3))` → (3304,3115)
/// in `shantay_pass.rs2`'s `[queue,shantay_pass_enter]`.
const SHANTAY_NORTH_TO: WorldTile = WorldTile {
    x: 3304,
    z: 3115,
    level: 0,
};
/// The Shantay henge edge ticks: OP_BASE 1 + the `p_teleport` tick + the
/// `p_telejump` tick (both `p_delay(0)` in the queue block).
const SHANTAY_NORTH_TICKS: i32 = 3;

/// Al Kharid border-toll and Shantay-pass edges: `TransportKind::Door`
/// edges that cost an item, derived from `scripts/areas/area_alkharid/
/// configs/border_gate.loc` and `shantay_pass.rs2` plus the jm2
/// placements. The toll gates (`border_gate_toll_left`/`_right`, loc
/// 2882/2883) parse as doors under the same [`parse_door_config`] rule
/// (`op1=Open`) once their config's name-keyed blocks resolve through the
/// loc id map ([`parse_door_config_ids`]), and derive their two
/// crossings like every door: `at` the placement tile (m51_50 (4,27)/
/// (4,28) = (3268,3227)/(3268,3228)), `to` the far-side walk-out,
/// `open_loc_id` the config's `next_loc_stage` leaf (loc 1562/1563),
/// `item_req` the 10-coin toll. The Shantay henge doorway (loc 4031,
/// `op1=Go-through`) derives exactly the gated northbound hop — `at` the
/// m51_48 (38,44) placement = (3302,3116), `to` [`SHANTAY_NORTH_TO`],
/// `item_req` one Shantay pass (`inv_del(inv, shantay_pass, 1)` in the
/// same block). The free desert exit — the same block's `coordz(coord)
/// <= coordz(loc_coord)` teleport-jump — emits no edge; it stays a plain
/// walk.
fn toll_edges(
    content_root: &Path,
    ids: &HashMap<String, i32>,
    positions: &HashMap<i32, Vec<Placement>>,
    graph: &mut TransportGraph,
    collision: &WorldCollision,
) {
    // The toll charge and the Shantay pass resolve by name through
    // `pack/obj.pack`; a missing pack skips the family instead of faking
    // an item id.
    let objs = obj_ids_by_name(content_root);
    let Some(&coins_id) = objs.get("coins") else {
        return;
    };
    let Some(&pass_id) = objs.get("shantay_pass") else {
        return;
    };

    let alkharid = content_root
        .join("scripts")
        .join("areas")
        .join("area_alkharid");
    let Ok(config) = fs::read_to_string(alkharid.join("configs").join("border_gate.loc")) else {
        return;
    };
    let toll_ids = parse_door_config_ids(&config, ids);
    let open_ids = parse_door_open_ids(&config, ids);
    for id in toll_ids {
        let Some(placements) = positions.get(&id) else {
            continue;
        };
        for p in placements {
            if p.level != 0 || p.shape != 0 {
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
                let Some(to) = door_far_side(at, dir, collision) else {
                    continue;
                };
                graph.edges.push(TransportEdge {
                    kind: TransportKind::Door,
                    at,
                    to,
                    loc_id: id,
                    option: 1, // Open (oploc1, `@find_and_talk_to_border_guard`)
                    ticks: 1,
                    dir: Some(dir),
                    open_loc_id: open_ids.get(&id).copied(),
                    skill_req: vec![],
                    item_req: vec![(coins_id, AL_KHARID_TOLL_COINS)],
                    quest_req: vec![],
                    varp_req: vec![],
                    worn_req: vec![],
                });
            }
        }
    }

    // The Shantay henge: exactly the gated northbound hop, `at` the loc's
    // placement tile (shape 10, unlike the wall doors).
    let Some(&henge_id) = ids.get("shantay_pass_henge_doorway") else {
        return;
    };
    let Some(placements) = positions.get(&henge_id) else {
        return;
    };
    for p in placements {
        if p.level != 0 {
            continue;
        }
        let at = WorldTile {
            x: p.x,
            z: p.z,
            level: p.level,
        };
        graph.edges.push(TransportEdge {
            kind: TransportKind::Door,
            at,
            to: SHANTAY_NORTH_TO,
            loc_id: henge_id,
            option: 1, // Go-through (oploc1)
            ticks: SHANTAY_NORTH_TICKS,
            dir: None,
            open_loc_id: None,
            skill_req: vec![],
            item_req: vec![(pass_id, 1)],
            quest_req: vec![],
            varp_req: vec![],
            worn_req: vec![],
        });
    }
}

// ---------------------------------------------------------------------------
// The Zanaris shed door (`quest_zanaris.rs2`): a worn-item teleport door.
// ---------------------------------------------------------------------------

/// The Zanaris shed door ticks: OP_BASE 1 + the door block's `p_delay(1)`
/// + the `player_teleport_normal` cast `p_delay(2)` (the whole Open
/// channel; the shimmer `mes` and the open anim add no delay).
const ZANARIS_DOOR_TICKS: i32 = 4;

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
fn zanaris_door_edges(
    content_root: &Path,
    ids: &HashMap<String, i32>,
    positions: &HashMap<i32, Vec<Placement>>,
    graph: &mut TransportGraph,
) {
    let Ok(script) = fs::read_to_string(
        content_root
            .join("scripts")
            .join("quests")
            .join("quest_zanaris")
            .join("scripts")
            .join("quest_zanaris.rs2"),
    ) else {
        return;
    };
    let Some((_, name, body)) = script_blocks(&script)
        .into_iter()
        .find(|(op, name, _)| op.as_str() == "oploc1" && name.as_str() == "zanarisdoor")
    else {
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
        return;
    };
    // The Dramen staff id (`pack/obj.pack`); a missing pack skips the
    // door instead of faking an id.
    let Some(&staff_id) = obj_ids_by_name(content_root).get("dramen_staff") else {
        return;
    };
    let Some(placements) = positions.get(&loc_id) else {
        return;
    };
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
        });
    }
}

/// Teleport edges (the any-tile layer): the seven spell teleports from
/// `skill_magic/configs/magic_spells.dbrow` plus the jewellery rub
/// teleports from `general/scripts/enchanted_jewellry/*.rs2`, all into
/// [`TransportGraph::teleports`] — never `edges`/`at`, so the default
/// [`crate::router::find`] never sees them.
fn teleport_edges(
    content_root: &Path,
    graph: &mut TransportGraph,
    skipped: &mut HashMap<&'static str, usize>,
) {
    let objs = obj_ids_by_name(content_root);
    spell_teleports(content_root, &objs, graph, skipped);
    jewellery_teleports(content_root, &objs, graph, skipped);
}

/// Spell teleports from `skill_magic/configs/magic_spells.dbrow`: each
/// `[magic_spell_teleport_*]` block declares `data=levelrequired,N`,
/// `data=runesrequired,<rune>,<count>[,<rune>,<count>]` (rune names
/// resolved through `pack/obj.pack`), and `data=tele_coord,<coord>`
/// (absolute). Requirement = the magic level (`skill_req`) plus the runes
/// (`item_req`); the members flag declares no gate this model carries.
/// Ticks = [`SPELL_TELEPORT_TICKS`].
fn spell_teleports(
    content_root: &Path,
    objs: &HashMap<String, i32>,
    graph: &mut TransportGraph,
    skipped: &mut HashMap<&'static str, usize>,
) {
    let path = content_root
        .join("scripts")
        .join("skill_magic")
        .join("configs")
        .join("magic_spells.dbrow");
    let Ok(text) = fs::read_to_string(&path) else {
        return;
    };
    // (levelrequired, rune pairs, tele_coord) of the current teleport block.
    let mut cur: Option<(Option<i32>, Vec<(String, i32)>, Option<String>)> = None;
    for raw in text.lines() {
        let line = raw.trim();
        if let Some(name) = dbrow_block(line) {
            if let Some(block) = cur.take() {
                push_spell_teleport(objs, graph, skipped, block);
            }
            cur = name
                .starts_with("magic_spell_teleport_")
                .then(|| (None, Vec::new(), None));
            continue;
        }
        let Some((level, runes, coord)) = &mut cur else {
            continue;
        };
        if let Some(rest) = line.strip_prefix("data=levelrequired,") {
            *level = rest.trim().parse().ok();
        } else if let Some(rest) = line.strip_prefix("data=runesrequired,") {
            *runes = rune_pairs(rest);
        } else if let Some(rest) = line.strip_prefix("data=tele_coord,") {
            *coord = Some(rest.trim().to_string());
        }
    }
    if let Some(block) = cur.take() {
        push_spell_teleport(objs, graph, skipped, block);
    }
}

/// `[<name>]` dbrow section header → the block name.
fn dbrow_block(line: &str) -> Option<&str> {
    let name = line.strip_prefix('[')?.strip_suffix(']')?;
    if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
        return None;
    }
    Some(name)
}

/// `firerune,1,airrune,3,lawrune,1[,null,null]` → rune/count pairs; the
/// trailing `null,null` slot padding is dropped.
fn rune_pairs(rest: &str) -> Vec<(String, i32)> {
    let toks: Vec<&str> = rest.split(',').map(|t| t.trim()).collect();
    let mut out = Vec::new();
    for pair in toks.chunks(2) {
        if pair.len() < 2 || pair[0] == "null" {
            continue;
        }
        if let Ok(count) = pair[1].parse::<i32>() {
            out.push((pair[0].to_string(), count));
        }
    }
    out
}

fn push_spell_teleport(
    objs: &HashMap<String, i32>,
    graph: &mut TransportGraph,
    skipped: &mut HashMap<&'static str, usize>,
    (level, runes, coord): (Option<i32>, Vec<(String, i32)>, Option<String>),
) {
    let Some(level) = level else {
        bump(skipped, SKIP_TELEPORT_BAD_DEST, 1);
        return;
    };
    let Some(coord) = coord.and_then(|c| coord_literal(&c)) else {
        bump(skipped, SKIP_TELEPORT_BAD_DEST, 1);
        return;
    };
    let mut item_req = Vec::with_capacity(runes.len());
    for (rune, count) in &runes {
        let Some(&id) = objs.get(rune) else {
            bump(skipped, SKIP_TELEPORT_UNRESOLVED_RUNE, 1);
            return;
        };
        item_req.push((id, *count));
    }
    graph.teleports.push(TransportEdge {
        kind: TransportKind::Teleport,
        at: TELEPORT_PLACEHOLDER_AT,
        to: WorldTile {
            x: coord.1,
            z: coord.2,
            level: coord.0,
        },
        loc_id: 0, // a spell button, not a loc/obj use
        option: 0,
        ticks: SPELL_TELEPORT_TICKS,
        dir: None,
        open_loc_id: None,
        skill_req: vec![(SKILL_MAGIC, level)],
        item_req,
        quest_req: vec![],
        varp_req: vec![],
        worn_req: vec![],
    });
}

/// Jewellery rub teleports from `general/scripts/enchanted_jewellry/*.rs2`:
/// `[opheld4,<name>]` blocks whose body — directly or through a forwarded
/// `@label` (the glory rubs share `@amulet_of_glory_interface`) — calls
/// `~player_teleport_normal(<coord>|map_findsquare(<coord>, …))`. The block
/// name is the charged obj's name, or a `_`-prefixed category the obj
/// config `skill_magic/configs/enchanted_jewelry.obj` resolves
/// (`category=…`); obj ids come from `pack/obj.pack`. Requirement = holding
/// the charged item (`item_req`); `option` 4 is the Rub op (`opheld4`).
/// Ticks = [`JEWELLERY_TELEPORT_TICKS`].
fn jewellery_teleports(
    content_root: &Path,
    objs: &HashMap<String, i32>,
    graph: &mut TransportGraph,
    skipped: &mut HashMap<&'static str, usize>,
) {
    let cats = jewellery_categories(content_root);
    let dir = content_root
        .join("scripts")
        .join("general")
        .join("scripts")
        .join("enchanted_jewellry");
    let Ok(entries) = fs::read_dir(&dir) else {
        return;
    };
    for ent in entries.flatten() {
        let path = ent.path();
        if path.extension().and_then(|s| s.to_str()) != Some("rs2") {
            continue;
        }
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        for (op, name, body) in jewellery_blocks(&text) {
            if op != "opheld4" {
                continue;
            }
            let dests = block_teleport_dests(&body, &text);
            if dests.is_empty() {
                continue;
            }
            let items: Vec<String> = match name.strip_prefix('_') {
                Some(cat) => cats.get(cat).cloned().unwrap_or_default(),
                None => vec![name.clone()],
            };
            for item in items {
                let Some(&obj_id) = objs.get(&item) else {
                    bump(skipped, SKIP_TELEPORT_UNRESOLVED_ITEM, 1);
                    continue;
                };
                for dest in &dests {
                    graph.teleports.push(TransportEdge {
                        kind: TransportKind::Teleport,
                        at: TELEPORT_PLACEHOLDER_AT,
                        to: *dest,
                        loc_id: obj_id,
                        option: 4, // Rub (opheld4)
                        ticks: JEWELLERY_TELEPORT_TICKS,
                        dir: None,
                        open_loc_id: None,
                        skill_req: vec![],
                        item_req: vec![(obj_id, 1)],
                        quest_req: vec![],
                        varp_req: vec![],
                        worn_req: vec![],
                    });
                }
            }
        }
    }
}

/// `category=<cat>` → obj block names, from `skill_magic/configs/
/// enchanted_jewelry.obj` (the `_`-prefixed `opheld4` blocks dispatch on
/// these categories).
fn jewellery_categories(content_root: &Path) -> HashMap<String, Vec<String>> {
    let mut out: HashMap<String, Vec<String>> = HashMap::new();
    let path = content_root
        .join("scripts")
        .join("skill_magic")
        .join("configs")
        .join("enchanted_jewelry.obj");
    let Ok(text) = fs::read_to_string(&path) else {
        return out;
    };
    let mut cur: Option<&str> = None;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') {
            cur = line.strip_prefix('[').and_then(|s| s.strip_suffix(']'));
            continue;
        }
        if let Some(cat) = line.strip_prefix("category=") {
            if let Some(name) = cur {
                out.entry(cat.trim().to_string())
                    .or_default()
                    .push(name.to_string());
            }
        }
    }
    out
}

/// `(op, name, body)` blocks in an enchanted_jewellry file. Headers here
/// are `[op,<name>]` possibly with body text on the same line (the glory
/// `opheld4` one-liners) and/or a `(params)` list
/// (`[label,<name>](string $m)`); the strict [`script_header`] rejects
/// both, so these files need their own lenient parse.
fn jewellery_blocks(text: &str) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    let mut cur: Option<(String, String)> = None;
    let mut body = String::new();
    for raw in text.lines() {
        let line = match raw.find("//") {
            Some(i) => raw[..i].trim(),
            None => raw.trim(),
        };
        if line.is_empty() {
            continue;
        }
        if let Some((header, rest)) = jewellery_header(line) {
            if let Some(prev) = cur.take() {
                out.push((prev.0, prev.1, std::mem::take(&mut body)));
            }
            cur = Some((header.0.to_string(), header.1.to_string()));
            if let Some(rest) = rest {
                body.push_str(rest);
                body.push('\n');
            }
        } else if cur.is_some() {
            body.push_str(line);
            body.push('\n');
        }
    }
    if let Some(prev) = cur.take() {
        out.push((prev.0, prev.1, body));
    }
    out
}

/// `[op,<name>](params)` block header → `((op, name), same-line body)`.
/// A header line carries a trailing body only when there is no `(params)`
/// list after the `]`.
fn jewellery_header(line: &str) -> Option<((&str, &str), Option<&str>)> {
    let rest = line.strip_prefix('[')?;
    let close = rest.find(']')?;
    let head = &rest[..close];
    let tail = rest[close + 1..].trim();
    let (a, b) = head.split_once(',')?;
    let word = |s: &str| !s.is_empty() && s.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_');
    if !word(a) || !word(b) {
        return None;
    }
    if tail.is_empty() {
        Some(((a, b), None))
    } else if tail.starts_with('(') && tail.ends_with(')') {
        Some(((a, b), None))
    } else {
        Some(((a, b), Some(tail)))
    }
}

/// The body of `[label,<name>](…)` in the raw script text, from the header
/// line to the next block header line (params tolerated; the strict
/// [`script_header`] skips these headers).
fn label_body_raw(text: &str, name: &str) -> Option<String> {
    let needle = format!("[label,{name}]");
    let mut in_label = false;
    let mut out = String::new();
    for raw in text.lines() {
        let line = match raw.find("//") {
            Some(i) => raw[..i].trim(),
            None => raw.trim(),
        };
        if line.is_empty() {
            continue;
        }
        if in_label {
            if jewellery_header(line).is_some() {
                break;
            }
            out.push_str(line);
            out.push('\n');
        } else if line.starts_with(&needle) {
            in_label = true;
        }
    }
    in_label.then_some(out)
}

/// The destinations an `opheld4` block can take the player to: direct
/// `~player_teleport_normal(...)` calls in the block, plus any such calls
/// in the `@label` bodies the block forwards to.
fn block_teleport_dests(body: &str, script_text: &str) -> Vec<WorldTile> {
    let mut out = Vec::new();
    for args in call_args_all(body, "~player_teleport_normal") {
        if let Some(dest) = args.first().and_then(|a| teleport_dest(a)) {
            out.push(dest);
        }
    }
    for label in body_labels(body) {
        if let Some(lb) = label_body_raw(script_text, &label) {
            out.extend(block_teleport_dests(&lb, script_text));
        }
    }
    out
}

/// The landing of one `~player_teleport_normal(...)` arg: a 5-part coord
/// literal, or `map_findsquare(<coord>, …)` (the search square is the
/// literal's own tile).
fn teleport_dest(arg: &str) -> Option<WorldTile> {
    let arg = arg.trim();
    let coord = if let Some(inner) = arg.strip_prefix("map_findsquare(") {
        inner.split(',').next()?.trim()
    } else {
        arg
    };
    coord_literal(coord).map(|(level, x, z)| WorldTile { x, z, level })
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
    if !name
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'$' || b == b'_')
    {
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

/// Every `name(...)` call's args in `text`, in source order (the
/// first-match [`call_args`] variant; the jewellery rub scripts carry one
/// teleport per `case`, and each must resolve).
fn call_args_all(text: &str, name: &str) -> Vec<Vec<String>> {
    let needle = format!("{name}(");
    let mut out = Vec::new();
    let mut from = 0usize;
    while let Some(rel) = text[from..].find(&needle) {
        let at = from + rel;
        if at > 0 {
            let prev = text.as_bytes()[at - 1];
            if prev.is_ascii_alphanumeric() || prev == b'_' {
                from = at + needle.len();
                continue;
            }
        }
        let after = &text[at + needle.len()..];
        let mut args = Vec::new();
        let mut depth = 1i32;
        let mut start = 0usize;
        let mut closed = None;
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
                        closed = Some(i + 1);
                        break;
                    }
                }
                _ => {}
            }
        }
        let Some(consumed) = closed else {
            break;
        };
        out.push(args);
        from = at + needle.len() + consumed;
    }
    out
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
    use crate::collision::{bake_from_maps, WorldCollision};
    use client::config::{Cache, LocType};
    use client::io::JagFile;

    /// The real Server content root this machine bakes against (the same
    /// path `nav-pack` defaults to); `None` when the checkout is absent,
    /// so the content-backed tests skip with a message instead of faking
    /// coordinates.
    fn real_content_root() -> Option<PathBuf> {
        let root = PathBuf::from("/Users/acfrazier/experiments/Server/content");
        if root.join("maps").is_dir() && root.join("pack").join("loc.pack").is_file() {
            Some(root)
        } else {
            eprintln!(
                "SKIP: Server content not found at {} (content-backed tests skipped)",
                root.display()
            );
            None
        }
    }

    /// The real client-cache loc defs (`nav-pack`'s collision table), or
    /// `None` when the cache jag is absent.
    fn real_loc_defs() -> Option<LocDefs> {
        let jag = PathBuf::from("/Users/acfrazier/experiments/Server/engine/data/pack/config");
        let bytes = std::fs::read(&jag).ok()?;
        let cache = Cache::unpack(&JagFile::new(bytes));
        Some(LocDefs::from_locs(&cache.locs))
    }

    /// Derive the transport graph from the real Server content (the
    /// collision bake the graph's doors walk against); `None` when the
    /// content root or client cache is absent, so the content-backed
    /// tests skip with a message instead of faking coordinates.
    fn derive_from_real_content() -> Option<(TransportGraph, WorldCollision)> {
        let root = real_content_root()?;
        let defs = real_loc_defs()?;
        let wc = bake_from_maps(&root.join("maps"), &defs, &HashSet::new())
            .expect("real Server content bakes");
        let graph = derive_transports(&root, &defs, &wc);
        Some((graph, wc))
    }

    /// The real content must derive the Rune Mysteries essence-mine
    /// entries — one `TransportKind::Npc` edge per wizard who knows the
    /// teleport (Aubury, Sedridor, Distentor, Cromperty, Brimstail), each
    /// carrying the Rune Mysteries quest name and landing on the mine pad
    /// (m45_75, the walkable centre anchor of the enclosed mine; the real
    /// landing is randomised among the `essence_mine_teleports` enum
    /// coords, so the executor accepts any landing in the mine). The gate
    /// is the script's `%runemysteries >= ^runemysteries_complete` — the
    /// `teleport_to_essence_mine` proc refuses below it. Skips with a
    /// message when the Server content tree or the client cache is absent;
    /// never fakes coordinates.
    #[test]
    fn derive_transports_emits_essence_mine_entries() {
        let Some((graph, _)) = derive_from_real_content() else {
            return;
        };
        let ess: Vec<_> = graph
            .edges
            .iter()
            .filter(|e| {
                e.kind == TransportKind::Npc
                    && e.quest_req.iter().any(|q| {
                        q.to_ascii_lowercase().contains("rune mysteries") || q == "runemysteries"
                    })
            })
            .cloned()
            .collect();
        assert!(ess.len() >= 4, "Aubury+Sedridor+…, got {}", ess.len());
        // Each edge is the wizard NPC placement -> the enclosed mine pad.
        for e in &ess {
            assert_eq!(
                e.to,
                WorldTile {
                    x: 2912,
                    z: 4833,
                    level: 0
                },
                "every wizard lands on the mine pad: {e:?}"
            );
            assert!(
                e.quest_req
                    .iter()
                    .any(|q| q.to_ascii_lowercase().contains("rune mysteries")),
                "Rune Mysteries on the entry: {e:?}"
            );
        }
        // The five known wizards pin their mined placement tiles.
        let wizards = [
            (
                553,
                WorldTile {
                    x: 3253,
                    z: 3402,
                    level: 0,
                },
            ), // aubury (Varrock)
            (
                300,
                WorldTile {
                    x: 3103,
                    z: 9571,
                    level: 0,
                },
            ), // head_wizard (tower cellar)
            (
                462,
                WorldTile {
                    x: 2594,
                    z: 3089,
                    level: 0,
                },
            ), // guild_wizard (Yanille)
            (
                844,
                WorldTile {
                    x: 2683,
                    z: 3326,
                    level: 0,
                },
            ), // ardounge_wizard (Cromperty)
            (
                171,
                WorldTile {
                    x: 2390,
                    z: 9810,
                    level: 0,
                },
            ), // gnome_brimstail
        ];
        for (npc, at) in wizards {
            assert!(
                ess.iter().any(|e| e.loc_id == npc && e.at == at),
                "no entry edge from {at:?} (npc {npc})"
            );
        }
    }

    /// The real content must derive Elkoy's two Tree Gnome Village maze
    /// escorts (`elkoy_edges`): the maze-side Elkoy (npc 473) escorts into
    /// the village (`p_telejump(^elkoy_maze_coord)` → (2515,3159)) and the
    /// village Elkoy (npc 474) escorts out (`p_telejump(^elkoy_entrance_coord)`
    /// → (2504,3192)), each `Talk-to` op 1 carrying the Tree Gnome Village
    /// quest name. Skips with a message when the Server content tree or the
    /// client cache is absent; never fakes coordinates.
    #[test]
    fn derive_transports_emits_elkoy_escort_both_ways() {
        let Some((graph, _)) = derive_from_real_content() else {
            return;
        };
        let elk: Vec<_> = graph
            .edges
            .iter()
            .filter(|e| {
                e.kind == TransportKind::Npc
                    && ((e.to.x == 2504 && e.to.z == 3192) || (e.to.x == 2515 && e.to.z == 3159))
            })
            .cloned()
            .collect();
        assert_eq!(
            elk.len(),
            2,
            "maze-side + village escort, got {}",
            elk.len()
        );
        // The maze-side Elkoy (npc 473) sits at the maze entrance
        // (m39_49 local (8,55) = (2504,3191)) and escorts into the village;
        // the village Elkoy (npc 474, local (18,23) = (2514,3159)) escorts
        // back out to the entrance. Both hops land on the script's own
        // `p_telejump` coords (the quest_tree.constant values), never a
        // snap.
        for e in &elk {
            assert_eq!(e.option, 1, "Talk-to: {e:?}");
            assert!(
                e.quest_req.iter().any(|q| q == "Tree Gnome Village"),
                "Tree Gnome Village on the escort: {e:?}"
            );
        }
        let into_maze = elk
            .iter()
            .find(|e| {
                e.at == WorldTile {
                    x: 2504,
                    z: 3191,
                    level: 0,
                }
            })
            .expect("maze-side Elkoy placement");
        assert_eq!(into_maze.loc_id, 473);
        assert_eq!(
            into_maze.to,
            WorldTile {
                x: 2515,
                z: 3159,
                level: 0
            }
        );
        let out_maze = elk
            .iter()
            .find(|e| {
                e.at == WorldTile {
                    x: 2514,
                    z: 3159,
                    level: 0,
                }
            })
            .expect("village Elkoy placement");
        assert_eq!(out_maze.loc_id, 474);
        assert_eq!(
            out_maze.to,
            WorldTile {
                x: 2504,
                z: 3192,
                level: 0
            }
        );
    }

    /// The real content must derive the Zanaris shed door: the
    /// `[oploc1,zanarisdoor]` block's Open channel teleports through to
    /// Zanaris (`0_50_149_20_56` = (3220,9592)) when the Dramen staff is
    /// worn, so the door edge carries the staff's obj id as `worn_req`
    /// and the Lost City quest name. Skips with a message when the Server
    /// content tree or the client cache is absent; never fakes
    /// coordinates.
    #[test]
    fn derive_transports_emits_zanaris_shed_door_with_worn_dramen() {
        let Some((graph, _)) = derive_from_real_content() else {
            return;
        };
        let e = graph
            .edges
            .iter()
            .find(|e| e.kind == TransportKind::Door && !e.worn_req.is_empty())
            .expect("shed door");
        assert!(!e.worn_req.is_empty());
        assert!(
            e.to.x > 3000 && e.to.z > 9000,
            "Zanaris landing, not Lumbridge swamp"
        );
    }

    /// A throwaway content root written on demand, removed on drop.
    struct Fixture {
        root: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let root = std::env::temp_dir()
                .join(format!("nav-transport-fixture-{}-{n}", std::process::id()));
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

    /// A collision bake over the fixture's maps, with the given door locs
    /// stamped blocked-when-closed. Fixtures that write no maps get a
    /// trivial single-square bake (their assertions never touch the
    /// collision; `derive_transports` only needs one to walk door far
    /// sides out on).
    fn bake_collision(fx: &Fixture, defs: &LocDefs, door_ids: &HashSet<i32>) -> WorldCollision {
        if !fx.path().join("maps").is_dir() {
            fx.write("maps/m44_53.jm2", "==== MAP ====\n0 0 0: h1 u50\n");
        }
        bake_from_maps(&fx.path().join("maps"), defs, door_ids).unwrap()
    }

    #[test]
    fn derive_transports_door_edge_at_dir_to_open_loc_id() {
        let fx = Fixture::new();
        fx.write("pack/loc.pack", "1530=loc_1530\n1531=loc_1531\n");
        fx.write(
            "scripts/doors/configs/doors.loc",
            "[loc_1530]\nname=Door\nop1=Open\ncategory=door_closed\nparam=next_loc_stage,loc_1531\n",
        );
        // m44_53 local (0,46) = absolute (2816,3438). Wall 980 (angle
        // SOUTH) sits on the door's south approach tile (2816,3437), so
        // the south-bound far-side walk-out stops on that tile — its W_S
        // face flag stands (face flags never disqualify). The closed
        // door's own angle-NORTH stamp puts W_S on (2816,3439), which also
        // stands.
        fx.write(
            "maps/m44_53.jm2",
            "\
==== MAP ====
0 0 45: h1 o6 u50
0 0 46: h1 o6 u50
0 0 47: h1 o6 u50
==== LOC ====
0 0 46: 1530 0 1
0 0 45: 980 0 3
",
        );
        let defs = loc_defs(&[(1530, 1, 1), (980, 1, 1)]);
        let mut door_ids = HashSet::new();
        door_ids.insert(1530);
        let wc = bake_from_maps(&fx.path().join("maps"), &defs, &door_ids).unwrap();
        let graph = derive_transports(fx.path(), &defs, &wc);

        let doors: Vec<_> = graph
            .edges
            .iter()
            .filter(|e| e.kind == TransportKind::Door && e.loc_id == 1530)
            .collect();
        // Two edges per placement: `dir` and its opposite, each with its
        // own far side. `at` is the door loc tile, `to` the far side
        // walked out to standability.
        assert_eq!(doors.len(), 2);
        let n = doors
            .iter()
            .find(|e| e.dir == Some(DoorDir::N))
            .expect("north-bound door edge");
        let s = doors
            .iter()
            .find(|e| e.dir == Some(DoorDir::S))
            .expect("south-bound door edge");
        for d in [n, s] {
            assert_eq!(
                d.at,
                WorldTile {
                    x: 2816,
                    z: 3438,
                    level: 0
                }
            );
            assert_eq!(d.open_loc_id, Some(1531));
            assert_eq!(d.option, 1);
            assert_eq!(d.ticks, 1);
            assert!(d.varp_req.is_empty());
        }
        assert_eq!(
            n.to,
            WorldTile {
                x: 2816,
                z: 3439,
                level: 0
            }
        );
        // The south-bound walk-out stops on wall 980's own tile: its W_S
        // face flag stands (the wall's face flag never disqualifies).
        assert_eq!(
            s.to,
            WorldTile {
                x: 2816,
                z: 3437,
                level: 0
            }
        );
        // The at-index keys the door loc tile with both directed edges.
        assert_eq!(graph.at[&n.at].len(), 2);
    }

    /// The real content must derive at least one `TransportKind::Door`
    /// edge for the Sinclair wooden fence gates (loc 1551 / 1553):
    /// `door_edges` reads `scripts/general_use/configs/gates.loc` into the
    /// door set, not only `scripts/doors/configs/*.loc`. Skips with a
    /// message when the Server content tree or the client cache is absent;
    /// never fakes coordinates.
    #[test]
    fn derive_transports_content_emits_sinclair_gate_edges() {
        let Some(root) = real_content_root() else {
            return;
        };
        let Some(defs) = real_loc_defs() else {
            eprintln!("SKIP: client cache config jag missing");
            return;
        };
        let wc = bake_from_maps(&root.join("maps"), &defs, &HashSet::new())
            .expect("real Server content bakes");
        let graph = derive_transports(&root, &defs, &wc);
        let gates: Vec<_> = graph
            .edges
            .iter()
            .filter(|e| e.kind == TransportKind::Door && (e.loc_id == 1551 || e.loc_id == 1553))
            .collect();
        assert!(
            !gates.is_empty(),
            "no Door edges for the Sinclair wooden gates (loc 1551/1553) from the real content"
        );
    }

    /// The real content must derive the spirit-tree network: the stronghold
    /// tree (ent) flies to village/varrock/khazard, the village tree
    /// (stronghold_ent) back to khazard/varrock/stronghold, and each young
    /// tree (loc_1317, placed twice) to the village — 8 directed hops, the
    /// same count the rs2b0t catalog carries. Skips with a message when the
    /// Server content tree or the client cache is absent; never fakes
    /// coordinates.
    #[test]
    fn derive_transports_emits_spirit_tree_edges() {
        let Some((graph, _)) = derive_from_real_content() else {
            return;
        };
        let trees: Vec<_> = graph
            .edges
            .iter()
            .filter(|e| e.kind == TransportKind::SpiritTree)
            .collect();
        let n = trees.len();
        assert!(n >= 8, "rs2b0t catalog is 8 directed hops, got {n}");
        // The stronghold tree carries the Grand Tree gate
        // (`%grandtree >= ^grandtree_complete`), the village and young
        // trees the Tree Gnome Village gate (`%treequest >= ^tree_complete`)
        // — the same varps the gliders gate on.
        for e in &trees {
            assert_eq!(e.option, 1, "Talk-to");
            assert_eq!(e.ticks, SPIRIT_TREE_TICKS);
            assert_eq!(e.dir, None);
            match e.loc_id {
                1293 => assert_eq!(e.varp_req, vec![(150, 160)]),
                1294 | 1317 => assert_eq!(e.varp_req, vec![(111, 9)]),
                other => panic!("unexpected spirit-tree loc id {other}"),
            }
        }
        // The stronghold tree (ent, loc 1293) reaches the village, varrock,
        // and khazard trees; the village tree (stronghold_ent, loc 1294)
        // reaches back to khazard, varrock, and the stronghold.
        let dests = |loc_id: i32| -> Vec<WorldTile> {
            let mut v: Vec<WorldTile> = trees
                .iter()
                .filter(|e| e.loc_id == loc_id)
                .map(|e| e.to)
                .collect();
            v.sort_by_key(|t| (t.x, t.z));
            v.dedup();
            v
        };
        assert_eq!(
            dests(1293),
            vec![
                WorldTile {
                    x: 2542,
                    z: 3169,
                    level: 0
                }, // ^village_tree
                WorldTile {
                    x: 2555,
                    z: 3259,
                    level: 0
                }, // ^khazard_tree
                WorldTile {
                    x: 3179,
                    z: 3507,
                    level: 0
                }, // ^varrock_tree
            ]
        );
        assert_eq!(
            dests(1294),
            vec![
                WorldTile {
                    x: 2461,
                    z: 3444,
                    level: 0
                }, // ^stronghold_tree
                WorldTile {
                    x: 2555,
                    z: 3259,
                    level: 0
                }, // ^khazard_tree
                WorldTile {
                    x: 3179,
                    z: 3507,
                    level: 0
                }, // ^varrock_tree
            ]
        );
        // The young tree (loc_1317) is placed twice and only reaches the
        // village.
        let young: Vec<_> = trees.iter().filter(|e| e.loc_id == 1317).collect();
        assert_eq!(young.len(), 2);
        assert!(young.iter().all(|e| e.to
            == WorldTile {
                x: 2542,
                z: 3169,
                level: 0
            }));
    }

    /// The real content must derive at least one `TransportKind::Npc` edge
    /// for the Shilo↔Brimhaven cart (`cart_edges`, the `hajedy.rs2` /
    /// `vigroy.rs2` route pair): coins on the fare and the Shilo Village
    /// journal name on the Brim→Shilo hop. Skips with a message when the
    /// Server content tree or the client cache is absent; never fakes
    /// coordinates.
    #[test]
    fn derive_transports_emits_shilo_brimhaven_cart() {
        let Some((graph, _)) = derive_from_real_content() else {
            return;
        };
        let carts: Vec<_> = graph
            .edges
            .iter()
            .filter(|e| e.kind == TransportKind::Npc)
            .cloned()
            .collect();
        assert!(
            carts.len() >= 2,
            "both cart directions derive, got {}",
            carts.len()
        );
        assert!(
            carts.iter().any(|e| !e.item_req.is_empty()),
            "coins on the fare"
        );
        assert!(
            carts.iter().any(|e| !e.quest_req.is_empty()),
            "Shilo complete on Brim→Shilo"
        );
    }

    /// The real content must derive the two wilderness lever hops
    /// (`wilderness_lever.rs2` locs 1814/1815): the Ardougne lever's `to`
    /// is inside the wilderness zone and the wilderness lever's `to` is
    /// not. Skips with a message when the Server content tree or the
    /// client cache is absent; never fakes coordinates.
    #[test]
    fn derive_transports_emits_wildy_ardougne_levers() {
        let Some((graph, _)) = derive_from_real_content() else {
            return;
        };
        let levers: Vec<_> = graph
            .edges
            .iter()
            .filter(|e| e.loc_id == 1814 || e.loc_id == 1815)
            .cloned()
            .collect();
        assert!(
            levers.len() >= 2,
            "both lever directions derive, got {}",
            levers.len()
        );
        assert!(
            levers
                .iter()
                .any(|e| crate::wilderness::in_wilderness(e.to)),
            "the Ardougne→wildy lever must land inside the wilderness"
        );
        assert!(
            levers
                .iter()
                .any(|e| !crate::wilderness::in_wilderness(e.to)),
            "the wildy→Ardougne lever must land outside the wilderness"
        );
    }

    /// The real content must derive the Al Kharid border toll and the
    /// Shantay northbound hop as item-gated `TransportKind::Door` edges.
    /// The toll gates (`border_gate_toll_left`/`_right`, loc 2882/2883 —
    /// the m51_50 (4,27)/(4,28) placements = (3268,3227)/(3268,3228))
    /// carry the 10-coin toll (`inv_del(inv, coins, 10)` in
    /// border_gate.rs2's `pass_toll_gate`); the Shantay henge doorway
    /// (loc 4031, m51_48 (38,44) = (3302,3116)) derives exactly one gated
    /// hop into the desert — `to` (3304,3115), the landing of the
    /// `[queue,shantay_pass_enter]` `p_teleport(0_51_48_40_46)` +
    /// `p_telejump(movecoord(coord,0,0,-3))`, `item_req` one Shantay pass
    /// (obj 1854). The free desert exit (the `coordz(coord) <=
    /// coordz(loc_coord)` teleport-jump in the same `[oploc1,...]` block)
    /// must NOT become an edge. Skips with a message when the Server
    /// content tree or the client cache is absent; never fakes
    /// coordinates.
    #[test]
    fn derive_transports_emits_alkharid_toll_and_shantay_north() {
        let Some((graph, _)) = derive_from_real_content() else {
            return;
        };
        // Both toll gates derive their two crossings (dir + opposite),
        // pinned to the mined placements.
        let tolls: Vec<_> = graph
            .edges
            .iter()
            .filter(|e| e.kind == TransportKind::Door && (e.loc_id == 2882 || e.loc_id == 2883))
            .cloned()
            .collect();
        assert!(
            !tolls.is_empty(),
            "no Door edges for the Al Kharid toll gates (loc 2882/2883)"
        );
        assert_eq!(
            tolls.iter().filter(|e| e.loc_id == 2882).count(),
            2,
            "left toll gate derives both crossings"
        );
        assert_eq!(
            tolls.iter().filter(|e| e.loc_id == 2883).count(),
            2,
            "right toll gate derives both crossings"
        );
        for e in &tolls {
            assert_eq!(
                e.at,
                if e.loc_id == 2882 {
                    WorldTile {
                        x: 3268,
                        z: 3227,
                        level: 0,
                    }
                } else {
                    WorldTile {
                        x: 3268,
                        z: 3228,
                        level: 0,
                    }
                }
            );
            assert!(
                e.item_req.iter().any(|(id, n)| *id == 995 && *n >= 10),
                "10-coin toll on {e:?}"
            );
            assert_eq!(e.option, 1, "Open op");
            assert_eq!(
                e.open_loc_id,
                Some(if e.loc_id == 2882 { 1562 } else { 1563 })
            );
        }
        // The Shantay henge carries exactly the one gated northbound hop.
        let henge: Vec<_> = graph
            .edges
            .iter()
            .filter(|e| e.loc_id == 4031)
            .cloned()
            .collect();
        assert_eq!(
            henge.len(),
            1,
            "only the gated northbound hop derives, got {}",
            henge.len()
        );
        assert_eq!(
            henge[0].at,
            WorldTile {
                x: 3302,
                z: 3116,
                level: 0,
            }
        );
        assert_eq!(
            henge[0].to,
            WorldTile {
                x: 3304,
                z: 3115,
                level: 0,
            }
        );
        assert!(
            henge[0]
                .item_req
                .iter()
                .any(|(id, n)| *id == 1854 && *n >= 1),
            "Shantay pass on the northbound hop"
        );
        assert_eq!(henge[0].option, 1, "Go-through op");
        assert_eq!(henge[0].dir, None);
    }

    /// The Ardougne→wilderness lever is an enter-wildy hop: default
    /// [`crate::router::find`] must never relax it (its `to` is inside the
    /// wilderness zone), and [`crate::router::find_with`] with
    /// `allow_wilderness` must route through it. Fixture: an isolated
    /// content root whose only lever is that one (same placement and
    /// `p_teleport` destination constant the real content declares).
    #[test]
    fn default_find_skips_the_ardougne_to_wildy_lever() {
        use crate::router::{find, find_with, FindOptions, RouteError};
        use crate::wilderness::in_wilderness;

        let fx = Fixture::new();
        fx.write("pack/loc.pack", "1814=wildinlever\n");
        fx.write(
            "scripts/areas/area_ardougne_east/configs/wilderness_lever.constant",
            "^ardougne_to_wilderness_coord = 0_49_61_18_20\n",
        );
        fx.write(
            "scripts/areas/area_ardougne_east/scripts/wilderness_lever.rs2",
            "\
[oploc1,wildinlever]
p_arrivedelay;
if (%warning_wilderness_teleport_lever = ^false) {
    ~mesbox(\"Warning! Pulling the lever will teleport you deep into the wilderness.\");
    def_int $choice = ~p_choice3_header(\"Yes I'm brave.\", 1, \"Eep! The wilderness... No thank you.\", 2, \"Yes please, don't show this message again.\", 3, \"Are you sure you wish to pull it?\");
    if ($choice = 2) {
        return;
    }
    if ($choice = 3) {
        %warning_wilderness_teleport_lever = ^true;
    }
}
anim(human_leverdown, 0);
sound_synth(lever, 1, 0);
loc_change(hauntedleverdown, 7);
if_close;
p_delay(1);
mes(\"You pull the lever...\");
p_delay(0);
~player_teleport_normal(^ardougne_to_wilderness_coord);
mes(\"...And teleport into the wilderness.\");
",
        );
        // m40_51 local (1,47) = absolute (2561,3311,0); the constant
        // `0_49_61_18_20` = (3154,3924,0), inside the surface zone.
        fx.write(
            "maps/m40_51.jm2",
            "\
==== MAP ====
0 1 47: h1 o6 u50
==== LOC ====
0 1 47: 1814 4
",
        );
        let defs = loc_defs(&[(1814, 1, 1)]);
        let wc = bake_collision(&fx, &defs, &HashSet::new());
        let graph = derive_transports(fx.path(), &defs, &wc);

        let at = WorldTile {
            x: 2561,
            z: 3311,
            level: 0,
        };
        let to = WorldTile {
            x: 3154,
            z: 3924,
            level: 0,
        };
        let levers: Vec<_> = graph
            .edges
            .iter()
            .filter(|e| e.kind == TransportKind::Door && e.loc_id == 1814)
            .cloned()
            .collect();
        assert_eq!(levers.len(), 1, "one placement, one Pull edge");
        assert_eq!(levers[0].at, at);
        assert_eq!(levers[0].to, to);
        assert_eq!(levers[0].option, 1); // Pull (oploc1)
        assert_eq!(levers[0].dir, None);
        assert!(in_wilderness(to));

        // Default find: the enter-wildy hop is never relaxed, and no walk
        // path can reach the landing — NoPath.
        assert!(matches!(find(&wc, &graph, at, to), Err(RouteError::NoPath)));
        // find_with(allow_wilderness): the same hop routes through.
        let route = find_with(
            &wc,
            &graph,
            at,
            to,
            FindOptions {
                allow_teleports: false,
                allow_wilderness: true,
            },
        )
        .expect("allow_wilderness routes the Ardougne→wildy lever");
        assert_eq!(route.dest, to);
    }

    /// The process nav pack path (`$NAV_PACK` or `~/.274bot/274bot.navpack`,
    /// the same default `nav-pack` writes and the panel reads).
    fn default_pack_path() -> PathBuf {
        match std::env::var("NAV_PACK") {
            Ok(p) => PathBuf::from(p),
            Err(_) => match std::env::var("HOME") {
                Ok(home) => PathBuf::from(format!("{home}/.274bot/274bot.navpack")),
                Err(_) => PathBuf::from(".274bot/274bot.navpack"),
            },
        }
    }

    /// The gates seam: a route must now exist from Seers street
    /// (2725,3485,0) to the rock-crab shore (2710,3720,0) once the fence
    /// gates join the door set. Loads the baked process pack (the rebaked
    /// one carries the gate edges) if present, else bakes + derives from
    /// the Server content. A `NoPath` here is the honest two-component
    /// signal — the test must fail, never be papered over with a fake
    /// corridor or a bank door.
    #[test]
    fn seers_street_reaches_rock_crabs_after_gates() {
        use crate::router::find;
        use crate::world::NavWorld;

        let from = WorldTile {
            x: 2725,
            z: 3485,
            level: 0,
        };
        let to = WorldTile {
            x: 2710,
            z: 3720,
            level: 0,
        };
        let pack = default_pack_path();
        let (collision, graph) = match NavWorld::load_pack(&pack) {
            Ok(world) => (world.collision, world.graph),
            Err(e) => {
                let Some(root) = real_content_root() else {
                    panic!(
                        "no nav pack at {} ({e:?}) and no Server content to bake the fallback",
                        pack.display()
                    );
                };
                let defs = real_loc_defs().expect("client cache config jag");
                let wc = bake_from_maps(&root.join("maps"), &defs, &HashSet::new())
                    .expect("real Server content bakes");
                let graph = derive_transports(&root, &defs, &wc);
                (wc, graph)
            }
        };
        let route = find(&collision, &graph, from, to).unwrap_or_else(|e| {
            panic!("Seers street -> rock crabs must route once gates join: {e:?}")
        });
        assert_eq!(route.dest, to);
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
        let mut door_ids = HashSet::new();
        door_ids.insert(1530);
        let wc = bake_collision(&fx, &defs, &door_ids);
        let graph = derive_transports(fx.path(), &defs, &wc);

        // The Catherby door (loc 1530 @ 2816,3438,0, angle 1): two edges
        // per placement — `at` the loc tile, `dir` N and S, each `to` the
        // far side walked out to standability, `Open` op 1, one tick.
        let doors: Vec<_> = graph
            .edges
            .iter()
            .filter(|e| e.kind == TransportKind::Door && e.loc_id == 1530)
            .collect();
        assert_eq!(doors.len(), 2);
        let n = doors
            .iter()
            .find(|e| e.dir == Some(DoorDir::N))
            .expect("north-bound door edge");
        let s = doors
            .iter()
            .find(|e| e.dir == Some(DoorDir::S))
            .expect("south-bound door edge");
        for d in [n, s] {
            assert_eq!(
                d.at,
                WorldTile {
                    x: 2816,
                    z: 3438,
                    level: 0
                }
            );
            assert_eq!(d.option, 1);
            assert_eq!(d.ticks, 1);
        }
        // (2816,3439) carries the closed door's own south-face stamp, which
        // stands (face flags never disqualify); the south far side is the
        // open tile straight below the door.
        assert_eq!(
            n.to,
            WorldTile {
                x: 2816,
                z: 3439,
                level: 0
            }
        );
        assert_eq!(
            s.to,
            WorldTile {
                x: 2816,
                z: 3437,
                level: 0
            }
        );

        // One ladder placement (id 1747 @ 2826,3402,0) climbing to
        // (1,2826,3468): one edge per placement — `at` the loc tile
        // (blocked), `to` the same landing.
        let ladders: Vec<_> = graph
            .edges
            .iter()
            .filter(|e| e.kind == TransportKind::Ladder && e.loc_id == 1747)
            .collect();
        assert_eq!(ladders.len(), 1);
        let landing = WorldTile {
            x: 2826,
            z: 3468,
            level: 1,
        };
        let ladder = &ladders[0];
        assert_eq!(
            ladder.at,
            WorldTile {
                x: 2826,
                z: 3402,
                level: 0
            }
        );
        assert_eq!(ladder.to, landing);
        assert_eq!(ladder.dir, None);
        assert_eq!(ladder.open_loc_id, None);
        assert_eq!(ladder.option, 1);
        assert_eq!(ladder.ticks, 3); // op base 1 + ladder extra 2
        assert!(ladder.skill_req.is_empty());

        // The at-index keys the door loc tile (both directed edges) and the
        // ladder loc tile.
        let door_at = WorldTile {
            x: 2816,
            z: 3438,
            level: 0,
        };
        assert_eq!(graph.at[&door_at].len(), 2);
        let door_tos: Vec<_> = graph.at[&door_at]
            .iter()
            .map(|&i| graph.edges[i].to)
            .collect();
        assert!(door_tos.contains(&WorldTile {
            x: 2816,
            z: 3439,
            level: 0
        }));
        assert!(door_tos.contains(&WorldTile {
            x: 2816,
            z: 3437,
            level: 0
        }));
        let ladder_at = WorldTile {
            x: 2826,
            z: 3402,
            level: 0,
        };
        assert_eq!(graph.at[&ladder_at].len(), 1);
        assert_eq!(graph.edges[graph.at[&ladder_at][0]].to, landing);
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
        let wc = bake_collision(&fx, &defs, &HashSet::new());
        let graph = derive_transports(fx.path(), &defs, &wc);

        let edges: Vec<_> = graph
            .edges
            .iter()
            .filter(|e| e.kind == TransportKind::AgilityShortcut && e.loc_id == 2298)
            .collect();
        assert_eq!(edges.len(), 1);
        let landing = WorldTile {
            x: 2821,
            z: 3400,
            level: 0,
        };
        let e = &edges[0];
        // One edge per placement: `at` the loc tile, `to` the shortcut dest.
        assert_eq!(
            e.at,
            WorldTile {
                x: 2821,
                z: 3397,
                level: 0
            }
        );
        assert_eq!(e.to, landing);
        assert_eq!(e.dir, None);
        assert_eq!(e.open_loc_id, None);
        assert_eq!(e.option, 1);
        assert_eq!(e.ticks, 1); // op base 1 + watchshortcut extra 0
        assert_eq!(e.skill_req, vec![(SKILL_AGILITY, 5)]);
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
            parse_statement(
                "@ladder_options(movecoord(coord(), 0, 1, 0), movecoord(coord(), 0, -1, 0));"
            ),
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
    fn parse_script_records_unguarded_statements_as_fallback() {
        let mut rules = HashMap::new();
        parse_script(
            "\
[oploc1,ship_ladder]
p_arrivedelay;
~climb_ladder(movecoord(coord(), 0, 1, 0), true);
",
            TransportKind::Ladder,
            &mut rules,
        );
        let (_, rule) = rules.get(&("ship_ladder".to_string(), 1)).unwrap();
        assert!(matches!(
            rule.fallback,
            Some(Outcome::Landing(Landing::FromLevel { d: 1 }))
        ));
    }

    #[test]
    fn parse_script_records_dialog_skip_as_fallback() {
        let mut rules = HashMap::new();
        parse_script(
            "\
[oploc1,laddermiddle]
p_arrivedelay;
@ladder_options(movecoord(coord(), 0, 1, 0), movecoord(coord(), 0, -1, 0));
",
            TransportKind::Ladder,
            &mut rules,
        );
        let (_, rule) = rules.get(&("laddermiddle".to_string(), 1)).unwrap();
        assert!(matches!(rule.fallback, Some(Outcome::Skipped(SKIP_DIALOG))));
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
        let wc = bake_collision(&fx, &defs, &HashSet::new());
        let graph = derive_transports(fx.path(), &defs, &wc);
        // The unknown ladder name resolves nothing; the only edges are the
        // explicit 2004 boat route, cart, essence-wizard, and gnome-glider
        // tables.
        let explicit = graph
            .edges
            .iter()
            .filter(|e| {
                e.kind == TransportKind::Boat
                    || e.kind == TransportKind::Glider
                    || e.kind == TransportKind::Npc
            })
            .count();
        assert_eq!(explicit, graph.edges.len());
        assert_eq!(
            graph
                .edges
                .iter()
                .filter(|e| e.kind == TransportKind::Boat)
                .count(),
            8
        );
        // 2 carts + the 5 essence-mine wizard entries + the 2 Elkoy maze
        // escorts.
        assert_eq!(
            graph
                .edges
                .iter()
                .filter(|e| e.kind == TransportKind::Npc)
                .count(),
            9
        );
        assert_eq!(
            graph
                .edges
                .iter()
                .filter(|e| e.kind == TransportKind::Glider)
                .count(),
            7
        );
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
        let wc = bake_collision(&fx, &defs, &HashSet::new());
        let graph = derive_transports(fx.path(), &defs, &wc);
        assert!(graph.edges.iter().all(|e| e.kind != TransportKind::Door));
    }

    #[test]
    fn derive_transports_emits_boat_edges_from_npc_tile_to_dock_tile() {
        let fx = Fixture::new();
        let defs = loc_defs(&[]);
        let wc = bake_collision(&fx, &defs, &HashSet::new());
        let graph = derive_transports(fx.path(), &defs, &wc);

        let boats: Vec<_> = graph
            .edges
            .iter()
            .filter(|e| e.kind == TransportKind::Boat)
            .collect();
        assert_eq!(boats.len(), 8);

        let boat = |npc: i32, at: WorldTile| -> &TransportEdge {
            boats
                .iter()
                .find(|e| e.loc_id == npc && e.at == at)
                .unwrap_or_else(|| panic!("boat route npc {npc} at {at:?}"))
        };

        // Port Sarim → Musa: `at` is Seaman Thresnor's tile (npc 378, jm2
        // m47_50 `==== NPC ====`), NOT the origin gangplank; `to` is the
        // Karamja dock past `sarimshipplank_off` (loc 2082, north-facing,
        // disembark lands loc + (0,-1,+2) = (2956,3146,0)), never the boat
        // interior (2956,3143,1).
        let ps_musa = boat(
            378,
            WorldTile {
                x: 3026,
                z: 3217,
                level: 0,
            },
        );
        assert_eq!(
            ps_musa.to,
            WorldTile {
                x: 2956,
                z: 3146,
                level: 0
            }
        );
        assert_eq!(ps_musa.option, 1); // Talk-to
        assert_eq!(ps_musa.ticks, 9); // set_sail delay 7 + gangplank crossing 2
        assert_eq!(ps_musa.item_req, vec![(995, 30)]); // 30-coin fare
        assert!(ps_musa.varp_req.is_empty());

        // Musa → Port Sarim: the customs officer (npc 380) at (2955,3146,0),
        // landing on the Port Sarim dock past `karamjashipplank_off`
        // (loc 2084, west-facing, disembark lands loc + (-2,-1,0) =
        // (3029,3217,0)).
        let musa_ps = boat(
            380,
            WorldTile {
                x: 2955,
                z: 3146,
                level: 0,
            },
        );
        assert_eq!(
            musa_ps.to,
            WorldTile {
                x: 3029,
                z: 3217,
                level: 0
            }
        );
        assert_eq!(musa_ps.ticks, 9);

        // No boat edge lands on a boat-interior/water tile: every `to` is a
        // dock tile past the destination gangplank.
        let interiors = [
            (2956, 3143), // Musa ship deck
            (3032, 3217), // Port Sarim ship deck
            (2683, 3268), // Ardougne ship deck
            (2775, 3234), // Brimhaven ship deck
            (2834, 3331), // Entrana ship deck
            (3048, 3231), // Port Sarim → Entrana ship deck
        ];
        for b in &boats {
            assert!(
                !interiors.contains(&(b.to.x, b.to.z)),
                "boat edge ends on a boat interior: {:?}",
                b.to
            );
        }

        // Shilo boats (Captain Shanks, npc 518) carry the Shilo Village gate:
        // `%zombiequeen >= ^zombiequeen_complete` (varp 116 = 15), and land
        // directly on the declared dock tiles (no gangplank crossing).
        let shanks: Vec<_> = boats.iter().filter(|e| e.loc_id == 518).collect();
        assert_eq!(shanks.len(), 2);
        for s in &shanks {
            assert_eq!(s.varp_req, vec![(116, 15)]);
            assert_eq!(s.option, 1);
        }
        let khazard = boat(
            518,
            WorldTile {
                x: 2763,
                z: 2961,
                level: 1,
            },
        );
        assert_eq!(
            khazard.to,
            WorldTile {
                x: 2680,
                z: 3150,
                level: 0
            }
        );
        assert_eq!(khazard.ticks, 9); // set_sail_cairn delay 9, direct landing
        let shanks_sarim = shanks
            .iter()
            .find(|s| {
                s.to == WorldTile {
                    x: 3047,
                    z: 3235,
                    level: 0,
                }
            })
            .expect("Shilo → Port Sarim boat");
        assert_eq!(shanks_sarim.ticks, 15);
    }

    #[test]
    fn derive_transports_carries_quest_door_varp_req() {
        let fx = Fixture::new();
        fx.write("pack/loc.pack", "2526=elenagateshut\n4=mcannondoor1\n");
        fx.write("pack/varp.pack", "165=elenaquest\n0=mcannon\n");
        fx.write(
            "scripts/quests/quest_elena/configs/doors.loc",
            "\
[elenagateshut]
name=Door
model=basic_wall
active=yes
op1=Open
category=door_open_and_close
param=next_loc_stage,elenagateopen
",
        );
        fx.write(
            "scripts/quests/quest_elena/configs/quest_elena.constant",
            "^quest_elena_freed_elena = 28\n^elena_complete = 29\n",
        );
        fx.write(
            "scripts/quests/quest_elena/scripts/plaguehouse.rs2",
            "\
[oploc1,elenagateshut] // elena door
switch_int(%elenaquest) {
    case ^quest_elena_freed_elena, ^elena_complete : ~open_and_close_door(loc_param(next_loc_stage), ~check_axis(coord, loc_coord, loc_angle), false);
    case default : mes(\"The door is locked.\");
}
",
        );
        fx.write(
            "scripts/quests/quest_mcannon/configs/mcannon_doors.loc",
            "\
[mcannondoor1]
name=Door
model=basic_wall
op1=Open
category=door_closed
",
        );
        fx.write(
            "scripts/quests/quest_mcannon/configs/quest_mcannon.constant",
            "^mcannon_tasked_with_fixing_cannon = 6\n",
        );
        fx.write(
            "scripts/quests/quest_mcannon/scripts/mcannon_doors.rs2",
            "\
[oploc1,mcannondoor1]
if (%mcannon >= ^mcannon_tasked_with_fixing_cannon) {
    @open_dwarf_cannon_door;
} else {
    mes(\"The door is locked.\");
}

[label,open_dwarf_cannon_door]
~open_and_close_door(loc_param(next_loc_stage), true, false);
",
        );
        fx.write(
            "maps/m44_53.jm2",
            "\
==== MAP ====
0 0 0: h1 o6 u50
0 0 2: h1 o6 u50
0 1 0: h1 o6 u50
0 3 0: h1 o6 u50
==== LOC ====
0 0 1: 2526 0 2
0 2 0: 4 0 0
",
        );
        let defs = loc_defs(&[(2526, 1, 1), (4, 1, 1)]);
        let mut door_ids = HashSet::new();
        door_ids.extend([2526, 4]);
        let wc = bake_collision(&fx, &defs, &door_ids);
        let graph = derive_transports(fx.path(), &defs, &wc);

        // The Elena door (Plague City) carries its `%elenaquest >= 28`
        // gate on its east-bound edge (the west-bound far side is off the
        // bake's grid, so no west-bound edge resolves).
        let elena: Vec<_> = graph
            .edges
            .iter()
            .filter(|e| e.kind == TransportKind::Door && e.loc_id == 2526)
            .collect();
        // One edge per placement (a single loc placement each); the west
        // far side never becomes standable inside the bake.
        assert_eq!(elena.len(), 1);
        for d in &elena {
            assert_eq!(d.varp_req, vec![(165, 28)]);
            assert!(d.quest_req.is_empty());
        }
        // The dwarf-cannon door's `if (%mcannon >= ^…) { @label }` gate.
        let cannon: Vec<_> = graph
            .edges
            .iter()
            .filter(|e| e.kind == TransportKind::Door && e.loc_id == 4)
            .collect();
        assert_eq!(cannon.len(), 2);
        for d in &cannon {
            assert_eq!(d.varp_req, vec![(0, 6)]);
        }
    }

    #[test]
    fn derive_transports_emits_glider_edges_from_platform_to_platform() {
        let fx = Fixture::new();
        let defs = loc_defs(&[]);
        let wc = bake_collision(&fx, &defs, &HashSet::new());
        let graph = derive_transports(fx.path(), &defs, &wc);

        let gliders: Vec<_> = graph
            .edges
            .iter()
            .filter(|e| e.kind == TransportKind::Glider)
            .collect();
        // The Grand Tree hub flies to all four pads and back from three of
        // them (`calc_glidervar` has no lemanto_andra → hub pair): 7 edges.
        assert_eq!(gliders.len(), 7);
        let hub = WorldTile {
            x: 2465,
            z: 3501,
            level: 3,
        };
        let sindarpos = WorldTile {
            x: 2850,
            z: 3497,
            level: 0,
        };
        let gandius = WorldTile {
            x: 2971,
            z: 2969,
            level: 0,
        };
        let lemanto_andra = WorldTile {
            x: 3320,
            z: 3430,
            level: 0,
        };
        let hub_edges: Vec<_> = gliders.iter().filter(|e| e.at == hub).collect();
        assert_eq!(hub_edges.len(), 4);
        assert!(hub_edges.iter().any(|e| e.to == sindarpos));
        assert!(hub_edges.iter().any(|e| e.to == gandius));
        assert!(hub_edges.iter().any(|e| e.to == lemanto_andra));
        let sindarpos_edges: Vec<_> = gliders.iter().filter(|e| e.at == sindarpos).collect();
        assert_eq!(sindarpos_edges.len(), 1);
        assert_eq!(sindarpos_edges[0].to, hub);
        // Lemanto Andra is one-way: no pad → hub flight exists in
        // gnome_glider.rs2.
        assert!(gliders.iter().all(|e| e.at != lemanto_andra));
        for g in &gliders {
            assert_eq!(g.varp_req, vec![(150, 160)]); // Grand Tree complete
            assert_eq!(g.option, 1); // Talk-to the Gnome pilot
            assert_eq!(g.loc_id, 170);
        }
    }

    #[test]
    fn derive_transports_derives_spell_teleports_as_any_tile_edges() {
        let fx = Fixture::new();
        fx.write("pack/obj.pack", "554=firerune\n556=airrune\n563=lawrune\n");
        fx.write(
            "scripts/skill_magic/configs/magic_spells.dbrow",
            "\
[magic_spell_teleport_varrock]
table=magic_spell_table
data=spell,^varrock_teleport
data=members,false
data=levelrequired,25
data=runesrequired,firerune,1,airrune,3,lawrune,1
data=experience,350
data=tele_coord,0_50_53_13_32

[magic_spell_teleport_trollheim]
table=magic_spell_table
data=spell,^trollheim_teleport
data=members,true
data=levelrequired,61
data=runesrequired,firerune,2,lawrune,2,null,null
data=experience,680
data=tele_coord,0_45_57_10_31
",
        );
        let defs = loc_defs(&[]);
        let wc = bake_collision(&fx, &defs, &HashSet::new());
        let graph = derive_transports(fx.path(), &defs, &wc);

        assert_eq!(graph.teleports.len(), 2);
        // Teleports never join the `at`-indexed edge set.
        assert!(graph
            .edges
            .iter()
            .all(|e| e.kind != TransportKind::Teleport));
        assert!(!graph.at.contains_key(&TELEPORT_PLACEHOLDER_AT));

        let varrock = graph
            .teleports
            .iter()
            .find(|e| {
                e.to == WorldTile {
                    x: 3213,
                    z: 3424,
                    level: 0,
                }
            })
            .expect("Varrock teleport");
        assert_eq!(varrock.kind, TransportKind::Teleport);
        assert_eq!(varrock.skill_req, vec![(SKILL_MAGIC, 25)]);
        assert_eq!(varrock.item_req, vec![(554, 1), (556, 3), (563, 1)]);
        assert_eq!(varrock.ticks, SPELL_TELEPORT_TICKS);

        let trollheim = graph
            .teleports
            .iter()
            .find(|e| {
                e.to == WorldTile {
                    x: 2890,
                    z: 3679,
                    level: 0,
                }
            })
            .expect("Trollheim teleport");
        // The trailing `null,null` rune-slot padding is dropped.
        assert_eq!(trollheim.item_req, vec![(554, 2), (563, 2)]);
        assert_eq!(trollheim.skill_req, vec![(SKILL_MAGIC, 61)]);
        assert_eq!(trollheim.ticks, SPELL_TELEPORT_TICKS);
    }

    #[test]
    fn derive_transports_derives_jewellery_teleports_with_item_reqs() {
        let fx = Fixture::new();
        fx.write(
            "pack/obj.pack",
            "1712=amulet_of_glory_4\n2552=ring_of_dueling_8\n",
        );
        fx.write(
            "scripts/skill_magic/configs/enchanted_jewelry.obj",
            "\
[ring_of_dueling_8]
name=Ring of dueling(8)
iop4=Rub
category=category_136
param=charges,8
",
        );
        fx.write(
            "scripts/general/scripts/enchanted_jewellry/amulet_of_glory.rs2",
            "\
[opheld4,amulet_of_glory_4] @amulet_of_glory_interface(\"Your amulet has three charges left.\");
[label,amulet_of_glory_interface](string $message)
def_obj $item = last_item;
switch_int($choice) {
    case 1 : ~player_teleport_normal(0_48_54_15_40);
    case 2 : ~player_teleport_normal(0_45_49_38_40);
    case 3 : ~player_teleport_normal(0_48_50_33_51);
    case 4 : ~player_teleport_normal(0_51_49_29_27);
}
",
        );
        fx.write(
            "scripts/general/scripts/enchanted_jewellry/ring_of_dueling.rs2",
            "\
[opheld4,_category_136]
mes(\"You rub the ring...\");
p_delay(1);
~player_teleport_normal(map_findsquare(0_51_50_51_35, 0, 2, ^map_findsquare_lineofwalk));
",
        );
        let defs = loc_defs(&[]);
        let wc = bake_collision(&fx, &defs, &HashSet::new());
        let graph = derive_transports(fx.path(), &defs, &wc);

        // Glory: the charged `_4` stage forwards to the interface label,
        // whose four cases are the four destinations; each carries the
        // charged item as its requirement.
        let glory: Vec<_> = graph
            .teleports
            .iter()
            .filter(|e| e.loc_id == 1712)
            .collect();
        assert_eq!(glory.len(), 4);
        let dests: HashSet<WorldTile> = glory.iter().map(|e| e.to).collect();
        assert_eq!(dests.len(), 4);
        for e in &glory {
            assert_eq!(e.item_req, vec![(1712, 1)]);
            assert_eq!(e.ticks, JEWELLERY_TELEPORT_TICKS);
            assert_eq!(e.option, 4); // Rub (opheld4)
            assert!(e.skill_req.is_empty());
        }
        assert!(glory.iter().any(|e| e.to
            == WorldTile {
                x: 3087,
                z: 3496,
                level: 0
            })); // Edgeville
        assert!(glory.iter().any(|e| e.to
            == WorldTile {
                x: 3293,
                z: 3163,
                level: 0
            })); // Al Kharid

        // Dueling: the `_category_136` script applies to every
        // `category=category_136` obj in enchanted_jewelry.obj.
        let duel = graph
            .teleports
            .iter()
            .find(|e| e.loc_id == 2552)
            .expect("ring of dueling teleport");
        assert_eq!(
            duel.to,
            WorldTile {
                x: 3315,
                z: 3235,
                level: 0
            }
        );
        assert_eq!(duel.item_req, vec![(2552, 1)]);
        assert_eq!(duel.ticks, JEWELLERY_TELEPORT_TICKS);

        // The placeholder `at` never enters the `at` index.
        assert!(!graph.at.contains_key(&TELEPORT_PLACEHOLDER_AT));
    }
}
