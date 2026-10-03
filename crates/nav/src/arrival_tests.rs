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

#[test]
fn area_arrival_checks_radius_plane_and_current_standability_only() {
    use crate::arrival::arrived_in_area;
    use api::snapshot::WorldTile;

    let centre = WorldTile {
        x: 2848,
        z: 3426,
        level: 0,
    };
    let shore = WorldTile {
        x: 2840,
        z: 3436,
        level: 0,
    };
    assert!(arrived_in_area(shore, centre, 40, |tile| tile == shore));
    assert!(!arrived_in_area(shore, centre, 40, |_| false));
    assert!(!arrived_in_area(shore, centre, 9, |_| unreachable!()));
    assert!(!arrived_in_area(shore, centre, -1, |_| unreachable!()));
    assert!(!arrived_in_area(
        WorldTile { level: 1, ..shore },
        centre,
        40,
        |_| unreachable!(),
    ));
    assert!(arrived_in_area(shore, shore, 0, |_| true));
    assert!(!arrived_in_area(shore, shore, 0, |_| false));
}
