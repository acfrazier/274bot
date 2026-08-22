//! Walkability grids.

use crate::tile::Tile;

/// A rectangular grid of per-tile walkability flags (v1: one byte per tile,
/// `1` = walkable). `origin` is the tile at `walk[0]`; the grid spans `width`
/// tiles in +x then `height` rows in +z, all on `origin.level`.
pub struct StepGrid {
    walk: Vec<u8>,
    width: usize,
    height: usize,
    origin: Tile,
}

impl StepGrid {
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
    /// x and z in 0..3, all walkable.
    pub fn fixture_open_3x3() -> Self {
        Self {
            walk: vec![1; 9],
            width: 3,
            height: 3,
            origin: Tile { x: 0, z: 0, level: 0 },
        }
    }
}
