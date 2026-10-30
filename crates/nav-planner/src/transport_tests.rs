use api::snapshot::WorldTile;

use super::{TransportEdge, TransportKind};

fn members_check_edge(kind: TransportKind, loc_id: i32, option: i32) -> TransportEdge {
    TransportEdge {
        takeoff: None,
        worn_all_req: Vec::new(),
        forbidden_held: Vec::new(),
        forbidden_worn: Vec::new(),
        kind,
        player_delta: None,
        at: WorldTile {
            x: 3200,
            z: 3200,
            level: 0,
        },
        to: WorldTile {
            x: 3201,
            z: 3200,
            level: 0,
        },
        loc_id,
        option,
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

#[test]
fn relative_landing_returns_none_on_coordinate_overflow() {
    let mut edge = members_check_edge(TransportKind::Ladder, 2871, 1);
    for (takeoff, delta) in [
        (
            WorldTile {
                x: 1,
                z: 0,
                level: 0,
            },
            WorldTile {
                x: i32::MAX,
                z: 0,
                level: 0,
            },
        ),
        (
            WorldTile {
                x: 0,
                z: 1,
                level: 0,
            },
            WorldTile {
                x: 0,
                z: i32::MAX,
                level: 0,
            },
        ),
        (
            WorldTile {
                x: 0,
                z: 0,
                level: 1,
            },
            WorldTile {
                x: 0,
                z: 0,
                level: i32::MAX,
            },
        ),
    ] {
        edge.player_delta = Some(delta);
        assert_eq!(edge.landing_from(takeoff), None);
    }
    edge.player_delta = Some(WorldTile {
        x: 2,
        z: -3,
        level: 1,
    });
    assert_eq!(
        edge.landing_from(WorldTile {
            x: 10,
            z: 20,
            level: 0
        }),
        Some(WorldTile {
            x: 12,
            z: 17,
            level: 1
        })
    );
}

#[test]
fn crossing_identity_keeps_one_anchor_and_distinguishes_both_wall_directions() {
    let mut inward = members_check_edge(TransportKind::Door, 3507, 1);
    inward.dir = Some(super::DoorDir::S);
    inward.to = WorldTile {
        z: inward.at.z - 1,
        ..inward.at
    };
    let mut outward = inward.clone();
    outward.dir = Some(super::DoorDir::N);
    outward.to = inward.at;
    let inward_id = inward.crossing_id().unwrap();
    let outward_id = outward.crossing_id().unwrap();
    assert_eq!(inward_id.at, inward.at);
    assert_eq!(outward_id.at, inward.at);
    assert_eq!(inward_id.takeoff, inward.at);
    assert_eq!(outward_id.takeoff, inward.to);
    assert_ne!(inward_id, outward_id);
}

#[test]
fn crossing_identity_distinguishes_exact_takeoffs_on_both_sides_of_one_anchor() {
    let mut edge = members_check_edge(TransportKind::Door, 3443, 1);
    let mut identities = Vec::new();
    for dz in [-1, 1] {
        edge.takeoff = Some(WorldTile {
            z: edge.at.z + dz,
            ..edge.at
        });
        let identity = edge.crossing_id().unwrap();
        assert_eq!(identity.at, edge.at);
        assert_eq!(Some(identity.takeoff), edge.takeoff);
        identities.push(identity);
    }
    assert_ne!(identities[0], identities[1]);
}
