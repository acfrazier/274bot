use super::arbiter::{self, Intent, TickPlan};
use super::frame::Frame;
use super::prayer::{PrayerSweep, SweepDecision};
use super::request::*;
use super::schedule::{elapsed, reached, Interaction, OpKind, Schedule};
use super::select;
use super::tables::{CombatTables, PotionKind, PrayerRole};
use super::threats::ThreatSet;
use crate::native::{ActionContext, ActionError, NativeMachine, WalkRequest};
use crate::shim::InteractReq;
use api::prayer::PrayerObservation;
use api::snapshot::ItemView;
use std::num::NonZeroU64;
use std::sync::Arc;
use std::task::Poll;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase { Prep, Engage, Fight, Escape, WindDown }
#[derive(Default)]
struct Counters {
    ticks: u16, swings: u16, damage: u16,
    food: u8, prayer: u8, boost: u8, antifire: u8,
    protected: u8, switches: u8, intruders: u8, restorations: u8, locked: u8, multi: u8,
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
    cleanup_since: u16,
    recovery_hp: i16,
    prayer_varp: i16,
    wear_id: i16,
    wear_slot: u8,
    drink_kind: PotionKind,
    phase: Phase,
    flags: u16,
}
const OBSERVED: u16 = 1;
const EMITTED: u16 = 2;
const LOST: u16 = 4;
const ANTIFIRE: u16 = 8;
const RECOVERY: u16 = 16;
const PRAYER_ON: u16 = 32;
const BOOST_WORTH: u16 = 64;
const SHIELD_OVERRIDE: u16 = 128;
const ENGAGEMENT_ATTACK: u16 = 256;
const _: () = assert!(std::mem::size_of::<Combat>() <= 512);

impl NativeMachine for Combat {
    type Args = (Arc<CombatRequest>, Arc<CombatTables>);
    type Output = CombatReport;
    fn begin((request, tables): Self::Args, cx: &mut ActionContext<'_>) -> Result<Self, ActionError> {
        if request.style != Style::Melee { return Err(unavailable("style slice")); }
        if request.prayer_mode != PrayerMode::Hold { return Err(unavailable("flick slice")); }
        if matches!(request.target, Target::Player { .. }) { return Err(unavailable("pvp slice")); }
        if request.tactic != Tactic::Open || matches!(request.fallback, Fallback::FaceTank { .. }) {
            return Err(unavailable("tactic slice"));
        }
        if tables.selected().selected_pin().map_err(|_| unavailable("combat pin"))?.as_ref() != cx.pin() {
            return Err(unavailable("combat pin"));
        }
        if let Target::Npc { types, .. } = &request.target {
            if types.is_empty() || types.iter().any(|id| tables.npc(*id).is_none()) {
                return Err(unavailable("combat npc facts"));
            }
            if !types.iter().any(|id| tables.npc_fact(*id).is_some_and(|row| row.attackable)) {
                return Err(unavailable("unattackable npc target"));
            }
        }
        let ticks = if request.budget_ticks == 0 { 1500 } else { request.budget_ticks };
        let mut machine = Self {
            request, tables, player_ident: None, deadline: cx.active_now().saturating_add(Duration::from_millis(u64::from(ticks) * 600)),
            pending_walk: None, threats: ThreatSet::default(), schedule: Schedule::default(), sweep: PrayerSweep::new(),
            counters: Counters::default(), sequence: 0, rhand_pick: -1, engaged_type: -1, suspended_type: -1,
            chat_since: -1, anim_id: -1, anim_frame: -1, engaged: None, suspended: None, end: None,
            antifire_sip: 0, lost_since: 0, escape_since: 0, last_tick: 0, emitted_tick: 0, cleanup_since: 0,
            recovery_hp: -1, prayer_varp: -1, wear_id: -1, wear_slot: 0,
            drink_kind: PotionKind::Prayer, phase: Phase::Prep, flags: 0,
        };
        if let Some(lines) = cx.snapshot().chat_lines(-1) {
            machine.chat_since = lines.value.iter().map(|row| row.sequence).max().unwrap_or(-1);
        }
        if let Some(frame) = Frame::borrow(cx.snapshot()) {
            let picked = select::pick_target(&machine.request, &frame, &machine.threats, &machine.tables, cx.run().slot ^ cx.run().run);
            if let Some(actor) = picked { machine.engage(actor, &frame); }
            machine.pick_weapon(&frame);
            machine.resolve_worth(&frame);
        }
        Ok(machine)
    }
    fn poll(&mut self, cx: &mut ActionContext<'_>) -> Poll<Result<CombatReport, ActionError>> {
        let snapshot = cx.snapshot();
        let Some(frame) = Frame::borrow(snapshot) else { return Poll::Pending; };
        let Some(chat) = snapshot.chat_lines(self.chat_since) else { return Poll::Pending; };
        let evidence = cx.evidence();
        if self.flags & OBSERVED != 0 && self.sequence == evidence.sequence { return Poll::Pending; }
        let tick = evidence.tick as u16;
        let new_tick = self.flags & OBSERVED == 0 || tick != self.last_tick;
        self.sequence = evidence.sequence;
        self.last_tick = tick;
        self.flags |= OBSERVED;
        if new_tick { self.counters.ticks = self.counters.ticks.saturating_add(1); }
        if self.phase != Phase::Escape && self.pending_walk.is_some_and(|id| cx.walk_receipt(id.get()).is_some()) {
            self.pending_walk = None;
        }
        let events = self.threats.observe(&frame, &self.tables, tick);
        self.counters.damage = self.counters.damage.saturating_add(events.damage);
        self.counters.protected = self.counters.protected.saturating_add(events.positive_protected);
        self.settle(&frame, tick);
        let died = chat.value.iter().any(|row| row.text.contains("Oh dear, you are dead"));
        if let Some(sequence) = chat.value.iter().map(|row| row.sequence).max() { self.chat_since = sequence; }
        if self.phase != Phase::WindDown {
            if died || arbiter::stat(&frame, 3).0 <= 0 { self.finish(CombatEnd::Died, tick); }
            else { self.observe_target(&frame, tick); }
            if self.phase != Phase::WindDown && cx.active_now() >= self.deadline { self.finish(CombatEnd::Budget, tick); }
            if self.phase != Phase::WindDown && self.request.until_ticks != 0 && self.counters.ticks >= self.request.until_ticks {
                self.finish(CombatEnd::Budget, tick);
            }
            if self.phase != Phase::WindDown && matches!(self.request.target, Target::Attacker { .. })
                && select::pick_target(&self.request, &frame, &self.threats, &self.tables, evidence.tick).is_none()
                && select::has_unattackable_allowed(&self.request, &frame, &self.threats) {
                self.finish(CombatEnd::Aborted(AbortReason::Unattackable), tick);
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
            let danger = self.threats.danger(&frame, &self.tables, tick, self.shield(&frame), self.antifire(tick));
            let lines = select::lines(danger, arbiter::stat(&frame, 3).1);
            let food = self.request.allow.food.then(|| arbiter::food(&frame, &self.tables, None, None)).flatten();
            self.safety_end(&frame, tick, danger, lines.emergency, food);
            if new_tick { self.counters.locked = self.counters.locked.saturating_add(1); }
            return Poll::Pending;
        }
        self.schedule.unlock_observed(tick);
        if self.flags & EMITTED != 0 && self.emitted_tick == tick { return Poll::Pending; }
        let decision = self.plan(&frame, tick, cx);
        match decision {
            Ok(mut plan) => match plan.take_first() {
                Some(intent) => match self.emit(intent, &frame, tick, cx) {
                    Ok(()) | Err(ActionError::BudgetExhausted) | Err(ActionError::Unavailable(_)) => Poll::Pending,
                    Err(error) => Poll::Ready(Err(error)),
                },
                None if self.phase == Phase::WindDown => {
                    match self.sweep.next(Some(self.tables.selected()), &prayer_observation(&frame)) {
                        SweepDecision::Done(report) if report.timed_out == 0 => Poll::Ready(Ok(self.report(evidence))),
                        SweepDecision::Done(_) => Poll::Ready(Err(ActionError::Failed("prayer cleanup did not settle".into()))),
                        _ => Poll::Pending,
                    }
                }
                None => Poll::Pending,
            },
            Err(error) => Poll::Ready(Err(error)),
        }
    }
    fn cancel(&mut self) {
        self.schedule.terminal();
        self.pending_walk = None;
    }
}

impl Combat {
    pub fn input_lock(&self) -> Option<u16> { self.schedule.locked(self.last_tick).then_some(self.schedule.input_lock) }
    pub fn interaction(&self) -> Interaction { self.schedule.interaction }
    pub fn cycle(&self) -> super::schedule::Cycle { self.schedule.cycle }
    pub fn engaged(&self) -> Option<ActorRef> { self.engaged }
    fn finish(&mut self, end: CombatEnd, tick: u16) {
        if self.end.is_some() { return; }
        self.end = Some(end);
        self.phase = Phase::WindDown;
        self.schedule.terminal();
        self.cleanup_since = tick;
        self.sweep = PrayerSweep::new();
    }
    fn report(&self, evidence: api::quest_progress::EvidenceStamp) -> CombatReport {
        CombatReport {
            end: self.end.expect("terminal phase has a latched end"), evidence, engaged: self.engaged, engaged_npc_type: self.engaged_type,
            ticks: self.counters.ticks, swings: self.counters.swings, casts: 0, damage_taken: self.counters.damage,
            food: self.counters.food, prayer_doses: self.counters.prayer, boost_doses: self.counters.boost,
            antifire_doses: self.counters.antifire, hits_while_protected: self.counters.protected,
            protect_switches: self.counters.switches, intruders: self.counters.intruders, ammo_pickups: 0,
            restorations: self.counters.restorations, locked_ticks: self.counters.locked,
            multi_op_plans: self.counters.multi,
            melee_mode_fallback: None,
            flick_resets: 0, flick_misses: 0, flick_fallback: false,
        }
    }
    fn rate(&self, frame: &Frame<'_>) -> u8 {
        frame.equipment.iter().find(|row| row.slot == 3).and_then(|row| self.tables.weapon_style(row.def.id)).map_or(4, |row| row.attackrate.max(1))
    }
    fn engage(&mut self, actor: ActorRef, frame: &Frame<'_>) {
        self.engaged = Some(actor);
        self.flags &= !LOST;
        match actor.kind {
            ActorKind::Npc => {
                self.engaged_type = frame.npcs.iter().find(|row| row.index == usize::from(actor.index)).and_then(|row| row.r#type).map_or(-1, |id| id as i32);
                self.player_ident = None;
            }
            ActorKind::Player => {
                self.engaged_type = -1;
                self.player_ident = frame.players.iter().find(|row| row.index == usize::from(actor.index)).and_then(|row| row.actor.name.as_deref()).map(select::ident::fnv1a);
            }
        }
        self.schedule.interaction = Interaction::Unknown;
        self.resolve_worth(frame);
    }
    fn resolve_worth(&mut self, frame: &Frame<'_>) {
        let carries_boost = self.request.kit.as_ref().is_some_and(|kit| kit.carry.iter().any(|(id, _)| {
            [PotionKind::SuperAttack, PotionKind::SuperStrength, PotionKind::SuperDefence].iter().any(|kind| {
                self.tables.potion(*kind).is_some_and(|family| family.doses.iter().flatten().any(|dose| dose.id == *id))
            })
        }));
        let worth = select::offensives_worth(self.engaged, self.engaged_type, &self.tables, carries_boost);
        if worth { self.flags |= BOOST_WORTH; } else { self.flags &= !BOOST_WORTH; }
        if self.tables.npc(self.engaged_type).is_some_and(|row| row.dragonfire.is_some()) { self.flags |= SHIELD_OVERRIDE; }
        else { self.flags &= !SHIELD_OVERRIDE; }
        if self.rhand_pick < 0 { self.pick_weapon(frame); }
    }
    fn pick_weapon(&mut self, frame: &Frame<'_>) {
        if self.request.kit.is_some() { return; }
        let Some(names) = self.tables.selected().equipment_names() else { return; };
        let Some(family) = names.family("melee_weapons") else { return; };
        let name = crate::melee_weapons::best_melee_weapon_by(family, arbiter::stat(frame, 0).1, false, |name| {
            frame.inventory.iter().chain(frame.equipment).any(|row| row.def.name.as_deref().is_some_and(|have| have.eq_ignore_ascii_case(name)))
        });
        if let Some(name) = name {
            self.rhand_pick = frame.inventory.iter().chain(frame.equipment).find(|row| row.def.name.as_deref().is_some_and(|have| have.eq_ignore_ascii_case(name))).map_or(-1, |row| row.def.id);
        }
    }
    fn observe_target(&mut self, frame: &Frame<'_>, tick: u16) {
        let seed = u64::from(tick) ^ self.sequence;
        if self.engaged.is_none() {
            if let Some(actor) = select::pick_target(&self.request, frame, &self.threats, &self.tables, seed) { self.engage(actor, frame); }
        }
        let mut present = false;
        let mut killed = false;
        if let Some(actor) = self.engaged {
            match actor.kind {
                ActorKind::Npc => if let Some(row) = frame.npcs.iter().find(|row| row.index == usize::from(actor.index)) {
                    let ty = row.r#type.map_or(-1, |id| id as i32);
                    let same = ty == self.engaged_type;
                    let listed = match &self.request.target { Target::Npc { types, .. } => types.contains(&ty), _ => same };
                    present = listed && row.actions.iter().flatten().any(|action| action == "Attack")
                        && distance(row.tile, self.request.stand.unwrap_or(frame.here)) <= i32::from(self.request.lost_radius);
                    killed = same && row.total_health > 0 && row.health == 0;
                    if present && !same {
                        self.engaged_type = ty;
                        self.schedule.interaction = Interaction::Unknown;
                        self.resolve_worth(frame);
                    }
                },
                ActorKind::Player => if let Some(row) = frame.players.iter().find(|row| row.index == usize::from(actor.index)
                    && self.player_ident.is_some() && row.actor.name.as_deref().map(select::ident::fnv1a) == self.player_ident) {
                    present = distance(row.actor.tile, self.request.stand.unwrap_or(frame.here)) <= i32::from(self.request.lost_radius);
                    killed = row.actor.total_health > 0 && row.actor.health == 0;
                },
            }
        }
        if killed {
            if self.resume_suspended(frame) { return; }
            self.finish(CombatEnd::Killed, tick);
            return;
        }
        if self.engaged.is_some() && !present {
            if self.flags & LOST == 0 { self.lost_since = tick; self.flags |= LOST; }
            else if elapsed(tick, self.lost_since) >= 3 {
                if !self.resume_suspended(frame) { self.finish(CombatEnd::TargetGone, tick); }
            }
            return;
        }
        if present { self.flags &= !LOST; }
        if self.engaged.is_none() {
            if self.flags & LOST == 0 { self.lost_since = tick; self.flags |= LOST; }
            else if elapsed(tick, self.lost_since) >= 15 { self.finish(CombatEnd::NoTarget, tick); }
        }
        if self.phase == Phase::Engage && self.flags & ENGAGEMENT_ATTACK != 0 && self.engaged.is_some_and(|actor| frame.in_combat.target.is_some_and(|target| actor.matches(target))
            || self.threats.iter(tick).any(|threat| threat.actor == actor)) {
            self.phase = Phase::Fight;
        }
        if matches!(self.request.target, Target::Npc { .. }) && self.request.intruder.player == IntruderRule::FightBack
            && self.engaged.is_some_and(|actor| actor.kind == ActorKind::Npc) {
            let player = self.threats.iter(tick).find(|threat| threat.actor.kind == ActorKind::Player).map(|threat| threat.actor);
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
        let Some(actor) = self.suspended.take() else { return false; };
        if frame.npcs.iter().any(|row| row.index == usize::from(actor.index) && row.r#type == usize::try_from(self.suspended_type).ok()
            && !(row.total_health > 0 && row.health == 0) && row.actions.iter().flatten().any(|action| action == "Attack")) {
            self.engage(actor, frame);
            self.phase = Phase::Fight;
            true
        } else { false }
    }
    fn settle(&mut self, frame: &Frame<'_>, tick: u16) {
        let rate = self.rate(frame);
        if super::style::melee::onset(&frame.local.player.actor, self.engaged, &self.tables, &mut self.anim_id, &mut self.anim_frame) {
            self.counters.swings = self.counters.swings.saturating_add(1);
            self.schedule.observe_swing(tick, rate);
        }
        for kind in [OpKind::Eat, OpKind::Drink, OpKind::Prayer, OpKind::Wear, OpKind::Retaliate, OpKind::Attack] {
            if !self.schedule.pending(kind) { continue; }
            let baseline = self.schedule.pending_base[kind.index()];
            let settled = match kind {
                OpKind::Eat => arbiter::stat(frame, 3).0 > i32::from(baseline),
                OpKind::Drink => arbiter::doses(frame, &self.tables, self.drink_kind) < baseline,
                OpKind::Prayer => super::prayer::PrayerToggle::new(
                    i32::from(self.prayer_varp), api::prayer::OnArg::Bool(self.flags & PRAYER_ON != 0),
                ).progress(&prayer_observation(frame), false) == super::prayer::ToggleProgress::Matched,
                OpKind::Wear => frame.equipment.iter().any(|row| row.slot == i32::from(self.wear_slot) && row.def.id == i32::from(self.wear_id)),
                OpKind::Retaliate => frame.varps.iter().any(|row| row.index == super::OPTION_NODEF && row.value == i32::from(baseline)),
                OpKind::Attack => self.engaged.is_some_and(|actor| frame.in_combat.target.is_some_and(|target| actor.matches(target)))
                    && (!self.schedule.clear_valid || (reached(tick, self.schedule.last_clear) && tick != self.schedule.last_clear)),
            };
            if settled {
                self.schedule.settle(kind);
                match kind {
                    OpKind::Eat => self.counters.food = self.counters.food.saturating_add(1),
                    OpKind::Drink => match self.drink_kind {
                        PotionKind::Prayer => self.counters.prayer = self.counters.prayer.saturating_add(1),
                        PotionKind::Antifire => self.counters.antifire = self.counters.antifire.saturating_add(1),
                        _ => self.counters.boost = self.counters.boost.saturating_add(1),
                    },
                    OpKind::Attack => self.schedule.interaction = Interaction::Installed,
                    _ => {},
                }
            } else if self.schedule.pending_age(kind, tick) >= match kind { OpKind::Eat => 3, OpKind::Drink => 2, _ => 4 } {
                self.schedule.timeout(kind);
                if self.schedule.unsettled[kind.index()] >= 2 {
                    match kind {
                        OpKind::Wear if self.wear_slot == 3 => self.finish(CombatEnd::Aborted(AbortReason::PrepFailed(PrepItem::Weapon)), tick),
                        OpKind::Wear if self.wear_slot == 5 && self.flags & SHIELD_OVERRIDE != 0 => self.finish(CombatEnd::Aborted(AbortReason::PrepFailed(PrepItem::Shield)), tick),
                        OpKind::Attack => self.finish(CombatEnd::Aborted(AbortReason::Unresponsive), tick),
                        _ => {},
                    }
                }
            }
        }
    }
    fn shield(&self, frame: &Frame<'_>) -> bool {
        self.tables.selected().item_by_alias("antidragonbreathshield").is_some_and(|item| frame.equipment.iter().any(|row| row.slot == 5 && row.def.id == item.id))
    }
    fn antifire(&self, tick: u16) -> bool { self.flags & ANTIFIRE != 0 && elapsed(tick, self.antifire_sip) < 600 }
    fn safety_end(&mut self, frame: &Frame<'_>, tick: u16, danger: Option<i32>, emergency: i32, food: Option<i32>) {
        if self.phase == Phase::Escape || self.phase == Phase::WindDown { return; }
        let no_food = danger != Some(0) && arbiter::stat(frame, 3).0 <= emergency && food.is_none();
        let no_fire = self.flags & SHIELD_OVERRIDE != 0 && !self.shield(frame) && !self.antifire(tick)
            && (!self.request.allow.equipment || self.tables.selected().item_by_alias("antidragonbreathshield").is_none_or(|item| arbiter::count(frame, item.id) == 0))
            && (!self.request.allow.potions || arbiter::potion_id(frame, &self.tables, PotionKind::Antifire).is_none());
        if no_food || no_fire {
            let reason = AbortReason::Unprotected(if no_fire { Unprotected::Dragonfire } else { Unprotected::NoFood });
            match self.request.fallback {
                Fallback::Retreat { .. } => { self.phase = Phase::Escape; self.escape_since = tick; self.schedule.terminal(); }
                Fallback::Fight if !no_fire => {},
                _ => self.finish(CombatEnd::Aborted(reason), tick),
            }
        }
    }
    fn plan(&mut self, frame: &Frame<'_>, tick: u16, cx: &ActionContext<'_>) -> Result<TickPlan, ActionError> {
        let (hp, hp_max) = arbiter::stat(frame, 3);
        let danger = self.threats.danger(frame, &self.tables, tick, self.shield(frame), self.antifire(tick));
        let lines = select::lines(danger, hp_max);
        let can_eat = self.request.allow.food && !self.schedule.pending(OpKind::Eat);
        let available_food = self.request.allow.food.then(|| arbiter::food(frame, &self.tables, None, None)).flatten();
        let food = self.request.allow.food.then(|| arbiter::food(frame, &self.tables, None, Some((&self.schedule, tick)))).flatten();
        if self.phase == Phase::WindDown {
            if hp > 0 && danger != Some(0) && hp <= lines.eat && can_eat {
                if let Some(id) = food { return Ok(TickPlan::single(Intent::Eat { id, recovery: false })); }
            }
            let obs = prayer_observation(frame);
            self.sweep.observe(&obs, elapsed(tick, self.cleanup_since) >= 4);
            return Ok(TickPlan::from_option(match self.sweep.next(Some(self.tables.selected()), &obs) {
                SweepDecision::Click(click) => Some(Intent::Prayer { varp: click.varp, button: click.button_com, on: false }),
                SweepDecision::Wait | SweepDecision::Done(_) => None,
            }));
        }
        self.safety_end(frame, tick, danger, lines.emergency, available_food);
        if self.phase == Phase::WindDown { return self.plan(frame, tick, cx); }
        if danger != Some(0) && hp <= lines.emergency && can_eat {
            if let Some(id) = food { return Ok(TickPlan::single(Intent::Eat { id, recovery: false })); }
        }
        let wanted = if matches!(self.phase, Phase::Escape | Phase::WindDown) || !self.request.allow.prayer { None }
            else { select::wanted_protect(&self.threats, frame, &self.tables, tick, self.shield(frame), self.antifire(tick)) };
        let (points, base) = arbiter::stat(frame, 5);
        if let Some(protect) = wanted.filter(|row| base >= row.level && !prayer_on(frame, row.varp)) {
            if points == 0 && self.request.allow.potions {
                if let Some(intent) = self.drink_or_recover(PotionKind::Prayer, true, frame, tick, lines.drink_gate, can_eat) { return Ok(TickPlan::single(intent)); }
            }
            if points > 0 && hp > lines.emergency && self.schedule.ready(OpKind::Prayer, tick) && !self.schedule.pending(OpKind::Prayer) {
                return Ok(TickPlan::single(Intent::Prayer { varp: protect.varp, button: protect.button_com, on: true }));
            }
        }
        if danger != Some(0) && hp <= lines.eat && can_eat {
            if let Some(id) = food { return Ok(TickPlan::single(Intent::Eat { id, recovery: false })); }
        }
        match self.phase {
            Phase::WindDown => return self.plan(frame, tick, cx),
            Phase::Escape => {
                let Fallback::Retreat { tile } = self.request.fallback else { unreachable!("escape requires retreat"); };
                if distance(frame.here, tile) <= 1 {
                    self.finish(CombatEnd::Aborted(AbortReason::Retreated), tick);
                    return self.plan(frame, tick, cx);
                }
                let terminal = self.pending_walk.and_then(|id| cx.walk_receipt(id.get())).is_some();
                if terminal || elapsed(tick, self.escape_since) >= 60 {
                    self.finish(CombatEnd::Aborted(AbortReason::RetreatFailed), tick);
                    return self.plan(frame, tick, cx);
                }
                if self.request.allow.retaliate_toggle && retaliate_on(frame) && self.schedule.ready(OpKind::Retaliate, tick) && !self.schedule.pending(OpKind::Retaliate) {
                    return Ok(TickPlan::single(Intent::Retaliate(false)));
                }
                if self.pending_walk.is_none() { return Ok(TickPlan::single(Intent::Walk { tile, escape: true })); }
                return Ok(TickPlan::default());
            }
            Phase::Prep => {
                let weapon = self.request.kit.as_ref().and_then(|kit| kit.worn.iter().find(|(slot, _)| *slot == 3).map(|(_, id)| *id))
                    .or((self.rhand_pick >= 0).then_some(self.rhand_pick));
                let conflict = self.flags & SHIELD_OVERRIDE != 0 && weapon.is_some_and(|id|
                    self.tables.selected().item_by_id(id).is_some_and(|item| item.is_two_handed()));
                let missing = self.request.allow.equipment && weapon.is_some_and(|id|
                    arbiter::count(frame, id) == 0 && !frame.equipment.iter().any(|row| row.slot == 3 && row.def.id == id));
                if conflict || missing {
                    self.finish(CombatEnd::Aborted(AbortReason::PrepFailed(if conflict { PrepItem::Shield } else { PrepItem::Weapon })), tick);
                    return self.plan(frame, tick, cx);
                }
                if let Some(intent) = self.wear(frame, tick) { return Ok(TickPlan::single(intent)); }
                if self.schedule.pending(OpKind::Wear) { return Ok(TickPlan::default()); }
                if self.request.allow.retaliate_toggle && retaliate_on(frame) != self.request.retaliate
                    && self.schedule.ready(OpKind::Retaliate, tick) && !self.schedule.pending(OpKind::Retaliate)
                    && self.schedule.unsettled[OpKind::Retaliate.index()] < 2 {
                    return Ok(TickPlan::single(Intent::Retaliate(self.request.retaliate)));
                }
                if self.schedule.pending(OpKind::Retaliate) { return Ok(TickPlan::default()); }
                if self.flags & SHIELD_OVERRIDE != 0 && (!self.antifire(tick) || elapsed(tick, self.antifire_sip) >= 590) {
                    if let Some(intent) = self.drink_or_recover(PotionKind::Antifire, true, frame, tick, lines.drink_gate, can_eat) { return Ok(TickPlan::single(intent)); }
                }
                if self.flags & BOOST_WORTH != 0 {
                    if let Some(kind) = self.boost(frame) {
                        if let Some(intent) = self.drink_or_recover(kind, false, frame, tick, lines.drink_gate, can_eat) { return Ok(TickPlan::single(intent)); }
                    }
                }
                if self.schedule.pending(OpKind::Drink) { return Ok(TickPlan::default()); }
                self.phase = Phase::Engage;
            }
            Phase::Engage | Phase::Fight => {},
        }
        if self.phase == Phase::Engage {
            if self.engaged.is_some() && self.schedule.ready(OpKind::Attack, tick) && !self.schedule.pending(OpKind::Attack) {
                return Ok(TickPlan::single(Intent::Attack { restoration: false }));
            }
            if self.engaged.is_none() && self.pending_walk.is_none() {
                if let Some(tile) = self.request.stand.filter(|tile| distance(frame.here, *tile) > 1) {
                    return Ok(TickPlan::single(Intent::Walk { tile, escape: false }));
                }
            }
            return Ok(TickPlan::default());
        }
        if self.phase != Phase::Fight { return Ok(TickPlan::default()); }
        if self.schedule.restore_ready(tick) && self.engaged.is_some() { return Ok(TickPlan::single(Intent::Attack { restoration: true })); }
        let floor = (base - (7 + base / 4)).max(3);
        let active_or_wanted = wanted.is_some() || frame.prayers.iter().any(|on| *on);
        if self.request.allow.prayer && self.request.allow.potions && active_or_wanted && points <= floor {
            if let Some(intent) = self.drink_or_recover(PotionKind::Prayer, true, frame, tick, lines.drink_gate, can_eat) { return Ok(TickPlan::single(intent)); }
        }
        let cycle_room = !self.schedule.cycle.known || !reached(tick.wrapping_add(2), self.schedule.cycle.deadline);
        if cycle_room && self.flags & BOOST_WORTH != 0 && self.target_above_quarter(frame) {
            if let Some(kind) = self.boost(frame) {
                if let Some(intent) = self.drink_or_recover(kind, false, frame, tick, lines.drink_gate, can_eat) { return Ok(TickPlan::single(intent)); }
            }
        }
        if cycle_room && self.flags & SHIELD_OVERRIDE != 0 && (!self.antifire(tick) || elapsed(tick, self.antifire_sip) >= 590) {
            if let Some(intent) = self.drink_or_recover(PotionKind::Antifire, true, frame, tick, lines.drink_gate, can_eat) { return Ok(TickPlan::single(intent)); }
        }
        let offense = self.flags & BOOST_WORTH != 0 && self.request.allow.prayer && points > 0
            && (points > floor || arbiter::potion_id(frame, &self.tables, PotionKind::Prayer).is_some());
        if offense && !self.schedule.pending(OpKind::Prayer) && self.schedule.ready(OpKind::Prayer, tick) {
            for role in [PrayerRole::Strength, PrayerRole::Attack] {
                if let Some(row) = (0..3).rev().filter_map(|tier| self.tables.prayer(role, tier)).find(|row| base >= row.level) {
                    if !prayer_on(frame, row.varp) { return Ok(TickPlan::single(Intent::Prayer { varp: row.varp, button: row.button_com, on: true })); }
                }
            }
        }
        if !self.schedule.pending(OpKind::Prayer) {
            for row in self.tables.selected().prayers() {
                if !prayer_on(frame, row.varp) { continue; }
                let keep = self.request.allow.prayer && (wanted.is_some_and(|wanted| wanted.varp == row.varp)
                    || (offense && [PrayerRole::Strength, PrayerRole::Attack].iter().any(|role|
                        (0..3).rev().filter_map(|tier| self.tables.prayer(*role, tier)).find(|row| base >= row.level).is_some_and(|best| best.varp == row.varp))));
                if !keep { return Ok(TickPlan::single(Intent::Prayer { varp: row.varp, button: row.button_com, on: false })); }
            }
        }
        if let Some(intent) = self.wear(frame, tick) { return Ok(TickPlan::single(intent)); }
        let mismatch = self.engaged.is_some_and(|actor| frame.in_combat.target.is_some_and(|target| !actor.matches(target)));
        if self.engaged.is_some() && (mismatch || self.schedule.stale_ready(tick, self.rate(frame))) {
            return Ok(TickPlan::single(Intent::Attack { restoration: false }));
        }
        Ok(TickPlan::default())
    }
    fn drink_or_recover(&self, kind: PotionKind, lowers_danger: bool, frame: &Frame<'_>, tick: u16, gate: i32, can_eat: bool) -> Option<Intent> {
        if !self.request.allow.potions || (kind == PotionKind::Prayer && !self.request.allow.prayer)
            || !self.schedule.ready(OpKind::Drink, tick) || self.schedule.pending(OpKind::Drink)
            || self.schedule.unsettled[OpKind::Drink.index()] >= 2 || arbiter::potion_id(frame, &self.tables, kind).is_none() { return None; }
        let hp = arbiter::stat(frame, 3).0;
        if hp > gate { return Some(Intent::Drink(kind)); }
        if lowers_danger && can_eat && (self.flags & RECOVERY == 0 || i32::from(self.recovery_hp) != hp) {
            return arbiter::food(frame, &self.tables, Some(gate), Some((&self.schedule, tick))).map(|id| Intent::Eat { id, recovery: true });
        }
        None
    }
    fn boost(&self, frame: &Frame<'_>) -> Option<PotionKind> {
        if !self.request.allow.potions { return None; }
        [PotionKind::SuperAttack, PotionKind::SuperStrength, PotionKind::SuperDefence].into_iter().find(|kind| {
            let Some(family) = self.tables.potion(*kind) else { return false; };
            let Some(stat) = family.stat else { return false; };
            let (effective, base) = arbiter::stat(frame, stat.index() as i32);
            let ceiling = base + family.constant + base * family.percent / 100;
            crate::boost_potions::boost_faded(f64::from(base), f64::from(effective), crate::boost_potions::BOOST_FLOOR)
                && effective < ceiling && arbiter::potion_id(frame, &self.tables, *kind).is_some()
        })
    }
    fn wear(&self, frame: &Frame<'_>, tick: u16) -> Option<Intent> {
        if !self.request.allow.equipment || !self.schedule.ready(OpKind::Wear, tick) || self.schedule.pending(OpKind::Wear)
            || self.schedule.unsettled[OpKind::Wear.index()] >= 2 { return None; }
        let shield = self.tables.selected().item_by_alias("antidragonbreathshield").map(|row| row.id);
        for slot in 0..14u8 {
            let desired = if slot == 5 && self.flags & SHIELD_OVERRIDE != 0 { shield }
                else if let Some(kit) = &self.request.kit { kit.worn.iter().find(|(wanted, _)| *wanted == slot).map(|(_, id)| *id) }
                else if slot == 3 && self.rhand_pick >= 0 { Some(self.rhand_pick) } else { None };
            if let Some(id) = desired {
                if !frame.equipment.iter().any(|row| row.slot == i32::from(slot) && row.def.id == id) && arbiter::count(frame, id) > 0 {
                    return Some(Intent::Wear { id, slot });
                }
            }
        }
        None
    }
    fn target_above_quarter(&self, frame: &Frame<'_>) -> bool {
        self.engaged.is_none_or(|actor| match actor.kind {
            ActorKind::Npc => frame.npcs.iter().find(|row| row.index == usize::from(actor.index)).is_some_and(|row| row.total_health <= 0 || row.health.saturating_mul(4) > row.total_health),
            ActorKind::Player => frame.players.iter().find(|row| row.index == usize::from(actor.index)).is_some_and(|row| row.actor.total_health <= 0 || row.actor.health.saturating_mul(4) > row.actor.total_health),
        })
    }
    fn emit(&mut self, intent: Intent, frame: &Frame<'_>, tick: u16, cx: &mut ActionContext<'_>) -> Result<(), ActionError> {
        let rate = self.rate(frame);
        let (request, kind, baseline) = match intent {
            Intent::Eat { id, .. } => (held(frame.inventory, id, "Eat")?, Some(OpKind::Eat), arbiter::stat(frame, 3).0 as i16),
            Intent::Drink(kind) => {
                let id = arbiter::potion_id(frame, &self.tables, kind).ok_or_else(|| unavailable("potion disappeared"))?;
                (held(frame.inventory, id, "Drink")?, Some(OpKind::Drink), arbiter::doses(frame, &self.tables, kind))
            }
            Intent::Prayer { button, .. } => (InteractReq::IfButton { component_id: button }, Some(OpKind::Prayer), 0),
            Intent::Wear { id, .. } => (InteractReq::Wear { name: item_name(frame.inventory, id)?.to_owned() }, Some(OpKind::Wear), 0),
            Intent::Retaliate(on) => (InteractReq::SetRetaliate { on }, Some(OpKind::Retaliate), i16::from(!on)),
            Intent::Attack { .. } => {
                let actor = self.engaged.ok_or_else(|| unavailable("target disappeared"))?;
                if !select::actor_attackable(frame, actor) { return Err(unavailable("unattackable target")); }
                let request = match actor.kind {
                    ActorKind::Npc => {
                        let row = frame.npcs.iter().find(|row| row.index == usize::from(actor.index) && row.r#type == usize::try_from(self.engaged_type).ok()).ok_or_else(|| unavailable("npc identity changed"))?;
                        InteractReq::Npc { name: row.name.as_deref().ok_or_else(|| unavailable("npc name"))?.to_owned(), action: "Attack".into(), index: Some(i32::from(actor.index)) }
                    }
                    ActorKind::Player => {
                        let name = frame.players.iter().find(|row| row.index == usize::from(actor.index)
                            && self.player_ident.is_some() && row.actor.name.as_deref().map(select::ident::fnv1a) == self.player_ident)
                            .and_then(|row| row.actor.name.as_deref()).ok_or_else(|| unavailable("player identity"))?;
                        InteractReq::Player { name: name.to_owned(), action: "Attack".into() }
                    }
                };
                (request, Some(OpKind::Attack), 0)
            }
            Intent::Walk { tile, .. } => {
                let id = cx.walk(WalkRequest { target: tile, radius: 1, options: Default::default(), required_after: cx.evidence(), evidence: None })?;
                self.pending_walk = NonZeroU64::new(id);
                self.schedule.clear(tick, 1, false);
                self.flags |= EMITTED;
                self.emitted_tick = tick;
                return Ok(());
            }
        };
        cx.emit(request)?;
        self.flags |= EMITTED;
        self.emitted_tick = tick;
        if let Some(kind) = kind {
            let effect = match intent {
                Intent::Eat { id, .. } => super::schedule::InputEffect::Food(self.tables.food(id).expect("planned food has pinned timing metadata")),
                Intent::Prayer { on: true, .. } => super::schedule::InputEffect::PrayerOn,
                Intent::Wear { id, .. } if self.tables.selected().item_by_alias("elemental_shield").is_some_and(|row| row.id == id) =>
                    super::schedule::InputEffect::ElementalShield,
                _ => super::schedule::InputEffect::Standard,
            };
            self.schedule.admitted(kind, tick, baseline, rate, self.phase == Phase::Fight, effect);
        }
        match intent {
            Intent::Eat { recovery: true, .. } => { self.recovery_hp = arbiter::stat(frame, 3).0 as i16; self.flags |= RECOVERY; }
            Intent::Drink(kind) => {
                self.drink_kind = kind;
                if kind == PotionKind::Antifire { self.antifire_sip = tick; self.flags |= ANTIFIRE; }
            }
            Intent::Prayer { varp, on, .. } => {
                self.prayer_varp = varp as i16;
                if on { self.flags |= PRAYER_ON; } else { self.flags &= !PRAYER_ON; }
                if self.phase == Phase::WindDown {
                    if let SweepDecision::Click(click) = self.sweep.next(Some(self.tables.selected()), &prayer_observation(frame)) { self.sweep.emitted(click); }
                    self.cleanup_since = tick;
                } else if on && self.tables.selected().prayers().iter().any(|row| row.varp == varp && row.name.starts_with("Protect")) {
                    self.counters.switches = self.counters.switches.saturating_add(1);
                }
            }
            Intent::Wear { id, slot } => { self.wear_id = id as i16; self.wear_slot = slot; }
            Intent::Attack { restoration } => {
                self.flags |= ENGAGEMENT_ATTACK;
                if restoration { self.counters.restorations = self.counters.restorations.saturating_add(1); }
            }
            _ => {},
        }
        Ok(())
    }
}
fn unavailable(reason: &'static str) -> ActionError { ActionError::Unavailable(reason.into()) }
fn prayer_on(frame: &Frame<'_>, varp: i32) -> bool { frame.varps.iter().any(|row| row.index == varp && row.value == 1) }
fn retaliate_on(frame: &Frame<'_>) -> bool { frame.varps.iter().any(|row| row.index == super::OPTION_NODEF && row.value == 0) }
fn prayer_observation(frame: &Frame<'_>) -> PrayerObservation {
    let mut obs = PrayerObservation::empty();
    (obs.points, obs.max) = arbiter::stat(frame, 5);
    for row in frame.varps { obs.set_varp(row.index, row.value); }
    obs
}
fn item_name(items: &[ItemView], id: i32) -> Result<&str, ActionError> {
    items.iter().find(|row| row.def.id == id).and_then(|row| row.def.name.as_deref()).ok_or_else(|| unavailable("item name"))
}
fn held(items: &[ItemView], id: i32, action: &str) -> Result<InteractReq, ActionError> {
    let row = items.iter().find(|row| row.def.id == id && row.count > 0).ok_or_else(|| unavailable("held item"))?;
    Ok(InteractReq::Held { name: item_name(items, id)?.to_owned(), action: action.to_owned(), slot: Some(row.slot) })
}
fn distance(a: api::WorldTile, b: api::WorldTile) -> i32 {
    if a.level != b.level { i32::MAX } else { (a.x - b.x).abs().max((a.z - b.z).abs()) }
}

#[cfg(test)]
#[path = "machine_tests.rs"]
mod tests;
