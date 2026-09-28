//! Common compiled-card contract. M-297 owns installation, lifecycle and action bodies.
//! These types do not install a second runtime or alter the existing card registry.
use std::marker::PhantomData;
use std::num::NonZeroU64;
use std::sync::{Arc, LazyLock};
use std::task::Poll;

use crate::quester::pair::QuestPairPort;
use crate::shim::{InteractReq, ScriptPaint};
use crate::{CompiledId, FindOptions, SettingDef};
use api::game_data::SelectedGameData;
use api::quest_progress::{EvidenceProvider, EvidenceStamp, QuestProgress};
use api::selected::{FactError, FamilyPreparation, QuestGate, RunKey, SelectedPin, Truth};
use api::{DetectedRandom, RandomClaim, WorldTile};

pub type SettingsBag = serde_json::Map<String, serde_json::Value>;

/// Validated registry-owned payload; construction and accessors belong to M-297.
pub struct PreparedConfig {
    _private: (),
}

pub struct PrepareContext<'a> {
    pub selected: Arc<SelectedGameData>,
    pub pin: Arc<SelectedPin>,
    pub banks: Arc<api::named_banks::NamedBankFacts>,
    pub families: &'a mut FamilyPreparation,
}

pub type PrepareCard =
    fn(&mut PrepareContext<'_>, u64, Arc<SettingsBag>) -> Result<Arc<PreparedConfig>, StartError>;
pub type CreateScript =
    fn(RunKey, Arc<PreparedConfig>, &mut RetainedMemory) -> Result<Box<dyn Script>, StartError>;

pub struct CompiledCard {
    pub id: CompiledId,
    pub name: &'static str,
    pub description: &'static str,
    pub category: &'static str,
    pub schema_version: u16,
    pub schema: fn() -> &'static [SettingDef],
    pub per_account_settings: &'static [&'static str],
    pub prepare: PrepareCard,
    pub create: CreateScript,
}

#[derive(Debug, Clone)]
pub enum StartError {
    Busy,
    Unavailable(Arc<str>),
    Facts(FactError),
    Config(ConfigError),
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError {
    pub field: Arc<str>,
    pub code: Arc<str>,
    pub message: Arc<str>,
}

/// Slot-owned lazy recovery cells. M-297 installs clue retention outside action/card drops.
pub struct RetainedMemory {
    _private: (),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsApply {
    Applied,
    PendingBoundary,
    RestartRequired,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    Operator,
    Replaced,
    Completed,
    Error,
    Removed,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interrupt {
    Pause,
    Resume,
    Hold(bool),
    SessionEnded,
    SessionReady,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScriptFlow {
    Continue,
    Blocked(ScriptFailure),
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptFailure {
    pub code: Arc<str>,
    pub message: Arc<str>,
    pub retryable: bool,
}

pub trait Script: Send {
    fn tick(&mut self, cx: &mut NativeTick<'_>) -> Result<ScriptFlow, ScriptFailure>;
    fn configure(&mut self, next: Arc<PreparedConfig>) -> Result<SettingsApply, ConfigError>;
    fn retry(&mut self) -> Result<(), ScriptFailure>;
    fn interrupt(&mut self, event: Interrupt);
    fn on_stop(&mut self, reason: StopReason);
    fn on_random(&mut self, event: &DetectedRandom) -> RandomClaim;
    fn recovery_anchor(&self) -> Option<WorldTile>;
}

pub struct NativeTick<'a> {
    pub actions: &'a mut NativeActions,
    pub cx: ActionContext<'a>,
    pub output: &'a mut dyn NativeOutput,
    pub pairs: Option<&'a dyn QuestPairPort>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativePhase {
    Preparing,
    Working,
    Waiting,
    Blocked,
    Complete,
}
#[derive(Debug, Clone)]
pub enum StatusValue {
    Text(Arc<str>),
    Integer(i64),
    Tile(WorldTile),
    Truth(Truth),
    Quest(Arc<QuestProgress>),
}
#[derive(Debug, Clone)]
pub struct StatusField {
    pub key: &'static str,
    pub label: &'static str,
    pub value: StatusValue,
}
#[derive(Debug, Clone)]
pub struct ScriptStatus {
    pub run: RunKey,
    pub card: CompiledId,
    pub phase: NativePhase,
    pub active_settings: u64,
    pub pending_settings: Option<u64>,
    pub fields: Arc<[StatusField]>,
    pub failure: Option<ScriptFailure>,
}

pub trait NativeOutput {
    fn status(&mut self, status: ScriptStatus);
    fn paint(&mut self, frame: Arc<ScriptPaint>);
    fn log(&mut self, level: api::hostlog::Level, message: &str);
    fn settings_applied(&mut self, revision: u64);
}

/// M-297 constructs the lazy slot facility, borrowing the single admission budget.
pub struct NativeActions {
    _private: (),
}
/// Host-only frame context; deliberately no public constructor or family preparation.
pub struct ActionContext<'a> {
    _frame: PhantomData<&'a mut ()>,
}
/// Revocable owner identity; no caller can mint a lease.
pub struct QuietReadLease {
    _run: RunKey,
    _request_id: NonZeroU64,
}

pub struct WalkRequest {
    pub target: WorldTile,
    pub radius: u16,
    pub options: FindOptions,
    pub required_after: EvidenceStamp,
    pub evidence: Option<Arc<dyn EvidenceProvider>>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WalkEnd {
    Arrived,
    RouteEnded,
    Refused,
    Blocked,
    Failed,
    Cancelled,
    NeedsEvidence(Arc<[QuestGate]>),
}
#[derive(Debug, Clone)]
pub struct WalkReceipt {
    pub request_id: u64,
    pub evidence: EvidenceStamp,
    pub end: WalkEnd,
}

impl ActionContext<'_> {
    /// body owned by M-297
    pub fn begin_quiet_read(&mut self, _request_id: u64) -> Result<QuietReadLease, ActionError> {
        Err(unavailable())
    }
    /// body owned by M-297
    pub fn charge_transition(&mut self) -> bool {
        false
    }
    /// body owned by M-297
    pub fn emit(&mut self, _request: InteractReq) -> Result<(), ActionError> {
        Err(unavailable())
    }
    /// body owned by M-297
    pub fn walk(&mut self, _request: WalkRequest) -> Result<u64, ActionError> {
        Err(unavailable())
    }
    /// body owned by M-297
    pub fn walk_receipt(&self, _request_id: u64) -> Option<&WalkReceipt> {
        None
    }
}

/// Non-Clone owner guard. M-297 adds the ledger/revocation body before construction.
#[must_use]
pub struct ActionHandle<M: NativeMachine> {
    _run: RunKey,
    _id: NonZeroU64,
    _machine: PhantomData<M>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionError {
    Busy,
    Held,
    BudgetExhausted,
    Stale,
    Cancelled,
    Unavailable(Arc<str>),
    Failed(Arc<str>),
}

pub trait NativeMachine: Send + 'static {
    type Args: Send;
    type Output: Send;
    fn begin(args: Self::Args, cx: &mut ActionContext<'_>) -> Result<Self, ActionError>
    where
        Self: Sized;
    fn poll(&mut self, cx: &mut ActionContext<'_>) -> Poll<Result<Self::Output, ActionError>>;
    fn cancel(&mut self);
}

impl NativeActions {
    /// body owned by M-297
    pub fn begin<M: NativeMachine>(
        &mut self,
        _args: M::Args,
        _cx: &mut ActionContext<'_>,
    ) -> Result<ActionHandle<M>, ActionError> {
        Err(unavailable())
    }
    /// body owned by M-297
    pub fn poll<M: NativeMachine>(
        &mut self,
        _handle: &ActionHandle<M>,
        _cx: &mut ActionContext<'_>,
    ) -> Poll<Result<M::Output, ActionError>> {
        Poll::Ready(Err(unavailable()))
    }
}

fn unavailable() -> ActionError {
    // One process allocation, never a new error string on each poll.
    static REASON: LazyLock<Arc<str>> =
        LazyLock::new(|| Arc::from("native action facility unavailable"));
    ActionError::Unavailable(Arc::clone(&REASON))
}
