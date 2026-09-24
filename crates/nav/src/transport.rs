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

/// Directed `island_rope_swing` hops from `shortcuts.loc` `start_coord` /
/// `end_coord`. Packed `at` is the standable start, not the loc origin.
/// Absolute params only emit when a placement footprint joins that start
/// — a far mapsquare copy cannot invent a remotely usable edge.
fn island_rope_edges(
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

struct IslandRopeLeaf {
    name: String,
    start: Option<WorldTile>,
    end: Option<WorldTile>,
    width: i32,
    length: i32,
}

fn island_rope_leaves(text: &str) -> Vec<IslandRopeLeaf> {
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

fn placement_joins(loc: &Placement, start: WorldTile, width: i32, length: i32) -> bool {
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

fn emit_island_rope_leaf(
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
    });
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
    let applicable = wilderness_lever_applicable(ids);
    let Ok(script) = fs::read_to_string(dir.join("scripts").join("wilderness_lever.rs2")) else {
        bump(skipped, SKIP_LEVER_SOURCE, applicable);
        return;
    };
    let Ok(constants) = fs::read_to_string(dir.join("configs").join("wilderness_lever.constant"))
    else {
        bump(skipped, SKIP_LEVER_SOURCE, applicable);
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

    let mut visited_oploc1 = HashSet::new();
    for (op, name, body) in script_blocks(&script) {
        if op != "oploc1" {
            continue;
        }
        visited_oploc1.insert(name.clone());
        let edge_start = graph.edges.len();
        let Some(&loc_id) = ids.get(&name) else {
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
    // Packed declared names whose `[oploc1,name]` block never appeared.
    // Visited names already used SKIP_LEVER_ROUTE on zero-emission.
    for name in WILDERNESS_LEVER_LOC_NAMES {
        if ids.contains_key(*name) && !visited_oploc1.contains(*name) {
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
    let gate_applicable = toll_gate_applicable(ids);
    let henge_applicable = ids.contains_key(SHANTAY_HENGE_LOC_NAME);
    // The toll charge and the Shantay pass resolve by name through
    // `pack/obj.pack`; a missing pack skips the family instead of faking
    // an item id.
    let objs = obj_ids_by_name(content_root);
    let (Some(&coins_id), Some(&pass_id)) = (objs.get("coins"), objs.get("shantay_pass")) else {
        bump(
            skipped,
            SKIP_TOLL_OBJ_PACK,
            toll_applicable_route_count(gate_applicable, henge_applicable),
        );
        return;
    };

    let alkharid = content_root
        .join("scripts")
        .join("areas")
        .join("area_alkharid");
    let Ok(config) = fs::read_to_string(alkharid.join("configs").join("border_gate.loc")) else {
        if gate_applicable > 0 {
            bump(skipped, SKIP_TOLL_CONFIG, gate_applicable);
        }
        if henge_applicable {
            bump(skipped, SKIP_TOLL_HENGE, 1);
        }
        return;
    };
    let toll_ids = parse_door_config_ids(&config, ids);
    let open_ids = parse_door_open_ids(&config, ids);
    // Packed declared gate ids whose named openable block was not admitted.
    // Ids that entered `toll_ids` already use SKIP_TOLL_GATE on zero-emission.
    bump_unadmitted_packed_ids(
        skipped,
        SKIP_TOLL_CONFIG,
        ids,
        TOLL_GATE_LOC_NAMES,
        &toll_ids,
    );
    for id in toll_ids {
        let edge_start = graph.edges.len();
        let Some(placements) = positions.get(&id) else {
            if ids.values().any(|&packed| packed == id) {
                bump(skipped, SKIP_TOLL_GATE, 1);
            }
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
        if graph.edges.len() == edge_start && ids.values().any(|&packed| packed == id) {
            bump(skipped, SKIP_TOLL_GATE, 1);
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
// Magic Guild doors (`magic_guild.rs2`): named loc-specific openers.
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
        });
    }
}

fn rangingguild_resolve_loc_id(
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

type RangingGuildOpener = (
    (i32, i32, i32),
    (i32, i32, i32),
    (i32, i32, i32),
    (i32, i32, i32),
);

fn rangingguild_parse_opener(content_root: &Path) -> Option<RangingGuildOpener> {
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

// ---------------------------------------------------------------------------
// The Zanaris shed door (`quest_zanaris.rs2`): a worn-item teleport door.
// ---------------------------------------------------------------------------

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
#[path = "transport_tests.rs"]
mod tests;
