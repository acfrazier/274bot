use std::mem::size_of;
use std::num::NonZeroU16;
use std::sync::Arc;

use api::identity::{ProcessIncarnation, RequestIdMinter, Scope, WorkIdentity};
use api::selected::{FactKey, QuestGate};
use api::snapshot::WorldTile;
use api::work::{InputRevision, Track, WorkFamily, WorkFamilySpec};
use client::dash3d::CollisionFlag;

use crate::collision::{pack_walk, WorldCollision};
use crate::quest_gates::{QuestFamilyId, QuestGates};
use crate::router::{Leg, Route, RouteError, TargetError};
use crate::transport::{
    DoorDir, TransportEdge, TransportGraph, TransportKind, WildernessRules, WildernessZone,
};
use crate::world::NavWorld;
use crate::zones::{Zone, ZoneClass, ZoneGroup, ZoneKey, ZoneKind, ZoneTable};

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

fn digest(tag: &[u8]) -> PackDigest {
    PackDigest::of(tag)
}

fn lease_from(collision: WorldCollision) -> NavLease {
    NavLease::wrap(
        Arc::new(NavWorld::from_parts(
            collision,
            TransportGraph::default(),
            Vec::new(),
        )),
        digest(b"fixture"),
    )
    .unwrap()
}

fn work_identity() -> WorkIdentity {
    WorkIdentity::new(
        RequestIdMinter::new().mint().unwrap(),
        ProcessIncarnation::from_start_nonce(std::num::NonZeroU64::MIN),
        Scope::Process,
        None,
    )
    .unwrap()
}

fn door_edge(at: WorldTile, to: WorldTile) -> TransportEdge {
    TransportEdge {
        kind: TransportKind::Door,
        at,
        to,
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
    }
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
fn two_leases_of_the_same_content_are_distinct_leases() {
    let world = Arc::new(NavWorld::from_parts(
        grid(tile(3200, 3200, 0), 1, 1, vec![0]),
        TransportGraph::default(),
        Vec::new(),
    ));
    let content = digest(b"same-bytes");
    let a = NavLease::wrap(Arc::clone(&world), content).unwrap();
    let b = NavLease::wrap(world, content).unwrap();
    assert_ne!(a.identity().pack(), b.identity().pack());
    assert_eq!(a.identity().content(), b.identity().content());
    assert_eq!(a.identity().format(), FORMAT_ID);
    assert_eq!(a.identity().version(), VERSION);
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
    assert!(
        !view.contains_tile(tile(3200, 3200, 4)),
        "level 4 is outside the bake"
    );
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
    // Production Reach radius 1 (`WalkRequest.radius`) admits adjacent
    // solid destinations. Radius 0 is on-destination only (`is_arrived`).
    assert!(view.arrived(neighbour, solid, ArrivalSpec::reach(1)));
    assert!(!view.arrived(neighbour, solid, ArrivalSpec::REACH));
    assert!(view.arrived(neighbour, neighbour, ArrivalSpec::REACH));
    // Area radius 0 on the face-flag tile: packed-bake standability, not walk.
    assert!(view.arrived(solid, solid, ArrivalSpec::area(0)));
    assert!(view.area_has_goal(solid, 0));
    assert!(!view.arrived(neighbour, solid, ArrivalSpec::area(0)));
    assert!(!view.arrived(tile(3201, 3200, 1), solid, ArrivalSpec::area(4)));
}

#[test]
fn arrival_spec_matches_native_walk_cases() {
    assert_eq!(ArrivalSpec::REACH.radius(), 0);
    assert_eq!(ArrivalSpec::reach(7).radius(), 7);
    assert_eq!(ArrivalSpec::reach(7).kind(), ArrivalKind::Reach);
    assert_eq!(ArrivalSpec::area(3).kind(), ArrivalKind::Area);
    assert_eq!(ArrivalSpec::area(3).radius(), 3);
    assert!(ArrivalSpec::area(1).with_loc(42).is_err());
    assert_eq!(
        ArrivalSpec::area(1).with_loc(42).unwrap_err(),
        ContractError::AreaWithLoc
    );
    let reach_loc = ArrivalSpec::reach(1).with_loc(99).unwrap();
    assert_eq!(reach_loc.loc_id(), Some(99));
    assert_eq!(reach_loc.radius(), 1);
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
    assert!(
        !view.step_ok(flagged, tile(3201, 3200, 1)),
        "dx=1 onto another plane is not a step; d==(0,0) would mask a missing plane check"
    );
}

#[test]
fn step_ok_diagonal_uses_orthogonal_faces() {
    let origin = tile(3200, 3200, 0);
    let from = tile(3200, 3200, 0);
    let diag = tile(3201, 3201, 0);
    let north = tile(3200, 3201, 0);
    let clear = lease_from(grid(origin, 2, 2, vec![0, 0, 0, 0]));
    assert!(clear.collision().step_ok(from, diag));

    // South face on the northern orthogonal neighbour. Indexing is
    // `z * width + x`; (0,1) is slot 2. `step_ok` (1,1) tests MASK_S there.
    let collision = grid(origin, 2, 2, vec![0, 0, CollisionFlag::W_S as u32, 0]);
    assert!(
        collision.walkable_word(north.x, north.z, 0) & CollisionFlag::W_S as u32 != 0,
        "packed walk must keep W_S on the northern neighbour"
    );
    assert!(
        !crate::router::step_ok(&collision, from, (1, 1)),
        "NE requires the northern neighbour's south face open"
    );
    let blocked = lease_from(collision);
    assert!(!blocked.collision().walkable(north));
    assert!(!blocked.collision().step_ok(from, diag));
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
    let mut edge = door_edge(to, from);
    edge.takeoff = Some(tile(3201, 3201, 0));
    edge.quest_gates = Some(
        QuestGates::new(
            QuestFamilyId::new([1; 32], NonZeroU16::MIN),
            [QuestGate::Complete(FactKey::new("cooks-assistant"))],
        )
        .unwrap(),
    );
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
    assert_eq!(hop.takeoff(), Some(tile(3201, 3201, 0)));
    assert_eq!(hop.effective_takeoff(), tile(3201, 3201, 0));
    assert_eq!(hop.held(), &[(995, 10)]);
    assert_eq!(hop.consumed(), &[(995, 5)]);
    assert_eq!(hop.returned(), &[(995, 1)]);
    assert_eq!(hop.worn_any(), &[772]);
    assert_eq!(hop.worn_all(), &[1, 2]);
    assert_ne!(hop.held(), hop.consumed());
    assert_ne!(hop.worn_any(), hop.worn_all());
    assert!(hop.has_quest_gate());
    assert_eq!(view.leg(2).unwrap_err(), ContractError::NoSuchLeg);
    let debug = format!("{hop:?}");
    assert!(debug.contains("TransportView"));
    assert!(!debug.contains("quest_gates"));
}

fn zone_world() -> (NavLease, NavLease) {
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
        &WildernessRules::default(),
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
    let a = NavLease::wrap(Arc::new(world_a), digest(b"zones-a")).unwrap();
    let b = NavLease::wrap(Arc::new(world_b), digest(b"zones-b")).unwrap();
    (a, b)
}

#[test]
fn zone_exemptions_are_pack_bound_and_unknown_keys_refuse() {
    assert_eq!(MAX_ZONE_KEYS, 8);
    let (a, b) = zone_world();
    let zones = a.zones().unwrap();
    let bound = zones.resolve("white-wolf-mountain").unwrap();
    assert_eq!(bound.key(), ZoneKey::Group(0));
    assert!(zones.name(bound).unwrap().contains("white-wolf-mountain"));
    assert!(zones.bind(ZoneKey::Zone(99)).is_err());
    assert_eq!(
        b.zones().unwrap().zone(bound).unwrap_err(),
        ContractError::PackMismatch
    );

    let zone_key = zones.bind(ZoneKey::Zone(0)).unwrap();
    let exempt = zones.exempt(&[zone_key]).unwrap();
    assert_eq!(exempt.pack(), a.identity().pack());
    assert!(zones.exemption_contains(exempt, zone_key).unwrap());
    assert_eq!(
        b.zones().unwrap().exemption_contains(exempt, zone_key),
        Err(ContractError::PackMismatch)
    );
    let foreign_key = b.zones().unwrap().bind(ZoneKey::Zone(0)).unwrap();
    assert_eq!(
        zones.exemption_contains(exempt, foreign_key),
        Err(ContractError::PackMismatch)
    );
    assert_eq!(
        zones.exempt(&[foreign_key]).unwrap_err(),
        ContractError::PackMismatch
    );

    let nine: Vec<BoundZoneKey> = (0..9).map(|_| zone_key).collect();
    assert_eq!(
        zones.exempt(&nine).unwrap_err(),
        ContractError::TooManyExemptions
    );

    let view = zones.zone(zone_key).unwrap();
    assert_eq!(zones.zone(view.key()).unwrap().key(), view.key());
    assert_eq!(view.group().unwrap().key(), ZoneKey::Group(0));
    assert!(!view.has_shape());
    assert_eq!(view.class(), ZoneClass::Always);
    assert_eq!(view.cap(), None);
    assert!(view.contains(tile(5, 5, 0)));
    assert!(zones
        .containing(tile(5, 5, 0))
        .any(|z| z.key() == view.key()));
    let kind = zones.kind(zone_key).unwrap();
    assert_eq!(kind.id(), "wolf");
    assert_eq!(kind.npc_id(), Some(96));
    let _ = kind.hazard_damage();
}

#[test]
fn route_permissions_reject_a_foreign_exemption() {
    let (a, b) = zone_world();
    let foreign = b
        .zones()
        .unwrap()
        .exempt(&[b.zones().unwrap().bind(ZoneKey::Zone(0)).unwrap()])
        .unwrap();
    let permissions = RoutePermissions::new().with_zones(foreign);
    let req = RouteRequest::new(tile(1, 1, 0), tile(2, 2, 0)).with_permissions(permissions);
    assert_eq!(
        req.permissions().zones().unwrap().pack(),
        b.identity().pack()
    );
    assert_ne!(
        req.permissions().zones().unwrap().pack(),
        a.identity().pack()
    );
}

#[test]
fn wilderness_view_has_no_grid_and_reports_membership() {
    let graph = TransportGraph {
        wilderness: WildernessRules {
            zones: vec![WildernessZone {
                x1: 2944,
                z1: 3520,
                x2: 3391,
                z2: 3967,
                level1: 0,
                level2: 3,
                origin_z: 3520,
            }],
            divisor: 8,
            offset: 1,
        },
        ..TransportGraph::default()
    };
    let lease = NavLease::wrap(
        Arc::new(NavWorld::from_parts(
            grid(tile(3200, 3200, 0), 1, 1, vec![0]),
            graph,
            Vec::new(),
        )),
        digest(b"wild"),
    )
    .unwrap();
    let wild = lease.wilderness();
    let outside = tile(3200, 3200, 0);
    assert!(!wild.contains(outside));
    assert_eq!(wild.level(outside), 0);
    let inside = tile(3072, 3525, 0);
    assert!(wild.contains(inside));
    assert!(wild.level(inside) > 0);
}

#[test]
fn route_request_defaults_to_reach_without_wilderness() {
    let req = RouteRequest::new(tile(1, 1, 0), tile(2, 2, 0));
    assert_eq!(req.arrival(), ArrivalSpec::REACH);
    assert!(!req.permissions().allow_teleports());
    assert!(!req.permissions().allow_wilderness());
    assert_eq!(req.revision(), InputRevision::FIRST);
}

#[test]
fn route_service_binds_the_family_and_maps_current_router_errors() {
    assert_eq!(<RouteService as WorkFamilySpec>::FAMILY, WorkFamily::Route);
    assert_eq!(RouteService::producer(), Track::T5);
    assert_eq!(RouteService::consumers(), &[Track::T1, Track::T2]);
    assert!(matches!(
        RouteService::map_route_error(RouteError::NoPath),
        Ok(RouteFamilyResult::NoPath)
    ));
    assert!(matches!(
        RouteService::map_route_error(RouteError::BudgetExhausted),
        Err(RouteFamilyError::BudgetExhausted)
    ));
    assert!(matches!(
        RouteService::map_target_error(TargetError::NotSettled),
        Err(RouteFamilyError::NotSettled)
    ));

    let collision = grid(tile(3200, 3200, 0), 1, 1, vec![0]);
    let graph = TransportGraph::default();
    let from = tile(3200, 3200, 0);
    let far = tile(4000, 4000, 0);
    match crate::router::find(&collision, &graph, from, far) {
        Err(error) => match RouteService::map_route_error(error) {
            Ok(RouteFamilyResult::NoPath) => {}
            other => panic!("disconnected tiles must map to NoPath, got {other:?}"),
        },
        Ok(route) => panic!(
            "expected NoPath, got a route with {} legs",
            route.legs.len()
        ),
    }

    let adjacent = tile(3201, 3200, 0);
    let open = grid(tile(3200, 3200, 0), 2, 1, vec![0, 0]);
    match crate::router::find(&open, &graph, from, adjacent) {
        Ok(route) => {
            assert!(route.ticks > 0.0);
            let lease = lease_from(open);
            let result = RouteService::path(&lease, work_identity(), InputRevision::FIRST, route);
            assert!(matches!(result, RouteFamilyResult::Path(_)));
        }
        Err(error) => panic!("open adjacent tiles must route, got {error:?}"),
    }

    let req = RouteRequest::new(from, adjacent);
    assert_eq!(RouteService::revision(&req), InputRevision::FIRST);
    let charge = RouteService::charge(&req);
    assert!(charge.total_bytes() > charge.input_bytes() || charge.result_bytes() > 0);
}

#[test]
fn real_pack_views_borrow_the_lease_and_keep_catalog_ids() {
    let Some(world) = NavWorld::load_default_pack_or_skip() else {
        return;
    };
    let origin = world.collision.origin;
    let width = world.collision.width;
    let height = world.collision.height;
    let lease = NavLease::wrap(Arc::new(world), digest(b"real-pack")).unwrap();
    assert_eq!(lease.identity().format(), FORMAT_ID);
    assert_eq!(lease.identity().version(), VERSION);
    let view = lease.collision();
    let inside = tile(origin.x + 1, origin.z + 1, 0);
    assert!(
        view.contains_tile(inside) || view.contains_tile(origin),
        "the packed origin neighbourhood must be in bounds"
    );
    let mut found_face = None;
    'scan: for lz in 0..height {
        for lx in 0..width {
            let t = tile(origin.x + lx as i32, origin.z + lz as i32, 0);
            if view.standable(t) && !view.walkable(t) {
                found_face = Some(t);
                break 'scan;
            }
        }
    }
    let face = found_face
        .expect("expected a packed face-flag tile (standable and not walkable) on the real pack");
    assert!(view.standable(face));
    assert!(!view.walkable(face));

    let zones = lease.zones().expect("v16 packs carry a zone table");
    for id in AVOID_CATALOG_IDS {
        assert!(
            zones.resolve(id).is_some(),
            "catalog id {id} must resolve on the real pack"
        );
    }
    assert!(zones.bind(ZoneKey::Zone(u16::MAX)).is_err());

    let wild = lease.wilderness();
    let wild_tile = tile(3072, 3525, 0);
    assert!(
        wild.contains(wild_tile),
        "2944..=3391, 3520+ is packed wilderness"
    );
    assert!(
        wild.level(wild_tile) > 0,
        "wilderness level formula is not zero inside the zone"
    );
}
