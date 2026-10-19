use api::identity::{Cancellation, Deadline};
use api::obj_names::LocDefs;
use api::selected::QuestGate;
use api::snapshot::WorldTile;
use api::work::{
    InspectionPhase, MonotonicClock, SharedCancellation, SliceBudget, StepContext, SystemClock,
};
use client::config::LocType;
use client::dash3d::CollisionFlag;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::future::Future;
use std::num::NonZeroU32;
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
    find, find_allow_teleports, find_bounded, find_first_missing_item_reqs_with_avoid,
    find_first_with, find_first_with_fallback, find_many_with, find_many_with_avoid_bounded,
    find_many_with_avoid_bounded_until, find_missing_item_reqs, find_missing_item_reqs_with_avoid,
    find_on_grid, find_unresolved_quest_gates, find_with, find_with_avoid, find_with_avoid_bounded,
    find_with_model, local_step_component, missing_item_reqs, step_ok, AvoidRect, CostModel,
    FallbackRoute, FindOptions, GridLeg, Leg, MissingReq, ReverseProof, RouteError, TargetError,
    BANK_TARGET_BUDGET, FIRST_TARGET_BUDGET, PER_STEP_WALK,
};
use crate::tile::Tile;
use crate::transport::{
    TransportEdge, TransportGraph, TransportKind, WildernessRules, WildernessZone,
};
use crate::world_state::WorldState;
use crate::zones::{Zone, ZoneClass, ZoneExempt, ZoneFilterOwned, ZoneKey, ZoneKind, ZoneTable};

#[test]
fn synchronous_driver_skips_suspension_and_restores_cooperative_mode() {
    fn unit_is_ready() -> bool {
        let mut unit = super::work_unit();
        let mut context = std::task::Context::from_waker(std::task::Waker::noop());
        Future::poll(std::pin::Pin::new(&mut unit), &mut context).is_ready()
    }

    assert!(!unit_is_ready());
    super::drive_to_completion(async {
        assert!(unit_is_ready());
        super::drive_to_completion(async {
            assert!(unit_is_ready());
        });
        assert!(unit_is_ready());
    });
    assert!(!unit_is_ready());

    let panicked = std::panic::catch_unwind(|| {
        super::drive_to_completion(async {
            assert!(unit_is_ready());
            panic!("exercise the synchronous polling guard");
        });
    });
    assert!(panicked.is_err());
    assert!(!unit_is_ready());
}

#[cfg(unix)]
#[test]
fn cooperative_mapping_failure_returns_typed_allocation_error() {
    let world = Arc::new(crate::world::NavWorld::from_parts(
        bake(4096, 1, &[]),
        TransportGraph::default(),
        Vec::new(),
    ));
    let lease = crate::producer::lease(&world).unwrap();
    let request = super::SearchRequest::Single {
        from: tile(0, 0, 0),
        to: tile(4095, 0, 0),
        options: FindOptions::default(),
        avoid: Box::default(),
        max_settles: 4096,
    };
    let mut cursor =
        super::SearchCursor::new(request, lease, Arc::new(WorldState::empty())).unwrap();
    let clock = SystemClock;
    let cancellation = SharedCancellation::new();
    super::zeroed_buffer::with_mapping_failure(|| {
        for _ in 0..128 {
            let budget = SliceBudget::new(
                NonZeroU32::new(64).unwrap(),
                Deadline::after(clock.now(), Duration::from_secs(5)),
            );
            let mut context = StepContext::<InspectionPhase>::new(&clock, budget, &cancellation);
            match cursor.advance(&mut context) {
                super::CursorStep::Failed(error) => {
                    assert!(matches!(error, super::SearchCursorError::Allocation));
                    return;
                }
                super::CursorStep::Yielded => {}
                super::CursorStep::Complete(_) | super::CursorStep::Cancelled(_) => {
                    panic!("a refused mapping must fail the cursor")
                }
            }
        }
        panic!("a refused mapping must fail within the finite slice limit");
    });
}

#[cfg(unix)]
#[test]
fn synchronous_mapping_failure_preserves_budget_exhausted() {
    let collision = bake(4096, 1, &[]);
    let graph = TransportGraph::default();
    let state = WorldState::empty();
    super::zeroed_buffer::with_mapping_failure(|| {
        assert_eq!(
            find_with_avoid_bounded(
                &collision,
                &graph,
                tile(0, 0, 0),
                tile(4095, 0, 0),
                FindOptions::default(),
                &state,
                &[],
                4096,
            ),
            Err(RouteError::BudgetExhausted),
        );
    });
}

#[test]
fn non_proof_goals_settle_without_a_future() {
    use super::{Goals, Settle, TargetCost};

    let target = WorldTile {
        x: 2,
        z: 0,
        level: 0,
    };
    let other = WorldTile { x: 1, ..target };
    let cost = TargetCost {
        ticks: 3.0,
        settled_at: 7,
    };
    let mut single = Goals::Single {
        to: target,
        cost: None,
    };
    assert!(matches!(
        single.settle_non_first(other, cost),
        Settle::Continue
    ));
    assert!(matches!(
        single.settle_non_first(target, cost),
        Settle::Stop
    ));
    assert!(matches!(single, Goals::Single { cost: Some(found), .. } if found == cost));

    let unique = HashMap::from([(target, 0)]);
    let mut costs = [Err(TargetError::NotSettled)];
    let mut many = Goals::Many {
        unique: &unique,
        costs: &mut costs,
        remaining: 1,
    };
    assert!(matches!(
        many.settle_non_first(other, cost),
        Settle::Continue
    ));
    assert!(matches!(many.settle_non_first(target, cost), Settle::Stop));
    assert_eq!(costs, [Ok(cost)]);
}

#[test]
fn pinned_reverse_admission_preserves_seen_and_queue() {
    let collision = bake(3, 1, &[]);
    let graph = TransportGraph::default();
    let state = WorldState::empty();
    let mut closure = super::drive_to_completion(super::ReverseClosure::new(super::ProofGates {
        collision: &collision,
        graph: &graph,
        state: &state,
        essence: None,
        from: tile(0, 0, 0),
        use_teleports: false,
        allow_wilderness: false,
        relax: super::Relax::Strict,
        inventory_can_grow: false,
        cooperative_storage: false,
    }))
    .unwrap();
    for (candidate, expected) in [
        (tile(1, 0, 0), None),
        (tile(1, 0, 0), None),
        (tile(0, 0, 0), Some(ReverseProof::Reachable)),
    ] {
        let mut future = std::pin::pin!(closure.admit(candidate));
        let mut context = std::task::Context::from_waker(std::task::Waker::noop());
        assert!(matches!(
            Future::poll(future.as_mut(), &mut context),
            std::task::Poll::Ready(Ok(actual)) if actual == expected
        ));
    }
    assert_eq!(closure.seen.len(), 2);
    assert_eq!(closure.queue.len(), 1);
    assert_eq!(closure.queue.front(), Some(&tile(1, 0, 0)));
}

#[test]
fn cooperative_cursor_checks_the_clock_once_per_work_boundary() {
    struct CountingClock {
        now: Instant,
        reads: std::cell::Cell<u32>,
    }

    impl MonotonicClock for CountingClock {
        fn now(&self) -> Instant {
            self.reads.set(self.reads.get() + 1);
            self.now
        }
    }

    let clock = CountingClock {
        now: Instant::now(),
        reads: std::cell::Cell::new(0),
    };
    let cancellation = SharedCancellation::new();
    let mut cursor = super::SearchCursor {
        future: Some(Box::pin(async {
            for _ in 0..128 {
                super::work_unit().await;
            }
            Err(super::SearchCursorError::AlreadyCompleted)
        })),
        progress: Arc::default(),
        input_bytes: 0,
        reservation: super::SearchReservation::default(),
    };
    let budget = SliceBudget::new(
        NonZeroU32::new(64).unwrap(),
        Deadline::after(clock.now, Duration::from_secs(1)),
    );
    let mut context = StepContext::<InspectionPhase>::new(&clock, budget, &cancellation);

    assert!(matches!(
        cursor.advance(&mut context),
        super::CursorStep::Yielded
    ));
    assert_eq!(context.units_this_slice(), 64);
    assert_eq!(
        clock.reads.get(),
        65,
        "check once before the first poll and after each consumed unit"
    );
}

#[test]
fn cooperative_cursor_returns_cancelled_and_drops_the_future() {
    let clock = SystemClock;
    let cancellation = SharedCancellation::new();
    let mut cursor = super::SearchCursor {
        future: Some(Box::pin(async {
            loop {
                super::work_unit().await;
            }
        })),
        progress: Arc::default(),
        input_bytes: 0,
        reservation: super::SearchReservation::default(),
    };
    let now = clock.now();
    let budget = SliceBudget::new(
        NonZeroU32::new(8).unwrap(),
        Deadline::after(now, Duration::from_secs(1)),
    );
    let mut context = StepContext::<InspectionPhase>::new(&clock, budget, &cancellation);
    assert!(matches!(
        cursor.advance(&mut context),
        super::CursorStep::Yielded
    ));
    assert!(
        cursor.future.is_some(),
        "a yielded search still owns its future"
    );

    cancellation
        .register(Cancellation::Operator)
        .expect("first cancellation wins");
    let budget = SliceBudget::new(
        NonZeroU32::new(8).unwrap(),
        Deadline::after(now, Duration::from_secs(1)),
    );
    let mut context = StepContext::<InspectionPhase>::new(&clock, budget, &cancellation);
    assert!(matches!(
        cursor.advance(&mut context),
        super::CursorStep::Cancelled(Cancellation::Operator)
    ));
    assert!(
        cursor.future.is_none(),
        "cancellation must drop the search future"
    );
}

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
        takeoff: None,
        worn_all_req: Vec::new(),
        forbidden_held: Vec::new(),
        forbidden_worn: Vec::new(),
        kind: TransportKind::Door,
        player_delta: None,
        at,
        to,
        loc_id: 1530,
        option: 1,
        ticks,
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
    };
    let mut graph = TransportGraph::default();
    graph.at.entry(at).or_default().push(0);
    graph.edges.push(edge);
    graph
}

#[test]
fn find_skips_relative_transport_when_takeoff_landing_overflows() {
    let collision = walled_5x5();
    let mut graph = door(tile(2, 2, 0), tile(4, 2, 0), 1);
    graph.edges[0].kind = TransportKind::Ladder;
    graph.edges[0].player_delta = Some(tile(i32::MAX, 0, 0));

    assert_eq!(
        find(&collision, &graph, tile(1, 2, 0), tile(4, 2, 0)).err(),
        Some(RouteError::NoPath)
    );
}

#[test]
fn exact_takeoff_is_the_only_indexed_transport_stand() {
    let collision = bake(10, 10, &[]);
    let required = tile(4, 3, 0);
    let mut graph = door(tile(4, 4, 0), tile(9, 9, 0), 1);
    graph.edges[0].kind = TransportKind::AgilityShortcut;
    graph.edges[0].takeoff = Some(required);
    graph.rebuild_index(&collision);

    assert_eq!(graph.at.get(&required).map(Vec::as_slice), Some(&[0][..]));
    assert!(!graph.at.contains_key(&tile(4, 4, 0)));
    let route = find(&collision, &graph, tile(4, 2, 0), tile(9, 9, 0)).unwrap();
    let (Leg::Walk { tiles }, Leg::Transport { edge }) = (&route.legs[0], &route.legs[1]) else {
        panic!("expected walk to exact takeoff followed by the transport");
    };
    assert_eq!(tiles.last(), Some(&required));
    assert_eq!(edge.takeoff, Some(required));
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
        takeoff: None,
        worn_all_req: Vec::new(),
        forbidden_held: Vec::new(),
        forbidden_worn: Vec::new(),
        kind: TransportKind::Teleport,
        player_delta: None,
        at: tile(0, 0, 0),
        to,
        loc_id: 0,
        option: 0,
        ticks,
        dir: None,
        open_loc_id: None,
        skill_req,
        item_req: vec![],
        consumed_req: item_req,
        item_returns: vec![],
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
fn router_prefers_worn_slash_web_action_when_knife_and_blade_are_both_available() {
    let collision = walled_5x5_gap(2);
    let at = tile(1, 2, 0);
    let to = tile(2, 2, 0);
    let edge = |option, item_req, worn_req| TransportEdge {
        takeoff: None,
        worn_all_req: Vec::new(),
        forbidden_held: Vec::new(),
        forbidden_worn: Vec::new(),
        kind: TransportKind::Door,
        at,
        to,
        loc_id: 733,
        option,
        ticks: 2,
        dir: Some(crate::transport::DoorDir::E),
        open_loc_id: Some(734),
        skill_req: vec![],
        item_req,
        consumed_req: vec![],
        item_returns: vec![],
        quest_req: vec![],
        varp_req: vec![],
        worn_req,
        members_req: false,
        wildy_cap: None,
        quest_gates: None,
        player_delta: None,
    };
    // Pack order deliberately matches the extractor: oplocu first, Slash
    // second. Planning must follow slash_checker when a blade is worn rather
    // than letting a graph-order tie send the less-preferred knife action.
    let mut graph = TransportGraph::default();
    graph.edges.push(edge(0, vec![(946, 1)], vec![]));
    graph.edges.push(edge(1, vec![], vec![1277, 1321]));
    graph.at.insert(at, vec![0, 1]);
    let from = tile(0, 2, 0);
    let dest = tile(4, 2, 0);
    let find_web_option = |state: &WorldState| {
        find_with(
            &collision,
            &graph,
            from,
            dest,
            FindOptions::default(),
            state,
        )
        .unwrap()
        .legs
        .into_iter()
        .find_map(|leg| match leg {
            Leg::Transport { edge } if edge.loc_id == 733 => Some(edge.option),
            _ => None,
        })
        .expect("wall crossing uses the modeled web transport")
    };

    let mut knife_only = WorldState::empty();
    knife_only.inv.insert(946, 1);
    assert_eq!(
        find_web_option(&knife_only),
        0,
        "use the carried knife if no slash blade is worn"
    );

    let mut knife_and_blade = knife_only;
    knife_and_blade.worn.insert(1277);
    assert_eq!(
        find_web_option(&knife_and_blade),
        1,
        "worn slash weapon selects oploc1 even while a knife is carried"
    );
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
        takeoff: None,
        worn_all_req: Vec::new(),
        forbidden_held: Vec::new(),
        forbidden_worn: Vec::new(),
        kind: TransportKind::Ladder,
        player_delta: None,
        at: tile(0, 0, 0),
        to: tile(1, 1, 1),
        loc_id: 1747,
        option: 1,
        ticks: 3,
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
    assert_eq!(edge.as_ref(), &ladder);
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
fn small_single_search_allocates_for_visited_tiles_not_the_node_budget() {
    let collision = bake(40, 1, &[]);
    let graph = TransportGraph::default();
    let state = WorldState::empty();
    // The larger measured peak is 3,186 bytes; 4 KiB leaves 910 bytes (28.6%) of headroom.
    for (destination, reachable) in [(tile(5, 5, 0), false), (tile(2, 0, 0), true)] {
        let mut result = None;
        let allocations = allocation_counter::measure(|| {
            result = Some(find_with(
                &collision,
                &graph,
                tile(0, 0, 0),
                destination,
                FindOptions::default(),
                &state,
            ));
        });
        let result = result.expect("the measured search runs");
        if reachable {
            assert_eq!(
                result.expect("the corridor destination is reachable").dest,
                destination
            );
        } else {
            assert!(matches!(result, Err(RouteError::NoPath)));
        }
        assert!(
            allocations.bytes_max <= 4 * 1024,
            "small single search retained maximum-budget storage: {allocations:?}"
        );
    }
}

#[test]
fn small_first_goal_diagnosis_allocates_for_visited_tiles_not_the_proof_budget() {
    let collision = bake(40, 1, &[]);
    let graph = TransportGraph::default();
    let state = WorldState::empty();
    let mut result = None;
    // The measured peak is 1,199 bytes; 2 KiB leaves 849 bytes (70.8%) of headroom.
    let allocations = allocation_counter::measure(|| {
        result = Some(find_first_missing_item_reqs_with_avoid(
            &collision,
            &graph,
            tile(0, 0, 0),
            &[tile(5, 5, 0)],
            &[],
            FindOptions::default(),
            &state,
            &[],
        ));
    });
    assert!(result.expect("the measured diagnosis runs").is_none());
    assert!(
        allocations.bytes_max <= 2 * 1024,
        "small first-goal diagnosis retained maximum-proof storage: {allocations:?}"
    );
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
    g.edges[0].consumed_req = vec![(995, 10)]; // the 10-coin toll
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
            Leg::Transport { edge } if edge.consumed_req == vec![(995, 10)]
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

#[test]
fn cooperative_quest_gate_diagnosis_matches_blocked_and_satisfied_stage_fixture() {
    let gate = gate_window("tbwt", "tbwt_main", Some(3), Some(3));
    let world = Arc::new(crate::world::NavWorld::from_parts(
        walled_5x5(),
        stage_door_graph(vec![gate]),
        Vec::new(),
    ));
    let lease = crate::producer::lease(&world).expect("existing stage-door fixture lease");
    let (from, to) = (tile(0, 0, 0), tile(4, 4, 0));
    let options = FindOptions::default();
    for (label, state, blocked) in [
        ("blocked", WorldState::empty(), true),
        (
            "satisfied",
            evidenced(tbwt_evidence(gate_range(Some(3), Some(3)))),
            false,
        ),
    ] {
        let route = find_with(&world.collision, &world.graph, from, to, options, &state);
        let old_gates = find_unresolved_quest_gates(
            &world.collision,
            &world.graph,
            from,
            &[to],
            options,
            &state,
            &[],
        );
        if blocked {
            assert_eq!(route, Err(RouteError::NoPath), "{label}");
            assert!(
                matches!(&old_gates, Ok(Some(gates)) if !gates.is_empty()),
                "the existing blocked stage fixture must name a gate"
            );
        } else {
            assert!(route.is_ok(), "{label}");
            assert_eq!(old_gates, Ok(None), "{label}");
        }
        let completion = advance_real_route_completion(
            crate::router::SearchRequest::UnresolvedQuestGates {
                from,
                targets: Box::new([to]),
                options,
                avoid: Box::new([]),
                max_settles: crate::router::NODE_BUDGET,
            },
            &lease,
            &state,
        );
        assert!(completion.forward_settled() > 0, "{label}");
        let crate::router::SearchOutcome::UnresolvedQuestGates(new_gates) =
            completion.into_parts().0
        else {
            panic!("quest-gate cursor returned the wrong result shape")
        };
        assert_eq!(
            format!("{old_gates:#?}").as_bytes(),
            format!("{new_gates:#?}").as_bytes(),
            "{label}: cooperative stage diagnosis must match the old API"
        );
    }
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
            takeoff: None,
            worn_all_req: Vec::new(),
            forbidden_held: Vec::new(),
            forbidden_worn: Vec::new(),
            kind: TransportKind::Door,
            player_delta: None,
            at,
            to: tile(200, 201, 0),
            loc_id: 1,
            option: 1,
            ticks: 2,
            dir: None,
            open_loc_id: None,
            skill_req: vec![],
            item_req: vec![],
            consumed_req: vec![],
            item_returns: vec![],
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

/// The goal-set BankBudget diagnosis is the relaxed first-goal search's:
/// it names the chosen goal's missing facts, falls back to the fallback set
/// when no preferred goal routes relaxed, and is `None` when no goal routes
/// even with the carry/wear gates ignored.
#[test]
fn first_goal_diagnosis_names_the_chosen_goals_missing_facts() {
    let from = tile(20, 20, 0);
    let stands = [tile(202, 201, 0), tile(201, 202, 0)];
    let opts = FindOptions {
        allow_bank_fetch: true,
        ..FindOptions::default()
    };
    let state = WorldState::empty();
    let diagnose = |wc: &WorldCollision,
                    graph: &TransportGraph,
                    targets: &[WorldTile],
                    fallback: &[WorldTile]| {
        find_first_missing_item_reqs_with_avoid(
            wc,
            graph,
            from,
            targets,
            fallback,
            opts,
            &state,
            &[],
        )
    };
    let worn = Some(vec![MissingReq::WearAny { ids: vec![2] }]);

    let (wc, graph) = sealed_room(true);
    assert_eq!(diagnose(&wc, &graph, &stands, &[]), worn);
    // A preferred goal on the blocked ring never routes: the stands are the
    // fallback set and their diagnosis is the answer.
    let ring = [tile(201, 203, 0)];
    assert_eq!(diagnose(&wc, &graph, &ring, &stands), worn);
    // A goal reachable with nothing fetched wins over the gated room.
    assert_eq!(
        diagnose(&wc, &graph, &[tile(30, 30, 0), stands[0]], &[]),
        Some(Vec::new())
    );

    let (wc, graph) = sealed_room(false);
    assert_eq!(diagnose(&wc, &graph, &stands, &[]), None);
    assert_eq!(diagnose(&wc, &graph, &ring, &stands), None);
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
    let search = |stands: &[WorldTile], reachable_budget| {
        super::first_search(
            &wc,
            &graph,
            from,
            stands,
            &[],
            opts,
            &state,
            super::Relax::Strict,
            &[],
            64,
            reachable_budget,
        )
    };

    // A goal at the landing proves reachability during initialization. The
    // farther goal needs backward steps before releasing its proof scratch.
    for stands in [stands, [tile(200, 200, 0)]] {
        let lifted = search(&stands, super::NODE_BUDGET);
        assert_eq!(lifted.proof(), ReverseProof::Reachable);
        let route = lifted
            .route()
            .expect("the teleport reaches the sealed stand");
        assert_eq!(route.dest, stands[0]);
        assert!(lifted.settled() > 64);
        assert_eq!(
            search(&stands, 64).route().err(),
            Some(RouteError::BudgetExhausted)
        );
    }
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
        takeoff: None,
        worn_all_req: Vec::new(),
        forbidden_held: Vec::new(),
        forbidden_worn: Vec::new(),
        kind,
        player_delta: None,
        at,
        to,
        loc_id: 1,
        option: 1,
        ticks,
        dir: None,
        open_loc_id: None,
        skill_req: vec![],
        item_req,
        consumed_req: vec![],
        item_returns: vec![],
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
    let Some(mut world) = crate::world::NavWorld::load_default_pack_or_skip() else {
        return;
    };
    // Retain the pre-zone banking cost witness independently of danger policy.
    world.graph.zones = None;
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
    let Some(mut world) = crate::world::NavWorld::load_default_pack_or_skip() else {
        return;
    };
    // Grants now try safe first; isolate the original collision-search cap.
    world.graph.zones = None;
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
            if opts.zones.is_all() {
                assert!(
                    search.settled() > 32_768,
                    "the unrestricted collision route must exercise the old scene cap"
                );
            }
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
    let Some(mut world) = crate::world::NavWorld::load_default_pack_or_skip() else {
        return;
    };
    // Measure one collision-search budget, not a separate danger fallback pass.
    world.graph.zones = None;
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
                let bound_edge_index = match edge.kind {
                    TransportKind::Teleport => {
                        assert!(opts.allow_teleports && graph.teleports.contains(edge));
                        None
                    }
                    TransportKind::EssenceExit => {
                        let session = opts
                            .essence
                            .as_ref()
                            .expect("return must use captured entry wizard");
                        assert!(crate::essence::ESSENCE_MINE_PORTALS.contains(&edge.at));
                        assert_eq!(
                            **edge,
                            crate::essence::essence_return_edge(edge.at, session)
                        );
                        None
                    }
                    _ => {
                        // A player-relative landing is materialized from the actual
                        // takeoff in a route. Restore its packed anchor landing before
                        // comparing it with the bound graph.
                        let index = if edge.player_delta.is_some() {
                            let mut anchored = edge.clone();
                            anchored.to = anchored
                                .landing_from(anchored.at)
                                .expect("relative transport landing from its anchor");
                            graph
                                .edges
                                .iter()
                                .position(|candidate| candidate == anchored.as_ref())
                        } else {
                            graph
                                .edges
                                .iter()
                                .position(|candidate| candidate == edge.as_ref())
                        };
                        Some(index.expect("transport must belong to the bound graph"))
                    }
                };
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
                        let index =
                            bound_edge_index.expect("ordinary transport belongs to the graph");
                        if edge.player_delta.is_some() {
                            assert_eq!(
                                edge.landing_from(previous),
                                Some(edge.to),
                                "relative transport landing must match its takeoff"
                            );
                        }
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
    g.edges[0].consumed_req.clear();
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

#[test]
fn worn_all_req_requires_every_equipped_piece_on_a_route() {
    let wc = walled_5x5();
    let mut graph = toll_graph();
    graph.edges[0].consumed_req.clear();
    graph.edges[0].worn_all_req = vec![1277, 1321];
    let from = tile(0, 0, 0);
    let to = tile(4, 4, 0);
    let one_worn = WorldState {
        inv: HashMap::from([(1277, 1), (1321, 1)]),
        worn: HashSet::from([1277]),
        ..WorldState::empty()
    };
    assert!(matches!(
        find_with(&wc, &graph, from, to, FindOptions::default(), &one_worn),
        Err(RouteError::NoPath)
    ));
    assert!(
        find_missing_item_reqs(&wc, &graph, from, to, FindOptions::default(), &one_worn).is_none(),
        "worn_all_req is not relaxed into a BankBudget shopping list"
    );

    let all_worn = WorldState {
        worn: HashSet::from([1277, 1321]),
        ..WorldState::empty()
    };
    assert!(find_with(&wc, &graph, from, to, FindOptions::default(), &all_worn).is_ok());
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
    assert_eq!(edge.consumed_req, vec![(554, 1), (556, 3), (563, 1)]);
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
    assert_eq!(edge.consumed_req, vec![(1712, 1)]); // the charged item
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
fn danger_grants_choose_the_safe_route_before_cheaper_zone_transit() {
    let collision = bake(8, 1, &[]);
    let from = tile(0, 0, 0);
    let goal = tile(7, 0, 0);
    let goals = [goal];
    let mut graph = door(from, goal, 20);
    install_zones(&collision, &mut graph, vec![rect_zone(2, 4, 0, 0)]);
    let state = WorldState::empty();
    for zones in [
        ZoneExempt::all(),
        ZoneExempt::named(&[ZoneKey::Zone(0)]).unwrap(),
    ] {
        let opts = FindOptions {
            zones,
            ..FindOptions::default()
        };
        let route = find_with(&collision, &graph, from, goal, opts, &state).unwrap();
        assert_eq!(route.ticks, 20.0, "a grant permits danger only as fallback");
        assert_eq!(
            find_first_with(&collision, &graph, from, &goals, opts, &state)
                .into_route()
                .unwrap(),
            route
        );
        assert_eq!(
            find_many_with(&collision, &graph, from, &goals, opts, &state)
                .route(0)
                .unwrap(),
            route
        );
    }
    graph.at.clear();
    graph.edges.clear();
    assert_eq!(
        find_with(
            &collision,
            &graph,
            from,
            goal,
            FindOptions::default(),
            &state
        ),
        Err(RouteError::NoPath)
    );
    let opts = FindOptions {
        zones: ZoneExempt::all(),
        ..FindOptions::default()
    };
    let fallback = find_with(&collision, &graph, from, goal, opts, &state).unwrap();
    assert_eq!(fallback.ticks, 3.5);
    assert!(route_walk_tiles(&fallback).any(|tile| tile.x == 3));
    assert_eq!(
        find_first_with(&collision, &graph, from, &goals, opts, &state)
            .into_route()
            .unwrap(),
        fallback
    );
    assert_eq!(
        find_many_with(&collision, &graph, from, &goals, opts, &state)
            .route(0)
            .unwrap(),
        fallback
    );
}

#[test]
fn endpoint_completion_avoids_unrelated_granted_transit() {
    let collision = bake(8, 1, &[]);
    let from = tile(0, 0, 0);
    let goal = tile(7, 0, 0);
    let goals = [goal];
    let mut graph = door(from, tile(5, 0, 0), 20);
    install_zones(
        &collision,
        &mut graph,
        vec![rect_zone(2, 4, 0, 0), rect_zone(6, 7, 0, 0)],
    );
    let state = WorldState::empty();
    for zones in [
        ZoneExempt::NONE,
        ZoneExempt::all(),
        ZoneExempt::named(&[ZoneKey::Zone(0)]).unwrap(),
    ] {
        let opts = FindOptions {
            zones,
            ..FindOptions::default()
        };
        let route = find_with(&collision, &graph, from, goal, opts, &state).unwrap();
        assert_eq!(route.ticks, 21.0);
        assert!(!route_walk_tiles(&route).any(|tile| tile.x == 3));
        assert_eq!(
            find_first_with(&collision, &graph, from, &goals, opts, &state)
                .into_route()
                .unwrap(),
            route
        );
        assert_eq!(
            find_many_with(&collision, &graph, from, &goals, opts, &state)
                .route(0)
                .unwrap(),
            route
        );
    }
    graph.at.clear();
    graph.edges.clear();
    assert_eq!(
        find_with(
            &collision,
            &graph,
            from,
            goal,
            FindOptions::default(),
            &state,
        ),
        Err(RouteError::NoPath)
    );
    let opts = FindOptions {
        zones: ZoneExempt::all(),
        ..FindOptions::default()
    };
    let route = find_with(&collision, &graph, from, goal, opts, &state).unwrap();
    assert_eq!(route.ticks, 3.5);
    assert!(route_walk_tiles(&route).any(|tile| tile.x == 3));
    assert_eq!(
        find_first_with(&collision, &graph, from, &goals, opts, &state)
            .into_route()
            .unwrap(),
        route
    );
    assert_eq!(
        find_many_with(&collision, &graph, from, &goals, opts, &state)
            .route(0)
            .unwrap(),
        route
    );
}

#[test]
fn endpoint_completion_fallback_outranks_granted_preferred_transit() {
    let blocked: [_; 8] = std::array::from_fn(|x| (x as i32, 1, CollisionFlag::SQ_BLOCKED as u32));
    let collision = bake(8, 3, &blocked);
    let from = tile(0, 0, 0);
    let preferred = tile(7, 0, 0);
    let fallback = tile(7, 2, 0);
    let mut graph = door(from, tile(5, 2, 0), 20);
    install_zones(
        &collision,
        &mut graph,
        vec![
            rect_zone(2, 4, 0, 0),
            rect_zone(6, 7, 0, 0),
            rect_zone(6, 7, 2, 2),
        ],
    );
    for zones in [
        ZoneExempt::NONE,
        ZoneExempt::all(),
        ZoneExempt::named(&[ZoneKey::Zone(0)]).unwrap(),
    ] {
        let search = find_first_with_fallback(
            &collision,
            &graph,
            from,
            &[preferred],
            &[fallback],
            FindOptions {
                zones,
                ..FindOptions::default()
            },
            &WorldState::empty(),
        );
        assert_eq!(search.route(), Err(RouteError::NoPath));
        assert!(
            matches!(search.fallback(), Some(FallbackRoute::Routed(route))
                if route.dest == fallback && route.ticks == 21.0)
        );
    }
}

#[test]
fn safe_fallback_targets_outrank_exempt_preferred_targets() {
    let collision = bake(8, 1, &[]);
    let from = tile(0, 0, 0);
    let safe = tile(1, 0, 0);
    let deep = tile(4, 0, 0);
    let beyond = tile(7, 0, 0);
    let mut graph = TransportGraph::default();
    install_zones(&collision, &mut graph, vec![rect_zone(2, 5, 0, 0)]);
    let named = ZoneExempt::named(&[graph.zones.as_ref().unwrap().key(0)]).unwrap();
    let state = WorldState::empty();
    for zones in [ZoneExempt::NONE, ZoneExempt::all(), named] {
        let opts = FindOptions {
            zones,
            ..FindOptions::default()
        };
        let search = |preferred: &[WorldTile], fallback: &[WorldTile]| {
            find_first_with_fallback(&collision, &graph, from, preferred, fallback, opts, &state)
        };
        let safer = search(&[deep], &[safe]);
        assert_eq!(safer.route(), Err(RouteError::NoPath));
        assert!(
            matches!(safer.fallback(), Some(FallbackRoute::Routed(route))
                if route.dest == safe && route.ticks == 0.5),
            "a safe acceptable fallback must beat an exempt preferred target"
        );

        let preferred_safe = search(&[safe], &[from]);
        assert_eq!(preferred_safe.route().unwrap().dest, safe);
        assert!(preferred_safe.fallback().is_none());

        let fallback_only = search(&[], &[safe]);
        assert!(
            matches!(fallback_only.fallback(), Some(FallbackRoute::Routed(route))
                if route.dest == safe && route.ticks == 0.5)
        );

        let exempt = search(&[deep], &[beyond]);
        assert_eq!(exempt.route().unwrap().dest, deep);
        assert_eq!(exempt.route().unwrap().ticks, 2.0);
        assert!(exempt.fallback().is_none());
    }
}

#[test]
fn endpoint_completion_and_grants_retry_after_safe_budget_exhaustion() {
    let collision = bake(40, 32, &[]);
    let from = tile(31, 0, 0);
    let goal = tile(35, 0, 0);
    // This real safe edge keeps the reverse proof open/reachable; the safe
    // Dijkstra pass spends its cap on the large western region first.
    let mut graph = door(from, goal, 100);
    install_zones(&collision, &mut graph, vec![rect_zone(33, 39, 0, 31)]);
    let state = WorldState::empty();
    for zones in [ZoneExempt::NONE, ZoneExempt::all()] {
        let opts = FindOptions {
            zones,
            ..FindOptions::default()
        };
        let route =
            super::find_with_avoid_bounded(&collision, &graph, from, goal, opts, &state, &[], 64)
                .unwrap();
        assert_eq!(route.ticks, 2.0);
        let first = super::first_search(
            &collision,
            &graph,
            from,
            &[goal],
            &[],
            opts,
            &state,
            super::Relax::Strict,
            &[],
            64,
            64,
        );
        assert!(first.settled() > 64, "both bounded passes ran");
        assert_eq!(first.into_route().unwrap(), route);
        let goals = [goal];
        let many = super::find_many_with_avoid_bounded(
            &collision,
            &graph,
            from,
            &goals,
            opts,
            &state,
            &[],
            64,
        );
        assert!(many.settled() > 64);
        assert_eq!(many.route(0).unwrap(), route);
    }
}

#[test]
fn deep_zone_goal_proof_avoids_exhausting_the_safe_world() {
    let collision = bake(128, 128, &[]);
    let from = tile(31, 64, 0);
    let goal = tile(35, 64, 0);
    let mut graph = TransportGraph::default();
    install_zones(&collision, &mut graph, vec![rect_zone(33, 127, 0, 127)]);
    let state = WorldState::empty();
    let goals = [goal];
    let opts = FindOptions::default();
    let search = find_first_with(&collision, &graph, from, &goals, opts, &state);
    assert!(search.settled() < 100, "{}", search.settled());
    assert_eq!(search.into_route().unwrap().ticks, 2.0);
    let many = find_many_with(&collision, &graph, from, &goals, opts, &state);
    assert!(many.settled() < 100, "{}", many.settled());
    assert_eq!(many.route(0).unwrap().ticks, 2.0);
}

#[test]
fn endpoint_goal_exemption_does_not_turn_its_zone_into_a_transit_corridor() {
    // The walk-only corridor to past crosses Z. Pocket is in Z but is
    // reachable directly, so its endpoint exemption must not help the
    // separate outside goal.
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
    let direct_to_pocket = door(from, pocket, 9).edges.pop().unwrap();
    graph.at.entry(from).or_default().push(graph.edges.len());
    graph.edges.push(direct_to_pocket);
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
    assert_eq!(first.completion_partitions(), 0);
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
    assert_eq!(many.completion_partitions(), 0);
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
    assert!(bypass.scratch_capacities().zone_mask_words > 0);
    assert_eq!(bypass.route(0).unwrap().ticks, 30.0);

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
        Err(RouteError::NoPath)
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
    let completion = find_first_with(
        &collision,
        &graph,
        from,
        &goals,
        FindOptions::default(),
        &state,
    )
    .into_route()
    .unwrap();
    assert_eq!(
        completion.dest, pocket,
        "the goal's zone allows completion, but cannot reach the outside goal"
    );
}

#[test]
fn endpoint_completion_prefers_safe_routes_and_never_leaves_the_goal_zone() {
    let collision = bake(8, 1, &[]);
    let from = tile(0, 0, 0);
    let goal = tile(4, 0, 0);
    let mut graph = door(from, goal, 10);
    install_zones(&collision, &mut graph, vec![rect_zone(2, 4, 0, 0)]);
    let state = WorldState::empty();
    let opts = FindOptions::default();
    let goals = [goal];
    let safe = find_with(&collision, &graph, from, goal, opts, &state).unwrap();
    assert_eq!(
        safe.ticks, 10.0,
        "safe landing wins over a shorter zone crossing"
    );
    assert_eq!(
        find_first_with(&collision, &graph, from, &goals, opts, &state)
            .into_route()
            .unwrap(),
        safe
    );
    assert_eq!(
        find_many_with(&collision, &graph, from, &goals, opts, &state)
            .route(0)
            .unwrap(),
        safe
    );
    graph.at.clear();
    graph.edges.clear();
    let completion = find_with(&collision, &graph, from, goal, opts, &state).unwrap();
    assert_eq!(completion.ticks, 2.0, "enter and move inside the goal zone");
    assert_eq!(
        find_first_with(&collision, &graph, from, &goals, opts, &state)
            .into_route()
            .unwrap(),
        completion
    );
    assert_eq!(
        find_many_with(&collision, &graph, from, &goals, opts, &state)
            .route(0)
            .unwrap(),
        completion
    );
    assert!(matches!(
        find_with(&collision, &graph, from, tile(6, 0, 0), opts, &state),
        Err(RouteError::NoPath)
    ));
}

#[test]
fn endpoint_exemptions_allow_departure_and_arrival_without_zone_transit() {
    let collision = bake(8, 1, &[]);
    let mut graph = TransportGraph::default();
    install_zones(&collision, &mut graph, vec![rect_zone(2, 4, 0, 0)]);
    let state = WorldState::empty();

    let leaving = find_with(
        &collision,
        &graph,
        tile(2, 0, 0),
        tile(6, 0, 0),
        FindOptions::default(),
        &state,
    )
    .unwrap();
    assert_eq!(leaving.ticks, 2.0, "the origin may escape through its zone");

    let arriving = find_with(
        &collision,
        &graph,
        tile(1, 0, 0),
        tile(2, 0, 0),
        FindOptions::default(),
        &state,
    )
    .unwrap();
    assert_eq!(arriving.ticks, 0.5, "the selected goal may be in its zone");
    assert!(matches!(
        find_with(
            &collision,
            &graph,
            tile(1, 0, 0),
            tile(6, 0, 0),
            FindOptions::default(),
            &state,
        ),
        Err(RouteError::NoPath)
    ));

    let named = ZoneExempt::named(&[ZoneKey::Zone(0)]).unwrap();
    let whole_walk = find_with(
        &collision,
        &graph,
        tile(1, 0, 0),
        tile(6, 0, 0),
        FindOptions {
            zones: named,
            ..FindOptions::default()
        },
        &state,
    )
    .unwrap();
    assert_eq!(whole_walk.ticks, 2.5, "named exemptions remain whole-walk");
}

#[test]
fn origin_endpoint_permission_cannot_reenter_its_active_zone() {
    let collision = bake(8, 1, &[]);
    let mut graph = TransportGraph {
        wilderness: WildernessRules {
            zones: vec![
                WildernessZone {
                    x1: 1,
                    x2: 2,
                    z1: 0,
                    z2: 0,
                    level1: 0,
                    level2: 0,
                    origin_z: 0,
                },
                WildernessZone {
                    x1: 5,
                    x2: 5,
                    z1: 0,
                    z2: 0,
                    level1: 0,
                    level2: 0,
                    origin_z: 0,
                },
            ],
            divisor: 0,
            offset: 0,
        },
        ..TransportGraph::default()
    };
    let mut zone = rect_zone(1, 5, 0, 0);
    zone.class = ZoneClass::LevelRule;
    zone.cap = 12;
    install_zones(&collision, &mut graph, vec![zone]);
    let state = WorldState {
        combat_level: Some(126),
        ..WorldState::empty()
    };
    let from = tile(1, 0, 0);
    let to = tile(6, 0, 0);
    let opts = FindOptions {
        allow_wilderness: true,
        ..FindOptions::default()
    };
    assert!(matches!(
        find_with(&collision, &graph, from, to, opts, &state),
        Err(RouteError::NoPath)
    ));
    assert!(
        find_with(
            &collision,
            &graph,
            from,
            to,
            FindOptions {
                zones: ZoneExempt::named(&[ZoneKey::Zone(0)]).unwrap(),
                ..opts
            },
            &state,
        )
        .is_ok(),
        "explicit named permission can still cross the full zone"
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
fn single_target_witness_observes_origin_escape_and_goal_entry() {
    let collision = bake(9, 1, &[]);
    let from = tile(1, 0, 0);
    let to = tile(7, 0, 0);
    let mut graph = TransportGraph::default();
    install_zones(
        &collision,
        &mut graph,
        vec![
            rect_zone(1, 2, 0, 0),
            rect_zone(6, 7, 0, 0),
            rect_zone(4, 4, 0, 0),
        ],
    );
    let state = WorldState::empty();
    let opts = FindOptions::default();
    assert!(matches!(
        find_with(&collision, &graph, from, to, opts, &state),
        Err(RouteError::NoPath)
    ));
    assert_eq!(
        crate::router::find_blocking_zones(&collision, &graph, from, to, opts, &state, &[]),
        Some(vec![ZoneKey::Zone(2)]),
        "origin escape and goal entry are permitted; only the intervening zone blocks"
    );
}

#[test]
fn refusal_attribution_reports_the_selected_witness_and_multi_goal_frontiers() {
    let blocked: Vec<_> = (0..3)
        .flat_map(|z| {
            (0..7).filter_map(move |x| {
                let open = z != 1 || x == 0 || x == 6;
                (!open).then_some((x, z, CollisionFlag::WALK_SCENERY as u32))
            })
        })
        .collect();
    let collision = bake(7, 3, &blocked);
    let from = tile(0, 1, 0);
    let to = tile(6, 1, 0);
    let mut graph = TransportGraph::default();
    install_zones(
        &collision,
        &mut graph,
        vec![
            rect_zone(3, 3, 0, 0),
            rect_zone(3, 3, 2, 2),
            rect_zone(3, 3, 1, 1),
        ],
    );
    let opts = FindOptions::default();
    let state = WorldState::empty();
    assert!(matches!(
        find_with(&collision, &graph, from, to, opts, &state),
        Err(RouteError::NoPath)
    ));

    let witness = find_with(
        &collision,
        &graph,
        from,
        to,
        FindOptions {
            zones: ZoneExempt::all(),
            ..opts
        },
        &state,
    )
    .unwrap();
    let table = graph.zones.as_ref().unwrap();
    let witness_zones: std::collections::HashSet<_> = route_walk_tiles(&witness)
        .flat_map(|tile| table.at(tile))
        .filter(|&index| index < 2)
        .collect();
    assert_eq!(
        witness_zones.len(),
        1,
        "one shortest witness traverses only one of the two blocked corridors"
    );
    let witness_zone = *witness_zones.iter().next().unwrap();
    let single_target =
        crate::router::find_blocking_zones(&collision, &graph, from, to, opts, &state, &[]);
    assert_eq!(
        single_target,
        Some(vec![ZoneKey::Zone(witness_zone)]),
        "single-target attribution follows the selected relaxed route"
    );

    let goals = [tile(6, 0, 0), tile(6, 2, 0), tile(3, 1, 0)];
    assert_eq!(
        find_first_with(&collision, &graph, from, &goals, opts, &state).into_route(),
        Err(RouteError::NoPath)
    );
    let first_frontier = crate::router::find_first_blocking_zones(
        &collision,
        &graph,
        from,
        &goals,
        opts,
        &state,
        &[],
    )
    .unwrap();
    assert_eq!(
        first_frontier,
        vec![ZoneKey::Zone(0), ZoneKey::Zone(1)],
        "multigoal attribution includes each feasible frontier and excludes the wall-sealed zone"
    );
    let one_goal = [to];
    let first_goal = crate::router::find_first_blocking_zones(
        &collision,
        &graph,
        from,
        &one_goal,
        opts,
        &state,
        &[],
    )
    .expect("one-goal first-goal witness");
    assert_eq!(
        first_goal,
        vec![ZoneKey::Zone(0), ZoneKey::Zone(1)],
        "one-goal first-goal attribution includes both feasible corridors"
    );
    assert_ne!(
        first_goal.as_slice(),
        single_target
            .as_deref()
            .expect("single-target witness exists"),
        "one-goal first-goal attribution differs from the selected-route witness"
    );

    let world = Arc::new(crate::world::NavWorld::from_parts(
        collision,
        graph,
        Vec::new(),
    ));
    let lease = crate::producer::lease(&world).expect("synthetic diagnosis lease");
    let zone_count = world
        .graph
        .zones
        .as_ref()
        .expect("synthetic zone table")
        .zones()
        .len();
    let diagnose = |diagnosis| {
        let endpoints = [from, to];
        let attribution_endpoints = match diagnosis {
            crate::router::BlockerDiagnosis::SingleTarget => endpoints.as_slice(),
            crate::router::BlockerDiagnosis::FirstGoal => &endpoints[..1],
        };
        let zone_filter = ZoneFilterOwned::new(
            zone_count,
            state.combat_level,
            attribution_endpoints,
            opts.zones,
        )
        .expect("synthetic blocking filter");
        advance_real_route_cursor(
            crate::router::SearchRequest::BlockingZones {
                from,
                targets: vec![to].into_boxed_slice(),
                diagnosis,
                options: opts,
                avoid: Vec::new().into_boxed_slice(),
                max_settles: crate::router::NODE_BUDGET,
                zone_filter,
            },
            &lease,
            &state,
        )
    };
    let crate::router::SearchOutcome::BlockingZones(cooperative_single) =
        diagnose(crate::router::BlockerDiagnosis::SingleTarget)
    else {
        panic!("single-target blocker diagnosis returned the wrong result shape")
    };
    assert_eq!(
        cooperative_single.as_deref(),
        single_target.as_deref(),
        "cooperative single-target result matches the old single-target API"
    );
    let crate::router::SearchOutcome::BlockingZones(cooperative_first_goal) =
        diagnose(crate::router::BlockerDiagnosis::FirstGoal)
    else {
        panic!("first-goal blocker diagnosis returned the wrong result shape")
    };
    assert_eq!(
        cooperative_first_goal.as_deref(),
        Some(first_goal.as_slice()),
        "cooperative one-goal FirstGoal result matches the old API"
    );
    assert_ne!(
        cooperative_single.as_deref(),
        cooperative_first_goal.as_deref(),
        "cooperative blocker modes must preserve their distinct witnesses"
    );
}

#[test]
fn refusal_attribution_does_not_override_unrelated_item_gates() {
    let collision = bake(5, 1, &[(2, 0, CollisionFlag::WALK_SCENERY as u32)]);
    let from = tile(0, 0, 0);
    let goal = tile(4, 0, 0);
    let mut graph = door(from, goal, 1);
    graph.edges[0].item_req.push((995, 1));
    install_zones(&collision, &mut graph, vec![rect_zone(1, 1, 0, 0)]);
    assert!(matches!(
        find_with(
            &collision,
            &graph,
            from,
            goal,
            FindOptions::default(),
            &WorldState::empty(),
        ),
        Err(RouteError::NoPath)
    ));
    assert_eq!(
        crate::router::find_blocking_zones(
            &collision,
            &graph,
            from,
            goal,
            FindOptions::default(),
            &WorldState::empty(),
            &[],
        ),
        None,
        "zones are not reported when the same route is impossible with zones lifted"
    );
}

#[test]
fn straddling_zone_endpoint_does_not_clear_its_active_interior() {
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
    for (min_x, partitions, reachable) in [(2, 0, false), (6, 0, true)] {
        let mut zone = rect_zone(min_x, 7, 0, 0);
        zone.class = ZoneClass::LevelRule;
        zone.cap = 12;
        install_zones(&collision, &mut graph, vec![zone]);
        let single = find_with(&collision, &graph, tile(0, 0, 0), goal, opts, &state);
        let first = find_first_with(&collision, &graph, tile(0, 0, 0), &[goal], opts, &state);
        assert_eq!(first.completion_partitions(), partitions);
        let first = first.into_route();
        let goals = [goal];
        let many = find_many_with(&collision, &graph, tile(0, 0, 0), &goals, opts, &state);
        assert_eq!(many.completion_partitions(), partitions);
        if reachable {
            let single = single.unwrap();
            assert_eq!((single.ticks, route_walk_tiles(&single).count()), (3.5, 8));
            assert_eq!(first.unwrap(), single);
            assert_eq!(many.route(0).unwrap(), single);
        } else {
            assert!(matches!(single, Err(RouteError::NoPath)));
            assert_eq!(first, Err(RouteError::NoPath));
            assert_eq!(many.route(0), Err(TargetError::NoPath));
        }
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
        assert_eq!(
            crate::router::find_blocking_zones(&collision, &graph, from, goal, opts, &state, &[],),
            Some(vec![ZoneKey::Zone(0)]),
            "a blocked transport/teleport landing is on the selected relaxed route"
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
    assert_eq!(
        crate::router::find_blocking_zones(&collision, &graph, from, goal, opts, &state, &[]),
        Some(vec![ZoneKey::Zone(0)]),
        "a blocked essence return landing is on the selected relaxed route"
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
    assert!(filter
        .blocking_at(&graph.wilderness, tile(5, 0, 0))
        .next()
        .is_some());
    assert!(filter
        .blocking_at(&graph.wilderness, tile(6, 0, 0))
        .next()
        .is_none());
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
    let mut previous = from;
    for leg in &route.legs {
        let arrival = match leg {
            Leg::Walk { tiles } => {
                for &tile in tiles {
                    assert!(
                        filter
                            .blocking_transition_at(
                                &world.graph.wilderness,
                                previous,
                                tile,
                                &|| tile == route.dest,
                            )
                            .next()
                            .is_none(),
                        "zone entered: {tile:?} from {previous:?}"
                    );
                    previous = tile;
                }
                continue;
            }
            Leg::Transport { edge } => edge.to,
        };
        assert!(
            filter
                .blocking_transition_at(&world.graph.wilderness, previous, arrival, &|| arrival
                    == route.dest,)
                .next()
                .is_none(),
            "zone landing: {arrival:?} from {previous:?}"
        );
        previous = arrival;
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
    let mut f2_cap_ticks = None;
    let mut f2_lifted_ticks = None;
    let mut f3_cap_ticks = None;
    let mut f3_lifted_ticks = None;
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
        match row {
            "F2-cap" => f2_cap_ticks = Some(route.ticks),
            "F2-lifted" => f2_lifted_ticks = Some(route.ticks),
            "F3-cap" => f3_cap_ticks = Some(route.ticks),
            "F3-lifted" => f3_lifted_ticks = Some(route.ticks),
            _ => {}
        }
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
    assert!(
        f2_lifted_ticks.expect("F2 combat 13 route") < f2_cap_ticks.expect("F2 combat 12 route"),
        "combat 13 should lift F2 and shorten the route compared with combat 12"
    );
    assert!(
        f3_lifted_ticks.expect("F3 combat 147 route") < f3_cap_ticks.expect("F3 combat 146 route"),
        "combat 147 should lift F3 and shorten the route compared with combat 146"
    );
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
    let safe = find_with(
        &world.collision,
        &world.graph,
        lumbridge,
        bank,
        opts,
        &zones_rich_289_state(Some(126)),
    )
    .unwrap();
    assert_eq!(
        route, safe,
        "named consent must still prefer the safe route"
    );
    assert_zone_route_clear(
        &world,
        lumbridge,
        &route,
        opts,
        &zones_rich_289_state(Some(126)),
    );
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
        let safe = find_with(
            &world.collision,
            &world.graph,
            taverley,
            catherby,
            opts,
            &zones_rich_289_state(Some(combat)),
        )
        .unwrap();
        assert_eq!(
            route, safe,
            "named consent must still prefer the safe route"
        );
    }
}

#[test]
fn real_289_first_refusal_witness_exempts_its_chosen_goal_zone() {
    let Some(world) = crate::world::NavWorld::load_default_pack_or_skip() else {
        return;
    };
    let from = tile(3123, 3245, 0);
    let goal = tile(3211, 3195, 0);
    let state = WorldState {
        combat_level: Some(3),
        stats: (0..21).map(|skill| (skill, 1)).collect(),
        ..WorldState::empty()
    };
    let opts = FindOptions::default();
    let table = world.graph.zones.as_ref().expect("v12 zones");
    let own_zone = table.resolve("giantrat1@3211,3195,0").unwrap();
    assert!(table.at(goal).any(|index| table.key(index) == own_zone));
    let single = crate::router::find_blocking_zones(
        &world.collision,
        &world.graph,
        from,
        goal,
        opts,
        &state,
        &[],
    );
    assert_eq!(single, None, "the goal's own zone is already exempt");
    let first = crate::router::find_first_blocking_zones(
        &world.collision,
        &world.graph,
        from,
        &[goal],
        opts,
        &state,
        &[],
    );
    assert_eq!(
        first, single,
        "radius/bank witnesses must exempt the chosen end"
    );
}

#[test]
fn real_289_refusals_report_best_relaxed_route_zone_witness() {
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
    let mountain = table.resolve("white-wolf-mountain").unwrap();
    let wolf = table.resolve("wolf@2647,3584,0").unwrap();
    assert_eq!(keys, vec![wolf, mountain]);
    let crossing = FindOptions {
        zones: ZoneExempt::named(&[mountain, wolf]).unwrap(),
        ..opts
    };
    let route = find_with(&world.collision, &world.graph, from, to, crossing, &state).unwrap();
    assert_eq!(
        (route.ticks, zone_route_cell_count(&route), route.legs.len()),
        (256.5, 502, 7)
    );
    let mut combat_129_ticks = None;
    let mut combat_147_ticks = None;
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
        match combat {
            129 => combat_129_ticks = Some(route.ticks),
            147 => combat_147_ticks = Some(route.ticks),
            _ => unreachable!("only the reviewed combat levels are checked"),
        }
    }
    assert!(
        combat_147_ticks.expect("combat 147 route") < combat_129_ticks.expect("combat 129 route"),
        "combat 147 should lift zones and shorten the refusal route compared with combat 129"
    );
    for members in [false, true] {
        let fresh = WorldState {
            combat_level: Some(3),
            map_members: members,
            stats: (0..21).map(|skill| (skill, 1)).collect(),
            ..WorldState::empty()
        };
        // Draynor to Lumbridge's (3224,3200) now routes: the brown bear's
        // rectangle no longer blocks it, and the giant rat tiles it crosses
        // are ones the engine's rat flood cannot reach. The rat pocket at
        // (3212,3179) south of it is still refused by two rats.
        let from = tile(3123, 3245, 0);
        let to = tile(3212, 3179, 0);
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
        let north_rat = table.resolve("giantrat1@3211,3195,0").unwrap();
        let south_rat = table.resolve("giantrat1@3211,3187,0").unwrap();
        assert_eq!(keys, vec![south_rat, north_rat], "{keys:?}");
        let route = find_with(
            &world.collision,
            &world.graph,
            from,
            to,
            FindOptions {
                zones: ZoneExempt::named(&[north_rat, south_rat]).unwrap(),
                ..opts
            },
            &fresh,
        )
        .unwrap();
        assert_eq!(
            (route.ticks, zone_route_cell_count(&route), route.legs.len()),
            (62.0, 125, 3)
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
    for (from, combat) in [
        (tile(3222, 3218, 0), 126),
        (tile(3222, 3218, 0), 3),
        (tile(3016, 9840, 0), 126),
        (tile(3016, 9840, 0), 3),
    ] {
        let state = zones_rich_289_state(Some(combat));
        let opts = FindOptions::default();
        let many = find_many_with(&world.collision, &world.graph, from, &targets, opts, &state);
        let granted = find_many_with(
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
                .zip(granted.results())
                .filter(|(a, b)| {
                    a.as_ref().map(|cost| cost.ticks) != b.as_ref().map(|cost| cost.ticks)
                })
                .count(),
            0,
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
    for (combat, partitions, ticks, first_target) in [
        (3, 2, [87.5, 70.5, 109.5], 0),
        (126, 1, [84.5, 67.5, 107.5], 0),
    ] {
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
        assert_eq!(first.completion_partitions(), 0);
        let first = first.into_route().unwrap();
        assert_eq!(
            (first.dest, first.ticks),
            (targets[first_target], ticks[first_target])
        );
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
    assert!(
        matches!(empty.fallback(), Some(FallbackRoute::Routed(route))
            if route.dest == zoned && route.ticks == 1.0)
    );
    assert_eq!(empty.completion_partitions(), 0);

    let world = Arc::new(crate::world::NavWorld::from_parts(
        collision,
        graph,
        Vec::new(),
    ));
    let lease = crate::producer::lease(&world).expect("zoned fixture lease");
    let preferred = [zoned];
    let fallback = [from];
    let beyond = [tile(4, 0, 0)];
    for (targets, fallback, preferred_wins) in [
        (&preferred[..], &fallback[..], true),
        (&beyond[..], &preferred[..], false),
        (&[][..], &preferred[..], false),
    ] {
        let completion = advance_real_route_completion(
            super::SearchRequest::First {
                from,
                targets: targets.to_vec().into_boxed_slice(),
                fallback: fallback.to_vec().into_boxed_slice(),
                options: FindOptions::default(),
                avoid: Box::new([]),
                max_settles: super::NODE_BUDGET,
            },
            &lease,
            &state,
        );
        let super::SearchOutcome::First { search } = completion.into_parts().0 else {
            panic!("first-goal cursor returned the wrong result shape");
        };
        let (route, fallback) = search.into_routes();
        if preferred_wins {
            assert_eq!(route.expect("preferred completion").destination(), zoned);
            assert!(fallback.is_none());
        } else {
            assert!(matches!(route, Err(RouteError::NoPath)));
            assert!(
                matches!(fallback, Some(super::CompactFallbackRoute::Routed(route))
                    if route.destination() == zoned && route.ticks() == 1.0)
            );
        }
    }
}

#[test]
fn ordered_first_compares_destination_zones_with_safe_arrivals_and_keeps_ties() {
    let collision = bake(5, 5, &[]);
    let mut graph = TransportGraph::default();
    install_zones(&collision, &mut graph, vec![rect_zone(1, 2, 0, 0)]);
    let from = tile(0, 0, 0);
    let zoned = tile(2, 0, 0);
    let tied_safe = tile(0, 2, 0);
    let farther_safe = tile(0, 4, 0);
    let state = WorldState::empty();
    let options = FindOptions {
        prefer_target_order: true,
        ..FindOptions::default()
    };
    let world = Arc::new(crate::world::NavWorld::from_parts(
        collision,
        graph,
        Vec::new(),
    ));
    let lease = crate::producer::lease(&world).expect("ordered zone fixture lease");
    for (targets, expected) in [
        ([farther_safe, zoned], zoned),
        ([zoned, tied_safe], zoned),
        ([tied_safe, zoned], tied_safe),
    ] {
        let single = find_with(
            &world.collision,
            &world.graph,
            from,
            expected,
            options,
            &state,
        )
        .expect("the destination keeps its own-zone permission");
        let synchronous = find_first_with(
            &world.collision,
            &world.graph,
            from,
            &targets,
            options,
            &state,
        )
        .into_route()
        .expect("the ordered goal set has a route");
        assert_eq!(
            (synchronous.dest, synchronous.ticks),
            (expected, single.ticks)
        );
        let completion = advance_real_route_completion(
            super::SearchRequest::First {
                from,
                targets: Box::new(targets),
                fallback: Box::new([]),
                options,
                avoid: Box::new([]),
                max_settles: super::NODE_BUDGET,
            },
            &lease,
            &state,
        );
        let super::SearchOutcome::First { search } = completion.into_parts().0 else {
            panic!("ordered first-goal cursor returned the wrong result shape");
        };
        let (route, fallback) = search.into_routes();
        let route = route.expect("the cooperative ordered goal set has a route");
        assert_eq!(
            (route.destination(), route.ticks()),
            (expected, single.ticks)
        );
        assert!(fallback.is_none());
    }
}

#[test]
fn ordered_first_keeps_unrelated_zone_transit_grants_as_fallback_permissions() {
    let collision = bake(5, 7, &[]);
    let mut graph = TransportGraph::default();
    install_zones(&collision, &mut graph, vec![rect_zone(1, 3, 0, 6)]);
    let from = tile(0, 0, 0);
    let safe = tile(0, 6, 0);
    let past_zone = tile(4, 0, 0);
    let state = WorldState::empty();
    let options = FindOptions {
        prefer_target_order: true,
        zones: ZoneExempt::all(),
        ..FindOptions::default()
    };
    let granted = find_with(&collision, &graph, from, past_zone, options, &state).unwrap();
    let route = find_first_with(
        &collision,
        &graph,
        from,
        &[past_zone, safe],
        options,
        &state,
    )
    .into_route()
    .unwrap();
    assert_eq!((route.dest, route.ticks), (safe, 3.0));
    assert!(granted.ticks < route.ticks);
}

#[test]
fn ordered_bank_failure_keeps_the_first_access_zone_witness() {
    let collision = bake(9, 1, &[]);
    let mut graph = TransportGraph::default();
    install_zones(&collision, &mut graph, vec![rect_zone(2, 3, 0, 0)]);
    let stands = [BankStand {
        name: "Fixture bank".into(),
        tile: tile(6, 0, 0),
        access: BankAccess::Booth { op: 2 },
    }];
    let from = tile(0, 0, 0);
    let state = WorldState::empty();
    let first = crate::bank_fetch::nearest_bank_access(&collision, &stands, from).unwrap();
    assert_eq!(first, tile(5, 0, 0));
    let error = crate::bank_fetch::plan_bank_fetch(
        &[MissingReq::Carry { id: 995, count: 1 }],
        &state,
        &[(995, 1)],
        &stands,
        from,
        &collision,
        &graph,
        FindOptions::default(),
        &[],
    )
    .expect_err("every access needs unrelated danger-zone transit");
    assert_eq!(
        error,
        crate::bank_fetch::BankFetchError::Access {
            access: first,
            error: RouteError::NoPath,
            blocked_zones: vec![ZoneKey::Zone(0)],
        }
    );
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
    assert_eq!(search.completion_partitions(), 0);
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

#[test]
fn blocked_normal_ghast_membership_follows_its_engine_wander_over_the_bog() {
    // REVIEW-NAV-WWM-ROUTE R1: `ghast_invis` (size 2, `moverestrict=
    // blocked+normal`, which the engine steps under LINE_OF_SIGHT) wanders
    // from (3478,3328) over Mort Myre bog to (3477,3325), cardinal-adjacent to
    // (3476,3325). A NORMAL flood carved that tile.
    let Some(world) = crate::world::NavWorld::load_default_pack_or_skip() else {
        return;
    };
    let table = world.graph.zones.as_ref().expect("v12 zones");
    let ghast = table.resolve("ghast_invis@3478,3328,0").unwrap();
    let ZoneKey::Zone(index) = ghast else {
        panic!("the ghast is ungrouped");
    };
    let zone = table.zones()[usize::from(index)];
    let member = |x: i32, z: i32| table.at(tile(x, z, 0)).any(|hit| hit == index);
    assert!(
        member(3476, 3325),
        "the wander witness's player tile stays dangerous"
    );
    let members = (zone.min_z..=zone.max_z)
        .flat_map(|z| (zone.min_x..=zone.max_x).map(move |x| (x, z)))
        .filter(|&(x, z)| member(x, z))
        .count();
    // The engine's own StepValidator/LineValidator flood over the same raw
    // flags (NAV-WWM-ROUTE-R2 `oracle/engine-audit.ts`) acquires from exactly
    // 187 of the 289 rectangle tiles.
    assert_eq!(members, 187);
}

#[test]
fn request_shape_input_bounds_fail_closed_on_overflow() {
    let many = crate::router::SearchRequestShape::Many {
        target_capacity: usize::MAX,
        options: FindOptions::default(),
        avoid_capacity: 1,
        max_settles: 64,
        reconstruct_best: false,
    };
    assert!(many.retained_input_bytes().is_none());

    let first = crate::router::SearchRequestShape::First {
        target_capacity: usize::MAX,
        fallback_capacity: 1,
        options: FindOptions::default(),
        avoid_capacity: 0,
        max_settles: 64,
    };
    assert!(first.target_count().is_none());
}

#[test]
fn missing_item_reservations_charge_carry_and_worn_plan_rows() {
    let mut graph = door(tile(0, 0, 0), tile(1, 0, 0), 1);
    graph.edges[0].item_req = vec![(10, 1), (11, 1)];
    graph.edges[0].consumed_req = vec![(12, 1)];
    graph.edges[0].item_returns = vec![(13, 1)];
    graph.edges[0].worn_req = vec![20, 21, 22];
    let mut teleport = teleport(tile(2, 0, 0), 1, Vec::new(), vec![(14, 1)])
        .teleports
        .remove(0);
    teleport.worn_req = vec![21, 22];
    graph.teleports.push(teleport);

    let envelope = crate::contract::SearchGraphEnvelope::from_graph(&graph);
    assert_eq!(envelope.carry_rows, 5);
    assert_eq!(envelope.worn_rows, 5);
    assert_eq!(envelope.max_worn_ids_per_edge, 3);

    let reservation = super::search_worst_case_reservation(
        super::SearchRequestShape::MissingItemReqs {
            target_capacity: 1,
            fallback_capacity: 1,
            options: FindOptions::default(),
            avoid_capacity: 0,
            max_settles: 8,
        },
        &graph,
        envelope,
    )
    .unwrap();
    assert_eq!(reservation.max_missing_requests, 13);
    assert_eq!(reservation.max_missing_carry_ids, 5);
    assert_eq!(reservation.max_missing_worn_ids, 24);
    assert_eq!(reservation.max_plan_item_rows, 10);

    let route_reservation = super::route_missing_requirements_reservation(0, 1, envelope).unwrap();
    assert_eq!(route_reservation.max_missing_requests, 6);
    assert_eq!(route_reservation.max_missing_carry_ids, 5);
    assert_eq!(route_reservation.max_missing_worn_ids, 3);
    assert_eq!(route_reservation.max_plan_item_rows, 6);
}

#[test]
fn missing_item_result_retention_uses_route_edges_not_the_search_budget() {
    let mut graph = toll_graph();
    graph.edges[0].worn_req = vec![7, 8];
    let world = Arc::new(crate::world::NavWorld::from_parts(
        walled_5x5(),
        graph,
        Vec::new(),
    ));
    let lease = crate::contract::NavLease::wrap(
        world,
        crate::contract::PackDigest::of(b"small toll route with a full search budget"),
    )
    .unwrap();
    let outcome = advance_real_route_cursor(
        super::SearchRequest::MissingItemReqs {
            from: tile(0, 0, 0),
            targets: vec![tile(4, 4, 0)].into_boxed_slice(),
            fallback: Box::default(),
            options: FindOptions::default(),
            avoid: Box::default(),
            max_settles: super::NODE_BUDGET,
        },
        &lease,
        &WorldState::empty(),
    );
    let super::SearchOutcome::MissingItemReqs(Some(missing)) = outcome else {
        panic!("the toll route must return its real carry and worn requirements");
    };
    assert_eq!(missing.len(), 2);
    assert!(matches!(
        missing.requirement_at(0),
        Some(super::MissingReqView::WearAny { ids: [7, 8] })
    ));
    assert!(matches!(
        missing.requirement_at(1),
        Some(super::MissingReqView::Carry { id: 995, count: 10 })
    ));
    assert!(
        missing.entries.capacity() <= 2,
        "one traversed edge and one carry ID need only two retained rows, not {}",
        missing.entries.capacity()
    );
}

#[test]
#[ignore = "loads the two reviewed real nav packs"]
fn reviewed_real_packs_have_exact_carry_and_worn_row_counts() {
    for (variable, fallback, expected_carry, expected_worn) in [
        (
            "NAV_PACK",
            "/srv/dev/scratch/274bot-evidence/NAV-GATED-ENTRANCES/reference-v17/274bot-289-v17.navpack",
            251,
            3_603,
        ),
        (
            "T5W_MEMBERSHIP_NAVPACK_274",
            "/srv/dev/scratch/274bot-evidence/NAV-GATED-ENTRANCES/reference-v17/274bot-274-v17.navpack",
            182,
            3_483,
        ),
    ] {
        let path = std::env::var_os(variable)
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(fallback));
        let world = crate::world::NavWorld::load_pack(&path)
            .expect("load the reviewed v17 pack from its real content");
        let envelope = crate::contract::SearchGraphEnvelope::from_graph(&world.graph);
        assert!(envelope.is_representable());
        assert_eq!(
            (envelope.carry_rows, envelope.worn_rows),
            (expected_carry, expected_worn),
            "{}",
            path.display()
        );
    }
}

#[test]
fn real_289_preflight_charges_exact_target_capacity_before_materialization() {
    let Some(world) = crate::world::NavWorld::load_default_pack_or_skip() else {
        return;
    };
    let world = Arc::new(world);
    let digest = *world
        .content
        .get()
        .expect("pack records its content digest");
    let bound_identity = crate::producer::bind(&world, digest).expect("bind the loaded pack once");
    assert!(
        world.search_envelope.get().is_some(),
        "bind pins the checked graph envelope before publication"
    );
    let lease = crate::producer::lease(&world).expect("bound real pack lease");
    assert_eq!(lease.identity(), bound_identity);
    assert_eq!(
        lease.search_envelope(),
        crate::contract::SearchGraphEnvelope::from_graph(&world.graph)
    );
    let state = Arc::new(WorldState::empty());
    let options = FindOptions::default();
    let shape = crate::router::SearchRequestShape::Many {
        target_capacity: 3,
        options,
        avoid_capacity: 2,
        max_settles: 64,
        reconstruct_best: true,
    };
    let preflight =
        crate::router::SearchCursor::preflight(shape, &lease, &state).expect("static preflight");
    let expected_input_bytes =
        3 * std::mem::size_of::<WorldTile>() + 2 * std::mem::size_of::<AvoidRect>();
    assert_eq!(preflight.input_bytes(), expected_input_bytes);
    assert!(preflight.reservation().future_layout_bytes > 0);
    let expected_reservation = preflight.reservation();

    let request = crate::router::SearchRequest::Many {
        from: tile(3222, 3218, 0),
        targets: vec![
            tile(3092, 3243, 0),
            tile(3253, 3420, 0),
            tile(3092, 3243, 0),
        ]
        .into_boxed_slice(),
        options,
        avoid: vec![
            AvoidRect {
                min_x: 1,
                max_x: 2,
                min_z: 3,
                max_z: 4,
                level: Some(0),
            },
            AvoidRect {
                min_x: 5,
                max_x: 6,
                min_z: 7,
                max_z: 8,
                level: Some(0),
            },
        ]
        .into_boxed_slice(),
        max_settles: 64,
        reconstruct_best: true,
    };
    let cursor = crate::router::SearchCursor::new_preflighted(
        request,
        lease.clone(),
        Arc::clone(&state),
        preflight,
    )
    .expect("materialized request matches its preflight");
    assert_eq!(cursor.input_bytes(), expected_input_bytes);
    assert_eq!(cursor.reservation(), expected_reservation);
    drop(cursor);
    let capacity_preflight =
        crate::router::SearchCursor::preflight(shape, &lease, &state).expect("same-pack shape");
    let too_many_targets = crate::router::SearchRequest::Many {
        from: tile(3222, 3218, 0),
        targets: vec![
            tile(3092, 3243, 0),
            tile(3253, 3420, 0),
            tile(3092, 3243, 0),
            tile(3253, 3420, 0),
        ]
        .into_boxed_slice(),
        options,
        avoid: Vec::<AvoidRect>::new().into_boxed_slice(),
        max_settles: 64,
        reconstruct_best: true,
    };
    assert!(matches!(
        crate::router::SearchCursor::new_preflighted(
            too_many_targets,
            lease.clone(),
            Arc::clone(&state),
            capacity_preflight,
        ),
        Err(crate::router::SearchCursorError::PreflightMismatch)
    ));
    let another_preflight =
        crate::router::SearchCursor::preflight(shape, &lease, &state).expect("same-pack shape");
    let another_lease = crate::contract::NavLease::wrap(Arc::clone(&world), digest)
        .expect("new lease identity for identical content");
    let same_shape_request = crate::router::SearchRequest::Many {
        from: tile(3222, 3218, 0),
        targets: vec![tile(3092, 3243, 0)].into_boxed_slice(),
        options,
        avoid: Vec::new().into_boxed_slice(),
        max_settles: 64,
        reconstruct_best: true,
    };
    assert!(matches!(
        crate::router::SearchCursor::new_preflighted(
            same_shape_request,
            another_lease,
            Arc::clone(&state),
            another_preflight,
        ),
        Err(crate::router::SearchCursorError::PreflightMismatch)
    ));
}

fn advance_real_route_completion(
    request: crate::router::SearchRequest,
    lease: &crate::contract::NavLease,
    state: &WorldState,
) -> crate::router::SearchCompletion {
    let state = Arc::new(state.clone());
    let shape = crate::router::SearchRequestShape::from_request(&request)
        .expect("real-content request input arithmetic fits");
    let preflight = crate::router::SearchCursor::preflight(shape, lease, &state)
        .expect("real-content shape fits its static reservation");
    let input_bytes = preflight.input_bytes();
    let reservation = preflight.reservation();
    let mut cursor = crate::router::SearchCursor::new_preflighted(
        request,
        lease.clone(),
        Arc::clone(&state),
        preflight,
    )
    .expect("real-content request fits its static reservation");
    assert_eq!(cursor.input_bytes(), input_bytes);
    assert_eq!(cursor.reservation(), reservation);
    let clock = SystemClock;
    let cancellation = SharedCancellation::new();
    let units = NonZeroU32::new(64).expect("finite nonzero work slice");
    let mut yielded = false;
    let mut last_forward_settled = 0;
    loop {
        let budget = SliceBudget::new(units, Deadline::after(clock.now(), Duration::from_secs(5)));
        let mut context = StepContext::<InspectionPhase>::new(&clock, budget, &cancellation);
        match cursor.advance(&mut context) {
            crate::router::CursorStep::Complete(completion) => {
                // A borrowed exact-route diagnosis can finish within one slice.
                assert!(
                    yielded || completion.forward_settled() < units.get() as usize,
                    "a real-content forward search that exceeds one slice must yield"
                );
                assert_eq!(cursor.forward_settled(), completion.forward_settled());
                return completion;
            }
            crate::router::CursorStep::Yielded => {
                yielded = true;
                let settled = cursor.forward_settled();
                assert!(settled >= last_forward_settled);
                last_forward_settled = settled;
            }
            crate::router::CursorStep::Cancelled(reason) => {
                panic!("unexpected route-search cancellation: {reason:?}")
            }
            crate::router::CursorStep::Failed(error) => {
                panic!("route-search cursor failed: {error:?}")
            }
        }
    }
}

fn advance_real_route_cursor(
    request: crate::router::SearchRequest,
    lease: &crate::contract::NavLease,
    state: &WorldState,
) -> crate::router::SearchOutcome {
    advance_real_route_completion(request, lease, state)
        .into_parts()
        .0
}

#[test]
fn retained_compact_view_is_allocation_free_and_pack_checked() {
    let world = Arc::new(crate::world::NavWorld::from_parts(
        WorldCollision {
            origin: tile(0, 0, 0),
            width: 128,
            height: 1,
            walk: vec![0; 128 * 4],
            blocked: vec![0; 8],
            flags: None,
        },
        TransportGraph::default(),
        Vec::new(),
    ));
    let lease = crate::producer::lease(&world).unwrap();
    let outcome = advance_real_route_cursor(
        crate::router::SearchRequest::Single {
            from: tile(0, 0, 0),
            to: tile(127, 0, 0),
            options: FindOptions::default(),
            avoid: Vec::new().into_boxed_slice(),
            max_settles: 256,
        },
        &lease,
        &WorldState::empty(),
    );
    let crate::router::SearchOutcome::Single {
        route: Ok(mut route),
    } = outcome
    else {
        panic!("synthetic corridor must produce a compact route")
    };
    let allocations = allocation_counter::measure(|| {
        let view = crate::producer::compact_route_view(&lease, &route).unwrap();
        assert_eq!(view.pack(), lease.identity().pack());
        assert_eq!(view.destination(), tile(127, 0, 0));
        assert_eq!(view.ticks(), route.ticks());
        assert_eq!(view.leg_count(), 1);
    });
    assert_eq!(allocations.bytes_total, 0);
    let foreign = crate::contract::NavLease::wrap(
        Arc::clone(&world),
        crate::contract::PackDigest::of(b"foreign compact assessment"),
    )
    .unwrap();
    assert_eq!(
        crate::producer::compact_route_view(&foreign, &route).unwrap_err(),
        crate::contract::ContractError::PackMismatch
    );
    route.lease = None;
    assert_eq!(
        crate::producer::compact_route_view(&lease, &route).unwrap_err(),
        crate::contract::ContractError::PackMismatch
    );
}

#[test]
fn real_289_cooperative_request_shapes_match_legacy_serialization() {
    let Some(world) = crate::world::NavWorld::load_default_pack_or_skip() else {
        return;
    };
    let world = Arc::new(world);
    let lease = crate::producer::lease(&world).expect("real pack lease");
    assert_eq!(
        lease.identity().content().bytes(),
        [
            0x4e, 0x8a, 0xb5, 0xaf, 0x46, 0x94, 0xcf, 0x67, 0x22, 0xed, 0x38, 0x36, 0xc3, 0xe1,
            0xf4, 0x2e, 0x9a, 0xc1, 0xbe, 0x13, 0x6d, 0x4d, 0x11, 0x27, 0xc7, 0xdb, 0x69, 0x8f,
            0x4f, 0xc6, 0x50, 0x0b,
        ],
        "qualified entrance pack SHA-256 is 4e8ab5af4694cf6722ed3836c3e1f42e9ac1be136d4d1127c7db698f4fc6500b; update this pin with the transport and route corpus diff"
    );
    assert_eq!(
        world.graph.edges.len(),
        3_361,
        "qualified entrance-pack census: 3249 unchanged edges plus 112 additions"
    );
    assert_eq!(
        world.graph.quest_family,
        Some(crate::quest_gates::QuestFamilyId::new(
            [
                0xd5, 0x89, 0xa7, 0xd7, 0x77, 0x05, 0x21, 0xa7, 0x2d, 0xc5, 0x02, 0x24, 0x5d, 0x41,
                0x61, 0x32, 0x14, 0x25, 0x9e, 0xc9, 0x2a, 0xc0, 0x96, 0x8a, 0x13, 0xb9, 0x33, 0x11,
                0xdd, 0xb9, 0xb5, 0x9a,
            ],
            std::num::NonZeroU16::new(1).expect("quest schema is nonzero"),
        )),
        "qualified selected quest family"
    );
    assert_eq!(
        world
            .graph
            .edges
            .iter()
            .filter(|edge| edge.quest_gates.is_some())
            .count(),
        62,
        "all 62 quest-gated edges are additions; the 3249 reference edges are unchanged"
    );
    let from = tile(3222, 3218, 0);
    let west_bank = tile(3092, 3243, 0);
    let east_bank = tile(3253, 3420, 0);
    let empty = WorldState::empty();
    let options = FindOptions::default();

    let old_single = find_with(
        &world.collision,
        &world.graph,
        from,
        west_bank,
        options,
        &empty,
    );
    assert!(old_single.is_ok(), "real west-bank route must be reachable");
    let new_single_completion = advance_real_route_completion(
        crate::router::SearchRequest::Single {
            from,
            to: west_bank,
            options,
            avoid: Vec::new().into_boxed_slice(),
            max_settles: crate::router::NODE_BUDGET,
        },
        &lease,
        &empty,
    );
    assert!(new_single_completion.forward_settled() > 0);
    {
        let view = new_single_completion
            .route_view(&lease)
            .expect("same-pack borrowed compact view")
            .expect("successful single route has a view");
        assert_eq!(view.destination(), west_bank);
        assert!(view.leg_count() > 0);
    }
    let new_single = new_single_completion.into_parts().0;
    let crate::router::SearchOutcome::Single { mut route } = new_single else {
        panic!("single cursor returned the wrong result shape")
    };
    if let Ok(route) = &mut route {
        route.retain_lease(lease.clone());
    }
    let old_bytes = format!("{old_single:#?}");
    let new_bytes = format!("{route:#?}");
    assert_eq!(
        old_bytes.as_bytes(),
        new_bytes.as_bytes(),
        "single route outcome must serialize byte-for-byte"
    );
    let compact_route = route.expect("real single cursor route is published");
    let result_bytes = crate::producer::compact_route_result_bytes(&compact_route)
        .expect("real compact plan size fits");
    let owner = crate::producer::OwnerResultLedger::with_limits(result_bytes, result_bytes);
    let reservation = owner
        .reserve_compact_route(&compact_route)
        .expect("exact real compact result reservation");
    assert_eq!(reservation.bytes(), result_bytes);
    let identity = api::identity::WorkIdentity::new(
        api::identity::RequestIdMinter::new()
            .mint()
            .expect("host request id"),
        api::identity::ProcessIncarnation::from_start_nonce(std::num::NonZeroU64::MIN),
        api::identity::Scope::Process,
        None,
    )
    .expect("valid route work identity");
    let plan = crate::producer::publish_compact_route(
        &lease,
        identity,
        api::work::InputRevision::FIRST,
        compact_route,
        reservation,
    );
    assert_eq!(owner.used_bytes(), result_bytes);
    assert_eq!(plan.identity(), identity);
    assert_eq!(plan.revision(), api::work::InputRevision::FIRST);
    assert_eq!(
        plan.view(&lease)
            .expect("same-pack real route view")
            .destination(),
        west_bank
    );
    let other_lease = crate::contract::NavLease::wrap(
        Arc::clone(&world),
        crate::contract::PackDigest::of(b"other real-pack lease"),
    )
    .expect("distinct route-view lease");
    assert_eq!(
        plan.view(&other_lease).unwrap_err(),
        crate::contract::ContractError::PackMismatch
    );
    let retained_plan = plan.clone();
    drop(plan);
    assert_eq!(owner.used_bytes(), result_bytes);
    drop(retained_plan);
    assert_eq!(owner.used_bytes(), 0);

    let targets = [west_bank, east_bank, west_bank];
    let old_many = find_many_with(
        &world.collision,
        &world.graph,
        from,
        &targets,
        options,
        &empty,
    );
    let old_best_index = old_many
        .results()
        .iter()
        .enumerate()
        .filter_map(|(index, result)| result.as_ref().ok().map(|cost| (index, cost.ticks)))
        .min_by(|(left_index, left_ticks), (right_index, right_ticks)| {
            left_ticks
                .total_cmp(right_ticks)
                .then_with(|| left_index.cmp(right_index))
        })
        .map(|(index, _)| index);
    assert!(
        old_best_index.is_some(),
        "real many-target search has a route"
    );
    let old_best = old_best_index.map(|index| {
        (
            index,
            old_many
                .route(index)
                .expect("settled legacy many-target route"),
        )
    });
    assert!(
        old_many.results().iter().all(|result| result.is_ok()),
        "all real bank targets must be reachable"
    );
    let old_bytes = format!("results={:#?}\nbest={old_best:#?}", old_many.results());
    let new_many_completion = advance_real_route_completion(
        crate::router::SearchRequest::Many {
            from,
            targets: targets.to_vec().into_boxed_slice(),
            options,
            avoid: Vec::new().into_boxed_slice(),
            max_settles: crate::router::NODE_BUDGET,
            reconstruct_best: true,
        },
        &lease,
        &empty,
    );
    let many_view = new_many_completion
        .route_view(&lease)
        .expect("same-pack borrowed many-target view")
        .expect("best many-target route has a view");
    assert_eq!(many_view.destination(), west_bank);
    let new_many = new_many_completion.into_parts().0;
    let crate::router::SearchOutcome::Many { results, mut best } = new_many else {
        panic!("many cursor returned the wrong result shape")
    };
    if let Some((_, route)) = &mut best {
        route.retain_lease(lease.clone());
    }
    let new_bytes = format!("results={:#?}\nbest={best:#?}", results.as_ref());
    assert_eq!(
        old_bytes.as_bytes(),
        new_bytes.as_bytes(),
        "many-target outcomes must serialize byte-for-byte"
    );

    let preferred = [east_bank];
    let fallback = [west_bank];
    let old_first = find_first_with_fallback(
        &world.collision,
        &world.graph,
        from,
        &preferred,
        &fallback,
        options,
        &empty,
    );
    assert!(
        old_first.route().is_ok(),
        "preferred real bank must be reachable"
    );
    assert!(
        old_first.fallback().is_none(),
        "a successful preferred route suppresses the fallback outcome"
    );
    let old_settled = old_first.settled();
    let old_proof = old_first.proof();
    let old_capacities = old_first.scratch_capacities();
    let old_partitions = old_first.completion_partitions();
    let (old_route, old_fallback) = old_first.into_routes();
    let old_bytes = format!(
        "routes={:#?}\nsettled={old_settled}\nproof={old_proof:?}\npartitions={old_partitions}",
        (old_route, old_fallback)
    );
    let new_first_completion = advance_real_route_completion(
        crate::router::SearchRequest::First {
            from,
            targets: preferred.to_vec().into_boxed_slice(),
            fallback: fallback.to_vec().into_boxed_slice(),
            options,
            avoid: Vec::new().into_boxed_slice(),
            max_settles: crate::router::NODE_BUDGET,
        },
        &lease,
        &empty,
    );
    let new_first_forward_settled = new_first_completion.forward_settled();
    let first_view = new_first_completion
        .route_view(&lease)
        .expect("same-pack borrowed first-target view")
        .expect("successful preferred first-target route has a view");
    assert_eq!(first_view.destination(), east_bank);
    let new_first = new_first_completion.into_parts().0;
    let crate::router::SearchOutcome::First { search } = new_first else {
        panic!("first-goal cursor returned the wrong result shape")
    };
    let new_settled = search.settled();
    assert_eq!(
        new_first_forward_settled, new_settled,
        "completion must report every forward label settled by its First passes"
    );
    let new_proof = search.proof();
    let new_capacities = search.scratch_capacities();
    let new_partitions = search.completion_partitions();
    let (mut new_route, mut new_fallback) = search.into_routes();
    assert!(
        new_route.is_ok(),
        "preferred real cursor route must be reachable"
    );
    assert!(
        new_fallback.is_none(),
        "a successful preferred cursor route suppresses the fallback outcome"
    );
    if let Ok(route) = &mut new_route {
        route.retain_lease(lease.clone());
    }
    if let Some(crate::router::CompactFallbackRoute::Routed(route)) = &mut new_fallback {
        route.retain_lease(lease.clone());
    }
    let new_bytes = format!(
        "routes={:#?}\nsettled={new_settled}\nproof={new_proof:?}\npartitions={new_partitions}",
        (new_route, new_fallback)
    );
    assert_eq!(
        old_bytes.as_bytes(),
        new_bytes.as_bytes(),
        "first-goal and fallback outcomes must serialize byte-for-byte"
    );
    eprintln!(
        "first-goal representation capacities: old={old_capacities:#?} new={new_capacities:#?}"
    );

    let item_state = real_maxed_state();
    let item_options = real_resilient_opts();
    let item_from = tile(3202, 9909, 0);
    let item_targets = [tile(3209, 9891, 0), tile(3210, 9892, 0)];
    let old_missing = find_first_missing_item_reqs_with_avoid(
        &world.collision,
        &world.graph,
        item_from,
        &item_targets,
        &[],
        item_options,
        &item_state,
        &[],
    );
    let new_missing = advance_real_route_cursor(
        crate::router::SearchRequest::MissingItemReqs {
            from: item_from,
            targets: item_targets.to_vec().into_boxed_slice(),
            fallback: Vec::new().into_boxed_slice(),
            options: item_options,
            avoid: Vec::new().into_boxed_slice(),
            max_settles: crate::router::NODE_BUDGET,
        },
        &lease,
        &item_state,
    );
    let crate::router::SearchOutcome::MissingItemReqs(missing) = new_missing else {
        panic!("missing-item cursor returned the wrong result shape")
    };
    assert_eq!(
        old_missing
            .as_ref()
            .map(|requests| format!("{requests:#?}")),
        missing.as_ref().map(|requests| format!("{requests:#?}")),
        "real sewer-web carry diagnosis must serialize byte-for-byte"
    );
    assert_eq!(
        old_missing,
        Some(vec![MissingReq::Carry { id: 946, count: 1 }]),
        "fixture must exercise the real knife gate"
    );

    let (edge_index, worn_edge, is_teleport) = world
        .graph
        .edges
        .iter()
        .enumerate()
        .find(|(_, edge)| edge.worn_req.len() > 1)
        .map(|(index, edge)| (index, edge, false))
        .or_else(|| {
            world
                .graph
                .teleports
                .iter()
                .enumerate()
                .find(|(_, edge)| edge.worn_req.len() > 1)
                .map(|(index, edge)| (index, edge, true))
        })
        .expect("the real 289 pack includes a multi-alternative worn transport");
    assert!(
        worn_edge.worn_req.windows(2).all(|pair| pair[0] < pair[1]),
        "the real web loader must retain its canonical sorted unique alternatives"
    );
    let edge_index = u32::try_from(edge_index).expect("real worn edge index fits");
    assert_eq!(edge_index & crate::router::DIAGNOSTIC_TELEPORT_BIT, 0);
    let old_worn_route = crate::router::Route {
        legs: vec![crate::router::Leg::Transport {
            edge: Box::new(worn_edge.clone()),
        }],
        dest: worn_edge.to,
        ticks: f64::from(worn_edge.ticks),
    };
    let no_items_or_worn = WorldState::empty();
    let old_worn = crate::router::missing_item_reqs(&old_worn_route, &no_items_or_worn);
    let new_worn_route = crate::router::CompactRoute {
        origin: worn_edge.at,
        dest: worn_edge.to,
        ticks: f64::from(worn_edge.ticks),
        walk_tiles: Vec::new().into_boxed_slice(),
        tags: vec![if is_teleport { 2 } else { 1 }].into_boxed_slice(),
        transition_count: 1,
        leg_count: 1,
        edge_ids: vec![edge_index].into_boxed_slice(),
        essence: None,
        checkpoints: Vec::new().into_boxed_slice(),
        lease: Some(lease.clone()),
    };
    let worn_route_backing = (
        new_worn_route.tags.as_ptr(),
        new_worn_route.edge_ids.as_ptr(),
    );
    let new_worn_outcome = advance_real_route_cursor(
        crate::router::SearchRequest::MissingItemReqsForRoute {
            route: new_worn_route,
        },
        &lease,
        &no_items_or_worn,
    );
    let crate::router::SearchOutcome::MissingItemReqsForRoute {
        route: returned_worn_route,
        missing: new_worn,
    } = new_worn_outcome
    else {
        panic!("real worn compact-route cursor returned the wrong result shape")
    };
    assert_eq!(
        worn_route_backing,
        (
            returned_worn_route.tags.as_ptr(),
            returned_worn_route.edge_ids.as_ptr(),
        ),
        "route diagnosis must return the input route's original backing"
    );
    assert_eq!(
        format!("{old_worn:#?}").as_bytes(),
        format!("{new_worn:#?}").as_bytes(),
        "real worn requirements must preserve legacy ordering and serialization"
    );
    let worn_index = (0..new_worn.len())
        .find(|&index| {
            matches!(
                new_worn.requirement_at(index),
                Some(crate::router::MissingReqView::WearAny { .. })
            )
        })
        .expect("real result includes its worn requirement");
    let Some(crate::router::MissingReqView::WearAny { ids }) = new_worn.requirement_at(worn_index)
    else {
        panic!("borrowed real worn requirement has the wrong shape")
    };
    assert!(
        std::ptr::eq(ids.as_ptr(), worn_edge.worn_req.as_ptr()),
        "worn IDs must borrow their immutable source-edge group"
    );
    assert_eq!(ids, worn_edge.worn_req.as_slice());

    let mut fetchable_state = item_state.clone();
    fetchable_state.inv.insert(946, 1);
    let route_target = item_targets[0];
    let old_fetchable_route = find_with(
        &world.collision,
        &world.graph,
        item_from,
        route_target,
        item_options,
        &fetchable_state,
    )
    .expect("the real sewer-web route opens with the fetched knife");
    let new_fetchable_route = advance_real_route_cursor(
        crate::router::SearchRequest::Single {
            from: item_from,
            to: route_target,
            options: item_options,
            avoid: Vec::new().into_boxed_slice(),
            max_settles: crate::router::NODE_BUDGET,
        },
        &lease,
        &fetchable_state,
    );
    let crate::router::SearchOutcome::Single {
        route: Ok(new_fetchable_route),
    } = new_fetchable_route
    else {
        panic!("fetchable real route cursor did not return a compact route")
    };
    assert_eq!(
        format!("{old_fetchable_route:#?}").as_bytes(),
        format!("{new_fetchable_route:#?}").as_bytes(),
        "fetchable real route must serialize byte-for-byte"
    );
    let old_exact_missing = crate::router::missing_item_reqs(&old_fetchable_route, &item_state);
    let route_backing = (
        new_fetchable_route.walk_tiles.as_ptr(),
        new_fetchable_route.tags.as_ptr(),
        new_fetchable_route.edge_ids.as_ptr(),
        new_fetchable_route.checkpoints.as_ptr(),
    );
    let route_bytes = new_fetchable_route.try_retained_bytes().unwrap();
    let envelope = lease.search_envelope();
    let empty_route_bound = crate::router::route_missing_requirements_reservation(
        0,
        new_fetchable_route.edge_ids.len(),
        envelope,
    )
    .unwrap();
    let owned_route_bound = crate::router::route_missing_requirements_reservation(
        route_bytes,
        new_fetchable_route.edge_ids.len(),
        envelope,
    )
    .unwrap();
    assert_eq!(
        owned_route_bound.result_bytes - empty_route_bound.result_bytes,
        route_bytes,
        "terminal diagnosis must charge its returned route backing"
    );
    assert_eq!(
        owned_route_bound.peak_bytes - empty_route_bound.peak_bytes,
        route_bytes,
        "the diagnostic peak must cover its returned route backing"
    );
    let new_exact_outcome = advance_real_route_cursor(
        crate::router::SearchRequest::MissingItemReqsForRoute {
            route: new_fetchable_route,
        },
        &lease,
        &item_state,
    );
    let crate::router::SearchOutcome::MissingItemReqsForRoute {
        route: returned_route,
        missing: new_exact_missing,
    } = new_exact_outcome
    else {
        panic!("exact-route cursor returned the wrong result shape")
    };
    assert_eq!(
        route_backing,
        (
            returned_route.walk_tiles.as_ptr(),
            returned_route.tags.as_ptr(),
            returned_route.edge_ids.as_ptr(),
            returned_route.checkpoints.as_ptr(),
        ),
        "route diagnosis must return the same backing allocations"
    );
    assert_eq!(
        format!("{old_fetchable_route:#?}").as_bytes(),
        format!("{returned_route:#?}").as_bytes(),
        "route diagnosis must return the unchanged real route"
    );
    assert_eq!(
        format!("{old_exact_missing:#?}").as_bytes(),
        format!("{new_exact_missing:#?}").as_bytes(),
        "exact real compact-route diagnosis must serialize byte-for-byte"
    );
    assert_eq!(
        new_exact_missing.try_retained_bytes(),
        new_exact_missing
            .entries
            .capacity()
            .checked_mul(std::mem::size_of::<super::MissingReqEntry>())
    );
    assert_eq!(
        old_exact_missing,
        vec![MissingReq::Carry { id: 946, count: 1 }],
        "exact route diagnosis must inspect the knife edge after fetchable search"
    );

    let blocker_state = zones_rich_289_state(Some(126));
    let mountain = tile(2664, 3664, 0);
    let old_blockers = crate::router::find_blocking_zones(
        &world.collision,
        &world.graph,
        from,
        mountain,
        options,
        &blocker_state,
        &[],
    );
    let table = world.graph.zones.as_ref().expect("real v12 zone table");
    let zone_filter = ZoneFilterOwned::new(
        table.zones().len(),
        blocker_state.combat_level,
        &[from, mountain],
        ZoneExempt::NONE,
    )
    .expect("bounded real-content blocking filter");
    let new_blockers = advance_real_route_cursor(
        crate::router::SearchRequest::BlockingZones {
            from,
            targets: vec![mountain].into_boxed_slice(),
            diagnosis: crate::router::BlockerDiagnosis::SingleTarget,
            options,
            avoid: Vec::new().into_boxed_slice(),
            max_settles: crate::router::NODE_BUDGET,
            zone_filter,
        },
        &lease,
        &blocker_state,
    );
    let crate::router::SearchOutcome::BlockingZones(new_blockers) = new_blockers else {
        panic!("blocking-zone cursor returned the wrong result shape")
    };
    assert!(
        old_blockers.as_ref().is_some_and(|keys| !keys.is_empty()),
        "real white-wolf-mountain content must provide a blocker witness"
    );
    let old_bytes = format!("{old_blockers:#?}");
    let new_bytes = format!("{new_blockers:#?}");
    assert_eq!(
        old_bytes.as_bytes(),
        new_bytes.as_bytes(),
        "blocking-zone keys must serialize byte-for-byte"
    );
    let blocker_keys = old_blockers
        .as_deref()
        .expect("blocker witness remains owned");
    let mut expected_detail = String::from("blocked by danger zones: ");
    for (index, &key) in blocker_keys.iter().enumerate() {
        if index != 0 {
            expected_detail.push_str(", ");
        }
        expected_detail.push_str(&table.label(key));
        expected_detail.push_str(" [");
        expected_detail.push_str(&table.name(key));
        expected_detail.push(']');
    }
    let mut detail = String::new();
    detail
        .try_reserve_exact(table.blocking_detail_bytes_bound())
        .expect("diagnostic reservation fits the real content");
    table
        .write_blocking_detail_prefix(&mut detail)
        .expect("writing to String cannot fail");
    for (index, &key) in blocker_keys.iter().enumerate() {
        if index != 0 {
            detail.push_str(", ");
        }
        table
            .write_blocking_detail_item(key, &mut detail)
            .expect("writing to String cannot fail");
    }
    assert_eq!(detail, expected_detail);
    assert!(detail.len() <= table.blocking_detail_bytes_bound());

    let quest_from = from;
    let quest_to = west_bank;
    let quest_state = WorldState::empty().with_map_members(true);
    let quest_options = FindOptions {
        allow_wilderness: true,
        zones: ZoneExempt::all(),
        ..FindOptions::default()
    };
    let old_gates = crate::router::find_unresolved_quest_gates(
        &world.collision,
        &world.graph,
        quest_from,
        &[quest_to],
        quest_options,
        &quest_state,
        &[],
    );
    assert_eq!(
        old_gates,
        Ok(None),
        "reference 289 pack has no quest gates; update this check if a later pack adds gates"
    );
    let new_gates = advance_real_route_completion(
        crate::router::SearchRequest::UnresolvedQuestGates {
            from: quest_from,
            targets: vec![quest_to].into_boxed_slice(),
            options: quest_options,
            avoid: Vec::new().into_boxed_slice(),
            max_settles: crate::router::NODE_BUDGET,
        },
        &lease,
        &quest_state,
    );
    let new_settled = new_gates.forward_settled();
    assert!(new_settled > 0);
    let crate::router::SearchOutcome::UnresolvedQuestGates(new_gates) = new_gates.into_parts().0
    else {
        panic!("quest-gate cursor returned the wrong result shape")
    };
    assert_eq!(
        new_gates,
        Ok(None),
        "the cooperative path must preserve the reference pack's no-gate result"
    );
    let old_bytes = format!("{old_gates:#?}");
    let new_bytes = format!("{new_gates:#?}");
    assert_eq!(
        old_bytes.as_bytes(),
        new_bytes.as_bytes(),
        "real quest-gate diagnosis must serialize byte-for-byte"
    );
}

#[test]
fn partition_action_trace_preserves_full_i32_coordinates_and_all_step_kinds() {
    let origin = tile(i32::MIN, i32::MAX - 1, i32::MIN);
    let after_walk = tile(i32::MIN, i32::MAX, i32::MIN);
    let after_relative = tile(i32::MIN + 1, i32::MAX - 1, -1);
    let after_teleport = tile(i32::MAX - 1, i32::MIN, i32::MAX);
    let after_east = tile(i32::MAX, i32::MIN, i32::MAX);
    let destination = tile(i32::MIN + 1, i32::MIN + 1, i32::MIN + 1);
    let mut graph = door(after_walk, after_relative, 7);
    graph.edges[0].player_delta = Some(tile(1, -1, i32::MAX));
    let mut teleport = graph.edges[0].clone();
    teleport.player_delta = None;
    teleport.to = after_teleport;
    graph.teleports.push(teleport);
    let trace = super::RouteTrace {
        origin,
        destination,
        ticks: 25.5,
        steps: vec![0, 1 << 30, 2 << 30, 2, 3 << 30],
        essence: Some(crate::essence::EssenceSession {
            wizard_npc: 553,
            return_tile: destination,
        }),
    };
    let route = super::drive_to_completion(trace.materialize(&graph))
        .expect("lossless action trace materializes");
    assert_eq!(
        &*route.walk_tiles,
        &[
            origin,
            after_walk,
            after_relative,
            after_teleport,
            after_east,
            destination
        ]
    );
    assert_eq!(&*route.edge_ids, &[0, 0, 0]);
    assert_eq!(route.destination(), destination);
    assert_eq!(route.ticks(), 25.5);
    assert_eq!(route.leg_count(), 7);
    let landings: Vec<_> = route
        .cursor(&graph)
        .filter_map(|leg| match leg {
            super::CompactRouteLeg::Transport(step) => Some(step.actual_to()),
            super::CompactRouteLeg::Walk(_) => None,
        })
        .collect();
    assert_eq!(landings, [after_relative, after_teleport, destination]);
}

#[test]
fn cooperative_many_retains_the_cheaper_earlier_zone_partition() {
    let collision = bake(3, 4, &[]);
    let mut graph = TransportGraph::default();
    install_zones(
        &collision,
        &mut graph,
        vec![rect_zone(0, 0, 0, 0), rect_zone(2, 2, 3, 3)],
    );
    let from = tile(1, 0, 0);
    let targets = [tile(0, 0, 0), tile(2, 3, 0)];
    let state = WorldState::empty();
    let options = FindOptions::default();
    let old = find_many_with(&collision, &graph, from, &targets, options, &state);
    let (costs, best, _) = super::drive_to_completion(super::many_cursor_search(
        &collision,
        &graph,
        from,
        &targets,
        options,
        &state,
        &[],
        64,
        true,
        None,
    ))
    .expect("cooperative partition search");
    assert_eq!(costs, old.results());
    let (index, route) = best.expect("a reachable partition wins");
    assert_eq!(index, 0);
    assert_eq!(
        route.to_route(&graph),
        old.route(0).expect("legacy cheapest route")
    );
}

#[test]
fn real_289_full_budget_phase_reservations_fit_the_whole_job_limit() {
    let Some(world) = crate::world::NavWorld::load_default_pack_or_skip() else {
        return;
    };
    let world = Arc::new(world);
    let lease = crate::producer::lease(&world).expect("real pack lease");
    let state = Arc::new(WorldState::empty());
    for allow_teleports in [false, true] {
        let options = FindOptions {
            allow_teleports,
            ..FindOptions::default()
        };
        let shapes = [
            super::SearchRequestShape::Single {
                options,
                avoid_capacity: 4,
                max_settles: super::NODE_BUDGET,
            },
            super::SearchRequestShape::Many {
                target_capacity: 256,
                options,
                avoid_capacity: 4,
                max_settles: super::NODE_BUDGET,
                reconstruct_best: true,
            },
            super::SearchRequestShape::First {
                target_capacity: 256,
                fallback_capacity: 256,
                options,
                avoid_capacity: 4,
                max_settles: super::NODE_BUDGET,
            },
            super::SearchRequestShape::MissingItemReqs {
                target_capacity: 256,
                fallback_capacity: 256,
                options,
                avoid_capacity: 4,
                max_settles: super::NODE_BUDGET,
            },
            super::SearchRequestShape::BlockingZones {
                target_capacity: 256,
                diagnosis: crate::router::BlockerDiagnosis::FirstGoal,
                options,
                avoid_capacity: 4,
                zone_filter_bytes: ZoneFilterOwned::worst_case_bytes(
                    world
                        .graph
                        .zones
                        .as_ref()
                        .expect("real zones")
                        .zones()
                        .len(),
                )
                .expect("zone filter bound"),
                max_settles: super::NODE_BUDGET,
            },
            super::SearchRequestShape::UnresolvedQuestGates {
                target_capacity: 256,
                options,
                avoid_capacity: 4,
                max_settles: super::NODE_BUDGET,
            },
        ];
        for shape in shapes {
            let preflight = super::SearchCursor::preflight(shape, &lease, &state)
                .expect("full-budget real-content preflight");
            let reservation = preflight.reservation();
            let total = preflight.input_bytes()
                + reservation.future_layout_bytes
                + reservation.peak_bytes
                + state
                    .try_shared_retained_bytes()
                    .expect("owned state charge");
            assert!(reservation.result_bytes <= reservation.peak_bytes);
            assert!(
                total <= 192 * 1024 * 1024,
                "{shape:?} needs {total} simultaneously retained bytes"
            );
        }
    }
}

#[test]
fn real_289_radius_goal_sets_fit_the_full_budget_reservation() {
    let Some(world) = crate::world::NavWorld::load_default_pack_or_skip() else {
        return;
    };
    let world = Arc::new(world);
    let lease = crate::producer::lease(&world).expect("real pack lease");
    let state = Arc::new(WorldState::empty());
    for radius in [40usize, 104] {
        let candidates = (2 * radius + 1).pow(2);
        for allow_teleports in [false, true] {
            let options = FindOptions {
                allow_teleports,
                ..FindOptions::default()
            };
            for shape in [
                super::SearchRequestShape::First {
                    target_capacity: candidates,
                    fallback_capacity: candidates,
                    options,
                    avoid_capacity: 0,
                    max_settles: super::NODE_BUDGET,
                },
                super::SearchRequestShape::MissingItemReqs {
                    target_capacity: candidates,
                    fallback_capacity: candidates,
                    options,
                    avoid_capacity: 0,
                    max_settles: super::NODE_BUDGET,
                },
            ] {
                let preflight = super::SearchCursor::preflight(shape, &lease, &state)
                    .expect("large goal-set preflight");
                let total = preflight.input_bytes()
                    + preflight.reservation().future_layout_bytes
                    + preflight.reservation().peak_bytes
                    + state.try_shared_retained_bytes().expect("state charge");
                assert!(
                    total <= 192 * 1024 * 1024,
                    "radius {radius}, {shape:?} needs {total} bytes"
                );
            }
        }
    }
}

#[path = "router/consumption_tests.rs"]
mod consumption;
