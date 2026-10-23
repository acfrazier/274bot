//! Pure poison-cure decisions shared by the combat machine and walk guard.

use api::condition::{poison_math, BotCondition, Exposure, FoodSummary, PoisonState, SessionMark};
use api::snapshot::ItemView;

use crate::combat::schedule::{OpKind, Schedule};
use crate::combat::select;
use crate::combat::tables::{CombatTables, PotionKind};

/// Operator policy for consuming a held poison cure.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum CurePoison {
    /// Consume a cure when the poison outlook makes it necessary.
    #[default]
    WhenNeeded = 0,
    /// Consume a cure whenever poison is active.
    Always = 1,
    /// Do not consume a cure automatically.
    Never = 2,
}

impl CurePoison {
    /// Return the stable byte used by the profile setting mirror.
    pub const fn to_byte(self) -> u8 {
        self as u8
    }

    /// Decode a setting byte, using the safe default for unknown values.
    pub const fn from_byte(value: u8) -> Self {
        match value {
            1 => Self::Always,
            2 => Self::Never,
            _ => Self::WhenNeeded,
        }
    }
}

/// Why the decision requires a cure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CureWhy {
    /// The next poison fire reaches the emergency line.
    Emergency,
    /// Held food cannot keep the player alive through the poison outlook.
    Lethal,
    /// The remaining poison damage reaches the eat line.
    BelowLine,
    /// Route admission refused because of poison.
    RouteRefused,
    /// The operator selected the always-cure policy.
    Always,
}

/// One poison-cure decision for the current observed tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CureDecision {
    /// No cure is required by the current outlook.
    None,
    /// Use one selected antipoison potion.
    Drink {
        /// Selected inventory object id.
        item: i32,
        /// Poison-cure potion family.
        kind: PotionKind,
        /// Reason for the drink.
        why: CureWhy,
    },
    /// Eat strange fruit when no allowed antipoison potion is held.
    EatFruit {
        /// Selected inventory object id.
        item: i32,
        /// Reason for eating the fruit.
        why: CureWhy,
    },
    /// A cure is required, but the policy, inventory, onset window, or clock
    /// prevents a click this tick.
    Needed {
        /// Reason for the cure need.
        why: CureWhy,
        /// Whether an allowed cure item is held.
        held: bool,
    },
}

/// Borrowed inputs for one cure decision.
pub(crate) struct CureInputs<'a> {
    /// The latest complete condition fact, or `None` when it is unavailable.
    pub cond: Option<&'a BotCondition>,
    /// Current expected damage from live attackers.
    pub danger: Option<i32>,
    /// Current poison-source evidence.
    pub exposure: Exposure,
    /// Automatic cure policy.
    pub policy: CurePoison,
    /// Whether route admission refused for poison.
    pub route_refused_for_poison: bool,
    /// Borrowed current inventory rows.
    pub inventory: &'a [ItemView],
    /// Selected revision's immutable combat facts.
    pub tables: &'a CombatTables,
    /// Shared action schedule.
    pub schedule: &'a Schedule,
    /// Whether clue inventory reserves forbid super antipoison.
    pub clue_reserve: bool,
    /// Current observed game tick.
    pub tick: u16,
    /// Strongest selected poison pulse.
    pub q_unknown: u8,
    /// Caller-specific minimum eat line.
    pub eat_floor: i32,
}

/// Select a cure without copying inventory rows or allocating.
pub(crate) fn cure_choice(input: &CureInputs<'_>) -> CureDecision {
    let Some(condition) = input.cond.filter(|condition| {
        condition.session == SessionMark::Live
            && condition.hp_known
            && matches!(
                condition.poison,
                PoisonState::Poisoned { .. } | PoisonState::Onset { .. }
            )
    }) else {
        return CureDecision::None;
    };

    let outlook = poison_math::remaining(
        condition.poison,
        input.exposure,
        input.tick,
        input.q_unknown,
    );
    let base_lines = select::lines(input.danger, i32::from(condition.hp_max));
    let fire = i32::from(outlook.per_fire_now);
    let emergency = base_lines.emergency.saturating_add(fire);
    let eat_line = base_lines.eat.saturating_add(fire).max(input.eat_floor);
    let usable = food_credit(condition.food, condition.hp_max, eat_line);
    let hp = i32::from(condition.hp);
    let remaining = i32::from(outlook.total);

    let why = if hp <= emergency {
        Some(CureWhy::Emergency)
    } else if hp.saturating_add(i32::from(usable)) <= remaining {
        Some(CureWhy::Lethal)
    } else if hp.saturating_sub(remaining) <= eat_line {
        Some(CureWhy::BelowLine)
    } else if input.route_refused_for_poison {
        Some(CureWhy::RouteRefused)
    } else if input.policy == CurePoison::Always {
        Some(CureWhy::Always)
    } else {
        None
    };
    let Some(why) = why else {
        return CureDecision::None;
    };

    let ordinary = held_dose(input.inventory, input.tables, PotionKind::Antipoison)
        .map(|item| (item, PotionKind::Antipoison));
    let super_antipoison = (!input.clue_reserve)
        .then(|| held_dose(input.inventory, input.tables, PotionKind::SuperAntipoison))
        .flatten()
        .map(|item| (item, PotionKind::SuperAntipoison));
    let candidate = ordinary.or(super_antipoison);
    let fruit = held_fruit(input.inventory, input.tables);
    let held = candidate.is_some() || fruit.is_some();

    if input.policy == CurePoison::Never {
        return CureDecision::Needed { why, held };
    }
    if outlook.pending
        && input.policy != CurePoison::Always
        && matches!(why, CureWhy::Lethal | CureWhy::BelowLine)
    {
        return CureDecision::Needed { why, held };
    }
    if input.schedule.pending(OpKind::Drink) || !input.schedule.ready(OpKind::Drink, input.tick) {
        return CureDecision::Needed { why, held };
    }
    if let Some((item, kind)) = candidate {
        return CureDecision::Drink { item, kind, why };
    }
    if let Some(item) = fruit {
        return CureDecision::EatFruit { item, why };
    }
    CureDecision::Needed { why, held: false }
}

fn held_dose(inventory: &[ItemView], tables: &CombatTables, kind: PotionKind) -> Option<i32> {
    let family = tables.potion(kind)?;
    family
        .doses
        .iter()
        .flatten()
        .filter(|dose| {
            inventory
                .iter()
                .any(|item| item.count > 0 && item.def.id == dose.id)
        })
        .min_by_key(|dose| dose.doses)
        .map(|dose| dose.id)
}

fn held_fruit(inventory: &[ItemView], tables: &CombatTables) -> Option<i32> {
    let item_id = tables.selected().item_by_alias("macro_triffidfruit")?.id;
    inventory
        .iter()
        .any(|item| item.count > 0 && item.def.id == item_id)
        .then_some(item_id)
}

fn food_credit(food: FoodSummary, hp_max: u8, eat_line: i32) -> u16 {
    let cap = (i32::from(hp_max) - eat_line).max(0);
    let unit = u16::from(food.smallest_heal.min(u8::try_from(cap).unwrap_or(u8::MAX)));
    u16::from(food.ordinary_count).saturating_mul(unit)
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::combat::schedule::{InputEffect, Schedule};
    use api::condition::{CureDoses, SessionMark};
    use api::obj_names::ItemDefView;
    use api::selected::ClientRevision;
    use api::snapshot::{ItemActionFamily, ItemContainer};

    fn tables() -> std::sync::Arc<CombatTables> {
        CombatTables::build(api::game_data::for_revision(ClientRevision::R289).unwrap()).unwrap()
    }

    fn held(id: i32, count: i32) -> ItemView {
        ItemView {
            def: ItemDefView {
                id,
                name: Some("held cure".into()),
                stackable: false,
                members: true,
                base_value: 0,
                noted: false,
                certificate_link: -1,
                certificate_template: -1,
            },
            container: ItemContainer::Inventory,
            action_family: ItemActionFamily::Held,
            slot: 0,
            count,
            actions: Vec::new(),
            component_id: 0,
        }
    }

    fn dose_id(tables: &CombatTables, kind: PotionKind, doses: u8) -> i32 {
        tables.potion(kind).unwrap().doses[usize::from(doses - 1)]
            .unwrap()
            .id
    }

    fn condition(hp: u8, hp_max: u8, poison: PoisonState, food: FoodSummary) -> BotCondition {
        BotCondition {
            tick: 10,
            session: SessionMark::Live,
            hp,
            hp_max,
            hp_known: true,
            poison,
            food,
            cures: CureDoses::default(),
        }
    }

    fn food(count: u8, smallest_heal: u8) -> FoodSummary {
        FoodSummary {
            ordinary_count: count,
            smallest_heal,
            ..FoodSummary::default()
        }
    }

    fn assert_case(
        tables: &CombatTables,
        condition: Option<BotCondition>,
        exposure: Exposure,
        policy: CurePoison,
        inventory: &[ItemView],
        q_unknown: u8,
        expected: CureDecision,
    ) {
        let schedule = Schedule::default();
        let actual = cure_choice(&CureInputs {
            cond: condition.as_ref(),
            danger: Some(0),
            exposure,
            policy,
            route_refused_for_poison: false,
            inventory,
            tables,
            schedule: &schedule,
            clue_reserve: false,
            tick: 10,
            q_unknown,
            eat_floor: 0,
        });
        assert_eq!(actual, expected, "condition {condition:?}");
    }

    #[test]
    fn cure_choice_rows() {
        let tables = tables();
        let ordinary_id = dose_id(&tables, PotionKind::Antipoison, 1);
        let ordinary = [held(ordinary_id, 1)];
        let unknown = PoisonState::Unknown { since: 0 };
        let clear = PoisonState::Clear;
        let immune = PoisonState::Immune { until_lower: 100 };
        let mark1 = PoisonState::Poisoned {
            per_tick: 1,
            last_mark: 0,
            hit_since_mark: false,
        };
        let mark2 = PoisonState::Poisoned {
            per_tick: 2,
            last_mark: 0,
            hit_since_mark: false,
        };
        let mark3 = PoisonState::Poisoned {
            per_tick: 3,
            last_mark: 0,
            hit_since_mark: false,
        };
        let mark5 = PoisonState::Poisoned {
            per_tick: 5,
            last_mark: 0,
            hit_since_mark: false,
        };
        let mark6 = PoisonState::Poisoned {
            per_tick: 6,
            last_mark: 0,
            hit_since_mark: false,
        };
        let mark6_after_hit = PoisonState::Poisoned {
            per_tick: 6,
            last_mark: 0,
            hit_since_mark: true,
        };
        let onset3 = PoisonState::Onset {
            since: 0,
            q: Some(3),
            hit_since_line: false,
        };
        let onset11 = PoisonState::Onset {
            since: 0,
            q: Some(11),
            hit_since_line: false,
        };
        let pending11 = PoisonState::Onset {
            since: 0,
            q: None,
            hit_since_line: false,
        };

        assert_case(
            &tables,
            None,
            Exposure::None,
            CurePoison::WhenNeeded,
            &ordinary,
            11,
            CureDecision::None,
        );
        let mut cond = condition(60, 99, mark6, FoodSummary::default());
        cond.session = SessionMark::Dead;
        assert_case(
            &tables,
            Some(cond),
            Exposure::None,
            CurePoison::WhenNeeded,
            &ordinary,
            11,
            CureDecision::None,
        );
        let mut cond = condition(60, 99, mark6, FoodSummary::default());
        cond.hp_known = false;
        assert_case(
            &tables,
            Some(cond),
            Exposure::None,
            CurePoison::WhenNeeded,
            &ordinary,
            11,
            CureDecision::None,
        );
        assert_case(
            &tables,
            Some(condition(60, 99, unknown, FoodSummary::default())),
            Exposure::None,
            CurePoison::WhenNeeded,
            &ordinary,
            11,
            CureDecision::None,
        );
        assert_case(
            &tables,
            Some(condition(60, 99, clear, FoodSummary::default())),
            Exposure::None,
            CurePoison::WhenNeeded,
            &ordinary,
            11,
            CureDecision::None,
        );
        assert_case(
            &tables,
            Some(condition(60, 99, immune, FoodSummary::default())),
            Exposure::None,
            CurePoison::WhenNeeded,
            &ordinary,
            11,
            CureDecision::None,
        );
        assert_case(
            &tables,
            Some(condition(40, 99, mark1, FoodSummary::default())),
            Exposure::None,
            CurePoison::WhenNeeded,
            &ordinary,
            11,
            CureDecision::None,
        );
        assert_case(
            &tables,
            Some(condition(20, 99, mark2, FoodSummary::default())),
            Exposure::None,
            CurePoison::WhenNeeded,
            &ordinary,
            11,
            CureDecision::None,
        );
        assert_case(
            &tables,
            Some(condition(20, 99, mark2, FoodSummary::default())),
            Exposure::None,
            CurePoison::Always,
            &ordinary,
            11,
            CureDecision::Drink {
                item: ordinary_id,
                kind: PotionKind::Antipoison,
                why: CureWhy::Always,
            },
        );
        assert_case(
            &tables,
            Some(condition(30, 99, mark6, food(5, 12))),
            Exposure::None,
            CurePoison::WhenNeeded,
            &ordinary,
            11,
            CureDecision::Drink {
                item: ordinary_id,
                kind: PotionKind::Antipoison,
                why: CureWhy::Lethal,
            },
        );
        assert_case(
            &tables,
            Some(condition(60, 99, mark6_after_hit, food(10, 12))),
            Exposure::None,
            CurePoison::WhenNeeded,
            &ordinary,
            11,
            CureDecision::Drink {
                item: ordinary_id,
                kind: PotionKind::Antipoison,
                why: CureWhy::Lethal,
            },
        );
        assert_case(
            &tables,
            Some(condition(60, 99, mark6, food(10, 12))),
            Exposure::None,
            CurePoison::WhenNeeded,
            &ordinary,
            11,
            CureDecision::Drink {
                item: ordinary_id,
                kind: PotionKind::Antipoison,
                why: CureWhy::BelowLine,
            },
        );
        assert_case(
            &tables,
            Some(condition(10, 10, mark3, food(5, 12))),
            Exposure::None,
            CurePoison::WhenNeeded,
            &ordinary,
            11,
            CureDecision::Drink {
                item: ordinary_id,
                kind: PotionKind::Antipoison,
                why: CureWhy::BelowLine,
            },
        );
        // The usable credit is capped at `5 * min(12, 10 - 7) = 15` by the
        // eat line (`food_credit`), not the nominal 60.
        assert_case(
            &tables,
            Some(condition(10, 10, mark6, food(5, 12))),
            Exposure::None,
            CurePoison::WhenNeeded,
            &ordinary,
            11,
            CureDecision::Drink {
                item: ordinary_id,
                kind: PotionKind::Antipoison,
                why: CureWhy::Lethal,
            },
        );
        assert_case(
            &tables,
            Some(condition(75, 99, mark5, FoodSummary::default())),
            Exposure::None,
            CurePoison::WhenNeeded,
            &ordinary,
            11,
            CureDecision::Drink {
                item: ordinary_id,
                kind: PotionKind::Antipoison,
                why: CureWhy::BelowLine,
            },
        );
        assert_case(
            &tables,
            Some(condition(60, 99, onset11, food(10, 12))),
            Exposure::None,
            CurePoison::WhenNeeded,
            &ordinary,
            11,
            CureDecision::Drink {
                item: ordinary_id,
                kind: PotionKind::Antipoison,
                why: CureWhy::Lethal,
            },
        );
        assert_case(
            &tables,
            Some(condition(60, 99, onset3, FoodSummary::default())),
            Exposure::None,
            CurePoison::WhenNeeded,
            &ordinary,
            11,
            CureDecision::None,
        );
        assert_case(
            &tables,
            Some(condition(60, 99, pending11, FoodSummary::default())),
            Exposure::None,
            CurePoison::WhenNeeded,
            &ordinary,
            11,
            CureDecision::Needed {
                why: CureWhy::Lethal,
                held: true,
            },
        );
        assert_case(
            &tables,
            Some(condition(12, 99, pending11, FoodSummary::default())),
            Exposure::None,
            CurePoison::WhenNeeded,
            &ordinary,
            11,
            CureDecision::Drink {
                item: ordinary_id,
                kind: PotionKind::Antipoison,
                why: CureWhy::Emergency,
            },
        );
        assert_case(
            &tables,
            Some(condition(
                60,
                99,
                PoisonState::Onset {
                    since: 0,
                    q: Some(3),
                    hit_since_line: true,
                },
                FoodSummary::default(),
            )),
            Exposure::None,
            CurePoison::WhenNeeded,
            &ordinary,
            11,
            CureDecision::Drink {
                item: ordinary_id,
                kind: PotionKind::Antipoison,
                why: CureWhy::Lethal,
            },
        );
        assert_case(
            &tables,
            Some(condition(60, 99, onset3, FoodSummary::default())),
            Exposure::Unknown,
            CurePoison::WhenNeeded,
            &ordinary,
            11,
            CureDecision::Drink {
                item: ordinary_id,
                kind: PotionKind::Antipoison,
                why: CureWhy::Lethal,
            },
        );
    }

    #[test]
    fn cure_choice_onset_uses_latched_pulse() {
        let tables = tables();
        let ordinary_id = dose_id(&tables, PotionKind::Antipoison, 1);
        let ordinary = [held(ordinary_id, 1)];
        let onset3 = PoisonState::Onset {
            since: 0,
            q: Some(3),
            hit_since_line: false,
        };
        assert_case(
            &tables,
            Some(condition(60, 99, onset3, FoodSummary::default())),
            Exposure::None,
            CurePoison::WhenNeeded,
            &ordinary,
            11,
            CureDecision::None,
        );
        assert_case(
            &tables,
            Some(condition(20, 99, onset3, FoodSummary::default())),
            Exposure::None,
            CurePoison::WhenNeeded,
            &ordinary,
            11,
            CureDecision::Drink {
                item: ordinary_id,
                kind: PotionKind::Antipoison,
                why: CureWhy::Lethal,
            },
        );
        let onset11 = PoisonState::Onset {
            since: 0,
            q: Some(11),
            hit_since_line: false,
        };
        assert_case(
            &tables,
            Some(condition(60, 99, onset11, FoodSummary::default())),
            Exposure::Attributable { max_pulse: 3 },
            CurePoison::WhenNeeded,
            &ordinary,
            11,
            CureDecision::Drink {
                item: ordinary_id,
                kind: PotionKind::Antipoison,
                why: CureWhy::Lethal,
            },
        );
    }

    #[test]
    fn cure_choice_pending_lethal_reports_only_and_emergency_acts() {
        let tables = tables();
        let ordinary_id = dose_id(&tables, PotionKind::Antipoison, 1);
        let ordinary = [held(ordinary_id, 1)];
        let pending = PoisonState::Onset {
            since: 0,
            q: None,
            hit_since_line: false,
        };
        assert_case(
            &tables,
            Some(condition(60, 99, pending, FoodSummary::default())),
            Exposure::None,
            CurePoison::WhenNeeded,
            &ordinary,
            11,
            CureDecision::Needed {
                why: CureWhy::Lethal,
                held: true,
            },
        );
        assert_case(
            &tables,
            Some(condition(12, 99, pending, FoodSummary::default())),
            Exposure::None,
            CurePoison::WhenNeeded,
            &ordinary,
            11,
            CureDecision::Drink {
                item: ordinary_id,
                kind: PotionKind::Antipoison,
                why: CureWhy::Emergency,
            },
        );
        assert_case(
            &tables,
            Some(condition(60, 99, pending, FoodSummary::default())),
            Exposure::None,
            CurePoison::Always,
            &ordinary,
            1,
            CureDecision::Drink {
                item: ordinary_id,
                kind: PotionKind::Antipoison,
                why: CureWhy::Always,
            },
        );
    }

    #[test]
    fn cure_choice_onset_unknown_source_uses_strongest() {
        let tables = tables();
        let ordinary_id = dose_id(&tables, PotionKind::Antipoison, 1);
        let ordinary = [held(ordinary_id, 1)];
        let onset = PoisonState::Onset {
            since: 0,
            q: Some(3),
            hit_since_line: true,
        };
        for exposure in [Exposure::None, Exposure::Unknown] {
            assert_case(
                &tables,
                Some(condition(60, 99, onset, FoodSummary::default())),
                exposure,
                CurePoison::WhenNeeded,
                &ordinary,
                11,
                CureDecision::Drink {
                    item: ordinary_id,
                    kind: PotionKind::Antipoison,
                    why: CureWhy::Lethal,
                },
            );
        }
    }

    #[test]
    fn cure_choice_smaller_mark_is_not_lethal() {
        let tables = tables();
        let ordinary_id = dose_id(&tables, PotionKind::Antipoison, 1);
        let ordinary = [held(ordinary_id, 1)];
        let condition = condition(
            75,
            99,
            PoisonState::Poisoned {
                per_tick: 5,
                last_mark: 10,
                hit_since_mark: false,
            },
            FoodSummary::default(),
        );
        assert_case(
            &tables,
            Some(condition),
            Exposure::None,
            CurePoison::WhenNeeded,
            &ordinary,
            11,
            CureDecision::Drink {
                item: ordinary_id,
                kind: PotionKind::Antipoison,
                why: CureWhy::BelowLine,
            },
        );
    }

    #[test]
    fn cure_choice_ordinary_before_super_and_reserve_forbids_super() {
        let tables = tables();
        let ordinary1 = dose_id(&tables, PotionKind::Antipoison, 1);
        let ordinary4 = dose_id(&tables, PotionKind::Antipoison, 4);
        let super1 = dose_id(&tables, PotionKind::SuperAntipoison, 1);
        let state = condition(
            20,
            99,
            PoisonState::Poisoned {
                per_tick: 2,
                last_mark: 0,
                hit_since_mark: false,
            },
            FoodSummary::default(),
        );
        let schedule = Schedule::default();
        let decide = |inventory: &[ItemView], clue_reserve| {
            cure_choice(&CureInputs {
                cond: Some(&state),
                danger: Some(0),
                exposure: Exposure::None,
                policy: CurePoison::Always,
                route_refused_for_poison: false,
                inventory,
                tables: &tables,
                schedule: &schedule,
                clue_reserve,
                tick: 10,
                q_unknown: 11,
                eat_floor: 0,
            })
        };
        assert_eq!(
            decide(&[held(ordinary4, 1), held(ordinary1, 1)], false),
            CureDecision::Drink {
                item: ordinary1,
                kind: PotionKind::Antipoison,
                why: CureWhy::Always,
            }
        );
        assert_eq!(
            decide(&[held(super1, 1)], false),
            CureDecision::Drink {
                item: super1,
                kind: PotionKind::SuperAntipoison,
                why: CureWhy::Always,
            }
        );
        assert_eq!(
            decide(&[held(super1, 1)], true),
            CureDecision::Needed {
                why: CureWhy::Always,
                held: false,
            }
        );
    }

    #[test]
    fn cure_choice_orders_doses_and_respects_policy_route_and_clock() {
        let tables = tables();
        let ordinary_1 = dose_id(&tables, PotionKind::Antipoison, 1);
        let ordinary_4 = dose_id(&tables, PotionKind::Antipoison, 4);
        let super_1 = dose_id(&tables, PotionKind::SuperAntipoison, 1);
        let fruit_id = tables
            .selected()
            .item_by_alias("macro_triffidfruit")
            .unwrap()
            .id;
        let poison = PoisonState::Poisoned {
            per_tick: 2,
            last_mark: 0,
            hit_since_mark: false,
        };
        let state = condition(20, 99, poison, FoodSummary::default());
        let ordinary = [held(ordinary_4, 1), held(ordinary_1, 1)];
        assert_eq!(
            cure_choice(&CureInputs {
                cond: Some(&state),
                danger: Some(0),
                exposure: Exposure::None,
                policy: CurePoison::Always,
                route_refused_for_poison: false,
                inventory: &ordinary,
                tables: &tables,
                schedule: &Schedule::default(),
                clue_reserve: false,
                tick: 10,
                q_unknown: 11,
                eat_floor: 0,
            }),
            CureDecision::Drink {
                item: ordinary_1,
                kind: PotionKind::Antipoison,
                why: CureWhy::Always,
            }
        );
        let super_only = [held(super_1, 1)];
        assert_eq!(
            cure_choice(&CureInputs {
                cond: Some(&state),
                danger: Some(0),
                exposure: Exposure::None,
                policy: CurePoison::Always,
                route_refused_for_poison: false,
                inventory: &super_only,
                tables: &tables,
                schedule: &Schedule::default(),
                clue_reserve: false,
                tick: 10,
                q_unknown: 11,
                eat_floor: 0,
            }),
            CureDecision::Drink {
                item: super_1,
                kind: PotionKind::SuperAntipoison,
                why: CureWhy::Always,
            }
        );
        let super_and_fruit = [held(super_1, 1), held(fruit_id, 1)];
        assert_eq!(
            cure_choice(&CureInputs {
                cond: Some(&state),
                danger: Some(0),
                exposure: Exposure::None,
                policy: CurePoison::Always,
                route_refused_for_poison: false,
                inventory: &super_and_fruit,
                tables: &tables,
                schedule: &Schedule::default(),
                clue_reserve: true,
                tick: 10,
                q_unknown: 11,
                eat_floor: 0,
            }),
            CureDecision::EatFruit {
                item: fruit_id,
                why: CureWhy::Always,
            }
        );
        assert_eq!(
            cure_choice(&CureInputs {
                cond: Some(&state),
                danger: Some(0),
                exposure: Exposure::None,
                policy: CurePoison::Never,
                route_refused_for_poison: true,
                inventory: &ordinary,
                tables: &tables,
                schedule: &Schedule::default(),
                clue_reserve: false,
                tick: 10,
                q_unknown: 11,
                eat_floor: 0,
            }),
            CureDecision::Needed {
                why: CureWhy::RouteRefused,
                held: true,
            }
        );

        let safe = condition(
            40,
            99,
            PoisonState::Poisoned {
                per_tick: 1,
                last_mark: 0,
                hit_since_mark: false,
            },
            FoodSummary::default(),
        );
        let mut busy = Schedule::default();
        busy.admitted(OpKind::Drink, 10, 0, false, InputEffect::Standard);
        assert_eq!(
            cure_choice(&CureInputs {
                cond: Some(&safe),
                danger: Some(0),
                exposure: Exposure::None,
                policy: CurePoison::Always,
                route_refused_for_poison: false,
                inventory: &ordinary,
                tables: &tables,
                schedule: &busy,
                clue_reserve: false,
                tick: 10,
                q_unknown: 11,
                eat_floor: 0,
            }),
            CureDecision::Needed {
                why: CureWhy::Always,
                held: true,
            }
        );
    }

    #[test]
    fn cure_choice_warm_ticks_allocate_nothing() {
        let tables = tables();
        let state = condition(
            60,
            99,
            PoisonState::Onset {
                since: 0,
                q: Some(11),
                hit_since_line: false,
            },
            FoodSummary::default(),
        );
        let inventory = [held(dose_id(&tables, PotionKind::Antipoison, 1), 1)];
        let schedule = Schedule::default();
        let input = CureInputs {
            cond: Some(&state),
            danger: Some(0),
            exposure: Exposure::None,
            policy: CurePoison::WhenNeeded,
            route_refused_for_poison: false,
            inventory: &inventory,
            tables: &tables,
            schedule: &schedule,
            clue_reserve: false,
            tick: 10,
            q_unknown: 11,
            eat_floor: 0,
        };
        let allocations = allocation_counter::measure(|| {
            for _ in 0..1_000 {
                std::hint::black_box(cure_choice(&input));
            }
        });
        assert_eq!(allocations, allocation_counter::AllocationInfo::default());
    }

    #[test]
    fn cure_policy_bytes_are_stable_and_fail_safe() {
        assert_eq!(CurePoison::WhenNeeded.to_byte(), 0);
        assert_eq!(CurePoison::Always.to_byte(), 1);
        assert_eq!(CurePoison::Never.to_byte(), 2);
        assert_eq!(CurePoison::from_byte(0), CurePoison::WhenNeeded);
        assert_eq!(CurePoison::from_byte(1), CurePoison::Always);
        assert_eq!(CurePoison::from_byte(2), CurePoison::Never);
        assert_eq!(CurePoison::from_byte(u8::MAX), CurePoison::WhenNeeded);
    }
}
