//! The original fortress NoPath repro against a freshly baked real 289 pack.
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use api::snapshot::WorldTile;
use nav::router::{find_first_with, find_with, FindOptions, Leg, RouteError};
use nav::transport::TransportKind;
use nav::world::NavWorld;
use nav::WorldState;

fn tile(x: i32, z: i32, level: i32) -> WorldTile {
    WorldTile { x, z, level }
}

#[test]
#[ignore = "requires NAV_QUEST_DOORS_PACK naming a freshly baked real 289 pack"]
fn fortress_routes_require_the_complete_worn_disguise() {
    let pack = PathBuf::from(std::env::var_os("NAV_QUEST_DOORS_PACK").expect("explicit pack"));
    let world = NavWorld::load_pack(&pack).expect("real 289 pack");
    let outside = tile(3016, 3512, 0);
    let inside = tile(3016, 3516, 0);
    let guard = tile(3016, 3514, 0);
    let disguise = WorldState {
        combat_level: Some(51),
        worn: HashSet::from([1139, 1101, 1333, 1201, 1079]),
        ..WorldState::empty()
    };
    let options = FindOptions::default();
    for worn in [HashSet::new(), HashSet::from([1139]), HashSet::from([1101])] {
        let missing = WorldState {
            worn,
            inv: HashMap::from([(1139, 1), (1101, 1)]),
            ..disguise.clone()
        };
        assert_eq!(
            find_with(
                &world.collision,
                &world.graph,
                outside,
                inside,
                options,
                &missing
            ),
            Err(RouteError::NoPath),
            "carrying a uniform or wearing only one piece cannot authorize ingress"
        );
    }
    let ingress = find_with(
        &world.collision,
        &world.graph,
        outside,
        inside,
        options,
        &disguise,
    )
    .expect("both disguise pieces currently worn authorize the real guard door");
    assert!(ingress.legs.iter().any(|leg| matches!(leg,
        Leg::Transport { edge } if edge.at == guard
            && edge.kind == TransportKind::Door
            && edge.worn_req.is_empty()
            && edge.worn_all_req == [1101, 1139]
    )));
    for at in [tile(3016, 3517, 0), tile(3030, 3510, 1)] {
        let edges: Vec<_> = world
            .graph
            .edges
            .iter()
            .filter(|edge| edge.at == at && edge.kind == TransportKind::Door)
            .collect();
        assert_eq!(
            edges.len(),
            2,
            "the real secret wall must prove both crossings: {at:?}"
        );
        assert!(edges
            .iter()
            .all(|edge| edge.worn_all_req.is_empty() && edge.worn_req.is_empty()));
    }
    let after_wall = tile(3016, 3519, 0);
    let wall_route = find_with(
        &world.collision,
        &world.graph,
        outside,
        after_wall,
        options,
        &disguise,
    )
    .expect("a legal stand beyond the secret wall is reachable with the disguise");
    assert!(wall_route.legs.iter().any(|leg| matches!(leg,
        Leg::Transport { edge } if edge.at == tile(3016, 3517, 0)
    )));
    let blocked_ladder = tile(3015, 3519, 0);
    assert!(!world.collision.standable(blocked_ladder));
    assert_eq!(
        find_with(
            &world.collision,
            &world.graph,
            outside,
            blocked_ladder,
            options,
            &disguise,
        ),
        Err(RouteError::NoPath),
        "never route onto the ladder's blocked anchor"
    );
    for (from, to) in [
        (tile(2971, 3311, 0), tile(3025, 3508, 0)),
        (outside, tile(3025, 3508, 0)),
        (inside, tile(3025, 3508, 0)),
        (outside, tile(3031, 3508, 1)),
    ] {
        // The real quest uses radius-one Reach arrival, not standing on the
        // wall-separated grate. The low-combat hole route also cannot waive
        // the Black Knights' danger zones.
        assert_eq!(
            find_with(&world.collision, &world.graph, from, to, options, &disguise,),
            Err(RouteError::NoPath),
            "preserve the original exact/unsafe refusal"
        );
        let targets: Vec<_> = (-1..=1)
            .flat_map(|dx| (-1..=1).map(move |dz| tile(to.x + dx, to.z + dz, to.level)))
            .filter(|stand| world.collision.standable(*stand))
            .collect();
        let mut permitted = disguise.clone();
        if to.level == 1 {
            permitted.combat_level = Some(126);
        }
        let result = find_first_with(
            &world.collision,
            &world.graph,
            from,
            &targets,
            options,
            &permitted,
        );
        let route = result.route().unwrap_or_else(|error| {
            panic!("qualified original fortress radius-one {from:?} -> {to:?}: {error:?}")
        });
        assert!(targets.contains(&route.dest));
        println!(
            "fortress {from:?} -> radius-one {to:?}, combat {:?}: {:?}, {} legs, {} ticks",
            permitted.combat_level,
            route.dest,
            route.legs.len(),
            route.ticks
        );
    }
}
