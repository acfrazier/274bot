//! Tile coordinates and distance.

/// A tile on the nav grid. `x`/`z` are the in-world plane; `level` is the
/// floor, so the same x/z can be a different tile on another level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Tile {
    pub x: i32,
    pub z: i32,
    pub level: i32,
}

/// Chebyshev (king-move) distance between two tiles: the larger of the
/// x-delta and z-delta. Level is not part of the distance.
pub fn chebyshev(a: Tile, b: Tile) -> i32 {
    (a.x - b.x).abs().max((a.z - b.z).abs())
}
