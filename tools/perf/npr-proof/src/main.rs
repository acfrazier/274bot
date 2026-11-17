//! Separate, full canonical NPR proof/original search workload. Not a W4 case.
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

#[cfg(feature = "api-observation")]
use api::prereq_knowledge::Observation;
use api::quest_progress::{EvidenceProvider, EvidenceStamp};
use api::selected::{InclusiveRange, QuestGate, RunKey, Truth};
use api::snapshot::WorldTile;
use nav::quest_gates::QuestEvidence;
use nav::router::{find_with, FindOptions, Leg, RouteError, NODE_BUDGET};
use nav::world::NavWorld;
use nav::world_state::WorldState;
use nav::zones::ZoneExempt;
use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};

#[cfg(target_os = "macos")]
mod footprint;

const INPUT_SHA256: &str = "a7792cb000aba1303e4ad2d87e977a646f46feccd1f88e3ef789910ac614da0a";
const FIXTURE_SHA256: &str = "554de14c680ccb2a53cc4cdf5469ccc4115943ee7e25db77756d4050b8f9bb42";
const QUERY_PAIRS: usize = 299;
type Pair = [[i32; 3]; 2];

#[derive(Debug)]
struct ProofEvidence;

impl ProofEvidence {
    fn truth(&self, gate: &QuestGate) -> Truth {
        let QuestGate::Window(window) = gate else {
            return Truth::Unknown;
        };
        let stage = match (window.quest.0.as_ref(), window.signal.0.as_ref()) {
            ("priestperil", "priestperil") => 61,
            ("druidspirit", "druidspirit") => 1,
            _ => return Truth::Unknown,
        };
        window.values.test(&InclusiveRange {
            min: Some(stage),
            max: Some(stage),
        })
    }
}

#[cfg(feature = "api-observation")]
impl EvidenceProvider for ProofEvidence {
    fn observe_gate(&self, gate: &QuestGate, required_after: EvidenceStamp) -> Option<Observation> {
        Some(Observation {
            truth: self.truth(gate),
            // Explicit corpus input at the requested floor, never a live observation.
            observed: Some(required_after),
        })
    }
}

#[cfg(not(feature = "api-observation"))]
impl EvidenceProvider for ProofEvidence {
    fn test_gate(&self, gate: &QuestGate, _required_after: EvidenceStamp) -> Truth {
        self.truth(gate)
    }
}

fn fixture() -> Result<Vec<Pair>, String> {
    let bytes = include_bytes!("../queries.json");
    if format!("{:x}", Sha256::digest(bytes)) != FIXTURE_SHA256 {
        return Err("frozen query fixture SHA-256 mismatch".into());
    }
    let pairs: Vec<Pair> = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
    if pairs.len() != QUERY_PAIRS {
        return Err("expected all 299 frozen query pairs".into());
    }
    Ok(pairs)
}

fn options() -> [(&'static str, FindOptions); 3] {
    [
        ("default", FindOptions::default()),
        (
            "teleports+wilderness",
            FindOptions {
                allow_teleports: true,
                allow_wilderness: true,
                ..FindOptions::default()
            },
        ),
        (
            "zones-all",
            FindOptions {
                allow_teleports: true,
                allow_wilderness: true,
                zones: ZoneExempt::all(),
                ..FindOptions::default()
            },
        ),
    ]
}

fn canonical_state() -> WorldState {
    let mut state = WorldState::empty().with_map_members(true);
    state.stats = (0..=20).map(|skill| (skill, 99)).collect();
    state.combat_level = Some(126);
    state
}

fn tile([x, z, level]: [i32; 3]) -> WorldTile {
    WorldTile { x, z, level }
}

fn proof_target(loc_id: i32) -> Option<usize> {
    match loc_id {
        3443 => Some(0),
        3506 | 3507 => Some(1),
        _ => None,
    }
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<PathBuf, String> {
    let mut args = args.into_iter();
    match (args.next().as_deref(), args.next(), args.next()) {
        (Some("--pack"), Some(pack), None) => Ok(PathBuf::from(pack)),
        _ => Err(
            "usage: npr-proof --pack PACK; MEMORY_WINDOW=1 enables parent-controlled memory pauses"
                .into(),
        ),
    }
}

fn memory_pause(marker: &str) -> Result<(), String> {
    println!("{marker} {}", std::process::id());
    std::io::stdout()
        .flush()
        .map_err(|error| error.to_string())?;
    let mut reply = String::new();
    if std::io::stdin()
        .read_line(&mut reply)
        .map_err(|error| error.to_string())?
        == 0
    {
        return Err(format!("memory parent closed stdin at {marker}"));
    }
    Ok(())
}

#[derive(Serialize)]
struct QueryResult {
    sequence: usize,
    options: &'static str,
    from: [i32; 3],
    to: [i32; 3],
    elapsed_ns: u128,
    outcome: &'static str,
    ticks: Option<f64>,
    legs: Option<usize>,
    proof_transport_uses: [usize; 2],
}

fn nearest_rank(sorted: &[u128], percent: usize) -> u128 {
    sorted[(sorted.len() * percent).div_ceil(100) - 1]
}

fn main() -> Result<(), String> {
    let pack = parse_args(std::env::args().skip(1))?;
    if NODE_BUDGET != 4_000_000 {
        return Err(format!(
            "canonical node budget is 4000000, not {NODE_BUDGET}"
        ));
    }
    let pairs = fixture()?;
    let world = NavWorld::load_pack(&pack).map_err(|error| format!("load pack: {error:?}"))?;
    let family = world.graph.quest_family.ok_or("pack has no quest family")?;
    let mut proof_edges = [0_usize; 2];
    for edge in &world.graph.edges {
        if let Some(target) = proof_target(edge.loc_id) {
            proof_edges[target] += 1;
        }
    }
    let required_after = EvidenceStamp {
        run: RunKey {
            slot: 0,
            run: 0,
            session: 0,
        },
        tick: 0,
        sequence: 0,
    };
    let provider: Arc<dyn EvidenceProvider> = Arc::new(ProofEvidence);
    let state = canonical_state();
    let mut results = Vec::with_capacity(QUERY_PAIRS * 3);
    let memory_window = std::env::var_os("MEMORY_WINDOW").is_some();
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    if memory_window {
        return Err("committed-memory protocol supports Linux and macOS only".into());
    }
    #[cfg(target_os = "macos")]
    let mut sampler = if memory_window {
        Some(footprint::Sampler::start()?)
    } else {
        None
    };
    #[cfg(target_os = "macos")]
    let baseline = if memory_window {
        Some(footprint::bytes()?)
    } else {
        None
    };
    if memory_window {
        memory_pause("MEMORY_BEFORE")?;
    }
    #[cfg(target_os = "macos")]
    if let Some(sampler) = sampler.as_mut() {
        sampler.begin()?;
    }
    for (label, opts) in options() {
        for pair in &pairs {
            // The canonical adapter clones the raw state and attaches the same
            // provider before each search. These operations are outside its clock.
            let mut overlaid = state.clone();
            overlaid.quest_evidence = Some(QuestEvidence::new(
                Arc::clone(&provider),
                family,
                required_after,
            ));
            let started = Instant::now();
            let route = find_with(
                &world.collision,
                &world.graph,
                tile(pair[0]),
                tile(pair[1]),
                opts,
                &overlaid,
            );
            let elapsed_ns = started.elapsed().as_nanos();
            let (outcome, ticks, legs) = match &route {
                Ok(route) => ("Route", Some(route.ticks), Some(route.legs.len())),
                Err(RouteError::NoPath) => ("NoPath", None, None),
                Err(RouteError::BudgetExhausted) => ("BudgetExhausted", None, None),
            };
            let mut proof_transport_uses = [0_usize; 2];
            if let Ok(route) = &route {
                for leg in &route.legs {
                    let Leg::Transport { edge } = leg else {
                        continue;
                    };
                    if let Some(target) = proof_target(edge.loc_id) {
                        proof_transport_uses[target] += 1;
                    }
                }
            }
            results.push(QueryResult {
                sequence: results.len(),
                options: label,
                from: pair[0],
                to: pair[1],
                elapsed_ns,
                outcome,
                ticks,
                legs,
                proof_transport_uses,
            });
        }
    }
    #[cfg(target_os = "macos")]
    let measurement = sampler.map(footprint::Sampler::finish).transpose()?;
    if memory_window {
        // No result sorting or JSON allocation is included in the memory window.
        memory_pause("MEMORY_AFTER")?;
    }
    #[cfg(target_os = "macos")]
    if let (Some(baseline), Some(measurement)) = (baseline, measurement) {
        eprintln!(
            "MEMORY_FOOTPRINT {}",
            json!({
                "baseline_bytes": baseline,
                "measurement": measurement,
            })
        );
    }
    let mut times: Vec<_> = results.iter().map(|row| row.elapsed_ns).collect();
    times.sort_unstable();
    let proof_transport_uses = results.iter().fold([0_usize; 2], |sum, row| {
        [
            sum[0] + row.proof_transport_uses[0],
            sum[1] + row.proof_transport_uses[1],
        ]
    });
    println!(
        "{}",
        json!({
            "schema": "274bot-npr-proof-v1",
            "name": "npr_proof_original_full_corpus",
            "profile": "proof",
            "pack": pack,
            "quest_family": format!("{family:?}"),
            "proof_edges_a1_a2": proof_edges,
            "proof_transport_uses_a1_a2": proof_transport_uses,
            "memory_window": memory_window,
            "timing_eligible": !memory_window,
            "input_trace_sha256": INPUT_SHA256,
            "query_fixture_sha256": FIXTURE_SHA256,
            "node_budget": NODE_BUDGET,
            "query_pairs": QUERY_PAIRS,
            "searches": results.len(),
            "measurement": "one complete ordered corpus pass; pack decoding, provider attachment, result extraction and serialization excluded from each search clock; not repeated warm searches",
            "stage_facts": [{"quest":"priestperil","signal":"priestperil","value":61}, {"quest":"druidspirit","signal":"druidspirit","value":1}],
            "stage_source": "explicit canonical corpus fixture; not live account observations",
            "unrelated_gates": "Unknown; raw input has no original quest evidence",
            "mixed_query_find_ns": {
                "total": times.iter().sum::<u128>(),
                "p50": nearest_rank(&times, 50),
                "p95": nearest_rank(&times, 95),
                "samples": times.len(),
                "first_query": results[0].elapsed_ns,
            },
            "results": results,
        })
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::selected::{FactKey, StageWindow};

    fn gate(quest: &str, signal: &str, min: i32, max: i32) -> QuestGate {
        QuestGate::Window(StageWindow {
            quest: FactKey(quest.into()),
            signal: FactKey(signal.into()),
            values: InclusiveRange {
                min: Some(min),
                max: Some(max),
            },
        })
    }

    #[test]
    fn frozen_fixture_and_order_cover_all_897_calls() {
        let pairs = fixture().expect("exact frozen fixture");
        assert_eq!(pairs.len(), 299);
        assert_eq!(pairs[0], [[3222, 3218, 0], [2584, 3419, 0]]);
        assert_eq!(options().len() * pairs.len(), 897);
        let groups = options();
        assert!(!groups[0].1.allow_teleports && !groups[0].1.allow_wilderness);
        assert!(groups[1].1.allow_teleports && groups[1].1.allow_wilderness);
        assert!(format!("{:?}", groups[2].1.zones).contains("all: true"));
        assert_eq!(NODE_BUDGET, 4_000_000);
        assert_eq!(proof_target(3443), Some(0));
        assert_eq!(proof_target(3506), Some(1));
        assert_eq!(proof_target(3507), Some(1));
        assert_eq!(proof_target(1), None);
    }

    #[test]
    fn raw_state_matches_canonical_input_without_unrelated_evidence() {
        let state = canonical_state();
        assert!(state.map_members);
        assert_eq!(state.combat_level, Some(126));
        assert_eq!(state.stats.len(), 21);
        assert!(state.stats.values().all(|value| *value == 99));
        assert!(state.inv.is_empty() && state.worn.is_empty());
        assert!(state.varps.is_empty() && state.quests.is_empty());
        assert!(state.quest_evidence.is_none());
        assert!(!state.allow_fetchable_worn);
        // Readiness-field assertions use the current observation API shape.
        #[cfg(feature = "api-observation")]
        assert!(!state.inventory_ready && !state.equipment_ready);
    }

    #[test]
    fn provider_tests_actual_stage_and_leaves_all_other_gates_unknown() {
        let provider = ProofEvidence;
        assert_eq!(
            provider.truth(&gate("priestperil", "priestperil", 61, 61)),
            Truth::True
        );
        assert_eq!(
            provider.truth(&gate("priestperil", "priestperil", 62, 80)),
            Truth::False
        );
        assert_eq!(
            provider.truth(&gate("druidspirit", "druidspirit", 1, 110)),
            Truth::True
        );
        assert_eq!(
            provider.truth(&gate("druidspirit", "druidspirit", 2, 110)),
            Truth::False
        );
        assert_eq!(
            provider.truth(&gate("priestperil", "other", 61, 61)),
            Truth::Unknown
        );
        assert_eq!(
            provider.truth(&gate("other", "other", 0, 100)),
            Truth::Unknown
        );
        assert_eq!(
            provider.truth(&QuestGate::Complete(FactKey("priestperil".into()))),
            Truth::Unknown
        );
    }

    #[test]
    fn actual_api_binding_preserves_the_request_floor() {
        let stamp = EvidenceStamp {
            run: RunKey {
                slot: 3,
                run: 4,
                session: 5,
            },
            tick: 6,
            sequence: 7,
        };
        let gate = gate("priestperil", "priestperil", 61, 61);
        #[cfg(feature = "api-observation")]
        {
            let observed = ProofEvidence
                .observe_gate(&gate, stamp)
                .expect("explicit input");
            assert_eq!(observed.truth, Truth::True);
            assert_eq!(observed.observed, Some(stamp));
        }
        #[cfg(not(feature = "api-observation"))]
        assert_eq!(ProofEvidence.test_gate(&gate, stamp), Truth::True);
    }

    #[test]
    fn arguments_and_quantiles_are_explicit() {
        assert_eq!(
            parse_args(["--pack".into(), "pack".into()]),
            Ok(PathBuf::from("pack"))
        );
        assert!(parse_args(Vec::<String>::new()).is_err());
        assert!(parse_args(["--pack".into(), "pack".into(), "--subset".into()]).is_err());
        assert_eq!(nearest_rank(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10], 50), 5);
        assert_eq!(nearest_rank(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10], 95), 10);
    }
}
