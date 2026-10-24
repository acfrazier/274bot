use api::snapshot::WorldTile;
use client::dash3d::CollisionFlag;
use nav_bake::pack::{encode, encode_grid};
use nav_planner::collision::WorldCollision;
use nav_planner::grid::StepGrid;
use nav_planner::router::{find, find_allow_teleports, Leg};
use nav_planner::transport::{TransportEdge, TransportGraph, TransportKind};
use nav_planner::world::NavWorld;

const BLOCKED: u32 = CollisionFlag::SQ_BLOCKED as u32 | CollisionFlag::WALK_BLOCK_FLAGS as u32;

fn tile(x: i32, z: i32, level: i32) -> WorldTile {
    WorldTile { x, z, level }
}
#[test]
fn load_pack_path_round_trips_a_fixture_grid() {
    let dir = std::env::temp_dir().join(format!(
        "274bot-navworld-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("fixture.navpack");
    std::fs::write(&path, encode_grid(&StepGrid::fixture_door_corridor())).unwrap();
    let w = NavWorld::load_pack(&path).expect("pack loads");
    assert_eq!(w.collision.width, 5);
    assert_eq!(w.graph.edges.len(), 1);
    assert!(w.collision.walkable(tile(1, 0, 0)));
    assert!(!w.collision.walkable(tile(2, 0, 0)));
    // The decoded grid routes exactly like the authored one.
    let r = find(&w.collision, &w.graph, tile(0, 0, 0), tile(4, 0, 0)).unwrap();
    // The 1-tick door taken from the origin (within the interact
    // radius of at=(1,0)) + 1 walk tile (0.5).
    assert_eq!(r.ticks, 1.5);
    assert_eq!(r.legs.len(), 3);
    let _ = std::fs::remove_dir_all(&dir);
}
#[test]
fn from_bytes_decodes_v8_and_legacy_grid_without_a_path() {
    let mut plane = vec![0u32; 4];
    plane[0] = BLOCKED;
    let mut flags = vec![0u32; 4 * plane.len()];
    flags[..plane.len()].copy_from_slice(&plane);
    let (walk, blocked) = nav_planner::collision::pack_walk(&flags);
    let collision = WorldCollision {
        origin: tile(0, 0, 0),
        width: 2,
        height: 2,
        walk,
        blocked,
        flags: None,
    };
    let v8 = encode(&collision, &TransportGraph::default(), &[]).unwrap();
    let packed = NavWorld::from_bytes(&v8).expect("v8 bytes decode");
    assert_eq!(packed.collision.width, 2);
    assert!(!packed.collision.walkable(tile(0, 0, 0)));

    let grid_bytes = encode_grid(&StepGrid::fixture_door_corridor());
    let grid = NavWorld::from_bytes(&grid_bytes).expect("legacy grid bytes decode");
    assert_eq!(grid.collision.width, 5);
    assert_eq!(grid.graph.edges.len(), 1);
    assert!(grid.collision.walkable(tile(1, 0, 0)));
    assert!(!grid.collision.walkable(tile(2, 0, 0)));
}

#[test]
fn load_pack_path_round_trips_a_world() {
    // A 5-tile corridor split by a door: the packed collision and
    // transport graph route exactly like the authored world.
    let mut plane = vec![0u32; 5];
    plane[2] = BLOCKED;
    let mut flags = vec![0u32; 4 * plane.len()];
    flags[..plane.len()].copy_from_slice(&plane);
    let (walk, blocked) = nav_planner::collision::pack_walk(&flags);
    let collision = WorldCollision {
        origin: tile(0, 0, 0),
        width: 5,
        height: 1,
        walk,
        blocked,
        flags: None,
    };
    let mut graph = TransportGraph::default();
    graph.edges.push(TransportEdge {
        takeoff: None,
        worn_all_req: Vec::new(),
        forbidden_held: Vec::new(),
        forbidden_worn: Vec::new(),
        kind: TransportKind::Door,
        player_delta: None,
        at: tile(1, 0, 0),
        to: tile(3, 0, 0),
        loc_id: 1530,
        option: 1,
        ticks: 1,
        dir: None,
        open_loc_id: None,
        skill_req: vec![],
        item_req: vec![],
        consumed_req: vec![],
        item_returns: vec![],
        quest_req: vec![],
        varp_req: vec![],
        worn_req: vec![],
        members_req: false,
        wildy_cap: None,
        quest_gates: None,
    });
    graph.at.entry(tile(1, 0, 0)).or_default().push(0);

    let dir = std::env::temp_dir().join(format!(
        "274bot-navworld-roundtrip-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("fixture.navpack");
    std::fs::write(&path, encode(&collision, &graph, &[]).unwrap()).unwrap();
    let w = NavWorld::load_pack(&path).expect("pack loads");
    assert_eq!(w.collision.origin, tile(0, 0, 0));
    assert_eq!(w.collision.walk, collision.walk);
    assert!(w.collision.flags.is_none());
    assert_eq!(w.graph.edges, graph.edges);
    assert_eq!(w.graph.at, graph.at);
    let r = find(&w.collision, &w.graph, tile(0, 0, 0), tile(4, 0, 0)).unwrap();
    // The 1-tick door taken from the origin (within the interact
    // radius of at=(1,0)) + 1 walk tile (0.5).
    assert_eq!(r.ticks, 1.5);
    assert_eq!(r.legs.len(), 3);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn world_round_trips_the_teleport_layer_off_the_default_find() {
    // A 5×5 bake walled down the middle: only the packed any-tile
    // teleport can cross it, and only under find_allow_teleports.
    let mut plane = vec![0u32; 25];
    for z in 0..5 {
        plane[z * 5 + 1] = BLOCKED;
        plane[z * 5 + 2] = BLOCKED;
    }
    let mut flags = vec![0u32; 4 * plane.len()];
    flags[..plane.len()].copy_from_slice(&plane);
    let (walk, blocked) = nav_planner::collision::pack_walk(&flags);
    let collision = WorldCollision {
        origin: tile(0, 0, 0),
        width: 5,
        height: 5,
        walk,
        blocked,
        flags: None,
    };
    let mut graph = TransportGraph::default();
    graph.teleports.push(TransportEdge {
        takeoff: None,
        worn_all_req: Vec::new(),
        forbidden_held: Vec::new(),
        forbidden_worn: Vec::new(),
        kind: TransportKind::Teleport,
        player_delta: None,
        at: tile(0, 0, 0),
        to: tile(4, 4, 0),
        loc_id: 0,
        option: 0,
        ticks: 3,
        dir: None,
        open_loc_id: None,
        skill_req: vec![(6, 25)],
        item_req: vec![],
        consumed_req: vec![(554, 1), (556, 3), (563, 1)],
        item_returns: vec![],
        quest_req: vec![],
        varp_req: vec![],
        worn_req: vec![],
        members_req: false,
        wildy_cap: None,
        quest_gates: None,
    });

    let dir = std::env::temp_dir().join(format!(
        "274bot-navworld-teles-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("fixture-teles.navpack");
    std::fs::write(&path, encode(&collision, &graph, &[]).unwrap()).unwrap();
    let w = NavWorld::load_pack(&path).expect("pack loads");
    assert_eq!(w.graph.teleports, graph.teleports);
    assert!(w.graph.edges.is_empty());
    assert!(w.graph.at.is_empty());
    // Default find ignores the teleport layer entirely…
    assert!(find(&w.collision, &w.graph, tile(0, 0, 0), tile(4, 4, 0)).is_err());
    // …and find_allow_teleports unions it in from anywhere (the state
    // proves the Varrock cast: Magic 25 + fire/air/law runes).
    let cast = nav_planner::world_state::WorldState {
        stats: std::collections::HashMap::from([(6, 25)]),
        inv: std::collections::HashMap::from([(554, 1), (556, 3), (563, 1)]),
        ..nav_planner::world_state::WorldState::default()
    };
    let r =
        find_allow_teleports(&w.collision, &w.graph, tile(0, 0, 0), tile(4, 4, 0), &cast).unwrap();
    assert_eq!(r.ticks, 3.0);
    assert!(r.legs.iter().any(|l| matches!(l, Leg::Transport { .. })));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn load_pack_round_trips_bank_stands() {
    // The v16 pack stores the bank stand table; NavWorld::banks exposes
    // it to the Banking session.
    let mut plane = vec![0u32; 4];
    plane[0] = BLOCKED;
    let mut flags = vec![0u32; 4 * plane.len()];
    flags[..plane.len()].copy_from_slice(&plane);
    let (walk, blocked) = nav_planner::collision::pack_walk(&flags);
    let collision = WorldCollision {
        origin: tile(0, 0, 0),
        width: 2,
        height: 2,
        walk,
        blocked,
        flags: None,
    };
    let banks = vec![nav_planner::pack::BankStand {
        name: "Bank booth".into(),
        tile: tile(1, 1, 0),
        access: nav_planner::pack::BankAccess::Booth { op: 2 },
    }];
    let dir = std::env::temp_dir().join(format!(
        "274bot-navworld-banks-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("fixture-banks.navpack");
    std::fs::write(
        &path,
        encode(&collision, &TransportGraph::default(), &banks).unwrap(),
    )
    .unwrap();
    let w = NavWorld::load_pack(&path).expect("pack loads");
    assert_eq!(w.banks(), &banks);
    let _ = std::fs::remove_dir_all(&dir);
}
