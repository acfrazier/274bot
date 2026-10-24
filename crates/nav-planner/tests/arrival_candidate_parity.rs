//! Compare cooperative arrival work with the saved ordered real-content oracle.
//!
//! The baseline files come from the unchanged synchronous producer. This test
//! does not regenerate expected results from the new kernel. Every cooperative
//! checkpoint returns Pending once. The collision callback retains its complete
//! provider domain, including reads outside the filter's dense square.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::task::{Context, Poll, Waker};
use std::time::Instant;

use api::query::{retain_arrival_candidates_cooperative, ArrivalCollisionBounds};
use api::snapshot::WorldTile;
use client::dash3d::CollisionFlag;
use nav_planner::collision::WorldCollision;
use nav_planner::world::NavWorld;
use serde::Deserialize;

// The frozen T1.2 policy gives the complete Route family 192 MiB. An isolated
// arrival phase cannot exceed that complete-job reservation.
const ROUTE_CAP_BYTES: usize = 192 * 1024 * 1024;
const SQ_BLOCKED: i32 = CollisionFlag::SQ_BLOCKED;

#[derive(Deserialize)]
struct Origin {
    x: i32,
    z: i32,
}

#[derive(Deserialize)]
struct CollisionDomain {
    origin: Origin,
    width: usize,
    height: usize,
    outside: String,
}

#[derive(Deserialize)]
struct Case {
    destination: WorldTile,
    radius: i32,
    input_count: usize,
    output_count: usize,
    forced_blocked_candidates: Vec<WorldTile>,
    collision_domain: CollisionDomain,
}

#[derive(Default)]
struct YieldOnce(bool);

impl Future for YieldOnce {
    type Output = ();

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<()> {
        let this = self.get_mut();
        if this.0 {
            Poll::Ready(())
        } else {
            this.0 = true;
            context.waker().wake_by_ref();
            Poll::Pending
        }
    }
}

fn required_path(name: &str) -> PathBuf {
    PathBuf::from(std::env::var_os(name).unwrap_or_else(|| panic!("set {name}")))
}

fn decode_tiles(bytes: &[u8]) -> Vec<WorldTile> {
    let (chunks, remainder) = bytes.as_chunks::<12>();
    assert!(remainder.is_empty());
    chunks
        .iter()
        .map(|chunk| WorldTile {
            x: i32::from_le_bytes(chunk[..4].try_into().unwrap()),
            z: i32::from_le_bytes(chunk[4..8].try_into().unwrap()),
            level: i32::from_le_bytes(chunk[8..].try_into().unwrap()),
        })
        .collect()
}

fn encode_tiles(tiles: &[WorldTile]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(tiles.len().checked_mul(12).unwrap());
    for tile in tiles {
        bytes.extend_from_slice(&tile.x.to_le_bytes());
        bytes.extend_from_slice(&tile.z.to_le_bytes());
        bytes.extend_from_slice(&tile.level.to_le_bytes());
    }
    bytes
}

fn collision_at(collision: &WorldCollision, tile: WorldTile) -> Option<i32> {
    let x = usize::try_from(tile.x.checked_sub(collision.origin.x)?).ok()?;
    let z = usize::try_from(tile.z.checked_sub(collision.origin.z)?).ok()?;
    ((0..=3).contains(&tile.level) && x < collision.width && z < collision.height)
        .then(|| collision.walkable_word(tile.x, tile.z, tile.level) as i32)
}

fn case_directories(root: &Path) -> Vec<PathBuf> {
    let mut paths: Vec<_> = std::fs::read_dir(root)
        .unwrap()
        .map(Result::unwrap)
        .filter(|entry| entry.file_type().unwrap().is_dir())
        .map(|entry| entry.path())
        .collect();
    paths.sort_unstable();
    paths
}

#[test]
fn ordered_tile_encoding_keeps_duplicates_and_signed_coordinates() {
    let tiles = [
        WorldTile {
            x: -1,
            z: i32::MAX,
            level: 3,
        },
        WorldTile {
            x: 0,
            z: i32::MIN,
            level: 0,
        },
        WorldTile {
            x: -1,
            z: i32::MAX,
            level: 3,
        },
    ];
    assert_eq!(decode_tiles(&encode_tiles(&tiles)), tiles);
}

#[test]
#[ignore = "real-content cooperative parity; set T5W_ARRIVAL_BASELINE_DIR, T5W_ARRIVAL_PARITY_DIR, and both navpack variables"]
#[expect(
    clippy::assertions_on_constants,
    reason = "The ignored measurement rejects debug runs without breaking debug compilation."
)]
fn cooperative_real_arrival_matches_saved_ordered_corpus() {
    assert!(
        !cfg!(debug_assertions),
        "run the measurement with --release"
    );
    assert_eq!(std::env::var("BOT_NAV_BUILD").as_deref(), Ok("skip"));
    let baseline = required_path("T5W_ARRIVAL_BASELINE_DIR");
    let output = required_path("T5W_ARRIVAL_PARITY_DIR");
    assert!(!output.exists(), "parity output must be fresh");
    let capture = output.join("capture");
    std::fs::create_dir_all(&capture).unwrap();
    let mut rows = Vec::new();
    let mut cancelled_cases = 0;
    for (revision, variable) in [
        ("274", "T5W_ARRIVAL_NAVPACK_274"),
        ("289", "T5W_ARRIVAL_NAVPACK_289"),
    ] {
        let pack = required_path(variable);
        let world = NavWorld::load_pack(&pack).unwrap();
        let source = baseline.join(revision);
        let expected_pack = std::fs::read_to_string(source.join("pack.sha256")).unwrap();
        assert_eq!(
            nav_planner::manifest::hash_file(&pack).unwrap(),
            expected_pack
        );
        let destination = capture.join(revision);
        std::fs::create_dir(&destination).unwrap();
        std::fs::write(destination.join("pack.sha256"), expected_pack).unwrap();
        let bounds = ArrivalCollisionBounds::new(
            world.collision.origin.x,
            world.collision.origin.z,
            world.collision.width,
            world.collision.height,
        )
        .unwrap();
        let cases = case_directories(&source);
        assert_eq!(cases.len(), 104, "the complete baseline is required");
        for path in cases {
            let name = path.file_name().unwrap().to_str().unwrap();
            let metadata = std::fs::read(path.join("case.json")).unwrap();
            let case: Case = serde_json::from_slice(&metadata).unwrap();
            assert_eq!(case.collision_domain.origin.x, bounds.origin_x());
            assert_eq!(case.collision_domain.origin.z, bounds.origin_z());
            assert_eq!(case.collision_domain.width, bounds.width());
            assert_eq!(case.collision_domain.height, bounds.height());
            assert_eq!(case.collision_domain.outside, "None");
            let input = std::fs::read(path.join("input.bin")).unwrap();
            let expected = std::fs::read(path.join("output.bin")).unwrap();
            let mut candidates = decode_tiles(&input);
            assert_eq!(candidates.len(), case.input_count);
            let mut blocked: Vec<_> = case
                .forced_blocked_candidates
                .iter()
                .map(|tile| (tile.x, tile.z, tile.level))
                .collect();
            blocked.sort_unstable();
            let collision = |tile: WorldTile| {
                if blocked.binary_search(&(tile.x, tile.z, tile.level)).is_ok() {
                    Some(SQ_BLOCKED)
                } else {
                    collision_at(&world.collision, tile)
                }
            };
            let candidate_bytes = candidates
                .capacity()
                .checked_mul(std::mem::size_of::<WorldTile>())
                .unwrap();
            let mut yield_unit = YieldOnce::default;
            let future = retain_arrival_candidates_cooperative(
                &mut candidates,
                case.destination,
                case.radius,
                bounds,
                &collision,
                &mut yield_unit,
            );
            let future_bytes = std::mem::size_of_val(&future);
            let mut pending_polls = 0u64;
            let mut maximum_poll_ns = 0;
            let started = Instant::now();
            let allocation = allocation_counter::measure(|| {
                let mut future = Box::pin(future);
                let mut context = Context::from_waker(Waker::noop());
                loop {
                    let poll_started = Instant::now();
                    let polled = future.as_mut().poll(&mut context);
                    maximum_poll_ns = maximum_poll_ns.max(poll_started.elapsed().as_nanos());
                    match polled {
                        Poll::Ready(()) => break,
                        Poll::Pending => pending_polls += 1,
                    }
                }
            });
            let elapsed_ns = started.elapsed().as_nanos();
            assert_eq!(allocation.bytes_current, 0, "scratch survived {name}");
            let peak_bytes = usize::try_from(allocation.bytes_max)
                .unwrap()
                .checked_add(candidate_bytes)
                .unwrap();
            assert!(
                peak_bytes <= ROUTE_CAP_BYTES,
                "arrival exceeds Route cap: {name}"
            );
            if case.input_count != 0 {
                assert!(pending_polls != 0, "the driver never yielded: {name}");
            }
            assert_eq!(candidates.len(), case.output_count, "{revision}/{name}");
            let actual = encode_tiles(&candidates);
            assert_eq!(actual, expected, "ordered bytes differ: {revision}/{name}");
            let case_output = destination.join(name);
            std::fs::create_dir(&case_output).unwrap();
            std::fs::write(case_output.join("case.json"), metadata).unwrap();
            std::fs::write(case_output.join("input.bin"), &input).unwrap();
            std::fs::write(case_output.join("output.bin"), actual).unwrap();
            rows.push(serde_json::json!({
                "revision": revision,
                "case": name,
                "input_count": case.input_count,
                "output_count": case.output_count,
                "pending_polls": pending_polls,
                "maximum_poll_ns": maximum_poll_ns,
                "elapsed_ns": elapsed_ns,
                "candidate_storage_bytes": candidate_bytes,
                "future_layout_bytes": future_bytes,
                "arrival_heap_peak_bytes": allocation.bytes_max,
                "total_arrival_peak_bytes": peak_bytes,
                "retained_scratch_bytes": allocation.bytes_current,
            }));
            if name == "Edgeville" {
                let mut cancelled = decode_tiles(&input);
                let mut yield_unit = YieldOnce::default;
                let allocation = allocation_counter::measure(|| {
                    let mut future = Box::pin(retain_arrival_candidates_cooperative(
                        &mut cancelled,
                        case.destination,
                        case.radius,
                        bounds,
                        &collision,
                        &mut yield_unit,
                    ));
                    let mut context = Context::from_waker(Waker::noop());
                    for _ in 0..64 {
                        assert!(future.as_mut().poll(&mut context).is_pending());
                    }
                });
                assert_eq!(allocation.bytes_current, 0, "cancelled scratch survived");
                cancelled_cases += 1;
            }
        }
    }
    assert_eq!(rows.len(), 208);
    assert_eq!(cancelled_cases, 2);
    let report = serde_json::json!({
        "cases": rows,
        "cancelled_cases": cancelled_cases,
        "all_ordered_outputs_match_saved_baseline": true,
        "yield_driver": "Every producer checkpoint returns Pending once.",
        "measurement_boundary": "The peak includes the pinned producer future and all arrival scratch, plus the existing candidate Vec. Pack decoding, callback fixture storage, capture, search, and host result owners are outside this isolated arrival measurement.",
    });
    std::fs::write(
        output.join("parity.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
}
