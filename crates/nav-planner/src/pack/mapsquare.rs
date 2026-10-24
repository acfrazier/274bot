use super::*;

/// All walkable tiles of `grid` on `level`, in row-major (z then x) order.
/// Tiles off `grid`'s own level yield an empty iterator.
pub fn walkable_dots(grid: &StepGrid, level: i32) -> impl Iterator<Item = Tile> + '_ {
    let (ox, oz) = (grid.origin.x, grid.origin.z);
    (0..grid.height)
        .flat_map(move |z| {
            (0..grid.width).map(move |x| Tile {
                x: ox + x as i32,
                z: oz + z as i32,
                level,
            })
        })
        .filter(move |t| grid.walkable(*t))
}
