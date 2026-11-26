//! Named Rust predicate registry and evaluators for Section 7.6 inventory rows
//! and the non-registry `duel_round_trip` helper.
//!
//! Each callable predicate is addressed by exact `&str` name with structured
//! parameters (no caller closures). Lookup and dispatch fail closed on unknown
//! names, non-callable terminal summaries, arming steps used as gates, missing
//! table authority, or invalid parameters.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fmt;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

use api::selected::{ClientRevision, SkillMinimum};
use api::snapshot::{GameSnapshot, ItemView, WorldTile};

use crate::proof::Proof;
use crate::scenarios::pair::{ClueFleetWitness, DuelRoundTrip, KqFleetWitness};
use crate::scenarios::production::{GnomeRadiusWitness, GNOME_RADIUS_WITNESS_EVIDENCE};

/// Canonical evidence string for the FlaxRunner companion barrier (row 1).
pub const EVIDENCE_FLAX_COMPANIONS_PREPARED: &str =
    "flax_spinner_relogged_and_prepared_before_start";
/// Canonical evidence string for the ClueDuel helper barrier (row 1).
pub const EVIDENCE_CLUE_COMPANIONS_PREPARED: &str = "clue_helper_prepared_before_start";
/// Canonical evidence string for the JiveKQ companions barrier (row 1).
pub const EVIDENCE_KQ_COMPANIONS_PREPARED: &str = "kq_companions_prepared_before_start";

/// Canonical evidence string for `quester_start_exact` (row 2).
pub const EVIDENCE_QUESTER_START_EXACT: &str =
    "exact qualification stats, carried items, worn kit and stand";
/// Canonical evidence string for `clue_duel_identities` (row 3).
pub const EVIDENCE_CLUE_DUEL_IDENTITIES: &str = "clue_duel_two_distinct_visible_accounts";
/// Canonical evidence string for `clue_duel_handshake` (row 4).
pub const EVIDENCE_CLUE_DUEL_HANDSHAKE: &str = "clue_duel_both_offer_confirm_rules_1024_pen";
/// Canonical evidence string for `clue_duel_trail` (row 5).
pub const EVIDENCE_CLUE_DUEL_TRAIL: &str = "clue_3554_casket_forfeit_helper_reset_reward";
/// Canonical evidence string for `clue_duel_reward_banked` (row 6).
pub const EVIDENCE_CLUE_DUEL_REWARD_BANKED: &str = "clue_reward_bank_delta_and_both_hp_unchanged";
/// Canonical evidence string for `bank_closed` (row 7).
pub const EVIDENCE_BANK_CLOSED: &str = "bank_closed";
/// Canonical evidence string for `clue_duel_status_settled` (row 8).
pub const EVIDENCE_CLUE_DUEL_STATUS_SETTLED: &str = "clue_solver_next_status_after_close";
/// Default settle window for `clue_duel_status_settled` (row 8).
pub const DEFAULT_CLUE_DUEL_STATUS_TICKS: u32 = 3;

/// Canonical evidence string for `kq_identities` (row 10).
pub const EVIDENCE_KQ_IDENTITIES: &str = "kq_four_distinct_account_sessions";
/// Canonical evidence string for `kq_first_descent` (row 12).
pub const EVIDENCE_KQ_FIRST_DESCENT: &str = "kq_pass_paths_packs_peer_hold_two_ropes_first_descent";
/// Canonical evidence string for `kq_first_combat` (row 13).
pub const EVIDENCE_KQ_FIRST_COMBAT: &str = "kq_both_forms_four_xp_food_resume_cardinal_formation";
/// Canonical evidence string for `kq_first_kill_banked` (row 14).
pub const EVIDENCE_KQ_FIRST_KILL_BANKED: &str = "kq_kill_first_fresh_bank_return";
/// Canonical evidence string for `kq_second_descent` (row 15).
pub const EVIDENCE_KQ_SECOND_DESCENT: &str = "kq_four_fresh_banks_second_descent_respawn_xp";
/// Canonical evidence string for `kq_safe_complete` (row 16).
pub const EVIDENCE_KQ_SAFE_COMPLETE: &str =
    "kq_escape_restock_second_kill_four_survivors_safely_banked";
/// Canonical evidence string for `kq_finish_ready` (row 17).
pub const EVIDENCE_KQ_FINISH_READY: &str = "kq_safe_finish_fleet_stopped_companion_banks_closed";

/// Canonical evidence string for `gnome_radius_respected` (row 19).
pub const EVIDENCE_GNOME_RADIUS_RESPECTED: &str = GNOME_RADIUS_WITNESS_EVIDENCE;
/// Canonical evidence string for `quest_seed_exact` (row 20).
pub const EVIDENCE_QUEST_SEED_EXACT: &str =
    "exact quest-watch seed: stats, inventory, worn kit, stand, quest colours and quest points";
/// Canonical evidence string for `standing_bank_closed` (row 21).
pub const EVIDENCE_STANDING_BANK_CLOSED: &str =
    "quest-watch seed start tile and closed bank before logout";
/// Canonical evidence string for `bank_exact` (row 22).
pub const EVIDENCE_BANK_EXACT: &str = "exact quest-watch seed bank contents";
/// Canonical evidence string for `world_speed_changed` (row 23).
pub const EVIDENCE_WORLD_SPEED_CHANGED: &str = "fresh exact system-chat world speed confirmation";
/// Canonical evidence string for `revision_landing_idle` (row 24).
pub const EVIDENCE_REVISION_LANDING_IDLE: &str = "revision_landing_idle";
/// Canonical evidence string for the extra `duel_round_trip` helper.
pub const EVIDENCE_DUEL_ROUND_TRIP: &str = "duel_arena_pen_entered_and_lobby_returned";

/// Quest-list colour of a completed quest (`ctx::quest_status`).
pub const QUEST_COLOUR_COMPLETE: i32 = 0xF800;
/// Quest-list colour of a quest that has not started.
pub const QUEST_COLOUR_NOT_STARTED: i32 = 0xF80000;
/// Hitpoints skill index (`Skill::names[3] == "Hitpoints"`).
pub const HITPOINTS_SKILL_INDEX: u8 = 3;
/// Minimum legal world tick speed in milliseconds for `world_speed_changed`.
pub const MIN_WORLD_SPEED_MS: u32 = 50;
/// Maximum legal world tick speed in milliseconds for `world_speed_changed`.
pub const MAX_WORLD_SPEED_MS: u32 = 5000;

/// Semantic kind of an entry in the named-predicate registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NamedPredicateKind {
    /// Standard callable pass/fail gate (admitted under Script or Table authority).
    Gate,
    /// Callable pass/fail alias over an existing `Proof` variant (`bank_closed`).
    ProofAlias,
    /// Named arming step (`kq_armed`): executes an arm transition, not a pass/fail gate.
    ArmingStep,
    /// Table-only pass/fail gate (`world_speed_changed`): refused without Table authority.
    TableOnlyGate,
    /// Rust `terminal_summary` hook (Clue/KQ family summaries): never script-selectable.
    TerminalSummary,
}

/// Caller authority when binding or evaluating a named predicate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PredicateAuthority {
    /// Script-selectable context (refuses table-only predicates and terminal summaries).
    Script,
    /// Rust table-scenario context (admits table-only predicates; still refuses summaries).
    Table,
}

/// Static registry entry for one Section 7.6 inventory row (or `duel_round_trip`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NamedPredicateDescriptor {
    /// Exact canonical lookup name.
    pub name: &'static str,
    /// Section 7.6 inventory row (`None` for the non-registry `duel_round_trip` helper).
    pub inventory_row: Option<u8>,
    /// Semantic kind governing lookup and call admission.
    pub kind: NamedPredicateKind,
    /// Canonical evidence string when not supplied per call.
    pub default_evidence: Option<&'static str>,
}

/// Complete static registry covering all 24 Section 7.6 inventory rows plus `duel_round_trip`.
pub const NAMED_PREDICATES: &[NamedPredicateDescriptor] = &[
    NamedPredicateDescriptor {
        name: "companions_prepared",
        inventory_row: Some(1),
        kind: NamedPredicateKind::Gate,
        default_evidence: None,
    },
    NamedPredicateDescriptor {
        name: "quester_start_exact",
        inventory_row: Some(2),
        kind: NamedPredicateKind::Gate,
        default_evidence: Some(EVIDENCE_QUESTER_START_EXACT),
    },
    NamedPredicateDescriptor {
        name: "clue_duel_identities",
        inventory_row: Some(3),
        kind: NamedPredicateKind::Gate,
        default_evidence: Some(EVIDENCE_CLUE_DUEL_IDENTITIES),
    },
    NamedPredicateDescriptor {
        name: "clue_duel_handshake",
        inventory_row: Some(4),
        kind: NamedPredicateKind::Gate,
        default_evidence: Some(EVIDENCE_CLUE_DUEL_HANDSHAKE),
    },
    NamedPredicateDescriptor {
        name: "clue_duel_trail",
        inventory_row: Some(5),
        kind: NamedPredicateKind::Gate,
        default_evidence: Some(EVIDENCE_CLUE_DUEL_TRAIL),
    },
    NamedPredicateDescriptor {
        name: "clue_duel_reward_banked",
        inventory_row: Some(6),
        kind: NamedPredicateKind::Gate,
        default_evidence: Some(EVIDENCE_CLUE_DUEL_REWARD_BANKED),
    },
    NamedPredicateDescriptor {
        name: "bank_closed",
        inventory_row: Some(7),
        kind: NamedPredicateKind::ProofAlias,
        default_evidence: Some(EVIDENCE_BANK_CLOSED),
    },
    NamedPredicateDescriptor {
        name: "clue_duel_status_settled",
        inventory_row: Some(8),
        kind: NamedPredicateKind::Gate,
        default_evidence: Some(EVIDENCE_CLUE_DUEL_STATUS_SETTLED),
    },
    NamedPredicateDescriptor {
        name: "clue_family_summary",
        inventory_row: Some(9),
        kind: NamedPredicateKind::TerminalSummary,
        default_evidence: None,
    },
    NamedPredicateDescriptor {
        name: "kq_identities",
        inventory_row: Some(10),
        kind: NamedPredicateKind::Gate,
        default_evidence: Some(EVIDENCE_KQ_IDENTITIES),
    },
    NamedPredicateDescriptor {
        name: "kq_armed",
        inventory_row: Some(11),
        kind: NamedPredicateKind::ArmingStep,
        default_evidence: None,
    },
    NamedPredicateDescriptor {
        name: "kq_first_descent",
        inventory_row: Some(12),
        kind: NamedPredicateKind::Gate,
        default_evidence: Some(EVIDENCE_KQ_FIRST_DESCENT),
    },
    NamedPredicateDescriptor {
        name: "kq_first_combat",
        inventory_row: Some(13),
        kind: NamedPredicateKind::Gate,
        default_evidence: Some(EVIDENCE_KQ_FIRST_COMBAT),
    },
    NamedPredicateDescriptor {
        name: "kq_first_kill_banked",
        inventory_row: Some(14),
        kind: NamedPredicateKind::Gate,
        default_evidence: Some(EVIDENCE_KQ_FIRST_KILL_BANKED),
    },
    NamedPredicateDescriptor {
        name: "kq_second_descent",
        inventory_row: Some(15),
        kind: NamedPredicateKind::Gate,
        default_evidence: Some(EVIDENCE_KQ_SECOND_DESCENT),
    },
    NamedPredicateDescriptor {
        name: "kq_safe_complete",
        inventory_row: Some(16),
        kind: NamedPredicateKind::Gate,
        default_evidence: Some(EVIDENCE_KQ_SAFE_COMPLETE),
    },
    NamedPredicateDescriptor {
        name: "kq_finish_ready",
        inventory_row: Some(17),
        kind: NamedPredicateKind::Gate,
        default_evidence: Some(EVIDENCE_KQ_FINISH_READY),
    },
    NamedPredicateDescriptor {
        name: "kq_family_summary",
        inventory_row: Some(18),
        kind: NamedPredicateKind::TerminalSummary,
        default_evidence: None,
    },
    NamedPredicateDescriptor {
        name: "gnome_radius_respected",
        inventory_row: Some(19),
        kind: NamedPredicateKind::Gate,
        default_evidence: Some(EVIDENCE_GNOME_RADIUS_RESPECTED),
    },
    NamedPredicateDescriptor {
        name: "quest_seed_exact",
        inventory_row: Some(20),
        kind: NamedPredicateKind::Gate,
        default_evidence: Some(EVIDENCE_QUEST_SEED_EXACT),
    },
    NamedPredicateDescriptor {
        name: "standing_bank_closed",
        inventory_row: Some(21),
        kind: NamedPredicateKind::Gate,
        default_evidence: Some(EVIDENCE_STANDING_BANK_CLOSED),
    },
    NamedPredicateDescriptor {
        name: "bank_exact",
        inventory_row: Some(22),
        kind: NamedPredicateKind::Gate,
        default_evidence: Some(EVIDENCE_BANK_EXACT),
    },
    NamedPredicateDescriptor {
        name: "world_speed_changed",
        inventory_row: Some(23),
        kind: NamedPredicateKind::TableOnlyGate,
        default_evidence: Some(EVIDENCE_WORLD_SPEED_CHANGED),
    },
    NamedPredicateDescriptor {
        name: "revision_landing_idle",
        inventory_row: Some(24),
        kind: NamedPredicateKind::Gate,
        default_evidence: Some(EVIDENCE_REVISION_LANDING_IDLE),
    },
    NamedPredicateDescriptor {
        name: "duel_round_trip",
        inventory_row: None,
        kind: NamedPredicateKind::Gate,
        default_evidence: Some(EVIDENCE_DUEL_ROUND_TRIP),
    },
];

/// Fail-closed error returned when looking up, binding, or executing a named predicate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NamedPredicateError {
    /// Exact name does not match any registered predicate.
    UnknownPredicate { name: String },
    /// Clue or KQ family summary is a Rust `terminal_summary` hook, not a callable predicate.
    NonCallableSummary {
        name: &'static str,
        inventory_row: u8,
    },
    /// Named arming step (`kq_armed`) was requested as a pass/fail gate.
    ArmingStepNotAGate { name: &'static str },
    /// Non-arming registry entry was passed to the arming-step binder.
    NotAnArmingStep {
        name: &'static str,
        kind: NamedPredicateKind,
    },
    /// Table-only gate (`world_speed_changed`) was requested without Table authority.
    TableOnlyAuthorityRequired { name: &'static str },
    /// Missing, wrong-variant, or invalid structured parameters.
    InvalidParams { name: &'static str, reason: String },
}

impl fmt::Display for NamedPredicateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownPredicate { name } => write!(f, "unknown named predicate {name:?}"),
            Self::NonCallableSummary {
                name,
                inventory_row,
            } => write!(
                f,
                "predicate {name:?} (row {inventory_row}) is a Rust terminal_summary hook and is not script-selectable"
            ),
            Self::ArmingStepNotAGate { name } => write!(
                f,
                "predicate {name:?} is a named arming step, not a pass/fail gate"
            ),
            Self::NotAnArmingStep { name, kind } => write!(
                f,
                "predicate {name:?} has kind {kind:?} and is not an arming step"
            ),
            Self::TableOnlyAuthorityRequired { name } => write!(
                f,
                "predicate {name:?} is table-only and requires PredicateAuthority::Table"
            ),
            Self::InvalidParams { name, reason } => {
                write!(f, "invalid parameters for {name:?}: {reason}")
            }
        }
    }
}

impl std::error::Error for NamedPredicateError {}

/// Pass/fail result of evaluating a bound named predicate gate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PredicateVerdict {
    Pass,
    Fail { reason: String },
}

impl PredicateVerdict {
    pub fn is_pass(&self) -> bool {
        matches!(self, Self::Pass)
    }

    pub fn failure_reason(&self) -> Option<&str> {
        match self {
            Self::Pass => None,
            Self::Fail { reason } => Some(reason.as_str()),
        }
    }
}

/// Snapshot and host metadata supplied to a named predicate evaluation.
#[derive(Clone, Copy)]
pub struct PredicateContext<'a> {
    pub snapshot: &'a GameSnapshot,
    /// Actual connected client revision. Required by `revision_landing_idle`;
    /// `None` fails closed for that predicate.
    pub actual_revision: Option<ClientRevision>,
}

impl<'a> PredicateContext<'a> {
    pub fn snapshot_only(snapshot: &'a GameSnapshot) -> Self {
        Self {
            snapshot,
            actual_revision: None,
        }
    }

    pub fn with_revision(snapshot: &'a GameSnapshot, actual_revision: ClientRevision) -> Self {
        Self {
            snapshot,
            actual_revision: Some(actual_revision),
        }
    }
}

/// Shared companion-readiness barrier backing `companions_prepared` (row 1).
#[derive(Clone, Debug)]
pub struct CompanionPrepBarrier {
    ready: Arc<AtomicU8>,
    expected: u8,
}

impl CompanionPrepBarrier {
    /// Construct a barrier for `1..=7` companion slots.
    pub fn try_new(companions: usize) -> Result<Self, NamedPredicateError> {
        if !(1..=7).contains(&companions) {
            return Err(NamedPredicateError::InvalidParams {
                name: "companions_prepared",
                reason: format!("companion count {companions} is outside 1..=7"),
            });
        }
        Ok(Self {
            ready: Arc::new(AtomicU8::new(0)),
            expected: ((1_u16 << (companions + 1)) - 2) as u8,
        })
    }

    pub(crate) fn new(companions: usize) -> Self {
        Self::try_new(companions).expect("companion count must be within 1..=7")
    }

    /// Record companion slot `profile` (`1..=7`) as prepared.
    pub fn mark_ready(&self, profile: usize) {
        if (1..=7).contains(&profile) {
            self.ready.fetch_or(1 << profile, Ordering::Release);
        }
    }

    /// True once every expected companion slot has reported ready.
    pub fn all_ready(&self) -> bool {
        self.ready.load(Ordering::Acquire) & self.expected == self.expected
    }

    fn state_masks(&self) -> (u8, u8) {
        (self.ready.load(Ordering::Acquire), self.expected)
    }
}

/// Parameters for `companions_prepared` (row 1).
#[derive(Debug, Clone)]
pub struct CompanionsPreparedParams {
    /// Explicit evidence string (preserves the three distinct fleet barrier labels).
    pub evidence: Cow<'static, str>,
    pub barrier: CompanionPrepBarrier,
}

/// Parameters for `quester_start_exact` (row 2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuesterStartExactParams {
    pub stand: WorldTile,
    pub stats: Vec<SkillMinimum>,
    pub inventory: Vec<(i32, i32)>,
    pub equipment: Vec<i32>,
    pub hitpoints_may_be_hurt: bool,
}

/// Shared handle to a `ClueFleetWitness` (rows 3–6, 8, and row 9 summary).
#[derive(Clone, Default)]
pub struct ClueFleetWitnessHandle {
    pub(crate) inner: Arc<Mutex<ClueFleetWitness>>,
}

impl fmt::Debug for ClueFleetWitnessHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClueFleetWitnessHandle").finish()
    }
}

impl ClueFleetWitnessHandle {
    pub fn new() -> Self {
        Self::default()
    }

    pub(crate) fn from_arc(inner: Arc<Mutex<ClueFleetWitness>>) -> Self {
        Self { inner }
    }

    pub fn start(&self) {
        self.inner.lock().expect("clue witness mutex").start();
    }

    pub fn observe(&self, slot_index: usize, snapshot: &GameSnapshot) {
        self.inner
            .lock()
            .expect("clue witness mutex")
            .observe(slot_index, snapshot);
    }

    pub fn terminal_summary(&self) -> String {
        self.inner
            .lock()
            .expect("clue witness mutex")
            .terminal_summary()
    }
}

/// Parameters for `clue_duel_status_settled` (row 8).
#[derive(Debug, Clone)]
pub struct ClueDuelStatusSettledParams {
    pub witness: ClueFleetWitnessHandle,
    /// Observed ticks to wait after close (`DEFAULT_CLUE_DUEL_STATUS_TICKS` = 3).
    pub ticks: u32,
}

impl ClueDuelStatusSettledParams {
    pub fn with_default_ticks(witness: ClueFleetWitnessHandle) -> Self {
        Self {
            witness,
            ticks: DEFAULT_CLUE_DUEL_STATUS_TICKS,
        }
    }
}

/// Shared handle to a `KqFleetWitness` (rows 10–17, and row 18 summary).
#[derive(Clone, Default)]
pub struct KqFleetWitnessHandle {
    pub(crate) inner: Arc<Mutex<KqFleetWitness>>,
}

impl fmt::Debug for KqFleetWitnessHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KqFleetWitnessHandle").finish()
    }
}

impl KqFleetWitnessHandle {
    pub fn new() -> Self {
        Self::default()
    }

    pub(crate) fn from_arc(inner: Arc<Mutex<KqFleetWitness>>) -> Self {
        Self { inner }
    }

    pub fn start(&self) {
        self.inner.lock().expect("KQ witness mutex").start();
    }

    pub fn is_started(&self) -> bool {
        self.inner.lock().expect("KQ witness mutex").is_started()
    }

    pub fn observe(&self, slot_index: usize, snapshot: &GameSnapshot) {
        self.inner
            .lock()
            .expect("KQ witness mutex")
            .observe(slot_index, snapshot);
    }

    pub fn begin_finish(&self) {
        self.inner.lock().expect("KQ witness mutex").begin_finish();
    }

    pub fn finish_started(&self) -> bool {
        self.inner
            .lock()
            .expect("KQ witness mutex")
            .finish_started()
    }

    pub fn terminal_summary(&self) -> String {
        self.inner
            .lock()
            .expect("KQ witness mutex")
            .terminal_summary()
    }
}

/// Shared handle to a `GnomeRadiusWitness` (row 19).
#[derive(Clone, Default)]
pub struct GnomeRadiusWitnessHandle {
    pub(crate) inner: Arc<Mutex<GnomeRadiusWitness>>,
}

impl fmt::Debug for GnomeRadiusWitnessHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GnomeRadiusWitnessHandle").finish()
    }
}

impl GnomeRadiusWitnessHandle {
    pub fn new() -> Self {
        Self::default()
    }
}

/// One expected skill row for `quest_seed_exact` (row 20).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuestSeedStat {
    pub index: i32,
    pub level: i32,
    /// Missing stats fail only when `listed == true`.
    pub listed: bool,
}

/// Explicit plan parameters for `quest_seed_exact` (row 20).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuestSeedExactParams {
    pub stand: WorldTile,
    pub stats: Vec<QuestSeedStat>,
    /// Skill index treated with Hitpoints effective-level rules (`> 0 && <= target`).
    pub hitpoints_index: Option<i32>,
    /// Exact expected inventory totals by item id (positive counts only).
    pub inventory: BTreeMap<i32, i32>,
    /// Exact expected worn equipment item ids.
    pub equipment: Vec<i32>,
    /// Quest-list display names expected at `QUEST_COLOUR_COMPLETE`.
    pub completed_quests: Vec<String>,
    /// Target quest-list display name expected at `QUEST_COLOUR_NOT_STARTED`.
    pub current_quest: String,
    /// Expected quest points value (checked only when `quest_points_varp` is `Some`).
    pub quest_points: i32,
    /// Optional quest-points varp index; absent skips QP check.
    pub quest_points_varp: Option<i32>,
}

/// Parameters for `standing_bank_closed` (row 21).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StandingBankClosedParams {
    pub stand: WorldTile,
}

/// Parameters for `bank_exact` (row 22).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BankExactParams {
    pub expected: BTreeMap<i32, i32>,
}

/// Shared pre-send chat-sequence baseline for `world_speed_changed` (row 23).
/// Fails closed until a baseline is explicitly captured or provided.
#[derive(Debug, Clone, Default)]
pub struct WorldSpeedBaseline {
    baseline: Arc<Mutex<Option<i32>>>,
}

impl WorldSpeedBaseline {
    /// Start with no baseline captured (fails closed until `capture` runs).
    pub fn unset() -> Self {
        Self::default()
    }

    /// Construct with an explicit pre-send chat-sequence baseline.
    pub fn fixed(baseline: i32) -> Self {
        Self {
            baseline: Arc::new(Mutex::new(Some(baseline))),
        }
    }

    /// Capture the newest chat-line sequence from `snapshot` before sending `::speed`.
    pub fn capture(&self, snapshot: &GameSnapshot) -> i32 {
        let seq = crate::quest_fast::chat_baseline(snapshot);
        *self.baseline.lock().expect("world speed baseline mutex") = Some(seq);
        seq
    }

    pub fn get(&self) -> Option<i32> {
        *self.baseline.lock().expect("world speed baseline mutex")
    }
}

/// Parameters for table-only `world_speed_changed` (row 23).
#[derive(Debug, Clone)]
pub struct WorldSpeedChangedParams {
    pub expected_ms: u32,
    pub baseline: WorldSpeedBaseline,
}

/// Parameters for `revision_landing_idle` (row 24).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionLandingIdleParams {
    pub expected_revision: ClientRevision,
    pub accepted_tiles: Vec<WorldTile>,
}

/// Shared handle to a `DuelRoundTrip` witness (extra non-registry helper).
#[derive(Clone, Default)]
pub struct DuelRoundTripHandle {
    pub(crate) inner: Arc<Mutex<DuelRoundTrip>>,
}

impl fmt::Debug for DuelRoundTripHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DuelRoundTripHandle").finish()
    }
}

impl DuelRoundTripHandle {
    pub fn new() -> Self {
        Self::default()
    }
}

/// Structured parameters supplied when calling a named predicate or arming step.
#[derive(Debug, Clone)]
pub enum NamedPredicateParams {
    None,
    CompanionsPrepared(CompanionsPreparedParams),
    QuesterStartExact(QuesterStartExactParams),
    ClueWitness(ClueFleetWitnessHandle),
    ClueDuelStatusSettled(ClueDuelStatusSettledParams),
    KqWitness(KqFleetWitnessHandle),
    GnomeRadius(GnomeRadiusWitnessHandle),
    QuestSeedExact(QuestSeedExactParams),
    StandingBankClosed(StandingBankClosedParams),
    BankExact(BankExactParams),
    WorldSpeedChanged(WorldSpeedChangedParams),
    RevisionLandingIdle(RevisionLandingIdleParams),
    DuelRoundTrip(DuelRoundTripHandle),
}

/// Return all 25 static descriptors (24 inventory rows + `duel_round_trip`).
pub fn all_descriptors() -> &'static [NamedPredicateDescriptor] {
    NAMED_PREDICATES
}

/// Look up a descriptor by exact canonical name. Fails closed on unknown names.
pub fn lookup(name: &str) -> Result<NamedPredicateDescriptor, NamedPredicateError> {
    NAMED_PREDICATES
        .iter()
        .copied()
        .find(|descriptor| descriptor.name == name)
        .ok_or_else(|| NamedPredicateError::UnknownPredicate {
            name: name.to_owned(),
        })
}

#[derive(Debug, Clone)]
enum BoundGateKind {
    CompanionsPrepared(CompanionsPreparedParams),
    QuesterStartExact(QuesterStartExactParams),
    ClueDuelIdentities(ClueFleetWitnessHandle),
    ClueDuelHandshake(ClueFleetWitnessHandle),
    ClueDuelTrail(ClueFleetWitnessHandle),
    ClueDuelRewardBanked(ClueFleetWitnessHandle),
    BankClosed,
    ClueDuelStatusSettled(ClueDuelStatusSettledParams),
    KqIdentities(KqFleetWitnessHandle),
    KqFirstDescent(KqFleetWitnessHandle),
    KqFirstCombat(KqFleetWitnessHandle),
    KqFirstKillBanked(KqFleetWitnessHandle),
    KqSecondDescent(KqFleetWitnessHandle),
    KqSafeComplete(KqFleetWitnessHandle),
    KqFinishReady(KqFleetWitnessHandle),
    GnomeRadiusRespected(GnomeRadiusWitnessHandle),
    QuestSeedExact(QuestSeedExactParams),
    StandingBankClosed(StandingBankClosedParams),
    BankExact(BankExactParams),
    WorldSpeedChanged(WorldSpeedChangedParams),
    RevisionLandingIdle(RevisionLandingIdleParams),
    DuelRoundTrip(DuelRoundTripHandle),
}

/// A validated, bound callable named predicate gate.
#[derive(Debug, Clone)]
pub struct NamedPredicateGate {
    descriptor: NamedPredicateDescriptor,
    evidence: Cow<'static, str>,
    kind: BoundGateKind,
}

impl NamedPredicateGate {
    /// Resolve `name`, enforce authority and kind rules, and validate `params`.
    pub fn bind(
        name: &str,
        params: NamedPredicateParams,
        authority: PredicateAuthority,
    ) -> Result<Self, NamedPredicateError> {
        let descriptor = lookup(name)?;
        match descriptor.kind {
            NamedPredicateKind::TerminalSummary => {
                return Err(NamedPredicateError::NonCallableSummary {
                    name: descriptor.name,
                    inventory_row: descriptor.inventory_row.unwrap_or(0),
                });
            }
            NamedPredicateKind::ArmingStep => {
                return Err(NamedPredicateError::ArmingStepNotAGate {
                    name: descriptor.name,
                });
            }
            NamedPredicateKind::TableOnlyGate if authority != PredicateAuthority::Table => {
                return Err(NamedPredicateError::TableOnlyAuthorityRequired {
                    name: descriptor.name,
                });
            }
            NamedPredicateKind::Gate
            | NamedPredicateKind::ProofAlias
            | NamedPredicateKind::TableOnlyGate => {}
        }

        let invalid = |reason: String| NamedPredicateError::InvalidParams {
            name: descriptor.name,
            reason,
        };

        let (evidence, kind) = match (descriptor.name, params) {
            ("companions_prepared", NamedPredicateParams::CompanionsPrepared(p)) => {
                if p.evidence.trim().is_empty() {
                    return Err(invalid("evidence label must be non-empty".into()));
                }
                let evidence = p.evidence.clone();
                (evidence, BoundGateKind::CompanionsPrepared(p))
            }
            ("quester_start_exact", NamedPredicateParams::QuesterStartExact(p)) => {
                for (id, count) in &p.inventory {
                    if *id < 0 || *count < 0 {
                        return Err(invalid(format!(
                            "inventory entry ({id}, {count}) must be non-negative"
                        )));
                    }
                }
                for id in &p.equipment {
                    if *id < 0 {
                        return Err(invalid(format!("equipment id {id} must be non-negative")));
                    }
                }
                (
                    Cow::Borrowed(EVIDENCE_QUESTER_START_EXACT),
                    BoundGateKind::QuesterStartExact(p),
                )
            }
            ("clue_duel_identities", NamedPredicateParams::ClueWitness(w)) => (
                Cow::Borrowed(EVIDENCE_CLUE_DUEL_IDENTITIES),
                BoundGateKind::ClueDuelIdentities(w),
            ),
            ("clue_duel_handshake", NamedPredicateParams::ClueWitness(w)) => (
                Cow::Borrowed(EVIDENCE_CLUE_DUEL_HANDSHAKE),
                BoundGateKind::ClueDuelHandshake(w),
            ),
            ("clue_duel_trail", NamedPredicateParams::ClueWitness(w)) => (
                Cow::Borrowed(EVIDENCE_CLUE_DUEL_TRAIL),
                BoundGateKind::ClueDuelTrail(w),
            ),
            ("clue_duel_reward_banked", NamedPredicateParams::ClueWitness(w)) => (
                Cow::Borrowed(EVIDENCE_CLUE_DUEL_REWARD_BANKED),
                BoundGateKind::ClueDuelRewardBanked(w),
            ),
            ("bank_closed", NamedPredicateParams::None) => (
                Cow::Borrowed(EVIDENCE_BANK_CLOSED),
                BoundGateKind::BankClosed,
            ),
            ("clue_duel_status_settled", NamedPredicateParams::ClueDuelStatusSettled(p)) => {
                if p.ticks == 0 {
                    return Err(invalid("ticks must be positive".into()));
                }
                (
                    Cow::Borrowed(EVIDENCE_CLUE_DUEL_STATUS_SETTLED),
                    BoundGateKind::ClueDuelStatusSettled(p),
                )
            }
            ("kq_identities", NamedPredicateParams::KqWitness(w)) => (
                Cow::Borrowed(EVIDENCE_KQ_IDENTITIES),
                BoundGateKind::KqIdentities(w),
            ),
            ("kq_first_descent", NamedPredicateParams::KqWitness(w)) => (
                Cow::Borrowed(EVIDENCE_KQ_FIRST_DESCENT),
                BoundGateKind::KqFirstDescent(w),
            ),
            ("kq_first_combat", NamedPredicateParams::KqWitness(w)) => (
                Cow::Borrowed(EVIDENCE_KQ_FIRST_COMBAT),
                BoundGateKind::KqFirstCombat(w),
            ),
            ("kq_first_kill_banked", NamedPredicateParams::KqWitness(w)) => (
                Cow::Borrowed(EVIDENCE_KQ_FIRST_KILL_BANKED),
                BoundGateKind::KqFirstKillBanked(w),
            ),
            ("kq_second_descent", NamedPredicateParams::KqWitness(w)) => (
                Cow::Borrowed(EVIDENCE_KQ_SECOND_DESCENT),
                BoundGateKind::KqSecondDescent(w),
            ),
            ("kq_safe_complete", NamedPredicateParams::KqWitness(w)) => (
                Cow::Borrowed(EVIDENCE_KQ_SAFE_COMPLETE),
                BoundGateKind::KqSafeComplete(w),
            ),
            ("kq_finish_ready", NamedPredicateParams::KqWitness(w)) => (
                Cow::Borrowed(EVIDENCE_KQ_FINISH_READY),
                BoundGateKind::KqFinishReady(w),
            ),
            ("gnome_radius_respected", NamedPredicateParams::GnomeRadius(w)) => (
                Cow::Borrowed(EVIDENCE_GNOME_RADIUS_RESPECTED),
                BoundGateKind::GnomeRadiusRespected(w),
            ),
            ("quest_seed_exact", NamedPredicateParams::QuestSeedExact(p)) => {
                validate_quest_seed_params(&p).map_err(invalid)?;
                (
                    Cow::Borrowed(EVIDENCE_QUEST_SEED_EXACT),
                    BoundGateKind::QuestSeedExact(p),
                )
            }
            ("standing_bank_closed", NamedPredicateParams::StandingBankClosed(p)) => (
                Cow::Borrowed(EVIDENCE_STANDING_BANK_CLOSED),
                BoundGateKind::StandingBankClosed(p),
            ),
            ("bank_exact", NamedPredicateParams::BankExact(p)) => {
                for (id, count) in &p.expected {
                    if *id < 0 || *count <= 0 {
                        return Err(invalid(format!(
                            "bank entry ({id}, {count}) must have non-negative id and positive count"
                        )));
                    }
                }
                (
                    Cow::Borrowed(EVIDENCE_BANK_EXACT),
                    BoundGateKind::BankExact(p),
                )
            }
            ("world_speed_changed", NamedPredicateParams::WorldSpeedChanged(p)) => {
                if !(MIN_WORLD_SPEED_MS..=MAX_WORLD_SPEED_MS).contains(&p.expected_ms) {
                    return Err(invalid(format!(
                        "expected_ms {} is outside {MIN_WORLD_SPEED_MS}..={MAX_WORLD_SPEED_MS}",
                        p.expected_ms
                    )));
                }
                (
                    Cow::Borrowed(EVIDENCE_WORLD_SPEED_CHANGED),
                    BoundGateKind::WorldSpeedChanged(p),
                )
            }
            ("revision_landing_idle", NamedPredicateParams::RevisionLandingIdle(p)) => {
                if p.accepted_tiles.is_empty() {
                    return Err(invalid("accepted_tiles must not be empty".into()));
                }
                (
                    Cow::Borrowed(EVIDENCE_REVISION_LANDING_IDLE),
                    BoundGateKind::RevisionLandingIdle(p),
                )
            }
            ("duel_round_trip", NamedPredicateParams::DuelRoundTrip(w)) => (
                Cow::Borrowed(EVIDENCE_DUEL_ROUND_TRIP),
                BoundGateKind::DuelRoundTrip(w),
            ),
            (_, wrong) => {
                return Err(invalid(format!(
                    "mismatched parameter variant {}",
                    param_variant_name(&wrong)
                )));
            }
        };

        Ok(Self {
            descriptor,
            evidence,
            kind,
        })
    }

    pub fn descriptor(&self) -> NamedPredicateDescriptor {
        self.descriptor
    }

    pub fn name(&self) -> &'static str {
        self.descriptor.name
    }

    pub fn evidence(&self) -> &str {
        self.evidence.as_ref()
    }

    /// Evaluate the bound gate against `ctx`, returning `Pass` or a descriptive `Fail`.
    pub fn evaluate(&self, ctx: PredicateContext<'_>) -> PredicateVerdict {
        match &self.kind {
            BoundGateKind::CompanionsPrepared(p) => {
                if p.barrier.all_ready() {
                    PredicateVerdict::Pass
                } else {
                    let (ready, expected) = p.barrier.state_masks();
                    PredicateVerdict::Fail {
                        reason: format!(
                            "companions_prepared: companion barrier not ready (ready=0x{ready:02x}, expected=0x{expected:02x})"
                        ),
                    }
                }
            }
            BoundGateKind::QuesterStartExact(p) => {
                match validate_quester_start_seed_with(
                    ctx.snapshot,
                    &p.stats,
                    &p.inventory,
                    &p.equipment,
                    p.hitpoints_may_be_hurt,
                )
                .and_then(|()| validate_quester_start_stand(ctx.snapshot.tile(), p.stand))
                {
                    Ok(()) => PredicateVerdict::Pass,
                    Err(reason) => PredicateVerdict::Fail { reason },
                }
            }
            BoundGateKind::ClueDuelIdentities(w) => {
                let mut witness = w.inner.lock().expect("clue witness mutex");
                witness.start();
                witness.observe(0, ctx.snapshot);
                if witness.identities_ready() {
                    PredicateVerdict::Pass
                } else {
                    PredicateVerdict::Fail {
                        reason:
                            "clue_duel_identities: two distinct visible accounts not yet observed"
                                .into(),
                    }
                }
            }
            BoundGateKind::ClueDuelHandshake(w) => {
                let mut witness = w.inner.lock().expect("clue witness mutex");
                witness.observe(0, ctx.snapshot);
                if witness.handshake_complete() {
                    PredicateVerdict::Pass
                } else {
                    PredicateVerdict::Fail {
                        reason: "clue_duel_handshake: lobby, offer, confirm (rules 1024), and pen not complete for both slots"
                            .into(),
                    }
                }
            }
            BoundGateKind::ClueDuelTrail(w) => {
                let mut witness = w.inner.lock().expect("clue witness mutex");
                witness.observe(0, ctx.snapshot);
                if witness.trail_complete() {
                    PredicateVerdict::Pass
                } else {
                    PredicateVerdict::Fail {
                        reason: "clue_duel_trail: casket, clue consumption, forfeit, reward, and return not complete"
                            .into(),
                    }
                }
            }
            BoundGateKind::ClueDuelRewardBanked(w) => {
                let mut witness = w.inner.lock().expect("clue witness mutex");
                witness.observe(0, ctx.snapshot);
                if witness.complete() {
                    PredicateVerdict::Pass
                } else {
                    PredicateVerdict::Fail {
                        reason: format!(
                            "clue_duel_reward_banked: reward not banked with both HP unchanged ({})",
                            witness.terminal_summary()
                        ),
                    }
                }
            }
            BoundGateKind::BankClosed => eval_bank_closed(ctx.snapshot),
            BoundGateKind::ClueDuelStatusSettled(p) => {
                let mut witness = p.witness.inner.lock().expect("clue witness mutex");
                witness.observe(0, ctx.snapshot);
                if witness.status_wait_done(ctx.snapshot.tick(), p.ticks) {
                    PredicateVerdict::Pass
                } else {
                    PredicateVerdict::Fail {
                        reason: format!(
                            "clue_duel_status_settled: waiting for {} observed ticks after close (tick={})",
                            p.ticks,
                            ctx.snapshot.tick()
                        ),
                    }
                }
            }
            BoundGateKind::KqIdentities(w) => {
                let mut witness = w.inner.lock().expect("KQ witness mutex");
                witness.start();
                witness.observe(0, ctx.snapshot);
                if witness.identities_ready() {
                    PredicateVerdict::Pass
                } else {
                    PredicateVerdict::Fail {
                        reason: format!(
                            "kq_identities: four distinct account sessions not yet observed ({})",
                            witness.terminal_summary()
                        ),
                    }
                }
            }
            BoundGateKind::KqFirstDescent(w) => {
                let mut witness = w.inner.lock().expect("KQ witness mutex");
                witness.observe(0, ctx.snapshot);
                if witness.first_descent() {
                    PredicateVerdict::Pass
                } else {
                    PredicateVerdict::Fail {
                        reason: format!("kq_first_descent: unmet ({})", witness.terminal_summary()),
                    }
                }
            }
            BoundGateKind::KqFirstCombat(w) => {
                let mut witness = w.inner.lock().expect("KQ witness mutex");
                witness.observe(0, ctx.snapshot);
                if witness.first_combat() {
                    PredicateVerdict::Pass
                } else {
                    PredicateVerdict::Fail {
                        reason: format!("kq_first_combat: unmet ({})", witness.terminal_summary()),
                    }
                }
            }
            BoundGateKind::KqFirstKillBanked(w) => {
                let mut witness = w.inner.lock().expect("KQ witness mutex");
                witness.observe(0, ctx.snapshot);
                if witness.first_kill_bank() {
                    PredicateVerdict::Pass
                } else {
                    PredicateVerdict::Fail {
                        reason: format!(
                            "kq_first_kill_banked: unmet ({})",
                            witness.terminal_summary()
                        ),
                    }
                }
            }
            BoundGateKind::KqSecondDescent(w) => {
                let mut witness = w.inner.lock().expect("KQ witness mutex");
                witness.observe(0, ctx.snapshot);
                if witness.second_descent() {
                    PredicateVerdict::Pass
                } else {
                    PredicateVerdict::Fail {
                        reason: format!(
                            "kq_second_descent: unmet ({})",
                            witness.terminal_summary()
                        ),
                    }
                }
            }
            BoundGateKind::KqSafeComplete(w) => {
                let mut witness = w.inner.lock().expect("KQ witness mutex");
                witness.observe(0, ctx.snapshot);
                if witness.complete() {
                    PredicateVerdict::Pass
                } else {
                    PredicateVerdict::Fail {
                        reason: format!("kq_safe_complete: unmet ({})", witness.terminal_summary()),
                    }
                }
            }
            BoundGateKind::KqFinishReady(w) => {
                let mut witness = w.inner.lock().expect("KQ witness mutex");
                witness.observe(0, ctx.snapshot);
                if witness.finish_ready() {
                    PredicateVerdict::Pass
                } else {
                    PredicateVerdict::Fail {
                        reason: format!("kq_finish_ready: unmet ({})", witness.terminal_summary()),
                    }
                }
            }
            BoundGateKind::GnomeRadiusRespected(w) => {
                match w
                    .inner
                    .lock()
                    .expect("gnome radius witness mutex")
                    .observe_detailed(ctx.snapshot)
                {
                    Ok(()) => PredicateVerdict::Pass,
                    Err(reason) => PredicateVerdict::Fail { reason },
                }
            }
            BoundGateKind::QuestSeedExact(p) => eval_quest_seed_exact(ctx.snapshot, p),
            BoundGateKind::StandingBankClosed(p) => eval_standing_bank_closed(ctx.snapshot, p),
            BoundGateKind::BankExact(p) => eval_bank_exact(ctx.snapshot, p),
            BoundGateKind::WorldSpeedChanged(p) => eval_world_speed_changed(ctx.snapshot, p),
            BoundGateKind::RevisionLandingIdle(p) => eval_revision_landing_idle(ctx, p),
            BoundGateKind::DuelRoundTrip(w) => {
                match w
                    .inner
                    .lock()
                    .expect("duel witness mutex")
                    .observe_detailed(ctx.snapshot)
                {
                    Ok(()) => PredicateVerdict::Pass,
                    Err(reason) => PredicateVerdict::Fail { reason },
                }
            }
        }
    }

    /// Convenience boolean wrapper over [`Self::evaluate`].
    pub fn check(&self, ctx: PredicateContext<'_>) -> bool {
        self.evaluate(ctx).is_pass()
    }
}

/// Bind and evaluate a callable named predicate gate in one call.
pub fn evaluate_by_name(
    name: &str,
    params: NamedPredicateParams,
    authority: PredicateAuthority,
    ctx: PredicateContext<'_>,
) -> Result<PredicateVerdict, NamedPredicateError> {
    let gate = NamedPredicateGate::bind(name, params, authority)?;
    Ok(gate.evaluate(ctx))
}

#[derive(Debug, Clone)]
enum BoundArmingKind {
    KqArmed(KqFleetWitnessHandle),
}

/// A validated, bound named arming step (row 11: `kq_armed`).
#[derive(Debug, Clone)]
pub struct NamedArmingStep {
    descriptor: NamedPredicateDescriptor,
    kind: BoundArmingKind,
}

impl NamedArmingStep {
    /// Resolve `name` as an arming step, failing closed on gates, summaries, or unknown names.
    pub fn bind(name: &str, params: NamedPredicateParams) -> Result<Self, NamedPredicateError> {
        let descriptor = lookup(name)?;
        match descriptor.kind {
            NamedPredicateKind::TerminalSummary => {
                return Err(NamedPredicateError::NonCallableSummary {
                    name: descriptor.name,
                    inventory_row: descriptor.inventory_row.unwrap_or(0),
                });
            }
            NamedPredicateKind::ArmingStep => {}
            other => {
                return Err(NamedPredicateError::NotAnArmingStep {
                    name: descriptor.name,
                    kind: other,
                });
            }
        }
        let kind = match (descriptor.name, params) {
            ("kq_armed", NamedPredicateParams::KqWitness(w)) => BoundArmingKind::KqArmed(w),
            (_, wrong) => {
                return Err(NamedPredicateError::InvalidParams {
                    name: descriptor.name,
                    reason: format!(
                        "mismatched parameter variant {}",
                        param_variant_name(&wrong)
                    ),
                });
            }
        };
        Ok(Self { descriptor, kind })
    }

    pub fn descriptor(&self) -> NamedPredicateDescriptor {
        self.descriptor
    }

    pub fn name(&self) -> &'static str {
        self.descriptor.name
    }

    /// Execute the arming transition.
    pub fn arm(&self) {
        match &self.kind {
            BoundArmingKind::KqArmed(witness) => witness.start(),
        }
    }
}

/// Bind and execute a named arming step in one call.
pub fn execute_arming_by_name(
    name: &str,
    params: NamedPredicateParams,
) -> Result<(), NamedPredicateError> {
    let step = NamedArmingStep::bind(name, params)?;
    step.arm();
    Ok(())
}

/// Shared exact-stand validation for `quester_start_exact` and `quester.rs`.
pub(crate) fn validate_quester_start_stand(
    tile: Option<(i32, i32, i32)>,
    stand: WorldTile,
) -> Result<(), String> {
    if tile != Some((stand.x, stand.z, stand.level)) {
        return Err(format!(
            "fixture stand expected ({},{},{}), observed {tile:?}",
            stand.x, stand.z, stand.level
        ));
    }
    Ok(())
}

/// Shared exact-seed validation for `quester_start_exact` and `quester.rs`.
pub(crate) fn validate_quester_start_seed_with(
    snapshot: &GameSnapshot,
    stats: &[SkillMinimum],
    inventory: &[(i32, i32)],
    equipment: &[i32],
    hitpoints_may_be_hurt: bool,
) -> Result<(), String> {
    if !snapshot.ingame() || snapshot.scene_state() != 2 {
        return Err("fixture Start needs ingame && scene_state == 2".into());
    }
    for minimum in stats {
        let stat = snapshot
            .stats()
            .iter()
            .find(|stat| stat.index == i32::from(minimum.skill))
            .ok_or_else(|| format!("fixture stat {} is unobserved", minimum.skill))?;
        let hurt_only = hitpoints_may_be_hurt
            && minimum.skill == HITPOINTS_SKILL_INDEX
            && stat.effective <= i32::from(minimum.level);
        if stat.base != i32::from(minimum.level)
            || (stat.effective != i32::from(minimum.level) && !hurt_only)
        {
            return Err(format!(
                "fixture stat {} expected {}, observed base {} effective {}",
                minimum.skill, minimum.level, stat.base, stat.effective
            ));
        }
    }
    for (id, count) in inventory {
        let held: i32 = snapshot
            .inventory()
            .iter()
            .filter(|item| item.def.id == *id)
            .map(|item| item.count)
            .sum();
        if held < *count {
            return Err(format!(
                "fixture item {id} expected {count}, observed {held}"
            ));
        }
    }
    for id in equipment {
        if !snapshot
            .equipment()
            .iter()
            .any(|item| item.def.id == *id && item.count > 0)
        {
            return Err(format!("fixture equipment {id} was not worn"));
        }
    }
    Ok(())
}

/// Shared positive-count item totals map used by `quest_seed_exact` and `bank_exact`.
pub(crate) fn item_totals(rows: &[ItemView]) -> BTreeMap<i32, i32> {
    let mut out = BTreeMap::new();
    for row in rows {
        if row.count > 0 {
            *out.entry(row.def.id).or_insert(0) += row.count;
        }
    }
    out
}

/// Shared quest-list colour matcher used by `quest_seed_exact` and `quest_watch.rs`.
pub(crate) fn quest_colour_is(snapshot: &GameSnapshot, display: &str, colour: i32) -> bool {
    snapshot
        .quest_statuses()
        .iter()
        .any(|row| row.name.eq_ignore_ascii_case(display) && row.colour == colour)
}

fn validate_quest_seed_params(p: &QuestSeedExactParams) -> Result<(), String> {
    if p.current_quest.trim().is_empty() {
        return Err("current_quest must be non-empty".into());
    }
    for stat in &p.stats {
        if stat.index < 0 || stat.level <= 0 {
            return Err(format!(
                "stat index {} level {} must have non-negative index and positive level",
                stat.index, stat.level
            ));
        }
    }
    for (id, count) in &p.inventory {
        if *id < 0 || *count <= 0 {
            return Err(format!(
                "inventory entry ({id}, {count}) must have non-negative id and positive count"
            ));
        }
    }
    for id in &p.equipment {
        if *id < 0 {
            return Err(format!("equipment id {id} must be non-negative"));
        }
    }
    for quest in &p.completed_quests {
        if quest.trim().is_empty() {
            return Err("completed_quests entries must be non-empty".into());
        }
    }
    if p.quest_points_varp.is_some_and(|id| id < 0) {
        return Err("quest_points_varp must be non-negative".into());
    }
    Ok(())
}

fn eval_bank_closed(snapshot: &GameSnapshot) -> PredicateVerdict {
    if Proof::BankClosed.check(snapshot, None) {
        return PredicateVerdict::Pass;
    }
    let reason = if !snapshot.ingame() || snapshot.scene_state() != 2 {
        format!(
            "bank_closed: requires ingame && scene_state == 2 (ingame={}, scene_state={})",
            snapshot.ingame(),
            snapshot.scene_state()
        )
    } else if snapshot.bank_component_id() >= 0 {
        format!(
            "bank_closed: bank component {} is still open",
            snapshot.bank_component_id()
        )
    } else {
        "bank_closed: bank is still marked loaded".into()
    };
    PredicateVerdict::Fail { reason }
}

fn eval_quest_seed_exact(
    snapshot: &GameSnapshot,
    params: &QuestSeedExactParams,
) -> PredicateVerdict {
    if !snapshot.ingame() || snapshot.scene_state() != 2 {
        return PredicateVerdict::Fail {
            reason: format!(
                "quest_seed_exact: requires ingame && scene_state == 2 (ingame={}, scene_state={})",
                snapshot.ingame(),
                snapshot.scene_state()
            ),
        };
    }
    if snapshot.tile() != Some((params.stand.x, params.stand.z, params.stand.level)) {
        return PredicateVerdict::Fail {
            reason: format!(
                "quest_seed_exact: stand expected ({},{},{}), observed {:?}",
                params.stand.x,
                params.stand.z,
                params.stand.level,
                snapshot.tile()
            ),
        };
    }
    for expected in &params.stats {
        let Some(stat) = snapshot
            .stats()
            .iter()
            .find(|stat| stat.index == expected.index)
        else {
            if expected.listed {
                return PredicateVerdict::Fail {
                    reason: format!(
                        "quest_seed_exact: listed stat {} is unobserved",
                        expected.index
                    ),
                };
            }
            continue;
        };
        // Current Hitpoints may drop when content at the start tile damages
        // the bot; the seeded fact is the base level and positive current HP
        // no greater than the target. Every other skill is exact on both base
        // and effective.
        let effective_ok = if params.hitpoints_index == Some(expected.index) {
            stat.effective > 0 && stat.effective <= expected.level
        } else {
            stat.effective == expected.level
        };
        if stat.base != expected.level || !effective_ok {
            return PredicateVerdict::Fail {
                reason: format!(
                    "quest_seed_exact: stat {} expected {}, observed base {} effective {}",
                    expected.index, expected.level, stat.base, stat.effective
                ),
            };
        }
    }
    let observed_inventory = item_totals(snapshot.inventory());
    if observed_inventory != params.inventory {
        return PredicateVerdict::Fail {
            reason: format!(
                "quest_seed_exact: inventory expected {:?}, observed {:?}",
                params.inventory, observed_inventory
            ),
        };
    }
    let worn_ids: Vec<i32> = snapshot
        .equipment()
        .iter()
        .filter(|item| item.count > 0)
        .map(|item| item.def.id)
        .collect();
    if worn_ids.len() != params.equipment.len()
        || !params.equipment.iter().all(|id| worn_ids.contains(id))
    {
        return PredicateVerdict::Fail {
            reason: format!(
                "quest_seed_exact: worn equipment expected {:?}, observed {:?}",
                params.equipment, worn_ids
            ),
        };
    }
    for display in &params.completed_quests {
        if !quest_colour_is(snapshot, display, QUEST_COLOUR_COMPLETE) {
            return PredicateVerdict::Fail {
                reason: format!(
                    "quest_seed_exact: completed quest {display:?} does not have complete colour"
                ),
            };
        }
    }
    if !quest_colour_is(snapshot, &params.current_quest, QUEST_COLOUR_NOT_STARTED) {
        return PredicateVerdict::Fail {
            reason: format!(
                "quest_seed_exact: current quest {:?} does not have not-started colour",
                params.current_quest
            ),
        };
    }
    if let Some(varp_id) = params.quest_points_varp {
        let observed = snapshot
            .varps()
            .iter()
            .find(|row| row.index == varp_id)
            .map_or(0, |row| row.value);
        if observed != params.quest_points {
            return PredicateVerdict::Fail {
                reason: format!(
                    "quest_seed_exact: quest points varp {varp_id} expected {}, observed {observed}",
                    params.quest_points
                ),
            };
        }
    }
    PredicateVerdict::Pass
}

fn eval_standing_bank_closed(
    snapshot: &GameSnapshot,
    params: &StandingBankClosedParams,
) -> PredicateVerdict {
    if !snapshot.ingame() || snapshot.scene_state() != 2 {
        return PredicateVerdict::Fail {
            reason: format!(
                "standing_bank_closed: requires ingame && scene_state == 2 (ingame={}, scene_state={})",
                snapshot.ingame(),
                snapshot.scene_state()
            ),
        };
    }
    if snapshot.tile() != Some((params.stand.x, params.stand.z, params.stand.level)) {
        return PredicateVerdict::Fail {
            reason: format!(
                "standing_bank_closed: stand expected ({},{},{}), observed {:?}",
                params.stand.x,
                params.stand.z,
                params.stand.level,
                snapshot.tile()
            ),
        };
    }
    if snapshot.bank_component_id() >= 0 {
        return PredicateVerdict::Fail {
            reason: format!(
                "standing_bank_closed: bank component {} is open",
                snapshot.bank_component_id()
            ),
        };
    }
    PredicateVerdict::Pass
}

fn eval_bank_exact(snapshot: &GameSnapshot, params: &BankExactParams) -> PredicateVerdict {
    if !snapshot.bank_loaded() {
        return PredicateVerdict::Fail {
            reason: "bank_exact: bank is not loaded".into(),
        };
    }
    let observed = item_totals(snapshot.bank());
    if observed != params.expected {
        return PredicateVerdict::Fail {
            reason: format!(
                "bank_exact: expected {:?}, observed {:?}",
                params.expected, observed
            ),
        };
    }
    PredicateVerdict::Pass
}

fn eval_world_speed_changed(
    snapshot: &GameSnapshot,
    params: &WorldSpeedChangedParams,
) -> PredicateVerdict {
    let Some(baseline) = params.baseline.get() else {
        return PredicateVerdict::Fail {
            reason: "world_speed_changed: pre-send chat baseline is absent".into(),
        };
    };
    if crate::quest_fast::tick_speed_confirmed(snapshot, baseline, params.expected_ms) {
        return PredicateVerdict::Pass;
    }
    let detail = match crate::proof::world_speed_confirmation_after(snapshot, baseline) {
        Some((actual, seq)) => format!(
            "world_speed_changed: expected {}ms after baseline {baseline}, observed {actual}ms at seq {seq}",
            params.expected_ms
        ),
        None => format!(
            "world_speed_changed: no fresh exact {}ms system-chat confirmation after baseline {baseline}",
            params.expected_ms
        ),
    };
    PredicateVerdict::Fail { reason: detail }
}

fn eval_revision_landing_idle(
    ctx: PredicateContext<'_>,
    params: &RevisionLandingIdleParams,
) -> PredicateVerdict {
    let Some(actual_revision) = ctx.actual_revision else {
        return PredicateVerdict::Fail {
            reason: "revision_landing_idle: actual client revision is unavailable".into(),
        };
    };
    if actual_revision != params.expected_revision {
        return PredicateVerdict::Fail {
            reason: format!(
                "revision_landing_idle: expected revision {:?}, observed {:?}",
                params.expected_revision, actual_revision
            ),
        };
    }
    let tile_ok = ctx.snapshot.tile().is_some_and(|(x, z, level)| {
        params
            .accepted_tiles
            .iter()
            .any(|tile| (tile.x, tile.z, tile.level) == (x, z, level))
    });
    if !tile_ok {
        return PredicateVerdict::Fail {
            reason: format!(
                "revision_landing_idle: tile {:?} is not in accepted landing tiles {:?}",
                ctx.snapshot.tile(),
                params.accepted_tiles
            ),
        };
    }
    if !Proof::NoActiveContinue.check(ctx.snapshot, None) {
        return PredicateVerdict::Fail {
            reason: "revision_landing_idle: active continue dialogue is present".into(),
        };
    }
    PredicateVerdict::Pass
}

fn param_variant_name(params: &NamedPredicateParams) -> &'static str {
    match params {
        NamedPredicateParams::None => "None",
        NamedPredicateParams::CompanionsPrepared(_) => "CompanionsPrepared",
        NamedPredicateParams::QuesterStartExact(_) => "QuesterStartExact",
        NamedPredicateParams::ClueWitness(_) => "ClueWitness",
        NamedPredicateParams::ClueDuelStatusSettled(_) => "ClueDuelStatusSettled",
        NamedPredicateParams::KqWitness(_) => "KqWitness",
        NamedPredicateParams::GnomeRadius(_) => "GnomeRadius",
        NamedPredicateParams::QuestSeedExact(_) => "QuestSeedExact",
        NamedPredicateParams::StandingBankClosed(_) => "StandingBankClosed",
        NamedPredicateParams::BankExact(_) => "BankExact",
        NamedPredicateParams::WorldSpeedChanged(_) => "WorldSpeedChanged",
        NamedPredicateParams::RevisionLandingIdle(_) => "RevisionLandingIdle",
        NamedPredicateParams::DuelRoundTrip(_) => "DuelRoundTrip",
    }
}

#[cfg(test)]
#[path = "named_predicates_tests.rs"]
mod tests;
