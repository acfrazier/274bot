use super::replay::{candidate_cost, Estimate};
use super::*;
use crate::combat::tables::CombatTables;
use crate::native::WalkAllow;
use api::selected::ClientRevision;
use api::WorldTile;
use nav::router::{Leg, Route};
use nav::transport::{TransportEdge, TransportKind, WildernessRules, WildernessZone};
use nav::zones::{Zone, ZoneClass, ZoneKind, ZoneTable};
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
fn u3_c4_uncapped_counts_ignore_built_limited_or_incomplete_scene() {
    // All these have a built scene, but none certifies the horizon: >15 tiles,
    // the 255-NPC view cap, a retained row absent now, and disjoint identities.
    for (live, estimate, nearby) in [(1, 2, 1), (4, 300, 255), (1, 1, 0), (2, 3, 3)] {
        assert_eq!(
            attacker_count(live, estimate, None).unwrap(),
            live + estimate
        );
        assert_eq!(
            attacker_count(live, estimate, Some(nearby)).unwrap(),
            (live + estimate).min(nearby)
        );
    }
    assert_eq!(attacker_count(u16::MAX, 1, None), Err(UnknownWhy::Overflow));
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
        .contains("separate safe walk"));
    assert_eq!(assess(&safe, &cx, poisoned).input.poison, poisoned.poison);
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
    assert_eq!(plan.crossings[0].retreat, CrossingGeom::NONE);
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
    let kinds = (0..4)
        .map(|i| {
            let mut kind = known_kind(&tables);
            kind.id = format!("ice{i}").into();
            kind
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
    assert_eq!(
        assess(
            &route,
            &context(&burst, &risk, &tables, &w),
            input(20, 40, 0, &tables)
        )
        .verdict,
        Verdict::Unsurvivable
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
    let mut npc = known_kind(&tables);
    npc.ap = true;
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
    let risk = RiskTables::build(&zones, &tables);
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
    let mut false_allow = WalkAllow::default();
    false_allow.food = false;
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
    let risk = RiskTables::build(&zones, &tables);
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
                tiles: vec![tile(100)],
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
    assert_eq!(result.verdict, Verdict::Unsurvivable);
    assert!(result.reason.contains("NoWayOut"));
}
