#![cfg(target_os = "linux")]

use std::fs;
use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::Duration;

use api::identity::Deadline;
use api::snapshot::WorldTile;
use api::work::{
    InspectionPhase, MonotonicClock, SharedCancellation, SliceBudget, StepContext, SystemClock,
};
use nav::grid::StepGrid;
use nav::producer;
use nav::router::{
    CursorStep, FindOptions, RouteError, SearchCursor, SearchOutcome, SearchRequest, NODE_BUDGET,
};
use nav::world::NavWorld;
use nav::WorldState;

const MAX_SLICES: usize = 140_000;
// The system-allocator probe measured a 2,441,216-byte peak delta.
// Four MiB leaves 1,753,088 bytes (71.8%) for page and allocator variation.
const RESIDENT_DELTA_LIMIT_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Copy)]
enum ExpectedResult {
    NoPath,
    Destination(WorldTile),
}

struct SearchObservation {
    name: &'static str,
    baseline_rss_bytes: usize,
    peak_rss_bytes: usize,
    slices: usize,
}

fn vm_rss_bytes() -> usize {
    let status = fs::read_to_string("/proc/self/status").expect("read process status");
    let kibibytes = status
        .lines()
        .find_map(|line| line.strip_prefix("VmRSS:"))
        .and_then(|value| value.split_whitespace().next())
        .and_then(|value| value.parse::<usize>().ok())
        .expect("process status contains VmRSS");
    kibibytes
        .checked_mul(1024)
        .expect("VmRSS byte conversion does not overflow")
}

fn run_search(
    world: &Arc<NavWorld>,
    name: &'static str,
    target: WorldTile,
    expected: ExpectedResult,
) -> SearchObservation {
    let lease = producer::lease(world).expect("lease the corridor world");
    let state = Arc::new(WorldState::empty());
    let request = SearchRequest::Single {
        from: WorldTile {
            x: 0,
            z: 0,
            level: 0,
        },
        to: target,
        options: FindOptions::default(),
        avoid: Box::new([]),
        max_settles: NODE_BUDGET,
    };

    let baseline_rss_bytes = vm_rss_bytes();
    let mut peak_rss_bytes = baseline_rss_bytes;
    let mut cursor = SearchCursor::new(request, lease, state).expect("construct search cursor");
    peak_rss_bytes = peak_rss_bytes.max(vm_rss_bytes());

    let clock = SystemClock;
    let cancellation = SharedCancellation::new();
    let units_per_slice = NonZeroU32::new(64).expect("64 work units per slice");
    let mut previous_settled = 0;
    let mut completion = None;
    let mut slices = 0;

    for slice in 1..=MAX_SLICES {
        // The cursor remains active at this point, including after each yield.
        peak_rss_bytes = peak_rss_bytes.max(vm_rss_bytes());
        let budget = SliceBudget::new(
            units_per_slice,
            Deadline::after(clock.now(), Duration::from_secs(5)),
        );
        let mut context = StepContext::<InspectionPhase>::new(&clock, budget, &cancellation);

        match cursor.advance(&mut context) {
            CursorStep::Complete(done) => {
                completion = Some(done);
                slices = slice;
                break;
            }
            CursorStep::Yielded => {
                peak_rss_bytes = peak_rss_bytes.max(vm_rss_bytes());
                let settled = cursor.forward_settled();
                assert!(
                    settled >= previous_settled,
                    "settlement progress moved backward"
                );
                previous_settled = settled;
            }
            CursorStep::Cancelled(reason) => {
                panic!("{name}: unexpected route-search cancellation: {reason:?}")
            }
            CursorStep::Failed(error) => {
                panic!("{name}: route-search cursor failed: {error:?}")
            }
        }
    }

    let completion = completion.unwrap_or_else(|| {
        panic!("{name}: route search exceeded the finite {MAX_SLICES}-slice limit")
    });
    let (outcome, _) = completion.into_parts();
    match (expected, outcome) {
        (
            ExpectedResult::NoPath,
            SearchOutcome::Single {
                route: Err(RouteError::NoPath),
            },
        ) => {}
        (
            ExpectedResult::Destination(expected_destination),
            SearchOutcome::Single { route: Ok(route) },
        ) => assert_eq!(
            route.destination(),
            expected_destination,
            "{name}: wrong destination"
        ),
        (ExpectedResult::NoPath, _) => panic!("{name}: expected an unreachable target"),
        (ExpectedResult::Destination(_), _) => panic!("{name}: expected a completed route"),
    }

    SearchObservation {
        name,
        baseline_rss_bytes,
        peak_rss_bytes,
        slices,
    }
}

#[test]
fn cooperative_searches_keep_resident_memory_bounded() {
    let world = Arc::new(NavWorld::from_grid(&StepGrid::fixture_open_1x40()));
    let unreachable = WorldTile {
        x: 5,
        z: 5,
        level: 0,
    };
    let reachable = WorldTile {
        x: 2,
        z: 0,
        level: 0,
    };

    // Defer the ceiling assertions until both active searches finish.
    let observations = [
        run_search(
            &world,
            "off-corridor refusal",
            unreachable,
            ExpectedResult::NoPath,
        ),
        run_search(
            &world,
            "reachable target",
            reachable,
            ExpectedResult::Destination(reachable),
        ),
    ];

    for observation in &observations {
        let resident_delta = observation
            .peak_rss_bytes
            .saturating_sub(observation.baseline_rss_bytes);
        assert!(
            resident_delta <= RESIDENT_DELTA_LIMIT_BYTES,
            "{} grew resident memory by {resident_delta} bytes over the {}-byte ceiling across {} slices",
            observation.name,
            RESIDENT_DELTA_LIMIT_BYTES,
            observation.slices,
        );
    }
}
