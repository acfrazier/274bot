use super::replay::{admission_passes, candidate_cost, Estimate, Grants};
use super::*;
use crate::combat::tables::CombatTables;
use crate::native::WalkAllow;
use api::selected::ClientRevision;
use api::WorldTile;
use nav::router::{Leg, Route};
use nav::transport::{TransportEdge, TransportKind, WildernessRules, WildernessZone};
use nav::zones::{Zone, ZoneClass, ZoneExempt, ZoneKey, ZoneKind, ZoneTable};
use std::sync::Arc;
fn tile(x: i32) -> WorldTile {
    WorldTile {
        x,
        z: 100,
        level: 0,
    }
}
fn tables() -> Arc<CombatTables> {
    CombatTables::build(api::game_data::for_revision(ClientRevision::R289).unwrap()).unwrap()
}
fn input(hp: u8, cap: u8, food: u8, tables: &CombatTables) -> RiskInput {
    let mut value = RiskInput {
        hp,
        hp_max: cap,
        prayer: 1,
        prayer_base: 1,
        combat: Some(1),
        pos: tile(80),
        poison: PoisonState::Clear,
        missing_facts: false,
        input_locked: false,
        unattributed: false,
        overflow: false,
        food_len: 1,
        free_slots: 28 - food,
        ..RiskInput::default()
    };
    value.food_ids[0] = tables.selected().item_by_alias("lobster").unwrap().id;
    value.food_counts[0] = food;
    value
}
fn route(first: i32, last: i32) -> Route {
    Route {
        legs: vec![Leg::Walk {
            tiles: (first..=last).map(tile).collect(),
        }],
        dest: tile(last),
        ticks: f64::from(last - first) / 2.0,
    }
}
fn zone_table(
    zones: Vec<Zone>,
    kinds: Vec<ZoneKind>,
    carves: Vec<(u16, nav::router::AvoidRect)>,
    shapes: Vec<u64>,
    wilderness: &WildernessRules,
) -> ZoneTable {
    ZoneTable::from_parts(
        zones,
        kinds,
        vec![],
        carves,
        shapes,
        WorldTile {
            x: 0,
            z: 0,
            level: 0,
        },
        4096,
        4096,
        wilderness,
    )
    .unwrap()
}
fn known_kind(tables: &CombatTables) -> ZoneKind {
    let npc = tables.selected().npc_by_config("icewarrior").unwrap();
    ZoneKind::new("ice", "Ice warrior", npc.id, 57, false, false)
}
fn one_zone(tables: &CombatTables, spawn: i32, radius: u8) -> ZoneTable {
    zone_table(
        vec![Zone::npc(
            tile(spawn),
            radius,
            ZoneClass::Always,
            u16::MAX,
            0,
        )],
        vec![known_kind(tables)],
        vec![],
        vec![],
        &WildernessRules::default(),
    )
}
/// Effective one-attacker fixture for the S3 hand-worked ice examples. This
/// tests S2a replay arithmetic, not S3's unimplemented eligibility discount.
fn corridor_plan(tables: &CombatTables, prefix: u16) -> RoutePlan {
    let ident = known_kind(tables).npc_id;
    RoutePlan {
        complete: true,
        intervals: vec![ZoneInterval::new(
            0,
            ident,
            tile(120),
            prefix,
            prefix + 25,
            prefix,
            prefix + 25,
            13,
            1,
            6,
            4,
            Style::Melee,
            false,
            false,
            false,
        )
        .unwrap()]
        .into_boxed_slice(),
        crossings: vec![CrossingGeom {
            first: prefix,
            last: prefix + 25,
            env_first: prefix,
            env_last: prefix + 25,
            intervals: (0, 1),
            retreat: prefix - 1,
            forward: prefix + 26,
            leg: 0,
        }]
        .into_boxed_slice(),
    }
}
fn context<'a>(
    zones: &'a ZoneTable,
    risk: &'a RiskTables,
    tables: &'a CombatTables,
    wilderness: &'a WildernessRules,
) -> Estimate<'a> {
    Estimate {
        zones: Some(zones),
        risks: risk,
        combat: tables,
        wilderness,
        allow: WalkAllow::default(),
        generation: 7,
    }
}

fn wizard_kind(tables: &CombatTables) -> ZoneKind {
    let npc = tables.selected().npc_by_config("wizard").unwrap();
    ZoneKind::new("wizard", "Wizard", npc.id, 1, npc.ap_attack, false)
}

fn synthetic_zone_table(kinds: Vec<ZoneKind>) -> ZoneTable {
    let mut definitions: Vec<ZoneKind> = Vec::new();
    let mut zones = Vec::with_capacity(kinds.len());
    for (index, kind) in kinds.into_iter().enumerate() {
        let kind_index = definitions
            .iter()
            .position(|definition| definition.npc_id == kind.npc_id && definition.id == kind.id)
            .unwrap_or_else(|| {
                let index = definitions.len();
                definitions.push(kind);
                index
            });
        zones.push(Zone::npc(
            tile(10 + index as i32 * 20),
            1,
            ZoneClass::Always,
            u16::MAX,
            kind_index as u16,
        ));
    }
    zone_table(
        zones,
        definitions,
        vec![],
        vec![],
        &WildernessRules::default(),
    )
}

fn synthetic_interval(
    zone: u16,
    ident: i32,
    spawn: WorldTile,
    (first, last): (u16, u16),
    max_hit: u8,
    rate: u8,
    unknown: bool,
) -> ZoneInterval {
    ZoneInterval::new(
        zone,
        ident,
        spawn,
        first,
        last,
        first,
        last,
        6,
        1,
        max_hit,
        rate,
        if unknown {
            Style::Unknown
        } else {
            Style::Melee
        },
        unknown,
        false,
        false,
    )
    .unwrap()
}

fn synthetic_crossing(
    first: u16,
    last: u16,
    intervals: (u16, u16),
    has_way_out: bool,
) -> CrossingGeom {
    CrossingGeom {
        first,
        last,
        env_first: first,
        env_last: last,
        intervals,
        retreat: if has_way_out {
            first.checked_sub(1).unwrap_or(CrossingGeom::NONE)
        } else {
            CrossingGeom::NONE
        },
        forward: if has_way_out {
            last.checked_add(1).unwrap_or(CrossingGeom::NONE)
        } else {
            CrossingGeom::NONE
        },
        leg: 0,
    }
}

fn synthetic_assessment(
    input: RiskInput,
    intervals: Vec<ZoneInterval>,
    plan_crossings: Vec<CrossingGeom>,
    displayed: usize,
    verdict: Verdict,
) -> RouteAssessment {
    let more = u8::try_from(plan_crossings.len().saturating_sub(displayed)).unwrap();
    let crossings = (0..displayed)
        .map(|index| Crossing {
            key: ZoneKey::Zone(index as u16),
            first: 0,
            last: 0,
            ticks: 0,
            worst: 0,
            volley: 0,
            max_hit: 0,
            rate: 0,
            style: Style::Melee,
            floor_deep: 0,
            bites: 0,
            single: false,
            protect_credited: false,
            unknown: false,
        })
        .collect::<Vec<_>>()
        .into_boxed_slice();
    RouteAssessment {
        verdict,
        plan: RoutePlan {
            complete: true,
            intervals: intervals.into_boxed_slice(),
            crossings: plan_crossings.into_boxed_slice(),
        },
        crossings,
        more,
        supplies: Box::new([]),
        hp_after: input.hp,
        volley: 0,
        input,
        generation: 7,
        reason: Arc::from("synthetic admission assessment"),
    }
}

#[test]
fn u2_admission_omits_fully_granted_unknown_physics_and_preserves_assessment() {
    let tables = tables();
    let wizard = wizard_kind(&tables);
    let known = known_kind(&tables);
    let wizard_id = wizard.npc_id;
    let known_id = known.npc_id;
    let zones = synthetic_zone_table(vec![wizard, known]);
    let walk = route(0, 40);
    let value = input(20, 20, 0, &tables);
    let assessment = synthetic_assessment(
        value,
        vec![
            synthetic_interval(0, wizard_id, tile(5), (5, 5), 6, 0, true),
            synthetic_interval(1, known_id, tile(20), (20, 20), 6, 4, false),
        ],
        vec![
            synthetic_crossing(5, 5, (0, 1), true),
            synthetic_crossing(20, 20, (1, 2), true),
        ],
        2,
        Verdict::Unknown(UnknownWhy::Kind(0)),
    );
    let grants = ZoneExempt::named(&[ZoneKey::Zone(0)]).unwrap();

    assert_eq!(
        admission_passes(
            &walk,
            &assessment,
            &zones,
            &tables,
            WalkAllow::default(),
            Grants::named(grants)
        ),
        Ok(true)
    );
    assert_eq!(assessment.plan.intervals.len(), 2);
    assert!(assessment.plan.intervals[0].unknown());
    assert_eq!(assessment.plan.intervals[0].rate, 0);
    assert_eq!(assessment.plan.crossings[0].intervals, (0, 1));
}

#[test]
fn u2_admission_rejects_a_mixed_granted_unknown_crossing() {
    let tables = tables();
    let wizard = wizard_kind(&tables);
    let known = known_kind(&tables);
    let wizard_id = wizard.npc_id;
    let known_id = known.npc_id;
    let zones = synthetic_zone_table(vec![wizard, known]);
    let walk = route(0, 20);
    let assessment = synthetic_assessment(
        input(90, 90, 0, &tables),
        vec![
            synthetic_interval(0, wizard_id, tile(5), (5, 5), 6, 0, true),
            synthetic_interval(1, known_id, tile(5), (5, 5), 6, 4, false),
        ],
        vec![synthetic_crossing(5, 5, (0, 2), true)],
        1,
        Verdict::Unknown(UnknownWhy::Kind(0)),
    );
    let grants = ZoneExempt::named(&[ZoneKey::Zone(0)]).unwrap();

    assert_eq!(
        admission_passes(
            &walk,
            &assessment,
            &zones,
            &tables,
            WalkAllow::default(),
            Grants::named(grants)
        ),
        Err(UnknownWhy::Kind(0))
    );
}

#[test]
fn u2_admission_keeps_known_granted_damage_before_ungranted_crossings() {
    let tables = tables();
    let known_id = known_kind(&tables).npc_id;
    let zones = synthetic_zone_table(vec![known_kind(&tables), known_kind(&tables)]);
    let walk = route(0, 40);
    let later = synthetic_interval(1, known_id, tile(20), (20, 20), 6, 4, false);
    let later_only = synthetic_assessment(
        input(20, 20, 0, &tables),
        vec![later],
        vec![synthetic_crossing(20, 20, (0, 1), true)],
        1,
        Verdict::Survivable,
    );
    assert_eq!(
        admission_passes(
            &walk,
            &later_only,
            &zones,
            &tables,
            WalkAllow::default(),
            Grants::named(ZoneExempt::NONE),
        ),
        Ok(true),
        "the later known crossing is survivable without prior damage"
    );

    let assessment = synthetic_assessment(
        input(20, 20, 0, &tables),
        vec![
            synthetic_interval(0, known_id, tile(5), (5, 5), 6, 4, false),
            later,
        ],
        vec![
            synthetic_crossing(5, 5, (0, 1), true),
            synthetic_crossing(20, 20, (1, 2), true),
        ],
        2,
        Verdict::Unsurvivable,
    );
    let grants = ZoneExempt::named(&[ZoneKey::Zone(0)]).unwrap();

    assert_eq!(
        admission_passes(
            &walk,
            &assessment,
            &zones,
            &tables,
            WalkAllow::default(),
            Grants::named(grants)
        ),
        Ok(false),
        "the granted first crossing's landed hit remains in the later floor"
    );
    assert_eq!(assessment.plan.intervals[0].max_hit, 6);
}

#[test]
fn u2_admission_checks_ninth_crossing_beyond_display_bound() {
    let tables = tables();
    let known_id = known_kind(&tables).npc_id;
    let zones = synthetic_zone_table((0..9).map(|_| known_kind(&tables)).collect());
    let walk = route(0, 100);
    let intervals = (0..9)
        .map(|zone| {
            let index = 2 + zone * 10;
            synthetic_interval(
                zone,
                known_id,
                tile(10 + i32::from(zone)),
                (index, index),
                6,
                4,
                false,
            )
        })
        .collect::<Vec<_>>();
    let plan_crossings = (0..9)
        .map(|zone| {
            let index = 2 + zone * 10;
            synthetic_crossing(index, index, (zone, zone + 1), true)
        })
        .collect::<Vec<_>>();
    let assessment = synthetic_assessment(
        input(50, 50, 0, &tables),
        intervals,
        plan_crossings,
        8,
        Verdict::Unsurvivable,
    );
    let keys = (0..8).map(ZoneKey::Zone).collect::<Vec<_>>();
    let grants = ZoneExempt::named(&keys).unwrap();

    assert_eq!(assessment.plan.crossings.len(), 9);
    assert_eq!(assessment.crossings.len(), 8);
    assert_eq!(assessment.more, 1);
    assert_eq!(
        admission_passes(
            &walk,
            &assessment,
            &zones,
            &tables,
            WalkAllow::default(),
            Grants::named(grants)
        ),
        Ok(false),
        "the ninth ungranted crossing still participates in complete-plan replay"
    );
}

#[test]
fn u2_admission_grants_a_fully_granted_no_way_out_crossing() {
    let tables = tables();
    let known = known_kind(&tables);
    let zones = synthetic_zone_table(vec![known]);
    let walk = route(0, 20);
    let assessment = synthetic_assessment(
        input(90, 90, 0, &tables),
        vec![synthetic_interval(
            0,
            zones.kinds()[0].npc_id,
            tile(10),
            (0, 20),
            6,
            4,
            false,
        )],
        vec![synthetic_crossing(0, 20, (0, 1), false)],
        1,
        Verdict::Unsurvivable,
    );
    let grants = ZoneExempt::named(&[ZoneKey::Zone(0)]).unwrap();

    assert_eq!(
        admission_passes(
            &walk,
            &assessment,
            &zones,
            &tables,
            WalkAllow::default(),
            Grants::named(grants)
        ),
        Ok(true)
    );
}

#[test]
fn u2_admission_scopes_unknown_poison_and_exempts_safe_only_walks() {
    let tables = tables();
    let known_id = known_kind(&tables).npc_id;
    let zones = synthetic_zone_table(vec![known_kind(&tables), known_kind(&tables)]);
    let walk = route(0, 40);
    let mut value = input(90, 90, 0, &tables);
    value.poison = PoisonState::Unknown { since: 0 };
    let assessment = synthetic_assessment(
        value,
        vec![
            synthetic_interval(0, known_id, tile(5), (5, 5), 6, 4, false),
            synthetic_interval(1, known_id, tile(20), (20, 20), 6, 4, false),
        ],
        vec![
            synthetic_crossing(5, 5, (0, 1), true),
            synthetic_crossing(20, 20, (1, 2), true),
        ],
        2,
        Verdict::Unknown(UnknownWhy::Poison),
    );
    let first_granted = ZoneExempt::named(&[ZoneKey::Zone(0)]).unwrap();
    assert_eq!(
        admission_passes(
            &walk,
            &assessment,
            &zones,
            &tables,
            WalkAllow::default(),
            Grants::named(first_granted),
        ),
        Err(UnknownWhy::Poison)
    );
    assert_eq!(
        admission_passes(
            &walk,
            &assessment,
            &zones,
            &tables,
            WalkAllow::default(),
            Grants::named(ZoneExempt::all()),
        ),
        Ok(true)
    );

    let safe = route(0, 20);
    let safe_only = synthetic_assessment(value, vec![], vec![], 0, Verdict::Survivable);
    assert_eq!(
        admission_passes(
            &safe,
            &safe_only,
            &zones,
            &tables,
            WalkAllow::default(),
            Grants::named(ZoneExempt::NONE),
        ),
        Ok(true)
    );
}

/// SURVIVABLE-DEST-GRANTS: a named-grant request also grants the zones
/// engaged at its route's endpoint. Without names, past the endpoint, or for
/// a zone inactive at the assessed combat level, the crossing is judged.
#[test]
fn u2_named_request_grants_zones_engaged_at_the_route_endpoint() {
    let tables = tables();
    let known_id = known_kind(&tables).npc_id;
    let wilderness = WildernessRules::default();
    // The ice warrior's level rule caps at 2 × 57 = 114.
    let zones = zone_table(
        vec![
            Zone::npc(tile(10), 1, ZoneClass::Always, u16::MAX, 0),
            Zone::npc(tile(30), 1, ZoneClass::LevelRule, 114, 0),
        ],
        vec![known_kind(&tables)],
        vec![],
        vec![],
        &wilderness,
    );
    let assessment = |combat| {
        let mut value = input(90, 90, 0, &tables);
        value.poison = PoisonState::Unknown { since: 0 };
        value.combat = Some(combat);
        synthetic_assessment(
            value,
            vec![
                synthetic_interval(0, known_id, tile(10), (10, 10), 6, 4, false),
                synthetic_interval(1, known_id, tile(30), (30, 30), 6, 4, false),
            ],
            vec![
                synthetic_crossing(10, 10, (0, 1), true),
                synthetic_crossing(30, 30, (1, 2), true),
            ],
            2,
            Verdict::Unknown(UnknownWhy::Poison),
        )
    };
    let named = ZoneExempt::named(&[ZoneKey::Zone(0)]).unwrap();
    let passes = |combat, walk: &Route, grants| {
        admission_passes(
            walk,
            &assessment(combat),
            &zones,
            &tables,
            WalkAllow::default(),
            grants,
        )
    };
    let ends_inside = route(0, 30);
    assert_eq!(
        passes(40, &ends_inside, Grants::request(named, &wilderness)),
        Ok(true)
    );
    assert_eq!(
        passes(40, &ends_inside, Grants::named(named)),
        Err(UnknownWhy::Poison),
        "without the endpoint grant the destination crossing is judged"
    );
    assert_eq!(
        passes(
            40,
            &ends_inside,
            Grants::request(ZoneExempt::NONE, &wilderness)
        ),
        Err(UnknownWhy::Poison),
        "a request without named grants keeps its endpoint crossing judged"
    );
    assert_eq!(
        passes(40, &route(0, 40), Grants::request(named, &wilderness)),
        Err(UnknownWhy::Poison),
        "a zone crossed on the way, not at the endpoint, is judged"
    );
    assert_eq!(
        passes(120, &ends_inside, Grants::request(named, &wilderness)),
        Err(UnknownWhy::Poison),
        "a zone inactive at the endpoint for the assessed level is not granted"
    );
}

pub(super) fn assess_poison_state(poison: PoisonState, crossing: bool) -> Arc<RouteAssessment> {
    let tables = tables();
    let zones = one_zone(&tables, 100, 1);
    let risks = RiskTables::build(&zones, &tables);
    let wilderness = WildernessRules::default();
    let mut value = input(90, 90, 0, &tables);
    value.poison = poison;
    let walk = if crossing {
        route(88, 112)
    } else {
        route(10, 20)
    };
    assess(&walk, &context(&zones, &risks, &tables, &wilderness), value)
}

#[test]
fn u9_compact_layout_and_limit_round_trips() {
    assert!(std::mem::size_of::<RiskInput>() <= 128);
    assert!(std::mem::size_of::<PoisonState>() <= 8);
    assert!(std::mem::size_of::<KindRisk>() <= 8);
    assert!(std::mem::size_of::<ZoneInterval>() <= 24);
    assert!(std::mem::size_of::<Crossing>() <= 32);
    for x in [0, 16383] {
        for z in [0, 16383] {
            for level in [0, 3] {
                let point = WorldTile { x, z, level };
                assert_eq!(PackedTile::new(point).unwrap().tile(), point);
                for ident in [-1, i32::from(i16::MAX)] {
                    let row = ZoneInterval::new(
                        u16::MAX,
                        ident,
                        point,
                        u16::MAX,
                        u16::MAX,
                        u16::MAX,
                        u16::MAX,
                        255,
                        255,
                        255,
                        255,
                        Style::Unknown,
                        true,
                        true,
                        true,
                    )
                    .unwrap();
                    assert_eq!(row.ident(), ident);
                    assert_eq!(row.a, u16::MAX);
                    assert_eq!(row.spawn.tile(), point);
                }
            }
        }
    }
    for point in [
        WorldTile {
            x: -1,
            z: 0,
            level: 0,
        },
        WorldTile {
            x: 16384,
            z: 0,
            level: 0,
        },
        WorldTile {
            x: 0,
            z: 16384,
            level: 0,
        },
        WorldTile {
            x: 0,
            z: 0,
            level: 4,
        },
    ] {
        assert_eq!(PackedTile::new(point), Err(UnknownWhy::Overflow));
    }
    assert_eq!(
        ZoneInterval::new(
            0,
            32768,
            tile(1),
            0,
            0,
            0,
            0,
            1,
            1,
            1,
            1,
            Style::Melee,
            false,
            false,
            false
        ),
        Err(UnknownWhy::Overflow)
    );
    assert_eq!(
        ZoneInterval::new(
            0,
            -2,
            tile(1),
            0,
            0,
            0,
            0,
            1,
            1,
            1,
            1,
            Style::Melee,
            false,
            false,
            false
        ),
        Err(UnknownWhy::Overflow)
    );
    assert_eq!(
        ZoneInterval::new(
            0,
            1,
            tile(1),
            2,
            1,
            0,
            3,
            1,
            1,
            1,
            1,
            Style::Melee,
            false,
            false,
            false
        ),
        Err(UnknownWhy::Overflow)
    );
}

#[test]
fn r1_m3_scene_counts_cannot_reduce_the_uncapped_union() {
    for (live, estimate, nearby) in [(1, 2, 1), (4, 300, 255), (1, 1, 0), (2, 3, 3)] {
        assert!(
            live + estimate > nearby,
            "nearby count is incomplete evidence"
        );
        assert_eq!(attacker_count(live, estimate).unwrap(), live + estimate);
    }
    assert_eq!(attacker_count(u16::MAX, 1), Err(UnknownWhy::Overflow));
}

#[test]
fn u2_unknown_classes_fail_closed_through_the_real_assessment() {
    let tables = tables();
    let wilderness = WildernessRules::default();
    for (config, members, class) in [
        ("barbarian", false, UnknownKind::Bespoke),
        ("red_dragon", false, UnknownKind::Dragonfire),
        ("trail_hard2", false, UnknownKind::CounterProtect),
        ("ikov_firewarrior", false, UnknownKind::Magic),
        ("wizard", false, UnknownKind::Mixed),
        ("man", false, UnknownKind::MissingStat),
        ("poisonspider", true, UnknownKind::Poison),
    ] {
        let npc = tables.selected().npc_by_config(config).unwrap();
        let zones = zone_table(
            vec![Zone::npc(tile(100), 2, ZoneClass::Always, u16::MAX, 0)],
            vec![ZoneKind::new(
                config,
                config,
                npc.id,
                1,
                npc.ap_attack,
                false,
            )],
            vec![],
            vec![],
            &wilderness,
        );
        let risks = RiskTables::build(&zones, &tables);
        let mut value = input(90, 90, 10, &tables);
        value.map_members = members;
        let assessment = assess(
            &route(88, 112),
            &context(&zones, &risks, &tables, &wilderness),
            value,
        );
        assert_eq!(
            assessment.verdict,
            Verdict::Unknown(UnknownWhy::Kind(0)),
            "{config}"
        );
        assert!(assessment.reason.contains(config), "{config}");
        assert!(
            assessment.reason.contains(&format!("{class:?}")),
            "{config}"
        );
    }
}

#[test]
fn u3_c4_real_assessment_sums_live_and_estimated_rows_without_scene_coverage() {
    let tables = tables();
    let wilderness = WildernessRules::default();
    // Scene readiness/count is deliberately not an assessment input. These
    // cases encode beyond-view, count-limited, absent-retained and disjoint
    // scenarios at the real assessment seam, not just n(k) arithmetic.
    for (case, live, estimated) in [
        ("horizon beyond distance 15", 1, 2),
        ("more than 255 nearby NPCs", 4, 300),
        ("retained row absent from current view", 1, 1),
        ("disjoint live and estimated actors", 2, 3),
    ] {
        let zones = zone_table(
            (0..estimated)
                .map(|index| {
                    let spawn = if estimated > 255 {
                        WorldTile {
                            x: 100 + (index % 20) as i32,
                            z: 93 + (index / 20) as i32,
                            level: 0,
                        }
                    } else {
                        tile(100 + index as i32)
                    };
                    Zone::npc(
                        spawn,
                        if estimated > 255 { 8 } else { 1 },
                        ZoneClass::Always,
                        u16::MAX,
                        0,
                    )
                })
                .collect(),
            vec![known_kind(&tables)],
            vec![],
            vec![],
            &wilderness,
        );
        let mut risks = RiskTables::build(&zones, &tables);
        if estimated > 255 {
            // A synthetic broad tether makes all 300 distinct spawns persist
            // beyond the 255-row view, without invalid acquisition geometry.
            risks.kinds[0].r = 64;
        }
        let mut value = input(255, 255, 0, &tables);
        value.free_slots = 0;
        value.live_len = live;
        for (index, row) in value.live.iter_mut().take(usize::from(live)).enumerate() {
            *row = LiveRow {
                actor: crate::combat::ActorRef {
                    kind: crate::combat::ActorKind::Npc,
                    index: 1000 + index as u16,
                },
                ident: known_kind(&tables).npc_id,
                max_hit: 6,
                rate: 4,
                due_tick: u16::MAX,
            };
        }
        let assessment = assess(
            // Start outside the synthetic 64-tile tether so this remains
            // an entering-overflow probe, not an origin-escape admission.
            &route(if estimated > 255 { 10 } else { 80 }, 120),
            &context(&zones, &risks, &tables, &wilderness),
            value,
        );
        if estimated > 255 {
            assert_eq!(
                assessment.verdict,
                Verdict::Unknown(UnknownWhy::Overflow),
                "{case}"
            );
        } else {
            assert_eq!(assessment.plan.intervals.len(), estimated, "{case}");
            assert_eq!(assessment.volley, 6 * (live + estimated as u8), "{case}");
            assert_eq!(
                assessment.crossings[0].worst,
                24 * u16::from(live) + 18 * estimated as u16,
                "{case}: every live row and every spawn contributes"
            );
        }
    }
}

#[test]
fn u3_floor_staircase_and_mixed_rate_uncapped_damage() {
    let tables = tables();
    let route = route(80, 108);
    let path = RoutePath::new(&route).unwrap();
    let plan = corridor_plan(&tables, 1);
    let input = input(40, 40, 0, &tables);
    for (j, expected) in [
        (0, 18),
        (3, 18),
        (4, 24),
        (7, 24),
        (8, 30),
        (15, 30),
        (16, 24),
        (19, 24),
        (20, 18),
        (23, 18),
        (24, 12),
        (25, 12),
    ] {
        assert_eq!(
            estimated_floor(path, &plan, j + 1, &input, &tables, WalkAllow::default()).unwrap(),
            Some(expected),
            "j={j}"
        );
    }
    let mut mixed = plan;
    let original = mixed.intervals[0];
    mixed.intervals = [1, 4, 5, 8]
        .into_iter()
        .map(|rate| ZoneInterval { rate, ..original })
        .collect();
    mixed.crossings[0].intervals = (0, 4);
    // Midpoint distance=13, latency=2; independently sum 6*ceil(15/r).
    assert_eq!(
        candidate_cost(path, &mixed, 13, false).unwrap(),
        Some(6 * (15 + 4 + 3 + 2))
    );
}

/// An independent straight-line content simulation. No production cost,
/// line, food, clock or replay helper is used. The hand-worked ice lines are
/// the design's external oracle, and foods apply at delayed server phases.
fn simulate_ice(
    start_hp: i32,
    cap: i32,
    lobsters: u8,
    prefix: u16,
    shrimp: u8,
) -> (bool, i32, u16, Vec<i32>) {
    let mut hp = start_hp;
    let mut left = [lobsters, shrimp];
    let heals = [12, 3];
    let mut bites = 0;
    let mut due: Option<(i32, i32)> = None;
    let mut ready = 0;
    let mut clicks = Vec::new();
    let crossing_start = (i32::from(prefix) * 3 + 1) / 2;
    for t in 0..crossing_start + 39 {
        let j = if t >= crossing_start {
            ((t - crossing_start) * 2 / 3) as usize
        } else {
            usize::MAX
        };
        let floor = if j < 26 {
            [
                18, 18, 18, 18, 24, 24, 24, 24, 30, 30, 30, 30, 30, 30, 30, 30, 24, 24, 24, 24, 18,
                18, 18, 18, 12, 12,
            ][j]
        } else {
            0
        };
        if t == crossing_start && hp <= floor {
            return (false, hp, bites, clicks);
        }
        if j < 26 && (t - crossing_start) % 4 == 0 {
            hp -= 6;
        }
        if hp <= 0 || hp <= floor && j < 26 {
            return (false, hp, bites, clicks);
        }
        if let Some((at, heal)) = due {
            if at == t {
                hp = (hp + heal).min(cap);
                due = None;
            }
        }
        let line = if j < 26 {
            [
                30, 30, 36, 36, 36, 36, 36, 36, 36, 36, 36, 36, 36, 36, 36, 30, 30, 30, 30, 24, 24,
                24, 24, 18, 18, 13,
            ][j]
        } else {
            0
        };
        let smallest = (0..2).filter(|i| left[*i] > 0).min_by_key(|i| heals[*i]);
        let topup = t < crossing_start
            && crossing_start - t <= 30
            && smallest.is_some_and(|i| hp + heals[i] <= cap);
        if due.is_none() && t >= ready && (topup || hp <= line) {
            let fitting = (0..2)
                .filter(|i| left[*i] > 0 && hp + heals[*i] <= cap)
                .max_by_key(|i| heals[*i]);
            if let Some(food) = fitting.or(smallest) {
                left[food] -= 1;
                bites += 1;
                due = Some((t + 2, heals[food]));
                ready = t + 3;
                clicks.push(t);
            }
        }
    }
    (true, hp, bites, clicks)
}
#[test]
fn u4_u5_c6_independent_ice_replay_and_exact_food_counts() {
    let tables = tables();
    let route = route(80, 127);
    let path = RoutePath::new(&route).unwrap();
    let plan = corridor_plan(&tables, 20);
    for (hp, cap, food, pass, after) in [
        (40, 40, 0, false, 22),
        (40, 40, 4, false, 22),
        (40, 40, 5, true, 16),
        (45, 45, 6, true, 21),
        (20, 40, 0, false, 14),
        (20, 40, 6, false, 22),
        (20, 40, 7, true, 16),
    ] {
        let input = input(hp, cap, food, &tables);
        let mut clicks = Vec::new();
        let actual = replay(
            path,
            &plan,
            &input,
            &tables,
            WalkAllow::default(),
            None,
            |tick, _, _, _, bite| {
                if bite.is_some() {
                    clicks.push(tick);
                }
            },
        )
        .unwrap();
        let independent = simulate_ice(i32::from(hp), i32::from(cap), food, 20, 0);
        assert_eq!(
            (actual.passed, actual.hp_after, actual.bites),
            (independent.0, independent.1, independent.2),
            "hp={hp},food={food}"
        );
        assert_eq!(clicks, independent.3, "hp={hp},food={food}");
        assert_eq!((actual.passed, actual.hp_after), (pass, after));
        if !pass {
            let needed = if hp == 20 {
                if food == 0 {
                    7
                } else {
                    1
                }
            } else if food == 0 {
                5
            } else {
                1
            };
            for extra in 1..=needed {
                let fixed = replay(
                    path,
                    &plan,
                    &input,
                    &tables,
                    WalkAllow::default(),
                    Some((input.food_ids[0], extra)),
                    |_, _, _, _, _| {},
                )
                .unwrap();
                assert_eq!(fixed.passed, extra == needed, "least additional count");
            }
        }
    }
}

#[test]
fn u4_prayer_credit_all_conditions_and_one_raw_volley() {
    let tables = tables();
    let zones = one_zone(&tables, 100, 1);
    let risk = RiskTables::build(&zones, &tables);
    let w = WildernessRules::default();
    let route = route(88, 112);
    let cx = context(&zones, &risk, &tables, &w);
    let mut good = input(90, 90, 0, &tables);
    good.prayer_base = 99;
    good.prayer = 99;
    let protected = assess(&route, &cx, good);
    assert!(protected.crossings[0].protect_credited);
    assert_eq!(protected.hp_after, 84);
    for which in 0..5 {
        let mut changed = good;
        let mut allow = WalkAllow::default();
        match which {
            0 => allow.prayer = false,
            1 => changed.prayer_base = 1,
            2 => changed.prayer = 1,
            3 => changed.other_prayers_on = true,
            _ => changed.off_debt = true,
        }
        let c = Estimate { allow, ..cx };
        let actual = assess(&route, &c, changed);
        assert!(!actual.crossings[0].protect_credited, "condition {which}");
    }
    let required = (RoutePath::new(&route).unwrap().ticks()
        - RoutePath::new(&route)
            .unwrap()
            .point(protected.plan.crossings[0].env_first)
            .unwrap()
            .tick
        + 8)
        / 5
        + 1;
    good.prayer = (required - 1) as u8;
    good.doses_prayer = 20;
    assert!(
        !assess(&route, &cx, good).crossings[0].protect_credited,
        "potions never substitute for a missing current point"
    );
}

#[test]
fn u4_u14_poison_through_safe_gap_and_no_crossing_exemption() {
    let tables = tables();
    let zones = one_zone(&tables, 120, 1);
    let risk = RiskTables::build(&zones, &tables);
    let w = WildernessRules::default();
    let cx = context(&zones, &risk, &tables, &w);
    let route = route(80, 145);
    let path = RoutePath::new(&route).unwrap();
    let plan = build_plan(path, &zones, &risk, &w, &input(90, 90, 0, &tables)).unwrap();
    let mut poisoned = input(90, 90, 0, &tables);
    poisoned.poison = PoisonState::Poisoned {
        per_tick: 1,
        last_tick: 0,
    };
    let mut ticks = Vec::new();
    replay(
        path,
        &plan,
        &poisoned,
        &tables,
        WalkAllow::default(),
        None,
        |tick, _, _, damage, _| {
            if damage > 0 && tick % 30 == 0 {
                ticks.push(tick);
            }
        },
    )
    .unwrap();
    assert!(ticks.contains(&0));
    assert!(ticks.contains(&30));
    assert!(ticks.contains(&90));
    let safe = self::route(10, 20);
    let mut unknown = poisoned;
    unknown.poison = PoisonState::Unknown { since: 0 };
    assert_eq!(assess(&safe, &cx, unknown).verdict, Verdict::Survivable);
    assert_eq!(
        assess(&route, &cx, unknown).verdict,
        Verdict::Unknown(UnknownWhy::Poison)
    );
    assert!(assess(&route, &cx, unknown)
        .reason
        .contains("separate safe-only walk"));
    assert_eq!(assess(&safe, &cx, poisoned).input.poison, poisoned.poison);
}

#[test]
fn u4_u14_no_crossing_poison_is_not_a_refusal_or_a_fetchable_food_fix() {
    let tables = tables();
    let zones = one_zone(&tables, 120, 1);
    let risks = RiskTables::build(&zones, &tables);
    let wilderness = WildernessRules::default();
    let cx = context(&zones, &risks, &tables, &wilderness);
    let safe = route(10, 30);
    let mut poisoned = input(10, 40, 0, &tables);
    poisoned.poison = PoisonState::Poisoned {
        per_tick: 11,
        last_tick: 0,
    };
    let assessment = assess(&safe, &cx, poisoned);
    assert!(assessment.plan.crossings.is_empty());
    assert_eq!(assessment.verdict, Verdict::Survivable);
    assert_eq!(
        assessment.hp_after, 0,
        "retain the conservative poison replay"
    );
    assert_eq!(assessment.input.poison, poisoned.poison);
    assert!(assessment
        .supplies
        .contains(&SupplyNeed::Info(InfoNeed::Antipoison)));
    assert!(!assessment
        .supplies
        .iter()
        .any(|need| matches!(need, SupplyNeed::Food { .. })));
}

#[test]
fn u3_c4_origin_rows_and_pending_damage_survive_an_empty_plan_endpoint() {
    let tables = tables();
    let zones = one_zone(&tables, 120, 1);
    let risks = RiskTables::build(&zones, &tables);
    let wilderness = WildernessRules::default();
    let cx = context(&zones, &risks, &tables, &wilderness);
    let short = route(10, 10);
    let mut value = input(90, 90, 0, &tables);
    value.pos = tile(10);
    value.prayer = 99;
    value.prayer_base = 99;
    value.live_len = 1;
    value.live[0] = LiveRow {
        actor: crate::combat::ActorRef {
            kind: crate::combat::ActorKind::Npc,
            index: 7,
        },
        ident: known_kind(&tables).npc_id,
        max_hit: 6,
        rate: 4,
        due_tick: 4,
    };
    let assessment = assess(&short, &cx, value);
    assert!(assessment.plan.crossings.is_empty());
    assert_eq!(
        (assessment.verdict, assessment.hp_after, assessment.volley),
        (Verdict::Survivable, 60, 6),
        "four raw origin volleys plus the retained launch"
    );
    let mut hits = Vec::new();
    replay(
        RoutePath::new(&short).unwrap(),
        &assessment.plan,
        &value,
        &tables,
        WalkAllow::default(),
        None,
        |tick, _, _, damage, _| {
            if damage > 0 {
                hits.push((tick, damage));
            }
        },
    )
    .unwrap();
    assert_eq!(hits, [(0, 6), (4, 12), (8, 6), (12, 6)]);
    value.hp = 20;
    value.hp_max = 40;
    let assessment = assess(&short, &cx, value);
    assert_eq!(assessment.verdict, Verdict::Unsurvivable);
    assert!(
        admission_passes(
            &short,
            &assessment,
            &zones,
            &tables,
            WalkAllow::default(),
            Grants::named(ZoneExempt::NONE)
        )
        .unwrap(),
        "the physical damage estimate is honest, but a no-crossing flight is never refused",
    );
}

fn edge(at: WorldTile, to: WorldTile, ticks: i32) -> TransportEdge {
    TransportEdge {
        kind: TransportKind::Ladder,
        at,
        to,
        takeoff: None,
        player_delta: None,
        loc_id: 1,
        option: 1,
        ticks,
        dir: None,
        open_loc_id: None,
        skill_req: vec![],
        item_req: vec![],
        consumed_req: vec![],
        item_returns: vec![],
        quest_req: vec![],
        varp_req: vec![],
        worn_req: vec![],
        worn_all_req: vec![],
        members_req: false,
        wildy_cap: None,
        quest_gates: None,
    }
}

#[test]
fn r1_m2_mid_crossing_transport_hold_never_credits_food() {
    let tables = tables();
    let zones = one_zone(&tables, 100, 6);
    let risks = RiskTables::build(&zones, &tables);
    let wilderness = WildernessRules::default();
    let walk = Route {
        legs: vec![
            Leg::Walk {
                tiles: (85..=98).map(tile).collect(),
            },
            Leg::Transport {
                edge: Box::new(edge(tile(98), tile(99), 9)),
            },
            Leg::Walk {
                tiles: (99..=125).map(tile).collect(),
            },
        ],
        dest: tile(125),
        ticks: 0.0,
    };
    let path = RoutePath::new(&walk).unwrap();
    let mut value = input(40, 40, 6, &tables);
    value.pos = tile(85);
    let assessment = assess(&walk, &context(&zones, &risks, &tables, &wilderness), value);
    let mut hold_bites = Vec::new();
    let mut hold_damage = 0;
    let baseline = replay(
        path,
        &assessment.plan,
        &value,
        &tables,
        WalkAllow::default(),
        None,
        |tick, _, _, damage, bite| {
            if (21..30).contains(&tick) {
                hold_damage += damage;
                if let Some(id) = bite {
                    hold_bites.push((tick, id));
                }
            }
        },
    )
    .unwrap();
    assert_eq!(hold_damage, 12, "hits still land during the delay");
    assert!(
        hold_bites.is_empty(),
        "server drops hold bites: {hold_bites:?}"
    );
    assert!(
        !baseline.passed,
        "the unavailable +12 cannot save this crossing"
    );
    assert_eq!(assessment.verdict, Verdict::Unsurvivable);
}

#[test]
fn r1_m2_pending_heal_waits_until_the_transport_delay_ends() {
    let tables = tables();
    let zones = one_zone(&tables, 500, 1);
    let risks = RiskTables::build(&zones, &tables);
    let wilderness = WildernessRules::default();
    let walk = Route {
        legs: vec![
            Leg::Walk {
                tiles: vec![tile(80)],
            },
            Leg::Transport {
                edge: Box::new(edge(tile(80), tile(81), 9)),
            },
            Leg::Walk {
                tiles: vec![tile(81), tile(82)],
            },
        ],
        dest: tile(82),
        ticks: 0.0,
    };
    let path = RoutePath::new(&walk).unwrap();
    let mut value = input(10, 40, 2, &tables);
    value.poison = PoisonState::Poisoned {
        per_tick: 5,
        last_tick: 0,
    };
    let plan = build_plan(path, &zones, &risks, &wilderness, &value).unwrap();
    let mut trace = Vec::new();
    replay(
        path,
        &plan,
        &value,
        &tables,
        WalkAllow::default(),
        None,
        |tick, _, hp, _, bite| trace.push((tick, hp, bite)),
    )
    .unwrap();
    assert!(trace[0].2.is_some(), "click before the hold is available");
    for (tick, hp, bite) in trace.iter().filter(|(tick, ..)| (2..11).contains(tick)) {
        assert_eq!(*hp, 5, "pending heal cannot land during delay at {tick}");
        assert!(bite.is_none(), "input is locked at {tick}");
    }
    assert_eq!(
        trace[11].1, 17,
        "the pending heal lands once access returns"
    );
}
#[test]
fn u1_geometry_activation_carves_origin_envelope_transport_and_nine_crossings() {
    let tables = tables();
    let kinds = vec![known_kind(&tables)];
    let wilderness = WildernessRules {
        zones: vec![WildernessZone {
            x1: 100,
            z1: 90,
            x2: 110,
            z2: 110,
            level1: 0,
            level2: 0,
            origin_z: 90,
        }],
        divisor: 8,
        offset: 1,
    };
    let mut zone = Zone::npc(tile(100), 2, ZoneClass::LevelRule, 114, 0);
    zone.wild = true;
    let carve = nav::router::AvoidRect {
        min_x: 98,
        max_x: 99,
        min_z: 98,
        max_z: 102,
        level: Some(0),
    };
    let zones = zone_table(vec![zone], kinds, vec![(0, carve)], vec![], &wilderness);
    let risk = RiskTables::build(&zones, &tables);
    let mut value = input(90, 90, 0, &tables);
    value.combat = Some(126);
    let route = route(90, 115);
    let path = RoutePath::new(&route).unwrap();
    let plan = build_plan(path, &zones, &risk, &wilderness, &value).unwrap();
    assert_eq!(
        plan.intervals[0].a, 10,
        "high combat acquires only on wild tile100, never carved98/99"
    );
    assert!(
        plan.intervals[0].b > 12,
        "deactivation outside acquisition does not drop pursuit"
    );
    assert!(plan.intervals[0].e < plan.intervals[0].a);
    let envelope_only = self::route(90, 97);
    assert!(build_plan(
        RoutePath::new(&envelope_only).unwrap(),
        &zones,
        &risk,
        &wilderness,
        &value
    )
    .unwrap()
    .intervals
    .is_empty());
    let origin = self::route(100, 115);
    let plan = build_plan(
        RoutePath::new(&origin).unwrap(),
        &zones,
        &risk,
        &wilderness,
        &value,
    )
    .unwrap();
    assert!(plan.complete);
    assert!(
        plan.crossings.is_empty(),
        "origin-to-exit is escape exposure"
    );
    assert!(plan.intervals.iter().any(|row| row.escaping()));
    let z = one_zone(&tables, 100, 1);
    let r = RiskTables::build(&z, &tables);
    let ladder = Route {
        legs: vec![
            Leg::Transport {
                edge: Box::new(edge(tile(50), tile(100), 7)),
            },
            Leg::Walk {
                tiles: vec![tile(100), tile(101)],
            },
        ],
        dest: tile(101),
        ticks: 8.0,
    };
    let assessed = assess(
        &ladder,
        &context(&z, &r, &tables, &WildernessRules::default()),
        input(90, 90, 0, &tables),
    );
    assert_eq!(assessed.verdict, Verdict::Unsurvivable);
    assert!(assessed.reason.contains("NoWayOut"));
    let taking_off = Route {
        legs: vec![
            Leg::Walk {
                tiles: vec![tile(100)],
            },
            Leg::Transport {
                edge: Box::new(edge(tile(100), tile(200), 7)),
            },
            Leg::Walk {
                tiles: vec![tile(200)],
            },
        ],
        dest: tile(200),
        ticks: 8.0,
    };
    assert_eq!(
        RoutePath::new(&taking_off)
            .unwrap()
            .point(0)
            .unwrap()
            .duration,
        9
    );
    let zones = zone_table(
        (0..9)
            .map(|i| Zone::npc(tile(100 + i * 40), 1, ZoneClass::Always, u16::MAX, 0))
            .collect(),
        vec![known_kind(&tables)],
        vec![],
        vec![],
        &WildernessRules::default(),
    );
    let risk = RiskTables::build(&zones, &tables);
    let route = self::route(80, 450);
    let mut value = input(255, 255, 0, &tables);
    value.prayer_base = 99;
    value.prayer = 255;
    let assessed = assess(
        &route,
        &context(&zones, &risk, &tables, &WildernessRules::default()),
        value,
    );
    assert_eq!(assessed.plan.crossings.len(), 9);
    assert_eq!(assessed.crossings.len(), 8);
    assert_eq!(assessed.more, 1);
}

#[test]
fn u1_u3_four_zone_crossing_keeps_distinct_ranges_rates_and_summed_hits() {
    let tables = tables();
    let w = WildernessRules::default();
    let kinds = [
        "icewarrior",
        "zombie_armed",
        "giantspider2",
        "pirate_aggressive",
    ]
    .into_iter()
    .map(|config| {
        let npc = tables.selected().npc_by_config(config).unwrap();
        ZoneKind::new(config, config, npc.id, 1, false, false)
    })
    .collect();
    let zones = zone_table(
        (0..4)
            .map(|i| Zone::npc(tile(100 + i), 1, ZoneClass::Always, u16::MAX, i as u16))
            .collect(),
        kinds,
        vec![],
        vec![],
        &w,
    );
    let mut risk = RiskTables::build(&zones, &tables);
    for (i, (hit, rate)) in [(2, 1), (3, 4), (4, 5), (5, 5)].into_iter().enumerate() {
        risk.kinds[i] = KindRisk::new(
            hit,
            rate,
            6 + i as u8,
            1 + i as u8,
            Style::Melee,
            UnknownKind::Known,
            0,
            true,
            false,
            false,
            false,
        );
    }
    let walk = route(90, 115);
    let path = RoutePath::new(&walk).unwrap();
    let value = input(255, 255, 0, &tables);
    let plan = build_plan(path, &zones, &risk, &w, &value).unwrap();
    assert_eq!(plan.crossings.len(), 1);
    assert_eq!(
        plan.crossings[0],
        CrossingGeom {
            first: 9,
            last: 22,
            env_first: 4,
            env_last: 22,
            intervals: (0, 4),
            retreat: 3,
            forward: 23,
            leg: 0
        }
    );
    let actual: Vec<_> = plan
        .intervals
        .iter()
        .map(|row| {
            (
                row.a,
                row.b,
                row.e,
                row.f,
                row.r,
                row.reach,
                row.rate,
                path.exposure_ticks(row.a, row.b).unwrap(),
            )
        })
        .collect();
    assert_eq!(
        actual,
        [
            (9, 16, 4, 16, 6, 1, 1, 12),
            (10, 18, 4, 18, 7, 2, 4, 14),
            (11, 20, 4, 20, 8, 3, 5, 15),
            (12, 22, 4, 22, 9, 4, 5, 17)
        ]
    );
    // Independent integer arithmetic: d=9, lag=2, then the pace-ahead
    // midpoint raises d to 10. No scene or single-way cap erases any row.
    assert_eq!(candidate_cost(path, &plan, 12, false).unwrap(), Some(58));
    assert_eq!(
        estimated_floor(path, &plan, 12, &value, &tables, WalkAllow::default()).unwrap(),
        Some(65)
    );
    let mut damage = 0;
    replay(
        path,
        &plan,
        &value,
        &tables,
        WalkAllow::default(),
        None,
        |_, _, _, hit, _| damage += hit,
    )
    .unwrap();
    assert_eq!(damage, 68); // 2*12 + 3*4 + 4*3 + 5*4.
    let assessed = assess(&walk, &context(&zones, &risk, &tables, &w), value);
    assert_eq!(
        (assessed.verdict, assessed.hp_after),
        (Verdict::Survivable, 187)
    );
    assert_eq!(
        (assessed.crossings[0].worst, assessed.crossings[0].volley),
        (68, 14)
    );
    assert!(!assessed.crossings[0].single);
}
#[test]
fn u5_witnesses_unknown_overflow_and_multi_burst() {
    let tables = tables();
    let zones = one_zone(&tables, 100, 1);
    let risk = RiskTables::build(&zones, &tables);
    let w = WildernessRules::default();
    let cx = context(&zones, &risk, &tables, &w);
    let route = route(88, 112);
    let missing = Estimate { zones: None, ..cx };
    assert_eq!(
        assess(&route, &missing, input(40, 40, 0, &tables)).verdict,
        Verdict::Unknown(UnknownWhy::NoZoneTable)
    );
    let mut bad = input(40, 40, 0, &tables);
    bad.unattributed = true;
    assert_eq!(
        assess(&route, &cx, bad).verdict,
        Verdict::Unknown(UnknownWhy::AlreadyEngaged)
    );
    let huge = Route {
        legs: vec![Leg::Walk {
            tiles: vec![tile(100); 65537],
        }],
        dest: tile(100),
        ticks: 0.0,
    };
    assert_eq!(
        assess(&huge, &cx, input(40, 40, 0, &tables)).verdict,
        Verdict::Unknown(UnknownWhy::Overflow)
    );
    let burst = zone_table(
        (0..8)
            .map(|i| Zone::npc(tile(97 + i), 1, ZoneClass::Always, u16::MAX, 0))
            .collect(),
        vec![known_kind(&tables)],
        vec![],
        vec![],
        &w,
    );
    let risk = RiskTables::build(&burst, &tables);
    let burst_assessment = assess(
        &route,
        &context(&burst, &risk, &tables, &w),
        input(20, 40, 0, &tables),
    );
    assert_eq!(burst_assessment.verdict, Verdict::Unsurvivable);
    assert!(
        burst_assessment.reason.contains("max HP"),
        "{}",
        burst_assessment.reason
    );
    let overflow = zone_table(
        (0..44)
            .map(|i| {
                Zone::npc(
                    WorldTile {
                        x: 97 + i % 7,
                        z: 97 + i / 7,
                        level: 0,
                    },
                    6,
                    ZoneClass::Always,
                    u16::MAX,
                    0,
                )
            })
            .collect(),
        vec![known_kind(&tables)],
        vec![],
        vec![],
        &w,
    );
    let risk = RiskTables::build(&overflow, &tables);
    assert_eq!(
        assess(
            &route,
            &context(&overflow, &risk, &tables, &w),
            input(255, 255, 0, &tables)
        )
        .verdict,
        Verdict::Unknown(UnknownWhy::Overflow)
    );
}

#[test]
fn u9_assessment_allocation_and_cpu_measurement() {
    let tables = tables();
    let zones = one_zone(&tables, 100, 1);
    let w = WildernessRules::default();
    let start = std::time::Instant::now();
    let risks = RiskTables::build(&zones, &tables);
    eprintln!("S2a RiskTables warm-combat build {:?}, KindRisk={} RiskInput={} Poison={} ZoneInterval={} Crossing={} Assessment={}",start.elapsed(),std::mem::size_of::<KindRisk>(),std::mem::size_of::<RiskInput>(),std::mem::size_of::<PoisonState>(),std::mem::size_of::<ZoneInterval>(),std::mem::size_of::<Crossing>(),std::mem::size_of::<RouteAssessment>());
    let cx = context(&zones, &risks, &tables, &w);
    let mut timings = Vec::new();
    for (name, route, input) in [
        ("safe", route(10, 30), input(40, 40, 0, &tables)),
        ("crossing", route(88, 112), input(90, 90, 0, &tables)),
        ("fixable", route(88, 112), input(20, 40, 0, &tables)),
    ] {
        let allocations = allocation_counter::measure(|| {
            let assessment = assess(&route, &cx, input);
            std::hint::black_box(assessment);
        });
        eprintln!(
            "S2a {name}: allocations={}, bytes={}",
            allocations.count_total, allocations.bytes_total
        );
        assert!(allocations.count_total <= 6, "{name}: {allocations:?}");
        timings.clear();
        for _ in 0..100 {
            let start = std::time::Instant::now();
            std::hint::black_box(assess(&route, &cx, input));
            timings.push(start.elapsed().as_nanos());
        }
        timings.sort_unstable();
        eprintln!("S2a {name}: p50={}ns p95={}ns", timings[50], timings[95]);
    }
}

#[test]
fn u1_shaped_and_ap_exposure_uses_local_ceiling_and_takeoff_hold() {
    let tables = tables();
    let w = WildernessRules::default();
    let npc = known_kind(&tables);
    let zones = zone_table(
        vec![Zone::shaped_npc(
            tile(100),
            1,
            1,
            ZoneClass::Always,
            u16::MAX,
            0,
            0,
        )],
        vec![npc],
        vec![],
        vec![0b010101010],
        &w,
    );
    let mut risk = RiskTables::build(&zones, &tables);
    let fact = risk.kinds[0];
    risk.kinds[0] = KindRisk::new(
        fact.max_hit,
        fact.rate,
        fact.r,
        fact.reach,
        fact.style,
        fact.unknown,
        fact.poison,
        fact.shared_attack(),
        fact.forcemulti(),
        true,
        fact.hazard(),
    );
    let shaped = Route {
        legs: vec![Leg::Walk {
            tiles: vec![
                tile(98),
                tile(99),
                WorldTile {
                    x: 99,
                    z: 99,
                    level: 0,
                },
                WorldTile {
                    x: 100,
                    z: 99,
                    level: 0,
                },
                WorldTile {
                    x: 101,
                    z: 99,
                    level: 0,
                },
                tile(101),
                tile(102),
            ],
        }],
        dest: tile(102),
        ticks: 7.0,
    };
    let path = RoutePath::new(&shaped).unwrap();
    let plan = build_plan(path, &zones, &risk, &w, &input(90, 90, 0, &tables)).unwrap();
    let row = plan.intervals[0];
    assert_eq!((row.a, row.b, row.e, row.f), (1, 5, 1, 5));
    // Local ceil(5 * 1.5), not a subtraction of globally rounded tile times.
    assert_eq!(path.exposure_ticks(row.a, row.b).unwrap(), 8);
    let mut landed = Vec::new();
    replay(
        path,
        &plan,
        &input(90, 90, 0, &tables),
        &tables,
        WalkAllow::default(),
        None,
        |tick, _, _, damage, _| {
            if damage > 0 {
                landed.push(tick)
            }
        },
    )
    .unwrap();
    assert_eq!(landed, vec![2, 6, 10]); // 8 + AP's 3-tick tail, rate 4.
    let mut transport = edge(tile(101), tile(120), 11);
    transport.takeoff = Some(tile(101));
    let held = Route {
        legs: vec![
            Leg::Walk {
                tiles: vec![tile(99), tile(100), tile(101)],
            },
            Leg::Transport {
                edge: Box::new(transport),
            },
            Leg::Walk {
                tiles: vec![tile(120)],
            },
        ],
        dest: tile(120),
        ticks: 0.0,
    };
    assert_eq!(
        RoutePath::new(&held).unwrap().exposure_ticks(1, 2).unwrap(),
        14
    );
}

#[test]
fn u4_mixed_food_topup_windows_exclusions_and_exact_threshold() {
    let tables = tables();
    let zones = one_zone(&tables, 120, 1);
    let risks = RiskTables::build(&zones, &tables);
    let w = WildernessRules::default();
    let cx = context(&zones, &risks, &tables, &w);
    let shrimp = tables.selected().item_by_alias("shrimp").unwrap().id;
    let lobster = tables.selected().item_by_alias("lobster").unwrap().id;
    let delayed = tables
        .selected()
        .item_by_alias("tbwt_cooked_karambwan")
        .unwrap()
        .id;
    let mut value = input(20, 40, 1, &tables);
    value.food_ids[1] = shrimp;
    value.food_counts[1] = 6;
    value.food_len = 2;
    let walk = route(100, 150);
    let path = RoutePath::new(&walk).unwrap();
    let plan = corridor_plan(&tables, 20);
    let mut bites = Vec::new();
    let actual = replay(
        path,
        &plan,
        &value,
        &tables,
        WalkAllow::default(),
        None,
        |tick, _, hp, _, bite| {
            if let Some(id) = bite {
                bites.push((tick, hp, id))
            }
        },
    )
    .unwrap();
    assert_eq!(bites[0], (0, 20, lobster));
    assert_eq!(bites[1], (3, 32, shrimp));
    assert_eq!(bites[2], (6, 35, shrimp));
    assert_eq!(bites[3], (33, 32, shrimp));
    assert!(bites.windows(2).all(|pair| pair[1].0 - pair[0].0 >= 3));
    let independent = simulate_ice(20, 40, 1, 20, 6);
    assert_eq!(
        (actual.passed, actual.hp_after, actual.bites),
        (independent.0, independent.1, independent.2)
    );
    assert_eq!(
        bites.iter().map(|(tick, _, _)| *tick).collect::<Vec<_>>(),
        independent.3
    );
    let false_allow = WalkAllow {
        food: false,
        ..WalkAllow::default()
    };
    assert_eq!(
        replay(
            path,
            &plan,
            &value,
            &tables,
            false_allow,
            None,
            |_, _, _, _, _| {}
        )
        .unwrap()
        .bites,
        0
    );
    value.food_ids = [delayed, -1, -1, -1, -1, -1];
    value.food_counts = [10, 0, 0, 0, 0, 0];
    value.more_food = true;
    value.food_len = 1;
    let assessed = assess(&walk, &cx, value);
    assert!(assessed.reason.contains("karambwan"), "{}", assessed.reason);
    assert!(assessed.reason.contains("six"), "{}", assessed.reason);
    assert_eq!(
        replay(
            path,
            &plan,
            &value,
            &tables,
            WalkAllow::default(),
            None,
            |_, _, _, _, _| {}
        )
        .unwrap()
        .bites,
        0
    );
    // Top-up is awake only within thirty walking ticks; no deficit-free top-up.
    let far_walk = route(100, 180);
    let far = RoutePath::new(&far_walk).unwrap();
    let far_plan = corridor_plan(&tables, 40);
    let mut first = None;
    replay(
        far,
        &far_plan,
        &input(20, 40, 10, &tables),
        &tables,
        WalkAllow::default(),
        None,
        |tick, i, _, _, bite| {
            if first.is_none() && bite.is_some() {
                first = Some((tick, i))
            }
        },
    )
    .unwrap();
    assert_eq!(first, Some((15, 10)));
    let short_walk = route(100, 150);
    let short = RoutePath::new(&short_walk).unwrap();
    let short_plan = corridor_plan(&tables, 2);
    let mut first = None;
    replay(
        short,
        &short_plan,
        &input(20, 40, 10, &tables),
        &tables,
        WalkAllow::default(),
        None,
        |tick, i, _, _, bite| {
            if first.is_none() && bite.is_some() {
                first = Some((tick, i))
            }
        },
    )
    .unwrap();
    assert_eq!(first, Some((0, 0)));
    let line = eat_line(
        path,
        &plan,
        31,
        &input(90, 90, 1, &tables),
        &tables,
        WalkAllow::default(),
    )
    .unwrap();
    assert_eq!(line, 36);
    let mut at_boundary = input(37, 90, 1, &tables);
    at_boundary.pos = tile(131);
    // Rebase the tested position to the origin, suppressing the pre-entry top-up.
    let mut boundary = corridor_plan(&tables, 20);
    boundary.intervals[0].a = 0;
    boundary.intervals[0].b = 25;
    boundary.intervals[0].e = 0;
    boundary.intervals[0].f = 25;
    boundary.crossings[0].first = 0;
    boundary.crossings[0].last = 25;
    boundary.crossings[0].env_first = 0;
    boundary.crossings[0].env_last = 25;
    boundary.crossings[0].retreat = CrossingGeom::NONE;
    boundary.crossings[0].forward = 26;
    let mut actual = Vec::new();
    replay(
        path,
        &boundary,
        &at_boundary,
        &tables,
        WalkAllow::default(),
        None,
        |tick, i, hp, _, bite| {
            if let Some(id) = bite {
                actual.push((tick, i, hp, id))
            }
        },
    )
    .unwrap();
    assert!(actual.iter().all(|(_, i, hp, _)| *hp
        <= eat_line(
            path,
            &boundary,
            *i,
            &at_boundary,
            &tables,
            WalkAllow::default()
        )
        .unwrap()));
}

#[test]
fn u5_b1_b3_live_classification_multi_and_input_limits() {
    let tables = tables();
    let zones = one_zone(&tables, 100, 1);
    let risks = RiskTables::build(&zones, &tables);
    let w = WildernessRules::default();
    let cx = context(&zones, &risks, &tables, &w);
    let walk = route(88, 112);
    let mut locked = input(90, 90, 0, &tables);
    locked.input_locked = true;
    assert_eq!(
        assess(&walk, &cx, locked).verdict,
        Verdict::Unknown(UnknownWhy::InputLock)
    );
    let mut overflow = input(90, 90, 0, &tables);
    overflow.overflow = true;
    assert_eq!(
        assess(&walk, &cx, overflow).verdict,
        Verdict::Unknown(UnknownWhy::Overflow)
    );
    let mut live = input(90, 90, 0, &tables);
    live.live_len = 1;
    live.live[0] = LiveRow {
        actor: crate::combat::ActorRef {
            kind: crate::combat::ActorKind::Npc,
            index: 7,
        },
        ident: tables.selected().npc_by_config("icewarrior").unwrap().id,
        max_hit: 6,
        rate: 4,
        due_tick: u16::MAX,
    };
    assert!(!matches!(
        assess(&walk, &cx, live).verdict,
        Verdict::Unknown(_)
    ));
    live.live[0].ident = tables.selected().npc_by_config("poisonspider").unwrap().id;
    live.map_members = true;
    assert_eq!(
        assess(&walk, &cx, live).verdict,
        Verdict::Unknown(UnknownWhy::AlreadyEngaged)
    );
    let path = RoutePath::new(&walk).unwrap();
    let ident = known_kind(&tables).npc_id;
    let weak = RoutePlan {
        complete: true,
        intervals: vec![ZoneInterval::new(
            0,
            ident,
            tile(100),
            1,
            2,
            1,
            2,
            8,
            1,
            3,
            5,
            Style::Melee,
            false,
            false,
            false,
        )
        .unwrap()]
        .into_boxed_slice(),
        crossings: vec![CrossingGeom {
            first: 1,
            last: 2,
            env_first: 1,
            env_last: 2,
            intervals: (0, 1),
            retreat: 0,
            forward: 3,
            leg: 0,
        }]
        .into_boxed_slice(),
    };
    assert_eq!(
        replay(
            path,
            &weak,
            &input(10, 40, 0, &tables),
            &tables,
            WalkAllow::default(),
            None,
            |_, _, _, _, _| {}
        )
        .unwrap()
        .hp_after,
        7
    );
    let mut poisoned = input(10, 40, 0, &tables);
    poisoned.poison = PoisonState::Poisoned {
        per_tick: 6,
        last_tick: 0,
    };
    assert!(
        estimated_floor(path, &weak, 1, &poisoned, &tables, WalkAllow::default())
            .unwrap()
            .unwrap()
            >= 12
    );
    assert!(
        !replay(
            path,
            &weak,
            &poisoned,
            &tables,
            WalkAllow::default(),
            None,
            |_, _, _, _, _| {}
        )
        .unwrap()
        .passed
    );
    let multi = RoutePlan {
        complete: true,
        intervals: (0..4)
            .map(|i| {
                ZoneInterval::new(
                    i,
                    ident,
                    tile(100),
                    1,
                    26,
                    1,
                    26,
                    8,
                    1,
                    6,
                    4,
                    Style::Melee,
                    false,
                    false,
                    false,
                )
                .unwrap()
            })
            .collect::<Vec<_>>()
            .into_boxed_slice(),
        crossings: vec![CrossingGeom {
            first: 1,
            last: 26,
            env_first: 1,
            env_last: 26,
            intervals: (0, 4),
            retreat: 0,
            forward: 27,
            leg: 0,
        }]
        .into_boxed_slice(),
    };
    let long = route(80, 120);
    let long = RoutePath::new(&long).unwrap();
    assert!(
        estimated_floor(
            long,
            &multi,
            13,
            &input(45, 45, 28, &tables),
            &tables,
            WalkAllow::default()
        )
        .unwrap()
        .unwrap()
            > 45
    );
    assert!(
        !replay(
            long,
            &multi,
            &input(45, 45, 28, &tables),
            &tables,
            WalkAllow::default(),
            None,
            |_, _, _, _, _| {}
        )
        .unwrap()
        .passed
    );
    let enormous = Route {
        legs: vec![
            Leg::Transport {
                edge: Box::new(edge(tile(50), tile(100), i32::MAX)),
            },
            Leg::Walk {
                tiles: vec![tile(100)],
            },
        ],
        dest: tile(100),
        ticks: 0.0,
    };
    assert_eq!(
        assess(&enormous, &cx, input(90, 90, 0, &tables)).verdict,
        Verdict::Unknown(UnknownWhy::Overflow)
    );
}

#[test]
fn u9_cold_shared_tables_and_kind_build_measurement() {
    let start = std::time::Instant::now();
    let selected = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let selected_elapsed = start.elapsed();
    let start = std::time::Instant::now();
    let combat = CombatTables::build(selected).unwrap();
    let combat_elapsed = start.elapsed();
    let kinds: Vec<_> = combat
        .selected()
        .npc_names()
        .unwrap()
        .rows
        .iter()
        .filter(|npc| npc.ops.iter().any(|op| op.eq_ignore_ascii_case("attack")))
        .take(250)
        .map(|npc| {
            ZoneKind::new(
                npc.config.as_str(),
                npc.config.as_str(),
                npc.id,
                1,
                false,
                false,
            )
        })
        .collect();
    let zones = zone_table(
        (0..kinds.len())
            .map(|i| {
                Zone::npc(
                    WorldTile {
                        x: 100 + (i % 20) as i32 * 20,
                        z: 100 + (i / 20) as i32 * 20,
                        level: 0,
                    },
                    1,
                    ZoneClass::Always,
                    u16::MAX,
                    i as u16,
                )
            })
            .collect(),
        kinds,
        vec![],
        vec![],
        &WildernessRules::default(),
    );
    let start = std::time::Instant::now();
    let mut payload = None;
    let allocations =
        allocation_counter::measure(|| payload = Some(RiskTables::build(&zones, &combat)));
    let elapsed = start.elapsed();
    eprintln!("S2a first build selected={selected_elapsed:?} combat={combat_elapsed:?} risk250={elapsed:?}; RiskTables allocations={} payload={}B retained={}B; kinds={}",
        allocations.count_total,allocations.bytes_total,allocations.bytes_current,zones.kinds().len());
    assert!(payload.unwrap().kind(249).is_some());
}

#[cfg(feature = "load")]
#[test]
#[ignore = "isolated cold construction-count probe; run explicitly"]
fn u9_normal_slot_start_does_not_construct_risk_tables() {
    assert_eq!(facts::build_count_for_test(), 0);
    let mut slot = crate::SlotScript::new();
    slot.start_load_with_loadouts(
        "export function tick() {}".into(),
        crate::load::LoadShape::NativeTick,
        vec![],
        &[],
    )
    .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while slot.state() != crate::slot::RunState::Running && std::time::Instant::now() < deadline {
        slot.observe_lifecycle();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(slot.state(), crate::slot::RunState::Running);
    assert_eq!(
        facts::build_count_for_test(),
        0,
        "ordinary slot startup must not initialize risk"
    );
    slot.stop();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while slot.state() != crate::slot::RunState::Idle && std::time::Instant::now() < deadline {
        slot.observe_lifecycle();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(slot.state(), crate::slot::RunState::Idle);
    let tables = tables();
    let zones = one_zone(&tables, 100, 1);
    let w = WildernessRules::default();
    let risk = RiskTables::build(&zones, &tables);
    assert_eq!(facts::build_count_for_test(), 1);
    assess(
        &route(88, 112),
        &context(&zones, &risk, &tables, &w),
        input(90, 90, 0, &tables),
    );
    assert_eq!(
        facts::build_count_for_test(),
        1,
        "assessment must reuse the immutable table"
    );
}

#[test]
fn u5_hand_examples_typed_verdict_and_least_additional_reference_food() {
    let tables = tables();
    let kind = known_kind(&tables);
    let initial = one_zone(&tables, 100, 1);
    let risk = RiskTables::build(&initial, &tables);
    let r = i32::from(risk.kind(0).unwrap().r);
    assert_eq!(r, 6, "selected icewarrior has maxrange=5 and attackrange=0");
    let w = WildernessRules::default();
    let zones = zone_table(
        vec![Zone::npc(
            tile(100),
            r as u8,
            ZoneClass::Always,
            u16::MAX,
            0,
        )],
        vec![kind],
        vec![],
        vec![],
        &w,
    );
    let risk = RiskTables::build(&zones, &tables);
    let mut tiles: Vec<_> = (100 - r - 20..100 - r).map(tile).collect();
    tiles.extend((100 - r..=100 + r).map(tile));
    tiles.extend((1..=r).map(|dz| WorldTile {
        x: 100 + r,
        z: 100 + dz,
        level: 0,
    }));
    tiles.extend((1..=7).map(|dx| WorldTile {
        x: 100 + r - dx,
        z: 100 + r,
        level: 0,
    }));
    tiles.extend((1..=3).map(|dz| WorldTile {
        x: 99,
        z: 100 + r + dz,
        level: 0,
    }));
    let dest = *tiles.last().unwrap();
    let walk = Route {
        legs: vec![Leg::Walk { tiles }],
        dest,
        ticks: 0.0,
    };
    let cx = context(&zones, &risk, &tables, &w);
    for (hp, cap, food, verdict, needed, after) in [
        (40, 40, 0, Verdict::FixableWith, 5, 22),
        (40, 40, 4, Verdict::FixableWith, 1, 22),
        (40, 40, 5, Verdict::Survivable, 0, 16),
        (45, 45, 6, Verdict::Survivable, 0, 21),
        (20, 40, 0, Verdict::FixableWith, 7, 14),
        (20, 40, 6, Verdict::FixableWith, 1, 22),
        (20, 40, 7, Verdict::Survivable, 0, 16),
    ] {
        let mut value = input(hp, cap, food, &tables);
        value.pos = tile(100 - r - 20);
        let actual = assess(&walk, &cx, value);
        assert_eq!(
            (actual.verdict, actual.hp_after),
            (verdict, after),
            "hp={hp},food={food}: {}",
            actual.reason
        );
        let more = actual
            .supplies
            .iter()
            .filter_map(|need| match need {
                SupplyNeed::Food { count, item, .. } => {
                    assert_eq!(*item, Some(value.food_ids[0]));
                    Some(*count)
                }
                _ => None,
            })
            .sum::<u8>();
        assert_eq!(more, needed, "hp={hp},food={food}");
    }
}

#[test]
fn u5_origin_volley_and_raw_pending_launch_are_not_prayer_credited() {
    let tables = tables();
    let zones = one_zone(&tables, 100, 1);
    let risk = RiskTables::build(&zones, &tables);
    let w = WildernessRules::default();
    let cx = context(&zones, &risk, &tables, &w);
    let walk = route(88, 112);
    let mut value = input(90, 90, 0, &tables);
    value.prayer = 99;
    value.prayer_base = 99;
    value.live_len = 1;
    value.live[0] = LiveRow {
        actor: crate::combat::ActorRef {
            kind: crate::combat::ActorKind::Npc,
            index: 7,
        },
        ident: known_kind(&tables).npc_id,
        max_hit: 6,
        rate: 4,
        due_tick: 2,
    };
    let assessed = assess(&walk, &cx, value);
    assert!(assessed.crossings[0].protect_credited);
    assert_eq!(assessed.crossings[0].volley, 12);
    assert_eq!(assessed.volley, 12);
    let path = RoutePath::new(&walk).unwrap();
    let mut damage_at_due = None;
    replay(
        path,
        &assessed.plan,
        &value,
        &tables,
        WalkAllow::default(),
        None,
        |tick, _, _, damage, _| {
            if tick == 2 {
                damage_at_due = Some(damage)
            }
        },
    )
    .unwrap();
    assert_eq!(
        damage_at_due,
        Some(6),
        "already-launched raw damage survives admission credit"
    );
}

#[test]
fn u1_ap_single_tile_at_an_odd_route_index_keeps_the_final_flight() {
    let tables = tables();
    let w = WildernessRules::default();
    let mut kind = known_kind(&tables);
    kind.ap = true;
    let zones = zone_table(
        vec![Zone::shaped_npc(
            tile(100),
            1,
            1,
            ZoneClass::Always,
            u16::MAX,
            0,
            0,
        )],
        vec![kind],
        vec![],
        vec![1 << 3],
        &w,
    );
    let mut risk = RiskTables::build(&zones, &tables);
    assert!(
        !risk.kinds[0].ap(),
        "selected NPC facts override the zone ap hint"
    );
    let fact = risk.kinds[0];
    risk.kinds[0] = KindRisk::new(
        fact.max_hit,
        fact.rate,
        fact.r,
        fact.reach,
        fact.style,
        fact.unknown,
        fact.poison,
        fact.shared_attack(),
        fact.forcemulti(),
        true,
        fact.hazard(),
    );
    let walk = Route {
        legs: vec![Leg::Walk {
            tiles: vec![tile(98), tile(99), tile(98), tile(97), tile(96)],
        }],
        dest: tile(96),
        ticks: 0.0,
    };
    let path = RoutePath::new(&walk).unwrap();
    let plan = build_plan(path, &zones, &risk, &w, &input(90, 90, 0, &tables)).unwrap();
    assert_eq!((plan.intervals[0].a, plan.intervals[0].b), (1, 1));
    let mut hits = Vec::new();
    replay(
        path,
        &plan,
        &input(90, 90, 0, &tables),
        &tables,
        WalkAllow::default(),
        None,
        |tick, _, _, damage, _| {
            if damage > 0 {
                hits.push(tick)
            }
        },
    )
    .unwrap();
    assert_eq!(hits, vec![2, 6]);
}

#[test]
fn u1_admission_does_not_credit_a_teleport_as_an_on_foot_exit() {
    let tables = tables();
    let zones = one_zone(&tables, 100, 1);
    let risk = RiskTables::build(&zones, &tables);
    let w = WildernessRules::default();
    let mut teleport = edge(tile(100), tile(200), 5);
    teleport.kind = TransportKind::Teleport;
    let walk = Route {
        legs: vec![
            Leg::Walk {
                tiles: vec![tile(80)],
            },
            // Enter by transport, so neither a same-leg retreat nor the
            // following teleport can supply an on-foot exit.
            Leg::Transport {
                edge: Box::new(edge(tile(80), tile(100), 5)),
            },
            Leg::Transport {
                edge: Box::new(teleport),
            },
            Leg::Walk {
                tiles: vec![tile(200)],
            },
        ],
        dest: tile(200),
        ticks: 0.0,
    };
    let result = assess(
        &walk,
        &context(&zones, &risk, &tables, &w),
        input(90, 90, 0, &tables),
    );
    assert_eq!(
        result.plan.crossings.len(),
        1,
        "this probe enters before teleporting"
    );
    assert_eq!(result.verdict, Verdict::Unsurvivable);
    assert!(result.reason.contains("NoWayOut"));
}

#[test]
fn u1_transport_input_hold_boundaries_cover_attached_and_standalone_legs() {
    for (legs, first, start, end) in [
        (
            vec![
                Leg::Walk {
                    tiles: vec![tile(80)],
                },
                Leg::Transport {
                    edge: Box::new(edge(tile(80), tile(81), 9)),
                },
                Leg::Walk {
                    tiles: vec![tile(81)],
                },
            ],
            0,
            2,
            11,
        ),
        (
            vec![
                Leg::Transport {
                    edge: Box::new(edge(tile(80), tile(81), 9)),
                },
                Leg::Walk {
                    tiles: vec![tile(81)],
                },
            ],
            0,
            0,
            9,
        ),
    ] {
        let route = Route {
            legs,
            dest: tile(81),
            ticks: 0.0,
        };
        let point = RoutePath::new(&route).unwrap().point(first).unwrap();
        assert!(!point.input_held(start - 1));
        assert!(point.input_held(start));
        assert!(point.input_held(end - 1));
        assert!(!point.input_held(end));
    }
    let walk = route(80, 81);
    assert!(!RoutePath::new(&walk)
        .unwrap()
        .point(0)
        .unwrap()
        .input_held(0));
}

/// S2b R3 M-A: an escape starts only from an engaged origin (inside A_z and
/// active). A fringe origin inside E_z but outside A_z that walks into A_z
/// enters the zone and is a crossing.
#[test]
fn r3_fringe_origin_entering_the_acquisition_set_is_a_crossing() {
    let tables = tables();
    let zones = one_zone(&tables, 100, 0);
    let risk = RiskTables::build(&zones, &tables);
    let r = i32::from(risk.kind(0).unwrap().r);
    assert!(r > 0, "the envelope must be wider than the radius-0 rect");
    let value = input(99, 99, 0, &tables);
    let wilderness = WildernessRules::default();
    let plan_from = |first: i32| {
        let walk = route(first, 100 + r + 5);
        build_plan(
            RoutePath::new(&walk).unwrap(),
            &zones,
            &risk,
            &wilderness,
            &value,
        )
        .unwrap()
    };
    let fringe = plan_from(100 - r);
    assert_eq!(fringe.intervals.len(), 1);
    assert_eq!(i32::from(fringe.intervals[0].a), r, "acquired at the spawn");
    assert_eq!(fringe.intervals[0].e, 0, "the origin is exposed");
    assert!(!fringe.intervals[0].escaping());
    assert_eq!(fringe.crossings.len(), 1, "walking into A_z is entry");
    let engaged = plan_from(100);
    assert_eq!(engaged.intervals.len(), 1);
    assert!(engaged.intervals[0].escaping());
    assert!(engaged.crossings.is_empty(), "leaving A_z is escape");
}

/// S2b R3 N1 (reviewer probe `partition_point_skips_a_non_escaping_origin_row`):
/// an engaged origin outside its own envelope (rect wider than spawn ± R)
/// would be a non-escaping `a = 0` row and break the escaping-prefix
/// partition. `ZoneInterval::new` rejects `e > a`, so the plan fails closed
/// as `Unknown(Overflow)`, never as a misfiled escape or crossing.
#[test]
fn r3_engaged_origin_outside_its_envelope_fails_closed() {
    let tables = tables();
    let probe = one_zone(&tables, 100, 0);
    let r = i32::from(RiskTables::build(&probe, &tables).kind(0).unwrap().r);
    let at = |x: i32, z: i32| WorldTile { x, z, level: 0 };
    let origin = at(300, 300);
    // P's rect (radius 2R) covers the origin; its envelope ends at z = 299.
    let wide = Zone::npc(
        at(300, 299 - r),
        u8::try_from(2 * r).unwrap(),
        ZoneClass::Always,
        u16::MAX,
        0,
    );
    let local = Zone::npc(origin, 0, ZoneClass::Always, u16::MAX, 0);
    let zones = zone_table(
        vec![wide, local],
        vec![known_kind(&tables)],
        vec![],
        vec![],
        &WildernessRules::default(),
    );
    let risk = RiskTables::build(&zones, &tables);
    let mut tiles = vec![origin, at(300, 299), origin];
    tiles.extend((301..=300 + r + 3).map(|x| at(x, 300)));
    let walk = Route {
        dest: *tiles.last().unwrap(),
        legs: vec![Leg::Walk { tiles }],
        ticks: 10.0,
    };
    let mut value = input(99, 99, 0, &tables);
    value.pos = origin;
    assert_eq!(
        build_plan(
            RoutePath::new(&walk).unwrap(),
            &zones,
            &risk,
            &WildernessRules::default(),
            &value,
        )
        .unwrap_err(),
        UnknownWhy::Overflow
    );
    let assessment = assess(
        &walk,
        &context(&zones, &risk, &tables, &WildernessRules::default()),
        value,
    );
    assert_eq!(assessment.verdict, Verdict::Unknown(UnknownWhy::Overflow));
    assert!(!assessment.plan.complete);
    assert!(admission_passes(
        &walk,
        &assessment,
        &zones,
        &tables,
        WalkAllow::default(),
        Grants::named(ZoneExempt::NONE)
    )
    .is_err());
}

/// S2b R3 M-B: a failed assessment carries an empty, incomplete plan.
/// Replaying that empty plan must never certify the route.
#[test]
fn r3_admission_never_passes_an_incomplete_plan() {
    let tables = tables();
    // Forty-one overlapping envelopes on a valid walk: the summed way-out
    // floor exceeds the u8 report bound, so the estimate fails after a
    // successful `build_plan`.
    let zones = zone_table(
        (100..=140)
            .map(|x| Zone::npc(tile(x), 0, ZoneClass::Always, u16::MAX, 0))
            .collect(),
        vec![known_kind(&tables)],
        vec![],
        vec![],
        &WildernessRules::default(),
    );
    let risk = RiskTables::build(&zones, &tables);
    let wilderness = WildernessRules::default();
    let walk = route(80, 170);
    assert!(build_plan(
        RoutePath::new(&walk).unwrap(),
        &zones,
        &risk,
        &wilderness,
        &input(99, 99, 0, &tables),
    )
    .is_ok_and(|plan| !plan.crossings.is_empty()));
    let assessment = assess(
        &walk,
        &context(&zones, &risk, &tables, &wilderness),
        input(99, 99, 0, &tables),
    );
    assert_eq!(assessment.verdict, Verdict::Unknown(UnknownWhy::Overflow));
    assert!(!assessment.plan.complete);
    for grants in [ZoneExempt::NONE, ZoneExempt::all()] {
        assert_eq!(
            admission_passes(
                &walk,
                &assessment,
                &zones,
                &tables,
                WalkAllow::default(),
                Grants::named(grants)
            ),
            Err(UnknownWhy::Overflow)
        );
    }
}
