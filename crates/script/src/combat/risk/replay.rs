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
use nav::contract::{BoundZoneExempt, BoundZoneKey, LegView, RouteView, WildernessView};
use std::fmt::{self, Write};
use std::future::Future;

/// The owner-boundary facts an assessment runs over: the pack's zone table
/// and wilderness views (both of the route's pack), the NPC-keyed risk
/// facts, and the selected combat tables.
#[derive(Clone, Copy)]
pub struct Estimate<'a> {
    pub zones: Option<ZoneTableView<'a>>,
    pub wilderness: WildernessView<'a>,
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
/// Empty ranges keep granted unknown crossings index-aligned but out of physics.
async fn crossing_with_index_at<'a, F, U>(
    plan: &'a RoutePlan,
    index: u16,
    checkpoint: &mut F,
) -> Option<(usize, &'a CrossingGeom)>
where
    F: FnMut() -> U,
    U: Future<Output = ()>,
{
    for (crossing_index, crossing) in plan.crossings.iter().enumerate() {
        let found = crossing.intervals.0 != crossing.intervals.1
            && crossing.first <= index
            && index <= crossing.last;
        super::checkpoint(checkpoint).await;
        if found {
            return Some((crossing_index, crossing));
        }
    }
    None
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
    crossing_with_index_at(plan, index, checkpoint)
        .await
        .map(|(_, crossing)| crossing)
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
    if !allow.prayer || input.other_prayers_on || input.off_debt {
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
    let npc = tables.npc(live.ident).ok_or(UnknownWhy::AlreadyEngaged)?;
    let r = super::facts::npc_envelope_radius(npc).ok_or(UnknownWhy::Overflow)?;
    add(mul(i32::from(r), 2)?, LAG)
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
    observe: impl FnMut(i32, u16, i32, i32, Option<i32>),
) -> Result<ReplayResult, UnknownWhy> {
    let mut checkpoint = || std::future::ready(());
    super::drive_synchronously(replay_with_admission_cooperative(
        path,
        plan,
        input,
        tables,
        allow,
        extra,
        |_, _| true,
        observe,
        &mut checkpoint,
    ))
}

/// Reuse the physical timeline while only requiring ungranted crossings to
/// pass. Grants bless admission, never remove their damage or food consumption.
#[allow(clippy::too_many_arguments)]
async fn replay_with_admission_cooperative<F, U>(
    path: RoutePath<'_>,
    plan: &RoutePlan,
    input: &RiskInput,
    tables: &CombatTables,
    allow: WalkAllow,
    extra: Option<(i32, u8)>,
    must_pass: impl Fn(usize, &CrossingGeom) -> bool,
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
        for (index, crossing) in plan.crossings.iter().enumerate() {
            if must_pass(index, crossing)
                && crossing.first == i
                && point.is_some_and(|point| point.tick == tick)
            {
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
        let active_crossing = crossing_with_index_at(plan, i, checkpoint).await;
        if active_crossing.is_some_and(|(index, crossing)| must_pass(index, crossing))
            && (hp <= 0 || floor.is_none_or(|floor| hp <= floor))
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

/// The one sentence a person sees when the route's poison state is unknown.
const POISON_UNKNOWN_REASON: &str = "Poison status is unknown until the bot can track it";
/// Why that is, for the debug log only.
const POISON_UNKNOWN_DETAIL: &str = "poison state unknown: a refused crossing is not armed and \
     cannot clear its own poison uncertainty, and waiting alone does not clear it; a separate \
     safe-only walk can provide recovery evidence once the runtime observer is available";

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

/// Whether every row of `crossing` is granted (`zone_granted` is fallible:
/// a key from another pack refuses the whole admission).
async fn crossing_granted_cooperative<F, U>(
    plan: &RoutePlan,
    crossing: &CrossingGeom,
    zone_granted: &impl Fn(BoundZoneKey) -> Result<bool, UnknownWhy>,
    checkpoint: &mut F,
) -> Result<bool, UnknownWhy>
where
    F: FnMut() -> U,
    U: Future<Output = ()>,
{
    for row in rows(plan, crossing) {
        let granted = zone_granted(row.zone)?;
        super::checkpoint(checkpoint).await;
        if !granted {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Copies the plan only when granted unknown rows must be omitted from replay.
/// The retained assessment stays complete and the all-known path stays borrowed.
async fn omit_granted_unknowns_cooperative<F, U>(
    plan: &RoutePlan,
    zone_granted: &impl Fn(BoundZoneKey) -> Result<bool, UnknownWhy>,
    checkpoint: &mut F,
) -> Result<Option<RoutePlan>, UnknownWhy>
where
    F: FnMut() -> U,
    U: Future<Output = ()>,
{
    let mut has_granted_unknown = false;
    for crossing in &plan.crossings {
        if crossing_granted_cooperative(plan, crossing, zone_granted, checkpoint).await? {
            for row in rows(plan, crossing) {
                if row.unknown() {
                    has_granted_unknown = true;
                }
                super::checkpoint(checkpoint).await;
                if has_granted_unknown {
                    break;
                }
            }
        }
        super::checkpoint(checkpoint).await;
        if has_granted_unknown {
            break;
        }
    }
    if !has_granted_unknown {
        return Ok(None);
    }

    let escape_end = plan
        .crossings
        .first()
        .map_or(plan.intervals.len(), |c| usize::from(c.intervals.0));
    let mut interval_count = escape_end;
    for crossing in &plan.crossings {
        let granted =
            crossing_granted_cooperative(plan, crossing, zone_granted, checkpoint).await?;
        let _ = u16::try_from(interval_count).map_err(|_| UnknownWhy::Overflow)?;
        for row in rows(plan, crossing) {
            if !granted || !row.unknown() {
                interval_count += 1;
            }
            super::checkpoint(checkpoint).await;
        }
        let _ = u16::try_from(interval_count).map_err(|_| UnknownWhy::Overflow)?;
        super::checkpoint(checkpoint).await;
    }

    let mut intervals = Vec::with_capacity(interval_count);
    let mut crossings = Vec::with_capacity(plan.crossings.len());
    for row in &plan.intervals[..escape_end] {
        intervals.push(*row);
        super::checkpoint(checkpoint).await;
    }
    for crossing in &plan.crossings {
        let granted =
            crossing_granted_cooperative(plan, crossing, zone_granted, checkpoint).await?;
        let start = u16::try_from(intervals.len()).map_err(|_| UnknownWhy::Overflow)?;
        for row in rows(plan, crossing) {
            if !granted || !row.unknown() {
                intervals.push(*row);
            }
            super::checkpoint(checkpoint).await;
        }
        let end = u16::try_from(intervals.len()).map_err(|_| UnknownWhy::Overflow)?;
        let mut projected = *crossing;
        projected.intervals = (start, end);
        crossings.push(projected);
        super::checkpoint(checkpoint).await;
    }

    Ok(Some(RoutePlan {
        complete: plan.complete,
        pack: plan.pack,
        intervals: intervals.into_boxed_slice(),
        crossings: crossings.into_boxed_slice(),
    }))
}

/// The zones enforcing admission treats as granted for one found route.
#[derive(Clone, Copy)]
pub struct Grants<'a> {
    /// The request's router exemptions, bound to the route's pack: named
    /// `cross` keys, or `all()`.
    pub named: BoundZoneExempt,
    /// When set, and the request named grants, zones engaged at the route's
    /// endpoint are granted as well.
    pub endpoint: Option<WildernessView<'a>>,
}
impl<'a> Grants<'a> {
    /// Only the request's exemptions; an endpoint crossing is judged.
    pub const fn named(named: BoundZoneExempt) -> Self {
        Self {
            named,
            endpoint: None,
        }
    }
    /// A walk that names grants (Path `cross`) relies on the router's
    /// endpoint completion for the zone it ends in: its destination zones
    /// are admitted with its named ones. Without named grants, endpoint
    /// crossings stay judged; transit through any unnamed zone always is.
    /// (Whether grants were named is decided against the pack's table in
    /// [`admission_passes`]; a bound exemption does not expose its keys.)
    pub const fn request(named: BoundZoneExempt, wilderness: WildernessView<'a>) -> Self {
        Self {
            named,
            endpoint: Some(wilderness),
        }
    }
}

/// Admission checks the complete plan, not the eight display rows. A fully
/// granted crossing is exempt from refusal; unknown physics in a mixed
/// crossing is not. Known granted damage remains in every subsequent floor.
/// A failed assessment's incomplete plan is never admitted: replaying its
/// empty plan would certify nothing. The route, plan and grants must all
/// belong to `zones`'s pack ([`UnknownWhy::NavIdentity`] otherwise).
pub fn admission_passes(
    route: RouteView<'_>,
    assessment: &RouteAssessment,
    zones: ZoneTableView<'_>,
    tables: &CombatTables,
    allow: WalkAllow,
    grants: Grants<'_>,
) -> Result<bool, UnknownWhy> {
    let mut checkpoint = || std::future::ready(());
    super::drive_synchronously(admission_passes_cooperative(
        route,
        assessment,
        zones,
        tables,
        allow,
        grants,
        &mut checkpoint,
    ))
}

/// Cooperative form of admission replay. The callback is awaited after each
/// bounded geometry, grant, projection, scan, or replay unit.
pub async fn admission_passes_cooperative<F, U>(
    route: RouteView<'_>,
    assessment: &RouteAssessment,
    zones: ZoneTableView<'_>,
    tables: &CombatTables,
    allow: WalkAllow,
    grants: Grants<'_>,
    checkpoint: &mut F,
) -> Result<bool, UnknownWhy>
where
    F: FnMut() -> U + Send,
    U: Future<Output = ()> + Send,
{
    let plan = &assessment.plan;
    if !plan.complete {
        return Err(match assessment.verdict {
            Verdict::Unknown(why) => why,
            _ => UnknownWhy::MissingFacts,
        });
    }
    if plan.pack != Some(zones.pack())
        || route.pack() != zones.pack()
        || grants.named.pack() != zones.pack()
    {
        return Err(UnknownWhy::NavIdentity);
    }
    // Endpoint grants apply only when the request named any exemption.
    let none = zones.exempt(&[]).map_err(nav_identity)?;
    let endpoint = grants.endpoint.filter(|_| grants.named != none);
    let zone_granted = |zone: BoundZoneKey| -> Result<bool, UnknownWhy> {
        if zones
            .exemption_contains(grants.named, zone)
            .map_err(nav_identity)?
        {
            return Ok(true);
        }
        let Some(wilderness) = endpoint else {
            return Ok(false);
        };
        let view = zones.zone(zone).map_err(nav_identity)?;
        Ok(super::geometry::engaged_at(
            view,
            &assessment.input,
            wilderness,
            route.destination(),
        ))
    };
    let mut all_granted = true;
    for crossing in &plan.crossings {
        if !crossing_granted_cooperative(plan, crossing, &zone_granted, checkpoint).await? {
            all_granted = false;
            break;
        }
        super::checkpoint(checkpoint).await;
    }
    if all_granted {
        return Ok(true);
    }
    if let Some(why) = input_problem(&assessment.input, tables) {
        return Err(why);
    }
    let words = plan.crossings.len().div_ceil(64);
    let mut must_pass = Vec::with_capacity(words);
    for _ in 0..words {
        must_pass.push(0u64);
        super::checkpoint(checkpoint).await;
    }
    for (index, crossing) in plan.crossings.iter().enumerate() {
        let granted =
            crossing_granted_cooperative(plan, crossing, &zone_granted, checkpoint).await?;
        if !granted {
            must_pass[index / 64] |= 1u64 << (index % 64);
            let mut unknown = None;
            for row in rows(plan, crossing) {
                unknown = row.unknown_kind();
                super::checkpoint(checkpoint).await;
                if unknown.is_some() {
                    break;
                }
            }
            if let Some(class) = unknown {
                return Err(UnknownWhy::Kind(class));
            }
            if matches!(assessment.input.poison, PoisonState::Unknown { .. }) {
                return Err(UnknownWhy::Poison);
            }
            if !crossing.has_way_out() {
                return Ok(false);
            }
        }
        super::checkpoint(checkpoint).await;
    }
    let projected = omit_granted_unknowns_cooperative(plan, &zone_granted, checkpoint).await?;
    let replay_plan = projected.as_ref().unwrap_or(plan);
    let path = RoutePath::new(route)?;
    replay_with_admission_cooperative(
        path,
        replay_plan,
        &assessment.input,
        tables,
        allow,
        None,
        |index, _| {
            must_pass
                .get(index / 64)
                .is_some_and(|word| word & (1u64 << (index % 64)) != 0)
        },
        |_, _, _, _, _| {},
        checkpoint,
    )
    .await
    .map(|result| result.passed)
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

/// Cooperative route-risk assessment over borrowed facade views. Each
/// checkpoint follows one bounded geometry, report, replay, or food unit.
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
    let mut unknown = input_problem(&input, context.combat);
    for row in &plan.intervals {
        if let Some(class) = row.unknown_kind() {
            let _ = write!(
                reason,
                "{}: {:?}; ",
                zones.zone(row.zone).map_err(nav_identity)?.kind().label(),
                Some(class)
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
            super::checkpoint(checkpoint).await;
        }
    }
    let mut max_volley = origin_volley;
    for (index, c) in plan.crossings.iter().enumerate() {
        let mut v = 0;
        let mut deep = 0;
        let credited = if unknown.is_none() {
            protect(
                c,
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
        for i in c.first..=c.last {
            v = v.max(volley(&plan, i, false, checkpoint).await?);
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
                deep = deep.max(floor.unwrap_or(255));
                if unaffordable_floor.is_none()
                    && floor.is_some_and(|cost| cost >= i32::from(input.hp_max))
                {
                    unaffordable_floor = Some((i, floor.unwrap_or(255)));
                }
            }
            super::checkpoint(checkpoint).await;
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
                super::checkpoint(checkpoint).await;
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
            let intervals = rows(&plan, c);
            let first = intervals[0];
            let max_hit = margin(&plan, c, checkpoint).await as u8;
            let mut rate = u8::MAX;
            let mut has_unknown = false;
            for row in intervals {
                rate = rate.min(row.rate);
                has_unknown |= row.unknown();
                super::checkpoint(checkpoint).await;
            }
            display.push(Crossing {
                key: first.key(zones)?,
                first: c.first,
                last: c.last,
                ticks,
                worst,
                volley: v as u8,
                max_hit,
                rate: if intervals.is_empty() { 0 } else { rate },
                style: first.style,
                floor_deep: deep as u8,
                bites: 0,
                single: false,
                protect_credited: credited,
                unknown: has_unknown,
            });
        }
        super::checkpoint(checkpoint).await;
    }

    let mut no_way_out = None;
    for crossing in &plan.crossings {
        if !crossing.has_way_out() {
            no_way_out = Some(crossing);
            break;
        }
        super::checkpoint(checkpoint).await;
    }
    let mut verdict = unknown.map_or(Verdict::Survivable, Verdict::Unknown);
    let mut hp_after = input.hp;
    let mut bites = 0;
    let mut extra_food = None;
    if unknown.is_none() {
        if no_way_out.is_some() || unaffordable_floor.is_some() {
            verdict = Verdict::Unsurvivable;
        } else {
            let baseline = replay_with_admission_cooperative(
                path,
                &plan,
                &input,
                context.combat,
                context.allow,
                None,
                |_, _| true,
                |_, _, _, _, _| {},
                checkpoint,
            )
            .await?;
            hp_after = bounded_hp(baseline.hp_after)?;
            bites = baseline.bites;
            if !baseline.passed
                && (!plan.intervals.is_empty()
                    || input.live_len != 0
                    || poison_charged(input.poison))
            {
                verdict = Verdict::Unsurvivable;
                let mut carried = None;
                for (id, count) in input.food_iter() {
                    if count > 0 {
                        if let Some(food) = context.combat.food(id).filter(|food| ordinary(food)) {
                            if carried.is_none_or(|best: &FoodFact| food.heal >= best.heal) {
                                carried = Some(food);
                            }
                        }
                    }
                    super::checkpoint(checkpoint).await;
                }
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
                            let candidate = replay_with_admission_cooperative(
                                path,
                                &plan,
                                &input,
                                context.combat,
                                context.allow,
                                Some((food.item_id, count)),
                                |_, _| true,
                                |_, _, _, _, _| {},
                                checkpoint,
                            )
                            .await?;
                            super::checkpoint(checkpoint).await;
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
        super::checkpoint(checkpoint).await;
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
        let mut needed = None;
        for c in &plan.crossings {
            let mut same_style = true;
            for row in rows(&plan, c) {
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
                    c,
                    &plan,
                    path,
                    &input,
                    context.combat,
                    context.allow,
                    checkpoint,
                )
                .await
            {
                if let Some(points) = prayer_points_needed(path, c) {
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
    if let Some((pulse, onset)) = poison_need {
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
    let mut supplies = Vec::with_capacity(need_count);
    supplies.extend(needs.into_iter().flatten());
    if matches!(verdict, Verdict::Unknown(UnknownWhy::Poison)) {
        // What the operator reads, in the panel, the TUI and the session log.
        let _ = write!(reason, "{POISON_UNKNOWN_REASON}; ");
        api::host_log!(
            api::hostlog::Category::NavTrace,
            api::hostlog::Level::Debug,
            "{POISON_UNKNOWN_DETAIL}"
        );
    }
    if let Some(c) = no_way_out {
        let ending = rows(&plan, c).last().ok_or(UnknownWhy::Overflow)?;
        let kind = zones.zone(ending.zone).map_err(nav_identity)?.kind();
        let _ = write!(reason, "NoWayOut: ends inside {}; ", kind.label());
        if let Some(LegView::Transport(hop)) = usize::from(c.leg)
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
            "way out at {},{} costs {cost}, at least max HP {}; ",
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
        super::checkpoint(checkpoint).await;
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
