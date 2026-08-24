//! A* routing on the step grid.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

use crate::grid::{DoorEdge, StepGrid};
use crate::tile::{chebyshev, Tile};

/// One leg of a route: a walk segment or a door crossing.
pub enum Leg {
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

/// A route from an origin to `dest`, split into legs.
pub struct Route {
    pub legs: Vec<Leg>,
    pub dest: Tile,
}

/// Returned by [`find`] when no walkable path connects the two tiles.
#[derive(Debug)]
pub struct NoPath;

/// A* over the 4-neighbour grid (N/E/S/W, cost 1, heuristic chebyshev),
/// extended by directed door edges: from a tile `d.from`, a door lets the
/// route jump to `d.to` at cost 2. Same level only. `from` is assumed to sit
/// on a walkable tile; every tile moved onto must be walkable. Legs split
/// around door crossings: a walk leg up to the door's `from`, the Door leg,
/// then a walk leg onward from its `to`. Each result leg is non-empty; the
/// first walk leg starts at `from` and the last ends at `to`.
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
    let mut came_from: HashMap<Tile, Back> = HashMap::new();

    best_g.insert(from, 0);
    open.push(Node {
        tile: from,
        f: chebyshev(from, to),
    });

    while let Some(Node { tile: cur, .. }) = open.pop() {
        if cur == to {
            let legs = reconstruct(cur, &came_from);
            return Ok(Route { legs, dest: to });
        }

        let cur_g = best_g[&cur];
        let mut relax = |nb: Tile, cost: i32, back: Back| {
            let tentative_g = cur_g + cost;
            if tentative_g < *best_g.get(&nb).unwrap_or(&i32::MAX) {
                came_from.insert(nb, back);
                best_g.insert(nb, tentative_g);
                open.push(Node {
                    tile: nb,
                    f: tentative_g + chebyshev(nb, to),
                });
            }
        };
        for nb in [north(cur), east(cur), south(cur), west(cur)] {
            if grid.walkable(nb) {
                relax(nb, 1, Back::Walk(cur));
            }
        }
        for d in &grid.doors {
            if d.from == cur && grid.walkable(d.to) {
                relax(d.to, 2, Back::Door(*d));
            }
        }
    }

    Err(NoPath)
}

/// Split the A* backtrack from `cur` back to `from` into legs at door
/// crossings. `from` is implicit: backtracking stops when the entry-less
/// start tile is reached, and that tile is already the last element of the
/// final walk segment.
fn reconstruct(cur: Tile, came_from: &HashMap<Tile, Back>) -> Vec<Leg> {
    // Walk tiles accumulated in reverse order (cur-side first).
    let mut walk_rev = vec![cur];
    let mut t = cur;
    let mut legs_rev: Vec<Leg> = Vec::new();
    while let Some(prev) = came_from.get(&t) {
        match prev {
            Back::Walk(pt) => {
                walk_rev.push(*pt);
                t = *pt;
            }
            Back::Door(d) => {
                walk_rev.reverse();
                legs_rev.push(Leg::Walk { tiles: walk_rev });
                legs_rev.push(Leg::Door {
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
    legs_rev.push(Leg::Walk { tiles: walk_rev });
    legs_rev.reverse();
    legs_rev
}

/// How `came_from`'s key was reached: by a walk step from `Walk`'s tile or
/// by a door crossing recorded in `Door`.
#[derive(Clone, Copy)]
enum Back {
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
    fn find_across_open_3x3_is_a_walk_leg() {
        let g = StepGrid::fixture_open_3x3();
        let r = find(
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
        let Leg::Walk { tiles } = &r.legs[0] else {
            panic!()
        };
        assert_eq!(tiles.first().unwrap().x, 0);
        assert_eq!(tiles.last(), Some(&r.dest));
    }

    #[test]
    fn find_through_wall_is_no_path() {
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
        assert!(find(
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
    fn find_uses_door_edge_across_a_wall() {
        let g = StepGrid::fixture_door_corridor();
        let r = find(
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
            .any(|l| matches!(l, Leg::Door { loc_id: 1530, .. })));
    }

    #[test]
    fn door_route_splits_into_walk_door_walk_legs() {
        let g = StepGrid::fixture_door_corridor();
        let r = find(
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
            Leg::Walk { tiles: w0 },
            Leg::Door {
                loc,
                loc_id,
                from,
                to,
            },
            Leg::Walk { tiles: w1 },
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
}
