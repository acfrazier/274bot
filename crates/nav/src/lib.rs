//! Nav: tile coordinates, Chebyshev distance, walkability grids, routing,
//! and arrival detection.

pub mod arrival;
pub mod collision;
pub mod grid;
pub mod pack;
pub mod router;
pub mod tile;
pub mod traveller;

#[cfg(test)]
mod tests {
    use crate::grid::StepGrid;
    use crate::tile::{chebyshev, Tile};

    #[test]
    fn fixture_center_is_walkable_and_origin_is_not_out_of_range() {
        let g = StepGrid::fixture_open_3x3();
        let c = Tile {
            x: 1,
            z: 1,
            level: 0,
        };
        assert!(g.walkable(c));
        assert!(!g.walkable(Tile {
            x: -1,
            z: 0,
            level: 0
        }));
        assert_eq!(
            chebyshev(
                c,
                Tile {
                    x: 3,
                    z: 1,
                    level: 0
                }
            ),
            2
        );
    }

    #[test]
    fn contains_is_bounds_and_level_only() {
        let g = StepGrid::fixture_open_3x3();
        assert!(g.contains(Tile {
            x: 0,
            z: 0,
            level: 0
        }));
        assert!(g.contains(Tile {
            x: 2,
            z: 2,
            level: 0
        }));
        assert!(!g.contains(Tile {
            x: 3,
            z: 0,
            level: 0
        }));
        assert!(!g.contains(Tile {
            x: 0,
            z: 0,
            level: 1
        }));
    }
}
