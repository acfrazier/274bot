use crate::arrival::arrived;
use crate::tile::Tile;

#[test]
fn arrived_on_tile_or_adjacent_if_solid() {
    let a = Tile {
        x: 10,
        z: 10,
        level: 0,
    };
    assert!(arrived(a, a, true));
    assert!(!arrived(
        a,
        Tile {
            x: 12,
            z: 10,
            level: 0
        },
        true
    ));
    assert!(arrived(
        a,
        Tile {
            x: 10,
            z: 11,
            level: 0
        },
        false
    ));
    assert!(!arrived(
        a,
        Tile {
            x: 10,
            z: 11,
            level: 0
        },
        true
    ));
}
