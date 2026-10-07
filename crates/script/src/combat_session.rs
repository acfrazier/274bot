//! The native card behind one `api.combat.fight` session.
//!
//! It drives the shared [`Combat`] machine once, through the slot's
//! [`crate::native::NativeActions`], with no change to the machine. It owns
//! only what a session adds around one fight, following Sherlock's card
//! behaviour for prayer ownership (`docs/api/script.md`, "Native combat
//! prayer ownership"):
//!
//! - **Normal end:** Combat's own wind-down clears the prayers it raised, so a
//!   reported fight owes nothing.
//! - **Cancel (Pause, user input, reconnect, machine error):** the accepted
//!   raises captured from the live handle are cleared by a scoped
//!   [`ClearPrayers`] before the session settles. User prayers are untouched.
//! - **Death** retires the obligation: the game cleared every prayer.
//! - **Stop or teardown:** the slot takes [`Script::prayer_cleanup`] before
//!   dropping the card and hands it to the host's off-click pump.
//!
//! The settled outcome is written to the seat's [`SettleCell`] before the
//! card returns [`ScriptFlow::Complete`].
use crate::api_combat::{CombatSummary, InterruptCause};
use crate::combat::{
    begin_clear_owned_prayers, ActorKind, ClearPrayers, Combat, CombatRequest, CombatTables,
    Hygiene, RaisedPrayers,
};
use crate::native::death::{hitpoints_zero, DeathLatch};
use crate::native::{
    ActionError, ActionHandle, Interrupt, NativePhase, NativeTick, Script, ScriptFailure,
    ScriptFlow, ScriptStatus, StatusField, StatusValue,
};
use crate::registry::CompiledId;
use std::sync::{Arc, Mutex};
use std::task::Poll;

/// The status card id; this card is never listed in Browse.
pub(crate) const CARD_ID: CompiledId = CompiledId("CombatSession");

/// How the card settled; the seat adds the token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Settle {
    Fought(CombatSummary),
    Interrupted(InterruptCause),
    Refused(Arc<str>),
    Failed(Arc<str>),
}

/// Written once by the card, read once by the seat after `Complete`.
pub(crate) type SettleCell = Arc<Mutex<Option<Settle>>>;

/// One live native machine. Never both Combat and ClearPrayers.
#[allow(
    clippy::large_enum_variant,
    reason = "Keep the live combat machine inline, as Sherlock does."
)]
enum Fight {
    Combat(ActionHandle<Combat>),
    ClearPrayers(ActionHandle<ClearPrayers>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    Starting,
    Fighting,
    ClearingPrayers,
}

impl Stage {
    fn as_str(self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Fighting => "fighting",
            Self::ClearingPrayers => "clearing-prayers",
        }
    }
}

pub(crate) struct CombatSessionCard {
    request: Arc<CombatRequest>,
    tables: Arc<CombatTables>,
    cell: SettleCell,
    fight: Option<Fight>,
    /// Accepted Combat raises still owed after the fight was cancelled.
    owned: RaisedPrayers,
    /// The decided end, waiting only for scoped prayer cleanup.
    end: Option<Settle>,
    begun: bool,
    death: DeathLatch,
    /// The last published `(stage, engaged kind, engaged index)`.
    published: Option<(Stage, Option<ActorKind>, u16)>,
}

impl CombatSessionCard {
    pub(crate) fn new(
        request: Arc<CombatRequest>,
        tables: Arc<CombatTables>,
        cell: SettleCell,
    ) -> Self {
        Self {
            request,
            tables,
            cell,
            fight: None,
            owned: RaisedPrayers::empty(),
            end: None,
            begun: false,
            death: DeathLatch::default(),
            published: None,
        }
    }

    /// Cancel a live fight, keeping its accepted raises as the obligation.
    fn cancel(&mut self, cause: InterruptCause) {
        match self.fight.take() {
            Some(Fight::Combat(handle)) => {
                self.owned.merge(handle.prayer_cleanup());
                self.end.get_or_insert(Settle::Interrupted(cause));
            }
            // A cancelled cleanup restarts from the retained obligation.
            Some(Fight::ClearPrayers(_)) => {}
            None if !self.begun => {
                self.end.get_or_insert(Settle::Interrupted(cause));
            }
            None => {}
        }
    }

    fn poll_fight(&mut self, tick: &mut NativeTick<'_>) -> Poll<()> {
        match self.fight.as_ref() {
            None => Poll::Ready(()),
            Some(Fight::Combat(handle)) => {
                let owned = handle.prayer_cleanup();
                match tick.actions.poll(handle, &mut tick.cx) {
                    Poll::Pending => Poll::Pending,
                    Poll::Ready(result) => {
                        self.fight = None;
                        self.end = Some(match result {
                            Ok(report) => {
                                // Wind-down already cleared Combat's raises.
                                Settle::Fought(CombatSummary::from(&report))
                            }
                            Err(error) => {
                                self.owned.merge(owned);
                                fight_error(error)
                            }
                        });
                        Poll::Ready(())
                    }
                }
            }
            Some(Fight::ClearPrayers(handle)) => match tick.actions.poll(handle, &mut tick.cx) {
                Poll::Pending => Poll::Pending,
                Poll::Ready(Ok(report)) => {
                    self.fight = None;
                    // A timed-out raise stays owed; the slot hands it to the
                    // host when it drops this settled card.
                    if report.timed_out == 0 {
                        self.owned = RaisedPrayers::empty();
                    }
                    self.settle();
                    Poll::Ready(())
                }
                Poll::Ready(Err(
                    ActionError::Busy
                    | ActionError::Held
                    | ActionError::Stale
                    | ActionError::Cancelled
                    | ActionError::BudgetExhausted,
                )) => {
                    self.fight = None;
                    Poll::Ready(())
                }
                Poll::Ready(Err(_)) => {
                    self.fight = None;
                    self.settle();
                    Poll::Ready(())
                }
            },
        }
    }

    /// Publish the decided end. Any obligation left is the host's to pay.
    fn settle(&mut self) {
        if let Some(end) = self.end.take() {
            *self
                .cell
                .lock()
                .unwrap_or_else(|poison| poison.into_inner()) = Some(end);
        }
    }

    fn settled(&self) -> bool {
        self.cell
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .is_some()
    }

    fn publish(&mut self, tick: &mut NativeTick<'_>) {
        let stage = match self.fight {
            Some(Fight::ClearPrayers(_)) => Stage::ClearingPrayers,
            Some(Fight::Combat(_)) => Stage::Fighting,
            None if self.end.is_some() && !self.owned.is_empty() => Stage::ClearingPrayers,
            None => Stage::Starting,
        };
        let engaged = match &self.fight {
            Some(Fight::Combat(handle)) => handle.inspect(Combat::engaged).flatten(),
            _ => None,
        };
        let view = (
            stage,
            engaged.map(|actor| actor.kind),
            engaged.map_or(0, |actor| actor.index),
        );
        if self.published == Some(view) {
            return;
        }
        self.published = Some(view);
        let kind = match view.1 {
            Some(ActorKind::Npc) => "npc",
            Some(ActorKind::Player) => "player",
            None => "none",
        };
        let index = engaged.map_or(-1, |actor| i64::from(actor.index));
        tick.output.status(ScriptStatus {
            run: tick.cx.run(),
            card: CARD_ID,
            phase: NativePhase::Working,
            active_settings: 1,
            pending_settings: None,
            fields: Arc::from([
                StatusField {
                    key: "stage",
                    label: "Stage",
                    value: StatusValue::Text(Arc::from(stage.as_str())),
                },
                StatusField {
                    key: "engaged_kind",
                    label: "Engaged",
                    value: StatusValue::Text(Arc::from(kind)),
                },
                StatusField {
                    key: "engaged_index",
                    label: "Engaged index",
                    value: StatusValue::Integer(index),
                },
            ]),
            failure: None,
        });
    }
}

fn fight_error(error: ActionError) -> Settle {
    match error {
        ActionError::UserInput => Settle::Interrupted(InterruptCause::UserInput),
        // The slot revokes and rekeys owners at a session boundary.
        ActionError::Stale | ActionError::Cancelled => {
            Settle::Interrupted(InterruptCause::Reconnect)
        }
        other => Settle::Failed(error_reason(&other)),
    }
}

fn error_reason(error: &ActionError) -> Arc<str> {
    match error {
        ActionError::Busy => Arc::from("busy"),
        ActionError::Held => Arc::from("held"),
        ActionError::BudgetExhausted => Arc::from("budget-exhausted"),
        ActionError::Stale => Arc::from("stale"),
        ActionError::Cancelled => Arc::from("cancelled"),
        ActionError::UserInput => Arc::from("user-input"),
        ActionError::Unavailable(why) => Arc::from(format!("unavailable:{why}")),
        ActionError::Failed(why) => Arc::from(format!("failed:{why}")),
        ActionError::Blocked(why) => Arc::from(format!("blocked:{why}")),
        ActionError::NeedsEvidence(_) => Arc::from("needs-evidence"),
    }
}

impl Script for CombatSessionCard {
    fn tick(&mut self, tick: &mut NativeTick<'_>) -> Result<ScriptFlow, ScriptFailure> {
        if self.settled() {
            return Ok(ScriptFlow::Complete);
        }
        if self.death.observe(tick.cx.snapshot()) {
            // Death cleared every prayer: the obligation is retired.
            self.cancel(InterruptCause::Died);
            self.fight = None;
            self.owned = RaisedPrayers::empty();
            self.end
                .get_or_insert(Settle::Interrupted(InterruptCause::Died));
            self.settle();
            return Ok(ScriptFlow::Complete);
        }
        if !tick.cx.eligible
            || !tick
                .frame
                .snapshot
                .is_some_and(|snapshot| snapshot.ingame() && snapshot.scene_state() == 2)
            || hitpoints_zero(tick.frame.snapshot.map(|snapshot| snapshot.stats()))
        {
            self.publish(tick);
            return Ok(ScriptFlow::Continue);
        }
        if self.poll_fight(tick).is_pending() {
            self.publish(tick);
            return Ok(ScriptFlow::Continue);
        }
        if self.settled() {
            return Ok(ScriptFlow::Complete);
        }
        if self.end.is_some() {
            if self.fight.is_none() {
                match begin_clear_owned_prayers(self.tables.selected(), self.owned, tick) {
                    Hygiene::Clean => {
                        self.owned = RaisedPrayers::empty();
                        self.settle();
                        return Ok(ScriptFlow::Complete);
                    }
                    Hygiene::Started(handle) => self.fight = Some(Fight::ClearPrayers(handle)),
                    Hygiene::Deferred => {}
                    Hygiene::Failed(_) => {
                        self.settle();
                        return Ok(ScriptFlow::Complete);
                    }
                }
            }
            self.publish(tick);
            return Ok(ScriptFlow::Continue);
        }
        if !self.begun {
            match tick.actions.begin::<Combat>(
                (Arc::clone(&self.request), Arc::clone(&self.tables)),
                &mut tick.cx,
            ) {
                Ok(handle) => {
                    self.begun = true;
                    self.fight = Some(Fight::Combat(handle));
                }
                Err(ActionError::Held | ActionError::Busy | ActionError::BudgetExhausted) => {}
                Err(ActionError::Unavailable(why)) => {
                    self.begun = true;
                    self.end = Some(Settle::Refused(Arc::from(format!("unavailable:{why}"))));
                    self.settle();
                    return Ok(ScriptFlow::Complete);
                }
                Err(error) => {
                    self.begun = true;
                    self.end = Some(Settle::Failed(error_reason(&error)));
                    self.settle();
                    return Ok(ScriptFlow::Complete);
                }
            }
        }
        self.publish(tick);
        Ok(ScriptFlow::Continue)
    }

    fn interrupt(&mut self, event: Interrupt) {
        match event {
            Interrupt::Pause => self.cancel(InterruptCause::Pause),
            // The seat signals a reconnect: a fight not yet begun has no
            // native handle that could go stale.
            Interrupt::SessionEnded => self.cancel(InterruptCause::Reconnect),
            Interrupt::Resume | Interrupt::Hold(_) | Interrupt::SessionReady => {}
        }
    }

    fn user_input(&mut self) {
        self.cancel(InterruptCause::UserInput);
    }

    fn prayer_cleanup(&self) -> RaisedPrayers {
        let mut owned = self.owned;
        if let Some(Fight::Combat(handle)) = self.fight.as_ref() {
            owned.merge(handle.prayer_cleanup());
        }
        owned
    }
}

#[cfg(test)]
#[path = "combat_session_tests.rs"]
pub(crate) mod tests;
