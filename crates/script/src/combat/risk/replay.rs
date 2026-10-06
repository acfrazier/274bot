use super::consts::{LAG, PACE_AHEAD, POISON_PERIOD, TOPUP_WINDOW};
use super::*;
use crate::combat::frame::Frame;
use crate::combat::policy;
use crate::combat::schedule::{InputEffect, OpKind, Schedule};
use crate::combat::select;
use crate::combat::tables::{CombatTables, FoodFact};
use crate::native::WalkAllow;
use api::obj_names::ItemDefView;
use api::snapshot::{
    ActorView, CombatView, HitmarkView, ItemActionFamily, ItemContainer, ItemView, LocalPlayerView,
    PlayerView, WorldStateView,
};
use nav::router::Route;
use nav::transport::WildernessRules;
use std::fmt::{self, Write};

#[derive(Clone, Copy)]
pub struct Estimate<'a> {
    pub zones: Option<&'a ZoneTable>,
    pub wilderness: &'a WildernessRules,
    pub risks: &'a RiskTables,
    pub combat: &'a CombatTables,
    pub allow: WalkAllow,
    pub generation: u64,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReplayResult {
    pub passed: bool,
    pub hp_after: i32,
    pub failed_tick: Option<i32>,
    pub bites: u16,
}
fn add(a: i32, b: i32) -> Result<i32, UnknownWhy> {
    a.checked_add(b).ok_or(UnknownWhy::Overflow)
}
fn mul(a: i32, b: i32) -> Result<i32, UnknownWhy> {
    a.checked_mul(b).ok_or(UnknownWhy::Overflow)
}
fn ceil(n: i32, d: i32) -> Result<i32, UnknownWhy> {
    if n < 0 || d <= 0 {
        return Err(UnknownWhy::Overflow);
    }
    Ok(add(n, d - 1)? / d)
}
fn crossing_at(plan: &RoutePlan, index: u16) -> Option<&CrossingGeom> {
    plan.crossings
        .iter()
        .find(|c| c.first <= index && index <= c.last)
}
fn rows<'a>(plan: &'a RoutePlan, c: &CrossingGeom) -> &'a [ZoneInterval] {
    &plan.intervals[usize::from(c.intervals.0)..usize::from(c.intervals.1)]
}
fn margin(plan: &RoutePlan, c: &CrossingGeom) -> i32 {
    rows(plan, c)
        .iter()
        .map(|r| i32::from(r.max_hit))
        .max()
        .unwrap_or(0)
        .max(2)
}
fn prayer_points_needed(path: RoutePath<'_>, c: &CrossingGeom) -> Option<i32> {
    let hold = path.ticks().checked_sub(path.point(c.env_first)?.tick)?;
    if hold < 0 {
        return None;
    }
    // ceil((hold + 4) / 5) + 1, without overflowing hold + 8.
    Some(hold / 5 + 2 + i32::from(hold % 5 >= 2))
}
fn protect(
    c: &CrossingGeom,
    plan: &RoutePlan,
    path: RoutePath<'_>,
    input: &RiskInput,
    tables: &CombatTables,
    allow: WalkAllow,
) -> bool {
    if !allow.prayer || input.other_prayers_on || input.off_debt {
        return false;
    }
    let intervals = rows(plan, c);
    let Some(first) = intervals.first() else {
        return false;
    };
    if first.style == Style::Unknown
        || intervals
            .iter()
            .any(|row| row.unknown() || row.style != first.style)
    {
        return false;
    }
    let Some(prayer) = select::protect_fact(tables, first.style.into()) else {
        return false;
    };
    let Some(needed) = prayer_points_needed(path, c) else {
        return false;
    };
    i32::from(input.prayer_base) >= prayer.level && i32::from(input.prayer) >= needed
}
fn distance(path: RoutePath<'_>, c: &CrossingGeom, i: u16) -> Result<Option<i32>, UnknownWhy> {
    let mut d = None;
    if c.retreat != CrossingGeom::NONE {
        d = Some(path.distance(i, c.retreat)?);
    }
    if c.forward != CrossingGeom::NONE {
        let forward = path.distance(i, c.forward)?;
        d = Some(d.map_or(forward, |old| old.min(forward)));
    }
    Ok(d)
}
/// The shared empty-live-set candidate cost. S2 never caps the zone sum by
/// nearby scene counts or assumes single-way coverage.
pub fn candidate_cost(
    path: RoutePath<'_>,
    plan: &RoutePlan,
    i: u16,
    credited: bool,
) -> Result<Option<i32>, UnknownWhy> {
    let Some(c) = crossing_at(plan, i) else {
        return Ok(Some(0));
    };
    let Some(d) = distance(path, c, i)? else {
        return Ok(None);
    };
    let mut damage = 0;
    if !credited {
        for row in rows(plan, c) {
            if row.a <= i && i <= row.b && !row.hazard() {
                damage = add(
                    damage,
                    mul(
                        i32::from(row.max_hit),
                        ceil(add(d, LAG)?, i32::from(row.rate))?,
                    )?,
                )?;
            }
        }
    }
    Ok(Some(damage))
}
fn poison_damage(poison: PoisonState, ticks: i32) -> Result<i32, UnknownWhy> {
    match poison {
        PoisonState::Poisoned { per_tick, .. } => {
            mul(i32::from(per_tick), ceil(ticks, POISON_PERIOD)?)
        }
        _ => Ok(0),
    }
}
/// Floor with no live rows. None means no executable on-foot way out.
pub fn estimated_floor(
    path: RoutePath<'_>,
    plan: &RoutePlan,
    i: u16,
    input: &RiskInput,
    tables: &CombatTables,
    allow: WalkAllow,
) -> Result<Option<i32>, UnknownWhy> {
    let Some(c) = crossing_at(plan, i) else {
        return Ok(Some(0));
    };
    let mut cost = 0;
    let mut poison = 0;
    for ahead in 0..=PACE_AHEAD {
        let Some(j) = i
            .checked_add(ahead)
            .filter(|j| usize::from(*j) < path.len())
        else {
            continue;
        };
        let Some(next) = crossing_at(plan, j) else {
            continue;
        };
        let Some(d) = distance(path, next, j)? else {
            return Ok(None);
        };
        let Some(value) = candidate_cost(
            path,
            plan,
            j,
            protect(next, plan, path, input, tables, allow),
        )?
        else {
            return Ok(None);
        };
        cost = cost.max(value);
        poison = poison.max(poison_damage(input.poison, add(d, LAG)?)?);
    }
    Ok(Some(add(add(cost, poison)?, margin(plan, c))?))
}
fn volley(plan: &RoutePlan, i: u16, credited: bool) -> Result<i32, UnknownWhy> {
    if credited {
        return Ok(0);
    }
    let mut value = 0;
    if let Some(c) = crossing_at(plan, i) {
        for row in rows(plan, c).iter().filter(|row| row.a <= i && i <= row.b) {
            value = add(value, i32::from(row.max_hit))?;
        }
    }
    Ok(value)
}
/// Intended threshold. Callers must gate hp <= this exact value before using
/// S1's rounded danger conversion (an even line must not become line + 1).
pub fn eat_line(
    path: RoutePath<'_>,
    plan: &RoutePlan,
    i: u16,
    input: &RiskInput,
    tables: &CombatTables,
    allow: WalkAllow,
) -> Result<i32, UnknownWhy> {
    let mut minimum_rate = u8::MAX;
    for c in &plan.crossings {
        if c.last >= i && path.distance(i, c.first.max(i))? <= TOPUP_WINDOW {
            minimum_rate =
                minimum_rate.min(rows(plan, c).iter().map(|row| row.rate).min().unwrap_or(4));
        }
    }
    if minimum_rate == u8::MAX {
        return Ok(match input.poison {
            PoisonState::Poisoned { per_tick, .. } => 2 * i32::from(per_tick) + 1,
            _ => 0,
        });
    }
    let look = u16::from(minimum_rate)
        .checked_mul(2)
        .ok_or(UnknownWhy::Overflow)?;
    let mut highest = 0;
    let mut m = 2;
    for ahead in 1..=look {
        let Some(j) = i
            .checked_add(ahead)
            .filter(|j| usize::from(*j) < path.len())
        else {
            break;
        };
        if let Some(c) = crossing_at(plan, j) {
            m = m.max(margin(plan, c));
            highest = highest.max(
                candidate_cost(path, plan, j, protect(c, plan, path, input, tables, allow))?
                    .unwrap_or(i32::MAX),
            );
        }
    }
    let next = i
        .checked_add(1)
        .filter(|j| usize::from(*j) < path.len())
        .unwrap_or(i);
    let credited =
        crossing_at(plan, next).is_some_and(|c| protect(c, plan, path, input, tables, allow));
    let v = volley(plan, next, credited)?.max(volley(plan, i, credited)?);
    let reserve = poison_damage(input.poison, add(i32::from(look), LAG)?)?;
    Ok(add(add(add(highest, m)?, v)?, reserve)?.max(add(mul(v, 2)?, 1)?))
}
fn ordinary(food: &FoodFact) -> bool {
    food.eat_delay_arg == Some(2)
        && food.message_delay.is_none_or(|delay| delay <= 0)
        && food.heal > 0
}
fn exposure(path: RoutePath<'_>, row: &ZoneInterval) -> Result<(i32, i32), UnknownWhy> {
    let start = path.point(row.a).ok_or(UnknownWhy::Overflow)?.tick;
    let end = path.interval_end_tick(row)?;
    Ok((start, end))
}
fn origin_duration(live: &LiveRow, tables: &CombatTables) -> Result<i32, UnknownWhy> {
    let npc = tables.npc(live.ident).ok_or(UnknownWhy::AlreadyEngaged)?;
    let r = super::facts::npc_envelope_radius(npc).ok_or(UnknownWhy::Overflow)?;
    add(mul(i32::from(r), 2)?, LAG)
}

fn hits(
    path: RoutePath<'_>,
    plan: &RoutePlan,
    tick: i32,
    input: &RiskInput,
    tables: &CombatTables,
    allow: WalkAllow,
) -> Result<i32, UnknownWhy> {
    let mut damage = 0;
    for c in &plan.crossings {
        if protect(c, plan, path, input, tables, allow) {
            if tick == path.point(c.first).ok_or(UnknownWhy::Overflow)?.tick {
                let mut worst = 0;
                for i in c.first..=c.last {
                    worst = worst.max(volley(plan, i, false)?);
                }
                damage = add(damage, worst)?;
            }
            continue;
        }
        for row in rows(plan, c) {
            let (start, end) = exposure(path, row)?;
            if row.hazard() {
                if tick == start {
                    damage = add(damage, i32::from(row.max_hit))?;
                }
            } else if tick >= start && tick < end && (tick - start) % i32::from(row.rate) == 0 {
                damage = add(damage, i32::from(row.max_hit))?;
            }
        }
    }
    for live in input.live.iter().take(usize::from(input.live_len)) {
        let duration = origin_duration(live, tables)?;
        if tick < duration && tick % i32::from(live.rate) == 0 {
            damage = add(damage, i32::from(live.max_hit))?;
        }
        // Pending launch damage is raw, independent of admission prayer credit.
        if live
            .due()
            .is_some_and(|due| tick == i32::from(due.wrapping_sub(input.tick)))
        {
            damage = add(damage, i32::from(live.max_hit))?;
        }
    }
    if tick % POISON_PERIOD == 0 {
        if let PoisonState::Poisoned { per_tick, .. } = input.poison {
            damage = add(damage, i32::from(per_tick))?;
        }
    }
    Ok(damage)
}

fn input_problem(input: &RiskInput, tables: &CombatTables) -> Option<UnknownWhy> {
    if input.overflow || input.live_len > 4 || input.food_len > 6 {
        return Some(UnknownWhy::Overflow);
    }
    if input.unattributed
        || input
            .live
            .iter()
            .take(usize::from(input.live_len))
            .any(|live| {
                live.actor.kind != crate::combat::ActorKind::Npc
                    || live.rate == 0
                    || tables.npc(live.ident).is_none_or(|row| {
                        KindRisk::from_npc(row, tables)
                            .unknown_for(input.map_members)
                            .is_some()
                    })
            })
    {
        return Some(UnknownWhy::AlreadyEngaged);
    }
    if input.missing_facts || input.hp == 0 || input.hp_max == 0 {
        return Some(UnknownWhy::MissingFacts);
    }
    if input.input_locked {
        return Some(UnknownWhy::InputLock);
    }
    None
}

/// Replays the shared picker against stack-only inventory/count overlays.
/// The hook exposes applied hits and bites to independent test simulations.
#[allow(clippy::too_many_arguments)]
pub fn replay(
    path: RoutePath<'_>,
    plan: &RoutePlan,
    input: &RiskInput,
    tables: &CombatTables,
    allow: WalkAllow,
    extra: Option<(i32, u8)>,
    mut observe: impl FnMut(i32, u16, i32, i32, Option<i32>),
) -> Result<ReplayResult, UnknownWhy> {
    if let Some(why) = input_problem(input, tables) {
        return Err(why);
    }
    if plan.intervals.is_empty()
        && input.live_len == 0
        && !matches!(input.poison, PoisonState::Poisoned { per_tick: 1.., .. })
    {
        return Ok(ReplayResult {
            passed: true,
            hp_after: i32::from(input.hp),
            failed_tick: None,
            bites: 0,
        });
    }
    let mut inventory: [ItemView; 7] = std::array::from_fn(|i| ItemView {
        def: ItemDefView {
            id: if i < 6 {
                input.food_ids[i]
            } else {
                extra.map_or(-1, |food| food.0)
            },
            name: None,
            stackable: false,
            members: false,
            base_value: 0,
            noted: false,
            certificate_link: -1,
            certificate_template: -1,
        },
        container: ItemContainer::Inventory,
        action_family: ItemActionFamily::Held,
        slot: i as i32,
        count: if i < 6 {
            i32::from(input.food_counts[i])
        } else {
            extra.map_or(0, |food| i32::from(food.1))
        },
        actions: Vec::new(),
        component_id: 0,
    });
    for item in &mut inventory {
        if !tables.food(item.def.id).is_some_and(ordinary) {
            item.count = 0;
        }
    }
    let local = LocalPlayerView {
        player: PlayerView {
            index: 0,
            actor: ActorView {
                name: None,
                actions: Vec::new(),
                tile: input.pos,
                distance: 0,
                animation: -1,
                animation_frame: -1,
                pose_animation: -1,
                orientation: 0,
                target_orientation: 0,
                overhead_text: None,
                spot_animation: -1,
                spot_animation_stamp: -1,
                health: i32::from(input.hp),
                total_health: i32::from(input.hp_max),
                face_entity: -1,
                target: None,
                moving: false,
                running: false,
                in_combat: false,
            },
            // A replayed stand is stationary: the path head is its tile.
            network: input.pos,
            combat_level: input.combat.map_or(0, i32::from),
            skill_level: 0,
            headicons: 0,
            weapon: None,
        },
        energy: 0,
        weight: 0,
    };
    let world = WorldStateView::default();
    let mut schedule = Schedule::default();
    let mut hp = i32::from(input.hp);
    let cap = i32::from(input.hp_max);
    let mut pending = None;
    let mut bites = 0u16;
    let mut i = 0u16;
    let mut duration = path.ticks();
    for row in &plan.intervals {
        duration = duration.max(path.interval_end_tick(row)?);
    }
    // An endpoint does not retire an origin attacker or its queued impact.
    for live in input.live_iter() {
        duration = duration.max(origin_duration(live, tables)?);
        if let Some(due) = live.due() {
            duration = duration.max(add(i32::from(due.wrapping_sub(input.tick)), 1)?);
        }
    }
    for tick in 0..duration {
        while usize::from(i) + 1 < path.len()
            && path.point(i + 1).ok_or(UnknownWhy::Overflow)?.tick <= tick
        {
            i += 1;
        }
        let point = path.point(i);
        let input_held = point.is_some_and(|point| point.input_held(tick));
        // Entry is checked before its first hit, including transport-entered endpoints.
        let floor = estimated_floor(path, plan, i, input, tables, allow)?;
        if plan
            .crossings
            .iter()
            .any(|c| c.first == i && point.is_some_and(|point| point.tick == tick))
            && floor.is_none_or(|floor| hp <= floor)
        {
            return Ok(ReplayResult {
                passed: false,
                hp_after: hp,
                failed_tick: Some(tick),
                bites,
            });
        }
        let landed = hits(path, plan, tick, input, tables, allow)?;
        hp -= landed;
        if hp <= 0 || (crossing_at(plan, i).is_some() && floor.is_none_or(|floor| hp <= floor)) {
            observe(tick, i, hp, landed, None);
            return Ok(ReplayResult {
                passed: false,
                hp_after: hp,
                failed_tick: Some(tick),
                bites,
            });
        }
        if let Some((due, heal)) = pending {
            if tick >= due && !input_held {
                hp = add(hp, heal)?.min(cap);
                pending = None;
                schedule.settle(OpKind::Eat);
            }
        }
        let mut bite = None;
        if allow.food
            && !input_held
            && pending.is_none()
            && schedule.ready(OpKind::Eat, tick as u16)
        {
            let line = eat_line(path, plan, i, input, tables, allow)?;
            let topup = crossing_at(plan, i).is_none()
                && plan.crossings.iter().any(|c| {
                    c.first > i
                        && path
                            .distance(i, c.first)
                            .is_ok_and(|gap| gap <= TOPUP_WINDOW)
                })
                && inventory
                    .iter()
                    .filter(|food| food.count > 0)
                    .filter_map(|food| tables.food(food.def.id))
                    .map(|food| food.heal)
                    .min()
                    .is_some_and(|heal| hp + heal <= cap);
            let intended = if topup { cap } else { line };
            // Exact threshold gating avoids C6's even-line rounding error.
            if (topup || hp <= intended) && intended > 1 {
                let frame = Frame {
                    here: input.pos,
                    combat_tab: None,
                    tick: tick as u16,
                    stats: &[],
                    inventory: &inventory,
                    equipment: &[],
                    npcs: &[],
                    players: &[],
                    local: &local,
                    world: &world,
                    projectiles: &[],
                    hitmarks: [HitmarkView {
                        value: 0,
                        kind: 0,
                        cycle: 0,
                    }; 4],
                    loop_cycle: 0,
                    prayers: [false; 15],
                    varps: &[],
                    in_combat: CombatView {
                        in_combat: false,
                        target: None,
                    },
                };
                if let Some(id) = policy::eat_choice(
                    &frame,
                    tables,
                    &schedule,
                    Some(intended / 2),
                    hp,
                    cap,
                    tick as u16,
                )
                .and_then(|choice| choice.ordinary)
                {
                    let food = tables.food(id).ok_or(UnknownWhy::MissingFacts)?;
                    let slot = inventory
                        .iter()
                        .position(|item| item.def.id == id && item.count > 0)
                        .ok_or(UnknownWhy::MissingFacts)?;
                    inventory[slot].count -= 1;
                    pending = Some((add(tick, LAG)?, food.heal));
                    schedule.admitted(OpKind::Eat, tick as u16, 0, false, InputEffect::Food(food));
                    bites = bites.checked_add(1).ok_or(UnknownWhy::Overflow)?;
                    bite = Some(id);
                }
            }
        }
        observe(tick, i, hp, landed, bite);
    }
    Ok(ReplayResult {
        passed: true,
        hp_after: hp,
        failed_tick: None,
        bites,
    })
}

struct Reason {
    bytes: [u8; 1536],
    len: usize,
}
impl Reason {
    fn new() -> Self {
        Self {
            bytes: [0; 1536],
            len: 0,
        }
    }
    fn arc(&self) -> Arc<str> {
        Arc::from(std::str::from_utf8(&self.bytes[..self.len]).expect("only UTF-8 writes"))
    }
}
impl Write for Reason {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        if self.len + text.len() > self.bytes.len() {
            return Err(fmt::Error);
        }
        self.bytes[self.len..self.len + text.len()].copy_from_slice(text.as_bytes());
        self.len += text.len();
        Ok(())
    }
}
fn bounded_hp(hp: i32) -> Result<u8, UnknownWhy> {
    u8::try_from(hp.max(0)).map_err(|_| UnknownWhy::Overflow)
}

/// Typed computation only. No routing retry, movement, guard, or slot ownership.
pub fn assess(route: &Route, context: &Estimate<'_>, input: RiskInput) -> Arc<RouteAssessment> {
    let mut reason = Reason::new();
    let result = assess_inner(route, context, input, &mut reason);
    match result {
        Ok(value) => Arc::new(value),
        Err(why) => {
            let _ = write!(reason, "Route assessment unknown: {why:?}");
            Arc::new(RouteAssessment {
                verdict: Verdict::Unknown(why),
                plan: RoutePlan::default(),
                crossings: Box::new([]),
                more: 0,
                supplies: Box::new([]),
                hp_after: input.hp,
                volley: u8::MAX,
                input,
                generation: context.generation,
                reason: reason.arc(),
            })
        }
    }
}
fn assess_inner(
    route: &Route,
    context: &Estimate<'_>,
    input: RiskInput,
    reason: &mut Reason,
) -> Result<RouteAssessment, UnknownWhy> {
    let path = RoutePath::new(route)?;
    let zones = context.zones.ok_or(UnknownWhy::NoZoneTable)?;
    let plan = build_plan(path, zones, context.risks, context.wilderness, &input)?;
    let mut unknown = input_problem(&input, context.combat);
    for row in &plan.intervals {
        if row.unknown() {
            let kind = zones.zones()[usize::from(row.zone)].kind;
            let class = context
                .risks
                .kind(kind)
                .and_then(|fact| fact.unknown_for(input.map_members));
            let _ = write!(
                reason,
                "{}: {:?}; ",
                zones.kinds()[usize::from(kind)].label,
                class
            );
            if unknown.is_none() {
                unknown = Some(match class {
                    Some(UnknownKind::MissingFacts) => UnknownWhy::MissingFacts,
                    Some(UnknownKind::Overflow) => UnknownWhy::Overflow,
                    _ => UnknownWhy::Kind(kind),
                });
            }
            break;
        }
    }
    if unknown.is_none()
        && !plan.crossings.is_empty()
        && matches!(input.poison, PoisonState::Unknown { .. })
    {
        unknown = Some(UnknownWhy::Poison);
    }

    // Validate report bounds before replay, so a huge transport hold cannot
    // turn a bounded Unknown(Overflow) assessment into a billion-tick replay.
    let mut display = Vec::with_capacity(plan.crossings.len().min(8));
    let mut origin_volley = 0;
    let mut origin_worst = 0;
    let mut unaffordable_floor = None;
    if unknown.is_none() {
        for live in input.live.iter().take(usize::from(input.live_len)) {
            let duration = origin_duration(live, context.combat)?;
            origin_volley = add(origin_volley, i32::from(live.max_hit))?;
            origin_worst = add(
                origin_worst,
                mul(
                    i32::from(live.max_hit),
                    ceil(duration, i32::from(live.rate))?,
                )?,
            )?;
            if live.due().is_some() {
                origin_worst = add(origin_worst, i32::from(live.max_hit))?;
            }
        }
    }
    let mut max_volley = origin_volley;
    for (index, c) in plan.crossings.iter().enumerate() {
        let mut v = 0;
        let mut deep = 0;
        let credited =
            unknown.is_none() && protect(c, &plan, path, &input, context.combat, context.allow);
        for i in c.first..=c.last {
            v = v.max(volley(&plan, i, false)?);
            if unknown.is_none() {
                let floor = estimated_floor(path, &plan, i, &input, context.combat, context.allow)?;
                deep = deep.max(floor.unwrap_or(255));
                if unaffordable_floor.is_none()
                    && floor.is_some_and(|cost| cost >= i32::from(input.hp_max))
                {
                    unaffordable_floor = Some((i, floor.unwrap_or(255)));
                }
            }
        }
        let mut worst = if credited { v } else { 0 };
        if unknown.is_none() && !credited {
            for row in rows(&plan, c) {
                let (start, end) = exposure(path, row)?;
                let count = if row.hazard() {
                    1
                } else {
                    ceil(end - start, i32::from(row.rate))?
                };
                worst = add(worst, mul(i32::from(row.max_hit), count)?)?;
            }
        }
        if index == 0 {
            v = add(v, origin_volley)?;
            worst = add(worst, origin_worst)?;
        }
        max_volley = max_volley.max(v);
        if v > 255 || deep > 255 {
            return Err(UnknownWhy::Overflow);
        }
        let ticks = u16::try_from(path.exposure_ticks(c.first, c.last)?)
            .map_err(|_| UnknownWhy::Overflow)?;
        let worst = u16::try_from(worst).map_err(|_| UnknownWhy::Overflow)?;
        if display.len() < 8 {
            let first = rows(&plan, c)[0];
            display.push(Crossing {
                key: first.key(zones),
                first: c.first,
                last: c.last,
                ticks,
                worst,
                volley: v as u8,
                max_hit: margin(&plan, c) as u8,
                rate: rows(&plan, c).iter().map(|row| row.rate).min().unwrap_or(0),
                style: first.style,
                floor_deep: deep as u8,
                bites: 0,
                single: false,
                protect_credited: credited,
                unknown: rows(&plan, c).iter().any(|row| row.unknown()),
            });
        }
    }

    let no_way_out = plan.crossings.iter().find(|c| !c.has_way_out());
    let mut verdict = unknown.map_or(Verdict::Survivable, Verdict::Unknown);
    let mut hp_after = input.hp;
    let mut bites = 0;
    let mut extra_food = None;
    if unknown.is_none() {
        if no_way_out.is_some() || unaffordable_floor.is_some() {
            verdict = Verdict::Unsurvivable;
        } else {
            let baseline = replay(
                path,
                &plan,
                &input,
                context.combat,
                context.allow,
                None,
                |_, _, _, _, _| {},
            )?;
            hp_after = bounded_hp(baseline.hp_after)?;
            bites = baseline.bites;
            // Poison alone never refuses a no-crossing walk. Known origin
            // attackers still retain their complete, raw admission budget.
            if !baseline.passed && (!plan.crossings.is_empty() || input.live_len > 0) {
                verdict = Verdict::Unsurvivable;
                let carried = input
                    .food_iter()
                    .filter(|(_, count)| *count > 0)
                    .filter_map(|(id, _)| context.combat.food(id))
                    .filter(|food| ordinary(food))
                    .max_by_key(|food| food.heal);
                let reference = carried.or_else(|| {
                    context
                        .combat
                        .selected()
                        .item_by_alias("lobster")
                        .and_then(|item| context.combat.food(item.id))
                });
                if context.allow.food {
                    if let Some(food) = reference {
                        for count in 1..=input.free_slots.min(28) {
                            let candidate = replay(
                                path,
                                &plan,
                                &input,
                                context.combat,
                                context.allow,
                                Some((food.item_id, count)),
                                |_, _, _, _, _| {},
                            )?;
                            if candidate.passed {
                                verdict = Verdict::FixableWith;
                                extra_food = Some((food, count));
                                break;
                            }
                        }
                    }
                }
            }
        }
    }
    let bites_display = u8::try_from(bites).map_err(|_| UnknownWhy::Overflow)?;
    for row in &mut display {
        row.bites = bites_display;
    }

    // Assemble informational rows on the stack, then allocate exactly once.
    // Protection and points remain suggestions; only ordinary food is fetched.
    let mut needs = [None; 6];
    let mut need_count = 0;
    if let Some((food, count)) = extra_food {
        let heal = u8::try_from(food.heal).map_err(|_| UnknownWhy::Overflow)?;
        needs[need_count] = Some(SupplyNeed::Food {
            heal_at_least: heal,
            heal_at_most: heal,
            count,
            item: Some(food.item_id),
        });
        need_count += 1;
    }
    for style in [Style::Melee, Style::Ranged] {
        let Some(prayer) = select::protect_fact(context.combat, style.into()) else {
            continue;
        };
        let relevant = plan.crossings.iter().filter(|c| {
            let intervals = rows(&plan, c);
            intervals
                .iter()
                .all(|row| !row.unknown() && row.style == style)
                && !protect(c, &plan, path, &input, context.combat, context.allow)
        });
        let needed = relevant.filter_map(|c| prayer_points_needed(path, c)).max();
        if let Some(needed) = needed {
            if i32::from(input.prayer_base) < prayer.level {
                needs[need_count] = Some(SupplyNeed::Info(InfoNeed::Protect {
                    style,
                    level: u8::try_from(prayer.level).map_err(|_| UnknownWhy::Overflow)?,
                }));
                need_count += 1;
            }
            if i32::from(input.prayer) < needed {
                needs[need_count] = Some(SupplyNeed::Info(InfoNeed::PrayerPoints {
                    needed: u16::try_from(needed).map_err(|_| UnknownWhy::Overflow)?,
                }));
                need_count += 1;
            }
        }
    }
    if let PoisonState::Poisoned { per_tick, .. } = input.poison {
        needs[need_count] = Some(SupplyNeed::Info(InfoNeed::Antipoison));
        need_count += 1;
        let _ = write!(
            reason,
            "poisoned ({per_tick} per 30 ticks): drink an antipoison to clear; "
        );
    }
    let mut supplies = Vec::with_capacity(need_count);
    supplies.extend(needs.into_iter().flatten());
    if matches!(verdict, Verdict::Unknown(UnknownWhy::Poison)) {
        let _ = write!(
            reason,
            "poison state unknown: settles after a separate safe walk click 30 ticks after login; "
        );
    }
    if let Some(c) = no_way_out {
        let ending = rows(&plan, c).last().ok_or(UnknownWhy::Overflow)?;
        let kind = &zones.kinds()[usize::from(zones.zones()[usize::from(ending.zone)].kind)];
        let _ = write!(reason, "NoWayOut: ends inside {}; ", kind.label);
        if let Some(nav::router::Leg::Transport { edge }) = usize::from(c.leg)
            .checked_sub(1)
            .and_then(|leg| route.legs.get(leg))
        {
            let _ = write!(reason, "entered by {:?}; ", edge.kind);
        } else {
            let _ = write!(reason, "standing inside or transport-entered; ");
        }
        let _ = write!(reason, "no way out on foot; ");
    }
    if let Some((index, cost)) = unaffordable_floor {
        let point = path.point(index).ok_or(UnknownWhy::Overflow)?;
        let _ = write!(
            reason,
            "way out at {},{} costs {cost}, at least max HP {}; ",
            point.tile.x, point.tile.z, input.hp_max
        );
    }
    if let Some(row) = plan.intervals.first() {
        let kind = &zones.kinds()[usize::from(zones.zones()[usize::from(row.zone)].kind)];
        let _ = write!(
            reason,
            "{}{} at {},{}; ",
            if row.a == 0 {
                "standing inside "
            } else {
                "Route crosses "
            },
            kind.label,
            row.spawn.tile().x,
            row.spawn.tile().z
        );
    }
    if let Some((food, count)) = extra_food {
        let name = context
            .combat
            .selected()
            .item_by_id(food.item_id)
            .and_then(|item| item.name.as_deref())
            .unwrap_or("ordinary food");
        let _ = write!(reason, "needs {count} {name}; ");
    }
    if input.more_food {
        let _ = write!(reason, "only six largest carried food kinds replayed; ");
    }
    for (id, count) in input.food_iter() {
        if count > 0 && context.combat.food(id).is_some_and(|food| !ordinary(food)) {
            let name = context
                .combat
                .selected()
                .item_by_id(id)
                .and_then(|item| item.name.as_deref())
                .unwrap_or("non-ordinary food");
            let _ = write!(reason, "{name}: message-delay/non-ordinary food excluded; ");
        }
    }
    let _ = write!(reason, "{verdict:?}; HP {} with {bites} bites, estimated {hp_after} HP at arrival; {} summed crossings", input.hp, plan.crossings.len());
    let more =
        u8::try_from(plan.crossings.len().saturating_sub(8)).map_err(|_| UnknownWhy::Overflow)?;
    Ok(RouteAssessment {
        verdict,
        plan,
        crossings: display.into_boxed_slice(),
        more,
        supplies: supplies.into_boxed_slice(),
        hp_after,
        volley: bounded_hp(max_volley)?,
        input,
        generation: context.generation,
        reason: reason.arc(),
    })
}
