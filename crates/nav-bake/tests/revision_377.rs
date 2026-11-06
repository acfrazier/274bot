//! Explicit real-content qualification. Ordinary suites retain their revision-289 pack.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use api::snapshot::WorldTile;
use nav_planner::manifest::{hash_file, nav_manifest_path, CacheManifest, NavManifest};
use nav_planner::router::{find_with, FindOptions, Leg};
use nav_planner::transport::{DoorDir, TransportKind};
use nav_planner::world::NavWorld;
use nav_planner::world_state::WorldState;
use serde_json::{json, Value};

fn required_path(name: &str) -> PathBuf {
    std::env::var_os(name)
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("set {name}"))
}

fn tile(x: i32, z: i32, level: i32) -> WorldTile {
    WorldTile { x, z, level }
}

fn pack_counts(path: &Path, world: &NavWorld) -> Value {
    let mut kinds = BTreeMap::new();
    for edge in &world.graph.edges {
        *kinds.entry(format!("{:?}", edge.kind)).or_insert(0_usize) += 1;
    }
    json!({
        "sha256": hash_file(path).expect("hash qualification navigation pack"),
        "bytes": std::fs::metadata(path).expect("stat qualification navigation pack").len(),
        "width": world.collision.width,
        "height": world.collision.height,
        "edges": world.graph.edges.len(),
        "teleports": world.graph.teleports.len(),
        "edge_kinds": kinds,
        "banks": world.banks().len(),
        "zones": world.graph.zones.as_ref().map(|table| table.zones().len()),
    })
}

#[test]
#[ignore = "requires NAV_377_PACK, NAV_377_CACHE, NAV_377_CONTENT, NAV_377_SNAPSHOT_ROOT, NAV_289_PACK and NAV_377_REPORT"]
fn real_revision_377_pack_identity_routes_and_transports() {
    let path = required_path("NAV_377_PACK");
    let cache_dir = required_path("NAV_377_CACHE");
    let content = required_path("NAV_377_CONTENT");
    let cache = CacheManifest::capture(377, &cache_dir).unwrap();
    let manifest: NavManifest =
        serde_json::from_slice(&std::fs::read(nav_manifest_path(&path)).unwrap()).unwrap();
    let content_id =
        nav_bake::bake::decoded_identity(377, &cache_dir, &required_path("NAV_377_SNAPSHOT_ROOT"))
            .unwrap();
    manifest
        .verify_pack(377, &cache, &hash_file(&path).unwrap(), Some(&content_id))
        .unwrap();
    assert_eq!(manifest.cache_id, cache.identity());
    assert_eq!(
        manifest.source_sha256.as_deref(),
        Some(
            nav_bake::bundle::source_digest(&content, &[&cache_dir.join("config")])
                .unwrap()
                .as_str()
        )
    );
    for (extension, expected) in [
        ("navflags", manifest.flags_sha256.as_ref()),
        ("navreach", manifest.reach_sha256.as_ref()),
        ("navcanlight", manifest.canlight_sha256.as_ref()),
        ("navpois", manifest.pois_sha256.as_ref()),
    ] {
        assert_eq!(
            Some(hash_file(&path.with_extension(extension)).unwrap()).as_ref(),
            expected,
            "{extension} binding"
        );
    }
    let world = NavWorld::load_pack(&path).unwrap();
    let zones = world.graph.zones.as_ref().unwrap();
    assert_eq!(zones.zones().len(), manifest.zone_count as usize);
    assert_eq!(
        zones
            .zones()
            .iter()
            .filter(|zone| zones.kinds()[usize::from(zone.kind)].npc_id >= 0)
            .count(),
        manifest.zone_npc_count as usize
    );
    let state = WorldState {
        map_members: true,
        inv: [(995, 30)].into_iter().collect(),
        ..WorldState::empty()
    };
    // This smoke isolates collision/transport connectivity, not account safety.
    let options = FindOptions {
        zones: nav_planner::zones::ZoneExempt::all(),
        ..Default::default()
    };
    let cities = [
        ("Lumbridge", tile(3222, 3218, 0)),
        ("Varrock", tile(3210, 3424, 0)),
        ("Falador", tile(2964, 3378, 0)),
    ];
    let mut routes = Vec::new();
    for (from_name, from) in cities {
        for (to_name, to) in cities {
            if from == to {
                continue;
            }
            let route = find_with(&world.collision, &world.graph, from, to, options, &state)
                .unwrap_or_else(|error| panic!("{from_name} to {to_name}: {error:?}"));
            assert_eq!(route.dest, to);
            routes.push(json!({"from": from_name, "to": to_name, "ticks": route.ticks, "legs": route.legs.len()}));
        }
    }
    let stairs = world
        .graph
        .edges
        .iter()
        .find(|edge| edge.loc_id == 1738 && edge.at == tile(3204, 3207, 0))
        .expect("377 Lumbridge staircase from stairs.rs2");
    assert_eq!(stairs.to, tile(3205, 3209, 1));
    assert_eq!(stairs.kind, TransportKind::Stairs);
    let climb = find_with(
        &world.collision,
        &world.graph,
        tile(3215, 3212, 0),
        stairs.to,
        options,
        &state,
    )
    .unwrap();
    assert!(climb
        .legs
        .iter()
        .any(|leg| matches!(leg, Leg::Transport { edge } if edge.loc_id == 1738)));
    let doors = world
        .graph
        .edges
        .iter()
        .filter(|edge| {
            edge.kind == TransportKind::Door
                && edge.at.level == 0
                && (3200..3264).contains(&edge.at.x)
                && (3200..3264).contains(&edge.at.z)
                && state.allows(edge)
        })
        .count();
    assert!(doors > 0, "real Lumbridge door crossings");
    // m50_50.jm2 places loc_1530 at 34,7 with angle 0; doors.loc
    // and the generic door_closed handler prove the 1530 -> 1531 swap.
    let mut door_routes = Vec::new();
    for (from, to) in [
        (tile(3233, 3207, 0), tile(3234, 3207, 0)),
        (tile(3234, 3207, 0), tile(3233, 3207, 0)),
    ] {
        let crossing =
            find_with(&world.collision, &world.graph, from, to, options, &state).unwrap();
        assert!(crossing.legs.iter().any(|leg| matches!(leg,
            Leg::Transport { edge } if edge.kind == TransportKind::Door
                && edge.at == tile(3234, 3207, 0)
                && edge.loc_id == 1530
                && edge.open_loc_id == Some(1531)
        )));
        door_routes
            .push(json!({"from": [from.x, from.z], "to": [to.x, to.z], "ticks": crossing.ticks}));
    }
    let ship = world
        .graph
        .edges
        .iter()
        .find(|edge| {
            edge.kind == TransportKind::Boat && edge.loc_id == 378 && edge.at == tile(3026, 3217, 0)
        })
        .expect("Port Sarim sailor");
    assert_eq!(ship.to, tile(2956, 3143, 1));
    assert_eq!(ship.consumed_req, [(995, 30)]);
    assert_eq!(ship.ticks, 7);
    let voyage = find_with(
        &world.collision,
        &world.graph,
        ship.at,
        tile(2956, 3147, 0),
        options,
        &state,
    )
    .unwrap();
    assert!(voyage.legs.iter().any(|leg| matches!(leg, Leg::Transport { edge } if edge.kind == TransportKind::Boat && edge.loc_id == 378)));
    assert!(voyage
        .legs
        .iter()
        .any(|leg| matches!(leg, Leg::Transport { edge } if edge.loc_id == 2082)));
    let reference_path = required_path("NAV_289_PACK");
    let reference = NavWorld::load_pack(&reference_path).unwrap();
    let report = json!({
        "verdict": "PASS",
        "revision": 377,
        "pack": pack_counts(&path, &world),
        "reference_289": pack_counts(&reference_path, &reference),
        "content_id": content_id,
        "manifest": manifest,
        "city_routes": routes,
        "lumbridge_doors": doors,
        "door_routes": door_routes,
        "stairs_ticks": climb.ticks,
        "ship_ticks": voyage.ticks,
        "limits": "Offline planner proof against Fairy Ring scripts. No live execution or historical accuracy claim. Other transport families are not qualified by this smoke."
    });
    std::fs::write(
        required_path("NAV_377_REPORT"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
}

#[test]
#[ignore = "requires an explicitly baked revision-377 NAV_377_PACK"]
fn real_revision_377_wheat_gates_connect_lumbridge() {
    let world = NavWorld::load_pack(&required_path("NAV_377_PACK")).unwrap();
    for (id, at, open_id) in [
        (1551, tile(3163, 3290, 0), 1552),
        (1553, tile(3162, 3290, 0), 1556),
    ] {
        let crossings: Vec<_> = world
            .graph
            .edges
            .iter()
            .filter(|edge| edge.kind == TransportKind::Door && edge.loc_id == id && edge.at == at)
            .collect();
        assert_eq!(crossings.len(), 2, "wheat-field gate {id} at {at:?}");
        for dir in [DoorDir::N, DoorDir::S] {
            let edge = crossings.iter().find(|edge| edge.dir == Some(dir)).unwrap();
            assert_eq!(edge.open_loc_id, Some(open_id), "{edge:?}");
            assert_eq!(edge.option, 1, "{edge:?}");
        }
    }
    let wheat = tile(3157, 3299, 0);
    let goals: Vec<_> = (-2..=2)
        .flat_map(|dx| (-2..=2).map(move |dz| tile(wheat.x + dx, wheat.z + dz, 0)))
        .filter(|at| world.collision.standable(*at))
        .collect();
    let state = WorldState {
        combat_level: Some(3),
        map_members: true,
        ..WorldState::empty()
    };
    for from in [tile(3222, 3218, 0), tile(3209, 3213, 0)] {
        let result = nav_planner::router::find_first_with(
            &world.collision,
            &world.graph,
            from,
            &goals,
            FindOptions::default(),
            &state,
        );
        let route = result
            .route()
            .unwrap_or_else(|error| panic!("Lumbridge {from:?} to wheat: {error:?}"));
        assert!(
            route.legs.iter().any(|leg| matches!(
                leg,
                Leg::Transport { edge }
                    if edge.kind == TransportKind::Door
                        && matches!(edge.loc_id, 1551 | 1553)
                        && edge.at.z == 3290
            )),
            "the field is entered through its authored gate: {route:?}"
        );
        eprintln!(
            "377 wheat route from={from:?} dest={:?} ticks={} legs={}",
            route.dest,
            route.ticks,
            route.legs.len()
        );
    }
}
