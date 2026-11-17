//! The Guardian's side of a saved Fight (design V11D 3, 4 and 6). The Guardian
//! keeps Host ownership of the event the whole time. For one of the six
//! fightable events it offers the exact actor to the script layer through the
//! claim source's port, folds the child's typed outcome on each fresh tick,
//! and answers with the escape solver it always had when the fight cannot go
//! on. It never reads a script's mind: a result is a typed value with a token,
//! and a result of another token changes nothing.
use super::*;

/// How the Guardian answers the hostile event it works.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Response {
    /// The escape solver works the event.
    #[default]
    Flee,
    /// A Fight was offered; the child owes admission by the next fresh
    /// observation.
    CombatPending(FightToken),
    /// The child fights.
    CombatRunning(FightToken),
    /// The child's fight ended; admitted cleanup finishes before the hold lifts.
    CombatEnding(FightToken),
    /// The exact actor died. The event ends when its removal is observed.
    Won,
}

impl Response {
    /// The token of the child the response waits on.
    pub(super) fn token(self) -> Option<FightToken> {
        match self {
            Self::CombatPending(token) | Self::CombatRunning(token) | Self::CombatEnding(token) => {
                Some(token)
            }
            Self::Flee | Self::Won => None,
        }
    }

    /// The escape solver is not working the event: no escape counter runs.
    pub(super) fn in_combat(self) -> bool {
        !matches!(self, Self::Flee)
    }
}

/// What a pass hands the act chain for a Fight beside the snapshot: the live
/// context (the saved choices) and the claim source's port the child is
/// offered through. One value keeps the chain inside the argument bound.
pub(super) struct FightDesk<'a, 'b, 'c> {
    pub(super) context: &'a GuardianLiveContext<'c>,
    pub(super) port: &'a mut Option<&'b mut dyn RandomClaimSource>,
}

/// One escape walk the Guardian stages for the slot observation to send
/// (design V11D 6.2): the existing candidate calculation and its send
/// bookkeeping, packaged so the observation can run it after the upkeep
/// guard, where an admitted input lock is known.
#[derive(Debug, Clone, Copy)]
pub struct EscapeIntent {
    token: FightToken,
    facts: FleeFacts,
    run: EvadeRun,
}

impl EscapeIntent {
    /// The fight whose upkeep guard serves this walk.
    pub fn token(&self) -> FightToken {
        self.token
    }

    /// Send the walk now through `driver`: the first accepted candidate, or the
    /// recovery step the Guardian's own escape takes. The result goes back to
    /// the Guardian, which alone updates its bookkeeping from it.
    pub fn send<D: Driver + ?Sized>(
        mut self,
        driver: &mut D,
        snapshot: &GameSnapshot,
    ) -> SentEscape {
        let fled = flee_step(driver, snapshot, &mut self.run, self.facts);
        SentEscape {
            token: self.token,
            run: self.run,
            fled,
            tick: snapshot.tick(),
        }
    }
}

/// An escape walk the observation sent.
#[derive(Debug, Clone, Copy)]
pub struct SentEscape {
    token: FightToken,
    run: EvadeRun,
    fled: bool,
    tick: u32,
}

impl SentEscape {
    /// The fight the walk belonged to.
    pub fn token(&self) -> FightToken {
        self.token
    }

    /// The observed tick the walk went out on.
    pub fn tick(&self) -> u32 {
        self.tick
    }

    /// Whether a flee walk was accepted.
    pub fn fled(&self) -> bool {
        self.fled
    }
}

/// An admitted input lock blocks the tick: no click, raw or routed, may go
/// out before `until` (low 16 bits of the game tick, wrap-aware).
pub(super) fn lock_blocks(tick: u32, until: u16) -> bool {
    (tick as u16).wrapping_sub(until) >= 0x8000
}

fn reason_of(reason: FightReason) -> ResponseReason {
    match reason {
        FightReason::EscapeReserve | FightReason::NoFood => ResponseReason::EscapeReserve,
        FightReason::UnknownFacts | FightReason::NoTarget => ResponseReason::UnavailableFacts,
        FightReason::NoAmmo
        | FightReason::NoRunes
        | FightReason::Dragonfire
        | FightReason::Preparation(_)
        | FightReason::Readiness(_) => ResponseReason::UnavailableSetup,
        FightReason::EpisodeBudget => ResponseReason::ExhaustedBudget,
        FightReason::NotEligible
        | FightReason::NoService
        | FightReason::AdmissionFailed
        | FightReason::Unattackable
        | FightReason::SingleWayBlocked
        | FightReason::Unresponsive
        | FightReason::CombatBudget
        | FightReason::InvalidEventTarget
        | FightReason::UnexpectedEnd
        | FightReason::Internal => ResponseReason::FightEnded,
    }
}

impl Guardian {
    /// The Evade run in flight, if an Evade is the active family.
    pub(super) fn evade_run_in_flight(&self) -> Option<&EvadeRun> {
        match &self.active.as_ref()?.family {
            Family::Evade(run) => Some(run),
            _ => None,
        }
    }

    fn evade_run_in_flight_mut(&mut self) -> Option<&mut EvadeRun> {
        match &mut self.active.as_mut()?.family {
            Family::Evade(run) => Some(run),
            _ => None,
        }
    }

    /// The fight (or its end) holds the event: no escape counter runs.
    pub(super) fn in_combat(&self) -> bool {
        self.evade_run_in_flight()
            .is_some_and(|run| run.response.in_combat())
    }

    /// An admitted input lock still blocks this tick's clicks.
    pub(super) fn lock_blocked(&self, tick: u32) -> bool {
        self.evade_run_in_flight()
            .and_then(|run| run.lock_until)
            .is_some_and(|until| lock_blocks(tick, until))
    }

    /// The fight token a live child holds for this slot, for the dispatch
    /// gate and the status row.
    pub(super) fn fight_token(&self) -> Option<FightToken> {
        let run = self.evade_run_in_flight()?;
        run.response.token().or(run.escape)
    }

    /// The saved policy of the event named `name` as `context` has it now.
    pub(super) fn saved_policy(context: &GuardianLiveContext<'_>, name: &str) -> RandomEventPolicy {
        context
            .event_policies
            .get(name)
            .copied()
            .unwrap_or_default()
    }

    /// One fresh tick of the fight, before detection: lifecycle revocations,
    /// the sent escape walk and the child's outcome fold into the active
    /// event. A repeated frame poll folds nothing twice.
    pub(super) fn fight_fold(
        &mut self,
        snap: &GameSnapshot,
        port: &mut Option<&mut dyn RandomClaimSource>,
        settings: &ProfileSettings,
        context: &GuardianLiveContext<'_>,
        fresh: bool,
    ) {
        if !fresh {
            return;
        }
        let Some(token) = self
            .evade_run_in_flight()
            .and_then(|run| run.response.token().or(run.escape))
        else {
            self.fight_close_orphan(port);
            return;
        };
        let Some(run) = self.evade_run_in_flight().copied() else {
            return;
        };
        let Some(source) = port.as_deref_mut() else {
            // No claim source means no child: nothing serves the fight.
            self.fight_to_flee(None, None, ResponseReason::Cancelled);
            return;
        };
        let name = self
            .evade
            .as_ref()
            .map_or("", |state| state.name.as_str())
            .to_owned();
        // Live changes revoke before anything else dispatches. A change of
        // policy or toggle keeps the episode's Host ownership.
        if run.response.token().is_some() {
            if !settings.random_events {
                source.fight_revoke(token, FightCancel::HandlingOff);
            } else if Self::saved_policy(context, &name) != RandomEventPolicy::Fight {
                source.fight_revoke(token, FightCancel::SettingChanged);
            }
        }
        let read = source.fight_read(token);
        let sent = source.fight_take_sent(token);
        let Some(read) = read else {
            // The slot no longer holds the child: its session or run ended.
            self.fight_to_flee(None, None, ResponseReason::Cancelled);
            self.fight_open = None;
            return;
        };
        if let Some(sent) = sent {
            if let Some(run) = self.evade_run_in_flight_mut() {
                // Only the flee bookkeeping returns; the response stays.
                let response = run.response;
                *run = EvadeRun {
                    response,
                    ..sent.run
                };
                run.staged = false;
            }
            if sent.fled {
                if let Some(state) = self.evade.as_mut() {
                    state.has_fled = true;
                }
            }
        }
        match read.outcome {
            FightOutcome::Pending => {
                let response = match read.phase {
                    FightPhase::Offered => Response::CombatPending(token),
                    FightPhase::Ending => Response::CombatEnding(token),
                    FightPhase::Fighting | FightPhase::Escaping | FightPhase::Done => {
                        Response::CombatRunning(token)
                    }
                };
                if let Some(run) = self.evade_run_in_flight_mut() {
                    run.response = response;
                    run.lock_until = read.blocked_until;
                }
            }
            FightOutcome::Killed => {
                source.fight_release(token);
                self.fight_spend();
                if let Some(run) = self.evade_run_in_flight_mut() {
                    run.response = Response::Won;
                    run.lock_until = None;
                }
            }
            FightOutcome::TargetGone => {
                source.fight_release(token);
                // A bite the core admitted before the actor left still locks
                // input: the walk waits for it.
                self.fight_to_flee(None, read.blocked_until, ResponseReason::FightEnded);
            }
            FightOutcome::FleeRequired(reason) => {
                // The child keeps one upkeep guard while it can; the walk out
                // is staged through it.
                let serves = read.phase == FightPhase::Escaping;
                if !serves {
                    source.fight_release(token);
                }
                self.fight_to_flee(
                    serves.then_some(token),
                    read.blocked_until,
                    reason_of(reason),
                );
                self.fight_start_counter(snap);
            }
            FightOutcome::Cancelled(_) => {
                source.fight_release(token);
                self.fight_to_flee(None, read.blocked_until, ResponseReason::Cancelled);
                self.fight_start_counter(snap);
            }
            FightOutcome::Died => {
                source.fight_release(token);
                self.fight_spend();
                // The death system owns what follows: no walk back.
                self.end_solver();
                self.end_any_family();
            }
            FightOutcome::Failed(reason) => {
                source.fight_release(token);
                if reason == FightReason::EpisodeBudget {
                    self.fight_spend();
                    if let Some(state) = self.evade.as_mut() {
                        state.gave_up = true;
                    }
                    self.end_solver();
                    self.end_any_family();
                } else {
                    self.fight_to_flee(None, read.blocked_until, reason_of(reason));
                    self.fight_start_counter(snap);
                }
            }
        }
        // Every outcome that released the child left no token with the run;
        // one that still holds it keeps it open for the next fold.
        if self.fight_token().is_none() {
            self.fight_open = None;
        }
    }

    /// Close a child no Evade run holds any more: another family replaced the
    /// active event, or the episode ended. It is revoked and released, so no
    /// child fights on unobserved, and its actor's Fight is spent: Fight never
    /// restarts for an actor in the episode it was offered for (design V11D
    /// 4.4). It runs at the start of a fresh pass and once more after the
    /// families ran, so the dispatch gate closes the pass the owner changes.
    pub(super) fn fight_close_orphan(&mut self, port: &mut Option<&mut dyn RandomClaimSource>) {
        if self.fight_token().is_some() {
            return;
        }
        let Some(actor) = self.fight_open.take() else {
            return;
        };
        if let Some(source) = port.as_deref_mut() {
            let token = FightToken {
                session: self.episode.session,
                serial: self.fight_serial,
            };
            source.fight_revoke(token, FightCancel::Replaced);
            source.fight_release(token);
        }
        self.fight_spent = Some(actor);
    }

    /// Fight never restarts for this actor in this episode.
    fn fight_spend(&mut self) {
        self.fight_spent = self
            .evade
            .as_ref()
            .map(|state| (state.index, state.npc_type));
    }

    /// End the fight response and answer with the escape solver. `serves` is
    /// the child whose upkeep guard serves the walk out.
    fn fight_to_flee(
        &mut self,
        serves: Option<FightToken>,
        lock_until: Option<u16>,
        why: ResponseReason,
    ) {
        self.fight_spend();
        if let Some(run) = self.evade_run_in_flight_mut() {
            run.response = Response::Flee;
            run.escape = serves;
            run.lock_until = lock_until;
            run.staged = false;
            run.why = Some(why);
        }
    }

    /// The first handoff to Flee starts the escape counters once, at the
    /// current distance from the unchanged spawn anchor.
    fn fight_start_counter(&mut self, snap: &GameSnapshot) {
        let Some(player) = snap.tile().map(|(x, z, _)| (x, z)) else {
            return;
        };
        let started = self.evade_run_in_flight().is_some_and(|run| run.from_fight);
        if started {
            return;
        }
        if let Some(run) = self.evade_run_in_flight_mut() {
            run.from_fight = true;
        }
        if let Some(state) = self.evade.as_mut() {
            state.furthest_distance = cheb(player, state.start_tile);
            state.no_progress_ticks = 0;
        }
    }

    /// Whether a Fight may be offered for the event now: the episode began
    /// under a saved Fight that still holds, the actor is a fightable event of
    /// the exact family its name says, and Fight has not already been spent
    /// for this actor.
    fn fight_wanted(
        &self,
        context: &GuardianLiveContext<'_>,
        settings: &ProfileSettings,
    ) -> Option<(FightEvent, FightActor)> {
        let state = self.evade.as_ref()?;
        if !settings.random_events
            || !self.episode_fight
            || state.gave_up
            || state.maxrange.is_none()
            || self.fight_spent == Some((state.index, state.npc_type))
            || Self::saved_policy(context, &state.name) != RandomEventPolicy::Fight
        {
            return None;
        }
        let npc_type = state.npc_type?;
        let event = api::random::fight_admission(&state.name, Some(npc_type))?;
        Some((
            event,
            FightActor {
                index: u16::try_from(state.index).ok()?,
                npc_type: u16::try_from(npc_type).ok()?,
            },
        ))
    }

    /// Offer the Fight of this episode, once, at the first escape step of an
    /// actor that was never fought. A refusal is a typed reason and the
    /// escape solver works the event as it always did.
    pub(super) fn fight_begin(
        &mut self,
        port: &mut Option<&mut dyn RandomClaimSource>,
        context: &GuardianLiveContext<'_>,
        settings: &ProfileSettings,
    ) -> bool {
        let Some((event, actor)) = self.fight_wanted(context, settings) else {
            return false;
        };
        let Some(source) = port.as_deref_mut() else {
            return false;
        };
        let Some(state) = self.evade.as_ref() else {
            return false;
        };
        let actor_id = (state.index, state.npc_type);
        self.fight_serial = self.fight_serial.wrapping_add(1);
        let token = FightToken {
            session: self.episode.session,
            serial: self.fight_serial,
        };
        // The child prices the walk the escape would take from the live scene
        // on every observation, so the offer names only where it starts and
        // how far it goes.
        let offer = FightOffer {
            token,
            event,
            actor,
            escape: FightEscape {
                anchor: state.start_tile,
                radius: state.flee_distance,
            },
            budget_ticks: EVADE_GIVE_UP_TICKS.saturating_sub(state.elapsed_ticks),
        };
        match source.fight_offer(&offer) {
            Ok(()) => {
                self.fight_open = Some(actor_id);
                if let Some(run) = self.evade_run_in_flight_mut() {
                    run.response = Response::CombatPending(token);
                }
                api::host_log!(
                    Category::RandomEvent,
                    Level::Info,
                    "fight: offered {:?} slot {} to the script layer",
                    event,
                    actor.index
                );
                true
            }
            Err(reason) => {
                api::host_log!(
                    Category::RandomEvent,
                    Level::Info,
                    "fight: {:?} not fought ({reason:?}); fleeing instead",
                    event
                );
                self.fight_spend();
                if let Some(run) = self.evade_run_in_flight_mut() {
                    run.why = Some(reason_of(reason));
                }
                false
            }
        }
    }

    /// Stage the escape walk through the child that serves it, once, instead
    /// of sending it before the observation can know an admitted lock. True
    /// when a child serves the escape (the walk is the observation's to send).
    pub(super) fn fight_stage_escape(
        &mut self,
        port: &mut Option<&mut dyn RandomClaimSource>,
        facts: FleeFacts,
        run: EvadeRun,
    ) -> bool {
        let Some(token) = run.escape else {
            return false;
        };
        let Some(source) = port.as_deref_mut() else {
            return false;
        };
        if !run.staged {
            source.fight_stage_escape(EscapeIntent { token, facts, run });
            if let Some(run) = self.evade_run_in_flight_mut() {
                run.staged = true;
            }
        }
        true
    }

    /// The typed response row of this publication (design V11D 7.1).
    pub(super) fn derive_response(
        &self,
        event: Option<&DetectedRandom>,
        toggle: bool,
    ) -> Option<RandomResponse> {
        let event = event?;
        if !toggle {
            return Some(RandomResponse {
                state: ResponseState::HandlingOff,
                reason: None,
            });
        }
        let active = self.active.as_ref();
        if active.is_some_and(|active| matches!(active.owner, Owner::Script { .. })) {
            return Some(RandomResponse {
                state: ResponseState::ScriptOwned,
                reason: None,
            });
        }
        match event.kind {
            RandomKind::Pick => Some(RandomResponse {
                state: ResponseState::Picking,
                reason: None,
            }),
            RandomKind::Evade => {
                let run = self.evade_run_in_flight().copied().unwrap_or_default();
                let excluded = self
                    .episode_fight
                    .then(|| api::random::fight_eligibility(&event.name));
                let released = self.evade.as_ref().is_some_and(|state| state.gave_up);
                let state = if released {
                    ResponseState::Stopped
                } else if run.response.in_combat() {
                    ResponseState::Fighting
                } else {
                    ResponseState::Fleeing
                };
                let reason = if released {
                    Some(ResponseReason::ExhaustedBudget)
                } else if run.response.in_combat() {
                    None
                } else if let Some(why) = run.why {
                    Some(why)
                } else {
                    match excluded.flatten() {
                        Some(api::random::FightEligibility::Excluded(
                            api::random::FightExclusion::Swarm,
                        )) => Some(ResponseReason::Immune),
                        Some(api::random::FightEligibility::Excluded(
                            api::random::FightExclusion::StrangePlant,
                        )) => Some(ResponseReason::NotFought),
                        _ => None,
                    }
                };
                Some(RandomResponse { state, reason })
            }
            _ => None,
        }
    }
}
