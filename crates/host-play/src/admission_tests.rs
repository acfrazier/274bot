use super::*;
use api::quest_progress::EvidenceProvider;
use api::selected::ClientRevision;
use api::selected::{QuestGate, RunKey, Truth};
use api::snapshot::WorldTile;
use client::dash3d::CollisionFlag;
use nav::collision::{pack_walk, WorldCollision};
use nav::quest_gates::{QuestEvidence, QuestFamilyId};
use nav::router::{Leg, RouteError};
use nav::transport::{
    TransportEdge, TransportGraph, TransportKind, WildernessRules, WildernessZone,
};
use nav::zones::{Zone, ZoneClass, ZoneGroup, ZoneKind, ZoneTable, NO_GROUP};
use nav::WorldState;
use script::combat::guard::{Escape, EscapeAction, EscapeOwner, EscapeState};
use script::combat::request::{ActorKind, ActorRef};
use script::combat::risk::LiveRow;
use std::num::NonZeroU16;
use std::sync::{Arc, LazyLock};

fn tile(x: i32, z: i32) -> WorldTile {
    WorldTile { x, z, level: 0 }
}

fn selected_combat() -> Arc<CombatTables> {
    static TABLES: LazyLock<Arc<CombatTables>> = LazyLock::new(|| {
        CombatTables::build(api::game_data::for_revision(ClientRevision::R289).unwrap()).unwrap()
    });
    Arc::clone(&TABLES)
}

fn ice_kind(combat: &CombatTables) -> ZoneKind {
    let npc = combat
        .selected()
        .npc_by_config("icewarrior")
        .expect("revision 289 ice warrior facts");
    ZoneKind::new("ice-warrior", "Ice warrior", npc.id, 57, false, false)
}

fn missing_kind() -> ZoneKind {
    ZoneKind::new(
        "missing-kind",
        "Missing selected facts",
        i32::from(i16::MAX),
        1,
        false,
        false,
    )
}

fn open_corridor(width: usize, height: usize, open_z: usize) -> WorldCollision {
    let cells = width * height * 4;
    let mut flags = vec![CollisionFlag::WALK_SCENERY as u32; cells];
    for level in 0..4 {
        for x in 0..width {
            flags[level * width * height + open_z * width + x] = 0;
        }
    }
    let (walk, blocked) = pack_walk(&flags);
    WorldCollision {
        origin: tile(0, 0),
        width,
        height,
        walk,
        blocked,
        flags: None,
    }
}

fn fixture_world(
    combat: Arc<CombatTables>,
    collision: WorldCollision,
    zones: Vec<Zone>,
    kinds: Vec<ZoneKind>,
    groups: Vec<ZoneGroup>,
    mut graph: TransportGraph,
) -> NavWorld {
    graph.zones = Some(
        ZoneTable::from_parts(
            zones,
            kinds,
            groups,
            Vec::new(),
            Vec::new(),
            collision.origin,
            collision.width as u32,
            collision.height as u32,
            &graph.wilderness,
        )
        .expect("valid synthetic zone table"),
    );
    let world = NavWorld::from_parts(collision, graph, Vec::new());
    let risks = RiskTables::build(world.graph.zones.as_ref().unwrap(), &combat);
    world.risk_facts(|| Some(Tables { combat, risks }));
    world
}

fn zone(spawn_x: i32, kind: u16, group: Option<u16>) -> Zone {
    let mut zone = Zone::npc(tile(spawn_x, 1), 0, ZoneClass::Always, u16::MAX, kind);
    zone.group = group.unwrap_or(NO_GROUP);
    zone
}

fn known_corridor(spawns: &[i32]) -> NavWorld {
    let combat = selected_combat();
    let kind = ice_kind(&combat);
    let zones = spawns.iter().map(|&x| zone(x, 0, None)).collect();
    fixture_world(
        combat,
        open_corridor(24, 3, 1),
        zones,
        vec![kind],
        Vec::new(),
        TransportGraph::default(),
    )
}

fn unknown_corridor(spawn: i32) -> NavWorld {
    fixture_world(
        selected_combat(),
        open_corridor(16, 3, 1),
        vec![zone(spawn, 0, None)],
        vec![missing_kind()],
        Vec::new(),
        TransportGraph::default(),
    )
}

fn empty_corridor() -> NavWorld {
    fixture_world(
        selected_combat(),
        open_corridor(16, 3, 1),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        TransportGraph::default(),
    )
}

fn input(hp: u8) -> RiskInput {
    RiskInput {
        pos: tile(0, 1),
        poison: PoisonState::Clear,
        tick: 10,
        hp,
        hp_max: 99,
        prayer: 10,
        prayer_base: 10,
        combat: Some(50),
        missing_facts: false,
        unattributed: false,
        input_locked: false,
        overflow: false,
        ..RiskInput::default()
    }
}

fn make_admission(policy: RiskPolicy, input: RiskInput) -> Admission {
    Admission {
        policy,
        grants: ZoneExempt::NONE,
        allow: WalkAllow::default(),
        input,
        generation: 17,
        compat_v1: false,
        escape: None,
    }
}

fn real_search(
    world: &NavWorld,
    from: WorldTile,
    to: WorldTile,
    options: FindOptions,
    state: &WorldState,
    avoid: &[AvoidRect],
) -> crate::RouteOutcome {
    match nav::router::find_with_avoid(
        &world.collision,
        &world.graph,
        from,
        to,
        options,
        state,
        avoid,
    ) {
        Ok(route) => crate::RouteOutcome::Routed(route),
        Err(RouteError::NoPath | RouteError::BudgetExhausted) => crate::RouteOutcome::NoPath,
    }
}

fn real_witness(
    world: &NavWorld,
    from: WorldTile,
    to: WorldTile,
    options: FindOptions,
    state: &WorldState,
    avoid: &[AvoidRect],
) -> Option<Vec<ZoneKey>> {
    nav::router::find_blocking_zones(
        &world.collision,
        &world.graph,
        from,
        to,
        options,
        state,
        avoid,
    )
}

fn routed(result: &RouteAdmission) -> &Route {
    match &result.outcome {
        crate::RouteOutcome::Routed(route) | crate::RouteOutcome::BankSession { route, .. } => {
            route
        }
        crate::RouteOutcome::NoPath => panic!("expected a real router route"),
    }
}

#[test]
fn unavailable_snapshots_preserve_missing_facts_without_inventing_events() {
    for map_members in [false, true] {
        let input = unavailable(map_members);
        assert!(input.missing_facts);
        assert!(!input.overflow && !input.unattributed);
        assert_eq!(input.map_members, map_members);
        assert_eq!(input.poison, PoisonState::Unknown { since: 0 });
    }
}

#[cfg(feature = "test-support")]
fn count_kernels<T>(work: impl FnOnce() -> T) -> (T, usize) {
    nav::router::count_kernel_searches(work)
}

#[cfg(feature = "test-support")]
#[test]
fn u6_three_zone_witness_runs_one_real_named_retry_and_counts_kernels() {
    let world = known_corridor(&[3, 6, 9]);
    let from = tile(0, 1);
    let to = tile(23, 1);
    let state = WorldState::default();
    let avoid = [];
    let options = FindOptions::default();
    let mut admission = make_admission(
        RiskPolicy::Inherit,
        RiskInput {
            prayer: 99,
            prayer_base: 99,
            ..input(99)
        },
    );
    admission.allow.prayer = true;
    let ((result, searches), kernels) = count_kernels(|| {
        let mut seen = Vec::new();
        let result = route(
            &world,
            &admission,
            options,
            |opts| {
                seen.push(opts);
                real_search(&world, from, to, opts, &state, &avoid)
            },
            || real_witness(&world, from, to, options, &state, &avoid),
        );
        (result, seen)
    });

    assert_eq!(searches.len(), 2, "strict search and one named retry");
    assert_eq!(searches[0].zones, ZoneExempt::NONE);
    assert_eq!(
        searches[1].zones,
        ZoneExempt::named(&[ZoneKey::Zone(0), ZoneKey::Zone(1), ZoneKey::Zone(2),]).unwrap()
    );
    assert_eq!(
        kernels, 5,
        "strict, repeated strict diagnosis, all-zone witness, then strict and named completion"
    );
    assert!(matches!(result.outcome, crate::RouteOutcome::Routed(_)));
    assert_eq!(result.tried, 1);
    assert_eq!(
        result.blocked.as_ref(),
        &[ZoneKey::Zone(0), ZoneKey::Zone(1), ZoneKey::Zone(2)]
    );
    assert_eq!(
        result.assessment.as_ref().unwrap().verdict,
        Verdict::Survivable
    );
    assert_eq!(result.refusal, None);
    assert_eq!(routed(&result).dest, to);
}

#[cfg(feature = "test-support")]
#[test]
fn u6_nine_zone_witness_is_bounded_and_never_researched() {
    let spawns = [1, 3, 5, 7, 9, 11, 13, 15, 17];
    let combat = selected_combat();
    let kind = ice_kind(&combat);
    let zones = spawns.iter().map(|&x| zone(x, 0, None)).collect();
    let world = fixture_world(
        combat,
        open_corridor(24, 3, 1),
        zones,
        vec![kind],
        Vec::new(),
        TransportGraph::default(),
    );
    let from = tile(0, 1);
    let to = tile(20, 1);
    let state = WorldState::default();
    let avoid = [];
    let options = FindOptions::default();
    let admission = make_admission(RiskPolicy::Inherit, input(99));
    let ((result, searches), kernels) = count_kernels(|| {
        let mut seen = 0;
        let result = route(
            &world,
            &admission,
            options,
            |opts| {
                seen += 1;
                real_search(&world, from, to, opts, &state, &avoid)
            },
            || real_witness(&world, from, to, options, &state, &avoid),
        );
        (result, seen)
    });

    assert_eq!(searches, 1);
    assert_eq!(
        kernels, 3,
        "strict, repeated strict diagnosis and the all-zone witness; no named retry"
    );
    assert_eq!(result.tried, 0);
    assert_eq!(result.blocked.len(), 9);
    assert_eq!(
        result.refusal,
        Some(WalkRefusal::NoRouteWithinBounds {
            tried: 0,
            last: None
        })
    );
}

#[cfg(feature = "test-support")]
#[test]
fn u6_group_witness_checks_all_members_and_retries_by_canonical_group() {
    let combat = selected_combat();
    let kind = ice_kind(&combat);
    let zones = vec![zone(4, 0, Some(0)), zone(7, 0, Some(0))];
    let group = ZoneGroup::new(
        "test-group",
        "Test group",
        AvoidRect {
            min_x: 4,
            max_x: 7,
            min_z: 1,
            max_z: 1,
            level: Some(0),
        },
        vec![0, 1],
    );
    let world = fixture_world(
        combat,
        open_corridor(24, 3, 1),
        zones,
        vec![kind],
        vec![group],
        TransportGraph::default(),
    );
    let from = tile(0, 1);
    let to = tile(23, 1);
    let state = WorldState::default();
    let avoid = [];
    let options = FindOptions::default();
    let mut admission = make_admission(
        RiskPolicy::Inherit,
        RiskInput {
            prayer: 99,
            prayer_base: 99,
            ..input(99)
        },
    );
    admission.allow.prayer = true;
    let ((result, searches), kernels) = count_kernels(|| {
        let mut seen = Vec::new();
        let result = route(
            &world,
            &admission,
            options,
            |opts| {
                seen.push(opts);
                real_search(&world, from, to, opts, &state, &avoid)
            },
            || real_witness(&world, from, to, options, &state, &avoid),
        );
        (result, seen)
    });

    assert_eq!(searches.len(), 2);
    assert_eq!(kernels, 5);
    assert_eq!(result.blocked.as_ref(), &[ZoneKey::Group(0)]);
    assert_eq!(
        searches[1].zones,
        ZoneExempt::named(&[ZoneKey::Group(0)]).unwrap()
    );
    assert_eq!(result.tried, 1);
    assert_eq!(result.refusal, None);
    assert_eq!(
        result.assessment.as_ref().unwrap().verdict,
        Verdict::Survivable
    );
}

#[cfg(feature = "test-support")]
#[test]
fn u6_unknown_member_in_group_blocks_the_first_witness() {
    let combat = selected_combat();
    let kinds = vec![ice_kind(&combat), missing_kind()];
    let zones = vec![zone(4, 0, Some(0)), zone(7, 1, Some(0))];
    let group = ZoneGroup::new(
        "partly-unknown",
        "Partly unknown",
        AvoidRect {
            min_x: 4,
            max_x: 7,
            min_z: 1,
            max_z: 1,
            level: Some(0),
        },
        vec![0, 1],
    );
    let world = fixture_world(
        combat,
        open_corridor(16, 3, 1),
        zones,
        kinds,
        vec![group],
        TransportGraph::default(),
    );
    let input = input(99);
    let from = tile(0, 1);
    let to = tile(10, 1);
    let state = WorldState::default();
    let options = FindOptions::default();
    let witness = real_witness(&world, from, to, options, &state, &[]).unwrap();
    assert_eq!(witness, [ZoneKey::Group(0)]);
    assert!(!witness_admissible(&world, &input, &witness));

    let admission = make_admission(RiskPolicy::Inherit, input);
    let ((result, searches), kernels) = count_kernels(|| {
        let mut seen = 0;
        let result = route(
            &world,
            &admission,
            options,
            |opts| {
                seen += 1;
                real_search(&world, from, to, opts, &state, &[])
            },
            || real_witness(&world, from, to, options, &state, &[]),
        );
        (result, seen)
    });
    assert_eq!(searches, 1);
    assert_eq!(kernels, 3);
    assert_eq!(result.tried, 0);
    assert_eq!(result.blocked.as_ref(), &[ZoneKey::Group(0)]);
    assert_eq!(
        result.refusal,
        Some(WalkRefusal::NoRouteWithinBounds {
            tried: 0,
            last: None
        })
    );
}

#[cfg(feature = "test-support")]
#[test]
fn u6_unknown_and_lethal_single_witnesses_are_refused_before_retry() {
    let unknown_world = unknown_corridor(5);
    let lethal_world = known_corridor(&[5]);
    let lethal_hp = tables(&lethal_world)
        .unwrap()
        .risks
        .kind(0)
        .unwrap()
        .max_hit;
    for (world, hp, expected_unknown) in [
        (unknown_world, 99, Some(UnknownWhy::MissingFacts)),
        (lethal_world, lethal_hp, None),
    ] {
        let input = input(hp);
        let from = tile(0, 1);
        let to = tile(10, 1);
        let state = WorldState::default();
        let options = FindOptions::default();
        let keys = real_witness(&world, from, to, options, &state, &[]).unwrap();
        assert_eq!(keys, [ZoneKey::Zone(0)]);
        assert!(!witness_admissible(&world, &input, &keys));
        if let Some(why) = expected_unknown {
            let kind = tables(&world).unwrap().risks.kind(0).unwrap();
            assert!(kind.unknown_for(input.map_members).is_some());
            assert_eq!(why, UnknownWhy::MissingFacts);
        } else {
            let max_hit = tables(&world).unwrap().risks.kind(0).unwrap().max_hit;
            assert_eq!(
                hp, max_hit,
                "the first witness is lethal at the HP boundary"
            );
        }

        let admission = make_admission(RiskPolicy::Inherit, input);
        let ((result, searches), kernels) = count_kernels(|| {
            let mut seen = 0;
            let result = route(
                &world,
                &admission,
                options,
                |opts| {
                    seen += 1;
                    real_search(&world, from, to, opts, &state, &[])
                },
                || real_witness(&world, from, to, options, &state, &[]),
            );
            (result, seen)
        });
        assert_eq!(searches, 1);
        assert_eq!(kernels, 3);
        assert_eq!(result.tried, 0);
        assert_eq!(
            result.refusal,
            Some(WalkRefusal::NoRouteWithinBounds {
                tried: 0,
                last: None
            })
        );
        assert!(result.assessment.is_none());
    }
}
#[cfg(feature = "test-support")]
#[test]
fn u6_real_budget_one_corridor_exhausts_the_named_retry_after_a_real_witness() {
    const BUDGET: usize = 1;
    const WIDTH: usize = 160;
    const WITNESS_X: i32 = 80;
    const DESTINATION_X: i32 = 150;
    let combat = selected_combat();
    let budget_exhaustion_corridor = fixture_world(
        combat.clone(),
        open_corridor(WIDTH, 3, 1),
        vec![zone(WITNESS_X, 0, None)],
        vec![ice_kind(&combat)],
        Vec::new(),
        TransportGraph::default(),
    );
    let from = tile(0, 1);
    let to = tile(DESTINATION_X, 1);
    let state = WorldState::default();
    let avoid = [];
    let options = FindOptions::default();
    let admission = make_admission(RiskPolicy::Inherit, input(99));
    let ((result, errors), kernels) = count_kernels(|| {
        let mut observed_errors = Vec::new();
        let result = route(
            &budget_exhaustion_corridor,
            &admission,
            options,
            |opts| match nav::router::find_with_avoid_bounded(
                &budget_exhaustion_corridor.collision,
                &budget_exhaustion_corridor.graph,
                from,
                to,
                opts,
                &state,
                &avoid,
                BUDGET,
            ) {
                Ok(found) => crate::RouteOutcome::Routed(found),
                Err(error) => {
                    observed_errors.push(error);
                    crate::RouteOutcome::NoPath
                }
            },
            || {
                real_witness(
                    &budget_exhaustion_corridor,
                    from,
                    to,
                    options,
                    &state,
                    &avoid,
                )
            },
        );
        (result, observed_errors)
    });

    assert_eq!(
        errors,
        [RouteError::BudgetExhausted, RouteError::BudgetExhausted]
    );
    assert_eq!(
        kernels, 5,
        "both bounded seam searches and the full witness diagnosis use real kernels"
    );
    assert_eq!(result.tried, 1);
    assert_eq!(result.blocked.as_ref(), &[ZoneKey::Zone(0)]);
    assert_eq!(
        result.refusal,
        Some(WalkRefusal::NoRouteWithinBounds {
            tried: 1,
            last: None
        })
    );
    assert!(result.assessment.is_none());
}

#[cfg(feature = "test-support")]
#[test]
fn u6_endpoint_completion_is_real_router_work_and_endpoint_risk_is_assessed() {
    let world = known_corridor(&[10]);
    let from = tile(0, 1);
    let to = tile(10, 1);
    let state = WorldState::default();
    let options = FindOptions::default();
    let admission = make_admission(RiskPolicy::Inherit, input(99));
    let ((result, search_calls), kernels) = count_kernels(|| {
        let mut calls = 0;
        let result = route(
            &world,
            &admission,
            options,
            |opts| {
                calls += 1;
                real_search(&world, from, to, opts, &state, &[])
            },
            || panic!("endpoint completion must not run witness diagnosis"),
        );
        (result, calls)
    });

    assert_eq!(search_calls, 1);
    assert_eq!(
        kernels, 1,
        "the reverse proof skips strict work; endpoint completion runs one kernel"
    );
    assert!(matches!(result.outcome, crate::RouteOutcome::Routed(_)));
    let assessment = result
        .assessment
        .as_ref()
        .expect("endpoint route is assessed");
    assert!(!assessment.plan.intervals.is_empty());
    assert!(
        !assessment.plan.crossings.is_empty(),
        "the endpoint crossing is retained"
    );
}

#[cfg(feature = "test-support")]
#[test]
fn u6_named_retry_keeps_teleport_wilderness_avoid_and_quest_authority() {
    let combat = selected_combat();
    let mut graph = TransportGraph {
        wilderness: WildernessRules {
            zones: vec![WildernessZone {
                x1: 6,
                z1: 0,
                x2: 8,
                z2: 0,
                level1: 0,
                level2: 0,
                origin_z: 0,
            }],
            divisor: 1,
            offset: 1,
        },
        ..Default::default()
    };
    let from = tile(0, 1);
    let to = tile(12, 1);
    let mut quest_edge = transport_edge(TransportKind::Ladder, from, to);
    quest_edge.quest_req.push("unproven quest".to_owned());
    graph.edges.push(quest_edge);
    graph.at.insert(from, vec![0]);
    graph
        .teleports
        .push(transport_edge(TransportKind::Teleport, from, to));
    let world = fixture_world(
        combat.clone(),
        open_corridor(16, 3, 1),
        vec![zone(5, 0, None)],
        vec![ice_kind(&combat)],
        Vec::new(),
        graph,
    );
    let state = WorldState::default();
    let original_state = state.clone();
    let avoid = [AvoidRect {
        min_x: 2,
        max_x: 2,
        min_z: 0,
        max_z: 0,
        level: Some(0),
    }];
    let original_avoid = avoid;
    let options = FindOptions {
        allow_teleports: false,
        allow_wilderness: false,
        allow_bank_fetch: false,
        essence: None,
        zones: ZoneExempt::NONE,
    };
    let admission = make_admission(RiskPolicy::Inherit, input(99));
    let ((result, searches), kernels) = count_kernels(|| {
        let mut seen = Vec::new();
        let result = route(
            &world,
            &admission,
            options,
            |opts| {
                assert_eq!(
                    state, original_state,
                    "quest/readiness evidence is request-frozen"
                );
                assert_eq!(avoid, original_avoid, "caller avoids are request-frozen");
                assert_eq!(opts.allow_teleports, options.allow_teleports);
                assert_eq!(opts.allow_wilderness, options.allow_wilderness);
                assert_eq!(opts.allow_bank_fetch, options.allow_bank_fetch);
                assert_eq!(opts.essence, options.essence);
                seen.push(opts);
                real_search(&world, from, to, opts, &state, &avoid)
            },
            || real_witness(&world, from, to, options, &state, &avoid),
        );
        (result, seen)
    });

    assert_eq!(searches.len(), 2);
    assert_eq!(kernels, 5);
    assert_eq!(searches[0].zones, ZoneExempt::NONE);
    assert_eq!(
        searches[1].zones,
        ZoneExempt::named(&[ZoneKey::Zone(0)]).unwrap()
    );
    let route = routed(&result);
    assert!(route.legs.iter().all(|leg| matches!(leg, Leg::Walk { .. })));
    for leg in &route.legs {
        if let Leg::Walk { tiles } = leg {
            assert!(tiles.iter().all(|&tile| !avoid[0].contains(tile)));
            assert!(tiles
                .iter()
                .all(|&tile| !world.graph.wilderness.contains(tile)));
        }
    }
    assert_eq!(state, original_state);
}

#[cfg(feature = "test-support")]
#[test]
fn c1_unknown_poison_refuses_crossing_but_not_safe_only_and_silence_does_not_clear() {
    let world = known_corridor(&[5]);
    let from = tile(0, 1);
    let to = tile(10, 1);
    let state = WorldState::default();
    let options = FindOptions::default();
    let mut uncertain = input(99);
    uncertain.tick = 31;
    uncertain.poison = PoisonState::Unknown { since: 0 };
    let admission = make_admission(RiskPolicy::Inherit, uncertain);
    let (crossing, kernels) = count_kernels(|| {
        route(
            &world,
            &admission,
            options,
            |opts| real_search(&world, from, to, opts, &state, &[]),
            || real_witness(&world, from, to, options, &state, &[]),
        )
    });
    assert_eq!(kernels, 5);
    assert_eq!(crossing.tried, 1);
    assert_eq!(
        crossing.refusal,
        Some(WalkRefusal::Unknown(UnknownWhy::Poison))
    );
    let unknown_assessment = crossing.assessment.as_ref().unwrap();
    assert_eq!(
        unknown_assessment.input.poison,
        PoisonState::Unknown { since: 0 }
    );
    assert!(unknown_assessment
        .reason
        .contains("waiting alone does not clear it"));
    assert!(unknown_assessment
        .reason
        .contains("separate safe-only walk"));

    let override_admission = make_admission(RiskPolicy::Proceed, uncertain);
    let (overridden, override_kernels) = count_kernels(|| {
        route(
            &world,
            &override_admission,
            options,
            |opts| real_search(&world, from, to, opts, &state, &[]),
            || panic!("Proceed uses its single all-zone search"),
        )
    });
    assert_eq!(override_kernels, 2);
    assert!(overridden.refusal.is_none());
    let override_assessment = overridden.assessment.as_ref().unwrap();
    assert_eq!(
        override_assessment.verdict,
        Verdict::Unknown(UnknownWhy::Poison)
    );
    assert!(override_assessment
        .reason
        .contains("when a crossing is refused"));

    let safe_world = empty_corridor();
    let safe_to = tile(10, 1);
    let safe_admission = make_admission(RiskPolicy::Inherit, uncertain);
    let (safe, safe_kernels) = count_kernels(|| {
        route(
            &safe_world,
            &safe_admission,
            options,
            |opts| real_search(&safe_world, from, safe_to, opts, &state, &[]),
            || real_witness(&safe_world, from, safe_to, options, &state, &[]),
        )
    });
    assert_eq!(safe_kernels, 1);
    assert!(safe.refusal.is_none());
    assert_eq!(
        safe.assessment.as_ref().unwrap().verdict,
        Verdict::Survivable
    );
    assert_eq!(
        safe.assessment.as_ref().unwrap().input.poison,
        uncertain.poison
    );

    let mut poisoned = input(99);
    poisoned.poison = PoisonState::Poisoned {
        per_tick: 6,
        last_tick: 31,
    };
    let poisoned_admission = make_admission(RiskPolicy::Proceed, poisoned);
    let (already_poisoned, poisoned_kernels) = count_kernels(|| {
        route(
            &world,
            &poisoned_admission,
            options,
            |opts| real_search(&world, from, to, opts, &state, &[]),
            || panic!("Proceed uses its single all-zone search"),
        )
    });
    assert_eq!(poisoned_kernels, 2);
    let estimate = already_poisoned.assessment.as_ref().unwrap();
    assert_eq!(estimate.input.poison, poisoned.poison);
    assert!(estimate.reason.contains("poisoned (6 per 30 ticks)"));
}

#[cfg(feature = "test-support")]
#[test]
fn u7_proceed_avoid_inherit_and_compat_keep_the_seam_contract() {
    let world = unknown_corridor(5);
    let from = tile(0, 1);
    let to = tile(10, 1);
    let state = WorldState::default();
    let options = FindOptions::default();

    let proceed = make_admission(RiskPolicy::Proceed, input(99));
    let ((proceeded, proceed_calls), proceed_kernels) = count_kernels(|| {
        let mut calls = 0;
        let result = route(
            &world,
            &proceed,
            options,
            |opts| {
                calls += 1;
                assert!(opts.zones.is_all());
                real_search(&world, from, to, opts, &state, &[])
            },
            || panic!("Proceed has one all-zone search and no diagnosis"),
        );
        (result, calls)
    });
    assert_eq!(proceed_calls, 1);
    assert_eq!(proceed_kernels, 2);
    assert!(proceeded.refusal.is_none());
    assert_eq!(
        proceeded.assessment.as_ref().unwrap().verdict,
        Verdict::Unknown(UnknownWhy::MissingFacts)
    );

    let mut compat = make_admission(RiskPolicy::Avoid, input(99));
    compat.compat_v1 = true;
    let (compat_result, compat_kernels) = count_kernels(|| {
        route(
            &world,
            &compat,
            options,
            |opts| {
                assert!(opts.zones.is_all());
                real_search(&world, from, to, opts, &state, &[])
            },
            || panic!("compat v1 does not diagnose or retry"),
        )
    });
    assert_eq!(compat_kernels, 2);
    assert!(compat_result.refusal.is_none());
    assert_eq!(
        compat_result.assessment.as_ref().unwrap().verdict,
        Verdict::Unknown(UnknownWhy::MissingFacts)
    );

    let known = known_corridor(&[5]);
    let grant = ZoneExempt::named(&[ZoneKey::Zone(0)]).unwrap();
    let mut avoid_with_grant = make_admission(RiskPolicy::Avoid, input(1));
    avoid_with_grant.grants = grant;
    let granted_options = FindOptions {
        zones: grant,
        ..FindOptions::default()
    };
    let (avoided, avoid_kernels) = count_kernels(|| {
        route(
            &known,
            &avoid_with_grant,
            granted_options,
            |opts| real_search(&known, from, to, opts, &state, &[]),
            || panic!("an explicit Avoid grant does not start the bounded retry"),
        )
    });
    assert_eq!(avoid_kernels, 2);
    assert!(
        avoided.refusal.is_none(),
        "the request-scoped named grant admits the crossing"
    );
    assert!(avoided.assessment.is_some());
}

#[cfg(feature = "test-support")]
#[test]
fn u7_endpoint_unknown_and_unsafe_crossings_keep_typed_refusals() {
    let from = tile(0, 1);
    let to = tile(10, 1);
    let state = WorldState::default();
    let options = FindOptions::default();

    let unknown_world = unknown_corridor(10);
    let unknown_request = make_admission(RiskPolicy::Avoid, input(99));
    let (unknown, unknown_kernels) = count_kernels(|| {
        route(
            &unknown_world,
            &unknown_request,
            options,
            |opts| real_search(&unknown_world, from, to, opts, &state, &[]),
            || panic!("Avoid does not perform the bounded witness retry"),
        )
    });
    assert_eq!(unknown_kernels, 1);
    assert_eq!(
        unknown.assessment.as_ref().unwrap().verdict,
        Verdict::Unknown(UnknownWhy::MissingFacts)
    );
    assert_eq!(
        unknown.refusal,
        Some(WalkRefusal::Unknown(UnknownWhy::MissingFacts))
    );

    let unsafe_world = known_corridor(&[10]);
    let unsafe_request = make_admission(RiskPolicy::Avoid, input(1));
    let (unsafe_endpoint, unsafe_kernels) = count_kernels(|| {
        route(
            &unsafe_world,
            &unsafe_request,
            options,
            |opts| real_search(&unsafe_world, from, to, opts, &state, &[]),
            || panic!("Avoid does not perform the bounded witness retry"),
        )
    });
    assert_eq!(unsafe_kernels, 1);
    let assessment = unsafe_endpoint.assessment.as_ref().unwrap();
    assert!(!assessment.plan.crossings.is_empty());
    assert_ne!(assessment.verdict, Verdict::Survivable);
    assert_eq!(
        unsafe_endpoint.refusal,
        Some(verdict_refusal(assessment.verdict))
    );
    assert!(matches!(
        unsafe_endpoint.refusal,
        Some(WalkRefusal::Unsurvivable | WalkRefusal::FixableWith)
    ));
}

#[test]
fn u7_refusal_mapping_preserves_typed_risk_refusals() {
    assert_eq!(
        verdict_refusal(Verdict::FixableWith),
        WalkRefusal::FixableWith
    );
    assert_eq!(
        verdict_refusal(Verdict::Unsurvivable),
        WalkRefusal::Unsurvivable
    );
    assert_eq!(
        verdict_refusal(Verdict::Unknown(UnknownWhy::Kind(4))),
        WalkRefusal::Unknown(UnknownWhy::Kind(4))
    );
}

#[test]
fn u11_fresh_ignores_only_clock_age() {
    let world = empty_corridor();
    let mut input = input(80);
    input.live_len = 1;
    input.live[0] = LiveRow {
        actor: ActorRef {
            kind: ActorKind::Npc,
            index: 2,
        },
        ident: 42,
        max_hit: 3,
        rate: 4,
        due_tick: 15,
    };
    let admission = make_admission(RiskPolicy::Inherit, input);
    let route = nav::router::find_with_avoid(
        &world.collision,
        &world.graph,
        tile(0, 1),
        tile(5, 1),
        FindOptions::default(),
        &WorldState::default(),
        &[],
    )
    .unwrap();
    let assessment = assess(&route, &world, &admission);

    let mut clock_only = input;
    clock_only.tick = clock_only.tick.wrapping_add(1);
    assert!(fresh(&assessment, clock_only));

    let mut changed = Vec::new();
    let mut hp = input;
    hp.hp -= 1;
    changed.push(hp);
    let mut food = input;
    food.food_len = 1;
    food.food_ids[0] = 379;
    food.food_counts[0] = 1;
    changed.push(food);
    let mut prayer = input;
    prayer.prayer += 1;
    changed.push(prayer);
    let mut position = input;
    position.pos = tile(1, 1);
    changed.push(position);
    let actor = input;
    let mut changed_actor = actor;
    changed_actor.live[0].actor.index += 1;
    changed.push(changed_actor);
    let mut changed_due = actor;
    changed_due.live[0].due_tick += 1;
    changed.push(changed_due);
    let mut poison = input;
    poison.poison = PoisonState::Poisoned {
        per_tick: 2,
        last_tick: 12,
    };
    changed.push(poison);
    let mut off_debt = input;
    off_debt.off_debt = true;
    changed.push(off_debt);

    for value in changed {
        let mut value = value;
        value.tick = value.tick.wrapping_add(1);
        assert!(
            !fresh(&assessment, value),
            "non-clock freshness input changed: {value:?}"
        );
    }
}

#[test]
fn u11_native_publication_reassesses_stale_input_then_refuses_before_follow() {
    let world = empty_corridor();
    let from = tile(0, 1);
    let to = tile(8, 1);
    let route = nav::router::find_with_avoid(
        &world.collision,
        &world.graph,
        from,
        to,
        FindOptions::default(),
        &WorldState::default(),
        &[],
    )
    .unwrap();
    let mut admission = make_admission(RiskPolicy::Inherit, input(80));
    admission.input.pos = from;
    let worker_assessment = assess(&route, &world, &admission);
    assert_eq!(worker_assessment.verdict, Verdict::Survivable);
    let mut bot = crate::script_runtime::NavBot {
        route: Some(route),
        admission: Some(Box::new(admission)),
        assessment: Some(Arc::clone(&worker_assessment)),
        admission_pending: true,
        walk_request_id: 27,
        route_generation: 17,
        ..Default::default()
    };

    publish(
        &mut bot,
        &world,
        &api::snapshot::GameSnapshot::new(),
        false,
        None,
    );

    assert!(!bot.admission_pending);
    assert!(bot.route.is_none(), "stale worker route is not followed");
    assert!(bot
        .assessment
        .as_ref()
        .is_some_and(|fresh_assessment| !Arc::ptr_eq(fresh_assessment, &worker_assessment)));
    assert!(bot.last_assessment.as_ref().unwrap().input.missing_facts);
    assert_eq!(
        bot.risk_refusal.as_deref().map(|entry| entry.1),
        Some(WalkRefusal::Unknown(UnknownWhy::MissingFacts))
    );
}

#[test]
fn u11_manual_stale_publication_refuses_and_retains_reason_before_held_follow() {
    let world = empty_corridor();
    let from = tile(0, 1);
    let to = tile(8, 1);
    let route = nav::router::find_with_avoid(
        &world.collision,
        &world.graph,
        from,
        to,
        FindOptions::default(),
        &WorldState::default(),
        &[],
    )
    .unwrap();
    let mut admission = make_admission(RiskPolicy::Inherit, input(80));
    admission.input.pos = from;
    let worker_assessment = assess(&route, &world, &admission);
    let mut arm = crate::WalkArm {
        route: Some(route),
        route_generation: 17,
        admission: Some(Box::new(admission)),
        assessment: Some(worker_assessment),
        admission_pending: true,
        ..Default::default()
    };
    assert!(
        !crate::WalkArm::may_follow(true),
        "the simulated outer hold remains active"
    );

    let refused = crate::observe_walk_arm_admission(
        &api::snapshot::GameSnapshot::new(),
        &mut arm,
        &world,
        false,
        None,
    );

    assert!(refused);
    assert!(arm.route.is_none());
    assert!(!arm.admission_pending);
    assert_eq!(
        arm.refusal,
        Some(WalkRefusal::Unknown(UnknownWhy::MissingFacts))
    );
    let reason = &arm.last_assessment.as_ref().unwrap().reason;
    assert!(reason.contains("Unknown(MissingFacts)"));
    assert!(
        arm.admission.is_none(),
        "the refusal closes this stale manual arm"
    );
}

#[test]
fn u11_publication_during_live_escape_refuses_without_reassessment() {
    let world = empty_corridor();
    let from = tile(0, 1);
    let to = tile(8, 1);
    let route = nav::router::find_with_avoid(
        &world.collision,
        &world.graph,
        from,
        to,
        FindOptions::default(),
        &WorldState::default(),
        &[],
    )
    .unwrap();
    let mut admission = make_admission(RiskPolicy::Inherit, input(80));
    admission.escape = Some(escape(EscapeState::Unresolved));
    let worker_assessment = assess(&route, &world, &admission);
    let mut bot = crate::script_runtime::NavBot {
        route: Some(route),
        admission: Some(Box::new(admission)),
        assessment: Some(Arc::clone(&worker_assessment)),
        admission_pending: true,
        walk_request_id: 28,
        route_generation: 17,
        ..Default::default()
    };

    publish(
        &mut bot,
        &world,
        &api::snapshot::GameSnapshot::new(),
        false,
        None,
    );

    assert!(bot.route.is_none());
    assert_eq!(
        bot.risk_refusal.as_deref().map(|entry| entry.1),
        Some(WalkRefusal::EscapeInProgress)
    );
    assert!(Arc::ptr_eq(
        bot.last_assessment.as_ref().unwrap(),
        &worker_assessment,
    ));
    assert!(bot
        .walk_outcome_detail
        .as_ref()
        .unwrap()
        .contains("escape in progress"));
}

#[cfg(feature = "test-support")]
#[test]
fn u11_live_escape_states_refuse_before_any_kernel_but_advisory_assess_is_allowed() {
    let world = empty_corridor();
    let from = tile(0, 1);
    let to = tile(8, 1);
    let route = nav::router::find_with_avoid(
        &world.collision,
        &world.graph,
        from,
        to,
        FindOptions::default(),
        &WorldState::default(),
        &[],
    )
    .unwrap();

    for state in [
        EscapeState::Running,
        EscapeState::Suspended,
        EscapeState::Unresolved,
    ] {
        let mut request = make_admission(RiskPolicy::Inherit, input(80));
        request.escape = Some(escape(state));
        let (result, kernels) = count_kernels(|| {
            super::route(
                &world,
                &request,
                FindOptions::default(),
                |_| panic!("live escape must refuse before calling the router"),
                || panic!("live escape must refuse before witness diagnosis"),
            )
        });
        assert_eq!(kernels, 0, "{state:?} escape performs no route kernel");
        assert_eq!(result.refusal, Some(WalkRefusal::EscapeInProgress));
        assert!(result.assessment.is_none());

        let advisory = assess(&route, &world, &request);
        assert_eq!(advisory.generation, request.generation);
        assert_eq!(
            advisory.input, request.input,
            "pure assessment remains available during escape"
        );
    }
}

#[test]
fn c5_route_basis_keeps_hard_authority_through_replacement_and_revocation() {
    let provider: Arc<dyn EvidenceProvider> = Arc::new(UnknownEvidence);
    let family = QuestFamilyId::new([0x27; 32], NonZeroU16::new(1).unwrap());
    let stamp = EvidenceStamp {
        run: RunKey {
            slot: 3,
            run: 1,
            session: 2,
        },
        tick: 40,
        sequence: 41,
    };
    let evidence = QuestEvidence::new(provider, family, stamp);
    let avoid = Arc::<[AvoidRect]>::from([AvoidRect {
        min_x: 3,
        max_x: 5,
        min_z: 1,
        max_z: 2,
        level: Some(0),
    }]);
    let frozen_options = FindOptions {
        allow_teleports: false,
        allow_wilderness: false,
        allow_bank_fetch: false,
        essence: None,
        zones: ZoneExempt::NONE,
    };
    let original_route = Route {
        legs: vec![Leg::Walk {
            tiles: vec![tile(0, 1), tile(1, 1)],
        }],
        dest: tile(1, 1),
        ticks: 0.5,
    };
    let original = Arc::new(RouteBasis {
        route: original_route.clone(),
        options: frozen_options,
        avoid: Arc::clone(&avoid),
        quest_evidence: Some(evidence),
    });
    let retained = Arc::clone(&original);
    let mut arm = crate::WalkArm {
        route: Some(original_route),
        basis: Some(original),
        ..Default::default()
    };

    let refreshed_route = Route {
        legs: vec![Leg::Walk {
            tiles: vec![tile(0, 1), tile(2, 1)],
        }],
        dest: tile(2, 1),
        ticks: 1.0,
    };
    let refreshed = Arc::new(RouteBasis {
        route: refreshed_route.clone(),
        options: retained.options,
        avoid: Arc::clone(&retained.avoid),
        quest_evidence: retained.quest_evidence.clone(),
    });
    arm.route = Some(refreshed_route);
    arm.basis = Some(Arc::clone(&refreshed));

    assert_eq!(arm.basis.as_ref().unwrap().options, frozen_options);
    assert_eq!(arm.basis.as_ref().unwrap().avoid.as_ref(), avoid.as_ref());
    assert_eq!(
        arm.basis
            .as_ref()
            .unwrap()
            .quest_evidence
            .as_ref()
            .unwrap()
            .family(),
        &family
    );
    assert_eq!(retained.route.dest, tile(1, 1));
    assert_eq!(retained.options, frozen_options);
    assert_eq!(retained.avoid.as_ref(), avoid.as_ref());
    assert_eq!(
        retained.quest_evidence.as_ref().unwrap().required_after(),
        stamp
    );

    arm.route = None;
    arm.basis = None;
    drop(refreshed);
    assert!(!retained.options.allow_teleports);
    assert!(!retained.options.allow_wilderness);
    assert!(!retained.options.allow_bank_fetch);
    assert_eq!(retained.avoid.as_ref(), avoid.as_ref());
    assert_eq!(retained.quest_evidence.as_ref().unwrap().family(), &family);
}

#[cfg(feature = "test-support")]
#[test]
#[ignore = "offline S2b admission microbenchmark; parent collects evidence"]
fn s2b_admission_route_and_publication_microbenchmarks() {
    use std::time::Instant;

    const WARMUPS: usize = 5;
    const SAMPLES: usize = 25;

    fn summarize(name: &str, mut samples: Vec<u128>, kernels: usize) {
        samples.sort_unstable();
        let p50 = samples[samples.len() / 2];
        let p95_index = samples
            .len()
            .saturating_mul(95)
            .div_ceil(100)
            .saturating_sub(1);
        let p95 = samples[p95_index.min(samples.len() - 1)];
        println!(
            "{{\"bench\":\"s2b-admission\",\"cell\":\"{name}\",\"samples\":{},\"p50_ns\":{p50},\"p95_ns\":{p95},\"kernel_searches\":{kernels}}}",
            samples.len()
        );
    }

    fn sample_route(name: &str, expected_kernels: usize, mut work: impl FnMut() -> RouteAdmission) {
        let mut samples = Vec::with_capacity(SAMPLES);
        for iteration in 0..(WARMUPS + SAMPLES) {
            let started = Instant::now();
            let (result, kernels) = count_kernels(&mut work);
            let elapsed = started.elapsed().as_nanos();
            assert_eq!(
                kernels, expected_kernels,
                "{name} actual router kernel count"
            );
            if name == "safe_route" {
                assert!(result.refusal.is_none());
            } else if name == "refused_route" {
                assert!(result.refusal.is_some());
            } else {
                assert_eq!(result.tried, 1);
            }
            if iteration >= WARMUPS {
                samples.push(elapsed);
            }
        }
        summarize(name, samples, expected_kernels);
    }

    let state = WorldState::default();
    let options = FindOptions::default();
    let safe_world = empty_corridor();
    let safe_admission = make_admission(RiskPolicy::Inherit, input(80));
    let safe_from = tile(0, 1);
    let safe_to = tile(12, 1);
    let _ = assess(
        &nav::router::find_with_avoid(
            &safe_world.collision,
            &safe_world.graph,
            safe_from,
            safe_to,
            options,
            &state,
            &[],
        )
        .unwrap(),
        &safe_world,
        &safe_admission,
    );
    sample_route("safe_route", 1, || {
        route(
            &safe_world,
            &safe_admission,
            options,
            |opts| real_search(&safe_world, safe_from, safe_to, opts, &state, &[]),
            || real_witness(&safe_world, safe_from, safe_to, options, &state, &[]),
        )
    });

    let refused_world = unknown_corridor(5);
    let refused_admission = make_admission(RiskPolicy::Inherit, input(80));
    let refused_from = tile(0, 1);
    let refused_to = tile(10, 1);
    sample_route("refused_route", 3, || {
        route(
            &refused_world,
            &refused_admission,
            options,
            |opts| real_search(&refused_world, refused_from, refused_to, opts, &state, &[]),
            || {
                real_witness(
                    &refused_world,
                    refused_from,
                    refused_to,
                    options,
                    &state,
                    &[],
                )
            },
        )
    });

    let re_combat = selected_combat();
    let re_search_world = fixture_world(
        re_combat.clone(),
        open_corridor(160, 3, 1),
        [5, 10, 15, 20, 25, 30, 35, 40]
            .into_iter()
            .map(|x| zone(x, 0, None))
            .collect(),
        vec![ice_kind(&re_combat)],
        Vec::new(),
        TransportGraph::default(),
    );
    let re_search_admission = make_admission(RiskPolicy::Inherit, input(99));
    let re_search_to = tile(150, 1);
    sample_route("worst_research_8_witnesses", 5, || {
        route(
            &re_search_world,
            &re_search_admission,
            options,
            |opts| real_search(&re_search_world, safe_from, re_search_to, opts, &state, &[]),
            || {
                real_witness(
                    &re_search_world,
                    safe_from,
                    re_search_to,
                    options,
                    &state,
                    &[],
                )
            },
        )
    });

    let snapshot = api::snapshot::GameSnapshot::new();
    let _ = capture(&snapshot, false, PoisonState::Clear, false);
    let publish_world = empty_corridor();
    let publish_from = tile(0, 1);
    let publish_to = tile(8, 1);
    let publish_route = nav::router::find_with_avoid(
        &publish_world.collision,
        &publish_world.graph,
        publish_from,
        publish_to,
        options,
        &state,
        &[],
    )
    .unwrap();
    let mut publish_admission = make_admission(RiskPolicy::Inherit, input(80));
    publish_admission.input.pos = publish_from;
    let worker_assessment = assess(&publish_route, &publish_world, &publish_admission);
    let mut samples = Vec::with_capacity(SAMPLES);
    for iteration in 0..(WARMUPS + SAMPLES) {
        let mut bot = crate::script_runtime::NavBot {
            route: Some(publish_route.clone()),
            admission: Some(Box::new(publish_admission)),
            assessment: Some(Arc::clone(&worker_assessment)),
            admission_pending: true,
            walk_request_id: 29,
            route_generation: 17,
            ..Default::default()
        };
        let started = Instant::now();
        let (_, kernels) = count_kernels(|| {
            publish(&mut bot, &publish_world, &snapshot, false, None);
        });
        let elapsed = started.elapsed().as_nanos();
        assert_eq!(kernels, 0, "publication reassessment does not route");
        assert!(bot.route.is_none());
        assert_eq!(
            bot.risk_refusal.as_deref().map(|entry| entry.1),
            Some(WalkRefusal::Unknown(UnknownWhy::MissingFacts))
        );
        if iteration >= WARMUPS {
            samples.push(elapsed);
        }
    }
    summarize("publication_reassessment", samples, 0);
}

fn transport_edge(kind: TransportKind, from: WorldTile, to: WorldTile) -> TransportEdge {
    TransportEdge {
        kind,
        at: from,
        to,
        takeoff: None,
        player_delta: None,
        loc_id: -1,
        option: 0,
        ticks: 1,
        dir: None,
        open_loc_id: None,
        skill_req: Vec::new(),
        item_req: Vec::new(),
        consumed_req: Vec::new(),
        item_returns: Vec::new(),
        quest_req: Vec::new(),
        varp_req: Vec::new(),
        worn_req: Vec::new(),
        worn_all_req: Vec::new(),
        members_req: false,
        wildy_cap: None,
        quest_gates: None,
    }
}

fn escape(state: EscapeState) -> Escape {
    Escape {
        action: EscapeAction::Retreat,
        state,
        owner: EscapeOwner::Native,
        allow: WalkAllow::default(),
        from: tile(0, 1),
        to: tile(4, 1),
        hp: 10,
        floor: 20,
        volley: 4,
        tick: 12,
        generation: 17,
        switches: 0,
        last_hit: 12,
        retired: None,
    }
}

struct UnknownEvidence;

impl EvidenceProvider for UnknownEvidence {
    fn test_gate(&self, _gate: &QuestGate, _required_after: EvidenceStamp) -> Truth {
        Truth::Unknown
    }
}
