use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

use crate::grid::{DoorEdge, StepGrid};
use crate::tile::{chebyshev, Tile};

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

