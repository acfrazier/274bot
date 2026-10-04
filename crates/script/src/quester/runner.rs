//! §3.2 tick loop with colour-first, boundary-triggered journal evidence.
use super::bank_memo::BankMemo;
use super::compile::{
    CompiledPath, CompiledStep, PredicateContext, StepContext, StepOutcome, StepRun,
};
use super::families::combat::CombatReceipt;
use super::progress::{quest_colour, resolve_colour, resolve_journal};
use super::provision::{ProvisionEvent, ProvisionMode, Provisioner};
use super::queue::QueueStatus;
use super::select::{select, sequence_for_stage, SelectionDecision};
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

use std::time::Duration;

// A read has at most three transactions (each can emit at most one row
// click). Adoption consumes a transaction too, but never adds a click.
const JOURNAL_READ_ATTEMPTS: u8 = 3;
const JOURNAL_RETRY_QUIET_TICKS: u64 = 3;
const QUEUE_QUEST_STATUS_WAIT: Duration = Duration::from_secs(30);
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
    retreat_completed: bool,
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
            retreat_completed: false,
            queue_fields: Arc::from([]),
            required_vs_live: Arc::from([]),
            tested_stats_warning: Arc::from(""),
        }
    }

    fn poll_provision(&mut self, tick: &mut NativeTick<'_>, mode: ProvisionMode) -> bool {
        let required_after = tick.cx.evidence();
        let not_started = self.stage.as_ref() == Some(&self.path.colour_not_started);
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
            mode,
            self.active_loadout.as_deref(),
            not_started,
        );
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
                self.dirty = true;
                false
            }
            Poll::Ready(Ok(ProvisionEvent::BankMemoUnknown)) => {
                self.published_bank_receipt = None;
                self.bank.clear();
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

    fn finish_quest(&mut self, tick: &mut NativeTick<'_>) -> ScriptFlow {
        if !self.poll_provision(tick, ProvisionMode::Retreat) {
            self.publish(tick.output);
            return ScriptFlow::Continue;
        }
        self.retreat_completed = self.provisioner.retreat_performed();
        self.published_bank_receipt = None;
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
    }

    fn set_last_error(&mut self, kind: QuesterFailureKind, message: Arc<str>) {
        self.last_error = Some(message);
        self.last_error_kind = kind;
        self.dirty = true;
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
        let current = self.current_step();
        let sequence = self.path.sequences.get(self.seq_index);
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
                | ProvisionPhase::Freshening
                | ProvisionPhase::Spillover
                | ProvisionPhase::Withdrawing
                | ProvisionPhase::Retreating
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
            (
                "step_id",
                "Step",
                current.map_or("", |step| step.id.0.as_ref()),
            ),
        ] {
            fields.push(StatusField {
                key,
                label,
                value: StatusValue::Text(Arc::from(text)),
            });
        }
        static EMPTY_CHILD: std::sync::LazyLock<Arc<str>> =
            std::sync::LazyLock::new(|| Arc::from(""));
        fields.push(StatusField {
            key: "child_step_id",
            label: "Acquisition child",
            value: StatusValue::Text(
                self.step
                    .as_ref()
                    .and_then(|run| run.child_step_id())
                    .map_or_else(|| Arc::clone(&EMPTY_CHILD), |id| Arc::clone(&id.0)),
            ),
        });
        for (key, label, value) in [
            ("sequence", "Sequence", self.seq_index as i64),
            (
                "sequence_count",
                "Sequence count",
                self.path.sequences.len() as i64,
            ),
            ("step_index", "Step index", self.step_index as i64),
            (
                "remaining_steps",
                "Remaining steps",
                sequence.map_or(0, |seq| seq.steps.len().saturating_sub(self.step_index)) as i64,
            ),
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
        self.pair_admitted = false;
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
                    if self.path.id.0.as_ref() == "blackarmgang" {
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

    fn adopt_progress(&mut self, tick: &mut NativeTick<'_>, progress: Arc<QuestProgress>, retarget: bool) -> bool {
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

    fn valid_progress(&self, tick: &NativeTick<'_>, progress: &QuestProgress, after: api::quest_progress::EvidenceStamp) -> bool {
        progress.quest == self.path.id && progress.binding == self.path.progress.binding
            && progress.role == self.path.role && progress.pin.as_ref() == tick.cx.pin()
            && progress.evidence.meets(after) && tick.cx.evidence().meets(progress.evidence)
            && progress.flags.iter().all(|flag| self.path.progress.flags.iter().any(|rule| rule.flag == flag.flag))
    }

    fn read_custom_stage(&mut self, tick: &mut NativeTick<'_>, retarget: bool) -> bool {
        if self.custom_reader.is_none() {
            let result = {
                let after = tick.cx.evidence();
                let mut cx = StepContext {
                    tick, quests: &self.quests, progress: self.progress_slice(),
                    required_after: after, bank: &self.bank, banks: &self.banks,
                };
                self.path.progress_reader.as_ref().unwrap().plan.begin(&mut cx)
            };
            match result {
                Ok(reader) => {
                    self.custom_reader = Some(reader);
                    self.custom_read_after = Some(tick.cx.evidence());
                }
                Err(ActionError::Busy | ActionError::Held | ActionError::BudgetExhausted) => return false,
                Err(error) => { self.record_failure(error); self.parked = true; return false; }
            }
        }
        let mut reader = self.custom_reader.take().unwrap();
        let result = {
            let after = self.custom_read_after.unwrap();
            let mut cx = StepContext {
                tick, quests: &self.quests, progress: self.progress_slice(),
                required_after: after, bank: &self.bank, banks: &self.banks,
            };
            reader.poll(&mut cx)
        };
        match result {
            Poll::Pending => { self.custom_reader = Some(reader); false }
            Poll::Ready(Err(error)) => { self.record_failure(error); self.parked = true; false }
            Poll::Ready(Ok(outcome)) => {
                let after = self.custom_read_after.take().unwrap();
                let Some(progress) = outcome.progress else {
                    self.record_failure(ActionError::Blocked(Arc::from("progress reader returned no owned progress")));
                    self.parked = true;
                    return false;
                };
                if outcome.evidence != progress.evidence || !self.valid_progress(tick, &progress, after) {
                    self.record_failure(ActionError::Stale);
                    self.parked = true;
                    return false;
                }
                self.adopt_progress(tick, progress, retarget)
            }
        }
    }

    fn admit_pair(&mut self, tick: &mut NativeTick<'_>) -> bool {
        if self.path.partner.is_none() || self.pair_admitted { return true; }
        if tick.pairs.is_some_and(|port| port.gang(tick.cx.run()).is_err()) {
            match self.gang_reader.poll(tick, &self.quests) {
                Poll::Pending => return false,
                Poll::Ready(Ok(_)) => {}
                Poll::Ready(Err(error)) => { self.record_failure(error); self.parked = true; return false; }
            }
        }
        if self.pair_admission.is_none() {
            let result = super::families::partner::admission(&self.path).and_then(|plan| {
                let after = tick.cx.evidence();
                let mut cx = StepContext {
                    tick, quests: &self.quests, progress: self.progress_slice(),
                    required_after: after, bank: &self.bank, banks: &self.banks,
                };
                plan.begin(&mut cx)
            });
            match result {
                Ok(run) => self.pair_admission = Some(run),
                Err(ActionError::Busy | ActionError::Held | ActionError::BudgetExhausted) => return false,
                Err(error) => { self.record_failure(error); self.parked = true; return false; }
            }
        }
        let mut run = self.pair_admission.take().unwrap();
        let result = {
            let after = tick.cx.evidence();
            let mut cx = StepContext {
                tick, quests: &self.quests, progress: self.progress_slice(),
                required_after: after, bank: &self.bank, banks: &self.banks,
            };
            run.poll(&mut cx)
        };
        match result {
            Poll::Pending => {
                self.pair_admission = Some(run);
                self.waiting = Some(("Partner admission", Arc::clone(&self.path.id.0)));
                self.dirty = true;
                false
            }
            Poll::Ready(Ok(_)) => {
                self.pair_admitted = true;
                self.waiting = None;
                self.dirty = true;
                true
            }
            Poll::Ready(Err(error)) => { self.record_failure(error); self.parked = true; false }
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
    fn tick(&mut self, tick: &mut NativeTick<'_>) -> Result<ScriptFlow, ScriptFailure> {
        if tick.cx.run() != self.run {
            self.run = tick.cx.run();
            self.watchdog = Watchdog::default();
            self.cancel_step(tick);
            self.last_combat = None;
            self.needs_read = true;
            self.progress = None;
        }
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
            self.retreat_completed = false;
            self.needs_read = true;
            self.progress = None;
            self.dirty = true;
            if exceeded {
                self.parked = true;
                self.set_last_error(
                    QuesterFailureKind::MaxDeaths,
                    Arc::from("maximum deaths exceeded; Stop/Start required"),
                );
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
            if !self.poll_provision(tick, ProvisionMode::Prepare) {
                self.publish(tick.output);
                return Ok(ScriptFlow::Continue);
            }
            let selected = {
                let pred = PredicateContext {
                    cx: &tick.cx,
                    quests: &self.quests,
                    progress: self.progress_slice(),
                    required_after: tick.cx.evidence(),
                    chat_since: super::families::reach::last_chat_seq(&tick.cx),
                    outcome: self.last_combat.as_ref().or(self.last_outcome.as_ref()),
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
                        self.clear_last_error();
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
            let step = if prelude {
                &self.path.prelude[index]
            } else {
                &self.path.sequences[self.seq_index].steps[index]
            };
            self.chat_since = super::families::reach::last_chat_seq(&tick.cx);
            let required_after = tick.cx.evidence();
            if step.kind.as_ref() == "combat" {
                self.last_combat = None;
            }
            let mut step_cx = StepContext {
                tick,
                quests: &self.quests,
                progress: self.progress_slice(),
                required_after,
                bank: &self.bank,
                banks: &self.banks,
                choices: &self.choices,
            };
            match step.plan.begin(&mut step_cx) {
                Ok(run) => {
                    self.step = Some(run);
                    self.last_outcome = None;
                    self.dirty = true;
                    self.clear_last_error();
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
                banks: &self.banks,
                choices: &self.choices,
            };
            self.step
                .as_mut()
                .map(|step| step.poll(&mut step_cx))
                .unwrap_or(Poll::Pending)
        };
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
                    if !self.valid_progress(tick, progress, outcome.evidence) {
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
                self.capture_prayer_cleanup();
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
                self.capture_prayer_cleanup();
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
        self.pair_admission = None;
        self.pair_admitted = false;
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
        self.watchdog = Watchdog::default();
        self.custom_reader = None;
        self.custom_read_after = None;
        self.pair_admission = None;
        self.pair_admitted = false;
        self.gang_reader.cancel();
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
        self.dirty = true;
        RandomClaim::Host
    }

    fn on_stop(&mut self, _reason: StopReason) {
        self.custom_reader = None;
        self.custom_read_after = None;
        self.pair_admission = None;
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

/// Queue ownership stays outside the active executor: only one Path is compiled
/// and retained at a time, and activation never decodes selected facts on-pump.
pub struct QueuedQuester {
    run: RunKey,
    selected: Arc<SelectedGameData>,
    quests: Arc<QuestCatalog>,
    banks: Arc<api::named_banks::NamedBankFacts>,
    choices: super::choices::QuestChoices,
    queue: super::queue::Queue<'static>,
    active: Option<Box<Quester>>,
    active_index: Option<usize>,
    preparing: Option<std::thread::JoinHandle<Result<Arc<CompiledPath>, Arc<str>>>>,
    gang_reader: super::gang::GangRead,
    pair_selection: Option<(usize, super::pair::Gang)>,
    completed: u16,
    deaths: u16,
    max_deaths: u8,
    anchor: Option<api::WorldTile>,
    retreats: u16,
    last_retreat: Option<Arc<str>>,
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
        queue: super::queue::Queue<'static>,
    ) -> Self {
        Self::new_with_max_deaths(run, selected, quests, banks, queue, default_max_deaths())
    }

    pub(super) fn new_with_max_deaths(
        run: RunKey,
        selected: Arc<SelectedGameData>,
        quests: Arc<QuestCatalog>,
        banks: Arc<api::named_banks::NamedBankFacts>,
        queue: super::queue::Queue<'static>,
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
            retreats: 0,
            last_retreat: None,
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
        self.retreats = retained.retreats;
        self.last_retreat = retained.last_retreat.clone();
        self.refresh_fields();
    }

    fn sync_retained(&self, tick: &mut NativeTick<'_>) {
        let retained = tick.cx.retained().quester();
        retained.anchor = self.anchor;
        retained.deaths = self.deaths;
        retained.completed = self.completed;
        retained.retreats = self.retreats;
        retained.last_retreat.clone_from(&self.last_retreat);
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
                key: "retreat_count",
                label: "Completed bank retreats",
                value: StatusValue::Integer(i64::from(self.retreats)),
            },
            StatusField {
                key: "session_deaths",
                label: "Session deaths",
                value: StatusValue::Integer(i64::from(self.deaths)),
            },
        ];
        if let Some(quest) = &self.last_retreat {
            fields.push(StatusField {
                key: "last_retreat",
                label: "Last bank retreat",
                value: StatusValue::Text(Arc::clone(quest)),
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
    fn prepare_pair_role(&mut self, index: usize, tick: &mut NativeTick<'_>) -> Poll<Result<super::pair::Gang, ActionError>> {
        if let Some((selected, gang)) = self.pair_selection {
            if selected == index { return Poll::Ready(Ok(gang)); }
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
                            return Poll::Ready(Err(ActionError::Blocked(Arc::from("choose an irreversible gang explicitly for this unjoined account"))));
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
                            if active.retreat_completed {
                                self.retreats = self.retreats.saturating_add(1);
                                self.last_retreat = Some(Arc::clone(&active.path.id.0));
                            }
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
            let paired = super::card::is_pair_path(self.queue.id(index).unwrap());
            let gang = if paired {
                match self.prepare_pair_role(index, tick) {
                    Poll::Pending => {
                        self.publish(tick.output, NativePhase::Waiting);
                        return Ok(ScriptFlow::Continue);
                    }
                    Poll::Ready(Ok(gang)) => Some(gang),
                    Poll::Ready(Err(error)) => {
                        self.queue.mark_blocked(index, Arc::from(format!("partner admission: {error:?}")));
                        self.refresh_fields();
                        return Ok(ScriptFlow::Continue);
                    }
                }
            } else { None };
            let id: Arc<str> = Arc::from(self.queue.id(index).expect("selected queue row"));
            let selected = Arc::clone(&self.selected);
            let quests = Arc::clone(&self.quests);
            self.active_index = Some(index);
            let worker = api::selected::FamilyPreparation::run(move |_| {
                let bytes = super::card::released_path(&id)
                    .ok_or_else(|| Arc::<str>::from("Path is not released"))?;
                super::compile::compile_path_for_gang(bytes, &selected, &quests, gang).map_err(|error| {
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
        let path = super::super::compile::compile_path(
            include_bytes!("../../paths/289/fixtures/combat_melee_food_only.json"),
            &data,
            &quests,
        )
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
