//! Compiled-card registration, validated configuration and shared output.
//! The action-machine facility is installed separately from card registration.
use std::any::Any;
use std::cell::RefCell;
use std::num::NonZeroU64;
use std::sync::{Arc, LazyLock};
use std::task::Poll;
use std::time::{Duration, Instant};

use crate::interact::InteractReq;
use crate::paint::ScriptPaint;
use crate::quester::pair::QuestPairPort;
use crate::{CompiledId, SettingDef};
use api::game_data::SelectedGameData;
use api::quest_progress::{EvidenceProvider, EvidenceStamp, QuestProgress};
use api::selected::{ClientRevision, FactError, FamilyPreparation, QuestGate, RunKey, Truth};
use api::snapshot::SnapshotView;
use api::{DetectedRandom, RandomClaim, WorldTile};

/// Minimal revision-289 data for synthetic slot tests that do not read game facts.
#[cfg(any(test, feature = "test-hooks"))]
pub fn minimal_selected_data_for_test() -> Arc<SelectedGameData> {
    static SELECTED: LazyLock<Arc<SelectedGameData>> = LazyLock::new(|| {
        let selected = serde_json::from_str(
            r#"{"schema_version":5,"revision":289,"items":[],"consumption":[],"pickpocket":[]}"#,
        )
        .expect("minimal selected game data fixture");
        Arc::new(selected)
    });
    Arc::clone(&SELECTED)
}

/// Check the generated source for a real revision-data test before loading it.
#[cfg(any(test, feature = "test-hooks"))]
pub fn has_local_game_data_for_test(revision: ClientRevision) -> bool {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../api/data/game-data")
        .join(format!("{}.json", revision.as_i32()));
    let path = path.to_string_lossy();
    let description = format!("R{} game data", revision.as_i32());
    if revision == ClientRevision::R377 {
        if api::test_support::local_asset_path_377("", &path, &description).is_none() {
            return false;
        }
        match api::game_data::for_revision(revision) {
            Ok(_) => true,
            Err(api::game_data::GameDataError::LocalDataUnavailable) => {
                eprintln!("Skipping R377 tests: optional local game data was not embedded");
                false
            }
            Err(error) => panic!("load {description}: {error}"),
        }
    } else {
        api::test_support::local_asset_path("", &path, &description).is_some()
    }
}

#[cfg(any(test, feature = "test-hooks"))]
#[macro_export]
macro_rules! require_local_game_data_for_test {
    ($revision:expr) => {
        if !$crate::native::has_local_game_data_for_test($revision) {
            return;
        }
    };
}

pub mod abi;
mod actions;
pub mod custody;
pub mod divergence;
pub mod recovery_hints;
pub mod request;
pub mod seat;
pub mod seat_rows;
pub mod services;
pub mod snapshot_source;
pub use request::NativeRequest;
pub use services::{
    LocalStateCapability, PromptPoint, ScriptServices, SeatPromptPort, ServiceAuthority,
    ServiceError,
};
pub mod death;
pub mod ledger;
mod owner;
mod prereq;
mod server_ordered;
pub mod thieve;
pub use prereq::{
    HandoverEntry, OuterIdentity, OuterWalk, PrereqAllow, PrereqFailure, PrereqMissing,
    PrereqRefusal, PrereqStepIdentity, PrereqSwitch, PrereqWaitName, TalkReachCause, WalkHandover,
};
pub mod thieving_core;
pub use server_ordered::ServerOrderedCompletion;
pub mod tickcycle;
pub mod walk;
pub mod walk_wait;
pub use ledger::{
    is_teleport, AfterHold, BankDepositTrace, HostAction, HostAuthority, HostEffect,
    QuietReadOwner, WALK_TILES_PER_TICK,
};
/// Static avoid-catalog identities shared with external runtime adapters.
pub use nav_planner::contract::AVOID_CATALOG_IDS;

pub type SettingsBag = serde_json::Map<String, serde_json::Value>;

/// Why the host's latest walk outcome ended, as compiled cards read it: the
/// two values of the schema enum `isolate.fbs` `WalkCancelReason`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum WalkCancelReason {
    #[default]
    None,
    /// A manual movement took the walk over.
    UserInput,
}

/// The largest walk outcome sequence the slot can pack: bits 0-62. The host
/// counts outcomes from 1 with a plain increment and never wraps; the
/// lifetime note is at `ledger::ObservedWalkOutcome`.
pub const WALK_OUTCOME_SEQ_MAX: u64 = u64::MAX >> 1;

/// A walk-local permission bit. Inherit uses the captured global and
/// per-script settings; Allow can add permission and Forbid can narrow it.
/// The danger-zone bit is the exception: only Allow has an effect, and Forbid
/// reads as Inherit, because the global danger setting has no exception.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum WalkBit {
    #[default]
    Inherit,
    Allow,
    Forbid,
}

impl WalkBit {
    pub const fn resolve(self, global: bool, script: bool) -> bool {
        match self {
            Self::Inherit => global || script,
            Self::Allow => true,
            Self::Forbid => false,
        }
    }

    pub(crate) const fn explicit(self) -> Option<bool> {
        match self {
            Self::Inherit => None,
            Self::Allow => Some(true),
            Self::Forbid => Some(false),
        }
    }
}

impl From<Option<bool>> for WalkBit {
    fn from(value: Option<bool>) -> Self {
        match value {
            None => Self::Inherit,
            Some(true) => Self::Allow,
            Some(false) => Self::Forbid,
        }
    }
}
/// How the host judges a walk that may meet danger zones. `Proceed` walks
/// without the survival check; `Avoid` keeps the check; `Inherit` is the held
/// middle level of the global setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RiskPolicy {
    Inherit,
    Proceed,
    Avoid,
}

/// Native navigation permissions attached to one request. Native fetch is
/// intentionally absent; it is not available to compiled scripts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WalkOptions {
    pub allow_teleports: WalkBit,
    pub allow_wilderness: WalkBit,
    pub allow_danger_zones: WalkBit,
    pub allow_prereq_dialogue: WalkBit,
    pub allow_quest_start: WalkBit,
}

/// Path-level grants; an explicit walk-request `Forbid` is the only restriction.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct PrereqOptions {
    pub dialogue: bool,
    pub quest_start: bool,
}

impl PrereqOptions {
    fn apply(self, options: &mut WalkOptions) {
        if self.dialogue && options.allow_prereq_dialogue == WalkBit::Inherit {
            options.allow_prereq_dialogue = WalkBit::Allow;
        }
        if self.quest_start && options.allow_quest_start == WalkBit::Inherit {
            options.allow_quest_start = WalkBit::Allow;
        }
    }
}

/// The validated, per-script permission bits frozen into a prepared card.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WalkPermissions {
    pub allow_teleports: bool,
    pub allow_wilderness: bool,
    pub allow_danger_zones: bool,
    pub allow_prereq_dialogue: bool,
    pub allow_quest_start: bool,
}

/// The operator's global PvP setting, projected by the host from the shared
/// preferences document (`pvp.start_fights`, PVP-DESIGN-1026 §2.7). The combat
/// machine reads it once, at `begin`, and only for a request that starts a
/// fight with a named player.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PvpGlobals {
    /// Whether any script may start a fight with a player without asking.
    pub start_fights: bool,
}

impl PvpGlobals {
    /// The setting a cold, refused or malformed preference store reads as:
    /// no global permission.
    pub const fn fail_closed() -> Self {
        Self {
            start_fights: false,
        }
    }
}

/// The host's danger decision for a candidate's actual walk access tiles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WalkDanger<'a> {
    Allowed,
    Denied { zone: &'a str },
    Unavailable,
}

/// The Wilderness level of a tile (content `~wilderness_level(coord)`: 0
/// outside every zone, `areas/area_wilderness/scripts/wilderness_levels.rs2`),
/// lent by the host from the selected nav pack. The script crate owns this
/// shape so the combat machine reads no navigation implementation type.
pub trait WildernessLevels: std::fmt::Debug {
    fn level(&self, tile: WorldTile) -> i32;
}

/// A tick-borrowed query over the host's effective native walk permissions.
/// Implementations resolve settings lazily and retain no state between ticks.
pub trait WalkDangerQuery {
    fn check(
        &self,
        target: WorldTile,
        loc_id: Option<i32>,
        radius: u16,
        arrival: nav_planner::contract::ArrivalKind,
    ) -> WalkDanger<'_>;

    /// Restrict only the chosen target's access goals after eligibility checks.
    /// `None` preserves the existing approach, including its preferred tier.
    /// Implementations must not allocate goals when no access tile is denied.
    fn safe_access_goals(
        &self,
        target: WorldTile,
        loc_id: Option<i32>,
        radius: u16,
        arrival: nav_planner::contract::ArrivalKind,
        existing: Option<&WalkGoals>,
    ) -> Result<Option<WalkGoals>, WalkGoalsError>;
}

/// Test fixtures with no host danger policy explicitly allow every access tile.
#[cfg(any(test, feature = "test-hooks"))]
pub struct AllowWalkDanger;

#[cfg(any(test, feature = "test-hooks"))]
impl WalkDangerQuery for AllowWalkDanger {
    fn check(
        &self,
        _target: WorldTile,
        _loc_id: Option<i32>,
        _radius: u16,
        _arrival: nav_planner::contract::ArrivalKind,
    ) -> WalkDanger<'_> {
        WalkDanger::Allowed
    }

    fn safe_access_goals(
        &self,
        _target: WorldTile,
        _loc_id: Option<i32>,
        _radius: u16,
        _arrival: nav_planner::contract::ArrivalKind,
        _existing: Option<&WalkGoals>,
    ) -> Result<Option<WalkGoals>, WalkGoalsError> {
        Ok(None)
    }
}

impl WalkPermissions {
    fn from_bag(card: CompiledId, bag: &SettingsBag) -> Self {
        let (teleports, wilderness, danger_zones) = if card == CompiledId("Gatherer") {
            ("allowTeleports", "allowWilderness", "allowDangerZones")
        } else {
            ("allow_teleports", "allow_wilderness", "allow_danger_zones")
        };
        let read = |key| {
            bag.get(key)
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false)
        };
        Self {
            allow_teleports: read(teleports),
            allow_wilderness: read(wilderness),
            allow_danger_zones: read(danger_zones),
            allow_prereq_dialogue: read("allow_prereq_dialogue")
                || (card == CompiledId("Gatherer") && read("allowPrereqDialogue")),
            allow_quest_start: read("allow_quest_start")
                || (card == CompiledId("Gatherer") && read("allowQuestStart")),
        }
    }
}

/// A registry-prepared card-owned value. Only card preparers can construct it.
pub struct PreparedConfig {
    card: CompiledId,
    schema: u16,
    revision: u64,
    bag: Arc<SettingsBag>,
    walk_permissions: WalkPermissions,
    value: Option<Box<dyn Any + Send + Sync>>,
}

impl PreparedConfig {
    /// Create the typed value returned by a compiled card's registered preparer.
    pub fn new<T: Send + Sync + 'static>(
        card: CompiledId,
        schema: u16,
        revision: u64,
        bag: Arc<SettingsBag>,
        value: T,
    ) -> Arc<Self> {
        let walk_permissions = WalkPermissions::from_bag(card, &bag);
        Arc::new(Self {
            card,
            schema,
            revision,
            bag,
            walk_permissions,
            value: Some(Box::new(value)),
        })
    }

    /// Prepare settings for the fixed card installed by `SlotScript::start_test_script`.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn for_test_script(revision: u64, bag: Arc<SettingsBag>) -> Arc<Self> {
        Self::new(CompiledId("test"), 1, revision, bag, ())
    }

    /// Preserve card metadata while testing a factory's prepared-value type check.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn for_test_card<T: Send + Sync + 'static>(
        card: CompiledId,
        schema: u16,
        revision: u64,
        bag: Arc<SettingsBag>,
        value: T,
    ) -> Arc<Self> {
        Self::new(card, schema, revision, bag, value)
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
    pub fn walk_permissions(&self) -> WalkPermissions {
        self.walk_permissions
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
    pub revision: ClientRevision,
    pub banks: Arc<api::named_banks::NamedBankFacts>,
    pub families: &'a mut FamilyPreparation,
    pub zones: Option<nav_planner::contract::ZoneTableView<'a>>,
}

pub type PrepareCard =
    fn(&mut PrepareContext<'_>, u64, Arc<SettingsBag>) -> Result<Arc<PreparedConfig>, StartError>;
pub type CreateScript =
    fn(RunKey, Arc<PreparedConfig>, &mut RetainedMemory) -> Result<Box<dyn Script>, StartError>;

/// Static lifecycle hooks registered by a compiled card.
///
/// The card descriptor owns these function pointers. They carry no per-bot
/// state. The owning slot invokes them on its pump thread, never a control
/// thread.
#[derive(Debug, Clone, Copy)]
pub struct CardLifecycle {
    pub on_stop: fn(),
    pub on_reset: fn(),
    pub on_resume: fn(),
    pub on_pause: fn(),
    pub on_hold: fn(bool),
}

#[derive(Debug, Clone, Copy)]
pub struct CompiledCard {
    pub id: CompiledId,
    pub native_kind: request::NativeKind,
    pub name: &'static str,
    pub description: &'static str,
    pub category: &'static str,
    pub schema_version: u16,
    pub schema: fn() -> &'static [SettingDef],
    pub per_account_settings: &'static [&'static str],
    pub prepare: PrepareCard,
    pub create: CreateScript,
    pub lifecycle: Option<&'static CardLifecycle>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartError {
    Busy,
    /// A typed refusal from preparation; temporary Full is retried first.
    WorkRefused(api::work::SubmissionRefusal),
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
            Self::WorkRefused(refusal) => write!(f, "preparation refused: {refusal}"),
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
    quester: crate::quester::QuesterRetained,
    sherlock: SherlockRetained,
    walk_handover: Option<Box<RetainedWalkHandover>>,
}

struct RetainedWalkHandover {
    run: RunKey,
    handover: WalkHandover,
}

/// Watchdog-retained death state; active clue tokens and action ownership stay card-local.
#[derive(Default)]
pub struct SherlockRetained {
    death_seq: Option<i32>,
    death_pending: bool,
}

impl SherlockRetained {
    pub fn death_seq(&self) -> Option<i32> {
        self.death_seq
    }

    pub fn set_death_seq(&mut self, death_seq: Option<i32>) {
        self.death_seq = death_seq;
    }

    pub fn death_pending(&self) -> bool {
        self.death_pending
    }

    pub fn set_death_pending(&mut self, death_pending: bool) {
        self.death_pending = death_pending;
    }
}

impl RetainedMemory {
    pub fn clue(&mut self) -> &mut crate::clue::ClueRecovery {
        &mut self.clue
    }

    pub fn gather(&mut self) -> &mut crate::gatherer::GatherRetained {
        &mut self.gather
    }

    pub fn quester(&mut self) -> &mut crate::quester::QuesterRetained {
        &mut self.quester
    }

    pub fn sherlock(&mut self) -> &mut SherlockRetained {
        &mut self.sherlock
    }

    /// Reserve a one-shot factory input for this exact compiled run.
    pub fn set_walk_handover(
        &mut self,
        run: RunKey,
        handover: WalkHandover,
    ) -> Result<(), StartError> {
        if self.walk_handover.is_some() {
            return Err(StartError::Busy);
        }
        self.walk_handover = Some(Box::new(RetainedWalkHandover { run, handover }));
        Ok(())
    }

    /// A stale preparation cannot consume a newer run's input.
    /// The start owner also takes this value when preparation is cancelled.
    pub fn take_walk_handover(&mut self, run: RunKey) -> Option<WalkHandover> {
        if self.walk_handover.as_ref()?.run != run {
            return None;
        }
        self.walk_handover.take().map(|pending| pending.handover)
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
    /// Visible per-account pairing intent; the Play broker never starts another slot.
    fn pair_settings(&self) -> Option<crate::quester::pair::PairSettings> {
        None
    }
    /// Paired Path currently active; no binding while queued, unpaired or finished.
    fn pair_binding(&self) -> Option<crate::quester::pair::PairBinding<'_>> {
        None
    }
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
    /// Manual movement took over the foreground this card owns as an API
    /// session, whether or not one of its walks was in flight. Walking work
    /// also learns it from its `WalkEnd::UserInput` receipt.
    fn user_input(&mut self) {}
    /// Accepted Combat raises still owed when the host retires this fight.
    fn prayer_cleanup(&self) -> crate::combat::RaisedPrayers {
        crate::combat::RaisedPrayers::empty()
    }
    fn on_stop(&mut self, _reason: StopReason) {}
    fn on_stop_with_output(&mut self, reason: StopReason, _output: &mut dyn NativeOutput) {
        self.on_stop(reason);
    }
    fn on_random(&mut self, _event: &DetectedRandom) -> RandomClaim {
        RandomClaim::Host
    }
    /// A safe wait on new world evidence rather than elapsed time. Return true
    /// only when a fresh observation could enable work and this script has no
    /// foreground action in flight. The slot additionally validates readiness,
    /// session, hold and action ownership before suspending gameplay-idle
    /// recovery. Scheduler and missing-packet watchdogs remain active.
    fn waits_for_evidence(&self) -> bool {
        false
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
    /// The slot's local-state capability (T5.2): operator loadouts and
    /// the account's durable identity. `None` when the composition bound
    /// no state service to this slot.
    pub state: Option<&'a crate::local_state::SlotState>,
    pub service_authority: Option<ServiceAuthority<'a>>,
    /// Borrow of the owner's admitted seat, not a second request scheduler.
    pub prompt_port: Option<&'a mut dyn SeatPromptPort>,
    /// Read-only danger admission for target selection, borrowed from the host.
    pub walk_danger: Option<&'a dyn WalkDangerQuery>,
    /// The operator's global PvP setting for this tick (`pvp.start_fights`,
    /// PVP-DESIGN-1026 §2.7). `None` when the host has no ready preferences;
    /// a caller then uses [`PvpGlobals::fail_closed`].
    pub pvp_globals: Option<PvpGlobals>,
    pub(crate) frame: HostFrame<'a>,
}

impl<'frame> NativeTick<'frame> {
    pub fn frame(&self) -> &HostFrame<'frame> {
        &self.frame
    }

    pub fn frame_mut(&mut self) -> &mut HostFrame<'frame> {
        &mut self.frame
    }

    /// Bind the actual native frame to the host's current execution fence,
    /// session gate and optional T5.2 state capability. A standalone test
    /// frame without the host authority cannot mint a service.
    pub fn services<'service>(
        &'service mut self,
    ) -> Result<ScriptServices<'service, 'frame>, ServiceError>
    where
        'frame: 'service,
    {
        let authority = self
            .service_authority
            .ok_or(ServiceError::AuthorityUnavailable)?;
        let state = self
            .state
            .map(|state| LocalStateCapability::new(state.port(), state.account(), state.scope()));
        match self.prompt_port.as_deref_mut() {
            Some(port) => ScriptServices::new(authority, self.actions, &mut self.cx, state)?
                .with_prompt_port(port),
            None => ScriptServices::new(authority, self.actions, &mut self.cx, state),
        }
    }

    /// The operator's loadout rows this tick may plan with: the bound
    /// service's current lease, or explicitly absent rows when no service
    /// is bound. `NotReady` while the domain is cold: the card retries on
    /// a later tick, never plans with fabricated empty rows.
    pub fn operator_loadouts(
        &self,
    ) -> Result<crate::local_state::OperatorLoadouts, api::local_state::ErrorCode> {
        match self.state {
            Some(state) => state.operator_loadouts(),
            None => Ok(crate::local_state::OperatorLoadouts::absent()),
        }
    }
    /// Report one confirmed gameplay change for the current run and session.
    /// The slot consumes it once through the progress watchdog. This sends no
    /// interaction and spends no event budget; call only after observing a
    /// successful outcome, never for a click or an expected result.
    pub fn note_progress(&mut self) {
        self.cx.note_progress();
    }
}

#[cfg(feature = "test-hooks")]
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
pub struct HostFrame<'a> {
    pub(crate) here: Option<(i32, i32, i32)>,
    pub(crate) snapshot: Option<&'a api::snapshot::GameSnapshot>,
    pub(crate) obj_names: Option<&'a api::obj_names::ObjNames>,
    pub(crate) compiled: crate::CompiledTick<'a>,
}

impl<'a> HostFrame<'a> {
    /// Test-only constructor that borrows the fixture's existing host frame.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn for_test(
        here: Option<(i32, i32, i32)>,
        snapshot: Option<&'a api::snapshot::GameSnapshot>,
        obj_names: Option<&'a api::obj_names::ObjNames>,
        compiled: crate::CompiledTick<'a>,
    ) -> Self {
        Self {
            here,
            snapshot,
            obj_names,
            compiled,
        }
    }

    /// Test-only replacement of the fixture's optional observed snapshot.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn set_test_snapshot(&mut self, snapshot: Option<&'a api::snapshot::GameSnapshot>) {
        self.snapshot = snapshot;
    }

    pub fn here(&self) -> Option<(i32, i32, i32)> {
        self.here
    }

    pub fn snapshot(&self) -> Option<&'a api::snapshot::GameSnapshot> {
        self.snapshot
    }

    pub fn obj_names(&self) -> Option<&'a api::obj_names::ObjNames> {
        self.obj_names
    }

    pub fn compiled(&self) -> &crate::CompiledTick<'a> {
        &self.compiled
    }

    pub fn compiled_mut(&mut self) -> &mut crate::CompiledTick<'a> {
        &mut self.compiled
    }
}

/// Test-only native tick over the fixture's existing frame, ledger and output.
#[cfg(any(test, feature = "test-hooks"))]
pub fn with_test_tick<R>(
    frame: HostFrame<'_>,
    evidence: EvidenceStamp,
    revision: ClientRevision,
    clock: (Duration, Instant),
    ledger: &mut Option<Box<ledger::Ledger>>,
    output: &mut dyn NativeOutput,
    f: impl FnOnce(&mut NativeTick<'_>) -> R,
) -> R {
    let mut retained = RetainedMemory::default();
    let mut budget = ledger::TickBudget::default();
    budget.observe(evidence.tick);
    let mut actions = NativeActions { _private: () };
    let snapshot = SnapshotView::new(frame.snapshot(), evidence)
        .with_bank_memory(frame.compiled().bank_memory);
    let eligible = !frame.compiled().hold;
    let mut tick = NativeTick {
        walk_danger: None,
        pvp_globals: None,
        state: None,
        service_authority: None,
        prompt_port: None,
        actions: &mut actions,
        cx: ActionContext {
            stage_cross: None,
            protected_items: None,
            walk_prereq: PrereqOptions::default(),
            condition: None,
            poison_unknown_pulse: u8::MAX,
            cure_poison: crate::survival::CurePoison::WhenNeeded,
            evidence,
            observed_walk_outcome_seq: 0,
            observed_walk_cancel_reason: WalkCancelReason::None,
            snapshot,
            revision,
            retained: &mut retained,
            action_id: 0,
            active_now: clock.0,
            wall_now: clock.1,
            ledger,
            pending_note_progress: None,
            budget: &mut budget,
            eligible,
            wilderness: None,
        },
        output,
        pairs: None,
        frame,
    };
    f(&mut tick)
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
    /// Typed survival outcome from this runner's most recent poll.
    pub survival: crate::survival::SurvivalReport,
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
    pub(crate) walk_prereq: PrereqOptions,
    pub(crate) revision: ClientRevision,
    pub(crate) snapshot: SnapshotView<'a>,
    /// Frame-local stage consent. Every route keeps the existing named `cross` path.
    pub(crate) stage_cross: Option<Arc<[Arc<str>]>>,
    pub(crate) condition: Option<api::condition::BotCondition>,
    pub(crate) poison_unknown_pulse: u8,
    pub(crate) cure_poison: crate::survival::CurePoison,
    pub(crate) retained: &'a mut RetainedMemory,
    pub(crate) action_id: u64,
    pub(crate) active_now: Duration,
    pub(crate) wall_now: Instant,
    pub(crate) ledger: &'a mut Option<Box<ledger::Ledger>>,
    pub(crate) pending_note_progress: Option<&'a mut Option<RunKey>>,
    pub(crate) budget: &'a mut ledger::TickBudget,
    pub(crate) eligible: bool,
    pub(crate) observed_walk_outcome_seq: u64,
    /// The host's cancel reason for `observed_walk_outcome_seq`.
    pub(crate) observed_walk_cancel_reason: WalkCancelReason,
    /// Compiled Path items reserved for the active run's food decisions.
    pub(crate) protected_items: Option<Arc<[i32]>>,
    /// The Wilderness levels the host lends for this frame; `None` when the
    /// host has no nav pack. The combat machine's zone gate treats `None` as
    /// "outside the wilderness".
    pub(crate) wilderness: Option<&'a dyn WildernessLevels>,
}
/// Track manual takeover outcomes relative to one native step's begin.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ManualTakeoverWatch {
    baseline: u64,
}

impl ManualTakeoverWatch {
    /// Capture the latest observed walk outcome when the step begins.
    pub fn begin(cx: &ActionContext<'_>) -> Self {
        Self {
            baseline: cx.observed_walk_outcome_seq,
        }
    }

    /// Return true when a later outcome carries the `UserInput` reason.
    pub fn interrupted(&self, cx: &ActionContext<'_>) -> bool {
        cx.observed_walk_outcome_seq != self.baseline
            && cx.observed_walk_cancel_reason == WalkCancelReason::UserInput
    }
}

/// Revocable owner identity; no caller can mint a lease.
pub struct QuietReadLease {
    owner: Arc<owner::Owner>,
    lease_id: NonZeroU64,
}

/// The owner waits for the page its own interaction opens. While one is live
/// the host holds its auto-run press (the run button's script begins with
/// `if_close`, `player_controls.rs2:34-35`). Dropping the lease, completing
/// or dropping the machine, and revoking the owner all release it.
/// Revocable owner identity; no caller can mint a lease.
pub struct ModalWaitLease {
    owner: Arc<owner::Owner>,
    lease_id: NonZeroU64,
}

/// The bound that the owner of a script wait enforces itself, in the
/// owner's own clock. The host ends its auto-run hold only when the owner
/// ends the wait, or, as a logged failure, after this deadline plus a stated
/// margin (an owner that stopped polling). A renewed wait carries its new
/// deadline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitDeadline {
    /// The request has not been dispatched, so its owner's page deadline
    /// has not started. Acceptance replaces this with the owner's bound;
    /// revocation still releases the lease.
    AwaitingDispatch,
    /// The observed game tick (`ActionContext::evidence().tick`) on which
    /// the owner ends the wait (`tick >= deadline`).
    Tick(u64),
    /// The active time (`ActionContext::active_now`) from which the owner
    /// ends the wait at its next poll. Pause, hold, not-ready and Blocked
    /// stop the active clock, so they do not spend this bound.
    Active(Duration),
}

impl WaitDeadline {
    /// The observed tick by which a polled owner has ended its wait, read in
    /// the host sample of `tick`. `active_now` is the owner's active clock at
    /// that sample. The owner checks its bound in that frame's poll, before
    /// the sample.
    ///
    /// An undispatched request has no terminal release yet, so it remains
    /// not due at every sample until its owner installs the accepted bound.
    ///
    /// An active-clock bound is converted at each sample, from the owner's
    /// remaining active time then: its wall equivalent is `now + (deadline -
    /// active_now)`. While that is still ahead, the answer is the current
    /// tick (not yet due). While the owner is paused, its active clock does
    /// not advance, so the bound moves forward with wall time and stays not
    /// due. The first sample with `active_now >= deadline` fixes the deadline
    /// at that tick: the owner's own deadline has then passed in its own
    /// clock. The owner's poll in that frame read its clock before the
    /// sample, so it sees the bound at its next poll; the host's margin
    /// covers that one tick.
    pub fn owner_tick(&mut self, active_now: Duration, tick: u64) -> u64 {
        match *self {
            WaitDeadline::AwaitingDispatch => tick,
            WaitDeadline::Tick(deadline) => deadline,
            WaitDeadline::Active(at) if active_now < at => tick,
            WaitDeadline::Active(_) => {
                *self = WaitDeadline::Tick(tick);
                tick
            }
        }
    }

    /// The observed tick for a lease that ends itself at the wall time `at`,
    /// in the read itself. A live read before `at` is not yet due; a live
    /// read after it is already past its deadline.
    pub fn self_ending_tick(at: Instant, now: Instant, tick: u64) -> u64 {
        if now < at {
            tick
        } else {
            tick.saturating_sub(1)
        }
    }
}

/// Owner allowances for a followed walk. Prayer must be allowed or
/// [`crate::combat::WalkGuard::begin`] refuses with `PrayerDisallowed`;
/// food and escape default to allowed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WalkAllow {
    pub prayer: bool,
    pub food: bool,
    pub escape: bool,
}

impl Default for WalkAllow {
    fn default() -> Self {
        Self {
            prayer: true,
            food: true,
            escape: true,
        }
    }
}

/// Bounded exact alternatives for one native Area walk. A reachable preferred
/// goal wins over every fallback goal, even when a fallback route is shorter.
#[derive(Debug, Clone)]
pub struct WalkGoals {
    goals: Arc<Vec<WorldTile>>,
    preferred_len: usize,
}

/// A candidate request is rejected, never silently shortened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum WalkGoalsError {
    #[error("candidate goal set is empty")]
    Empty,
    #[error("{count} unique candidate goals exceed the limit of {limit}")]
    TooMany { count: usize, limit: usize },
    #[error("candidate goals require exact Area arrival without a location")]
    IncompatibleArrival,
}

impl WalkGoals {
    /// The generated 274/289 movement rectangles need at most 810 goals;
    /// stationary NPCs need 961. This leaves 63 goals of headroom.
    pub const MAX: usize = 1024;

    /// All goals belong to the preferred tier.
    pub fn new(mut goals: Vec<WorldTile>) -> Result<Self, WalkGoalsError> {
        goals.sort_unstable_by_key(|tile| (tile.level, tile.x, tile.z));
        goals.dedup();
        let preferred_len = goals.len();
        Self::from_parts(goals, preferred_len)
    }

    /// Preserve the supplied preferred goals before trying the remaining goals.
    /// The cap applies to the unique union, not to either tier separately.
    pub fn with_preferred(
        mut goals: Vec<WorldTile>,
        preferred: &[WorldTile],
    ) -> Result<Self, WalkGoalsError> {
        goals.sort_unstable_by_key(|tile| (tile.level, tile.x, tile.z));
        goals.dedup();
        for &tile in preferred {
            let key = (tile.level, tile.x, tile.z);
            if let Err(index) =
                goals.binary_search_by_key(&key, |tile| (tile.level, tile.x, tile.z))
            {
                goals.insert(index, tile);
            }
        }
        let mut preferred_len = 0;
        for &tile in preferred {
            if let Some(index) = goals[preferred_len..].iter().position(|goal| *goal == tile) {
                goals[preferred_len..=preferred_len + index].rotate_right(1);
                preferred_len += 1;
            }
        }
        Self::from_parts(goals, preferred_len)
    }

    fn from_parts(goals: Vec<WorldTile>, preferred_len: usize) -> Result<Self, WalkGoalsError> {
        if goals.is_empty() {
            return Err(WalkGoalsError::Empty);
        }
        if goals.len() > Self::MAX {
            return Err(WalkGoalsError::TooMany {
                count: goals.len(),
                limit: Self::MAX,
            });
        }
        // Retain the producer's allocation instead of copying its tiles into
        // an Arc slice. A thin Arc plus the tier boundary stays two words.
        Ok(Self {
            goals: Arc::new(goals),
            preferred_len,
        })
    }

    pub fn as_slice(&self) -> &[WorldTile] {
        self.goals.as_slice()
    }

    pub fn preferred(&self) -> &[WorldTile] {
        &self.goals[..self.preferred_len]
    }

    pub fn fallback(&self) -> &[WorldTile] {
        &self.goals[self.preferred_len..]
    }

    /// Shared Vec header, Arc counters and the retained tile capacity.
    pub fn retained_bytes(&self) -> Option<usize> {
        std::mem::size_of::<Vec<WorldTile>>()
            .checked_add(2 * std::mem::size_of::<usize>())?
            .checked_add(
                self.goals
                    .capacity()
                    .checked_mul(std::mem::size_of::<WorldTile>())?,
            )
    }
}

#[derive(Clone)]
pub struct WalkRequest {
    pub target: WorldTile,
    /// Exact alternatives for a native Area walk with radius zero and no loc.
    /// `None` preserves the existing single-target route and arrival behavior.
    pub candidate_goals: Option<WalkGoals>,
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
    pub arrival: nav_planner::contract::ArrivalKind,
    pub options: WalkOptions,
    pub required_after: EvidenceStamp,
    pub evidence: Option<Arc<dyn EvidenceProvider>>,
    pub cross: Box<[Arc<str>]>,
    pub avoid: Box<[Arc<str>]>,
    /// Create a [`crate::combat::WalkGuard`] for this followed route.
    pub protect: bool,
    /// Create a food-only [`crate::combat::WalkGuard`] when `protect` is
    /// false. A declared danger crossing arms this guard for ordinary walks.
    /// Food use still follows `allow.food`.
    pub food_guard: bool,
    /// Retain held-cure upkeep without requesting protection or food upkeep.
    /// Crossing normalization preserves this request's food allowance.
    /// Only the exhausted combat escape sets this flag.
    pub cure_guard: bool,
    /// Represented on the request so a protect walk can still be refused
    /// when the owner disallows prayer.
    pub allow: WalkAllow,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WalkRefusal {
    SearchIncomplete {
        nodes_settled: Option<usize>,
    },
    NoRouteWithinBounds {
        tried: u8,
        last: Option<crate::combat::risk::Verdict>,
    },
    RouteTooLarge {
        required_bytes: usize,
        maximum_bytes: usize,
    },
    Unsurvivable,
    PoisonUnsettled {
        waited: u8,
        cause: api::condition::WindowReset,
    },
    Unknown(crate::combat::risk::UnknownWhy),
    EscapeInProgress,
    Prerequisite(Arc<PrereqRefusal>),
}

impl std::fmt::Display for WalkRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SearchIncomplete {
                nodes_settled: Some(settled),
            } => write!(f, "route search incomplete after {settled} settled nodes"),
            Self::SearchIncomplete {
                nodes_settled: None,
            } => f.write_str("route search incomplete"),
            Self::NoRouteWithinBounds { tried, last } => {
                write!(f, "no route within bounds after {tried} search attempts")?;
                if let Some(last) = last {
                    write!(f, " (last risk verdict: {last:?})")?;
                }
                Ok(())
            }
            Self::RouteTooLarge {
                required_bytes,
                maximum_bytes,
            } => write!(
                f,
                "route result needs {required_bytes} bytes, above the {maximum_bytes}-byte limit",
            ),
            Self::Unsurvivable => f.write_str("route is unsurvivable"),
            Self::PoisonUnsettled { waited, cause } => write!(
                f,
                "poison state remained unsettled after {waited} observed ticks ({cause:?})",
            ),
            Self::Unknown(why) => write!(f, "route risk is unknown: {why}"),
            Self::EscapeInProgress => f.write_str("escape in progress"),
            Self::Prerequisite(refusal) => std::fmt::Display::fmt(refusal, f),
        }
    }
}

/// Correlated advisory result from a compute-only walk assessment.
#[derive(Debug, Clone)]
pub struct AssessReceipt {
    pub request_id: u64,
    pub evidence: EvidenceStamp,
    pub assessment: Option<Arc<crate::combat::risk::RouteAssessment>>,
    pub refusal: Option<WalkRefusal>,
    pub detail: Option<Arc<str>>,
    pub route_ticks: u32,
}

/// Native walk failure classification, independent of Traveller internals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WalkStallReason {
    /// The hop exhausted its budget after making partial progress.
    Expired,
    /// The hop exhausted its budget without leaving the send tile.
    Dropped,
    /// The live scene refused the route's final tile.
    EndBlocked,
}

/// Cold terminal data stays shared instead of enlarging every native ledger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalkStall {
    pub at: WorldTile,
    pub aiming: WorldTile,
    pub why: WalkStallReason,
    pub tries: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WalkEnd {
    Arrived,
    /// The host route planner found no route satisfying the request.
    NoPath,
    UserInput,
    RouteEnded,
    /// A Traveller hop reached its finite budget without its aim being reached.
    /// Retain its actual position, aim, reason and send count for the owner.
    Stalled(Arc<WalkStall>),
    Refused,
    Blocked,
    Failed,
    Aborted,
    Cancelled,
    NeedsEvidence(Arc<[QuestGate]>),
    NeedsStep(Arc<[api::prereq_steps::ActionableGate]>),
    GateRefused(Arc<api::prereq_knowledge::RefusalObservation>),
}
// USER-0.2.3-GATHERER §1 records a progressed return ending Expired at the
// Traveller's 60-call bound (equal to observed ticks in the current host);
// §3 records the normal content-derived stand succeeding. One fresh plan
// from the observed position is the supported recovery.
const MAX_RETURN_RECOVERY_ATTEMPTS: u8 = 2;

/// Shared bounded-recovery policy, not a content timer: one initial route and
/// two fresh replans. Quester walk first set this cap in b26097813.
pub(crate) const QUEST_WALK_MAX_ATTEMPTS: u8 = 3;

/// Shares Quester's walk-step retry budget and next-tick recovery state with
/// every family that owns an approach walk.
pub(crate) struct WalkRetryState {
    recovery: ReturnStallRecovery,
    // Read only while recovery is pending; that state owns whether this tick is set.
    retry_after_tick: u64,
}

impl Default for WalkRetryState {
    fn default() -> Self {
        Self {
            recovery: ReturnStallRecovery::with_max_attempts(QUEST_WALK_MAX_ATTEMPTS),
            retry_after_tick: 0,
        }
    }
}

impl WalkRetryState {
    pub(crate) fn observe_failure(&mut self, receipt: &WalkReceipt, tick: u64) -> bool {
        if self.recovery.observe_receipt(receipt) {
            self.retry_after_tick = tick;
            true
        } else {
            false
        }
    }

    pub(crate) fn observe_approach_failure(&mut self, receipt: &WalkReceipt, tick: u64) -> bool {
        self.observe_approach_retry(receipt, tick, Self::retryable_receipt(receipt))
    }

    pub(crate) fn observe_wall_approach_failure(
        &mut self,
        receipt: &WalkReceipt,
        tick: u64,
        has_wall_fallback: bool,
    ) -> bool {
        let retryable = if has_wall_fallback
            && receipt.end == WalkEnd::Refused
            && matches!(receipt.refusal, Some(WalkRefusal::SearchIncomplete { .. }))
        {
            receipt
                .blocked
                .as_deref()
                .is_none_or(|blocked| blocked.is_empty())
        } else {
            Self::retryable_receipt(receipt)
        };
        self.observe_approach_retry(receipt, tick, retryable)
    }

    fn observe_approach_retry(
        &mut self,
        receipt: &WalkReceipt,
        tick: u64,
        retryable: bool,
    ) -> bool {
        if !self.recovery.observe_retryable(retryable) {
            return false;
        }
        self.retry_after_tick = tick;
        let attempt = self.retry_attempt_number();
        let detail = receipt.failure_detail();
        api::host_log!(
            api::hostlog::Category::NavTrace,
            api::hostlog::Level::Debug,
            "quester-approach-retry tick={tick} attempt={attempt}/{} end={:?} detail={detail:?}",
            QUEST_WALK_MAX_ATTEMPTS,
            receipt.end,
        );
        true
    }

    pub(crate) fn retryable_receipt(receipt: &WalkReceipt) -> bool {
        let has_blocked_zones = receipt
            .blocked
            .as_deref()
            .is_some_and(|blocked| !blocked.is_empty());
        receipt.refusal.is_none()
            && !has_blocked_zones
            && matches!(
                &receipt.end,
                WalkEnd::NoPath | WalkEnd::RouteEnded | WalkEnd::Stalled(_) | WalkEnd::Failed
            )
    }

    pub(crate) fn pending(&self) -> bool {
        self.recovery.pending()
    }

    pub(crate) fn waiting_for_next_tick(&self, tick: u64) -> bool {
        self.pending() && tick <= self.retry_after_tick
    }

    pub(crate) fn defer_until_next_tick(&mut self, tick: u64) {
        if self.pending() {
            self.retry_after_tick = tick;
        }
    }

    pub(crate) fn action_submitted(&mut self) {
        self.recovery.action_submitted();
        self.retry_after_tick = 0;
    }

    pub(crate) fn reset(&mut self) {
        self.recovery.reset();
        self.retry_after_tick = 0;
    }

    pub(crate) fn retry_attempt_number(&self) -> u8 {
        self.recovery.retry_attempt_number()
    }
}

/// Counts recovery submissions separately from the initial action.
/// `max_attempts` includes that initial action.
pub(crate) struct ReturnStallRecovery {
    actions: u8,
    max_attempts: u8,
    pending: bool,
}

impl Default for ReturnStallRecovery {
    fn default() -> Self {
        Self {
            actions: 0,
            max_attempts: MAX_RETURN_RECOVERY_ATTEMPTS,
            pending: false,
        }
    }
}

impl ReturnStallRecovery {
    /// Set a total request limit, including the initial request.
    pub(crate) fn with_max_attempts(max_attempts: u8) -> Self {
        Self {
            max_attempts: max_attempts.max(1),
            ..Self::default()
        }
    }

    pub(crate) fn observe(&mut self, end: &WalkEnd) -> bool {
        let transient = matches!(
            end,
            WalkEnd::Stalled(stall)
                if matches!(stall.why, WalkStallReason::Expired | WalkStallReason::Dropped)
        );
        self.observe_retryable(transient)
    }

    pub(crate) fn observe_receipt(&mut self, receipt: &WalkReceipt) -> bool {
        self.observe_retryable(WalkRetryState::retryable_receipt(receipt))
    }

    /// Dialogue also retries expired assessments, which have no walk receipt.
    pub(crate) fn observe_retryable(&mut self, retryable: bool) -> bool {
        retryable && self.admit(self.max_attempts.saturating_sub(1))
    }

    /// Admits one more recovery action for an ordinary walk failure: no
    /// route, a failed or blocked walk, a route that ended short, or any
    /// stall. User input, cancellation, a safety or permission refusal, an
    /// escape abort and a quest-gate need stay terminal at once; the owner
    /// must not retry them. `budget` is the caller's own bound on actions.
    pub(crate) fn observe_failure(&mut self, end: &WalkEnd, budget: u8) -> bool {
        let ordinary = matches!(
            end,
            WalkEnd::NoPath
                | WalkEnd::Failed
                | WalkEnd::Blocked
                | WalkEnd::RouteEnded
                | WalkEnd::Stalled(_)
        );
        ordinary && self.admit(budget)
    }

    fn admit(&mut self, budget: u8) -> bool {
        if self.actions < budget && !self.pending {
            self.pending = true;
            true
        } else {
            false
        }
    }

    pub(crate) fn pending(&self) -> bool {
        self.pending
    }

    pub(crate) fn action_submitted(&mut self) {
        if self.pending {
            self.actions = self.actions.saturating_add(1);
            self.pending = false;
        }
    }

    pub(crate) fn reset(&mut self) {
        self.actions = 0;
        self.pending = false;
    }

    /// Number of the next request after the initial request.
    pub(crate) fn retry_attempt_number(&self) -> u8 {
        self.actions.saturating_add(2)
    }

    /// Recovery actions already submitted; the owner picks the next kind.
    pub(crate) fn actions(&self) -> u8 {
        self.actions
    }
}

#[derive(Debug, Clone)]
pub struct WalkReceipt {
    pub request_id: u64,
    pub evidence: EvidenceStamp,
    /// Chosen destination for a successful exact Area walk (radius zero,
    /// without a loc), with or without candidates. Other arrivals report None.
    pub goal: Option<WorldTile>,
    pub end: WalkEnd,
    pub blocked: Option<Arc<[nav_planner::contract::ZoneKey]>>,
    pub detail: Option<Arc<str>>,
    pub refusal: Option<WalkRefusal>,
    pub assessment: Option<Arc<crate::combat::risk::RouteAssessment>>,
    pub escape: Option<crate::combat::guard::Escape>,
    /// Effective prerequisite grants captured by host admission. Missing means denied.
    pub prereq_allow: Option<PrereqAllow>,
}
impl WalkReceipt {
    /// Prefer host detail and a typed stall over an applicable model reason,
    /// then a typed refusal. A route assessment is not a reason for early end.
    pub fn failure_detail(&self) -> Option<Arc<str>> {
        self.detail
            .as_ref()
            .map(Arc::clone)
            .or_else(|| match &self.end {
                WalkEnd::Stalled(stall) => Some(Arc::from(format!(
                    "walk stalled ({:?}) at {:?}, aiming at {:?}, tries: {}",
                    stall.why, stall.at, stall.aiming, stall.tries
                ))),
                _ => None,
            })
            .or_else(|| {
                if matches!(&self.end, WalkEnd::RouteEnded) {
                    None
                } else {
                    self.assessment
                        .as_ref()
                        .map(|assessment| Arc::clone(&assessment.reason))
                }
            })
            .or_else(|| {
                self.refusal
                    .as_ref()
                    .map(|refusal| Arc::from(refusal.to_string()))
            })
    }
    /// Return the host or typed detail, with the same fallback used by `into_arrival`.
    pub(crate) fn failure_detail_or_fallback(&self) -> Arc<str> {
        self.failure_detail().unwrap_or_else(|| {
            Arc::from(match &self.end {
                WalkEnd::NoPath => "walk has no route",
                WalkEnd::RouteEnded => "walk route ended before arrival",
                _ => "walk failed",
            })
        })
    }

    /// Accept a walk only when the adapter proved its requested arrival.
    /// Route completion alone does not satisfy an area or reach goal.
    pub fn into_arrival(self) -> Result<EvidenceStamp, ActionError> {
        let failure_detail = self.failure_detail();
        match self.end {
            WalkEnd::Arrived => Ok(self.evidence),
            WalkEnd::UserInput => Err(ActionError::UserInput),
            WalkEnd::NeedsEvidence(gates) => Err(ActionError::NeedsEvidence(gates)),
            WalkEnd::NeedsStep(gates) => Err(ActionError::NeedsStep(gates)),
            WalkEnd::GateRefused(observation) => Err(ActionError::GateRefused(observation)),
            WalkEnd::Cancelled => Err(ActionError::Cancelled),
            WalkEnd::NoPath => Err(ActionError::Blocked(failure_detail.unwrap_or_else(|| {
                static REASON: LazyLock<Arc<str>> =
                    LazyLock::new(|| Arc::from("walk has no route"));
                Arc::clone(&REASON)
            }))),
            WalkEnd::RouteEnded => Err(ActionError::Blocked(failure_detail.unwrap_or_else(|| {
                static REASON: LazyLock<Arc<str>> =
                    LazyLock::new(|| Arc::from("walk route ended before arrival"));
                Arc::clone(&REASON)
            }))),
            WalkEnd::Stalled(_)
            | WalkEnd::Aborted
            | WalkEnd::Refused
            | WalkEnd::Blocked
            | WalkEnd::Failed => Err(ActionError::Blocked(failure_detail.unwrap_or_else(|| {
                static REASON: LazyLock<Arc<str>> = LazyLock::new(|| Arc::from("walk failed"));
                Arc::clone(&REASON)
            }))),
        }
    }
}
#[cfg(test)]
mod walk_receipt_tests {
    use super::*;

    #[test]
    fn route_ended_walk_uses_arrival_failure_instead_of_risk_reason() {
        let risk_reason: Arc<str> = Arc::from("survivable route assessment");
        let assessment = Arc::new(crate::combat::risk::RouteAssessment {
            verdict: crate::combat::risk::Verdict::Survivable,
            bound: 0.0,
            poison_test: crate::combat::risk::PoisonTest::NotNeeded,
            plan: crate::combat::risk::RoutePlan::default(),
            crossings: Box::new([]),
            more: 0,
            supplies: Box::new([]),
            hp_after: 10,
            volley: 0,
            input: crate::combat::risk::RiskInput::default(),
            generation: 1,
            reason: Arc::clone(&risk_reason),
        });
        let receipt = WalkReceipt {
            request_id: 7,
            evidence: EvidenceStamp {
                run: RunKey {
                    slot: 1,
                    run: 1,
                    session: 1,
                },
                tick: 1,
                sequence: 1,
            },
            goal: None,
            end: WalkEnd::RouteEnded,
            blocked: None,
            detail: None,
            refusal: None,
            assessment: Some(assessment),
            escape: None,
            prereq_allow: None,
        };
        assert_eq!(
            receipt
                .assessment
                .as_ref()
                .map(|assessment| assessment.reason.as_ref()),
            Some(risk_reason.as_ref())
        );
        assert_eq!(receipt.failure_detail(), None);
        assert_eq!(
            receipt.into_arrival(),
            Err(ActionError::Blocked(Arc::from(
                "walk route ended before arrival"
            )))
        );
    }
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

/// Where one single interaction request is in the host's dispatch lifecycle
/// ([`ActionContext::interaction_dispatch`]). The host takes queued native
/// work only in an observation pass that may dispatch: connected, not held,
/// the slot's dispatch open (`script_observe.rs:1344-1363`,
/// `SlotScript::take_native_action`). It records the receipt of every
/// interaction it takes in that same pass (`script_observe.rs:1649-1664`).
/// A held pass does not dispatch: it drops queued gameplay instead
/// (`SlotScript::seat_held_tick`, `GameplaySeat::hold` and `held_tick`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InteractionDispatch<'a> {
    /// The host still holds the request: no pass that may dispatch has run
    /// since it was queued. The wait belongs to the host's lifecycle.
    Queued,
    /// The host dropped the request without sending it (a held tick, a
    /// guardian hold, a seat refusal, a session reset), or this action no
    /// longer owns it (cancelled, or superseded by a later request) and has
    /// no receipt for it. No receipt will come.
    Unsent,
    /// The host took the request in a pass that could dispatch it and
    /// settled it: `accepted` is the dispatch result.
    Settled(&'a InteractionReceipt),
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
    pub fn revision(&self) -> ClientRevision {
        self.revision
    }
    pub fn snapshot(&self) -> SnapshotView<'a> {
        self.snapshot
    }
    /// The Wilderness levels the host lends for this frame, when it has a nav
    /// pack.
    pub fn wilderness(&self) -> Option<&'a dyn WildernessLevels> {
        self.wilderness
    }
    /// The host's complete live condition, checked against the frame at entry.
    pub fn condition(&self) -> Option<&api::condition::BotCondition> {
        self.condition.as_ref()
    }
    /// Strongest poison pulse in the selected content, or the saturated bound.
    pub fn poison_unknown_pulse(&self) -> u8 {
        self.poison_unknown_pulse
    }
    pub fn eligible(&self) -> bool {
        self.eligible
    }
    /// The profile's Cure poison setting for this frame.
    pub fn cure_poison(&self) -> crate::survival::CurePoison {
        self.cure_poison
    }
    pub(crate) fn protected_items(&self) -> &[i32] {
        self.protected_items.as_deref().unwrap_or(&[])
    }
    pub fn retained(&mut self) -> &mut RetainedMemory {
        self.retained
    }
    pub fn action_id(&self) -> u64 {
        self.action_id
    }
    pub(crate) fn note_progress(&mut self) {
        if !self.eligible {
            return;
        }
        if let Some(pending) = self.pending_note_progress.as_deref_mut() {
            *pending = Some(self.evidence.run);
        }
    }
}

/// Non-Clone guard. Dropping it revokes host work before local cleanup.
#[must_use]
pub struct ActionHandle<M: NativeMachine> {
    owner: Arc<owner::Owner>,
    machine: RefCell<Option<M>>,
}

/// Revocation only: a Play-owned pair lease can fence queued work without
/// taking the actor's slot lock or accessing its machine.
#[derive(Clone)]
pub struct ActionRevoker(RevokerTarget);
#[derive(Clone)]
enum RevokerTarget {
    Owner(Arc<owner::Owner>),
    Logical(Arc<parking_lot::Mutex<LogicalRevocation>>),
}
struct LogicalRevocation {
    run: RunKey,
    revoked: bool,
    current: Option<ActionRevoker>,
}

/// A lazily allocated revocation link for a logical action whose native owner
/// changes. Installing after a concurrent revoke fences the new owner too.
pub(crate) struct ActionLink(Arc<parking_lot::Mutex<LogicalRevocation>>);
impl ActionLink {
    pub(crate) fn new(current: ActionRevoker) -> Self {
        Self(Arc::new(parking_lot::Mutex::new(LogicalRevocation {
            run: current.run(),
            revoked: false,
            current: Some(current),
        })))
    }
    pub(crate) fn revoker(&self) -> ActionRevoker {
        ActionRevoker(RevokerTarget::Logical(Arc::clone(&self.0)))
    }
    pub(crate) fn revoked(&self) -> bool {
        self.0.lock().revoked
    }
    pub(crate) fn replace(&self, current: Option<ActionRevoker>) {
        let mut state = self.0.lock();
        if state.revoked {
            drop(state);
            if let Some(current) = current {
                current.revoke();
            }
        } else {
            state.current = current;
        }
    }
}
impl ActionRevoker {
    pub fn run(&self) -> RunKey {
        match &self.0 {
            RevokerTarget::Owner(owner) => owner.run,
            RevokerTarget::Logical(state) => state.lock().run,
        }
    }
    pub fn revoke(&self) {
        match &self.0 {
            RevokerTarget::Owner(owner) => owner.revoke(),
            RevokerTarget::Logical(state) => {
                let current = {
                    let mut state = state.lock();
                    state.revoked = true;
                    state.current.take()
                };
                if let Some(current) = current {
                    current.revoke();
                }
            }
        }
    }
}
impl<M: NativeMachine> ActionHandle<M> {
    pub fn revoker(&self) -> ActionRevoker {
        ActionRevoker(RevokerTarget::Owner(Arc::clone(&self.owner)))
    }
}
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ActionError {
    #[error("the native action owner is busy")]
    Busy,
    #[error("the native action is held")]
    Held,
    #[error("this native runtime exhausted its observed-tick emission allowance or owner storage")]
    BudgetExhausted,
    #[error("the native action identity is stale")]
    Stale,
    #[error("the native action was cancelled")]
    Cancelled,
    /// A step adapter was interrupted by manual movement, not owner revocation.
    #[error("manual input interrupted the native action")]
    UserInput,
    #[error("the native action is unavailable: {0}")]
    Unavailable(Arc<str>),
    #[error("the native action failed: {0}")]
    Failed(Arc<str>),
    /// A typed permanent refusal from the shared bank-deposit machine.
    #[error("bank deposit was refused: {0:?}")]
    BankDeposit(crate::bank::ops::DepositFailure),
    /// A hit on the local player closed the dialogue before it finished.
    #[error("combat interrupted the dialogue")]
    CombatInterrupted,
    /// The step cannot continue safely without an explicit operator retry.
    #[error("the native action is blocked: {0}")]
    Blocked(Arc<str>),
    /// The action requires typed quest evidence before it can continue.
    #[error("the native action needs quest evidence: {0:?}")]
    NeedsEvidence(Arc<[QuestGate]>),
    #[error("the native action needs a navigation prerequisite: {0:?}")]
    NeedsStep(Arc<[api::prereq_steps::ActionableGate]>),
    #[error("a navigation crossing refused the account: {0:?}")]
    GateRefused(Arc<api::prereq_knowledge::RefusalObservation>),
    #[error(transparent)]
    Prerequisite(Arc<PrereqRefusal>),
    /// Typed capacity or duplicate refusal from exact ground-item Take.
    #[error("ground-item Take was refused: {0:?}")]
    GroundTake(api::ground_take::GroundRefusal),
    /// A typed permanent refusal of a player-combat request at `begin`
    /// (PVP-DESIGN-1026 §2.1).
    #[error("player combat was refused: {0:?}")]
    PvpRefused(crate::combat::PvpRefusal),
    /// Invalid bounded candidate-goal request, rejected before host admission.
    #[error("walk goals were refused: {0}")]
    WalkGoals(WalkGoalsError),
    /// A typed server-ordered admission refusal, before host dispatch.
    #[error(transparent)]
    ServerOrdered(#[from] api::interact::server_ordered::ServerOrderedError),
}

/// A poll-time admission that has used this tick's event must retry later.
/// Callers commit request state only after this returns `Ready(Ok(_))`.
pub fn defer_budget<T>(result: Result<T, ActionError>) -> Poll<Result<T, ActionError>> {
    match result {
        Err(ActionError::BudgetExhausted) => Poll::Pending,
        result => Poll::Ready(result),
    }
}

/// What a machine did when it resumed after a guardian hold
/// ([`NativeMachine::after_hold`]). [`NativeActions::poll`] logs it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoldResume {
    /// The request needs nothing: its goal is already met, or the host still
    /// holds it, or the machine has none in flight. The text says which.
    Kept(&'static str),
    /// The machine sent its request again, once. The text names the request.
    Reissued(&'static str),
    /// The host dropped the request unsent, and the machine ends so that its
    /// owner, which holds the goal, checks the goal before it sends another.
    /// The text names the request.
    Returned(&'static str),
}

/// The one `info` line for a resume after a guardian hold, logged by whoever
/// consumed it ([`ActionContext::take_hold_resume`]).
pub fn log_hold_resume(action: &str, resume: HoldResume) {
    let (what, text) = match resume {
        HoldResume::Kept(text) => ("kept its request", text),
        HoldResume::Reissued(text) => ("re-issued its request once", text),
        HoldResume::Returned(text) => ("returned its dropped request to its owner", text),
    };
    #[cfg(test)]
    RESUME_LOG.with(|lines| {
        lines.borrow_mut().push(format!(
            "native action {action} resumed after a guardian hold and {what}: {text}"
        ));
    });
    api::host_log!(
        api::hostlog::Category::ScriptLifecycle,
        api::hostlog::Level::Info,
        "native action {action} resumed after a guardian hold and {what}: {text}"
    );
}

#[cfg(test)]
thread_local! {
    static RESUME_LOG: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// The resume lines this test thread has logged ([`log_hold_resume`]).
#[cfg(test)]
pub(crate) fn take_resume_log() -> Vec<String> {
    RESUME_LOG.with(|lines| std::mem::take(&mut *lines.borrow_mut()))
}

pub trait NativeMachine: Send + 'static {
    type Args: Send;
    type Output: Send;
    /// A machine that keeps a request in flight across a guardian hold sets
    /// this and implements [`Self::after_hold`]. The hold drops queued host
    /// work and the guardian sends its own interactions, so such a request can
    /// be lost, or taken over, with no failure the machine would see. The
    /// owner layer ([`NativeActions::poll`]) then leaves the machine alone for
    /// the first observed tick after the hold, so an in-flight server effect
    /// can land, and calls [`Self::after_hold`] once on the next.
    const RESUMES_AFTER_HOLD: bool = false;
    fn begin(args: Self::Args, cx: &mut ActionContext<'_>) -> Result<Self, ActionError>
    where
        Self: Sized;
    fn poll(&mut self, cx: &mut ActionContext<'_>) -> Poll<Result<Self::Output, ActionError>>;
    /// Called once, before the machine's next [`Self::poll`], on the first
    /// observed tick that follows the settle tick of a guardian hold. The
    /// machine re-checks its own goal first (the page is open, the item count
    /// rose, the receipt arrived) and re-issues its request at most once only
    /// when the goal is not met and the request was lost or taken over. A
    /// machine that does not know the goal (it is its owner's) does not send
    /// again: it returns [`HoldResume::Returned`] and its next poll ends it,
    /// so the owner checks the goal first. A `BudgetExhausted` error retries
    /// on the next tick.
    fn after_hold(&mut self, _cx: &mut ActionContext<'_>) -> Result<HoldResume, ActionError> {
        Ok(HoldResume::Kept(
            "this machine keeps no request across a hold",
        ))
    }
    /// Cancel an unfinished machine before abandoning it.
    /// After cancellation, the owner must not call `poll` or `after_hold`.
    /// Nested owners obey the same rule as the native action slot.
    fn cancel(&mut self) {}
    fn interrupt(&mut self) {}
    fn prayer_cleanup(&self) -> crate::combat::RaisedPrayers {
        crate::combat::RaisedPrayers::empty()
    }
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
#[cfg(test)]
mod walk_permission_tests {
    use super::*;

    #[test]
    fn walk_bits_are_additive_except_for_a_walk_local_forbid() {
        let cases = [
            (WalkBit::Inherit, false, false, false),
            (WalkBit::Inherit, true, false, true),
            (WalkBit::Inherit, false, true, true),
            (WalkBit::Allow, false, false, true),
            (WalkBit::Allow, true, false, true),
            (WalkBit::Forbid, false, false, false),
            (WalkBit::Forbid, true, true, false),
        ];
        for (bit, global, script, expected) in cases {
            assert_eq!(bit.resolve(global, script), expected);
        }
        assert_eq!(
            WalkOptions::default(),
            WalkOptions {
                allow_teleports: WalkBit::Inherit,
                allow_wilderness: WalkBit::Inherit,
                allow_danger_zones: WalkBit::Inherit,
                allow_prereq_dialogue: WalkBit::Inherit,
                allow_quest_start: WalkBit::Inherit,
            }
        );
    }

    #[test]
    fn walk_allow_defaults_enable_food_and_escape() {
        let allow = WalkAllow::default();
        assert!(allow.prayer);
        assert!(allow.food);
        assert!(allow.escape);
    }

    #[test]
    fn candidate_goal_limit_refuses_instead_of_truncating() {
        let tile = WorldTile {
            x: 1,
            z: 2,
            level: 0,
        };
        assert!(matches!(
            WalkGoals::new(Vec::new()),
            Err(WalkGoalsError::Empty)
        ));
        let tiles = |count| {
            (0..count)
                .map(|x| WorldTile {
                    x: x as i32,
                    ..tile
                })
                .collect()
        };
        let maximum = WalkGoals::new(tiles(WalkGoals::MAX)).unwrap();
        assert_eq!(maximum.as_slice().len(), WalkGoals::MAX);
        assert_eq!(maximum.preferred(), maximum.as_slice());
        assert!(maximum.fallback().is_empty());
        assert!(matches!(
            WalkGoals::new(tiles(WalkGoals::MAX + 1)),
            Err(WalkGoalsError::TooMany { count, limit })
                if count == WalkGoals::MAX + 1 && limit == WalkGoals::MAX
        ));
    }

    #[test]
    fn candidate_tiers_share_one_unique_bound_without_inline_growth() {
        let tile = |x| WorldTile { x, z: 0, level: 0 };
        let goals = WalkGoals::with_preferred(
            vec![tile(1), tile(2), tile(2), tile(3)],
            &[tile(3), tile(3), tile(4)],
        )
        .unwrap();
        assert_eq!(goals.preferred(), &[tile(3), tile(4)]);
        assert_eq!(goals.fallback(), &[tile(1), tile(2)]);
        assert_eq!(goals.as_slice().len(), 4);
        assert_eq!(
            std::mem::size_of::<WalkGoals>(),
            2 * std::mem::size_of::<usize>()
        );
        let maximum = (0..WalkGoals::MAX).map(|x| tile(x as i32)).collect();
        assert!(matches!(
            WalkGoals::with_preferred(maximum, &[tile(WalkGoals::MAX as i32)]),
            Err(WalkGoalsError::TooMany { count, limit })
                if count == WalkGoals::MAX + 1 && limit == WalkGoals::MAX
        ));
        let deduplicated = WalkGoals::new(vec![tile(1); WalkGoals::MAX + 1]).unwrap();
        assert_eq!(deduplicated.as_slice(), &[tile(1)]);
        assert_eq!(
            WalkGoalsError::TooMany {
                count: 1025,
                limit: 1024
            }
            .to_string(),
            "1025 unique candidate goals exceed the limit of 1024"
        );
    }

    #[test]
    fn refusal_variants_preserve_distinct_typed_kinds() {
        let refusals = [
            WalkRefusal::SearchIncomplete {
                nodes_settled: Some(524_288),
            },
            WalkRefusal::SearchIncomplete {
                nodes_settled: None,
            },
            WalkRefusal::NoRouteWithinBounds {
                tried: 2,
                last: Some(crate::combat::risk::Verdict::Unsurvivable),
            },
            WalkRefusal::Unsurvivable,
            WalkRefusal::PoisonUnsettled {
                waited: 18,
                cause: api::condition::WindowReset::Modal,
            },
            WalkRefusal::Unknown(crate::combat::risk::UnknownWhy::MissingFacts),
            WalkRefusal::EscapeInProgress,
            WalkRefusal::RouteTooLarge {
                required_bytes: 64 * 1024 * 1024 + 1,
                maximum_bytes: 64 * 1024 * 1024,
            },
        ];
        for left in 0..refusals.len() {
            for right in left + 1..refusals.len() {
                assert_ne!(refusals[left], refusals[right]);
            }
        }
        assert_eq!(
            WalkRefusal::Unknown(crate::combat::risk::UnknownWhy::MissingFacts).to_string(),
            "route risk is unknown: required combat facts are unavailable"
        );
    }
    #[test]
    fn poison_unsettled_display_names_wait_and_reset_cause() {
        assert_eq!(
            WalkRefusal::PoisonUnsettled {
                waited: 18,
                cause: api::condition::WindowReset::Modal,
            }
            .to_string(),
            "poison state remained unsettled after 18 observed ticks (Modal)",
        );
    }

    #[test]
    fn aborted_walk_maps_to_blocked_with_the_escape_detail() {
        let receipt = WalkReceipt {
            goal: None,
            request_id: 7,
            evidence: EvidenceStamp {
                run: RunKey {
                    slot: 1,
                    run: 1,
                    session: 1,
                },
                tick: 1,
                sequence: 1,
            },
            end: WalkEnd::Aborted,
            blocked: None,
            detail: Some(Arc::from("retreated from the danger zone")),
            refusal: None,
            assessment: None,
            escape: None,
            prereq_allow: None,
        };
        assert_eq!(
            receipt.into_arrival(),
            Err(ActionError::Blocked(Arc::from(
                "retreated from the danger zone"
            )))
        );
    }
    #[test]
    fn incomplete_search_stays_within_the_compat_blocked_result() {
        let detail: Arc<str> = Arc::from(
            "search_result=BudgetExhausted nodes_settled=unavailable (incomplete search)",
        );
        let receipt = WalkReceipt {
            goal: None,
            request_id: 7,
            evidence: EvidenceStamp {
                run: RunKey {
                    slot: 1,
                    run: 1,
                    session: 1,
                },
                tick: 1,
                sequence: 1,
            },
            end: WalkEnd::Refused,
            blocked: None,
            detail: Some(Arc::clone(&detail)),
            refusal: Some(WalkRefusal::SearchIncomplete {
                nodes_settled: None,
            }),
            assessment: None,
            escape: None,
            prereq_allow: None,
        };
        assert_eq!(
            receipt.into_arrival(),
            Err(ActionError::Blocked(detail)),
            "script callers still receive the compat Blocked error variant",
        );
    }

    #[test]
    fn typed_refusal_supplies_a_reason_when_host_detail_is_absent() {
        let receipt = WalkReceipt {
            goal: None,
            request_id: 7,
            evidence: EvidenceStamp {
                run: RunKey {
                    slot: 1,
                    run: 1,
                    session: 1,
                },
                tick: 1,
                sequence: 1,
            },
            end: WalkEnd::Refused,
            blocked: None,
            detail: None,
            refusal: Some(WalkRefusal::EscapeInProgress),
            assessment: None,
            escape: None,
            prereq_allow: None,
        };
        assert_eq!(
            receipt.failure_detail().as_deref(),
            Some("escape in progress")
        );
    }

    #[test]
    fn prepared_permissions_are_frozen_from_the_native_settings_bag() {
        let bag: SettingsBag = serde_json::from_value(serde_json::json!({
            "allow_teleports": true,
            "allow_wilderness": false,
            "allow_danger_zones": true,
            "allow_prereq_dialogue": true,
            "allow_quest_start": false,
        }))
        .unwrap();
        let roundtrip: SettingsBag =
            serde_json::from_slice(&serde_json::to_vec(&bag).unwrap()).unwrap();
        let config = PreparedConfig::new(
            CompiledId("Receiver"),
            1,
            7,
            Arc::new(roundtrip.clone()),
            (),
        );
        assert_eq!(
            config.walk_permissions(),
            WalkPermissions {
                allow_teleports: true,
                allow_wilderness: false,
                allow_danger_zones: true,
                allow_prereq_dialogue: true,
                allow_quest_start: false,
            }
        );
        assert_eq!(config.bag(), &roundtrip);

        let legacy = PreparedConfig::new(CompiledId("Receiver"), 1, 8, Arc::default(), ());
        assert_eq!(legacy.walk_permissions(), WalkPermissions::default());
    }

    #[test]
    fn gatherer_permissions_use_camel_case_keys() {
        let bag: SettingsBag = serde_json::from_value(serde_json::json!({
            "allowTeleports": true,
            "allowWilderness": true,
            "allowDangerZones": false,
            "allowPrereqDialogue": false,
            "allowQuestStart": true,
        }))
        .unwrap();
        let config = PreparedConfig::new(CompiledId("Gatherer"), 4, 1, Arc::new(bag), ());
        assert_eq!(
            config.walk_permissions(),
            WalkPermissions {
                allow_teleports: true,
                allow_wilderness: true,
                allow_danger_zones: false,
                allow_prereq_dialogue: false,
                allow_quest_start: true,
            }
        );
    }
}
#[cfg(test)]
mod defer_budget_tests {
    use super::*;

    #[test]
    fn defer_budget_only_defers_budget_exhaustion() {
        assert_eq!(defer_budget(Ok(17)), Poll::Ready(Ok(17)));
        assert_eq!(
            defer_budget::<()>(Err(ActionError::BudgetExhausted)),
            Poll::Pending
        );

        let errors = [
            ActionError::Busy,
            ActionError::Held,
            ActionError::Stale,
            ActionError::Cancelled,
            ActionError::UserInput,
            ActionError::Unavailable(Arc::from("unavailable")),
            ActionError::Failed(Arc::from("failed")),
            ActionError::BankDeposit(crate::bank::ops::DepositFailure::FullStack),
            ActionError::Blocked(Arc::from("blocked")),
            ActionError::GroundTake(api::ground_take::GroundRefusal::Capacity),
            ActionError::NeedsEvidence(Arc::<[QuestGate]>::from([])),
        ];
        for error in errors {
            assert_eq!(
                defer_budget::<()>(Err(error.clone())),
                Poll::Ready(Err(error))
            );
        }
    }
}
