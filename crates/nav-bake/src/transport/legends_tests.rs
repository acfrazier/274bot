use std::sync::Arc;

use super::*;
use api::prereq_knowledge::Observation;
use api::quest_progress::{EvidenceProvider, EvidenceStamp};
use api::selected::{FactKey, InclusiveRange, QuestGate, RunKey, Truth};
const GATE_LOC_IDS: [i32; 2] = [2391, 2392];
const MAP_IDS: [i32; 2] = [714, 715];
const AXE_IDS: [i32; 7] = [1349, 1351, 1353, 1355, 1357, 1359, 1361];
const MACHETTE_ID: i32 = 975;
const JUNGLE_LOC_IDS: [i32; 5] = [2887, 2889, 2890, 2892, 2893];

struct LegendsStageEvidence {
    value: i32,
    stamp: EvidenceStamp,
}

impl EvidenceProvider for LegendsStageEvidence {
    fn observe_gate(&self, gate: &QuestGate, required_after: EvidenceStamp) -> Option<Observation> {
        let truth = if !self.stamp.meets(required_after) {
            Truth::Unknown
        } else {
            match gate {
                QuestGate::Window(window)
                    if window.quest.0.as_ref() == "legends"
                        && window.signal.0.as_ref() == "legendsquest" =>
                {
                    window.values.test(&InclusiveRange {
                        min: Some(self.value),
                        max: Some(self.value),
                    })
                }
                _ => Truth::Unknown,
            }
        };
        Some(Observation {
            truth,
            observed: Some(self.stamp),
        })
    }
}

fn legends_progress_evidence(
    family: nav_planner::quest_gates::QuestFamilyId,
    value: i32,
) -> nav_planner::quest_gates::QuestEvidence {
    let stamp = EvidenceStamp {
        run: RunKey {
            slot: 1,
            run: 1,
            session: 1,
        },
        tick: 1,
        sequence: 1,
    };
    nav_planner::quest_gates::QuestEvidence::new(
        Arc::new(LegendsStageEvidence { value, stamp }),
        family,
        stamp,
    )
}

#[test]
fn legends_stage_observation_retains_its_own_stamp_and_refuses_a_later_floor() {
    let stamp = EvidenceStamp {
        run: RunKey {
            slot: 1,
            run: 1,
            session: 1,
        },
        tick: 7,
        sequence: 3,
    };
    let provider = LegendsStageEvidence { value: 1, stamp };
    let gate = QuestGate::Window(api::selected::StageWindow {
        quest: FactKey::new("legends"),
        signal: FactKey::new("legendsquest"),
        values: InclusiveRange {
            min: Some(1),
            max: None,
        },
    });
    let observed = provider.observe_gate(&gate, stamp).unwrap();
    assert_eq!(observed.truth, Truth::True);
    assert_eq!(observed.observed, Some(stamp));
    let later = EvidenceStamp { tick: 8, ..stamp };
    let stale = provider.observe_gate(&gate, later).unwrap();
    assert_eq!(stale.truth, Truth::Unknown);
    assert_eq!(stale.observed, Some(stamp));
    assert_eq!(provider.test_gate(&gate, later), Truth::Unknown);
}

#[test]
fn legends_gate_requires_started_progress_and_crosses_each_leaf_both_ways() {
    let fx = Fixture::new();
    write_legends_content(&fx);
    fx.write(
        "maps/m42_52.jm2",
        "==== LOC ====\n0 40 21: 2391 0 1\n0 41 21: 2392 0 1\n",
    );
    let selected = api::game_data::for_revision(client::io::ClientRevision::R289).unwrap();
    let (facts, identity) = selected.quest_identity_for_bake().unwrap();
    let family = nav_planner::quest_gates::QuestFamilyId::new(identity.sha256, identity.schema);
    let quests = QuestSource {
        family,
        facts: &facts,
    };
    let collision = open_collision(2724, 3345, 12, 12);
    let (graph, _, _) = super::super::derive_transports_with_audit(
        fx.path(),
        &loc_defs(&[]),
        &collision,
        None,
        Some(&quests),
        None,
    );

    let gate_edges = graph
        .edges
        .iter()
        .filter(|edge| GATE_LOC_IDS.contains(&edge.loc_id) && edge.open_loc_id.is_some())
        .collect::<Vec<_>>();
    assert_eq!(
        gate_edges.len(),
        4,
        "expected both directions of both leaves"
    );
    for (loc_id, x, open_id) in [(2391, 2728, 1562), (2392, 2729, 1563)] {
        let crossings = gate_edges
            .iter()
            .copied()
            .filter(|edge| edge.loc_id == loc_id)
            .collect::<Vec<_>>();
        assert_eq!(crossings.len(), 2, "gate {loc_id} should cross both ways");
        assert_eq!(
            crossings
                .iter()
                .map(|edge| edge.dir.unwrap())
                .collect::<HashSet<_>>(),
            HashSet::from([DoorDir::N, DoorDir::S]),
        );
        for edge in crossings {
            assert_eq!(edge.kind, TransportKind::Door);
            assert_eq!(
                edge.at,
                WorldTile {
                    x,
                    z: 3349,
                    level: 0
                }
            );
            assert_eq!(edge.open_loc_id, Some(open_id));
            assert_eq!(edge.option, 1);
            assert_eq!(edge.takeoff, None);
            assert_eq!(edge.player_delta, None);
            assert!(edge.item_req.is_empty());
            assert!(edge.varp_req.is_empty());
            let Some(gates) = edge.quest_gates.as_ref() else {
                panic!("the gate carries Legends stage evidence");
            };
            let [QuestGate::Window(window)] = gates.gates() else {
                panic!("the gate carries one Legends stage window");
            };
            assert_eq!(window.quest, FactKey::new("legends"));
            assert_eq!(window.signal, FactKey::new("legendsquest"));
            assert_eq!(
                window.values,
                InclusiveRange {
                    min: Some(1),
                    max: None,
                }
            );

            let mut started = nav_planner::world_state::WorldState::empty();
            started.quest_evidence = Some(legends_progress_evidence(family, 1));
            assert!(started.allows(edge));
            let mut not_started = nav_planner::world_state::WorldState::empty();
            not_started.quest_evidence = Some(legends_progress_evidence(family, 0));
            assert!(!not_started.allows(edge));
            let mut varp_only = nav_planner::world_state::WorldState::empty();
            varp_only.varps.insert(139, 1);
            assert!(!varp_only.allows(edge));
        }
    }

    let (without_quest, _, _) = super::super::derive_transports_with_audit(
        fx.path(),
        &loc_defs(&[]),
        &collision,
        None,
        None,
        None,
    );
    assert!(!without_quest
        .edges
        .iter()
        .any(|edge| GATE_LOC_IDS.contains(&edge.loc_id) && edge.open_loc_id.is_some()));
}

#[test]
fn every_mature_jungle_loc_has_map_or_completion_tool_and_takeoff_gates() {
    let fx = Fixture::new();
    write_legends_content(&fx);
    write_jungle_map(&fx, &JUNGLE_LOC_IDS);
    let selected = api::game_data::for_revision(client::io::ClientRevision::R289).unwrap();
    let (facts, identity) = selected.quest_identity_for_bake().unwrap();
    let quests = QuestSource {
        family: nav_planner::quest_gates::QuestFamilyId::new(identity.sha256, identity.schema),
        facts: &facts,
    };
    let defs = loc_defs(&[
        (2887, 2, 2),
        (2889, 1, 1),
        (2890, 1, 1),
        (2892, 1, 1),
        (2893, 1, 1),
    ]);
    let collision = open_collision(90, 90, 80, 40);
    let (graph, _, _) = super::super::derive_transports_with_audit(
        fx.path(),
        &defs,
        &collision,
        None,
        Some(&quests),
        None,
    );

    for (index, loc_id) in JUNGLE_LOC_IDS.into_iter().enumerate() {
        let edges = graph
            .edges
            .iter()
            .filter(|edge| edge.kind == TransportKind::AgilityShortcut && edge.loc_id == loc_id)
            .collect::<Vec<_>>();
        assert!(!edges.is_empty(), "no jungle crossing for loc {loc_id}");
        let mut groups = HashMap::<(i32, i32, i32, i32, i32, i32), usize>::new();
        for edge in &edges {
            let takeoff = edge.takeoff.expect("jungle edge pins its operable stand");
            *groups
                .entry((
                    takeoff.x,
                    takeoff.z,
                    takeoff.level,
                    edge.to.x,
                    edge.to.z,
                    edge.to.level,
                ))
                .or_default() += 1;
            assert_eq!(
                edge.at,
                WorldTile {
                    x: 100 + index as i32 * 6,
                    z: 100,
                    level: 0
                }
            );
            assert_eq!(edge.option, 1);
            assert_eq!(edge.player_delta, None);
            assert_eq!(edge.open_loc_id, None);
            // jungle_tree.rs2:61-69,115-118: every tool/unlock variant
            // retains the first successful roll's content minimum.
            assert_eq!(
                edge.ticks,
                if matches!(loc_id, 2892 | 2893) { 3 } else { 5 }
            );
            assert!(edge.skill_req.is_empty(), "axes have no level gate");
            let has_map = edge
                .item_req
                .iter()
                .any(|(id, count)| MAP_IDS.contains(id) && *count == 1);
            let has_complete = edge.quest_gates.as_ref().is_some_and(|gates| {
                gates.gates() == [QuestGate::Complete(FactKey::new("legends"))]
            });
            assert_ne!(
                has_map, has_complete,
                "one map item or complete Legends is required"
            );

            let inventory_machette = edge.item_req.contains(&(MACHETTE_ID, 1));
            let worn_machette = edge.worn_all_req == [MACHETTE_ID];
            assert_ne!(
                inventory_machette, worn_machette,
                "machette must be held or worn"
            );

            let inventory_axe = edge
                .item_req
                .iter()
                .any(|(id, count)| AXE_IDS.contains(id) && *count == 1);
            let worn_axe = edge.worn_req == AXE_IDS;
            assert_ne!(
                inventory_axe, worn_axe,
                "an axe may be carried or worn in the right hand"
            );
            assert!(
                !(worn_machette && worn_axe),
                "machette and axe cannot both occupy the right hand"
            );
        }
        let carried_tools = edges
            .iter()
            .copied()
            .find(|edge| {
                edge.quest_gates.is_none()
                    && edge.worn_req.is_empty()
                    && edge.worn_all_req.is_empty()
                    && edge
                        .item_req
                        .iter()
                        .any(|(id, count)| MAP_IDS.contains(id) && *count == 1)
                    && edge.item_req.contains(&(MACHETTE_ID, 1))
                    && edge
                        .item_req
                        .iter()
                        .any(|(id, count)| AXE_IDS.contains(id) && *count == 1)
            })
            .expect("map, machette and carried axe variant");
        let map_id = carried_tools
            .item_req
            .iter()
            .find_map(|(id, count)| (MAP_IDS.contains(id) && *count == 1).then_some(*id))
            .expect("map prerequisite");
        let axe_id = carried_tools
            .item_req
            .iter()
            .find_map(|(id, count)| (AXE_IDS.contains(id) && *count == 1).then_some(*id))
            .expect("axe prerequisite");
        let mut ready = nav_planner::world_state::WorldState::empty();
        ready.map_members = true;
        ready.inventory_ready = true;
        ready.equipment_ready = true;
        for &(id, count) in &carried_tools.item_req {
            ready.inv.insert(id, count);
        }
        assert!(
            ready.allows(carried_tools),
            "loc {loc_id} must admit its map route"
        );
        for (missing_id, label) in [(map_id, "map"), (MACHETTE_ID, "machette"), (axe_id, "axe")] {
            let mut missing = ready.clone();
            missing.inv.remove(&missing_id);
            assert!(
                !missing.allows(carried_tools),
                "loc {loc_id} must refuse when {label} is missing"
            );
        }

        let worn_axe = edges
            .iter()
            .copied()
            .find(|edge| {
                edge.quest_gates.is_none()
                    && edge.worn_req == AXE_IDS
                    && edge.worn_all_req.is_empty()
                    && edge.item_req.contains(&(MACHETTE_ID, 1))
                    && edge
                        .item_req
                        .iter()
                        .any(|(id, count)| MAP_IDS.contains(id) && *count == 1)
            })
            .expect("map, machette and right-hand axe variant");
        let mut worn_axe_state = nav_planner::world_state::WorldState::empty();
        worn_axe_state.map_members = true;
        worn_axe_state.inventory_ready = true;
        worn_axe_state.equipment_ready = true;
        for &(id, count) in &worn_axe.item_req {
            worn_axe_state.inv.insert(id, count);
        }
        worn_axe_state.worn.insert(AXE_IDS[0]);
        assert!(
            worn_axe_state.allows(worn_axe),
            "loc {loc_id} must admit a worn axe"
        );

        let worn_machette = edges
            .iter()
            .copied()
            .find(|edge| {
                edge.quest_gates.is_none()
                    && edge.worn_req.is_empty()
                    && edge.worn_all_req == [MACHETTE_ID]
                    && edge
                        .item_req
                        .iter()
                        .any(|(id, count)| MAP_IDS.contains(id) && *count == 1)
                    && edge
                        .item_req
                        .iter()
                        .any(|(id, count)| AXE_IDS.contains(id) && *count == 1)
            })
            .expect("map, worn machette and carried axe variant");
        let mut worn_machette_state = nav_planner::world_state::WorldState::empty();
        worn_machette_state.map_members = true;
        worn_machette_state.inventory_ready = true;
        worn_machette_state.equipment_ready = true;
        for &(id, count) in &worn_machette.item_req {
            worn_machette_state.inv.insert(id, count);
        }
        worn_machette_state.worn.insert(MACHETTE_ID);
        assert!(
            worn_machette_state.allows(worn_machette),
            "loc {loc_id} must admit a worn machette"
        );
        assert!(
            groups.values().all(|&count| count == 45),
            "each exact takeoff/landing needs 3 map unlocks × 15 compatible tool states: {groups:?}"
        );
    }
}

#[test]
fn jungle_crossing_accepts_blocked_jungle_landing_only() {
    let selected = api::game_data::for_revision(client::io::ClientRevision::R289).unwrap();
    let (facts, identity) = selected.quest_identity_for_bake().unwrap();
    let quests = QuestSource {
        family: nav_planner::quest_gates::QuestFamilyId::new(identity.sha256, identity.schema),
        facts: &facts,
    };
    let defs = loc_defs(&[(2887, 2, 2), (2889, 1, 1)]);
    let blocked_tile = WorldTile {
        x: 101,
        z: 100,
        level: 0,
    };
    let mut collision = open_collision(90, 90, 80, 40);
    let blocked_index = (blocked_tile.z - collision.origin.z) as usize * collision.width
        + (blocked_tile.x - collision.origin.x) as usize;
    collision.flags.as_mut().unwrap()[blocked_index] |= nav_planner::collision::SQ_BLOCKED;

    let with_jungle_landing = Fixture::new();
    write_legends_content(&with_jungle_landing);
    with_jungle_landing.write(
        "maps/m1_1.jm2",
        "==== LOC ====\n0 36 36: 2887 10 0\n0 37 36: 2889 10 0\n",
    );
    let (graph, _, _) = super::super::derive_transports_with_audit(
        with_jungle_landing.path(),
        &defs,
        &collision,
        None,
        Some(&quests),
        None,
    );
    let takeoff = WorldTile {
        x: 99,
        z: 100,
        level: 0,
    };
    let edge = graph
        .edges
        .iter()
        .find(|edge| {
            edge.loc_id == 2887
                && edge.takeoff == Some(takeoff)
                && edge.to == blocked_tile
                && edge.quest_gates.is_none()
                && edge.worn_req.is_empty()
                && edge.worn_all_req.is_empty()
                && edge
                    .item_req
                    .iter()
                    .any(|(id, count)| MAP_IDS.contains(id) && *count == 1)
                && edge
                    .item_req
                    .iter()
                    .any(|(id, count)| *id == MACHETTE_ID && *count == 1)
                && edge
                    .item_req
                    .iter()
                    .any(|(id, count)| AXE_IDS.contains(id) && *count == 1)
        })
        .expect("the carried map, machete, and axe permit this jungle landing");
    let mut ready = nav_planner::world_state::WorldState::empty();
    ready.map_members = true;
    ready.inventory_ready = true;
    ready.equipment_ready = true;
    for &(id, count) in &edge.item_req {
        ready.inv.insert(id, count);
    }
    assert!(ready.allows(edge));

    let without_jungle_landing = Fixture::new();
    write_legends_content(&without_jungle_landing);
    without_jungle_landing.write("maps/m1_1.jm2", "==== LOC ====\n0 36 36: 2887 10 0\n");
    let (graph, _, _) = super::super::derive_transports_with_audit(
        without_jungle_landing.path(),
        &defs,
        &collision,
        None,
        Some(&quests),
        None,
    );
    assert!(
        !graph.edges.iter().any(|edge| {
            edge.loc_id == 2887 && edge.takeoff == Some(takeoff) && edge.to == blocked_tile
        }),
        "an arbitrary blocked tile remains an invalid landing"
    );
}

#[test]
fn jungle_refuses_to_bake_when_a_required_tool_or_proven_map_is_missing() {
    let selected = api::game_data::for_revision(client::io::ClientRevision::R289).unwrap();
    let (facts, identity) = selected.quest_identity_for_bake().unwrap();
    let quests = QuestSource {
        family: nav_planner::quest_gates::QuestFamilyId::new(identity.sha256, identity.schema),
        facts: &facts,
    };
    let defs = loc_defs(&[(2887, 2, 2)]);
    let collision = open_collision(90, 90, 30, 30);
    let complete = Fixture::new();
    write_legends_content(&complete);
    write_jungle_map(&complete, &[2887]);
    let (ready, _, _) = super::super::derive_transports_with_audit(
        complete.path(),
        &defs,
        &collision,
        None,
        Some(&quests),
        None,
    );
    assert!(
        ready
            .edges
            .iter()
            .any(|edge| edge.kind == TransportKind::AgilityShortcut && edge.loc_id == 2887),
        "the positive control must emit the jungle crossing"
    );

    for (obj_pack, quest_source, missing) in [
        (
            "975=machette\n1349=iron_axe\n1351=bronze_axe\n1353=steel_axe\n1355=mithril_axe\n1357=adamant_axe\n1359=rune_axe\n1361=black_axe\n",
            None,
            "map or completed-quest proof",
        ),
        (
            "714=thkaramjamap\n715=thkaramjamapcomp\n1349=iron_axe\n1351=bronze_axe\n1353=steel_axe\n1355=mithril_axe\n1357=adamant_axe\n1359=rune_axe\n1361=black_axe\n",
            Some(&quests),
            "machette",
        ),
        (
            "714=thkaramjamap\n715=thkaramjamapcomp\n975=machette\n",
            Some(&quests),
            "axe",
        ),
    ] {
        let fx = Fixture::new();
        write_legends_content(&fx);
        fx.write("pack/obj.pack", obj_pack);
        write_jungle_map(&fx, &[2887]);
        let (graph, _, _) = super::super::derive_transports_with_audit(
            fx.path(),
            &defs,
            &collision,
            None,
            quest_source,
            None,
        );
        assert!(
            !graph.edges.iter().any(|edge| {
                edge.kind == TransportKind::AgilityShortcut && edge.loc_id == 2887
            }),
            "jungle edge must be absent without {missing}"
        );
    }
}

#[test]
fn legends_gate_engine_closure_follows_helpers_into_general_scripts() {
    const MULTIPLY2_BLOCK: &str = "// general/scripts/misc/math_procs.rs2:1\n[proc,multiply2](int $int1, int $int2, int $m)(int, int)\n$int1 = calc($int1 * $m);\n$int2 = calc($int2 * $m);\nreturn($int1, $int2);\n";
    let fx = Fixture::new();
    write_legends_content(&fx);
    let engine_procs = super::super::stage_doors::ENGINE_DOOR_PROCS;
    let door_procs = engine_procs.replace(MULTIPLY2_BLOCK, "");
    assert_ne!(
        door_procs, engine_procs,
        "multiply2 must be moved to its source tree"
    );
    fx.write(
        "scripts/doors/scripts/open_and_close_double_doors.rs2",
        &door_procs,
    );
    fx.write(
        "scripts/general/scripts/misc/math_procs.rs2",
        MULTIPLY2_BLOCK,
    );
    fx.write(
        "maps/m42_52.jm2",
        "==== LOC ====\n0 40 21: 2391 0 1\n0 41 21: 2392 0 1\n",
    );
    let selected = api::game_data::for_revision(client::io::ClientRevision::R289).unwrap();
    let (facts, identity) = selected.quest_identity_for_bake().unwrap();
    let quests = QuestSource {
        family: nav_planner::quest_gates::QuestFamilyId::new(identity.sha256, identity.schema),
        facts: &facts,
    };
    let (graph, _, _) = super::super::derive_transports_with_audit(
        fx.path(),
        &loc_defs(&[]),
        &open_collision(2724, 3345, 12, 12),
        None,
        Some(&quests),
        None,
    );
    assert_eq!(
        graph
            .edges
            .iter()
            .filter(|edge| GATE_LOC_IDS.contains(&edge.loc_id) && edge.open_loc_id.is_some())
            .count(),
        4,
        "the gate proof must follow the helper into its general script"
    );
}

fn write_legends_content(fx: &Fixture) {
    fx.write(
        "scripts/quests/quest_legends/scripts/legends_gate.rs2",
        include_str!("legends_gate_template.rs2"),
    );
    fx.write(
        "scripts/quests/quest_legends/scripts/jungle_tree.rs2",
        include_str!("legends_jungle_template.rs2"),
    );
    fx.write(
        "scripts/skill_woodcutting/scripts/woodcut.rs2",
        include_str!("legends_axe_template.rs2"),
    );
    fx.write(
        "scripts/general/scripts/misc/coord_procs.rs2",
        include_str!("legends_coords_template.rs2"),
    );
    fx.write(
        "scripts/doors/scripts/open_and_close_double_doors.rs2",
        super::super::stage_doors::ENGINE_DOOR_PROCS,
    );
    fx.write(
        "scripts/quests/quest_legends/configs/quest_legends.constant",
        "^legends_not_started=0\n",
    );
    fx.write(
        "scripts/general/configs/quest.constant",
        "^legends_complete=75\n",
    );
    fx.write(
        "scripts/general/configs/wearpos.constant",
        "^wearpos_rhand=0\n",
    );
    fx.write(
        "scripts/quests/quest_legends/configs/quest_legends.loc",
        "[legendsguildgatel]\nactive=yes\nop1=Open\ncategory=double_door_open_and_close_left\nparam=next_loc_stage,loc_1562\n\
         [legendsguildgater]\nactive=yes\nop1=Open\ncategory=double_door_open_and_close_right\nparam=next_loc_stage,loc_1563\n\
         [kharazi_jungle_tree_logs]\nop1=Chop-down\nblockwalk=no\ncategory=jungle_tree\nparam=next_loc_stage,junglestump_kharazi\n\
         [kharazi_jungle_tree1]\nop1=Chop-down\nblockwalk=no\ncategory=jungle_tree\nparam=next_loc_stage,junglestump_kharazi\n\
         [kharazi_jungle_tree2]\nop1=Chop-down\nblockwalk=no\ncategory=jungle_tree\nparam=next_loc_stage,junglestump_kharazi\n\
         [kharazi_jungle_plant1]\nop1=Chop-down\nblockwalk=no\ncategory=jungle_bush\nparam=next_loc_stage,kharazi_jungle_plant1_slashed\n\
         [kharazi_jungle_plant2]\nop1=Chop-down\nblockwalk=no\ncategory=jungle_bush\nparam=next_loc_stage,kharazi_jungle_plant1_slashed\n",
    );
    fx.write(
        "scripts/_unpack/225/all.obj",
        "[machette]\nwearpos=righthand\n",
    );
    fx.write(
        "scripts/skill_woodcutting/configs/axes/axes.obj",
        "[rune_axe]\nwearpos=righthand\n[adamant_axe]\nwearpos=righthand\n\
         [mithril_axe]\nwearpos=righthand\n[black_axe]\nwearpos=righthand\n\
         [steel_axe]\nwearpos=righthand\n[iron_axe]\nwearpos=righthand\n\
         [bronze_axe]\nwearpos=righthand\n",
    );
    fx.write(
        "pack/loc.pack",
        "2391=legendsguildgatel\n2392=legendsguildgater\n1562=loc_1562\n1563=loc_1563\n\
         2887=kharazi_jungle_tree_logs\n2889=kharazi_jungle_tree1\n\
         2890=kharazi_jungle_tree2\n2892=kharazi_jungle_plant1\n\
         2893=kharazi_jungle_plant2\n2891=junglestump_kharazi\n\
         2894=kharazi_jungle_plant1_slashed\n",
    );
    fx.write(
        "pack/obj.pack",
        "714=thkaramjamap\n715=thkaramjamapcomp\n975=machette\n\
         1359=rune_axe\n1357=adamant_axe\n1355=mithril_axe\n\
         1361=black_axe\n1353=steel_axe\n1349=iron_axe\n1351=bronze_axe\n",
    );
    fx.write("pack/varp.pack", "139=legendsquest\n");
}

fn write_jungle_map(fx: &Fixture, ids: &[i32]) {
    let mut text = String::from("==== LOC ====\n");
    for (index, id) in ids.iter().copied().enumerate() {
        let local_x = 36 + index as i32 * 6;
        text.push_str(&format!("0 {local_x} 36: {id} 10 0\n"));
    }
    fx.write("maps/m1_1.jm2", &text);
}

fn open_collision(x: i32, z: i32, width: usize, height: usize) -> WorldCollision {
    let cells = width * height * nav_planner::collision::LEVELS;
    WorldCollision {
        origin: WorldTile { x, z, level: 0 },
        width,
        height,
        walk: vec![0; cells],
        blocked: vec![0; cells.div_ceil(64)],
        flags: Some(vec![0; cells]),
    }
}
