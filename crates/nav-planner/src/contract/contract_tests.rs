//! Every §23 §1.1 promise is checked differentially: the facade and the
//! production function it freezes run over the same inputs (synthetic grids
//! and the real pack) and must agree. The production side is written here
//! from the production call sites, never from the facade.

use std::collections::{HashMap, HashSet};
use std::mem::size_of;
use std::num::NonZeroU16;
use std::sync::Arc;

use api::selected::{FactKey, QuestGate};
use api::snapshot::WorldTile;
use api::work::{InputRevision, Track, WorkFamily, WorkFamilySpec, WorkPhase};
use client::dash3d::CollisionFlag;

use crate::collision::{pack_walk, WorldCollision};
use crate::quest_gates::{QuestFamilyId, QuestGates};
use crate::router::{FindOptions, Leg, Route, RouteError, TargetError};
use crate::transport::{
    DoorDir, TransportEdge, TransportGraph, TransportKind, WildernessRules, WildernessZone,
};
use crate::world::NavWorld;
use crate::world_state::WorldState;
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
        forbidden_held: Vec::new(),
        forbidden_worn: Vec::new(),
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

/// Production step validity: `nav_planner::movement::step_ok` as Gatherer's retreat
/// flood calls it (`gatherer/runner.rs:185`), on the mover's own plane.
fn production_step(collision: &WorldCollision, from: WorldTile, d: (i32, i32)) -> bool {
    crate::movement::step_ok(collision, from, d)
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
        !crate::movement::step_ok(&collision, from, (1, 1)),
        "NE requires the northern neighbour's south face open"
    );
    let blocked = lease_from(collision);
    assert!(!blocked.collision().walkable(north));
    assert!(!blocked.collision().step_ok(from, diag));
}

#[test]
fn borrowed_route_view_reports_the_production_route() {
    let (collision, here) = reach_fixture();
    let graph = TransportGraph::default();
    let dest = tile(3205, 3203, 0);
    let route = crate::router::find(&collision, &graph, here, dest).expect("open path");
    let lease = lease_from(collision);
    let view = crate::producer::route_view(&lease, &route).unwrap();
    assert_eq!(view.pack(), lease.identity().pack());
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
    let walk = view.leg(0).unwrap().walk_tiles().unwrap();
    assert!(!walk.is_empty());
    assert!(walk.iter().all(|t| t.x != 3202));
    assert!(route.ticks > 0.0);
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
    let route = Route {
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
    };
    let view = crate::producer::route_view(&lease, &route).unwrap();
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
            let view = crate::producer::route_view(&lease, &route).expect("borrowed route view");
            assert_eq!(view.destination(), adjacent);
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

// --- Transport facts through the router (§23 §1.1 "Transport facts") ---

/// The production admission predicate every transport leg of a found route
/// satisfied: `WorldState::allows` (`world_state.rs:155`), the gate
/// `find_with` relaxes nothing against.
fn production_allows(state: &WorldState, edge: &TransportEdge) -> bool {
    state.allows(edge)
}

/// A 5×3 bake at (3200,3200) split by two walls (between x=3200|3201 and
/// x=3202|3203, the client's `W_E`/`W_W` pairs so no step or diagonal
/// crosses) with two directed doors on row z=3201 as the only crossings.
fn two_door_world(first: TransportEdge, second: TransportEdge) -> (WorldCollision, TransportGraph) {
    let w_e = CollisionFlag::W_E as u32;
    let w_w = CollisionFlag::W_W as u32;
    let mut level0 = vec![0u32; 5 * 3];
    for z in 0..3 {
        level0[z * 5] |= w_e;
        level0[z * 5 + 1] |= w_w;
        level0[z * 5 + 2] |= w_e;
        level0[z * 5 + 3] |= w_w;
    }
    let collision = grid(tile(3200, 3200, 0), 5, 3, level0);
    let mut graph = TransportGraph {
        edges: vec![first, second],
        ..TransportGraph::default()
    };
    graph.rebuild_index(&collision);
    (collision, graph)
}

/// Every walk/transport leg relation the facade promises, checked against
/// the route the router actually returned: the walk leg before a hop ends
/// on a tile the production footprint/takeoff predicate admits for that
/// packed edge (`TransportGraph::admissible_from`, `transport.rs:308`;
/// exact when the takeoff is pinned), the walk leg after it starts on its
/// landing, hop `n` satisfied the production admission predicate under
/// `states[n]` (the last state covers later hops), and the view's tick cost
/// is the walk half-ticks plus the hop ticks. Returns the hop count.
fn check_route_view_against_router(
    view: RouteView<'_>,
    route: &Route,
    collision: &WorldCollision,
    graph: &TransportGraph,
    states: &[&WorldState],
) -> usize {
    assert_eq!(view.destination(), route.dest);
    assert_eq!(view.ticks(), route.ticks);
    assert_eq!(view.leg_count(), route.legs.len());
    let mut hops = 0usize;
    let mut ticks = 0.0f64;
    for (index, leg) in route.legs.iter().enumerate() {
        let facade = view.leg(index).unwrap();
        match leg {
            Leg::Walk { tiles } => {
                assert_eq!(facade.walk_tiles().unwrap(), tiles.as_slice());
                ticks += 0.5 * (tiles.len() - 1) as f64;
            }
            Leg::Transport { edge } => {
                let state = states[hops.min(states.len() - 1)];
                hops += 1;
                let hop = facade.transport().unwrap();
                assert_eq!(hop.kind(), edge.kind);
                assert_eq!(hop.ticks(), edge.ticks);
                assert_eq!(hop.anchor(), edge.at);
                assert_eq!(hop.landing(), edge.to);
                assert_eq!(hop.takeoff(), edge.takeoff);
                assert_eq!(hop.effective_takeoff(), edge.takeoff.unwrap_or(edge.at));
                assert_eq!(hop.loc_id(), edge.loc_id);
                assert_eq!(hop.option(), edge.option);
                assert_eq!(hop.direction(), edge.dir);
                assert_eq!(hop.displacement(), edge.player_delta);
                assert_eq!(hop.held(), edge.item_req.as_slice());
                assert_eq!(hop.consumed(), edge.consumed_req.as_slice());
                assert_eq!(hop.returned(), edge.item_returns.as_slice());
                assert_eq!(hop.worn_any(), edge.worn_req.as_slice());
                assert_eq!(hop.worn_all(), edge.worn_all_req.as_slice());
                assert_eq!(hop.forbidden_held(), edge.forbidden_held.as_slice());
                assert_eq!(hop.forbidden_worn(), edge.forbidden_worn.as_slice());
                assert_eq!(hop.skill_requirements(), edge.skill_req.as_slice());
                assert_eq!(hop.members_required(), edge.members_req);
                assert_eq!(hop.wilderness_cap(), edge.wildy_cap);
                assert_eq!(hop.has_quest_gate(), edge.quest_gates.is_some());
                assert!(
                    production_allows(state, edge),
                    "the router only takes hops the state proves: {hop:?}"
                );
                ticks += f64::from(edge.ticks);
                let before = view.leg(index - 1).unwrap().walk_tiles().unwrap();
                let takeoff = *before.last().expect("a walk leg precedes every hop");
                assert!(collision.standable(takeoff));
                if graph.teleports.iter().any(|packed| packed == &**edge) {
                    assert_eq!(hop.anchor(), tile(0, 0, 0), "a teleport has no anchor");
                } else {
                    // A relative landing is rewritten onto the planned `to`;
                    // match the packed row on everything else.
                    let packed = graph
                        .edges
                        .iter()
                        .position(|packed| {
                            packed == &**edge
                                || (packed.player_delta.is_some()
                                    && TransportEdge {
                                        to: packed.to,
                                        ..(**edge).clone()
                                    } == *packed)
                        })
                        .expect("the hop is a packed edge");
                    assert!(
                        graph.admissible_from(collision, packed, takeoff),
                        "the walk ends on a stand the production takeoff predicate admits"
                    );
                    if let Some(pinned) = hop.takeoff() {
                        assert_eq!(takeoff, pinned, "a pinned takeoff is exact");
                    }
                }
                let after = view.leg(index + 1).unwrap().walk_tiles().unwrap();
                assert_eq!(after.first().copied(), Some(hop.landing()));
            }
        }
    }
    assert_eq!(
        view.ticks(),
        ticks,
        "facade ticks are the half-tick walk cost plus hop ticks"
    );
    hops
}

#[test]
fn transport_legs_report_the_router_edges_and_the_production_admission_facts() {
    // Door 1 needs a reusable held pair, spends coins and returns a token;
    // door 2 is passable only by holding that returned token.
    let mut first = door_edge(tile(3200, 3201, 0), tile(3201, 3201, 0));
    first.skill_req = vec![(16, 10)];
    first.item_req = vec![(1, 2)];
    first.consumed_req = vec![(995, 5)];
    first.item_returns = vec![(996, 1)];
    first.worn_req = vec![772, 773];
    first.worn_all_req = vec![10, 11];
    first.takeoff = Some(tile(3200, 3202, 0));
    let mut second = door_edge(tile(3202, 3201, 0), tile(3203, 3201, 0));
    second.skill_req.clear();
    second.item_req = vec![(996, 1)];
    second.consumed_req.clear();
    second.item_returns.clear();
    second.worn_req.clear();
    second.worn_all_req.clear();
    second.ticks = 3;
    let (collision, graph) = two_door_world(first.clone(), second.clone());
    let from = tile(3200, 3200, 0);
    let to = tile(3204, 3202, 0);

    // Nothing proven: the router fails closed and the family maps NoPath.
    let empty = WorldState::empty();
    assert!(!production_allows(&empty, &first));
    match crate::router::find(&collision, &graph, from, to) {
        Err(error) => assert!(matches!(
            RouteService::map_route_error(error),
            Ok(RouteFamilyResult::NoPath)
        )),
        Ok(route) => panic!("unproven doors must not route, got {route:?}"),
    }

    let rich = WorldState {
        inv: HashMap::from([(1, 2), (995, 5)]),
        worn: HashSet::from([772, 10, 11]),
        stats: HashMap::from([(16, 10)]),
        ..WorldState::empty()
    };
    assert!(production_allows(&rich, &first));
    assert!(
        !production_allows(&rich, &second),
        "the token is not carried; only door 1's return supplies it"
    );
    let route =
        crate::router::find_with(&collision, &graph, from, to, FindOptions::default(), &rich)
            .expect("both doors open with the proven facts");
    let lease = lease_from(collision);
    let view = crate::producer::route_view(&lease, &route).unwrap();
    let after_first = WorldState {
        inv: HashMap::from([(1, 2), (996, 1)]),
        ..rich.clone()
    };
    // Hop 1 is admitted under the proven facts; hop 2 under the balance
    // after hop 1's spend and return.
    let hops = {
        let mut count = 0;
        for (index, leg) in route.legs.iter().enumerate() {
            if let Leg::Transport { edge } = leg {
                let hop = view.leg(index).unwrap().transport().unwrap();
                let state = if count == 0 { &rich } else { &after_first };
                assert!(production_allows(state, edge), "hop {count} admitted");
                assert_eq!(hop.held(), edge.item_req.as_slice());
                count += 1;
            }
        }
        count
    };
    assert_eq!(hops, 2);
    assert_eq!(
        check_route_view_against_router(
            view,
            &route,
            &lease.world.collision,
            &graph,
            &[&rich, &after_first]
        ),
        2
    );

    // Real values: the pinned takeoff is where the first walk ends, the
    // held/consumed/returned/worn-any/worn-all facts are the five distinct
    // requirement lists, and the second door's held fact is the first's
    // returned token.
    let first_walk = view.leg(0).unwrap().walk_tiles().unwrap();
    assert_eq!(first_walk.first().copied(), Some(from));
    assert_eq!(first_walk.last().copied(), Some(tile(3200, 3202, 0)));
    let hop1 = view.leg(1).unwrap().transport().unwrap();
    assert_eq!(hop1.effective_takeoff(), tile(3200, 3202, 0));
    assert_eq!(hop1.held(), &[(1, 2)]);
    assert_eq!(hop1.consumed(), &[(995, 5)]);
    assert_eq!(hop1.returned(), &[(996, 1)]);
    assert_eq!(hop1.worn_any(), &[772, 773]);
    assert_eq!(hop1.worn_all(), &[10, 11]);
    assert_eq!(hop1.skill_requirements(), &[(16, 10)]);
    let hop2 = view.leg(3).unwrap().transport().unwrap();
    assert_eq!(hop2.held(), hop1.returned());
    assert_eq!(hop2.ticks(), 3);
    assert_eq!(hop2.takeoff(), None);
    assert_eq!(hop2.effective_takeoff(), hop2.anchor());
    assert_eq!(
        view.leg(4).unwrap().walk_tiles().unwrap().last().copied(),
        Some(to)
    );
    assert_eq!(view.ticks(), route.ticks);
    assert!(view.ticks() >= 5.0, "two hops (2 + 3 ticks) plus walking");

    // Each requirement fact independently gates admission, exactly as the
    // production predicate and the router decide it.
    let lacking: [(&str, WorldState); 6] = [
        (
            "held",
            WorldState {
                inv: HashMap::from([(995, 5)]),
                ..rich.clone()
            },
        ),
        (
            "consumed",
            WorldState {
                inv: HashMap::from([(1, 2), (995, 4)]),
                ..rich.clone()
            },
        ),
        (
            "worn-any",
            WorldState {
                worn: HashSet::from([10, 11]),
                ..rich.clone()
            },
        ),
        (
            "worn-all (one of two)",
            WorldState {
                worn: HashSet::from([772, 10]),
                ..rich.clone()
            },
        ),
        (
            "skill",
            WorldState {
                stats: HashMap::from([(16, 9)]),
                ..rich.clone()
            },
        ),
        ("returned token held by nobody", WorldState::empty()),
    ];
    for (name, state) in &lacking {
        assert!(
            !production_allows(state, &first) || !production_allows(state, &second),
            "{name}: some hop is refused by the production predicate"
        );
        assert_eq!(
            crate::router::find_with(
                &lease.world.collision,
                &graph,
                from,
                to,
                FindOptions::default(),
                state
            )
            .unwrap_err(),
            RouteError::NoPath,
            "{name}: the router fails closed"
        );
    }
    // Worn-any is any one alternative; worn-all is every listed id.
    let other_alternative = WorldState {
        worn: HashSet::from([773, 10, 11]),
        ..rich.clone()
    };
    assert!(production_allows(&other_alternative, &first));
    assert!(crate::router::find_with(
        &lease.world.collision,
        &graph,
        from,
        to,
        FindOptions::default(),
        &other_alternative
    )
    .is_ok());
    let carried_not_worn = WorldState {
        inv: HashMap::from([(1, 2), (995, 5), (10, 1), (11, 1)]),
        worn: HashSet::from([772]),
        ..rich.clone()
    };
    assert!(
        !production_allows(&carried_not_worn, &first),
        "worn-all cannot be met by carried items"
    );
    // The returned token is a router fact: without hop 1's return, hop 2
    // stays shut even though everything else is proven.
    let mut no_return = first;
    no_return.item_returns.clear();
    let (collision2, graph2) = two_door_world(no_return, second.clone());
    assert_eq!(
        crate::router::find_with(
            &collision2,
            &graph2,
            from,
            to,
            FindOptions::default(),
            &rich
        )
        .unwrap_err(),
        RouteError::NoPath
    );
}

#[test]
fn real_pack_transport_legs_match_the_router_and_admission_facts() {
    let Some(world) = NavWorld::load_default_pack_or_skip() else {
        return;
    };
    let lease = NavLease::wrap(Arc::new(world), digest(b"real-pack")).unwrap();
    let collision = &lease.world.collision;
    let graph = &lease.world.graph;

    // Lumbridge castle: the ground-floor staircase is the only way up, so a
    // route to the first floor carries a transport leg the router chose.
    let upstairs = tile(3205, 3209, 1);
    let empty = WorldState::empty();
    let route = crate::router::find(collision, graph, LUMBRIDGE, upstairs)
        .expect("the castle stairs route");
    let view = crate::producer::route_view(&lease, &route).unwrap();
    let hops = check_route_view_against_router(view, &route, collision, graph, &[&empty]);
    assert!(hops >= 1, "a plane change needs a transport leg");
    let stairs = view
        .legs()
        .filter_map(|leg| leg.transport())
        .find(|hop| hop.kind() == TransportKind::Stairs)
        .expect("the staircase hop");
    assert_eq!(stairs.anchor(), tile(3204, 3207, 0));
    assert_eq!(stairs.landing(), upstairs);
    assert_eq!(stairs.loc_id(), 1738);
    assert_eq!(stairs.ticks(), 1);
    assert_eq!(stairs.takeoff(), None);
    assert!(stairs.held().is_empty() && stairs.consumed().is_empty());
    assert!(stairs.worn_any().is_empty() && stairs.worn_all().is_empty());
    assert!(!stairs.has_quest_gate());
    let packed = graph
        .edges
        .iter()
        .find(|edge| edge.at == stairs.anchor() && edge.to == stairs.landing())
        .expect("the packed staircase edge");
    assert_eq!(packed.kind, TransportKind::Stairs);
    assert!(production_allows(&empty, packed));

    // A charged teleport: consumed and returned are the packed jewellery
    // charge variants, and the hop is admitted only while the charge is
    // carried.
    let edgeville = tile(3087, 3496, 0);
    let charged = WorldState {
        inv: HashMap::from([(1712, 1)]),
        ..WorldState::empty()
    };
    let teleports = FindOptions {
        allow_teleports: true,
        ..FindOptions::default()
    };
    let route =
        crate::router::find_with(collision, graph, LUMBRIDGE, edgeville, teleports, &charged)
            .expect("a glory teleport to Edgeville");
    let view = crate::producer::route_view(&lease, &route).unwrap();
    assert_eq!(
        check_route_view_against_router(view, &route, collision, graph, &[&charged]),
        1
    );
    let glory = view
        .legs()
        .find_map(|leg| leg.transport())
        .filter(|hop| hop.kind() == TransportKind::Teleport)
        .expect("the teleport leg");
    assert_eq!(glory.landing(), edgeville);
    assert_eq!(glory.consumed(), &[(1712, 1)]);
    assert_eq!(glory.returned(), &[(1710, 1)]);
    assert!(glory.held().is_empty());
    let packed = graph
        .teleports
        .iter()
        .find(|edge| edge.to == edgeville && edge.consumed_req == [(1712, 1)])
        .expect("the packed glory(4) teleport");
    assert_eq!(glory.ticks(), packed.ticks);
    assert_eq!(glory.returned(), packed.item_returns.as_slice());
    assert!(production_allows(&charged, packed));
    assert!(!production_allows(&empty, packed));
    let uncharged =
        crate::router::find_with(collision, graph, LUMBRIDGE, edgeville, teleports, &empty);
    assert!(
        uncharged.map_or(true, |route| route.legs.iter().all(
            |leg| !matches!(leg, Leg::Transport { edge } if edge.kind == TransportKind::Teleport)
        )),
        "without the charge the router never takes the teleport"
    );

    // Every packed hop: the facade's requirement facts are exactly what the
    // production predicate gates on. A state proving everything the hop
    // lists is admitted unless a quest-stage gate is present (no evidence
    // here); dropping any one listed category refuses it.
    let mut positive = [0usize; 6];
    let mut checked = 0usize;
    for edge in graph.edges.iter().chain(graph.teleports.iter()) {
        let hop = TransportView::from_edge(edge);
        let mut inv: HashMap<i32, i32> = HashMap::new();
        for &(id, count) in hop.held() {
            let entry = inv.entry(id).or_default();
            *entry = (*entry).max(count);
        }
        for &(id, packed) in hop.consumed() {
            let entry = inv.entry(id).or_default();
            *entry = (*entry).max(packed.max(200));
        }
        let mut worn: HashSet<i32> = hop.worn_all().iter().copied().collect();
        if let Some(&any) = hop.worn_any().first() {
            worn.insert(any);
        }
        let proving = WorldState {
            inv,
            worn,
            inventory_ready: true,
            equipment_ready: true,
            stats: hop.skill_requirements().iter().copied().collect(),
            varps: edge.varp_req.iter().copied().collect(),
            quests: edge.quest_req.iter().cloned().collect(),
            map_members: hop.members_required(),
            ..WorldState::empty()
        };
        assert_eq!(
            production_allows(&proving, edge),
            !hop.has_quest_gate(),
            "proving every listed fact admits the hop unless it is quest-gated: {hop:?}"
        );
        if hop.has_quest_gate() {
            continue;
        }
        checked += 1;
        let without: [(usize, WorldState); 6] = [
            (
                0,
                WorldState {
                    inv: proving
                        .inv
                        .iter()
                        .filter(|(id, _)| !hop.held().iter().any(|(held, _)| held == *id))
                        .map(|(&id, &count)| (id, count))
                        .collect(),
                    ..proving.clone()
                },
            ),
            (
                1,
                WorldState {
                    inv: proving
                        .inv
                        .iter()
                        .filter(|(id, _)| !hop.consumed().iter().any(|(used, _)| used == *id))
                        .map(|(&id, &count)| (id, count))
                        .collect(),
                    ..proving.clone()
                },
            ),
            (
                2,
                WorldState {
                    worn: hop.worn_all().iter().copied().collect(),
                    ..proving.clone()
                },
            ),
            (
                3,
                WorldState {
                    worn: hop.worn_any().first().copied().into_iter().collect(),
                    ..proving.clone()
                },
            ),
            (
                4,
                WorldState {
                    stats: HashMap::new(),
                    ..proving.clone()
                },
            ),
            (
                5,
                WorldState {
                    map_members: false,
                    ..proving.clone()
                },
            ),
        ];
        let present = [
            !hop.held().is_empty(),
            !hop.consumed().is_empty(),
            !hop.worn_any().is_empty(),
            !hop.worn_all().is_empty(),
            !hop.skill_requirements().is_empty(),
            hop.members_required(),
        ];
        for (category, state) in &without {
            // Dropping a category that shares an obj id with another
            // category may refuse through that other category; the
            // assertion is one-directional there.
            let shares = match category {
                0 => hop
                    .held()
                    .iter()
                    .any(|(id, _)| hop.consumed().iter().any(|(used, _)| used == id)),
                1 => hop
                    .consumed()
                    .iter()
                    .any(|(id, _)| hop.held().iter().any(|(held, _)| held == id)),
                2 | 3 => hop
                    .worn_any()
                    .first()
                    .is_some_and(|any| hop.worn_all().contains(any)),
                _ => false,
            };
            let admitted = production_allows(state, edge);
            if present[*category] {
                assert!(!admitted, "dropping a listed fact refuses the hop: {hop:?}");
                positive[*category] += 1;
            } else if !shares {
                assert!(admitted, "an unlisted fact does not gate the hop: {hop:?}");
            }
        }
    }
    assert!(checked > 1000, "the real pack carries thousands of hops");
    assert!(
        positive.iter().all(|&count| count > 0),
        "held/consumed/worn-any/worn-all/skill/members each gate real hops: {positive:?}"
    );
}

// --- Zone class, membership, kind/hazard/shape facts (§23 §1.1 "Zones") ---

/// Kinds 0: wolf (NPC 96); 1: the curated lava bridge hazard. Zones 0: an
/// unshaped wolf zone with a carved-out corner; 1: a shaped wolf zone whose
/// 3×3 mask drops every corner; 2: the hazard rectangle; 3: a level-rule
/// wolf zone with a combat cap, on plane 1.
fn shaped_zone_table() -> ZoneTable {
    let wolf = ZoneKind::new("wolf", "Wolf", 96, 32, true, false);
    let lava = ZoneKind::new(
        "ikov-lava-bridge",
        "Temple of Ikov lava bridge",
        -1,
        0,
        false,
        true,
    );
    let plain = Zone::npc(tile(2, 2, 0), 1, ZoneClass::Always, u16::MAX, 0);
    let shaped = Zone::shaped_npc(tile(8, 2, 0), 1, 1, ZoneClass::Always, u16::MAX, 0, 0);
    let hazard = Zone::hazard(
        AvoidRect {
            min_x: 2,
            max_x: 4,
            min_z: 8,
            max_z: 9,
            level: Some(0),
        },
        0,
        1,
    );
    let capped = Zone::npc(tile(8, 8, 1), 1, ZoneClass::LevelRule, 64, 0);
    ZoneTable::from_parts(
        vec![plain, shaped, hazard, capped],
        vec![wolf, lava],
        Vec::new(),
        vec![(
            0,
            AvoidRect {
                min_x: 1,
                max_x: 1,
                min_z: 3,
                max_z: 3,
                level: Some(0),
            },
        )],
        // Row-major over the 3×3 bounding rect (min_x..=max_x, min_z..=max_z):
        // every corner is masked out.
        vec![0b010_111_010],
        tile(0, 0, 0),
        12,
        12,
        &WildernessRules::default(),
    )
    .unwrap()
}

/// Membership of every tile of `index`'s bounding rect, one tile beyond it,
/// on its own plane and the plane above, compared against the production
/// membership `ZoneTable::at` (`zones.rs:474`). Returns how many tiles of
/// the rect are members.
fn check_membership_against_table(view: ZoneView<'_>, table: &ZoneTable, index: u16) -> usize {
    let (min_x, min_z, max_x, max_z) = view.bounds();
    let plane = i32::from(view.plane());
    let mut members = 0usize;
    for level in [plane, plane + 1] {
        for x in min_x - 1..=max_x + 1 {
            for z in min_z - 1..=max_z + 1 {
                let t = tile(x, z, level);
                let production = table.at(t).any(|i| i == index);
                assert_eq!(view.contains(t), production, "zone {index} contains {t:?}");
                if production && level == plane {
                    members += 1;
                }
                if level != plane || x < min_x || x > max_x || z < min_z || z > max_z {
                    assert!(
                        !production,
                        "outside the plane-aware rect is never a member"
                    );
                }
            }
        }
    }
    members
}

#[test]
fn zone_shape_carve_class_and_hazard_facts_match_the_table() {
    let table = shaped_zone_table();
    let mut world = NavWorld::from_parts(
        grid(tile(0, 0, 0), 1, 1, vec![0]),
        TransportGraph::default(),
        Vec::new(),
    );
    world.graph.zones = Some(table);
    let lease = NavLease::wrap(Arc::new(world), digest(b"shaped")).unwrap();
    let zones = lease.zones().unwrap();
    let table = lease.world.graph.zones.as_ref().unwrap();

    let plain = zones.zone(zones.bind(ZoneKey::Zone(0)).unwrap()).unwrap();
    let shaped = zones.zone(zones.bind(ZoneKey::Zone(1)).unwrap()).unwrap();
    let hazard = zones.zone(zones.bind(ZoneKey::Zone(2)).unwrap()).unwrap();
    let capped = zones.zone(zones.bind(ZoneKey::Zone(3)).unwrap()).unwrap();

    // Shape presence is a real fact: the masked zone has one, the others
    // do not.
    assert!(shaped.has_shape());
    assert!(!plain.has_shape());
    assert!(!hazard.has_shape());
    assert!(!capped.has_shape());

    // Membership against the production table: 9 − 4 corners for the
    // shaped zone; 9 − 1 carved tile for the plain zone; the full 3×2
    // hazard rect; the capped zone on plane 1 only.
    assert_eq!(check_membership_against_table(shaped, table, 1), 5);
    assert_eq!(check_membership_against_table(plain, table, 0), 8);
    assert_eq!(check_membership_against_table(hazard, table, 2), 6);
    assert_eq!(check_membership_against_table(capped, table, 3), 9);
    assert!(
        shaped.contains(tile(8, 2, 0)),
        "the shaped spawn is a member"
    );
    assert!(!shaped.contains(tile(7, 1, 0)), "a masked corner is not");
    assert!(!shaped.contains(tile(9, 3, 0)));
    assert!(
        plain.contains(tile(1, 1, 0)),
        "an uncarved corner is a member"
    );
    assert!(!plain.contains(tile(1, 3, 0)), "the carved tile is not");
    assert!(!plain.contains(tile(2, 2, 1)), "membership is plane-aware");
    assert!(capped.contains(tile(8, 8, 1)));
    assert!(!capped.contains(tile(8, 8, 0)));
    assert_eq!(
        zones.containing(tile(1, 3, 0)).count(),
        0,
        "a carved tile belongs to no zone"
    );
    assert_eq!(
        zones
            .containing(tile(3, 8, 0))
            .map(|z| z.key().key())
            .collect::<Vec<_>>(),
        vec![ZoneKey::Zone(2)]
    );

    // Class and cap: level-rule zones expose their combat cap, always-on
    // zones none.
    assert_eq!(capped.class(), ZoneClass::LevelRule);
    assert_eq!(capped.cap(), Some(64));
    assert_eq!(capped.plane(), 1);
    assert_eq!(plain.class(), ZoneClass::Always);
    assert_eq!(plain.cap(), None);
    assert_eq!(hazard.class(), ZoneClass::Always);
    for (view, index) in [(plain, 0usize), (shaped, 1), (hazard, 2), (capped, 3)] {
        let row = &table.zones()[index];
        assert_eq!(view.class(), row.class);
        assert_eq!(
            view.cap(),
            (row.class == ZoneClass::LevelRule).then_some(row.cap)
        );
        assert_eq!(view.plane(), row.level);
        assert_eq!(view.spawn(), (row.spawn_x, row.spawn_z));
        assert_eq!(view.bounds(), (row.min_x, row.min_z, row.max_x, row.max_z));
        assert_eq!(view.group().is_none(), row.group == crate::zones::NO_GROUP);
        let kind = &table.kinds()[usize::from(row.kind)];
        assert_eq!(view.kind().id(), kind.id.as_ref());
        assert_eq!(view.kind().label(), kind.label.as_ref());
        assert_eq!(
            view.kind().npc_id(),
            (kind.npc_id >= 0).then_some(kind.npc_id)
        );
        assert_eq!(view.kind().vislevel(), kind.vislevel);
        assert_eq!(view.kind().aggressive(), kind.ap);
        assert_eq!(view.kind().vis_off(), kind.vis_off);
        assert_eq!(
            view.kind().hazard_damage(),
            crate::zones::curated::hazard_damage(kind.id.as_ref())
        );
    }

    // Kind and hazard facts with real values: the wolf is an aggressive NPC
    // with no curated damage; the lava bridge is a non-NPC hazard whose
    // curated damage is 20 (`zones/curated.rs:60-72`).
    let wolf = plain.kind();
    assert_eq!(wolf.npc_id(), Some(96));
    assert_eq!(wolf.vislevel(), 32);
    assert!(wolf.aggressive());
    assert!(!wolf.vis_off());
    assert_eq!(wolf.hazard_damage(), None);
    let lava = hazard.kind();
    assert_eq!(lava.id(), "ikov-lava-bridge");
    assert_eq!(lava.npc_id(), None);
    assert!(lava.vis_off());
    assert_eq!(lava.hazard_damage(), Some(20));
    assert_eq!(
        zones
            .kind(zones.resolve("ikov-lava-bridge").unwrap())
            .unwrap()
            .hazard_damage(),
        Some(20),
        "the hazard resolves by its curated id"
    );
    assert_eq!(zones.resolve("ikov-lava-bridge"), Some(hazard.key()));
    assert_eq!(zones.resolve("not-a-zone"), None);
}

#[test]
fn real_pack_shape_carve_hazard_and_class_facts_match_the_table() {
    let Some(world) = NavWorld::load_default_pack_or_skip() else {
        return;
    };
    let lease = NavLease::wrap(Arc::new(world), digest(b"real-pack")).unwrap();
    let zones = lease.zones().expect("v16 packs carry a zone table");
    let table = lease.world.graph.zones.as_ref().unwrap();

    // Every shaped zone: shape presence is positive and its membership
    // matrix is the production table's, with at least one in-rect tile
    // masked out.
    let shaped: Vec<u16> = (0..table.zones().len())
        .map(|index| index as u16)
        .filter(|&index| table.zones()[usize::from(index)].shape != crate::zones::NO_SHAPE)
        .collect();
    assert!(!shaped.is_empty(), "the real pack carries shaped zones");
    for &index in &shaped {
        let view = zones
            .zone(zones.bind(ZoneKey::Zone(index)).unwrap())
            .unwrap();
        assert!(view.has_shape(), "zone {index} is shaped");
        let (min_x, min_z, max_x, max_z) = view.bounds();
        let rect = ((max_x - min_x + 1) * (max_z - min_z + 1)) as usize;
        let members = check_membership_against_table(view, table, index);
        assert!(
            members > 0 && members < rect,
            "zone {index}: {members} of {rect} rect tiles"
        );
    }
    // Unshaped zones report no shape, and a carved zone excludes its carve.
    let unshaped = (0..table.zones().len() as u16)
        .find(|&index| table.zones()[usize::from(index)].shape == crate::zones::NO_SHAPE)
        .expect("an unshaped zone");
    assert!(!zones
        .zone(zones.bind(ZoneKey::Zone(unshaped)).unwrap())
        .unwrap()
        .has_shape());
    let mut carved_checked = 0usize;
    for &(index, rect) in table.carves().iter().take(64) {
        let view = zones
            .zone(zones.bind(ZoneKey::Zone(index)).unwrap())
            .unwrap();
        let (min_x, min_z, max_x, max_z) = view.bounds();
        let level = rect.level.unwrap_or(i32::from(view.plane()));
        for x in rect.min_x.max(min_x)..=rect.max_x.min(max_x) {
            for z in rect.min_z.max(min_z)..=rect.max_z.min(max_z) {
                let t = tile(x, z, level);
                assert!(!view.contains(t), "carved tile {t:?} of zone {index}");
                assert!(!table.at(t).any(|i| i == index));
                carved_checked += 1;
            }
        }
    }
    assert!(carved_checked > 0, "real carves cover real tiles");

    // Hazard: the curated lava bridge is packed, resolves by id, and its
    // damage is the curated 20. Every kind agrees with the curated table.
    let lava = zones
        .resolve("ikov-lava-bridge")
        .expect("the packed hazard");
    let lava_view = zones.zone(lava).unwrap();
    assert_eq!(lava_view.kind().id(), "ikov-lava-bridge");
    assert_eq!(lava_view.kind().npc_id(), None);
    assert_eq!(lava_view.kind().hazard_damage(), Some(20));
    assert_eq!(lava_view.bounds(), (2648, 9828, 2650, 9829));
    assert!(lava_view.contains(tile(2649, 9828, 0)));
    assert!(!lava_view.contains(tile(2649, 9828, 1)));
    let mut hazards = 0usize;
    for index in 0..table.zones().len() as u16 {
        let view = zones
            .zone(zones.bind(ZoneKey::Zone(index)).unwrap())
            .unwrap();
        let row = &table.zones()[usize::from(index)];
        let kind = &table.kinds()[usize::from(row.kind)];
        let damage = crate::zones::curated::hazard_damage(kind.id.as_ref());
        assert_eq!(view.kind().hazard_damage(), damage);
        assert_eq!(
            view.kind().npc_id(),
            (kind.npc_id >= 0).then_some(kind.npc_id)
        );
        assert_eq!(view.class(), row.class);
        assert_eq!(
            view.cap(),
            (row.class == ZoneClass::LevelRule).then_some(row.cap)
        );
        hazards += usize::from(damage.is_some());
    }
    assert!(hazards > 0);
    // A real level-rule zone exposes its packed cap (twice the kind's
    // visibility level).
    let (index, row) = table
        .zones()
        .iter()
        .enumerate()
        .find(|(_, row)| row.class == ZoneClass::LevelRule)
        .expect("a level-rule zone");
    let view = zones
        .zone(zones.bind(ZoneKey::Zone(index as u16)).unwrap())
        .unwrap();
    assert_eq!(view.cap(), Some(row.cap));
    assert_eq!(row.cap, 2 * view.kind().vislevel());
}
