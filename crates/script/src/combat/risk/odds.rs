//! No-action damage probabilities. This core does not change route verdicts.
//!
//! Successive engine rolls are treated as independent. An attack misses on a
//! tied accuracy roll and otherwise draws uniform inclusive damage `0..=amount`.
//! Certain and poison events deal their full amount, without an accuracy roll.
use super::consts::MAX_EVENTS;
use super::UnknownWhy;

/// `P(U[0, attack_roll] > U[0, defence_roll])`. Inclusive ties miss.
/// Content: `npc_combat_melee.rs2:18-24`, shared `randominc` roll comparison.
pub fn hit_chance(attack_roll: u32, defence_roll: u32) -> f64 {
    let attack = f64::from(attack_roll);
    let defence = f64::from(defence_roll);
    if attack_roll > defence_roll {
        1.0 - (defence + 2.0) / (2.0 * (attack + 1.0))
    } else {
        attack / (2.0 * (defence + 1.0))
    }
}

/// A compact hit chance rounded toward positive infinity, never toward safety.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HitChance(f32);
impl HitChance {
    /// Round upward from the closed form. Solves reject non-probabilities.
    pub fn ceil_from(h: f64) -> Self {
        let nearest = h as f32;
        Self(if f64::from(nearest) < h {
            nearest.next_up()
        } else {
            nearest
        })
    }
    /// The stored upward-rounded probability, widened without loss.
    pub fn get(self) -> f64 {
        f64::from(self.0)
    }
}

/// The distribution applied by an event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventKind {
    /// Accuracy roll followed by inclusive uniform damage.
    Attack,
    /// Point mass at the maximum, including forced-maximum NPC attacks.
    Certain,
    /// Point mass at the charged poison pulse.
    Poison,
}

/// One scheduled event. Ticks have no role in the no-action total-damage bound.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Event {
    pub tick: u16,
    pub kind: EventKind,
    pub amount: u8,
    pub hit: HitChance,
}
const _: () = assert!(std::mem::size_of::<Event>() == 8);

const EMPTY: Event = Event {
    tick: 0,
    kind: EventKind::Certain,
    amount: 0,
    hit: HitChance(0.0),
};

/// Thread-owned event list, never held across a cooperative checkpoint.
/// One allocation on creation, none on clear or push. Never retained per bot.
pub struct EventBuf {
    events: Box<[Event; MAX_EVENTS]>,
    len: u16,
}
impl EventBuf {
    pub fn new() -> Self {
        Self {
            events: Box::new([EMPTY; MAX_EVENTS]),
            len: 0,
        }
    }
    pub fn clear(&mut self) {
        self.len = 0;
    }
    /// Refuse the first event above the cap without changing the stored slice.
    pub fn push(&mut self, event: Event) -> Result<(), UnknownWhy> {
        let index = usize::from(self.len);
        if index == MAX_EVENTS {
            return Err(UnknownWhy::Overflow);
        }
        self.events[index] = event;
        self.len += 1;
        Ok(())
    }
    pub fn events(&self) -> &[Event] {
        &self.events[..usize::from(self.len)]
    }
}
impl Default for EventBuf {
    fn default() -> Self {
        Self::new()
    }
}

/// Tail probability and its executed probability-cell multiply-add count.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DeathBound {
    pub bound: f64,
    pub visits: u32,
}
impl DeathBound {
    /// Add the derived rounding guard, not an empirical tolerance.
    pub fn admits(self, limit: f64) -> bool {
        self.bound + f64::from(self.visits) * f64::EPSILON <= limit
    }
}

const CELLS: usize = 128;
const _: () = assert!(std::mem::size_of::<[[f64; CELLS]; 2]>() == 2048);
const _: () = assert!(MAX_EVENTS <= u16::MAX as usize);
const _: () = assert!(MAX_EVENTS * CELLS * 256 <= u32::MAX as usize);

fn validate(events: &[Event], hp: u8) -> Result<(), UnknownWhy> {
    if events.len() > MAX_EVENTS || usize::from(hp) >= CELLS {
        return Err(UnknownWhy::Overflow);
    }
    if hp == 0 {
        return Err(UnknownWhy::MissingFacts);
    }
    for event in events {
        if event.kind == EventKind::Attack && !(0.0..=1.0).contains(&event.hit.get()) {
            return Err(UnknownWhy::MissingFacts);
        }
    }
    Ok(())
}

/// Fill a capped total-damage distribution. The last cell absorbs lethal mass.
/// Canonical `(amount, hit-bits)` order makes the arithmetic bitwise independent
/// of input order. Each distinct distribution scans the borrowed events once,
/// counting duplicates without copying or sorting a second event buffer.
/// For n events and d distinct attack distributions this uses at most n(d+1)
/// event inspections, in addition to the probability-cell visits. Fixed damage
/// is summed exactly first. There are only the two probability rows on stack.
fn distribution<'a>(
    events: &[Event],
    hp: u8,
    rows: &'a mut [[f64; CELLS]; 2],
) -> Result<(&'a [f64; CELLS], u32), UnknownWhy> {
    validate(events, hp)?;
    let cap = usize::from(hp);
    let fixed: usize = events
        .iter()
        .filter(|event| event.kind != EventKind::Attack)
        .map(|event| usize::from(event.amount))
        .sum();
    let [first, second] = rows;
    let mut current = first;
    let mut next = second;
    current[fixed.min(cap)] = 1.0;
    let mut visits = 0;
    let mut previous = None;
    loop {
        let mut selected = None;
        let mut count = 0;
        for event in events {
            if event.kind != EventKind::Attack || event.amount == 0 || event.hit.get() == 0.0 {
                continue;
            }
            let key = (event.amount, event.hit.0.to_bits());
            if previous.is_some_and(|prior| key <= prior) {
                continue;
            }
            match selected {
                Some(best) if key > best => {}
                Some(best) if key == best => count += 1,
                _ => {
                    selected = Some(key);
                    count = 1;
                }
            }
        }
        let Some((amount, hit_bits)) = selected else {
            break;
        };
        let hit = f64::from(f32::from_bits(hit_bits));
        let positive = hit / (f64::from(amount) + 1.0);
        let zero = 1.0 - hit + positive;
        for _ in 0..count {
            next.fill(0.0);
            next[cap] = current[cap];
            for (damage_so_far, &mass) in current[..cap].iter().enumerate() {
                if mass == 0.0 {
                    continue;
                }
                for damage in 0..=usize::from(amount) {
                    let probability = if damage == 0 { zero } else { positive };
                    let target = (damage_so_far + damage).min(cap);
                    next[target] = mass.mul_add(probability, next[target]);
                    visits += 1;
                }
            }
            std::mem::swap(&mut current, &mut next);
        }
        previous = selected;
    }
    Ok((current, visits))
}

/// `P(total damage >= hp0)`, without food, prayer, regeneration or escape.
/// No allocation. Reject zero HP as missing facts, HP above 127 or more than
/// 2048 events as overflow. The event order does not change the result's bits.
pub fn death_bound(events: &[Event], hp0: u8) -> Result<DeathBound, UnknownWhy> {
    let mut rows = [[0.0; CELLS]; 2];
    let (mass, visits) = distribution(events, hp0, &mut rows)?;
    Ok(DeathBound {
        bound: mass[usize::from(hp0)].min(1.0),
        visits,
    })
}

/// Smallest admitted HP in `1..=hp_max`, or zero when HP 1 is already safe.
/// Return `hp_max + 1` when none is safe. Compute all tails in one convolution.
/// This is a pure part-2 primitive. It has no route-verdict role in part 1.
pub fn escape_floor(corridor: &[Event], hp_max: u8, limit: f64) -> Result<u8, UnknownWhy> {
    let mut rows = [[0.0; CELLS]; 2];
    let (mass, mut visits) = distribution(corridor, hp_max, &mut rows)?;
    let mut tail = mass[usize::from(hp_max)];
    let mut floor = hp_max + 1;
    for hp in (1..=hp_max).rev() {
        if hp != hp_max {
            tail = mass[usize::from(hp)].mul_add(1.0, tail);
            visits += 1;
        }
        if (DeathBound {
            bound: tail.min(1.0),
            visits,
        })
        .admits(limit)
        {
            floor = hp;
        }
    }
    Ok(if floor == 1 { 0 } else { floor })
}

#[cfg(test)]
#[path = "odds_tests.rs"]
mod tests;
