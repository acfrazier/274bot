#![cfg(not(debug_assertions))]

//! Opt-in release measurement for the existing packed arrival-candidate filter.
//!
//! The dense cases reproduce `host-play::script_nav::approach_tiles` candidate
//! construction and order, before its final sort. Exact-replay cases use the
//! same real-pack candidates, but the callback deliberately marks selected
//! candidate tiles blocked. That test-only input forces the filter's existing
//! replay branch; its exact reference uses the same callback. No production API
//! or collision semantics are changed.

use std::path::{Path, PathBuf};
use std::time::Instant;

use api::query::{is_arrived, retain_arrival_candidates, CollisionArrivalProbe};
use api::snapshot::WorldTile;
use client::dash3d::CollisionFlag;
use nav::collision::WorldCollision;
use nav::world::NavWorld;

const RADIUS: i32 = 104;
const CANDIDATE_SIDE: usize = (RADIUS as usize * 2) + 1;
const CANDIDATE_CELLS: usize = CANDIDATE_SIDE * CANDIDATE_SIDE;
const FILTER_EXTENT: usize = (RADIUS as usize * 3) + 1;
const FILTER_SIDE: usize = FILTER_EXTENT * 2 + 1;
const FILTER_CELLS: usize = FILTER_SIDE * FILTER_SIDE;
const EXPANSION_BUDGET: usize = CANDIDATE_CELLS;
const SAMPLE_COUNT: usize = 101;
const REPLAY_COHORT_SIZE: usize = 64;
const SQ_BLOCKED: i32 = CollisionFlag::SQ_BLOCKED as i32;

#[derive(Clone, Copy)]
struct PackInput {
    revision_label: &'static str,
    env_name: &'static str,
}

const PACKS: [PackInput; 2] = [
    PackInput {
        revision_label: "274",
        env_name: "T5W_ARRIVAL_NAVPACK_274",
    },
    PackInput {
        revision_label: "289",
        env_name: "T5W_ARRIVAL_NAVPACK_289",
    },
];

#[derive(Clone, Copy)]
struct Destination {
    name: &'static str,
    tile: WorldTile,
}

const DENSE_DESTINATIONS: [Destination; 6] = [
    Destination {
        name: "Lumbridge",
        tile: WorldTile {
            x: 3222,
            z: 3218,
            level: 0,
        },
    },
    Destination {
        name: "Varrock",
        tile: WorldTile {
            x: 3210,
            z: 3424,
            level: 0,
        },
    },
    Destination {
        name: "Falador",
        tile: WorldTile {
            x: 2965,
            z: 3378,
            level: 0,
        },
    },
    Destination {
        name: "Draynor",
        tile: WorldTile {
            x: 3081,
            z: 3250,
            level: 0,
        },
    },
    Destination {
        name: "Port Sarim",
        tile: WorldTile {
            x: 3025,
            z: 3209,
            level: 0,
        },
    },
    Destination {
        name: "Edgeville",
        tile: WorldTile {
            x: 3087,
            z: 3496,
            level: 0,
        },
    },
];

#[derive(Clone, Copy)]
enum CaseKind {
    DenseCity,
    ExactReplaySingle,
    ExactReplayCohort,
}

impl CaseKind {
    fn name(self) -> &'static str {
        match self {
            Self::DenseCity => "dense_city_max_radius",
            Self::ExactReplaySingle => "exact_replay_single_max_radius",
            Self::ExactReplayCohort => "exact_replay_cohort64_max_radius",
        }
    }

    fn replay_limit(self) -> Option<usize> {
        match self {
            Self::DenseCity => None,
            Self::ExactReplaySingle => Some(1),
            Self::ExactReplayCohort => Some(REPLAY_COHORT_SIZE),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
struct TileKey {
    x: i32,
    z: i32,
    level: i32,
}

impl From<WorldTile> for TileKey {
    fn from(tile: WorldTile) -> Self {
        Self {
            x: tile.x,
            z: tile.z,
            level: tile.level,
        }
    }
}

#[derive(Clone, Copy)]
struct Sample {
    elapsed_ns: u128,
    allocation_peak_bytes: u128,
    allocation_current_bytes: u128,
    candidate_storage_bytes: usize,
    output_fingerprint: u64,
}

struct Distribution {
    p50: u128,
    p99: u128,
    max: u128,
}

impl Distribution {
    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "p50": self.p50,
            "p99": self.p99,
            "max": self.max,
        })
    }
}

fn quantiles(values: impl IntoIterator<Item = u128>) -> Distribution {
    let mut values: Vec<_> = values.into_iter().collect();
    assert_eq!(values.len(), SAMPLE_COUNT);
    values.sort_unstable();
    let nearest_rank = |percentile: usize| (percentile * values.len()).div_ceil(100).max(1) - 1;
    Distribution {
        p50: values[nearest_rank(50)],
        p99: values[nearest_rank(99)],
        max: values[values.len() - 1],
    }
}

fn packed_collision_at(collision: &WorldCollision, tile: WorldTile) -> Option<i32> {
    // Match script_nav::approach_tiles' packed callback, including its
    // bounds/level checks and `walkable_word` representation.
    let x = usize::try_from(tile.x.checked_sub(collision.origin.x)?).ok()?;
    let z = usize::try_from(tile.z.checked_sub(collision.origin.z)?).ok()?;
    ((0..=3).contains(&tile.level) && x < collision.width && z < collision.height)
        .then(|| collision.walkable_word(tile.x, tile.z, tile.level) as i32)
}

fn chebyshev_distance(a: WorldTile, b: WorldTile) -> u32 {
    a.x.abs_diff(b.x).max(a.z.abs_diff(b.z))
}

fn producer_candidates(world: &NavWorld, destination: WorldTile, kind: CaseKind) -> Vec<WorldTile> {
    let mut candidates = Vec::new();
    let replay_limit = kind.replay_limit();

    // This is the producer's pre-filter order: dx outer, dz inner, then
    // standability. The native producer sorts only after retain_arrival_candidates.
    'x: for dx in -RADIUS..=RADIUS {
        for dz in -RADIUS..=RADIUS {
            let tile = WorldTile {
                x: destination.x + dx,
                z: destination.z + dz,
                level: destination.level,
            };
            if !world.collision.standable(tile) {
                continue;
            }

            if replay_limit.is_some() {
                if chebyshev_distance(tile, destination) != RADIUS as u32 {
                    continue;
                }
                let flags = packed_collision_at(&world.collision, tile)
                    .expect("a real standable candidate must be probeable");
                assert_eq!(flags & SQ_BLOCKED, 0);
            }

            candidates.push(tile);
            if replay_limit.is_some_and(|limit| candidates.len() == limit) {
                break 'x;
            }
        }
    }

    if let Some(limit) = replay_limit {
        assert_eq!(
            candidates.len(),
            limit,
            "{} needs {limit} real standable ring candidates at {:?}",
            kind.name(),
            destination
        );
    } else {
        assert!(
            candidates.len() >= 1_000,
            "dense destination {:?} produced only {} standable candidates",
            destination,
            candidates.len()
        );
    }
    candidates
}

fn fingerprint_tile(hash: &mut u64, tile: WorldTile) {
    const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
    for word in [tile.x as u32, tile.z as u32, tile.level as u32] {
        *hash ^= u64::from(word);
        *hash = hash.wrapping_mul(FNV_PRIME);
    }
}

fn fingerprint(tiles: &[WorldTile]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325;
    for &tile in tiles {
        fingerprint_tile(&mut hash, tile);
    }
    hash
}

fn exact_reference(
    candidates: &[WorldTile],
    destination: WorldTile,
    collision_at: &impl Fn(WorldTile) -> Option<i32>,
) -> (usize, u64) {
    let probe = CollisionArrivalProbe::new(|tile| collision_at(tile));
    let mut count = 0;
    let mut hash = 0xcbf2_9ce4_8422_2325;
    for &candidate in candidates {
        if is_arrived(candidate, destination, RADIUS, || &probe) {
            count += 1;
            fingerprint_tile(&mut hash, candidate);
        }
    }
    (count, hash)
}

fn source_revision_evidence(kind: CaseKind, forced_count: usize) -> &'static str {
    match kind {
        CaseKind::DenseCity => {
            "no internal replay counter is exposed; this real-collision dense case reports no replay count"
        }
        CaseKind::ExactReplaySingle | CaseKind::ExactReplayCohort => {
            assert!(forced_count > 0);
            "test callback returns Some(SQ_BLOCKED) for each listed in-window candidate; reach.rs queues each at the !flags_open branch (around 1733-1738), then the replay loop calls is_arrived (around 1967-1969). This is source-path proof, not an added runtime counter."
        }
    }
}

fn run_case(
    revision: PackInput,
    pack_path: &Path,
    world: &NavWorld,
    destination: Destination,
    kind: CaseKind,
) {
    packed_collision_at(&world.collision, destination.tile).unwrap_or_else(|| {
        panic!(
            "{} destination {:?} is outside the pack",
            destination.name, destination.tile
        )
    });

    let mut samples = Vec::with_capacity(SAMPLE_COUNT);
    let mut expected_output = None;
    let mut expected_reference_output = None;
    let mut expected_input_fingerprint = None;
    let mut stable_candidate_storage = None;
    let guaranteed_replay_count = kind.replay_limit().unwrap_or(0);
    for _ in 0..SAMPLE_COUNT {
        // Pack decoding and candidate construction are outside both the timer
        // and allocation counter. A fresh Vec is passed to every sample so its
        // own allocation is the only pre-existing candidate buffer.
        let mut candidates = producer_candidates(world, destination.tile, kind);
        let input_count = candidates.len();
        let input_fingerprint = fingerprint(&candidates);
        assert_eq!(
            expected_input_fingerprint.get_or_insert(input_fingerprint),
            &input_fingerprint,
            "producer order/input changed between samples"
        );
        let candidate_storage_bytes = candidates.capacity() * std::mem::size_of::<WorldTile>();
        assert_eq!(
            stable_candidate_storage.get_or_insert(candidate_storage_bytes),
            &candidate_storage_bytes,
            "candidate Vec capacity changed between samples"
        );

        // Exact-replay cases override only their chosen candidate words in the
        // callback. This gives deterministic evidence for the existing
        // blocked-candidate replay branch while retaining real packed data for
        // every other coordinate. The fixed-size stack keys add no measured or
        // resident heap allocation.
        let mut forced_keys = [TileKey::default(); REPLAY_COHORT_SIZE];
        let forced_count = guaranteed_replay_count;
        for (slot, &tile) in candidates.iter().take(forced_count).enumerate() {
            forced_keys[slot] = TileKey::from(tile);
        }
        let forced_keys = &forced_keys[..forced_count];
        assert!(forced_keys.windows(2).all(|pair| pair[0] < pair[1]));

        let collision_at = |tile| {
            if forced_keys.binary_search(&TileKey::from(tile)).is_ok() {
                Some(SQ_BLOCKED)
            } else {
                packed_collision_at(&world.collision, tile)
            }
        };
        let forced_replay_count = candidates
            .iter()
            .filter(|&&tile| {
                tile != destination.tile
                    && tile.level == destination.tile.level
                    && chebyshev_distance(tile, destination.tile) <= RADIUS as u32
                    && collision_at(tile).is_some_and(|flags| flags & SQ_BLOCKED != 0)
            })
            .count();
        if forced_count > 0 {
            assert_eq!(forced_replay_count, input_count);
        } else {
            assert_eq!(forced_replay_count, 0);
        }

        if forced_count > 0 && expected_reference_output.is_none() {
            expected_reference_output = Some(exact_reference(
                &candidates,
                destination.tile,
                &collision_at,
            ));
        }
        let reference = expected_reference_output;

        let mut elapsed_ns = 0;
        let allocations = allocation_counter::measure(|| {
            let started = Instant::now();
            retain_arrival_candidates(&mut candidates, destination.tile, RADIUS, &collision_at);
            elapsed_ns = started.elapsed().as_nanos();
        });

        assert_eq!(
            candidates.capacity(),
            candidate_storage_bytes / std::mem::size_of::<WorldTile>(),
            "retain must preserve the pre-owned candidate buffer"
        );
        assert_eq!(
            allocations.bytes_current, 0,
            "all filter scratch must be dropped before the measured call returns"
        );
        let output_count = candidates.len();
        let output_fingerprint = fingerprint(&candidates);
        if let Some((reference_count, reference_fingerprint)) = reference {
            assert_eq!(output_count, reference_count, "exact output count diverged");
            assert_eq!(
                output_fingerprint, reference_fingerprint,
                "exact candidate output/order diverged from is_arrived"
            );
        }
        let sample_outcome = (
            input_count,
            output_count,
            input_fingerprint,
            output_fingerprint,
        );
        if let Some(expected) = expected_output {
            assert_eq!(
                sample_outcome, expected,
                "filter output changed between samples"
            );
        } else {
            expected_output = Some(sample_outcome);
        }

        samples.push(Sample {
            elapsed_ns,
            allocation_peak_bytes: allocations.bytes_max as u128,
            allocation_current_bytes: allocations.bytes_current as u128,
            candidate_storage_bytes,
            output_fingerprint,
        });
    }

    let input_count = expected_output.expect("at least one sample").0;
    let output_count = expected_output.expect("at least one sample").1;
    let input_fingerprint = expected_input_fingerprint.expect("at least one sample");
    let output_fingerprint = samples[0].output_fingerprint;
    let candidate_storage_bytes = samples[0].candidate_storage_bytes;
    let elapsed = quantiles(samples.iter().map(|sample| sample.elapsed_ns));
    let allocation_peak = quantiles(samples.iter().map(|sample| sample.allocation_peak_bytes));
    let allocation_current =
        quantiles(samples.iter().map(|sample| sample.allocation_current_bytes));
    let total_peak = quantiles(
        samples
            .iter()
            .map(|sample| sample.allocation_peak_bytes + sample.candidate_storage_bytes as u128),
    );
    let total_current =
        quantiles(samples.iter().map(|sample| {
            sample.allocation_current_bytes + sample.candidate_storage_bytes as u128
        }));
    let exact_reference = expected_reference_output;

    println!(
        "{}",
        serde_json::json!({
            "measurement": "arrival_candidate_qualification",
            "revision": revision.revision_label,
            "revision_evidence": format!("explicit {} environment label; navpack does not encode the game-content revision", revision.env_name),
            "pack_path": pack_path.display().to_string(),
            "destination": {
                "name": destination.name,
                "x": destination.tile.x,
                "z": destination.tile.z,
                "level": destination.tile.level,
            },
            "case": kind.name(),
            "radius": RADIUS,
            "candidate_scan_square": {
                "side": CANDIDATE_SIDE,
                "cells": CANDIDATE_CELLS,
            },
            "candidate_input_count": input_count,
            "candidate_output_count": output_count,
            "candidate_order": "dx ascending outer loop, dz ascending inner loop, standable-only; same pre-sort order as host-play::script_nav::approach_tiles",
            "candidate_input_fingerprint_fnv1a64": format!("0x{input_fingerprint:016x}"),
            "candidate_output_fingerprint_fnv1a64": format!("0x{output_fingerprint:016x}"),
            "exact_reference_output_count": exact_reference.map(|value| value.0),
            "exact_reference_output_fingerprint_fnv1a64": exact_reference.map(|value| format!("0x{:016x}", value.1)),
            "forced_replay_candidate_count": guaranteed_replay_count,
            "replay_evidence": source_revision_evidence(kind, guaranteed_replay_count),
            "test_only_collision_override": if guaranteed_replay_count > 0 { "Some(SQ_BLOCKED) on selected real standable radius-104 candidates; all other words come from the real packed grid" } else { "none; real packed collision words only" },
            "filter_dense_square": {
                "extent_each_side": FILTER_EXTENT,
                "side": FILTER_SIDE,
                "cells": FILTER_CELLS,
            },
            "filter_expansion_budget": EXPANSION_BUDGET,
            "sample_count": SAMPLE_COUNT,
            "elapsed_ns": elapsed.json(),
            "allocation_bytes_max": allocation_peak.json(),
            "allocation_bytes_current": allocation_current.json(),
            "candidate_input_storage_bytes": candidate_storage_bytes,
            "candidate_output_storage_bytes": candidate_storage_bytes,
            "total_peak_heap_bytes": total_peak.json(),
            "total_retained_heap_bytes": total_current.json(),
            "ownership_boundary": "NavWorld::load_pack and candidate construction are outside measurement. Each input Vec is built before allocation_counter::measure and mutated in place. Timer encloses only retain_arrival_candidates. Allocation bytes_max/current count only heap allocations made inside that call; candidate Vec capacity is reported separately and added to total peak/current. Exact reference validation and JSON/statistics are outside the measured interval.",
            "collision_source": "WorldCollision::walkable_word with the exact bounds/level checks used by host-play::script_nav::approach_tiles; packed face bits plus blocked bit reconstruct the caller's derived flags, so raw .navflags is not needed for this filter path.",
        })
    );
}

fn required_pack_path(env_name: &str) -> PathBuf {
    PathBuf::from(
        std::env::var_os(env_name)
            .unwrap_or_else(|| panic!("{env_name} must name an explicit real navpack")),
    )
}

#[test]
#[ignore = "explicit opt-in release-only measurement; set both T5W_ARRIVAL_NAVPACK_274 and T5W_ARRIVAL_NAVPACK_289"]
fn measure_real_max_radius_arrival_candidates() {
    // This file is compiled only without debug assertions. The source
    // arithmetic remains explicit: extent = 104*3+1 = 313, side = 2*313+1 = 627.

    for revision in PACKS {
        let path = required_pack_path(revision.env_name);
        let world = NavWorld::load_pack(&path).unwrap_or_else(|error| {
            panic!(
                "could not load {} pack {}: {error:?}",
                revision.revision_label,
                path.display()
            )
        });

        for destination in DENSE_DESTINATIONS {
            run_case(revision, &path, &world, destination, CaseKind::DenseCity);
        }

        // Keep one maximum-radius exact-replay singleton and cohort for both
        // real revisions. Their candidate tiles are from Varrock's actual
        // standable radius-104 producer sequence; the test-only blocked words
        // make the existing direct-to-replay branch provable without telemetry.
        let varrock = DENSE_DESTINATIONS[1];
        run_case(
            revision,
            &path,
            &world,
            varrock,
            CaseKind::ExactReplaySingle,
        );
        run_case(
            revision,
            &path,
            &world,
            varrock,
            CaseKind::ExactReplayCohort,
        );
    }
}

fn write_ordered_tiles(path: &Path, tiles: &[WorldTile]) {
    use std::io::{BufWriter, Write};

    let mut output = BufWriter::new(std::fs::File::create(path).unwrap());
    for tile in tiles {
        for word in [tile.x, tile.z, tile.level] {
            output.write_all(&word.to_le_bytes()).unwrap();
        }
    }
    output.flush().unwrap();
}

fn capture_parity_case(
    root: &Path,
    world: &NavWorld,
    name: &str,
    destination: WorldTile,
    radius: i32,
    mut candidates: Vec<WorldTile>,
    forced_blocked: &[WorldTile],
) {
    let directory = root.join(name);
    std::fs::create_dir(&directory).unwrap();
    write_ordered_tiles(&directory.join("input.bin"), &candidates);
    let mut blocked: Vec<_> = forced_blocked.iter().copied().map(TileKey::from).collect();
    blocked.sort_unstable();
    let collision_at = |tile: WorldTile| {
        if blocked.binary_search(&TileKey::from(tile)).is_ok() {
            Some(SQ_BLOCKED)
        } else {
            packed_collision_at(&world.collision, tile)
        }
    };
    let input_count = candidates.len();
    retain_arrival_candidates(&mut candidates, destination, radius, &collision_at);
    write_ordered_tiles(&directory.join("output.bin"), &candidates);
    let metadata = serde_json::json!({
        "destination": destination,
        "radius": radius,
        "input_count": input_count,
        "output_count": candidates.len(),
        "forced_blocked_candidates": forced_blocked,
        "tile_encoding": "ordered x,z,level signed-i32 little-endian, 12 bytes per tile",
        "collision_domain": {
            "origin": world.collision.origin,
            "width": world.collision.width,
            "height": world.collision.height,
            "outside": "None",
        },
    });
    std::fs::write(
        directory.join("case.json"),
        serde_json::to_vec_pretty(&metadata).unwrap(),
    )
    .unwrap();
}

fn broad_candidates(world: &NavWorld, destination: WorldTile, radius: i32) -> Vec<WorldTile> {
    let mut candidates = Vec::new();
    for dx in -radius..=radius {
        for dz in -radius..=radius {
            let tile = WorldTile {
                x: destination.x + dx,
                z: destination.z + dz,
                level: destination.level,
            };
            if world.collision.standable(tile) {
                candidates.push(tile);
            }
        }
    }
    candidates
}

#[test]
#[ignore = "capture ordered real-kernel parity corpus; set both navpack variables and T5W_ARRIVAL_CAPTURE_DIR"]
fn capture_real_arrival_candidate_corpus() {
    let output = required_pack_path("T5W_ARRIVAL_CAPTURE_DIR");
    assert!(!output.exists(), "capture output must be fresh");
    std::fs::create_dir_all(&output).unwrap();
    for revision in PACKS {
        let path = required_pack_path(revision.env_name);
        let world = NavWorld::load_pack(&path).unwrap();
        let root = output.join(revision.revision_label);
        std::fs::create_dir(&root).unwrap();
        std::fs::write(
            root.join("pack.sha256"),
            nav::manifest::hash_file(&path).unwrap(),
        )
        .unwrap();
        let mut cases = 0usize;
        for destination in DENSE_DESTINATIONS {
            capture_parity_case(
                &root,
                &world,
                destination.name,
                destination.tile,
                RADIUS,
                producer_candidates(&world, destination.tile, CaseKind::DenseCity),
                &[],
            );
            cases += 1;
            for (index, (dx, dz)) in [(-104, -104), (-104, 104), (104, -104), (104, 104)]
                .into_iter()
                .enumerate()
            {
                let tile = WorldTile {
                    x: destination.tile.x + dx,
                    z: destination.tile.z + dz,
                    level: destination.tile.level,
                };
                capture_parity_case(
                    &root,
                    &world,
                    &format!("{}-neighbour-{index}", destination.name),
                    tile,
                    RADIUS,
                    broad_candidates(&world, tile, RADIUS),
                    &[],
                );
                cases += 1;
            }
        }
        for kind in [CaseKind::ExactReplaySingle, CaseKind::ExactReplayCohort] {
            let destination = DENSE_DESTINATIONS[1].tile;
            let candidates = producer_candidates(&world, destination, kind);
            let forced = candidates.clone();
            capture_parity_case(
                &root,
                &world,
                kind.name(),
                destination,
                RADIUS,
                candidates,
                &forced,
            );
            cases += 1;
        }
        for index in 0..64usize {
            let tile = WorldTile {
                x: world.collision.origin.x
                    + (world.collision.width * (2 * (index % 8) + 1) / 16) as i32,
                z: world.collision.origin.z
                    + (world.collision.height * (2 * (index / 8) + 1) / 16) as i32,
                level: (index / 16) as i32,
            };
            let radius = if index % 2 == 0 { RADIUS } else { 32 };
            capture_parity_case(
                &root,
                &world,
                &format!("grid-{index:02}"),
                tile,
                radius,
                broad_candidates(&world, tile, radius),
                &[],
            );
            cases += 1;
        }
        for (index, (x, z)) in [
            (0, 0),
            (world.collision.width - 1, 0),
            (0, world.collision.height - 1),
            (world.collision.width - 1, world.collision.height - 1),
        ]
        .into_iter()
        .enumerate()
        {
            for level in [0, 3] {
                let tile = WorldTile {
                    x: world.collision.origin.x + x as i32,
                    z: world.collision.origin.z + z as i32,
                    level,
                };
                capture_parity_case(
                    &root,
                    &world,
                    &format!("boundary-{index}-level-{level}"),
                    tile,
                    RADIUS,
                    broad_candidates(&world, tile, RADIUS),
                    &[],
                );
                cases += 1;
            }
        }
        assert_eq!(cases, 104);
        eprintln!(
            "arrival ordered real corpus: revision={} cases={} output={}",
            revision.revision_label,
            cases,
            root.display()
        );
    }
}
