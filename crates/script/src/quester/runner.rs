//! §3.2 tick loop with colour-first, boundary-triggered journal evidence.
use super::bank_memo::BankMemo;
use super::compile::{
    CompiledPath, CompiledStep, PredicateContext, StepContext, StepOutcome, StepRun,
};
use super::death::DeathLatch;
use super::families::combat::CombatReceipt;
use super::progress::{quest_colour, resolve_colour, resolve_journal};
use super::provision::Provisioner;
use super::select::{select, sequence_for_stage, SelectionDecision};
use super::watchdog::{Watchdog, WatchdogAction};
use crate::combat::ClearPrayers;
use crate::native::{
    ActionError, ActionHandle, Interrupt, NativeOutput, NativePhase, NativeTick, Script,
    ScriptFailure, ScriptFlow, ScriptStatus, StatusField, StatusValue, StopReason,
};
use crate::quest_journal::{JournalMachine, JournalRequest};
use crate::CompiledId;
use api::game_data::SelectedGameData;
use api::quest_facts::QuestCatalog;
use api::quest_progress::{JournalRead, QuestProgress};
use api::selected::{FactKey, Knowledge, RunKey, Truth};
use api::snapshot::QuestListStatus;
use api::{DetectedRandom, RandomClaim};
use std::num::NonZeroU32;
use std::sync::Arc;
use std::task::Poll;

use std::time::Duration;

// A read has at most three transactions (each can emit at most one row
// click). Adoption consumes a transaction too, but never adds a click.
const JOURNAL_READ_ATTEMPTS: u8 = 3;
const JOURNAL_RETRY_QUIET_TICKS: u64 = 3;
const JOURNAL_RETRY_LIMIT_BUSY: &str =
    "journal read retry limit reached (journal remained busy during read)";
const JOURNAL_RETRY_LIMIT_OWNERSHIP_LOST: &str =
    "journal read retry limit reached (journal ownership repeatedly lost)";
pub struct Quester {
    run: RunKey,
    path: Arc<CompiledPath>,
    selected: Arc<SelectedGameData>,
    quests: Arc<QuestCatalog>,
    stage: Option<FactKey>,
    progress: Option<Arc<QuestProgress>>,
    journal: Option<ActionHandle<JournalMachine>>,
    bank: BankMemo,
    last_read: Option<Arc<JournalRead>>,
    journal_text: Option<Arc<str>>,
    read_requested: bool,
    selection_since: Option<Duration>,
    seq_index: usize,
    step_index: usize,
    step: Option<Box<dyn StepRun>>,
    clear_prayers: Option<ActionHandle<ClearPrayers>>,
    prayer_cleanup_pending: bool,
    last_outcome: Option<StepOutcome>,
    advances: bool,
    in_prelude: bool,
    settling: bool,
    settle_deadline: Duration,
    chat_since: i32,
    needs_read: bool,
    unreadable_since: Option<Duration>,
    unreadable_reads: u8,
    journal_attempts: u8,
    journal_retry_pending: bool,
    journal_quiet_since: Option<NonZeroU32>,
    empty_reads: u8,
    park_reason: &'static str,
    last_error: Option<Arc<str>>,
    waiting: Option<(&'static str, Arc<str>)>,
    deaths: u8,
    attempts: u8,
    fail_streak: u8,
    parked: bool,
    journal_opened: bool,
    dirty: bool,
    watchdog: Watchdog,
    death: DeathLatch,
    provisioner: Provisioner,
}
fn append_combat_status(fields: &mut Vec<StatusField>, outcome: Option<&StepOutcome>) {
    let Some(receipt) = outcome
        .and_then(|outcome| outcome.receipt.as_ref())
        .and_then(|receipt| receipt.as_any().downcast_ref::<CombatReceipt>())
    else {
        return;
    };
    let report = receipt.report;
    fields.push(StatusField {
        key: "combat_end",
        label: "Combat end",
        value: StatusValue::Text(Arc::from(format!("{:?}", report.end))),
    });
    fields.push(StatusField {
        key: "combat_target_gone_restarts",
        label: "Combat target-gone restarts",
        value: StatusValue::Integer(i64::from(receipt.target_gone_restarts)),
    });
    if let Some(mode) = report.melee_mode_fallback {
        static MODES: std::sync::LazyLock<[Arc<str>; 4]> = std::sync::LazyLock::new(|| [
            Arc::from("accurate"), Arc::from("aggressive"), Arc::from("defensive"), Arc::from("controlled"),
        ]);
        fields.push(StatusField {
            key: "combat_melee_mode_fallback",
            label: "Combat melee-mode fallback",
            value: StatusValue::Text(Arc::clone(&MODES[mode as usize])),
        });
    }
    for (key, label, value) in [
        (
            "combat_evidence_run_slot",
            "Combat evidence run slot",
            report.evidence.run.slot,
        ),
        (
            "combat_evidence_run",
            "Combat evidence run",
            report.evidence.run.run,
        ),
        (
            "combat_evidence_session",
            "Combat evidence session",
            report.evidence.run.session,
        ),
        (
            "combat_evidence_tick",
            "Combat evidence tick",
            report.evidence.tick,
        ),
        (
            "combat_evidence_sequence",
            "Combat evidence sequence",
            report.evidence.sequence,
        ),
    ] {
        fields.push(StatusField {
            key,
            label,
            value: StatusValue::Integer(i64::try_from(value).unwrap_or(i64::MAX)),
        });
    }
    fields.push(StatusField {
        key: "combat_engaged",
        label: "Combat actor engaged",
        value: StatusValue::Truth(if report.engaged.is_some() {
            Truth::True
        } else {
            Truth::False
        }),
    });
    fields.push(StatusField {
        key: "combat_engaged_npc_type",
        label: "Combat NPC type",
        value: StatusValue::Integer(i64::from(report.engaged_npc_type)),
    });
    if let Some(actor) = report.engaged {
        fields.push(StatusField {
            key: "combat_engaged_kind",
            label: "Combat actor kind",
            value: StatusValue::Text(Arc::from(format!("{:?}", actor.kind))),
        });
        fields.push(StatusField {
            key: "combat_engaged_index",
            label: "Combat actor index",
            value: StatusValue::Integer(i64::from(actor.index)),
        });
    }
    for (key, label, value) in [
        ("combat_ticks", "Combat ticks", i64::from(report.ticks)),
        ("combat_swings", "Combat swings", i64::from(report.swings)),
        ("combat_casts", "Combat casts", i64::from(report.casts)),
        (
            "combat_damage_taken",
            "Combat damage taken",
            i64::from(report.damage_taken),
        ),
        ("combat_food", "Combat food", i64::from(report.food)),
        (
            "combat_prayer_doses",
            "Combat prayer doses",
            i64::from(report.prayer_doses),
        ),
        (
            "combat_boost_doses",
            "Combat boost doses",
            i64::from(report.boost_doses),
        ),
        (
            "combat_antifire_doses",
            "Combat antifire doses",
            i64::from(report.antifire_doses),
        ),
        (
            "combat_hits_while_protected",
            "Hits while protected",
            i64::from(report.hits_while_protected),
        ),
        (
            "combat_protect_switches",
            "Protect switches",
            i64::from(report.protect_switches),
        ),
        (
            "combat_intruders",
            "Combat intruders",
            i64::from(report.intruders),
        ),
        (
            "combat_ammo_pickups",
            "Combat ammo pickups",
            i64::from(report.ammo_pickups),
        ),
        (
            "combat_restorations",
            "Combat restorations",
            i64::from(report.restorations),
        ),
        (
            "combat_locked_ticks",
            "Combat locked ticks",
            i64::from(report.locked_ticks),
        ),
        (
            "combat_multi_op_plans",
            "Combat multi-operation plans",
            i64::from(report.multi_op_plans),
        ),
        (
            "combat_flick_resets",
            "Combat flick resets",
            i64::from(report.flick_resets),
        ),
        (
            "combat_flick_misses",
            "Combat flick misses",
            i64::from(report.flick_misses),
        ),
    ] {
        fields.push(StatusField {
            key,
            label,
            value: StatusValue::Integer(value),
        });
    }
    fields.push(StatusField {
        key: "combat_flick_fallback",
        label: "Combat flick fallback",
        value: StatusValue::Truth(if report.flick_fallback {
            Truth::True
        } else {
            Truth::False
        }),
    });
}

impl Quester {
    pub fn new(
        run: RunKey,
        path: Arc<CompiledPath>,
        selected: Arc<SelectedGameData>,
        quests: Arc<QuestCatalog>,
    ) -> Self {
        Self {
            run,
            path,
            selected,
            quests,
            stage: None,
            progress: None,
            journal: None,
            bank: BankMemo::default(),
            last_read: None,
            journal_text: None,
            read_requested: false,
            selection_since: None,
            seq_index: 0,
            step_index: 0,
            step: None,
            clear_prayers: None,
            prayer_cleanup_pending: true,
            last_outcome: None,
            advances: false,
            in_prelude: false,
            settling: false,
            settle_deadline: Duration::ZERO,
            chat_since: 0,
            needs_read: true,
            unreadable_since: None,
            unreadable_reads: 0,
            journal_attempts: 0,
            journal_retry_pending: false,
            journal_quiet_since: None,
            empty_reads: 0,
            park_reason: "no progress",
            last_error: None,
            waiting: None,
            deaths: 0,
            attempts: 0,
            fail_streak: 0,
            parked: false,
            journal_opened: false,
            dirty: true,
            watchdog: Watchdog::default(),
            death: DeathLatch::default(),
            provisioner: Provisioner::None,
        }
    }

    pub fn journal_opened(&self) -> bool {
        self.journal_opened
    }

    pub fn deaths(&self) -> u8 {
        self.deaths
    }

    pub fn stage(&self) -> Option<&FactKey> {
        self.stage.as_ref()
    }

    pub fn progress(&self) -> Option<&QuestProgress> {
        self.progress.as_deref()
    }

    pub fn last_journal(&self) -> Option<&JournalRead> {
        self.last_read.as_deref()
    }

    fn progress_slice(&self) -> &[QuestProgress] {
        self.progress
            .as_deref()
            .map(std::slice::from_ref)
            .unwrap_or(&[])
    }

    fn blocked_failure(&self) -> ScriptFailure {
        ScriptFailure {
            code: Arc::from("parked"),
            message: self
                .last_error
                .clone()
                .unwrap_or_else(|| Arc::from(self.park_reason)),
            retryable: true,
        }
    }

    fn record_failure(&mut self, error: ActionError) {
        self.last_error = Some(match error {
            ActionError::Unavailable(reason)
            | ActionError::Failed(reason)
            | ActionError::Blocked(reason) => reason,
            error => Arc::from(format!("step error: {error:?}")),
        });
        self.dirty = true;
    }

    fn update_wait(&mut self) {
        let waiting = self.step.as_ref().and_then(|step| step.waiting_for());
        if self
            .waiting
            .as_ref()
            .map(|(reason, name)| (*reason, name.as_ref()))
            != waiting.map(|(reason, name)| (reason, name.as_ref()))
        {
            self.waiting = waiting.map(|(reason, name)| (reason, Arc::clone(name)));
            self.dirty = true;
        }
    }

    fn publish(&mut self, output: &mut dyn NativeOutput) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        let stage = self
            .stage
            .as_ref()
            .map(|s| StatusValue::Text(Arc::from(s.0.as_ref())))
            .unwrap_or(StatusValue::Text(Arc::from("unknown")));
        let mut fields = vec![
            StatusField {
                key: "quest",
                label: "Quest",
                value: StatusValue::Text(Arc::clone(&self.path.display_name)),
            },
            StatusField {
                key: "stage",
                label: "Stage",
                value: stage,
            },
            StatusField {
                key: "deaths",
                label: "Deaths",
                value: StatusValue::Integer(i64::from(self.deaths)),
            },
            StatusField {
                key: "needs_read",
                label: "Reading progress",
                value: StatusValue::Truth(if self.needs_read || self.read_requested {
                    Truth::True
                } else {
                    Truth::False
                }),
            },
        ];
        append_combat_status(&mut fields, self.last_outcome.as_ref());
        if let Some(progress) = &self.progress {
            fields.push(StatusField {
                key: "progress",
                label: "Progress",
                value: StatusValue::Quest(Arc::clone(progress)),
            });
            fields.push(StatusField {
                key: "rule",
                label: "Matched rule",
                value: StatusValue::Text(match &progress.rule {
                    Knowledge::Known(rule) => Arc::clone(&rule.0),
                    Knowledge::Unknown(_) | Knowledge::Partial { .. } => Arc::from("unknown"),
                }),
            });
        }
        if let Some(text) = &self.journal_text {
            fields.push(StatusField {
                key: "journal_lines",
                label: "Journal",
                value: StatusValue::Text(Arc::clone(text)),
            });
        }
        if let Some(hint) = self
            .stage
            .as_ref()
            .and_then(|stage| self.path.progress.varp_hint(stage))
        {
            fields.push(StatusField {
                key: "varp_hint",
                label: "Fixture stage hint",
                value: StatusValue::Integer(i64::from(hint)),
            });
        }
        let detail = self
            .waiting
            .as_ref()
            .map(|(reason, name)| ("waiting_for", *reason, name))
            .or_else(|| {
                self.last_error
                    .as_ref()
                    .map(|reason| ("last_failure", "Last failure", reason))
            });
        if let Some((key, label, value)) = detail {
            fields.push(StatusField {
                key,
                label,
                value: StatusValue::Text(Arc::clone(value)),
            });
        }
        output.status(ScriptStatus {
            run: self.run,
            card: CompiledId("Quester"),
            phase: if self.parked {
                NativePhase::Blocked
            } else {
                NativePhase::Working
            },
            active_settings: 1,
            pending_settings: None,
            fields: fields.into(),
            failure: self.parked.then(|| self.blocked_failure()),
        });
    }

    fn cancel_step(&mut self, tick: &mut NativeTick<'_>) {
        if let Some(mut step) = self.step.take() {
            step.cancel(tick.actions);
        }
        self.clear_prayers = None;
        self.prayer_cleanup_pending = true;
        self.last_outcome = None;
        self.journal = None;
        self.advances = false;
        self.attempts = 0;
        self.settling = false;
        self.settle_deadline = Duration::ZERO;
        if self.waiting.take().is_some() {
            self.dirty = true;
        }
        self.last_error = None;
    }

    fn current_step(&self) -> Option<&CompiledStep> {
        if self.in_prelude {
            self.path.prelude.get(self.step_index)
        } else {
            self.path
                .sequences
                .get(self.seq_index)?
                .steps
                .get(self.step_index)
        }
    }

    fn wait_for_read(&mut self, tick: &NativeTick<'_>, reason: &'static str) -> bool {
        let since = self.unreadable_since.get_or_insert(tick.cx.active_now());
        if tick.cx.active_now().saturating_sub(*since) >= Duration::from_secs(8) {
            self.unreadable_reads += 1;
            *since = tick.cx.active_now();
            if self.unreadable_reads >= 2 {
                self.parked = true;
                self.park_reason = reason;
                self.last_error = None;
                self.dirty = true;
            }
        }
        false
    }

    fn wait_for_journal_read(&mut self, tick: &NativeTick<'_>, reason: &'static str) -> bool {
        let waiting = self.wait_for_read(tick, reason);
        if self.parked {
            if let Some(chat) = tick.cx.snapshot().chat_modal() {
                if chat.value.root != -1 || !chat.value.texts.is_empty() {
                    self.last_error = Some(Arc::from(format!(
                        "journal blocked by modal root {}{}",
                        chat.value.root,
                        chat.value
                            .texts
                            .iter()
                            .find(|text| !text.is_empty())
                            .map(|text| format!(" ({text})"))
                            .unwrap_or_default()
                    )));
                }
            }
        }
        waiting
    }

    /// `limit` is the Blocked message once the per-read cap is spent; it names
    /// the cap and the same transient cause as `reason`.
    fn retry_journal_read(
        &mut self,
        tick: &NativeTick<'_>,
        reason: &'static str,
        limit: &'static str,
    ) -> bool {
        self.journal_retry_pending = true;
        self.journal_quiet_since = None;
        if self.journal_attempts >= JOURNAL_READ_ATTEMPTS {
            self.parked = true;
            self.park_reason = "journal read retry limit reached";
            self.last_error = Some(Arc::from(limit));
            self.dirty = true;
            return false;
        }
        self.wait_for_read(tick, reason)
    }

    fn read_stage(&mut self, tick: &mut NativeTick<'_>, retarget: bool) -> bool {
        let progress = if let Some(handle) = self.journal.as_ref() {
            match tick.actions.poll(handle, &mut tick.cx) {
                Poll::Pending => return false,
                Poll::Ready(Err(error)) => {
                    self.journal = None;
                    match error {
                        ActionError::Busy | ActionError::Held | ActionError::BudgetExhausted => {
                            return self.retry_journal_read(
                                tick,
                                "journal remained busy during read",
                                JOURNAL_RETRY_LIMIT_BUSY,
                            );
                        }
                        ActionError::Failed(reason)
                            if reason.as_ref() == "journal ownership lost before close"
                                || reason.as_ref() == "journal ownership lost while closing" =>
                        {
                            return self.retry_journal_read(
                                tick,
                                "journal ownership repeatedly lost",
                                JOURNAL_RETRY_LIMIT_OWNERSHIP_LOST,
                            );
                        }
                        error => {
                            self.record_failure(error);
                            self.parked = true;
                        }
                    }
                    return false;
                }
                Poll::Ready(Ok(read)) => {
                    self.journal = None;
                    let progress = resolve_journal(&self.path, &read, self.progress.as_deref());
                    self.journal_text = Some(Arc::from(
                        read.lines
                            .iter()
                            .map(AsRef::as_ref)
                            .collect::<Vec<&str>>()
                            .join("\n"),
                    ));
                    self.last_read = Some(Arc::new(read));
                    progress
                }
            }
        } else {
            let Some(colour) = quest_colour(&self.path, &self.quests, tick.cx.snapshot()) else {
                return self.wait_for_read(tick, "quest colour unavailable or unknown stage");
            };
            if colour == QuestListStatus::Unknown {
                return self.wait_for_read(tick, "quest colour unavailable or unknown stage");
            }
            if colour == QuestListStatus::InProgress && !self.path.progress.rules.is_empty() {
                if self.journal_retry_pending {
                    let closed = tick.cx.snapshot().main_modal().is_some_and(|modal| {
                        modal.value.root == -1 && modal.value.texts.is_empty()
                    }) && tick.cx.snapshot().chat_modal().is_some_and(|modal| {
                        modal.value.root == -1 && modal.value.texts.is_empty()
                    });
                    if !closed {
                        self.journal_quiet_since = None;
                        return self
                            .wait_for_journal_read(tick, "journal retry quiet period unavailable");
                    }
                    let current_tick = tick.cx.evidence().tick;
                    // The +1 encoding reserves zero for "not started"; 32-bit
                    // game ticks cover about 81 years at the engine's tick rate,
                    // and a saturated stamp only shortens that quiet wait.
                    let since = self.journal_quiet_since.get_or_insert_with(|| {
                        let stamp =
                            u32::try_from(current_tick.saturating_add(1)).unwrap_or(u32::MAX);
                        NonZeroU32::new(stamp).unwrap_or(NonZeroU32::MIN)
                    });
                    if current_tick.saturating_sub(u64::from(since.get().saturating_sub(1)))
                        < JOURNAL_RETRY_QUIET_TICKS
                    {
                        return self
                            .wait_for_journal_read(tick, "journal retry quiet period unavailable");
                    }
                }
                let args = JournalRequest {
                    quest: self.path.id.clone(),
                    facts: Arc::clone(&self.quests),
                };
                match tick.actions.begin::<JournalMachine>(args, &mut tick.cx) {
                    Ok(handle) => {
                        self.journal = Some(handle);
                        self.journal_attempts += 1;
                        self.journal_retry_pending = false;
                        self.journal_quiet_since = None;
                        self.journal_opened = true;
                        self.dirty = true;
                    }
                    Err(ActionError::Busy | ActionError::Held | ActionError::BudgetExhausted) => {
                        return self
                            .wait_for_journal_read(tick, "journal blocked by an occupied modal");
                    }
                    Err(error) => {
                        self.record_failure(error);
                        self.parked = true;
                    }
                }
                return false;
            }
            resolve_colour(
                &self.path,
                colour,
                tick.cx.evidence(),
                Arc::new(tick.cx.pin().clone()),
            )
        };
        let stage = match &progress.stage {
            Knowledge::Known(stage) => Some(stage.clone()),
            Knowledge::Unknown(_) | Knowledge::Partial { .. } => None,
        };
        self.journal_attempts = 0;
        self.journal_retry_pending = false;
        self.journal_quiet_since = None;
        self.progress = Some(Arc::new(progress));
        self.dirty = true;
        let sequence = stage
            .as_ref()
            .and_then(|stage| sequence_for_stage(&self.path, &stage.0));
        let Some((stage, sequence)) = stage.zip(sequence) else {
            self.stage = None;
            if self.last_read.is_some() {
                self.parked = true;
                self.park_reason = "no journal rule matched or stage has no sequence";
                self.last_error = None;
                return false;
            }
            return self.wait_for_read(tick, "quest colour unavailable or unknown stage");
        };
        self.unreadable_since = None;
        self.unreadable_reads = 0;
        self.selection_since = None;
        if self.stage.as_ref() != Some(&stage) {
            self.stage = Some(stage);
            self.empty_reads = 0;
        }
        if retarget {
            self.seq_index = sequence;
            self.step_index = 0;
            self.in_prelude = false;
        } else {
            // Journal latency does not spend the family's post-read settle
            // window; its stage predicate only sees the newly acquired proof.
            self.settle_deadline = tick.cx.active_now()
                + self
                    .current_step()
                    .map(|step| step.plan.settle_timeout())
                    .unwrap_or_default();
        }
        self.needs_read = false;
        self.read_requested = false;
        true
    }

    fn on_step_boundary(&mut self, tick: &NativeTick<'_>) {
        self.prayer_cleanup_pending = true;
        let mut inv = [(0, 0); 28];
        let mut inv_len = 0usize;
        if let Some(rows) = tick.cx.snapshot().inventory() {
            for item in rows.value.iter().take(28) {
                inv[inv_len] = (item.def.id, item.count);
                inv_len += 1;
            }
        }
        let mut worn = [0; 14];
        let mut worn_len = 0usize;
        if let Some(rows) = tick.cx.snapshot().equipment() {
            for item in rows.value.iter().take(14) {
                worn[worn_len] = item.def.id;
                worn_len += 1;
            }
        }
        let tile = tick.cx.snapshot().here().map(|obs| obs.value);
        let xp = tick
            .cx
            .snapshot()
            .stats()
            .map(|stats| stats.value.iter().map(|s| s.xp).sum())
            .unwrap_or(0);
        if matches!(
            self.watchdog.observe(
                self.stage.as_ref(),
                tile,
                &inv[..inv_len],
                &worn[..worn_len],
                xp,
            ),
            WatchdogAction::Park
        ) {
            self.parked = true;
            self.dirty = true;
        }
    }
}

impl Script for Quester {
    fn tick(&mut self, tick: &mut NativeTick<'_>) -> Result<ScriptFlow, ScriptFailure> {
        if tick.cx.run() != self.run {
            self.run = tick.cx.run();
            self.cancel_step(tick);
            self.needs_read = true;
            self.progress = None;
        }
        if !tick.cx.eligible {
            self.publish(tick.output);
            return Ok(ScriptFlow::Continue);
        }
        if self.prayer_cleanup_pending && self.step.is_some() {
            self.cancel_step(tick);
            self.dirty = true;
        }
        if self.death.observe(tick.cx.snapshot()) {
            self.cancel_step(tick);
            self.deaths = self.deaths.saturating_add(1);
            self.needs_read = true;
            self.progress = None;
            self.dirty = true;
        }
        if let Some(result) = self
            .clear_prayers
            .as_ref()
            .map(|handle| tick.actions.poll(handle, &mut tick.cx))
        {
            match result {
                Poll::Pending => {
                    self.publish(tick.output);
                    return Ok(ScriptFlow::Continue);
                }
                Poll::Ready(Ok(report)) => {
                    self.clear_prayers = None;
                    if report.timed_out != 0 {
                        self.prayer_cleanup_pending = true;
                        self.parked = true;
                        self.record_failure(ActionError::Blocked(Arc::from(
                            "prayer cleanup timed out",
                        )));
                        self.publish(tick.output);
                        return Ok(ScriptFlow::Blocked(self.blocked_failure()));
                    }
                    self.prayer_cleanup_pending = false;
                }
                Poll::Ready(Err(
                    ActionError::Busy
                    | ActionError::Held
                    | ActionError::Stale
                    | ActionError::Cancelled
                    | ActionError::BudgetExhausted,
                )) => {
                    self.clear_prayers = None;
                    self.prayer_cleanup_pending = true;
                    self.publish(tick.output);
                    return Ok(ScriptFlow::Continue);
                }
                Poll::Ready(Err(error)) => {
                    self.clear_prayers = None;
                    self.parked = true;
                    self.record_failure(error);
                    self.publish(tick.output);
                    return Ok(ScriptFlow::Blocked(self.blocked_failure()));
                }
            }
        }
        if self.prayer_cleanup_pending && self.step.is_none() && !self.settling {
            let Some(active) = tick.cx.snapshot().prayers_active() else {
                self.publish(tick.output);
                return Ok(ScriptFlow::Continue);
            };
            if active.value.iter().any(|on| *on) {
                match tick
                    .actions
                    .begin::<ClearPrayers>(Arc::clone(&self.selected), &mut tick.cx)
                {
                    Ok(handle) => {
                        self.clear_prayers = Some(handle);
                        self.publish(tick.output);
                        return Ok(ScriptFlow::Continue);
                    }
                    Err(
                        ActionError::Busy
                        | ActionError::Held
                        | ActionError::Stale
                        | ActionError::Cancelled
                        | ActionError::BudgetExhausted,
                    ) => {
                        self.publish(tick.output);
                        return Ok(ScriptFlow::Continue);
                    }
                    Err(error) => {
                        self.parked = true;
                        self.record_failure(error);
                        self.publish(tick.output);
                        return Ok(ScriptFlow::Blocked(self.blocked_failure()));
                    }
                }
            }
            self.prayer_cleanup_pending = false;
        }
        if self.parked {
            self.publish(tick.output);
            return Ok(ScriptFlow::Blocked(self.blocked_failure()));
        }
        if self.step.is_none() && !self.settling && !self.needs_read {
            let contradicted = quest_colour(&self.path, &self.quests, tick.cx.snapshot())
                .is_some_and(|colour| match colour {
                    QuestListStatus::Complete => {
                        self.stage.as_ref() != Some(&self.path.colour_complete)
                    }
                    QuestListStatus::NotStarted => {
                        self.stage.as_ref() != Some(&self.path.colour_not_started)
                    }
                    QuestListStatus::InProgress => self.stage.as_ref().is_some_and(|stage| {
                        stage == &self.path.colour_not_started
                            || stage == &self.path.colour_complete
                    }),
                    QuestListStatus::Unknown => false,
                });
            if self.read_requested || contradicted {
                self.needs_read = true;
                self.dirty = true;
            }
        }
        if self.needs_read {
            let retarget = !self.settling && self.step.is_none();
            if !self.read_stage(tick, retarget) {
                self.publish(tick.output);
                return Ok(ScriptFlow::Continue);
            }
            if let Some(step) = self.step.as_mut() {
                step.progress_read_completed(tick.cx.active_now());
            }
        }
        if self
            .path
            .sequences
            .get(self.seq_index)
            .is_some_and(|seq| seq.terminal && seq.steps.is_empty())
        {
            self.publish(tick.output);
            return Ok(ScriptFlow::Complete);
        }
        if self.settling {
            let truth = {
                let pred = PredicateContext {
                    cx: &tick.cx,
                    quests: &self.quests,
                    progress: self.progress_slice(),
                    required_after: tick.cx.evidence(),
                    chat_since: self.chat_since,
                    outcome: self.last_outcome.as_ref(),
                    bank: &self.bank,
                };
                self.current_step()
                    .map(|step| step.settle.evaluate(&pred))
                    .unwrap_or(Truth::False)
            };
            if truth == Truth::True {
                self.settling = false;
                self.settle_deadline = Duration::ZERO;
                self.fail_streak = 0;
                if let Some(stage) = self.stage.as_ref() {
                    self.seq_index =
                        sequence_for_stage(&self.path, stage.0.as_ref()).unwrap_or(self.seq_index);
                    self.step_index = 0;
                }
                self.on_step_boundary(tick);
            } else {
                if tick.cx.active_now() >= self.settle_deadline {
                    self.settling = false;
                    self.fail_streak = self.fail_streak.saturating_add(1);
                    self.record_failure(ActionError::Failed(Arc::from("step settle timeout")));
                    if self.fail_streak >= 5 {
                        self.parked = true;
                    }
                    self.on_step_boundary(tick);
                    if let Some(stage) = self.stage.as_ref() {
                        self.seq_index =
                            sequence_for_stage(&self.path, &stage.0).unwrap_or(self.seq_index);
                        self.step_index = 0;
                    }
                }
            }
            self.publish(tick.output);
            return Ok(ScriptFlow::Continue);
        }
        if self.step.is_none() {
            let _ = self.provisioner.check();
            let selected = {
                let pred = PredicateContext {
                    cx: &tick.cx,
                    quests: &self.quests,
                    progress: self.progress_slice(),
                    required_after: tick.cx.evidence(),
                    chat_since: super::families::reach::last_chat_seq(&tick.cx),
                    outcome: self.last_outcome.as_ref(),
                    bank: &self.bank,
                };
                match select(&self.path, self.seq_index, &pred) {
                    SelectionDecision::Selected(sel) => {
                        Ok(Some((sel.index, sel.step.advances, sel.prelude)))
                    }
                    SelectionDecision::Exhausted => Ok(None),
                    SelectionDecision::Unknown => Err(()),
                }
            };
            let selected = match selected {
                Ok(selected) => selected,
                Err(()) => {
                    let since = self.selection_since.get_or_insert(tick.cx.active_now());
                    if tick.cx.active_now().saturating_sub(*since) >= Duration::from_secs(16) {
                        self.parked = true;
                        self.park_reason = "skip predicate evidence unavailable";
                        self.last_error = None;
                        self.dirty = true;
                    }
                    self.publish(tick.output);
                    return Ok(ScriptFlow::Continue);
                }
            };
            self.selection_since = None;
            if selected.is_none() {
                self.last_outcome = None;
            }
            let Some((index, advances, prelude)) = selected else {
                self.empty_reads += 1;
                if self.empty_reads >= 2 {
                    self.parked = true;
                    self.park_reason = "no step for stage";
                    self.last_error = None;
                    self.dirty = true;
                }
                self.needs_read = true;
                self.publish(tick.output);
                return Ok(ScriptFlow::Continue);
            };
            self.step_index = index;
            self.advances = advances;
            self.empty_reads = 0;
            self.in_prelude = prelude;
            let step = if prelude {
                &self.path.prelude[index]
            } else {
                &self.path.sequences[self.seq_index].steps[index]
            };
            self.chat_since = super::families::reach::last_chat_seq(&tick.cx);
            let required_after = tick.cx.evidence();
            let mut step_cx = StepContext {
                tick,
                quests: &self.quests,
                progress: self.progress_slice(),
                required_after,
                bank: &self.bank,
            };
            match step.plan.begin(&mut step_cx) {
                Ok(run) => {
                    self.step = Some(run);
                    self.last_outcome = None;
                    self.dirty = true;
                    self.last_error = None;
                }
                Err(error) => {
                    self.attempts = self.attempts.saturating_add(1);
                    self.record_failure(error);
                    if self.attempts >= 5 {
                        self.parked = true;
                    }
                }
            }
            self.publish(tick.output);
            return Ok(ScriptFlow::Continue);
        }
        let required_after = tick.cx.evidence();
        let poll = {
            let mut step_cx = StepContext {
                tick,
                quests: &self.quests,
                progress: self
                    .progress
                    .as_deref()
                    .map(std::slice::from_ref)
                    .unwrap_or(&[]),
                required_after,
                bank: &self.bank,
            };
            self.step
                .as_mut()
                .map(|step| step.poll(&mut step_cx))
                .unwrap_or(Poll::Pending)
        };
        match poll {
            Poll::Pending => {
                if self
                    .step
                    .as_ref()
                    .is_some_and(|step| step.needs_progress_read())
                {
                    self.needs_read = true;
                    self.dirty = true;
                }
            }
            Poll::Ready(Ok(outcome)) => {
                if let Some(receipt) = outcome.receipt.as_deref().and_then(|receipt| {
                    receipt
                        .as_any()
                        .downcast_ref::<crate::native_bank::BankReceipt>()
                }) {
                    self.bank.update(receipt);
                }
                self.last_outcome = Some(outcome);
                self.prayer_cleanup_pending = true;
                self.dirty = true;
                if self.advances {
                    self.needs_read = true;
                }
                self.step = None;
                self.settling = true;
                self.settle_deadline = tick.cx.active_now()
                    + self
                        .current_step()
                        .map(|step| step.plan.settle_timeout())
                        .unwrap_or_default();
            }
            Poll::Ready(Err(error @ ActionError::Blocked(_))) => {
                self.step = None;
                self.last_outcome = None;
                self.prayer_cleanup_pending = true;
                self.parked = true;
                self.record_failure(error);
                self.update_wait();
                self.publish(tick.output);
                return Ok(ScriptFlow::Blocked(self.blocked_failure()));
            }
            Poll::Ready(Err(error)) => {
                self.step = None;
                self.last_outcome = None;
                self.prayer_cleanup_pending = true;
                self.fail_streak = self.fail_streak.saturating_add(1);
                self.record_failure(error);
                if self.fail_streak >= 5 {
                    self.parked = true;
                }
                self.on_step_boundary(tick);
            }
        }
        self.update_wait();
        self.publish(tick.output);
        Ok(ScriptFlow::Continue)
    }

    fn interrupt(&mut self, event: Interrupt) {
        match event {
            Interrupt::Resume | Interrupt::SessionReady => {
                self.needs_read = true;
                self.step = None;
                self.prayer_cleanup_pending = true;
                self.last_outcome = None;
                self.settling = false;
                self.journal = None;
                self.progress = None;
                self.unreadable_since = None;
                self.journal_attempts = 0;
                self.journal_retry_pending = false;
                self.journal_quiet_since = None;
                self.selection_since = None;
                self.dirty = true;
                self.waiting = None;
            }
            Interrupt::SessionEnded => {
                self.step = None;
                self.prayer_cleanup_pending = true;
                self.last_outcome = None;
                self.journal = None;
                self.progress = None;
                self.settling = false;
                self.needs_read = true;
                self.journal_attempts = 0;
                self.journal_retry_pending = false;
                self.journal_quiet_since = None;
                self.waiting = None;
            }
            Interrupt::Pause | Interrupt::Hold(_) => {}
        }
    }
    fn on_random(&mut self, _event: &DetectedRandom) -> RandomClaim {
        self.prayer_cleanup_pending = true;
        RandomClaim::Host
    }

    fn on_stop(&mut self, _reason: StopReason) {
        self.step = None;
        self.journal = None;
        self.settling = false;
        self.dirty = true;
        self.waiting = None;
        self.clear_prayers = None;
        self.prayer_cleanup_pending = false;
        self.last_outcome = None;
    }

    fn read_journal(&mut self) -> Result<(), ScriptFailure> {
        if self.parked {
            self.retry()?;
        }
        self.read_requested = true;
        self.dirty = true;
        Ok(())
    }

    fn retry(&mut self) -> Result<(), ScriptFailure> {
        self.parked = false;
        self.needs_read = true;
        self.step = None;
        self.clear_prayers = None;
        self.prayer_cleanup_pending = true;
        self.last_outcome = None;
        self.journal = None;
        self.progress = None;
        self.settling = false;
        self.selection_since = None;
        self.fail_streak = 0;
        self.attempts = 0;
        self.watchdog = Watchdog::default();
        self.empty_reads = 0;
        self.unreadable_reads = 0;
        self.unreadable_since = None;
        self.journal_attempts = 0;
        self.journal_retry_pending = false;
        self.journal_quiet_since = None;
        self.last_error = None;
        self.waiting = None;
        self.park_reason = "no progress";
        self.dirty = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quester_struct_fits_the_per_bot_budget() {
        let bytes = std::mem::size_of::<Quester>();
        let bank_bytes = std::mem::size_of::<BankMemo>();
        eprintln!("Quester size_of={bytes}; BankMemo size_of={bank_bytes}, heap=0");
        assert!(bytes < 4096, "Quester is {bytes} bytes");
        assert!(bank_bytes <= 520, "BankMemo is {bank_bytes} bytes");
    }
    #[test]
    fn combat_report_is_exposed_in_quester_status() {
        use crate::combat::{CombatEnd, CombatReport};
        use api::quest_progress::EvidenceStamp;

        #[derive(Default)]
        struct Capture(Vec<ScriptStatus>);
        impl NativeOutput for Capture {
            fn status(&mut self, status: ScriptStatus) {
                self.0.push(status);
            }
            fn paint(&mut self, _: Arc<crate::shim::ScriptPaint>) {}
            fn log(&mut self, _: api::hostlog::Level, _: &str) {}
            fn settings_applied(&mut self, _: u64) {}
        }

        let (mut script, _) = fixture();
        let report = CombatReport {
            end: CombatEnd::TargetGone,
            evidence: EvidenceStamp {
                run: script.run,
                tick: 23,
                sequence: 4,
            },
            engaged: None,
            engaged_npc_type: 477,
            ticks: 9,
            swings: 3,
            casts: 0,
            damage_taken: 2,
            food: 1,
            prayer_doses: 0,
            boost_doses: 0,
            antifire_doses: 0,
            hits_while_protected: 0,
            protect_switches: 0,
            intruders: 0,
            ammo_pickups: 0,
            restorations: 0,
            locked_ticks: 0,
            multi_op_plans: 0,
            melee_mode_fallback: None,
            flick_resets: 0,
            flick_misses: 0,
            flick_fallback: false,
        };
        script.last_outcome = Some(StepOutcome {
            progress: None,
            evidence: report.evidence,
            receipt: Some(Arc::new(CombatReceipt {
                report,
                target_gone_restarts: 1,
            })),
        });
        let mut output = Capture::default();
        script.publish(&mut output);
        let fields = &output.0.last().unwrap().fields;
        assert!(fields.iter().any(|field| {
            field.key == "combat_end"
                && matches!(&field.value, StatusValue::Text(value) if value.as_ref() == "TargetGone")
        }));
        for (key, expected) in [
            ("combat_evidence_run_slot", 1),
            ("combat_evidence_run", 1),
            ("combat_evidence_session", 1),
            ("combat_evidence_tick", 23),
            ("combat_evidence_sequence", 4),
        ] {
            assert!(fields.iter().any(|field| {
                field.key == key
                    && matches!(&field.value, StatusValue::Integer(value) if *value == expected)
            }));
        }
        assert!(fields.iter().any(|field| {
            field.key == "combat_engaged"
                && matches!(&field.value, StatusValue::Truth(Truth::False))
        }));
        assert!(fields.iter().any(|field| {
            field.key == "combat_engaged_npc_type"
                && matches!(&field.value, StatusValue::Integer(477))
        }));
        assert!(fields.iter().any(|field| {
            field.key == "combat_food" && matches!(&field.value, StatusValue::Integer(1))
        }));
        assert!(fields.iter().any(|field| {
            field.key == "combat_target_gone_restarts"
                && matches!(&field.value, StatusValue::Integer(1))
        }));
    }

    #[test]
    fn message_settle_uses_step_begin_chat_not_tick_sequence() {
        use super::super::families::tests::with_tick;
        use api::snapshot::{ChatLineView, GameSnapshot, QuestStatusView};
        let data = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
        let quests = Arc::new(QuestCatalog::from_identity(data.quest_identity()).unwrap());
        let mut document = super::super::compile::decode_cook().unwrap();
        let step = &mut document.roles[0].sequences[0].steps[0];
        step.kind = "wait".into();
        step.args = serde_json::json!({"until":{"All":[]},"max_ticks": 10});
        step.advances = false;
        step.settle = super::super::path::PredicateDocument::Fact {
            kind: "message".into(),
            version: 1,
            args: serde_json::json!({"any": ["you put the grain in the hopper"]}),
        };
        let path =
            super::super::compile::compile_uncached_for_test(&document, &data, &quests).unwrap();
        let run = RunKey {
            slot: 1,
            run: 1,
            session: 1,
        };
        let mut script = Quester::new(run, path, Arc::clone(&data), quests);
        let mut s = GameSnapshot::new();
        s.seed_ingame(2);
        s.seed_quest_statuses(
            vec![QuestStatusView {
                name: "Cook's Assistant".into(),
                component_id: 0,
                colour: 0xf80000,
            }],
            true,
        );
        s.seed_chat_lines(vec![ChatLineView {
            type_: 0,
            username: None,
            text: "you put the grain in the hopper".into(),
            sequence: 2,
        }]);
        let mut ledger = None;
        with_tick(&s, &mut ledger, 5000, |t| script.tick(t).unwrap());
        with_tick(&s, &mut ledger, 5001, |t| script.tick(t).unwrap());
        with_tick(&s, &mut ledger, 5002, |t| script.tick(t).unwrap());
        assert!(script.settling, "old matching chat must not settle");
        s.seed_chat_lines(vec![ChatLineView {
            type_: 0,
            username: None,
            text: "You put the grain in the hopper.".into(),
            sequence: 3,
        }]);
        with_tick(&s, &mut ledger, 5003, |t| script.tick(t).unwrap());
        assert!(
            !script.settling,
            "a new sequence 3 message settles at tick 5003"
        );
    }

    fn fixture() -> (Quester, api::snapshot::GameSnapshot) {
        let data = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
        let quests = Arc::new(QuestCatalog::from_identity(data.quest_identity()).unwrap());
        let path = super::super::compile::compile_uncached_for_test(
            &super::super::compile::decode_cook().unwrap(),
            &data,
            &quests,
        )
        .unwrap();
        let mut s = api::snapshot::GameSnapshot::new();
        s.seed_ingame(2);
        (
            Quester::new(
                RunKey {
                    slot: 1,
                    run: 1,
                    session: 1,
                },
                path,
                Arc::clone(&data),
                quests,
            ),
            s,
        )
    }

    #[test]
    fn combat_interrupted_talk_parks_without_advancing_or_reading() {
        use super::super::families::tests::{seed_dialogue_combat, with_tick, with_tick_output};
        use api::snapshot::{GameSnapshot, QuestStatusView};
        #[derive(Default)]
        struct Capture(Vec<ScriptStatus>);
        impl NativeOutput for Capture {
            fn status(&mut self, status: ScriptStatus) {
                self.0.push(status);
            }
            fn paint(&mut self, _: Arc<crate::shim::ScriptPaint>) {}
            fn log(&mut self, _: api::hostlog::Level, _: &str) {}
            fn settings_applied(&mut self, _: u64) {}
        }
        let data = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
        let quests = Arc::new(QuestCatalog::from_identity(data.quest_identity()).unwrap());
        let mut document = super::super::compile::decode_cook().unwrap();
        let step = &mut document.roles[0].sequences[0].steps[0];
        step.kind = "talk".into();
        step.args = serde_json::json!({"npc": "cook"});
        step.advances = true;
        step.skip_if = super::super::path::PredicateDocument::Any(vec![]);
        step.settle = super::super::path::PredicateDocument::All(vec![]);
        let path =
            super::super::compile::compile_uncached_for_test(&document, &data, &quests).unwrap();
        let mut script = Quester::new(
            RunKey {
                slot: 1,
                run: 1,
                session: 1,
            },
            path,
            Arc::clone(&data),
            quests,
        );
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        seed_dialogue_combat(&mut snapshot, false);
        snapshot.seed_quest_statuses(
            vec![QuestStatusView {
                name: "Cook's Assistant".into(),
                component_id: 0,
                colour: 0xf80000,
            }],
            true,
        );
        snapshot.seed_chat_modal(4882, vec![]);
        let mut ledger = None;
        for tick in 1..=3 {
            with_tick(&snapshot, &mut ledger, tick, |t| {
                assert!(matches!(script.tick(t).unwrap(), ScriptFlow::Continue));
            });
        }
        let cursor = (script.seq_index, script.step_index);
        let progress_evidence = script.progress.as_ref().unwrap().evidence;
        snapshot.seed_chat_modal(-1, vec![]);
        seed_dialogue_combat(&mut snapshot, true);
        let mut output = Capture::default();
        for tick in 4..=9 {
            if tick > 4 {
                seed_dialogue_combat(&mut snapshot, false);
            }
            with_tick_output(&snapshot, &mut ledger, tick, &mut output, |t| {
                assert!(matches!(
                    script.tick(t).unwrap(),
                    ScriptFlow::Blocked(failure)
                        if failure.message.as_ref() == "dialogue interrupted by combat"
                ));
            });
        }
        assert_eq!((script.seq_index, script.step_index), cursor);
        assert_eq!(
            script.progress.as_ref().unwrap().evidence,
            progress_evidence
        );
        assert!(!script.needs_read && !script.settling);
        assert!(!script.journal_opened());
        let status = output.0.last().unwrap();
        assert_eq!(status.phase, NativePhase::Blocked);
        assert_eq!(
            status.failure.as_ref().unwrap().message.as_ref(),
            "dialogue interrupted by combat"
        );
        script.interrupt(Interrupt::Resume);
        with_tick_output(&snapshot, &mut ledger, 10, &mut output, |t| {
            assert!(matches!(
                script.tick(t).unwrap(),
                ScriptFlow::Blocked(failure)
                    if failure.message.as_ref() == "dialogue interrupted by combat"
            ));
        });
        script.interrupt(Interrupt::SessionReady);
        with_tick_output(&snapshot, &mut ledger, 11, &mut output, |t| {
            assert!(matches!(
                script.tick(t).unwrap(),
                ScriptFlow::Blocked(failure)
                    if failure.message.as_ref() == "dialogue interrupted by combat"
            ));
        });
    }

    #[test]
    fn unreadable_start_never_dispatches_start_and_retargets_when_ready() {
        use super::super::families::tests::with_tick;
        let (mut script, mut s) = fixture();
        let mut ledger = None;
        with_tick(&s, &mut ledger, 10, |t| script.tick(t).unwrap());
        assert!(script.needs_read);
        assert!(script.step.is_none());
        s.seed_quest_statuses(
            vec![api::snapshot::QuestStatusView {
                name: "Cook's Assistant".into(),
                component_id: 0,
                colour: 0xf8f800,
            }],
            true,
        );
        with_tick(&s, &mut ledger, 11, |t| script.tick(t).unwrap());
        assert_eq!(script.stage().unwrap().0.as_ref(), "cook:1");
        assert_eq!(script.current_step().unwrap().id.0.as_ref(), "egg");
        assert!(ledger.as_ref().is_none_or(|l| l.outbox.is_empty()));
    }

    #[test]
    fn unavailable_colour_and_unselectable_stage_park_instead_of_spinning() {
        use super::super::families::tests::with_tick;
        let (mut script, s) = fixture();
        script.record_failure(ActionError::Unavailable("stale walk failure".into()));
        let mut ledger = None;
        for tick in 1..100 {
            with_tick(&s, &mut ledger, tick, |t| script.tick(t).unwrap());
        }
        assert!(script.parked);
        assert_eq!(
            script.blocked_failure().message.as_ref(),
            "quest colour unavailable or unknown stage"
        );
        assert!(ledger.as_ref().is_none_or(|l| l.outbox.is_empty()));
        let (mut script, mut s) = fixture();
        script.record_failure(ActionError::Unavailable("stale walk failure".into()));
        let path = Arc::get_mut(&mut script.path).unwrap();
        for step in &mut path.sequences[1].steps {
            struct Yes;
            impl super::super::compile::PredicatePlan for Yes {
                fn evaluate(&self, _: &PredicateContext<'_, '_>) -> Truth {
                    Truth::True
                }
            }
            step.skip_if = Arc::new(Yes);
        }
        s.seed_quest_statuses(
            vec![api::snapshot::QuestStatusView {
                name: "Cook's Assistant".into(),
                component_id: 0,
                colour: 0xf8f800,
            }],
            true,
        );
        for tick in 1..10 {
            with_tick(&s, &mut ledger, tick, |t| script.tick(t).unwrap());
        }
        assert!(script.parked);
        assert_eq!(
            script.blocked_failure().message.as_ref(),
            "no step for stage"
        );
    }

    #[test]
    fn missing_spawn_status_names_the_wait_and_retains_the_park_cause() {
        use super::super::families::tests::{with_tick, with_tick_output};
        use api::snapshot::{GameSnapshot, QuestStatusView};
        #[derive(Default)]
        struct Capture(Vec<ScriptStatus>);
        impl NativeOutput for Capture {
            fn status(&mut self, status: ScriptStatus) {
                self.0.push(status);
            }
            fn paint(&mut self, _: Arc<crate::shim::ScriptPaint>) {}
            fn log(&mut self, _: api::hostlog::Level, _: &str) {}
            fn settings_applied(&mut self, _: u64) {}
        }
        for (recipe, expected_name) in [
            (None, "Egg"),
            (Some("acquire:milk"), "Bucket"),
            (Some("acquire:flour"), "Pot"),
        ] {
            let data = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
            let quests = Arc::new(QuestCatalog::from_identity(data.quest_identity()).unwrap());
            let mut document = super::super::compile::decode_cook().unwrap();
            if let Some(recipe) = recipe {
                let mut step = document.quest.as_ref().unwrap().acquire[recipe][0].clone();
                step.id = FactKey("spawn-policy".into());
                // Exercise the real recipe's wait policy after approach; live runs
                // cover its authored geometry.
                step.args.as_object_mut().unwrap().remove("anchor");
                document.roles[0].sequences[0].steps[0] = step;
            } else {
                let step = &mut document.roles[0].sequences[0].steps[0];
                step.kind = "interact".into();
                step.args = serde_json::json!({"target":{"ground":"egg"},"op":"Take","wait_if_missing":true});
                step.advances = false;
            }
            let path = super::super::compile::compile_uncached_for_test(&document, &data, &quests)
                .unwrap();
            let mut script = Quester::new(
                RunKey {
                    slot: 1,
                    run: 1,
                    session: 1,
                },
                path,
                Arc::clone(&data),
                quests,
            );
            let mut s = GameSnapshot::new();
            s.seed_ingame(2);
            s.seed_inventory(vec![], 28);
            s.seed_ground_items(vec![]);
            s.seed_quest_statuses(
                vec![QuestStatusView {
                    name: "Cook's Assistant".into(),
                    component_id: 0,
                    colour: 0xf80000,
                }],
                true,
            );
            let mut ledger = None;
            let mut output = Capture::default();
            for tick in 1..=3 {
                with_tick_output(&s, &mut ledger, tick, &mut output, |t| {
                    script.tick(t).unwrap();
                });
            }
            let status = output.0.last().unwrap();
            assert_eq!(status.phase, NativePhase::Working);
            assert!(status.failure.is_none());
            assert!(status
            .fields
            .iter()
            .any(|field| field.label == "Waiting for ground spawn"
                && matches!(&field.value, StatusValue::Text(value) if value.as_ref() == expected_name)), "{expected_name} must await its shared spawn");
            assert!(ledger.as_ref().unwrap().outbox.is_empty());
            script.fail_streak = 4;
            with_tick_output(&s, &mut ledger, 205, &mut output, |t| {
                script.tick(t).unwrap();
            });
            let status = output.0.last().unwrap();
            assert_eq!(status.phase, NativePhase::Blocked);
            let failure = status.failure.as_ref().unwrap();
            assert!(failure.message.contains(expected_name) && failure.message.contains("spawn"));
            let flow = with_tick(&s, &mut ledger, 206, |t| script.tick(t).unwrap());
            assert!(
                matches!(flow, ScriptFlow::Blocked(failure) if failure.message.contains(expected_name))
            );
        }
    }

    #[test]
    fn retry_clears_attempt_and_watchdog_exhaustion() {
        let (mut script, s) = fixture();
        script.attempts = 5;
        script.parked = true;
        for _ in 0..9 {
            script.watchdog.observe(None, None, &[], &[], 0);
        }
        script.retry().unwrap();
        assert_eq!(script.attempts, 0);
        super::super::families::tests::with_tick(&s, &mut None, 1, |t| script.on_step_boundary(t));
        assert!(!script.parked);
    }
}

#[cfg(test)]
#[path = "journal_runner_tests.rs"]
mod journal_tests;
