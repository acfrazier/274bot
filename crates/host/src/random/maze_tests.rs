use super::*;

#[test]
fn every_maze_spawn_solves_to_the_chamber_door() {
    let g = graph();
    for spawn in MAZE_SPAWNS {
        let route = select_route(g, spawn).expect("spawn must not be trapped");
        assert!(!route.is_empty());
        assert_eq!(
            *route.last().expect("non-empty"),
            MAZE_SHRINE_DOOR,
            "every route ends at the shrine chamber door"
        );
    }
}

#[test]
fn observed_tile_solves_its_own_short_route_not_the_sw_corners() {
    let g = graph();
    let observed = select_route(g, (2905, 4566)).expect("observed tile solves");
    let sw = select_route(g, (2891, 4555)).expect("sw corner solves");
    assert_eq!(observed.len(), 7);
    assert_ne!(
        observed, sw,
        "a different spawn must not be handed the SW route"
    );
    assert_eq!(
        observed,
        vec![
            (2911, 4566),
            (2906, 4586),
            (2916, 4586),
            (2912, 4584),
            (2918, 4576),
            (2912, 4572),
            (2910, 4576),
        ]
    );
}

#[test]
fn disconnected_tile_returns_no_route() {
    let g = graph();
    // Outside the enclosing walls: the layout cannot reach the shrine.
    assert_eq!(select_route(g, (2880, 4544)), None);
    assert!(solve_route(g, (2880, 4544)).is_empty());
}

#[test]
fn door_passable_gates_the_approach_side() {
    // 3628 (dir 0) opens from either side.
    let d0 = DoorInfo {
        tile: (2900, 4550),
        id: 3628,
        angle: 2,
    };
    assert!(door_passable(&d0, (2900, 4550)));
    assert!(door_passable(&d0, (2901, 4550)));
    // 3629 (dir 1) opens only from the same-axis side.
    let d1 = DoorInfo {
        tile: (2900, 4550),
        id: 3629,
        angle: 0,
    };
    assert!(door_passable(&d1, (2900, 4550)));
    assert!(!door_passable(&d1, (2901, 4550)));
    // 3630 (dir 2) opens only from the opposite side.
    let d2 = DoorInfo {
        tile: (2900, 4550),
        id: 3630,
        angle: 0,
    };
    assert!(!door_passable(&d2, (2900, 4550)));
    assert!(door_passable(&d2, (2901, 4550)));
}
