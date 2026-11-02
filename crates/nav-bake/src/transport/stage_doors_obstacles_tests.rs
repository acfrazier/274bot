use super::*;
use api::prereq_knowledge::Observation;
use api::quest_progress::{EvidenceProvider, EvidenceStamp};
use api::selected::{InclusiveRange, QuestGate, RunKey, Truth};
use nav_planner::quest_gates::{QuestEvidence, QuestFamilyId};
use std::sync::Arc;

// Verbatim handlers, with comments removed:
// quest_death/scripts/quest_death.rs2:103-118;
// quest_troll/scripts/quest_troll.rs2:1-72;
// quest_upass/scripts/upass_obstacles.rs2:3-31;
// area_ardougne_west/scripts/doors.rs2:15-16,24-54;
// quest_itwatchtower/scripts/quest_itwatchtower.rs2:1-8,261-266;
// quest_ball/scripts/quest_ball.rs2:33-58.
const HANDLERS: &str = include_str!("obstacle_handlers.rs2");

fn obstacle_fixture(script: &str) -> Fixture {
    let fx = Fixture::new();
    write_stage_engine(&fx);
    fx.write("scripts/obstacles.rs2", script);
    fx.write(
        "scripts/motion.rs2",
        include_str!("engine_forced_procs.rs2"),
    );
    fx.write(
        "scripts/appearance.rs2",
        include_str!("engine_obstacle_procs.rs2"),
    );
    fx.write("pack/loc.pack", "3730=death_fullstyle\n3748=troll_climbingrocks\n3790=troll_climbingrocks_top\n3791=troll_climbingrocks_bottom\n2048=ardougnewalldoor_left\n2049=ardougnewalldoor_right\n2299=loc_2299\n2796=watchladderup\n2865=shockgatel\n2866=shockgater\n1562=loc_1562\n1563=loc_1563\n83=inviswall\n");
    fx.write(
        "pack/obj.pack",
        "3105=death_climbingboots\n1059=leather_gloves\n",
    );
    fx.write(
        "pack/varp.pack",
        "317=troll_quest\n212=itwatchtower\n68=biohazard\n",
    );
    fx.write("scripts/obstacles.constant", "^exact_north = 0\n^exact_east = 1\n^exact_south = 2\n^exact_west = 3\n^troll_started = 10\n^troll_complete = 50\n^itwatchtower_complete = 13\n^biohazard_complete = 16\n");
    fx.write(
        "scripts/obstacles.varp",
        "[troll_quest]\nscope=perm\n[itwatchtower]\nscope=perm\n[biohazard]\nscope=perm\n",
    );
    fx.write("scripts/obstacles.loc", "[death_fullstyle]\nop1=Climb-over\n[troll_climbingrocks]\nop1=Climb\n[troll_climbingrocks_top]\nop1=Climb\n[troll_climbingrocks_bottom]\nop1=Climb\n[ardougnewalldoor_left]\nop1=Open\n[ardougnewalldoor_right]\nop1=Open\n[loc_2299]\nop1=Climb-up\n[watchladderup]\nop1=Climb-up\n[shockgatel]\nop1=Open\ncategory=ball_irongate\nparam=next_loc_stage,loc_1562\n[shockgater]\nop1=Open\ncategory=ball_irongate\nparam=next_loc_stage,loc_1563\n");
    fx.write("scripts/general/scripts/quests.rs2", &format!("{JOURNAL_GREEN_SOURCE}~send_quest_progress_colour(questlist:troll, %troll_quest, ^troll_complete);\n~send_quest_progress_colour(questlist:itwatchtower, %itwatchtower, ^itwatchtower_complete);\n~send_quest_progress_colour(questlist:biohazard, %biohazard, ^biohazard_complete);\n"));
    fx.write("scripts/player/interfaces/questlist.if", "[troll]\ntext=Troll Stronghold\n[itwatchtower]\ntext=Watch Tower\n[biohazard]\ntext=Biohazard\n");
    for (name, locs) in [
        ("m44_55", "0 1 42: 3730 10\n"),
        ("m44_56", "0 40 28: 3748 10 2\n0 41 28: 3748 10\n0 43 42: 3790 22 2\n0 43 43: 3790 22 1\n0 44 42: 3791 22 2\n0 44 43: 3791 22 1\n0 62 38: 3791 22 3\n0 62 39: 3791 22\n0 63 38: 3790 22 1\n0 63 39: 3790 22 1\n"),
        ("m45_56", ""),
        ("m39_51", "0 61 35: 2049 10 1\n0 61 36: 2048 10 3\n0 62 35: 2048 10 1\n0 62 36: 2049 10 3\n"),
        ("m39_48", "0 52 47: 2299 4 3\n1 53 39: 2796 10\n"),
        ("m45_73", ""),
        ("m45_154", "0 22 17: 2866 0 2\n0 22 18: 2865 0 2\n"),
    ] {
        let mut map = String::from("==== MAP ====\n");
        for level in 0..3 {
            for x in 0..64 {
                for z in 0..64 {
                    map.push_str(&format!("{level} {x} {z}: h1 u50\n"));
                }
            }
        }
        fx.write(&format!("maps/{name}.jm2"), &(map + "==== LOC ====\n" + locs));
    }
    fx
}

fn obstacle_graph(fx: &Fixture) -> TransportGraph {
    let selected = api::game_data::for_revision(client::io::ClientRevision::R289).unwrap();
    let (facts, identity) = selected.quest_identity_for_bake().unwrap();
    obstacle_graph_with_facts(
        fx,
        &facts,
        QuestFamilyId::new(identity.sha256, identity.schema),
    )
}

fn obstacle_graph_with_facts(
    fx: &Fixture,
    facts: &api::game_data::QuestIdentityFacts,
    family: QuestFamilyId,
) -> TransportGraph {
    let defs = loc_defs(&[
        (3730, 1, 2),
        (3748, 1, 1),
        (3790, 1, 1),
        (3791, 1, 1),
        (2048, 1, 1),
        (2049, 1, 1),
        (2299, 1, 1),
        (2796, 1, 1),
        (2865, 1, 1),
        (2866, 1, 1),
    ]);
    let collision = bake_collision(fx, &defs, &HashSet::new());
    let quests = QuestSource { family, facts };
    derive_transports_with_audit(fx.path(), &defs, &collision, None, Some(&quests)).0
}

struct ObstacleSignals(&'static str, i32, EvidenceStamp);
impl EvidenceProvider for ObstacleSignals {
    fn observe_gate(&self, gate: &QuestGate, required_after: EvidenceStamp) -> Option<Observation> {
        let truth = if !self.2.meets(required_after) {
            Truth::Unknown
        } else {
            match gate {
                QuestGate::Window(window) if window.signal.0.as_ref() == self.0 => {
                    window.values.test(&InclusiveRange {
                        min: Some(self.1),
                        max: Some(self.1),
                    })
                }
                QuestGate::Complete(quest)
                    if quest.0.as_ref() == "troll" && self.0 == "troll_quest" =>
                {
                    // The fixture uses the selected finite domain0..50.
                    match self.1 {
                        50 => Truth::True,
                        0..=49 => Truth::False,
                        _ => Truth::Unknown,
                    }
                }
                QuestGate::Complete(quest)
                    if quest.0.as_ref() == "biohazard" && self.0 == "biohazard" =>
                {
                    if self.1 == 16 {
                        Truth::True
                    } else {
                        Truth::False
                    }
                }
                QuestGate::Complete(quest)
                    if quest.0.as_ref() == "itwatchtower" && self.0 == "itwatchtower" =>
                {
                    if self.1 >= 13 {
                        Truth::True
                    } else {
                        Truth::False
                    }
                }
                _ => Truth::Unknown,
            }
        };
        Some(Observation {
            truth,
            observed: Some(self.2),
        })
    }
}

fn obstacle_state(graph: &TransportGraph, signal: &'static str, value: i32) -> WorldState {
    let stamp = EvidenceStamp {
        run: RunKey {
            slot: 1,
            run: 1,
            session: 1,
        },
        tick: 1,
        sequence: 1,
    };
    WorldState::empty().with_quest_evidence(QuestEvidence::new(
        Arc::new(ObstacleSignals(signal, value, stamp)),
        graph.quest_family.unwrap(),
        stamp,
    ))
}

fn crossing(graph: &TransportGraph, id: i32, from: WorldTile, to: WorldTile) -> &TransportEdge {
    graph
        .edges
        .iter()
        .find(|edge| edge.loc_id == id && edge.takeoff == Some(from) && edge.to == to)
        .unwrap_or_else(|| panic!("missing {id}: {from:?} -> {to:?}"))
}

#[test]
fn obstacle_stile_has_exact_ungated_final_landings() {
    let graph = obstacle_graph(&obstacle_fixture(HANDLERS));
    for (from, to) in [
        (tile(2817, 3561), tile(2817, 3564)),
        (tile(2817, 3564), tile(2817, 3561)),
    ] {
        let edge = crossing(&graph, 3730, from, to);
        assert!(WorldState::empty().allows(edge));
        assert!(
            edge.skill_req.is_empty() && edge.varp_req.is_empty() && edge.quest_gates.is_none()
        );
        assert_eq!(edge.ticks, 5);
    }
}

#[test]
fn obstacle_lower_rocks_require_started_progress_skill_and_south_only_worn_boots() {
    // At7dac,289.json:128978-990 proves the finite Troll domain0..50.
    // death_ig_commander.rs2:82 writes start10; quest_troll.rs2:367 writes
    // complete50. F-C may emit several windows and a terminal Complete gate.
    let graph = obstacle_graph(&obstacle_fixture(HANDLERS));
    for x in [2856, 2857] {
        for (from, to) in [
            (tile(x, 3611), tile(x, 3613)),
            (tile(x, 3613), tile(x, 3611)),
        ] {
            let matches_crossing = |edge: &&TransportEdge| {
                edge.loc_id == 3748 && edge.takeoff == Some(from) && edge.to == to
            };
            for edge in graph.edges.iter().filter(matches_crossing) {
                assert_eq!(edge.skill_req, [(16, 15)]);
                assert!(edge.varp_req.is_empty() && edge.quest_req.is_empty());
                assert_eq!(
                    edge.worn_all_req.as_slice(),
                    if from.z == 3611 { &[3105][..] } else { &[] }
                );
            }
            let allows = |state: &WorldState| {
                graph
                    .edges
                    .iter()
                    .filter(matches_crossing)
                    .any(|edge| state.allows(edge))
            };
            for value in 0..=50 {
                let mut state = obstacle_state(&graph, "troll_quest", value);
                state.stats.insert(16, 15);
                state.worn.insert(3105);
                assert_eq!(allows(&state), value >= 10, "Troll value {value}");
                state.stats.insert(16, 14);
                assert!(!allows(&state), "low skill at Troll value {value}");
                state.stats.insert(16, 15);
                state.worn.clear();
                state.inv.insert(3105, 1);
                assert_eq!(
                    allows(&state),
                    value >= 10 && from.z == 3613,
                    "carried boots at Troll value {value}"
                );
            }
            for value in [i32::MIN, -1, 51, i32::MAX] {
                let mut state = obstacle_state(&graph, "troll_quest", value);
                state.stats.insert(16, 15);
                state.worn.insert(3105);
                assert!(!allows(&state), "out-of-domain Troll value {value}");
            }
            let mut unknown = WorldState::empty();
            unknown.stats.insert(16, 15);
            unknown.worn.insert(3105);
            assert!(!allows(&unknown));
        }
    }
}

#[test]
fn obstacle_upper_rocks_preserve_each_placement_and_skill_gate() {
    let graph = obstacle_graph(&obstacle_fixture(HANDLERS));
    for (west, east, top_on_west, zs) in [
        (2858, 2861, true, [3626, 3627]),
        (2877, 2880, false, [3622, 3623]),
    ] {
        for z in zs {
            for (id, from, to) in [
                (
                    if top_on_west { 3790 } else { 3791 },
                    tile(west, z),
                    tile(east, z),
                ),
                (
                    if top_on_west { 3791 } else { 3790 },
                    tile(east, z),
                    tile(west, z),
                ),
            ] {
                let edge = crossing(&graph, id, from, to);
                assert_eq!(edge.skill_req, [(16, 15)]);
                assert!(
                    edge.quest_req.is_empty()
                        && edge.varp_req.is_empty()
                        && edge.quest_gates.is_none()
                        && edge.worn_all_req.is_empty()
                );
                let mut state = WorldState::empty();
                state.stats.insert(16, 14);
                assert!(!state.allows(edge));
                state.stats.insert(16, 15);
                assert!(state.allows(edge));
            }
        }
    }
}

#[test]
fn obstacle_wall_gate_requires_exact_biohazard_completion_through_the_label() {
    let graph = obstacle_graph(&obstacle_fixture(HANDLERS));
    for (id, from, to) in [
        (2049, tile(2556, 3299), tile(2559, 3299)),
        (2048, tile(2556, 3300), tile(2559, 3300)),
        (2048, tile(2559, 3299), tile(2556, 3299)),
        (2049, tile(2559, 3300), tile(2556, 3300)),
    ] {
        let edge = crossing(&graph, id, from, to);
        assert!(obstacle_state(&graph, "biohazard", 16).allows(edge));
        assert!(!obstacle_state(&graph, "biohazard", 15).allows(edge));
        assert!(!obstacle_state(&graph, "biohazard", 17).allows(edge));
        assert!(!WorldState::empty().allows(edge));
    }
}

#[test]
fn obstacle_watchtower_window_and_precompletion_ladder_keep_their_conditions() {
    let graph = obstacle_graph(&obstacle_fixture(HANDLERS));
    let edge = crossing(
        &graph,
        2299,
        tile(2548, 3119),
        WorldTile {
            x: 2548,
            z: 3117,
            level: 1,
        },
    );
    let mut state = WorldState::empty();
    state.stats.insert(16, 17);
    assert!(!state.allows(edge));
    state.stats.insert(16, 18);
    assert!(state.allows(edge));
    let edge = crossing(
        &graph,
        2796,
        WorldTile {
            x: 2549,
            z: 3112,
            level: 1,
        },
        WorldTile {
            x: 2549,
            z: 3112,
            level: 2,
        },
    );
    assert!(obstacle_state(&graph, "itwatchtower", 0).allows(edge));
    assert!(obstacle_state(&graph, "itwatchtower", 12).allows(edge));
    assert!(!obstacle_state(&graph, "itwatchtower", 13).allows(edge));
    assert!(!WorldState::empty().allows(edge));
}

#[test]
fn obstacle_cellar_gate_requires_worn_leather_gloves_in_both_directions() {
    let graph = obstacle_graph(&obstacle_fixture(HANDLERS));
    for (id, z) in [(2866, 9873), (2865, 9874)] {
        for (from, to) in [
            (tile(2902, z), tile(2903, z)),
            (tile(2903, z), tile(2902, z)),
        ] {
            let edge = crossing(&graph, id, from, to);
            assert_eq!(edge.worn_all_req, [1059]);
            assert!(
                edge.quest_gates.is_none() && edge.quest_req.is_empty() && edge.varp_req.is_empty()
            );
            let mut state = WorldState::empty();
            state.inv.insert(1059, 1);
            assert!(!state.allows(edge));
            state.worn.insert(1059);
            assert!(state.allows(edge));
        }
    }
}

#[test]
fn obstacle_proofs_refuse_changed_rolls_labels_and_movement_primitives() {
    for (script, id) in [
        (
            HANDLERS.replace("~damage_self(3);", "~damage_self(4);"),
            3748,
        ),
        (HANDLERS.replace("170, 253", "170, 254"), 3790),
        (
            HANDLERS.replace(
                "~forcemove(movecoord($loc_coord, 2, 0, 0));",
                "~forcemove(movecoord($loc_coord, 3, 0, 0));",
            ),
            2048,
        ),
    ] {
        let graph = obstacle_graph(&obstacle_fixture(&script));
        assert!(!graph.edges.iter().any(|edge| edge.loc_id == id));
    }
    let fx = obstacle_fixture(HANDLERS);
    fx.write(
        "scripts/appearance.rs2",
        &include_str!("engine_obstacle_procs.rs2").replace(
            "p_telejump($coord);",
            "p_telejump(movecoord($coord, 1, 0, 0));",
        ),
    );
    assert!(!obstacle_graph(&fx)
        .edges
        .iter()
        .any(|edge| edge.loc_id == 2299));
}

#[test]
fn obstacle_wall_equality_stays_a_window_without_the_completion_upper_bound() {
    let fx = obstacle_fixture(HANDLERS);
    fx.write(
        "scripts/future.rs2",
        "[proc,future_stage]\n%biohazard = 17;\n",
    );
    let selected = api::game_data::for_revision(client::io::ClientRevision::R289).unwrap();
    let (mut facts, identity) = selected.quest_identity_for_bake().unwrap();
    let row = facts
        .rows
        .iter_mut()
        .find(|row| row.id == "biohazard")
        .unwrap();
    // At7dac,289.json:128138-150 and real quest_biohazard.rs2:160 prove
    // completion16/domain0..16. Only this synthetic future writer adds17;
    // its in-memory family must say so too. Actual generated facts stay intact.
    assert_eq!(row.stage_domain.unwrap().max, Some(16));
    row.stage_domain = Some(InclusiveRange {
        min: Some(0),
        max: Some(17),
    });
    let graph = obstacle_graph_with_facts(
        &fx,
        &facts,
        QuestFamilyId::new(identity.sha256, identity.schema),
    );
    let edge = crossing(&graph, 2049, tile(2556, 3299), tile(2559, 3299));
    let gates = edge.quest_gates.as_ref().expect("exact signal gate");
    let [QuestGate::Window(window)] = gates.gates() else {
        panic!("the future writer forbids a completion-only proof");
    };
    assert_eq!(window.values.min, Some(16));
    assert_eq!(window.values.max, Some(16));
    for value in 0..=17 {
        assert_eq!(
            obstacle_state(&graph, "biohazard", value).allows(edge),
            value == 16,
            "Biohazard value {value}"
        );
    }
    for value in [i32::MIN, -1, 18, i32::MAX] {
        assert!(!obstacle_state(&graph, "biohazard", value).allows(edge));
    }
    assert!(!WorldState::empty().allows(edge));
}
