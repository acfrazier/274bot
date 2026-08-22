//! A* walk-only routing on the step grid.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

use crate::grid::StepGrid;
use crate::tile::{chebyshev, Tile};

/// One leg of a route. v1 only has walk legs; door legs arrive in Task 3.
pub enum Leg {
    Walk { tiles: Vec<Tile> },
}

/// A route from an origin to `dest`, split into legs.
pub struct Route {
    pub legs: Vec<Leg>,
    pub dest: Tile,
}

/// Returned by [`find`] when no walkable path connects the two tiles.
#[derive(Debug)]
pub struct NoPath;

/// A* over the 4-neighbour grid (N/E/S/W, cost 1, heuristic chebyshev), same
/// level only. `from` is assumed to sit on a walkable tile; every neighbour
/// moved onto must be walkable. The returned walk leg runs from `from`
/// through `dest`, all on `from.level`.
pub fn find(grid: &StepGrid, from: Tile, to: Tile) -> Result<Route, NoPath> {
    if from.level != to.level {
        return Err(NoPath);
    }
    if from == to {
        return Ok(Route {
            legs: vec![Leg::Walk { tiles: vec![from] }],
            dest: to,
        });
    }

    let mut open = BinaryHeap::new();
    let mut best_g: HashMap<Tile, i32> = HashMap::new();
    let mut came_from: HashMap<Tile, Tile> = HashMap::new();

    best_g.insert(from, 0);
    open.push(Node {
        tile: from,
        f: chebyshev(from, to),
    });

    while let Some(Node { tile: cur, .. }) = open.pop() {
        if cur == to {
            let mut tiles = vec![cur];
            let mut t = cur;
            while let Some(&prev) = came_from.get(&t) {
                tiles.push(prev);
                t = prev;
            }
            tiles.reverse();
            return Ok(Route {
                legs: vec![Leg::Walk { tiles }],
                dest: to,
            });
        }

        let cur_g = best_g[&cur];
        for nb in [north(cur), east(cur), south(cur), west(cur)] {
            if !grid.walkable(nb) {
                continue;
            }
            let tentative_g = cur_g + 1;
            if tentative_g < *best_g.get(&nb).unwrap_or(&i32::MAX) {
                came_from.insert(nb, cur);
                best_g.insert(nb, tentative_g);
                open.push(Node {
                    tile: nb,
                    f: tentative_g + chebyshev(nb, to),
                });
            }
        }
    }

    Err(NoPath)
}

fn north(t: Tile) -> Tile {
    Tile { x: t.x, z: t.z + 1, level: t.level }
}
fn east(t: Tile) -> Tile {
    Tile { x: t.x + 1, z: t.z, level: t.level }
}
fn south(t: Tile) -> Tile {
    Tile { x: t.x, z: t.z - 1, level: t.level }
}
fn west(t: Tile) -> Tile {
    Tile { x: t.x - 1, z: t.z, level: t.level }
}

/// Heap entry; `Ord` is reversed so the smallest f pops first, with tile
/// coordinates as tie-breakers to keep the ordering total.
struct Node {
    tile: Tile,
    f: i32,
}

impl PartialEq for Node {
    fn eq(&self, other: &Self) -> bool {
        self.f == other.f && self.tile == other.tile
    }
}
impl Eq for Node {}

impl PartialOrd for Node {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Node {
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
    use crate::grid::StepGrid;
    use crate::router::{find, Leg};
    use crate::tile::Tile;

    #[test]
    // `Walk` is the only variant until Task 3 adds Door; keep the brief's
    // assertion verbatim so the else arm is still checked when Door lands.
    #[allow(irrefutable_let_patterns)]
    fn find_across_open_3x3_is_a_walk_leg() {
        let g = StepGrid::fixture_open_3x3();
        let r = find(&g, Tile { x: 0, z: 0, level: 0 }, Tile { x: 2, z: 2, level: 0 }).unwrap();
        assert_eq!(r.dest, Tile { x: 2, z: 2, level: 0 });
        let Leg::Walk { tiles } = &r.legs[0] else { panic!() };
        assert_eq!(tiles.first().unwrap().x, 0);
        assert_eq!(tiles.last(), Some(&r.dest));
    }

    #[test]
    fn find_through_wall_is_no_path() {
        let mut g = StepGrid::fixture_open_3x3();
        g.set_walkable(Tile { x: 1, z: 0, level: 0 }, false);
        g.set_walkable(Tile { x: 1, z: 1, level: 0 }, false);
        g.set_walkable(Tile { x: 1, z: 2, level: 0 }, false);
        assert!(find(&g, Tile { x: 0, z: 1, level: 0 }, Tile { x: 2, z: 1, level: 0 }).is_err());
    }
}
