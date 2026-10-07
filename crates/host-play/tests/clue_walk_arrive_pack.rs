//! Optional real-pack companion to the clue machine/approach regressions.
//! Run with WORLD_NAV_PACK pointing at the selected revision's pack.
use api::WorldTile;
use nav::router::{find_with, FindOptions, Leg, RouteError};
use nav::world::NavWorld;
use nav::WorldState;

#[test]
#[ignore = "requires explicit WORLD_NAV_PACK for revision 289"]
fn entrana_drawers_east_stand_routes_via_real_boat_and_house_door() {
    let pack = std::env::var_os("WORLD_NAV_PACK").expect("explicit pack path");
    let world = NavWorld::load_pack(std::path::Path::new(&pack)).expect("load selected pack");
    let from = WorldTile {
        x: 3092,
        z: 3242,
        level: 0,
    };
    let drawers = WorldTile {
        x: 2818,
        z: 3351,
        level: 0,
    };
    // drawers2 is placed at angle 3; native operable-stand regressions prove
    // this east-side stand from its forceapproach and scene wall geometry.
    let stand = WorldTile {
        x: 2819,
        z: 3351,
        level: 0,
    };
    let mut state = WorldState::empty().with_map_members(true);
    state.inv.insert(3579, 1);
    state.inv.insert(952, 1);
    state.combat_level = Some(3);
    assert!(!world.collision.walkable(drawers));
    assert!(matches!(
        find_with(
            &world.collision,
            &world.graph,
            from,
            drawers,
            FindOptions::default(),
            &state
        ),
        Err(RouteError::NoPath)
    ));
    let route = find_with(
        &world.collision,
        &world.graph,
        from,
        stand,
        FindOptions::default(),
        &state,
    )
    .expect("east operable stand must route");
    assert_eq!(route.dest, stand);
    assert_eq!(
        (stand.x - drawers.x).abs().max((stand.z - drawers.z).abs()),
        1
    );
    let transports: Vec<_> = route
        .legs
        .iter()
        .filter_map(|leg| match leg {
            Leg::Transport { edge } => Some(edge.loc_id),
            Leg::Walk { .. } => None,
        })
        .collect();
    let boat = transports
        .iter()
        .position(|id| *id == 657)
        .expect("real Port Sarim monk boat");
    let plank = transports
        .iter()
        .position(|id| *id == 2415)
        .expect("Entrana disembarkation");
    let door = transports
        .iter()
        .position(|id| *id == 1533)
        .expect("enter the drawers house, not stop outside its wall");
    assert!(
        boat < plank && plank < door,
        "transport order: {transports:?}"
    );
    println!(
        "Entrana route: dest={:?}, ticks={}, transports={transports:?}",
        route.dest, route.ticks
    );
}
