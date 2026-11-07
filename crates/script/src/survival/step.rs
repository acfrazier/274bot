//! Per-runner survival decisions and the non-terminal survival hold.

use std::fmt;

use api::condition::{poison_math, Exposure, PoisonState};
use api::hostlog::{Category, Emit, Level};

use crate::combat::frame::Frame;
use crate::combat::policy::{self, EatChoice};
use crate::combat::schedule::{InputEffect, OpKind, Schedule};
use crate::combat::select;
use crate::combat::tables::CombatTables;
use crate::combat::threats::ThreatSet;
use crate::gatherer::oneop::{OneOpArgs, OneOpKind};
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
    /// A held click is due, but the engine still delays the player after the
    /// last survival click it ran, so a click now would be dropped
    /// (`OpHeldHandler.ts:16-19`). The runner sends nothing and keeps its
    /// work held this tick.
    Hold,
    Need(SurvivalNeed),
}

/// Observed ticks after a survival click in which its server delays are
/// replayed into the decision schedule. It outlasts every delay that
/// [`Schedule::admitted`] sets for a drink, a Strange fruit or a food (the
/// longest is a food's `message_delay + 2`), and it keeps the 16-bit frame
/// ticks of [`Schedule`] far from their wrap.
const CLICK_REPLAY_TICKS: u64 = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClickKind {
    Drink,
    Fruit,
    Food,
}

/// The last held survival click the runner sent. `frame_tick` is the frame
/// tick the schedule replays it at: the frame the click was sent after, until
/// its op settles on the content's own signal, then the frame before the one
/// that first showed that signal (see [`SurvivalStep::record_settled`]).
/// `observed` holds the low 16 bits of the observed tick that bound the
/// replay. Both are compared by wrapping subtraction, so a reset of either
/// count ends the replay instead of extending it. Revision-289 item ids fit in
/// 16 bits; the record stays 8 bytes so the step keeps its 192-byte bound.
#[derive(Debug, Clone, Copy)]
struct SentClick {
    kind: ClickKind,
    /// True until the runner's op for this click settles or ends.
    awaiting: bool,
    frame_tick: u16,
    observed: u16,
    item_id: u16,
}

/// A runner's compact threat history, park and current report.
#[derive(Default)]
pub struct SurvivalStep {
    threats: ThreatSet,
    sent: Option<SentClick>,
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
        let (schedule, replayed) = self.schedule(tick, low_ticks(cx.evidence().tick), tables);
        if schedule.locked(tick) {
            // The drink's `p_delay(1)` (`consume_effects.rs2:124-126`,
            // `consume_messages.dbrow` `message_delay,1`) keeps the player
            // delayed through the next two server ticks (`PlayerOps.ts:376-378`,
            // `World.ts:692` clears it only after that tick's client input), so
            // the engine would drop any click sent now. Keep the report of the
            // click in flight.
            return SurvivalWant::Hold;
        }
        let danger = self.threats.danger(&frame, tables, tick, false, false);
        let exposure = math::exposure(&self.threats, tables, tick);
        let condition = cx.condition();
        let q_unknown = cx.poison_unknown_pulse();
        let outlook = condition
            .map(|condition| poison_math::remaining(condition.poison, exposure, tick, q_unknown));
        let poison_fire = outlook.map_or(0, |outlook| i32::from(outlook.per_fire_now));
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
        if let Some((args, decision)) = click(cure, frame.inventory) {
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
            let bite = |schedule: &Schedule| {
                policy::eat_choice(
                    &frame,
                    tables,
                    schedule,
                    danger,
                    poison_fire,
                    hp,
                    hp_max,
                    tick,
                    eat_floor,
                )
                .and_then(|choice| eat_args(choice, frame.inventory))
            };
            let timed = bite(&schedule);
            // A bite that only the next-food clock of the last survival click
            // holds back (`consume.rs2:122-127`) is still this tick's work.
            let waiting = timed.is_none()
                && replayed
                && !schedule.ready(OpKind::Eat, tick)
                && bite(&Schedule::default()).is_some();
            if timed.is_some() || waiting {
                self.last = SurvivalReport {
                    decision: Decision::Eating,
                    why,
                    need,
                    tried: 0,
                    owner: Owner::Step,
                };
                return timed.map_or(SurvivalWant::Hold, SurvivalWant::Click);
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

    /// The server delays of the last survival click, replayed at its anchored
    /// frame tick. The runner's own `OneOp` waits for the click's effect, so
    /// the replayed operation is settled at once and only its delays remain.
    /// The second value is true when a click was replayed.
    fn schedule(&mut self, tick: u16, observed: u16, tables: &CombatTables) -> (Schedule, bool) {
        let mut schedule = Schedule::default();
        let Some(sent) = self.sent else {
            return (schedule, false);
        };
        let in_window = u64::from(observed.wrapping_sub(sent.observed)) <= CLICK_REPLAY_TICKS
            && u64::from(tick.wrapping_sub(sent.frame_tick)) <= CLICK_REPLAY_TICKS;
        if !in_window {
            self.sent = None;
            return (schedule, false);
        }
        match sent.kind {
            ClickKind::Drink => schedule.admitted(
                OpKind::Drink,
                sent.frame_tick,
                0,
                false,
                InputEffect::Standard,
            ),
            ClickKind::Fruit => schedule.admitted(
                OpKind::Drink,
                sent.frame_tick,
                0,
                false,
                InputEffect::CureFruit,
            ),
            ClickKind::Food => {
                let Some(food) = tables.food(i32::from(sent.item_id)) else {
                    self.sent = None;
                    return (schedule, false);
                };
                schedule.admitted(
                    OpKind::Eat,
                    sent.frame_tick,
                    0,
                    false,
                    InputEffect::Food(food),
                );
            }
        }
        schedule.settle(OpKind::Drink);
        schedule.settle(OpKind::Eat);
        (schedule, true)
    }

    /// Record a drink, Strange fruit or bite the runner has just sent, so the
    /// next decisions wait for the delays the engine applies to it: the
    /// drink's 3-tick lock (`combat/schedule.rs` `OpKind::Drink`) and a
    /// food's next-food clock (`consume.rs2:122-127`). Until the op settles,
    /// the delays count from the send frame: the engine runs the click in the
    /// next server tick at the earliest. Other operations (closing a page)
    /// leave the clocks unchanged and end the wait for the last click's
    /// effect, because the runner's op is now the other operation.
    pub fn record_sent(&mut self, kind: OneOpKind, cx: &ActionContext<'_>) {
        let (kind, item_id) = match kind {
            OneOpKind::Drink { item_id, .. } => (ClickKind::Drink, item_id),
            OneOpKind::CureFruit { item_id, .. } => (ClickKind::Fruit, item_id),
            OneOpKind::Eat { item_id, .. } => (ClickKind::Food, item_id),
            OneOpKind::Wear { .. }
            | OneOpKind::CloseModal
            | OneOpKind::CloseChat { .. }
            | OneOpKind::ContinueDialog => {
                if let Some(sent) = &mut self.sent {
                    sent.awaiting = false;
                }
                return;
            }
        };
        let (Some(frame), Ok(item_id)) = (Frame::borrow(cx.snapshot()), u16::try_from(item_id))
        else {
            return;
        };
        self.sent = Some(SentClick {
            kind,
            awaiting: true,
            item_id,
            frame_tick: frame.tick,
            observed: low_ticks(cx.evidence().tick),
        });
    }

    /// The runner's op for the last survival click has ended. `applied` is
    /// true when it settled on the content's own signal: the dose, fruit or
    /// food count drop. The consume script makes that change in the server
    /// tick that runs the click (`consume.rs2:122-127` sets `%eat_delay`
    /// there; `consume_effects.rs2:113-126` deletes the dose and calls
    /// `p_delay(1)` there). A Hitpoints rise is never that signal: natural
    /// regeneration also raises Hitpoints (`stat.rs2:58-60`), so `OneOp`
    /// settles a bite on the count drop alone. The first frame that shows
    /// the signal is the update of that server tick, and a click sent after
    /// frame `F` runs in tick `F + 1` at the earliest. So the click's delays
    /// are replayed as if it was sent after the frame before the signal: a
    /// drink run in tick `E` delays the player until `E + 2`
    /// (`PlayerOps.ts:376-379`), and the
    /// engine drops a held click run in `E + 1` or `E + 2`
    /// (`OpHeldHandler.ts:16-19`), so the first click it runs is sent after
    /// frame `E + 2`, which is the replayed lock's end. A signal that arrives
    /// late moves every clock with it. A click whose op ended without the
    /// signal keeps the send-frame clocks.
    pub fn record_settled(&mut self, applied: bool, cx: &ActionContext<'_>) {
        let Some(sent) = self.sent.as_mut().filter(|sent| sent.awaiting) else {
            return;
        };
        sent.awaiting = false;
        if !applied {
            return;
        }
        let Some(frame) = Frame::borrow(cx.snapshot()) else {
            return;
        };
        sent.frame_tick = frame.tick.wrapping_sub(1);
        sent.observed = low_ticks(cx.evidence().tick).wrapping_sub(1);
    }

    /// The runner defers this frame's click to a walk, a fight or a bank
    /// transfer's boundary (design A6.6): report the need, not a click in
    /// progress.
    pub fn defer(&mut self) {
        if matches!(self.last.decision, Decision::Drinking | Decision::Eating) {
            self.last.decision = Decision::Needed;
        }
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
) -> Option<(OneOpArgs, Decision)> {
    match cure {
        CureDecision::Drink { item, .. } => {
            let (name, count) = held_item(inventory, item)?;
            Some((OneOpArgs::drink(name, item, count), Decision::Drinking))
        }
        CureDecision::EatFruit { item, .. } => {
            let (name, count) = held_item(inventory, item)?;
            Some((OneOpArgs::cure_fruit(name, item, count), Decision::Eating))
        }
        CureDecision::None | CureDecision::Needed { .. } => None,
    }
}

fn eat_args(choice: EatChoice, inventory: &[api::snapshot::ItemView]) -> Option<OneOpArgs> {
    let item = choice.ordinary.or(choice.combo)?;
    let (name, count) = held_item(inventory, item)?;
    Some(OneOpArgs::eat(name, item, count))
}

/// The low 16 bits of an observed tick count.
fn low_ticks(tick: u64) -> u16 {
    (tick & u64::from(u16::MAX)) as u16
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
            poison_window: None,
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
        want_with_floor(step, snapshot, condition, policy, tick_number, 0)
    }

    fn want_with_floor(
        step: &mut SurvivalStep,
        snapshot: &GameSnapshot,
        condition: &BotCondition,
        policy: CurePoison,
        tick_number: u64,
        eat_floor: i32,
    ) -> SurvivalWant {
        let mut ledger = None;
        with_tick(snapshot, &mut ledger, tick_number, |tick| {
            tick.cx.condition = Some(*condition);
            tick.cx.poison_unknown_pulse = 11;
            tick.cx.cure_poison = policy;
            step.want(&tick.cx, eat_floor)
        })
    }

    /// The Gatherer's default eat floor for 99 Hitpoints
    /// (`gatherer/runner.rs` `survival_eat_floor`: half the base, rounded up).
    const GATHERER_FLOOR: i32 = 50;

    #[test]
    fn survival_step_parks_on_need_not_on_cure_reason() {
        if !api::test_support::game_data_available() {
            return;
        }
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
        if !api::test_support::game_data_available() {
            return;
        }
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
        if !api::test_support::game_data_available() {
            return;
        }
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

    fn lobster() -> ItemView {
        let id = crate::combat::guard::shared_tables()
            .unwrap()
            .selected()
            .item_by_alias("lobster")
            .unwrap()
            .id;
        held(id, "Lobster")
    }

    fn low_hp_snapshot(inventory: Vec<ItemView>) -> GameSnapshot {
        let mut snapshot = snapshot(inventory);
        snapshot.seed_stats(vec![StatView {
            index: 3,
            name: "hitpoints".into(),
            base: 99,
            effective: 5,
            xp: 0,
            used: true,
        }]);
        snapshot
    }

    fn record(step: &mut SurvivalStep, snapshot: &GameSnapshot, kind: OneOpKind, tick: u64) {
        let mut ledger = None;
        with_tick(snapshot, &mut ledger, tick, |tick| {
            step.record_sent(kind, &tick.cx)
        });
    }

    /// A cure and a bite due together (design A6.4): after the drink the step
    /// holds the bite through the drink's lock (Argon review finding 3). The
    /// drink's `p_delay(1)` delays the player through the next two server
    /// ticks (`consume_effects.rs2:124-126`, `consume_messages.dbrow`
    /// `message_delay,1`, `PlayerOps.ts:376-378`), and the engine drops a
    /// held click while delayed (`OpHeldHandler.ts:16-19`).
    #[test]
    fn survival_step_holds_the_bite_through_the_drink_lock() {
        if !api::test_support::game_data_available() {
            return;
        }
        let id = dose_id();
        let cured = condition(5, PoisonState::Immune { until_lower: 80 });
        let snapshot = low_hp_snapshot(vec![lobster()]);
        let mut step = SurvivalStep::default();
        record(
            &mut step,
            &snapshot,
            OneOpKind::Drink {
                item_id: id,
                before_doses: 1,
            },
            20,
        );
        for tick in 21..=22 {
            assert!(
                matches!(
                    want_with_floor(
                        &mut step,
                        &snapshot,
                        &cured,
                        CurePoison::WhenNeeded,
                        tick,
                        GATHERER_FLOOR
                    ),
                    SurvivalWant::Hold
                ),
                "tick {tick}"
            );
        }
        let SurvivalWant::Click(args) = want_with_floor(
            &mut step,
            &snapshot,
            &cured,
            CurePoison::WhenNeeded,
            23,
            GATHERER_FLOOR,
        ) else {
            panic!("the bite follows the drink lock");
        };
        assert!(matches!(args.kind, OneOpKind::Eat { .. }));
        assert_eq!(step.report().decision, Decision::Eating);
    }

    /// Two bites in a row: the second waits for the server's next-food clock
    /// (`consume.rs2:122-127` returns without eating while
    /// `%eat_delay >= map_clock`), and the step holds the runner meanwhile.
    #[test]
    fn survival_step_holds_a_second_bite_for_the_next_food_clock() {
        if !api::test_support::game_data_available() {
            return;
        }
        let food = lobster();
        let delay = crate::combat::guard::shared_tables()
            .unwrap()
            .food(food.def.id)
            .unwrap()
            .eat_delay_arg
            .expect("lobster uses the shared next-food clock");
        let ready = 20 + u64::try_from(delay).unwrap() + 1;
        let cured = condition(5, PoisonState::Clear);
        let snapshot = low_hp_snapshot(vec![food.clone(), food.clone()]);
        let mut step = SurvivalStep::default();
        record(
            &mut step,
            &snapshot,
            OneOpKind::Eat {
                item_id: food.def.id,
                before_count: 3,
            },
            20,
        );
        for tick in 21..ready {
            assert!(
                matches!(
                    want_with_floor(
                        &mut step,
                        &snapshot,
                        &cured,
                        CurePoison::WhenNeeded,
                        tick,
                        GATHERER_FLOOR
                    ),
                    SurvivalWant::Hold
                ),
                "tick {tick}"
            );
        }
        assert!(matches!(
            want_with_floor(
                &mut step,
                &snapshot,
                &cured,
                CurePoison::WhenNeeded,
                ready,
                GATHERER_FLOOR
            ),
            SurvivalWant::Click(OneOpArgs {
                kind: OneOpKind::Eat { .. },
                ..
            })
        ));
    }

    /// Strange fruit cures poison and heals nothing (`strange_fruit.rs2:1-12`).
    /// Its op settles on the count drop alone with the drink's three-tick
    /// bound, so natural regeneration cannot settle it (Argon review
    /// finding 12).
    #[test]
    fn survival_step_eats_strange_fruit_as_a_count_settled_cure() {
        if !api::test_support::game_data_available() {
            return;
        }
        let fruit = crate::combat::guard::shared_tables()
            .unwrap()
            .selected()
            .item_by_alias("macro_triffidfruit")
            .unwrap()
            .id;
        let poisoned = condition(
            5,
            PoisonState::Onset {
                since: 20,
                q: Some(11),
                hit_since_line: false,
            },
        );
        let snapshot = snapshot(vec![held(fruit, "Strange fruit")]);
        let mut step = SurvivalStep::default();
        let SurvivalWant::Click(args) =
            want(&mut step, &snapshot, &poisoned, CurePoison::WhenNeeded, 20)
        else {
            panic!("a held Strange fruit is a cure");
        };
        assert_eq!(
            args.kind,
            OneOpKind::CureFruit {
                item_id: fruit,
                before_count: 1
            }
        );
        assert_eq!(args.bound_ticks, 3);
    }

    #[test]
    fn survival_step_never_policy_reports_only() {
        if !api::test_support::game_data_available() {
            return;
        }
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
        if !api::test_support::game_data_available() {
            return;
        }
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
        // SURVIVAL-DESIGN-1022/design-final.md:A7:642-643 gives the remaining
        // poison total and the ceiling quantity rule. Content queues
        // ceil(poison/5) then decrements (`content/scripts/skill_combat/scripts/poison.rs2:20-30`):
        // R0(54) = 5*(1+...+10) + 4*11 = 319.
        // With p=11, eat_line=12, cap=20-12=8, and deficit=319-13+1=307.
        // Therefore ceil(307/8)=39 food.
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
