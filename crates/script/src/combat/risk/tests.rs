use super::replay::{candidate_cost, Estimate};
use super::*;
use crate::combat::tables::CombatTables;
use crate::native::WalkAllow;
use api::selected::ClientRevision;
use api::WorldTile;
use nav_planner::contract::{BoundZoneKey, NavLease, RouteView, WildernessView, ZoneTableView};
use nav_planner::router::{Leg, Route};
use nav_planner::transport::{TransportEdge, TransportKind, WildernessRules, WildernessZone};
use nav_planner::zones::{Zone, ZoneClass, ZoneKind, ZoneTable};
use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll, Waker};

/// A synthetic zone table and wilderness served through the contract views,
/// as the host serves the real pack. The lease comes from the one script
/// test site allowed to construct a world (`gatherer::select::tests`); the
/// fixture keeps a copy of the raw table only to name rows by index.
struct Fixture {
    lease: NavLease,
    table: ZoneTable,
}
impl Fixture {
    fn new(table: ZoneTable, wilderness: WildernessRules) -> Self {
        let zones = table.clone();
        let lease = crate::gatherer::select::tests::test_nav_lease(move |world| {
            world.graph.zones = Some(zones);
            world.graph.wilderness = wilderness;
        });
        Self { lease, table }
    }
    /// No zones at all: route geometry alone.
    fn empty() -> Self {
        Self::new(
            zone_table(vec![], vec![], vec![], vec![], &WildernessRules::default()),
            WildernessRules::default(),
        )
    }
    fn pack(&self) -> PackId {
        self.lease.identity().pack()
    }
    fn zones(&self) -> ZoneTableView<'_> {
        self.lease
            .zones()
            .expect("the fixture world carries a zone table")
    }
    fn wilderness(&self) -> WildernessView<'_> {
        self.lease.wilderness()
    }
    fn route<'a>(&self, route: &'a Route) -> RouteView<'a> {
        nav_planner::producer::route_view(&self.lease, route).expect("the lease identity is minted")
    }
    fn path<'a>(&self, route: &'a Route) -> RoutePath<'a> {
        RoutePath::new(self.route(route)).unwrap()
    }
    /// Bind a raw key the way the pack would: a zone row is found through
    /// the tiles it contains, a group through its catalog id.
    fn bound(&self, key: ZoneKey) -> BoundZoneKey {
        let zones = self.zones();
        match key {
            ZoneKey::Zone(index) => {
                let zone = &self.table.zones()[usize::from(index)];
                let level = i32::from(zone.level);
                (zone.min_x..=zone.max_x)
                    .flat_map(|x| (zone.min_z..=zone.max_z).map(move |z| WorldTile { x, z, level }))
                    .flat_map(|tile| zones.containing(tile))
                    .map(|view| view.key())
                    .find(|bound| bound.key() == key)
                    .expect("a zone row contains at least one of its tiles")
            }
            ZoneKey::Group(index) => zones
                .resolve(self.table.groups()[usize::from(index)].id.as_ref())
                .expect("a group resolves by its id"),
        }
    }
}
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
    let mut value = RiskInput::default();
    value.hp = hp;
    value.hp_max = cap;
    value.prayer = 1;
    value.prayer_base = 1;
    value.combat = Some(1);
    value.pos = tile(80);
    value.poison = PoisonState::Clear;
    value.set_missing_facts(false);
    value.set_input_locked(false);
    value.set_unattributed(false);
    value.set_overflow(false);
    value.food_len = 1;
    value.free_slots = 28 - food;
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
    carves: Vec<(u16, nav_planner::router::AvoidRect)>,
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
fn corridor_plan(fx: &Fixture, tables: &CombatTables, prefix: u16) -> RoutePlan {
    let ident = known_kind(tables).npc_id;
    RoutePlan {
        complete: true,
        pack: Some(fx.pack()),
        intervals: vec![ZoneInterval::new(
            fx.bound(ZoneKey::Zone(0)),
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
            None,
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
fn context<'a>(fx: &'a Fixture, risk: &'a RiskTables, tables: &'a CombatTables) -> Estimate<'a> {
    Estimate {
        zones: Some(fx.zones()),
        risks: risk,
        combat: tables,
        wilderness: fx.wilderness(),
        allow: WalkAllow::default(),
        generation: 7,
        permitted: false,
    }
}

fn poll_ready<F: Future>(future: F) -> F::Output {
    let mut future = std::pin::pin!(future);
    let mut context = Context::from_waker(Waker::noop());
    match future.as_mut().poll(&mut context) {
        Poll::Ready(output) => output,
        Poll::Pending => panic!("ready checkpoint suspended"),
    }
}

fn assert_assessments_equal(expected: &RouteAssessment, actual: &RouteAssessment) {
    assert_eq!(expected.verdict, actual.verdict);
    assert_eq!(expected.plan.complete, actual.plan.complete);
    assert_eq!(expected.plan.pack, actual.plan.pack);
    assert_eq!(expected.plan.intervals, actual.plan.intervals);
    assert_eq!(expected.plan.crossings, actual.plan.crossings);
    assert_eq!(expected.crossings, actual.crossings);
    assert_eq!(expected.more, actual.more);
    assert_eq!(expected.supplies, actual.supplies);
    assert_eq!(expected.hp_after, actual.hp_after);
    assert_eq!(expected.volley, actual.volley);
    assert_eq!(expected.bound, actual.bound);
    assert_eq!(expected.poison_test, actual.poison_test);
    assert_eq!(expected.input, actual.input);
    assert_eq!(expected.generation, actual.generation);
    assert_eq!(expected.reason, actual.reason);
}

fn assert_assessment_equivalence(
    route: RouteView<'_>,
    estimate: &Estimate<'_>,
    input: RiskInput,
) -> Arc<RouteAssessment> {
    let synchronous = assess(route, estimate, input);
    let mut checkpoint = || std::future::ready(());
    let cooperative = poll_ready(assess_cooperative(route, estimate, input, &mut checkpoint));
    assert_assessments_equal(&synchronous, &cooperative);
    synchronous
}

struct DropProbe(Arc<AtomicUsize>);

impl Drop for DropProbe {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}

struct YieldOnce {
    yielded: bool,
    probe: Option<DropProbe>,
}

impl Future for YieldOnce {
    type Output = ();

    fn poll(self: std::pin::Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        if this.yielded {
            this.probe.take();
            Poll::Ready(())
        } else {
            this.yielded = true;
            Poll::Pending
        }
    }
}

fn wolf_zone(tables: &CombatTables, spawns: &[i32], radius: u8) -> ZoneTable {
    let npc = tables.selected().npc_by_config("wolf").unwrap();
    zone_table(
        spawns
            .iter()
            .map(|spawn| Zone::npc(tile(*spawn), radius, ZoneClass::Always, u16::MAX, 0))
            .collect(),
        vec![ZoneKind::new("wolf", "Wolf", npc.id, 1, false, false)],
        vec![],
        vec![],
        &WildernessRules::default(),
    )
}

fn wolf_input(hp: u8, cap: u8, tables: &CombatTables) -> RiskInput {
    let mut value = input(hp, cap, 0, tables);
    value.defence = 40;
    value.worn_defence = [188, 202, 198, 190];
    value
}

fn schedule_interval(
    fx: &Fixture,
    zone: u16,
    ident: i32,
    spawn: i32,
    ticks: std::ops::RangeInclusive<u16>,
    max_hit: u8,
    rate: u8,
) -> ZoneInterval {
    ZoneInterval::new(
        fx.bound(ZoneKey::Zone(zone)),
        ident,
        tile(spawn),
        *ticks.start(),
        *ticks.end(),
        *ticks.start(),
        *ticks.end(),
        1,
        1,
        max_hit,
        rate,
        Style::Melee,
        None,
        false,
        false,
    )
    .unwrap()
}

fn schedule_plan(fx: &Fixture, intervals: Vec<ZoneInterval>) -> RoutePlan {
    RoutePlan {
        complete: true,
        pack: Some(fx.pack()),
        intervals: intervals.into_boxed_slice(),
        crossings: Vec::new().into_boxed_slice(),
    }
}

fn transport_entering_walk(first: i32, last: i32) -> Route {
    Route {
        legs: vec![
            Leg::Transport {
                edge: Box::new(edge(tile(80), tile(first), 5)),
            },
            Leg::Walk {
                tiles: (first..=last).map(tile).collect(),
            },
        ],
        dest: tile(last),
        ticks: 0.0,
    }
}

#[test]
fn timeline_counts_each_zone_once_per_own_rate() {
    if !api::test_support::game_data_available() {
        return;
    }
    let tables = tables();
    let risk = RiskTables::build(&tables);
    let fx = Fixture::new(wolf_zone(&tables, &[100], 1), WildernessRules::default());
    let walk = route(100, 107);
    let plan = schedule_plan(
        &fx,
        vec![schedule_interval(
            &fx,
            0,
            tables.selected().npc_by_config("wolf").unwrap().id,
            100,
            0..=7,
            6,
            6,
        )],
    );
    let value = wolf_input(40, 40, &tables);
    let mut events = odds::EventBuf::new();
    let summary = timeline::build_timeline(
        fx.path(&walk),
        &plan,
        &value,
        &tables,
        &risk,
        timeline::PoisonCharge::None,
        &mut events,
    )
    .unwrap();

    // Old: one rounded window attack; new: own-rate ticks 0 and 6.
    assert_eq!(summary.attacks, 2);
    assert_eq!(
        events
            .events()
            .iter()
            .map(|event| event.tick)
            .collect::<Vec<_>>(),
        [0, 6],
        "12 ticks at rate 6 has two attacks, not one or three",
    );
}

#[test]
fn timeline_sums_every_exposed_zone() {
    if !api::test_support::game_data_available() {
        return;
    }
    let tables = tables();
    let risk = RiskTables::build(&tables);
    let spawns: Vec<_> = (0..12).map(|index| 100 + index * 2).collect();
    let fx = Fixture::new(wolf_zone(&tables, &spawns, 1), WildernessRules::default());
    let ident = tables.selected().npc_by_config("wolf").unwrap().id;
    let plan = schedule_plan(
        &fx,
        (0..12)
            .map(|index| {
                schedule_interval(&fx, index, ident, spawns[usize::from(index)], 0..=1, 6, 4)
            })
            .collect(),
    );
    let walk = route(100, 101);
    let value = wolf_input(40, 40, &tables);
    let mut events = odds::EventBuf::new();
    let summary = timeline::build_timeline(
        fx.path(&walk),
        &plan,
        &value,
        &tables,
        &risk,
        timeline::PoisonCharge::None,
        &mut events,
    )
    .unwrap();

    // Old: a four-attacker cap discarded rows; new: all 12 zone events sum.
    assert_eq!(summary.attacks, 12, "no per-window attacker cap");
    assert_eq!(events.events().len(), 12);
    assert!(events.events().iter().all(|event| event.tick == 0));
}

#[test]
fn timeline_charges_unknown_and_npc_poison_pulses_both() {
    if !api::test_support::game_data_available() {
        return;
    }
    let tables = tables();
    let npc = tables.selected().npc_by_config("poisonspider").unwrap();
    let fx = Fixture::new(
        zone_table(
            vec![Zone::npc(tile(100), 1, ZoneClass::Always, u16::MAX, 0)],
            vec![ZoneKind::new(
                "poisonspider",
                "Poison spider",
                npc.id,
                1,
                npc.ap_attack,
                false,
            )],
            vec![],
            vec![],
            &WildernessRules::default(),
        ),
        WildernessRules::default(),
    );
    let risks = RiskTables::build(&tables);
    let kind = risks.npc(npc.id).unwrap();
    assert_eq!(kind.unknown_for(), None);
    assert!(kind.poison > 0);
    let walk = route(80, 124);
    let row = ZoneInterval::new(
        fx.bound(ZoneKey::Zone(0)),
        npc.id,
        tile(100),
        0,
        44,
        0,
        44,
        kind.r,
        kind.reach,
        kind.max_hit,
        kind.rate,
        kind.style,
        None,
        kind.ap(),
        false,
    )
    .unwrap();
    let plan = schedule_plan(&fx, vec![row]);
    let mut value = input(90, 90, 0, &tables);
    value.set_map_members(true);
    value.poison = PoisonState::Unknown { since: 0 };
    let mut events = odds::EventBuf::new();

    timeline::build_timeline(
        fx.path(&walk),
        &plan,
        &value,
        &tables,
        &risks,
        timeline::PoisonCharge::Unknown {
            q: risks.poison_unknown_v,
        },
        &mut events,
    )
    .unwrap();

    let pulses: Vec<_> = events
        .events()
        .iter()
        .filter(|event| event.kind == odds::EventKind::Poison)
        .map(|event| (event.tick, event.amount))
        .collect();
    let npc_pulse = (0, kind.poison);
    let unknown_pulse = (0, risks.poison_unknown_v);
    // Old: Unknown charge replaced active NPC poison; new: both sources pulse.
    assert_eq!(
        pulses,
        [
            npc_pulse,
            (30, kind.poison),
            (60, kind.poison),
            unknown_pulse,
            (30, risks.poison_unknown_v),
            (60, risks.poison_unknown_v),
        ],
        "the NPC onset and Unknown-state charge are both deterministic sources",
    );
}

#[test]
fn poison_charge_includes_tick_zero() {
    if !api::test_support::game_data_available() {
        return;
    }
    let tables = tables();
    let risk = RiskTables::build(&tables);
    let fx = Fixture::empty();
    let plan = schedule_plan(&fx, Vec::new());
    let mut value = input(90, 90, 0, &tables);
    value.poison = PoisonState::Unknown { since: 0 };

    // Old: the first pulse waited a period; new: charge starts at route tick 0.
    for (walk, expected_ticks) in [(route(10, 28), vec![0]), (route(10, 29), vec![0, 30])] {
        let mut events = odds::EventBuf::new();
        timeline::build_timeline(
            fx.path(&walk),
            &plan,
            &value,
            &tables,
            &risk,
            timeline::PoisonCharge::Unknown { q: 11 },
            &mut events,
        )
        .unwrap();
        assert_eq!(
            events
                .events()
                .iter()
                .filter(|event| event.kind == odds::EventKind::Poison)
                .map(|event| event.tick)
                .collect::<Vec<_>>(),
            expected_ticks,
        );
    }
}

#[test]
fn admission_wolf_crossing_admits_at_base_40_without_food() {
    if !api::test_support::game_data_available() {
        return;
    }
    let tables = tables();
    let fx = Fixture::new(wolf_zone(&tables, &[100], 1), WildernessRules::default());
    let risk = RiskTables::build(&tables);
    let walk = route(90, 108);
    let cx = context(&fx, &risk, &tables);
    let assessment =
        assert_assessment_equivalence(fx.route(&walk), &cx, wolf_input(40, 40, &tables));

    // Old: this route required a food fix; new: its no-food bound admits.
    assert_eq!(
        assessment.verdict,
        Verdict::Survivable,
        "{}",
        assessment.reason
    );
    assert_eq!(assessment.input.food_counts[0], 0);
    assert!(assessment.bound <= consts::DEATH_BOUND);
    assert_eq!(assessment.hp_after, 40);
}

#[test]
fn admission_no_grants_passes_through_estimate() {
    if !api::test_support::game_data_available() {
        return;
    }
    let tables = tables();
    let fx = Fixture::new(wolf_zone(&tables, &[100], 1), WildernessRules::default());
    let risk = RiskTables::build(&tables);
    let cx = context(&fx, &risk, &tables);
    assert!(!cx.permitted);

    let walk = route(90, 108);
    let assessment = assess(fx.route(&walk), &cx, wolf_input(40, 40, &tables));
    // Old: no grant value was carried; new: None keeps the full risk check.
    assert_eq!(assessment.verdict, Verdict::Survivable);
    assert!(assessment.plan.complete);
}

#[test]
fn admission_granted_unsurvivable_crossing_is_refused() {
    if !api::test_support::game_data_available() {
        return;
    }
    let tables = tables();
    let fx = Fixture::new(wolf_zone(&tables, &[100], 1), WildernessRules::default());
    let risk = RiskTables::build(&tables);
    let walk = route(90, 108);
    let mut cx = context(&fx, &risk, &tables);
    cx.permitted = true;

    // Permission never bypasses the death bound; it only waives NoWayOut.
    let assessment = assess(fx.route(&walk), &cx, input(1, 1, 0, &tables));
    assert_eq!(assessment.verdict, Verdict::Unsurvivable);
    assert!(assessment.bound > consts::DEATH_BOUND);
}

#[test]
fn admission_declared_no_way_out_endpoint_is_judged_on_the_walk() {
    if !api::test_support::game_data_available() {
        return;
    }
    let tables = tables();
    let fx = Fixture::new(one_zone(&tables, 100, 1), WildernessRules::default());
    let risk = RiskTables::build(&tables);
    let walk = transport_entering_walk(100, 101);
    let assessment = assess(
        fx.route(&walk),
        &context(&fx, &risk, &tables),
        input(90, 90, 0, &tables),
    );

    // Old: endpoint arrival could bypass the crossing; new: judge the walk.
    assert_eq!(assessment.plan.crossings.len(), 1);
    assert!(!assessment.plan.crossings[0].has_way_out());
    assert_eq!(assessment.verdict, Verdict::Unsurvivable);
    assert!(assessment.reason.contains("NoWayOut"));
}

#[test]
fn admission_permitted_walk_waives_no_way_out_but_not_the_death_bound() {
    if !api::test_support::game_data_available() {
        return;
    }
    let tables = tables();
    let fx = Fixture::new(one_zone(&tables, 100, 1), WildernessRules::default());
    let risk = RiskTables::build(&tables);
    let walk = transport_entering_walk(100, 101);
    let mut cx = context(&fx, &risk, &tables);
    cx.permitted = true;

    let covered = assess(fx.route(&walk), &cx, input(90, 90, 0, &tables));
    assert_eq!(covered.plan.crossings.len(), 1);
    assert!(!covered.plan.crossings[0].has_way_out());
    assert_eq!(covered.verdict, Verdict::Survivable, "{}", covered.reason);
    assert!(!covered.reason.contains("NoWayOut"));

    let lethal = assess(fx.route(&walk), &cx, input(1, 1, 0, &tables));
    assert_eq!(lethal.verdict, Verdict::Unsurvivable);
    assert!(lethal.bound > consts::DEATH_BOUND);
    assert!(!lethal.reason.contains("NoWayOut"));
}

#[test]
fn admission_permission_never_bypasses_unknown_facts() {
    if !api::test_support::game_data_available() {
        return;
    }
    let tables = tables();
    let missing_id = (0..=i32::from(i16::MAX))
        .find(|id| tables.npc(*id).is_none())
        .expect("the selected table has an unused compact NPC id");
    let w = WildernessRules::default();
    let fx = Fixture::new(
        zone_table(
            vec![Zone::npc(tile(100), 1, ZoneClass::Always, u16::MAX, 0)],
            vec![ZoneKind::new(
                "missing",
                "Missing combat facts",
                missing_id,
                1,
                false,
                false,
            )],
            vec![],
            vec![],
            &w,
        ),
        w,
    );
    let risk = RiskTables::build(&tables);
    let mut cx = context(&fx, &risk, &tables);
    cx.permitted = true;
    let walk = transport_entering_walk(100, 101);
    let assessment = assess(fx.route(&walk), &cx, input(90, 90, 0, &tables));

    // Permission only waives NoWayOut; unknown facts still refuse.
    assert_eq!(
        assessment.verdict,
        Verdict::Unknown(UnknownWhy::MissingFacts)
    );
    assert!(!assessment.reason.contains("NoWayOut"));
}

#[test]
fn admission_credits_no_protect() {
    if !api::test_support::game_data_available() {
        return;
    }
    let tables = tables();
    let fx = Fixture::new(wolf_zone(&tables, &[100], 1), WildernessRules::default());
    let risk = RiskTables::build(&tables);
    let walk = route(90, 108);
    let cx = context(&fx, &risk, &tables);
    let unprotected = assess(fx.route(&walk), &cx, wolf_input(40, 40, &tables));
    let mut protected_input = wolf_input(40, 40, &tables);
    protected_input.active_protect = Style::Melee;
    let protected = assess(fx.route(&walk), &cx, protected_input);

    // Old: active Protect lowered risk; new: the bound is unchanged.
    assert_eq!(unprotected.verdict, protected.verdict);
    assert_eq!(unprotected.bound, protected.bound);
}

#[test]
fn crossing_display_volley_saturates_u8() {
    if !api::test_support::game_data_available() {
        return;
    }
    let tables = tables();
    let fx = Fixture::new(
        wolf_zone(&tables, &[100, 101], 1),
        WildernessRules::default(),
    );
    let mut risk = RiskTables::build(&tables);
    let wolf = tables.selected().npc_by_config("wolf").unwrap().id;
    risk.npc_mut(wolf).unwrap().max_hit = 200;
    let walk = route(90, 108);
    let assessment = assess(
        fx.route(&walk),
        &context(&fx, &risk, &tables),
        wolf_input(40, 40, &tables),
    );

    // Old: display overflow became Unknown(Overflow); new: u8 output saturates.
    assert_eq!(assessment.plan.intervals.len(), 2);
    assert_eq!(assessment.crossings[0].volley, u8::MAX);
    assert_eq!(assessment.volley, u8::MAX);
}

#[test]
fn admission_reason_states_an_upper_bound() {
    if !api::test_support::game_data_available() {
        return;
    }
    let tables = tables();
    let fx = Fixture::new(wolf_zone(&tables, &[100], 1), WildernessRules::default());
    let risk = RiskTables::build(&tables);
    let walk = route(90, 108);
    let assessment = assess(
        fx.route(&walk),
        &context(&fx, &risk, &tables),
        input(1, 1, 0, &tables),
    );

    // Old: the reason claimed a lower bound; new: state the conservative upper bound.
    assert_eq!(assessment.verdict, Verdict::Unsurvivable);
    assert!(assessment.reason.contains("upper bound"));
    assert!(!assessment.reason.contains("at least"));
}

#[test]
fn verdict_fixable_with_is_gone() {
    // Exhaustive matching is intentional: the old FixableWith verdict has no
    // remaining risk-core or host-admission representation.
    let is_terminal = |verdict| match verdict {
        Verdict::Survivable | Verdict::Unsurvivable | Verdict::Unknown(_) => true,
    };
    assert!(is_terminal(Verdict::Survivable));
}

#[test]
fn poison_charged_pass_admits_at_once() {
    if !api::test_support::game_data_available() {
        return;
    }
    let tables = tables();
    let fx = Fixture::new(wolf_zone(&tables, &[100], 1), WildernessRules::default());
    let risk = RiskTables::build(&tables);
    let walk = route(90, 108);
    let mut value = wolf_input(40, 40, &tables);
    value.poison = PoisonState::Unknown { since: 0 };
    let assessment = assess(fx.route(&walk), &context(&fx, &risk, &tables), value);

    // Old: Unknown always started a hold; new: a passing charged bound admits now.
    assert_eq!(
        assessment.verdict,
        Verdict::Survivable,
        "{}",
        assessment.reason
    );
    assert!(matches!(
        assessment.poison_test,
        PoisonTest::PassesCharged { charged } if charged <= consts::DEATH_BOUND
    ));
}

#[test]
fn poison_charged_fail_clear_pass_holds_at_retreat() {
    if !api::test_support::game_data_available() {
        return;
    }
    let tables = tables();
    let fx = Fixture::new(one_zone(&tables, 100, 1), WildernessRules::default());
    let risk = RiskTables::build(&tables);
    let walk = route(80, 252);
    let mut value = input(90, 90, 0, &tables);
    value.poison = PoisonState::Unknown { since: 0 };
    let assessment = assess(fx.route(&walk), &context(&fx, &risk, &tables), value);

    // Old: Unknown was refused before reassessment; new: wait at the safe retreat.
    assert_eq!(
        assessment.verdict,
        Verdict::Survivable,
        "{}",
        assessment.reason
    );
    let retreat = assessment.plan.crossings[0].retreat;
    assert_ne!(retreat, CrossingGeom::NONE);
    assert!(assessment.bound <= consts::DEATH_BOUND);
    assert!(matches!(
        assessment.poison_test,
        PoisonTest::WaitForClear {
            charged,
            until_index,
        } if charged > consts::DEATH_BOUND && until_index == retreat
    ));
}

#[test]
fn poison_no_safe_tile_admits_on_clear_bound() {
    if !api::test_support::game_data_available() {
        return;
    }
    let tables = tables();
    let fx = Fixture::new(one_zone(&tables, 100, 1), WildernessRules::default());
    let risk = RiskTables::build(&tables);
    let walk = transport_entering_walk(100, 300);
    let mut value = input(90, 90, 0, &tables);
    value.poison = PoisonState::Unknown { since: 0 };
    let assessment = assess(fx.route(&walk), &context(&fx, &risk, &tables), value);

    // Old: Unknown alone refused a no-retreat route; new: admit on the clear bound.
    assert_eq!(
        assessment.verdict,
        Verdict::Survivable,
        "{}",
        assessment.reason
    );
    assert_eq!(assessment.plan.crossings[0].retreat, CrossingGeom::NONE);
    assert!(assessment.bound <= consts::DEATH_BOUND);
    assert!(matches!(
        assessment.poison_test,
        PoisonTest::NoSafeTile { charged } if charged > consts::DEATH_BOUND
    ));
    assert!(assessment.reason.contains("no tile to wait on"));
    let mut low_hp = input(1, 1, 0, &tables);
    low_hp.poison = PoisonState::Unknown { since: 0 };
    let refused = assess(fx.route(&walk), &context(&fx, &risk, &tables), low_hp);
    // Old: Unknown itself determined the refusal; new: the clear bound must fail.
    assert_eq!(refused.verdict, Verdict::Unsurvivable);
    assert!(refused.bound > consts::DEATH_BOUND);
    assert_eq!(refused.poison_test, PoisonTest::NotNeeded);
    assert!(!refused.reason.contains("no tile to wait on"));
}

#[test]
fn poison_facts_missing_is_unknown() {
    if !api::test_support::game_data_available() {
        return;
    }
    let tables = tables();
    let fx = Fixture::new(one_zone(&tables, 100, 1), WildernessRules::default());
    let mut risk = RiskTables::build(&tables);
    risk.poison_unknown_v = u8::MAX;
    let walk = route(88, 112);
    let mut value = input(90, 90, 0, &tables);
    value.poison = PoisonState::Unknown { since: 0 };
    let assessment = assess(fx.route(&walk), &context(&fx, &risk, &tables), value);

    // Old: missing poison data could understate risk; new: invalid q fails closed.
    assert_eq!(assessment.verdict, Verdict::Unknown(UnknownWhy::Poison));
}

#[test]
fn poison_zero_crossings_never_waits() {
    if !api::test_support::game_data_available() {
        return;
    }
    let assessment = assess_poison_state(PoisonState::Unknown { since: 0 }, false);

    // Old: Unknown alone blocked all routes; new: zero crossings need no hold.
    assert!(assessment.plan.crossings.is_empty());
    assert_eq!(assessment.verdict, Verdict::Survivable);
    assert_eq!(assessment.poison_test, PoisonTest::NotNeeded);
}

pub(super) fn assess_poison_state(poison: PoisonState, crossing: bool) -> Arc<RouteAssessment> {
    let tables = tables();
    let fx = Fixture::new(one_zone(&tables, 100, 1), WildernessRules::default());
    let risks = RiskTables::build(&tables);
    let mut value = input(90, 90, 0, &tables);
    value.poison = poison;
    let walk = if crossing {
        route(88, 112)
    } else {
        route(10, 20)
    };
    assess(fx.route(&walk), &context(&fx, &risks, &tables), value)
}

fn unattributed_onset() -> PoisonState {
    PoisonState::Onset {
        since: 0,
        q: None,
        hit_since_line: false,
    }
}

#[test]
fn replay_onset_charges_unknown_pulse() {
    if !api::test_support::game_data_available() {
        return;
    }
    let tables = tables();
    let fx = Fixture::empty();
    let walk = route(10, 20);
    let mut value = input(90, 90, 0, &tables);
    value.poison = unattributed_onset();

    let result = super::replay::replay(
        fx.path(&walk),
        &RoutePlan::default(),
        &value,
        &tables,
        WalkAllow::default(),
        None,
        |_, _, _, _, _| {},
    )
    .unwrap();
    assert_eq!(result.hp_after, 79);
}

#[test]
fn replay_hits_charges_onset_per_period() {
    if !api::test_support::game_data_available() {
        return;
    }
    let tables = tables();
    let fx = Fixture::empty();
    let walk = route(10, 132);
    let mut value = input(90, 90, 0, &tables);
    value.poison = unattributed_onset();

    let result = super::replay::replay(
        fx.path(&walk),
        &RoutePlan::default(),
        &value,
        &tables,
        WalkAllow::default(),
        None,
        |_, _, _, _, _| {},
    )
    .unwrap();
    // 123 tiles take 185 bounded replay ticks: fires at 0, 30, ..., 180.
    assert!(result.passed);
    assert_eq!(result.hp_after, 90 - 7 * 11);
}

#[test]
fn replay_early_pass_refuses_onset() {
    if !api::test_support::game_data_available() {
        return;
    }
    let tables = tables();
    let fx = Fixture::empty();
    let walk = route(10, 20);
    let mut value = input(5, 5, 0, &tables);
    value.poison = unattributed_onset();

    let result = super::replay::replay(
        fx.path(&walk),
        &RoutePlan::default(),
        &value,
        &tables,
        WalkAllow::default(),
        None,
        |_, _, _, _, _| {},
    )
    .unwrap();
    assert!(!result.passed);
    assert_eq!(result.failed_tick, Some(0));
}

/// A no-crossing route skips only the Unknown-poison wait, not lethal observed Onset damage.
#[test]
fn replay_no_crossing_walk_refuses_lethal_onset_without_food_credit() {
    if !api::test_support::game_data_available() {
        return;
    }
    let tables = tables();
    let fx = Fixture::empty();
    let risks = RiskTables::build(&tables);
    let estimate = context(&fx, &risks, &tables);
    let short_walk = route(10, 20);
    let mut no_food = input(5, 5, 0, &tables);
    no_food.poison = unattributed_onset();
    let assessment = assess(fx.route(&short_walk), &estimate, no_food);
    assert!(assessment.plan.crossings.is_empty());
    assert_eq!(assessment.verdict, Verdict::Survivable);

    let long_walk = route(10, 252);
    let mut food_is_not_credited = input(20, 30, 1, &tables);
    food_is_not_credited.poison = unattributed_onset();
    let assessment = assess(fx.route(&long_walk), &estimate, food_is_not_credited);
    assert!(assessment.plan.crossings.is_empty());
    // Old: Survivable from a no-crossing exemption. New: Onset is known, so its lethal pulses count.
    assert_eq!(assessment.verdict, Verdict::Unsurvivable);
    assert!(assessment.bound > consts::DEATH_BOUND);
    assert_eq!(assessment.hp_after, 20);
    assert!(assessment
        .supplies
        .iter()
        .all(|need| !matches!(need, SupplyNeed::Food { .. })));
}

#[test]
fn replay_refreshed_poisoned_charges_max_pulse() {
    if !api::test_support::game_data_available() {
        return;
    }
    let tables = tables();
    let fx = Fixture::empty();
    let walk = route(10, 20);
    let mut value = input(90, 90, 0, &tables);
    value.poison = PoisonState::Poisoned {
        per_tick: 3,
        last_mark: 0,
        hit_since_mark: true,
    };

    let result = super::replay::replay(
        fx.path(&walk),
        &RoutePlan::default(),
        &value,
        &tables,
        WalkAllow::default(),
        None,
        |_, _, _, _, _| {},
    )
    .unwrap();
    assert_eq!(result.hp_after, 79);
    assert_eq!(
        eat_line(
            fx.path(&walk),
            &RoutePlan::default(),
            0,
            &value,
            &tables,
            WalkAllow::default()
        ),
        Ok(23),
        "a refreshed no-crossing floor uses the strongest pulse"
    );
}

#[test]
fn u9_compact_layout_and_limit_round_trips() {
    if !api::test_support::game_data_available() {
        return;
    }
    assert!(std::mem::size_of::<RiskInput>() <= 128);
    assert!(std::mem::size_of::<PoisonState>() <= 8);
    assert!(std::mem::size_of::<KindRisk>() <= 12);
    // T5.1: the retained zone identity is a pack-bound key (16 bytes), so a
    // row is 40 bytes, up from 24.
    assert!(std::mem::size_of::<ZoneInterval>() <= 40);
    assert!(std::mem::size_of::<Crossing>() <= 32);
    let fx = Fixture::new(one_zone(&tables(), 100, 1), WildernessRules::default());
    let zone = fx.bound(ZoneKey::Zone(0));
    for x in [0, 16383] {
        for z in [0, 16383] {
            for level in [0, 3] {
                let point = WorldTile { x, z, level };
                assert_eq!(PackedTile::new(point).unwrap().tile(), point);
                for ident in [-1, i32::from(i16::MAX)] {
                    let row = ZoneInterval::new(
                        zone,
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
                        Some(UnknownKind::Overflow),
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
            zone,
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
            None,
            false,
            false
        ),
        Err(UnknownWhy::Overflow)
    );
    assert_eq!(
        ZoneInterval::new(
            zone,
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
            None,
            false,
            false
        ),
        Err(UnknownWhy::Overflow)
    );
    assert_eq!(
        ZoneInterval::new(
            zone,
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
            None,
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
    if !api::test_support::game_data_available() {
        return;
    }
    let tables = tables();
    let wilderness = WildernessRules::default();
    for (config, class) in [
        ("shadow_spider", UnknownKind::Bespoke),
        ("red_dragon", UnknownKind::Dragonfire),
        ("trail_hard2", UnknownKind::CounterProtect),
        ("ikov_firewarrior", UnknownKind::Magic),
        ("wizard", UnknownKind::Mixed),
    ] {
        let npc = tables.selected().npc_by_config(config).unwrap();
        let fx = Fixture::new(
            zone_table(
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
            ),
            wilderness.clone(),
        );
        let risks = RiskTables::build(&tables);
        let value = input(90, 90, 10, &tables);
        let assessment = assess(
            fx.route(&route(88, 112)),
            &context(&fx, &risks, &tables),
            value,
        );
        assert_eq!(
            assessment.verdict,
            Verdict::Unknown(UnknownWhy::Kind(class)),
            "{config}"
        );
        assert!(assessment.reason.contains(config), "{config}");
        assert!(
            assessment.reason.contains(&format!("{class:?}")),
            "{config}"
        );
    }
    let poison = tables.selected().npc_by_config("poisonspider").unwrap();
    let poison_risk = KindRisk::from_npc(poison, &tables);
    assert_eq!(poison_risk.unknown, UnknownKind::Known);
    assert_eq!(poison_risk.unknown_for(), None);

    // No strength= line: the engine default is 1 (NpcType.ts:106). Max hit 1
    // is select.rs:204 at strength 1 and strengthbonus 0. No packed 289 row
    // still lacks the stat after that default, so this test no longer has a
    // real-table MissingStat row.
    let man = tables.selected().npc_by_config("man").unwrap();
    assert_eq!(man.strength, Some(1));
    let man_risk = KindRisk::from_npc(man, &tables);
    assert_eq!(man_risk.unknown, UnknownKind::Known);
    assert_eq!(man_risk.max_hit, 1);
    assert_eq!(crate::combat::select::facts::npc_max_hit(man), Some(1));
}

/// T5.1 representation closure: risk reads each zone's facts through that
/// zone's facade kind view, keyed by NPC id, never through the pack's kind
/// rows or their order. Here kind rows are stored in an order unrelated to
/// their zones, one kind serves three zones, and NPC, magic, mixed,
/// curated-hazard, uncurated-hazard and missing-facts kinds all appear, so a
/// lookup by any other row or kind index misclassifies some zone.
#[test]
fn t5_1_each_zone_classifies_as_the_kind_its_pack_row_names() {
    if !api::test_support::game_data_available() {
        return;
    }
    let tables = tables();
    let npc = |config: &str| tables.selected().npc_by_config(config).unwrap();
    let kind = |id: &str, npc_id: i32| ZoneKind::new(id, id, npc_id, 1, false, false);
    let hazard = |max_hit, unknown| {
        KindRisk::new(
            max_hit,
            1,
            0,
            0,
            Style::Unknown,
            unknown,
            0,
            0,
            false,
            false,
            false,
            true,
        )
    };
    let ice = KindRisk::from_npc(npc("icewarrior"), &tables);
    let magic = KindRisk::from_npc(npc("ikov_firewarrior"), &tables);
    let wizard = KindRisk::from_npc(npc("wizard"), &tables);
    let missing = KindRisk::new(
        0,
        0,
        0,
        0,
        Style::Unknown,
        UnknownKind::MissingFacts,
        0,
        0,
        false,
        false,
        false,
        false,
    );
    let kinds = vec![
        kind("ikov-firewarrior", npc("ikov_firewarrior").id),
        kind("ikov-lava-bridge", -1),
        kind("uncurated-hazard", -1),
        kind("wizard", npc("wizard").id),
        kind("missing-facts", i32::from(i16::MAX)),
        known_kind(&tables),
    ];
    // (kind row of zone row i, its class and facts)
    let rows = [
        (5, None, ice),
        (0, Some(UnknownKind::Magic), magic),
        (5, None, ice),
        (3, Some(UnknownKind::Mixed), wizard),
        (1, None, hazard(20, UnknownKind::Known)),
        (
            2,
            Some(UnknownKind::HazardDamage),
            hazard(0, UnknownKind::HazardDamage),
        ),
        (4, Some(UnknownKind::MissingFacts), missing),
        (5, None, ice),
    ];
    let w = WildernessRules::default();
    let fx = Fixture::new(
        zone_table(
            rows.iter()
                .enumerate()
                .map(|(row, (kind, _, _))| {
                    Zone::npc(
                        tile(10 + 30 * row as i32),
                        1,
                        ZoneClass::Always,
                        u16::MAX,
                        *kind,
                    )
                })
                .collect(),
            kinds,
            vec![],
            vec![],
            &w,
        ),
        w,
    );
    let risks = RiskTables::build(&tables);
    let zones = fx.zones();
    for (row, (kind, class, facts)) in rows.iter().enumerate() {
        let view = zones.zone(fx.bound(ZoneKey::Zone(row as u16))).unwrap();
        assert_eq!(
            view.kind().id(),
            fx.table.kinds()[usize::from(*kind)].id.as_ref()
        );
        let read = risks.zone_kind(view.kind());
        assert_eq!(read, *facts, "zone row {row}");
        assert_eq!(read.unknown_for(), *class, "zone row {row}");
    }

    let walk = route(0, 10 + 30 * rows.len() as i32);
    let mut value = input(99, 99, 0, &tables);
    value.set_map_members(true);
    let plan = build_plan(fx.path(&walk), zones, &risks, fx.wilderness(), &value).unwrap();
    let mut seen = [false; 8];
    for interval in plan.intervals.iter() {
        let ZoneKey::Zone(row) = interval.zone.key() else {
            panic!("an interval names a zone row");
        };
        let (_, class, facts) = rows[usize::from(row)];
        assert_eq!(interval.unknown_kind(), class, "zone row {row}");
        assert_eq!(
            (
                interval.max_hit,
                interval.rate,
                interval.r,
                interval.reach,
                interval.style,
                interval.ap(),
                interval.hazard(),
            ),
            (
                facts.max_hit,
                facts.rate,
                facts.r,
                facts.reach,
                facts.style,
                facts.ap(),
                facts.hazard(),
            ),
            "zone row {row}"
        );
        seen[usize::from(row)] = true;
    }
    assert!(seen.iter().all(|seen| *seen), "every zone is crossed");

    let cx = context(&fx, &risks, &tables);
    let assessment = assess(fx.route(&walk), &cx, value);
    assert_eq!(
        assessment.verdict,
        Verdict::Unknown(UnknownWhy::Kind(UnknownKind::Magic)),
        "the first unsupported zone on the walk is the magic firewarrior"
    );
    assert!(
        assessment
            .reason
            .contains("Route crosses Ice warrior at 10,100"),
        "{}",
        assessment.reason
    );
}

#[test]
fn u3_c4_real_assessment_sums_live_and_estimated_rows_without_scene_coverage() {
    if !api::test_support::game_data_available() {
        return;
    }
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
        let fx = Fixture::new(
            zone_table(
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
            ),
            wilderness.clone(),
        );
        let mut risks = RiskTables::build(&tables);
        if estimated > 255 {
            // A synthetic broad tether makes all 300 distinct spawns persist
            // beyond the 255-row view, without invalid acquisition geometry.
            risks.npc_mut(known_kind(&tables).npc_id).unwrap().r = 64;
        }
        let mut value = input(99, 99, 0, &tables);
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
            fx.route(&route(if estimated > 255 { 10 } else { 80 }, 120)),
            &context(&fx, &risks, &tables),
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
    if !api::test_support::game_data_available() {
        return;
    }
    let tables = tables();
    let fx = Fixture::new(one_zone(&tables, 120, 1), WildernessRules::default());
    let route = route(80, 108);
    let path = fx.path(&route);
    let plan = corridor_plan(&fx, &tables, 1);
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
    if !api::test_support::game_data_available() {
        return;
    }
    let tables = tables();
    let fx = Fixture::new(one_zone(&tables, 120, 1), WildernessRules::default());
    let route = route(80, 127);
    let path = fx.path(&route);
    let plan = corridor_plan(&fx, &tables, 20);
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
    if !api::test_support::game_data_available() {
        return;
    }
    let tables = tables();
    let fx = Fixture::new(one_zone(&tables, 100, 1), WildernessRules::default());
    let risk = RiskTables::build(&tables);
    let route = route(88, 112);
    let cx = context(&fx, &risk, &tables);
    let mut good = input(90, 90, 0, &tables);
    good.prayer_base = 99;
    good.prayer = 99;
    let protected = assess(fx.route(&route), &cx, good);
    assert!(protected.crossings[0].protect_credited);
    assert_eq!(protected.hp_after, 90);
    for which in 0..5 {
        let mut changed = good;
        let mut allow = WalkAllow::default();
        match which {
            0 => allow.prayer = false,
            1 => changed.prayer_base = 1,
            2 => changed.prayer = 1,
            3 => changed.set_other_prayers_on(true),
            _ => changed.set_off_debt(true),
        }
        let c = Estimate { allow, ..cx };
        let actual = assess(fx.route(&route), &c, changed);
        assert!(!actual.crossings[0].protect_credited, "condition {which}");
    }
    let required = (fx.path(&route).ticks()
        - fx.path(&route)
            .point(protected.plan.crossings[0].env_first)
            .unwrap()
            .tick
        + 8)
        / 5
        + 1;
    good.prayer = (required - 1) as u8;
    good.doses_prayer = 20;
    assert!(
        !assess(fx.route(&route), &cx, good).crossings[0].protect_credited,
        "potions never substitute for a missing current point"
    );
}

#[test]
fn u4_u14_poison_through_safe_gap_and_no_crossing_exemption() {
    if !api::test_support::game_data_available() {
        return;
    }
    let tables = tables();
    let fx = Fixture::new(one_zone(&tables, 120, 1), WildernessRules::default());
    let risk = RiskTables::build(&tables);
    let cx = context(&fx, &risk, &tables);
    let route = route(80, 145);
    let path = fx.path(&route);
    let plan = build_plan(
        path,
        fx.zones(),
        &risk,
        fx.wilderness(),
        &input(90, 90, 0, &tables),
    )
    .unwrap();
    let mut poisoned = input(90, 90, 0, &tables);
    poisoned.poison = PoisonState::Poisoned {
        per_tick: 1,
        last_mark: 0,
        hit_since_mark: false,
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
    let safe_unknown = assert_assessment_equivalence(fx.route(&safe), &cx, unknown);
    assert_eq!(safe_unknown.verdict, Verdict::Survivable);
    let unknown_assessment = assert_assessment_equivalence(fx.route(&route), &cx, unknown);
    assert_eq!(unknown_assessment.verdict, Verdict::Survivable);
    assert!(matches!(
        unknown_assessment.poison_test,
        PoisonTest::PassesCharged { charged } if charged <= consts::DEATH_BOUND
    ));
    assert!(unknown_assessment
        .reason
        .contains("unknown poison charged bound"));
    let poisoned_assessment = assert_assessment_equivalence(fx.route(&safe), &cx, poisoned);
    assert_eq!(poisoned_assessment.input.poison, poisoned.poison);
}

#[test]
fn u4_u14_no_crossing_poison_refuses_lethal_bound() {
    if !api::test_support::game_data_available() {
        return;
    }
    let tables = tables();
    let fx = Fixture::new(one_zone(&tables, 120, 1), WildernessRules::default());
    let risks = RiskTables::build(&tables);
    let cx = context(&fx, &risks, &tables);
    let safe = route(10, 30);
    let mut poisoned = input(10, 40, 0, &tables);
    poisoned.poison = PoisonState::Poisoned {
        per_tick: 11,
        last_mark: 0,
        hit_since_mark: false,
    };
    let assessment = assess(fx.route(&safe), &cx, poisoned);
    assert!(assessment.plan.crossings.is_empty());
    // An empty crossing list cannot turn a lethal poison bound into safety.
    assert_eq!(assessment.verdict, Verdict::Unsurvivable);
    assert_eq!(assessment.hp_after, poisoned.hp);
    assert!(assessment.bound > consts::DEATH_BOUND);
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
    if !api::test_support::game_data_available() {
        return;
    }
    let tables = tables();
    let fx = Fixture::new(one_zone(&tables, 120, 1), WildernessRules::default());
    let risks = RiskTables::build(&tables);
    let cx = context(&fx, &risks, &tables);
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
    let assessment = assess(fx.route(&short), &cx, value);
    assert!(assessment.plan.crossings.is_empty());
    assert_eq!(
        (assessment.verdict, assessment.hp_after, assessment.volley),
        (Verdict::Survivable, 90, 6),
        "assessment retains HP input and reports origin volley",
    );
    let mut hits = Vec::new();
    replay(
        fx.path(&short),
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
    let mut low_hp = value;
    low_hp.hp = 20;
    low_hp.hp_max = 40;
    let low_hp_assessment = assess(fx.route(&short), &cx, low_hp);
    assert_eq!(low_hp_assessment.verdict, Verdict::Unsurvivable);
    assert_eq!(low_hp_assessment.hp_after, 20);
    assert!(low_hp_assessment.bound > consts::DEATH_BOUND);
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
        forbidden_held: Vec::new(),
        forbidden_worn: Vec::new(),
        members_req: false,
        wildy_cap: None,
        quest_gates: None,
    }
}

#[test]
fn r1_m2_mid_crossing_transport_hold_never_credits_food() {
    if !api::test_support::game_data_available() {
        return;
    }
    let tables = tables();
    let fx = Fixture::new(one_zone(&tables, 100, 6), WildernessRules::default());
    let risks = RiskTables::build(&tables);
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
    let path = fx.path(&walk);
    let mut value = input(40, 40, 6, &tables);
    value.pos = tile(85);
    let cx = context(&fx, &risks, &tables);
    let assessment = assess(fx.route(&walk), &cx, value);
    let mut no_food = input(40, 40, 0, &tables);
    no_food.pos = tile(85);
    let no_food_assessment = assess(fx.route(&walk), &cx, no_food);
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
        "the max-hit replay still fails despite the probabilistic no-food admission"
    );
    assert_eq!(
        (assessment.verdict, assessment.bound),
        (no_food_assessment.verdict, no_food_assessment.bound),
        "inventory must not change P1-C's no-food bound"
    );
    assert_eq!(assessment.verdict, Verdict::Survivable);
}

#[test]
fn r1_m2_pending_heal_waits_until_the_transport_delay_ends() {
    if !api::test_support::game_data_available() {
        return;
    }
    let tables = tables();
    let fx = Fixture::new(one_zone(&tables, 500, 1), WildernessRules::default());
    let risks = RiskTables::build(&tables);
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
    let path = fx.path(&walk);
    let mut value = input(10, 40, 2, &tables);
    value.poison = PoisonState::Poisoned {
        per_tick: 5,
        last_mark: 0,
        hit_since_mark: false,
    };
    let plan = build_plan(path, fx.zones(), &risks, fx.wilderness(), &value).unwrap();
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
    if !api::test_support::game_data_available() {
        return;
    }
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
    let carve = nav_planner::router::AvoidRect {
        min_x: 98,
        max_x: 99,
        min_z: 98,
        max_z: 102,
        level: Some(0),
    };
    let fx = Fixture::new(
        zone_table(vec![zone], kinds, vec![(0, carve)], vec![], &wilderness),
        wilderness,
    );
    let risk = RiskTables::build(&tables);
    let mut value = input(90, 90, 0, &tables);
    value.combat = Some(126);
    let route = route(90, 115);
    let shaped_assessment =
        assert_assessment_equivalence(fx.route(&route), &context(&fx, &risk, &tables), value);
    assert!(shaped_assessment.plan.complete);
    let path = fx.path(&route);
    let plan = build_plan(path, fx.zones(), &risk, fx.wilderness(), &value).unwrap();
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
        fx.path(&envelope_only),
        fx.zones(),
        &risk,
        fx.wilderness(),
        &value
    )
    .unwrap()
    .intervals
    .is_empty());
    let origin = self::route(100, 115);
    let plan = build_plan(fx.path(&origin), fx.zones(), &risk, fx.wilderness(), &value).unwrap();
    assert!(plan.complete);
    assert!(
        plan.crossings.is_empty(),
        "origin-to-exit is escape exposure"
    );
    assert!(plan.intervals.iter().any(|row| row.escaping()));
    let fx = Fixture::new(one_zone(&tables, 100, 1), WildernessRules::default());
    let r = RiskTables::build(&tables);
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
    let assessed = assert_assessment_equivalence(
        fx.route(&ladder),
        &context(&fx, &r, &tables),
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
    assert_eq!(fx.path(&taking_off).point(0).unwrap().duration, 9);
    let fx = Fixture::new(
        zone_table(
            (0..9)
                .map(|i| Zone::npc(tile(100 + i * 40), 1, ZoneClass::Always, u16::MAX, 0))
                .collect(),
            vec![known_kind(&tables)],
            vec![],
            vec![],
            &WildernessRules::default(),
        ),
        WildernessRules::default(),
    );
    let risk = RiskTables::build(&tables);
    let route = self::route(80, 450);
    let mut value = input(99, 99, 0, &tables);
    value.prayer_base = 99;
    value.prayer = 255;
    let assessed =
        assert_assessment_equivalence(fx.route(&route), &context(&fx, &risk, &tables), value);
    assert_eq!(assessed.plan.crossings.len(), 9);
    assert_eq!(assessed.crossings.len(), 8);
    assert_eq!(assessed.more, 1);
}

#[test]
fn u1_u3_four_zone_crossing_keeps_distinct_ranges_rates_and_summed_hits() {
    if !api::test_support::game_data_available() {
        return;
    }
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
    let fx = Fixture::new(
        zone_table(
            (0..4)
                .map(|i| Zone::npc(tile(100 + i), 1, ZoneClass::Always, u16::MAX, i as u16))
                .collect(),
            kinds,
            vec![],
            vec![],
            &w,
        ),
        w.clone(),
    );
    let mut risk = RiskTables::build(&tables);
    for (i, (hit, rate)) in [(2, 1), (3, 4), (4, 5), (5, 5)].into_iter().enumerate() {
        *risk.npc_mut(fx.table.kinds()[i].npc_id).unwrap() = KindRisk::new(
            hit,
            rate,
            6 + i as u8,
            1 + i as u8,
            Style::Melee,
            UnknownKind::Known,
            0,
            640,
            true,
            false,
            false,
            false,
        );
    }
    let walk = route(90, 115);
    let path = fx.path(&walk);
    let value = input(99, 99, 0, &tables);
    let plan = build_plan(path, fx.zones(), &risk, fx.wilderness(), &value).unwrap();
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
    assert_eq!(
        damage, 37,
        "replay stops at the crossing floor; the full own-rate bound remains 68"
    );
    let assessed =
        assert_assessment_equivalence(fx.route(&walk), &context(&fx, &risk, &tables), value);
    assert_eq!(
        (assessed.verdict, assessed.hp_after),
        (Verdict::Survivable, 99)
    );
    assert_eq!(
        (assessed.crossings[0].worst, assessed.crossings[0].volley),
        (68, 14)
    );
    assert!(!assessed.crossings[0].single);
}
#[test]
fn u5_witnesses_unknown_overflow_and_multi_burst() {
    if !api::test_support::game_data_available() {
        return;
    }
    let tables = tables();
    let fx = Fixture::new(one_zone(&tables, 100, 1), WildernessRules::default());
    let risk = RiskTables::build(&tables);
    let w = WildernessRules::default();
    let cx = context(&fx, &risk, &tables);
    let route = route(88, 112);
    let missing = Estimate { zones: None, ..cx };
    let missing_result =
        assert_assessment_equivalence(fx.route(&route), &missing, input(40, 40, 0, &tables));
    assert_eq!(
        missing_result.verdict,
        Verdict::Unknown(UnknownWhy::NoZoneTable)
    );
    let mut bad = input(40, 40, 0, &tables);
    bad.set_unattributed(true);
    let bad_result = assert_assessment_equivalence(fx.route(&route), &cx, bad);
    assert_eq!(
        bad_result.verdict,
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
        assert_assessment_equivalence(fx.route(&huge), &cx, input(40, 40, 0, &tables)).verdict,
        Verdict::Unknown(UnknownWhy::Overflow)
    );
    let burst = Fixture::new(
        zone_table(
            (0..8)
                .map(|i| Zone::npc(tile(97 + i), 1, ZoneClass::Always, u16::MAX, 0))
                .collect(),
            vec![known_kind(&tables)],
            vec![],
            vec![],
            &w,
        ),
        w.clone(),
    );
    let risk = RiskTables::build(&tables);
    let burst_assessment = assert_assessment_equivalence(
        burst.route(&route),
        &context(&burst, &risk, &tables),
        input(20, 40, 0, &tables),
    );
    assert_eq!(burst_assessment.verdict, Verdict::Unsurvivable);
    assert!(
        burst_assessment.reason.contains("max HP"),
        "{}",
        burst_assessment.reason
    );
    let overflow = Fixture::new(
        zone_table(
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
        ),
        w,
    );
    let risk = RiskTables::build(&tables);
    let display_overflow_assessment = assert_assessment_equivalence(
        overflow.route(&route),
        &context(&overflow, &risk, &tables),
        input(90, 90, 0, &tables),
    );
    assert_eq!(
        display_overflow_assessment.verdict,
        Verdict::Unsurvivable,
        "display saturation must not replace numeric death classification"
    );
    assert_eq!(display_overflow_assessment.crossings[0].volley, u8::MAX);
    assert_eq!(display_overflow_assessment.volley, u8::MAX);
    assert!(display_overflow_assessment.bound > consts::DEATH_BOUND);
}

#[test]
fn u9_assessment_allocation_and_cpu_measurement() {
    if !api::test_support::game_data_available() {
        return;
    }
    let tables = tables();
    let start = std::time::Instant::now();
    let risks = RiskTables::build(&tables);
    eprintln!(
        "P1-C RiskTables warm-combat build {:?}, KindRisk={} RiskInput={} Poison={} ZoneInterval={} Crossing={} Assessment={} EventBuf={} Event={}",
        start.elapsed(),
        std::mem::size_of::<KindRisk>(),
        std::mem::size_of::<RiskInput>(),
        std::mem::size_of::<PoisonState>(),
        std::mem::size_of::<ZoneInterval>(),
        std::mem::size_of::<Crossing>(),
        std::mem::size_of::<RouteAssessment>(),
        std::mem::size_of::<odds::EventBuf>(),
        std::mem::size_of::<odds::Event>(),
    );

    let fx = Fixture::new(one_zone(&tables, 100, 1), WildernessRules::default());
    let cx = context(&fx, &risks, &tables);
    let wolf_fx = Fixture::new(wolf_zone(&tables, &[100], 1), WildernessRules::default());
    let wolf_cx = context(&wolf_fx, &risks, &tables);
    let wwm_rows = tables.selected().npc_names().unwrap().rows.iter();
    let mut wwm_npcs: Vec<_> = wwm_rows
        .filter(|npc| {
            matches!(
                npc.damagetype.as_deref(),
                Some(
                    "^damagetype_stab"
                        | "^stab_style"
                        | "^damagetype_slash"
                        | "^slash_style"
                        | "^damagetype_crush"
                        | "^crush_style"
                        | "^damagetype_ranged"
                        | "^ranged_style"
                        | "^damagetype_melee"
                )
            ) && KindRisk::from_npc(npc, &tables).attack_roll > 0
        })
        .collect();
    wwm_npcs.sort_unstable_by_key(|npc| npc.id);
    assert!(
        wwm_npcs.len() >= 20,
        "the selected table needs 20 distinct supported kinds"
    );
    let mut wwm_zones = Vec::with_capacity(20);
    let mut wwm_kinds = Vec::with_capacity(20);
    let mut wwm_ids = Vec::with_capacity(20);
    for (index, npc) in wwm_npcs.into_iter().take(20).enumerate() {
        let kind = u16::try_from(index).expect("20 kinds fit the zone-kind index");
        wwm_zones.push(Zone::npc(
            tile(100 + i32::try_from(index).expect("20 zones fit tile arithmetic")),
            1,
            ZoneClass::Always,
            u16::MAX,
            kind,
        ));
        wwm_kinds.push(ZoneKind::new(
            npc.id.to_string(),
            npc.config.as_str(),
            npc.id,
            1,
            false,
            false,
        ));
        wwm_ids.push(npc.id);
    }
    let wwm_fx = Fixture::new(
        zone_table(
            wwm_zones,
            wwm_kinds,
            vec![],
            vec![],
            &WildernessRules::default(),
        ),
        WildernessRules::default(),
    );
    let mut wwm_risks = RiskTables::build(&tables);
    for ident in &wwm_ids {
        let fact = *wwm_risks.npc(*ident).expect("selected NPC fact");
        // Extend each synthetic exposure, keeping the rate-4 cadence.
        *wwm_risks.npc_mut(*ident).expect("selected NPC fact") = KindRisk::new(
            6,
            4,
            11,
            1,
            Style::Melee,
            UnknownKind::Known,
            0,
            fact.attack_roll,
            true,
            false,
            false,
            false,
        );
    }
    let wwm_cx = context(&wwm_fx, &wwm_risks, &tables);
    let wwm_route = route(80, 140);
    let wwm_input = input(40, 40, 0, &tables);
    let wwm_plan = build_plan(
        wwm_fx.path(&wwm_route),
        wwm_fx.zones(),
        &wwm_risks,
        wwm_fx.wilderness(),
        &wwm_input,
    )
    .unwrap();
    assert_eq!(wwm_plan.intervals.len(), 20);
    let mut wwm_events = odds::EventBuf::new();
    let wwm_schedule = timeline::build_timeline(
        wwm_fx.path(&wwm_route),
        &wwm_plan,
        &wwm_input,
        &tables,
        &wwm_risks,
        timeline::PoisonCharge::None,
        &mut wwm_events,
    )
    .unwrap();
    assert!(
        wwm_schedule.attacks >= 100,
        "summed distinct-kind workload: {wwm_schedule:?}"
    );

    let stall_fx = Fixture::new(wolf_zone(&tables, &[100], 1), WildernessRules::default());
    let stall_cx = context(&stall_fx, &risks, &tables);
    let stall_route = Route {
        legs: vec![
            Leg::Walk {
                tiles: (80..=100).map(tile).collect(),
            },
            Leg::Transport {
                edge: Box::new(edge(tile(100), tile(101), 120)),
            },
            Leg::Walk {
                tiles: (101..=120).map(tile).collect(),
            },
        ],
        dest: tile(120),
        ticks: 0.0,
    };

    let wolf_27_route = route(90, 107);
    assert_eq!(
        RoutePath::new(wolf_fx.route(&wolf_27_route))
            .unwrap()
            .ticks(),
        27
    );

    let three_fx = Fixture::new(
        wolf_zone(&tables, &[200, 500, 800], 1),
        WildernessRules::default(),
    );
    let three_cx = context(&three_fx, &risks, &tables);
    let three_route = route(10, 1009);
    let three_input = input(90, 90, 0, &tables);
    assert_eq!(
        RoutePath::new(three_fx.route(&three_route))
            .unwrap()
            .ticks(),
        1500
    );
    let three_plan = build_plan(
        three_fx.path(&three_route),
        three_fx.zones(),
        &risks,
        three_fx.wilderness(),
        &three_input,
    )
    .unwrap();
    assert_eq!(three_plan.crossings.len(), 3);
    let mut poison_input = input(90, 90, 0, &tables);
    poison_input.poison = PoisonState::Unknown { since: 0 };
    let poison_fx = Fixture::new(wolf_zone(&tables, &[100], 1), WildernessRules::default());
    let poison_cx = context(&poison_fx, &risks, &tables);
    let poison_route = route(80, 124);

    let mut timings = Vec::with_capacity(100);
    let cases = [
        ("safe", &fx, &cx, route(10, 30), input(40, 40, 0, &tables)),
        (
            "crossing",
            &fx,
            &cx,
            route(88, 112),
            input(90, 90, 0, &tables),
        ),
        (
            "unsurvivable",
            &fx,
            &cx,
            route(88, 112),
            input(20, 40, 0, &tables),
        ),
        (
            "wolf_27",
            &wolf_fx,
            &wolf_cx,
            wolf_27_route,
            wolf_input(40, 40, &tables),
        ),
        ("wwm_summed", &wwm_fx, &wwm_cx, wwm_route, wwm_input),
        (
            "wolf_stall_120",
            &stall_fx,
            &stall_cx,
            stall_route,
            wolf_input(40, 40, &tables),
        ),
        (
            "route_1500_ticks_three_crossings",
            &three_fx,
            &three_cx,
            three_route,
            three_input,
        ),
        (
            "poison_two_bounds",
            &poison_fx,
            &poison_cx,
            poison_route,
            poison_input,
        ),
    ];
    for (name, fixture, context, route, input) in cases {
        let allocations = allocation_counter::measure(|| {
            let assessment = assess(fixture.route(&route), context, input);
            std::hint::black_box(assessment);
        });
        eprintln!(
            "P1-C {name}: allocations={}, bytes={}",
            allocations.count_total, allocations.bytes_total
        );
        assert!(allocations.count_total <= 6, "{name}: {allocations:?}");
        timings.clear();
        for _ in 0..100 {
            let start = std::time::Instant::now();
            std::hint::black_box(assess(fixture.route(&route), context, input));
            timings.push(start.elapsed().as_nanos());
        }
        timings.sort_unstable();
        eprintln!("P1-C {name}: p50={}ns p95={}ns", timings[50], timings[95]);
    }
}

#[test]
fn u1_shaped_and_ap_exposure_uses_local_ceiling_and_takeoff_hold() {
    if !api::test_support::game_data_available() {
        return;
    }
    let tables = tables();
    let w = WildernessRules::default();
    let npc = known_kind(&tables);
    let fx = Fixture::new(
        zone_table(
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
        ),
        w.clone(),
    );
    let mut risk = RiskTables::build(&tables);
    let npc = fx.table.kinds()[0].npc_id;
    let fact = *risk.npc(npc).unwrap();
    *risk.npc_mut(npc).unwrap() = KindRisk::new(
        fact.max_hit,
        fact.rate,
        fact.r,
        fact.reach,
        fact.style,
        fact.unknown,
        fact.poison,
        fact.attack_roll,
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
    let path = fx.path(&shaped);
    let plan = build_plan(
        path,
        fx.zones(),
        &risk,
        fx.wilderness(),
        &input(90, 90, 0, &tables),
    )
    .unwrap();
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
    assert_eq!(fx.path(&held).exposure_ticks(1, 2).unwrap(), 14);
}

#[test]
fn u4_mixed_food_topup_windows_exclusions_and_exact_threshold() {
    if !api::test_support::game_data_available() {
        return;
    }
    let tables = tables();
    let fx = Fixture::new(one_zone(&tables, 120, 1), WildernessRules::default());
    let risks = RiskTables::build(&tables);
    let cx = context(&fx, &risks, &tables);
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
    let path = fx.path(&walk);
    let plan = corridor_plan(&fx, &tables, 20);
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
    value.set_more_food(true);
    value.food_len = 1;
    let assessed = assess(fx.route(&walk), &cx, value);
    assert_eq!(assessed.hp_after, value.hp);
    assert!(
        !assessed.reason.contains("karambwan"),
        "{}",
        assessed.reason
    );
    assert!(assessed
        .supplies
        .iter()
        .all(|need| !matches!(need, SupplyNeed::Food { .. })));
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
    let far = fx.path(&far_walk);
    let far_plan = corridor_plan(&fx, &tables, 40);
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
    let short = fx.path(&short_walk);
    let short_plan = corridor_plan(&fx, &tables, 2);
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
    let mut boundary = corridor_plan(&fx, &tables, 20);
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
    if !api::test_support::game_data_available() {
        return;
    }
    let tables = tables();
    let fx = Fixture::new(one_zone(&tables, 100, 1), WildernessRules::default());
    let risks = RiskTables::build(&tables);
    let cx = context(&fx, &risks, &tables);
    let walk = route(88, 112);
    let mut locked = input(90, 90, 0, &tables);
    locked.set_input_locked(true);
    assert_eq!(
        assess(fx.route(&walk), &cx, locked).verdict,
        Verdict::Unknown(UnknownWhy::InputLock)
    );
    let mut overflow = input(90, 90, 0, &tables);
    overflow.set_overflow(true);
    assert_eq!(
        assess(fx.route(&walk), &cx, overflow).verdict,
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
        assess(fx.route(&walk), &cx, live).verdict,
        Verdict::Unknown(_)
    ));
    live.live[0].ident = tables.selected().npc_by_config("poisonspider").unwrap().id;
    live.set_map_members(true);
    // Poison is a numeric fact, not an unsupported live-attacker class.
    assert!(!matches!(
        assess(fx.route(&walk), &cx, live).verdict,
        Verdict::Unknown(_)
    ));
    let path = fx.path(&walk);
    let ident = known_kind(&tables).npc_id;
    let weak = RoutePlan {
        complete: true,
        pack: Some(fx.pack()),
        intervals: vec![ZoneInterval::new(
            fx.bound(ZoneKey::Zone(0)),
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
            None,
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
        last_mark: 0,
        hit_since_mark: false,
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
        pack: Some(fx.pack()),
        // Four attackers of the one fixture zone; replay reads the rows'
        // physics, never their zone identity.
        intervals: (0..4)
            .map(|_| {
                ZoneInterval::new(
                    fx.bound(ZoneKey::Zone(0)),
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
                    None,
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
    let long = fx.path(&long);
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
    let enormous_result =
        assert_assessment_equivalence(fx.route(&enormous), &cx, input(90, 90, 0, &tables));
    assert_eq!(
        enormous_result.verdict,
        Verdict::Unknown(UnknownWhy::Overflow)
    );
}

#[test]
fn u9_cold_shared_tables_and_kind_build_measurement() {
    if !api::test_support::game_data_available() {
        return;
    }
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
    let fx = Fixture::new(
        zone_table(
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
        ),
        WildernessRules::default(),
    );
    let start = std::time::Instant::now();
    let mut payload = None;
    let allocations = allocation_counter::measure(|| payload = Some(RiskTables::build(&combat)));
    let elapsed = start.elapsed();
    eprintln!("S2a first build selected={selected_elapsed:?} combat={combat_elapsed:?} risk={elapsed:?}; RiskTables allocations={} payload={}B retained={}B; npcs={} zoned kinds={}",
        allocations.count_total,allocations.bytes_total,allocations.bytes_current,combat.selected().npc_names().unwrap().rows.len(),fx.table.kinds().len());
    let payload = payload.unwrap();
    assert!(facts::build_count_for_test() > 0);
    for kind in fx.table.kinds() {
        assert!(payload.npc(kind.npc_id).is_some(), "{}", kind.id);
    }
}

#[test]
#[ignore = "isolated cold construction-count probe; run explicitly"]
fn u9_risk_assessment_reuses_one_immutable_table() {
    assert_eq!(facts::build_count_for_test(), 0);
    let tables = tables();
    let fx = Fixture::new(one_zone(&tables, 100, 1), WildernessRules::default());
    let risk = RiskTables::build(&tables);
    assert_eq!(facts::build_count_for_test(), 1);
    assess(
        fx.route(&route(88, 112)),
        &context(&fx, &risk, &tables),
        input(90, 90, 0, &tables),
    );
    assert_eq!(
        facts::build_count_for_test(),
        1,
        "assessment must reuse the immutable table"
    );
}

#[test]
fn assessment_does_not_credit_food_or_report_fixable_with() {
    if !api::test_support::game_data_available() {
        return;
    }
    let tables = tables();
    let fx = Fixture::new(one_zone(&tables, 100, 1), WildernessRules::default());
    let risk = RiskTables::build(&tables);
    let walk = route(88, 112);
    let cx = context(&fx, &risk, &tables);
    let mut no_food = input(40, 40, 0, &tables);
    no_food.pos = tile(88);
    let mut carried_food = input(40, 40, 5, &tables);
    carried_food.pos = no_food.pos;

    let without = assert_assessment_equivalence(fx.route(&walk), &cx, no_food);
    let with = assess(fx.route(&walk), &cx, carried_food);
    assert_eq!(without.verdict, with.verdict);
    assert_eq!(without.bound, with.bound);
    assert_eq!(without.hp_after, 40);
    assert_eq!(with.hp_after, 40);
    assert!(without
        .supplies
        .iter()
        .chain(with.supplies.iter())
        .all(|need| !matches!(need, SupplyNeed::Food { .. })));
}

#[test]
fn u5_origin_volley_and_raw_pending_launch_are_not_prayer_credited() {
    if !api::test_support::game_data_available() {
        return;
    }
    let tables = tables();
    let fx = Fixture::new(one_zone(&tables, 100, 1), WildernessRules::default());
    let risk = RiskTables::build(&tables);
    let cx = context(&fx, &risk, &tables);
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
    let assessed = assess(fx.route(&walk), &cx, value);
    assert!(assessed.crossings[0].protect_credited);
    assert_eq!(assessed.crossings[0].volley, 12);
    assert_eq!(assessed.volley, 12);
    let path = fx.path(&walk);
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
    if !api::test_support::game_data_available() {
        return;
    }
    let tables = tables();
    let w = WildernessRules::default();
    let mut kind = known_kind(&tables);
    kind.ap = true;
    let fx = Fixture::new(
        zone_table(
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
        ),
        w.clone(),
    );
    let mut risk = RiskTables::build(&tables);
    let npc = fx.table.kinds()[0].npc_id;
    let fact = *risk.npc(npc).unwrap();
    assert!(!fact.ap(), "selected NPC facts override the zone ap hint");
    *risk.npc_mut(npc).unwrap() = KindRisk::new(
        fact.max_hit,
        fact.rate,
        fact.r,
        fact.reach,
        fact.style,
        fact.unknown,
        fact.poison,
        fact.attack_roll,
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
    let path = fx.path(&walk);
    let plan = build_plan(
        path,
        fx.zones(),
        &risk,
        fx.wilderness(),
        &input(90, 90, 0, &tables),
    )
    .unwrap();
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
    if !api::test_support::game_data_available() {
        return;
    }
    let tables = tables();
    let fx = Fixture::new(one_zone(&tables, 100, 1), WildernessRules::default());
    let risk = RiskTables::build(&tables);
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
        fx.route(&walk),
        &context(&fx, &risk, &tables),
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
    let fx = Fixture::empty();
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
        let point = fx.path(&route).point(first).unwrap();
        assert!(!point.input_held(start - 1));
        assert!(point.input_held(start));
        assert!(point.input_held(end - 1));
        assert!(!point.input_held(end));
    }
    let walk = route(80, 81);
    assert!(!fx.path(&walk).point(0).unwrap().input_held(0));
}

/// S2b R3 M-A: an escape starts only from an engaged origin (inside A_z and
/// active). A fringe origin inside E_z but outside A_z that walks into A_z
/// enters the zone and is a crossing.
#[test]
fn r3_fringe_origin_entering_the_acquisition_set_is_a_crossing() {
    if !api::test_support::game_data_available() {
        return;
    }
    let tables = tables();
    let fx = Fixture::new(one_zone(&tables, 100, 0), WildernessRules::default());
    let risk = RiskTables::build(&tables);
    let r = i32::from(risk.npc(known_kind(&tables).npc_id).unwrap().r);
    assert!(r > 0, "the envelope must be wider than the radius-0 rect");
    let value = input(99, 99, 0, &tables);
    let plan_from = |first: i32| {
        let walk = route(first, 100 + r + 5);
        build_plan(fx.path(&walk), fx.zones(), &risk, fx.wilderness(), &value).unwrap()
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
    if !api::test_support::game_data_available() {
        return;
    }
    let tables = tables();
    let r = i32::from(
        RiskTables::build(&tables)
            .npc(known_kind(&tables).npc_id)
            .unwrap()
            .r,
    );
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
    let fx = Fixture::new(
        zone_table(
            vec![wide, local],
            vec![known_kind(&tables)],
            vec![],
            vec![],
            &WildernessRules::default(),
        ),
        WildernessRules::default(),
    );
    let risk = RiskTables::build(&tables);
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
        build_plan(fx.path(&walk), fx.zones(), &risk, fx.wilderness(), &value).unwrap_err(),
        UnknownWhy::Overflow
    );
    let assessment = assess(fx.route(&walk), &context(&fx, &risk, &tables), value);
    assert_eq!(assessment.verdict, Verdict::Unknown(UnknownWhy::Overflow));
    assert!(!assessment.plan.complete);
}

/// A complete but lethal crossing cannot mask a separate geometry failure.
#[test]
fn r3_incomplete_plan_stays_unknown() {
    if !api::test_support::game_data_available() {
        return;
    }
    let tables = tables();
    let risk = RiskTables::build(&tables);
    // Enter the wide zone's envelope after starting inside its acquisition
    // rectangle. This makes e > a rather than omitting that zone entirely.
    let mut tiles = vec![tile(80), WorldTile { z: 99, ..tile(80) }, tile(80)];
    tiles.extend((81..=170).map(tile));
    let walk = Route {
        dest: tile(170),
        legs: vec![Leg::Walk { tiles }],
        ticks: 92.0,
    };
    let mut value = input(99, 99, 0, &tables);
    value.pos = tile(80);
    let make_dangerous_zones = || {
        (100..=140)
            .map(|x| Zone::npc(tile(x), 0, ZoneClass::Always, u16::MAX, 0))
            .collect::<Vec<_>>()
    };

    let valid_fx = Fixture::new(
        zone_table(
            make_dangerous_zones(),
            vec![known_kind(&tables)],
            vec![],
            vec![],
            &WildernessRules::default(),
        ),
        WildernessRules::default(),
    );
    let valid_plan = build_plan(
        valid_fx.path(&walk),
        valid_fx.zones(),
        &risk,
        valid_fx.wilderness(),
        &value,
    )
    .unwrap();
    assert!(valid_plan.complete);
    assert!(!valid_plan.crossings.is_empty());
    let valid_assessment = assert_assessment_equivalence(
        valid_fx.route(&walk),
        &context(&valid_fx, &risk, &tables),
        value,
    );
    assert_eq!(valid_assessment.verdict, Verdict::Unsurvivable);
    assert!(valid_assessment.bound > consts::DEATH_BOUND);

    let radius = i32::from(risk.npc(known_kind(&tables).npc_id).unwrap().r);
    assert!(radius > 0);
    let origin = WorldTile {
        x: 80,
        z: 100,
        level: 0,
    };
    let mut malformed_zones = make_dangerous_zones();
    malformed_zones.push(Zone::npc(
        WorldTile {
            x: origin.x,
            z: 99 - radius,
            level: 0,
        },
        u8::try_from(2 * radius).unwrap(),
        ZoneClass::Always,
        u16::MAX,
        0,
    ));
    let malformed_fx = Fixture::new(
        zone_table(
            malformed_zones,
            vec![known_kind(&tables)],
            vec![],
            vec![],
            &WildernessRules::default(),
        ),
        WildernessRules::default(),
    );
    assert_eq!(
        build_plan(
            malformed_fx.path(&walk),
            malformed_fx.zones(),
            &risk,
            malformed_fx.wilderness(),
            &value,
        )
        .unwrap_err(),
        UnknownWhy::Overflow
    );
    let assessment = assert_assessment_equivalence(
        malformed_fx.route(&walk),
        &context(&malformed_fx, &risk, &tables),
        value,
    );
    assert_eq!(assessment.verdict, Verdict::Unknown(UnknownWhy::Overflow));
    assert!(!assessment.plan.complete);
    assert!(assessment.plan.crossings.is_empty());
}

/// T5.1: routes, plans and explicit grants retain their pack identity.
#[test]
fn t5_1_assessment_refuses_another_pack() {
    if !api::test_support::game_data_available() {
        return;
    }
    let tables = tables();
    let fx = Fixture::new(one_zone(&tables, 100, 1), WildernessRules::default());
    let other = Fixture::new(one_zone(&tables, 100, 1), WildernessRules::default());
    assert_ne!(fx.pack(), other.pack(), "two worlds, two identities");
    let risk = RiskTables::build(&tables);
    let walk = route(88, 112);
    let value = input(90, 90, 0, &tables);

    // The route view of another pack cannot build this pack's plan.
    assert_eq!(
        build_plan(
            other.path(&walk),
            fx.zones(),
            &risk,
            fx.wilderness(),
            &value
        )
        .unwrap_err(),
        UnknownWhy::NavIdentity
    );
    let foreign = assess(other.route(&walk), &context(&fx, &risk, &tables), value);
    assert_eq!(foreign.verdict, Verdict::Unknown(UnknownWhy::NavIdentity));
    assert!(!foreign.plan.complete);
    assert!(foreign.reason.contains("NavIdentity"));

    // A plan built on this pack is judged only against this pack.
    let assessment = assess(fx.route(&walk), &context(&fx, &risk, &tables), value);
    assert!(assessment.plan.complete);
    assert_eq!(assessment.plan.pack, Some(fx.pack()));
    assert_eq!(assessment.plan.intervals.len(), 1);
    assert_eq!(assessment.plan.intervals[0].zone.pack(), fx.pack());
    assert_eq!(
        fx.zones()
            .kind(assessment.plan.intervals[0].zone)
            .unwrap()
            .npc_id(),
        Some(fx.table.kinds()[0].npc_id)
    );
    assert_eq!(
        other.zones().kind(assessment.plan.intervals[0].zone).err(),
        Some(nav_planner::contract::ContractError::PackMismatch)
    );
    assert_eq!(
        assessment.plan.intervals[0].key(fx.zones()),
        Ok(ZoneKey::Zone(0))
    );
    assert_eq!(
        assessment.plan.intervals[0].key(other.zones()),
        Err(UnknownWhy::NavIdentity)
    );
}
fn yielding_checkpoint(
    calls: Arc<AtomicUsize>,
    drops: Arc<AtomicUsize>,
) -> impl FnMut() -> YieldOnce {
    move || {
        calls.fetch_add(1, Ordering::Relaxed);
        YieldOnce {
            yielded: false,
            probe: Some(DropProbe(Arc::clone(&drops))),
        }
    }
}

#[test]
fn t5w_assessment_yields_and_drops_geometry_and_timeline_state() {
    if !api::test_support::game_data_available() {
        return;
    }
    let tables = tables();
    let risk = RiskTables::build(&tables);
    let walk = route(80, 139);
    let fx = Fixture::new(one_zone(&tables, 100, 1), WildernessRules::default());
    let estimate = context(&fx, &risk, &tables);
    let value = input(99, 99, 0, &tables);
    let calls = Arc::new(AtomicUsize::new(0));
    let drops = Arc::new(AtomicUsize::new(0));
    let mut checkpoint = yielding_checkpoint(Arc::clone(&calls), Arc::clone(&drops));
    let route_view = fx.route(&walk);
    let mut future = Box::pin(assess_cooperative(
        route_view,
        &estimate,
        value,
        &mut checkpoint,
    ));
    let mut task_context = Context::from_waker(Waker::noop());

    assert!(future.as_mut().poll(&mut task_context).is_pending());
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    assert_eq!(drops.load(Ordering::Relaxed), 0);
    assert!(future.as_mut().poll(&mut task_context).is_pending());
    assert_eq!(calls.load(Ordering::Relaxed), 2);
    drop(future);
    assert_eq!(
        calls.load(Ordering::Relaxed),
        2,
        "drop schedules no more work"
    );
    assert_eq!(
        drops.load(Ordering::Relaxed),
        2,
        "suspended state is released"
    );

    let fx = Fixture::empty();
    let walk = route(90, 119);
    let mut estimate = context(&fx, &risk, &tables);
    estimate.allow = WalkAllow {
        prayer: false,
        food: false,
        escape: false,
    };
    let route_view = fx.route(&walk);
    let mut clear = input(99, 99, 0, &tables);
    clear.food_len = 0;
    let baseline_calls = Arc::new(AtomicUsize::new(0));
    let mut immediate = {
        let baseline_calls = Arc::clone(&baseline_calls);
        move || {
            baseline_calls.fetch_add(1, Ordering::Relaxed);
            std::future::ready(())
        }
    };
    let _ = poll_ready(assess_cooperative(
        route_view,
        &estimate,
        clear,
        &mut immediate,
    ));
    let clear_baseline_units = baseline_calls.load(Ordering::Relaxed);

    let mut poisoned = clear;
    poisoned.poison = PoisonState::Poisoned {
        per_tick: 1,
        last_mark: 0,
        hit_since_mark: false,
    };
    let calls = Arc::new(AtomicUsize::new(0));
    let drops = Arc::new(AtomicUsize::new(0));
    let mut checkpoint = yielding_checkpoint(Arc::clone(&calls), Arc::clone(&drops));
    let mut future = Box::pin(assess_cooperative(
        route_view,
        &estimate,
        poisoned,
        &mut checkpoint,
    ));
    let mut task_context = Context::from_waker(Waker::noop());
    // A clear and a poisoned empty-plan case both have one timeline-bound pass.
    // This baseline includes the final post-boundary checkpoint.
    for _ in 0..clear_baseline_units {
        assert!(future.as_mut().poll(&mut task_context).is_pending());
    }
    assert_eq!(calls.load(Ordering::Relaxed), clear_baseline_units);
    drop(future);
    assert_eq!(
        calls.load(Ordering::Relaxed),
        clear_baseline_units,
        "cancelled assessment cannot continue"
    );
    assert_eq!(
        drops.load(Ordering::Relaxed),
        clear_baseline_units,
        "geometry/timeline checkpoint futures are dropped"
    );
}

#[test]
fn t5w_heap_bound_covers_one_assessment_without_projection() {
    let bounds = assessment_heap_bounds().expect("risk heap bound is representable");
    assert_eq!(bounds.peak_bytes, bounds.retained_bytes);
    assert_eq!(admission_peak_heap_bytes(), Some(bounds.peak_bytes));
}
