//! Every §23 §1.1 promise is checked differentially: the facade and the
//! production function it freezes run over the same inputs (synthetic grids
//! and the real pack) and must agree. The production side is written here
//! from the production call sites, never from the facade.

use std::mem::size_of;
use std::num::NonZeroU16;
use std::sync::Arc;

use api::identity::{ProcessIncarnation, RequestIdMinter, Scope, WorkIdentity};
use api::selected::{FactKey, QuestGate};
use api::snapshot::WorldTile;
use api::work::{InputRevision, Track, WorkFamily, WorkFamilySpec, WorkPhase};
use client::dash3d::CollisionFlag;

use crate::collision::{pack_walk, WorldCollision};
use crate::quest_gates::{QuestFamilyId, QuestGates};
use crate::router::{Leg, Route, RouteError, TargetError};
use crate::transport::{
    DoorDir, TransportEdge, TransportGraph, TransportKind, WildernessRules, WildernessZone,
};
use crate::world::NavWorld;
use crate::zones::{
    Zone, ZoneClass, ZoneExempt, ZoneFilter, ZoneGroup, ZoneKey, ZoneKind, ZoneTable,
};

use super::*;

const DIRS: [(i32, i32); 8] = [
    (0, 1),
    (0, -1),
    (1, 0),
    (-1, 0),
    (1, 1),
    (1, -1),
    (-1, 1),
    (-1, -1),
];

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

/// The host's packed collision surface for arrival probes, written from
/// `host-play/src/script_nav.rs:1913-1916`, not from the facade.
fn host_collision_at(collision: &WorldCollision) -> impl Fn(WorldTile) -> Option<i32> + '_ {
    move |tile: WorldTile| {
        let x = usize::try_from(tile.x.checked_sub(collision.origin.x)?).ok()?;
        let z = usize::try_from(tile.z.checked_sub(collision.origin.z)?).ok()?;
        ((0..=3).contains(&tile.level) && x < collision.width && z < collision.height)
            .then(|| collision.walkable_word(tile.x, tile.z, tile.level) as i32)
    }
}

/// Production packed Reach: `api::query::is_arrived` over the host's
/// collision surface, and the host's actual entry point
/// `retain_arrival_candidates` (`script_nav.rs:1912-1917`) agreeing with it.
fn production_reach(collision: &WorldCollision, here: WorldTile, dest: WorldTile, r: i32) -> bool {
    let collision_at = host_collision_at(collision);
    let probe = api::query::CollisionArrivalProbe::new(&collision_at);
    let arrived = api::query::is_arrived(here, dest, r, || &probe);
    let mut candidates = vec![here];
    api::query::retain_arrival_candidates(&mut candidates, dest, r, &collision_at);
    assert_eq!(
        candidates.len() == 1,
        arrived,
        "is_arrived and retain_arrival_candidates disagree for {here:?} -> {dest:?} r={r}"
    );
    arrived
}

/// Production packed Area goal: `gatherer/select.rs:1085-1090`.
fn production_area_goal(collision: &WorldCollision, centre: WorldTile, r: i32) -> bool {
    crate::arrival::area_has_standable_goal(centre, r, |t| collision.standable(t))
}

/// Production Area arrival rule (`native/walk.rs:50-56`) over the bake's
/// standability, the surface Gatherer uses for packed goals.
fn production_area_arrival(
    collision: &WorldCollision,
    here: WorldTile,
    centre: WorldTile,
    r: i32,
) -> bool {
    crate::arrival::arrived_in_area(here, centre, r, |t| collision.standable(t))
}

/// Production step validity: `nav::router::step_ok` as Gatherer's retreat
/// flood calls it (`gatherer/runner.rs:185`), on the mover's own plane.
fn production_step(collision: &WorldCollision, from: WorldTile, d: (i32, i32)) -> bool {
    crate::router::step_ok(collision, from, d)
}

/// Every tile of the `half`-radius window around `centre` on `centre`'s
/// plane, including tiles outside the bake.
fn window(centre: WorldTile, half: i32) -> impl Iterator<Item = WorldTile> {
    (-half..=half).flat_map(move |dx| {
        (-half..=half).map(move |dz| tile(centre.x + dx, centre.z + dz, centre.level))
    })
}

/// The Reach/Area/step fixture: a 7×5 bake at (3200,3200) with the mover at
/// (3204,3202). Column x=3202 is solid, so x<=3201 is walkable but cut off.
///
/// ```text
/// z=3204  . S . . X . .      S solid (WALK_SCENERY)
/// z=3203  . . S . . F .      F face flag (W_S): standable, not walkable
/// z=3202  . . S . H . S      H here
/// z=3201  . . S . . . .      . open
/// z=3200  . . S . . . .
///         x=3200 ... 3206
/// ```
fn reach_fixture() -> (WorldCollision, WorldTile) {
    let solid = CollisionFlag::WALK_SCENERY as u32;
    let face = CollisionFlag::W_S as u32;
    let mut level0 = vec![0u32; 7 * 5];
    let set = |cells: &mut Vec<u32>, x: i32, z: i32, flag: u32| {
        cells[(z - 3200) as usize * 7 + (x - 3200) as usize] |= flag;
    };
    for z in 3200..=3204 {
        set(&mut level0, 3202, z, solid);
    }
    set(&mut level0, 3201, 3204, solid);
    set(&mut level0, 3204, 3204, solid);
    set(&mut level0, 3206, 3202, solid);
    set(&mut level0, 3205, 3203, face);
    (grid(tile(3200, 3200, 0), 7, 5, level0), tile(3204, 3202, 0))
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
fn standable_and_walkable_match_the_production_predicates() {
    let (collision, here) = reach_fixture();
    let lease = lease_from(collision);
    let view = lease.collision();
    let collision = &lease.world.collision;
    for level in 0..=4 {
        for t in window(tile(here.x, here.z, level), 5) {
            assert_eq!(view.standable(t), collision.standable(t), "standable {t:?}");
            assert_eq!(view.walkable(t), collision.walkable(t), "walkable {t:?}");
            assert_eq!(
                view.contains_tile(t),
                (0..4).contains(&t.level)
                    && (3200..3207).contains(&t.x)
                    && (3200..3205).contains(&t.z),
                "contains {t:?}"
            );
        }
    }
    // Real values: a face flag is standable and not walkable; solid is
    // neither; level 4 is outside the bake.
    let face = tile(3205, 3203, 0);
    let solid = tile(3202, 3202, 0);
    assert!(view.standable(face) && !view.walkable(face));
    assert!(!view.standable(solid) && !view.walkable(solid));
    assert!(view.standable(here) && view.walkable(here));
    assert!(
        !view.contains_tile(tile(3200, 3200, 4)),
        "level 4 is outside the bake"
    );
    assert!(!view.contains_tile(tile(3199, 3200, 0)));
}

#[test]
fn reach_matches_production_is_arrived_over_a_grid() {
    let (collision, here) = reach_fixture();
    let lease = lease_from(collision);
    let view = lease.collision();
    let collision = &lease.world.collision;
    let mut compared = 0usize;
    for radius in 0..=3u16 {
        for level in [0, 1] {
            for dest in window(tile(here.x, here.z, level), 4) {
                let facade = view
                    .arrived(here, dest, ArrivalSpec::reach(radius))
                    .unwrap();
                let production = production_reach(collision, here, dest, i32::from(radius));
                assert_eq!(
                    facade, production,
                    "Reach r={radius} here={here:?} dest={dest:?}: facade {facade}, production {production}"
                );
                compared += 1;
            }
        }
    }
    assert_eq!(compared, 4 * 2 * 81);

    // Real values from the parity table (the review's probe is the first
    // two rows: a reachable walkable destination inside the radius arrives).
    let reach = |dest, r| view.arrived(here, dest, ArrivalSpec::reach(r)).unwrap();
    assert!(reach(here, 0), "standing on dest");
    assert!(reach(tile(3205, 3202, 0), 1), "walkable reachable, dist 1");
    assert!(reach(tile(3203, 3200, 0), 2), "walkable reachable, dist 2");
    assert!(!reach(tile(3203, 3200, 0), 1), "outside the radius");
    assert!(
        !reach(tile(3205, 3202, 0), 0),
        "radius 0 is on-destination only"
    );
    assert!(
        !reach(tile(3201, 3202, 0), 3),
        "walkable but cut off by the solid column: never arrives"
    );
    assert!(
        reach(tile(3206, 3202, 0), 2),
        "solid destination adjacent through an open edge arrives"
    );
    assert!(
        !reach(tile(3206, 3202, 0), 1),
        "the radius bounds the distance even to a solid destination"
    );
    assert!(
        reach(tile(3204, 3204, 0), 2),
        "solid, approached from the south"
    );
    assert!(
        !reach(tile(3201, 3204, 0), 3),
        "solid behind the wall: no reachable neighbour, never arrives"
    );
    assert!(
        !reach(tile(3205, 3203, 0), 1),
        "the face tile is entered only from the west; radius 1 permits (2*1+1)^2 = 9 \
         expansions, too few for the two-step detour (is_arrived: the radius bounds the probe budget)"
    );
    assert!(
        reach(tile(3205, 3203, 0), 2),
        "a face-flag tile is entered through its open face once the budget allows the detour"
    );
    assert!(
        !reach(tile(3204, 3202, 1), 3),
        "another plane never arrives"
    );
    assert!(
        !reach(tile(3205, 3202, 1), 1),
        "another plane never arrives"
    );
    assert!(
        reach(tile(3207, 3202, 0), 3),
        "outside the bake the destination cannot be probed: distance alone decides (is_arrived)"
    );
    assert!(!reach(tile(3207, 3202, 0), 2));
}

#[test]
fn reach_with_a_loc_is_refused_by_the_bake() {
    let (collision, here) = reach_fixture();
    let lease = lease_from(collision);
    let spec = ArrivalSpec::reach(1).with_loc(99).unwrap();
    assert_eq!(spec.loc_id(), Some(99));
    assert_eq!(
        lease.collision().arrived(here, tile(3205, 3202, 0), spec),
        Err(ContractError::LocArrivalNeedsScene)
    );
}

#[test]
fn area_matches_production_standable_goal_and_arrival_over_a_grid() {
    let (collision, here) = reach_fixture();
    let lease = lease_from(collision);
    let view = lease.collision();
    let collision = &lease.world.collision;
    for radius in 0..=3u16 {
        for level in [0, 1] {
            for centre in window(tile(here.x, here.z, level), 4) {
                assert_eq!(
                    view.area_has_goal(centre, radius),
                    production_area_goal(collision, centre, i32::from(radius)),
                    "area goal r={radius} centre={centre:?}"
                );
                for mover in window(here, 2) {
                    assert_eq!(
                        view.arrived(mover, centre, ArrivalSpec::area(radius))
                            .unwrap(),
                        production_area_arrival(collision, mover, centre, i32::from(radius)),
                        "area arrival r={radius} here={mover:?} centre={centre:?}"
                    );
                }
            }
        }
    }
    // Real values: Area is standability, not walkability, and has no
    // path-to-centre requirement.
    let solid = tile(3202, 3202, 0);
    let face = tile(3205, 3203, 0);
    let cut_off = tile(3201, 3202, 0);
    assert!(
        !view.area_has_goal(solid, 0),
        "a solid centre has no radius-0 goal"
    );
    assert!(
        view.area_has_goal(solid, 1),
        "its standable neighbours are goals"
    );
    assert!(view.arrived(face, face, ArrivalSpec::area(0)).unwrap());
    assert!(!view.arrived(solid, solid, ArrivalSpec::area(0)).unwrap());
    assert!(
        view.arrived(cut_off, here, ArrivalSpec::area(3)).unwrap(),
        "Area needs no path to the centre"
    );
    assert!(!view
        .arrived(tile(3204, 3202, 1), here, ArrivalSpec::area(4))
        .unwrap());
    assert!(
        view.arrived(here, tile(3207, 3202, 0), ArrivalSpec::area(3))
            .unwrap(),
        "Area arrival checks the mover's tile, not the centre's"
    );
}

#[test]
fn arrival_spec_matches_native_walk_cases() {
    assert_eq!(ArrivalSpec::REACH.radius(), 0);
    assert_eq!(ArrivalSpec::default(), ArrivalSpec::REACH);
    assert_eq!(ArrivalSpec::reach(7).radius(), 7);
    assert_eq!(ArrivalSpec::reach(7).kind(), ArrivalKind::Reach);
    assert_eq!(ArrivalSpec::area(3).kind(), ArrivalKind::Area);
    assert_eq!(ArrivalSpec::area(3).radius(), 3);
    assert_eq!(
        ArrivalSpec::area(1).with_loc(42).unwrap_err(),
        ContractError::AreaWithLoc
    );
    let reach_loc = ArrivalSpec::reach(1).with_loc(99).unwrap();
    assert_eq!(reach_loc.loc_id(), Some(99));
    assert_eq!(reach_loc.radius(), 1);
}

#[test]
fn step_ok_matches_router_step_ok_on_the_plane_and_refuses_other_planes() {
    let (collision, here) = reach_fixture();
    let lease = lease_from(collision);
    let view = lease.collision();
    let collision = &lease.world.collision;
    let mut compared = 0usize;
    for from in window(here, 5) {
        for d in DIRS {
            let to = tile(from.x + d.0, from.z + d.1, from.level);
            assert_eq!(
                view.step_ok(from, to),
                production_step(collision, from, d),
                "step {from:?} by {d:?}"
            );
            // The router copies the mover's plane onto the neighbour; the
            // facade takes a destination tile and refuses a plane change.
            assert!(
                !view.step_ok(from, tile(to.x, to.z, 1)),
                "plane change {from:?} -> {to:?}"
            );
            compared += 1;
        }
        assert!(!view.step_ok(from, from), "d == (0,0) is not a step");
    }
    assert_eq!(compared, 121 * 8);
    // Real values.
    assert!(view.step_ok(here, tile(3205, 3202, 0)));
    assert!(
        !view.step_ok(tile(3203, 3202, 0), tile(3202, 3202, 0)),
        "into the solid column"
    );
    assert!(
        view.step_ok(tile(3205, 3202, 0), tile(3205, 3203, 0))
            == production_step(collision, tile(3205, 3202, 0), (0, 1))
    );
    assert!(
        !view.step_ok(tile(3205, 3202, 0), tile(3205, 3203, 0)),
        "W_S on the destination blocks a northward step"
    );
    assert!(
        view.step_ok(tile(3204, 3203, 0), tile(3205, 3203, 0)),
        "the face tile is entered from the west"
    );
    assert!(
        !view.step_ok(tile(3206, 3201, 0), tile(3207, 3202, 0)),
        "off the bake"
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
fn route_plan_lease_reports_the_production_route_and_binds_to_its_pack() {
    let (collision, here) = reach_fixture();
    let graph = TransportGraph::default();
    let dest = tile(3205, 3203, 0);
    let route = crate::router::find(&collision, &graph, here, dest).expect("open path");
    let lease = lease_from(collision);
    let other = lease_from(grid(tile(3200, 3200, 0), 1, 1, vec![0]));
    let identity = work_identity();
    let plan = lease.publish_route(identity, InputRevision::new(3), route.clone());
    assert_eq!(plan.pack(), lease.identity().pack());
    assert_eq!(plan.identity(), identity);
    assert_eq!(plan.revision(), InputRevision::new(3));
    let view = plan.view(&lease).unwrap();
    assert_eq!(view.destination(), route.dest);
    assert_eq!(view.ticks(), route.ticks);
    assert_eq!(view.leg_count(), route.legs.len());
    for (index, leg) in route.legs.iter().enumerate() {
        let facade = view.leg(index).unwrap();
        match leg {
            Leg::Walk { tiles } => assert_eq!(facade.walk_tiles().unwrap(), tiles.as_slice()),
            Leg::Transport { edge } => {
                assert_eq!(facade.transport().unwrap().anchor(), edge.at);
            }
        }
    }
    assert_eq!(
        view.leg(route.legs.len()).unwrap_err(),
        ContractError::NoSuchLeg
    );
    // Real values: a walk from here to the face tile ends on it and runs
    // through the open row, never the solid column.
    let walk = view.leg(0).unwrap().walk_tiles().unwrap();
    assert!(!walk.is_empty());
    assert!(walk.iter().all(|t| t.x != 3202));
    assert!(route.ticks > 0.0);
    assert_eq!(plan.view(&other).unwrap_err(), ContractError::PackMismatch);
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

/// Three zones: 0 and 1 are the two members of group 0, 2 is ungrouped.
fn zone_table() -> ZoneTable {
    let mut first = Zone::npc(tile(2, 2, 0), 1, ZoneClass::Always, u16::MAX, 0);
    first.group = 0;
    let mut second = Zone::npc(tile(8, 2, 0), 1, ZoneClass::Always, u16::MAX, 0);
    second.group = 0;
    let third = Zone::npc(tile(8, 8, 0), 1, ZoneClass::Always, u16::MAX, 0);
    ZoneTable::from_parts(
        vec![first, second, third],
        vec![ZoneKind::new("wolf", "Wolf", 96, 1, false, false)],
        vec![ZoneGroup::new(
            "white-wolf-mountain",
            "White Wolf Mountain",
            AvoidRect {
                min_x: 0,
                max_x: 10,
                min_z: 0,
                max_z: 4,
                level: None,
            },
            vec![0u16, 1u16],
        )],
        Vec::new(),
        Vec::new(),
        tile(0, 0, 0),
        12,
        12,
        &WildernessRules::default(),
    )
    .unwrap()
}

fn zone_world() -> (NavLease, NavLease) {
    let table = zone_table();
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

/// A tile inside zone `index` of `table`.
fn tile_in_zone(table: &ZoneTable, index: u16) -> WorldTile {
    let zone = &table.zones()[usize::from(index)];
    let t = tile(zone.spawn_x, zone.spawn_z, i32::from(zone.level));
    assert!(
        table.at(t).any(|i| i == index),
        "spawn tile is inside its zone"
    );
    t
}

/// Production whole-walk exemption: the zone is masked out of
/// `ZoneFilter::blocking_at` (`nav/src/zones.rs:756-812`), i.e. it no
/// longer blocks its own tile under the route predicate.
fn production_zone_exempt(table: &ZoneTable, exempt: &ZoneExempt, index: u16) -> bool {
    // Without a combat level the filter treats every zone as active on its
    // own tile (`combat 0 <= cap`), so only the whole-walk mask can remove
    // it from `blocking_at`.
    let wilderness = WildernessRules::default();
    let filter = ZoneFilter::new(table, None, &[], exempt);
    let t = tile_in_zone(table, index);
    let blocked = filter.blocking_at(&wilderness, t).any(|i| i == index);
    !blocked
}

#[test]
fn exemption_contains_matches_the_router_whole_walk_mask() {
    let (a, _) = zone_world();
    let zones = a.zones().unwrap();
    let table = a.world.graph.zones.as_ref().unwrap();
    let cases: [(&str, Vec<ZoneKey>); 6] = [
        ("none", vec![]),
        ("zone0", vec![ZoneKey::Zone(0)]),
        ("zone2", vec![ZoneKey::Zone(2)]),
        ("zone0+zone1", vec![ZoneKey::Zone(0), ZoneKey::Zone(1)]),
        ("group0", vec![ZoneKey::Group(0)]),
        ("group0+zone2", vec![ZoneKey::Group(0), ZoneKey::Zone(2)]),
    ];
    for (name, keys) in &cases {
        let bound: Vec<BoundZoneKey> = keys.iter().map(|&k| zones.bind(k).unwrap()).collect();
        let exempt = zones.exempt(&bound).unwrap();
        let raw = ZoneExempt::named(keys).unwrap();
        for index in 0..3u16 {
            assert_eq!(
                zones
                    .exemption_contains(exempt, zones.bind(ZoneKey::Zone(index)).unwrap())
                    .unwrap(),
                production_zone_exempt(table, &raw, index),
                "{name}: zone {index}"
            );
        }
        let members = &table.groups()[0].members;
        assert_eq!(
            zones
                .exemption_contains(exempt, zones.bind(ZoneKey::Group(0)).unwrap())
                .unwrap(),
            members
                .iter()
                .all(|&m| production_zone_exempt(table, &raw, m)),
            "{name}: group 0 is exempt only when every member is masked"
        );
    }
    let all = zones.exempt_all();
    assert!(all.is_all());
    for index in 0..3u16 {
        assert!(zones
            .exemption_contains(all, zones.bind(ZoneKey::Zone(index)).unwrap())
            .unwrap());
        assert!(production_zone_exempt(table, &ZoneExempt::all(), index));
    }
    assert!(zones
        .exemption_contains(all, zones.bind(ZoneKey::Group(0)).unwrap())
        .unwrap());

    // Real values, including the review's whole-group probe.
    let one_member = zones
        .exempt(&[zones.bind(ZoneKey::Zone(0)).unwrap()])
        .unwrap();
    let group = zones.bind(ZoneKey::Group(0)).unwrap();
    assert!(zones
        .exemption_contains(one_member, zones.bind(ZoneKey::Zone(0)).unwrap())
        .unwrap());
    assert!(!zones
        .exemption_contains(one_member, zones.bind(ZoneKey::Zone(1)).unwrap())
        .unwrap());
    assert!(
        !zones.exemption_contains(one_member, group).unwrap(),
        "one exempt member does not exempt the group"
    );
    let whole = zones.exempt(&[group]).unwrap();
    assert!(zones.exemption_contains(whole, group).unwrap());
    assert!(zones
        .exemption_contains(whole, zones.bind(ZoneKey::Zone(1)).unwrap())
        .unwrap());
    assert!(!zones
        .exemption_contains(whole, zones.bind(ZoneKey::Zone(2)).unwrap())
        .unwrap());
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
    let foreign_key = b.zones().unwrap().bind(ZoneKey::Zone(0)).unwrap();
    // Key side: pack-B key against view A.
    assert_eq!(
        zones.exemption_contains(exempt, foreign_key),
        Err(ContractError::PackMismatch)
    );
    // Exemption side alone: pack-A exemption with pack-B key against view B.
    assert_eq!(
        b.zones().unwrap().exemption_contains(exempt, foreign_key),
        Err(ContractError::PackMismatch),
        "the exemption's own pack is checked, not only the key's"
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
    let eight: Vec<BoundZoneKey> = (0..8).map(|_| zone_key).collect();
    assert!(zones.exempt(&eight).is_ok());

    let view = zones.zone(zone_key).unwrap();
    assert_eq!(zones.zone(view.key()).unwrap().key(), view.key());
    assert_eq!(view.group().unwrap().key(), ZoneKey::Group(0));
    assert!(!view.has_shape());
    assert_eq!(view.class(), ZoneClass::Always);
    assert_eq!(view.cap(), None);
    assert!(view.contains(tile(2, 2, 0)));
    assert!(zones
        .containing(tile(2, 2, 0))
        .any(|z| z.key() == view.key()));
    let kind = zones.kind(zone_key).unwrap();
    assert_eq!(kind.id(), "wolf");
    assert_eq!(kind.npc_id(), Some(96));
    assert_eq!(
        kind.hazard_damage(),
        None,
        "an npc kind carries no hazard damage"
    );
}

#[test]
fn route_request_binds_to_its_lease_and_rejects_a_foreign_exemption() {
    let (a, b) = zone_world();
    let foreign = b
        .zones()
        .unwrap()
        .exempt(&[b.zones().unwrap().bind(ZoneKey::Zone(0)).unwrap()])
        .unwrap();
    let permissions = RoutePermissions::new().with_zones(foreign);
    let req = RouteRequest::new(tile(1, 1, 0), tile(2, 2, 0))
        .with_permissions(permissions)
        .with_revision(InputRevision::new(4));
    assert_eq!(
        req.bind(&a).unwrap_err(),
        ContractError::PackMismatch,
        "pack B's exemption cannot be searched on lease A"
    );
    let bound = req.bind(&b).unwrap();
    assert_eq!(bound.pack(), b.identity().pack());
    assert_eq!(bound.request(), req);
    assert_eq!(RouteService::revision(&bound), InputRevision::new(4));
    assert!(RouteService::charge(&bound).input_bytes() >= size_of::<RouteRequest>());
    // No exemption binds to any lease.
    let plain = RouteRequest::new(tile(1, 1, 0), tile(2, 2, 0));
    assert_eq!(plain.bind(&a).unwrap().pack(), a.identity().pack());
    assert_eq!(plain.bind(&b).unwrap().pack(), b.identity().pack());
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
    let rules = &lease.world.graph.wilderness;
    for t in [
        tile(3200, 3200, 0),
        tile(3072, 3525, 0),
        tile(2944, 3520, 3),
        tile(3391, 3967, 0),
    ] {
        assert_eq!(wild.contains(t), rules.contains(t), "contains {t:?}");
        assert_eq!(wild.level(t), rules.level(t), "level {t:?}");
    }
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
    assert_eq!(RouteService::producer(), WorkFamily::Route.producer());
    assert_eq!(RouteService::consumers(), WorkFamily::Route.consumers());
    assert_eq!(RouteService::producer(), Track::T5);
    assert_eq!(RouteService::consumers(), &[Track::T1, Track::T2]);
    assert_eq!(WorkPhase::index(RoutePhase::Search), 0);
    assert_eq!(WorkPhase::index(RoutePhase::Commit), 1);
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

    let lease = lease_from(grid(tile(3200, 3200, 0), 2, 1, vec![0, 0]));
    let req = RouteRequest::new(from, adjacent).bind(&lease).unwrap();
    assert_eq!(RouteService::revision(&req), InputRevision::FIRST);
    let charge = RouteService::charge(&req);
    assert_eq!(charge.input_bytes(), size_of::<BoundRouteRequest>());
    assert_eq!(
        charge.result_bytes(),
        0,
        "the plan lease is charged in its owner ledger from publication"
    );
    assert!(WorkFamily::Route.charge_rule().contains("owner ledger"));
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

/// Lumbridge castle courtyard: open ground in every 289 bake.
const LUMBRIDGE: WorldTile = WorldTile {
    x: 3222,
    z: 3218,
    level: 0,
};

#[test]
fn real_pack_reach_area_and_step_match_production() {
    let Some(world) = NavWorld::load_default_pack_or_skip() else {
        return;
    };
    let lease = NavLease::wrap(Arc::new(world), digest(b"real-pack")).unwrap();
    let view = lease.collision();
    let collision = &lease.world.collision;
    assert!(
        view.walkable(LUMBRIDGE),
        "Lumbridge courtyard is walkable in the bake"
    );

    // Anchors found on the real pack: a walkable tile with a solid
    // neighbour, and a face-flag tile, near Lumbridge.
    let mut solid_edge = None;
    let mut face = None;
    for t in window(LUMBRIDGE, 24) {
        if view.walkable(t) {
            for d in DIRS {
                let n = tile(t.x + d.0, t.z + d.1, 0);
                if solid_edge.is_none() && !view.standable(n) && !view.step_ok(t, n) {
                    solid_edge = Some((t, n));
                }
            }
        }
        if face.is_none() && view.standable(t) && !view.walkable(t) {
            face = Some(t);
        }
    }
    let (stand, solid) = solid_edge.expect("a walkable tile beside a solid tile near Lumbridge");
    let face = face.expect("a face-flag tile near Lumbridge");

    let mut compared = 0usize;
    for here in [LUMBRIDGE, stand, face] {
        for radius in 0..=3u16 {
            for dest in window(here, 4).chain(window(tile(here.x, here.z, 1), 1)) {
                assert_eq!(
                    view.arrived(here, dest, ArrivalSpec::reach(radius))
                        .unwrap(),
                    production_reach(collision, here, dest, i32::from(radius)),
                    "real pack Reach r={radius} here={here:?} dest={dest:?}"
                );
                assert_eq!(
                    view.arrived(here, dest, ArrivalSpec::area(radius)).unwrap(),
                    production_area_arrival(collision, here, dest, i32::from(radius)),
                    "real pack Area arrival r={radius} here={here:?} dest={dest:?}"
                );
                assert_eq!(
                    view.area_has_goal(dest, radius),
                    production_area_goal(collision, dest, i32::from(radius)),
                    "real pack Area goal r={radius} centre={dest:?}"
                );
                compared += 1;
            }
        }
        for from in window(here, 4) {
            for d in DIRS {
                let to = tile(from.x + d.0, from.z + d.1, from.level);
                assert_eq!(
                    view.step_ok(from, to),
                    production_step(collision, from, d),
                    "real pack step {from:?} by {d:?}"
                );
            }
        }
    }
    assert_eq!(compared, 3 * 4 * (81 + 9));

    // Real values on the real pack.
    assert!(view
        .arrived(LUMBRIDGE, LUMBRIDGE, ArrivalSpec::REACH)
        .unwrap());
    assert!(
        view.arrived(stand, solid, ArrivalSpec::reach(1)).unwrap(),
        "a solid tile beside an open tile is reached by approach"
    );
    assert!(
        !view.arrived(stand, solid, ArrivalSpec::REACH).unwrap(),
        "radius 0 never arrives at a solid tile"
    );
    assert!(!view.step_ok(stand, solid));
    assert!(
        view.area_has_goal(solid, 1),
        "the standable neighbour is an Area goal"
    );
    assert!(!view.area_has_goal(solid, 0));
    assert!(view.arrived(face, face, ArrivalSpec::area(0)).unwrap());
    assert!(
        view.arrived(
            LUMBRIDGE,
            tile(LUMBRIDGE.x + 1, LUMBRIDGE.z, 0),
            ArrivalSpec::reach(1)
        )
        .unwrap()
            == production_reach(
                collision,
                LUMBRIDGE,
                tile(LUMBRIDGE.x + 1, LUMBRIDGE.z, 0),
                1
            )
    );
    assert!(!view
        .arrived(
            LUMBRIDGE,
            tile(LUMBRIDGE.x, LUMBRIDGE.z, 1),
            ArrivalSpec::reach(3)
        )
        .unwrap());
}

#[test]
fn real_pack_exemptions_match_the_router_mask_for_catalog_groups() {
    let Some(world) = NavWorld::load_default_pack_or_skip() else {
        return;
    };
    let lease = NavLease::wrap(Arc::new(world), digest(b"real-pack")).unwrap();
    let zones = lease.zones().expect("v16 packs carry a zone table");
    let table = lease.world.graph.zones.as_ref().unwrap();
    let mountain = zones.resolve("white-wolf-mountain").unwrap();
    let jail = zones.resolve("draynor-jail-guards").unwrap();
    let (ZoneKey::Group(mountain_group), ZoneKey::Group(jail_group)) = (mountain.key(), jail.key())
    else {
        panic!("catalog ids resolve to groups on the real pack");
    };
    assert_ne!(mountain_group, jail_group);
    let mountain_members = &table.groups()[usize::from(mountain_group)].members;
    let jail_members = &table.groups()[usize::from(jail_group)].members;
    assert!(
        mountain_members.len() > 1,
        "the mountain is more than one wolf"
    );

    // Exempting the mountain group covers every member and no jail guard,
    // exactly as the router masks it.
    let whole = zones.exempt(&[mountain]).unwrap();
    let raw_whole = ZoneExempt::named(&[mountain.key()]).unwrap();
    for &member in mountain_members.iter() {
        let key = zones.bind(ZoneKey::Zone(member)).unwrap();
        assert!(zones.exemption_contains(whole, key).unwrap());
        assert_eq!(
            zones.exemption_contains(whole, key).unwrap(),
            production_zone_exempt(table, &raw_whole, member)
        );
    }
    for &guard in jail_members.iter() {
        let key = zones.bind(ZoneKey::Zone(guard)).unwrap();
        assert!(!zones.exemption_contains(whole, key).unwrap());
        assert_eq!(
            zones.exemption_contains(whole, key).unwrap(),
            production_zone_exempt(table, &raw_whole, guard)
        );
    }
    assert!(zones.exemption_contains(whole, mountain).unwrap());
    assert!(!zones.exemption_contains(whole, jail).unwrap());

    // Exempting one wolf does not exempt the mountain.
    let first = mountain_members[0];
    let one = zones
        .exempt(&[zones.bind(ZoneKey::Zone(first)).unwrap()])
        .unwrap();
    let raw_one = ZoneExempt::named(&[ZoneKey::Zone(first)]).unwrap();
    assert!(production_zone_exempt(table, &raw_one, first));
    assert!(!production_zone_exempt(
        table,
        &raw_one,
        mountain_members[1]
    ));
    assert!(!zones.exemption_contains(one, mountain).unwrap());
    assert!(zones
        .exemption_contains(one, zones.bind(ZoneKey::Zone(first)).unwrap())
        .unwrap());
    assert!(!zones
        .exemption_contains(one, zones.bind(ZoneKey::Zone(mountain_members[1])).unwrap())
        .unwrap());
}
