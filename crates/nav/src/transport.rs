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
mod brass_key;
mod condparse;
mod doors;
mod gates;
mod gliders;
mod index;
mod levers;
mod magic_guild;
mod membergate;
mod npc_hops;
mod quest_doors;
mod ranging_guild;
mod script_text;
mod shortcuts;
mod spirit_trees;
mod static_routes;
mod teleports;
mod toll;
mod vertical;
mod webs;
mod zanaris;

use brass_key::*;
use condparse::{
    arm_opens_directly, body_labels, check_axis_def, check_axis_or_proc, if_head_and_arm,
    proc_bitfield_varp, proc_bodies, script_blocks, script_header, script_varp_gate,
    top_level_statements,
};
use doors::*;
use gates::*;
use gliders::*;
use index::*;
pub(crate) use index::{loc_ids_by_name, loc_positions, Placement};
use levers::*;
use magic_guild::*;
use membergate::*;
use npc_hops::*;
use quest_doors::*;
use ranging_guild::*;
use script_text::*;
use shortcuts::*;
use spirit_trees::*;
use static_routes::*;
use teleports::*;
use toll::*;
use vertical::*;
use webs::*;
use zanaris::*;

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
    island_rope_edges(
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

    // The derivers visit hash sets and directory listings whose order varies
    // per run and per filesystem. A revision's pack must be a fixed point of
    // its inputs, so edges are put in a canonical order before indexing.
    graph.edges.sort_by(edge_order);
    graph.teleports.sort_by(edge_order);
    for (i, e) in graph.edges.iter().enumerate() {
        graph.at.entry(e.at).or_default().push(i);
    }

    (graph, skipped)
}

/// Canonical pack order: by kind, loc and interact tile, then by every other
/// packed field. Jewellery rubs and spirit-tree destinations are the
/// exception: their relative order is the script's dialog choice order that
/// the traveller answers by position (`dest_dialog_choice`), so a sibling
/// family compares equal and the stable sort keeps the emitted order.
fn edge_order(a: &TransportEdge, b: &TransportEdge) -> std::cmp::Ordering {
    fn tile(t: WorldTile) -> (i32, i32, i32) {
        (t.level, t.x, t.z)
    }
    fn rest(e: &TransportEdge) -> impl Ord + '_ {
        (
            tile(e.to),
            (e.option, e.ticks, e.dir.map(|d| d as u8), e.open_loc_id),
            (&e.skill_req, &e.item_req, &e.quest_req),
            (&e.varp_req, &e.worn_req, e.members_req),
        )
    }
    let family = (a.kind as u8, a.loc_id, tile(a.at)).cmp(&(b.kind as u8, b.loc_id, tile(b.at)));
    let dialog_ordered = match a.kind {
        TransportKind::Teleport => a.loc_id > 0,
        TransportKind::SpiritTree => true,
        _ => false,
    };
    if family.is_ne() || dialog_ordered {
        return family;
    }
    rest(a).cmp(&rest(b))
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
const SKIP_TOLL_GATE: &str = "Al Kharid border toll gate placement did not derive crossings";
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
const SKIP_ISLAND_ROPE_CONFIG: &str = "shortcuts.loc did not load or admit island_rope_swing";
const SKIP_ISLAND_ROPE_PARAMS: &str = "island_rope_swing block missing start_coord or end_coord";
const SKIP_ISLAND_ROPE_JOIN: &str = "island_rope_swing placement does not join start_coord";
/// Category leaf whose handler omits the agility-10 gate (`loc_type ! tree_ropeswing2`).
const ISLAND_ROPE_RETURN: &str = "tree_ropeswing2";
const ISLAND_ROPE_CATEGORY: &str = "island_rope_swing";

/// Pack names for wilderness lever `[oploc1,*]` blocks (one skip unit each).
const WILDERNESS_LEVER_LOC_NAMES: &[&str] = &["wildinlever", "wildoutlever"];
/// Pack names for Al Kharid border toll gates (one skip unit each).
const TOLL_GATE_LOC_NAMES: &[&str] = &["border_gate_toll_left", "border_gate_toll_right"];
const SHANTAY_HENGE_LOC_NAME: &str = "shantay_pass_henge_doorway";
/// One reciprocal enter/exit pair for ranging guild door 2514.
const RANGINGGUILD_DECLARED_PAIR: usize = 1;
/// One Zanaris shed `[oploc1,zanarisdoor]` hop (per level-0 wall placement).
const ZANARIS_DECLARED_ROUTES: usize = 1;

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
    // island_rope_swing: curated extra=1 (ticks=2), fullstyle exactmove
    // analogue. Source does not publish a nav extra; unmeasured.
    ("tree_ropeswing1", 1),
    ("tree_ropeswing2", 1),
    ("tree_ropeswing3", 1),
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

/// How many declared producer loc names are present in the selected pack.
fn packed_declared_names(ids: &HashMap<String, i32>, names: &[&str]) -> usize {
    names.iter().filter(|name| ids.contains_key(**name)).count()
}

fn wilderness_lever_applicable(ids: &HashMap<String, i32>) -> usize {
    packed_declared_names(ids, WILDERNESS_LEVER_LOC_NAMES)
}

fn toll_gate_applicable(ids: &HashMap<String, i32>) -> usize {
    packed_declared_names(ids, TOLL_GATE_LOC_NAMES)
}

fn toll_applicable_route_count(gate_applicable: usize, henge_applicable: bool) -> usize {
    gate_applicable + usize::from(henge_applicable)
}

/// One skip per unique packed declared loc id that is not in `admitted`.
/// Duplicate pack names for the same id are not counted twice.
fn bump_unadmitted_packed_ids(
    skipped: &mut HashMap<&'static str, usize>,
    reason: &'static str,
    ids: &HashMap<String, i32>,
    names: &[&str],
    admitted: &HashSet<i32>,
) {
    let mut seen = HashSet::new();
    for name in names {
        let Some(&id) = ids.get(*name) else {
            continue;
        };
        if !seen.insert(id) {
            continue;
        }
        if !admitted.contains(&id) {
            bump(skipped, reason, 1);
        }
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
// Doors.
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Al Kharid border toll and the Shantay northbound hop (item-gated gates).
// ---------------------------------------------------------------------------

/// The Shantay henge doorway loc id (`shantay_pass_henge_doorway` in
/// `pack/loc.pack`): the two loc-4031 Door edges both interact it, and
/// the traveller drives the gated branch's pass-handover chat dialogs
/// for this loc (see [`crate::traveller`]).
pub(crate) const SHANTAY_HENGE_LOC_ID: i32 = 4031;

#[cfg(test)]
#[path = "transport_tests.rs"]
mod tests;
