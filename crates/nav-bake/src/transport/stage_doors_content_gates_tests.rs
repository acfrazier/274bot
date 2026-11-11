use super::*;
use api::prereq_knowledge::Observation;
use api::quest_progress::{EvidenceProvider, EvidenceStamp};
use api::selected::{InclusiveRange, QuestGate, RunKey, Truth};
use nav_planner::quest_gates::QuestEvidence;
use nav_planner::transport::TransportGraph;
use std::sync::Arc;

// quest_grandtree/scripts/quest_grandtree.rs2:112-151, with only comments removed.
const SHIPYARD: &str = r#"
[oploc1,_shipyard_gate]
if(%grandtree >= ^grandtree_released_prison) {
    if(coordx(coord) < coordx(loc_coord) & npc_find(coord, grandtree_shipyardguard, 5, 0) = true) {
        if(%mm_main > ^mm_started) {
            if_close;
            p_delay(2);
            npc_facesquare(0_46_47_1_33);
            p_delay(0);
            @open_shipyard_gate;
            return;
        }
        @shipyardworker_gate;
    } else {
        @open_shipyard_gate;
    }
}
~mesbox("The gate is locked.");
[label,open_shipyard_gate]
if(coordx(coord) < coordx(loc_coord)) {
    p_teleport(loc_coord);
} else {
    p_teleport(movecoord(loc_coord, -1, 0, 0));
}
loc_findallzone(coord);
while(loc_findnext = true) {
    if(loc_category = shipyard_gate) {
        def_coord $central_coord = loc_coord;
        def_int $orig_angle = loc_angle;
        def_loc $type = loc_type;
        loc_change(inviswall, 3);
        if($type = grandtree_fencegate_l) {
            loc_add(movecoord($central_coord, -1, 0, 0), inacfishinggateclosedl, 3, loc_shape, 3);
            loc_add(movecoord($central_coord, -2, 0, 0), inacfishinggateclosedr, 3, loc_shape, 3);
        }
    }
}
p_delay(0);
"#;

// areas/area_varrock/scripts/east_gate.rs2:4-65, with only comments removed.
const VARROCK: &str = r#"
[oploc1,_varrock_gate]
if(map_members = ^false) {
    mes(^mes_members_gate);
    return;
}
if(~check_axis(coord, loc_coord, loc_angle) = false) {
    if(map_members = ^false) {
        mes(^mes_members_gate);
        return;
    }
    if(%biohazard >= ^biohazard_given_distillator & %biohazard < ^biohazard_found_secret & npc_find(coord, bioguard1, 2, 0) = true) {
        ~chatnpc("<p,neutral>Halt. I need to conduct a search on you. There have been reports of someone bringing a virus into Varrock.");
        if_close;
        mes("The guard searches you.");
        p_delay(3);
        if(inv_total(inv, ethenea) > 0) {
            mes("He takes the vial of ethenea from you.");
            inv_del(inv, ethenea, 1);
            p_delay(3);
        }
        if(inv_total(inv, liquid_honey) > 0) {
            mes("He takes the vial of liquid honey from you.");
            inv_del(inv, liquid_honey, 1);
            p_delay(3);
        }
        if(inv_total(inv, sulphuric_broline) > 0) {
            mes("He takes the vial of sulphuric broline from you.");
            inv_del(inv, sulphuric_broline, 1);
            p_delay(3);
        }
        ~chatnpc("<p,neutral>You may now pass.");
        if_close;
        ~forcemove(0_50_53_63_13);
        p_delay(0);
        p_teleport(movecoord(coord, 1, 0, 0));
        @open_varrockgate;
    }
}
if(coordx(coord) < coordx(loc_coord)) {
    p_teleport(loc_coord);
} else {
    p_teleport(movecoord(loc_coord, -1, 0, 0));
}
@open_varrockgate;
[label,open_varrockgate]
loc_findallzone(movecoord(coord, 1, 0, 0));
while(loc_findnext = true) {
    if(loc_category = varrock_gate) {
        def_coord $central_coord = loc_coord;
        def_int $orig_angle = loc_angle;
        def_loc $type = loc_type;
        loc_change(inviswall, 3);
        if($type = guidorgatelclosed) {
            loc_add(movecoord($central_coord, -1, 0, 0), guidorgatelopen, 3, loc_shape, 3);
            loc_add(movecoord($central_coord, -2, 0, 0), guidorgateropen, 3, loc_shape, 3);
        }
    }
}
sound_synth(door_open, 1, 0);
p_delay(0);
"#;

fn content_gate_graph(script: &str, shipyard: bool) -> TransportGraph {
    let fx = Fixture::new();
    write_stage_engine(&fx);
    fx.write(
        "scripts/agility.rs2",
        include_str!("engine_forced_procs.rs2"),
    );
    fx.write("scripts/gates.rs2", script);
    fx.write(
        "pack/loc.pack",
        "49=inacfishinggateclosedl\n50=inacfishinggateclosedr\n83=inviswall\n2050=guidorgatelclosed\n2051=guidorgaterclosed\n2052=guidorgatelopen\n2053=guidorgateropen\n2438=grandtree_fencegate_l\n2439=grandtree_fencegate_r\n",
    );
    fx.write(
        "pack/obj.pack",
        "415=ethenea\n416=liquid_honey\n417=sulphuric_broline\n",
    );
    fx.write(
        "pack/varp.pack",
        "68=biohazard\n150=grandtree\n365=mm_main\n",
    );
    fx.write(
        "scripts/gates.constant",
        "^grandtree_released_prison = 80\n^mm_started = 1\n^biohazard_given_distillator = 10\n^biohazard_found_secret = 14\n",
    );
    fx.write(
        "scripts/gates.loc",
        "[grandtree_fencegate_l]\nop1=Open\ncategory=shipyard_gate\n[grandtree_fencegate_r]\nop1=Open\ncategory=shipyard_gate\n[guidorgatelclosed]\nop1=Open\ncategory=varrock_gate\n[guidorgaterclosed]\nop1=Open\ncategory=varrock_gate\n",
    );
    let (left, right) = if shipyard { (2438, 2439) } else { (2050, 2051) };
    write_open_square(&fx, &format!("0 5 46: {left} 0\n0 5 47: {right} 0\n"));
    let defs = loc_defs(&[(left, 1, 1), (right, 1, 1)]);
    let collision = bake_collision(&fx, &defs, &HashSet::new());
    let selected = api::game_data::for_revision(client::io::ClientRevision::R289).unwrap();
    let facts = selected.quest_identity().expect("selected quest facts");
    let quests = QuestSource { facts };
    derive_transports_with_audit(
        fx.path(),
        &defs,
        &collision,
        None,
        Some(&quests),
        None,
        None,
    )
    .0
}

struct ExactSignals(Vec<(&'static str, &'static str, i32)>, EvidenceStamp);
impl EvidenceProvider for ExactSignals {
    fn observe_gate(&self, gate: &QuestGate, required_after: EvidenceStamp) -> Option<Observation> {
        let truth = if !self.1.meets(required_after) {
            Truth::Unknown
        } else if let QuestGate::Window(window) = gate {
            self.0
                .iter()
                .find(|(quest, signal, _)| {
                    window.quest.0.as_ref() == *quest && window.signal.0.as_ref() == *signal
                })
                .map_or(Truth::Unknown, |(_, _, value)| {
                    window.values.test(&InclusiveRange {
                        min: Some(*value),
                        max: Some(*value),
                    })
                })
        } else {
            Truth::Unknown
        };
        Some(Observation {
            truth,
            observed: Some(self.1),
        })
    }
}

fn content_state(grandtree: i32, mm: i32, biohazard: i32) -> WorldState {
    let stamp = EvidenceStamp {
        run: RunKey {
            slot: 1,
            run: 1,
            session: 1,
        },
        tick: 1,
        sequence: 1,
    };
    let mut state = WorldState::empty().with_quest_evidence(QuestEvidence::new(
        Arc::new(ExactSignals(
            vec![
                ("grandtree", "grandtree", grandtree),
                ("mm", "mm_main", mm),
                ("biohazard", "biohazard", biohazard),
            ],
            stamp,
        )),
        stamp,
    ));
    state.map_members = true;
    state.inventory_ready = true;
    state.equipment_ready = true;
    state
}

fn content_crossing(
    graph: &TransportGraph,
    state: &WorldState,
    id: i32,
    z: i32,
    east: bool,
) -> bool {
    let (from_x, to_x) = if east { (2820, 2821) } else { (2821, 2820) };
    graph.edges.iter().any(|edge| {
        edge.loc_id == id
            && edge.takeoff == Some(tile(from_x, z))
            && edge.to == tile(to_x, z)
            && state.allows(edge)
    })
}

#[test]
fn actual_shipyard_handlers_preserve_direct_windows_and_refuse_the_password_arm() {
    if !api::test_support::game_data_available() {
        return;
    }
    let graph = content_gate_graph(SHIPYARD, true);
    let direct = content_state(80, 2, 14);
    let password = content_state(80, 1, 14);
    let unreleased = content_state(79, 2, 14);
    for (id, z) in [(2438, 3438), (2439, 3439)] {
        assert!(content_crossing(&graph, &direct, id, z, true), "entry {id}");
        assert!(content_crossing(&graph, &direct, id, z, false), "exit {id}");
        assert!(!content_crossing(&graph, &password, id, z, true));
        assert!(content_crossing(&graph, &password, id, z, false));
        assert!(!content_crossing(&graph, &unreleased, id, z, true));
        assert!(!content_crossing(&graph, &unreleased, id, z, false));
    }
    assert!(graph
        .edges
        .iter()
        .all(|edge| edge.quest_req.is_empty() && edge.varp_req.is_empty()));
}

#[test]
fn actual_varrock_handlers_keep_membership_and_refuse_the_guard_search_arm() {
    if !api::test_support::game_data_available() {
        return;
    }
    let graph = content_gate_graph(VARROCK, false);
    let direct = content_state(80, 2, 14);
    let searching = content_state(80, 2, 10);
    let mut free = content_state(80, 2, 14);
    free.map_members = false;
    for (id, z) in [(2050, 3438), (2051, 3439)] {
        assert!(content_crossing(&graph, &direct, id, z, true), "entry {id}");
        assert!(content_crossing(&graph, &direct, id, z, false), "exit {id}");
        assert!(!content_crossing(&graph, &searching, id, z, true));
        assert!(content_crossing(&graph, &searching, id, z, false));
        assert!(!content_crossing(&graph, &free, id, z, true));
        assert!(!content_crossing(&graph, &free, id, z, false));
    }
}

#[test]
fn shipyard_interface_close_proof_is_lost_after_a_command_or_yield() {
    if !api::test_support::game_data_available() {
        return;
    }
    for prefix in [
        "p_delay(0);",
        "if_openchat(chat);",
        "if (map_members = true) { if_openchat(chat); }",
    ] {
        let script = SHIPYARD.replace("if_close;", &format!("{prefix} if_close;"));
        let graph = content_gate_graph(&script, true);
        let direct = content_state(80, 2, 14);
        assert!(
            !content_crossing(&graph, &direct, 2438, 3438, true),
            "{prefix}"
        );
        assert!(content_crossing(&graph, &direct, 2438, 3438, false));
    }
}
