//! Per-runner survival decisions and the non-terminal survival hold.

use std::fmt;

use api::condition::{poison_math, Exposure, PoisonState};
use api::hostlog::{Category, Emit, Level};

use crate::combat::frame::Frame;
use crate::combat::policy::{self, EatChoice};
use crate::combat::schedule::Schedule;
use crate::combat::select;
use crate::combat::threats::ThreatSet;
use crate::gatherer::oneop::OneOpArgs;
use crate::native::ActionContext;
use crate::survival::cure::{cure_choice, CureDecision, CureInputs, CureWhy};
use crate::survival::math::{self, NeedWhy, SurvivalNeed};

/// The last survival decision published by the owning native runner.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SurvivalReport {
    pub decision: Decision,
    pub why: Option<CureWhy>,
    pub need: Option<SurvivalNeed>,
    pub tried: u8,
    pub owner: Owner,
}

const _: () = assert!(std::mem::size_of::<SurvivalReport>() <= 12);

/// One current-frame survival outcome.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Decision {
    #[default]
    None,
    Drinking,
    Eating,
    Needed,
    Parked,
}

/// The owner that last ran the survival decision.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Owner {
    #[default]
    None,
    Guard,
    Combat,
    Step,
}

/// One runner action or the supply need that keeps the runner from work.
pub enum SurvivalWant {
    Nothing,
    Click(OneOpArgs),
    Need(SurvivalNeed),
}

/// A runner's compact threat history, park and current report.
#[derive(Default)]
pub struct SurvivalStep {
    threats: ThreatSet,
    park: Option<SurvivalNeed>,
    last: SurvivalReport,
}

const _: () = assert!(std::mem::size_of::<SurvivalStep>() <= 192);

impl SurvivalStep {
    /// Observe threats and choose the cure, bite, or park trigger for this frame.
    /// `eat_floor` is the caller's additional minimum eating line.
    pub fn want(&mut self, cx: &ActionContext<'_>, eat_floor: i32) -> SurvivalWant {
        let snapshot = cx.snapshot();
        let Some(frame) = Frame::borrow(snapshot) else {
            self.last = SurvivalReport::default();
            return SurvivalWant::Nothing;
        };
        let Some(tables) = crate::combat::guard::shared_tables_ref() else {
            self.last = SurvivalReport::default();
            return SurvivalWant::Nothing;
        };
        let tick = frame.tick;
        self.threats.observe(&frame, tables, tick);
        let danger = self.threats.danger(&frame, tables, tick, false, false);
        let exposure = math::exposure(&self.threats, tables, tick);
        let condition = cx.condition();
        let q_unknown = cx.poison_unknown_pulse();
        let outlook = condition
            .map(|condition| poison_math::remaining(condition.poison, exposure, tick, q_unknown));
        let poison_fire = outlook.map_or(0, |outlook| i32::from(outlook.per_fire_now));
        let schedule = Schedule::default();
        let cure = cure_choice(&CureInputs {
            cond: condition,
            danger,
            exposure,
            policy: cx.cure_poison(),
            route_refused_for_poison: false,
            inventory: frame.inventory,
            tables,
            schedule: &schedule,
            clue_reserve: false,
            tick,
            q_unknown,
            eat_floor,
        });
        let hp = frame
            .stats
            .iter()
            .find(|stat| stat.index == 3)
            .map(|stat| stat.effective);
        let hp_max = frame
            .stats
            .iter()
            .find(|stat| stat.index == 3)
            .map_or(0, |stat| stat.base);
        let eat_line = select::lines(danger, hp_max)
            .with_poison(poison_fire)
            .eat
            .max(eat_floor);
        let need =
            math::survival_need(condition, danger, exposure, tick, q_unknown, None, eat_line);
        let why = match cure {
            CureDecision::Drink { why, .. }
            | CureDecision::EatFruit { why, .. }
            | CureDecision::Needed { why, .. } => Some(why),
            CureDecision::None => None,
        };
        if let Some((args, decision)) = click(cure, frame.inventory, hp) {
            self.last = SurvivalReport {
                decision,
                why,
                need,
                tried: 0,
                owner: Owner::Step,
            };
            return SurvivalWant::Click(args);
        }
        if let Some(hp) = hp {
            if let Some(choice) = policy::eat_choice_with_floor(
                &frame,
                tables,
                &schedule,
                danger,
                poison_fire,
                hp,
                hp_max,
                tick,
                eat_floor,
            ) {
                if let Some(args) = eat_args(choice, frame.inventory, hp) {
                    self.last = SurvivalReport {
                        decision: Decision::Eating,
                        why,
                        need,
                        tried: 0,
                        owner: Owner::Step,
                    };
                    return SurvivalWant::Click(args);
                }
            }
        }

        if let Some(need) = need {
            let report_only = matches!(need, SurvivalNeed::Cure { .. })
                && cx.cure_poison() == crate::survival::CurePoison::Never;
            let pending_lethal = matches!(
                need,
                SurvivalNeed::Cure {
                    why: NeedWhy::Lethal,
                    ..
                }
            ) && outlook.is_some_and(|outlook| outlook.pending);
            let held_allowed_cure = matches!(cure, CureDecision::Needed { held: true, .. });
            self.last = SurvivalReport {
                decision: if self.park.is_some() {
                    Decision::Parked
                } else {
                    Decision::Needed
                },
                why,
                need: Some(need),
                tried: 0,
                owner: Owner::Step,
            };
            if self.park.is_none() && !report_only && !pending_lethal && !held_allowed_cure {
                return SurvivalWant::Need(need);
            }
            return SurvivalWant::Nothing;
        }

        if need.is_none() && self.park.is_some() {
            self.unpark(cx);
        }

        self.last = match cure {
            CureDecision::Needed { why, .. } => SurvivalReport {
                decision: Decision::Needed,
                why: Some(why),
                need: None,
                tried: 0,
                owner: Owner::Step,
            },
            _ if self.park.is_some() => SurvivalReport {
                decision: Decision::Parked,
                why: None,
                need: self.park,
                tried: 0,
                owner: Owner::Step,
            },
            _ => SurvivalReport::default(),
        };
        SurvivalWant::Nothing
    }

    pub fn parked(&self) -> Option<SurvivalNeed> {
        self.park
    }

    pub fn report(&self) -> SurvivalReport {
        self.last
    }

    /// Enter the runner-owned hold and record its transition once.
    pub fn park(&mut self, need: SurvivalNeed, cx: &ActionContext<'_>) {
        if self.park.is_none() {
            let (hp, remaining, q, widened) = self.log_context(cx);
            api::hostlog::emit(
                Emit {
                    category: Category::NavEvent,
                    level: Level::Info,
                    slot: None,
                    always_stderr: false,
                },
                format_args!(
                    "survival park need={need} hp={hp} R={remaining} q={q} widened={widened}"
                ),
            );
        }
        self.park = Some(need);
        self.last = SurvivalReport {
            decision: Decision::Parked,
            why: self.last.why,
            need: Some(need),
            tried: self.last.tried,
            owner: Owner::Step,
        };
    }

    /// Leave the hold after fresh evidence shows the need is gone.
    pub fn unpark(&mut self, cx: &ActionContext<'_>) {
        if self.park.take().is_some() {
            let (hp, remaining, _, _) = self.log_context(cx);
            api::hostlog::emit(
                Emit {
                    category: Category::NavEvent,
                    level: Level::Info,
                    slot: None,
                    always_stderr: false,
                },
                format_args!("survival unpark hp={hp} R={remaining}"),
            );
        }
        self.last = SurvivalReport::default();
    }

    fn log_context(&self, cx: &ActionContext<'_>) -> (i32, u16, u8, bool) {
        let condition = cx.condition();
        let Some(tables) = crate::combat::guard::shared_tables_ref() else {
            return (
                condition.map_or(0, |condition| i32::from(condition.hp)),
                0,
                0,
                true,
            );
        };
        let snapshot = cx.snapshot();
        let tick = u16::try_from(cx.evidence().tick).unwrap_or(u16::MAX);
        let exposure = Frame::borrow(snapshot).map_or(Exposure::Unknown, |frame| {
            math::exposure(&self.threats, tables, frame.tick)
        });
        let q_unknown = cx.poison_unknown_pulse();
        let (hp, remaining, q, widened) = condition.map_or((0, 0, 0, true), |condition| {
            let outlook = poison_math::remaining(condition.poison, exposure, tick, q_unknown);
            let (q, widened) = match condition.poison {
                PoisonState::Onset { q, .. } => (q.unwrap_or(q_unknown), q.is_none()),
                PoisonState::Poisoned {
                    per_tick,
                    hit_since_mark,
                    ..
                } => (per_tick, hit_since_mark || exposure == Exposure::Unknown),
                _ => (0, exposure == Exposure::Unknown),
            };
            (i32::from(condition.hp), outlook.total, q, widened)
        });
        (hp, remaining, q, widened)
    }
}

impl fmt::Display for SurvivalNeed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SurvivalNeed::Cure { or_food, .. } if *or_food > 0 => {
                write!(f, "poisoned: would die; needs antipoison or {or_food} food")
            }
            SurvivalNeed::Cure { .. } => f.write_str("poisoned: would die; needs antipoison"),
            SurvivalNeed::Food { count, .. } => write!(f, "would die: needs {count} food"),
        }
    }
}

impl fmt::Display for SurvivalReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self.decision {
            Decision::None => "None",
            Decision::Drinking => "Drinking",
            Decision::Eating => "Eating",
            Decision::Needed => "Needed",
            Decision::Parked => "Parked",
        })?;
        if let Some(need) = self.need {
            write!(f, ": {need}")?;
        } else if let Some(why) = self.why {
            f.write_str(match why {
                CureWhy::Emergency => ": emergency",
                CureWhy::Lethal => ": lethal",
                CureWhy::BelowLine => ": below eating line",
                CureWhy::RouteRefused => ": route refused",
                CureWhy::Always => ": always",
            })?;
        }
        Ok(())
    }
}

fn click(
    cure: CureDecision,
    inventory: &[api::snapshot::ItemView],
    hp: Option<i32>,
) -> Option<(OneOpArgs, Decision)> {
    match cure {
        CureDecision::Drink { item, .. } => {
            let (name, count) = held_item(inventory, item)?;
            Some((OneOpArgs::drink(name, item, count), Decision::Drinking))
        }
        CureDecision::EatFruit { item, .. } => {
            let (name, count) = held_item(inventory, item)?;
            let hp = hp?;
            Some((OneOpArgs::eat(name, item, count, hp), Decision::Eating))
        }
        CureDecision::None | CureDecision::Needed { .. } => None,
    }
}

fn eat_args(
    choice: EatChoice,
    inventory: &[api::snapshot::ItemView],
    hp: i32,
) -> Option<OneOpArgs> {
    let item = choice.ordinary.or(choice.combo)?;
    let (name, count) = held_item(inventory, item)?;
    Some(OneOpArgs::eat(name, item, count, hp))
}

fn held_item(inventory: &[api::snapshot::ItemView], item_id: i32) -> Option<(&str, i32)> {
    let name = inventory
        .iter()
        .find(|row| row.def.id == item_id)?
        .def
        .name
        .as_deref()?;
    let count = inventory
        .iter()
        .filter(|row| row.def.id == item_id)
        .map(|row| row.count)
        .sum::<i32>();
    (count > 0).then_some((name, count))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::combat::tables::PotionKind;
    use crate::gatherer::oneop::OneOpKind;
    use crate::quester::families::tests::{local_player, with_tick};
    use crate::survival::CurePoison;
    use api::condition::{BotCondition, CureDoses, FoodSummary, PoisonState, SessionMark};
    use api::obj_names::ItemDefView;
    use api::snapshot::{
        GameSnapshot, HitmarkView, HitmarksView, ItemActionFamily, ItemContainer, ItemView,
        StatView, WorldStateView, WorldTile,
    };

    fn condition(hp: u8, poison: PoisonState) -> BotCondition {
        BotCondition {
            tick: 20,
            session: SessionMark::Live,
            hp,
            hp_max: 99,
            hp_known: true,
            poison,
            food: FoodSummary::default(),
            cures: CureDoses::default(),
        }
    }

    fn snapshot(inventory: Vec<ItemView>) -> GameSnapshot {
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_world(WorldStateView {
            map_base_x: 3200,
            map_base_z: 3200,
            members: true,
            ..WorldStateView::default()
        });
        snapshot.seed_local_player(local_player(WorldTile {
            x: 3200,
            z: 3200,
            level: 0,
        }));
        snapshot.seed_stats(vec![StatView {
            index: 3,
            name: "hitpoints".into(),
            base: 99,
            effective: 60,
            xp: 0,
            used: true,
        }]);
        snapshot.seed_inventory(inventory, 28);
        snapshot.seed_equipment(vec![]);
        snapshot.seed_npcs(vec![]);
        snapshot.seed_players(vec![]);
        snapshot.seed_projectiles(vec![]);
        snapshot.seed_varps(vec![]);
        snapshot.seed_hitmarks(HitmarksView {
            marks: [HitmarkView {
                value: 0,
                kind: 0,
                cycle: 0,
            }; 4],
            loop_cycle: 0,
        });
        snapshot
    }

    fn held(id: i32, name: &str) -> ItemView {
        ItemView {
            def: ItemDefView {
                id,
                name: Some(name.into()),
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
            count: 1,
            actions: Vec::new(),
            component_id: 0,
        }
    }

    fn dose_id() -> i32 {
        crate::combat::guard::shared_tables()
            .unwrap()
            .potion(PotionKind::Antipoison)
            .unwrap()
            .doses[0]
            .unwrap()
            .id
    }

    fn want(
        step: &mut SurvivalStep,
        snapshot: &GameSnapshot,
        condition: &BotCondition,
        policy: CurePoison,
        tick_number: u64,
    ) -> SurvivalWant {
        let mut ledger = None;
        with_tick(snapshot, &mut ledger, tick_number, |tick| {
            tick.cx.condition = Some(*condition);
            tick.cx.poison_unknown_pulse = 11;
            tick.cx.cure_poison = policy;
            step.want(&tick.cx, 0)
        })
    }

    #[test]
    fn survival_step_parks_on_need_not_on_cure_reason() {
        let condition = condition(
            5,
            PoisonState::Onset {
                since: 20,
                q: Some(11),
                hit_since_line: false,
            },
        );
        let snapshot = snapshot(vec![]);
        let want = want(
            &mut SurvivalStep::default(),
            &snapshot,
            &condition,
            CurePoison::WhenNeeded,
            20,
        );
        assert!(matches!(
            want,
            SurvivalWant::Need(SurvivalNeed::Cure {
                why: NeedWhy::Emergency,
                ..
            })
        ));
    }

    #[test]
    fn survival_step_drink_precedes_emergency_bite() {
        let id = dose_id();
        let condition = condition(
            5,
            PoisonState::Onset {
                since: 20,
                q: Some(11),
                hit_since_line: false,
            },
        );
        let snapshot = snapshot(vec![held(id, "antipoison")]);
        let mut step = SurvivalStep::default();
        let want = want(&mut step, &snapshot, &condition, CurePoison::WhenNeeded, 20);
        let SurvivalWant::Click(args) = want else {
            panic!("a held cure must precede emergency food");
        };
        assert_eq!(
            args.kind,
            OneOpKind::Drink {
                item_id: id,
                before_doses: 1
            }
        );
        assert_eq!(step.report().decision, Decision::Drinking);
    }

    #[test]
    fn survival_step_lethal_waits_for_latch() {
        let mut condition = condition(
            60,
            PoisonState::Onset {
                since: 20,
                q: None,
                hit_since_line: false,
            },
        );
        let snapshot = snapshot(vec![]);
        let mut step = SurvivalStep::default();
        assert!(matches!(
            want(&mut step, &snapshot, &condition, CurePoison::WhenNeeded, 20),
            SurvivalWant::Nothing
        ));
        assert_eq!(step.report().decision, Decision::Needed);

        condition.poison = PoisonState::Poisoned {
            per_tick: 11,
            last_mark: 20,
            hit_since_mark: false,
        };
        assert!(matches!(
            want(&mut step, &snapshot, &condition, CurePoison::WhenNeeded, 21),
            SurvivalWant::Need(SurvivalNeed::Cure {
                why: NeedWhy::Lethal,
                ..
            })
        ));
    }

    #[test]
    fn survival_step_never_policy_reports_only() {
        let id = dose_id();
        let condition = condition(
            5,
            PoisonState::Onset {
                since: 20,
                q: Some(11),
                hit_since_line: false,
            },
        );
        let snapshot = snapshot(vec![held(id, "antipoison")]);
        let mut step = SurvivalStep::default();
        assert!(matches!(
            want(&mut step, &snapshot, &condition, CurePoison::Never, 20),
            SurvivalWant::Nothing
        ));
        assert_eq!(step.report().decision, Decision::Needed);
        assert!(step.report().need.is_some());
        assert!(step.parked().is_none());
    }
    #[test]
    fn survival_need_uses_poison_raised_eat_line_for_food_credit() {
        let mut condition = condition(
            13,
            PoisonState::Poisoned {
                per_tick: 11,
                last_mark: 20,
                hit_since_mark: false,
            },
        );
        condition.hp_max = 20;
        let mut snapshot = snapshot(vec![]);
        snapshot.seed_stats(vec![StatView {
            index: 3,
            name: "hitpoints".into(),
            base: 20,
            effective: 13,
            xp: 0,
            used: true,
        }]);
        // Poison content queues ceil(poison/5) then decrements
        // (`content/scripts/skill_combat/scripts/poison.rs2:20-30`): R0(54)=319.
        // At HP 13, p=11 gives line 12 and cap 8, so ceil((319-13+1)/8)=39.
        assert!(matches!(
            want(
                &mut SurvivalStep::default(),
                &snapshot,
                &condition,
                CurePoison::WhenNeeded,
                0
            ),
            SurvivalWant::Need(SurvivalNeed::Cure {
                or_food: 39,
                why: NeedWhy::Lethal,
                ..
            })
        ));
    }
}
