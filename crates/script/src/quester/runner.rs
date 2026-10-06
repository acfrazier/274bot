//! §3.2 tick loop with colour-first, boundary-triggered journal evidence.
use super::bank_memo::BankMemo;
use super::compile::{
    AcquisitionTraceOutcome, CompiledPath, CompiledStep, PredicateContext, StepContext,
    StepOutcome, StepRun, StepTraceEvent,
};
use super::families::combat::CombatReceipt;
use super::progress::{quest_colour, resolve_colour, resolve_journal};
use super::provision::{ProvisionEvent, ProvisionPhase, Provisioner};
use super::queue::QueueStatus;
use super::registry::PathSource;
use super::select::{select_with_skips, sequence_for_stage, SelectionDecision};
use super::watchdog::{Watchdog, WatchdogAction};
use crate::combat::{begin_clear_owned_prayers, ClearPrayers, Hygiene, RaisedPrayers};
use crate::native::death::{death_cap_exceeded, default_max_deaths, DeathLatch};
use crate::native::{
    ActionError, ActionHandle, Interrupt, NativeOutput, NativePhase, NativeTick, Script,
    ScriptFailure, ScriptFlow, ScriptStatus, StatusField, StatusValue, StopReason,
};
use crate::quest_journal::{JournalMachine, JournalRequest};
use crate::CompiledId;
use api::game_data::SelectedGameData;
use api::quest_facts::QuestCatalog;
use api::quest_progress::{EvidenceStamp, JournalRead, QuestProgress};
use api::selected::{FactKey, Knowledge, RunKey, Truth};
use api::snapshot::QuestListStatus;
use api::{DetectedRandom, RandomClaim};
use std::num::NonZeroU32;
use std::sync::Arc;
use std::task::Poll;

use std::fmt::{self, Write as _};
use std::time::Duration;

// A read has at most three transactions (each can emit at most one row
// click). Adoption consumes a transaction too, but never adds a click.
const JOURNAL_READ_ATTEMPTS: u8 = 3;
const JOURNAL_RETRY_QUIET_TICKS: u64 = 3;
const QUEUE_QUEST_STATUS_WAIT: Duration = Duration::from_secs(30);
// Pair admission uses the broker's ten-minute inactivity budget in active time.
const PAIR_ADMISSION_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const JOURNAL_RETRY_LIMIT_BUSY: &str =
    "journal read retry limit reached (journal remained busy during read)";
const JOURNAL_RETRY_LIMIT_OWNERSHIP_LOST: &str =
    "journal read retry limit reached (journal ownership repeatedly lost)";

// Reports a newly published bank receipt. Unchanged stamps avoid receipt inspection.
fn publish_in_flight_bank_receipt(
    outcome: &StepOutcome,
    bank: &mut BankMemo,
    published_bank_receipt: &mut Option<EvidenceStamp>,
    dirty: &mut bool,
) -> bool {
    if *published_bank_receipt == Some(outcome.evidence) {
        return false;
    }
    let Some(receipt) = outcome.receipt.as_deref().and_then(|receipt| {
        receipt
            .as_any()
            .downcast_ref::<crate::native_bank::BankReceipt>()
    }) else {
        return false;
    };
    bank.update(receipt);
    *published_bank_receipt = Some(outcome.evidence);
    *dirty = true;
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QuesterFailureKind {
    Other,
    ManualMovement,
    MaxDeaths,
    NeedsEvidence,
}

struct ParkedStep {
    sequence_index: usize,
    step_index: usize,
    in_prelude: bool,
    id: Arc<str>,
}

const RUN_TRACE_EVENT_LIMIT: usize = 32;
const RUN_TRACE_LINE_LIMIT: usize = 192;

struct TraceEntry {
    line: String,
    repeats: u32,
}

#[derive(Default)]
struct RunTrace {
    entries: Vec<TraceEntry>,
    started: bool,
    truncated: bool,
    terminal_logged: bool,
}

struct TraceLineWriter<'a> {
    line: &'a mut String,
    remaining: usize,
    truncated: bool,
}

impl fmt::Write for TraceLineWriter<'_> {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        let mut end = 0;
        for (index, character) in text.char_indices() {
            let next = index + character.len_utf8();
            if next > self.remaining {
                break;
            }
            end = next;
        }
        self.line.push_str(&text[..end]);
        self.remaining -= end;
        if end == text.len() {
            Ok(())
        } else {
            self.truncated = true;
            Err(fmt::Error)
        }
    }
}

fn trace_line(args: fmt::Arguments<'_>, limit: usize) -> (String, bool) {
    let mut line = String::new();
    let mut writer = TraceLineWriter {
        line: &mut line,
        remaining: limit.saturating_sub(3),
        truncated: false,
    };
    let _ = writer.write_fmt(args);
    let truncated = writer.truncated;
    if truncated {
        line.push_str("...");
    }
    (line, truncated)
}

impl RunTrace {
    fn record(
        &mut self,
        output: &mut dyn NativeOutput,
        level: api::hostlog::Level,
        args: fmt::Arguments<'_>,
    ) {
        let (line, truncated) = trace_line(args, RUN_TRACE_LINE_LIMIT);
        self.truncated |= truncated;
        if let Some(entry) = self.entries.iter_mut().find(|entry| entry.line == line) {
            entry.repeats = entry.repeats.saturating_add(1);
            return;
        }
        if self.entries.len() >= RUN_TRACE_EVENT_LIMIT {
            self.truncated = true;
            return;
        }
        output.log(level, &line);
        self.entries.push(TraceEntry { line, repeats: 0 });
    }

    fn flush_repeats(&mut self, output: &mut dyn NativeOutput) {
        for entry in &mut self.entries {
            if entry.repeats == 0 {
                continue;
            }
            let (line, _) = trace_line(
                format_args!(
                    "{} (repeated {} additional times)",
                    entry.line, entry.repeats
                ),
                RUN_TRACE_LINE_LIMIT + 64,
            );
            output.log(api::hostlog::Level::Warn, &line);
            entry.repeats = 0;
        }
        if self.truncated {
            output.log(
                api::hostlog::Level::Warn,
                "quester trace truncated after the event limit",
            );
            self.truncated = false;
        }
    }

    fn terminal(
        &mut self,
        output: &mut dyn NativeOutput,
        level: api::hostlog::Level,
        args: fmt::Arguments<'_>,
    ) {
        let (line, truncated) = trace_line(args, RUN_TRACE_LINE_LIMIT);
        output.log(level, &line);
        if truncated {
            output.log(
                api::hostlog::Level::Warn,
                "quester terminal trace event was truncated",
            );
        }
    }
}
fn trace_root_event(
    trace: &mut RunTrace,
    output: &mut dyn NativeOutput,
    level: api::hostlog::Level,
    quest: &str,
    stage: &str,
    step: &CompiledStep,
    event: fmt::Arguments<'_>,
) {
    trace.record(
        output,
        level,
        format_args!(
            "quester {quest}: stage {stage} step {} ({}) {event}",
            step.id.0.as_ref(),
            step.kind.as_ref()
        ),
    );
}

fn trace_error_reason(error: &ActionError) -> &str {
    match error {
        ActionError::Unavailable(reason)
        | ActionError::Failed(reason)
        | ActionError::Blocked(reason) => reason.as_ref(),
        ActionError::NeedsEvidence(_) => "needs evidence",
        ActionError::UserInput => super::families::MANUAL_MOVEMENT_MESSAGE,
        _ => "step error",
    }
}

pub struct Quester {
    run: RunKey,
    path: Arc<CompiledPath>,
    selected: Arc<SelectedGameData>,
    quests: Arc<QuestCatalog>,
    banks: Arc<api::named_banks::NamedBankFacts>,
    choices: super::choices::QuestChoices,
    stage: Option<FactKey>,
    progress: Option<Arc<QuestProgress>>,
    journal: Option<ActionHandle<JournalMachine>>,
    custom_reader: Option<Box<dyn StepRun>>,
    custom_read_after: Option<api::quest_progress::EvidenceStamp>,
    pair_admission: Option<Box<dyn StepRun>>,
    pair_admission_since: Option<Duration>,
    pair_begin_since: Option<Duration>,
    pair_admitted: bool,
    gang_reader: super::gang::GangRead,
    bank: BankMemo,
    last_read: Option<Arc<JournalRead>>,
    journal_text: Option<Arc<str>>,
    read_requested: bool,
    selection_since: Option<Duration>,
    seq_index: usize,
    step_index: usize,
    step: Option<Box<dyn StepRun>>,
    /// Freshness boundary for the active step; overwritten at each successful begin.
    step_after: api::quest_progress::EvidenceStamp,
    failed_acquisition_child: Option<(Arc<str>, Arc<str>)>,
    trace: RunTrace,
    parked_step: Option<ParkedStep>,
    clear_prayers: Option<ActionHandle<ClearPrayers>>,
    prayer_cleanup_pending: bool,
    prayer_cleanup_owned: RaisedPrayers,
    last_outcome: Option<StepOutcome>,
    published_bank_receipt: Option<EvidenceStamp>,
    /// Latest Combat family receipt, kept after later non-combat steps begin
    /// so Path `combat_end` skip_if can still select the caller walk-out
    /// (design-combat.md:664).
    last_combat: Option<StepOutcome>,
    published_receipt: Option<Arc<dyn super::compile::FamilyReceipt>>,
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
    last_error_kind: QuesterFailureKind,
    waiting: Option<(&'static str, Arc<str>)>,
    deaths: u16,
    prior_deaths: u16,
    max_deaths: u8,
    attempts: u8,
    fail_streak: u8,
    parked: bool,
    journal_opened: bool,
    dirty: bool,
    watchdog: Watchdog,
    death: DeathLatch,
    anchor: Option<api::WorldTile>,
    provisioner: Provisioner,
    active_loadout: Option<Arc<str>>,
    queue_fields: Arc<[StatusField]>,
    required_vs_live: Arc<[StatusField]>,
    tested_stats_warning: Arc<str>,
}
fn skill_status_fields(gates: &[super::eligibility::SkillGate]) -> Arc<[StatusField]> {
    // Fixed keys keep the producer compact and allow S5 to consume integers
    // without parsing display text. A missing live field means unobserved.
    const KEYS: [(&str, &str); 25] = [
        ("required_attack", "live_attack"),
        ("required_defence", "live_defence"),
        ("required_strength", "live_strength"),
        ("required_hitpoints", "live_hitpoints"),
        ("required_ranged", "live_ranged"),
        ("required_prayer", "live_prayer"),
        ("required_magic", "live_magic"),
        ("required_cooking", "live_cooking"),
        ("required_woodcutting", "live_woodcutting"),
        ("required_fletching", "live_fletching"),
        ("required_fishing", "live_fishing"),
        ("required_firemaking", "live_firemaking"),
        ("required_crafting", "live_crafting"),
        ("required_smithing", "live_smithing"),
        ("required_mining", "live_mining"),
        ("required_herblore", "live_herblore"),
        ("required_agility", "live_agility"),
        ("required_thieving", "live_thieving"),
        ("required_slayer", "live_slayer"),
        ("required_farming", "live_farming"),
        ("required_runecraft", "live_runecraft"),
        ("required_skill_21", "live_skill_21"),
        ("required_skill_22", "live_skill_22"),
        ("required_skill_23", "live_skill_23"),
        ("required_skill_24", "live_skill_24"),
    ];
    let mut fields = vec![StatusField {
        key: "required_vs_live",
        label: "Skill gates",
        value: StatusValue::Integer(gates.len() as i64),
    }];
    for (skill, (required_key, live_key)) in KEYS.iter().copied().enumerate() {
        let Some(gate) = gates
            .iter()
            .filter(|gate| usize::from(gate.skill) == skill)
            .max_by_key(|gate| gate.required)
        else {
            continue;
        };
        fields.push(StatusField {
            key: required_key,
            label: "Required base level",
            value: StatusValue::Integer(i64::from(gate.required)),
        });
        if let Some(live) = gate.live {
            fields.push(StatusField {
                key: live_key,
                label: "Live base level",
                value: StatusValue::Integer(i64::from(live)),
            });
        }
    }
    fields.into()
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
        static MODES: std::sync::LazyLock<[Arc<str>; 4]> = std::sync::LazyLock::new(|| {
            [
                Arc::from("accurate"),
                Arc::from("aggressive"),
                Arc::from("defensive"),
                Arc::from("controlled"),
            ]
        });
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
        banks: Arc<api::named_banks::NamedBankFacts>,
    ) -> Self {
        Self {
            run,
            path,
            selected,
            quests,
            banks,
            choices: super::choices::QuestChoices::default(),
            stage: None,
            progress: None,
            journal: None,
            custom_reader: None,
            custom_read_after: None,
            pair_admission: None,
            pair_admission_since: None,
            pair_begin_since: None,
            pair_admitted: false,
            gang_reader: super::gang::GangRead::default(),
            bank: BankMemo::default(),
            last_read: None,
            journal_text: None,
            read_requested: false,
            selection_since: None,
            seq_index: 0,
            step_index: 0,
            step: None,
            step_after: api::quest_progress::EvidenceStamp {
                run,
                tick: 0,
                sequence: 0,
            },
            clear_prayers: None,
            prayer_cleanup_pending: false,
            prayer_cleanup_owned: RaisedPrayers::empty(),
            last_outcome: None,
            published_bank_receipt: None,
            last_combat: None,
            published_receipt: None,
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
            failed_acquisition_child: None,
            trace: RunTrace::default(),
            parked_step: None,
            last_error_kind: QuesterFailureKind::Other,
            waiting: None,
            deaths: 0,
            max_deaths: default_max_deaths(),
            prior_deaths: 0,
            attempts: 0,
            fail_streak: 0,
            parked: false,
            journal_opened: false,
            dirty: true,
            watchdog: Watchdog::default(),
            death: DeathLatch::default(),
            anchor: None,
            provisioner: Provisioner::new(),
            active_loadout: None,
            queue_fields: Arc::from([]),
            required_vs_live: Arc::from([]),
            tested_stats_warning: Arc::from(""),
        }
    }
    fn trace_start(&mut self, output: &mut dyn NativeOutput) {
        if self.trace.started {
            return;
        }
        self.trace.started = true;
        let stage = self
            .stage
            .as_ref()
            .map_or("unknown", |stage| stage.0.as_ref());
        self.trace.record(
            output,
            api::hostlog::Level::Info,
            format_args!("quester {}: run start stage={stage}", self.path.id.0),
        );
    }

    fn trace_step_event(&mut self, output: &mut dyn NativeOutput, event: StepTraceEvent) {
        let quest = &self.path.id.0;
        let stage = self
            .stage
            .as_ref()
            .map_or("unknown", |stage| stage.0.as_ref());
        match event {
            StepTraceEvent::Acquisition {
                recipe,
                child_step,
                outcome,
            } => match outcome {
                AcquisitionTraceOutcome::Begin => self.trace.record(
                    output,
                    api::hostlog::Level::Info,
                    format_args!(
                        "quester {quest}: stage {stage} recipe {recipe} child {child_step} begin"
                    ),
                ),
                AcquisitionTraceOutcome::Skipped(predicate) => self.trace.record(
                    output,
                    api::hostlog::Level::Info,
                    format_args!(
                        "quester {quest}: stage {stage} recipe {recipe} child {child_step} skipped: {predicate} evaluated true"
                    ),
                ),
                AcquisitionTraceOutcome::Settled => self.trace.record(
                    output,
                    api::hostlog::Level::Info,
                    format_args!(
                        "quester {quest}: stage {stage} recipe {recipe} child {child_step} settled"
                    ),
                ),
                AcquisitionTraceOutcome::Failed(reason) => self.trace.record(
                    output,
                    api::hostlog::Level::Warn,
                    format_args!(
                        "quester {quest}: stage {stage} recipe {recipe} child {child_step} failed: {reason}"
                    ),
                ),
            },
            StepTraceEvent::CombatSubOperationEnd { target, end } => self.trace.record(
                output,
                api::hostlog::Level::Info,
                format_args!(
                    "quester {quest}: stage {stage} combat sub-operation target={target:?} end={end:?}"
                ),
            ),
        }
    }

    fn trace_parked(&mut self, output: &mut dyn NativeOutput) {
        if self.trace.terminal_logged {
            return;
        }
        let root_step = self
            .parked_step
            .as_ref()
            .map(|step| Arc::clone(&step.id))
            .or_else(|| self.current_step().map(|step| Arc::clone(&step.id.0)));
        let active_child = self.step.as_ref().and_then(|run| {
            Some((
                Arc::clone(run.child_recipe_id()?),
                Arc::clone(&run.child_step_id()?.0),
            ))
        });
        let child = active_child.or_else(|| self.failed_acquisition_child.clone());
        let root_step = root_step.as_deref().unwrap_or("unknown");
        let (recipe, child_step) = child.as_ref().map_or(("", ""), |(recipe, child)| {
            (recipe.as_ref(), child.as_ref())
        });
        let reason = self.last_error.as_deref().unwrap_or(self.park_reason);
        self.trace.flush_repeats(output);
        self.trace.terminal(
            output,
            api::hostlog::Level::Warn,
            format_args!(
                "quester {}: park context step={root_step} child_recipe={recipe} child={child_step}",
                self.path.id.0
            ),
        );
        self.trace.terminal(
            output,
            api::hostlog::Level::Warn,
            format_args!("quester {}: park: {reason}", self.path.id.0),
        );
        self.trace.terminal_logged = true;
    }

    fn poll_provision(&mut self, tick: &mut NativeTick<'_>) -> bool {
        let required_after = tick.cx.evidence();
        let bank_phase = self.provisioner.bank_phase();
        let revision = self.provisioner.status_revision();
        let mut cx = StepContext {
            tick,
            quests: &self.quests,
            progress: self
                .progress
                .as_deref()
                .map(std::slice::from_ref)
                .unwrap_or(&[]),
            required_after,
            bank: &self.bank,
            banks: &self.banks,
            choices: &self.choices,
        };
        let result = self.provisioner.poll(
            &mut cx,
            &self.path.provisioning,
            self.active_loadout.as_deref(),
        );
        while let Some(event) = self.provisioner.take_trace_event() {
            self.trace_step_event(tick.output, event);
        }
        let bank_event = match (&result, bank_phase, self.provisioner.bank_phase()) {
            (Poll::Pending, None, Some(phase)) => Some((phase, "begin")),
            (Poll::Ready(Ok(ProvisionEvent::BankReceipt(_))), Some(phase), _) => {
                Some((phase, "settled"))
            }
            (Poll::Ready(Err(_)), Some(phase), _) => Some((phase, "failed")),
            _ => None,
        };
        if let Some((phase, event)) = bank_event {
            let action = match phase {
                ProvisionPhase::Scanning => "scan",
                ProvisionPhase::Spillover => "deposit-capacity",
                ProvisionPhase::Withdrawing => "withdraw",
                _ => "other",
            };
            self.trace.record(
                tick.output,
                api::hostlog::Level::Info,
                format_args!(
                    "quester {}: provision bank {action} {event}",
                    self.path.id.0
                ),
            );
        }
        self.dirty |= self.provisioner.status_revision() != revision;
        match result {
            Poll::Pending => {
                if let Some(outcome) = self.provisioner.in_flight_outcome() {
                    publish_in_flight_bank_receipt(
                        outcome,
                        &mut self.bank,
                        &mut self.published_bank_receipt,
                        &mut self.dirty,
                    );
                }
                if self.provisioner.needs_progress_read() {
                    self.needs_read = true;
                    self.dirty = true;
                }
                false
            }
            Poll::Ready(Ok(ProvisionEvent::Ready)) => true,
            Poll::Ready(Ok(ProvisionEvent::BankReceipt(receipt))) => {
                self.bank.update(&receipt);
                if let Some(step) = self.step.as_mut() {
                    step.bank_scan_completed();
                }
                self.dirty = true;
                false
            }
            Poll::Ready(Ok(ProvisionEvent::Acquired)) => {
                self.published_bank_receipt = None;
                // Inventory changes outside the bank do not change its stock.
                // Recipe bank operations already publish their own receipts.
                self.dirty = true;
                false
            }
            Poll::Ready(Ok(ProvisionEvent::Blocked { item })) => {
                self.parked = true;
                self.record_failure(ActionError::Blocked(item));
                false
            }
            Poll::Ready(Err(
                error @ (ActionError::Blocked(_)
                | ActionError::NeedsEvidence(_)
                | ActionError::UserInput),
            )) => {
                self.parked = true;
                self.record_failure(error);
                false
            }
            Poll::Ready(Err(error)) => {
                // Synthetic provisioning work uses the authored step failure
                // policy too; it must not turn one transient family refusal into
                // an immediate parked quest.
                self.provisioner.cancel();
                self.published_bank_receipt = None;
                self.record_step_failure(error, tick);
                false
            }
        }
    }

    fn start_predicate_bank_scan(&mut self, tick: &mut NativeTick<'_>) {
        let required_after = tick.cx.evidence();
        {
            let mut cx = StepContext {
                tick,
                quests: &self.quests,
                progress: self
                    .progress
                    .as_deref()
                    .map(std::slice::from_ref)
                    .unwrap_or(&[]),
                required_after,
                bank: &self.bank,
                banks: &self.banks,
                choices: &self.choices,
            };
            self.provisioner
                .start_predicate_scan(&mut cx, &self.path.provisioning);
        }
        self.dirty = true;
        self.trace.record(
            tick.output,
            api::hostlog::Level::Info,
            format_args!("quester {}: provision bank scan begin", self.path.id.0),
        );
    }

    fn finish_quest(&mut self, tick: &mut NativeTick<'_>) -> ScriptFlow {
        self.published_bank_receipt = None;
        self.trace.flush_repeats(tick.output);
        self.trace.terminal(
            tick.output,
            api::hostlog::Level::Info,
            format_args!("quester {}: finish", self.path.id.0),
        );
        self.emit_status(tick.output, NativePhase::Complete);
        ScriptFlow::Complete
    }

    pub fn journal_opened(&self) -> bool {
        self.journal_opened
    }

    pub fn deaths(&self) -> u16 {
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
            code: Arc::from(match self.last_error_kind {
                QuesterFailureKind::ManualMovement => "manual-movement",
                QuesterFailureKind::MaxDeaths => "max-deaths",
                QuesterFailureKind::NeedsEvidence => "needs-evidence",
                QuesterFailureKind::Other => "parked",
            }),
            message: self
                .last_error
                .clone()
                .unwrap_or_else(|| Arc::from(self.park_reason)),
        }
    }

    fn clear_last_error(&mut self) {
        self.last_error = None;
        self.last_error_kind = QuesterFailureKind::Other;
        self.failed_acquisition_child = None;
        self.parked_step = None;
    }

    fn set_last_error(&mut self, kind: QuesterFailureKind, message: Arc<str>) {
        self.last_error = Some(message);
        self.last_error_kind = kind;
        self.dirty = true;
    }
    fn capture_failed_acquisition_child(&mut self) {
        self.failed_acquisition_child = self.step.as_ref().and_then(|run| {
            Some((
                Arc::clone(run.child_recipe_id()?),
                Arc::clone(&run.child_step_id()?.0),
            ))
        });
    }

    fn record_failure(&mut self, error: ActionError) {
        let (kind, message) = match error {
            ActionError::NeedsEvidence(gates) => {
                let message = if gates.is_empty() {
                    match self.step.as_ref().and_then(|step| step.waiting_for()) {
                        Some((reason, name)) => {
                            Arc::<str>::from(format!("needs evidence: {reason}: {name}"))
                        }
                        None => Arc::from("needs evidence: no current wait detail"),
                    }
                } else {
                    Arc::from("walk needs authoritative quest-gate evidence")
                };
                self.set_last_error(QuesterFailureKind::NeedsEvidence, message);
                return;
            }
            ActionError::UserInput => (
                QuesterFailureKind::ManualMovement,
                super::families::manual_movement_message(),
            ),
            ActionError::Unavailable(reason)
            | ActionError::Failed(reason)
            | ActionError::Blocked(reason) => (QuesterFailureKind::Other, reason),
            error => (
                QuesterFailureKind::Other,
                Arc::from(format!("step error: {error:?}")),
            ),
        };
        self.set_last_error(kind, message);
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
        if self.parked {
            self.trace_parked(output);
        }
        let outcome = self
            .step
            .as_ref()
            .and_then(|step| step.in_flight_outcome())
            .or(self.last_outcome.as_ref());
        let combat_status = self.last_combat.as_ref().or(outcome);
        let receipt = combat_status.and_then(|outcome| outcome.receipt.as_ref());
        let unchanged = match (receipt, self.published_receipt.as_ref()) {
            (Some(current), Some(previous)) => Arc::ptr_eq(current, previous),
            (None, None) => true,
            _ => false,
        };
        if !unchanged {
            self.dirty = true;
        }
        if !self.dirty {
            return;
        }
        self.emit_status(
            output,
            if self.parked {
                NativePhase::Blocked
            } else {
                NativePhase::Working
            },
        );
    }

    fn emit_status(&mut self, output: &mut dyn NativeOutput, phase: NativePhase) {
        let phase = if !self.queue_fields.is_empty()
            && (phase == NativePhase::Complete
                || (phase == NativePhase::Blocked
                    && !matches!(
                        self.last_error_kind,
                        QuesterFailureKind::ManualMovement
                            | QuesterFailureKind::MaxDeaths
                            | QuesterFailureKind::NeedsEvidence
                    ))) {
            NativePhase::Working
        } else {
            phase
        };
        self.dirty = false;
        let outcome = self
            .step
            .as_ref()
            .and_then(|step| step.in_flight_outcome())
            .or(self.last_outcome.as_ref());
        let combat_status = self.last_combat.as_ref().or(outcome);
        let receipt = combat_status.and_then(|outcome| outcome.receipt.as_ref());
        self.published_receipt = receipt.map(Arc::clone);
        let stage = self
            .stage
            .as_ref()
            .map(|s| StatusValue::Text(Arc::from(s.0.as_ref())))
            .unwrap_or(StatusValue::Text(Arc::from("unknown")));
        let mut fields = vec![
            StatusField {
                key: "display",
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
                value: StatusValue::Integer(i64::from(self.prior_deaths) + i64::from(self.deaths)),
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
        let parked_step = if self.parked {
            self.parked_step.as_ref()
        } else {
            None
        };
        let display_sequence_index =
            parked_step.map_or(self.seq_index, |parked| parked.sequence_index);
        let display_step_index = parked_step.map_or(self.step_index, |parked| parked.step_index);
        let sequence = self.path.sequences.get(display_sequence_index);
        let current = parked_step
            .and_then(|parked| {
                if parked.in_prelude {
                    self.path.prelude.get(parked.step_index)
                } else {
                    self.path
                        .sequences
                        .get(parked.sequence_index)?
                        .steps
                        .get(parked.step_index)
                }
            })
            .or_else(|| self.current_step());
        let colour = if self.stage.as_ref() == Some(&self.path.colour_complete) {
            "complete"
        } else if self.stage.as_ref() == Some(&self.path.colour_not_started) {
            "not_started"
        } else if self.stage.is_some() {
            "in_progress"
        } else {
            "unknown"
        };
        use super::provision::ProvisionPhase;
        let provision = self.provisioner.status();
        let action = if self.parked {
            "blocked"
        } else if matches!(
            provision.phase,
            ProvisionPhase::Scanning | ProvisionPhase::Spillover | ProvisionPhase::Withdrawing
        ) {
            "banking"
        } else if self.waiting.is_some() || self.needs_read || self.settling {
            "waiting"
        } else {
            match current.map(|step| step.kind.as_ref()) {
                Some("walk") => "walking",
                Some("talk") => "talking",
                Some("combat") => "fighting",
                Some("bank" | "loadout") => "banking",
                _ => "working",
            }
        };
        for (key, label, text) in [
            ("quest_id", "Quest ID", self.path.id.0.as_ref()),
            ("colour", "Quest colour", colour),
            ("action_state", "Action", action),
        ] {
            fields.push(StatusField {
                key,
                label,
                value: StatusValue::Text(Arc::from(text)),
            });
        }
        fields.push(StatusField {
            key: "step_id",
            label: "Step",
            value: StatusValue::Text(parked_step.map_or_else(
                || current.map_or_else(|| Arc::from(""), |step| Arc::clone(&step.id.0)),
                |parked| Arc::clone(&parked.id),
            )),
        });
        let active_child = self.step.as_ref().and_then(|run| {
            Some((
                Arc::clone(run.child_recipe_id()?),
                Arc::clone(&run.child_step_id()?.0),
            ))
        });
        let child = active_child.or_else(|| self.failed_acquisition_child.clone());
        static EMPTY_CHILD: std::sync::LazyLock<Arc<str>> =
            std::sync::LazyLock::new(|| Arc::from(""));
        fields.push(StatusField {
            key: "child_recipe_id",
            label: "Acquisition recipe",
            value: StatusValue::Text(child.as_ref().map_or_else(
                || Arc::clone(&EMPTY_CHILD),
                |(recipe, _)| Arc::clone(recipe),
            )),
        });
        fields.push(StatusField {
            key: "child_step_id",
            label: "Acquisition child",
            value: StatusValue::Text(
                child
                    .as_ref()
                    .map_or_else(|| Arc::clone(&EMPTY_CHILD), |(_, step)| Arc::clone(step)),
            ),
        });
        let remaining_steps = parked_step.filter(|parked| parked.in_prelude).map_or_else(
            || sequence.map_or(0, |seq| seq.steps.len().saturating_sub(display_step_index)),
            |_| self.path.prelude.len().saturating_sub(display_step_index),
        );
        for (key, label, value) in [
            ("sequence", "Sequence", display_sequence_index as i64),
            (
                "sequence_count",
                "Sequence count",
                self.path.sequences.len() as i64,
            ),
            ("step_index", "Step index", display_step_index as i64),
            ("remaining_steps", "Remaining steps", remaining_steps as i64),
            ("attempts", "Attempts", i64::from(self.attempts)),
            (
                "no_progress",
                "Unchanged steps",
                i64::from(self.watchdog.unchanged()),
            ),
        ] {
            fields.push(StatusField {
                key,
                label,
                value: StatusValue::Integer(value),
            });
        }
        fields.extend_from_slice(&self.queue_fields);
        fields.extend_from_slice(&self.required_vs_live);
        fields.push(StatusField {
            key: "tested_stats_warning",
            label: "Tested stats warning",
            value: StatusValue::Text(Arc::clone(&self.tested_stats_warning)),
        });
        fields.push(StatusField {
            key: "pin",
            label: "Selected pin",
            value: StatusValue::Text(self.selected.selected_pin().map_or_else(
                |_| Arc::from("unavailable"),
                |pin| {
                    Arc::from(format!(
                        "r{} engine:{} content:{} nav:{:02x?}",
                        pin.revision.as_i32(),
                        pin.engine_commit,
                        pin.content_commit,
                        pin.nav_sha256
                    ))
                },
            )),
        });
        fields.push(StatusField {
            key: "tactic",
            label: "Combat tactic",
            value: StatusValue::Text(
                current
                    .and_then(|step| step.tactic.clone())
                    .unwrap_or_else(|| Arc::from("")),
            ),
        });
        fields.push(StatusField {
            key: "role",
            label: "Role",
            value: StatusValue::Text(
                self.path
                    .role
                    .as_ref()
                    .map_or_else(|| Arc::from(""), |role| Arc::clone(&role.0)),
            ),
        });
        fields.push(StatusField {
            key: "step_comment",
            label: "Step comment",
            value: StatusValue::Text(
                current
                    .and_then(|step| step.comment.clone())
                    .unwrap_or_else(|| Arc::from("")),
            ),
        });
        fields.extend([
            StatusField {
                key: "provision",
                label: "Provisioning",
                value: StatusValue::Text(format!("{:?}", provision.phase).into()),
            },
            StatusField {
                key: "provision_item",
                label: "Provision item",
                value: StatusValue::Text(Arc::from(provision.item.unwrap_or(""))),
            },
            StatusField {
                key: "provision_need",
                label: "Needed",
                value: StatusValue::Integer(i64::from(provision.need)),
            },
            StatusField {
                key: "provision_pack",
                label: "In pack",
                value: StatusValue::Integer(i64::from(provision.pack)),
            },
            StatusField {
                key: "bank_known",
                label: "Bank observed",
                value: StatusValue::Truth(if provision.bank_known {
                    Truth::True
                } else {
                    Truth::Unknown
                }),
            },
            StatusField {
                key: "provision_attempts",
                label: "Provision attempts",
                value: StatusValue::Integer(i64::from(provision.attempts)),
            },
            StatusField {
                key: "active_loadout",
                label: "Active loadout",
                value: StatusValue::Text(
                    self.active_loadout.clone().unwrap_or_else(|| Arc::from("")),
                ),
            },
        ]);
        fields.extend([
            StatusField {
                key: "coin_float",
                label: "Coin float",
                value: StatusValue::Integer(i64::from(self.path.provisioning.coin_float)),
            },
            StatusField {
                key: "coin_drawn",
                label: "Coin float drawn",
                value: StatusValue::Truth(if self.provisioner.coin_drawn() {
                    Truth::True
                } else {
                    Truth::False
                }),
            },
            StatusField {
                key: "carry_drawn",
                label: "Carry drawn row bits",
                value: StatusValue::Text(format!("{:016x}", self.provisioner.carry_drawn()).into()),
            },
        ]);
        if let Some(bank) = provision.bank {
            fields.push(StatusField {
                key: "provision_bank",
                label: "In bank",
                value: StatusValue::Integer(i64::from(bank)),
            });
        }
        append_combat_status(&mut fields, combat_status);
        if let Some(progress) = &self.progress {
            fields.push(StatusField {
                key: "quest",
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
        if let Some(text) = self.journal_text.take() {
            fields.push(StatusField {
                key: "journal_lines",
                label: "Journal",
                value: StatusValue::Text(text),
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
        if self.parked {
            fields.push(StatusField {
                key: "block_reason",
                label: "Blocked",
                value: StatusValue::Text(self.blocked_failure().message),
            });
        }
        output.status(ScriptStatus {
            run: self.run,
            card: CompiledId("Quester"),
            phase,
            active_settings: 1,
            pending_settings: None,
            fields: fields.into(),
            failure: (phase == NativePhase::Blocked).then(|| self.blocked_failure()),
        });
    }

    fn capture_prayer_cleanup(&mut self) {
        let raised = self
            .step
            .as_ref()
            .map_or_else(RaisedPrayers::empty, |step| step.prayer_cleanup());
        self.prayer_cleanup_owned.merge(raised);
        self.prayer_cleanup_pending = !self.prayer_cleanup_owned.is_empty();
    }

    fn cancel_step(&mut self, tick: &mut NativeTick<'_>) {
        self.capture_prayer_cleanup();
        if let Some(mut step) = self.step.take() {
            step.cancel(tick.actions);
        }
        self.provisioner.cancel();
        self.clear_prayers = None;
        self.last_outcome = None;
        self.published_bank_receipt = None;
        self.journal = None;
        self.custom_reader = None;
        self.custom_read_after = None;
        self.pair_admission = None;
        self.pair_begin_since = None;
        self.gang_reader.cancel();
        self.advances = false;
        self.attempts = 0;
        self.settling = false;
        self.settle_deadline = Duration::ZERO;
        if self.waiting.take().is_some() {
            self.dirty = true;
        }
        self.clear_last_error();
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
                self.clear_last_error();
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
                    self.set_last_error(
                        QuesterFailureKind::Other,
                        Arc::from(format!(
                            "journal blocked by modal root {}{}",
                            chat.value.root,
                            chat.value
                                .texts
                                .iter()
                                .find(|text| !text.is_empty())
                                .map(|text| format!(" ({text})"))
                                .unwrap_or_default()
                        )),
                    );
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
            self.set_last_error(QuesterFailureKind::Other, Arc::from(limit));
            return false;
        }
        self.wait_for_read(tick, reason)
    }

    fn read_stage(&mut self, tick: &mut NativeTick<'_>, retarget: bool) -> bool {
        if self.path.progress_reader.is_some() {
            return self.read_custom_stage(tick, retarget);
        }
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
                    if super::pair::PairQuest::from_path(self.path.id.0.as_ref())
                        == Some(super::pair::PairQuest::Arrav)
                    {
                        if let Some(port) = tick.pairs {
                            if let Err(error) = port.observe_gang(&read) {
                                self.record_failure(error.action());
                                self.parked = true;
                                return false;
                            }
                        }
                    }
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
        self.adopt_progress(tick, Arc::new(progress), retarget)
    }

    fn adopt_progress(
        &mut self,
        tick: &mut NativeTick<'_>,
        progress: Arc<QuestProgress>,
        retarget: bool,
    ) -> bool {
        let stage = match &progress.stage {
            Knowledge::Known(stage) => Some(stage.clone()),
            Knowledge::Unknown(_) | Knowledge::Partial { .. } => None,
        };
        self.journal_attempts = 0;
        self.journal_retry_pending = false;
        self.journal_quiet_since = None;
        self.progress = Some(progress);
        self.dirty = true;
        let sequence = stage
            .as_ref()
            .and_then(|stage| sequence_for_stage(&self.path, &stage.0));
        let Some((stage, sequence)) = stage.zip(sequence) else {
            self.stage = None;
            if self.last_read.is_some() {
                self.parked = true;
                self.park_reason = "no journal rule matched or stage has no sequence";
                self.clear_last_error();
                return false;
            }
            return self.wait_for_read(tick, "quest colour unavailable or unknown stage");
        };
        self.unreadable_since = None;
        self.unreadable_reads = 0;
        self.selection_since = None;
        if self.stage.as_ref() != Some(&stage) {
            let previous = self
                .stage
                .as_ref()
                .map_or("unknown", |previous| previous.0.as_ref());
            self.trace.record(
                tick.output,
                api::hostlog::Level::Info,
                format_args!("quester {}: stage {previous} → {}", self.path.id.0, stage.0),
            );
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

    fn valid_progress(
        &self,
        tick: &NativeTick<'_>,
        progress: &QuestProgress,
        after: api::quest_progress::EvidenceStamp,
    ) -> bool {
        progress.quest == self.path.id
            && progress.binding == self.path.progress.binding
            && progress.role == self.path.role
            && progress.pin.as_ref() == tick.cx.pin()
            && progress.evidence.meets(after)
            && progress.evidence != after
            && tick.cx.evidence().meets(progress.evidence)
            && match &progress.stage {
                Knowledge::Known(stage) => {
                    self.path.progress.stage_keys.contains(stage)
                        && (progress.complete != Truth::True
                            || stage == &self.path.progress.colour_complete)
                }
                Knowledge::Unknown(_) | Knowledge::Partial { .. } => {
                    progress.complete != Truth::True
                }
            }
            && progress.flags.iter().all(|flag| {
                self.path
                    .progress
                    .flags
                    .iter()
                    .any(|rule| rule.flag == flag.flag)
            })
    }

    fn read_custom_stage(&mut self, tick: &mut NativeTick<'_>, retarget: bool) -> bool {
        if self.custom_reader.is_none() {
            let result = {
                let after = tick.cx.evidence();
                let mut cx = StepContext {
                    tick,
                    quests: &self.quests,
                    progress: self.progress_slice(),
                    required_after: after,
                    bank: &self.bank,
                    banks: &self.banks,
                    choices: &self.choices,
                };
                self.path
                    .progress_reader
                    .as_ref()
                    .unwrap()
                    .plan
                    .begin(&mut cx)
            };
            match result {
                Ok(reader) => {
                    self.custom_reader = Some(reader);
                    self.custom_read_after = Some(tick.cx.evidence());
                }
                Err(ActionError::Busy | ActionError::Held | ActionError::BudgetExhausted) => {
                    return false
                }
                Err(error) => {
                    self.record_failure(error);
                    self.parked = true;
                    return false;
                }
            }
        }
        let mut reader = self.custom_reader.take().unwrap();
        let result = {
            let after = self.custom_read_after.unwrap();
            let mut cx = StepContext {
                tick,
                quests: &self.quests,
                progress: self.progress_slice(),
                required_after: after,
                bank: &self.bank,
                banks: &self.banks,
                choices: &self.choices,
            };
            reader.poll(&mut cx)
        };
        match result {
            Poll::Pending => {
                self.custom_reader = Some(reader);
                false
            }
            Poll::Ready(Err(error)) => {
                self.record_failure(error);
                self.parked = true;
                false
            }
            Poll::Ready(Ok(outcome)) => {
                let after = self.custom_read_after.take().unwrap();
                let Some(progress) = outcome.progress else {
                    self.record_failure(ActionError::Blocked(Arc::from(
                        "progress reader returned no owned progress",
                    )));
                    self.parked = true;
                    return false;
                };
                if outcome.evidence != progress.evidence
                    || !self.valid_progress(tick, &progress, after)
                {
                    self.record_failure(ActionError::Stale);
                    self.parked = true;
                    return false;
                }
                self.adopt_progress(tick, progress, retarget)
            }
        }
    }

    fn pair_step_active(&self) -> bool {
        self.step.is_some()
            && self
                .current_step()
                .is_some_and(|step| step.kind.as_ref() == "partner")
    }

    fn pair_work_pending(&self) -> bool {
        self.pair_admission_since.is_some()
            || self.pair_admission.is_some()
            || self.pair_begin_since.is_some()
            || self.pair_step_active()
    }

    fn wait_for_pair_admission(&mut self, now: Duration) -> bool {
        let _ = self.pair_admission_since.get_or_insert(now);
        self.waiting = Some(("Partner admission", Arc::clone(&self.path.id.0)));
        self.dirty = true;
        false
    }

    fn fail_pair_admission(&mut self, error: ActionError) -> bool {
        self.pair_admission = None;
        self.pair_admission_since = None;
        self.waiting = None;
        self.record_failure(error);
        self.parked = true;
        false
    }

    fn admit_pair(&mut self, tick: &mut NativeTick<'_>) -> bool {
        if self.path.partner.is_none() {
            return true;
        }
        if self.pair_admitted {
            self.pair_admission_since = None;
            return true;
        }

        let now = tick.cx.active_now();
        if self
            .pair_admission_since
            .is_some_and(|since| now.saturating_sub(since) >= PAIR_ADMISSION_TIMEOUT)
        {
            return self.fail_pair_admission(ActionError::Blocked(Arc::from(
                "partner admission timed out; Stop and Start both accounts",
            )));
        }

        let result = tick
            .pairs
            .ok_or_else(|| {
                ActionError::Unavailable(Arc::from(
                    "partner capability is not installed in this Play",
                ))
            })
            .and_then(|port| {
                port.settings(tick.cx.run())
                    .map_err(super::pair::PairError::action)
            });
        match result {
            Ok(_) => {}
            Err(ActionError::Busy | ActionError::Held | ActionError::BudgetExhausted) => {
                return self.wait_for_pair_admission(now);
            }
            Err(error) => return self.fail_pair_admission(error),
        }

        if tick
            .pairs
            .is_some_and(|port| port.gang(tick.cx.run()).is_err())
        {
            match self.gang_reader.poll(tick, &self.quests) {
                Poll::Pending => return false,
                Poll::Ready(Ok(_)) => {}
                Poll::Ready(Err(error)) => return self.fail_pair_admission(error),
            }
        }
        if self.pair_admission.is_none() {
            let _ = self.pair_admission_since.get_or_insert(now);
            let result = super::families::partner::admission(&self.path).and_then(|plan| {
                let after = tick.cx.evidence();
                let mut cx = StepContext {
                    tick,
                    quests: &self.quests,
                    progress: self.progress_slice(),
                    required_after: after,
                    bank: &self.bank,
                    banks: &self.banks,
                    choices: &self.choices,
                };
                plan.begin(&mut cx)
            });
            match result {
                Ok(run) => self.pair_admission = Some(run),
                Err(ActionError::Busy | ActionError::Held | ActionError::BudgetExhausted) => {
                    return self.wait_for_pair_admission(now);
                }
                Err(error) => return self.fail_pair_admission(error),
            }
        }

        let mut run = self.pair_admission.take().unwrap();
        let result = {
            let after = tick.cx.evidence();
            let mut cx = StepContext {
                tick,
                quests: &self.quests,
                progress: self.progress_slice(),
                required_after: after,
                bank: &self.bank,
                banks: &self.banks,
                choices: &self.choices,
            };
            run.poll(&mut cx)
        };
        match result {
            Poll::Pending => {
                self.pair_admission = Some(run);
                self.wait_for_pair_admission(now)
            }
            Poll::Ready(Ok(_)) => {
                self.pair_admitted = true;
                self.pair_admission_since = None;
                self.waiting = None;
                self.dirty = true;
                true
            }
            Poll::Ready(Err(error)) => self.fail_pair_admission(error),
        }
    }

    fn record_step_failure(&mut self, error: ActionError, tick: &NativeTick<'_>) {
        self.fail_streak = self.fail_streak.saturating_add(1);
        self.record_failure(error);
        if self.fail_streak >= 5 {
            self.parked = true;
        }
        self.on_step_boundary(tick);
    }

    fn on_step_boundary(&mut self, tick: &NativeTick<'_>) {
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
        if tile.is_some() {
            self.anchor = tile;
        }
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
    fn pair_binding(&self) -> Option<super::pair::PairBinding<'_>> {
        let declaration = self.path.partner.as_ref()?;
        Some(super::pair::PairBinding {
            path: &self.path.id,
            protocol: &declaration.protocol,
            digest: &self.path.digest,
            role: self.path.role.as_ref()?,
        })
    }
    fn tick(&mut self, tick: &mut NativeTick<'_>) -> Result<ScriptFlow, ScriptFailure> {
        if tick.cx.run() != self.run {
            self.run = tick.cx.run();
            self.trace = RunTrace::default();
            self.watchdog = Watchdog::default();
            self.cancel_step(tick);
            self.pair_admission = None;
            self.pair_admission_since = None;
            self.waiting = None;
            self.pair_admitted = false;
            self.last_combat = None;
            self.needs_read = true;
            self.progress = None;
        }
        self.trace_start(tick.output);
        if !tick.cx.eligible {
            self.watchdog = Watchdog::default();
            self.publish(tick.output);
            return Ok(ScriptFlow::Continue);
        }
        if self.prayer_cleanup_pending && self.step.is_some() {
            self.cancel_step(tick);
            self.dirty = true;
        }
        if self.anchor.is_none() {
            self.anchor = tick.cx.snapshot().here().map(|here| here.value);
        }
        let died = self.death.observe(tick.cx.snapshot());
        {
            let retained = tick.cx.retained().quester();
            retained.anchor = self.anchor;
            retained.death_seq = self.death.watermark();
        }
        if died {
            let exceeded = death_cap_exceeded(
                self.prior_deaths.saturating_add(self.deaths),
                self.max_deaths,
            );
            self.watchdog = Watchdog::default();
            self.cancel_step(tick);
            self.prayer_cleanup_owned = RaisedPrayers::empty();
            self.prayer_cleanup_pending = false;
            self.last_combat = None;
            self.deaths = self.deaths.saturating_add(1);
            tick.cx.retained().quester().deaths = self.prior_deaths.saturating_add(self.deaths);
            self.provisioner.reset(tick.actions);
            self.active_loadout = None;
            self.needs_read = true;
            self.progress = None;
            self.dirty = true;
            if exceeded {
                self.parked = true;
                self.set_last_error(
                    QuesterFailureKind::MaxDeaths,
                    Arc::from("maximum deaths exceeded; Stop/Start required"),
                );
                self.trace_parked(tick.output);
                self.emit_status(tick.output, NativePhase::Blocked);
                return Ok(ScriptFlow::Blocked(self.blocked_failure()));
            }
        }
        if !self.parked
            && crate::native::death::hitpoints_zero(
                tick.cx.snapshot().stats().map(|stats| stats.value),
            )
        {
            // Damage precedes the content's death message. Do not poll a live
            // dialogue/step into a combat failure while that message is owed.
            self.watchdog = Watchdog::default();
            self.publish(tick.output);
            return Ok(ScriptFlow::Continue);
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
                        self.prayer_cleanup_pending = !self.prayer_cleanup_owned.is_empty();
                        self.parked = true;
                        self.record_failure(ActionError::Blocked(Arc::from(
                            "prayer cleanup timed out",
                        )));
                        self.publish(tick.output);
                        return Ok(ScriptFlow::Blocked(self.blocked_failure()));
                    }
                    self.prayer_cleanup_owned = RaisedPrayers::empty();
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
                    self.prayer_cleanup_pending = !self.prayer_cleanup_owned.is_empty();
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
            match begin_clear_owned_prayers(&self.selected, self.prayer_cleanup_owned, tick) {
                Hygiene::Clean => {
                    self.prayer_cleanup_owned = RaisedPrayers::empty();
                    self.prayer_cleanup_pending = false;
                }
                Hygiene::Started(handle) => {
                    self.clear_prayers = Some(handle);
                    self.publish(tick.output);
                    return Ok(ScriptFlow::Continue);
                }
                Hygiene::Deferred => {
                    self.publish(tick.output);
                    return Ok(ScriptFlow::Continue);
                }
                Hygiene::Failed(error) => {
                    self.parked = true;
                    self.record_failure(error);
                    self.publish(tick.output);
                    return Ok(ScriptFlow::Blocked(self.blocked_failure()));
                }
            }
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
            let retarget =
                !self.settling && self.step.is_none() && !self.provisioner.needs_progress_read();
            if !self.read_stage(tick, retarget) {
                self.publish(tick.output);
                return Ok(ScriptFlow::Continue);
            }
            if let Some(step) = self.step.as_mut() {
                step.progress_read_completed(tick.cx.active_now());
            }
            self.provisioner
                .progress_read_completed(tick.cx.active_now());
        }
        if self
            .path
            .sequences
            .get(self.seq_index)
            .is_some_and(|seq| seq.terminal && seq.steps.is_empty())
        {
            return Ok(self.finish_quest(tick));
        }
        if !self.admit_pair(tick) {
            self.publish(tick.output);
            return Ok(ScriptFlow::Continue);
        }
        if self.settling {
            let truth = {
                let pred = PredicateContext {
                    cx: &tick.cx,
                    pairs: tick.pairs,
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
                let step = if self.in_prelude {
                    self.path.prelude.get(self.step_index)
                } else {
                    self.path
                        .sequences
                        .get(self.seq_index)
                        .and_then(|sequence| sequence.steps.get(self.step_index))
                };
                if let Some(step) = step {
                    let stage = self
                        .stage
                        .as_ref()
                        .map_or("unknown", |stage| stage.0.as_ref());
                    trace_root_event(
                        &mut self.trace,
                        tick.output,
                        api::hostlog::Level::Info,
                        self.path.id.0.as_ref(),
                        stage,
                        step,
                        format_args!("settled"),
                    );
                }
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
                    let error = ActionError::Failed(Arc::from("step settle timeout"));
                    let step = if self.in_prelude {
                        self.path.prelude.get(self.step_index)
                    } else {
                        self.path
                            .sequences
                            .get(self.seq_index)
                            .and_then(|sequence| sequence.steps.get(self.step_index))
                    };
                    let failed_step = step.map(|step| ParkedStep {
                        sequence_index: self.seq_index,
                        step_index: self.step_index,
                        in_prelude: self.in_prelude,
                        id: Arc::clone(&step.id.0),
                    });
                    if let Some(step) = step {
                        let stage = self
                            .stage
                            .as_ref()
                            .map_or("unknown", |stage| stage.0.as_ref());
                        trace_root_event(
                            &mut self.trace,
                            tick.output,
                            api::hostlog::Level::Warn,
                            self.path.id.0.as_ref(),
                            stage,
                            step,
                            format_args!("failed: step settle timeout"),
                        );
                    }
                    self.record_failure(error);
                    if self.fail_streak >= 5 {
                        self.parked = true;
                    }
                    self.on_step_boundary(tick);
                    if self.parked {
                        self.parked_step = failed_step;
                    }
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
            if !self.poll_provision(tick) {
                self.publish(tick.output);
                return Ok(ScriptFlow::Continue);
            }
            let selected = {
                let path = &self.path;
                let seq_index = self.seq_index;
                let quests = &self.quests;
                let progress = self
                    .progress
                    .as_deref()
                    .map(std::slice::from_ref)
                    .unwrap_or(&[]);
                let outcome = self.last_combat.as_ref().or(self.last_outcome.as_ref());
                let bank = &self.bank;
                let stage = self
                    .stage
                    .as_ref()
                    .map_or("unknown", |stage| stage.0.as_ref());
                let trace = &mut self.trace;
                let output = &mut *tick.output;
                let pred = PredicateContext {
                    cx: &tick.cx,
                    pairs: tick.pairs,
                    quests,
                    progress,
                    required_after: tick.cx.evidence(),
                    chat_since: super::families::reach::last_chat_seq(&tick.cx),
                    outcome,
                    bank,
                };
                match select_with_skips(path, seq_index, &pred, |step| {
                    trace.record(
                        output,
                        api::hostlog::Level::Info,
                        format_args!(
                            "quester {}: stage {stage} step {} skipped: {} evaluated true",
                            path.id.0, step.id.0, step.skip_if_summary
                        ),
                    );
                }) {
                    SelectionDecision::Selected(sel) => {
                        Ok(Some((sel.index, sel.step.advances, sel.prelude)))
                    }
                    SelectionDecision::Exhausted => Ok(None),
                    SelectionDecision::Unknown(sel) => Err((
                        sel.index,
                        sel.prelude,
                        Arc::clone(&sel.step.id.0),
                        Arc::clone(&sel.step.skip_if_summary),
                        sel.step.skip_if.requires_bank(),
                    )),
                }
            };
            let selected = match selected {
                Ok(selected) => selected,
                Err((index, prelude, id, predicate, requires_bank)) => {
                    let stage = self
                        .stage
                        .as_ref()
                        .map_or("unknown", |stage| stage.0.as_ref());
                    self.trace.record(
                        tick.output,
                        api::hostlog::Level::Info,
                        format_args!(
                            "quester {}: stage {stage} step {id} skip predicate waiting: {predicate}",
                            self.path.id.0
                        ),
                    );
                    if requires_bank && !self.bank.known() {
                        self.start_predicate_bank_scan(tick);
                        self.selection_since = None;
                        self.publish(tick.output);
                        return Ok(ScriptFlow::Continue);
                    }
                    let since = self.selection_since.get_or_insert(tick.cx.active_now());
                    if tick.cx.active_now().saturating_sub(*since) >= Duration::from_secs(16) {
                        self.parked = true;
                        self.park_reason = "skip predicate evidence unavailable";
                        self.clear_last_error();
                        self.parked_step = Some(ParkedStep {
                            sequence_index: self.seq_index,
                            step_index: index,
                            in_prelude: prelude,
                            id,
                        });
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
                if self
                    .path
                    .sequences
                    .get(self.seq_index)
                    .is_some_and(|seq| seq.terminal)
                {
                    return Ok(self.finish_quest(tick));
                }
                self.empty_reads += 1;
                if self.empty_reads >= 2 {
                    self.parked = true;
                    self.park_reason = "no step for stage";
                    self.clear_last_error();
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
            self.failed_acquisition_child = None;
            self.parked_step = None;
            if self.pair_begin_since.is_some_and(|since| {
                tick.cx.active_now().saturating_sub(since) >= PAIR_ADMISSION_TIMEOUT
            }) {
                if let Some(port) = tick.pairs {
                    port.invalidate(tick.cx.run());
                }
                self.pair_begin_since = None;
                self.waiting = None;
                self.record_failure(ActionError::Blocked(Arc::from(
                    "partner phase begin timed out after 10 minutes of active time; Stop and Start both accounts",
                )));
                self.parked = true;
                self.publish(tick.output);
                return Ok(ScriptFlow::Continue);
            }
            let step = if prelude {
                &self.path.prelude[index]
            } else {
                &self.path.sequences[self.seq_index].steps[index]
            };
            let pair_step = step.kind.as_ref() == "partner";
            self.chat_since = super::families::reach::last_chat_seq(&tick.cx);
            let required_after = tick.cx.evidence();
            if step.kind.as_ref() == "combat" {
                self.last_combat = None;
            }
            let stage = self
                .stage
                .as_ref()
                .map_or("unknown", |stage| stage.0.as_ref());
            trace_root_event(
                &mut self.trace,
                tick.output,
                api::hostlog::Level::Info,
                self.path.id.0.as_ref(),
                stage,
                step,
                format_args!("begin"),
            );
            let result = {
                let mut step_cx = StepContext {
                    tick,
                    quests: &self.quests,
                    progress: self.progress_slice(),
                    required_after,
                    bank: &self.bank,
                    banks: &self.banks,
                    choices: &self.choices,
                };
                step.plan.begin(&mut step_cx)
            };
            match result {
                Ok(run) => {
                    self.pair_begin_since = None;
                    self.waiting = None;
                    self.step = Some(run);
                    self.step_after = required_after;
                    self.last_outcome = None;
                    self.dirty = true;
                    self.clear_last_error();
                }
                Err(ActionError::Busy | ActionError::Held | ActionError::BudgetExhausted)
                    if pair_step =>
                {
                    self.pair_begin_since.get_or_insert(tick.cx.active_now());
                    self.waiting = Some(("Partner phase begin", Arc::clone(&self.path.id.0)));
                    self.clear_last_error();
                    self.dirty = true;
                }
                Err(error) => {
                    let reason = trace_error_reason(&error);
                    self.trace.record(
                        tick.output,
                        api::hostlog::Level::Warn,
                        format_args!(
                            "quester {}: stage {stage} step {} ({}) begin failed: {reason}",
                            self.path.id.0, step.id.0, step.kind
                        ),
                    );
                    self.pair_begin_since = None;
                    self.waiting = None;
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
        let needs_bank_scan = self
            .step
            .as_ref()
            .is_some_and(|step| step.needs_bank_scan());
        let bank_scan_active = self.provisioner.bank_phase().is_some();
        let bank_scan_status =
            self.step.is_some() && self.provisioner.status().phase == ProvisionPhase::Scanning;
        if (needs_bank_scan && !self.bank.known()) || bank_scan_active || bank_scan_status {
            if needs_bank_scan && !self.bank.known() && !bank_scan_active {
                self.start_predicate_bank_scan(tick);
            }
            if !self.poll_provision(tick) {
                self.publish(tick.output);
                return Ok(ScriptFlow::Continue);
            }
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
                banks: &self.banks,
                choices: &self.choices,
            };
            self.step
                .as_mut()
                .map(|step| step.poll(&mut step_cx))
                .unwrap_or(Poll::Pending)
        };
        loop {
            let event = self.step.as_mut().and_then(|step| step.take_trace_event());
            let Some(event) = event else {
                break;
            };
            self.trace_step_event(tick.output, event);
        }
        match poll {
            Poll::Pending => {
                if let Some(outcome) = self.step.as_ref().and_then(|step| step.in_flight_outcome())
                {
                    let carries_bank_receipt = publish_in_flight_bank_receipt(
                        outcome,
                        &mut self.bank,
                        &mut self.published_bank_receipt,
                        &mut self.dirty,
                    );
                    if carries_bank_receipt {
                        self.last_outcome = Some(StepOutcome {
                            progress: outcome.progress.clone(),
                            evidence: outcome.evidence,
                            receipt: outcome.receipt.clone(),
                        });
                    }
                }
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
                if let Some(progress) = &outcome.progress {
                    if outcome.evidence != progress.evidence
                        || !self.valid_progress(tick, progress, self.step_after)
                    {
                        self.record_step_failure(ActionError::Stale, tick);
                        self.step = None;
                        return Ok(ScriptFlow::Continue);
                    }
                    self.adopt_progress(tick, Arc::clone(progress), false);
                }
                if let Some(loadout) = self.current_step().and_then(|step| step.loadout.clone()) {
                    self.active_loadout = Some(loadout);
                }
                if let Some(receipt) = outcome.receipt.as_deref().and_then(|receipt| {
                    receipt
                        .as_any()
                        .downcast_ref::<crate::native_bank::BankReceipt>()
                }) {
                    self.bank.update(receipt);
                }
                if outcome.receipt.as_ref().is_some_and(|receipt| {
                    receipt.as_any().downcast_ref::<CombatReceipt>().is_some()
                }) {
                    self.last_combat = Some(StepOutcome {
                        progress: outcome.progress.clone(),
                        evidence: outcome.evidence,
                        receipt: outcome.receipt.clone(),
                    });
                }
                self.last_outcome = Some(outcome);
                self.capture_prayer_cleanup();
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
            Poll::Ready(Err(
                error @ (ActionError::Blocked(_)
                | ActionError::NeedsEvidence(_)
                | ActionError::UserInput),
            )) => {
                if let Some(outcome) = self
                    .step
                    .as_ref()
                    .and_then(|step| step.in_flight_outcome())
                    .filter(|outcome| {
                        outcome.receipt.as_ref().is_some_and(|receipt| {
                            receipt.as_any().downcast_ref::<CombatReceipt>().is_some()
                        })
                    })
                {
                    self.last_combat = Some(StepOutcome {
                        progress: outcome.progress.clone(),
                        evidence: outcome.evidence,
                        receipt: outcome.receipt.clone(),
                    });
                }
                let root_step = if self.in_prelude {
                    self.path.prelude.get(self.step_index)
                } else {
                    self.path
                        .sequences
                        .get(self.seq_index)
                        .and_then(|sequence| sequence.steps.get(self.step_index))
                };
                if let Some(step) = root_step {
                    let stage = self
                        .stage
                        .as_ref()
                        .map_or("unknown", |stage| stage.0.as_ref());
                    trace_root_event(
                        &mut self.trace,
                        tick.output,
                        api::hostlog::Level::Warn,
                        self.path.id.0.as_ref(),
                        stage,
                        step,
                        format_args!("failed: {}", trace_error_reason(&error)),
                    );
                }
                self.capture_prayer_cleanup();
                self.capture_failed_acquisition_child();
                self.record_failure(error);
                self.step = None;
                self.last_outcome = None;
                self.prayer_cleanup_pending = !self.prayer_cleanup_owned.is_empty();
                self.parked = true;
                self.update_wait();
                self.publish(tick.output);
                return Ok(ScriptFlow::Blocked(self.blocked_failure()));
            }
            Poll::Ready(Err(error)) => {
                let root_step = if self.in_prelude {
                    self.path.prelude.get(self.step_index)
                } else {
                    self.path
                        .sequences
                        .get(self.seq_index)
                        .and_then(|sequence| sequence.steps.get(self.step_index))
                };
                if let Some(step) = root_step {
                    let stage = self
                        .stage
                        .as_ref()
                        .map_or("unknown", |stage| stage.0.as_ref());
                    trace_root_event(
                        &mut self.trace,
                        tick.output,
                        api::hostlog::Level::Warn,
                        self.path.id.0.as_ref(),
                        stage,
                        step,
                        format_args!("failed: {}", trace_error_reason(&error)),
                    );
                }
                self.capture_prayer_cleanup();
                self.capture_failed_acquisition_child();
                self.step = None;
                self.last_outcome = None;
                self.record_step_failure(error, tick);
            }
        }
        self.update_wait();
        self.publish(tick.output);
        Ok(ScriptFlow::Continue)
    }

    fn interrupt(&mut self, event: Interrupt) {
        self.custom_reader = None;
        self.custom_read_after = None;
        let cancel_pair_work = matches!(event, Interrupt::Pause | Interrupt::SessionEnded);
        let pair_work_pending = self.pair_work_pending();
        if !matches!(event, Interrupt::Hold(_)) {
            self.pair_admission = None;
            self.pair_admission_since = None;
            self.pair_begin_since = None;
        }
        if cancel_pair_work {
            self.pair_admitted = false;
            if self.pair_step_active() {
                self.step = None;
            }
            if pair_work_pending {
                self.waiting = None;
            }
        }
        self.gang_reader.cancel();
        self.watchdog = Watchdog::default();
        self.dirty = true;
        match event {
            Interrupt::Resume | Interrupt::SessionReady => {
                self.capture_prayer_cleanup();
                self.needs_read = true;
                self.step = None;
                self.provisioner.cancel();
                self.prayer_cleanup_pending = !self.prayer_cleanup_owned.is_empty();
                self.published_bank_receipt = None;
                self.last_outcome = None;
                self.last_combat = None;
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
                self.capture_prayer_cleanup();
                self.step = None;
                self.provisioner.cancel();
                self.prayer_cleanup_pending = !self.prayer_cleanup_owned.is_empty();
                self.published_bank_receipt = None;
                self.last_outcome = None;
                self.last_combat = None;
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
        let preserve_pair_work = self.pair_work_pending();
        self.watchdog = Watchdog::default();
        self.custom_reader = None;
        self.custom_read_after = None;
        self.gang_reader.cancel();
        if !preserve_pair_work {
            self.pair_admission = None;
            self.pair_admission_since = None;
            // Dropping these guards revokes their native owners before the host can dispatch them.
            self.capture_prayer_cleanup();
            self.step = None;
            self.provisioner.cancel();
            self.clear_prayers = None;
            self.journal = None;
            self.needs_read = true;
            self.published_bank_receipt = None;
            self.last_outcome = None;
            self.last_combat = None;
            self.advances = false;
            self.attempts = 0;
            self.progress = None;
            self.settling = false;
            self.settle_deadline = Duration::ZERO;
            self.unreadable_since = None;
            self.journal_attempts = 0;
            self.journal_retry_pending = false;
            self.journal_quiet_since = None;
            self.selection_since = None;
            self.waiting = None;
        }
        self.dirty = true;
        RandomClaim::Host
    }

    fn on_stop(&mut self, _reason: StopReason) {
        self.custom_reader = None;
        self.custom_read_after = None;
        self.pair_admission = None;
        self.pair_admission_since = None;
        self.pair_begin_since = None;
        self.pair_admitted = false;
        self.gang_reader.cancel();
        self.step = None;
        self.provisioner.cancel();
        self.journal = None;
        self.settling = false;
        self.dirty = true;
        self.waiting = None;
        self.clear_prayers = None;
        self.prayer_cleanup_pending = false;
        self.published_bank_receipt = None;
        self.last_outcome = None;
        self.last_combat = None;
    }
    fn on_stop_with_output(&mut self, reason: StopReason, output: &mut dyn NativeOutput) {
        self.trace.flush_repeats(output);
        self.trace.terminal(
            output,
            api::hostlog::Level::Info,
            format_args!("quester {}: stop {reason:?}", self.path.id.0),
        );
        self.on_stop(reason);
    }

    fn recovery_anchor(&self) -> Option<api::WorldTile> {
        self.anchor
    }

    fn read_journal(&mut self) -> Result<(), ScriptFailure> {
        self.read_requested = true;
        self.dirty = true;
        Ok(())
    }
    fn prayer_cleanup(&self) -> RaisedPrayers {
        let mut owned = self.prayer_cleanup_owned;
        if let Some(step) = self.step.as_ref() {
            owned.merge(step.prayer_cleanup());
        }
        owned
    }
}

/// Queue ownership stays outside the active executor: folder snapshots share
/// validated bytes, and only the active compiled Path is retained per run.
pub struct QueuedQuester {
    run: RunKey,
    selected: Arc<SelectedGameData>,
    quests: Arc<QuestCatalog>,
    banks: Arc<api::named_banks::NamedBankFacts>,
    choices: super::choices::QuestChoices,
    queue: super::queue::Queue,
    active: Option<Box<Quester>>,
    active_index: Option<usize>,
    preparing: Option<std::thread::JoinHandle<Result<Arc<CompiledPath>, Arc<str>>>>,
    gang_reader: super::gang::GangRead,
    pair_selection: Option<(usize, super::pair::Gang)>,
    completed: u16,
    deaths: u16,
    max_deaths: u8,
    anchor: Option<api::WorldTile>,
    fields: Arc<[StatusField]>,
    gate_fields: Arc<[StatusField]>,
    quest_status_since: Option<Duration>,
    dirty: bool,
}

impl QueuedQuester {
    pub fn new(
        run: RunKey,
        selected: Arc<SelectedGameData>,
        quests: Arc<QuestCatalog>,
        banks: Arc<api::named_banks::NamedBankFacts>,
        queue: super::queue::Queue,
    ) -> Self {
        Self::new_with_max_deaths(run, selected, quests, banks, queue, default_max_deaths())
    }

    pub(super) fn new_with_max_deaths(
        run: RunKey,
        selected: Arc<SelectedGameData>,
        quests: Arc<QuestCatalog>,
        banks: Arc<api::named_banks::NamedBankFacts>,
        queue: super::queue::Queue,
        max_deaths: u8,
    ) -> Self {
        let mut this = Self {
            run,
            selected,
            quests,
            banks,
            choices: super::choices::QuestChoices::default(),
            queue,
            active: None,
            active_index: None,
            preparing: None,
            gang_reader: super::gang::GangRead::default(),
            pair_selection: None,
            completed: 0,
            deaths: 0,
            max_deaths,
            anchor: None,
            fields: Arc::from([]),
            gate_fields: Arc::from([]),
            quest_status_since: None,
            dirty: true,
        };
        this.refresh_fields();
        this
    }
    /// Apply account input before activation. Cached Paths remain shared.
    pub fn set_choices(&mut self, choices: super::choices::QuestChoices) {
        self.choices = choices;
        if let Some(active) = &mut self.active {
            active.choices = choices;
        }
    }

    pub(super) fn restore(&mut self, retained: &super::QuesterRetained) {
        self.anchor = retained.anchor;
        self.deaths = retained.deaths;
        self.completed = retained.completed;
        self.refresh_fields();
    }

    fn sync_retained(&self, tick: &mut NativeTick<'_>) {
        let retained = tick.cx.retained().quester();
        retained.anchor = self.anchor;
        retained.deaths = self.deaths;
        retained.completed = self.completed;
    }

    fn refresh_fields(&mut self) {
        let mut fields = vec![
            StatusField {
                key: "queue",
                label: "Queue",
                value: StatusValue::Text(self.queue.status_text().into()),
            },
            StatusField {
                key: "completed",
                label: "Completed this session",
                value: StatusValue::Integer(i64::from(self.completed)),
            },
            StatusField {
                key: "session_deaths",
                label: "Session deaths",
                value: StatusValue::Integer(i64::from(self.deaths)),
            },
        ];
        if let Some(report) = self.queue.path_report() {
            fields.push(StatusField {
                key: "path_validation",
                label: "Path validation",
                value: StatusValue::Text(Arc::clone(report)),
            });
            if let Some(index) = self.active_index {
                static SOURCES: std::sync::LazyLock<[Arc<str>; 3]> =
                    std::sync::LazyLock::new(|| {
                        [
                            Arc::from(PathSource::Bundled.label()),
                            Arc::from(PathSource::Folder.label()),
                            Arc::from(PathSource::Draft.label()),
                        ]
                    });
                let source = self.queue.path_source(index);
                fields.push(StatusField {
                    key: "path_source",
                    label: "Path source",
                    value: StatusValue::Text(Arc::clone(&SOURCES[source as usize])),
                });
            }
        }
        if let Some(reason) = self
            .active_index
            .and_then(|index| self.queue.reason(index))
            .or_else(|| {
                self.queue
                    .rows()
                    .iter()
                    .enumerate()
                    .filter(|(_, row)| row.picked && !row.skipped)
                    .find_map(|(index, _)| self.queue.reason(index))
            })
        {
            fields.push(StatusField {
                key: "block_reason",
                label: "Queue block reason",
                value: StatusValue::Text(Arc::from(reason)),
            });
        }
        if self.active.is_none() {
            fields.extend_from_slice(&self.gate_fields);
        }
        self.fields = fields.into();
        if let Some(active) = self.active.as_mut() {
            active.queue_fields = Arc::clone(&self.fields);
            active.dirty = true;
        }
        self.dirty = true;
    }

    fn publish(&mut self, output: &mut dyn NativeOutput, phase: NativePhase) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        let mut fields = self.fields.to_vec();
        fields.push(StatusField {
            key: "deaths",
            label: "Deaths",
            value: StatusValue::Integer(i64::from(self.deaths)),
        });
        let failure = (phase == NativePhase::Blocked).then(|| self.blocked());
        if let Some(failure) = &failure {
            if !fields.iter().any(|field| field.key == "block_reason") {
                fields.push(StatusField {
                    key: "block_reason",
                    label: "Queue block reason",
                    value: StatusValue::Text(Arc::clone(&failure.message)),
                });
            }
        }
        if phase == NativePhase::Waiting {
            fields.push(StatusField {
                key: "waiting_for",
                label: "Waiting for",
                value: StatusValue::Text(Arc::from(if self.quest_status_since.is_some() {
                    "quest list from login; no gameplay until observed"
                } else {
                    "host safety hold to clear"
                })),
            });
        }
        output.status(ScriptStatus {
            run: self.run,
            card: CompiledId("Quester"),
            phase,
            active_settings: 1,
            pending_settings: None,
            fields: fields.into(),
            failure,
        });
    }

    fn blocked(&self) -> ScriptFailure {
        if self.quest_status_since.is_some() {
            return ScriptFailure {
                code: Arc::from("quest-status-unobserved"),
                message: Arc::from(
                    "quest list was not observed within 30 seconds: log out and log in normally outside the tutorial (finish it if needed), then Stop/Start Quester",
                ),
            };
        }
        if !self
            .queue
            .rows()
            .iter()
            .any(|row| row.picked && !row.skipped)
        {
            return ScriptFailure {
                code: Arc::from("empty-queue"),
                message: Arc::from(
                    "no quests selected: review Quests and Skip in Script prefs; empty Quests selects all released quests, then Stop/Start Quester",
                ),
            };
        }
        let (reason, status) = self
            .queue
            .rows()
            .iter()
            .enumerate()
            .filter(|(_, row)| row.picked && !row.skipped)
            .find_map(|(index, row)| self.queue.reason(index).map(|reason| (reason, row.status)))
            .unwrap_or(("no eligible selected quest", QueueStatus::Blocked));
        // A requirement block can be changed in Script prefs; a parked run
        // (a refused walk, a failed action) can't, so don't send the user there.
        let recovery = if status == QueueStatus::Blocked {
            "review Quests/Skip and requirements in Script prefs, resolve the condition, then Stop/Start Quester"
        } else {
            "resolve the condition, then Stop/Start Quester"
        };
        ScriptFailure {
            code: Arc::from("queue-blocked"),
            message: format!("{reason}; {recovery}").into(),
        }
    }

    fn activate(&mut self, tick: &mut NativeTick<'_>, path: Arc<CompiledPath>) {
        let index = self.active_index.expect("preparing queue row");
        let result = super::eligibility::evaluate(
            &path,
            &tick.cx.snapshot(),
            &self.quests,
            &BankMemo::default(),
        );
        self.gate_fields = skill_status_fields(&result.skill_gates);
        match result.state {
            super::eligibility::Eligibility::Done => self.queue.mark_done(index),
            super::eligibility::Eligibility::Blocked(reasons) => {
                self.queue.mark_blocked(
                    index,
                    reasons
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join("; ")
                        .into(),
                );
            }
            super::eligibility::Eligibility::Ready => {
                self.queue.mark_running(index);
                let mut active = Quester::new(
                    self.run,
                    path,
                    Arc::clone(&self.selected),
                    Arc::clone(&self.quests),
                    Arc::clone(&self.banks),
                );
                tick.output.log(
                    api::hostlog::Level::Info,
                    &format!(
                        "Quester Start: Path {} source={} digest={} step_comment={:?}",
                        active.path.id.0,
                        self.queue.path_source(index).label(),
                        super::registry::digest_text(&active.path.digest),
                        active
                            .current_step()
                            .and_then(|step| step.comment.as_deref())
                            .unwrap_or(""),
                    ),
                );
                active.choices = self.choices;
                active.prior_deaths = self.deaths;
                active.max_deaths = self.max_deaths;
                active.anchor = self.anchor;
                active.death = DeathLatch::from_watermark(tick.cx.retained().quester().death_seq);
                active.required_vs_live = skill_status_fields(&result.skill_gates);
                active.tested_stats_warning = match active.path.tested_stats.as_deref() {
                    None => Arc::from("No qualified stats recorded"),
                    Some(tested) => {
                        let stats = tick.cx.snapshot().stats();
                        tested
                            .iter()
                            .filter_map(|minimum| {
                                let live = stats.as_ref().and_then(|stats| {
                                    stats.value.iter().find(|stat| {
                                        stat.used && stat.index == i32::from(minimum.skill)
                                    })
                                });
                                match live {
                                    Some(stat) if stat.base >= i32::from(minimum.level) => None,
                                    Some(stat) => Some(format!(
                                        "{}: live {} below tested {}",
                                        stat.name, stat.base, minimum.level
                                    )),
                                    None => Some(format!(
                                        "skill {}: live unobserved, tested {}",
                                        minimum.skill, minimum.level
                                    )),
                                }
                            })
                            .collect::<Vec<_>>()
                            .join("; ")
                            .into()
                    }
                };
                self.active = Some(Box::new(active));
            }
        }
        self.refresh_fields();
    }
    fn prepare_pair_role(
        &mut self,
        index: usize,
        tick: &mut NativeTick<'_>,
    ) -> Poll<Result<super::pair::Gang, ActionError>> {
        if let Some((selected, gang)) = self.pair_selection {
            if selected == index {
                return Poll::Ready(Ok(gang));
            }
        }
        match self.gang_reader.poll(tick, &self.quests) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Err(error)) => Poll::Ready(Err(error)),
            Poll::Ready(Ok(observed)) => {
                let gang = match observed {
                    Knowledge::Known(Some(gang)) => {
                        if self.queue.gang.is_some_and(|declared| declared != gang) {
                            return Poll::Ready(Err(super::pair::PairError::WrongGang.action()));
                        }
                        gang
                    }
                    Knowledge::Known(None) => {
                        let Some(gang) = self.queue.gang else {
                            return Poll::Ready(Err(ActionError::Blocked(Arc::from(
                                "choose an irreversible gang explicitly for this unjoined account",
                            ))));
                        };
                        gang
                    }
                    _ => return Poll::Ready(Err(super::pair::PairError::UnknownGang.action())),
                };
                self.pair_selection = Some((index, gang));
                Poll::Ready(Ok(gang))
            }
        }
    }
}

impl Script for QueuedQuester {
    fn pair_settings(&self) -> Option<super::pair::PairSettings> {
        Some(super::pair::PairSettings {
            partner: self.queue.partner_account.clone(),
            gang: self.queue.gang,
        })
    }
    fn pair_binding(&self) -> Option<super::pair::PairBinding<'_>> {
        self.active
            .as_ref()
            .and_then(|active| active.pair_binding())
    }
    fn tick(&mut self, tick: &mut NativeTick<'_>) -> Result<ScriptFlow, ScriptFailure> {
        if self.run != tick.cx.run() {
            self.run = tick.cx.run();
            self.gang_reader.cancel();
            self.pair_selection = None;
            self.queue.refresh_blocked();
            self.quest_status_since = None;
            self.refresh_fields();
        }
        if !tick.cx.eligible {
            if let Some(active) = self.active.as_mut() {
                active.watchdog = Watchdog::default();
            }
            self.publish(tick.output, NativePhase::Waiting);
            return Ok(ScriptFlow::Continue);
        }
        // Baseline old chat once at Start, before the off-pump Path compile.
        // A recreated card instead keeps the prior watermark, including deaths
        // received while its old instance was absent.
        if tick.cx.retained().quester().death_seq.is_none() {
            let mut baseline = DeathLatch::default();
            baseline.observe(tick.cx.snapshot());
            tick.cx.retained().quester().death_seq = baseline.watermark();
        }
        if let Some(active) = self.active.as_mut() {
            let flow = active.tick(tick)?;
            self.anchor = active.anchor;
            tick.cx.retained().quester().anchor = self.anchor;
            match &flow {
                ScriptFlow::Continue => return Ok(ScriptFlow::Continue),
                ScriptFlow::Blocked(failure)
                    if matches!(failure.code.as_ref(), "manual-movement" | "max-deaths") =>
                {
                    return Ok(flow);
                }
                ScriptFlow::Complete | ScriptFlow::Blocked(_) => {
                    let active = self.active.take().expect("active quest");
                    let index = self.active_index.expect("active queue row");
                    self.deaths = self.deaths.saturating_add(active.deaths);
                    match flow {
                        ScriptFlow::Complete => {
                            self.queue.mark_done(index);
                            self.completed = self.completed.saturating_add(1);
                            self.anchor = None;
                            self.queue.refresh_blocked();
                        }
                        ScriptFlow::Blocked(failure) => {
                            self.queue.mark_parked(index, failure.message)
                        }
                        ScriptFlow::Continue => unreachable!(),
                    }
                    self.refresh_fields();
                    self.sync_retained(tick);
                }
            }
        }
        if !self
            .queue
            .rows()
            .iter()
            .any(|row| row.picked && !row.skipped)
        {
            self.publish(tick.output, NativePhase::Blocked);
            return Ok(ScriptFlow::Blocked(self.blocked()));
        }
        // Login publishes the quest-list family separately from the scene.
        // Unknown evidence is not an ineligible quest: retain admission until
        // it arrives, without compiling paths or emitting gameplay effects.
        if self.queue.next_candidate().is_some() && tick.cx.snapshot().quest_statuses().is_none() {
            let now = tick.cx.active_now();
            if self.quest_status_since.is_none() {
                self.quest_status_since = Some(now);
                self.dirty = true;
            }
            let since = self.quest_status_since.expect("quest observation wait");
            if now.saturating_sub(since) >= QUEUE_QUEST_STATUS_WAIT {
                self.dirty = true;
                self.publish(tick.output, NativePhase::Blocked);
                return Ok(ScriptFlow::Blocked(self.blocked()));
            }
            self.publish(tick.output, NativePhase::Waiting);
            return Ok(ScriptFlow::Continue);
        }
        if self.quest_status_since.take().is_some() {
            self.refresh_fields();
        }
        if self
            .preparing
            .as_ref()
            .is_some_and(|worker| worker.is_finished())
        {
            let result = self.preparing.take().expect("finished preparation").join();
            match result {
                Ok(Ok(path)) => self.activate(tick, path),
                failure => {
                    let reason = match failure {
                        Ok(Err(reason)) => reason,
                        Err(_) => Arc::from("Path compiler worker panicked"),
                        Ok(Ok(_)) => unreachable!(),
                    };
                    self.queue
                        .mark_blocked(self.active_index.expect("preparing row"), reason);
                    self.refresh_fields();
                }
            }
            self.publish(tick.output, NativePhase::Working);
            return Ok(ScriptFlow::Continue);
        }
        if self.preparing.is_some() {
            self.publish(tick.output, NativePhase::Preparing);
            return Ok(ScriptFlow::Continue);
        }
        if let Some(index) = self.queue.next_candidate() {
            let paired = super::pair::PairQuest::from_path(self.queue.id(index).unwrap()).is_some();
            let gang = if paired {
                match self.prepare_pair_role(index, tick) {
                    Poll::Pending => {
                        self.publish(tick.output, NativePhase::Waiting);
                        return Ok(ScriptFlow::Continue);
                    }
                    Poll::Ready(Ok(gang)) => Some(gang),
                    Poll::Ready(Err(error)) => {
                        self.queue.mark_blocked(
                            index,
                            Arc::from(format!("partner admission: {error:?}")),
                        );
                        self.refresh_fields();
                        return Ok(ScriptFlow::Continue);
                    }
                }
            } else {
                None
            };
            let id: Arc<str> = Arc::from(self.queue.id(index).expect("selected queue row"));
            tick.output.log(
                api::hostlog::Level::Info,
                &format!("quester queue: preparing next Path {id}"),
            );
            let bytes = self.queue.path_bytes(index);
            let selected = Arc::clone(&self.selected);
            let quests = Arc::clone(&self.quests);
            self.active_index = Some(index);
            let worker = api::selected::FamilyPreparation::run(move |cap| {
                let bytes =
                    bytes.ok_or_else(|| Arc::<str>::from(format!("Path {id} is unavailable")))?;
                super::compile::compile_path_for_gang(bytes.as_ref(), &selected, &quests, cap, gang)
                    .map_err(|error| {
                        let detail = error.detail.as_deref().unwrap_or("Path compilation failed");
                        Arc::from(match &error.step {
                            Some(step) => format!("{} [{}]: {detail}", error.code, step.0),
                            None => format!("{}: {detail}", error.code),
                        })
                    })
            });
            match worker {
                Ok(worker) => self.preparing = Some(worker),
                Err(error) => {
                    self.queue
                        .mark_blocked(index, Arc::from(format!("Path preparation: {error}")));
                    self.refresh_fields();
                }
            }
            self.dirty = true;
            self.publish(tick.output, NativePhase::Preparing);
            return Ok(ScriptFlow::Continue);
        }
        self.dirty = true;
        if self.queue.all_done() {
            self.publish(tick.output, NativePhase::Complete);
            // Completion ends the allowance just like Stop. A later operator
            // Start can reuse the slot, but must not inherit this finished run.
            *tick.cx.retained().quester() = super::QuesterRetained::default();
            Ok(ScriptFlow::Complete)
        } else {
            self.publish(tick.output, NativePhase::Blocked);
            Ok(ScriptFlow::Blocked(self.blocked()))
        }
    }

    fn interrupt(&mut self, event: Interrupt) {
        self.gang_reader.cancel();
        self.pair_selection = None;
        if let Some(active) = self.active.as_mut() {
            active.interrupt(event);
        }
    }

    fn on_stop(&mut self, reason: StopReason) {
        self.gang_reader.cancel();
        self.pair_selection = None;
        if let Some(active) = self.active.as_mut() {
            active.on_stop(reason);
        }
    }
    fn on_stop_with_output(&mut self, reason: StopReason, output: &mut dyn NativeOutput) {
        if let Some(active) = self.active.as_mut() {
            active.on_stop_with_output(reason, output);
        }
    }

    fn on_random(&mut self, event: &DetectedRandom) -> RandomClaim {
        self.gang_reader.cancel();
        self.pair_selection = None;
        self.active
            .as_mut()
            .map_or(RandomClaim::Host, |active| active.on_random(event))
    }

    fn recovery_anchor(&self) -> Option<api::WorldTile> {
        self.active
            .as_ref()
            .and_then(|active| active.recovery_anchor())
            .or(self.anchor)
    }
    fn prayer_cleanup(&self) -> RaisedPrayers {
        self.active
            .as_ref()
            .map_or_else(RaisedPrayers::empty, |active| active.prayer_cleanup())
    }

    fn read_journal(&mut self) -> Result<(), ScriptFailure> {
        if let Some(active) = self.active.as_mut() {
            active.read_journal()
        } else {
            Err(ScriptFailure {
                code: Arc::from("no-active-quest"),
                message: Arc::from("No active quest to read"),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_status_is_nonterminal_and_journal_payload_is_published_once() {
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
        script.queue_fields = Arc::from([StatusField {
            key: "queue",
            label: "Queue",
            value: StatusValue::Text(Arc::from("R?")),
        }]);
        script.required_vs_live = skill_status_fields(&[super::super::eligibility::SkillGate {
            id: Arc::from("attack-gate"),
            skill: 0,
            name: Arc::from("Attack"),
            required: 40,
            live: Some(42),
        }]);
        script.journal_text = Some(Arc::from("fresh journal"));
        let mut output = Capture::default();
        script.emit_status(&mut output, NativePhase::Complete);
        assert_eq!(output.0[0].phase, NativePhase::Working);
        assert!(output.0[0]
            .fields
            .iter()
            .any(|field| field.key == "journal_lines"
                && field.value == StatusValue::Text(Arc::from("fresh journal"))));
        assert!(
            output.0[0]
                .fields
                .iter()
                .any(|field| field.key == "required_attack"
                    && field.value == StatusValue::Integer(40))
        );
        assert!(output.0[0]
            .fields
            .iter()
            .any(|field| field.key == "live_attack" && field.value == StatusValue::Integer(42)));
        script.emit_status(&mut output, NativePhase::Blocked);
        assert_eq!(output.0[1].phase, NativePhase::Working);
        assert!(!output.0[1]
            .fields
            .iter()
            .any(|field| field.key == "journal_lines"));
        script.last_error_kind = QuesterFailureKind::ManualMovement;
        script.emit_status(&mut output, NativePhase::Blocked);
        assert_eq!(output.0[2].phase, NativePhase::Blocked);
    }

    #[test]
    fn authored_and_synthetic_family_failures_share_the_bounded_retry_policy() {
        let (mut script, snapshot) = fixture();
        let mut ledger = None;
        for attempt in 1..=5 {
            super::super::families::tests::with_tick(&snapshot, &mut ledger, attempt, |tick| {
                script.record_step_failure(
                    ActionError::Failed(Arc::from("interact target not reached")),
                    tick,
                );
            });
            assert_eq!(script.fail_streak, attempt as u8);
            assert_eq!(script.parked, attempt == 5);
        }
        assert_eq!(
            script.blocked_failure().message.as_ref(),
            "interact target not reached"
        );
    }

    #[test]
    fn quester_struct_fits_the_per_bot_budget() {
        let bytes = std::mem::size_of::<Quester>();
        let bank_bytes = std::mem::size_of::<BankMemo>();
        assert!(std::mem::size_of::<QueuedQuester>() < 4096);
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
        let allocations = allocation_counter::measure(|| {
            for _ in 0..200 {
                script.publish(&mut output);
            }
        });
        assert_eq!(allocations.count_total, 0);
        assert_eq!(
            output.0.len(),
            1,
            "unchanged reports must not be republished"
        );

        let mut killed = report;
        killed.end = CombatEnd::Killed;
        killed.evidence.tick = 24;
        script.last_outcome = Some(StepOutcome {
            progress: None,
            evidence: killed.evidence,
            receipt: Some(Arc::new(CombatReceipt {
                report: killed,
                target_gone_restarts: 1,
            })),
        });
        script.publish(&mut output);
        assert_eq!(output.0.len(), 2);
        assert!(output.0[1].fields.iter().any(|field| {
            field.key == "combat_end"
                && matches!(&field.value, StatusValue::Text(value) if value.as_ref() == "Killed")
        }));
        script.last_outcome = None;
        script.publish(&mut output);
        assert_eq!(output.0.len(), 3);
        assert!(!output.0[2]
            .fields
            .iter()
            .any(|field| field.key == "combat_end"));

        script.last_combat = Some(StepOutcome {
            progress: None,
            evidence: report.evidence,
            receipt: Some(Arc::new(CombatReceipt {
                report,
                target_gone_restarts: 1,
            })),
        });
        script.publish(&mut output);
        assert_eq!(output.0.len(), 4);
        assert!(output.0[3].fields.iter().any(|field| {
            field.key == "combat_end"
                && matches!(&field.value, StatusValue::Text(value) if value.as_ref() == "TargetGone")
        }));
    }

    #[test]
    fn no_food_abort_walks_then_parks_with_receipt_without_advancing() {
        use crate::native::{HostEffect, WalkEnd, WalkReceipt};
        use api::quest_progress::EvidenceStamp;
        use api::selected::ClientRevision;
        use api::snapshot::{GameSnapshot, NpcView};

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

        fn scene(here: api::WorldTile, npc_tile: api::WorldTile, npc_type: i32) -> GameSnapshot {
            let mut snapshot = GameSnapshot::new();
            snapshot.seed_ingame(2);
            snapshot.seed_inventory(Vec::new(), 28);
            snapshot.seed_local_player(super::super::families::tests::local_player(here));
            snapshot.seed_npcs(vec![NpcView {
                index: 0,
                r#type: Some(usize::try_from(npc_type).unwrap()),
                name: Some("Khazard warlord".into()),
                actions: vec![Some("Attack".into())],
                tile: npc_tile,
                distance: 1,
                animation: -1,
                animation_frame: 0,
                pose_animation: -1,
                orientation: 0,
                target_orientation: 0,
                overhead_text: None,
                spot_animation: -1,
                spot_animation_stamp: 0,
                health: 40,
                total_health: 40,
                face_entity: -1,
                target: None,
                moving: false,
                running: false,
                in_combat: true,
                level: 0,
                size: 1,
                network: npc_tile,
                x: npc_tile.x,
                z: npc_tile.z,
                yaw: 0,
            }]);
            snapshot
        }

        fn chebyshev(a: api::WorldTile, b: api::WorldTile) -> i32 {
            (a.x - b.x).abs().max((a.z - b.z).abs())
        }

        let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
        let quests = Arc::new(QuestCatalog::from_identity(data.quest_identity()).unwrap());
        let path = super::super::compile::prepare_for_test({
            let data = Arc::clone(&data);
            let quests = Arc::clone(&quests);
            move |cap| {
                super::super::compile::compile_path(
                    include_bytes!("../../paths/289/fixtures/combat_melee_food_only.json"),
                    &data,
                    &quests,
                    cap,
                )
            }
        })
        .unwrap();
        let run = RunKey {
            slot: 1,
            run: 1,
            session: 1,
        };
        let mut script = Quester::new(
            run,
            path,
            Arc::clone(&data),
            quests,
            Arc::new(api::named_banks::NamedBankFacts::empty()),
        );
        script.stage = Some(FactKey::new("imp:0"));
        script.needs_read = false;
        script.prayer_cleanup_pending = false;

        let here = api::WorldTile {
            x: 2458,
            z: 3303,
            level: 0,
        };
        let npc_tile = api::WorldTile {
            x: 2457,
            z: 3302,
            level: 0,
        };
        let stand = npc_tile;
        let warlord = data.npc_by_config("khazard_warlord").unwrap();
        let snapshot = scene(here, npc_tile, warlord.id);
        let mut ledger = None;
        let mut output = Capture::default();
        let step = super::super::families::tests::with_tick_output(
            &snapshot,
            &mut ledger,
            24,
            &mut output,
            |tick| {
                let required_after = tick.cx.evidence();
                let mut step_cx = StepContext {
                    tick,
                    quests: &script.quests,
                    progress: &[],
                    required_after,
                    bank: &script.bank,
                    banks: &script.banks,
                    choices: &script.choices,
                };
                super::super::families::combat::tests::no_food_abort_run_for_runner(
                    &mut step_cx,
                    stand,
                )
            },
        );
        script.step = Some(step);
        let cursor = (script.seq_index, script.step_index);

        let (request_id, destination) = {
            let action = ledger
                .as_ref()
                .unwrap()
                .outbox
                .iter()
                .find(|action| matches!(&action.effect, HostEffect::Walk(_)))
                .expect("the CombatRun must issue its abort walk");
            let HostEffect::Walk(request) = &action.effect else {
                unreachable!();
            };
            (action.request_id.get(), request.target)
        };
        assert_ne!(destination, stand, "the unsafe boss stand is not an exit");
        assert!(
            chebyshev(destination, npc_tile) > warlord.maxrange + warlord.attackrange,
            "the abort walk must leave the warlord's spawn chase envelope"
        );

        let before_arrival = super::super::families::tests::with_tick_output(
            &snapshot,
            &mut ledger,
            25,
            &mut output,
            |tick| script.tick(tick).unwrap(),
        );
        assert!(matches!(before_arrival, ScriptFlow::Continue));
        assert!(!script.parked, "do not park before the abort walk settles");
        assert_eq!((script.seq_index, script.step_index), cursor);

        let arrived = scene(destination, npc_tile, warlord.id);
        ledger.as_mut().unwrap().walk = Some(WalkReceipt {
            request_id,
            evidence: EvidenceStamp {
                run,
                tick: 26,
                sequence: 26,
            },
            end: WalkEnd::Arrived,
            blocked: None,
            detail: None,
        });
        assert!(matches!(
            super::super::families::tests::with_tick_output(
                &arrived,
                &mut ledger,
                26,
                &mut output,
                |tick| script.tick(tick).unwrap()
            ),
            ScriptFlow::Blocked(_)
        ));
        assert!(script.parked);
        assert!(script.step.is_none());
        assert_eq!(
            (script.seq_index, script.step_index),
            cursor,
            "the failed combat step must not advance after its walk"
        );
        let retained = script
            .last_combat
            .as_ref()
            .and_then(|outcome| outcome.receipt.as_ref())
            .and_then(|receipt| receipt.as_any().downcast_ref::<CombatReceipt>())
            .expect("the runner must retain the terminal combat receipt");
        assert_eq!(
            retained.report.end,
            crate::combat::CombatEnd::Aborted(crate::combat::AbortReason::Unprotected(
                crate::combat::Unprotected::NoFood
            ))
        );
        assert_eq!(retained.report.evidence.tick, 12);
        assert_eq!(retained.report.engaged.map(|actor| actor.index), Some(0));
        assert_eq!(retained.report.engaged_npc_type, warlord.id);
        assert!(
            output.0.last().unwrap().fields.iter().any(|field| {
                field.key == "combat_end"
                    && matches!(&field.value, StatusValue::Text(value)
                        if value.as_ref() == "Aborted(Unprotected(NoFood))")
            }),
            "the original NoFood end must remain visible after the walk"
        );
        assert_eq!(
            ledger.as_ref().unwrap().outbox.len(),
            1,
            "the aborted CombatRun must not restart combat after walking"
        );
    }

    #[test]
    fn message_settle_uses_step_begin_chat_not_tick_sequence() {
        use super::super::families::tests::with_tick;
        use api::snapshot::{ChatLineView, GameSnapshot, QuestStatusView};
        let data = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
        let quests = Arc::new(QuestCatalog::from_identity(data.quest_identity()).unwrap());
        let mut document = super::super::compile::decode_cook().unwrap();
        document.quest.as_mut().unwrap().owns_inventory = true;
        let step = &mut document.roles[0].sequences[0].steps[0];
        step.kind = "wait".into();
        step.args = serde_json::json!({"until":{"All":[]},"max_ticks": 10});
        step.advances = Some(false);
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
        let mut script = Quester::new(
            run,
            path,
            Arc::clone(&data),
            quests,
            Arc::new(api::named_banks::NamedBankFacts::empty()),
        );
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

    pub(super) fn fixture() -> (Quester, api::snapshot::GameSnapshot) {
        let data = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
        let quests = Arc::new(QuestCatalog::from_identity(data.quest_identity()).unwrap());
        // These tests isolate runner/dialogue transitions. Provisioning has its own
        // behavioral fixtures, and the live queue exercises the unchanged Cook Path.
        let mut document = super::super::compile::decode_cook().unwrap();
        document.quest.as_mut().unwrap().owns_inventory = true;
        let path =
            super::super::compile::compile_uncached_for_test(&document, &data, &quests).unwrap();
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
                Arc::new(api::named_banks::NamedBankFacts::empty()),
            ),
            s,
        )
    }
    fn status_fixture(
        mut document: super::super::path::PathDocument,
    ) -> (Quester, api::snapshot::GameSnapshot) {
        let data = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
        let quests = Arc::new(QuestCatalog::from_identity(data.quest_identity()).unwrap());
        document.quest.as_mut().unwrap().owns_inventory = true;
        let path =
            super::super::compile::compile_uncached_for_test(&document, &data, &quests).unwrap();
        let run = RunKey {
            slot: 1,
            run: 1,
            session: 1,
        };
        let mut script = Quester::new(
            run,
            path,
            Arc::clone(&data),
            quests,
            Arc::new(api::named_banks::NamedBankFacts::empty()),
        );
        let stage = script.path.colour_not_started.clone();
        script.seq_index =
            sequence_for_stage(&script.path, stage.0.as_ref()).expect("not-started sequence");
        script.stage = Some(stage);
        script.needs_read = false;
        let mut snapshot = api::snapshot::GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_quest_statuses(
            vec![api::snapshot::QuestStatusView {
                name: "Cook's Assistant".into(),
                component_id: 42,
                colour: 0xf80000,
            }],
            true,
        );
        (script, snapshot)
    }

    #[derive(Default)]
    struct StatusCapture(Vec<ScriptStatus>);

    impl NativeOutput for StatusCapture {
        fn status(&mut self, status: ScriptStatus) {
            self.0.push(status);
        }
        fn paint(&mut self, _: Arc<crate::shim::ScriptPaint>) {}
        fn log(&mut self, _: api::hostlog::Level, _: &str) {}
        fn settings_applied(&mut self, _: u64) {}
    }
    #[derive(Default)]
    struct TraceCapture {
        statuses: Vec<ScriptStatus>,
        logs: Vec<(api::hostlog::Level, String)>,
    }

    impl NativeOutput for TraceCapture {
        fn status(&mut self, status: ScriptStatus) {
            self.statuses.push(status);
        }
        fn paint(&mut self, _: Arc<crate::shim::ScriptPaint>) {}
        fn log(&mut self, level: api::hostlog::Level, message: &str) {
            self.logs.push((level, message.to_owned()));
        }
        fn settings_applied(&mut self, _: u64) {}
    }

    #[test]
    fn run_trace_caps_distinct_events() {
        let mut trace = RunTrace::default();
        let mut output = TraceCapture::default();
        for event in 0..=RUN_TRACE_EVENT_LIMIT {
            trace.record(
                &mut output,
                api::hostlog::Level::Info,
                format_args!("event {event}"),
            );
        }
        assert_eq!(output.logs.len(), RUN_TRACE_EVENT_LIMIT);
        trace.flush_repeats(&mut output);
        assert!(output
            .logs
            .last()
            .unwrap()
            .1
            .contains("truncated after the event limit"));
    }

    fn test_step(
        id: &str,
        kind: &str,
        args: serde_json::Value,
        skip_if: super::super::path::PredicateDocument,
        settle: super::super::path::PredicateDocument,
    ) -> super::super::path::StepDocument {
        super::super::path::StepDocument {
            id: FactKey::new(id),
            kind: kind.to_owned(),
            version: 1,
            args,
            comment: None,
            advances: Some(false),
            skip_if,
            settle,
        }
    }

    fn cook_acquire_document(
        child: super::super::path::StepDocument,
    ) -> super::super::path::PathDocument {
        use super::super::path::PredicateDocument;

        let mut document = super::super::compile::decode_cook().unwrap();
        document
            .quest
            .as_mut()
            .unwrap()
            .acquire
            .insert("test:trace-child".to_owned(), vec![child]);
        document.roles[0].sequences[0].steps = vec![test_step(
            "root-acquire-step",
            "acquire",
            serde_json::json!({"recipe":"test:trace-child"}),
            PredicateDocument::Any(vec![]),
            PredicateDocument::All(vec![]),
        )];
        document
    }

    fn status_text<'a>(status: &'a ScriptStatus, key: &str) -> &'a str {
        status
            .fields
            .iter()
            .find(|field| field.key == key)
            .and_then(|field| match &field.value {
                StatusValue::Text(value) => Some(value.as_ref()),
                _ => None,
            })
            .unwrap_or_else(|| panic!("status field {key} is missing or not text"))
    }

    fn park_status(document: super::super::path::PathDocument, max_tick: u64) -> ScriptStatus {
        use super::super::families::tests::with_tick_output;

        let (mut script, snapshot) = status_fixture(document);
        let mut ledger = None;
        let mut output = StatusCapture::default();
        for tick in 1..=max_tick {
            with_tick_output(&snapshot, &mut ledger, tick, &mut output, |native| {
                script.tick(native).unwrap();
            });
            if script.parked {
                break;
            }
        }
        assert!(script.parked, "the failing run must park");
        output
            .0
            .last()
            .cloned()
            .expect("parked status was published")
    }

    fn assert_acquisition_child_failure(
        status: &ScriptStatus,
        recipe: &str,
        child: &str,
        reason: &str,
    ) {
        assert_eq!(status.phase, NativePhase::Blocked);
        assert_eq!(status_text(status, "step_id"), "root-acquire-step");
        assert_eq!(status_text(status, "child_recipe_id"), recipe);
        assert_eq!(status_text(status, "child_step_id"), child);
        assert_eq!(status_text(status, "last_failure"), reason);
        assert_eq!(status.failure.as_ref().unwrap().message.as_ref(), reason);
    }

    #[test]
    fn acquisition_settle_timeout_keeps_child_in_parked_status() {
        use super::super::path::PredicateDocument;

        let child = test_step(
            "child-settle-timeout",
            "wait",
            serde_json::json!({"until":{"All":[]},"max_ticks":1}),
            PredicateDocument::Any(vec![]),
            PredicateDocument::Any(vec![]),
        );
        let status = park_status(cook_acquire_document(child), 120);
        assert_acquisition_child_failure(
            &status,
            "test:trace-child",
            "child-settle-timeout",
            "acquire settle timeout",
        );
    }

    #[test]
    fn acquisition_skip_evidence_timeout_keeps_child_in_parked_status() {
        use super::super::path::PredicateDocument;

        let child = test_step(
            "child-skip-timeout",
            "wait",
            serde_json::json!({"until":{"All":[]},"max_ticks":1}),
            PredicateDocument::Fact {
                kind: "has_item".to_owned(),
                version: 1,
                args: serde_json::json!({"obj":"egg"}),
            },
            PredicateDocument::All(vec![]),
        );
        let status = park_status(cook_acquire_document(child), 240);
        assert_acquisition_child_failure(
            &status,
            "test:trace-child",
            "child-skip-timeout",
            "acquire skip predicate evidence unavailable",
        );
    }

    #[test]
    fn root_skip_evidence_timeout_names_the_unknown_step() {
        use super::super::path::PredicateDocument;

        let mut document = super::super::compile::decode_cook().unwrap();
        document.roles[0].sequences[0].steps = vec![
            test_step(
                "skipped-before-unknown",
                "wait",
                serde_json::json!({"until":{"All":[]},"max_ticks":1}),
                PredicateDocument::Fact {
                    kind: "quest_colour".to_owned(),
                    version: 1,
                    args: serde_json::json!({"quest":"cook","is":"not_started"}),
                },
                PredicateDocument::All(vec![]),
            ),
            test_step(
                "root-skip-timeout",
                "wait",
                serde_json::json!({"until":{"All":[]},"max_ticks":1}),
                PredicateDocument::Fact {
                    kind: "has_item".to_owned(),
                    version: 1,
                    args: serde_json::json!({"obj":"egg"}),
                },
                PredicateDocument::All(vec![]),
            ),
        ];
        let status = park_status(document, 240);
        assert_eq!(status.phase, NativePhase::Blocked);
        assert_eq!(status_text(&status, "step_id"), "root-skip-timeout");
        assert_eq!(status_text(&status, "child_recipe_id"), "");
        assert_eq!(status_text(&status, "child_step_id"), "");
    }

    #[test]
    fn acquisition_child_begin_failure_keeps_child_in_parked_status() {
        use super::super::path::{PathDocument, PredicateDocument};

        let mut document: PathDocument =
            serde_json::from_str(super::super::compile::SHEEP_JSON).unwrap();
        document.quest.as_mut().unwrap().acquire.insert(
            "test:trace-child".to_owned(),
            vec![test_step(
                "child-begin-failure",
                "make",
                serde_json::json!({
                    "loc":{"name":"spinningwheel","op":"Spin"},
                    "anchor":{"tile":[2982,3315,0],"source":"test"},
                    "product":"ball_of_wool",
                    "qty":{"progress":{"quest":"sheep","flag":"sheep:balls_to_go"}}
                }),
                PredicateDocument::Any(vec![]),
                PredicateDocument::All(vec![]),
            )],
        );
        document.roles[0].sequences[0].steps = vec![test_step(
            "root-acquire-step",
            "acquire",
            serde_json::json!({"recipe":"test:trace-child"}),
            PredicateDocument::Any(vec![]),
            PredicateDocument::All(vec![]),
        )];

        let status = park_status(document, 40);
        assert_acquisition_child_failure(
            &status,
            "test:trace-child",
            "child-begin-failure",
            "production quantity evidence unavailable",
        );
    }

    #[test]
    fn later_begin_failure_does_not_publish_stale_acquisition_child() {
        use super::super::families::tests::with_tick_output;
        use super::super::path::{PathDocument, PredicateDocument};

        let mut document: PathDocument =
            serde_json::from_str(super::super::compile::SHEEP_JSON).unwrap();
        document.quest.as_mut().unwrap().acquire.insert(
            "test:trace-child".to_owned(),
            vec![test_step(
                "child-first-failure",
                "wait",
                serde_json::json!({"until":{"Any":[]},"max_ticks":1}),
                PredicateDocument::Any(vec![]),
                PredicateDocument::All(vec![]),
            )],
        );
        document.roles[0].sequences[0].steps = vec![
            test_step(
                "root-acquire-step",
                "acquire",
                serde_json::json!({"recipe":"test:trace-child"}),
                PredicateDocument::Fact {
                    kind: "has_item".to_owned(),
                    version: 1,
                    args: serde_json::json!({"obj":"egg"}),
                },
                PredicateDocument::All(vec![]),
            ),
            test_step(
                "later-begin-failure",
                "make",
                serde_json::json!({
                    "loc":{"name":"spinningwheel","op":"Spin"},
                    "anchor":{"tile":[2982,3315,0],"source":"test"},
                    "product":"ball_of_wool",
                    "qty":{"progress":{"quest":"sheep","flag":"sheep:balls_to_go"}}
                }),
                PredicateDocument::Any(vec![]),
                PredicateDocument::All(vec![]),
            ),
        ];

        let (mut script, mut snapshot) = status_fixture(document);
        snapshot.seed_inventory(Vec::new(), 28);
        let mut ledger = None;
        let mut output = StatusCapture::default();
        for tick in 1..=80 {
            with_tick_output(&snapshot, &mut ledger, tick, &mut output, |native| {
                script.tick(native).unwrap();
            });
            if script.fail_streak > 0 {
                let egg = script.selected.item_by_alias("egg").unwrap();
                snapshot.seed_inventory(
                    vec![api::snapshot::ItemView {
                        def: api::obj_names::ItemDefView {
                            id: egg.id,
                            name: Some("Egg".into()),
                            stackable: false,
                            members: false,
                            base_value: 0,
                            noted: false,
                            certificate_link: -1,
                            certificate_template: -1,
                        },
                        container: api::snapshot::ItemContainer::Inventory,
                        action_family: api::snapshot::ItemActionFamily::Held,
                        slot: 0,
                        count: 1,
                        actions: Vec::new(),
                        component_id: 0,
                    }],
                    28,
                );
                break;
            }
        }
        assert_eq!(script.fail_streak, 1, "the acquire child failure occurred");

        for tick in 81..=120 {
            with_tick_output(&snapshot, &mut ledger, tick, &mut output, |native| {
                script.tick(native).unwrap();
            });
            if script.parked {
                break;
            }
        }
        assert!(script.parked, "the later begin failures must park");
        let status = output.0.last().expect("parked status was published");
        assert_eq!(status_text(status, "step_id"), "later-begin-failure");
        assert_eq!(status_text(status, "child_recipe_id"), "");
        assert_eq!(status_text(status, "child_step_id"), "");
    }

    #[test]
    fn runner_trace_records_skip_settle_failure_park_and_collapsed_retries() {
        use super::super::families::tests::with_tick_output;
        use super::super::path::PredicateDocument;

        let mut document = super::super::compile::decode_cook().unwrap();
        document.roles[0].sequences[0].steps = vec![
            test_step(
                "skip-not-started",
                "wait",
                serde_json::json!({"until":{"All":[]},"max_ticks":1}),
                PredicateDocument::Fact {
                    kind: "quest_colour".to_owned(),
                    version: 1,
                    args: serde_json::json!({"quest":"cook","is":"not_started"}),
                },
                PredicateDocument::All(vec![]),
            ),
            test_step(
                "settled-step",
                "wait",
                serde_json::json!({"until":{"Fact":{"kind":"has_item","version":1,"args":{"obj":"egg"}}},"max_ticks":10}),
                PredicateDocument::Fact {
                    kind: "has_item".to_owned(),
                    version: 1,
                    args: serde_json::json!({"obj":"egg"}),
                },
                PredicateDocument::All(vec![]),
            ),
            test_step(
                "retry-step",
                "wait",
                serde_json::json!({"until":{"Any":[]},"max_ticks":1}),
                PredicateDocument::Any(vec![]),
                PredicateDocument::Any(vec![]),
            ),
        ];
        let (mut script, mut snapshot) = status_fixture(document);
        snapshot.seed_inventory(Vec::new(), 27);
        script.stage = None;
        script.needs_read = true;
        let egg_id = script.selected.item_by_alias("egg").unwrap().id;
        let mut seeded_egg = false;
        let mut ledger = None;
        let mut output = TraceCapture::default();
        for tick in 1..=40 {
            with_tick_output(&snapshot, &mut ledger, tick, &mut output, |native| {
                script.tick(native).unwrap();
            });
            if !seeded_egg && script.step.is_some() {
                snapshot.seed_inventory(
                    vec![api::snapshot::ItemView {
                        def: api::obj_names::ItemDefView {
                            id: egg_id,
                            name: Some("Egg".into()),
                            stackable: false,
                            members: false,
                            base_value: 0,
                            noted: false,
                            certificate_link: -1,
                            certificate_template: -1,
                        },
                        container: api::snapshot::ItemContainer::Inventory,
                        action_family: api::snapshot::ItemActionFamily::Held,
                        slot: 0,
                        count: 1,
                        actions: Vec::new(),
                        component_id: 0,
                    }],
                    28,
                );
                seeded_egg = true;
            }
            if script.parked {
                break;
            }
        }
        assert!(script.parked, "the repeated wait failures must park");
        for tick in 41..=43 {
            with_tick_output(&snapshot, &mut ledger, tick, &mut output, |native| {
                script.tick(native).unwrap();
            });
        }

        let messages: Vec<&str> = output
            .logs
            .iter()
            .map(|(_, message)| message.as_str())
            .collect();
        assert_eq!(messages[0], "quester cook: run start stage=unknown");
        assert_eq!(messages[1], "quester cook: stage unknown → cook:0");
        assert!(messages.iter().any(|line| {
            line.contains("step skip-not-started skipped:")
                && line.contains("quest_colour")
                && line.contains("evaluated true")
        }));
        assert!(
            messages
                .iter()
                .any(|line| line == &"quester cook: stage cook:0 step settled-step (wait) settled"),
            "trace logs: {messages:#?}"
        );
        assert!(messages.iter().any(|line| {
            line == &"quester cook: stage cook:0 step retry-step (wait) failed: wait exhausted"
        }));
        assert_eq!(
            messages
                .iter()
                .filter(|line| line.contains("step retry-step (wait) begin"))
                .count(),
            2,
            "one original and one retry-count summary should represent all begins"
        );
        assert_eq!(
            messages
                .iter()
                .filter(|line| line.contains("step retry-step (wait) failed: wait exhausted"))
                .count(),
            2,
            "one original and one retry-count summary should represent all failures"
        );
        assert!(messages.last().unwrap().contains("park: wait exhausted"));
        assert_eq!(
            messages
                .iter()
                .filter(|line| line.contains(": park: "))
                .count(),
            1,
            "a parked run logs its park once, not on every later tick: {messages:#?}"
        );
        assert!(output.logs.iter().any(|(level, message)| {
            *level == api::hostlog::Level::Warn && message.contains("repeated 4 additional times")
        }));
    }

    #[test]
    fn acquisition_child_failure_is_visible_in_parked_status() {
        use super::super::families::tests::with_tick_output;
        use super::super::path::{PredicateDocument, StepDocument};

        let mut document = super::super::compile::decode_cook().unwrap();
        document.quest.as_mut().unwrap().acquire.insert(
            "test:child-failure".into(),
            vec![StepDocument {
                id: FactKey::new("child-failing-step"),
                kind: "wait".into(),
                version: 1,
                args: serde_json::json!({"until":{"Any":[]},"max_ticks":1}),
                comment: None,
                advances: Some(false),
                skip_if: PredicateDocument::Any(vec![]),
                settle: PredicateDocument::All(vec![]),
            }],
        );
        document.roles[0].sequences[0].steps = vec![StepDocument {
            id: FactKey::new("root-acquire-step"),
            kind: "acquire".into(),
            version: 1,
            args: serde_json::json!({"recipe":"test:child-failure"}),
            comment: None,
            advances: Some(false),
            skip_if: PredicateDocument::Any(vec![]),
            settle: PredicateDocument::All(vec![]),
        }];
        let (mut script, snapshot) = status_fixture(document);
        let mut ledger = None;
        let mut output = TraceCapture::default();
        for tick in 1..=40 {
            with_tick_output(&snapshot, &mut ledger, tick, &mut output, |native| {
                script.tick(native).unwrap();
            });
            if script.parked {
                break;
            }
        }

        assert!(
            script.parked,
            "the repeated child failures must park the run"
        );
        let status = output.statuses.last().expect("parked status was published");
        assert_eq!(status.phase, NativePhase::Blocked);
        assert!(status.fields.iter().any(|field| {
            field.key == "step_id"
                && field.value == StatusValue::Text(Arc::from("root-acquire-step"))
        }));
        assert!(status.fields.iter().any(|field| {
            field.key == "child_recipe_id"
                && field.value == StatusValue::Text(Arc::from("test:child-failure"))
        }));
        assert!(status.fields.iter().any(|field| {
            field.key == "child_step_id"
                && field.value == StatusValue::Text(Arc::from("child-failing-step"))
        }));
        assert!(status.fields.iter().any(|field| {
            field.key == "last_failure"
                && field.value == StatusValue::Text(Arc::from("wait exhausted"))
        }));
        assert_eq!(
            status.failure.as_ref().unwrap().message.as_ref(),
            "wait exhausted"
        );
        assert!(output.logs.iter().any(|(_, line)| {
            line.contains("recipe test:child-failure child child-failing-step begin")
        }));
        assert!(output.logs.iter().any(|(_, line)| {
            line.contains(
                "recipe test:child-failure child child-failing-step failed: wait exhausted",
            )
        }));
        assert!(output.logs.iter().any(|(_, line)| {
            line.contains(
                "park context step=root-acquire-step child_recipe=test:child-failure child=child-failing-step",
            )
        }));
    }

    #[test]
    fn settle_timeout_park_keeps_timed_out_step_in_status() {
        use super::super::families::tests::with_tick_output;
        use super::super::path::{PredicateDocument, StepDocument};

        let mut document = super::super::compile::decode_cook().unwrap();
        let wait_step = |id: &str, skip_if, settle| StepDocument {
            id: FactKey::new(id),
            kind: "wait".into(),
            version: 1,
            args: serde_json::json!({"until":{"All":[]},"max_ticks":100}),
            comment: None,
            advances: Some(false),
            skip_if,
            settle,
        };
        document.roles[0].sequences[0].steps = vec![
            wait_step(
                "settle-skipped-step",
                PredicateDocument::Fact {
                    kind: "has_item".into(),
                    version: 1,
                    args: serde_json::json!({"obj":"egg"}),
                },
                PredicateDocument::All(vec![]),
            ),
            wait_step(
                "settle-timeout-step",
                PredicateDocument::Any(vec![]),
                PredicateDocument::Any(vec![]),
            ),
        ];
        let (mut script, mut snapshot) = status_fixture(document);
        let egg_id = script.selected.item_by_alias("egg").unwrap().id;
        snapshot.seed_inventory(
            vec![api::snapshot::ItemView {
                def: api::obj_names::ItemDefView {
                    id: egg_id,
                    name: Some("Egg".into()),
                    stackable: false,
                    members: false,
                    base_value: 0,
                    noted: false,
                    certificate_link: -1,
                    certificate_template: -1,
                },
                container: api::snapshot::ItemContainer::Inventory,
                action_family: api::snapshot::ItemActionFamily::Held,
                slot: 0,
                count: 1,
                actions: Vec::new(),
                component_id: 0,
            }],
            28,
        );
        let mut ledger = None;
        let mut output = StatusCapture::default();
        for tick in 1..=100 {
            with_tick_output(&snapshot, &mut ledger, tick, &mut output, |native| {
                script.tick(native).unwrap();
            });
            if script.parked {
                break;
            }
        }

        assert!(script.parked, "repeated settle timeouts must park the run");
        assert_eq!(script.step_index, 0, "timeout retry cursor still restarts");
        let status = output.0.last().expect("parked status was published");
        assert_eq!(status.phase, NativePhase::Blocked);
        assert!(status
            .fields
            .iter()
            .any(|field| { field.key == "step_index" && field.value == StatusValue::Integer(1) }));
        assert!(status.fields.iter().any(|field| {
            field.key == "step_id"
                && field.value == StatusValue::Text(Arc::from("settle-timeout-step"))
        }));
        assert!(status.fields.iter().any(|field| {
            field.key == "last_failure"
                && field.value == StatusValue::Text(Arc::from("step settle timeout"))
        }));
        assert_eq!(
            status.failure.as_ref().unwrap().message.as_ref(),
            "step settle timeout"
        );
    }

    #[test]
    fn quester_reports_walk_evidence_as_a_terminal_block() {
        let (mut script, _) = fixture();
        let gates: Arc<[api::selected::QuestGate]> =
            Arc::from([api::selected::QuestGate::Complete(FactKey::new(
                "test-quest",
            ))]);
        script.record_failure(ActionError::NeedsEvidence(gates));
        assert_eq!(script.blocked_failure().code.as_ref(), "needs-evidence");
        script.clear_last_error();
        assert!(script.last_error.is_none());
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
        document.quest.as_mut().unwrap().owns_inventory = true;
        let step = &mut document.roles[0].sequences[0].steps[0];
        step.kind = "talk".into();
        step.args = serde_json::json!({"npc": "cook"});
        step.advances = Some(true);
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
            Arc::new(api::named_banks::NamedBankFacts::empty()),
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
            document.quest.as_mut().unwrap().owns_inventory = true;
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
                step.settle = super::super::path::PredicateDocument::Any(Vec::new());
                step.advances = Some(false);
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
                Arc::new(api::named_banks::NamedBankFacts::empty()),
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
    fn unprotectable_walk_publishes_its_cause_and_keeps_working() {
        use super::super::families::tests::{with_tick, with_tick_output};
        use crate::native::{HostEffect, WalkEvent, WalkEventKind};
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
        document.quest.as_mut().unwrap().owns_inventory = true;
        let step = &mut document.roles[0].sequences[0].steps[0];
        step.kind = "walk".into();
        step.args = serde_json::json!({
            "tile": [3103, 3163, 2], "source": "test fixture", "radius": 6, "guard": "protect"
        });
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
            data,
            quests,
            Arc::new(api::named_banks::NamedBankFacts::empty()),
        );
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_quest_statuses(
            vec![QuestStatusView {
                name: "Cook's Assistant".into(),
                component_id: 0,
                colour: 0xf80000,
            }],
            true,
        );
        let mut ledger = None;
        let mut queued_at = None;
        for tick in 1..=8 {
            with_tick(&snapshot, &mut ledger, tick, |native| {
                assert_eq!(script.tick(native).unwrap(), ScriptFlow::Continue);
            });
            if ledger.as_ref().is_some_and(|ledger| {
                ledger
                    .outbox
                    .iter()
                    .any(|action| matches!(action.effect, HostEffect::Walk(_)))
            }) {
                queued_at = Some(tick);
                break;
            }
        }
        let tick = queued_at.expect("authored walk queued") + 1;
        let action = &ledger.as_ref().unwrap().outbox[0];
        let authority = action.authority();
        let event = WalkEvent {
            request_id: action.request_id.get(),
            evidence: api::quest_progress::EvidenceStamp {
                run: action.run(),
                tick,
                sequence: tick,
            },
            kind: WalkEventKind::Unprotectable {
                protect: crate::combat::GuardProtect::Missiles,
            },
            detail: Arc::from("Prayer 40 needed for Protect from Missiles"),
        };
        ledger.as_mut().unwrap().walk_events.push(event);
        let cursor = (script.seq_index, script.step_index);
        let mut capture = Capture::default();
        for at in tick..tick + 3 {
            with_tick_output(&snapshot, &mut ledger, at, &mut capture, |native| {
                assert_eq!(script.tick(native).unwrap(), ScriptFlow::Continue);
            });
        }
        let status = capture.0.last().expect("warning changes status");
        assert_eq!(status.phase, NativePhase::Working);
        assert!(status.failure.is_none());
        assert!(status.fields.iter().any(|field| {
            field.label == "Walk protection"
                && field.value
                    == StatusValue::Text(Arc::from("Prayer 40 needed for Protect from Missiles"))
        }));
        assert!(!script.parked && authority.live());
        assert_eq!((script.seq_index, script.step_index), cursor);
        assert_eq!(ledger.as_ref().unwrap().outbox.len(), 1);
    }

    #[test]
    fn user_input_walk_blocks_without_attempts_or_repeated_work() {
        use super::super::families::tests::{post_user_input_walk_receipt, with_tick};
        use api::snapshot::{GameSnapshot, QuestStatusView};

        let data = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
        let quests = Arc::new(QuestCatalog::from_identity(data.quest_identity()).unwrap());
        let mut document = super::super::compile::decode_cook().unwrap();
        document.quest.as_mut().unwrap().owns_inventory = true;
        let step = &mut document.roles[0].sequences[0].steps[0];
        step.kind = "walk".into();
        step.args =
            serde_json::json!({"tile": [3200, 3200, 0], "source": "test fixture", "radius": 1});
        step.advances = Some(true);
        step.skip_if = super::super::path::PredicateDocument::Any(vec![]);
        step.settle = super::super::path::PredicateDocument::All(vec![]);
        let path =
            super::super::compile::compile_uncached_for_test(&document, &data, &quests).unwrap();
        let run = RunKey {
            slot: 1,
            run: 1,
            session: 1,
        };
        let mut script = Quester::new(
            run,
            path,
            Arc::clone(&data),
            quests,
            Arc::new(api::named_banks::NamedBankFacts::empty()),
        );
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_quest_statuses(
            vec![QuestStatusView {
                name: "Cook's Assistant".into(),
                component_id: 0,
                colour: 0xf80000,
            }],
            true,
        );
        let mut ledger = None;
        let mut walk_started = None;
        for tick in 1..=8 {
            with_tick(&snapshot, &mut ledger, tick, |native| {
                assert!(matches!(script.tick(native).unwrap(), ScriptFlow::Continue));
            });
            if ledger.as_ref().is_some_and(|ledger| {
                ledger
                    .outbox
                    .iter()
                    .any(|action| matches!(&action.effect, crate::native::HostEffect::Walk(_)))
            }) {
                walk_started = Some(tick);
                break;
            }
        }
        let poll_tick = walk_started.expect("Quester must start the authored walk") + 1;
        script.attempts = 4;
        script.fail_streak = 4;
        let cursor = (script.seq_index, script.step_index);
        post_user_input_walk_receipt(&mut ledger, poll_tick);

        for tick in poll_tick..=poll_tick + 2 {
            assert!(matches!(
                with_tick(&snapshot, &mut ledger, tick, |native| {
                    script.tick(native).unwrap()
                }),
                ScriptFlow::Blocked(_)
            ));
            assert_eq!((script.seq_index, script.step_index), cursor);
            assert_eq!(script.attempts, 4);
            assert_eq!(script.fail_streak, 4);
            if tick == poll_tick {
                ledger.as_mut().unwrap().outbox.clear();
            }
        }
        assert!(ledger.as_ref().unwrap().outbox.is_empty());
    }
    #[test]
    fn missing_partner_admission_expires_with_clear_stop_start_reason() {
        use crate::quester::pair::{
            AccountKey, Gang, PairError, PairFrame, PairRegistration, PairRequest, PairSettings,
            PairStep, PairToken, PartnerDeclaration, PartnerRole, QuestPairPort, RoleReceipt,
        };
        use std::task::Poll;
        use std::time::Instant;

        struct MissingPeer;
        static PORT: MissingPeer = MissingPeer;
        impl QuestPairPort for MissingPeer {
            fn shared(&self) -> Arc<dyn QuestPairPort> {
                Arc::new(Self)
            }
            fn observe(&self, _: PairRegistration, _: PairFrame<'_>) {}
            fn invalidate(&self, _: RunKey) {}
            fn busy(&self) -> bool {
                false
            }
            fn world_changed(&self, _: &str, _: u16) {}
            fn settings(&self, _: RunKey) -> Result<PairSettings, PairError> {
                Ok(PairSettings {
                    partner: Some(AccountKey(Arc::from("bob"))),
                    gang: Some(Gang::Phoenix),
                })
            }
            fn observe_gang(
                &self,
                _: &api::quest_progress::JournalRead,
            ) -> Result<Knowledge<Option<Gang>>, PairError> {
                Ok(Knowledge::Known(None))
            }
            fn gang(
                &self,
                caller: RunKey,
            ) -> Result<(Knowledge<Option<Gang>>, EvidenceStamp), PairError> {
                Ok((
                    Knowledge::Known(None),
                    EvidenceStamp {
                        run: caller,
                        tick: 1,
                        sequence: 1,
                    },
                ))
            }
            fn partner_item_count(
                &self,
                _: EvidenceStamp,
                _: &crate::quester::pair::PairItemRequest,
            ) -> Result<i32, PairError> {
                Ok(0)
            }
            fn waiting(&self, _: RunKey) -> bool {
                false
            }
            fn token(&self, _: RunKey, _: &FactKey) -> Result<PairToken, PairError> {
                Err(PairError::NotReady)
            }
            fn register_action(
                &self,
                _: &PairToken,
                _: RunKey,
                _: crate::native::ActionRevoker,
            ) -> Result<(), PairError> {
                Ok(())
            }
            fn begin(&self, _: PairRequest) -> Result<PairToken, PairError> {
                Err(PairError::PartnerNotInPlay)
            }
            fn poll(&self, _: &PairToken, _: RunKey) -> Poll<Result<PairStep, PairError>> {
                Poll::Ready(Err(PairError::PartnerNotInPlay))
            }
            fn report(&self, _: &PairToken, _: RoleReceipt) -> Result<(), PairError> {
                Ok(())
            }
            fn trade_ready(
                &self,
                _: &PairToken,
                _: RunKey,
                _: bool,
                _: EvidenceStamp,
            ) -> Result<bool, PairError> {
                Ok(false)
            }
            fn gameplay_progress(&self, _: RunKey, _: EvidenceStamp, _: Instant) {}
            fn cancel(&self, _: &PairToken) {}
        }

        let selected = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
        let quests = Arc::new(QuestCatalog::from_identity(selected.quest_identity()).unwrap());
        let mut document = super::super::compile::decode_cook().unwrap();
        document.partner = Some(PartnerDeclaration {
            protocol: FactKey::new("arrav"),
            roles: [
                PartnerRole {
                    id: FactKey::new("phoenix"),
                    gang: Gang::Phoenix,
                },
                PartnerRole {
                    id: FactKey::new("blackarm"),
                    gang: Gang::BlackArm,
                },
            ],
        });
        let mut phoenix = document.roles[0].clone();
        phoenix.role = Some(FactKey::new("phoenix"));
        let mut blackarm = document.roles[0].clone();
        blackarm.role = Some(FactKey::new("blackarm"));
        document.roles = vec![phoenix, blackarm];
        let bytes = serde_json::to_vec(&document).unwrap();
        let path = super::super::compile::prepare_for_test({
            let selected = Arc::clone(&selected);
            let quests = Arc::clone(&quests);
            move |worker| {
                super::super::compile::compile_path_for_gang(
                    &bytes,
                    &selected,
                    &quests,
                    worker,
                    Some(Gang::Phoenix),
                )
            }
        })
        .unwrap();
        let run = RunKey {
            slot: 1,
            run: 1,
            session: 1,
        };
        let mut script = Quester::new(
            run,
            path,
            Arc::clone(&selected),
            Arc::clone(&quests),
            Arc::new(api::named_banks::NamedBankFacts::empty()),
        );
        let facts = quests.quest("cook").unwrap();
        let mut snapshot = api::snapshot::GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_quest_statuses(
            vec![api::snapshot::QuestStatusView {
                name: facts.display.to_string(),
                component_id: 1,
                colour: 0xf80000,
            }],
            true,
        );
        let mut ledger = None;

        assert_eq!(
            super::super::families::tests::with_tick(&snapshot, &mut ledger, 1, |tick| {
                tick.pairs = Some(&PORT);
                script.tick(tick).unwrap()
            }),
            ScriptFlow::Continue
        );

        let _ = super::super::families::tests::with_tick(&snapshot, &mut ledger, 1002, |tick| {
            tick.pairs = Some(&PORT);
            script.tick(tick).unwrap()
        });
        assert!(
            script.parked,
            "the 10-minute active-time bound must park the Quester"
        );
        assert_eq!(
            script.blocked_failure().message.as_ref(),
            "partner admission timed out; Stop and Start both accounts"
        );
    }

    #[test]
    fn quest_pair_r2_phase_begin_busy_wait_has_an_active_time_bound() {
        use super::super::compile::StepPlan;
        struct BusyPhase;
        impl StepPlan for BusyPhase {
            fn begin(&self, _: &mut StepContext<'_, '_>) -> Result<Box<dyn StepRun>, ActionError> {
                Err(ActionError::Busy)
            }
        }
        struct NeverSkip;
        impl super::super::compile::PredicatePlan for NeverSkip {
            fn evaluate(&self, _: &PredicateContext<'_, '_>) -> Truth {
                Truth::False
            }
        }
        let (mut script, snapshot) = fixture();
        script.needs_read = false;
        script.stage = Some(script.path.colour_not_started.clone());
        let path = Arc::get_mut(&mut script.path).unwrap();
        let step = &mut path.sequences[0].steps[0];
        step.kind = Arc::from("partner");
        step.skip_if = Arc::new(NeverSkip);
        step.plan = Arc::new(BusyPhase);
        let mut ledger = None;
        for tick in 1..=10 {
            super::super::families::tests::with_tick(&snapshot, &mut ledger, tick, |native| {
                assert_eq!(script.tick(native).unwrap(), ScriptFlow::Continue);
            });
            assert!(
                !script.parked,
                "a retryable pair begin must not consume failed attempts"
            );
            assert_eq!(script.attempts, 0);
        }
        super::super::families::tests::with_tick(&snapshot, &mut ledger, 1001, |native| {
            script.tick(native).unwrap();
        });
        assert!(
            script.parked,
            "a stuck phase begin has a ten-minute active bound"
        );
        assert_eq!(
            script.blocked_failure().message.as_ref(),
            "partner phase begin timed out after 10 minutes of active time; Stop and Start both accounts"
        );
    }
    struct BankSkipFixture {
        script: Quester,
        snapshot: api::snapshot::GameSnapshot,
        ledger: Option<Box<crate::native::ledger::Ledger>>,
        bank: api::named_banks::NamedBank,
        bank_tile: api::snapshot::WorldTile,
        stock: Vec<api::snapshot::ItemView>,
        selected_bank: bool,
        opened_bank: bool,
    }

    impl BankSkipFixture {
        fn drive(&mut self, tick: u64, output: &mut dyn NativeOutput) -> ScriptFlow {
            let flow = super::super::families::tests::with_tick_output(
                &self.snapshot,
                &mut self.ledger,
                tick,
                output,
                |native| self.script.tick(native).unwrap(),
            );
            let bank_pick_pending = self.ledger.as_ref().is_some_and(|ledger| {
                ledger.outbox.first().is_some_and(|action| {
                    matches!(&action.effect, crate::native::HostEffect::BankPick(_))
                })
            });
            let open_stand_pending = self.ledger.as_ref().is_some_and(|ledger| {
                ledger.outbox.first().is_some_and(|action| {
                    matches!(
                        &action.effect,
                        crate::native::HostEffect::Interaction(
                            crate::shim::InteractReq::OpenStand { .. }
                        )
                    )
                })
            });
            if bank_pick_pending {
                let action = self.ledger.as_mut().unwrap().outbox.remove(0);
                let authority = action.authority();
                self.ledger.as_mut().unwrap().complete_bank_pick(
                    &authority,
                    crate::bank::BankPickReceipt {
                        request_id: authority.request_id().get(),
                        evidence: api::quest_progress::EvidenceStamp {
                            run: authority.run(),
                            tick,
                            sequence: tick,
                        },
                        selected: crate::bank::SelectedBank {
                            bank_index: 0,
                            access_tile: self.bank_tile,
                            kind: crate::bank::PickKind::Reachable,
                            access: Some(Arc::new(crate::bank::BankStandAccess {
                                bank: self.bank,
                                stand_tile: self.bank_tile,
                                kind: crate::bank::AccessKind::Booth,
                                stand_op: 1,
                                name: None,
                                choose: None,
                            })),
                        },
                    },
                );
                self.selected_bank = true;
            } else if open_stand_pending {
                let action = self.ledger.as_mut().unwrap().outbox.remove(0);
                let authority = action.authority();
                self.ledger.as_mut().unwrap().complete_interaction(
                    &authority,
                    crate::native::InteractionReceipt {
                        request_id: authority.request_id().get(),
                        evidence: api::quest_progress::EvidenceStamp {
                            run: authority.run(),
                            tick,
                            sequence: tick,
                        },
                        accepted: true,
                        chat_since: 0,
                    },
                );
                self.snapshot
                    .seed_bank_observation(1, tick, Some(self.stock.clone()), Vec::new());
                self.opened_bank = true;
            } else if self
                .ledger
                .as_ref()
                .is_some_and(|ledger| !ledger.outbox.is_empty())
            {
                panic!("unexpected bank-scan action");
            }
            flow
        }
    }

    fn bank_skip_fixture(bank_count: i32, nested_recipe: bool) -> BankSkipFixture {
        use super::super::path::PredicateDocument;
        use api::snapshot::{ItemActionFamily, ItemContainer, LocLayer, LocView, WorldTile};

        let mut document = super::super::compile::decode_cook().unwrap();
        document.quest.as_mut().unwrap().owns_inventory = true;
        let bank_has = PredicateDocument::Fact {
            kind: "bank_has".into(),
            version: 1,
            args: serde_json::json!({"obj":"logs","qty":1}),
        };
        let nested_bank_has = PredicateDocument::All(vec![PredicateDocument::Any(vec![bank_has])]);
        let wait = serde_json::json!({"until":{"Any":[]},"max_ticks":1000});
        let bank_skip_steps = vec![
            test_step(
                "skip-if-bank-has-logs",
                "wait",
                wait.clone(),
                nested_bank_has,
                PredicateDocument::All(vec![]),
            ),
            test_step(
                "fallback-if-bank-has-logs",
                "wait",
                wait,
                PredicateDocument::Any(vec![]),
                PredicateDocument::All(vec![]),
            ),
        ];
        if nested_recipe {
            document
                .quest
                .as_mut()
                .unwrap()
                .acquire
                .insert("test:bank-skip".to_owned(), bank_skip_steps);
            document.roles[0].sequences[0].steps = vec![test_step(
                "root-bank-skip-acquire",
                "acquire",
                serde_json::json!({"recipe":"test:bank-skip"}),
                PredicateDocument::Any(vec![]),
                PredicateDocument::All(vec![]),
            )];
        } else {
            document.roles[0].sequences[0].steps = bank_skip_steps;
        }
        let (mut script, mut snapshot) = status_fixture(document);
        let bank_tile = WorldTile {
            x: 3092,
            z: 3242,
            level: 0,
        };
        let bank = api::named_banks::NamedBank::new("Runner predicate bank", bank_tile);
        script.banks = Arc::new(api::named_banks::NamedBankFacts::from_banks(vec![bank]));
        let logs = script.selected.item_by_alias("logs").unwrap();
        let stock = if bank_count > 0 {
            vec![api::snapshot::ItemView {
                def: api::obj_names::ItemDefView {
                    id: logs.id,
                    name: Some("Logs".into()),
                    stackable: false,
                    members: false,
                    base_value: 0,
                    noted: false,
                    certificate_link: -1,
                    certificate_template: -1,
                },
                container: ItemContainer::Bank,
                action_family: ItemActionFamily::Component,
                slot: 0,
                count: bank_count,
                actions: vec![Some("Withdraw-1".into())],
                component_id: 7,
            }]
        } else {
            Vec::new()
        };
        snapshot.seed_inventory(Vec::new(), 28);
        snapshot.seed_local_player(super::super::families::tests::local_player(bank_tile));
        snapshot.seed_locs(vec![LocView {
            id: 2213,
            name: Some("Bank booth".into()),
            actions: vec![Some("Use-quickly".into())],
            tile: bank_tile,
            distance: 0,
            typecode: 0,
            info: 0,
            description: None,
            layer: LocLayer::GroundDecoration,
            shape: 0,
            angle: 0,
            width: 1,
            length: 1,
            footprint_width: 1,
            footprint_length: 1,
            block_walk: false,
            block_range: false,
            active: true,
            animation: -1,
            map_function: -1,
            map_scene: -1,
            force_approach: 0,
        }]);
        BankSkipFixture {
            script,
            snapshot,
            ledger: None,
            bank,
            bank_tile,
            stock,
            selected_bank: false,
            opened_bank: false,
        }
    }

    #[test]
    fn nested_bank_has_skip_starts_real_scan_and_receipt_selects_step() {
        for (bank_count, expected_step) in [
            (0, "skip-if-bank-has-logs"),
            (1, "fallback-if-bank-has-logs"),
        ] {
            let mut fixture = bank_skip_fixture(bank_count, false);
            let mut output = StatusCapture::default();
            assert!(!fixture.script.bank.known());

            fixture.drive(1, &mut output);
            assert_eq!(
                fixture.script.provisioner.status().phase,
                super::super::provision::ProvisionPhase::Scanning,
                "unknown nested bank_has must start scanning immediately"
            );
            assert!(output
                .0
                .iter()
                .any(|status| status_text(status, "provision") == "Scanning"));
            assert!(!fixture.script.parked);

            for tick in 2..=64 {
                fixture.drive(tick, &mut output);
                if fixture.script.bank.known() && fixture.script.step.is_some() {
                    break;
                }
            }

            assert!(fixture.selected_bank, "the real scan selects a bank");
            assert!(
                fixture.opened_bank,
                "the real scan opens the selected stand"
            );
            assert!(fixture.script.bank.known(), "the scan publishes a receipt");
            let logs_id = fixture.script.selected.item_by_alias("logs").unwrap().id;
            assert_eq!(fixture.script.bank.count(logs_id), Some(bank_count));
            assert_eq!(
                fixture.script.current_step().map(|step| step.id.0.as_ref()),
                Some(expected_step),
                "the observed bank receipt resolves the nested skip predicate"
            );
            assert!(!fixture.script.parked);
        }
    }

    #[test]
    fn nested_acquire_bank_has_skip_starts_real_scan_and_receipt_selects_child() {
        for (bank_count, expected_step) in [
            (0, "skip-if-bank-has-logs"),
            (1, "fallback-if-bank-has-logs"),
        ] {
            let mut fixture = bank_skip_fixture(bank_count, true);
            let mut output = StatusCapture::default();
            for tick in 1..=3 {
                fixture.drive(tick, &mut output);
            }

            assert_eq!(
                fixture.script.provisioner.status().phase,
                super::super::provision::ProvisionPhase::Scanning,
                "an unknown nested acquisition bank_has must start scanning"
            );
            assert!(output
                .0
                .iter()
                .any(|status| status_text(status, "provision") == "Scanning"));

            for tick in 4..=64 {
                fixture.drive(tick, &mut output);
                let status = output.0.last().unwrap();
                if fixture.script.bank.known()
                    && status_text(status, "child_step_id") == expected_step
                {
                    break;
                }
            }

            assert!(fixture.selected_bank, "the child scan selects a real bank");
            assert!(
                fixture.opened_bank,
                "the child scan opens the selected stand"
            );
            assert!(
                fixture.script.bank.known(),
                "the child scan publishes a receipt"
            );
            let logs_id = fixture.script.selected.item_by_alias("logs").unwrap().id;
            assert_eq!(fixture.script.bank.count(logs_id), Some(bank_count));
            assert_eq!(
                status_text(output.0.last().unwrap(), "child_step_id"),
                expected_step,
                "the observed receipt selects the expected acquisition child"
            );
            assert!(!fixture.script.parked);
        }
    }

    #[test]
    fn non_bank_unknown_skip_keeps_waiting_without_starting_bank_scan() {
        use super::super::path::PredicateDocument;

        let mut document = super::super::compile::decode_cook().unwrap();
        document.quest.as_mut().unwrap().owns_inventory = true;
        document.roles[0].sequences[0].steps = vec![test_step(
            "wait-for-inventory-evidence",
            "wait",
            serde_json::json!({"until":{"Any":[]},"max_ticks":100}),
            PredicateDocument::All(vec![PredicateDocument::Fact {
                kind: "has_item".into(),
                version: 1,
                args: serde_json::json!({"obj":"egg"}),
            }]),
            PredicateDocument::All(vec![]),
        )];
        let (mut script, snapshot) = status_fixture(document);
        let mut ledger = None;
        let mut output = StatusCapture::default();
        for tick in 1..=32 {
            super::super::families::tests::with_tick_output(
                &snapshot,
                &mut ledger,
                tick,
                &mut output,
                |native| {
                    script.tick(native).unwrap();
                },
            );
            if script.parked {
                break;
            }
        }
        assert!(script.parked, "non-bank evidence retains the bounded park");
        assert_eq!(script.park_reason, "skip predicate evidence unavailable");
        assert_eq!(
            script.provisioner.status().phase,
            super::super::provision::ProvisionPhase::Ready
        );
        assert!(ledger
            .as_ref()
            .is_none_or(|ledger| ledger.outbox.is_empty()));
    }
}

#[cfg(test)]
#[path = "journal_runner_tests.rs"]
mod journal_tests;

#[cfg(test)]
#[path = "queue_runner_tests.rs"]
mod queue_tests;

#[cfg(test)]
#[path = "recovery_runner_tests.rs"]
mod recovery_tests;

#[cfg(test)]
#[path = "cook_bank_runner_tests.rs"]
mod cook_bank_tests;

#[cfg(test)]
#[path = "cook_dialogue_runner_tests.rs"]
mod cook_dialogue_tests;
