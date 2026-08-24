//! Walkability grids.

use crate::tile::Tile;

/// One door on the grid. `from` and `to` are the walkable tiles on either
/// side; `loc` is the door's own tile (typically not walkable) and `loc_id`
/// its client object id. Traversal is directed `from` -> `to`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DoorEdge {
    pub loc: Tile,
    pub loc_id: i32,
    pub from: Tile,
    pub to: Tile,
}

/// A rectangular grid of per-tile walkability flags (v1: one byte per tile,
/// `1` = walkable). `origin` is the tile at `walk[0]`; the grid spans `width`
/// tiles in +x then `height` rows in +z, all on `origin.level`.
pub struct StepGrid {
    pub(crate) walk: Vec<u8>,
    pub(crate) width: usize,
    pub(crate) height: usize,
    pub(crate) origin: Tile,
    /// Door edges routing may traverse.
    pub doors: Vec<DoorEdge>,
}

impl StepGrid {
    /// Build a grid from raw parts. `walk` is row-major `z` then `x` (same
    /// indexing as [`StepGrid::walkable`]), one byte per tile, `1` = walkable.
    pub(crate) fn from_parts(
        origin: Tile,
        width: usize,
        height: usize,
        walk: Vec<u8>,
        doors: Vec<DoorEdge>,
    ) -> Self {
        Self {
            walk,
            width,
            height,
            origin,
            doors,
        }
    }

    /// True if `t` is on this grid's level, inside its bounds, and marked
    /// walkable. Anything outside the grid is not walkable.
    pub fn walkable(&self, t: Tile) -> bool {
        if t.level != self.origin.level {
            return false;
        }
        let x = t.x - self.origin.x;
        let z = t.z - self.origin.z;
        if x < 0 || z < 0 {
            return false;
        }
        let (x, z) = (x as usize, z as usize);
        if x >= self.width || z >= self.height {
            return false;
        }
        self.walk[z * self.width + x] == 1
    }

    /// Mark tile `t` walkable or blocked. Tiles outside the grid or on
    /// another level are ignored.
    pub fn set_walkable(&mut self, t: Tile, walkable: bool) {
        if t.level != self.origin.level {
            return;
        }
        let x = t.x - self.origin.x;
        let z = t.z - self.origin.z;
        if x < 0 || z < 0 {
            return;
        }
        let (x, z) = (x as usize, z as usize);
        if x >= self.width || z >= self.height {
            return;
        }
        self.walk[z * self.width + x] = if walkable { 1 } else { 0 };
    }

    /// A 3×3 grid on level 0 with its corner at `(0, 0)`: tiles with
    /// x and z in 0..3, all walkable. No doors.
    pub fn fixture_open_3x3() -> Self {
        Self {
            walk: vec![1; 9],
            width: 3,
            height: 3,
            origin: Tile {
                x: 0,
                z: 0,
                level: 0,
            },
            doors: vec![],
        }
    }

    /// A 1×40 corridor on level 0: x in 0..40 at z=0, all walkable. No
    /// doors. Long enough for a walk leg whose far end is more than the
    /// traveller's 20-tile near-end threshold.
    pub fn fixture_open_1x40() -> Self {
        Self {
            walk: vec![1; 40],
            width: 40,
            height: 1,
            origin: Tile {
                x: 0,
                z: 0,
                level: 0,
            },
            doors: vec![],
        }
    }

    /// A 1×5 corridor on level 0: x in 0..5 at z=0, walkable except a wall
    /// at x=2. A door edge crosses the wall: (1,0) -> (3,0) via loc (2,0),
    /// loc_id 1530.
    pub fn fixture_door_corridor() -> Self {
        let mut walk = vec![1; 5];
        walk[2] = 0;
        Self {
            walk,
            width: 5,
            height: 1,
            origin: Tile {
                x: 0,
                z: 0,
                level: 0,
            },
            doors: vec![DoorEdge {
                loc: Tile {
                    x: 2,
                    z: 0,
                    level: 0,
                },
                loc_id: 1530,
                from: Tile {
                    x: 1,
                    z: 0,
                    level: 0,
                },
                to: Tile {
                    x: 3,
                    z: 0,
                    level: 0,
                },
            }],
        }
    }
}
