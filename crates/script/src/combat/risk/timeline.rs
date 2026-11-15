use super::consts::{LAG, MAX_EVENTS, POISON_PERIOD, SINGLE_WAY_LULL};
use super::odds::{hit_chance, Event, EventBuf, EventKind, HitChance};
use super::{RiskInput, RiskTables, RoutePath, RoutePlan, UnknownWhy, ZoneInterval};
use crate::combat::tables::CombatTables;
use api::condition::PoisonState;
use nav_planner::contract::MultiwayView;
use std::cell::RefCell;

/// Poison events to charge in addition to poison inflicted by exposed NPCs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PoisonCharge {
    None,
    Known { per_tick: u8, first: u16 },
    Unknown { q: u8 },
}

/// Event counts and the final inclusive route tick represented by the schedule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TimelineSummary {
    pub attacks: u16,
    pub fixed: u16,
    pub ticks: u16,
    /// Rows whose own schedule a single-way cluster replaced (zero for the
    /// all-summed schedule).
    pub single_way_rows: u16,
}
pub(crate) struct TimelineInputs<'a> {
    path: RoutePath<'a>,
    plan: &'a RoutePlan,
    input: &'a RiskInput,
    tables: &'a CombatTables,
    risks: &'a RiskTables,
    multiway: MultiwayView<'a>,
}

impl<'a> TimelineInputs<'a> {
    pub(crate) fn new(
        path: RoutePath<'a>,
        plan: &'a RoutePlan,
        input: &'a RiskInput,
        tables: &'a CombatTables,
        risks: &'a RiskTables,
        multiway: MultiwayView<'a>,
    ) -> Self {
        Self {
            path,
            plan,
            input,
            tables,
            risks,
            multiway,
        }
    }
}

struct TimelineEvents<'a> {
    out: &'a mut EventBuf,
    summary: TimelineSummary,
}

thread_local! {
    static EVENT_BUFFER: RefCell<EventBuf> = RefCell::new(EventBuf::new());
}

/// Reuse the worker thread's one lazily allocated event buffer. The borrow is
/// deliberately confined to synchronous timeline construction and convolution.
pub(super) fn with_event_buffer<T>(f: impl FnOnce(&mut EventBuf) -> T) -> T {
    EVENT_BUFFER.with(|buffer| f(&mut buffer.borrow_mut()))
}

fn add_event(
    out: &mut EventBuf,
    summary: &mut TimelineSummary,
    tick: i32,
    kind: EventKind,
    amount: u8,
    hit: HitChance,
) -> Result<(), UnknownWhy> {
    let tick = u16::try_from(tick).map_err(|_| UnknownWhy::Overflow)?;
    out.push(Event {
        tick,
        kind,
        amount,
        hit,
    })?;
    let count = match kind {
        EventKind::Attack => &mut summary.attacks,
        EventKind::Certain | EventKind::Poison => &mut summary.fixed,
    };
    *count = count.checked_add(1).ok_or(UnknownWhy::Overflow)?;
    if usize::from(summary.attacks) + usize::from(summary.fixed) > MAX_EVENTS {
        return Err(UnknownWhy::Overflow);
    }
    Ok(())
}

fn relative_first_tick(now: u16, mark: u16) -> u16 {
    let elapsed = now.wrapping_sub(mark) % POISON_PERIOD as u16;
    POISON_PERIOD as u16 - elapsed
}

fn damage_type_index(name: &str) -> Option<usize> {
    match name {
        "^damagetype_stab" | "^stab_style" => Some(0),
        "^damagetype_slash" | "^slash_style" => Some(1),
        "^damagetype_crush" | "^crush_style" => Some(2),
        "^damagetype_ranged" | "^ranged_style" => Some(3),
        _ => None,
    }
}

pub(super) fn npc_hit(
    ident: i32,
    attack_roll: u32,
    input: &RiskInput,
    tables: &CombatTables,
) -> Result<HitChance, UnknownWhy> {
    let row = super::facts::effective_npc_row(ident, tables).ok_or(UnknownWhy::MissingFacts)?;
    let rolls = super::facts::defence_rolls(input.defence, input.style_def, input.worn_defence)?;
    let defence_roll = match row.damagetype.as_deref().and_then(damage_type_index) {
        Some(index) => rolls[index],
        None if row.damagetype.as_deref() == Some("^damagetype_melee") => rolls[..3]
            .iter()
            .copied()
            .min()
            .ok_or(UnknownWhy::MissingFacts)?,
        None => return Err(UnknownWhy::MissingFacts),
    };
    Ok(HitChance::ceil_from(hit_chance(attack_roll, defence_roll)))
}

fn attack_duration(live: &super::LiveRow, risks: &RiskTables) -> Result<i32, UnknownWhy> {
    let kind = risks.npc(live.ident).ok_or(UnknownWhy::AlreadyEngaged)?;
    if kind.r == 0 {
        return Err(UnknownWhy::Overflow);
    }
    i32::from(kind.r)
        .checked_mul(2)
        .and_then(|ticks| ticks.checked_add(LAG))
        .ok_or(UnknownWhy::Overflow)
}

fn extend_poison(
    out: &mut EventBuf,
    summary: &mut TimelineSummary,
    amount: u8,
    first: i32,
    last: i32,
) -> Result<(), UnknownWhy> {
    if amount == 0 || first > last {
        return Ok(());
    }
    let period = POISON_PERIOD;
    let mut tick = first;
    while tick <= last {
        add_event(
            out,
            summary,
            tick,
            EventKind::Poison,
            amount,
            HitChance::ceil_from(0.0),
        )?;
        tick = tick.checked_add(period).ok_or(UnknownWhy::Overflow)?;
    }
    Ok(())
}

fn kind_attack(
    row: &ZoneInterval,
    input: &RiskInput,
    risks: &RiskTables,
    tables: &CombatTables,
) -> Result<(EventKind, HitChance, u8), UnknownWhy> {
    let kind = risks.npc(row.ident()).ok_or(UnknownWhy::MissingFacts)?;
    if kind.unknown_for().is_some() {
        return Err(UnknownWhy::Kind(kind.unknown));
    }
    let rate = row.rate;
    if rate == 0 {
        return Err(UnknownWhy::MissingFacts);
    }
    if kind.shared_attack() {
        Ok((
            EventKind::Attack,
            npc_hit(row.ident(), kind.attack_roll, input, tables)?,
            rate,
        ))
    } else {
        Ok((EventKind::Certain, HitChance::ceil_from(0.0), rate))
    }
}

/// One row's own schedule: an attack every `rate` ticks over its interval.
fn add_row_events(
    events: &mut TimelineEvents<'_>,
    row: &ZoneInterval,
    kind: EventKind,
    hit: HitChance,
    rate: u8,
    start: i32,
    end: i32,
) -> Result<(), UnknownWhy> {
    let mut tick = start;
    while tick < end {
        add_event(
            events.out,
            &mut events.summary,
            tick,
            kind,
            row.max_hit,
            hit,
        )?;
        tick = tick
            .checked_add(i32::from(rate))
            .ok_or(UnknownWhy::Overflow)?;
    }
    Ok(())
}

/// Build the complete, summed, own-rate event schedule for one route.
pub(crate) fn build_timeline(
    path: RoutePath<'_>,
    plan: &RoutePlan,
    input: &RiskInput,
    tables: &CombatTables,
    risks: &RiskTables,
    poison: PoisonCharge,
    out: &mut EventBuf,
) -> Result<TimelineSummary, UnknownWhy> {
    build(
        TimelineInputs::new(path, plan, input, tables, risks, MultiwayView::none()),
        poison,
        out,
    )
}

/// The schedule with single-way combat counted: in a non-multiway area the
/// content lets one NPC at a time attack a player. Content
/// `npc_combat.rs2:372-389` (`npc_check_notcombat`, `npc_check_notcombat_self`)
/// is called at the start of every shared melee, ranged and magic attack
/// (`npc_combat.rs2:63-67`, `npc_combat_ranged.rs2:20-24`,
/// `npc_combat_magic.rs2:60-64`): while the player was in combat within 8
/// ticks and `%aggressive_npc` is another NPC, the attack does not happen.
/// An NPC with `npc_forcemulti`, an NPC on a multiway cell
/// (`maps/multiway.csv`) and a bespoke attack skip that rule, so they keep
/// their own schedules.
///
/// Rows that can attack in single-way combat and whose intervals overlap form
/// one cluster, whatever their kinds. Its attacks are at most one per `g`
/// ticks over the union of its intervals, where `g` is its smallest attack
/// interval (at most 8): the same NPC repeats at its own interval, and a
/// different NPC may attack only 8 ticks after the player's last combat. So
/// the cluster is `ceil(span / g)` attacks with its largest maximum hit and
/// highest hit chance, never more events than the rows summed for rows of one
/// kind. An NPC that records combat only on a hit (`combat_vars_on_hit`,
/// content `mortton_shades.rs2:239-246`) can let a second NPC attack after
/// misses; its hits are still `g` ticks apart (a hit sets the variables), so a
/// cluster with such a member rolls damage on every one of those slots.
/// The caller takes the smaller of this bound and the all-summed bound.
pub(crate) fn build_single_way_timeline(
    schedule: TimelineInputs<'_>,
    poison: PoisonCharge,
    out: &mut EventBuf,
) -> Result<TimelineSummary, UnknownWhy> {
    build(schedule, poison, out)
}

/// The rows of one overlapping run that may attack in single-way combat.
#[derive(Clone, Copy)]
struct Cluster {
    first: usize,
    last: usize,
    end: i32,
    rows: u32,
}

/// The collapsed schedule of rows proven single-way: the union of their
/// intervals and the strongest attack of any member.
#[derive(Clone, Copy)]
struct Merged {
    first: usize,
    start: i32,
    end: i32,
    amount: u8,
    rate: u8,
    hit: HitChance,
    on_hit_only: bool,
    rows: u32,
}

/// Whether every route tile a row's NPC can hit the player on is known to be
/// outside the multiway table. The NPC stands within its reach of the player.
fn row_is_single_way(
    path: RoutePath<'_>,
    row: &ZoneInterval,
    risks: &RiskTables,
    multiway: MultiwayView<'_>,
) -> Result<bool, UnknownWhy> {
    let kind = risks.npc(row.ident()).ok_or(UnknownWhy::MissingFacts)?;
    let margin = kind.reach.saturating_add(1);
    for index in row.a..=row.b {
        let tile = path.point(index).ok_or(UnknownWhy::Overflow)?.tile;
        if !multiway.single_way_around(tile, margin) {
            return Ok(false);
        }
    }
    Ok(true)
}

/// A row that may join a single-way cluster: a shared attack of an NPC without
/// `npc_forcemulti`, attacking at least every [`SINGLE_WAY_LULL`] ticks.
fn candidate(row: &ZoneInterval, risks: &RiskTables, event: EventKind, rate: u8) -> bool {
    event == EventKind::Attack
        && rate <= SINGLE_WAY_LULL
        && risks
            .npc(row.ident())
            .is_some_and(|kind| kind.shared_attack() && !kind.forcemulti())
}

/// Emit a merged run: one attacker at a time when it holds two rows or more,
/// else the row's own schedule.
fn emit_merged(
    schedule: &TimelineInputs<'_>,
    merged: &Merged,
    events: &mut TimelineEvents<'_>,
) -> Result<(), UnknownWhy> {
    if merged.rows < 2 {
        let row = &schedule.plan.intervals[merged.first];
        let (kind, hit, rate) = kind_attack(row, schedule.input, schedule.risks, schedule.tables)?;
        let start = schedule.path.point(row.a).ok_or(UnknownWhy::Overflow)?.tick;
        let end = schedule.path.interval_end_tick(row)?;
        return add_row_events(events, row, kind, hit, rate, start, end);
    }
    // At most one attack per interval over the union; a member that records
    // combat only on a hit makes every one of those slots a damage roll.
    let hit = if merged.on_hit_only {
        HitChance::ceil_from(1.0)
    } else {
        merged.hit
    };
    let mut tick = merged.start;
    while tick < merged.end {
        add_event(
            events.out,
            &mut events.summary,
            tick,
            EventKind::Attack,
            merged.amount,
            hit,
        )?;
        tick = tick
            .checked_add(i32::from(merged.rate))
            .ok_or(UnknownWhy::Overflow)?;
    }
    events.summary.single_way_rows = events
        .summary
        .single_way_rows
        .saturating_add(u16::try_from(merged.rows).unwrap_or(u16::MAX));
    Ok(())
}

/// Emit a finished cluster. Its rows that are known to attack in single-way
/// combat are merged by overlap again (a row on a multiway cell may have been
/// the link); the others keep their own schedules.
fn flush(
    schedule: &TimelineInputs<'_>,
    cluster: &Cluster,
    events: &mut TimelineEvents<'_>,
) -> Result<(), UnknownWhy> {
    let mut open: Option<Merged> = None;
    for index in cluster.first..=cluster.last {
        let row = &schedule.plan.intervals[index];
        if row.hazard() {
            continue;
        }
        let (event, hit, rate) = kind_attack(row, schedule.input, schedule.risks, schedule.tables)?;
        if !candidate(row, schedule.risks, event, rate) {
            continue;
        }
        let start = schedule.path.point(row.a).ok_or(UnknownWhy::Overflow)?.tick;
        let end = schedule.path.interval_end_tick(row)?;
        if cluster.rows >= 2
            && !row_is_single_way(schedule.path, row, schedule.risks, schedule.multiway)?
        {
            add_row_events(events, row, event, hit, rate, start, end)?;
            continue;
        }
        let on_hit_only = schedule
            .risks
            .npc(row.ident())
            .is_some_and(|fact| fact.on_hit_only());
        match open.as_mut() {
            Some(merged) if start < merged.end => {
                merged.end = merged.end.max(end);
                merged.amount = merged.amount.max(row.max_hit);
                merged.rate = merged.rate.min(rate);
                if hit.get() > merged.hit.get() {
                    merged.hit = hit;
                }
                merged.on_hit_only |= on_hit_only;
                merged.rows += 1;
            }
            _ => {
                if let Some(done) = open.take() {
                    emit_merged(schedule, &done, events)?;
                }
                open = Some(Merged {
                    first: index,
                    start,
                    end,
                    amount: row.max_hit,
                    rate,
                    hit,
                    on_hit_only,
                    rows: 1,
                });
            }
        }
    }
    if let Some(done) = open {
        emit_merged(schedule, &done, events)?;
    }
    Ok(())
}

fn build(
    schedule: TimelineInputs<'_>,
    poison: PoisonCharge,
    out: &mut EventBuf,
) -> Result<TimelineSummary, UnknownWhy> {
    out.clear();
    let mut events = TimelineEvents {
        out,
        summary: TimelineSummary {
            attacks: 0,
            fixed: 0,
            ticks: 0,
            single_way_rows: 0,
        },
    };
    let mut last_tick = schedule.path.ticks();

    let mut open: Option<Cluster> = None;
    for (index, row) in schedule.plan.intervals.iter().enumerate() {
        let start = schedule.path.point(row.a).ok_or(UnknownWhy::Overflow)?.tick;
        let end = schedule.path.interval_end_tick(row)?;
        last_tick = last_tick.max(end);
        if row.hazard() {
            add_event(
                events.out,
                &mut events.summary,
                start,
                EventKind::Certain,
                row.max_hit,
                HitChance::ceil_from(0.0),
            )?;
            continue;
        }
        let (kind, hit, rate) = kind_attack(row, schedule.input, schedule.risks, schedule.tables)?;
        if !schedule.multiway.known() || !candidate(row, schedule.risks, kind, rate) {
            add_row_events(&mut events, row, kind, hit, rate, start, end)?;
            continue;
        }
        match open.as_mut() {
            Some(cluster) if start < cluster.end => {
                cluster.end = cluster.end.max(end);
                cluster.last = index;
                cluster.rows += 1;
            }
            _ => {
                if let Some(done) = open.take() {
                    flush(&schedule, &done, &mut events)?;
                }
                open = Some(Cluster {
                    first: index,
                    last: index,
                    end,
                    rows: 1,
                });
            }
        }
    }
    if let Some(done) = open {
        flush(&schedule, &done, &mut events)?;
    }

    for live in schedule.input.live_iter() {
        let kind = schedule
            .risks
            .npc(live.ident)
            .ok_or(UnknownWhy::AlreadyEngaged)?;
        if kind.unknown_for().is_some() || live.rate == 0 {
            return Err(UnknownWhy::AlreadyEngaged);
        }
        let duration = attack_duration(live, schedule.risks)?;
        last_tick = last_tick.max(duration);
        let (event_kind, hit) = if kind.shared_attack() {
            (
                EventKind::Attack,
                npc_hit(
                    live.ident,
                    kind.attack_roll,
                    schedule.input,
                    schedule.tables,
                )?,
            )
        } else {
            (EventKind::Certain, HitChance::ceil_from(0.0))
        };
        let mut tick = 0;
        while tick < duration {
            add_event(
                events.out,
                &mut events.summary,
                tick,
                event_kind,
                live.max_hit,
                hit,
            )?;
            tick = tick
                .checked_add(i32::from(live.rate))
                .ok_or(UnknownWhy::Overflow)?;
        }
        if let Some(due) = live.due() {
            let due = i32::from(due.wrapping_sub(schedule.input.tick));
            last_tick = last_tick.max(due.saturating_add(1));
            add_event(
                events.out,
                &mut events.summary,
                due,
                event_kind,
                live.max_hit,
                hit,
            )?;
        }
    }

    let last_tick = u16::try_from(last_tick).map_err(|_| UnknownWhy::Overflow)?;
    events.summary.ticks = last_tick;
    if schedule.input.map_members() {
        for live in schedule.input.live_iter() {
            let kind = schedule
                .risks
                .npc(live.ident)
                .ok_or(UnknownWhy::AlreadyEngaged)?;
            extend_poison(
                events.out,
                &mut events.summary,
                kind.poison,
                0,
                i32::from(last_tick),
            )?;
        }
    }
    for row in &schedule.plan.intervals {
        if row.hazard() || !schedule.input.map_members() {
            continue;
        }
        let kind = schedule
            .risks
            .npc(row.ident())
            .ok_or(UnknownWhy::MissingFacts)?;
        if kind.poison == 0 {
            continue;
        }
        let first = schedule.path.point(row.a).ok_or(UnknownWhy::Overflow)?.tick;
        extend_poison(
            events.out,
            &mut events.summary,
            kind.poison,
            first,
            i32::from(last_tick),
        )?;
    }

    match poison {
        PoisonCharge::None => {}
        PoisonCharge::Known { per_tick, first } => extend_poison(
            events.out,
            &mut events.summary,
            per_tick,
            i32::from(first),
            i32::from(last_tick),
        )?,
        PoisonCharge::Unknown { q } => {
            extend_poison(events.out, &mut events.summary, q, 0, i32::from(last_tick))?
        }
    }

    Ok(events.summary)
}

pub(crate) fn known_poison_charge(
    poison: PoisonState,
    tick: u16,
    q_unknown: u8,
) -> Result<PoisonCharge, UnknownWhy> {
    match poison {
        PoisonState::Unknown { .. } | PoisonState::Clear | PoisonState::Immune { .. } => {
            Ok(PoisonCharge::None)
        }
        PoisonState::Onset { since, .. } => {
            if q_unknown == u8::MAX {
                return Err(UnknownWhy::Poison);
            }
            Ok(PoisonCharge::Known {
                per_tick: q_unknown,
                first: relative_first_tick(tick, since),
            })
        }
        PoisonState::Poisoned {
            per_tick,
            last_mark,
            hit_since_mark,
        } => {
            let per_tick = if hit_since_mark {
                if q_unknown == u8::MAX {
                    return Err(UnknownWhy::Poison);
                }
                per_tick.max(q_unknown)
            } else {
                per_tick
            };
            Ok(PoisonCharge::Known {
                per_tick,
                first: relative_first_tick(tick, last_mark),
            })
        }
    }
}
