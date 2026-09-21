//! Content-derived transport graph: doors, ladders, stairs, agility
//! shortcuts, boats, gnome gliders, spirit trees, wilderness levers, the
//! Al Kharid toll / Shantay pass item gates, the keyed Edgeville brass-key
//! door, the Rune Mysteries
//! essence-mine wizard and Elkoy's Tree Gnome Village maze escort NPC
//! hops, and magic teleports as directed transport edges built from the
//! Server's own content — `scripts/{doors, ladders+stairs, interface_boat,
//! skill_magic, skill_agility}` and the Ardougne wilderness_lever pair,
//! `pack/loc.pack`, and the `maps/*.jm2` loc placements — instead of a
//! hand-authored table.
//!
//! The ladder/stairs parsing is a port of m8aq `api/nav/transports.ts`
//! (`resolvePlacements`: `p_telejump`/`p_teleport`/`~climb_ladder` +
//! `movecoord`/coordinate literals under `switch_coord`/`switch_int` guards);
//! agility shortcuts port `resolveShortcutPlacements`. Doors derive two
//! edges per jm2 placement — `dir` and its opposite — from the door
//! configs + the baked collision (`at` = the loc tile, `to` each
//! direction's adjacent standable tile, `open_loc_id` from the config's
//! `next_loc_stage`). Boats are
//! an explicit 2004 route table (dock NPC tile → destination ship deck,
//! then a loc-backed Cross on the boat-side `_gangplank_disembark`),
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
mod condparse;
use condparse::{
    arm_opens_directly, body_labels, check_axis_def, check_axis_or_proc, if_head_and_arm,
    proc_bitfield_varp, proc_bodies, script_blocks, script_header, script_varp_gate,
    top_level_statements,
};


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
    /// The Rune Essence mine exit portal (`blankrunestone_exit_portal`):
    /// never packed — the pack carries wizard → mine-pad entry edges
    /// only. The router synthesizes one per live [`crate::essence::EssenceSession`],
    /// returning to the entry wizard's overworld anchor.
    EssenceExit,
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
/// declares nothing). `worn_req` is the obj ids of which **any one** must
/// be equipped (a Dramen staff is a one-id list; slashable webs list
/// every blade whose `slashattack_anim` is not unarmed). `option` 0 on a
/// loc hop means use the first `item_req` obj on the loc (`oplocu`);
/// option 1 is `oploc1`.
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
    /// WORLD membership required (`MAP_MEMBERS`). False on every existing
    /// deriver; only the canonical `membergatel`/`membergater` family sets
    /// this. Packed as a `u8` on the v9 wire after `worn_req`.
    pub members_req: bool,
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
/// and the baked whole-world [`WorldCollision`] (door edges place their
/// `to` on an adjacent standable tile; door edges carry `dir` and
/// `open_loc_id`, every other kind keeps `dir: None`/`open_loc_id: None`).
///
/// Doors come from `scripts/doors/configs/*.loc` + the jm2 LOC placements;
/// ladders/stairs from `scripts/ladders+stairs/scripts/*.rs2`; agility
/// shortcuts from `scripts/skill_agility/scripts/*.rs2`. Placements and
/// destinations that resolve emit an edge — doors emit two per placement
/// (`dir` and its opposite, each with an adjacent standable destination); `at` the
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
    let (graph, skipped) = derive_transports_with_skips(content_root, loc_defs, collision);
    report(content_root, &graph, &skipped);
    graph
}

fn derive_transports_with_skips(
    content_root: &Path,
    loc_defs: &LocDefs,
    collision: &WorldCollision,
) -> (TransportGraph, HashMap<&'static str, usize>) {
    let mut graph = TransportGraph::default();
    let mut skipped: HashMap<&'static str, usize> = HashMap::new();

    let ids = loc_ids_by_name(content_root);
    let positions = loc_positions(content_root);

    door_edges(content_root, &ids, &mut graph, &mut skipped, collision);
    brass_key_door_edges(
        content_root,
        &ids,
        &positions,
        &mut graph,
        &mut skipped,
        collision,
    );
    membergate_edges(content_root, &ids, &mut graph, &mut skipped, collision);
    magicguild_door_edges(
        content_root,
        &ids,
        &positions,
        &mut graph,
        collision,
        &mut skipped,
    );
    rangingguild_door_edges(content_root, &ids, &positions, &mut graph, &mut skipped);
    ladder_stair_edges(
        content_root,
        &ids,
        &positions,
        loc_defs,
        &mut graph,
        &mut skipped,
    );
    trapdoor_edges(content_root, &ids, &positions, &mut graph, &mut skipped);
    shortcut_edges(
        content_root,
        &ids,
        &positions,
        loc_defs,
        &mut graph,
        &mut skipped,
    );
    web_edges(
        content_root,
        &ids,
        &positions,
        &mut graph,
        collision,
        &mut skipped,
    );
    boat_edges(&mut graph);
    cart_edges(&mut graph);
    essence_mine_edges(&mut graph);
    elkoy_edges(&mut graph);
    glider_edges(&mut graph);
    spirit_tree_edges(content_root, &ids, &positions, &mut graph, &mut skipped);
    lever_edges(content_root, &ids, &positions, &mut graph, &mut skipped);
    toll_edges(
        content_root,
        &ids,
        &positions,
        &mut graph,
        collision,
        &mut skipped,
    );
    zanaris_door_edges(content_root, &ids, &positions, &mut graph, &mut skipped);
    teleport_edges(content_root, &mut graph, &mut skipped);

    for (i, e) in graph.edges.iter().enumerate() {
        graph.at.entry(e.at).or_default().push(i);
    }

    (graph, skipped)
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
const SKIP_WEB_NO_FAR: &str = "slashable web has no standable far side";
const SKIP_FREE_ARM_GATE_CONFLICT: &str =
    "door reads as both a varp-gated door and a directional free arm (gate kept)";
const SKIP_GATE_MEMBER_OVERRIDE: &str =
    "closed gate member has a loc-specific open script (named override wins)";
const SKIP_GATE_MEMBER_HANDLER: &str = "closed gate category has no verified generic open handler";
const SKIP_GATE_MEMBER_SHAPE: &str = "closed gate member declares no Open op (unsupported shape)";
const SKIP_GATE_MEMBER_STAGE: &str =
    "closed gate member's next_loc_stage open leaf is unresolved or mismatched";
const SKIP_BRASS_KEY_SOURCE: &str =
    "brass-key door aliases, config, or canonical item-use handler did not match";
const SKIP_BRASS_KEY_SHAPE: &str = "brass-key door placement is not a level-0 straight wall";
const SKIP_BRASS_KEY_ENDPOINT: &str = "brass-key door source teleport endpoint is not standable";
const SKIP_GATE_MEMBER_PAIR: &str =
    "closed gate member has no adjacent paired open-stage placement";
const SKIP_GATE_MEMBER_CONFLICT: &str = "closed gate member is defined twice with different data";
const SKIP_MEMBERGATE_HANDLER: &str =
    "membergate named handler is missing, extra-guarded, or not the canonical opener";
const SKIP_MEMBERGATE_CONFLICT: &str = "membergate is defined twice with different data";
const SKIP_MEMBERGATE_STAGE: &str = "membergate open leaf is unresolved or mismatched";
const SKIP_MEMBERGATE_PAIR: &str = "membergate has no complementary paired placement";
const SKIP_MEMBERGATE_SHAPE: &str = "membergate declares no Open op or the wrong closed category";
const SKIP_LEVER_SOURCE: &str = "wilderness lever script or constants did not load";
const SKIP_LEVER_ROUTE: &str = "wilderness lever oploc1 block did not resolve to a hop";
const SKIP_TOLL_OBJ_PACK: &str = "toll or shantay item name missing from pack/obj.pack";
const SKIP_TOLL_CONFIG: &str = "Al Kharid border_gate.loc did not load or parse toll gates";
const SKIP_TOLL_HENGE: &str = "Shantay henge doorway did not resolve to hops";
const SKIP_MAGICGUILD_SCRIPT: &str = "magic guild opener script did not load or parse";
const SKIP_MAGICGUILD_CONFIG: &str = "magic guild door config did not admit named doors";
const SKIP_MAGICGUILD_DOOR: &str = "magic guild door placement did not derive crossings";
const SKIP_RANGINGGUILD_CONFIG: &str = "ranging guild ranging.loc did not load or admit Open";
const SKIP_RANGINGGUILD_PACK: &str = "ranging guild door pack id does not match 2514";
const SKIP_RANGINGGUILD_SCRIPT: &str = "ranging guild door opener did not parse";
const SKIP_RANGINGGUILD_PLACEMENT: &str =
    "ranging guild door placement is not the expected single diagonal";
const SKIP_ZANARIS_SOURCE: &str = "Zanaris shed door script did not load or declare oploc1";
const SKIP_ZANARIS_ROUTE: &str = "Zanaris shed door block did not resolve to a hop";

/// Declared wilderness lever hops (`wildinlever` / `wildoutlever`).
const LEVER_DECLARED_ROUTES: usize = 2;
/// Al Kharid border toll gate locs (`border_gate_toll_left` / `_right`).
const TOLL_BORDER_GATES: usize = 2;
/// One reciprocal enter/exit pair for ranging guild door 2514.
const RANGINGGUILD_DECLARED_PAIR: usize = 1;
/// One Zanaris shed `[oploc1,zanarisdoor]` hop (per level-0 wall placement).
const ZANARIS_DECLARED_ROUTES: usize = 1;
/// Named magic guild doors (`magicguild_door_l` / `_r`).
const MAGICGUILD_DECLARED_DOORS: usize = 2;

/// m8aq `types.ts` world box: every reachable 2004 tile. Destinations
/// outside it are skipped (m8aq's `idxOf` returns -1 there).
const LEVELS: i32 = 4;
const X0: i32 = 1856;
const X1: i32 = 3648;
const Z0: i32 = 1280;
const Z1: i32 = 10368;
/// `ladder_cellar`'s +6400/-6400 z shift (m8aq `CELLAR_SHIFT`).
pub(crate) const CELLAR_SHIFT: i32 = 6400;
/// Standard RS2 skill id (Server `PlayerStat`).
const SKILL_AGILITY: i32 = 16;
/// Standard RS2 skill id for Magic (Server `PlayerStat`).
const SKILL_MAGIC: i32 = 6;
/// Standard RS2 skill id for Ranged (Server `PlayerStat`).
const SKILL_RANGED: i32 = 4;
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
    // trapdoors.rs2: Open then Climb-down / p_telejump z±6400.
    ("trapdoor", 2),
    ("trapdoor_open", 2),
    ("trapdoor_level1", 2),
    ("trapdoor_open_level1", 2),
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
    // agility_dungeon.rs2: Walk-across the Yanille balancing ledge
    // (forcemove 8 tiles + p_delay).
    ("balancing_ledge3", 8),
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
pub(crate) struct Placement {
    pub(crate) id: i32,
    pub(crate) shape: i32,
    pub(crate) angle: i32,
    pub(crate) level: i32,
    pub(crate) x: i32,
    pub(crate) z: i32,
}

/// `pack/loc.pack` id→name lines → name → id (m8aq `locIdsByName`).
pub(crate) fn loc_ids_by_name(content_root: &Path) -> HashMap<String, i32> {
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
pub(crate) fn loc_positions(content_root: &Path) -> HashMap<i32, Vec<Placement>> {
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
/// bidirectional), `to` = each direction's adjacent tile when
/// [`WorldCollision::standable`] accepts it (otherwise no edge), `open_loc_id` = the
/// config's `param=next_loc_stage` open leaf. `option` 1 is the `Open` op;
/// each `to` is that crossing's arrival side, never a snap. Quest-gated
/// doors (`scripts/quests/*/configs/*.loc` and
/// `scripts/areas/*/configs/*.loc` named blocks) join the door set when
/// their `[oploc1,<name>]` open script declares a varp gate; the gate is
/// carried on every edge of the door (`varp_req`), never invented.
/// A door whose open script instead proves a directional free arm
/// ([`quest_door_free_arms`]) keeps that crossing with empty requirements.
/// The gated reverse is omitted unless [`completed_quest_reverse`] proves a
/// conservative completed-quest requirement for that loc (Death Plateau hut
/// doors only); a missing or incomplete quest fact still fails closed. The
/// v8 pack carries no masked bitfield, so raw varp 315 is never the gate.
/// Closed fence-gate members declared outside `gates.loc` (the quest/area
/// configs) join the door set only through [`inherited_closed_gates`] —
/// the closed gate categories whose effective handler is the verified
/// generic one.
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
    // Closed gate members declared outside `gates.loc` (the quest/area
    // configs) join the same set only while their effective handler is the
    // verified generic category open behavior.
    let positions = loc_positions(content_root);
    let supported_handlers = generic_gate_handlers(content_root);
    let inherited =
        inherited_closed_gates(content_root, ids, &positions, &supported_handlers, skipped);
    door_ids.extend(inherited.keys().copied());
    open_ids.extend(inherited.iter().map(|(&id, &open)| (id, open)));
    let constants = script_constants(content_root);
    let varps = varp_ids_by_name(content_root);
    let door_names = door_config_names(content_root, ids);
    let door_reqs = quest_door_reqs(content_root, &door_names, ids, &constants, &varps);
    let mut free_arms = quest_door_free_arms(content_root, &door_names, ids, &constants, &varps);
    // A door that also declares a readable direct-varp gate keeps the
    // existing two-edge gate: the two readings would disagree about the
    // same crossing, and the free arm is never a fallback for a gate.
    for id in door_reqs.keys() {
        if free_arms.remove(id).is_some() {
            bump(skipped, SKIP_FREE_ARM_GATE_CONFLICT, 1);
        }
    }
    door_ids.extend(door_reqs.keys().copied());
    door_ids.extend(free_arms.keys().copied());

    if door_ids.is_empty() {
        bump(skipped, SKIP_NO_DOOR_CONFIGS, 1);
        return;
    }
    for id in &door_ids {
        let Some(placements) = positions.get(id) else {
            continue;
        };
        let free_arm = free_arms.get(id);
        let varp_req = door_reqs.get(id).cloned().unwrap_or_default();
        for p in placements {
            // The collision bake (and its `standable`) is level 0 only.
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
            // A door is bidirectional: an edge in `dir` and one in its
            // opposite, each with an adjacent standable destination. A blocked
            // neighbor yields no edge; opening a door cannot erase scenery.
            let quest_reverse =
                free_arm.and_then(|arm| completed_quest_reverse(*id, arm, ids, &varps));
            for dir in [angle_dir, opposite(angle_dir)] {
                let is_free = free_arm.is_some_and(|arm| dir == arm.free_dir(angle_dir));
                let is_gated_reverse = free_arm.is_some() && !is_free;
                // A directional door keeps the crossing its open script
                // proves free. The gated reverse is emitted only with the
                // conservative completed-quest mapping; otherwise it stays
                // omitted rather than ungated.
                if is_gated_reverse && quest_reverse.is_none() {
                    continue;
                }
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
                    quest_req: if is_gated_reverse {
                        quest_reverse
                            .map(|q| vec![q.to_string()])
                            .unwrap_or_default()
                    } else {
                        vec![]
                    },
                    // The proven free crossing has no requirement; a door
                    // with a readable gate carries it on both crossings.
                    // The quest-gated reverse is not a varp gate.
                    varp_req: if free_arm.is_some() {
                        vec![]
                    } else {
                        varp_req.clone()
                    },
                    worn_req: vec![],
                    members_req: false,
                });
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Edgeville brass-key hut door.
// ---------------------------------------------------------------------------

const BRASS_KEY_DOOR_NAME: &str = "brasskeydoor";
const BRASS_KEY_NAME: &str = "edgevilledungeonkey";
const BRASS_KEY_OPEN_LABEL: &str = "open_edgeville_dungeon_door";
const BRASS_KEY_LOCKED_HANDLER: &str = r#"mes("The door is locked.");"#;
const BRASS_KEY_USE_HANDLER: &str = r#"
switch_obj(last_useitem) {
    case edgevilledungeonkey : @open_edgeville_dungeon_door;
    case default : ~displaymessage(^dm_default);
}
"#;
const BRASS_KEY_OPEN_HANDLER: &str = r#"
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
fn brass_key_door_edges(
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
            });
        }
    }
}

/// Resolve the brass-key door's configured replacement leaf. Duplicate
/// identical declarations are harmless; a conflicting declaration omits the
/// family.
fn brass_key_open_loc_id(
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
fn brass_key_handler_matches(content_root: &Path) -> bool {
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
fn selected_script_bodies(text: &str, selected_op: &str, selected_name: &str) -> Vec<String> {
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

// ---------------------------------------------------------------------------
// Canonical membergate family (membergatel / membergater).
// ---------------------------------------------------------------------------

const MEMBERGATE_LEFT: &str = "membergatel";
const MEMBERGATE_RIGHT: &str = "membergater";
const MEMBERGATE_LEFT_CLOSED: &str = "door_left_closed";
const MEMBERGATE_RIGHT_CLOSED: &str = "door_right_closed";
const MEMBERGATE_LEFT_OPENED: &str = "door_left_opened";
const MEMBERGATE_RIGHT_OPENED: &str = "door_right_opened";
const MEMBERGATE_LEFT_OPEN: &str =
    "if(map_members=^false){mes(^mes_members_gate);return;}~open_double_doors_left(500,door_right_closed,loc_param(open_sound));";
const MEMBERGATE_RIGHT_OPEN: &str =
    "if(map_members=^false){mes(^mes_members_gate);return;}~open_double_doors_right(500,door_left_closed,loc_param(open_sound));";

/// Closed membergate block as read from `doubledoors.loc`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MembergateDef {
    left: bool,
    op_open: bool,
    category_ok: bool,
    open: Option<i32>,
}

/// Open-leaf provenance for `loc_1560` / `loc_1561`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MembergateOpenLeaf {
    left: bool,
    op_close: bool,
    category_ok: bool,
}

/// Dedicated `membergatel`/`membergater` crossings. Not `parse_door_config`,
/// not fence-gate inheritance, not a generic double-door interpreter.
/// Both directions, `members_req`, `open_double_doors_*` morph.
fn membergate_edges(
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

    let mut placed: HashMap<(i32, i32, i32), Vec<(i32, i32, i32)>> = HashMap::new();
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
                });
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MembergateHandler {
    Ok,
    Invalid,
    Conflict,
}

/// Loc-specific `[oploc1,membergatel|membergater]` bodies under `scripts`.
/// Same-line and next-line forms. Duplicate identical canonical bodies are
/// fine; a second different body is a conflict. Extra guards / a different
/// opener fail closed.
fn membergate_handlers(content_root: &Path) -> HashMap<bool, MembergateHandler> {
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

fn membergate_oploc_handlers(text: &str) -> Vec<(bool, String)> {
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

fn membergate_loc_defs(
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
fn membergate_pair_tile(at: WorldTile, angle_dir: DoorDir, right: bool) -> WorldTile {
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

// ---------------------------------------------------------------------------
// Inherited closed fence gates (quest/area configs).
// ---------------------------------------------------------------------------

/// The two closed fence-gate categories and their generic category handlers
/// in `scripts/general_use/scripts/gates.rs2`:
/// `[oploc1,_gate_main_closed] ~open_gate;` (main) and
/// `[oploc1,_gate_outer_closed] ~open_outer_gate;` (outer). The engine
/// resolves a loc's open script as loc-specific `[oploc1,<name>]` >
/// category `[oploc1,<category>]` > global, so a member with no
/// loc-specific block inherits the category handler; `open_gate` opens the
/// main gate plus the adjacent paired outer (`get_pair_coord`), and
/// `open_outer_gate` resolves the adjacent main. Those two forwarding
/// bodies plus the two procs are the whole supported behavior.
const GATE_MAIN_CLOSED: &str = "gate_main_closed";
const GATE_OUTER_CLOSED: &str = "gate_outer_closed";
const GATE_MAIN_OPEN: &str = "gate_main_open";
const GATE_OUTER_OPEN: &str = "gate_outer_open";
const GATE_MAIN_HANDLER: &str = "~open_gate;";
const GATE_OUTER_HANDLER: &str = "~open_outer_gate;";
const GATE_MAIN_PROC: &str = "open_gate";
const GATE_OUTER_PROC: &str = "open_outer_gate";

/// A closed-gate `category=` value → the flavor (`false` main, `true`
/// outer). Every other category is `None`: an unknown category has no
/// verified handler and is never inherited.
fn closed_gate_category(value: &str) -> Option<bool> {
    match value.trim() {
        GATE_MAIN_CLOSED => Some(false),
        GATE_OUTER_CLOSED => Some(true),
        _ => None,
    }
}

/// The open leaf's category for a closed-gate flavor (`gate_main_open` /
/// `gate_outer_open`).
fn open_gate_category(outer: bool) -> &'static str {
    if outer {
        GATE_OUTER_OPEN
    } else {
        GATE_MAIN_OPEN
    }
}

/// A block body normalized for exact comparison: `//` comments dropped and
/// all whitespace removed.
fn normalized_body(body: &str) -> String {
    let mut out = String::new();
    for raw in body.lines() {
        let line = match raw.find("//") {
            Some(i) => &raw[..i],
            None => raw,
        };
        out.extend(line.chars().filter(|c| !c.is_whitespace()));
    }
    out
}

/// Every `.loc` config text under `scripts`, recursively.
fn visit_loc_configs(dir: &Path, cb: &mut impl FnMut(&str)) {
    let mut pending = vec![dir.to_path_buf()];
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
                    cb(&text);
                }
            }
        }
    }
}

/// The `[oploc1,_gate_main_closed]` / `[oploc1,_gate_outer_closed]`
/// category handlers in a script text → `(outer, body)`. `gates.rs2`
/// writes them as one line (`[oploc1,_gate_main_closed] ~open_gate;`), a
/// shape [`script_blocks`] cannot see (its header must be alone on the
/// line), so both the inline and the next-line body forms are read here.
/// Any other header closes the previous body.
fn gate_category_handlers(text: &str) -> Vec<(bool, String)> {
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
            let outer = match name.trim() {
                "_gate_main_closed" if op.trim() == "oploc1" => false,
                "_gate_outer_closed" if op.trim() == "oploc1" => true,
                _ => continue,
            };
            cur = Some((outer, body.to_string()));
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

/// Loc names targeted by `[oploc1,<name>]` in a script text, including
/// same-line bodies (`[oploc1,foo] ~open_gate;`) that [`script_blocks`]
/// cannot see.
fn oploc1_names(text: &str, into: &mut HashSet<String>) {
    for raw in text.lines() {
        let line = match raw.find("//") {
            Some(i) => &raw[..i],
            None => raw,
        };
        let line = line.trim();
        let Some(rest) = line.strip_prefix('[') else {
            continue;
        };
        let Some((header, _)) = rest.split_once(']') else {
            continue;
        };
        let Some((op, name)) = header.split_once(',') else {
            continue;
        };
        if op.trim() == "oploc1" {
            let name = name.trim();
            if !name.is_empty() {
                into.insert(name.to_string());
            }
        }
    }
}

/// The closed-gate categories this content actually supports, as the
/// verified generic open behavior: the `[oploc1,_gate_*_closed]` body must
/// be exactly the forwarding call (whitespace/comments ignored) and the
/// forwarded `[proc,…]` must be defined somewhere under `scripts`. A
/// category with no such block, a differently shaped body (any extra
/// statement proves nothing), a disagreeing duplicate, or a missing proc
/// is not supported — nothing inherits from it.
fn generic_gate_handlers(content_root: &Path) -> HashSet<bool> {
    let mut bodies: HashMap<bool, bool> = HashMap::new();
    let mut procs: HashSet<&'static str> = HashSet::new();
    visit_rs2(&content_root.join("scripts"), &mut |text| {
        for (outer, body) in gate_category_handlers(text) {
            let expected = if outer {
                GATE_OUTER_HANDLER
            } else {
                GATE_MAIN_HANDLER
            };
            let ok = normalized_body(&body) == expected;
            bodies
                .entry(outer)
                .and_modify(|prev| *prev &= ok)
                .or_insert(ok);
        }
        for proc in [GATE_MAIN_PROC, GATE_OUTER_PROC] {
            if !proc_bodies(text, proc).is_empty() {
                procs.insert(proc);
            }
        }
    });
    [false, true]
        .into_iter()
        .filter(|&outer| {
            let proc = if outer {
                GATE_OUTER_PROC
            } else {
                GATE_MAIN_PROC
            };
            bodies.get(&outer).copied().unwrap_or(false) && procs.contains(proc)
        })
        .collect()
}

/// One `.loc` block that declares a closed gate category, as read from the
/// quest/area config trees.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct InheritedGate {
    /// The block's closed category (`false` main, `true` outer); `None`
    /// when the block declares neither.
    category: Option<bool>,
    /// The block's `op1=Open` line.
    op_open: bool,
    /// The `next_loc_stage` open leaf id, resolved through `pack/loc.pack`.
    open: Option<i32>,
}

impl InheritedGate {
    fn new() -> Self {
        InheritedGate {
            category: None,
            op_open: false,
            open: None,
        }
    }
}

/// A `param=next_loc_stage,<value>` value → the open leaf id. Named values
/// and `loc_N` aliases resolve through [`loc_pack_id`]; an integer that is
/// not a pack id is refused.
fn stage_open_loc_id(value: &str, ids: &HashMap<String, i32>) -> Option<i32> {
    loc_pack_id(value, ids)
}

/// Named loc row or `loc_N` alias → pack id. `loc_N` is admitted only when
/// N is an id `pack/loc.pack` actually carries; an arbitrary integer is not
/// a loc. Named rows win when the pack lists that name.
fn loc_pack_id(name: &str, ids: &HashMap<String, i32>) -> Option<i32> {
    if let Some(&id) = ids.get(name) {
        return Some(id);
    }
    let n = name.strip_prefix("loc_")?;
    if n.is_empty() || !n.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let id: i32 = n.parse().ok()?;
    ids.values().copied().any(|v| v == id).then_some(id)
}

/// Every closed-gate declaration in one `.loc` text: `(id, gate)` per block
/// that names one of the two closed gate categories, with the block's
/// `op1=Open` line and its resolved `next_loc_stage` open leaf. Numeric
/// `[loc_N]` aliases resolve through [`loc_pack_id`]; every syntactically
/// valid `[header]` ends the previous block even when the name does not
/// resolve. Unresolved headers and blocks whose category is `gate_*_open`
/// (the open leaves) yield nothing.
fn closed_gate_blocks(text: &str, ids: &HashMap<String, i32>) -> Vec<(i32, InheritedGate)> {
    let mut out = Vec::new();
    let mut cur: Option<(i32, InheritedGate)> = None;
    for raw in text.lines() {
        let line = raw.trim();
        if let Some(name) = config_header(line) {
            if let Some(done) = cur.take() {
                out.push(done);
            }
            if let Some(id) = loc_pack_id(name, ids) {
                cur = Some((id, InheritedGate::new()));
            }
            continue;
        }
        let Some((_, gate)) = cur.as_mut() else {
            continue;
        };
        if line == "op1=Open" {
            gate.op_open = true;
        } else if let Some(value) = line.strip_prefix("category=") {
            if let Some(outer) = closed_gate_category(value) {
                gate.category = Some(outer);
            }
        } else if let Some(rest) = line.strip_prefix("param=") {
            if let Some((key, value)) = rest.split_once(',') {
                if key.trim() == "next_loc_stage" {
                    gate.open = stage_open_loc_id(value.trim(), ids);
                }
            }
        }
    }
    if let Some(done) = cur {
        out.push(done);
    }
    out
}

/// The tile of a closed gate's paired counterpart, from the
/// `[proc,get_pair_coord]` rule in `scripts/general_use/scripts/gates.rs2`:
/// the offset runs along the gate's wall (`west`→z+1, `north`→x+1, the
/// south/east cases mirrored) and `$outer` flips it. A gate member cannot
/// be read as the generic pair without it.
fn gate_pair_tile(at: WorldTile, angle_dir: DoorDir, outer: bool) -> WorldTile {
    let dir = if outer { -1 } else { 1 };
    match angle_dir {
        DoorDir::W => WorldTile {
            x: at.x,
            z: at.z + dir,
            level: at.level,
        },
        DoorDir::N => WorldTile {
            x: at.x + dir,
            z: at.z,
            level: at.level,
        },
        DoorDir::E => WorldTile {
            x: at.x,
            z: at.z - dir,
            level: at.level,
        },
        DoorDir::S => WorldTile {
            x: at.x - dir,
            z: at.z,
            level: at.level,
        },
    }
}

/// Closed fence-gate members declared outside
/// `scripts/general_use/configs/gates.loc` (the `scripts/quests` and
/// `scripts/areas` config trees), `loc id → next_loc_stage open leaf id`.
/// A member is admitted only while its whole generic inheritance is
/// provable from the same canonical data:
///
/// - the block declares one of the two closed gate categories with
///   `op1=Open` (both, not either: a member without the open op is a shape
///   this derivation does not support);
/// - that category's generic handler is verified in the content
///   ([`generic_gate_handlers`], handler body plus proc);
/// - a `next_loc_stage` open leaf resolves through loc.pack, and that
///   leaf's own config is present with one unconflicted matching
///   `gate_*_open` category;
/// - no loc-specific `[oploc1,<name>]` block exists anywhere under
///   `scripts` (the resolver's first priority — a named override, gated or
///   denied, is never promoted);
/// - every level-0 placement resolves its paired counterpart
///   ([`gate_pair_tile`]) to a placement of the complementary closed
///   category, so the pair the generic handler walks is really there;
/// - the member is not defined twice with disagreeing data.
///
/// Anything unresolved, unsupported or malformed is counted and left out,
/// never promoted to an ungated crossing.
fn inherited_closed_gates(
    content_root: &Path,
    ids: &HashMap<String, i32>,
    positions: &HashMap<i32, Vec<Placement>>,
    supported: &HashSet<bool>,
    skipped: &mut HashMap<&'static str, usize>,
) -> HashMap<i32, i32> {
    let mut defs: HashMap<i32, InheritedGate> = HashMap::new();
    let mut conflicted: HashSet<i32> = HashSet::new();
    let mut names: HashMap<i32, HashSet<String>> = HashMap::new();
    let scripts = content_root.join("scripts");
    let mut pending = vec![scripts.join("quests"), scripts.join("areas")];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for ent in entries.flatten() {
            let path = ent.path();
            if path.is_dir() {
                pending.push(path);
                continue;
            }
            if path.extension().and_then(|s| s.to_str()) != Some("loc") {
                continue;
            }
            let Ok(text) = fs::read_to_string(&path) else {
                continue;
            };
            for raw in text.lines() {
                let line = raw.trim();
                if let Some(name) = config_header(line) {
                    if let Some(id) = loc_pack_id(name, ids) {
                        names.entry(id).or_default().insert(name.to_string());
                    }
                }
            }
            for (id, gate) in closed_gate_blocks(&text, ids) {
                match defs.get(&id) {
                    Some(prev) if *prev != gate => {
                        defs.remove(&id);
                        conflicted.insert(id);
                    }
                    Some(_) => {}
                    None if conflicted.contains(&id) => {}
                    None => {
                        defs.insert(id, gate);
                    }
                }
            }
        }
    }
    // Every loc id any `.loc` under `scripts` names → its category, for the
    // open-leaf and pair checks (the leaf of a `gates.loc` member is
    // defined there, not in the member's own config).
    let mut categories: HashMap<i32, String> = HashMap::new();
    let mut category_conflicted: HashSet<i32> = HashSet::new();
    visit_loc_configs(&scripts, &mut |text| {
        let mut cur: Option<i32> = None;
        for raw in text.lines() {
            let line = raw.trim();
            if let Some(name) = config_header(line) {
                cur = loc_pack_id(name, ids);
                continue;
            }
            let Some(id) = cur else {
                continue;
            };
            if let Some(value) = line.strip_prefix("category=") {
                let value = value.trim();
                match categories.get(&id) {
                    Some(previous) if previous != value => {
                        categories.remove(&id);
                        category_conflicted.insert(id);
                    }
                    Some(_) => {}
                    None if category_conflicted.contains(&id) => {}
                    None => {
                        categories.insert(id, value.to_string());
                    }
                }
            }
        }
    });
    // Loc-specific `[oploc1,<name>]` blocks, the resolver's first priority.
    // Same-line bodies (`[oploc1,foo] ~open_gate;`) are included:
    // [`script_blocks`] cannot see a header that is not alone on the line.
    let mut overridden: HashSet<String> = HashSet::new();
    visit_rs2(&scripts, &mut |text| {
        oploc1_names(text, &mut overridden);
    });
    // Every placed loc id per tile, for the pair check.
    let mut placed: HashMap<(i32, i32, i32), Vec<i32>> = HashMap::new();
    for (&id, ps) in positions {
        for p in ps {
            placed.entry((p.x, p.z, p.level)).or_default().push(id);
        }
    }
    let mut out = HashMap::new();
    for (id, gate) in defs {
        let Some(outer) = gate.category else {
            continue;
        };
        if conflicted.contains(&id) {
            bump(skipped, SKIP_GATE_MEMBER_CONFLICT, 1);
            continue;
        }
        if !gate.op_open {
            bump(skipped, SKIP_GATE_MEMBER_SHAPE, 1);
            continue;
        }
        let Some(open) = gate.open else {
            bump(skipped, SKIP_GATE_MEMBER_STAGE, 1);
            continue;
        };
        if !supported.contains(&outer) {
            bump(skipped, SKIP_GATE_MEMBER_HANDLER, 1);
            continue;
        }
        if names
            .get(&id)
            .is_some_and(|ns| ns.iter().any(|n| overridden.contains(n)))
        {
            bump(skipped, SKIP_GATE_MEMBER_OVERRIDE, 1);
            continue;
        }
        if category_conflicted.contains(&open)
            || categories.get(&open).map(String::as_str) != Some(open_gate_category(outer))
        {
            bump(skipped, SKIP_GATE_MEMBER_STAGE, 1);
            continue;
        }
        let paired = positions.get(&id).is_none_or(|ps| {
            ps.iter().filter(|p| p.level == 0).all(|p| {
                let Some(angle_dir) = door_dir(p.angle) else {
                    return false;
                };
                let at = WorldTile {
                    x: p.x,
                    z: p.z,
                    level: p.level,
                };
                let pair = gate_pair_tile(at, angle_dir, outer);
                placed
                    .get(&(pair.x, pair.z, pair.level))
                    .is_some_and(|ids| {
                        ids.iter().any(|lid| {
                            !category_conflicted.contains(lid)
                                && categories.get(lid).and_then(|c| closed_gate_category(c))
                                    == Some(!outer)
                        })
                    })
            })
        });
        if !paired {
            bump(skipped, SKIP_GATE_MEMBER_PAIR, 1);
            continue;
        }
        out.insert(id, open);
    }
    out
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

/// Slashable webs (`bigweb_slashable` / loc 733): two edges per
/// far-side dir — `oplocu` with an unequippable knife (`option` 0,
/// `item_req`), and `oploc1` Slash (`option` 1) when any
/// `slashattack_anim` blade is worn (`worn_req`, any-of). `loc_change`
/// to `bigweb_slashed`. Same crossing shape as a door so the traveller
/// trolls the 50% slash fail. Wilderness placements pack too; [`crate::router::find`]
/// still refuses wildy tiles unless the search opts in.
const WEB_TICKS: i32 = 2;
/// `oplocu`: use the first `item_req` obj on the loc (the knife).
const WEB_USE_OPTION: i32 = 0;

fn web_edges(
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
                });
            }
        }
    }
}

/// Obj ids whose `.obj` block sets `slashattack_anim` to something other
/// than `human_unarmedpunch` — the same test `~slash_checker` uses on the
/// worn right hand. Unnamed pack rows are skipped.
fn slash_weapon_ids(content_root: &Path, objs: &HashMap<String, i32>) -> Vec<i32> {
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

/// A wall door can expose the adjacent tile, not erase intervening scenery.
fn door_far_side(at: WorldTile, dir: DoorDir, collision: &WorldCollision) -> Option<WorldTile> {
    let (dx, dz) = match dir {
        DoorDir::N => (0, 1),
        DoorDir::S => (0, -1),
        DoorDir::E => (1, 0),
        DoorDir::W => (-1, 0),
    };
    let to = WorldTile {
        x: at.x + dx,
        z: at.z + dz,
        level: at.level,
    };
    collision.standable(to).then_some(to)
}

// Web footprint traversal retains its existing behavior in this wall-door
// correction; webs are not shape-0 wall doors and need separate validation.
fn web_far_side(at: WorldTile, dir: DoorDir, collision: &WorldCollision) -> Option<WorldTile> {
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

/// A door whose `[oploc1,<name>]` block proves one crossing free: the block
/// opens on a `~check_axis` side test whose other disjunct is a
/// `~<proc> >= ^<const>` bitfield gate. The proven crossing is emitted with
/// empty requirements. The gated reverse is emitted only when
/// [`completed_quest_reverse`] can prove a conservative completed-quest
/// requirement for that loc; otherwise it stays out rather than open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FreeDoorArm {
    /// The `~check_axis(coord, loc_coord, loc_angle)` value the block opens
    /// on without any gate.
    free_when_check_axis: bool,
    /// Resolved `^const` minimum on the gated disjunct.
    gated_min: i32,
    /// Proven `getbit_range` varp id, lower bit, and upper bit. Completion
    /// is attached only when this identity is `%death_map` bits 0..3.
    bitfield_varp: i32,
    bitfield_lo: i32,
    bitfield_hi: i32,
}

impl FreeDoorArm {
    /// The crossing direction the free arm lands on, given the placement
    /// angle's own direction (the client's always-reached neighbour side —
    /// `test_wall` WALL_STRAIGHT EAST → destX+1, NORTH → destZ+1,
    /// WEST → destX−1, SOUTH → destZ−1). The native hop walks the player
    /// onto the door's own tile only for the crossing that starts on the
    /// far side and travels that way (`changeWallStraight`'s `canTravel`
    /// polarity lets the player step onto the loc along the angle, not
    /// against it), and that stand is the one `~check_axis` reads `true`
    /// for; the opposite crossing executes from the always-reached
    /// neighbour and reads `false`.
    fn free_dir(&self, angle_dir: DoorDir) -> DoorDir {
        if self.free_when_check_axis {
            angle_dir
        } else {
            opposite(angle_dir)
        }
    }
}

/// Door loc id → the directional free arm its `[oploc1,<name>]` block
/// proves ([`free_door_arm`]). A door whose blocks disagree about the arm
/// proves neither; a door whose block is any other shape stays out of this
/// map (and out of the pack entirely when no gate is readable either), so
/// an unread door is never promoted to ungated.
fn quest_door_free_arms(
    content_root: &Path,
    door_names: &HashSet<String>,
    ids: &HashMap<String, i32>,
    constants: &HashMap<String, i32>,
    varps: &HashMap<String, i32>,
) -> HashMap<i32, FreeDoorArm> {
    let mut out: HashMap<i32, FreeDoorArm> = HashMap::new();
    let mut conflicted: HashSet<i32> = HashSet::new();
    visit_rs2(&content_root.join("scripts"), &mut |text| {
        for (op, name, block) in script_blocks(text) {
            if op != "oploc1" || !door_names.contains(&name) {
                continue;
            }
            let Some(&id) = ids.get(&name) else {
                continue;
            };
            let Some(arm) = free_door_arm(&block, text, constants, varps) else {
                continue;
            };
            match out.get(&id) {
                Some(&prev) if prev != arm => {
                    out.remove(&id);
                    conflicted.insert(id);
                }
                Some(_) => {}
                None if conflicted.contains(&id) => {}
                None => {
                    out.insert(id, arm);
                }
            }
        }
    });
    out
}

/// The free arm a door's `[oploc1,<name>]` block proves, from exactly the
/// supported top-level sequence:
///
/// ```text
/// def_boolean $<b> = ~check_axis(coord, loc_coord, loc_angle);
/// if($<b> = <true|false> | ~<proc> >= ^<const>) { …opens the door directly… }
/// ```
///
/// The def must be the first top-level statement and the `if` the second;
/// extra statements before that pair (an outer guard, an earlier return, a
/// reassignment of `$<b>`) prove nothing. `$<b>` must be the boolean that
/// statement's own `check_axis` def introduces; the opening head must have
/// exactly two disjuncts — that boolean test and one `~<proc> >= ^<const>`
/// compare — with no nesting and no `&`; `<const>` must resolve in the
/// script constants; `<proc>` must be a `[proc,…]` block in the same script
/// text whose body is exactly
/// `return (getbit_range(%<varp>, ^<lo>, ^<hi>));` with the varp in
/// `pack/varp.pack` and both range constants resolving; and the arm must
/// open directly with the canonical `~open_and_close_door2(<loc>, $<b>,
/// door_open)` statement (no nested braces, no earlier return or quoted
/// text, no unrelated `~open_` name). Every other shape — a raw `%varp`
/// compare, a different comparator, a missing or differently-shaped proc,
/// an unresolved name, an arm that opens only inside a nested gate, an
/// `else` on the opening `if` — proves nothing at all, so the door keeps
/// no free arm and is never promoted to ungated. Knock and dialogue
/// branches after the opening `if` are never read.
fn free_door_arm(
    block: &str,
    script_text: &str,
    constants: &HashMap<String, i32>,
    varps: &HashMap<String, i32>,
) -> Option<FreeDoorArm> {
    let stmts = top_level_statements(block);
    let axis_bool = check_axis_def(stmts.first()?)?;
    let (head, arm) = if_head_and_arm(stmts.get(1)?)?;
    let (free_when_check_axis, proc, cname) = check_axis_or_proc(&head, &axis_bool)?;
    let gated_min = *constants.get(&cname)?;
    if !arm_opens_directly(&arm, &axis_bool) {
        return None;
    }
    let (bitfield_varp, bitfield_lo, bitfield_hi) =
        proc_bitfield_varp(script_text, &proc, constants, varps)?;
    Some(FreeDoorArm {
        free_when_check_axis,
        gated_min,
        bitfield_varp,
        bitfield_lo,
        bitfield_hi,
    })
}

/// Journal name of Death Plateau as the client stores a completed quest.
const DEATH_PLATEAU_QUEST: &str = "Death Plateau";
/// Front hut door (`death_sherpa_door`): gated min is `^death_spoken_tenzing`,
/// free when `$leaving = true`.
const DEATH_FRONT_DOOR: &str = "death_sherpa_door";
const DEATH_FRONT_GATED_MIN: i32 = 2;
const DEATH_FRONT_FREE_WHEN_CHECK_AXIS: bool = true;
/// Rear hut door (`death_sherpa_backdoor`): gated min is `^death_got_map`,
/// free when `$leaving = false`.
const DEATH_BACK_DOOR: &str = "death_sherpa_backdoor";
const DEATH_BACK_GATED_MIN: i32 = 7;
const DEATH_BACK_FREE_WHEN_CHECK_AXIS: bool = false;
/// `%death_map` bits `^death_map_lower..^death_map_upper` (0..3).
const DEATH_MAP_VARP: &str = "death_map";
const DEATH_MAP_LO: i32 = 0;
const DEATH_MAP_HI: i32 = 3;

/// Conservative completed-quest requirement for a directional door's gated
/// reverse, from an explicit source-backed mapping: only the two Death
/// Plateau hut doors whose proven free-arm shape still carries the current
/// thresholds, polarities, and `%death_map` bits 0..3 (front entry ≥ 2
/// leaving-true, garden exit ≥ 7 leaving-false). Completion is reachable
/// only after `death_get_map >= death_scouted_area` (8), so a completed
/// journal row implies both thresholds; in-progress map stages stay
/// unsupported. Loc ids resolve from the selected content's `loc.pack`.
/// An unrelated varp, a different bit range, the wrong polarity, or a
/// loc that is not the canonical sherpa door does not borrow completion.
/// This is not a general quest-implication engine and does not read varp
/// 315 as a raw gate.
fn completed_quest_reverse(
    id: i32,
    arm: &FreeDoorArm,
    ids: &HashMap<String, i32>,
    varps: &HashMap<String, i32>,
) -> Option<&'static str> {
    let &death_map = varps.get(DEATH_MAP_VARP)?;
    if arm.bitfield_varp != death_map
        || arm.bitfield_lo != DEATH_MAP_LO
        || arm.bitfield_hi != DEATH_MAP_HI
    {
        return None;
    }
    if ids.get(DEATH_FRONT_DOOR) == Some(&id)
        && arm.gated_min == DEATH_FRONT_GATED_MIN
        && arm.free_when_check_axis == DEATH_FRONT_FREE_WHEN_CHECK_AXIS
    {
        return Some(DEATH_PLATEAU_QUEST);
    }
    if ids.get(DEATH_BACK_DOOR) == Some(&id)
        && arm.gated_min == DEATH_BACK_GATED_MIN
        && arm.free_when_check_axis == DEATH_BACK_FREE_WHEN_CHECK_AXIS
    {
        return Some(DEATH_PLATEAU_QUEST);
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
fn trapdoor_edges(
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

/// Yanille dungeon ledge (`agility_dungeon.rs2` `balancing_ledge3`):
/// `coordz(loc_coord) > 9518` walks south to `0_40_148_20_40` (2580,9512);
/// else north to `0_40_148_20_48` (2580,9520). Dual placements, one
/// directed hop each. `at` is the loc tile (mid-ledge, blocked); take-off
/// is the standable start tile one step away (`INTERACT_RADIUS` 1).
fn balancing_ledge3_dests(loc: &Placement) -> Vec<WorldTile> {
    let to_z = if loc.z > 9518 { 9512 } else { 9520 };
    vec![WorldTile {
        level: loc.level,
        x: loc.x,
        z: to_z,
    }]
}

/// The `stat(agility) < N` level each `[oploc1,<name>]` block declares.
fn shortcut_agility_reqs(content_root: &Path) -> HashMap<String, i32> {
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

/// Destination-ship disembark: `oploc1` `Cross` on the `_gangplank_disembark`
/// loc (`gangplank.rs2`), landing on the dock. Live Port Sarim → Musa
/// stalls on the Musa deck if this hop is folded into the Boat edge —
/// `set_sail` only telejumps onto the ship.
#[derive(Debug, Clone, Copy)]
struct DisembarkPlank {
    loc_id: i32,
    at: WorldTile,
    to: WorldTile,
}

/// One 2004 boat journey: talk to the dock NPC at `at`, sail onto the
/// destination ship (`to` = the `set_sail` deck tile). `plank` is the
/// boat-side gangplank off that ship; Shanks `set_sail_cairn` lands on
/// the dock with `plank: None`. `at` is the NPC spawn, never the origin
/// gangplank (board planks refuse until the sailor is spoken to).
#[derive(Debug, Clone, Copy)]
struct BoatRoute {
    /// npc.pack id of the dock NPC who starts the journey.
    npc: i32,
    at: WorldTile,
    to: WorldTile,
    plank: Option<DisembarkPlank>,
    /// `set_sail` / `set_sail_cairn` `p_delay` only (plank ticks are on
    /// the disembark edge).
    ticks: i32,
    /// `(obj id, count)` fare the journey charges, if any.
    fare: Option<(i32, i32)>,
    /// `(varp id, min value)` quest gate, if any.
    varp_req: Option<(i32, i32)>,
}

/// The 2004 boat journeys: `~set_sail(` landings from area scripts, NPC
/// tiles from jm2, disembark locs from `gangplank.loc` / loc.pack (jm2
/// placements). Board planks (`*_on`) are not packed — they mes and
/// refuse until the sailor is spoken to.
const GANGPLANK_TICKS: i32 = 2;

const BOAT_ROUTES: &[BoatRoute] = &[
    // Seaman Thresnor (npc 378) on the Port Sarim pier (m47_50): lands on
    // the Karamja ship (2956,3143,1); `sarimshipplank_off` (loc 2082 at
    // 2956,3144,1, m46_49) Cross to the Karamja dock (2956,3146,0).
    BoatRoute {
        npc: 378,
        at: WorldTile {
            x: 3026,
            z: 3217,
            level: 0,
        },
        to: WorldTile {
            x: 2956,
            z: 3143,
            level: 1,
        },
        plank: Some(DisembarkPlank {
            loc_id: 2082,
            at: WorldTile {
                x: 2956,
                z: 3144,
                level: 1,
            },
            to: WorldTile {
                x: 2956,
                z: 3146,
                level: 0,
            },
        }),
        ticks: 7,
        fare: Some((995, 30)),
        varp_req: None,
    },
    // Customs officer (npc 380) at Musa Point (m46_49): lands on the Port
    // Sarim ship (3032,3217,1); `karamjashipplank_off` (loc 2084 at
    // 3031,3217,1, m47_50) to the Port Sarim dock (3029,3217,0).
    BoatRoute {
        npc: 380,
        at: WorldTile {
            x: 2955,
            z: 3146,
            level: 0,
        },
        to: WorldTile {
            x: 3032,
            z: 3217,
            level: 1,
        },
        plank: Some(DisembarkPlank {
            loc_id: 2084,
            at: WorldTile {
                x: 3031,
                z: 3217,
                level: 1,
            },
            to: WorldTile {
                x: 3029,
                z: 3217,
                level: 0,
            },
        }),
        ticks: 7,
        fare: Some((995, 30)),
        varp_req: None,
    },
    // Customs officer (npc 380) at the Brimhaven dock: lands on the
    // Ardougne ship (2683,3268,1); `brimhavenshipplank_off` (loc 2086 at
    // 2683,3269,1, m41_51) to the Ardougne dock (2683,3271,0).
    BoatRoute {
        npc: 380,
        at: WorldTile {
            x: 2772,
            z: 3231,
            level: 0,
        },
        to: WorldTile {
            x: 2683,
            z: 3268,
            level: 1,
        },
        plank: Some(DisembarkPlank {
            loc_id: 2086,
            at: WorldTile {
                x: 2683,
                z: 3269,
                level: 1,
            },
            to: WorldTile {
                x: 2683,
                z: 3271,
                level: 0,
            },
        }),
        ticks: 7,
        fare: Some((995, 30)),
        varp_req: None,
    },
    // Captain Barnaby (npc 381) at the Ardougne dock: lands on the
    // Brimhaven ship (2775,3234,1); `ardougneshipplank_off` (loc 2088 at
    // 2774,3234,1, m43_50) to the Brimhaven dock (2772,3234,0).
    BoatRoute {
        npc: 381,
        at: WorldTile {
            x: 2679,
            z: 3275,
            level: 0,
        },
        to: WorldTile {
            x: 2775,
            z: 3234,
            level: 1,
        },
        plank: Some(DisembarkPlank {
            loc_id: 2088,
            at: WorldTile {
                x: 2774,
                z: 3234,
                level: 1,
            },
            to: WorldTile {
                x: 2772,
                z: 3234,
                level: 0,
            },
        }),
        ticks: 7,
        fare: Some((995, 30)),
        varp_req: None,
    },
    // Monk of Entrana (shipmonk, npc 657): lands on the Entrana ship
    // (2834,3331,1); `ship_from_entrana_off` (loc 2415 at 2834,3333,1,
    // m44_52) to the Entrana dock (2834,3335,0). Delay 13.
    BoatRoute {
        npc: 657,
        at: WorldTile {
            x: 3049,
            z: 3235,
            level: 0,
        },
        to: WorldTile {
            x: 2834,
            z: 3331,
            level: 1,
        },
        plank: Some(DisembarkPlank {
            loc_id: 2415,
            at: WorldTile {
                x: 2834,
                z: 3333,
                level: 1,
            },
            to: WorldTile {
                x: 2834,
                z: 3335,
                level: 0,
            },
        }),
        ticks: 13,
        fare: None,
        varp_req: None,
    },
    // Monk of Entrana (shipmonk2, npc 658): lands on the Port Sarim ship
    // (3048,3231,1); `ship_to_entrana_off` (loc 2413 at 3048,3232,1,
    // m47_50) to the Port Sarim dock (3048,3234,0). Delay 14.
    BoatRoute {
        npc: 658,
        at: WorldTile {
            x: 2835,
            z: 3336,
            level: 0,
        },
        to: WorldTile {
            x: 3048,
            z: 3231,
            level: 1,
        },
        plank: Some(DisembarkPlank {
            loc_id: 2413,
            at: WorldTile {
                x: 3048,
                z: 3232,
                level: 1,
            },
            to: WorldTile {
                x: 3048,
                z: 3234,
                level: 0,
            },
        }),
        ticks: 14,
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
        plank: None,
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
        plank: None,
        ticks: 15,
        fare: None,
        varp_req: Some((116, 15)),
    },
];

/// Boat edges from the explicit 2004 route table: one `Talk-to` edge per
/// journey (NPC tile → `set_sail` deck), plus a loc-backed disembark
/// hop (`Cross` on the boat-side gangplank → dock). Kind is Ladder: a
/// level-changing loc op, not an NPC.
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
            members_req: false,
        });
        if let Some(p) = r.plank {
            graph.edges.push(TransportEdge {
                kind: TransportKind::Ladder,
                at: p.at,
                to: p.to,
                loc_id: p.loc_id,
                option: 1,
                ticks: GANGPLANK_TICKS,
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
            members_req: false,
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
/// teleport hop per wizard, landing on the mine pad. The return is not
/// packed — the mine exit portal's hop is synthesized per-slot from the
/// traveller's [`crate::essence::EssenceSession`], so the mine is never
/// a corridor between arbitrary overworld tiles. The gate is the quest
/// journal's row name ("Rune Mysteries Quest", green at
/// `%runemysteries >= ^runemysteries_complete`) — the same name
/// `WorldState::from_snapshot` reads from the quest tab; the
/// perm-scoped `%runemysteries` varp is never transmitted, so a
/// `varp_req` gate could never pass live.
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
            quest_req: vec!["Rune Mysteries Quest".to_string()],
            varp_req: vec![],
            worn_req: vec![],
            members_req: false,
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
            members_req: false,
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
/// `[opnpc1,gnomepilot]` block). Live `WorldState` may prove that as the
/// varp **or** as the green journal row (the kit waits `QuestDone`);
/// missing varps fail closed, so each flight packs both proofs.
const GLIDER_QUEST_REQ: (i32, i32) = (150, 160);
const GLIDER_QUEST_NAME: &str = "The Grand Tree";

/// Glider edges from the fixed platform table: the hub to every pad, and
/// back from the round-trip pads. `calc_glidervar` in `gnome_glider.rs2`
/// allows only hub↔pad flights (pad↔pad shows "You can't go there at the
/// moment."), and has no lemanto_andra → hub pair, so Lemanto Andra is
/// one-way. The flight is a `p_delay(3)` + teleport on top of the
/// `Talk-to` op.
fn glider_edges(graph: &mut TransportGraph) {
    for (pad, round_trip) in GLIDER_PADS {
        push_glider_flight(graph, GLIDER_HUB, *pad);
        if *round_trip {
            push_glider_flight(graph, *pad, GLIDER_HUB);
        }
    }
}

fn push_glider_flight(graph: &mut TransportGraph, at: WorldTile, to: WorldTile) {
    graph.edges.push(glider_edge(at, to, true));
    graph.edges.push(glider_edge(at, to, false));
}

fn glider_edge(at: WorldTile, to: WorldTile, varp_gate: bool) -> TransportEdge {
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
        quest_req: if varp_gate {
            vec![]
        } else {
            vec![GLIDER_QUEST_NAME.to_string()]
        },
        varp_req: if varp_gate {
            vec![GLIDER_QUEST_REQ]
        } else {
            vec![]
        },
        worn_req: vec![],
        members_req: false,
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
    skipped: &mut HashMap<&'static str, usize>,
) {
    let dir = content_root
        .join("scripts")
        .join("areas")
        .join("area_ardougne_east");
    let Ok(script) = fs::read_to_string(dir.join("scripts").join("wilderness_lever.rs2")) else {
        bump(skipped, SKIP_LEVER_SOURCE, LEVER_DECLARED_ROUTES);
        return;
    };
    let Ok(constants) = fs::read_to_string(dir.join("configs").join("wilderness_lever.constant"))
    else {
        bump(skipped, SKIP_LEVER_SOURCE, LEVER_DECLARED_ROUTES);
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
        let edge_start = graph.edges.len();
        let Some(&loc_id) = ids.get(&name) else {
            bump(skipped, SKIP_LEVER_ROUTE, 1);
            continue;
        };
        let Some(placements) = positions.get(&loc_id) else {
            bump(skipped, SKIP_LEVER_ROUTE, 1);
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
                    members_req: false,
                });
            }
        }
        if graph.edges.len() == edge_start {
            bump(skipped, SKIP_LEVER_ROUTE, 1);
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
/// The Shantay henge doorway loc id (`shantay_pass_henge_doorway` in
/// `pack/loc.pack`): the two loc-4031 Door edges both interact it, and
/// the traveller drives the gated branch's pass-handover chat dialogs
/// for this loc (see [`crate::traveller`]).
pub(crate) const SHANTAY_HENGE_LOC_ID: i32 = 4031;

/// The Shantay henge edge ticks: OP_BASE 1 + the `p_teleport` tick + the
/// `p_telejump` tick (both `p_delay(0)` in the queue block).
const SHANTAY_NORTH_TICKS: i32 = 3;

/// The Shantay henge free desert exit's `to`: the desert branch
/// (`coordz(coord) <= coordz(loc_coord)`) telejumps the player
/// `movecoord(coord,0,0,3)` — north of the loc — from wherever they
/// stand. From the desert-side stand directly south-east of the henge
/// ((3303,3115)) that landing is (3303,3118), an open tile north of the
/// gate; the hop's close-enough-2 arrive arm also covers the adjacent
/// desert-side approaches' landings ((3303,3117), (3302,3118)).
const SHANTAY_SOUTH_TO: WorldTile = WorldTile {
    x: 3303,
    z: 3118,
    level: 0,
};
/// The free desert exit ticks: OP_BASE 1 + the `p_telejump` tick (the
/// branch's own `p_delay(0)`).
const SHANTAY_SOUTH_TICKS: i32 = 2;

/// Al Kharid border-toll and Shantay-pass edges: `TransportKind::Door`
/// edges that cost an item, derived from `scripts/areas/area_alkharid/
/// configs/border_gate.loc` and `shantay_pass.rs2` plus the jm2
/// placements. The toll gates (`border_gate_toll_left`/`_right`, loc
/// 2882/2883) parse as doors under the same [`parse_door_config`] rule
/// (`op1=Open`) once their config's name-keyed blocks resolve through the
/// loc id map ([`parse_door_config_ids`]), and derive their two
/// crossings like every door: `at` the placement tile (m51_50 (4,27)/
/// (4,28) = (3268,3227)/(3268,3228)), `to` the adjacent standable tile,
/// `open_loc_id` the config's `next_loc_stage` leaf (loc 1562/1563),
/// `item_req` the 10-coin toll. The Shantay henge doorway (loc 4031,
/// `op1=Go-through`) derives two `TransportKind::Door` edges, one per
/// script branch — the gated hop (`at` the m51_48 (38,44) placement =
/// (3302,3116), `to` [`SHANTAY_NORTH_TO`], `item_req` one Shantay pass,
/// from the pass/Al Kharid side `coordz(coord) > coordz(loc_coord)`, the
/// `inv_del(inv, shantay_pass, 1)` + `[queue,shantay_pass_enter]`
/// teleport) and the free desert exit (`at` one tile south of the
/// placement, on the desert side `coordz(coord) <= coordz(loc_coord)`,
/// `to` [`SHANTAY_SOUTH_TO`], **no** `item_req` — the same block's
/// `p_telejump(movecoord(coord,0,0,3))`). The pack edge is **not** a
/// plain walk: both hops are op interactions with the loc, and the gated
/// hop is the only edge into the desert (a pass-side player routes
/// through it; the free edge's `at` sits on the desert side, so the
/// interaction never fires the pass branch).
fn toll_edges(
    content_root: &Path,
    ids: &HashMap<String, i32>,
    positions: &HashMap<i32, Vec<Placement>>,
    graph: &mut TransportGraph,
    collision: &WorldCollision,
    skipped: &mut HashMap<&'static str, usize>,
) {
    // The toll charge and the Shantay pass resolve by name through
    // `pack/obj.pack`; a missing pack skips the family instead of faking
    // an item id.
    let objs = obj_ids_by_name(content_root);
    let (Some(&coins_id), Some(&pass_id)) = (objs.get("coins"), objs.get("shantay_pass")) else {
        bump(
            skipped,
            SKIP_TOLL_OBJ_PACK,
            TOLL_BORDER_GATES + 1, // two toll gates + one henge doorway route
        );
        return;
    };

    let alkharid = content_root
        .join("scripts")
        .join("areas")
        .join("area_alkharid");
    let Ok(config) = fs::read_to_string(alkharid.join("configs").join("border_gate.loc")) else {
        bump(skipped, SKIP_TOLL_CONFIG, TOLL_BORDER_GATES);
        if ids.get("shantay_pass_henge_doorway").is_some() {
            bump(skipped, SKIP_TOLL_HENGE, 1);
        }
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
                    members_req: false,
                });
            }
        }
    }

    toll_shantay_henge_edges(ids, positions, graph, pass_id, skipped);
}

fn toll_shantay_henge_edges(
    ids: &HashMap<String, i32>,
    positions: &HashMap<i32, Vec<Placement>>,
    graph: &mut TransportGraph,
    pass_id: i32,
    skipped: &mut HashMap<&'static str, usize>,
) {
    // The Shantay henge: two edges, one per `[oploc1,shantay_pass_
    // henge_doorway]` branch — the gated hop `at` the loc's placement
    // tile (shape 10, unlike the wall doors), and the free desert exit
    // `at` one tile south of it (the desert side `coordz(coord) <=
    // coordz(loc_coord)`; the interaction from any tile on that side of
    // the gate always fires the free branch).
    let Some(&henge_id) = ids.get("shantay_pass_henge_doorway") else {
        return;
    };
    let Some(placements) = positions.get(&henge_id) else {
        bump(skipped, SKIP_TOLL_HENGE, 1);
        return;
    };
    let edge_start = graph.edges.len();
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
            members_req: false,
        });
        graph.edges.push(TransportEdge {
            kind: TransportKind::Door,
            at: WorldTile {
                x: p.x,
                z: p.z - 1,
                level: p.level,
            },
            to: SHANTAY_SOUTH_TO,
            loc_id: henge_id,
            option: 1, // Go-through (oploc1)
            ticks: SHANTAY_SOUTH_TICKS,
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
    if graph.edges.len() == edge_start {
        bump(skipped, SKIP_TOLL_HENGE, 1);
    }
}

// ---------------------------------------------------------------------------
// The Zanaris shed door (`quest_zanaris.rs2`): a worn-item teleport door.
// ---------------------------------------------------------------------------

/// Named Magic Guild doors (`magicguild_door_l` / `_r`). Not inherited
/// closed gates — `[oploc1,magicguild_door_*]` in `magic_guild.rs2` is a
/// loc-specific opener, so [`inherited_closed_gates`] refuses them.
/// `~check_axis_locactive` + `stat(magic) < N` gates only the entering
/// crossing (`door_open` / the loc's facing); the exit arm is ungated.
const MAGICGUILD_DOOR_LEFT: &str = "magicguild_door_l";
const MAGICGUILD_DOOR_RIGHT: &str = "magicguild_door_r";
const MAGICGUILD_OPEN_LABEL: &str = "open_mageguild_door";

fn magicguild_door_edges(
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
    if !script_path.is_file() {
        if ids.contains_key(MAGICGUILD_DOOR_LEFT) || ids.contains_key(MAGICGUILD_DOOR_RIGHT) {
            bump(skipped, SKIP_MAGICGUILD_SCRIPT, MAGICGUILD_DECLARED_DOORS);
        }
        return;
    }
    let Some(level) = magicguild_entering_magic_level(content_root) else {
        bump(skipped, SKIP_MAGICGUILD_SCRIPT, MAGICGUILD_DECLARED_DOORS);
        return;
    };
    let admitted = magicguild_door_open_ids(content_root, ids);
    if admitted.is_empty() {
        if ids.contains_key(MAGICGUILD_DOOR_LEFT) || ids.contains_key(MAGICGUILD_DOOR_RIGHT) {
            bump(skipped, SKIP_MAGICGUILD_CONFIG, MAGICGUILD_DECLARED_DOORS);
        }
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
fn magicguild_entering_magic_level(content_root: &Path) -> Option<i32> {
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

fn magicguild_oploc_jumps_to_opener(text: &str) -> bool {
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

fn magicguild_opener_jump(rest: &str) -> bool {
    let flat: String = rest.chars().filter(|c| !c.is_whitespace()).collect();
    flat.starts_with(&format!("@{MAGICGUILD_OPEN_LABEL}(")) && flat.ends_with(");")
}

/// Named Ranging Guild door (`ranging_guild_door`). Not an inherited
/// closed gate and not a Magic Guild `door_far_side` copy: the 289 opener
/// is a shape-9 diagonal wall that `~forcemove`s to a stand then
/// `p_teleport`s relative to that stand. `at` is the origin stand, `to`
/// the teleport dest; `dir` and `open_loc_id` stay `None` (the 3-tick
/// `loc_1532` add is visual, not a swing leaf).
const RANGINGGUILD_DOOR_NAME: &str = "ranging_guild_door";
const RANGINGGUILD_DOOR_ID: i32 = 2514;
const RANGINGGUILD_DOOR_SHAPE: i32 = 9; // LocShape::WALL_DIAGONAL
const RANGINGGUILD_EXIT_HALFPLANE: &str =
    "coordx(coord)>coordx(loc_coord)|coordz(coord)<coordz(loc_coord)";
const RANGINGGUILD_EXIT_FORCE: (i32, i32, i32) = (1, 0, -1);
const RANGINGGUILD_EXIT_TELE: (i32, i32, i32) = (-2, 0, 2);
const RANGINGGUILD_ENTER_FORCE: (i32, i32, i32) = (-1, 0, 1);
const RANGINGGUILD_ENTER_TELE: (i32, i32, i32) = (2, 0, -2);

enum RangingLocResolve {
    Resolved(i32),
    NotApplicable,
    Skip(&'static str),
}

fn rangingguild_door_edges(
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
        bump(skipped, SKIP_RANGINGGUILD_SCRIPT, RANGINGGUILD_DECLARED_PAIR);
        return;
    };
    let Some(placement) = rangingguild_unique_placement(positions, loc_id) else {
        bump(skipped, SKIP_RANGINGGUILD_PLACEMENT, RANGINGGUILD_DECLARED_PAIR);
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
        });
    }
}

fn rangingguild_resolve_loc_id(content_root: &Path, ids: &HashMap<String, i32>) -> RangingLocResolve {
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

fn rangingguild_unique_placement(
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

fn rangingguild_apply(tile: WorldTile, (dx, d_level, dz): (i32, i32, i32)) -> WorldTile {
    WorldTile {
        x: tile.x + dx,
        z: tile.z + dz,
        level: tile.level + d_level,
    }
}

fn rangingguild_parse_opener(
    content_root: &Path,
) -> Option<(
    (i32, i32, i32),
    (i32, i32, i32),
    (i32, i32, i32),
    (i32, i32, i32),
)> {
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
fn rangingguild_forcemove_before_teleport(flat: &str) -> bool {
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
fn rangingguild_first_return_after_teleport(flat: &str) -> bool {
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
fn rangingguild_enter_gate_before_crossing(flat: &str) -> bool {
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

fn rangingguild_strip_line_comments(text: &str) -> String {
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

fn rangingguild_flatten(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

fn rangingguild_split_leading_if(text: &str) -> Option<(String, String, String)> {
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

fn rangingguild_match(text: &str, open_idx: usize, open: char, close: char) -> Option<usize> {
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

fn rangingguild_unique_delta(arm: &str, name: &str, base: &str) -> Option<(i32, i32, i32)> {
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

fn rangingguild_movecoord_delta(expr: &str, base: &str) -> Option<(i32, i32, i32)> {
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

fn rangingguild_stat_ranged_lt(arm: &str) -> Option<i32> {
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

fn magicguild_door_open_ids(content_root: &Path, ids: &HashMap<String, i32>) -> HashMap<i32, i32> {
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

fn named_loc_has_open(text: &str, name: &str) -> bool {
    let mut in_block = false;
    let mut open = false;
    for raw in text.lines() {
        let line = raw.trim();
        if let Some(header) = config_header(line) {
            if in_block {
                return open;
            }
            in_block = header == name;
            open = false;
            continue;
        }
        if in_block && line == "op1=Open" {
            open = true;
        }
    }
    in_block && open
}

/// The Zanaris shed door ticks: OP_BASE 1, the door block's `p_delay(1)`,
/// and the `player_teleport_normal` cast `p_delay(2)` (the whole Open
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
    skipped: &mut HashMap<&'static str, usize>,
) {
    let Ok(script) = fs::read_to_string(
        content_root
            .join("scripts")
            .join("quests")
            .join("quest_zanaris")
            .join("scripts")
            .join("quest_zanaris.rs2"),
    ) else {
        if ids.contains_key("zanarisdoor") {
            bump(skipped, SKIP_ZANARIS_SOURCE, ZANARIS_DECLARED_ROUTES);
        }
        return;
    };
    let Some((_, name, body)) = script_blocks(&script)
        .into_iter()
        .find(|(op, name, _)| op.as_str() == "oploc1" && name.as_str() == "zanarisdoor")
    else {
        if ids.contains_key("zanarisdoor") {
            bump(skipped, SKIP_ZANARIS_SOURCE, ZANARIS_DECLARED_ROUTES);
        }
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
        bump(skipped, SKIP_ZANARIS_ROUTE, ZANARIS_DECLARED_ROUTES);
        return;
    };
    // The Dramen staff id (`pack/obj.pack`); a missing pack skips the
    // door instead of faking an id.
    let Some(&staff_id) = obj_ids_by_name(content_root).get("dramen_staff") else {
        bump(skipped, SKIP_ZANARIS_ROUTE, ZANARIS_DECLARED_ROUTES);
        return;
    };
    let Some(placements) = positions.get(&loc_id) else {
        bump(skipped, SKIP_ZANARIS_ROUTE, ZANARIS_DECLARED_ROUTES);
        return;
    };
    let edge_start = graph.edges.len();
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
            members_req: false,
        });
    }
    if graph.edges.len() == edge_start {
        bump(skipped, SKIP_ZANARIS_ROUTE, ZANARIS_DECLARED_ROUTES);
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
    type SpellTeleportBlock = (Option<i32>, Vec<(String, i32)>, Option<String>);
    let mut cur: Option<SpellTeleportBlock> = None;
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
        members_req: false,
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
                        members_req: false,
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
    if tail.is_empty() || (tail.starts_with('(') && tail.ends_with(')')) {
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
    } else {
        let r = rest.strip_prefix("int")?;
        (SwitchKind::Int, r)
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
    use std::collections::{HashMap, HashSet};
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
            .find(|e| e.kind == TransportKind::Door && e.worn_req == [772])
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

    /// `(at, dir, to)` of every door edge of `loc_id`, sorted, so a
    /// directional door's exact crossings can be asserted.
    fn door_crossings(graph: &TransportGraph, loc_id: i32) -> Vec<((i32, i32), char, (i32, i32))> {
        let mut out: Vec<_> = graph
            .edges
            .iter()
            .filter(|e| e.kind == TransportKind::Door && e.loc_id == loc_id)
            .map(|e| {
                let dir = match e.dir {
                    Some(DoorDir::N) => 'N',
                    Some(DoorDir::E) => 'E',
                    Some(DoorDir::S) => 'S',
                    Some(DoorDir::W) => 'W',
                    None => panic!("door edge without a dir: {e:?}"),
                };
                ((e.at.x, e.at.z), dir, (e.to.x, e.to.z))
            })
            .collect();
        out.sort();
        out
    }

    fn write_brass_key_door_source(fx: &Fixture, handler: &str) {
        fx.write("pack/loc.pack", "1535=loc_1535\n1804=brasskeydoor\n");
        fx.write("pack/obj.pack", "983=edgevilledungeonkey\n");
        fx.write(
            "scripts/_unpack/225/all.loc",
            "\
[loc_1535]
name=Door

[brasskeydoor]
name=Door
desc=This door requires a key.
model=basic_wall
active=yes
op1=Open
param=next_loc_stage,loc_1535
",
        );
        fx.write(
            "scripts/areas/area_edgeville/scripts/edgeville_dungeon.rs2",
            handler,
        );
        write_blocked_square(
            fx,
            48,
            53,
            &[(3115, 3448), (3115, 3449), (3115, 3450), (3115, 3451)],
            "0 43 58: 1804 0 3\n",
        );
    }

    fn canonical_brass_key_door_handler() -> &'static str {
        "\
[oploc1,brasskeydoor]
mes(\"The door is locked.\");

[oplocu,brasskeydoor]
switch_obj(last_useitem) {
    case edgevilledungeonkey : @open_edgeville_dungeon_door;
    case default : ~displaymessage(^dm_default);
}

[label,open_edgeville_dungeon_door]
if (inv_total(inv, edgevilledungeonkey) > 0) {
    mes(\"You unlock the door.\");
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
    mes(\"The door is locked.\");
}
"
    }

    #[test]
    fn derive_transports_emits_source_shaped_brass_key_door() {
        let fx = Fixture::new();
        write_brass_key_door_source(&fx, canonical_brass_key_door_handler());
        let defs = loc_defs(&[(1535, 1, 1), (1804, 1, 1)]);
        let wc = bake_collision(&fx, &defs, &HashSet::from([1804]));
        let graph = derive_transports(fx.path(), &defs, &wc);
        let edges: Vec<_> = graph
            .edges
            .iter()
            .filter(|edge| edge.loc_id == 1804)
            .collect();
        assert_eq!(
            edges.len(),
            2,
            "one keyed crossing per direction: {edges:?}"
        );
        assert_eq!(
            door_crossings(&graph, 1804),
            vec![
                ((3115, 3449), 'N', (3115, 3450)),
                ((3115, 3450), 'S', (3115, 3449)),
            ],
            "reverse starts at the temporary replacement leaf; it is not a generic adjacent-door edge"
        );
        for edge in edges {
            assert_eq!(edge.option, 0, "oplocu, never the locked oploc1");
            assert_eq!(edge.item_req, vec![(983, 1)]);
            assert_eq!(edge.open_loc_id, Some(1535));
            assert!(edge.skill_req.is_empty());
            assert!(edge.quest_req.is_empty());
            assert!(edge.varp_req.is_empty());
            assert!(edge.worn_req.is_empty());
            assert!(!edge.members_req);
            assert!(wc.standable(edge.at), "standable take-off: {edge:?}");
            assert!(wc.standable(edge.to), "standable landing: {edge:?}");
        }
    }

    #[test]
    fn brass_key_door_routes_both_ways_only_with_held_key() {
        use crate::router::{find_with, FindOptions, Leg, RouteError};

        let fx = Fixture::new();
        write_brass_key_door_source(&fx, canonical_brass_key_door_handler());
        let defs = loc_defs(&[(1535, 1, 1), (1804, 1, 1)]);
        let wc = bake_collision(&fx, &defs, &HashSet::from([1804]));
        let graph = derive_transports(fx.path(), &defs, &wc);
        let hut = WorldTile {
            x: 3115,
            z: 3451,
            level: 0,
        };
        let outside = WorldTile {
            x: 3115,
            z: 3448,
            level: 0,
        };
        let empty = crate::world_state::WorldState::empty();
        for (from, to) in [(hut, outside), (outside, hut)] {
            assert!(
                matches!(
                    find_with(&wc, &graph, from, to, FindOptions::default(), &empty),
                    Err(RouteError::NoPath)
                ),
                "the locked Open action must not become a free edge"
            );
        }

        let keyed = crate::world_state::WorldState {
            inv: HashMap::from([(983, 1)]),
            ..crate::world_state::WorldState::empty()
        };
        for (from, to, dir) in [(hut, outside, DoorDir::S), (outside, hut, DoorDir::N)] {
            let route = find_with(&wc, &graph, from, to, FindOptions::default(), &keyed)
                .expect("held brass key permits the short hut crossing");
            let hop = route
                .legs
                .iter()
                .find_map(|leg| match leg {
                    Leg::Transport { edge } if edge.loc_id == 1804 => Some(edge),
                    _ => None,
                })
                .expect("route uses the keyed brass-hut door");
            assert_eq!(hop.dir, Some(dir));
            assert_eq!(
                hop.option, 0,
                "traveller dispatches existing use-item-on-loc"
            );
            assert_eq!(hop.item_req, vec![(983, 1)]);
        }
        assert_eq!(
            keyed.inv.get(&983),
            Some(&1),
            "routing proves possession; the source never consumes the key"
        );
    }

    #[test]
    fn brass_key_door_fails_closed_when_alias_or_handler_changes() {
        let defs = loc_defs(&[(1535, 1, 1), (1804, 1, 1)]);
        for (label, handler) in [
            (
                "wrong use item",
                canonical_brass_key_door_handler()
                    .replace("case edgevilledungeonkey :", "case muddy_key :"),
            ),
            (
                "missing inventory guard",
                canonical_brass_key_door_handler()
                    .replace("inv_total(inv, edgevilledungeonkey) > 0", "true"),
            ),
            (
                "consumed key",
                canonical_brass_key_door_handler().replace(
                    "mes(\"You unlock the door.\");",
                    "inv_del(inv, edgevilledungeonkey, 1);",
                ),
            ),
        ] {
            let fx = Fixture::new();
            write_brass_key_door_source(&fx, &handler);
            let wc = bake_collision(&fx, &defs, &HashSet::from([1804]));
            let graph = derive_transports(fx.path(), &defs, &wc);
            assert!(
                graph.edges.iter().all(|edge| edge.loc_id != 1804),
                "{label} must omit the family"
            );
        }

        let fx = Fixture::new();
        write_brass_key_door_source(&fx, canonical_brass_key_door_handler());
        fx.write("pack/obj.pack", "");
        let wc = bake_collision(&fx, &defs, &HashSet::from([1804]));
        let graph = derive_transports(fx.path(), &defs, &wc);
        assert!(
            graph.edges.iter().all(|edge| edge.loc_id != 1804),
            "missing selected key alias must omit the family"
        );
    }

    #[test]
    fn selected_274_and_289_content_derives_the_keyed_hut_crossing() {
        use crate::router::{find_with, FindOptions, Leg};

        fn assert_selected(label: &str, graph: &TransportGraph, wc: &WorldCollision) {
            let edges: Vec<_> = graph
                .edges
                .iter()
                .filter(|edge| edge.loc_id == 1804)
                .collect();
            assert_eq!(edges.len(), 2, "{label}: {edges:?}");
            assert_eq!(
                door_crossings(graph, 1804),
                vec![
                    ((3115, 3449), 'N', (3115, 3450)),
                    ((3115, 3450), 'S', (3115, 3449)),
                ],
                "{label}"
            );
            assert!(edges.iter().all(|edge| {
                edge.option == 0
                    && edge.item_req == [(983, 1)]
                    && edge.open_loc_id == Some(1535)
                    && wc.standable(edge.at)
                    && wc.standable(edge.to)
            }));
            assert!(
                graph
                    .edges
                    .iter()
                    .all(|edge| edge.loc_id != 1804 || edge.option != 1),
                "{label}: locked Open must not become a free edge"
            );

            let inside = WorldTile {
                x: 3116,
                z: 3450,
                level: 0,
            };
            let outside = WorldTile {
                x: 3115,
                z: 3449,
                level: 0,
            };
            let unkeyed = find_with(
                wc,
                graph,
                inside,
                outside,
                FindOptions::default(),
                &crate::world_state::WorldState::empty(),
            );
            if let Ok(route) = &unkeyed {
                assert!(
                    route.legs.iter().all(|leg| !matches!(
                        leg,
                        Leg::Transport { edge } if edge.loc_id == 1804
                    )),
                    "{label}: no held key must reject the keyed crossing"
                );
            }
            let keyed = crate::world_state::WorldState {
                inv: HashMap::from([(983, 1)]),
                ..crate::world_state::WorldState::empty()
            };
            for (from, to, dir) in [(inside, outside, DoorDir::S), (outside, inside, DoorDir::N)] {
                let route = find_with(wc, graph, from, to, FindOptions::default(), &keyed)
                    .unwrap_or_else(|error| panic!("{label}: keyed short route: {error:?}"));
                assert!(
                    route.ticks < 10.0,
                    "{label}: keyed crossing must beat the long public-dungeon route: {route:?}"
                );
                assert!(route.legs.iter().any(|leg| matches!(
                    leg,
                    Leg::Transport { edge }
                        if edge.loc_id == 1804
                            && edge.option == 0
                            && edge.dir == Some(dir)
                )));
            }
            assert_eq!(
                keyed.inv.get(&983),
                Some(&1),
                "{label}: key is not consumed"
            );
        }

        if let Some((graph, wc)) = derive_from_real_content() {
            let root = real_content_root().expect("274 root already selected");
            let ids = loc_ids_by_name(&root);
            assert!(brass_key_handler_matches(&root), "274 handler shape");
            assert_eq!(
                brass_key_open_loc_id(&root, &ids, ids[BRASS_KEY_DOOR_NAME]),
                Some(1535),
                "274 replacement leaf"
            );
            assert_selected("274", &graph, &wc);
        }
        if let Some((graph, wc)) = derive_from_lostcity_content() {
            let root = PathBuf::from("/Users/acfrazier/experiments/lostcity-289/content");
            let ids = loc_ids_by_name(&root);
            assert!(brass_key_handler_matches(&root), "289 handler shape");
            assert_eq!(
                brass_key_open_loc_id(&root, &ids, ids[BRASS_KEY_DOOR_NAME]),
                Some(1535),
                "289 replacement leaf"
            );
            assert_selected("289", graph, wc);
        }
    }

    #[test]
    fn door_far_side_does_not_skip_bank_return_obstacles() {
        // Captured at (2651..=2657,3292): counter/plant/bench footprints
        // separate Door1530 at2656 from the old bogus west landing2651.
        let mut flags = vec![0u32; 7 * 4];
        flags[..7].copy_from_slice(&[0x4020, 0x4120, 0x4120, 0x4120, 0x4120, 0x5028, 0x10080]);
        let (walk, blocked) = crate::collision::pack_walk(&flags);
        let collision = WorldCollision {
            origin: WorldTile {
                x: 2651,
                z: 3292,
                level: 0,
            },
            width: 7,
            height: 1,
            walk,
            blocked,
            flags: Some(flags),
        };
        let at = WorldTile {
            x: 2656,
            z: 3292,
            level: 0,
        };
        assert_eq!(door_far_side(at, DoorDir::W, &collision), None);
        assert_eq!(
            door_far_side(at, DoorDir::E, &collision),
            Some(WorldTile {
                x: 2657,
                z: 3292,
                level: 0
            })
        );
        assert_eq!(door_far_side(at, DoorDir::N, &collision), None);
        assert_eq!(door_far_side(at, DoorDir::S, &collision), None);
    }

    #[test]
    fn web_far_side_preserves_multi_tile_footprint_crossing() {
        let mut flags = vec![0u32; 3 * 4];
        flags[1] = 0x100; // Adjacent scenery remains part of the web walk-out.
        let (walk, blocked) = crate::collision::pack_walk(&flags);
        let collision = WorldCollision {
            origin: WorldTile {
                x: 0,
                z: 0,
                level: 0,
            },
            width: 3,
            height: 1,
            walk,
            blocked,
            flags: Some(flags),
        };
        let at = WorldTile {
            x: 0,
            z: 0,
            level: 0,
        };
        assert_eq!(
            web_far_side(at, DoorDir::E, &collision),
            Some(WorldTile {
                x: 2,
                z: 0,
                level: 0
            })
        );
        assert_eq!(door_far_side(at, DoorDir::E, &collision), None);
        assert_eq!(web_far_side(at, DoorDir::W, &collision), None);
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
        // the south-bound adjacent destination accepts that tile — its W_S
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
        // own adjacent standable destination. `at` is the door loc tile.
        assert_eq!(doors.len(), 2);
        for edge in &doors {
            assert_eq!(edge.at.level, edge.to.level);
            assert_eq!(
                (edge.at.x - edge.to.x).abs() + (edge.at.z - edge.to.z).abs(),
                1
            );
        }
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
        // The south-bound destination is wall 980's own tile: its W_S
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
    /// Shantay-pass edges as item-gated `TransportKind::Door` edges.
    /// The toll gates (`border_gate_toll_left`/`_right`, loc 2882/2883 —
    /// the m51_50 (4,27)/(4,28) placements = (3268,3227)/(3268,3228))
    /// carry the 10-coin toll (`inv_del(inv, coins, 10)` in
    /// border_gate.rs2's `pass_toll_gate`); the Shantay henge doorway
    /// (loc 4031, m51_48 (38,44) = (3302,3116)) derives **two** edges,
    /// one per `shantay_pass.rs2` `[oploc1,...]` branch — the gated hop
    /// into the desert (`at` the placement, `to` (3304,3115), the
    /// landing of the `[queue,shantay_pass_enter]` `p_teleport
    /// (0_51_48_40_46)` + `p_telejump(movecoord(coord,0,0,-3))`,
    /// `item_req` one Shantay pass (obj 1854)) and the free desert exit
    /// (`at` (3302,3115) one tile south of the placement, `to`
    /// (3303,3118), the `coordz(coord) <= coordz(loc_coord)`
    /// `p_telejump(movecoord(coord,0,0,3))` landing, **no** `item_req`).
    /// Only the northbound hop carries the pass. Skips with a message
    /// when the Server content tree or the client cache is absent; never
    /// fakes coordinates.
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
        // The Shantay henge carries exactly two edges, one per
        // `[oploc1,shantay_pass_henge_doorway]` branch: the gated hop
        // into the desert and the free desert exit.
        let henge: Vec<_> = graph
            .edges
            .iter()
            .filter(|e| e.loc_id == 4031)
            .cloned()
            .collect();
        assert_eq!(
            henge.len(),
            2,
            "exactly two Shantay henge edges derive (the gated desert hop \
             and the free desert exit), got {}",
            henge.len()
        );
        let gated = henge
            .iter()
            .find(|e| !e.item_req.is_empty())
            .expect("one Shantay henge edge carries the pass");
        let free = henge
            .iter()
            .find(|e| e.item_req.is_empty())
            .expect("one Shantay henge edge is free");
        assert_eq!(
            gated.at,
            WorldTile {
                x: 3302,
                z: 3116,
                level: 0,
            }
        );
        assert_eq!(
            gated.to,
            WorldTile {
                x: 3304,
                z: 3115,
                level: 0,
            }
        );
        assert!(
            gated.item_req.iter().any(|(id, n)| *id == 1854 && *n >= 1),
            "Shantay pass on the gated desert hop"
        );
        assert_eq!(gated.option, 1, "Go-through op");
        assert_eq!(gated.dir, None);
        assert_eq!(
            free.at,
            WorldTile {
                x: 3302,
                z: 3115,
                level: 0,
            }
        );
        assert_eq!(
            free.to,
            WorldTile {
                x: 3303,
                z: 3118,
                level: 0,
            }
        );
        assert!(free.item_req.is_empty(), "the desert exit is free");
        assert_eq!(free.option, 1, "Go-through op");
        assert_eq!(free.dir, None);
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
                allow_bank_fetch: false,
                ..FindOptions::default()
            },
            &crate::world_state::WorldState::empty(),
        )
        .expect("allow_wilderness routes the Ardougne→wildy lever");
        assert_eq!(route.dest, to);
    }

    /// The gates seam: a route must now exist from Seers street
    /// (2725,3485,0) to the rock-crab shore (2710,3720,0) once the fence
    /// gates join the door set. Loads the baked process pack if present,
    /// else bakes from Server content. GitHub has neither — skip, do not
    /// panic. A `NoPath` with a pack is the honest two-component signal.
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
        let (collision, graph) = if let Some(world) = NavWorld::load_default_pack_or_skip() {
            (world.collision, world.graph)
        } else {
            let Some(root) = real_content_root() else {
                return;
            };
            let Some(defs) = real_loc_defs() else {
                return;
            };
            let wc = bake_from_maps(&root.join("maps"), &defs, &HashSet::new())
                .expect("real Server content bakes");
            let graph = derive_transports(&root, &defs, &wc);
            (wc, graph)
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
        // adjacent standable destination, `Open` op 1, one tick.
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
    fn derive_transports_emits_edgeville_trapdoor() {
        let fx = Fixture::new();
        fx.write("pack/loc.pack", "1568=trapdoor\n1570=trapdoor_open\n");
        fx.write(
            "maps/m48_54.jm2",
            "\
==== MAP ====
0 25 12: h1 o6 u50
0 24 12: h1 o6 u50
==== LOC ====
0 25 12: 1568 22 2
",
        );
        fx.write(
            "scripts/general_use/scripts/trapdoors.rs2",
            "\
[oploc1,trapdoor]
mes(\"The trapdoor opens...\");
loc_change(trapdoor_open, 500);

[oploc1,trapdoor_open]
mes(\"You climb down through the trapdoor...\");
p_telejump(movecoord(coord(), 0, 0, 6400));
",
        );
        let defs = loc_defs(&[(1568, 1, 1), (1570, 1, 1)]);
        let wc = bake_collision(&fx, &defs, &HashSet::new());
        let graph = derive_transports(fx.path(), &defs, &wc);
        let at = WorldTile {
            x: 3097,
            z: 3468,
            level: 0,
        };
        let edges: Vec<_> = graph
            .at
            .get(&at)
            .into_iter()
            .flatten()
            .map(|&i| &graph.edges[i])
            .collect();
        assert_eq!(edges.len(), 1, "got {edges:?}");
        let e = edges[0];
        assert_eq!(e.kind, TransportKind::Ladder);
        assert_eq!(e.loc_id, 1568);
        assert_eq!(e.open_loc_id, Some(1570));
        assert_eq!(e.option, 1);
        assert_eq!(
            e.to,
            WorldTile {
                x: 3097,
                z: 9868,
                level: 0
            }
        );
        assert_eq!(e.ticks, 3);
        assert!(!e.members_req);
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
        assert!(parse_statement("@unhandled_stairs(loc_coord);").is_none());
        assert!(matches!(
            parse_statement("@ladder_to_dwarf_remains;"),
            Some(Outcome::Skipped(SKIP_HANDOFF))
        ));
        assert!(matches!(
            parse_statement("def_int $option = ~p_choice2_header(\"Climb Up.\", 1, \"Climb Down.\", 2, \"Climb up or down the ladder?\");"),
            Some(Outcome::Skipped(SKIP_DIALOG))
        ));
        assert!(parse_statement("p_arrivedelay;").is_none());
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
        // The unknown ladder name resolves nothing; remaining edges are the
        // explicit 2004 boat/cart/wizard/glider tables plus boat-side
        // disembark planks (Ladder loc hops, not script-derived ladders).
        let explicit = graph
            .edges
            .iter()
            .filter(|e| {
                e.kind == TransportKind::Boat
                    || e.kind == TransportKind::Glider
                    || e.kind == TransportKind::Npc
                    || e.kind == TransportKind::Ladder
            })
            .count();
        assert_eq!(explicit, graph.edges.len());
        assert_eq!(
            graph
                .edges
                .iter()
                .filter(|e| e.kind == TransportKind::Ladder)
                .count(),
            6
        );
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
            14
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

        // Port Sarim → Musa: Talk-to lands on the Karamja ship deck;
        // `sarimshipplank_off` (loc 2082) is a separate loc hop off the boat.
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
                z: 3143,
                level: 1
            }
        );
        assert_eq!(ps_musa.option, 1); // Talk-to
        assert_eq!(ps_musa.ticks, 7); // set_sail delay only
        assert_eq!(ps_musa.item_req, vec![(995, 30)]); // 30-coin fare
        assert!(ps_musa.varp_req.is_empty());
        let musa_plank = graph
            .edges
            .iter()
            .find(|e| e.kind == TransportKind::Ladder && e.loc_id == 2082)
            .expect("sarimshipplank_off");
        assert_eq!(
            musa_plank.at,
            WorldTile {
                x: 2956,
                z: 3144,
                level: 1
            }
        );
        assert_eq!(
            musa_plank.to,
            WorldTile {
                x: 2956,
                z: 3146,
                level: 0
            }
        );
        assert_eq!(musa_plank.option, 1); // Cross
        assert_eq!(musa_plank.ticks, GANGPLANK_TICKS);

        // Musa → Port Sarim: deck landing, then karamjashipplank_off 2084.
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
                x: 3032,
                z: 3217,
                level: 1
            }
        );
        assert_eq!(musa_ps.ticks, 7);
        assert!(graph
            .edges
            .iter()
            .any(|e| e.kind == TransportKind::Ladder && e.loc_id == 2084));

        // Sail hops land on the ship; Shanks is the exception (direct dock).
        let interiors = [
            (2956, 3143, 1),
            (3032, 3217, 1),
            (2683, 3268, 1),
            (2775, 3234, 1),
            (2834, 3331, 1),
            (3048, 3231, 1),
        ];
        for b in boats.iter().filter(|b| b.loc_id != 518) {
            assert!(
                interiors.contains(&(b.to.x, b.to.z, b.to.level)),
                "sail hop should land on the ship deck, got {:?}",
                b.to
            );
        }

        // Shilo boats (Captain Shanks, npc 518) carry the Shilo Village gate
        // and land directly on the dock (`set_sail_cairn`, no plank).
        let shanks: Vec<_> = boats.iter().filter(|e| e.loc_id == 518).collect();
        assert_eq!(shanks.len(), 2);
        for s in &shanks {
            assert_eq!(s.varp_req, vec![(116, 15)]);
            assert_eq!(s.option, 1);
            assert_eq!(s.to.level, 0);
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
        assert_eq!(khazard.ticks, 9);
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

    /// Slashable webs pack two edges per crossing: knife `oplocu`
    /// (`option` 0, `item_req` knife — the knife is unequippable) and
    /// `oploc1` Slash (`option` 1, `worn_req` every slash-anim blade).
    /// Wilderness placements (z ≥ 3520) are included; `find` still gates wildy.
    #[test]
    fn derive_transports_emits_slashable_web_knife_edges() {
        let Some((graph, _)) = derive_from_real_content() else {
            return;
        };
        let webs: Vec<_> = graph
            .edges
            .iter()
            .filter(|e| e.kind == TransportKind::Door && e.loc_id == 733)
            .collect();
        assert!(
            !webs.is_empty(),
            "bigweb_slashable placements must pack (Yanille + wilderness)"
        );
        let knife: Vec<_> = webs.iter().filter(|w| w.option == 0).collect();
        let slash: Vec<_> = webs.iter().filter(|w| w.option == 1).collect();
        assert_eq!(
            knife.len(),
            slash.len(),
            "one knife use + one Slash per dir"
        );
        assert!(!knife.is_empty());
        for w in &knife {
            assert_eq!(w.item_req, vec![(946, 1)], "unequippable knife: {w:?}");
            assert!(w.worn_req.is_empty(), "{w:?}");
            assert_eq!(w.open_loc_id, Some(734), "{w:?}");
            assert_eq!(w.ticks, WEB_TICKS);
            assert!(w.dir.is_some());
        }
        for w in &slash {
            assert!(w.item_req.is_empty(), "Slash is worn-blade, not inv: {w:?}");
            assert!(
                w.worn_req.contains(&1277),
                "bronze_sword is a slash blade: {w:?}"
            );
            assert!(!w.worn_req.contains(&946), "knife is unequippable");
            assert_eq!(w.open_loc_id, Some(734), "{w:?}");
            assert_eq!(w.ticks, WEB_TICKS);
            assert!(w.dir.is_some());
        }
        assert!(
            webs.iter()
                .any(|e| e.at.z >= 3520 && e.at.x >= 2944 && e.at.x <= 3391),
            "wilderness webs pack too (surface band z≥3520)"
        );
        // Yanille dungeon mouth (m40_48).
        assert!(
            webs.iter()
                .any(|e| e.at.x >= 2560 && e.at.x < 2624 && e.at.z >= 3072 && e.at.z < 3136),
            "Yanille webs pack (m40_48)"
        );
    }

    /// The Yanille dungeon balancing ledge (`balancing_ledge3` / loc 2303)
    /// is the hop that connects the cellar landing to the chaos-druid
    /// warrior field. `agility_dungeon.rs2` `oploc1` Walk-across, Agility
    /// 40, start tiles `0_40_148_20_48` / `_20_40`.
    #[test]
    fn derive_transports_emits_yanille_balancing_ledge() {
        let Some((graph, _)) = derive_from_real_content() else {
            return;
        };
        let ledges: Vec<_> = graph
            .edges
            .iter()
            .filter(|e| e.kind == TransportKind::AgilityShortcut && e.loc_id == 2303)
            .collect();
        assert_eq!(
            ledges.len(),
            2,
            "balancing_ledge3 dual placements (N→S and S→N)"
        );
        for e in &ledges {
            assert_eq!(e.option, 1, "Walk-across: {e:?}");
            assert_eq!(e.skill_req, vec![(SKILL_AGILITY, 40)], "{e:?}");
            assert_eq!(e.at.x, 2580);
            assert_eq!(e.to.x, 2580);
            assert!(e.item_req.is_empty());
        }
        assert!(
            ledges.iter().any(|e| e.to.z == 9512),
            "N→S lands 0_40_148_20_40: {ledges:?}"
        );
        assert!(
            ledges.iter().any(|e| e.to.z == 9520),
            "S→N lands 0_40_148_20_48: {ledges:?}"
        );
    }

    /// Live step 27: Yanille bank → dungeon warriors. Knife (web) +
    /// Agility 40 (ledge). Empty WorldState stays NoPath.
    #[test]
    fn yanille_bank_reaches_dungeon_warriors_with_knife_and_agility() {
        let Some((graph, collision)) = derive_from_real_content() else {
            return;
        };
        let bank = WorldTile {
            x: 2612,
            z: 3092,
            level: 0,
        };
        let warriors = WorldTile {
            x: 2580,
            z: 9501,
            level: 0,
        };
        assert!(
            crate::router::find_with(
                &collision,
                &graph,
                bank,
                warriors,
                crate::router::FindOptions::default(),
                &crate::world_state::WorldState::empty(),
            )
            .is_err(),
            "empty WorldState cannot take the knife web or the Agility-40 ledge"
        );
        let mut state = crate::world_state::WorldState::empty();
        state.inv.insert(946, 1);
        state.stats.insert(SKILL_AGILITY, 40);
        let route = crate::router::find_with(
            &collision,
            &graph,
            bank,
            warriors,
            crate::router::FindOptions::default(),
            &state,
        )
        .expect("Yanille bank → dungeon warriors with knife + Agility 40");
        let hops: Vec<_> = route
            .legs
            .iter()
            .filter_map(|l| match l {
                crate::router::Leg::Transport { edge } => {
                    Some((edge.kind, edge.loc_id, edge.at, edge.to, edge.option))
                }
                crate::router::Leg::Walk { .. } => None,
            })
            .collect();
        assert!(
            hops.iter()
                .any(|(_, loc_id, _, _, option)| *loc_id == 733 && *option == 0),
            "knife in inv takes the oplocu hop: {hops:?}"
        );
        let mut worn = crate::world_state::WorldState::empty();
        worn.stats.insert(SKILL_AGILITY, 40);
        worn.worn.insert(1277);
        let worn_route = crate::router::find_with(
            &collision,
            &graph,
            bank,
            warriors,
            crate::router::FindOptions::default(),
            &worn,
        )
        .expect("Yanille bank → dungeon warriors with a worn bronze sword");
        assert!(
            worn_route.legs.iter().any(|l| match l {
                crate::router::Leg::Transport { edge } => {
                    edge.loc_id == 733 && edge.option == 1
                }
                _ => false,
            }),
            "worn slash blade takes oploc1 Slash"
        );
        let walked_web = route.legs.iter().any(|l| match l {
            crate::router::Leg::Walk { tiles } => tiles.iter().any(|t| {
                t.level == 0
                    && t.x >= 2568
                    && t.x <= 2578
                    && t.z >= 3120
                    && t.z <= 3128
                    && graph.edges.iter().any(|e| {
                        e.loc_id == 733 && e.at.x == t.x && e.at.z == t.z && e.at.level == 0
                    })
            }),
            _ => false,
        });
        assert!(
            !walked_web,
            "walk must not step onto the web loc tile: {hops:?}"
        );
    }

    /// The Yanille cellar stairs pack (in-town and outside-town mouths).
    #[test]
    fn yanille_cellar_stairs_pack_to_the_dungeon() {
        let Some((graph, _)) = derive_from_real_content() else {
            return;
        };
        assert!(
            graph.edges.iter().any(|e| {
                e.kind == TransportKind::Stairs
                    && e.at
                        == WorldTile {
                            x: 2569,
                            z: 3122,
                            level: 0,
                        }
                    && e.to.z >= 9472
            }),
            "outside-town cellar stairs 2569,3122 → dungeon"
        );
        assert!(
            graph.edges.iter().any(|e| {
                e.kind == TransportKind::Stairs
                    && e.at
                        == WorldTile {
                            x: 2603,
                            z: 3078,
                            level: 0,
                        }
                    && e.to.z >= 9472
            }),
            "in-town cellar stairs 2603,3078 → dungeon"
        );
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

    /// Tenzing's hut doors prove one free crossing each from the exact
    /// check-axis / proc-bitfield open shape: 3745's exit (dir E, free
    /// under `$leaving = true`) and 3746's entry from the north (dir S,
    /// free under `$leaving = false`). The gated reverse crossings carry
    /// completed `Death Plateau` (front entry W min 2, garden exit N min 7)
    /// and never a raw varp-315 gate. The direct-varp castle door keeps both
    /// crossings and its gate, and the crossing the free arm lands on follows
    /// the placement angle, not the door id.
    #[test]
    fn derive_transports_emits_tenzing_free_door_arms() {
        let fx = Fixture::new();
        fx.write(
            "pack/loc.pack",
            "3743=death_castledoor\n3745=death_sherpa_door\n3746=death_sherpa_backdoor\n",
        );
        fx.write("pack/varp.pack", "314=death_equiproom\n315=death_map\n");
        fx.write(
            "scripts/quests/quest_death/configs/quest_death.loc",
            "\
[death_sherpa_door]
name=Door
active=yes
op1=Open

[death_sherpa_backdoor]
name=Door
active=yes
op1=Open

[death_castledoor]
name=Door
active=yes
op1=Open
",
        );
        fx.write(
            "scripts/quests/quest_death/configs/quest_death.constant",
            "\
^death_spoken_saba = 1
^death_spoken_tenzing = 2
^death_got_map = 7
^death_unlocked_door = 70
^death_map_lower = 0
^death_map_upper = 3
",
        );
        // The three `[oploc1,…]` blocks and the `death_get_map` proc
        // verbatim from the 289 and 274 content roots.
        fx.write(
            "scripts/quests/quest_death/scripts/quest_death.rs2",
            "\
[oploc1,death_sherpa_door]
def_boolean $leaving = ~check_axis(coord, loc_coord, loc_angle);
if($leaving = true | ~death_get_map >= ^death_spoken_tenzing) {
    ~open_and_close_door2(loc_1532, $leaving, door_open); // open door loc isnt in this version?
    return;
}
sound_synth(knock_knock, 1, 0);
~mesbox(\"You knock on the door.\");
if(npc_find(coord, death_sherpa, 5, 0) = true) {
    ~chatnpc(\"<p,angry>No milk today! Thank you!\");
}

[oploc1,death_sherpa_backdoor]
def_boolean $leaving = ~check_axis(coord, loc_coord, loc_angle);
if($leaving = false | ~death_get_map >= ^death_got_map) {
    ~open_and_close_door2(loc_1532, $leaving, door_open); // open door loc isnt in this version?
    return;
}
if(npc_find(coord, death_sherpa, 5, 0) = true) {
    ~chatnpc(\"<p,angry>Where do you think you're going? This is private property!\");
}

[oploc1,death_castledoor]
if(%death_equiproom >= ^death_unlocked_door) {
    ~open_and_close_door2(castledoor_inactive, ~check_axis(coord, loc_coord, loc_angle), door_open); // nicedoor_open
    return;
}
mes(\"The door is locked.\");

[proc,death_get_map]()(int)
return (getbit_range(%death_map, ^death_map_lower, ^death_map_upper));
",
        );
        // The 289/274 placements: 3745 at (2822,3555) angle 2, 3746 at
        // (2820,3557) angle 1, plus a second 3745 placement at angle 0
        // (mirrored) and the castle door.
        fx.write(
            "maps/m44_55.jm2",
            "\
==== MAP ====
0 6 35: h98 f4 u64
0 4 37: h98 f4 u64

==== LOC ====
0 6 35: 3745 0 2
0 4 37: 3746 0 1
0 6 10: 3745 0 0
0 4 10: 3743 0 2
",
        );
        let defs = loc_defs(&[(3743, 1, 1), (3745, 1, 1), (3746, 1, 1)]);
        let wc = bake_collision(&fx, &defs, &HashSet::new());
        let graph = derive_transports(fx.path(), &defs, &wc);

        assert_eq!(
            door_crossings(&graph, 3745),
            vec![
                ((2822, 3530), 'E', (2823, 3530)),
                ((2822, 3530), 'W', (2821, 3530)),
                ((2822, 3555), 'E', (2823, 3555)),
                ((2822, 3555), 'W', (2821, 3555)),
            ],
            "3745 emits the free `$leaving = true` crossing along the \
             placement angle and the gated reverse"
        );
        assert_eq!(
            door_crossings(&graph, 3746),
            vec![
                ((2820, 3557), 'N', (2820, 3558)),
                ((2820, 3557), 'S', (2820, 3556)),
            ],
            "3746 emits the free `$leaving = false` crossing into the hut \
             and the gated garden exit"
        );
        let free_3745 = [((2822, 3530), DoorDir::W), ((2822, 3555), DoorDir::E)];
        let gated_3745 = [((2822, 3530), DoorDir::E), ((2822, 3555), DoorDir::W)];
        for e in graph
            .edges
            .iter()
            .filter(|e| e.kind == TransportKind::Door && matches!(e.loc_id, 3745 | 3746))
        {
            assert_eq!(e.option, 1, "Open op: {e:?}");
            assert!(
                e.varp_req.is_empty()
                    && e.item_req.is_empty()
                    && e.worn_req.is_empty()
                    && e.skill_req.is_empty(),
                "Tenzing hut doors never carry a raw varp/item/wear/skill gate: {e:?}"
            );
            let at = (e.at.x, e.at.z);
            let free = match e.loc_id {
                3745 => free_3745.contains(&(at, e.dir.unwrap())),
                3746 => e.dir == Some(DoorDir::S),
                _ => false,
            };
            let gated = match e.loc_id {
                3745 => gated_3745.contains(&(at, e.dir.unwrap())),
                3746 => e.dir == Some(DoorDir::N),
                _ => false,
            };
            if free {
                assert!(
                    e.quest_req.is_empty(),
                    "a proven free crossing carries no requirement: {e:?}"
                );
            } else if gated {
                assert_eq!(
                    e.quest_req,
                    vec!["Death Plateau".to_string()],
                    "gated reverse requires completed Death Plateau: {e:?}"
                );
            } else {
                panic!("unexpected Tenzing crossing: {e:?}");
            }
        }
        // High unrelated raw 315 bits do not satisfy the gated reverse:
        // the mapping is the completed journal row, not varp 315.
        let gated = graph
            .edges
            .iter()
            .find(|e| e.loc_id == 3745 && e.dir == Some(DoorDir::W) && e.at.z == 3555)
            .expect("3745 W front entry");
        let empty = crate::world_state::WorldState::empty();
        assert!(!empty.allows(gated), "absent quest rejects the gated entry");
        let high_bits = crate::world_state::WorldState {
            varps: HashMap::from([(315, i32::MAX)]),
            ..crate::world_state::WorldState::empty()
        };
        assert!(
            !high_bits.allows(gated),
            "high raw 315 bits do not bypass the completed-quest gate"
        );
        let incomplete = crate::world_state::WorldState {
            quests: HashSet::from(["Imp Catcher".to_string()]),
            varps: HashMap::from([(315, i32::MAX)]),
            ..crate::world_state::WorldState::empty()
        };
        assert!(
            !incomplete.allows(gated),
            "an unrelated completed quest does not open the gated entry"
        );
        let done = crate::world_state::WorldState {
            quests: HashSet::from(["Death Plateau".to_string()]),
            ..crate::world_state::WorldState::empty()
        };
        assert!(
            done.allows(gated),
            "completed Death Plateau allows the gated entry"
        );
        // The direct-varp castle door is untouched: both crossings, its gate.
        let castle: Vec<_> = graph
            .edges
            .iter()
            .filter(|e| e.kind == TransportKind::Door && e.loc_id == 3743)
            .collect();
        assert_eq!(castle.len(), 2);
        for e in &castle {
            assert_eq!(e.varp_req, vec![(314, 70)]);
            assert!(e.quest_req.is_empty());
            assert!(
                empty.allows(e) == false,
                "the castle door still fails closed without varp 314"
            );
            let with_varp = crate::world_state::WorldState {
                varps: HashMap::from([(314, 70)]),
                ..crate::world_state::WorldState::empty()
            };
            assert!(with_varp.allows(e), "direct-varp castle gate unchanged");
        }
    }

    /// Doors whose open script only *almost* matches the supported free-arm
    /// shape prove nothing and stay out of the pack — never ungated: a
    /// malformed proc body, an unreachable proc, a raw `%varp` in the OR
    /// head, an unresolved varp or constant, an arm that does not open, a
    /// `<` compare, and a dialogue-only block.
    #[test]
    fn derive_transports_omits_unproven_directional_door_arms() {
        let fx = Fixture::new();
        fx.write(
            "pack/loc.pack",
            "5001=bad_proc_body\n5002=no_proc\n5003=raw_varp_or\n5004=no_open_arm\n\
             5005=lt_compare\n5006=dialogue_only\n5007=unknown_varp\n5008=unknown_constant\n",
        );
        fx.write("pack/varp.pack", "315=death_map\n");
        fx.write(
            "scripts/quests/quest_death/configs/quest_death.loc",
            "\
[bad_proc_body]
op1=Open
[no_proc]
op1=Open
[raw_varp_or]
op1=Open
[no_open_arm]
op1=Open
[lt_compare]
op1=Open
[dialogue_only]
op1=Open
[unknown_varp]
op1=Open
[unknown_constant]
op1=Open
",
        );
        fx.write(
            "scripts/quests/quest_death/configs/quest_death.constant",
            "^death_spoken_tenzing = 2\n^death_map_lower = 0\n^death_map_upper = 3\n",
        );
        fx.write(
            "scripts/quests/quest_death/scripts/quest_death.rs2",
            "\
[oploc1,bad_proc_body]
def_boolean $leaving = ~check_axis(coord, loc_coord, loc_angle);
if($leaving = true | ~death_get_map >= ^death_spoken_tenzing) {
    ~open_and_close_door2(loc_1532, $leaving, door_open);
    return;
}

[oploc1,no_proc]
def_boolean $leaving = ~check_axis(coord, loc_coord, loc_angle);
if($leaving = true | ~death_missing_proc >= ^death_spoken_tenzing) {
    ~open_and_close_door2(loc_1532, $leaving, door_open);
    return;
}

[oploc1,raw_varp_or]
def_boolean $leaving = ~check_axis(coord, loc_coord, loc_angle);
if($leaving = true | %death_map >= ^death_spoken_tenzing) {
    ~open_and_close_door2(loc_1532, $leaving, door_open);
    return;
}

[oploc1,no_open_arm]
def_boolean $leaving = ~check_axis(coord, loc_coord, loc_angle);
if($leaving = true | ~death_get_map >= ^death_spoken_tenzing) {
    ~mesbox(\"The door is stuck.\");
    return;
}

[oploc1,lt_compare]
def_boolean $leaving = ~check_axis(coord, loc_coord, loc_angle);
if($leaving = true | ~death_get_map < ^death_spoken_tenzing) {
    ~open_and_close_door2(loc_1532, $leaving, door_open);
    return;
}

[oploc1,dialogue_only]
def_boolean $leaving = ~check_axis(coord, loc_coord, loc_angle);
if(npc_find(coord, death_sherpa, 5, 0) = true) {
    ~chatnpc(\"<p,angry>This is private property!\");
}

[oploc1,unknown_varp]
def_boolean $leaving = ~check_axis(coord, loc_coord, loc_angle);
if($leaving = true | ~death_get_unknown >= ^death_spoken_tenzing) {
    ~open_and_close_door2(loc_1532, $leaving, door_open);
    return;
}

[oploc1,unknown_constant]
def_boolean $leaving = ~check_axis(coord, loc_coord, loc_angle);
if($leaving = true | ~death_get_map >= ^death_unknown_stage) {
    ~open_and_close_door2(loc_1532, $leaving, door_open);
    return;
}

[proc,death_get_map]()(int)
return (getbit_range(%death_map, ^death_map_lower, ^death_map_upper) + 1);

[proc,death_get_unknown]()(int)
return (getbit_range(%death_unknown_varp, ^death_map_lower, ^death_map_upper));
",
        );
        fx.write(
            "maps/m44_55.jm2",
            "\
==== MAP ====
0 6 35: h98 f4 u64

==== LOC ====
0 6 35: 5001 0 2
0 6 36: 5002 0 2
0 6 37: 5003 0 2
0 6 38: 5004 0 2
0 6 39: 5005 0 2
0 6 40: 5006 0 2
0 6 41: 5007 0 2
0 6 42: 5008 0 2
",
        );
        let sizes = [(5001, 1, 1), (5002, 1, 1), (5003, 1, 1), (5004, 1, 1)]
            .into_iter()
            .chain([(5005, 1, 1), (5006, 1, 1), (5007, 1, 1), (5008, 1, 1)])
            .collect::<Vec<_>>();
        let defs = loc_defs(&sizes);
        let wc = bake_collision(&fx, &defs, &HashSet::new());
        let graph = derive_transports(fx.path(), &defs, &wc);

        for id in 5001..=5008 {
            assert!(
                door_crossings(&graph, id).is_empty(),
                "loc {id} proves no free arm and must not be packed"
            );
            assert!(
                graph.edges.iter().all(|e| e.loc_id != id),
                "loc {id} must not be reachable through any other edge kind"
            );
        }
    }

    /// Extra conditions on an otherwise-valid canonical block/proc must not
    /// prove a free arm: a wrapping outer `if`, an earlier conditional
    /// return, a reassigned check-axis boolean, and an open call nested
    /// inside the opening arm. The control loc uses the same valid
    /// `death_get_map` proc as Tenzing and must still emit its free crossing,
    /// so each negative is a refusal of extra conditions, not a dead pipeline.
    #[test]
    fn derive_transports_omits_extra_condition_directional_door_arms() {
        let fx = Fixture::new();
        fx.write(
            "pack/loc.pack",
            "5010=good_door\n5011=nested_outer\n5012=earlier_return\n\
             5013=reassigned_axis\n5014=nested_open\n",
        );
        fx.write("pack/varp.pack", "315=death_map\n");
        fx.write(
            "scripts/quests/quest_death/configs/quest_death.loc",
            "\
[good_door]
op1=Open
[nested_outer]
op1=Open
[earlier_return]
op1=Open
[reassigned_axis]
op1=Open
[nested_open]
op1=Open
",
        );
        fx.write(
            "scripts/quests/quest_death/configs/quest_death.constant",
            "^death_spoken_tenzing = 2\n^death_map_lower = 0\n^death_map_upper = 3\n",
        );
        fx.write(
            "scripts/quests/quest_death/scripts/quest_death.rs2",
            "\
[oploc1,good_door]
def_boolean $leaving = ~check_axis(coord, loc_coord, loc_angle);
if($leaving = true | ~death_get_map >= ^death_spoken_tenzing) {
    ~open_and_close_door2(loc_1532, $leaving, door_open);
    return;
}

[oploc1,nested_outer]
if(inv_total(inv, coins) > 0) {
    def_boolean $leaving = ~check_axis(coord, loc_coord, loc_angle);
    if($leaving = true | ~death_get_map >= ^death_spoken_tenzing) {
        ~open_and_close_door2(loc_1532, $leaving, door_open);
        return;
    }
}

[oploc1,earlier_return]
if(inv_total(inv, coins) = 0) {
    return;
}
def_boolean $leaving = ~check_axis(coord, loc_coord, loc_angle);
if($leaving = true | ~death_get_map >= ^death_spoken_tenzing) {
    ~open_and_close_door2(loc_1532, $leaving, door_open);
    return;
}

[oploc1,reassigned_axis]
def_boolean $leaving = ~check_axis(coord, loc_coord, loc_angle);
$leaving = false;
if($leaving = true | ~death_get_map >= ^death_spoken_tenzing) {
    ~open_and_close_door2(loc_1532, $leaving, door_open);
    return;
}

[oploc1,nested_open]
def_boolean $leaving = ~check_axis(coord, loc_coord, loc_angle);
if($leaving = true | ~death_get_map >= ^death_spoken_tenzing) {
    if(inv_total(inv, coins) > 0) {
        ~open_and_close_door2(loc_1532, $leaving, door_open);
        return;
    }
}

[proc,death_get_map]()(int)
return (getbit_range(%death_map, ^death_map_lower, ^death_map_upper));
",
        );
        fx.write(
            "maps/m44_55.jm2",
            "\
==== MAP ====
0 6 35: h98 f4 u64

==== LOC ====
0 6 35: 5010 0 2
0 6 36: 5011 0 2
0 6 37: 5012 0 2
0 6 38: 5013 0 2
0 6 39: 5014 0 2
",
        );
        let defs = loc_defs(&[
            (5010, 1, 1),
            (5011, 1, 1),
            (5012, 1, 1),
            (5013, 1, 1),
            (5014, 1, 1),
        ]);
        let wc = bake_collision(&fx, &defs, &HashSet::new());
        let graph = derive_transports(fx.path(), &defs, &wc);

        assert_eq!(
            door_crossings(&graph, 5010),
            vec![((2822, 3555), 'E', (2823, 3555))],
            "the valid control must still prove its free arm from the same proc"
        );
        for (id, why) in [
            (
                5011,
                "a wrapping outer if must not prove the inner free arm",
            ),
            (
                5012,
                "an earlier conditional return must not prove a later free arm",
            ),
            (
                5013,
                "a reassigned check-axis boolean must not prove a free arm",
            ),
            (
                5014,
                "an open nested inside the opening arm must not prove a free arm",
            ),
        ] {
            assert!(
                door_crossings(&graph, id).is_empty(),
                "loc {id} must not be packed: {why}"
            );
        }
    }

    /// Quoted text, an unrelated `~open_` name, `~open_overlay`, and an
    /// open after `return` must not prove a free arm. The control loc uses
    /// the canonical `~open_and_close_door2(loc_1532, $leaving, door_open)`
    /// sequence and must still emit.
    #[test]
    fn derive_transports_omits_noncanonical_directional_door_openers() {
        let fx = Fixture::new();
        fx.write(
            "pack/loc.pack",
            "5020=good_open\n5021=quoted_open\n5022=open_overlay\n\
             5023=unrelated_open\n5024=open_after_return\n",
        );
        fx.write("pack/varp.pack", "315=death_map\n");
        fx.write(
            "scripts/quests/quest_death/configs/quest_death.loc",
            "\
[good_open]
op1=Open
[quoted_open]
op1=Open
[open_overlay]
op1=Open
[unrelated_open]
op1=Open
[open_after_return]
op1=Open
",
        );
        fx.write(
            "scripts/quests/quest_death/configs/quest_death.constant",
            "^death_spoken_tenzing = 2\n^death_map_lower = 0\n^death_map_upper = 3\n",
        );
        fx.write(
            "scripts/quests/quest_death/scripts/quest_death.rs2",
            "\
[oploc1,good_open]
def_boolean $leaving = ~check_axis(coord, loc_coord, loc_angle);
if($leaving = true | ~death_get_map >= ^death_spoken_tenzing) {
    ~open_and_close_door2(loc_1532, $leaving, door_open);
    return;
}

[oploc1,quoted_open]
def_boolean $leaving = ~check_axis(coord, loc_coord, loc_angle);
if($leaving = true | ~death_get_map >= ^death_spoken_tenzing) {
    mes(\\\"try ~open_and_close_door2(loc_1532, $leaving, door_open)\\\");
    return;
}

[oploc1,open_overlay]
def_boolean $leaving = ~check_axis(coord, loc_coord, loc_angle);
if($leaving = true | ~death_get_map >= ^death_spoken_tenzing) {
    ~open_overlay(overlay_door);
    return;
}

[oploc1,unrelated_open]
def_boolean $leaving = ~check_axis(coord, loc_coord, loc_angle);
if($leaving = true | ~death_get_map >= ^death_spoken_tenzing) {
    ~open_gate();
    return;
}

[oploc1,open_after_return]
def_boolean $leaving = ~check_axis(coord, loc_coord, loc_angle);
if($leaving = true | ~death_get_map >= ^death_spoken_tenzing) {
    return;
    ~open_and_close_door2(loc_1532, $leaving, door_open);
}

[proc,death_get_map]()(int)
return (getbit_range(%death_map, ^death_map_lower, ^death_map_upper));
",
        );
        fx.write(
            "maps/m44_55.jm2",
            "\
==== MAP ====
0 6 35: h98 f4 u64

==== LOC ====
0 6 35: 5020 0 2
0 6 36: 5021 0 2
0 6 37: 5022 0 2
0 6 38: 5023 0 2
0 6 39: 5024 0 2
",
        );
        let defs = loc_defs(&[
            (5020, 1, 1),
            (5021, 1, 1),
            (5022, 1, 1),
            (5023, 1, 1),
            (5024, 1, 1),
        ]);
        let wc = bake_collision(&fx, &defs, &HashSet::new());
        let graph = derive_transports(fx.path(), &defs, &wc);

        assert_eq!(
            door_crossings(&graph, 5020),
            vec![((2822, 3555), 'E', (2823, 3555))],
            "the valid control must still prove its canonical opener"
        );
        for (id, why) in [
            (5021, "a quoted open call must not prove a free arm"),
            (5022, "~open_overlay must not prove a free arm"),
            (5023, "an unrelated ~open_ name must not prove a free arm"),
            (5024, "an open after return must not prove a free arm"),
        ] {
            assert!(
                door_crossings(&graph, id).is_empty(),
                "loc {id} must not be packed: {why}"
            );
        }
    }

    /// Completed Death Plateau attaches only to the canonical sherpa loc
    /// with `%death_map` bits 0..3 and the source polarity/threshold. An
    /// unrelated varp, a different bit range, or the wrong polarity still
    /// proves a free arm when the bitfield shape is valid, but must not
    /// borrow the reverse. A duplicated proc proves nothing.
    #[test]
    fn derive_transports_omits_borrowed_death_plateau_reverse() {
        fn graph_for(
            script: &str,
            loc_pack: &str,
            varp_pack: &str,
            constants: &str,
            loc_line: &str,
        ) -> TransportGraph {
            let fx = Fixture::new();
            fx.write("pack/loc.pack", loc_pack);
            fx.write("pack/varp.pack", varp_pack);
            fx.write(
                "scripts/quests/quest_death/configs/quest_death.loc",
                "\
[death_sherpa_door]
op1=Open
[death_sherpa_backdoor]
op1=Open
[good_door]
op1=Open
",
            );
            fx.write(
                "scripts/quests/quest_death/configs/quest_death.constant",
                constants,
            );
            fx.write("scripts/quests/quest_death/scripts/quest_death.rs2", script);
            fx.write(
                "maps/m44_55.jm2",
                &format!(
                    "\
==== MAP ====
0 6 35: h98 f4 u64

==== LOC ====
{loc_line}
"
                ),
            );
            let defs = loc_defs(&[(3745, 1, 1), (3746, 1, 1), (5010, 1, 1)]);
            let wc = bake_collision(&fx, &defs, &HashSet::new());
            derive_transports(fx.path(), &defs, &wc)
        }
        let constants = "\
^death_spoken_tenzing = 2
^death_got_map = 7
^death_map_lower = 0
^death_map_upper = 3
";
        let loc_pack = "3745=death_sherpa_door\n3746=death_sherpa_backdoor\n5010=good_door\n";
        let varps = "314=death_equiproom\n315=death_map\n";
        let control_script = "\
[oploc1,death_sherpa_door]
def_boolean $leaving = ~check_axis(coord, loc_coord, loc_angle);
if($leaving = true | ~death_get_map >= ^death_spoken_tenzing) {
    ~open_and_close_door2(loc_1532, $leaving, door_open);
    return;
}

[oploc1,good_door]
def_boolean $leaving = ~check_axis(coord, loc_coord, loc_angle);
if($leaving = true | ~death_get_map >= ^death_spoken_tenzing) {
    ~open_and_close_door2(loc_1532, $leaving, door_open);
    return;
}

[proc,death_get_map]()(int)
return (getbit_range(%death_map, ^death_map_lower, ^death_map_upper));
";
        let control = graph_for(
            control_script,
            loc_pack,
            varps,
            constants,
            "0 6 35: 3745 0 2\n0 6 36: 5010 0 2\n",
        );
        assert_eq!(
            door_crossings(&control, 3745),
            vec![
                ((2822, 3555), 'E', (2823, 3555)),
                ((2822, 3555), 'W', (2821, 3555)),
            ],
            "the sherpa control keeps the free exit and gated reverse"
        );
        let gated = control
            .edges
            .iter()
            .find(|e| e.loc_id == 3745 && e.dir == Some(DoorDir::W))
            .expect("3745 W");
        assert_eq!(gated.quest_req, vec!["Death Plateau".to_string()]);
        assert_eq!(
            door_crossings(&control, 5010),
            vec![((2822, 3556), 'E', (2823, 3556))],
            "an unrelated loc with the same shape must not be a dead pipeline"
        );
        assert!(
            control
                .edges
                .iter()
                .find(|e| e.loc_id == 5010)
                .is_some_and(|e| e.quest_req.is_empty()),
            "a non-sherpa loc must not borrow Death Plateau"
        );

        let wrong_varp = graph_for(
            "\
[oploc1,death_sherpa_door]
def_boolean $leaving = ~check_axis(coord, loc_coord, loc_angle);
if($leaving = true | ~death_get_map >= ^death_spoken_tenzing) {
    ~open_and_close_door2(loc_1532, $leaving, door_open);
    return;
}

[oploc1,good_door]
def_boolean $leaving = ~check_axis(coord, loc_coord, loc_angle);
if($leaving = true | ~death_get_map >= ^death_spoken_tenzing) {
    ~open_and_close_door2(loc_1532, $leaving, door_open);
    return;
}

[proc,death_get_map]()(int)
return (getbit_range(%death_equiproom, ^death_map_lower, ^death_map_upper));
",
            loc_pack,
            varps,
            constants,
            "0 6 35: 3745 0 2\n0 6 36: 5010 0 2\n",
        );
        assert_eq!(
            door_crossings(&wrong_varp, 3745),
            vec![((2822, 3555), 'E', (2823, 3555))],
            "a valid bitfield on the wrong varp still proves the free arm"
        );
        assert!(
            wrong_varp
                .edges
                .iter()
                .filter(|e| e.loc_id == 3745)
                .all(|e| e.quest_req.is_empty() && e.dir == Some(DoorDir::E)),
            "an unrelated varp must not borrow the Death Plateau reverse"
        );
        assert_eq!(
            door_crossings(&wrong_varp, 5010),
            vec![((2822, 3556), 'E', (2823, 3556))],
            "the control loc in the wrong-varp fixture must still emit"
        );

        let wrong_range = graph_for(
            "\
[oploc1,death_sherpa_door]
def_boolean $leaving = ~check_axis(coord, loc_coord, loc_angle);
if($leaving = true | ~death_get_map >= ^death_spoken_tenzing) {
    ~open_and_close_door2(loc_1532, $leaving, door_open);
    return;
}

[oploc1,good_door]
def_boolean $leaving = ~check_axis(coord, loc_coord, loc_angle);
if($leaving = true | ~good_get_map >= ^death_spoken_tenzing) {
    ~open_and_close_door2(loc_1532, $leaving, door_open);
    return;
}

[proc,death_get_map]()(int)
return (getbit_range(%death_map, ^death_map_lower, ^death_got_map));

[proc,good_get_map]()(int)
return (getbit_range(%death_map, ^death_map_lower, ^death_map_upper));
",
            loc_pack,
            varps,
            constants,
            "0 6 35: 3745 0 2\n0 6 36: 5010 0 2\n",
        );
        assert_eq!(
            door_crossings(&wrong_range, 3745),
            vec![((2822, 3555), 'E', (2823, 3555))],
            "bits 0..7 still prove a free arm"
        );
        assert!(
            wrong_range
                .edges
                .iter()
                .filter(|e| e.loc_id == 3745)
                .all(|e| e.quest_req.is_empty()),
            "a different bit range must not borrow Death Plateau"
        );
        assert_eq!(
            door_crossings(&wrong_range, 5010),
            vec![((2822, 3556), 'E', (2823, 3556))],
            "the 0..3 control must still emit beside the range negative"
        );

        let wrong_polarity = graph_for(
            "\
[oploc1,death_sherpa_door]
def_boolean $leaving = ~check_axis(coord, loc_coord, loc_angle);
if($leaving = false | ~death_get_map >= ^death_spoken_tenzing) {
    ~open_and_close_door2(loc_1532, $leaving, door_open);
    return;
}

[oploc1,good_door]
def_boolean $leaving = ~check_axis(coord, loc_coord, loc_angle);
if($leaving = true | ~death_get_map >= ^death_spoken_tenzing) {
    ~open_and_close_door2(loc_1532, $leaving, door_open);
    return;
}

[proc,death_get_map]()(int)
return (getbit_range(%death_map, ^death_map_lower, ^death_map_upper));
",
            loc_pack,
            varps,
            constants,
            "0 6 35: 3745 0 2\n0 6 36: 5010 0 2\n",
        );
        assert_eq!(
            door_crossings(&wrong_polarity, 3745),
            vec![((2822, 3555), 'W', (2821, 3555))],
            "the flipped polarity still proves its free crossing"
        );
        assert!(
            wrong_polarity
                .edges
                .iter()
                .filter(|e| e.loc_id == 3745)
                .all(|e| e.quest_req.is_empty()),
            "the wrong polarity must not borrow Death Plateau"
        );
        assert_eq!(
            door_crossings(&wrong_polarity, 5010),
            vec![((2822, 3556), 'E', (2823, 3556))],
            "the polarity control must still emit"
        );

        let duplicate = graph_for(
            "\
[oploc1,death_sherpa_door]
def_boolean $leaving = ~check_axis(coord, loc_coord, loc_angle);
if($leaving = true | ~death_get_map >= ^death_spoken_tenzing) {
    ~open_and_close_door2(loc_1532, $leaving, door_open);
    return;
}

[oploc1,good_door]
def_boolean $leaving = ~check_axis(coord, loc_coord, loc_angle);
if($leaving = true | ~good_get_map >= ^death_spoken_tenzing) {
    ~open_and_close_door2(loc_1532, $leaving, door_open);
    return;
}

[proc,death_get_map]()(int)
return (getbit_range(%death_map, ^death_map_lower, ^death_map_upper));

[proc,death_get_map]()(int)
return (getbit_range(%death_map, ^death_map_lower, ^death_map_upper));

[proc,good_get_map]()(int)
return (getbit_range(%death_map, ^death_map_lower, ^death_map_upper));
",
            loc_pack,
            varps,
            constants,
            "0 6 35: 3745 0 2\n0 6 36: 5010 0 2\n",
        );
        assert!(
            door_crossings(&duplicate, 3745).is_empty(),
            "a duplicated proc must not prove a free arm or borrow completion"
        );
        assert_eq!(
            door_crossings(&duplicate, 5010),
            vec![((2822, 3556), 'E', (2823, 3556))],
            "the unique-proc control must still emit beside the duplicate"
        );
    }

    /// The real Server content (274) carries the same four Tenzing
    /// directions: 3745's free exit E and gated entry W (completed Death
    /// Plateau), 3746's free garden-to-hut S and gated garden exit N, while
    /// the direct-varp castle door keeps both crossings and its gate.
    #[test]
    fn derive_transports_tenzing_free_arms_from_real_content() {
        let Some((graph, _)) = derive_from_real_content() else {
            return;
        };
        assert_eq!(
            door_crossings(&graph, 3745),
            vec![
                ((2822, 3555), 'E', (2823, 3555)),
                ((2822, 3555), 'W', (2821, 3555)),
            ],
            "3745 free exit E and gated front entry W"
        );
        assert_eq!(
            door_crossings(&graph, 3746),
            vec![
                ((2820, 3557), 'N', (2820, 3558)),
                ((2820, 3557), 'S', (2820, 3556)),
            ],
            "3746 gated garden exit N and free garden-to-hut S"
        );
        for e in graph.edges.iter().filter(|e| e.loc_id == 3745) {
            match e.dir {
                Some(DoorDir::E) => assert!(e.quest_req.is_empty() && e.varp_req.is_empty()),
                Some(DoorDir::W) => {
                    assert_eq!(e.quest_req, vec!["Death Plateau".to_string()]);
                    assert!(e.varp_req.is_empty());
                }
                other => panic!("unexpected 3745 dir {other:?}"),
            }
        }
        for e in graph.edges.iter().filter(|e| e.loc_id == 3746) {
            match e.dir {
                Some(DoorDir::S) => assert!(e.quest_req.is_empty() && e.varp_req.is_empty()),
                Some(DoorDir::N) => {
                    assert_eq!(e.quest_req, vec!["Death Plateau".to_string()]);
                    assert!(e.varp_req.is_empty());
                }
                other => panic!("unexpected 3746 dir {other:?}"),
            }
        }
        let castle: Vec<_> = graph
            .edges
            .iter()
            .filter(|e| e.kind == TransportKind::Door && e.loc_id == 3743)
            .collect();
        assert_eq!(castle.len(), 2, "the direct-varp door keeps both crossings");
        for e in &castle {
            assert_eq!(
                e.varp_req,
                vec![(314, 70)],
                "unchanged `%death_equiproom` gate"
            );
            assert!(e.quest_req.is_empty());
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
        // them (`calc_glidervar` has no lemanto_andra → hub pair): 7
        // flights × varp + journal proofs.
        assert_eq!(gliders.len(), 14);
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
        assert_eq!(hub_edges.len(), 8);
        assert!(hub_edges.iter().any(|e| e.to == sindarpos));
        assert!(hub_edges.iter().any(|e| e.to == gandius));
        assert!(hub_edges.iter().any(|e| e.to == lemanto_andra));
        let sindarpos_edges: Vec<_> = gliders.iter().filter(|e| e.at == sindarpos).collect();
        assert_eq!(sindarpos_edges.len(), 2);
        assert_eq!(sindarpos_edges[0].to, hub);
        // Lemanto Andra is one-way: no pad → hub flight exists in
        // gnome_glider.rs2.
        assert!(gliders.iter().all(|e| e.at != lemanto_andra));
        for g in &gliders {
            assert_eq!(g.option, 1, "Talk-to the Gnome pilot");
            assert_eq!(g.loc_id, 170);
            let varp = g.varp_req == [(150, 160)];
            let journal = g.quest_req == ["The Grand Tree".to_string()];
            assert!(
                varp ^ journal,
                "each flight is varp XOR journal, not both: {g:?}"
            );
        }
    }

    /// Live step 29: Gandius pad → Grand Tree hub. Empty WorldState
    /// cannot prove `%grandtree >= 160`; with that varp the packed glider
    /// hop is the route.
    #[test]
    fn gandius_glider_reaches_grand_tree_hub_with_grandtree_varp() {
        let Some((graph, collision)) = derive_from_real_content() else {
            return;
        };
        let pad = WorldTile {
            x: 2971,
            z: 2969,
            level: 0,
        };
        let hub = WorldTile {
            x: 2465,
            z: 3501,
            level: 3,
        };
        assert!(
            crate::router::find_with(
                &collision,
                &graph,
                pad,
                hub,
                crate::router::FindOptions::default(),
                &crate::world_state::WorldState::empty(),
            )
            .is_err(),
            "empty WorldState cannot take the Grand Tree glider"
        );
        let mut state = crate::world_state::WorldState::empty();
        state.varps.insert(150, 160);
        crate::router::find_with(
            &collision,
            &graph,
            pad,
            hub,
            crate::router::FindOptions::default(),
            &state,
        )
        .expect("Gandius → Grand Tree hub with grandtree 160");
        let mut journal = crate::world_state::WorldState::empty();
        journal.quests.insert("The Grand Tree".into());
        crate::router::find_with(
            &collision,
            &graph,
            pad,
            hub,
            crate::router::FindOptions::default(),
            &journal,
        )
        .expect("Gandius → Grand Tree hub with journal complete");
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

    /// Explicit real-content qualification inputs. Ordinary `cargo test -p nav`
    /// does not call this; the ignored tests below fail closed when the env
    /// is missing or a named path is absent. Does not scan default
    /// HOME/experiments layouts or unrelated worktrees.
    fn required_qualification_inputs() -> (Vec<PathBuf>, LocDefs) {
        let roots_raw = std::env::var("NAV_CONTENT_ROOT").unwrap_or_else(|_| {
            panic!(
                "NAV_CONTENT_ROOT is required (colon-separated content roots); \
                 this ignored qualification must not skip"
            )
        });
        let cache_raw = std::env::var("NAV_CACHE").unwrap_or_else(|_| {
            panic!(
                "NAV_CACHE is required (client config jag); \
                 this ignored qualification must not skip"
            )
        });
        let mut roots = Vec::new();
        for raw in roots_raw.split(':').filter(|s| !s.is_empty()) {
            let root = PathBuf::from(raw);
            assert!(
                root.join("maps").is_dir() && root.join("pack").join("loc.pack").is_file(),
                "NAV_CONTENT_ROOT entry {} is missing maps/ or pack/loc.pack",
                root.display()
            );
            roots.push(root);
        }
        assert!(
            !roots.is_empty(),
            "NAV_CONTENT_ROOT did not name any content root"
        );
        let cache_path = PathBuf::from(&cache_raw);
        let bytes = std::fs::read(&cache_path).unwrap_or_else(|e| {
            panic!("NAV_CACHE {} is unreadable: {e}", cache_path.display());
        });
        let cache = Cache::unpack(&JagFile::new(bytes));
        (roots, LocDefs::from_locs(&cache.locs))
    }

    fn derive_from_root_with(root: &Path, defs: &LocDefs) -> (TransportGraph, WorldCollision) {
        let wc = bake_from_maps(&root.join("maps"), defs, &HashSet::new())
            .unwrap_or_else(|e| panic!("qualification content bakes ({e:?}) {}", root.display()));
        let graph = derive_transports(root, defs, &wc);
        (graph, wc)
    }

    /// The real 289 and 274 content must derive the closed fence-gate pair
    /// behind Tenzing's passage (3725 `death_fencegate_l` at (2824,3555),
    /// 3726 `death_fencegate_r` at (2824,3554)) as ordinary closed-gate
    /// crossings: both members declare `category=gate_main_closed` /
    /// `gate_outer_closed` with `op1=Open` in
    /// `scripts/quests/quest_death/configs/quest_death.loc`, have no
    /// loc-specific `[oploc1,…]` block anywhere, and inherit
    /// `[oploc1,_gate_main_closed] ~open_gate;` /
    /// `[oploc1,_gate_outer_closed] ~open_outer_gate;` from
    /// `scripts/general_use/scripts/gates.rs2`. The Paterdomus pair
    /// (memberfencegate_l/_r, loc 1598/1599) carries the same categories and
    /// `op1=Open` but has loc-specific open scripts
    /// (`scripts/areas/area_paterdomus/scripts/paterdomus_members_gate.rs2`,
    /// the members gate), so it must never be inherited. The previously
    /// supported `gates.loc` members keep their crossings.
    #[test]
    #[ignore = "NAV_CONTENT_ROOT and NAV_CACHE required; absence fails"]
    fn derive_transports_tenzing_gate_pair_from_real_content() {
        let (roots, defs) = required_qualification_inputs();
        for root in roots {
            let (graph, _) = derive_from_root_with(&root, &defs);
            let doors = graph
                .edges
                .iter()
                .filter(|e| e.kind == TransportKind::Door)
                .count();
            eprintln!(
                "qualification {} edges={} doors={}",
                root.display(),
                graph.edges.len(),
                doors
            );
            assert_eq!(
                door_crossings(&graph, 3725),
                vec![
                    ((2824, 3555), 'E', (2825, 3555)),
                    ((2824, 3555), 'W', (2823, 3555)),
                ],
                "3725 must cross both ways ({})",
                root.display()
            );
            assert_eq!(
                door_crossings(&graph, 3726),
                vec![((2824, 3554), 'E', (2825, 3554))],
                "3726 keeps only its standable east crossing ({})",
                root.display()
            );
            let gates: Vec<_> = graph
                .edges
                .iter()
                .filter(|e| matches!(e.loc_id, 3725 | 3726))
                .collect();
            assert_eq!(gates.len(), 3, "({})", root.display());
            for e in gates {
                assert_eq!(e.option, 1, "{e:?} ({})", root.display());
                assert_eq!(
                    e.open_loc_id,
                    Some(if e.loc_id == 3725 { 3727 } else { 3728 }),
                    "the stage leaf of {e:?} ({})",
                    root.display()
                );
                assert!(
                    e.varp_req.is_empty()
                        && e.quest_req.is_empty()
                        && e.item_req.is_empty()
                        && e.worn_req.is_empty()
                        && e.skill_req.is_empty(),
                    "an inherited generic gate carries no requirement: {e:?} ({})",
                    root.display()
                );
            }
            // The named-override members stay out of the pack entirely.
            for id in [1598, 1599] {
                assert!(
                    door_crossings(&graph, id).is_empty(),
                    "Paterdomus loc {id} has a loc-specific open script and must not \
                     be inherited ({})",
                    root.display()
                );
            }
            // The generic `gates.loc` members keep their existing crossings.
            for id in [1551, 1553] {
                assert!(
                    !door_crossings(&graph, id).is_empty(),
                    "generic fence gate loc {id} lost its crossings ({})",
                    root.display()
                );
            }
        }
    }

    /// The derived fence-gate crossing must route the recorded start
    /// (2823,3555, the hut's front-door passage) out to the road and back:
    /// both legs hop loc 3725 through the fence. Incomplete / empty state
    /// still cannot enter the hut (3745 W requires completed Death Plateau);
    /// with that quest the recorded ClimbingBoots walk routes through 3745 W.
    /// Hut → road stays free via 3745 E. 3746 S remains the garden return,
    /// not a road entry.
    #[test]
    #[ignore = "NAV_CONTENT_ROOT and NAV_CACHE required; absence fails"]
    fn tenzing_passage_and_road_route_through_the_inherited_gate() {
        use crate::router::{find_with, FindOptions, Leg};
        let (roots, defs) = required_qualification_inputs();
        for root in roots {
            let (graph, wc) = derive_from_root_with(&root, &defs);
            let state = crate::world_state::WorldState::empty();
            let passage = WorldTile {
                x: 2823,
                z: 3555,
                level: 0,
            };
            let road = WorldTile {
                x: 2826,
                z: 3556,
                level: 0,
            };
            for (label, from, to, dir) in [
                ("passage -> road", passage, road, DoorDir::E),
                ("road -> passage", road, passage, DoorDir::W),
            ] {
                let route = find_with(&wc, &graph, from, to, FindOptions::default(), &state)
                    .unwrap_or_else(|e| panic!("{label} must route ({e:?})"));
                assert_eq!(route.dest, to, "{label} ({})", root.display());
                let hop = route
                    .legs
                    .iter()
                    .find_map(|l| match l {
                        Leg::Transport { edge } if edge.loc_id == 3725 => Some(edge.clone()),
                        _ => None,
                    })
                    .unwrap_or_else(|| {
                        panic!("{label} must hop the fence gate ({})", root.display())
                    });
                assert_eq!(hop.dir, Some(dir), "{label} ({})", root.display());
                assert_eq!(
                    (hop.at.x, hop.at.z),
                    (2824, 3555),
                    "{label} ({})",
                    root.display()
                );
                assert_eq!(
                    (hop.to.x, hop.to.z),
                    if dir == DoorDir::E {
                        (2825, 3555)
                    } else {
                        (2823, 3555)
                    },
                    "{label}: the landing on the crossing's far side ({})",
                    root.display()
                );
            }
            // Incomplete state: the hut's front room stays sealed.
            let hut = WorldTile {
                x: 2820,
                z: 3556,
                level: 0,
            };
            assert!(
                find_with(&wc, &graph, passage, hut, FindOptions::default(), &state).is_err(),
                "passage -> hut stays NoPath without completed Death Plateau ({})",
                root.display()
            );
            let high_bits = crate::world_state::WorldState {
                varps: HashMap::from([(315, i32::MAX)]),
                ..crate::world_state::WorldState::empty()
            };
            assert!(
                find_with(
                    &wc,
                    &graph,
                    passage,
                    hut,
                    FindOptions::default(),
                    &high_bits
                )
                .is_err(),
                "high raw 315 bits do not open 3745 W ({})",
                root.display()
            );
            let done = crate::world_state::WorldState {
                quests: HashSet::from(["Death Plateau".to_string()]),
                ..crate::world_state::WorldState::empty()
            };
            let entry = find_with(&wc, &graph, passage, hut, FindOptions::default(), &done)
                .unwrap_or_else(|e| {
                    panic!(
                        "passage -> hut must route with completed Death Plateau ({e:?}) ({})",
                        root.display()
                    )
                });
            let hop_3745 = entry
                .legs
                .iter()
                .find_map(|l| match l {
                    Leg::Transport { edge } if edge.loc_id == 3745 => Some(edge.clone()),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("passage -> hut must hop 3745 W ({})", root.display()));
            assert_eq!(hop_3745.dir, Some(DoorDir::W), "{}", root.display());
            assert_eq!(
                (hop_3745.at.x, hop_3745.at.z, hop_3745.to.x, hop_3745.to.z),
                (2822, 3555, 2821, 3555),
                "{}",
                root.display()
            );
            assert_eq!(
                hop_3745.quest_req,
                vec!["Death Plateau".to_string()],
                "{}",
                root.display()
            );
            // Hut -> road does not need the quest: free 3745 E then 3725 E.
            let exit = find_with(&wc, &graph, hut, road, FindOptions::default(), &state)
                .unwrap_or_else(|e| {
                    panic!(
                        "hut -> road must route without a quest ({e:?}) ({})",
                        root.display()
                    )
                });
            assert!(
                exit.legs.iter().any(|l| matches!(
                    l,
                    Leg::Transport { edge } if edge.loc_id == 3745 && edge.dir == Some(DoorDir::E)
                )),
                "hut -> road hops 3745 E ({})",
                root.display()
            );
            assert!(
                exit.legs.iter().any(|l| matches!(
                    l,
                    Leg::Transport { edge } if edge.loc_id == 3725 && edge.dir == Some(DoorDir::E)
                )),
                "hut -> road hops the fence ({})",
                root.display()
            );
            // Taverley is south of the compound gates on the road toward
            // Falador; empty state can leave the passage. Falador's interior
            // pin is a separate city-wall problem — the fence is not the seal
            // once passage -> road routes.
            let taverley = WorldTile {
                x: 2895,
                z: 3435,
                level: 0,
            };
            find_with(
                &wc,
                &graph,
                passage,
                taverley,
                FindOptions::default(),
                &state,
            )
            .unwrap_or_else(|e| {
                panic!(
                    "passage -> Taverley must no longer be sealed by the missing compound gates ({e:?}) ({})",
                    root.display()
                )
            });
            find_with(&wc, &graph, taverley, hut, FindOptions::default(), &done).unwrap_or_else(
                |e| {
                    panic!(
                        "Taverley -> hut with completed Death Plateau ({e:?}) ({})",
                        root.display()
                    )
                },
            );
            assert!(
                find_with(&wc, &graph, taverley, hut, FindOptions::default(), &state).is_err(),
                "Taverley -> hut stays NoPath without the quest ({})",
                root.display()
            );
            // 3746 S remains the garden return; N is the gated reverse.
            assert_eq!(
                door_crossings(&graph, 3746),
                vec![
                    ((2820, 3557), 'N', (2820, 3558)),
                    ((2820, 3557), 'S', (2820, 3556)),
                ],
                "3746 keeps garden -> hut free and the gated reverse ({})",
                root.display()
            );
            let bank = WorldTile {
                x: 2946,
                z: 3369,
                level: 0,
            };
            for (label, from) in [("passage", passage), ("Taverley", taverley)] {
                match find_with(&wc, &graph, from, bank, FindOptions::default(), &state) {
                    Ok(route) => eprintln!(
                        "{label} -> BANK_STAND(2946,3369) ok dest=({},{},{}) legs={} ({})",
                        route.dest.x,
                        route.dest.z,
                        route.dest.level,
                        route.legs.len(),
                        root.display()
                    ),
                    Err(e) => eprintln!(
                        "{label} -> BANK_STAND(2946,3369) {e:?} ({})",
                        root.display()
                    ),
                }
            }
        }
    }

    fn membergate_pack() -> &'static str {
        "\
1596=membergatel
1597=membergater
1560=loc_1560
1561=loc_1561
"
    }

    fn membergate_loc_blocks() -> &'static str {
        "\
[membergatel]
name=Gate
desc=A wrought iron gate.
model=outdoorfurniture_metalgateclosedl
op1=Open
active=yes
blockrange=no
raiseobject=no
category=door_left_closed
param=next_loc_stage,loc_1560
param=open_sound,grate_open

[membergater]
name=Gate
desc=A wrought iron gate.
model=outdoorfurniture_metalgateclosedl
op1=Open
mirror=yes
active=yes
blockrange=no
raiseobject=no
category=door_right_closed
param=next_loc_stage,loc_1561
param=open_sound,grate_open

[loc_1560]
name=Gate
desc=A wrought iron gate.
model=outdoorfurniture_metalgateclosedl
op1=Close
active=yes
raiseobject=no
category=door_left_opened
param=next_loc_stage,loc_1557
param=close_sound,grate_close

[loc_1561]
name=Gate
desc=A wrought iron gate.
model=outdoorfurniture_metalgateclosedl
op1=Close
mirror=yes
active=yes
raiseobject=no
category=door_right_opened
param=next_loc_stage,loc_1558
param=close_sound,grate_close
"
    }

    fn membergate_handlers_text() -> &'static str {
        "\
[oploc1,membergatel]
if (map_members = ^false) {
    mes(^mes_members_gate);
    return;
}
~open_double_doors_left(500, door_right_closed, loc_param(open_sound));

[oploc1,membergater]
if (map_members = ^false) {
    mes(^mes_members_gate);
    return;
}
~open_double_doors_right(500, door_left_closed, loc_param(open_sound));

[proc,open_double_doors_left](int $duration, category $category, synth $sound)
return;

[proc,open_double_doors_right](int $duration, category $category, synth $sound)
return;
"
    }

    fn write_membergate_family(fx: &Fixture) {
        fx.write("pack/loc.pack", membergate_pack());
        fx.write(
            "scripts/doors/configs/doubledoors.loc",
            membergate_loc_blocks(),
        );
        fx.write(
            "scripts/doors/scripts/doubledoors.rs2",
            membergate_handlers_text(),
        );
    }

    fn parse_membergate_defs(
        text: &str,
    ) -> (
        HashMap<i32, MembergateDef>,
        HashMap<i32, MembergateOpenLeaf>,
        HashSet<i32>,
    ) {
        let fx = Fixture::new();
        fx.write("scripts/doors/configs/doubledoors.loc", text);
        membergate_loc_defs(
            fx.path(),
            &HashMap::from([
                (MEMBERGATE_LEFT.to_string(), 1596),
                (MEMBERGATE_RIGHT.to_string(), 1597),
                ("loc_1560".to_string(), 1560),
                ("loc_1561".to_string(), 1561),
            ]),
        )
    }

    #[test]
    fn membergate_open_leaf_conflicts_are_order_independent_and_permanent() {
        let valid = "[loc_1560]\nop1=Close\ncategory=door_left_opened\n";
        let invalid = "[loc_1560]\nop1=Close\ncategory=other\n";
        for text in [
            format!("{valid}{invalid}"),
            format!("{invalid}{valid}"),
            format!("{valid}{invalid}{valid}"),
        ] {
            let (_, leaves, conflicted) = parse_membergate_defs(&text);
            assert!(conflicted.contains(&1560), "{text:?}");
            assert!(!leaves.contains_key(&1560), "{text:?}");
        }
    }

    #[test]
    fn membergate_numeric_closed_alias_conflicts_with_named_definition() {
        let valid =
            "[membergatel]\nop1=Open\ncategory=door_left_closed\nparam=next_loc_stage,loc_1560\n";
        let invalid = "[loc_1596]\nop1=Close\ncategory=other\n";
        for text in [format!("{valid}{invalid}"), format!("{invalid}{valid}")] {
            let (defs, _, conflicted) = parse_membergate_defs(&text);
            assert!(conflicted.contains(&1596), "{text:?}");
            assert!(!defs.contains_key(&1596), "{text:?}");
        }
    }

    #[test]
    fn membergate_relevant_keys_cannot_promote_an_invalid_block() {
        let text = "\
[membergatel]
op1=Close
op1=Open
category=other
category=door_left_closed
param=next_loc_stage,missing_leaf
param=next_loc_stage,loc_1560
";
        let (defs, _, _) = parse_membergate_defs(text);
        assert!(
            defs.get(&1596)
                .is_none_or(|def| !def.op_open || !def.category_ok || def.open.is_none()),
            "contradictory repeated keys must not produce a valid definition: {defs:?}"
        );
    }

    #[test]
    fn membergate_identical_duplicate_definitions_remain_valid() {
        let text = format!("{}{}", membergate_loc_blocks(), membergate_loc_blocks());
        let (defs, leaves, conflicted) = parse_membergate_defs(&text);
        assert!(conflicted.is_empty(), "{conflicted:?}");
        assert_eq!(defs.len(), 2, "{defs:?}");
        assert_eq!(leaves.len(), 2, "{leaves:?}");
    }

    fn write_blocked_square(fx: &Fixture, mx: i32, mz: i32, walk: &[(i32, i32)], locs: &str) {
        let mut map = String::from("==== MAP ====\n");
        let ox = mx * 64;
        let oz = mz * 64;
        let walk: HashSet<(i32, i32)> = walk.iter().copied().collect();
        for lz in 0..64i32 {
            for lx in 0..64i32 {
                if !walk.contains(&(ox + lx, oz + lz)) {
                    map.push_str(&format!("0 {lx} {lz}: f1 u48\n"));
                }
            }
        }
        map.push_str("\n==== LOC ====\n");
        map.push_str(locs);
        fx.write(&format!("maps/m{mx}_{mz}.jm2"), &map);
    }

    /// The Taverley membergate pair emits four crossings with members_req.
    #[test]
    fn derive_transports_emits_membergate_family_crossings() {
        let fx = Fixture::new();
        write_membergate_family(&fx);
        fx.write(
            "maps/m45_53.jm2",
            "\
==== MAP ====
0 55 58: f1 u48

==== LOC ====
0 55 58: 1597 0 2
0 55 59: 1596 0 2
",
        );
        let defs = loc_defs(&[(1596, 1, 1), (1597, 1, 1), (1560, 1, 1), (1561, 1, 1)]);
        let wc = bake_collision(&fx, &defs, &HashSet::from([1596, 1597]));
        let graph = derive_transports(fx.path(), &defs, &wc);
        assert_eq!(
            door_crossings(&graph, 1596),
            vec![
                ((2935, 3451), 'E', (2936, 3451)),
                ((2935, 3451), 'W', (2934, 3451)),
            ],
            "1596 crosses both ways"
        );
        assert_eq!(
            door_crossings(&graph, 1597),
            vec![
                ((2935, 3450), 'E', (2936, 3450)),
                ((2935, 3450), 'W', (2934, 3450)),
            ],
            "1597 crosses both ways"
        );
        for id in [1560, 1561] {
            assert!(
                door_crossings(&graph, id).is_empty(),
                "open leaf {id} is not a crossing"
            );
        }
        for e in graph
            .edges
            .iter()
            .filter(|e| matches!(e.loc_id, 1596 | 1597))
        {
            assert_eq!(e.option, 1, "{e:?}");
            assert!(e.members_req, "{e:?}");
            assert_eq!(
                e.open_loc_id,
                Some(if e.loc_id == 1596 { 1560 } else { 1561 }),
                "{e:?}"
            );
            assert!(
                e.skill_req.is_empty()
                    && e.item_req.is_empty()
                    && e.quest_req.is_empty()
                    && e.varp_req.is_empty()
                    && e.worn_req.is_empty(),
                "{e:?}"
            );
        }
        assert!(
            !crate::pack::parse_door_config(membergate_loc_blocks()).contains(&1596),
            "parse_door_config still ignores named membergate blocks"
        );
    }

    /// Empty WorldState cannot walk Taverley to BANK_STAND; map_members opens
    /// the membergate hop. The corridor is sealed except through 1596/1597.
    #[test]
    fn membergate_routes_taverley_bank_only_when_map_members() {
        use crate::router::{find_with, FindOptions, Leg, RouteError};
        let fx = Fixture::new();
        write_membergate_family(&fx);
        let mut walk = Vec::new();
        for x in 2895..=2934 {
            walk.push((x, 3435));
        }
        for z in 3435..=3451 {
            walk.push((2934, z));
        }
        for z in 3450..=3451 {
            walk.push((2935, z));
            walk.push((2936, z));
        }
        for z in 3369..=3451 {
            walk.push((2936, z));
        }
        for x in 2936..=2946 {
            walk.push((x, 3369));
        }
        write_blocked_square(&fx, 45, 53, &walk, "0 55 58: 1597 0 2\n0 55 59: 1596 0 2\n");
        write_blocked_square(&fx, 45, 52, &walk, "");
        write_blocked_square(&fx, 46, 52, &walk, "");
        let defs = loc_defs(&[(1596, 1, 1), (1597, 1, 1), (1560, 1, 1), (1561, 1, 1)]);
        let wc = bake_collision(&fx, &defs, &HashSet::from([1596, 1597]));
        let graph = derive_transports(fx.path(), &defs, &wc);
        let taverley = WorldTile {
            x: 2895,
            z: 3435,
            level: 0,
        };
        let bank = WorldTile {
            x: 2946,
            z: 3369,
            level: 0,
        };
        let empty = crate::world_state::WorldState::empty();
        assert!(
            matches!(
                find_with(&wc, &graph, taverley, bank, FindOptions::default(), &empty),
                Err(RouteError::NoPath)
            ),
            "empty state cannot open the members gate"
        );
        assert!(
            matches!(
                find_with(&wc, &graph, bank, taverley, FindOptions::default(), &empty),
                Err(RouteError::NoPath)
            ),
            "empty reverse is also NoPath"
        );
        let members = empty.clone().with_map_members(true);
        let there = find_with(
            &wc,
            &graph,
            taverley,
            bank,
            FindOptions::default(),
            &members,
        )
        .expect("members world routes Taverley to bank");
        assert!(
            there.legs.iter().any(|l| matches!(
                l,
                Leg::Transport { edge } if matches!(edge.loc_id, 1596 | 1597) && edge.members_req
            )),
            "the hop is a membergate: {there:?}"
        );
        let back = find_with(
            &wc,
            &graph,
            bank,
            taverley,
            FindOptions::default(),
            &members,
        )
        .expect("members world routes bank to Taverley");
        assert!(back.legs.iter().any(|l| matches!(
            l,
            Leg::Transport { edge } if matches!(edge.loc_id, 1596 | 1597)
        )));
    }

    /// Near-misses stay out; a control pair in the same fixture is admitted.
    #[test]
    fn derive_transports_omits_unproven_membergate_members() {
        let fx = Fixture::new();
        fx.write(
            "pack/loc.pack",
            "\
1596=membergatel
1597=membergater
1560=loc_1560
1561=loc_1561
1598=memberfencegate_l
1599=memberfencegate_r
5001=plainopen
5002=unpaired_left
5003=unpaired_right
",
        );
        fx.write(
            "scripts/doors/configs/doubledoors.loc",
            &format!(
                "{}
[plainopen]
op1=Open
category=door_left_closed
param=next_loc_stage,loc_1560

[memberfencegate_l]
op1=Open
category=gate_main_closed
param=next_loc_stage,loc_1560

[memberfencegate_r]
op1=Open
category=gate_outer_closed
param=next_loc_stage,loc_1561
",
                membergate_loc_blocks()
            ),
        );
        fx.write(
            "scripts/doors/scripts/doubledoors.rs2",
            &format!(
                "{}
[oploc1,plainopen]
~open_double_doors_left(500, door_right_closed, loc_param(open_sound));

[oploc1,memberfencegate_l]
if(map_members = ^false) {{
    mes(^mes_members_gate);
    return;
}}
~open_gate;
",
                membergate_handlers_text()
            ),
        );
        fx.write(
            "maps/m45_53.jm2",
            "\
==== MAP ====
0 0 0: f1 u48

==== LOC ====
0 55 58: 1597 0 2
0 55 59: 1596 0 2
0 10 10: 5002 0 2
0 20 20: 1598 0 2
0 20 21: 1599 0 2
0 30 30: 5001 0 2
",
        );
        let defs = loc_defs(&[
            (1596, 1, 1),
            (1597, 1, 1),
            (1560, 1, 1),
            (1561, 1, 1),
            (1598, 1, 1),
            (1599, 1, 1),
            (5001, 1, 1),
            (5002, 1, 1),
        ]);
        let wc = bake_collision(&fx, &defs, &HashSet::new());
        let graph = derive_transports(fx.path(), &defs, &wc);
        assert!(
            !door_crossings(&graph, 1596).is_empty() && !door_crossings(&graph, 1597).is_empty(),
            "control Taverley pair is admitted"
        );
        for id in [1598, 1599, 5001, 5002] {
            assert!(
                graph.edges.iter().all(|e| e.loc_id != id),
                "unproven loc {id} must not emit"
            );
        }
    }

    /// A second named handler with a different body fail-closes the family.
    #[test]
    fn derive_transports_omits_membergate_handler_conflicts() {
        let fx = Fixture::new();
        write_membergate_family(&fx);
        fx.write(
            "scripts/areas/area_extra/scripts/extra.rs2",
            "[oploc1,membergatel] mes(^mes_members_gate);\n",
        );
        fx.write(
            "maps/m45_53.jm2",
            "\
==== MAP ====
0 55 58: f1 u48

==== LOC ====
0 55 58: 1597 0 2
0 55 59: 1596 0 2
",
        );
        let defs = loc_defs(&[(1596, 1, 1), (1597, 1, 1), (1560, 1, 1), (1561, 1, 1)]);
        let wc = bake_collision(&fx, &defs, &HashSet::new());
        let graph = derive_transports(fx.path(), &defs, &wc);
        assert!(
            door_crossings(&graph, 1596).is_empty() && door_crossings(&graph, 1597).is_empty(),
            "a conflicting extra handler fail-closes the family"
        );
    }

    fn find_radius3(
        wc: &WorldCollision,
        graph: &TransportGraph,
        from: WorldTile,
        to: WorldTile,
        state: &crate::world_state::WorldState,
    ) -> Result<crate::router::Route, crate::router::RouteError> {
        use crate::router::{find_with, local_step_component, FindOptions, RouteError};
        let mut dests: Vec<_> = local_step_component(wc, to, 3).into_iter().collect();
        dests.sort_by_key(|t| ((t.x - from.x).abs().max((t.z - from.z).abs()), t.x, t.z));
        let mut last = Err(RouteError::NoPath);
        for dest in dests {
            match find_with(wc, graph, from, dest, FindOptions::default(), state) {
                Ok(route) => return Ok(route),
                Err(e) => last = Err(e),
            }
        }
        last
    }

    /// Real 274+289 content: Taverley 1596/1597 crossings, empty-state
    /// radius-3 bank remains NoPath, members world routes through the family.
    /// 274config probes of 289 content are not fresh 289 qualification.
    #[test]
    #[ignore = "NAV_CONTENT_ROOT and NAV_CACHE required; absence fails"]
    fn membergate_taverley_bank_from_real_content() {
        use crate::router::{Leg, RouteError};
        let (roots, defs) = required_qualification_inputs();
        let taverley = WorldTile {
            x: 2895,
            z: 3435,
            level: 0,
        };
        let passage = WorldTile {
            x: 2823,
            z: 3555,
            level: 0,
        };
        let bank = WorldTile {
            x: 2946,
            z: 3369,
            level: 0,
        };
        for root in roots {
            let (graph, wc) = derive_from_root_with(&root, &defs);
            let crossings_at = |id, x, z| {
                door_crossings(&graph, id)
                    .into_iter()
                    .filter(|((at_x, at_z), _, _)| *at_x == x && *at_z == z)
                    .collect::<Vec<_>>()
            };
            assert_eq!(
                crossings_at(1596, 2935, 3451),
                vec![
                    ((2935, 3451), 'E', (2936, 3451)),
                    ((2935, 3451), 'W', (2934, 3451)),
                ],
                "1596 ({})",
                root.display()
            );
            assert_eq!(
                crossings_at(1597, 2935, 3450),
                vec![
                    ((2935, 3450), 'E', (2936, 3450)),
                    ((2935, 3450), 'W', (2934, 3450)),
                ],
                "1597 ({})",
                root.display()
            );
            let family: Vec<_> = graph
                .edges
                .iter()
                .filter(|e| matches!(e.loc_id, 1596 | 1597))
                .collect();
            let family_placements: HashSet<_> =
                family.iter().map(|edge| (edge.loc_id, edge.at)).collect();
            eprintln!(
                "membergate proof {}: family_edges={} family_placements={}",
                root.display(),
                family.len(),
                family_placements.len()
            );
            for e in &family {
                assert!(e.members_req, "{e:?} ({})", root.display());
                assert!(
                    family.iter().any(|pair| {
                        pair.loc_id != e.loc_id
                            && pair.at.level == e.at.level
                            && pair.dir == e.dir
                            && (pair.at.x - e.at.x).abs() + (pair.at.z - e.at.z).abs() == 1
                    }),
                    "admitted family placement has no complementary pair: {e:?} ({})",
                    root.display()
                );
            }
            for id in [1598, 1599] {
                assert!(
                    door_crossings(&graph, id).is_empty(),
                    "Paterdomus {id} must stay out ({})",
                    root.display()
                );
            }
            let empty = crate::world_state::WorldState::empty();
            for (label, from, to) in [
                ("Taverley -> bank", taverley, bank),
                ("bank -> Taverley", bank, taverley),
                ("passage -> bank", passage, bank),
                ("bank -> passage", bank, passage),
            ] {
                assert!(
                    matches!(
                        find_radius3(&wc, &graph, from, to, &empty),
                        Err(RouteError::NoPath)
                    ),
                    "{label} unknown/false membership remains NoPath ({})",
                    root.display()
                );
            }
            let members = empty.clone().with_map_members(true);
            let agility1 = crate::world_state::WorldState {
                stats: std::collections::HashMap::from([(16, 1)]),
                map_members: true,
                ..crate::world_state::WorldState::default()
            };
            let there = find_radius3(&wc, &graph, taverley, bank, &agility1).unwrap_or_else(|e| {
                panic!("members Taverley -> bank ({e:?}) ({})", root.display())
            });
            assert!(
                there.legs.iter().any(|l| matches!(
                    l,
                    Leg::Transport { edge } if matches!(edge.loc_id, 1596 | 1597)
                )),
                "Taverley -> bank hops membergate ({})",
                root.display()
            );
            let route_fact = |label: &str, route: &crate::router::Route| {
                let transports: Vec<_> = route
                    .legs
                    .iter()
                    .filter_map(|leg| match leg {
                        Leg::Transport { edge } => Some(edge.loc_id),
                        _ => None,
                    })
                    .collect();
                eprintln!(
                    "{label}: dest=({},{},{}) legs={} transports={transports:?} ({})",
                    route.dest.x,
                    route.dest.z,
                    route.dest.level,
                    route.legs.len(),
                    root.display()
                );
            };
            route_fact("members Taverley -> bank radius3", &there);
            let back = find_radius3(&wc, &graph, bank, taverley, &members).unwrap_or_else(|e| {
                panic!("members bank -> Taverley ({e:?}) ({})", root.display())
            });
            route_fact("members bank -> Taverley radius3", &back);
            let done = crate::world_state::WorldState {
                quests: ["Death Plateau".to_string()].into(),
                stats: std::collections::HashMap::from([(16, 1)]),
                map_members: true,
                ..crate::world_state::WorldState::default()
            };
            let passage_to_bank =
                find_radius3(&wc, &graph, passage, bank, &done).unwrap_or_else(|e| {
                    panic!(
                        "passage -> bank with Death Plateau ({e:?}) ({})",
                        root.display()
                    )
                });
            route_fact(
                "members passage -> bank radius3 Death Plateau",
                &passage_to_bank,
            );
            let bank_to_passage =
                find_radius3(&wc, &graph, bank, passage, &done).unwrap_or_else(|e| {
                    panic!(
                        "bank -> passage with Death Plateau ({e:?}) ({})",
                        root.display()
                    )
                });
            route_fact(
                "members bank -> passage radius3 Death Plateau",
                &bank_to_passage,
            );
        }
    }

    /// The closed-gate inheritance is admitted from a quest config with the
    /// real source shapes: the two category members, their `op1=Open`, the
    /// named `next_loc_stage` leaves and the adjacent pair all resolve, and
    /// the members cross both ways with no requirement (the generic
    /// category handler opens them).
    #[test]
    fn derive_transports_emits_inherited_closed_gate_crossings() {
        let fx = Fixture::new();
        fx.write(
            "pack/loc.pack",
            "\
3725=death_fencegate_l
3726=death_fencegate_r
3727=death_openfencegate_l
3728=death_openfencegate_r
",
        );
        fx.write(
            "scripts/general_use/scripts/gates.rs2",
            "\
[proc,open_gate]
def_coord $main_open = ~movecoord_loc_return(~gate_set_close(loc_angle, 1));
return;

[proc,open_outer_gate]
loc_findallzone(~get_pair_coord(loc_coord, loc_angle, true));
return;

[oploc1,_gate_main_closed] ~open_gate;
[oploc1,_gate_outer_closed] ~open_outer_gate;
",
        );
        // Both closed members and both open leaves, verbatim shapes from
        // `scripts/quests/quest_death/configs/quest_death.loc`.
        fx.write(
            "scripts/quests/quest_death/configs/quest_death.loc",
            "\
[death_fencegate_l]
name=Gate
op1=Open
active=yes
blockrange=no
category=gate_main_closed
param=next_loc_stage,death_openfencegate_l

[death_fencegate_r]
name=Gate
op1=Open
active=yes
blockrange=no
mirror=yes
category=gate_outer_closed
param=next_loc_stage,death_openfencegate_r

[death_openfencegate_l]
name=Gate
op1=Close
active=yes
blockrange=no
category=gate_main_open
param=next_loc_stage,death_fencegate_l

[death_openfencegate_r]
name=Gate
op1=Close
active=yes
blockrange=no
mirror=yes
category=gate_outer_open
param=next_loc_stage,death_fencegate_r
",
        );
        // The m44_55 placements: 3726 at (2824,3554), 3725 at (2824,3555),
        // both angle 2, with (2823,3554) blocked exactly as the real map is.
        fx.write(
            "maps/m44_55.jm2",
            "\
==== MAP ====
0 7 34: f1 u48

==== LOC ====
0 8 34: 3726 0 2
0 8 35: 3725 0 2
",
        );
        let defs = loc_defs(&[(3725, 1, 1), (3726, 1, 1), (3727, 1, 1), (3728, 1, 1)]);
        let wc = bake_collision(&fx, &defs, &HashSet::new());
        let graph = derive_transports(fx.path(), &defs, &wc);

        assert_eq!(
            door_crossings(&graph, 3725),
            vec![
                ((2824, 3555), 'E', (2825, 3555)),
                ((2824, 3555), 'W', (2823, 3555)),
            ],
            "the main member crosses both ways"
        );
        assert_eq!(
            door_crossings(&graph, 3726),
            vec![((2824, 3554), 'E', (2825, 3554))],
            "the outer member keeps only its standable east crossing"
        );
        for id in [3727, 3728] {
            assert!(
                door_crossings(&graph, id).is_empty(),
                "the open leaf {id} is not a crossing"
            );
        }
        let mut leaves = HashSet::new();
        for e in graph
            .edges
            .iter()
            .filter(|e| matches!(e.loc_id, 3725 | 3726))
        {
            assert_eq!(e.option, 1, "Open op: {e:?}");
            assert!(
                e.varp_req.is_empty()
                    && e.quest_req.is_empty()
                    && e.item_req.is_empty()
                    && e.worn_req.is_empty()
                    && e.skill_req.is_empty(),
                "the inherited category handler carries no requirement: {e:?}"
            );
            leaves.insert(e.open_loc_id);
        }
        assert_eq!(
            leaves,
            HashSet::from([Some(3727), Some(3728)]),
            "each member carries its own stage leaf"
        );
    }

    /// Every near-miss below shares the closed categories with the admitted
    /// pair but breaks one part of the inheritance: a loc-specific open
    /// script (named override), a category with no verified handler, a
    /// member without the `Open` op, an unresolvable `next_loc_stage`, a
    /// member with no adjacent paired placement, and a member defined twice
    /// with disagreeing data. None may be promoted, and an unrelated
    /// `op1=Open` quest door is not admitted either. The valid control pair
    /// (5071/5072) is admitted from the same fixture, so every negative
    /// below is a refusal, not a dead pipeline. Only the main category
    /// handler is verified here: the outer handler is missing entirely, so
    /// the control's outer member (the pair the main needs) is exactly the
    /// unsupported-handler case.
    #[test]
    fn derive_transports_omits_unproven_inherited_gate_members() {
        let fx = Fixture::new();
        fx.write(
            "pack/loc.pack",
            "\
5001=death_gate_override
5002=death_gate_override_outer
5003=death_gate_override_open
5004=death_gate_override_outer_open
5021=death_gate_noop
5025=death_gate_noop_open
5031=death_gate_nostage
5041=death_gate_unpaired
5045=death_gate_unpaired_open
5051=death_gate_conflict
5059=death_gate_conflict_open
5061=death_plainopen
5071=death_gate_control_main
5072=death_gate_control_outer
5073=death_gate_control_open
",
        );
        // Only the main category handler is verified here: the outer handler
        // is missing entirely, so `gate_outer_closed` has nothing to
        // inherit.
        fx.write(
            "scripts/general_use/scripts/gates.rs2",
            "\
[proc,open_gate]
return;

[oploc1,_gate_main_closed] ~open_gate;
",
        );
        fx.write(
            "scripts/quests/quest_neg/configs/neg.loc",
            "\
[death_gate_override]
op1=Open
category=gate_main_closed
param=next_loc_stage,death_gate_override_open

[death_gate_override_outer]
op1=Open
category=gate_outer_closed
param=next_loc_stage,death_gate_override_outer_open

[death_gate_noop]
op1=Climb
category=gate_main_closed
param=next_loc_stage,death_gate_noop_open

[death_gate_nostage]
op1=Open
category=gate_main_closed
param=next_loc_stage,death_gate_absent_leaf

[death_gate_unpaired]
op1=Open
category=gate_main_closed
param=next_loc_stage,death_gate_unpaired_open

[death_gate_conflict]
op1=Open
category=gate_main_closed
param=next_loc_stage,death_gate_conflict_open

[death_plainopen]
op1=Open

[death_gate_control_main]
op1=Open
category=gate_main_closed
param=next_loc_stage,death_gate_control_open

[death_gate_control_outer]
op1=Open
category=gate_outer_closed
param=next_loc_stage,death_gate_control_open

[death_gate_control_open]
op1=Close
category=gate_main_open
",
        );
        // The same member again, with a different category and stage.
        fx.write(
            "scripts/quests/quest_neg/configs/neg_again.loc",
            "\
[death_gate_conflict]
op1=Open
category=gate_outer_closed
param=next_loc_stage,death_gate_conflict_open
",
        );
        // The loc-specific open scripts the resolver prefers, and an
        // unrelated free-standing `Open` door.
        fx.write(
            "scripts/quests/quest_neg/scripts/neg.rs2",
            "\
[oploc1,death_gate_override]
mes(^mes_members_gate);
return;

[oploc1,death_gate_override_outer]
mes(^mes_members_gate);
return;

[oploc1,death_plainopen]
~open_and_close_door2(loc_1532, true, door_open);
return;
",
        );
        // Pairs sit at (x, z) + (x, z+1) for angle 2 (outer at z, main at
        // z+1, the `get_pair_coord` rule); the unpaired member has no
        // partner placement at all.
        fx.write(
            "maps/m44_53.jm2",
            "\
==== MAP ====
0 0 0: f0 u48

==== LOC ====
0 1 3: 5002 0 2
0 1 4: 5001 0 2
0 2 3: 5072 0 2
0 2 4: 5071 0 2
0 4 4: 5021 0 2
0 5 4: 5031 0 2
0 6 4: 5041 0 2
0 7 4: 5051 0 2
0 8 4: 5061 0 2
",
        );
        let defs = loc_defs(&[
            (5001, 1, 1),
            (5002, 1, 1),
            (5021, 1, 1),
            (5031, 1, 1),
            (5041, 1, 1),
            (5051, 1, 1),
            (5061, 1, 1),
            (5071, 1, 1),
            (5072, 1, 1),
        ]);
        let wc = bake_collision(&fx, &defs, &HashSet::new());
        let graph = derive_transports(fx.path(), &defs, &wc);

        // The control: the same shapes without any flaw do cross.
        assert_eq!(
            door_crossings(&graph, 5071),
            vec![
                ((2818, 3396), 'E', (2819, 3396)),
                ((2818, 3396), 'W', (2817, 3396)),
            ],
            "the valid control member must be admitted from the same fixture"
        );
        for (id, why) in [
            (5001, "the main member has a loc-specific open script"),
            (5002, "the outer member has a loc-specific open script"),
            (5072, "the outer category handler is missing"),
            (5021, "the member has no Open op"),
            (5031, "the next_loc_stage leaf does not resolve"),
            (5041, "the member has no paired placement"),
            (5051, "the member is defined twice with different data"),
            (5061, "an unrelated op1=Open door without a gate category"),
        ] {
            assert!(
                door_crossings(&graph, id).is_empty(),
                "loc {id} must not be promoted: {why}"
            );
        }
    }

    /// Unresolvable headers must end the previous block, `loc_N` aliases
    /// must resolve through loc.pack, and a missing or mismatched open-leaf
    /// category must refuse the member. The control pair in the same
    /// fixture still crosses.
    #[test]
    fn derive_transports_omits_malformed_inherited_gate_blocks() {
        let fx = Fixture::new();
        fx.write(
            "pack/loc.pack",
            "\
5081=scan_control_main
5082=scan_control_outer
5083=scan_control_open
5084=scan_control_outer_open
5085=scan_leak_main
5086=scan_leak_outer
5087=scan_leak_open
5088=scan_leak_outer_open
5089=scan_missing_main
5090=scan_missing_outer
5091=scan_missing_open
5092=scan_mismatch_main
5093=scan_mismatch_outer
5094=scan_mismatch_open
5095=scan_bogus_main
5096=scan_bogus_outer
5097=scan_bogus_open
",
        );
        fx.write(
            "scripts/general_use/scripts/gates.rs2",
            "\
[proc,open_gate]
return;

[proc,open_outer_gate]
return;

[oploc1,_gate_main_closed] ~open_gate;
[oploc1,_gate_outer_closed] ~open_outer_gate;
",
        );
        fx.write(
            "scripts/quests/quest_scan/configs/scan.loc",
            "\
[scan_control_main]
op1=Open
category=gate_main_closed
param=next_loc_stage,scan_control_open

[scan_control_outer]
op1=Open
category=gate_outer_closed
param=next_loc_stage,scan_control_outer_open

[scan_control_open]
op1=Close
category=gate_main_open

[scan_control_outer_open]
op1=Close
category=gate_outer_open

[scan_leak_main]
op1=Open
category=gate_main_closed
param=next_loc_stage,scan_leak_open

[not_in_the_pack]
op1=Open
category=gate_outer_closed
param=next_loc_stage,scan_leak_outer_open

[scan_leak_outer]
op1=Open
category=gate_outer_closed
param=next_loc_stage,scan_leak_outer_open

[scan_leak_open]
op1=Close
category=gate_main_open

[scan_leak_outer_open]
op1=Close
category=gate_outer_open

[scan_missing_main]
op1=Open
category=gate_main_closed
param=next_loc_stage,scan_missing_open

[scan_missing_outer]
op1=Open
category=gate_outer_closed
param=next_loc_stage,scan_missing_open

[scan_mismatch_main]
op1=Open
category=gate_main_closed
param=next_loc_stage,scan_mismatch_open

[scan_mismatch_outer]
op1=Open
category=gate_outer_closed
param=next_loc_stage,scan_mismatch_open

[scan_mismatch_open]
op1=Close
category=gate_outer_open

[scan_bogus_main]
op1=Open
category=gate_main_closed
param=next_loc_stage,loc_9999

[scan_bogus_outer]
op1=Open
category=gate_outer_closed
param=next_loc_stage,scan_bogus_open

[scan_bogus_open]
op1=Close
category=gate_main_open
",
        );
        fx.write(
            "maps/m44_53.jm2",
            "\
==== MAP ====
0 0 0: f0 u48

==== LOC ====
0 1 3: 5082 0 2
0 1 4: 5081 0 2
0 2 3: 5086 0 2
0 2 4: 5085 0 2
0 3 3: 5090 0 2
0 3 4: 5089 0 2
0 4 3: 5093 0 2
0 4 4: 5092 0 2
0 5 3: 5096 0 2
0 5 4: 5095 0 2
",
        );
        let defs = loc_defs(&[
            (5081, 1, 1),
            (5082, 1, 1),
            (5085, 1, 1),
            (5086, 1, 1),
            (5089, 1, 1),
            (5090, 1, 1),
            (5092, 1, 1),
            (5093, 1, 1),
            (5095, 1, 1),
            (5096, 1, 1),
        ]);
        let wc = bake_collision(&fx, &defs, &HashSet::new());
        let graph = derive_transports(fx.path(), &defs, &wc);

        assert_eq!(
            door_crossings(&graph, 5081),
            vec![
                ((2817, 3396), 'E', (2818, 3396)),
                ((2817, 3396), 'W', (2816, 3396)),
            ],
            "the valid control must still be admitted"
        );
        assert_eq!(
            door_crossings(&graph, 5085),
            vec![
                ((2818, 3396), 'E', (2819, 3396)),
                ((2818, 3396), 'W', (2817, 3396)),
            ],
            "an unresolvable header must not steal the previous member"
        );
        for (id, why) in [
            (
                5089,
                "a missing open-leaf category config must not be admitted",
            ),
            (5092, "a mismatched open-leaf category must not be admitted"),
            (
                5095,
                "a loc_N alias that is not in loc.pack must not be admitted",
            ),
        ] {
            assert!(
                door_crossings(&graph, id).is_empty(),
                "loc {id} must not be promoted: {why}"
            );
        }
    }

    #[test]
    fn inherited_gate_open_leaf_categories_fail_closed_on_conflicts_and_aliases() {
        let fx = Fixture::new();
        let ids = HashMap::from([
            ("control_main".to_string(), 6001),
            ("control_open".to_string(), 6002),
            ("duplicate_main".to_string(), 6011),
            ("duplicate_open".to_string(), 6012),
            ("conflict_main".to_string(), 6021),
            ("conflict_open".to_string(), 6022),
            ("alias_main".to_string(), 6031),
            ("alias_open".to_string(), 6032),
        ]);
        fx.write(
            "scripts/general_use/scripts/gates.rs2",
            "\
[proc,open_gate]
return;

[oploc1,_gate_main_closed] ~open_gate;
",
        );
        fx.write(
            "scripts/quests/quest_scan/configs/categories.loc",
            "\
[control_main]
op1=Open
category=gate_main_closed
param=next_loc_stage,control_open

[control_open]
category=gate_main_open

[duplicate_main]
op1=Open
category=gate_main_closed
param=next_loc_stage,duplicate_open

[duplicate_open]
category=gate_main_open

[loc_6012]
category=gate_main_open

[conflict_main]
op1=Open
category=gate_main_closed
param=next_loc_stage,conflict_open

[conflict_open]
category=gate_main_open

[conflict_open]
category=gate_outer_open

[conflict_open]
category=gate_main_open

[alias_main]
op1=Open
category=gate_main_closed
param=next_loc_stage,alias_open

[alias_open]
category=gate_main_open

[loc_6032]
category=gate_outer_open

[alias_open]
category=gate_main_open
",
        );

        let supported = generic_gate_handlers(fx.path());
        let mut skipped = HashMap::new();
        let inherited =
            inherited_closed_gates(fx.path(), &ids, &HashMap::new(), &supported, &mut skipped);

        assert_eq!(inherited.get(&6001), Some(&6002), "valid control");
        assert_eq!(
            inherited.get(&6011),
            Some(&6012),
            "repeating the same category, including through loc_N, remains valid"
        );
        assert_eq!(
            inherited.get(&6021),
            None,
            "a differing category permanently conflicts the named open leaf"
        );
        assert_eq!(
            inherited.get(&6031),
            None,
            "a differing loc_N alias permanently conflicts the same open leaf id"
        );
    }

    fn write_magicguild_source(fx: &Fixture, script: &str) {
        fx.write(
            "pack/loc.pack",
            "\
1600=magicguild_door_l
1601=magicguild_door_r
1522=loc_1522
1523=loc_1523
",
        );
        fx.write(
            "scripts/areas/area_yanille/configs/magic_guild/magic_guild.loc",
            "\
[magicguild_door_l]
name=Magic guild door
desc=The doors to the Magic Guild.
model=castle_doubledoorl
op1=Open
raiseobject=no
category=double_door_open_and_close_left
param=next_loc_stage,loc_1522
param=open_sound,null

[magicguild_door_r]
name=Magic guild door
desc=The doors to the Magic guild.
model=castle_doubledoorl
op1=Open
mirror=yes
raiseobject=no
category=double_door_open_and_close_right
param=next_loc_stage,loc_1523
param=open_sound,null
",
        );
        fx.write("scripts/areas/area_yanille/scripts/magic_guild.rs2", script);
    }

    fn magicguild_opener_script() -> &'static str {
        "\
[oploc1,magicguild_door_l] @open_mageguild_door(^left);
[oploc1,magicguild_door_r] @open_mageguild_door(^right);

[label,open_mageguild_door](int $side)
def_boolean $entering = ~check_axis_locactive(coord);
if($entering = true & stat(magic) < 66) {
    if(npc_find(coord, guild_wizard, 14, 0) = true) {
        ~chatnpc(\"<p,neutral>You need a magic level of 66.|The magical energy in here is unsafe for those below that level.\");
    }
    return;
}
~open_and_close_double_door2($entering, $side, door_open);
"
    }

    fn write_magicguild_yanille_placements(fx: &Fixture) {
        fx.write(
            "maps/m40_48.jm2",
            "\
==== MAP ====
0 23 15: h1 o6 u50
0 23 16: h1 o6 u50
0 24 15: h1 o6 u50
0 24 16: h1 o6 u50
0 25 15: h1 o6 u50
0 25 16: h1 o6 u50
0 36 15: h1 o6 u50
0 36 16: h1 o6 u50
0 37 15: h1 o6 u50
0 37 16: h1 o6 u50
0 38 15: h1 o6 u50
0 38 16: h1 o6 u50

==== LOC ====
0 24 15: 1601 0 2
0 24 16: 1600 0 2
0 37 15: 1600 0
0 37 16: 1601 0
",
        );
    }

    fn magic_state(level: i32) -> crate::world_state::WorldState {
        crate::world_state::WorldState {
            stats: HashMap::from([(SKILL_MAGIC, level)]),
            ..crate::world_state::WorldState::empty()
        }
    }

    fn derive_from_lostcity_content() -> Option<&'static (TransportGraph, WorldCollision)> {
        static CELL: std::sync::OnceLock<Option<(TransportGraph, WorldCollision)>> =
            std::sync::OnceLock::new();
        CELL.get_or_init(|| {
            let root = PathBuf::from("/Users/acfrazier/experiments/lostcity-289/content");
            if !root.join("maps").is_dir() || !root.join("pack").join("loc.pack").is_file() {
                eprintln!(
                    "SKIP: lostcity-289 content not found at {} (content-backed tests skipped)",
                    root.display()
                );
                return None;
            }
            let defs = real_loc_defs()?;
            let wc = bake_from_maps(&root.join("maps"), &defs, &HashSet::new())
                .expect("lostcity-289 content bakes");
            let graph = derive_transports(&root, &defs, &wc);
            Some((graph, wc))
        })
        .as_ref()
    }

    /// Wizard Guild stairs from `stairs.rs2` + `m40_48.jm2` must already be
    /// packed. If this fails, the Magic shop→bank hole is not door-only.
    #[test]
    fn yanille_wizard_guild_stair_pairs_exist() {
        let Some((graph, _)) = derive_from_lostcity_content() else {
            return;
        };
        let down = graph
            .edges
            .iter()
            .find(|e| {
                e.kind == TransportKind::Stairs
                    && e.loc_id == 1723
                    && e.at
                        == WorldTile {
                            x: 2590,
                            z: 3090,
                            level: 1,
                        }
            })
            .expect("1723 Climb-down at 2590,3090,1");
        assert_eq!(
            down.to,
            WorldTile {
                x: 2590,
                z: 3088,
                level: 0
            },
            "0_40_48_30_16 landing"
        );
        let up = graph
            .edges
            .iter()
            .find(|e| {
                e.kind == TransportKind::Stairs
                    && e.loc_id == 1722
                    && e.at
                        == WorldTile {
                            x: 2590,
                            z: 3089,
                            level: 0,
                        }
            })
            .expect("1722 Climb-up at 2590,3089,0");
        assert_eq!(
            up.to,
            WorldTile {
                x: 2590,
                z: 3092,
                level: 1
            },
            "1_40_48_30_20 landing"
        );
    }

    /// Named-override guild doors are not inherited gates. The opener in
    /// `magic_guild.rs2` admits `Open` hops at the four map tiles, and
    /// `stat(magic) < 66` applies only on the entering (`check_axis` /
    /// `door_open`) crossing.
    #[test]
    fn derive_transports_emits_magicguild_door_crossings() {
        let fx = Fixture::new();
        write_magicguild_source(&fx, magicguild_opener_script());
        write_magicguild_yanille_placements(&fx);
        let defs = loc_defs(&[(1600, 1, 1), (1601, 1, 1), (1522, 1, 1), (1523, 1, 1)]);
        let wc = bake_collision(&fx, &defs, &HashSet::from([1600, 1601]));
        let graph = derive_transports(fx.path(), &defs, &wc);
        assert_eq!(
            door_crossings(&graph, 1600),
            vec![
                ((2584, 3088), 'E', (2585, 3088)),
                ((2584, 3088), 'W', (2583, 3088)),
                ((2597, 3087), 'E', (2598, 3087)),
                ((2597, 3087), 'W', (2596, 3087)),
            ],
            "1600 at the west and east guild doors"
        );
        assert_eq!(
            door_crossings(&graph, 1601),
            vec![
                ((2584, 3087), 'E', (2585, 3087)),
                ((2584, 3087), 'W', (2583, 3087)),
                ((2597, 3088), 'E', (2598, 3088)),
                ((2597, 3088), 'W', (2596, 3088)),
            ],
            "1601 paired with 1600"
        );
        for e in graph
            .edges
            .iter()
            .filter(|e| matches!(e.loc_id, 1600 | 1601))
        {
            assert_eq!(e.kind, TransportKind::Door, "{e:?}");
            assert_eq!(e.option, 1, "stock Open {e:?}");
            assert_eq!(
                e.open_loc_id,
                Some(if e.loc_id == 1600 { 1522 } else { 1523 }),
                "{e:?}"
            );
            assert!(
                e.item_req.is_empty()
                    && e.quest_req.is_empty()
                    && e.varp_req.is_empty()
                    && e.worn_req.is_empty()
                    && !e.members_req,
                "{e:?}"
            );
            let entering = match (e.at.x, e.dir) {
                (2584, Some(DoorDir::E)) | (2597, Some(DoorDir::W)) => true,
                (2584, Some(DoorDir::W)) | (2597, Some(DoorDir::E)) => false,
                other => panic!("unexpected magicguild crossing {other:?}"),
            };
            if entering {
                assert_eq!(e.skill_req, vec![(SKILL_MAGIC, 66)], "enter {e:?}");
            } else {
                assert!(e.skill_req.is_empty(), "exit must stay ungated {e:?}");
            }
        }
    }

    /// A missing named opener must not invent skill-gated hops.
    #[test]
    fn derive_transports_omits_magicguild_doors_without_named_opener() {
        let fx = Fixture::new();
        write_magicguild_source(&fx, "");
        write_magicguild_yanille_placements(&fx);
        let defs = loc_defs(&[(1600, 1, 1), (1601, 1, 1), (1522, 1, 1), (1523, 1, 1)]);
        let wc = bake_collision(&fx, &defs, &HashSet::from([1600, 1601]));
        let graph = derive_transports(fx.path(), &defs, &wc);
        assert!(
            door_crossings(&graph, 1600).is_empty() && door_crossings(&graph, 1601).is_empty(),
            "no named override → no magicguild edges"
        );
    }

    /// WorldState magic gates entering only. Exiting a sealed corridor is
    /// free; entering with magic 65 stays NoPath.
    #[test]
    fn magicguild_door_eligibility_follows_entering_axis() {
        use crate::router::{find_with, FindOptions, Leg, RouteError};
        let fx = Fixture::new();
        write_magicguild_source(&fx, magicguild_opener_script());
        let mut walk = Vec::new();
        for x in 2580..=2583 {
            walk.push((x, 3088));
        }
        for x in 2585..=2588 {
            walk.push((x, 3088));
        }
        write_blocked_square(&fx, 40, 48, &walk, "0 24 16: 1600 0 2\n0 24 15: 1601 0 2\n");
        let defs = loc_defs(&[(1600, 1, 1), (1601, 1, 1), (1522, 1, 1), (1523, 1, 1)]);
        let wc = bake_collision(&fx, &defs, &HashSet::from([1600, 1601]));
        let graph = derive_transports(fx.path(), &defs, &wc);
        let outside = WorldTile {
            x: 2583,
            z: 3088,
            level: 0,
        };
        let inside = WorldTile {
            x: 2585,
            z: 3088,
            level: 0,
        };
        let empty = crate::world_state::WorldState::empty();
        let low = magic_state(65);
        let ok = magic_state(66);
        assert!(
            matches!(
                find_with(&wc, &graph, outside, inside, FindOptions::default(), &empty),
                Err(RouteError::NoPath)
            ),
            "empty stats cannot enter"
        );
        assert!(
            matches!(
                find_with(&wc, &graph, outside, inside, FindOptions::default(), &low),
                Err(RouteError::NoPath)
            ),
            "magic 65 cannot enter"
        );
        let enter = find_with(&wc, &graph, outside, inside, FindOptions::default(), &ok)
            .expect("magic 66 enters");
        assert!(
            enter.legs.iter().any(|l| matches!(
                l,
                Leg::Transport { edge }
                    if edge.loc_id == 1600
                        && edge.dir == Some(DoorDir::E)
                        && edge.skill_req == vec![(SKILL_MAGIC, 66)]
            )),
            "enter hops 1600 E with the parsed magic gate: {enter:?}"
        );
        let exit = find_with(&wc, &graph, inside, outside, FindOptions::default(), &empty)
            .expect("exit does not need magic");
        assert!(
            exit.legs.iter().any(|l| matches!(
                l,
                Leg::Transport { edge }
                    if edge.loc_id == 1600
                        && edge.dir == Some(DoorDir::W)
                        && edge.skill_req.is_empty()
            )),
            "exit hops 1600 W ungated: {exit:?}"
        );
    }

    /// Live hole: floor-1 shop → Yanille bank is NoPath until the guild
    /// doors join. After import, exiting stays eligible without magic;
    /// entering the shop from the bank requires magic 66.
    #[test]
    fn magic_guild_shop_bank_route_uses_derived_doors() {
        use crate::router::{find_with, FindOptions, Leg, RouteError};
        let Some((graph, wc)) = derive_from_lostcity_content() else {
            return;
        };
        let shop = WorldTile {
            x: 2594,
            z: 3090,
            level: 1,
        };
        let bank = WorldTile {
            x: 2613,
            z: 3092,
            level: 0,
        };
        let doors: Vec<_> = graph
            .edges
            .iter()
            .filter(|e| e.kind == TransportKind::Door && matches!(e.loc_id, 1600 | 1601))
            .collect();
        assert_eq!(
            doors.len(),
            8,
            "four guild door tiles × two crossings, got {doors:?}"
        );
        let empty = crate::world_state::WorldState::empty();
        let low = magic_state(65);
        let ok = magic_state(66);
        let leave = find_with(wc, graph, shop, bank, FindOptions::default(), &empty)
            .unwrap_or_else(|e| panic!("shop → bank must exit without a magic gate ({e:?})"));
        assert_eq!(leave.dest, bank);
        assert!(
            leave.legs.iter().any(|l| matches!(
                l,
                Leg::Transport { edge } if edge.loc_id == 1723
            )),
            "shop → bank climbs down 1723: {leave:?}"
        );
        assert!(
            leave.legs.iter().any(|l| matches!(
                l,
                Leg::Transport { edge }
                    if matches!(edge.loc_id, 1600 | 1601) && edge.skill_req.is_empty()
            )),
            "shop → bank exits an ungated guild door: {leave:?}"
        );
        assert!(
            matches!(
                find_with(wc, graph, bank, shop, FindOptions::default(), &empty),
                Err(RouteError::NoPath)
            ),
            "empty stats cannot enter the guild"
        );
        assert!(
            matches!(
                find_with(wc, graph, bank, shop, FindOptions::default(), &low),
                Err(RouteError::NoPath)
            ),
            "magic 65 cannot enter the guild"
        );
        let enter = find_with(wc, graph, bank, shop, FindOptions::default(), &ok)
            .unwrap_or_else(|e| panic!("bank → shop with magic 66 ({e:?})"));
        assert_eq!(enter.dest, shop);
        assert!(
            enter.legs.iter().any(|l| matches!(
                l,
                Leg::Transport { edge }
                    if matches!(edge.loc_id, 1600 | 1601)
                        && edge.skill_req == vec![(SKILL_MAGIC, 66)]
            )),
            "bank → shop enters a magic-gated door: {enter:?}"
        );
        assert!(
            enter.legs.iter().any(|l| matches!(
                l,
                Leg::Transport { edge } if edge.loc_id == 1722
            )),
            "bank → shop climbs 1722: {enter:?}"
        );
    }

    const RANGINGGUILD_OUTSIDE: WorldTile = WorldTile {
        x: 2657,
        z: 3439,
        level: 0,
    };
    const RANGINGGUILD_INSIDE: WorldTile = WorldTile {
        x: 2659,
        z: 3437,
        level: 0,
    };
    const RANGINGGUILD_LOC: WorldTile = WorldTile {
        x: 2658,
        z: 3438,
        level: 0,
    };

    fn write_rangingguild_source(fx: &Fixture, script: &str) {
        fx.write(
            "pack/loc.pack",
            "\
2514=ranging_guild_door
1532=loc_1532
",
        );
        fx.write(
            "scripts/minigames/game_ranging/configs/ranging.loc",
            "\
[ranging_guild_door]
name=Guild door
desc=The door to the Ranging Guild.
model=basic_wall
active=yes
op1=Open
",
        );
        fx.write(
            "scripts/minigames/game_ranging/scripts/ranging_guild_door.rs2",
            script,
        );
    }

    fn write_rangingguild_placement(fx: &Fixture, loc_lines: &str) {
        fx.write(
            "maps/m41_53.jm2",
            &format!(
                "\
==== MAP ====
0 33 47: h1 u50
0 34 46: h1 u50
0 35 45: h1 u50

==== LOC ====
{loc_lines}
"
            ),
        );
    }

    fn rangingguild_opener_script() -> &'static str {
        "\
[oploc1,ranging_guild_door]
if(coordx(coord) > coordx(loc_coord) | coordz(coord) < coordz(loc_coord)) {
    sound_synth(door_open, 1, 0);
    ~forcemove(movecoord(loc_coord, 1, 0, -1));
    loc_change(inviswall, 3);
    loc_add(movecoord($loc_coord, $x, 0, $z), loc_1532, modulo(add($angle, 1), 4), $shape, 3);
    p_teleport(movecoord(coord, -2, 0, 2));
    return;
}
if(stat(ranged) < 40) {
    return;
}
sound_synth(door_open, 1, 0);
~forcemove(movecoord(loc_coord, -1, 0, 1));
loc_change(inviswall, 3);
loc_add(movecoord($loc_coord, $x, 0, $z), loc_1532, modulo(add($angle, 1), 4), $shape, 3);
p_teleport(movecoord(coord, 2, 0, -2));
"
    }

    fn ranging_state(level: i32) -> crate::world_state::WorldState {
        crate::world_state::WorldState {
            stats: HashMap::from([(SKILL_RANGED, level)]),
            ..crate::world_state::WorldState::empty()
        }
    }

    fn rangingguild_doors(graph: &TransportGraph) -> Vec<&TransportEdge> {
        graph
            .edges
            .iter()
            .filter(|e| e.kind == TransportKind::Door && e.loc_id == 2514)
            .collect()
    }

    fn rangingguild_usable_from<'a>(
        graph: &'a TransportGraph,
        stand: WorldTile,
    ) -> Vec<&'a TransportEdge> {
        rangingguild_doors(graph)
            .into_iter()
            .filter(|e| {
                e.at.level == stand.level
                    && (e.at.x - stand.x).abs().max((e.at.z - stand.z).abs()) <= 1
            })
            .collect()
    }

    fn assert_rangingguild_records(graph: &TransportGraph) {
        let doors = rangingguild_doors(graph);
        assert_eq!(doors.len(), 2, "exactly the reciprocal pair: {doors:?}");
        let enter = doors
            .iter()
            .find(|e| e.at == RANGINGGUILD_OUTSIDE && e.to == RANGINGGUILD_INSIDE)
            .unwrap_or_else(|| panic!("enter stand→landing missing: {doors:?}"));
        let exit = doors
            .iter()
            .find(|e| e.at == RANGINGGUILD_INSIDE && e.to == RANGINGGUILD_OUTSIDE)
            .unwrap_or_else(|| panic!("exit stand→landing missing: {doors:?}"));
        for (e, skill) in [
            (*enter, vec![(SKILL_RANGED, 40)]),
            (*exit, Vec::<(i32, i32)>::new()),
        ] {
            assert_eq!(e.option, 1, "{e:?}");
            assert_eq!(e.ticks, 1, "{e:?}");
            assert_eq!(e.dir, None, "{e:?}");
            assert_eq!(e.open_loc_id, None, "visual loc_1532 is not a leaf {e:?}");
            assert_eq!(e.skill_req, skill, "{e:?}");
            assert!(
                e.item_req.is_empty()
                    && e.quest_req.is_empty()
                    && e.varp_req.is_empty()
                    && e.worn_req.is_empty()
                    && !e.members_req,
                "{e:?}"
            );
        }
        assert_ne!(
            enter.at, RANGINGGUILD_LOC,
            "at must be the origin stand, not the loc tile"
        );
        assert_ne!(exit.at, RANGINGGUILD_LOC);
    }

    fn derive_rangingguild_fixture(script: &str, loc_lines: &str) -> TransportGraph {
        let fx = Fixture::new();
        write_rangingguild_source(&fx, script);
        write_rangingguild_placement(&fx, loc_lines);
        let defs = loc_defs(&[(2514, 1, 1), (1532, 1, 1)]);
        let wc = bake_collision(&fx, &defs, &HashSet::from([2514]));
        derive_transports(fx.path(), &defs, &wc)
    }

    /// Named-override diagonal wall: origin stands are the script
    /// forcemove tiles, landings are the coord-relative teleports, enter
    /// only carries Ranged 40, and loc_1532 stays a visual.
    #[test]
    fn derive_transports_emits_rangingguild_door_stand_teleport_pair() {
        let graph = derive_rangingguild_fixture(rangingguild_opener_script(), "0 34 46: 2514 9\n");
        assert_rangingguild_records(&graph);
    }

    /// Missing opener, missing/moved 40-check, inverted half-plane,
    /// changed or swapped offset pairs, commented-out required forms,
    /// extra contradictory calls, swapped forcemove/teleport order,
    /// early exit return, a bare enter 40-check, a return between the
    /// enter calls, shape 0, angle drift, or a second level-0 2514 must
    /// emit nothing. Player-relative `p_teleport` cannot invent a `to`
    /// the way [`parse_landing`] skips it.
    #[test]
    fn derive_transports_omits_unproven_rangingguild_door_forms() {
        let canonical = rangingguild_opener_script();
        let placement = "0 34 46: 2514 9\n";
        let cases = [
            ("missing opener", "", placement),
            (
                "missing ranged gate",
                &canonical.replace("if(stat(ranged) < 40) {\n    return;\n}\n", ""),
                placement,
            ),
            (
                "gate on exit",
                "\
[oploc1,ranging_guild_door]
if(coordx(coord) > coordx(loc_coord) | coordz(coord) < coordz(loc_coord)) {
    if(stat(ranged) < 40) {
        return;
    }
    ~forcemove(movecoord(loc_coord, 1, 0, -1));
    p_teleport(movecoord(coord, -2, 0, 2));
    return;
}
~forcemove(movecoord(loc_coord, -1, 0, 1));
p_teleport(movecoord(coord, 2, 0, -2));
",
                placement,
            ),
            (
                "inverted half-plane",
                &canonical.replace(
                    "coordx(coord) > coordx(loc_coord) | coordz(coord) < coordz(loc_coord)",
                    "coordx(coord) < coordx(loc_coord) | coordz(coord) > coordz(loc_coord)",
                ),
                placement,
            ),
            (
                "swapped forcemove offsets",
                &canonical
                    .replace("movecoord(loc_coord, 1, 0, -1)", "TMP_EXIT_FORCEMOVE")
                    .replace(
                        "movecoord(loc_coord, -1, 0, 1)",
                        "movecoord(loc_coord, 1, 0, -1)",
                    )
                    .replace("TMP_EXIT_FORCEMOVE", "movecoord(loc_coord, -1, 0, 1)"),
                placement,
            ),
            (
                "changed teleport offsets",
                &canonical.replace(
                    "p_teleport(movecoord(coord, -2, 0, 2))",
                    "p_teleport(movecoord(coord, 2, 0, -2))",
                ),
                placement,
            ),
            (
                "commented forcemove",
                &canonical.replace(
                    "~forcemove(movecoord(loc_coord, 1, 0, -1));",
                    "// ~forcemove(movecoord(loc_coord, 1, 0, -1));",
                ),
                placement,
            ),
            (
                "commented ranged gate",
                &canonical.replace(
                    "if(stat(ranged) < 40) {\n    return;\n}",
                    "// if(stat(ranged) < 40) {\n//     return;\n// }",
                ),
                placement,
            ),
            (
                "additional contradictory teleport",
                &canonical.replace(
                    "p_teleport(movecoord(coord, -2, 0, 2));",
                    "p_teleport(movecoord(coord, -2, 0, 2));\n    p_teleport(movecoord(coord, 2, 0, -2));",
                ),
                placement,
            ),
            (
                "player-relative landing without loc forcemove",
                "\
[oploc1,ranging_guild_door]
if(coordx(coord) > coordx(loc_coord) | coordz(coord) < coordz(loc_coord)) {
    p_teleport(movecoord(coord, -2, 0, 2));
    return;
}
if(stat(ranged) < 40) {
    return;
}
p_teleport(movecoord(coord, 2, 0, -2));
",
                placement,
            ),
            (
                "exit teleport before forcemove",
                "\
[oploc1,ranging_guild_door]
if(coordx(coord) > coordx(loc_coord) | coordz(coord) < coordz(loc_coord)) {
    p_teleport(movecoord(coord, -2, 0, 2));
    ~forcemove(movecoord(loc_coord, 1, 0, -1));
    return;
}
if(stat(ranged) < 40) {
    return;
}
~forcemove(movecoord(loc_coord, -1, 0, 1));
p_teleport(movecoord(coord, 2, 0, -2));
",
                placement,
            ),
            (
                "enter teleport before forcemove",
                "\
[oploc1,ranging_guild_door]
if(coordx(coord) > coordx(loc_coord) | coordz(coord) < coordz(loc_coord)) {
    ~forcemove(movecoord(loc_coord, 1, 0, -1));
    p_teleport(movecoord(coord, -2, 0, 2));
    return;
}
if(stat(ranged) < 40) {
    return;
}
p_teleport(movecoord(coord, 2, 0, -2));
~forcemove(movecoord(loc_coord, -1, 0, 1));
",
                placement,
            ),
            (
                "exit return before crossing",
                "\
[oploc1,ranging_guild_door]
if(coordx(coord) > coordx(loc_coord) | coordz(coord) < coordz(loc_coord)) {
    return;
    ~forcemove(movecoord(loc_coord, 1, 0, -1));
    p_teleport(movecoord(coord, -2, 0, 2));
    return;
}
if(stat(ranged) < 40) {
    return;
}
~forcemove(movecoord(loc_coord, -1, 0, 1));
p_teleport(movecoord(coord, 2, 0, -2));
",
                placement,
            ),
            (
                "enter ranged check without if-return gate",
                "\
[oploc1,ranging_guild_door]
if(coordx(coord) > coordx(loc_coord) | coordz(coord) < coordz(loc_coord)) {
    ~forcemove(movecoord(loc_coord, 1, 0, -1));
    p_teleport(movecoord(coord, -2, 0, 2));
    return;
}
stat(ranged)<40
~forcemove(movecoord(loc_coord, -1, 0, 1));
p_teleport(movecoord(coord, 2, 0, -2));
",
                placement,
            ),
            (
                "enter return between forcemove and teleport",
                "\
[oploc1,ranging_guild_door]
if(coordx(coord) > coordx(loc_coord) | coordz(coord) < coordz(loc_coord)) {
    ~forcemove(movecoord(loc_coord, 1, 0, -1));
    p_teleport(movecoord(coord, -2, 0, 2));
    return;
}
if(stat(ranged) < 40) {
    return;
}
~forcemove(movecoord(loc_coord, -1, 0, 1));
return;
p_teleport(movecoord(coord, 2, 0, -2));
",
                placement,
            ),
            ("shape 0", canonical, "0 34 46: 2514 0\n"),
            ("angle change", canonical, "0 34 46: 2514 9 1\n"),
            (
                "duplicate level-0 placement",
                canonical,
                "0 34 46: 2514 9\n0 35 45: 2514 9\n",
            ),
        ];
        for (label, script, loc_lines) in cases {
            let graph = derive_rangingguild_fixture(script, loc_lines);
            assert!(
                rangingguild_doors(&graph).is_empty(),
                "{label} must omit 2514, got {:?}",
                rangingguild_doors(&graph)
            );
        }
    }

    /// Reciprocal stands are Chebyshev 2 apart, so radius 1 admits only
    /// the hop whose `at` is this stand. A shared `at=loc` pair would
    /// expose both hops from either landing.
    #[test]
    fn rangingguild_door_radius1_admits_only_the_reciprocal_stand() {
        let graph = derive_rangingguild_fixture(rangingguild_opener_script(), "0 34 46: 2514 9\n");
        assert_rangingguild_records(&graph);
        let from_outside = rangingguild_usable_from(&graph, RANGINGGUILD_OUTSIDE);
        assert_eq!(from_outside.len(), 1, "{from_outside:?}");
        assert_eq!(from_outside[0].at, RANGINGGUILD_OUTSIDE);
        assert_eq!(from_outside[0].to, RANGINGGUILD_INSIDE);
        assert_eq!(from_outside[0].skill_req, vec![(SKILL_RANGED, 40)]);
        let from_inside = rangingguild_usable_from(&graph, RANGINGGUILD_INSIDE);
        assert_eq!(from_inside.len(), 1, "{from_inside:?}");
        assert_eq!(from_inside[0].at, RANGINGGUILD_INSIDE);
        assert_eq!(from_inside[0].to, RANGINGGUILD_OUTSIDE);
        assert!(from_inside[0].skill_req.is_empty());
        assert_eq!(
            (RANGINGGUILD_OUTSIDE.x - RANGINGGUILD_INSIDE.x)
                .abs()
                .max((RANGINGGUILD_OUTSIDE.z - RANGINGGUILD_INSIDE.z).abs()),
            2,
            "stands must stay outside INTERACT_RADIUS 1 of each other"
        );
    }

    #[test]
    fn derive_transports_rangingguild_door_pair_from_real_content() {
        let Some((graph, _)) = derive_from_lostcity_content() else {
            return;
        };
        assert_rangingguild_records(graph);
        let from_outside = rangingguild_usable_from(graph, RANGINGGUILD_OUTSIDE);
        assert_eq!(from_outside.len(), 1, "{from_outside:?}");
        assert_eq!(from_outside[0].skill_req, vec![(SKILL_RANGED, 40)]);
        let from_inside = rangingguild_usable_from(graph, RANGINGGUILD_INSIDE);
        assert_eq!(from_inside.len(), 1, "{from_inside:?}");
        assert!(from_inside[0].skill_req.is_empty());
    }

    /// Graph evidence only: Seers → JUDGE_STAND enters through 2514 at
    /// the outside stand when Ranged is 70; empty / 39 stay NoPath.
    #[test]
    fn ranging_guild_seers_judge_route_uses_derived_door() {
        use crate::router::{find_with, FindOptions, Leg, RouteError};
        let Some((graph, wc)) = derive_from_lostcity_content() else {
            return;
        };
        let seers = WorldTile {
            x: 2722,
            z: 3493,
            level: 0,
        };
        let judge = WorldTile {
            x: 2670,
            z: 3418,
            level: 0,
        };
        let opts = FindOptions {
            allow_wilderness: true,
            allow_teleports: false,
            ..FindOptions::default()
        };
        let empty = crate::world_state::WorldState::empty();
        let low = ranging_state(39);
        let ok = ranging_state(70);
        assert!(
            matches!(
                find_with(wc, graph, seers, judge, opts, &empty),
                Err(RouteError::NoPath)
            ),
            "empty stats cannot enter"
        );
        assert!(
            matches!(
                find_with(wc, graph, seers, judge, opts, &low),
                Err(RouteError::NoPath)
            ),
            "ranged 39 cannot enter"
        );
        let enter = find_with(wc, graph, seers, judge, opts, &ok)
            .unwrap_or_else(|e| panic!("ranged 70 Seers → judge ({e:?})"));
        assert_eq!(enter.dest, judge);
        assert!(
            enter.legs.iter().any(|l| matches!(
                l,
                Leg::Transport { edge }
                    if edge.loc_id == 2514
                        && edge.at == RANGINGGUILD_OUTSIDE
                        && edge.to == RANGINGGUILD_INSIDE
                        && edge.skill_req == vec![(SKILL_RANGED, 40)]
                        && edge.dir.is_none()
                        && edge.open_loc_id.is_none()
            )),
            "enter hops 2514 at the outside stand: {enter:?}"
        );
    }

    fn skip_total(skipped: &HashMap<&'static str, usize>, reason: &str) -> usize {
        *skipped.get(reason).unwrap_or(&0)
    }

    /// N1: missing required `ranging.loc` must bump, not silently omit 2514.
    #[test]
    fn n1_ranging_missing_loc_increments_skip_without_2514_edges() {
        let fx = Fixture::new();
        fx.write("pack/loc.pack", "2514=ranging_guild_door\n1532=loc_1532\n");
        fx.write(
            "scripts/minigames/game_ranging/scripts/ranging_guild_door.rs2",
            rangingguild_opener_script(),
        );
        write_rangingguild_placement(&fx, "0 34 46: 2514 9\n");
        let defs = loc_defs(&[(2514, 1, 1), (1532, 1, 1)]);
        let wc = bake_collision(&fx, &defs, &HashSet::from([2514]));
        let (graph, skipped) = derive_transports_with_skips(fx.path(), &defs, &wc);
        assert!(rangingguild_doors(&graph).is_empty());
        assert_eq!(
            skip_total(&skipped, SKIP_RANGINGGUILD_CONFIG),
            RANGINGGUILD_DECLARED_PAIR,
            "one skip unit is the enter/exit pair: {skipped:?}"
        );
    }

    #[test]
    fn n1_valid_ranging_fixture_has_no_ranging_skip_reasons() {
        let fx = Fixture::new();
        write_rangingguild_source(&fx, rangingguild_opener_script());
        write_rangingguild_placement(&fx, "0 34 46: 2514 9\n");
        let defs = loc_defs(&[(2514, 1, 1), (1532, 1, 1)]);
        let wc = bake_collision(&fx, &defs, &HashSet::from([2514]));
        let (graph, skipped) = derive_transports_with_skips(fx.path(), &defs, &wc);
        assert_rangingguild_records(&graph);
        assert_eq!(skip_total(&skipped, SKIP_RANGINGGUILD_CONFIG), 0);
        assert_eq!(skip_total(&skipped, SKIP_RANGINGGUILD_SCRIPT), 0);
        assert_eq!(skip_total(&skipped, SKIP_RANGINGGUILD_PLACEMENT), 0);
        assert_eq!(skip_total(&skipped, SKIP_RANGINGGUILD_PACK), 0);
    }

    #[test]
    fn n1_lever_missing_source_increments_declared_route_skips() {
        let fx = Fixture::new();
        fx.write("pack/loc.pack", "1814=wildinlever\n");
        let defs = loc_defs(&[(1814, 1, 1)]);
        let wc = bake_collision(&fx, &defs, &HashSet::new());
        let (graph, skipped) = derive_transports_with_skips(fx.path(), &defs, &wc);
        assert!(
            graph.edges.iter().all(|e| e.loc_id != 1814),
            "no edges for the declared lever loc without source"
        );
        assert_eq!(
            skip_total(&skipped, SKIP_LEVER_SOURCE),
            LEVER_DECLARED_ROUTES
        );
    }

    #[test]
    fn n1_magicguild_missing_opener_increments_script_skips_not_edges() {
        let fx = Fixture::new();
        write_magicguild_source(&fx, "");
        write_magicguild_yanille_placements(&fx);
        let defs = loc_defs(&[(1600, 1, 1), (1601, 1, 1), (1522, 1, 1), (1523, 1, 1)]);
        let wc = bake_collision(&fx, &defs, &HashSet::from([1600, 1601]));
        let (graph, skipped) = derive_transports_with_skips(fx.path(), &defs, &wc);
        assert!(
            door_crossings(&graph, 1600).is_empty() && door_crossings(&graph, 1601).is_empty()
        );
        assert_eq!(
            skip_total(&skipped, SKIP_MAGICGUILD_SCRIPT),
            MAGICGUILD_DECLARED_DOORS
        );
    }

    #[test]
    fn n1_zanaris_missing_script_increments_skip_without_shed_edge() {
        let fx = Fixture::new();
        fx.write("pack/loc.pack", "2409=zanarisdoor\n");
        fx.write("pack/obj.pack", "772=dramen_staff\n");
        fx.write(
            "maps/m50_49.jm2",
            "\
==== MAP ====
0 20 56: h1 u50
==== LOC ====
0 20 56: 2409 0 0
",
        );
        let defs = loc_defs(&[(2409, 1, 1)]);
        let wc = bake_collision(&fx, &defs, &HashSet::from([2409]));
        let (graph, skipped) = derive_transports_with_skips(fx.path(), &defs, &wc);
        assert!(
            graph.edges.iter().all(|e| e.worn_req != vec![772]),
            "no dramen-gated shed hop"
        );
        assert_eq!(
            skip_total(&skipped, SKIP_ZANARIS_SOURCE),
            ZANARIS_DECLARED_ROUTES
        );
    }

    #[test]
    fn n1_toll_missing_border_gate_config_skips_gates_and_henge_when_packed() {
        let fx = Fixture::new();
        fx.write("pack/obj.pack", "995=coins\n1854=shantay_pass\n");
        fx.write("pack/loc.pack", "4031=shantay_pass_henge_doorway\n");
        fx.write(
            "maps/m51_48.jm2",
            "\
==== MAP ====
0 38 44: h1 u50
==== LOC ====
0 38 44: 4031 10 0
",
        );
        let defs = loc_defs(&[(4031, 1, 1)]);
        let wc = bake_collision(&fx, &defs, &HashSet::from([4031]));
        let (graph, skipped) = derive_transports_with_skips(fx.path(), &defs, &wc);
        assert!(
            graph
                .edges
                .iter()
                .all(|e| e.loc_id != 4031 && e.item_req != vec![(995, AL_KHARID_TOLL_COINS)]),
            "no toll or henge hops without border_gate.loc"
        );
        assert_eq!(skip_total(&skipped, SKIP_TOLL_CONFIG), TOLL_BORDER_GATES);
        assert_eq!(skip_total(&skipped, SKIP_TOLL_HENGE), 1);
    }
}
