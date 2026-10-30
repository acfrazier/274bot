//! Pure poison-exposure classification and survival arithmetic.

use api::condition::{poison_math, BotCondition, Exposure, FoodSummary, PoisonState, SessionMark};

use crate::combat::request::ActorKind;
use crate::combat::select;
use crate::combat::tables::CombatTables;
use crate::combat::threats::{StyleObs, ThreatSet};

/// Classify whether the current live threat set can expose the player to poison.
///
/// The result is conservative. Any incomplete set, non-NPC row, unsupported
/// style, or missing poison fact returns [`Exposure::Unknown`].
pub fn exposure(threats: &ThreatSet, tables: &CombatTables, tick: u16) -> Exposure {
    if threats.has_overflow(tick) || threats.has_unattributed_event(tick) {
        return Exposure::Unknown;
    }

    let mut any = false;
    let mut max_pulse = 0;
    for threat in threats.iter(tick) {
        any = true;
        if threat.actor.kind != ActorKind::Npc
            || !matches!(threat.style, StyleObs::Melee | StyleObs::Ranged)
        {
            return Exposure::Unknown;
        }
        let Some(npc) = tables.npc(threat.ident) else {
            return Exposure::Unknown;
        };
        let Some(pulse) = crate::combat::risk::poison_pulse(npc.poison_severity) else {
            return Exposure::Unknown;
        };
        max_pulse = max_pulse.max(pulse);
    }

    if any {
        Exposure::Attributable { max_pulse }
    } else {
        Exposure::None
    }
}

/// The next poison fire's damage for this frame's condition fact.
///
/// An absent fact gives 0. The exposure comes from the caller's own threat
/// set at the same tick. The unknown-source bound comes from the caller's
/// selected tables. Callers add the result to their danger lines. One fire
/// can land on any accessible tick.
pub fn poison_fire(
    condition: Option<&BotCondition>,
    threats: &ThreatSet,
    tables: &CombatTables,
    tick: u16,
) -> i32 {
    let Some(condition) = condition else {
        return 0;
    };
    // `remaining` gives no fire for the other poison states, whatever the
    // exposure is. Return before the threat pass for them.
    if !matches!(
        condition.poison,
        PoisonState::Onset { .. } | PoisonState::Poisoned { .. }
    ) {
        return 0;
    }
    i32::from(
        poison_math::remaining(
            condition.poison,
            exposure(threats, tables, tick),
            tick,
            tables.unknown_poison_pulse(),
        )
        .per_fire_now,
    )
}

/// The reason that the shared survival need requires a cure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NeedWhy {
    /// Current HP is at or below the poison-adjusted emergency line.
    Emergency,
    /// Held food cannot carry HP through the remaining poison outlook.
    Lethal,
}

/// Additional held supplies required to clear a survival need.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurvivalNeed {
    /// One cure dose, or this many ordinary food items when no cure is allowed.
    Cure {
        /// Ordinary food items to add as a cure alternative.
        or_food: u8,
        /// Heal value used to estimate the food quantity.
        ref_heal: u8,
        /// Why the poison requires a cure.
        why: NeedWhy,
    },
    /// Ordinary food items required to reach the effective eating line.
    Food {
        /// Ordinary food items to add.
        count: u8,
        /// Heal value used to estimate the food quantity.
        ref_heal: u8,
    },
}

/// Return the heal the held ordinary food can deliver below the eating line.
///
/// The smallest held heal is credited for each held item. This is a
/// conservative lower bound that does not copy or inspect inventory rows.
pub fn food_credit(food: &FoodSummary, hp_max: u8, eat_line: i32) -> u16 {
    let cap = i32::from(hp_max).saturating_sub(eat_line).max(0);
    let heal = food.smallest_heal.min(u8::try_from(cap).unwrap_or(u8::MAX));
    u16::from(food.ordinary_count).saturating_mul(u16::from(heal))
}

/// Return the additional held supplies needed to satisfy the current condition.
///
/// `eat_line` is the caller's effective line. It may include a caller-specific
/// floor in addition to the poison-adjusted combat line.
pub fn survival_need(
    condition: Option<&BotCondition>,
    danger: Option<i32>,
    exposure: Exposure,
    tick: u16,
    q_unknown: u8,
    bank_best_heal: Option<u8>,
    eat_line: i32,
) -> Option<SurvivalNeed> {
    let condition = condition
        .filter(|condition| condition.session == SessionMark::Live && condition.hp_known)?;
    let lines = select::lines(danger, i32::from(condition.hp_max));
    let poison_active = matches!(
        condition.poison,
        PoisonState::Onset { .. } | PoisonState::Poisoned { .. }
    );
    let outlook = poison_math::remaining(condition.poison, exposure, tick, q_unknown);
    let lines = lines.with_poison(i32::from(outlook.per_fire_now));
    let eat_line = eat_line.max(lines.eat);
    let ref_heal = if condition.food.ordinary_count > 0 && condition.food.best_heal > 0 {
        condition.food.best_heal
    } else {
        bank_best_heal.filter(|heal| *heal > 0).unwrap_or(12)
    };
    let unit = i32::from(ref_heal).min(i32::from(condition.hp_max).saturating_sub(eat_line).max(0));
    let food = food_credit(&condition.food, condition.hp_max, eat_line);
    let hp = i32::from(condition.hp);
    if poison_active {
        let why = if hp <= lines.emergency {
            Some(NeedWhy::Emergency)
        } else if hp.saturating_add(i32::from(food)) <= i32::from(outlook.total) {
            Some(NeedWhy::Lethal)
        } else {
            None
        }?;
        let deficit = match why {
            NeedWhy::Emergency => lines.emergency.saturating_sub(hp).saturating_add(1),
            NeedWhy::Lethal => i32::from(outlook.total)
                .saturating_sub(hp)
                .saturating_sub(i32::from(food))
                .saturating_add(1),
        };
        return Some(SurvivalNeed::Cure {
            or_food: quantity(deficit, unit),
            ref_heal,
            why,
        });
    }
    if hp <= eat_line && condition.food.ordinary_count == 0 && unit > 0 {
        return Some(SurvivalNeed::Food {
            count: quantity(eat_line.saturating_sub(hp).saturating_add(1), unit),
            ref_heal,
        });
    }
    None
}

fn quantity(deficit: i32, unit: i32) -> u8 {
    if unit <= 0 {
        return 0;
    }
    let units = deficit.max(1).saturating_add(unit.saturating_sub(1)) / unit;
    u8::try_from(units).unwrap_or(u8::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::condition::{CureDoses, FoodSummary};

    fn fact(hp: u8, poison: PoisonState, food: FoodSummary) -> BotCondition {
        BotCondition {
            tick: 20,
            session: SessionMark::Live,
            hp,
            hp_max: 99,
            hp_known: true,
            poison,
            food,
            cures: CureDoses::default(),
        }
    }

    fn onset(q: Option<u8>) -> PoisonState {
        PoisonState::Onset {
            since: 20,
            q,
            hit_since_line: false,
        }
    }

    fn cure(need: Option<SurvivalNeed>) -> (u8, NeedWhy) {
        match need.expect("need exists") {
            SurvivalNeed::Cure { or_food, why, .. } => (or_food, why),
            SurvivalNeed::Food { .. } => panic!("expected cure need"),
        }
    }

    #[test]
    fn need_emergency_arm_at_line() {
        let condition = fact(5, onset(Some(11)), FoodSummary::default());
        let (or_food, why) = cure(survival_need(
            Some(&condition),
            None,
            Exposure::None,
            20,
            11,
            None,
            0,
        ));
        assert_eq!(why, NeedWhy::Emergency);
        assert_eq!(or_food, 5);
    }

    // Pinned content gives Tribesman poison severity 55
    // (`content/scripts/_unpack/225/all.npc:4598-4634`); the poison timer queues
    // `calc((%poison + 4) / 5)` then decrements by one
    // (`content/scripts/skill_combat/scripts/poison.rs2:20-30`), so this source
    // is pulse 11 and Rq(11) is 330. Lobster heals 12
    // (`content/scripts/player/configs/consumption/consume_normal.dbrow:172-180`),
    // so ten held lobsters credit 120 below the line, not their nominal total
    // heals at arbitrary HP.
    // `Some(0)` below is a complete, quiet threat set; `None` means danger is
    // unknown and keeps the conservative half-max eat line.
    #[test]
    fn need_lethal_at_line_unknown_source() {
        let condition = fact(
            60,
            onset(None),
            FoodSummary {
                ordinary_count: 10,
                best_heal: 12,
                smallest_heal: 12,
                ..FoodSummary::default()
            },
        );
        let (or_food, why) = cure(survival_need(
            Some(&condition),
            Some(0),
            Exposure::None,
            20,
            11,
            None,
            0,
        ));
        assert_eq!(why, NeedWhy::Lethal);
        assert_eq!(or_food, 13);
    }

    #[test]
    fn need_none_at_line_latched_weak_source() {
        let condition = fact(60, onset(Some(3)), FoodSummary::default());
        assert_eq!(
            survival_need(Some(&condition), None, Exposure::None, 20, 11, None, 0,),
            None
        );
    }

    #[test]
    fn need_latched_strong_source_survives_weak_current_set() {
        let condition = fact(60, onset(Some(11)), FoodSummary::default());
        let (_, why) = cure(survival_need(
            Some(&condition),
            Some(0),
            Exposure::Attributable { max_pulse: 3 },
            20,
            11,
            None,
            0,
        ));
        assert_eq!(why, NeedWhy::Lethal);
    }

    #[test]
    fn need_lethal_counts_usable_food() {
        let food = FoodSummary {
            ordinary_count: 5,
            best_heal: 12,
            smallest_heal: 12,
            ..FoodSummary::default()
        };
        let condition = fact(
            30,
            PoisonState::Poisoned {
                per_tick: 6,
                last_mark: 20,
                hit_since_mark: false,
            },
            food,
        );
        assert_eq!(
            cure(survival_need(
                Some(&condition),
                Some(0),
                Exposure::None,
                20,
                11,
                None,
                0,
            )),
            (1, NeedWhy::Lethal)
        );
        let six_food = fact(
            30,
            condition.poison,
            FoodSummary {
                ordinary_count: 6,
                ..food
            },
        );
        assert_eq!(
            survival_need(Some(&six_food), Some(0), Exposure::None, 20, 11, None, 0,),
            None
        );
    }

    // The content's Lobster row sets a 12-HP heal
    // (`content/scripts/player/configs/consumption/consume_normal.dbrow:172-180`).
    // At HP max 10 and eat line 8, each held Lobster credits only the 2 HP
    // that can fit below the line (`food_credit` is a lower bound, not delivered healing).
    #[test]
    fn need_food_credit_caps_at_eating_line() {
        let food = FoodSummary {
            ordinary_count: 3,
            best_heal: 12,
            smallest_heal: 12,
            ..FoodSummary::default()
        };
        assert_eq!(food_credit(&food, 10, 8), 6);
    }

    #[test]
    fn food_credit_is_conservative() {
        for hp_max in 10_u8..=99 {
            for eat_line in 1_u8..=20.min(hp_max) {
                for heal in 1_u8..=22 {
                    let food = FoodSummary {
                        ordinary_count: 3,
                        best_heal: heal,
                        smallest_heal: heal,
                        ..FoodSummary::default()
                    };
                    let credit = food_credit(&food, hp_max, i32::from(eat_line));
                    let hp_before = i32::from(eat_line - 1);
                    let room = i32::from(hp_max) - hp_before;
                    let actual_bite = u16::try_from(i32::from(heal).min(room)).unwrap();
                    let delivered = u16::from(food.ordinary_count) * actual_bite;
                    assert!(
                        delivered >= credit,
                        "hp_max={hp_max} eat_line={eat_line} heal={heal} credit={credit} delivered={delivered}"
                    );
                }
            }
        }

        let food = FoodSummary {
            ordinary_count: 5,
            best_heal: 7,
            smallest_heal: 5,
            ..FoodSummary::default()
        };
        let credit = food_credit(&food, 30, 20);
        let delivered = u16::from(food.ordinary_count) * 7;
        assert!(delivered > credit);
    }

    #[test]
    fn need_food_uses_the_effective_eat_floor() {
        let condition = fact(45, PoisonState::Clear, FoodSummary::default());
        assert_eq!(
            survival_need(Some(&condition), None, Exposure::None, 20, 11, Some(9), 70,),
            Some(SurvivalNeed::Food {
                count: 3,
                ref_heal: 9,
            })
        );
    }

    #[test]
    fn need_food_uses_default_reference_heal_without_food_facts() {
        let condition = fact(49, PoisonState::Clear, FoodSummary::default());
        assert_eq!(
            survival_need(Some(&condition), None, Exposure::None, 20, 11, None, 0,),
            Some(SurvivalNeed::Food {
                count: 1,
                ref_heal: 12,
            })
        );
    }

    #[test]
    fn need_requires_a_live_known_fact() {
        let mut condition = fact(5, onset(Some(11)), FoodSummary::default());
        assert_eq!(
            survival_need(None, None, Exposure::None, 20, 11, None, 0),
            None
        );
        condition.session = SessionMark::Boundary;
        assert_eq!(
            survival_need(Some(&condition), None, Exposure::None, 20, 11, None, 0),
            None
        );
        condition.session = SessionMark::Live;
        condition.hp_known = false;
        assert_eq!(
            survival_need(Some(&condition), None, Exposure::None, 20, 11, None, 0),
            None
        );
    }
}
