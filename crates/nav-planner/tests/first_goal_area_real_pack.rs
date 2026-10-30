//! First-goal Area routing from Draynor to Almera on the real v17 pack.

use std::collections::{HashMap, HashSet};

use api::snapshot::WorldTile;
use nav_planner::router::{find_first_with, FindOptions, Leg, RouteError};
use nav_planner::world::NavWorld;
use nav_planner::WorldState;

fn tile(x: i32, z: i32) -> WorldTile {
    WorldTile { x, z, level: 0 }
}

fn waterfall_state() -> WorldState {
    WorldState {
        inv: HashMap::from([(954, 1), (995, 60)]),
        worn: HashSet::from([1163, 1333, 1113, 1201, 1079]),
        inventory_ready: true,
        equipment_ready: true,
        stats: (0..=20).map(|id| (id, 1)).collect(),
        combat_level: Some(3),
        map_members: true,
        ..WorldState::empty()
    }
}

fn almera_area_stands(world: &NavWorld) -> Vec<WorldTile> {
    let centre = tile(2522, 3498);
    let mut stands = Vec::new();
    for x in centre.x - 8..=centre.x + 8 {
        for z in centre.z - 8..=centre.z + 8 {
            let stand = tile(x, z);
            if world.collision.standable(stand) {
                stands.push(stand);
            }
        }
    }
    stands
}

#[test]
fn draynor_to_almera_area_goal_routes_with_the_observed_waterfall_state() {
    let Some(world) = NavWorld::load_default_pack_or_skip() else {
        return;
    };
    let stands = almera_area_stands(&world);
    let exact = tile(2530, 3490);
    assert_eq!(stands.len(), 189, "the real pack's radius-8 area stand set");
    assert!(stands.contains(&exact), "the exact target is standable");

    let state = waterfall_state();
    for allow_teleports in [false, true] {
        let search = find_first_with(
            &world.collision,
            &world.graph,
            tile(3092, 3243),
            &stands,
            FindOptions {
                allow_wilderness: true,
                allow_teleports,
                ..FindOptions::default()
            },
            &state,
        );
        let route = search.route().unwrap_or_else(|error| {
            panic!("Almera Area route with allow_teleports={allow_teleports}: {error:?}")
        });
        assert_eq!(route.dest, exact);
        assert!(
            route.ticks <= 538.5,
            "allow_teleports={allow_teleports}: {} ticks",
            route.ticks
        );
        assert!(route.legs.iter().all(|leg| match leg {
            Leg::Walk { .. } => true,
            Leg::Transport { edge } => {
                !edge.item_req.iter().any(|(id, _)| *id == 1712)
                    && !edge.consumed_req.iter().any(|(id, _)| *id == 1712)
            }
        }), "the route must not use a charged glory");
    }

    let wilderness_off = find_first_with(
        &world.collision,
        &world.graph,
        tile(3092, 3243),
        &stands,
        FindOptions {
            allow_wilderness: false,
            allow_teleports: false,
            ..FindOptions::default()
        },
        &state,
    );
    assert_eq!(wilderness_off.route().err(), Some(RouteError::NoPath));
}
