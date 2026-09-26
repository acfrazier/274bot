//! Frozen `AttackClock` (`api/combat/eatTiming.ts:27-51`): the tick our
//! attack animation began, keyed on animation-id changes. The first observed
//! id other than `-1` and every later change to another non-idle id start a
//! new swing; returning to `-1` starts nothing (`:31-39`).
//!
//! `fightUpkeep` keeps one module-level clock (`fightUpkeep.ts:9-19`); each
//! `new AttackClock()` a script makes (GreenDragon) keeps its own, held here
//! by slot so the JS instance carries only the slot. Every frozen caller
//! feeds `reader.selfAnim()` (the local player's primary animation), so the
//! clock reads that id from the posted scene rather than from the argument.

use crate::observed;
use std::cell::{Cell, RefCell};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct AttackClock {
    last_anim: i32,
    started_tick: f64,
}

impl AttackClock {
    const fn new() -> Self {
        Self {
            last_anim: -1,
            started_tick: -1.0,
        }
    }

    /// Frozen `observe(anim, tick)` (`eatTiming.ts:31-39`).
    fn observe(&mut self, anim: i32, tick: f64) {
        if anim != self.last_anim {
            self.last_anim = anim;
            if anim != -1 {
                self.started_tick = tick;
            }
        }
    }

    /// Frozen `attackedThisTick(tick)` (`eatTiming.ts:42-44`).
    fn attacked_this_tick(&self, tick: f64) -> bool {
        self.started_tick == tick
    }
}

thread_local! {
    /// The `fightUpkeep` module clock, one per isolate thread (the frozen
    /// module lives as long as the script).
    static FIGHT: Cell<AttackClock> = const { Cell::new(AttackClock::new()) };
    /// One slot per `new AttackClock()` in this isolate thread. Instances are
    /// script fields, so slots are never reused; the table dies with the
    /// isolate thread.
    static CLOCKS: RefCell<Vec<AttackClock>> = const { RefCell::new(Vec::new()) };
}

/// The posted primary animation id of the local player and the last posted
/// tick (frozen `reader.selfAnim()`, `BotHost.tickCount`).
fn posted() -> (i32, f64) {
    observed::with(|scene| {
        let anim = scene.latest().self_anim().unwrap_or(-1);
        (anim, scene.tick().unwrap_or(0) as f64)
    })
}

/// Frozen `swingStartedThisTick()` (`fightUpkeep.ts:16-19`): observe, then
/// ask the module clock about this tick.
pub(crate) fn swing_started_this_tick() -> bool {
    let (anim, tick) = posted();
    FIGHT.with(|cell| {
        let mut clock = cell.get();
        clock.observe(anim, tick);
        cell.set(clock);
        clock.attacked_this_tick(tick)
    })
}

/// `new AttackClock()`: a fresh clock; returns its slot.
pub(crate) fn clock_new() -> usize {
    CLOCKS.with(|clocks| {
        let mut clocks = clocks.borrow_mut();
        clocks.push(AttackClock::new());
        clocks.len() - 1
    })
}

fn with_clock<R>(slot: usize, f: impl FnOnce(&mut AttackClock) -> R) -> Option<R> {
    CLOCKS.with(|clocks| clocks.borrow_mut().get_mut(slot).map(f))
}

/// `clock.observe(reader.selfAnim(), tick)`; `None` for an unknown slot.
pub(crate) fn clock_observe(slot: usize, tick: f64) -> Option<()> {
    let (anim, _) = posted();
    with_clock(slot, |clock| clock.observe(anim, tick))
}

/// `clock.attackedThisTick(tick)`.
pub(crate) fn clock_attacked(slot: usize, tick: f64) -> Option<bool> {
    with_clock(slot, |clock| clock.attacked_this_tick(tick))
}

/// `clock.reset()` (`eatTiming.ts:46-49`).
pub(crate) fn clock_reset(slot: usize) -> Option<()> {
    with_clock(slot, |clock| *clock = AttackClock::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Frozen `observe`: the first non-idle id starts a swing, a changed
    /// non-idle id starts another, the same id or a return to idle does not.
    #[test]
    fn a_swing_starts_on_each_new_non_idle_animation() {
        let mut clock = AttackClock::new();
        let mut starts = Vec::new();
        for (tick, anim) in [(1, 390), (2, 390), (3, 391), (4, -1), (5, -1), (6, 391)] {
            clock.observe(anim, f64::from(tick));
            starts.push(clock.attacked_this_tick(f64::from(tick)));
        }
        assert_eq!(starts, [true, false, true, false, false, true]);
    }
}
