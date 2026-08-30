//! Pure nav-paint buffers: remaining path tiles, draw subsampling,
//! collision face bits, client trail tones, transport hull targets, and
//! the step-ok component flood. No imgui, no client draw — the panel's
//! pack map and the client fork's 3D paints consume these.

use std::collections::{HashSet, VecDeque};

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
/// player. If `here` is not on the list (pushed off the BFS), the full
/// list is kept. The trail is every tryMove BFS tile — not the entity
/// walk buffer (max 9).
pub fn remaining_trail(tiles: &[WorldTile], here: Option<WorldTile>) -> Vec<WorldTile> {
    if let Some(h) = here {
        if let Some(i) = tiles.iter().position(|t| t.x == h.x && t.z == h.z) {
            if i + 1 == tiles.len() {
                return Vec::new();
            }
            return tiles[i..].to_vec();
        }
    }
    tiles.to_vec()
}

/// A loc-backed transport hop a consumer may hull: the interact loc id and
/// its placement tile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HullTarget {
    pub loc_id: i32,
    pub at: WorldTile,
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
                    if next || tile_idx <= next_only_plus {
                        if !out.contains(&hop) {
                            out.push(hop);
                        }
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

/// The paint-only reach bitset: one bit per walk cell per level (the same
/// level-major indexing as the walk grid), length
/// `ceil(walk.len() / 64)`. A bit is set when the tile is in the `step_ok`
/// flood from any [`reach_seeds`] seed — connected via walk ∪ transports
/// ∪ teles. `find` never reads this; it is the debug overlay's in-graph
/// answer.
pub fn bake_reach(c: &WorldCollision, graph: &TransportGraph) -> Vec<u64> {
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
mod tests {
    use api::snapshot::WorldTile;
    use client::dash3d::CollisionFlag;

    use super::*;
    use crate::collision::WorldCollision;
    use crate::router::{Leg, Route};
    use crate::transport::{TransportEdge, TransportGraph, TransportKind};

    fn tile(x: i32, z: i32, level: i32) -> WorldTile {
        WorldTile { x, z, level }
    }

    /// A `width`×`height` level-0 bake at (0,0) with the given per-tile
    /// flags OR'd in. Planes 1..=3 stay empty (the per-level bake shape).
    fn bake(width: usize, height: usize, extras: &[(i32, i32, u32)]) -> WorldCollision {
        let mut plane = vec![0u32; width * height];
        for &(x, z, f) in extras {
            plane[z as usize * width + x as usize] |= f;
        }
        let mut flags = vec![0u32; 4 * plane.len()];
        flags[..plane.len()].copy_from_slice(&plane);
        WorldCollision {
            origin: tile(0, 0, 0),
            width,
            height,
            walk: crate::collision::pack_walk_u16(&flags),
            flags: None,
        }
    }

    fn edge(kind: TransportKind, at: WorldTile, to: WorldTile, loc_id: i32) -> TransportEdge {
        TransportEdge {
            kind,
            at,
            to,
            loc_id,
            option: 1,
            ticks: 1,
            dir: None,
            open_loc_id: None,
            skill_req: vec![],
            item_req: vec![],
            quest_req: vec![],
            varp_req: vec![],
            worn_req: vec![],
        }
    }

    fn route(legs: Vec<Leg>) -> Route {
        let dest = legs
            .iter()
            .rev()
            .find_map(|l| match l {
                Leg::Walk { tiles } => tiles.last().copied(),
                Leg::Transport { edge } => Some(edge.to),
            })
            .unwrap_or(tile(0, 0, 0));
        Route {
            legs,
            dest,
            ticks: 0.0,
        }
    }

    #[test]
    fn remaining_path_includes_transport_at_to() {
        // Walk then Door; here on first tile → remaining has walk + at + to
        let r = route(vec![
            Leg::Walk {
                tiles: vec![tile(0, 0, 0), tile(1, 0, 0)],
            },
            Leg::Transport {
                edge: edge(TransportKind::Door, tile(2, 0, 0), tile(3, 0, 0), 1530),
            },
        ]);
        let tiles = remaining_path_tiles(&r, Some(tile(0, 0, 0)));
        let flat: Vec<WorldTile> = tiles.iter().map(|p| p.tile).collect();
        assert_eq!(
            flat,
            vec![tile(0, 0, 0), tile(1, 0, 0), tile(2, 0, 0), tile(3, 0, 0)],
            "here on the first tile keeps the whole path"
        );
        assert!(
            tiles[..2].iter().all(|p| !p.transport),
            "walk tiles are not transport"
        );
        assert!(
            tiles[2..].iter().all(|p| p.transport),
            "at and to carry the transport flag"
        );
    }

    #[test]
    fn remaining_path_tiles_skips_done_legs_and_trims_here() {
        let r = route(vec![
            Leg::Walk {
                tiles: vec![tile(0, 0, 0), tile(1, 0, 0), tile(2, 0, 0)],
            },
            Leg::Transport {
                edge: edge(TransportKind::Door, tile(3, 0, 0), tile(4, 0, 0), 1530),
            },
            Leg::Walk {
                tiles: vec![tile(4, 0, 0), tile(5, 0, 0)],
            },
        ]);
        // Mid-leg: the current walk leg trims to here onward; the transport
        // and following walk leg stay; the crossing tile dedups to one.
        let tiles = remaining_path_tiles(&r, Some(tile(1, 0, 0)));
        let flat: Vec<WorldTile> = tiles.iter().map(|p| p.tile).collect();
        assert_eq!(
            flat,
            vec![
                tile(1, 0, 0),
                tile(2, 0, 0),
                tile(3, 0, 0),
                tile(4, 0, 0),
                tile(5, 0, 0)
            ]
        );
        assert_eq!(tiles.len(), 5, "the crossing tile must dedup to one");
        // At a leg end the done walk leg is skipped entirely.
        let tiles = remaining_path_tiles(&r, Some(tile(2, 0, 0)));
        let flat: Vec<WorldTile> = tiles.iter().map(|p| p.tile).collect();
        assert_eq!(flat, vec![tile(3, 0, 0), tile(4, 0, 0), tile(5, 0, 0)]);
    }

    #[test]
    fn select_draw_keeps_hops_and_terminal() {
        let force = vec![100];
        let idx = select_draw_indices(0, 400, &force);
        assert!(idx.contains(&0) && idx.contains(&399) && idx.contains(&100));
        assert!(idx.len() <= MAX_DRAW_TILES + 4);
    }

    #[test]
    fn collision_at_reports_south_face_not_blocked_ground() {
        // raw W_S only → nsew.s true, blocked false (face ≠ blanket walkable)
        let c = bake(1, 1, &[(0, 0, CollisionFlag::W_S as u32)]);
        let fb = collision_at(&c, tile(0, 0, 0));
        assert!(fb.s);
        assert!(!fb.n && !fb.e && !fb.w);
        assert!(!fb.blocked, "a bare face flag is not blocked ground");
    }

    #[test]
    fn collision_at_blocks_ground_and_scenery() {
        let c = bake(
            1,
            3,
            &[
                (0, 0, CollisionFlag::WR_GRND as u32),
                (0, 1, CollisionFlag::WALK_SCENERY as u32),
                (0, 2, CollisionFlag::W_N as u32),
            ],
        );
        assert!(collision_at(&c, tile(0, 0, 0)).blocked);
        assert!(collision_at(&c, tile(0, 1, 0)).blocked);
        assert!(
            !collision_at(&c, tile(0, 2, 0)).blocked,
            "W_N is a face, not a ground block"
        );
    }

    #[test]
    fn trail_two_tone_only_when_run_on() {
        let tiles = [tile(0, 0, 0), tile(0, 1, 0), tile(0, 2, 0)];
        assert!(trail_tones(&tiles, false)
            .iter()
            .all(|(_, t)| *t == TrailTone::Primary));
        let t = trail_tones(&tiles, true);
        assert_eq!(t[0].1, TrailTone::Primary);
        assert_eq!(t[1].1, TrailTone::RunAlt);
        assert_eq!(t[2].1, TrailTone::Primary);
        // Checkerboard by world tile, not list index: a diagonal step does
        // not flip just because it is the second entry.
        let diag = trail_tones(&[tile(0, 0, 0), tile(1, 1, 0)], true);
        assert_eq!(diag[0].1, TrailTone::Primary);
        assert_eq!(diag[1].1, TrailTone::Primary);
    }

    #[test]
    fn remaining_trail_keeps_mid_path_but_clears_arrived_dest() {
        let tiles: Vec<WorldTile> = (0..21).map(|x| tile(x, 0, 0)).collect();
        assert_eq!(remaining_trail(&tiles, None).len(), 21);
        let rest = remaining_trail(&tiles, Some(tile(5, 0, 0)));
        assert_eq!(rest.len(), 16, "a 21-tile BFS is not capped at 9");
        assert_eq!(rest[0], tile(5, 0, 0));
        assert_eq!(rest.last().copied(), Some(tile(20, 0, 0)));
        assert!(
            remaining_trail(&tiles, Some(tile(20, 0, 0))).is_empty(),
            "arrived dest must not persist under the player"
        );
        // Off the path: keep the full click, do not invent a trim.
        assert_eq!(remaining_trail(&tiles, Some(tile(99, 0, 0))).len(), 21);
    }

    #[test]
    fn hull_skips_teleport_and_missing_loc() {
        // A teleport hop has no loc scenery.
        let r = route(vec![Leg::Transport {
            edge: edge(TransportKind::Teleport, tile(0, 0, 0), tile(1, 0, 0), 0),
        }]);
        assert!(
            hull_targets(&r, None, 12).is_empty(),
            "teleport hops have no hull"
        );
        // NPC hops (boat/glider) have no loc either.
        let r = route(vec![Leg::Transport {
            edge: edge(TransportKind::Boat, tile(0, 0, 0), tile(1, 0, 0), 0),
        }]);
        assert!(
            hull_targets(&r, None, 12).is_empty(),
            "NPC hops have no hull"
        );
        // A door with a loc id resolves to one target.
        let r = route(vec![Leg::Transport {
            edge: edge(TransportKind::Door, tile(2, 0, 0), tile(3, 0, 0), 1530),
        }]);
        assert_eq!(
            hull_targets(&r, None, 12),
            vec![HullTarget {
                loc_id: 1530,
                at: tile(2, 0, 0)
            }]
        );
    }

    #[test]
    fn hull_targets_keeps_window_hops_and_always_the_next() {
        // The first ladder is far beyond the window but is the next loc
        // hop (always kept); the door past it is neither next nor in-window.
        let r = route(vec![
            Leg::Walk {
                tiles: (0..50).map(|x| tile(x, 0, 0)).collect(),
            },
            Leg::Transport {
                edge: edge(TransportKind::Ladder, tile(50, 0, 0), tile(51, 0, 0), 1111),
            },
            Leg::Walk {
                tiles: (51..101).map(|x| tile(x, 0, 0)).collect(),
            },
            Leg::Transport {
                edge: edge(TransportKind::Door, tile(101, 0, 0), tile(102, 0, 0), 1530),
            },
        ]);
        assert_eq!(
            hull_targets(&r, None, 12),
            vec![HullTarget {
                loc_id: 1111,
                at: tile(50, 0, 0)
            }],
            "only the next loc hop survives a far window"
        );
    }

    #[test]
    fn hull_targets_keeps_in_window_hops() {
        let r = route(vec![
            Leg::Walk {
                tiles: vec![tile(0, 0, 0), tile(1, 0, 0)],
            },
            Leg::Transport {
                edge: edge(TransportKind::Door, tile(2, 0, 0), tile(3, 0, 0), 1530),
            },
            Leg::Walk {
                tiles: vec![tile(3, 0, 0), tile(4, 0, 0)],
            },
            Leg::Transport {
                edge: edge(TransportKind::Door, tile(5, 0, 0), tile(6, 0, 0), 1531),
            },
        ]);
        assert_eq!(
            hull_targets(&r, None, 12),
            vec![
                HullTarget {
                    loc_id: 1530,
                    at: tile(2, 0, 0)
                },
                HullTarget {
                    loc_id: 1531,
                    at: tile(5, 0, 0)
                },
            ]
        );
    }

    #[test]
    fn hull_targets_window_counts_from_the_trimmed_start() {
        // `here` mid-walk trims the first walk leg, so `next_only_plus` is
        // measured from the same remaining start `remaining_path_tiles`
        // uses. Walk (0..5), door A, walk (6..12), door B, here=(3,0),
        // next_only_plus=12: the un-trimmed count puts door B at tile 13
        // (dropped), the trimmed count at tile 10 (kept).
        let r = route(vec![
            Leg::Walk {
                tiles: (0..5).map(|x| tile(x, 0, 0)).collect(),
            },
            Leg::Transport {
                edge: edge(TransportKind::Door, tile(5, 0, 0), tile(6, 0, 0), 1530),
            },
            Leg::Walk {
                tiles: (6..12).map(|x| tile(x, 0, 0)).collect(),
            },
            Leg::Transport {
                edge: edge(TransportKind::Door, tile(12, 0, 0), tile(13, 0, 0), 1531),
            },
        ]);
        assert_eq!(
            hull_targets(&r, Some(tile(3, 0, 0)), 12),
            vec![
                HullTarget {
                    loc_id: 1530,
                    at: tile(5, 0, 0)
                },
                HullTarget {
                    loc_id: 1531,
                    at: tile(12, 0, 0)
                },
            ],
            "the follow-on door is in the window from the trimmed start"
        );
    }

    #[test]
    fn hull_targets_collapses_duplicate_door_targets() {
        // A door placement contributes two directed edges sharing one `at`.
        let r = route(vec![
            Leg::Transport {
                edge: edge(TransportKind::Door, tile(2, 0, 0), tile(3, 0, 0), 1530),
            },
            Leg::Transport {
                edge: edge(TransportKind::Door, tile(2, 0, 0), tile(1, 0, 0), 1530),
            },
        ]);
        assert_eq!(
            hull_targets(&r, None, 12).len(),
            1,
            "duplicate targets collapse"
        );
    }

    /// A 7×7 bake: a 3×3 open corner plus an isolated open tile moated by
    /// WR_GRND; everything else blocked ground.
    fn disconnected_world() -> WorldCollision {
        let mut extras = Vec::new();
        for z in 0..7 {
            for x in 0..7 {
                let open = (x < 3 && z < 3) || (x == 5 && z == 5);
                if !open {
                    extras.push((x, z, CollisionFlag::WR_GRND as u32));
                }
            }
        }
        bake(7, 7, &extras)
    }

    #[test]
    fn flood_two_seeds_disconnected_have_two_sizes() {
        // 3x3 open vs isolated tile with WR_GRND moat
        let c = disconnected_world();
        let (a, b) = flood_sizes(&c, tile(0, 0, 0), Some(tile(5, 5, 0)));
        assert!(a >= 1 && b.unwrap() >= 1);
        assert_ne!(
            flood_component_id(&c, &[tile(0, 0, 0)], tile(0, 0, 0)),
            flood_component_id(&c, &[tile(0, 0, 0)], tile(5, 5, 0))
        );
    }

    #[test]
    fn flood_marks_two_components_from_two_seeds() {
        let c = disconnected_world();
        let seeds = [tile(0, 0, 0), tile(5, 5, 0)];
        assert_eq!(flood_component_id(&c, &seeds, tile(1, 1, 0)), Some(0));
        assert_eq!(flood_component_id(&c, &seeds, tile(5, 5, 0)), Some(1));
        // A moat tile belongs to no flood.
        assert_eq!(flood_component_id(&c, &seeds, tile(3, 3, 0)), None);
    }

    #[test]
    fn flood_components_sets_match_component_ids() {
        let c = disconnected_world();
        let seeds = [tile(0, 0, 0), tile(5, 5, 0)];
        let comps = flood_components(&c, &seeds);
        assert_eq!(comps.len(), 2);
        assert!(comps[0].contains(&tile(1, 1, 0)));
        assert!(comps[1].contains(&tile(5, 5, 0)));
        assert!(
            !comps[0].contains(&tile(5, 5, 0)),
            "the moat keeps the disconnected seed out"
        );
        assert_eq!(flood_component_id(&c, &seeds, tile(1, 1, 0)), Some(0));
    }

    #[test]
    fn flood_same_component_counts_once() {
        let c = bake(3, 3, &[]);
        assert_eq!(
            flood_sizes(&c, tile(0, 0, 0), Some(tile(2, 2, 0))),
            (9, None)
        );
    }

    /// A graph with the given `edges` and `teleports` (the `at` index is
    /// irrelevant to the reach bake, which reads the edge lists directly).
    fn graph(edges: Vec<TransportEdge>, teleports: Vec<TransportEdge>) -> TransportGraph {
        TransportGraph {
            edges,
            teleports,
            ..Default::default()
        }
    }

    #[test]
    fn walled_courtyard_is_walkable_but_unreached() {
        // 5×5 open, inner 1×1 at (2,2) with all W_* faces on its four walls
        // and no transport. (2,2) walkable_word has faces; find from (0,0)
        // is NoPath; bake_reach does not set (2,2).
        let c = bake(5, 5, &[(2, 2, CollisionFlag::WALK_BLOCK_FLAGS as u32)]);
        let g = TransportGraph::default();
        let at = tile(2, 2, 0);
        assert_ne!(
            c.walkable_word(2, 2, 0) & CollisionFlag::WALK_BLOCK_FLAGS as u32,
            0,
            "the sealed courtyard word carries the face bits"
        );
        assert!(
            !collision_at(&c, at).blocked,
            "a walled floor is standable ground, not blocked"
        );
        assert!(
            crate::router::find(&c, &g, tile(0, 0, 0), at).is_err(),
            "find from outside the sealed courtyard is NoPath"
        );
        let bits = bake_reach(&c, &g);
        assert_eq!(bits.len(), c.walk.len().div_ceil(64));
        assert!(
            !reached(&bits, &c, at),
            "no transport seeds flood the sealed courtyard"
        );
        assert!(
            !reached(&bits, &c, tile(0, 0, 0)),
            "no seeds, nothing reached"
        );
        assert!(
            !reached(&bits, &c, tile(99, 99, 0)),
            "tiles outside the grid are never reached"
        );
    }

    #[test]
    fn reach_seeds_cover_edges_and_teleport_landings() {
        // Teleports have no fixed origin — only the landing seeds the
        // any-tile layer's component; a regular edge seeds both ends.
        let g = graph(
            vec![edge(
                TransportKind::Door,
                tile(0, 0, 0),
                tile(1, 0, 0),
                1530,
            )],
            vec![edge(
                TransportKind::Teleport,
                tile(0, 0, 0),
                tile(4, 4, 0),
                0,
            )],
        );
        let seeds = reach_seeds(&g);
        assert!(seeds.contains(&tile(0, 0, 0)), "edge at seeds");
        assert!(seeds.contains(&tile(1, 0, 0)), "edge to seeds");
        assert!(seeds.contains(&tile(4, 4, 0)), "teleport to seeds");
    }

    #[test]
    fn bake_reach_floods_the_walk_region_but_not_moated_tiles() {
        // The 7×7 disconnected world: a 3×3 open corner and an isolated
        // open tile moated by WR_GRND. A door edge inside the corner seeds
        // it; the flood covers the corner, never the island or the moat.
        let c = disconnected_world();
        let g = graph(
            vec![edge(
                TransportKind::Door,
                tile(0, 0, 0),
                tile(1, 1, 0),
                1530,
            )],
            vec![],
        );
        let bits = bake_reach(&c, &g);
        assert!(reached(&bits, &c, tile(0, 0, 0)), "the edge at is a seed");
        assert!(reached(&bits, &c, tile(1, 1, 0)), "the edge to is a seed");
        assert!(reached(&bits, &c, tile(2, 2, 0)), "the corner floods");
        assert!(
            !reached(&bits, &c, tile(5, 5, 0)),
            "the moated island stays unreached"
        );
        assert!(
            !reached(&bits, &c, tile(3, 3, 0)),
            "moat ground is never reached"
        );
        assert!(
            !reached(&bits, &c, tile(0, 0, 1)),
            "unknown levels read false"
        );
    }
}
