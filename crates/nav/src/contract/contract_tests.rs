use std::mem::size_of;
use std::sync::Arc;

use api::identity::{ProcessIncarnation, RequestIdMinter, Scope, WorkIdentity};
use api::snapshot::WorldTile;
use api::work::InputRevision;
use client::dash3d::CollisionFlag;

use crate::collision::{pack_walk, WorldCollision};
use crate::router::{Leg, Route};
use crate::transport::{DoorDir, TransportEdge, TransportGraph, TransportKind};
use crate::world::NavWorld;
use crate::zones::{Zone, ZoneClass, ZoneExempt, ZoneGroup, ZoneKey, ZoneKind, ZoneTable};

use super::*;

fn tile(x: i32, z: i32, level: i32) -> WorldTile {
    WorldTile { x, z, level }
}

fn grid(origin: WorldTile, width: usize, height: usize, level0: Vec<u32>) -> WorldCollision {
    assert_eq!(level0.len(), width * height);
    let plane = width * height;
    let mut flags = vec![0u32; 4 * plane];
    flags[..plane].copy_from_slice(&level0);
    let (walk, blocked) = pack_walk(&flags);
    WorldCollision {
        origin,
        width,
        height,
        walk,
        blocked,
        flags: Some(flags),
    }
}

fn lease_from(collision: WorldCollision) -> NavLease {
    NavLease::wrap(Arc::new(NavWorld::from_parts(
        collision,
        TransportGraph::default(),
        Vec::new(),
    )))
    .unwrap()
}

fn work_identity() -> WorkIdentity {
    WorkIdentity::new(
        RequestIdMinter::new().mint().unwrap(),
        ProcessIncarnation::mint(),
        Scope::Process,
    )
    .unwrap()
}

#[test]
fn collision_view_is_a_borrow_not_a_world_copy() {
    assert!(size_of::<CollisionView<'_>>() <= size_of::<usize>() * 3);
    fn assert_copy<T: Copy>() {}
    assert_copy::<CollisionView<'_>>();
    assert_copy::<RouteView<'_>>();
    assert_copy::<LegView<'_>>();
}

#[test]
fn standable_is_not_walkable_on_a_face_flag() {
    let origin = tile(3200, 3200, 0);
    let lease = lease_from(grid(origin, 2, 1, vec![CollisionFlag::W_N as u32, 0]));
    let view = lease.collision();
    let flagged = tile(3200, 3200, 0);
    let clear = tile(3201, 3200, 0);
    assert!(view.standable(flagged));
    assert!(!view.walkable(flagged));
    assert!(view.standable(clear));
    assert!(view.walkable(clear));
    assert!(view.contains_tile(flagged));
    assert!(!view.contains_tile(tile(3199, 3200, 0)));
}

#[test]
fn reach_uses_walkable_and_area_uses_standable() {
    let lease = lease_from(grid(
        tile(3200, 3200, 0),
        2,
        1,
        vec![CollisionFlag::W_N as u32, 0],
    ));
    let view = lease.collision();
    let solid = tile(3200, 3200, 0);
    let neighbour = tile(3201, 3200, 0);
    // Reach: a face-flag destination is solid (!walkable), so adjacent
    // arrival succeeds. Treating standable as walkable would refuse it.
    assert!(view.arrived(neighbour, solid, ArrivalSpec::REACH));
    assert!(view.arrived(neighbour, neighbour, ArrivalSpec::REACH));
    // Area radius 0 on the face-flag tile: current standability, not walk.
    assert!(view.arrived(solid, solid, ArrivalSpec::area(0)));
    assert!(view.area_has_goal(solid, 0));
    assert!(!view.arrived(neighbour, solid, ArrivalSpec::area(0)));
    assert!(!view.arrived(tile(3201, 3200, 1), solid, ArrivalSpec::area(4)));
}

#[test]
fn step_ok_is_directional_not_a_walkable_boolean() {
    let lease = lease_from(grid(
        tile(3200, 3200, 0),
        2,
        1,
        vec![CollisionFlag::W_N as u32, 0],
    ));
    let view = lease.collision();
    let flagged = tile(3200, 3200, 0);
    let clear = tile(3201, 3200, 0);
    assert!(view.walkable(clear));
    assert!(
        view.step_ok(clear, flagged),
        "W_N does not block an east-west step; walkable() would refuse the dest"
    );
    assert!(!view.walkable(flagged));
    assert!(!view.step_ok(clear, clear));
    assert!(!view.step_ok(clear, tile(3201, 3200, 1)));
}

#[test]
fn a_lease_used_against_another_pack_is_refused() {
    let a = lease_from(grid(tile(3200, 3200, 0), 1, 1, vec![0]));
    let b = lease_from(grid(tile(3200, 3200, 0), 1, 1, vec![0]));
    assert_ne!(a.identity().pack(), b.identity().pack());
    let dest = tile(3200, 3200, 0);
    let plan = a.publish_route(
        work_identity(),
        InputRevision::FIRST,
        Route {
            legs: vec![Leg::Walk { tiles: vec![dest] }],
            dest,
            ticks: 0.0,
        },
    );
    assert!(plan.view(&a).is_ok());
    assert_eq!(plan.view(&b).unwrap_err(), ContractError::PackMismatch);
}

#[test]
fn route_view_keeps_exact_half_tick_cost_and_borrowed_legs() {
    let lease = lease_from(grid(tile(3200, 3200, 0), 2, 1, vec![0, 0]));
    let from = tile(3200, 3200, 0);
    let to = tile(3201, 3200, 0);
    let edge = TransportEdge {
        kind: TransportKind::Door,
        at: to,
        to: from,
        takeoff: None,
        player_delta: None,
        loc_id: 1,
        option: 1,
        ticks: 2,
        dir: Some(DoorDir::N),
        open_loc_id: None,
        skill_req: vec![(16, 10)],
        item_req: vec![(995, 10)],
        consumed_req: vec![(995, 5)],
        item_returns: vec![(995, 1)],
        quest_req: vec![],
        varp_req: vec![],
        worn_req: vec![772],
        worn_all_req: vec![1, 2],
        members_req: false,
        wildy_cap: None,
        quest_gates: None,
    };
    let plan = lease.publish_route(
        work_identity(),
        InputRevision::FIRST,
        Route {
            legs: vec![
                Leg::Walk {
                    tiles: vec![from, to],
                },
                Leg::Transport {
                    edge: Box::new(edge),
                },
            ],
            dest: from,
            ticks: 0.5,
        },
    );
    let view = plan.view(&lease).unwrap();
    assert_eq!(view.destination(), from);
    assert_eq!(view.ticks(), 0.5);
    assert_ne!(view.ticks(), view.ticks().trunc());
    assert_eq!(view.leg_count(), 2);
    let walk = view.leg(0).unwrap().walk_tiles().unwrap();
    assert_eq!(walk, &[from, to]);
    let hop = view.leg(1).unwrap().transport().unwrap();
    assert_eq!(hop.kind(), TransportKind::Door);
    assert_eq!(hop.ticks(), 2);
    assert_eq!(hop.anchor(), to);
    assert_eq!(hop.landing(), from);
    assert_eq!(hop.effective_takeoff(), to);
    assert_eq!(hop.held(), &[(995, 10)]);
    assert_eq!(hop.consumed(), &[(995, 5)]);
    assert_eq!(hop.returned(), &[(995, 1)]);
    assert_eq!(hop.worn_any(), &[772]);
    assert_eq!(hop.worn_all(), &[1, 2]);
    assert_ne!(hop.held(), hop.consumed());
    assert_ne!(hop.worn_any(), hop.worn_all());
    assert!(!hop.has_quest_gate());
    assert!(view.leg(2).is_none());
}

#[test]
fn zone_exemptions_are_bounded_and_unknown_keys_refuse() {
    assert_eq!(MAX_ZONE_KEYS, 8);
    let keys: Vec<ZoneKey> = (0..9).map(|i| ZoneKey::Zone(i as u16)).collect();
    assert!(ZoneExempt::named(&keys).is_err());
    let eight: Vec<ZoneKey> = (0..8).map(|i| ZoneKey::Zone(i as u16)).collect();
    assert!(ZoneExempt::named(&eight).is_ok());
    assert_eq!(
        AVOID_CATALOG_IDS,
        &["white-wolf-mountain", "draynor-jail-guards"]
    );

    let mut grouped = Zone::npc(tile(5, 5, 0), 2, ZoneClass::Always, u16::MAX, 0);
    grouped.group = 0;
    let table = ZoneTable::from_parts(
        vec![grouped],
        vec![ZoneKind::new("wolf", "Wolf", 96, 1, false, false)],
        vec![ZoneGroup::new(
            "white-wolf-mountain",
            "White Wolf Mountain",
            AvoidRect {
                min_x: 0,
                max_x: 10,
                min_z: 0,
                max_z: 10,
                level: None,
            },
            vec![0u16],
        )],
        Vec::new(),
        Vec::new(),
        tile(0, 0, 0),
        12,
        12,
        &crate::transport::WildernessRules::default(),
    )
    .unwrap();
    let mut world_a = NavWorld::from_parts(
        grid(tile(0, 0, 0), 1, 1, vec![0]),
        TransportGraph::default(),
        Vec::new(),
    );
    world_a.graph.zones = Some(table.clone());
    let mut world_b = NavWorld::from_parts(
        grid(tile(0, 0, 0), 1, 1, vec![0]),
        TransportGraph::default(),
        Vec::new(),
    );
    world_b.graph.zones = Some(table);
    let a = NavLease::wrap(Arc::new(world_a)).unwrap();
    let b = NavLease::wrap(Arc::new(world_b)).unwrap();
    let zones = a.zones().unwrap();
    let bound = zones.resolve("white-wolf-mountain").unwrap();
    assert_eq!(bound.key(), ZoneKey::Group(0));
    assert!(zones.name(bound).unwrap().contains("white-wolf-mountain"));
    assert!(zones.bind(ZoneKey::Zone(99)).is_err());
    assert_eq!(
        b.zones().unwrap().zone(bound).unwrap_err(),
        ContractError::PackMismatch
    );
}

#[test]
fn wilderness_view_has_no_grid_and_reports_membership() {
    let lease = lease_from(grid(tile(3200, 3200, 0), 1, 1, vec![0]));
    let wild = lease.wilderness();
    let t = tile(3200, 3200, 0);
    assert!(!wild.contains(t));
    assert_eq!(wild.level(t), 0);
}

#[test]
fn route_request_defaults_to_reach_without_wilderness() {
    let req = RouteRequest::new(tile(1, 1, 0), tile(2, 2, 0));
    assert_eq!(req.arrival(), ArrivalSpec::REACH);
    assert!(!req.permissions().allow_teleports());
    assert!(!req.permissions().allow_wilderness());
}

#[test]
fn real_pack_views_borrow_the_lease_and_keep_catalog_ids() {
    let Some(world) = NavWorld::load_default_pack_or_skip() else {
        return;
    };
    let origin = world.collision.origin;
    let lease = NavLease::wrap(Arc::new(world)).unwrap();
    assert_eq!(lease.identity().format(), FORMAT_ID);
    let view = lease.collision();
    let inside = tile(origin.x + 1, origin.z + 1, 0);
    let _ = view.standable(inside);
    let _ = view.walkable(inside);
    if let Some(zones) = lease.zones() {
        for id in AVOID_CATALOG_IDS {
            let _ = zones.resolve(id);
        }
        assert!(zones.bind(ZoneKey::Zone(u16::MAX)).is_err());
    }
    let wild = lease.wilderness();
    let _ = wild.contains(inside);
    let _ = wild.level(inside);
}
