//! Producer-owned differential for the packed Reach promise (contract §23
//! §1.1 "Coordinates and arrival"): the host's actual candidate filter,
//! [`approach_tiles`] with `estimate = false` — the production collision
//! callback at `script_nav.rs:1912-1917` — against the nav facade's
//! `CollisionView::arrived`, over the same world. nav cannot observe this
//! producer (D11: `host-play → nav`, never the reverse), so the comparison
//! lives here. The facade side never calls the producer and the producer
//! side never calls the facade.

use std::future::Future;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};

use api::snapshot::WorldTile;
use client::dash3d::CollisionFlag;
use nav::collision::{pack_walk, WorldCollision};
use nav::contract::{ArrivalSpec, NavLease, PackDigest};
use nav::transport::TransportGraph;
use nav::world::NavWorld;

use super::{
    approach_tiles, arrival_filter_needed, capture_loc_geometry, route_arrival_filter_future,
    RouteTargetCursor, ScriptRouteRequest,
};

fn tile(x: i32, z: i32, level: i32) -> WorldTile {
    WorldTile { x, z, level }
}

/// Facade candidates for a packed Reach walk: every bake-standable tile of
/// the radius window that `CollisionView::arrived` accepts, in a fixed
/// order independent of the producer's sort.
fn facade_candidates(lease: &NavLease, to: WorldTile, radius: i32) -> Vec<WorldTile> {
    let view = lease.collision();
    let r = radius.clamp(0, 104);
    let spec = ArrivalSpec::reach(u16::try_from(r).unwrap());
    let mut tiles = Vec::new();
    for dx in -r..=r {
        for dz in -r..=r {
            let t = tile(to.x + dx, to.z + dz, to.level);
            if view.standable(t) && view.arrived(t, to, spec).unwrap() {
                tiles.push(t);
            }
        }
    }
    tiles.sort_by_key(|t| (t.x, t.z, t.level));
    tiles
}

/// The producer's candidates in the facade's order (membership, not the
/// producer's nearest-first ordering, is the shared promise).
fn producer_candidates(
    world: &NavWorld,
    from: WorldTile,
    to: WorldTile,
    radius: i32,
) -> Vec<WorldTile> {
    let mut tiles = approach_tiles(world, from, to, radius, false);
    tiles.sort_by_key(|t| (t.x, t.z, t.level));
    tiles
}

fn compare(world: &Arc<NavWorld>, lease: &NavLease, from: WorldTile, to: WorldTile, radius: i32) {
    let producer = producer_candidates(world, from, to, radius);
    let facade = facade_candidates(lease, to, radius);
    assert_eq!(
        producer, facade,
        "Reach candidates for {to:?} r={radius} from {from:?}: producer vs facade"
    );
    // The producer's estimate admits every standable window tile without
    // the arrival filter; the facade's standability is the same predicate.
    let mut estimate = approach_tiles(world, from, to, radius, true);
    estimate.sort_by_key(|t| (t.x, t.z, t.level));
    let view = lease.collision();
    let r = radius.clamp(0, 104);
    let mut standable = Vec::new();
    for dx in -r..=r {
        for dz in -r..=r {
            let t = tile(to.x + dx, to.z + dz, to.level);
            if view.standable(t) {
                standable.push(t);
            }
        }
    }
    standable.sort_by_key(|t| (t.x, t.z, t.level));
    assert_eq!(
        estimate, standable,
        "estimate candidates for {to:?} r={radius}"
    );
}

/// A 7×5 bake at (3200,3200) with an asymmetric solid column (x=3202, so
/// x<=3201 is open but cut off), two more solids, and a face-flag tile
/// (W_S at (3205,3203): standable, enterable only from the side).
fn asymmetric_world() -> Arc<NavWorld> {
    let solid = CollisionFlag::WALK_SCENERY as u32;
    let face = CollisionFlag::W_S as u32;
    let mut level0 = vec![0u32; 7 * 5];
    let mut set = |x: i32, z: i32, flag: u32| {
        level0[(z - 3200) as usize * 7 + (x - 3200) as usize] |= flag;
    };
    for z in 3200..=3204 {
        set(3202, z, solid);
    }
    set(3201, 3204, solid);
    set(3204, 3204, solid);
    set(3206, 3202, solid);
    set(3205, 3203, face);
    let plane = 7 * 5;
    let mut flags = vec![0u32; 4 * plane];
    flags[..plane].copy_from_slice(&level0);
    let (walk, blocked) = pack_walk(&flags);
    Arc::new(NavWorld::from_parts(
        WorldCollision {
            origin: tile(3200, 3200, 0),
            width: 7,
            height: 5,
            walk,
            blocked,
            flags: Some(flags),
        },
        TransportGraph::default(),
        Vec::new(),
    ))
}

#[test]
fn approach_tiles_reach_candidates_match_the_facade_on_the_asymmetric_fixture() {
    let world = asymmetric_world();
    let lease = NavLease::wrap(Arc::clone(&world), PackDigest::of(b"asymmetric")).unwrap();
    let here = tile(3204, 3202, 0);
    let mut compared = 0usize;
    for radius in 0..=3 {
        for level in [0, 1] {
            for dx in -3..=3 {
                for dz in -3..=3 {
                    let to = tile(here.x + dx, here.z + dz, level);
                    compare(&world, &lease, here, to, radius);
                    compared += 1;
                }
            }
        }
    }
    assert_eq!(compared, 4 * 2 * 49);

    // Real values the comparison rests on.
    let view = lease.collision();
    // A destination right beside the wall at radius 2: the open tiles
    // behind the solid column are in the window but unreachable.
    let beside_wall = tile(3203, 3202, 0);
    let producer = producer_candidates(&world, here, beside_wall, 2);
    assert!(producer.contains(&beside_wall));
    assert!(producer.contains(&tile(3203, 3201, 0)));
    assert!(
        producer.iter().all(|t| t.x >= 3203),
        "cut-off tiles behind the solid column are never candidates: {producer:?}"
    );
    assert!(view.standable(tile(3201, 3202, 0)));
    assert!(!view
        .arrived(tile(3201, 3202, 0), beside_wall, ArrivalSpec::reach(2))
        .unwrap());
    // A solid destination at radius 1 is approached from its open sides
    // only; the solid itself is never a candidate.
    let solid = tile(3202, 3202, 0);
    let producer = producer_candidates(&world, here, solid, 1);
    assert!(!producer.contains(&solid));
    assert!(producer.contains(&tile(3203, 3202, 0)));
    assert!(producer.contains(&tile(3201, 3202, 0)));
    // The face tile is a candidate at radius 2 from its side, not through
    // its blocked south face at radius 1 from below.
    let face = tile(3205, 3203, 0);
    assert!(producer_candidates(&world, here, face, 0).contains(&face));
    let below = tile(3205, 3202, 0);
    let from_below = producer_candidates(&world, here, below, 1);
    assert!(from_below.contains(&below));
    assert_eq!(
        from_below.contains(&face),
        view.arrived(face, below, ArrivalSpec::reach(1)).unwrap()
    );
    // Candidates are plane-aware: a destination on the (open) plane 1 is
    // answered on plane 1 only, and plane 4 is outside the bake.
    let upstairs = producer_candidates(&world, here, tile(3204, 3202, 1), 1);
    assert_eq!(upstairs.len(), 9);
    assert!(upstairs.iter().all(|t| t.level == 1));
    assert!(producer_candidates(&world, here, tile(3204, 3202, 4), 3).is_empty());
}

#[test]
fn approach_tiles_reach_candidates_match_the_facade_on_the_real_pack() {
    let Some(path) = std::env::var_os("NAV_PACK") else {
        eprintln!("SKIP: NAV_PACK is not set");
        return;
    };
    let world = Arc::new(NavWorld::load_pack(std::path::Path::new(&path)).expect("real nav pack"));
    let lease = NavLease::wrap(Arc::clone(&world), PackDigest::of(b"real-pack")).unwrap();
    let view = lease.collision();
    // Lumbridge castle courtyard, plus a walkable tile beside a solid and a
    // face-flag tile found near it.
    let lumbridge = tile(3222, 3218, 0);
    assert!(view.walkable(lumbridge));
    let mut solid_edge = None;
    let mut face = None;
    for dx in -24..=24 {
        for dz in -24..=24 {
            let t = tile(lumbridge.x + dx, lumbridge.z + dz, 0);
            if solid_edge.is_none() && view.walkable(t) {
                for (ex, ez) in [(0, 1), (0, -1), (1, 0), (-1, 0)] {
                    let n = tile(t.x + ex, t.z + ez, 0);
                    if !view.standable(n) {
                        solid_edge = Some((t, n));
                        break;
                    }
                }
            }
            if face.is_none() && view.standable(t) && !view.walkable(t) {
                face = Some(t);
            }
        }
    }
    let (stand, solid) = solid_edge.expect("a walkable tile beside a solid tile near Lumbridge");
    let face = face.expect("a face-flag tile near Lumbridge");

    let mut compared = 0usize;
    for anchor in [lumbridge, stand, solid, face] {
        for radius in 0..=3 {
            for dx in -2..=2 {
                for dz in -2..=2 {
                    for level in [0, 1] {
                        let to = tile(anchor.x + dx, anchor.z + dz, level);
                        compare(&world, &lease, lumbridge, to, radius);
                        compared += 1;
                    }
                }
            }
        }
    }
    assert_eq!(compared, 4 * 4 * 25 * 2);

    // Real values on the real pack.
    let courtyard = producer_candidates(&world, lumbridge, lumbridge, 1);
    assert!(courtyard.contains(&lumbridge));
    assert!(
        courtyard.len() > 1,
        "open ground has neighbours: {courtyard:?}"
    );
    let beside = producer_candidates(&world, lumbridge, solid, 1);
    assert!(
        !beside.contains(&solid),
        "a solid destination is never stood on"
    );
    assert!(
        beside.contains(&stand),
        "it is approached from its open side"
    );
    assert!(
        producer_candidates(&world, lumbridge, solid, 0).is_empty(),
        "radius 0 never arrives at a solid tile"
    );
    assert!(producer_candidates(&world, lumbridge, face, 0).contains(&face));
    assert!(
        producer_candidates(&world, lumbridge, lumbridge, 3)
            .iter()
            .all(|t| t.level == 0),
        "candidates stay on the destination's plane"
    );
}

fn router_goal_tiles(
    world: &Arc<NavWorld>,
    snapshot: Option<&api::snapshot::GameSnapshot>,
    from: WorldTile,
    to: WorldTile,
    loc_id: Option<i32>,
    radius: i32,
    arrival: nav::arrival::ArrivalKind,
) -> Vec<WorldTile> {
    let live_geometry = if radius > 0 {
        snapshot.and_then(|snapshot| {
            loc_id.and_then(|loc_id| capture_loc_geometry(snapshot, to, radius, loc_id))
        })
    } else {
        None
    };
    let request = ScriptRouteRequest {
        generation: 1,
        request_id: 1,
        world: Arc::clone(world),
        from,
        to,
        candidate_goals: None,
        radius,
        loc_id,
        arrival,
        opts: super::FindOptions::default(),
        state: None,
        admission: crate::admission::Admission::manual(
            super::FindOptions::default(),
            Default::default(),
            1,
            crate::WalkGlobals::default(),
        ),
        bank_origin: super::Origin::Unknown,
        bank: Vec::new(),
        live_geometry,
        exclusions: None,
        compat_tile_intent: false,
        completion: super::RouteCompletion::default(),
    };
    let mut cursor = RouteTargetCursor::new(&request).expect("bounded route targets");
    while !cursor.advance(&request) {}
    cursor.start_sort();
    while cursor.sort.is_some() && !cursor.advance(&request) {}
    let mut targets = cursor.targets;
    if arrival_filter_needed(&request) {
        let collision = &world.collision;
        let bounds = api::query::ArrivalCollisionBounds::new(
            collision.origin.x,
            collision.origin.z,
            collision.width,
            collision.height,
        )
        .expect("packed collision bounds");
        targets = block_on(route_arrival_filter_future(
            Arc::clone(world),
            targets,
            to,
            radius,
            bounds,
            crate::admission::route_yield_unit,
        ));
    }
    targets.sort_unstable_by_key(|tile| (tile.x, tile.z, tile.level));
    targets
}

fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = Box::pin(future);
    let mut context = Context::from_waker(Waker::noop());
    loop {
        match Future::poll(future.as_mut(), &mut context) {
            Poll::Ready(output) => return output,
            Poll::Pending => {}
        }
    }
}

fn selection_goal_tiles(
    world: &NavWorld,
    snapshot: Option<&api::snapshot::GameSnapshot>,
    to: WorldTile,
    loc_id: Option<i32>,
    radius: i32,
    arrival: nav::arrival::ArrivalKind,
) -> Vec<WorldTile> {
    crate::walk_permissions::selection_access_tiles(
        world,
        snapshot,
        to,
        loc_id,
        u16::try_from(radius).expect("selection radius fits its input"),
        arrival,
    )
    .collect()
}

/// Fixture loc metadata uses known 2x2 geometry; scene collision words come
/// from the selected NAV_PACK.
fn tree_snapshot(
    world: &NavWorld,
    target: WorldTile,
    loc_id: i32,
    modeled: bool,
    force_approach: i32,
    blocked: Option<WorldTile>,
) -> api::snapshot::GameSnapshot {
    const WIDTH: i32 = 10;
    const HEIGHT: i32 = 10;
    let base_x = target.x - 4;
    let base_z = target.z - 4;
    let mut collision_flags = Vec::with_capacity((WIDTH * HEIGHT) as usize);
    for local_x in 0..WIDTH {
        for local_z in 0..HEIGHT {
            collision_flags.push(
                i32::try_from(world.collision.walkable_word(
                    base_x + local_x,
                    base_z + local_z,
                    target.level,
                ))
                .unwrap_or(CollisionFlag::SQ_BLOCKED),
            );
        }
    }
    if let Some(blocked) = blocked {
        assert_eq!(blocked.level, target.level);
        let local_x = blocked.x - base_x;
        let local_z = blocked.z - base_z;
        assert!((0..WIDTH).contains(&local_x));
        assert!((0..HEIGHT).contains(&local_z));
        collision_flags[(local_x * HEIGHT + local_z) as usize] = CollisionFlag::SQ_BLOCKED;
    }
    let mut snapshot = api::snapshot::GameSnapshot::new();
    snapshot.seed_scene(api::snapshot::SceneView {
        available: true,
        base_x,
        base_z,
        level: target.level,
        width: WIDTH,
        height: HEIGHT,
        collision_flags,
    });
    if modeled {
        snapshot.seed_locs(vec![api::snapshot::LocView {
            typecode: 10,
            info: 0,
            id: loc_id,
            name: Some("Tree".into()),
            description: None,
            actions: vec![Some("Chop down".into())],
            tile: target,
            distance: 0,
            layer: api::snapshot::LocLayer::Ground,
            shape: 10,
            angle: 0,
            width: 2,
            length: 2,
            footprint_width: 2,
            footprint_length: 2,
            block_walk: false,
            block_range: false,
            active: true,
            animation: -1,
            map_function: -1,
            map_scene: -1,
            force_approach,
        }]);
    }
    snapshot
}

#[test]
fn explicit_loc_fallback_keeps_router_radius_stands_without_reach_filtering() {
    let world = asymmetric_world();
    let target = tile(3203, 3202, 0);
    let cut_off = tile(3201, 3202, 0);
    let route = router_goal_tiles(
        &world,
        None,
        target,
        target,
        Some(1278),
        2,
        nav::arrival::ArrivalKind::Reach,
    );
    assert!(route.contains(&cut_off));
    let selector = selection_goal_tiles(
        &world,
        None,
        target,
        Some(1278),
        2,
        nav::arrival::ArrivalKind::Reach,
    );
    assert_eq!(
        route, selector,
        "an off-scene explicit loc uses the router's standable-radius goals"
    );

    // This is the old-code divergence control: the tile is standable and in
    // the radius square, but the intervening solid wall makes plain Reach
    // false. The old selector filtered it while the router retained it.
    let probe = api::query::CollisionArrivalProbe::new(|tile| {
        super::arrival_collision_flags(&world.collision, tile)
    });
    assert!(!api::query::is_arrived(cut_off, target, 2, || &probe));

    let reach = router_goal_tiles(
        &world,
        None,
        target,
        target,
        None,
        2,
        nav::arrival::ArrivalKind::Reach,
    );
    assert!(!reach.contains(&cut_off));
    assert_eq!(
        reach,
        selection_goal_tiles(
            &world,
            None,
            target,
            None,
            2,
            nav::arrival::ArrivalKind::Reach,
        )
    );

    // A radius beyond the candidate clamp still uses the route's raw arrival
    // radius for packed reach budgets while its candidate window stays 104.
    assert_eq!(
        router_goal_tiles(
            &world,
            None,
            target,
            target,
            None,
            105,
            nav::arrival::ArrivalKind::Reach,
        ),
        selection_goal_tiles(
            &world,
            None,
            target,
            None,
            105,
            nav::arrival::ArrivalKind::Reach,
        )
    );

    let blocked_target = tile(3202, 3202, 0);
    assert_eq!(
        router_goal_tiles(
            &world,
            None,
            blocked_target,
            target,
            None,
            0,
            nav::arrival::ArrivalKind::Reach,
        ),
        vec![blocked_target]
    );
    assert_eq!(
        selection_goal_tiles(
            &world,
            None,
            blocked_target,
            None,
            0,
            nav::arrival::ArrivalKind::Reach,
        ),
        vec![blocked_target],
        "exact radius zero preserves the requested tile"
    );

    let approach_target = tile(3204, 3202, 0);
    let forced = tree_snapshot(&world, approach_target, 1278, true, 0x8, None);
    let forced_route = router_goal_tiles(
        &world,
        Some(&forced),
        approach_target,
        approach_target,
        Some(1278),
        2,
        nav::arrival::ArrivalKind::Reach,
    );
    assert!(
        !forced_route.contains(&tile(
            approach_target.x - 1,
            approach_target.z,
            approach_target.level,
        )),
        "the forced-west mask rejects that approach"
    );
    assert_eq!(
        forced_route,
        selection_goal_tiles(
            &world,
            Some(&forced),
            approach_target,
            Some(1278),
            2,
            nav::arrival::ArrivalKind::Reach,
        )
    );

    let blocked_approach = tile(
        approach_target.x - 1,
        approach_target.z,
        approach_target.level,
    );
    let colliding = tree_snapshot(
        &world,
        approach_target,
        1278,
        true,
        0,
        Some(blocked_approach),
    );
    let collision_route = router_goal_tiles(
        &world,
        Some(&colliding),
        approach_target,
        approach_target,
        Some(1278),
        2,
        nav::arrival::ArrivalKind::Reach,
    );
    assert!(!collision_route.contains(&blocked_approach));
    assert_eq!(
        collision_route,
        selection_goal_tiles(
            &world,
            Some(&colliding),
            approach_target,
            Some(1278),
            2,
            nav::arrival::ArrivalKind::Reach,
        )
    );
}

#[test]
fn router_and_selector_goal_tiles_match_for_real_tree_anchors_on_nav_pack() {
    let Some(path) = std::env::var_os("NAV_PACK") else {
        eprintln!("SKIP: NAV_PACK is not set");
        return;
    };
    let world = Arc::new(NavWorld::load_pack(std::path::Path::new(&path)).expect("real nav pack"));
    let trees = [
        (tile(3113, 3235, 0), 1278), // Draynor normal tree.
        (tile(3111, 3226, 0), 1276), // Nearby Draynor tree.
        (tile(3225, 3232, 0), 1278), // Normal tree in m50_50.
    ];
    let in_bounds = |coordinate: i32, origin: i32, extent: usize| {
        coordinate
            .checked_sub(origin)
            .and_then(|offset| usize::try_from(offset).ok())
            .is_some_and(|offset| offset < extent)
    };
    for (target, loc_id) in trees {
        assert!(
            (0..=3).contains(&target.level)
                && in_bounds(target.x, world.collision.origin.x, world.collision.width)
                && in_bounds(target.z, world.collision.origin.z, world.collision.height),
            "real tree anchor {target:?} must be covered by the NAV_PACK"
        );
        for radius in [0, 1, 2, 4] {
            let modeled = tree_snapshot(&world, target, loc_id, true, 0, None);
            assert!(
                capture_loc_geometry(&modeled, target, 1, loc_id).is_some(),
                "catalog tree {target:?} id={loc_id} must be modeled in-scene"
            );
            for arrival in [
                nav::arrival::ArrivalKind::Reach,
                nav::arrival::ArrivalKind::Area,
            ] {
                let router = router_goal_tiles(
                    &world,
                    Some(&modeled),
                    target,
                    target,
                    Some(loc_id),
                    radius,
                    arrival,
                );
                let selector = selection_goal_tiles(
                    &world,
                    Some(&modeled),
                    target,
                    Some(loc_id),
                    radius,
                    arrival,
                );
                assert_eq!(
                    router, selector,
                    "modeled tree {target:?} id={loc_id} r={radius} arrival={arrival:?}"
                );
            }
        }

        let unmodeled = tree_snapshot(&world, target, loc_id, false, 0, None);
        for radius in [1, 2, 4] {
            for arrival in [
                nav::arrival::ArrivalKind::Reach,
                nav::arrival::ArrivalKind::Area,
            ] {
                for snapshot in [None, Some(&unmodeled)] {
                    let router = router_goal_tiles(
                        &world,
                        snapshot,
                        target,
                        target,
                        Some(loc_id),
                        radius,
                        arrival,
                    );
                    let selector = selection_goal_tiles(
                        &world,
                        snapshot,
                        target,
                        Some(loc_id),
                        radius,
                        arrival,
                    );
                    assert_eq!(
                        router, selector,
                        "fallback tree {target:?} id={loc_id} r={radius} arrival={arrival:?}"
                    );
                }
                let router = router_goal_tiles(
                    &world,
                    Some(&unmodeled),
                    target,
                    target,
                    None,
                    radius,
                    arrival,
                );
                let selector =
                    selection_goal_tiles(&world, Some(&unmodeled), target, None, radius, arrival);
                assert_eq!(
                    router, selector,
                    "plain tree tile {target:?} r={radius} arrival={arrival:?}"
                );
            }
        }
    }
}
