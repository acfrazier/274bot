use api::obj_names::LocDefs;
use api::selected::QuestGate;
use api::snapshot::WorldTile;
use client::config::LocType;
use client::dash3d::CollisionFlag;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::bank_fetch::fetchable_state;
use crate::collision::{bake_from_maps, WorldCollision};
use crate::grid::StepGrid;
use crate::pack::{BankAccess, BankStand};
use crate::quest_gates::tests::{
    family as gate_family, range as gate_range, stamp, tbwt_evidence, window as gate_window,
    Resolved,
};
use crate::quest_gates::{QuestEvidence, QuestFamilyMismatch, QuestGates};
use crate::router::{
    find, find_allow_teleports, find_bounded, find_first_with, find_first_with_fallback,
    find_many_with, find_many_with_avoid_bounded, find_many_with_avoid_bounded_until,
    find_missing_item_reqs, find_missing_item_reqs_with_avoid, find_on_grid,
    find_unresolved_quest_gates, find_with, find_with_avoid, find_with_avoid_bounded,
    find_with_model, local_step_component, missing_item_reqs, step_ok, AvoidRect, CostModel,
    FallbackRoute, FindOptions, GridLeg, Leg, MissingReq, ReverseProof, RouteError, TargetError,
    BANK_TARGET_BUDGET, FIRST_TARGET_BUDGET, PER_STEP_WALK,
};
use crate::tile::Tile;
use crate::transport::{
    TransportEdge, TransportGraph, TransportKind, WildernessRules, WildernessZone,
};
use crate::world_state::WorldState;
use crate::zones::{Zone, ZoneClass, ZoneExempt, ZoneKey, ZoneKind, ZoneTable};

#[test]
fn local_component_rejects_invalid_origins_and_clamps_radius() {
    let collision = WorldCollision {
        origin: WorldTile {
            x: 0,
            z: 0,
            level: 0,
        },
        width: 3,
        height: 3,
        walk: vec![0; 36],
        blocked: vec![0],
        flags: None,
    };
    assert!(local_step_component(
        &collision,
        WorldTile {
            x: -1,
            z: 0,
            level: 0,
        },
        1,
    )
    .is_empty());
    assert!(local_step_component(
        &collision,
        WorldTile {
            x: 0,
            z: 0,
            level: 4,
        },
        1,
    )
    .is_empty());
    assert_eq!(
        local_step_component(&collision, collision.origin, 999).len(),
        9
    );
}

#[test]
fn find_on_grid_across_open_3x3_is_a_walk_leg() {
    let g = StepGrid::fixture_open_3x3();
    let r = find_on_grid(
        &g,
        Tile {
            x: 0,
            z: 0,
            level: 0,
        },
        Tile {
            x: 2,
            z: 2,
            level: 0,
        },
    )
    .unwrap();
    assert_eq!(
        r.dest,
        Tile {
            x: 2,
            z: 2,
            level: 0
        }
    );
    let GridLeg::Walk { tiles } = &r.legs[0] else {
        panic!()
    };
    assert_eq!(tiles.first().unwrap().x, 0);
    assert_eq!(tiles.last(), Some(&r.dest));
}

#[test]
fn find_on_grid_through_wall_is_no_path() {
    let mut g = StepGrid::fixture_open_3x3();
    g.set_walkable(
        Tile {
            x: 1,
            z: 0,
            level: 0,
        },
        false,
    );
    g.set_walkable(
        Tile {
            x: 1,
            z: 1,
            level: 0,
        },
        false,
    );
    g.set_walkable(
        Tile {
            x: 1,
            z: 2,
            level: 0,
        },
        false,
    );
    assert!(find_on_grid(
        &g,
        Tile {
            x: 0,
            z: 1,
            level: 0
        },
        Tile {
            x: 2,
            z: 1,
            level: 0
        }
    )
    .is_err());
}

#[test]
fn find_on_grid_uses_door_edge_across_a_wall() {
    let g = StepGrid::fixture_door_corridor();
    let r = find_on_grid(
        &g,
        Tile {
            x: 0,
            z: 0,
            level: 0,
        },
        Tile {
            x: 4,
            z: 0,
            level: 0,
        },
    )
    .unwrap();
    assert!(r
        .legs
        .iter()
        .any(|l| matches!(l, GridLeg::Door { loc_id: 1530, .. })));
}

#[test]
fn door_route_splits_into_walk_door_walk_legs_on_grid() {
    let g = StepGrid::fixture_door_corridor();
    let r = find_on_grid(
        &g,
        Tile {
            x: 0,
            z: 0,
            level: 0,
        },
        Tile {
            x: 4,
            z: 0,
            level: 0,
        },
    )
    .unwrap();
    assert_eq!(r.legs.len(), 3);
    let (
        GridLeg::Walk { tiles: w0 },
        GridLeg::Door {
            loc,
            loc_id,
            from,
            to,
        },
        GridLeg::Walk { tiles: w1 },
    ) = (&r.legs[0], &r.legs[1], &r.legs[2])
    else {
        panic!("expected Walk, Door, Walk legs");
    };
    assert_eq!(
        w0.first(),
        Some(&Tile {
            x: 0,
            z: 0,
            level: 0
        })
    );
    assert_eq!(w0.last(), Some(from));
    assert_eq!(w1.first(), Some(to));
    assert_eq!(
        w1.last(),
        Some(&Tile {
            x: 4,
            z: 0,
            level: 0
        })
    );
    assert_eq!(loc_id, &1530);
    assert_eq!(
        loc,
        &Tile {
            x: 2,
            z: 0,
            level: 0
        }
    );
}

// --- Dijkstra router over collision + transport graph ---

fn tile(x: i32, z: i32, level: i32) -> WorldTile {
    WorldTile { x, z, level }
}

/// A `width × height` level-0 bake at (0,0) with the given per-tile
/// flags OR'd in. Planes 1..=3 stay empty (the per-level bake shape).
fn bake(width: usize, height: usize, extras: &[(i32, i32, u32)]) -> WorldCollision {
    bake_at(0, 0, width, height, extras)
}

/// A `width × height` level-0 bake at (`ox`, `oz`): the origin-offset
/// variant of [`bake`] for fixtures pinned to real world tiles (the
/// essence-mine mapsquare).
fn bake_at(
    ox: i32,
    oz: i32,
    width: usize,
    height: usize,
    extras: &[(i32, i32, u32)],
) -> WorldCollision {
    let mut plane = vec![0u32; width * height];
    for &(x, z, f) in extras {
        plane[(z - oz) as usize * width + (x - ox) as usize] |= f;
    }
    let mut flags = vec![0u32; 4 * plane.len()];
    flags[..plane.len()].copy_from_slice(&plane);
    let (walk, blocked) = crate::collision::pack_walk(&flags);
    WorldCollision {
        origin: tile(ox, oz, 0),
        width,
        height,
        walk,
        blocked,
        flags: None,
    }
}

/// A scratch mapsquare directory for one fixture, removed on drop.
struct FixDir(PathBuf);

impl FixDir {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("274bot-nav-router-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        FixDir(dir)
    }
}

impl Drop for FixDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A one-loc `LocDefs` table.
fn defs(locs: &[LocType]) -> LocDefs {
    LocDefs::from_locs(locs)
}

/// A 5×5 grid split by a wall between x=1 and x=2: the client's `W_E` on
/// column 1 and `W_W` on column 2, so no step (or diagonal) crosses.
fn walled_5x5() -> WorldCollision {
    let mut extras = Vec::new();
    for z in 0..5 {
        extras.push((1, z, CollisionFlag::W_E as u32));
        extras.push((2, z, CollisionFlag::W_W as u32));
    }
    bake(5, 5, &extras)
}

/// The same wall with a door gap at `gap_z`: column 1 carries no `W_E`
/// there, so the door's `from` tile stays open while column 2's `W_W`
/// still seals the crossing.
fn walled_5x5_gap(gap_z: i32) -> WorldCollision {
    let mut extras = Vec::new();
    for z in 0..5 {
        if z != gap_z {
            extras.push((1, z, CollisionFlag::W_E as u32));
        }
        extras.push((2, z, CollisionFlag::W_W as u32));
    }
    bake(5, 5, &extras)
}

/// A 5×6 bake (x=0..4, z=0..5) walled between the west and east sides:
/// column 2 carries `W_W` for every row, column 1 carries `W_E` for
/// rows 1..=5, and the door's own `at=(2,0)` tile carries
/// `W_W | WR_GRND` — blocked, sealing the row-0 gap. The only crossing
/// is the door edge, and `at` itself is never standable. Row 4 (`W_N`
/// on every column) seals the north strip (z=5) away from the door's
/// radius-3 neighborhood.
fn blocked_door_fixture() -> WorldCollision {
    let mut extras = Vec::new();
    for z in 1..=5 {
        extras.push((1, z, CollisionFlag::W_E as u32));
        extras.push((2, z, CollisionFlag::W_W as u32));
    }
    for x in 0..5 {
        extras.push((x, 4, CollisionFlag::W_N as u32));
    }
    extras.push((2, 0, (CollisionFlag::W_W | CollisionFlag::WR_GRND) as u32));
    bake(5, 6, &extras)
}

/// One directed door edge `at -> to` (loc 1530, `Open` op 1).
fn door(at: WorldTile, to: WorldTile, ticks: i32) -> TransportGraph {
    let edge = TransportEdge {
        kind: TransportKind::Door,
        at,
        to,
        loc_id: 1530,
        option: 1,
        ticks,
        dir: None,
        open_loc_id: None,
        skill_req: vec![],
        item_req: vec![],
        quest_req: vec![],
        varp_req: vec![],
        worn_req: vec![],
        members_req: false,
        wildy_cap: None,
        quest_gates: None,
    };
    let mut graph = TransportGraph::default();
    graph.at.entry(at).or_default().push(0);
    graph.edges.push(edge);
    graph
}

#[test]
fn rectangular_transport_takeoffs_share_forward_and_reverse_wall_admission() {
    let from = tile(0, 6, 0);
    let to = tile(59, 4, 1);
    let state = WorldState::empty();
    let opts = FindOptions::default();
    for (wall, blocked_sides, allowed) in [
        (0, 0xd, true),
        (CollisionFlag::W_W as u32, 0xd, false),
        (0, 0xf, false),
    ] {
        // A corridor long enough to run the backward proof reaches only the
        // east face, four tiles beyond the anchor. The destination is sealed
        // on another plane: radius-one reverse predecessors would miss it.
        let mut flags = vec![CollisionFlag::SQ_BLOCKED as u32; 4 * 64 * 8];
        for x in 0..=54 {
            flags[6 * 64 + x] = 0;
        }
        flags[4 * 64 + 54] = wall;
        flags[5 * 64 + 54] = wall;
        flags[64 * 8 + 4 * 64 + 59] = 0;
        let (walk, blocked) = crate::collision::pack_walk(&flags);
        let collision = WorldCollision {
            origin: tile(0, 0, 0),
            width: 64,
            height: 8,
            walk,
            blocked,
            flags: None,
        };
        let mut graph = door(tile(50, 3, 0), to, 1);
        graph.edges[0].kind = TransportKind::Stairs;
        graph.approaches = vec![Some(api::query::loc_approach::LocApproach {
            width: 4,
            length: 3,
            blocked_sides,
        })];
        graph.rebuild_index(&collision);
        assert_eq!(
            graph.admissible_from(&collision, 0, tile(54, 5, 0)),
            allowed
        );
        let direct = find_with(&collision, &graph, from, to, opts, &state);
        let first = find_first_with(&collision, &graph, from, &[to], opts, &state);
        if allowed {
            assert_eq!(direct.unwrap().ticks, 28.5);
            assert_eq!(first.route().unwrap().dest, to);
        } else {
            assert_eq!(direct.err(), Some(RouteError::NoPath));
            assert_eq!(first.route().err(), Some(RouteError::NoPath));
            assert_eq!(first.proof(), ReverseProof::Unreachable);
        }
    }
}

/// One any-tile teleport edge in `graph.teleports` (never in `at`).
fn teleport(
    to: WorldTile,
    ticks: i32,
    skill_req: Vec<(i32, i32)>,
    item_req: Vec<(i32, i32)>,
) -> TransportGraph {
    let mut graph = TransportGraph::default();
    graph.teleports.push(TransportEdge {
        kind: TransportKind::Teleport,
        at: tile(0, 0, 0),
        to,
        loc_id: 0,
        option: 0,
        ticks,
        dir: None,
        open_loc_id: None,
        skill_req,
        item_req,
        quest_req: vec![],
        varp_req: vec![],
        worn_req: vec![],
        members_req: false,
        wildy_cap: None,
        quest_gates: None,
    });
    graph
}

#[test]
fn find_across_open_room_is_a_single_walk_leg() {
    let wc = bake(5, 5, &[]);
    let g = TransportGraph::default();
    let r = find(&wc, &g, tile(0, 0, 0), tile(4, 4, 0)).unwrap();
    assert_eq!(r.dest, tile(4, 4, 0));
    assert_eq!(r.ticks, 2.0); // 4 run steps at 0.5 ticks each
    assert_eq!(r.legs.len(), 1);
    let Leg::Walk { tiles } = &r.legs[0] else {
        panic!("walk-only route");
    };
    assert_eq!(tiles.first(), Some(&tile(0, 0, 0)));
    assert_eq!(tiles.last(), Some(&tile(4, 4, 0)));
}

#[test]
fn find_from_equals_to_is_a_single_tile_walk() {
    let wc = bake(3, 3, &[]);
    let g = TransportGraph::default();
    let r = find(&wc, &g, tile(1, 1, 0), tile(1, 1, 0)).unwrap();
    assert_eq!(r.ticks, 0.0); // no steps walked
    assert_eq!(
        r.legs,
        vec![Leg::Walk {
            tiles: vec![tile(1, 1, 0)]
        }]
    );
}

#[test]
fn find_through_an_unbroken_wall_is_no_path() {
    let wc = walled_5x5();
    let g = TransportGraph::default();
    assert!(matches!(
        find(&wc, &g, tile(0, 0, 0), tile(4, 4, 0)),
        Err(RouteError::NoPath)
    ));
}

#[test]
fn router_uses_transport_across_a_wall() {
    // The wall has a door gap at z=2: the door's from tile stays open,
    // but the wall's own stamps otherwise seal the crossing.
    let wc = walled_5x5_gap(2);
    let g = door(tile(1, 2, 0), tile(2, 2, 0), 2);
    let r = find(&wc, &g, tile(0, 0, 0), tile(4, 4, 0)).unwrap();
    assert_eq!(r.dest, tile(4, 4, 0));
    // Origin (0,0) is chebyshev 2 from at=(1,2), so the search walks
    // one step to an adjacent take-off (0.5) then the 2-tick door plus
    // two run steps from the far side (1.0).
    assert_eq!(r.ticks, 3.5);
    assert_eq!(r.legs.len(), 3);
    let (Leg::Walk { tiles: w0 }, Leg::Transport { edge }, Leg::Walk { tiles: w1 }) =
        (&r.legs[0], &r.legs[1], &r.legs[2])
    else {
        panic!("expected Walk, Transport, Walk legs");
    };
    assert_eq!(w0.first(), Some(&tile(0, 0, 0)));
    assert_eq!(
        w0.len(),
        2,
        "walk up to an adjacent take-off, not from (0,0)"
    );
    assert_eq!(edge.loc_id, 1530);
    assert_eq!(edge.ticks, 2);
    assert_eq!(edge.at, tile(1, 2, 0));
    assert_eq!(edge.to, tile(2, 2, 0));
    assert_eq!(w1.first(), Some(&tile(2, 2, 0)));
    assert_eq!(w1.last(), Some(&tile(4, 4, 0)));
}

#[test]
fn find_prefers_a_cheap_walk_over_a_costly_transport() {
    let wc = bake(5, 5, &[]);
    let g = door(tile(0, 0, 0), tile(4, 4, 0), 1000);
    let r = find(&wc, &g, tile(0, 0, 0), tile(4, 4, 0)).unwrap();
    // The 1000-tick door loses to the 2.0-tick walk (4 run steps).
    assert_eq!(r.ticks, 2.0);
    assert_eq!(r.legs.len(), 1);
    assert!(matches!(&r.legs[0], Leg::Walk { .. }));
}

/// The blocked interact target: the door's `at=(2,0)` tile carries a
/// footprint/ground block, so it is neither standable nor walkable and
/// the old `at`-only expansion could never settle a node on it. The
/// neighborhood expansion must take the door from a *neighbouring*
/// standable tile instead.
#[test]
fn router_takes_a_transport_from_a_standable_tile_within_the_interact_radius() {
    let wc = blocked_door_fixture();
    let g = door(tile(2, 0, 0), tile(2, 2, 0), 2);
    let r = find(&wc, &g, tile(1, 0, 0), tile(4, 2, 0)).unwrap();
    assert_eq!(r.dest, tile(4, 2, 0));
    // The 2-tick door taken from (1,0) + 2 run steps from (2,2) (1.0).
    assert_eq!(r.ticks, 3.0);
    let (Leg::Walk { tiles: w0 }, Leg::Transport { edge }, Leg::Walk { tiles: w1 }) =
        (&r.legs[0], &r.legs[1], &r.legs[2])
    else {
        panic!("expected Walk, Transport, Walk legs");
    };
    // The walk leg before the door ends at the take-off tile (1,0) —
    // a standable tile within the interact radius of `at`, not `at`
    // itself.
    assert_eq!(w0, &vec![tile(1, 0, 0)]);
    assert_eq!(edge.loc_id, 1530);
    assert_eq!(edge.ticks, 2);
    assert_eq!(edge.at, tile(2, 0, 0));
    assert_eq!(edge.to, tile(2, 2, 0));
    assert_eq!(w1.first(), Some(&tile(2, 2, 0)));
    assert_eq!(w1.last(), Some(&tile(4, 2, 0)));
    // Never steps onto the blocked `at` tile.
    let stepped: Vec<WorldTile> = r
        .legs
        .iter()
        .flat_map(|l| match l {
            Leg::Walk { tiles } => tiles.clone(),
            Leg::Transport { .. } => vec![],
        })
        .collect();
    assert!(!stepped.contains(&tile(2, 0, 0)));
}

#[test]
fn router_does_not_use_a_transport_from_beyond_the_interact_radius() {
    let wc = blocked_door_fixture();
    let g = door(tile(2, 0, 0), tile(2, 2, 0), 2);
    // (1,5) sits at chebyshev 5 from `at=(2,0)` and is sealed away from
    // every within-radius tile by the row-4 wall, so the door stays
    // unusable: no path reaches the east side.
    assert!(matches!(
        find(&wc, &g, tile(1, 5, 0), tile(4, 2, 0)),
        Err(RouteError::NoPath)
    ));
    // A same-strip destination routes by walking, never via the door.
    let r = find(&wc, &g, tile(1, 5, 0), tile(0, 5, 0)).unwrap();
    assert_eq!(r.ticks, 0.5);
    assert!(r.legs.iter().all(|l| matches!(l, Leg::Walk { .. })));
}

/// A south face flag on the middle tile blocks entering it from the
/// south, not from the north — a 1-wide corridor stays a corridor.
#[test]
fn find_face_flags_block_only_the_matching_direction() {
    let wc = bake(1, 3, &[(0, 1, CollisionFlag::W_S as u32)]);
    let g = TransportGraph::default();
    assert!(
        matches!(
            find(&wc, &g, tile(0, 0, 0), tile(0, 2, 0)),
            Err(RouteError::NoPath)
        ),
        "cannot enter the W_S tile from the south"
    );
    let r = find(&wc, &g, tile(0, 2, 0), tile(0, 0, 0)).expect("north-to-south still walks");
    assert!(r.legs.iter().all(|l| matches!(l, Leg::Walk { .. })));
}

/// The wall-tile fixture from the live `nav_door` trace: wall 980
/// (WALL_STRAIGHT, south) at (2816,3437) and door 1530 (WALL_STRAIGHT,
/// north) at (2816,3438) — m44_53 `0 0 45: 980 0 3` and
/// `0 0 46: 1530 0 1`. `step_ok` must reject every step into the wall
/// tile (the east step the live walker took) and the closed door, while
/// genuinely open neighbours still pass, and the router never routes
/// onto the wall tile.
#[test]
fn wall_tile_blocks_through_wall_steps_and_the_router_avoids_it() {
    let fix = FixDir::new("wall-980-door-1530");
    fs::write(
        fix.0.join("m43_53.jm2"),
        "==== MAP ====\n0 63 44: h1 u50\n0 63 45: h1 u50\n0 63 46: h1 u50\n==== LOC ====\n",
    )
    .unwrap();
    fs::write(
            fix.0.join("m44_53.jm2"),
            "==== MAP ====\n0 0 43: h1 u50\n0 0 44: h10 u50\n0 0 45: h19 o10 u48\n0 0 46: h30 o10 u48\n0 0 47: h30 o5 f4 u50\n==== LOC ====\n0 0 45: 980 0 3\n0 0 46: 1530 0 1\n",
        )
        .unwrap();
    let locs = defs(&[
        LocType {
            id: 980,
            blockwalk: true,
            ..LocType::default()
        },
        LocType {
            id: 1530,
            blockwalk: true,
            ..LocType::default()
        },
    ]);
    let mut door_ids = HashSet::new();
    door_ids.insert(1530);
    let wc = bake_from_maps(&fix.0, &locs, &door_ids).unwrap();
    let g = TransportGraph::default();

    // South face of wall 980: cannot enter that tile from the south.
    // Entering it from the west is a walk along the wall, not through it.
    assert!(!step_ok(&wc, tile(2816, 3436, 0), (0, 1)));
    assert!(step_ok(&wc, tile(2815, 3437, 0), (1, 0)));
    assert!(!step_ok(&wc, tile(2816, 3438, 0), (0, 1)));
    // A genuinely open neighbour still passes.
    assert!(step_ok(&wc, tile(2815, 3437, 0), (0, 1)));
    assert!(step_ok(&wc, tile(2815, 3437, 0), (-1, 0)));

    let r = find(&wc, &g, tile(2813, 3436, 0), tile(2815, 3438, 0)).unwrap();
    let Leg::Walk { tiles } = &r.legs[0] else {
        panic!("walk-only route");
    };
    // Crossing the wall's south face is still rejected; the path stays
    // on the open side.
    assert!(!tiles.contains(&tile(2816, 3436, 0)));
}

#[test]
fn find_transport_changes_level_and_walks_upstairs() {
    let wc = bake(4, 4, &[]);
    let ladder = TransportEdge {
        kind: TransportKind::Ladder,
        at: tile(0, 0, 0),
        to: tile(1, 1, 1),
        loc_id: 1747,
        option: 1,
        ticks: 3,
        dir: None,
        open_loc_id: None,
        skill_req: vec![],
        item_req: vec![],
        quest_req: vec![],
        varp_req: vec![],
        worn_req: vec![],
        members_req: false,
        wildy_cap: None,
        quest_gates: None,
    };
    let mut g = TransportGraph::default();
    g.at.entry(ladder.at).or_default().push(0);
    g.edges.push(ladder.clone());
    let r = find(&wc, &g, tile(0, 0, 0), tile(3, 1, 1)).unwrap();
    assert_eq!(r.dest, tile(3, 1, 1));
    // The 3-tick ladder plus 2 run steps on level 1 (1.0).
    assert_eq!(r.ticks, 4.0);
    let (Leg::Walk { tiles: w0 }, Leg::Transport { edge }, Leg::Walk { tiles: w1 }) =
        (&r.legs[0], &r.legs[1], &r.legs[2])
    else {
        panic!("expected Walk, Transport, Walk legs");
    };
    assert_eq!(w0, &vec![tile(0, 0, 0)]);
    assert_eq!(edge, &ladder);
    assert_eq!(w1.first(), Some(&tile(1, 1, 1)));
    assert_eq!(w1.last(), Some(&tile(3, 1, 1)));
}

#[test]
fn find_exhausts_the_node_budget_before_giving_up() {
    let wc = bake(10, 10, &[]);
    let g = TransportGraph::default();
    assert!(matches!(
        find_bounded(
            &wc,
            &g,
            tile(0, 0, 0),
            tile(9, 9, 0),
            CostModel::running(),
            8,
        ),
        Err(RouteError::BudgetExhausted)
    ));
    let r = find_bounded(
        &wc,
        &g,
        tile(0, 0, 0),
        tile(9, 9, 0),
        CostModel::running(),
        4096,
    )
    .unwrap();
    assert_eq!(r.dest, tile(9, 9, 0));
}

#[test]
fn find_prefers_a_cheap_door_over_a_long_walk_around() {
    // A 5×20 bake walled between x=1 and x=2 for z=1..=18 with a door
    // gap at z=10: crossing on foot means walking 20 tiles around the
    // wall ends (~10 ticks at the run rate), so the 1-tick door at
    // mid-wall is the cheaper total-tick route.
    let mut extras = Vec::new();
    for z in 1..=18 {
        if z != 10 {
            extras.push((1, z, CollisionFlag::W_E as u32));
        }
        extras.push((2, z, CollisionFlag::W_W as u32));
    }
    let wc = bake(5, 20, &extras);
    let g = door(tile(1, 10, 0), tile(2, 10, 0), 1);
    let r = find(&wc, &g, tile(0, 10, 0), tile(4, 10, 0)).unwrap();
    assert_eq!(r.dest, tile(4, 10, 0));
    // The origin sits within the door's interact radius of at=(1,10),
    // so the 1-tick door is taken from it (1.0) plus 2 walk tiles from
    // its far side (1.0).
    assert_eq!(r.ticks, 2.0);
    assert!(r.legs.iter().any(|l| matches!(l, Leg::Transport { .. })));
}

#[test]
fn find_prefers_walking_around_over_a_cheap_door() {
    // A single west-face flag at (2,2): walking around is 3 run steps
    // (1.5 ticks), cheaper than the 2-tick door, so the router walks.
    let wc = bake(5, 5, &[(2, 2, CollisionFlag::W_W as u32)]);
    let g = door(tile(1, 2, 0), tile(2, 2, 0), 2);
    let r = find(&wc, &g, tile(0, 2, 0), tile(3, 2, 0)).unwrap();
    assert_eq!(r.ticks, 1.5);
    assert!(r.legs.iter().all(|l| matches!(l, Leg::Walk { .. })));
}

// --- WorldState gating (Task 1: find fails closed on unpaid edges) ---

/// An Al Kharid toll shape: the 5×5 wall is unbroken except for one
/// door crossing, and that door costs 10 coins (`item_req`, the same
/// requirement `toll_edges` derives for the border gates).
fn toll_graph() -> TransportGraph {
    let mut g = door(tile(1, 2, 0), tile(2, 2, 0), 2);
    g.edges[0].item_req = vec![(995, 10)]; // the 10-coin toll
    g
}

/// A toll edge with an empty WorldState is not in the route — the
/// search cannot prove the player can pay, so it fails closed
/// (`NoPath`). The same edge with 10 coins in the inventory routes.
#[test]
fn find_gates_toll_edge_on_inventory_coins() {
    let wc = walled_5x5();
    let g = toll_graph();
    let from = tile(0, 0, 0);
    let to = tile(4, 4, 0);
    assert!(
        matches!(
            find_with(
                &wc,
                &g,
                from,
                to,
                FindOptions::default(),
                &WorldState::empty()
            ),
            Err(RouteError::NoPath)
        ),
        "empty WorldState must not relax the unpaid toll"
    );
    // The same search with 10 coins in the inventory crosses.
    let rich = WorldState {
        inv: HashMap::from([(995, 10)]),
        ..WorldState::default()
    };
    let r = find_with(&wc, &g, from, to, FindOptions::default(), &rich).unwrap();
    assert_eq!(r.dest, to);
    assert!(
        r.legs.iter().any(|l| matches!(
            l,
            Leg::Transport { edge } if edge.item_req == vec![(995, 10)]
        )),
        "the toll crossing is the transport leg"
    );
}

// --- Quest-stage gates: typed windows, decided by evidence only ---

/// The walled 5×5 whose one door is gated by `gates` (quest family 1).
fn stage_door_graph(gates: Vec<QuestGate>) -> TransportGraph {
    let mut g = door(tile(1, 2, 0), tile(2, 2, 0), 1);
    g.quest_family = Some(gate_family(1));
    g.edges[0].quest_gates = Some(QuestGates::new(gate_family(1), gates).unwrap());
    g
}

fn evidenced(evidence: QuestEvidence) -> WorldState {
    WorldState::empty().with_quest_evidence(evidence)
}

/// An exact stage window `[3,3]` opens its door only while fresh evidence
/// proves the signal is 3. No, stale or straddling evidence is `Unknown`:
/// the door is not taken and the diagnosis names the gate for a journal
/// read. Disjoint evidence is `False`: no read can open it, so nothing is
/// named — and a bank trip never opens a stage gate either.
#[test]
fn exact_stage_window_door_routes_only_on_proven_evidence() {
    let wc = walled_5x5();
    let exact = gate_window("tbwt", "tbwt_main", Some(3), Some(3));
    let g = stage_door_graph(vec![exact.clone()]);
    let (from, to) = (tile(0, 0, 0), tile(4, 4, 0));
    let opts = FindOptions::default();
    let route = |state: &WorldState| find_with(&wc, &g, from, to, opts, state);
    let needs =
        |state: &WorldState| find_unresolved_quest_gates(&wc, &g, from, &[to], opts, state, &[]);
    let named: Result<Option<Arc<[QuestGate]>>, QuestFamilyMismatch> =
        Ok(Some(Arc::from(vec![exact.clone()])));

    let proven = evidenced(tbwt_evidence(gate_range(Some(3), Some(3))));
    let crossed = route(&proven).expect("a proven window opens the door");
    assert!(crossed
        .legs
        .iter()
        .any(|leg| matches!(leg, Leg::Transport { edge } if edge.quest_gates.is_some())));
    assert_eq!(needs(&proven), Ok(None));

    let stale = evidenced(QuestEvidence::new(
        Arc::new(Resolved {
            stamp: stamp(10),
            signals: vec![("tbwt", "tbwt_main", gate_range(Some(3), Some(3)))],
            complete: vec![],
        }),
        gate_family(1),
        stamp(11),
    ));
    let straddling = evidenced(tbwt_evidence(gate_range(Some(3), Some(4))));
    for (label, state) in [
        ("no evidence", WorldState::empty()),
        ("evidence older than the floor", stale),
        ("3 or 4 still possible", straddling),
    ] {
        assert_eq!(route(&state), Err(RouteError::NoPath), "{label}");
        assert_eq!(needs(&state), named, "{label}");
        assert_eq!(
            find_missing_item_reqs(&wc, &g, from, to, opts, &state),
            None,
            "{label}: fetching items cannot decide a stage gate"
        );
    }

    let past = evidenced(tbwt_evidence(gate_range(Some(4), Some(6))));
    assert_eq!(route(&past), Err(RouteError::NoPath));
    assert_eq!(needs(&past), Ok(None), "a disproven window needs no read");
}

/// An upper-only window `[None,3]` holds for every possible value up to
/// and including 3, and for nothing that may exceed it.
#[test]
fn upper_only_stage_window_door_routes_through_its_bound() {
    let wc = walled_5x5();
    let g = stage_door_graph(vec![gate_window("tbwt", "tbwt_main", None, Some(3))]);
    for (possible, routes) in [
        (gate_range(Some(0), Some(3)), true),
        (gate_range(Some(3), Some(3)), true),
        (gate_range(None, Some(3)), true),
        (gate_range(Some(3), Some(4)), false),
        (gate_range(Some(4), None), false),
    ] {
        let state = evidenced(tbwt_evidence(possible));
        let result = find_with(
            &wc,
            &g,
            tile(0, 0, 0),
            tile(4, 4, 0),
            FindOptions::default(),
            &state,
        );
        assert_eq!(result.is_ok(), routes, "{possible:?}");
    }
}

/// However much an undecided gate would save, the search walks the long way
/// round (the wall is open at z=4); the same door is the shorter route once
/// evidence proves it.
#[test]
fn undecided_stage_gate_is_never_a_shortcut() {
    let mut extras = Vec::new();
    for z in 0..4 {
        extras.push((1, z, CollisionFlag::W_E as u32));
        extras.push((2, z, CollisionFlag::W_W as u32));
    }
    let wc = bake(5, 5, &extras);
    let g = stage_door_graph(vec![gate_window("tbwt", "tbwt_main", Some(3), Some(3))]);
    let (from, to) = (tile(0, 2, 0), tile(4, 2, 0));
    let opts = FindOptions::default();
    let undecided = evidenced(tbwt_evidence(gate_range(Some(2), Some(3))));
    let detour = find_with(&wc, &g, from, to, opts, &undecided).expect("the gap routes");
    assert!(detour
        .legs
        .iter()
        .all(|leg| matches!(leg, Leg::Walk { .. })));
    let proven = evidenced(tbwt_evidence(gate_range(Some(3), Some(3))));
    let direct = find_with(&wc, &g, from, to, opts, &proven).unwrap();
    assert!(direct.ticks < detour.ticks, "{direct:?} vs {detour:?}");
    assert!(direct
        .legs
        .iter()
        .any(|leg| matches!(leg, Leg::Transport { .. })));
}

/// Evidence prepared from another quest family never opens a gate, even
/// when its provider would prove the window, and the diagnosis refuses
/// instead of asking for journal reads that could not help.
#[test]
fn foreign_quest_family_evidence_is_refused_not_retried() {
    let wc = walled_5x5();
    let g = stage_door_graph(vec![gate_window("tbwt", "tbwt_main", Some(3), Some(3))]);
    let (from, to) = (tile(0, 0, 0), tile(4, 4, 0));
    let opts = FindOptions::default();
    let foreign = evidenced(QuestEvidence::new(
        Arc::new(Resolved {
            stamp: stamp(10),
            signals: vec![("tbwt", "tbwt_main", gate_range(Some(3), Some(3)))],
            complete: vec![],
        }),
        gate_family(2),
        stamp(5),
    ));
    assert_eq!(
        find_with(&wc, &g, from, to, opts, &foreign),
        Err(RouteError::NoPath)
    );
    assert_eq!(
        find_unresolved_quest_gates(&wc, &g, from, &[to], opts, &foreign, &[]),
        Err(QuestFamilyMismatch {
            pack: Some(gate_family(1)),
            expected: gate_family(2),
        })
    );
}

/// Only the gates evidence leaves undecided are named; a proven gate on the
/// same crossing is not re-read.
#[test]
fn stage_gate_diagnosis_names_only_undecided_gates() {
    let wc = walled_5x5();
    let heroes = gate_window("heroes", "heroes_main", Some(2), Some(2));
    let g = stage_door_graph(vec![
        gate_window("tbwt", "tbwt_main", Some(3), Some(3)),
        heroes.clone(),
    ]);
    let state = evidenced(tbwt_evidence(gate_range(Some(3), Some(3))));
    let (from, to) = (tile(0, 0, 0), tile(4, 4, 0));
    let opts = FindOptions::default();
    assert_eq!(
        find_with(&wc, &g, from, to, opts, &state),
        Err(RouteError::NoPath)
    );
    let named = find_unresolved_quest_gates(&wc, &g, from, &[to], opts, &state, &[])
        .unwrap()
        .expect("the heroes window is undecided");
    assert_eq!(&named[..], &[heroes]);
}

/// A door that needs a carried rope *and* an exact stage window: each
/// diagnosis sees past the other's gate and names only its own kind, so a
/// host with neither fact learns both what to fetch and what to read.
#[test]
fn rope_and_stage_door_diagnoses_name_each_missing_kind() {
    const ROPE: i32 = 954;
    let wc = walled_5x5();
    let window = gate_window("tbwt", "tbwt_main", Some(3), Some(3));
    let mut g = stage_door_graph(vec![window.clone()]);
    g.edges[0].item_req = vec![(ROPE, 1)];
    let (from, to) = (tile(0, 0, 0), tile(4, 4, 0));
    let opts = FindOptions::default();
    let rope = vec![MissingReq::Carry { id: ROPE, count: 1 }];
    let read: Option<Arc<[QuestGate]>> = Some(Arc::from(vec![window]));
    let with_rope = |state: WorldState| WorldState {
        inv: HashMap::from([(ROPE, 1)]),
        ..state
    };
    let undecided = || evidenced(tbwt_evidence(gate_range(Some(2), Some(3))));
    let proven = || evidenced(tbwt_evidence(gate_range(Some(3), Some(3))));
    let disproven = || evidenced(tbwt_evidence(gate_range(Some(4), Some(6))));

    for (label, state, fetch, journal, routes) in [
        (
            "neither",
            undecided(),
            Some(rope.clone()),
            read.clone(),
            false,
        ),
        (
            "rope only",
            with_rope(undecided()),
            None,
            read.clone(),
            false,
        ),
        ("stage only", proven(), Some(rope.clone()), None, false),
        ("both", with_rope(proven()), Some(vec![]), None, true),
        ("stage disproven", disproven(), None, None, false),
    ] {
        assert_eq!(
            find_with(&wc, &g, from, to, opts, &state).is_ok(),
            routes,
            "{label}"
        );
        assert_eq!(
            find_missing_item_reqs(&wc, &g, from, to, opts, &state),
            fetch,
            "{label}: carry diagnosis"
        );
        assert_eq!(
            find_unresolved_quest_gates(&wc, &g, from, &[to], opts, &state, &[]),
            Ok(journal),
            "{label}: stage diagnosis"
        );
    }
}

#[test]
fn many_targets_keep_shortcuts_duplicates_budget_and_deadline_distinct() {
    let wc = bake(5, 1, &[]);
    let graph = TransportGraph::default();
    let from = tile(0, 0, 0);
    let targets = [tile(2, 0, 0), tile(4, 0, 0), tile(2, 0, 0), from];
    let opts = FindOptions::default();
    let state = WorldState::empty();
    let at_three = find_many_with_avoid_bounded(&wc, &graph, from, &targets, opts, &state, &[], 3);
    assert_eq!(at_three.results()[0].as_ref().unwrap().settled_at, 3);
    assert_eq!(at_three.results()[2], at_three.results()[0]);
    assert_eq!(at_three.results()[1], Err(TargetError::BudgetExhausted));
    assert_eq!(at_three.results()[3].as_ref().unwrap().settled_at, 0);
    assert_eq!(at_three.route(0).unwrap().ticks, 1.0);
    assert_eq!(at_three.route(1), Err(TargetError::BudgetExhausted));
    assert_eq!(
        find_with_avoid_bounded(&wc, &graph, from, targets[1], opts, &state, &[], 3),
        Err(RouteError::BudgetExhausted)
    );
    let at_five = find_many_with_avoid_bounded(&wc, &graph, from, &targets, opts, &state, &[], 5);
    assert_eq!(at_five.results()[1].as_ref().unwrap().settled_at, 5);
    assert_eq!(at_five.route(1).unwrap().dest, targets[1]);
    assert_eq!(at_five.route(1).unwrap().ticks, 2.0);
    let empty = find_many_with(&wc, &graph, from, &[], opts, &state);
    assert!(empty.results().is_empty());
    assert_eq!(empty.settled(), 0);
    let deadline = find_many_with_avoid_bounded_until(
        &wc,
        &graph,
        from,
        &targets,
        opts,
        &state,
        &[],
        5,
        Some(Instant::now() - Duration::from_secs(1)),
    );
    assert!(!deadline.complete());
    assert_eq!(deadline.results()[1], Err(TargetError::NotSettled));
    assert_eq!(deadline.results()[3].as_ref().unwrap().ticks, 0.0);
    let origin_only = [from];
    let zero = find_many_with_avoid_bounded(&wc, &graph, from, &origin_only, opts, &state, &[], 0);
    assert_eq!(zero.results()[0].as_ref().unwrap().settled_at, 0);
    assert_eq!(zero.settled(), 0);
    let blocked_target = [tile(4, 4, 0)];
    let exhausted = find_many_with_avoid_bounded(
        &walled_5x5(),
        &graph,
        tile(0, 0, 0),
        &blocked_target,
        opts,
        &state,
        &[],
        1000,
    );
    assert_eq!(exhausted.results(), &[Err(TargetError::NoPath)]);
}

#[test]
fn first_target_search_stops_before_an_unreachable_sibling_floods_the_map() {
    let wc = bake(100, 1, &[]);
    let graph = TransportGraph::default();
    let from = tile(0, 0, 0);
    let targets = [tile(2, 0, 0), tile(200, 0, 0)];
    let search = find_first_with(
        &wc,
        &graph,
        from,
        &targets,
        FindOptions::default(),
        &WorldState::empty(),
    );

    assert_eq!(search.route().unwrap().dest, targets[0]);
    assert_eq!(
        search.settled(),
        3,
        "the cheapest goal ends the search before the unreachable target"
    );
    let scratch = search.scratch_capacities();
    assert!(scratch.distances < 16);
    assert!(scratch.predecessors < 16);
    assert!(scratch.settled < 16);
    assert!(scratch.heap < 16);
}

/// A goal whose only route needs more settles than the old scene-sized
/// first-goal cap (32,768) still routes.
#[test]
fn first_target_search_routes_a_goal_past_the_old_scene_cap() {
    const LEN: i32 = 40_002;
    let wc = bake(LEN as usize, 1, &[]);
    let goal = tile(LEN - 1, 0, 0);
    let search = find_first_with(
        &wc,
        &TransportGraph::default(),
        tile(0, 0, 0),
        &[goal],
        FindOptions::default(),
        &WorldState::empty(),
    );

    let route = search.route().expect("the corridor end is reachable");
    assert_eq!(route.dest, goal);
    assert_eq!(route.ticks, f64::from(LEN - 1) * 0.5);
}

/// A 3x3 room (x, z in 200..=202) sealed by a ring of blocked tiles in an
/// open 256x256 plane. With `door`, a worn-gated door (obj 2) at the ring
/// tile (199, 201) lands on (200, 201) from the west.
fn sealed_room(door: bool) -> (WorldCollision, TransportGraph) {
    let mut ring = Vec::new();
    for x in 199..=203 {
        for z in 199..=203 {
            if x == 199 || x == 203 || z == 199 || z == 203 {
                ring.push((x, z, CollisionFlag::SQ_BLOCKED as u32));
            }
        }
    }
    let mut graph = TransportGraph::default();
    if door {
        let at = tile(199, 201, 0);
        graph.at.entry(at).or_default().push(0);
        graph.edges.push(TransportEdge {
            kind: TransportKind::Door,
            at,
            to: tile(200, 201, 0),
            loc_id: 1,
            option: 1,
            ticks: 2,
            dir: None,
            open_loc_id: None,
            skill_req: vec![],
            item_req: vec![],
            quest_req: vec![],
            varp_req: vec![],
            worn_req: vec![2],
            members_req: false,
            wildy_cap: None,
            quest_gates: None,
        });
    }
    (bake(256, 256, &ring), graph)
}

/// Stands sealed in a small room are proven unreachable from their own
/// backward region instead of by flooding the 65,536-tile plane, for the
/// strict search and for the relaxed BankBudget diagnosis alike.
#[test]
fn first_target_search_proves_sealed_stands_unreachable_without_flooding() {
    let (wc, graph) = sealed_room(false);
    let from = tile(20, 20, 0);
    let stands = [tile(200, 201, 0), tile(201, 202, 0)];
    let opts = FindOptions {
        allow_bank_fetch: true,
        ..FindOptions::default()
    };
    let state = WorldState::empty();

    let strict = find_first_with(&wc, &graph, from, &stands, opts, &state);
    assert_eq!(strict.route().err(), Some(RouteError::NoPath));
    assert_eq!(strict.proof(), ReverseProof::Unreachable);
    assert!(
        strict.settled() < 64,
        "the room and its ring decide, not the plane: {}",
        strict.settled()
    );
    assert_eq!(
        find_missing_item_reqs(&wc, &graph, from, stands[0], opts, &state),
        None
    );
    let relaxed = super::first_search(
        &wc,
        &graph,
        from,
        &stands[..1],
        &[],
        opts,
        &state,
        super::Relax::CarryWorn,
        &[],
        super::NODE_BUDGET,
        super::NODE_BUDGET,
    );
    assert_eq!(relaxed.proof(), ReverseProof::Unreachable);
    assert!(relaxed.settled() < 64, "{}", relaxed.settled());
}

/// The strict proof honors the door's worn gate, so it closes at once; the
/// relaxed diagnosis crosses the door and names the missing worn obj; and a
/// search under what a session can fetch crosses it only when the obj is
/// banked, proving the room closed at once when it is not.
#[test]
fn worn_gated_room_is_proven_strictly_and_reached_only_with_the_obj_fetchable() {
    let (wc, graph) = sealed_room(true);
    let from = tile(20, 20, 0);
    let stands = [tile(202, 201, 0), tile(201, 202, 0)];
    let opts = FindOptions {
        allow_bank_fetch: true,
        ..FindOptions::default()
    };
    let state = WorldState::empty();

    let strict = find_first_with(&wc, &graph, from, &stands, opts, &state);
    assert_eq!(strict.route().err(), Some(RouteError::NoPath));
    assert_eq!(strict.proof(), ReverseProof::Unreachable);
    assert!(strict.settled() < 64, "{}", strict.settled());
    assert_eq!(
        find_missing_item_reqs(&wc, &graph, from, stands[0], opts, &state),
        Some(vec![MissingReq::WearAny { ids: vec![2] }])
    );

    let booth = [BankStand {
        name: "Bank booth".into(),
        tile: tile(10, 10, 0),
        access: BankAccess::Booth { op: 2 },
    }];
    let banked = fetchable_state(&state, &[(2, 1)], &booth);
    let search = find_first_with(&wc, &graph, from, &stands, opts, &banked);
    let route = search.route().expect("the banked obj opens the door");
    assert!(stands.contains(&route.dest));
    assert_eq!(
        missing_item_reqs(route, &state),
        vec![MissingReq::WearAny { ids: vec![2] }]
    );

    let elsewhere = fetchable_state(&state, &[(3, 1)], &booth);
    let search = find_first_with(&wc, &graph, from, &stands, opts, &elsewhere);
    assert_eq!(search.route().err(), Some(RouteError::NoPath));
    assert_eq!(search.proof(), ReverseProof::Unreachable);
    assert!(search.settled() < 64, "{}", search.settled());
}

/// Neither side decides: the start's corridor and the stand's backward
/// corridor are disjoint and longer than both budgets, as on the 289 bake
/// when a stand is fed by the unstamped upper planes. The search stops at
/// its budget instead of flooding the start's corridor, and a fallback goal
/// out of reach gets no second budget-limited search.
#[test]
fn first_target_search_stops_at_its_budget_when_the_proof_cannot_decide() {
    let len = FIRST_TARGET_BUDGET + FIRST_TARGET_BUDGET / 4;
    let walls: Vec<_> = (0..len as i32)
        .flat_map(|x| {
            [
                (x, 0, CollisionFlag::W_N as u32),
                (x, 1, CollisionFlag::W_S as u32),
            ]
        })
        .collect();
    let wc = bake(len, 2, &walls);
    let search = find_first_with_fallback(
        &wc,
        &TransportGraph::default(),
        tile(0, 0, 0),
        &[tile(len as i32 - 1, 1, 0)],
        &[tile(len as i32 - 2, 1, 0)],
        FindOptions::default(),
        &WorldState::empty(),
    );

    assert_eq!(search.route().err(), Some(RouteError::BudgetExhausted));
    assert_eq!(search.proof(), ReverseProof::Abandoned);
    assert!(
        search.settled() < len,
        "the budget must stop the search before it floods the {len}-tile corridor"
    );
    assert_eq!(
        search.fallback(),
        Some(&FallbackRoute::Failed(RouteError::BudgetExhausted))
    );
    assert!(search.settled() <= FIRST_TARGET_BUDGET);
    let scratch = search.scratch_capacities();
    assert!(scratch.reverse < len, "{scratch:?}");
}

/// A reachable preferred goal wins over a cheaper fallback goal, and the
/// fallback set answers only when no preferred goal routes.
#[test]
fn preferred_goal_beats_a_cheaper_fallback_goal() {
    let wc = bake(24, 1, &[]);
    let search = find_first_with_fallback(
        &wc,
        &TransportGraph::default(),
        tile(0, 0, 0),
        &[tile(20, 0, 0)],
        &[tile(1, 0, 0)],
        FindOptions::default(),
        &WorldState::empty(),
    );

    assert_eq!(search.route().map(|route| route.dest), Ok(tile(20, 0, 0)));
    assert!(search.fallback().is_none());
}

/// Preferred goals sealed in a room: a fallback goal the search settled on
/// the way answers at once. One past where the proof stopped the search is
/// left undecided, and a search over the fallback set alone finds it.
#[test]
fn sealed_preferred_goals_fall_back_to_the_cheapest_fallback_goal() {
    let (wc, graph) = sealed_room(false);
    let from = tile(20, 20, 0);
    let stands = [tile(200, 201, 0), tile(201, 202, 0)];
    let state = WorldState::empty();
    let opts = FindOptions::default();

    let near = find_first_with_fallback(
        &wc,
        &graph,
        from,
        &stands,
        &[tile(150, 150, 0), tile(21, 20, 0)],
        opts,
        &state,
    );
    assert_eq!(near.route().err(), Some(RouteError::NoPath));
    assert_eq!(near.proof(), ReverseProof::Unreachable);
    let Some(FallbackRoute::Routed(route)) = near.fallback() else {
        panic!(
            "the adjacent fallback goal settled first: {:?}",
            near.fallback()
        );
    };
    assert_eq!(route.dest, tile(21, 20, 0));
    assert!(near.settled() < 64, "{}", near.settled());

    let fallback = [tile(150, 150, 0)];
    let far = find_first_with_fallback(&wc, &graph, from, &stands, &fallback, opts, &state);
    assert_eq!(far.route().err(), Some(RouteError::NoPath));
    assert_eq!(far.fallback(), Some(&FallbackRoute::Undecided));
    assert!(far.settled() < 64, "{}", far.settled());
    let alone = find_first_with(&wc, &graph, from, &fallback, opts, &state);
    assert_eq!(alone.route().map(|route| route.dest), Ok(fallback[0]));
}

/// A backward proof that meets a teleport landing usable from the origin
/// lifts the unproven budget: the teleport costs more than walking the
/// whole plane, so Dijkstra settles the plane before taking it.
#[test]
fn first_target_proof_of_reachability_lifts_the_unproven_budget() {
    let (wc, _) = sealed_room(false);
    let graph = teleport(tile(200, 200, 0), 1_000, vec![], vec![]);
    let from = tile(20, 20, 0);
    let stands = [tile(202, 202, 0)];
    let opts = FindOptions {
        allow_teleports: true,
        ..FindOptions::default()
    };
    let state = WorldState::empty();
    let search = |reachable_budget| {
        super::first_search(
            &wc,
            &graph,
            from,
            &stands,
            &[],
            opts,
            &state,
            super::Relax::Strict,
            &[],
            64,
            reachable_budget,
        )
    };

    let lifted = search(super::NODE_BUDGET);
    assert_eq!(lifted.proof(), ReverseProof::Reachable);
    let route = lifted
        .route()
        .expect("the teleport reaches the sealed stand");
    assert_eq!(route.dest, stands[0]);
    assert!(lifted.settled() > 64);
    assert_eq!(search(64).route().err(), Some(RouteError::BudgetExhausted));
}

/// R6-B1: a fallback set keeps its own proof when the preferred budget stops
/// the search. On a 1024² plane the preferred goal lies on a plane no move
/// reaches, too large for its proof to close, so the stands stop at the
/// unproven budget. The radius tile sits in a sealed room whose only way in
/// is a teleport dearer than walking the whole plane: its proof meets the
/// landing at once, but it settles only after the plane, past the stands'
/// budget. It is left undecided rather than failed, and the search over it
/// alone, under the budget its proof lifts, routes to it.
#[test]
fn fallback_proven_reachable_past_the_preferred_budget_still_routes() {
    const SIZE: i32 = 1024;
    let mut ring = Vec::new();
    for x in 600..=604 {
        for z in 600..=604 {
            if x == 600 || x == 604 || z == 600 || z == 604 {
                ring.push((x, z, CollisionFlag::SQ_BLOCKED as u32));
            }
        }
    }
    let wc = bake(SIZE as usize, SIZE as usize, &ring);
    let graph = teleport(tile(601, 601, 0), 1_000, vec![], vec![]);
    let from = tile(20, 20, 0);
    let stands = [tile(20, 20, 1)];
    let fallback = [tile(603, 603, 0)];
    let opts = FindOptions {
        allow_teleports: true,
        ..FindOptions::default()
    };
    let state = WorldState::empty();

    let shared = find_first_with_fallback(&wc, &graph, from, &stands, &fallback, opts, &state);
    assert_eq!(shared.route().err(), Some(RouteError::BudgetExhausted));
    assert_eq!(shared.settled(), FIRST_TARGET_BUDGET);
    assert_eq!(shared.fallback(), Some(&FallbackRoute::Undecided));

    let alone = find_first_with(&wc, &graph, from, &fallback, opts, &state);
    assert_eq!(alone.proof(), ReverseProof::Reachable);
    assert_eq!(alone.route().map(|route| route.dest), Ok(fallback[0]));
    assert!(
        alone.settled() > (SIZE * SIZE) as usize - 25,
        "{}",
        alone.settled()
    );
}

/// The fallback set's budget, lifted by its own proof, decides it at the
/// settles the preferred budget spent: lifted to exactly those settles, a
/// search over it alone would stop there, so it has failed; one settle more
/// leaves it undecided.
#[test]
fn fallback_fails_only_when_its_own_budget_is_spent_with_the_preferred() {
    let (wc, _) = sealed_room(false);
    let graph = teleport(tile(200, 200, 0), 1_000, vec![], vec![]);
    let from = tile(20, 20, 0);
    let stands = [tile(20, 20, 1)];
    let fallback = [tile(202, 202, 0)];
    let opts = FindOptions {
        allow_teleports: true,
        ..FindOptions::default()
    };
    let state = WorldState::empty();
    let search = |reachable_budget| {
        super::first_search(
            &wc,
            &graph,
            from,
            &stands,
            &fallback,
            opts,
            &state,
            super::Relax::Strict,
            &[],
            64,
            reachable_budget,
        )
    };

    let spent = search(64);
    assert_eq!(spent.settled(), 64);
    assert_eq!(
        spent.fallback(),
        Some(&FallbackRoute::Failed(RouteError::BudgetExhausted))
    );
    assert_eq!(search(65).fallback(), Some(&FallbackRoute::Undecided));
}

/// A shared first-goal search and a search over its fallback set alone,
/// under the same gates and budgets: the shared search's preferred answer is
/// the preferred-only search's, and its fallback answer is what the
/// fallback-only search reports, or undecided where that search needs more
/// settles than the shared one spent.
#[allow(clippy::too_many_arguments)] // the first_search surface
fn assert_fallback_matches_alone(
    wc: &WorldCollision,
    graph: &TransportGraph,
    from: WorldTile,
    targets: &[WorldTile],
    fallback: &[WorldTile],
    opts: FindOptions,
    state: &WorldState,
    budget: usize,
    reachable_budget: usize,
) -> Option<FallbackRoute> {
    let search = |targets: &[WorldTile], fallback: &[WorldTile]| {
        super::first_search(
            wc,
            graph,
            from,
            targets,
            fallback,
            opts,
            state,
            super::Relax::Strict,
            &[],
            budget,
            reachable_budget,
        )
    };
    let shared = search(targets, fallback);
    let preferred = search(targets, &[]);
    let summary = |route: Result<&crate::router::Route, RouteError>| {
        route.map(|route| (route.dest, route.ticks))
    };
    assert_eq!(summary(shared.route()), summary(preferred.route()));
    assert_eq!(shared.settled(), preferred.settled());
    let alone = search(fallback, &[]);
    match shared.fallback() {
        None => assert!(shared.route().is_ok() || fallback.is_empty()),
        Some(FallbackRoute::Routed(route)) => {
            assert_eq!(Ok((route.dest, route.ticks)), summary(alone.route()));
        }
        Some(FallbackRoute::Failed(error)) => assert_eq!(Err(*error), summary(alone.route())),
        Some(FallbackRoute::Undecided) => assert!(
            alone.settled() >= shared.settled(),
            "undecided at {} but alone decided at {}",
            shared.settled(),
            alone.settled()
        ),
    }
    shared.fallback().cloned()
}

/// A fallback goal settling past the fallback set's unproven budget, in a
/// search the preferred proof has lifted, counts only if the fallback's own
/// proof showed a goal reachable within that budget. The preferred goal is
/// reached only by a teleport dearer than walking the whole plane, so its
/// proof lifts the search while it settles last; the fallback goal is 30
/// tiles off, well past its budget of 64, and its proof cannot reach the
/// origin in 64 steps: a search over it alone stops at the budget, and so
/// does the set.
#[test]
fn late_fallback_goal_is_refused_past_its_own_unproven_budget() {
    let wc = bake(128, 128, &[]);
    let graph = teleport(tile(20, 20, 1), 1_000, vec![], vec![]);
    let from = tile(20, 20, 0);
    let preferred = [tile(20, 20, 1)];
    let opts = FindOptions {
        allow_teleports: true,
        ..FindOptions::default()
    };
    let state = WorldState::empty();

    let far = assert_fallback_matches_alone(
        &wc,
        &graph,
        from,
        &preferred,
        &[tile(50, 20, 0)],
        opts,
        &state,
        64,
        10_000,
    );
    assert_eq!(
        far,
        Some(FallbackRoute::Failed(RouteError::BudgetExhausted))
    );
    // A fallback goal within its budget routes without a proof.
    let near = assert_fallback_matches_alone(
        &wc,
        &graph,
        from,
        &preferred,
        &[tile(22, 20, 0)],
        opts,
        &state,
        64,
        10_000,
    );
    assert!(matches!(near, Some(FallbackRoute::Routed(_))), "{near:?}");
}

/// Seeded differential over random worlds: walls with sealed pockets, a
/// second plane, gated and ungated ladders and teleports, random goal sets
/// and budgets. The shared search's fallback answer always matches a search
/// over the fallback alone.
#[test]
fn shared_fallback_matches_a_fallback_only_search_on_random_worlds() {
    const SIZE: i32 = 40;
    let mut seed = 0x9e37_79b9_7f4a_7c15_u64;
    let mut next = move |bound: u64| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed % bound
    };
    let edge = |kind, at, to, ticks, item_req| TransportEdge {
        kind,
        at,
        to,
        loc_id: 1,
        option: 1,
        ticks,
        dir: None,
        open_loc_id: None,
        skill_req: vec![],
        item_req,
        quest_req: vec![],
        varp_req: vec![],
        worn_req: vec![],
        members_req: false,
        wildy_cap: None,
        quest_gates: None,
    };
    let mut outcomes = HashMap::new();
    for _ in 0..400 {
        let mut extras = Vec::new();
        let density = 10 + next(35);
        for x in 0..SIZE {
            for z in 0..SIZE {
                if next(100) < density {
                    extras.push((x, z, CollisionFlag::SQ_BLOCKED as u32));
                }
            }
        }
        let wc = bake(SIZE as usize, SIZE as usize, &extras);
        let random_tile = |next: &mut dyn FnMut(u64) -> u64| {
            tile(
                next(SIZE as u64) as i32,
                next(SIZE as u64) as i32,
                (next(4) == 0) as i32,
            )
        };
        let mut graph = TransportGraph::default();
        for _ in 0..next(4) {
            let at = random_tile(&mut next);
            let to = random_tile(&mut next);
            let gate = if next(3) == 0 { vec![(995, 1)] } else { vec![] };
            graph.at.entry(at).or_default().push(graph.edges.len());
            let ticks = 1 + next(20) as i32;
            graph
                .edges
                .push(edge(TransportKind::Ladder, at, to, ticks, gate));
        }
        for _ in 0..next(3) {
            let to = random_tile(&mut next);
            let gate = if next(3) == 0 { vec![(995, 1)] } else { vec![] };
            // Up to dearer than walking every tile, so a preferred goal
            // behind one can settle after the fallback's budget.
            let ticks = 1 + next(1_600) as i32;
            graph.teleports.push(edge(
                TransportKind::Teleport,
                tile(0, 0, 0),
                to,
                ticks,
                gate,
            ));
        }
        let from = tile(next(SIZE as u64) as i32, next(SIZE as u64) as i32, 0);
        let mut targets: Vec<_> = (0..1 + next(3)).map(|_| random_tile(&mut next)).collect();
        // Often a teleport landing, which its proof shows reachable at once.
        if let Some(teleport) = graph.teleports.first().filter(|_| next(2) == 0) {
            targets.push(teleport.to);
        }
        let fallback: Vec<_> = (0..1 + next(6)).map(|_| random_tile(&mut next)).collect();
        let opts = FindOptions {
            allow_teleports: next(2) == 0,
            ..FindOptions::default()
        };
        let budget = 1 + next(300) as usize;
        let reachable_budget = budget + next(4_000) as usize;
        let outcome = assert_fallback_matches_alone(
            &wc,
            &graph,
            from,
            &targets,
            &fallback,
            opts,
            &WorldState::empty(),
            budget,
            reachable_budget,
        );
        let kind = match outcome {
            None => "preferred",
            Some(FallbackRoute::Routed(_)) => "routed",
            Some(FallbackRoute::Failed(RouteError::NoPath)) => "no-path",
            Some(FallbackRoute::Failed(_)) => "budget",
            Some(FallbackRoute::Undecided) => "undecided",
        };
        *outcomes.entry(kind).or_insert(0) += 1;
    }
    // The worlds exercise every fallback answer.
    for kind in ["preferred", "routed", "no-path", "budget", "undecided"] {
        assert!(outcomes.get(kind).is_some_and(|&n| n > 0), "{outcomes:?}");
    }
}

#[test]
fn bank_budget_accepts_the_goal_after_500000_predecessors() {
    // A 1-wide corridor guarantees that the goal is pop 500001.
    let wc = bake(500_001, 1, &[]);
    let graph = TransportGraph::default();
    let from = tile(0, 0, 0);
    let goal = tile(500_000, 0, 0);
    let opts = FindOptions::default();
    let state = WorldState::empty();
    let targets = [goal];
    let before = find_many_with_avoid_bounded(
        &wc,
        &graph,
        from,
        &targets,
        opts,
        &state,
        &[],
        BANK_TARGET_BUDGET - 1,
    );
    assert_eq!(before.results(), &[Err(TargetError::BudgetExhausted)]);
    let at = find_many_with_avoid_bounded(
        &wc,
        &graph,
        from,
        &targets,
        opts,
        &state,
        &[],
        BANK_TARGET_BUDGET,
    );
    assert_eq!(at.results()[0].as_ref().unwrap().settled_at, 500_001);
    assert_eq!(at.results()[0].as_ref().unwrap().ticks, 250_000.0);
    assert_eq!(
        find_with_avoid_bounded(
            &wc,
            &graph,
            from,
            goal,
            opts,
            &state,
            &[],
            BANK_TARGET_BUDGET,
        )
        .unwrap()
        .ticks,
        250_000.0
    );
}

#[test]
fn many_targets_preserve_blocked_origin_and_exact_blocked_destinations() {
    let blocked = CollisionFlag::SQ_BLOCKED as u32 | CollisionFlag::WALK_BLOCK_FLAGS as u32;
    let wc = bake(3, 1, &[(0, 0, blocked), (2, 0, blocked)]);
    let graph = TransportGraph::default();
    let from = tile(0, 0, 0);
    let targets = [from, tile(1, 0, 0), tile(2, 0, 0)];
    let many = find_many_with(
        &wc,
        &graph,
        from,
        &targets,
        FindOptions::default(),
        &WorldState::empty(),
    );
    assert_eq!(many.route(0).unwrap().ticks, 0.0);
    assert_eq!(many.route(1).unwrap().ticks, 0.5);
    assert_eq!(many.results()[2], Err(TargetError::NoPath));
}

#[test]
fn many_targets_share_admitted_teleports_and_essence_returns() {
    let wc = walled_5x5();
    let dest = tile(4, 4, 0);
    let graph = teleport(dest, 3, vec![(6, 25)], vec![(554, 1), (556, 3), (563, 1)]);
    let from = tile(0, 0, 0);
    let targets = [tile(1, 0, 0), dest];
    for (state, opts) in [
        (
            WorldState::empty(),
            FindOptions {
                allow_teleports: true,
                ..FindOptions::default()
            },
        ),
        (
            spell_state(),
            FindOptions {
                allow_teleports: true,
                ..FindOptions::default()
            },
        ),
        (spell_state(), FindOptions::default()),
    ] {
        let many = find_many_with(&wc, &graph, from, &targets, opts, &state);
        for (index, &target) in targets.iter().enumerate() {
            match find_with(&wc, &graph, from, target, opts, &state) {
                Ok(route) => assert_eq!(many.route(index).unwrap().ticks, route.ticks),
                Err(error) => assert_eq!(many.results()[index], Err(error.into())),
            }
        }
    }
    let wc = mine_bake();
    let graph = TransportGraph::default();
    let from = tile(2912, 4833, 0);
    let targets = [tile(3253, 3401, 0), tile(3106, 9572, 0)];
    for essence in [None, crate::essence::essence_session_for_wizard(553)] {
        let opts = FindOptions {
            essence,
            ..FindOptions::default()
        };
        let many = find_many_with(&wc, &graph, from, &targets, opts, &WorldState::empty());
        for (index, &target) in targets.iter().enumerate() {
            match find_with(&wc, &graph, from, target, opts, &WorldState::empty()) {
                Ok(route) => assert_eq!(many.route(index).unwrap().ticks, route.ticks),
                Err(error) => assert_eq!(many.results()[index], Err(error.into())),
            }
        }
    }
}

#[test]
fn bank_targets_match_independent_find_with_on_real_289_pack() {
    let Some(world) = crate::world::NavWorld::load_default_pack_or_skip() else {
        return;
    };
    let data = api::game_data::for_revision(client::io::ClientRevision::R289).unwrap();
    world.bind_named_bank_facts(&data).unwrap();
    let facts = world.named_bank_facts().unwrap();
    let placements = &data.bank_placements().unwrap().rows;
    for bank in facts.banks().iter().filter(|bank| bank.routable) {
        assert!(
            world.collision.standable(bank.tile),
            "blocked stand: {}",
            bank.name
        );
        assert!(
            placements.iter().any(|p| {
                if p.name != bank.name || p.level != bank.tile.level {
                    return false;
                }
                let dx = (p.x - bank.tile.x)
                    .max(bank.tile.x - (p.x + p.width - 1))
                    .max(0);
                let dz = (p.z - bank.tile.z)
                    .max(bank.tile.z - (p.z + p.length - 1))
                    .max(0);
                dx.max(dz) == 1
            }),
            "{} needs a real adjacent selected-content access",
            bank.name
        );
    }
    let named = |name: &str| {
        facts
            .banks()
            .iter()
            .find(|bank| bank.name == name)
            .unwrap()
            .tile
    };
    assert_eq!(named("Varrock West"), tile(3185, 3440, 0));
    assert_ne!(
        world.collision.walkable_word(3185, 3440, 0) & CollisionFlag::W_S as u32,
        0
    );
    assert_eq!(
        named("Ardougne East"),
        tile(2655, 3283, 0),
        "customer side, not the bankers' aisle"
    );
    let raw: Vec<_> = world.banks().iter().map(|bank| bank.tile).collect();
    assert!(
        raw.len() >= 64,
        "289 pack must contain the 64-bank workload"
    );
    let resolved: Vec<_> = facts.banks().iter().filter(|bank| bank.routable).collect();
    let mut targets = raw.clone();
    targets.extend(resolved.iter().map(|bank| bank.tile));
    let empty = WorldState::empty();
    let members = WorldState {
        map_members: true,
        quests: HashSet::from([
            "Prince Ali Rescue".into(),
            "Rune Mysteries".into(),
            "Lost City".into(),
            "Shilo Village".into(),
        ]),
        inv: HashMap::from([(995, 10000), (554, 1000), (556, 1000), (563, 1000)]),
        stats: HashMap::from([(6, 99), (10, 99)]),
        ..WorldState::empty()
    };
    for (label, from, state) in [
        ("lumbridge", tile(3222, 3218, 0), &empty),
        ("dwarven_mine", tile(3016, 9840, 0), &members),
    ] {
        // Unbounded no-avoid rows compare directly to native find_with, not a
        // differently capped search. Raw booths and actual C1 stands coexist.
        let costs = compare_real_bank_targets(
            &world,
            label,
            from,
            &targets,
            FindOptions::default(),
            state,
            &[],
            None,
        );
        for name in ["Varrock West", "Edgeville", "Falador East"] {
            let index = resolved.iter().position(|bank| bank.name == name).unwrap();
            assert!(
                costs[raw.len() + index].is_ok(),
                "{label}: {name} must be a reachable C1 stand"
            );
        }
        if label == "dwarven_mine" {
            let cost = |name| {
                costs[raw.len() + resolved.iter().position(|bank| bank.name == name).unwrap()]
                    .unwrap()
            };
            assert!(
                cost("Falador East") < cost("Edgeville"),
                "recorded dungeon witness must prefer walking over air proximity"
            );
            eprintln!(
                "dungeon witness: Falador East={} Edgeville={}",
                cost("Falador East"),
                cost("Edgeville")
            );
        }
    }
    let probes = [
        named("Falador East"),
        named("Varrock West"),
        named("Edgeville"),
        named("Shilo Village"),
        named("Zanaris"),
    ];
    let inside_avoid = [AvoidRect {
        min_x: 3015,
        max_x: 3017,
        min_z: 9839,
        max_z: 9841,
        level: Some(0),
    }];
    let outside_avoid = [AvoidRect {
        min_x: 3223,
        max_x: 3226,
        min_z: 3215,
        max_z: 3221,
        level: Some(0),
    }];
    let essence = crate::essence::essence_session_for_wizard(553);
    for (label, from, state, opts, avoid, budget) in [
        (
            "empty",
            tile(3016, 9840, 0),
            &empty,
            FindOptions::default(),
            &[][..],
            BANK_TARGET_BUDGET,
        ),
        (
            "members-wilderness",
            tile(3016, 9840, 0),
            &members,
            FindOptions {
                allow_wilderness: true,
                ..Default::default()
            },
            &[][..],
            BANK_TARGET_BUDGET,
        ),
        (
            "origin-inside-avoid",
            tile(3016, 9840, 0),
            &members,
            FindOptions::default(),
            &inside_avoid[..],
            BANK_TARGET_BUDGET,
        ),
        (
            "origin-outside-avoid",
            tile(3222, 3218, 0),
            &members,
            FindOptions::default(),
            &outside_avoid[..],
            BANK_TARGET_BUDGET,
        ),
        (
            "teleports",
            tile(3222, 3218, 0),
            &members,
            FindOptions {
                allow_teleports: true,
                ..Default::default()
            },
            &[][..],
            BANK_TARGET_BUDGET,
        ),
        (
            "essence-no-return",
            tile(2912, 4833, 0),
            &members,
            FindOptions::default(),
            &[][..],
            BANK_TARGET_BUDGET,
        ),
        (
            "essence-return",
            tile(2912, 4833, 0),
            &members,
            FindOptions {
                essence,
                ..Default::default()
            },
            &[][..],
            BANK_TARGET_BUDGET,
        ),
        (
            "budget",
            tile(3222, 3218, 0),
            &empty,
            FindOptions::default(),
            &[][..],
            50,
        ),
    ] {
        let costs = compare_real_bank_targets(
            &world,
            label,
            from,
            &probes,
            opts,
            state,
            avoid,
            Some(budget),
        );
        if label == "budget" {
            assert!(costs.contains(&Err(TargetError::BudgetExhausted)));
        }
        if label == "essence-no-return" {
            assert!(costs.iter().all(Result::is_err));
        }
        if label == "essence-return" {
            assert!(
                costs[1].is_ok(),
                "captured Aubury return must reach Varrock West"
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn compare_real_bank_targets(
    world: &crate::world::NavWorld,
    label: &str,
    from: WorldTile,
    targets: &[WorldTile],
    opts: FindOptions,
    state: &WorldState,
    avoid: &[AvoidRect],
    budget: Option<usize>,
) -> Vec<Result<f64, TargetError>> {
    let opts = FindOptions {
        zones: crate::zones::ZoneExempt::all(),
        ..opts
    };
    let started = Instant::now();
    let many = match budget {
        Some(budget) => find_many_with_avoid_bounded(
            &world.collision,
            &world.graph,
            from,
            targets,
            opts,
            state,
            avoid,
            budget,
        ),
        None => find_many_with(&world.collision, &world.graph, from, targets, opts, state),
    };
    let shared = started.elapsed();
    let started = Instant::now();
    let mut costs = Vec::with_capacity(targets.len());
    let mut teleported = false;
    let mut returned = false;
    for (index, &target) in targets.iter().enumerate() {
        let independent = match budget {
            Some(budget) => find_with_avoid_bounded(
                &world.collision,
                &world.graph,
                from,
                target,
                opts,
                state,
                avoid,
                budget,
            ),
            None => find_with(&world.collision, &world.graph, from, target, opts, state),
        };
        let cost = independent
            .as_ref()
            .map(|route| route.ticks)
            .map_err(|&error| error.into());
        assert_eq!(
            many.results()[index]
                .as_ref()
                .map(|cost| cost.ticks)
                .map_err(|&error| error),
            cost,
            "{label}: target {index} {target:?}"
        );
        if independent.is_ok() {
            let actual = many.route(index).unwrap();
            assert_eq!(actual.dest, target);
            validate_real_route(&world.collision, &world.graph, &actual, state, opts, avoid);
            teleported |= actual.legs.iter().any(|leg| matches!(leg, Leg::Transport { edge } if edge.kind == TransportKind::Teleport));
            returned |= actual.legs.iter().any(|leg| matches!(leg, Leg::Transport { edge } if edge.kind == TransportKind::EssenceExit));
        }
        costs.push(cost);
    }
    if label == "teleports" {
        assert!(
            teleported,
            "matrix must actually use an admitted native teleport"
        );
    }
    if label == "essence-return" {
        assert!(returned, "matrix must actually use the captured return");
    }
    eprintln!(
        "bank real 289 {label}: targets={} successes={} settled={} shared={:?} independent={:?}",
        targets.len(),
        costs.iter().filter(|cost| cost.is_ok()).count(),
        many.settled(),
        shared,
        started.elapsed()
    );
    costs
}

#[test]
fn first_target_rc_booth_stays_in_the_frozen_search_ballpark_on_real_289_pack() {
    let Some(world) = crate::world::NavWorld::load_default_pack_or_skip() else {
        return;
    };
    let from = tile(2653, 3289, 0);
    let targets = [
        tile(2655, 3286, 0),
        tile(2657, 3286, 0),
        tile(2656, 3287, 0),
    ];
    let state = WorldState {
        map_members: true,
        ..WorldState::empty()
    };
    let started = Instant::now();
    let search = find_first_with(
        &world.collision,
        &world.graph,
        from,
        &targets,
        FindOptions {
            allow_wilderness: true,
            allow_bank_fetch: true,
            zones: crate::zones::ZoneExempt::all(),
            ..FindOptions::default()
        },
        &state,
    );
    let elapsed = started.elapsed();
    let scratch = search.scratch_capacities();
    eprintln!(
        "RC booth first-goal: elapsed={elapsed:?}, settled={}, scratch capacities={scratch:?}",
        search.settled()
    );

    let route = search.route().expect("diagnosis stand is reachable");
    assert_eq!(route.dest, targets[0]);
    assert_eq!(route.ticks, 12.5);
    assert!(
        search.settled() < 5_000,
        "first-goal search must not flood the packed world"
    );
    assert!(
        scratch.distances + scratch.predecessors + scratch.settled + scratch.heap < 16_384,
        "first-goal scratch must stay near the short route, got {scratch:?}"
    );
}

#[test]
fn lumbridge_castle_stairs_route_to_first_floor_wheel_on_real_289_pack() {
    let Some(world) = crate::world::NavWorld::load_default_pack_or_skip() else {
        return;
    };
    let state = WorldState::empty();
    let opts = FindOptions {
        zones: crate::zones::ZoneExempt::all(),
        ..FindOptions::default()
    };
    let wheel = tile(3209, 3212, 1);
    assert!(
        !world.collision.standable(wheel),
        "wheel occupies its target"
    );
    let dest = tile(3209, 3213, 1);
    let edge = world
        .graph
        .edges
        .iter()
        .find(|edge| edge.loc_id == 1738 && edge.at == tile(3204, 3207, 0))
        .expect("real Lumbridge south staircase");
    eprintln!("Lumbridge stairs: {edge:?}; allowed={}", state.allows(edge));
    assert!(state.allows(edge), "ordinary castle stairs have no gates");
    assert!(world.collision.standable(edge.to));
    let index = world
        .graph
        .edges
        .iter()
        .position(|candidate| candidate == edge)
        .unwrap();
    assert!(!world
        .graph
        .admissible_from(&world.collision, index, tile(3205, 3206, 0)));
    assert!(world
        .graph
        .admissible_from(&world.collision, index, tile(3205, 3209, 0)));
    for from in [tile(3205, 3206, 0), tile(3215, 3212, 0)] {
        let route = find_with(&world.collision, &world.graph, from, dest, opts, &state)
            .expect("ground floor and courtyard route to the wheel");
        assert_eq!(route.dest, dest, "preserve the exact operable wheel stand");
        eprintln!("Lumbridge from {from:?}: {route:?}");
        validate_real_route(&world.collision, &world.graph, &route, &state, opts, &[]);
        assert!(route.legs.iter().any(|leg| matches!(
            leg,
            Leg::Transport { edge } if edge.kind == TransportKind::Stairs
                && edge.at.level == 0 && edge.to.level == 1
        )));
    }
}

fn real_members_state() -> WorldState {
    WorldState {
        map_members: true,
        ..WorldState::empty()
    }
}

/// Members with every skill at 99, so only item/worn gates can block.
fn real_maxed_state() -> WorldState {
    WorldState {
        stats: (0..25).map(|skill| (skill, 99)).collect(),
        ..real_members_state()
    }
}

fn real_resilient_opts() -> FindOptions {
    FindOptions {
        allow_wilderness: true,
        allow_bank_fetch: true,
        zones: crate::zones::ZoneExempt::all(),
        ..FindOptions::default()
    }
}

/// Reachable in-scene stands past the old 32,768-settle cap route to the
/// stand: Falador's west-wall bank from outside the wall (a detour through
/// the city gate) and a long Wilderness web detour.
#[test]
fn first_target_stands_past_the_old_scene_cap_route_on_real_289_pack() {
    let Some(world) = crate::world::NavWorld::load_default_pack_or_skip() else {
        return;
    };
    let state = real_members_state();
    let falador = [
        tile(2944, 3367, 0),
        tile(2945, 3366, 0),
        tile(2945, 3368, 0),
    ];
    for from in [tile(2929, 3365, 0), tile(2930, 3351, 0)] {
        for opts in [real_resilient_opts(), FindOptions::default()] {
            let search =
                find_first_with(&world.collision, &world.graph, from, &falador, opts, &state);
            let route = search.route().expect("the Falador booth stand routes");
            assert_eq!(route.dest, tile(2945, 3368, 0));
            let independent = find_with(
                &world.collision,
                &world.graph,
                from,
                route.dest,
                opts,
                &state,
            )
            .expect("the Falador booth stand routes independently");
            assert_eq!(route.ticks, independent.ticks);
        }
    }
    let web = [tile(3157, 3950, 0), tile(3156, 3949, 0)];
    let search = find_first_with(
        &world.collision,
        &world.graph,
        tile(3148, 3943, 0),
        &web,
        real_resilient_opts(),
        &real_maxed_state(),
    );
    let route = search.route().expect("the web's far-side stand routes");
    assert_eq!(route.dest, tile(3156, 3949, 0));
    let independent = find_with(
        &world.collision,
        &world.graph,
        tile(3148, 3943, 0),
        route.dest,
        real_resilient_opts(),
        &real_maxed_state(),
    )
    .expect("the web's far-side stand routes independently");
    assert_eq!(route.ticks, independent.ticks);
}

/// Tree Gnome Stronghold middle booth: its south stand is an 11-tile pocket,
/// proven unreachable at once; its north stand's backward region climbs
/// ladders into the unstamped upper planes, so the budget alone bounds the
/// pair, far below the 2,901,702-node flood of everything reachable.
#[test]
fn stronghold_middle_booth_search_is_bounded_on_real_289_pack() {
    let Some(world) = crate::world::NavWorld::load_default_pack_or_skip() else {
        return;
    };
    let state = real_members_state();
    let opts = real_resilient_opts();
    let from = tile(2452, 3481, 1);
    let south = tile(2449, 3480, 1);
    let north = tile(2449, 3482, 1);

    let pocket = find_first_with(&world.collision, &world.graph, from, &[south], opts, &state);
    assert_eq!(pocket.route().err(), Some(RouteError::NoPath));
    assert_eq!(pocket.proof(), ReverseProof::Unreachable);
    assert!(pocket.settled() < 64, "{}", pocket.settled());

    let stands = [south, north];
    let search = find_first_with(&world.collision, &world.graph, from, &stands, opts, &state);
    assert_eq!(search.route().err(), Some(RouteError::BudgetExhausted));
    assert!(search.settled() <= FIRST_TARGET_BUDGET);
}

/// The Varrock-sewer web: the strict search proves the stands behind the web
/// unreachable from their small backward region, and with the knife banked a
/// search under what a session can fetch reaches a stand needing only it.
#[test]
fn sewer_web_stands_are_proven_then_reached_with_the_banked_knife_on_real_289_pack() {
    let Some(world) = crate::world::NavWorld::load_default_pack_or_skip() else {
        return;
    };
    let state = real_maxed_state();
    let opts = real_resilient_opts();
    let from = tile(3202, 9909, 0);
    let stands = [tile(3209, 9891, 0), tile(3210, 9892, 0)];

    let strict = find_first_with(&world.collision, &world.graph, from, &stands, opts, &state);
    assert_eq!(strict.route().err(), Some(RouteError::NoPath));
    assert_eq!(strict.proof(), ReverseProof::Unreachable);
    assert!(strict.settled() < 2_000, "{}", strict.settled());
    let fetchable = fetchable_state(&state, &[(946, 1)], world.banks());
    let search = find_first_with(
        &world.collision,
        &world.graph,
        from,
        &stands,
        opts,
        &fetchable,
    );
    let route = search.route().expect("the banked knife opens the web");
    assert!(stands.contains(&route.dest));
    assert_eq!(
        missing_item_reqs(route, &state),
        vec![MissingReq::Carry { id: 946, count: 1 }]
    );
}

fn validate_real_route(
    collision: &WorldCollision,
    graph: &TransportGraph,
    route: &crate::router::Route,
    state: &WorldState,
    opts: FindOptions,
    avoid: &[AvoidRect],
) {
    let mut sum = 0.0;
    let mut current = None;
    for leg in &route.legs {
        match leg {
            Leg::Walk { tiles } => {
                if let Some(previous) = current {
                    assert_eq!(tiles[0], previous);
                }
                for step in tiles.windows(2) {
                    let d = (step[1].x - step[0].x, step[1].z - step[0].z);
                    assert!(step_ok(collision, step[0], d), "invalid walk {step:?}");
                    assert!(super::wildy_step_ok(
                        graph,
                        step[0],
                        step[1],
                        opts.allow_wilderness
                    ));
                    assert!(
                        super::tile_in_any_avoid(step[0], avoid)
                            || !super::tile_in_any_avoid(step[1], avoid)
                    );
                    sum += 0.5;
                }
                current = tiles.last().copied();
            }
            Leg::Transport { edge } => {
                let previous = current.expect("transport follows walk");
                assert!(state.allows(edge));
                match edge.kind {
                    TransportKind::Teleport => {
                        assert!(opts.allow_teleports && graph.teleports.contains(edge));
                    }
                    TransportKind::EssenceExit => {
                        let session = opts
                            .essence
                            .as_ref()
                            .expect("return must use captured entry wizard");
                        assert!(crate::essence::ESSENCE_MINE_PORTALS.contains(&edge.at));
                        assert_eq!(*edge, crate::essence::essence_return_edge(edge.at, session));
                    }
                    _ => assert!(
                        graph.edges.contains(edge),
                        "transport must belong to the bound graph"
                    ),
                }
                if edge.kind != TransportKind::Teleport {
                    assert!(collision.standable(previous));
                    assert_eq!(previous.level, edge.at.level);
                    if edge.kind == TransportKind::EssenceExit {
                        assert!(
                            (previous.x - edge.at.x)
                                .abs()
                                .max((previous.z - edge.at.z).abs())
                                <= 1
                        );
                    } else {
                        let index = graph
                            .edges
                            .iter()
                            .position(|candidate| candidate == edge)
                            .unwrap();
                        assert!(
                            graph.admissible_from(collision, index, previous),
                            "transport cannot operate from {previous:?}: {edge:?}"
                        );
                    }
                }
                assert!(super::wildy_step_ok(
                    graph,
                    previous,
                    edge.to,
                    opts.allow_wilderness
                ));
                if edge.kind == TransportKind::Teleport {
                    assert!(
                        graph.teleport_legal_from(previous, edge),
                        "teleport from {previous:?} exceeds packed cap"
                    );
                }
                assert!(
                    super::tile_in_any_avoid(previous, avoid)
                        || !super::tile_in_any_avoid(edge.to, avoid)
                );
                sum += edge.ticks as f64;
                current = Some(edge.to);
            }
        }
    }
    assert_eq!(current, Some(route.dest));
    assert_eq!(sum, route.ticks);
}

/// The `allow_bank_fetch` opt-in must not insert a bank leg or relax
/// an item req: with the flag on and no coins, the toll edge stays
/// unusable and the search is still `NoPath`. The BankBudget session
/// lives OUTSIDE the router — a bare `find_with` (flag on, no
/// session planned) never fetches.
#[test]
fn allow_bank_fetch_does_not_relax_item_reqs() {
    let wc = walled_5x5();
    let g = toll_graph();
    assert!(
        matches!(
            find_with(
                &wc,
                &g,
                tile(0, 0, 0),
                tile(4, 4, 0),
                FindOptions {
                    allow_bank_fetch: true,
                    ..FindOptions::default()
                },
                &WorldState::empty(),
            ),
            Err(RouteError::NoPath)
        ),
        "allow_bank_fetch alone must not fetch: no coins still means no route"
    );
    // The flag must not BLOCK a state-proven edge either — it only
    // opts the caller into the session, and this state proves the
    // toll on its own.
    let rich = WorldState {
        inv: HashMap::from([(995, 10)]),
        ..WorldState::default()
    };
    assert!(
        find_with(
            &wc,
            &g,
            tile(0, 0, 0),
            tile(4, 4, 0),
            FindOptions {
                allow_bank_fetch: true,
                ..FindOptions::default()
            },
            &rich,
        )
        .is_ok(),
        "the flag never blocks a state-proven edge"
    );
}

/// The BankBudget diagnosis: `find_missing_item_reqs` re-runs the
/// search with only the carry/wear gates ignored and reports exactly
/// the facts the strict search could not prove. `find_with` itself
/// never relaxes — this arm is the session's.
#[test]
fn find_missing_item_reqs_reports_only_unproven_carry_and_wear() {
    let wc = walled_5x5();
    let g = toll_graph();
    let from = tile(0, 0, 0);
    let to = tile(4, 4, 0);
    assert_eq!(
        find_missing_item_reqs(
            &wc,
            &g,
            from,
            to,
            FindOptions::default(),
            &WorldState::empty(),
        ),
        Some(vec![MissingReq::Carry { id: 995, count: 10 }]),
        "the empty state misses the 10-coin toll"
    );
    // A short stack is still missing: the relaxed route crosses but
    // the strict gate needs the full count.
    let poor = WorldState {
        inv: HashMap::from([(995, 5)]),
        ..WorldState::default()
    };
    assert_eq!(
        find_missing_item_reqs(&wc, &g, from, to, FindOptions::default(), &poor),
        Some(vec![MissingReq::Carry { id: 995, count: 10 }])
    );
    // A state-proven edge needs no fetch.
    let rich = WorldState {
        inv: HashMap::from([(995, 10)]),
        ..WorldState::default()
    };
    assert_eq!(
        find_missing_item_reqs(&wc, &g, from, to, FindOptions::default(), &rich),
        Some(vec![]),
        "a state-proven edge needs no fetch"
    );
}

/// `worn_req` is any-of: while any listed id is worn nothing is
/// missing (the edge already passes); with none worn the diagnosis
/// is one [`MissingReq::WearAny`] carrying the whole alternative
/// list, so the session can fetch whichever one the player can get.
#[test]
fn find_missing_item_reqs_treats_worn_req_as_any_of() {
    let wc = walled_5x5();
    let mut g = toll_graph();
    g.edges[0].item_req = vec![];
    g.edges[0].worn_req = vec![1277, 1321]; // bronze sword, bronze scimitar
    let from = tile(0, 0, 0);
    let to = tile(4, 4, 0);
    // One listed blade worn: the worn gate passes, nothing to fetch.
    let wearing = WorldState {
        worn: HashSet::from([1321]),
        ..WorldState::default()
    };
    assert_eq!(
        find_missing_item_reqs(&wc, &g, from, to, FindOptions::default(), &wearing),
        Some(vec![]),
        "any-of means a worn alternative leaves nothing missing"
    );
    // None worn: one WearAny listing both alternatives.
    assert_eq!(
        find_missing_item_reqs(
            &wc,
            &g,
            from,
            to,
            FindOptions::default(),
            &WorldState::empty()
        ),
        Some(vec![MissingReq::WearAny {
            ids: vec![1277, 1321],
        }]),
        "no worn alternative: the session may fetch either blade"
    );
}

/// A route blocked by a skill gate is not a banking problem: the
/// relaxed search still fails, so the diagnosis is `None` and no
/// session can help.
#[test]
fn find_missing_item_reqs_is_none_when_a_non_item_gate_blocks() {
    let wc = walled_5x5();
    let mut g = toll_graph();
    g.edges[0].skill_req = vec![(6, 25)]; // Magic 25
    let from = tile(0, 0, 0);
    let to = tile(4, 4, 0);
    assert_eq!(
        find_missing_item_reqs(
            &wc,
            &g,
            from,
            to,
            FindOptions::default(),
            &WorldState::empty(),
        ),
        None,
        "a Magic 25 gate is not an item/worn gap: no session"
    );
}

// --- EssenceSession (Task 3): the mine exit returns only to the entry wizard ---

/// A 64×64 level-0 bake at (2880, 4800) — the whole Rune Essence mine
/// mapsquare (m45_75): the pad (2912,4833), the four exit portal
/// placements (2885,4850), (2889,4813), (2932,4854), (2933,4815), and
/// the walkable mine floor. Everything outside the bake is
/// unwalkable, so without the session return edge the mine is a
/// sealed dead end.
fn mine_bake() -> WorldCollision {
    bake_at(2880, 4800, 64, 64, &[])
}

#[test]
fn find_from_the_mine_requires_a_session_to_return() {
    let wc = mine_bake();
    let g = TransportGraph::default();
    let pad = tile(2912, 4833, 0);
    let aubury = tile(3253, 3401, 0); // ^essence_mine_to_aubury
                                      // No session: the mine is sealed — the pack carries no return
                                      // edges, so `find` (and `find_with` with no latch) is NoPath.
    assert!(matches!(
        find(&wc, &g, pad, aubury),
        Err(RouteError::NoPath)
    ));
    assert!(matches!(
        find_with(
            &wc,
            &g,
            pad,
            aubury,
            FindOptions::default(),
            &WorldState::empty(),
        ),
        Err(RouteError::NoPath)
    ));
    // With the session the exit portal returns to the entry wizard's
    // overworld anchor.
    let session = crate::essence::essence_session_for_wizard(553).unwrap();
    let r = find_with(
        &wc,
        &g,
        pad,
        aubury,
        FindOptions {
            essence: Some(session),
            ..FindOptions::default()
        },
        &WorldState::empty(),
    )
    .unwrap();
    assert_eq!(r.dest, aubury);
    let edge = r
        .legs
        .iter()
        .find_map(|l| match l {
            Leg::Transport { edge } => Some(edge),
            _ => None,
        })
        .expect("the route's only transport leg is the return hop");
    assert_eq!(edge.kind, TransportKind::EssenceExit);
    assert_eq!(
        edge.to, aubury,
        "the return lands on the entry wizard's anchor"
    );
    assert_eq!(edge.loc_id, crate::essence::ESSENCE_MINE_PORTAL_LOC_ID);
}

#[test]
fn the_session_return_reaches_only_the_entry_wizards_tile() {
    let wc = mine_bake();
    let g = TransportGraph::default();
    let pad = tile(2912, 4833, 0);
    let sedridor = tile(3106, 9572, 0); // ^essence_mine_to_sedridor
                                        // The exit returns only to Aubury; from his Varrock anchor the
                                        // fixture world reaches nothing else, so Sedridor's cellar anchor
                                        // is NoPath.
    let aubury = crate::essence::essence_session_for_wizard(553).unwrap();
    assert!(matches!(
        find_with(
            &wc,
            &g,
            pad,
            sedridor,
            FindOptions {
                essence: Some(aubury),
                ..FindOptions::default()
            },
            &WorldState::empty(),
        ),
        Err(RouteError::NoPath)
    ));
    // A session for the other wizard returns to the other anchor.
    let sed_session = crate::essence::essence_session_for_wizard(300).unwrap();
    let r = find_with(
        &wc,
        &g,
        pad,
        sedridor,
        FindOptions {
            essence: Some(sed_session),
            ..FindOptions::default()
        },
        &WorldState::empty(),
    )
    .unwrap();
    assert_eq!(r.dest, sedridor);
}

#[test]
fn walk_only_route_ticks_are_the_walk_tick_cost() {
    // Walking is no longer free: a walk-only route's `ticks` is the
    // walk cost (0.5 per tile at the run rate), not 0.
    let wc = bake(5, 5, &[]);
    let g = TransportGraph::default();
    let r = find(&wc, &g, tile(0, 0, 0), tile(4, 4, 0)).unwrap();
    assert_eq!(r.ticks, 2.0);
    assert!(r.legs.iter().all(|l| matches!(l, Leg::Walk { .. })));
}

#[test]
fn find_cost_model_sets_the_walk_rate_per_search() {
    // The run-vs-walk rate is a per-search input: the same 4-tile walk
    // costs 2 ticks at the running pace (0.5/tile) and 4 at the walking
    // pace (1/tile).
    let wc = bake(5, 5, &[]);
    let g = TransportGraph::default();
    let run = find_with_model(&wc, &g, tile(0, 0, 0), tile(4, 4, 0), CostModel::running()).unwrap();
    assert_eq!(run.ticks, 2.0);
    let walk = CostModel {
        run_per_step: PER_STEP_WALK,
        walk_per_step: PER_STEP_WALK,
    };
    let r = find_with_model(&wc, &g, tile(0, 0, 0), tile(4, 4, 0), walk).unwrap();
    assert_eq!(r.ticks, 4.0);
}

// --- allow_teleports: the any-tile teleport layer ---

/// A WorldState that proves the Varrock spell (Magic 25 + fire/air/law
/// runes) and a charged glory, so the teleport-layer tests can route.
fn spell_state() -> WorldState {
    WorldState {
        stats: HashMap::from([(6, 25)]),
        inv: HashMap::from([(554, 1), (556, 3), (563, 1), (1712, 1)]),
        ..WorldState::default()
    }
}

#[test]
fn find_never_uses_a_spell_teleport_but_find_allow_teleports_does() {
    // The wall splits the 5×5 bake; only the any-tile spell teleport can
    // cross it, and only when allow_teleports is on (and the state
    // proves the cast).
    let wc = walled_5x5();
    let dest = tile(4, 4, 0);
    let g = teleport(
        dest,
        3,                                  // OP_BASE 1 + the cast p_delay(2)
        vec![(6, 25)],                      // Magic level 25 (Varrock)
        vec![(554, 1), (556, 3), (563, 1)], // fire + air + law runes
    );
    assert!(matches!(
        find(&wc, &g, tile(0, 0, 0), dest),
        Err(RouteError::NoPath)
    ));
    // The empty state cannot prove the cast: the edge stays refused.
    assert!(
        matches!(
            find_allow_teleports(&wc, &g, tile(0, 0, 0), dest, &WorldState::empty()),
            Err(RouteError::NoPath)
        ),
        "allow_teleports still gates the spell on the WorldState"
    );
    let r = find_allow_teleports(&wc, &g, tile(0, 0, 0), dest, &spell_state()).unwrap();
    assert_eq!(r.dest, dest);
    // Teleported from the origin — no walk, just the cast.
    assert_eq!(r.ticks, 3.0);
    let leg = r
        .legs
        .iter()
        .find(|l| matches!(l, Leg::Transport { .. }))
        .expect("a teleport leg");
    let Leg::Transport { edge } = leg else {
        unreachable!()
    };
    assert_eq!(edge.kind, TransportKind::Teleport);
    assert_eq!(edge.skill_req, vec![(6, 25)]);
    assert_eq!(edge.item_req, vec![(554, 1), (556, 3), (563, 1)]);
    assert_eq!(edge.to, dest);
    assert_eq!(edge.ticks, 3);
}

#[test]
fn quest_stage_gated_teleport_routes_only_on_proven_evidence() {
    let wc = walled_5x5();
    let dest = tile(4, 4, 0);
    let gate = gate_window("tbwt", "tbwt_main", Some(3), Some(3));
    let mut g = teleport(dest, 3, vec![], vec![]);
    g.quest_family = Some(gate_family(1));
    g.teleports[0].quest_gates =
        Some(QuestGates::new(gate_family(1), [gate]).expect("valid stage gate"));

    assert_eq!(
        find_allow_teleports(&wc, &g, tile(0, 0, 0), dest, &WorldState::empty()),
        Err(RouteError::NoPath),
        "missing quest evidence refuses the gated teleport"
    );
    let disproven = evidenced(tbwt_evidence(gate_range(Some(4), Some(6))));
    assert_eq!(
        find_allow_teleports(&wc, &g, tile(0, 0, 0), dest, &disproven),
        Err(RouteError::NoPath),
        "evidence outside the stage window refuses the gated teleport"
    );

    let proven = evidenced(tbwt_evidence(gate_range(Some(3), Some(3))));
    let route = find_allow_teleports(&wc, &g, tile(0, 0, 0), dest, &proven)
        .expect("proven quest evidence opens the gated teleport");
    assert!(route
        .legs
        .iter()
        .any(|leg| matches!(leg, Leg::Transport { edge } if edge.quest_gates.is_some())));
}

#[test]
fn find_never_uses_a_jewellery_teleport_by_default() {
    let wc = walled_5x5();
    let dest = tile(4, 4, 0);
    let g = teleport(dest, 2, vec![], vec![(1712, 1)]); // charged glory
    assert!(matches!(
        find(&wc, &g, tile(0, 0, 0), dest),
        Err(RouteError::NoPath)
    ));
    // The charged item is on the player: the rub routes.
    let r = find_allow_teleports(&wc, &g, tile(0, 0, 0), dest, &spell_state()).unwrap();
    assert_eq!(r.ticks, 2.0); // OP_BASE 1 + the rub p_delay(1)
    let Leg::Transport { edge } = r
        .legs
        .iter()
        .find(|l| matches!(l, Leg::Transport { .. }))
        .unwrap()
    else {
        unreachable!()
    };
    assert_eq!(edge.item_req, vec![(1712, 1)]); // the charged item
    assert!(edge.skill_req.is_empty());
}

#[test]
fn find_allow_teleports_is_usable_from_any_tile() {
    let wc = walled_5x5();
    let dest = tile(4, 4, 0);
    let g = teleport(dest, 2, vec![], vec![(1712, 1)]);
    for origin in [tile(0, 0, 0), tile(1, 3, 0)] {
        let r = find_allow_teleports(&wc, &g, origin, dest, &spell_state()).unwrap();
        assert_eq!(r.dest, dest);
        assert_eq!(r.ticks, 2.0, "teleport from {origin:?} costs the rub only");
        assert!(r.legs.iter().any(|l| matches!(l, Leg::Transport { .. })));
    }
}

#[test]
fn find_allow_teleports_still_prefers_walking_when_cheaper() {
    // Total-tick cost still governs: on an open 5×5 the 4 run steps
    // (2.0) beat the 3-tick teleport, so the route walks.
    let wc = bake(5, 5, &[]);
    let dest = tile(4, 4, 0);
    let g = teleport(dest, 3, vec![(6, 25)], vec![]);
    let r = find_allow_teleports(&wc, &g, tile(0, 0, 0), dest, &spell_state()).unwrap();
    assert_eq!(r.ticks, 2.0);
    assert!(r.legs.iter().all(|l| matches!(l, Leg::Walk { .. })));
}

// --- wilderness opt-in (FindOptions.allow_wilderness) ---

/// A `width × height` all-open level-0 bake at `origin` (flags all 0,
/// walkable derived).
fn open_world(origin: WorldTile, width: usize, height: usize) -> WorldCollision {
    let flags = vec![0u32; width * height];
    let (walk, blocked) = crate::collision::pack_walk(&flags);
    WorldCollision {
        origin,
        width,
        height,
        walk,
        blocked,
        flags: None,
    }
}

#[test]
fn find_does_not_enter_wilderness_without_the_flag() {
    let wc = open_world(
        WorldTile {
            x: 3099,
            z: 3518,
            level: 0,
        },
        5,
        12,
    );
    let g = TransportGraph {
        wilderness: surface_wildy_rules(),
        ..Default::default()
    };
    let from = WorldTile {
        x: 3100,
        z: 3519,
        level: 0,
    }; // z 3519 < 3520
    let to = WorldTile {
        x: 3100,
        z: 3525,
        level: 0,
    }; // in zone
    assert!(matches!(find(&wc, &g, from, to), Err(RouteError::NoPath)));
    let ok = find_with(
        &wc,
        &g,
        from,
        to,
        FindOptions {
            allow_teleports: false,
            allow_wilderness: true,
            allow_bank_fetch: false,
            ..FindOptions::default()
        },
        &WorldState::empty(),
    );
    assert!(ok.is_ok());
}

#[test]
fn already_in_wilderness_can_walk_out_without_the_flag() {
    let wc = open_world(
        WorldTile {
            x: 3099,
            z: 3518,
            level: 0,
        },
        5,
        12,
    );
    let g = TransportGraph {
        wilderness: surface_wildy_rules(),
        ..Default::default()
    };
    let from = WorldTile {
        x: 3100,
        z: 3525,
        level: 0,
    };
    let to = WorldTile {
        x: 3100,
        z: 3519,
        level: 0,
    };
    assert!(find(&wc, &g, from, to).is_ok());
}

#[test]
fn find_allow_teleports_still_refuses_a_wilderness_landing() {
    // A walled 5×12 bake at (3099,3518): only the any-tile teleport
    // crosses the wall, but its landing (3102,3525) is inside the
    // zone. Default find refuses (wall), allow_teleports alone still
    // refuses (the wildy landing), and both flags together route.
    let mut flags = vec![0u32; 5 * 12];
    for z in 0..12 {
        flags[z * 5 + 1] |= CollisionFlag::W_E as u32;
        flags[z * 5 + 2] |= CollisionFlag::W_W as u32;
    }
    let (walk, blocked) = crate::collision::pack_walk(&flags);
    let wc = WorldCollision {
        origin: WorldTile {
            x: 3099,
            z: 3518,
            level: 0,
        },
        width: 5,
        height: 12,
        walk,
        blocked,
        flags: None,
    };
    let dest = tile(3102, 3525, 0);
    let mut g = teleport(dest, 3, vec![(6, 25)], vec![(554, 1), (556, 3), (563, 1)]);
    g.wilderness = surface_wildy_rules();
    let from = tile(3100, 3519, 0);
    assert!(matches!(find(&wc, &g, from, dest), Err(RouteError::NoPath)));
    // The state proves the cast (Magic 25 + runes), so only the wildy
    // landing refuses it.
    assert!(
        matches!(
            find_allow_teleports(&wc, &g, from, dest, &spell_state()),
            Err(RouteError::NoPath)
        ),
        "a teleport landing inside the wilderness must stay refused"
    );
    let ok = find_with(
        &wc,
        &g,
        from,
        dest,
        FindOptions {
            allow_teleports: true,
            allow_wilderness: true,
            allow_bank_fetch: false,
            ..FindOptions::default()
        },
        &spell_state(),
    );
    assert!(ok.is_ok());
}

// --- AvoidRect / find_with_avoid (inspect slice 1) ---

fn avoid_box(min_x: i32, max_x: i32, min_z: i32, max_z: i32) -> AvoidRect {
    AvoidRect {
        min_x,
        max_x,
        min_z,
        max_z,
        level: None,
    }
}

#[test]
fn avoid_rect_contains_uses_inclusive_bounds_and_optional_level() {
    let all_levels = avoid_box(1, 3, 4, 6);
    assert!(all_levels.contains(tile(1, 4, 0)));
    assert!(all_levels.contains(tile(3, 6, 2)));
    assert!(!all_levels.contains(tile(0, 4, 0)));
    assert!(!all_levels.contains(tile(1, 7, 0)));
    let lvl1 = AvoidRect {
        min_x: 0,
        max_x: 9,
        min_z: 0,
        max_z: 9,
        level: Some(1),
    };
    assert!(!lvl1.contains(tile(5, 5, 0)));
    assert!(lvl1.contains(tile(5, 5, 1)));
}

#[test]
fn find_with_avoid_walk_rules_outside_in_start_inside_and_destination() {
    let wc = bake(5, 5, &[]);
    let g = TransportGraph::default();
    let patch = [avoid_box(1, 3, 1, 3)];
    let opts = FindOptions::default();
    let empty = WorldState::empty();
    // Outside cannot step into the patch; the open grid still routes around it.
    let r = find_with_avoid(&wc, &g, tile(0, 0, 0), tile(4, 4, 0), opts, &empty, &patch).unwrap();
    let stepped: Vec<WorldTile> = r
        .legs
        .iter()
        .flat_map(|l| match l {
            Leg::Walk { tiles } => tiles.clone(),
            Leg::Transport { .. } => vec![],
        })
        .collect();
    assert!(
        !stepped
            .windows(2)
            .any(|w| !patch[0].contains(w[0]) && patch[0].contains(w[1])),
        "must not enter the avoid patch from outside"
    );
    // Destination inside + start outside cannot enter.
    assert!(matches!(
        find_with_avoid(&wc, &g, tile(0, 0, 0), tile(2, 2, 0), opts, &empty, &patch,),
        Err(RouteError::NoPath)
    ));
    // Start already inside the union may leave: two overlapping rects, escape semantics.
    let union = [avoid_box(1, 2, 1, 2), avoid_box(2, 3, 1, 2)];
    let out = find_with_avoid(&wc, &g, tile(2, 1, 0), tile(4, 4, 0), opts, &empty, &union).unwrap();
    assert_eq!(out.dest, tile(4, 4, 0));
    // Destination inside while start is inside is allowed.
    assert!(find_with_avoid(&wc, &g, tile(2, 2, 0), tile(1, 1, 0), opts, &empty, &patch,).is_ok());
    // Level-scoped avoid applies only on the matching plane.
    let level0_block = AvoidRect {
        min_x: 2,
        max_x: 2,
        min_z: 2,
        max_z: 2,
        level: Some(0),
    };
    assert!(matches!(
        find_with_avoid(
            &wc,
            &g,
            tile(0, 0, 0),
            tile(2, 2, 0),
            opts,
            &empty,
            &[level0_block],
        ),
        Err(RouteError::NoPath)
    ));
    assert!(
        find_with_avoid(
            &wc,
            &g,
            tile(0, 0, 1),
            tile(2, 2, 1),
            opts,
            &empty,
            &[level0_block],
        )
        .is_ok(),
        "level-0 avoid must not block the same x/z on another plane"
    );
}

#[test]
fn find_with_avoid_transport_and_teleport_landings_follow_outside_in_rule() {
    let wc_wall = walled_5x5();
    let door_landing_inside = door(tile(1, 2, 0), tile(2, 2, 0), 2);
    let landing_only = [avoid_box(2, 2, 2, 2)];
    let opts = FindOptions::default();
    let empty = WorldState::empty();
    assert!(
        find_with(
            &wc_wall,
            &door_landing_inside,
            tile(0, 0, 0),
            tile(4, 0, 0),
            opts,
            &empty
        )
        .is_ok(),
        "sanity: without avoid the door route exists"
    );
    // Sealed wall: the only crossing lands on the avoided tile.
    let door_blocked = find_with_avoid(
        &wc_wall,
        &door_landing_inside,
        tile(0, 0, 0),
        tile(4, 0, 0),
        opts,
        &empty,
        &landing_only,
    );
    assert!(
        matches!(door_blocked, Err(RouteError::NoPath)),
        "door landing inside avoid is refused from outside, got {door_blocked:?}"
    );
    // Takeoff `at` may sit inside avoid when the landing is outside.
    let wc_door = blocked_door_fixture();
    let g_at_inside = door(tile(2, 0, 0), tile(2, 2, 0), 2);
    let at_only = [avoid_box(2, 0, 2, 0)];
    let via_door = find_with_avoid(
        &wc_door,
        &g_at_inside,
        tile(1, 0, 0),
        tile(4, 2, 0),
        opts,
        &empty,
        &at_only,
    )
    .unwrap();
    assert!(
        via_door
            .legs
            .iter()
            .any(|l| matches!(l, Leg::Transport { .. })),
        "approach from outside may still use the door when the landing is outside avoid"
    );
    // Any-tile teleport landing uses the same rule (wall leaves teleport as the only hop).
    let wc_tp = walled_5x5();
    let dest = tile(4, 4, 0);
    let g_tp = teleport(dest, 2, vec![], vec![(1712, 1)]);
    let tp_patch = [avoid_box(4, 4, 4, 4)];
    assert!(
        matches!(
            find_with_avoid(
                &wc_tp,
                &g_tp,
                tile(0, 0, 0),
                dest,
                FindOptions {
                    allow_teleports: true,
                    ..FindOptions::default()
                },
                &spell_state(),
                &tp_patch,
            ),
            Err(RouteError::NoPath)
        ),
        "teleport landing on an avoided tile is refused from outside"
    );
    // Nonempty avoid that misses the landing still permits the teleport hop (not a walk-around).
    let tp_allowed = find_with_avoid(
        &wc_tp,
        &g_tp,
        tile(0, 0, 0),
        dest,
        FindOptions {
            allow_teleports: true,
            ..FindOptions::default()
        },
        &spell_state(),
        &[avoid_box(1, 1, 1, 1)],
    )
    .unwrap();
    assert_eq!(tp_allowed.dest, dest);
    let tp_leg = tp_allowed
        .legs
        .iter()
        .find_map(|l| match l {
            Leg::Transport { edge } => Some(edge),
            _ => None,
        })
        .expect("walled bake requires the teleport leg, not a walk detour");
    assert_eq!(tp_leg.kind, TransportKind::Teleport);
    assert_eq!(tp_leg.to, dest);
    // Essence return landing is gated the same way.
    let wc_mine = mine_bake();
    let session = crate::essence::essence_session_for_wizard(553).unwrap();
    let aubury = session.return_tile;
    let mine_patch = [avoid_box(aubury.x, aubury.x, aubury.z, aubury.z)];
    assert!(matches!(
        find_with_avoid(
            &wc_mine,
            &TransportGraph::default(),
            tile(2912, 4833, 0),
            aubury,
            FindOptions {
                essence: Some(session),
                ..FindOptions::default()
            },
            &empty,
            &mine_patch,
        ),
        Err(RouteError::NoPath)
    ));
    // Avoid the mine pad only; the return landing stays clear.
    let pad = tile(2912, 4833, 0);
    let essence_allowed = find_with_avoid(
        &wc_mine,
        &TransportGraph::default(),
        pad,
        aubury,
        FindOptions {
            essence: Some(session),
            ..FindOptions::default()
        },
        &empty,
        &[avoid_box(pad.x, pad.x, pad.z, pad.z)],
    )
    .unwrap();
    assert_eq!(essence_allowed.dest, aubury);
    let return_leg = essence_allowed
        .legs
        .iter()
        .find_map(|l| match l {
            Leg::Transport { edge } => Some(edge),
            _ => None,
        })
        .expect("mine exit must use the essence return hop");
    assert_eq!(return_leg.kind, TransportKind::EssenceExit);
    assert_eq!(return_leg.to, aubury);
}

#[test]
fn find_with_avoid_empty_matches_find_with_and_does_not_bypass_gates() {
    let wc = bake(5, 5, &[]);
    let g = TransportGraph::default();
    let from = tile(0, 0, 0);
    let to = tile(4, 4, 0);
    let opts = FindOptions::default();
    let empty = WorldState::empty();
    let plain = find_with(&wc, &g, from, to, opts, &empty).unwrap();
    let no_avoid = find_with_avoid(&wc, &g, from, to, opts, &empty, &[]).unwrap();
    assert_eq!(plain.ticks, no_avoid.ticks);
    assert_eq!(plain.legs.len(), no_avoid.legs.len());
    // Subsequent plain find is unchanged after an avoid search.
    assert_eq!(
        find_with(&wc, &g, from, to, opts, &empty).unwrap().ticks,
        plain.ticks
    );
    // Wilderness / teleport gates stay fail-closed with avoid present.
    let wc_w = open_world(
        WorldTile {
            x: 3099,
            z: 3518,
            level: 0,
        },
        5,
        12,
    );
    let wild_to = tile(3100, 3525, 0);
    let g_w = TransportGraph {
        wilderness: surface_wildy_rules(),
        ..Default::default()
    };
    assert!(matches!(
        find_with_avoid(
            &wc_w,
            &g_w,
            tile(3100, 3519, 0),
            wild_to,
            opts,
            &empty,
            &[avoid_box(0, 9999, 0, 9999)],
        ),
        Err(RouteError::NoPath)
    ));
    let wc_wall = walled_5x5();
    let dest = tile(4, 4, 0);
    let g_tp = teleport(dest, 2, vec![], vec![(1712, 1)]);
    assert!(matches!(
        find_with_avoid(
            &wc_wall,
            &g_tp,
            tile(0, 0, 0),
            dest,
            FindOptions {
                allow_teleports: true,
                ..FindOptions::default()
            },
            &WorldState::empty(),
            &[],
        ),
        Err(RouteError::NoPath)
    ));
}

#[test]
fn find_missing_item_reqs_with_avoid_and_budget_errors_stay_distinct() {
    let wc = walled_5x5();
    let g = toll_graph();
    let from = tile(0, 0, 0);
    let to = tile(4, 4, 0);
    // Block only the door landing tile, not the whole east side destination.
    let landing_only = [avoid_box(2, 2, 2, 2)];
    assert_eq!(
        find_missing_item_reqs_with_avoid(
            &wc,
            &g,
            from,
            to,
            FindOptions::default(),
            &WorldState::empty(),
            &landing_only,
        ),
        None,
        "avoid blocking the only crossing is not an item gap"
    );
    assert_eq!(
        find_missing_item_reqs_with_avoid(
            &wc,
            &g,
            from,
            to,
            FindOptions::default(),
            &WorldState::empty(),
            &[],
        ),
        Some(vec![MissingReq::Carry { id: 995, count: 10 }])
    );
    let open = bake(10, 10, &[]);
    assert!(matches!(
        find_bounded(
            &open,
            &TransportGraph::default(),
            tile(0, 0, 0),
            tile(9, 9, 0),
            CostModel::running(),
            8,
        ),
        Err(RouteError::BudgetExhausted)
    ));
    assert!(
        matches!(
            find_with_avoid(
                &open,
                &TransportGraph::default(),
                tile(0, 0, 0),
                tile(5, 5, 0),
                FindOptions::default(),
                &WorldState::empty(),
                &[avoid_box(5, 5, 5, 5)],
            ),
            Err(RouteError::NoPath)
        ),
        "dest inside avoid from outside is NoPath, not budget exhaustion"
    );
}

#[test]
fn lumbridge_cow_pen_to_varrock_uses_the_south_gate() {
    // (3253,3282) is inside the cow pen. The south gate (loc 1551/1553
    // at 3253,3266/3267) is adjacent from inside. The north-west road
    // gate at (3241,3301) is three tiles through the north fence —
    // INTERACT_RADIUS 3 lets find "use" it from inside and the walker
    // then aims at the fence. GitHub has no pack — skip, do not panic.
    let Some(world) = crate::world::NavWorld::load_default_pack_or_skip() else {
        return;
    };
    let from = WorldTile {
        x: 3253,
        z: 3282,
        level: 0,
    };
    let to = WorldTile {
        x: 3213,
        z: 3424,
        level: 0,
    };
    let route = find(&world.collision, &world.graph, from, to)
        .unwrap_or_else(|e| panic!("cow pen -> Varrock must route: {e:?}"));
    let first_door = route.legs.iter().find_map(|l| match l {
        Leg::Transport { edge } if edge.kind == TransportKind::Door => Some(edge),
        _ => None,
    });
    let door = first_door.expect("must exit the pen through a door");
    assert!(
            (door.at.x == 3253 && (door.at.z == 3266 || door.at.z == 3267))
                || (door.at.x == 3253 && (door.to.z == 3266 || door.to.z == 3267)),
            "first door must be the south cow-pen gate (3253,3266/3267), got at=({}, {}) to=({}, {}) loc={}",
            door.at.x,
            door.at.z,
            door.to.x,
            door.to.z,
            door.loc_id
        );
    assert_ne!(
        (door.at.x, door.at.z),
        (3241, 3301),
        "must not clip through the north fence to the road gate"
    );
}

#[test]
fn packed_edgeville_bank_return_to_eggs_uses_the_surface_trapdoor() {
    // Live a5z328_0: WalkNear from 3094,3489 to 3120,9952 radius 3 after
    // the Edgeville bank cycle produced no nav-follow. Stock maps place
    // trapdoor 1568 at 3097,3468; derive_transports now emits that hop.
    // GitHub has no pack — skip, do not panic. A pre-rebake pack still
    // lacks the edge, so the test inserts the derived hop.
    let Some(world) = crate::world::NavWorld::load_default_pack_or_skip() else {
        return;
    };
    let from = WorldTile {
        x: 3094,
        z: 3489,
        level: 0,
    };
    let eggs = WorldTile {
        x: 3120,
        z: 9952,
        level: 0,
    };
    let trapdoor_at = WorldTile {
        x: 3097,
        z: 3468,
        level: 0,
    };
    let opts = FindOptions {
        allow_teleports: false,
        allow_wilderness: true,
        allow_bank_fetch: true,
        zones: crate::zones::ZoneExempt::all(),
        ..FindOptions::default()
    };
    let route = find_with(
        &world.collision,
        &world.graph,
        from,
        eggs,
        opts,
        &WorldState::empty().with_map_members(true),
    )
    .unwrap_or_else(|e| panic!("Edgeville bank -> red spider eggs must route: {e:?}"));
    let used_trap = route.legs.iter().any(|leg| match leg {
        Leg::Transport { edge } => {
            (edge.loc_id == 1568 || edge.loc_id == 1570)
                && edge.at.x == trapdoor_at.x
                && edge.at.z == trapdoor_at.z
        }
        _ => false,
    });
    assert!(
        used_trap,
        "return must use the surface trapdoor, legs={:?}",
        route
            .legs
            .iter()
            .filter_map(|leg| match leg {
                Leg::Transport { edge } => Some((edge.kind, edge.loc_id, edge.at, edge.to)),
                _ => None,
            })
            .collect::<Vec<_>>()
    );
}

fn surface_wildy_rules() -> WildernessRules {
    WildernessRules {
        zones: vec![WildernessZone {
            x1: 2944,
            z1: 3520,
            x2: 3391,
            z2: 6399,
            level1: 0,
            level2: 3,
            origin_z: 3520,
        }],
        divisor: 8,
        offset: 1,
    }
}

fn walled_wildy_row(z: i32) -> WorldCollision {
    let ox = 3100;
    bake_at(
        ox,
        z,
        5,
        1,
        &[
            (ox + 1, z, CollisionFlag::W_E as u32),
            (ox + 2, z, CollisionFlag::W_W as u32),
        ],
    )
}

fn capped_spell(to: WorldTile, cap: i32, rules: WildernessRules) -> TransportGraph {
    let mut graph = teleport(to, 3, vec![(6, 25)], vec![(554, 1), (556, 3), (563, 1)]);
    graph.teleports[0].wildy_cap = Some(cap);
    graph.wilderness = rules;
    graph
}

fn teleport_opts() -> FindOptions {
    FindOptions {
        allow_teleports: true,
        allow_wilderness: true,
        allow_bank_fetch: false,
        ..FindOptions::default()
    }
}

fn spell_only_state() -> WorldState {
    WorldState {
        stats: HashMap::from([(6, 99)]),
        inv: HashMap::from([(554, 20), (556, 20), (563, 20)]),
        ..WorldState::default()
    }
}

/// Level 20 is the last legal spell takeoff; 21 cannot teleport across the wall.
#[test]
fn spell_teleport_cap_is_exact_at_level_20() {
    let rules = surface_wildy_rules();
    assert_eq!(
        rules.level(tile(3100, 3679, 0)),
        20,
        "z 3679 is the last tile of level 20"
    );
    assert_eq!(
        rules.level(tile(3100, 3680, 0)),
        21,
        "z 3680 is the first tile of level 21"
    );
    let dest = tile(3104, 3679, 0);
    let graph = capped_spell(dest, 20, rules.clone());
    let wc20 = walled_wildy_row(3679);
    let r = find_with(
        &wc20,
        &graph,
        tile(3100, 3679, 0),
        dest,
        teleport_opts(),
        &spell_only_state(),
    )
    .expect("level 20 may still cast");
    assert!(
        r.legs.iter().any(|l| matches!(l, Leg::Transport { .. })),
        "level 20 uses the spell"
    );

    let dest21 = tile(3104, 3680, 0);
    let graph21 = capped_spell(dest21, 20, rules);
    let wc21 = walled_wildy_row(3680);
    assert!(
        matches!(
            find_with(
                &wc21,
                &graph21,
                tile(3100, 3680, 0),
                dest21,
                teleport_opts(),
                &spell_only_state(),
            ),
            Err(RouteError::NoPath)
        ),
        "level 21 cannot teleport across the wall"
    );
}

/// Glory's higher cap is exact at 30 vs 31.
#[test]
fn glory_teleport_cap_is_exact_at_level_30() {
    let rules = surface_wildy_rules();
    assert_eq!(rules.level(tile(3100, 3759, 0)), 30);
    assert_eq!(rules.level(tile(3100, 3760, 0)), 31);
    let dest = tile(3104, 3759, 0);
    let mut graph = teleport(dest, 2, vec![], vec![(1712, 1)]);
    graph.teleports[0].wildy_cap = Some(30);
    graph.wilderness = rules.clone();
    let glory = WorldState {
        inv: HashMap::from([(1712, 1)]),
        ..WorldState::default()
    };
    let r = find_with(
        &walled_wildy_row(3759),
        &graph,
        tile(3100, 3759, 0),
        dest,
        teleport_opts(),
        &glory,
    )
    .expect("level 30 may still rub glory");
    assert!(r.legs.iter().any(|l| matches!(l, Leg::Transport { .. })));

    let dest31 = tile(3104, 3760, 0);
    graph.teleports[0].to = dest31;
    assert!(
        matches!(
            find_with(
                &walled_wildy_row(3760),
                &graph,
                tile(3100, 3760, 0),
                dest31,
                teleport_opts(),
                &glory,
            ),
            Err(RouteError::NoPath)
        ),
        "level 31 cannot glory across the wall"
    );
}

fn install_zones(collision: &WorldCollision, graph: &mut TransportGraph, mut zones: Vec<Zone>) {
    // Rectangular corridor fixtures use canonical NPC reach squares with
    // carved exterior cells; production 289 zones still ship no carves.
    let mut carves = Vec::new();
    for (index, zone) in zones.iter_mut().enumerate() {
        let desired = *zone;
        let x = desired.min_x + (desired.max_x - desired.min_x + 1) / 2;
        let z = desired.min_z + (desired.max_z - desired.min_z + 1) / 2;
        let radius = (x - desired.min_x).max(z - desired.min_z);
        *zone = Zone::npc(
            tile(x, z, i32::from(desired.level)),
            u8::try_from(radius).unwrap(),
            desired.class,
            desired.cap,
            desired.kind,
        );
        for (min_x, max_x, min_z, max_z) in [
            (zone.min_x, desired.min_x - 1, zone.min_z, zone.max_z),
            (desired.max_x + 1, zone.max_x, zone.min_z, zone.max_z),
            (desired.min_x, desired.max_x, zone.min_z, desired.min_z - 1),
            (desired.min_x, desired.max_x, desired.max_z + 1, zone.max_z),
        ] {
            if min_x <= max_x && min_z <= max_z {
                carves.push((
                    u16::try_from(index).unwrap(),
                    crate::router::AvoidRect {
                        min_x,
                        max_x,
                        min_z,
                        max_z,
                        level: Some(i32::from(zone.level)),
                    },
                ));
            }
        }
    }
    graph.zones = Some(
        ZoneTable::from_parts(
            zones,
            vec![ZoneKind::new(
                "fixture",
                "Fixture hunter",
                1,
                6,
                true,
                false,
            )],
            vec![],
            carves,
            vec![],
            collision.origin,
            collision.width as u32,
            collision.height as u32,
            &graph.wilderness,
        )
        .unwrap(),
    );
}

fn rect_zone(min_x: i32, max_x: i32, min_z: i32, max_z: i32) -> Zone {
    let mut zone = Zone::npc(tile(min_x, min_z, 0), 0, ZoneClass::Always, u16::MAX, 0);
    zone.min_x = min_x;
    zone.max_x = max_x;
    zone.min_z = min_z;
    zone.max_z = max_z;
    zone
}

fn route_walk_tiles(route: &crate::router::Route) -> impl Iterator<Item = WorldTile> + '_ {
    route
        .legs
        .iter()
        .flat_map(|leg| match leg {
            Leg::Walk { tiles } => tiles.as_slice(),
            Leg::Transport { .. } => &[],
        })
        .copied()
}

#[test]
fn zoned_candidate_end_authorizes_only_its_own_completion_route() {
    // S=(0,0), G1=(16,0); the narrow corridor crosses Z. G2=(14,4)
    // is its only dead-end pocket. Walking costs 8 and 9 respectively.
    let mut blocked: Vec<_> = (0..5)
        .flat_map(|z| {
            (0..17).filter_map(move |x| {
                (z != 0 && x != 14).then_some((x, z, CollisionFlag::WALK_SCENERY as u32))
            })
        })
        .collect();
    blocked.push((14, 1, (CollisionFlag::W_W | CollisionFlag::W_E) as u32));
    let collision = bake(17, 5, &blocked);
    let from = tile(0, 0, 0);
    let past = tile(16, 0, 0);
    let pocket = tile(14, 4, 0);
    let goals = [past, pocket];
    let mut graph = door(from, past, 30);
    install_zones(&collision, &mut graph, vec![rect_zone(2, 14, 0, 4)]);
    let state = WorldState::empty();
    let first = find_first_with(
        &collision,
        &graph,
        from,
        &goals,
        FindOptions::default(),
        &state,
    );
    assert_eq!(first.completion_partitions(), 1);
    let first = first.into_route().unwrap();
    assert_eq!((first.dest, first.ticks), (pocket, 9.0));
    let many = find_many_with(
        &collision,
        &graph,
        from,
        &goals,
        FindOptions::default(),
        &state,
    );
    assert_eq!(many.completion_partitions(), 1);
    assert_eq!(many.route(0).unwrap().ticks, 30.0);
    assert_eq!(many.route(1).unwrap().ticks, 9.0);
    let bypass = find_many_with(
        &collision,
        &graph,
        from,
        &goals,
        FindOptions {
            zones: ZoneExempt::all(),
            ..FindOptions::default()
        },
        &state,
    );
    assert_eq!(bypass.completion_partitions(), 0);
    assert_eq!(bypass.scratch_capacities().zone_mask_words, 0);
    assert_eq!(bypass.route(0).unwrap().ticks, 8.0);

    graph.at.clear();
    graph.edges.clear();
    assert!(matches!(
        find_with(
            &collision,
            &graph,
            from,
            past,
            FindOptions::default(),
            &state
        ),
        Err(RouteError::NoPath),
    ));
    assert_eq!(
        crate::router::find_blocking_zones(
            &collision,
            &graph,
            from,
            past,
            FindOptions::default(),
            &state,
            &[],
        ),
        Some(vec![ZoneKey::Zone(0)]),
    );
    assert_eq!(
        find_first_with(
            &collision,
            &graph,
            from,
            &goals,
            FindOptions::default(),
            &state
        )
        .into_route()
        .unwrap()
        .dest,
        pocket,
    );
}

#[test]
fn starting_zone_exemption_never_exempts_an_overlapping_zone() {
    let collision = bake(9, 3, &[]);
    let mut graph = TransportGraph::default();
    // Z2 is wholly inside Z1: testing only Z2\Z1 would test no cells.
    install_zones(
        &collision,
        &mut graph,
        vec![rect_zone(0, 4, 0, 1), rect_zone(3, 4, 0, 1)],
    );
    let route = find_with(
        &collision,
        &graph,
        tile(0, 0, 0),
        tile(8, 0, 0),
        FindOptions::default(),
        &WorldState::empty(),
    )
    .unwrap();
    let table = graph.zones.as_ref().unwrap();
    assert!(route_walk_tiles(&route).any(|t| table.at(t).any(|i| i == 0)));
    assert!(route_walk_tiles(&route).all(|t| table.at(t).all(|i| i != 1)));
}

#[test]
fn diagnosis_names_only_zones_active_under_the_original_predicate() {
    let collision = bake(9, 1, &[]);
    let mut graph = TransportGraph::default();
    install_zones(
        &collision,
        &mut graph,
        vec![
            Zone::npc(tile(2, 0, 0), 0, ZoneClass::LevelRule, 12, 0),
            Zone::npc(tile(4, 0, 0), 0, ZoneClass::Always, u16::MAX, 0),
        ],
    );
    let state = WorldState {
        combat_level: Some(126),
        ..WorldState::empty()
    };
    assert!(matches!(
        find_with(
            &collision,
            &graph,
            tile(0, 0, 0),
            tile(8, 0, 0),
            FindOptions::default(),
            &state
        ),
        Err(RouteError::NoPath),
    ));
    assert_eq!(
        crate::router::find_blocking_zones(
            &collision,
            &graph,
            tile(0, 0, 0),
            tile(8, 0, 0),
            FindOptions::default(),
            &state,
            &[],
        ),
        Some(vec![ZoneKey::Zone(1)]),
    );
}

#[test]
fn straddling_zone_endpoint_keys_on_geometry_not_goal_activation() {
    let collision = bake(9, 1, &[]);
    let mut graph = TransportGraph {
        wilderness: WildernessRules {
            zones: vec![WildernessZone {
                x1: 2,
                x2: 5,
                z1: 0,
                z2: 0,
                level1: 0,
                level2: 0,
                origin_z: 0,
            }],
            ..WildernessRules::default()
        },
        ..TransportGraph::default()
    };
    let state = WorldState {
        combat_level: Some(126),
        ..WorldState::empty()
    };
    let opts = FindOptions {
        allow_wilderness: true,
        ..FindOptions::default()
    };
    let goal = tile(7, 0, 0);
    for (min_x, partitions) in [(2, 1), (6, 0)] {
        let mut zone = rect_zone(min_x, 7, 0, 0);
        zone.class = ZoneClass::LevelRule;
        zone.cap = 12;
        install_zones(&collision, &mut graph, vec![zone]);
        let single = find_with(&collision, &graph, tile(0, 0, 0), goal, opts, &state).unwrap();
        assert_eq!((single.ticks, route_walk_tiles(&single).count()), (3.5, 8));
        let first = find_first_with(&collision, &graph, tile(0, 0, 0), &[goal], opts, &state);
        assert_eq!(first.completion_partitions(), partitions);
        assert_eq!(first.into_route().unwrap(), single);
        let goals = [goal];
        let many = find_many_with(&collision, &graph, tile(0, 0, 0), &goals, opts, &state);
        assert_eq!(many.completion_partitions(), partitions);
        assert_eq!(many.route(0).unwrap(), single);
    }
}

#[test]
fn transport_and_any_tile_teleport_landings_are_checked_independently() {
    let collision = bake(8, 1, &[(2, 0, CollisionFlag::WALK_SCENERY as u32)]);
    let from = tile(0, 0, 0);
    let landing = tile(3, 0, 0);
    let goal = tile(7, 0, 0);
    for teleport in [false, true] {
        let mut graph = door(from, landing, 2);
        if teleport {
            let mut edge = graph.edges.pop().unwrap();
            edge.kind = TransportKind::Teleport;
            graph.at.clear();
            graph.teleports.push(edge);
        }
        install_zones(&collision, &mut graph, vec![rect_zone(3, 3, 0, 0)]);
        let opts = FindOptions {
            allow_teleports: true,
            ..FindOptions::default()
        };
        let state = WorldState::empty();
        assert!(
            matches!(
                find_with(&collision, &graph, from, goal, opts, &state),
                Err(RouteError::NoPath),
            ),
            "landing must block, teleport={teleport}"
        );
        let route = find_with(
            &collision,
            &graph,
            from,
            goal,
            FindOptions {
                zones: ZoneExempt::named(&[ZoneKey::Zone(0)]).unwrap(),
                ..opts
            },
            &state,
        )
        .unwrap();
        assert!(route
            .legs
            .iter()
            .any(|leg| { matches!(leg, Leg::Transport { edge } if edge.to == landing) }));
    }
}

#[test]
fn essence_return_landing_is_checked_before_the_following_walk() {
    let session = crate::essence::essence_session_for_wizard(553).unwrap();
    let landing = session.return_tile;
    // One permanent wall separates the two regions, leaving the real
    // essence return as the sole route out of the mine.
    let wall: Vec<_> = (2880..3264)
        .map(|x| (x, 4799, CollisionFlag::WALK_SCENERY as u32))
        .collect();
    let collision = bake_at(2880, 3400, 384, 1464, &wall);
    let mut graph = TransportGraph::default();
    install_zones(
        &collision,
        &mut graph,
        vec![rect_zone(landing.x, landing.x, landing.z, landing.z)],
    );
    let from = tile(2912, 4833, 0);
    let goal = tile(landing.x + 1, landing.z, 0);
    let opts = FindOptions {
        essence: Some(session),
        ..FindOptions::default()
    };
    let state = WorldState::empty();
    assert!(matches!(
        find_with(&collision, &graph, from, goal, opts, &state),
        Err(RouteError::NoPath),
    ));
    let route = find_with(
        &collision,
        &graph,
        from,
        goal,
        FindOptions {
            zones: ZoneExempt::named(&[ZoneKey::Zone(0)]).unwrap(),
            ..opts
        },
        &state,
    )
    .unwrap();
    assert!(route
        .legs
        .iter()
        .any(|leg| { matches!(leg, Leg::Transport { edge } if edge.to == landing) }));
}

#[test]
fn combat_lifting_is_by_tile_and_never_bypasses_wilderness_permission() {
    let collision = bake(9, 1, &[]);
    let mut graph = TransportGraph {
        wilderness: WildernessRules {
            zones: vec![WildernessZone {
                x1: 2,
                x2: 5,
                z1: 0,
                z2: 0,
                level1: 0,
                level2: 0,
                origin_z: 0,
            }],
            ..WildernessRules::default()
        },
        ..TransportGraph::default()
    };
    let mut zone = rect_zone(2, 7, 0, 0);
    zone.class = ZoneClass::LevelRule;
    zone.cap = 12;
    install_zones(&collision, &mut graph, vec![zone]);
    let state = WorldState {
        combat_level: Some(126),
        ..WorldState::empty()
    };
    let allowed = FindOptions {
        allow_wilderness: true,
        ..FindOptions::default()
    };
    let filter = crate::zones::ZoneFilter::new(
        graph.zones.as_ref().unwrap(),
        state.combat_level,
        &[tile(0, 0, 0), tile(8, 0, 0)],
        &ZoneExempt::NONE,
    );
    assert!(filter.blocks(&graph.wilderness, tile(5, 0, 0)));
    assert!(!filter.blocks(&graph.wilderness, tile(6, 0, 0)));
    assert!(matches!(
        find_with(
            &collision,
            &graph,
            tile(0, 0, 0),
            tile(8, 0, 0),
            allowed,
            &state
        ),
        Err(RouteError::NoPath),
    ));
    assert_eq!(
        find_with(
            &collision,
            &graph,
            tile(6, 0, 0),
            tile(8, 0, 0),
            allowed,
            &state
        )
        .unwrap()
        .ticks,
        1.0,
    );
    let crossed = FindOptions {
        zones: ZoneExempt::all(),
        ..allowed
    };
    assert_eq!(
        find_with(
            &collision,
            &graph,
            tile(0, 0, 0),
            tile(8, 0, 0),
            crossed,
            &state
        )
        .unwrap()
        .ticks,
        4.0,
    );
    assert!(matches!(
        find_with(
            &collision,
            &graph,
            tile(0, 0, 0),
            tile(8, 0, 0),
            FindOptions {
                allow_wilderness: false,
                ..crossed
            },
            &state,
        ),
        Err(RouteError::NoPath),
    ));
}

fn zones_rich_289_state(combat_level: Option<i32>) -> WorldState {
    WorldState {
        combat_level,
        map_members: true,
        quests: [
            "Prince Ali Rescue",
            "Rune Mysteries",
            "Lost City",
            "Shilo Village",
            "Tree Gnome Village",
            "The Grand Tree",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect(),
        inv: HashMap::from([
            (995, 10_000),
            (554, 1_000),
            (555, 1_000),
            (556, 1_000),
            (557, 1_000),
            (558, 1_000),
            (561, 1_000),
            (563, 1_000),
        ]),
        stats: (0..21).map(|skill| (skill, 99)).collect(),
        ..WorldState::empty()
    }
}

fn zone_route_cell_count(route: &crate::router::Route) -> usize {
    route
        .legs
        .iter()
        .map(|leg| match leg {
            Leg::Walk { tiles } => tiles.len(),
            Leg::Transport { .. } => 1,
        })
        .sum()
}

fn assert_zone_route_clear(
    world: &crate::world::NavWorld,
    from: WorldTile,
    route: &crate::router::Route,
    opts: FindOptions,
    state: &WorldState,
) {
    let filter = crate::zones::ZoneFilter::new(
        world.graph.zones.as_ref().expect("v12 zones"),
        state.combat_level,
        &[from, route.dest],
        &opts.zones,
    );
    for leg in &route.legs {
        match leg {
            Leg::Walk { tiles } => {
                for &tile in tiles {
                    assert!(
                        !filter.blocks(&world.graph.wilderness, tile),
                        "zone entered: {tile:?}"
                    );
                }
            }
            Leg::Transport { edge } => {
                assert!(
                    !filter.blocks(&world.graph.wilderness, edge.to),
                    "zone landing: {:?}",
                    edge.to
                );
            }
        }
    }
}

#[test]
fn real_289_zone_reroutes_lifting_endpoint_exemptions_and_stationary_road() {
    let Some(world) = crate::world::NavWorld::load_default_pack_or_skip() else {
        return;
    };
    assert!(
        world.graph.zones.is_some(),
        "this acceptance requires the v12 bake"
    );
    let lumbridge = tile(3222, 3218, 0);
    let bank = tile(3092, 3243, 0);
    let east_bank = tile(3253, 3420, 0);
    let taverley = tile(2895, 3450, 0);
    let catherby = tile(2809, 3440, 0);
    let manor = tile(3109, 3353, 0);
    let opts = FindOptions::default();
    for (row, from, to, combat) in [
        ("F1", lumbridge, bank, Some(126)),
        ("F1-low", lumbridge, bank, Some(3)),
        ("F2-cap", east_bank, tile(3253, 3375, 0), Some(12)),
        ("F2-unknown", east_bank, tile(3253, 3375, 0), None),
        ("F2-lifted", east_bank, tile(3253, 3375, 0), Some(13)),
        ("F3", taverley, catherby, Some(126)),
        ("F3-cap", taverley, catherby, Some(146)),
        ("F3-low", taverley, catherby, Some(3)),
        ("F3-lifted", taverley, catherby, Some(147)),
        ("P1", bank, tile(3123, 3245, 0), Some(3)),
        ("P2", east_bank, tile(3253, 3402, 0), Some(3)),
        ("P5", bank, manor, Some(126)),
        ("P5-low", bank, manor, Some(3)),
        ("P6", lumbridge, manor, Some(126)),
        ("P6-low", lumbridge, manor, Some(3)),
    ] {
        let state = zones_rich_289_state(combat);
        let route = find_with(&world.collision, &world.graph, from, to, opts, &state).unwrap();
        assert_zone_route_clear(&world, from, &route, opts, &state);
        if row.starts_with("P5") || row.starts_with("P6") {
            assert!(
                !route_walk_tiles(&route).any(|t| t == tile(3110, 3339, 0)),
                "{row}"
            );
            assert!(
                route_walk_tiles(&route).any(|t| t == tile(3109, 3339, 0)),
                "{row}"
            );
        }
    }
    let table = world.graph.zones.as_ref().unwrap();
    let r2_squares: Vec<_> = table
        .zones()
        .iter()
        .filter(|zone| zone.shape != crate::zones::NO_SHAPE)
        .map(|zone| AvoidRect {
            min_x: zone.min_x,
            max_x: zone.max_x,
            min_z: zone.min_z,
            max_z: zone.max_z,
            level: Some(i32::from(zone.level)),
        })
        .collect();
    for from in [bank, lumbridge] {
        assert!(
            matches!(
                find_with_avoid(
                    &world.collision,
                    &world.graph,
                    from,
                    manor,
                    FindOptions {
                        zones: ZoneExempt::all(),
                        ..opts
                    },
                    &zones_rich_289_state(Some(126)),
                    &r2_squares,
                ),
                Err(RouteError::NoPath),
            ),
            "R2 stationary squares must close the same manor fixture"
        );
    }
    let jail = ZoneExempt::named(&[table.resolve("draynor-jail-guards").unwrap()]).unwrap();
    let route = find_with(
        &world.collision,
        &world.graph,
        lumbridge,
        bank,
        FindOptions {
            zones: jail,
            ..opts
        },
        &zones_rich_289_state(Some(126)),
    )
    .unwrap();
    assert_eq!((route.ticks, zone_route_cell_count(&route)), (83.5, 168));
    let mountain = ZoneExempt::named(&[table.resolve("white-wolf-mountain").unwrap()]).unwrap();
    for combat in [3, 126, 146] {
        let route = find_with(
            &world.collision,
            &world.graph,
            taverley,
            catherby,
            FindOptions {
                zones: mountain,
                ..opts
            },
            &zones_rich_289_state(Some(combat)),
        )
        .unwrap();
        assert_eq!((route.ticks, zone_route_cell_count(&route)), (125.5, 252));
    }
}

#[test]
fn real_289_refusals_name_the_shortest_unblocked_route_zones() {
    let Some(world) = crate::world::NavWorld::load_default_pack_or_skip() else {
        return;
    };
    let table = world.graph.zones.as_ref().expect("v12 zones");
    let opts = FindOptions::default();
    let from = tile(3222, 3218, 0);
    let to = tile(2664, 3664, 0);
    let state = zones_rich_289_state(Some(126));
    assert!(matches!(
        find_with(&world.collision, &world.graph, from, to, opts, &state),
        Err(RouteError::NoPath)
    ));
    let keys = crate::router::find_blocking_zones(
        &world.collision,
        &world.graph,
        from,
        to,
        opts,
        &state,
        &[],
    )
    .unwrap();
    assert_eq!(
        keys,
        vec![
            table.resolve("white-wolf-mountain").unwrap(),
            table.resolve("wolf@2647,3584,0").unwrap(),
        ]
    );
    let crossing = FindOptions {
        zones: ZoneExempt::named(&keys).unwrap(),
        ..opts
    };
    let route = find_with(&world.collision, &world.graph, from, to, crossing, &state).unwrap();
    assert_eq!(
        (route.ticks, zone_route_cell_count(&route), route.legs.len()),
        (256.5, 502, 7)
    );
    assert!(matches!(
        find_with(
            &world.collision,
            &world.graph,
            from,
            to,
            FindOptions {
                zones: ZoneExempt::named(&keys[..1]).unwrap(),
                ..opts
            },
            &state,
        ),
        Err(RouteError::NoPath),
    ));
    let wolf_exempt = FindOptions {
        zones: ZoneExempt::named(&keys[1..]).unwrap(),
        ..opts
    };
    let route = find_with(
        &world.collision,
        &world.graph,
        from,
        to,
        wolf_exempt,
        &state,
    )
    .unwrap();
    assert_zone_route_clear(&world, from, &route, wolf_exempt, &state);
    for combat in [129, 147] {
        let route = find_with(
            &world.collision,
            &world.graph,
            from,
            to,
            opts,
            &zones_rich_289_state(Some(combat)),
        )
        .unwrap();
        assert_zone_route_clear(
            &world,
            from,
            &route,
            opts,
            &zones_rich_289_state(Some(combat)),
        );
    }
    for members in [false, true] {
        let fresh = WorldState {
            combat_level: Some(3),
            map_members: members,
            stats: (0..21).map(|skill| (skill, 1)).collect(),
            ..WorldState::empty()
        };
        let from = tile(3123, 3245, 0);
        let to = tile(3224, 3200, 0);
        assert!(matches!(
            find_with(&world.collision, &world.graph, from, to, opts, &fresh),
            Err(RouteError::NoPath)
        ));
        let keys = crate::router::find_blocking_zones(
            &world.collision,
            &world.graph,
            from,
            to,
            opts,
            &fresh,
            &[],
        )
        .unwrap();
        assert_eq!(
            keys,
            vec![
                table.resolve("brownbear@3176,3223,0").unwrap(),
                table.resolve("giantrat1@3211,3195,0").unwrap(),
            ]
        );
        let route = find_with(
            &world.collision,
            &world.graph,
            from,
            to,
            FindOptions {
                zones: ZoneExempt::named(&keys).unwrap(),
                ..opts
            },
            &fresh,
        )
        .unwrap();
        assert_eq!(
            (route.ticks, zone_route_cell_count(&route), route.legs.len()),
            (53.5, 108, 3)
        );
    }
}

#[test]
fn real_289_named_bank_many_matches_every_single_without_goal_partitions() {
    let Some(world) = crate::world::NavWorld::load_default_pack_or_skip() else {
        return;
    };
    let data = api::game_data::for_revision(client::io::ClientRevision::R289).unwrap();
    world.bind_named_bank_facts(&data).unwrap();
    let banks = world.named_bank_facts().unwrap();
    let targets: Vec<_> = banks
        .banks()
        .iter()
        .filter(|b| b.routable)
        .map(|b| b.tile)
        .collect();
    assert_eq!(targets.len(), 19);
    for (from, combat, changed) in [
        (tile(3222, 3218, 0), 126, 8),
        (tile(3222, 3218, 0), 3, 17),
        (tile(3016, 9840, 0), 126, 7),
        (tile(3016, 9840, 0), 3, 13),
    ] {
        let state = zones_rich_289_state(Some(combat));
        let opts = FindOptions::default();
        let many = find_many_with(&world.collision, &world.graph, from, &targets, opts, &state);
        let legacy = find_many_with(
            &world.collision,
            &world.graph,
            from,
            &targets,
            FindOptions {
                zones: ZoneExempt::all(),
                ..opts
            },
            &state,
        );
        assert_eq!(many.completion_partitions(), 0);
        assert_eq!(many.results().iter().filter(|r| r.is_ok()).count(), 17);
        assert_eq!(
            many.results()
                .iter()
                .zip(legacy.results())
                .filter(|(a, b)| {
                    a.as_ref().map(|cost| cost.ticks) != b.as_ref().map(|cost| cost.ticks)
                })
                .count(),
            changed
        );
        for (index, &target) in targets.iter().enumerate() {
            let single = find_with(&world.collision, &world.graph, from, target, opts, &state);
            match (many.route(index), single) {
                (Ok(many), Ok(single)) => assert_eq!(
                    many.ticks, single.ticks,
                    "from={from:?} target={target:?} combat={combat}"
                ),
                (Err(TargetError::NoPath), Err(RouteError::NoPath)) => {}
                pair => panic!("many/single differ: {pair:?}"),
            }
        }
    }
}

#[test]
fn real_289_grouped_goals_merge_their_own_endpoint_completion_costs() {
    let Some(world) = crate::world::NavWorld::load_default_pack_or_skip() else {
        return;
    };
    let from = tile(3222, 3218, 0);
    let targets = [
        tile(3092, 3243, 0),
        tile(3123, 3245, 0),
        tile(3253, 3402, 0),
    ];
    for (combat, partitions, ticks) in [(3, 2, [87.5, 70.5, 109.5]), (126, 1, [84.5, 67.5, 107.5])]
    {
        let state = zones_rich_289_state(Some(combat));
        let opts = FindOptions::default();
        let many = find_many_with(&world.collision, &world.graph, from, &targets, opts, &state);
        assert_eq!(many.completion_partitions(), partitions);
        for (index, &target) in targets.iter().enumerate() {
            let single =
                find_with(&world.collision, &world.graph, from, target, opts, &state).unwrap();
            assert_eq!(
                (many.route(index).unwrap().ticks, single.ticks),
                (ticks[index], ticks[index])
            );
        }
        let first = find_first_with(&world.collision, &world.graph, from, &targets, opts, &state);
        assert_eq!(first.completion_partitions(), partitions);
        let first = first.into_route().unwrap();
        assert_eq!((first.dest, first.ticks), (targets[1], ticks[1]));
    }
}

#[test]
fn zoned_first_keeps_preferred_priority_and_does_not_use_fallback_zones_for_transit() {
    let collision = bake(5, 1, &[]);
    let mut graph = TransportGraph::default();
    install_zones(&collision, &mut graph, vec![rect_zone(2, 2, 0, 0)]);
    let from = tile(0, 0, 0);
    let zoned = tile(2, 0, 0);
    let state = WorldState::empty();
    let preferred = find_first_with_fallback(
        &collision,
        &graph,
        from,
        &[zoned],
        &[from],
        FindOptions::default(),
        &state,
    );
    assert_eq!(
        (
            preferred.route().unwrap().dest,
            preferred.route().unwrap().ticks
        ),
        (zoned, 1.0)
    );
    assert!(
        preferred.fallback().is_none(),
        "a preferred completion wins over the zero-cost fallback"
    );
    let refused = find_first_with_fallback(
        &collision,
        &graph,
        from,
        &[tile(4, 0, 0)],
        &[zoned],
        FindOptions::default(),
        &state,
    );
    assert_eq!(refused.route(), Err(RouteError::NoPath));
    assert!(
        matches!(refused.fallback(), Some(FallbackRoute::Routed(route))
        if route.dest == zoned && route.ticks == 1.0)
    );
    let empty = find_first_with_fallback(
        &collision,
        &graph,
        from,
        &[],
        &[zoned],
        FindOptions::default(),
        &state,
    );
    assert!(matches!(empty.fallback(), Some(FallbackRoute::Undecided)));
    assert_eq!(empty.completion_partitions(), 0);
}

#[test]
fn expired_many_deadline_cannot_publish_a_completion_route() {
    let collision = bake(5, 1, &[]);
    let mut graph = TransportGraph::default();
    install_zones(&collision, &mut graph, vec![rect_zone(2, 2, 0, 0)]);
    let goals = [tile(2, 0, 0), tile(4, 0, 0)];
    let search = find_many_with_avoid_bounded_until(
        &collision,
        &graph,
        tile(0, 0, 0),
        &goals,
        FindOptions::default(),
        &WorldState::empty(),
        &[],
        BANK_TARGET_BUDGET,
        Some(Instant::now() - Duration::from_secs(1)),
    );
    assert!(!search.complete());
    assert_eq!(search.completion_partitions(), 1);
    assert_eq!(
        search.results(),
        &[Err(TargetError::NotSettled), Err(TargetError::NotSettled)]
    );
    assert!(search.route(0).is_err());
    assert!(search.route(1).is_err());
}

#[test]
fn linked_battle_mage_hunts_on_the_raw_plane_used_by_routes() {
    let Some(world) = crate::world::NavWorld::load_default_pack_or_skip() else {
        return;
    };
    let table = world.graph.zones.as_ref().expect("v12 zones");
    let mage = table.resolve("zamorak_mage@3100,3927,0").unwrap();
    assert!(table
        .at(tile(3100, 3927, 0))
        .any(|index| table.key(index) == mage));
    assert!(!table
        .at(tile(3100, 3927, 1))
        .any(|index| table.key(index) == mage));
    let from = tile(3090, 3917, 0);
    let to = tile(3110, 3937, 0);
    let opts = FindOptions {
        allow_wilderness: true,
        ..Default::default()
    };
    let state = WorldState {
        combat_level: Some(126),
        ..WorldState::empty()
    };
    let crossing = find_with(
        &world.collision,
        &world.graph,
        from,
        to,
        FindOptions {
            zones: ZoneExempt::all(),
            ..opts
        },
        &state,
    )
    .unwrap();
    assert!(crossing.legs.iter().any(|leg| match leg {
        Leg::Walk { tiles } => tiles
            .iter()
            .any(|&cell| { table.at(cell).any(|index| table.key(index) == mage) }),
        Leg::Transport { edge } => table.at(edge.to).any(|index| table.key(index) == mage),
    }));
    assert!(matches!(
        find_with(&world.collision, &world.graph, from, to, opts, &state),
        Err(RouteError::NoPath),
    ));
    let blocked = crate::router::find_blocking_zones(
        &world.collision,
        &world.graph,
        from,
        to,
        opts,
        &state,
        &[],
    )
    .expect("active zone witness");
    assert!(blocked.contains(&mage), "{blocked:?}");
}
