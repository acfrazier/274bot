//! Content-gated entrances and the resource regions they serve on the real 289 pack.

use std::sync::{Arc, LazyLock};

use api::game_data;
use api::quest_facts::QuestCatalog;
use api::quest_progress::{EvidenceStamp, QuestListEvidenceProvider};
use api::selected::{ClientRevision, QuestGate, RunKey, Truth};
use api::snapshot::{GameSnapshot, QuestStatusView, SnapshotView, WorldTile};
use nav::quest_gates::{QuestEvidence, QuestFamilyId};
use nav::router::{
    find_first_with, find_missing_item_reqs, find_unresolved_quest_gates, find_with, FindOptions,
    Leg, MissingReq, RouteError,
};
use nav::transport::TransportEdge;
use nav::world::NavWorld;
use nav::WorldState;

fn tile(x: i32, z: i32) -> WorldTile {
    WorldTile { x, z, level: 0 }
}

fn world() -> &'static NavWorld {
    static WORLD: LazyLock<NavWorld> = LazyLock::new(|| {
        let path =
            std::path::PathBuf::from(std::env::var_os("NAV_PACK").expect("explicit NAV_PACK"));
        let mut world = NavWorld::load_pack(&path).expect("real content entrance pack");
        let (_, _, _, flags) = nav::pack::decode_flags_sidecar(
            &std::fs::read(path.with_extension("navflags")).expect("real flags"),
        )
        .expect("decode flags");
        world.collision.attach_flags(flags);
        // These regressions isolate content access; danger policy has its own suite.
        world.graph.zones = None;
        world
    });
    &WORLD
}

fn stamp() -> EvidenceStamp {
    EvidenceStamp {
        run: RunKey {
            slot: 1,
            run: 1,
            session: 1,
        },
        tick: 10,
        sequence: 10,
    }
}

fn completed_state() -> WorldState {
    let selected = game_data::for_revision(ClientRevision::R289).expect("selected revision");
    let (facts, identity) = selected
        .quest_identity_for_bake()
        .expect("verified quest family");
    let catalog = QuestCatalog::from_identity(Some(&facts)).expect("quest catalog");
    let family = QuestFamilyId::new(identity.sha256, identity.schema);
    world()
        .graph
        .admit_quest_family(&family)
        .expect("exact selected quest family");
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    snapshot.seed_quest_statuses(
        catalog
            .iter()
            .enumerate()
            .map(|(index, quest)| QuestStatusView {
                component_id: index as i32,
                name: quest.display.to_string(),
                colour: nav::world_state::QUEST_COMPLETE_COLOUR,
            })
            .collect(),
        true,
    );
    let provider = QuestListEvidenceProvider::from_snapshot(
        &catalog,
        Some(identity),
        Some(identity),
        SnapshotView::new(Some(&snapshot), stamp()),
    );
    WorldState {
        inventory_ready: true,
        equipment_ready: true,
        stats: (0..=30).map(|id| (id, 99)).collect(),
        combat_level: Some(126),
        quests: catalog
            .iter()
            .map(|quest| quest.display.to_string())
            .collect(),
        ..WorldState::empty().with_map_members(true)
    }
    .with_quest_evidence(QuestEvidence::new(Arc::new(provider), family, stamp()))
}

fn ingress(id: i32, gate: impl Fn(&TransportEdge) -> bool) -> &'static TransportEdge {
    let candidates: Vec<_> = world()
        .graph
        .edges
        .iter()
        .filter(|edge| edge.loc_id == id)
        .collect();
    for edge in &candidates {
        println!("entrance loc {id}: {edge:?}");
    }
    candidates
        .into_iter()
        .find(|edge| gate(edge))
        .unwrap_or_else(|| panic!("missing content-gated ingress for loc {id}"))
}

fn resource_targets(region: [i32; 4]) -> Vec<WorldTile> {
    (region[0] - 1..=region[2] + 1)
        .flat_map(|x| (region[1] - 1..=region[3] + 1).map(move |z| tile(x, z)))
        .filter(|&at| world().collision.standable(at))
        .collect()
}

fn route_to_resources(edge: &TransportEdge, region: [i32; 4], state: &WorldState) -> WorldTile {
    let world = world();
    let from = edge.takeoff.unwrap_or(edge.at);
    let targets = resource_targets(region);
    let result = find_first_with(
        &world.collision,
        &world.graph,
        from,
        &targets,
        FindOptions::default(),
        state,
    );
    let route = result.route().unwrap_or_else(|error| {
        panic!(
            "loc {} cannot reach its resource region from {from:?}: {error:?}",
            edge.loc_id
        )
    });
    assert!(
        route
            .legs
            .iter()
            .any(|leg| matches!(leg, Leg::Transport { edge: used } if used.loc_id == edge.loc_id)),
        "resource route must use the qualified entrance"
    );
    println!(
        "resource route loc {}: {from:?} -> {:?}, {} ticks",
        edge.loc_id, route.dest, route.ticks
    );
    route.dest
}

/// quest_grandtree.rs2:1-11: completed quest, one delay, player-relative cellar shift.
#[test]
#[ignore = "requires freshly baked real 289 NAV_PACK and its sidecars"]
fn grand_tree_completion_opens_both_mine_resource_regions() {
    let edge = ingress(2446, |edge| edge.quest_gates.is_some());
    let state = completed_state();
    let gates = edge.quest_gates.as_ref().expect("typed completion gate");
    assert!(gates
        .gates()
        .iter()
        .all(|gate| matches!(gate, QuestGate::Complete(_))));
    assert!(
        edge.quest_req.is_empty(),
        "new gates do not use display-name completion sets"
    );
    assert!(
        edge.varp_req.is_empty(),
        "server-only quest varp is not route evidence"
    );
    assert_eq!(state.quest_gates(edge), Truth::True);
    route_to_resources(edge, [2472, 9905, 2494, 9917], &state);
    route_to_resources(edge, [2435, 9898, 2453, 9915], &state);
    let mut missing = state;
    missing.quest_evidence = None;
    let from = edge.takeoff.unwrap_or(edge.at);
    assert_eq!(
        find_with(
            &world().collision,
            &world().graph,
            from,
            edge.to,
            FindOptions::default(),
            &missing
        ),
        Err(RouteError::NoPath)
    );
    let unresolved = find_unresolved_quest_gates(
        &world().collision,
        &world().graph,
        from,
        &[edge.to],
        FindOptions::default(),
        &missing,
        &[],
    )
    .expect("matching family diagnosis")
    .expect("typed completion requirement");
    assert_eq!(&*unresolved, gates.gates());
}

/// quest_desertrescue.rs2:68-132: metal key held, Ana absent; the guard may search.
#[test]
#[ignore = "requires freshly baked real 289 NAV_PACK and its sidecars"]
fn surface_camp_gate_needs_the_metal_key() {
    let edge = ingress(2673, |edge| {
        edge.item_req.contains(&(1839, 1)) && edge.takeoff.is_some_and(|from| from.x < edge.to.x)
    });
    let mut state = completed_state();
    state.inv.insert(1839, 1);
    assert!(
        edge.forbidden_held.contains(&1842),
        "Ana cannot be carried through the gate"
    );
    route_to_resources(edge, [3284, 3015, 3304, 3025], &state);
    state.inv.clear();
    let from = edge.takeoff.unwrap_or(edge.at);
    assert_eq!(
        find_with(
            &world().collision,
            &world().graph,
            from,
            edge.to,
            FindOptions::default(),
            &state
        ),
        Err(RouteError::NoPath)
    );
    let missing = find_missing_item_reqs(
        &world().collision,
        &world().graph,
        from,
        edge.to,
        FindOptions::default(),
        &state,
    )
    .expect("typed key diagnosis");
    assert!(missing.contains(&MissingReq::Carry { id: 1839, count: 1 }));
}

/// quest_desertrescue.rs2:402-469: all three slave-kit pieces worn, then two movements.
#[test]
#[ignore = "requires freshly baked real 289 NAV_PACK and its sidecars"]
fn underground_camp_requires_all_kit_pieces_worn_and_reaches_lower_mine() {
    let edge = ingress(2675, |edge| edge.worn_all_req == [1844, 1845, 1846]);
    let mut state = completed_state();
    state.worn.extend([1844, 1845, 1846]);
    assert_eq!(
        edge.to,
        tile(3278, 9427),
        "the inner door step is part of the proven landing"
    );
    route_to_resources(edge, [3284, 9412, 3306, 9453], &state);
    let from = edge.takeoff.unwrap_or(edge.at);
    // A kit and even the northern key do not bridge the intervening minecart:
    // quest_desertrescue.rs2:808-834 requires a dialogue and a chance check.
    // The separate site stays hidden until prerequisite steps represent this
    // outcome and bound retries of observed failures.
    state.inv.insert(1843, 1);
    let northern_targets = resource_targets([3288, 9462, 3324, 9469]);
    let northern = find_first_with(
        &world().collision,
        &world().graph,
        from,
        &northern_targets,
        FindOptions::default(),
        &state,
    );
    assert!(matches!(northern.route(), Err(RouteError::NoPath)));
    for missing in [1844, 1845, 1846] {
        let mut partial = state.clone();
        partial.worn.remove(&missing);
        partial.inv.insert(missing, 1);
        assert_eq!(
            find_with(
                &world().collision,
                &world().graph,
                from,
                edge.to,
                FindOptions::default(),
                &partial
            ),
            Err(RouteError::NoPath),
            "carrying piece {missing} cannot replace wearing it"
        );
    }
}

/// jail_doors.rs2:8-12,21-35: use a held dusty key; the reverse side is free.
#[test]
#[ignore = "requires freshly baked real 289 NAV_PACK and its sidecars"]
fn dusty_key_door_opens_the_lava_eel_resource_route() {
    let edge = ingress(2623, |edge| edge.item_req.contains(&(1590, 1)));
    let mut state = completed_state();
    state.inv.insert(1590, 1);
    route_to_resources(edge, [2889, 9766, 2891, 9766], &state);
    state.inv.clear();
    let from = edge.takeoff.unwrap_or(edge.at);
    assert_eq!(
        find_with(
            &world().collision,
            &world().graph,
            from,
            edge.to,
            FindOptions::default(),
            &state
        ),
        Err(RouteError::NoPath)
    );
    let missing = find_missing_item_reqs(
        &world().collision,
        &world().graph,
        from,
        edge.to,
        FindOptions::default(),
        &state,
    )
    .expect("typed dusty-key diagnosis");
    assert!(missing.contains(&MissingReq::Carry { id: 1590, count: 1 }));
}
