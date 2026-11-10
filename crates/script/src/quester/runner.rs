//! §3.2 tick loop with colour-first, boundary-triggered journal evidence.
use super::compile::{
    AcquisitionTraceOutcome, CompiledPath, CompiledStep, PredicateContext, StepAttemptBudget,
    StepAttemptReceipt, StepContext, StepOutcome, StepRun, StepTraceEvent,
};
use super::families::combat::CombatReceipt;
use super::families::dialogue::{
    page_open, Dialogue, DialogueArgs, DialogueOptions, DialogueTarget,
};
use super::progress::{quest_colour, resolve_colour, resolve_journal};
use super::provision::{
    BankCloser, BankLease, Closed, ProvisionEvent, ProvisionPhase, Provisioner,
};
use super::queue::QueueStatus;
use super::registry::PathSource;
use super::select::{select_with_skips, sequence_for_stage, SelectionDecision};
use super::watchdog::{Watchdog, WatchdogAction};
use crate::combat::{begin_clear_owned_prayers, ClearPrayers, Hygiene, RaisedPrayers};
use crate::gatherer::oneop::{OneOp, OneOpArgs};
use crate::native::death::{death_cap_exceeded, default_max_deaths, DeathLatch};
use crate::native::{
    ActionError, ActionHandle, Interrupt, NativeOutput, NativePhase, NativeTick, Script,
    ScriptFailure, ScriptFlow, ScriptStatus, StatusField, StatusValue, StopReason,
};
use crate::preparation::{self, PreparationInput, PreparationKind, PreparedTask};
use crate::quest_journal::{JournalMachine, JournalRequest, ROOT_289};
use crate::CompiledId;
use api::game_data::SelectedGameData;
use api::identity::{Cancellation, Scope, SlotKey};
use api::quest_facts::QuestCatalog;
use api::quest_progress::{EvidenceStamp, JournalRead, QuestProgress};
use api::selected::{FactKey, Knowledge, RunKey, Truth};
use api::snapshot::QuestListStatus;
use api::{DetectedRandom, RandomClaim};
use std::num::NonZeroU32;
use std::sync::Arc;
use std::task::Poll;

use std::fmt::{self, Write as _};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::time::Duration;

// A read has at most three transactions (each can emit at most one row
// click). Adoption consumes a transaction too, but never adds a click.
const JOURNAL_READ_ATTEMPTS: u8 = 3;
const JOURNAL_RETRY_QUIET_TICKS: u64 = 3;
/// Completed steps a single tick may carry into the next settle, read and
/// selection (TICK-FIX #5). Each pass needs a step that completed without
/// spending the tick's interaction event; the per-tick transition budget
/// bounds the native work underneath as well.
const SAME_TICK_PASSES: u8 = 3;
/// Provisioning outcomes one tick may re-plan after (TICK-FIX #7).
const PROVISION_REPLANS: u8 = 3;
/// Three 8-tick close operations fit within 24 open-page observations.
/// Refused begins and spent interaction events use the same bound.
const PARK_PAGE_LIMIT: u8 = 24;

/// A selected step's position, independent of its live machine handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StepSlot {
    in_prelude: bool,
    seq_index: usize,
    step_index: usize,
}

/// Survival owns this damage until selection names a different step.
#[derive(Debug, Clone, Copy, Default)]
struct SelectedHazard {
    slot: Option<StepSlot>,
    hazard: Option<crate::survival::StepHazard>,
    /// Whether hazard admission finished or the runner emitted an undeclared-damage warning.
    handled: bool,
}

impl SelectedHazard {
    fn select(&mut self, slot: StepSlot, hazard: Option<crate::survival::StepHazard>) {
        if self.slot == Some(slot) {
            return;
        }
        self.slot = Some(slot);
        self.hazard = hazard;
        self.handled = false;
    }

    fn clear(&mut self) {
        *self = Self::default();
    }
}

/// One provisioning poll as the runner sees it.
enum Provisioned {
    Ready,
    Wait,
    /// A bank receipt or a finished acquisition: re-plan.
    Landed,
}
/// Ticks an advancing combat step's progress read waits for its killed NPC to
/// leave the scene. The content writes quest progress from the NPC's death
/// queue after `npc_death` (`skill_combat/scripts/npc/npc_death.rs2:10-25`:
/// `npc_arrivedelay` up to 2 ticks, the death anim, `npc_delay(1)`,
/// `npc_del`), e.g. Priest in Peril's guardian dog sets `%priestperil` after
/// it (`area_mausoleum/scripts/temple_guardian.rs2:1-12`), while the combat
/// step completes on the observed zero health.
const KILL_READ_BOUND_TICKS: u64 = 6;

/// The NPC an advancing combat step killed, while it is still in the scene.
fn killed_npc_in_scene(tick: &NativeTick<'_>, outcome: Option<&StepOutcome>) -> Option<u16> {
    let receipt = outcome?
        .receipt
        .as_ref()?
        .as_any()
        .downcast_ref::<CombatReceipt>()?;
    if receipt.report.end != crate::combat::CombatEnd::Killed {
        return None;
    }
    let target = receipt.report.engaged?;
    (target.kind == api::snapshot::ActorKind::Npc && npc_in_scene(tick, target.index))
        .then_some(target.index)
}

fn npc_in_scene(tick: &NativeTick<'_>, index: u16) -> bool {
    tick.cx
        .snapshot()
        .npcs()
        .is_some_and(|npcs| npcs.value.iter().any(|npc| npc.index == usize::from(index)))
}

// Continue drains a single progress read may spend on chat pages no step
// owns, and the active time they may take together. A page that reopens past
// either parks with its root and text.
const JOURNAL_CONTINUE_DRAINS: u8 = 3;
const JOURNAL_DRAIN_WINDOW: Duration = Duration::from_secs(30);
const QUEUE_QUEST_STATUS_WAIT: Duration = Duration::from_secs(30);
// Pair admission uses the broker's ten-minute inactivity budget in active time.
const PAIR_ADMISSION_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const JOURNAL_RETRY_LIMIT_BUSY: &str =
    "journal read retry limit reached (journal remained busy during read)";
const JOURNAL_RETRY_LIMIT_OWNERSHIP_LOST: &str =
    "journal read retry limit reached (journal ownership repeatedly lost)";
const JOURNAL_RETRY_LIMIT_MODAL_TIMEOUT: &str =
    "journal read retry limit reached (journal modal repeatedly timed out)";

/// Whether the frame's bank memory is known (`Hint` or `Session`): the gate
/// for a predicate scan — an `Unknown` bank is the only one a trip can teach.
fn bank_known(tick: &NativeTick<'_>) -> bool {
    tick.cx
        .snapshot()
        .bank_memory()
        .is_some_and(api::bank_memory::BankMemory::known)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QuesterFailureKind {
    Other,
    ManualMovement,
    MaxDeaths,
    NeedsEvidence,
    HazardUnsurvivable,
}

struct ParkedStep {
    sequence_index: usize,
    step_index: usize,
    in_prelude: bool,
    id: Arc<str>,
}

/// Distinct trace lines a stage keeps before new lines are dropped. The floor
/// covers the run start and stages with no steps; a stage with steps gets
/// `RUN_TRACE_EVENT_PER_STEP × units + RUN_TRACE_EVENT_SLACK`, where a unit is
/// one of its own steps or one step of a recipe its `acquire` steps run.
const RUN_TRACE_EVENT_MIN: usize = 32;
/// Lines one trace unit can log: begin, skip and settled/failed, plus one
/// spare for a retried begin, a restart line or a skip predicate that waits.
const RUN_TRACE_EVENT_PER_STEP: usize = 4;
/// Lines for provisioning, the stage change itself and the run start.
const RUN_TRACE_EVENT_SLACK: usize = 8;
const RUN_TRACE_LINE_LIMIT: usize = 192;

/// The distinct-line budget of one stage, from its compiled trace units
/// (`CompiledSequence::trace_units`): every step line and every line of the
/// recipes its acquire steps run fits, and a short stage keeps the 32-line
/// floor.
fn stage_event_limit(units: usize) -> usize {
    RUN_TRACE_EVENT_MIN.max(
        units
            .saturating_mul(RUN_TRACE_EVENT_PER_STEP)
            .saturating_add(RUN_TRACE_EVENT_SLACK),
    )
}

/// One distinct trace line of the current stage: two independent 64-bit
/// digests of the line and how many times it repeated. The line text is not
/// kept here; only an entry that repeats keeps its text, in
/// `RunTrace::repeated`. A hit needs both digests, so a collision in one
/// cannot hide a different line or fold its count into another line's
/// summary; a real collision in both is accepted rather than checked.
struct TraceEntry {
    hash: u64,
    /// An FNV-1a digest where `hash` is SipHash-1-3: two functions, so one
    /// collision does not repeat in the other.
    check: u64,
    repeats: u32,
}

/// The text of a trace entry that repeated, kept so `flush_repeats` can print
/// the line with its count. A stage of distinct lines keeps none.
struct RepeatedLine {
    entry: usize,
    line: Box<str>,
}

struct RunTrace {
    entries: Vec<TraceEntry>,
    /// Texts of the current stage's entries that repeated; see `TraceEntry`.
    repeated: Vec<RepeatedLine>,
    /// Distinct lines the current stage keeps; see `stage_event_limit`.
    limit: usize,
    started: bool,
    truncated: bool,
    terminal_logged: bool,
}

impl Default for RunTrace {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            repeated: Vec::new(),
            limit: RUN_TRACE_EVENT_MIN,
            started: false,
            truncated: false,
            terminal_logged: false,
        }
    }
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

/// The table's identity for one trace line. `DefaultHasher` is deterministic
/// for a run, so equal lines always meet the same entry.
fn trace_hash(line: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    line.hash(&mut hasher);
    hasher.finish()
}

/// The second, independent digest of a trace line: 64-bit FNV-1a, where
/// `trace_hash` is SipHash-1-3. A repeat needs both to match, so neither
/// function's collision alone can hide a different line.
fn trace_check(line: &str) -> u64 {
    let mut check: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in line.bytes() {
        check ^= u64::from(byte);
        check = check.wrapping_mul(0x0000_0100_0000_01b3);
    }
    check
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
        let hash = trace_hash(&line);
        let check = trace_check(&line);
        if let Some(index) = self
            .entries
            .iter()
            .position(|entry| entry.hash == hash && entry.check == check)
        {
            let repeats = self.entries[index].repeats.saturating_add(1);
            self.entries[index].repeats = repeats;
            if repeats == 1 {
                // The first repeat keeps the line text this log call already
                // built; both digests matched, so it is the first line's text,
                // and `flush_repeats` prints it with the count.
                self.repeated.push(RepeatedLine {
                    entry: index,
                    line: line.into_boxed_str(),
                });
            }
            return;
        }
        if self.entries.len() >= self.limit {
            self.truncated = true;
            return;
        }
        output.log(level, &line);
        self.entries.push(TraceEntry {
            hash,
            check,
            repeats: 0,
        });
    }

    /// Start a stage's fresh budget, sized from its compiled trace units. The
    /// ending stage's folded repeats and truncation warning are emitted first;
    /// then its lines are dropped, so one bot's memory stays bounded by its
    /// largest stage and a later stage logs its lines even when an earlier
    /// one was truncated.
    fn begin_stage(&mut self, output: &mut dyn NativeOutput, units: usize) {
        self.flush_repeats(output);
        self.entries.clear();
        self.limit = stage_event_limit(units);
    }

    fn flush_repeats(&mut self, output: &mut dyn NativeOutput) {
        self.repeated
            .sort_unstable_by_key(|repeated| repeated.entry);
        for repeated in std::mem::take(&mut self.repeated) {
            let Some(entry) = self.entries.get_mut(repeated.entry) else {
                continue;
            };
            if entry.repeats == 0 {
                continue;
            }
            let (line, _) = trace_line(
                format_args!(
                    "{} (repeated {} additional times)",
                    repeated.line, entry.repeats
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

/// `modal root R (first text)`: the open chat page a journal park names.
fn chat_page_label(tick: &NativeTick<'_>) -> String {
    let Some(chat) = tick.cx.snapshot().chat_modal() else {
        return String::from("an unobserved chat page");
    };
    let mut label = format!("modal root {}", chat.value.root);
    if let Some(text) = chat.value.texts.iter().find(|text| !text.is_empty()) {
        let _ = write!(label, " ({text})");
    }
    label
}

fn trace_error_reason(error: &ActionError) -> &str {
    match error {
        ActionError::Unavailable(reason)
        | ActionError::Failed(reason)
        | ActionError::Blocked(reason) => reason.as_ref(),
        ActionError::BankDeposit(reason) => reason.message(),
        ActionError::NeedsEvidence(_) => "needs evidence",
        ActionError::CombatInterrupted => "dialogue interrupted by combat",
        ActionError::UserInput => super::families::MANUAL_MOVEMENT_MESSAGE,
        _ => "step error",
    }
}
fn quest_gate_evidence_detail(gates: &[api::selected::QuestGate]) -> Arc<str> {
    let mut detail = String::from("needs evidence:");
    for (index, gate) in gates.iter().enumerate() {
        if index > 0 {
            detail.push(';');
        }
        detail.push_str(" quest ");
        match gate {
            api::selected::QuestGate::Complete(quest) => {
                let _ = write!(detail, "{}; completion; evidence state unknown", quest.0);
            }
            api::selected::QuestGate::Window(window) => {
                let _ = write!(
                    detail,
                    "{}; signal {}; window; minimum ",
                    window.quest.0, window.signal.0
                );
                match window.values.min {
                    Some(minimum) => {
                        let _ = write!(detail, "{minimum}");
                    }
                    None => detail.push_str("none"),
                }
                detail.push_str("; maximum ");
                match window.values.max {
                    Some(maximum) => {
                        let _ = write!(detail, "{maximum}");
                    }
                    None => detail.push_str("none"),
                }
                detail.push_str("; evidence state unknown");
            }
        }
    }
    Arc::from(detail)
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
    /// Continuation driver for an unowned chat continue page that blocks the
    /// journal read; see `drain_unowned_continue`.
    journal_drain: Option<ActionHandle<Dialogue>>,
    journal_drains: u8,
    journal_drain_since: Option<Duration>,
    custom_reader: Option<Box<dyn StepRun>>,
    custom_read_after: Option<EvidenceStamp>,
    pair_admission: Option<Box<dyn StepRun>>,
    pair_admission_since: Option<Duration>,
    pair_begin_since: Option<Duration>,
    pair_admitted: bool,
    gang_reader: super::gang::GangRead,
    last_read: Option<Arc<JournalRead>>,
    journal_text: Option<Arc<str>>,
    read_requested: bool,
    selection_since: Option<Duration>,
    seq_index: usize,
    step_index: usize,
    /// Resume index for an `Ordered` sequence: past the last settled step,
    /// held on a failed one, zero after a restart or a sequence change.
    cursor: usize,
    step: Option<Box<dyn StepRun>>,
    survival: crate::survival::SurvivalStep,
    survival_action: Option<ActionHandle<OneOp>>,
    survival_deferred: bool,
    survival_interrupting: bool,
    selected_hazard: SelectedHazard,
    park_page_ticks: u8,
    park_page_seen: bool,
    park_bound_hp: i32,
    /// Freshness boundary for the active step; overwritten at each successful begin.
    step_after: EvidenceStamp,
    failed_acquisition_child: Option<(Arc<str>, Arc<str>)>,
    trace: RunTrace,
    parked_step: Option<ParkedStep>,
    clear_prayers: Option<ActionHandle<ClearPrayers>>,
    prayer_cleanup_pending: bool,
    prayer_cleanup_owned: RaisedPrayers,
    last_outcome: Option<StepOutcome>,
    /// Latest Combat family receipt, kept after later non-combat steps begin
    /// so Path `combat_end` skip_if can still select the caller walk-out
    /// (design-combat.md:664).
    last_combat: Option<StepOutcome>,
    published_receipt: Option<Arc<dyn super::compile::FamilyReceipt>>,
    last_attempt_receipt: Option<Arc<dyn super::compile::FamilyReceipt>>,
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
    /// The step that last failed with a dialogue page still open, as
    /// `(in_prelude, sequence, step)`. Its next begin continues that page
    /// from the current tile; every begin takes the value.
    open_page_retry: Option<(bool, usize, usize)>,
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
    step_attempts: u32,
    attempt_step: Option<FactKey>,
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
    /// An advancing combat step that killed its target reads progress once
    /// that NPC (index) has left the scene, or at the evidence tick bound.
    read_after_kill: Option<(u16, u64)>,
    /// Every root step begun, in order. Several may begin within one tick,
    /// and the run trace folds repeated lines, so tests read this instead.
    #[cfg(test)]
    begun: Vec<Arc<str>>,
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
    fields.push(StatusField {
        key: "combat_kills",
        label: "Confirmed kills in step",
        value: StatusValue::Integer(i64::from(receipt.kills)),
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
            journal_drain: None,
            journal_drains: 0,
            journal_drain_since: None,
            custom_reader: None,
            custom_read_after: None,
            pair_admission: None,
            pair_admission_since: None,
            pair_begin_since: None,
            pair_admitted: false,
            gang_reader: super::gang::GangRead::default(),
            last_read: None,
            journal_text: None,
            read_requested: false,
            selection_since: None,
            seq_index: 0,
            step_index: 0,
            cursor: 0,
            step: None,
            survival: crate::survival::SurvivalStep::default(),
            survival_action: None,
            survival_deferred: false,
            survival_interrupting: false,
            selected_hazard: SelectedHazard::default(),
            park_page_ticks: 0,
            park_page_seen: false,
            park_bound_hp: 0,
            step_after: api::quest_progress::EvidenceStamp {
                run,
                tick: 0,
                sequence: 0,
            },
            clear_prayers: None,
            prayer_cleanup_pending: false,
            prayer_cleanup_owned: RaisedPrayers::empty(),
            last_outcome: None,
            last_combat: None,
            published_receipt: None,
            last_attempt_receipt: None,
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
            open_page_retry: None,
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
            step_attempts: 0,
            attempt_step: None,
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
            read_after_kill: None,
            #[cfg(test)]
            begun: Vec::new(),
        }
    }

    /// The current sequence's trace units, or zero (the 32-line floor) when
    /// the runner has no sequence yet.
    fn stage_units(&self) -> usize {
        self.path
            .sequences
            .get(self.seq_index)
            .map_or(0, |sequence| sequence.trace_units)
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
                AcquisitionTraceOutcome::Restarted { restart, max, goal } => self.trace.record(
                    output,
                    api::hostlog::Level::Info,
                    format_args!(
                        "quester {quest}: stage {stage} recipe {recipe} restart {restart}/{max} after child {child_step}: {goal} is still false"
                    ),
                ),
            },
            StepTraceEvent::WalkRetry {
                attempt,
                failure_detail,
            } => self.trace.record(
                output,
                api::hostlog::Level::Info,
                format_args!(
                    "quester {quest}: stage {stage} retrying walk attempt {attempt}: {failure_detail}"
                ),
            ),
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

    /// Poll provisioning; `true` once it is Ready. A bank receipt or a
    /// finished acquisition re-plans on the tick it lands (TICK-FIX #7,
    /// E-Q7/D5) while the tick's event is unspent: a step that was waiting
    /// on its predicate scan is polled now, an idle runner plans the next
    /// bank trip or recipe now.
    fn poll_provision(&mut self, tick: &mut NativeTick<'_>) -> bool {
        for _ in 0..PROVISION_REPLANS {
            let provisioned = self.poll_provision_once(tick);
            if self.survival_interrupting {
                return false;
            }
            match provisioned {
                Provisioned::Ready => return true,
                Provisioned::Wait => return false,
                Provisioned::Landed => {
                    if !tick.cx.may_continue_this_tick() {
                        return false;
                    }
                    // A step waiting on its scan polls only after provisioning
                    // closed the bank its own trips opened.
                    if self.step.is_some() && !self.provisioner.session_open() {
                        return true;
                    }
                }
            }
        }
        false
    }

    fn poll_provision_once(&mut self, tick: &mut NativeTick<'_>) -> Provisioned {
        let required_after = tick.cx.evidence();
        let bank_phase = self.provisioner.bank_phase();
        let bank_starts = self.provisioner.bank_starts();
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
            banks: &self.banks,
            choices: &self.choices,
        };
        let result = self.provisioner.poll(
            &mut cx,
            &self.path.provisioning,
            self.active_loadout.as_deref(),
            self.stage.as_ref(),
        );
        while let Some(event) = self.provisioner.take_trace_event() {
            self.trace_step_event(tick.output, event);
        }
        // A trip may begin and settle (or fail) within one poll now that a
        // started trip is polled on its start tick, so both lines are traced.
        let started = (self.provisioner.bank_starts() != bank_starts)
            .then(|| self.provisioner.status().phase);
        let ended = bank_phase.or(started);
        let mut bank_events: [Option<(ProvisionPhase, &str)>; 2] = [None, None];
        if let Some(phase) = started {
            bank_events[0] = Some((phase, "begin"));
        }
        bank_events[1] = match (&result, ended) {
            (Poll::Ready(Ok(ProvisionEvent::BankReceipt(_))), Some(phase)) => {
                Some((phase, "settled"))
            }
            (Poll::Ready(Err(_)), Some(phase)) => Some((phase, "failed")),
            _ => None,
        };
        for (phase, event) in bank_events.into_iter().flatten() {
            let action = match phase {
                ProvisionPhase::Scanning => "scan",
                ProvisionPhase::Spillover => "deposit-capacity",
                ProvisionPhase::Withdrawing => "withdraw",
                ProvisionPhase::Closing => "close",
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
                if self.provisioner.needs_progress_read() {
                    self.needs_read = true;
                    self.dirty = true;
                }
                Provisioned::Wait
            }
            Poll::Ready(Ok(ProvisionEvent::Ready)) => Provisioned::Ready,
            Poll::Ready(Ok(ProvisionEvent::BankReceipt(_))) => {
                // The open table reached the account's bank memory through the
                // host's per-frame observe (design-bank-snapshot §1.3); the
                // step only needs to know its scan is over.
                if let Some(step) = self.step.as_mut() {
                    step.bank_scan_completed();
                }
                self.dirty = true;
                Provisioned::Landed
            }
            Poll::Ready(Ok(ProvisionEvent::Acquired)) => {
                // Inventory changes outside the bank do not change its stock.
                self.dirty = true;
                Provisioned::Landed
            }
            Poll::Ready(Ok(ProvisionEvent::Blocked { item })) => {
                self.parked = true;
                self.record_failure(ActionError::Blocked(item));
                Provisioned::Wait
            }
            Poll::Ready(Err(
                error @ (ActionError::Blocked(_)
                | ActionError::CombatInterrupted
                | ActionError::NeedsEvidence(_)
                | ActionError::UserInput),
            )) => {
                self.parked = true;
                self.record_failure(error);
                Provisioned::Wait
            }
            Poll::Ready(Err(error)) => {
                // Synthetic provisioning work uses the authored step failure
                // policy too; it must not turn one transient family refusal into
                // an immediate parked quest.
                self.provisioner.cancel();
                self.record_step_failure(error, tick);
                Provisioned::Wait
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
        self.selected_hazard.clear();
        self.step_index = self
            .path
            .sequences
            .get(self.seq_index)
            .map_or(self.path.prelude.len(), |sequence| sequence.steps.len());
        self.trace.flush_repeats(tick.output);
        self.trace.terminal(
            tick.output,
            api::hostlog::Level::Info,
            format_args!("quester {}: finish", self.path.id.0),
        );
        self.emit_status(tick.output, NativePhase::Complete);
        ScriptFlow::Complete
    }

    pub(super) fn bank_lease(&self) -> Option<BankLease> {
        self.provisioner.bank_lease()
    }

    /// The lease of a provisioning session that has ended with its bank open.
    pub(super) fn take_unowned_bank_lease(&mut self) -> Option<BankLease> {
        self.provisioner.take_unowned_lease()
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
                QuesterFailureKind::HazardUnsurvivable => "hazard-unsurvivable",
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
                    quest_gate_evidence_detail(gates.as_ref())
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
            ActionError::BankDeposit(reason) => {
                (QuesterFailureKind::Other, Arc::from(reason.message()))
            }
            ActionError::CombatInterrupted => (
                QuesterFailureKind::Other,
                Arc::from("dialogue interrupted by combat"),
            ),
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
            } else if self.survival.parked().is_some() {
                NativePhase::Waiting
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
            StatusField {
                key: "survival",
                label: "Survival",
                value: StatusValue::Text(format!("{}", self.survival.report()).into()),
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
            ProvisionPhase::Scanning
                | ProvisionPhase::Spillover
                | ProvisionPhase::Withdrawing
                | ProvisionPhase::Closing
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
        let budget_stop_spent = parked_step.is_some()
            && current
                .and_then(|step| step.attempt_budget.as_ref())
                .is_some_and(|budget| self.step_attempts >= budget.max_attempts);
        if let Some(receipt) = self.last_attempt_receipt.as_deref() {
            if let Some(attempts) = receipt.attempt_count() {
                fields.push(StatusField {
                    key: "step_attempts",
                    label: "Step attempts",
                    value: StatusValue::Integer(i64::from(attempts)),
                });
            }
            if budget_stop_spent {
                if let Some(stop) = receipt.attempt_stop() {
                    fields.push(StatusField {
                        key: "step_stop",
                        label: "Step stop",
                        value: StatusValue::Text(Arc::clone(stop)),
                    });
                }
            }
        }
        fields.extend_from_slice(&self.queue_fields);
        fields.extend_from_slice(&self.required_vs_live);
        fields.push(StatusField {
            key: "tested_stats_warning",
            label: "Tested stats warning",
            value: StatusValue::Text(Arc::clone(&self.tested_stats_warning)),
        });
        fields.push(StatusField {
            key: "selected_revision",
            label: "Selected revision",
            value: StatusValue::Integer(i64::from(self.selected.revision())),
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
        if let Some(receipt) = outcome
            .and_then(|outcome| outcome.receipt.as_ref())
            .and_then(|receipt| {
                receipt
                    .as_any()
                    .downcast_ref::<super::families::timed_ground::TimedGroundReceipt>()
            })
        {
            fields.extend([
                StatusField {
                    key: "ground_waited_ticks",
                    label: "Ground pickup observed ticks",
                    value: StatusValue::Integer(
                        i64::try_from(receipt.waited_ticks).unwrap_or(i64::MAX),
                    ),
                },
                StatusField {
                    key: "ground_held",
                    label: "Ground pickup held",
                    value: StatusValue::Integer(i64::from(receipt.held)),
                },
            ]);
        }
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
            survival: self.survival.report(),
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
    fn guarded_survival_step(&self) -> bool {
        self.step.is_some()
            && self
                .current_step()
                .is_some_and(|step| matches!(step.kind.as_ref(), "walk" | "combat"))
    }

    fn survival_transfer_in_flight(&self) -> bool {
        self.step
            .as_ref()
            .is_some_and(|step| step.transfer_in_flight())
            || self.provisioner.transfer_in_flight()
    }

    fn interrupt_survival_transfer(&mut self, tick: &mut NativeTick<'_>) {
        if self.survival_interrupting {
            return;
        }
        if let Some(step) = &mut self.step {
            if step.transfer_in_flight() {
                step.interrupt_transfer(tick.actions);
                self.survival_interrupting = true;
                return;
            }
        }
        if self.provisioner.transfer_in_flight() {
            self.provisioner.interrupt_transfer(tick.actions);
            self.survival_interrupting = true;
        }
    }

    fn cancel_detached_work_for_survival(&mut self, tick: &mut NativeTick<'_>) {
        let mut cancelled_read = self.journal.take().is_some();
        cancelled_read |= self.journal_drain.take().is_some();
        if let Some(mut reader) = self.custom_reader.take() {
            reader.cancel(tick.actions);
            cancelled_read = true;
        }
        self.custom_read_after = None;
        if cancelled_read {
            self.needs_read = true;
            self.read_after_kill = None;
        }
        if let Some(mut admission) = self.pair_admission.take() {
            admission.cancel(tick.actions);
            self.pair_admitted = false;
        }
        self.pair_admission_since = None;
        self.pair_begin_since = None;
        self.gang_reader.cancel();
    }

    /// Drop the Provisioner's runs for survival, keeping the prayers its
    /// acquisition run raised for the owned prayer cleanup.
    fn cancel_provisioner_for_survival(&mut self) {
        self.prayer_cleanup_owned
            .merge(self.provisioner.prayer_cleanup());
        self.prayer_cleanup_pending = !self.prayer_cleanup_owned.is_empty();
        self.provisioner.cancel();
    }

    fn cancel_step_for_survival(&mut self, tick: &mut NativeTick<'_>) {
        self.capture_prayer_cleanup();
        if let Some(mut step) = self.step.take() {
            step.cancel(tick.actions);
        }
        self.cancel_provisioner_for_survival();
        self.last_outcome = None;
        self.cancel_detached_work_for_survival(tick);
        self.advances = false;
        self.attempts = 0;
        self.settling = false;
        self.settle_deadline = Duration::ZERO;
        self.needs_read = true;
        self.progress = None;
        self.parked_step = None;
        if self.waiting.take().is_some() {
            self.dirty = true;
        }
        self.clear_last_error();
        self.dirty = true;
    }

    fn cancel_prayer_clear_for_survival(&mut self, tick: &mut NativeTick<'_>) {
        if let Some(handle) = self.clear_prayers.take() {
            tick.actions.cancel(handle);
        }
        self.prayer_cleanup_pending = !self.prayer_cleanup_owned.is_empty();
    }

    fn begin_survival_click(&mut self, mut args: OneOpArgs, tick: &mut NativeTick<'_>) {
        if !tick.cx.may_continue_this_tick() {
            self.survival_deferred = true;
            return;
        }
        if let Some(close) = OneOpArgs::close_modal_if_open(tick.cx.snapshot()) {
            args = close;
        }
        let kind = args.kind;
        match tick.actions.begin::<OneOp>(args, &mut tick.cx) {
            Ok(handle) => {
                if api::hostlog::enabled(api::hostlog::Category::ScriptTrace) {
                    if let Some(((kind, deadline), snapshot)) = handle
                        .inspect(OneOp::trace_state)
                        .zip(tick.frame().snapshot())
                    {
                        api::host_log!(
                            api::hostlog::Category::ScriptTrace,
                            api::hostlog::Level::Debug,
                            "quester survival op event=begin tick={} kind={kind:?} applied=pending deadline={deadline} player_info={}",
                            tick.cx.evidence().tick,
                            snapshot.gens().player_info
                        );
                    }
                }
                self.survival_action = Some(handle);
                self.survival_deferred = false;
                self.survival.record_sent(kind, &tick.cx);
                self.dirty = true;
            }
            Err(
                ActionError::Busy
                | ActionError::Held
                | ActionError::Stale
                | ActionError::Cancelled
                | ActionError::BudgetExhausted,
            ) => self.survival_deferred = true,
            Err(error) => {
                self.parked = true;
                self.record_failure(error);
                self.dirty = true;
            }
        }
    }

    /// Cancel the work a survival click or park replaces (design A6.5 Quester
    /// items 3, 5 and 6). A park also ends a settle wait and asks for a fresh
    /// read, as the un-park's `Interrupt::Resume` shape does: the park can
    /// outlast the settle deadline.
    fn cancel_work_for_survival(&mut self, tick: &mut NativeTick<'_>, park: bool) {
        if self.step.is_some() || park {
            self.cancel_step_for_survival(tick);
        } else if self.provisioner.has_run() {
            self.cancel_provisioner_for_survival();
            self.cancel_detached_work_for_survival(tick);
        } else {
            self.cancel_detached_work_for_survival(tick);
        }
    }

    fn poll_survival(&mut self, tick: &mut NativeTick<'_>) {
        let before = self.survival.report();
        let was_parked = self.survival.parked().is_some();
        let want = self.survival.want(
            &tick.cx,
            crate::survival::step::Upkeep {
                eat_floor: 0,
                hazard: self.selected_hazard.hazard,
            },
        );
        self.warn_undeclared_hazard(tick);
        if was_parked && self.survival.parked().is_none() {
            // The un-park resumes the way `Interrupt::Resume` does (design
            // A6.5 item 2): a fresh read, with no step.
            self.needs_read = true;
            self.park_page_ticks = 0;
            self.park_page_seen = false;
        }
        if self.survival_action.is_none() {
            match want {
                crate::survival::step::SurvivalWant::Nothing => {
                    if !self.survival_interrupting {
                        self.survival_deferred = false;
                    }
                }
                crate::survival::step::SurvivalWant::Hold => self.survival_deferred = true,
                crate::survival::step::SurvivalWant::Click(args) => {
                    self.cancel_prayer_clear_for_survival(tick);
                    if self.guarded_survival_step() {
                        self.survival.defer();
                        self.survival_deferred = true;
                    } else if self.survival_transfer_in_flight() {
                        self.interrupt_survival_transfer(tick);
                        self.survival.defer();
                        self.survival_deferred = true;
                    } else {
                        self.cancel_work_for_survival(tick, false);
                        self.begin_survival_click(args, tick);
                    }
                }
                crate::survival::step::SurvivalWant::Need(need) => {
                    if self.guarded_survival_step() {
                        self.survival_deferred = true;
                    } else if self.survival_transfer_in_flight() {
                        self.interrupt_survival_transfer(tick);
                        self.survival_deferred = true;
                    } else {
                        self.cancel_prayer_clear_for_survival(tick);
                        self.cancel_work_for_survival(tick, true);
                        self.survival.park(need, &tick.cx);
                        self.park_page_ticks = 0;
                        self.park_page_seen = false;
                        self.park_bound_hp = tick
                            .cx
                            .condition()
                            .map_or(0, |condition| i32::from(condition.hp));
                        self.log_hazard_park_bound("edge", tick);
                    }
                }
            }
        }
        if self.survival.report() != before {
            self.dirty = true;
        }
    }

    fn warn_undeclared_hazard(&mut self, tick: &mut NativeTick<'_>) {
        if self.selected_hazard.hazard.is_some()
            || self.selected_hazard.slot.is_none()
            || self.selected_hazard.handled
        {
            return;
        }
        let Some(hit) = self.survival.unattributed_hit(&tick.cx) else {
            return;
        };
        let Some(step) = self.current_step() else {
            return;
        };
        tick.output.log(
            api::hostlog::Level::Warn,
            &format!(
                "quester undeclared damage step={} hit={hit} hp={}",
                step.id.0,
                tick.cx.condition().map_or(0, |condition| condition.hp)
            ),
        );
        self.selected_hazard.handled = true;
    }

    fn log_hazard_park_bound(&self, cause: &'static str, tick: &mut NativeTick<'_>) {
        let Some(hazard) = self.selected_hazard.hazard else {
            return;
        };
        if !matches!(
            self.survival.parked(),
            Some(crate::survival::math::SurvivalNeed::Food { .. })
        ) || !self.survival.quiet_hazard_recovery(&tick.cx)
        {
            return;
        }
        let Some(condition) = tick.cx.condition().filter(|condition| condition.hp_known) else {
            return;
        };
        let hp = i32::from(condition.hp);
        let Some(release_hp) = hazard.release_hitpoints(hp, i32::from(condition.hp_max), 0) else {
            return;
        };
        let Some(step) = self.current_step() else {
            return;
        };
        // login.rs2:38 installs health_regen at 100 ticks. stat.rs2:59-60
        // heals one point. The final tick lets the poll observe the release.
        let bound_ticks = (release_hp - hp) * 100 + 1;
        tick.output.log(
            api::hostlog::Level::Info,
            &format!(
                "quester hazard park step={} cause={cause} anchor_tick={} hp={hp} release_hp={release_hp} bound_ticks={bound_ticks}",
                step.id.0,
                tick.cx.evidence().tick
            ),
        );
    }

    fn reconcile_park_page(&mut self, tick: &mut NativeTick<'_>) {
        if self.survival.parked().is_none() || self.parked {
            return;
        }
        if let Some(condition) = tick.cx.condition().filter(|condition| condition.hp_known) {
            let hp = i32::from(condition.hp);
            if hp < self.park_bound_hp {
                self.park_bound_hp = hp;
                self.log_hazard_park_bound("hit", tick);
            }
        }
        let close = OneOpArgs::close_modal_if_open(tick.cx.snapshot());
        let page_open = close.is_some();
        if !page_open && self.park_page_seen {
            self.log_hazard_park_bound("close", tick);
        }
        if let Some(close) = close {
            self.park_page_ticks = self.park_page_ticks.saturating_add(1);
            if self.park_page_ticks > PARK_PAGE_LIMIT {
                self.parked = true;
                self.record_failure(ActionError::Blocked(Arc::from(
                    "survival park: the page did not close within 24 parked ticks",
                )));
                self.dirty = true;
            } else if self.survival_action.is_none() {
                self.begin_survival_click(close, tick);
            }
        }
        self.park_page_seen = page_open;
    }

    /// Poll the survival op. Returns true while it stays live, or when it
    /// failed and parked the runner, so the caller ends this tick. An op that
    /// settles or is not applied in this poll returns false: the survival
    /// poll after it decides this tick's next click or hold, and ordinary work
    /// takes the tick when survival wants nothing more.
    fn poll_survival_action(&mut self, tick: &mut NativeTick<'_>) -> bool {
        let Some(handle) = self.survival_action.as_ref() else {
            return false;
        };
        let trace = api::hostlog::enabled(api::hostlog::Category::ScriptTrace)
            .then(|| {
                let ((kind, deadline), snapshot) = handle
                    .inspect(OneOp::trace_state)
                    .zip(tick.frame().snapshot())?;
                Some((kind, deadline, snapshot.gens().player_info))
            })
            .flatten();
        let result = tick.actions.poll(handle, &mut tick.cx);
        if let (Some((kind, deadline, player_info)), Poll::Ready(result)) = (trace, &result) {
            api::host_log!(
                api::hostlog::Category::ScriptTrace,
                api::hostlog::Level::Debug,
                "quester survival op event=end tick={} kind={kind:?} applied={} deadline={deadline} player_info={}",
                tick.cx.evidence().tick,
                matches!(result, Ok(true)),
                player_info
            );
        }
        let applied = match result {
            Poll::Pending => {
                self.publish(tick.output);
                return true;
            }
            Poll::Ready(Ok(settled)) => settled,
            Poll::Ready(Err(
                ActionError::Busy
                | ActionError::Held
                | ActionError::Stale
                | ActionError::Cancelled
                | ActionError::BudgetExhausted,
            )) => false,
            Poll::Ready(Err(error)) => {
                self.survival.record_settled(false, &tick.cx);
                self.survival_action = None;
                self.parked = true;
                self.record_failure(error);
                self.dirty = true;
                self.publish(tick.output);
                return true;
            }
        };
        // Anchor the click's delays on the frame that showed its effect
        // (`SurvivalStep::record_settled`).
        self.survival.record_settled(applied, &tick.cx);
        self.survival_action = None;
        self.survival_deferred = true;
        self.dirty = true;
        self.publish(tick.output);
        false
    }

    fn finish_survival_transfer(&mut self, tick: &mut NativeTick<'_>) -> bool {
        if !self.survival_interrupting || self.survival_transfer_in_flight() {
            return false;
        }
        self.survival_interrupting = false;
        if self.step.is_some() {
            self.cancel_step_for_survival(tick);
        } else if self.provisioner.has_run() {
            self.cancel_provisioner_for_survival();
            self.cancel_detached_work_for_survival(tick);
        }
        self.survival_deferred = true;
        true
    }

    fn cancel_step(&mut self, tick: &mut NativeTick<'_>) {
        self.capture_prayer_cleanup();
        if let Some(mut step) = self.step.take() {
            step.cancel(tick.actions);
        }
        self.provisioner.cancel();
        self.clear_prayers = None;
        self.last_outcome = None;
        self.journal = None;
        self.journal_drain = None;
        self.custom_reader = None;
        self.custom_read_after = None;
        self.pair_admission = None;
        self.pair_begin_since = None;
        self.gang_reader.cancel();
        self.advances = false;
        self.attempts = 0;
        self.step_attempts = 0;
        self.attempt_step = None;
        self.last_attempt_receipt = None;
        self.settling = false;
        self.settle_deadline = Duration::ZERO;
        if self.waiting.take().is_some() {
            self.dirty = true;
        }
        self.clear_last_error();
    }

    /// The guardian hold teleported the player, which clears the player's
    /// interaction and moves them off the tile the step planned from: drop the
    /// step handle and its native owners, and read the journal again. Progress
    /// and the ordered cursor stay, so selection resumes at the same step and
    /// its own `skip_if`. A step that had already finished (`settling`) keeps
    /// `settling`, `last_outcome`, `advances` and `settle_deadline`, so it goes
    /// on from its own settle predicate, not from cursor 0. Partner work
    /// pending keeps the step: the pair's protocol owns it.
    fn drop_step_for_teleport(&mut self) {
        if self.pair_work_pending() {
            api::host_log!(
                api::hostlog::Category::ScriptLifecycle,
                api::hostlog::Level::Info,
                "quester {}: the player was teleported during a guardian hold; partner work is pending, so the step stays (cursor {})",
                self.path.id.0,
                self.cursor
            );
            return;
        }
        api::host_log!(
            api::hostlog::Category::ScriptLifecycle,
            api::hostlog::Level::Info,
            "quester {}: the player was teleported during a guardian hold; dropping the in-flight step, keeping progress and cursor {} (settling: {})",
            self.path.id.0,
            self.cursor,
            self.settling
        );
        let settling = self.settling;
        self.pair_admission = None;
        self.pair_admission_since = None;
        // Dropping these guards revokes their native owners before the host can dispatch them.
        self.capture_prayer_cleanup();
        self.step = None;
        self.provisioner.cancel();
        self.clear_prayers = None;
        self.journal = None;
        self.journal_drain = None;
        self.needs_read = true;
        self.last_combat = None;
        self.attempts = 0;
        if !settling {
            self.last_outcome = None;
            self.advances = false;
            self.settle_deadline = Duration::ZERO;
        }
        self.unreadable_since = None;
        self.journal_attempts = 0;
        self.journal_drains = 0;
        self.journal_drain_since = None;
        self.journal_retry_pending = false;
        self.journal_quiet_since = None;
        self.selection_since = None;
        self.waiting = None;
        self.dirty = true;
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

    fn scope_stage_cross(&self, tick: &mut NativeTick<'_>) {
        static EMPTY: std::sync::LazyLock<Arc<[Arc<str>]>> =
            std::sync::LazyLock::new(|| Arc::from([]));
        let cross = self
            .stage
            .as_ref()
            .and_then(|stage| sequence_for_stage(&self.path, &stage.0))
            .map_or(&*EMPTY, |index| &self.path.sequences[index].cross);
        tick.cx.stage_cross = Some(Arc::clone(cross));
    }

    /// Re-targets the sequence after a settle outcome. The ordered cursor moves
    /// past a settled sequence step, holds on a failed one, and starts over
    /// when the stage now names another sequence.
    fn retarget_after_settle(&mut self, settled: bool) {
        let Some(stage) = self.stage.as_ref() else {
            return;
        };
        let sequence = sequence_for_stage(&self.path, &stage.0).unwrap_or(self.seq_index);
        if sequence != self.seq_index {
            self.cursor = 0;
        } else if settled && !self.in_prelude {
            self.cursor = self.step_index + 1;
        }
        self.seq_index = sequence;
        self.step_index = 0;
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
            if let Some(modal) = tick.cx.snapshot().main_modal().filter(|modal| {
                modal.value.root != ROOT_289
                    && (modal.value.root != -1 || !modal.value.texts.is_empty())
            }) {
                self.set_last_error(
                    QuesterFailureKind::Other,
                    Arc::from(format!(
                        "journal blocked by modal root {}{}",
                        modal.value.root,
                        modal
                            .value
                            .texts
                            .iter()
                            .find(|text| !text.is_empty())
                            .map(|text| format!(" ({text})"))
                            .unwrap_or_default()
                    )),
                );
            } else if let Some(chat) = tick.cx.snapshot().chat_modal() {
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

    /// Journal `begin` is Busy on a chat continue page (`chat_page_open`).
    /// While a step or provisioning run is live, that page is its dialogue's
    /// and the latched-continue Busy rule holds: the owner advances it. With
    /// no live owner (a zone trigger's player chat, or the tail of a finished
    /// step's conversation) nothing else will click it, so the read drains it
    /// with the shared continuation driver, Chat continue clicks only (a menu
    /// fails strict; a Main scroll, book or modal the page opens ends the
    /// drain untouched), and retries. A read spends at most `JOURNAL_CONTINUE_DRAINS`
    /// drains within `JOURNAL_DRAIN_WINDOW`; a page that keeps reopening then
    /// parks with its root and text. `None`: not drainable, so the caller waits.
    fn drain_unowned_continue(&mut self, tick: &mut NativeTick<'_>) -> Option<bool> {
        if self.step.is_some() || self.provisioner.run_live() {
            return None;
        }
        let main_closed = tick
            .cx
            .snapshot()
            .main_modal()
            .is_some_and(|modal| modal.value.root == -1 && modal.value.texts.is_empty());
        let continue_open = tick
            .cx
            .snapshot()
            .chat_modal()
            .is_some_and(|chat| chat.value.continue_component_id >= 0);
        if !main_closed || !continue_open {
            return None;
        }
        if self.journal_drains >= JOURNAL_CONTINUE_DRAINS {
            self.park_on_reopening_continue(
                tick,
                format_args!("the chat continue reopened after {JOURNAL_CONTINUE_DRAINS} drains"),
            );
            return Some(false);
        }
        let args = DialogueArgs {
            target: DialogueTarget::Continuation,
            options: DialogueOptions::chat_continue_only(),
        };
        match tick.actions.begin::<Dialogue>(args, &mut tick.cx) {
            Ok(handle) => {
                let page = chat_page_label(tick);
                self.trace.record(
                    tick.output,
                    api::hostlog::Level::Info,
                    format_args!(
                        "quester {}: journal read drains an unowned chat continue ({page})",
                        self.path.id.0
                    ),
                );
                self.journal_drain = Some(handle);
                self.journal_drains += 1;
                self.journal_drain_since.get_or_insert(tick.cx.active_now());
                self.journal_quiet_since = None;
                self.dirty = true;
                // Never a completed read: the read retries once the page is gone.
                self.poll_journal_drain(tick);
                Some(false)
            }
            Err(ActionError::Busy | ActionError::Held | ActionError::BudgetExhausted) => None,
            Err(error) => {
                self.record_failure(error);
                self.parked = true;
                Some(false)
            }
        }
    }

    /// Polls the continue drain. `true` while it runs or after it parked the
    /// read; `false` once it is gone, so the read retries on this tick.
    fn poll_journal_drain(&mut self, tick: &mut NativeTick<'_>) -> bool {
        let Some(handle) = self.journal_drain.as_ref() else {
            return false;
        };
        if self
            .journal_drain_since
            .is_some_and(|since| tick.cx.active_now().saturating_sub(since) >= JOURNAL_DRAIN_WINDOW)
        {
            // Dropping the handle revokes the driver before it clicks again.
            self.journal_drain = None;
            self.park_on_reopening_continue(
                tick,
                format_args!(
                    "the chat continue kept reopening for {} s",
                    JOURNAL_DRAIN_WINDOW.as_secs()
                ),
            );
            return true;
        }
        let result = match tick.actions.poll(handle, &mut tick.cx) {
            Poll::Pending => return true,
            Poll::Ready(result) => result,
        };
        self.journal_drain = None;
        self.dirty = true;
        match result {
            // A failed or interrupted drain spent its budget; the retried
            // read drains again or parks on the cap.
            Ok(_)
            | Err(
                ActionError::Busy
                | ActionError::Held
                | ActionError::BudgetExhausted
                | ActionError::Stale
                | ActionError::Cancelled
                | ActionError::CombatInterrupted,
            ) => false,
            Err(error) => {
                self.parked = true;
                self.park_reason = "journal blocked by an unowned chat page";
                let page = chat_page_label(tick);
                self.set_last_error(
                    QuesterFailureKind::Other,
                    Arc::from(format!(
                        "journal blocked by {page}: {}",
                        trace_error_reason(&error)
                    )),
                );
                true
            }
        }
    }

    fn park_on_reopening_continue(&mut self, tick: &NativeTick<'_>, cause: fmt::Arguments<'_>) {
        self.parked = true;
        self.park_reason = "journal blocked by a reopening chat continue";
        let page = chat_page_label(tick);
        self.set_last_error(
            QuesterFailureKind::Other,
            Arc::from(format!("journal blocked by {page}: {cause}")),
        );
    }

    fn read_stage(&mut self, tick: &mut NativeTick<'_>, retarget: bool) -> bool {
        if self.path.progress_reader.is_some() {
            return self.read_custom_stage(tick, retarget);
        }
        if self.journal.is_some()
            && tick.cx.revision() == api::selected::ClientRevision::R377
            && self.selected.revision() == 377
            && quest_colour(&self.path, &self.quests, tick.cx.snapshot())
                == Some(QuestListStatus::Complete)
        {
            // Queued 377 completion can overtake a stage-journal click.
            // Drop revokes that obsolete read and its quiet lease without
            // closing the scroll; the normal colour resolver owns completion.
            self.journal = None;
        }
        if self.poll_journal_drain(tick) {
            return false;
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
                        ActionError::Failed(reason)
                            if reason.as_ref() == "journal modal timeout" =>
                        {
                            return self.retry_journal_read(
                                tick,
                                "journal modal repeatedly timed out",
                                JOURNAL_RETRY_LIMIT_MODAL_TIMEOUT,
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
                // A random-event owner can still be closing its Main window when
                // the step requests progress. Do not begin or close over it.
                if tick.cx.snapshot().main_modal().is_some_and(|modal| {
                    modal.value.root != ROOT_289
                        && (modal.value.root != -1 || !modal.value.texts.is_empty())
                }) {
                    return self
                        .wait_for_journal_read(tick, "journal blocked by an occupied modal");
                }
                if self.journal_retry_pending {
                    let closed = tick.cx.snapshot().main_modal().is_some_and(|modal| {
                        modal.value.root == -1 && modal.value.texts.is_empty()
                    }) && tick.cx.snapshot().chat_modal().is_some_and(|modal| {
                        modal.value.root == -1 && modal.value.texts.is_empty()
                    });
                    if !closed {
                        self.journal_quiet_since = None;
                        if let Some(read) = self.drain_unowned_continue(tick) {
                            return read;
                        }
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
                        // The quest-row click goes out on the begin tick
                        // (TICK-FIX #6, E-Q5); the reentry polls the handle.
                        if tick.cx.may_continue_this_tick() {
                            return self.read_stage(tick, retarget);
                        }
                    }
                    Err(ActionError::Busy | ActionError::Held | ActionError::BudgetExhausted) => {
                        if let Some(read) = self.drain_unowned_continue(tick) {
                            return read;
                        }
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
            resolve_colour(&self.path, colour, tick.cx.evidence())
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
        self.journal_drains = 0;
        self.journal_drain_since = None;
        self.journal_retry_pending = false;
        self.journal_quiet_since = None;
        if self.progress.is_none() {
            // Progress is dropped on every restart (Start, Resume, session
            // change, run key change, death), never by a random event, so the
            // first adoption after a drop replays an ordered sequence from its first step.
            // The reset precedes storing the read: an Unknown or Partial first
            // read must not spend it, or the Known read that follows would
            // resume at the stale cursor.
            self.cursor = 0;
        }
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
            // Each stage keeps its own trace budget, sized from its compiled
            // steps and the recipe steps its acquire steps run, so a long
            // quest logs every later step line too.
            self.trace
                .begin_stage(tick.output, self.path.sequences[sequence].trace_units);
            self.trace.record(
                tick.output,
                api::hostlog::Level::Info,
                format_args!("quester {}: stage {previous} → {}", self.path.id.0, stage.0),
            );
            self.stage = Some(stage);
            self.empty_reads = 0;
        }
        if retarget {
            // A same-stage re-read keeps the cursor; another sequence starts
            // at its top.
            if self.seq_index != sequence {
                self.cursor = 0;
            }
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

    fn current_attempt_budget(&self) -> Option<StepAttemptBudget> {
        let budget = self.current_step()?.attempt_budget.as_ref()?;
        Some(StepAttemptBudget {
            max_attempts: budget.max_attempts,
            stop: Arc::clone(&budget.stop),
        })
    }

    fn reset_attempt_scope_if_needed(&mut self, step: &FactKey) {
        if self.attempt_step.as_ref() != Some(step) {
            self.attempt_step = Some(step.clone());
            self.step_attempts = 0;
            self.last_attempt_receipt = None;
        }
    }

    fn record_budgeted_attempt(&mut self, budget: &StepAttemptBudget, include_outcome: bool) {
        self.step_attempts = self.step_attempts.saturating_add(1);
        let inner = if include_outcome {
            self.last_outcome
                .as_mut()
                .and_then(|outcome| outcome.receipt.take())
        } else {
            None
        };
        let receipt: Arc<dyn super::compile::FamilyReceipt> = Arc::new(StepAttemptReceipt::new(
            inner,
            self.step_attempts,
            Arc::clone(&budget.stop),
        ));
        if include_outcome {
            if let Some(outcome) = self.last_outcome.as_mut() {
                outcome.receipt = Some(Arc::clone(&receipt));
            }
        }
        self.last_attempt_receipt = Some(receipt);
    }

    fn park_at_attempt_budget(&mut self, budget: &StepAttemptBudget) {
        if self.step_attempts < budget.max_attempts {
            return;
        }
        let Some((sequence_index, step_index, in_prelude, step_id)) =
            self.current_step().map(|step| {
                (
                    self.seq_index,
                    self.step_index,
                    self.in_prelude,
                    Arc::clone(&step.id.0),
                )
            })
        else {
            return;
        };
        let reason = self
            .last_error
            .as_deref()
            .map(Arc::<str>::from)
            .unwrap_or_else(|| Arc::from("step failed"));
        self.parked_step = Some(ParkedStep {
            sequence_index,
            step_index,
            in_prelude,
            id: Arc::clone(&step_id),
        });
        self.parked = true;
        self.last_error = Some(Arc::from(format!(
            "{}: step {} stopped after {} attempts: {}",
            budget.stop, step_id, self.step_attempts, reason
        )));
        self.dirty = true;
    }

    fn park_at_run_retry_limit(&mut self) {
        let Some((sequence_index, step_index, in_prelude, step_id)) =
            self.current_step().map(|step| {
                (
                    self.seq_index,
                    self.step_index,
                    self.in_prelude,
                    Arc::clone(&step.id.0),
                )
            })
        else {
            return;
        };
        let reason = self
            .last_error
            .as_deref()
            .map(Arc::<str>::from)
            .unwrap_or_else(|| Arc::from("step failed"));
        self.parked_step = Some(ParkedStep {
            sequence_index,
            step_index,
            in_prelude,
            id: step_id,
        });
        self.parked = true;
        self.last_error = Some(Arc::from(format!(
            "run retry limit reached after {} consecutive failures: {}",
            self.fail_streak, reason
        )));
        self.dirty = true;
    }

    fn record_step_begin_failure(&mut self, error: ActionError) {
        if matches!(&error, ActionError::UserInput) {
            self.record_failure(error);
            self.parked = true;
            return;
        }
        self.attempts = self.attempts.saturating_add(1);
        self.record_failure(error);
        if let Some(budget) = self.current_attempt_budget() {
            self.fail_streak = self.fail_streak.saturating_add(1);
            self.record_budgeted_attempt(&budget, false);
            self.park_at_attempt_budget(&budget);
            if !self.parked && u32::from(self.fail_streak) >= super::path::MAX_STEP_ATTEMPTS {
                self.park_at_run_retry_limit();
            }
        } else if self.attempts >= 5 {
            self.parked = true;
        }
    }

    fn record_step_success(&mut self) {
        if let Some(budget) = self.current_attempt_budget() {
            self.record_budgeted_attempt(&budget, true);
        }
        self.attempt_step = None;
        self.step_attempts = 0;
    }

    fn record_step_failure(&mut self, error: ActionError, tick: &NativeTick<'_>) {
        self.record_step_failure_with_outcome(error, tick, false);
    }

    fn record_step_failure_with_outcome(
        &mut self,
        error: ActionError,
        tick: &NativeTick<'_>,
        include_outcome: bool,
    ) {
        let user_input = matches!(&error, ActionError::UserInput);
        if !user_input {
            self.fail_streak = self.fail_streak.saturating_add(1);
        }
        self.record_failure(error);
        let budget = self.current_attempt_budget();
        if !user_input {
            if let Some(budget) = &budget {
                self.record_budgeted_attempt(budget, include_outcome);
                self.park_at_attempt_budget(budget);
                if !self.parked && u32::from(self.fail_streak) >= super::path::MAX_STEP_ATTEMPTS {
                    self.park_at_run_retry_limit();
                }
            } else if self.fail_streak >= 5 {
                self.parked = true;
            }
        }
        self.on_step_boundary_with_watchdog(tick, true);
    }

    fn on_step_boundary(&mut self, tick: &NativeTick<'_>) {
        self.on_step_boundary_with_watchdog(tick, true);
    }

    fn on_step_boundary_with_watchdog(&mut self, tick: &NativeTick<'_>, observe_watchdog: bool) {
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
        if observe_watchdog
            && matches!(
                self.watchdog.observe(
                    self.stage.as_ref(),
                    tile,
                    &inv[..inv_len],
                    &worn[..worn_len],
                    xp,
                ),
                WatchdogAction::Park
            )
        {
            self.parked = true;
            self.dirty = true;
        }
    }

    /// An explicit read request, or a quest-list colour that contradicts the
    /// held stage, rereads progress before the next selection.
    fn request_contradicted_read(&mut self, tick: &NativeTick<'_>) {
        let contradicted =
            quest_colour(&self.path, &self.quests, tick.cx.snapshot()).is_some_and(|colour| {
                match colour {
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
                }
            });
        if self.read_requested || contradicted {
            self.needs_read = true;
            self.dirty = true;
        }
    }

    /// The tick body after the prologue (run change, eligibility, death,
    /// prayer cleanup, park). A step that completes re-enters this body once
    /// on the same tick ([`SAME_TICK_PASSES`]) while the tick's interaction
    /// event is unspent, so its settle, the progress read an advancing step
    /// owes, the next selection, its begin and its first poll all use the
    /// snapshot that proved the completion (TICK-FIX #4/#5/#6).
    fn advance(
        &mut self,
        tick: &mut NativeTick<'_>,
        pass: u8,
    ) -> Result<ScriptFlow, ScriptFailure> {
        self.scope_stage_cross(tick);
        if self.step.is_none() && !self.settling && !self.needs_read {
            self.request_contradicted_read(tick);
        }
        if self.needs_read {
            if let Some((index, until)) = self.read_after_kill {
                // The quest progress lands with the killed NPC's removal.
                if tick.cx.evidence().tick < until && npc_in_scene(tick, index) {
                    self.publish(tick.output);
                    return Ok(ScriptFlow::Continue);
                }
                self.read_after_kill = None;
            }
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
        self.scope_stage_cross(tick);
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
            let mut select_now = false;
            let mut reread_now = false;
            let truth = {
                let pred = PredicateContext {
                    cx: &tick.cx,
                    pairs: tick.pairs,
                    quests: &self.quests,
                    progress: self.progress_slice(),
                    required_after: tick.cx.evidence(),
                    chat_since: self.chat_since,
                    outcome: self.last_outcome.as_ref(),
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
                self.record_step_success();
                self.retarget_after_settle(true);
                self.on_step_boundary(tick);
                // Select on the snapshot that proved the settle instead of a
                // tick later. Anything the next tick's prologue would do first
                // (park, prayer cleanup, a reread) keeps the old order, and a
                // tick whose single interaction event is spent, or that first
                // shows a new scene, waits.
                if !self.parked && !self.prayer_cleanup_pending {
                    self.request_contradicted_read(tick);
                }
                select_now = !self.parked
                    && !self.prayer_cleanup_pending
                    && !self.needs_read
                    && tick.cx.may_continue_this_tick();
                // A boundary that owes a progress read (an advancing step, a
                // contradicted colour) reads now and selects on its result
                // (TICK-FIX #6, E-Q3): a colour read is immediate, a journal
                // read sends its click this tick.
                reread_now = !self.parked
                    && !self.prayer_cleanup_pending
                    && self.needs_read
                    && tick.cx.may_continue_this_tick();
            } else {
                if tick.cx.active_now() >= self.settle_deadline {
                    self.settling = false;
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
                    self.record_step_failure_with_outcome(error, tick, true);
                    // Progress may land after the earlier read while an advancing step settles.
                    self.needs_read = !self.parked && (self.needs_read || self.advances);
                    self.dirty = true;
                    if self.parked {
                        self.parked_step = failed_step;
                    }
                    self.retarget_after_settle(false);
                }
            }
            if !select_now {
                if reread_now && pass < SAME_TICK_PASSES && !self.survival_deferred {
                    return self.advance(tick, pass + 1);
                }
                self.publish(tick.output);
                return Ok(ScriptFlow::Continue);
            }
            if self
                .path
                .sequences
                .get(self.seq_index)
                .is_some_and(|seq| seq.terminal && seq.steps.is_empty())
            {
                return Ok(self.finish_quest(tick));
            }
        }
        if self.step.is_none() {
            if !self.poll_provision(tick) {
                self.publish(tick.output);
                return Ok(ScriptFlow::Continue);
            }
            // A receipt may settle on a wake after this tick's event or on
            // scene entry. Fresh selection waits until both fences are clear.
            if !tick.cx.may_continue_this_tick() {
                self.publish(tick.output);
                return Ok(ScriptFlow::Continue);
            }
            let (selected, skipped_to) = {
                let path = &self.path;
                let seq_index = self.seq_index;
                let cursor = self.cursor;
                let quests = &self.quests;
                let progress = self
                    .progress
                    .as_deref()
                    .map(std::slice::from_ref)
                    .unwrap_or(&[]);
                let outcome = self.last_combat.as_ref().or(self.last_outcome.as_ref());
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
                };
                match select_with_skips(path, seq_index, cursor, &pred, |step| {
                    trace.record(
                        output,
                        api::hostlog::Level::Info,
                        format_args!(
                            "quester {}: stage {stage} step {} skipped: {} evaluated true",
                            path.id.0, step.id.0, step.skip_if_summary
                        ),
                    );
                }) {
                    SelectionDecision::Selected(sel) => (
                        Ok(Some((sel.index, sel.step.advances, sel.prelude))),
                        (!sel.prelude).then_some(sel.index),
                    ),
                    SelectionDecision::Exhausted => (
                        Ok(None),
                        path.sequences.get(seq_index).map(|seq| seq.steps.len()),
                    ),
                    SelectionDecision::Unknown(sel) => (
                        Err((
                            sel.index,
                            sel.prelude,
                            Arc::clone(&sel.step.id.0),
                            Arc::clone(&sel.step.skip_if_summary),
                            sel.step.skip_if.requires_bank(),
                        )),
                        (!sel.prelude).then_some(sel.index),
                    ),
                }
            };
            // Every sequence step the selector passed over had a proven-true
            // skip_if, so those skips commit to the ordered cursor whether the
            // selection then started a step, blocked on Unknown evidence, or
            // ran out of steps: a skipped prefix never re-runs without a
            // reset. Prelude skips never move it.
            if let Some(cursor) = skipped_to {
                self.cursor = cursor;
            }
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
                    if requires_bank && !bank_known(tick) {
                        self.start_predicate_bank_scan(tick);
                        self.selection_since = None;
                        // The scan trip starts on this tick (TICK-FIX #4).
                        if tick.cx.may_continue_this_tick() {
                            self.poll_provision(tick);
                        }
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
            let Some((index, advances, prelude)) = selected else {
                self.selected_hazard.clear();
                if self
                    .path
                    .sequences
                    .get(self.seq_index)
                    .is_some_and(|seq| seq.terminal)
                {
                    return Ok(self.finish_quest(tick));
                }
                self.last_outcome = None;
                if self
                    .path
                    .sequences
                    .get(self.seq_index)
                    .is_some_and(|seq| seq.order == super::path::SequenceOrder::Ordered)
                {
                    let stage = self
                        .stage
                        .as_ref()
                        .map_or("unknown", |stage| stage.0.as_ref());
                    self.trace.record(
                        tick.output,
                        api::hostlog::Level::Info,
                        format_args!(
                            "quester {}: stage {stage} ordered sequence has no step left at cursor {}; rereading progress",
                            self.path.id.0, self.cursor
                        ),
                    );
                }
                self.empty_reads += 1;
                if self.empty_reads >= 2 {
                    self.in_prelude = false;
                    self.step_index = self
                        .path
                        .sequences
                        .get(self.seq_index)
                        .map_or(0, |sequence| sequence.steps.len());
                    self.parked_step = None;
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
            let step_id = if prelude {
                self.path.prelude[index].id.clone()
            } else {
                self.path.sequences[self.seq_index].steps[index].id.clone()
            };
            self.reset_attempt_scope_if_needed(&step_id);
            let step = if prelude {
                &self.path.prelude[index]
            } else {
                &self.path.sequences[self.seq_index].steps[index]
            };
            self.selected_hazard.select(
                StepSlot {
                    in_prelude: prelude,
                    seq_index: self.seq_index,
                    step_index: index,
                },
                step.hazard,
            );
            if let Some(hazard) = step.hazard.filter(|_| !self.selected_hazard.handled) {
                let Some(hp_max) = tick
                    .cx
                    .snapshot()
                    .stats()
                    .and_then(|stats| stats.value.iter().find(|stat| stat.index == 3))
                    .map(|stat| stat.base)
                else {
                    self.publish(tick.output);
                    return Ok(ScriptFlow::Continue);
                };
                let max = hazard.max_damage(hp_max, hp_max);
                let line = crate::combat::select::lines(Some(max), hp_max).eat;
                tick.output.log(
                    api::hostlog::Level::Info,
                    &format!(
                        "quester hazard step={} max={max} source={} line={line}",
                        step.id.0,
                        step.hazard_source.as_deref().unwrap_or("")
                    ),
                );
                self.selected_hazard.handled = true;
                if line >= hp_max {
                    let id = Arc::clone(&step.id.0);
                    self.parked_step = Some(ParkedStep {
                        sequence_index: self.seq_index,
                        step_index: index,
                        in_prelude: prelude,
                        id: Arc::clone(&id),
                    });
                    self.set_last_error(
                        QuesterFailureKind::HazardUnsurvivable,
                        Arc::from(format!(
                            "step {id}: its damage {max} needs more than {hp_max} Hitpoints to clear the eat line {line}"
                        )),
                    );
                    self.parked = true;
                    self.dirty = true;
                    self.trace_parked(tick.output);
                    self.publish(tick.output);
                    return Ok(ScriptFlow::Blocked(self.blocked_failure()));
                }
                self.survival_deferred = true;
                self.dirty = true;
                self.publish(tick.output);
                return Ok(ScriptFlow::Continue);
            }
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
            let trace_snapshot = api::hostlog::enabled(api::hostlog::Category::ScriptTrace)
                .then(|| tick.frame().snapshot())
                .flatten();
            trace_root_event(
                &mut self.trace,
                tick.output,
                api::hostlog::Level::Info,
                self.path.id.0.as_ref(),
                stage,
                step,
                format_args!("begin"),
            );
            if let Some(snapshot) = trace_snapshot {
                api::host_log!(
                    api::hostlog::Category::ScriptTrace,
                    api::hostlog::Level::Info,
                    "quester {}: stage {stage} step {} ({}) player_info={}",
                    self.path.id.0,
                    step.id.0,
                    step.kind,
                    snapshot.gens().player_info
                );
            }
            #[cfg(test)]
            self.begun.push(Arc::clone(&step.id.0));
            let resume_page = self.open_page_retry.take() == Some((prelude, self.seq_index, index));
            let result = {
                let mut step_cx = StepContext {
                    tick,
                    quests: &self.quests,
                    progress: self.progress_slice(),
                    required_after,
                    banks: &self.banks,
                    choices: &self.choices,
                };
                if resume_page {
                    step.plan.begin_open_page(&mut step_cx)
                } else {
                    step.plan.begin(&mut step_cx)
                }
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
                Err(ActionError::BudgetExhausted) => {
                    // A composed begin can exhaust the transition allowance
                    // too. Retry without counting a failed step attempt.
                    self.pair_begin_since = None;
                    self.waiting = None;
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
                    self.record_step_begin_failure(error);
                }
            }
            // A begun step is polled on its begin tick unless the begin
            // already spent the tick's event (TICK-FIX #4, C-BEGIN-NO-POLL).
            if self.step.is_none() || !tick.cx.may_continue_this_tick() {
                self.publish(tick.output);
                return Ok(ScriptFlow::Continue);
            }
        }
        let needs_bank_scan = self
            .step
            .as_ref()
            .is_some_and(|step| step.needs_bank_scan());
        let bank_scan_active = self.provisioner.bank_phase().is_some();
        let bank_scan_status =
            self.step.is_some() && self.provisioner.status().phase == ProvisionPhase::Scanning;
        let bank_known = bank_known(tick);
        if (needs_bank_scan && !bank_known) || bank_scan_active || bank_scan_status {
            if needs_bank_scan && !bank_known && !bank_scan_active {
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
                if !self.needs_read
                    && self
                        .step
                        .as_ref()
                        .is_some_and(|step| step.needs_progress_read())
                {
                    self.needs_read = true;
                    self.dirty = true;
                    // An advancing acquire child reads on its completion
                    // tick, like an advancing root step (TICK-FIX #6).
                    if pass < SAME_TICK_PASSES
                        && tick.cx.may_continue_this_tick()
                        && !self.survival_deferred
                    {
                        return self.advance(tick, pass + 1);
                    }
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
                // The bank's rows are the host's to observe (design-bank-snapshot
                // §1.3); a bank receipt carries nothing the memory lacks.
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
                    self.read_after_kill = killed_npc_in_scene(tick, self.last_outcome.as_ref())
                        .map(|index| (index, tick.cx.evidence().tick + KILL_READ_BOUND_TICKS));
                }
                self.step = None;
                self.settling = true;
                self.settle_deadline = tick.cx.active_now()
                    + self
                        .current_step()
                        .map(|step| step.plan.settle_timeout())
                        .unwrap_or_default();
                // Settle, reread and select on the snapshot that proved the
                // completion instead of a tick later (TICK-FIX #5/#6, E-Q1/E-Q3).
                if pass < SAME_TICK_PASSES
                    && tick.cx.may_continue_this_tick()
                    && !self.survival_deferred
                {
                    return self.advance(tick, pass + 1);
                }
            }
            Poll::Ready(Err(
                error @ (ActionError::Blocked(_)
                | ActionError::CombatInterrupted
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
                let budget = self.current_attempt_budget();
                if let Some(budget) = &budget {
                    if !matches!(&error, ActionError::UserInput) {
                        self.record_budgeted_attempt(budget, false);
                    }
                }
                self.record_failure(error);
                self.step = None;
                self.last_outcome = None;
                self.prayer_cleanup_pending = !self.prayer_cleanup_owned.is_empty();
                self.parked = true;
                if let Some(budget) = &budget {
                    self.park_at_attempt_budget(budget);
                }
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
                // A page that is still open belongs to this step: its retry
                // continues the page where the content left the player.
                self.open_page_retry = page_open(&tick.cx).then_some((
                    self.in_prelude,
                    self.seq_index,
                    self.step_index,
                ));
                self.record_step_failure(error, tick);
            }
        }
        self.update_wait();
        self.publish(tick.output);
        Ok(ScriptFlow::Continue)
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
        tick.cx.walk_prereq = self.path.prereq;
        if tick.cx.run() != self.run {
            api::host_log!(
                api::hostlog::Category::ScriptLifecycle,
                api::hostlog::Level::Info,
                "quester {}: run key changed from {:?} to {:?}; dropping the step and progress",
                self.path.id.0,
                self.run,
                tick.cx.run()
            );
            self.run = tick.cx.run();
            // A new run starts a fresh trace: print the ending run's pending
            // repeat counts first, then give the current stage its own budget.
            // `stage` does not change here, so the stage hook will not run
            // again for it and this is the only place the new run is sized.
            self.trace.flush_repeats(tick.output);
            let units = self.stage_units();
            self.trace = RunTrace::default();
            self.trace.begin_stage(tick.output, units);
            self.watchdog = Watchdog::default();
            self.cancel_step(tick);
            self.survival = crate::survival::SurvivalStep::default();
            self.survival_action = None;
            self.survival_deferred = false;
            self.survival_interrupting = false;
            self.selected_hazard.clear();
            self.park_page_ticks = 0;
            self.park_page_seen = false;
            self.pair_admission = None;
            self.pair_admission_since = None;
            self.waiting = None;
            self.pair_admitted = false;
            self.last_combat = None;
            self.read_after_kill = None;
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
        if tick.cx.take_hold_teleport() {
            self.drop_step_for_teleport();
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
            api::host_log!(
                api::hostlog::Category::ScriptLifecycle,
                api::hostlog::Level::Info,
                "quester {}: the player died; dropping the step and progress (deaths so far {})",
                self.path.id.0,
                self.prior_deaths.saturating_add(self.deaths)
            );
            let exceeded = death_cap_exceeded(
                self.prior_deaths.saturating_add(self.deaths),
                self.max_deaths,
            );
            self.watchdog = Watchdog::default();
            self.cancel_step(tick);
            self.survival = crate::survival::SurvivalStep::default();
            self.survival_action = None;
            self.survival_deferred = false;
            self.survival_interrupting = false;
            self.selected_hazard.clear();
            self.park_page_ticks = 0;
            self.park_page_seen = false;
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
        // Poll the survival op first: when it settles (a page closed, a drink
        // applied or one the engine dropped after its lock), the survival poll
        // below takes this tick's interaction event for the next click instead
        // of losing a tick (design A6.5 drink bound: tick 0 close, tick 1 drink).
        let survival_op_live = self.poll_survival_action(tick);
        if !self.parked {
            self.poll_survival(tick);
            self.reconcile_park_page(tick);
        }
        if self.parked && self.survival.parked().is_some() {
            self.publish(tick.output);
            return Ok(ScriptFlow::Blocked(self.blocked_failure()));
        }
        if survival_op_live || self.survival_action.is_some() {
            self.publish(tick.output);
            return Ok(ScriptFlow::Continue);
        }
        if self.prayer_cleanup_pending
            && self.step.is_some()
            && !self.survival_deferred
            && !self.survival_interrupting
        {
            self.cancel_step(tick);
            self.dirty = true;
        }
        if let Some(result) = self
            .clear_prayers
            .as_ref()
            .map(|handle| tick.actions.poll(handle, &mut tick.cx))
        {
            match result {
                Poll::Pending if !self.survival_interrupting => {
                    self.publish(tick.output);
                    return Ok(ScriptFlow::Continue);
                }
                Poll::Pending => {}
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
        if self.prayer_cleanup_pending
            && self.step.is_none()
            && !self.settling
            && self.survival.parked().is_none()
            && !self.survival_deferred
            && !self.survival_interrupting
        {
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
        if self.survival.parked().is_some() {
            self.publish(tick.output);
            return Ok(ScriptFlow::Continue);
        }
        if self.survival_deferred
            && self.step.is_none()
            && !self.provisioner.has_run()
            && !self.survival_interrupting
        {
            self.publish(tick.output);
            return Ok(ScriptFlow::Continue);
        }
        let guarded_survival_step = self.survival_deferred && self.guarded_survival_step();
        let previous_cross = tick.cx.stage_cross.take();
        let flow = self.advance(tick, 0);
        tick.cx.stage_cross = previous_cross;
        let flow = flow?;
        let transfer_boundary = self.finish_survival_transfer(tick);
        let guarded_step_finished = guarded_survival_step && self.step.is_none();
        if transfer_boundary || guarded_step_finished {
            self.survival_deferred = false;
            self.poll_survival(tick);
            if self.poll_survival_action(tick) {
                return Ok(ScriptFlow::Continue);
            }
            if self.survival.parked().is_some() || self.survival_deferred {
                self.publish(tick.output);
                return Ok(ScriptFlow::Continue);
            }
        }
        Ok(flow)
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
        let preserve_survival_park =
            matches!(event, Interrupt::Resume) && self.survival.parked().is_some();
        self.gang_reader.cancel();
        self.watchdog = Watchdog::default();
        self.dirty = true;
        match event {
            Interrupt::Resume | Interrupt::SessionReady => {
                api::host_log!(
                    api::hostlog::Category::ScriptLifecycle,
                    api::hostlog::Level::Info,
                    "quester {}: {event:?} interrupt; dropping the step and progress so the operator's takeover or the new session re-reads from the top",
                    self.path.id.0
                );
                if !preserve_survival_park {
                    self.survival = crate::survival::SurvivalStep::default();
                }
                self.survival_action = None;
                self.survival_deferred = false;
                self.survival_interrupting = false;
                self.capture_prayer_cleanup();
                self.needs_read = true;
                self.step = None;
                self.provisioner.cancel();
                self.prayer_cleanup_pending = !self.prayer_cleanup_owned.is_empty();
                self.last_outcome = None;
                self.last_combat = None;
                self.settling = false;
                self.journal = None;
                self.journal_drain = None;
                self.progress = None;
                self.unreadable_since = None;
                self.journal_attempts = 0;
                self.journal_drains = 0;
                self.journal_drain_since = None;
                self.journal_retry_pending = false;
                self.journal_quiet_since = None;
                self.selection_since = None;
                self.dirty = true;
                self.waiting = None;
            }
            Interrupt::SessionEnded => {
                api::host_log!(
                    api::hostlog::Category::ScriptLifecycle,
                    api::hostlog::Level::Info,
                    "quester {}: session ended; dropping the step and progress",
                    self.path.id.0
                );
                self.survival = crate::survival::SurvivalStep::default();
                self.survival_action = None;
                self.survival_deferred = false;
                self.survival_interrupting = false;
                self.selected_hazard.clear();
                self.park_page_ticks = 0;
                self.park_page_seen = false;
                self.capture_prayer_cleanup();
                self.step = None;
                self.provisioner.cancel();
                self.prayer_cleanup_pending = !self.prayer_cleanup_owned.is_empty();
                self.last_outcome = None;
                self.last_combat = None;
                self.journal = None;
                self.journal_drain = None;
                self.progress = None;
                self.settling = false;
                self.needs_read = true;
                self.journal_attempts = 0;
                self.journal_drains = 0;
                self.journal_drain_since = None;
                self.journal_retry_pending = false;
                self.journal_quiet_since = None;
                self.waiting = None;
            }
            Interrupt::Pause | Interrupt::Hold(_) => {}
        }
    }
    /// A random event keeps the quest's own progress and its in-flight step.
    /// The engine spawns the event NPC beside the player on a login timer
    /// (`login.rs2:40` sets `general_macro_events` to 500 ticks;
    /// `macro_events.rs2:2-12` spawns it), so every bot meets one about 500
    /// ticks after login, in the middle of whatever step it runs. Dropping the
    /// step and progress there made the next journal read restart the step
    /// from cursor 0 with no failure (QUESTER-READ-KEEPS-STEP). The guardian
    /// holds the script while it answers the event
    /// (`RandomStatus::hold`, only with `settings.random_events` on), and the
    /// step's own checks decide what a held tick did, as they already do for
    /// partner work.
    ///
    /// Nothing here depends on the event's name. Two things follow the hold,
    /// both by observation:
    /// - A step whose action was interrupted (the guardian sent its own
    ///   interaction, or the host dropped its queued request during the hold)
    ///   re-issues that request once, after one observed tick and only if its
    ///   goal is not already met ([`crate::native::NativeMachine::after_hold`]).
    /// - If the player was teleported while the hold lasted
    ///   ([`crate::native::is_teleport`]), the step handle is dropped
    ///   ([`Self::drop_step_for_teleport`]); progress and the ordered cursor
    ///   stay.
    ///
    /// With `settings.random_events` off the guardian neither holds nor acts
    /// (`guardian.rs`: `hold` and `ours` need the setting), so the event is not
    /// answered and the guardian does not interrupt the step. The Quester
    /// keeps the step, progress and cursor exactly as with the setting on and
    /// no teleport; it does not classify a teleport that no hold surrounds,
    /// and the step's own checks judge whatever the ignored event does to the
    /// player (an attack, a content teleport, a trapped square).
    fn on_random(&mut self, event: &DetectedRandom) -> RandomClaim {
        self.watchdog = Watchdog::default();
        self.custom_reader = None;
        self.custom_read_after = None;
        self.gang_reader.cancel();
        api::host_log!(
            api::hostlog::Category::ScriptLifecycle,
            api::hostlog::Level::Info,
            "quester {}: random event {:?} {:?} keeps the step, progress and cursor {} (step in flight: {}, partner work pending: {})",
            self.path.id.0,
            event.kind,
            event.name,
            self.cursor,
            self.step.is_some(),
            self.pair_work_pending()
        );
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
        self.journal_drain = None;
        self.settling = false;
        self.dirty = true;
        self.waiting = None;
        self.clear_prayers = None;
        self.prayer_cleanup_pending = false;
        self.last_outcome = None;
        self.last_combat = None;
        self.survival_action = None;
        self.survival_deferred = false;
        self.survival_interrupting = false;
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

    fn waits_for_evidence(&self) -> bool {
        self.survival.parked().is_some()
            && !self.parked
            && self.survival_action.is_none()
            && self.step.is_none()
            && !self.provisioner.has_run()
            && self.clear_prayers.is_none()
            && self.journal.is_none()
            && self.journal_drain.is_none()
            && self.custom_reader.is_none()
            && self.pair_admission.is_none()
            && !self.survival_interrupting
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
    preparing: Option<PreparedTask<Result<Arc<CompiledPath>, Arc<str>>>>,
    gang_reader: super::gang::GangRead,
    pair_selection: Option<(usize, super::pair::Gang)>,
    completed: u16,
    deaths: u16,
    max_deaths: u8,
    anchor: Option<api::WorldTile>,
    fields: Arc<[StatusField]>,
    gate_fields: Arc<[StatusField]>,
    quest_status_since: Option<Duration>,
    /// An open bank that this run's provisioning left, owed a close before
    /// anything else acts. Ended sessions hand their lease here.
    owed_bank: Option<BankLease>,
    owed_closer: BankCloser,
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
            owed_bank: None,
            owed_closer: BankCloser::default(),
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
        self.owed_bank = retained.bank_lease;
        self.refresh_fields();
    }

    fn sync_retained(&self, tick: &mut NativeTick<'_>) {
        let retained = tick.cx.retained().quester();
        retained.anchor = self.anchor;
        retained.deaths = self.deaths;
        retained.completed = self.completed;
    }

    /// Takes the lease of an ended provisioning session from the active
    /// Quester. It then owes the close. Called after anything that can end a
    /// session without a close: an error, a cancel, a resume, a random event.
    /// Never called on Stop: a Stop sends no close.
    fn reclaim_bank(&mut self) {
        if let Some(lease) = self
            .active
            .as_mut()
            .and_then(|active| active.take_unowned_bank_lease())
        {
            self.owed_bank = Some(lease);
        }
    }

    /// Closes the bank provisioning left open, through the bank kernel's
    /// close. `true` while the close runs. This runs after the held-tick check
    /// and before any journal read, Path compile or step, so a resumed or
    /// restarted run does not read under the bank. It sends nothing at zero
    /// hitpoints, and nothing unless this frame still shows the leased bank
    /// open. A close the kernel gives up on is logged and dropped, which
    /// bounds it.
    fn settle_owed_bank(&mut self, tick: &mut NativeTick<'_>) -> bool {
        self.reclaim_bank();
        if self.owed_bank.is_none() && !self.owed_closer.active() {
            return false;
        }
        if crate::native::death::hitpoints_zero(tick.cx.snapshot().stats().map(|stats| stats.value))
        {
            return false;
        }
        match self.owed_closer.drive(tick, &mut self.owed_bank) {
            Closed::Done => false,
            Closed::Pending => true,
            Closed::Failed(error) => {
                tick.output.log(
                    api::hostlog::Level::Warn,
                    &format!("quester queue: the open bank was not closed: {error}"),
                );
                false
            }
        }
    }

    /// Keeps the lease in the slot's retained cell, so an automatic restart
    /// can close the bank. Operator Stop discards that cell.
    fn sync_bank_lease(&self, tick: &mut NativeTick<'_>) {
        tick.cx.retained().quester().bank_lease = self
            .owed_bank
            .or_else(|| self.active.as_ref().and_then(|active| active.bank_lease()));
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
        if self.active.is_some()
            && self
                .active_index
                .is_some_and(|index| self.queue.path_is_draft(index))
        {
            static DRAFT_STATUS: std::sync::LazyLock<Arc<str>> =
                std::sync::LazyLock::new(|| Arc::from("Untested draft Path"));
            fields.push(StatusField {
                key: "draft_status",
                label: "Path status",
                value: StatusValue::Text(Arc::clone(&DRAFT_STATUS)),
            });
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
            // The active executor is gone by now. The row keeps where it
            // stopped, so a Blocked status names the quest, stage and step
            // instead of only the queue reason.
            if let Some(parked) = self
                .blocked_row()
                .and_then(|index| self.queue.parked(index))
            {
                static UNKNOWN_STAGE: std::sync::LazyLock<Arc<str>> =
                    std::sync::LazyLock::new(|| Arc::from("unknown"));
                static BLOCKED_ACTION: std::sync::LazyLock<Arc<str>> =
                    std::sync::LazyLock::new(|| Arc::from("blocked"));
                fields.extend([
                    StatusField {
                        key: "stage",
                        label: "Stage",
                        value: StatusValue::Text(match &parked.stage {
                            Some(stage) => Arc::clone(stage),
                            None => Arc::clone(&UNKNOWN_STAGE),
                        }),
                    },
                    StatusField {
                        key: "quest_id",
                        label: "Quest ID",
                        value: StatusValue::Text(Arc::clone(&parked.quest)),
                    },
                    StatusField {
                        key: "step_id",
                        label: "Step",
                        value: StatusValue::Text(Arc::clone(&parked.step)),
                    },
                    StatusField {
                        key: "last_failure",
                        label: "Last failure",
                        value: StatusValue::Text(Arc::clone(&parked.failure)),
                    },
                    StatusField {
                        key: "action_state",
                        label: "Action",
                        value: StatusValue::Text(Arc::clone(&BLOCKED_ACTION)),
                    },
                ]);
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
            survival: self
                .active
                .as_ref()
                .map_or_else(crate::survival::SurvivalReport::default, |active| {
                    active.survival.report()
                }),
            failure,
        });
    }

    /// The row `blocked()` reports: the first picked, unskipped row with a
    /// reason. The Blocked status reads its parked detail, so the published
    /// fields and the failure message describe the same row.
    fn blocked_row(&self) -> Option<usize> {
        self.queue
            .rows()
            .iter()
            .enumerate()
            .filter(|(_, row)| row.picked && !row.skipped)
            .find(|(index, _)| self.queue.reason(*index).is_some())
            .map(|(index, _)| index)
    }

    fn blocked(&self) -> ScriptFailure {
        if self.quest_status_since.is_some() {
            return ScriptFailure {
                code: Arc::from("quest-status-unobserved"),
                message: Arc::from(
                    "quest list was not observed within 30 seconds: log out and log in normally outside the tutorial (finish it if needed), then start the Quester again",
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
                    "no quests selected: review Quests and Skip in Script prefs; empty Quests selects all released quests, then start the Quester again",
                ),
            };
        }
        let reported = self.blocked_row();
        let reason = reported
            .and_then(|index| self.queue.reason(index))
            .unwrap_or("no eligible selected quest");
        let status = reported
            .and_then(|index| self.queue.status(index))
            .unwrap_or(QueueStatus::Blocked);
        // A requirement block can be changed in Script prefs; a parked run
        // (a refused walk, a failed action) can't, so don't send the user there.
        let recovery = if status == QueueStatus::Blocked {
            "review Quests/Skip and requirements in Script prefs, resolve the condition, then start the Quester again"
        } else {
            "resolve the condition, then start the Quester again"
        };
        ScriptFailure {
            code: Arc::from("queue-blocked"),
            message: format!("{reason}; {recovery}").into(),
        }
    }

    fn activate(&mut self, tick: &mut NativeTick<'_>, path: Arc<CompiledPath>) {
        let index = self.active_index.expect("preparing queue row");
        let result = super::eligibility::evaluate(&path, &tick.cx.snapshot(), &self.quests);
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
        let closing = self.settle_owed_bank(tick);
        self.sync_bank_lease(tick);
        if closing {
            self.publish(tick.output, NativePhase::Waiting);
            return Ok(ScriptFlow::Continue);
        }
        if let Some(active) = self.active.as_mut() {
            let flow = active.tick(tick)?;
            self.anchor = active.anchor;
            tick.cx.retained().quester().anchor = self.anchor;
            self.reclaim_bank();
            self.sync_bank_lease(tick);
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
                            // The executor is dropped here; keep where it
                            // stopped so the queue-level Blocked status still
                            // names the stage and step that parked it.
                            self.queue.mark_parked(
                                index,
                                super::queue::ParkedRun {
                                    quest: Arc::clone(&active.path.id.0),
                                    stage: active.stage.as_ref().map(|stage| Arc::clone(&stage.0)),
                                    step: active
                                        .parked_step
                                        .as_ref()
                                        .map(|parked| Arc::clone(&parked.id))
                                        .or_else(|| {
                                            active.current_step().map(|step| Arc::clone(&step.id.0))
                                        })
                                        .unwrap_or_else(|| {
                                            static EMPTY_STEP: std::sync::LazyLock<Arc<str>> =
                                                std::sync::LazyLock::new(|| Arc::from(""));
                                            Arc::clone(&EMPTY_STEP)
                                        }),
                                    failure: Arc::clone(&failure.message),
                                },
                            );
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
                        Err(error) => Arc::from(format!("Path compiler worker: {error}")),
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
            // The operator's rows are leased here, on the slot thread, from
            // the bound state capability; the compile runs off the pump
            // with that immutable snapshot. A cold domain waits a tick; a
            // refused loadouts file does not repair itself on a tick, so the
            // Path is blocked with the reason instead.
            let operator = match tick.operator_loadouts() {
                Ok(operator) => operator,
                Err(code @ api::local_state::ErrorCode::NotReady { .. }) => {
                    tick.output.log(
                        api::hostlog::Level::Info,
                        &format!("quester queue: Path {id} waits for operator loadouts: {code}"),
                    );
                    self.publish(tick.output, NativePhase::Waiting);
                    return Ok(ScriptFlow::Continue);
                }
                Err(code) => {
                    self.queue
                        .mark_blocked(index, Arc::from(format!("operator loadouts: {code}")));
                    self.refresh_fields();
                    return Ok(ScriptFlow::Continue);
                }
            };
            let bytes = self.queue.path_bytes(index);
            let selected = Arc::clone(&self.selected);
            let quests = Arc::clone(&self.quests);
            let nav = tick.frame().compiled().nav.cloned();
            self.active_index = Some(index);
            let input = PreparationInput::new(
                self.run.run,
                bytes.as_ref().map_or(0, |source| source.as_ref().len()),
                2 * 1024 * 1024,
                2 * 1024 * 1024,
            );
            let slot = SlotKey::from_incarnation(self.run.slot);
            let worker = match (input, slot) {
                (Ok(input), Some(slot)) => preparation::submit(
                    PreparationKind::Profile,
                    Scope::Slot(slot),
                    input.with_quester_progress_compile(),
                    move |cap| {
                        let bytes = bytes
                            .ok_or_else(|| Arc::<str>::from(format!("Path {id} is unavailable")))?;
                        let compiled = super::compile::compile_path_for_gang(
                            bytes.as_ref(),
                            &selected,
                            &quests,
                            cap,
                            gang,
                            &operator,
                            nav.as_ref()
                                .and_then(nav_planner::contract::NavLease::zones),
                        );
                        compiled.map_err(|error| {
                            let detail =
                                error.detail.as_deref().unwrap_or("Path compilation failed");
                            Arc::from(match &error.step {
                                Some(step) => format!("{} [{}]: {detail}", error.code, step.0),
                                None => format!("{}: {detail}", error.code),
                            })
                        })
                    },
                ),
                (Err(error), _) => Err(error.into()),
                (_, None) => {
                    self.queue
                        .mark_blocked(index, "Path preparation: slot not bound".into());
                    self.refresh_fields();
                    self.dirty = true;
                    self.publish(tick.output, NativePhase::Preparing);
                    return Ok(ScriptFlow::Continue);
                }
            };
            match worker {
                Ok(worker) => {
                    tick.output.log(
                        api::hostlog::Level::Info,
                        &format!(
                            "quester queue: preparing next Path {}",
                            self.queue.id(index).expect("selected queue row")
                        ),
                    );
                    self.preparing = Some(worker);
                }
                Err(preparation::PreparationError::Refused(api::work::SubmissionRefusal::Full)) => {
                    // No Path failed. The bounded queue retains this candidate
                    // and the next game tick retries admission without blocking.
                    self.active_index = None;
                }
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
        if !matches!(event, Interrupt::Hold(_)) {
            self.owed_closer.cancel();
        }
        if let Some(active) = self.active.as_mut() {
            active.interrupt(event);
        }
        self.reclaim_bank();
    }

    fn on_stop(&mut self, reason: StopReason) {
        self.gang_reader.cancel();
        self.pair_selection = None;
        if let Some(preparing) = &mut self.preparing {
            preparing.cancel(Cancellation::Operator);
        }
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
        let claim = self
            .active
            .as_mut()
            .map_or(RandomClaim::Host, |active| active.on_random(event));
        self.reclaim_bank();
        claim
    }

    fn recovery_anchor(&self) -> Option<api::WorldTile> {
        self.active
            .as_ref()
            .and_then(|active| active.recovery_anchor())
            .or(self.anchor)
    }
    fn waits_for_evidence(&self) -> bool {
        self.active
            .as_ref()
            .is_some_and(|active| active.waits_for_evidence())
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
        if !api::test_support::game_data_available() {
            return;
        }
        #[derive(Default)]
        struct Capture(Vec<ScriptStatus>);
        impl NativeOutput for Capture {
            fn status(&mut self, status: ScriptStatus) {
                self.0.push(status);
            }
            fn paint(&mut self, _: Arc<crate::paint::ScriptPaint>) {}
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
        if !api::test_support::game_data_available() {
            return;
        }
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
    fn budget_counts_begin_and_poll_failures_together() {
        if !api::test_support::game_data_available() {
            return;
        }
        let (mut script, snapshot) = fixture();
        {
            let path = Arc::get_mut(&mut script.path).expect("unshared path");
            path.sequences[0].steps[0].attempt_budget = Some(StepAttemptBudget {
                max_attempts: 2,
                stop: Arc::from("retry-stop"),
            });
        }
        let attempt_step = script.path.sequences[0].steps[0].id.clone();
        script.reset_attempt_scope_if_needed(&attempt_step);

        script.record_step_begin_failure(ActionError::Failed(Arc::from("begin failed")));
        assert_eq!(script.step_attempts, 1);
        assert!(!script.parked);

        let mut ledger = None;
        super::super::families::tests::with_tick(&snapshot, &mut ledger, 1, |tick| {
            script.record_step_failure(ActionError::Failed(Arc::from("poll failed")), tick);
        });
        assert_eq!(script.step_attempts, 2);
        assert!(script.parked);
        assert!(script
            .last_error
            .as_deref()
            .is_some_and(|error| error.contains("retry-stop")));
        let receipt = script
            .last_attempt_receipt
            .as_deref()
            .expect("the exhausted attempt has a receipt");
        assert_eq!(receipt.attempt_count(), Some(2));
        assert_eq!(
            receipt.attempt_stop().map(|stop| stop.as_ref()),
            Some("retry-stop")
        );
    }

    #[test]
    fn begin_user_input_parks_without_spending_the_step_budget() {
        if !api::test_support::game_data_available() {
            return;
        }
        let (mut script, _) = fixture();
        {
            let path = Arc::get_mut(&mut script.path).expect("unshared path");
            path.sequences[0].steps[0].attempt_budget = Some(StepAttemptBudget {
                max_attempts: 1,
                stop: Arc::from("retry-stop"),
            });
        }
        let attempt_step = script.path.sequences[0].steps[0].id.clone();
        script.reset_attempt_scope_if_needed(&attempt_step);

        script.record_step_begin_failure(ActionError::UserInput);

        assert_eq!(script.step_attempts, 0);
        assert_eq!(script.fail_streak, 0);
        assert!(script.parked);
        assert!(script.last_attempt_receipt.is_none());
        assert_eq!(script.blocked_failure().code.as_ref(), "manual-movement");
    }

    #[test]
    fn alternating_budgeted_steps_hit_the_runwide_failure_bound() {
        if !api::test_support::game_data_available() {
            return;
        }
        let (mut script, snapshot) = fixture();
        let sequence_index = script
            .path
            .sequences
            .iter()
            .position(|sequence| sequence.steps.len() >= 2)
            .expect("Cook has a sequence with two steps");
        let mut step_ids = Vec::with_capacity(2);
        {
            let path = Arc::get_mut(&mut script.path).expect("unshared path");
            for step in path.sequences[sequence_index].steps.iter_mut().take(2) {
                step.attempt_budget = Some(StepAttemptBudget {
                    max_attempts: 32,
                    stop: Arc::from("retry-stop"),
                });
                step_ids.push(step.id.clone());
            }
        }
        script.seq_index = sequence_index;

        let mut ledger = None;
        for failed_attempt in 0..32u8 {
            let step_index = usize::from(failed_attempt % 2);
            script.step_index = step_index;
            script.stage = Some(FactKey::new(if step_index == 0 {
                "alternating-stage-a"
            } else {
                "alternating-stage-b"
            }));
            script.reset_attempt_scope_if_needed(&step_ids[step_index]);
            super::super::families::tests::with_tick(
                &snapshot,
                &mut ledger,
                u64::from(failed_attempt) + 1,
                |tick| {
                    script.record_step_failure(
                        ActionError::Failed(Arc::from("alternating failure")),
                        tick,
                    );
                },
            );
            assert_eq!(script.fail_streak, failed_attempt + 1);
            assert_eq!(script.parked, failed_attempt == 31);
            if failed_attempt < 31 {
                assert_eq!(script.step_attempts, 1);
            }
        }
        assert!(script
            .last_error
            .as_deref()
            .is_some_and(|message| message.contains("run retry limit")));
        assert_eq!(script.step_attempts, 1);
    }

    #[test]
    fn budgeted_failures_still_observe_the_no_progress_watchdog() {
        if !api::test_support::game_data_available() {
            return;
        }
        let (mut script, snapshot) = fixture();
        let sequence_index = script
            .path
            .sequences
            .iter()
            .position(|sequence| !sequence.steps.is_empty())
            .expect("Cook has a nonempty sequence");
        {
            let path = Arc::get_mut(&mut script.path).expect("unshared path");
            path.sequences[sequence_index].steps[0].attempt_budget = Some(StepAttemptBudget {
                max_attempts: 32,
                stop: Arc::from("retry-stop"),
            });
        }
        script.seq_index = sequence_index;
        script.step_index = 0;
        script.stage = Some(FactKey::new("watchdog-stage"));
        let step_id = script.current_step().expect("current step").id.clone();
        script.reset_attempt_scope_if_needed(&step_id);

        let mut ledger = None;
        for tick in 1..=9 {
            super::super::families::tests::with_tick(&snapshot, &mut ledger, tick, |native| {
                script.record_step_failure(
                    ActionError::Failed(Arc::from("unchanged failure")),
                    native,
                );
            });
            if script.parked {
                break;
            }
        }

        assert!(
            script.parked,
            "the no-progress watchdog still parks the run"
        );
        assert_eq!(script.watchdog.unchanged(), 8);
        assert_eq!(script.step_attempts, 9);
    }

    /// The Quester holds no bank table of its own (design-bank-snapshot D8):
    /// the account's memory is the host's and reaches it as a borrow.
    #[test]
    fn quester_struct_fits_the_per_bot_budget() {
        let bytes = std::mem::size_of::<Quester>();
        // 3640 B before the dev 900f2dccd merge. Dev's revision-377 journal work
        // added `layout: Layout` to `quest_journal::native::JournalMachine`
        // (192 -> 200 B); the Quester holds two `ActionHandle<JournalMachine>`
        // (`journal`, `gang_reader`), +8 B each. No Quester field of this input grew.
        // The shop phase then grew `AcquireRun` by 40 B, and the Quester's
        // `Provisioner` holds one inline: 3696 B (macOS aarch64).
        assert_eq!(bytes, 3696, "Quester changed its measured per-bot layout");
        assert!(
            std::mem::size_of::<crate::survival::SurvivalStep>() <= 192,
            "SurvivalStep={} exceeds its 192 B bound",
            std::mem::size_of::<crate::survival::SurvivalStep>()
        );
        assert!(std::mem::size_of::<QueuedQuester>() < 4096);
        eprintln!("Quester size_of={bytes}; bank memory borrowed, heap=0");
        eprintln!(
            "upkeep layouts SelectedHazard={} StepHazard={} CompiledStep={} SurvivalStep={} SurvivalReport={} OneOp={}",
            std::mem::size_of::<SelectedHazard>(),
            std::mem::size_of::<crate::survival::StepHazard>(),
            std::mem::size_of::<CompiledStep>(),
            std::mem::size_of::<crate::survival::SurvivalStep>(),
            std::mem::size_of::<crate::survival::step::SurvivalReport>(),
            std::mem::size_of::<OneOp>(),
        );
        assert!(bytes < 4096, "Quester is {bytes} bytes");
    }
    #[test]
    fn quester_capability_status_field_size_is_bounded() {
        let bytes = std::mem::size_of::<StatusField>();
        eprintln!("Quester capability StatusField={bytes}; combat adds one, timed take adds two");
        assert!(bytes <= 56, "capability status field grew to {bytes} bytes");
    }

    #[test]
    fn family_receipts_are_exposed_in_quester_status() {
        if !api::test_support::game_data_available() {
            return;
        }
        use crate::combat::{CombatEnd, CombatReport};
        use api::quest_progress::EvidenceStamp;

        #[derive(Default)]
        struct Capture(Vec<ScriptStatus>);
        impl NativeOutput for Capture {
            fn status(&mut self, status: ScriptStatus) {
                self.0.push(status);
            }
            fn paint(&mut self, _: Arc<crate::paint::ScriptPaint>) {}
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
            pvp_blocked: 0,
            pvp_source: None,
            kill_line_seen: false,
            switches: 0,
            switch_failed: 0,
            active_style: crate::combat::Style::Melee,
        };
        script.last_outcome = Some(StepOutcome {
            progress: None,
            evidence: report.evidence,
            receipt: Some(Arc::new(CombatReceipt {
                report,
                target_gone_restarts: 1,
                kills: 0,
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
                kills: 1,
            })),
        });
        script.publish(&mut output);
        assert_eq!(output.0.len(), 2);
        assert!(output.0[1].fields.iter().any(|field| {
            field.key == "combat_end"
                && matches!(&field.value, StatusValue::Text(value) if value.as_ref() == "Killed")
        }));
        assert!(output.0[1].fields.iter().any(|field| {
            field.key == "combat_kills" && matches!(field.value, StatusValue::Integer(1))
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
                kills: 0,
            })),
        });
        script.publish(&mut output);
        assert_eq!(output.0.len(), 4);
        assert!(output.0[3].fields.iter().any(|field| {
            field.key == "combat_end"
                && matches!(&field.value, StatusValue::Text(value) if value.as_ref() == "TargetGone")
        }));
        assert!(output.0[3].fields.iter().any(|field| {
            field.key == "combat_kills" && matches!(field.value, StatusValue::Integer(0))
        }));
        script.last_combat = None;
        script.last_outcome = Some(StepOutcome {
            progress: None,
            evidence: report.evidence,
            receipt: Some(Arc::new(
                super::super::families::timed_ground::TimedGroundReceipt {
                    waited_ticks: 100,
                    held: 1,
                },
            )),
        });
        script.publish(&mut output);
        assert_eq!(output.0.len(), 5);
        for (key, expected) in [("ground_waited_ticks", 100), ("ground_held", 1)] {
            assert!(output.0[4].fields.iter().any(|field| {
                field.key == key
                    && matches!(field.value, StatusValue::Integer(value) if value == expected)
            }));
        }
        let allocations = allocation_counter::measure(|| {
            for _ in 0..200 {
                script.publish(&mut output);
            }
        });
        assert_eq!(allocations.count_total, 0);
        assert_eq!(
            output.0.len(),
            5,
            "unchanged timed receipts are not republished"
        );
    }

    #[test]
    fn no_food_abort_walks_then_parks_with_receipt_without_advancing() {
        if !api::test_support::game_data_available() {
            return;
        }
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
            fn paint(&mut self, _: Arc<crate::paint::ScriptPaint>) {}
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
                    &crate::local_state::OperatorLoadouts::absent(),
                    None,
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
            goal: None,
            request_id,
            evidence: EvidenceStamp {
                run,
                tick: 26,
                sequence: 26,
            },
            end: WalkEnd::Arrived,
            blocked: None,
            detail: None,
            refusal: None,
            assessment: None,
            escape: None,
            prereq_allow: None,
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
        if !api::test_support::game_data_available() {
            return;
        }
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
        let begun = script.begun.len();
        with_tick(&s, &mut ledger, 5003, |t| script.tick(t).unwrap());
        assert!(
            script.begun.len() > begun,
            "a new sequence 3 message settles at tick 5003 and the next step begins"
        );
    }

    #[test]
    fn completed_step_settles_and_hands_over_on_its_ready_tick() {
        if !api::test_support::game_data_available() {
            return;
        }
        use super::super::families::tests::with_tick_output;
        use api::snapshot::{GameSnapshot, QuestStatusView};
        let data = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
        let quests = Arc::new(QuestCatalog::from_identity(data.quest_identity()).unwrap());
        let mut document = super::super::compile::decode_cook().unwrap();
        document.quest.as_mut().unwrap().owns_inventory = true;
        let step = &mut document.roles[0].sequences[0].steps[0];
        step.kind = "wait".into();
        step.args = serde_json::json!({"until":{"All":[]},"max_ticks": 10});
        step.advances = Some(false);
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
        // TICK-FIX #4/#5 (E-Q1/E-Q2): the wait is begun and polled on the same
        // tick, completes, settles on the snapshot that proved it, and the
        // next step is selected and begun, all within tick 5000.
        let mut ledger = None;
        let mut output = TraceCapture::default();
        with_tick_output(&s, &mut ledger, 5000, &mut output, |t| {
            script.tick(t).unwrap()
        });
        let lines: Vec<&str> = output.logs.iter().map(|(_, line)| line.as_str()).collect();
        let first = lines
            .iter()
            .position(|line| line.ends_with("(wait) begin"))
            .unwrap_or_else(|| panic!("the wait step begins on tick 5000: {lines:#?}"));
        let settled = lines
            .iter()
            .position(|line| line.ends_with("(wait) settled"))
            .unwrap_or_else(|| panic!("the wait step settles on tick 5000: {lines:#?}"));
        assert!(first < settled);
        // The authored sequence selects the same wait again; the run trace
        // folds that repeated `begin` line into its entry's repeat count and
        // keeps the line text for it.
        let begins = script
            .trace
            .repeated
            .iter()
            .find(|repeated| repeated.line.ends_with("(wait) begin"))
            .map_or(0, |repeated| {
                script.trace.entries[repeated.entry].repeats + 1
            });
        assert!(
            begins >= 2,
            "the next step begins on the settle tick: {begins} begin(s), {lines:#?}"
        );
    }

    /// TICK-FIX #4 (C-BEGIN-NO-POLL, E-Q2): the root step is polled on the
    /// tick it is begun, so its first action (here the talk's approach walk
    /// or Talk-to) is already queued when that tick ends.
    #[test]
    fn begun_root_step_acts_on_its_begin_tick() {
        if !api::test_support::game_data_available() {
            return;
        }
        use super::super::families::tests::with_tick;
        let (mut script, snapshot) = status_fixture(super::super::compile::decode_cook().unwrap());
        let mut ledger = None;
        with_tick(&snapshot, &mut ledger, 1, |t| script.tick(t).unwrap());
        assert!(script.step.is_some(), "the start talk is begun on tick 1");
        assert!(
            ledger
                .as_ref()
                .is_some_and(|ledger| !ledger.outbox.is_empty()),
            "the begun talk acts on tick 1"
        );
    }

    #[test]
    fn same_tick_receipt_and_settle_do_not_fail_the_next_step_begin() {
        if !api::test_support::game_data_available() {
            return;
        }
        use super::super::families::tests::{with_tick, with_tick_snapshots};
        use super::super::path::{PredicateDocument, SequenceOrder, StepDocument};
        use api::snapshot::{GameSnapshot, QuestStatusView, SnapshotView, VarpView};

        let mut document = super::super::compile::decode_cook().unwrap();
        let sequence = &mut document.roles[0].sequences[0];
        sequence.order = SequenceOrder::Ordered;
        sequence.steps = [("retaliate-off", false), ("retaliate-on", true)]
            .into_iter()
            .map(|(id, retaliate)| StepDocument {
                id: FactKey::new(id),
                kind: "setting".into(),
                version: 1,
                args: serde_json::json!({ "retaliate": retaliate }),
                comment: None,
                advances: Some(false),
                max_attempts: None,
                stop: None,
                hazard: None,
                skip_if: PredicateDocument::Any(vec![]),
                settle: PredicateDocument::All(vec![]),
            })
            .collect();
        let (mut script, mut before) = status_fixture(document);
        before.seed_varps(vec![VarpView {
            index: crate::combat::OPTION_NODEF,
            value: 0,
        }]);
        let mut after = GameSnapshot::new();
        after.seed_ingame(2);
        after.seed_quest_statuses(
            vec![QuestStatusView {
                name: "Cook's Assistant".into(),
                component_id: 42,
                colour: 0xf80000,
            }],
            true,
        );
        after.seed_varps(vec![VarpView {
            index: crate::combat::OPTION_NODEF,
            value: 1,
        }]);
        // Existing genuine failures are retained, but pumps must not add a fifth.
        script.attempts = 4;
        let mut ledger = None;
        with_tick_snapshots(&before, &after, &mut ledger, 1, |tick, after| {
            script.tick(tick).unwrap();
            assert!(script.step.is_some());
            assert!(tick.cx.interaction_event_spent());
            let authority = tick.cx.ledger.as_ref().unwrap().outbox[0].authority();
            let evidence = tick.cx.evidence();
            tick.cx.ledger.as_mut().unwrap().complete_interaction(
                &authority,
                crate::native::InteractionReceipt {
                    request_id: authority.request_id().get(),
                    evidence,
                    accepted: true,
                    chat_since: 0,
                },
            );
            tick.cx.snapshot = SnapshotView::new(Some(after), tick.cx.evidence());
            tick.frame.snapshot = Some(after);
            script.tick(tick).unwrap();
            assert!(
                script.settling,
                "the real setting receipt is ready on tick 1"
            );
            script.tick(tick).unwrap();
            assert!(
                !script.settling,
                "the trivial settle is also ready on tick 1"
            );
            assert_eq!(script.cursor, 1);
            for _ in 0..16 {
                script.tick(tick).unwrap();
                assert!(!script.parked);
                assert!(script.step.is_none());
                assert_eq!(script.attempts, 4);
                assert_eq!(script.fail_streak, 0);
                assert!(script.last_error.is_none());
            }
            assert_eq!(tick.cx.ledger.as_ref().unwrap().outbox.len(), 1);
        });
        with_tick(&after, &mut ledger, 2, |tick| {
            script.tick(tick).unwrap();
            assert!(script.step.is_some(), "the second setting begins next tick");
            assert_eq!(script.step_index, 1);
            let queued = &tick.cx.ledger.as_ref().unwrap().outbox;
            assert_eq!(queued.len(), 1);
            assert!(matches!(
                queued[0].effect,
                crate::native::HostEffect::Interaction(
                    crate::interact::InteractReq::SetRetaliate { on: true }
                )
            ));
        });
        assert_eq!(script.attempts, 4);
        assert!(!script.parked);
    }

    #[test]
    fn scene_entry_defers_fresh_step_selection() {
        if !api::test_support::game_data_available() {
            return;
        }
        use super::super::families::tests::with_tick_snapshots;
        use super::super::path::{PredicateDocument, SequenceOrder, StepDocument};
        use api::snapshot::{GameSnapshot, QuestStatusView, SnapshotView, VarpView};

        let mut document = super::super::compile::decode_cook().unwrap();
        let sequence = &mut document.roles[0].sequences[0];
        sequence.order = SequenceOrder::Ordered;
        sequence.steps = [("retaliate-off", false), ("retaliate-on", true)]
            .into_iter()
            .map(|(id, retaliate)| StepDocument {
                id: FactKey::new(id),
                kind: "setting".into(),
                version: 1,
                args: serde_json::json!({ "retaliate": retaliate }),
                comment: None,
                advances: Some(false),
                max_attempts: None,
                stop: None,
                hazard: None,
                skip_if: PredicateDocument::Any(vec![]),
                settle: PredicateDocument::All(vec![]),
            })
            .collect();
        let (mut script, mut before) = status_fixture(document);
        before.seed_varps(vec![VarpView {
            index: crate::combat::OPTION_NODEF,
            value: 0,
        }]);
        let mut after = GameSnapshot::new();
        after.seed_ingame(2);
        after.seed_quest_statuses(
            vec![QuestStatusView {
                name: "Cook's Assistant".into(),
                component_id: 42,
                colour: 0xf80000,
            }],
            true,
        );
        after.seed_varps(vec![VarpView {
            index: crate::combat::OPTION_NODEF,
            value: 1,
        }]);

        let mut ledger = None;
        with_tick_snapshots(&before, &after, &mut ledger, 1, |tick, after| {
            script.tick(tick).unwrap();
            let authority = tick.cx.ledger.as_ref().unwrap().outbox[0].authority();
            let evidence = tick.cx.evidence();
            tick.cx.ledger.as_mut().unwrap().complete_interaction(
                &authority,
                crate::native::InteractionReceipt {
                    request_id: authority.request_id().get(),
                    evidence,
                    accepted: true,
                    chat_since: 0,
                },
            );
            tick.cx.snapshot = SnapshotView::new(Some(after), tick.cx.evidence());
            tick.frame.snapshot = Some(after);
            script.tick(tick).unwrap();
            assert!(script.settling);
            script.tick(tick).unwrap();
            assert!(!script.settling);
            assert_eq!(script.cursor, 1);
            assert!(script.step.is_none());
        });

        let scene_snapshot = |origin| {
            let mut snapshot = GameSnapshot::new();
            snapshot.seed_ingame(2);
            snapshot.seed_quest_statuses(
                vec![QuestStatusView {
                    name: "Cook's Assistant".into(),
                    component_id: 42,
                    colour: 0xf80000,
                }],
                true,
            );
            snapshot.seed_varps(vec![VarpView {
                index: crate::combat::OPTION_NODEF,
                value: 1,
            }]);
            snapshot.seed_local_player(super::super::families::tests::local_player(
                api::WorldTile {
                    x: 3200,
                    z: 3280,
                    level: 0,
                },
            ));
            snapshot.seed_world(api::snapshot::WorldStateView {
                map_base_x: origin,
                map_base_z: 3280,
                ..Default::default()
            });
            snapshot
        };
        let scene_before = scene_snapshot(3200);
        let entered = scene_snapshot(3208);
        with_tick_snapshots(&after, &entered, &mut ledger, 2, |tick, entered| {
            tick.cx.budget.observe_frame(1, Some(&scene_before));
            tick.cx.budget.observe_frame(2, Some(entered));
            assert!(tick.cx.entered_scene());
            tick.cx.snapshot = SnapshotView::new(Some(entered), tick.cx.evidence());
            tick.frame.snapshot = Some(entered);

            script.tick(tick).unwrap();
            assert!(
                !script
                    .begun
                    .iter()
                    .any(|step| step.as_ref() == "retaliate-on"),
                "fresh step selection must wait through scene entry"
            );
            assert!(
                script.step.is_none(),
                "fresh step selection must wait through scene entry"
            );
            assert_eq!(script.cursor, 1);
            assert_eq!(tick.cx.ledger.as_ref().unwrap().outbox.len(), 1);
        });
    }

    #[test]
    fn budget_exhausted_root_begin_retries_without_failing_the_step() {
        if !api::test_support::game_data_available() {
            return;
        }
        use super::super::families::tests::{with_tick, with_tick_snapshots};
        use super::super::path::{PredicateDocument, SequenceOrder, StepDocument};
        use api::snapshot::{GameSnapshot, QuestStatusView, SnapshotView, VarpView};

        let mut document = super::super::compile::decode_cook().unwrap();
        let sequence = &mut document.roles[0].sequences[0];
        sequence.order = SequenceOrder::Ordered;
        sequence.steps = [("retaliate-off", false), ("retaliate-on", true)]
            .into_iter()
            .map(|(id, retaliate)| StepDocument {
                id: FactKey::new(id),
                kind: "setting".into(),
                version: 1,
                args: serde_json::json!({ "retaliate": retaliate }),
                comment: None,
                advances: Some(false),
                max_attempts: None,
                stop: None,
                hazard: None,
                skip_if: PredicateDocument::Any(vec![]),
                settle: PredicateDocument::All(vec![]),
            })
            .collect();
        let (mut script, mut before) = status_fixture(document);
        before.seed_varps(vec![VarpView {
            index: crate::combat::OPTION_NODEF,
            value: 0,
        }]);
        let mut after = GameSnapshot::new();
        after.seed_ingame(2);
        after.seed_quest_statuses(
            vec![QuestStatusView {
                name: "Cook's Assistant".into(),
                component_id: 42,
                colour: 0xf80000,
            }],
            true,
        );
        after.seed_varps(vec![VarpView {
            index: crate::combat::OPTION_NODEF,
            value: 1,
        }]);

        let mut ledger = None;
        with_tick_snapshots(&before, &after, &mut ledger, 1, |tick, after| {
            script.tick(tick).unwrap();
            let authority = tick.cx.ledger.as_ref().unwrap().outbox[0].authority();
            let evidence = tick.cx.evidence();
            tick.cx.ledger.as_mut().unwrap().complete_interaction(
                &authority,
                crate::native::InteractionReceipt {
                    request_id: authority.request_id().get(),
                    evidence,
                    accepted: true,
                    chat_since: 0,
                },
            );
            tick.cx.snapshot = SnapshotView::new(Some(after), tick.cx.evidence());
            tick.frame.snapshot = Some(after);
            script.tick(tick).unwrap();
            assert!(script.settling);
            script.tick(tick).unwrap();
            assert!(!script.settling);
            assert_eq!(script.cursor, 1);
            assert!(script.step.is_none());
        });

        with_tick(&after, &mut ledger, 2, |tick| {
            for _ in 0..32 {
                assert!(tick.cx.budget.transition());
            }
            assert!(!tick.cx.budget.transition());
            script.tick(tick).unwrap();

            assert!(
                script
                    .begun
                    .iter()
                    .any(|step| step.as_ref() == "retaliate-on"),
                "the second root step must reach its begin attempt"
            );
            assert!(script.step.is_none());
            assert_eq!(script.cursor, 1);
            assert_eq!(script.attempts, 0);
            assert_eq!(script.fail_streak, 0);
            assert!(script.last_error.is_none());
        });

        with_tick(&after, &mut ledger, 3, |tick| {
            script.tick(tick).unwrap();
            assert!(
                script.step.is_some(),
                "a fresh transition budget must retry the same root step"
            );
            assert_eq!(script.attempts, 0);
        });
    }

    #[test]
    fn stage_cross_follows_observed_stage_and_never_the_previous_cursor() {
        if !api::test_support::game_data_available() {
            return;
        }
        let (mut script, snapshot) = status_fixture(super::super::compile::decode_cook().unwrap());
        let path = Arc::get_mut(&mut script.path).unwrap();
        path.sequences[0].cross = Arc::from([Arc::from("first-zone")]);
        path.sequences[1].cross = Arc::from([Arc::from("second-zone")]);
        let stages = [
            path.sequences[0].stage.clone(),
            path.sequences[1].stage.clone(),
        ];
        let mut ledger = None;
        let mut output = StatusCapture::default();
        super::super::families::tests::with_tick_output(
            &snapshot,
            &mut ledger,
            1,
            &mut output,
            |tick| {
                script.stage = Some(stages[0].clone());
                script.scope_stage_cross(tick);
                assert_eq!(
                    tick.cx.stage_cross.as_deref().unwrap(),
                    &[Arc::<str>::from("first-zone")]
                );
                script.stage = Some(stages[1].clone());
                script.scope_stage_cross(tick);
                assert_eq!(
                    tick.cx.stage_cross.as_deref().unwrap(),
                    &[Arc::<str>::from("second-zone")]
                );
                script.stage = None;
                script.scope_stage_cross(tick);
                assert!(tick.cx.stage_cross.as_deref().unwrap().is_empty());
            },
        );
    }

    pub(super) struct SurvivalTestCounts {
        pub(super) began: std::sync::atomic::AtomicUsize,
        pub(super) cancelled: std::sync::atomic::AtomicUsize,
    }

    pub(super) fn survival_condition(
        poison: api::condition::PoisonState,
        hp: u8,
    ) -> api::condition::BotCondition {
        use api::condition::{BotCondition, CureDoses, FoodSummary, SessionMark};
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

    /// A Quester on the Cook path whose first step is a never-settling `kind`
    /// step, with HP 5 of 99, one held antipoison dose and a complete frame.
    pub(super) fn survival_step_fixture(
        kind: &str,
    ) -> (
        Quester,
        api::snapshot::GameSnapshot,
        Arc<SurvivalTestCounts>,
    ) {
        use crate::quester::compile::PredicatePlan;
        use api::selected::Truth;
        use api::snapshot::{
            HitmarkView, HitmarksView, ItemActionFamily, ItemContainer, ItemView, StatView,
            WorldStateView,
        };
        use std::sync::atomic::{AtomicUsize, Ordering};

        struct NeverSkip;
        impl PredicatePlan for NeverSkip {
            fn evaluate(&self, _: &PredicateContext<'_, '_>) -> Truth {
                Truth::False
            }
        }

        struct PendingPlan(Arc<SurvivalTestCounts>);
        impl super::super::compile::StepPlan for PendingPlan {
            fn begin(&self, _: &mut StepContext<'_, '_>) -> Result<Box<dyn StepRun>, ActionError> {
                self.0.began.fetch_add(1, Ordering::Relaxed);
                Ok(Box::new(PendingRun(Arc::clone(&self.0))))
            }
        }

        struct PendingRun(Arc<SurvivalTestCounts>);
        impl StepRun for PendingRun {
            fn poll(
                &mut self,
                _: &mut StepContext<'_, '_>,
            ) -> Poll<Result<StepOutcome, ActionError>> {
                Poll::Pending
            }

            fn cancel(&mut self, _: &mut crate::native::NativeActions) {
                self.0.cancelled.fetch_add(1, Ordering::Relaxed);
            }
        }

        let (mut script, mut snapshot) =
            status_fixture(super::super::compile::decode_cook().unwrap());
        let stage = script.path.colour_not_started.clone();
        script.seq_index =
            sequence_for_stage(&script.path, stage.0.as_ref()).expect("not-started sequence");
        script.stage = Some(stage);
        script.cursor = 0;
        script.needs_read = false;
        let sequence_index = script.seq_index;
        let path = Arc::get_mut(&mut script.path).expect("unshared compiled path");
        let sequence = &mut path.sequences[sequence_index];
        let step = sequence
            .steps
            .first_mut()
            .expect("the Cook path has a step");
        step.kind = Arc::from(kind);
        let counts = Arc::new(SurvivalTestCounts {
            began: AtomicUsize::new(0),
            cancelled: AtomicUsize::new(0),
        });
        step.plan = Arc::new(PendingPlan(Arc::clone(&counts)));
        step.skip_if = Arc::new(NeverSkip);
        step.settle = Arc::new(NeverSkip);
        step.skip_if_summary = Arc::from("test step");

        let at = api::WorldTile {
            x: 3200,
            z: 3200,
            level: 0,
        };
        snapshot.seed_tick(20);
        snapshot.seed_ingame(2);
        snapshot.seed_world(WorldStateView {
            map_base_x: 3200,
            map_base_z: 3200,
            members: true,
            ..WorldStateView::default()
        });
        snapshot.seed_local_player(super::super::families::tests::local_player(at));
        snapshot.seed_stats(vec![StatView {
            index: 3,
            name: "hitpoints".into(),
            base: 99,
            effective: 5,
            xp: 0,
            used: true,
        }]);
        snapshot.seed_equipment(vec![]);
        snapshot.seed_npcs(vec![]);
        snapshot.seed_players(Vec::new());
        snapshot.seed_projectiles(Vec::new());
        snapshot.seed_hitmarks(HitmarksView {
            marks: [HitmarkView {
                value: 0,
                kind: 0,
                cycle: 0,
            }; 4],
            loop_cycle: 0,
        });

        let dose_id = crate::combat::guard::shared_tables()
            .unwrap()
            .potion(crate::combat::tables::PotionKind::Antipoison)
            .unwrap()
            .doses[0]
            .unwrap()
            .id;
        let dose = ItemView {
            def: super::super::families::tests::def(dose_id, "Antipoison"),
            container: ItemContainer::Inventory,
            action_family: ItemActionFamily::Held,
            slot: 0,
            count: 1,
            actions: Vec::new(),
            component_id: 0,
        };
        snapshot.seed_inventory(vec![dose], 28);
        (script, snapshot, counts)
    }

    fn quester_survival_cancels_step_and_restarts(kind: &str) {
        use crate::survival::CurePoison;
        use api::condition::PoisonState;
        use std::sync::atomic::Ordering;

        let condition = survival_condition;
        let (mut script, mut snapshot, counts) = survival_step_fixture(kind);
        let mut ledger = None;
        super::super::families::tests::with_tick(&snapshot, &mut ledger, 1, |tick| {
            tick.cx.condition = Some(condition(PoisonState::Clear, 60));
            tick.cx.cure_poison = CurePoison::WhenNeeded;
            script.tick(tick).unwrap();
        });
        assert_eq!(counts.began.load(Ordering::Relaxed), 1);
        assert!(script.step.is_some());

        if kind == "talk" {
            snapshot.seed_main_modal(42, Vec::new());
        }
        super::super::families::tests::with_tick(&snapshot, &mut ledger, 2, |tick| {
            tick.cx.condition = Some(condition(
                PoisonState::Onset {
                    since: 20,
                    q: Some(11),
                    hit_since_line: false,
                },
                5,
            ));
            tick.cx.poison_unknown_pulse = 11;
            tick.cx.cure_poison = CurePoison::WhenNeeded;
            script.tick(tick).unwrap();
        });
        if matches!(kind, "walk" | "combat") {
            assert_eq!(counts.cancelled.load(Ordering::Relaxed), 0);
            assert!(script.step.is_some());
            assert!(script.survival_action.is_none());
            assert!(script.survival_deferred);
            // The walk guard or the combat machine owns the drink; the runner
            // reports the need, not a drink of its own (design A6.6).
            assert_eq!(
                script.survival.report().decision,
                crate::survival::Decision::Needed
            );
            return;
        }
        if kind == "talk" {
            assert_eq!(counts.cancelled.load(Ordering::Relaxed), 1);
            assert!(script.step.is_none());
            assert!(matches!(
                ledger
                    .as_ref()
                    .unwrap()
                    .outbox
                    .last()
                    .map(|action| &action.effect),
                Some(crate::native::HostEffect::Interaction(
                    crate::interact::InteractReq::CloseModal
                ))
            ));
            snapshot.seed_main_modal(-1, Vec::new());
            snapshot.seed_tick(21);
            super::super::families::tests::with_tick(&snapshot, &mut ledger, 3, |tick| {
                tick.cx.condition = Some(condition(
                    PoisonState::Onset {
                        since: 20,
                        q: Some(11),
                        hit_since_line: false,
                    },
                    5,
                ));
                tick.cx.poison_unknown_pulse = 11;
                tick.cx.cure_poison = CurePoison::WhenNeeded;
                let evidence = tick.cx.evidence();
                let authority = tick
                    .cx
                    .ledger
                    .as_ref()
                    .unwrap()
                    .outbox
                    .last()
                    .unwrap()
                    .authority();
                tick.cx.ledger.as_mut().unwrap().complete_interaction(
                    &authority,
                    crate::native::InteractionReceipt {
                        request_id: authority.request_id().get(),
                        evidence,
                        accepted: true,
                        chat_since: 0,
                    },
                );
                script.tick(tick).unwrap();
            });
            // The close settled in tick 3 and the drink takes the same tick
            // (design A6.5: tick 0 close, tick 1 drink).
            assert!(matches!(
                ledger.as_ref().unwrap().outbox.last().map(|action| &action.effect),
                Some(crate::native::HostEffect::Interaction(
                    crate::interact::InteractReq::Held { action, .. }
                )) if action == "Drink"
            ));
            return;
        }

        assert_eq!(counts.cancelled.load(Ordering::Relaxed), 1);
        assert!(script.step.is_none());
        assert_eq!(
            script.survival.report().decision,
            crate::survival::Decision::Drinking
        );
        assert!(matches!(
            ledger.as_ref().unwrap().outbox.last().map(|action| &action.effect),
            Some(crate::native::HostEffect::Interaction(
                crate::interact::InteractReq::Held { action, .. }
            )) if action == "Drink"
        ));

        snapshot.seed_tick(21);
        snapshot.seed_inventory(vec![], 28);
        super::super::families::tests::with_tick(&snapshot, &mut ledger, 3, |tick| {
            tick.cx.condition = Some(condition(PoisonState::Immune { until_lower: 80 }, 5));
            script.tick(tick).unwrap();
        });
        assert!(script.step.is_none());
        assert!(script.survival_action.is_none());

        // The drink sent at observed tick 2 delays the player through the next
        // two server ticks (`consume_effects.rs2:124-126`,
        // `PlayerOps.ts:376-378`), so the step waits for the drink lock
        // instead of sending clicks the engine drops.
        super::super::families::tests::with_tick(&snapshot, &mut ledger, 4, |tick| {
            tick.cx.condition = Some(condition(PoisonState::Immune { until_lower: 80 }, 5));
            script.tick(tick).unwrap();
        });
        assert_eq!(counts.began.load(Ordering::Relaxed), 1);
        assert!(script.step.is_none());

        super::super::families::tests::with_tick(&snapshot, &mut ledger, 5, |tick| {
            tick.cx.condition = Some(condition(PoisonState::Immune { until_lower: 80 }, 5));
            script.tick(tick).unwrap();
        });
        assert_eq!(counts.began.load(Ordering::Relaxed), 2);
        assert!(
            script.step.is_some(),
            "the step resumes after the cure settles"
        );
        assert!(!script.parked);
    }

    #[test]
    fn quester_polls_survival_inside_gather_step() {
        if !api::test_support::game_data_available() {
            return;
        }
        quester_survival_cancels_step_and_restarts("gather");
    }

    /// The survival cancel leaves the state `Interrupt::Resume` leaves (design
    /// A6.5 Quester item 3): no step, a fresh read owed, no settle wait, and
    /// no retained progress, so `advance` restarts the step.
    #[test]
    fn quester_cancel_through_resume_path_restarts_step() {
        if !api::test_support::game_data_available() {
            return;
        }
        use crate::survival::CurePoison;
        use api::condition::PoisonState;
        use std::sync::atomic::Ordering;

        let (mut script, snapshot, counts) = survival_step_fixture("gather");
        let mut ledger = None;
        super::super::families::tests::with_tick(&snapshot, &mut ledger, 1, |tick| {
            tick.cx.condition = Some(survival_condition(PoisonState::Clear, 60));
            tick.cx.cure_poison = CurePoison::WhenNeeded;
            script.tick(tick).unwrap();
        });
        assert!(script.step.is_some());
        script.needs_read = false;
        script.settling = true;
        super::super::families::tests::with_tick(&snapshot, &mut ledger, 2, |tick| {
            tick.cx.condition = Some(survival_condition(
                PoisonState::Onset {
                    since: 20,
                    q: Some(11),
                    hit_since_line: false,
                },
                5,
            ));
            tick.cx.poison_unknown_pulse = 11;
            tick.cx.cure_poison = CurePoison::WhenNeeded;
            script.tick(tick).unwrap();
        });
        assert_eq!(counts.cancelled.load(Ordering::Relaxed), 1);
        assert!(script.step.is_none());
        assert!(script.needs_read);
        assert!(!script.settling);
        assert!(script.progress.is_none());
        assert!(script.survival_action.is_some());
    }

    /// A park can outlast a settle wait's deadline. Parking with no step ends
    /// the settle wait and owes a fresh read, and the un-park owes one too, as
    /// `Interrupt::Resume` does (design A6.5 item 2; Argon review finding 10).
    /// Before, the deadline expired during the park and the first tick after
    /// the un-park failed the step with "step settle timeout".
    #[test]
    fn quester_park_ends_a_settle_wait_and_unpark_rereads() {
        if !api::test_support::game_data_available() {
            return;
        }
        use crate::survival::CurePoison;
        use api::condition::PoisonState;

        let (mut script, mut snapshot, counts) = survival_step_fixture("gather");
        snapshot.seed_inventory(vec![], 28);
        let mut ledger = None;
        super::super::families::tests::with_tick(&snapshot, &mut ledger, 1, |tick| {
            tick.cx.condition = Some(survival_condition(PoisonState::Clear, 60));
            tick.cx.cure_poison = CurePoison::WhenNeeded;
            script.tick(tick).unwrap();
        });
        // The step completed and the Quester waits for its outcome.
        script.step = None;
        script.settling = true;
        script.settle_deadline = Duration::from_secs(1);
        script.needs_read = false;
        super::super::families::tests::with_tick(&snapshot, &mut ledger, 2, |tick| {
            tick.cx.condition = Some(survival_condition(
                PoisonState::Onset {
                    since: 20,
                    q: Some(11),
                    hit_since_line: false,
                },
                5,
            ));
            tick.cx.poison_unknown_pulse = 11;
            tick.cx.cure_poison = CurePoison::WhenNeeded;
            script.tick(tick).unwrap();
        });
        assert!(script.survival.parked().is_some());
        assert!(!script.settling, "the park ends the settle wait");
        assert_eq!(script.settle_deadline, Duration::ZERO);
        assert!(script.needs_read);

        script.needs_read = false;
        super::super::families::tests::with_tick(&snapshot, &mut ledger, 3, |tick| {
            tick.cx.condition = Some(survival_condition(
                PoisonState::Immune { until_lower: 80 },
                60,
            ));
            tick.cx.cure_poison = CurePoison::WhenNeeded;
            script.tick(tick).unwrap();
        });
        assert!(script.survival.parked().is_none());
        assert!(!script.parked);
        assert!(script.last_error.is_none());
        // The fresh read is taken at once in this fixture, and `advance`
        // begins the step again in the same tick instead of waiting out the
        // old settle deadline.
        assert_eq!(counts.began.load(std::sync::atomic::Ordering::Relaxed), 2);
        assert!(script.step.is_some());
    }

    #[test]
    fn quester_closes_dialogue_page_before_drink() {
        if !api::test_support::game_data_available() {
            return;
        }
        quester_survival_cancels_step_and_restarts("talk");
    }

    /// An enter-amount prompt (a Make-X count after `if_close`) has no modal
    /// root, yet the client refuses every held click while it is open and the
    /// engine keeps the suspended script's `protect` (`Player.ts:2173-2178`).
    /// Survival closes it first (Argon review finding 4), then drinks in the
    /// tick the close settles.
    #[test]
    fn quester_closes_count_dialog_before_drink() {
        if !api::test_support::game_data_available() {
            return;
        }
        use crate::survival::CurePoison;
        use api::condition::PoisonState;

        let onset = PoisonState::Onset {
            since: 20,
            q: Some(11),
            hit_since_line: false,
        };
        let (mut script, mut snapshot, _) = survival_step_fixture("gather");
        let mut ledger = None;
        super::super::families::tests::with_tick(&snapshot, &mut ledger, 1, |tick| {
            tick.cx.condition = Some(survival_condition(PoisonState::Clear, 60));
            tick.cx.cure_poison = CurePoison::WhenNeeded;
            script.tick(tick).unwrap();
        });
        snapshot.seed_count_dialog(true);
        super::super::families::tests::with_tick(&snapshot, &mut ledger, 2, |tick| {
            tick.cx.condition = Some(survival_condition(onset, 5));
            tick.cx.poison_unknown_pulse = 11;
            tick.cx.cure_poison = CurePoison::WhenNeeded;
            script.tick(tick).unwrap();
        });
        assert!(matches!(
            ledger
                .as_ref()
                .unwrap()
                .outbox
                .last()
                .map(|action| &action.effect),
            Some(crate::native::HostEffect::Interaction(
                crate::interact::InteractReq::CloseModal
            ))
        ));

        snapshot.seed_count_dialog(false);
        super::super::families::tests::with_tick(&snapshot, &mut ledger, 3, |tick| {
            tick.cx.condition = Some(survival_condition(onset, 5));
            tick.cx.poison_unknown_pulse = 11;
            tick.cx.cure_poison = CurePoison::WhenNeeded;
            script.tick(tick).unwrap();
        });
        assert!(matches!(
            ledger.as_ref().unwrap().outbox.last().map(|action| &action.effect),
            Some(crate::native::HostEffect::Interaction(
                crate::interact::InteractReq::Held { action, .. }
            )) if action == "Drink"
        ));
        assert!(!script.parked);
    }

    #[test]
    fn quester_never_cancels_walk_or_combat_step() {
        if !api::test_support::game_data_available() {
            return;
        }
        quester_survival_cancels_step_and_restarts("walk");
        quester_survival_cancels_step_and_restarts("combat");
    }

    /// The engine drops a held op while the player is delayed
    /// (`OpHeldHandler.ts:16-19`), as in the tick after a mining success's
    /// `p_delay(0)` (`mining.rs2:139`). The drink then shows no dose drop, and
    /// the Quester must click again in the tick the three-tick drink lock
    /// ends, not one tick later and not after the generic eight-tick bound.
    #[test]
    fn quester_resends_a_dropped_drink_after_the_drink_lock() {
        if !api::test_support::game_data_available() {
            return;
        }
        use crate::survival::CurePoison;
        use api::condition::PoisonState;

        let onset = PoisonState::Onset {
            since: 20,
            q: Some(11),
            hit_since_line: false,
        };
        // The outbox keeps only the latest interaction, so record each drink
        // request id once.
        let mut drink_requests: Vec<u64> = Vec::new();
        let mut note_drink = |ledger: &Option<Box<crate::native::ledger::Ledger>>| {
            let Some(action) = ledger.as_ref().and_then(|ledger| ledger.outbox.last()) else {
                return drink_requests.len();
            };
            let is_drink = matches!(
                &action.effect,
                crate::native::HostEffect::Interaction(
                    crate::interact::InteractReq::Held { action, .. }
                ) if action == "Drink"
            );
            let request = action.authority().request_id().get();
            if is_drink && !drink_requests.contains(&request) {
                drink_requests.push(request);
            }
            drink_requests.len()
        };
        let (mut script, snapshot, _) = survival_step_fixture("gather");
        let mut ledger = None;
        super::super::families::tests::with_tick(&snapshot, &mut ledger, 1, |tick| {
            tick.cx.condition = Some(survival_condition(PoisonState::Clear, 60));
            tick.cx.cure_poison = CurePoison::WhenNeeded;
            script.tick(tick).unwrap();
        });
        super::super::families::tests::with_tick(&snapshot, &mut ledger, 2, |tick| {
            tick.cx.condition = Some(survival_condition(onset, 5));
            tick.cx.poison_unknown_pulse = 11;
            tick.cx.cure_poison = CurePoison::WhenNeeded;
            script.tick(tick).unwrap();
        });
        assert_eq!(
            note_drink(&ledger),
            1,
            "the drink is the first survival click"
        );

        // The client sent it and the host accepted it; the delayed engine
        // dropped it, so the dose stays held.
        let mut drinks_by_tick = Vec::new();
        for observed in 3..=6 {
            super::super::families::tests::with_tick(&snapshot, &mut ledger, observed, |tick| {
                tick.cx.condition = Some(survival_condition(onset, 5));
                tick.cx.poison_unknown_pulse = 11;
                tick.cx.cure_poison = CurePoison::WhenNeeded;
                if observed == 3 {
                    let evidence = tick.cx.evidence();
                    let ledger = tick.cx.ledger.as_mut().unwrap();
                    let authority = ledger.outbox.last().unwrap().authority();
                    ledger.complete_interaction(
                        &authority,
                        crate::native::InteractionReceipt {
                            request_id: authority.request_id().get(),
                            evidence,
                            accepted: true,
                            chat_since: 0,
                        },
                    );
                }
                script.tick(tick).unwrap();
            });
            drinks_by_tick.push((observed, note_drink(&ledger)));
        }
        assert_eq!(
            drinks_by_tick,
            vec![(3, 1), (4, 1), (5, 2), (6, 2)],
            "a drink with no dose drop is clicked again in the tick its three-tick lock ends"
        );
    }

    /// The drink's dose drop arrives one tick later than usual (tick 4). The
    /// drink's `p_delay(1)` (`consume_effects.rs2:124-126`) starts in the
    /// server tick that ran it (`PlayerOps.ts:376-379`), so the step restarts
    /// when that lock ends (tick 6), not when a lock counted from the send
    /// would end (tick 5).
    #[test]
    fn quester_late_drink_effect_moves_the_drink_lock() {
        if !api::test_support::game_data_available() {
            return;
        }
        use crate::survival::CurePoison;
        use api::condition::PoisonState;
        use std::sync::atomic::Ordering;

        let onset = PoisonState::Onset {
            since: 20,
            q: Some(11),
            hit_since_line: false,
        };
        let cured = PoisonState::Immune { until_lower: 80 };
        let (mut script, mut snapshot, counts) = survival_step_fixture("gather");
        let mut ledger = None;
        let mut tick_at = |script: &mut Quester,
                           snapshot: &api::snapshot::GameSnapshot,
                           observed: u64,
                           poison: PoisonState,
                           hp: u8| {
            super::super::families::tests::with_tick(snapshot, &mut ledger, observed, |tick| {
                tick.cx.condition = Some(survival_condition(poison, hp));
                tick.cx.poison_unknown_pulse = 11;
                tick.cx.cure_poison = CurePoison::WhenNeeded;
                script.tick(tick).unwrap();
            });
        };
        tick_at(&mut script, &snapshot, 1, PoisonState::Clear, 60);
        assert_eq!(counts.began.load(Ordering::Relaxed), 1);
        tick_at(&mut script, &snapshot, 2, onset, 5);
        assert!(script.step.is_none(), "the drink cancels the step");
        assert!(script.survival_action.is_some());
        tick_at(&mut script, &snapshot, 3, onset, 5);
        assert!(script.survival_action.is_some(), "tick 3: no dose drop yet");
        snapshot.seed_inventory(vec![], 28);
        for observed in 4..=5 {
            tick_at(&mut script, &snapshot, observed, cured, 5);
            assert!(script.survival_action.is_none(), "tick {observed}");
            assert_eq!(
                counts.began.load(Ordering::Relaxed),
                1,
                "no step inside the lock of a drink run in tick 4: tick {observed}"
            );
        }
        tick_at(&mut script, &snapshot, 6, cured, 5);
        assert_eq!(
            counts.began.load(Ordering::Relaxed),
            2,
            "the step restarts when the late drink's lock ends"
        );
        assert!(script.step.is_some());
    }

    /// A bite settles (count drop) in a tick where no lock holds and survival
    /// wants nothing more: the step restarts in that same tick. The Quester
    /// bites only on a threat, here a weak poison whose next fire (3) reaches
    /// its eating line (`1 + 3`). Ordinary food sets no input lock (no
    /// `message_delay` row for the lobster in `consume_messages.dbrow`), and
    /// the next-food clock holds only bites.
    #[test]
    fn quester_resumes_work_in_the_tick_its_bite_settles() {
        if !api::test_support::game_data_available() {
            return;
        }
        use crate::survival::CurePoison;
        use api::condition::PoisonState;
        use api::snapshot::{ItemActionFamily, ItemContainer, ItemView, StatView};
        use std::sync::atomic::Ordering;

        let (mut script, mut snapshot, counts) = survival_step_fixture("gather");
        let set_hp = |snapshot: &mut api::snapshot::GameSnapshot, effective| {
            snapshot.seed_stats(vec![StatView {
                index: 3,
                name: "hitpoints".into(),
                base: 99,
                effective,
                xp: 0,
                used: true,
            }]);
        };
        let lobster_id = api::game_data::for_revision(api::selected::ClientRevision::R289)
            .unwrap()
            .item_by_alias("lobster")
            .unwrap()
            .id;
        snapshot.seed_inventory(
            vec![ItemView {
                def: super::super::families::tests::def(lobster_id, "Lobster"),
                container: ItemContainer::Inventory,
                action_family: ItemActionFamily::Held,
                slot: 0,
                count: 1,
                actions: vec![Some("Eat".into())],
                component_id: 0,
            }],
            28,
        );
        set_hp(&mut snapshot, 60);
        let mut ledger = None;
        let weak = PoisonState::Poisoned {
            per_tick: 3,
            last_mark: 20,
            hit_since_mark: false,
        };
        let mut tick_at = |script: &mut Quester,
                           snapshot: &api::snapshot::GameSnapshot,
                           observed: u64,
                           poison: PoisonState,
                           hp: u8| {
            super::super::families::tests::with_tick(snapshot, &mut ledger, observed, |tick| {
                tick.cx.condition = Some(survival_condition(poison, hp));
                tick.cx.cure_poison = CurePoison::WhenNeeded;
                script.tick(tick).unwrap();
            });
        };
        tick_at(&mut script, &snapshot, 1, PoisonState::Clear, 60);
        assert_eq!(counts.began.load(Ordering::Relaxed), 1);
        set_hp(&mut snapshot, 4);
        tick_at(&mut script, &snapshot, 2, weak, 4);
        assert!(script.step.is_none(), "the bite cancels the step");
        assert!(script.survival_action.is_some());

        snapshot.seed_inventory(vec![], 28);
        set_hp(&mut snapshot, 60);
        tick_at(&mut script, &snapshot, 3, weak, 60);
        assert!(script.survival_action.is_none());
        assert_eq!(
            counts.began.load(Ordering::Relaxed),
            2,
            "the step restarts in the tick the bite settles"
        );
        assert!(script.step.is_some());
    }

    /// Natural regeneration restores one Hitpoint on its own timer
    /// (`stat.rs2:58-60`). A rise in the frame after a bite was sent, with the
    /// food still held, is not the bite: the survival op keeps waiting for the
    /// count drop, so the next-food clock is not anchored on the rise.
    #[test]
    fn quester_natural_regeneration_does_not_settle_a_bite() {
        if !api::test_support::game_data_available() {
            return;
        }
        use crate::survival::CurePoison;
        use api::condition::PoisonState;
        use api::snapshot::{ItemActionFamily, ItemContainer, ItemView, StatView};

        let (mut script, mut snapshot, _) = survival_step_fixture("gather");
        let set_hp = |snapshot: &mut api::snapshot::GameSnapshot, effective| {
            snapshot.seed_stats(vec![StatView {
                index: 3,
                name: "hitpoints".into(),
                base: 99,
                effective,
                xp: 0,
                used: true,
            }]);
        };
        let lobster_id = api::game_data::for_revision(api::selected::ClientRevision::R289)
            .unwrap()
            .item_by_alias("lobster")
            .unwrap()
            .id;
        let lobsters = |count| {
            vec![ItemView {
                def: super::super::families::tests::def(lobster_id, "Lobster"),
                container: ItemContainer::Inventory,
                action_family: ItemActionFamily::Held,
                slot: 0,
                count,
                actions: vec![Some("Eat".into())],
                component_id: 0,
            }]
        };
        snapshot.seed_inventory(lobsters(2), 28);
        set_hp(&mut snapshot, 60);
        let mut ledger = None;
        let weak = PoisonState::Poisoned {
            per_tick: 3,
            last_mark: 20,
            hit_since_mark: false,
        };
        let mut tick_at = |script: &mut Quester,
                           snapshot: &api::snapshot::GameSnapshot,
                           observed: u64,
                           poison: PoisonState,
                           hp: u8| {
            super::super::families::tests::with_tick(snapshot, &mut ledger, observed, |tick| {
                tick.cx.condition = Some(survival_condition(poison, hp));
                tick.cx.cure_poison = CurePoison::WhenNeeded;
                script.tick(tick).unwrap();
            });
        };
        tick_at(&mut script, &snapshot, 1, PoisonState::Clear, 60);
        set_hp(&mut snapshot, 4);
        tick_at(&mut script, &snapshot, 2, weak, 4);
        assert!(script.survival_action.is_some(), "the bite is sent");

        set_hp(&mut snapshot, 5);
        tick_at(&mut script, &snapshot, 3, weak, 5);
        assert!(
            script.survival_action.is_some(),
            "a hitpoints rise with the food still held is not the bite"
        );

        snapshot.seed_inventory(lobsters(1), 28);
        tick_at(&mut script, &snapshot, 4, weak, 5);
        assert!(
            script.survival_action.is_none(),
            "the count drop settles the bite"
        );
    }

    #[test]
    fn quester_survival_trace_pairs_a_retried_bite() {
        use api::condition::PoisonState;
        use api::snapshot::{ItemActionFamily, ItemContainer, ItemView, StatView};

        const CHILD: &str = "QUESTER_SURVIVAL_TRACE_TEST_CHILD";
        const TEST: &str = "quester::runner::tests::quester_survival_trace_pairs_a_retried_bite";
        if !api::test_support::game_data_available() {
            return;
        }
        let item_id = api::game_data::for_revision(api::selected::ClientRevision::R289)
            .unwrap()
            .item_by_alias("lobster")
            .unwrap()
            .id;
        if std::env::var_os(CHILD).is_none() {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", TEST, "--nocapture"])
                .env(CHILD, "1")
                .env("BOT_DEBUG", "1")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "trace child failed:\n{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr),
            );
            let stderr = String::from_utf8_lossy(&output.stderr);
            let lines: Vec<_> = stderr
                .lines()
                .filter_map(|line| {
                    line.split_once("quester survival op ")
                        .map(|(_, event)| event)
                })
                .collect();
            assert_eq!(
                lines,
                [
                    format!("event=begin tick=2 kind=Eat {{ item_id: {item_id}, before_count: 2 }} applied=pending deadline=10 player_info=0"),
                    format!("event=end tick=10 kind=Eat {{ item_id: {item_id}, before_count: 2 }} applied=false deadline=10 player_info=0"),
                    format!("event=begin tick=10 kind=Eat {{ item_id: {item_id}, before_count: 2 }} applied=pending deadline=18 player_info=0"),
                    format!("event=end tick=11 kind=Eat {{ item_id: {item_id}, before_count: 2 }} applied=true deadline=18 player_info=0"),
                ],
                "the first bite must end unapplied before its same-tick retry begins"
            );
            return;
        }

        let (mut script, mut snapshot, _) = survival_step_fixture("gather");
        let food = |count| {
            vec![ItemView {
                def: super::super::families::tests::def(item_id, "Lobster"),
                container: ItemContainer::Inventory,
                action_family: ItemActionFamily::Held,
                slot: 0,
                count,
                actions: vec![Some("Eat".into())],
                component_id: 0,
            }]
        };
        let set_hp = |snapshot: &mut api::snapshot::GameSnapshot, effective| {
            snapshot.seed_stats(vec![StatView {
                index: 3,
                name: "hitpoints".into(),
                base: 99,
                effective,
                xp: 0,
                used: true,
            }]);
        };
        snapshot.seed_inventory(food(2), 28);
        set_hp(&mut snapshot, 60);
        let mut ledger = None;
        super::super::families::tests::with_tick(&snapshot, &mut ledger, 1, |tick| {
            tick.cx.condition = Some(survival_condition(PoisonState::Clear, 60));
            script.tick(tick).unwrap();
        });
        let poison = PoisonState::Poisoned {
            per_tick: 3,
            last_mark: 20,
            hit_since_mark: false,
        };
        set_hp(&mut snapshot, 4);
        for observed in 2..=10 {
            super::super::families::tests::with_tick(&snapshot, &mut ledger, observed, |tick| {
                tick.cx.condition = Some(survival_condition(poison, 4));
                script.tick(tick).unwrap();
            });
            assert!(script.survival_action.is_some());
        }
        snapshot.seed_inventory(food(1), 28);
        set_hp(&mut snapshot, 16);
        super::super::families::tests::with_tick(&snapshot, &mut ledger, 11, |tick| {
            tick.cx.condition = Some(survival_condition(poison, 16));
            script.tick(tick).unwrap();
        });
        assert!(script.survival_action.is_none());
    }

    #[test]
    fn quester_drinks_while_clear_prayers_pending() {
        if !api::test_support::game_data_available() {
            return;
        }
        use crate::combat::{ClearPrayers, ClearPrayersArgs, RaisedPrayers};
        use crate::survival::CurePoison;
        use api::condition::{BotCondition, CureDoses, FoodSummary, PoisonState, SessionMark};
        use api::snapshot::{
            HitmarkView, HitmarksView, ItemActionFamily, ItemContainer, ItemView, StatView,
            VarpView, WorldStateView,
        };

        let (mut script, mut snapshot) =
            status_fixture(super::super::compile::decode_cook().unwrap());
        let at = api::WorldTile {
            x: 3200,
            z: 3200,
            level: 0,
        };
        snapshot.seed_tick(20);
        snapshot.seed_ingame(2);
        snapshot.seed_world(WorldStateView {
            map_base_x: 3200,
            map_base_z: 3200,
            members: true,
            ..WorldStateView::default()
        });
        snapshot.seed_local_player(super::super::families::tests::local_player(at));
        snapshot.seed_stats(vec![StatView {
            index: 3,
            name: "hitpoints".into(),
            base: 99,
            effective: 5,
            xp: 0,
            used: true,
        }]);
        snapshot.seed_equipment(vec![]);
        snapshot.seed_npcs(vec![]);
        snapshot.seed_players(Vec::new());
        snapshot.seed_projectiles(Vec::new());
        snapshot.seed_hitmarks(HitmarksView {
            marks: [HitmarkView {
                value: 0,
                kind: 0,
                cycle: 0,
            }; 4],
            loop_cycle: 0,
        });
        let dose_id = crate::combat::guard::shared_tables()
            .unwrap()
            .potion(crate::combat::tables::PotionKind::Antipoison)
            .unwrap()
            .doses[0]
            .unwrap()
            .id;
        snapshot.seed_inventory(
            vec![ItemView {
                def: super::super::families::tests::def(dose_id, "Antipoison"),
                container: ItemContainer::Inventory,
                action_family: ItemActionFamily::Held,
                slot: 0,
                count: 1,
                actions: Vec::new(),
                component_id: 0,
            }],
            28,
        );

        let prayer = script
            .selected
            .prayers()
            .first()
            .expect("selected prayer data is available");
        let varp = prayer.varp;
        snapshot.seed_varps(
            script
                .selected
                .prayers()
                .iter()
                .enumerate()
                .map(|(index, row)| VarpView {
                    index: row.varp,
                    value: i32::from(index == 0),
                })
                .collect(),
        );
        let mut owned = RaisedPrayers::empty();
        owned.accepted(varp, true, 0);
        script.prayer_cleanup_owned = owned;
        script.prayer_cleanup_pending = true;

        let mut ledger = None;
        let handle = super::super::families::tests::with_tick(&snapshot, &mut ledger, 1, |tick| {
            let handle = tick
                .actions
                .begin::<ClearPrayers>(
                    ClearPrayersArgs::owned(Arc::clone(&script.selected), owned),
                    &mut tick.cx,
                )
                .unwrap();
            assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
            handle
        });
        script.clear_prayers = Some(handle);
        super::super::families::tests::with_tick(&snapshot, &mut ledger, 1, |tick| {
            let evidence = tick.cx.evidence();
            let authority = tick
                .cx
                .ledger
                .as_ref()
                .unwrap()
                .outbox
                .last()
                .unwrap()
                .authority();
            tick.cx.ledger.as_mut().unwrap().complete_interaction(
                &authority,
                crate::native::InteractionReceipt {
                    request_id: authority.request_id().get(),
                    evidence,
                    accepted: true,
                    chat_since: 0,
                },
            );
        });

        let condition = |poison| BotCondition {
            tick: 20,
            session: SessionMark::Live,
            hp: 5,
            hp_max: 99,
            hp_known: true,
            poison,
            poison_window: None,
            food: FoodSummary::default(),
            cures: CureDoses::default(),
        };
        super::super::families::tests::with_tick(&snapshot, &mut ledger, 2, |tick| {
            tick.cx.condition = Some(condition(PoisonState::Clear));
            tick.cx.cure_poison = CurePoison::WhenNeeded;
            script.tick(tick).unwrap();
        });
        assert!(script.clear_prayers.is_some());

        super::super::families::tests::with_tick(&snapshot, &mut ledger, 3, |tick| {
            tick.cx.condition = Some(condition(PoisonState::Onset {
                since: 20,
                q: Some(11),
                hit_since_line: false,
            }));
            tick.cx.poison_unknown_pulse = 11;
            tick.cx.cure_poison = CurePoison::WhenNeeded;
            script.tick(tick).unwrap();
        });
        assert!(script.clear_prayers.is_none());
        assert!(script.prayer_cleanup_pending);
        assert!(matches!(
            ledger.as_ref().unwrap().outbox.last().map(|action| &action.effect),
            Some(crate::native::HostEffect::Interaction(
                crate::interact::InteractReq::Held { action, .. }
            )) if action == "Drink"
        ));

        snapshot.seed_tick(21);
        snapshot.seed_inventory(Vec::new(), 28);
        super::super::families::tests::with_tick(&snapshot, &mut ledger, 4, |tick| {
            tick.cx.condition = Some(condition(PoisonState::Immune { until_lower: 80 }));
            tick.cx.cure_poison = CurePoison::WhenNeeded;
            let evidence = tick.cx.evidence();
            let authority = tick
                .cx
                .ledger
                .as_ref()
                .unwrap()
                .outbox
                .last()
                .unwrap()
                .authority();
            tick.cx.ledger.as_mut().unwrap().complete_interaction(
                &authority,
                crate::native::InteractionReceipt {
                    request_id: authority.request_id().get(),
                    evidence,
                    accepted: true,
                    chat_since: 0,
                },
            );
            script.tick(tick).unwrap();
        });
        // The drink sent at observed tick 3 delays the player through the next
        // two server ticks (`consume_effects.rs2:124-126`,
        // `PlayerOps.ts:376-378`), so the prayer clicks wait for the drink
        // lock and restart at tick 6.
        super::super::families::tests::with_tick(&snapshot, &mut ledger, 5, |tick| {
            tick.cx.condition = Some(condition(PoisonState::Immune { until_lower: 80 }));
            tick.cx.cure_poison = CurePoison::WhenNeeded;
            script.tick(tick).unwrap();
        });
        assert!(
            script.clear_prayers.is_none(),
            "no prayer click inside the drink lock"
        );
        super::super::families::tests::with_tick(&snapshot, &mut ledger, 6, |tick| {
            tick.cx.condition = Some(condition(PoisonState::Immune { until_lower: 80 }));
            tick.cx.cure_poison = CurePoison::WhenNeeded;
            script.tick(tick).unwrap();
        });
        assert!(
            script.clear_prayers.is_some(),
            "cleanup restarts after the drink"
        );
        super::super::families::tests::with_tick(&snapshot, &mut ledger, 7, |tick| {
            tick.cx.condition = Some(condition(PoisonState::Immune { until_lower: 80 }));
            tick.cx.cure_poison = CurePoison::WhenNeeded;
            script.tick(tick).unwrap();
        });
        assert!(matches!(
            ledger
                .as_ref()
                .unwrap()
                .outbox
                .last()
                .map(|action| &action.effect),
            Some(crate::native::HostEffect::Interaction(
                crate::interact::InteractReq::IfButton { .. }
            ))
        ));
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
    pub(super) fn status_fixture(
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

    #[test]
    fn jailer_terminal_status_uses_the_completed_sequence_cursor() {
        if !api::test_support::game_data_available() {
            return;
        }
        use super::super::path::PathDocument;

        let document: PathDocument =
            serde_json::from_slice(include_bytes!("../../paths/289/scorpcatcher.json")).unwrap();
        let (mut script, snapshot) = status_fixture(document);
        let stage = FactKey::new("scorpcatcher:2");
        script.seq_index =
            sequence_for_stage(&script.path, stage.0.as_ref()).expect("Jailer sequence");
        script.stage = Some(stage);
        script.step_index = 1;
        let sequence_len = script.path.sequences[script.seq_index].steps.len();
        let mut ledger = None;
        let mut output = StatusCapture::default();
        super::super::families::tests::with_tick_output(
            &snapshot,
            &mut ledger,
            1,
            &mut output,
            |tick| assert_eq!(script.finish_quest(tick), ScriptFlow::Complete),
        );

        let status = output.0.last().expect("terminal status");
        assert_eq!(status.phase, NativePhase::Complete);
        assert_eq!(
            status
                .fields
                .iter()
                .find(|field| field.key == "step_index")
                .and_then(|field| match &field.value {
                    StatusValue::Integer(value) => Some(*value),
                    _ => None,
                }),
            Some(sequence_len as i64)
        );
        assert_eq!(
            status
                .fields
                .iter()
                .find(|field| field.key == "remaining_steps")
                .and_then(|field| match &field.value {
                    StatusValue::Integer(value) => Some(*value),
                    _ => None,
                }),
            Some(0)
        );
        assert_eq!(status_text(status, "step_id"), "");
    }

    fn accept_last_interaction(tick: &mut crate::native::NativeTick<'_>) {
        let evidence = tick.cx.evidence();
        let authority = tick
            .cx
            .ledger
            .as_ref()
            .unwrap()
            .outbox
            .last()
            .unwrap()
            .authority();
        tick.cx.ledger.as_mut().unwrap().complete_interaction(
            &authority,
            crate::native::InteractionReceipt {
                request_id: authority.request_id().get(),
                evidence,
                accepted: true,
                chat_since: 0,
            },
        );
    }

    #[test]
    fn quester_resume_drops_step_and_cancels_provisioner_unchanged() {
        if !api::test_support::game_data_available() {
            return;
        }
        use crate::native::Interrupt;

        struct PendingStep;
        impl StepRun for PendingStep {
            fn poll(
                &mut self,
                _: &mut StepContext<'_, '_>,
            ) -> Poll<Result<StepOutcome, ActionError>> {
                Poll::Pending
            }

            fn cancel(&mut self, _: &mut crate::native::NativeActions) {}
        }

        let (mut step_script, _) = fixture();
        step_script.step = Some(Box::new(PendingStep));
        step_script.interrupt(Interrupt::Resume);
        assert!(step_script.step.is_none(), "Resume drops the active step");

        use crate::survival::CurePoison;
        use api::condition::{BotCondition, CureDoses, FoodSummary, PoisonState, SessionMark};
        let (mut provision_script, snapshot, memory, _) = provisioning_survival_fixture();
        let mut ledger = None;
        let condition = BotCondition {
            tick: 20,
            session: SessionMark::Live,
            hp: 60,
            hp_max: 99,
            hp_known: true,
            poison: PoisonState::Clear,
            poison_window: None,
            food: FoodSummary::default(),
            cures: CureDoses::default(),
        };
        super::super::families::tests::with_tick_bank(
            &snapshot,
            Some(&memory),
            &mut ledger,
            1,
            |tick| {
                tick.cx.condition = Some(condition);
                tick.cx.cure_poison = CurePoison::WhenNeeded;
                provision_script.tick(tick).unwrap();
                if tick
                    .cx
                    .ledger
                    .as_ref()
                    .is_some_and(|ledger| !ledger.outbox.is_empty())
                {
                    accept_last_interaction(tick);
                }
            },
        );
        assert!(provision_script.provisioner.has_run());
        provision_script.interrupt(Interrupt::Resume);
        assert!(!provision_script.provisioner.has_run());
    }

    #[test]
    fn quester_prayer_cleanup_park_still_blocks() {
        if !api::test_support::game_data_available() {
            return;
        }
        use super::super::families::tests::with_tick;

        let (mut script, snapshot) = fixture();
        script.parked = true;
        script.prayer_cleanup_pending = true;
        let mut ledger = None;
        let flow = with_tick(&snapshot, &mut ledger, 1, |tick| script.tick(tick).unwrap());

        assert!(matches!(flow, ScriptFlow::Blocked(_)));
        assert!(script.parked);
        assert!(script.begun.is_empty());
        assert!(ledger
            .as_ref()
            .is_none_or(|ledger| ledger.outbox.is_empty()));
    }

    fn provisioning_survival_fixture() -> (
        Quester,
        api::snapshot::GameSnapshot,
        api::bank_memory::BankMemory,
        api::snapshot::ItemView,
    ) {
        use super::super::path::{QuestBankDocument, QuestItemKindDocument};
        use api::bank_memory::{BankMemory, ObservedAt};
        use api::named_banks::{NamedBank, NamedBankFacts};
        use api::snapshot::QuestStatusView;
        use api::snapshot::{
            GameSnapshot, HitmarkView, HitmarksView, ItemActionFamily, ItemContainer, ItemView,
            StatView, VarpView, WorldStateView,
        };

        let stand = api::WorldTile {
            x: 3210,
            z: 3210,
            level: 0,
        };
        let bank = NamedBank::new("Test bank", stand);
        let banks = Arc::new(NamedBankFacts::from_banks(vec![bank]));
        let data = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
        let quests = Arc::new(QuestCatalog::from_identity(data.quest_identity()).unwrap());
        let mut document = super::super::compile::decode_cook().unwrap();
        let quest = document.quest.as_mut().unwrap();
        quest.owns_inventory = false;
        quest.bank = QuestBankDocument::Tile {
            tile: [stand.x, stand.z, stand.level],
            source: "test fixture".into(),
            required: true,
        };
        let item = quest.items.first_mut().expect("Cook path has a quest item");
        item.kind = QuestItemKindDocument::MustHave;
        item.acquire = None;
        item.from_stage = None;
        let path =
            super::super::compile::compile_uncached_for_test(&document, &data, &quests).unwrap();
        let provision_item = path
            .provisioning
            .items
            .first()
            .expect("the item is due at the not-started stage");
        let bank_item = ItemView {
            def: super::super::families::tests::def(provision_item.id, &provision_item.name),
            container: ItemContainer::Bank,
            action_family: ItemActionFamily::Component,
            slot: 0,
            count: i32::try_from(provision_item.qty).unwrap(),
            actions: vec![Some("Withdraw-1".into()), Some("Withdraw-X".into())],
            component_id: 7,
        };
        let mut memory = BankMemory::default();
        memory.observe(
            std::slice::from_ref(&bank_item),
            1,
            ObservedAt {
                unix_secs: 1,
                tick: 1,
                bank_session: 1,
            },
        );

        let mut script = Quester::new(
            RunKey {
                slot: 1,
                run: 1,
                session: 1,
            },
            path,
            Arc::clone(&data),
            quests,
            banks,
        );
        let stage = script.path.colour_not_started.clone();
        script.seq_index =
            sequence_for_stage(&script.path, stage.0.as_ref()).expect("not-started sequence");
        script.stage = Some(stage);
        script.needs_read = false;

        let mut snapshot = GameSnapshot::new();
        snapshot.seed_tick(20);
        snapshot.seed_ingame(2);
        snapshot.seed_world(WorldStateView {
            map_base_x: 3200,
            map_base_z: 3200,
            members: true,
            ..WorldStateView::default()
        });
        snapshot.seed_local_player(super::super::families::tests::local_player(stand));
        snapshot.seed_stats(vec![StatView {
            index: 3,
            name: "hitpoints".into(),
            base: 99,
            effective: 60,
            xp: 0,
            used: true,
        }]);
        snapshot.seed_equipment(vec![]);
        snapshot.seed_npcs(vec![]);
        snapshot.seed_players(Vec::new());
        snapshot.seed_projectiles(Vec::new());
        snapshot.seed_varps(
            data.prayers()
                .iter()
                .map(|prayer| VarpView {
                    index: prayer.varp,
                    value: 0,
                })
                .collect(),
        );
        snapshot.seed_hitmarks(HitmarksView {
            marks: [HitmarkView {
                value: 0,
                kind: 0,
                cycle: 0,
            }; 4],
            loop_cycle: 0,
        });
        let dose_id = crate::combat::guard::shared_tables()
            .unwrap()
            .potion(crate::combat::tables::PotionKind::Antipoison)
            .unwrap()
            .doses[0]
            .unwrap()
            .id;
        snapshot.seed_inventory(
            vec![ItemView {
                def: super::super::families::tests::def(dose_id, "Antipoison"),
                container: ItemContainer::Inventory,
                action_family: ItemActionFamily::Held,
                slot: 0,
                count: 1,
                actions: Vec::new(),
                component_id: 0,
            }],
            28,
        );
        snapshot.seed_bank_observation(10, 1, Some(vec![bank_item.clone()]), Vec::new());
        snapshot.seed_main_modal(10, Vec::new());
        snapshot.seed_quest_statuses(
            vec![QuestStatusView {
                name: "Cook's Assistant".into(),
                component_id: 42,
                colour: 0xf80000,
            }],
            true,
        );
        (script, snapshot, memory, bank_item)
    }

    #[test]
    fn quester_survival_cancels_provisioner_acquire_run() {
        if !api::test_support::game_data_available() {
            return;
        }
        use crate::survival::CurePoison;
        use api::condition::{BotCondition, CureDoses, FoodSummary, PoisonState, SessionMark};

        let (mut script, mut snapshot, memory, _) = provisioning_survival_fixture();
        let mut ledger = None;
        let condition = |poison, hp| BotCondition {
            tick: 20,
            session: SessionMark::Live,
            hp,
            hp_max: 99,
            hp_known: true,
            poison,
            poison_window: None,
            food: FoodSummary::default(),
            cures: CureDoses::default(),
        };
        super::super::families::tests::with_tick_bank(
            &snapshot,
            Some(&memory),
            &mut ledger,
            1,
            |tick| {
                tick.cx.condition = Some(condition(PoisonState::Clear, 60));
                tick.cx.cure_poison = CurePoison::WhenNeeded;
                script.tick(tick).unwrap();
                if tick
                    .cx
                    .ledger
                    .as_ref()
                    .is_some_and(|ledger| !ledger.outbox.is_empty())
                {
                    accept_last_interaction(tick);
                }
            },
        );
        assert!(script.provisioner.has_run());
        assert!(!script.provisioner.transfer_in_flight());

        snapshot.seed_main_modal(-1, Vec::new());
        super::super::families::tests::with_tick_bank(
            &snapshot,
            Some(&memory),
            &mut ledger,
            2,
            |tick| {
                tick.cx.condition = Some(condition(
                    PoisonState::Onset {
                        since: 20,
                        q: Some(11),
                        hit_since_line: false,
                    },
                    5,
                ));
                tick.cx.poison_unknown_pulse = 11;
                tick.cx.cure_poison = CurePoison::WhenNeeded;
                script.tick(tick).unwrap();
            },
        );
        assert!(!script.provisioner.has_run());
        assert!(matches!(
            ledger.as_ref().unwrap().outbox.last().map(|action| &action.effect),
            Some(crate::native::HostEffect::Interaction(
                crate::interact::InteractReq::Held { action, .. }
            )) if action == "Drink"
        ));
    }

    #[test]
    fn quester_parks_during_provisioning_and_restarts_trip_after_unpark() {
        if !api::test_support::game_data_available() {
            return;
        }
        use crate::survival::CurePoison;
        use api::condition::{BotCondition, CureDoses, FoodSummary, PoisonState, SessionMark};
        use api::snapshot::{ItemActionFamily, ItemContainer};

        let (mut script, mut snapshot, mut memory, bank_item) = provisioning_survival_fixture();
        snapshot.seed_inventory(Vec::new(), 28);
        let mut ledger = None;
        let condition = |poison, hp| BotCondition {
            tick: 20,
            session: SessionMark::Live,
            hp,
            hp_max: 99,
            hp_known: true,
            poison,
            poison_window: None,
            food: FoodSummary::default(),
            cures: CureDoses::default(),
        };
        super::super::families::tests::with_tick_bank(
            &snapshot,
            Some(&memory),
            &mut ledger,
            1,
            |tick| {
                tick.cx.condition = Some(condition(PoisonState::Clear, 60));
                tick.cx.cure_poison = CurePoison::WhenNeeded;
                script.tick(tick).unwrap();
                accept_last_interaction(tick);
            },
        );
        assert!(script.provisioner.has_run());

        super::super::families::tests::with_tick_bank(
            &snapshot,
            Some(&memory),
            &mut ledger,
            2,
            |tick| {
                tick.cx.condition = Some(condition(
                    PoisonState::Onset {
                        since: 20,
                        q: Some(11),
                        hit_since_line: false,
                    },
                    5,
                ));
                tick.cx.poison_unknown_pulse = 11;
                tick.cx.cure_poison = CurePoison::WhenNeeded;
                script.tick(tick).unwrap();
            },
        );
        assert!(!script.provisioner.has_run());
        assert!(script.survival.parked().is_some());
        assert_eq!(
            script.survival.report().decision,
            crate::survival::Decision::Parked
        );
        assert!(!script.parked);
        let parked_revision = script.provisioner.status_revision();
        let parked_outbox_len = ledger.as_ref().unwrap().outbox.len();
        snapshot.seed_tick(21);
        super::super::families::tests::with_tick_bank(
            &snapshot,
            Some(&memory),
            &mut ledger,
            3,
            |tick| {
                tick.cx.condition = Some(condition(
                    PoisonState::Onset {
                        since: 20,
                        q: Some(11),
                        hit_since_line: false,
                    },
                    5,
                ));
                tick.cx.poison_unknown_pulse = 11;
                tick.cx.cure_poison = CurePoison::WhenNeeded;
                script.tick(tick).unwrap();
            },
        );
        assert_eq!(script.provisioner.status_revision(), parked_revision);
        assert_eq!(ledger.as_ref().unwrap().outbox.len(), parked_outbox_len);
        assert!(script.survival.parked().is_some());

        snapshot.seed_tick(22);
        let mut held_item = bank_item;
        held_item.container = ItemContainer::Inventory;
        held_item.action_family = ItemActionFamily::Held;
        held_item.actions = vec![Some("Use".into())];
        snapshot.seed_main_modal(-1, Vec::new());
        snapshot.seed_bank_observation(-1, 2, None, Vec::new());
        snapshot.seed_inventory(vec![held_item], 28);
        memory.observe(
            &[],
            1,
            api::bank_memory::ObservedAt {
                unix_secs: 1,
                tick: 22,
                bank_session: 1,
            },
        );
        super::super::families::tests::with_tick_bank(
            &snapshot,
            Some(&memory),
            &mut ledger,
            4,
            |tick| {
                tick.cx.condition = Some(condition(PoisonState::Immune { until_lower: 80 }, 5));
                tick.cx.cure_poison = CurePoison::WhenNeeded;
                script.tick(tick).unwrap();
            },
        );
        assert!(script.survival.parked().is_none());
        assert!(!script.parked);
        assert_eq!(
            script.provisioner.status().phase,
            ProvisionPhase::Ready,
            "the trip is re-planned after the need clears"
        );
    }

    #[test]
    fn quester_survival_during_provisioning_with_transfer_in_flight() {
        if !api::test_support::game_data_available() {
            return;
        }
        use crate::survival::CurePoison;
        use api::condition::{BotCondition, CureDoses, FoodSummary, PoisonState, SessionMark};
        use api::snapshot::{ItemActionFamily, ItemContainer};

        let (mut script, mut snapshot, mut memory, bank_item) = provisioning_survival_fixture();
        let mut ledger = None;
        let condition = |poison, hp| BotCondition {
            tick: 20,
            session: SessionMark::Live,
            hp,
            hp_max: 99,
            hp_known: true,
            poison,
            poison_window: None,
            food: FoodSummary::default(),
            cures: CureDoses::default(),
        };

        let stand = api::WorldTile {
            x: 3210,
            z: 3210,
            level: 0,
        };
        let bank = api::named_banks::NamedBank::new("Test bank", stand);
        let access = Arc::new(crate::bank::BankStandAccess {
            bank,
            stand_tile: stand,
            kind: crate::bank::AccessKind::Booth,
            stand_op: 1,
            name: None,
            choose: None,
        });
        let respond_bank_pick = |tick: &mut crate::native::NativeTick<'_>| {
            let Some(action) = tick.cx.ledger.as_mut().and_then(|ledger| {
                let index = ledger.outbox.iter().position(|action| {
                    matches!(&action.effect, crate::native::HostEffect::BankPick(_))
                })?;
                Some(ledger.outbox.remove(index))
            }) else {
                return;
            };
            let authority = action.authority();
            let request_id = authority.request_id().get();
            let evidence = tick.cx.evidence();
            tick.cx.ledger.as_mut().unwrap().complete_bank_pick(
                &authority,
                crate::bank::BankPickReceipt {
                    request_id,
                    evidence,
                    outcome: crate::bank::BankPickOutcome::Selected(crate::bank::SelectedBank {
                        bank_index: 0,
                        access_tile: stand,
                        kind: crate::bank::PickKind::Reachable,
                        access: Some(Arc::clone(&access)),
                    }),
                },
            );
        };
        let respond_open_stand = |tick: &mut crate::native::NativeTick<'_>| {
            let Some(action) = tick.cx.ledger.as_mut().and_then(|ledger| {
                let index = ledger.outbox.iter().position(|action| {
                    matches!(
                        &action.effect,
                        crate::native::HostEffect::Interaction(
                            crate::interact::InteractReq::OpenStand { .. }
                        )
                    )
                })?;
                Some(ledger.outbox.remove(index))
            }) else {
                return;
            };
            let authority = action.authority();
            let evidence = tick.cx.evidence();
            tick.cx.ledger.as_mut().unwrap().complete_interaction(
                &authority,
                crate::native::InteractionReceipt {
                    request_id: authority.request_id().get(),
                    evidence,
                    accepted: true,
                    chat_since: 0,
                },
            );
        };
        let mut saw_withdraw = false;
        super::super::families::tests::with_tick_bank(
            &snapshot,
            Some(&memory),
            &mut ledger,
            1,
            |tick| {
                tick.cx.condition = Some(condition(PoisonState::Clear, 60));
                tick.cx.cure_poison = CurePoison::WhenNeeded;
                script.tick(tick).unwrap();
                respond_bank_pick(tick);
                respond_open_stand(tick);
                let (withdraw, interaction) =
                    tick.cx.ledger.as_ref().map_or((false, false), |ledger| {
                        let last = ledger.outbox.last().map(|action| &action.effect);
                        (
                            matches!(
                                last,
                                Some(crate::native::HostEffect::Interaction(
                                    crate::interact::InteractReq::WithdrawX { .. }
                                ))
                            ),
                            matches!(last, Some(crate::native::HostEffect::Interaction(_))),
                        )
                    });
                saw_withdraw |= withdraw;
                if interaction {
                    accept_last_interaction(tick);
                }
            },
        );
        assert!(script.provisioner.has_run());

        let mut next_tick = 2;
        if !script.provisioner.transfer_in_flight() {
            for _ in 0..10 {
                let poll_tick = next_tick;
                next_tick += 1;
                let outbox_before = ledger
                    .as_ref()
                    .and_then(|ledger| ledger.outbox.last())
                    .map_or(0, |action| action.request_id.get());
                super::super::families::tests::with_tick_bank(
                    &snapshot,
                    Some(&memory),
                    &mut ledger,
                    poll_tick,
                    |tick| {
                        tick.cx.condition = Some(condition(PoisonState::Clear, 60));
                        tick.cx.cure_poison = CurePoison::WhenNeeded;
                        script.tick(tick).unwrap();
                        respond_bank_pick(tick);
                        respond_open_stand(tick);
                        let (withdraw, interaction) =
                            tick.cx.ledger.as_ref().map_or((false, false), |ledger| {
                                let new_action = ledger
                                    .outbox
                                    .last()
                                    .is_some_and(|action| action.request_id.get() > outbox_before);
                                let last = ledger.outbox.last().map(|action| &action.effect);
                                (
                                    new_action
                                        && matches!(
                                            last,
                                            Some(crate::native::HostEffect::Interaction(
                                                crate::interact::InteractReq::WithdrawX { .. }
                                            ))
                                        ),
                                    new_action
                                        && matches!(
                                            last,
                                            Some(crate::native::HostEffect::Interaction(_))
                                        ),
                                )
                            });
                        saw_withdraw |= withdraw;
                        if interaction {
                            accept_last_interaction(tick);
                        }
                    },
                );
                if script.provisioner.transfer_in_flight() {
                    break;
                }
            }
        }
        assert!(saw_withdraw, "provisioning must reach its bank withdrawal");
        assert!(script.provisioner.transfer_in_flight());
        assert!(script.step.is_none());
        let onset_tick = next_tick;
        next_tick += 1;
        super::super::families::tests::with_tick_bank(
            &snapshot,
            Some(&memory),
            &mut ledger,
            onset_tick,
            |tick| {
                tick.cx.condition = Some(condition(
                    PoisonState::Onset {
                        since: 20,
                        q: Some(11),
                        hit_since_line: false,
                    },
                    5,
                ));
                tick.cx.poison_unknown_pulse = 11;
                tick.cx.cure_poison = CurePoison::WhenNeeded;
                script.tick(tick).unwrap();
            },
        );
        assert!(script.survival_interrupting);
        assert!(script.provisioner.transfer_in_flight());
        assert!(script.provisioner.has_run());
        assert!(script.step.is_none());
        assert!(!matches!(
            ledger.as_ref().unwrap().outbox.last().map(|action| &action.effect),
            Some(crate::native::HostEffect::Interaction(
                crate::interact::InteractReq::Held { action, .. }
            )) if action == "Drink"
        ));

        let mut held_item = bank_item;
        held_item.container = ItemContainer::Inventory;
        held_item.action_family = ItemActionFamily::Held;
        held_item.slot = 1;
        held_item.actions = vec![Some("Use".into())];
        let dose_id = crate::combat::guard::shared_tables()
            .unwrap()
            .potion(crate::combat::tables::PotionKind::Antipoison)
            .unwrap()
            .doses[0]
            .unwrap()
            .id;
        let dose_item = api::snapshot::ItemView {
            def: super::super::families::tests::def(dose_id, "Antipoison"),
            container: ItemContainer::Inventory,
            action_family: ItemActionFamily::Held,
            slot: 0,
            count: 1,
            actions: Vec::new(),
            component_id: 0,
        };
        snapshot.seed_tick(21);
        snapshot.seed_inventory(vec![dose_item.clone(), held_item.clone()], 28);
        snapshot.seed_bank_observation(10, 2, Some(Vec::new()), Vec::new());
        memory.observe(
            &[],
            1,
            api::bank_memory::ObservedAt {
                unix_secs: 1,
                tick: 21,
                bank_session: 1,
            },
        );
        let boundary_tick = next_tick;
        next_tick += 1;
        super::super::families::tests::with_tick_bank(
            &snapshot,
            Some(&memory),
            &mut ledger,
            boundary_tick,
            |tick| {
                tick.cx.condition = Some(condition(
                    PoisonState::Onset {
                        since: 20,
                        q: Some(11),
                        hit_since_line: false,
                    },
                    5,
                ));
                tick.cx.poison_unknown_pulse = 11;
                tick.cx.cure_poison = CurePoison::WhenNeeded;
                script.tick(tick).unwrap();
            },
        );
        assert!(!script.provisioner.has_run());
        assert!(!script.survival_interrupting);
        assert!(matches!(
            ledger
                .as_ref()
                .unwrap()
                .outbox
                .last()
                .map(|action| &action.effect),
            Some(crate::native::HostEffect::Interaction(
                crate::interact::InteractReq::CloseModal
            ))
        ));

        snapshot.seed_tick(21);
        snapshot.seed_main_modal(-1, Vec::new());
        snapshot.seed_bank_observation(-1, 2, None, Vec::new());
        let close_modal_tick = next_tick;
        next_tick += 1;
        super::super::families::tests::with_tick_bank(
            &snapshot,
            Some(&memory),
            &mut ledger,
            close_modal_tick,
            |tick| {
                tick.cx.condition = Some(condition(
                    PoisonState::Onset {
                        since: 20,
                        q: Some(11),
                        hit_since_line: false,
                    },
                    5,
                ));
                tick.cx.poison_unknown_pulse = 11;
                tick.cx.cure_poison = CurePoison::WhenNeeded;
                accept_last_interaction(tick);
                script.tick(tick).unwrap();
            },
        );
        let drink_tick = next_tick;
        next_tick += 1;
        super::super::families::tests::with_tick_bank(
            &snapshot,
            Some(&memory),
            &mut ledger,
            drink_tick,
            |tick| {
                tick.cx.condition = Some(condition(
                    PoisonState::Onset {
                        since: 20,
                        q: Some(11),
                        hit_since_line: false,
                    },
                    5,
                ));
                tick.cx.poison_unknown_pulse = 11;
                tick.cx.cure_poison = CurePoison::WhenNeeded;
                script.tick(tick).unwrap();
            },
        );
        assert!(matches!(
            ledger.as_ref().unwrap().outbox.last().map(|action| &action.effect),
            Some(crate::native::HostEffect::Interaction(
                crate::interact::InteractReq::Held { action, .. }
            )) if action == "Drink"
        ));

        // The drink went out in the close's settle tick; its dose drop shows
        // one tick later than usual, so its lock counts from that drop
        // (`SurvivalStep::record_settled`).
        snapshot.seed_tick(22);
        snapshot.seed_inventory(vec![held_item], 28);
        let immune_accept_tick = next_tick;
        next_tick += 1;
        super::super::families::tests::with_tick_bank(
            &snapshot,
            Some(&memory),
            &mut ledger,
            immune_accept_tick,
            |tick| {
                tick.cx.condition = Some(condition(PoisonState::Immune { until_lower: 80 }, 5));
                tick.cx.cure_poison = CurePoison::WhenNeeded;
                accept_last_interaction(tick);
                script.tick(tick).unwrap();
            },
        );
        let lock_tick = next_tick;
        next_tick += 1;
        super::super::families::tests::with_tick_bank(
            &snapshot,
            Some(&memory),
            &mut ledger,
            lock_tick,
            |tick| {
                tick.cx.condition = Some(condition(PoisonState::Immune { until_lower: 80 }, 5));
                tick.cx.cure_poison = CurePoison::WhenNeeded;
                script.tick(tick).unwrap();
            },
        );
        assert_eq!(
            script.provisioner.status().phase,
            ProvisionPhase::Idle,
            "provisioning waits inside the lock of the drink applied at the dose drop"
        );
        let settle_tick = next_tick;
        super::super::families::tests::with_tick_bank(
            &snapshot,
            Some(&memory),
            &mut ledger,
            settle_tick,
            |tick| {
                tick.cx.condition = Some(condition(PoisonState::Immune { until_lower: 80 }, 5));
                tick.cx.cure_poison = CurePoison::WhenNeeded;
                script.tick(tick).unwrap();
            },
        );
        assert!(!script.provisioner.has_run());
        assert_eq!(script.provisioner.status().phase, ProvisionPhase::Ready);
    }

    #[test]
    fn parked_runner_keeps_eating_and_unparks_when_need_clears() {
        if !api::test_support::game_data_available() {
            return;
        }
        use super::super::families::tests::with_tick;
        use crate::native::{HostEffect, ScriptFlow};
        use crate::survival::CurePoison;
        use api::condition::{BotCondition, CureDoses, FoodSummary, PoisonState, SessionMark};
        use api::snapshot::{ItemActionFamily, ItemContainer, ItemView, StatView};

        let (mut script, mut snapshot, _, _) = provisioning_survival_fixture();
        snapshot.seed_main_modal(-1, Vec::new());
        snapshot.seed_inventory(Vec::new(), 28);
        let data = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
        let lobster_id = data.item_by_alias("lobster").unwrap().id;
        let set_hp = |snapshot: &mut api::snapshot::GameSnapshot, effective| {
            snapshot.seed_stats(vec![StatView {
                index: 3,
                name: "hitpoints".into(),
                base: 99,
                effective,
                xp: 0,
                used: true,
            }]);
        };
        let condition = |poison, hp, food| BotCondition {
            tick: 20,
            session: SessionMark::Live,
            hp,
            hp_max: 99,
            hp_known: true,
            poison,
            poison_window: None,
            food,
            cures: CureDoses::default(),
        };
        let mut ledger = None;

        with_tick(&snapshot, &mut ledger, 1, |tick| {
            tick.cx.condition = Some(condition(
                PoisonState::Onset {
                    since: 20,
                    q: Some(11),
                    hit_since_line: false,
                },
                5,
                FoodSummary::default(),
            ));
            tick.cx.poison_unknown_pulse = 11;
            tick.cx.cure_poison = CurePoison::WhenNeeded;
            assert!(matches!(script.tick(tick).unwrap(), ScriptFlow::Continue));
        });
        assert!(script.survival.parked().is_some());

        snapshot.seed_inventory(
            vec![ItemView {
                def: super::super::families::tests::def(lobster_id, "Lobster"),
                container: ItemContainer::Inventory,
                action_family: ItemActionFamily::Held,
                slot: 0,
                count: 1,
                actions: vec![Some("Eat".into())],
                component_id: 3214,
            }],
            28,
        );
        set_hp(&mut snapshot, 5);
        with_tick(&snapshot, &mut ledger, 2, |tick| {
            tick.cx.condition = Some(condition(
                PoisonState::Onset {
                    since: 20,
                    q: Some(11),
                    hit_since_line: false,
                },
                5,
                FoodSummary {
                    ordinary_count: 1,
                    ordinary_heal_total: 12,
                    best_heal: 12,
                    smallest_heal: 12,
                    combo_count: 0,
                    more_kinds: false,
                },
            ));
            tick.cx.poison_unknown_pulse = 11;
            tick.cx.cure_poison = CurePoison::WhenNeeded;
            assert!(matches!(script.tick(tick).unwrap(), ScriptFlow::Continue));
            assert!(matches!(
                tick.cx.ledger.as_ref().unwrap().outbox.last().map(|action| &action.effect),
                Some(HostEffect::Interaction(
                    crate::interact::InteractReq::Held { action, .. }
                )) if action == "Eat"
            ));
            accept_last_interaction(tick);
        });
        set_hp(&mut snapshot, 17);
        assert!(script.survival.parked().is_some());

        snapshot.seed_inventory(Vec::new(), 28);
        with_tick(&snapshot, &mut ledger, 3, |tick| {
            tick.cx.condition = Some(condition(
                PoisonState::Immune { until_lower: 80 },
                17,
                FoodSummary::default(),
            ));
            tick.cx.cure_poison = CurePoison::WhenNeeded;
            assert!(matches!(script.tick(tick).unwrap(), ScriptFlow::Continue));
        });
        assert!(script.survival.parked().is_none());
        assert!(script.survival_action.is_none());
    }

    #[test]
    fn parked_quester_publishes_waiting_continue_status_and_resumes_parked() {
        if !api::test_support::game_data_available() {
            return;
        }
        use super::super::families::tests::with_tick_output;
        use crate::native::{Interrupt, NativePhase, ScriptFlow};
        use crate::survival::CurePoison;
        use api::condition::{BotCondition, CureDoses, FoodSummary, PoisonState, SessionMark};

        let (mut script, mut snapshot, _, _) = provisioning_survival_fixture();
        snapshot.seed_inventory(Vec::new(), 28);
        snapshot.seed_main_modal(-1, Vec::new());
        snapshot.seed_chat_modal(-1, Vec::new());
        let condition = BotCondition {
            tick: 20,
            session: SessionMark::Live,
            hp: 5,
            hp_max: 99,
            hp_known: true,
            poison: PoisonState::Onset {
                since: 20,
                q: Some(11),
                hit_since_line: false,
            },
            poison_window: None,
            food: FoodSummary::default(),
            cures: CureDoses::default(),
        };
        let mut ledger = None;
        let mut output = StatusCapture::default();

        with_tick_output(&snapshot, &mut ledger, 1, &mut output, |tick| {
            tick.cx.condition = Some(condition);
            tick.cx.cure_poison = CurePoison::WhenNeeded;
            assert!(matches!(script.tick(tick).unwrap(), ScriptFlow::Continue));
        });
        assert!(script.waits_for_evidence());
        assert!(script.survival.parked().is_some());
        let status = output.0.last().expect("parked status is published");
        assert_eq!(status.phase, NativePhase::Waiting);
        assert!(status.failure.is_none());

        script.interrupt(Interrupt::Pause);
        assert!(script.survival.parked().is_some());
        script.interrupt(Interrupt::Resume);
        assert!(script.survival.parked().is_some());

        with_tick_output(&snapshot, &mut ledger, 2, &mut output, |tick| {
            tick.cx.condition = Some(condition);
            tick.cx.cure_poison = CurePoison::WhenNeeded;
            assert!(matches!(script.tick(tick).unwrap(), ScriptFlow::Continue));
        });
        assert!(script.waits_for_evidence());
        let status = output
            .0
            .last()
            .expect("continued parked status is published");
        assert_eq!(status.phase, NativePhase::Waiting);
        assert!(status.failure.is_none());
    }

    #[derive(Default)]
    struct StatusCapture(Vec<ScriptStatus>);

    impl NativeOutput for StatusCapture {
        fn status(&mut self, status: ScriptStatus) {
            self.0.push(status);
        }
        fn paint(&mut self, _: Arc<crate::paint::ScriptPaint>) {}
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
        fn paint(&mut self, _: Arc<crate::paint::ScriptPaint>) {}
        fn log(&mut self, level: api::hostlog::Level, message: &str) {
            self.logs.push((level, message.to_owned()));
        }
        fn settings_applied(&mut self, _: u64) {}
    }

    /// A stage caps its distinct lines at its own budget (the 32-line floor
    /// for a stage with no steps), warns once, and the next stage starts over.
    #[test]
    fn run_trace_caps_distinct_events() {
        let mut trace = RunTrace::default();
        let mut output = TraceCapture::default();
        trace.begin_stage(&mut output, 0);
        for event in 0..=RUN_TRACE_EVENT_MIN {
            trace.record(
                &mut output,
                api::hostlog::Level::Info,
                format_args!("event {event}"),
            );
        }
        assert_eq!(output.logs.len(), RUN_TRACE_EVENT_MIN);
        assert!(trace.truncated);
        trace.flush_repeats(&mut output);
        assert!(output
            .logs
            .last()
            .unwrap()
            .1
            .contains("truncated after the event limit"));
    }

    /// The budget is `4 × units + 8` above the floor, where a unit is one of
    /// the stage's steps or one step of a recipe its acquire steps run; one
    /// stage keeps a begin, skip and settled/failed line for each of them.
    #[test]
    fn run_trace_stage_budget_is_sized_from_the_stage() {
        assert_eq!(stage_event_limit(0), RUN_TRACE_EVENT_MIN);
        assert_eq!(stage_event_limit(6), RUN_TRACE_EVENT_MIN);
        assert_eq!(stage_event_limit(7), 36);
        assert_eq!(stage_event_limit(27), 116);
        assert_eq!(stage_event_limit(13 + 32), 188);
        assert_eq!(stage_event_limit(107), 436);
        assert_eq!(stage_event_limit(usize::MAX), usize::MAX);
    }

    /// A stage longer than the old run-wide 32-line budget keeps every step
    /// line: more than 32 distinct lines are logged, not dropped.
    #[test]
    fn run_trace_keeps_every_line_of_a_long_stage() {
        let mut trace = RunTrace::default();
        let mut output = TraceCapture::default();
        let steps = 24;
        trace.begin_stage(&mut output, steps);
        assert_eq!(trace.limit, stage_event_limit(steps));
        for step in 0..steps {
            for event in ["begin", "skipped", "settled"] {
                trace.record(
                    &mut output,
                    api::hostlog::Level::Info,
                    format_args!("quester test: stage long:0 step {step} (wait) {event}"),
                );
            }
        }
        assert!(
            steps * 3 > RUN_TRACE_EVENT_MIN,
            "the stage must need more lines than the old run budget"
        );
        assert_eq!(output.logs.len(), steps * 3);
        assert!(!trace.truncated, "every step line fits: {:#?}", output.logs);
    }

    /// A truncated stage warns when it ends, then hands the next stage a
    /// fresh budget: its lines are logged again, including lines the ending
    /// stage had to drop.
    #[test]
    fn run_trace_new_stage_after_truncation_logs_its_lines_again() {
        let mut trace = RunTrace::default();
        let mut output = TraceCapture::default();
        trace.begin_stage(&mut output, 0);
        for event in 0..40 {
            trace.record(
                &mut output,
                api::hostlog::Level::Info,
                format_args!("event {event}"),
            );
        }
        assert!(trace.truncated);
        assert_eq!(output.logs.len(), RUN_TRACE_EVENT_MIN);
        let before = output.logs.len();
        trace.begin_stage(&mut output, 7);
        assert!(trace.limit > RUN_TRACE_EVENT_MIN);
        assert_eq!(
            output.logs[before..]
                .iter()
                .map(|(_, line)| line.as_str())
                .collect::<Vec<_>>(),
            vec!["quester trace truncated after the event limit"]
        );
        assert!(!trace.truncated);
        assert!(trace.entries.is_empty());
        assert!(trace.repeated.is_empty());
        for event in 0..trace.limit {
            trace.record(
                &mut output,
                api::hostlog::Level::Info,
                format_args!("event {event}"),
            );
        }
        assert_eq!(
            output.logs.len(),
            before + 1 + trace.limit,
            "the new stage logs every line, including ones the ending stage dropped"
        );
        assert!(output.logs[before + 1..]
            .iter()
            .any(|(_, line)| line == "event 32"));
        assert!(!trace.truncated);
    }

    /// Repeated identical lines still fold into one line plus a count, and
    /// only then is the line text kept for the summary.
    #[test]
    fn run_trace_folds_repeated_lines() {
        let mut trace = RunTrace::default();
        let mut output = TraceCapture::default();
        trace.begin_stage(&mut output, 4);
        for _ in 0..3 {
            trace.record(
                &mut output,
                api::hostlog::Level::Info,
                format_args!("quester test: stage long:0 step retry (wait) begin"),
            );
        }
        assert_eq!(
            output.logs.len(),
            1,
            "identical lines log once: {:?}",
            output.logs
        );
        assert_eq!(trace.entries.len(), 1);
        assert_eq!(trace.entries[0].repeats, 2);
        assert_eq!(
            trace.repeated.len(),
            1,
            "only the repeated entry keeps its text"
        );
        assert_eq!(
            &*trace.repeated[0].line,
            "quester test: stage long:0 step retry (wait) begin"
        );
        trace.flush_repeats(&mut output);
        assert_eq!(output.logs.len(), 2);
        assert!(output.logs[1].1.ends_with("(repeated 2 additional times)"));
        assert!(
            trace.repeated.is_empty(),
            "the flushed summary drops the text it printed"
        );
    }

    /// A line logged once keeps no text: the table holds only its hash and a
    /// zero repeat count until the same line repeats.
    #[test]
    fn run_trace_keeps_no_text_until_a_line_repeats() {
        let mut trace = RunTrace::default();
        let mut output = TraceCapture::default();
        trace.begin_stage(&mut output, 4);
        for event in ["begin", "settled"] {
            trace.record(
                &mut output,
                api::hostlog::Level::Info,
                format_args!("quester test: stage long:0 step once (wait) {event}"),
            );
        }
        assert_eq!(trace.entries.len(), 2);
        assert!(trace.repeated.is_empty(), "distinct lines keep no text");
        trace.record(
            &mut output,
            api::hostlog::Level::Info,
            format_args!("quester test: stage long:0 step once (wait) begin"),
        );
        assert_eq!(
            trace.repeated.len(),
            1,
            "the first repeat keeps the line text"
        );
        assert_eq!(trace.entries[trace.repeated[0].entry].repeats, 1);
        assert_eq!(
            &*trace.repeated[0].line,
            "quester test: stage long:0 step once (wait) begin"
        );
    }

    /// The worst bundled stage by trace units: its own steps plus every step
    /// of the recipes its acquire steps run, nested recipes included.
    fn worst_bundled_stage_units() -> (usize, String) {
        use super::super::registry::PathRegistry;
        let empty = std::collections::BTreeMap::new();
        let mut worst = (0, String::new());
        for id in PathRegistry::Bundled.ids() {
            let Some(bytes) = super::super::compile::path_bytes(&id) else {
                continue;
            };
            let document: super::super::path::PathDocument =
                serde_json::from_slice(bytes).expect("bundled Path document");
            let recipes = document
                .quest
                .as_ref()
                .map_or(&empty, |quest| &quest.acquire);
            for sequence in document.roles.iter().flat_map(|role| &role.sequences) {
                let units = super::super::compile::sequence_trace_units(&sequence.steps, recipes);
                if units > worst.0 {
                    worst = (units, sequence.stage.0.to_string());
                }
            }
        }
        worst
    }

    /// The worst bundled stage (`prince:30`: 7 steps and 100 recipe steps)
    /// sets the table's ceiling: 107 units, so 436 distinct lines. A distinct
    /// line keeps two digests and a count and no text, so the retained heap is
    /// the entry vector alone.
    #[test]
    fn run_trace_worst_bundled_stage_table_stays_under_sixteen_kib() {
        let (worst, stage) = worst_bundled_stage_units();
        assert_eq!(
            (worst, stage.as_str()),
            (107, "prince:30"),
            "the bundled guides changed their worst stage; re-measure the trace ceiling"
        );
        let mut trace = RunTrace::default();
        // `StatusCapture` ignores log lines, so the measured closure counts
        // only the trace's own allocations.
        let mut output = StatusCapture::default();
        trace.begin_stage(&mut output, worst);
        assert_eq!(trace.limit, 436);
        let measured = allocation_counter::measure(|| {
            for line in 0..trace.limit {
                trace.record(
                    &mut output,
                    api::hostlog::Level::Info,
                    format_args!("quester test: stage {stage} step {line} (walk) begin"),
                );
            }
        });
        let retained = usize::try_from(measured.bytes_current).unwrap();
        let table = trace.entries.capacity() * std::mem::size_of::<TraceEntry>()
            + trace.repeated.capacity() * std::mem::size_of::<RepeatedLine>();
        println!(
            "RunTrace worst stage: {} distinct lines, {}B retained, {} entries x {}B + {} repeats x {}B",
            trace.entries.len(),
            retained,
            trace.entries.capacity(),
            std::mem::size_of::<TraceEntry>(),
            trace.repeated.capacity(),
            std::mem::size_of::<RepeatedLine>()
        );
        assert_eq!(trace.entries.len(), 436);
        assert!(trace.repeated.is_empty(), "no line repeats in this stage");
        assert_eq!(
            retained, table,
            "the retained heap is the table's own buffers"
        );
        assert!(
            retained < 16 * 1024,
            "a 436-line stage keeps {retained}B, over the 16 KiB budget"
        );
    }

    /// A stage whose acquire recipe has 32 children logs every child's begin
    /// and settled line and every later step line. The step-only budget held
    /// 60 lines here, fewer than the 64 child lines alone, so later lines were
    /// dropped; the budget now counts the recipe's steps too.
    #[test]
    fn wide_acquire_recipe_logs_every_child_and_later_step_line() {
        if !api::test_support::game_data_available() {
            return;
        }
        use super::super::families::tests::with_tick_output;
        use super::super::path::PredicateDocument;

        let wait = || serde_json::json!({"until":{"All":[]},"max_ticks":10});
        let mut document = super::super::compile::decode_cook().unwrap();
        document.quest.as_mut().unwrap().acquire.insert(
            "test:wide".to_owned(),
            (1..=32)
                .map(|n| {
                    test_step(
                        &format!("wide-{n}"),
                        "wait",
                        wait(),
                        PredicateDocument::Any(vec![]),
                        PredicateDocument::All(vec![]),
                    )
                })
                .collect(),
        );
        let mut steps = vec![test_step(
            "wide-acquire",
            "acquire",
            serde_json::json!({"recipe":"test:wide"}),
            PredicateDocument::Any(vec![]),
            PredicateDocument::All(vec![]),
        )];
        steps.extend((1..=12).map(|n| {
            test_step(
                &format!("after-{n}"),
                "wait",
                wait(),
                PredicateDocument::Any(vec![]),
                PredicateDocument::All(vec![]),
            )
        }));
        let units = steps.len() + 32;
        assert_eq!(units, 45, "13 sequence steps and 32 recipe children");
        document.roles[0].sequences[0].steps = steps;
        document.roles[0].sequences[0].order = super::super::path::SequenceOrder::Ordered;
        let (mut script, mut snapshot) = status_fixture(document);
        // Start before the first stage is known so the run adopts `cook:0` and
        // sizes its trace budget from this sequence.
        script.stage = None;
        script.needs_read = true;
        let mut ledger = None;
        let mut output = TraceCapture::default();
        for tick in 1..=300 {
            // Real steps move the player or train a skill. The synthetic waits
            // change nothing observable, so give the no-progress watchdog a
            // fresh xp total at every step boundary.
            snapshot.seed_stats(vec![api::snapshot::StatView {
                index: 0,
                name: api::snapshot::stat_name(0).to_owned(),
                effective: 1,
                base: 1,
                xp: i32::try_from(tick).unwrap_or(i32::MAX),
                used: true,
            }]);
            with_tick_output(&snapshot, &mut ledger, tick, &mut output, |native| {
                script.tick(native).unwrap();
            });
            if tick == 1 {
                assert_eq!(
                    script.stage.as_ref().map(|stage| stage.0.as_ref()),
                    Some("cook:0"),
                    "the run adopts the first stage on its first tick"
                );
                assert_eq!(
                    script.trace.limit,
                    stage_event_limit(units),
                    "the stage budget counts its 32 recipe children"
                );
            }
            let done = output
                .logs
                .iter()
                .any(|(_, line)| line.ends_with("step after-12 (wait) settled"));
            if done || script.parked {
                break;
            }
        }
        let lines: Vec<&str> = output.logs.iter().map(|(_, line)| line.as_str()).collect();
        for n in 1..=32 {
            for event in ["begin", "settled"] {
                let expected =
                    format!("quester cook: stage cook:0 recipe test:wide child wide-{n} {event}");
                assert!(
                    lines.iter().any(|line| *line == expected),
                    "missing {expected}: {lines:#?}"
                );
            }
        }
        for n in 1..=12 {
            for event in ["begin", "settled"] {
                let expected = format!("quester cook: stage cook:0 step after-{n} (wait) {event}");
                assert!(
                    lines.iter().any(|line| *line == expected),
                    "missing {expected}: {lines:#?}"
                );
            }
        }
        assert!(
            lines.iter().any(|line| {
                *line == "quester cook: stage cook:0 step wide-acquire (acquire) settled"
            }),
            "the acquire step settles after its recipe: {lines:#?}"
        );
        let last_child = lines
            .iter()
            .rposition(|line| {
                *line == "quester cook: stage cook:0 recipe test:wide child wide-32 settled"
            })
            .expect("the last child settles");
        assert!(
            !lines[..last_child].contains(&"quester trace truncated after the event limit"),
            "the stage truncated before its last child line: {lines:#?}"
        );
    }

    /// A new run key prints the ending run's pending repeat counts and starts
    /// the current stage's budget. Without it the new run kept the 32-line
    /// floor, because the unchanged stage never enters the stage hook again.
    #[test]
    fn new_run_key_flushes_repeats_and_sizes_the_current_stage() {
        if !api::test_support::game_data_available() {
            return;
        }
        use super::super::families::tests::with_tick_output;
        use super::super::path::PredicateDocument;

        let mut document = super::super::compile::decode_cook().unwrap();
        document.roles[0].sequences[0].steps = (1..=10)
            .map(|n| {
                test_step(
                    &format!("pending-{n}"),
                    "wait",
                    serde_json::json!({"until":{"Any":[]},"max_ticks":1000}),
                    PredicateDocument::Any(vec![]),
                    PredicateDocument::All(vec![]),
                )
            })
            .collect();
        let steps = document.roles[0].sequences[0].steps.len();
        let (mut script, snapshot) = status_fixture(document);
        script.stage = None;
        script.needs_read = true;
        let mut ledger = None;
        let mut output = TraceCapture::default();
        with_tick_output(&snapshot, &mut ledger, 1, &mut output, |native| {
            script.tick(native).unwrap();
        });
        assert_eq!(
            script.stage.as_ref().map(|stage| stage.0.as_ref()),
            Some("cook:0")
        );
        assert_eq!(
            script.trace.limit,
            stage_event_limit(steps),
            "the stage budget is sized from its own steps"
        );
        assert!(script.trace.limit > RUN_TRACE_EVENT_MIN);
        // A line that repeated in the ending run still owes its count.
        for _ in 0..2 {
            script.trace.record(
                &mut output,
                api::hostlog::Level::Info,
                format_args!("quester cook: pending"),
            );
        }
        let before = output.logs.len();
        // The queue forwards a new run key to the existing Quester.
        script.run = RunKey {
            slot: 1,
            run: 2,
            session: 1,
        };
        with_tick_output(&snapshot, &mut ledger, 2, &mut output, |native| {
            script.tick(native).unwrap();
        });
        let fresh: Vec<&str> = output.logs[before..]
            .iter()
            .map(|(_, line)| line.as_str())
            .collect();
        assert_eq!(
            fresh.first().copied(),
            Some("quester cook: pending (repeated 1 additional times)"),
            "the ending run's count is printed before the new trace: {fresh:#?}"
        );
        assert_eq!(
            script.trace.limit,
            stage_event_limit(steps),
            "the new run keeps the current stage's budget, not the 32-line floor"
        );
        assert!(
            fresh.contains(&"quester cook: run start stage=cook:0"),
            "the new run starts its own trace: {fresh:#?}"
        );
    }

    /// A first-digest collision with a different line logs that line as its own
    /// entry instead of hiding it and naming the wrong line in a summary; the
    /// same line still folds into its new entry.
    #[test]
    fn run_trace_hash_collision_logs_the_different_line() {
        let mut trace = RunTrace::default();
        let mut output = TraceCapture::default();
        trace.begin_stage(&mut output, 4);
        trace.record(
            &mut output,
            api::hostlog::Level::Info,
            format_args!("first line"),
        );
        assert_eq!(output.logs.len(), 1);
        // Make the stored first line collide with a line that is not in the
        // table in the first digest only: its second digest stays its own.
        trace.entries[0].hash = trace_hash("second line");
        trace.entries[0].check = trace_check("first line");
        trace.record(
            &mut output,
            api::hostlog::Level::Info,
            format_args!("second line"),
        );
        assert_eq!(
            output.logs.len(),
            2,
            "a different line is logged, not skipped: {:?}",
            output.logs
        );
        assert_eq!(trace.entries.len(), 2, "it counts as a new entry");
        assert_eq!(
            trace.entries[0].repeats, 0,
            "the colliding entry's count is untouched"
        );
        assert!(trace.repeated.is_empty(), "no summary was started");
        // The same line again folds into its own entry, not the colliding one.
        trace.record(
            &mut output,
            api::hostlog::Level::Info,
            format_args!("second line"),
        );
        assert_eq!(output.logs.len(), 2, "the same line folds");
        assert_eq!(trace.entries[1].repeats, 1);
        assert_eq!(
            trace.repeated[0].entry, 1,
            "the summary names the line's own entry"
        );
        assert_eq!(&*trace.repeated[0].line, "second line");
    }

    /// A stage longer than the old 32-line run budget logs every step line in
    /// a real run: 20 steps keep their begin and settled lines.
    #[test]
    fn long_stage_logs_every_step_line_across_the_old_run_cap() {
        if !api::test_support::game_data_available() {
            return;
        }
        use super::super::families::tests::with_tick_output;
        use super::super::path::{PredicateDocument, SequenceOrder};

        let mut document = super::super::compile::decode_cook().unwrap();
        let first = &mut document.roles[0].sequences[0];
        first.order = SequenceOrder::Ordered;
        first.steps = (1..=20)
            .map(|n| {
                test_step(
                    &format!("long-{n}"),
                    "wait",
                    serde_json::json!({"until":{"All":[]},"max_ticks":10}),
                    PredicateDocument::Any(vec![]),
                    PredicateDocument::All(vec![]),
                )
            })
            .collect();
        let (mut script, mut snapshot) = status_fixture(document);
        // Start before the first stage is known so the run adopts `cook:0` and
        // sizes its trace budget from this sequence.
        script.stage = None;
        script.needs_read = true;
        let mut ledger = None;
        let mut output = TraceCapture::default();
        for tick in 1..=200 {
            // Real steps move the player or train a skill. The synthetic waits
            // change nothing observable, so give the no-progress watchdog a
            // fresh xp total at every step boundary.
            snapshot.seed_stats(vec![api::snapshot::StatView {
                index: 0,
                name: api::snapshot::stat_name(0).to_owned(),
                effective: 1,
                base: 1,
                xp: i32::try_from(tick).unwrap_or(i32::MAX),
                used: true,
            }]);
            with_tick_output(&snapshot, &mut ledger, tick, &mut output, |native| {
                script.tick(native).unwrap();
            });
            if script.parked {
                break;
            }
        }
        assert!(
            script.parked,
            "the ordered sequence runs off its end: {:#?}",
            output.logs
        );
        let lines: Vec<&str> = output.logs.iter().map(|(_, line)| line.as_str()).collect();
        for n in 1..=20 {
            assert!(
                lines
                    .iter()
                    .any(|line| line.ends_with(&format!("step long-{n} (wait) begin"))),
                "step long-{n} keeps its begin line: {lines:#?}"
            );
            assert!(
                lines
                    .iter()
                    .any(|line| line.ends_with(&format!("step long-{n} (wait) settled"))),
                "step long-{n} keeps its settled line: {lines:#?}"
            );
        }
        assert_eq!(
            lines
                .iter()
                .filter(|line| line.contains("step long-") && line.contains(" begin"))
                .count(),
            20,
            "each of the 20 begins is logged: {lines:#?}"
        );
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
            max_attempts: None,
            stop: None,
            hazard: None,
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
        if !api::test_support::game_data_available() {
            return;
        }
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
        if !api::test_support::game_data_available() {
            return;
        }
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
        if !api::test_support::game_data_available() {
            return;
        }
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
        if !api::test_support::game_data_available() {
            return;
        }
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
        if !api::test_support::game_data_available() {
            return;
        }
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
        if !api::test_support::game_data_available() {
            return;
        }
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
        assert_eq!(
            script.trace.entries.first().map(|entry| entry.hash),
            Some(trace_hash("quester cook: stage unknown → cook:0")),
            "a stage change starts a fresh trace table with its own line"
        );
        assert_eq!(
            script.trace.limit,
            stage_event_limit(script.path.sequences[script.seq_index].trace_units),
            "the stage's budget comes from its compiled steps and recipe steps"
        );
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
        if !api::test_support::game_data_available() {
            return;
        }
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
                max_attempts: None,
                stop: None,
                hazard: None,
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
            max_attempts: None,
            stop: None,
            hazard: None,
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

    fn held_items(script: &Quester, aliases: &[&str]) -> Vec<api::snapshot::ItemView> {
        aliases
            .iter()
            .zip(0..)
            .map(|(alias, slot)| api::snapshot::ItemView {
                def: api::obj_names::ItemDefView {
                    id: script.selected.item_by_alias(alias).unwrap().id,
                    name: Some((*alias).into()),
                    stackable: false,
                    members: false,
                    base_value: 0,
                    noted: false,
                    certificate_link: -1,
                    certificate_template: -1,
                },
                container: api::snapshot::ItemContainer::Inventory,
                action_family: api::snapshot::ItemActionFamily::Held,
                slot,
                count: 1,
                actions: Vec::new(),
                component_id: 0,
            })
            .collect()
    }

    /// A two-child pie recipe shaped like Knight's Sword's: `dispose` runs
    /// only on a held burnt pie, `cook` is skipped once the pie is held and
    /// settles on any outcome (no product), and the root acquire's goal is a
    /// held redberry pie.
    fn pie_retry_document(
        args: serde_json::Value,
        dispose_skip: super::super::path::PredicateDocument,
        cook_skip: super::super::path::PredicateDocument,
    ) -> super::super::path::PathDocument {
        use super::super::path::PredicateDocument;
        let mut document = super::super::compile::decode_cook().unwrap();
        document.quest.as_mut().unwrap().acquire.insert(
            "test:retry".to_owned(),
            vec![
                test_step(
                    "dispose",
                    "wait",
                    serde_json::json!({"until":{"All":[]},"max_ticks":1}),
                    dispose_skip,
                    PredicateDocument::All(vec![]),
                ),
                test_step(
                    "cook",
                    "wait",
                    serde_json::json!({"until":{"All":[]},"max_ticks":1}),
                    cook_skip,
                    PredicateDocument::All(vec![]),
                ),
            ],
        );
        document.roles[0].sequences[0].steps = vec![test_step(
            "root-acquire-step",
            "acquire",
            args,
            pie_has("redberry_pie"),
            pie_has("redberry_pie"),
        )];
        document
    }

    fn pie_has(obj: &str) -> super::super::path::PredicateDocument {
        super::super::path::PredicateDocument::Fact {
            kind: "has_item".to_owned(),
            version: 1,
            args: serde_json::json!({ "obj": obj }),
        }
    }

    /// A child completes on the poll that observes its effect, and a pass
    /// hands over to the next child within that tick (TICK-FIX #5). These
    /// tests act between ticks while a child runs, so `cook` waits for a pie
    /// (burnt or cooked) and `dispose` for the burnt pie to leave the pack,
    /// as the real children wait for their server effects.
    fn children_wait_for_their_effects(document: &mut super::super::path::PathDocument) {
        let children = document
            .quest
            .as_mut()
            .unwrap()
            .acquire
            .get_mut("test:retry")
            .unwrap();
        children[0].args = serde_json::json!({
            "until": {"Not": {"Fact": {"kind": "has_item", "version": 1, "args": {"obj": "burnt_pie"}}}},
            "max_ticks": 10
        });
        children[1].args = serde_json::json!({
            "until": {"Any": [
                {"Fact": {"kind": "has_item", "version": 1, "args": {"obj": "redberry_pie"}}},
                {"Fact": {"kind": "has_item", "version": 1, "args": {"obj": "burnt_pie"}}}
            ]},
            "max_ticks": 10
        });
    }

    fn current_child(script: &Quester) -> Option<String> {
        script
            .step
            .as_ref()
            .and_then(|run| run.child_step_id())
            .map(|id| id.0.to_string())
    }

    fn restart_lines(output: &TraceCapture) -> Vec<&str> {
        output
            .logs
            .iter()
            .map(|(_, line)| line.as_str())
            .filter(|line| line.contains("recipe test:retry restart "))
            .collect()
    }

    const ROOT_SETTLED: &str =
        "quester cook: stage cook:0 step root-acquire-step (acquire) settled";

    #[test]
    fn acquire_restarts_recipe_after_lost_product_then_succeeds() {
        if !api::test_support::game_data_available() {
            return;
        }
        use super::super::families::tests::with_tick_output;
        use super::super::path::PredicateDocument;

        let mut document = pie_retry_document(
            serde_json::json!({"recipe":"test:retry"}),
            PredicateDocument::Not(Box::new(pie_has("burnt_pie"))),
            pie_has("redberry_pie"),
        );
        children_wait_for_their_effects(&mut document);
        let (mut script, mut snapshot) = status_fixture(document);
        snapshot.seed_inventory(Vec::new(), 28);
        let mut ledger = None;
        let mut output = TraceCapture::default();
        let (mut burnt, mut emptied, mut cooked) = (false, false, false);
        for tick in 1..=80 {
            with_tick_output(&snapshot, &mut ledger, tick, &mut output, |native| {
                script.tick(native).unwrap();
            });
            assert_eq!(
                script.fail_streak, 0,
                "a lost product is not a step failure"
            );
            assert!(!script.parked);
            let restarts = restart_lines(&output).len();
            match (restarts, current_child(&script).as_deref()) {
                // The first bake burns: the cook child settles on a burnt pie.
                (0, Some("cook")) if !burnt => {
                    snapshot.seed_inventory(held_items(&script, &["burnt_pie"]), 28);
                    burnt = true;
                }
                (1, Some("dispose")) if !emptied => {
                    snapshot.seed_inventory(Vec::new(), 28);
                    emptied = true;
                }
                // The second bake cooks.
                (1, Some("cook")) if emptied && !cooked => {
                    snapshot.seed_inventory(held_items(&script, &["redberry_pie"]), 28);
                    cooked = true;
                }
                _ => {}
            }
            if output.logs.iter().any(|(_, line)| line == ROOT_SETTLED) {
                break;
            }
        }
        let messages: Vec<&str> = output.logs.iter().map(|(_, line)| line.as_str()).collect();
        assert!(
            burnt && emptied && cooked,
            "burn, dispose and re-cook all ran: {messages:#?}"
        );
        assert_eq!(
            restart_lines(&output),
            vec![
                "quester cook: stage cook:0 recipe test:retry restart 1/8 after child cook: has_item({\"obj\":\"redberry_pie\"}) is still false"
            ],
            "{messages:#?}"
        );
        assert!(messages.contains(&ROOT_SETTLED), "{messages:#?}");
        assert!(
            !messages
                .iter()
                .any(|line| line.contains("step settle timeout")),
            "{messages:#?}"
        );
    }

    #[test]
    fn acquire_restart_bound_parks_with_the_attempt_count() {
        if !api::test_support::game_data_available() {
            return;
        }
        use super::super::families::tests::with_tick_output;
        use super::super::path::PredicateDocument;

        let document = pie_retry_document(
            serde_json::json!({"recipe":"test:retry","max_restarts":2}),
            PredicateDocument::Not(Box::new(pie_has("burnt_pie"))),
            pie_has("redberry_pie"),
        );
        let (mut script, mut snapshot) = status_fixture(document);
        snapshot.seed_inventory(Vec::new(), 28);
        let mut ledger = None;
        let mut output = TraceCapture::default();
        for tick in 1..=80 {
            with_tick_output(&snapshot, &mut ledger, tick, &mut output, |native| {
                script.tick(native).unwrap();
            });
            if script.parked {
                break;
            }
        }
        let messages: Vec<&str> = output.logs.iter().map(|(_, line)| line.as_str()).collect();
        assert!(script.parked, "the exhausted bound parks: {messages:#?}");
        let reason = "test:retry gave up after 3 attempts: has_item({\"obj\":\"redberry_pie\"}) is still false";
        let status = output.statuses.last().expect("parked status was published");
        assert_eq!(status.phase, NativePhase::Blocked);
        assert_eq!(status_text(status, "step_id"), "root-acquire-step");
        assert_eq!(status_text(status, "last_failure"), reason);
        assert_eq!(status.failure.as_ref().unwrap().message.as_ref(), reason);
        assert_eq!(restart_lines(&output).len(), 2, "{messages:#?}");
        assert!(messages.iter().any(|line| line.contains("restart 2/2")));
        assert!(
            !messages
                .iter()
                .any(|line| line.contains("step settle timeout")),
            "the bound parks before any parent settle window: {messages:#?}"
        );
    }

    #[test]
    fn acquire_that_succeeds_first_time_does_not_restart() {
        if !api::test_support::game_data_available() {
            return;
        }
        use super::super::families::tests::with_tick_output;
        use super::super::path::PredicateDocument;

        let mut document = pie_retry_document(
            serde_json::json!({"recipe":"test:retry"}),
            PredicateDocument::Not(Box::new(pie_has("burnt_pie"))),
            pie_has("redberry_pie"),
        );
        children_wait_for_their_effects(&mut document);
        let (mut script, mut snapshot) = status_fixture(document);
        snapshot.seed_inventory(Vec::new(), 28);
        let mut ledger = None;
        let mut output = TraceCapture::default();
        let mut cooked = false;
        for tick in 1..=40 {
            with_tick_output(&snapshot, &mut ledger, tick, &mut output, |native| {
                script.tick(native).unwrap();
            });
            if !cooked && current_child(&script).as_deref() == Some("cook") {
                snapshot.seed_inventory(held_items(&script, &["redberry_pie"]), 28);
                cooked = true;
            }
            if output.logs.iter().any(|(_, line)| line == ROOT_SETTLED) {
                break;
            }
        }
        let messages: Vec<&str> = output.logs.iter().map(|(_, line)| line.as_str()).collect();
        assert!(cooked && messages.contains(&ROOT_SETTLED), "{messages:#?}");
        assert!(restart_lines(&output).is_empty(), "{messages:#?}");
        assert!(!script.parked);
        assert_eq!(script.fail_streak, 0);
        assert!(messages.contains(
            &"quester cook: stage cook:0 recipe test:retry child dispose skipped: not has_item({\"obj\":\"burnt_pie\"}) evaluated true"
        ));
    }

    /// A pass that began no child made no progress, so the goal miss goes to
    /// the parent settle window instead of looping on skips.
    #[test]
    fn acquire_pass_that_only_skips_does_not_restart() {
        if !api::test_support::game_data_available() {
            return;
        }
        use super::super::families::tests::with_tick_output;
        use super::super::path::PredicateDocument;

        let document = pie_retry_document(
            serde_json::json!({"recipe":"test:retry"}),
            PredicateDocument::Not(Box::new(pie_has("burnt_pie"))),
            PredicateDocument::Not(Box::new(pie_has("redberry_pie"))),
        );
        let (mut script, mut snapshot) = status_fixture(document);
        snapshot.seed_inventory(Vec::new(), 28);
        let mut ledger = None;
        let mut output = TraceCapture::default();
        for tick in 1..=60 {
            with_tick_output(&snapshot, &mut ledger, tick, &mut output, |native| {
                script.tick(native).unwrap();
            });
            if script.fail_streak > 0 {
                break;
            }
        }
        let messages: Vec<&str> = output.logs.iter().map(|(_, line)| line.as_str()).collect();
        assert_eq!(script.fail_streak, 1, "{messages:#?}");
        assert!(restart_lines(&output).is_empty(), "{messages:#?}");
        assert!(messages
            .iter()
            .any(|line| line
                .contains("step root-acquire-step (acquire) failed: step settle timeout")));
    }

    /// The goal miss restarts only while the acquire step would still be
    /// selected: once its `skip_if` holds, the runner's settle window decides.
    #[test]
    fn acquire_does_not_restart_once_its_skip_holds() {
        if !api::test_support::game_data_available() {
            return;
        }
        use super::super::families::tests::with_tick_output;
        use super::super::path::PredicateDocument;

        let mut document = pie_retry_document(
            serde_json::json!({"recipe":"test:retry"}),
            PredicateDocument::Not(Box::new(pie_has("burnt_pie"))),
            pie_has("redberry_pie"),
        );
        document.roles[0].sequences[0].steps[0].skip_if =
            PredicateDocument::Any(vec![pie_has("redberry_pie"), pie_has("burnt_pie")]);
        children_wait_for_their_effects(&mut document);
        let (mut script, mut snapshot) = status_fixture(document);
        snapshot.seed_inventory(Vec::new(), 28);
        let mut ledger = None;
        let mut output = TraceCapture::default();
        let mut burnt = false;
        for tick in 1..=60 {
            with_tick_output(&snapshot, &mut ledger, tick, &mut output, |native| {
                script.tick(native).unwrap();
            });
            if !burnt && current_child(&script).as_deref() == Some("cook") {
                snapshot.seed_inventory(held_items(&script, &["burnt_pie"]), 28);
                burnt = true;
            }
            if script.fail_streak > 0 {
                break;
            }
        }
        let messages: Vec<&str> = output.logs.iter().map(|(_, line)| line.as_str()).collect();
        assert!(burnt, "{messages:#?}");
        assert_eq!(script.fail_streak, 1, "{messages:#?}");
        assert!(restart_lines(&output).is_empty(), "{messages:#?}");
    }

    #[test]
    fn settle_timeout_park_keeps_timed_out_step_in_status() {
        if !api::test_support::game_data_available() {
            return;
        }
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
            max_attempts: None,
            stop: None,
            hazard: None,
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
        if !api::test_support::game_data_available() {
            return;
        }
        let (mut script, _) = fixture();
        let gates: Arc<[api::selected::QuestGate]> = Arc::from([api::selected::QuestGate::Window(
            api::selected::StageWindow {
                quest: FactKey::new("biohazard"),
                signal: FactKey::new("biohazard"),
                values: api::selected::InclusiveRange {
                    min: None,
                    max: Some(9),
                },
            },
        )]);
        script.record_failure(ActionError::NeedsEvidence(gates));
        assert_eq!(script.blocked_failure().code.as_ref(), "needs-evidence");
        let detail = script.blocked_failure().message;
        assert!(detail.contains("biohazard"), "{detail}");
        assert!(detail.contains("signal biohazard"), "{detail}");
        assert!(detail.contains("window"), "{detail}");
        assert!(detail.contains("minimum none"), "{detail}");
        assert!(detail.contains("maximum 9"), "{detail}");
        assert!(detail.contains("evidence state unknown"), "{detail}");
        script.clear_last_error();
        assert!(script.last_error.is_none());
    }

    #[test]
    fn combat_interrupted_talk_parks_without_advancing_or_reading() {
        if !api::test_support::game_data_available() {
            return;
        }
        use super::super::families::tests::{
            seed_dialogue_attacker, seed_dialogue_combat, with_tick, with_tick_output,
        };
        use api::snapshot::{GameSnapshot, QuestStatusView};
        #[derive(Default)]
        struct Capture(Vec<ScriptStatus>);
        impl NativeOutput for Capture {
            fn status(&mut self, status: ScriptStatus) {
                self.0.push(status);
            }
            fn paint(&mut self, _: Arc<crate::paint::ScriptPaint>) {}
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
        seed_dialogue_attacker(&mut snapshot, api::snapshot::ActorKind::Npc);
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
        if !api::test_support::game_data_available() {
            return;
        }
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
        if !api::test_support::game_data_available() {
            return;
        }
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
        let mut output = StatusCapture::default();
        for tick in 1..10 {
            super::super::families::tests::with_tick_output(
                &s,
                &mut ledger,
                tick,
                &mut output,
                |t| script.tick(t).unwrap(),
            );
        }
        assert!(script.parked);
        assert!(script.current_step().is_none());
        assert!(script.parked_step.is_none());
        assert_eq!(
            script.blocked_failure().message.as_ref(),
            "no step for stage"
        );
        let status = output.0.last().expect("parked status was published");
        assert_eq!(status_text(status, "step_id"), "");
        assert_eq!(status_text(status, "block_reason"), "no step for stage");
    }

    #[test]
    fn missing_spawn_status_names_the_wait_and_retains_the_park_cause() {
        if !api::test_support::game_data_available() {
            return;
        }
        use super::super::families::tests::{with_tick, with_tick_output};
        use api::snapshot::{GameSnapshot, QuestStatusView};
        #[derive(Default)]
        struct Capture(Vec<ScriptStatus>);
        impl NativeOutput for Capture {
            fn status(&mut self, status: ScriptStatus) {
                self.0.push(status);
            }
            fn paint(&mut self, _: Arc<crate::paint::ScriptPaint>) {}
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
    fn path_header_false_preserves_global_consent_on_the_real_walk_request() {
        if !api::test_support::game_data_available() {
            return;
        }
        use super::super::families::tests::with_tick;
        use crate::native::{HostEffect, WalkBit};
        use api::snapshot::{GameSnapshot, QuestStatusView};

        let data = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
        let quests = Arc::new(QuestCatalog::from_identity(data.quest_identity()).unwrap());
        let mut document = super::super::compile::decode_cook().unwrap();
        document.quest.as_mut().unwrap().owns_inventory = true;
        document.prereq.quest_start = Some(false);
        document.prereq.dialogue = Some(false);
        let step = &mut document.roles[0].sequences[0].steps[0];
        step.kind = "walk".into();
        step.args = serde_json::json!({
            "tile": [3103, 3163, 2], "source": "header consent fixture"
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
        let mut checked = false;
        for tick in 1..=8 {
            with_tick(&snapshot, &mut ledger, tick, |native| {
                assert_eq!(script.tick(native).unwrap(), ScriptFlow::Continue);
            });
            if let Some(request) = ledger.as_ref().and_then(|ledger| {
                ledger
                    .outbox
                    .iter()
                    .find_map(|action| match &action.effect {
                        HostEffect::Walk(request) => Some(request),
                        _ => None,
                    })
            }) {
                assert_eq!(request.options.allow_prereq_dialogue, WalkBit::Inherit);
                assert_eq!(request.options.allow_quest_start, WalkBit::Inherit);
                checked = true;
                break;
            }
        }
        assert!(
            checked,
            "the real Path runner must submit its narrowed walk"
        );
    }

    #[test]
    fn unprotectable_walk_publishes_its_cause_and_keeps_working() {
        if !api::test_support::game_data_available() {
            return;
        }
        use super::super::families::tests::{with_tick, with_tick_output};
        use crate::native::{HostEffect, WalkEvent, WalkEventKind};
        use api::snapshot::{GameSnapshot, QuestStatusView};
        #[derive(Default)]
        struct Capture(Vec<ScriptStatus>);
        impl NativeOutput for Capture {
            fn status(&mut self, status: ScriptStatus) {
                self.0.push(status);
            }
            fn paint(&mut self, _: Arc<crate::paint::ScriptPaint>) {}
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
        if !api::test_support::game_data_available() {
            return;
        }
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
        if !api::test_support::game_data_available() {
            return;
        }
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
                    &crate::local_state::OperatorLoadouts::absent(),
                    None,
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
        if !api::test_support::game_data_available() {
            return;
        }
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
        /// The account's bank memory, filled the way the host fills it.
        bank: api::bank_memory::BankMemory,
        bank_fixture: api::named_banks::NamedBank,
        bank_tile: api::snapshot::WorldTile,
        stock: Vec<api::snapshot::ItemView>,
        selected_bank: bool,
        opened_bank: bool,
    }

    impl BankSkipFixture {
        fn drive(&mut self, tick: u64, output: &mut dyn NativeOutput) -> ScriptFlow {
            // The host's per-frame observe (design-bank-snapshot §1.3).
            self.bank.track(&self.snapshot, tick);
            let flow = super::super::families::tests::with_tick_output_bank(
                &self.snapshot,
                Some(&self.bank),
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
                            crate::interact::InteractReq::OpenStand { .. }
                        )
                    )
                })
            });
            let close_pending = self.ledger.as_ref().is_some_and(|ledger| {
                ledger.outbox.first().is_some_and(|action| {
                    matches!(
                        &action.effect,
                        crate::native::HostEffect::Interaction(crate::interact::InteractReq::Close)
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
                        outcome: crate::bank::BankPickOutcome::Selected(
                            crate::bank::SelectedBank {
                                bank_index: 0,
                                access_tile: self.bank_tile,
                                kind: crate::bank::PickKind::Reachable,
                                access: Some(Arc::new(crate::bank::BankStandAccess {
                                    bank: self.bank_fixture,
                                    stand_tile: self.bank_tile,
                                    kind: crate::bank::AccessKind::Booth,
                                    stand_op: 1,
                                    name: None,
                                    choose: None,
                                })),
                            },
                        ),
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
            } else if close_pending {
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
                    .seed_bank_observation(-1, tick, None, Vec::new());
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
            bank: api::bank_memory::BankMemory::default(),
            bank_fixture: bank,
            bank_tile,
            stock,
            selected_bank: false,
            opened_bank: false,
        }
    }

    #[test]
    fn nested_bank_has_skip_starts_real_scan_and_receipt_selects_step() {
        if !api::test_support::game_data_available() {
            return;
        }
        for (bank_count, expected_step) in [
            (0, "skip-if-bank-has-logs"),
            (1, "fallback-if-bank-has-logs"),
        ] {
            let mut fixture = bank_skip_fixture(bank_count, false);
            let mut output = StatusCapture::default();
            assert!(!fixture.bank.known());

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
                if fixture.bank.known() && fixture.script.step.is_some() {
                    break;
                }
            }

            assert!(fixture.selected_bank, "the real scan selects a bank");
            assert!(
                fixture.opened_bank,
                "the real scan opens the selected stand"
            );
            assert!(
                fixture.bank.known(),
                "the scan opens the bank and the host observes it"
            );
            let logs_id = fixture.script.selected.item_by_alias("logs").unwrap().id;
            assert_eq!(fixture.bank.count(logs_id), Some(bank_count));
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
        if !api::test_support::game_data_available() {
            return;
        }
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
                if fixture.bank.known() && status_text(status, "child_step_id") == expected_step {
                    break;
                }
            }

            assert!(fixture.selected_bank, "the child scan selects a real bank");
            assert!(
                fixture.opened_bank,
                "the child scan opens the selected stand"
            );
            assert!(fixture.bank.known(), "the child scan observes the bank");
            let logs_id = fixture.script.selected.item_by_alias("logs").unwrap().id;
            assert_eq!(fixture.bank.count(logs_id), Some(bank_count));
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
        if !api::test_support::game_data_available() {
            return;
        }
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

#[cfg(test)]
#[path = "ordered_runner_tests.rs"]
mod ordered_tests;

#[cfg(test)]
#[path = "squire_bank_runner_tests.rs"]
mod squire_bank_tests;

#[cfg(test)]
#[path = "squire_portrait_runner_tests.rs"]
mod squire_portrait_tests;

#[cfg(test)]
#[path = "upkeep_runner_tests.rs"]
mod upkeep_tests;

#[cfg(test)]
#[path = "open_page_retry_runner_tests.rs"]
mod open_page_retry_tests;
