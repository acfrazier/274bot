//! Dijkstra routing over the whole-world collision bake (`WorldCollision`)
//! and the content-derived transport graph (`TransportGraph`), plus the
//! legacy step-grid A* kept under [`find_on_grid`] for the traveller and
//! live harnesses until the collision+transport router replaces them.
//!
//! Walking steps are cost 0 (0-1 BFS via a deque); a transport edge costs
//! its `ticks` (a min-heap). A step into a neighbour is allowed only when
//! the neighbour's flags pass the client's directional movement test (the
//! `PL_WALK_*` masks in `tryMove`), never the blanket `walkable()` check.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, HashSet, VecDeque};

use api::snapshot::WorldTile;
use client::dash3d::CollisionFlag;

use crate::collision::WorldCollision;
use crate::grid::{DoorEdge, StepGrid};
use crate::tile::{chebyshev, Tile};
use crate::transport::{TransportEdge, TransportGraph};

/// One leg of a route: a walk run or one transport crossing. Consecutive
/// walk tiles collapse into a single `Walk` leg; each transport edge is
/// its own `Transport` leg.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Leg {
    Walk { tiles: Vec<WorldTile> },
    Transport { edge: TransportEdge },
}

/// A route from the `find` origin to `dest`. `ticks` is the sum of the
/// transport legs' tick costs; walking is cost 0.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Route {
    pub legs: Vec<Leg>,
    pub dest: WorldTile,
    pub ticks: i32,
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

/// Dijkstra over `collision` walk steps (cost 0, deque) and `graph`
/// transport edges (cost `edge.ticks`, min-heap). Walk steps are
/// 8-directional and stay on `from`'s level; transports may change level.
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
    find_bounded(collision, graph, from, to, NODE_BUDGET)
}

/// [`find`] with an explicit node-expansion budget (the search gives up
/// with [`RouteError::BudgetExhausted`] once `budget` tiles are settled).
fn find_bounded(
    collision: &WorldCollision,
    graph: &TransportGraph,
    from: WorldTile,
    to: WorldTile,
    budget: usize,
) -> Result<Route, RouteError> {
    if from == to {
        return Ok(Route {
            legs: vec![Leg::Walk { tiles: vec![from] }],
            dest: to,
            ticks: 0,
        });
    }

    let mut dist: HashMap<WorldTile, i32> = HashMap::new();
    let mut came_from: HashMap<WorldTile, Back> = HashMap::new();
    let mut deque: VecDeque<WorldTile> = VecDeque::new();
    let mut heap: BinaryHeap<HeapNode> = BinaryHeap::new();
    let mut done: HashSet<WorldTile> = HashSet::new();

    dist.insert(from, 0);
    deque.push_back(from);

    let mut expanded = 0usize;
    while !deque.is_empty() || !heap.is_empty() {
        // The deque holds the 0-cost states at the current minimum cost
        // and the heap the positive-cost frontier, so states pop in
        // non-decreasing cost order and the first pop settles a tile's
        // distance (stale heap entries are skipped by the cost check).
        let (cur, cost) = if let Some(t) = deque.pop_front() {
            (t, dist[&t])
        } else {
            let n = heap.pop().expect("heap non-empty when deque is empty");
            if dist.get(&n.tile) != Some(&n.cost) {
                continue;
            }
            (n.tile, n.cost)
        };
        if !done.insert(cur) {
            continue;
        }
        expanded += 1;
        if expanded > budget {
            return Err(RouteError::BudgetExhausted);
        }
        if cur == to {
            let (legs, ticks) = reconstruct(to, &came_from, graph);
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
                if !done.contains(&nb) && dist.get(&nb).is_none_or(|&g| g > cost) {
                    dist.insert(nb, cost);
                    came_from.insert(nb, Back::Walk(cur));
                    deque.push_front(nb);
                }
            }
        }
        if let Some(idxs) = graph.from.get(&cur) {
            for &ei in idxs {
                let edge = &graph.edges[ei];
                let nd = cost + edge.ticks;
                if !done.contains(&edge.to) && dist.get(&edge.to).is_none_or(|&g| g > nd) {
                    dist.insert(edge.to, nd);
                    came_from.insert(edge.to, Back::Transport(ei));
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

/// Whether a one-tile step from `cur` by `d` is allowed — the client's
/// `tryMove` movement test, not the blanket `walkable()` (which is stricter
/// than the client: a tile with only, say, a `W_S` face flag is still
/// standable). A step into a neighbour clears that neighbour's `PL_WALK_*`
/// mask for the face/corner the step enters through; a diagonal step
/// additionally clears both orthogonal neighbours' cardinal masks, exactly
/// like `tryMove`'s BFS. Every step stays inside the bake's x/z grid (the
/// whole-world mapsquare bbox); on other levels the bake carries no flags
/// yet — transports may land there — so steps are unrestricted within that
/// plane.
fn step_ok(collision: &WorldCollision, cur: WorldTile, d: (i32, i32)) -> bool {
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
    let f = |x: i32, z: i32| collision.flag(x, z, nb.level);
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

/// How a tile was reached: by a 0-cost walk step from `Walk`'s tile or by
/// transport edge `Transport` (an index into [`TransportGraph::edges`]).
#[derive(Clone, Copy)]
enum Back {
    Walk(WorldTile),
    Transport(usize),
}

/// Split the backtrack from `to` back to the entry-less origin into legs:
/// consecutive walk tiles collapse into one `Walk` leg per run, and each
/// transport edge is its own `Transport` leg. Returns `(legs, ticks)`.
fn reconstruct(
    to: WorldTile,
    came_from: &HashMap<WorldTile, Back>,
    graph: &TransportGraph,
) -> (Vec<Leg>, i32) {
    // Walk tiles in backtrack order (dest side first).
    let mut walk_rev = vec![to];
    let mut t = to;
    let mut legs_rev: Vec<Leg> = Vec::new();
    let mut ticks = 0;
    while let Some(prev) = came_from.get(&t) {
        match *prev {
            Back::Walk(pt) => {
                walk_rev.push(pt);
                t = pt;
            }
            Back::Transport(ei) => {
                walk_rev.reverse();
                legs_rev.push(Leg::Walk { tiles: walk_rev });
                let edge = graph.edges[ei].clone();
                ticks += edge.ticks;
                legs_rev.push(Leg::Transport { edge });
                t = graph.edges[ei].from;
                walk_rev = vec![t];
            }
        }
    }
    walk_rev.reverse();
    legs_rev.push(Leg::Walk { tiles: walk_rev });
    legs_rev.reverse();
    (legs_rev, ticks)
}

/// Heap entry for transport relaxations; `Ord` is reversed so the smallest
/// cost pops first, with tile coordinates as tie-breakers to keep the
/// ordering total.
struct HeapNode {
    cost: i32,
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
            .cmp(&self.cost)
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
    use api::snapshot::WorldTile;
    use client::dash3d::CollisionFlag;

    use crate::collision::WorldCollision;
    use crate::grid::StepGrid;
    use crate::router::{find, find_bounded, find_on_grid, GridLeg, Leg, RouteError};
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
    /// flags OR'd in.
    fn bake(width: usize, height: usize, extras: &[(i32, i32, u32)]) -> WorldCollision {
        let mut flags = vec![0u32; width * height];
        for &(x, z, f) in extras {
            flags[z as usize * width + x as usize] |= f;
        }
        WorldCollision {
            origin: tile(0, 0, 0),
            width,
            height,
            flags,
        }
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

    /// One directed door edge `from -> to` (loc 1530, `Open` op 1).
    fn door(from: WorldTile, to: WorldTile, ticks: i32) -> TransportGraph {
        let edge = TransportEdge {
            kind: TransportKind::Door,
            from,
            to,
            loc_id: 1530,
            option: 1,
            ticks,
            skill_req: vec![],
            item_req: vec![],
            quest_req: vec![],
            varp_req: vec![],
        };
        let mut graph = TransportGraph::default();
        graph.from.entry(from).or_default().push(0);
        graph.edges.push(edge);
        graph
    }

    #[test]
    fn find_across_open_room_is_a_single_walk_leg() {
        let wc = bake(5, 5, &[]);
        let g = TransportGraph::default();
        let r = find(&wc, &g, tile(0, 0, 0), tile(4, 4, 0)).unwrap();
        assert_eq!(r.dest, tile(4, 4, 0));
        assert_eq!(r.ticks, 0);
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
        assert_eq!(r.ticks, 0);
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
        let wc = walled_5x5();
        let g = door(tile(1, 2, 0), tile(2, 2, 0), 2);
        let r = find(&wc, &g, tile(0, 0, 0), tile(4, 4, 0)).unwrap();
        assert_eq!(r.dest, tile(4, 4, 0));
        assert_eq!(r.ticks, 2);
        assert_eq!(r.legs.len(), 3);
        let (
            Leg::Walk { tiles: w0 },
            Leg::Transport { edge },
            Leg::Walk { tiles: w1 },
        ) = (&r.legs[0], &r.legs[1], &r.legs[2])
        else {
            panic!("expected Walk, Transport, Walk legs");
        };
        assert_eq!(w0.first(), Some(&tile(0, 0, 0)));
        assert_eq!(w0.last(), Some(&tile(1, 2, 0)));
        assert_eq!(edge.loc_id, 1530);
        assert_eq!(edge.ticks, 2);
        assert_eq!(edge.from, tile(1, 2, 0));
        assert_eq!(edge.to, tile(2, 2, 0));
        assert_eq!(w1.first(), Some(&tile(2, 2, 0)));
        assert_eq!(w1.last(), Some(&tile(4, 4, 0)));
    }

    #[test]
    fn find_prefers_free_walk_over_a_costly_transport() {
        let wc = bake(5, 5, &[]);
        let g = door(tile(0, 0, 0), tile(4, 4, 0), 1000);
        let r = find(&wc, &g, tile(0, 0, 0), tile(4, 4, 0)).unwrap();
        assert_eq!(r.ticks, 0);
        assert_eq!(r.legs.len(), 1);
        assert!(matches!(&r.legs[0], Leg::Walk { .. }));
    }

    /// A tile with only a `W_S` face flag is still standable: the client's
    /// movement test checks only the face the step enters through, so a
    /// route may pass through it from the north, but the reverse direction
    /// steps into the blocked face and cannot.
    #[test]
    fn find_steps_respect_directional_face_flags() {
        // A 1-tile-wide column: the only route through (0,1) is vertical.
        let wc = bake(1, 3, &[(0, 1, CollisionFlag::W_S as u32)]);
        let g = TransportGraph::default();
        let r = find(&wc, &g, tile(0, 2, 0), tile(0, 0, 0)).unwrap();
        let Leg::Walk { tiles } = &r.legs[0] else {
            panic!("walk-only route");
        };
        assert_eq!(tiles, &vec![tile(0, 2, 0), tile(0, 1, 0), tile(0, 0, 0)]);
        assert!(matches!(
            find(&wc, &g, tile(0, 0, 0), tile(0, 2, 0)),
            Err(RouteError::NoPath)
        ));
    }

    #[test]
    fn find_transport_changes_level_and_walks_upstairs() {
        let wc = bake(4, 4, &[]);
        let ladder = TransportEdge {
            kind: TransportKind::Ladder,
            from: tile(0, 0, 0),
            to: tile(1, 1, 1),
            loc_id: 1747,
            option: 1,
            ticks: 3,
            skill_req: vec![],
            item_req: vec![],
            quest_req: vec![],
            varp_req: vec![],
        };
        let mut g = TransportGraph::default();
        g.from.entry(ladder.from).or_default().push(0);
        g.edges.push(ladder.clone());
        let r = find(&wc, &g, tile(0, 0, 0), tile(3, 1, 1)).unwrap();
        assert_eq!(r.dest, tile(3, 1, 1));
        assert_eq!(r.ticks, 3);
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
            find_bounded(&wc, &g, tile(0, 0, 0), tile(9, 9, 0), 8),
            Err(RouteError::BudgetExhausted)
        ));
        let r = find_bounded(&wc, &g, tile(0, 0, 0), tile(9, 9, 0), 4096).unwrap();
        assert_eq!(r.dest, tile(9, 9, 0));
    }
}
