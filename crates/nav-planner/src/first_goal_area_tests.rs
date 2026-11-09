//! First-goal Area routing from Draynor to Almera on the real v17 pack.

use std::collections::{HashMap, HashSet};

use crate::router::{find_first_with, find_with, missing_item_reqs, FindOptions};
use crate::world::NavWorld;
use crate::WorldState;
use api::snapshot::WorldTile;

fn tile(x: i32, z: i32) -> WorldTile {
    WorldTile { x, z, level: 0 }
}

fn waterfall_state() -> WorldState {
    WorldState {
        inv: HashMap::from([(954, 1), (995, 60)]),
        worn: HashSet::from([1163, 1333, 1113, 1201, 1079]),
        inventory_ready: true,
        equipment_ready: true,
        stats: (0..21)
            .map(|id| (id, if id < 7 && id != 5 { 40 } else { 1 }))
            .collect(),
        combat_level: Some(46),
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
        let options = FindOptions {
            allow_wilderness: true,
            allow_teleports,
            ..FindOptions::default()
        };
        let exact_route = find_with(
            &world.collision,
            &world.graph,
            tile(3092, 3243),
            exact,
            options,
            &state,
        )
        .expect("the exact target routes within NODE_BUDGET");
        assert!(exact_route.ticks <= 538.5);
        let search = find_first_with(
            &world.collision,
            &world.graph,
            tile(3092, 3243),
            &stands,
            options,
            &state,
        );
        let route = search.route().unwrap_or_else(|error| {
            panic!("Almera Area route with allow_teleports={allow_teleports}: {error:?}")
        });
        assert!(stands.contains(&route.dest));
        assert!(
            route.ticks <= exact_route.ticks,
            "allow_teleports={allow_teleports}: {} ticks",
            route.ticks
        );
        assert!(
            missing_item_reqs(route, &state).is_empty(),
            "the route needs no item absent from the observed inventory and equipment"
        );
    }
}
