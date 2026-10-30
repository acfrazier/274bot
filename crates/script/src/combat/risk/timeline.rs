use super::consts::{LAG, MAX_EVENTS, POISON_PERIOD};
use super::odds::{hit_chance, Event, EventBuf, EventKind, HitChance};
use super::{RiskInput, RiskTables, RoutePath, RoutePlan, UnknownWhy, ZoneInterval};
use crate::combat::tables::CombatTables;
use api::condition::PoisonState;
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

fn npc_hit(
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
    out.clear();
    let mut summary = TimelineSummary {
        attacks: 0,
        fixed: 0,
        ticks: 0,
    };
    let mut last_tick = path.ticks();

    for row in &plan.intervals {
        let start = path.point(row.a).ok_or(UnknownWhy::Overflow)?.tick;
        let end = path.interval_end_tick(row)?;
        last_tick = last_tick.max(end);
        if row.hazard() {
            add_event(
                out,
                &mut summary,
                start,
                EventKind::Certain,
                row.max_hit,
                HitChance::ceil_from(0.0),
            )?;
        } else {
            let (kind, hit, rate) = kind_attack(row, input, risks, tables)?;
            let mut tick = start;
            while tick < end {
                add_event(out, &mut summary, tick, kind, row.max_hit, hit)?;
                tick = tick
                    .checked_add(i32::from(rate))
                    .ok_or(UnknownWhy::Overflow)?;
            }
        }
    }

    for live in input.live_iter() {
        let kind = risks.npc(live.ident).ok_or(UnknownWhy::AlreadyEngaged)?;
        if kind.unknown_for().is_some() || live.rate == 0 {
            return Err(UnknownWhy::AlreadyEngaged);
        }
        let duration = attack_duration(live, risks)?;
        last_tick = last_tick.max(duration);
        let (event_kind, hit) = if kind.shared_attack() {
            (
                EventKind::Attack,
                npc_hit(live.ident, kind.attack_roll, input, tables)?,
            )
        } else {
            (EventKind::Certain, HitChance::ceil_from(0.0))
        };
        let mut tick = 0;
        while tick < duration {
            add_event(out, &mut summary, tick, event_kind, live.max_hit, hit)?;
            tick = tick
                .checked_add(i32::from(live.rate))
                .ok_or(UnknownWhy::Overflow)?;
        }
        if let Some(due) = live.due() {
            let due = i32::from(due.wrapping_sub(input.tick));
            last_tick = last_tick.max(due.saturating_add(1));
            add_event(out, &mut summary, due, event_kind, live.max_hit, hit)?;
        }
    }

    let last_tick = u16::try_from(last_tick).map_err(|_| UnknownWhy::Overflow)?;
    summary.ticks = last_tick;
    if input.map_members() {
        for live in input.live_iter() {
            let kind = risks.npc(live.ident).ok_or(UnknownWhy::AlreadyEngaged)?;
            extend_poison(out, &mut summary, kind.poison, 0, i32::from(last_tick))?;
        }
    }
    for row in &plan.intervals {
        if row.hazard() || !input.map_members() {
            continue;
        }
        let kind = risks.npc(row.ident()).ok_or(UnknownWhy::MissingFacts)?;
        if kind.poison == 0 {
            continue;
        }
        let first = path.point(row.a).ok_or(UnknownWhy::Overflow)?.tick;
        extend_poison(out, &mut summary, kind.poison, first, i32::from(last_tick))?;
    }

    match poison {
        PoisonCharge::None => {}
        PoisonCharge::Known { per_tick, first } => extend_poison(
            out,
            &mut summary,
            per_tick,
            i32::from(first),
            i32::from(last_tick),
        )?,
        PoisonCharge::Unknown { q } => {
            extend_poison(out, &mut summary, q, 0, i32::from(last_tick))?
        }
    }

    Ok(summary)
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
