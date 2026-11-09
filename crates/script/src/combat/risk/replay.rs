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
use nav_planner::contract::{BoundZoneExempt, LegView, MultiwayView, RouteView, WildernessView};
use std::fmt::{self, Write};
use std::future::Future;

/// The owner-boundary facts an assessment runs over: the pack's zone table
/// and wilderness views (both of the route's pack), the NPC-keyed risk
/// facts, and the selected combat tables.
#[derive(Clone, Copy)]
pub struct Estimate<'a> {
    pub zones: Option<ZoneTableView<'a>>,
    pub wilderness: WildernessView<'a>,
    /// Multi-combat map cells. [`MultiwayView::none`] keeps every attacker
    /// summed (an absent table never grants a single-way claim).
    pub multiway: MultiwayView<'a>,
    pub risks: &'a RiskTables,
    pub combat: &'a CombatTables,
    pub allow: WalkAllow,
    pub generation: u64,
    pub grants: Option<Grants<'a>>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PoisonTest {
    NotNeeded,
    PassesCharged { charged: f64 },
    WaitForClear { charged: f64, until_index: u16 },
    NoSafeTile { charged: f64 },
}

const _: () = assert!(std::mem::size_of::<PoisonTest>() <= 24);

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
async fn crossing_at<'a, F, U>(
    plan: &'a RoutePlan,
    index: u16,
    checkpoint: &mut F,
) -> Option<&'a CrossingGeom>
where
    F: FnMut() -> U,
    U: Future<Output = ()>,
{
    for crossing in &plan.crossings {
        let found = crossing.intervals.0 != crossing.intervals.1
            && crossing.first <= index
            && index <= crossing.last;
        super::checkpoint(checkpoint).await;
        if found {
            return Some(crossing);
        }
    }
    None
}

fn rows<'a>(plan: &'a RoutePlan, c: &CrossingGeom) -> &'a [ZoneInterval] {
    &plan.intervals[usize::from(c.intervals.0)..usize::from(c.intervals.1)]
}
async fn margin<F, U>(plan: &RoutePlan, c: &CrossingGeom, checkpoint: &mut F) -> i32
where
    F: FnMut() -> U,
    U: Future<Output = ()>,
{
    let mut maximum = 0;
    for row in rows(plan, c) {
        maximum = maximum.max(i32::from(row.max_hit));
        super::checkpoint(checkpoint).await;
    }
    maximum.max(2)
}
fn prayer_points_needed(path: RoutePath<'_>, c: &CrossingGeom) -> Option<i32> {
    let hold = path.ticks().checked_sub(path.point(c.env_first)?.tick)?;
    if hold < 0 {
        return None;
    }
    // ceil((hold + 4) / 5) + 1, without overflowing hold + 8.
    Some(hold / 5 + 2 + i32::from(hold % 5 >= 2))
}
async fn protect<F, U>(
    c: &CrossingGeom,
    plan: &RoutePlan,
    path: RoutePath<'_>,
    input: &RiskInput,
    tables: &CombatTables,
    allow: WalkAllow,
    checkpoint: &mut F,
) -> bool
where
    F: FnMut() -> U,
    U: Future<Output = ()>,
{
    if !allow.prayer || input.other_prayers_on() || input.off_debt() {
        return false;
    }
    let intervals = rows(plan, c);
    let Some(first) = intervals.first() else {
        return false;
    };
    if first.unknown() || first.style == Style::Unknown {
        return false;
    }
    let mut consistent = true;
    for row in intervals.iter().skip(1) {
        if row.unknown() || row.style != first.style {
            consistent = false;
        }
        super::checkpoint(checkpoint).await;
        if !consistent {
            return false;
        }
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
    let mut checkpoint = || std::future::ready(());
    super::drive_synchronously(candidate_cost_cooperative(
        path,
        plan,
        i,
        credited,
        &mut checkpoint,
    ))
}

async fn candidate_cost_cooperative<F, U>(
    path: RoutePath<'_>,
    plan: &RoutePlan,
    i: u16,
    credited: bool,
    checkpoint: &mut F,
) -> Result<Option<i32>, UnknownWhy>
where
    F: FnMut() -> U,
    U: Future<Output = ()>,
{
    let Some(c) = crossing_at(plan, i, checkpoint).await else {
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
            super::checkpoint(checkpoint).await;
        }
    }
    Ok(Some(damage))
}

fn poison_pulse_now(poison: PoisonState, q_unknown: u8) -> u8 {
    match poison {
        PoisonState::Onset { .. } => q_unknown,
        PoisonState::Poisoned {
            per_tick,
            hit_since_mark,
            ..
        } if hit_since_mark => per_tick.max(q_unknown),
        PoisonState::Poisoned { per_tick, .. } => per_tick,
        PoisonState::Unknown { .. } | PoisonState::Clear | PoisonState::Immune { .. } => 0,
    }
}

fn poison_charged(poison: PoisonState) -> bool {
    match poison {
        PoisonState::Onset { .. } => true,
        PoisonState::Poisoned {
            per_tick,
            hit_since_mark,
            ..
        } => per_tick != 0 || hit_since_mark,
        PoisonState::Unknown { .. } | PoisonState::Clear | PoisonState::Immune { .. } => false,
    }
}

fn poison_damage(poison: PoisonState, ticks: i32, q_unknown: u8) -> Result<i32, UnknownWhy> {
    mul(
        i32::from(poison_pulse_now(poison, q_unknown)),
        ceil(ticks, POISON_PERIOD)?,
    )
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
    let mut checkpoint = || std::future::ready(());
    super::drive_synchronously(estimated_floor_cooperative(
        path,
        plan,
        i,
        input,
        tables,
        allow,
        &mut checkpoint,
    ))
}

async fn estimated_floor_cooperative<F, U>(
    path: RoutePath<'_>,
    plan: &RoutePlan,
    i: u16,
    input: &RiskInput,
    tables: &CombatTables,
    allow: WalkAllow,
    checkpoint: &mut F,
) -> Result<Option<i32>, UnknownWhy>
where
    F: FnMut() -> U,
    U: Future<Output = ()>,
{
    let Some(c) = crossing_at(plan, i, checkpoint).await else {
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
        let Some(next) = crossing_at(plan, j, checkpoint).await else {
            continue;
        };
        let Some(d) = distance(path, next, j)? else {
            return Ok(None);
        };
        let credited = protect(next, plan, path, input, tables, allow, checkpoint).await;
        let Some(value) = candidate_cost_cooperative(path, plan, j, credited, checkpoint).await?
        else {
            return Ok(None);
        };
        cost = cost.max(value);
        poison = poison.max(poison_damage(
            input.poison,
            add(d, LAG)?,
            tables.unknown_poison_pulse(),
        )?);
    }
    Ok(Some(add(
        add(cost, poison)?,
        margin(plan, c, checkpoint).await,
    )?))
}

async fn volley<F, U>(
    plan: &RoutePlan,
    i: u16,
    credited: bool,
    checkpoint: &mut F,
) -> Result<i32, UnknownWhy>
where
    F: FnMut() -> U,
    U: Future<Output = ()>,
{
    if credited {
        return Ok(0);
    }
    let mut value = 0;
    if let Some(c) = crossing_at(plan, i, checkpoint).await {
        for row in rows(plan, c) {
            if row.a <= i && i <= row.b {
                value = add(value, i32::from(row.max_hit))?;
            }
            super::checkpoint(checkpoint).await;
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
    let mut checkpoint = || std::future::ready(());
    super::drive_synchronously(eat_line_cooperative(
        path,
        plan,
        i,
        input,
        tables,
        allow,
        &mut checkpoint,
    ))
}

async fn eat_line_cooperative<F, U>(
    path: RoutePath<'_>,
    plan: &RoutePlan,
    i: u16,
    input: &RiskInput,
    tables: &CombatTables,
    allow: WalkAllow,
    checkpoint: &mut F,
) -> Result<i32, UnknownWhy>
where
    F: FnMut() -> U,
    U: Future<Output = ()>,
{
    let mut minimum_rate = u8::MAX;

    for c in &plan.crossings {
        if c.intervals.0 != c.intervals.1
            && c.last >= i
            && path.distance(i, c.first.max(i))? <= TOPUP_WINDOW
        {
            let mut rate = u8::MAX;
            let mut has_rows = false;
            for row in rows(plan, c) {
                rate = rate.min(row.rate);
                has_rows = true;
                super::checkpoint(checkpoint).await;
            }
            minimum_rate = minimum_rate.min(if has_rows { rate } else { 4 });
        }
        super::checkpoint(checkpoint).await;
    }
    if minimum_rate == u8::MAX {
        return Ok(match input.poison {
            PoisonState::Onset { .. } | PoisonState::Poisoned { .. } => {
                2 * i32::from(poison_pulse_now(
                    input.poison,
                    tables.unknown_poison_pulse(),
                )) + 1
            }
            PoisonState::Unknown { .. } | PoisonState::Clear | PoisonState::Immune { .. } => 0,
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
        if let Some(c) = crossing_at(plan, j, checkpoint).await {
            m = m.max(margin(plan, c, checkpoint).await);
            let credited = protect(c, plan, path, input, tables, allow, checkpoint).await;
            highest = highest.max(
                candidate_cost_cooperative(path, plan, j, credited, checkpoint)
                    .await?
                    .unwrap_or(i32::MAX),
            );
        }
        super::checkpoint(checkpoint).await;
    }
    let next = i
        .checked_add(1)
        .filter(|j| usize::from(*j) < path.len())
        .unwrap_or(i);
    let credited = match crossing_at(plan, next, checkpoint).await {
        Some(c) => protect(c, plan, path, input, tables, allow, checkpoint).await,
        None => false,
    };
    let v = volley(plan, next, credited, checkpoint)
        .await?
        .max(volley(plan, i, credited, checkpoint).await?);
    let reserve = poison_damage(
        input.poison,
        add(i32::from(look), LAG)?,
        tables.unknown_poison_pulse(),
    )?;
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
    let npc =
        super::facts::effective_npc_row(live.ident, tables).ok_or(UnknownWhy::AlreadyEngaged)?;
    let kind = KindRisk::from_npc(npc, tables);
    if kind.r == 0 {
        return Err(UnknownWhy::Overflow);
    }
    add(mul(i32::from(kind.r), 2)?, LAG)
}

async fn hits<F, U>(
    path: RoutePath<'_>,
    plan: &RoutePlan,
    tick: i32,
    input: &RiskInput,
    tables: &CombatTables,
    allow: WalkAllow,
    checkpoint: &mut F,
) -> Result<i32, UnknownWhy>
where
    F: FnMut() -> U,
    U: Future<Output = ()>,
{
    let mut damage = 0;
    for c in &plan.crossings {
        if protect(c, plan, path, input, tables, allow, checkpoint).await {
            if tick == path.point(c.first).ok_or(UnknownWhy::Overflow)?.tick {
                let mut worst = 0;
                for i in c.first..=c.last {
                    worst = worst.max(volley(plan, i, false, checkpoint).await?);
                }
                damage = add(damage, worst)?;
            }
            super::checkpoint(checkpoint).await;
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
            super::checkpoint(checkpoint).await;
        }
        super::checkpoint(checkpoint).await;
    }
    // Initial envelope exposure is still physical damage, but escaping it is
    // never an entering-crossing admission obligation.
    let escape_end = plan
        .crossings
        .first()
        .map_or(plan.intervals.len(), |c| usize::from(c.intervals.0));
    for row in &plan.intervals[..escape_end] {
        let (start, end) = exposure(path, row)?;
        if row.hazard() {
            if tick == start {
                damage = add(damage, i32::from(row.max_hit))?;
            }
        } else if tick >= start && tick < end && (tick - start) % i32::from(row.rate) == 0 {
            damage = add(damage, i32::from(row.max_hit))?;
        }
        super::checkpoint(checkpoint).await;
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
        super::checkpoint(checkpoint).await;
    }
    if tick % POISON_PERIOD == 0 {
        damage = add(
            damage,
            i32::from(poison_pulse_now(
                input.poison,
                tables.unknown_poison_pulse(),
            )),
        )?;
    }
    Ok(damage)
}

fn input_problem(input: &RiskInput, tables: &CombatTables) -> Option<UnknownWhy> {
    if input.overflow() || input.live_len > 4 || input.food_len > 6 {
        return Some(UnknownWhy::Overflow);
    }
    if input.unattributed()
        || input
            .live
            .iter()
            .take(usize::from(input.live_len))
            .any(|live| {
                live.actor.kind != crate::combat::ActorKind::Npc
                    || live.rate == 0
                    || super::facts::effective_npc_row(live.ident, tables)
                        .is_none_or(|row| KindRisk::from_npc(row, tables).unknown_for().is_some())
            })
    {
        return Some(UnknownWhy::AlreadyEngaged);
    }
    if input.missing_facts() || input.hp == 0 || input.hp_max == 0 {
        return Some(UnknownWhy::MissingFacts);
    }
    if input.input_locked() {
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
    observe: impl FnMut(i32, u16, i32, i32, Option<i32>),
) -> Result<ReplayResult, UnknownWhy> {
    let mut checkpoint = || std::future::ready(());
    super::drive_synchronously(replay_cooperative(
        path,
        plan,
        input,
        tables,
        allow,
        extra,
        observe,
        &mut checkpoint,
    ))
}

/// Replays the public, complete route plan and never skips crossing physics.
#[allow(clippy::too_many_arguments)]
async fn replay_cooperative<F, U>(
    path: RoutePath<'_>,
    plan: &RoutePlan,
    input: &RiskInput,
    tables: &CombatTables,
    allow: WalkAllow,
    extra: Option<(i32, u8)>,
    mut observe: impl FnMut(i32, u16, i32, i32, Option<i32>),
    checkpoint: &mut F,
) -> Result<ReplayResult, UnknownWhy>
where
    F: FnMut() -> U,
    U: Future<Output = ()>,
{
    if let Some(why) = input_problem(input, tables) {
        return Err(why);
    }
    if plan.intervals.is_empty() && input.live_len == 0 && !poison_charged(input.poison) {
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
        super::checkpoint(checkpoint).await;
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
        animation_update: None,
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
        super::checkpoint(checkpoint).await;
    }
    // An endpoint does not retire an origin attacker or its queued impact.
    for live in input.live_iter() {
        duration = duration.max(origin_duration(live, tables)?);
        if let Some(due) = live.due() {
            duration = duration.max(add(i32::from(due.wrapping_sub(input.tick)), 1)?);
        }
        super::checkpoint(checkpoint).await;
    }
    for tick in 0..duration {
        while usize::from(i) + 1 < path.len()
            && path.point(i + 1).ok_or(UnknownWhy::Overflow)?.tick <= tick
        {
            i += 1;
            super::checkpoint(checkpoint).await;
        }
        let point = path.point(i);
        let input_held = point.is_some_and(|point| point.input_held(tick));
        // Entry is checked before its first hit, including transport-entered endpoints.
        let floor =
            estimated_floor_cooperative(path, plan, i, input, tables, allow, checkpoint).await?;
        let mut entry_obligation = false;
        for crossing in &plan.crossings {
            if crossing.first == i && point.is_some_and(|point| point.tick == tick) {
                entry_obligation = true;
            }
            super::checkpoint(checkpoint).await;
            if entry_obligation {
                break;
            }
        }
        if entry_obligation && floor.is_none_or(|floor| hp <= floor) {
            return Ok(ReplayResult {
                passed: false,
                hp_after: hp,
                failed_tick: Some(tick),
                bites,
            });
        }
        let landed = hits(path, plan, tick, input, tables, allow, checkpoint).await?;
        hp -= landed;
        let active_crossing = crossing_at(plan, i, checkpoint).await;
        if active_crossing.is_some() && (hp <= 0 || floor.is_none_or(|floor| hp <= floor))
            || (active_crossing.is_none() && hp <= 0)
        {
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
            let line =
                eat_line_cooperative(path, plan, i, input, tables, allow, checkpoint).await?;
            let outside_crossing = crossing_at(plan, i, checkpoint).await.is_none();
            let mut upcoming_crossing = false;
            if outside_crossing {
                for crossing in &plan.crossings {
                    let upcoming = crossing.intervals.0 != crossing.intervals.1
                        && crossing.first > i
                        && path
                            .distance(i, crossing.first)
                            .is_ok_and(|gap| gap <= TOPUP_WINDOW);
                    super::checkpoint(checkpoint).await;
                    if upcoming {
                        upcoming_crossing = true;
                        break;
                    }
                }
            }
            let smallest_heal = inventory
                .iter()
                .filter(|food| food.count > 0)
                .filter_map(|food| tables.food(food.def.id))
                .map(|food| food.heal)
                .min();
            let topup = outside_crossing
                && upcoming_crossing
                && smallest_heal.is_some_and(|heal| hp + heal <= cap);
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
                    reach: None,
                    wilderness: None,
                };
                // The replay's own line already charges the poison term
                // (`intended` above), so it passes no second fire here, or
                // one fire would count twice.
                if let Some(id) = policy::eat_choice(
                    &frame,
                    tables,
                    &schedule,
                    Some(intended / 2),
                    0,
                    hp,
                    cap,
                    tick as u16,
                    0,
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
        super::checkpoint(checkpoint).await;
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

/// The zones enforcing admission treats as granted for one found route.
#[derive(Clone, Copy)]
pub struct Grants<'a> {
    /// Request-bound named exemptions (`cross` keys or an explicit `all()`).
    pub named: BoundZoneExempt,
    /// Router endpoint completion is retained as request context but is never
    /// an admission exemption; only `named` affects the NoWayOut exception.
    pub endpoint: Option<WildernessView<'a>>,
}
impl<'a> Grants<'a> {
    /// Only the request's explicit exemptions.
    pub const fn named(named: BoundZoneExempt) -> Self {
        Self {
            named,
            endpoint: None,
        }
    }
    /// Retain router request context; endpoint completion does not grant risk.
    pub const fn request(named: BoundZoneExempt, wilderness: WildernessView<'a>) -> Self {
        Self {
            named,
            endpoint: Some(wilderness),
        }
    }
}

/// Typed computation only. No routing retry, movement, guard, or slot ownership.
/// `route` and `context.zones` must be views of the same pack.
pub fn assess(
    route: RouteView<'_>,
    context: &Estimate<'_>,
    input: RiskInput,
) -> Arc<RouteAssessment> {
    let mut checkpoint = || std::future::ready(());
    super::drive_synchronously(assess_cooperative(route, context, input, &mut checkpoint))
}
fn timeline_bound(
    path: RoutePath<'_>,
    plan: &RoutePlan,
    input: &RiskInput,
    context: &Estimate<'_>,
    poison: super::timeline::PoisonCharge,
) -> Result<(super::timeline::TimelineSummary, super::odds::DeathBound), UnknownWhy> {
    super::timeline::with_event_buffer(|events| {
        // Single-way combat (content `npc_combat.rs2:372-389`) lets one
        // attacker at a time hit the player. The schedule that counts it is
        // an upper bound of the real process, and so is the all-summed
        // schedule; the smaller of the two bounds is still an upper bound.
        let single = super::timeline::build_single_way_timeline(
            path,
            plan,
            input,
            context.combat,
            context.risks,
            poison,
            context.multiway,
            events,
        );
        let single = match single {
            Ok(summary) if summary.single_way_rows == 0 => {
                // Nothing collapsed: this schedule is the all-summed one.
                let bound = super::odds::death_bound(events.events(), input.hp)?;
                return Ok((summary, bound));
            }
            Ok(summary) => super::odds::death_bound(events.events(), input.hp)
                .map(|bound| (summary, bound))
                .ok(),
            Err(_) => None,
        };
        let summed = super::timeline::build_timeline(
            path,
            plan,
            input,
            context.combat,
            context.risks,
            poison,
            events,
        )
        .and_then(|summary| {
            super::odds::death_bound(events.events(), input.hp).map(|bound| (summary, bound))
        });
        match (single, summed) {
            (Some(single), Ok(summed)) => Ok(if score(single.1) <= score(summed.1) {
                single
            } else {
                summed
            }),
            (Some(single), Err(_)) => Ok(single),
            (None, summed) => summed,
        }
    })
}

/// The admission statistic of a bound: the closed form plus its rounding guard.
fn score(bound: super::odds::DeathBound) -> f64 {
    bound.bound + f64::from(bound.visits) * f64::EPSILON
}

async fn timeline_bound_cooperative<F, U>(
    path: RoutePath<'_>,
    plan: &RoutePlan,
    input: &RiskInput,
    context: &Estimate<'_>,
    poison: super::timeline::PoisonCharge,
    checkpoint: &mut F,
) -> Result<(super::timeline::TimelineSummary, super::odds::DeathBound), UnknownWhy>
where
    F: FnMut() -> U,
    U: Future<Output = ()>,
{
    // The thread-owned event buffer and convolution form one atomic unit.
    // Checkpoint on each side of its borrow so another assessment cannot
    // replace its events during the solve.
    super::checkpoint(checkpoint).await;
    let result = timeline_bound(path, plan, input, context, poison);
    // The event buffer borrow ended with `timeline_bound`.
    super::checkpoint(checkpoint).await;
    result
}

async fn crossing_named<F, U>(
    plan: &RoutePlan,
    crossing: &CrossingGeom,
    zones: ZoneTableView<'_>,
    grants: Option<Grants<'_>>,
    checkpoint: &mut F,
) -> Result<bool, UnknownWhy>
where
    F: FnMut() -> U,
    U: Future<Output = ()>,
{
    let Some(grants) = grants else {
        return Ok(false);
    };
    for row in rows(plan, crossing) {
        let named = zones
            .exemption_contains(grants.named, row.zone)
            .map_err(nav_identity)?;
        super::checkpoint(checkpoint).await;
        if !named {
            return Ok(false);
        }
    }
    Ok(true)
}

fn display_u8(value: u64) -> u8 {
    u8::try_from(value).unwrap_or(u8::MAX)
}

/// Cooperative route-risk assessment over borrowed facade views. Each atomic
/// timeline/convolution pass has checkpoints immediately before its event
/// buffer borrow and after the synchronous pass releases it.
pub async fn assess_cooperative<F, U>(
    route: RouteView<'_>,
    context: &Estimate<'_>,
    input: RiskInput,
    checkpoint: &mut F,
) -> Arc<RouteAssessment>
where
    F: FnMut() -> U + Send,
    U: Future<Output = ()> + Send,
{
    let mut reason = Reason::new();
    let result = assess_inner(route, context, input, &mut reason, checkpoint).await;
    match result {
        Ok(value) => Arc::new(value),
        Err(why) => {
            let _ = write!(reason, "Route assessment unknown ({why:?}): {why}");
            Arc::new(RouteAssessment {
                verdict: Verdict::Unknown(why),
                plan: RoutePlan::default(),
                crossings: Box::new([]),
                more: 0,
                supplies: Box::new([]),
                hp_after: input.hp,
                volley: u8::MAX,
                bound: 1.0,
                poison_test: PoisonTest::NotNeeded,
                input,
                generation: context.generation,
                reason: reason.arc(),
            })
        }
    }
}

async fn assess_inner<F, U>(
    route: RouteView<'_>,
    context: &Estimate<'_>,
    input: RiskInput,
    reason: &mut Reason,
    checkpoint: &mut F,
) -> Result<RouteAssessment, UnknownWhy>
where
    F: FnMut() -> U,
    U: Future<Output = ()>,
{
    let path = RoutePath::new(route)?;
    let zones = context.zones.ok_or(UnknownWhy::NoZoneTable)?;
    let plan = super::geometry::build_plan_cooperative(
        path,
        zones,
        context.risks,
        context.wilderness,
        &input,
        checkpoint,
    )
    .await?;
    // Incomplete geometry cannot support numeric risk or no-way-out decisions.
    if !plan.complete {
        return Err(UnknownWhy::Overflow);
    }

    let mut unknown = input_problem(&input, context.combat);
    if unknown.is_none()
        && context
            .grants
            .is_some_and(|grants| grants.named.pack() != zones.pack())
    {
        unknown = Some(UnknownWhy::NavIdentity);
    }
    for row in &plan.intervals {
        if let Some(class) = row.unknown_kind() {
            let _ = write!(
                reason,
                "{}: {class:?}; ",
                zones.zone(row.zone).map_err(nav_identity)?.kind().label(),
            );
            if unknown.is_none() {
                unknown = Some(match class {
                    UnknownKind::MissingFacts => UnknownWhy::MissingFacts,
                    UnknownKind::Overflow => UnknownWhy::Overflow,
                    _ => UnknownWhy::Kind(class),
                });
            }
            break;
        }
        super::checkpoint(checkpoint).await;
    }

    let unknown_poison =
        matches!(input.poison, PoisonState::Unknown { .. }) && !plan.crossings.is_empty();
    let mut unknown_poison_q = None;
    let mut known_poison = super::timeline::PoisonCharge::None;
    if unknown.is_none() {
        if unknown_poison {
            let q = context.risks.poison_unknown_v;
            if q == u8::MAX {
                unknown = Some(UnknownWhy::Poison);
            } else {
                unknown_poison_q = Some(q);
            }
        } else {
            match super::timeline::known_poison_charge(
                input.poison,
                input.tick,
                context.risks.poison_unknown_v,
            ) {
                Ok(charge) => known_poison = charge,
                Err(why) => unknown = Some(why),
            }
        }
    }

    let mut verdict = unknown.map_or(Verdict::Survivable, Verdict::Unknown);
    let mut bound = 1.0;
    let mut poison_test = PoisonTest::NotNeeded;
    if unknown.is_none() {
        if let Some(q) = unknown_poison_q {
            let (_, clear) = timeline_bound_cooperative(
                path,
                &plan,
                &input,
                context,
                super::timeline::PoisonCharge::None,
                checkpoint,
            )
            .await?;
            let (_, charged) = timeline_bound_cooperative(
                path,
                &plan,
                &input,
                context,
                super::timeline::PoisonCharge::Unknown { q },
                checkpoint,
            )
            .await?;
            bound = clear.bound;
            if charged.admits(super::consts::DEATH_BOUND) {
                bound = charged.bound;
                poison_test = PoisonTest::PassesCharged {
                    charged: charged.bound,
                };
            } else if clear.admits(super::consts::DEATH_BOUND) {
                if let Some(crossing) = plan.crossings.first() {
                    if crossing.retreat != CrossingGeom::NONE {
                        poison_test = PoisonTest::WaitForClear {
                            charged: charged.bound,
                            until_index: crossing.retreat,
                        };
                    } else {
                        poison_test = PoisonTest::NoSafeTile {
                            charged: charged.bound,
                        };
                    }
                }
            } else {
                verdict = Verdict::Unsurvivable;
            }
        } else {
            let (_, result) =
                timeline_bound_cooperative(path, &plan, &input, context, known_poison, checkpoint)
                    .await?;
            bound = result.bound;
            if !result.admits(super::consts::DEATH_BOUND) {
                verdict = Verdict::Unsurvivable;
            }
        }
    }

    let mut no_way_out = None;
    if unknown.is_none() {
        for crossing in &plan.crossings {
            if !crossing.has_way_out()
                && !crossing_named(&plan, crossing, zones, context.grants, checkpoint).await?
            {
                no_way_out = Some(*crossing);
                break;
            }
            super::checkpoint(checkpoint).await;
        }
    }
    if no_way_out.is_some() {
        verdict = Verdict::Unsurvivable;
    }

    let mut display = Vec::with_capacity(plan.crossings.len().min(8));
    let mut origin_volley = 0u64;
    let mut origin_worst = 0u64;
    let mut unaffordable_floor = None;
    if unknown.is_none() {
        for live in input.live_iter() {
            let duration = origin_duration(live, context.combat)?;
            origin_volley = origin_volley.saturating_add(u64::from(live.max_hit));
            let events = u64::try_from(ceil(duration, i32::from(live.rate))?)
                .map_err(|_| UnknownWhy::Overflow)?;
            origin_worst =
                origin_worst.saturating_add(u64::from(live.max_hit).saturating_mul(events));
            if live.due().is_some() {
                origin_worst = origin_worst.saturating_add(u64::from(live.max_hit));
            }
            super::checkpoint(checkpoint).await;
        }
    }
    let mut max_volley = origin_volley;
    for (index, crossing) in plan.crossings.iter().enumerate() {
        let mut volley_max = 0u64;
        let mut floor_deep = 0u64;
        let credited = if unknown.is_none() {
            protect(
                crossing,
                &plan,
                path,
                &input,
                context.combat,
                context.allow,
                checkpoint,
            )
            .await
        } else {
            false
        };
        for i in crossing.first..=crossing.last {
            volley_max = volley_max.max(
                u64::try_from(volley(&plan, i, false, checkpoint).await?)
                    .map_err(|_| UnknownWhy::Overflow)?,
            );
            if unknown.is_none() {
                let floor = estimated_floor_cooperative(
                    path,
                    &plan,
                    i,
                    &input,
                    context.combat,
                    context.allow,
                    checkpoint,
                )
                .await?;
                floor_deep =
                    floor_deep.max(u64::try_from(floor.unwrap_or(255).max(0)).unwrap_or(u64::MAX));
                if unaffordable_floor.is_none()
                    && floor.is_some_and(|cost| cost >= i32::from(input.hp_max))
                {
                    unaffordable_floor = Some((i, floor.unwrap_or(255)));
                }
            }
            super::checkpoint(checkpoint).await;
        }
        let mut worst = if credited { volley_max } else { 0 };
        if unknown.is_none() && !credited {
            for row in rows(&plan, crossing) {
                let (start, end) = exposure(path, row)?;
                let count = if row.hazard() {
                    1
                } else {
                    u64::try_from(ceil(end - start, i32::from(row.rate))?)
                        .map_err(|_| UnknownWhy::Overflow)?
                };
                worst = worst.saturating_add(u64::from(row.max_hit).saturating_mul(count));
                super::checkpoint(checkpoint).await;
            }
        }
        if index == 0 {
            volley_max = volley_max.saturating_add(origin_volley);
            worst = worst.saturating_add(origin_worst);
        }
        max_volley = max_volley.max(volley_max);
        let ticks = path.exposure_ticks(crossing.first, crossing.last)?;
        let ticks = u16::try_from(ticks).unwrap_or(u16::MAX);
        if display.len() < 8 {
            let intervals = rows(&plan, crossing);
            let first = intervals.first().ok_or(UnknownWhy::Overflow)?;
            let max_hit = display_u8(
                u64::try_from(margin(&plan, crossing, checkpoint).await).unwrap_or(u64::MAX),
            );
            let mut rate = u8::MAX;
            let mut has_unknown = false;
            for row in intervals {
                rate = rate.min(row.rate);
                has_unknown |= row.unknown();
                super::checkpoint(checkpoint).await;
            }
            display.push(Crossing {
                key: first.key(zones)?,
                first: crossing.first,
                last: crossing.last,
                ticks,
                worst: u16::try_from(worst).unwrap_or(u16::MAX),
                volley: display_u8(volley_max),
                max_hit,
                rate: if intervals.is_empty() { 0 } else { rate },
                style: first.style,
                floor_deep: display_u8(floor_deep),
                bites: 0,
                single: false,
                protect_credited: credited,
                unknown: has_unknown,
            });
        }
        super::checkpoint(checkpoint).await;
    }

    let mut needs = [None; 6];
    let mut need_count = 0;
    for style in [Style::Melee, Style::Ranged] {
        let Some(prayer) = select::protect_fact(context.combat, style.into()) else {
            continue;
        };
        let mut needed = None;
        for crossing in &plan.crossings {
            let mut same_style = true;
            for row in rows(&plan, crossing) {
                if row.unknown() || row.style != style {
                    same_style = false;
                }
                super::checkpoint(checkpoint).await;
                if !same_style {
                    break;
                }
            }
            if same_style
                && !protect(
                    crossing,
                    &plan,
                    path,
                    &input,
                    context.combat,
                    context.allow,
                    checkpoint,
                )
                .await
            {
                if let Some(points) = prayer_points_needed(path, crossing) {
                    needed = Some(needed.map_or(points, |old: i32| old.max(points)));
                }
            }
            super::checkpoint(checkpoint).await;
        }
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
    if unknown.is_none() {
        let poison_need = match input.poison {
            PoisonState::Onset { .. } => Some((context.risks.poison_unknown_v, true)),
            PoisonState::Poisoned {
                per_tick,
                hit_since_mark,
                ..
            } => Some((
                if hit_since_mark {
                    per_tick.max(context.risks.poison_unknown_v)
                } else {
                    per_tick
                },
                false,
            )),
            PoisonState::Unknown { .. } | PoisonState::Clear | PoisonState::Immune { .. } => None,
        };
        if let Some((pulse, onset)) = poison_need.filter(|(pulse, _)| *pulse != 0) {
            needs[need_count] = Some(SupplyNeed::Info(InfoNeed::Antipoison));
            need_count += 1;
            if onset {
                let _ = write!(
                    reason,
                    "poisoned (onset, up to {pulse} per 30 ticks): drink an antipoison to clear; "
                );
            } else {
                let _ = write!(
                    reason,
                    "poisoned ({pulse} per 30 ticks): drink an antipoison to clear; "
                );
            }
        }
    }
    let mut supplies = Vec::with_capacity(need_count);
    supplies.extend(needs.into_iter().flatten());

    match verdict {
        Verdict::Survivable => {
            let _ = write!(
                reason,
                "chance of death at most {:.1}% (bound {:.0}%), without eating, prayer or escape counted; ",
                bound * 100.0,
                super::consts::DEATH_BOUND * 100.0,
            );
        }
        Verdict::Unsurvivable => {
            let _ = write!(
                reason,
                "the model's upper bound on the chance of death is {:.1}%; at most {:.0}% is admitted (no eating, prayer or escape counted); ",
                bound * 100.0,
                super::consts::DEATH_BOUND * 100.0,
            );
        }
        Verdict::Unknown(why) => {
            let _ = write!(reason, "Route assessment unknown: {why}; ");
        }
    }
    match poison_test {
        PoisonTest::NotNeeded => {}
        PoisonTest::PassesCharged { charged } => {
            let _ = write!(
                reason,
                "unknown poison charged bound {:.1}% passes; ",
                charged * 100.0
            );
        }
        PoisonTest::WaitForClear {
            charged,
            until_index,
        } => {
            let _ = write!(
                reason,
                "unknown poison charged bound {:.1}% fails; wait for Clear at retreat index {until_index} (clear bound {:.1}%); ",
                charged * 100.0,
                bound * 100.0,
            );
        }
        PoisonTest::NoSafeTile { charged } => {
            let _ = write!(
                reason,
                "poison state unknown; the route starts inside a crossing, so there is no tile to wait on; admitted on the poison-Clear bound {:.1}% (charged {:.1}%); ",
                bound * 100.0,
                charged * 100.0,
            );
        }
    }
    if let Some(crossing) = no_way_out {
        let ending = rows(&plan, &crossing).last().ok_or(UnknownWhy::Overflow)?;
        let kind = zones.zone(ending.zone).map_err(nav_identity)?.kind();
        let _ = write!(reason, "NoWayOut: ends inside {}; ", kind.label());
        if let Some(LegView::Transport(hop)) = usize::from(crossing.leg)
            .checked_sub(1)
            .and_then(|leg| route.leg(leg).ok())
        {
            let _ = write!(reason, "entered by {:?}; ", hop.kind());
        } else {
            let _ = write!(reason, "standing inside or transport-entered; ");
        }
        let _ = write!(reason, "no way out on foot; ");
    }
    if let Some((index, cost)) = unaffordable_floor {
        let point = path.point(index).ok_or(UnknownWhy::Overflow)?;
        let _ = write!(
            reason,
            "way out at {},{} costs {cost}, max HP {}; ",
            point.tile.x, point.tile.z, input.hp_max
        );
    }
    if let Some(row) = plan.intervals.first() {
        let kind = zones.zone(row.zone).map_err(nav_identity)?.kind();
        let _ = write!(
            reason,
            "{}{} at {},{}; ",
            if row.a == 0 {
                "standing inside "
            } else {
                "Route crosses "
            },
            kind.label(),
            row.spawn.tile().x,
            row.spawn.tile().z
        );
    }

    let more =
        display_u8(u64::try_from(plan.crossings.len().saturating_sub(8)).unwrap_or(u64::MAX));
    Ok(RouteAssessment {
        verdict,
        plan,
        crossings: display.into_boxed_slice(),
        more,
        supplies: supplies.into_boxed_slice(),
        hp_after: input.hp,
        volley: display_u8(max_volley),
        bound,
        poison_test,
        input,
        generation: context.generation,
        reason: reason.arc(),
    })
}
