use super::*;
use crate::native::HostEffect;
use crate::quester::families::tests::{local_player, with_tick};
use api::snapshot::{ChatLineView, GameSnapshot, StatView};

// ---------------------------------------------------------------------------
// A literal port of the content's timer, written from its own coordinates and
// offsets (`upass_grid.rs2:61-83`, `quest_upass.constant:36-38`), so the model
// above is checked against the content and not against itself.

type Tile = (i32, i32);

const COL1: Tile = (2475, 9673);
const COL3: Tile = (2471, 9673);
const COL5: Tile = (2467, 9673);

fn move_by(at: Tile, dx: i32, dz: i32) -> Tile {
    (at.0 + dx, at.1 + dz)
}

fn inzone(south_west: Tile, north_east: Tile, at: Tile) -> bool {
    (south_west.0..=north_east.0).contains(&at.0) && (south_west.1..=north_east.1).contains(&at.1)
}

/// Whether `[timer,upass_grid_traps]` calls `@upass_fail_grid` for `route`
/// (the three digits) at tile `at`.
fn content_traps(route: [u8; 3], at: Tile) -> bool {
    let [first, second, third] = route.map(i32::from);
    let first_two = |col: Tile, digit: i32| {
        (1..=5).any(|k| {
            digit == k
                && !inzone(
                    move_by(col, -2, 2 * (k - 1)),
                    move_by(col, 3, 2 * k - 1),
                    at,
                )
        })
    };
    let third_col = |digit: i32| {
        (1..=5).any(|k| {
            digit == k
                && !inzone(
                    move_by(COL5, 0, 2 * (k - 1)),
                    move_by(COL5, 3, 2 * k - 1),
                    at,
                )
        })
    };
    inzone(COL5, move_by(COL1, 1, 9), at)
        && first_two(COL1, first)
        && first_two(COL3, second)
        && third_col(third)
}

/// Every route `setupassgrilltrap` can store (`upass_grid.rs2:51-57`): a first
/// digit 1 to 5, then each digit within one of the last, kept in 1 to 5.
fn routes() -> Vec<[u8; 3]> {
    let mut all = Vec::new();
    for first in 1..=5u8 {
        for second in first.saturating_sub(1).max(1)..=(first + 1).min(5) {
            for third in second.saturating_sub(1).max(1)..=(second + 1).min(5) {
                all.push([first, second, third]);
            }
        }
    }
    all
}

fn known(route: [u8; 3]) -> GridMemory {
    let mut memory = GridMemory::default();
    for (col, lane) in route.into_iter().enumerate() {
        memory.confirm(Probe {
            col: col as u8,
            lane,
        });
    }
    memory
}

fn window_tiles() -> impl Iterator<Item = Tile> {
    (WIN_X.0..=WIN_X.1).flat_map(|x| (WIN_Z.0..=WIN_Z.1).map(move |z| (x, z)))
}

/// Each tile of a straight segment after `from`, up to and including `to`.
fn segment(from: Tile, to: Tile) -> Vec<Tile> {
    assert!(
        from.0 == to.0 || from.1 == to.1,
        "segment {from:?} -> {to:?} is not straight"
    );
    let step = ((to.0 - from.0).signum(), (to.1 - from.1).signum());
    let mut at = from;
    let mut tiles = Vec::new();
    while at != to {
        at = (at.0 + step.0, at.1 + step.1);
        tiles.push(at);
    }
    tiles
}

#[test]
fn route_count_matches_the_content_generator() {
    // 5 first digits; the others take 2 (at an edge) or 3 choices.
    assert_eq!(routes().len(), 35);
    assert!(routes()
        .iter()
        .all(|route| route.windows(2).all(|pair| pair[0].abs_diff(pair[1]) <= 1)));
}

#[test]
fn proven_tiles_are_exactly_the_tiles_the_content_does_not_trap() {
    for route in routes() {
        let memory = known(route);
        for (x, z) in window_tiles() {
            assert_eq!(
                memory.proves_safe(x, z),
                !content_traps(route, (x, z)),
                "route {route:?} at {x},{z}"
            );
        }
    }
}

#[test]
fn partial_knowledge_never_proves_a_tile_the_content_traps() {
    for route in routes() {
        for known_columns in 0..3 {
            let mut memory = GridMemory::default();
            for (col, lane) in route.iter().enumerate().take(known_columns) {
                memory.confirm(Probe {
                    col: col as u8,
                    lane: *lane,
                });
            }
            for (x, z) in window_tiles() {
                if memory.proves_safe(x, z) {
                    assert!(
                        !content_traps(route, (x, z)),
                        "route {route:?}, {known_columns} columns known, proves a trap tile {x},{z}"
                    );
                }
            }
        }
    }
}

#[test]
fn the_probe_tile_is_covered_by_one_block_and_the_launch_tile_by_two() {
    let covering = |x: i32| {
        BLOCK_X
            .iter()
            .filter(|(lo, hi)| (*lo..=*hi).contains(&x))
            .count()
    };
    for x in PROBE_X {
        assert_eq!(covering(x), 1, "probe x {x}");
    }
    assert_eq!(
        covering(LAUNCH_X[0]),
        1,
        "column 1 launches east of the area"
    );
    assert!(LAUNCH_X[0] > AREA_X.1);
    assert_eq!(covering(LAUNCH_X[1]), 2);
    assert_eq!(covering(LAUNCH_X[2]), 2);
    // The entrance the climb returns to is outside the timer's area.
    assert!(ENTRY.x > AREA_X.1);
    assert_eq!(lane_of(ENTRY.z), 3);
}

#[test]
fn safe_routes_are_straight_and_stay_on_proven_tiles() {
    for route in routes() {
        let memory = known(route);
        let goal = (WIN_X.0, 9677);
        for from in window_tiles().filter(|(x, z)| memory.proves_safe(*x, *z)) {
            let path = safe_route(&memory, from, goal)
                .unwrap_or_else(|| panic!("route {route:?}: no way from {from:?}"));
            let mut at = from;
            for corner in path.corners() {
                let next = (i32::from(corner.0), i32::from(corner.1));
                for tile in segment(at, next) {
                    assert!(
                        !content_traps(route, tile),
                        "route {route:?}: {from:?} -> {goal:?} crosses the trap tile {tile:?}"
                    );
                }
                at = next;
            }
            assert_eq!(at, goal, "route {route:?} from {from:?}");
        }
    }
}

#[test]
fn an_unproven_goal_has_no_safe_route() {
    let memory = known([3, 3, 3]);
    assert!(safe_route(&memory, (2477, 9677), (2472, 9673)).is_none());
    assert!(safe_route(&GridMemory::default(), (2477, 9677), (2476, 9677)).is_none());
}

// ---------------------------------------------------------------------------
// The exploration: every route, with the content's timer as the oracle.

struct Crossing {
    traps: u8,
    /// Lanes probed, in order, as `(column, lane, trapped)`.
    tried: Vec<(u8, u8, bool)>,
}

/// Cross the grid once with `memory`, the way the step walks it: proven tiles
/// to the launch tile, then the unproven tiles of the probe; the content's
/// timer decides a trap, and a trap returns the player to the entrance.
fn cross(route: [u8; 3], memory: &mut GridMemory) -> Crossing {
    let mut at = (ENTRY.x, ENTRY.z);
    let mut crossing = Crossing {
        traps: 0,
        tried: Vec::new(),
    };
    for _ in 0..64 {
        match memory.next() {
            Next::Inconsistent => panic!("route {route:?}: every allowed lane trapped"),
            Next::Known => {
                let goal = (WIN_X.0, 9677);
                let path = safe_route(memory, at, goal).expect("a known route is walkable");
                for corner in path.corners() {
                    let next = (i32::from(corner.0), i32::from(corner.1));
                    for tile in segment(at, next) {
                        assert!(!content_traps(route, tile), "{route:?} traps at {tile:?}");
                    }
                    at = next;
                }
                assert_eq!(at, goal);
                return crossing;
            }
            Next::Probe(probe) => {
                let launch = launch_tile(memory, probe);
                let path = safe_route(memory, at, launch)
                    .unwrap_or_else(|| panic!("route {route:?}: no way from {at:?} to {launch:?}"));
                for corner in path.corners() {
                    let next = (i32::from(corner.0), i32::from(corner.1));
                    for tile in segment(at, next) {
                        assert!(!content_traps(route, tile), "{route:?} traps at {tile:?}");
                    }
                    at = next;
                }
                at = launch;
                let (tiles, len) = explore_tiles(memory, probe);
                let mut trapped = false;
                for corner in &tiles[..len] {
                    let next = (i32::from(corner.0), i32::from(corner.1));
                    for tile in segment(at, next) {
                        if content_traps(route, tile) {
                            trapped = true;
                            break;
                        }
                        at = tile;
                    }
                    if trapped {
                        break;
                    }
                }
                crossing.tried.push((probe.col, probe.lane, trapped));
                if trapped {
                    memory.rule_out(probe);
                    memory.count_trap();
                    crossing.traps += 1;
                    at = (ENTRY.x, ENTRY.z);
                } else {
                    memory.confirm(probe);
                }
            }
        }
    }
    panic!("route {route:?}: no crossing within 64 probes");
}

#[test]
fn every_route_is_crossed_within_the_content_trap_bound() {
    let mut worst = 0;
    for route in routes() {
        let mut memory = GridMemory::default();
        let crossing = cross(route, &mut memory);
        assert!(
            crossing.traps <= MAX_TRAPS,
            "route {route:?} took {} traps: {:?}",
            crossing.traps,
            crossing.tried
        );
        // One trap per lane tried and ruled out, never a repeat.
        let mut seen = std::collections::HashSet::new();
        for (col, lane, _) in &crossing.tried {
            assert!(seen.insert((*col, *lane)), "{route:?} retried a lane");
        }
        assert_eq!(known_route(&memory), route);
        worst = worst.max(crossing.traps);
    }
    eprintln!("upass grid: worst case over all routes is {worst} traps (bound {MAX_TRAPS})");
    assert_eq!(MAX_TRAPS, 8);
    assert!(worst <= MAX_TRAPS);
}

fn known_route(memory: &GridMemory) -> [u8; 3] {
    memory.lane
}

#[test]
fn a_second_crossing_with_the_kept_memory_takes_no_trap() {
    for route in routes() {
        let mut memory = GridMemory::default();
        let first = cross(route, &mut memory);
        let before = memory.traps();
        let second = cross(route, &mut memory);
        assert_eq!(
            second.traps, 0,
            "route {route:?} after {} traps",
            first.traps
        );
        assert!(second.tried.is_empty(), "a known route probes nothing");
        assert_eq!(memory.traps(), before);
    }
}

#[test]
fn lane_order_prefers_the_entrance_row_then_the_previous_lane() {
    let mut memory = GridMemory::default();
    let mut order = Vec::new();
    while let Next::Probe(probe) = memory.next() {
        order.push(probe.lane);
        memory.rule_out(probe);
    }
    assert_eq!(order, [3, 2, 4, 1, 5]);
    assert_eq!(memory.next(), Next::Inconsistent);

    let mut memory = known([1, 1, 1]);
    memory.lane[2] = 0;
    memory.lane[1] = 0;
    memory.lane[0] = 4;
    let mut order = Vec::new();
    while let Next::Probe(probe) = memory.next() {
        assert_eq!(probe.col, 1);
        order.push(probe.lane);
        memory.rule_out(probe);
    }
    assert_eq!(order, [4, 3, 5], "previous lane, then its neighbours");
    memory.lane[0] = 1;
    memory.ruled_out = [0; 3];
    let mut order = Vec::new();
    while let Next::Probe(probe) = memory.next() {
        order.push(probe.lane);
        memory.rule_out(probe);
    }
    assert_eq!(order, [1, 2], "lane 0 does not exist");
}

#[test]
fn a_stale_route_clears_but_keeps_the_trap_count() {
    let mut memory = known([2, 3, 3]);
    memory.rule_out(Probe { col: 0, lane: 5 });
    memory.count_trap();
    memory.clear_route();
    assert_eq!(memory.lane, [0; 3]);
    assert_eq!(memory.ruled_out, [0; 3]);
    assert_eq!(memory.traps(), 1);
}

#[test]
fn the_next_trap_must_not_be_able_to_kill() {
    assert!(!survives_trap(0));
    assert!(!survives_trap(15), "15 Hitpoints die to the 15 damage fall");
    assert!(survives_trap(16));
    assert!(survives_trap(99));
}

#[test]
fn per_bot_memory_stays_a_few_bytes() {
    // The route in memory: three digits, three lane masks, one trap count.
    assert_eq!(std::mem::size_of::<GridMemory>(), 7);
    assert_eq!(std::mem::size_of::<Option<GridMemory>>(), 8);
    // The step's own state lives only while the step runs.
    let run = std::mem::size_of::<Run>();
    eprintln!(
        "upass grid Run={run} bytes, Route={}",
        std::mem::size_of::<Route>()
    );
    // RUN_MEASURED B measured, 64 B headroom (the default for a struct size):
    // the retained `PrereqWalk` (as in the `Make` family's 1176 B run) lives
    // once per running step, and the saved-route state (the memory, the run's
    // copy, the read ticket, the write in flight, its expected version and
    // stamp, and the held end of the step) added to the 392 B it had.
    assert!(run <= RUN_MEASURED + 64, "Run grew to {run} bytes");
    // The saved route's read and write state: STORE_MEASURED B measured,
    // 64 B headroom.
    let store = std::mem::size_of::<RouteStore>();
    eprintln!("upass grid RouteStore={store} bytes");
    assert!(
        store <= STORE_MEASURED + 64,
        "RouteStore grew to {store} bytes"
    );
    assert!(std::mem::size_of::<Route>() <= 56);
}

/// Bytes measured on this build (`size_of`, aarch64; no pointer-width field).
const RUN_MEASURED: usize = 512;
const STORE_MEASURED: usize = 80;
const RETAINED_MEASURED: usize = 64;

#[test]
fn the_run_keeps_the_route_in_eight_bytes_of_the_slots_retained_cell() {
    // The run's copy of the route (`QuesterRetained::upass_grid`): one box
    // pointer in the slot's cell, allocated once by the first step of a run
    // that reads its route. The box holds the memory and the saved-route state
    // that must outlive a step the runner builds again (the write in flight,
    // the expected version, the observed stamp and the conflict stop). The cell
    // is RETAINED_MEASURED B; the ceiling is that plus 64 B (the default
    // headroom for a struct size). The Gatherer exclusive-heap caps hold the
    // measured cell (`gatherer_exclusive_heap_stays_inside_the_measured_native_bound`):
    // a slot that never crosses the grid allocates nothing for it.
    assert_eq!(std::mem::size_of::<GridRun>(), 8);
    let boxed = std::mem::size_of::<RunRoute>();
    eprintln!("upass grid RunRoute={boxed} bytes (boxed once per run)");
    assert!(boxed <= STORE_MEASURED + 8 + 64, "RunRoute grew to {boxed} bytes");
    let cell = std::mem::size_of::<crate::quester::QuesterRetained>();
    eprintln!("QuesterRetained={cell} bytes");
    assert!(
        cell <= RETAINED_MEASURED + 64,
        "QuesterRetained grew to {cell} bytes"
    );
}

// ---------------------------------------------------------------------------
// The machine, one observed tick at a time.

fn with_step<R>(
    tick: &mut crate::native::NativeTick<'_>,
    f: impl FnOnce(&mut StepContext<'_, '_>) -> R,
) -> R {
    let quests = api::quest_facts::QuestCatalog::empty();
    let banks = Arc::new(api::named_banks::NamedBankFacts::empty());
    let choices = crate::quester::choices::QuestChoices::default();
    let required_after = tick.cx.evidence();
    f(&mut StepContext {
        tick,
        quests: &quests,
        progress: &[],
        required_after,
        banks: &banks,
        choices: &choices,
    })
}

fn world(x: i32, z: i32) -> WorldTile {
    WorldTile { x, z, level: 0 }
}

fn snapshot(at: WorldTile, hitpoints: i32, trap_line_seq: Option<i32>) -> GameSnapshot {
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    snapshot.seed_local_player(local_player(at));
    snapshot.seed_stats(vec![StatView {
        index: HITPOINTS,
        name: "hitpoints".into(),
        effective: hitpoints,
        base: 40,
        xp: 0,
        used: true,
    }]);
    snapshot.seed_chat_lines(
        trap_line_seq
            .map(|sequence| ChatLineView {
                type_: 0,
                username: None,
                text: TRAP_MESSAGE.into(),
                sequence,
            })
            .into_iter()
            .collect(),
    );
    snapshot
}

fn run_at(arrival: WorldTile) -> Run {
    Run {
        arrival,
        hand_holds_id: 3365,
        phase: Phase::Decide,
        hop: Hop::default(),
        click: None,
        chat_mark: 0,
        stats_wait: 0,
        memory: GridMemory::default(),
        store: RouteStore::memory_only(),
        hold: None,
    }
}

const FAR_SIDE: WorldTile = WorldTile {
    x: 2466,
    z: 9677,
    level: 0,
};

/// Poll once at `tick`, with `memory` as the run's route before and the poll's
/// result written back after. The run learns for this run only here; the saved
/// route has its own tests below.
fn poll(
    run: &mut Run,
    snapshot: &GameSnapshot,
    ledger: &mut Option<Box<crate::native::ledger::Ledger>>,
    memory: &mut GridMemory,
    tick: u64,
) -> Poll<Result<StepOutcome, ActionError>> {
    run.memory = *memory;
    let result = with_tick(snapshot, ledger, tick, |native| {
        with_step(native, |cx| run.poll(cx))
    });
    *memory = run.memory;
    result
}

fn last_walk(ledger: &Option<Box<crate::native::ledger::Ledger>>) -> Option<(WorldTile, u16)> {
    ledger
        .as_ref()?
        .outbox
        .iter()
        .rev()
        .find_map(|action| match &action.effect {
            HostEffect::Walk(request) => Some((request.target, request.radius)),
            _ => None,
        })
}

#[test]
fn the_step_stops_when_the_next_trap_could_kill() {
    if !api::test_support::game_data_available() {
        return;
    }
    // Entrance tile, nothing known: the first probe is lane 3 at 2476,9677.
    let at = world(ENTRY.x, ENTRY.z);
    let mut ledger = None;
    let mut memory = GridMemory::default();
    let mut run = run_at(FAR_SIDE);
    let low = snapshot(at, 15, None);
    assert!(poll(&mut run, &low, &mut ledger, &mut memory, 1).is_pending());
    let stopped = poll(&mut run, &low, &mut ledger, &mut memory, 2);
    let Poll::Ready(Err(ActionError::Failed(detail))) = stopped else {
        panic!("15 Hitpoints must stop the step with a failure");
    };
    assert!(
        detail.contains("15 Hitpoints would not survive"),
        "{detail}"
    );
    assert!(last_walk(&ledger).is_none(), "nothing may be sent first");
    assert_eq!(memory.traps(), 0);
}

#[test]
fn the_step_walks_the_probe_when_hitpoints_allow_it() {
    if !api::test_support::game_data_available() {
        return;
    }
    let at = world(ENTRY.x, ENTRY.z);
    let mut ledger = None;
    let mut memory = GridMemory::default();
    let mut run = run_at(FAR_SIDE);
    let healthy = snapshot(at, 16, None);
    for tick in 1..=3 {
        assert!(poll(&mut run, &healthy, &mut ledger, &mut memory, tick).is_pending());
    }
    assert_eq!(
        last_walk(&ledger),
        Some((world(2476, 9677), 0)),
        "lane 3, column 1: the exact probe tile"
    );
    assert!(matches!(run.phase, Phase::Explore { .. }));
}

#[test]
fn hitpoints_that_never_show_stop_the_step_after_the_bound() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut blind = GameSnapshot::new();
    blind.seed_ingame(2);
    blind.seed_local_player(local_player(world(ENTRY.x, ENTRY.z)));
    let mut ledger = None;
    let mut memory = GridMemory::default();
    let mut run = run_at(FAR_SIDE);
    let mut result = Poll::Pending;
    for tick in 1..=u64::from(STATS_TICKS) + 4 {
        result = poll(&mut run, &blind, &mut ledger, &mut memory, tick);
        if result.is_ready() {
            break;
        }
    }
    assert!(matches!(result, Poll::Ready(Err(ActionError::Failed(_)))));
    assert!(last_walk(&ledger).is_none());
}

#[test]
fn a_trap_line_on_a_probe_rules_that_lane_out_and_counts_it() {
    if !api::test_support::game_data_available() {
        return;
    }
    let probe = Probe { col: 0, lane: 3 };
    let mut run = run_at(FAR_SIDE);
    run.chat_mark = 4;
    run.phase = Phase::Observe { probe, until: 99 };
    let mut ledger = None;
    let mut memory = GridMemory::default();
    // A line older than the mark does not count.
    let old = snapshot(world(2476, 9677), 40, Some(4));
    assert!(poll(&mut run, &old, &mut ledger, &mut memory, 10).is_pending());
    assert!(matches!(run.phase, Phase::Observe { .. }));
    assert_eq!(memory.traps(), 0);
    // A newer one does.
    let trapped = snapshot(world(2476, 9677), 40, Some(5));
    assert!(poll(&mut run, &trapped, &mut ledger, &mut memory, 11).is_pending());
    assert!(matches!(run.phase, Phase::Fallen { deadline: 15 }));
    assert_eq!(memory.traps(), 1);
    assert_eq!(memory.ruled_out, [0b00100, 0, 0], "lane 3 is ruled out");
    assert_eq!(memory.next(), Next::Probe(Probe { col: 0, lane: 2 }));
    // The pit teleport moves the step on to the climb.
    let pit = snapshot(world(2396, 9560), 25, None);
    assert!(poll(&mut run, &pit, &mut ledger, &mut memory, 12).is_pending());
    assert!(matches!(
        run.phase,
        Phase::Climb {
            pit_seen: 12,
            attempts: 0,
            click: Click::Ready
        }
    ));
}

#[test]
fn a_trap_without_its_line_is_seen_from_the_pit_position() {
    if !api::test_support::game_data_available() {
        return;
    }
    let probe = Probe { col: 0, lane: 1 };
    let mut run = run_at(FAR_SIDE);
    run.phase = Phase::Explore {
        tiles: [pt(2476, 9673), (0, 0)],
        len: 1,
        next: 0,
        probe: Some(probe),
    };
    let mut ledger = None;
    let mut memory = GridMemory::default();
    let pit = snapshot(world(2395, 9559), 25, None);
    assert!(poll(&mut run, &pit, &mut ledger, &mut memory, 10).is_pending());
    assert!(matches!(run.phase, Phase::Climb { .. }));
    assert_eq!(memory.ruled_out, [0b00001, 0, 0]);
    assert_eq!(memory.traps(), 1);
}

#[test]
fn a_trap_on_a_proven_tile_clears_the_stale_route_and_still_counts() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut run = run_at(FAR_SIDE);
    let mut spine = Route::empty();
    spine.push(pt(2474, 9677));
    run.phase = Phase::Spine {
        route: spine,
        next: 0,
        last_radius: 0,
        after: After::Decide,
    };
    let mut ledger = None;
    let mut memory = known([3, 3, 3]);
    let trapped = snapshot(world(2476, 9677), 40, Some(1));
    assert!(poll(&mut run, &trapped, &mut ledger, &mut memory, 10).is_pending());
    assert_eq!(memory.lane, [0; 3], "the learned route no longer holds");
    assert_eq!(memory.traps(), 1);
    assert!(matches!(run.phase, Phase::Fallen { .. }));
}

#[test]
fn a_quiet_probe_confirms_its_lane_after_the_observed_ticks() {
    if !api::test_support::game_data_available() {
        return;
    }
    let probe = Probe { col: 0, lane: 3 };
    let mut run = run_at(FAR_SIDE);
    run.phase = Phase::Observe {
        probe,
        until: 10 + OBSERVE_TICKS,
    };
    let mut ledger = None;
    let mut memory = GridMemory::default();
    let quiet = snapshot(world(2476, 9677), 40, None);
    for tick in 10..10 + OBSERVE_TICKS {
        assert!(poll(&mut run, &quiet, &mut ledger, &mut memory, tick).is_pending());
        assert_eq!(
            memory.lane, [0; 3],
            "tick {tick}: still waiting for the timer"
        );
    }
    assert!(poll(
        &mut run,
        &quiet,
        &mut ledger,
        &mut memory,
        10 + OBSERVE_TICKS
    )
    .is_pending());
    assert_eq!(memory.lane, [3, 0, 0]);
    assert!(matches!(run.phase, Phase::Decide));
    assert_eq!(memory.next(), Next::Probe(Probe { col: 1, lane: 3 }));
}

#[test]
fn a_probe_that_ended_off_its_tile_is_not_confirmed() {
    if !api::test_support::game_data_available() {
        return;
    }
    let probe = Probe { col: 0, lane: 3 };
    let mut run = run_at(FAR_SIDE);
    run.phase = Phase::Observe { probe, until: 10 };
    let mut ledger = None;
    let mut memory = GridMemory::default();
    let elsewhere = snapshot(world(2477, 9677), 40, None);
    assert!(poll(&mut run, &elsewhere, &mut ledger, &mut memory, 10).is_pending());
    assert_eq!(memory.lane, [0; 3]);
    assert!(matches!(run.phase, Phase::Decide));
}

#[test]
fn a_fall_without_a_pit_teleport_fails_after_its_bound() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut run = run_at(FAR_SIDE);
    run.phase = Phase::Fallen { deadline: 14 };
    let mut ledger = None;
    let mut memory = GridMemory::default();
    let stuck = snapshot(world(2476, 9677), 40, None);
    assert!(poll(&mut run, &stuck, &mut ledger, &mut memory, 13).is_pending());
    let result = poll(&mut run, &stuck, &mut ledger, &mut memory, 14);
    assert!(matches!(result, Poll::Ready(Err(ActionError::Failed(_)))));
}

#[test]
fn the_trap_budget_stops_the_step_before_another_probe() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut run = run_at(FAR_SIDE);
    let mut ledger = None;
    let mut memory = GridMemory::default();
    for _ in 0..MAX_TRAPS {
        memory.count_trap();
    }
    let healthy = snapshot(world(ENTRY.x, ENTRY.z), 40, None);
    assert!(poll(&mut run, &healthy, &mut ledger, &mut memory, 1).is_pending());
    let result = poll(&mut run, &healthy, &mut ledger, &mut memory, 2);
    let Poll::Ready(Err(ActionError::Failed(detail))) = result else {
        panic!("the trap budget must stop the step with a failure");
    };
    assert!(detail.contains("traps taken"), "{detail}");
    assert!(last_walk(&ledger).is_none());
}

#[test]
fn the_step_ends_when_the_player_is_on_the_far_side() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut run = run_at(FAR_SIDE);
    let mut ledger = None;
    let mut memory = known([3, 3, 3]);
    let far = snapshot(world(2466, 9677), 40, None);
    assert!(matches!(
        poll(&mut run, &far, &mut ledger, &mut memory, 1),
        Poll::Ready(Ok(_))
    ));
}

#[test]
fn a_tile_inside_the_timer_area_next_to_the_far_side_does_not_end_the_step() {
    if !api::test_support::game_data_available() {
        return;
    }
    // 2467,9677 is within one tile of the far side but still inside the
    // timer's area: it is only safe while lane 3 is column 3's lane.
    let mut run = run_at(FAR_SIDE);
    let mut ledger = None;
    let mut memory = known([3, 3, 3]);
    let inside = snapshot(world(2467, 9677), 40, None);
    assert!(poll(&mut run, &inside, &mut ledger, &mut memory, 1).is_pending());
    for tick in 2..=3 {
        assert!(poll(&mut run, &inside, &mut ledger, &mut memory, tick).is_pending());
    }
    assert_eq!(
        last_walk(&ledger),
        Some((FAR_SIDE, 0)),
        "out of the area, exactly"
    );
}

#[test]
fn a_known_route_walks_straight_lanes_to_the_far_side_without_exploring() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut run = run_at(FAR_SIDE);
    let mut ledger = None;
    let mut memory = known([3, 3, 3]);
    let at_entry = snapshot(world(ENTRY.x, ENTRY.z), 16, None);
    for tick in 1..=2 {
        assert!(poll(&mut run, &at_entry, &mut ledger, &mut memory, tick).is_pending());
    }
    // Lane 3 is straight: one segment to the far side tile, exactly.
    assert_eq!(last_walk(&ledger), Some((world(2466, 9677), 0)));
    assert_eq!(memory.traps(), 0);
}

fn climb_clicks(
    ledger: &Option<Box<crate::native::ledger::Ledger>>,
) -> Vec<(i32, i32, String, Option<i32>)> {
    ledger
        .as_ref()
        .map(|ledger| {
            ledger
                .outbox
                .iter()
                .filter_map(|action| match &action.effect {
                    HostEffect::Interaction(InteractReq::Loc {
                        x, z, action, id, ..
                    }) => Some((*x, *z, action.clone(), *id)),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn the_climb_waits_out_the_fall_then_clicks_the_hand_hold_under_the_player() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut run = run_at(FAR_SIDE);
    run.phase = Phase::climb(10);
    let mut ledger = None;
    let mut memory = GridMemory::default();
    let pit = snapshot(world(2396, 9560), 25, None);
    for tick in 10..10 + POST_FALL_TICKS {
        assert!(poll(&mut run, &pit, &mut ledger, &mut memory, tick).is_pending());
        assert!(
            climb_clicks(&ledger).is_empty(),
            "tick {tick}: the fall still runs"
        );
    }
    // The tick after the wait begins the click's machine; the next one sends.
    assert!(poll(&mut run, &pit, &mut ledger, &mut memory, 12).is_pending());
    assert!(poll(&mut run, &pit, &mut ledger, &mut memory, 13).is_pending());
    assert_eq!(
        climb_clicks(&ledger),
        [(2396, 9560, "Climb".to_owned(), Some(3365))],
        "one Climb on the loc under the player"
    );
    // Still queued: no second click.
    for tick in 14..20 {
        assert!(poll(&mut run, &pit, &mut ledger, &mut memory, tick).is_pending());
    }
    assert_eq!(climb_clicks(&ledger).len(), 1);
}

#[test]
fn a_climb_that_leaves_the_pit_ends_the_wait() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut run = run_at(FAR_SIDE);
    run.phase = Phase::climb(10);
    let mut ledger = None;
    let mut memory = GridMemory::default();
    let pit = snapshot(world(2395, 9559), 25, None);
    for tick in 10..14 {
        assert!(poll(&mut run, &pit, &mut ledger, &mut memory, tick).is_pending());
    }
    let entrance = snapshot(world(ENTRY.x, ENTRY.z), 10, None);
    assert!(poll(&mut run, &entrance, &mut ledger, &mut memory, 15).is_pending());
    assert!(matches!(run.phase, Phase::Decide));
    assert_eq!(climb_clicks(&ledger).len(), 1);
}

#[test]
fn the_climb_repeats_after_its_wait_and_stops_at_the_attempt_bound() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut run = run_at(FAR_SIDE);
    run.phase = Phase::climb(10);
    let mut ledger = None;
    let mut memory = GridMemory::default();
    let pit = snapshot(world(2396, 9559), 25, None);
    let mut result = Poll::Pending;
    let mut sent = 0;
    for tick in 10..400 {
        result = poll(&mut run, &pit, &mut ledger, &mut memory, tick);
        if result.is_ready() {
            break;
        }
        // The host takes each click and accepts it in the same pass.
        if ledger.is_some() {
            for (authority, effect) in
                crate::quester::families::tests::host_takes_queued_work(&mut ledger)
            {
                if matches!(effect, HostEffect::Interaction(InteractReq::Loc { .. })) {
                    sent += 1;
                    crate::quester::families::tests::host_accepts(&mut ledger, &authority, tick);
                }
            }
        }
    }
    let Poll::Ready(Err(ActionError::Failed(detail))) = result else {
        panic!("the climb must stop after its attempts");
    };
    assert!(detail.contains("climbs did not leave the pit"), "{detail}");
    assert_eq!(sent, usize::from(CLIMB_ATTEMPTS));
}

// ---------------------------------------------------------------------------
// The saved route. A port that keeps knowledge facts per account the way the
// service's SQLite store does: a read is queued and answered on a later poll,
// every committed fact has the root-wide version of its write transaction, a
// write names the version it replaces (compare-and-swap) and settles on a
// later observation, and a fact is replaced by its key. The host's own SQLite
// tests drive the same store against the real service
// (`host-play` `local_state::upass_route_tests`).

use crate::local_state::{KnowledgeTicket, SlotState, StatePort, StoredKnowledge};
use crate::native::ledger::{Ledger, TickBudget};
use crate::native::{NativeOutput, RetainedMemory, ScriptStatus};
use crate::quester::families::tests::with_tick_retained_output;
use crate::quester::handlers::upass_store::{
    self, Loaded, CONFIRM_TICKS, LOAD_TICKS, MAX_WRITE_FAILURES,
};
use api::identity::{DurableTimestamp, RequestId, RequestIdMinter, Settlement};
use api::local_state::{
    AccountKey, ContentScope, DomainEdit, DomainIdentity, DomainLease, DurableReceipt, ErrorCode,
    FailureReason, KnowledgeKey, KnowledgeProvenance, KnowledgeRecord, KnowledgeRevocation,
    KnowledgeStatus, LocalStatePort, StoreVersion, SubmitReceipt, WriteSettlement,
    KNOWLEDGE_SCHEMA_VERSION,
};
use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;

/// The durable side of the fake: the committed route fact by account, and the
/// root-wide transaction sequence.
#[derive(Default)]
pub(in crate::quester) struct Files {
    pub(in crate::quester) by_account: Mutex<HashMap<String, StoredKnowledge>>,
    sequence: AtomicU64,
}

impl Files {
    /// Commit `record` as another writer would, at the next sequence.
    fn commit(&self, record: KnowledgeRecord) -> StoreVersion {
        let mut version = StoreVersion::INITIAL;
        for _ in 0..=self.sequence.fetch_add(1, Ordering::SeqCst) {
            version = version.checked_next().expect("a version is available");
        }
        let account = account_of(record.identity());
        self.by_account
            .lock()
            .unwrap()
            .insert(account, StoredKnowledge::new(Arc::new(record), version));
        version
    }

    /// The committed route of `account`.
    fn stored(&self, account: &str) -> Option<StoredKnowledge> {
        self.by_account.lock().unwrap().get(account).cloned()
    }
}

/// One service incarnation over `Files`.
pub(in crate::quester) struct FakeService {
    files: Arc<Files>,
    minter: RequestIdMinter,
    /// Read polls answered "not yet" before a read completes.
    cold: AtomicUsize,
    /// Reads refused with this code.
    pub(in crate::quester) refuse: Mutex<Option<ErrorCode>>,
    /// Submissions that fail on settlement.
    fail_writes: AtomicUsize,
    /// Observations that answer "not yet" before a write settles (a request
    /// takes this many, counted when it is submitted).
    pub(in crate::quester) slow: AtomicUsize,
    pub(in crate::quester) submitted: AtomicUsize,
    /// Reads started.
    reads: AtomicUsize,
    /// Every submitted fact, in order.
    sent: Mutex<Vec<KnowledgeRecord>>,
    pending: Mutex<HashMap<RequestId, (KnowledgeRecord, usize)>>,
}

impl FakeService {
    pub(in crate::quester) fn over(files: &Arc<Files>) -> Arc<Self> {
        Arc::new(Self {
            files: Arc::clone(files),
            minter: RequestIdMinter::new(),
            cold: AtomicUsize::new(0),
            refuse: Mutex::new(None),
            fail_writes: AtomicUsize::new(0),
            slow: AtomicUsize::new(0),
            submitted: AtomicUsize::new(0),
            reads: AtomicUsize::new(0),
            sent: Mutex::new(Vec::new()),
            pending: Mutex::new(HashMap::new()),
        })
    }

    fn sent(&self) -> Vec<KnowledgeRecord> {
        self.sent.lock().unwrap().clone()
    }
}

fn account_of(identity: &DomainIdentity) -> String {
    identity.account().to_string()
}

impl LocalStatePort for FakeService {
    fn lease(&self, identity: &DomainIdentity) -> Result<DomainLease, ErrorCode> {
        Err(ErrorCode::NotReady {
            domain: identity.domain(),
        })
    }

    fn submit(&self, edit: DomainEdit) -> SubmitReceipt {
        let DomainEdit::Knowledge(record) = edit else {
            panic!("the step writes knowledge only");
        };
        self.submitted.fetch_add(1, Ordering::SeqCst);
        self.sent.lock().unwrap().push(record.clone());
        let request = self.minter.mint().unwrap();
        let slow = self.slow.load(Ordering::SeqCst);
        self.pending.lock().unwrap().insert(request, (record, slow));
        SubmitReceipt::Accepted { request }
    }

    fn observe(&self, request: RequestId) -> Option<WriteSettlement> {
        let mut pending = self.pending.lock().unwrap();
        let (_, wait) = pending.get_mut(&request)?;
        if *wait > 0 {
            *wait -= 1;
            return None;
        }
        let (record, _) = pending.remove(&request)?;
        if self
            .fail_writes
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |left| left.checked_sub(1))
            .is_ok()
        {
            return Some(Settlement::Failed(ErrorCode::Failed {
                domain: record.identity().domain(),
                reason: FailureReason::Io,
            }));
        }
        let identity = record.identity().clone();
        let found = self
            .files
            .stored(&account_of(&identity))
            .map_or(StoreVersion::INITIAL, |stored| stored.version());
        if record.expected() != found {
            return Some(Settlement::Failed(ErrorCode::Conflict {
                expected: record.expected(),
                found,
            }));
        }
        let version = self.files.commit(record);
        Some(Settlement::Completed(DurableReceipt::new(
            identity,
            version,
            DurableTimestamp::from_unix_seconds(2),
            Some(request),
        )))
    }
}

impl StatePort for FakeService {
    fn lease_version(&self, identity: &DomainIdentity) -> Result<StoreVersion, ErrorCode> {
        Err(ErrorCode::NotReady {
            domain: identity.domain(),
        })
    }

    fn start_knowledge_read(&self, _: &DomainIdentity) -> Result<KnowledgeTicket, ErrorCode> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        match *self.refuse.lock().unwrap() {
            Some(code) => Err(code),
            None => Ok(KnowledgeTicket::new(0, 1)),
        }
    }

    fn poll_knowledge_read(
        &self,
        identity: &DomainIdentity,
        _: KnowledgeTicket,
        key: &KnowledgeKey,
    ) -> Option<Result<Option<StoredKnowledge>, ErrorCode>> {
        assert_eq!(key.as_str(), upass_store::KEY);
        if self
            .cold
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |left| left.checked_sub(1))
            .is_ok()
        {
            return None;
        }
        Some(Ok(self.files.stored(&account_of(identity))))
    }
}

/// A slot a tick can borrow (`NativeTick::state` is tied to the tick's own
/// lifetime, which a closure's captured state cannot satisfy); leaked, in a test.
pub(in crate::quester) fn leaked_slot(
    service: &Arc<FakeService>,
    account: &str,
) -> &'static SlotState {
    Box::leak(Box::new(slot_for(service, account)))
}

fn slot_for(service: &Arc<FakeService>, account: &str) -> SlotState {
    SlotState::new(
        Arc::clone(service) as Arc<dyn StatePort>,
        AccountKey::try_new("local-289", account).unwrap(),
        ContentScope::try_new(api::selected::ClientRevision::R289, "test-content").unwrap(),
    )
}

/// What the run's retained cell holds of the route: the saved payload, the
/// traps taken, and whether the store may lack the route; `None` while the
/// run has not read its route.
pub(in crate::quester) fn retained_route(
    retained: &mut RetainedMemory,
) -> Option<([u8; PAYLOAD_LEN], u8, bool)> {
    retained.quester().upass_grid.kept().map(|kept| {
        (
            kept.memory.to_payload(),
            kept.memory.traps(),
            kept.store.unsaved(),
        )
    })
}

/// The run's retained route as the slot's cell would hold it after a step
/// learned `memory` for this run only.
pub(in crate::quester) fn seed_retained_route(
    retained: &mut RetainedMemory,
    payload: [u8; PAYLOAD_LEN],
    traps: u8,
) {
    let mut memory = GridMemory::from_payload(&payload).expect("a route the step writes");
    for _ in 0..traps {
        memory.count_trap();
    }
    retained.quester().upass_grid.keep(RunRoute {
        memory,
        store: RouteStore::memory_only(),
    });
}

/// A route as learned part-way: column 1 lane 3; column 2 lane 3 trapped and
/// lane 2 held; column 3 lane 2 trapped; with three traps taken.
fn learned() -> GridMemory {
    let mut memory = GridMemory::default();
    memory.confirm(Probe { col: 0, lane: 3 });
    memory.rule_out(Probe { col: 1, lane: 3 });
    memory.confirm(Probe { col: 1, lane: 2 });
    memory.rule_out(Probe { col: 2, lane: 2 });
    for _ in 0..3 {
        memory.count_trap();
    }
    memory
}

thread_local! {
    /// The observed tick the next helper call runs at (one test, one thread).
    static TICKS: Cell<u64> = const { Cell::new(1) };
}

fn next_tick() -> u64 {
    TICKS.with(|ticks| {
        let tick = ticks.get();
        ticks.set(tick + 1);
        tick
    })
}

/// Read the saved route the way a starting step does, however many ticks the
/// file takes to read.
fn load_now(store: &mut RouteStore, state: &SlotState) -> (GridMemory, Loaded) {
    for _ in 0..=LOAD_TICKS + 5 {
        if let Poll::Ready(result) = store.load(Some(state), next_tick()) {
            return result.expect("the route reads");
        }
    }
    panic!("the route did not read");
}

/// Send `memory` and observe it settle.
fn save_now(store: &mut RouteStore, state: &SlotState, memory: &GridMemory) {
    store.changed();
    for _ in 0..200 {
        store.pump(Some(state), next_tick(), memory);
        if store.settled() {
            return;
        }
    }
    panic!("the write did not settle");
}

#[test]
fn a_saved_route_has_no_trap_count_and_only_routes_content_allows() {
    let memory = learned();
    let payload = memory.to_payload();
    assert_eq!(payload, [3, 2, 0, 0, 0b00100, 0b00010]);
    let back = GridMemory::from_payload(&payload).unwrap();
    assert_eq!(back.lane, memory.lane);
    assert_eq!(back.ruled_out, memory.ruled_out);
    assert_eq!(back.traps(), 0, "the trap bound is per run");
    assert_eq!(GridMemory::from_payload(&[0; 6]), Some(GridMemory::default()));
    // The route a step writes when a lane held at its first try, and when its
    // earlier candidates trapped first.
    for good in [
        &[3, 0, 0, 0, 0, 0][..],
        &[2, 0, 0, 0b00100, 0, 0][..],
        &[3, 3, 0, 0, 0, 0][..],
        &[3, 2, 0, 0, 0b00100, 0][..],
        &[0, 0, 0, 0b00100, 0, 0][..],
        // Every lane of column 1 trapped: the stored route was stale.
        &[0, 0, 0, 0b11111, 0, 0][..],
    ] {
        assert!(GridMemory::from_payload(good).is_some(), "{good:?}");
    }
    // Each of these is a route the server cannot have drawn, a history the
    // step cannot have produced, or a damaged entry.
    for bad in [
        &[][..],
        &[3, 2, 0, 0, 0][..],
        &[3, 2, 0, 0, 0, 0, 0][..],
        &[6, 0, 0, 0, 0, 0][..],
        &[255, 0, 0, 0, 0, 0][..],
        // A gap in the confirmed columns.
        &[0, 3, 0, 0, 0, 0][..],
        // Column 2 lane 3 is not the first lane of column 1's neighbours.
        &[1, 3, 0, 0, 0, 0][..],
        // A mask past the five lanes.
        &[3, 2, 0, 0, 0b00100, 0b100000][..],
        // A confirmed lane that is also ruled out.
        &[3, 2, 0, 0b00100, 0, 0][..],
        // The reviewer's two examples (review-upass-persist.md, R3): a ruled-out
        // lane in column 2 while column 1 is not confirmed.
        &[0, 0, 0, 0, 1, 0][..],
        // A ruled-out lane 5 in column 2 after lane 1 in column 1 (its candidates
        // are lanes 1 and 2, `upass_grid.rs2:51-56`).
        &[1, 0, 0, 0, 16, 0][..],
        // A column held without the candidates before it ruled out: lane 2 in
        // column 2 means lane 3 trapped first.
        &[3, 2, 0, 0, 0, 0][..],
        // A column 1 lane 2 means lane 3 trapped first.
        &[2, 0, 0, 0, 0, 0][..],
        // Ruled out lanes that are not a leading run of the tried order.
        &[0, 0, 0, 0b00010, 0, 0][..],
        &[3, 0, 0, 0, 0b00010, 0][..],
        // A column after the next one holds nothing yet.
        &[3, 0, 0, 0, 0, 0b00001][..],
    ] {
        assert_eq!(GridMemory::from_payload(bad), None, "{bad:?}");
    }
}

/// Every payload the reader accepts is a state the step can reach, and every
/// state the step can reach is accepted: the states grow by one probe at a
/// time, each probe confirming its lane or ruling it out, from nothing known.
#[test]
fn the_reader_accepts_exactly_the_histories_the_step_can_write() {
    let mut reachable = HashSet::new();
    let mut open = vec![GridMemory::default()];
    while let Some(memory) = open.pop() {
        if !reachable.insert(memory.to_payload()) {
            continue;
        }
        if let Next::Probe(probe) = memory.next() {
            let mut held = memory;
            held.confirm(probe);
            open.push(held);
            let mut trapped = memory;
            trapped.rule_out(probe);
            open.push(trapped);
        }
    }
    let mut accepted = HashSet::new();
    for lanes in 0..216u16 {
        for masks in 0..(1u32 << 15) {
            let payload = [
                (lanes % 6) as u8,
                (lanes / 6 % 6) as u8,
                (lanes / 36) as u8,
                (masks & 31) as u8,
                (masks >> 5 & 31) as u8,
                (masks >> 10 & 31) as u8,
            ];
            if GridMemory::from_payload(&payload).is_some() {
                accepted.insert(payload);
            }
        }
    }
    assert_eq!(accepted, reachable);
    eprintln!("upass grid: {} histories", reachable.len());
}

#[test]
fn a_learned_route_survives_a_restart_and_a_relog() {
    let files = Arc::new(Files::default());
    let memory = learned();
    {
        // The process that learned it.
        let service = FakeService::over(&files);
        let state = slot_for(&service, "alice");
        let mut store = RouteStore::loading();
        let (first, loaded) = load_now(&mut store, &state);
        assert_eq!(loaded, Loaded::Absent);
        assert!(first.is_empty());
        save_now(&mut store, &state, &memory);
        assert_eq!(service.submitted.load(Ordering::SeqCst), 1);
    }
    // A new process (a new service over the same files), and the same account
    // logging in again as a new slot: both read it back, with no traps taken.
    for spelling in ["alice", "Alice"] {
        let service = FakeService::over(&files);
        let state = slot_for(&service, spelling);
        let mut store = RouteStore::loading();
        let (loaded_memory, loaded) = load_now(&mut store, &state);
        assert_eq!(loaded, Loaded::Saved(2));
        assert_eq!(loaded_memory.lane, memory.lane);
        assert_eq!(loaded_memory.ruled_out, memory.ruled_out);
        assert_eq!(loaded_memory.traps(), 0);
        assert_eq!(loaded_memory.next(), memory_next_after_restart());
    }
}

/// With columns 1 and 2 known and lane 2 ruled out of column 3, the next probe
/// is column 3 lane 1 (the previous lane's lower neighbour), not the lane that
/// trapped.
fn memory_next_after_restart() -> Next {
    Next::Probe(Probe { col: 2, lane: 1 })
}

#[test]
fn a_new_account_starts_empty() {
    let files = Arc::new(Files::default());
    let service = FakeService::over(&files);
    let alice = slot_for(&service, "alice");
    let mut store = RouteStore::loading();
    load_now(&mut store, &alice);
    save_now(&mut store, &alice, &learned());
    let bob = slot_for(&service, "bob");
    let mut fresh = RouteStore::loading();
    let (memory, loaded) = load_now(&mut fresh, &bob);
    assert_eq!(loaded, Loaded::Absent);
    assert!(memory.is_empty());
}

#[test]
fn an_entry_that_does_not_decode_starts_empty_and_the_next_write_replaces_it() {
    let files = Arc::new(Files::default());
    let service = FakeService::over(&files);
    let state = slot_for(&service, "alice");
    // A damaged payload: a lane of 9, committed by an earlier writer.
    let damaged = KnowledgeRecord::try_new(
        state.account().clone(),
        state.scope().clone(),
        StoreVersion::INITIAL,
        upass_store::KEY,
        KnowledgeProvenance::Observed,
        KnowledgeStatus::Known,
        api::local_state::validate_knowledge_payload(KNOWLEDGE_SCHEMA_VERSION, &[9, 0, 0, 0, 0, 0])
            .unwrap(),
        DurableTimestamp::from_unix_seconds(1),
    )
    .unwrap();
    files.commit(damaged);
    let mut store = RouteStore::loading();
    let (memory, loaded) = load_now(&mut store, &state);
    assert_eq!(loaded, Loaded::Corrupt);
    assert!(memory.is_empty());
    // The step logs one line for it, and learning goes on and saves.
    save_now(&mut store, &state, &learned());
    let mut again = RouteStore::loading();
    assert_eq!(load_now(&mut again, &state).1, Loaded::Saved(2));
}

#[test]
fn a_file_the_service_refuses_is_learned_without_saving() {
    let files = Arc::new(Files::default());
    let service = FakeService::over(&files);
    let refusal = ErrorCode::Failed {
        domain: api::local_state::LocalStateDomain::Knowledge,
        reason: FailureReason::Schema,
    };
    *service.refuse.lock().unwrap() = Some(refusal);
    let state = slot_for(&service, "alice");
    let mut store = RouteStore::loading();
    let (memory, loaded) = load_now(&mut store, &state);
    assert_eq!(loaded, Loaded::Refused(refusal));
    assert!(memory.is_empty());
    store.changed();
    store.pump(Some(&state), next_tick(), &learned());
    assert!(store.settled(), "nothing waits on a write that is not made");
    assert!(!store.unsaved(), "a file that cannot be written is not owed a write");
    assert_eq!(service.submitted.load(Ordering::SeqCst), 0);
}

#[test]
fn a_slot_without_a_state_capability_learns_for_the_run_only() {
    let mut store = RouteStore::loading();
    let Poll::Ready(Ok((memory, loaded))) = store.load(None, 1) else {
        panic!("no capability answers at once");
    };
    assert_eq!(loaded, Loaded::NoCapability);
    assert!(memory.is_empty());
    store.changed();
    store.pump(None, 2, &learned());
    assert!(store.settled());
    assert!(!store.unsaved());
}

/// A step the runner builds again in the same run takes the run's route and
/// saved-route state: it reads nothing, and an older route another writer
/// saved meanwhile does not replace the run's own.
#[test]
fn a_step_built_again_in_the_run_takes_the_runs_route_and_reads_nothing() {
    let files = Arc::new(Files::default());
    let service = FakeService::over(&files);
    let state = slot_for(&service, "alice");
    let mut store = RouteStore::loading();
    load_now(&mut store, &state);
    let mut kept = learned();
    kept.count_trap();
    store.changed();
    let mut run = GridRun::default();
    run.keep(RunRoute {
        memory: kept,
        store,
    });
    files.commit(
        KnowledgeRecord::try_new(
            state.account().clone(),
            state.scope().clone(),
            StoreVersion::INITIAL,
            upass_store::KEY,
            KnowledgeProvenance::Observed,
            KnowledgeStatus::Known,
            api::local_state::validate_knowledge_payload(
                KNOWLEDGE_SCHEMA_VERSION,
                &[3, 0, 0, 0, 0, 0],
            )
            .unwrap(),
            DurableTimestamp::from_unix_seconds(1),
        )
        .unwrap(),
    );
    let reads = service.reads.load(Ordering::SeqCst);
    let step = Run::new(FAR_SIDE, 3365, run.kept());
    assert_eq!(step.memory, kept, "the run's copy, with its traps");
    assert!(matches!(step.phase, Phase::Decide));
    assert!(!step.store.is_loading());
    assert!(step.store.unsaved(), "the owed write is still owed");
    assert_eq!(step.store, store, "the same expected version and stamp");
    assert_eq!(service.reads.load(Ordering::SeqCst), reads, "nothing is read");
}

#[test]
fn a_cold_file_is_waited_for_in_observed_ticks_and_never_read_as_empty() {
    let files = Arc::new(Files::default());
    let service = FakeService::over(&files);
    service.cold.store(3, Ordering::SeqCst);
    let state = slot_for(&service, "alice");
    let mut store = RouteStore::loading();
    for tick in 1..=3 {
        assert!(store.load(Some(&state), tick).is_pending());
    }
    assert!(matches!(
        store.load(Some(&state), 4),
        Poll::Ready(Ok(_))
    ));
    // A file that never reads fails the step after the wait, counted in ticks:
    // many calls inside one tick do not spend it.
    service.cold.store(usize::MAX, Ordering::SeqCst);
    let mut stuck = RouteStore::loading();
    for _ in 0..100 {
        assert!(stuck.load(Some(&state), 10).is_pending());
    }
    assert!(stuck.load(Some(&state), 10 + LOAD_TICKS).is_pending());
    assert!(matches!(
        stuck.load(Some(&state), 10 + LOAD_TICKS + 1),
        Poll::Ready(Err(ActionError::Failed(_)))
    ));
}

#[test]
fn a_failed_write_is_sent_again_and_stops_after_the_bound() {
    let files = Arc::new(Files::default());
    let service = FakeService::over(&files);
    let state = slot_for(&service, "alice");
    let mut store = RouteStore::loading();
    load_now(&mut store, &state);
    service.fail_writes.store(2, Ordering::SeqCst);
    save_now(&mut store, &state, &learned());
    assert_eq!(service.submitted.load(Ordering::SeqCst), 3, "twice failed, once saved");
    let mut again = RouteStore::loading();
    assert_eq!(load_now(&mut again, &state).1, Loaded::Saved(2));
    // A file that never takes a write: the run gives up saving, nothing waits,
    // and the run's copy says the file lacks the route.
    service.fail_writes.store(usize::MAX, Ordering::SeqCst);
    let before = service.submitted.load(Ordering::SeqCst);
    save_now(&mut again, &state, &GridMemory::default());
    assert_eq!(
        service.submitted.load(Ordering::SeqCst) - before,
        usize::from(MAX_WRITE_FAILURES)
    );
    assert!(again.settled());
    assert!(again.unsaved(), "a later step of the run tries again");
}

#[test]
fn a_refused_write_is_sent_again_once_per_observed_tick() {
    struct Full(AtomicUsize);
    impl LocalStatePort for Full {
        fn lease(&self, identity: &DomainIdentity) -> Result<DomainLease, ErrorCode> {
            Err(ErrorCode::NotReady {
                domain: identity.domain(),
            })
        }
        fn submit(&self, edit: DomainEdit) -> SubmitReceipt {
            self.0.fetch_add(1, Ordering::SeqCst);
            SubmitReceipt::Refused(ErrorCode::Busy {
                domain: edit.identity().domain(),
            })
        }
        fn observe(&self, _: RequestId) -> Option<WriteSettlement> {
            None
        }
    }
    impl StatePort for Full {
        fn lease_version(&self, _: &DomainIdentity) -> Result<StoreVersion, ErrorCode> {
            Ok(StoreVersion::INITIAL)
        }
        fn start_knowledge_read(&self, _: &DomainIdentity) -> Result<KnowledgeTicket, ErrorCode> {
            Ok(KnowledgeTicket::new(0, 1))
        }
        fn poll_knowledge_read(
            &self,
            _: &DomainIdentity,
            _: KnowledgeTicket,
            _: &KnowledgeKey,
        ) -> Option<Result<Option<StoredKnowledge>, ErrorCode>> {
            Some(Ok(None))
        }
    }
    let port = Arc::new(Full(AtomicUsize::new(0)));
    let state = SlotState::new(
        Arc::clone(&port) as Arc<dyn StatePort>,
        AccountKey::try_new("local-289", "alice").unwrap(),
        ContentScope::try_new(api::selected::ClientRevision::R289, "test-content").unwrap(),
    );
    let mut store = RouteStore::loading();
    load_now(&mut store, &state);
    store.changed();
    // Ten polls inside one tick spend one refusal, not ten.
    for _ in 0..10 {
        store.pump(Some(&state), 100, &learned());
    }
    assert_eq!(port.0.load(Ordering::SeqCst), 1);
    for tick in 101..=104 {
        store.pump(Some(&state), tick, &learned());
    }
    assert_eq!(
        port.0.load(Ordering::SeqCst),
        usize::from(MAX_WRITE_FAILURES),
        "one refusal per tick until the run stops saving"
    );
    assert!(store.settled());
}

/// An accepted write is followed to its terminal outcome. However long the
/// writer takes, the request is not forgotten and sent again, and the store
/// never reports itself settled before the outcome.
#[test]
fn an_accepted_write_is_followed_until_it_settles_and_never_sent_again() {
    let files = Arc::new(Files::default());
    let service = FakeService::over(&files);
    let state = slot_for(&service, "alice");
    let mut store = RouteStore::loading();
    load_now(&mut store, &state);
    service.slow.store(500, Ordering::SeqCst);
    store.changed();
    for _ in 0..400 {
        store.pump(Some(&state), next_tick(), &learned());
        assert!(!store.settled(), "the write has no outcome yet");
        assert!(store.unsaved());
    }
    assert_eq!(
        service.submitted.load(Ordering::SeqCst),
        1,
        "the request is the only one the writer was given"
    );
    for _ in 0..200 {
        store.pump(Some(&state), next_tick(), &learned());
    }
    assert!(store.settled());
    assert!(!store.unsaved());
    assert_eq!(service.submitted.load(Ordering::SeqCst), 1);
    let mut after = RouteStore::loading();
    assert_eq!(load_now(&mut after, &state).1, Loaded::Saved(2));
}

/// A route learned while a write is in flight is sent after that write, so the
/// file's last write is the newest route.
#[test]
fn a_change_while_a_write_is_in_flight_is_sent_after_it() {
    let files = Arc::new(Files::default());
    let service = FakeService::over(&files);
    let state = slot_for(&service, "alice");
    let mut store = RouteStore::loading();
    load_now(&mut store, &state);
    service.slow.store(3, Ordering::SeqCst);
    let mut first = GridMemory::default();
    first.confirm(Probe { col: 0, lane: 3 });
    store.changed();
    store.pump(Some(&state), next_tick(), &first);
    assert_eq!(service.submitted.load(Ordering::SeqCst), 1);
    // The second lane is learned while the first write is in flight.
    let second = learned();
    store.changed();
    store.pump(Some(&state), next_tick(), &second);
    assert_eq!(service.submitted.load(Ordering::SeqCst), 1, "one in flight");
    assert!(!store.settled());
    for _ in 0..10 {
        store.pump(Some(&state), next_tick(), &second);
    }
    assert!(store.settled());
    assert_eq!(service.submitted.load(Ordering::SeqCst), 2);
    let mut after = RouteStore::loading();
    let (memory, loaded) = load_now(&mut after, &state);
    assert_eq!(loaded, Loaded::Saved(2));
    assert_eq!(memory.lane, second.lane);
}

#[test]
fn only_a_changed_route_is_written() {
    let files = Arc::new(Files::default());
    let service = FakeService::over(&files);
    let state = slot_for(&service, "alice");
    let mut store = RouteStore::loading();
    load_now(&mut store, &state);
    for _ in 0..10 {
        store.pump(Some(&state), next_tick(), &learned());
    }
    assert_eq!(service.submitted.load(Ordering::SeqCst), 0);
}

/// The step polls the store on every observed tick, so a poll that changes
/// nothing allocates nothing: an idle store, and a store following a write the
/// service has not settled.
#[test]
fn unchanged_route_polls_allocate_nothing() {
    let files = Arc::new(Files::default());
    let service = FakeService::over(&files);
    let state = slot_for(&service, "alice");
    let mut store = RouteStore::loading();
    load_now(&mut store, &state);
    let memory = learned();
    let idle = allocation_counter::measure(|| {
        for _ in 0..1000 {
            store.pump(Some(&state), next_tick(), std::hint::black_box(&memory));
            std::hint::black_box(store.settled());
            std::hint::black_box(store.unsaved());
        }
    });
    assert_eq!(idle.count_total, 0, "{idle:?}");
    assert_eq!(idle.bytes_total, 0, "{idle:?}");
    service.slow.store(usize::MAX, Ordering::SeqCst);
    store.changed();
    store.pump(Some(&state), next_tick(), &memory);
    assert!(!store.settled());
    let waiting = allocation_counter::measure(|| {
        for _ in 0..1000 {
            store.pump(Some(&state), next_tick(), std::hint::black_box(&memory));
            std::hint::black_box(store.settled());
        }
    });
    assert_eq!(waiting.count_total, 0, "{waiting:?}");
    assert_eq!(waiting.bytes_total, 0, "{waiting:?}");
    // Without a capability, nothing is checked and nothing allocates.
    let mut none = RouteStore::loading();
    let _ = none.load(None, 1);
    let nothing = allocation_counter::measure(|| {
        for _ in 0..1000 {
            none.pump(None, next_tick(), std::hint::black_box(&memory));
        }
    });
    assert_eq!(nothing.count_total, 0, "{nothing:?}");
    // Keeping an unchanged route for the run reuses its one box.
    let mut run = GridRun::default();
    run.keep(RunRoute { memory, store });
    let kept = allocation_counter::measure(|| {
        for _ in 0..1000 {
            run.keep(std::hint::black_box(RunRoute { memory, store }));
        }
    });
    assert_eq!(kept.count_total, 0, "{kept:?}");
}

/// A cleared route is published as an `Unknown` tombstone over the route it
/// replaces, and read back as an invalidated route that starts empty.
#[test]
fn a_cleared_route_is_saved_as_an_unknown_tombstone() {
    let files = Arc::new(Files::default());
    let service = FakeService::over(&files);
    let state = slot_for(&service, "alice");
    let mut store = RouteStore::loading();
    load_now(&mut store, &state);
    save_now(&mut store, &state, &learned());
    let positive = files.stored("local-289/alice").expect("the route is saved");
    assert_eq!(positive.record().status(), KnowledgeStatus::Known);
    assert_eq!(positive.record().provenance(), KnowledgeProvenance::Observed);
    assert_eq!(positive.record().payload(), learned().to_payload());
    // The learned route went stale: the step cleared it.
    let mut stale = learned();
    stale.clear_route();
    save_now(&mut store, &state, &stale);
    let tombstone = files.stored("local-289/alice").expect("the tombstone is saved");
    assert_eq!(tombstone.record().status(), KnowledgeStatus::Unknown);
    assert_eq!(
        tombstone.record().revocation(),
        Some(KnowledgeRevocation::ObservationRefused)
    );
    assert_eq!(tombstone.record().provenance(), KnowledgeProvenance::Observed);
    assert!(tombstone.record().payload().is_empty());
    assert_eq!(tombstone.record().expected(), positive.version());
    assert!(tombstone.version() > positive.version());
    let mut again = RouteStore::loading();
    let (memory, loaded) = load_now(&mut again, &state);
    assert_eq!(loaded, Loaded::Invalidated);
    assert!(memory.is_empty());
}

/// A failed write is sent again as the same fact: the stamp of the
/// observation and the version it replaces are not renewed by the retry.
#[test]
fn a_retried_write_keeps_its_stamp_and_expected_version() {
    let files = Arc::new(Files::default());
    let service = FakeService::over(&files);
    let state = slot_for(&service, "alice");
    let mut store = RouteStore::loading();
    let mut first = GridMemory::default();
    first.confirm(Probe { col: 0, lane: 3 });
    load_now(&mut store, &state);
    save_now(&mut store, &state, &first);
    let saved = files.stored("local-289/alice").unwrap().version();
    service.fail_writes.store(2, Ordering::SeqCst);
    store.changed();
    // The retries cross at least one wall-clock second.
    let mut ticks = 0;
    while !store.settled() {
        store.pump(Some(&state), next_tick(), &learned());
        if ticks == 1 {
            std::thread::sleep(std::time::Duration::from_millis(1100));
        }
        ticks += 1;
        assert!(ticks < 50, "the write did not settle");
    }
    let sent = service.sent();
    assert_eq!(sent.len(), 4, "one first write, two failures, one success");
    let retries = &sent[1..];
    for retry in retries {
        assert_eq!(retry, &retries[0], "the retry is the same fact");
        assert_eq!(retry.expected(), saved);
    }
    let committed = files.stored("local-289/alice").unwrap();
    assert_eq!(committed.record(), &retries[0]);
    assert_eq!(store.expected_for_test(), committed.version());
}

/// A positive write that lost to a later invalidation by another writer is
/// refused by the version check and never sent again: the run keeps its route
/// for itself, and the store keeps the tombstone.
#[test]
fn a_stale_positive_retry_cannot_replace_a_later_invalidation() {
    let files = Arc::new(Files::default());
    let service = FakeService::over(&files);
    let state = slot_for(&service, "alice");
    let mut store = RouteStore::loading();
    let mut first = GridMemory::default();
    first.confirm(Probe { col: 0, lane: 3 });
    load_now(&mut store, &state);
    save_now(&mut store, &state, &first);
    let known = files.stored("local-289/alice").unwrap();
    // The next write fails once, and is retried with the version it read.
    service.fail_writes.store(1, Ordering::SeqCst);
    store.changed();
    store.pump(Some(&state), next_tick(), &learned());
    store.pump(Some(&state), next_tick(), &learned());
    assert!(!store.settled(), "the failed write is owed again");
    // Meanwhile another writer invalidates the route in the same second.
    let tombstone = known.record().invalidated_with(
        known.version(),
        known.record().observed_at(),
        KnowledgeRevocation::ObservationRefused,
    );
    let tombstone_version = files.commit(tombstone.clone());
    for _ in 0..5 {
        store.pump(Some(&state), next_tick(), &learned());
    }
    assert!(store.settled());
    assert!(!store.unsaved(), "a conflict is not owed a write");
    let sent = service.sent();
    assert_eq!(sent.len(), 3, "the first write, the failure, one retry");
    assert_eq!(sent[2], sent[1], "the retry kept its stamp and expected version");
    assert_eq!(sent[2].expected(), known.version());
    let stored = files.stored("local-289/alice").unwrap();
    assert_eq!(stored.version(), tombstone_version);
    assert_eq!(stored.record(), &tombstone, "the invalidation stands");
    // Learning more in this run sends nothing over it.
    store.changed();
    for _ in 0..5 {
        store.pump(Some(&state), next_tick(), &learned());
    }
    assert_eq!(service.sent().len(), 3);
}

/// A `Partial` fact is read as the route it holds, not as Unknown.
#[test]
fn a_partial_fact_is_read_as_its_route() {
    let files = Arc::new(Files::default());
    let service = FakeService::over(&files);
    let state = slot_for(&service, "alice");
    let partial = KnowledgeRecord::try_new(
        state.account().clone(),
        state.scope().clone(),
        StoreVersion::INITIAL,
        upass_store::KEY,
        KnowledgeProvenance::Observed,
        KnowledgeStatus::Partial,
        api::local_state::validate_knowledge_payload(
            KNOWLEDGE_SCHEMA_VERSION,
            &learned().to_payload(),
        )
        .unwrap(),
        DurableTimestamp::from_unix_seconds(1),
    )
    .unwrap();
    let version = files.commit(partial);
    let mut store = RouteStore::loading();
    let (memory, loaded) = load_now(&mut store, &state);
    assert_eq!(loaded, Loaded::Saved(2));
    assert_eq!(memory.to_payload(), learned().to_payload());
    assert_eq!(store.expected_for_test(), version);
}

// ---------------------------------------------------------------------------
// The step with a slot's retained cell and state capability that outlive each
// tick, as the slot's do.

/// The script's log lines a step wrote.
#[derive(Default)]
struct Sink(Vec<(api::hostlog::Level, String)>);

impl NativeOutput for Sink {
    fn status(&mut self, _: ScriptStatus) {}
    fn paint(&mut self, _: Arc<crate::paint::ScriptPaint>) {}
    fn log(&mut self, level: api::hostlog::Level, line: &str) {
        self.0.push((level, line.to_owned()));
    }
    fn settings_applied(&mut self, _: u64) {}
}

struct Stage {
    ledger: Option<Box<Ledger>>,
    budget: TickBudget,
    retained: RetainedMemory,
    state: Option<&'static SlotState>,
    tick: u64,
    log: Sink,
}

impl Stage {
    fn new(state: Option<&'static SlotState>) -> Self {
        Self {
            ledger: None,
            budget: TickBudget::default(),
            retained: RetainedMemory::default(),
            state,
            tick: 0,
            log: Sink::default(),
        }
    }

    /// Run `f` at the current observed tick.
    fn at<R>(&mut self, snapshot: &GameSnapshot, f: impl FnOnce(&mut StepContext<'_, '_>) -> R) -> R {
        with_tick_retained_output(
            snapshot,
            &mut self.ledger,
            &mut self.budget,
            &mut self.retained,
            self.state,
            self.tick,
            &mut self.log,
            |native| with_step(native, f),
        )
    }

    /// The step the runner builds, from this run's retained cell.
    fn begin(&mut self) -> Run {
        Run::new(FAR_SIDE, 3365, self.retained.quester().upass_grid.kept())
    }

    /// One poll at the next observed tick.
    fn poll(
        &mut self,
        run: &mut Run,
        snapshot: &GameSnapshot,
    ) -> Poll<Result<StepOutcome, ActionError>> {
        self.tick += 1;
        self.poll_again(run, snapshot)
    }

    /// One more poll inside the same observed tick.
    fn poll_again(
        &mut self,
        run: &mut Run,
        snapshot: &GameSnapshot,
    ) -> Poll<Result<StepOutcome, ActionError>> {
        self.at(snapshot, |cx| run.poll(cx))
    }

    /// The runner drops a step for survival.
    fn drop_for_survival(&mut self, mut run: Run, snapshot: &GameSnapshot) {
        with_tick_retained_output(
            snapshot,
            &mut self.ledger,
            &mut self.budget,
            &mut self.retained,
            self.state,
            self.tick,
            &mut self.log,
            |native| run.cancel(native.actions),
        );
    }

    fn route(&mut self) -> Option<([u8; PAYLOAD_LEN], u8, bool)> {
        retained_route(&mut self.retained)
    }
}

/// Poll `run` until it is `Ready`, at most `limit` ticks.
fn until_ready(
    stage: &mut Stage,
    run: &mut Run,
    snapshot: &GameSnapshot,
    limit: u64,
) -> Option<Result<StepOutcome, ActionError>> {
    for _ in 0..limit {
        if let Poll::Ready(result) = stage.poll(run, snapshot) {
            return Some(result);
        }
    }
    None
}

/// A step that learned lane 3 of column 1 (a quiet probe), stands at the far
/// side, and has one write accepted by the service.
fn learned_one_lane(stage: &mut Stage) -> Run {
    let healthy = snapshot(world(ENTRY.x, ENTRY.z), 40, None);
    let mut run = stage.begin();
    // The route is read, then the probe of lane 3 in column 1 stands quiet on
    // its tile for its observed ticks.
    assert!(stage.poll(&mut run, &healthy).is_pending());
    assert!(matches!(run.phase, Phase::Decide));
    run.phase = Phase::Observe {
        probe: Probe { col: 0, lane: 3 },
        until: stage.tick + 1,
    };
    let on_tile = snapshot(world(2476, 9677), 40, None);
    assert!(stage.poll(&mut run, &on_tile).is_pending());
    assert!(matches!(run.phase, Phase::Decide));
    let (payload, traps, _) = stage.route().expect("the run read its route");
    assert_eq!(payload, [3, 0, 0, 0, 0, 0], "column 1 is lane 3");
    assert_eq!(traps, 0);
    run
}

#[test]
fn the_step_saves_what_it_learns_and_ends_after_the_write_settled() {
    if !api::test_support::game_data_available() {
        return;
    }
    let files = Arc::new(Files::default());
    let service = FakeService::over(&files);
    // A write takes three observations to settle.
    service.slow.store(3, Ordering::SeqCst);
    let state = leaked_slot(&service, "alice");
    let mut stage = Stage::new(Some(state));
    let mut run = learned_one_lane(&mut stage);
    assert_eq!(service.submitted.load(Ordering::SeqCst), 1);
    // The step reaches the far side while the write is unsettled: it waits.
    let far = snapshot(world(2466, 9677), 40, None);
    let result = until_ready(&mut stage, &mut run, &far, 10).expect("the step ends");
    assert!(result.is_ok());
    assert_eq!(stage.route().map(|route| route.2), Some(false), "saved");
    // A step of a new run reads column 1 back.
    let mut restarted = RouteStore::loading();
    let (memory, loaded) = load_now(&mut restarted, state);
    assert_eq!(loaded, Loaded::Saved(1));
    assert_eq!(memory.lane, [3, 0, 0]);
}

/// A slow writer: the write settles some ticks after the step reached the far
/// side. The step waits for it in observed ticks (the wait does not run out
/// sooner when the step is polled more than once a tick) and ends with the
/// route saved, never with its last write unknown.
#[test]
fn a_slow_writer_holds_the_end_of_the_step_for_observed_ticks_not_polls() {
    if !api::test_support::game_data_available() {
        return;
    }
    let files = Arc::new(Files::default());
    let service = FakeService::over(&files);
    // The service answers "not yet" 60 times. The step is polled twice in each
    // observed tick, so the writer is observed twice a tick and settles after
    // about 30 ticks: more calls than the bound's 40, fewer ticks.
    service.slow.store(60, Ordering::SeqCst);
    let state = leaked_slot(&service, "alice");
    let mut stage = Stage::new(Some(state));
    let mut run = learned_one_lane(&mut stage);
    let far = snapshot(world(2466, 9677), 40, None);
    let mut ended = None;
    for _ in 0..CONFIRM_TICKS {
        // Two polls inside one observed tick, as the runner may call.
        let first = stage.poll(&mut run, &far);
        if let Poll::Ready(result) = first {
            ended = Some((stage.tick, result));
            break;
        }
        if let Poll::Ready(result) = stage.poll_again(&mut run, &far) {
            ended = Some((stage.tick, result));
            break;
        }
    }
    let (tick, result) = ended.expect("the step ends within the bound");
    assert!(result.is_ok());
    assert!(
        tick >= 25,
        "the step waited for the writer (ended at tick {tick})"
    );
    assert_eq!(service.submitted.load(Ordering::SeqCst), 1, "never sent again");
    assert_eq!(stage.route().map(|route| route.2), Some(false));
    let mut restarted = RouteStore::loading();
    let (memory, _) = load_now(&mut restarted, state);
    assert_eq!(memory.lane, [3, 0, 0], "the last route is the saved one");
}

/// A writer that never settles: the step ends after the stated number of
/// observed ticks with the route kept in the run's memory, one warning line
/// says the save is unconfirmed, and the request was never sent a second time.
#[test]
fn a_writer_that_never_settles_ends_the_step_with_the_route_kept_and_one_warning() {
    if !api::test_support::game_data_available() {
        return;
    }
    let files = Arc::new(Files::default());
    let service = FakeService::over(&files);
    service.slow.store(usize::MAX, Ordering::SeqCst);
    let state = leaked_slot(&service, "alice");
    let mut stage = Stage::new(Some(state));
    let mut run = learned_one_lane(&mut stage);
    let far = snapshot(world(2466, 9677), 40, None);
    // The poll that reaches the far side holds the end; count from there.
    assert!(stage.poll(&mut run, &far).is_pending());
    let held_at = stage.tick;
    let mut ended_at = None;
    for _ in 0..CONFIRM_TICKS + 5 {
        // Many polls inside one tick do not spend the bound.
        let mut result = stage.poll(&mut run, &far);
        for _ in 0..3 {
            if result.is_pending() {
                result = stage.poll_again(&mut run, &far);
            }
        }
        if let Poll::Ready(result) = result {
            ended_at = Some((stage.tick, result));
            break;
        }
    }
    let (tick, result) = ended_at.expect("the step ends");
    assert!(result.is_ok(), "the crossing itself succeeded");
    assert_eq!(tick, held_at + CONFIRM_TICKS, "the bound is in observed ticks");
    assert_eq!(
        service.submitted.load(Ordering::SeqCst),
        1,
        "the accepted request was followed, not sent again"
    );
    let (payload, traps, unsaved) = stage.route().expect("the route is kept");
    assert_eq!(payload, [3, 0, 0, 0, 0, 0]);
    assert_eq!(traps, 0);
    assert!(unsaved, "the run's copy says the file may lack the route");
    let warnings = stage
        .log
        .0
        .iter()
        .filter(|(level, line)| {
            *level == api::hostlog::Level::Warn && line.contains("not confirmed saved")
        })
        .count();
    assert_eq!(warnings, 1, "one warning line");
}

/// Learn the lane of `probe` with a quiet probe: the step stands on its probe
/// tile for the observed ticks.
fn learn_probe(stage: &mut Stage, run: &mut Run, probe: Probe) {
    run.phase = Phase::Observe {
        probe,
        until: stage.tick + 1,
    };
    let on_tile = snapshot(
        world(PROBE_X[usize::from(probe.col)], lane_z(probe.lane)),
        40,
        None,
    );
    assert!(stage.poll(run, &on_tile).is_pending());
}

/// Poll `run` for `ticks` observed ticks at the entrance.
fn idle(stage: &mut Stage, run: &mut Run, ticks: usize) {
    let healthy = snapshot(world(ENTRY.x, ENTRY.z), 40, None);
    for _ in 0..ticks {
        let _ = stage.poll(run, &healthy);
    }
}

/// A step built again in the same run follows the write its predecessor left
/// in flight: it does not send a second write against the version the store
/// held before that write, so the predecessor's commit is no conflict, and a
/// later lane is saved on top of the committed version.
#[test]
fn a_step_built_again_follows_its_predecessors_write_and_saves_later_lanes() {
    if !api::test_support::game_data_available() {
        return;
    }
    let files = Arc::new(Files::default());
    let service = FakeService::over(&files);
    service.slow.store(5, Ordering::SeqCst);
    let state = leaked_slot(&service, "alice");
    let mut stage = Stage::new(Some(state));
    let run = learned_one_lane(&mut stage);
    assert_eq!(service.submitted.load(Ordering::SeqCst), 1);
    let healthy = snapshot(world(ENTRY.x, ENTRY.z), 40, None);
    stage.drop_for_survival(run, &healthy);
    service.slow.store(0, Ordering::SeqCst);
    let reads = service.reads.load(Ordering::SeqCst);
    let mut next = stage.begin();
    idle(&mut stage, &mut next, 8);
    assert_eq!(service.submitted.load(Ordering::SeqCst), 1, "followed, not sent again");
    assert_eq!(service.reads.load(Ordering::SeqCst), reads);
    let first = files.stored("local-289/alice").expect("the predecessor's write");
    assert_eq!(first.record().payload(), [3, 0, 0, 0, 0, 0]);
    assert_eq!(stage.route().map(|route| route.2), Some(false));
    // A later lane is saved over the committed version.
    learn_probe(&mut stage, &mut next, Probe { col: 1, lane: 3 });
    idle(&mut stage, &mut next, 3);
    let sent = service.sent();
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[1].expected(), first.version());
    let second = files.stored("local-289/alice").expect("the later lane");
    assert_eq!(second.record().payload(), [3, 3, 0, 0, 0, 0]);
    assert!(second.version() > first.version());
    assert_eq!(stage.route().map(|route| route.2), Some(false));
}

/// An unsaved positive change survives a step rebuilt in the same run with
/// the version it replaces and its stamp. When another writer has invalidated
/// the route meanwhile, the owed write is refused by that version, the
/// tombstone stands, and the run stops saving, also for the steps it builds
/// later.
#[test]
fn a_rebuilt_step_never_writes_its_stale_route_over_a_newer_tombstone() {
    if !api::test_support::game_data_available() {
        return;
    }
    let files = Arc::new(Files::default());
    let service = FakeService::over(&files);
    let state = leaked_slot(&service, "alice");
    let mut stage = Stage::new(Some(state));
    let mut run = learned_one_lane(&mut stage);
    idle(&mut stage, &mut run, 2);
    let known = files.stored("local-289/alice").expect("column 1 is saved");
    // The next write fails once: the change stays owed against `known`.
    service.fail_writes.store(1, Ordering::SeqCst);
    learn_probe(&mut stage, &mut run, Probe { col: 1, lane: 3 });
    idle(&mut stage, &mut run, 1);
    assert_eq!(stage.route().map(|route| route.2), Some(true), "owed");
    let healthy = snapshot(world(ENTRY.x, ENTRY.z), 40, None);
    stage.drop_for_survival(run, &healthy);
    // Another writer invalidates the route.
    let tombstone = known.record().invalidated_with(
        known.version(),
        known.record().observed_at(),
        KnowledgeRevocation::ObservationRefused,
    );
    let tombstone_version = files.commit(tombstone.clone());
    let mut next = stage.begin();
    idle(&mut stage, &mut next, 4);
    let sent = service.sent();
    assert_eq!(sent.len(), 3, "column 1, the failed write, one retry");
    assert_eq!(sent[2], sent[1], "the retry is the same fact: stamp and version");
    assert_eq!(sent[2].expected(), known.version());
    let stored = files.stored("local-289/alice").unwrap();
    assert_eq!(stored.version(), tombstone_version);
    assert_eq!(stored.record(), &tombstone, "the tombstone stands");
    assert_eq!(stage.route().map(|route| route.2), Some(false));
    // The conflict stop is the run's: a step built again learns a lane and
    // saves nothing.
    stage.drop_for_survival(next, &healthy);
    let mut later = stage.begin();
    learn_probe(&mut stage, &mut later, Probe { col: 2, lane: 3 });
    idle(&mut stage, &mut later, 3);
    assert_eq!(service.sent().len(), 3);
    assert_eq!(files.stored("local-289/alice").unwrap().record(), &tombstone);
}

#[test]
fn a_restarted_step_does_not_try_a_lane_that_trapped_before() {
    if !api::test_support::game_data_available() {
        return;
    }
    let files = Arc::new(Files::default());
    let service = FakeService::over(&files);
    let state = leaked_slot(&service, "alice");
    let mut first = RouteStore::loading();
    load_now(&mut first, state);
    // The old step: lane 3 trapped in column 1, then lane 2 trapped.
    let mut old = GridMemory::default();
    old.rule_out(Probe { col: 0, lane: 3 });
    old.rule_out(Probe { col: 0, lane: 2 });
    save_now(&mut first, state, &old);
    // A new run's step reads that and probes lane 4 (the order is 3, 2, 4, 1,
    // 5), walking to its own tile.
    let mut stage = Stage::new(Some(state));
    let healthy = snapshot(world(ENTRY.x, ENTRY.z), 40, None);
    let mut run = stage.begin();
    for _ in 0..3 {
        let _ = stage.poll(&mut run, &healthy);
    }
    let (payload, traps, _) = stage.route().expect("read");
    assert_eq!(payload, old.to_payload());
    assert_eq!(traps, 0, "a new run starts with no traps taken");
    let memory = GridMemory::from_payload(&payload).unwrap();
    assert_eq!(memory.next(), Next::Probe(Probe { col: 0, lane: 4 }));
    // The first walk is along proven tiles to the lane's launch tile.
    let (x, z) = launch_tile(&memory, Probe { col: 0, lane: 4 });
    assert_eq!(
        last_walk(&stage.ledger),
        Some((world(x, z), 0)),
        "the launch tile of lane 4, column 1"
    );
    assert_eq!(z, lane_z(4));
}

/// With the 8 traps of a fixed route already spent in this run, a step built
/// again does not try another lane, whatever the file holds.
#[test]
fn the_trap_bound_holds_for_a_step_built_again_in_the_run() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut stage = Stage::new(None);
    seed_retained_route(&mut stage.retained, [0, 0, 0, 0, 0, 0], MAX_TRAPS);
    let healthy = snapshot(world(ENTRY.x, ENTRY.z), 40, None);
    let mut run = stage.begin();
    let result = until_ready(&mut stage, &mut run, &healthy, 10).expect("the step stops");
    let Err(ActionError::Failed(detail)) = result else {
        panic!("the bound must stop the step");
    };
    assert!(detail.contains("traps taken"), "{detail}");
    assert!(last_walk(&stage.ledger).is_none(), "no probe was walked");
}
