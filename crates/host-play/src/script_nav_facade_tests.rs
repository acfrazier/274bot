//! Producer-owned differential for the packed Reach promise (contract §23
//! §1.1 "Coordinates and arrival"): the host's actual candidate filter,
//! [`approach_tiles`] with `estimate = false` — the production collision
//! callback at `script_nav.rs:1912-1917` — against the nav facade's
//! `CollisionView::arrived`, over the same world. nav cannot observe this
//! producer (D11: `host-play → nav`, never the reverse), so the comparison
//! lives here. The facade side never calls the producer and the producer
//! side never calls the facade.

use std::sync::Arc;

use api::snapshot::WorldTile;
use client::dash3d::CollisionFlag;
use nav::collision::{pack_walk, WorldCollision};
use nav::contract::{ArrivalSpec, NavLease, PackDigest};
use nav::transport::TransportGraph;
use nav::world::NavWorld;

use super::approach_tiles;

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
