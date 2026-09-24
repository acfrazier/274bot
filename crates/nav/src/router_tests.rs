use api::obj_names::LocDefs;
use api::snapshot::WorldTile;
use client::config::{Cache, LocType};
use client::dash3d::CollisionFlag;
use client::io::JagFile;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::collision::{bake_from_maps, WorldCollision};
use crate::grid::StepGrid;
use crate::router::{
    find, find_allow_teleports, find_bounded, find_many_with, find_many_with_avoid_bounded,
    find_many_with_avoid_bounded_until, find_missing_item_reqs, find_missing_item_reqs_with_avoid,
    find_on_grid, find_with, find_with_avoid, find_with_avoid_bounded, find_with_model,
    local_step_component, step_ok, AvoidRect, CostModel, FindOptions, GridLeg, Leg, MissingReq,
    RouteError, TargetError, BANK_TARGET_BUDGET, PER_STEP_WALK,
};
use crate::tile::Tile;
use crate::transport::{
    derive_transports, TransportEdge, TransportGraph, TransportKind, WildernessRules,
    WildernessZone,
};
use crate::world_state::WorldState;

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
    };
    let mut graph = TransportGraph::default();
    graph.at.entry(at).or_default().push(0);
    graph.edges.push(edge);
    graph
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
    let raw: Vec<_> = world.banks().iter().map(|b| b.tile).collect();
    assert!(
        raw.len() >= 64,
        "289 bank pack must contain the 64-stand workload"
    );
    let ring = [
        (0, 1),
        (0, -1),
        (1, 0),
        (-1, 0),
        (-1, -1),
        (1, -1),
        (-1, 1),
        (1, 1),
    ];
    let resolved: Vec<_> = raw
        .iter()
        .filter_map(|booth| {
            ring.iter()
                .map(|&(dx, dz)| tile(booth.x + dx, booth.z + dz, booth.level))
                .find(|&stand| !raw.contains(&stand) && world.collision.walkable(stand))
        })
        .collect();
    eprintln!(
        "289 packed bank booth targets: {}, adjacent walk stands: {} ({} unresolved)",
        raw.len(),
        resolved.len(),
        raw.len() - resolved.len()
    );
    let mut targets = raw.clone();
    targets.extend_from_slice(&resolved);
    let empty = WorldState::empty();
    let members = WorldState {
        map_members: true,
        quests: HashSet::from([
            "Prince Ali Rescue".to_string(),
            "Rune Mysteries".to_string(),
        ]),
        inv: HashMap::from([(995, 10), (554, 100), (556, 100), (563, 100)]),
        stats: HashMap::from([(6, 99)]),
        ..WorldState::empty()
    };
    // The default two origins compare every booth target and every resolved
    // stand against independent find_with (including blocked/raw booth errors).
    for (label, from, state) in [
        ("lumbridge", tile(3222, 3218, 0), &empty),
        ("dwarven_mine", tile(3016, 9840, 0), &members),
    ] {
        let start = Instant::now();
        let many = find_many_with(
            &world.collision,
            &world.graph,
            from,
            &targets,
            FindOptions::default(),
            state,
        );
        let shared = start.elapsed();
        let start = Instant::now();
        let mut successes = 0usize;
        for (index, &target) in targets.iter().enumerate() {
            let independent = find_with(
                &world.collision,
                &world.graph,
                from,
                target,
                FindOptions::default(),
                state,
            );
            match independent {
                Ok(expected) => {
                    successes += 1;
                    assert_eq!(
                        many.results()[index].as_ref().unwrap().ticks,
                        expected.ticks,
                        "{label} target {index} {target:?}"
                    );
                    let actual = many.route(index).unwrap();
                    assert_eq!(actual.dest, target);
                    assert_eq!(actual.ticks, expected.ticks);
                    validate_real_route(&world.collision, &actual, state, false, &[]);
                }
                Err(error) => assert_eq!(
                    many.results()[index],
                    Err(error.into()),
                    "{label} target {index} {target:?}"
                ),
            }
        }
        eprintln!("bank real 289 {label}: {} targets, {successes} successes, {} settled, shared {:?}, independent {:?}",
            targets.len(), many.settled(), shared, start.elapsed());
        assert!(
            resolved
                .iter()
                .enumerate()
                .any(|(i, _)| many.results()[raw.len() + i].is_ok()),
            "resolved walk stands must include reachable banks"
        );
    }

    // Same fixed query settings for each comparison. Avoidances, wilderness,
    // essence and admitted native teleports use the same shared kernel.
    let probes: Vec<_> = resolved
        .iter()
        .copied()
        .take(8)
        .chain([
            tile(3013, 3355, 0),
            tile(3094, 3489, 0),
            tile(3213, 3424, 0),
        ])
        .collect();
    let avoid = [AvoidRect {
        min_x: 3015,
        max_x: 3017,
        min_z: 9839,
        max_z: 9841,
        level: Some(0),
    }];
    for (from, state, opts, rects, budget) in [
        (
            tile(3016, 9840, 0),
            &empty,
            FindOptions::default(),
            &[][..],
            BANK_TARGET_BUDGET,
        ),
        (
            tile(3016, 9840, 0),
            &members,
            FindOptions {
                allow_wilderness: true,
                ..FindOptions::default()
            },
            &avoid[..],
            BANK_TARGET_BUDGET,
        ),
        (
            tile(3222, 3218, 0),
            &members,
            FindOptions {
                allow_teleports: true,
                ..FindOptions::default()
            },
            &[][..],
            BANK_TARGET_BUDGET,
        ),
        (
            tile(3222, 3218, 0),
            &empty,
            FindOptions::default(),
            &[][..],
            50,
        ),
    ] {
        let many = find_many_with_avoid_bounded(
            &world.collision,
            &world.graph,
            from,
            &probes,
            opts,
            state,
            rects,
            budget,
        );
        if budget == 50 {
            assert!(
                many.results().contains(&Err(TargetError::BudgetExhausted)),
                "real pack bounded row must exercise the budget error"
            );
        }
        for (index, &target) in probes.iter().enumerate() {
            let independent = find_with_avoid_bounded(
                &world.collision,
                &world.graph,
                from,
                target,
                opts,
                state,
                rects,
                budget,
            );
            match independent {
                Ok(route) => {
                    assert_eq!(
                        many.results()[index].as_ref().unwrap().ticks,
                        route.ticks,
                        "matrix target {index}, origin {from:?}"
                    );
                    validate_real_route(
                        &world.collision,
                        &many.route(index).unwrap(),
                        state,
                        opts.allow_wilderness,
                        rects,
                    );
                }
                Err(error) => assert_eq!(
                    many.results()[index],
                    Err(error.into()),
                    "matrix target {index}, origin {from:?}"
                ),
            }
        }
    }
    // Find-only timing receipt: 64 packed bank placements, using an
    // adjacent walk stand wherever the selected world supplies one and raw
    // interact tiles for unresolved placements. Both loops use this exact
    // target list/state. Timings are diagnostic, not a CI threshold.
    let mut workload: Vec<_> = resolved.iter().copied().take(64).collect();
    workload.extend(raw.iter().copied().take(64 - workload.len()));
    let from = tile(3222, 3218, 0);
    let start = Instant::now();
    let many = find_many_with(
        &world.collision,
        &world.graph,
        from,
        &workload,
        FindOptions::default(),
        &empty,
    );
    let shared = start.elapsed();
    let start = Instant::now();
    let individual: Vec<_> = workload
        .iter()
        .map(|&target| {
            find_with(
                &world.collision,
                &world.graph,
                from,
                target,
                FindOptions::default(),
                &empty,
            )
        })
        .collect();
    let naive = start.elapsed();
    let success = individual.iter().filter(|result| result.is_ok()).count();
    for (index, result) in individual.iter().enumerate() {
        assert_eq!(
            many.results()[index]
                .as_ref()
                .map(|cost| cost.ticks)
                .map_err(|&e| e),
            result
                .as_ref()
                .map(|route| route.ticks)
                .map_err(|&e| e.into())
        );
    }
    eprintln!("bank 64-stand real 289: shared {:?}, naive {:?}, {} settled, {success} successes, {} errors, scratch capacities {:?}",
        shared, naive, many.settled(), 64-success, many.scratch_capacities());
}

fn validate_real_route(
    collision: &WorldCollision,
    route: &crate::router::Route,
    state: &WorldState,
    allow_wilderness: bool,
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
                    assert!(super::wildy_step_ok(step[0], step[1], allow_wilderness));
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
                if edge.kind != TransportKind::Teleport {
                    assert!(collision.standable(previous));
                    assert_eq!(previous.level, edge.at.level);
                    assert!(
                        (previous.x - edge.at.x)
                            .abs()
                            .max((previous.z - edge.at.z).abs())
                            <= 1
                    );
                }
                assert!(super::wildy_step_ok(previous, edge.to, allow_wilderness));
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
    let g = TransportGraph::default();
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
    let g = TransportGraph::default();
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
    let g = teleport(dest, 3, vec![(6, 25)], vec![(554, 1), (556, 3), (563, 1)]);
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
    assert!(matches!(
        find_with_avoid(
            &wc_w,
            &TransportGraph::default(),
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
    let ladder_at = WorldTile {
        x: 3096,
        z: 9867,
        level: 0,
    };
    let trap_edges: Vec<_> = world
        .graph
        .at
        .get(&trapdoor_at)
        .into_iter()
        .flatten()
        .map(|&i| &world.graph.edges[i])
        .collect();
    let ladder_edges: Vec<_> = world
        .graph
        .at
        .get(&ladder_at)
        .into_iter()
        .flatten()
        .map(|&i| &world.graph.edges[i])
        .collect();
    assert!(
        ladder_edges.iter().any(|e| e.loc_id == 1755),
        "packed graph must keep the dungeon exit ladder 1755 at 3096,9867, got {:?}",
        ladder_edges
            .iter()
            .map(|e| (e.loc_id, e.to, e.kind))
            .collect::<Vec<_>>()
    );
    let opts = FindOptions {
        allow_teleports: false,
        allow_wilderness: true,
        allow_bank_fetch: true,
        ..FindOptions::default()
    };
    let mut graph = TransportGraph {
        edges: world.graph.edges.clone(),
        at: world.graph.at.clone(),
        teleports: world.graph.teleports.clone(),
        wilderness: world.graph.wilderness.clone(),
    };
    if !trap_edges
        .iter()
        .any(|e| e.loc_id == 1568 || e.loc_id == 1570)
    {
        let dest = WorldTile {
            x: trapdoor_at.x,
            z: trapdoor_at.z + crate::transport::CELLAR_SHIFT,
            level: trapdoor_at.level,
        };
        let idx = graph.edges.len();
        graph.edges.push(TransportEdge {
            kind: TransportKind::Ladder,
            at: trapdoor_at,
            to: dest,
            loc_id: 1568,
            option: 1,
            ticks: 3,
            dir: None,
            open_loc_id: Some(1570),
            skill_req: vec![],
            item_req: vec![],
            quest_req: vec![],
            varp_req: vec![],
            worn_req: vec![],
            members_req: false,
            wildy_cap: None,
        });
        graph.at.entry(trapdoor_at).or_default().push(idx);
    }
    let route = find_with(
        &world.collision,
        &graph,
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

#[test]
fn packed_wildy_wolf_pit_reaches_ridge_approach() {
    let content = PathBuf::from("/Users/acfrazier/experiments/lostcity-289/content");
    let jag = PathBuf::from("/Users/acfrazier/.274bot/unpack-289/config");
    if !content.join("maps/m46_61.jm2").is_file() {
        eprintln!("SKIP: no 289 wilderness mapsquare at {}", content.display());
        return;
    }
    let Ok(bytes) = fs::read(&jag) else {
        eprintln!("SKIP: no 289 config jag at {}", jag.display());
        return;
    };
    let defs = LocDefs::from_locs(&Cache::unpack(&JagFile::new(bytes)).locs);
    let tmp = std::env::temp_dir().join(format!("wildy-pit-maps-{}", std::process::id()));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).unwrap();
    fs::copy(content.join("maps/m46_61.jm2"), tmp.join("m46_61.jm2")).unwrap();
    let collision = bake_from_maps(&tmp, &defs, &HashSet::new()).expect("bake m46_61");
    let _ = fs::remove_dir_all(&tmp);
    let graph = derive_transports(&content, &defs, &collision);
    let pit = WorldTile {
        x: 3001,
        z: 3923,
        level: 0,
    };
    let ridge = WorldTile {
        x: 2998,
        z: 3924,
        level: 0,
    };
    let approach = WorldTile {
        x: 2998,
        z: 3916,
        level: 0,
    };
    let opts = FindOptions {
        allow_teleports: false,
        allow_wilderness: true,
        allow_bank_fetch: true,
        ..FindOptions::default()
    };
    let state = WorldState::empty().with_map_members(true);
    find_with(&collision, &graph, pit, approach, opts, &state).unwrap_or_else(|e| {
            panic!("wolf pit (3001,3923) -> ridge approach (2998,3916) must walk around the east railings: {e:?}")
        });
    assert!(
            matches!(
                find_with(&collision, &graph, ridge, approach, opts, &state),
                Err(RouteError::NoPath)
            ),
            "the ridge corridor cannot walk south through loc_2309; recovery is from the pit after the fall"
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

fn teleport_takeoff(start: WorldTile, route: &crate::router::Route) -> Vec<(WorldTile, i32)> {
    let mut cur = start;
    let mut out = Vec::new();
    for leg in &route.legs {
        match leg {
            Leg::Walk { tiles } => {
                if let Some(last) = tiles.last() {
                    cur = *last;
                }
            }
            Leg::Transport { edge } => {
                if edge.kind == TransportKind::Teleport {
                    if let Some(cap) = edge.wildy_cap {
                        out.push((cur, cap));
                    }
                }
                cur = edge.to;
            }
        }
    }
    out
}

#[test]
fn real_pack_refuses_teleports_above_derived_caps() {
    let Some(world) = crate::world::NavWorld::load_default_pack_or_skip() else {
        return;
    };
    if world.graph.wilderness.zones.is_empty()
        || world.graph.teleports.iter().all(|e| e.wildy_cap.is_none())
    {
        eprintln!("SKIP: packed graph has no derived wilderness teleport caps");
        return;
    }
    let spell = world
        .graph
        .teleports
        .iter()
        .find(|e| e.loc_id == 0 && e.wildy_cap.is_some())
        .expect("packed spell teleport with a cap");
    let spell_cap = spell.wildy_cap.unwrap();
    let dest = spell.to;
    let mut inv = HashMap::new();
    for &(id, n) in &spell.item_req {
        inv.insert(id, n.max(5));
    }
    let magic = spell
        .skill_req
        .iter()
        .find(|(id, _)| *id == 6)
        .map(|(_, lvl)| *lvl)
        .unwrap_or(99);
    let state = WorldState {
        stats: HashMap::from([(6, magic)]),
        inv,
        ..WorldState::default()
    };
    let opts = teleport_opts();
    let below = standable_wildy_level(&world, spell_cap).expect("standable tile at spell cap");
    let above =
        standable_wildy_level(&world, spell_cap + 1).expect("standable tile above spell cap");
    let r_below = find_with(&world.collision, &world.graph, below, dest, opts, &state)
        .unwrap_or_else(|e| panic!("from level {spell_cap} {below:?} -> {dest:?}: {e:?}"));
    assert!(
        r_below
            .legs
            .iter()
            .any(|l| matches!(l, Leg::Transport { edge } if edge.kind == TransportKind::Teleport)),
        "from below the cap the route still teleports"
    );
    for (from, cap) in teleport_takeoff(below, &r_below) {
        assert!(
            world.graph.wilderness.level(from) <= cap,
            "teleport from {from:?} level {} > cap {cap}",
            world.graph.wilderness.level(from)
        );
    }
    let r_above = find_with(&world.collision, &world.graph, above, dest, opts, &state);
    if let Ok(route) = r_above {
        for (from, cap) in teleport_takeoff(above, &route) {
            assert!(
                world.graph.wilderness.level(from) <= cap,
                "above-cap start {above:?} teleported from {from:?} level {} > {cap}",
                world.graph.wilderness.level(from)
            );
            assert_ne!(
                from, above,
                "must not teleport from the above-cap origin {above:?}"
            );
        }
    }
    let glory_cap = world
        .graph
        .teleports
        .iter()
        .filter(|e| e.loc_id > 0)
        .filter_map(|e| e.wildy_cap)
        .max()
        .expect("jewellery cap");
    if glory_cap > spell_cap {
        let glory = world
            .graph
            .teleports
            .iter()
            .find(|e| e.wildy_cap == Some(glory_cap))
            .unwrap();
        let gstate = WorldState {
            inv: HashMap::from([(glory.item_req[0].0, 1)]),
            ..WorldState::default()
        };
        let g_below =
            standable_wildy_level(&world, glory_cap).expect("standable tile at glory cap");
        let g_above =
            standable_wildy_level(&world, glory_cap + 1).expect("standable tile above glory cap");
        let ok = find_with(
            &world.collision,
            &world.graph,
            g_below,
            glory.to,
            opts,
            &gstate,
        )
        .unwrap_or_else(|e| panic!("glory from level {glory_cap}: {e:?}"));
        assert!(ok
            .legs
            .iter()
            .any(|l| matches!(l, Leg::Transport { edge } if edge.kind == TransportKind::Teleport)));
        if let Ok(route) = find_with(
            &world.collision,
            &world.graph,
            g_above,
            glory.to,
            opts,
            &gstate,
        ) {
            for (from, cap) in teleport_takeoff(g_above, &route) {
                assert!(world.graph.wilderness.level(from) <= cap);
                assert_ne!(from, g_above);
            }
        }
    }
}

fn standable_wildy_level(world: &crate::world::NavWorld, want: i32) -> Option<WorldTile> {
    let rules = &world.graph.wilderness;
    if rules.divisor <= 0 {
        return None;
    }
    for zone in &rules.zones {
        if zone.level1 > 0 {
            continue;
        }
        let z0 = zone.origin_z + (want - rules.offset) * rules.divisor;
        let z1 = z0 + rules.divisor - 1;
        for z in z0..=z1 {
            for x in (zone.x1..=zone.x2).step_by(3) {
                let t = WorldTile { x, z, level: 0 };
                if world.collision.standable(t) {
                    return Some(t);
                }
            }
        }
    }
    None
}
