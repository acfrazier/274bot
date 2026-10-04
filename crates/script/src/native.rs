//! Compiled-card registration, validated configuration and shared output.
//! The action-machine facility is installed separately from card registration.
use std::any::Any;
use std::cell::RefCell;
use std::num::NonZeroU64;
use std::sync::{Arc, LazyLock};
use std::task::Poll;
use std::time::{Duration, Instant};

use crate::quester::pair::QuestPairPort;
use crate::shim::{InteractReq, ScriptPaint};
use crate::{CompiledId, FindOptions, SettingDef};
use api::game_data::SelectedGameData;
use api::quest_progress::{EvidenceProvider, EvidenceStamp, QuestProgress};
use api::selected::{FactError, FamilyPreparation, QuestGate, RunKey, SelectedPin, Truth};
use api::snapshot::SnapshotView;
use api::{DetectedRandom, RandomClaim, WorldTile};

mod actions;
pub mod death;
pub(crate) mod ledger;
mod owner;
pub mod walk;
pub mod walk_wait;
pub use ledger::{HostAction, HostAuthority, HostEffect, QuietReadOwner};

pub type SettingsBag = serde_json::Map<String, serde_json::Value>;

/// A registry-prepared card-owned value. Only card preparers can construct it.
pub struct PreparedConfig {
    card: CompiledId,
    schema: u16,
    revision: u64,
    bag: Arc<SettingsBag>,
    value: Option<Box<dyn Any + Send + Sync>>,
}

impl PreparedConfig {
    pub(crate) fn new<T: Send + Sync + 'static>(
        card: CompiledId,
        schema: u16,
        revision: u64,
        bag: Arc<SettingsBag>,
        value: T,
    ) -> Arc<Self> {
        Arc::new(Self {
            card,
            schema,
            revision,
            bag,
            value: Some(Box::new(value)),
        })
    }

    pub fn card(&self) -> CompiledId {
        self.card
    }
    pub fn schema_version(&self) -> u16 {
        self.schema
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn bag(&self) -> &SettingsBag {
        &self.bag
    }
    pub fn get<T: Send + Sync + 'static>(&self) -> Option<&T> {
        self.value.as_deref()?.downcast_ref()
    }
}

impl Drop for PreparedConfig {
    fn drop(&mut self) {
        // Card-owned payloads can be last-released by the UI, a rejected
        // delivery, or a detached preparation worker's thread packet.
        // Contain their destructors at the owner, independent of the caller.
        let value = self.value.take();
        if let Err(payload) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(value)))
        {
            // A custom panic payload may itself panic when destroyed.
            std::mem::forget(payload);
        }
    }
}

impl std::fmt::Debug for PreparedConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedConfig")
            .field("card", &self.card)
            .field("schema", &self.schema)
            .field("revision", &self.revision)
            .finish_non_exhaustive()
    }
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

#[derive(Debug)]
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

#[derive(Debug, Clone, PartialEq, Eq)]
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

impl ConfigError {
    pub fn new(field: &str, code: &str, message: impl Into<Arc<str>>) -> Self {
        Self {
            field: field.into(),
            code: code.into(),
            message: message.into(),
        }
    }
}

impl std::fmt::Display for StartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Busy => f.write_str("script already active: stop it first"),
            Self::Unavailable(reason) => f.write_str(reason),
            Self::Facts(error) => write!(f, "selected facts: {error:?}"),
            Self::Config(error) => write!(f, "{}: {} ({})", error.field, error.message, error.code),
        }
    }
}

impl std::error::Error for StartError {}

/// Slot-owned lazy recovery cells. M-297 installs clue retention outside action/card drops.
#[derive(Default)]
pub struct RetainedMemory {
    clue: crate::clue::ClueRecovery,
    gather: crate::gatherer::GatherRetained,
}

impl RetainedMemory {
    pub fn clue(&mut self) -> &mut crate::clue::ClueRecovery {
        &mut self.clue
    }

    pub fn gather(&mut self) -> &mut crate::gatherer::GatherRetained {
        &mut self.gather
    }
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
}

pub trait Script: Send {
    fn tick(&mut self, cx: &mut NativeTick<'_>) -> Result<ScriptFlow, ScriptFailure>;
    fn configure(&mut self, _next: Arc<PreparedConfig>) -> Result<SettingsApply, ConfigError> {
        Err(ConfigError::new(
            "",
            "configuration-unsupported",
            "this instance has no configuration receiver",
        ))
    }
    /// Request one colour-first journal read at the next safe step boundary.
    fn read_journal(&mut self) -> Result<(), ScriptFailure> {
        Err(ScriptFailure {
            code: "read-journal-unsupported".into(),
            message: "this card does not read quest journals".into(),
        })
    }
    fn interrupt(&mut self, _event: Interrupt) {}
    fn on_stop(&mut self, _reason: StopReason) {}
    fn on_random(&mut self, _event: &DetectedRandom) -> RandomClaim {
        RandomClaim::Host
    }
    fn recovery_anchor(&self) -> Option<WorldTile> {
        None
    }
}

pub struct NativeTick<'a> {
    pub actions: &'a mut NativeActions,
    pub cx: ActionContext<'a>,
    pub output: &'a mut dyn NativeOutput,
    pub pairs: Option<&'a dyn QuestPairPort>,
    #[cfg(feature = "load")]
    pub(crate) frame: HostFrame<'a>,
}

#[cfg(all(feature = "load", feature = "test-hooks"))]
impl NativeTick<'_> {
    /// Admission-race fixtures use Sherlock's existing queue, never a second
    /// test dispatch path. Not present in production builds.
    pub fn queue_test_interaction(&mut self, request: InteractReq) {
        self.frame
            .compiled
            .interacts
            .as_mut()
            .expect("compiled test queue")
            .push(request);
    }
}

/// The existing Sherlock adapter's single borrowed host bridge. No driver or
/// action facility escapes here. The host/Load clue extraction replaces this
/// bridge; effects still use the slot's existing compiled queue in the meantime.
#[cfg(feature = "load")]
pub(crate) struct HostFrame<'a> {
    pub here: Option<(i32, i32, i32)>,
    pub snapshot: Option<&'a api::snapshot::GameSnapshot>,
    pub obj_names: Option<&'a api::obj_names::ObjNames>,
    pub compiled: crate::CompiledTick<'a>,
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

impl PartialEq for StatusValue {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Text(a), Self::Text(b)) => a == b,
            (Self::Integer(a), Self::Integer(b)) => a == b,
            (Self::Tile(a), Self::Tile(b)) => a == b,
            (Self::Truth(a), Self::Truth(b)) => a == b,
            // Progress publications are immutable and shared on change.
            (Self::Quest(a), Self::Quest(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
}
#[derive(Debug, Clone, PartialEq)]
pub struct StatusField {
    pub key: &'static str,
    pub label: &'static str,
    pub value: StatusValue,
}
#[derive(Debug, Clone, PartialEq)]
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

/// Typed machine admission into the slot's foreground action ledger.
pub struct NativeActions {
    pub(crate) _private: (),
}
/// Host-only frame context; deliberately no public constructor or family preparation.
pub struct ActionContext<'a> {
    pub(crate) evidence: EvidenceStamp,
    pub(crate) pin: &'a SelectedPin,
    pub(crate) snapshot: SnapshotView<'a>,
    pub(crate) retained: &'a mut RetainedMemory,
    pub(crate) action_id: u64,
    pub(crate) active_now: Duration,
    pub(crate) wall_now: Instant,
    pub(crate) ledger: &'a mut Option<Box<ledger::Ledger>>,
    pub(crate) budget: &'a mut ledger::TickBudget,
    pub(crate) eligible: bool,
    pub(crate) observed_walk_outcome_seq: u64,
}
/// Revocable owner identity; no caller can mint a lease.
pub struct QuietReadLease {
    owner: Arc<owner::Owner>,
    lease_id: NonZeroU64,
}

/// Owner allowances for a followed walk. Prayer must be allowed or
/// [`crate::combat::WalkGuard::begin`] refuses with `PrayerDisallowed`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WalkAllow {
    pub prayer: bool,
}

impl Default for WalkAllow {
    fn default() -> Self {
        Self { prayer: true }
    }
}

pub struct WalkRequest {
    pub target: WorldTile,
    /// Explicit identity for a walk whose destination is a loc origin.
    /// `None` deliberately keeps the destination tile-based.
    pub loc_id: Option<i32>,
    /// Chebyshev arrival margin. With `loc_id: None`, it measures from
    /// `target`; with `Some(loc_id)`, it measures from the full rotated footprint.
    /// A loc stand must also pass the shared live wall/force-approach rule.
    /// Perimeter stands are admitted only when their footprint distance is
    /// within this margin; off-scene geometry is an estimate, not arrival proof.
    pub radius: u16,
    /// Destination settlement mode; `Reach` remains the default behavior.
    pub arrival: nav::arrival::ArrivalKind,
    pub options: FindOptions,
    pub required_after: EvidenceStamp,
    pub evidence: Option<Arc<dyn EvidenceProvider>>,
    pub cross: Box<[Arc<str>]>,
    /// Create a [`crate::combat::WalkGuard`] for this followed route.
    pub protect: bool,
    /// Represented on the request so a protect walk can still be refused
    /// when the owner disallows prayer.
    pub allow: WalkAllow,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WalkEnd {
    Arrived,
    UserInput,
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
    pub blocked: Option<Arc<[nav::zones::ZoneKey]>>,
    pub detail: Option<Arc<str>>,
}

/// Non-terminal evidence from a followed walk. The owner chooses whether to
/// continue, cancel or replace the route; observing this never revokes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WalkEventKind {
    Unprotectable {
        protect: crate::combat::GuardProtect,
    },
}

#[derive(Debug, Clone)]
pub struct WalkEvent {
    pub request_id: u64,
    pub evidence: EvidenceStamp,
    pub kind: WalkEventKind,
    pub detail: Arc<str>,
}

/// Result of the host's dispatch attempt, not proof of a server-side change.
/// Machines must still observe the corresponding world/interface transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InteractionReceipt {
    pub request_id: u64,
    pub evidence: EvidenceStamp,
    pub accepted: bool,
    /// Newest chat sequence in the host's pre-send snapshot, not in the
    /// later snapshot where the script consumes this receipt.
    pub chat_since: i32,
}

impl<'a> ActionContext<'a> {
    pub fn run(&self) -> RunKey {
        self.evidence.run
    }
    pub fn active_now(&self) -> Duration {
        self.active_now
    }
    pub fn wall_now(&self) -> Instant {
        self.wall_now
    }
    pub fn evidence(&self) -> EvidenceStamp {
        self.evidence
    }
    pub fn pin(&self) -> &SelectedPin {
        self.pin
    }
    pub fn snapshot(&self) -> SnapshotView<'a> {
        self.snapshot
    }
    pub fn retained(&mut self) -> &mut RetainedMemory {
        self.retained
    }
    pub fn action_id(&self) -> u64 {
        self.action_id
    }
}

/// Non-Clone guard. Dropping it revokes host work before local cleanup.
#[must_use]
pub struct ActionHandle<M: NativeMachine> {
    owner: Arc<owner::Owner>,
    machine: RefCell<Option<M>>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionError {
    Busy,
    Held,
    BudgetExhausted,
    Stale,
    Cancelled,
    /// A step adapter was interrupted by manual movement, not owner revocation.
    UserInput,
    Unavailable(Arc<str>),
    Failed(Arc<str>),
    /// The step cannot continue safely without an explicit operator retry.
    Blocked(Arc<str>),
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

#[cfg(test)]
mod preparation_drop_tests {
    use super::*;

    struct PanickingConfig;
    impl Drop for PanickingConfig {
        fn drop(&mut self) {
            panic!("card configuration destructor");
        }
    }

    #[test]
    fn settings_worker_result_can_be_discarded_without_unwinding() {
        let worker = std::thread::spawn(|| {
            PreparedConfig::new(
                CompiledId("test"),
                1,
                1,
                Arc::new(SettingsBag::new()),
                PanickingConfig,
            )
        });
        let prepared = worker.join().unwrap();
        let discarded = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(prepared)));
        assert!(
            discarded.is_ok(),
            "discarding stale settings unwound into the caller"
        );
    }
}
