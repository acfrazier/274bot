//! Find-only 289 navigation corpus for the release performance receipt.
//!
//! Pack decoding happens once before any timed case. Each case discloses its
//! first (cold-search) duration separately from warm repetitions. Search
//! scratch is reported as allocated entry capacities, not invented byte
//! precision for `HashMap` buckets.
//! `NAV_PERF_COUNTERS_ONLY=1` disables every clock read and reports only
//! deterministic outcomes, settled nodes and scratch capacities.

use api::snapshot::WorldTile;
use nav::router::{
    find_blocking_zones, find_first_with, find_many_with, FindOptions, RouteError, SearchCapacities,
};
use nav::world::NavWorld;
use nav::world_state::WorldState;
use nav::zones::ZoneExempt;
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
    zone_mask_words: usize,
}

impl PeakScratch {
    fn observe(&mut self, value: SearchCapacities) {
        self.distances = self.distances.max(value.distances);
        self.predecessors = self.predecessors.max(value.predecessors);
        self.settled = self.settled.max(value.settled);
        self.heap = self.heap.max(value.heap);
        self.reverse = self.reverse.max(value.reverse);
        self.reverse_queue = self.reverse_queue.max(value.reverse_queue);
        self.zone_mask_words = self.zone_mask_words.max(value.zone_mask_words);
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
            "zone_mask_words": self.zone_mask_words,
            "sum_entries": self.distances + self.predecessors + self.settled
                + self.heap + self.reverse + self.reverse_queue + self.zone_mask_words,
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

fn counters_only() -> bool {
    std::env::var("NAV_PERF_COUNTERS_ONLY").as_deref() == Ok("1")
}

fn measure_case(name: &str, iterations: usize, mut run: impl FnMut() -> Observation) -> Value {
    let timed = !counters_only();
    let mut times_ms = Vec::with_capacity(if timed { iterations } else { 0 });
    let mut outcomes = Vec::with_capacity(iterations);
    let mut settled_peak = 0usize;
    let mut scratch = PeakScratch::default();
    for _ in 0..iterations {
        let started = timed.then(Instant::now);
        let observation = run();
        if let Some(started) = started {
            times_ms.push(started.elapsed().as_secs_f64() * 1000.0);
        }
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
        combat_level: Some(126),
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
    let decode_started = (!counters_only()).then(Instant::now);
    let mut world = NavWorld::load_pack(&pack).map_err(|error| format!("load pack: {error:?}"))?;
    let decode_ms = decode_started.map(|started| started.elapsed().as_secs_f64() * 1000.0);
    let game_data = api::game_data::for_revision(client::io::ClientRevision::R289)?;
    world
        .bind_named_bank_facts(&game_data)
        .map_err(|error| format!("bind named bank facts: {error:?}"))?;
    // The original FLOOR corpus predates zone policy. Keep those five
    // controls zone-free rather than using a danger grant as a bypass.
    let zones = world.graph.zones.take();
    let empty = WorldState::empty();
    let rich = rich_289_state();
    let legacy = FindOptions {
        zones: ZoneExempt::all(),
        ..FindOptions::default()
    };

    let reachable_target = [tile(2965, 3379, 0)];
    let reachable = measure_case("reachable_lumbridge_falador", iterations, || {
        let search = find_many_with(
            &world.collision,
            &world.graph,
            tile(3222, 3218, 0),
            &reachable_target,
            legacy,
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
            legacy,
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
            legacy,
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
                ..legacy
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
            legacy,
            &empty,
        );
        Observation {
            outcome: format!("{:?}", search.results()[0]),
            settled: search.settled(),
            scratch: search.scratch_capacities(),
        }
    });

    world.graph.zones = zones;
    let mut zone_cases = Vec::new();
    for level in [126, 3] {
        let mut state = rich_289_state();
        state.combat_level = Some(level);
        zone_cases.push(measure_case(
            &format!("reachable_lumbridge_falador_zones_l{level}"),
            iterations,
            || {
                let search = find_many_with(
                    &world.collision,
                    &world.graph,
                    tile(3222, 3218, 0),
                    &reachable_target,
                    FindOptions::default(),
                    &state,
                );
                Observation {
                    outcome: format!("{:?}", search.results()[0]),
                    settled: search.settled(),
                    scratch: search.scratch_capacities(),
                }
            },
        ));
    }
    let mut low = rich_289_state();
    low.combat_level = Some(3);
    zone_cases.push(measure_case(
        "bank_pick_all_stands_zones_l3",
        iterations,
        || {
            let search = find_many_with(
                &world.collision,
                &world.graph,
                tile(3016, 9840, 0),
                &bank_targets,
                FindOptions::default(),
                &low,
            );
            let reached = search.results().iter().filter(|r| r.is_ok()).count();
            assert_eq!(
                (reached, bank_targets.len(), search.completion_partitions()),
                (17, 19, 0)
            );
            Observation {
                outcome: format!("reached:{reached}/{}", bank_targets.len()),
                settled: search.settled(),
                scratch: search.scratch_capacities(),
            }
        },
    ));
    zone_cases.push(measure_case(
        "radius_walk_candidates_zones_l3",
        iterations,
        || {
            let search = find_first_with(
                &world.collision,
                &world.graph,
                tile(2615, 3332, 0),
                &radius_targets,
                FindOptions::default(),
                &low,
            );
            Observation {
                outcome: format!("{:?}", search.route().map(|route| route.dest)),
                settled: search.settled(),
                scratch: search.scratch_capacities(),
            }
        },
    ));
    zone_cases.push(measure_case(
        "lumbridge_rellekka_refused_zones_l126",
        iterations,
        || {
            let from = tile(3222, 3218, 0);
            let to = tile(2664, 3664, 0);
            let search = find_first_with(
                &world.collision,
                &world.graph,
                from,
                &[to],
                FindOptions::default(),
                &rich,
            );
            assert_eq!(search.route().unwrap_err(), RouteError::NoPath);
            let blocked = find_blocking_zones(
                &world.collision,
                &world.graph,
                from,
                to,
                FindOptions::default(),
                &rich,
                &[],
            )
            .expect("zone refusal witness");
            let table = world.graph.zones.as_ref().expect("v12 zones");
            let names: Vec<_> = blocked.iter().map(|&key| table.name(key)).collect();
            // Preserve the actual diagnostic witness for cross-version
            // comparison; its shape belongs to the refusal implementation.
            assert!(!names.is_empty());
            Observation {
                outcome: format!("NoPath blocked:{names:?}"),
                settled: search.settled(),
                scratch: search.scratch_capacities(),
            }
        },
    ));
    zone_cases.push(measure_case(
        "taverley_catherby_zones_l126",
        iterations,
        || {
            let target = [tile(2809, 3440, 0)];
            let search = find_many_with(
                &world.collision,
                &world.graph,
                tile(2895, 3450, 0),
                &target,
                FindOptions::default(),
                &rich,
            );
            let route = search.route(0).expect("rich-state mountain detour");
            let tiles: usize = route
                .legs
                .iter()
                .map(|leg| match leg {
                    nav::router::Leg::Walk { tiles } => tiles.len(),
                    nav::router::Leg::Transport { .. } => 1,
                })
                .sum();
            assert_eq!((route.ticks, tiles), (341.0, 683));
            Observation {
                outcome: format!("{:?}", search.results()[0]),
                settled: search.settled(),
                scratch: search.scratch_capacities(),
            }
        },
    ));
    let mut live = WorldState::empty();
    live.map_members = true;
    live.combat_level = Some(50);
    live.inv.insert(995, 60);
    for (name, from, to, opts, state) in [
        (
            "danger_live_off",
            tile(2809, 3441, 0),
            tile(3103, 3163, 2),
            FindOptions::default(),
            &live,
        ),
        (
            "danger_live_grant",
            tile(2809, 3441, 0),
            tile(3103, 3163, 2),
            legacy,
            &live,
        ),
        (
            "falador_lumbridge_zones_l3",
            tile(2965, 3379, 0),
            tile(3222, 3218, 0),
            FindOptions::default(),
            &low,
        ),
        (
            "deep_zone_goal_teles_off",
            tile(2895, 3450, 0),
            tile(2852, 3493, 0),
            FindOptions::default(),
            &low,
        ),
        (
            "deep_zone_goal_teles_on",
            tile(2895, 3450, 0),
            tile(2852, 3493, 0),
            FindOptions {
                allow_teleports: true,
                ..FindOptions::default()
            },
            &low,
        ),
        (
            "all_off_control",
            tile(3222, 3218, 0),
            tile(2965, 3379, 0),
            FindOptions::default(),
            &empty,
        ),
    ] {
        zone_cases.push(measure_case(name, iterations, || {
            let search = find_first_with(&world.collision, &world.graph, from, &[to], opts, state);
            Observation {
                outcome: format!(
                    "{:?}",
                    search.route().map(|route| (route.dest, route.ticks))
                ),
                settled: search.settled(),
                scratch: search.scratch_capacities(),
            }
        }));
    }
    let mut cases = vec![reachable, bank_pick, radius, teleport, unreachable];
    cases.extend(zone_cases);

    println!(
        "{}",
        json!({
            "schema": "274bot-nav-perf-v2",
            "revision": 289,
            "pack": pack,
            "pack_decode_ms_excluded_from_find": decode_ms,
            "cold_warm_disclosure": "first search per case is cold; p50/p95 use subsequent searches on the same decoded world",
            "cases": cases,
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
