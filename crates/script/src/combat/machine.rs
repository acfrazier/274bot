use super::arbiter::{self, PlanRow, RowKind, TickPlan};
use super::frame::Frame;
use super::policy;
use super::prayer::{PrayerSweep, RaisedPrayers};
use super::request::*;
use super::schedule::{elapsed, reached, Interaction, OpKind, Schedule};
use super::select;
use super::tables::{CombatTab, CombatTables, PotionKind, PrayerRole};
use super::threats::{StyleObs, ThreatSet};
use crate::native::{ActionContext, ActionError, NativeMachine, WalkEnd, WalkRequest};
use crate::shim::InteractReq;
use api::prayer::PrayerObservation;
use api::snapshot::ItemView;
use std::num::NonZeroU64;
use std::sync::Arc;
use std::task::Poll;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Prep,
    Engage,
    Fight,
    Escape,
    WindDown,
}
#[derive(Default)]
struct Counters {
    ticks: u16,
    swings: u16,
    damage: u16,
    food: u8,
    prayer: u8,
    boost: u8,
    antifire: u8,
    protected: u8,
    switches: u8,
    intruders: u8,
    restorations: u8,
    locked: u8,
    multi: u8,
}

/// Server-effect identity survives the host's rotating dispatch-receipt ring.
#[derive(Clone, Copy, Default)]
struct PendingRow {
    row: PlanRow,
    baseline: i16,
    age: u8,
}
/// One shared planner and one host owner. Observation commits are independent
/// from admitted-operation commits; no-op ticks are real progress.
pub struct Combat {
    request: Arc<CombatRequest>,
    tables: Arc<CombatTables>,
    player_ident: Option<i32>,
    deadline: Duration,
    pending_walk: Option<NonZeroU64>,
    threats: ThreatSet,
    schedule: Schedule,
    sweep: PrayerSweep,
    counters: Counters,
    plan: TickPlan,
    plan_first_id: u64,
    pending: [PendingRow; 5],
    staged_pending: u8,
    // Three-tick deadlines are retired every observed tick, before byte wrap.
    prayer_ready: [u8; 15],
    prayer_ready_mask: u16,
    // Bit 15 marks readiness; the low 15 bits protect the complete user baseline.
    baseline_on: u16,
    raised_prayers: RaisedPrayers,
    eat_ready: u16,
    food_total: u16,
    mode_fallback: Option<MeleeMode>,
    // Fourteen wear slots: two failure bits and one Prep-origin bit each.
    // The remaining bits hold Prep-origin and skipped boost-family masks.
    prep_failures: u64,
    sequence: u64,
    rhand_pick: i32,
    engaged_type: i32,
    suspended_type: i32,
    chat_since: i32,
    anim_id: i32,
    anim_frame: i32,
    engaged: Option<ActorRef>,
    suspended: Option<ActorRef>,
    end: Option<CombatEnd>,
    antifire_sip: u16,
    lost_since: u16,
    escape_since: u16,
    last_tick: u16,
    emitted_tick: u16,
    phase: Phase,
    flags: u16,
}
const OBSERVED: u16 = 1;
const EMITTED: u16 = 2;
const LOST: u16 = 4;
const ANTIFIRE: u16 = 8;
const PLAN_FIGHT: u16 = 16;
const RESTORE_COUNTED: u16 = 32;
const BOOST_WORTH: u16 = 64;
const SHIELD_OVERRIDE: u16 = 128;
const ENGAGEMENT_ATTACK: u16 = 256;
const FAILED_WEAPON: u16 = 512;
const FAILED_SHIELD: u16 = 1024;
const FAILED_ATTACK: u16 = 2048;
const TERMINAL_FOOD: u16 = 4096;
const PREP_RETALIATE: u16 = 8192;
const SKIP_RETALIATE: u16 = 16384;
const LEASH_WALKED: u16 = 32768;
const POTION_PREP_SHIFT: u32 = 42;
const POTION_SKIP_SHIFT: u32 = 49;
const _: () = assert!(std::mem::size_of::<Combat>() <= 512);
const BASELINE_READY: u16 = 1 << api::prayer::PRAYER_COUNT;

impl NativeMachine for Combat {
    type Args = (Arc<CombatRequest>, Arc<CombatTables>);
    type Output = CombatReport;
    fn begin(
        (request, tables): Self::Args,
        cx: &mut ActionContext<'_>,
    ) -> Result<Self, ActionError> {
        if request.style != Style::Melee {
            return Err(unavailable("style slice"));
        }
        if request.prayer_mode != PrayerMode::Hold {
            return Err(unavailable("flick slice"));
        }
        if matches!(request.target, Target::Player { .. }) {
            return Err(unavailable("pvp slice"));
        }
        if request.tactic != Tactic::Open || matches!(request.fallback, Fallback::FaceTank { .. }) {
            return Err(unavailable("tactic slice"));
        }
        if tables
            .selected()
            .selected_pin()
            .map_err(|_| unavailable("combat pin"))?
            .as_ref()
            != cx.pin()
        {
            return Err(unavailable("combat pin"));
        }
        if let Target::Npc { types, .. } = &request.target {
            if types.is_empty() || types.iter().any(|id| tables.npc(*id).is_none()) {
                return Err(unavailable("combat npc facts"));
            }
            if !types
                .iter()
                .any(|id| tables.npc_fact(*id).is_some_and(|row| row.attackable))
            {
                return Err(unavailable("unattackable npc target"));
            }
        }
        let ticks = if request.budget_ticks == 0 {
            1500
        } else {
            request.budget_ticks
        };
        let mut machine = Self {
            request,
            tables,
            player_ident: None,
            deadline: cx
                .active_now()
                .saturating_add(Duration::from_millis(u64::from(ticks) * 600)),
            pending_walk: None,
            threats: ThreatSet::default(),
            schedule: Schedule::default(),
            sweep: PrayerSweep::new(),
            counters: Counters::default(),
            sequence: 0,
            rhand_pick: -1,
            engaged_type: -1,
            suspended_type: -1,
            chat_since: -1,
            anim_id: -1,
            anim_frame: -1,
            engaged: None,
            suspended: None,
            end: None,
            antifire_sip: 0,
            lost_since: 0,
            escape_since: 0,
            last_tick: 0,
            emitted_tick: 0,
            plan: TickPlan::default(),
            plan_first_id: 0,
            pending: [PendingRow::default(); 5],
            staged_pending: 0,
            prayer_ready: [0; 15],
            prayer_ready_mask: 0,
            baseline_on: 0,
            raised_prayers: RaisedPrayers::default(),
            eat_ready: 0,
            food_total: 0,
            mode_fallback: None,
            prep_failures: 0,
            phase: Phase::Prep,
            flags: 0,
        };
        if let Some(lines) = cx.snapshot().chat_lines(-1) {
            machine.chat_since = lines
                .value
                .iter()
                .map(|row| row.sequence)
                .max()
                .unwrap_or(-1);
        }
        if let Some(frame) = Frame::borrow(cx.snapshot()) {
            machine.capture_prayer_baseline(&prayer_observation(&frame));
            let picked = select::pick_target(
                &machine.request,
                &frame,
                &machine.threats,
                &machine.tables,
                cx.run().slot ^ cx.run().run,
            );
            if let Some(actor) = picked {
                machine.engage(actor, &frame);
            }
            machine.pick_weapon(&frame);
            machine.resolve_worth(&frame);
        }
        Ok(machine)
    }
    fn poll(&mut self, cx: &mut ActionContext<'_>) -> Poll<Result<CombatReport, ActionError>> {
        if self.pending_walk.is_some_and(|id| {
            cx.walk_receipt(id.get())
                .is_some_and(|receipt| receipt.end == WalkEnd::UserInput)
        }) {
            return Poll::Ready(Err(ActionError::UserInput));
        }
        let snapshot = cx.snapshot();
        let Some(frame) = Frame::borrow(snapshot) else {
            return Poll::Pending;
        };
        let Some(chat) = snapshot.chat_lines(self.chat_since) else {
            return Poll::Pending;
        };
        let evidence = cx.evidence();
        if self.flags & OBSERVED != 0
            && (self.sequence == evidence.sequence || self.last_tick == evidence.tick as u16)
        {
            return Poll::Pending;
        }
        // Dispatch happened against the preceding poll's snapshot. Do not
        // overwrite its plan until every receipt is available.
        if !self.receipts(cx) {
            return Poll::Pending;
        }
        self.capture_prayer_baseline(&prayer_observation(&frame));
        let tick = evidence.tick as u16;
        self.advance_prayer_ready(tick);
        let gap = self.flags & OBSERVED == 0 || elapsed(tick, self.last_tick) > 1;
        self.sequence = evidence.sequence;
        self.last_tick = tick;
        if gap {
            self.advance_eat_ready(tick);
            self.schedule.interaction = Interaction::Unknown;
            self.schedule.cycle.known = false;
        }
        self.flags |= OBSERVED;
        self.counters.ticks = self.counters.ticks.saturating_add(1);
        if self.phase != Phase::Escape
            && self
                .pending_walk
                .is_some_and(|id| cx.walk_receipt(id.get()).is_some())
        {
            self.pending_walk = None;
        }
        let events = self.threats.observe(&frame, &self.tables, tick);
        self.counters.damage = self.counters.damage.saturating_add(events.damage);
        self.counters.protected = self
            .counters
            .protected
            .saturating_add(events.positive_protected);
        self.settle(&frame, tick);
        let died = chat
            .value
            .iter()
            .any(|row| row.text.contains("Oh dear, you are dead"));
        if let Some(sequence) = chat.value.iter().map(|row| row.sequence).max() {
            self.chat_since = sequence;
        }
        if self.phase != Phase::WindDown {
            if died || arbiter::stat(&frame, 3).0 <= 0 {
                self.finish(CombatEnd::Died, tick);
            } else {
                self.observe_target(&frame, tick);
            }
            if self.phase != Phase::WindDown && cx.active_now() >= self.deadline {
                self.finish(CombatEnd::Budget, tick);
            }
            if self.phase != Phase::WindDown
                && self.request.until_ticks != 0
                && self.counters.ticks >= self.request.until_ticks
            {
                self.finish(CombatEnd::Budget, tick);
            }
            if self.phase != Phase::WindDown
                && matches!(self.request.target, Target::Attacker { .. })
                && select::pick_target(
                    &self.request,
                    &frame,
                    &self.threats,
                    &self.tables,
                    evidence.tick,
                )
                .is_none()
                && select::has_unattackable_allowed(&self.request, &frame, &self.threats)
            {
                self.finish(CombatEnd::Aborted(AbortReason::Unattackable), tick);
            }
            if self.phase != Phase::WindDown {
                let failure = if self.flags & FAILED_WEAPON != 0 {
                    Some(AbortReason::PrepFailed(PrepItem::Weapon))
                } else if self.flags & FAILED_SHIELD != 0 {
                    Some(AbortReason::PrepFailed(PrepItem::Shield))
                } else if self.flags & FAILED_ATTACK != 0 {
                    Some(AbortReason::Unresponsive)
                } else {
                    None
                };
                if let Some(reason) = failure {
                    self.finish(CombatEnd::Aborted(reason), tick);
                }
            }
        }
        if self.phase == Phase::Escape {
            if let Fallback::Retreat { tile } = self.request.fallback {
                if distance(frame.here, tile) <= 1 {
                    self.finish(CombatEnd::Aborted(AbortReason::Retreated), tick);
                }
            }
        }
        // Onsets/settles/ends above commit even when this tick must be silent.
        if self.schedule.locked(tick) {
            let danger = self.threats.danger(
                &frame,
                &self.tables,
                tick,
                self.shield(&frame),
                self.antifire(tick),
            );
            let lines = select::lines(danger, arbiter::stat(&frame, 3).1);
            let food_available = self.emergency_food_available(&frame, danger);
            self.safety_end(&frame, tick, danger, lines.emergency, food_available);
            self.counters.locked = self.counters.locked.saturating_add(1);
            return Poll::Pending;
        }
        self.schedule.unlock_observed(tick);
        self.plan = TickPlan::default();
        match self.plan(&frame, tick, cx) {
            Ok(plan) => {
                self.plan = plan;
                if plan.len == 0 {
                    if self.phase == Phase::WindDown
                        && self.baseline_on & BASELINE_READY != 0
                        && self.sweep.pending_mask() == 0
                        && !(self.flags & TERMINAL_FOOD != 0 && self.pending_row(RowKind::Eat))
                        && self
                            .sweep
                            .candidates(self.tables.selected(), &prayer_observation(&frame))
                            .find(|click| self.raised_prayers.contains(click.varp))
                            .is_none()
                    {
                        if self.sweep.report().timed_out != 0 {
                            return Poll::Ready(Err(ActionError::Failed(
                                "prayer cleanup did not settle".into(),
                            )));
                        }
                        if self.end == Some(CombatEnd::TargetGone)
                            && self.flags & LOST != 0
                            && self.engaged.is_some()
                            && self.flags & LEASH_WALKED == 0
                        {
                            if let Some(destination) = self.leash_cancel_tile(&frame) {
                                if let Err(error) = self.walk(destination, tick, cx) {
                                    return Poll::Ready(Err(error));
                                }
                                self.flags |= LEASH_WALKED;
                                return Poll::Pending;
                            }
                        }
                        return Poll::Ready(Ok(self.report(evidence)));
                    }
                    return Poll::Pending;
                }
                match self.emit(&frame, tick, cx) {
                    Ok(())
                    | Err(ActionError::BudgetExhausted)
                    | Err(ActionError::Unavailable(_)) => Poll::Pending,
                    Err(error) => Poll::Ready(Err(error)),
                }
            }
            Err(error) => Poll::Ready(Err(error)),
        }
    }
    fn cancel(&mut self) {
        self.schedule.terminal();
        self.pending_walk = None;
    }
}

impl Combat {
    pub fn input_lock(&self) -> Option<u16> {
        let schedule = self.projected_schedule();
        schedule
            .locked(self.last_tick)
            .then_some(schedule.input_lock)
    }
    pub fn interaction(&self) -> Interaction {
        self.projected_schedule().interaction
    }
    pub fn cycle(&self) -> super::schedule::Cycle {
        self.projected_schedule().cycle
    }
    pub fn eat_ready(&self) -> u16 {
        self.eat_ready
    }
    pub fn engaged(&self) -> Option<ActorRef> {
        self.engaged
    }
    fn finish(&mut self, end: CombatEnd, _tick: u16) {
        if self.end.is_some() {
            return;
        }
        self.end = Some(end);
        self.phase = Phase::WindDown;
        self.schedule.terminal();
        self.sweep = PrayerSweep::new();
    }
    fn report(&self, evidence: api::quest_progress::EvidenceStamp) -> CombatReport {
        CombatReport {
            end: self.end.expect("terminal phase has a latched end"),
            evidence,
            engaged: self.engaged,
            engaged_npc_type: self.engaged_type,
            ticks: self.counters.ticks,
            swings: self.counters.swings,
            casts: 0,
            damage_taken: self.counters.damage,
            food: self.counters.food,
            prayer_doses: self.counters.prayer,
            boost_doses: self.counters.boost,
            antifire_doses: self.counters.antifire,
            hits_while_protected: self.counters.protected,
            protect_switches: self.counters.switches,
            intruders: self.counters.intruders,
            ammo_pickups: 0,
            restorations: self.counters.restorations,
            locked_ticks: self.counters.locked,
            multi_op_plans: self.counters.multi,
            melee_mode_fallback: self.mode_fallback,
            flick_resets: 0,
            flick_misses: 0,
            flick_fallback: false,
        }
    }
    fn rate(&self, frame: &Frame<'_>) -> u8 {
        frame
            .equipment
            .iter()
            .find(|row| row.slot == 3)
            .and_then(|row| self.tables.weapon_style(row.def.id))
            .map_or(4, |row| row.attackrate.max(1))
    }
    fn engage(&mut self, actor: ActorRef, frame: &Frame<'_>) {
        if self.engaged != Some(actor) {
            for pending in &mut self.pending {
                if pending.row.kind == RowKind::Attack {
                    *pending = PendingRow::default();
                }
            }
            self.schedule.settle(OpKind::Attack);
        }
        self.engaged = Some(actor);
        self.flags &= !LOST;
        match actor.kind {
            ActorKind::Npc => {
                self.engaged_type = frame
                    .npcs
                    .iter()
                    .find(|row| row.index == usize::from(actor.index))
                    .and_then(|row| row.r#type)
                    .map_or(-1, |id| id as i32);
                self.player_ident = None;
            }
            ActorKind::Player => {
                self.engaged_type = -1;
                self.player_ident = frame
                    .players
                    .iter()
                    .find(|row| row.index == usize::from(actor.index))
                    .and_then(|row| row.actor.name.as_deref())
                    .map(select::ident::fnv1a);
            }
        }
        self.schedule.interaction = Interaction::Unknown;
        self.resolve_worth(frame);
    }
    fn resolve_worth(&mut self, frame: &Frame<'_>) {
        let carries_boost = self.request.kit.as_ref().is_some_and(|kit| {
            kit.carry.iter().any(|(id, _)| {
                [
                    PotionKind::SuperAttack,
                    PotionKind::SuperStrength,
                    PotionKind::SuperDefence,
                ]
                .iter()
                .any(|kind| {
                    self.tables.potion(*kind).is_some_and(|family| {
                        family.doses.iter().flatten().any(|dose| dose.id == *id)
                    })
                })
            })
        });
        let worth =
            select::offensives_worth(self.engaged, self.engaged_type, &self.tables, carries_boost);
        if worth {
            self.flags |= BOOST_WORTH;
        } else {
            self.flags &= !BOOST_WORTH;
        }
        if self
            .tables
            .npc(self.engaged_type)
            .is_some_and(|row| row.dragonfire.is_some())
        {
            self.flags |= SHIELD_OVERRIDE;
        } else {
            self.flags &= !SHIELD_OVERRIDE;
        }
        if self.rhand_pick < 0 {
            self.pick_weapon(frame);
        }
    }
    fn pick_weapon(&mut self, frame: &Frame<'_>) {
        if self.request.kit.is_some() {
            return;
        }
        let Some(names) = self.tables.selected().equipment_names() else {
            return;
        };
        let Some(family) = names.family("melee_weapons") else {
            return;
        };
        let name = crate::melee_weapons::best_melee_weapon_by(
            family,
            arbiter::stat(frame, 0).1,
            false,
            &[],
            |name| {
                frame.inventory.iter().chain(frame.equipment).any(|row| {
                    row.def
                        .name
                        .as_deref()
                        .is_some_and(|have| have.eq_ignore_ascii_case(name))
                })
            },
        );
        if let Some(name) = name {
            self.rhand_pick = frame
                .inventory
                .iter()
                .chain(frame.equipment)
                .find(|row| {
                    row.def
                        .name
                        .as_deref()
                        .is_some_and(|have| have.eq_ignore_ascii_case(name))
                })
                .map_or(-1, |row| row.def.id);
        }
    }
    fn observe_target(&mut self, frame: &Frame<'_>, tick: u16) {
        let seed = u64::from(tick) ^ self.sequence;
        if self.engaged.is_none() {
            if let Some(actor) =
                select::pick_target(&self.request, frame, &self.threats, &self.tables, seed)
            {
                self.engage(actor, frame);
            }
        }
        let mut present = false;
        let mut killed = false;
        if let Some(actor) = self.engaged {
            match actor.kind {
                ActorKind::Npc => {
                    if let Some(row) = frame
                        .npcs
                        .iter()
                        .find(|row| row.index == usize::from(actor.index))
                    {
                        let ty = row.r#type.map_or(-1, |id| id as i32);
                        let same = ty == self.engaged_type;
                        let listed = match &self.request.target {
                            Target::Npc { types, .. } => types.contains(&ty),
                            _ => same,
                        };
                        present = listed
                            && row
                                .actions
                                .iter()
                                .flatten()
                                .any(|action| action == "Attack")
                            && distance(row.tile, self.request.stand.unwrap_or(frame.here))
                                <= i32::from(self.request.lost_radius);
                        killed = same && row.total_health > 0 && row.health == 0;
                        if present && !same {
                            self.engaged_type = ty;
                            self.schedule.interaction = Interaction::Unknown;
                            self.resolve_worth(frame);
                        }
                    }
                }
                ActorKind::Player => {
                    if let Some(row) = frame.players.iter().find(|row| {
                        row.index == usize::from(actor.index)
                            && self.player_ident.is_some()
                            && row.actor.name.as_deref().map(select::ident::fnv1a)
                                == self.player_ident
                    }) {
                        present =
                            distance(row.actor.tile, self.request.stand.unwrap_or(frame.here))
                                <= i32::from(self.request.lost_radius);
                        killed = row.actor.total_health > 0 && row.actor.health == 0;
                    }
                }
            }
        }
        if killed {
            if self.resume_suspended(frame) {
                return;
            }
            self.finish(CombatEnd::Killed, tick);
            return;
        }
        if self.engaged.is_some() && !present {
            if self.flags & LOST == 0 {
                self.lost_since = tick;
                self.flags |= LOST;
            } else if elapsed(tick, self.lost_since) >= 3 && !self.resume_suspended(frame) {
                self.finish(CombatEnd::TargetGone, tick);
            }
            return;
        }
        if present {
            self.flags &= !(LOST | LEASH_WALKED);
        }
        if self.engaged.is_none() {
            if self.flags & LOST == 0 {
                self.lost_since = tick;
                self.flags |= LOST;
            } else if elapsed(tick, self.lost_since) >= 15 {
                self.finish(CombatEnd::NoTarget, tick);
            }
        }
        if self.phase == Phase::Engage
            && self.flags & ENGAGEMENT_ATTACK != 0
            && self.engaged.is_some_and(|actor| {
                frame
                    .in_combat
                    .target
                    .is_some_and(|target| actor.matches(target))
                    || self.threats.iter(tick).any(|threat| threat.actor == actor)
            })
        {
            self.phase = Phase::Fight;
        }
        if matches!(self.request.target, Target::Npc { .. })
            && self.request.intruder.player == IntruderRule::FightBack
            && self
                .engaged
                .is_some_and(|actor| actor.kind == ActorKind::Npc)
        {
            let player = self
                .threats
                .iter(tick)
                .find(|threat| threat.actor.kind == ActorKind::Player)
                .map(|threat| threat.actor);
            if let Some(actor) = player {
                self.suspended = self.engaged;
                self.suspended_type = self.engaged_type;
                self.engage(actor, frame);
                self.counters.intruders = self.counters.intruders.saturating_add(1);
                self.phase = Phase::Fight;
            }
        }
    }
    fn resume_suspended(&mut self, frame: &Frame<'_>) -> bool {
        let Some(actor) = self.suspended.take() else {
            return false;
        };
        if frame.npcs.iter().any(|row| {
            row.index == usize::from(actor.index)
                && row.r#type == usize::try_from(self.suspended_type).ok()
                && !(row.total_health > 0 && row.health == 0)
                && row
                    .actions
                    .iter()
                    .flatten()
                    .any(|action| action == "Attack")
        }) {
            self.engage(actor, frame);
            self.phase = Phase::Fight;
            true
        } else {
            false
        }
    }
    fn advance_eat_ready(&mut self, tick: u16) {
        let ready = tick.wrapping_add(2);
        if self.flags & OBSERVED == 0 || reached(ready, self.eat_ready) {
            self.eat_ready = ready;
        }
    }
    fn pending_row(&self, kind: RowKind) -> bool {
        self.pending.iter().any(|pending| pending.row.kind == kind)
    }
    fn pending_id(&self, kind: RowKind, id: i32) -> bool {
        self.pending
            .iter()
            .any(|pending| pending.row.kind == kind && pending.row.id == id)
    }
    fn effect(&self, row: PlanRow) -> super::schedule::InputEffect<'_> {
        match row.kind {
            RowKind::Eat => super::schedule::InputEffect::Food(
                self.tables.food(row.id).expect("planned food fact"),
            ),
            RowKind::Prayer if row.aux != 0 => super::schedule::InputEffect::PrayerOn,
            RowKind::Wear
                if self
                    .tables
                    .selected()
                    .item_by_alias("elemental_shield")
                    .is_some_and(|item| item.id == row.id) =>
            {
                super::schedule::InputEffect::ElementalShield
            }
            _ => super::schedule::InputEffect::Standard,
        }
    }
    fn projected_schedule(&self) -> Schedule {
        let mut schedule = self.schedule;
        if self.plan_first_id != 0 {
            for row in self.plan.iter() {
                schedule.admitted(
                    op_kind(row.kind),
                    self.emitted_tick,
                    self.plan_rate(),
                    self.flags & PLAN_FIGHT != 0,
                    self.effect(row),
                );
                if row.kind == RowKind::Attack
                    && schedule.clear_valid
                    && schedule.last_clear == self.emitted_tick
                {
                    schedule.restore_due = self.emitted_tick;
                }
            }
        }
        schedule
    }
    fn plan_rate(&self) -> u8 {
        self.plan
            .iter()
            .find(|row| row.kind == RowKind::Attack)
            .map_or(4, |row| row.aux)
    }
    fn prayer_varp(&self, button: i32) -> Option<i32> {
        self.tables
            .selected()
            .prayers()
            .iter()
            .find(|row| row.button_com == button)
            .map(|row| row.varp)
    }
    fn receipts(&mut self, cx: &ActionContext<'_>) -> bool {
        if self.plan_first_id == 0 {
            return true;
        }
        let mut accepted = 0;
        for index in 0..u64::from(self.plan.len) {
            let Some(receipt) = cx.interaction_receipt(self.plan_first_id + index) else {
                return false;
            };
            if receipt.accepted && accepted == index {
                accepted += 1;
            }
        }
        let fight = self.flags & PLAN_FIGHT != 0;
        let rate = self.plan_rate();
        let mut stage = self.staged_pending;
        for index in 0..usize::from(self.plan.len) {
            let slot = stage.trailing_zeros() as usize;
            stage &= !(1 << slot);
            let row = self.plan.rows[index];
            if index >= accepted as usize {
                self.pending[slot] = PendingRow::default();
                continue;
            }
            if row.kind != RowKind::Attack {
                // A newly accepted clearing input cancels any previous
                // installed/pending interaction, but not this plan's terminal.
                for (slot, pending) in self.pending.iter_mut().enumerate() {
                    if self.staged_pending & (1 << slot) == 0 && pending.row.kind == RowKind::Attack
                    {
                        *pending = PendingRow::default();
                    }
                }
            }
            if fight && row.kind != RowKind::Attack && self.flags & RESTORE_COUNTED == 0 {
                self.counters.restorations = self.counters.restorations.saturating_add(1);
                self.flags |= RESTORE_COUNTED;
            }
            let effect = match row.kind {
                RowKind::Eat => super::schedule::InputEffect::Food(
                    self.tables.food(row.id).expect("planned food fact"),
                ),
                RowKind::Prayer if row.aux != 0 => super::schedule::InputEffect::PrayerOn,
                RowKind::Wear
                    if self
                        .tables
                        .selected()
                        .item_by_alias("elemental_shield")
                        .is_some_and(|item| item.id == row.id) =>
                {
                    super::schedule::InputEffect::ElementalShield
                }
                _ => super::schedule::InputEffect::Standard,
            };
            self.schedule
                .admitted(op_kind(row.kind), self.emitted_tick, rate, fight, effect);
            match row.kind {
                RowKind::Prayer => {
                    let varp = self.prayer_varp(row.id).expect("planned prayer component");
                    let displaced = self.prayer_displaced(varp, row.aux != 0);
                    self.baseline_on &= !(displaced | prayer_bit(varp));
                    self.raised_prayers.accepted(varp, row.aux != 0, displaced);
                    if row.aux != 0 {
                        let bit = (varp - api::prayer::PRAYER_VARP0) as usize;
                        self.prayer_ready[bit] = self.emitted_tick.wrapping_add(3) as u8;
                        self.prayer_ready_mask |= 1 << bit;
                        if (0..3)
                            .filter_map(|tier| self.tables.prayer(PrayerRole::Protect, tier))
                            .any(|fact| fact.varp == varp)
                        {
                            self.counters.switches = self.counters.switches.saturating_add(1);
                        }
                    } else if self.phase == Phase::WindDown {
                        if let Some((index, fact)) = self
                            .tables
                            .selected()
                            .prayers()
                            .iter()
                            .enumerate()
                            .find(|(_, fact)| fact.varp == varp)
                        {
                            self.sweep.accepted(super::prayer::SweepClick {
                                index,
                                varp,
                                button_com: fact.button_com,
                            });
                        }
                    }
                }
                RowKind::Wear if !fight => self.prep_failures |= 4 << (u32::from(row.aux) * 3),
                RowKind::Retaliate if !fight => self.flags |= PREP_RETALIATE,
                RowKind::Drink => {
                    if !fight {
                        self.prep_failures |= 1 << (POTION_PREP_SHIFT + u32::from(row.aux));
                    }
                    if potion_kind(row.aux) == PotionKind::Antifire {
                        self.antifire_sip = self.emitted_tick;
                    }
                }
                RowKind::Style => {
                    if let Some(mode) = MeleeMode::from_code((row.aux >> 2) & 3) {
                        self.mode_fallback =
                            (Some(mode) != self.request.melee_mode).then_some(mode);
                    }
                }
                RowKind::Attack => {
                    self.flags |= ENGAGEMENT_ATTACK;
                    self.flags &= !RESTORE_COUNTED;
                    if self.schedule.clear_valid && self.schedule.last_clear == self.emitted_tick {
                        self.schedule.restore_due = self.emitted_tick;
                    }
                }
                _ => {}
            }
        }
        self.plan_first_id = 0;
        self.staged_pending = 0;
        true
    }
    fn settle(&mut self, frame: &Frame<'_>, tick: u16) {
        let total = frame
            .inventory
            .iter()
            .filter(|item| {
                self.tables
                    .food(item.def.id)
                    .is_some_and(|food| food.eat_delay_arg.is_some())
            })
            .fold(0u16, |total, item| {
                total.saturating_add(item.count.clamp(0, i32::from(u16::MAX)) as u16)
            });
        if total < self.food_total {
            self.advance_eat_ready(tick);
        }
        self.food_total = total;
        let timed_out = self
            .pending
            .iter()
            .filter(|pending| {
                pending.row.kind == RowKind::Prayer && pending.row.aux == 0 && pending.age >= 3
            })
            .filter_map(|pending| self.prayer_varp(pending.row.id))
            .fold(0u32, |mask, varp| {
                mask | (1 << (varp - api::prayer::PRAYER_VARP0))
            });
        let observation = prayer_observation(frame);
        self.sweep.observe(&observation, timed_out);
        for row in self.tables.selected().prayers() {
            if observation.is_off(row.varp) && !self.pending_id(RowKind::Prayer, row.button_com) {
                self.raised_prayers.accepted(row.varp, false, 0);
            }
        }
        if super::style::melee::onset(
            &frame.local.player.actor,
            self.engaged,
            &self.tables,
            &mut self.anim_id,
            &mut self.anim_frame,
        ) && (!self.schedule.clear_valid
            || (reached(tick, self.schedule.last_clear) && tick != self.schedule.last_clear))
        {
            self.counters.swings = self.counters.swings.saturating_add(1);
            self.schedule.observe_swing(tick, self.rate(frame));
        }
        for slot in 0..self.pending.len() {
            let pending = self.pending[slot];
            let row = pending.row;
            if row.kind == RowKind::Empty {
                continue;
            }
            let kind = op_kind(row.kind);
            let settled =
                match row.kind {
                    RowKind::Eat => arbiter::count(frame, row.id) < i32::from(pending.baseline),
                    RowKind::Drink => {
                        arbiter::doses(frame, &self.tables, potion_kind(row.aux)) < pending.baseline
                    }
                    RowKind::Prayer => self.prayer_varp(row.id).is_some_and(|varp| {
                        frame.varps.iter().any(|value| {
                            value.index == varp && value.value == i32::from(row.aux != 0)
                        })
                    }),
                    RowKind::Wear => frame
                        .equipment
                        .iter()
                        .any(|item| item.slot == i32::from(row.aux) && item.def.id == row.id),
                    RowKind::Style => self.tables.melee_mode_varp().is_some_and(|varp| {
                        frame.varps.iter().any(|value| {
                            value.index == varp && value.value == i32::from(row.aux & 3)
                        })
                    }),
                    RowKind::Retaliate => frame.varps.iter().any(|value| {
                        value.index == super::OPTION_NODEF && value.value == i32::from(row.aux == 0)
                    }),
                    RowKind::Attack => {
                        self.engaged
                            .is_some_and(|actor| actor_token(actor) == row.id)
                            && (self.schedule.interaction == Interaction::Installed
                                || self.engaged.is_some_and(|actor| {
                                    frame
                                        .in_combat
                                        .target
                                        .is_some_and(|target| actor.matches(target))
                                }))
                            && (!self.schedule.clear_valid
                                || (reached(tick, self.schedule.last_clear)
                                    && tick != self.schedule.last_clear))
                    }
                    RowKind::Empty => false,
                };
            if settled {
                self.pending[slot] = PendingRow::default();
                match row.kind {
                    RowKind::Eat => self.counters.food = self.counters.food.saturating_add(1),
                    RowKind::Drink => match potion_kind(row.aux) {
                        PotionKind::Prayer => {
                            self.counters.prayer = self.counters.prayer.saturating_add(1)
                        }
                        PotionKind::Antifire => {
                            self.counters.antifire = self.counters.antifire.saturating_add(1);
                            self.flags |= ANTIFIRE;
                        }
                        _ => self.counters.boost = self.counters.boost.saturating_add(1),
                    },
                    RowKind::Attack => self.schedule.interaction = Interaction::Installed,
                    RowKind::Wear => self.prep_failures &= !(7 << (u32::from(row.aux) * 3)),
                    RowKind::Retaliate => self.flags &= !PREP_RETALIATE,
                    _ => {}
                }
                if !self.pending_row(row.kind) {
                    self.schedule.settle(kind);
                }
            } else {
                self.pending[slot].age = pending.age.saturating_add(1);
                let window = match row.kind {
                    RowKind::Eat => 6,
                    RowKind::Drink | RowKind::Wear | RowKind::Retaliate | RowKind::Style => 4,
                    RowKind::Prayer if row.aux == 0 => 4,
                    RowKind::Prayer | RowKind::Attack => 8,
                    RowKind::Empty => unreachable!("pending row"),
                };
                if self.pending[slot].age < window {
                    continue;
                }
                self.pending[slot] = PendingRow::default();
                if row.kind == RowKind::Prayer && row.aux != 0 {
                    if let Some(varp) = self.prayer_varp(row.id) {
                        if observation.is_off(varp) {
                            self.raised_prayers.accepted(varp, false, 0);
                        }
                    }
                }
                if !self.pending_row(row.kind) {
                    self.schedule.timeout(kind);
                }
                let failures = self.schedule.unsettled[kind.index()];
                match row.kind {
                    RowKind::Wear => {
                        let shift = u32::from(row.aux) * 3;
                        let count = (((self.prep_failures >> shift) & 3) + 1).min(3);
                        self.prep_failures =
                            (self.prep_failures & !(3 << shift)) | (count << shift);
                        if count >= 2 && row.aux == 3 {
                            self.flags |= FAILED_WEAPON;
                        } else if count >= 2 && row.aux == 5 && self.flags & SHIELD_OVERRIDE != 0 {
                            self.flags |= FAILED_SHIELD;
                        } else if count >= 3 && !self.wear_skipped(row.aux) {
                            self.flags |= FAILED_ATTACK;
                        }
                    }
                    RowKind::Drink
                        if failures >= 2
                            && (1..=5).contains(&row.aux)
                            && self.prep_failures
                                & (1 << (POTION_PREP_SHIFT + u32::from(row.aux)))
                                != 0 =>
                    {
                        self.prep_failures |= 1 << (POTION_SKIP_SHIFT + u32::from(row.aux));
                    }
                    RowKind::Retaliate if failures >= 2 && self.flags & PREP_RETALIATE != 0 => {
                        self.flags |= SKIP_RETALIATE
                    }
                    _ if failures >= 3 => self.flags |= FAILED_ATTACK,
                    _ => {}
                }
            }
        }
    }
    fn shield(&self, frame: &Frame<'_>) -> bool {
        self.tables
            .selected()
            .item_by_alias("antidragonbreathshield")
            .is_some_and(|item| {
                frame
                    .equipment
                    .iter()
                    .any(|row| row.slot == 5 && row.def.id == item.id)
            })
    }
    fn antifire(&self, tick: u16) -> bool {
        self.flags & ANTIFIRE != 0 && elapsed(tick, self.antifire_sip) < 600
    }
    fn safety_end(
        &mut self,
        frame: &Frame<'_>,
        tick: u16,
        danger: Option<i32>,
        emergency: i32,
        food_available: bool,
    ) {
        if self.phase == Phase::Escape || self.phase == Phase::WindDown {
            return;
        }
        let no_food =
            danger != Some(0) && arbiter::stat(frame, 3).0 <= emergency && !food_available;
        let no_fire = self.flags & SHIELD_OVERRIDE != 0
            && !self.shield(frame)
            && !self.antifire(tick)
            && (!self.request.allow.equipment
                || self
                    .tables
                    .selected()
                    .item_by_alias("antidragonbreathshield")
                    .is_none_or(|item| arbiter::count(frame, item.id) == 0))
            && (!self.request.allow.potions
                || arbiter::potion_id(frame, &self.tables, PotionKind::Antifire).is_none());
        if no_food || no_fire {
            let reason = AbortReason::Unprotected(if no_fire {
                Unprotected::Dragonfire
            } else {
                Unprotected::NoFood
            });
            match self.request.fallback {
                Fallback::Retreat { .. } => {
                    self.phase = Phase::Escape;
                    self.escape_since = tick;
                    self.schedule.terminal();
                }
                Fallback::Fight if !no_fire => {}
                _ => self.finish(CombatEnd::Aborted(reason), tick),
            }
        }
    }
    fn push(&self, plan: &mut TickPlan, row: PlanRow) -> bool {
        let free = self
            .pending
            .iter()
            .filter(|pending| pending.row.kind == RowKind::Empty)
            .count();
        let reserve =
            if matches!(self.phase, Phase::Engage | Phase::Fight) && !row.terminal(&self.tables) {
                2
            } else {
                0
            };
        // A clearing row also reserves server-effect storage for its Attack.
        if usize::from(plan.len) + 1 + usize::from(reserve != 0) > free {
            return false;
        }
        plan.push(row, reserve, &self.tables)
    }
    fn emergency_food_available(&self, frame: &Frame<'_>, danger: Option<i32>) -> bool {
        if !self.request.allow.food {
            return false;
        }
        if arbiter::food_by(frame, &self.tables, None, |food| {
            food.eat_delay_arg.is_some()
        })
        .is_some()
        {
            return true;
        }

        let (hp, max) = arbiter::stat(frame, 3);
        arbiter::food_by(frame, &self.tables, None, |food| {
            let Some(delay) = food.message_delay else {
                return false;
            };
            let gate = danger.map_or((max + 1) / 2, |danger| {
                danger.saturating_mul(delay + 2).saturating_add(1)
            });
            food.eat_delay_arg.is_none() && hp.saturating_add(food.heal).min(max) > gate
        })
        .is_some()
    }
    fn plain_food(&self, frame: &Frame<'_>, tick: u16, gate: Option<i32>) -> Option<i32> {
        if !self.request.allow.food || self.pending_row(RowKind::Eat) {
            return None;
        }
        arbiter::food_by(frame, &self.tables, gate, |food| {
            food.eat_delay_arg.is_some() && self.schedule.food_ready(food, tick)
        })
    }
    fn eat(&self, plan: &mut TickPlan, frame: &Frame<'_>, tick: u16, danger: Option<i32>) {
        if plan.contains(RowKind::Eat)
            || self.pending_row(RowKind::Eat)
            || !self.request.allow.food
            || self.schedule.unsettled[OpKind::Eat.index()] >= 3
        {
            return;
        }
        let (hp, max) = arbiter::stat(frame, 3);
        let ordinary = self.plain_food(frame, tick, None);
        if let Some(id) = ordinary {
            if !self.push(plan, PlanRow::new(RowKind::Eat, id, 0)) {
                return;
            }
        }
        let heal = ordinary
            .and_then(|id| self.tables.food(id))
            .map_or(0, |food| food.heal);
        let plain_insufficient =
            hp.saturating_add(heal).min(max) <= select::lines(danger, max).drink_gate;
        if ordinary.is_some() && (!plain_insufficient || !reached(tick, self.eat_ready)) {
            return;
        }
        let combo = arbiter::food_by(frame, &self.tables, None, |food| {
            let Some(delay) = food.message_delay else {
                return false;
            };
            let gate = danger.map_or((max + 1) / 2, |danger| {
                danger.saturating_mul(delay + 2).saturating_add(1)
            });
            food.eat_delay_arg.is_none()
                && hp.saturating_add(heal).saturating_add(food.heal).min(max) > gate
        });
        if let Some(id) = combo {
            self.push(plan, PlanRow::new(RowKind::Eat, id, 0));
        }
    }
    fn drink_or_recover(
        &self,
        plan: &mut TickPlan,
        kind: PotionKind,
        lowers_danger: bool,
        frame: &Frame<'_>,
        tick: u16,
        gate: i32,
    ) {
        if plan.closed(&self.tables)
            || !self.request.allow.potions
            || (kind == PotionKind::Prayer && !self.request.allow.prayer)
            || !self.schedule.ready(OpKind::Drink, tick)
            || self.pending_row(RowKind::Drink)
            || self.prep_failures & (1 << (POTION_SKIP_SHIFT + kind as u32)) != 0
        {
            return;
        }
        let Some(id) = arbiter::potion_id(frame, &self.tables, kind) else {
            return;
        };
        let (hp, max) = arbiter::stat(frame, 3);
        let eat = plan.iter().find(|row| row.kind == RowKind::Eat);
        let projected = if reached(tick, self.eat_ready) {
            hp.saturating_add(
                eat.and_then(|row| self.tables.food(row.id))
                    .map_or(0, |food| food.heal),
            )
            .min(max)
        } else {
            hp
        };
        if projected > gate {
            self.push(plan, PlanRow::new(RowKind::Drink, id, kind as u8));
            return;
        }
        if !lowers_danger || eat.is_some() {
            return;
        }
        if let Some(food) = self.plain_food(frame, tick, Some(gate)) {
            if self.push(plan, PlanRow::new(RowKind::Eat, food, 0)) && reached(tick, self.eat_ready)
            {
                self.push(plan, PlanRow::new(RowKind::Drink, id, kind as u8));
            }
        }
    }
    fn advance_prayer_ready(&mut self, tick: u16) {
        if self.flags & OBSERVED != 0 && elapsed(tick, self.last_tick) >= 128 {
            self.prayer_ready_mask = 0;
            return;
        }
        let mut pending = self.prayer_ready_mask;
        while pending != 0 {
            let bit = pending.trailing_zeros() as usize;
            let mask = 1 << bit;
            pending &= !mask;
            if (tick as u8).wrapping_sub(self.prayer_ready[bit]) as i8 >= 0 {
                self.prayer_ready_mask &= !mask;
            }
        }
    }
    fn prayer_ready(&self, varp: i32, tick: u16) -> bool {
        let bit = (varp - api::prayer::PRAYER_VARP0) as usize;
        bit < 15
            && (self.prayer_ready_mask & (1 << bit) == 0
                || (tick as u8).wrapping_sub(self.prayer_ready[bit]) as i8 >= 0)
    }
    fn project_prayer(&self, mask: &mut u16, varp: i32, on: bool) {
        if on {
            for role in [
                PrayerRole::Protect,
                PrayerRole::Strength,
                PrayerRole::Attack,
            ] {
                if (0..3)
                    .filter_map(|tier| self.tables.prayer(role, tier))
                    .any(|row| row.varp == varp)
                {
                    for row in (0..3).filter_map(|tier| self.tables.prayer(role, tier)) {
                        *mask &= !(1 << (row.varp - api::prayer::PRAYER_VARP0));
                    }
                }
            }
            *mask |= 1 << (varp - api::prayer::PRAYER_VARP0);
        } else {
            *mask &= !(1 << (varp - api::prayer::PRAYER_VARP0));
        }
    }
    fn capture_prayer_baseline(&mut self, observation: &PrayerObservation) {
        if self.baseline_on & BASELINE_READY != 0 {
            return;
        }
        if (0..api::prayer::PRAYER_COUNT)
            .all(|index| observation.varp_observed(api::prayer::PRAYER_VARP0 + index as i32))
        {
            self.baseline_on = BASELINE_READY
                | (0..api::prayer::PRAYER_COUNT).fold(0, |mask, index| {
                    let varp = api::prayer::PRAYER_VARP0 + index as i32;
                    mask | if observation.is_on(varp) {
                        prayer_bit(varp)
                    } else {
                        0
                    }
                });
        }
    }
    fn prayer_displaced(&self, varp: i32, on: bool) -> u16 {
        if !on {
            return 0;
        }
        let mut projected = BASELINE_READY - 1;
        self.project_prayer(&mut projected, varp, true);
        (BASELINE_READY - 1) & !projected
    }
    pub(crate) fn prayer_plan_first_id(&self) -> u64 {
        self.plan_first_id
    }
    /// Include accepted dispatches that a cancellation interrupted before poll.
    /// Planned, queued, missing and refused receipts confer no ownership.
    pub(crate) fn prayer_cleanup(&self, accepted_prefix: usize) -> RaisedPrayers {
        let mut raised = self.raised_prayers;
        if self.plan_first_id != 0 {
            for row in self.plan.iter().take(accepted_prefix) {
                if row.kind == RowKind::Prayer {
                    let varp = self.prayer_varp(row.id).expect("planned prayer component");
                    raised.accepted(
                        varp,
                        row.aux != 0,
                        self.prayer_displaced(varp, row.aux != 0),
                    );
                }
            }
        }
        raised
    }
    fn offensive_prayer(
        &self,
        role: PrayerRole,
        base: i32,
        observation: &PrayerObservation,
    ) -> Option<&api::game_data::PrayerFact> {
        // A user's tier wins even when Combat can afford a stronger one.
        (0..3)
            .rev()
            .filter_map(|tier| self.tables.prayer(role, tier))
            .find(|row| {
                self.baseline_on & prayer_bit(row.varp) != 0
                    || (observation.is_on(row.varp) && !self.raised_prayers.contains(row.varp))
            })
            .or_else(|| {
                (0..3)
                    .rev()
                    .filter_map(|tier| self.tables.prayer(role, tier))
                    .find(|row| base >= row.level)
            })
    }
    fn plan(
        &mut self,
        frame: &Frame<'_>,
        tick: u16,
        cx: &mut ActionContext<'_>,
    ) -> Result<TickPlan, ActionError> {
        let mut plan = TickPlan::default();
        let (hp, hp_max) = arbiter::stat(frame, 3);
        let mut danger = self.threats.danger(
            frame,
            &self.tables,
            tick,
            self.shield(frame),
            self.antifire(tick),
        );
        let mut lines = select::lines(danger, hp_max);
        let food_available = self.emergency_food_available(frame, danger);
        self.safety_end(frame, tick, danger, lines.emergency, food_available);
        if self.phase == Phase::WindDown {
            let obs = prayer_observation(frame);
            if self.baseline_on & BASELINE_READY == 0 {
                return Ok(plan);
            }
            for click in self
                .sweep
                .candidates(self.tables.selected(), &obs)
                .filter(|click| self.raised_prayers.contains(click.varp))
            {
                if !self.pending_id(RowKind::Prayer, click.button_com) {
                    self.push(
                        &mut plan,
                        PlanRow::new(RowKind::Prayer, click.button_com, 0),
                    );
                }
            }
            if plan.len == 0
                && self.flags & TERMINAL_FOOD == 0
                && hp > 0
                && danger != Some(0)
                && hp <= lines.eat
            {
                self.eat(&mut plan, frame, tick, danger);
            }
            return Ok(plan);
        }
        if self.phase == Phase::Escape {
            let Fallback::Retreat { tile } = self.request.fallback else {
                unreachable!("escape requires retreat");
            };
            if self
                .pending_walk
                .and_then(|id| cx.walk_receipt(id.get()))
                .is_some()
                || elapsed(tick, self.escape_since) >= 60
            {
                self.finish(CombatEnd::Aborted(AbortReason::RetreatFailed), tick);
                return self.plan(frame, tick, cx);
            }
            if self.request.allow.retaliate_toggle && retaliate_on(frame) {
                if !self.pending_row(RowKind::Retaliate)
                    && self.schedule.ready(OpKind::Retaliate, tick)
                {
                    self.push(&mut plan, PlanRow::new(RowKind::Retaliate, 0, 0));
                }
                return Ok(plan);
            }
            if hp > 0 && danger != Some(0) && hp <= lines.eat {
                self.eat(&mut plan, frame, tick, danger);
            }
            if plan.len == 0 && self.pending_walk.is_none() {
                self.walk(tile, tick, cx)?;
            }
            return Ok(plan);
        }
        // Emergency uses the original danger, before free reducers project it.
        if danger != Some(0) && hp <= lines.emergency {
            self.eat(&mut plan, frame, tick, danger);
        }
        if plan.closed(&self.tables) {
            return Ok(plan);
        }
        let (points, base) = arbiter::stat(frame, 5);
        let prayer_allowed = self.baseline_on & BASELINE_READY != 0 && self.request.allow.prayer;
        let wanted = prayer_allowed
            .then(|| {
                policy::wanted_protect(
                    &self.threats,
                    frame,
                    &self.tables,
                    tick,
                    self.shield(frame),
                    self.antifire(tick),
                )
            })
            .flatten();
        let observation = prayer_observation(frame);
        let mut prayers = frame
            .varps
            .iter()
            .filter(|row| {
                row.value == 1
                    && (api::prayer::PRAYER_VARP0..api::prayer::PRAYER_VARP0 + 15)
                        .contains(&row.index)
            })
            .fold(0u16, |mask, row| {
                mask | (1 << (row.index - api::prayer::PRAYER_VARP0))
            });
        if let Some(protect) =
            wanted.filter(|row| base >= row.level && observation.is_off(row.varp))
        {
            if points == 0 {
                self.drink_or_recover(
                    &mut plan,
                    PotionKind::Prayer,
                    true,
                    frame,
                    tick,
                    lines.drink_gate,
                );
            }
            if points > 0
                && self.prayer_ready(protect.varp, tick)
                && !self.pending_id(RowKind::Prayer, protect.button_com)
                && self.push(
                    &mut plan,
                    PlanRow::new(RowKind::Prayer, protect.button_com, 1),
                )
            {
                self.project_prayer(&mut prayers, protect.varp, true);
                let style = (0..3)
                    .find(|tier| {
                        self.tables
                            .prayer(PrayerRole::Protect, *tier)
                            .is_some_and(|row| row.varp == protect.varp)
                    })
                    .map(|tier| match tier {
                        0 => StyleObs::Magic,
                        1 => StyleObs::Ranged,
                        _ => StyleObs::Melee,
                    });
                danger = self.threats.danger_with(
                    &self.tables,
                    tick,
                    self.shield(frame),
                    self.antifire(tick),
                    style,
                );
                lines = select::lines(danger, hp_max);
            }
        }
        let wanted_varp = wanted.map(|row| row.varp);
        let protection_wanted = wanted_varp.is_some();
        if !plan.closed(&self.tables) && danger != Some(0) && hp <= lines.eat {
            self.eat(&mut plan, frame, tick, danger);
        }
        if plan.closed(&self.tables) {
            return Ok(plan);
        }
        // A sticky target beyond lost_radius is dropped after a grace; interrupt
        // the server's queued chase before the terminal report (design-combat.md
        // §1.1, §2.6; combat-s3a-ReviewCombatFable.md F5).
        if self.flags & LOST != 0 && self.engaged.is_some() && self.phase != Phase::WindDown {
            if plan.len == 0 && self.flags & LEASH_WALKED == 0 {
                if let Some(destination) = self.leash_cancel_tile(frame) {
                    self.walk(destination, tick, cx)?;
                    self.flags |= LEASH_WALKED;
                    return Ok(plan);
                }
            }
            return Ok(plan);
        }
        if self.phase == Phase::Prep {
            let weapon = self.desired(3);
            let conflict = self.flags & SHIELD_OVERRIDE != 0
                && weapon.is_some_and(|id| {
                    self.tables
                        .selected()
                        .item_by_id(id)
                        .is_some_and(|item| item.is_two_handed())
                });
            let missing = self.request.allow.equipment
                && weapon.is_some_and(|id| {
                    arbiter::count(frame, id) == 0
                        && !frame
                            .equipment
                            .iter()
                            .any(|row| row.slot == 3 && row.def.id == id)
                });
            if conflict || missing {
                self.finish(
                    CombatEnd::Aborted(AbortReason::PrepFailed(if conflict {
                        PrepItem::Shield
                    } else {
                        PrepItem::Weapon
                    })),
                    tick,
                );
                return self.plan(frame, tick, cx);
            }
            self.wear(&mut plan, frame, tick);
            self.style(&mut plan, frame, tick);
            self.retaliate(&mut plan, frame, tick);
            if self.flags & SHIELD_OVERRIDE != 0
                && (!self.antifire(tick) || elapsed(tick, self.antifire_sip) >= 590)
            {
                self.drink_or_recover(
                    &mut plan,
                    PotionKind::Antifire,
                    true,
                    frame,
                    tick,
                    lines.drink_gate,
                );
            }
            if self.flags & BOOST_WORTH != 0 {
                if let Some(kind) = self.boost(frame) {
                    self.drink_or_recover(&mut plan, kind, false, frame, tick, lines.drink_gate);
                }
            }
            if plan.closed(&self.tables)
                || self.pending_row(RowKind::Wear)
                || self.pending_row(RowKind::Drink)
            {
                return Ok(plan);
            }
            // Only completed or in-plan equipment permits engagement.
            let missing_wear = (0..14).any(|slot| {
                self.desired(slot).is_some_and(|id| {
                    !frame
                        .equipment
                        .iter()
                        .any(|row| row.slot == i32::from(slot) && row.def.id == id)
                        && !self.wear_skipped(slot)
                        && arbiter::count(frame, id) > 0
                        && !plan
                            .iter()
                            .any(|row| row.kind == RowKind::Wear && row.aux == slot && row.id == id)
                })
            });
            if missing_wear && self.request.allow.equipment {
                return Ok(plan);
            }
            self.phase = Phase::Engage;
        }
        if self.phase == Phase::Engage {
            self.style(&mut plan, frame, tick);
            self.retaliate(&mut plan, frame, tick);
            if self.engaged.is_some()
                && (plan.len != 0
                    || (!self.pending_row(RowKind::Attack)
                        && self.schedule.ready(OpKind::Attack, tick)))
            {
                let attack = self.attack_row(&plan, frame);
                self.push(&mut plan, attack);
            } else if plan.len == 0 && self.engaged.is_none() && self.pending_walk.is_none() {
                if let Some(tile) = self
                    .request
                    .stand
                    .filter(|tile| distance(frame.here, *tile) > 1)
                {
                    self.walk(tile, tick, cx)?;
                }
            }
            return Ok(plan);
        }
        if self.request.allow.prayer
            && (protection_wanted || prayers != 0)
            && policy::prayer_sip_due(points, base)
        {
            self.drink_or_recover(
                &mut plan,
                PotionKind::Prayer,
                true,
                frame,
                tick,
                lines.drink_gate,
            );
        }
        let cycle_room = !self.schedule.cycle.known
            || !reached(tick.wrapping_add(2), self.schedule.cycle.deadline);
        if cycle_room && self.flags & BOOST_WORTH != 0 && self.target_above_quarter(frame) {
            if let Some(kind) = self.boost(frame) {
                self.drink_or_recover(&mut plan, kind, false, frame, tick, lines.drink_gate);
            }
        }
        if cycle_room
            && self.flags & SHIELD_OVERRIDE != 0
            && (!self.antifire(tick) || elapsed(tick, self.antifire_sip) >= 590)
        {
            self.drink_or_recover(
                &mut plan,
                PotionKind::Antifire,
                true,
                frame,
                tick,
                lines.drink_gate,
            );
        }
        if plan.closed(&self.tables) {
            return Ok(plan);
        }
        let offense = self.flags & BOOST_WORTH != 0
            && prayer_allowed
            && (frame.local.player.actor.in_combat || self.threats.iter(tick).next().is_some())
            && points > 0
            && (points > policy::prayer_sip_floor(base)
                || arbiter::potion_id(frame, &self.tables, PotionKind::Prayer).is_some());
        let offensives = if offense {
            [
                self.offensive_prayer(PrayerRole::Strength, base, &observation),
                self.offensive_prayer(PrayerRole::Attack, base, &observation),
            ]
        } else {
            [None, None]
        };
        for row in offensives.into_iter().flatten() {
            if prayers & prayer_bit(row.varp) == 0
                && observation.is_off(row.varp)
                && self.prayer_ready(row.varp, tick)
                && !self.pending_id(RowKind::Prayer, row.button_com)
                && self.push(&mut plan, PlanRow::new(RowKind::Prayer, row.button_com, 1))
            {
                self.project_prayer(&mut prayers, row.varp, true);
            }
        }
        for row in self.tables.selected().prayers() {
            if prayers & (1 << (row.varp - api::prayer::PRAYER_VARP0)) == 0
                || !self.raised_prayers.contains(row.varp)
                || self.pending_id(RowKind::Prayer, row.button_com)
            {
                continue;
            }
            let keep = prayer_allowed
                && (wanted_varp == Some(row.varp)
                    || offensives
                        .iter()
                        .flatten()
                        .any(|best| best.varp == row.varp));
            if !keep && self.push(&mut plan, PlanRow::new(RowKind::Prayer, row.button_com, 0)) {
                self.project_prayer(&mut prayers, row.varp, false);
            }
        }
        self.wear(&mut plan, frame, tick);
        self.style(&mut plan, frame, tick);
        self.retaliate(&mut plan, frame, tick);
        let mismatch = self.engaged.is_some_and(|actor| {
            frame
                .in_combat
                .target
                .is_some_and(|target| !actor.matches(target))
        });
        if !plan.closed(&self.tables)
            && self.engaged.is_some()
            && (plan.len != 0
                || (!self.pending_row(RowKind::Attack)
                    && (self.schedule.restore_ready(tick)
                        || mismatch
                        || self.schedule.stale_ready(tick, self.rate(frame)))))
        {
            let attack = self.attack_row(&plan, frame);
            self.push(&mut plan, attack);
        }
        Ok(plan)
    }
    fn attack_row(&self, plan: &TickPlan, frame: &Frame<'_>) -> PlanRow {
        let rate = plan
            .iter()
            .find(|row| row.kind == RowKind::Wear && row.aux == 3)
            .and_then(|row| self.tables.weapon_style(row.id))
            .map_or_else(|| self.rate(frame), |weapon| weapon.attackrate.max(1));
        PlanRow::new(
            RowKind::Attack,
            actor_token(self.engaged.expect("planned engaged actor")),
            rate,
        )
    }
    fn boost(&self, frame: &Frame<'_>) -> Option<PotionKind> {
        if !self.request.allow.potions {
            return None;
        }
        [
            PotionKind::SuperAttack,
            PotionKind::SuperStrength,
            PotionKind::SuperDefence,
        ]
        .into_iter()
        .find(|kind| {
            let Some(family) = self.tables.potion(*kind) else {
                return false;
            };
            let Some(stat) = family.stat else {
                return false;
            };
            let (effective, base) = arbiter::stat(frame, stat.index() as i32);
            let ceiling = base + family.constant + base * family.percent / 100;
            crate::boost_potions::boost_faded(
                f64::from(base),
                f64::from(effective),
                crate::boost_potions::BOOST_FLOOR,
            ) && effective < ceiling
                && arbiter::potion_id(frame, &self.tables, *kind).is_some()
        })
    }
    fn desired(&self, slot: u8) -> Option<i32> {
        if slot == 5 && self.flags & SHIELD_OVERRIDE != 0 {
            self.tables
                .selected()
                .item_by_alias("antidragonbreathshield")
                .map(|row| row.id)
        } else if let Some(kit) = &self.request.kit {
            kit.worn
                .iter()
                .find(|(wanted, _)| *wanted == slot)
                .map(|(_, id)| *id)
        } else if slot == 3 && self.rhand_pick >= 0 {
            Some(self.rhand_pick)
        } else {
            None
        }
    }
    fn wear(&self, plan: &mut TickPlan, frame: &Frame<'_>, tick: u16) {
        if !self.request.allow.equipment || !self.schedule.ready(OpKind::Wear, tick) {
            return;
        }
        for slot in 0..14u8 {
            if self.wear_skipped(slot) {
                continue;
            }
            if let Some(id) = self.desired(slot) {
                if !frame
                    .equipment
                    .iter()
                    .any(|row| row.slot == i32::from(slot) && row.def.id == id)
                    && arbiter::count(frame, id) > 0
                    && !self
                        .pending
                        .iter()
                        .any(|pending| pending.row.kind == RowKind::Wear && pending.row.aux == slot)
                {
                    self.push(plan, PlanRow::new(RowKind::Wear, id, slot));
                }
            }
        }
    }
    fn wear_skipped(&self, slot: u8) -> bool {
        let state = (self.prep_failures >> (u32::from(slot) * 3)) & 7;
        state & 4 != 0 && state & 3 >= 2
    }
    fn style(&mut self, plan: &mut TickPlan, frame: &Frame<'_>, tick: u16) {
        let Some(wanted) = self.request.melee_mode else {
            return;
        };
        if plan.contains(RowKind::Style)
            || self.pending_row(RowKind::Style)
            || !self.schedule.ready(OpKind::Style, tick)
        {
            return;
        }
        let worn = if let Some(row) = frame.equipment.iter().find(|row| row.slot == 3) {
            let Some(style) = self.tables.weapon_style(row.def.id) else {
                return;
            };
            let Some(tab) = style.tab else {
                return;
            };
            tab
        } else {
            CombatTab::Unarmed
        };
        let Some(root) = self.tables.combat_tab_root(worn) else {
            return;
        };
        if frame.combat_tab != Some(root) {
            return;
        }
        if plan.iter().any(|row| {
            row.kind == RowKind::Wear
                && row.aux == 3
                && self
                    .tables
                    .weapon_style(row.id)
                    .is_none_or(|style| style.tab != Some(worn))
        }) {
            return;
        }
        // A preceding wear may still be waiting for its new exact tab.
        if self
            .pending
            .iter()
            .any(|pending| pending.row.kind == RowKind::Wear && pending.row.aux == 3)
        {
            return;
        }
        let observed = self
            .tables
            .melee_mode_varp()
            .and_then(|varp| frame.varps.iter().find(|row| row.index == varp))
            .and_then(|row| u8::try_from(row.value).ok());
        let Some(choice) = self.tables.melee_mode(worn, wanted, observed) else {
            return;
        };
        if observed != Some(choice.slot) {
            self.push(
                plan,
                PlanRow::new(
                    RowKind::Style,
                    choice.button,
                    choice.slot | ((choice.actual as u8) << 2),
                ),
            );
        } else {
            self.mode_fallback = (choice.actual != wanted).then_some(choice.actual);
        }
    }
    fn retaliate(&self, plan: &mut TickPlan, frame: &Frame<'_>, tick: u16) {
        if !plan.contains(RowKind::Retaliate)
            && self.request.allow.retaliate_toggle
            && retaliate_on(frame) != self.request.retaliate
            && self.flags & SKIP_RETALIATE == 0
            && self.schedule.ready(OpKind::Retaliate, tick)
            && !self.pending_row(RowKind::Retaliate)
        {
            self.push(
                plan,
                PlanRow::new(RowKind::Retaliate, 0, u8::from(self.request.retaliate)),
            );
        }
    }
    fn target_above_quarter(&self, frame: &Frame<'_>) -> bool {
        self.engaged.is_none_or(|actor| match actor.kind {
            ActorKind::Npc => frame
                .npcs
                .iter()
                .find(|row| row.index == usize::from(actor.index))
                .is_some_and(|row| {
                    row.total_health <= 0 || row.health.saturating_mul(4) > row.total_health
                }),
            ActorKind::Player => frame
                .players
                .iter()
                .find(|row| row.index == usize::from(actor.index))
                .is_some_and(|row| {
                    row.actor.total_health <= 0
                        || row.actor.health.saturating_mul(4) > row.actor.total_health
                }),
        })
    }
    fn leash_cancel_tile(&self, frame: &Frame<'_>) -> Option<api::WorldTile> {
        let stand = self.request.stand?;
        if distance(frame.here, stand) > 1 {
            return Some(stand);
        }
        let actor = self.engaged?;
        let target = match actor.kind {
            ActorKind::Npc => frame
                .npcs
                .iter()
                .find(|row| {
                    row.index == usize::from(actor.index)
                        && row.r#type.map_or(-1, |id| id as i32) == self.engaged_type
                })
                .map(|row| row.tile),
            ActorKind::Player => frame
                .players
                .iter()
                .find(|row| {
                    row.index == usize::from(actor.index)
                        && self.player_ident.is_some()
                        && row.actor.name.as_deref().map(select::ident::fnv1a) == self.player_ident
                })
                .map(|row| row.actor.tile),
        }?;
        // A walk back to an already-reached stand produces no driver movement;
        // move just beyond its radius to cancel the live server chase instead.
        let away_x = (frame.here.x - target.x).signum();
        let away_z = (frame.here.z - target.z).signum();
        [
            (away_x * 2, away_z * 2),
            (away_x * 2, 0),
            (0, away_z * 2),
            (2, 0),
            (-2, 0),
            (0, 2),
            (0, -2),
        ]
        .into_iter()
        .find_map(|(dx, dz)| {
            if dx == 0 && dz == 0 {
                return None;
            }
            let x = frame.here.x + dx;
            let z = frame.here.z + dz;
            ((0..=16383).contains(&x) && (0..=16383).contains(&z)).then_some(api::WorldTile {
                x,
                z,
                level: frame.here.level,
            })
        })
        .or(Some(stand))
    }
    fn walk(
        &mut self,
        tile: api::WorldTile,
        tick: u16,
        cx: &mut ActionContext<'_>,
    ) -> Result<(), ActionError> {
        let id = cx.walk(WalkRequest {
            target: tile,
            loc_id: None,
            radius: 1,
            arrival: nav::arrival::ArrivalKind::Reach,
            options: Default::default(),
            required_after: cx.evidence(),
            evidence: None,
            cross: Vec::new().into_boxed_slice(),
            protect: false,
            allow: Default::default(),
        })?;
        self.pending_walk = NonZeroU64::new(id);
        self.schedule.clear(tick, 1, false);
        self.flags |= EMITTED;
        self.emitted_tick = tick;
        Ok(())
    }
    fn emit(
        &mut self,
        frame: &Frame<'_>,
        tick: u16,
        cx: &mut ActionContext<'_>,
    ) -> Result<(), ActionError> {
        let mut rows: [Option<InteractReq>; 5] = Default::default();
        let mut baselines = [0i16; 5];
        for (index, row) in self.plan.iter().enumerate() {
            rows[index] = Some(match row.kind {
                RowKind::Eat => {
                    baselines[index] =
                        arbiter::count(frame, row.id).min(i32::from(i16::MAX)) as i16;
                    held(frame.inventory, row.id, "Eat")?
                }
                RowKind::Drink => {
                    baselines[index] = arbiter::doses(frame, &self.tables, potion_kind(row.aux));
                    held(frame.inventory, row.id, "Drink")?
                }
                RowKind::Prayer | RowKind::Style => InteractReq::IfButton {
                    component_id: row.id,
                },
                RowKind::Wear => InteractReq::Wear {
                    name: item_name(frame.inventory, row.id)?.to_owned(),
                },
                RowKind::Retaliate => InteractReq::SetRetaliate { on: row.aux != 0 },
                RowKind::Attack => {
                    let actor = self
                        .engaged
                        .ok_or_else(|| unavailable("target disappeared"))?;
                    if actor_token(actor) != row.id {
                        return Err(unavailable("planned target changed"));
                    }
                    if !select::actor_attackable(frame, actor) {
                        return Err(unavailable("unattackable target"));
                    }
                    match actor.kind {
                        ActorKind::Npc => {
                            let npc = frame
                                .npcs
                                .iter()
                                .find(|row| {
                                    row.index == usize::from(actor.index)
                                        && row.r#type == usize::try_from(self.engaged_type).ok()
                                })
                                .ok_or_else(|| unavailable("npc identity changed"))?;
                            InteractReq::Npc {
                                name: npc
                                    .name
                                    .as_deref()
                                    .ok_or_else(|| unavailable("npc name"))?
                                    .to_owned(),
                                action: "Attack".into(),
                                index: Some(i32::from(actor.index)),
                            }
                        }
                        ActorKind::Player => {
                            let name = frame
                                .players
                                .iter()
                                .find(|row| {
                                    row.index == usize::from(actor.index)
                                        && self.player_ident.is_some()
                                        && row.actor.name.as_deref().map(select::ident::fnv1a)
                                            == self.player_ident
                                })
                                .and_then(|row| row.actor.name.as_deref())
                                .ok_or_else(|| unavailable("player identity"))?;
                            InteractReq::Player {
                                name: name.to_owned(),
                                action: "Attack".into(),
                            }
                        }
                    }
                }
                RowKind::Empty => unreachable!("compact combat plan"),
            });
        }
        let first = cx.emit_batch(rows)?;
        self.plan_first_id = first;
        self.emitted_tick = tick;
        self.flags |= EMITTED;
        if self.phase == Phase::Fight {
            self.flags |= PLAN_FIGHT;
        } else {
            self.flags &= !PLAN_FIGHT;
        }
        if self.phase == Phase::WindDown && self.plan.contains(RowKind::Eat) {
            self.flags |= TERMINAL_FOOD;
        }
        if self.plan.len >= 2 {
            self.counters.multi = self.counters.multi.saturating_add(1);
        }
        let mut index = 0;
        for (slot, pending) in self.pending.iter_mut().enumerate() {
            if index == usize::from(self.plan.len) {
                break;
            }
            if pending.row.kind == RowKind::Empty {
                *pending = PendingRow {
                    row: self.plan.rows[index],
                    baseline: baselines[index],
                    age: 0,
                };
                self.staged_pending |= 1 << slot;
                index += 1;
            }
        }
        debug_assert_eq!(index, usize::from(self.plan.len));
        Ok(())
    }
}
fn op_kind(kind: RowKind) -> OpKind {
    match kind {
        RowKind::Eat => OpKind::Eat,
        RowKind::Drink => OpKind::Drink,
        RowKind::Prayer => OpKind::Prayer,
        RowKind::Wear => OpKind::Wear,
        RowKind::Style => OpKind::Style,
        RowKind::Retaliate => OpKind::Retaliate,
        RowKind::Attack => OpKind::Attack,
        RowKind::Empty => unreachable!("empty pending operation"),
    }
}
fn potion_kind(code: u8) -> PotionKind {
    match code {
        0 => PotionKind::Prayer,
        1 => PotionKind::SuperAttack,
        2 => PotionKind::SuperStrength,
        3 => PotionKind::SuperDefence,
        4 => PotionKind::Ranging,
        5 => PotionKind::Magic,
        6 => PotionKind::Antifire,
        _ => unreachable!("planned potion kind"),
    }
}

fn unavailable(reason: &'static str) -> ActionError {
    ActionError::Unavailable(reason.into())
}
fn prayer_bit(varp: i32) -> u16 {
    1 << (varp - api::prayer::PRAYER_VARP0)
}
fn retaliate_on(frame: &Frame<'_>) -> bool {
    frame
        .varps
        .iter()
        .any(|row| row.index == super::OPTION_NODEF && row.value == 0)
}
fn prayer_observation(frame: &Frame<'_>) -> PrayerObservation {
    let mut obs = PrayerObservation::empty();
    (obs.points, obs.max) = arbiter::stat(frame, 5);
    for row in frame.varps {
        obs.set_varp(row.index, row.value);
    }
    obs
}
fn item_name(items: &[ItemView], id: i32) -> Result<&str, ActionError> {
    items
        .iter()
        .find(|row| row.def.id == id)
        .and_then(|row| row.def.name.as_deref())
        .ok_or_else(|| unavailable("item name"))
}
fn held(items: &[ItemView], id: i32, action: &str) -> Result<InteractReq, ActionError> {
    let row = items
        .iter()
        .find(|row| row.def.id == id && row.count > 0)
        .ok_or_else(|| unavailable("held item"))?;
    Ok(InteractReq::Held {
        name: item_name(items, id)?.to_owned(),
        action: action.to_owned(),
        slot: Some(row.slot),
        target_item_id: None,
    })
}
fn distance(a: api::WorldTile, b: api::WorldTile) -> i32 {
    if a.level != b.level {
        i32::MAX
    } else {
        (a.x - b.x).abs().max((a.z - b.z).abs())
    }
}

#[cfg(test)]
#[path = "machine_tests.rs"]
mod tests;

fn actor_token(actor: ActorRef) -> i32 {
    i32::from(actor.index)
        | if actor.kind == ActorKind::Player {
            1 << 16
        } else {
            0
        }
}
