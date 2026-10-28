//! [`OperatorSession`]: the production operator lifecycle shared by the
//! panel and the TUI. It owns the unlocked vault, the [`Play`], fleet
//! membership and the logout latch, the selected bot, the per-slot surface
//! IO, pending removals, the polled status rows and operation results.
//!
//! Host-play stays authoritative for worker existence, connection and
//! readiness, the login queue, script execution and cancellation; this
//! module only decides *when* to ask it for something and reports what the
//! host then observably did. Nothing here blocks: removal is a polled clean
//! logout bounded by [`SLOT_REMOVE_TIMEOUT`], and workers are joined only
//! once finished.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use api::host_log;
use api::hostlog::{Category, Level, Source};
use api::identity::SessionKey;
use api::lifecycle::Intent;
use host::{CapacityEval, FrameBuf, SlotInput};
use host_play::{InstancePermit, Play, SlotArm, SlotStatus, WalkArms};
use vault::{Profile, ProfileSettings, RandomEventPolicy, Vault, VaultChange};

use crate::capacity::{CapacityNotice, CapacityWarning, RenderPolicyLabel};
use crate::fleet::Fleet;
use crate::operations::{
    write_op, ActionKind, OpChange, OperationBook, OperationId, OperationReport, Outcome,
};
use crate::profile_saves::{SaveBook, SaveRecord, SaveResult, SaveSettled, WriteFailure};
use crate::profiles::{ProfileAuditBefore, ProfileAuditKind, ProfileWriter, Written};
use crate::resources::{ResourceView, Resources};
use crate::scripts::{
    LiveDelivery, LiveSettings, ScriptSettingsLog, SettingsResult, SettingsWrite,
};
use crate::surface::SlotSurface;
use crate::views::{FleetView, Inputs, QueuePlace, Views};

/// Clean-logout window a connected member gets on removal before its worker
/// is stopped regardless.
pub(crate) const SAVING: &str = "profile is still saving";

pub const SLOT_REMOVE_TIMEOUT: Duration = Duration::from_secs(10);

/// Prefer `last` when it is still in `names`; otherwise the first name.
/// Load all uses this when nothing is selected, matching MultiBox-on.
pub fn pick_focus(names: &[String], last: Option<&str>) -> Option<String> {
    if let Some(last) = last {
        if names.iter().any(|n| n == last) {
            return Some(last.to_string());
        }
    }
    names.first().cloned()
}

/// A removal is owned by the exact arm that received its clean-logout
/// request. `io` stays here while that worker drains so re-adding the member
/// reattaches the same surface devices without a second client lifetime.
struct PendingRemoval<Io> {
    started: Instant,
    arm: Arc<SlotArm>,
    io: Option<Io>,
    op: OperationId,
}

/// A spawn waiting for the previous worker of the same name to exit (a
/// removed member loaded again while its stopped worker is still ending).
/// The surface IO stays in `slots`; a Log in issued meanwhile replaces the
/// arm, so the new worker starts with the latest login intent.
struct DeferredSpawn {
    profile: Profile,
    input: Option<Arc<SlotInput>>,
    mailbox: Option<Arc<FrameBuf>>,
    arm: Option<Arc<SlotArm>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TransitionKind {
    SlotUp,
    LoginError,
    Ingame,
    Scene,
    Welcome,
    WelcomeFailure,
}

/// One observed status change since the previous poll.
#[derive(Debug, PartialEq, Eq)]
pub struct Transition {
    kind: TransitionKind,
    error: Option<String>,
    state: Option<i32>,
    text: Option<String>,
}

#[allow(non_upper_case_globals)]
impl Transition {
    pub const SlotUp: Self = Self {
        kind: TransitionKind::SlotUp,
        error: None,
        state: None,
        text: None,
    };
    pub const Ingame: Self = Self {
        kind: TransitionKind::Ingame,
        error: None,
        state: None,
        text: None,
    };

    /// Scene-state change.
    pub fn scene(state: i32) -> Self {
        Self {
            kind: TransitionKind::Scene,
            error: None,
            state: Some(state),
            text: None,
        }
    }

    /// Login error text.
    pub fn login_error(error: impl Into<String>) -> Self {
        Self {
            kind: TransitionKind::LoginError,
            error: Some(error.into()),
            state: None,
            text: None,
        }
    }

    /// Welcome-modal line.
    pub fn welcome(text: impl Into<String>) -> Self {
        Self {
            kind: TransitionKind::Welcome,
            error: None,
            state: None,
            text: Some(text.into()),
        }
    }

    /// Welcome-modal dismiss failure.
    pub fn welcome_failure(error: impl Into<String>) -> Self {
        Self {
            kind: TransitionKind::WelcomeFailure,
            error: Some(error.into()),
            state: None,
            text: None,
        }
    }

    pub(crate) fn kind(&self) -> TransitionKind {
        self.kind
    }

    /// Login or welcome-failure text, when this carries one.
    pub fn error(&self) -> Option<&str> {
        match self.kind {
            TransitionKind::LoginError | TransitionKind::WelcomeFailure => self.error.as_deref(),
            _ => None,
        }
    }

    /// Scene state, when this is a scene change.
    pub fn state(&self) -> Option<i32> {
        self.state
    }

    /// Welcome line, when this is a welcome.
    pub fn text(&self) -> Option<&str> {
        match self.kind {
            TransitionKind::Welcome => self.text.as_deref(),
            _ => None,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct SlotTransition {
    pub(crate) slot: String,
    pub(crate) transition: Transition,
}

impl SlotTransition {
    /// The slot that changed.
    pub fn slot(&self) -> &str {
        &self.slot
    }

    /// What changed.
    pub fn transition(&self) -> &Transition {
        &self.transition
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SelectionKind {
    Unchanged,
    Changed,
}

/// Selection change result. `previous` is the bot that lost selection.
#[derive(Debug, PartialEq, Eq)]
pub struct Selection {
    kind: SelectionKind,
    previous: Option<String>,
}

#[allow(non_upper_case_globals)]
impl Selection {
    pub const Unchanged: Self = Self {
        kind: SelectionKind::Unchanged,
        previous: None,
    };

    pub(crate) fn changed(previous: Option<String>) -> Self {
        Self {
            kind: SelectionKind::Changed,
            previous,
        }
    }

    /// Whether selection stayed the same.
    pub fn is_unchanged(&self) -> bool {
        self.kind == SelectionKind::Unchanged
    }

    /// The bot that lost selection, when selection changed.
    pub fn previous(&self) -> Option<&str> {
        match self.kind {
            SelectionKind::Changed => self.previous.as_deref(),
            SelectionKind::Unchanged => None,
        }
    }
}

/// What a removal did to the selection.
#[derive(Debug, PartialEq, Eq)]
pub struct Removal {
    pub(crate) op: OperationId,
    /// The removed member was selected and its neighbour is now selected.
    pub(crate) reselected: Option<String>,
    /// The removed member was selected and no member remains to select.
    pub(crate) selection_cleared: bool,
}

impl Removal {
    /// The removal's operation.
    pub fn op(&self) -> OperationId {
        self.op
    }

    /// The neighbour now selected, when the removed member was selected.
    pub fn reselected(&self) -> Option<&str> {
        self.reselected.as_deref()
    }

    /// Whether the removed member was selected and no member remains.
    pub fn selection_cleared(&self) -> bool {
        self.selection_cleared
    }
}

enum ScriptStartKind {
    Compiled {
        id: script::CompiledId,
        bag: serde_json::Map<String, serde_json::Value>,
    },
    #[cfg(feature = "js")]
    Load {
        js: String,
        shape: script_js::LoadShape,
        bag: Option<serde_json::Map<String, serde_json::Value>>,
        siblings: Vec<(String, String)>,
    },
}

/// A script Start request. Native configuration/factory preparation and Load
/// isolate setup both settle asynchronously through the host.
pub struct ScriptStart(ScriptStartKind);

impl ScriptStart {
    /// Start a compiled card.
    pub fn compiled(
        id: crate::scripts::CompiledCardId,
        bag: serde_json::Map<String, serde_json::Value>,
    ) -> Self {
        let id = id.to_script();
        Self(ScriptStartKind::Compiled { id, bag })
    }

    #[cfg(feature = "js")]
    /// Start a loaded JS/TS card.
    pub fn load(
        js: String,
        shape: crate::scripts::LoadShape,
        bag: Option<serde_json::Map<String, serde_json::Value>>,
        siblings: Vec<(String, String)>,
    ) -> Self {
        let shape = shape.to_script();
        Self(ScriptStartKind::Load {
            js,
            shape,
            bag,
            siblings,
        })
    }

    pub(crate) fn is_compiled(&self) -> bool {
        matches!(self.0, ScriptStartKind::Compiled { .. })
    }
}

/// A Load Start whose isolate setup settled (or was cancelled by removal:
/// `outcome == None`). Front ends consume these to commit assignments and
/// record load diagnostics.
#[derive(Debug, PartialEq, Eq)]
pub struct StartSettled {
    pub(crate) slot: String,
    pub(crate) op: OperationId,
    pub(crate) outcome: Option<script::StartOutcome>,
}

impl StartSettled {
    /// The slot whose Start settled.
    pub fn slot(&self) -> &str {
        &self.slot
    }

    /// The Start operation.
    pub fn op(&self) -> OperationId {
        self.op
    }

    /// Isolate setup outcome, or `None` if removal cancelled it.
    #[cfg(any(test, feature = "test-support"))]
    pub fn outcome(&self) -> Option<&script::StartOutcome> {
        self.outcome.as_ref()
    }
}

/// What a running slot learns once a profile write is durable. Writes of
/// one profile saved in one commit each settle and deliver their own
/// setting; only a later one with the same setting replaces an earlier
/// one's.
#[derive(PartialEq)]
pub struct ArmMirror(ArmMirrorKind);

/// A script assignment command's profile write after it settles.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ScriptAssignmentWrite {
    op: OperationId,
    result: SettingsResult,
}

impl ScriptAssignmentWrite {
    pub(crate) fn op(&self) -> OperationId {
        self.op
    }

    pub(crate) fn result(&self) -> &SettingsResult {
        &self.result
    }
}

impl std::fmt::Debug for ArmMirror {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&self.0, f)
    }
}

#[derive(Debug, PartialEq)]
enum ArmMirrorKind {
    /// No live effect.
    None,
    /// Signals the script coordinator after the profile assignment is durable.
    ScriptAssignment,
    AutoLogin(bool),
    Guardian {
        random_events: bool,
        lamp_skill: String,
        lamp_auto: bool,
    },
    RandomEventPolicy {
        event: String,
        policy: RandomEventPolicy,
    },
    /// The Cure poison setting's post-write mirror.
    Survival {
        new: crate::profile_draft::CurePoison,
    },
    /// Memory mode (`ProfileSettings.lowmem`): queued for the next login.
    /// Set when the toggle is issued and re-affirmed once durable; a failed
    /// write resets the arm to the restored durable value. No client state
    /// changes until the worker begins its next handshake.
    Memory(bool),
    /// Handshake-time settings (password, world) for the next login, and
    /// the whole bag for the run captured at edit time when the save
    /// changed its profile-global settings (the clue duel partner).
    Remember(Option<LiveSettings>),
    /// A card's parameters: posted to the run captured at edit time, if
    /// any, and reported through [`OperatorSession::take_settings_writes`].
    ScriptSettings {
        /// The card's identity key (its bag in the profile).
        card: String,
        live: Option<LiveSettings>,
    },
    /// Native drafts are prepared off-pump before staging or writing the row.
    NativeSettings {
        id: script::CompiledId,
        bag: Arc<serde_json::Map<String, serde_json::Value>>,
        live: Option<LiveSettings>,
        /// Only an accepted unassigned-target copy originates assignment intent.
        assign: bool,
    },
}

impl ArmMirror {
    pub fn none() -> Self {
        Self(ArmMirrorKind::None)
    }

    pub(crate) fn script_assignment() -> Self {
        Self(ArmMirrorKind::ScriptAssignment)
    }

    pub fn auto_login(on: bool) -> Self {
        Self(ArmMirrorKind::AutoLogin(on))
    }

    pub fn guardian(random_events: bool, lamp_skill: String, lamp_auto: bool) -> Self {
        Self(ArmMirrorKind::Guardian {
            random_events,
            lamp_skill,
            lamp_auto,
        })
    }

    pub fn random_event_policy(event: String, policy: crate::GuardianPolicy) -> Self {
        let policy = policy.to_vault();
        Self(ArmMirrorKind::RandomEventPolicy { event, policy })
    }

    pub fn survival(new: crate::CurePoison) -> Self {
        Self(ArmMirrorKind::Survival { new })
    }

    pub fn memory(lowmem: bool) -> Self {
        Self(ArmMirrorKind::Memory(lowmem))
    }

    pub fn remember(live: Option<LiveSettings>) -> Self {
        Self(ArmMirrorKind::Remember(live))
    }

    pub fn script_settings(card: String, live: Option<LiveSettings>) -> Self {
        Self(ArmMirrorKind::ScriptSettings { card, live })
    }
    pub(crate) fn script_settings_identity(&self) -> Option<String> {
        match &self.0 {
            ArmMirrorKind::ScriptSettings { card, .. } => Some(card.clone()),
            ArmMirrorKind::NativeSettings { id, .. } => Some(script::compiled_identity_key(*id)),
            _ => None,
        }
    }

    pub fn native_settings(
        id: crate::scripts::CompiledCardId,
        bag: Arc<serde_json::Map<String, serde_json::Value>>,
        live: Option<LiveSettings>,
        assign: bool,
    ) -> Self {
        let id = id.to_script();
        Self(ArmMirrorKind::NativeSettings {
            id,
            bag,
            live,
            assign,
        })
    }

    pub(crate) fn is_native_settings(&self) -> bool {
        matches!(&self.0, ArmMirrorKind::NativeSettings { .. })
    }

    /// Whether this mirror, on a later write committed together with
    /// `earlier`'s, sets the same setting, so `earlier` need not settle or
    /// reach the slot on its own. Parameters replace the same card's, a
    /// later assignment replaces an earlier assignment, and a profile save
    /// replaces earlier handshake settings and partner: the later edit
    /// captured the current run, if any.
    fn replaces(&self, earlier: &Self) -> bool {
        match (&self.0, &earlier.0) {
            (ArmMirrorKind::AutoLogin(_), ArmMirrorKind::AutoLogin(_))
            | (ArmMirrorKind::Guardian { .. }, ArmMirrorKind::Guardian { .. })
            | (ArmMirrorKind::Survival { .. }, ArmMirrorKind::Survival { .. })
            | (ArmMirrorKind::Remember(_), ArmMirrorKind::Remember(_))
            | (ArmMirrorKind::Memory(_), ArmMirrorKind::Memory(_))
            | (ArmMirrorKind::ScriptAssignment, ArmMirrorKind::ScriptAssignment) => true,
            (
                ArmMirrorKind::RandomEventPolicy { event: later, .. },
                ArmMirrorKind::RandomEventPolicy { event, .. },
            ) => later == event,
            (
                ArmMirrorKind::ScriptSettings { card: later, .. },
                ArmMirrorKind::ScriptSettings { card, .. },
            ) => later == card,
            _ => false,
        }
    }
}

struct PendingWrite {
    label: &'static str,
    mirror: ArmMirror,
    member: String,
    copy_start_hold: Option<native_settings::NativeCopyHold>,
    copy_abandoned: bool,
}

/// The memory mode applied at the last login against the queued setting.
/// Automatic socket reconnects keep that mode; a clean logout permits a change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryNotice {
    /// The slot's detail mode as the login handshake sent it.
    pub login_lowmem: bool,
    /// The effective setting for the next handshake (profile or session override).
    pub desired_lowmem: bool,
    /// A Relog-now is in flight: logout issued or its login half queued.
    pub relog_pending: bool,
    /// Whether the slot currently has a connected session to relog.
    pub connected: bool,
    /// The host's handshake gate: a dirty reconnect keeps the applied mode.
    pub login_applies_memory: bool,
}

impl MemoryNotice {
    pub const NEXT_LOGIN_NOTE: &'static str = "The entire memory mode applies at the next login.";

    /// Whether a different mode is queued for the next login.
    pub fn differs(self) -> bool {
        self.login_lowmem != self.desired_lowmem
    }

    pub fn can_relog(self) -> bool {
        self.connected && self.differs() && !self.relog_pending
    }

    pub fn mode_name(lowmem: bool) -> &'static str {
        if lowmem {
            "lowmem"
        } else {
            "highmem"
        }
    }

    /// Shared panel/TUI notice, including the queued action's disabled state.
    pub fn notice_text(self) -> &'static str {
        if self.relog_pending {
            "Relog queued — login follows once logged out."
        } else if !self.connected {
            if !self.login_applies_memory {
                return "Connection dropped — Log in keeps the current mode. Relog after connecting to apply the queued mode.";
            }
            "Applies at the next Log in."
        } else {
            "Applies at the next login. Relog now to apply it."
        }
    }

    /// Applied mode first; a queued setting must never look like a live switch.
    pub fn status_text(lowmem: bool, notice: Option<Self>) -> std::borrow::Cow<'static, str> {
        match notice {
            Some(n) if n.differs() => std::borrow::Cow::Owned(format!(
                "{} ({} {})",
                Self::mode_name(n.login_lowmem),
                if !n.connected && !n.login_applies_memory {
                    "queued"
                } else {
                    "next login"
                },
                Self::mode_name(n.desired_lowmem)
            )),
            Some(n) => std::borrow::Cow::Borrowed(Self::mode_name(n.login_lowmem)),
            None => std::borrow::Cow::Borrowed(Self::mode_name(lowmem)),
        }
    }
}

mod commands;
mod fleet;
mod fleet_plan;
mod form;
mod loadouts;
pub(crate) mod map;
mod native_settings;
pub(crate) mod prefs;
mod scripts;
mod vault_rows;
mod world_switch;
pub(crate) use commands::StagedProfileWrite;
pub use commands::{GuardianEdit, SubmitError, Submitted, SurvivalEdit, Targets};
pub use map::{GameInputEvent, MapDesk};
pub(crate) use native_settings::NativeCopyCancellation;
use native_settings::{PendingDelivery, PendingPreparation};
pub use prefs::{DrawPolicy, Notice, SettingEvent, SettingEventState, RENDER_ALL_WARNING_ACK_KEY};

pub struct OperatorSession<Io> {
    vault: Option<Vault>,
    play: Option<Play>,
    fleet: Fleet,
    selected: Option<String>,
    slots: HashMap<String, Io>,
    removals: HashMap<String, PendingRemoval<Io>>,
    /// Spawns queued behind a still-stopping worker, by slot.
    deferred: HashMap<String, DeferredSpawn>,
    statuses: Vec<SlotStatus>,
    /// The next poll's rows; swapped with `statuses` so both keep their
    /// buffers and a steady-state poll allocates nothing.
    polled: Vec<SlotStatus>,
    transitions: Vec<SlotTransition>,
    script_lines: Vec<(String, String)>,
    operations: OperationBook,
    /// Durable vault writes, created with the first write.
    writer: Option<ProfileWriter>,
    writes: HashMap<OperationId, PendingWrite>,
    /// The durable (last committed) value of every profile with a queued
    /// write; `None` when it has no durable row yet (a first save or a
    /// rename target). Spawns and arms read this, never the staged vault:
    /// an unsaved edit reaches a slot only through its post-write mirror.
    durable: HashMap<String, Option<Profile>>,
    #[cfg(any(test, feature = "test-support"))]
    write_gate: Arc<std::sync::Mutex<()>>,
    /// The newest write per profile: only its failure restores the durable
    /// value, so an older failure cannot undo a newer staged edit.
    latest_write: HashMap<String, OperationId>,
    /// Tombstones retained across removal/recreation fence preparation replies.
    profile_edits: HashMap<String, OperationId>,
    /// Only edits of the same card supersede its pending preparation/delivery.
    native_edits: HashMap<(String, String), OperationId>,
    preparations: HashMap<OperationId, PendingPreparation>,
    deliveries: Vec<PendingDelivery>,
    write_failures: Vec<WriteFailure>,
    /// Profile-form saves in flight and settled ones not yet taken.
    saves: SaveBook,
    /// Settled script-parameter writes, drained by the script coordinator.
    settings_writes: Vec<SettingsWrite>,
    /// Audit metadata for queued script-setting writes.
    script_settings_logs: HashMap<OperationId, ScriptSettingsLog>,

    /// Command-backed script assignment writes, drained by the script coordinator.
    script_assignment_writes: VecDeque<ScriptAssignmentWrite>,
    /// Load Starts whose setup has not settled, by slot.
    starts: HashMap<String, OperationId>,
    settled_starts: Vec<StartSettled>,
    spawn_workers: bool,
    #[cfg(any(test, feature = "test-support"))]
    bypass_asset_startup: bool,
    /// Fleet rows and the selected slot's detail, refreshed by each poll.
    views: Views,
    /// Start-all / marked-Start places fed into the single row derivation.
    start_places: HashMap<String, QueuePlace>,
    /// Operator Relog-nows pending a different owner-authorized session.
    mem_relog: HashMap<String, Option<SessionKey>>,
    /// The bound server's public worlds, read once when the play starts
    /// (`None` on a one-world server): the Switch world choices.
    public_worlds: Option<Arc<host_play::public_worlds::PublicWorlds>>,
    /// Operation outcomes taken by the last poll (reused buffer).
    op_changes: Vec<OpChange>,
    /// Reused buffer for operation log lines.
    op_line: String,
    /// The one process resource meter; its OS probe runs on its own thread.
    resources: Resources,
    /// The front end's walk arms: a member with an armed WalkTo route is
    /// walking (script walks are on the host rows).
    /// Per-slot frame state the slot threads share: walk arms, published
    /// snapshots, step latch.
    frames: Arc<crate::frames::FrameShared>,
    #[cfg(feature = "live-harness")]
    live_runner: crate::live::RunnerHandle,
    /// Shared preference applied whenever a Play is installed and whenever
    /// either front end updates its projection.
    pause_script_on_manual_walk_abort: bool,
    walk_globals: crate::walk_permissions::WalkGlobals,
    walk_globals_store: Option<host_play::local_state::Preferences>,
    /// The process local-state service the front end's composition root
    /// opened; every `Play` this session adopts uses it, so the process has
    /// one writer for its state files.
    local_state: Option<Arc<host_play::local_state::LocalStateService>>,
    /// The preferences handle the per-frame snapshot read uses, made when
    /// the service is bound.
    preference_reader: Option<crate::prefs::UiPreferences>,
    preferences_adopted: bool,
    preference_error: Option<String>,
    initial_preference_failure: Option<crate::prefs::PreferenceError>,
    /// Process-lifetime single-instance lock (or an explicit skip).
    _instance: InstancePermit,
    /// Shared host-capacity warning; both front ends read [`Self::capacity_notice`].
    capacity: Option<CapacityNotice>,
    capacity_kind: CapacityKind,
    capacity_render: RenderPolicyLabel,
    /// Next evaluate ([`CAPACITY_EVAL_EVERY`] apart).
    capacity_due: Option<Instant>,
    /// Moves whenever [`Self::capacity_notice`] changes.
    capacity_generation: u64,
    /// Moves whenever the vault's profile rows may have changed: a staged
    /// create, rename, delete or settings write, a restore after a failed
    /// write, or a vault opened or replaced. The fleet model rebuilds its
    /// profile universe only then.
    profiles_generation: u64,
    /// The saved Load order, reconciled with the vault.
    profile_order: crate::profile_order::ProfileOrder,
    /// The preferences hold a saved order (the operator reordered, or a
    /// rename kept a place): reconcile changes are written back.
    order_saved: bool,
    /// The saved Load order the latest durable snapshot holds: a reorder
    /// whose write fails goes back to it.
    durable_order: crate::profile_order::ProfileOrder,
    /// The stored order member the last snapshot held (`None`: absent), to
    /// tell when the stored order moved.
    durable_order_doc: Option<serde_json::Value>,
    /// Pending and failed state of every saved setting
    /// ([`Self::setting_status`]).
    settings: crate::setting::SettingBook,
    /// The durable preference version the session-owned state (walk
    /// grants, the manual-walk pause, the Load order, the session file)
    /// last followed.
    core_version: Option<api::local_state::StoreVersion>,
    /// The durable preference version the lent models (draw policy, bake
    /// gate, Quester source, walk view) last followed.
    models_version: Option<api::local_state::StoreVersion>,
    /// The session file state the durable preference last asked for.
    session_log_wanted: Option<bool>,
    /// Whether a failure of the session file sink was already reported
    /// for the current durable choice.
    session_sink_reported: bool,
    /// The Quester source the last accepted (still pending) write asked
    /// for, the baseline the next change is made against.
    quester_intent: Option<crate::quester_paths::QuesterPathsView>,
    /// The profiles generation the order was last reconciled at.
    order_reconciled_at: Option<u64>,
    /// Order writes that are not commands (a rename or a reconcile), still
    /// queued on the writer thread; a failure is logged.
    order_writes: Vec<host_play::local_state::QueuedUpdate>,
    /// Operator commands: the book and their settlement state.
    commands: commands::CommandDesk,
    /// Load all's preferred selection when nothing is selected.
    focus_hint: Option<String>,
}

/// How often the session evaluates host capacity.
const CAPACITY_EVAL_EVERY: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum CapacityKind {
    #[default]
    Clear,
    Silence,
    Warn,
}

impl<Io> OperatorSession<Io> {
    /// A session holding `permit` (the instance lock or the decision to run
    /// without it).
    pub fn open(permit: crate::launch::InstancePermit) -> Self {
        Self::new(permit.into_host())
    }

    pub fn new(instance: InstancePermit) -> Self {
        crate::log::global();
        Self {
            vault: None,
            play: None,
            fleet: Fleet::default(),
            selected: None,
            slots: HashMap::new(),
            removals: HashMap::new(),
            deferred: HashMap::new(),
            statuses: Vec::new(),
            polled: Vec::new(),
            transitions: Vec::new(),
            script_lines: Vec::new(),
            operations: OperationBook::default(),
            writer: None,
            writes: HashMap::new(),
            durable: HashMap::new(),
            #[cfg(any(test, feature = "test-support"))]
            write_gate: Arc::default(),
            latest_write: HashMap::new(),
            profile_edits: HashMap::new(),
            native_edits: HashMap::new(),
            preparations: HashMap::new(),
            deliveries: Vec::new(),
            write_failures: Vec::new(),
            saves: SaveBook::default(),
            script_settings_logs: HashMap::new(),

            settings_writes: Vec::new(),
            script_assignment_writes: VecDeque::new(),
            starts: HashMap::new(),
            settled_starts: Vec::new(),
            spawn_workers: true,
            #[cfg(any(test, feature = "test-support"))]
            bypass_asset_startup: false,
            views: Views::default(),
            start_places: HashMap::new(),
            mem_relog: HashMap::new(),
            public_worlds: None,
            op_changes: Vec::new(),
            op_line: String::new(),
            resources: Resources::default(),
            frames: Arc::new(crate::frames::FrameShared::new()),
            #[cfg(feature = "live-harness")]
            live_runner: crate::live::RunnerHandle::default(),
            pause_script_on_manual_walk_abort: true,
            walk_globals: crate::walk_permissions::WalkGlobals::default(),
            walk_globals_store: None,
            local_state: None,
            preference_reader: None,
            preferences_adopted: false,
            preference_error: None,
            initial_preference_failure: None,
            _instance: instance,
            capacity: None,
            capacity_kind: CapacityKind::Clear,
            capacity_render: RenderPolicyLabel::OnlySelected,
            capacity_due: None,
            capacity_generation: 0,
            profiles_generation: 0,
            profile_order: crate::profile_order::ProfileOrder::default(),
            order_saved: false,
            durable_order: crate::profile_order::ProfileOrder::default(),
            durable_order_doc: None,
            settings: crate::setting::SettingBook::default(),
            core_version: None,
            models_version: None,
            session_log_wanted: None,
            session_sink_reported: false,
            quester_intent: None,
            order_reconciled_at: None,
            order_writes: Vec::new(),
            commands: commands::CommandDesk::default(),
            focus_hint: None,
        }
    }

    #[cfg(feature = "live-harness")]
    pub fn live_runner(&self) -> &crate::live::RunnerHandle {
        &self.live_runner
    }

    /// [`Self::start`] for front-end and integration test fixtures.
    #[cfg(any(test, feature = "test-support"))]
    pub fn start_for_test(&mut self, vault: Vault, play: Play) {
        self.start(vault, play);
    }

    /// Adopt an unlocked vault and its freshly built [`Play`]. No slot is
    /// spawned here.
    pub(crate) fn start(&mut self, vault: Vault, mut play: Play) {
        for profile in vault.profiles() {
            api::hostlog::register_secret(&profile.password);
        }
        if let Some(service) = self.local_state.as_ref() {
            play.adopt_local_state(Arc::clone(service));
        }
        play.set_pause_script_on_manual_walk_abort(self.pause_script_on_manual_walk_abort);
        play.set_walk_globals(self.walk_globals.to_host());
        if let Some(preferences) = self.walk_globals_store.as_ref() {
            play.set_walk_globals_store(preferences.clone());
        }
        play.statuses_into(&mut self.statuses);
        self.public_worlds = play
            .server_profile()
            .and_then(|profile| profile.public_worlds())
            .cloned();
        self.play = Some(play);
        self.vault = Some(vault);
        self.profiles_generation = self.profiles_generation.wrapping_add(1);
        self.writer = None;
        // Publish the UID/name projection before any startup or explicit Load.
        self.reconcile_profile_order();
    }

    /// Start the play session `launch` describes, with `hook` as the front
    /// end's per-frame client work on every slot, and adopt it with `vault`.
    pub(crate) fn start_play(
        &mut self,
        vault: Vault,
        launch: crate::launch::PlayLaunch,
        hook: Arc<dyn crate::frames::FrameHook>,
    ) -> Result<(), String> {
        use crate::launch::LaunchParts;
        let (parts, auto_world, keep_world_state) = launch.into_parts();
        let map_members = parts.map_members();
        self.frames.set_keep_world_state(keep_world_state);
        let ops = crate::frames::ClientOps::new(
            host_play::player_here_tile,
            host_play::Host::publish_frontend_snapshot,
            host_play::step_walk_arm_follow,
            |client| client.gens.player,
        );
        #[cfg(feature = "live-harness")]
        let ops = ops.with_live_frame(crate::live::ScenarioFrame::new(
            self.live_runner.share(),
            scenario::ScenarioRunner::tick_with_hold,
            scenario::ScenarioRunner::companion_tick,
        ));
        let per_frame = crate::frames::slot_frame(Arc::clone(&self.frames), hook, ops);
        let mut play = match parts {
            LaunchParts::Validated(validated, mainland) => host_play::run_prepared_template(
                validated,
                mainland,
                Vec::new(),
                |_| (None, None),
                per_frame,
            )?,
            LaunchParts::Template(template, mainland) => host_play::run_with_template(
                template,
                mainland,
                Vec::new(),
                |_| (None, None),
                per_frame,
            )?,
            LaunchParts::Direct(options) => {
                host_play::run_with_io(&options, Vec::new(), |_| (None, None), per_frame)
            }
        };
        if let Some(number) = auto_world {
            play.set_auto_world(number)?;
        }
        self.frames.bind_world(play.world(), map_members);
        self.frames.bind_script_nav_paint(play.script_nav_paint());
        self.start(vault, play);
        #[cfg(feature = "live-harness")]
        {
            // `start` adopts the session state before the runner snapshots its StartHandle.
            self.live_runner
                .install_play(self.play.as_ref().expect("started play is installed"));
        }
        Ok(())
    }

    /// Open (or first-run create) the vault at `path` with `typed` and
    /// start the play session `launch` describes, with `hook` as the front
    /// end's per-frame client work.
    pub(crate) fn unlock_vault(
        &mut self,
        path: &std::path::Path,
        typed: &str,
        launch: crate::launch::PlayLaunch,
        hook: Arc<dyn crate::frames::FrameHook>,
    ) -> Result<(), crate::launch::UnlockError> {
        use crate::launch::{UnlockError, UnlockStage};
        let vault = crate::launch::open_vault_typed(path, typed)
            .map_err(|error| UnlockError::new(UnlockStage::Vault, error.to_string()))?;
        self.start_play(vault, launch, hook)
            .map_err(|error| UnlockError::new(UnlockStage::Launch, error))
    }

    /// Whether a walk ended, was refused or was cancelled since the last
    /// call (clears the flag).
    pub fn take_walk_clear(&self) -> bool {
        self.frames.take_walk_clear()
    }

    /// Read `name`'s walk arm (tests and the live probe proofs).
    #[cfg(any(test, feature = "test-support"))]
    pub fn with_walk_arm<R>(
        &self,
        name: &str,
        f: impl FnOnce(&host_play::WalkArm) -> R,
    ) -> Option<R> {
        let arm = self.frames.walk_arms().lock().unwrap().get(name).cloned()?;
        let arm = arm.lock().unwrap();
        Some(f(&arm))
    }

    /// Whether `name` has an armed walk route.
    pub fn walk_route_armed(&self, name: &str) -> bool {
        self.frames
            .walk_arms()
            .lock()
            .unwrap()
            .get(name)
            .is_some_and(|arm| arm.lock().unwrap().route.is_some())
    }

    /// Read `name`'s latest published snapshot.
    pub fn with_slot_snapshot<R>(
        &self,
        name: &str,
        f: impl FnOnce(&api::snapshot::GameSnapshot) -> R,
    ) -> Option<R> {
        self.frames.with_snapshot(name, f)
    }

    /// Read `name`'s latest published snapshot with its navigation world
    /// state (kept when the launch asked for it).
    pub fn with_slot_world_state<R>(
        &self,
        name: &str,
        f: impl FnOnce(&api::snapshot::GameSnapshot, &nav_planner::WorldState) -> R,
    ) -> Option<R> {
        self.frames.with_world_state(name, f)
    }

    pub(crate) fn walk_arms(&self) -> &WalkArms {
        self.frames.walk_arms()
    }

    pub(crate) fn frames(&self) -> &crate::frames::FrameShared {
        &self.frames
    }

    /// Bind the process local-state service the front end's composition
    /// root opened, before any `Play` starts: [`Self::start`] hands it to
    /// each `Play`, so slots and forms write through one service.
    pub fn bind_local_state(&mut self, service: Arc<host_play::local_state::LocalStateService>) {
        self.preference_reader = Some(crate::prefs::UiPreferences::new(service.preferences()));
        self.local_state = Some(service);
        self.preferences_adopted = false;
        self.preference_error = None;
        self.initial_preference_failure = None;
        self.core_version = None;
        self.models_version = None;
        self.session_log_wanted = None;
        self.durable_order_doc = None;
        self.session_sink_reported = false;
    }

    /// Open the process local-state service over the operator's state
    /// root (`<bot home>/.274bot`) and bind it: the composition root's one
    /// writer of preferences and loadouts.
    pub fn open_process_state(&mut self) {
        let root = host_play::local_state::StateRoot::new(script::bot_home().join(".274bot"));
        self.bind_local_state(host_play::local_state::LocalStateService::open_process(
            root,
        ));
    }

    /// The bound service's preferences document.
    pub fn preferences(&self) -> Option<crate::prefs::UiPreferences> {
        self.local_state
            .as_ref()
            .map(|service| crate::prefs::UiPreferences::new(service.preferences()))
    }

    /// The retained startup or quiet-save preference error, if one occurred.
    pub fn preference_error(&self) -> Option<&str> {
        self.preference_error.as_deref()
    }

    /// Read the latest durable preference snapshot and let the
    /// session-owned state follow it once per durable version: the walk
    /// grants, the manual-walk pause, the Load order and the session file.
    /// A cold store returns `None` (the shown state is provisional and
    /// nothing is saved over it); a refused store keeps its diagnostic until
    /// it reads again. The lent models follow the same snapshot in
    /// [`Self::poll_preferences`].
    pub(crate) fn sync_snapshot(&mut self) -> Option<crate::prefs::PreferenceSnapshot> {
        // The handle is made once at the bind: a per-poll handle would
        // allocate on every frame of a steady session.
        let preferences = self.preference_reader.as_ref()?.share();
        match preferences.snapshot() {
            Ok(snapshot) => {
                if self.initial_preference_failure.take().is_some() {
                    self.preference_error = None;
                }
                if self.core_version != Some(snapshot.version()) {
                    let first = self.core_version.is_none();
                    self.core_version = Some(snapshot.version());
                    self.follow_snapshot(&preferences, &snapshot, first);
                }
                Some(snapshot)
            }
            Err(error) if error.is_not_ready() => None,
            Err(error) => {
                if self.initial_preference_failure.as_ref() != Some(&error) {
                    self.preference_error = Some(error.to_string());
                    self.initial_preference_failure = Some(error);
                }
                None
            }
        }
    }

    /// Prepare automatic startup without loading or selecting a profile.
    /// Both front ends use this gate before consuming a deferred request.
    /// A cold or failed preference store is not an empty saved order.
    pub fn prepare_startup(&mut self) -> bool {
        self.sync_snapshot();
        if self.vault.is_none() || (self.local_state.is_some() && !self.preferences_adopted) {
            return false;
        }
        self.reconcile_profile_order();
        true
    }

    /// The process local-state service bound at startup, if any.
    pub(crate) fn local_state(&self) -> Option<&Arc<host_play::local_state::LocalStateService>> {
        self.local_state.as_ref()
    }

    /// [`Self::local_state`] for front-end tests.
    #[cfg(any(test, feature = "test-support"))]
    pub fn local_state_for_test(&self) -> Option<&Arc<host_play::local_state::LocalStateService>> {
        self.local_state.as_ref()
    }

    /// Project the persisted manual-movement pause setting onto the current
    /// Play and retain it for the next Play/session slot.
    pub(crate) fn set_pause_script_on_manual_walk_abort(&mut self, enabled: bool) {
        if self.pause_script_on_manual_walk_abort == enabled {
            return;
        }
        self.pause_script_on_manual_walk_abort = enabled;
        if let Some(play) = self.play.as_ref() {
            play.set_pause_script_on_manual_walk_abort(enabled);
        }
    }

    /// The manual-movement pause this session runs with now (tests).
    #[cfg(test)]
    pub(crate) fn running_pause_script_on_manual_walk_abort(&self) -> bool {
        self.pause_script_on_manual_walk_abort
    }

    /// Publish grants before or after Play installation.
    pub(crate) fn set_walk_globals(&mut self, globals: crate::walk_permissions::WalkGlobals) {
        self.walk_globals = globals;
        if let Some(play) = self.play.as_ref() {
            play.set_walk_globals(globals.to_host());
        }
    }

    /// Interactive frontends share the durable preferences even after
    /// Continue anyway: every admission reads the globals from them.
    pub(crate) fn set_walk_globals_store(&mut self, preferences: crate::prefs::UiPreferences) {
        let preferences = preferences.inner().clone();
        if let Some(play) = self.play.as_mut() {
            play.set_walk_globals_store(preferences.clone());
        }
        self.walk_globals_store = Some(preferences);
    }

    pub(crate) fn vault(&self) -> Option<&Vault> {
        self.vault.as_ref()
    }

    pub(crate) fn play(&self) -> Option<&Play> {
        self.play.as_ref()
    }

    #[cfg(test)]
    pub(crate) fn play_mut(&mut self) -> Option<&mut Play> {
        self.play.as_mut()
    }

    /// The play session, for front-end test fixtures.
    #[cfg(any(test, feature = "test-support"))]
    pub fn play_for_test(&self) -> Option<&Play> {
        self.play.as_ref()
    }

    /// The play session, for front-end test fixtures (writes).
    #[cfg(any(test, feature = "test-support"))]
    pub fn play_mut_for_test(&mut self) -> Option<&mut Play> {
        self.play.as_mut()
    }

    /// Drop the play (joins its workers). Final teardown only.
    pub(crate) fn close_play(&mut self) {
        self.deferred.clear();
        self.mem_relog.clear();
        self.public_worlds = None;
        self.play = None;
    }

    /// Offer `numbers` as this server's public worlds (a direct test play
    /// has none).
    #[cfg(any(test, feature = "test-support"))]
    pub fn set_public_worlds_for_test(&mut self, numbers: &[u16]) {
        self.public_worlds = Some(Arc::new(host_play::public_worlds::PublicWorlds {
            schema_version: 1,
            worlds: numbers
                .iter()
                .map(|&number| host_play::public_worlds::PublicWorld {
                    number,
                    host: format!("w{number}.test"),
                    port: 443,
                    node_id: i32::from(number) + 9,
                })
                .collect(),
        }));
    }

    pub fn fleet(&self) -> &Fleet {
        &self.fleet
    }

    pub fn members(&self) -> &[String] {
        self.fleet.members()
    }

    pub fn selected(&self) -> Option<&str> {
        self.selected.as_deref()
    }

    /// Surface IO of every slot this session spawned (or retains for a
    /// terminal lifetime).
    pub fn slots(&self) -> &HashMap<String, Io> {
        &self.slots
    }

    pub fn slot_io(&self, name: &str) -> Option<&Io> {
        self.slots.get(name)
    }

    pub fn removal_pending(&self, name: &str) -> bool {
        self.removals.contains_key(name)
    }

    /// Rows from the last [`Self::poll`] (or [`Self::start`]).
    #[cfg(any(test, feature = "test-support"))]
    pub fn statuses_for_test(&self) -> &[SlotStatus] {
        &self.statuses
    }

    /// Find a terminal startup failure among slots owned by the live runner.
    #[cfg(feature = "live-harness")]
    pub fn live_terminal_startup_failure(&self) -> Option<String> {
        self.play
            .as_ref()
            .and_then(|_| self.live_runner.terminal_startup_failure(&self.statuses))
    }

    /// Copy the rows into a front end's retained vector, reusing its row
    /// buffers: no allocation when nothing grew.
    pub fn copy_statuses_into(&self, out: &mut Vec<SlotStatus>) {
        out.truncate(self.statuses.len());
        let kept = out.len();
        out.clone_from_slice(&self.statuses[..kept]);
        out.extend_from_slice(&self.statuses[kept..]);
    }

    pub(crate) fn status(&self, name: &str) -> Option<&SlotStatus> {
        self.statuses.iter().find(|s| s.username == name)
    }

    /// The raw status row of `name`, for front-end test fixtures.
    #[cfg(any(test, feature = "test-support"))]
    pub fn status_for_test(&self, name: &str) -> Option<&SlotStatus> {
        self.status(name)
    }

    /// Status changes observed by the last poll.
    pub fn transitions(&self) -> &[SlotTransition] {
        &self.transitions
    }

    pub fn operation(&self, id: OperationId) -> Option<&OperationReport> {
        self.operations.get(id)
    }

    pub fn last_operation(&self) -> Option<&OperationReport> {
        self.operations.last()
    }

    /// Fleet rows, counts and the selected slot's detail as of the last
    /// poll. Both front ends render these instead of deriving their own.
    pub fn fleet_view(&self) -> FleetView<'_> {
        self.views.view(self.operations.last())
    }

    /// The process resource meter as of its last 1 Hz sample.
    pub fn resources(&self) -> &ResourceView {
        self.resources.view()
    }

    /// Shared host-capacity warning, when the host is behind.
    pub fn capacity_notice(&self) -> Option<&CapacityNotice> {
        self.capacity.as_ref()
    }

    /// Moves whenever [`Self::capacity_notice`] changed, so a front end
    /// copies the line only then.
    pub fn capacity_generation(&self) -> u64 {
        self.capacity_generation
    }

    /// What the front end is rendering, included in the capacity warning.
    pub(crate) fn set_capacity_render(&mut self, render: RenderPolicyLabel) {
        self.capacity_render = render;
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn inject_capacity_warning(&mut self, warning: CapacityWarning) {
        self.apply_capacity(CapacityEval::Warn(warning.to_host()));
    }

    /// Moves whenever a sample changed [`Self::resources`].
    pub fn resource_generation(&self) -> u64 {
        self.resources.generation()
    }

    /// Hand over the front end's walk arms (the map its WalkTo arms and
    /// its slot hook follows) so the rows show those walks too.
    #[cfg(any(test, feature = "test-support"))]
    pub fn set_walk_arms(&mut self, arms: WalkArms) {
        let shared =
            Arc::get_mut(&mut self.frames).expect("walk arms are replaced before any play starts");
        shared.replace_walk_arms(arms);
    }

    /// Publish a retained snapshot and navigation state for frontend tests.
    #[cfg(any(test, feature = "test-support"))]
    pub fn publish_test_snapshot(
        &mut self,
        name: &str,
        snapshot: api::snapshot::GameSnapshot,
        world_state: nav_planner::WorldState,
    ) {
        self.frames
            .publish_test_snapshot(name, snapshot, world_state);
    }

    /// Complete a retained walk arm and raise the next-frame clear signal.
    #[cfg(any(test, feature = "test-support"))]
    pub fn finish_walk_for_test(&mut self, name: &str) {
        if let Some(arm) = self.frames.walk_arms().lock().unwrap().get(name).cloned() {
            arm.lock().unwrap().route = None;
        }
        self.frames.mark_walk_clear();
    }

    /// Open an operation for a coordinator in this crate (scripts).
    pub(crate) fn open_operation(&mut self, action: ActionKind) -> OperationId {
        self.operations.open(action)
    }

    pub(crate) fn set_outcome(&mut self, op: OperationId, slot: &str, outcome: Outcome) {
        self.operations.set(op, slot, outcome);
    }
    pub(crate) fn track_script_settings_log(&mut self, op: OperationId, audit: ScriptSettingsLog) {
        self.script_settings_logs.insert(op, audit);
    }

    #[cfg(feature = "js")]
    pub(crate) fn log_legacy_script_settings(
        &mut self,
        card: &str,
        action: &str,
        field: &str,
        old_value: Option<&serde_json::Value>,
        new_value: &serde_json::Value,
    ) -> OperationId {
        let op = self.open_operation(ActionKind::SaveProfile);
        self.operations.set(op, "legacy", Outcome::Completed);
        let old_value =
            old_value.map_or_else(|| "<unset>".to_owned(), serde_json::Value::to_string);
        crate::log::global().process_line(
            Source::Script,
            Level::Info,
            &format!(
                "saved script settings; profile legacy; card {card}; action {action}; field {field}; old {old_value}; new {new_value}; op#{}",
                op.0
            ),
        );
        op
    }

    /// Failure text of `id`, for a front end's error line.
    pub fn failure(&self, id: OperationId) -> Option<String> {
        self.operations
            .get(id)
            .and_then(OperationReport::first_failure)
    }

    /// Load Starts settled since the last take.
    pub fn take_settled_starts(&mut self) -> Vec<StartSettled> {
        std::mem::take(&mut self.settled_starts)
    }

    /// Vault usernames plus any running slot outside the vault.
    /// The stable selection identity for a vault profile. Front ends use this
    /// instead of display names when they mirror marked rows.
    pub fn profile_identity(&self, name: &str) -> Option<crate::selection::ProfileIdentity> {
        self.vault
            .as_ref()
            .and_then(|vault| vault.get(name))
            .map(|profile| crate::selection::ProfileIdentity::uid(profile.uid))
    }

    /// [`Self::profile_identity`], or the deterministic fixture identity
    /// for a name with no vault row (it marks nothing real).
    pub fn profile_identity_or_synthetic(&self, name: &str) -> crate::selection::ProfileIdentity {
        self.profile_identity(name)
            .unwrap_or_else(|| crate::selection::ProfileIdentity::synthetic(name))
    }

    /// Every profile name: the vault's in the saved Load order, then any
    /// live slot without a vault row.
    pub fn profile_names(&self) -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        if let Some(vault) = self.vault.as_ref() {
            names.extend(
                self.profile_order
                    .names()
                    .iter()
                    .filter(|name| vault.get(name).is_some())
                    .cloned(),
            );
            for profile in vault.profiles() {
                if !names.contains(&profile.username) {
                    names.push(profile.username.clone());
                }
            }
        }
        for s in &self.statuses {
            if !names.contains(&s.username) {
                names.push(s.username.clone());
            }
        }
        names
    }

    /// The saved Load order.
    pub fn profile_order(&self) -> &crate::profile_order::ProfileOrder {
        &self.profile_order
    }

    /// `name`'s place in the saved Load order; `None` for a name it does
    /// not hold (sorted after every ordered name).
    pub(crate) fn order_rank(&self, name: &str) -> Option<usize> {
        self.profile_order.position(name)
    }

    /// Follow the saved Load order of the latest durable snapshot. When the
    /// stored order moved (the first adoption, another process, or this
    /// session's own write landing) the durable copy becomes it, and so does
    /// the shown order unless a write of the shown order is still pending: a
    /// pending reorder keeps its preview until it settles, and a failed one
    /// goes back to the durable copy ([`Self::revert_profile_order`]).
    pub(crate) fn follow_profile_order(
        &mut self,
        snapshot: &crate::prefs::PreferenceSnapshot,
        first: bool,
    ) {
        let value = snapshot
            .document()
            .get(crate::profile_order::PROFILE_ORDER_KEY);
        if value.is_some() {
            self.order_saved = true;
        } else if first {
            self.order_saved = false;
        }
        if !first && self.durable_order_doc.as_ref() == value {
            return;
        }
        self.durable_order_doc = value.cloned();
        let loaded = crate::profile_order::ProfileOrder::from_value(value);
        let pending = !self.order_writes.is_empty()
            || matches!(
                self.settings.status(crate::Setting::ProfileOrder),
                crate::SettingStatus::Pending
            );
        if first || !pending {
            self.profile_order.replace(loaded);
            self.durable_order.assign_from(&self.profile_order);
            self.profiles_generation = self.profiles_generation.wrapping_add(1);
        } else {
            self.durable_order.replace(loaded);
        }
        self.order_reconciled_at = None;
        self.reconcile_profile_order();
    }

    /// Make the saved order match the vault (seed, append, drop) and the
    /// fleet members follow it. Moves the profiles generation when the
    /// order changed, so the fleet views rebuild. A profile the vault
    /// lacks stays stored while a write of it is queued (its deletion is
    /// not durable, and a failure puts it back in its place); otherwise it
    /// is dropped, and the saved order is written.
    pub(crate) fn reconcile_profile_order(&mut self) {
        if self.order_reconciled_at == Some(self.profiles_generation) {
            return;
        }
        let changed = match self.vault.as_ref() {
            Some(vault) => {
                let rows = || {
                    vault
                        .profiles()
                        .map(|profile| (profile.uid, profile.username.as_str()))
                };
                let durable = &self.durable;
                let pending = |uid: i32| {
                    durable
                        .values()
                        .any(|profile| profile.as_ref().is_some_and(|p| p.uid == uid))
                };
                // The durable copy follows the vault too, so a failed
                // write's revert shows current names.
                self.durable_order.reconcile(rows(), pending);
                self.profile_order.reconcile(rows(), pending)
            }
            None => crate::profile_order::Reconciled::default(),
        };
        if changed.stored_changed || changed.sequence_changed {
            self.profiles_generation = self.profiles_generation.wrapping_add(1);
        }
        if changed.stored_changed && self.order_saved {
            self.persist_profile_order_quietly();
        }
        self.order_reconciled_at = Some(self.profiles_generation);
        self.sort_members_by_order();
    }

    /// After profile writes settle: a held deletion may be durable now, so
    /// reconcile again (dropping it from the saved order).
    fn reconcile_after_writes(&mut self) {
        if self.profile_order.holds_deleted() {
            self.order_reconciled_at = None;
            self.reconcile_profile_order();
        }
    }

    /// The uid for a new profile: above every uid the vault holds and
    /// every uid the saved order has held, so a new profile never takes a
    /// deleted profile's place. Shared by every path that creates a
    /// profile.
    pub fn fresh_uid(&self) -> i32 {
        let vault = self.vault.as_ref().map(crate::profiles::next_vault_uid);
        let order = self
            .profile_order
            .max_uid()
            .max(self.durable_order.max_uid())
            .saturating_add(1);
        vault.unwrap_or(crate::profiles::FIRST_UID).max(order)
    }

    /// Order the fleet members by the saved order (stable: a member the
    /// order lacks keeps its place after the ordered ones). A login
    /// already queued keeps its queue place.
    pub(crate) fn sort_members_by_order(&mut self) {
        let order = &self.profile_order;
        self.fleet.sort_by_rank(|name| order.position(name));
    }

    /// Write the saved order outside a command (a rename, a reconcile). A
    /// queued write is polled by [`Self::poll`]; a failure is logged.
    fn persist_profile_order_quietly(&mut self) {
        let Some(preferences) = self.preferences() else {
            return;
        };
        match preferences.begin_set(
            crate::profile_order::PROFILE_ORDER_KEY,
            self.profile_order.to_value(),
        ) {
            // Nothing to wait for; the durable copy follows the snapshot.
            Ok(host_play::local_state::Update::Durable) => {
                let setting = crate::Setting::ProfileOrder;
                let had_failure = self.settings.failure(setting).is_some();
                self.settings.clear_failure(setting);
                if had_failure {
                    self.note_setting(setting, prefs::SettingEventState::Cleared);
                }
                self.note_setting(setting, prefs::SettingEventState::Saved);
            }
            Ok(host_play::local_state::Update::Queued(queued)) => {
                let setting = crate::Setting::ProfileOrder;
                let had_failure = self.settings.failure(setting).is_some();
                self.settings.begin(setting);
                if had_failure {
                    self.note_setting(setting, prefs::SettingEventState::Cleared);
                }
                self.order_writes.push(queued);
                self.note_setting(setting, prefs::SettingEventState::Pending);
            }
            Err(error) => self.fail_order_write(&error, false),
        }
    }

    /// Poll the order writes that are not commands. A failure stays on the
    /// Load order setting ([`Self::setting_status`]); the shown order keeps
    /// the vault's truth and the durable copy stays the last stored order,
    /// so a later reorder that fails goes back to what is stored.
    pub(crate) fn poll_order_writes(&mut self) {
        let mut index = 0;
        while index < self.order_writes.len() {
            match self.order_writes[index].poll() {
                None => index += 1,
                Some(outcome) => {
                    self.order_writes.swap_remove(index);
                    match outcome {
                        Ok(()) => {
                            let setting = crate::Setting::ProfileOrder;
                            let had_failure = self.settings.failure(setting).is_some();
                            self.settings.succeed(setting);
                            if had_failure {
                                self.note_setting(setting, prefs::SettingEventState::Cleared);
                            }
                            self.note_setting(setting, prefs::SettingEventState::Saved);
                        }
                        Err(error) => self.fail_order_write(&error, true),
                    }
                }
            }
        }
    }

    fn fail_order_write(&mut self, error: &crate::prefs::PreferenceError, was_pending: bool) {
        order_write_failed(error);
        let text = format!("Load order not saved: {error}");
        let event_error = self
            .commands
            .watching_setting_events()
            .then(|| text.clone());
        let now = Instant::now();
        if was_pending {
            self.settings.fail(crate::Setting::ProfileOrder, text, now);
        } else {
            self.settings
                .fail_now(crate::Setting::ProfileOrder, text, now);
        }
        if let Some(error) = event_error {
            self.commands.note_setting_event(
                crate::Setting::ProfileOrder.identity(),
                prefs::SettingEventState::Failed,
                Some(error),
            );
        }
    }

    /// Live slots other than the selected one.
    pub fn background_bot_count(&self) -> usize {
        self.play
            .as_ref()
            .map(|play| play.background_bot_count(self.selected()))
            .unwrap_or(0)
    }

    /// Make `name` the selected bot: pure bookkeeping, never a spawn or a
    /// login. The host's focus follows (the preferred login owner), and both
    /// the outgoing and incoming slots are woken so a parked worker re-reads
    /// its draw state within a frame.
    pub(crate) fn select(&mut self, name: &str) -> Selection {
        if self.selected.as_deref() == Some(name) || self.profile_saving(name) {
            return Selection::Unchanged;
        }
        let previous = self.selected.replace(name.to_string());
        if let Some(play) = self.play.as_mut() {
            play.focus(name);
            if let Some(old) = previous.as_deref() {
                play.wake(old);
            }
        }
        Selection::changed(previous)
    }

    /// Control arm for a vault profile: auto-login remains a saved policy,
    /// while a fleet logout latch starts the worker in an explicit
    /// logged-out hold.
    pub fn arm_for_profile(&self, name: &str) -> Option<Arc<SlotArm>> {
        let profile = self.durable_profile(name)?;
        let arm = SlotArm::new(profile.uid, profile.settings.auto_login);
        arm.random_events
            .store(profile.settings.random_events, Ordering::Relaxed);
        arm.lamp_auto
            .store(profile.settings.lamp_auto, Ordering::Relaxed);
        *arm.lamp_skill.lock().unwrap() = profile.settings.lamp_skill.clone();
        *arm.random_event_policies.lock().unwrap() = profile.settings.random_event_policies.clone();
        if self.fleet.latched(name) {
            arm.hold_logged_out();
        }
        Some(arm)
    }

    /// Spawn `name`'s worker when it has none. `arm` carries the spawn's
    /// login intent. Existing IO with no arm is a preserved terminal
    /// lifetime and restarts only when `restart_terminal` is set (explicit
    /// Log in). Without a play (pre-unlock) only the IO is registered.
    /// While the name's previous worker is still stopping, the spawn is
    /// queued and [`Self::poll`] starts it once that worker has exited.
    pub(crate) fn ensure_slot<S: SlotSurface<Io = Io>>(
        &mut self,
        name: &str,
        arm: Option<Arc<SlotArm>>,
        restart_terminal: bool,
        surface: &mut S,
    ) -> Result<(), String> {
        if self.profile_saving(name) {
            return Err(format!("{name}: {SAVING}"));
        }
        if self
            .play
            .as_ref()
            .is_some_and(|play| play.arm(name).is_some())
        {
            return Ok(());
        }
        if self.slots.contains_key(name) && (self.play.is_none() || !restart_terminal) {
            return Ok(());
        }
        let Some(mut profile) = self.durable_profile(name).cloned() else {
            return Ok(());
        };
        self.deferred.remove(name);
        let walk_was_armed = self.frames.end_lifetime(name);
        surface.lifetime_reset(name, walk_was_armed);
        let retained = self.slots.remove(name);
        let attach = surface.attach(
            name,
            &mut crate::surface::SpawnProfile::new(&mut profile),
            retained,
        );
        let (io, input, mailbox) = attach.into_parts();
        if let Some(arm) = arm.as_ref() {
            // The surface may apply a session-only override to the disposable
            // spawn profile. Queue that mode for this lifetime's next login.
            arm.set_lowmem_handshake(profile.settings.lowmem);
        }
        if !self.spawn_workers {
            self.old_lifetime_retired(name);
            if let (Some(play), Some(arm)) = (self.play.as_mut(), arm.as_ref()) {
                play.attach_arm(name, Arc::clone(arm));
            }
            self.slots.insert(name.to_string(), io);
            return Ok(());
        }
        #[cfg(any(test, feature = "test-support"))]
        if let (true, Some(arm)) = (self.bypass_asset_startup, arm.as_ref()) {
            arm.bypass_asset_startup_for_test();
        }
        if self
            .play
            .as_ref()
            .is_some_and(|play| play.slot_stopping(name))
        {
            self.deferred.insert(
                name.to_string(),
                DeferredSpawn {
                    profile,
                    input,
                    mailbox,
                    arm,
                },
            );
            self.slots.insert(name.to_string(), io);
            return Ok(());
        }
        if self.play.is_some() {
            self.old_lifetime_retired(name);
        }
        if let Some(play) = self.play.as_mut() {
            if let Err(error) = play.try_spawn_slot(profile, input, mailbox, arm) {
                self.slots.insert(name.to_string(), io);
                return Err(error);
            }
        }
        self.slots.insert(name.to_string(), io);
        Ok(())
    }

    /// A new lifetime of `name` is about to start, so its previous one has
    /// exited: a Remove still waiting on that worker is complete (poll can
    /// no longer tell once the replacement arm exists).
    fn old_lifetime_retired(&mut self, name: &str) {
        self.operations
            .resolve_pending(ActionKind::Remove, name, Outcome::Completed);
    }

    /// Cancel a pending removal in response to an operator action. The
    /// retained IO is restored only when no replacement arm exists or when
    /// the current arm is the exact lifetime that owned the removal. Returns
    /// true only when the current arm also owns the cancelled removal, so
    /// callers may undo that removal's clean-logout request.
    fn cancel_removal(&mut self, name: &str) -> bool {
        let Some(mut pending) = self.removals.remove(name) else {
            return false;
        };
        self.operations.set(pending.op, name, Outcome::Cancelled);
        let current = self.play.as_ref().and_then(|play| play.arm(name));
        let owns_cancelled_removal = current
            .as_ref()
            .is_some_and(|arm| Arc::ptr_eq(arm, &pending.arm));
        if current.is_none() || owns_cancelled_removal {
            if let Some(io) = pending.io.take() {
                self.slots.entry(name.to_string()).or_insert(io);
            }
        }
        owns_cancelled_removal
    }

    /// Bring `name`'s worker up without changing login intent: cancel a
    /// pending removal and spawn with the profile's saved arm. A terminal
    /// lifetime is kept (its failure stays visible) until explicit Log in.
    /// The running bot is a fleet member, as every running lifetime is
    /// ([`Self::seed_running`]): a single-bot focus (the unlock's
    /// auto-login of the last focused profile, a chooser pick with MultiBox
    /// off) is loaded like a Load, so the Fleet window lists it and a
    /// loaded script can start on it.
    pub(crate) fn open_slot<S: SlotSurface<Io = Io>>(
        &mut self,
        name: &str,
        surface: &mut S,
    ) -> Result<(), String> {
        self.cancel_removal(name);
        let arm = self.arm_for_profile(name);
        self.ensure_slot(name, arm, false, surface)?;
        if self.fleet.add(name) {
            self.sort_members_by_order();
        }
        Ok(())
    }

    /// Add `name` to the fleet and bring its worker up. Login intent follows
    /// the profile's auto-login unless the logout latch blocks it. Returns
    /// whether the name was newly added. Tests; operators submit
    /// [`OperatorCommand::Load`].
    #[cfg(test)]
    pub(crate) fn load<S: SlotSurface<Io = Io>>(
        &mut self,
        name: &str,
        surface: &mut S,
    ) -> (OperationId, bool) {
        let op = self.operations.open(ActionKind::Load);
        let added = self.load_member(op, name, surface);
        (op, added)
    }

    /// Load every profile (vault plus running slots) that is not a member
    /// yet. Returns the operation and how many were newly added.
    ///
    /// Never steals an existing selection, and later logins of the new
    /// members do not either. When nothing is selected, [`pick_focus`]
    /// chooses a member (last focus if it is now on the wall, else the
    /// first) so panel and TUI have something to show. Single-row Load /
    /// clicking a bot still focuses that member.
    #[cfg(test)]
    pub(crate) fn load_all<S: SlotSurface<Io = Io>>(
        &mut self,
        surface: &mut S,
        last_focus: Option<&str>,
    ) -> (OperationId, usize) {
        let op = self.operations.open(ActionKind::Load);
        let mut added = 0;
        for name in self.profile_names() {
            if self.load_member(op, &name, surface) {
                added += 1;
            }
        }
        if self.selected.is_none() {
            if let Some(name) = pick_focus(self.members(), last_focus) {
                self.select(&name);
            }
        }
        (op, added)
    }

    fn load_member<S: SlotSurface<Io = Io>>(
        &mut self,
        op: OperationId,
        name: &str,
        surface: &mut S,
    ) -> bool {
        if self.profile_saving(name) {
            self.operations.set(op, name, Outcome::skipped(SAVING));
            return false;
        }
        let cancelled_removal = self.cancel_removal(name);
        let added = self.fleet.add(name);
        if added {
            self.sort_members_by_order();
        }
        let auto_login = self
            .durable_profile(name)
            .map(|p| p.settings.auto_login)
            .unwrap_or(false);
        let want_login = self.fleet.should_auto_login(name, auto_login);
        let running = self.play.as_ref().and_then(|play| play.arm(name));
        let result = match running {
            // Already running (re-load): refresh the saved auto intent. Only
            // a cancelled removal may reverse the clean logout it requested;
            // persisted/operator and repeat-guard latches remain parked.
            Some(arm) => {
                arm.set_auto_login(auto_login);
                if cancelled_removal && want_login && arm.login_latched() {
                    arm.arm_explicit_login();
                }
                if let Some(play) = self.play.as_ref() {
                    play.wake(name);
                }
                Ok(())
            }
            None => {
                let arm = self.arm_for_profile(name);
                self.ensure_slot(name, arm, false, surface)
            }
        };
        self.operations.set(op, name, completed_or_failed(result));
        added
    }

    /// Log in one member: cancel its removal, clear its latch, and arm an
    /// explicit one-shot handshake. A finished worker is reaped and a
    /// terminal lifetime recreated with its retained IO.
    pub(crate) fn login<S: SlotSurface<Io = Io>>(
        &mut self,
        name: &str,
        surface: &mut S,
    ) -> OperationId {
        let op = self.operations.open(ActionKind::Login);
        self.cancel_removal(name);
        self.fleet.clear_latch(name);
        if let Some(play) = self.play.as_mut() {
            play.reap_finished_workers();
        }
        let result = self.arm_login(name, surface);
        if let Some(play) = self.play.as_ref() {
            play.wake(name);
        }
        self.record_login(op, name, result);
        op
    }

    /// Log in every member. Queue membership follows worker arrival; the
    /// selected slot (or the first spawned one) is the sole priority
    /// exception.
    #[cfg(test)]
    pub(crate) fn login_all<S: SlotSurface<Io = Io>>(&mut self, surface: &mut S) -> OperationId {
        let names = self.fleet.members().to_vec();
        self.login_members(&names, surface)
    }

    /// Log in the named members only, as one operation.
    pub(crate) fn login_members<S: SlotSurface<Io = Io>>(
        &mut self,
        names: &[String],
        surface: &mut S,
    ) -> OperationId {
        let op = self.operations.open(ActionKind::Login);
        for name in names {
            self.cancel_removal(name);
            self.fleet.clear_latch(name);
        }
        if let Some(play) = self.play.as_mut() {
            play.reap_finished_workers();
        }
        for name in names {
            let result = self.arm_login(name, surface);
            self.record_login(op, name, result);
        }
        let head = self
            .selected
            .clone()
            .filter(|n| self.slots.contains_key(n))
            .or_else(|| self.slots.keys().next().cloned());
        if let Some(play) = self.play.as_ref() {
            if let Some(head) = head {
                play.prefer_login(&head);
            }
            play.wake_all();
        }
        op
    }

    fn arm_login<S: SlotSurface<Io = Io>>(
        &mut self,
        name: &str,
        surface: &mut S,
    ) -> Result<(), String> {
        // A manual Login supersedes the pending Relog-now; the worker still
        // owns the applied server-mode observation.
        self.mem_relog.remove(name);
        if let Some(arm) = self.play.as_ref().and_then(|play| play.arm(name)) {
            self.post(name, &arm, Intent::Login);
            return Ok(());
        }
        let arm = self.arm_for_profile(name);
        if let Some(arm) = arm.as_ref() {
            self.post(name, arm, Intent::Login);
        }
        self.ensure_slot(name, arm, true, surface)
    }

    fn record_login(&mut self, op: OperationId, name: &str, result: Result<(), String>) {
        self.operations.cancel_pending(ActionKind::Logout, name);
        self.operations.cancel_pending(ActionKind::Login, name);
        let outcome = match result {
            Err(error) => Outcome::failed(error),
            Ok(()) if self.play.is_none() => Outcome::failed("vault locked"),
            Ok(())
                if self.play.as_ref().and_then(|p| p.arm(name)).is_none()
                    && !self.deferred.contains_key(name) =>
            {
                Outcome::failed(format!("no profile {name}"))
            }
            Ok(()) => Outcome::Pending,
        };
        self.operations.set(op, name, outcome);
    }

    /// Log out one member: latch it so auto-login is blocked until Log in,
    /// then arm a clean logout. The slot stays up; only intent changes. A
    /// queued or connecting worker cancels its handshake at the next check.
    /// Tests; operators submit [`OperatorCommand::Logout`].
    #[cfg(test)]
    pub(crate) fn logout(&mut self, name: &str) -> OperationId {
        let op = self.operations.open(ActionKind::Logout);
        self.logout_member(op, name);
        if let Some(play) = self.play.as_ref() {
            play.wake(name);
        }
        op
    }

    /// Log out every member and every other running slot.
    #[cfg(test)]
    pub(crate) fn logout_all(&mut self) -> OperationId {
        let mut names = self.fleet.members().to_vec();
        if let Some(play) = &self.play {
            for s in play.statuses() {
                if !names.iter().any(|n| n == &s.username) {
                    names.push(s.username);
                }
            }
        }
        self.logout_members(&names)
    }

    /// Log out the named slots only, as one operation.
    pub(crate) fn logout_members(&mut self, names: &[String]) -> OperationId {
        let op = self.operations.open(ActionKind::Logout);
        for name in names {
            self.logout_member(op, name);
        }
        if let Some(play) = self.play.as_ref() {
            play.wake_all();
        }
        op
    }

    fn logout_member(&mut self, op: OperationId, name: &str) {
        // An explicit Logout supersedes the owner's Relog-now login half.
        self.mem_relog.remove(name);
        self.fleet.latch_logout(name);
        self.operations.cancel_pending(ActionKind::Login, name);
        self.operations.cancel_pending(ActionKind::Logout, name);
        if let Some(arm) = self.play.as_ref().and_then(|p| p.arm(name)) {
            self.post(name, &arm, Intent::Logout);
        } else if let Some(arm) = self.deferred.get(name).and_then(|d| d.arm.clone()) {
            // Not spawned yet: it starts on the title, held logged out.
            self.post(name, &arm, Intent::HoldLoggedOut);
        }
        self.operations.set(op, name, Outcome::Pending);
    }

    /// The queued memory mode, including any session-only spawn override.
    pub fn memory_mode(&self, name: &str) -> Option<bool> {
        self.play
            .as_ref()
            .and_then(|play| play.arm(name))
            .and_then(|arm| arm.lowmem_handshake())
            .or_else(|| {
                self.deferred
                    .get(name)
                    .map(|spawn| spawn.profile.settings.lowmem)
            })
            .or_else(|| self.vault.as_ref()?.get(name).map(|p| p.settings.lowmem))
    }

    /// Applied vs queued memory mode, plus any Relog-now in flight.
    /// `None` until the worker's first successful login.
    pub fn memory_status(&self, name: &str) -> Option<MemoryNotice> {
        let status = self
            .statuses
            .iter()
            .find(|status| status.username == name)?;
        Some(MemoryNotice {
            login_lowmem: status.login_lowmem?,
            desired_lowmem: self.memory_mode(name)?,
            relog_pending: self.mem_relog.contains_key(name),
            connected: status.connected,
            login_applies_memory: self
                .play
                .as_ref()
                .and_then(|play| play.arm(name))
                .is_some_and(|arm| arm.login_applies_memory()),
        })
    }

    /// Whether Relog-now must warn first: the slot runs a script a logout
    /// would interrupt (starting, running or paused).
    pub fn memory_relog_warning(&self, name: &str) -> bool {
        matches!(
            self.play.as_ref().map(|play| play.script_state(name)),
            Some(script::RunState::Starting | script::RunState::Running | script::RunState::Paused)
        )
    }

    /// `name`'s current script paint frame, from its status row. A frame
    /// that advertises host controls also carries the run it was read with
    /// (profile, run and lifecycle state), which decides their labels and
    /// binds their activations.
    pub fn script_paint(&self, name: &str) -> Option<crate::scripts::PaintFrame> {
        let paint = self.status(name)?.script_paint.clone()?;
        let run = crate::scripts::advertises_any_control(&paint).then(|| self.paint_run(name));
        Some(crate::scripts::PaintFrame::new(paint, run))
    }

    /// The run a host control on `name`'s paint is shown for.
    fn paint_run(&self, name: &str) -> crate::scripts::PaintRun {
        let held = self
            .play
            .as_ref()
            .and_then(|play| play.script_run_key(name));
        crate::scripts::PaintRun {
            profile: self.profile_identity_or_synthetic(name),
            run: held.map(|(slot, generation, _)| crate::scripts::RunToken::new(slot, generation)),
            state: held.map_or(crate::scripts::ScriptRunState::Idle, |(_, _, state)| {
                crate::scripts::ScriptRunState::from_script(state)
            }),
        }
    }

    /// Why a host paint control may not issue its row for `name` now, or
    /// `None` when its binding still holds: `name` is the profile it was
    /// shown for, the slot still holds the same run, the current frame has
    /// the same generation and still advertises the control, and the run's
    /// state still takes the shown action.
    pub(crate) fn paint_control_refusal(
        &self,
        name: &str,
        activation: &crate::scripts::ControlActivation,
    ) -> Option<&'static str> {
        if self.profile_identity_or_synthetic(name) != activation.profile {
            return Some("the paint control belongs to another profile");
        }
        let run = self.paint_run(name);
        if run.run != Some(activation.run) {
            return Some("the paint control's run has ended");
        }
        let current = self
            .status(name)
            .and_then(|status| status.script_paint.as_deref());
        if !current.is_some_and(|paint| {
            paint.generation == activation.generation
                && crate::scripts::advertises_control(paint, activation.control)
        }) {
            return Some("the paint control's frame is stale");
        }
        if !activation.control.applies(run.state) {
            return Some("the run no longer takes this paint control");
        }
        None
    }

    /// `name`'s script lifecycle state; `Idle` without a play or a script.
    pub fn script_state(&self, name: &str) -> crate::scripts::ScriptRunState {
        self.play
            .as_ref()
            .map_or(crate::scripts::ScriptRunState::Idle, |play| {
                crate::scripts::ScriptRunState::from_script(play.script_state(name))
            })
    }

    /// Whether a play session is running.
    pub fn has_play(&self) -> bool {
        self.play.is_some()
    }

    /// `name`'s status row.
    pub fn slot_view(&self, name: &str) -> Option<crate::SlotView<'_>> {
        self.status(name).map(crate::SlotView::new)
    }

    /// Whether `name` has a status row.
    pub fn has_status(&self, name: &str) -> bool {
        self.status(name).is_some()
    }

    /// Run an operator-typed debug command on `name`'s slot.
    pub(crate) fn debug_command(&self, name: &str, cmd: &str) -> Result<(), String> {
        let play = self.play.as_ref().ok_or("no active play session")?;
        play.cheat(name, cmd).map_err(|refusal| refusal.to_string())
    }

    /// Run the front end's tutorial probe (`getvar tutorial`, a read the
    /// panel sends itself once a slot is in game) on `name`'s slot. Not an
    /// operator decision: the panel's rail cheat buttons go through
    /// [`crate::OperatorCommand::DebugCommand`].
    pub fn debug_command_internal(&self, name: &str, cmd: &str) -> Result<(), String> {
        let play = self.play.as_ref().ok_or("no active play session")?;
        play.cheat_internal(name, cmd)
            .map_err(|refusal| refusal.to_string())
    }

    /// Kick `name`'s slot thread so a parked slot applies a draw, capture or
    /// focus change within a frame.
    pub fn wake_slot(&self, name: &str) {
        if let Some(play) = &self.play {
            play.wake(name);
        }
    }

    /// Kick every slot thread.
    pub fn wake_all_slots(&self) {
        if let Some(play) = &self.play {
            play.wake_all();
        }
    }

    /// `name`'s last script error, if any.
    pub fn script_last_error(&self, name: &str) -> Option<String> {
        self.play.as_ref()?.script_last_error(name)
    }

    /// `name`'s run progress, while a script run is armed.
    pub fn script_progress(&self, name: &str) -> Option<crate::progress::RunProgress> {
        self.play
            .as_ref()?
            .script_progress(name)
            .map(crate::progress::RunProgress::new)
    }

    /// Why queued script Starts are waiting, if they are.
    pub fn script_start_wait_reason(&self) -> Option<Arc<String>> {
        self.play.as_ref()?.script_start_wait_reason()
    }

    /// Deliver a paint-button click to `name`'s script (the
    /// `ScriptPaintInput` dispatch target).
    pub(crate) fn script_paint_click(&self, name: &str, id: &str, generation: u64) {
        if let Some(play) = &self.play {
            play.script_paint_click(name, id, generation);
        }
    }

    /// Deliver a paint chrome selection to `name`'s script (the
    /// `ScriptPaintInput` dispatch target).
    pub(crate) fn script_paint_select(
        &self,
        name: &str,
        key: &str,
        select_name: &str,
        generation: u64,
    ) {
        if let Some(play) = &self.play {
            play.script_paint_select(name, key, select_name, generation);
        }
    }

    /// Whether the bound map treats the world as members.
    pub fn map_members(&self) -> bool {
        self.play.as_ref().is_some_and(|play| play.map_members())
    }

    /// `name`'s known bank rows (empty without a play).
    pub fn bank_rows(&self, name: &str) -> nav_planner::bank_fetch::BankRows {
        self.play
            .as_ref()
            .map(|play| play.bank_rows(name))
            .unwrap_or_default()
    }

    /// The bound navigation world, once loaded.
    pub fn nav_world(&self) -> Option<Arc<nav_planner::world::NavWorld>> {
        self.play.as_ref()?.world()
    }

    /// The selected game data, once loaded.
    pub fn game_data(&self) -> Option<Arc<api::game_data::SelectedGameData>> {
        self.play.as_ref()?.game_data()
    }

    /// The object-name table (empty without a play).
    pub fn obj_names(&self) -> Option<Arc<api::obj_names::ObjNames>> {
        self.play.as_ref().map(|play| play.obj_names())
    }

    /// Whether `name`'s script is paused (its Pause button reads Resume).
    pub fn script_paused(&self, name: &str) -> bool {
        self.play
            .as_ref()
            .is_some_and(|play| play.script_state(name) == script::RunState::Paused)
    }

    /// Queue and persist the memory mode for the next login. Both front ends
    /// use this command; it never mutates the running client or logs out.
    /// [`Self::memory_status`] retains the applied mode through socket reconnects.
    pub(crate) fn set_memory_mode(
        &mut self,
        name: &str,
        lowmem: bool,
    ) -> Result<OperationId, String> {
        let mut profile = self
            .vault
            .as_ref()
            .ok_or_else(|| "memory: vault locked".to_string())?
            .get(name)
            .cloned()
            .ok_or_else(|| format!("memory: no profile {name}"))?;
        profile.settings.lowmem = lowmem;
        let op = self.save_profile(profile, ArmMirror::memory(lowmem), "memory")?;
        if let Some(arm) = self.play.as_ref().and_then(|play| play.arm(name)) {
            arm.set_lowmem_handshake(lowmem);
        }
        if let Some(spawn) = self.deferred.get_mut(name) {
            spawn.profile.settings.lowmem = lowmem;
            if let Some(arm) = spawn.arm.as_ref() {
                arm.set_lowmem_handshake(lowmem);
            }
        }
        Ok(op)
    }

    /// Post one Relog-now row. The owner holds its login half across the
    /// current session exit; the frontend only tracks the originating
    /// session for the operator's pending notice.
    pub(crate) fn request_memory_relog<S: SlotSurface<Io = Io>>(
        &mut self,
        name: &str,
        surface: &mut S,
    ) -> OperationId {
        let Some(arm) = self.play.as_ref().and_then(|play| play.arm(name)) else {
            return self.login(name, surface);
        };
        let op = self.operations.open(ActionKind::Logout);
        let previous = arm.session_key();
        self.post(name, &arm, Intent::RelogNow);
        self.mem_relog.insert(name.to_string(), previous);
        self.operations.cancel_pending(ActionKind::Logout, name);
        self.operations.set(op, name, Outcome::Pending);
        if let Some(play) = self.play.as_ref() {
            play.wake(name);
        }
        op
    }

    fn finish_memory_relogs(&mut self) {
        let statuses = &self.statuses;
        let play = self.play.as_ref();
        self.mem_relog.retain(|name, previous| {
            let logged_in = statuses
                .iter()
                .find(|status| status.username == *name)
                .is_some_and(|status| status.ingame);
            let arm = play.and_then(|play| play.arm(name));
            let failed = arm.as_ref().is_some_and(|arm| arm.login_held_by_error());
            let new_session = arm
                .as_ref()
                .and_then(|arm| arm.session_key())
                .is_some_and(|current| Some(current) != *previous);
            !((logged_in && new_session) || failed || arm.is_none())
        });
    }

    /// Adopt every running slot into the fleet (MultiBox on), cancelling
    /// their pending removals so the retained IO returns.
    pub(crate) fn seed_running(&mut self) {
        let running: Vec<String> = self
            .play
            .as_ref()
            .map(|p| p.statuses().into_iter().map(|s| s.username).collect())
            .unwrap_or_default();
        for name in &running {
            self.cancel_removal(name);
        }
        self.fleet.seed_running(&running);
        self.sort_members_by_order();
    }

    /// Remove a member and return immediately. A connected slot gets a
    /// bounded clean-logout window advanced by [`Self::poll`]; a
    /// disconnected worker is stopped asynchronously. Neither path sleeps
    /// or joins on the caller. When the removed member was selected, its
    /// neighbour is selected (or the selection cleared).
    pub(crate) fn remove<S: SlotSurface<Io = Io>>(
        &mut self,
        name: &str,
        now: Instant,
        surface: &mut S,
    ) -> Removal {
        let op = self.operations.open(ActionKind::Remove);
        if self.profile_saving(name) {
            self.operations.set(op, name, Outcome::failed(SAVING));
            return Removal {
                op,
                reselected: None,
                selection_cleared: false,
            };
        }
        let selected = self.selected.clone();
        let neighbour = self.fleet.focus_neighbour(name, selected.as_deref());
        self.fleet.remove(name);
        self.fleet.clear_latch(name);
        self.operations.cancel_pending(ActionKind::Login, name);
        self.deferred.remove(name);
        self.mem_relog.remove(name);
        let connected = self
            .play
            .as_ref()
            .is_some_and(|play| play.slot_connected(name));
        let retained = self
            .slots
            .remove(name)
            .or_else(|| self.removals.remove(name).and_then(|pending| pending.io));
        let arm = self.play.as_ref().and_then(|play| play.arm(name));
        match (connected, self.play.as_mut(), arm) {
            (true, Some(play), Some(arm)) => {
                // Clean logout only; Stop follows disconnect or timeout.
                arm.request_logout();
                play.wake(name);
                self.removals.insert(
                    name.to_string(),
                    PendingRemoval {
                        started: now,
                        arm,
                        io: retained,
                        op,
                    },
                );
            }
            (_, Some(play), _) => {
                self.removals.remove(name);
                play.begin_stop_slot(name);
            }
            (_, None, _) => {
                self.removals.remove(name);
            }
        }
        self.operations.set(op, name, Outcome::Pending);
        let walk_was_armed = self.frames.end_lifetime(name);
        surface.lifetime_reset(name, walk_was_armed);
        surface.released(name);
        let mut removal = Removal {
            op,
            reselected: None,
            selection_cleared: false,
        };
        if selected.as_deref() == Some(name) {
            match neighbour {
                Some(next) => {
                    self.select(&next);
                    removal.reselected = Some(next);
                }
                None => {
                    self.selected = None;
                    removal.selection_cleared = true;
                }
            }
        }
        removal
    }

    /// Advance removals, reap finished workers, resolve script Start/Stop
    /// for every slot (an offline or queued slot has no observe of its own),
    /// refresh status rows, record transitions, settle operations and
    /// refresh the projections ([`Self::fleet_view`]) and, once a second,
    /// the resource meter. Call once per UI frame or headless tick.
    pub fn poll(&mut self) {
        self.poll_at(Instant::now());
    }

    pub fn poll_at(&mut self, now: Instant) {
        self.advance_removals(now);
        self.poll_host_at(now);
    }

    /// Second half of [`Self::poll`]: resolve script Start/Stop, refresh
    /// rows and transitions, settle operations, then the projections and
    /// the meter. Front ends that must run their own work between the
    /// phases call the halves directly.
    pub fn poll_host(&mut self) {
        self.poll_host_at(Instant::now());
    }

    fn poll_host_at(&mut self, now: Instant) {
        self.sync_snapshot();
        // The saved Load order follows the vault before anything reads it
        // (both halves of a split poll reach here).
        self.reconcile_profile_order();
        self.poll_order_writes();
        self.refresh_fleet_result();
        self.transitions.clear();
        self.op_changes.clear();
        self.take_preparations(false);
        self.take_writes();
        self.take_native_deliveries(false);
        // Commands dispatched since the last poll come before the status
        // changes they caused; settlements follow them.
        self.log_operations();
        if let Some(play) = self.play.as_ref() {
            play.pump_script_lifecycles();
            play.statuses_into(&mut self.polled);
            self.poll_starts();
            record_transitions(&self.statuses, &self.polled, &mut self.transitions);
            std::mem::swap(&mut self.statuses, &mut self.polled);
            self.finish_memory_relogs();
            self.log_poll();
            self.settle_operations();
            self.log_operations();
        }
        self.refresh_views(now);
        self.resources
            .poll(now, self.play.as_ref(), self.selected.as_deref());
        self.refresh_capacity(now);
        self.settle_commands();
    }

    /// Evaluate host capacity once per [`CAPACITY_EVAL_EVERY`]; the notice
    /// text is rebuilt only when the warning's figures change.
    fn refresh_capacity(&mut self, now: Instant) {
        if self.capacity_due.is_some_and(|due| now < due) {
            return;
        }
        self.capacity_due = Some(now + CAPACITY_EVAL_EVERY);
        let eval = self.play.as_ref().map_or(CapacityEval::Clear, |play| {
            play.capacity()
                .evaluate(now, self.capacity_render.to_host())
        });
        self.apply_capacity(eval);
    }

    /// Show, keep, hide or clear the warning, logging each transition into
    /// or out of it once.
    fn apply_capacity(&mut self, eval: CapacityEval) {
        let was_warning = self.capacity_kind == CapacityKind::Warn;
        let kind = match eval {
            CapacityEval::Warn(host_warning) => {
                let warning = CapacityWarning::from_host(host_warning);
                if self.capacity.as_ref().map(CapacityNotice::warning) != Some(warning) {
                    self.capacity = Some(CapacityNotice::from_warning(warning));
                    self.capacity_generation = self.capacity_generation.wrapping_add(1);
                }
                if !was_warning {
                    if let Some(notice) = self.capacity.as_ref() {
                        host_log!(Category::Lifecycle, Level::Warn, "{}", notice.notice_text());
                    }
                }
                CapacityKind::Warn
            }
            CapacityEval::Silence => {
                if was_warning {
                    host_log!(
                        Category::Lifecycle,
                        Level::Info,
                        "Host warning hidden: no game packets for 10 s, so the game server is \
                         quiet, not this machine"
                    );
                }
                CapacityKind::Silence
            }
            CapacityEval::Clear => {
                if was_warning {
                    host_log!(Category::Lifecycle, Level::Info, "Host is keeping up again");
                }
                CapacityKind::Clear
            }
        };
        if kind != CapacityKind::Warn && self.capacity.take().is_some() {
            self.capacity_generation = self.capacity_generation.wrapping_add(1);
        }
        self.capacity_kind = kind;
    }

    /// Move the operation outcomes recorded since the last take onto each
    /// slot's log (`op#<id> <action> <outcome>`) and keep them for the
    /// rows.
    fn log_operations(&mut self) {
        let start = self.op_changes.len();
        self.operations.append_changes(&mut self.op_changes);
        if self.op_changes.len() == start {
            return;
        }
        let log = crate::log::global();
        for change in self.op_changes[start..]
            .iter()
            .filter(|c| crate::views::tracked(c))
        {
            self.op_line.clear();
            let _ = write_op(&mut self.op_line, change.id, change.action, &change.outcome);
            let (source, level) = crate::views::log_class(change);
            log.slot_line(&change.slot, source, level, &self.op_line);
        }
    }

    /// Replace the Start-all / marked-Start places used by [`update_row`].
    /// Called only when the admit queue published a change.
    pub fn publish_start_queue(&mut self, place_of: impl Fn(&str) -> Option<QueuePlace>) {
        self.start_places.clear();
        for name in self.fleet.members() {
            if let Some(place) = place_of(name) {
                self.start_places.insert(name.clone(), place);
            }
        }
        self.refresh_views(Instant::now());
    }

    fn refresh_views(&mut self, now: Instant) {
        let selected = self.selected.as_deref();
        let card = selected
            .and_then(|name| self.vault.as_ref()?.get(name))
            .and_then(|profile| profile.settings.script_assignment.as_ref())
            .map(|assignment| {
                if assignment.display_name.is_empty() {
                    assignment.identity.as_str()
                } else {
                    assignment.display_name.as_str()
                }
            });
        // Map before arm, the order every walk-arm user takes; an arm its
        // slot thread holds is not waited on (see `views::update_row`).
        let walks = Some(
            self.frames
                .walk_arms()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
        let input = Inputs {
            members: self.fleet.members(),
            selected,
            statuses: &self.statuses,
            play: self.play.as_ref(),
            walks: walks.as_deref(),
            card,
            start_places: &self.start_places,
            now,
        };
        self.views.refresh(&input, &self.op_changes);
    }

    /// Move this poll's transitions and every slot's staged script lines
    /// onto the shared log. Script lines stay readable for one poll through
    /// [`Self::script_lines`] (harness watches).
    fn log_poll(&mut self) {
        let log = crate::log::global();
        for change in &self.transitions {
            let (source, level, text): (Source, Level, std::borrow::Cow<'_, str>) =
                match change.transition.kind() {
                    TransitionKind::SlotUp => (Source::Host, Level::Info, "slot up".into()),
                    TransitionKind::LoginError => {
                        let e = change.transition.error().unwrap_or_default();
                        (Source::Login, Level::Error, format!("login {e}").into())
                    }
                    // Visible by default: the operator reads a stuck bot's
                    // story (logged in, which scene) at a glance. The host's
                    // LoadingScene/Ready phase lines say the same at Debug.
                    TransitionKind::Ingame => (Source::Login, Level::Info, "ingame".into()),
                    TransitionKind::Scene => {
                        let scene = change.transition.state().unwrap_or_default();
                        (Source::Host, Level::Info, format!("scene {scene}").into())
                    }
                    TransitionKind::Welcome => {
                        let line = change.transition.text().unwrap_or_default();
                        (Source::Login, Level::Info, line.into())
                    }
                    TransitionKind::WelcomeFailure => {
                        let line = change.transition.error().unwrap_or_default();
                        (Source::Login, Level::Warn, line.into())
                    }
                };
            log.slot_line(&change.slot, source, level, &text);
        }
        self.script_lines.clear();
        let Some(play) = self.play.as_ref() else {
            return;
        };
        for status in &self.statuses {
            for line in play.script_take_pending_logs(&status.username) {
                let (source, level) = crate::log::classify_script_line(&line);
                log.slot_line(&status.username, source, level, &line);
                self.script_lines.push((status.username.clone(), line));
            }
        }
    }

    /// Script lines taken by the last poll, `(slot, line)` in order.
    pub fn script_lines(&self) -> &[(String, String)] {
        &self.script_lines
    }

    /// First half of [`Self::poll`]: reap finished workers, start the
    /// spawns that waited for them and advance pending removals (stop on
    /// disconnect or timeout). Never joins a live worker.
    pub fn advance_removals(&mut self, now: Instant) {
        if let Some(play) = self.play.as_mut() {
            play.pump_worker_reaps();
        }
        self.spawn_deferred();
        if self.removals.is_empty() {
            return;
        }
        let ready: Vec<(String, bool)> = self
            .removals
            .iter()
            .map(|(name, pending)| {
                let current_arm = self.play.as_ref().and_then(|play| play.arm(name));
                let owns_current_lifetime = current_arm
                    .as_ref()
                    .is_some_and(|arm| Arc::ptr_eq(arm, &pending.arm));
                let disconnected = !self
                    .play
                    .as_ref()
                    .is_some_and(|play| play.slot_connected(name));
                let timed_out =
                    now.saturating_duration_since(pending.started) >= SLOT_REMOVE_TIMEOUT;
                // A missing arm means the pending lifetime ended by itself;
                // retire its preserved terminal row. A different arm is a
                // replacement: drop the stale entry, never stop the
                // replacement.
                (
                    name.clone(),
                    current_arm.is_none() || (owns_current_lifetime && (disconnected || timed_out)),
                )
            })
            .collect();
        for (name, stop) in ready {
            if stop {
                if let Some(play) = self.play.as_mut() {
                    play.begin_stop_slot(&name);
                }
            }
            let still_owner = self
                .play
                .as_ref()
                .and_then(|play| play.arm(&name))
                .is_some_and(|arm| {
                    self.removals
                        .get(&name)
                        .is_some_and(|pending| Arc::ptr_eq(&arm, &pending.arm))
                });
            if stop || !still_owner {
                self.removals.remove(&name);
            }
        }
    }

    /// Start every queued spawn whose previous worker has exited. It spawns
    /// from the durable row as it is now (a save made while it waited
    /// applies), keeping the surface's per-spawn render choices.
    fn spawn_deferred(&mut self) {
        if self.deferred.is_empty() {
            return;
        }
        let Some(play) = self.play.as_ref() else {
            return;
        };
        let ready: Vec<String> = self
            .deferred
            .keys()
            .filter(|name| !play.slot_stopping(name))
            .cloned()
            .collect();
        for name in ready {
            let Some(spawn) = self.deferred.remove(&name) else {
                continue;
            };
            // A replacement worker has its own mailbox and session; the
            // previous lifetime's pending notice cannot follow it.
            self.mem_relog.remove(&name);
            self.old_lifetime_retired(&name);
            // Deleted while it waited: nothing to spawn.
            let Some(mut profile) = self.durable_profile(&name).cloned() else {
                continue;
            };
            profile.settings.raster = spawn.profile.settings.raster;
            profile.settings.lowmem = spawn.profile.settings.lowmem;
            if let Some(arm) = spawn.arm.as_ref() {
                arm.set_auto_login(profile.settings.auto_login);
            }
            let Some(play) = self.play.as_mut() else {
                return;
            };
            if let Err(error) = play.try_spawn_slot(profile, spawn.input, spawn.mailbox, spawn.arm)
            {
                crate::log::global().slot_line(&name, Source::Host, Level::Error, &error);
            }
        }
    }

    fn poll_starts(&mut self) {
        let Some(play) = self.play.as_ref() else {
            return;
        };
        if self.starts.is_empty() {
            return;
        }
        let mut settled = Vec::new();
        for (name, op) in &self.starts {
            match play.script_poll_start(name) {
                script::StartPoll::Pending => {}
                script::StartPoll::Settled(outcome) => {
                    settled.push((name.clone(), *op, Some(outcome)))
                }
                // The slot was removed (its Stop cancelled the Start).
                script::StartPoll::NotOwed => settled.push((name.clone(), *op, None)),
            }
        }
        for (slot, op, outcome) in settled {
            self.starts.remove(&slot);
            let result = match &outcome {
                Some(script::StartOutcome::Ready) => Outcome::Completed,
                Some(script::StartOutcome::Failed(error)) => Outcome::failed(error.clone()),
                Some(script::StartOutcome::Rejected(error)) => Outcome::failed(error.to_string()),
                Some(script::StartOutcome::Cancelled) | None => Outcome::Cancelled,
            };
            self.operations.set(op, &slot, result);
            self.settled_starts.push(StartSettled { slot, op, outcome });
        }
    }

    fn settle_operations(&mut self) {
        let play = self.play.as_ref();
        let statuses = &self.statuses;
        let removals = &self.removals;
        let deferred = &self.deferred;
        self.operations.settle(|action, slot| {
            let row = statuses.iter().find(|s| s.username == slot);
            let arm = play.and_then(|p| p.arm(slot));
            match action {
                ActionKind::Login => {
                    let row = match row {
                        Some(row) => row,
                        None if arm.is_none() && !deferred.contains_key(slot) => {
                            return Some(Outcome::Cancelled)
                        }
                        None => return None,
                    };
                    if row.ingame {
                        Some(Outcome::Completed)
                    } else if let Some(error) = row.terminal_startup_error() {
                        Some(Outcome::failed(error.to_string()))
                    } else if row.worker_terminal.is_some() {
                        Some(Outcome::failed(
                            row.error.clone().unwrap_or_else(|| "worker ended".into()),
                        ))
                    } else if arm.as_ref().is_some_and(|arm| arm.login_latched()) {
                        // The arm, not the row: a row published before the
                        // Log in may still show the old latch.
                        Some(Outcome::Cancelled)
                    } else {
                        None
                    }
                }
                ActionKind::Logout => match row {
                    Some(row) if row.connected => None,
                    _ => Some(Outcome::Completed),
                },
                ActionKind::Remove => {
                    let stopping = play.is_some_and(|p| p.slot_stopping(slot));
                    (!removals.contains_key(slot) && arm.is_none() && !stopping)
                        .then_some(Outcome::Completed)
                }
                ActionKind::ScriptStop => {
                    let state = play.map_or(script::RunState::Idle, |p| p.script_state(slot));
                    matches!(state, script::RunState::Idle | script::RunState::Error)
                        .then_some(Outcome::Completed)
                }
                ActionKind::ScriptPause => {
                    match play.map_or(script::RunState::Idle, |p| p.script_state(slot)) {
                        script::RunState::Paused => Some(Outcome::Completed),
                        script::RunState::Idle | script::RunState::Error => {
                            Some(Outcome::skipped("no running script"))
                        }
                        _ => None,
                    }
                }
                ActionKind::ScriptResume => {
                    match play.map_or(script::RunState::Idle, |p| p.script_state(slot)) {
                        script::RunState::Paused => None,
                        script::RunState::Idle | script::RunState::Error => {
                            Some(Outcome::skipped("no script"))
                        }
                        _ => Some(Outcome::Completed),
                    }
                }
                // Settled at dispatch (Load, Select), by poll_starts, or by
                // the profile writer.
                ActionKind::Select
                | ActionKind::Load
                | ActionKind::ScriptStart
                | ActionKind::SaveProfile
                | ActionKind::DeleteProfile
                | ActionKind::SyncSettings => None,
            }
        });
    }

    /// Start a script on `name`. Both execution kinds settle through
    /// [`Self::poll`], even while the slot is offline or queued.
    /// `identity` is the Load identity; native installation owns its identity.
    /// [`Self::start_script`] for front-end tests; the refusal as text.
    #[cfg(any(test, feature = "test-support"))]
    pub fn start_script_for_test(
        &mut self,
        name: &str,
        start: ScriptStart,
        identity: Option<String>,
    ) -> Result<OperationId, String> {
        self.start_script(name, start, identity)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn start_script(
        &mut self,
        name: &str,
        start: ScriptStart,
        identity: Option<String>,
    ) -> Result<OperationId, script::StartLoadError> {
        let play = self
            .play
            .as_ref()
            .ok_or_else(|| script::StartLoadError::Refused("no play".into()))?;
        let compiled = start.is_compiled();
        match start.0 {
            ScriptStartKind::Compiled { id, bag } => play.script_start_typed(name, id, bag)?,
            #[cfg(feature = "js")]
            ScriptStartKind::Load {
                js,
                shape,
                bag,
                siblings,
            } => play.script_start_load_typed(name, js, shape, bag, siblings)?,
        }
        if !compiled {
            if let Some(identity) = identity {
                play.script_attach_identity(name, identity);
            }
        }
        let op = self.operations.open(ActionKind::ScriptStart);
        self.operations.set(op, name, Outcome::Pending);
        self.starts.insert(name.to_string(), op);
        Ok(op)
    }

    /// Pause `name`'s script, or resume it when paused.
    pub(crate) fn toggle_pause(&mut self, name: &str) -> OperationId {
        let paused = self
            .play
            .as_ref()
            .is_some_and(|play| play.script_state(name) == script::RunState::Paused);
        let action = if paused {
            ActionKind::ScriptResume
        } else {
            ActionKind::ScriptPause
        };
        let op = self.operations.open(action);
        match self.play.as_ref() {
            Some(play) if paused => play.script_resume(name),
            Some(play) => play.script_pause(name),
            None => {
                self.operations.set(op, name, Outcome::failed("no play"));
                return op;
            }
        }
        self.operations.set(op, name, Outcome::Pending);
        op
    }

    /// Stop `name`'s script (teardown hook, instance dropped). Settles when
    /// the host reports the slot idle, including while offline.
    pub(crate) fn stop_script(&mut self, name: &str) -> OperationId {
        let op = self.operations.open(ActionKind::ScriptStop);
        match self.play.as_ref() {
            Some(play) => {
                play.script_stop(name);
                self.operations.set(op, name, Outcome::Pending);
            }
            None => self.operations.set(op, name, Outcome::failed("no play")),
        }
        op
    }

    /// Stop every listed slot's script, including a slot still reaping a
    /// previous run with a replacement Start queued behind it: Stop drops
    /// that queued Start. Returns the operation and how many were stopped.
    pub(crate) fn stop_scripts(&mut self, names: &[String]) -> (OperationId, usize) {
        let op = self.operations.open(ActionKind::ScriptStop);
        let mut stopped = 0;
        for name in names {
            let state = self
                .play
                .as_ref()
                .map_or(script::RunState::Idle, |play| play.script_state(name));
            let outcome = match (state, self.play.as_ref()) {
                (
                    script::RunState::Running
                    | script::RunState::Paused
                    | script::RunState::Starting
                    | script::RunState::Stopping,
                    Some(play),
                ) => {
                    play.script_stop(name);
                    stopped += 1;
                    Outcome::Pending
                }
                _ => Outcome::skipped("no script"),
            };
            self.operations.set(op, name, outcome);
        }
        (op, stopped)
    }

    /// Stage `profile` in the in-memory vault and queue its durable write.
    /// Returns at once; the operation settles in [`Self::poll`], where a
    /// successful write applies `mirror` to a running slot and a failed one
    /// restores the durable value (unless a newer write is queued) and is
    /// reported through [`Self::take_write_failures`] prefixed by `label`.
    pub(crate) fn save_profile(
        &mut self,
        profile: Profile,
        mirror: ArmMirror,
        label: &'static str,
    ) -> Result<OperationId, String> {
        self.save_profile_with_log_action(profile, mirror, label, "profile save")
    }

    /// Save a profile with the action that should own any durable assignment
    /// transition it writes. Native preparations derive their action from
    /// their assignment intent instead.
    pub(crate) fn save_profile_with_log_action(
        &mut self,
        profile: Profile,
        mirror: ArmMirror,
        label: &'static str,
        assignment_action: &'static str,
    ) -> Result<OperationId, String> {
        if let Some((id, bag)) = mirror.native_draft() {
            return self.queue_native_settings(profile, mirror, label, id, bag, None);
        }
        self.ensure_writer();
        self.hold_durable(&profile.username)
            .ok_or_else(|| format!("{label}: vault locked"))?;
        api::hostlog::register_secret(&profile.password);
        if let Some(vault) = self.vault_for_write() {
            vault.stage_upsert(profile.clone());
        }
        Ok(self.submit_write(
            ActionKind::SaveProfile,
            vec![VaultChange::Upsert(profile)],
            mirror,
            label,
            assignment_action,
        ))
    }

    /// Stage a new profile like [`Self::save_profile`], refused when a
    /// profile of that username already exists: a new row never overwrites
    /// another profile.
    pub(crate) fn create_profile(
        &mut self,
        profile: Profile,
        mirror: ArmMirror,
        label: &'static str,
    ) -> Result<OperationId, String> {
        self.create_profile_with_log_action(profile, mirror, label, "profile save")
    }

    /// [`Self::create_profile`] with the action that owns its assignment log.
    pub(crate) fn create_profile_with_log_action(
        &mut self,
        profile: Profile,
        mirror: ArmMirror,
        label: &'static str,
        assignment_action: &'static str,
    ) -> Result<OperationId, String> {
        self.refuse_existing(&profile.username, label)?;
        self.save_profile_with_log_action(profile, mirror, label, assignment_action)
    }

    /// Rename `old` to `profile.username` as one transaction: the new row
    /// and the removal of the old one are written in a single commit, and a
    /// failure puts both back. Refused when another profile already has the
    /// new username: a rename never overwrites it.
    pub(crate) fn rename_profile(
        &mut self,
        old: &str,
        profile: Profile,
        mirror: ArmMirror,
        label: &'static str,
    ) -> Result<OperationId, String> {
        if old == profile.username {
            return self.save_profile(profile, mirror, label);
        }
        self.refuse_existing(&profile.username, label)?;
        if let Some((id, bag)) = mirror.native_draft() {
            return self.queue_native_settings(profile, mirror, label, id, bag, Some(old));
        }
        self.ensure_writer();
        self.hold_durable(&profile.username)
            .ok_or_else(|| format!("{label}: vault locked"))?;
        self.hold_durable(old);
        self.cancel_profile_start(old);
        api::hostlog::register_secret(&profile.password);
        if let Some(vault) = self.vault_for_write() {
            vault.stage_upsert(profile.clone());
            vault.stage_remove(old);
        }
        Ok(self.submit_write(
            ActionKind::SaveProfile,
            vec![
                VaultChange::Upsert(profile),
                VaultChange::Remove(old.to_string()),
            ],
            mirror,
            label,
            "profile rename",
        ))
    }

    fn refuse_existing(&self, username: &str, label: &str) -> Result<(), String> {
        if self
            .vault
            .as_ref()
            .is_some_and(|v| v.get(username).is_some())
        {
            return Err(format!(
                "{label}: a profile named {username} already exists"
            ));
        }
        Ok(())
    }

    /// Whether a profile editor holding (`username`, `password`, `settings`)
    /// for `target` has unsaved changes: anything its Save path would write
    /// differs from the vault row. For an existing target Save writes the
    /// canonicalized username (trimmed), the password as typed, and only
    /// the settings fields the credentials form owns: `world` and the
    /// trimmed `clue_duel_partner`. Every other setting (auto-login,
    /// random/lamp, raster, tutorial, script assignment/parameters, …) is
    /// either persisted immediately by its own control or preserved from the
    /// current vault row, so a concurrent change there never counts as
    /// dirty. `None` (or `""`) is the new-profile form: a typed name or
    /// password, or any non-default setting, counts. A missing row for an
    /// existing target counts as dirty, so leaving the form always asks
    /// instead of silently dropping the draft. The panel's Profiles form
    /// gates its explicit edit-target switch on this; the TUI settings popup
    /// persists each edit as it is made, so it never holds a draft.
    pub fn profile_form_dirty(&self, target: Option<&str>, draft: &crate::ProfileDraft) -> bool {
        let username = draft.username();
        let password = draft.password_secret().as_str();
        let settings = draft.settings().settings();
        match target.filter(|t| !t.is_empty()) {
            Some(name) => match self.vault.as_ref().and_then(|v| v.get(name)) {
                Some(row) => {
                    row.username != username.trim()
                        || row.password != password
                        || row.settings.world != settings.world
                        || row.settings.clue_duel_partner.trim()
                            != settings.clue_duel_partner.trim()
                }
                None => true,
            },
            None => {
                !username.trim().is_empty()
                    || !password.is_empty()
                    || *settings != ProfileSettings::default()
            }
        }
    }

    /// Delete a vault profile only; a live member is not logged out or
    /// dropped. Returns `None` when there was no such profile.
    pub(crate) fn vault_remove(&mut self, name: &str) -> Result<Option<OperationId>, String> {
        if self.profile_saving(name) {
            return Err(format!("chooser: {name}: {SAVING}"));
        }
        self.ensure_writer();
        let vault = self
            .vault
            .as_ref()
            .ok_or_else(|| "chooser: vault locked".to_string())?;
        if vault.get(name).is_none() {
            return Ok(None);
        }
        self.hold_durable(name);
        self.cancel_profile_start(name);
        if let Some(vault) = self.vault_for_write() {
            vault.stage_remove(name);
        }
        Ok(Some(self.submit_write(
            ActionKind::DeleteProfile,
            vec![VaultChange::Remove(name.to_string())],
            ArmMirror::none(),
            "chooser",
            "profile delete",
        )))
    }

    /// Removing an assignment owner wins against an unsettled Start, without
    /// stopping an independently running member during vault-only deletion.
    fn cancel_profile_start(&self, name: &str) {
        if let Some(play) = self.play.as_ref() {
            if self.starts.contains_key(name)
                || play.script_state(name) == script::RunState::Starting
            {
                play.script_stop(name);
            }
        }
    }

    /// The writer snapshots the durable vault, so it must exist before the
    /// first change is staged.
    fn ensure_writer(&mut self) {
        if self.writer.is_none() {
            if let Some(vault) = self.vault.as_ref() {
                self.writer = Some(ProfileWriter::spawn(
                    vault.store(),
                    #[cfg(any(test, feature = "test-support"))]
                    Arc::clone(&self.write_gate),
                ));
            }
        }
    }

    fn submit_write(
        &mut self,
        action: ActionKind,
        changes: Vec<VaultChange>,
        mirror: ArmMirror,
        label: &'static str,
        assignment_action: &'static str,
    ) -> OperationId {
        let op = self.operations.open(action);
        self.submit_write_at(op, changes, mirror, label, assignment_action);
        op
    }

    fn submit_write_at(
        &mut self,
        op: OperationId,
        changes: Vec<VaultChange>,
        mirror: ArmMirror,
        label: &'static str,
        assignment_action: &'static str,
    ) {
        // The operation's member is the profile the edit is about (a
        // rename's new name).
        let member = changes[0].username().to_string();
        self.operations.set(op, &member, Outcome::Pending);
        for change in &changes {
            self.latest_write.insert(change.username().to_string(), op);
            if matches!(change, VaultChange::Remove(_)) {
                self.profile_edits.insert(change.username().to_string(), op);
            }
        }
        if let Some(card) = mirror.native_identity() {
            self.native_edits.insert((member.clone(), card), op);
        }
        let audit = if matches!(&mirror.0, ArmMirrorKind::Survival { .. }) {
            Some(ProfileAuditKind::SurvivalCurePoison)
        } else {
            mirror
                .script_settings_identity()
                .map(ProfileAuditKind::ScriptSettings)
        };
        self.writes.insert(
            op,
            PendingWrite {
                label,
                mirror,
                member,
                copy_start_hold: None,
                copy_abandoned: false,
            },
        );
        if let Some(writer) = self.writer.as_mut() {
            writer.submit(op, changes, assignment_action, audit);
        }
    }

    /// Persist a profile's auto-login; a running slot's arm follows once the
    /// write is durable. Never spawns or stops a slot.
    pub(crate) fn set_auto_login(&mut self, name: &str, on: bool) -> Result<OperationId, String> {
        let mut profile = self
            .vault
            .as_ref()
            .ok_or_else(|| "auto-login: vault locked".to_string())?
            .get(name)
            .cloned()
            .ok_or_else(|| format!("auto-login: no profile {name}"))?;
        profile.settings.auto_login = on;
        self.save_profile(profile, ArmMirror::auto_login(on), "auto-login")
    }

    /// Persist a profile's random-event guardian fields; a running slot's
    /// arm follows once the write is durable, so toggling off never acts or
    /// holds without a respawn.
    pub(crate) fn set_random_settings(
        &mut self,
        name: &str,
        random_events: bool,
        lamp_skill: &str,
        lamp_auto: bool,
    ) -> Result<OperationId, String> {
        let mut profile = self
            .vault
            .as_ref()
            .ok_or_else(|| "random: vault locked".to_string())?
            .get(name)
            .cloned()
            .ok_or_else(|| format!("random: no profile {name}"))?;
        profile.settings.random_events = random_events;
        profile.settings.lamp_skill = lamp_skill.to_string();
        profile.settings.lamp_auto = lamp_auto;
        let mirror = ArmMirror::guardian(random_events, lamp_skill.to_string(), lamp_auto);
        self.save_profile(profile, mirror, "random")
    }

    /// Persist one hostile random's policy and mirror it to a running slot
    /// once the vault write is durable.
    pub(crate) fn set_random_event_policy(
        &mut self,
        name: &str,
        event: &str,
        policy: crate::GuardianPolicy,
    ) -> Result<OperationId, String> {
        let mut profile = self
            .vault
            .as_ref()
            .ok_or_else(|| "random: vault locked".to_string())?
            .get(name)
            .cloned()
            .ok_or_else(|| format!("random: no profile {name}"))?;
        let event = event.trim().to_ascii_lowercase();
        profile
            .settings
            .set_random_event_policy(&event, policy.to_vault());
        self.save_profile(
            profile,
            ArmMirror::random_event_policy(event, policy),
            "random",
        )
    }

    /// Persist a profile's Cure poison setting through `save_profile`
    /// (the only write path); a running slot's arm follows once the
    /// write is durable, so the slot sees the change within one frame
    /// and never before it is saved.
    pub(crate) fn set_survival_settings(
        &mut self,
        name: &str,
        cure_poison: crate::CurePoison,
    ) -> Result<OperationId, String> {
        let mut profile = self
            .vault
            .as_ref()
            .ok_or_else(|| "survival: vault locked".to_string())?
            .get(name)
            .cloned()
            .ok_or_else(|| format!("survival: no profile {name}"))?;
        profile.settings.survival.cure_poison = cure_poison.to_vault();
        self.save_profile(profile, ArmMirror::survival(cure_poison), "survival")
    }

    /// The profile a form's Save writes: the draft's credentials over the
    /// existing row's uid and settings (a form edits only its world and clue
    /// partner), or a new row with a fresh uid and the draft's settings.
    /// `None` while the vault is locked.
    pub(crate) fn compose_profile(
        &self,
        rename_from: Option<&str>,
        username: &str,
        draft: &crate::ProfileDraft,
        form_open: bool,
    ) -> Option<Profile> {
        let vault = self.vault.as_ref()?;
        let existing = vault.get(rename_from.unwrap_or(username)).cloned();
        let staged = draft.settings().settings();
        Some(Profile {
            uid: existing
                .as_ref()
                .map(|p| p.uid)
                .unwrap_or_else(|| self.fresh_uid()),
            username: username.to_string(),
            password: draft.password_secret().clone(),
            settings: existing
                .map(|p| {
                    let mut settings = p.settings;
                    if form_open {
                        settings.world = staged.world;
                        settings.clue_duel_partner = staged.clue_duel_partner.trim().to_string();
                    }
                    settings
                })
                .unwrap_or_else(|| staged.clone()),
        })
    }

    /// Persist the cached TutSkip state of `name` (an engine `getvar` read,
    /// or the operator's TutSkip through
    /// [`crate::OperatorCommand::MarkTutorialSkipped`]). Unchanged state
    /// writes nothing.
    /// Adopt the engine's `getvar tutorial` reading of `name` as its
    /// cached TutSkip state: an owner result, not an operator decision
    /// (the operator's TutSkip is [`crate::OperatorCommand::MarkTutorialSkipped`]).
    pub fn adopt_tutorial_reading(
        &mut self,
        name: &str,
        skipped: bool,
    ) -> Result<(), crate::profile_saves::ProfileWriteRefused> {
        self.record_tutorial_state(name, skipped)
            .map(|_| ())
            .map_err(crate::profile_saves::ProfileWriteRefused)
    }

    pub(crate) fn record_tutorial_state(
        &mut self,
        name: &str,
        skipped: bool,
    ) -> Result<Option<OperationId>, String> {
        let Some(mut profile) = self.vault.as_ref().and_then(|v| v.get(name)).cloned() else {
            return Ok(None);
        };
        if profile.settings.tutorial_skipped == Some(skipped) {
            return Ok(None);
        }
        profile.settings.tutorial_skipped = Some(skipped);
        self.save_profile(profile, ArmMirror::none(), "tutorial")
            .map(Some)
    }

    /// Persist `name`'s game-image raster mode.
    pub(crate) fn set_profile_raster(
        &mut self,
        name: &str,
        raster: crate::command::RasterMode,
    ) -> Result<Option<OperationId>, String> {
        let Some(mut profile) = self.vault.as_ref().and_then(|v| v.get(name)).cloned() else {
            return Ok(None);
        };
        profile.settings.raster = raster.to_vault();
        self.save_profile(profile, ArmMirror::none(), "render prefs")
            .map(Some)
    }

    /// Whether a vault is open.
    pub fn vault_open(&self) -> bool {
        self.vault.is_some()
    }

    /// The open vault for a staged profile change; moves
    /// [`Self::profiles_generation`] so a fleet model rebuilds its
    /// profile universe.
    pub(crate) fn vault_for_write(&mut self) -> Option<&mut Vault> {
        self.profiles_generation = self.profiles_generation.wrapping_add(1);
        self.vault.as_mut()
    }

    /// Moves whenever the vault's profile rows may have changed (a staged
    /// create, rename, delete or settings write, a restore after a failed
    /// write, or a vault opened or replaced).
    pub fn profiles_generation(&self) -> u64 {
        self.profiles_generation
    }

    /// The staged (vault) row of `name`, as the forms show it.
    pub fn profile(&self, name: &str) -> Option<crate::ProfileView<'_>> {
        self.vault.as_ref()?.get(name).map(crate::ProfileView::new)
    }

    /// Every vault profile, in vault order.
    pub fn profiles(&self) -> impl Iterator<Item = crate::ProfileView<'_>> {
        self.vault
            .iter()
            .flat_map(|vault| vault.profiles().map(crate::ProfileView::new))
    }

    /// Whether `name` has no durable row yet: its first save (new or
    /// renamed profile) is still being written. Such a profile cannot be
    /// selected, loaded, removed or deleted until the write settles.
    pub fn profile_saving(&self, name: &str) -> bool {
        matches!(self.durable.get(name), Some(None))
    }

    /// The profile as last committed to disk: what a spawn or arm may use.
    /// Equal to the vault row unless a write for `name` is still queued.
    pub fn durable_profile(&self, name: &str) -> Option<&Profile> {
        match self.durable.get(name) {
            Some(durable) => durable.as_ref(),
            None => self.vault.as_ref().and_then(|v| v.get(name)),
        }
    }

    /// Record `name`'s durable value before its first queued change is
    /// staged. `None` when the vault is locked.
    fn hold_durable(&mut self, name: &str) -> Option<()> {
        let current = self.vault.as_ref()?.get(name).cloned();
        self.durable.entry(name.to_string()).or_insert(current);
        Some(())
    }

    /// Failed profile writes since the last take, for the banner or status
    /// line (each displays as `label: error`).
    pub fn take_write_failures(&mut self) -> Vec<WriteFailure> {
        std::mem::take(&mut self.write_failures)
    }

    /// Register the form save `record` under its write's operation. The
    /// write settles in [`Self::poll`]; the record then moves from
    /// [`Self::saves_in_flight`] to [`Self::take_settled_saves`], with the
    /// write's result.
    pub(crate) fn track_save(&mut self, record: SaveRecord) {
        self.saves.track(record);
    }

    /// The form saves whose writes have not settled, oldest first.
    pub fn saves_in_flight(&self) -> &[SaveRecord] {
        self.saves.in_flight()
    }

    /// The form saves settled since the last take, in the order they
    /// settled.
    pub fn take_settled_saves(&mut self) -> Vec<SaveSettled> {
        self.saves.take_settled()
    }

    /// Script-parameter writes settled since the last take: what was saved
    /// and, separately, what reached the run captured at edit time.
    pub fn take_settings_writes(&mut self) -> Vec<SettingsWrite> {
        std::mem::take(&mut self.settings_writes)
    }

    pub(crate) fn take_script_assignment_write(&mut self) -> Option<ScriptAssignmentWrite> {
        self.script_assignment_writes.pop_front()
    }

    /// Wait for every queued profile write and settle it. For startup and
    /// harness boundaries only: it blocks on disk I/O, so frame paths use
    /// [`Self::poll`] instead.
    pub fn flush_writes(&mut self) {
        self.take_preparations(true);
        let mut settled = false;
        while let Some(written) = self.writer.as_mut().and_then(ProfileWriter::wait_take) {
            self.settle_write(written);
            settled = true;
        }
        if settled {
            self.reconcile_after_writes();
        }
        self.take_native_deliveries(true);
    }

    fn take_writes(&mut self) {
        let mut settled = false;
        while let Some(written) = self.writer.as_mut().and_then(ProfileWriter::try_take) {
            self.settle_write(written);
            settled = true;
        }
        if settled {
            self.reconcile_after_writes();
        }
        self.release_abandoned_copy_writes();
    }

    fn settle_write(&mut self, written: Written) {
        let script_settings_log = self.script_settings_logs.remove(&written.op);
        let Some(mut pending) = self.writes.remove(&written.op) else {
            return;
        };
        let audit_before = written.audit_before;
        let profile_name = pending.member.as_str();
        let committed_profile = written
            .durable
            .iter()
            .find(|(name, _)| name == profile_name)
            .and_then(|(_, profile)| profile.as_ref());
        let own_setting = written.superseded
            && self.keeps_own_setting(profile_name, &pending.mirror, &written.later);
        let settles_successfully = written.result.is_ok()
            && !pending.copy_abandoned
            && (!written.superseded || own_setting);
        // Audit lines use the writer's pre-commit value and the committed row.
        let setting_log = if settles_successfully {
            script_settings_log.and_then(|audit| {
                committed_profile
                    .map(|after| audit.details(audit_before.as_deref(), &after.settings))
            })
        } else {
            None
        };
        let survival_transition = if settles_successfully {
            match (
                &pending.mirror.0,
                audit_before.as_deref(),
                committed_profile,
            ) {
                (
                    ArmMirrorKind::Survival { .. },
                    Some(ProfileAuditBefore::SurvivalCurePoison(old)),
                    Some(after),
                ) => {
                    let old = crate::CurePoison::from_vault(*old);
                    let new = crate::CurePoison::from_vault(after.settings.survival.cure_poison);
                    (old != new).then_some((old, new))
                }
                _ => None,
            }
        } else {
            None
        };

        // Profiles whose newest queued write this is: only those may be put
        // back on failure, so an older failure cannot undo a newer edit.
        let mut newest = Vec::new();
        let mut committed = None;
        for (name, durable) in written.durable {
            if name == pending.member {
                committed.clone_from(&durable);
            }
            if self.latest_write.get(&name) == Some(&written.op) {
                self.latest_write.remove(&name);
                self.durable.remove(&name);
                newest.push((name, durable));
            } else if let Some(held) = self.durable.get_mut(&name) {
                // A newer write is still queued: track what is durable now.
                held.clone_from(&durable);
            }
        }
        if pending.copy_abandoned {
            // The copy wait already failed. Still reconcile a late writer's
            // durable result, but do not deliver or announce a second result.
            if written.result.is_err() {
                if let Some(vault) = self.vault_for_write() {
                    for (name, durable) in newest {
                        vault.restore(&name, durable);
                    }
                }
            }
            return;
        }
        let member = pending.member;
        let script_assignment = matches!(&pending.mirror.0, ArmMirrorKind::ScriptAssignment);
        let script_settings = matches!(
            &pending.mirror.0,
            ArmMirrorKind::ScriptSettings { .. } | ArmMirrorKind::NativeSettings { .. }
        );

        // Form saves use this operation's commit result, even when a later
        // write in the same commit also updates the profile.

        let tracked = self.saves.tracks(written.op);
        let durable_row = committed.is_some();
        let commit_error = written.result.as_ref().err().filter(|_| tracked).cloned();
        let result = match written.result {
            Ok(()) if !written.superseded || own_setting => {
                self.operations.set(written.op, &member, Outcome::Completed);
                if script_settings {
                    if let Some(details) = setting_log.as_deref() {
                        crate::log::global().slot_line(
                            &member,
                            Source::Script,
                            Level::Info,
                            &format!(
                                "saved script settings; profile {member}; {details}; op#{}",
                                written.op.0
                            ),
                        );
                    }
                }
                if let Some((old, new)) = survival_transition {
                    crate::log::global().slot_line(
                        &member,
                        Source::Host,
                        Level::Info,
                        &format!(
                            "survival cure_poison {} -> {}; profile {member}; action survival; op#{}",
                            old.key(),
                            new.key(),
                            written.op.0
                        ),
                    );
                }

                if tracked {
                    let saved = if durable_row {
                        SaveResult::Saved
                    } else {
                        SaveResult::Gone
                    };
                    self.saves.settle(written.op, saved);
                }
                if matches!(
                    &pending.mirror.0,
                    ArmMirrorKind::Remember(Some(live)) if live.run.is_some()
                ) {
                    let ArmMirrorKind::Remember(Some(live)) =
                        std::mem::replace(&mut pending.mirror, ArmMirror::none()).0
                    else {
                        unreachable!("Remember(Some) with a run was matched above");
                    };
                    if let Some(play) = self.play.as_mut() {
                        if let Some(profile) = committed {
                            play.remember_profile(profile);
                        }
                    }
                    self.queue_native_delivery(written.op, member, live);
                    return;
                }
                // Operation IDs order profile saves across commits, not just
                // within a writer batch. A late memory completion must never
                // re-arm a mode the operator has since cancelled.
                let delivery = if matches!(&pending.mirror.0, ArmMirrorKind::Memory(_))
                    && !newest.iter().any(|(name, _)| *name == member)
                {
                    LiveDelivery::NotRunning
                } else {
                    self.apply_mirror(&member, pending.mirror, committed)
                };
                SettingsResult::saved(delivery)
            }
            Err(error) if !written.superseded => {
                // A failed memory toggle put the arm ahead of the restored
                // durable row: pull it back so the next handshake sends the
                // restored value, never the refused one.
                let reset_lowmem = match &pending.mirror.0 {
                    ArmMirrorKind::Memory(_) => newest
                        .iter()
                        .find(|(name, _)| *name == member)
                        .and_then(|(_, durable)| durable.as_ref())
                        .map(|profile| profile.settings.lowmem),
                    _ => None,
                };
                if let Some(vault) = self.vault_for_write() {
                    for (name, durable) in newest {
                        vault.restore(&name, durable);
                    }
                }
                if let Some(lowmem) = reset_lowmem {
                    if let Some(play) = self.play.as_mut() {
                        if let Some(arm) = play.arm(&member) {
                            arm.set_lowmem_handshake(lowmem);
                        }
                    }
                    if let Some(spawn) = self.deferred.get_mut(&member) {
                        spawn.profile.settings.lowmem = lowmem;
                        if let Some(arm) = spawn.arm.as_ref() {
                            arm.set_lowmem_handshake(lowmem);
                        }
                    }
                }
                let failure =
                    WriteFailure::new(written.op, member.clone(), pending.label, error.clone());
                if tracked {
                    self.saves
                        .settle(written.op, SaveResult::failed(failure.duplicate()));
                }
                self.write_failures.push(failure);
                self.operations
                    .set(written.op, &member, Outcome::failed(error.clone()));
                SettingsResult::failed(error)
            }
            _ => {
                self.operations.set(written.op, &member, Outcome::Cancelled);
                if tracked {
                    // Reported once, through the write that owns the row.
                    let result = match commit_error {
                        Some(error) => SaveResult::failed(WriteFailure::new(
                            written.op,
                            member.clone(),
                            pending.label,
                            error,
                        )),
                        None if durable_row => SaveResult::Saved,
                        None => SaveResult::Gone,
                    };
                    self.saves.settle(written.op, result);
                }
                SettingsResult::Superseded
            }
        };
        if script_assignment {
            self.script_assignment_writes
                .push_back(ScriptAssignmentWrite {
                    op: written.op,
                    result,
                });
        } else if script_settings {
            self.settings_writes
                .push(SettingsWrite::new(written.op, member, result));
        }
    }

    /// Whether a superseded write still settles on its own: it carries a
    /// setting (a live effect or a card's parameters) that no later write
    /// of `member` in its commit (`later`) replaces.
    fn keeps_own_setting(&self, member: &str, mirror: &ArmMirror, later: &[OperationId]) -> bool {
        !matches!(&mirror.0, ArmMirrorKind::None)
            && !later
                .iter()
                .filter_map(|op| self.writes.get(op))
                .any(|next| next.member == member && next.mirror.replaces(mirror))
    }

    /// Apply a durable write's live effect. Returns what a script-parameter
    /// push did (`NotRunning` for every other mirror).
    fn apply_mirror(
        &mut self,
        name: &str,
        mirror: ArmMirror,
        committed: Option<Profile>,
    ) -> LiveDelivery {
        let Some(play) = self.play.as_mut() else {
            return LiveDelivery::NotRunning;
        };
        match mirror.0 {
            ArmMirrorKind::None => {}
            ArmMirrorKind::ScriptAssignment => {}
            ArmMirrorKind::AutoLogin(on) => {
                if let Some(arm) = play.arm(name) {
                    arm.set_auto_login(on);
                }
                play.wake(name);
            }
            ArmMirrorKind::Guardian {
                random_events,
                lamp_skill,
                lamp_auto,
            } => {
                if let Some(arm) = play.arm(name) {
                    arm.random_events.store(random_events, Ordering::Relaxed);
                    arm.lamp_auto.store(lamp_auto, Ordering::Relaxed);
                    *arm.lamp_skill.lock().unwrap() = lamp_skill;
                }
            }
            ArmMirrorKind::RandomEventPolicy { event, policy } => {
                if let Some(arm) = play.arm(name) {
                    arm.random_event_policies
                        .lock()
                        .unwrap()
                        .insert(event, policy);
                }
            }
            ArmMirrorKind::Survival { new } => {
                if let Some(arm) = play.arm(name) {
                    arm.set_cure_poison(new.to_vault());
                }
            }
            ArmMirrorKind::Memory(lowmem) => {
                // Re-affirm the toggle the command already armed: the
                // durable row and the handshake channel agree again.
                if let Some(arm) = play.arm(name) {
                    arm.set_lowmem_handshake(lowmem);
                }
            }
            ArmMirrorKind::Remember(live) => {
                // The committed row, not the staged vault (a newer edit may
                // be queued behind this one).
                if let Some(profile) = committed {
                    play.remember_profile(profile);
                }
                return deliver_settings(play, name, live);
            }
            ArmMirrorKind::ScriptSettings { live, .. } => {
                return deliver_settings(play, name, live)
            }
            ArmMirrorKind::NativeSettings { .. } => {
                return LiveDelivery::rejected("native settings were not prepared")
            }
        }
        LiveDelivery::NotRunning
    }
}

/// Post a durable bag to the run it was edited for. A different identity
/// or generation means the run it targeted is gone: that run is never
/// posted to (a replacement Start already read the saved bag).
fn deliver_settings(play: &Play, name: &str, live: Option<LiveSettings>) -> LiveDelivery {
    let Some(live) = live else {
        return LiveDelivery::NotRunning;
    };
    let same_run = play.script_source_identity(name).as_deref() == Some(live.identity.as_str())
        && play.script_runtime_generation(name) == Some(live.generation);
    if !same_run {
        return LiveDelivery::Stale;
    }
    if let Some(target) = live.run {
        let Some(config) = live.prepared else {
            return LiveDelivery::rejected("native settings were not prepared");
        };
        return match play.script_configure_compiled(name, config, target) {
            script::CompiledDelivery::Applied => LiveDelivery::Applied,
            script::CompiledDelivery::PendingBoundary => LiveDelivery::PendingBoundary,
            script::CompiledDelivery::RestartRequired => LiveDelivery::RestartRequired,
            script::CompiledDelivery::Unchanged => LiveDelivery::Unchanged,
            script::CompiledDelivery::Stale => LiveDelivery::Stale,
            script::CompiledDelivery::NotRunning => LiveDelivery::NotRunning,
            script::CompiledDelivery::Rejected(error) => LiveDelivery::rejected(format!(
                "{}: {} ({})",
                error.field, error.message, error.code
            )),
        };
    }
    if play.script_post_settings_fenced(name, &live.bag, &live.identity, live.generation) {
        return LiveDelivery::Delivered;
    }
    match play.script_state(name) {
        script::RunState::Idle | script::RunState::Error => LiveDelivery::NotRunning,
        _ => LiveDelivery::Unchanged,
    }
}

#[cfg(any(test, feature = "test-support"))]
impl<Io> OperatorSession<Io> {
    /// Fixture seam: spawn attaches the arm to the play without a worker
    /// thread, so lifecycle tests never contact a server.
    pub fn set_spawn_workers(&mut self, on: bool) {
        self.spawn_workers = on;
    }

    /// Fixture seam: spawned workers skip client asset startup and go
    /// straight to the login queue (a fake login server needs no cache).
    pub fn set_bypass_asset_startup(&mut self, on: bool) {
        self.bypass_asset_startup = on;
    }

    /// Fixture seam: the meter reads its process counters from `probe`
    /// (a failing, unsupported or counting probe) instead of the OS.
    pub fn set_resource_probe(&mut self, probe: fn() -> crate::resources::ProcessProbe) {
        self.resources = Resources::with_probe(probe);
    }

    /// Rows re-derived since the session started (their facts changed).
    #[cfg(test)]
    pub(crate) fn rows_rebuilt(&self) -> u64 {
        self.views.rebuilt
    }

    /// Direct vault access for fixture setup. Settles queued writes first
    /// and lets the next write snapshot whatever the fixture persisted.
    #[cfg(any(test, feature = "test-support"))]
    pub fn vault_mut(&mut self) -> Option<&mut Vault> {
        self.flush_writes();
        self.writer = None;
        self.vault_for_write()
    }

    /// Held by the writer while it gathers and commits a batch; holding it
    /// queues several writes into one batch.
    #[cfg(any(test, feature = "test-support"))]
    pub fn write_gate(&self) -> Arc<std::sync::Mutex<()>> {
        Arc::clone(&self.write_gate)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn set_vault(&mut self, vault: Option<Vault>) {
        self.flush_writes();
        self.vault = vault;
        self.profiles_generation = self.profiles_generation.wrapping_add(1);
        self.writer = None;
    }

    pub fn set_play(&mut self, play: Option<Play>) {
        if let Some(play) = play.as_ref() {
            self.frames.bind_world(play.world(), play.map_members());
        }
        self.play = play;
    }

    pub fn insert_slot_io(&mut self, name: &str, io: Io) {
        self.slots.insert(name.to_string(), io);
    }

    pub fn fleet_mut(&mut self) -> &mut Fleet {
        &mut self.fleet
    }
}

fn completed_or_failed(result: Result<(), String>) -> Outcome {
    match result {
        Ok(()) => Outcome::Completed,
        Err(error) => Outcome::failed(error),
    }
}

fn record_transitions(
    previous: &[SlotStatus],
    current: &[SlotStatus],
    out: &mut Vec<SlotTransition>,
) {
    let mut push = |slot: &str, transition| {
        out.push(SlotTransition {
            slot: slot.to_string(),
            transition,
        })
    };
    for (index, s) in current.iter().enumerate() {
        let name = s.username.as_str();
        // Rows keep their order between polls: check the same index first.
        let before = previous
            .get(index)
            .filter(|p| p.username == s.username)
            .or_else(|| previous.iter().find(|p| p.username == s.username));
        match before {
            None => {
                push(name, Transition::SlotUp);
                if let Some(e) = &s.error {
                    push(name, Transition::login_error(e.clone()));
                }
            }
            Some(p) => {
                if p.error.is_none() {
                    if let Some(e) = &s.error {
                        push(name, Transition::login_error(e.clone()));
                    }
                }
                if !p.ingame && s.ingame {
                    push(name, Transition::Ingame);
                }
                if p.scene_state != s.scene_state {
                    push(name, Transition::scene(s.scene_state));
                }
                if p.welcome_notice != s.welcome_notice {
                    if let Some(line) = &s.welcome_notice {
                        push(name, Transition::welcome(line.clone()));
                    }
                }
                if p.welcome_failure.is_none() {
                    if let Some(line) = &s.welcome_failure {
                        push(name, Transition::welcome_failure(line.clone()));
                    }
                }
            }
        }
    }
}

/// Log a saved Load order write that failed outside a command.
fn order_write_failed(error: &crate::prefs::PreferenceError) {
    host_log!(
        Category::Lifecycle,
        Level::Warn,
        "profile order: the saved Load order was not written: {error}"
    );
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
