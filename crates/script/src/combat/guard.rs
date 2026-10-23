//! Host-side followed-route safety driver. It holds protection and can eat,
//! but never attacks or flicks. The host native follow site owns one driver per armed route.
use super::arbiter;
use super::frame::Frame;
use super::policy;
use super::prayer::RaisedPrayers;
use super::schedule::{
    elapsed, reached, InputEffect, OpKind, Schedule, EAT_OBSERVATION_WINDOW_TICKS,
};
use super::select;
use super::tables::{CombatTables, PotionKind, PrayerRole};
use super::threats::ThreatSet;
use crate::native::WalkRequest;
use crate::survival::cure::{self, CureDecision, CureInputs, CurePoison};
use api::condition::BotCondition;
use api::game_data::PrayerFact;
use api::selected::ClientRevision;
use api::snapshot::SnapshotView;
use std::sync::{Arc, LazyLock};

pub const MIN_PROTECT_PRAYER_LEVEL: i32 = 37;
const PRAYER_STAT: i32 = 5;
const HITPOINTS_STAT: i32 = 3;

/// Prayer scheduling window, shared with host walk-end debt settlement.
pub const GUARD_PRAYER_WINDOW_TICKS: u16 = 3;
/// Protection prayer the driver wanted but could not raise.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuardProtect {
    Magic,
    Missiles,
    Melee,
}

/// Why the wanted protect could not be raised.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuardFailure {
    PrayerLevel,
    NoPrayerPoints,
}

/// One host-executed decision. A walk hop plus one of these is two user
/// events, never an exclusive combat batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuardOp {
    IfButton {
        component: i32,
    },
    Drink {
        name: Arc<str>,
        kind: PotionKind,
    },
    Eat {
        name: Arc<str>,
    },
    Locked {
        until: u16,
    },
    Unprotectable {
        protect: GuardProtect,
        reason: GuardFailure,
    },
}

/// Why [`WalkGuard::begin`] refused before the route was armed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuardRefusal {
    PrayerTooLow,
    PrayerDisallowed,
    FoodDisallowed,
    Snapshot,
    Tables,
}

/// Passive receipt vocabulary for the runtime escape net. S2b publishes these
/// types; the host owns their execution and lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EscapeAction {
    Retreat,
    Forward,
    Teleport,
    Stand,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EscapeState {
    Running,
    Suspended,
    Done,
    Unresolved,
    Dropped(DropWhy),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EscapeOwner {
    Native,
    Manual,
    Host,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropWhy {
    UserInput,
    Death,
    Reconnect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetireWhy {
    Safe,
    Quiet,
    Bound,
    Stop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Escape {
    pub action: EscapeAction,
    pub state: EscapeState,
    pub owner: EscapeOwner,
    pub allow: crate::native::WalkAllow,
    pub from: api::WorldTile,
    pub to: api::WorldTile,
    pub hp: u8,
    pub floor: u8,
    pub volley: u8,
    pub tick: u32,
    pub generation: u64,
    pub switches: u8,
    pub last_hit: u32,
    pub retired: Option<RetireWhy>,
}

const _: () = assert!(std::mem::size_of::<Escape>() <= 64);

/// Per-followed-route protection and eat driver. Threats plus a few clocks;
/// tables live behind an `Arc`.
pub struct WalkGuard {
    threats: ThreatSet,
    tables: Arc<CombatTables>,
    schedule: Schedule,
    last_tick: u16,
    follow_until: u16,
    drop_component: i32,
    pending_com: i32,
    raised_prayers: RaisedPrayers,
    prayer_admission_tick: u16,
    unprotectable: u8,
    flags: u8,
    drink_points: u8,
    drink_doses: u8,
    drink_kind: PotionKind,
    eat_id: i32,
    eat_count: u16,
    eat_hp: i16,
    eat_admission_tick: u16,
}

const FLAG_FOLLOW: u8 = 1;
const FLAG_SEEN: u8 = 2;
const FLAG_FOOD_ALLOWED: u8 = 4;
const FLAG_NO_POINTS_REPORTED: u8 = 8;
const FLAG_FOOD_ONLY: u8 = 16;
const FLAG_CURE_FRUIT: u8 = 32;

// A11 bounds the cure receipt state to four bytes; keep this shared guard
// owner within its existing 264-byte ceiling.
const _: () = assert!(std::mem::size_of::<WalkGuard>() <= 264);

static TABLES: LazyLock<Option<Arc<CombatTables>>> = LazyLock::new(|| {
    api::game_data::for_revision(ClientRevision::R289)
        .ok()
        .and_then(|data| CombatTables::build(data).ok())
});

/// Shared by the guard and passive admission facts; never builds a second table.
pub fn shared_tables() -> Result<Arc<CombatTables>, GuardRefusal> {
    TABLES.clone().ok_or(GuardRefusal::Tables)
}

fn prayer_base(snapshot: &SnapshotView<'_>) -> Option<i32> {
    snapshot
        .stats()?
        .value
        .iter()
        .find(|row| row.index == PRAYER_STAT)
        .map(|row| row.base)
}

fn food_count(frame: &Frame<'_>, id: i32) -> u16 {
    frame
        .inventory
        .iter()
        .filter(|row| row.def.id == id)
        .fold(0u16, |total, row| {
            total.saturating_add(u16::try_from(row.count.max(0)).unwrap_or(u16::MAX))
        })
}

fn protect_kind(tables: &CombatTables, fact: &PrayerFact) -> Option<GuardProtect> {
    (0..3).find_map(|tier| {
        tables
            .prayer(PrayerRole::Protect, tier)
            .filter(|row| row.varp == fact.varp)
            .map(|_| match tier {
                0 => GuardProtect::Magic,
                1 => GuardProtect::Missiles,
                _ => GuardProtect::Melee,
            })
    })
}

fn kind_bit(kind: GuardProtect) -> u8 {
    match kind {
        GuardProtect::Magic => 1,
        GuardProtect::Missiles => 2,
        GuardProtect::Melee => 4,
    }
}

impl WalkGuard {
    /// Documented begin. Refuses before the route is armed when no protect
    /// is reachable at all.
    pub fn begin(request: &WalkRequest, snapshot: &SnapshotView<'_>) -> Result<Self, GuardRefusal> {
        Self::begin_with(request, snapshot, shared_tables()?)
    }

    /// Host/tests supply the pin's tables explicitly.
    pub fn begin_with(
        request: &WalkRequest,
        snapshot: &SnapshotView<'_>,
        tables: Arc<CombatTables>,
    ) -> Result<Self, GuardRefusal> {
        if !request.allow.prayer {
            return Err(GuardRefusal::PrayerDisallowed);
        }
        let base = prayer_base(snapshot).ok_or(GuardRefusal::Snapshot)?;
        if base < MIN_PROTECT_PRAYER_LEVEL {
            return Err(GuardRefusal::PrayerTooLow);
        }
        Ok(Self::new(tables, request.allow.food, false))
    }

    /// Begin food-only upkeep for a route when protection is not allowed.
    pub fn begin_food_only(
        request: &WalkRequest,
        snapshot: &SnapshotView<'_>,
    ) -> Result<Self, GuardRefusal> {
        Self::begin_food_only_with(request, snapshot, shared_tables()?)
    }

    /// Host/tests supply the pin's tables explicitly.
    pub fn begin_food_only_with(
        request: &WalkRequest,
        _snapshot: &SnapshotView<'_>,
        tables: Arc<CombatTables>,
    ) -> Result<Self, GuardRefusal> {
        if !request.allow.food {
            return Err(GuardRefusal::FoodDisallowed);
        }
        Ok(Self::new(tables, true, true))
    }

    /// Whether any recognized food remains in the observed inventory.
    /// Missing inventory is unknown, not exhausted.
    pub fn has_food(&self, snapshot: &SnapshotView<'_>) -> Option<bool> {
        if self.flags & FLAG_FOOD_ALLOWED == 0 {
            return Some(false);
        }
        let inventory = snapshot.inventory()?;
        Some(
            inventory
                .value
                .iter()
                .any(|row| row.count > 0 && self.tables.food(row.def.id).is_some()),
        )
    }

    /// Prayers this guard has observed being raised by its admitted click.
    pub fn prayer_cleanup(&self) -> RaisedPrayers {
        self.raised_prayers
    }

    fn new(tables: Arc<CombatTables>, food_allowed: bool, food_only: bool) -> Self {
        Self {
            threats: ThreatSet::default(),
            tables,
            schedule: Schedule::default(),
            last_tick: 0,
            follow_until: 0,
            drop_component: 0,
            pending_com: 0,
            raised_prayers: RaisedPrayers::default(),
            prayer_admission_tick: 0,
            unprotectable: 0,
            flags: if food_allowed { FLAG_FOOD_ALLOWED } else { 0 }
                | if food_only { FLAG_FOOD_ONLY } else { 0 },
            drink_points: 0,
            drink_doses: 0,
            drink_kind: PotionKind::Prayer,
            eat_id: 0,
            eat_count: 0,
            eat_hp: 0,
            eat_admission_tick: 0,
        }
    }

    /// Previous drink lock or protect click still covers `tick`: the route
    /// owner issues no follow click.
    pub fn blocks_follow(&self, tick: u16) -> bool {
        (self.flags & FLAG_FOLLOW != 0 && !reached(tick, self.follow_until))
            || self.schedule.locked(tick)
    }

    /// Refresh observed-on state and settle a pending click only after its
    /// requested protection is observed.
    pub fn observe_prayer(&mut self, snapshot: &SnapshotView<'_>) {
        let Some(frame) = Frame::borrow(*snapshot) else {
            return;
        };
        self.observe_protect(&frame);
    }

    fn observe_protect(&mut self, frame: &Frame<'_>) {
        // A late on observation may belong to a user, not the admitted click.
        // Expire even without a wanted threat, before adopting any protect.
        if self
            .pending_protect_tick()
            .is_some_and(|tick| elapsed(frame.tick, tick) > GUARD_PRAYER_WINDOW_TICKS)
        {
            self.schedule.timeout(OpKind::Prayer);
        }
        if let Some(fact) = select::active_protect(frame, &self.tables)
            .and_then(|style| select::protect_fact(&self.tables, style))
        {
            if self.pending_protect() == Some(fact.button_com) {
                self.schedule.settle(OpKind::Prayer);
                self.drop_component = fact.button_com;
                self.raised_prayers = RaisedPrayers::default();
                self.raised_prayers.accepted(fact.varp, true, 0);
            } else if !self.raised_prayers.contains(fact.varp) {
                self.drop_component = 0;
                self.raised_prayers = RaisedPrayers::default();
            }
        } else {
            self.drop_component = 0;
            self.raised_prayers = RaisedPrayers::default();
        }
    }

    /// Successfully sent protection click that has not yet been observed on.
    pub fn pending_protect(&self) -> Option<i32> {
        (self.schedule.pending(OpKind::Prayer) && self.pending_com > 0).then_some(self.pending_com)
    }
    /// Schedule tick of the admitted protect while it is awaiting observation.
    pub fn pending_protect_tick(&self) -> Option<u16> {
        self.pending_protect().map(|_| self.prayer_admission_tick)
    }

    /// Commit a proposal only after its host interaction was successfully sent.
    pub fn admitted(&mut self, op: &GuardOp, snapshot: &SnapshotView<'_>) {
        let Some(frame) = Frame::borrow(*snapshot) else {
            return;
        };
        let tick = frame.tick;
        match op {
            GuardOp::IfButton { component }
                if *component > 0
                    && (0..3).any(|tier| {
                        self.tables
                            .prayer(PrayerRole::Protect, tier)
                            .is_some_and(|fact| fact.button_com == *component)
                    }) =>
            {
                self.schedule
                    .admitted(OpKind::Prayer, tick, 0, false, InputEffect::PrayerOn);
                self.pending_com = *component;
                self.prayer_admission_tick = tick;
                self.follow_until = tick.wrapping_add(1);
                self.flags |= FLAG_FOLLOW;
            }
            GuardOp::Drink { kind, .. } => {
                let (points, _) = arbiter::stat(&frame, PRAYER_STAT);
                self.drink_points =
                    u8::try_from(points.clamp(0, i32::from(u8::MAX))).unwrap_or(u8::MAX);
                self.drink_doses = u8::try_from(
                    arbiter::doses(&frame, &self.tables, *kind).clamp(0, i16::from(u8::MAX)),
                )
                .unwrap_or(u8::MAX);
                self.drink_kind = *kind;
                self.flags &= !FLAG_CURE_FRUIT;
                self.schedule
                    .admitted(OpKind::Drink, tick, 0, false, InputEffect::Standard);
            }
            GuardOp::Eat { name } => {
                let Some(row) = frame.inventory.iter().find(|row| {
                    row.def
                        .name
                        .as_deref()
                        .is_some_and(|got| got.eq_ignore_ascii_case(name.as_ref()))
                }) else {
                    return;
                };
                let id = row.def.id;
                if let Some(food) = self.tables.food(id) {
                    let (hp, _) = arbiter::stat(&frame, HITPOINTS_STAT);
                    self.schedule
                        .admitted(OpKind::Eat, tick, 0, false, InputEffect::Food(food));
                    self.eat_admission_tick = tick;
                    self.eat_id = id;
                    self.eat_count = food_count(&frame, id);
                    self.eat_hp = hp.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16;
                } else if self
                    .tables
                    .selected()
                    .item_by_alias("macro_triffidfruit")
                    .is_some_and(|fruit| fruit.id == id)
                {
                    self.schedule
                        .admitted(OpKind::Drink, tick, 0, false, InputEffect::Standard);
                    self.flags |= FLAG_CURE_FRUIT;
                    self.eat_admission_tick = tick;
                    self.eat_id = id;
                    self.eat_count = food_count(&frame, id);
                }
            }
            _ => {}
        }
    }

    /// Observe one tick with the slot-owned condition and current cure policy.
    /// Commit a sent proposal with [`Self::admitted`]. An absent condition adds
    /// no poison fire to the eat line.
    pub fn tick(
        &mut self,
        snapshot: &SnapshotView<'_>,
        condition: Option<&BotCondition>,
        cure_policy: CurePoison,
    ) -> Option<GuardOp> {
        let frame = Frame::borrow(*snapshot)?;
        let tick = frame.tick;
        if self.flags & FLAG_SEEN != 0 && tick == self.last_tick {
            return None;
        }
        self.flags |= FLAG_SEEN;
        self.last_tick = tick;
        self.threats.observe(&frame, &self.tables, tick);
        let (hp, hp_max) = arbiter::stat(&frame, HITPOINTS_STAT);
        self.settle_eat(&frame, tick, hp);
        if self.blocks_follow(tick) {
            return Some(GuardOp::Locked {
                until: if self.schedule.locked(tick) {
                    self.schedule.input_lock
                } else {
                    self.follow_until
                },
            });
        }
        let (points, base) = arbiter::stat(&frame, PRAYER_STAT);
        if self.flags & FLAG_FOOD_ONLY == 0 {
            self.observe_protect(&frame);
        }
        self.settle_drink(&frame, tick, points);
        let danger = self
            .threats
            .danger(&frame, &self.tables, tick, false, false);
        if let Some(op) = self.cure_op(&frame, condition, cure_policy, danger, tick) {
            return Some(op);
        }
        let poison_fire =
            crate::survival::math::poison_fire(condition, &self.threats, &self.tables, tick);
        let eat_choice = if self.flags & FLAG_FOOD_ALLOWED != 0
            && !self.schedule.pending(OpKind::Eat)
            && self.schedule.unsettled[OpKind::Eat.index()] < 3
        {
            policy::eat_choice(
                &frame,
                &self.tables,
                &self.schedule,
                danger,
                poison_fire,
                hp,
                hp_max,
                tick,
            )
        } else {
            None
        };
        if (danger != Some(0) || poison_fire > 0)
            && hp
                <= select::lines(danger, hp_max)
                    .with_poison(poison_fire)
                    .emergency
        {
            if let Some(eat) = self.eat_op(&frame, eat_choice) {
                return Some(eat);
            }
        }
        if self.flags & FLAG_FOOD_ONLY != 0 {
            return self.eat_op(&frame, eat_choice);
        }
        let Some(wanted) =
            policy::wanted_protect(&self.threats, &frame, &self.tables, tick, false, false)
        else {
            return self.eat_op(&frame, eat_choice);
        };
        let Some(kind) = protect_kind(&self.tables, wanted) else {
            return self.eat_op(&frame, eat_choice);
        };
        if points == 0
            && !self.schedule.pending(OpKind::Drink)
            && arbiter::potion_id(&frame, &self.tables, PotionKind::Prayer).is_none()
            && self.flags & FLAG_NO_POINTS_REPORTED == 0
        {
            self.flags |= FLAG_NO_POINTS_REPORTED;
            return Some(GuardOp::Unprotectable {
                protect: kind,
                reason: GuardFailure::NoPrayerPoints,
            });
        }
        if base < wanted.level {
            let bit = kind_bit(kind);
            if self.unprotectable & bit != 0 {
                return self.eat_op(&frame, eat_choice);
            }
            self.unprotectable |= bit;
            return Some(GuardOp::Unprotectable {
                protect: kind,
                reason: GuardFailure::PrayerLevel,
            });
        }
        if select::prayer_on(&frame, wanted.varp) {
            return self
                .maybe_floor_sip(&frame, tick, points, base, hp, hp_max)
                .or_else(|| self.eat_op(&frame, eat_choice));
        }
        let same_toggle = self.pending_com == wanted.button_com;
        if self.schedule.pending(OpKind::Prayer) && same_toggle {
            if !self.schedule.ready(OpKind::Prayer, tick) {
                return self.eat_op(&frame, eat_choice);
            }
            self.schedule.timeout(OpKind::Prayer);
        }
        if !self.schedule.ready(OpKind::Prayer, tick) && same_toggle {
            return self.eat_op(&frame, eat_choice);
        }
        if points == 0 {
            return self
                .drink(&frame, tick, hp, hp_max, true)
                .or_else(|| self.eat_op(&frame, eat_choice));
        }
        Some(GuardOp::IfButton {
            component: wanted.button_com,
        })
    }
    fn cure_op(
        &self,
        frame: &Frame<'_>,
        condition: Option<&BotCondition>,
        policy: CurePoison,
        danger: Option<i32>,
        tick: u16,
    ) -> Option<GuardOp> {
        let decision = cure::cure_choice(&CureInputs {
            cond: condition,
            danger,
            exposure: crate::survival::math::exposure(&self.threats, &self.tables, tick),
            policy,
            route_refused_for_poison: false,
            inventory: frame.inventory,
            tables: &self.tables,
            schedule: &self.schedule,
            clue_reserve: false,
            tick,
            q_unknown: self.tables.unknown_poison_pulse(),
            eat_floor: 0,
        });
        match decision {
            CureDecision::Drink { item, kind, .. } => {
                let name = item_name(frame, item)?;
                Some(GuardOp::Drink { name, kind })
            }
            CureDecision::EatFruit { item, .. } => {
                let name = item_name(frame, item)?;
                Some(GuardOp::Eat { name })
            }
            CureDecision::None | CureDecision::Needed { .. } => None,
        }
    }
    /// Whether the current observed HP exceeds the guard's food line for its
    /// current live danger estimate. Unknown danger is never considered safe.
    pub fn safe_to_stop(&self, snapshot: &SnapshotView<'_>) -> bool {
        let Some(frame) = Frame::borrow(*snapshot) else {
            return false;
        };
        let (hp, hp_max) = arbiter::stat(&frame, HITPOINTS_STAT);
        self.threats
            .danger(&frame, &self.tables, frame.tick, false, false)
            .is_some_and(|danger| hp > select::lines(Some(danger), hp_max).eat)
    }

    /// Retire the admitted protect and, while a switch is pending, the distinct
    /// prior style this guard already owned. The host settles both by observation.
    pub fn end(self) -> (GuardOp, Option<i32>) {
        let pending = self.pending_protect();
        let owned = (!self.raised_prayers.is_empty() && self.drop_component > 0)
            .then_some(self.drop_component);
        let component = pending.or(owned).unwrap_or(0);
        let fallback = pending.and_then(|pending| owned.filter(|owned| *owned != pending));
        (GuardOp::IfButton { component }, fallback)
    }

    fn maybe_floor_sip(
        &mut self,
        frame: &Frame<'_>,
        tick: u16,
        points: i32,
        base: i32,
        hp: i32,
        hp_max: i32,
    ) -> Option<GuardOp> {
        if !policy::prayer_sip_due(points, base) {
            return None;
        }
        self.drink(frame, tick, hp, hp_max, true)
    }

    fn drink(
        &mut self,
        frame: &Frame<'_>,
        tick: u16,
        hp: i32,
        hp_max: i32,
        gated: bool,
    ) -> Option<GuardOp> {
        if self.schedule.pending(OpKind::Drink) || !self.schedule.ready(OpKind::Drink, tick) {
            return None;
        }
        if gated {
            let danger = self.threats.danger(frame, &self.tables, tick, false, false);
            if hp <= select::lines(danger, hp_max).drink_gate {
                return None;
            }
        }
        let id = arbiter::potion_id(frame, &self.tables, PotionKind::Prayer)?;
        let name = frame
            .inventory
            .iter()
            .find(|row| row.def.id == id)
            .and_then(|row| row.def.name.as_deref())
            .map(Arc::<str>::from)?;
        Some(GuardOp::Drink {
            name,
            kind: PotionKind::Prayer,
        })
    }

    fn eat_op(&self, frame: &Frame<'_>, choice: Option<policy::EatChoice>) -> Option<GuardOp> {
        let choice = choice?;
        let id = choice.ordinary.or(choice.combo)?;
        let name = frame
            .inventory
            .iter()
            .find(|row| row.def.id == id)
            .and_then(|row| row.def.name.as_deref())
            .map(Arc::<str>::from)?;
        Some(GuardOp::Eat { name })
    }

    fn settle_eat(&mut self, frame: &Frame<'_>, tick: u16, hp: i32) {
        if !self.schedule.pending(OpKind::Eat) {
            return;
        }
        let observed = food_count(frame, self.eat_id) < self.eat_count
            || (hp.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16) > self.eat_hp;
        if observed {
            self.schedule.settle(OpKind::Eat);
            self.eat_id = 0;
            self.eat_count = 0;
            self.eat_hp = 0;
            self.eat_admission_tick = 0;
        } else if elapsed(tick, self.eat_admission_tick) >= u16::from(EAT_OBSERVATION_WINDOW_TICKS)
        {
            self.schedule.timeout(OpKind::Eat);
            self.eat_id = 0;
            self.eat_count = 0;
            self.eat_hp = 0;
            self.eat_admission_tick = 0;
        }
    }

    fn settle_drink(&mut self, frame: &Frame<'_>, tick: u16, points: i32) {
        if !self.schedule.pending(OpKind::Drink) {
            return;
        }
        if self.flags & FLAG_CURE_FRUIT != 0 {
            let observed = food_count(frame, self.eat_id) < self.eat_count;
            if observed {
                self.schedule.settle(OpKind::Drink);
                self.flags &= !FLAG_CURE_FRUIT;
                self.eat_id = 0;
                self.eat_count = 0;
                self.eat_admission_tick = 0;
            } else if self.schedule.ready(OpKind::Drink, tick) {
                self.schedule.timeout(OpKind::Drink);
                self.flags &= !FLAG_CURE_FRUIT;
                self.eat_id = 0;
                self.eat_count = 0;
                self.eat_admission_tick = 0;
            }
            return;
        }
        let doses = arbiter::doses(frame, &self.tables, self.drink_kind);
        let observed = (self.drink_kind == PotionKind::Prayer
            && points > i32::from(self.drink_points))
            || doses < i16::from(self.drink_doses);
        if observed {
            self.schedule.settle(OpKind::Drink);
        } else if self.schedule.ready(OpKind::Drink, tick) {
            self.schedule.timeout(OpKind::Drink);
        }
    }
}

fn item_name(frame: &Frame<'_>, id: i32) -> Option<Arc<str>> {
    frame
        .inventory
        .iter()
        .find(|row| row.def.id == id)
        .and_then(|row| row.def.name.as_deref())
        .map(Arc::<str>::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::quest_progress::EvidenceStamp;
    use api::selected::RunKey;
    use api::snapshot::{
        ActorKind, ActorTargetView, ActorView, GameSnapshot, HitmarkView, HitmarksView, ItemView,
        LocalPlayerView, NpcView, PlayerView, ProjectileView, StatView, VarpView, WorldStateView,
        WorldTile,
    };
    use api::{obj_names::ItemDefView, snapshot::ItemActionFamily, snapshot::ItemContainer};

    fn dummy_stamp(tick: u64) -> EvidenceStamp {
        EvidenceStamp {
            run: RunKey {
                slot: 0,
                run: 0,
                session: 0,
            },
            tick,
            sequence: 0,
        }
    }

    fn tile(x: i32, z: i32) -> WorldTile {
        WorldTile { x, z, level: 0 }
    }

    fn actor(at: WorldTile) -> ActorView {
        ActorView {
            name: None,
            actions: Vec::new(),
            tile: at,
            distance: 0,
            animation: -1,
            animation_frame: -1,
            pose_animation: -1,
            orientation: 0,
            target_orientation: 0,
            overhead_text: None,
            spot_animation: -1,
            spot_animation_stamp: -1,
            health: 40,
            total_health: 40,
            face_entity: -1,
            target: None,
            moving: false,
            running: false,
            in_combat: false,
        }
    }

    struct Scene {
        tables: Arc<CombatTables>,
        snapshot: GameSnapshot,
        local: LocalPlayerView,
        npcs: Vec<NpcView>,
        stats: Vec<StatView>,
        varps: Vec<VarpView>,
        inventory: Vec<ItemView>,
        projectiles: Vec<ProjectileView>,
        condition: Option<BotCondition>,
    }

    impl Scene {
        fn new(prayer_base: i32) -> Self {
            let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
            let tables = CombatTables::build(Arc::clone(&data)).unwrap();
            let row = data.npc_by_config("ardougne_archer").unwrap();
            let at = tile(2600, 3200);
            let npc = NpcView {
                index: 7,
                r#type: Some(row.id as usize),
                name: row.display.clone(),
                actions: vec![Some("Attack".into())],
                tile: tile(at.x + 1, at.z),
                distance: 1,
                animation: -1,
                animation_frame: -1,
                pose_animation: -1,
                orientation: 0,
                target_orientation: 0,
                overhead_text: None,
                spot_animation: -1,
                spot_animation_stamp: -1,
                health: 50,
                total_health: 50,
                face_entity: -1,
                target: None,
                moving: false,
                running: false,
                in_combat: false,
                level: 37,
                size: 1,
                network: tile(at.x + 1, at.z),
                x: 0,
                z: 0,
                yaw: 0,
            };
            let varps = data
                .prayers()
                .iter()
                .map(|row| VarpView {
                    index: row.varp,
                    value: 0,
                })
                .collect();
            let mut scene = Self {
                tables,
                snapshot: GameSnapshot::new(),
                npcs: vec![npc],
                local: LocalPlayerView {
                    player: PlayerView {
                        index: 1,
                        network: at,
                        actor: actor(at),
                        combat_level: 60,
                        skill_level: 0,
                        headicons: 0,
                        weapon: None,
                    },
                    energy: 100,
                    weight: 0,
                    animation_update: None,
                },
                stats: (0..25)
                    .map(|index| StatView {
                        index,
                        name: String::new(),
                        effective: if index == 5 { prayer_base } else { 40 },
                        base: if index == 5 { prayer_base } else { 40 },
                        xp: 0,
                        used: api::snapshot::stat_used(index as usize),
                    })
                    .collect(),
                varps,
                inventory: Vec::new(),
                projectiles: Vec::new(),
                condition: None,
            };
            scene.refresh();
            scene
        }

        fn refresh(&mut self) {
            self.snapshot.seed_ingame(2);
            self.snapshot.seed_world(WorldStateView {
                map_base_x: 2560,
                map_base_z: 3160,
                members: true,
                ..WorldStateView::default()
            });
            self.snapshot.seed_stats(self.stats.clone());
            self.snapshot.seed_varps(self.varps.clone());
            self.snapshot.seed_inventory(self.inventory.clone(), 28);
            self.snapshot.seed_equipment(Vec::new());
            self.snapshot.seed_npcs(self.npcs.clone());
            self.snapshot.seed_players(Vec::new());
            self.snapshot.seed_projectiles(self.projectiles.clone());
            self.snapshot.seed_local_player(self.local.clone());
            self.snapshot.seed_hitmarks(HitmarksView {
                marks: [HitmarkView {
                    value: 0,
                    kind: 0,
                    cycle: 0,
                }; 4],
                loop_cycle: 0,
            });
        }

        fn view(&self) -> SnapshotView<'_> {
            self.view_at(0)
        }

        fn view_at(&self, tick: u16) -> SnapshotView<'_> {
            SnapshotView::new(Some(&self.snapshot), dummy_stamp(u64::from(tick)))
        }

        fn condition(&self) -> Option<&BotCondition> {
            self.condition.as_ref()
        }

        fn set_poisoned(&mut self, per_tick: u8) {
            self.condition = Some(BotCondition {
                poison: api::condition::PoisonState::Poisoned {
                    per_tick,
                    last_mark: 0,
                    hit_since_mark: false,
                },
                ..BotCondition::default()
            });
        }

        fn request(&self) -> WalkRequest {
            WalkRequest {
                target: tile(2610, 3200),
                candidate_goals: None,
                loc_id: None,
                radius: 1,
                arrival: nav::contract::ArrivalKind::Reach,
                options: crate::native::WalkOptions::default(),
                required_after: dummy_stamp(0),
                evidence: None,
                cross: Box::default(),
                protect: true,
                food_guard: false,
                allow: Default::default(),
            }
        }

        fn begin(&self) -> Result<WalkGuard, GuardRefusal> {
            WalkGuard::begin_with(&self.request(), &self.view(), Arc::clone(&self.tables))
        }

        fn face_us(&mut self) {
            self.npcs[0].target = Some(ActorTargetView {
                kind: ActorKind::Player,
                index: 1,
            });
            self.npcs[0].in_combat = true;
            self.refresh();
        }

        fn launch_known_melee_threat(&mut self) {
            let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
            let row = data
                .npc_names()
                .unwrap()
                .rows
                .iter()
                .find(|row| {
                    row.attack_kind.is_none()
                        && super::super::select::facts::npc_max_hit(row).is_some_and(|hit| hit > 0)
                        && !row.bespoke
                })
                .expect("selected content includes a known-damage melee NPC");
            self.npcs[0].r#type = Some(row.id as usize);
            self.npcs[0].name = row.display.clone();
            self.face_us();
        }

        fn launch_style(&mut self, spotanim: i32) {
            self.face_us();
            self.projectiles = vec![ProjectileView {
                spotanim,
                level: 0,
                src: self.npcs[0].tile,
                target: Some(ActorTargetView {
                    kind: ActorKind::Player,
                    index: 1,
                }),
                t1: 0,
                t2: 30,
            }];
            self.refresh();
        }

        fn launch_arrow(&mut self) {
            self.launch_style(9);
        }

        fn launch_spell(&mut self) {
            self.launch_style(88);
        }

        fn magic(&self) -> &PrayerFact {
            self.tables.prayer(PrayerRole::Protect, 0).unwrap()
        }

        fn missiles(&self) -> &PrayerFact {
            self.tables.prayer(PrayerRole::Protect, 1).unwrap()
        }

        fn set_protect_varp(&mut self, varp: i32, on: bool) {
            if let Some(row) = self.varps.iter_mut().find(|row| row.index == varp) {
                row.value = i32::from(on);
            }
            self.refresh();
        }

        fn set_missiles(&mut self, on: bool) {
            let varp = self.missiles().varp;
            self.set_protect_varp(varp, on);
        }

        fn add_prayer_potion(&mut self) {
            let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
            let item = data.item_by_alias("4doseprayerrestore").unwrap();
            self.inventory.push(ItemView {
                def: ItemDefView {
                    id: item.id,
                    name: item.name.clone(),
                    stackable: false,
                    members: false,
                    base_value: 1,
                    noted: false,
                    certificate_link: -1,
                    certificate_template: -1,
                },
                container: ItemContainer::Inventory,
                action_family: ItemActionFamily::Held,
                slot: 0,
                count: 1,
                actions: vec![Some("Drink".into())],
                component_id: 3214,
            });
            self.refresh();
        }
        fn add_antipoison(&mut self) {
            let id = self.tables.potion(PotionKind::Antipoison).unwrap().doses[0]
                .unwrap()
                .id;
            self.inventory.push(ItemView {
                def: ItemDefView {
                    id,
                    name: Some("Antipoison".into()),
                    stackable: false,
                    members: true,
                    base_value: 1,
                    noted: false,
                    certificate_link: -1,
                    certificate_template: -1,
                },
                container: ItemContainer::Inventory,
                action_family: ItemActionFamily::Held,
                slot: self.inventory.len() as i32,
                count: 1,
                actions: vec![Some("Drink".into())],
                component_id: 3214,
            });
            self.refresh();
        }
        fn add_food(&mut self, alias: &str, slot: i32, count: i32) {
            let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
            let item = data.item_by_alias(alias).unwrap();
            self.inventory.push(ItemView {
                def: ItemDefView {
                    id: item.id,
                    name: item.name.clone(),
                    stackable: false,
                    members: false,
                    base_value: 1,
                    noted: false,
                    certificate_link: -1,
                    certificate_template: -1,
                },
                container: ItemContainer::Inventory,
                action_family: ItemActionFamily::Held,
                slot,
                count,
                actions: vec![Some("Eat".into())],
                component_id: 3214,
            });
            self.refresh();
        }

        fn launch_unknown(&mut self) {
            self.face_us();
            self.projectiles = vec![ProjectileView {
                spotanim: i32::MAX,
                level: 0,
                src: self.npcs[0].tile,
                target: Some(ActorTargetView {
                    kind: ActorKind::Player,
                    index: 1,
                }),
                t1: 0,
                t2: 30,
            }];
            self.refresh();
        }

        fn launch_troll_rock(&mut self) {
            let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
            let thrower = data.npc_by_config("death_troll_thrower1").unwrap();
            self.npcs[0].r#type = Some(thrower.id as usize);
            self.npcs[0].name = thrower.display.clone();
            self.npcs[0].animation = 1142;
            self.npcs[0].tile = self.local.player.actor.tile;
            self.npcs[0].network = self.local.player.actor.tile;
            self.launch_style(276);
            self.projectiles[0].t1 = 364;
            self.projectiles[0].t2 = 381;
            self.refresh();
            self.snapshot.seed_hitmarks(HitmarksView {
                marks: [HitmarkView {
                    value: 0,
                    kind: 0,
                    cycle: 0,
                }; 4],
                loop_cycle: 364,
            });
        }
    }

    #[test]
    fn driver_fits_the_per_route_budget() {
        assert!(std::mem::size_of::<WalkGuard>() <= 264);
    }

    #[test]
    fn begin_refuses_when_no_protect_is_reachable() {
        let scene = Scene::new(36);
        assert_eq!(scene.begin().err(), Some(GuardRefusal::PrayerTooLow));
    }

    #[test]
    fn begin_admits_the_lowest_protect() {
        let scene = Scene::new(37);
        assert!(scene.begin().is_ok());
    }

    #[test]
    fn begin_does_not_own_a_preexisting_protect_for_immediate_end() {
        let mut scene = Scene::new(43);
        let melee = scene.tables.prayer(PrayerRole::Protect, 2).unwrap();
        scene.set_protect_varp(melee.varp, true);
        let guard = scene.begin().unwrap();
        assert_eq!(guard.end().0, GuardOp::IfButton { component: 0 });
    }

    #[test]
    fn an_externally_observed_protect_is_not_owned_without_guard_admission() {
        let mut scene = Scene::new(43);
        let missiles_varp = scene.missiles().varp;
        let mut guard = scene.begin().unwrap();
        scene.set_protect_varp(missiles_varp, true);
        guard.observe_prayer(&scene.view_at(0));
        assert_eq!(guard.end().0, GuardOp::IfButton { component: 0 });
    }

    #[test]
    fn a_preexisting_desired_protect_stays_user_owned_after_guard_observation() {
        let mut scene = Scene::new(43);
        scene.launch_arrow();
        scene.set_missiles(true);
        let mut guard = scene.begin().unwrap();
        assert_eq!(guard.tick(&scene.view_at(0), scene.condition(), CurePoison::WhenNeeded), None);
        assert_eq!(guard.end().0, GuardOp::IfButton { component: 0 });
    }

    #[test]
    fn a_protect_observed_after_the_admission_window_is_not_owned() {
        let mut scene = Scene::new(43);
        scene.launch_arrow();
        let mut guard = scene.begin().unwrap();
        let click = guard.tick(&scene.view_at(0), scene.condition(), crate::survival::CurePoison::WhenNeeded).unwrap();
        guard.admitted(&click, &scene.view_at(0));
        scene.set_missiles(true);
        guard.observe_prayer(&scene.view_at(4));
        assert_eq!(guard.pending_protect(), None);
        assert_eq!(
            guard.end().0,
            GuardOp::IfButton { component: 0 },
            "an expired dropped enable cannot adopt a later user's protect"
        );
    }

    #[test]
    fn missiles_above_prayer_base_are_unprotectable_and_send_no_click() {
        let mut scene = Scene::new(37);
        scene.launch_arrow();
        let mut guard = scene.begin().unwrap();
        match guard.tick(&scene.view(), scene.condition(), crate::survival::CurePoison::WhenNeeded) {
            Some(GuardOp::Unprotectable {
                protect: GuardProtect::Missiles,
                reason: GuardFailure::PrayerLevel,
            }) => {}
            other => panic!("wanted unprotectable missiles, got {other:?}"),
        }
        assert!(
            !matches!(
                guard.tick(&scene.view(), scene.condition(), CurePoison::WhenNeeded),
                Some(GuardOp::IfButton { .. })
            ),
            "unprotectable must not emit a prayer packet"
        );
    }

    #[test]
    fn prayer_level_failure_is_once_per_style_without_blocking_other_protects() {
        let mut scene = Scene::new(37);
        scene.launch_arrow();
        let mut guard = scene.begin().unwrap();
        assert_eq!(
            guard.tick(&scene.view_at(0), scene.condition(), CurePoison::WhenNeeded),
            Some(GuardOp::Unprotectable {
                protect: GuardProtect::Missiles,
                reason: GuardFailure::PrayerLevel,
            })
        );

        scene.launch_spell();
        let magic_component = scene.magic().button_com;
        let magic = guard.tick(&scene.view_at(1), scene.condition(), crate::survival::CurePoison::WhenNeeded).unwrap();
        assert_eq!(
            magic,
            GuardOp::IfButton {
                component: magic_component
            }
        );
        guard.admitted(&magic, &scene.view_at(1));

        scene.launch_arrow();
        assert_eq!(
            guard.tick(&scene.view_at(4), scene.condition(), CurePoison::WhenNeeded),
            None,
            "returning to the already-reported level shortfall must not emit it again"
        );
    }

    #[test]
    fn no_prayer_points_without_a_potion_reports_once_for_the_walk() {
        let mut scene = Scene::new(43);
        scene.stats[5].effective = 0;
        scene.launch_arrow();
        let mut guard = scene.begin().unwrap();
        assert_eq!(
            guard.tick(&scene.view_at(0), scene.condition(), CurePoison::WhenNeeded),
            Some(GuardOp::Unprotectable {
                protect: GuardProtect::Missiles,
                reason: GuardFailure::NoPrayerPoints,
            })
        );
        assert_eq!(
            guard.tick(&scene.view_at(1), scene.condition(), CurePoison::WhenNeeded),
            None,
            "the owner warning is once-only while the walk continues"
        );
    }

    #[test]
    fn missiles_protect_clicks_within_two_ticks_of_the_launch() {
        let mut scene = Scene::new(43);
        scene.launch_arrow();
        let mut guard = scene.begin().unwrap();
        let op = guard.tick(&scene.view_at(0), scene.condition(), crate::survival::CurePoison::WhenNeeded);
        let missiles = scene.missiles();
        assert_eq!(
            op,
            Some(GuardOp::IfButton {
                component: missiles.button_com
            })
        );
        assert!(
            !guard.blocks_follow(0),
            "a proposed click cannot block a walk hop"
        );
        assert_eq!(guard.pending_protect(), None);
        guard.admitted(op.as_ref().unwrap(), &scene.view_at(0));
        assert!(guard.blocks_follow(0));
        assert!(!guard.blocks_follow(1));
        assert_eq!(
            guard.pending_protect(),
            Some(missiles.button_com),
            "only a successfully admitted click becomes pending"
        );
    }

    #[test]
    fn begin_refuses_when_the_owner_disallows_prayer() {
        let scene = Scene::new(43);
        let mut request = scene.request();
        request.allow.prayer = false;
        assert_eq!(
            WalkGuard::begin_with(&request, &scene.view(), Arc::clone(&scene.tables)).err(),
            Some(GuardRefusal::PrayerDisallowed)
        );
    }

    #[test]
    fn pending_protect_click_is_not_repeated_before_ack() {
        let mut scene = Scene::new(43);
        scene.launch_arrow();
        let mut guard = scene.begin().unwrap();
        let first = guard.tick(&scene.view_at(0), scene.condition(), crate::survival::CurePoison::WhenNeeded).unwrap();
        assert!(matches!(first, GuardOp::IfButton { .. }), "{first:?}");
        assert_eq!(guard.pending_protect(), None);
        assert_eq!(guard.pending_protect_tick(), None);
        guard.admitted(&first, &scene.view_at(0));
        assert_eq!(guard.pending_protect(), Some(scene.missiles().button_com));
        assert_eq!(guard.pending_protect_tick(), Some(0));
        assert!(
            !matches!(
                guard.tick(&scene.view_at(1), scene.condition(), CurePoison::WhenNeeded),
                Some(GuardOp::IfButton { .. })
            ),
            "an admitted but unacknowledged protect must not toggle again"
        );
        assert_eq!(guard.pending_protect_tick(), Some(0));
        assert!(
            !matches!(
                guard.tick(&scene.view_at(2), scene.condition(), CurePoison::WhenNeeded),
                Some(GuardOp::IfButton { .. })
            ),
            "three-tick prayer pacing must hold after the first successful send"
        );
    }

    #[test]
    fn refused_protect_admission_retries_without_consuming_pacing() {
        let mut scene = Scene::new(43);
        scene.launch_arrow();
        let mut guard = scene.begin().unwrap();
        let first = guard.tick(&scene.view_at(0), scene.condition(), crate::survival::CurePoison::WhenNeeded).unwrap();
        assert!(matches!(first, GuardOp::IfButton { .. }));
        assert_eq!(guard.pending_protect(), None);
        assert!(!guard.blocks_follow(0));

        let retry = guard.tick(&scene.view_at(1), scene.condition(), crate::survival::CurePoison::WhenNeeded).unwrap();
        assert_eq!(retry, first, "a refused host send must remain retryable");
        guard.admitted(&retry, &scene.view_at(1));
        assert_eq!(guard.pending_protect(), Some(scene.missiles().button_com));
        assert_eq!(guard.pending_protect_tick(), Some(1));
        assert!(guard.blocks_follow(1));
        assert!(
            guard.tick(&scene.view_at(2), scene.condition(), CurePoison::WhenNeeded).is_none(),
            "pacing starts only after the successful retry"
        );
    }

    #[test]
    fn end_owes_off_for_a_protect_sent_before_acknowledgement() {
        let mut scene = Scene::new(43);
        scene.launch_arrow();
        let mut guard = scene.begin().unwrap();
        let click = guard.tick(&scene.view_at(0), scene.condition(), crate::survival::CurePoison::WhenNeeded).unwrap();
        guard.admitted(&click, &scene.view_at(0));
        assert_eq!(
            guard.end().0,
            GuardOp::IfButton {
                component: scene.missiles().button_com
            },
            "a sent but unacknowledged protect still needs its matching off-click"
        );
    }

    #[test]
    fn preexisting_protect_switch_ends_only_the_guard_admitted_new_style() {
        let mut scene = Scene::new(43);
        let melee_varp = scene.tables.prayer(PrayerRole::Protect, 2).unwrap().varp;
        scene.set_protect_varp(melee_varp, true);
        scene.launch_arrow();
        let mut guard = scene.begin().unwrap();
        let missiles = scene.missiles().button_com;
        let click = guard.tick(&scene.view_at(0), scene.condition(), crate::survival::CurePoison::WhenNeeded).unwrap();
        assert_eq!(
            click,
            GuardOp::IfButton {
                component: missiles
            }
        );
        guard.admitted(&click, &scene.view_at(0));

        guard.observe_prayer(&scene.view_at(1));
        assert_eq!(
            guard.pending_protect(),
            Some(missiles),
            "the preexisting style cannot acknowledge the guard's switch"
        );
        scene.set_protect_varp(melee_varp, false);
        scene.set_missiles(true);
        guard.observe_prayer(&scene.view_at(2));
        assert_eq!(guard.pending_protect(), None);
        assert_eq!(guard.pending_protect_tick(), None);
        assert_eq!(
            guard.end().0,
            GuardOp::IfButton {
                component: missiles
            },
            "ending clears only the newly admitted style and never restores the old one"
        );
    }

    #[test]
    fn end_does_not_click_when_protect_was_turned_off() {
        let mut scene = Scene::new(43);
        scene.launch_arrow();
        let mut guard = scene.begin().unwrap();
        let click = guard.tick(&scene.view_at(0), scene.condition(), crate::survival::CurePoison::WhenNeeded).unwrap();
        guard.admitted(&click, &scene.view_at(0));
        scene.set_missiles(true);
        assert!(guard.tick(&scene.view_at(3), scene.condition(), CurePoison::WhenNeeded).is_none());
        scene.set_missiles(false);
        guard.observe_prayer(&scene.view_at(4));
        assert_eq!(
            guard.end().0,
            GuardOp::IfButton { component: 0 },
            "a remembered button must not toggle an already-off prayer on"
        );
    }

    #[test]
    fn end_clicks_only_the_protect_last_observed_on() {
        let mut scene = Scene::new(43);
        scene.launch_arrow();
        let mut guard = scene.begin().unwrap();
        let click = guard.tick(&scene.view_at(0), scene.condition(), crate::survival::CurePoison::WhenNeeded).unwrap();
        guard.admitted(&click, &scene.view_at(0));
        scene.set_missiles(true);
        assert!(guard.tick(&scene.view_at(3), scene.condition(), CurePoison::WhenNeeded).is_none());
        let missiles = scene.missiles().button_com;
        assert_eq!(
            guard.end().0,
            GuardOp::IfButton {
                component: missiles
            }
        );
    }

    #[test]
    fn prayer_sip_uses_the_shared_c5_floor() {
        for (points, due) in [(27, false), (26, true), (9, true), (8, true)] {
            let mut scene = Scene::new(43);
            scene.stats[5].effective = points;
            scene.add_prayer_potion();
            scene.launch_arrow();
            scene.set_missiles(true);
            let mut guard = scene.begin().unwrap();
            let op = guard.tick(&scene.view_at(10), scene.condition(), crate::survival::CurePoison::WhenNeeded);
            assert_eq!(
                matches!(&op, Some(GuardOp::Drink { .. })),
                due,
                "Prayer points {points}, got {op:?}"
            );
        }
    }

    #[test]
    fn a_prayer_dose_locks_the_follow_for_two_ticks() {
        let mut scene = Scene::new(43);
        scene.stats[5].effective = 0;
        scene.add_prayer_potion();
        scene.launch_arrow();
        let mut guard = scene.begin().unwrap();
        let drink = guard.tick(&scene.view_at(10), scene.condition(), crate::survival::CurePoison::WhenNeeded).unwrap();
        assert!(matches!(drink, GuardOp::Drink { .. }), "got {drink:?}");
        assert!(!guard.blocks_follow(11), "a drink proposal is not admitted");
        guard.admitted(&drink, &scene.view_at(10));
        assert!(guard.blocks_follow(11));
        assert_eq!(
            guard.tick(&scene.view_at(11), scene.condition(), CurePoison::WhenNeeded),
            Some(GuardOp::Locked { until: 13 })
        );
        assert!(guard.blocks_follow(12));
        assert!(!guard.blocks_follow(13));
    }

    #[test]
    fn locked_tick_records_a_brief_melee_onset_before_the_attacker_disappears() {
        let mut scene = Scene::new(43);
        scene.stats[5].effective = 0;
        scene.add_prayer_potion();
        scene.launch_arrow();
        let mut guard = scene.begin().unwrap();
        let drink = guard.tick(&scene.view_at(10), scene.condition(), crate::survival::CurePoison::WhenNeeded).unwrap();
        assert!(matches!(drink, GuardOp::Drink { .. }));
        guard.admitted(&drink, &scene.view_at(10));

        let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
        let melee_npc = data.npc_by_config("khazard_warlord").unwrap();
        let melee_sequence = data
            .style_seqs()
            .iter()
            .find(|row| {
                scene
                    .tables
                    .style_seq(row.seq_id)
                    .is_some_and(|mask| mask.contains(super::super::tables::StyleMask::MELEE))
            })
            .unwrap()
            .seq_id;
        scene.stats[5].effective = 5;
        scene.projectiles.clear();
        scene.npcs[0].r#type = Some(melee_npc.id as usize);
        scene.npcs[0].name = melee_npc.display.clone();
        scene.npcs[0].target = Some(ActorTargetView {
            kind: ActorKind::Player,
            index: 1,
        });
        scene.npcs[0].in_combat = false;
        scene.npcs[0].animation = melee_sequence;
        scene.npcs[0].animation_frame = 0;
        scene.refresh();
        assert_eq!(
            guard.tick(&scene.view_at(11), scene.condition(), CurePoison::WhenNeeded),
            Some(GuardOp::Locked { until: 13 })
        );

        scene.npcs.clear();
        scene.refresh();
        assert_eq!(
            guard.tick(&scene.view_at(12), scene.condition(), CurePoison::WhenNeeded),
            Some(GuardOp::Locked { until: 13 })
        );
        let melee_component = scene
            .tables
            .prayer(PrayerRole::Protect, 2)
            .unwrap()
            .button_com;
        assert_eq!(
            guard.tick(&scene.view_at(13), scene.condition(), CurePoison::WhenNeeded),
            Some(GuardOp::IfButton {
                component: melee_component
            }),
            "the onset observed during the lock must survive the attacker's disappearance"
        );
    }

    #[test]
    fn known_report_only_mixed_melee_and_thrower_can_switch_on_each_projectile_edge() {
        // A future pacing fix should invert this report-only characterization.
        let mut scene = Scene::new(43);
        let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
        let thrower = data.npc_by_config("death_troll_thrower1").unwrap();
        scene.npcs[0].r#type = Some(thrower.id as usize);
        scene.npcs[0].name = thrower.display.clone();
        scene.launch_style(276);
        let melee_row = data.npc_by_config("khazard_warlord").unwrap();
        let mut melee_npc = scene.npcs[0].clone();
        melee_npc.index = 8;
        melee_npc.r#type = Some(melee_row.id as usize);
        melee_npc.name = melee_row.display.clone();
        melee_npc.animation = data
            .style_seqs()
            .iter()
            .find(|row| {
                scene
                    .tables
                    .style_seq(row.seq_id)
                    .is_some_and(|mask| mask.contains(super::super::tables::StyleMask::MELEE))
            })
            .unwrap()
            .seq_id;
        melee_npc.animation_frame = 0;
        scene.npcs.push(melee_npc);
        scene.refresh();
        let missiles = scene.missiles().button_com;
        let melee = scene.tables.prayer(PrayerRole::Protect, 2).unwrap().clone();
        let mut guard = scene.begin().unwrap();
        let first = guard.tick(&scene.view_at(10), scene.condition(), crate::survival::CurePoison::WhenNeeded).unwrap();
        assert_eq!(
            first,
            GuardOp::IfButton {
                component: missiles
            }
        );
        guard.admitted(&first, &scene.view_at(10));
        scene.set_missiles(true);
        scene.projectiles.clear();
        scene.refresh();
        let second = guard.tick(&scene.view_at(11), scene.condition(), crate::survival::CurePoison::WhenNeeded).unwrap();
        assert_eq!(
            second,
            GuardOp::IfButton {
                component: melee.button_com
            },
            "without the projectile override the stronger melee threat wins"
        );
        guard.admitted(&second, &scene.view_at(11));
        scene.set_missiles(false);
        scene.set_protect_varp(melee.varp, true);
        scene.launch_style(276);
        let third = guard.tick(&scene.view_at(12), scene.condition(), crate::survival::CurePoison::WhenNeeded).unwrap();
        assert_eq!(
            third,
            GuardOp::IfButton {
                component: missiles
            },
            "the next thrower projectile overrides the same retained melee threat"
        );
        guard.admitted(&third, &scene.view_at(12));
        assert!(
            guard.blocks_follow(12),
            "each admitted switch skips that tick's follow"
        );
    }

    #[test]
    fn a_troll_rock_launch_selects_missiles() {
        let mut scene = Scene::new(43);
        let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
        let row = data.npc_by_config("death_troll_thrower1").unwrap();
        scene.npcs[0].r#type = Some(row.id as usize);
        scene.npcs[0].name = row.display.clone();
        scene.npcs[0].animation = 1142;
        scene.npcs[0].tile = scene.local.player.actor.tile;
        scene.npcs[0].network = scene.local.player.actor.tile;
        scene.launch_style(276);
        scene.projectiles[0].t1 = 364;
        scene.projectiles[0].t2 = 381;
        scene.refresh();
        scene.snapshot.seed_hitmarks(HitmarksView {
            marks: [HitmarkView {
                value: 0,
                kind: 0,
                cycle: 0,
            }; 4],
            loop_cycle: 364,
        });
        let mut guard = scene.begin().unwrap();
        let op = guard.tick(&scene.view(), scene.condition(), crate::survival::CurePoison::WhenNeeded);
        assert_eq!(
            op,
            Some(GuardOp::IfButton {
                component: scene.missiles().button_com
            }),
            "a classified rock projectile must select Missiles, got {op:?}"
        );
    }

    #[test]
    fn a_ranged_projectile_onset_does_not_select_melee() {
        let mut scene = Scene::new(43);
        let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
        let row = data.npc_by_config("death_troll_thrower1").unwrap();
        scene.npcs[0].r#type = Some(row.id as usize);
        scene.npcs[0].name = row.display.clone();
        scene.npcs[0].animation = 1142;
        scene.npcs[0].tile = scene.local.player.actor.tile;
        scene.npcs[0].network = scene.local.player.actor.tile;
        scene.launch_style(276);
        scene.npcs[0].in_combat = false;
        scene.projectiles[0].src = tile(scene.npcs[0].tile.x, scene.npcs[0].tile.z + 1);
        scene.projectiles[0].t1 = 364;
        scene.projectiles[0].t2 = 381;
        scene.refresh();
        scene.snapshot.seed_hitmarks(HitmarksView {
            marks: [HitmarkView {
                value: 0,
                kind: 0,
                cycle: 0,
            }; 4],
            loop_cycle: 360,
        });
        let mut guard = scene.begin().unwrap();
        let op = guard.tick(&scene.view(), scene.condition(), crate::survival::CurePoison::WhenNeeded);
        let melee = scene.tables.prayer(PrayerRole::Protect, 2).unwrap();
        assert_ne!(
            op,
            Some(GuardOp::IfButton {
                component: melee.button_com
            }),
            "a ranged projectile onset must not select Melee, got {op:?}"
        );
        assert_eq!(
            op,
            Some(GuardOp::IfButton {
                component: scene.missiles().button_com
            }),
            "a ranged projectile onset must select Missiles, got {op:?}"
        );
    }

    #[test]
    fn incoming_projectile_does_not_override_selected_protect() {
        let mut scene = Scene::new(43);
        scene.launch_spell();
        let mut guard = scene.begin().unwrap();
        let op = guard.tick(&scene.view(), scene.condition(), crate::survival::CurePoison::WhenNeeded);
        let magic = scene.magic();
        let missiles = scene.missiles();
        assert_eq!(
            op,
            Some(GuardOp::IfButton {
                component: magic.button_com
            }),
            "a magic projectile must keep the shared selector's Protect from Magic, got {op:?}"
        );
        assert_ne!(
            op,
            Some(GuardOp::IfButton {
                component: missiles.button_com
            }),
            "an incoming projectile must not override the selected style with Missiles"
        );
    }

    #[test]
    fn dragonfire_and_magic_projectiles_agree_on_the_same_protection_varp() {
        let mut scene = Scene::new(43);
        // R289 classifies dragonfire at the attacker, not on a projectile.
        // Extend only this test's classification fixture to exercise the
        // representable incoming Dragonfire + Magic agreement; prayer and
        // NPC facts still come from the selected revision.
        let mut facts: serde_json::Value =
            serde_json::from_str(include_str!("../../../api/data/game-data/289.json")).unwrap();
        let dragonfire = facts["style_spotanims"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|row| row["style"] == 8)
            .expect("selected dragonfire classification");
        dragonfire["where"] = serde_json::json!("projectile");
        let dragonfire = i32::try_from(dragonfire["spotanim_id"].as_i64().unwrap()).unwrap();
        scene.tables =
            CombatTables::build(Arc::new(serde_json::from_value(facts).unwrap())).unwrap();
        let cow = scene.tables.selected().npc_by_config("cow").unwrap().id;
        scene.npcs[0].r#type = Some(cow as usize);
        scene.npcs[0].in_combat = true;
        scene.npcs[0].target = Some(ActorTargetView {
            kind: ActorKind::Player,
            index: 1,
        });
        scene.projectiles = [88, dragonfire]
            .map(|spotanim| ProjectileView {
                spotanim,
                level: 0,
                src: tile(2800, 3300),
                target: Some(ActorTargetView {
                    kind: ActorKind::Player,
                    index: 1,
                }),
                t1: 0,
                t2: 30,
            })
            .to_vec();
        scene.refresh();
        let mut guard = scene.begin().unwrap();
        assert_eq!(
            guard.tick(&scene.view(), scene.condition(), CurePoison::WhenNeeded),
            Some(GuardOp::IfButton {
                component: scene.magic().button_com,
            }),
            "the incoming volley agrees on Magic, despite the cow's Melee fact"
        );
    }

    #[test]
    fn first_dose_does_not_leave_drink_pending() {
        let mut scene = Scene::new(43);
        scene.stats[5].effective = 0;
        scene.add_prayer_potion();
        scene.launch_arrow();
        let mut guard = scene.begin().unwrap();
        let first = guard.tick(&scene.view_at(10), scene.condition(), crate::survival::CurePoison::WhenNeeded).unwrap();
        assert!(matches!(first, GuardOp::Drink { .. }), "got {first:?}");
        guard.admitted(&first, &scene.view_at(10));
        assert_eq!(
            guard.tick(&scene.view_at(11), scene.condition(), CurePoison::WhenNeeded),
            Some(GuardOp::Locked { until: 13 })
        );
        let _ = guard.tick(&scene.view_at(12), scene.condition(), crate::survival::CurePoison::WhenNeeded);
        let second = guard.tick(&scene.view_at(13), scene.condition(), crate::survival::CurePoison::WhenNeeded);
        assert!(
            matches!(second, Some(GuardOp::Drink { .. })),
            "a later dose may be proposed after the admitted dose times out"
        );
    }
    fn is_eat(op: &Option<GuardOp>) -> bool {
        matches!(op, Some(GuardOp::Eat { .. }))
    }

    fn assert_eat(op: Option<&GuardOp>, item: &str) {
        match op {
            Some(GuardOp::Eat { name }) => assert_eq!(name.as_ref(), item),
            other => panic!("wanted Eat {item}, got {other:?}"),
        }
    }

    #[test]
    fn live_thrower_at_half_max_eats_the_largest_fitting_food() {
        let mut scene = Scene::new(37);
        scene.stats[3].effective = 20;
        for slot in 0..4 {
            scene.add_food("lobster", slot, 1);
        }
        scene.add_food("shrimp", 4, 1);
        scene.launch_troll_rock();
        let mut guard = scene.begin().unwrap();

        let op = guard.tick(&scene.view(), scene.condition(), crate::survival::CurePoison::WhenNeeded);
        assert_eat(op.as_ref(), "Lobster");
    }

    #[test]
    fn zero_danger_does_not_trigger_eating() {
        let mut scene = Scene::new(43);
        scene.stats[3].effective = 1;
        scene.add_food("lobster", 0, 1);
        let mut guard = scene.begin().unwrap();

        let op = guard.tick(&scene.view_at(0), scene.condition(), crate::survival::CurePoison::WhenNeeded);
        assert!(!is_eat(&op), "a Some(0) danger must not eat: {op:?}");
        assert_eq!(op, None);
    }

    #[test]
    fn unknown_danger_uses_half_max_and_the_combo_drink_gate() {
        for (hp, should_eat) in [(20, true), (21, false)] {
            let mut scene = Scene::new(43);
            scene.stats[3].effective = hp;
            scene.launch_unknown();
            scene.add_food("lobster", 0, 1);
            let mut guard = scene.begin().unwrap();

            let op = guard.tick(&scene.view_at(0), scene.condition(), crate::survival::CurePoison::WhenNeeded);
            assert_eq!(is_eat(&op), should_eat, "unknown danger at {hp}/40: {op:?}");
        }

        for (hp, should_eat) in [(2, false), (3, true)] {
            let mut scene = Scene::new(43);
            scene.stats[3].effective = hp;
            scene.launch_unknown();
            scene.add_food("tbwt_cooked_karambwan", 0, 1);
            let mut guard = scene.begin().unwrap();

            let op = guard.tick(&scene.view_at(0), scene.condition(), crate::survival::CurePoison::WhenNeeded);
            assert_eq!(
                is_eat(&op),
                should_eat,
                "combo at {hp}/40 must cross the strict half-max gate: {op:?}"
            );
        }
    }

    #[test]
    fn ordinary_food_waits_for_its_schedule_clock_after_count_drop() {
        let mut scene = Scene::new(43);
        scene.stats[3].effective = 1;
        scene.launch_arrow();
        scene.add_food("lobster", 0, 2);
        let mut guard = scene.begin().unwrap();

        let first = guard.tick(&scene.view_at(0), scene.condition(), crate::survival::CurePoison::WhenNeeded).unwrap();
        assert_eat(Some(&first), "Lobster");
        guard.admitted(&first, &scene.view_at(0));

        scene.inventory[0].count = 1;
        scene.refresh();
        let observed = guard.tick(&scene.view_at(1), scene.condition(), crate::survival::CurePoison::WhenNeeded);
        assert!(
            !is_eat(&observed),
            "the next-food clock is not ready: {observed:?}"
        );
        assert!(!guard.schedule.pending(OpKind::Eat));
        assert_eq!(guard.schedule.unsettled[OpKind::Eat.index()], 0);

        let early = guard.tick(&scene.view_at(2), scene.condition(), crate::survival::CurePoison::WhenNeeded);
        assert!(!is_eat(&early), "food must wait through tick 2: {early:?}");
        let ready = guard.tick(&scene.view_at(3), scene.condition(), crate::survival::CurePoison::WhenNeeded);
        assert_eat(ready.as_ref(), "Lobster");
    }

    #[test]
    fn hp_rise_settles_an_admitted_eat() {
        let mut scene = Scene::new(43);
        scene.stats[3].effective = 1;
        scene.launch_arrow();
        scene.add_food("lobster", 0, 1);
        let mut guard = scene.begin().unwrap();

        let eat = guard.tick(&scene.view_at(0), scene.condition(), crate::survival::CurePoison::WhenNeeded).unwrap();
        guard.admitted(&eat, &scene.view_at(0));
        assert!(guard.schedule.pending(OpKind::Eat));

        scene.stats[3].effective = 40;
        scene.refresh();
        let _ = guard.tick(&scene.view_at(1), scene.condition(), crate::survival::CurePoison::WhenNeeded);
        assert!(!guard.schedule.pending(OpKind::Eat));
        assert_eq!(guard.schedule.unsettled[OpKind::Eat.index()], 0);
    }

    #[test]
    fn lagged_karambwan_settles_and_retries_at_the_emergency_line() {
        let mut scene = Scene::new(43);
        scene.launch_unknown();
        for slot in 0..4 {
            scene.add_food("tbwt_cooked_karambwan", slot, 1);
        }
        let emergency = {
            let frame = Frame::borrow(scene.view_at(0)).unwrap();
            let mut threats = ThreatSet::default();
            threats.observe(&frame, &scene.tables, 0);
            let danger = threats.danger(&frame, &scene.tables, 0, false, false);
            assert_ne!(danger, Some(0));
            select::lines(danger, 40).emergency
        };
        scene.stats[3].effective = emergency;
        scene.refresh();
        let mut guard = scene.begin().unwrap();

        for tick in [0, 4, 8, 12] {
            assert_eq!(scene.stats[3].effective, emergency);
            let eat = guard.tick(&scene.view_at(tick), scene.condition(), crate::survival::CurePoison::WhenNeeded).unwrap();
            assert_eat(Some(&eat), "Cooked karambwan");
            guard.admitted(&eat, &scene.view_at(tick));

            let pending = guard.tick(&scene.view_at(tick.wrapping_add(1)), scene.condition(), crate::survival::CurePoison::WhenNeeded);
            assert!(
                !is_eat(&pending),
                "a pending Eat must not repeat: {pending:?}"
            );
            assert!(
                guard.schedule.pending(OpKind::Eat),
                "the observation window must outlast the food clock"
            );
            assert_eq!(guard.schedule.unsettled[OpKind::Eat.index()], 0);

            scene.inventory.remove(0);
            scene.refresh();
            let settled = guard.tick(&scene.view_at(tick.wrapping_add(2)), scene.condition(), crate::survival::CurePoison::WhenNeeded);
            assert!(
                !is_eat(&settled),
                "the combo input lock is still active: {settled:?}"
            );
            assert!(!guard.schedule.pending(OpKind::Eat));
            assert_eq!(
                guard.schedule.unsettled[OpKind::Eat.index()],
                0,
                "each two-tick count-drop observation must settle the Eat"
            );
        }
    }

    #[test]
    fn an_unobserved_eat_times_out_three_times_then_stops() {
        let mut scene = Scene::new(43);
        scene.stats[3].effective = 1;
        scene.launch_arrow();
        scene.add_food("lobster", 0, 1);
        let mut guard = scene.begin().unwrap();

        let first = guard.tick(&scene.view_at(0), scene.condition(), crate::survival::CurePoison::WhenNeeded).unwrap();
        assert_eat(Some(&first), "Lobster");
        guard.admitted(&first, &scene.view_at(0));
        let window = u16::from(EAT_OBSERVATION_WINDOW_TICKS);
        for tick in 1..window {
            let op = guard.tick(&scene.view_at(tick), scene.condition(), crate::survival::CurePoison::WhenNeeded);
            assert!(!is_eat(&op), "a pending Eat must not repeat: {op:?}");
            assert!(guard.schedule.pending(OpKind::Eat));
        }

        for (tick, unsettled) in [(window, 1), (window * 2, 2)] {
            let op = guard.tick(&scene.view_at(tick), scene.condition(), crate::survival::CurePoison::WhenNeeded);
            assert_eat(op.as_ref(), "Lobster");
            assert_eq!(guard.schedule.unsettled[OpKind::Eat.index()], unsettled);
            guard.admitted(op.as_ref().unwrap(), &scene.view_at(tick));
        }

        let stopped = guard.tick(&scene.view_at(window * 3), scene.condition(), crate::survival::CurePoison::WhenNeeded);
        assert!(
            !is_eat(&stopped),
            "third timeout must stop food: {stopped:?}"
        );
        assert_eq!(guard.schedule.unsettled[OpKind::Eat.index()], 3);
        assert!(!guard.schedule.pending(OpKind::Eat));
    }

    #[test]
    fn message_delay_food_locks_the_follow_clock() {
        let mut scene = Scene::new(43);
        scene.stats[3].effective = 10;
        scene.launch_unknown();
        scene.add_food("tbwt_cooked_karambwan", 0, 1);
        let mut guard = scene.begin().unwrap();

        let eat = guard.tick(&scene.view_at(0), scene.condition(), crate::survival::CurePoison::WhenNeeded).unwrap();
        assert_eat(Some(&eat), "Cooked karambwan");
        guard.admitted(&eat, &scene.view_at(0));
        assert!(guard.blocks_follow(1));
        assert_eq!(
            guard.tick(&scene.view_at(1), scene.condition(), CurePoison::WhenNeeded),
            Some(GuardOp::Locked { until: 4 })
        );
        assert!(guard.blocks_follow(0));
        assert!(guard.blocks_follow(2));
        assert!(guard.blocks_follow(3));
        assert!(!guard.blocks_follow(4));
    }

    #[test]
    fn emergency_eating_precedes_protection_but_protection_wins_above_emergency() {
        let mut scene = Scene::new(43);
        scene.launch_known_melee_threat();
        scene.add_food("lobster", 0, 1);
        let lines = {
            let frame = Frame::borrow(scene.view_at(0)).unwrap();
            let mut threats = ThreatSet::default();
            threats.observe(&frame, &scene.tables, 0);
            let danger = threats
                .danger(&frame, &scene.tables, 0, false, false)
                .expect("the live melee NPC has known danger");
            assert!(danger > 0);
            select::lines(Some(danger), 40)
        };
        assert!(lines.emergency < lines.eat);
        scene.stats[3].effective = lines.emergency;
        scene.refresh();
        let mut guard = scene.begin().unwrap();
        assert_eat(
            guard.tick(&scene.view_at(0), scene.condition(), crate::survival::CurePoison::WhenNeeded).as_ref(),
            "Lobster",
        );

        scene.stats[3].effective = lines.emergency + 1;
        scene.refresh();
        let mut guard = scene.begin().unwrap();
        let op = guard.tick(&scene.view_at(0), scene.condition(), crate::survival::CurePoison::WhenNeeded);
        assert!(
            matches!(
                op,
                Some(GuardOp::IfButton { component })
                    if component == scene.tables.prayer(PrayerRole::Protect, 2).unwrap().button_com
            ),
            "protection must win before an ordinary Eat: {op:?}"
        );
    }

    #[test]
    fn guard_cure_before_emergency_bite() {
        let mut scene = Scene::new(43);
        scene.launch_known_melee_threat();
        scene.add_food("lobster", 0, 1);
        scene.add_antipoison();
        let lines = {
            let frame = Frame::borrow(scene.view_at(0)).unwrap();
            let mut threats = ThreatSet::default();
            threats.observe(&frame, &scene.tables, 0);
            let danger = threats
                .danger(&frame, &scene.tables, 0, false, false)
                .expect("known melee danger");
            select::lines(Some(danger), 40)
        };
        scene.stats[3].effective = lines.emergency;
        scene.refresh();
        let condition = BotCondition {
            tick: 0,
            session: api::condition::SessionMark::Live,
            hp: u8::try_from(scene.stats[3].effective).unwrap(),
            hp_max: 40,
            hp_known: true,
            poison: api::condition::PoisonState::Onset {
                since: 0,
                q: Some(11),
                hit_since_line: false,
            },
            food: api::condition::FoodSummary::default(),
            cures: api::condition::CureDoses::default(),
        };
        let request = scene.request();
        for food_only in [false, true] {
            let mut guard = if food_only {
                WalkGuard::begin_food_only_with(
                    &request,
                    &scene.view_at(0),
                    Arc::clone(&scene.tables),
                )
                .unwrap()
            } else {
                WalkGuard::begin_with(&request, &scene.view_at(0), Arc::clone(&scene.tables))
                    .unwrap()
            };
            let op = guard.tick(&scene.view_at(0), Some(&condition), CurePoison::WhenNeeded)
                .unwrap();
            assert!(
                matches!(
                    op,
                    GuardOp::Drink {
                        kind: PotionKind::Antipoison,
                        ..
                    }
                ),
                "food_only={food_only}, got {op:?}"
            );
        }
    }

    #[test]
    fn food_only_guard_drinks_cure() {
        let mut scene = Scene::new(43);
        scene.stats[3].effective = 10;
        scene.add_antipoison();
        let condition = BotCondition {
            tick: 0,
            session: api::condition::SessionMark::Live,
            hp: 10,
            hp_max: 40,
            hp_known: true,
            poison: api::condition::PoisonState::Onset {
                since: 0,
                q: Some(11),
                hit_since_line: false,
            },
            food: api::condition::FoodSummary::default(),
            cures: api::condition::CureDoses::default(),
        };
        let mut guard = WalkGuard::begin_food_only_with(
            &scene.request(),
            &scene.view(),
            Arc::clone(&scene.tables),
        )
        .unwrap();
        assert!(matches!(
            guard.tick(&scene.view_at(0), Some(&condition), CurePoison::WhenNeeded),
            Some(GuardOp::Drink {
                kind: PotionKind::Antipoison,
                ..
            })
        ));
    }

    #[test]
    fn guard_drinks_at_line_before_first_mark() {
        let mut scene = Scene::new(43);
        scene.stats[3].effective = 60;
        scene.stats[3].base = 99;
        scene.add_antipoison();
        scene.refresh();
        let condition = BotCondition {
            tick: 0,
            session: api::condition::SessionMark::Live,
            hp: 60,
            hp_max: 99,
            hp_known: true,
            poison: api::condition::PoisonState::Onset {
                since: 0,
                q: Some(11),
                hit_since_line: false,
            },
            food: api::condition::FoodSummary::default(),
            cures: api::condition::CureDoses::default(),
        };
        let mut guard = WalkGuard::begin_food_only_with(
            &scene.request(),
            &scene.view(),
            Arc::clone(&scene.tables),
        )
        .unwrap();
        let op = guard.tick(&scene.view_at(0), Some(&condition), CurePoison::WhenNeeded)
            .unwrap();
        assert!(
            matches!(
                op,
                GuardOp::Drink {
                    kind: PotionKind::Antipoison,
                    ..
                }
            ),
            "the onset line must cure before the first poison mark: {op:?}"
        );
    }

    #[test]
    fn guard_without_poison_unchanged() {
        let mut scene = Scene::new(43);
        scene.launch_known_melee_threat();
        scene.add_antipoison();
        let mut guard = scene.begin().unwrap();
        let op = guard.tick(&scene.view_at(0), None, CurePoison::WhenNeeded).unwrap();
        assert!(
            matches!(op, GuardOp::IfButton { .. }),
            "without a condition fact, the guard keeps its protection behavior: {op:?}"
        );
    }

    #[test]
    fn antipoison_receipt_settles_on_the_dose_drop() {
        let mut scene = Scene::new(43);
        scene.stats[3].effective = 10;
        scene.add_antipoison();
        let mut guard = WalkGuard::begin_food_only_with(
            &scene.request(),
            &scene.view(),
            Arc::clone(&scene.tables),
        )
        .unwrap();
        let active = BotCondition {
            tick: 0,
            session: api::condition::SessionMark::Live,
            hp: 10,
            hp_max: 40,
            hp_known: true,
            poison: api::condition::PoisonState::Onset {
                since: 0,
                q: Some(11),
                hit_since_line: false,
            },
            food: api::condition::FoodSummary::default(),
            cures: api::condition::CureDoses {
                antipoison: 1,
                ..api::condition::CureDoses::default()
            },
        };
        let drink = guard.tick(&scene.view_at(0), Some(&active), CurePoison::WhenNeeded)
            .unwrap();
        assert!(matches!(
            drink,
            GuardOp::Drink {
                kind: PotionKind::Antipoison,
                ..
            }
        ));
        guard.admitted(&drink, &scene.view_at(0));
        assert!(guard.schedule.pending(OpKind::Drink));

        let consumed_id = scene.tables.potion(PotionKind::Antipoison).unwrap().doses[0]
            .unwrap()
            .id;
        scene.inventory.retain(|row| row.def.id != consumed_id);
        scene.refresh();
        let clear = BotCondition {
            tick: 3,
            poison: api::condition::PoisonState::Clear,
            ..active
        };
        assert_eq!(
            guard.tick(&scene.view_at(3), Some(&clear), CurePoison::WhenNeeded),
            None
        );
        assert!(!guard.schedule.pending(OpKind::Drink));
    }

    #[test]
    fn strange_fruit_receipt_settles_on_the_count_drop() {
        let mut scene = Scene::new(43);
        scene.stats[3].effective = 10;
        scene.add_food("macro_triffidfruit", 0, 1);
        let fruit_id = scene
            .tables
            .selected()
            .item_by_alias("macro_triffidfruit")
            .unwrap()
            .id;
        let condition = BotCondition {
            tick: 0,
            session: api::condition::SessionMark::Live,
            hp: 10,
            hp_max: 40,
            hp_known: true,
            poison: api::condition::PoisonState::Onset {
                since: 0,
                q: Some(11),
                hit_since_line: false,
            },
            food: api::condition::FoodSummary::default(),
            cures: api::condition::CureDoses::default(),
        };
        let mut guard = WalkGuard::begin_food_only_with(
            &scene.request(),
            &scene.view_at(0),
            Arc::clone(&scene.tables),
        )
        .unwrap();
        let fruit = guard.tick(&scene.view_at(0), Some(&condition), CurePoison::WhenNeeded)
            .unwrap();
        assert!(matches!(
            fruit,
            GuardOp::Eat { ref name } if name.as_ref() == "Strange fruit"
        ));
        guard.admitted(&fruit, &scene.view_at(0));
        assert!(guard.schedule.pending(OpKind::Drink));

        scene.inventory.retain(|item| item.def.id != fruit_id);
        scene.refresh();
        let clear = BotCondition {
            tick: 3,
            poison: api::condition::PoisonState::Clear,
            ..condition
        };
        assert_eq!(
            guard.tick(&scene.view_at(3), Some(&clear), CurePoison::WhenNeeded),
            None
        );
        assert!(!guard.schedule.pending(OpKind::Drink));
    }
    #[test]
    fn food_permission_defaults_true_and_can_disable_eating() {
        assert!(crate::native::WalkAllow::default().food);

        let mut scene = Scene::new(43);
        scene.stats[3].effective = 1;
        scene.launch_arrow();
        scene.add_food("lobster", 0, 1);
        let mut request = scene.request();
        request.allow.food = false;
        let mut guard =
            WalkGuard::begin_with(&request, &scene.view(), Arc::clone(&scene.tables)).unwrap();
        let op = guard.tick(&scene.view_at(0), scene.condition(), crate::survival::CurePoison::WhenNeeded);
        assert!(
            matches!(
                op,
                Some(GuardOp::IfButton { component })
                    if component == scene.missiles().button_com
            ),
            "food denial must still allow protection: {op:?}"
        );
    }

    fn food_id(alias: &str) -> i32 {
        api::game_data::for_revision(ClientRevision::R289)
            .unwrap()
            .item_by_alias(alias)
            .unwrap()
            .id
    }

    fn eat_at(
        scene: &Scene,
        danger: Option<i32>,
        poison_fire: i32,
        hp: i32,
        hp_max: i32,
    ) -> Option<policy::EatChoice> {
        let frame = Frame::borrow(scene.view()).unwrap();
        policy::eat_choice(
            &frame,
            &scene.tables,
            &Schedule::default(),
            danger,
            poison_fire,
            hp,
            hp_max,
            0,
        )
    }

    #[test]
    fn eat_line_adds_next_poison_fire() {
        let mut scene = Scene::new(43);
        scene.add_food("lobster", 0, 1);
        // Danger Some(0) gives a base eat line of 1. One fire of 6 raises it
        // to 7, so HP 7 eats and HP 8 does not.
        let choice = eat_at(&scene, Some(0), 6, 7, 40).expect("HP 7 is at the raised line");
        assert_eq!(choice.ordinary, Some(food_id("lobster")));
        assert_eq!(eat_at(&scene, Some(0), 6, 8, 40), None);
        assert_eq!(eat_at(&scene, Some(0), 0, 7, 40), None);
    }

    #[test]
    fn eat_line_keeps_half_max_when_danger_unknown() {
        let mut scene = Scene::new(43);
        scene.add_food("lobster", 0, 1);
        // Unknown danger keeps the half-max line of 50 at max HP 99. One fire
        // of 1 raises it to 51, never to a small fabricated line.
        assert_eq!(select::lines(None, 99).with_poison(1).eat, 51);
        assert!(eat_at(&scene, None, 1, 51, 99).is_some());
        assert_eq!(eat_at(&scene, None, 1, 52, 99), None);
    }

    #[test]
    fn quiet_poison_tick_reaches_picker() {
        let mut scene = Scene::new(43);
        scene.add_food("lobster", 0, 1);
        // No attacker danger exists. A poison fire of 1 alone must still
        // reach the food picker at the raised line of 2.
        let choice = eat_at(&scene, Some(0), 1, 2, 40).expect("a quiet poison tick eats");
        assert_eq!(choice.ordinary, Some(food_id("lobster")));
    }

    #[test]
    fn eat_never_bites_at_full_hp() {
        let mut scene = Scene::new(43);
        scene.add_food("lobster", 0, 5);
        // Max HP 10 at full HP with a fire of 11 due. A bite heals nothing.
        assert_eq!(eat_at(&scene, Some(0), 11, 10, 10), None);
        // Max HP 99 at full HP under danger 60. The eat line of 121 is above
        // full HP, but a bite still heals nothing.
        assert_eq!(eat_at(&scene, Some(60), 0, 99, 99), None);
    }

    #[test]
    fn eat_small_account_bites_once_per_fire() {
        let mut scene = Scene::new(43);
        scene.add_food("lobster", 0, 5);
        // After a fire of 6, HP 4 of 10 bites once. At HP 10 of 10, with the
        // next fire still due, it does not bite again.
        assert!(eat_at(&scene, Some(0), 6, 4, 10).is_some());
        assert_eq!(eat_at(&scene, Some(0), 6, 10, 10), None);
    }

    #[test]
    fn replay_line_exceeds_guard_line_on_quiet_poison() {
        // One quiet poisoned case for both real decision paths: a six-damage
        // fire with no attacker. This replaces `replay_and_guard_agree_poisoned`,
        // which supplied its own replay danger and never called the replay.
        // The paths do not agree: the guard charges the fire onto its own
        // base line, while the replay folds the fire into a more
        // conservative no-crossing line of its own.
        let mut scene = Scene::new(43);
        scene.stats[3].effective = 8;
        scene.add_food("lobster", 0, 1);
        scene.set_poisoned(6);
        let poison_fire = crate::survival::math::poison_fire(
            scene.condition(),
            &ThreatSet::default(),
            &scene.tables,
            0,
        );
        assert_eq!(poison_fire, 6, "one six-damage fire is due");

        // The replay's line from its own no-crossing path: `eat_line` returns
        // `2 * poison_pulse_now + 1` when no crossing lies inside the top-up
        // window (`combat/risk/replay.rs:391-400`). A marked six-damage poison
        // has a pulse of 6 (`replay.rs:212-222`), so the line is 2 * 6 + 1 = 13.
        let lease = crate::gatherer::select::tests::test_nav_lease(|_| {});
        let route = nav::router::Route {
            legs: vec![nav::router::Leg::Walk {
                tiles: (10..=20).map(|x| tile(x, 100)).collect(),
            }],
            dest: tile(20, 100),
            ticks: 0.0,
        };
        let path =
            crate::combat::risk::RoutePath::new(nav::producer::route_view(&lease, &route).unwrap())
                .unwrap();
        let plan = crate::combat::risk::RoutePlan::default();
        let mut input = crate::combat::risk::RiskInput {
            hp: 8,
            hp_max: 40,
            pos: tile(10, 100),
            poison: api::condition::PoisonState::Poisoned {
                per_tick: 6,
                last_mark: 0,
                hit_since_mark: false,
            },
            food_len: 1,
            // `RiskInput::default()` marks the capture incomplete; this scene
            // is a complete quiet observation.
            missing_facts: false,
            unattributed: false,
            input_locked: false,
            overflow: false,
            ..crate::combat::risk::RiskInput::default()
        };
        input.food_ids[0] = food_id("lobster");
        input.food_counts[0] = 1;
        let line = crate::combat::risk::eat_line(
            path,
            &plan,
            0,
            &input,
            &scene.tables,
            crate::native::WalkAllow::default(),
        )
        .unwrap();
        assert_eq!(line, 13, "the replay's own quiet poison line");

        // The real replay loop's eat decision (`replay.rs:774-842`) calls
        // `eat_choice` only when `topup || hp <= intended`, with
        // `danger = Some(intended / 2)` and the `poison_fire` argument 0
        // because its own line already charged the fire
        // (`replay.rs:801-803,832-841`). This fixture has no crossing, so
        // `intended = line = 13`. The tick-0 fire lands before the decision
        // (`replay.rs:746-747`).
        let replay_bites = |hp: u8| {
            let mut input = input;
            input.hp = hp;
            let mut bites = Vec::new();
            let result = crate::combat::risk::replay(
                path,
                &plan,
                &input,
                &scene.tables,
                crate::native::WalkAllow::default(),
                None,
                |tick, _, _, _, bite| {
                    if let Some(id) = bite {
                        bites.push((tick, id));
                    }
                },
            )
            .unwrap();
            assert!(result.passed, "a one-fire quiet route passes at HP {hp}");
            (result.hp_after, bites)
        };
        // At HP 8 the tick-0 fire leaves 2, below the replay's line 13: the
        // outer gate admits the decision and it bites on the first tick.
        assert_eq!(
            replay_bites(8).1,
            vec![(0, food_id("lobster"))],
            "the replay's line 13 permits a bite at HP 8"
        );
        // HP 19 -> 13 after the fire still bites; HP 20 -> 14 does not. The
        // HP 20 result is the replay's outer gate at `replay.rs:803`:
        // decision HP 14 exceeds `intended = 13`, so `eat_choice` is never
        // called and this case does not exercise the zero `poison_fire`
        // argument at `replay.rs:837`.
        assert_eq!(replay_bites(19).1, vec![(0, food_id("lobster"))]);
        assert_eq!(
            replay_bites(20).1,
            Vec::new(),
            "the replay's intended line 13 rejects the HP 14 decision before eat_choice runs"
        );

        // The guard's line for the same case is `lines(danger) + p`
        // (`combat/guard.rs:454-470` passes the observed danger and the fire
        // to the shared helper): base line 1 plus the six-damage fire is 7.
        assert_eq!(select::lines(Some(0), 40).with_poison(poison_fire).eat, 7);
        let mut at_eight = Scene::new(43);
        at_eight.stats[3].effective = 8;
        at_eight.add_food("lobster", 0, 1);
        at_eight.set_poisoned(6);
        let mut guard = at_eight.begin().unwrap();
        assert_eq!(
            guard.tick(&at_eight.view_at(0), at_eight.condition(), CurePoison::WhenNeeded),
            None,
            "the guard's line 7 rejects a bite at HP 8"
        );
        let mut at_seven = Scene::new(43);
        at_seven.stats[3].effective = 7;
        at_seven.add_food("lobster", 0, 1);
        at_seven.set_poisoned(6);
        let mut guard = at_seven.begin().unwrap();
        assert_eat(
            guard.tick(&at_seven.view_at(0), at_seven.condition(), crate::survival::CurePoison::WhenNeeded)
                .as_ref(),
            "Lobster",
        );
    }

    #[test]
    fn guard_eats_on_quiet_poison_tick() {
        let mut scene = Scene::new(43);
        scene.stats[3].effective = 7;
        scene.add_food("lobster", 0, 1);
        let mut guard = scene.begin().unwrap();
        assert_eq!(guard.tick(&scene.view_at(0), scene.condition(), CurePoison::WhenNeeded), None);

        let mut scene = Scene::new(43);
        scene.stats[3].effective = 7;
        scene.add_food("lobster", 0, 1);
        scene.set_poisoned(6);
        let mut guard = scene.begin().unwrap();
        assert_eat(
            guard.tick(&scene.view_at(0), scene.condition(), crate::survival::CurePoison::WhenNeeded).as_ref(),
            "Lobster",
        );
    }

    #[test]
    fn eat_choice_without_poison_unchanged() {
        let mut scene = Scene::new(43);
        scene.add_food("lobster", 0, 1);
        // With no poison fire and HP below max, every line keeps its old
        // value: Some(0) never eats, danger 5 eats at 11, unknown danger
        // eats at half of 40, and dead HP never eats.
        for (danger, hp, eats) in [
            (Some(0), 5, false),
            (Some(5), 11, true),
            (Some(5), 12, false),
            (Some(1), 3, true),
            (None, 20, true),
            (None, 21, false),
            (Some(5), 0, false),
        ] {
            assert_eq!(
                eat_at(&scene, danger, 0, hp, 40).is_some(),
                eats,
                "danger {danger:?} at HP {hp}/40"
            );
        }
    }

    #[test]
    fn food_by_picker_unchanged() {
        let mut scene = Scene::new(43);
        scene.add_food("lobster", 0, 1);
        scene.add_food("shrimp", 1, 1);
        let lobster = food_id("lobster");
        let shrimp = food_id("shrimp");
        let pick = |hp: i32| {
            policy::food_by(
                hp,
                40,
                [(lobster, 1), (shrimp, 1)].into_iter(),
                &scene.tables,
                None,
                |_| true,
            )
        };
        // A deficit of 20 takes the largest fitting heal, the lobster. A
        // deficit of 10 takes the shrimp, because the lobster does not fit.
        // A deficit of 1 fits neither, so it takes the smallest, the shrimp.
        assert_eq!(pick(20), Some(lobster));
        assert_eq!(pick(30), Some(shrimp));
        assert_eq!(pick(39), Some(shrimp));
    }
}
