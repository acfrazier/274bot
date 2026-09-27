//! Find-only 289 navigation corpus for the release performance receipt.
//!
//! Pack decoding happens once before any timed case. Each case discloses its
//! first (cold-search) duration separately from warm repetitions. Search
//! scratch is reported as allocated entry capacities, not invented byte
//! precision for `HashMap` buckets.

use api::snapshot::WorldTile;
use nav::router::{find_first_with, find_many_with, FindOptions, SearchCapacities};
use nav::world::NavWorld;
use nav::world_state::WorldState;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Instant;

#[derive(Clone, Copy, Default)]
struct PeakScratch {
    distances: usize,
    predecessors: usize,
    settled: usize,
    heap: usize,
    reverse: usize,
    reverse_queue: usize,
}

impl PeakScratch {
    fn observe(&mut self, value: SearchCapacities) {
        self.distances = self.distances.max(value.distances);
        self.predecessors = self.predecessors.max(value.predecessors);
        self.settled = self.settled.max(value.settled);
        self.heap = self.heap.max(value.heap);
        self.reverse = self.reverse.max(value.reverse);
        self.reverse_queue = self.reverse_queue.max(value.reverse_queue);
    }

    fn to_json(self) -> Value {
        json!({
            "unit": "allocated_entry_capacity",
            "distances": self.distances,
            "predecessors": self.predecessors,
            "settled": self.settled,
            "heap": self.heap,
            "reverse": self.reverse,
            "reverse_queue": self.reverse_queue,
            "sum_entries": self.distances + self.predecessors + self.settled
                + self.heap + self.reverse + self.reverse_queue,
        })
    }
}

struct Observation {
    outcome: String,
    settled: usize,
    scratch: SearchCapacities,
}

fn tile(x: i32, z: i32, level: i32) -> WorldTile {
    WorldTile { x, z, level }
}

fn percentile(sorted: &[f64], percentile: f64) -> Option<f64> {
    if sorted.is_empty() {
        return None;
    }
    let index = ((sorted.len() as f64 * percentile).ceil() as usize).saturating_sub(1);
    Some(sorted[index])
}

fn measure_case(name: &str, iterations: usize, mut run: impl FnMut() -> Observation) -> Value {
    let mut times_ms = Vec::with_capacity(iterations);
    let mut outcomes = Vec::with_capacity(iterations);
    let mut settled_peak = 0usize;
    let mut scratch = PeakScratch::default();
    for _ in 0..iterations {
        let started = Instant::now();
        let observation = run();
        times_ms.push(started.elapsed().as_secs_f64() * 1000.0);
        outcomes.push(observation.outcome);
        settled_peak = settled_peak.max(observation.settled);
        scratch.observe(observation.scratch);
    }
    let cold_ms = times_ms.first().copied();
    let mut warm_ms = times_ms.get(1..).unwrap_or(&[]).to_vec();
    warm_ms.sort_by(f64::total_cmp);
    json!({
        "name": name,
        "iterations": iterations,
        "cold_find_ms": cold_ms,
        "warm_find_ms": {
            "p50": percentile(&warm_ms, 0.50),
            "p95": percentile(&warm_ms, 0.95),
            "samples": warm_ms.len(),
        },
        "outcomes": outcomes,
        "peak_settled_nodes": settled_peak,
        "peak_scratch": scratch.to_json(),
    })
}

fn rich_289_state() -> WorldState {
    WorldState {
        map_members: true,
        quests: HashSet::from([
            "Prince Ali Rescue".into(),
            "Rune Mysteries".into(),
            "Lost City".into(),
            "Shilo Village".into(),
            "Tree Gnome Village".into(),
            "The Grand Tree".into(),
        ]),
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

fn parse_args() -> Result<(PathBuf, usize), String> {
    let mut args = std::env::args().skip(1);
    let mut pack = None;
    let mut iterations = 11usize;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--pack" => pack = Some(PathBuf::from(args.next().ok_or("--pack needs a path")?)),
            "--iterations" => {
                iterations = args
                    .next()
                    .ok_or("--iterations needs a value")?
                    .parse()
                    .map_err(|_| "invalid --iterations")?;
                if iterations < 2 {
                    return Err("--iterations must be at least 2".into());
                }
            }
            _ => return Err(format!("unknown argument {arg}")),
        }
    }
    Ok((pack.ok_or("--pack is required")?, iterations))
}

fn main() -> Result<(), String> {
    let (pack, iterations) = parse_args()?;
    let decode_started = Instant::now();
    let world = NavWorld::load_pack(&pack).map_err(|error| format!("load pack: {error:?}"))?;
    let decode_ms = decode_started.elapsed().as_secs_f64() * 1000.0;
    let game_data = api::game_data::for_revision(client::io::ClientRevision::R289)?;
    world
        .bind_named_bank_facts(&game_data)
        .map_err(|error| format!("bind named bank facts: {error:?}"))?;
    let empty = WorldState::empty();
    let rich = rich_289_state();

    let reachable_target = [tile(2965, 3379, 0)];
    let reachable = measure_case("reachable_lumbridge_falador", iterations, || {
        let search = find_many_with(
            &world.collision,
            &world.graph,
            tile(3222, 3218, 0),
            &reachable_target,
            FindOptions::default(),
            &empty,
        );
        Observation {
            outcome: format!("{:?}", search.results()[0]),
            settled: search.settled(),
            scratch: search.scratch_capacities(),
        }
    });

    let bank_targets: Vec<_> = world
        .named_bank_facts()
        .ok_or("named bank facts unavailable after binding")?
        .banks()
        .iter()
        .filter(|bank| bank.routable)
        .map(|bank| bank.tile)
        .collect();
    if bank_targets.is_empty() {
        return Err("289 corpus has no routable bank stands".into());
    }
    let bank_pick = measure_case("bank_pick_all_stands", iterations, || {
        let search = find_many_with(
            &world.collision,
            &world.graph,
            tile(3016, 9840, 0),
            &bank_targets,
            FindOptions::default(),
            &rich,
        );
        let reached = search
            .results()
            .iter()
            .filter(|result| result.is_ok())
            .count();
        Observation {
            outcome: format!("reached:{reached}/{}", bank_targets.len()),
            settled: search.settled(),
            scratch: search.scratch_capacities(),
        }
    });

    let radius_center = tile(2553, 3406, 0);
    let mut radius_targets = Vec::new();
    for x in radius_center.x - 10..=radius_center.x + 10 {
        for z in radius_center.z - 10..=radius_center.z + 10 {
            let candidate = tile(x, z, 0);
            if world.collision.standable(candidate) {
                radius_targets.push(candidate);
            }
        }
    }
    let radius = measure_case("radius_walk_candidates", iterations, || {
        let search = find_first_with(
            &world.collision,
            &world.graph,
            tile(2615, 3332, 0),
            &radius_targets,
            FindOptions::default(),
            &rich,
        );
        Observation {
            outcome: format!("{:?}", search.route().map(|route| route.dest)),
            settled: search.settled(),
            scratch: search.scratch_capacities(),
        }
    });

    let teleport_target = [tile(2817, 3443, 0)];
    let teleport = measure_case("teleports_on_real_state", iterations, || {
        let search = find_many_with(
            &world.collision,
            &world.graph,
            tile(3222, 3218, 0),
            &teleport_target,
            FindOptions {
                allow_teleports: true,
                ..FindOptions::default()
            },
            &rich,
        );
        Observation {
            outcome: format!("{:?}", search.results()[0]),
            settled: search.settled(),
            scratch: search.scratch_capacities(),
        }
    });

    let unreachable_target = [tile(2817, 3443, 0)];
    let unreachable = measure_case("unreachable_fail_closed", iterations, || {
        let search = find_many_with(
            &world.collision,
            &world.graph,
            tile(3222, 3218, 0),
            &unreachable_target,
            FindOptions::default(),
            &empty,
        );
        Observation {
            outcome: format!("{:?}", search.results()[0]),
            settled: search.settled(),
            scratch: search.scratch_capacities(),
        }
    });

    println!(
        "{}",
        json!({
            "schema": "274bot-nav-perf-v1",
            "revision": 289,
            "pack": pack,
            "pack_decode_ms_excluded_from_find": decode_ms,
            "cold_warm_disclosure": "first search per case is cold; p50/p95 use subsequent searches on the same decoded world",
            "cases": [reachable, bank_pick, radius, teleport, unreachable],
        })
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::percentile;

    #[test]
    fn percentile_uses_nearest_rank() {
        let samples: Vec<f64> = (1..=10).map(f64::from).collect();
        assert_eq!(percentile(&samples, 0.50), Some(5.0));
        assert_eq!(percentile(&samples, 0.95), Some(10.0));
        assert_eq!(percentile(&samples, 1.0), Some(10.0));
        assert_eq!(percentile(&[], 0.50), None);
    }
}
