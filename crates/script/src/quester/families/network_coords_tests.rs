use super::*;
use api::snapshot::SceneView;

// F, observed tick 66: the real 32x32 collision window from the release pack.
// Pack SHA-256: 731ca2ecd11248e0c9b0ee7620107b655e824326a65cf09712d4cbc8c360db69.
// The retained source is RELEASE-0.2.9.1/wrong-memory-probes/recorded-collision.json.
// Words are x-major/z-minor, matching SceneView and the host flood helper.
const RECORDED_COLLISION_RLE: &[(usize, i32)] = &[
    (2, 0x00000000),
    (1, 0x00280100),
    (19, 0x00000000),
    (1, 0x00280100),
    (11, 0x00000000),
    (1, 0x00280100),
    (7, 0x00000000),
    (1, 0x00280100),
    (3, 0x00000000),
    (1, 0x00280100),
    (7, 0x00000000),
    (1, 0x00280100),
    (11, 0x00000000),
    (1, 0x00280100),
    (9, 0x00000000),
    (1, 0x00280100),
    (9, 0x00000000),
    (1, 0x00280100),
    (11, 0x00000000),
    (1, 0x00280100),
    (4, 0x00000000),
    (2, 0x00280100),
    (7, 0x00000000),
    (2, 0x00280100),
    (4, 0x00000000),
    (1, 0x00280100),
    (11, 0x00000000),
    (1, 0x00280100),
    (4, 0x00000000),
    (2, 0x00280100),
    (3, 0x00000000),
    (1, 0x00280100),
    (3, 0x00000000),
    (2, 0x00280100),
    (4, 0x00000000),
    (1, 0x00280100),
    (11, 0x00000000),
    (1, 0x00280100),
    (9, 0x00000000),
    (1, 0x00280100),
    (9, 0x00000000),
    (1, 0x00280100),
    (11, 0x00000000),
    (2, 0x00280100),
    (17, 0x00000000),
    (2, 0x00280100),
    (11, 0x00000000),
    (17, 0x00280100),
    (1, 0x00000008),
    (3, 0x00280100),
    (26, 0x00000000),
    (2, 0x00280100),
    (1, 0x00000080),
    (2, 0x00280100),
    (28, 0x00000000),
    (1, 0x00280100),
    (1, 0x00000000),
    (1, 0x00280100),
    (29, 0x00000000),
    (1, 0x00280100),
    (1, 0x00000000),
    (2, 0x00280100),
    (27, 0x00000000),
    (2, 0x00280100),
    (1, 0x00000000),
    (3, 0x00280100),
    (24, 0x00000000),
    (4, 0x00280100),
    (1, 0x00000000),
    (4, 0x00280100),
    (22, 0x00000000),
    (3, 0x00280100),
    (5, 0x00000000),
    (2, 0x00280100),
    (21, 0x00000000),
    (3, 0x00280100),
    (6, 0x00000000),
    (2, 0x00280100),
    (16, 0x00000000),
    (8, 0x00280100),
    (6, 0x00000000),
    (2, 0x00280100),
    (16, 0x00000000),
    (1, 0x00280100),
    (13, 0x00000000),
    (2, 0x00280100),
    (16, 0x00000000),
    (8, 0x00280100),
    (6, 0x00000000),
    (2, 0x00280100),
    (22, 0x00000000),
    (10, 0x00280100),
    (23, 0x00000000),
    (9, 0x00280100),
    (392, 0x00000000),
];

fn recorded_scene() -> SceneView {
    let mut collision_flags = Vec::with_capacity(32 * 32);
    for &(count, word) in RECORDED_COLLISION_RLE {
        collision_flags.resize(collision_flags.len() + count, word);
    }
    assert_eq!(collision_flags.len(), 32 * 32);
    SceneView {
        available: true,
        base_x: 3424,
        base_z: 9878,
        level: 0,
        width: 32,
        height: 32,
        collision_flags,
    }
}

fn open_scene(base_x: i32, base_z: i32) -> SceneView {
    SceneView {
        available: true,
        base_x,
        base_z,
        level: 0,
        width: 16,
        height: 16,
        collision_flags: vec![0; 16 * 16],
    }
}

fn actors(
    rendered_player: WorldTile,
    network_player: WorldTile,
    rendered_npc: WorldTile,
    network_npc: WorldTile,
    scene: SceneView,
) -> GameSnapshot {
    assert!(
        api::test_support::game_data_available(),
        "network regressions require generated data"
    );
    let mut snapshot = ready();
    snapshot.seed_local_player(local_player_with_network(rendered_player, network_player));
    let mut npc = fred_npc(1, rendered_npc);
    npc.network = network_npc;
    snapshot.seed_npcs(vec![npc]);
    snapshot.seed_scene(scene);
    snapshot
}

fn drezel(snapshot: &mut GameSnapshot) -> dialogue::DialogueArgs {
    let mut npc = snapshot.npcs()[0].clone();
    npc.index = 6271;
    npc.r#type = Some(1049);
    npc.name = Some("Drezel".into());
    snapshot.seed_npcs(vec![npc]);
    let mut args = fred_args();
    args.target = dialogue::DialogueTarget::Npc {
        id: 1049,
        name: Arc::from("Drezel"),
    };
    args
}

fn no_talk(ledger: &Option<Box<ledger::Ledger>>) {
    assert!(
        ledger
            .as_ref()
            .unwrap()
            .outbox
            .iter()
            .all(|action| !matches!(
                &action.effect,
                HostEffect::Interaction(InteractReq::Npc { .. })
            )),
        "Talk must not precede the network approach"
    );
}

fn approach_target(ledger: &Option<Box<ledger::Ledger>>) -> WorldTile {
    match &ledger.as_ref().unwrap().outbox.last().unwrap().effect {
        HostEffect::Walk(request) | HostEffect::AssessWalk(request) => request.target,
        _ => panic!("a walk or route assessment must precede Talk"),
    }
}

#[test]
fn network_coords_recorded_tick66_approaches_drezel_before_talk() {
    let mut snapshot = actors(
        tile(3440, 9890),
        tile(3440, 9893),
        tile(3439, 9894),
        tile(3438, 9896),
        recorded_scene(),
    );
    let args = drezel(&mut snapshot);
    let mut ledger = None;
    let _handle = with_tick_scene_reach(&snapshot, &mut ledger, 66, |tick| {
        assert_eq!(
            reach::npc_route_steps(&tick.cx, &snapshot.npcs()[0]),
            Some(3)
        );
        let handle = tick
            .actions
            .begin::<dialogue::Dialogue>(args, &mut tick.cx)
            .unwrap();
        assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
        handle
    });
    no_talk(&ledger);
    assert_eq!(approach_target(&ledger), tile(3438, 9896));
}

#[test]
fn network_coords_recorded_near_server_tuple_still_permits_talk() {
    let mut snapshot = actors(
        tile(3439, 9894),
        tile(3437, 9896),
        tile(3436, 9897),
        tile(3436, 9897),
        recorded_scene(),
    );
    let args = drezel(&mut snapshot);
    let mut ledger = None;
    let _handle = with_tick_scene_reach(&snapshot, &mut ledger, 70, |tick| {
        let handle = tick
            .actions
            .begin::<dialogue::Dialogue>(args, &mut tick.cx)
            .unwrap();
        assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
        handle
    });
    assert!(ledger
        .as_ref()
        .unwrap()
        .outbox
        .iter()
        .any(|action| matches!(
            &action.effect,
            HostEffect::Interaction(InteractReq::Npc {
                index: Some(6271),
                ..
            })
        )));
}

#[test]
fn network_coords_dialogue_range_uses_the_nearest_footprint_tile() {
    for (player, expected) in [
        (tile(9, 5), false),
        (tile(5, 9), false),
        (tile(10, 5), true),
        (tile(5, 10), true),
    ] {
        let mut snapshot = actors(
            tile(100, 100),
            player,
            tile(100, 100),
            tile(5, 5),
            open_scene(0, 0),
        );
        let mut npc = snapshot.npcs()[0].clone();
        npc.size = 3;
        snapshot.seed_npcs(vec![npc]);
        let mut ledger = None;
        with_tick_scene_reach(&snapshot, &mut ledger, 1, |tick| {
            let handle = tick
                .actions
                .begin::<dialogue::Dialogue>(fred_args(), &mut tick.cx)
                .unwrap();
            assert_eq!(
                handle
                    .inspect_in(&mut tick.cx, |dialogue, cx| dialogue
                        .target_out_of_range(cx))
                    .unwrap(),
                expected,
                "range from {player:?} to a size-three footprint"
            );
        });
    }
}

#[test]
fn network_coords_dialogue_replans_when_only_the_network_target_moves() {
    let mut snapshot = actors(
        tile(100, 100),
        tile(100, 100),
        tile(104, 100),
        tile(104, 100),
        open_scene(96, 96),
    );
    let mut ledger = None;
    let handle = with_tick_scene_reach(&snapshot, &mut ledger, 1, |tick| {
        let handle = tick
            .actions
            .begin::<dialogue::Dialogue>(fred_args(), &mut tick.cx)
            .unwrap();
        assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
        handle
    });
    assert_eq!(approach_target(&ledger), tile(104, 100));
    post_walk_receipt(&mut ledger, 2, WalkEnd::Arrived, None);
    snapshot.seed_local_player(local_player_with_network(tile(100, 100), tile(103, 100)));
    let mut npc = snapshot.npcs()[0].clone();
    npc.network = tile(107, 100);
    snapshot.seed_npcs(vec![npc]);
    with_tick_scene_reach(&snapshot, &mut ledger, 2, |tick| {
        assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
    });
    no_talk(&ledger);
    assert_eq!(approach_target(&ledger), tile(107, 100));
}

#[test]
fn network_coords_stale_origin_flood_cannot_authorize_talk() {
    let old = actors(
        tile(99, 100),
        tile(99, 100),
        tile(101, 100),
        tile(101, 100),
        open_scene(96, 96),
    );
    let stale = scene_reach(&old);
    let snapshot = actors(
        tile(99, 100),
        tile(100, 100),
        tile(101, 100),
        tile(101, 100),
        open_scene(96, 96),
    );
    let mut ledger = None;
    let _handle = with_tick_reach(&snapshot, &stale, &mut ledger, 1, |tick| {
        let handle = tick
            .actions
            .begin::<dialogue::Dialogue>(fred_args(), &mut tick.cx)
            .unwrap();
        assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
        handle
    });
    no_talk(&ledger);
    assert_eq!(approach_target(&ledger), tile(101, 100));
    assert!(matches!(
        &ledger.as_ref().unwrap().outbox.last().unwrap().effect,
        HostEffect::AssessWalk(_)
    ));
}

#[test]
fn network_coords_chooser_route_depth_and_arrival_share_the_network_footprint() {
    let snapshot = actors(
        tile(5, 5),
        tile(8, 5),
        tile(5, 5),
        tile(9, 5),
        open_scene(0, 0),
    );
    with_tick_scene_reach(&snapshot, &mut None, 1, |tick| {
        let npc = &snapshot.npcs()[0];
        assert_eq!(reach::npc_route_steps(&tick.cx, npc), Some(0));
        assert!(reach::npc_adjacent(&tick.cx, npc));
        let choice = reach::choose_npc(
            &tick.cx,
            758,
            Some("Talk-to"),
            reach::Area::new(Some(tile(9, 5)), 0),
            &reach::Avoid::default(),
        );
        assert!(matches!(choice, reach::Pick::Reachable(npc) if npc.index == 7));
    });
}

fn point_scene(player: WorldTile) -> SceneView {
    SceneView {
        available: true,
        base_x: player.x,
        base_z: player.z,
        level: player.level,
        width: 1,
        height: 1,
        collision_flags: vec![0],
    }
}

fn npc_reach_args() -> reach::ReachArgs {
    reach::ReachArgs {
        kind: reach::ReachKind::Npc {
            id: 758,
            name: Arc::from("Fred the Farmer"),
        },
        op: Arc::from("Talk-to"),
        anchor: None,
        radius: 10,
        wait_if_missing: true,
        target_tile: None,
        reachable_only: false,
    }
}

#[test]
fn network_coords_generic_npc_reach_approaches_the_network_target() {
    let snapshot = actors(
        tile(100, 100),
        tile(100, 100),
        tile(101, 100),
        tile(105, 100),
        point_scene(tile(100, 100)),
    );
    let mut ledger = None;
    let _handle = with_tick_scene_reach(&snapshot, &mut ledger, 1, |tick| {
        tick.actions
            .begin::<reach::Reach>(npc_reach_args(), &mut tick.cx)
            .unwrap()
    });
    no_talk(&ledger);
    assert_eq!(approach_target(&ledger), tile(105, 100));
}

#[test]
fn network_coords_generic_npc_reach_retry_resolves_the_network_target_again() {
    let mut snapshot = actors(
        tile(100, 100),
        tile(100, 100),
        tile(104, 100),
        tile(104, 100),
        point_scene(tile(100, 100)),
    );
    let mut ledger = None;
    let handle = with_tick_scene_reach(&snapshot, &mut ledger, 1, |tick| {
        tick.actions
            .begin::<reach::Reach>(npc_reach_args(), &mut tick.cx)
            .unwrap()
    });
    assert_eq!(approach_target(&ledger), tile(104, 100));
    post_walk_receipt(
        &mut ledger,
        2,
        WalkEnd::NoPath,
        Some("first approach failed"),
    );
    for _ in 0..2 {
        assert!(with_tick_scene_reach(&snapshot, &mut ledger, 2, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        })
        .is_pending());
    }
    let mut npc = snapshot.npcs()[0].clone();
    npc.network = tile(107, 100);
    snapshot.seed_npcs(vec![npc]);
    assert!(with_tick_scene_reach(&snapshot, &mut ledger, 3, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert_eq!(approach_target(&ledger), tile(107, 100));
}
