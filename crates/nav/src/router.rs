//! Dijkstra routing over the whole-world collision bake (`WorldCollision`)
//! and the content-derived transport graph (`TransportGraph`), plus the
//! legacy step-grid A* kept under [`find_on_grid`] for the traveller and
//! live harnesses until the collision+transport router replaces them.
//!
//! One Dijkstra minimizes **total ticks**: a walk step costs the search's
//! run rate ([`CostModel::running`]: 0.5 ticks/tile, 2 tiles/tick) and a
//! transport edge costs its `ticks` (OP_BASE + duration). Costs are `f64`
//! — 0.5 and every reachable sum are exact — ordered with `total_cmp` in
//! the heap so equal costs tie-break on tile coordinates. A step into a
//! neighbour is allowed only when the neighbour's derived walkable word
//! passes the client's directional movement test (the `PL_WALK_*` masks in
//! `tryMove`), never the blanket `walkable()` check.
//!
//! [`find`] and [`find_with_model`] never see teleports; the any-tile
//! teleport layer ([`TransportGraph::teleports`]) only joins the search
//! through [`find_allow_teleports`]/[`find_allow_teleports_with_model`].
//! Wilderness tiles ([`crate::wilderness::in_wilderness`]) are refused
//! unless the search's [`FindOptions::allow_wilderness`] is set; every
//! option'd entry point is [`find_with`].

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, HashSet};

use api::snapshot::WorldTile;
use client::dash3d::CollisionFlag;

use crate::collision::WorldCollision;
use crate::grid::{DoorEdge, StepGrid};
use crate::tile::{chebyshev, Tile};
use crate::transport::{TransportEdge, TransportGraph};
use crate::wilderness::in_wilderness;

/// One leg of a route: a walk run or one transport crossing. Consecutive
/// walk tiles collapse into a single `Walk` leg; each transport edge is
/// its own `Transport` leg.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Leg {
    Walk { tiles: Vec<WorldTile> },
    Transport { edge: TransportEdge },
}

/// A route from the `find` origin to `dest`. `ticks` is the total tick
/// cost of the whole route: walk steps at the search's run rate (0.5 per
/// tile, exact in f64) plus every transport edge's `ticks`. Half-ticks are
/// never truncated.
#[derive(Debug, Clone, PartialEq)]
pub struct Route {
    pub legs: Vec<Leg>,
    pub dest: WorldTile,
    pub ticks: f64,
}

/// Run pace: 2 tiles per tick, 0.5 ticks per tile.
pub const PER_STEP_RUN: f64 = 0.5;
/// Walk pace: 1 tile per tick, 1 tick per tile.
pub const PER_STEP_WALK: f64 = 1.0;

/// The per-search walking rate. This task's searches walk at the fixed
/// running pace ([`CostModel::running`]); the agility/regen energy-aware
/// fallback will flip to the walk pace when run energy can't sustain
/// running.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CostModel {
    /// Ticks per walked tile at the running pace (0.5).
    pub run_per_step: f64,
    /// Ticks per walked tile at the walking pace (1.0).
    pub walk_per_step: f64,
}

impl CostModel {
    /// The fixed running rate this task searches with.
    pub fn running() -> Self {
        CostModel {
            run_per_step: PER_STEP_RUN,
            walk_per_step: PER_STEP_WALK,
        }
    }
}

/// Per-search opt-ins, both default off so [`find`] keeps the safe
/// defaults. `allow_teleports` unions the any-tile teleport layer in;
/// `allow_wilderness` lets the search step into (or land in) the
/// wilderness zone ([`crate::wilderness::in_wilderness`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FindOptions {
    pub allow_teleports: bool,
    pub allow_wilderness: bool,
}

/// Why [`find`] failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteError {
    /// No walk/transport path reaches the destination.
    NoPath,
    /// The node-expansion budget was exhausted before reaching it.
    BudgetExhausted,
}

/// Node-expansion budget bounding a [`find`] search — the m8aq route-cutoff
/// concept. The whole 2004 world is ~16M tiles, so any real route stays far
/// under this; it only stops pathological floods.
const NODE_BUDGET: usize = 4_000_000;

/// The interact radius (chebyshev) a transport edge is usable from: any
/// standable tile within this distance of the edge's `at` is a valid
/// take-off. The approach is derived at expansion time, never baked.
const INTERACT_RADIUS: i32 = 3;

/// The eight step deltas (client coordinates: +x east, +z north).
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

/// The client's `PL_WALK_*` movement masks, as `u32` to match the flag word.
const MASK_N: u32 = CollisionFlag::PL_WALK_N as u32;
const MASK_E: u32 = CollisionFlag::PL_WALK_E as u32;
const MASK_S: u32 = CollisionFlag::PL_WALK_S as u32;
const MASK_W: u32 = CollisionFlag::PL_WALK_W as u32;
const MASK_NE: u32 = CollisionFlag::PL_WALK_NE as u32;
const MASK_SE: u32 = CollisionFlag::PL_WALK_SE as u32;
const MASK_NW: u32 = CollisionFlag::PL_WALK_NW as u32;
const MASK_SW: u32 = CollisionFlag::PL_WALK_SW as u32;

/// Dijkstra over `collision` walk steps (each costing the search's run
/// rate) and `graph` transport edges (each costing `edge.ticks`), all in
/// one heap minimizing total ticks. Walk steps are 8-directional and stay
/// on `from`'s level; transports may change level.
///
/// The origin may sit on a blocked tile (a loc-blocked tele landing); only
/// the tiles stepped *onto* are tested. Destinations are reached exactly:
/// a blocked destination is `NoPath` unless a transport lands on it.
pub fn find(
    collision: &WorldCollision,
    graph: &TransportGraph,
    from: WorldTile,
    to: WorldTile,
) -> Result<Route, RouteError> {
    find_with(collision, graph, from, to, FindOptions::default())
}

/// [`find`] with explicit opt-ins: [`FindOptions::allow_teleports`] unions
/// the any-tile teleport layer in and [`FindOptions::allow_wilderness`]
/// allows entering the wilderness zone. Both default off, so plain
/// [`find`] keeps the safe defaults.
pub fn find_with(
    collision: &WorldCollision,
    graph: &TransportGraph,
    from: WorldTile,
    to: WorldTile,
    opts: FindOptions,
) -> Result<Route, RouteError> {
    find_bounded_impl(
        collision,
        graph,
        from,
        to,
        CostModel::running(),
        NODE_BUDGET,
        opts.allow_teleports,
        opts.allow_wilderness,
    )
}

/// [`find`] with an explicit per-search cost model (the run-vs-walk rate).
pub fn find_with_model(
    collision: &WorldCollision,
    graph: &TransportGraph,
    from: WorldTile,
    to: WorldTile,
    model: CostModel,
) -> Result<Route, RouteError> {
    find_bounded_impl(collision, graph, from, to, model, NODE_BUDGET, false, false)
}

/// [`find`] with the any-tile teleport layer unioned in: every edge in
/// `graph.teleports` is usable from **any** node at cost `edge.ticks`, so
/// a teleport can take the route across a wall, a level boundary, or half
/// the map. Requirements stay on the edges (never gated here, like every
/// other transport edge); the caller checks them. Default [`find`]/
/// [`find_with_model`] never see teleports. Wilderness stays refused
/// (`allow_wilderness` off).
pub fn find_allow_teleports(
    collision: &WorldCollision,
    graph: &TransportGraph,
    from: WorldTile,
    to: WorldTile,
) -> Result<Route, RouteError> {
    find_with(
        collision,
        graph,
        from,
        to,
        FindOptions {
            allow_teleports: true,
            allow_wilderness: false,
        },
    )
}

/// [`find_allow_teleports`] with an explicit per-search cost model.
pub fn find_allow_teleports_with_model(
    collision: &WorldCollision,
    graph: &TransportGraph,
    from: WorldTile,
    to: WorldTile,
    model: CostModel,
) -> Result<Route, RouteError> {
    find_bounded_impl(collision, graph, from, to, model, NODE_BUDGET, true, false)
}

/// [`find`] with an explicit cost model and node-expansion budget (the
/// search gives up with [`RouteError::BudgetExhausted`] once `budget`
/// tiles are settled). Teleports and wilderness stay off. Test-only: the
/// budget knob has no production caller yet.
#[cfg(test)]
fn find_bounded(
    collision: &WorldCollision,
    graph: &TransportGraph,
    from: WorldTile,
    to: WorldTile,
    model: CostModel,
    budget: usize,
) -> Result<Route, RouteError> {
    find_bounded_impl(collision, graph, from, to, model, budget, false, false)
}

/// The shared Dijkstra; `use_teleports` unions the any-tile teleport layer
/// into the relaxation from every settled node. Transport edges are relaxed
/// from any standable tile within [`INTERACT_RADIUS`] of their `at` (never
/// from `at` itself when it is blocked); walk steps are the strict
/// directional [`step_ok`] test throughout. `allow_wilderness` gates
/// stepping into (or landing in) the wilderness zone.
fn find_bounded_impl(
    collision: &WorldCollision,
    graph: &TransportGraph,
    from: WorldTile,
    to: WorldTile,
    model: CostModel,
    budget: usize,
    use_teleports: bool,
    allow_wilderness: bool,
) -> Result<Route, RouteError> {
    if from == to {
        return Ok(Route {
            legs: vec![Leg::Walk { tiles: vec![from] }],
            dest: to,
            ticks: 0.0,
        });
    }

    let mut dist: HashMap<WorldTile, f64> = HashMap::new();
    let mut came_from: HashMap<WorldTile, Back> = HashMap::new();
    let mut heap: BinaryHeap<HeapNode> = BinaryHeap::new();
    let mut done: HashSet<WorldTile> = HashSet::new();

    dist.insert(from, 0.0);
    heap.push(HeapNode {
        cost: 0.0,
        tile: from,
    });

    let mut expanded = 0usize;
    while let Some(n) = heap.pop() {
        let cur = n.tile;
        // A stale heap entry (a cheaper path was found after the push) is
        // skipped; the first pop at the settled distance settles the tile.
        if dist.get(&cur) != Some(&n.cost) {
            continue;
        }
        if !done.insert(cur) {
            continue;
        }
        expanded += 1;
        if expanded > budget {
            return Err(RouteError::BudgetExhausted);
        }
        if cur == to {
            let (legs, ticks) = reconstruct(to, &came_from, graph, model);
            return Ok(Route {
                legs,
                dest: to,
                ticks,
            });
        }

        for d in STEPS {
            if step_ok(collision, cur, d) {
                let nb = WorldTile {
                    x: cur.x + d.0,
                    z: cur.z + d.1,
                    level: cur.level,
                };
                if !wildy_step_ok(cur, nb, allow_wilderness) {
                    continue;
                }
                let nd = n.cost + model.run_per_step;
                if !done.contains(&nb) && dist.get(&nb).is_none_or(|&g| g > nd) {
                    dist.insert(nb, nd);
                    came_from.insert(nb, Back::Walk(cur));
                    heap.push(HeapNode { cost: nd, tile: nb });
                }
            }
        }
        // Transport edges are usable from any standable tile within the
        // interact radius of their `at` — the approach is derived here,
        // never baked. `at` itself is the interact target and may be
        // blocked (a wall loc or NPC); only the take-off tile needs to be
        // standable. Each edge is indexed under its unique `at`, so the
        // fixed offset sweep finds it exactly once per node (a radius-3
        // square may cover several `at` tiles — each is a distinct edge).
        if collision.standable(cur) {
            for dx in -INTERACT_RADIUS..=INTERACT_RADIUS {
                for dz in -INTERACT_RADIUS..=INTERACT_RADIUS {
                    let at = WorldTile {
                        x: cur.x + dx,
                        z: cur.z + dz,
                        level: cur.level,
                    };
                    let Some(idxs) = graph.at.get(&at) else {
                        continue;
                    };
                    for &ei in idxs {
                        let edge = &graph.edges[ei];
                        if !wildy_step_ok(cur, edge.to, allow_wilderness) {
                            continue;
                        }
                        let nd = n.cost + edge.ticks as f64;
                        if !done.contains(&edge.to) && dist.get(&edge.to).is_none_or(|&g| g > nd) {
                            dist.insert(edge.to, nd);
                            came_from.insert(edge.to, Back::Transport { from: cur, ei });
                            heap.push(HeapNode {
                                cost: nd,
                                tile: edge.to,
                            });
                        }
                    }
                }
            }
        }
        // The any-tile teleport layer: every teleport edge leaves from the
        // current node, wherever it is. The landing is trusted like every
        // other transport `to` (no walkability filter — the content
        // declares it).
        if use_teleports {
            for (ti, edge) in graph.teleports.iter().enumerate() {
                if !wildy_step_ok(cur, edge.to, allow_wilderness) {
                    continue;
                }
                let nd = n.cost + edge.ticks as f64;
                if !done.contains(&edge.to) && dist.get(&edge.to).is_none_or(|&g| g > nd) {
                    dist.insert(edge.to, nd);
                    came_from.insert(edge.to, Back::Teleport {
                        from: cur,
                        index: ti,
                    });
                    heap.push(HeapNode {
                        cost: nd,
                        tile: edge.to,
                    });
                }
            }
        }
    }
    Err(RouteError::NoPath)
}

/// Whether the search may move from `cur` onto `next`: without
/// `allow_wilderness` a non-wilderness node may not relax into a
/// wilderness tile (a walk step or a transport landing). Once inside the
/// wilderness the search walks freely — only the entry is gated.
fn wildy_step_ok(cur: WorldTile, next: WorldTile, allow_wilderness: bool) -> bool {
    allow_wilderness || in_wilderness(cur) || !in_wilderness(next)
}

/// Whether a one-tile step from `cur` by `d` is allowed — the client's
/// `tryMove` movement test against the collision bake's derived walkable
/// word, not the blanket `walkable()`. A step into a neighbour clears that
/// neighbour's `PL_WALK_*` mask for the face/corner the step enters
/// through; a diagonal step additionally clears both orthogonal neighbours'
/// cardinal masks, exactly like `tryMove`'s BFS. The derived word carries
/// the `SQ_BLOCKED` base on any wall/scenery/ground tile, so those tiles
/// reject entry from every direction. Every step stays inside the bake's
/// x/z grid (the whole-world mapsquare bbox); each level tests its own
/// plane (an unstamped plane is unrestricted within it).
pub(crate) fn step_ok(collision: &WorldCollision, cur: WorldTile, d: (i32, i32)) -> bool {
    let nb = WorldTile {
        x: cur.x + d.0,
        z: cur.z + d.1,
        level: cur.level,
    };
    let lx = nb.x - collision.origin.x;
    let lz = nb.z - collision.origin.z;
    if lx < 0 || lz < 0 {
        return false;
    }
    if lx as usize >= collision.width || lz as usize >= collision.height {
        return false;
    }
    let f = |x: i32, z: i32| collision.walkable_word(x, z, nb.level);
    match (d.0, d.1) {
        // Cardinal: the destination's face toward `cur`.
        (0, 1) => f(nb.x, nb.z) & MASK_S == 0,
        (0, -1) => f(nb.x, nb.z) & MASK_N == 0,
        (1, 0) => f(nb.x, nb.z) & MASK_W == 0,
        (-1, 0) => f(nb.x, nb.z) & MASK_E == 0,
        // Diagonal: the destination's corner mask plus both orthogonals.
        (-1, -1) => {
            f(nb.x, nb.z) & MASK_NE == 0
                && f(cur.x - 1, cur.z) & MASK_E == 0
                && f(cur.x, cur.z - 1) & MASK_N == 0
        }
        (1, -1) => {
            f(nb.x, nb.z) & MASK_NW == 0
                && f(cur.x + 1, cur.z) & MASK_W == 0
                && f(cur.x, cur.z - 1) & MASK_N == 0
        }
        (-1, 1) => {
            f(nb.x, nb.z) & MASK_SE == 0
                && f(cur.x - 1, cur.z) & MASK_E == 0
                && f(cur.x, cur.z + 1) & MASK_S == 0
        }
        (1, 1) => {
            f(nb.x, nb.z) & MASK_SW == 0
                && f(cur.x + 1, cur.z) & MASK_W == 0
                && f(cur.x, cur.z + 1) & MASK_S == 0
        }
        _ => false,
    }
}

/// How a tile was reached: by a walk step (each costing the search's run
/// rate) from `Walk`'s tile, by transport edge `Transport` (an index
/// into [`TransportGraph::edges`], taken from `from` — the standable tile
/// within the edge's interact radius that it was relaxed from), or by
/// any-tile teleport edge `Teleport` (an index into
/// [`TransportGraph::teleports`], taken from `from` — the node it was
/// relaxed from).
#[derive(Clone, Copy)]
enum Back {
    Walk(WorldTile),
    Transport { from: WorldTile, ei: usize },
    Teleport { from: WorldTile, index: usize },
}

/// Split the backtrack from `to` back to the entry-less origin into legs:
/// consecutive walk tiles collapse into one `Walk` leg per run, and each
/// transport edge is its own `Transport` leg. Returns `(legs, ticks)`
/// where `ticks` is the total: `(walk tiles − 1) × run rate` per walk leg
/// plus each transport edge's ticks.
fn reconstruct(
    to: WorldTile,
    came_from: &HashMap<WorldTile, Back>,
    graph: &TransportGraph,
    model: CostModel,
) -> (Vec<Leg>, f64) {
    // Walk tiles in backtrack order (dest side first).
    let mut walk_rev = vec![to];
    let mut t = to;
    let mut legs_rev: Vec<Leg> = Vec::new();
    let mut ticks = 0.0;
    while let Some(prev) = came_from.get(&t) {
        match *prev {
            Back::Walk(pt) => {
                walk_rev.push(pt);
                t = pt;
            }
            Back::Transport { from, ei } => {
                ticks += walk_ticks(&walk_rev, model);
                walk_rev.reverse();
                legs_rev.push(Leg::Walk { tiles: walk_rev });
                let edge = graph.edges[ei].clone();
                ticks += edge.ticks as f64;
                legs_rev.push(Leg::Transport { edge });
                // The walk leg before the transport resumes from the tile
                // the edge was actually taken on — the standable take-off
                // within the interact radius, never the edge's `at` (which
                // may be a blocked interact target the player cannot stand
                // on).
                t = from;
                walk_rev = vec![t];
            }
            Back::Teleport { from, index } => {
                ticks += walk_ticks(&walk_rev, model);
                walk_rev.reverse();
                legs_rev.push(Leg::Walk { tiles: walk_rev });
                let edge = graph.teleports[index].clone();
                ticks += edge.ticks as f64;
                legs_rev.push(Leg::Transport { edge });
                // The walk leg before the teleport resumes from the tile the
                // teleport was actually taken on (a teleport has no `at`).
                t = from;
                walk_rev = vec![t];
            }
        }
    }
    ticks += walk_ticks(&walk_rev, model);
    walk_rev.reverse();
    legs_rev.push(Leg::Walk { tiles: walk_rev });
    legs_rev.reverse();
    (legs_rev, ticks)
}

/// The tick cost of one collapsed walk leg: `tiles.len() − 1` steps at the
/// model's run rate (the origin tile is stood on, not walked to).
fn walk_ticks(tiles: &[WorldTile], model: CostModel) -> f64 {
    tiles.len().saturating_sub(1) as f64 * model.run_per_step
}

/// Heap entry for relaxations over total tick cost; `Ord` is reversed so
/// the smallest cost pops first, with `total_cmp` giving f64 a total order
/// and tile coordinates as tie-breakers to keep the ordering total.
struct HeapNode {
    cost: f64,
    tile: WorldTile,
}

impl PartialEq for HeapNode {
    fn eq(&self, other: &Self) -> bool {
        self.cost == other.cost && self.tile == other.tile
    }
}
impl Eq for HeapNode {}

impl PartialOrd for HeapNode {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for HeapNode {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .cost
            .total_cmp(&self.cost)
            .then_with(|| self.tile.x.cmp(&other.tile.x))
            .then_with(|| self.tile.z.cmp(&other.tile.z))
            .then_with(|| self.tile.level.cmp(&other.tile.level))
    }
}

// ---------------------------------------------------------------------------
// Legacy step-grid A* (the pre-Task-13 router), kept under `find_on_grid`
// for the traveller and the live harnesses until they move to the
// collision+transport router.
// ---------------------------------------------------------------------------

/// One leg of a step-grid route: a walk segment or a door crossing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GridLeg {
    Walk {
        tiles: Vec<Tile>,
    },
    Door {
        loc: Tile,
        loc_id: i32,
        from: Tile,
        to: Tile,
    },
}

/// A step-grid route from an origin to `dest`, split into legs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GridRoute {
    pub legs: Vec<GridLeg>,
    pub dest: Tile,
}

/// Returned by [`find_on_grid`] when no walkable path connects the two
/// tiles.
#[derive(Debug)]
pub struct NoPath;

/// A* over the 4-neighbour grid (N/E/S/W, cost 1, heuristic chebyshev),
/// extended by directed door edges: from a tile `d.from`, a door lets the
/// route jump to `d.to` at cost 2. Same level only. `from` is assumed to sit
/// on a walkable tile; every tile moved onto must be walkable. Legs split
/// around door crossings: a walk leg up to the door's `from`, the Door leg,
/// then a walk leg onward from its `to`. Each result leg is non-empty; the
/// first walk leg starts at `from` and the last ends at `to`.
pub fn find_on_grid(grid: &StepGrid, from: Tile, to: Tile) -> Result<GridRoute, NoPath> {
    if from.level != to.level {
        return Err(NoPath);
    }
    if from == to {
        return Ok(GridRoute {
            legs: vec![GridLeg::Walk { tiles: vec![from] }],
            dest: to,
        });
    }

    let mut open = BinaryHeap::new();
    let mut best_g: HashMap<Tile, i32> = HashMap::new();
    let mut came_from: HashMap<Tile, GridBack> = HashMap::new();

    best_g.insert(from, 0);
    open.push(GridNode {
        tile: from,
        f: chebyshev(from, to),
    });

    while let Some(GridNode { tile: cur, .. }) = open.pop() {
        if cur == to {
            let legs = reconstruct_on_grid(cur, &came_from);
            return Ok(GridRoute { legs, dest: to });
        }

        let cur_g = best_g[&cur];
        let mut relax = |nb: Tile, cost: i32, back: GridBack| {
            let tentative_g = cur_g + cost;
            if tentative_g < *best_g.get(&nb).unwrap_or(&i32::MAX) {
                came_from.insert(nb, back);
                best_g.insert(nb, tentative_g);
                open.push(GridNode {
                    tile: nb,
                    f: tentative_g + chebyshev(nb, to),
                });
            }
        };
        for nb in [north(cur), east(cur), south(cur), west(cur)] {
            if grid.walkable(nb) {
                relax(nb, 1, GridBack::Walk(cur));
            }
        }
        for d in &grid.doors {
            if d.from == cur && grid.walkable(d.to) {
                relax(d.to, 2, GridBack::Door(*d));
            }
        }
    }

    Err(NoPath)
}

/// Split the A* backtrack from `cur` back to `from` into legs at door
/// crossings. `from` is implicit: backtracking stops when the entry-less
/// start tile is reached, and that tile is already the last element of the
/// final walk segment.
fn reconstruct_on_grid(cur: Tile, came_from: &HashMap<Tile, GridBack>) -> Vec<GridLeg> {
    // Walk tiles accumulated in reverse order (cur-side first).
    let mut walk_rev = vec![cur];
    let mut t = cur;
    let mut legs_rev: Vec<GridLeg> = Vec::new();
    while let Some(prev) = came_from.get(&t) {
        match prev {
            GridBack::Walk(pt) => {
                walk_rev.push(*pt);
                t = *pt;
            }
            GridBack::Door(d) => {
                walk_rev.reverse();
                legs_rev.push(GridLeg::Walk { tiles: walk_rev });
                legs_rev.push(GridLeg::Door {
                    loc: d.loc,
                    loc_id: d.loc_id,
                    from: d.from,
                    to: d.to,
                });
                // The next walk segment runs up to this door's `from`.
                walk_rev = vec![d.from];
                t = d.from;
            }
        }
    }
    walk_rev.reverse();
    legs_rev.push(GridLeg::Walk { tiles: walk_rev });
    legs_rev.reverse();
    legs_rev
}

/// How `came_from`'s key was reached: by a walk step from `Walk`'s tile or
/// by a door crossing recorded in `Door`.
#[derive(Clone, Copy)]
enum GridBack {
    Walk(Tile),
    Door(DoorEdge),
}

fn north(t: Tile) -> Tile {
    Tile {
        x: t.x,
        z: t.z + 1,
        level: t.level,
    }
}
fn east(t: Tile) -> Tile {
    Tile {
        x: t.x + 1,
        z: t.z,
        level: t.level,
    }
}
fn south(t: Tile) -> Tile {
    Tile {
        x: t.x,
        z: t.z - 1,
        level: t.level,
    }
}
fn west(t: Tile) -> Tile {
    Tile {
        x: t.x - 1,
        z: t.z,
        level: t.level,
    }
}

/// Heap entry; `Ord` is reversed so the smallest f pops first, with tile
/// coordinates as tie-breakers to keep the ordering total.
struct GridNode {
    tile: Tile,
    f: i32,
}

impl PartialEq for GridNode {
    fn eq(&self, other: &Self) -> bool {
        self.f == other.f && self.tile == other.tile
    }
}
impl Eq for GridNode {}

impl PartialOrd for GridNode {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for GridNode {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .f
            .cmp(&self.f)
            .then_with(|| self.tile.x.cmp(&other.tile.x))
            .then_with(|| self.tile.z.cmp(&other.tile.z))
            .then_with(|| self.tile.level.cmp(&other.tile.level))
    }
}

#[cfg(test)]
mod tests {
    use api::obj_names::LocDefs;
    use api::snapshot::WorldTile;
    use client::config::LocType;
    use client::dash3d::CollisionFlag;
    use std::collections::HashSet;
    use std::fs;
    use std::path::PathBuf;

    use crate::collision::{bake_from_maps, WorldCollision};
    use crate::grid::StepGrid;
    use crate::router::{
        find, find_allow_teleports, find_bounded, find_on_grid, find_with, find_with_model,
        step_ok, CostModel, FindOptions, GridLeg, Leg, RouteError, PER_STEP_WALK,
    };
    use crate::tile::Tile;
    use crate::transport::{TransportEdge, TransportGraph, TransportKind};

    #[test]
    fn find_on_grid_across_open_3x3_is_a_walk_leg() {
        let g = StepGrid::fixture_open_3x3();
        let r = find_on_grid(
            &g,
            Tile {
                x: 0,
                z: 0,
                level: 0,
            },
            Tile {
                x: 2,
                z: 2,
                level: 0,
            },
        )
        .unwrap();
        assert_eq!(
            r.dest,
            Tile {
                x: 2,
                z: 2,
                level: 0
            }
        );
        let GridLeg::Walk { tiles } = &r.legs[0] else {
            panic!()
        };
        assert_eq!(tiles.first().unwrap().x, 0);
        assert_eq!(tiles.last(), Some(&r.dest));
    }

    #[test]
    fn find_on_grid_through_wall_is_no_path() {
        let mut g = StepGrid::fixture_open_3x3();
        g.set_walkable(
            Tile {
                x: 1,
                z: 0,
                level: 0,
            },
            false,
        );
        g.set_walkable(
            Tile {
                x: 1,
                z: 1,
                level: 0,
            },
            false,
        );
        g.set_walkable(
            Tile {
                x: 1,
                z: 2,
                level: 0,
            },
            false,
        );
        assert!(find_on_grid(
            &g,
            Tile {
                x: 0,
                z: 1,
                level: 0
            },
            Tile {
                x: 2,
                z: 1,
                level: 0
            }
        )
        .is_err());
    }

    #[test]
    fn find_on_grid_uses_door_edge_across_a_wall() {
        let g = StepGrid::fixture_door_corridor();
        let r = find_on_grid(
            &g,
            Tile {
                x: 0,
                z: 0,
                level: 0,
            },
            Tile {
                x: 4,
                z: 0,
                level: 0,
            },
        )
        .unwrap();
        assert!(r
            .legs
            .iter()
            .any(|l| matches!(l, GridLeg::Door { loc_id: 1530, .. })));
    }

    #[test]
    fn door_route_splits_into_walk_door_walk_legs_on_grid() {
        let g = StepGrid::fixture_door_corridor();
        let r = find_on_grid(
            &g,
            Tile {
                x: 0,
                z: 0,
                level: 0,
            },
            Tile {
                x: 4,
                z: 0,
                level: 0,
            },
        )
        .unwrap();
        assert_eq!(r.legs.len(), 3);
        let (
            GridLeg::Walk { tiles: w0 },
            GridLeg::Door {
                loc,
                loc_id,
                from,
                to,
            },
            GridLeg::Walk { tiles: w1 },
        ) = (&r.legs[0], &r.legs[1], &r.legs[2])
        else {
            panic!("expected Walk, Door, Walk legs");
        };
        assert_eq!(
            w0.first(),
            Some(&Tile {
                x: 0,
                z: 0,
                level: 0
            })
        );
        assert_eq!(w0.last(), Some(from));
        assert_eq!(w1.first(), Some(to));
        assert_eq!(
            w1.last(),
            Some(&Tile {
                x: 4,
                z: 0,
                level: 0
            })
        );
        assert_eq!(loc_id, &1530);
        assert_eq!(
            loc,
            &Tile {
                x: 2,
                z: 0,
                level: 0
            }
        );
    }

    // --- Dijkstra router over collision + transport graph ---

    fn tile(x: i32, z: i32, level: i32) -> WorldTile {
        WorldTile { x, z, level }
    }

    /// A `width × height` level-0 bake at (0,0) with the given per-tile
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
            flags: flags.clone(),
            walkable: crate::collision::derive_walkable(&flags),
        }
    }

    /// A scratch mapsquare directory for one fixture, removed on drop.
    struct FixDir(PathBuf);

    impl FixDir {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "274bot-nav-router-{name}-{}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            FixDir(dir)
        }
    }

    impl Drop for FixDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A one-loc `LocDefs` table.
    fn defs(locs: &[LocType]) -> LocDefs {
        LocDefs::from_locs(locs)
    }

    /// A 5×5 grid split by a wall between x=1 and x=2: the client's `W_E` on
    /// column 1 and `W_W` on column 2, so no step (or diagonal) crosses.
    fn walled_5x5() -> WorldCollision {
        let mut extras = Vec::new();
        for z in 0..5 {
            extras.push((1, z, CollisionFlag::W_E as u32));
            extras.push((2, z, CollisionFlag::W_W as u32));
        }
        bake(5, 5, &extras)
    }

    /// The same wall with a door gap at `gap_z`: column 1 carries no `W_E`
    /// there, so the door's `from` tile stays open while column 2's `W_W`
    /// still seals the crossing.
    fn walled_5x5_gap(gap_z: i32) -> WorldCollision {
        let mut extras = Vec::new();
        for z in 0..5 {
            if z != gap_z {
                extras.push((1, z, CollisionFlag::W_E as u32));
            }
            extras.push((2, z, CollisionFlag::W_W as u32));
        }
        bake(5, 5, &extras)
    }

    /// A 5×6 bake (x=0..4, z=0..5) walled between the west and east sides:
    /// column 2 carries `W_W` for every row, column 1 carries `W_E` for
    /// rows 1..=5, and the door's own `at=(2,0)` tile carries
    /// `W_W | WR_GRND` — blocked, sealing the row-0 gap. The only crossing
    /// is the door edge, and `at` itself is never standable. Row 4 (`W_N`
    /// on every column) seals the north strip (z=5) away from the door's
    /// radius-3 neighborhood.
    fn blocked_door_fixture() -> WorldCollision {
        let mut extras = Vec::new();
        for z in 1..=5 {
            extras.push((1, z, CollisionFlag::W_E as u32));
            extras.push((2, z, CollisionFlag::W_W as u32));
        }
        for x in 0..5 {
            extras.push((x, 4, CollisionFlag::W_N as u32));
        }
        extras.push((2, 0, (CollisionFlag::W_W | CollisionFlag::WR_GRND) as u32));
        bake(5, 6, &extras)
    }

    /// One directed door edge `at -> to` (loc 1530, `Open` op 1).
    fn door(at: WorldTile, to: WorldTile, ticks: i32) -> TransportGraph {
        let edge = TransportEdge {
            kind: TransportKind::Door,
            at,
            to,
            loc_id: 1530,
            option: 1,
            ticks,
            dir: None,
            open_loc_id: None,
            skill_req: vec![],
            item_req: vec![],
            quest_req: vec![],
            varp_req: vec![],
        };
        let mut graph = TransportGraph::default();
        graph.at.entry(at).or_default().push(0);
        graph.edges.push(edge);
        graph
    }

    /// One any-tile teleport edge in `graph.teleports` (never in `at`).
    fn teleport(
        to: WorldTile,
        ticks: i32,
        skill_req: Vec<(i32, i32)>,
        item_req: Vec<(i32, i32)>,
    ) -> TransportGraph {
        let mut graph = TransportGraph::default();
        graph.teleports.push(TransportEdge {
            kind: TransportKind::Teleport,
            at: tile(0, 0, 0),
            to,
            loc_id: 0,
            option: 0,
            ticks,
            dir: None,
            open_loc_id: None,
            skill_req,
            item_req,
            quest_req: vec![],
            varp_req: vec![],
        });
        graph
    }

    #[test]
    fn find_across_open_room_is_a_single_walk_leg() {
        let wc = bake(5, 5, &[]);
        let g = TransportGraph::default();
        let r = find(&wc, &g, tile(0, 0, 0), tile(4, 4, 0)).unwrap();
        assert_eq!(r.dest, tile(4, 4, 0));
        assert_eq!(r.ticks, 2.0); // 4 run steps at 0.5 ticks each
        assert_eq!(r.legs.len(), 1);
        let Leg::Walk { tiles } = &r.legs[0] else {
            panic!("walk-only route");
        };
        assert_eq!(tiles.first(), Some(&tile(0, 0, 0)));
        assert_eq!(tiles.last(), Some(&tile(4, 4, 0)));
    }

    #[test]
    fn find_from_equals_to_is_a_single_tile_walk() {
        let wc = bake(3, 3, &[]);
        let g = TransportGraph::default();
        let r = find(&wc, &g, tile(1, 1, 0), tile(1, 1, 0)).unwrap();
        assert_eq!(r.ticks, 0.0); // no steps walked
        assert_eq!(r.legs, vec![Leg::Walk { tiles: vec![tile(1, 1, 0)] }]);
    }

    #[test]
    fn find_through_an_unbroken_wall_is_no_path() {
        let wc = walled_5x5();
        let g = TransportGraph::default();
        assert!(matches!(
            find(&wc, &g, tile(0, 0, 0), tile(4, 4, 0)),
            Err(RouteError::NoPath)
        ));
    }

    #[test]
    fn router_uses_transport_across_a_wall() {
        // The wall has a door gap at z=2: the door's from tile stays open,
        // but the wall's own stamps otherwise seal the crossing.
        let wc = walled_5x5_gap(2);
        let g = door(tile(1, 2, 0), tile(2, 2, 0), 2);
        let r = find(&wc, &g, tile(0, 0, 0), tile(4, 4, 0)).unwrap();
        assert_eq!(r.dest, tile(4, 4, 0));
        // The origin sits within the door's interact radius of at=(1,2),
        // so the door is taken straight from it (2.0) plus 3 walk steps
        // from its far side (1.0, two run steps).
        assert_eq!(r.ticks, 3.0);
        assert_eq!(r.legs.len(), 3);
        let (
            Leg::Walk { tiles: w0 },
            Leg::Transport { edge },
            Leg::Walk { tiles: w1 },
        ) = (&r.legs[0], &r.legs[1], &r.legs[2])
        else {
            panic!("expected Walk, Transport, Walk legs");
        };
        assert_eq!(w0, &vec![tile(0, 0, 0)]);
        assert_eq!(edge.loc_id, 1530);
        assert_eq!(edge.ticks, 2);
        assert_eq!(edge.at, tile(1, 2, 0));
        assert_eq!(edge.to, tile(2, 2, 0));
        assert_eq!(w1.first(), Some(&tile(2, 2, 0)));
        assert_eq!(w1.last(), Some(&tile(4, 4, 0)));
    }

    #[test]
    fn find_prefers_a_cheap_walk_over_a_costly_transport() {
        let wc = bake(5, 5, &[]);
        let g = door(tile(0, 0, 0), tile(4, 4, 0), 1000);
        let r = find(&wc, &g, tile(0, 0, 0), tile(4, 4, 0)).unwrap();
        // The 1000-tick door loses to the 2.0-tick walk (4 run steps).
        assert_eq!(r.ticks, 2.0);
        assert_eq!(r.legs.len(), 1);
        assert!(matches!(&r.legs[0], Leg::Walk { .. }));
    }

    /// The blocked interact target: the door's `at=(2,0)` tile carries a
    /// footprint/ground block, so it is neither standable nor walkable and
    /// the old `at`-only expansion could never settle a node on it. The
    /// neighborhood expansion must take the door from a *neighbouring*
    /// standable tile instead.
    #[test]
    fn router_takes_a_transport_from_a_standable_tile_within_the_interact_radius() {
        let wc = blocked_door_fixture();
        let g = door(tile(2, 0, 0), tile(2, 2, 0), 2);
        let r = find(&wc, &g, tile(1, 0, 0), tile(4, 2, 0)).unwrap();
        assert_eq!(r.dest, tile(4, 2, 0));
        // The 2-tick door taken from (1,0) + 2 run steps from (2,2) (1.0).
        assert_eq!(r.ticks, 3.0);
        let (
            Leg::Walk { tiles: w0 },
            Leg::Transport { edge },
            Leg::Walk { tiles: w1 },
        ) = (&r.legs[0], &r.legs[1], &r.legs[2])
        else {
            panic!("expected Walk, Transport, Walk legs");
        };
        // The walk leg before the door ends at the take-off tile (1,0) —
        // a standable tile within the interact radius of `at`, not `at`
        // itself.
        assert_eq!(w0, &vec![tile(1, 0, 0)]);
        assert_eq!(edge.loc_id, 1530);
        assert_eq!(edge.ticks, 2);
        assert_eq!(edge.at, tile(2, 0, 0));
        assert_eq!(edge.to, tile(2, 2, 0));
        assert_eq!(w1.first(), Some(&tile(2, 2, 0)));
        assert_eq!(w1.last(), Some(&tile(4, 2, 0)));
        // Never steps onto the blocked `at` tile.
        let stepped: Vec<WorldTile> = r
            .legs
            .iter()
            .flat_map(|l| match l {
                Leg::Walk { tiles } => tiles.clone(),
                Leg::Transport { .. } => vec![],
            })
            .collect();
        assert!(!stepped.contains(&tile(2, 0, 0)));
    }

    #[test]
    fn router_does_not_use_a_transport_from_beyond_the_interact_radius() {
        let wc = blocked_door_fixture();
        let g = door(tile(2, 0, 0), tile(2, 2, 0), 2);
        // (1,5) sits at chebyshev 5 from `at=(2,0)` and is sealed away from
        // every within-radius tile by the row-4 wall, so the door stays
        // unusable: no path reaches the east side.
        assert!(matches!(
            find(&wc, &g, tile(1, 5, 0), tile(4, 2, 0)),
            Err(RouteError::NoPath)
        ));
        // A same-strip destination routes by walking, never via the door.
        let r = find(&wc, &g, tile(1, 5, 0), tile(0, 5, 0)).unwrap();
        assert_eq!(r.ticks, 0.5);
        assert!(r.legs.iter().all(|l| matches!(l, Leg::Walk { .. })));
    }

    /// A south face flag on the middle tile blocks entering it from the
    /// south, not from the north — a 1-wide corridor stays a corridor.
    #[test]
    fn find_face_flags_block_only_the_matching_direction() {
        let wc = bake(1, 3, &[(0, 1, CollisionFlag::W_S as u32)]);
        let g = TransportGraph::default();
        assert!(
            matches!(
                find(&wc, &g, tile(0, 0, 0), tile(0, 2, 0)),
                Err(RouteError::NoPath)
            ),
            "cannot enter the W_S tile from the south"
        );
        let r = find(&wc, &g, tile(0, 2, 0), tile(0, 0, 0)).expect("north-to-south still walks");
        assert!(r.legs.iter().all(|l| matches!(l, Leg::Walk { .. })));
    }

    /// The wall-tile fixture from the live `nav_door` trace: wall 980
    /// (WALL_STRAIGHT, south) at (2816,3437) and door 1530 (WALL_STRAIGHT,
    /// north) at (2816,3438) — m44_53 `0 0 45: 980 0 3` and
    /// `0 0 46: 1530 0 1`. `step_ok` must reject every step into the wall
    /// tile (the east step the live walker took) and the closed door, while
    /// genuinely open neighbours still pass, and the router never routes
    /// onto the wall tile.
    #[test]
    fn wall_tile_blocks_through_wall_steps_and_the_router_avoids_it() {
        let fix = FixDir::new("wall-980-door-1530");
        fs::write(
            fix.0.join("m43_53.jm2"),
            "==== MAP ====\n0 63 44: h1 u50\n0 63 45: h1 u50\n0 63 46: h1 u50\n==== LOC ====\n",
        )
        .unwrap();
        fs::write(
            fix.0.join("m44_53.jm2"),
            "==== MAP ====\n0 0 43: h1 u50\n0 0 44: h10 u50\n0 0 45: h19 o10 u48\n0 0 46: h30 o10 u48\n0 0 47: h30 o5 f4 u50\n==== LOC ====\n0 0 45: 980 0 3\n0 0 46: 1530 0 1\n",
        )
        .unwrap();
        let locs = defs(&[
            LocType {
                id: 980,
                blockwalk: true,
                ..LocType::default()
            },
            LocType {
                id: 1530,
                blockwalk: true,
                ..LocType::default()
            },
        ]);
        let mut door_ids = HashSet::new();
        door_ids.insert(1530);
        let wc = bake_from_maps(&fix.0, &locs, &door_ids).unwrap();
        let g = TransportGraph::default();

        // South face of wall 980: cannot enter that tile from the south.
        // Entering it from the west is a walk along the wall, not through it.
        assert!(!step_ok(&wc, tile(2816, 3436, 0), (0, 1)));
        assert!(step_ok(&wc, tile(2815, 3437, 0), (1, 0)));
        assert!(!step_ok(&wc, tile(2816, 3438, 0), (0, 1)));
        // A genuinely open neighbour still passes.
        assert!(step_ok(&wc, tile(2815, 3437, 0), (0, 1)));
        assert!(step_ok(&wc, tile(2815, 3437, 0), (-1, 0)));

        let r = find(&wc, &g, tile(2813, 3436, 0), tile(2815, 3438, 0)).unwrap();
        let Leg::Walk { tiles } = &r.legs[0] else {
            panic!("walk-only route");
        };
        // Crossing the wall's south face is still rejected; the path stays
        // on the open side.
        assert!(!tiles.contains(&tile(2816, 3436, 0)));
    }

    #[test]
    fn find_transport_changes_level_and_walks_upstairs() {
        let wc = bake(4, 4, &[]);
        let ladder = TransportEdge {
            kind: TransportKind::Ladder,
            at: tile(0, 0, 0),
            to: tile(1, 1, 1),
            loc_id: 1747,
            option: 1,
            ticks: 3,
            dir: None,
            open_loc_id: None,
            skill_req: vec![],
            item_req: vec![],
            quest_req: vec![],
            varp_req: vec![],
        };
        let mut g = TransportGraph::default();
        g.at.entry(ladder.at).or_default().push(0);
        g.edges.push(ladder.clone());
        let r = find(&wc, &g, tile(0, 0, 0), tile(3, 1, 1)).unwrap();
        assert_eq!(r.dest, tile(3, 1, 1));
        // The 3-tick ladder plus 2 run steps on level 1 (1.0).
        assert_eq!(r.ticks, 4.0);
        let (
            Leg::Walk { tiles: w0 },
            Leg::Transport { edge },
            Leg::Walk { tiles: w1 },
        ) = (&r.legs[0], &r.legs[1], &r.legs[2])
        else {
            panic!("expected Walk, Transport, Walk legs");
        };
        assert_eq!(w0, &vec![tile(0, 0, 0)]);
        assert_eq!(edge, &ladder);
        assert_eq!(w1.first(), Some(&tile(1, 1, 1)));
        assert_eq!(w1.last(), Some(&tile(3, 1, 1)));
    }

    #[test]
    fn find_exhausts_the_node_budget_before_giving_up() {
        let wc = bake(10, 10, &[]);
        let g = TransportGraph::default();
        assert!(matches!(
            find_bounded(
                &wc,
                &g,
                tile(0, 0, 0),
                tile(9, 9, 0),
                CostModel::running(),
                8,
            ),
            Err(RouteError::BudgetExhausted)
        ));
        let r = find_bounded(
            &wc,
            &g,
            tile(0, 0, 0),
            tile(9, 9, 0),
            CostModel::running(),
            4096,
        )
        .unwrap();
        assert_eq!(r.dest, tile(9, 9, 0));
    }

    #[test]
    fn find_prefers_a_cheap_door_over_a_long_walk_around() {
        // A 5×20 bake walled between x=1 and x=2 for z=1..=18 with a door
        // gap at z=10: crossing on foot means walking 20 tiles around the
        // wall ends (~10 ticks at the run rate), so the 1-tick door at
        // mid-wall is the cheaper total-tick route.
        let mut extras = Vec::new();
        for z in 1..=18 {
            if z != 10 {
                extras.push((1, z, CollisionFlag::W_E as u32));
            }
            extras.push((2, z, CollisionFlag::W_W as u32));
        }
        let wc = bake(5, 20, &extras);
        let g = door(tile(1, 10, 0), tile(2, 10, 0), 1);
        let r = find(&wc, &g, tile(0, 10, 0), tile(4, 10, 0)).unwrap();
        assert_eq!(r.dest, tile(4, 10, 0));
        // The origin sits within the door's interact radius of at=(1,10),
        // so the 1-tick door is taken from it (1.0) plus 2 walk tiles from
        // its far side (1.0).
        assert_eq!(r.ticks, 2.0);
        assert!(r.legs.iter().any(|l| matches!(l, Leg::Transport { .. })));
    }

    #[test]
    fn find_prefers_walking_around_over_a_cheap_door() {
        // A single west-face flag at (2,2): walking around is 3 run steps
        // (1.5 ticks), cheaper than the 2-tick door, so the router walks.
        let wc = bake(5, 5, &[(2, 2, CollisionFlag::W_W as u32)]);
        let g = door(tile(1, 2, 0), tile(2, 2, 0), 2);
        let r = find(&wc, &g, tile(0, 2, 0), tile(3, 2, 0)).unwrap();
        assert_eq!(r.ticks, 1.5);
        assert!(r.legs.iter().all(|l| matches!(l, Leg::Walk { .. })));
    }

    #[test]
    fn walk_only_route_ticks_are_the_walk_tick_cost() {
        // Walking is no longer free: a walk-only route's `ticks` is the
        // walk cost (0.5 per tile at the run rate), not 0.
        let wc = bake(5, 5, &[]);
        let g = TransportGraph::default();
        let r = find(&wc, &g, tile(0, 0, 0), tile(4, 4, 0)).unwrap();
        assert_eq!(r.ticks, 2.0);
        assert!(r.legs.iter().all(|l| matches!(l, Leg::Walk { .. })));
    }

    #[test]
    fn find_cost_model_sets_the_walk_rate_per_search() {
        // The run-vs-walk rate is a per-search input: the same 4-tile walk
        // costs 2 ticks at the running pace (0.5/tile) and 4 at the walking
        // pace (1/tile).
        let wc = bake(5, 5, &[]);
        let g = TransportGraph::default();
        let run = find_with_model(&wc, &g, tile(0, 0, 0), tile(4, 4, 0), CostModel::running())
            .unwrap();
        assert_eq!(run.ticks, 2.0);
        let walk = CostModel {
            run_per_step: PER_STEP_WALK,
            walk_per_step: PER_STEP_WALK,
        };
        let r = find_with_model(&wc, &g, tile(0, 0, 0), tile(4, 4, 0), walk).unwrap();
        assert_eq!(r.ticks, 4.0);
    }

    // --- allow_teleports: the any-tile teleport layer ---

    #[test]
    fn find_never_uses_a_spell_teleport_but_find_allow_teleports_does() {
        // The wall splits the 5×5 bake; only the any-tile spell teleport can
        // cross it, and only when allow_teleports is on.
        let wc = walled_5x5();
        let dest = tile(4, 4, 0);
        let g = teleport(
            dest,
            3, // OP_BASE 1 + the cast p_delay(2)
            vec![(6, 25)], // Magic level 25 (Varrock)
            vec![(554, 1), (556, 3), (563, 1)], // fire + air + law runes
        );
        assert!(matches!(
            find(&wc, &g, tile(0, 0, 0), dest),
            Err(RouteError::NoPath)
        ));
        let r = find_allow_teleports(&wc, &g, tile(0, 0, 0), dest).unwrap();
        assert_eq!(r.dest, dest);
        // Teleported from the origin — no walk, just the cast.
        assert_eq!(r.ticks, 3.0);
        let leg = r
            .legs
            .iter()
            .find(|l| matches!(l, Leg::Transport { .. }))
            .expect("a teleport leg");
        let Leg::Transport { edge } = leg else {
            unreachable!()
        };
        assert_eq!(edge.kind, TransportKind::Teleport);
        assert_eq!(edge.skill_req, vec![(6, 25)]);
        assert_eq!(edge.item_req, vec![(554, 1), (556, 3), (563, 1)]);
        assert_eq!(edge.to, dest);
        assert_eq!(edge.ticks, 3);
    }

    #[test]
    fn find_never_uses_a_jewellery_teleport_by_default() {
        let wc = walled_5x5();
        let dest = tile(4, 4, 0);
        let g = teleport(dest, 2, vec![], vec![(1712, 1)]); // charged glory
        assert!(matches!(
            find(&wc, &g, tile(0, 0, 0), dest),
            Err(RouteError::NoPath)
        ));
        let r = find_allow_teleports(&wc, &g, tile(0, 0, 0), dest).unwrap();
        assert_eq!(r.ticks, 2.0); // OP_BASE 1 + the rub p_delay(1)
        let Leg::Transport { edge } = r
            .legs
            .iter()
            .find(|l| matches!(l, Leg::Transport { .. }))
            .unwrap()
        else {
            unreachable!()
        };
        assert_eq!(edge.item_req, vec![(1712, 1)]); // the charged item
        assert!(edge.skill_req.is_empty());
    }

    #[test]
    fn find_allow_teleports_is_usable_from_any_tile() {
        let wc = walled_5x5();
        let dest = tile(4, 4, 0);
        let g = teleport(dest, 2, vec![], vec![(1712, 1)]);
        for origin in [tile(0, 0, 0), tile(1, 3, 0)] {
            let r = find_allow_teleports(&wc, &g, origin, dest).unwrap();
            assert_eq!(r.dest, dest);
            assert_eq!(r.ticks, 2.0, "teleport from {origin:?} costs the rub only");
            assert!(r
                .legs
                .iter()
                .any(|l| matches!(l, Leg::Transport { .. })));
        }
    }

    #[test]
    fn find_allow_teleports_still_prefers_walking_when_cheaper() {
        // Total-tick cost still governs: on an open 5×5 the 4 run steps
        // (2.0) beat the 3-tick teleport, so the route walks.
        let wc = bake(5, 5, &[]);
        let dest = tile(4, 4, 0);
        let g = teleport(dest, 3, vec![(6, 25)], vec![]);
        let r = find_allow_teleports(&wc, &g, tile(0, 0, 0), dest).unwrap();
        assert_eq!(r.ticks, 2.0);
        assert!(r.legs.iter().all(|l| matches!(l, Leg::Walk { .. })));
    }

    // --- wilderness opt-in (FindOptions.allow_wilderness) ---

    /// A `width × height` all-open level-0 bake at `origin` (flags all 0,
    /// walkable derived).
    fn open_world(origin: WorldTile, width: usize, height: usize) -> WorldCollision {
        let flags = vec![0u32; width * height];
        WorldCollision {
            origin,
            width,
            height,
            walkable: crate::collision::derive_walkable(&flags),
            flags,
        }
    }

    #[test]
    fn find_does_not_enter_wilderness_without_the_flag() {
        let wc = open_world(WorldTile { x: 3099, z: 3518, level: 0 }, 5, 12);
        let g = TransportGraph::default();
        let from = WorldTile { x: 3100, z: 3519, level: 0 }; // z 3519 < 3520
        let to = WorldTile { x: 3100, z: 3525, level: 0 };   // in zone
        assert!(matches!(find(&wc, &g, from, to), Err(RouteError::NoPath)));
        let ok = find_with(
            &wc,
            &g,
            from,
            to,
            FindOptions {
                allow_teleports: false,
                allow_wilderness: true,
            },
        );
        assert!(ok.is_ok());
    }

    #[test]
    fn already_in_wilderness_can_walk_out_without_the_flag() {
        let wc = open_world(WorldTile { x: 3099, z: 3518, level: 0 }, 5, 12);
        let g = TransportGraph::default();
        let from = WorldTile { x: 3100, z: 3525, level: 0 };
        let to = WorldTile { x: 3100, z: 3519, level: 0 };
        assert!(find(&wc, &g, from, to).is_ok());
    }

    #[test]
    fn find_allow_teleports_still_refuses_a_wilderness_landing() {
        // A walled 5×12 bake at (3099,3518): only the any-tile teleport
        // crosses the wall, but its landing (3102,3525) is inside the
        // zone. Default find refuses (wall), allow_teleports alone still
        // refuses (the wildy landing), and both flags together route.
        let mut flags = vec![0u32; 5 * 12];
        for z in 0..12 {
            flags[z * 5 + 1] |= CollisionFlag::W_E as u32;
            flags[z * 5 + 2] |= CollisionFlag::W_W as u32;
        }
        let wc = WorldCollision {
            origin: WorldTile {
                x: 3099,
                z: 3518,
                level: 0,
            },
            width: 5,
            height: 12,
            walkable: crate::collision::derive_walkable(&flags),
            flags,
        };
        let dest = tile(3102, 3525, 0);
        let g = teleport(dest, 3, vec![(6, 25)], vec![(554, 1), (556, 3), (563, 1)]);
        let from = tile(3100, 3519, 0);
        assert!(matches!(find(&wc, &g, from, dest), Err(RouteError::NoPath)));
        assert!(
            matches!(
                find_allow_teleports(&wc, &g, from, dest),
                Err(RouteError::NoPath)
            ),
            "a teleport landing inside the wilderness must stay refused"
        );
        let ok = find_with(
            &wc,
            &g,
            from,
            dest,
            FindOptions {
                allow_teleports: true,
                allow_wilderness: true,
            },
        );
        assert!(ok.is_ok());
    }
}
