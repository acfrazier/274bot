//! Pure nav-paint buffers: remaining path tiles, draw subsampling,
//! collision face bits, client trail tones, transport hull targets, and
//! the step-ok component flood. No imgui, no client draw — the panel's
//! pack map and the client fork's 3D paints consume these.

use std::collections::{HashSet, VecDeque};
use std::sync::atomic::{AtomicU32, Ordering};

use api::snapshot::WorldTile;
use client::dash3d::CollisionFlag;

use crate::collision::{WorldCollision, SQ_BLOCKED};
use crate::router::{step_ok, Leg, Route};
use crate::transport::{TransportGraph, TransportKind};

/// One tile of the remaining path: a walk tile or a transport hop tile
/// (`transport` true on the hop's `at` and `to`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PathTile {
    pub tile: WorldTile,
    pub transport: bool,
}

/// The tiles still ahead on a whole-world `Route`, front to back. Walk
/// legs contribute every tile; a transport leg contributes its `at` and
/// `to` so the path stays connected across the crossing. When `here` is
/// given (the player's observed tile), legs already traversed are skipped
/// exactly as the follow run skips them, and the current walk leg is
/// trimmed to the tiles from `here` onward. Consecutive duplicate tiles (a
/// transport's `to` is the next walk leg's first tile) collapse into one,
/// keeping the transport flag.
pub fn remaining_path_tiles(route: &Route, here: Option<WorldTile>) -> Vec<PathTile> {
    let mut leg = 0;
    if let Some(here) = here {
        while leg < route.legs.len() {
            let done = match &route.legs[leg] {
                Leg::Walk { tiles } => tiles.last().is_some_and(|last| *last == here),
                Leg::Transport { edge } => edge.to == here,
            };
            if !done {
                break;
            }
            leg += 1;
        }
    }
    let mut out: Vec<PathTile> = Vec::new();
    for (i, l) in route.legs.iter().enumerate().skip(leg) {
        match l {
            Leg::Walk { tiles } => {
                let tiles: &[WorldTile] = if i == leg {
                    if let Some(here) = here {
                        if let Some(pos) = tiles.iter().position(|t| *t == here) {
                            &tiles[pos..]
                        } else {
                            tiles
                        }
                    } else {
                        tiles
                    }
                } else {
                    tiles
                };
                for t in tiles {
                    push_tile(&mut out, *t, false);
                }
            }
            Leg::Transport { edge } => {
                push_tile(&mut out, edge.at, true);
                push_tile(&mut out, edge.to, true);
            }
        }
    }
    out
}

/// Append a tile unless it repeats the last one; a repeat keeps (ORs in)
/// the transport flag so a hop's `at` that is also the previous walk
/// leg's last tile stays transport-coloured.
fn push_tile(out: &mut Vec<PathTile>, t: WorldTile, transport: bool) {
    if let Some(prev) = out.last_mut() {
        if prev.tile == t {
            prev.transport |= transport;
            return;
        }
    }
    out.push(PathTile { tile: t, transport });
}

/// Max tile quads a consumer draws (the far path is subsampled, rs2b0t
/// budget math).
pub const MAX_DRAW_TILES: usize = 160;
/// Always this many path steps ahead of the current index at full density.
pub const NEAR_FULL_DENSITY: usize = 48;

/// Indices of path tiles to draw: full density for [`NEAR_FULL_DENSITY`]
/// steps from `from_idx`, then a stride over the rest under
/// [`MAX_DRAW_TILES`], always keeping the terminal and every `force`
/// index (hop tiles are never subsampled away).
pub fn select_draw_indices(from_idx: usize, path_len: usize, force: &[usize]) -> Vec<usize> {
    if path_len == 0 || from_idx >= path_len {
        return Vec::new();
    }
    let start = from_idx;
    let near_end = path_len.min(start + NEAR_FULL_DENSITY);
    let mut chosen: HashSet<usize> = (start..near_end).collect();
    let remaining = path_len - near_end;
    if remaining > 0 {
        let budget = MAX_DRAW_TILES.saturating_sub(chosen.len());
        if budget > 0 {
            let stride = remaining.div_ceil(budget).max(1);
            let mut i = near_end;
            while i < path_len {
                chosen.insert(i);
                i += stride;
            }
        }
    }
    chosen.insert(path_len - 1);
    for &i in force {
        if i >= start && i < path_len {
            chosen.insert(i);
        }
    }
    let mut out: Vec<usize> = chosen.into_iter().collect();
    out.sort_unstable();
    out
}

/// The raw wall faces and the blocked-ground state of one tile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FaceBits {
    /// The raw `W_N` wall bit.
    pub n: bool,
    /// The raw `W_E` wall bit.
    pub e: bool,
    /// The raw `W_S` wall bit.
    pub s: bool,
    /// The raw `W_W` wall bit.
    pub w: bool,
    /// The tile cannot be stood on: a scenery footprint, a `WR_GRND`
    /// ground block, or the shared `SQ_BLOCKED` base. A bare face flag is
    /// not blocked ground — the wall's face-flagged floor tile is
    /// standable even though the router can never walk onto it.
    pub blocked: bool,
    pub ne: bool,
    pub se: bool,
    pub nw: bool,
    pub sw: bool,
    /// Raw low collision byte, retaining all eight W_* bits.
    pub raw: u8,
}

/// The collision state a consumer paints at `t`: the raw `W_*` face bits
/// and the blanket blocked flag. `flags` answers when the caller holds a
/// loaded flags sidecar (a panel side table — the shared world's `flags`
/// field is `None` while the sidecar is mapped); without one the
/// reconstructed walk word answers — blocked when any `SQ_BLOCKED`
/// constituent is set, NSEW from the `W_*` face bits. The passed buffer
/// must match the world's grid header.
pub fn collision_at_with(c: &WorldCollision, t: WorldTile, flags: Option<&[u32]>) -> FaceBits {
    let (raw, blocked) = match flags {
        Some(flags) => {
            let raw = c.flag_index(flags, t.x, t.z, t.level);
            (raw, raw & SQ_BLOCKED != 0)
        }
        None => {
            let word = c.walkable_word(t.x, t.z, t.level);
            (word, word & SQ_BLOCKED != 0)
        }
    };
    FaceBits {
        n: raw & CollisionFlag::W_N as u32 != 0,
        e: raw & CollisionFlag::W_E as u32 != 0,
        s: raw & CollisionFlag::W_S as u32 != 0,
        w: raw & CollisionFlag::W_W as u32 != 0,
        ne: raw & CollisionFlag::W_NE as u32 != 0,
        se: raw & CollisionFlag::W_SE as u32 != 0,
        nw: raw & CollisionFlag::W_NW as u32 != 0,
        sw: raw & CollisionFlag::W_SW as u32 != 0,
        raw: raw as u8,
        blocked,
    }
}

/// The collision state a consumer paints at `t`: the world's own raw
/// flags answer when the sidecar is attached
/// ([`WorldCollision::attach_flags`]), else the reconstructed walk word
/// (see [`collision_at_with`]).
pub fn collision_at(c: &WorldCollision, t: WorldTile) -> FaceBits {
    collision_at_with(c, t, c.flags.as_deref())
}

/// The tone of one client-trail tile: solid, or the run alternate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrailTone {
    Primary,
    RunAlt,
}

/// Client-trail tiles for paint: every tile carries [`TrailTone::Primary`]
/// when run is off; with run on, tiles checkerboard by world `(x + z) & 1`
/// (rs2b0t `pathScenePaint`, stable as the path trims).
pub fn trail_tones(tiles: &[WorldTile], run_on: bool) -> Vec<(WorldTile, TrailTone)> {
    tiles
        .iter()
        .copied()
        .map(|t| {
            let tone = if run_on && (t.x + t.z) & 1 == 1 {
                TrailTone::RunAlt
            } else {
                TrailTone::Primary
            };
            (t, tone)
        })
        .collect()
}

/// Trim a client-trail to the tiles from `here` onward (the occupied
/// tile stays, so the path does not flicker as the player steps). If
/// `here` is the dest, return empty so dest does not persist under the
/// player. If `here` is not on the list (arrived then stepped off, a
/// cancel, teleport), return empty so a later revisit cannot resurrect
/// the last click. `here == None` keeps the list: one stationary /
/// unknown tick during startup must not drop a pending trail. The trail
/// is every tryMove BFS tile — not the entity walk buffer (max 9).
pub fn remaining_trail(tiles: &[WorldTile], here: Option<WorldTile>) -> Vec<WorldTile> {
    let Some(h) = here else {
        return tiles.to_vec();
    };
    match tiles.iter().position(|t| t.x == h.x && t.z == h.z) {
        Some(i) if i + 1 == tiles.len() => Vec::new(),
        Some(i) => tiles[i..].to_vec(),
        None => Vec::new(),
    }
}

/// A loc-backed transport hop a consumer may hull: the interact loc id and
/// its placement tile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HullTarget {
    pub loc_id: i32,
    pub at: WorldTile,
}

/// One hop caption: the interact `at` tile and a short kind word.
/// Teleport / EssenceExit are skipped (the landing floats off-scene).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HopCaption {
    pub at: WorldTile,
    pub text: String,
}

fn hop_kind_word(kind: TransportKind) -> Option<&'static str> {
    match kind {
        TransportKind::Door => Some("Door"),
        TransportKind::Ladder => Some("Ladder"),
        TransportKind::Stairs => Some("Stairs"),
        TransportKind::Boat => Some("Boat"),
        TransportKind::AgilityShortcut => Some("Shortcut"),
        TransportKind::Glider => Some("Glider"),
        TransportKind::SpiritTree => Some("Spirit tree"),
        TransportKind::Npc => Some("Npc"),
        TransportKind::Teleport | TransportKind::EssenceExit => None,
    }
}

/// Remaining transport hops that get a caption, one per hop at `at`.
pub fn hop_captions(route: &Route, here: Option<WorldTile>) -> Vec<HopCaption> {
    let mut leg = 0;
    if let Some(here) = here {
        while leg < route.legs.len() {
            let done = match &route.legs[leg] {
                Leg::Walk { tiles } => tiles.last().is_some_and(|last| *last == here),
                Leg::Transport { edge } => edge.to == here,
            };
            if !done {
                break;
            }
            leg += 1;
        }
    }
    let mut out = Vec::new();
    for l in route.legs.iter().skip(leg) {
        if let Leg::Transport { edge } = l {
            let text = if edge.loc_id == 733 {
                Some("Web")
            } else {
                hop_kind_word(edge.kind)
            };
            if let Some(text) = text {
                if out.last().is_none_or(|c: &HopCaption| c.at != edge.at) {
                    out.push(HopCaption {
                        at: edge.at,
                        text: text.into(),
                    });
                }
            }
        }
    }
    out
}

/// Transport kinds with a loc to hull. Teleport, Boat and Glider hops are
/// NPC/spell-backed and draw no hull.
fn is_loc_backed(kind: TransportKind) -> bool {
    matches!(
        kind,
        TransportKind::Door
            | TransportKind::Ladder
            | TransportKind::Stairs
            | TransportKind::AgilityShortcut
    )
}

/// Planned loc-backed hops on the remaining route: from the current
/// remaining start through `next_only_plus` remaining tiles, always
/// including the next loc-backed hop. Done legs are skipped like
/// [`remaining_path_tiles`]; duplicate targets (a door placement's two
/// directed edges share one `at`) collapse.
pub fn hull_targets(
    route: &Route,
    here: Option<WorldTile>,
    next_only_plus: usize,
) -> Vec<HullTarget> {
    let mut leg = 0;
    if let Some(here) = here {
        while leg < route.legs.len() {
            let done = match &route.legs[leg] {
                Leg::Walk { tiles } => tiles.last().is_some_and(|last| *last == here),
                Leg::Transport { edge } => edge.to == here,
            };
            if !done {
                break;
            }
            leg += 1;
        }
    }
    let mut out: Vec<HullTarget> = Vec::new();
    let mut tile_idx = 0usize;
    let mut seen_loc_hop = false;
    for (i, l) in route.legs.iter().enumerate().skip(leg) {
        match l {
            Leg::Walk { tiles } => {
                // The first remaining walk leg counts from `here` onward,
                // exactly like `remaining_path_tiles` trims it, so the
                // window is measured from the same remaining start.
                let mut count = tiles.len();
                if i == leg {
                    if let Some(here) = here {
                        if let Some(pos) = tiles.iter().position(|t| *t == here) {
                            count = tiles.len() - pos;
                        }
                    }
                }
                tile_idx += count;
            }
            Leg::Transport { edge } => {
                if is_loc_backed(edge.kind) {
                    let hop = HullTarget {
                        loc_id: edge.loc_id,
                        at: edge.at,
                    };
                    let next = !seen_loc_hop;
                    seen_loc_hop = true;
                    if (next || tile_idx <= next_only_plus) && !out.contains(&hop) {
                        out.push(hop);
                    }
                }
                tile_idx += 2;
            }
        }
    }
    out
}

/// The eight step deltas (client coordinates: +x east, +z north), the
/// same array the router expands with.
const STEPS: [(i32, i32); 8] = [
    (0, 1),
    (0, -1),
    (1, 0),
    (-1, 0),
    (-1, -1),
    (1, -1),
    (-1, 1),
    (1, 1),
];

/// True when a standable tile has at least one `step_ok` neighbour — a
/// scatter seed must be able to walk off, not a 1-tile cage or a face-locked
/// wall cell.
pub fn can_step_off(c: &WorldCollision, t: WorldTile) -> bool {
    STEPS.iter().any(|&d| step_ok(c, t, d))
}

/// Every tile reachable from `seed` through the router's directional
/// `step_ok` test (the same movement the router relaxes with). The seed
/// itself is always in the component, like the router's origin handling.
fn flood_component(c: &WorldCollision, seed: WorldTile) -> HashSet<WorldTile> {
    let mut seen = HashSet::new();
    let mut queue = VecDeque::new();
    seen.insert(seed);
    queue.push_back(seed);
    while let Some(cur) = queue.pop_front() {
        for d in STEPS {
            if !step_ok(c, cur, d) {
                continue;
            }
            let nb = WorldTile {
                x: cur.x + d.0,
                z: cur.z + d.1,
                level: cur.level,
            };
            if seen.insert(nb) {
                queue.push_back(nb);
            }
        }
    }
    seen
}

/// The component id of `t` under the given seeds, in seed order (the
/// first seed whose flood reaches `t` wins), or `None` when no seed's
/// flood reaches it.
pub fn flood_component_id(c: &WorldCollision, seeds: &[WorldTile], t: WorldTile) -> Option<u32> {
    for (i, &seed) in seeds.iter().enumerate() {
        if flood_component(c, seed).contains(&t) {
            return Some(i as u32);
        }
    }
    None
}

/// The step-ok reachable set for every seed, in seed order — one BFS per
/// seed, computed once. A consumer that marks many tiles (a viewport
/// flood) computes the sets once and probes them instead of re-flooding
/// per tile through [`flood_component_id`].
pub fn flood_components(c: &WorldCollision, seeds: &[WorldTile]) -> Vec<HashSet<WorldTile>> {
    seeds.iter().map(|&s| flood_component(c, s)).collect()
}

/// The flood size from `a`. When `b` is given and reachable from `a` the
/// pair reports one component (`None` for the second); when the two seeds
/// are disconnected both sizes are reported.
pub fn flood_sizes(
    c: &WorldCollision,
    a: WorldTile,
    b: Option<WorldTile>,
) -> (usize, Option<usize>) {
    let comp_a = flood_component(c, a);
    let b_size = match b {
        Some(b) if !comp_a.contains(&b) => Some(flood_component(c, b).len()),
        _ => None,
    };
    (comp_a.len(), b_size)
}

/// The reach-flood seeds: every transport edge's `at` and `to`, plus
/// every teleport's `to` (teleports have no fixed origin — the landing
/// anchors the any-tile layer's component). The bake's `seen` set
/// dedupes them.
pub fn reach_seeds(graph: &TransportGraph) -> Vec<WorldTile> {
    let mut seeds: Vec<WorldTile> = Vec::new();
    for e in &graph.edges {
        seeds.push(e.at);
        seeds.push(e.to);
    }
    for e in &graph.teleports {
        seeds.push(e.to);
    }
    seeds
}

/// Process-wide `bake_reach` invocations. Bundled runtime must keep this at
/// zero; the external/custom path may increment once per bound world.
static BAKE_REACH_CALLS: AtomicU32 = AtomicU32::new(0);

/// How many times [`bake_reach`] has run in this process.
pub fn bake_reach_calls() -> u32 {
    BAKE_REACH_CALLS.load(Ordering::Relaxed)
}

/// Test helper: reset the flood counter between cases.
pub fn reset_bake_reach_calls() {
    BAKE_REACH_CALLS.store(0, Ordering::Relaxed);
}

/// The paint-only reach bitset: one bit per walk cell per level (the same
/// level-major indexing as the walk grid), length
/// `ceil(walk.len() / 64)`. A bit is set when the tile is in the `step_ok`
/// flood from any [`reach_seeds`] seed — connected via walk ∪ transports
/// ∪ teles. `find` never reads this; it is the debug overlay's in-graph
/// answer.
pub fn bake_reach(c: &WorldCollision, graph: &TransportGraph) -> Vec<u64> {
    BAKE_REACH_CALLS.fetch_add(1, Ordering::Relaxed);
    let mut bits = vec![0u64; c.walk.len().div_ceil(64)];
    let mut seen: HashSet<WorldTile> = HashSet::new();
    let mut queue: VecDeque<WorldTile> = VecDeque::new();
    for seed in reach_seeds(graph) {
        if seen.insert(seed) {
            queue.push_back(seed);
        }
    }
    while let Some(cur) = queue.pop_front() {
        set_reach_bit(&mut bits, c, cur);
        for d in STEPS {
            if !step_ok(c, cur, d) {
                continue;
            }
            let nb = WorldTile {
                x: cur.x + d.0,
                z: cur.z + d.1,
                level: cur.level,
            };
            if seen.insert(nb) {
                queue.push_back(nb);
            }
        }
    }
    bits
}

/// Whether the walk cell of `t` is set in a [`bake_reach`] bitset;
/// `false` for tiles outside the grid or on unknown levels, and when the
/// bitset is shorter than the tile's word.
pub fn reached(bits: &[u64], c: &WorldCollision, t: WorldTile) -> bool {
    let Some(idx) = reach_cell_index(c, t) else {
        return false;
    };
    bits.get(idx / 64)
        .is_some_and(|w| w & (1 << (idx % 64)) != 0)
}

/// The walk-buffer index of `t` (the same level-major indexing as the
/// walk grid), `None` outside the grid or on unknown levels.
fn reach_cell_index(c: &WorldCollision, t: WorldTile) -> Option<usize> {
    if !(0..4).contains(&t.level) {
        return None;
    }
    let lx = t.x - c.origin.x;
    let lz = t.z - c.origin.z;
    if lx < 0 || lz < 0 {
        return None;
    }
    let (lx, lz) = (lx as usize, lz as usize);
    if lx >= c.width || lz >= c.height {
        return None;
    }
    Some(t.level as usize * c.width * c.height + lz * c.width + lx)
}

/// Set the walk-cell bit of `t` (a seed may sit outside the bake or on a
/// blocked tile — the router floods from such origins too).
fn set_reach_bit(bits: &mut [u64], c: &WorldCollision, t: WorldTile) {
    if let Some(idx) = reach_cell_index(c, t) {
        if let Some(word) = bits.get_mut(idx / 64) {
            *word |= 1 << (idx % 64);
        }
    }
}

#[cfg(test)]
#[path = "paint_tests.rs"]
mod tests;
