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
