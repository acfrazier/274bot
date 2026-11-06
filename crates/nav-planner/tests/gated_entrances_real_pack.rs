//! Content-gated entrances and the resource regions they serve on the real 289 pack.

use std::collections::HashMap;
use std::num::NonZeroU32;
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use api::game_data;
use api::identity::Deadline;
use api::prereq_knowledge::Observation;
use api::quest_facts::QuestCatalog;
use api::quest_progress::{EvidenceProvider, EvidenceStamp, QuestListEvidenceProvider};
#[expect(
    clippy::disallowed_types,
    reason = "read-only revision value selects real-pack fixture data and does not perform an action"
)]
use api::selected::ClientRevision;
use api::selected::{FamilyPreparation, QuestGate, RunKey, StageWindow, Truth};
use api::snapshot::{GameSnapshot, QuestStatusView, SnapshotView, WorldTile};
use api::work::{PreparationPhase, SharedCancellation, SliceBudget, StepContext, SystemClock};
use nav_planner::quest_gates::QuestEvidence;
use nav_planner::router::{
    find_actionable_quest_gates, find_first_with, find_missing_item_reqs,
    find_unresolved_quest_gates, find_with, FindOptions, Leg, MissingReq, RouteError,
};
use nav_planner::transport::TransportEdge;
use nav_planner::world::NavWorld;
use nav_planner::WorldState;

fn tile(x: i32, z: i32) -> WorldTile {
    WorldTile { x, z, level: 0 }
}

fn world() -> &'static NavWorld {
    static WORLD: LazyLock<NavWorld> = LazyLock::new(|| {
        let path =
            std::path::PathBuf::from(std::env::var_os("NAV_PACK").expect("explicit NAV_PACK"));
        let mut world = NavWorld::load_pack(&path).expect("real content entrance pack");
        let (_, _, _, flags) = nav_planner::pack::decode_flags_sidecar(
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
    let facts = selected.quest_identity().expect("selected quest facts");
    let catalog = QuestCatalog::from_identity(Some(facts)).expect("quest catalog");
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    snapshot.seed_quest_statuses(
        catalog
            .iter()
            .enumerate()
            .map(|(index, quest)| QuestStatusView {
                component_id: index as i32,
                name: quest.display.to_string(),
                colour: nav_planner::world_state::QUEST_COMPLETE_COLOUR,
            })
            .collect(),
        true,
    );
    let provider = QuestListEvidenceProvider::from_snapshot(
        &catalog,
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
    .with_quest_evidence(QuestEvidence::new(Arc::new(provider), stamp()))
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

/// Fixed route evidence for comparing strict A1/A2 passage behavior by truth state.
/// All unrelated quest gates stay proven so these two passages are the only changed facts.
#[derive(Clone, Copy)]
struct MorytaniaGateEvidence(Truth);

impl EvidenceProvider for MorytaniaGateEvidence {
    fn observe_gate(&self, gate: &QuestGate, required_after: EvidenceStamp) -> Option<Observation> {
        let observed = stamp();
        if !observed.meets(required_after) {
            return None;
        }
        let truth = match gate {
            QuestGate::Window(window)
                if window.quest.0.as_ref() == "priestperil"
                    && window.signal.0.as_ref() == "priestperil"
                    && window.values.min == Some(61)
                    && window.values.max == Some(61) =>
            {
                self.0
            }
            QuestGate::Window(window)
                if window.quest.0.as_ref() == "druidspirit"
                    && window.signal.0.as_ref() == "druidspirit"
                    && window.values.min == Some(1)
                    && window.values.max == Some(110) =>
            {
                self.0
            }
            _ => Truth::True,
        };
        Some(Observation {
            truth,
            observed: Some(observed),
        })
    }
}

fn prerequisite_catalog() -> Arc<api::prereq_steps::PrereqCatalog> {
    let selected = game_data::for_revision(ClientRevision::R289).expect("selected revision");
    let clock = SystemClock;
    let cancel = SharedCancellation::new();
    let mut step = StepContext::<PreparationPhase>::new(
        &clock,
        SliceBudget::new(
            NonZeroU32::MIN,
            Deadline::at(Instant::now() + Duration::from_secs(60)),
        ),
        &cancel,
    );
    FamilyPreparation::within(&mut step, |worker| selected.prepare_prereq_steps(worker))
        .expect("verified prerequisite catalog")
}

fn morytania_state(
    base: &WorldState,
    truth: Truth,
    establishable: &Arc<[StageWindow]>,
) -> WorldState {
    base.clone()
        .with_quest_evidence(QuestEvidence::new(
            Arc::new(MorytaniaGateEvidence(truth)),
            stamp(),
        ))
        .with_establishable(Arc::clone(establishable))
}

fn is_a1_gate(gate: &QuestGate) -> bool {
    matches!(
        gate,
        QuestGate::Window(window)
            if window.quest.0.as_ref() == "priestperil"
                && window.signal.0.as_ref() == "priestperil"
                && window.values.min == Some(61)
                && window.values.max == Some(61)
    )
}

fn is_a2_gate(gate: &QuestGate) -> bool {
    matches!(
        gate,
        QuestGate::Window(window)
            if window.quest.0.as_ref() == "druidspirit"
                && window.signal.0.as_ref() == "druidspirit"
                && window.values.min == Some(1)
                && window.values.max == Some(110)
    )
}

/// This real-pack matrix covers the ten newly reachable Morytania sites.
/// It is not the separate 1,378-search, five-mode ordinary route corpus.
#[test]
#[ignore = "requires freshly baked real 289 NAV_PACK and its sidecars"]
fn morytania_sites_compare_proof_unknown_and_false_gate_routes() {
    const SITES: [(&str, [i32; 4]); 10] = [
        ("fishing.canifis.sw", [3479, 3430, 3490, 3449]),
        ("fishing.mort_ton.w", [3437, 3271, 3445, 3280]),
        ("fishing.river_salve.se", [3424, 3407, 3440, 3417]),
        ("woodcutting.canifis", [3527, 3465, 3540, 3482]),
        ("woodcutting.haunted_woods", [3569, 3461, 3597, 3503]),
        ("woodcutting.haunted_woods.2", [3549, 3483, 3555, 3508]),
        ("woodcutting.haunted_woods.e", [3600, 3467, 3641, 3511]),
        ("woodcutting.mort_ton", [3459, 3264, 3516, 3319]),
        ("woodcutting.river_salve.ne", [3422, 3459, 3422, 3459]),
        ("woodcutting.temple", [3430, 3457, 3443, 3498]),
    ];

    let mut crossing_ids = HashMap::new();
    for (index, edge) in world()
        .graph
        .edges
        .iter()
        .chain(&world().graph.teleports)
        .enumerate()
    {
        let crossing = edge
            .crossing_id()
            .expect("representable real-pack crossing");
        let physical = (
            edge.kind,
            edge.loc_id,
            edge.option,
            edge.at,
            crossing.takeoff,
            edge.to,
        );
        if let Some(previous) = crossing_ids.insert(crossing, physical) {
            assert_eq!(
                previous, physical,
                "different physical crossings share an id at edge {index}: {crossing:?}"
            );
        }
    }
    println!("real-pack unique crossing ids: {}", crossing_ids.len());
    println!(
        "real-pack census: ordinary_edges={} teleports={} quest_gated_edges={}",
        world().graph.edges.len(),
        world().graph.teleports.len(),
        world()
            .graph
            .edges
            .iter()
            .filter(|edge| edge.quest_gates.is_some())
            .count(),
    );

    let a1 = ingress(3443, |edge| {
        edge.option == 1 && edge.at == tile(3440, 9886) && edge.to == tile(3423, 3485)
    });
    assert!(
        a1.quest_gates
            .as_ref()
            .is_some_and(|gates| gates.gates().iter().any(is_a1_gate)),
        "the real A1 crossing carries the exact [61,61] stage gate"
    );
    let a1_directions: Vec<_> = world()
        .graph
        .edges
        .iter()
        .enumerate()
        .filter(|(_, edge)| edge.loc_id == a1.loc_id && edge.at == a1.at && edge.to == a1.to)
        .collect();
    assert_eq!(
        a1_directions.len(),
        2,
        "both exact A1 takeoffs are retained"
    );
    assert!(
        !world().collision.standable(a1.at),
        "the barrier anchor is blocked"
    );
    for (index, edge) in &a1_directions {
        let crossing = edge.crossing_id().unwrap();
        assert_eq!(crossing.at, a1.at);
        assert_ne!(crossing.takeoff, crossing.at);
        assert!(world().collision.standable(crossing.takeoff));
        assert!(world()
            .graph
            .admissible_from(&world().collision, *index, crossing.takeoff));
    }
    let mut a1_takeoffs: Vec<_> = a1_directions
        .iter()
        .map(|(_, edge)| edge.crossing_id().unwrap().takeoff)
        .collect();
    a1_takeoffs.sort_by_key(|at| (at.x, at.z, at.level));
    assert_eq!(a1_takeoffs, [tile(3440, 9885), tile(3440, 9887)]);
    let a2: Vec<_> = world()
        .graph
        .edges
        .iter()
        .filter(|edge| matches!(edge.loc_id, 3506 | 3507))
        .collect();
    assert_eq!(a2.len(), 4, "both leaves carry both directed crossings");
    for (loc_id, anchor, takeoff, to) in [
        (3507, tile(3443, 3458), tile(3443, 3458), tile(3443, 3457)),
        (3506, tile(3444, 3458), tile(3444, 3458), tile(3444, 3457)),
        (3507, tile(3443, 3458), tile(3443, 3457), tile(3443, 3458)),
        (3506, tile(3444, 3458), tile(3444, 3457), tile(3444, 3458)),
    ] {
        assert!(
            a2.iter().any(|edge| {
                edge.loc_id == loc_id
                    && edge.option == 1
                    && edge.at == anchor
                    && edge.to == to
                    && edge.crossing_id().unwrap().takeoff == takeoff
            }),
            "missing A2 directed crossing {loc_id} anchor {anchor:?}: {takeoff:?} -> {to:?}"
        );
    }
    assert!(
        a2.iter().all(|edge| edge
            .quest_gates
            .as_ref()
            .is_some_and(|gates| { gates.gates().iter().any(is_a2_gate) })),
        "every real A2 crossing carries the [1,110] stage gate"
    );

    let from = a1.takeoff.unwrap_or(a1.at);
    let base = completed_state();
    let catalog = prerequisite_catalog();
    let establishable = catalog.establishable_windows();
    for edge in a1_directions
        .iter()
        .map(|(_, edge)| *edge)
        .chain(a2.iter().copied())
    {
        let crossing = edge.crossing_id().expect("representable directed crossing");
        for gate in edge
            .quest_gates
            .as_ref()
            .expect("typed crossing gates")
            .gates()
        {
            assert!(
                catalog.for_gate(gate, &crossing).is_some(),
                "baked gate and complete CrossingId must select the real row: {crossing:?}, {gate:?}"
            );
        }
    }
    let proof = morytania_state(&base, Truth::True, &establishable);
    let unknown = morytania_state(&base, Truth::Unknown, &establishable);
    let false_state = morytania_state(&base, Truth::False, &establishable);
    let world = world();
    let east = tile(3422, 3485);
    let underground = tile(3440, 9887);
    let east_stand = tile(3423, 3485); // holy_barrier.rs2's outward landing.
    assert!(!world.collision.standable(east));
    assert!(world.collision.standable(east_stand));
    assert!(
        world
            .graph
            .edges
            .iter()
            .any(|edge| edge.at == east && edge.to == underground),
        "the east trapdoor must enter the mausoleum"
    );
    for (start, target, loc_id) in [
        (east_stand, underground, 3432),
        (underground, east_stand, 3443),
    ] {
        let route = find_with(
            &world.collision,
            &world.graph,
            start,
            target,
            FindOptions::default(),
            &proof,
        )
        .unwrap_or_else(|error| {
            panic!(
                "Morytania-side mausoleum access {start:?} -> {target:?}: {error:?}; \
                 start standable={}, target standable={}",
                world.collision.standable(start),
                world.collision.standable(target),
            )
        });
        println!("mausoleum access {start:?} -> {target:?}: {route:?}");
        assert!(route
            .legs
            .iter()
            .any(|leg| matches!(leg, Leg::Transport { edge } if edge.loc_id == loc_id)));
    }
    let mut unknown_routes = Vec::with_capacity(SITES.len());
    let mut false_routes = Vec::with_capacity(SITES.len());

    for (name, region) in SITES {
        let targets = resource_targets(region);
        assert!(
            !targets.is_empty(),
            "{name} has no standable resource target"
        );

        let proof_search = find_first_with(
            &world.collision,
            &world.graph,
            from,
            &targets,
            FindOptions::default(),
            &proof,
        );
        let proof_route = proof_search
            .route()
            .unwrap_or_else(|error| panic!("proof cannot reach {name}: {error:?}"));
        assert!(
            proof_route.legs.iter().any(
                |leg| matches!(leg, Leg::Transport { edge } if matches!(edge.loc_id, 3443 | 3506 | 3507))
            ),
            "proof route to {name} must use an A1/A2 crossing"
        );
        println!("morytania route state=proof site={name}: {proof_route:?}");

        for (label, state, routes) in [
            ("unknown", &unknown, &mut unknown_routes),
            ("false", &false_state, &mut false_routes),
        ] {
            let search = find_first_with(
                &world.collision,
                &world.graph,
                from,
                &targets,
                FindOptions::default(),
                state,
            );
            let result = search.route();
            let rendered = format!("{result:?}");
            println!("morytania route state={label} site={name}: {rendered}");
            if matches!(result, Err(RouteError::NoPath)) {
                let actionable = find_actionable_quest_gates(
                    &world.collision,
                    &world.graph,
                    from,
                    &targets,
                    FindOptions::default(),
                    state,
                    &[],
                )
                .expect("strict NoPath has an actionable A1/A2 diagnosis");
                assert!(
                    actionable
                        .iter()
                        .any(|gate| { matches!(gate.crossing.loc_id, 3443 | 3506 | 3507) }),
                    "{label} diagnosis for {name} must identify an A1/A2 crossing"
                );
                println!("morytania diagnosis state={label} site={name}: {actionable:?}");
            }
            routes.push(rendered);
        }
    }

    assert_eq!(
        unknown_routes, false_routes,
        "unknown and false A1/A2 evidence must produce byte-identical strict route results"
    );
}
