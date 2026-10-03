use api::snapshot::WorldTile;
use nav::router::{find_first_with, find_with, FindOptions, Leg, RouteError};
use nav::transport::TransportKind;
use nav::world::NavWorld;
use nav::WorldState;

#[test]
#[ignore = "requires NAV_SCRIPTED_LADDERS_PACK pointing to a real 289 pack; absence fails"]
fn mainland_web_path_reaches_mage_arena_bank_cellar() {
    let path = std::env::var_os("NAV_SCRIPTED_LADDERS_PACK")
        .expect("NAV_SCRIPTED_LADDERS_PACK must name the real 289 pack");
    let world = NavWorld::load_pack(std::path::Path::new(&path)).expect("decode real 289 pack");
    let origin = WorldTile {
        x: 3097,
        z: 3957,
        level: 0,
    };
    let bank = WorldTile {
        x: 2534,
        z: 4713,
        level: 0,
    };
    let state = WorldState {
        combat_level: Some(126),
        inv: std::collections::HashMap::from([(946, 1)]),
        ..WorldState::empty().with_map_members(true)
    };
    let route = find_with(
        &world.collision,
        &world.graph,
        origin,
        bank,
        FindOptions {
            allow_wilderness: true,
            allow_teleports: false,
            allow_bank_fetch: false,
            ..FindOptions::default()
        },
        &state,
    )
    .expect("mainland side of Mage Arena webs must reach Gundai's cellar");
    let hops: Vec<_> = route
        .legs
        .iter()
        .filter_map(|leg| match leg {
            Leg::Transport { edge } => Some(edge.as_ref()),
            _ => None,
        })
        .collect();
    assert!(
        hops.iter().any(|edge| edge.loc_id == 733),
        "route must cross a Mage Arena web: {route:?}"
    );
    assert!(
        hops.iter()
            .any(|edge| edge.kind == TransportKind::Ladder && edge.loc_id == 2871),
        "route must climb the content-derived cellar ladder: {route:?}"
    );
    assert_eq!(route.dest, bank);
    println!("edges={} route={route:?}", world.graph.edges.len());
}

#[test]
#[ignore = "requires NAV_SCRIPTED_LADDERS_PACK pointing to a real 289 pack; absence fails"]
fn refused_rellekka_search_does_not_flood_from_a_nominal_stair_landing() {
    use std::collections::{HashMap, HashSet};

    let path = std::env::var_os("NAV_SCRIPTED_LADDERS_PACK")
        .expect("NAV_SCRIPTED_LADDERS_PACK must name the real 289 pack");
    let world = NavWorld::load_pack(std::path::Path::new(&path)).expect("decode real 289 pack");
    let state = WorldState {
        map_members: true,
        combat_level: Some(126),
        quests: HashSet::from([
            "Prince Ali Rescue".into(),
            "Rune Mysteries".into(),
            "Lost City".into(),
            "Shilo Village".into(),
            "Tree Gnome Village".into(),
            "The Grand Tree".into(),
        ]),
        inv: HashMap::from([
            (995, 10_000),
            (554, 1_000),
            (555, 1_000),
            (556, 1_000),
            (557, 1_000),
            (558, 1_000),
            (561, 1_000),
            (563, 1_000),
        ]),
        stats: (0..21).map(|skill| (skill, 99)).collect(),
        ..WorldState::empty()
    };
    let search = find_first_with(
        &world.collision,
        &world.graph,
        WorldTile {
            x: 3222,
            z: 3218,
            level: 0,
        },
        &[WorldTile {
            x: 2664,
            z: 3664,
            level: 0,
        }],
        FindOptions::default(),
        &state,
    );
    assert_eq!(search.route().unwrap_err(), RouteError::NoPath);
    println!(
        "refused_rellekka settled={} proof={:?} scratch={:?}",
        search.settled(),
        search.proof(),
        search.scratch_capacities(),
    );
}
