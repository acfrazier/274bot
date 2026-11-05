use super::*;
use api::prereq_knowledge::Observation;
use api::quest_progress::{EvidenceProvider, EvidenceStamp};
use api::selected::{InclusiveRange, QuestGate, RunKey, Truth};
use nav_planner::quest_gates::QuestEvidence;
use nav_planner::transport::{TransportEdge, TransportGraph};
use std::sync::Arc;

struct StageEvidence {
    quest: &'static str,
    signal: &'static str,
    observed: EvidenceStamp,
    stage: Option<InclusiveRange>,
    complete: bool,
}

impl EvidenceProvider for StageEvidence {
    fn observe_gate(&self, gate: &QuestGate, required_after: EvidenceStamp) -> Option<Observation> {
        let truth = if !self.observed.meets(required_after) {
            Truth::Unknown
        } else {
            match gate {
                QuestGate::Complete(quest) if quest.0.as_ref() == self.quest => {
                    if self.complete {
                        Truth::True
                    } else {
                        Truth::False
                    }
                }
                QuestGate::Window(window)
                    if window.quest.0.as_ref() == self.quest
                        && window.signal.0.as_ref() == self.signal =>
                {
                    self.stage
                        .map_or(Truth::Unknown, |possible| window.values.test(&possible))
                }
                _ => Truth::Unknown,
            }
        };
        Some(Observation {
            truth,
            observed: Some(self.observed),
        })
    }
}

fn stage_state(
    quest: &'static str,
    signal: &'static str,
    stage: Option<InclusiveRange>,
    complete: bool,
) -> WorldState {
    let observed = EvidenceStamp {
        run: RunKey {
            slot: 1,
            run: 1,
            session: 1,
        },
        tick: 10,
        sequence: 0,
    };
    let required_after = EvidenceStamp {
        tick: 5,
        ..observed
    };
    WorldState::empty().with_quest_evidence(QuestEvidence::new(
        Arc::new(StageEvidence {
            quest,
            signal,
            observed,
            stage,
            complete,
        }),
        required_after,
    ))
}

fn grand_tree_state(stage: Option<InclusiveRange>, complete: bool) -> WorldState {
    stage_state("grandtree", "grandtree", stage, complete)
}

// Test the union of the authoritative partitions, not one historical edge shape.
fn assert_stage_values(
    edges: &[TransportEdge],
    quest: &'static str,
    signal: &'static str,
    first: i32,
    last: i32,
) {
    let selected =
        api::game_data::for_revision(client::io::ClientRevision::R289).expect("selected revision");
    let facts = selected.quest_identity().expect("selected quest facts");
    let row = facts
        .rows
        .iter()
        .find(|row| row.id == quest && row.varp == signal)
        .expect("selected signal");
    assert_eq!(
        row.stage_domain,
        Some(InclusiveRange {
            min: Some(0),
            max: Some(last),
        })
    );
    let allows = |value, complete| {
        let state = stage_state(
            quest,
            signal,
            Some(InclusiveRange {
                min: Some(value),
                max: Some(value),
            }),
            complete,
        );
        edges
            .iter()
            .any(|edge| edge.loc_id == 6000 && state.allows(edge))
    };
    for value in 0..=last {
        assert_eq!(
            allows(value, row.colour_authoritative && value >= row.complete),
            value >= first,
            "{signal} value {value}"
        );
    }
    for value in [i32::MIN, -1, last + 1, i32::MAX] {
        assert!(!allows(value, false), "{signal} out-of-domain {value}");
    }
    assert!(!edges
        .iter()
        .any(|edge| edge.loc_id == 6000 && WorldState::empty().allows(edge)));
}

fn stage_world(handler: &str) -> (TransportGraph, nav_planner::collision::WorldCollision) {
    let fx = Fixture::new();
    write_forced_fixture(&fx, handler);
    fx.write(
        "pack/loc.pack",
        "6000=obstacle\n6001=cleared_obstacle\n7000=inviswall\n",
    );
    derive_entrance_fixture(&fx)
}

fn stage_edges(handler: &str) -> Vec<TransportEdge> {
    stage_world(handler)
        .0
        .edges
        .into_iter()
        .filter(|edge| edge.loc_id == 6000)
        .collect()
}

fn paired_stage_handler(guard: &str) -> String {
    paired_stage_handler_mode(guard, false)
}

fn paired_stage_handler_with_tail_label(guard: &str) -> String {
    paired_stage_handler_mode(guard, true)
}

fn paired_stage_handler_mode(guard: &str, tail_label: bool) -> String {
    let tail_call = if tail_label {
        "@open_stage_gate;\n[label,open_stage_gate]\n"
    } else {
        ""
    };
    format!(
        r#"
[oploc2,obstacle]
{guard}
if (coordx(coord) < coordx(loc_coord)) {{
    p_teleport(movecoord(loc_coord, 2, 0, 0));
}} else {{
    p_teleport(movecoord(loc_coord, -2, 0, 0));
}}
{tail_call}loc_findallzone(movecoord(coord, 1, 0, 0));
while (loc_findnext = true) {{
    if (loc_category = gate_area) {{
        def_coord $central_coord = loc_coord();
        def_int $orig_angle = loc_angle();
        def_int $type = loc_type;
        loc_change(inviswall, 3);
        if ($type = cleared_obstacle) {{
            loc_add(movecoord($central_coord, -1, 0, 0), cleared_obstacle, 3, loc_shape, 3);
            loc_add(movecoord($central_coord, -2, 0, 0), cleared_obstacle, 3, loc_shape, 3);
        }}
    }}
}}
sound_synth(door_open, 1, 0);
p_delay(0);
"#
    )
}

fn quest_complete(varp: &str) -> i32 {
    let selected =
        api::game_data::for_revision(client::io::ClientRevision::R289).expect("selected revision");
    selected
        .quest_identity()
        .expect("selected quest facts")
        .rows
        .iter()
        .find(|row| row.varp == varp)
        .expect("selected quest signal")
        .complete
}

fn scenery_edges(second_offset: i32, delay: i32) -> Vec<TransportEdge> {
    let mut handler = String::from(
        r#"
[oploc2,obstacle]
if (%grandtree < 80) { return; }
p_delay(1);
if (coordx(coord) < coordx(loc_coord)) {
    p_teleport(movecoord(loc_coord, 2, 0, 0));
} else {
    p_teleport(movecoord(loc_coord, -2, 0, 0));
}
loc_findallzone(movecoord(coord, 1, 0, 0));
while (loc_findnext = true) {
    if (loc_category = gate_area) {
        def_coord $central_coord = loc_coord();
        def_int $orig_angle = loc_angle();
        def_int $type = loc_type;
        loc_change(inviswall, 3);
        if ($type = cleared_obstacle) {
            loc_add(movecoord($central_coord, -1, 0, 0), cleared_obstacle, 3, loc_shape, 3);
"#,
    );
    handler.push_str(&format!(
        "            loc_add(movecoord($central_coord, {second_offset}, 0, 0), cleared_obstacle, 3, loc_shape, 3);\n"
    ));
    handler.push_str(&format!(
        "        }}\n    }}\n}}\nsound_synth(door_open, 1, 0);\np_delay({delay});\n"
    ));
    let fx = Fixture::new();
    write_forced_fixture(&fx, &handler);
    fx.write(
        "pack/loc.pack",
        "6000=obstacle\n6001=cleared_obstacle\n7000=inviswall\n",
    );
    let (graph, _) = derive_entrance_fixture(&fx);
    graph
        .edges
        .into_iter()
        .filter(|edge| edge.loc_id == 6000)
        .collect()
}
fn stage_edges_for_facts(
    handler: &str,
    facts: &api::game_data::QuestIdentityFacts,
) -> Vec<TransportEdge> {
    let fx = Fixture::new();
    write_forced_fixture(&fx, handler);
    fx.write(
        "pack/loc.pack",
        "6000=obstacle\n6001=cleared_obstacle\n7000=inviswall\n",
    );
    let defs = loc_defs(&[(6000, 2, 2), (6001, 2, 2)]);
    let collision = bake_collision(&fx, &defs, &HashSet::new());
    let quests = QuestSource { facts };
    derive_transports_with_audit(fx.path(), &defs, &collision, None, Some(&quests), None)
        .0
        .edges
        .into_iter()
        .filter(|edge| edge.loc_id == 6000)
        .collect()
}

fn window_covers(edge: &TransportEdge, quest: &str, signal: &str, value: i32) -> bool {
    let Some(gates) = &edge.quest_gates else {
        return false;
    };
    let [QuestGate::Window(window)] = gates.gates() else {
        return false;
    };
    window.quest.0.as_ref() == quest
        && window.signal.0.as_ref() == signal
        && window.values.test(&InclusiveRange {
            min: Some(value),
            max: Some(value),
        }) == Truth::True
}

#[test]
fn grand_tree_gate_retains_stage_windows_without_colour_authority() {
    let handler = paired_stage_handler("if (%grandtree < 80) { return; }");
    let (graph, collision) = stage_world(&handler);
    let complete = quest_complete("grandtree");
    assert!(complete > 80);
    // At7dac,289.json:128372-384 proves [0,160]. Content writes release80
    // in king_narnode.rs2:423 and completion160 in quest_grandtree.rs2:198.
    assert_stage_values(&graph.edges, "grandtree", "grandtree", 80, 160);
    let edges: Vec<_> = graph
        .edges
        .iter()
        .filter(|edge| edge.loc_id == 6000)
        .collect();
    let window = edges
        .iter()
        .find(|edge| window_covers(edge, "grandtree", "grandtree", 80))
        .expect("Grand Tree's pre-completion gate window");
    assert!(!WorldState::empty().allows(window));
    // Current guarded-writer analysis is conservative, not colour-authoritative.
    // See BATCH-029-INT/w3-hero-grandtree-colour-guards.md for both actual writers.
    assert!(edges
        .iter()
        .any(|edge| window_covers(edge, "grandtree", "grandtree", 160)));
    assert!(edges.iter().all(|edge| {
        edge.quest_gates.as_ref().is_some_and(|gates| {
            matches!(gates.gates(), [QuestGate::Window(window)]
                if window.quest.0.as_ref() == "grandtree"
                    && window.signal.0.as_ref() == "grandtree")
        })
    }));
    assert!(edges.iter().all(|edge| {
        edge.at == tile(2819, 3438)
            && edge.takeoff.is_some()
            && edge.option == 2
            && edge.quest_req.is_empty()
            && edge.varp_req.is_empty()
    }));
    assert!(edges.iter().any(|edge| edge.to == tile(2821, 3438)));
    assert!(edges.iter().any(|edge| edge.to == tile(2817, 3438)));

    let from = tile(2818, 3438);
    let to = tile(2821, 3438);
    let in_window = grand_tree_state(
        Some(InclusiveRange {
            min: Some(80),
            max: Some(80),
        }),
        false,
    );
    assert!(crosses(&graph, &collision, from, to, &in_window, 6000).unwrap());
    let below_window = grand_tree_state(
        Some(InclusiveRange {
            min: Some(0),
            max: Some(79),
        }),
        false,
    );
    assert_eq!(
        crosses(&graph, &collision, from, to, &below_window, 6000),
        Err(RouteError::NoPath)
    );
    let complete_stage = grand_tree_state(
        Some(InclusiveRange {
            min: Some(160),
            max: Some(160),
        }),
        false,
    );
    assert!(crosses(&graph, &collision, from, to, &complete_stage, 6000).unwrap());
    assert_eq!(
        crosses(
            &graph,
            &collision,
            from,
            to,
            &grand_tree_state(None, true),
            6000,
        ),
        Err(RouteError::NoPath),
        "colour-only completion cannot prove the guarded stage window"
    );
    assert_eq!(
        crosses(&graph, &collision, from, to, &WorldState::empty(), 6000),
        Err(RouteError::NoPath)
    );
}

#[test]
fn monkey_madness_started_signal_preserves_every_authoritative_value_above_one() {
    // At7dac,289.json:129242-254 proves [0,10], not [2,infinity).
    // quest_mm.constant:1-11 defines0..10; mm_caranock.rs2:51 writes2,
    // quest_mm.rs2:1168 writes9, and mm_daero.rs2:293 writes10 after completion.
    let handler = paired_stage_handler("if (%mm_main <= 1) { return; }");
    let (graph, _) = stage_world(&handler);
    assert_stage_values(&graph.edges, "mm", "mm_main", 2, 10);
    assert!(graph
        .edges
        .iter()
        .any(|edge| window_covers(edge, "mm", "mm_main", 2)));
    assert!(graph
        .edges
        .iter()
        .filter(|edge| edge.loc_id == 6000)
        .all(|edge| {
            matches!(
                edge.quest_gates.as_ref().map(|gates| gates.gates()),
                Some([QuestGate::Window(_)])
            )
        }));
}

#[test]
fn varrock_style_gate_uses_its_reachable_scenery_label() {
    let handler = paired_stage_handler_with_tail_label("if (%grandtree < 80) { return; }");
    let (graph, _) = stage_world(&handler);
    assert_stage_values(&graph.edges, "grandtree", "grandtree", 80, 160);
    assert!(graph
        .edges
        .iter()
        .any(|edge| window_covers(edge, "grandtree", "grandtree", 80)));
}

#[test]
fn generic_stage_windows_keep_authority_and_dialogue_is_rejected() {
    let generic = stage_edges(
        r#"
[oploc2,obstacle]
if (%grandtree < 80) { return; }
p_delay(1);
p_teleport(0_44_53_5_46);
"#,
    );
    // The later F-C proof applies to generic entrances too, not just paired
    // scenery gates. The same selected domain and content writers apply.
    assert_stage_values(&generic, "grandtree", "grandtree", 80, 160);

    let dialogue = stage_edges(
        r#"
[oploc2,obstacle]
if (%biohazard < 10) { return; }
if (%biohazard >= 14) { return; }
if (npc_find(coord(), gate_guard, 5, 0) = true) {
    npc_chat("Choose the guard's response.");
}
p_delay(1);
p_teleport(0_44_53_5_46);
"#,
    );
    assert!(
        dialogue.is_empty(),
        "dialogue-dependent crossings are not proven"
    );
}

#[test]
fn scenery_tail_requires_the_exact_loc_delta_and_final_delay() {
    let complete = quest_complete("grandtree");
    assert!(complete > 80);
    let edges = scenery_edges(-2, 0);
    assert_stage_values(&edges, "grandtree", "grandtree", 80, 160);
    assert!(edges
        .iter()
        .any(|edge| window_covers(edge, "grandtree", "grandtree", 80)));
    assert!(
        scenery_edges(-3, 0).is_empty(),
        "loc delta drift is unproven"
    );
    assert!(
        scenery_edges(-2, 1).is_empty(),
        "unknown tail delay is unproven"
    );
}

#[test]
fn absent_or_ambiguous_stage_signal_never_authorizes_a_crossing() {
    let handler = paired_stage_handler("if (%grandtree < 80) { return; }");
    let selected =
        api::game_data::for_revision(client::io::ClientRevision::R289).expect("selected revision");
    let mut facts = selected
        .quest_identity()
        .expect("selected quest facts")
        .clone();
    facts.rows.retain(|row| row.varp != "grandtree");
    assert!(stage_edges_for_facts(&handler, &facts).is_empty());

    let mut facts = selected
        .quest_identity()
        .expect("selected quest facts")
        .clone();
    let mut duplicate = facts
        .rows
        .iter()
        .find(|row| row.varp == "grandtree")
        .expect("Grand Tree signal row")
        .clone();
    duplicate.id = "ambiguous_grandtree".to_owned();
    facts.rows.push(duplicate);
    assert!(stage_edges_for_facts(&handler, &facts).is_empty());
}

#[test]
fn unknown_progress_varp_does_not_create_a_passable_edge() {
    let edges = stage_edges(
        r#"
[oploc2,obstacle]
if (%unverified_stage > 1) { return; }
p_delay(1);
p_teleport(0_44_53_5_46);
"#,
    );
    assert!(edges.is_empty());
}
