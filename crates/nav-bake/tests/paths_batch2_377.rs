//! Source-grounded batch-2 connectivity and unchanged production-route evidence.
use std::path::PathBuf;

use api::snapshot::WorldTile;
use nav_planner::router::{find_with, FindOptions, Leg};
use nav_planner::world::NavWorld;
use nav_planner::world_state::WorldState;
use serde_json::json;

fn path(name: &str) -> PathBuf {
    std::env::var_os(name).map(PathBuf::from).unwrap_or_else(|| panic!("set {name}"))
}

fn tile(x: i32, z: i32, level: i32) -> WorldTile {
    WorldTile { x, z, level }
}

#[test]
#[ignore = "requires NAV_377_PACK and PATHS_377_ROUTE_REPORT"]
fn palace_coin_and_blurite_routes_follow_real_content() {
    let world = NavWorld::load_pack(&path("NAV_377_PACK")).unwrap();
    let state = WorldState {
        combat_level: Some(80),
        map_members: true,
        ..WorldState::empty()
    };
    let options = FindOptions {
        zones: nav_planner::zones::ZoneExempt::all(),
        ..Default::default()
    };
    let mut rows = Vec::new();
    let mut failed = false;
    for (name, from, to) in [
        ("palace-west-coin", tile(3220, 3220, 0), tile(3228, 3504, 0)),
        ("palace-east-coin", tile(3220, 3220, 0), tile(3232, 3500, 0)),
        ("blurite-entrance", tile(3220, 3220, 0), tile(3007, 9549, 0)),
        ("blurite-exit", tile(3007, 9549, 0), tile(3007, 3150, 0)),
    ] {
        rows.push(match find_with(&world.collision, &world.graph, from, to, options, &state) {
            Ok(route) => json!({"name":name,"from":from,"to":to,"reachable":true,"ticks":route.ticks,"legs":format!("{:?}",route.legs)}),
            Err(error) => {
                failed = true;
                json!({"name":name,"from":from,"to":to,"reachable":false,"error":format!("{error:?}")})
            }
        });
    }
    std::fs::write(path("PATHS_377_ROUTE_REPORT"), serde_json::to_vec_pretty(&json!({
        "verdict":if failed {"FAIL"} else {"PASS"}, "routes":rows,
    })).unwrap()).unwrap();
    assert!(!failed, "source-grounded palace/blurite routes missing; see PATHS_377_ROUTE_REPORT");
    let entry = world.graph.edges.iter().find(|edge| edge.loc_id == 9472).expect("Asgarnian ice dungeon direct trapdoor");
    assert_eq!(entry.at, tile(3008, 3150, 0));
    assert_eq!(entry.to, tile(3007, 9549, 0));
    assert_eq!(entry.open_loc_id, None);
    assert_eq!(entry.ticks, 3);
    let route = find_with(&world.collision, &world.graph, tile(3007, 3150, 0), entry.to, options, &state).unwrap();
    assert!(route.legs.iter().any(|leg| matches!(leg, Leg::Transport { edge } if edge.loc_id == 9472)));
}

#[test]
#[ignore = "requires NAV_289_PACK and PATHS_289_CORPUS_REPORT; compare bytes before/after"]
fn production_289_route_corpus() {
    let world = NavWorld::load_pack(&path("NAV_289_PACK")).unwrap();
    let state = WorldState {
        combat_level: Some(80), map_members: true,
        inv: [(995, 100)].into_iter().collect(), ..WorldState::empty()
    };
    let options = FindOptions { zones: nav_planner::zones::ZoneExempt::all(), ..Default::default() };
    let sites = [tile(3222,3218,0),tile(3092,3243,0),tile(3210,3424,0),tile(2964,3378,0),tile(3015,3204,0),tile(3228,3504,0)];
    let mut rows = Vec::new();
    for from in sites {
        for to in sites {
            if from == to { continue; }
            rows.push(match find_with(&world.collision, &world.graph, from, to, options, &state) {
                Ok(route) => json!({"from":from,"to":to,"ticks":route.ticks,"legs":format!("{:?}",route.legs)}),
                Err(error) => json!({"from":from,"to":to,"error":format!("{error:?}")}),
            });
        }
    }
    std::fs::write(path("PATHS_289_CORPUS_REPORT"), serde_json::to_vec_pretty(&rows).unwrap()).unwrap();
}
