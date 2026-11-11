//! Food-aware damage probability for one ordinary food kind.
//!
//! Food timing follows the selected content contract: `FoodFact` stores the
//! consume, skill and optional message delays (`tables.rs:192-199`); the caller
//! admits only ordinary food with `eat_delay_arg=Some(2)` and absent or
//! nonpositive `message_delay`. The engine next-food guard accepts the next
//! bite at earliest `T+3` (`consume.rs2:122-127`; `schedule.rs:124-128`). The
//! `skill_delay_arg` extends the known action-cycle deadline
//! (`schedule.rs:148-151`), but does not change the food clock. The heal is
//! deliberately credited only at `T+2`, after damage at that tick. Any
//! message-delay input lock remains caller-supplied through `GuardTick`
//! (`consume.rs2:114-134`; `schedule.rs:153-161`).
//!
//! The guard's bite rule is the shared one, `policy::eats_food` (the same
//! `policy::bite_tier` the walk guard, the combat machine, the survival step
//! and the route replay use): critical at or below `burst + 1` plus the
//! poison fire, where any bite is taken; caution up to `2 * danger + 1` plus
//! the fire, where only a bite that does not overheal is taken; and the
//! known-zero danger gate. This model has no prayer, regeneration or escape
//! credit. Damage events at a tick are applied in
//! canonical order before a due heal and before the guard can admit food.

use super::consts::MAX_EVENTS;
use super::odds::{death_bound, DeathBound, Event, EventKind, HitChance};
use super::UnknownWhy;
use crate::combat::policy;
use std::cell::RefCell;

const HP_STATES: usize = 128;
const FOOD_STATES: usize = 29;
const PHASE_STATES: usize = 4;
const STATE_CELLS: usize = HP_STATES * FOOD_STATES * PHASE_STATES;
const MAX_FOOD: u8 = 28;
const PHASE_HEAL_DUE: usize = 1;
const PHASE_COOLDOWN_3: usize = 3;
/// Maximum boxed backing bytes allocated once per worker thread, excluding the
/// fixed-size wrapper that lives inline in thread-local storage.
pub const SCRATCH_HEAP_BYTES: usize =
    2 * STATE_CELLS * std::mem::size_of::<f64>() + MAX_EVENTS * std::mem::size_of::<Event>();
const _: () = assert!(std::mem::size_of::<[f64; STATE_CELLS]>() == 118_784);
const _: () = assert!(std::mem::size_of::<[Event; MAX_EVENTS]>() == 16_384);
const _: () = assert!(SCRATCH_HEAP_BYTES == 253_952);

/// One ordinary food kind available to the calculation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Food {
    /// Selected item id; the caller guarantees all bites have these facts.
    pub item: i32,
    /// Hitpoints restored by one bite, applied after the conservative delay.
    pub heal: u8,
    /// Available carried bites of this same food kind.
    pub count: u8,
}

/// Guard observation used at one simulation tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GuardTick {
    /// Charged incoming danger, or `None` for the selector's conservative line.
    pub danger: Option<i32>,
    /// The most damage that can land before a bite takes effect (each
    /// attacker's maximum hit once per attack it can make in the bite delay,
    /// `policy::hits_before_bite`); `None` when `danger` is `None`. It sets
    /// the critical floor.
    pub burst: Option<i32>,
    /// Poison damage charged into the selector's bite threshold.
    pub poison_fire: i32,
    /// Whether the guard cannot admit any action on this tick.
    pub input_locked: bool,
}

/// Bounded per-thread scratch. The boxed buffers allocate once on first use and
/// are reused on every later evaluation; none is retained per bot.
struct Scratch {
    current: Box<[f64]>,
    next: Box<[f64]>,
    ordered: Box<[Event]>,
}

const _: () = assert!(std::mem::size_of::<Scratch>() == 3 * std::mem::size_of::<Box<[f64]>>());
impl Scratch {
    fn new() -> Self {
        Self {
            current: vec![0.0; STATE_CELLS].into_boxed_slice(),
            next: vec![0.0; STATE_CELLS].into_boxed_slice(),
            ordered: vec![empty_event(); MAX_EVENTS].into_boxed_slice(),
        }
    }
}

fn empty_event() -> Event {
    Event {
        tick: 0,
        kind: EventKind::Certain,
        amount: 0,
        hit: HitChance::ceil_from(0.0),
    }
}

thread_local! {
    static SCRATCH: RefCell<Scratch> = RefCell::new(Scratch::new());
}

#[inline]
const fn state_index(hp: usize, food: usize, phase: usize) -> usize {
    (hp * FOOD_STATES + food) * PHASE_STATES + phase
}

fn add_visits(visits: &mut u32, amount: u32) -> Result<(), UnknownWhy> {
    *visits = visits.checked_add(amount).ok_or(UnknownWhy::Overflow)?;
    Ok(())
}

#[inline]
fn add_mass(
    output: &mut [f64],
    index: usize,
    mass: f64,
    probability: f64,
    visits: &mut u32,
) -> Result<(), UnknownWhy> {
    output[index] = mass.mul_add(probability, output[index]);
    add_visits(visits, 1)
}

fn event_key(event: &Event) -> (u16, u8, u8, u64) {
    let kind = match event.kind {
        EventKind::Attack => 0,
        EventKind::Certain => 1,
        EventKind::Poison => 2,
    };
    (event.tick, kind, event.amount, event.hit.get().to_bits())
}

fn validate_bounds(events: &[Event], hp: u8, hp_max: u8, food: Food) -> Result<(), UnknownWhy> {
    if events.len() > MAX_EVENTS || hp > 127 || hp_max > 127 || food.count > MAX_FOOD {
        return Err(UnknownWhy::Overflow);
    }
    if hp == 0 || hp_max == 0 || hp > hp_max {
        return Err(UnknownWhy::MissingFacts);
    }
    Ok(())
}

fn validate_attacks(events: &[Event]) -> Result<(), UnknownWhy> {
    for event in events {
        if event.kind == EventKind::Attack && !(0.0..=1.0).contains(&event.hit.get()) {
            return Err(UnknownWhy::MissingFacts);
        }
    }
    Ok(())
}

fn update_ready_min(minimum: &mut u8, hp: usize, food: usize, phase: usize, mass: f64, hp_max: u8) {
    if mass > 0.0 && hp > 0 && hp < usize::from(hp_max) && food > 0 && phase == 0 {
        let hp = hp as u8;
        if *minimum == 0 || hp < *minimum {
            *minimum = hp;
        }
    }
}

/// Apply one event to the state distribution. The event is applied after every
/// earlier event at its tick; canonical sorting is performed by the caller.
fn apply_event(
    current: &[f64],
    next: &mut [f64],
    event: Event,
    hp_max: u8,
    max_food: u8,
    visits: &mut u32,
) -> Result<u8, UnknownWhy> {
    let hit = if event.kind == EventKind::Attack {
        event.hit.get()
    } else {
        1.0
    };
    if event.amount == 0 || hit == 0.0 {
        return Ok(0);
    }

    next.fill(0.0);
    let hp_cap = usize::from(hp_max);
    let food_cap = usize::from(max_food);
    let amount = usize::from(event.amount);
    let (hit_zero, hit_positive) = if event.kind == EventKind::Attack {
        let positive = hit / (f64::from(event.amount) + 1.0);
        let zero = 1.0 - hit + positive;
        // One division, one subtraction, one addition in the event kernel.
        add_visits(visits, 3)?;
        (zero, positive)
    } else {
        (1.0, 0.0)
    };
    let mut ready_min = 0;
    for hp in 0..=hp_cap {
        for food in 0..=food_cap {
            for phase in 0..PHASE_STATES {
                let mass = current[state_index(hp, food, phase)];
                if mass == 0.0 {
                    continue;
                }
                if hp == 0 || event.kind != EventKind::Attack {
                    let target_hp = if hp == 0 {
                        0
                    } else {
                        hp.saturating_sub(amount)
                    };
                    let target = state_index(target_hp, food, phase);
                    add_mass(next, target, mass, 1.0, visits)?;
                    update_ready_min(&mut ready_min, target_hp, food, phase, next[target], hp_max);
                    continue;
                }
                for damage in 0..=amount {
                    let probability = if damage == 0 { hit_zero } else { hit_positive };
                    if probability == 0.0 {
                        continue;
                    }
                    let target_hp = hp.saturating_sub(damage);
                    let target = state_index(target_hp, food, phase);
                    add_mass(next, target, mass, probability, visits)?;
                    update_ready_min(&mut ready_min, target_hp, food, phase, next[target], hp_max);
                }
            }
        }
    }
    Ok(ready_min)
}

/// Decrement the 3-tick ordinary-food clock. A bite at T enters phase 3;
/// damage at T+2 is applied while phase 1 is still pending, then its heal lands.
fn advance_phase(
    current: &[f64],
    next: &mut [f64],
    hp_max: u8,
    max_food: u8,
    visits: &mut u32,
) -> Result<(u8, u8), UnknownWhy> {
    next.fill(0.0);
    let mut ready_min = 0;
    let mut phase_mask = 0;
    for hp in 0..=usize::from(hp_max) {
        for food in 0..=usize::from(max_food) {
            for phase in 0..PHASE_STATES {
                let mass = current[state_index(hp, food, phase)];
                if mass == 0.0 {
                    continue;
                }
                let next_phase = phase.saturating_sub(1);
                let target = state_index(hp, food, next_phase);
                add_mass(next, target, mass, 1.0, visits)?;
                if next_phase != 0 {
                    phase_mask |= 1 << next_phase;
                }
                update_ready_min(&mut ready_min, hp, food, next_phase, next[target], hp_max);
            }
        }
    }
    Ok((ready_min, phase_mask))
}

/// Deliver heals due in phase 1 after this tick's damage. Descending HP order
/// permits the capped monotone mapping to merge in place without another array.
fn deliver_heal(
    current: &mut [f64],
    hp_max: u8,
    max_food: u8,
    heal: u8,
    visits: &mut u32,
) -> Result<(), UnknownWhy> {
    if heal == 0 {
        return Ok(());
    }
    let hp_cap = usize::from(hp_max);
    for hp in (1..=hp_cap).rev() {
        let target_hp = (hp + usize::from(heal)).min(hp_cap);
        if target_hp == hp {
            continue;
        }
        for food in 0..=usize::from(max_food) {
            let source = state_index(hp, food, PHASE_HEAL_DUE);
            let mass = current[source];
            if mass == 0.0 {
                continue;
            }
            current[source] = 0.0;
            let target = state_index(target_hp, food, PHASE_HEAL_DUE);
            current[target] = mass.mul_add(1.0, current[target]);
            add_visits(visits, 1)?;
        }
    }
    Ok(())
}

fn admit_food(
    current: &[f64],
    next: &mut [f64],
    hp_max: u8,
    max_food: u8,
    bites_at: impl Fn(i32) -> bool,
    visits: &mut u32,
) -> Result<(u8, bool), UnknownWhy> {
    next.fill(0.0);
    let mut ready_min = 0;
    let mut ate = false;
    for hp in 0..=usize::from(hp_max) {
        for food in 0..=usize::from(max_food) {
            for phase in 0..PHASE_STATES {
                let mass = current[state_index(hp, food, phase)];
                if mass == 0.0 {
                    continue;
                }
                let should_eat = hp > 0
                    && hp < usize::from(hp_max)
                    && food > 0
                    && phase == 0
                    && i32::try_from(hp).is_ok_and(&bites_at);
                let (target_hp, target_food, target_phase) = if should_eat {
                    ate = true;
                    (hp, food - 1, PHASE_COOLDOWN_3)
                } else {
                    (hp, food, phase)
                };
                let target = state_index(target_hp, target_food, target_phase);
                add_mass(next, target, mass, 1.0, visits)?;
                update_ready_min(
                    &mut ready_min,
                    target_hp,
                    target_food,
                    target_phase,
                    next[target],
                    hp_max,
                );
            }
        }
    }
    Ok((ready_min, ate))
}

fn death_probability(current: &[f64], max_food: u8, visits: &mut u32) -> Result<f64, UnknownWhy> {
    let mut dead = 0.0;
    for food in 0..=usize::from(max_food) {
        for phase in 0..PHASE_STATES {
            let mass = current[state_index(0, food, phase)];
            if mass != 0.0 {
                dead = mass.mul_add(1.0, dead);
                add_visits(visits, 1)?;
            }
        }
    }
    Ok(dead.min(1.0))
}

/// Calculate the death probability when one kind of ordinary carried food can
/// be admitted by the supplied guard.
///
/// Each bite heals after two ticks and makes the next bite eligible three ticks
/// after admission. At every tick, scheduled damage precedes delayed healing,
/// which precedes a possible guard bite. `Some(0)` danger suppresses a bite
/// unless `poison_fire` is positive, matching the live guard's charged-poison
/// rule. `Food` must describe homogeneous ordinary food; callers must not add
/// bank food or mix incompatible food timings.
///
/// HP is restricted to `1..=127`, food count to `0..=28`, and the event cap is
/// `risk::consts::MAX_EVENTS`. Invalid or unbounded inputs fail closed. Zero
/// food uses the no-food `death_bound` implementation exactly.
pub fn guarded_bound(
    events: &[Event],
    hp: u8,
    hp_max: u8,
    food: Food,
    guard: impl Fn(u16) -> GuardTick,
) -> Result<DeathBound, UnknownWhy> {
    validate_bounds(events, hp, hp_max, food)?;
    if food.count == 0 || events.is_empty() {
        return death_bound(events, hp);
    }
    validate_attacks(events)?;

    SCRATCH.with(|scratch| {
        let mut borrowed = scratch.borrow_mut();
        let scratch = &mut *borrowed;
        scratch.current.fill(0.0);
        scratch.next.fill(0.0);
        for (slot, event) in events.iter().copied().enumerate() {
            scratch.ordered[slot] = event;
        }
        scratch.ordered[..events.len()].sort_unstable_by_key(event_key);

        let max_food = food.count;
        let initial = state_index(usize::from(hp), usize::from(max_food), 0);
        scratch.current[initial] = 1.0;
        let mut ready_min = if hp < hp_max { hp } else { 0 };
        let mut phase_mask = 0_u8;
        let mut visits = 0_u32;
        let final_tick = u32::from(scratch.ordered[events.len() - 1].tick);
        let mut event_index = 0;

        for tick in 0..=final_tick {
            if tick != 0 && phase_mask != 0 {
                let (new_ready_min, new_phase_mask) = advance_phase(
                    &scratch.current,
                    &mut scratch.next,
                    hp_max,
                    max_food,
                    &mut visits,
                )?;
                std::mem::swap(&mut scratch.current, &mut scratch.next);
                ready_min = new_ready_min;
                phase_mask = new_phase_mask;
            }

            while event_index < events.len() && u32::from(scratch.ordered[event_index].tick) == tick
            {
                let event = scratch.ordered[event_index];
                let old_min = ready_min;
                let new_min = apply_event(
                    &scratch.current,
                    &mut scratch.next,
                    event,
                    hp_max,
                    max_food,
                    &mut visits,
                )?;
                if event.amount != 0 && (event.kind != EventKind::Attack || event.hit.get() != 0.0)
                {
                    std::mem::swap(&mut scratch.current, &mut scratch.next);
                    ready_min = new_min;
                } else {
                    ready_min = old_min;
                }
                event_index += 1;
            }

            if phase_mask & (1 << PHASE_HEAL_DUE) != 0 {
                deliver_heal(
                    &mut scratch.current,
                    hp_max,
                    max_food,
                    food.heal,
                    &mut visits,
                )?;
            }

            let observation = guard(tick as u16);
            let can_eat = (observation.danger != Some(0) || observation.poison_fire > 0)
                && !observation.input_locked
                && ready_min != 0;
            if can_eat {
                let hp_cap = i32::from(hp_max);
                let heal = i32::from(food.heal);
                let line = policy::eat_line(
                    observation.danger,
                    observation.burst,
                    hp_cap,
                    observation.poison_fire,
                    0,
                );
                if i32::from(ready_min) <= line {
                    let (new_ready_min, ate) = admit_food(
                        &scratch.current,
                        &mut scratch.next,
                        hp_max,
                        max_food,
                        |hp| {
                            policy::eats_food(
                                hp,
                                hp_cap,
                                heal,
                                observation.danger,
                                observation.burst,
                                observation.poison_fire,
                                0,
                            )
                        },
                        &mut visits,
                    )?;
                    std::mem::swap(&mut scratch.current, &mut scratch.next);
                    ready_min = new_ready_min;
                    if ate {
                        phase_mask |= 1 << PHASE_COOLDOWN_3;
                    }
                }
            }
        }

        let bound = death_probability(&scratch.current, max_food, &mut visits)?;
        Ok(DeathBound { bound, visits })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn certain(tick: u16, amount: u8) -> Event {
        Event {
            tick,
            kind: EventKind::Certain,
            amount,
            hit: HitChance::ceil_from(0.0),
        }
    }

    fn attack(tick: u16, amount: u8, hit: f64) -> Event {
        Event {
            tick,
            kind: EventKind::Attack,
            amount,
            hit: HitChance::ceil_from(hit),
        }
    }

    const fn food(heal: u8, count: u8) -> Food {
        Food {
            item: 100,
            heal,
            count,
        }
    }

    fn ordinary_guard(_: u16) -> GuardTick {
        GuardTick {
            danger: Some(1),
            burst: Some(1),
            poison_fire: 0,
            input_locked: false,
        }
    }

    fn locked_guard(_: u16) -> GuardTick {
        GuardTick {
            danger: Some(1),
            burst: Some(1),
            poison_fire: 0,
            input_locked: true,
        }
    }

    #[test]
    fn zero_food_is_exactly_the_no_food_bound() {
        let events = [attack(3, 4, 0.37), certain(0, 2), attack(3, 2, 0.6)];
        let expected = death_bound(&events, 9).unwrap();
        let actual = guarded_bound(&events, 9, 12, food(8, 0), ordinary_guard).unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn food_rescues_a_later_nonlethal_sequence() {
        let events = [certain(0, 2), certain(1, 2), certain(3, 5)];
        let with_food = guarded_bound(&events, 5, 10, food(5, 1), ordinary_guard).unwrap();
        let without_food = death_bound(&events, 5).unwrap();
        assert_eq!(with_food.bound, 0.0);
        assert_eq!(without_food.bound, 1.0);
    }

    #[test]
    fn lethal_first_volley_cannot_be_rescued() {
        let events = [certain(0, 5), certain(1, 1)];
        let actual = guarded_bound(&events, 5, 10, food(10, 2), ordinary_guard).unwrap();
        assert_eq!(actual.bound, 1.0);
    }

    #[test]
    fn heal_arrival_and_three_tick_cooldown_are_conservative() {
        // The first heal lands at tick 2, after that tick's damage. The second
        // bite is not admissible until tick 3; allowing it at tick 2 would
        // deliver an extra heal before the final volley.
        let events = [certain(0, 5), certain(2, 2), certain(4, 1), certain(5, 3)];
        let actual = guarded_bound(&events, 8, 8, food(2, 2), ordinary_guard).unwrap();
        assert_eq!(actual.bound, 1.0);

        let before_heal = [certain(0, 3), certain(1, 2)];
        let actual = guarded_bound(&before_heal, 5, 10, food(5, 1), ordinary_guard).unwrap();
        assert_eq!(actual.bound, 1.0);
    }

    #[test]
    fn delayed_heal_is_capped_at_maximum_hp() {
        let events = [certain(3, 10)];
        let actual = guarded_bound(&events, 1, 10, food(u8::MAX, 1), ordinary_guard).unwrap();
        assert_eq!(actual.bound, 1.0);
    }

    #[test]
    fn input_lock_prevents_food_admission() {
        let events = [certain(0, 2), certain(3, 1)];
        let actual = guarded_bound(&events, 3, 10, food(2, 1), locked_guard).unwrap();
        assert_eq!(actual.bound, 1.0);
    }

    #[test]
    fn invalid_bounds_fail_closed() {
        assert_eq!(
            guarded_bound(&[], 0, 10, food(5, 1), ordinary_guard),
            Err(UnknownWhy::MissingFacts)
        );
        assert_eq!(
            guarded_bound(&[], 1, 0, food(5, 1), ordinary_guard),
            Err(UnknownWhy::MissingFacts)
        );
        assert_eq!(
            guarded_bound(&[], 5, 4, food(5, 1), ordinary_guard),
            Err(UnknownWhy::MissingFacts)
        );
        assert_eq!(
            guarded_bound(&[], 128, 128, food(5, 1), ordinary_guard),
            Err(UnknownWhy::Overflow)
        );
        assert_eq!(
            guarded_bound(&[], 5, 10, food(5, 29), ordinary_guard),
            Err(UnknownWhy::Overflow)
        );
        assert_eq!(
            guarded_bound(&[attack(0, 1, f64::NAN)], 5, 10, food(5, 1), ordinary_guard),
            Err(UnknownWhy::MissingFacts)
        );
    }

    #[test]
    fn charged_poison_opens_the_known_zero_danger_food_gate() {
        let events = [certain(3, 2)];
        let no_charge = |_: u16| GuardTick {
            danger: Some(0),
            burst: Some(0),
            poison_fire: 0,
            input_locked: false,
        };
        let charged = |_: u16| GuardTick {
            danger: Some(0),
            burst: Some(0),
            poison_fire: 1,
            input_locked: false,
        };
        let refused = guarded_bound(&events, 2, 10, food(3, 1), no_charge).unwrap();
        let admitted = guarded_bound(&events, 2, 10, food(3, 1), charged).unwrap();
        assert_eq!(refused.bound, 1.0);
        assert_eq!(admitted.bound, 0.0);
    }

    #[test]
    fn input_event_order_does_not_change_the_result() {
        let first = [attack(2, 4, 0.6), certain(0, 1), attack(2, 3, 0.4)];
        let second = [attack(2, 3, 0.4), attack(2, 4, 0.6), certain(0, 1)];
        let a = guarded_bound(&first, 7, 10, food(3, 2), ordinary_guard).unwrap();
        let b = guarded_bound(&second, 7, 10, food(3, 2), ordinary_guard).unwrap();
        assert_eq!(a.bound.to_bits(), b.bound.to_bits());
        assert_eq!(a.visits, b.visits);
    }

    #[test]
    fn reusable_scratch_has_a_bounded_byte_layout() {
        assert_eq!(SCRATCH_HEAP_BYTES, 253_952);
        assert_eq!(std::mem::size_of::<Food>(), 8);
        assert_eq!(
            std::mem::size_of::<Scratch>(),
            3 * std::mem::size_of::<Box<[f64]>>()
        );
        assert_eq!(std::mem::size_of::<GuardTick>(), 24);
    }
}
