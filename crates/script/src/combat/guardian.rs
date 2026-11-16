//! The native Fight child: one Combat engagement against one Guardian event
//! actor (design V11D 4.1, 5 and 6). It adds no combat logic. It builds the
//! request, prices the escape, steps the shared Combat core on fresh
//! observations, and maps the core's end to a typed [`FightOutcome`]. The
//! slot holds at most one child, in `SlotScript`, so a slot that never
//! loaded a card has a home for it.
//!
//! The Guardian keeps Host ownership throughout. The child never claims an
//! event and never lifts the hold: it only emits gameplay rows while the
//! Guardian's current token names it, and it reports results through
//! [`GuardianChild::read`].
use super::frame::Frame;
use super::request::{
    AbortReason, Allowances, CombatEnd, CombatRequest, EscapeReserve, Fallback, IntruderPolicy,
    IntruderRule, PrepItem, PrepReadiness, Style, Tactic, Target, Unprotected,
};
use super::schedule::Schedule;
use super::tables::CombatTables;
use super::threats::StyleObs;
use super::{select, Combat, FoodReserve, WalkGuard};
use crate::native::ledger::Runtime;
use crate::native::{
    ActionContext, ActionError, ActionHandle, HostAction, NativeActions, PvpGlobals,
    RetainedMemory, WalkAllow, WalkGoals, WalkRequest,
};
use api::game_data::SelectedGameData;
use api::random::{
    FightActor, FightCancel, FightEvent, FightOffer, FightOutcome, FightPhase, FightPrep,
    FightRead, FightReadiness, FightReason, FightToken,
};
use api::selected::RunKey;
use api::snapshot::SnapshotView;
use std::sync::Arc;
use std::task::Poll;

/// The child's run is never a card's: card runs count up from zero.
const CHILD_RUN_BIT: u64 = 1 << 63;

/// The attack interval the content gives an NPC that sets none
/// (`combat.param:76-79`).
const DEFAULT_ATTACK_INTERVAL: i32 = 4;

/// The run identity of the child for `serial` in a slot incarnation and work
/// epoch.
pub(crate) fn child_run(slot: u64, serial: u32, session: u64) -> RunKey {
    RunKey {
        slot,
        run: CHILD_RUN_BIT | u64::from(serial),
        session,
    }
}

/// The content facts of an event actor that price its escape: its greatest
/// unprotected hit across the styles the content gives it, and its fastest
/// attack interval. `None` when a needed fact is missing: unknown danger is
/// never zero danger.
pub fn escape_reserve(tables: &CombatTables, npc_type: i32) -> Option<EscapeReserve> {
    let row = tables.npc(npc_type)?;
    let hit = [StyleObs::Melee, StyleObs::Ranged]
        .into_iter()
        .filter_map(|style| select::facts::npc_style_max_hit(row, style))
        .max()
        .or_else(|| {
            // The golem's attack handler is bespoke, so the generic facts say
            // nothing about it: its two styles come from its own stats.
            let golem = FightEvent::of_npc_type(usize::try_from(npc_type).ok()?)
                == Some(FightEvent::RockGolem);
            golem.then(|| select::facts::golem_max_hit(row)).flatten()
        })?;
    let interval = row
        .attackrate
        .filter(|rate| *rate > 0)
        .unwrap_or(DEFAULT_ATTACK_INTERVAL);
    Some(EscapeReserve {
        hit,
        interval: u8::try_from(interval).ok()?,
    })
}

/// The style the held setup fights in: a wielded ranged weapon fights ranged,
/// a staff fights with magic, anything else, even bare hands, fights melee.
pub fn held_style(tables: &CombatTables, frame: &Frame<'_>) -> Style {
    let weapon = frame.equipment.iter().find(|row| row.slot == 3);
    let Some(style) = weapon.and_then(|row| tables.weapon_style(row.def.id)) else {
        return Style::Melee;
    };
    match style.style {
        2 => Style::Ranged,
        4 => Style::Mage,
        _ => Style::Melee,
    }
}

/// The Combat request of one event fight: `Tactic::Open`, `Fallback::Abort`,
/// the exact actor, and only what the held setup and the shared core allow.
/// It buys, banks and wears nothing, drinks no boost, and never picks a
/// player or another attacker (design V11D 5.2).
pub fn fight_request(
    offer: &FightOffer,
    style: Style,
    reserve: EscapeReserve,
    budget_ticks: u16,
) -> CombatRequest {
    CombatRequest {
        target: Target::ExactNpc {
            index: offer.actor.index,
            npc_type: i32::from(offer.actor.npc_type),
        },
        tactic: Tactic::Open,
        style,
        budget_ticks: budget_ticks.max(1),
        allow: Allowances {
            prayer: true,
            food: true,
            // Cures run through the shared core's cure policy, not this flag:
            // it gates boosts, prayer sips and antifire, none of which Fight
            // consumes.
            potions: false,
            equipment: false,
            retaliate_toggle: true,
            locked_slots: 0,
            special: false,
        },
        fallback: Fallback::Abort,
        intruder: IntruderPolicy {
            player: IntruderRule::Ignore,
            npc: IntruderRule::Ignore,
        },
        escape_reserve: Some(reserve),
        ..CombatRequest::default()
    }
}

/// Map the end of the shared core to the port's outcome (design V11D 6.1).
/// Every variant is named: a new core end needs an explicit mapping here.
/// `actor_in_view` is whether the exact actor still shows in the scene, and
/// `budget_left` whether the episode has time left.
pub fn outcome_of_end(end: CombatEnd, actor_in_view: bool, budget_left: bool) -> FightOutcome {
    match end {
        CombatEnd::Killed => FightOutcome::Killed,
        CombatEnd::TargetGone => FightOutcome::TargetGone,
        CombatEnd::NoTarget if !actor_in_view => FightOutcome::TargetGone,
        CombatEnd::NoTarget => FightOutcome::FleeRequired(FightReason::NoTarget),
        CombatEnd::Budget if budget_left => FightOutcome::FleeRequired(FightReason::CombatBudget),
        CombatEnd::Budget => FightOutcome::Failed(FightReason::EpisodeBudget),
        CombatEnd::Died => FightOutcome::Died,
        CombatEnd::Aborted(reason) => match reason {
            // An Open, Abort fight raises the no-food abort when the escape
            // reserve is crossed, so it is the reserve in this context.
            AbortReason::Unprotected(Unprotected::NoFood) => {
                FightOutcome::FleeRequired(FightReason::EscapeReserve)
            }
            AbortReason::Unprotected(Unprotected::Dragonfire) => {
                FightOutcome::FleeRequired(FightReason::Dragonfire)
            }
            AbortReason::Unprotected(Unprotected::NoAmmo) => {
                FightOutcome::FleeRequired(FightReason::NoAmmo)
            }
            AbortReason::Unprotected(Unprotected::NoRunes) => {
                FightOutcome::FleeRequired(FightReason::NoRunes)
            }
            AbortReason::Unattackable => FightOutcome::FleeRequired(FightReason::Unattackable),
            AbortReason::SingleWayBlocked => {
                FightOutcome::FleeRequired(FightReason::SingleWayBlocked)
            }
            AbortReason::Unresponsive => FightOutcome::FleeRequired(FightReason::Unresponsive),
            AbortReason::PrepFailed(item) => {
                FightOutcome::FleeRequired(FightReason::Preparation(match item {
                    PrepItem::Weapon => FightPrep::Weapon,
                    PrepItem::Shield => FightPrep::Shield,
                    PrepItem::Ammo => FightPrep::Ammo,
                    PrepItem::Staff => FightPrep::Staff,
                    PrepItem::Arm => FightPrep::Arm,
                }))
            }
            AbortReason::PrepReadiness(readiness) => {
                FightOutcome::FleeRequired(FightReason::Readiness(match readiness {
                    PrepReadiness::CombatRootMissing => FightReadiness::CombatRootMissing,
                    PrepReadiness::CombatRootWrong => FightReadiness::CombatRootWrong,
                    PrepReadiness::StyleEchoMissing => FightReadiness::StyleEchoMissing,
                }))
            }
            // Open and Abort never raise these; if they appear they still
            // never imply success.
            AbortReason::SafespotBroken | AbortReason::Retreated | AbortReason::RetreatFailed => {
                FightOutcome::FleeRequired(FightReason::UnexpectedEnd)
            }
            AbortReason::PvpRefused(_) => FightOutcome::Failed(FightReason::InvalidEventTarget),
        },
    }
}

/// The reason an action error from the core ends the child.
fn outcome_of_error(error: &ActionError) -> FightOutcome {
    match error {
        ActionError::Cancelled | ActionError::Stale => {
            FightOutcome::FleeRequired(FightReason::AdmissionFailed)
        }
        ActionError::Unavailable(_) | ActionError::PvpRefused(_) => {
            FightOutcome::FleeRequired(FightReason::UnknownFacts)
        }
        ActionError::Held | ActionError::Busy | ActionError::BudgetExhausted => {
            FightOutcome::FleeRequired(FightReason::AdmissionFailed)
        }
        _ => FightOutcome::Failed(FightReason::Internal),
    }
}

/// One walk the Guardian would send, as the escape estimate needs it.
fn assessment_request(
    offer: &FightOffer,
    here: api::WorldTile,
    cx: &ActionContext<'_>,
) -> Result<WalkRequest, FightReason> {
    let goals = offer
        .escape
        .ring
        .iter()
        .map(|&(x, z)| api::WorldTile {
            x,
            z,
            level: here.level,
        })
        .collect::<Vec<_>>();
    let goals = WalkGoals::new(goals).map_err(|_| FightReason::UnknownFacts)?;
    let target = goals.as_slice()[0];
    Ok(WalkRequest {
        target,
        candidate_goals: Some(goals),
        loc_id: None,
        radius: 0,
        arrival: nav_planner::contract::ArrivalKind::Area,
        options: Default::default(),
        required_after: cx.evidence(),
        evidence: None,
        cross: Box::default(),
        avoid: Box::default(),
        protect: false,
        food_guard: false,
        cure_guard: false,
        allow: WalkAllow::default(),
    })
}

/// Where the child stands.
enum State {
    /// Offered: the escape estimate and admission are due by the next fresh
    /// observation.
    Offered,
    /// Combat is running against the exact actor. The handle is boxed: it is
    /// the one large variant, and only a fighting child holds it.
    Fighting(Box<ActionHandle<Combat>>),
    /// Combat ended with Flee required; one upkeep guard serves the walk.
    Escaping,
    /// Nothing is left to do.
    Done,
}

/// What the child keeps of the escape estimate: the assessed walking ticks,
/// the tile they were assessed from, and the hold allowance they carry. A
/// player a step or two off that tile pays the distance on top of the
/// assessed walk.
#[derive(Debug, Clone, Copy)]
struct Estimate {
    walk: u16,
    from: (i32, i32),
    slack: u16,
}

/// The lock-wait ticks the walk out may spend after its first click (design
/// V11D 6.2): the assessed bound `W` less the movement ticks of the assessed
/// route, saturating at zero. An exact-step bound has none.
pub(crate) fn escape_allowance(route_ticks: u32, route_moves: u32) -> u16 {
    u16::try_from(route_ticks.saturating_sub(route_moves)).unwrap_or(u16::MAX)
}

/// Whether the walk out admits no lock-producing upkeep ahead of its next
/// click (design V11D 6.2). Before the first click it admits one, the prefix
/// the reserve priced; after it, upkeep waits once the allowance is spent.
fn defers_locks(clicked: bool, allowance: u16, lock_ops: u8) -> bool {
    if clicked {
        allowance == 0
    } else {
        lock_ops > 0
    }
}

/// The child's state, apart from the ledger it steps against.
pub(crate) struct Inner {
    offer: FightOffer,
    tables: Arc<CombatTables>,
    protected: Option<Arc<[FoodReserve]>>,
    state: State,
    outcome: FightOutcome,
    /// The pending advisory walk, awaiting its receipt.
    assess: Option<u64>,
    estimate: Option<Estimate>,
    /// The first observed tick of this child: the estimate is due by the next.
    first_tick: Option<u64>,
    observed_tick: u32,
    /// Fresh observations the fight used, for the combat-budget mapping.
    ticks: u16,
    last_tick: Option<u64>,
    /// The core's clocks and unsettled bite as of its last poll, for the hand
    /// over to the upkeep guard.
    schedule: Schedule,
    bite: Option<super::guard::PendingBite>,
    guard: Option<WalkGuard>,
    blocked_until: Option<u16>,
    /// The lock-wait ticks the walk out may still spend after its first click:
    /// the estimate's hold allowance, spent one tick per fresh tick a lock
    /// withholds a click.
    allowance: u16,
    /// The first click of the walk out reached the Driver.
    clicked: bool,
    /// The last fresh tick that spent the allowance.
    spent_tick: Option<u16>,
    /// Prayers the core raised and had not restored when the child ended.
    prayers: super::prayer::RaisedPrayers,
}

/// One live Guardian Fight, with the ledger it steps against.
pub struct GuardianChild {
    run: RunKey,
    runtime: Runtime,
    retained: RetainedMemory,
    revision: api::selected::ClientRevision,
    inner: Inner,
}

impl GuardianChild {
    /// Begin a child for `offer`. It holds the selected-revision combat tables
    /// and the suspended run's food reserve, and nothing else of it.
    pub fn begin(
        offer: FightOffer,
        run: RunKey,
        selected: &Arc<SelectedGameData>,
        protected: Option<Arc<[FoodReserve]>>,
    ) -> Result<Self, FightReason> {
        if FightEvent::of_npc_type(usize::from(offer.actor.npc_type)) != Some(offer.event) {
            return Err(FightReason::NotEligible);
        }
        let tables =
            CombatTables::build(Arc::clone(selected)).map_err(|_| FightReason::UnknownFacts)?;
        escape_reserve(&tables, i32::from(offer.actor.npc_type))
            .ok_or(FightReason::UnknownFacts)?;
        let revision = crate::slot::compiled::selected_revision(selected)
            .map_err(|_| FightReason::UnknownFacts)?;
        Ok(Self {
            revision,
            run,
            runtime: Runtime::default(),
            retained: RetainedMemory::default(),
            inner: Inner {
                offer,
                tables,
                protected,
                state: State::Offered,
                outcome: FightOutcome::Pending,
                assess: None,
                estimate: None,
                first_tick: None,
                observed_tick: 0,
                ticks: 0,
                last_tick: None,
                schedule: Schedule::default(),
                bite: None,
                guard: None,
                blocked_until: None,
                allowance: 0,
                clicked: false,
                spent_tick: None,
                prayers: super::prayer::RaisedPrayers::empty(),
            },
        })
    }

    /// The token the Guardian offered this fight under.
    pub fn token(&self) -> FightToken {
        self.inner.offer.token
    }

    /// The child's run: the owner every action of this fight carries.
    pub fn run(&self) -> RunKey {
        self.run
    }

    /// The exact actor this child fights.
    pub fn actor(&self) -> FightActor {
        self.inner.offer.actor
    }

    /// What the Guardian reads each tick.
    pub fn read(&self) -> FightRead {
        self.inner.read()
    }

    /// Whether this child may still send gameplay: its fight runs and no
    /// revocation or end has been recorded.
    pub fn fighting(&self) -> bool {
        matches!(self.inner.state, State::Fighting(_)) || self.inner.estimating()
    }

    /// Whether the fight ended with Flee required and the child's upkeep
    /// guard is serving the walk out.
    pub fn escaping(&self) -> bool {
        matches!(self.inner.state, State::Escaping)
    }

    /// Whether the child is finished and holds nothing.
    pub fn done(&self) -> bool {
        matches!(self.inner.state, State::Done)
    }

    /// Revoke the fight now, before the next dispatch: the core's authority
    /// ends, the cancellation is recorded and the admitted prayer cleanup is
    /// kept for the host. The lock end of the upkeep guard is published
    /// before the guard is dropped.
    pub fn revoke(&mut self, cancel: FightCancel) {
        self.inner.revoke(cancel);
        self.runtime.revoke();
    }

    /// One observation, in a frame the host built. The caller states whether
    /// the child may act: the session is current and ready apart from the
    /// Guardian's own hold, and no manual takeover holds the account.
    pub(crate) fn step(&mut self, ctx: &crate::ctx::ScriptCtx<'_>, online: bool) {
        if self.inner.outcome.is_terminal() && !matches!(self.inner.state, State::Escaping) {
            return;
        }
        self.runtime
            .clock
            .observe(std::time::Instant::now(), online);
        let mut cx = crate::slot::compiled::frame_context(
            ctx,
            self.run,
            self.revision,
            &mut self.retained,
            &mut self.runtime,
        );
        cx.eligible = online;
        cx.protected_items = self.inner.protected.clone();
        self.inner.step(&mut cx, online);
    }

    /// The next action this child admitted, with its custody released to the
    /// host dispatch. `None` when nothing is queued.
    pub fn take_action(&mut self) -> Option<HostAction> {
        let ledger = self.runtime.ledger.as_mut()?;
        while !ledger.outbox.is_empty() {
            let mut action = ledger.outbox.remove(0);
            if action.live() {
                action.release_custody();
                return Some(action);
            }
        }
        None
    }

    /// Whether an admitted action waits for the host dispatch.
    pub fn has_actions(&self) -> bool {
        self.runtime
            .ledger
            .as_ref()
            .is_some_and(|ledger| ledger.outbox.iter().any(HostAction::live))
    }

    /// The ledger a host receipt for `run` belongs to, when `run` is this
    /// child's.
    pub(crate) fn ledger_for(
        &mut self,
        run: RunKey,
    ) -> Option<&mut Option<Box<crate::native::ledger::Ledger>>> {
        (run == self.run).then_some(&mut self.runtime.ledger)
    }

    /// Run `upkeep` on the single upkeep guard of the walk out, once for
    /// this observation, and publish the lock it leaves. Returns whether
    /// `tick` is blocked: no outward click may go out. `None` when no guard
    /// serves an escape.
    pub fn escape_upkeep(
        &mut self,
        tick: u16,
        upkeep: impl FnOnce(&mut Option<WalkGuard>),
    ) -> Option<bool> {
        self.inner.escape_upkeep(tick, upkeep)
    }

    /// The first click of the walk out reached the Driver: from now on a
    /// lock that withholds a click spends the allowance.
    pub fn escape_sent(&mut self) {
        self.inner.clicked = true;
    }

    /// Prayers the core raised and had not restored, taken once: the host
    /// restores them as it does after a Stop.
    pub fn take_prayer_cleanup(&mut self) -> super::prayer::RaisedPrayers {
        std::mem::replace(
            &mut self.inner.prayers,
            super::prayer::RaisedPrayers::empty(),
        )
    }
}

impl Inner {
    fn estimating(&self) -> bool {
        matches!(self.state, State::Offered)
    }

    fn read(&self) -> FightRead {
        let phase = match &self.state {
            State::Offered => FightPhase::Offered,
            State::Fighting(handle) => {
                if handle.inspect(Combat::winding_down).unwrap_or(false) {
                    FightPhase::Ending
                } else {
                    FightPhase::Fighting
                }
            }
            State::Escaping => FightPhase::Escaping,
            State::Done => FightPhase::Done,
        };
        FightRead {
            token: self.offer.token,
            phase,
            outcome: self.outcome,
            blocked_until: self.blocked_until,
            observed_tick: self.observed_tick,
        }
    }

    /// Record the lock the guard still holds at `tick`, if any, and say
    /// whether `tick` is blocked.
    fn publish_lock(&mut self, tick: u16) -> bool {
        let end = self.guard.as_ref().and_then(|guard| guard.lock_end(tick));
        if end.is_some() {
            self.blocked_until = end;
        } else if self
            .blocked_until
            .is_some_and(|until| super::schedule::reached(tick, until))
        {
            self.blocked_until = None;
        }
        end.is_some()
    }

    /// See [`GuardianChild::escape_upkeep`]. Before the first click the guard
    /// admits one lock-producing upkeep, the prefix the reserve priced. After
    /// the first click each fresh tick a lock withholds spends one tick of the
    /// allowance, and at zero the guard admits no lock-producing upkeep ahead
    /// of the next click.
    fn escape_upkeep(
        &mut self,
        tick: u16,
        upkeep: impl FnOnce(&mut Option<WalkGuard>),
    ) -> Option<bool> {
        if !matches!(self.state, State::Escaping) {
            return None;
        }
        let lock_ops = self.guard.as_ref().map_or(0, WalkGuard::lock_ops);
        let defer = defers_locks(self.clicked, self.allowance, lock_ops);
        if let Some(guard) = self.guard.as_mut() {
            guard.defer_lock_upkeep(defer);
        }
        upkeep(&mut self.guard);
        let blocked = self.publish_lock(tick);
        if self.clicked && blocked && self.spent_tick != Some(tick) {
            self.spent_tick = Some(tick);
            self.allowance = self.allowance.saturating_sub(1);
        }
        Some(blocked)
    }

    fn revoke(&mut self, cancel: FightCancel) {
        // The lock end leaves before the guard is dropped.
        if let Some(guard) = &self.guard {
            let tick = self.last_tick.map_or(0, |tick| tick as u16);
            if let Some(end) = guard.lock_end(tick) {
                self.blocked_until = Some(end);
            }
        }
        if let State::Fighting(handle) = &self.state {
            self.prayers = handle.prayer_cleanup();
        }
        if let Some(guard) = self.guard.take() {
            let owed = guard.prayer_cleanup();
            self.prayers.merge(owed);
        }
        self.state = State::Done;
        if !self.outcome.is_terminal() || matches!(self.outcome, FightOutcome::FleeRequired(_)) {
            self.outcome = FightOutcome::Cancelled(cancel);
        }
    }

    fn finish(&mut self, outcome: FightOutcome) {
        self.outcome = outcome;
        self.state = State::Done;
    }

    fn step(&mut self, cx: &mut ActionContext<'_>, online: bool) {
        let evidence = cx.evidence();
        self.observed_tick = evidence.tick as u32;
        let fresh = self.last_tick != Some(evidence.tick);
        self.last_tick = Some(evidence.tick);
        let first = *self.first_tick.get_or_insert(evidence.tick);
        if fresh && matches!(self.state, State::Fighting(_)) {
            self.ticks = self.ticks.saturating_add(1);
        }
        if !online {
            // A held or lost session: nothing is sent. The Guardian decides
            // what its next tick does about the child.
            return;
        }
        match &self.state {
            State::Offered => self.step_offered(cx, first),
            State::Fighting(_) => self.step_fighting(cx),
            State::Escaping | State::Done => {}
        }
    }

    /// The offer's estimate and admission, due by the next fresh observation.
    fn step_offered(&mut self, cx: &mut ActionContext<'_>, first: u64) {
        let evidence = cx.evidence();
        if self.assess.is_none() {
            let here = match Frame::borrow(cx.snapshot()) {
                Some(frame) => frame.here,
                None => return,
            };
            let request = match assessment_request(&self.offer, here, cx) {
                Ok(request) => request,
                Err(reason) => return self.finish(FightOutcome::FleeRequired(reason)),
            };
            match cx.assess_walk(request) {
                Ok(id) => self.assess = Some(id),
                Err(_) => return self.finish(FightOutcome::FleeRequired(FightReason::NoService)),
            }
        }
        let id = self.assess.expect("the advisory walk is pending");
        match cx.assess_receipt(id) {
            Some(receipt) => {
                let ok = receipt.refusal.is_none() && receipt.route_ticks > 0;
                let walk = u16::try_from(receipt.route_ticks).unwrap_or(u16::MAX);
                let slack = escape_allowance(receipt.route_ticks, receipt.route_moves);
                cx.cancel_request(id);
                self.assess = None;
                if !ok {
                    return self.finish(FightOutcome::FleeRequired(FightReason::UnknownFacts));
                }
                let from = Frame::borrow(cx.snapshot())
                    .map_or((0, 0), |frame| (frame.here.x, frame.here.z));
                self.estimate = Some(Estimate { walk, from, slack });
                self.begin_combat(cx);
            }
            None if evidence.tick > first => {
                // The next fresh observation: no estimate means Flee.
                cx.cancel_request(id);
                self.assess = None;
                self.finish(FightOutcome::FleeRequired(FightReason::UnknownFacts));
            }
            None => {}
        }
    }

    /// The walking ticks to price this observation: the assessed walk plus
    /// the distance the player is from the tile it was assessed on.
    fn walk_now(&self, frame: &Frame<'_>) -> u16 {
        let Some(estimate) = self.estimate else {
            return 0;
        };
        let off = (frame.here.x - estimate.from.0)
            .abs()
            .max((frame.here.z - estimate.from.1).abs());
        estimate
            .walk
            .saturating_add(u16::try_from(off).unwrap_or(u16::MAX))
    }

    fn begin_combat(&mut self, cx: &mut ActionContext<'_>) {
        let Some(frame) = Frame::borrow(cx.snapshot()) else {
            return;
        };
        let npc_type = i32::from(self.offer.actor.npc_type);
        let Some(reserve) = escape_reserve(&self.tables, npc_type) else {
            return self.finish(FightOutcome::FleeRequired(FightReason::UnknownFacts));
        };
        let remaining = self.offer.budget_ticks.saturating_sub(self.ticks);
        let style = held_style(&self.tables, &frame);
        let request = Arc::new(fight_request(&self.offer, style, reserve, remaining));
        let mut actions = NativeActions { _private: () };
        let handle = match actions.begin::<Combat>(
            (request, Arc::clone(&self.tables), PvpGlobals::fail_closed()),
            cx,
        ) {
            Ok(handle) => handle,
            Err(ActionError::BudgetExhausted) => return,
            Err(error) => return self.finish(outcome_of_error(&error)),
        };
        handle.set_escape_walk(self.walk_now(&frame));
        self.state = State::Fighting(Box::new(handle));
        self.step_fighting(cx);
    }

    fn step_fighting(&mut self, cx: &mut ActionContext<'_>) {
        let State::Fighting(handle) = &self.state else {
            return;
        };
        let Some(frame) = Frame::borrow(cx.snapshot()) else {
            return;
        };
        let walk = self.walk_now(&frame);
        let in_view = frame.npcs.iter().any(|npc| {
            npc.index == usize::from(self.offer.actor.index)
                && npc.r#type == Some(usize::from(self.offer.actor.npc_type))
        });
        handle.set_escape_walk(walk);
        let mut actions = NativeActions { _private: () };
        match actions.poll(handle, cx) {
            Poll::Pending => {
                if let Some(schedule) = handle.inspect(Combat::handoff_schedule) {
                    self.schedule = schedule;
                }
                self.bite = handle.inspect(Combat::pending_bite).flatten();
            }
            Poll::Ready(Ok(report)) => {
                let budget_left = self.ticks < self.offer.budget_ticks;
                let outcome = outcome_of_end(report.end, in_view, budget_left);
                self.ended(cx, outcome);
            }
            Poll::Ready(Err(error)) => {
                let outcome = outcome_of_error(&error);
                self.ended(cx, outcome);
            }
        }
    }

    /// The core ended. A fight that hands over to Flee keeps one upkeep guard
    /// that starts from the core's clocks and unsettled bite.
    fn ended(&mut self, cx: &mut ActionContext<'_>, outcome: FightOutcome) {
        self.outcome = outcome;
        if let FightOutcome::FleeRequired(_) = outcome {
            if self.start_guard(cx) {
                self.state = State::Escaping;
                self.allowance = self.estimate.map_or(0, |estimate| estimate.slack);
                return;
            }
        }
        self.state = State::Done;
    }

    fn start_guard(&mut self, cx: &mut ActionContext<'_>) -> bool {
        let view: SnapshotView<'_> = cx.snapshot();
        let Ok(request) = assessment_request(
            &self.offer,
            Frame::borrow(view).map_or(
                api::WorldTile {
                    x: 0,
                    z: 0,
                    level: 0,
                },
                |frame| frame.here,
            ),
            cx,
        ) else {
            return false;
        };
        let Ok(mut guard) = WalkGuard::begin_food_only_with_protected(
            &request,
            &view,
            Arc::clone(&self.tables),
            self.protected.clone(),
        ) else {
            return false;
        };
        guard.continue_from(self.schedule, self.bite, &view);
        self.guard = Some(guard);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An escaping child whose upkeep guard holds an input-locking karambwan
    /// bite, as the walk out of a fight that handed over leaves it.
    fn escaping_child() -> (Inner, u16) {
        let (tables, guard, tick) = super::super::guard::tests::karambwan_guard_after_a_bite();
        let offer = FightOffer {
            token: FightToken {
                session: 1,
                serial: 1,
            },
            event: FightEvent::RiverTroll,
            actor: FightActor {
                index: 7,
                npc_type: 1,
            },
            escape: api::random::FightEscape {
                anchor: (0, 0),
                radius: 12,
                ring: [(6, 0); 8],
            },
            budget_ticks: 200,
        };
        let inner = Inner {
            offer,
            tables,
            protected: None,
            state: State::Escaping,
            outcome: FightOutcome::FleeRequired(FightReason::EscapeReserve),
            assess: None,
            estimate: None,
            first_tick: Some(0),
            observed_tick: u32::from(tick) + 1,
            ticks: 0,
            last_tick: Some(u64::from(tick) + 1),
            schedule: Schedule::default(),
            bite: None,
            guard: Some(guard),
            blocked_until: None,
            allowance: 0,
            clicked: false,
            spent_tick: None,
            prayers: super::super::prayer::RaisedPrayers::empty(),
        };
        (inner, tick)
    }

    /// R3-2: a Pause or a Stop that revokes a child whose upkeep guard still
    /// holds the input lock of an admitted karambwan bite publishes the lock end
    /// first, and only then drops the guard. The Guardian sends nothing before
    /// that tick, so no click goes out inside the lock the dropped guard held.
    #[test]
    fn a_pause_or_stop_publishes_the_lock_end_before_the_guard_is_dropped() {
        if !api::test_support::game_data_available() {
            return;
        }
        for cancel in [FightCancel::Paused, FightCancel::Stopped] {
            let (mut inner, tick) = escaping_child();
            assert!(inner.guard.is_some() && inner.blocked_until.is_none());
            inner.revoke(cancel);
            let read = inner.read();
            let end = read
                .blocked_until
                .unwrap_or_else(|| panic!("{cancel:?}: the lock end is published"));
            assert!(
                !super::super::schedule::reached(tick.wrapping_add(1), end),
                "{cancel:?}: the lock still blocks the tick after the bite"
            );
            assert!(
                inner.guard.is_none(),
                "{cancel:?}: the guard is dropped after"
            );
            assert_eq!(read.outcome, FightOutcome::Cancelled(cancel));
            assert_eq!(read.phase, FightPhase::Done);
        }
    }

    /// A guard with no admitted lock publishes none: a revocation that finds
    /// the walk free blocks nothing.
    #[test]
    fn a_revocation_with_no_lock_publishes_none() {
        if !api::test_support::game_data_available() {
            return;
        }
        let (mut inner, tick) = escaping_child();
        inner.last_tick = Some(u64::from(tick) + 40);
        inner.revoke(FightCancel::Stopped);
        assert_eq!(inner.read().blocked_until, None);
        assert!(inner.guard.is_none());
    }

    /// R3-3: the allowance after the first click is the bound less the
    /// movement ticks of the assessed route, saturating at zero. An exact-step
    /// bound, or a route longer than its bound, has none.
    #[test]
    fn the_allowance_is_the_bound_less_the_movement_ticks_and_saturates_at_zero() {
        assert_eq!(escape_allowance(9, 6), 3);
        assert_eq!(escape_allowance(7, 7), 0, "an exact-step bound has none");
        assert_eq!(escape_allowance(4, 6), 0, "saturating at zero");
        assert_eq!(escape_allowance(0, 0), 0);
        assert_eq!(escape_allowance(u32::MAX, 0), u16::MAX, "and at the top");
    }

    /// The deferral rule: one lock-producing upkeep before the first click,
    /// then none ahead of a click once the allowance is spent.
    #[test]
    fn lock_producing_upkeep_waits_after_the_prefix_and_after_a_spent_allowance() {
        assert!(!defers_locks(false, 0, 0), "the prefix is admitted");
        assert!(
            defers_locks(false, 9, 1),
            "a second one waits for the click"
        );
        assert!(!defers_locks(true, 2, 1), "after the click, with allowance");
        assert!(defers_locks(true, 0, 1), "after the click, with none left");
        assert!(
            defers_locks(true, 0, 0),
            "an exact-step bound defers at once"
        );
    }

    /// R3-3: each fresh tick a lock withholds a click spends one tick of the
    /// allowance once the first click went out, and no more than once a tick.
    /// At zero the guard is told to admit no lock-producing upkeep.
    #[test]
    fn a_withheld_tick_after_the_first_click_spends_the_allowance_once() {
        if !api::test_support::game_data_available() {
            return;
        }
        let (mut inner, tick) = escaping_child();
        let end = inner
            .guard
            .as_ref()
            .and_then(|guard| guard.lock_end(tick.wrapping_add(1)))
            .expect("the karambwan bite locks input");
        assert_eq!(end, tick.wrapping_add(4), "locked on the next three ticks");
        inner.allowance = 2;
        // Before the first click the locked tick spends nothing; the bite the
        // guard admitted is its one prefix, so a second one waits.
        let mut defers = None;
        let blocked = inner.escape_upkeep(tick.wrapping_add(1), |guard| {
            defers = guard.as_ref().map(WalkGuard::defers_lock_upkeep);
        });
        assert_eq!(blocked, Some(true));
        assert_eq!(defers, Some(true), "the prefix is spent");
        assert_eq!(inner.allowance, 2);

        inner.clicked = true;
        let mut spent = Vec::new();
        for step in 2..=3u16 {
            for _ in 0..2 {
                let mut defers = None;
                let blocked = inner.escape_upkeep(tick.wrapping_add(step), |guard| {
                    defers = guard.as_ref().map(WalkGuard::defers_lock_upkeep);
                });
                assert_eq!(blocked, Some(true));
                spent.push((inner.allowance, defers));
            }
        }
        // Tick 2 spends one (twice observed, once spent); tick 3 spends the
        // last. The guard is told to wait only once none is left.
        assert_eq!(
            spent,
            [
                (1, Some(false)),
                (1, Some(false)),
                (0, Some(false)),
                (0, Some(true)),
            ]
        );
        // The lock ends: the walk is free, and the allowance stays at zero.
        assert_eq!(inner.escape_upkeep(end, |_| {}), Some(false));
        assert_eq!(inner.allowance, 0);
    }

    /// R3-3: with an exact-step bound the allowance is zero, so a second due
    /// bite after the first click is deferred: the guard is told so before it
    /// proposes anything.
    #[test]
    fn with_an_exact_step_bound_the_guard_defers_a_second_bite_after_the_first_click() {
        if !api::test_support::game_data_available() {
            return;
        }
        let (mut inner, tick) = escaping_child();
        inner.allowance = escape_allowance(7, 7);
        inner.clicked = true;
        let end = tick.wrapping_add(4);
        let mut defers = None;
        let blocked = inner.escape_upkeep(end, |guard| {
            defers = guard.as_ref().map(WalkGuard::defers_lock_upkeep);
        });
        assert_eq!(blocked, Some(false), "the first bite's lock is over");
        assert_eq!(defers, Some(true), "a second bite waits ahead of follow");

        let (mut inner, tick) = escaping_child();
        inner.allowance = escape_allowance(9, 6);
        inner.clicked = true;
        let mut defers = None;
        inner.escape_upkeep(tick.wrapping_add(4), |guard| {
            defers = guard.as_ref().map(WalkGuard::defers_lock_upkeep);
        });
        assert_eq!(defers, Some(false), "a bound with allowance admits it");
    }
}
