//! Compiled solve-clue card (`Sherlock`): play, identify, dispatch.
//!
//! The clue machine is [`crate::clue`] — the same token / identify / phase /
//! clock machine the frozen JS wrapper calls through `__rs2b0t_clue`, running
//! on this slot's pump thread. This module owns no second solver: no
//! membership table, no trail table, no classify, no phase and no clock of
//! its own. It owns the two things that wrapper owned besides the machine:
//! marshalling the observed frame onto the machine's payload, and mapping the
//! machine's own verbs onto this slot's interact queue.
//!
//! One [`Script::tick`] is one wrapper `execute()` iteration:
//!
//! * No live session: the `begin` call. A refusal (no held membership, no
//!   selected pin) leaves the card idle and it asks again next tick; a
//!   `token` answer starts the session and this tick keeps iterating.
//! * Live: `next` calls until the machine waits, yields, ends the session or
//!   enqueues one verb — the one verb per tick the wrapper's own
//!   `delayTicks(1)` gives.
//! * `callback.enabled` is the machine's own gate question. The answer is
//!   `true` while this card is started (a compiled card ticks only while the
//!   slot runs it, and there is no settings `enabled` flag); `resume` then
//!   rides exactly the one following call.
//! * `callback.log` and `callback.setStatus` are forwarded to the shared
//!   native output owner without changing the machine's callback ordering.
//! * `grind-ready` continues this tick's iterate: the live-token handback is
//!   not a verb and not a delay, so the collect's own completion — the
//!   pack-full warning, `clue solved`, `grind-ready`, `done` — is reached
//!   inside the one tick that latched it. `supplies-needed` is the named
//!   wait-class: the tick ends there, nothing is fetched, and the token lives
//!   for the pack that posts the tool.
//! * `done`, `dead`, `abandon` and `guardian-lost` are terminal: the session
//!   ends, its token is cleared, and only `done` — the abort a finished
//!   collect dies on — advances the card's local solved count. An `aborted`
//!   answer ends too, on the machine's own reason: `none-held` is the landed
//!   abort and not a finished clue, and it counts nothing.
//! * Native death observation uses the shared `DeathLatch` chat signal. HP
//!   zero alone freezes this card before it can emit work, but does not end the
//!   clue; a newly latched death chat ends the live token without advancing
//!   `solved`, even if HP has already recovered. The shared clue machine's
//!   explicit observation input is supplied only by native callers, so
//!   compatibility callers keep their existing HP-based behavior.
//!
//! Every page is read fail-closed from the observed frame
//! (snapshot, tile and object names through the crate-private host-frame
//! bridge): an absent slot is omitted rather than
//! defaulted, so the machine never reads an invented `-1`, `28` or slot `0`.
//! The `next` page set is the adapter's: required keys always, optional
//! chat / bank / shop / overlay slots only when this frame carried them.
//! [`enqueue`] maps every adapter `ENQUEUED_KINDS` verb onto this slot's
//! interact queue. `walk_missing_carry` is isolate-posted from a walk
//! outcome this compiled tick does not observe, so it stays omitted.
//!
//! Out of scope for this card, on purpose: keyboard, the duel `3554` family,
//! honor-SETTINGS and loadout provisioning.

#[cfg(test)]
use api::game_data::SelectedGameData;
use api::snapshot::{ActorKind, ActorTargetView, GameSnapshot};
use serde_json::{json, Map, Value};

use std::sync::{Arc, LazyLock};

use crate::clue::{Delegation, Outcome};
use crate::combat::{
    begin_clear_owned_prayers, ClearPrayers, Combat, CombatRequest, CombatTables, Hygiene,
    RaisedPrayers,
};
use crate::native::death::hitpoints_zero;
use crate::native::{
    ActionError, ActionHandle, CompiledCard, ConfigError, HostFrame, Interrupt, NativeOutput,
    NativePhase, NativeTick, PrepareContext, PreparedConfig, RetainedMemory, Script, ScriptFailure,
    ScriptFlow, ScriptStatus, SettingsApply, SettingsBag, StartError, StatusField, StatusValue,
};
use crate::shim::{InteractReq, ScriptPaint};
use crate::{CompiledId, SettingDef};
use api::selected::RunKey;
use std::task::Poll;

pub(crate) const CARD: CompiledCard = CompiledCard {
    id: CompiledId("Sherlock"),
    name: "Sherlock",
    description: "Rust-native clue trail solver — waits for a clue and solves it.",
    category: "Treasure Trails",
    schema_version: 1,
    schema: settings_schema,
    per_account_settings: &[],
    prepare,
    create,
};

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SherlockSettings {
    // A profile-global value, not a card control. The existing clue adapter
    // does not implement the duel family; its action extraction owns that.
    #[serde(default, rename = "clueDuelPartner")]
    _clue_duel_partner: String,
    #[serde(default, rename = "allow_teleports")]
    _allow_teleports: bool,
    #[serde(default, rename = "allow_wilderness")]
    _allow_wilderness: bool,
    #[serde(default, rename = "allow_danger_zones")]
    _allow_danger_zones: bool,
}

fn settings_schema() -> &'static [SettingDef] {
    static SETTINGS: LazyLock<Vec<SettingDef>> = LazyLock::new(|| {
        [
            ("allow_teleports", "Allow teleports"),
            ("allow_wilderness", "Allow wilderness"),
            ("allow_danger_zones", "Allow danger zones"),
        ]
        .into_iter()
        .map(|(id, label)| SettingDef {
            id: id.into(),
            ty: "boolean".into(),
            default: Some("false".into()),
            label: Some(label.into()),
            min: None,
            max: None,
            step: None,
            options: Vec::new(),
            option_labels: Vec::new(),
            group: Some("Walk permissions".into()),
            show_if: None,
            options_from: None,
            csv_toggle: None,
            help: Some("Allow this script when the global setting is off.".into()),
            item_option_spec: None,
        })
        .collect()
    });
    &SETTINGS
}

fn prepare(
    cx: &mut PrepareContext<'_>,
    revision: u64,
    bag: Arc<SettingsBag>,
) -> Result<Arc<PreparedConfig>, StartError> {
    if cx
        .selected
        .selected_pin()
        .map_err(StartError::Facts)?
        .as_ref()
        != cx.pin.as_ref()
    {
        return Err(StartError::Facts(api::selected::FactError::PinMismatch));
    }
    if cx.selected.trails().is_none()
        || cx.selected.talk_key().is_none()
        || cx.selected.trio_givers().is_none()
    {
        return Err(StartError::Unavailable(
            "Sherlock clue facts unavailable".into(),
        ));
    }
    let _settings = <SherlockSettings as serde::Deserialize>::deserialize(
        serde::de::value::MapDeserializer::new(
            bag.iter().map(|(key, value)| (key.as_str(), value)),
        ),
    )
    .map_err(|e| StartError::Config(ConfigError::new("", "invalid-settings", e.to_string())))?;
    let tables = CombatTables::build(Arc::clone(&cx.selected))
        .map_err(|_| StartError::Unavailable("Sherlock combat tables unavailable".into()))?;
    Ok(PreparedConfig::new(
        CARD.id,
        CARD.schema_version,
        revision,
        bag,
        Prepared { tables },
    ))
}

struct Prepared {
    tables: Arc<CombatTables>,
}

fn validate_config(config: &PreparedConfig) -> Result<(), ConfigError> {
    if config.card() != CARD.id
        || config.schema_version() != CARD.schema_version
        || config.get::<Prepared>().is_none()
    {
        return Err(ConfigError::new(
            "",
            "config-identity",
            "configuration does not belong to Sherlock",
        ));
    }
    Ok(())
}

fn create(
    run: RunKey,
    config: Arc<PreparedConfig>,
    retained: &mut RetainedMemory,
) -> Result<Box<dyn Script>, StartError> {
    validate_config(&config).map_err(StartError::Config)?;
    let tables = config
        .get::<Prepared>()
        .map(|prepared| Arc::clone(&prepared.tables));
    let retained = retained.sherlock();
    Ok(Box::new(Sherlock {
        run: Some(run),
        revision: config.revision(),
        dirty: true,
        hygiene_pending: false,
        death: crate::native::death::DeathLatch::from_watermark(retained.death_seq),
        death_pending: retained.death_pending,
        tables,
        ..Default::default()
    }))
}

/// The `generation` every call posts. The machine captures it at `begin` and
/// requires the same value back on every `next`; the compiled session is
/// aborted on its own pump instead ([`crate::slot::SlotScript`]), so the stamp
/// repeats the wrapper's own literal rather than carrying a session identity
/// of its own.
const GENERATION: u64 = 0;

/// The machine's own terminal kind for a finished collect and the one end this
/// card counts locally — the string the `end` predicate reads.
const DONE: &str = "done";

/// How many machine calls one tick may make before it waits for the next one.
/// The machine answers a gate question, a log line and a status line at most
/// once each before it waits, yields or enqueues, and a finished collect's own
/// exit is exactly four handoffs — the pack-full warning, the `clue solved`
/// status, the live-token `grind-ready` continue and the `done` behind it — so
/// four is every callback path; the bound is what keeps a changed machine from
/// parking the pump thread on one observed frame.
const CALLBACK_HANDOFFS: usize = 4;

/// One live native machine. Never both Combat and ClearPrayers.
#[allow(
    clippy::large_enum_variant,
    reason = "Keep the live combat machine inline; size_of::<Sherlock>() stays ≤ 1 KiB."
)]
enum Fight {
    Combat(ActionHandle<Combat>),
    ClearPrayers(ActionHandle<ClearPrayers>),
}

/// The compiled solve-clue card.
#[derive(Default)]
pub struct Sherlock {
    /// The live session token; `None` while there is no session to continue.
    token: Option<u64>,
    /// Clue steps this card finished, advanced by the machine's terminal
    /// `done` and published through the shared status/paint projection.
    solved: u32,
    run: Option<RunKey>,
    revision: u64,
    status: Option<Arc<str>>,
    dirty: bool,
    /// Sequence-aware chat death observation. The slot retains its watermark
    /// across watchdog recreation so a death in the gap is still observed.
    death: crate::native::death::DeathLatch,
    /// A latched death waiting for the interaction sink to become available.
    death_pending: bool,
    fight: Option<Fight>,
    combat_id: Option<u32>,
    pending: Option<(u32, Arc<CombatRequest>)>,
    outcome: Option<Outcome>,
    hygiene_pending: bool,
    hygiene_owned: RaisedPrayers,
    tables: Option<Arc<CombatTables>>,
    blocked: Option<ScriptFailure>,
    block_user_input: bool,
}

impl Script for Sherlock {
    fn tick(&mut self, tick: &mut NativeTick<'_>) -> Result<ScriptFlow, ScriptFailure> {
        // A reconnect advances the run's session; publish the current key.
        let run = tick.cx.run();
        if self.run != Some(run) {
            self.run = Some(run);
            self.dirty = true;
        }
        let death_observed = self.death.observe(tick.cx.snapshot());
        if death_observed {
            if self.token.is_some() {
                self.death_pending = true;
            } else {
                // Do not start a new clue on the same frame as a death signal.
                self.cancel_for_death(tick.actions);
                self.persist_death_state(tick);
                self.publish(tick.output);
                return Ok(self
                    .blocked
                    .clone()
                    .map_or(ScriptFlow::Continue, ScriptFlow::Blocked));
            }
        }
        if self.death_pending {
            // The explicit signal is terminal even on a held/loading frame;
            // do not poll combat or emit any other work before aborting it.
            self.cancel_for_death(tick.actions);
            if self.tick_frame(tick, true) && self.token.is_none() {
                self.death_pending = false;
            }
            self.persist_death_state(tick);
            self.publish(tick.output);
            return Ok(self
                .blocked
                .clone()
                .map_or(ScriptFlow::Continue, ScriptFlow::Blocked));
        }
        self.persist_death_state(tick);
        if let Some(failure) = self.blocked.clone() {
            self.publish(tick.output);
            return Ok(ScriptFlow::Blocked(failure));
        }
        if !tick.cx.eligible
            || !tick
                .frame
                .snapshot
                .is_some_and(|snapshot| snapshot.ingame() && snapshot.scene_state() == 2)
            || hitpoints_zero(tick.frame.snapshot.map(|snapshot| snapshot.stats()))
        {
            // Inactive, held and HP-zero frames never advance clue/combat work.
            // The latch still observes chat above, so a death while held is
            // consumed by the terminal branch instead of being lost.
            self.publish(tick.output);
            return Ok(ScriptFlow::Continue);
        }
        match self.poll_fight(tick) {
            Poll::Pending => {
                self.publish(tick.output);
                return Ok(ScriptFlow::Continue);
            }
            Poll::Ready(()) => {}
        }
        if self.fight.is_some() {
            self.publish(tick.output);
            return Ok(ScriptFlow::Continue);
        }
        if let Some(failure) = self.blocked.clone() {
            self.publish(tick.output);
            return Ok(ScriptFlow::Blocked(failure));
        }
        if self.hygiene_pending {
            let Some(tables) = self.tables.as_ref() else {
                self.publish(tick.output);
                return Ok(ScriptFlow::Continue);
            };
            match begin_clear_owned_prayers(tables.selected(), self.hygiene_owned, tick) {
                Hygiene::Clean => {
                    self.hygiene_owned = RaisedPrayers::empty();
                    self.hygiene_pending = false;
                }
                Hygiene::Started(handle) => {
                    self.fight = Some(Fight::ClearPrayers(handle));
                    self.publish(tick.output);
                    return Ok(ScriptFlow::Continue);
                }
                Hygiene::Deferred => {
                    self.publish(tick.output);
                    return Ok(ScriptFlow::Continue);
                }
                Hygiene::Failed(_) => {
                    let failure = ScriptFailure {
                        code: Arc::from("combat-failed"),
                        message: Arc::from("prayer hygiene failed"),
                    };
                    self.blocked = Some(failure.clone());
                    self.publish(tick.output);
                    return Ok(ScriptFlow::Blocked(failure));
                }
            }
        }
        if let Some(failure) = self.blocked.clone() {
            self.publish(tick.output);
            return Ok(ScriptFlow::Blocked(failure));
        }
        if let Some((id, request)) = self.pending.take() {
            if let Some(tables) = self.tables.as_ref() {
                match tick
                    .actions
                    .begin::<Combat>((Arc::clone(&request), Arc::clone(tables)), &mut tick.cx)
                {
                    Ok(handle) => {
                        self.fight = Some(Fight::Combat(handle));
                        self.combat_id = Some(id);
                        self.publish(tick.output);
                        return Ok(ScriptFlow::Continue);
                    }
                    Err(ActionError::Held | ActionError::Busy | ActionError::BudgetExhausted) => {
                        self.pending = Some((id, request));
                        self.publish(tick.output);
                        return Ok(ScriptFlow::Continue);
                    }
                    Err(_) => self.outcome = Some(Outcome::failed(id)),
                }
            } else {
                self.outcome = Some(Outcome::failed(id));
            }
        }
        self.tick_frame(tick, false);
        self.publish(tick.output);
        if let Some(failure) = self.blocked.clone() {
            return Ok(ScriptFlow::Blocked(failure));
        }
        Ok(ScriptFlow::Continue)
    }

    fn configure(&mut self, next: Arc<PreparedConfig>) -> Result<SettingsApply, ConfigError> {
        validate_config(&next)?;
        self.revision = next.revision();
        if let Some(prepared) = next.get::<Prepared>() {
            self.tables = Some(Arc::clone(&prepared.tables));
        }
        self.dirty = true;
        Ok(SettingsApply::Applied)
    }

    fn interrupt(&mut self, event: Interrupt) {
        if matches!(event, Interrupt::Pause) {
            if let Some(Fight::Combat(handle)) = self.fight.as_ref() {
                self.hygiene_owned = handle.prayer_cleanup();
            }
            if let Some(id) = self.combat_id.take() {
                self.outcome = Some(Outcome::cancelled(id));
            } else if let Some((id, _)) = self.pending.take() {
                self.outcome = Some(Outcome::cancelled(id));
            }
            self.fight = None;
            self.hygiene_pending = !self.hygiene_owned.is_empty();
        }
    }

    fn prayer_cleanup(&self) -> RaisedPrayers {
        match self.fight.as_ref() {
            Some(Fight::Combat(handle)) => handle.prayer_cleanup(),
            _ => self.hygiene_owned,
        }
    }
}

impl Sherlock {
    fn persist_death_state(&self, tick: &mut NativeTick<'_>) {
        let retained = tick.cx.retained().sherlock();
        retained.death_seq = self.death.watermark();
        retained.death_pending = self.death_pending;
    }

    fn poll_fight(&mut self, tick: &mut NativeTick<'_>) -> Poll<()> {
        if let Some(Fight::Combat(handle)) = self.fight.as_ref() {
            self.hygiene_owned = handle.prayer_cleanup();
        }
        match self.fight.as_ref() {
            Some(Fight::Combat(handle)) => match tick.actions.poll(handle, &mut tick.cx) {
                Poll::Pending => Poll::Pending,
                Poll::Ready(result) => {
                    let id = self.combat_id.take().unwrap_or(0);
                    self.fight = None;
                    if result.is_err() {
                        self.hygiene_pending = !self.hygiene_owned.is_empty();
                    } else {
                        self.hygiene_owned = RaisedPrayers::empty();
                        self.hygiene_pending = false;
                    }
                    if matches!(result, Err(ActionError::UserInput)) {
                        self.block_user_input = true;
                    }
                    self.outcome = Some(Outcome::from_poll(id, result));
                    Poll::Ready(())
                }
            },
            Some(Fight::ClearPrayers(handle)) => match tick.actions.poll(handle, &mut tick.cx) {
                Poll::Pending => Poll::Pending,
                Poll::Ready(Ok(report)) => {
                    self.fight = None;
                    if report.timed_out != 0 {
                        self.hygiene_pending = !self.hygiene_owned.is_empty();
                        self.blocked = Some(hygiene_failure(ActionError::Blocked(Arc::from(
                            "prayer cleanup timed out",
                        ))));
                    } else {
                        self.hygiene_owned = RaisedPrayers::empty();
                        self.hygiene_pending = false;
                    }
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
                Poll::Ready(Err(error)) => {
                    self.fight = None;
                    self.blocked = Some(hygiene_failure(error));
                    Poll::Ready(())
                }
            },
            None => Poll::Ready(()),
        }
    }

    fn tick_frame(&mut self, tick: &mut NativeTick<'_>, death_observed: bool) -> bool {
        let Some(mut sink) = tick.frame.compiled.interacts.take() else {
            return false;
        };
        let previous = (self.token, self.solved);
        self.pump(tick, &mut sink, death_observed);
        self.dirty |= previous != (self.token, self.solved);
        tick.frame.compiled.interacts = Some(sink);
        true
    }

    #[cfg(test)]
    fn tick_host(&mut self, ctx: &mut HostFrame<'_>, output: &mut dyn NativeOutput) {
        if hitpoints_zero(ctx.snapshot.map(|snapshot| snapshot.stats())) {
            return;
        }
        let Some(mut sink) = ctx.compiled.interacts.take() else {
            return;
        };
        let previous = (self.token, self.solved);
        self.pump_host(ctx, &mut sink, output);
        self.dirty |= previous != (self.token, self.solved);
        ctx.compiled.interacts = Some(sink);
    }

    fn publish(&mut self, output: &mut dyn NativeOutput) {
        let Some(run) = self.run else { return };
        if !std::mem::take(&mut self.dirty) {
            return;
        }
        let text = self
            .status
            .clone()
            .unwrap_or_else(|| Arc::from("Waiting for a clue"));
        output.status(ScriptStatus {
            run,
            card: CARD.id,
            phase: if self.token.is_some() {
                NativePhase::Working
            } else {
                NativePhase::Waiting
            },
            active_settings: self.revision,
            pending_settings: None,
            fields: Arc::from([
                StatusField {
                    key: "status",
                    label: "Status",
                    value: StatusValue::Text(text.clone()),
                },
                StatusField {
                    key: "solved",
                    label: "Clues solved",
                    value: StatusValue::Integer(i64::from(self.solved)),
                },
            ]),
            failure: None,
        });
        output.paint(Arc::new(ScriptPaint {
            title: Some(CARD.name.into()),
            lines: vec![text.to_string(), format!("Clues solved: {}", self.solved)],
            generation: run.run,
            ..Default::default()
        }));
    }
    /// One pump iteration: the `begin` call while there is no live session,
    /// then one `execute()` iteration over it.
    fn pump(
        &mut self,
        tick: &mut NativeTick<'_>,
        sink: &mut Vec<InteractReq>,
        death_observed: bool,
    ) {
        let selected = tick.frame.compiled.selected;
        let token = match self.token {
            Some(token) => token,
            None => {
                let begin = crate::clue::dispatch(selected, &begin_payload(&tick.frame));
                let Some(started) = answered_token(&begin) else {
                    return;
                };
                self.token = Some(started);
                started
            }
        };
        let combat = self.outcome.take().map(Outcome::page);
        let page = next_payload(&tick.frame, token, combat);
        #[cfg(test)]
        record_combat_page(page.get("combat"));
        self.iterate(tick, page, sink, death_observed);
        if self.block_user_input {
            self.block_user_input = false;
            self.blocked = Some(ScriptFailure {
                code: Arc::from("manual-movement"),
                message: Arc::from("manual movement"),
            });
        }
    }

    /// Host-frame pump for unit tests that do not own a native ledger.
    #[cfg(test)]
    fn pump_host(
        &mut self,
        ctx: &HostFrame<'_>,
        sink: &mut Vec<InteractReq>,
        output: &mut dyn NativeOutput,
    ) {
        let selected = ctx.compiled.selected;
        let token = match self.token {
            Some(token) => token,
            None => {
                let begin = crate::clue::dispatch(selected, &begin_payload(ctx));
                let Some(started) = answered_token(&begin) else {
                    return;
                };
                self.token = Some(started);
                started
            }
        };
        let combat = self.outcome.take().map(Outcome::page);
        let page = next_payload(ctx, token, combat);
        #[cfg(test)]
        record_combat_page(page.get("combat"));
        self.iterate_host(selected, page, sink, output);
    }

    fn iterate(
        &mut self,
        tick: &mut NativeTick<'_>,
        mut page: Value,
        sink: &mut Vec<InteractReq>,
        death_observed: bool,
    ) {
        let selected = tick.frame.compiled.selected;
        let mut resume = false;
        for _ in 0..CALLBACK_HANDOFFS {
            let Some(fields) = page.as_object_mut() else {
                return;
            };
            if resume {
                fields.insert("resume".into(), Value::Bool(true));
            } else {
                fields.remove("resume");
            }
            let answer = crate::clue::next_native(selected, &page, death_observed);
            resume = false;
            let kind = answer.get("kind").and_then(Value::as_str).unwrap_or("");
            match kind {
                "callback.enabled" => resume = true,
                "callback.log" => {
                    if let Some(message) = answer.get("message").and_then(Value::as_str) {
                        tick.output.log(api::hostlog::Level::Info, message);
                    }
                }
                "callback.setStatus" => {
                    if let Some(message) = answer.get("message").and_then(Value::as_str) {
                        if self.status.as_deref() != Some(message) {
                            self.status = Some(Arc::from(message));
                            self.dirty = true;
                        }
                    }
                }
                "wait" | "yield" | "supplies-needed" => return,
                "grind-ready" => {}
                "done" | "dead" | "abandon" | "guardian-lost" => {
                    self.end(kind);
                    return;
                }
                "aborted" => {
                    let reason = answer.get("reason").and_then(Value::as_str).unwrap_or("");
                    self.end(reason);
                    if reason.starts_with("combat-") {
                        self.blocked = Some(ScriptFailure {
                            code: Arc::from(reason),
                            message: Arc::from(reason),
                        });
                    }
                    return;
                }
                "combat" => {
                    self.begin_combat(tick, &answer);
                    return;
                }
                kind => {
                    enqueue(sink, kind, &answer);
                    return;
                }
            }
        }
    }

    #[cfg(test)]
    fn iterate_host(
        &mut self,
        selected: Option<&SelectedGameData>,
        mut page: Value,
        sink: &mut Vec<InteractReq>,
        output: &mut dyn NativeOutput,
    ) {
        let mut resume = false;
        for _ in 0..CALLBACK_HANDOFFS {
            let Some(fields) = page.as_object_mut() else {
                return;
            };
            if resume {
                fields.insert("resume".into(), Value::Bool(true));
            } else {
                fields.remove("resume");
            }
            let answer = crate::clue::dispatch(selected, &page);
            resume = false;
            let kind = answer.get("kind").and_then(Value::as_str).unwrap_or("");
            match kind {
                "callback.enabled" => resume = true,
                "callback.log" => {
                    if let Some(message) = answer.get("message").and_then(Value::as_str) {
                        output.log(api::hostlog::Level::Info, message);
                    }
                }
                "callback.setStatus" => {
                    if let Some(message) = answer.get("message").and_then(Value::as_str) {
                        if self.status.as_deref() != Some(message) {
                            self.status = Some(Arc::from(message));
                            self.dirty = true;
                        }
                    }
                }
                "wait" | "yield" | "supplies-needed" => return,
                "grind-ready" => {}
                "done" | "dead" | "abandon" | "guardian-lost" => {
                    self.end(kind);
                    return;
                }
                "aborted" => {
                    let reason = answer.get("reason").and_then(Value::as_str).unwrap_or("");
                    self.end(reason);
                    return;
                }
                "combat" => {
                    let Some(d) = Delegation::parse(&answer) else {
                        self.end("clue-combat-verb");
                        return;
                    };
                    self.outcome = Some(Outcome::failed(d.id));
                    return;
                }
                kind => {
                    enqueue(sink, kind, &answer);
                    return;
                }
            }
        }
    }

    fn begin_combat(&mut self, tick: &mut NativeTick<'_>, answer: &Value) {
        let Some(d) = Delegation::parse(answer) else {
            self.end("clue-combat-verb");
            return;
        };
        let Some(tables) = self.tables.as_ref() else {
            self.outcome = Some(Outcome::failed(d.id));
            return;
        };
        match tick
            .actions
            .begin::<Combat>((Arc::new(d.request()), Arc::clone(tables)), &mut tick.cx)
        {
            Ok(handle) => {
                self.fight = Some(Fight::Combat(handle));
                self.combat_id = Some(d.id);
                let name = tables
                    .npc(d.npc_type)
                    .and_then(|row| row.display.as_deref())
                    .unwrap_or("guardian");
                let status = Arc::<str>::from(format!("Fighting {name}"));
                if self.status.as_deref() != Some(status.as_ref()) {
                    self.status = Some(status);
                    self.dirty = true;
                }
            }
            Err(ActionError::Held | ActionError::Busy | ActionError::BudgetExhausted) => {
                self.pending = Some((d.id, Arc::new(d.request())));
            }
            Err(_) => self.outcome = Some(Outcome::failed(d.id)),
        }
    }

    /// Abort the current action authority after the chat latch confirms death.
    /// The clue machine receives the terminal signal separately so this cannot
    /// be mistaken for a combat result or a solved clue.
    fn cancel_for_death(&mut self, actions: &mut crate::native::NativeActions) {
        if let Some(fight) = self.fight.take() {
            match fight {
                Fight::Combat(handle) => actions.cancel(handle),
                Fight::ClearPrayers(handle) => actions.cancel(handle),
            }
        }
        // Death turns prayers off. A later activation belongs to the user,
        // not to the Combat owner that died.
        self.hygiene_owned = RaisedPrayers::empty();
        self.hygiene_pending = false;
        self.combat_id = None;
        self.pending = None;
        self.outcome = None;
        self.block_user_input = false;
    }
    /// End the live session and clear its token. The machine's own `done` —
    /// the abort a finished collect dies on — is the one end this card counts
    /// locally; a `none-held` abort, a terminal `dead`, `abandon` or
    /// `guardian-lost`, and the stale token a stop or a reset leaves just end
    /// it. Nothing is emitted either way: the machine owns completion, and its
    /// `clue solved` line is answered rather than posted on.
    fn end(&mut self, reason: &str) {
        if reason == DONE {
            self.solved = self.solved.saturating_add(1);
        }
        self.token = None;
        self.death_pending = false;
    }
}

#[cfg(test)]
thread_local! {
    static LAST_COMBAT_PAGE: std::cell::RefCell<Option<Value>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
fn record_combat_page(combat: Option<&Value>) {
    LAST_COMBAT_PAGE.with(|slot| *slot.borrow_mut() = combat.cloned());
}

#[cfg(test)]
fn take_combat_page() -> Option<Value> {
    LAST_COMBAT_PAGE.with(|slot| slot.borrow_mut().take())
}

/// The `begin` call's payload: the required page, and nothing else.
fn begin_payload(ctx: &HostFrame<'_>) -> Value {
    json!({
        "op": "begin",
        "generation": GENERATION,
        "held": held_page(ctx),
    })
}

/// The `next` page set is the adapter's: required keys always, optional
/// chat / bank / shop / overlay slots only when this frame carried them.
/// Native death authority is a separate Rust argument to `next_native`, not
/// a page field. Compatibility retains its posted-HP rule.
fn next_payload(ctx: &HostFrame<'_>, token: u64, combat: Option<Value>) -> Value {
    let mut page = Map::new();
    page.insert("op".into(), json!("next"));
    page.insert("token".into(), json!(token));
    page.insert("generation".into(), json!(GENERATION));
    page.insert("held".into(), held_page(ctx));
    page.insert("hold".into(), json!(ctx.compiled.hold));
    page.insert("combat_driver".into(), json!(true));
    if let Some(combat) = combat {
        page.insert("combat".into(), combat);
    }
    page.insert("locs".into(), loc_page(ctx));
    page.insert("ground".into(), ground_page(ctx));
    page.insert("inv".into(), inv_page(ctx));
    page.insert("npcs".into(), npc_page(ctx));
    page.insert("equipment".into(), equipment_page(ctx));
    if let Some((x, z, level)) = ctx.here {
        page.insert("here".into(), json!({ "x": x, "z": z, "level": level }));
    }
    let Some(snapshot) = ctx.snapshot else {
        return Value::Object(page);
    };
    if let Some(booth) = nearest_booth(snapshot) {
        page.insert("nearest_booth".into(), booth);
    }
    page.insert(
        "bank_open".into(),
        json!(snapshot.bank_component_id() != -1),
    );
    page.insert("main_modal_id".into(), json!(snapshot.modals().main));
    page.insert("chat_modal_id".into(), json!(snapshot.modals().chat));
    page.insert(
        "chat_continue".into(),
        json!(snapshot.chat_continue_component_id() != -1),
    );
    if let Some(options) = chat_options_page(snapshot) {
        page.insert("chat_options".into(), options);
    }
    page.insert(
        "count_dialog_open".into(),
        json!(snapshot.count_dialog_open()),
    );
    page.insert("inv_size".into(), json!(snapshot.inventory_size()));
    page.insert("self_slot".into(), json!(snapshot.self_slot()));
    if let Some(target) = snapshot
        .local_player()
        .and_then(|local| local.player.actor.target)
    {
        let (kind, index) = wire_target(Some(target));
        page.insert("self_target_kind".into(), json!(kind));
        page.insert("self_target_index".into(), json!(index));
    }
    if let Some(hitpoints) = snapshot
        .stats()
        .iter()
        .find(|stat| stat.name == "hitpoints")
    {
        page.insert("hitpoints".into(), json!(hitpoints.effective));
    }
    if let Some(overlay) = snapshot.varps().iter().find(|varp| varp.index == 95) {
        page.insert("varp95".into(), json!(overlay.value));
    }
    let board = snapshot.puzzle_board();
    page.insert(
        "puzzle_board".into(),
        json!({
            "component_id": board.component_id,
            "size": board.size,
            "items": board
                .items
                .iter()
                .map(|item| json!({ "slot": item.slot, "id": item.def.id }))
                .collect::<Vec<_>>(),
        }),
    );
    page.insert("puzzle_board_generation".into(), json!(board.generation));
    page.insert("shop_open".into(), json!(snapshot.shop().open));
    if snapshot.shop().open {
        page.insert("shop_stock".into(), shop_stock_page(snapshot));
    }
    Value::Object(page)
}

/// The posted pack page the identify reads: `(obj id, count)` pairs in slot
/// order, and nothing else. An absent frame is an empty page, never an error.
fn held_page(ctx: &HostFrame<'_>) -> Value {
    let rows: &[api::snapshot::ItemView] = ctx.snapshot.map_or(&[], GameSnapshot::inventory);
    Value::Array(
        rows.iter()
            .map(|item| json!([item.def.id, item.count]))
            .collect(),
    )
}

/// The posted scene page: every placed loc on the observed frame with its
/// native ops.
fn loc_page(ctx: &HostFrame<'_>) -> Value {
    let rows: &[api::snapshot::LocView] = ctx.snapshot.map_or(&[], GameSnapshot::locs);
    Value::Array(
        rows.iter()
            .map(|loc| {
                json!({
                    "id": loc.id,
                    "x": loc.tile.x,
                    "z": loc.tile.z,
                    "level": loc.tile.level,
                    "actions": action_strings(&loc.actions),
                })
            })
            .collect(),
    )
}

/// The posted ground page the collect arm Takes from: the scene shape plus the
/// display name the host resolves. A row whose id resolves to no name posts
/// `null`, exactly like the posted page it is marshalled from.
fn ground_page(ctx: &HostFrame<'_>) -> Value {
    let rows: &[api::snapshot::GroundItemView] =
        ctx.snapshot.map_or(&[], GameSnapshot::ground_items);
    let names = ctx.obj_names;
    Value::Array(
        rows.iter()
            .map(|item| {
                json!({
                    "id": item.def.id,
                    "x": item.tile.x,
                    "z": item.tile.z,
                    "level": item.tile.level,
                    "actions": action_strings(&item.actions),
                    "name": names.and_then(|names| names.name(item.def.id)),
                })
            })
            .collect(),
    )
}

/// The posted pack page the collect arm reads: the display name the Drop
/// resolves and the positive count that occupies a slot. This is not a second
/// inventory read — the identify page above is the same observed frame.
fn inv_page(ctx: &HostFrame<'_>) -> Value {
    let rows: &[api::snapshot::ItemView] = ctx.snapshot.map_or(&[], GameSnapshot::inventory);
    let names = ctx.obj_names;
    Value::Array(
        rows.iter()
            .map(|item| {
                json!({
                    "id": item.def.id,
                    "name": names
                        .and_then(|names| names.name(item.def.id))
                        .or(item.def.name.as_deref()),
                    "count": item.count,
                })
            })
            .collect(),
    )
}

/// The posted npc page the guarded encounter observes: the slot index the
/// Attack rides, the type id, the posted name and tile, the health pair the
/// kill is read through, and the target pair `targets_me` compares.
fn npc_page(ctx: &HostFrame<'_>) -> Value {
    let rows: &[api::snapshot::NpcView] = ctx.snapshot.map_or(&[], GameSnapshot::npcs);
    Value::Array(
        rows.iter()
            .map(|npc| {
                let (target_kind, target_index) = wire_target(npc.target);
                json!({
                    "index": npc.index as i32,
                    "id": npc.r#type.map_or(-1, |kind| kind as i32),
                    "name": npc.name,
                    "x": npc.tile.x,
                    "z": npc.tile.z,
                    "level": npc.tile.level,
                    "distance": npc.distance,
                    "health": npc.health,
                    "max_health": npc.total_health,
                    "in_combat": npc.in_combat,
                    "actions": action_strings(&npc.actions),
                    "target_kind": target_kind,
                    "target_index": target_index,
                })
            })
            .collect(),
    )
}

/// The posted worn page the Entrana strip reads: raw rows with their own
/// `slot`, omit-if-absent the way the adapter's `clueEquipmentPage` is. A
/// page that posted nothing is an empty list, never a second equipment read.
fn equipment_page(ctx: &HostFrame<'_>) -> Value {
    let rows: &[api::snapshot::ItemView] = ctx.snapshot.map_or(&[], GameSnapshot::equipment);
    let names = ctx.obj_names;
    Value::Array(
        rows.iter()
            .filter_map(|item| {
                let mut row = Map::new();
                if let Some(name) = names
                    .and_then(|names| names.name(item.def.id))
                    .or(item.def.name.as_deref())
                    .filter(|name| !name.is_empty())
                {
                    row.insert("name".into(), json!(name));
                }
                row.insert("id".into(), json!(item.def.id));
                row.insert("count".into(), json!(item.count));
                row.insert("slot".into(), json!(item.slot));
                (!row.is_empty()).then_some(Value::Object(row))
            })
            .collect(),
    )
}

/// The posted nearest Use-quickly booth, or `None` when the frame posted no
/// booth on the plane. Never an invented stand.
fn nearest_booth(snapshot: &GameSnapshot) -> Option<Value> {
    let loc = snapshot.nearest_use_quickly_booth()?;
    let mut row = Map::new();
    row.insert("x".into(), json!(loc.tile.x));
    row.insert("z".into(), json!(loc.tile.z));
    row.insert("level".into(), json!(loc.tile.level));
    row.insert("id".into(), json!(loc.id));
    if let Some(name) = loc.name.as_deref().filter(|name| !name.is_empty()) {
        row.insert("name".into(), json!(name));
    }
    if let Some(op) = loc
        .actions
        .iter()
        .flatten()
        .find(|action| action.eq_ignore_ascii_case("Use-quickly"))
    {
        row.insert("op".into(), json!(op));
    }
    Some(Value::Object(row))
}

/// The posted chat choices, or `None` when the frame posted no rows with
/// text. Option numbers keep the original 1-based slots so a dropped row
/// does not renumber the ones around it.
fn chat_options_page(snapshot: &GameSnapshot) -> Option<Value> {
    let rows: Vec<Value> = snapshot
        .chat_options()
        .iter()
        .enumerate()
        .filter(|(_, option)| !option.text.is_empty())
        .map(|(index, option)| {
            json!({
                "text": option.text,
                "option": index as i32 + 1,
            })
        })
        .collect();
    (!rows.is_empty()).then_some(Value::Array(rows))
}

/// The posted shop stock of an open shop. A row without a clickable id is
/// still posted as the observation it is.
fn shop_stock_page(snapshot: &GameSnapshot) -> Value {
    Value::Array(
        snapshot
            .shop()
            .stock
            .iter()
            .map(|item| {
                let mut row = Map::new();
                row.insert("id".into(), json!(item.def.id));
                if let Some(name) = item.def.name.as_deref().filter(|name| !name.is_empty()) {
                    row.insert("name".into(), json!(name));
                }
                row.insert("count".into(), json!(item.count));
                row.insert("slot".into(), json!(item.slot));
                row.insert("component".into(), json!(item.component_id));
                Value::Object(row)
            })
            .collect(),
    )
}

/// A native op list as the posted page carries it: the ops that are strings,
/// in slot order.
fn action_strings(actions: &[Option<String>]) -> Vec<&str> {
    actions
        .iter()
        .filter_map(|action| action.as_deref())
        .collect()
}

fn hygiene_failure(error: ActionError) -> ScriptFailure {
    let message = match error {
        ActionError::Failed(message)
        | ActionError::Blocked(message)
        | ActionError::Unavailable(message) => message,
        _ => Arc::from("prayer hygiene failed"),
    };
    ScriptFailure {
        code: Arc::from("combat-failed"),
        message,
    }
}

/// The posted target pair: `1` an npc, `2` a player, and the `(0, -1)` pair
/// neither the machine's `targets_me` nor a posted local npc target matches —
/// the same wire pair the posted scene rows carry.
fn wire_target(target: Option<ActorTargetView>) -> (i32, i32) {
    match target {
        Some(target) => (
            match target.kind {
                ActorKind::Npc => 1,
                ActorKind::Player => 2,
            },
            target.index as i32,
        ),
        None => (0, -1),
    }
}

/// The token a `begin` answer started, when it started one.
fn answered_token(answer: &Value) -> Option<u64> {
    (answer.get("kind").and_then(Value::as_str) == Some("token"))
        .then(|| answer.get("token").and_then(Value::as_u64))
        .flatten()
}

/// Map one machine verb onto this slot's interact queue: the same variants the
/// isolate forwards for the same verbs, one explicit arm per kind.
///
/// The walk is the ordinary [`InteractReq::Walk`], retaining its existing
/// boolean wire bits. This compiled card's false values are interpreted as
/// Inherit by the host admission; they do not veto captured script permissions.
/// It is never `WalkTo` (host navigation) or a driver call. The loc, npc and obj
/// verbs keep the identity the machine posted beside them, so the host matches
/// that row and refuses a stale one. An unknown kind is not a verb: nothing is
/// enqueued for it, and a step missing a field it needs is not a verb either.
fn enqueue(sink: &mut Vec<InteractReq>, _kind: &str, step: &Value) {
    if let Some(req) = crate::clue::verb_req(step) {
        sink.push(req);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::game_data::TrailMembershipRow;
    use client::client::{Client, ClientConfig, Skill};
    use client::config::if_type::{ButtonType, ComponentType, IfType, IfTypeMut};
    use client::config::ObjType;
    use client::dash3d::ClientObj;
    use client::datastruct::LinkList;
    use client::io::{ClientRevision, ServerProt};
    use serde::Deserialize;
    use std::sync::Arc;

    #[test]
    fn walk_permission_schema_and_legacy_settings_are_snake_case_and_false() {
        assert_eq!(CARD.schema_version, 1);
        let legacy_bag = SettingsBag::new();
        let legacy = SherlockSettings::deserialize(serde::de::value::MapDeserializer::new(
            legacy_bag.iter().map(|(key, value)| (key.as_str(), value)),
        ))
        .unwrap();
        assert!(!legacy._allow_teleports);
        assert!(!legacy._allow_wilderness);
        assert!(!legacy._allow_danger_zones);

        for id in ["allow_teleports", "allow_wilderness", "allow_danger_zones"] {
            let definition = settings_schema()
                .iter()
                .find(|setting| setting.id == id)
                .unwrap();
            assert_eq!(definition.ty, "boolean");
            assert_eq!(definition.default.as_deref(), Some("false"));
            assert_eq!(definition.group.as_deref(), Some("Walk permissions"));
        }

        let mut bag = SettingsBag::new();
        bag.insert("clueDuelPartner".into(), serde_json::json!("alice"));
        bag.insert("allow_teleports".into(), serde_json::json!(true));
        bag.insert("allow_wilderness".into(), serde_json::json!(false));
        bag.insert("allow_danger_zones".into(), serde_json::json!(true));
        let restored: SettingsBag =
            serde_json::from_value(serde_json::to_value(&bag).unwrap()).unwrap();
        assert_eq!(restored, bag);
        let settings = SherlockSettings::deserialize(serde::de::value::MapDeserializer::new(
            restored.iter().map(|(key, value)| (key.as_str(), value)),
        ))
        .unwrap();
        assert_eq!(settings._clue_duel_partner, "alice");
        assert!(settings._allow_teleports);
        assert!(!settings._allow_wilderness);
        assert!(settings._allow_danger_zones);
    }

    /// The selected-revision facts every session identifies against.
    fn selected() -> Arc<SelectedGameData> {
        api::game_data::for_revision(ClientRevision::R274).expect("selected data")
    }

    /// The first selected trail row the search arm walks to: the landed
    /// `trail_loc=^true` membership with its own decodable `trail_coord`.
    fn search_row(data: &SelectedGameData) -> (i32, (i32, i32, i32)) {
        let row = data
            .trails()
            .expect("trails")
            .rows
            .iter()
            .find(|row| {
                row.role == "clue"
                    && row
                        .params
                        .iter()
                        .any(|param| param.key == "trail_loc" && param.value == "^true")
                    && coord_of(row).is_some()
            })
            .expect("a selected search row");
        (row.id, coord_of(row).expect("coord"))
    }

    /// The tile one selected `trail_coord` packs, decoded here so the test
    /// reads the walk destination off the row and not off the port:
    /// `level_mapX_mapZ_localX_localZ`, 64 tiles per map square.
    fn coord_of(row: &TrailMembershipRow) -> Option<(i32, i32, i32)> {
        let token = row
            .params
            .iter()
            .find(|param| param.key == "trail_coord")?
            .value
            .clone();
        let parts: Vec<i32> = token
            .split('_')
            .filter_map(|part| part.parse().ok())
            .collect();
        let [level, map_x, map_z, local_x, local_z] = parts[..] else {
            return None;
        };
        Some((map_x * 64 + local_x, map_z * 64 + local_z, level))
    }

    /// Whether a selected family names `id`: a membership row, a challenge
    /// scroll or a talk step. The held ids a frame posts as its own pack are
    /// none of these.
    fn named_by_a_family(data: &SelectedGameData, id: i32) -> bool {
        let trails = data.trails().expect("trails");
        let talk = data.talk_key().expect("talk_key");
        trails.rows.iter().any(|row| row.id == id)
            || trails.challenge_answers.iter().any(|row| row.id == id)
            || talk.talk.iter().any(|row| row.id == id)
    }

    /// A membership row the selected family never names: a held id no identify
    /// pass reads, no challenge scroll answers and no talk step publishes. The
    /// challenge ids are no longer that exemplar — the machine's own seam joins
    /// a held scroll onto its parent talk step — so the id is searched for
    /// rather than taken from one of the families.
    fn unrelated(data: &SelectedGameData) -> i32 {
        (1..)
            .find(|id| !named_by_a_family(data, *id))
            .expect("a held id no selected family names")
    }

    /// The pack rows a collect frame posts: two held ids no selected family
    /// names, so the machine's own identify reads the frame as `none-held`
    /// while the live token's collect survives it.
    fn pack_rows(data: &SelectedGameData) -> [(i32, i32); 2] {
        let mut ids = (1..).filter(|id| !named_by_a_family(data, *id));
        let first = ids.next().expect("a held id no selected family names");
        let second = ids
            .next()
            .expect("a second held id no selected family names");
        [(first, 1), (second, 1)]
    }

    /// The first selected casket row this card's Open dispatch names: a
    /// `casket` membership whose own id joins a selected item with a display
    /// name and whose access is not the packed bound — the row the trail-end
    /// collect's Open goes out for.
    fn casket_row(data: &SelectedGameData) -> i32 {
        data.trails()
            .expect("trails")
            .rows
            .iter()
            .find(|row| {
                row.role == "casket"
                    && row.access.as_deref() != Some("constrained")
                    && data
                        .item_by_id(row.id)
                        .and_then(|item| item.name.as_deref())
                        .is_some_and(|name| !name.is_empty())
            })
            .expect("a selected casket row with a display name")
            .id
    }

    /// The first selected unguarded-dig membership: a `clue` row with a
    /// decodable `trail_coord`, no search `trail_loc`, no `trail_guardian` and
    /// no coordinate-trio `trail_sextant=yes`. The machine's own walk-to-Dig
    /// arm answers this row with one walk and no other verb.
    fn dig_row(data: &SelectedGameData) -> i32 {
        data.trails()
            .expect("trails")
            .rows
            .iter()
            .find(|row| {
                row.role == "clue"
                    && row.access.as_deref() != Some("constrained")
                    && coord_of(row).is_some()
                    && !row.params.iter().any(|param| {
                        param.key == "trail_loc"
                            || param.key == "trail_guardian"
                            || (param.key == "trail_sextant" && param.value == "yes")
                    })
            })
            .expect("a selected unguarded dig membership")
            .id
    }

    fn cfg() -> ClientConfig {
        ClientConfig {
            host: "127.0.0.1".into(),
            port: 43594,
            cache_dir: "/tmp".into(),
            members: true,
            lowmem: false,
        }
    }

    /// A client whose backpack holds exactly `held` `(obj id, count)` rows,
    /// with a named obj def per row so the posted page resolves names.
    fn client_with(held: &[(i32, i32)]) -> Client {
        let mut c = Client::new(cfg());
        if !held.is_empty() {
            let cache = Arc::get_mut(&mut c.cache).expect("sole cache owner");
            for (id, _) in held {
                while cache.objs.len() <= *id as usize {
                    cache.objs.push(ObjType::default());
                }
                cache.objs[*id as usize] = ObjType {
                    id: *id,
                    name: format!("obj {id}"),
                    ..Default::default()
                };
            }
        }
        c.side_icon[3] = 300;
        c.set_iface(
            300,
            IfType {
                id: 300,
                layer_id: 300,
                children: Some(vec![301]),
                ..Default::default()
            },
        );
        c.set_iface(
            301,
            IfType {
                id: 301,
                layer_id: 300,
                r#type: ComponentType::TYPE_INV,
                obj_ops: true,
                ..Default::default()
            },
        );
        c.set_iface_mut(
            301,
            IfTypeMut {
                link_obj_type: Some(held.iter().map(|(id, _)| id + 1).collect()),
                link_obj_number: Some(held.iter().map(|(_, count)| *count).collect()),
                ..Default::default()
            },
        );
        c.bump_gens(ServerProt::UPDATE_INV_FULL);
        c
    }

    fn snap(c: &mut Client) -> GameSnapshot {
        let mut s = GameSnapshot::new();
        s.rebuild(c);
        s
    }

    /// The client build base the ground frame plants its scene on, so the
    /// posted row's tile is the tile the frame posts as `here`.
    const SCENE_BASE: i32 = 3200;

    /// One observed frame: its snapshot, the posted tile, this frame's
    /// cooperative interrupt and the obj-id → name table its pages resolve
    /// against (`None` for the frames whose observed scene posts no name).
    struct Frame {
        snapshot: GameSnapshot,
        here: Option<(i32, i32, i32)>,
        hold: bool,
        names: Option<api::obj_names::ObjNames>,
    }

    impl Frame {
        fn new(held: &[(i32, i32)], here: Option<(i32, i32, i32)>) -> Self {
            let mut c = client_with(held);
            Self {
                snapshot: snap(&mut c),
                here,
                hold: false,
                names: None,
            }
        }

        fn ctx<'a>(
            &'a mut self,
            _driver: &'a mut dyn api::interact::Driver,
            selected: Option<&'a SelectedGameData>,
        ) -> HostFrame<'a> {
            HostFrame {
                here: self.here,
                snapshot: Some(&self.snapshot),
                obj_names: self.names.as_ref(),
                compiled: crate::CompiledTick {
                    selected,
                    reach: None,
                    bank_memory: None,
                    collision: None,
                    hold: self.hold,
                    world_members: api::selected::Truth::Unknown,
                    interacts: Some(Vec::new()),
                },
            }
        }
    }

    /// A frame whose observed scene posts one named, takeable ground stack of
    /// `stack` on `tile`, over the pack rows `held`: the collect's own loot
    /// row, on the tile the frame posts as `here`.
    ///
    /// The ground page resolves its display name through the shared obj table,
    /// so the stack's definition joins the client's cache and the table is
    /// built from the cache — a row the page carries never invents a name.
    fn loot_frame(held: &[(i32, i32)], stack: i32, tile: (i32, i32, i32)) -> Frame {
        let mut c = client_with(held);
        c.map_build_base_x = SCENE_BASE;
        c.map_build_base_z = SCENE_BASE;
        c.minusedlevel = tile.2;
        {
            let cache = Arc::get_mut(&mut c.cache).expect("sole cache owner");
            while cache.objs.len() <= stack as usize {
                cache.objs.push(ObjType::default());
            }
            cache.objs[stack as usize] = ObjType {
                id: stack,
                name: format!("obj {stack}"),
                ..Default::default()
            };
        }
        let mut list = LinkList::new();
        list.push(ClientObj::new(stack, 1));
        c.ground_obj[tile.2 as usize][(tile.0 - SCENE_BASE) as usize]
            [(tile.1 - SCENE_BASE) as usize] = Some(Box::new(list));
        // The scene gen a landed ground row moves, and nothing else: the rest
        // of this frame's pages are the same observed ones.
        c.bump_gens(ServerProt::OBJ_ADD);
        let names = api::obj_names::ObjNames::from_objs(&c.cache.objs);
        Frame {
            snapshot: snap(&mut c),
            here: Some(tile),
            hold: false,
            names: Some(names),
        }
    }

    /// A frame whose observed stats post hitpoints at `hp`, with every other
    /// skill healthy; native tick entry uses the effective value as a safety
    /// gate before it can emit work.
    fn wounded(held: &[(i32, i32)], hp: i32) -> Frame {
        let mut c = client_with(held);
        let slot = Skill::names
            .iter()
            .position(|name| *name == "hitpoints")
            .expect("the selected skill table names hitpoints");
        for level in c.stat_effective_level.iter_mut() {
            *level = 10;
        }
        c.stat_effective_level[slot] = hp;
        c.bump_gens(ServerProt::UPDATE_STAT);
        Frame {
            snapshot: snap(&mut c),
            here: Some(far()),
            hold: false,
            names: None,
        }
    }

    /// A frame whose chat modal posts a continue button and nothing else:
    /// the observed `chat_continue` the frozen `drainChat` reads, with no
    /// option list for the professor arm to steal.
    fn continue_dialog(held: &[(i32, i32)], here: Option<(i32, i32, i32)>) -> Frame {
        let mut c = client_with(held);
        c.set_iface(
            2000,
            IfType {
                id: 2000,
                layer_id: 2000,
                children: Some(vec![2003]),
                ..Default::default()
            },
        );
        c.set_iface(
            2003,
            IfType {
                id: 2003,
                layer_id: 2000,
                r#type: ComponentType::TYPE_TEXT,
                ..Default::default()
            },
        );
        c.set_iface_mut(
            2003,
            IfTypeMut {
                button_type: ButtonType::BUTTON_CONTINUE,
                ..Default::default()
            },
        );
        c.chat_modal_id = 2000;
        c.bump_gens(ServerProt::IF_OPENCHAT);
        Frame {
            snapshot: snap(&mut c),
            here,
            hold: false,
            names: None,
        }
    }

    /// The first selected talk membership whose jm2 spawn is unique, so the
    /// talk arm has a tile to walk to when no continue is posted.
    fn talk_row(data: &SelectedGameData) -> i32 {
        data.talk_key()
            .expect("talk_key")
            .talk
            .iter()
            .find(|row| row.spawn.is_some())
            .expect("a unique-spawn talk membership")
            .id
    }

    /// One tick over `frame`, and the requests it queued.
    fn tick(
        frame: &mut Frame,
        script: &mut Sherlock,
        selected: Option<&SelectedGameData>,
    ) -> Vec<InteractReq> {
        let mut driver = crate::ctx::test_support::NullDriver::default();
        let mut ctx = frame.ctx(&mut driver, selected);
        script.tick_host(&mut ctx, &mut crate::slot::compiled::TestOutput::default());
        ctx.compiled.interacts.take().unwrap_or_default()
    }

    /// A tile far enough from the decoded search tile that the machine walks.
    fn far() -> (i32, i32, i32) {
        (2000, 2000, 0)
    }

    /// The one walk the machine dispatches for that search step: the ordinary
    /// `Walk` with every option the machine itself leaves off.
    fn walk_to(tile: (i32, i32, i32)) -> InteractReq {
        InteractReq::Walk {
            x: tile.0,
            z: tile.1,
            level: tile.2,
            allow_teleports: false,
            allow_wilderness: false,
            allow_bank_fetch: false,
            request_id: 0,
            avoid: Vec::new(),
            cross: Vec::new(),
        }
    }

    fn page_keys(page: &Value) -> Vec<String> {
        let mut keys: Vec<String> = page
            .as_object()
            .expect("a page object")
            .keys()
            .cloned()
            .collect();
        keys.sort_unstable();
        keys
    }

    #[test]
    fn begin_page_pins_the_machine_keys() {
        let mut frame = Frame::new(&[], Some(far()));
        let mut driver = crate::ctx::test_support::NullDriver::default();
        let page = {
            let ctx = frame.ctx(&mut driver, None);
            begin_payload(&ctx)
        };
        assert_eq!(page_keys(&page), ["generation", "held", "op"]);
        assert_eq!(page["op"], "begin");
        assert_eq!(page["generation"], GENERATION);
        assert_eq!(page["held"], json!([]));
    }

    #[test]
    fn next_page_pins_the_machine_keys_over_the_observed_frame() {
        let data = selected();
        let (held_id, tile) = search_row(&data);
        let mut frame = Frame::new(&[(held_id, 1)], Some(far()));
        let mut driver = crate::ctx::test_support::NullDriver::default();
        let page = {
            let ctx = frame.ctx(&mut driver, Some(&data));
            next_payload(&ctx, 7, None)
        };
        assert_eq!(
            page_keys(&page),
            [
                "bank_open",
                "chat_continue",
                "chat_modal_id",
                "combat_driver",
                "count_dialog_open",
                "equipment",
                "generation",
                "ground",
                "held",
                "here",
                "hold",
                "inv",
                "inv_size",
                "locs",
                "main_modal_id",
                "npcs",
                "op",
                "puzzle_board",
                "puzzle_board_generation",
                "self_slot",
                "shop_open",
                "token",
            ],
        );
        assert_eq!(page["op"], "next");
        assert_eq!(page["token"], 7);
        assert_eq!(page["generation"], GENERATION);
        assert_eq!(page["hold"], false);
        assert_eq!(page["held"], json!([[held_id, 1]]));
        assert_eq!(page["here"], json!({ "x": 2000, "z": 2000, "level": 0 }));
        assert_eq!(
            page["main_modal_id"], -1,
            "the observed main modal, not an invented one"
        );
        assert_eq!(page["chat_modal_id"], -1, "the observed closed chat");
        assert_eq!(page["chat_continue"], false, "no continue on this frame");
        assert_eq!(page["count_dialog_open"], false);
        assert_eq!(page["bank_open"], false);
        assert_eq!(page["shop_open"], false);
        assert_eq!(page["equipment"], json!([]));
        assert_eq!(page["inv_size"], 1, "the inv tab's own slot count");
        assert_eq!(page["self_slot"], -1, "the client's own local slot");
        assert_eq!(page["locs"], json!([]));
        assert_eq!(page["npcs"], json!([]));
        assert_eq!(page["ground"], json!([]));
        assert_eq!(
            page["puzzle_board"],
            json!({ "component_id": -1, "size": 0, "items": [] }),
            "a closed board is what the observed frame carries"
        );
        assert!(
            page.get("hitpoints").is_none()
                && page.get("varp95").is_none()
                && page.get("self_target_kind").is_none()
                && page.get("nearest_booth").is_none()
                && page.get("chat_options").is_none()
                && page.get("shop_stock").is_none()
                && page.get("walk_missing_carry").is_none(),
            "a fact this frame did not carry is omitted, never defaulted: {page}"
        );
        assert_ne!(tile.0, 0, "the decoded search tile is a real destination");
    }

    #[test]
    fn next_page_omits_the_tile_and_the_pages_the_frame_lacks() {
        // No observed tile: `here` is not posted, and the required pages are
        // still the frame's own.
        let mut untiled = Frame::new(&[], None);
        let mut driver = crate::ctx::test_support::NullDriver::default();
        let untiled_page = {
            let ctx = untiled.ctx(&mut driver, None);
            next_payload(&ctx, 7, None)
        };
        assert!(untiled_page.get("here").is_none(), "{untiled_page}");
        assert_eq!(untiled_page["held"], json!([]));
        assert_eq!(untiled_page["hold"], false);

        // No snapshot at all: every page is empty and every optional slot is
        // omitted — never a closed `-1`, never the client's `28`, never
        // slot `0`.
        let mut unobserved = Frame::new(&[], None);
        let mut driver = crate::ctx::test_support::NullDriver::default();
        let mut ctx = unobserved.ctx(&mut driver, None);
        ctx.snapshot = None;
        let page = next_payload(&ctx, 1, None);
        assert_eq!(
            page_keys(&page),
            [
                "combat_driver",
                "equipment",
                "generation",
                "ground",
                "held",
                "hold",
                "inv",
                "locs",
                "npcs",
                "op",
                "token",
            ],
        );
        assert_eq!(page["held"], json!([]));
        assert_eq!(page["inv"], json!([]));
        assert_eq!(page["equipment"], json!([]));
        assert_eq!(page["locs"], json!([]));
        assert_eq!(page["npcs"], json!([]));
        assert_eq!(page["ground"], json!([]));
        assert!(page.get("inv_size").is_none(), "{page}");
        assert!(page.get("main_modal_id").is_none(), "{page}");
        assert!(page.get("chat_modal_id").is_none(), "{page}");
        assert!(page.get("chat_continue").is_none(), "{page}");
        assert!(page.get("self_slot").is_none(), "{page}");
        assert!(page.get("puzzle_board").is_none(), "{page}");
    }

    #[test]
    fn a_held_search_row_walks_with_every_walk_option_off() {
        let data = selected();
        let (held_id, tile) = search_row(&data);
        let mut frame = Frame::new(&[(held_id, 1)], Some(far()));
        let mut script = Sherlock::default();
        let sink = tick(&mut frame, &mut script, Some(&data));
        // Reaching the walk at all is the gate answered `true`: an unanswered
        // `callback.enabled` loops on the same question and never reaches a
        // verb, so this one enqueue is the whole handoff walked through.
        assert_eq!(
            sink,
            vec![walk_to(tile)],
            "the decoded search tile, on the ordinary Walk with every option off"
        );
        assert!(script.token.is_some(), "the begin started this session");
        assert_eq!(script.solved, 0);
    }

    #[test]
    fn nothing_held_is_an_idle_begin_the_next_frame_starts() {
        let data = selected();
        let mut script = Sherlock::default();

        // Nothing held: the begin is refused, so the card stays idle and
        // enqueues nothing — this tick and the next.
        let mut bare = Frame::new(&[], Some(far()));
        for _ in 0..2 {
            assert!(tick(&mut bare, &mut script, Some(&data)).is_empty());
        }
        assert!(script.token.is_none());
        assert_eq!(script.solved, 0, "a refused begin is not a finished clue");

        // Nothing held against a pin this build does not have: the same idle.
        let mut unpinned = Frame::new(&[], Some(far()));
        assert!(tick(&mut unpinned, &mut script, None).is_empty());
        assert!(script.token.is_none());

        // The same card, one held search row: this frame begins the session
        // the idle frames waited for, so they never wedged it.
        let (held_id, tile) = search_row(&data);
        let mut held = Frame::new(&[(held_id, 1)], Some(far()));
        assert_eq!(
            tick(&mut held, &mut script, Some(&data)),
            vec![walk_to(tile)]
        );
        assert!(script.token.is_some());
    }

    #[test]
    fn the_identify_is_the_machines_own_membership_read() {
        let data = selected();
        let mut script = Sherlock::default();

        // A held id the selected family never names is not a step: this card
        // identifies nothing of its own and stays idle.
        let mut unrelated_frame = Frame::new(&[(unrelated(&data), 1)], Some(far()));
        assert!(tick(&mut unrelated_frame, &mut script, Some(&data)).is_empty());
        assert!(script.token.is_none());

        // A held row with no positive count is not held either.
        let (held_id, _) = search_row(&data);
        let mut zero = Frame::new(&[(held_id, 0)], Some(far()));
        assert!(tick(&mut zero, &mut script, Some(&data)).is_empty());
        assert!(script.token.is_none());
    }

    #[test]
    fn a_none_held_abort_clears_the_token_without_counting() {
        let data = selected();
        let (held_id, tile) = search_row(&data);
        let mut script = Sherlock::default();

        let mut held = Frame::new(&[(held_id, 1)], Some(far()));
        assert_eq!(
            tick(&mut held, &mut script, Some(&data)),
            vec![walk_to(tile)]
        );
        let token = script.token.expect("a live session");

        // The clue is gone from the pack: the machine's own `none-held` abort.
        // The session ends and the token is cleared, but the collect's `done`
        // is the one end that counts, so this landed abort advances nothing —
        // and nothing is emitted for it: no verb, no completion line.
        let mut dropped = Frame::new(&[], Some(far()));
        assert!(tick(&mut dropped, &mut script, Some(&data)).is_empty());
        assert!(script.token.is_none(), "the session ended");
        assert_eq!(script.solved, 0, "a landed abort is not a finished clue");

        // A fresh frame starts a fresh session, not the old token.
        let mut again = Frame::new(&[(held_id, 1)], Some(far()));
        assert_eq!(
            tick(&mut again, &mut script, Some(&data)),
            vec![walk_to(tile)]
        );
        assert_ne!(script.token, Some(token));

        // A token the machine no longer has is the same end, without a count.
        script.token = Some(0);
        assert!(tick(&mut again, &mut script, Some(&data)).is_empty());
        assert!(script.token.is_none(), "a stale token is not a session");
        assert_eq!(script.solved, 0, "only the collect's own done counts");
    }

    #[test]
    fn the_finished_collect_counts_and_clears_the_token_in_one_iterate() {
        let data = selected();
        let casket = casket_row(&data);
        let mut script = Sherlock::default();

        // The held casket's Open goes out on the first tick: the gate, the
        // progress line, the status line, then the one verb that is also the
        // collect's entry permit.
        let mut held = Frame::new(&[(casket, 1)], Some(far()));
        let opened = tick(&mut held, &mut script, Some(&data));
        assert!(
            matches!(opened.as_slice(), [InteractReq::Held { action, .. }] if action == "Open"),
            "the casket's Open is the one verb: {opened:?}"
        );
        assert!(script.token.is_some(), "the casket opened a live session");

        // The casket has left the pack and the pack is full with nothing
        // droppable: one iterate runs the machine's whole exit — the pack-full
        // warning, the `clue solved` status, the live-token `grind-ready`
        // continue, and the `done` the session dies on — which is exactly the
        // four handoffs this card allows. The `grind-ready` is a continue and
        // not an end, so the `done` behind it lands in this same tick.
        let tile = (SCENE_BASE + 10, SCENE_BASE + 12, 0);
        let mut loot = loot_frame(&pack_rows(&data), 900, tile);
        let sank = tick(&mut loot, &mut script, Some(&data));
        assert!(
            sank.is_empty(),
            "the completion is answered, never forwarded as a verb: {sank:?}"
        );
        assert_eq!(script.solved, 1, "the collect's own done is the one count");
        assert!(script.token.is_none(), "done clears the token");
    }

    #[test]
    fn a_posted_zero_hitpoint_page_freezes_without_ending_the_session() {
        let data = selected();
        let (held_id, tile) = search_row(&data);
        let mut script = Sherlock::default();

        let mut held = Frame::new(&[(held_id, 1)], Some(far()));
        assert_eq!(
            tick(&mut held, &mut script, Some(&data)),
            vec![walk_to(tile)]
        );
        let token = script.token.expect("a live clue session");

        // Zero effective hitpoints without a death chat signal freezes the
        // token: no work is enqueued, no end is invented, and nothing solves.
        let mut downed = wounded(&[(held_id, 1)], 0);
        assert!(tick(&mut downed, &mut script, Some(&data)).is_empty());
        assert_eq!(script.token, Some(token), "HP alone does not end the clue");
        assert_eq!(script.solved, 0, "HP alone does not solve the clue");

        // The same token resumes clue handling when HP is observed again.
        let mut recovered = Frame::new(&[(held_id, 1)], Some(far()));
        let _ = tick(&mut recovered, &mut script, Some(&data));
        assert_eq!(script.token, Some(token), "recovery keeps the live token");
    }

    #[test]
    fn a_supplies_needed_tick_keeps_the_token_and_fetches_nothing() {
        let data = selected();
        let dig = dig_row(&data);
        let mut script = Sherlock::default();

        // The dig arm walks to its own decoded tile: the walk is the machine's
        // answer and its tile is where the arrival is read next tick.
        let mut walking = Frame::new(&[(dig, 1)], Some(far()));
        let walked = tick(&mut walking, &mut script, Some(&data));
        let tile = match walked.as_slice() {
            [InteractReq::Walk {
                x,
                z,
                level,
                allow_teleports: false,
                allow_wilderness: false,
                allow_bank_fetch: false,
                request_id: 0,
                avoid: _,
                cross: _,
            }] => (*x, *z, *level),
            other => panic!("the dig arm answers with one optionless walk: {other:?}"),
        };
        let token = script.token.expect("a live session");

        // Arrived without the Spade on the posted pack: the named
        // `supplies-needed` wait-class. The tick ends there — nothing is
        // fetched, nothing is enqueued — and the token lives for the pack that
        // posts the tool.
        let mut landed = Frame::new(&[(dig, 1)], Some(tile));
        assert!(tick(&mut landed, &mut script, Some(&data)).is_empty());
        assert_eq!(script.token, Some(token), "the wait-class keeps the token");
        assert_eq!(script.solved, 0);
    }

    #[test]
    fn the_page_marshals_the_observed_pack_and_scene() {
        let data = selected();
        let (held_id, _) = search_row(&data);
        let mut frame = Frame::new(&[(held_id, 1)], Some(far()));
        let mut driver = crate::ctx::test_support::NullDriver::default();
        let ctx = frame.ctx(&mut driver, Some(&data));
        let page = next_payload(&ctx, 3, None);
        let inv = page["inv"].as_array().expect("inv page");
        assert_eq!(inv.len(), 1, "{page}");
        assert_eq!(inv[0]["id"], held_id);
        assert_eq!(inv[0]["count"], 1);
        assert_eq!(inv[0]["name"], format!("obj {held_id}"));
        assert_eq!(
            page["held"],
            json!([[held_id, 1]]),
            "the identify page is the same observed pack"
        );
        assert!(
            page["locs"].as_array().expect("locs").is_empty(),
            "an unobserved scene is an empty page, never an invented row"
        );
    }

    #[test]
    fn the_page_carries_the_frames_cooperative_interrupt() {
        let mut frame = Frame::new(&[], Some(far()));
        let mut driver = crate::ctx::test_support::NullDriver::default();
        let inert = {
            let ctx = frame.ctx(&mut driver, None);
            next_payload(&ctx, 1, None)
        };
        assert_eq!(inert["hold"], false);

        // The frame's own `hold || ours`: the same pair the isolate's
        // `EventSignal.pending()` reads, so the machine yields instead of
        // driving the step on a frame the guardian froze.
        frame.hold = true;
        let held = {
            let ctx = frame.ctx(&mut driver, None);
            next_payload(&ctx, 1, None)
        };
        assert_eq!(held["hold"], true);
    }

    #[test]
    fn every_machine_verb_maps_onto_one_interact_request() {
        let mut sink = Vec::new();
        enqueue(
            &mut sink,
            "held",
            &json!({ "kind": "held", "name": "Spade", "action": "Dig" }),
        );
        enqueue(
            &mut sink,
            "loc",
            &json!({
                "kind": "loc",
                "x": 1,
                "z": 2,
                "level": 0,
                "action": "Search",
                "id": 2646,
            }),
        );
        enqueue(
            &mut sink,
            "npc",
            &json!({
                "kind": "npc",
                "name": "Zamorak wizard",
                "action": "Attack",
                "index": 4,
            }),
        );
        enqueue(
            &mut sink,
            "if-button",
            &json!({ "kind": "if-button", "component_id": 77 }),
        );
        enqueue(&mut sink, "close-modal", &json!({ "kind": "close-modal" }));
        enqueue(
            &mut sink,
            "obj",
            &json!({
                "kind": "obj",
                "x": 9,
                "z": 8,
                "level": 1,
                "name": "Reward casket (easy)",
                "action": "Take",
            }),
        );
        enqueue(
            &mut sink,
            "puzzle-move",
            &json!({
                "kind": "puzzle-move",
                "id": 1,
                "slot": 2,
                "component": 6600,
                "generation": 7,
            }),
        );
        enqueue(&mut sink, "continue", &json!({ "kind": "continue" }));
        enqueue(
            &mut sink,
            "answer",
            &json!({ "kind": "answer", "option": 2 }),
        );
        enqueue(
            &mut sink,
            "answer-count",
            &json!({ "kind": "answer-count", "value": 6859 }),
        );
        enqueue(
            &mut sink,
            "shop-button",
            &json!({
                "kind": "shop-button",
                "shop": "Buy",
                "name": "Shantay pass",
                "id": 1854,
                "slot": 3,
                "component": 3900,
                "chunk": 1,
            }),
        );
        enqueue(
            &mut sink,
            "wear",
            &json!({ "kind": "wear", "name": "Bronze platebody" }),
        );
        enqueue(
            &mut sink,
            "unequip",
            &json!({ "kind": "unequip", "name": "Bronze platebody" }),
        );
        enqueue(
            &mut sink,
            "deposit",
            &json!({ "kind": "deposit", "name": "Bronze platebody" }),
        );
        enqueue(
            &mut sink,
            "withdraw",
            &json!({
                "kind": "withdraw",
                "name": "Bronze platebody",
                "action": "Withdraw-1",
            }),
        );
        enqueue(
            &mut sink,
            "walk-nearest-bank",
            &json!({ "kind": "walk-nearest-bank" }),
        );
        enqueue(
            &mut sink,
            "open-booth",
            &json!({
                "kind": "open-booth",
                "x": 3091,
                "z": 3245,
                "level": 0,
                "id": 2213,
                "name": "Bank booth",
                "action": "Use-quickly",
            }),
        );
        enqueue(&mut sink, "close", &json!({ "kind": "close" }));
        assert_eq!(
            sink,
            vec![
                InteractReq::Held {
                    name: "Spade".into(),
                    action: "Dig".into(),
                    slot: None,
                    target_item_id: None,
                },
                InteractReq::Loc {
                    x: 1,
                    z: 2,
                    level: 0,
                    action: "Search".into(),
                    id: Some(2646),
                },
                InteractReq::Npc {
                    name: "Zamorak wizard".into(),
                    action: "Attack".into(),
                    index: Some(4),
                },
                InteractReq::IfButton { component_id: 77 },
                InteractReq::CloseModal,
                InteractReq::Obj {
                    x: 9,
                    z: 8,
                    level: 1,
                    name: Some("Reward casket (easy)".into()),
                    action: "Take".into(),
                },
                InteractReq::PuzzleMove {
                    id: 1,
                    slot: 2,
                    component: 6600,
                    generation: 7,
                },
                InteractReq::ContinueDialog { component_id: None },
                InteractReq::Answer { option: 2 },
                InteractReq::AnswerCount { value: 6859 },
                InteractReq::ShopButton {
                    kind: "Buy".into(),
                    name: "Shantay pass".into(),
                    id: 1854,
                    slot: 3,
                    component: 3900,
                    chunk: 1,
                },
                InteractReq::Wear {
                    name: "Bronze platebody".into(),
                },
                InteractReq::Unequip {
                    name: "Bronze platebody".into(),
                },
                InteractReq::Deposit {
                    name: "Bronze platebody".into(),
                },
                InteractReq::Withdraw {
                    name: "Bronze platebody".into(),
                    action: "Withdraw-1".into(),
                },
                InteractReq::WalkNearestBank,
                InteractReq::OpenBooth {
                    x: 3091,
                    z: 3245,
                    level: 0,
                    id: 2213,
                    name: Some("Bank booth".into()),
                    action: Some("Use-quickly".into()),
                },
                InteractReq::Close,
            ]
        );

        // A kind that is not a verb — the machine's own callback and wait
        // kinds, its terminal kinds and its completion line — and a verb
        // missing a field it needs: none of them is enqueued as anything.
        let mut unknown = Vec::new();
        for kind in [
            "callback.enabled",
            "callback.log",
            "callback.setStatus",
            "wait",
            "yield",
            "grind-ready",
            "supplies-needed",
            "done",
            "dead",
            "abandon",
            "guardian-lost",
            "aborted",
            "clue solved",
        ] {
            enqueue(
                &mut unknown,
                kind,
                &json!({ "kind": kind, "message": kind }),
            );
        }
        enqueue(&mut unknown, "walk", &json!({ "kind": "walk", "x": 1 }));
        assert!(
            unknown.is_empty(),
            "a kind that is not a verb, and a verb with no fields, enqueue nothing"
        );
    }

    #[test]
    fn a_tick_without_a_sink_dispatches_nothing() {
        let data = selected();
        let (held_id, _) = search_row(&data);
        let mut frame = Frame::new(&[(held_id, 1)], Some(far()));
        let mut script = Sherlock::default();
        let mut driver = crate::ctx::test_support::NullDriver::default();
        {
            let mut ctx = frame.ctx(&mut driver, Some(&data));
            ctx.compiled.interacts = None;
            script.tick_host(&mut ctx, &mut crate::slot::compiled::TestOutput::default());
        }
        assert!(
            script.token.is_none(),
            "a card with nowhere to send a verb does not start a session"
        );
    }

    #[test]
    fn a_frozen_frame_yields_instead_of_walking() {
        let data = selected();
        let (held_id, _) = search_row(&data);
        let mut frame = Frame::new(&[(held_id, 1)], Some(far()));
        frame.hold = true;
        let mut script = Sherlock::default();
        // The guardian's own freeze: the machine yields the step rather than
        // walking through the held frame. Nothing is enqueued, and the token
        // it began with stays live for the thaw.
        let sink = tick(&mut frame, &mut script, Some(&data));
        assert!(sink.is_empty(), "{sink:?}");
        assert!(
            script.token.is_some(),
            "the frozen frame is not a session end"
        );
    }

    /// A posted continue-dialog produces a continue step, not another Talk-to
    /// and not a wait: Sherlock marshals `chat_continue`, the machine drains
    /// it at `Steady`, and enqueue maps it onto `ContinueDialog`.
    #[test]
    fn a_posted_continue_dialog_produces_a_continue_step() {
        let data = selected();
        let held_id = talk_row(&data);
        let mut script = Sherlock::default();

        let mut walking = Frame::new(&[(held_id, 1)], Some(far()));
        let first = tick(&mut walking, &mut script, Some(&data));
        assert!(
            matches!(first.as_slice(), [InteractReq::Walk { .. }]),
            "the talk arm walks when no chat is posted: {first:?}"
        );
        assert!(script.token.is_some());

        let mut chatting = continue_dialog(&[(held_id, 1)], Some(far()));
        let page = {
            let mut driver = crate::ctx::test_support::NullDriver::default();
            let ctx = chatting.ctx(&mut driver, Some(&data));
            next_payload(&ctx, script.token.expect("live token"), None)
        };
        assert_eq!(page["chat_continue"], true, "{page}");
        assert_eq!(page["chat_modal_id"], 2000, "{page}");
        assert!(page.get("chat_options").is_none(), "{page}");

        let sink = tick(&mut chatting, &mut script, Some(&data));
        assert_eq!(
            sink,
            vec![InteractReq::ContinueDialog { component_id: None }],
            "a posted continue is not a Talk-to and not a wait"
        );
        assert!(script.token.is_some(), "continue keeps the session");
    }

    /// Unknown kinds are not locs. Shared `verb_req` is the isolate family
    /// and this card's enqueue; the JS ENQUEUED_KINDS table is gone.
    #[test]
    fn unknown_kind_is_not_enqueued_as_a_loc() {
        assert!(crate::clue::verb_req(&serde_json::json!({
            "kind": "nope",
            "x": 1,
            "z": 2,
            "level": 0,
            "action": "Search",
            "id": 1
        }))
        .is_none());
        assert!(matches!(
            crate::clue::verb_req(&serde_json::json!({ "kind": "continue" })),
            Some(InteractReq::ContinueDialog { component_id: None })
        ));
        assert!(matches!(
            crate::clue::verb_req(
                &serde_json::json!({ "kind": "walk", "x": 1, "z": 2, "level": 0 })
            ),
            Some(InteractReq::Walk { .. })
        ));
    }

    #[test]
    fn sherlock_fits_in_one_kib_with_one_live_machine() {
        eprintln!(
            "Fight={} Combat={} ClearPrayers={} Sherlock={}",
            std::mem::size_of::<super::Fight>(),
            std::mem::size_of::<crate::combat::Combat>(),
            std::mem::size_of::<crate::combat::ClearPrayers>(),
            std::mem::size_of::<Sherlock>(),
        );
        assert!(
            std::mem::size_of::<Sherlock>() <= 1024,
            "Sherlock={} Fight={}",
            std::mem::size_of::<Sherlock>(),
            std::mem::size_of::<super::Fight>(),
        );
    }
}

#[cfg(test)]
#[path = "sherlock_native_tests.rs"]
mod native_tests;
