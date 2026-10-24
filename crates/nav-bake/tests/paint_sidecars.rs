use api::snapshot::WorldTile;
use client::dash3d::CollisionFlag;
use nav_bake::pack::{encode, encode_reach_sidecar};
use nav_planner::collision::WorldCollision;
use nav_planner::pack::{decode_reach_sidecar, sha256_hex};
use nav_planner::paint::bake_reach;
use nav_planner::transport::{TransportEdge, TransportGraph, TransportKind};

fn tile(x: i32, z: i32, level: i32) -> WorldTile {
    WorldTile { x, z, level }
}

fn bake(width: usize, height: usize, extras: &[(i32, i32, u32)]) -> WorldCollision {
    let mut plane = vec![0u32; width * height];
    for &(x, z, flag) in extras {
        plane[z as usize * width + x as usize] |= flag;
    }
    let mut flags = vec![0u32; 4 * plane.len()];
    flags[..plane.len()].copy_from_slice(&plane);
    let (walk, blocked) = nav_planner::collision::pack_walk(&flags);
    WorldCollision {
        origin: tile(0, 0, 0),
        width,
        height,
        walk,
        blocked,
        flags: None,
    }
}

fn edge(kind: TransportKind, at: WorldTile, to: WorldTile, loc_id: i32) -> TransportEdge {
    TransportEdge {
        takeoff: None,
        worn_all_req: Vec::new(),
        forbidden_held: Vec::new(),
        forbidden_worn: Vec::new(),
        kind,
        player_delta: None,
        at,
        to,
        loc_id,
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
    }
}

fn disconnected_world() -> WorldCollision {
    let mut extras = Vec::new();
    for z in 0..7 {
        for x in 0..7 {
            let open = (x < 3 && z < 3) || (x == 5 && z == 5);
            if !open {
                extras.push((x, z, CollisionFlag::WR_GRND as u32));
            }
        }
    }
    bake(7, 7, &extras)
}

fn graph(edges: Vec<TransportEdge>, teleports: Vec<TransportEdge>) -> TransportGraph {
    TransportGraph {
        edges,
        teleports,
        ..Default::default()
    }
}
#[test]
fn reach_sidecar_bits_equal_bake_reach_on_disconnected_and_courtyard_worlds() {
    
    use sha2::{Digest, Sha256};

    let disconnected = (
        disconnected_world(),
        graph(
            vec![edge(
                TransportKind::Door,
                tile(0, 0, 0),
                tile(1, 1, 0),
                1530,
            )],
            vec![],
        ),
    );
    let courtyard = (
        bake(5, 5, &[(2, 2, CollisionFlag::WALK_BLOCK_FLAGS as u32)]),
        TransportGraph::default(),
    );
    for (name, (c, g)) in [("disconnected", disconnected), ("courtyard", courtyard)] {
        let expected = bake_reach(&c, &g);
        let pack = encode(&c, &g, &[]).unwrap();
        let digest: [u8; 32] = Sha256::digest(&pack).into();
        let bytes = encode_reach_sidecar(c.origin, c.width, c.height, &expected, &digest);
        let side = decode_reach_sidecar(&bytes).unwrap();
        assert_eq!(
            side.bits, expected,
            "{name}: sidecar bits must equal bake_reach"
        );
        assert_eq!(side.word_count, expected.len());
        assert_eq!(side.origin, c.origin);
        assert_eq!((side.width, side.height), (c.width, c.height));
        assert_eq!(sha256_hex(&side.binding), sha256_hex(&digest));
    }
}

#[test]
fn same_geometry_different_transport_does_not_reuse_reach_binding() {
    
    use sha2::{Digest, Sha256};

    let c = disconnected_world();
    let corner = graph(
        vec![edge(
            TransportKind::Door,
            tile(0, 0, 0),
            tile(1, 1, 0),
            1530,
        )],
        vec![],
    );
    let island = graph(
        vec![edge(
            TransportKind::Door,
            tile(5, 5, 0),
            tile(5, 5, 0),
            1531,
        )],
        vec![],
    );
    let bits_a = bake_reach(&c, &corner);
    let bits_b = bake_reach(&c, &island);
    assert_ne!(
        bits_a, bits_b,
        "different seeds must produce different reach"
    );
    let pack_a = encode(&c, &corner, &[]).unwrap();
    let pack_b = encode(&c, &island, &[]).unwrap();
    let digest_a: [u8; 32] = Sha256::digest(&pack_a).into();
    let digest_b: [u8; 32] = Sha256::digest(&pack_b).into();
    assert_ne!(digest_a, digest_b);
    let side_a = encode_reach_sidecar(c.origin, c.width, c.height, &bits_a, &digest_a);
    let decoded_a = decode_reach_sidecar(&side_a).unwrap();
    assert_eq!(sha256_hex(&decoded_a.binding), sha256_hex(&digest_a));
    assert_ne!(sha256_hex(&decoded_a.binding), sha256_hex(&digest_b));
    assert_ne!(decoded_a.bits, bits_b);
}
