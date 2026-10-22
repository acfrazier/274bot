use super::*;
use crate::admission::{RiskPolicy, WalkAllow, WalkRefusal};
use api::quest_progress::EvidenceProvider;
use api::selected::ClientRevision;
use api::selected::{QuestGate, RunKey, Truth};
use api::snapshot::WorldTile;
use client::dash3d::CollisionFlag;
use nav::collision::{pack_walk, WorldCollision};
use nav::quest_gates::{QuestEvidence, QuestFamilyId};
use nav::router::{Leg, RouteError};
use nav::transport::TransportGraph;
#[cfg(feature = "test-support")]
use nav::transport::{TransportEdge, TransportKind, WildernessRules, WildernessZone};
use nav::zones::{Zone, ZoneClass, ZoneGroup, ZoneKind, ZoneTable, NO_GROUP};
use nav::WorldState;
use script::combat::guard::{Escape, EscapeAction, EscapeOwner, EscapeState};
use script::combat::request::{ActorKind, ActorRef};
use script::combat::risk::{KindRisk, LiveRow, UnknownKind};
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

#[cfg(feature = "test-support")]
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
    let risks = RiskTables::build(&combat);
    prime_tables(&world, Some(Tables { combat, risks }));
    world
}

/// Script's facts for zone row `row`, read through the facade kind view as
/// the host's witness check reads them.
fn kind_facts(world: &NavWorld, row: u16) -> KindRisk {
    let zones = producer::zones_view(world).unwrap().unwrap();
    let bound = producer::bind_key(zones, ZoneKey::Zone(row)).unwrap();
    tables(world)
        .unwrap()
        .risks
        .zone_kind(zones.zone(bound).unwrap().kind())
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

#[cfg(feature = "test-support")]
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
        enforce: true,
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
    let lethal_hp = kind_facts(&lethal_world, 0).max_hit;
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
            let kind = kind_facts(&world, 0);
            assert!(kind.unknown_for(input.map_members).is_some());
            assert_eq!(why, UnknownWhy::MissingFacts);
        } else {
            let max_hit = kind_facts(&world, 0).max_hit;
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

/// T5.1 r3: the witness check binds each raw key to the zone view and reads
/// facts through `ZoneKindView`. Its verdicts equal both a literal table and
/// the round-2 rule, re-derived from the raw kind rows (an NPC's selected
/// facts or missing facts; for a hazard, its curated damage), for an NPC
/// kind, a curated hazard, an uncurated hazard, a missing-facts kind, a
/// known group, a group with one missing member, mixed key sets and keys
/// this pack does not name.
#[test]
fn witness_verdicts_through_the_kind_view_match_the_round_two_rule() {
    let combat = selected_combat();
    let kinds = vec![
        ice_kind(&combat),
        ZoneKind::new("ikov-lava-bridge", "Lava bridge", -1, 0, false, false),
        ZoneKind::new("t51-uncurated", "Uncurated hazard", -1, 0, false, false),
        ZoneKind::new(
            "missing-kind",
            "Missing selected facts",
            i32::from(i16::MAX),
            1,
            false,
            false,
        ),
    ];
    let hazard = |min_x, max_x, kind| {
        Zone::hazard(
            AvoidRect {
                min_x,
                max_x,
                min_z: 1,
                max_z: 1,
                level: Some(0),
            },
            0,
            kind,
        )
    };
    let group = |id: &str, min_x, max_x, members: Vec<u16>| {
        ZoneGroup::new(
            id.to_string(),
            id.to_string(),
            AvoidRect {
                min_x,
                max_x,
                min_z: 1,
                max_z: 1,
                level: Some(0),
            },
            members,
        )
    };
    // Rows: 0 ice, 1 curated hazard, 2 uncurated hazard, 3 missing;
    // group 0 = rows 4, 5 (ice, ice); group 1 = rows 6, 7 (ice, missing).
    let zones = vec![
        zone(2, 0, None),
        hazard(4, 5, 1),
        hazard(7, 7, 2),
        zone(9, 3, None),
        zone(11, 0, Some(0)),
        zone(13, 0, Some(0)),
        zone(15, 0, Some(1)),
        zone(17, 3, Some(1)),
    ];
    let world = fixture_world(
        combat,
        open_corridor(24, 3, 1),
        zones,
        kinds,
        vec![
            group("known", 11, 13, vec![4, 5]),
            group("partly", 15, 17, vec![6, 7]),
        ],
        TransportGraph::default(),
    );

    // The facts each row's kind view yields.
    let ice = kind_facts(&world, 0);
    assert_eq!(
        (ice.max_hit, ice.unknown, ice.hazard()),
        (6, UnknownKind::Known, false)
    );
    let lava = kind_facts(&world, 1);
    assert_eq!(
        (lava.max_hit, lava.unknown, lava.hazard()),
        (20, UnknownKind::Known, true)
    );
    assert_eq!(kind_facts(&world, 2).unknown, UnknownKind::HazardDamage);
    assert_eq!(kind_facts(&world, 3).unknown, UnknownKind::MissingFacts);
    for row in [4, 5, 6] {
        assert_eq!(kind_facts(&world, row), ice);
    }
    assert_eq!(kind_facts(&world, 7).unknown, UnknownKind::MissingFacts);

    // Round 2's rule over raw storage: the selected NPC's facts (or missing
    // facts), or a hazard's curated damage (unknown when uncurated).
    let table = world.graph.zones.as_ref().unwrap();
    let risks = &tables(&world).unwrap().risks;
    let row_ok = |row: u16, input: &RiskInput| {
        let Some(zone) = table.zones().get(usize::from(row)) else {
            return false;
        };
        let kind = &table.kinds()[usize::from(zone.kind)];
        if kind.npc_id == -1 {
            nav::zones::curated::hazard_damage(kind.id.as_ref()).is_some_and(|max| max < input.hp)
        } else {
            risks.npc(kind.npc_id).is_some_and(|facts| {
                facts.unknown_for(input.map_members).is_none() && facts.max_hit < input.hp
            })
        }
    };
    let round_two = |keys: &[ZoneKey], input: &RiskInput| {
        !keys.is_empty()
            && keys.iter().all(|key| match *key {
                ZoneKey::Zone(row) => row_ok(row, input),
                ZoneKey::Group(index) => {
                    table.groups().get(usize::from(index)).is_some_and(|group| {
                        !group.members.is_empty()
                            && group.members.iter().all(|&row| row_ok(row, input))
                    })
                }
            })
    };

    // Literal table: the largest hit each key's members can take, `None`
    // when any member is unsupported or the key names nothing here.
    let cases: [(&[ZoneKey], Option<u8>); 12] = [
        (&[ZoneKey::Zone(0)], Some(6)),
        (&[ZoneKey::Zone(1)], Some(20)),
        (&[ZoneKey::Zone(2)], None),
        (&[ZoneKey::Zone(3)], None),
        (&[ZoneKey::Group(0)], Some(6)),
        (&[ZoneKey::Group(1)], None),
        (&[ZoneKey::Zone(0), ZoneKey::Zone(1)], Some(20)),
        (&[ZoneKey::Zone(1), ZoneKey::Group(0)], Some(20)),
        (&[ZoneKey::Group(0), ZoneKey::Zone(3)], None),
        (&[ZoneKey::Zone(8)], None),
        (&[ZoneKey::Group(2)], None),
        (&[], None),
    ];
    let mut admitted = 0;
    for map_members in [false, true] {
        for hp in [1, 6, 7, 20, 21, 99] {
            let input = RiskInput {
                map_members,
                ..input(hp)
            };
            for (keys, max_hit) in cases {
                let verdict = witness_admissible(&world, &input, keys);
                let context = format!("keys {keys:?}, hp {hp}, members {map_members}");
                assert_eq!(verdict, max_hit.is_some_and(|max| max < hp), "{context}");
                assert_eq!(verdict, round_two(keys, &input), "{context}");
                admitted += usize::from(verdict);
            }
        }
    }
    // Both verdicts occur, for each member flag: hp 7 and 20 admit 2 key
    // sets each, hp 21 and 99 admit 5 each.
    assert_eq!(admitted, 2 * (2 + 2 + 5 + 5));
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
        prefer_target_order: false,
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
    let world = known_corridor(&[20]);
    let from = tile(0, 1);
    let to = tile(20, 1);
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
    assert_eq!(kernels, 1, "endpoint completion needs no named retry");
    assert_eq!(crossing.tried, 0);
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
        .contains("Poison status is unknown until the bot can track it"));
    assert!(!unknown_assessment.reason.contains("S2c"));

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
    assert_eq!(override_kernels, 1);
    assert!(overridden.refusal.is_none());
    let override_assessment = overridden.assessment.as_ref().unwrap();
    assert_eq!(
        override_assessment.verdict,
        Verdict::Unknown(UnknownWhy::Poison)
    );
    assert!(override_assessment
        .reason
        .starts_with("Poison status is unknown until the bot can track it; "));

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
    assert_eq!(poisoned_kernels, 1);
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
        verdict_refusal(Verdict::Unknown(UnknownWhy::Kind(UnknownKind::Magic))),
        WalkRefusal::Unknown(UnknownWhy::Kind(UnknownKind::Magic))
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
    let world = known_corridor(&[20]);
    let from = tile(0, 1);
    let to = tile(20, 1);
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
        route: Some(Arc::new(route)),
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
    let world = known_corridor(&[20]);
    let from = tile(0, 1);
    let to = tile(20, 1);
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
        route: Some(Arc::new(route)),
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
        route: Some(Arc::new(route)),
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
        prefer_target_order: false,
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
    let original_route = Arc::new(original_route);
    let original = Arc::new(RouteBasis {
        _route: crate::walk_plan::RoutePath::legacy(Arc::clone(&original_route)),
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
    assert!(Arc::ptr_eq(
        arm.route.as_ref().unwrap(),
        retained._route.legacy_route().unwrap()
    ));
    let refreshed_route = Arc::new(refreshed_route);
    let refreshed = Arc::new(RouteBasis {
        _route: crate::walk_plan::RoutePath::legacy(Arc::clone(&refreshed_route)),
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
    assert_eq!(retained._route.destination(), tile(1, 1));
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
            route: Some(Arc::new(publish_route.clone())),
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
        assert!(bot.route.is_some(), "missing facts cannot refuse fleeing");
        assert!(bot.risk_refusal.is_none());
        assert_eq!(
            bot.assessment.as_ref().unwrap().verdict,
            Verdict::Unknown(UnknownWhy::MissingFacts)
        );
        if iteration >= WARMUPS {
            samples.push(elapsed);
        }
    }
    summarize("publication_reassessment", samples, 0);

    // Exercise the long, many-leg S2a assessment seam without copying tiles.
    let combat = selected_combat();
    let long_world = fixture_world(
        combat.clone(),
        open_corridor(2048, 3, 1),
        [100, 400, 700, 1000, 1300, 1600, 1900]
            .into_iter()
            .map(|x| zone(x, 0, None))
            .collect(),
        vec![ice_kind(&combat)],
        Vec::new(),
        TransportGraph::default(),
    );
    let long_route = Route {
        legs: (0..64)
            .map(|leg| Leg::Walk {
                tiles: (leg * 32..(leg + 1) * 32).map(|x| tile(x, 1)).collect(),
            })
            .collect(),
        dest: tile(2047, 1),
        ticks: 1023.5,
    };
    let long_admission = make_admission(RiskPolicy::Avoid, input(99));
    let mut samples = Vec::with_capacity(SAMPLES);
    for iteration in 0..(WARMUPS + SAMPLES) {
        let started = Instant::now();
        let ((assessment, permitted), kernels) = count_kernels(|| {
            let assessment = assess(&long_route, &long_world, &long_admission);
            let permitted = permits(&long_route, &long_world, &long_admission, &assessment);
            (assessment, permitted)
        });
        let elapsed = started.elapsed().as_nanos();
        assert_eq!(kernels, 0);
        assert!(assessment.plan.complete);
        assert_eq!(assessment.plan.intervals.len(), 7);
        assert_eq!(assessment.plan.crossings.len(), 7);
        assert!(!permitted);
        if iteration >= WARMUPS {
            samples.push(elapsed);
        }
    }
    summarize("long_2048_tiles_64_legs_7_crossings", samples, 0);
}

#[cfg(feature = "test-support")]
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
        forbidden_held: Vec::new(),
        forbidden_worn: Vec::new(),
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

#[cfg(feature = "test-support")]
#[test]
fn unknown_poison_witness_is_not_researched() {
    let world = known_corridor(&[12]);
    let from = tile(0, 1);
    let to = tile(23, 1);
    let options = FindOptions::default();
    let state = WorldState::default();
    let request = make_admission(
        RiskPolicy::Inherit,
        RiskInput {
            poison: PoisonState::default(),
            ..input(99)
        },
    );
    let (result, kernels) = count_kernels(|| {
        route(
            &world,
            &request,
            options,
            |opts| real_search(&world, from, to, opts, &state, &[]),
            || real_witness(&world, from, to, options, &state, &[]),
        )
    });
    assert_eq!(
        kernels, 3,
        "strict plus diagnosis only; no doomed named retry"
    );
    assert_eq!(result.tried, 0);
    assert_eq!(result.blocked.as_ref(), &[ZoneKey::Zone(0)]);
    assert!(matches!(
        result.refusal,
        Some(WalkRefusal::NoRouteWithinBounds { tried: 0, .. })
    ));
}

#[test]
fn missing_facts_at_publication_never_refuse_a_no_crossing_walk() {
    let world = empty_corridor();
    let route = Arc::new(
        nav::router::find_with_avoid(
            &world.collision,
            &world.graph,
            tile(0, 1),
            tile(8, 1),
            FindOptions::default(),
            &WorldState::default(),
            &[],
        )
        .unwrap(),
    );
    let snapshot = api::snapshot::GameSnapshot::new();
    for policy in [RiskPolicy::Avoid, RiskPolicy::Inherit, RiskPolicy::Proceed] {
        let admission = make_admission(policy, input(80));
        let assessment = assess(&route, &world, &admission);
        let mut bot = crate::script_runtime::NavBot {
            route: Some(Arc::clone(&route)),
            admission: Some(Box::new(admission)),
            assessment: Some(Arc::clone(&assessment)),
            admission_pending: true,
            ..Default::default()
        };
        publish(&mut bot, &world, &snapshot, false, None);
        assert!(Arc::ptr_eq(bot.route.as_ref().unwrap(), &route));
        assert!(bot.risk_refusal.is_none());
        assert_eq!(
            bot.assessment.as_ref().unwrap().verdict,
            Verdict::Unknown(UnknownWhy::MissingFacts)
        );
        let mut arm = crate::WalkArm {
            route: Some(Arc::clone(&route)),
            admission: Some(Box::new(admission)),
            assessment: Some(assessment),
            admission_pending: true,
            ..Default::default()
        };
        assert!(!crate::observe_walk_arm_admission(
            &snapshot, &mut arm, &world, false, None
        ));
        assert!(Arc::ptr_eq(arm.route.as_ref().unwrap(), &route));
        assert!(arm.refusal.is_none());
        assert_eq!(
            arm.assessment.as_ref().unwrap().verdict,
            Verdict::Unknown(UnknownWhy::MissingFacts)
        );
    }
}

#[test]
fn held_inherited_admission_keeps_the_exact_legacy_router_outcome() {
    const { assert!(!crate::NET_AVAILABLE) };
    let world = known_corridor(&[12]);
    let state = WorldState::default();
    for level in [
        crate::DangerLevel::Never,
        crate::DangerLevel::WhenSurvivable,
        crate::DangerLevel::Always,
    ] {
        let mut globals = crate::WalkGlobals::default();
        globals.set_danger_level(level);
        let options = globals.manual_options(false);
        for (from, to) in [
            (tile(0, 1), tile(5, 1)),
            (tile(0, 1), tile(12, 1)),
            (tile(12, 1), tile(0, 1)),
            (tile(0, 1), tile(23, 1)),
        ] {
            let expected = real_search(&world, from, to, options, &state, &[]);
            let request = Admission {
                policy: globals.risk_policy(Default::default(), Default::default()),
                enforce: globals.enforces_risk(Default::default(), Default::default()),
                ..Admission::manual(
                    options,
                    RiskInput {
                        pos: from,
                        poison: PoisonState::default(),
                        unattributed: true,
                        ..input(1)
                    },
                    1,
                    globals,
                )
            };
            let actual = route(
                &world,
                &request,
                options,
                |opts| real_search(&world, from, to, opts, &state, &[]),
                || real_witness(&world, from, to, options, &state, &[]),
            );
            assert_eq!(actual.tried, 0);
            match (&expected, &actual.outcome) {
                (
                    crate::RouteOutcome::Routed(expected),
                    crate::RouteOutcome::Routed(actual_route),
                ) => {
                    assert_eq!(expected, actual_route, "{level:?}: {from:?}->{to:?}");
                    assert!(actual.refusal.is_none());
                }
                (crate::RouteOutcome::NoPath, crate::RouteOutcome::NoPath) => {}
                _ => panic!("held admission changed router outcome: {level:?}: {from:?}->{to:?}"),
            }
        }
    }
}

/// R3 M-A: the named retry's keys never reach `admission.grants`, so the
/// retried route must be judged. From a fringe origin (inside E_z, outside
/// A_z) walking into A_z is a crossing, never an unjudged escape.
#[test]
fn named_retry_from_a_fringe_origin_still_judges_the_crossing() {
    let combat = selected_combat();
    let kind = ice_kind(&combat);
    let spawn = 30;
    let world = fixture_world(
        combat,
        open_corridor(70, 3, 1),
        vec![zone(spawn, 0, None)],
        vec![kind],
        Vec::new(),
        TransportGraph::default(),
    );
    let r = i32::from(kind_facts(&world, 0).r);
    let from = tile(spawn - r, 1);
    let to = tile(69, 1);
    let state = WorldState::default();
    let options = FindOptions::default();
    for (hp, refusal) in [(12, Some(WalkRefusal::Unsurvivable)), (99, None)] {
        let request = make_admission(
            RiskPolicy::Inherit,
            RiskInput {
                pos: from,
                ..input(hp)
            },
        );
        let result = route(
            &world,
            &request,
            options,
            |opts| real_search(&world, from, to, opts, &state, &[]),
            || real_witness(&world, from, to, options, &state, &[]),
        );
        assert_eq!(result.tried, 1, "HP {hp}: strict NoPath, one named retry");
        assert_eq!(request.grants, ZoneExempt::NONE);
        let assessment = result.assessment.as_ref().unwrap();
        assert_eq!(assessment.plan.crossings.len(), 1, "HP {hp}");
        assert!(assessment.plan.intervals.iter().all(|row| !row.escaping()));
        assert_eq!(result.refusal, refusal, "HP {hp}: {}", assessment.reason);
    }
}

/// R3 L-C (`nav_fares_live` "refusal armed a walk"): a held default manual
/// NoPath is the pre-S2b bare refusal. It must not create an arm for the
/// focused slot, nor stop the walk an existing arm is following. Enforcing
/// refusals keep S2b's retained arm refusal.
#[test]
fn held_manual_no_path_leaves_walk_arms_untouched() {
    const { assert!(!crate::NET_AVAILABLE) };
    let world = known_corridor(&[12]);
    let state = WorldState::default();
    let from = nav::tile::Tile {
        x: 0,
        z: 1,
        level: 0,
    };
    let to = nav::tile::Tile {
        x: 23,
        z: 1,
        level: 0,
    };
    let walking = Arc::new(nav::router::Route {
        legs: vec![Leg::Walk {
            tiles: vec![tile(0, 1), tile(1, 1)],
        }],
        dest: tile(1, 1),
        ticks: 0.5,
    });
    let arms = crate::WalkArms::default();
    arms.lock().unwrap().insert(
        "alice".into(),
        Arc::new(std::sync::Mutex::new(crate::WalkArm {
            route: Some(Arc::clone(&walking)),
            ..Default::default()
        })),
    );
    let held = Admission::manual(
        FindOptions::default(),
        RiskInput {
            pos: tile(0, 1),
            ..input(99)
        },
        1,
        crate::WalkGlobals::default(),
    );
    assert!(!held.enforce);
    for name in ["alice", "bob"] {
        let refused = crate::arm_walk_on(
            &world,
            from,
            to,
            FindOptions::default(),
            &state,
            &nav::bank_fetch::BankRows::default(),
            held,
            &arms,
            Some(name),
        )
        .unwrap_err();
        assert!(refused.refusal.is_some(), "the witness names the blocker");
    }
    {
        let arms = arms.lock().unwrap();
        assert!(!arms.contains_key("bob"), "a held NoPath creates no arm");
        let alice = arms["alice"].lock().unwrap();
        assert!(Arc::ptr_eq(alice.route.as_ref().unwrap(), &walking));
        assert!(alice.refusal.is_none());
    }
    let enforcing = Admission {
        enforce: true,
        ..held
    };
    let refused = crate::arm_walk_on(
        &world,
        from,
        to,
        FindOptions::default(),
        &state,
        &nav::bank_fetch::BankRows::default(),
        enforcing,
        &arms,
        Some("alice"),
    )
    .unwrap_err();
    let arms = arms.lock().unwrap();
    let alice = arms["alice"].lock().unwrap();
    assert!(alice.route.is_none(), "an enforcing refusal stops the walk");
    assert_eq!(alice.refusal, refused.refusal);
}

fn unknown_poison(hp: u8) -> RiskInput {
    RiskInput {
        poison: PoisonState::Unknown { since: 0 },
        ..input(hp)
    }
}

/// SURVIVABLE-DEST-GRANTS: a `cross` walk names its transit grant and relies
/// on the router's endpoint completion for the zone it ends in. That
/// destination zone is granted, not judged, so Unknown poison (no S2c
/// observer yet) does not refuse the walk the Path authored.
#[test]
fn named_cross_walk_admits_its_destination_zone_with_unknown_poison() {
    let world = known_corridor(&[5, 20]);
    let (from, to) = (tile(0, 1), tile(20, 1));
    let state = WorldState::default();
    let grants = ZoneExempt::named(&[ZoneKey::Zone(0)]).unwrap();
    let options = FindOptions {
        zones: grants,
        ..FindOptions::default()
    };
    let admission = Admission {
        grants,
        ..make_admission(RiskPolicy::Inherit, unknown_poison(99))
    };
    let result = route(
        &world,
        &admission,
        options,
        |opts| real_search(&world, from, to, opts, &state, &[]),
        || real_witness(&world, from, to, options, &state, &[]),
    );
    assert_eq!(result.refusal, None);
    assert_eq!(routed(&result).dest, to);
    let assessment = result.assessment.as_ref().unwrap();
    assert_eq!(assessment.verdict, Verdict::Unknown(UnknownWhy::Poison));
    assert_eq!(
        assessment.plan.crossings.len(),
        2,
        "the named transit crossing and the destination crossing"
    );
    assert_eq!(
        assessment.input.poison,
        PoisonState::Unknown { since: 0 },
        "admission grants the zone; it never claims the poison clear"
    );
}

/// The destination grant is the route endpoint's zones only. A crossing of an
/// unnamed zone on the way, separate from or merged into the destination's
/// crossing, is still judged and refused while poison is Unknown.
#[test]
fn named_cross_walk_still_judges_an_ungranted_zone_on_the_way() {
    let state = WorldState::default();
    let (from, to) = (tile(0, 1), tile(20, 1));
    for (spawns, shape) in [([5, 12, 20], "separate"), ([5, 18, 20], "merged")] {
        let world = known_corridor(&spawns);
        // The router never hands admission this route under these grants; a
        // route that crosses the unnamed zone must still be judged if it does.
        let crossing = match real_search(
            &world,
            from,
            to,
            FindOptions {
                zones: ZoneExempt::all(),
                ..FindOptions::default()
            },
            &state,
            &[],
        ) {
            crate::RouteOutcome::Routed(route) => route,
            _ => panic!("{shape}: the open corridor routes under all()"),
        };
        assert_eq!(crossing.dest, to);
        let named = Admission {
            grants: ZoneExempt::named(&[ZoneKey::Zone(0)]).unwrap(),
            ..make_admission(RiskPolicy::Inherit, unknown_poison(99))
        };
        let assessment = assess(&crossing, &world, &named);
        assert_eq!(assessment.verdict, Verdict::Unknown(UnknownWhy::Poison));
        assert!(
            !permits(&crossing, &world, &named, &assessment),
            "{shape}: the unnamed zone at {} is judged",
            spawns[1]
        );
        let both = Admission {
            grants: ZoneExempt::named(&[ZoneKey::Zone(0), ZoneKey::Zone(1)]).unwrap(),
            ..named
        };
        assert!(
            permits(&crossing, &world, &both, &assessment),
            "{shape}: naming it as well grants every crossing"
        );
        let held = Admission {
            enforce: false,
            ..named
        };
        assert!(permits(&crossing, &world, &held, &assessment));
    }
}

fn path_step<'a>(value: &'a serde_json::Value, id: &str) -> Option<&'a serde_json::Value> {
    match value {
        serde_json::Value::Object(map) if map.get("id").and_then(|v| v.as_str()) == Some(id) => {
            Some(value)
        }
        serde_json::Value::Object(map) => map.values().find_map(|child| path_step(child, id)),
        serde_json::Value::Array(items) => items.iter().find_map(|child| path_step(child, id)),
        _ => None,
    }
}

/// One squire Path walk step through the native request seam and its
/// approach tiles, at combat 40 with Unknown poison and `cross` enforcing.
fn real_squire_walk(
    world: &Arc<NavWorld>,
    squire: &serde_json::Value,
    step: &str,
    from: WorldTile,
) -> (WorldTile, ZoneExempt, RouteAdmission) {
    let args = &path_step(squire, step).expect("squire step")["args"];
    let at = |index: usize| i32::try_from(args["tile"][index].as_i64().unwrap()).unwrap();
    let to = WorldTile {
        x: at(0),
        z: at(1),
        level: at(2),
    };
    let cross: Vec<Arc<str>> = args["cross"]
        .as_array()
        .unwrap()
        .iter()
        .map(|name| Arc::from(name.as_str().unwrap()))
        .collect();
    assert!(!cross.is_empty(), "{step} names its crossings");
    let state = WorldState {
        combat_level: Some(40),
        ..WorldState::default()
    };
    let (opts, _) = crate::resolve_route_exclusions(
        FindOptions::default(),
        world,
        from,
        to,
        &state,
        crate::ScriptRouteExclusions {
            cross,
            ..Default::default()
        },
    )
    .unwrap();
    let globals = crate::WalkGlobals::default();
    let admission = Admission {
        policy: globals.risk_policy(Default::default(), Default::default()),
        // A nonempty `cross` arms enforcement while the default is held.
        enforce: true,
        grants: opts.zones,
        allow: WalkAllow::default(),
        input: RiskInput {
            pos: from,
            combat: Some(40),
            hp_max: 40,
            ..unknown_poison(40)
        },
        generation: 1,
        compat_v1: false,
        escape: None,
    };
    let request = crate::ScriptRouteRequest {
        candidate_goals: None,
        admission,
        generation: 1,
        request_id: 1,
        world: Arc::clone(world),
        from,
        to,
        radius: i32::try_from(args["radius"].as_i64().unwrap()).unwrap(),
        loc_id: None,
        arrival: nav::arrival::ArrivalKind::Reach,
        opts,
        state: Some(state),
        bank_origin: api::bank_memory::Origin::Unknown,
        bank: Vec::new(),
        live_geometry: None,
        exclusions: None,
        compat_tile_intent: false,
        completion: Default::default(),
    };
    let (admitted, _, options) = request.drive_route_admission_for_test();
    (to, options.zones, admitted)
}

/// The headed Knight's Sword stall (DIAG-SURVIVABLE-POISON): squire's own
/// `ice-walk-1-entry` step from the ice-dungeon ladder, then the rest of the
/// ice chain from each previous stand, all with Unknown poison.
/// `ice-walk-1-entry` (radius-1 stand inside a hobgoblin zone) and
/// `ice-walk-3-to-hobgoblins` (six pirates fill the 8-key cap; the
/// hobgoblin destination is unnamed) are admitted only by the endpoint grant.
#[test]
#[ignore = "requires WORLD_NAV_PACK pointing to the real 289 nav pack"]
fn real_squire_ice_chain_admits_named_and_destination_zones_with_unknown_poison() {
    let pack = std::env::var_os("WORLD_NAV_PACK").expect("WORLD_NAV_PACK is required");
    let world = Arc::new(NavWorld::load_pack(std::path::Path::new(&pack)).unwrap());
    let squire: serde_json::Value =
        serde_json::from_str(include_str!("../../script/paths/289/squire.json")).unwrap();
    let mut from = WorldTile {
        x: 3009,
        z: 9550,
        level: 0,
    };
    for (step, endpoint_granted) in [
        ("ice-walk-1-entry", true),
        ("ice-walk-2-pirates", false),
        ("ice-walk-3-to-hobgoblins", true),
        ("ice-walk-4-hobgoblins", false),
        ("ice-walk-5-blurite-rock", false),
        ("leave-ice-5", false),
        ("leave-ice-4", false),
        ("leave-ice-3", false),
        ("leave-ice-2", false),
        ("leave-ice-1", false),
    ] {
        let (to, grants, admitted) = real_squire_walk(&world, &squire, step, from);
        let route = routed(&admitted);
        let assessment = admitted.assessment.as_ref().expect("a routed assessment");
        assert_eq!(
            admitted.refusal, None,
            "{step} from {from:?} to {:?}: {}",
            route.dest, assessment.reason
        );
        assert!((route.dest.x - to.x).abs() <= 1 && (route.dest.z - to.z).abs() <= 1);
        assert_eq!(assessment.input.poison, PoisonState::Unknown { since: 0 });
        let zones = producer::zones_view(&world).unwrap().unwrap();
        let named_only = assessment.plan.crossings.is_empty()
            || risk::admission_passes(
                producer::route_view(&*world, route).unwrap(),
                assessment,
                zones,
                &tables(&world).unwrap().combat,
                WalkAllow::default(),
                risk::Grants::named(producer::bind_exempt(zones, grants).unwrap()),
            )
            .unwrap_or(false);
        assert_eq!(
            !named_only, endpoint_granted,
            "{step}: whether only the endpoint grant admits it"
        );
        from = route.dest;
    }
}
#[cfg(test)]
mod compact_final_route_tests {
    use super::*;
    use api::identity::{ProcessIncarnation, RequestIdMinter, Scope, WorkIdentity};
    use api::work::{
        InputRevision, InspectionPhase, MonotonicClock, SharedCancellation, SliceBudget,
        StepContext, SystemClock,
    };
    use std::num::NonZeroU32;
    use std::sync::Mutex;

    #[derive(Default)]
    struct IdleOut;

    impl api::prot::Out for IdleOut {
        fn p1_enc(&mut self, _opcode: i32) {}
        fn p1(&mut self, _value: i32) {}
        fn p2(&mut self, _value: i32) {}
        fn p4(&mut self, _value: i32) {}
        fn pjstr(&mut self, _value: &str) {}
    }

    #[derive(Default)]
    struct IdleDriver {
        out: IdleOut,
    }

    impl api::interact::Driver for IdleDriver {
        fn set_menu(&mut self, _slot: i32, _action: i32, _a: i32, _b: i32, _c: i32) {}
        fn do_action(&mut self, _slot: i32) -> bool {
            false
        }
        fn try_move(
            &mut self,
            _src_x: i32,
            _src_z: i32,
            _dx: i32,
            _dz: i32,
            _try_nearest: bool,
            _loc_width: i32,
            _loc_length: i32,
            _loc_angle: i32,
            _loc_shape: i32,
            _forceapproach: i32,
            _type: i32,
        ) -> bool {
            false
        }
        fn local_route(&self) -> Option<(i32, i32)> {
            None
        }
        fn build_base(&self) -> (i32, i32) {
            (0, 0)
        }
        fn loc_typecode(&self, _scene_x: i32, _scene_z: i32) -> Option<i32> {
            None
        }
        fn out(&mut self) -> &mut dyn api::prot::Out {
            &mut self.out
        }
        fn login(&mut self, _username: &str, _password: &str, _reconnect: bool) -> bool {
            false
        }
    }

    fn compact_route_path(
        world: &Arc<NavWorld>,
        from: WorldTile,
        to: WorldTile,
    ) -> crate::walk_plan::RoutePath {
        let lease = producer::lease(world).expect("fixture navigation lease");
        let mut cursor = nav::router::SearchCursor::new(
            nav::router::SearchRequest::Single {
                from,
                to,
                options: FindOptions {
                    zones: ZoneExempt::all(),
                    ..FindOptions::default()
                },
                avoid: Box::default(),
                max_settles: 1_024,
            },
            lease.clone(),
            Arc::new(WorldState::default()),
        )
        .expect("small fixture route cursor");
        let clock = SystemClock;
        let cancellation = SharedCancellation::new();
        let units = NonZeroU32::new(64).expect("nonzero work slice");
        let completion = loop {
            let budget = SliceBudget::new(
                units,
                api::identity::Deadline::after(clock.now(), std::time::Duration::from_secs(5)),
            );
            let mut context = StepContext::<InspectionPhase>::new(&clock, budget, &cancellation);
            match cursor.advance(&mut context) {
                nav::router::CursorStep::Complete(completion) => break completion,
                nav::router::CursorStep::Yielded => {}
                nav::router::CursorStep::Cancelled(reason) => {
                    panic!("fixture route cancelled: {reason:?}")
                }
                nav::router::CursorStep::Failed(error) => {
                    panic!("fixture route failed: {error:?}")
                }
            }
        };
        let (outcome, _) = completion.into_parts();
        let nav::router::SearchOutcome::Single { route: Ok(route) } = outcome else {
            panic!("fixture search did not return one compact route")
        };
        let bytes =
            nav::producer::compact_route_result_bytes(&route).expect("route size is bounded");
        let ledger = nav::producer::OwnerResultLedger::with_limits(bytes, bytes);
        let reservation = ledger
            .try_reserve_bytes(bytes)
            .expect("fixture route result reservation");
        let identity = WorkIdentity::new(
            RequestIdMinter::new().mint().expect("host request id"),
            ProcessIncarnation::from_start_nonce(
                std::num::NonZeroU64::new(1).expect("nonzero process nonce"),
            ),
            Scope::Process,
            None,
        )
        .expect("valid fixture work identity");
        let plan = nav::producer::publish_compact_route(
            &lease,
            identity,
            InputRevision::FIRST,
            route,
            reservation.clone(),
        );
        let custody: Arc<dyn std::any::Any + Send + Sync> =
            Arc::new(reservation.hold(plan.clone()));
        crate::walk_plan::RoutePath::compact(plan, lease, custody)
    }

    #[test]
    fn compact_script_nav_paint_borrows_the_compact_route_projection() {
        let world = Arc::new(empty_corridor());
        let from = tile(0, 1);
        let dest = tile(3, 1);
        let route = compact_route_path(&world, from, dest);
        let bot = crate::script_runtime::NavBot {
            compact_route: Some(route),
            map_route_generation: 9,
            ..Default::default()
        };
        let navs = Arc::new(Mutex::new(std::collections::HashMap::from([(
            "alice".to_owned(),
            bot,
        )])));
        let paint = crate::ScriptNavPaint { navs };
        paint.with_map_route("alice", |projection| {
            let projection = projection.expect("compact script route");
            assert_eq!(
                projection.stamp.source,
                crate::walk_map::RouteSource::Script
            );
            assert_eq!(projection.stamp.generation, 9);
            assert_eq!(projection.stamp.destination, dest);
            assert!(matches!(
                projection.route,
                crate::walk_map::RouteProjectionPath::Compact(_)
            ));
        });
    }

    fn wearable_item(id: i32) -> api::snapshot::ItemView {
        api::snapshot::ItemView {
            def: api::obj_names::ItemDefView {
                id,
                name: Some("fixture shield".into()),
                stackable: false,
                members: false,
                base_value: 0,
                noted: false,
                certificate_link: -1,
                certificate_template: -1,
            },
            container: api::snapshot::ItemContainer::Equipment,
            action_family: api::snapshot::ItemActionFamily::Held,
            slot: 5,
            count: 1,
            actions: Vec::new(),
            component_id: -1,
        }
    }

    fn post_wear_snapshot(from: WorldTile) -> GameSnapshot {
        let actor = api::snapshot::ActorView {
            name: Some("post-wear-fixture".into()),
            actions: Vec::new(),
            tile: from,
            distance: 0,
            animation: -1,
            animation_frame: 0,
            pose_animation: -1,
            orientation: 0,
            target_orientation: 0,
            overhead_text: None,
            spot_animation: -1,
            spot_animation_stamp: 0,
            health: 80,
            total_health: 80,
            face_entity: -1,
            target: None,
            moving: false,
            running: false,
            in_combat: false,
        };
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_local_player(api::snapshot::LocalPlayerView {
            player: api::snapshot::PlayerView {
                index: 0,
                network: from,
                actor,
                combat_level: 50,
                skill_level: 1,
                headicons: 0,
                weapon: None,
            },
            energy: 100,
            weight: 0,
            animation_update: None,
        });
        snapshot.seed_equipment(vec![wearable_item(2)]);
        snapshot.seed_inventory(Vec::new(), 28);
        snapshot.seed_stats(vec![api::snapshot::StatView {
            index: 3,
            name: "hitpoints".into(),
            effective: 70,
            base: 80,
            xp: 0,
            used: true,
        }]);
        snapshot.seed_world(api::snapshot::WorldStateView {
            map_base_x: 0,
            map_base_z: 0,
            level: from.level,
            members: false,
            multi_combat: false,
            player_count: 1,
            npc_count: 0,
            cycle: 1,
        });
        snapshot.seed_hitmarks(api::snapshot::HitmarksView {
            marks: [api::snapshot::HitmarkView {
                value: 0,
                kind: 0,
                cycle: 0,
            }; 4],
            loop_cycle: 0,
        });
        snapshot
    }

    fn pending_wear(
        final_route: crate::walk_plan::RoutePath,
        dest: WorldTile,
        id: i32,
    ) -> crate::PendingBankFetch {
        crate::PendingBankFetch {
            steps: std::collections::VecDeque::from([nav::bank_fetch::BankStep::Wear { id }]),
            dest,
            opts: FindOptions::default(),
            final_route,
            avoid: Vec::new(),
            progress: Default::default(),
        }
    }

    fn compact_bank_bot(
        final_route: crate::walk_plan::RoutePath,
        assessment: Arc<RouteAssessment>,
        admission: Admission,
        dest: WorldTile,
    ) -> crate::script_runtime::NavBot {
        crate::script_runtime::NavBot {
            bank_fetch: Some(pending_wear(final_route, dest, 2)),
            requested_route: Some((dest, 0, false, false, false, ZoneExempt::NONE)),
            walk_request_id: 7,
            route_generation: 3,
            admission: Some(Box::new(admission)),
            assessment: Some(assessment),
            admission_pending: true,
            ..Default::default()
        }
    }

    fn finish_carried_wear(
        bot: &mut crate::script_runtime::NavBot,
        snapshot: &GameSnapshot,
        world: &NavWorld,
        from: WorldTile,
    ) {
        let mut driver = IdleDriver::default();
        assert!(!crate::script_runtime::step_bank_fetch_on_bot(
            &mut driver,
            snapshot,
            bot,
            Some(world),
            Some((from.x, from.z, from.level)),
            false,
        ));
    }

    #[test]
    fn compact_carried_wear_final_route_reassesses_post_session_facts() {
        let world = Arc::new(empty_corridor());
        let from = tile(0, 1);
        let dest = tile(10, 1);
        let final_route = compact_route_path(&world, from, dest);
        let admission = make_admission(RiskPolicy::Proceed, input(80));
        let before = assess_route_path(&final_route, &world, &admission);
        assert!(!before.input.missing_facts);
        let mut snapshot = post_wear_snapshot(from);
        let mut food = wearable_item(379);
        food.container = api::snapshot::ItemContainer::Inventory;
        food.slot = 0;
        food.count = 2;
        snapshot.seed_inventory(vec![food], 28);
        let mut bot = compact_bank_bot(final_route, Arc::clone(&before), admission, dest);

        finish_carried_wear(&mut bot, &snapshot, &world, from);

        let after = bot.assessment.as_ref().expect("reassessed final route");
        assert!(!Arc::ptr_eq(&before, after));
        assert!(
            after.input.missing_facts,
            "post-wear facts must replace stale input"
        );
        assert_eq!(bot.admission.as_deref().unwrap().input, after.input);
        assert_eq!(
            after.input,
            capture(&snapshot, false, before.input.poison, false)
        );
        assert_eq!(after.input.hp, 70);
        assert!(after.input.food_ids.contains(&379));
        assert_eq!(after.input.food_counts.iter().copied().sum::<u8>(), 2);
        assert!(!bot.admission_pending);
        assert!(bot.route.is_none(), "the final route remains compact");
        assert_eq!(bot.route_destination(), Some(dest));
        assert!(bot.risk_refusal.is_none());
    }

    #[test]
    fn compact_carried_wear_final_route_refuses_unknown_post_session_facts() {
        let world = Arc::new(known_corridor(&[5]));
        let from = tile(0, 1);
        let dest = tile(10, 1);
        let final_route = compact_route_path(&world, from, dest);
        let admission = make_admission(RiskPolicy::Inherit, input(80));
        let before = assess_route_path(&final_route, &world, &admission);
        assert!(!before.input.missing_facts);
        let snapshot = post_wear_snapshot(from);
        let mut bot = compact_bank_bot(final_route, Arc::clone(&before), admission, dest);

        finish_carried_wear(&mut bot, &snapshot, &world, from);

        let after = bot.assessment.as_ref().expect("reassessed final route");
        assert!(!Arc::ptr_eq(&before, after));
        assert_eq!(
            after.input,
            capture(&snapshot, false, before.input.poison, false)
        );
        assert!(
            after.input.missing_facts,
            "incomplete post-wear facts fail closed"
        );
        assert!(!bot.admission_pending);
        assert!(bot.route.is_none(), "refusal clears the compact route");
        assert_eq!(
            bot.risk_refusal.as_deref().map(|(_, refusal)| *refusal),
            Some(WalkRefusal::Unknown(UnknownWhy::MissingFacts))
        );
        assert_eq!(bot.route_destination(), None);
    }
}
