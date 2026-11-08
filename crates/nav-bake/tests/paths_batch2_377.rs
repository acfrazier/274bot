//! Source-grounded batch-2 connectivity and unchanged production-route evidence.
use std::path::PathBuf;

use api::query::{is_arrived, CollisionArrivalProbe};
use api::snapshot::WorldTile;
use nav_planner::router::{find_first_with, find_with, FindOptions, Leg};
use nav_planner::world::NavWorld;
use nav_planner::world_state::WorldState;
use serde_json::json;

fn path(name: &str) -> PathBuf {
    std::env::var_os(name)
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("set {name}"))
}

fn tile(x: i32, z: i32, level: i32) -> WorldTile {
    WorldTile { x, z, level }
}

#[test]
#[ignore = "requires an explicitly baked revision-377 NAV_377_PACK"]
fn server_only_wall_3550_blocks_the_real_map_boundary() {
    let world = NavWorld::load_pack(&path("NAV_377_PACK")).unwrap();
    // Canon m34_49.jm2:4968 places 3550 at (25,33), wall shape 0,
    // angle 0. Opcode 74 must not remove its engine-side west/east wall.
    // The pack retains wall faces; load_pack does not attach the raw flags sidecar.
    assert_ne!(
        world.collision.walkable_word(2201, 3169, 0) & client::dash3d::CollisionFlag::W_W as u32,
        0
    );
    assert_ne!(
        world.collision.walkable_word(2200, 3169, 0) & client::dash3d::CollisionFlag::W_E as u32,
        0
    );
}

#[test]
#[ignore = "requires NAV_377_PACK and PATHS_377_POLICY_REPORT"]
fn palace_sewer_default_danger_policy_observes_zombie_level_boundary() {
    let world = NavWorld::load_pack(&path("NAV_377_PACK")).unwrap();
    let zones = world.graph.zones.as_ref().unwrap();
    let zombies: Vec<_> = zones
        .zones()
        .iter()
        .filter_map(|zone| {
            let kind = &zones.kinds()[usize::from(zone.kind)];
            if kind.npc_id != 75
                || !(3220..=3235).contains(&zone.spawn_x)
                || !(9890..=9915).contains(&zone.spawn_z)
            {
                return None;
            }
            Some(json!({
                "npc_id": kind.npc_id, "vislevel": kind.vislevel,
                "cap": zone.cap, "class": format!("{:?}", zone.class),
                "spawn": [zone.spawn_x, zone.spawn_z]
            }))
        })
        .collect();
    let mut rows = Vec::new();
    let mut matched = !zombies.is_empty();
    // Canon Npc.ts:1017 suppresses this hunt only above twice vislevel:
    // all.npc:2289 gives 24, so 46/48 remain aggressive and 49 is exempt.
    for (combat_level, expected) in [(46, false), (48, false), (49, true)] {
        let state = WorldState {
            combat_level: Some(combat_level),
            map_members: true,
            ..WorldState::empty()
        };
        let route = find_with(
            &world.collision,
            &world.graph,
            tile(3220, 3220, 0),
            tile(3232, 3500, 0),
            FindOptions::default(),
            &state,
        );
        matched &= route.is_ok() == expected;
        rows.push(json!({
            "combat_level": combat_level, "expected_route": expected,
            "route": route.as_ref().ok().map(|route| json!({"ticks": route.ticks, "legs": route.legs.len()})),
            "error": route.as_ref().err().map(|error| format!("{error:?}"))
        }));
    }
    std::fs::write(
        path("PATHS_377_POLICY_REPORT"),
        serde_json::to_vec_pretty(&json!({
            "verdict": if matched {"PASS"} else {"FAIL"},
            "zombies": zombies,
            "routes": rows,
        }))
        .unwrap(),
    )
    .unwrap();
    assert!(
        matched,
        "default danger-policy boundary; see PATHS_377_POLICY_REPORT"
    );
}

#[test]
#[ignore = "requires NAV_377_PACK and PATHS_377_MORGAN_REPORT"]
fn morgan_cupboard_route_returns_to_ground_floor() {
    let world = NavWorld::load_pack(&path("NAV_377_PACK")).unwrap();
    let state = WorldState {
        combat_level: Some(49),
        map_members: true,
        ..WorldState::empty()
    };
    let mut rows = Vec::new();
    let mut failed = false;
    let to = tile(3232, 3500, 0);
    // Match take-hammer-coins: the native ground interaction uses radius-six
    // Reach, not an exact stand on the palace alcove's coin spawn.
    let probe = CollisionArrivalProbe::new(|tile: WorldTile| {
        Some(world.collision.walkable_word(tile.x, tile.z, tile.level) as i32)
    });
    let goals: Vec<_> = (-6..=6)
        .flat_map(|dx| (-6..=6).map(move |dz| tile(to.x + dx, to.z + dz, to.level)))
        .filter(|stand| world.collision.standable(*stand) && is_arrived(*stand, to, 6, || &probe))
        .collect();
    assert!(!goals.is_empty(), "the real coins have a Reach approach");
    for from in [tile(3102, 3266, 1), tile(3097, 3270, 1)] {
        let result = find_first_with(
            &world.collision,
            &world.graph,
            from,
            &goals,
            FindOptions::default(),
            &state,
        );
        rows.push(match result.route() {
            Ok(route) => {
                assert!(route
                    .legs
                    .iter()
                    .any(|leg| { matches!(leg, Leg::Transport { edge } if edge.loc_id == 1723) }));
                json!({"from":from,"to":to,"ticks":route.ticks,"legs":format!("{:?}",route.legs)})
            }
            Err(error) => {
                failed = true;
                json!({"from":from,"to":to,"error":format!("{error:?}")})
            }
        });
    }
    let edges: Vec<_> = world
        .graph
        .edges
        .iter()
        .enumerate()
        .filter(|(_, edge)| (3090..=3110).contains(&edge.at.x) && (3260..=3280).contains(&edge.at.z))
        .map(|(index, edge)| json!({"loc":edge.loc_id,"at":edge.at,"to":edge.to,"ticks":edge.ticks,"approach":format!("{:?}",world.graph.approaches.get(index))}))
        .collect();
    std::fs::write(
        path("PATHS_377_MORGAN_REPORT"),
        serde_json::to_vec_pretty(&json!({
            "verdict": if failed {"FAIL"} else {"PASS"},
            "routes": rows,
            "edges": edges,
            "reach_goals": goals,
        }))
        .unwrap(),
    )
    .unwrap();
    assert!(
        !failed,
        "Morgan upstairs return; see PATHS_377_MORGAN_REPORT"
    );
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
        (
            "palace-sewer-ladder-approach",
            tile(3220, 3220, 0),
            tile(3230, 9903, 0),
        ),
        (
            "varrock-manhole-west-approach",
            tile(3220, 3220, 0),
            tile(3236, 3458, 0),
        ),
        (
            "varrock-sewer-to-palace-ladder",
            tile(3237, 9858, 0),
            tile(3230, 9903, 0),
        ),
        ("blurite-entrance", tile(3220, 3220, 0), tile(3007, 9549, 0)),
        ("blurite-exit", tile(3007, 9549, 0), tile(3007, 3150, 0)),
    ] {
        rows.push(match find_with(&world.collision, &world.graph, from, to, options, &state) {
            Ok(route) => {
                if matches!(name, "palace-west-coin" | "palace-east-coin") {
                    for loc_id in [881, 1755] {
                        assert!(route.legs.iter().any(|leg| {
                            matches!(leg, Leg::Transport { edge } if edge.loc_id == loc_id)
                        }), "{name} must use the source-proved sewer entrance and palace ladder");
                    }
                }
                json!({"name":name,"from":from,"to":to,"reachable":true,"ticks":route.ticks,"legs":format!("{:?}",route.legs)})
            }
            Err(error) => {
                failed = true;
                json!({"name":name,"from":from,"to":to,"reachable":false,"error":format!("{error:?}")})
            }
        });
    }
    std::fs::write(path("PATHS_377_ROUTE_REPORT"), serde_json::to_vec_pretty(&json!({
        "verdict":if failed {"FAIL"} else {"PASS"}, "routes":rows,
        "palace_edges": world.graph.edges.iter()
            .filter(|edge| matches!(edge.loc_id, 881 | 882 | 1755) && (3200..=3260).contains(&edge.at.x) && (3450..=3510).contains(&(edge.at.z % 6400)))
            .map(|edge| json!({"loc_id":edge.loc_id,"at":edge.at,"to":edge.to,"delta":edge.player_delta,"open":edge.open_loc_id,"ticks":edge.ticks}))
            .collect::<Vec<_>>(),
    })).unwrap()).unwrap();
    assert!(
        !failed,
        "source-grounded palace/blurite routes missing; see PATHS_377_ROUTE_REPORT"
    );
    let manhole = world
        .graph
        .edges
        .iter()
        .find(|edge| edge.loc_id == 881 && edge.at == tile(3237, 3458, 0))
        .expect("Varrock surface manhole open-and-climb pair");
    assert_eq!(manhole.to, tile(3237, 9858, 0));
    assert_eq!(manhole.open_loc_id, Some(882));
    assert_eq!(manhole.ticks, 3);
    let entry = world
        .graph
        .edges
        .iter()
        .find(|edge| edge.loc_id == 9472)
        .expect("Asgarnian ice dungeon direct trapdoor");
    assert_eq!(entry.at, tile(3008, 3150, 0));
    assert_eq!(entry.to, tile(3007, 9549, 0));
    assert_eq!(entry.open_loc_id, None);
    assert_eq!(entry.ticks, 3);
    let route = find_with(
        &world.collision,
        &world.graph,
        tile(3007, 3150, 0),
        entry.to,
        options,
        &state,
    )
    .unwrap();
    assert!(route
        .legs
        .iter()
        .any(|leg| matches!(leg, Leg::Transport { edge } if edge.loc_id == 9472)));
}

#[test]
#[ignore = "requires NAV_289_PACK and PATHS_289_CORPUS_REPORT; compare bytes before/after"]
fn production_289_route_corpus() {
    let world = NavWorld::load_pack(&path("NAV_289_PACK")).unwrap();
    let state = WorldState {
        combat_level: Some(80),
        map_members: true,
        inv: [(995, 100)].into_iter().collect(),
        ..WorldState::empty()
    };
    let options = FindOptions {
        zones: nav_planner::zones::ZoneExempt::all(),
        ..Default::default()
    };
    let sites = [
        tile(3222, 3218, 0),
        tile(3092, 3243, 0),
        tile(3210, 3424, 0),
        tile(2964, 3378, 0),
        tile(3015, 3204, 0),
        tile(3228, 3504, 0),
    ];
    let mut rows = Vec::new();
    for from in sites {
        for to in sites {
            if from == to {
                continue;
            }
            rows.push(match find_with(&world.collision, &world.graph, from, to, options, &state) {
                Ok(route) => json!({"from":from,"to":to,"ticks":route.ticks,"legs":format!("{:?}",route.legs)}),
                Err(error) => json!({"from":from,"to":to,"error":format!("{error:?}")}),
            });
        }
    }
    std::fs::write(
        path("PATHS_289_CORPUS_REPORT"),
        serde_json::to_vec_pretty(&rows).unwrap(),
    )
    .unwrap();
}
