//! The Guardian Fight-to-Flee line: the Hitpoints a fight must keep to afford
//! its walk out (design V11D 5.3). Pure arithmetic over content facts and one
//! assessed walk. The Combat core reads it; the adapter prices the inputs.
//!
//! The reserve counts the attacks that can land before the player is outside
//! the safety radius. The walk starts after the input lock `L` ends and takes
//! `W` ticks, plus one tick to hand the outcome to the next Guardian pass:
//! `T = 1 + L + W`. The actor launches `N = 1 + floor(T / r)` attacks inside
//! `T`, counting one due at once and one at the horizon, and one more hit can
//! already be queued (`J` settles it). The line adds one further hit, the one
//! that can cross it between two fresh decisions.

use api::condition::POISON_PERIOD;

/// Tick opportunities a queued event hit needs to land. Melee queues its
/// damage when it swings; the golem's projectile lands after
/// `floor((120 + 5) / 60) = 2` script ticks (`npc_combat_melee.rs2:47`,
/// `macro_event_rock_golem.rs2:48,65`). Both settle inside three
/// opportunities, which is shorter than the four-tick attack interval.
pub const SETTLE_TICKS: i32 = 3;

/// The inputs of one Fight-to-Flee line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EscapeCost {
    /// `D`: the greatest hit the event can land, unprotected.
    pub hit: i32,
    /// `r`: the fastest attack interval, at least one tick.
    pub interval: i32,
    /// `W`: the full walking-tick bound of the escape route.
    pub walk: i32,
    /// `L`: the input lock still to run plus the lock of one upkeep bite the
    /// escape can need before it starts.
    pub lock: i32,
    /// Damage of one poison fire, `0` while not poisoned.
    pub poison_fire: i32,
    /// Poison fires still to come, at most.
    pub poison_fires: i32,
    /// Poison damage still to come, at most.
    pub poison_total: i32,
}

impl EscapeCost {
    /// `T`: the ticks from the decision to the first step outside the radius.
    pub fn horizon(&self) -> i32 {
        1i32.saturating_add(self.lock.max(0))
            .saturating_add(self.walk.max(0))
    }

    /// The poison damage that can land inside `ticks`: every fire the span can
    /// hold, capped by the damage that remains. One fire can land on any
    /// tick, so a span holds `floor(ticks / period) + 1` of them.
    pub fn poison_loss(&self, ticks: i32) -> i32 {
        let span = i32::from(POISON_PERIOD);
        let in_span = ticks.max(0) / span + 1;
        self.poison_fire
            .max(0)
            .saturating_mul(self.poison_fires.max(0).min(in_span))
            .min(self.poison_total.max(0))
    }

    /// `N`: the attacks the actor can launch inside the horizon.
    pub fn attacks(&self) -> i32 {
        1i32.saturating_add(self.horizon() / self.interval.max(1))
    }

    /// The Hitpoints the walk out needs from the moment it starts:
    /// `(N + 1) * D + poison_loss(T + J) + 1`.
    pub fn reserve(&self) -> i32 {
        let hits = self
            .attacks()
            .saturating_add(1)
            .saturating_mul(self.hit.max(0));
        hits.saturating_add(self.poison_loss(self.horizon().saturating_add(SETTLE_TICKS)))
            .saturating_add(1)
    }

    /// The line: with no unreserved food, a fight at or below this many
    /// Hitpoints hands over to Flee before another attack.
    pub fn flee_hp(&self) -> i32 {
        self.reserve()
            .saturating_add(self.hit.max(0))
            .saturating_add(self.poison_fire.max(0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cost(hit: i32, interval: i32, walk: i32, lock: i32) -> EscapeCost {
        EscapeCost {
            hit,
            interval,
            walk,
            lock,
            poison_fire: 0,
            poison_fires: 0,
            poison_total: 0,
        }
    }

    #[test]
    fn the_design_example_gives_seventy_one() {
        // A top-tier event (max hit 14, interval 4), an open seven-step walk
        // and no lock: T = 8, N = 3, reserve 4 * 14 + 1 = 57, line 57 + 14.
        let c = cost(14, 4, 7, 0);
        assert_eq!(c.horizon(), 8);
        assert_eq!(c.attacks(), 3);
        assert_eq!(c.reserve(), 57);
        assert_eq!(c.flee_hp(), 71);
    }

    #[test]
    fn the_conservative_eleven_tick_route_gives_eighty_five() {
        let c = cost(14, 4, 11, 0);
        assert_eq!(c.horizon(), 12);
        assert_eq!(c.attacks(), 4);
        assert_eq!(c.flee_hp(), 85);
    }

    #[test]
    fn a_lock_or_a_longer_detour_raises_the_line_by_the_same_formula() {
        let open = cost(14, 4, 7, 0).flee_hp();
        // Four more ticks add one launch.
        assert_eq!(cost(14, 4, 7, 4).flee_hp(), open + 14);
        assert_eq!(cost(14, 4, 11, 0).flee_hp(), open + 14);
        // Less than a full interval adds no launch.
        assert_eq!(cost(14, 4, 7, 2).flee_hp(), open);
    }

    #[test]
    fn a_lower_tier_pays_a_smaller_line() {
        // Tier 1: strength 9 gives max hit 2.
        assert_eq!(cost(2, 4, 7, 0).flee_hp(), 4 * 2 + 1 + 2);
    }

    #[test]
    fn the_line_is_above_the_emergency_and_eat_lines_of_the_content_example() {
        // Emergency `D + 1` = 15 and eat `2D + 1` = 29 for D = 14: a line above
        // the larger is above both.
        let line = cost(14, 4, 7, 0).flee_hp();
        assert!(line > 29, "the line {line} is above the eat line");
    }

    #[test]
    fn poison_adds_every_fire_the_span_can_hold_and_the_next_step() {
        let mut c = cost(14, 4, 7, 0);
        c.poison_fire = 2;
        c.poison_fires = 9;
        c.poison_total = 90;
        // T + J = 11 ticks hold one fire: 2 damage, plus the step allowance.
        assert_eq!(c.poison_loss(c.horizon() + SETTLE_TICKS), 2);
        assert_eq!(c.flee_hp(), 71 + 2 + 2);
        // A span of 31 ticks can hold two fires.
        assert_eq!(c.poison_loss(31), 4);
        // The remaining damage caps the loss.
        c.poison_total = 3;
        assert_eq!(c.poison_loss(61), 3);
    }

    #[test]
    fn arithmetic_saturates_instead_of_wrapping() {
        let c = cost(i32::MAX, 1, i32::MAX, i32::MAX);
        assert_eq!(c.flee_hp(), i32::MAX);
    }
}
