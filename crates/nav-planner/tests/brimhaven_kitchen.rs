//! Door-specific corpus from real packed geometry. The small collision crop
//! prevents the neighbouring kitchen panel from replacing the tested door.
use api::prereq_knowledge::Observation;
use api::quest_progress::{EvidenceProvider, EvidenceStamp};
use api::selected::{InclusiveRange, QuestGate, RunKey, Truth};
use nav_planner::collision::WorldCollision;
use nav_planner::quest_gates::QuestEvidence;
use nav_planner::router::{find_with, FindOptions, Leg};
use nav_planner::transport::TransportGraph;
use nav_planner::world::NavWorld;
use nav_planner::WorldState;
use std::sync::Arc;

fn stamp() -> EvidenceStamp {
    EvidenceStamp {
        run: RunKey {
            slot: 1,
            run: 1,
            session: 1,
        },
        tick: 1,
        sequence: 1,
    }
}

#[derive(Clone, Copy)]
struct Account {
    hero: Option<i32>,
    phoenix: Option<i32>,
    black_arm: bool,
}
impl EvidenceProvider for Account {
    fn observe_gate(&self, gate: &QuestGate, required: EvidenceStamp) -> Option<Observation> {
        if !stamp().meets(required) {
            return None;
        }
        let truth = match gate {
            QuestGate::Window(window) => {
                let value = match (window.quest.0.as_ref(), window.signal.0.as_ref()) {
                    ("hero", "heroquest") => self.hero,
                    ("blackarmgang", "phoenixgang") => self.phoenix,
                    _ => None,
                }?;
                window.values.test(&InclusiveRange {
                    min: Some(value),
                    max: Some(value),
                })
            }
            QuestGate::Complete(quest) if quest.0.as_ref() == "hero" => {
                if self.hero? >= 15 {
                    Truth::True
                } else {
                    Truth::False
                }
            }
            QuestGate::Complete(quest) if quest.0.as_ref() == "blackarmgang" => {
                if self.black_arm || self.phoenix? == 10 {
                    Truth::True
                } else {
                    Truth::False
                }
            }
            _ => return None,
        };
        Some(Observation {
            truth,
            observed: Some(stamp()),
        })
    }
}

fn matrix(world: &NavWorld, qualified: bool) -> Vec<serde_json::Value> {
    let edges: Vec<_> = world
        .graph
        .edges
        .iter()
        .filter(|e| e.loc_id == 2628)
        .cloned()
        .collect();
    assert_eq!(edges.len(), 2, "both kitchen door directions exist");
    let origin = api::WorldTile {
        x: 2788,
        z: 3189,
        level: 0,
    };
    let mut collision = WorldCollision {
        origin,
        width: 1,
        height: 2,
        walk: vec![0; 8],
        blocked: vec![0],
        flags: None,
    };
    for level in 0..4 {
        for z in 0..2 {
            collision.walk[level * 2 + z] =
                world
                    .collision
                    .walkable_word(2788, 3189 + z as i32, level as i32) as u8;
        }
    }
    let mut graph = TransportGraph {
        edges,
        approaches: world
            .graph
            .edges
            .iter()
            .enumerate()
            .filter(|(_, edge)| edge.loc_id == 2628)
            .map(|(index, _)| world.graph.approaches.get(index).copied().flatten())
            .collect(),
        ..Default::default()
    };
    graph.rebuild_index(&collision);
    assert_eq!(
        graph.at.values().map(Vec::len).sum::<usize>(),
        2,
        "both real crossings are indexed (qualified={qualified}): {graph:?}"
    );
    let profiles = [
        (
            "phoenix_alfonse",
            Account {
                hero: Some(3),
                phoenix: Some(10),
                black_arm: false,
            },
            qualified,
        ),
        (
            "phoenix_complete",
            Account {
                hero: Some(15),
                phoenix: Some(10),
                black_arm: false,
            },
            true,
        ),
        (
            "phoenix_too_early",
            Account {
                hero: Some(2),
                phoenix: Some(10),
                black_arm: false,
            },
            false,
        ),
        (
            "black_arm_complete",
            Account {
                hero: Some(15),
                phoenix: Some(0),
                black_arm: true,
            },
            !qualified,
        ),
        (
            "unknown",
            Account {
                hero: None,
                phoenix: None,
                black_arm: false,
            },
            false,
        ),
    ];
    let mut rows = Vec::new();
    for (profile, account, expected) in profiles {
        let state = WorldState::empty()
            .with_map_members(true)
            .with_quest_evidence(QuestEvidence::new(Arc::new(account), stamp()));
        for edge in &graph.edges {
            let id = edge.crossing_id().expect("kitchen door crossing identity");
            let route = find_with(
                &collision,
                &graph,
                id.takeoff,
                id.to,
                FindOptions::default(),
                &state,
            );
            let used = route.as_ref().is_ok_and(|route| {
                route
                    .legs
                    .iter()
                    .any(|leg| matches!(leg, Leg::Transport { edge } if edge.loc_id == 2628))
            });
            assert_eq!(
                used, expected,
                "profile={profile}, crossing={id:?}, result={route:?}"
            );
            assert_eq!(
                route.is_ok(),
                expected,
                "no bypass of the door in this crop"
            );
            rows.push(serde_json::json!({ "profile": profile, "authority": "declared fixture", "heroquest_varp_188": account.hero, "phoenixgang_varp_146": account.phoenix, "blackarmgang_varp_145": if account.black_arm { Some(4) } else { None }, "crossing": id, "route": format!("{route:?}"), "admitted": used, "gate": edge.quest_gates.as_ref().map(|gates| gates.gates()), "source": "brimhaven_restaurant.rs2:1-6; quest_hero.constant:4; quest.constant:7-8; quests.rs2:328-332" }));
        }
    }
    rows
}

#[test]
#[ignore = "requires the R2 qualified pack; reference pack remains the comparator"]
fn kitchen_gate_correction_preserves_phoenix_and_refuses_black_arm_both_ways() {
    let world = NavWorld::load_pack(&std::path::PathBuf::from(
        std::env::var_os("NAV_PACK").expect("explicit qualified NAV_PACK"),
    ))
    .expect("qualified pack");
    let candidate = matrix(&world, true);
    let baseline = std::env::var_os("NAV_PREREQ_BASELINE_PACK").map(|path| {
        let world = NavWorld::load_pack(&std::path::PathBuf::from(path)).expect("baseline pack");
        matrix(&world, false)
    });
    let value = serde_json::json!({ "candidate": candidate, "baseline": baseline, "classification": "corrected gate, not a lost Phoenix route" });
    if let Some(path) = std::env::var_os("NAV_PREREQ_KITCHEN_MATRIX") {
        std::fs::write(path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    }
    println!("KITCHEN_CORPUS {}", value);
}
