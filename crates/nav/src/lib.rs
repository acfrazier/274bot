//! Nav: tile coordinates, Chebyshev distance, and walkability grids.

pub mod grid;
pub mod router;
pub mod tile;

#[cfg(test)]
mod tests {
    use crate::grid::StepGrid;
    use crate::tile::{chebyshev, Tile};

    #[test]
    fn fixture_center_is_walkable_and_origin_is_not_out_of_range() {
        let g = StepGrid::fixture_open_3x3();
        let c = Tile { x: 1, z: 1, level: 0 };
        assert!(g.walkable(c));
        assert!(!g.walkable(Tile { x: -1, z: 0, level: 0 }));
        assert_eq!(chebyshev(c, Tile { x: 3, z: 1, level: 0 }), 2);
    }
}
