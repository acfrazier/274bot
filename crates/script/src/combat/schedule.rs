//! Server input, next-food, interaction and swing clocks are independent.
use super::tables::FoodFact;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum OpKind {
    Eat,
    Drink,
    Prayer,
    Wear,
    Retaliate,
    Attack,
    Style,
}
impl OpKind {
    pub const fn index(self) -> usize {
        self as usize
    }
}
#[derive(Clone, Copy)]
pub enum InputEffect<'a> {
    Standard,
    PrayerOn,
    ElementalShield,
    Food(&'a FoodFact),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Interaction {
    Installed,
    Cleared,
    #[default]
    Unknown,
}
#[derive(Debug, Clone, Copy, Default)]
pub struct Cycle {
    pub deadline: u16,
    pub known: bool,
}
/// All short windows are less than half the u16 range. Unlike comparing
/// raw tick numbers, this remains correct across the low-16-bit wrap.
pub fn reached(tick: u16, deadline: u16) -> bool {
    tick.wrapping_sub(deadline) < 0x8000
}
pub fn elapsed(tick: u16, since: u16) -> u16 {
    tick.wrapping_sub(since)
}
#[derive(Debug, Clone, Copy, Default)]
pub struct Schedule {
    earliest: [u16; 7],
    pub unsettled: [u8; 7],
    ready_mask: u8,
    pending_mask: u8,
    pub cycle: Cycle,
    pub input_lock: u16,
    lock_valid: bool,
    pub interaction: Interaction,
    pub restore_due: u16,
    pub restore_owed: bool,
    pub last_clear: u16,
    pub clear_valid: bool,
    pub last_attack: u16,
    pub attack_valid: bool,
    pub last_swing: u16,
    pub swing_valid: bool,
}
impl Schedule {
    pub fn ready(&self, kind: OpKind, tick: u16) -> bool {
        let bit = 1 << kind.index();
        self.ready_mask & bit == 0 || reached(tick, self.earliest[kind.index()])
    }
    /// A null source argument bypasses the engine's existing next-food guard.
    pub fn food_ready(&self, food: &FoodFact, tick: u16) -> bool {
        food.eat_delay_arg.is_none() || self.ready(OpKind::Eat, tick)
    }
    pub fn pending(&self, kind: OpKind) -> bool {
        self.pending_mask & (1 << kind.index()) != 0
    }
    pub fn settle(&mut self, kind: OpKind) {
        self.pending_mask &= !(1 << kind.index());
        self.unsettled[kind.index()] = 0;
    }
    pub fn timeout(&mut self, kind: OpKind) {
        self.pending_mask &= !(1 << kind.index());
        self.unsettled[kind.index()] = self.unsettled[kind.index()].saturating_add(1);
    }
    pub fn locked(&self, tick: u16) -> bool {
        self.lock_valid && !reached(tick, self.input_lock)
    }
    pub fn unlock_observed(&mut self, tick: u16) {
        if self.lock_valid && reached(tick, self.input_lock) {
            self.lock_valid = false;
        }
    }
    /// Apply only after successful host admission, never after a refused emit.
    pub fn admitted(
        &mut self,
        kind: OpKind,
        tick: u16,
        rate: u8,
        fight: bool,
        effect: InputEffect<'_>,
    ) {
        let index = kind.index();
        let delay = match kind {
            OpKind::Eat => match effect {
                InputEffect::Food(food) => food.eat_delay_arg.map(|arg| arg.wrapping_add(1) as u16),
                _ => unreachable!("food admission requires its immutable timing fact"),
            },
            OpKind::Drink | OpKind::Prayer => Some(3),
            OpKind::Wear | OpKind::Retaliate | OpKind::Style => Some(2),
            OpKind::Attack => Some(u16::from(rate.max(1)) + 1),
        };
        if kind != OpKind::Prayer || matches!(effect, InputEffect::PrayerOn) {
            if let Some(delay) = delay {
                self.earliest[index] = tick.wrapping_add(delay);
                self.ready_mask |= 1 << index;
            }
        }
        self.pending_mask |= 1 << index;
        match kind {
            OpKind::Eat => {
                let InputEffect::Food(food) = effect else {
                    unreachable!("food admission requires its immutable timing fact")
                };
                if self.cycle.known {
                    if let Some(arg) = food.skill_delay_arg {
                        self.cycle.deadline = self.cycle.deadline.wrapping_add(arg as u16);
                    }
                }
                let wait = if let Some(arg) = food.message_delay {
                    let delay = arg.wrapping_add(2) as u16;
                    self.input_lock = tick.wrapping_add(delay);
                    self.lock_valid = true;
                    delay
                } else {
                    1
                };
                self.clear(tick, wait, fight);
            }
            OpKind::Drink => {
                self.input_lock = tick.wrapping_add(3);
                self.lock_valid = true;
                self.clear(tick, 3, fight);
            }
            OpKind::Wear => {
                let elemental_shield = matches!(effect, InputEffect::ElementalShield);
                if elemental_shield {
                    self.input_lock = tick.wrapping_add(2);
                    self.lock_valid = true;
                }
                self.clear(tick, if elemental_shield { 2 } else { 1 }, fight);
            }
            OpKind::Prayer | OpKind::Retaliate | OpKind::Style => self.clear(tick, 1, fight),
            OpKind::Attack => {
                self.last_attack = tick;
                self.attack_valid = true;
                self.restore_owed = false;
                self.interaction = Interaction::Unknown;
            }
        }
    }
    pub fn clear(&mut self, tick: u16, wait: u16, fight: bool) {
        self.interaction = Interaction::Cleared;
        self.last_clear = tick;
        self.clear_valid = true;
        self.restore_owed = fight;
        self.restore_due = tick.wrapping_add(wait);
    }
    pub fn observe_swing(&mut self, tick: u16, rate: u8) {
        // A late observation of the swing cancelled by this tick's clearing
        // op cannot erase the food extension or reinstall the interaction.
        if self.clear_valid && (!reached(tick, self.last_clear) || tick == self.last_clear) {
            return;
        }
        self.cycle = Cycle {
            deadline: tick.wrapping_add(u16::from(rate)),
            known: true,
        };
        self.last_swing = tick;
        self.swing_valid = true;
        self.interaction = Interaction::Installed;
        self.settle(OpKind::Attack);
    }
    pub fn restore_ready(&self, tick: u16) -> bool {
        self.restore_owed
            && self.interaction == Interaction::Cleared
            && reached(tick, self.restore_due)
    }
    pub fn stale_ready(&self, tick: u16, rate: u8) -> bool {
        if !self.ready(OpKind::Attack, tick)
            || (self.cycle.known && !reached(tick, self.cycle.deadline))
        {
            return false;
        }
        if self.interaction == Interaction::Cleared {
            return false;
        }
        let since = if self.swing_valid
            && (!self.attack_valid || reached(self.last_swing, self.last_attack))
        {
            self.last_swing
        } else {
            self.last_attack
        };
        !self.attack_valid || elapsed(tick, since) > u16::from(rate)
    }
    pub fn terminal(&mut self) {
        self.restore_owed = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ordinary_food() -> FoodFact {
        FoodFact {
            item_id: 379,
            heal: 12,
            eat_delay_arg: Some(2),
            skill_delay_arg: Some(3),
            message_delay: None,
        }
    }

    #[test]
    fn dose_settlement_never_unlocks_input_and_food_clock_does_not_gate_doses() {
        let mut schedule = Schedule::default();
        schedule.admitted(
            OpKind::Eat,
            10,
            4,
            true,
            InputEffect::Food(&ordinary_food()),
        );
        assert!(!schedule.ready(OpKind::Eat, 11));
        assert!(schedule.ready(OpKind::Drink, 11));
        schedule.admitted(OpKind::Drink, 11, 4, true, InputEffect::Standard);
        schedule.settle(OpKind::Drink);
        assert!(schedule.locked(12));
        assert!(schedule.locked(13));
        assert!(!schedule.locked(14));
        assert!(!schedule.restore_ready(13));
        assert!(schedule.restore_ready(14));
    }

    #[test]
    fn food_extends_past_or_future_deadline_and_restoration_bypasses_attack_pacing() {
        let mut schedule = Schedule::default();
        schedule.admitted(OpKind::Attack, 9, 4, true, InputEffect::Standard);
        schedule.observe_swing(9, 4);
        schedule.admitted(
            OpKind::Eat,
            11,
            4,
            true,
            InputEffect::Food(&ordinary_food()),
        );
        assert_eq!(schedule.cycle.deadline, 16);
        assert!(!schedule.ready(OpKind::Attack, 12));
        assert!(schedule.restore_ready(12));
        schedule.admitted(OpKind::Attack, 12, 4, true, InputEffect::Standard);
        assert!(!schedule.stale_ready(15, 4));
        schedule.observe_swing(16, 4);
        assert_eq!(schedule.cycle.deadline, 20);
        schedule.cycle.deadline = 24;
        schedule.admitted(
            OpKind::Eat,
            32,
            4,
            true,
            InputEffect::Food(&ordinary_food()),
        );
        assert_eq!(schedule.cycle.deadline, 27);
        assert!(schedule.restore_ready(33));
    }

    #[test]
    fn clearing_runs_coalesce_and_prep_escape_cleanup_create_no_obligation() {
        let mut schedule = Schedule::default();
        schedule.admitted(OpKind::Prayer, 40, 4, true, InputEffect::PrayerOn);
        schedule.admitted(
            OpKind::Eat,
            41,
            4,
            true,
            InputEffect::Food(&ordinary_food()),
        );
        assert!(!schedule.restore_ready(41));
        assert!(schedule.restore_ready(42));
        schedule.admitted(OpKind::Attack, 42, 4, true, InputEffect::Standard);
        assert!(!schedule.restore_owed);
        for kind in [OpKind::Drink, OpKind::Retaliate, OpKind::Prayer] {
            schedule.admitted(kind, 50, 4, false, InputEffect::Standard);
            assert!(!schedule.restore_owed);
        }
        schedule.admitted(OpKind::Prayer, 60, 4, true, InputEffect::PrayerOn);
        schedule.terminal();
        assert!(!schedule.restore_ready(61));
    }

    #[test]
    fn stale_retry_uses_rate_and_clear_tick_evidence_cannot_install_interaction() {
        let mut schedule = Schedule::default();
        schedule.admitted(OpKind::Attack, 100, 7, true, InputEffect::Standard);
        assert!(!schedule.stale_ready(107, 7));
        assert!(schedule.stale_ready(108, 7));
        schedule.observe_swing(108, 7);
        schedule.admitted(
            OpKind::Eat,
            110,
            7,
            true,
            InputEffect::Food(&ordinary_food()),
        );
        schedule.observe_swing(110, 7);
        assert_eq!(schedule.interaction, Interaction::Cleared);
        assert_eq!(schedule.cycle.deadline, 118);
        assert!(schedule.restore_ready(111));
    }

    #[test]
    fn all_clocks_survive_tick_wrap_and_unadmitted_state_is_inert() {
        let mut schedule = Schedule::default();
        assert!(!schedule.locked(u16::MAX));
        assert!(!schedule.pending(OpKind::Drink));
        schedule.admitted(OpKind::Drink, u16::MAX - 1, 4, true, InputEffect::Standard);
        assert!(schedule.locked(u16::MAX));
        assert!(schedule.locked(0));
        assert!(!schedule.locked(1));
        assert!(schedule.restore_ready(1));
        assert!(!reached(u16::MAX, 1));
        assert!(reached(1, u16::MAX));
    }

    #[test]
    fn null_food_arguments_preserve_existing_clocks_and_message_delay_locks_input() {
        let ordinary = ordinary_food();
        let karambwan = FoodFact {
            item_id: 3144,
            heal: 18,
            eat_delay_arg: None,
            skill_delay_arg: None,
            message_delay: Some(2),
        };
        let mut schedule = Schedule::default();
        schedule.observe_swing(10, 4);
        schedule.admitted(OpKind::Eat, 11, 4, true, InputEffect::Food(&ordinary));
        assert_eq!(schedule.cycle.deadline, 17);
        assert!(!schedule.food_ready(&ordinary, 11));
        assert!(schedule.food_ready(&karambwan, 11));
        schedule.admitted(OpKind::Eat, 11, 4, true, InputEffect::Food(&karambwan));
        assert_eq!(schedule.cycle.deadline, 17);
        assert!(!schedule.food_ready(&ordinary, 13));
        assert!(schedule.food_ready(&ordinary, 14));
        schedule.settle(OpKind::Eat);
        for tick in 11..15 {
            assert!(schedule.locked(tick));
            assert!(!schedule.restore_ready(tick));
        }
        assert!(!schedule.locked(15));
        assert!(schedule.restore_ready(15));
    }

    #[test]
    fn zero_food_delay_and_zero_message_delay_are_not_null() {
        let food = FoodFact {
            item_id: 2323,
            heal: 5,
            eat_delay_arg: Some(0),
            skill_delay_arg: Some(1),
            message_delay: Some(0),
        };
        let mut schedule = Schedule::default();
        schedule.observe_swing(u16::MAX - 1, 4);
        schedule.admitted(OpKind::Eat, u16::MAX, 4, true, InputEffect::Food(&food));
        assert_eq!(schedule.cycle.deadline, 3);
        assert!(!schedule.food_ready(&food, u16::MAX));
        assert!(schedule.food_ready(&food, 0));
        assert!(schedule.locked(u16::MAX));
        assert!(schedule.locked(0));
        assert!(!schedule.locked(1));
        assert!(schedule.restore_ready(1));
    }
}

#[cfg(test)]
mod input_effect_tests {
    use super::*;

    #[test]
    fn off_sweep_is_unpaced_but_preserves_the_last_on_cooldown() {
        let mut schedule = Schedule::default();
        schedule.admitted(OpKind::Prayer, 10, 4, true, InputEffect::PrayerOn);
        schedule.admitted(OpKind::Prayer, 11, 4, false, InputEffect::Standard);
        schedule.settle(OpKind::Prayer);
        assert!(!schedule.ready(OpKind::Prayer, 12));
        schedule.admitted(OpKind::Prayer, 12, 4, false, InputEffect::Standard);
        schedule.settle(OpKind::Prayer);
        assert!(schedule.ready(OpKind::Prayer, 13));
        assert!(!schedule.restore_owed);
        assert!(!schedule.locked(13));
    }

    #[test]
    fn elemental_shield_observation_cannot_shorten_its_input_lock() {
        let mut schedule = Schedule::default();
        schedule.admitted(OpKind::Wear, 20, 4, true, InputEffect::ElementalShield);
        schedule.settle(OpKind::Wear);
        assert!(schedule.locked(21));
        assert!(!schedule.restore_ready(21));
        assert!(!schedule.locked(22));
        assert!(schedule.restore_ready(22));
    }
}
