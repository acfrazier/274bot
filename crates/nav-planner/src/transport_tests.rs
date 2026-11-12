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

/// The service-pick cost unit is half a tick (`api::service_pick::RouteCost`).
/// The router adds 0.5 per run step, the walk rate of 1.0 per walked step, and
/// each transport edge's `ticks` (an `i32`). A route is therefore a whole
/// count of half ticks only while every edge of the shipped graph carries a
/// non-negative whole tick count whose double fits the cost range.
#[test]
fn every_transport_cost_is_whole_half_ticks() {
    use crate::essence::ESSENCE_MINE_EXIT_TICKS;
    use crate::router::{PER_STEP_RUN, PER_STEP_WALK};

    for rate in [PER_STEP_RUN, PER_STEP_WALK] {
        assert_eq!(
            (rate * 2.0).fract(),
            0.0,
            "a walked step is whole half ticks"
        );
    }
    const {
        assert!(ESSENCE_MINE_EXIT_TICKS >= 0);
    }

    let default_pack = crate::world::NavWorld::default_pack_path();
    let default_pack = default_pack.to_string_lossy();
    let Some(path) = api::test_support::local_asset_path(
        "NAV_PACK",
        &default_pack,
        "revision-289 navigation pack",
    ) else {
        return;
    };
    let world = crate::world::NavWorld::load_pack(&path)
        .unwrap_or_else(|error| panic!("could not load nav pack at {}: {error}", path.display()));
    let mut checked = 0_usize;
    for (set, edges) in [
        ("edges", &world.graph.edges),
        ("teleports", &world.graph.teleports),
    ] {
        for edge in edges {
            let cost = f64::from(edge.ticks);
            assert!(
                edge.ticks >= 0,
                "{set} edge {:?} at {:?} has the negative cost {}",
                edge.kind,
                edge.at,
                edge.ticks
            );
            assert_eq!(
                (cost * 2.0).fract(),
                0.0,
                "{set} edge {:?} at {:?} costs {cost} ticks, not whole half ticks",
                edge.kind,
                edge.at
            );
            assert!(
                u32::try_from(edge.ticks).is_ok_and(|ticks| ticks.checked_mul(2).is_some()),
                "{set} edge {:?} at {:?} costs {} ticks, past the half-tick range",
                edge.kind,
                edge.at,
                edge.ticks
            );
            checked += 1;
        }
    }
    assert!(
        checked > 1_000,
        "the real pack must carry its transport graph, saw {checked} edges"
    );
    println!("transport edges checked: {checked}");
}
