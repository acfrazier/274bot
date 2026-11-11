//! The stale-HP rule for a bite and its one survival log line. The step, the
//! walk guard and the combat machine share both, so they cannot drift.

use api::hostlog::{Category, Level};

use crate::combat::tables::FoodFact;

/// Observed ticks, counted from the server tick that ran a bite, that a bite
/// may wait for its heal to show in the Hitpoints stat. It is the food's own
/// next-food delay: `consume.rs2:122-127` sets `%eat_delay` to
/// `map_clock + eat_delay_arg` and refuses a bite while
/// `%eat_delay >= map_clock`, so the next accepted bite is `eat_delay_arg + 1`
/// ticks later (`Schedule::admitted` uses the same number). A combo food has
/// no next-food clock; its `p_delay(message_delay)` plus the two ticks of
/// input lock after it (`Schedule::admitted`) is its delay, and a food with
/// neither delay is over in one tick.
pub(crate) fn heal_wait_ticks(food: &FoodFact) -> u16 {
    let wait = match (food.eat_delay_arg, food.message_delay) {
        (Some(arg), _) => arg.saturating_add(1),
        (None, Some(delay)) => delay.saturating_add(2),
        (None, None) => 1,
    };
    u16::try_from(wait.max(1)).unwrap_or(u16::MAX)
}

/// What the Hitpoints stat has shown since a bite ran.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Heal {
    /// Hitpoints still read what they read before the bite and the bound is
    /// open: the last bite's heal is not seen, so Hitpoints are stale.
    Pending,
    /// Hitpoints rose: the heal landed.
    Rose,
    /// Hitpoints fell: a hit landed first, which is a reason to eat, not to
    /// wait.
    Fell,
    /// The bound passed with Hitpoints unchanged, for example a hit equal to
    /// the heal in the same tick.
    Unseen,
}

/// Judge the last bite from the current Hitpoints. `bite_ticks` counts
/// observed ticks since the server tick that ran the bite. Natural
/// regeneration can also move Hitpoints (`stat.rs2:58-60`); a caller asks
/// this only after the count drop showed the bite ran, so a regeneration
/// point cannot stand in for a bite that has not run.
pub(crate) fn heal(hp_before: i16, hp_now: i32, bite_ticks: u16, food: &FoodFact) -> Heal {
    if hp_now > i32::from(hp_before) {
        Heal::Rose
    } else if hp_now < i32::from(hp_before) {
        Heal::Fell
    } else if bite_ticks >= heal_wait_ticks(food) {
        Heal::Unseen
    } else {
        Heal::Pending
    }
}

/// How one bite ended, for its log line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// Hitpoints rose after the bite.
    Healed,
    /// Hitpoints fell after the bite: a hit landed first.
    Fell,
    /// The bound passed with Hitpoints unchanged.
    NoHeal,
    /// The server did not run the bite inside its observation window.
    NotRun,
}

impl Outcome {
    const fn name(self) -> &'static str {
        match self {
            Self::Healed => "healed",
            Self::Fell => "hp-fell",
            Self::NoHeal => "no-heal-seen",
            Self::NotRun => "not-run",
        }
    }
}

/// The owner of the bite, for its log line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Eater {
    Step,
    Guard,
    Combat,
}

impl Eater {
    const fn name(self) -> &'static str {
        match self {
            Self::Step => "step",
            Self::Guard => "guard",
            Self::Combat => "combat",
        }
    }
}

/// What a bite was chosen on, kept until its log line goes out: the critical
/// floor (the HP at or below which the next hit can kill and the bite is
/// taken whatever it overheals) and the danger the floors came from. Two
/// bytes: Hitpoints never pass 99 in revision 289, so both saturate, and the
/// danger byte keeps 255 for "unknown".
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Basis {
    critical: u8,
    danger: u8,
}

const DANGER_UNKNOWN: u8 = u8::MAX;

impl Basis {
    pub(crate) fn new(critical: i32, danger: Option<i32>) -> Self {
        Self {
            critical: u8::try_from(critical.max(0)).unwrap_or(u8::MAX),
            danger: danger.map_or(DANGER_UNKNOWN, |danger| {
                u8::try_from(danger.max(0)).unwrap_or(DANGER_UNKNOWN - 1)
            }),
        }
    }

    pub(crate) fn critical(self) -> i32 {
        i32::from(self.critical)
    }

    pub(crate) fn danger(self) -> Option<i32> {
        (self.danger != DANGER_UNKNOWN).then_some(i32::from(self.danger))
    }
}

/// One bite as the survival log line reports it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Bite {
    pub eater: Eater,
    pub outcome: Outcome,
    pub food_id: i32,
    pub heal: i32,
    pub hp_before: i32,
    pub hp_max: i32,
    pub basis: Basis,
    pub hp_after: i32,
}

impl Bite {
    /// The log text. Every field is a number except the owner, the outcome
    /// and an unknown danger.
    pub(crate) fn line(&self) -> String {
        let danger = self
            .basis
            .danger()
            .map_or_else(|| "unknown".to_owned(), |danger| danger.to_string());
        format!(
            "survival eat owner={} outcome={} food={} heal={} hp_before={} hp_max={} \
             danger={} critical_floor={} hp_after={}",
            self.eater.name(),
            self.outcome.name(),
            self.food_id,
            self.heal,
            self.hp_before,
            self.hp_max,
            danger,
            self.basis.critical(),
            self.hp_after,
        )
    }

    /// Emit the line once, at the end of the bite.
    pub(crate) fn log(&self) {
        api::host_log!(Category::NavEvent, Level::Info, "{}", self.line());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn food(eat_delay_arg: Option<i32>, message_delay: Option<i32>) -> FoodFact {
        FoodFact {
            item_id: 379,
            heal: 12,
            eat_delay_arg,
            skill_delay_arg: Some(3),
            message_delay,
        }
    }

    #[test]
    fn the_heal_wait_is_the_foods_own_delay() {
        assert_eq!(heal_wait_ticks(&food(Some(2), None)), 3);
        assert_eq!(heal_wait_ticks(&food(Some(0), None)), 1);
        assert_eq!(heal_wait_ticks(&food(None, Some(1))), 3);
        assert_eq!(heal_wait_ticks(&food(None, None)), 1);
    }

    #[test]
    fn a_bite_waits_for_its_heal_until_the_bound() {
        let lobster = food(Some(2), None);
        assert_eq!(heal(21, 21, 0, &lobster), Heal::Pending);
        assert_eq!(heal(21, 21, 2, &lobster), Heal::Pending);
        assert_eq!(heal(21, 21, 3, &lobster), Heal::Unseen);
        assert_eq!(heal(21, 33, 0, &lobster), Heal::Rose, "the heal landed");
        assert_eq!(heal(21, 15, 0, &lobster), Heal::Fell, "a hit landed first");
    }

    #[test]
    fn the_log_line_names_every_field_of_the_bite() {
        let bite = Bite {
            eater: Eater::Step,
            outcome: Outcome::Healed,
            food_id: 379,
            heal: 12,
            hp_before: 21,
            hp_max: 40,
            basis: Basis::new(19, Some(18)),
            hp_after: 33,
        };
        assert_eq!(
            bite.line(),
            "survival eat owner=step outcome=healed food=379 heal=12 hp_before=21 hp_max=40 \
             danger=18 critical_floor=19 hp_after=33"
        );
        let unknown = Bite {
            basis: Basis::new(10, None),
            ..bite
        };
        assert!(unknown.line().contains("danger=unknown critical_floor=10"));
    }
}
