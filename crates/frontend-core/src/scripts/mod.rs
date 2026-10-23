//! Script coordination shared by the panel and the TUI: the card library
//! and catalog, per-profile assignment and pending Browse selection,
//! per-profile parameter bags (legacy migration, typed edits, live push),
//! Start / Start all (paced through a login-FIFO-style permit) / Stop all with their settlement, the reload and
//! catalog-refresh transaction ([`reload`]) and bulk parameter sync
//! ([`sync`]).
//!
//! [`Scripts`] holds coordination state only; every operation takes the
//! [`OperatorSession`] that owns the vault, the play and the profile
//! writer. The host still owns isolate lifecycle and execution identity.
//! Operator-facing text is published as one [`Notice`] the front end shows
//! on its message line, in call order (the last one wins).

mod loadouts;
mod native;
mod paint;
pub mod paths;
mod vocab;
pub(crate) use loadouts::Refused;
pub use loadouts::{LoadoutForm, Loadouts, LoadoutsError, LoadoutsErrorKind, WornSlot};
pub use native::{NativeDetail, SchemaView};
pub(crate) use paint::{
    advertises_any_control, advertises_control, is_host_control, ControlActivation, PaintRun,
    RunToken,
};
pub use paint::{
    map_applet_rect, CanvasCache, CanvasOps, CanvasRaster, ChromeBandView, PaintButtonView,
    PaintControl, PaintFrame, PaintView,
};
pub use vocab::{
    format_setting_value, CardRuntime, CardSource, CompiledCardId, LoadFailureView, LoadShape,
    LoadStage, LoadedCard, OwnedSchema, SchemaRef, ScriptChoice, ScriptRunState, SettingView,
    StatusView,
};

mod marked;
mod parameter_edit;
mod reload;
mod start_admit;
mod start_tally;
use parameter_edit::ParameterEdit;
pub use parameter_edit::{
    parse_parameter_text, resolve_parameter_options, ParameterCommit, ParameterEditKey,
    ParameterOptions,
};

mod sync;

use std::collections::{HashMap, VecDeque};
use std::fmt::Write as _;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use api::identity::Settlement;
pub use start_admit::START_ADMIT_PER_FRAME;
use start_admit::{QueuedStart, StartAdmit, StartPermit};
use start_tally::{Outcome, StartTally};

use api::hostlog::{Level, Source};
use serde_json::{Map, Value};
use vault::ScriptAssignment;

use crate::command::{CommandFailure, Completion, FailureKind, Subject};
use crate::operations::OperationId;
use crate::session::{
    ArmMirror, OperatorSession, ScriptAssignmentWrite, ScriptStart, StartSettled,
};
use crate::QueuePlace;

pub use reload::{
    CatalogError, PendingReload, PendingReloadKind, ReloadOutcome, ReloadWarning, TranspileError,
};
pub use sync::{RestartPrompt, SyncReport, SyncScope};

/// A card's parameters bound to the run they were edited for.
#[derive(Debug)]
pub struct LiveSettings {
    /// Card identity and run generation the push is fenced to.
    pub(crate) identity: String,
    pub(crate) generation: u64,
    /// Native controls also bind the worker incarnation and session.
    pub(crate) run: Option<api::selected::RunKey>,
    /// The run's whole bag: the card's merged parameters and its profile's
    /// global settings.
    pub(crate) bag: Arc<Map<String, Value>>,
    /// Present only after the native preparer has validated the complete bag.
    pub(crate) prepared: Option<Arc<script::native::PreparedConfig>>,
}

impl PartialEq for LiveSettings {
    fn eq(&self, other: &Self) -> bool {
        self.identity == other.identity
            && self.generation == other.generation
            && self.run == other.run
            && self.bag == other.bag
            && match (&self.prepared, &other.prepared) {
                (None, None) => true,
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                _ => false,
            }
    }
}

impl LiveSettings {
    /// Card identity the push is fenced to.
    pub fn identity(&self) -> &str {
        &self.identity
    }

    /// Run generation the push is fenced to.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Native controls also bind the worker incarnation and session.
    pub fn run(&self) -> Option<api::selected::RunKey> {
        self.run
    }

    /// The run's whole bag.
    pub fn bag(&self) -> &Map<String, Value> {
        &self.bag
    }

    /// Present only after the native preparer has validated the complete bag.
    #[cfg(any(test, feature = "test-support"))]
    pub fn prepared(&self) -> Option<&script::native::PreparedConfig> {
        self.prepared.as_deref()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LiveDeliveryKind {
    NotRunning,
    Delivered,
    Applied,
    PendingBoundary,
    RestartRequired,
    Rejected,
    Unchanged,
    Stale,
}

/// What a durable parameter write did to the run captured at edit time.
#[derive(Debug, PartialEq, Eq)]
pub struct LiveDelivery {
    kind: LiveDeliveryKind,
    reason: Option<String>,
}

#[allow(non_upper_case_globals)]
impl LiveDelivery {
    /// No run of that card was captured, or it has ended.
    pub const NotRunning: Self = Self {
        kind: LiveDeliveryKind::NotRunning,
        reason: None,
    };
    pub const Delivered: Self = Self {
        kind: LiveDeliveryKind::Delivered,
        reason: None,
    };
    pub const Applied: Self = Self {
        kind: LiveDeliveryKind::Applied,
        reason: None,
    };
    pub const PendingBoundary: Self = Self {
        kind: LiveDeliveryKind::PendingBoundary,
        reason: None,
    };
    pub const RestartRequired: Self = Self {
        kind: LiveDeliveryKind::RestartRequired,
        reason: None,
    };
    /// The run already had this bag.
    pub const Unchanged: Self = Self {
        kind: LiveDeliveryKind::Unchanged,
        reason: None,
    };
    /// The captured run was replaced (identity or generation changed)
    /// before the write became durable; it was not posted to.
    pub const Stale: Self = Self {
        kind: LiveDeliveryKind::Stale,
        reason: None,
    };

    /// The write was rejected with `reason`.
    pub fn rejected(reason: impl Into<String>) -> Self {
        Self {
            kind: LiveDeliveryKind::Rejected,
            reason: Some(reason.into()),
        }
    }

    pub(crate) fn kind(&self) -> LiveDeliveryKind {
        self.kind
    }

    /// Rejection text, when this delivery was refused.
    pub fn rejection(&self) -> Option<&str> {
        self.reason.as_deref()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SettingsResultKind {
    Saved,
    Superseded,
    Failed,
}

#[derive(Debug, PartialEq, Eq)]
pub struct SettingsResult {
    kind: SettingsResultKind,
    delivery: Option<LiveDelivery>,
    error: Option<String>,
}

#[allow(non_upper_case_globals)]
impl SettingsResult {
    /// This edit never settled on its own: a newer edit of the same card's
    /// parameters in the same commit replaced it, or the commit failed and
    /// a newer write of the profile reports the failure.
    pub const Superseded: Self = Self {
        kind: SettingsResultKind::Superseded,
        delivery: None,
        error: None,
    };

    /// A durable save with its live delivery.
    pub fn saved(delivery: LiveDelivery) -> Self {
        Self {
            kind: SettingsResultKind::Saved,
            delivery: Some(delivery),
            error: None,
        }
    }

    /// A failed write.
    pub fn failed(error: impl Into<String>) -> Self {
        Self {
            kind: SettingsResultKind::Failed,
            delivery: None,
            error: Some(error.into()),
        }
    }

    /// Whether this result is a durable save.
    pub fn is_saved(&self) -> bool {
        self.kind == SettingsResultKind::Saved
    }

    /// Whether this edit was replaced by a later write.
    pub fn is_superseded(&self) -> bool {
        self.kind == SettingsResultKind::Superseded
    }

    /// Whether this result failed.
    pub fn is_failed(&self) -> bool {
        self.kind == SettingsResultKind::Failed
    }

    /// Live delivery of a saved write.
    pub fn delivery(&self) -> Option<&LiveDelivery> {
        self.delivery.as_ref()
    }

    /// Failure text, when this result failed.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
}

/// One settled script-parameter write.
#[derive(Debug, PartialEq, Eq)]
pub struct SettingsWrite {
    pub(crate) op: OperationId,
    pub(crate) profile: String,
    pub(crate) result: SettingsResult,
}

impl SettingsWrite {
    pub(crate) fn new(op: OperationId, profile: impl Into<String>, result: SettingsResult) -> Self {
        Self {
            op,
            profile: profile.into(),
            result,
        }
    }

    /// The write's operation.
    pub fn op(&self) -> OperationId {
        self.op
    }

    /// The profile written.
    pub fn profile(&self) -> &str {
        &self.profile
    }

    /// How the write ended.
    pub fn result(&self) -> &SettingsResult {
        &self.result
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NoticeKind {
    Show,
    Clear,
    Retract,
}

/// Operator-facing message from the last script operation.
#[derive(Debug, PartialEq, Eq)]
pub struct Notice {
    kind: NoticeKind,
    text: Option<String>,
    with: Option<String>,
}

#[allow(non_upper_case_globals)]
impl Notice {
    pub const Clear: Self = Self {
        kind: NoticeKind::Clear,
        text: None,
        with: None,
    };

    /// Show `text` on the banner.
    pub fn show(text: impl Into<String>) -> Self {
        Self {
            kind: NoticeKind::Show,
            text: Some(text.into()),
            with: None,
        }
    }

    /// Replace the banner with `with` only while it still shows `shown`:
    /// an asynchronous update (a Start settling) never erases a newer
    /// message the front end or another command put there.
    pub fn retract(shown: impl Into<String>, with: Option<String>) -> Self {
        Self {
            kind: NoticeKind::Retract,
            text: Some(shown.into()),
            with,
        }
    }

    /// The banner text this notice would show, if any.
    pub fn text(&self) -> Option<&str> {
        match self.kind {
            NoticeKind::Show => self.text.as_deref(),
            NoticeKind::Clear => None,
            NoticeKind::Retract => self.with.as_deref(),
        }
    }

    /// Apply this notice to a front end's banner (the one both the panel
    /// and the TUI use).
    pub fn apply(self, banner: &mut Option<String>) {
        match self.kind {
            NoticeKind::Show => *banner = self.text,
            NoticeKind::Clear => *banner = None,
            NoticeKind::Retract => {
                if banner.as_deref() == self.text.as_deref() {
                    *banner = self.with;
                }
            }
        }
    }
}

/// Which operator action a pending Start came from: it decides where a
/// setup failure is reported and whether the saved assignment may change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StartKind {
    Start,
    Reload,
    RestartSettings,
}

/// A Start rejected before or while admitting its script: the kind and the
/// sentence the front end shows.
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct ScriptStartError {
    kind: ScriptStartErrorKind,
    message: String,
}

/// The kind of a [`ScriptStartError`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptStartErrorKind {
    /// The Start was refused (no assignment, already active, missing card).
    Refused,
    /// The saved loadout selection is required, but no loadouts exist.
    NoLoadouts,
    /// The Start waits on another decision (uncommitted settings, a
    /// blocked sync).
    Waiting,
    /// The script runtime could not load the card.
    RuntimeLoad,
}

impl ScriptStartError {
    fn refused(message: impl Into<String>) -> Self {
        Self {
            kind: ScriptStartErrorKind::Refused,
            message: message.into(),
        }
    }

    /// The kind of rejection.
    pub fn kind(&self) -> ScriptStartErrorKind {
        self.kind
    }
}

impl From<script::StartLoadError> for ScriptStartError {
    fn from(error: script::StartLoadError) -> Self {
        let (kind, message) = match error {
            script::StartLoadError::Refused(reason) => (ScriptStartErrorKind::Refused, reason),
            script::StartLoadError::NoLoadouts(reason) => {
                (ScriptStartErrorKind::NoLoadouts, reason)
            }
            script::StartLoadError::Waiting(reason) => (ScriptStartErrorKind::Waiting, reason),
            script::StartLoadError::RuntimeLoad(reason) => {
                (ScriptStartErrorKind::RuntimeLoad, reason)
            }
        };
        Self { kind, message }
    }
}

/// A Start whose worker has not settled. Assignment changes only on Ready.
#[derive(Debug, Clone)]
enum PendingCard {
    Compiled(script::CompiledId),
    Loaded(Box<script::JsCard>),
}

#[derive(Debug, Clone)]
struct PendingStart {
    card: PendingCard,
    kind: StartKind,
    /// Counted as started in the running Start tally: a setup failure
    /// moves it to failed there.
    tallied: bool,
}

#[derive(Debug, Clone)]
struct SettingsCard {
    selection: script::ScriptSel,
    name: String,
}

/// Where a new skip or failure in the running tally is logged.
#[derive(Debug, Clone, Copy)]
enum LogTo {
    /// The bot's own log.
    Slot,
    /// The process log (a marked row with no profile).
    Process,
    /// Already logged (a settled Start's operation line) or not a skip.
    None,
}

/// One shared Browse card: a compiled registry card or a loaded JS card.
/// Loaded cards remain borrowed from the library, so a Browse render does not
/// clone source bytes or the whole card.
#[derive(Debug, Clone, Copy)]
pub struct BrowseCard<'a>(BrowseCardKind<'a>);

#[derive(Debug, Clone, Copy)]
enum BrowseCardKind<'a> {
    Compiled(&'static script::native::CompiledCard),
    Loaded(&'a script::JsCard),
}

impl<'a> BrowseCard<'a> {
    pub fn name(self) -> &'a str {
        match self.0 {
            BrowseCardKind::Compiled(card) => card.name,
            BrowseCardKind::Loaded(card) => card.name.as_str(),
        }
    }

    pub fn description(self) -> &'a str {
        match self.0 {
            BrowseCardKind::Compiled(card) => card.description,
            BrowseCardKind::Loaded(card) => card.description.as_str(),
        }
    }

    pub fn category(self) -> &'a str {
        match self.0 {
            BrowseCardKind::Compiled(card) => card.category,
            BrowseCardKind::Loaded(card) => card.category.as_str(),
        }
    }

    pub fn tags(self) -> Option<&'a [String]> {
        match self.0 {
            BrowseCardKind::Compiled(_) => None,
            BrowseCardKind::Loaded(card) => Some(&card.tags),
        }
    }

    /// How the card runs.
    pub fn kind(self) -> CardRuntime {
        match self.0 {
            BrowseCardKind::Compiled(_) => CardRuntime::Compiled,
            BrowseCardKind::Loaded(card) => CardRuntime::from_script(card.kind),
        }
    }

    /// Where the card came from.
    pub fn source(self) -> CardSource {
        match self.0 {
            BrowseCardKind::Compiled(_) => CardSource::Builtin,
            BrowseCardKind::Loaded(card) => CardSource::from_script(card.source),
        }
    }

    pub fn unloadable(self) -> Option<&'a str> {
        match self.0 {
            BrowseCardKind::Compiled(_) => None,
            BrowseCardKind::Loaded(card) => card.unloadable.as_deref(),
        }
    }

    /// The loaded JS card, if this is one.
    pub fn loaded(self) -> Option<LoadedCard<'a>> {
        match self.0 {
            BrowseCardKind::Compiled(_) => None,
            BrowseCardKind::Loaded(card) => Some(LoadedCard::new(card)),
        }
    }

    /// The compiled card's id, if this is one.
    pub fn compiled_id(self) -> Option<CompiledCardId> {
        match self.0 {
            BrowseCardKind::Compiled(card) => Some(CompiledCardId::from_script(card.id)),
            BrowseCardKind::Loaded(_) => None,
        }
    }

    /// The card as a picker choice.
    pub fn selection(self) -> ScriptChoice {
        ScriptChoice::from_script(match self.0 {
            BrowseCardKind::Compiled(card) => script::ScriptSel::Compiled(card.id),
            BrowseCardKind::Loaded(card) => {
                script::ScriptSel::Loaded(card.source, card.identity_id())
            }
        })
    }
}

/// A click on a script's paint: a button, or a choice in one of its
/// select rows, for the paint `generation` it was drawn from. A front end
/// builds a button's input with [`PaintView::activate`] and stages it for
/// `ScriptPaintInput`; the seam issues a host control's own Pause, Resume
/// or Stop row instead.
#[derive(Debug)]
pub struct PaintInput(pub(crate) PaintAction);

#[derive(Debug)]
pub(crate) enum PaintAction {
    Click {
        id: String,
        generation: u64,
    },
    Select {
        key: String,
        name: String,
        generation: u64,
    },
    /// A host control, before the seam checks its binding.
    Control(paint::ControlActivation),
    /// A host control the seam refused before dispatch, and why.
    Refused(&'static str),
}

impl PaintInput {
    /// A click on the paint button `id`.
    pub(crate) fn click(id: String, generation: u64) -> Self {
        Self(PaintAction::Click { id, generation })
    }

    /// The choice `name` in the paint select row `key`.
    pub fn select(key: String, name: String, generation: u64) -> Self {
        Self(PaintAction::Select {
            key,
            name,
            generation,
        })
    }
}

#[derive(Debug)]
pub(crate) struct StagedSetting {
    pub(crate) profile: Option<String>,
    pub(crate) selection: ScriptChoice,
    pub(crate) field: String,
    pub(crate) value: Value,
}

#[derive(Debug)]
pub(crate) struct StagedParameterValidation {
    pub(crate) key: ParameterEditKey,
    pub(crate) selection: ScriptChoice,
    pub(crate) field: String,
    pub(crate) baseline_text: String,
    pub(crate) baseline_value: Option<Value>,
    pub(crate) definition: script::SettingDef,
    pub(crate) options: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatalogActionKind {
    Fill,
    Import,
    Refresh,
}

#[derive(Debug)]
pub(crate) struct StagedCatalogAction {
    pub(crate) kind: CatalogActionKind,
    pub(crate) root: Option<PathBuf>,
    pub(crate) path_file: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SyncCommandInput {
    Prepare,
    Apply,
}

#[derive(Debug)]
pub(crate) struct StagedSyncCommand {
    pub(crate) command: SyncCommandInput,
    pub(crate) source: String,
    pub(crate) selection: ScriptChoice,
}

#[derive(Debug)]
struct StagedReloadTarget {
    source: CardSource,
    lookup: String,
}

#[derive(Debug)]
pub(crate) enum StagedParameterDiscard {
    Field(ParameterEditKey),
    Profile(String),
}

#[derive(Debug)]
struct PendingStartCommand {
    request: api::identity::RequestId,
    subject: Subject,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PendingApplicationWait {
    CatalogFill,
    CatalogImport,
    Reload,
    RestartSettings,
}

#[derive(Debug)]
struct PendingApplicationCommand {
    request: api::identity::RequestId,
    wait: PendingApplicationWait,
}

#[derive(Debug)]
struct PendingCatalogAssignmentCommand {
    request: api::identity::RequestId,
    subject: Subject,
    profile: String,
    selection: ScriptChoice,
    catalog_root: Option<PathBuf>,
    restart: bool,
}

/// One Assign or Assign & restart member: the row's request and subject,
/// the bot, its card and the catalogue root the card resolves against.
#[derive(Debug)]
pub(crate) struct PendingAssignmentWrite {
    pub(crate) request: api::identity::RequestId,
    pub(crate) subject: Subject,
    pub(crate) profile: String,
    pub(crate) selection: ScriptChoice,
    pub(crate) catalog_root: Option<PathBuf>,
    pub(crate) restart: bool,
}

#[derive(Debug)]
pub(crate) enum AssignmentCommandResult {
    Done,
    Operation(OperationId),
    Later,
    Skipped(crate::command::SkipReason),
    Rejected(String),
}

#[derive(Debug)]
struct TranspileEntry {
    source: CardSource,
    lookup: String,
    requests: Vec<api::identity::RequestId>,
}

pub struct Scripts {
    /// The Browse card both surfaces display.
    browse_selection: Option<ScriptChoice>,
    staged_browse_pick: Option<Option<ScriptChoice>>,
    staged_cancel_reason: Option<&'static str>,
    staged_file_path: Option<PathBuf>,
    staged_paint_input: Option<PaintInput>,
    staged_setting: Option<StagedSetting>,
    staged_parameter_validation: Option<StagedParameterValidation>,
    staged_catalog_action: Option<StagedCatalogAction>,
    staged_defer_catalog: Option<PathBuf>,
    staged_sync: Option<StagedSyncCommand>,
    staged_start_catalog_root: Option<PathBuf>,
    staged_reload_target: Option<StagedReloadTarget>,
    staged_transpile_target: Option<(CardSource, String)>,
    pending_catalog_assignment_commands: Vec<PendingCatalogAssignmentCommand>,
    pending_assignment_writes: HashMap<OperationId, Vec<PendingAssignmentWrite>>,
    restart_settings_failure: Option<String>,
    staged_parameter_discard: Option<StagedParameterDiscard>,
    transpile_queue: VecDeque<TranspileEntry>,
    transpile_armed: bool,
    transpile_done: usize,
    transpile_total: usize,
    transpile_all_request: Option<api::identity::RequestId>,
    transpile_all_failure: Option<String>,
    pending_start_commands: HashMap<String, Vec<PendingStartCommand>>,
    pending_application_commands: Vec<PendingApplicationCommand>,
    catalog_fill_update: Option<Result<(), String>>,
    catalog_import_update: Option<(PathBuf, Result<usize, String>)>,
    catalog_import_command_result: Option<Result<usize, String>>,
    start_command_failures: Vec<(String, String)>,
    /// Queued Starts that left the queue skipped: their commands settle
    /// on the next poll ([`Self::settle_start_command_failures`]).
    start_command_skips: Vec<(String, String)>,
    /// Assign & restart commands whose restart was withdrawn before it
    /// started, with the reason: they settle on the next poll.
    withdrawn_restart_commands: Vec<(PendingStartCommand, String)>,
    /// The card library: catalog, loaded files, transpile cache.
    js: script::JsLibrary,
    /// Legacy global per-card overrides (`script-settings.json`), claimed
    /// into a profile's bag on its first use of a card.
    legacy: script::ScriptSettingsStore,
    /// Scenario/live inject merged last on Start.
    inject: Option<Map<String, Value>>,
    /// Per-profile Browse selection, never treated as a successful Start.
    pending_browse: HashMap<String, ScriptChoice>,
    /// Reused editor keys for the currently displayed profile and card.
    parameter_key_cache: Vec<ParameterEditKey>,
    parameter_key_scope: Option<(Option<String>, script::ScriptSel)>,
    parameter_edits: HashMap<ParameterEditKey, ParameterEdit>,
    settings_cards: HashMap<OperationId, SettingsCard>,
    starts: HashMap<String, PendingStart>,
    /// Paced Start-all / marked-Start places. Empty in the idle path.
    admit: StartAdmit,
    /// Catalog root for grants that settle after the click frame.
    admit_catalog: Option<PathBuf>,
    /// The running Start all / marked-Start report (see [`start_tally`]).
    tally: Option<StartTally>,
    last_bulk_report: Option<String>,
    /// A planned fleet action ([`crate::Targets::Plan`]) is being
    /// dispatched: the tally or sync report it opens reports through the
    /// plan's one result line, not the banner.
    planned_run: bool,
    catalog_filled: bool,
    /// Root of the most recent catalog registry scan.
    catalog_root: Option<PathBuf>,
    /// Bounded WorkAdmission jobs for catalog scans, loaded Starts, and warmup.
    async_work: reload::ScriptWorkState,
    reload: reload::ReloadState,
    sync: sync::SyncState,
    notice: Option<Notice>,
    /// The text of the last notice shown (`None` once cleared): a settled
    /// Start only replaces the banner when it still shows the load-failure
    /// list it changes.
    shown: Option<String>,
    start_failures: Vec<(String, String)>,
    /// Test-only: fail the replacement Start for this profile after it
    /// passed eligibility and was stopped.
    #[cfg(any(test, feature = "test-support"))]
    fail_reload_start_for: Option<String>,
}

impl Scripts {
    pub fn new(js: script::JsLibrary, legacy: script::ScriptSettingsStore) -> Self {
        Self {
            browse_selection: None,
            staged_browse_pick: None,
            staged_cancel_reason: None,
            staged_file_path: None,
            staged_paint_input: None,
            staged_setting: None,
            staged_parameter_validation: None,
            staged_catalog_action: None,
            staged_defer_catalog: None,
            staged_sync: None,
            staged_start_catalog_root: None,
            staged_parameter_discard: None,
            transpile_queue: VecDeque::new(),
            staged_reload_target: None,
            staged_transpile_target: None,
            pending_catalog_assignment_commands: Vec::new(),
            pending_assignment_writes: HashMap::new(),
            restart_settings_failure: None,
            transpile_armed: false,
            transpile_done: 0,
            transpile_total: 0,
            transpile_all_request: None,
            transpile_all_failure: None,
            pending_start_commands: HashMap::new(),
            pending_application_commands: Vec::new(),
            catalog_import_update: None,
            catalog_import_command_result: None,
            catalog_fill_update: None,
            start_command_failures: Vec::new(),
            start_command_skips: Vec::new(),
            withdrawn_restart_commands: Vec::new(),
            js,
            legacy,
            inject: None,
            pending_browse: HashMap::new(),
            parameter_key_cache: Vec::new(),
            parameter_key_scope: None,
            parameter_edits: HashMap::new(),
            settings_cards: HashMap::new(),
            starts: HashMap::new(),
            admit: StartAdmit::default(),
            admit_catalog: None,
            tally: None,
            last_bulk_report: None,
            planned_run: false,
            catalog_filled: false,
            catalog_root: None,
            async_work: reload::ScriptWorkState::default(),
            reload: reload::ReloadState::default(),
            sync: sync::SyncState::default(),
            notice: None,
            shown: None,
            start_failures: Vec::new(),
            #[cfg(any(test, feature = "test-support"))]
            fail_reload_start_for: None,
        }
    }

    /// The shared Browse selection.
    pub fn browse_selection(&self) -> Option<&ScriptChoice> {
        self.browse_selection.as_ref()
    }

    /// Stage the Start-cancellation tally reason for the next command row.
    pub fn stage_cancel_queued(&mut self, reason: &'static str) {
        self.staged_cancel_reason = Some(reason);
    }

    pub(crate) fn take_staged_cancel_reason(&mut self) -> Option<&'static str> {
        self.staged_cancel_reason.take()
    }

    /// Stage a Browse card for the next `PickBrowseCard` command.
    pub fn stage_browse_pick(&mut self, selection: ScriptChoice) {
        self.staged_browse_pick = Some(Some(selection));
    }

    /// Stage clearing the shared Browse card for the next `PickBrowseCard` command.
    pub fn stage_browse_pick_clear(&mut self) {
        self.staged_browse_pick = Some(None);
    }

    pub(crate) fn take_staged_browse_pick(&mut self) -> Option<Option<ScriptChoice>> {
        self.staged_browse_pick.take()
    }

    pub(crate) fn pick_browse_card(&mut self, selection: Option<ScriptChoice>) {
        self.browse_selection = selection;
    }

    /// Stage a local script path for `LoadScriptFile`.
    pub fn stage_load_file(&mut self, path: PathBuf) {
        self.staged_file_path = Some(path);
    }

    pub(crate) fn take_staged_file_path(&mut self) -> Option<PathBuf> {
        self.staged_file_path.take()
    }

    /// Stage one script paint answer.
    pub fn stage_paint_input(&mut self, input: PaintInput) {
        self.staged_paint_input = Some(input);
    }

    pub(crate) fn take_staged_paint_input(&mut self) -> Option<PaintInput> {
        self.staged_paint_input.take()
    }

    /// The staged host-control activation, if the staged paint input is one.
    pub(crate) fn staged_paint_control(&self) -> Option<ControlActivation> {
        match self.staged_paint_input {
            Some(PaintInput(PaintAction::Control(activation))) => Some(activation),
            _ => None,
        }
    }

    /// Replace the staged host-control activation with its refusal, which
    /// the `ScriptPaintInput` row then reports.
    pub(crate) fn refuse_staged_paint_control(&mut self, reason: &'static str) {
        self.staged_paint_input = Some(PaintInput(PaintAction::Refused(reason)));
    }

    /// Stage one typed parameter value for a profile or legacy settings save.
    pub fn stage_setting(
        &mut self,
        profile: Option<&str>,
        selection: &ScriptChoice,
        field: &str,
        value: Value,
    ) {
        self.staged_setting = Some(StagedSetting {
            profile: profile.map(str::to_owned),
            selection: selection.duplicate(),
            field: field.to_owned(),
            value,
        });
    }

    pub(crate) fn take_staged_setting(&mut self) -> Option<StagedSetting> {
        self.staged_setting.take()
    }

    /// Stage the schema needed to validate the parameter editor's current
    /// text when `EditParameterText` is submitted.
    pub fn stage_parameter_validation(
        &mut self,
        key: ParameterEditKey,
        selection: &ScriptChoice,
        baseline_text: &str,
        baseline_value: Option<Value>,
        definition: SettingView<'_>,
        options: &[String],
    ) {
        self.staged_parameter_validation = Some(StagedParameterValidation {
            key,
            selection: selection.duplicate(),
            field: definition.id().to_owned(),
            baseline_text: baseline_text.to_owned(),
            baseline_value,
            definition: definition.def().clone(),
            options: options.to_vec(),
        });
    }

    pub(crate) fn take_staged_parameter_validation(&mut self) -> Option<StagedParameterValidation> {
        self.staged_parameter_validation.take()
    }

    /// Stage catalogue work for `ImportCatalog`.
    pub fn stage_catalog_action(
        &mut self,
        kind: CatalogActionKind,
        root: Option<PathBuf>,
        path_file: PathBuf,
    ) {
        self.staged_catalog_action = Some(StagedCatalogAction {
            kind,
            root,
            path_file,
        });
    }

    pub(crate) fn take_staged_catalog_action(&mut self) -> Option<StagedCatalogAction> {
        self.staged_catalog_action.take()
    }

    /// Stage the durable Not now marker path.
    pub fn stage_defer_catalog(&mut self, path: PathBuf) {
        self.staged_defer_catalog = Some(path);
    }

    pub(crate) fn take_staged_defer_catalog(&mut self) -> Option<PathBuf> {
        self.staged_defer_catalog.take()
    }

    pub(crate) fn stage_sync_command(
        &mut self,
        command: SyncCommandInput,
        source: &str,
        selection: &ScriptChoice,
    ) {
        self.staged_sync = Some(StagedSyncCommand {
            command,
            source: source.to_owned(),
            selection: selection.duplicate(),
        });
    }

    pub(crate) fn take_staged_sync_command(&mut self) -> Option<StagedSyncCommand> {
        self.staged_sync.take()
    }

    /// Stage the catalog root used by the next Start.
    pub fn stage_start_catalog_root(&mut self, root: Option<PathBuf>) {
        self.staged_start_catalog_root = root;
    }

    pub(crate) fn take_staged_start_catalog_root(&mut self) -> Option<PathBuf> {
        self.staged_start_catalog_root.take()
    }

    pub fn stage_reload_target(&mut self, target: Option<(CardSource, String)>) {
        self.staged_reload_target =
            target.map(|(source, lookup)| StagedReloadTarget { source, lookup });
    }

    pub(crate) fn take_staged_reload_target(&mut self) -> Option<(CardSource, String)> {
        self.staged_reload_target
            .take()
            .map(|target| (target.source, target.lookup))
    }

    pub fn stage_transpile_target(&mut self, source: CardSource, lookup: &str) {
        self.staged_transpile_target = Some((source, lookup.to_owned()));
    }

    pub(crate) fn take_staged_transpile_target(&mut self) -> Option<(CardSource, String)> {
        self.staged_transpile_target.take()
    }

    pub(crate) fn register_start_command(
        &mut self,
        profile: String,
        request: api::identity::RequestId,
        subject: Subject,
    ) {
        self.pending_start_commands
            .entry(profile)
            .or_default()
            .push(PendingStartCommand { request, subject });
    }

    /// Stage the focused parameter edit for `DiscardParameterText`.
    pub fn stage_parameter_discard(&mut self, key: ParameterEditKey) {
        self.staged_parameter_discard = Some(StagedParameterDiscard::Field(key));
    }

    /// Stage all drafts for one profile, including cards no longer displayed.
    /// Accepted writes still block Start until their terminal result.
    pub fn stage_profile_parameter_discard(&mut self, profile: &str) {
        self.staged_parameter_discard = Some(StagedParameterDiscard::Profile(profile.to_owned()));
    }

    pub(crate) fn take_staged_parameter_discard(&mut self) -> Option<StagedParameterDiscard> {
        self.staged_parameter_discard.take()
    }

    pub(crate) fn await_application_command(
        &mut self,
        request: api::identity::RequestId,
        wait: PendingApplicationWait,
    ) {
        self.pending_application_commands
            .push(PendingApplicationCommand { request, wait });
    }

    pub(crate) fn defer_assignment_command(
        &mut self,
        request: api::identity::RequestId,
        subject: Subject,
        profile: String,
        selection: ScriptChoice,
        catalog_root: Option<PathBuf>,
        restart: bool,
    ) {
        self.pending_catalog_assignment_commands
            .push(PendingCatalogAssignmentCommand {
                request,
                subject,
                profile,
                selection,
                catalog_root,
                restart,
            });
    }

    pub(crate) fn dispatch_assignment_command<Io>(
        &mut self,
        core: &mut OperatorSession<Io>,
        command: PendingAssignmentWrite,
    ) -> AssignmentCommandResult {
        let PendingAssignmentWrite {
            request,
            subject,
            profile,
            selection,
            catalog_root,
            restart,
        } = command;
        if restart {
            self.open_tally("Assign & restart");
            self.tally_record(&profile, Outcome::Queued, LogTo::None);
            self.publish_bulk();
        }
        if !restart && script_active(core, &profile) {
            return AssignmentCommandResult::Skipped(crate::command::SkipReason::ScriptActive);
        }
        let selected_script = selection.as_script().clone();
        let assignment = match self.assignment_for(&selected_script, catalog_root.as_deref()) {
            Ok(Some(assignment)) => assignment,
            Ok(None) => {
                self.defer_assignment_command(
                    request,
                    subject,
                    profile,
                    selection,
                    catalog_root,
                    restart,
                );
                return AssignmentCommandResult::Later;
            }
            Err(reason) => {
                if restart {
                    self.fail_assignment_restart(&profile, &reason);
                }
                return AssignmentCommandResult::Rejected(reason);
            }
        };
        self.clear_pending_browse(&profile);
        let log_action = if restart {
            "Assign & restart"
        } else {
            "Assign"
        };
        let write = if restart {
            self.queue_assignment_with_mirror(
                core,
                &profile,
                assignment,
                log_action,
                ArmMirror::script_assignment(),
            )
        } else {
            self.queue_assignment(core, &profile, assignment, log_action)
        };
        match write {
            Ok(Some(op)) if restart => {
                self.pending_assignment_writes.entry(op).or_default().push(
                    PendingAssignmentWrite {
                        request,
                        subject,
                        profile,
                        selection,
                        catalog_root,
                        restart: true,
                    },
                );
                AssignmentCommandResult::Later
            }
            Ok(Some(op)) => AssignmentCommandResult::Operation(op),
            Ok(None) if restart => {
                self.start_assigned_command(
                    core,
                    PendingAssignmentWrite {
                        request,
                        subject,
                        profile,
                        selection,
                        catalog_root,
                        restart: true,
                    },
                );
                AssignmentCommandResult::Later
            }
            Ok(None) => AssignmentCommandResult::Done,
            Err(reason) => {
                if restart {
                    self.fail_assignment_restart(&profile, &reason);
                }
                AssignmentCommandResult::Rejected(reason)
            }
        }
    }

    pub(crate) fn queue_transpile_command(
        &mut self,
        request: api::identity::RequestId,
        source: CardSource,
        lookup: &str,
    ) -> Result<bool, String> {
        if self.js.get(source.to_script(), lookup).is_none() {
            return Err(format!("unavailable: {lookup}"));
        }
        if self.card_is_prepared(source, lookup) {
            return Ok(false);
        }
        self.enqueue_transpile(source, lookup.to_owned(), true, Some(request));
        Ok(true)
    }

    pub(crate) fn queue_transpile_all_command(
        &mut self,
        request: api::identity::RequestId,
    ) -> Result<bool, String> {
        if self.transpile_all_request.is_some() {
            return Err("transpile all is already pending".into());
        }
        let need = self.cards_needing_transpile();
        if need.is_empty() {
            return Ok(false);
        }
        self.transpile_all_request = Some(request);
        self.transpile_all_failure = None;
        if self.transpile_queue.is_empty() {
            self.transpile_done = 0;
        }
        for (source, name) in need {
            self.enqueue_transpile(source, name, false, None);
        }
        self.transpile_total = self.transpile_queue.len();
        self.transpile_armed = true;
        Ok(true)
    }

    pub fn transpile_front(&self) -> Option<(CardSource, &str)> {
        self.transpile_queue
            .front()
            .map(|entry| (entry.source, entry.lookup.as_str()))
    }

    pub fn transpile_progress(&self) -> (usize, usize) {
        (self.transpile_done, self.transpile_total)
    }

    fn enqueue_transpile(
        &mut self,
        source: CardSource,
        lookup: String,
        to_front: bool,
        request: Option<api::identity::RequestId>,
    ) {
        if self.transpile_queue.is_empty() {
            self.transpile_done = 0;
            self.transpile_total = 0;
        }
        if let Some(index) = self
            .transpile_queue
            .iter()
            .position(|entry| entry.source == source && entry.lookup == lookup)
        {
            if let Some(request) = request {
                self.transpile_queue[index].requests.push(request);
            }
            if to_front && index != 0 {
                if let Some(entry) = self.transpile_queue.remove(index) {
                    self.transpile_queue.push_front(entry);
                }
            }
        } else {
            let mut requests = Vec::new();
            if let Some(request) = request {
                requests.push(request);
            }
            let entry = TranspileEntry {
                source,
                lookup,
                requests,
            };
            if to_front {
                self.transpile_queue.push_front(entry);
            } else {
                self.transpile_queue.push_back(entry);
            }
        }
        self.transpile_total = self.transpile_queue.len();
        self.transpile_armed = true;
    }

    pub fn take_catalog_import_update(&mut self) -> Option<(PathBuf, Result<usize, String>)> {
        self.catalog_import_update.take()
    }

    /// The operator's card library and legacy per-card store at their
    /// default paths. A missing library store is a first run; a refused or
    /// corrupt one is returned as text for the front end to say out loud
    /// (nothing is saved over a refused store).
    pub fn open_default() -> (Self, Option<String>) {
        let mut js = script::JsLibrary::new(script::default_js_store());
        let restore_error = js.restore().err();
        (
            Self::new(js, script::ScriptSettingsStore::with_default_path()),
            restore_error,
        )
    }

    /// The card library, inside this crate.
    pub(crate) fn library(&self) -> &script::JsLibrary {
        &self.js
    }

    /// The card library.
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn js(&self) -> &script::JsLibrary {
        &self.js
    }

    /// The card library, for load/transpile.
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn js_mut(&mut self) -> &mut script::JsLibrary {
        &mut self.js
    }

    /// Replace the card library (tests).
    #[cfg(any(test, feature = "test-support"))]
    pub fn set_js(&mut self, js: script::JsLibrary) {
        self.js = js;
    }

    /// Use a fresh card library stored at `store` with its transpile cache
    /// under `cache_root` (tests).
    #[cfg(any(test, feature = "test-support"))]
    pub fn use_library_at(&mut self, store: PathBuf, cache_root: PathBuf) {
        self.js = script::JsLibrary::with_cache(store, cache_root);
    }

    /// Replace the legacy global per-card store (tests).
    #[cfg(any(test, feature = "test-support"))]
    pub fn set_legacy(&mut self, legacy: script::ScriptSettingsStore) {
        self.legacy = legacy;
    }

    /// The loaded card `name` (a File card's lookup is its path).
    pub fn card(&self, source: CardSource, name: &str) -> Option<LoadedCard<'_>> {
        self.js.get(source.to_script(), name).map(LoadedCard::new)
    }

    /// Every loaded card, in library order.
    pub fn cards(&self) -> impl ExactSizeIterator<Item = LoadedCard<'_>> + '_ {
        self.js.cards().iter().map(LoadedCard::new)
    }

    /// Whether the library holds any card from `source`.
    pub fn has_cards_from(&self, source: CardSource) -> bool {
        let source = source.to_script();
        self.js.cards().iter().any(|card| card.source == source)
    }

    /// The recorded load failure for a card identity key.
    pub fn load_failure(&self, identity_key: &str) -> Option<LoadFailureView<'_>> {
        self.js.load_failure(identity_key).map(LoadFailureView::new)
    }

    /// Every recorded load failure.
    pub fn load_failures(&self) -> impl Iterator<Item = LoadFailureView<'_>> + '_ {
        self.js
            .load_failures()
            .into_iter()
            .map(LoadFailureView::new)
    }

    /// How many load failures are recorded.
    pub fn load_failure_count(&self) -> usize {
        self.js.load_failures().len()
    }

    /// The recorded load failures as named, copyable lines.
    pub fn named_failure_output(&self) -> String {
        self.js.named_failure_output()
    }

    /// Loaded cards whose JavaScript is not yet prepared, by lookup.
    pub fn cards_needing_transpile(&self) -> Vec<(CardSource, String)> {
        self.js
            .cards_needing_transpile()
            .into_iter()
            .map(|(source, name)| (CardSource::from_script(source), name))
            .collect()
    }

    /// Whether a loaded card's JavaScript is prepared.
    pub fn card_is_prepared(&self, source: CardSource, name: &str) -> bool {
        self.js.js_is_ready(source.to_script(), name)
    }

    /// Load a local JS/TS file into the library (registering a picker card
    /// and persisting the library store). Returns the loaded card.
    pub(crate) fn load_file(&mut self, path: &Path) -> Result<LoadedCard<'_>, String> {
        let card = self.js.load(path)?;
        let lookup = card.identity_id();
        self.js
            .get(card.source, &lookup)
            .map(LoadedCard::new)
            .ok_or_else(|| format!("loaded card {lookup} is missing from the library"))
    }

    /// Load one exact in-tree example and return its canonical File-card identity.
    pub fn load_live_example_identity(&mut self, file_name: &str) -> Result<String, String> {
        let path = script::live_example_path(file_name)
            .ok_or_else(|| format!("no in-tree example {file_name}"))?;
        let loaded = self
            .js
            .load(&path)
            .map_err(|error| format!("load {file_name}: {error}"))?;
        let identity = loaded.identity_id();
        self.js
            .get(script::ScriptSource::File, &identity)
            .map(|_| identity.clone())
            .ok_or_else(|| format!("file example {file_name} missing after identity load"))
    }

    /// Same-folder `./Foo.js` sibling modules a loaded card imports, read
    /// for an isolate Start.
    pub fn card_siblings(
        &self,
        source: CardSource,
        lookup: &str,
    ) -> Result<Vec<(String, String)>, String> {
        let card = self
            .js
            .get(source.to_script(), lookup)
            .ok_or_else(|| format!("unavailable: {lookup}"))?;
        host_play::live_start::card_siblings(&self.js, card)
    }

    /// Write one legacy global per-card override and save the store.
    pub(crate) fn set_legacy_value<Io>(
        &mut self,
        core: &mut OperatorSession<Io>,
        source: CardSource,
        name: &str,
        id: &str,
        old_value: Option<Value>,
        value: Value,
    ) -> Result<(), String> {
        let source = source.to_script();
        let card = self.legacy.card_key(source, name);
        self.legacy.set_value(source, name, id, value.clone());
        self.legacy.save()?;
        core.log_legacy_script_settings(&card, "parameter edit", id, old_value.as_ref(), &value);
        Ok(())
    }

    /// Disjoint library, legacy store and inject bag for live Start.
    #[cfg(feature = "live-harness")]
    pub(crate) fn live_start_parts(
        &mut self,
    ) -> (
        &mut script::JsLibrary,
        &script::ScriptSettingsStore,
        Option<&Map<String, Value>>,
    ) {
        (&mut self.js, &self.legacy, self.inject.as_ref())
    }

    /// Legacy global per-card overrides.
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn legacy(&self) -> &script::ScriptSettingsStore {
        &self.legacy
    }

    /// Legacy global per-card overrides, for writes (tests).
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn legacy_mut(&mut self) -> &mut script::ScriptSettingsStore {
        &mut self.legacy
    }

    /// Set the scenario/live inject bag.
    pub fn set_inject(&mut self, inject: Option<Map<String, Value>>) {
        self.inject = inject;
    }

    /// Fail the replacement Start for this profile (tests).
    #[cfg(any(test, feature = "test-support"))]
    pub fn set_fail_reload_start_for(&mut self, profile: Option<String>) {
        self.fail_reload_start_for = profile;
    }

    fn track_settings_card(&mut self, op: OperationId, selection: script::ScriptSel, name: &str) {
        self.settings_cards.insert(
            op,
            SettingsCard {
                selection,
                name: name.to_string(),
            },
        );
    }

    /// The message to show since the last take.
    pub fn take_notice(&mut self) -> Option<Notice> {
        self.notice.take()
    }

    pub fn pending_restart_prompt(&self) -> Option<&RestartPrompt> {
        self.sync.pending_restart_prompt()
    }

    pub(crate) fn dismiss_restart_prompt(&mut self) {
        self.sync.dismiss_restart_prompt();
    }

    pub fn restart_required_for(&self, profile: &str, selection: &ScriptChoice) -> bool {
        let selection = selection.as_script();
        self.sync
            .restart_card(profile)
            .is_some_and(|required| required == selection)
    }

    pub fn has_restart_badge(&self, profile: &str) -> bool {
        self.sync.has_restart_badge(profile)
    }

    pub fn restart_generation(&self) -> u64 {
        self.sync.restart_generation()
    }

    pub(crate) fn restart_pending_settings<Io>(
        &mut self,
        core: &mut OperatorSession<Io>,
        catalog_root: Option<&Path>,
    ) -> Result<(), String> {
        self.restart_pending_settings_with_after_check(core, catalog_root, |_, _| {})
    }

    /// Keep the check-to-stop boundary deterministic in tests.
    fn restart_pending_settings_with_after_check<Io>(
        &mut self,
        core: &mut OperatorSession<Io>,
        catalog_root: Option<&Path>,
        mut after_check: impl FnMut(&mut OperatorSession<Io>, &sync::RestartTarget),
    ) -> Result<(), String> {
        let prompt = self
            .sync
            .take_restart_prompt()
            .ok_or_else(|| "no restart prompt".to_string())?;
        let mut skipped = Vec::new();
        for target in prompt.targets() {
            let (identity, generation) = match self.restart_target_skip(core, target) {
                Ok(run) => run,
                Err(reason) => {
                    self.sync.clear_restart_badge(&target.profile);
                    skipped.push((target.profile.clone(), reason.to_string()));
                    continue;
                }
            };
            if self.admit.contains(&target.profile) {
                skipped.push((target.profile.clone(), "start already queued".into()));
                continue;
            }
            after_check(core, target);
            let stopped = core.play().is_some_and(|play| {
                play.script_stop_if_identity_generation(&target.profile, &identity, generation)
            });
            if !stopped {
                self.sync.clear_restart_badge(&target.profile);
                skipped.push((target.profile.clone(), "not running".into()));
                continue;
            }
            match self.queue_pending_restart_settings(
                core,
                &target.profile,
                target.selection.clone(),
            ) {
                Ok(()) => {}
                Err(reason) => skipped.push((target.profile.clone(), reason)),
            }
        }
        self.admit_starts(core, catalog_root);
        if !skipped.is_empty() {
            self.show(
                skipped
                    .iter()
                    .map(|(profile, reason)| format!("Skipped {profile}: {reason}"))
                    .collect::<Vec<_>>()
                    .join("; "),
            );
        }
        Ok(())
    }

    fn restart_target_skip<Io>(
        &self,
        core: &OperatorSession<Io>,
        target: &sync::RestartTarget,
    ) -> Result<(String, u64), &'static str> {
        let profile = &target.profile;
        if !wall_member(core, profile) {
            return Err("not loaded");
        }
        let Some(play) = core.play() else {
            return Err("not running");
        };
        let Some(generation) = play.script_runtime_generation(profile) else {
            return Err("not running");
        };
        if !matches!(
            play.script_state(profile),
            script::RunState::Running | script::RunState::Paused
        ) {
            return Err("not running");
        }
        let Some(run_identity) = play.script_source_identity(profile) else {
            return Err("different script");
        };
        let (has_arm, logged_out) = arm_flags(core, profile);
        if !has_arm || logged_out {
            return Err("logged out");
        }
        let Some(assignment) = Self::assignment(core, profile) else {
            return Err("reassigned");
        };
        if sel_from_assignment(&assignment).as_ref() != Some(&target.selection) {
            return Err("reassigned");
        }
        let identity = vault::assignment_key(&assignment.source_kind, &assignment.identity);
        if run_identity != identity {
            return Err("different script");
        }
        let status = play.script_native_status(profile);
        if let Some(status) = status.as_deref() {
            if play.script_native_run(profile) != Some(status.run)
                || !matches!(target.selection, script::ScriptSel::Compiled(id) if id == status.card)
            {
                return Err("different script");
            }
            if status
                .pending_settings
                .is_none_or(|pending| pending <= status.active_settings)
            {
                return Err("settings no longer pending");
            }
        }
        if !self.sync.restart_target_is_current(
            profile,
            &target.selection,
            target.generation,
            status.as_deref().map(|status| status.active_settings),
            status.as_deref().and_then(|status| status.pending_settings),
        ) {
            return Err("settings no longer pending");
        }
        Ok((run_identity, generation))
    }

    pub(crate) fn show(&mut self, text: impl Into<String>) {
        let text = text.into();
        self.shown = Some(text.clone());
        self.notice = Some(Notice::show(text));
    }

    fn clear_notice(&mut self) {
        self.shown = None;
        self.notice = Some(Notice::Clear);
    }

    /// Show the library's load-failure list (or clear the banner when there
    /// is none): what an operator Start reports once it is accepted.
    pub fn show_load_failures(&mut self) {
        if self.js.load_failures().is_empty() {
            self.clear_notice();
        } else {
            self.show(self.js.named_failure_output());
        }
    }

    /// Operator Start setup failures (profile, `script: …` text) since the
    /// last take, for callers that attribute them further.
    pub fn take_start_failures(&mut self) -> Vec<(String, String)> {
        std::mem::take(&mut self.start_failures)
    }

    // ---- catalog -------------------------------------------------------

    /// Whether the `$RS2B0T` catalog has been registered (or its first-run
    /// prompt handled) this session.
    pub fn catalog_filled(&self) -> bool {
        self.catalog_filled
    }

    #[cfg(test)]
    /// Queue the catalog's first source-only scan once per root. The shared
    /// WorkAdmission worker performs all registry and source-file reads.
    pub(crate) fn fill_catalog_once(&mut self, root: Option<&Path>) {
        let Some(root) = root else {
            return;
        };
        if self.catalog_filled && self.catalog_root.as_deref() == Some(root) {
            return;
        }
        if let Err(error) = self.request_catalog_fill(root, &script::default_rs2b0t_path_file()) {
            self.show(format!("$RS2B0T registry: {error}"));
        }
    }

    /// The shared Browse card sequence: compiled registry cards first, then
    /// loaded JS/catalog cards in library order.
    pub fn browse_cards(&self) -> impl Iterator<Item = BrowseCard<'_>> {
        script::compiled_cards()
            .iter()
            .map(|card| BrowseCard(BrowseCardKind::Compiled(card)))
            .chain(
                self.js
                    .cards()
                    .iter()
                    .map(|card| BrowseCard(BrowseCardKind::Loaded(card))),
            )
    }

    // ---- assignment and Browse selection -------------------------------

    /// `profile`'s last successful assignment (as saved, staged edits
    /// included).
    pub(crate) fn assignment<Io>(
        core: &OperatorSession<Io>,
        profile: &str,
    ) -> Option<ScriptAssignment> {
        core.vault()
            .and_then(|v| v.get(profile))
            .and_then(|p| p.settings.script_assignment.clone())
    }

    /// The display name of `profile`'s last successful assignment.
    pub fn assignment_name<'a, Io>(
        core: &'a OperatorSession<Io>,
        profile: &str,
    ) -> Option<&'a str> {
        core.vault()
            .and_then(|v| v.get(profile))
            .and_then(|p| p.settings.script_assignment.as_ref())
            .map(|a| a.display_name.as_str())
    }

    /// `profile`'s last successful assignment (tests).
    #[cfg(any(test, feature = "test-support"))]
    pub fn profile_assignment<Io>(
        core: &OperatorSession<Io>,
        profile: &str,
    ) -> Option<ScriptAssignment> {
        Self::assignment(core, profile)
    }

    /// The card a profile's script heading names: its pending Browse
    /// selection, else its last successful assignment.
    pub fn heading<Io>(&self, core: &OperatorSession<Io>, profile: &str) -> Option<ScriptChoice> {
        if let Some(sel) = self.pending_browse.get(profile) {
            return Some(sel.duplicate());
        }
        Self::assignment(core, profile)
            .and_then(|a| sel_from_assignment(&a))
            .map(ScriptChoice::from_script)
    }

    /// The label of [`Self::heading`], borrowed (no copy): the pending
    /// Browse selection's label, else the assignment's display name.
    pub fn heading_label<'a, Io>(
        &'a self,
        core: &'a OperatorSession<Io>,
        profile: &str,
    ) -> Option<&'a str> {
        if let Some(sel) = self.pending_browse.get(profile) {
            return Some(sel.label());
        }
        Self::assignment_name(core, profile)
    }

    /// Whether `profile`'s Browse selection differs from its assignment.
    pub fn heading_is_pending<Io>(&self, core: &OperatorSession<Io>, profile: &str) -> bool {
        let Some(pending) = self.pending_browse.get(profile) else {
            return false;
        };
        match Self::assignment(core, profile) {
            None => true,
            Some(asg) => sel_from_assignment(&asg).as_ref() != Some(pending.as_script()),
        }
    }

    pub(crate) fn set_pending_browse(&mut self, profile: &str, sel: ScriptChoice) {
        self.pending_browse.insert(profile.to_string(), sel);
    }

    pub fn pending_browse(&self, profile: &str) -> Option<&ScriptChoice> {
        self.pending_browse.get(profile)
    }

    /// Stage `profile`'s settings and queue their durable write.
    fn upsert_profile_settings<Io>(
        &mut self,
        core: &mut OperatorSession<Io>,
        profile: &str,
        mirror: ArmMirror,
        log_action: &'static str,
        edit: impl FnOnce(&mut vault::ProfileSettings),
    ) -> Result<OperationId, String> {
        let setting_identity = mirror.script_settings_identity();
        let result = core.vault().map(|vault| {
            if mirror.is_native_settings() {
                core.profile_for_edit(profile).cloned()
            } else {
                vault.get(profile).cloned()
            }
        });
        let result = match result {
            None => Err("script: vault locked".to_string()),
            Some(None) => Err(format!("script: no profile {profile}")),
            Some(Some(mut row)) => {
                let previous = setting_identity
                    .as_deref()
                    .map(|identity| script_settings_values(&row.settings, identity));
                edit(&mut row.settings);
                let details = setting_identity.as_deref().zip(previous.as_ref()).map(
                    |(identity, previous)| {
                        let current = script_settings_values(&row.settings, identity);
                        self.script_settings_log_details(identity, log_action, previous, &current)
                    },
                );
                let result = core.save_profile_with_log_action(row, mirror, "script", log_action);
                if let (Ok(op), Some(details)) = (result.as_ref(), details) {
                    core.track_script_settings_log(*op, details);
                }
                result
            }
        };
        if let Err(error) = &result {
            self.show(error.clone());
        }
        result
    }

    fn script_settings_log_details(
        &self,
        identity: &str,
        action: &str,
        old: &Map<String, Value>,
        new: &Map<String, Value>,
    ) -> String {
        if let Some(name) = identity.strip_prefix("compiled:") {
            if let Some(card) = script::compiled_id(name).and_then(script::compiled_card) {
                let schema = (card.schema)();
                return format_script_settings_log(card.name, action, &schema, old, new);
            }
        } else if let Some((source, lookup)) = identity.split_once(':') {
            if let Some(card) =
                script::parse_source_kind(source).and_then(|source| self.js.get(source, lookup))
            {
                return format_script_settings_log(
                    &card.name,
                    action,
                    &card.settings_schema,
                    old,
                    new,
                );
            }
        }
        format_script_settings_log(identity, action, &[], old, new)
    }

    /// Save `profile`'s last successful assignment. Re-starting the same
    /// card writes nothing.
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn persist_assignment<Io>(
        &mut self,
        core: &mut OperatorSession<Io>,
        profile: &str,
        assignment: ScriptAssignment,
    ) -> bool {
        self.persist_assignment_with_action(core, profile, assignment, "assignment")
    }

    pub(crate) fn queue_assignment<Io>(
        &mut self,
        core: &mut OperatorSession<Io>,
        profile: &str,
        assignment: ScriptAssignment,
        log_action: &'static str,
    ) -> Result<Option<OperationId>, String> {
        self.queue_assignment_with_mirror(core, profile, assignment, log_action, ArmMirror::none())
    }

    fn queue_assignment_with_mirror<Io>(
        &mut self,
        core: &mut OperatorSession<Io>,
        profile: &str,
        assignment: ScriptAssignment,
        log_action: &'static str,
        mirror: ArmMirror,
    ) -> Result<Option<OperationId>, String> {
        if Self::assignment(core, profile).as_ref() == Some(&assignment) {
            return Ok(None);
        }
        self.upsert_profile_settings(core, profile, mirror, log_action, |settings| {
            settings.script_assignment = Some(assignment);
        })
        .map(Some)
    }

    pub(crate) fn persist_assignment_with_action<Io>(
        &mut self,
        core: &mut OperatorSession<Io>,
        profile: &str,
        assignment: ScriptAssignment,
        log_action: &'static str,
    ) -> bool {
        self.queue_assignment(core, profile, assignment, log_action)
            .is_ok()
    }

    pub(crate) fn mark_assignment_unavailable<Io>(
        &mut self,
        core: &mut OperatorSession<Io>,
        profile: &str,
        reason: impl Into<String>,
    ) {
        let reason = reason.into();
        let _ = self.upsert_profile_settings(
            core,
            profile,
            ArmMirror::none(),
            "unavailable",
            |settings| {
                if let Some(asg) = settings.script_assignment.as_mut() {
                    asg.unavailable = Some(reason);
                }
            },
        );
    }

    // ---- per-profile parameters ----------------------------------------

    /// Copy the legacy global overrides for `card_name` into `profile`'s
    /// bag on its first use of the card.
    fn claim_legacy_for<Io>(
        &mut self,
        core: &mut OperatorSession<Io>,
        profile: &str,
        key: &str,
        card_name: &str,
    ) {
        let already = core
            .vault()
            .and_then(|v| v.get(profile))
            .is_some_and(|p| p.settings.script_settings.contains_key(key));
        if already {
            return;
        }
        let legacy = self.legacy_overrides_for(key, card_name);
        let _ = self.upsert_profile_settings(
            core,
            profile,
            ArmMirror::none(),
            "legacy claim",
            |settings| {
                script::claim_legacy_overrides(settings, key, card_name, &legacy);
            },
        );
    }

    /// `profile`'s overrides bag for the card `key`, claiming the legacy
    /// global overrides into the profile on its first use of the card. That
    /// claim queues a durable write; a read that must not write (a
    /// confirmation still awaiting the operator) uses
    /// [`Self::peek_profile_overrides`].
    pub fn profile_overrides<Io>(
        &mut self,
        core: &mut OperatorSession<Io>,
        profile: &str,
        key: &str,
        card_name: &str,
    ) -> Map<String, Value> {
        self.claim_legacy_for(core, profile, key, card_name);
        self.peek_profile_overrides(core, profile, key, card_name)
    }

    /// The bag [`Self::profile_overrides`] returns, without the claim: the
    /// profile's own overrides for the card, or, before its first use of the
    /// card, the migrated legacy overrides the claim would store. Writes
    /// nothing and queues nothing.
    pub fn peek_profile_overrides<Io>(
        &self,
        core: &OperatorSession<Io>,
        profile: &str,
        key: &str,
        card_name: &str,
    ) -> Map<String, Value> {
        let Some(row) = core.vault().and_then(|v| v.get(profile)) else {
            return Map::new();
        };
        match row.settings.script_settings.get(key) {
            Some(own) => own.clone(),
            None => {
                script::migrate_overrides(card_name, &self.legacy_overrides_for(key, card_name))
            }
        }
    }

    /// The legacy global overrides for the card `key` names.
    fn legacy_overrides_for(&self, key: &str, card_name: &str) -> Map<String, Value> {
        self.legacy.overrides(
            script::parse_source_kind(key.split(':').next().unwrap_or("catalog"))
                .unwrap_or(script::ScriptSource::Catalog),
            card_name,
        )
    }

    /// What a Start of this card on `profile` receives: schema defaults,
    /// the profile's overrides, then the inject.
    pub fn merged_profile_bag<Io>(
        &mut self,
        core: &mut OperatorSession<Io>,
        profile: &str,
        source: CardSource,
        name: &str,
        path: &Path,
        schema: SchemaRef<'_>,
    ) -> Map<String, Value> {
        let schema = schema.defs();
        let source = source.to_script();
        let key = script::card_identity_key(source, path, name);
        let overrides = self.profile_overrides(core, profile, &key, name);
        self.run_bag(core, profile, schema, &overrides)
    }

    /// The bag a run on `profile` receives, on every path (Start, a
    /// parameter edit, Apply to all): the card's merged parameters, then
    /// the profile-global settings unless the inject names its own. The
    /// isolate replaces both bags on each post, so a card-only bag would
    /// wipe the account's clue duel partner from a running script.
    fn run_bag<Io>(
        &self,
        core: &OperatorSession<Io>,
        profile: &str,
        schema: &[script::SettingDef],
        overrides: &Map<String, Value>,
    ) -> Map<String, Value> {
        let partner = core
            .settings_for_edit(profile)
            .map_or("", |settings| settings.clue_duel_partner.as_str());
        self.run_bag_with(schema, overrides, partner)
    }

    /// [`Self::run_bag`] with the profile's global partner given.
    fn run_bag_with(
        &self,
        schema: &[script::SettingDef],
        overrides: &Map<String, Value>,
        partner: &str,
    ) -> Map<String, Value> {
        let mut bag = script::merge_bag(schema, overrides, self.inject.as_ref());
        let injected = self
            .inject
            .as_ref()
            .is_some_and(|inject| inject.contains_key(script::CLUE_DUEL_PARTNER));
        let partner = partner.trim();
        if !injected && !partner.is_empty() {
            bag.insert(
                script::CLUE_DUEL_PARTNER.into(),
                Value::String(partner.into()),
            );
        }
        bag
    }

    /// The bag the running assignment of `profile` holds once `settings`
    /// are saved, for a profile save that changes a global setting: frozen
    /// scripts read the clue duel partner live, at each crossing. `None`
    /// when no run of the profile's assigned card is live.
    pub fn profile_save_live<Io>(
        &self,
        core: &OperatorSession<Io>,
        profile: &str,
        settings: &vault::ProfileSettings,
    ) -> Option<LiveSettings> {
        let assignment = settings.script_assignment.as_ref()?;
        let key = assignment.key();
        let (identity, generation) = live_fence(core, profile, &key)?;
        let (schema, overrides) = match sel_from_assignment(assignment)? {
            script::ScriptSel::Loaded(source, lookup) => (
                self.js
                    .get(source, &lookup)
                    .map(|card| card.settings_schema.as_slice())
                    .unwrap_or_default(),
                settings
                    .script_settings
                    .get(&key)
                    .cloned()
                    .unwrap_or_default(),
            ),
            script::ScriptSel::Compiled(id) => {
                let card = script::compiled_card(id)?;
                let values = match settings.script_settings.get(&key) {
                    Some(entry) => {
                        let (version, values) = vault::CompiledSettingsRecord::view(entry).ok()?;
                        if version != card.schema_version {
                            return None;
                        }
                        values.clone()
                    }
                    None => Map::new(),
                };
                ((card.schema)(), values)
            }
        };
        Some(LiveSettings {
            identity,
            generation,
            run: core.play().and_then(|play| play.script_native_run(profile)),
            bag: Arc::new(self.run_bag_with(schema, &overrides, &settings.clue_duel_partner)),
            prepared: None,
        })
    }

    /// The legacy global bag for a card (no profile focused).
    pub fn legacy_bag(
        &self,
        source: CardSource,
        name: &str,
        schema: SchemaRef<'_>,
    ) -> Map<String, Value> {
        let schema = schema.defs();
        let source = source.to_script();
        self.legacy
            .merged_bag(source, name, schema, self.inject.as_ref())
    }
    /// Read one setting from its durable profile or legacy bag, with schema
    /// defaults applied but no staged writes or scenario injection.
    pub(crate) fn durable_setting_value<Io>(
        &self,
        core: &OperatorSession<Io>,
        profile: Option<&str>,
        selection: &ScriptChoice,
        field: &str,
    ) -> Option<Value> {
        match selection.as_script() {
            script::ScriptSel::Loaded(source, lookup) => {
                let card = self.js.get(*source, lookup)?;
                let overrides = match profile {
                    Some(profile) => {
                        let identity = script::card_identity_key(*source, &card.path, &card.name);
                        let row = core.durable_profile(profile)?;
                        row.settings
                            .script_settings
                            .get(&identity)
                            .cloned()
                            .unwrap_or_else(|| {
                                script::migrate_overrides(
                                    &card.name,
                                    &self.legacy_overrides_for(&identity, &card.name),
                                )
                            })
                    }
                    None => self.legacy.overrides(*source, &card.name),
                };
                script::merge_bag(&card.settings_schema, &overrides, None)
                    .get(field)
                    .cloned()
            }
            script::ScriptSel::Compiled(id) => {
                let profile = profile?;
                let card = script::compiled_card(*id)?;
                let row = core.durable_profile(profile)?;
                let identity = script::compiled_identity_key(*id);
                let overrides = match row.settings.script_settings.get(&identity) {
                    Some(entry) => {
                        let (version, values) = vault::CompiledSettingsRecord::view(entry).ok()?;
                        if version != card.schema_version {
                            return None;
                        }
                        values.clone()
                    }
                    None => Map::new(),
                };
                script::merge_bag(&(card.schema)(), &overrides, None)
                    .get(field)
                    .cloned()
            }
        }
    }

    /// Set one typed parameter on `profile`'s bag for a card. The write is
    /// queued at once; the matching run (same card identity, generation
    /// captured now) receives the merged bag only once it is durable, and
    /// the result arrives as a [`SettingsWrite`]. A prepared bulk sync from
    /// this profile is dropped: its snapshot no longer matches.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn set_profile_setting<Io>(
        &mut self,
        core: &mut OperatorSession<Io>,
        profile: &str,
        source: CardSource,
        name: &str,
        path: &Path,
        id: &str,
        value: Value,
    ) -> Result<OperationId, String> {
        let source = source.to_script();
        let key = script::card_identity_key(source, path, name);
        self.claim_legacy_for(core, profile, &key, name);
        let mut bag = core
            .vault()
            .and_then(|v| v.get(profile))
            .and_then(|p| p.settings.script_settings.get(&key).cloned())
            .unwrap_or_default();
        let migrated = script::migrate_legacy_setting_value(name, id, &value);
        let lookup = lookup_name(source, name, path);
        let migrated = self
            .js
            .get(source, &lookup)
            .and_then(|card| card.settings_schema.iter().find(|setting| setting.id == id))
            .map(|setting| script::coerce_setting_value(&setting.ty, &migrated))
            .unwrap_or(migrated);
        bag.insert(id.to_string(), migrated);
        let live = self.live_settings(core, profile, &key, source, name, path, &bag);
        let result = self.upsert_profile_settings(
            core,
            profile,
            ArmMirror::script_settings(key.clone(), live),
            "parameter edit",
            |settings| {
                settings.script_settings.insert(key.clone(), bag);
            },
        );
        if let Ok(op) = result.as_ref() {
            self.track_settings_card(*op, script::ScriptSel::Loaded(source, lookup), name);
            self.sync.source_edited(profile, &key);
            self.clear_notice();
        }
        result
    }

    /// The run of card `key` on `profile` a parameter write is for, with
    /// the bag it would receive; `None` when no run of that card exists.
    #[allow(clippy::too_many_arguments)]
    fn live_settings<Io>(
        &self,
        core: &OperatorSession<Io>,
        profile: &str,
        key: &str,
        source: script::ScriptSource,
        name: &str,
        path: &Path,
        overrides: &Map<String, Value>,
    ) -> Option<LiveSettings> {
        let (identity, generation) = live_fence(core, profile, key)?;
        let schema = self
            .js
            .get(source, &lookup_name(source, name, path))
            .map(|c| c.settings_schema.as_slice())
            .unwrap_or_default();
        let bag = self.run_bag(core, profile, schema, overrides);
        Some(LiveSettings {
            identity,
            generation,
            run: None,
            bag: Arc::new(bag),
            prepared: None,
        })
    }

    // ---- Start / Start all / Stop all ----------------------------------

    /// Whether any Load Start has not settled, including paced Starts still
    /// waiting for a permit.
    pub fn starts_pending(&self) -> bool {
        !self.starts.is_empty() || !self.admit.is_empty() || self.start_preparation_pending()
    }

    /// k-of-n place while `profile` waits for a Start-all (or marked) permit.
    pub fn start_queue_place(&self, profile: &str) -> Option<QueuePlace> {
        self.admit.status(profile)
    }

    /// Whether a Start of `profile` is still waiting anywhere it can wait
    /// before its slot runs it: a paced place, a card preparation (tallied
    /// or not), a marked catalog restart, or an Assign & restart waiting
    /// for the catalog scan or for its assignment write. Each of these is
    /// what [`Self::cancel_queued_as`] withdraws.
    pub fn start_waiting(&self, profile: &str) -> bool {
        self.admit.status(profile).is_some()
            || self.has_start_preparation(profile)
            || self.pending_marked_restart(profile)
            || self
                .pending_catalog_assignment_commands
                .iter()
                .any(|command| command.restart && command.profile == profile)
            || self
                .pending_assignment_writes
                .values()
                .flatten()
                .any(|command| command.restart && command.profile == profile)
    }

    /// Drop a queued-but-not-yet-admitted Start. Returns whether a place
    /// was removed. Already-dispatched Starts are stopped through the host.
    pub(crate) fn cancel_queued(&mut self, profile: &str) -> bool {
        self.cancel_queued_as(profile, "cancelled")
    }

    /// Withdraw every waiting Start of `profile` (see
    /// [`Self::start_waiting`]), record `reason` in the running Start
    /// tally and settle each withdrawn Start's command on the next poll.
    pub(crate) fn cancel_queued_as(&mut self, profile: &str, reason: &str) -> bool {
        let mut cancelled = false;
        if self
            .cancel_pending_catalog_assignment(Some(profile))
            .is_some()
        {
            self.credit_skipped(profile, reason);
            cancelled = true;
        }
        if self.admit.leave(profile).is_some() {
            self.credit_skipped(profile, reason);
            cancelled = true;
        }
        if let Some((kind, tallied)) = self.cancel_start_preparation(profile) {
            if tallied {
                self.skip_unstarted_tally(profile, reason);
            } else {
                if kind == StartKind::RestartSettings {
                    self.show(format!("Restart cancelled for {profile}: {reason}"));
                }
                self.start_command_skips
                    .push((profile.to_string(), reason.to_string()));
            }
            cancelled = true;
        }
        if self.withdraw_assignment_restarts(Some(profile), reason) {
            self.publish_bulk();
            cancelled = true;
        }
        cancelled
    }

    /// Drop the restart of every Assign & restart of `profile` (every
    /// profile when `None`) that still waits: one waiting for the catalog
    /// scan is dropped whole (nothing was written); one waiting for its
    /// assignment write keeps the write and drops the Start after it. Each
    /// command settles on the next poll. Returns whether any was dropped.
    fn withdraw_assignment_restarts(&mut self, profile: Option<&str>, reason: &str) -> bool {
        let matches = |restart: bool, name: &str| restart && profile.is_none_or(|p| p == name);
        let mut withdrawn = Vec::new();
        let mut index = 0;
        while index < self.pending_catalog_assignment_commands.len() {
            let command = &self.pending_catalog_assignment_commands[index];
            if matches(command.restart, &command.profile) {
                let command = self.pending_catalog_assignment_commands.swap_remove(index);
                withdrawn.push((command.profile, command.request, command.subject));
            } else {
                index += 1;
            }
        }
        for commands in self.pending_assignment_writes.values_mut() {
            let mut index = 0;
            while index < commands.len() {
                if matches(commands[index].restart, &commands[index].profile) {
                    let command = commands.swap_remove(index);
                    withdrawn.push((command.profile, command.request, command.subject));
                } else {
                    index += 1;
                }
            }
        }
        self.pending_assignment_writes
            .retain(|_, commands| !commands.is_empty());
        let any = !withdrawn.is_empty();
        for (name, request, subject) in withdrawn {
            self.tally_record(&name, Outcome::Skipped(reason.to_string()), LogTo::Slot);
            self.withdrawn_restart_commands
                .push((PendingStartCommand { request, subject }, reason.to_string()));
        }
        any
    }

    /// Open the running tally for a Start click. While a bot it holds still
    /// waits for its permit or its setup, the click folds into it (and
    /// names it); otherwise the finished tally is replaced. A tally a
    /// planned fleet action opens (the last click decides) is never put on
    /// the banner: the plan's result line reports it.
    pub(crate) fn open_tally(&mut self, label: &'static str) {
        let open = self.tally.as_ref().is_some_and(StartTally::waiting)
            || self.starts.values().any(|pending| pending.tallied)
            || self.tallied_start_preparation_pending();
        match self.tally.as_mut() {
            Some(tally) if open => tally.relabel(label),
            _ => self.tally = Some(StartTally::new(label)),
        }
        if let Some(tally) = self.tally.as_mut() {
            tally.set_quiet(self.planned_run);
        }
    }

    /// Mark the dispatch of a planned fleet action ([`crate::Targets::Plan`])
    /// while it runs: see [`Self::open_tally`] and
    /// [`Self::apply_settings_sync`].
    pub(crate) fn set_planned_run(&mut self, planned: bool) {
        self.planned_run = planned;
    }

    /// A click-time skip. A bot this tally already started stays started.
    pub(crate) fn tally_skip(&mut self, profile: &str, reason: &str) {
        self.tally_record(profile, Outcome::Skipped(reason.to_string()), LogTo::Slot);
    }

    /// A click-time skip of a marked row with no profile (no slot to log on).
    pub(crate) fn tally_skip_unavailable(&mut self, row: &str, reason: &str) {
        self.tally_record(row, Outcome::Skipped(reason.to_string()), LogTo::Process);
    }

    pub(crate) fn tally_fail(&mut self, profile: &str, reason: &str) {
        self.tally_record(profile, Outcome::Failed(reason.to_string()), LogTo::Slot);
    }
    pub(crate) fn tally_queued_pending(&mut self, profile: &str) {
        self.tally_record(
            profile,
            Outcome::Held("catalog scan pending".into()),
            LogTo::Slot,
        );
    }

    /// Record `profile`'s outcome in the running tally. A new skip or
    /// failure also goes to the log (`log`), so it survives a later banner.
    fn tally_record(&mut self, profile: &str, outcome: Outcome, log: LogTo) {
        let Some(tally) = self.tally.as_mut() else {
            return;
        };
        if !tally.set(profile, outcome) {
            return;
        }
        let (level, verb, reason) = match tally.outcome(profile) {
            Some(Outcome::Skipped(reason)) => (Level::Info, "skipped", reason),
            Some(Outcome::Failed(reason)) => (Level::Error, "failed", reason),
            Some(Outcome::Held(reason)) => (Level::Info, "queued", reason),
            _ => return,
        };
        let label = tally.label();
        match log {
            LogTo::Slot => crate::log::global().slot_line(
                profile,
                Source::Host,
                level,
                &format!("{label} {verb}: {reason}"),
            ),
            LogTo::Process => crate::log::global().process_line(
                Source::Host,
                level,
                &format!("{label} {verb} {profile}: {reason}"),
            ),
            LogTo::None => {}
        }
    }

    #[cfg(test)]
    /// Start `profile` on its last successful assignment.
    pub(crate) fn start_profile<Io>(
        &mut self,
        core: &mut OperatorSession<Io>,
        profile: &str,
        catalog_root: Option<&Path>,
    ) -> Result<Option<OperationId>, ScriptStartError> {
        if script_active(core, profile) {
            return Ok(None);
        }
        let sel = Self::assignment(core, profile)
            .and_then(|a| sel_from_assignment(&a))
            .ok_or_else(|| ScriptStartError::refused("no assignment"))?;
        self.start_sel(core, profile, &sel, catalog_root, StartKind::Start, false)
            .map_err(ScriptStartError::from)
    }

    /// Operator Start on `profile`: its pending Browse selection, then the
    /// shared Browse selection, then the front end's heading, then its
    /// assignment. Another profile's draft never reaches this Start.
    /// Refused while the profile's script is active.
    pub(crate) fn start_selected<Io>(
        &mut self,
        core: &mut OperatorSession<Io>,
        profile: &str,
        heading: Option<&ScriptChoice>,
        catalog_root: Option<&Path>,
    ) -> Result<Option<OperationId>, ScriptStartError> {
        let heading = heading.map(ScriptChoice::as_script);
        if script_active(core, profile) {
            return Err(ScriptStartError::refused(
                "script already active: stop it first",
            ));
        }
        let sel = self
            .pending_browse
            .get(profile)
            .map(|sel| sel.as_script().clone())
            .or_else(|| {
                self.browse_selection
                    .as_ref()
                    .map(ScriptChoice::as_script)
                    .cloned()
            })
            .or_else(|| heading.cloned())
            .or_else(|| Self::assignment(core, profile).and_then(|a| sel_from_assignment(&a)))
            .ok_or_else(|| ScriptStartError::refused("no assignment"))?;
        self.start_sel(core, profile, &sel, catalog_root, StartKind::Start, false)
            .map_err(ScriptStartError::from)
    }

    /// The loaded card `(source, lookup)` names, if it can start. A file
    /// card whose file is gone is refused as missing, the same on every
    /// platform: Start never runs a deleted script from the copy loaded
    /// earlier, whether or not the card's stored path still resolves.
    /// Missing catalog cards name the root whose registry was scanned.
    pub(crate) fn startable_card(
        &self,
        source: script::ScriptSource,
        lookup: &str,
    ) -> Result<&script::JsCard, String> {
        self.js
            .get(source, lookup)
            .filter(|card| source != script::ScriptSource::File || card.path.is_file())
            .ok_or_else(|| match source {
                script::ScriptSource::File => format!("missing file: {lookup}"),
                script::ScriptSource::Catalog => match self.catalog_root.as_deref() {
                    Some(root) => format!(
                        "catalog at {} was scanned, but card {lookup} is missing",
                        root.display()
                    ),
                    None => {
                        format!("catalog card {lookup} is missing; no catalog root was scanned")
                    }
                },
                _ => format!("unavailable: {lookup}"),
            })
    }

    fn start_sel<Io>(
        &mut self,
        core: &mut OperatorSession<Io>,
        profile: &str,
        sel: &script::ScriptSel,
        catalog_root: Option<&Path>,
        kind: StartKind,
        tallied: bool,
    ) -> Result<Option<OperationId>, script::StartLoadError> {
        if self.parameter_edit_blocks_start(profile) {
            return Err(script::StartLoadError::Waiting(
                "Settings are uncommitted or saving. In Script prefs, press Enter to save a field or discard unsaved settings.".into(),
            ));
        }
        if core.play().is_none() {
            return Err(script::StartLoadError::Refused("no play".into()));
        }
        let tallied = tallied || self.admit.contains(profile);
        let result = self.dispatch_start(core, profile, sel, catalog_root, kind, tallied);
        if matches!(&result, Err(script::StartLoadError::Waiting(_))) {
            if let Err(error) = &result {
                self.show(error.to_string());
            }
            return result;
        }
        let from_queue = self.admit.leave(profile).is_some();
        match &result {
            Ok(_) => self.credit_queue_outcome(from_queue, profile, Ok(())),
            Err(error) => self.credit_queue_outcome(from_queue, profile, Err(&error.to_string())),
        }
        result
    }

    fn dispatch_start<Io>(
        &mut self,
        core: &mut OperatorSession<Io>,
        profile: &str,
        sel: &script::ScriptSel,
        catalog_root: Option<&Path>,
        kind: StartKind,
        tallied: bool,
    ) -> Result<Option<OperationId>, script::StartLoadError> {
        // Every running bot is a fleet member (`open_slot`, `seed_running`),
        // so a non-member is a profile that is not loaded: say so, for both
        // card kinds, instead of the host's "no slot".
        if !wall_member(core, profile) {
            return Err(script::StartLoadError::Refused(format!(
                "{profile} is not loaded"
            )));
        }
        match sel {
            script::ScriptSel::Compiled(id) => {
                let bag = self
                    .compiled_bag(core, profile, CompiledCardId::from_script(*id))
                    .map_err(script::StartLoadError::Refused)?;
                let op = core.start_script(
                    profile,
                    ScriptStart::compiled(CompiledCardId::from_script(*id), bag),
                    None,
                )?;
                self.starts.insert(
                    profile.to_string(),
                    PendingStart {
                        card: PendingCard::Compiled(*id),
                        kind,
                        tallied,
                    },
                );
                Ok(Some(op))
            }
            script::ScriptSel::Loaded(..) => {
                if let Some(reason) = core
                    .play()
                    .and_then(host_play::Play::script_start_wait_reason)
                {
                    return Err(script::StartLoadError::Waiting(format!(
                        "{profile}: blocked, {reason}"
                    )));
                }
                self.queue_loaded_start(core, profile, sel, catalog_root, kind, tallied)
                    .map(|()| None)
            }
        }
    }

    /// Start every wall member idle on its last successful assignment.
    /// Eligible Starts are enqueued and released through the Start-all
    /// permit (at most [`START_ADMIT_PER_FRAME`] in this click). Active
    /// members are skipped. The running tally is shown; a member whose setup
    /// fails later moves from started to failed.
    pub(crate) fn start_all<Io>(
        &mut self,
        core: &mut OperatorSession<Io>,
        catalog_root: Option<&Path>,
    ) -> Vec<String> {
        let members = core.members().to_vec();
        if members.is_empty() {
            self.show("Start all: no wall members");
            return Vec::new();
        }
        if let Some(root) = catalog_root {
            self.admit_catalog = Some(root.to_path_buf());
        }
        self.open_tally("Start all");
        let mut accepted = Vec::new();
        for name in members {
            if self.admit.contains(&name) || self.has_start_preparation(&name) {
                accepted.push(name);
                continue;
            }
            if script_active(core, &name) {
                self.tally_skip(&name, "already active");
                continue;
            }
            let sel = Self::assignment(core, &name).and_then(|a| sel_from_assignment(&a));
            let Some(sel) = sel else {
                self.tally_fail(&name, "no assignment");
                continue;
            };
            match self.queue_start(core, &name, sel.clone(), Some(sel)) {
                Ok(()) => accepted.push(name),
                Err(error) => self.tally_fail(&name, &error),
            }
        }
        self.admit_starts(core, catalog_root);
        self.publish_bulk();
        accepted
    }

    /// Enqueue `profile` for a paced Start in the running tally (opened by
    /// the click). Immediate Start paths call [`Self::start_sel`] instead.
    /// Idempotent per profile.
    pub(crate) fn queue_start<Io>(
        &mut self,
        core: &OperatorSession<Io>,
        profile: &str,
        sel: script::ScriptSel,
        assigned: Option<script::ScriptSel>,
    ) -> Result<(), String> {
        if core.play().is_none() {
            return Err("no play".into());
        }
        if script_active(core, profile) {
            return Err("script already active: stop it first".into());
        }
        if self.admit.contains(profile) {
            return Ok(());
        }
        if self.has_start_preparation(profile) {
            self.tally_skip(profile, "already starting");
            return Ok(());
        }
        let (had_arm, latched) = arm_flags(core, profile);
        self.admit.enqueue(QueuedStart {
            profile: profile.to_string(),
            sel,
            assigned,
            latched,
            had_arm,
            kind: StartKind::Start,
        });
        self.tally_record(profile, Outcome::Queued, LogTo::None);
        Ok(())
    }

    /// Grant up to [`START_ADMIT_PER_FRAME`] waiting Starts.
    pub(crate) fn admit_starts<Io>(
        &mut self,
        core: &mut OperatorSession<Io>,
        catalog_root: Option<&Path>,
    ) {
        if let Some(root) = catalog_root {
            self.admit_catalog = Some(root.to_path_buf());
        }
        self.admit_ready(core);
        self.publish_start_places(core);
    }

    #[cfg(test)]
    /// Stop every member's script (and any other running slot), including a
    /// slot still reaping a reload whose replacement Start is queued: that
    /// Start is dropped. Queued-but-not-yet-admitted Starts are cancelled.
    /// Any reload in preparation or awaiting confirmation is cancelled.
    /// Returns how many were stopped.
    pub(crate) fn stop_all<Io>(&mut self, core: &mut OperatorSession<Io>) -> usize {
        self.stop_all_with_receipt(core).0
    }

    pub(crate) fn stop_all_with_receipt<Io>(
        &mut self,
        core: &mut OperatorSession<Io>,
    ) -> (usize, Option<OperationId>) {
        let cancelled = self.admit.drain();
        for entry in &cancelled {
            self.withdraw_start(&entry.profile, "cancelled");
        }
        let preparing = self.cancel_all_start_preparations();
        for (profile, _, tallied) in &preparing {
            if *tallied {
                if let Some(tally) = &mut self.tally {
                    tally.retract_unstarted(profile);
                }
                self.withdraw_start(profile, "cancelled");
            } else {
                self.start_command_skips
                    .push((profile.clone(), "cancelled".to_string()));
            }
        }
        let pending_marked = self
            .cancel_pending_catalog_assignment(None)
            .unwrap_or_default();
        for profile in &pending_marked {
            self.withdraw_start(profile, "cancelled");
        }
        let restarts = self.withdraw_assignment_restarts(None, "cancelled");
        let cancelled_any = !cancelled.is_empty()
            || !preparing.is_empty()
            || !pending_marked.is_empty()
            || restarts;
        if cancelled_any {
            self.publish_bulk();
        }
        self.publish_start_places(core);
        let (stopped, operation) = if core.play().is_none() {
            (0, None)
        } else {
            let names = slot_names(core);
            let (operation, stopped) = core.stop_scripts(&names);
            (stopped, Some(operation))
        };
        self.reload.invalidate();
        let report = format!("Stop all: stopped {stopped}");
        if self.last_bulk_report.as_deref() != Some(report.as_str()) {
            self.last_bulk_report = Some(report);
            if stopped > 0 || cancelled_any {
                self.clear_notice();
            }
        }
        (stopped, operation)
    }

    /// The running Start all / marked-Start report, or the last Stop all
    /// report when that came later.
    pub fn last_bulk_report(&self) -> Option<&str> {
        self.last_bulk_report.as_deref()
    }

    fn pump_transpile_queue<Io>(&mut self, core: &mut OperatorSession<Io>) {
        let settled = if let Some((source, lookup, result)) = self.take_transpile_result() {
            let index = self
                .transpile_queue
                .iter()
                .position(|entry| entry.source == source && entry.lookup == lookup);
            if let Some(entry) = index.and_then(|index| self.transpile_queue.remove(index)) {
                self.finish_transpile_entry(core, entry, result);
            }
            true
        } else {
            false
        };
        let armed = std::mem::replace(&mut self.transpile_armed, false);
        if armed && !settled && !self.transpile_pending() {
            if let Some(entry) = self.transpile_queue.pop_front() {
                match self.request_transpile(entry.source, &entry.lookup) {
                    Ok(()) => self.transpile_queue.push_front(entry),
                    Err(reason) => self.finish_transpile_entry(core, entry, Err(reason)),
                }
            }
        }
        if self.transpile_queue.is_empty() {
            if let Some(request) = self.transpile_all_request.take() {
                let (terminal, detail) = match self.transpile_all_failure.take() {
                    Some(detail) => (
                        Settlement::Failed(CommandFailure::Failed(FailureKind::JobFailed)),
                        Some(detail),
                    ),
                    None => (Settlement::Completed(Completion::Done), None),
                };
                core.settle_later(request, Subject::Application, terminal, detail.as_deref());
                self.transpile_done = 0;
                self.transpile_total = 0;
            }
        } else {
            self.transpile_armed = true;
        }
    }

    fn finish_transpile_entry<Io>(
        &mut self,
        core: &mut OperatorSession<Io>,
        entry: TranspileEntry,
        result: Result<(), TranspileError>,
    ) {
        self.transpile_done = self.transpile_done.saturating_add(1);
        let (terminal, detail) = match result {
            Ok(()) => (Settlement::Completed(Completion::Done), None),
            Err(error) => {
                let reason = error.to_string();
                self.show(format!("Transpile {}: {reason}", entry.lookup));
                if self.transpile_all_request.is_some() && self.transpile_all_failure.is_none() {
                    self.transpile_all_failure = Some(reason.clone());
                }
                (
                    Settlement::Failed(CommandFailure::Failed(FailureKind::JobFailed)),
                    Some(reason),
                )
            }
        };
        for request in entry.requests {
            core.settle_later(
                request,
                Subject::Application,
                terminal.clone(),
                detail.as_deref(),
            );
        }
    }

    fn retain_catalog_fill_result(&mut self) {
        if self.catalog_fill_update.is_none() {
            self.catalog_fill_update = self
                .take_catalog_fill_result()
                .map(|result| result.map_err(|error| error.to_string()));
        }
    }

    fn settle_pending_catalog_assignment_commands<Io>(&mut self, core: &mut OperatorSession<Io>) {
        if self.pending_catalog_assignment_commands.is_empty() || self.catalog_work_pending() {
            return;
        }
        self.retain_catalog_fill_result();
        let pending = std::mem::take(&mut self.pending_catalog_assignment_commands);
        for command in pending {
            let catalog_failure = self
                .catalog_fill_update
                .as_ref()
                .and_then(|result| result.as_ref().err())
                .cloned();
            if let Some(reason) = catalog_failure {
                if command.restart {
                    self.fail_assignment_restart(&command.profile, &reason);
                }
                core.settle_later(
                    command.request,
                    command.subject,
                    Settlement::Failed(CommandFailure::Failed(FailureKind::JobFailed)),
                    Some(&reason),
                );
                continue;
            }
            let root = command
                .catalog_root
                .clone()
                .or_else(|| self.catalog_root.clone());
            let selection = command.selection.as_script().clone();
            let assignment = match self.assignment_for(&selection, root.as_deref()) {
                Ok(Some(assignment)) => assignment,
                Ok(None) => {
                    self.pending_catalog_assignment_commands.push(command);
                    continue;
                }
                Err(reason) => {
                    if command.restart {
                        self.fail_assignment_restart(&command.profile, &reason);
                    }
                    core.settle_later(
                        command.request,
                        command.subject,
                        Settlement::Failed(CommandFailure::Rejected(
                            crate::command::Rejection::StartRefused,
                        )),
                        Some(&reason),
                    );
                    continue;
                }
            };
            self.clear_pending_browse(&command.profile);
            let log_action = if command.restart {
                "Assign & restart"
            } else {
                "Assign"
            };
            match self.queue_assignment_with_mirror(
                core,
                &command.profile,
                assignment,
                log_action,
                ArmMirror::script_assignment(),
            ) {
                Ok(Some(op)) => self.pending_assignment_writes.entry(op).or_default().push(
                    PendingAssignmentWrite {
                        request: command.request,
                        subject: command.subject,
                        profile: command.profile,
                        selection: command.selection,
                        catalog_root: command.catalog_root,
                        restart: command.restart,
                    },
                ),
                Ok(None) if command.restart => self.start_assigned_command(
                    core,
                    PendingAssignmentWrite {
                        request: command.request,
                        subject: command.subject,
                        profile: command.profile,
                        selection: command.selection,
                        catalog_root: command.catalog_root,
                        restart: true,
                    },
                ),
                Ok(None) => core.settle_later(
                    command.request,
                    command.subject,
                    Settlement::Completed(Completion::Done),
                    None,
                ),
                Err(reason) => {
                    if command.restart {
                        self.fail_assignment_restart(&command.profile, &reason);
                    }
                    core.settle_later(
                        command.request,
                        command.subject,
                        Settlement::Failed(CommandFailure::Rejected(
                            crate::command::Rejection::VaultRefused,
                        )),
                        Some(&reason),
                    );
                }
            }
        }
    }

    fn settle_assignment_write<Io>(
        &mut self,
        core: &mut OperatorSession<Io>,
        write: ScriptAssignmentWrite,
    ) {
        let Some(commands) = self.pending_assignment_writes.remove(&write.op()) else {
            return;
        };
        let failure = if write.result().is_failed() {
            Some(
                write
                    .result()
                    .error()
                    .unwrap_or("profile assignment write failed"),
            )
        } else if write.result().is_superseded() {
            Some("profile assignment was superseded")
        } else {
            None
        };
        for command in commands {
            if let Some(reason) = failure {
                if command.restart {
                    self.fail_assignment_restart(&command.profile, reason);
                }
                core.settle_later(
                    command.request,
                    command.subject,
                    Settlement::Failed(CommandFailure::Failed(FailureKind::WriteFailed)),
                    Some(reason),
                );
            } else if command.restart {
                self.start_assigned_command(core, command);
            } else {
                core.settle_later(
                    command.request,
                    command.subject,
                    Settlement::Completed(Completion::Done),
                    None,
                );
            }
        }
    }

    fn fail_assignment_restart(&mut self, profile: &str, reason: &str) {
        self.tally_record(profile, Outcome::Failed(reason.to_owned()), LogTo::None);
        self.publish_bulk();
    }

    fn start_assigned_command<Io>(
        &mut self,
        core: &mut OperatorSession<Io>,
        command: PendingAssignmentWrite,
    ) {
        let state = core.play().map_or(script::RunState::Idle, |play| {
            play.script_state(&command.profile)
        });
        let stopping = matches!(
            state,
            script::RunState::Starting | script::RunState::Running | script::RunState::Paused
        );
        if stopping {
            core.stop_script(&command.profile);
        }
        match self.queue_restart(
            core,
            &command.profile,
            command.selection.as_script().clone(),
            stopping || state == script::RunState::Stopping,
        ) {
            Ok(()) => {
                self.register_start_command(command.profile, command.request, command.subject);
                self.admit_starts(core, command.catalog_root.as_deref());
            }
            Err(reason) => {
                self.fail_assignment_restart(&command.profile, &reason);
                core.settle_later(
                    command.request,
                    command.subject,
                    Settlement::Failed(CommandFailure::Failed(FailureKind::StartFailed)),
                    Some(&reason),
                );
            }
        }
    }

    fn settle_pending_application_commands<Io>(&mut self, core: &mut OperatorSession<Io>) {
        self.retain_catalog_fill_result();
        // Only an ImportCatalog row collects the import receipt; a request
        // made without one keeps it for its caller.
        let import_pending = self
            .pending_application_commands
            .iter()
            .any(|pending| pending.wait == PendingApplicationWait::CatalogImport);
        if let Some((root, result)) = import_pending
            .then(|| self.take_catalog_import_result())
            .flatten()
        {
            let result = result.map_err(|error| error.to_string());
            self.catalog_import_command_result = Some(result.clone());
            self.catalog_import_update = Some((root, result));
        }
        let pending = std::mem::take(&mut self.pending_application_commands);
        for command in pending {
            let result = match command.wait {
                PendingApplicationWait::CatalogFill => {
                    self.catalog_fill_update
                        .as_ref()
                        .map(|result| match result {
                            Ok(()) => (Settlement::Completed(Completion::Done), None),
                            Err(reason) => (
                                Settlement::Failed(CommandFailure::Failed(FailureKind::JobFailed)),
                                Some(reason.clone()),
                            ),
                        })
                }
                PendingApplicationWait::CatalogImport => self
                    .catalog_import_command_result
                    .as_ref()
                    .map(|result| match result {
                        Ok(_) => (Settlement::Completed(Completion::Done), None),
                        Err(reason) => (
                            Settlement::Failed(CommandFailure::Failed(FailureKind::JobFailed)),
                            Some(reason.clone()),
                        ),
                    }),
                PendingApplicationWait::Reload => self.reload_outcome().map(|outcome| {
                    outcome.failure().map_or_else(
                        || (Settlement::Completed(Completion::Done), None),
                        |reason| {
                            (
                                Settlement::Failed(CommandFailure::Failed(FailureKind::JobFailed)),
                                Some(reason.to_owned()),
                            )
                        },
                    )
                }),
                PendingApplicationWait::RestartSettings => {
                    (!self.starts_pending()).then(|| match self.restart_settings_failure.take() {
                        Some(reason) => (
                            Settlement::Failed(CommandFailure::Failed(FailureKind::StartFailed)),
                            Some(reason),
                        ),
                        None => (Settlement::Completed(Completion::Done), None),
                    })
                }
            };
            if let Some((terminal, detail)) = result {
                core.settle_later(
                    command.request,
                    Subject::Application,
                    terminal,
                    detail.as_deref(),
                );
            } else if command.wait == PendingApplicationWait::Reload
                && !self.reload_validation_pending()
            {
                core.settle_later(
                    command.request,
                    Subject::Application,
                    Settlement::Cancelled(api::identity::Cancellation::Operator),
                    None,
                );
            } else if matches!(
                command.wait,
                PendingApplicationWait::CatalogFill | PendingApplicationWait::CatalogImport
            ) && !self.catalog_work_pending()
            {
                core.settle_later(
                    command.request,
                    Subject::Application,
                    Settlement::Completed(Completion::Done),
                    None,
                );
            } else {
                self.pending_application_commands.push(command);
            }
        }
        if !self
            .pending_application_commands
            .iter()
            .any(|pending| pending.wait == PendingApplicationWait::CatalogImport)
        {
            self.catalog_import_command_result = None;
        }
        if self.pending_catalog_assignment_commands.is_empty()
            && !self
                .pending_application_commands
                .iter()
                .any(|pending| pending.wait == PendingApplicationWait::CatalogFill)
        {
            self.catalog_fill_update = None;
        }
    }

    // ---- per-frame settlement ------------------------------------------

    /// Fold everything that settled since the last frame: reload
    /// validation, Load Starts (Ready commits the assignment and clears the
    /// card's load diagnostic; a failure records it) and parameter writes.
    /// Call once per UI frame, after [`OperatorSession::poll`].
    pub fn poll<Io>(&mut self, core: &mut OperatorSession<Io>) {
        self.settle_parameter_edits(core);
        self.poll_script_work(core);
        self.pump_transpile_queue(core);
        self.settle_pending_catalog_assignment_commands(core);
        self.poll_reload_preparation(core);
        self.poll_reload_validation(core);
        self.release_stopped_restarts(core);
        if !self.admit.is_empty() {
            self.admit_ready(core);
        }
        self.publish_start_places(core);
        self.settle_starts(core);
        self.settle_start_command_failures(core);
        self.settle_pending_application_commands(core);
        while let Some(write) = core.take_script_assignment_write() {
            self.settle_assignment_write(core, write);
        }
        for write in core.take_settings_writes() {
            let is_sync_write = self.sync.tracks_write(write.op);
            if write.result.delivery() == Some(&LiveDelivery::RestartRequired) {
                let card = self.settings_cards.remove(&write.op).or_else(|| {
                    let assignment = Self::assignment(core, &write.profile)?;
                    Some(SettingsCard {
                        selection: sel_from_assignment(&assignment)?,
                        name: assignment.display_name,
                    })
                });
                if let Some(card) = card {
                    let status = core
                        .play()
                        .and_then(|play| play.script_native_status(&write.profile));
                    self.sync.note_restart_required(
                        &write.profile,
                        &card.selection,
                        status.as_deref().map(|status| status.active_settings),
                    );
                    if !is_sync_write {
                        self.sync
                            .queue_single_restart_prompt(write.op, card, &write.profile);
                    }
                }
            } else {
                self.settings_cards.remove(&write.op);
            }
            let sync_outcome = self.sync.record(&write);
            if let Some((op, outcome)) = sync_outcome {
                core.set_outcome(op, &write.profile, outcome);
            }
        }
        self.pump_copy_queues(core);
        self.sync.observe_restart_activation(core);
        if let Some(summary) = self.sync.take_settled_summary() {
            self.show(summary);
        }
    }

    fn settle_starts<Io>(&mut self, core: &mut OperatorSession<Io>) {
        for StartSettled {
            slot: name,
            outcome,
            ..
        } in core.take_settled_starts()
        {
            let command_outcome = outcome.clone();
            let Some(pending) = self.starts.remove(&name) else {
                self.settle_start_commands(core, &name, command_outcome);
                continue;
            };
            if matches!(&outcome, Some(script::StartOutcome::Ready)) {
                self.sync.clear_restart_badge(&name);
            }
            if pending.kind == StartKind::RestartSettings {
                match command_outcome.as_ref() {
                    Some(script::StartOutcome::Failed(reason))
                        if self.restart_settings_failure.is_none() =>
                    {
                        self.restart_settings_failure = Some(reason.clone());
                    }
                    Some(script::StartOutcome::Rejected(reason))
                        if self.restart_settings_failure.is_none() =>
                    {
                        self.restart_settings_failure = Some(reason.to_string());
                    }
                    _ => {}
                }
            }
            match outcome {
                Some(script::StartOutcome::Ready) if pending.kind == StartKind::RestartSettings => {
                }
                Some(script::StartOutcome::Ready) => match pending.card {
                    PendingCard::Compiled(id) => {
                        let key = script::compiled_identity_key(id);
                        let _ = self.upsert_profile_settings(
                            core,
                            &name,
                            ArmMirror::none(),
                            "Start",
                            |settings| {
                                settings.script_assignment = Some(script::compiled_assignment(id));
                                if settings.script_settings.get(&key).is_none_or(Map::is_empty) {
                                    let schema_version =
                                        if settings.script_settings.contains_key(&key) {
                                            1 // Empty legacy envelopes are schema 1.
                                        } else {
                                            script::compiled_card(id)
                                                .expect("registered card")
                                                .schema_version
                                        };
                                    settings.script_settings.insert(
                                        key,
                                        vault::CompiledSettingsRecord {
                                            schema_version,
                                            values: Map::new(),
                                        }
                                        .into_entry(),
                                    );
                                }
                            },
                        );
                    }
                    PendingCard::Loaded(card) => {
                        let key = card.identity_key();
                        let had_failure = self.js.load_failure(&key).is_some();
                        let listing = self.shown.take_if(|shown| {
                            had_failure && *shown == self.js.named_failure_output()
                        });
                        let _ = self.js.record_start_result(&card, Ok(()));
                        if let Some(shown) = listing {
                            let with = (!self.js.load_failures().is_empty())
                                .then(|| self.js.named_failure_output());
                            self.shown.clone_from(&with);
                            self.notice = Some(Notice::retract(shown, with));
                        }
                        self.persist_assignment_with_action(
                            core,
                            &name,
                            card.assignment(),
                            "Start",
                        );
                    }
                },
                Some(script::StartOutcome::Failed(error)) => {
                    let diagnostic = match pending.card {
                        PendingCard::Loaded(card) => self
                            .js
                            .record_start_result(
                                &card,
                                Err(script::StartLoadError::RuntimeLoad(error.clone())),
                            )
                            .err()
                            .unwrap_or(error),
                        PendingCard::Compiled(_) => error,
                    };
                    self.report_start_failure(&name, pending.kind, pending.tallied, diagnostic);
                }
                Some(script::StartOutcome::Rejected(error)) => {
                    self.report_start_failure(
                        &name,
                        pending.kind,
                        pending.tallied,
                        error.to_string(),
                    );
                }
                Some(script::StartOutcome::Cancelled) | None => {}
            }
            self.settle_start_commands(core, &name, command_outcome);
        }
    }

    fn settle_start_commands<Io>(
        &mut self,
        core: &mut OperatorSession<Io>,
        profile: &str,
        outcome: Option<script::StartOutcome>,
    ) {
        let Some(commands) = self.pending_start_commands.remove(profile) else {
            return;
        };
        for command in commands {
            let (terminal, detail): (Settlement<Completion, CommandFailure>, Option<String>) =
                match outcome.as_ref() {
                    Some(script::StartOutcome::Ready) => {
                        (Settlement::Completed(Completion::Done), None)
                    }
                    Some(script::StartOutcome::Failed(reason)) => (
                        Settlement::Failed(CommandFailure::Failed(FailureKind::StartFailed)),
                        Some(reason.clone()),
                    ),
                    Some(script::StartOutcome::Rejected(reason)) => (
                        Settlement::Failed(CommandFailure::Failed(FailureKind::StartFailed)),
                        Some(reason.to_string()),
                    ),
                    Some(script::StartOutcome::Cancelled) | None => (
                        Settlement::Cancelled(api::identity::Cancellation::Superseded),
                        None,
                    ),
                };
            core.settle_later(
                command.request,
                command.subject,
                terminal,
                detail.as_deref(),
            );
        }
    }

    /// Settle the Start commands of profiles whose Start failed or was
    /// skipped without a slot terminal (setup failures, and queued Starts
    /// that left the queue failed or skipped), so a command's report
    /// never waits on a Start that will not happen.
    fn settle_start_command_failures<Io>(&mut self, core: &mut OperatorSession<Io>) {
        for (profile, reason) in std::mem::take(&mut self.start_command_failures) {
            let Some(commands) = self.pending_start_commands.remove(&profile) else {
                continue;
            };
            for command in commands {
                core.settle_later(
                    command.request,
                    command.subject,
                    Settlement::Failed(CommandFailure::Failed(FailureKind::StartFailed)),
                    Some(&reason),
                );
            }
        }
        for (profile, reason) in std::mem::take(&mut self.start_command_skips) {
            let Some(commands) = self.pending_start_commands.remove(&profile) else {
                continue;
            };
            let terminal = withdrawn_terminal(&reason);
            for command in commands {
                core.settle_later(
                    command.request,
                    command.subject,
                    terminal.clone(),
                    Some(&reason),
                );
            }
        }
        for (command, reason) in std::mem::take(&mut self.withdrawn_restart_commands) {
            core.settle_later(
                command.request,
                command.subject,
                withdrawn_terminal(&reason),
                Some(&reason),
            );
        }
    }

    /// A Start that failed during setup. Its operation already logged the
    /// failure on the bot.
    fn report_start_failure(
        &mut self,
        name: &str,
        kind: StartKind,
        tallied: bool,
        diagnostic: String,
    ) {
        if kind == StartKind::Reload {
            self.show(format!("reload: {name}: {diagnostic}"));
            return;
        }
        let message = format!("script: {diagnostic}");
        if tallied
            && self
                .tally
                .as_ref()
                .is_some_and(|t| t.outcome(name).is_some())
        {
            self.tally_record(name, Outcome::Failed(diagnostic.clone()), LogTo::None);
            self.publish_bulk();
        } else {
            self.show(message.clone());
        }
        self.start_command_failures
            .push((name.to_string(), diagnostic));
        self.start_failures.push((name.to_string(), message));
    }

    fn admit_ready<Io>(&mut self, core: &mut OperatorSession<Io>) {
        let mut granted = 0;
        while granted < START_ADMIT_PER_FRAME {
            if self.admit.poll() != StartPermit::Grant {
                break;
            }
            let Some(entry) = self.admit.take_head() else {
                break;
            };
            if self.dispatch_queued(core, entry) {
                granted += 1;
            }
        }
    }

    /// `true` when `start_sel` ran (counts against the frame budget).
    fn dispatch_queued<Io>(&mut self, core: &mut OperatorSession<Io>, entry: QueuedStart) -> bool {
        if !wall_member(core, &entry.profile) || core.removal_pending(&entry.profile) {
            self.drop_queued(&entry, "removed");
            return false;
        }
        if script_active(core, &entry.profile) {
            self.drop_queued(&entry, "already active");
            return false;
        }
        if let Some(required) = &entry.assigned {
            let current =
                Self::assignment(core, &entry.profile).and_then(|a| sel_from_assignment(&a));
            if current.as_ref() != Some(required) {
                self.drop_queued(&entry, "reassigned");
                return false;
            }
        }
        let (has_arm, latched_now) = arm_flags(core, &entry.profile);
        if entry.had_arm && !has_arm {
            self.drop_queued(&entry, "disconnected");
            return false;
        }
        if !entry.latched && latched_now {
            self.drop_queued(&entry, "logged out");
            return false;
        }
        let catalog = self.admit_catalog.clone();
        let tallied = entry.kind != StartKind::RestartSettings;
        match self.start_sel(
            core,
            &entry.profile,
            &entry.sel,
            catalog.as_deref(),
            entry.kind,
            tallied,
        ) {
            Ok(_) if tallied => self.credit_started(&entry.profile),
            Ok(_) => {}
            Err(script::StartLoadError::Waiting(reason)) => {
                if tallied {
                    self.tally_record(&entry.profile, Outcome::Held(reason), LogTo::Slot);
                    self.publish_bulk();
                }
                self.admit.return_head(entry);
                return true;
            }
            Err(error) if tallied => self.credit_failed(&entry.profile, &error.to_string()),
            Err(error) => self.show(format!("Restart failed for {}: {error}", entry.profile)),
        }
        true
    }

    fn drop_queued(&mut self, entry: &QueuedStart, reason: &str) {
        if entry.kind == StartKind::RestartSettings {
            self.show(format!("Skipped {}: {reason}", entry.profile));
        } else {
            self.credit_skipped(&entry.profile, reason);
        }
    }

    /// Credit a Start that left the queue through [`Self::start_sel`].
    fn credit_queue_outcome(&mut self, from_queue: bool, profile: &str, outcome: Result<(), &str>) {
        if !from_queue {
            return;
        }
        match outcome {
            Ok(()) => self.credit_started(profile),
            Err(error) => self.credit_failed(profile, error),
        }
    }

    fn credit_started(&mut self, profile: &str) {
        self.tally_record(profile, Outcome::Started, LogTo::None);
        self.publish_bulk();
    }

    fn credit_failed(&mut self, profile: &str, error: &str) {
        self.tally_record(profile, Outcome::Failed(error.to_string()), LogTo::Slot);
        self.publish_bulk();
        self.start_command_failures
            .push((profile.to_string(), error.to_string()));
    }

    fn credit_skipped(&mut self, profile: &str, reason: &str) {
        self.withdraw_start(profile, reason);
        self.publish_bulk();
    }

    /// A Start that left the queue or its preparation without running:
    /// its tally row says why, and its command settles on the next poll.
    fn withdraw_start(&mut self, profile: &str, reason: &str) {
        self.tally_record(profile, Outcome::Skipped(reason.to_string()), LogTo::Slot);
        self.start_command_skips
            .push((profile.to_string(), reason.to_string()));
    }
    /// The permit was credited, but the worker had not dispatched a script.
    fn skip_unstarted_tally(&mut self, profile: &str, reason: &str) {
        if let Some(tally) = &mut self.tally {
            tally.retract_unstarted(profile);
        }
        self.credit_skipped(profile, reason);
    }

    /// Show the running tally when its text changed (never a planned
    /// action's: its result line reports it).
    pub(crate) fn publish_bulk(&mut self) {
        let Some(tally) = self.tally.as_ref() else {
            return;
        };
        let report = tally.report();
        if self.last_bulk_report.as_deref() == Some(report.as_str()) {
            return;
        }
        let quiet = tally.quiet();
        self.last_bulk_report = Some(report.clone());
        if !quiet {
            self.show(report);
        }
    }

    pub(crate) fn publish_start_places<Io>(&mut self, core: &mut OperatorSession<Io>) {
        if !self.admit.take_publish() {
            return;
        }
        core.publish_start_queue(|name| self.admit.status(name));
    }
}
fn script_settings_values(settings: &vault::ProfileSettings, identity: &str) -> Map<String, Value> {
    if identity.starts_with("compiled:") {
        return settings
            .script_settings
            .get(identity)
            .and_then(|entry| vault::CompiledSettingsRecord::view(entry).ok())
            .map(|(_, values)| values.clone())
            .unwrap_or_default();
    }
    settings
        .script_settings
        .get(identity)
        .cloned()
        .unwrap_or_default()
}

fn format_script_settings_log(
    card: &str,
    action: &str,
    schema: &[script::SettingDef],
    old: &Map<String, Value>,
    new: &Map<String, Value>,
) -> String {
    let mut details = String::new();
    write!(&mut details, "card {card}; action {action}").expect("writing to String cannot fail");
    let mut changed = false;
    for setting in schema {
        if old.get(&setting.id) == new.get(&setting.id) {
            continue;
        }
        changed = true;
        let default = setting_default_value(setting);
        append_script_setting_log_change(
            &mut details,
            &setting.id,
            old.get(&setting.id).or(default.as_ref()),
            new.get(&setting.id).or(default.as_ref()),
        );
    }
    for (field, old_value) in old {
        if schema.iter().any(|setting| setting.id == *field) || new.get(field) == Some(old_value) {
            continue;
        }
        changed = true;
        append_script_setting_log_change(&mut details, field, Some(old_value), new.get(field));
    }
    for (field, new_value) in new {
        if schema.iter().any(|setting| setting.id == *field) || old.contains_key(field) {
            continue;
        }
        changed = true;
        append_script_setting_log_change(&mut details, field, None, Some(new_value));
    }
    if !changed {
        let old = serde_json::to_string(old).expect("JSON settings maps always serialize");
        let new = serde_json::to_string(new).expect("JSON settings maps always serialize");
        append_script_setting_log_change(
            &mut details,
            "settings",
            Some(&Value::String(old)),
            Some(&Value::String(new)),
        );
    }
    details
}

fn append_script_setting_log_change(
    details: &mut String,
    field: &str,
    old: Option<&Value>,
    new: Option<&Value>,
) {
    write!(details, "; field {field}; old ").expect("writing to String cannot fail");
    match old {
        Some(value) => write!(details, "{value}").expect("writing to String cannot fail"),
        None => details.push_str("<unset>"),
    }
    details.push_str("; new ");
    match new {
        Some(value) => write!(details, "{value}").expect("writing to String cannot fail"),
        None => details.push_str("<unset>"),
    }
}

fn setting_default_value(setting: &script::SettingDef) -> Option<Value> {
    setting.default.as_deref().map(|default| {
        serde_json::from_str(default).unwrap_or_else(|_| Value::String(default.to_owned()))
    })
}

/// The command terminal of a Start withdrawn before it ran, by the reason
/// it was withdrawn.
fn withdrawn_terminal(reason: &str) -> Settlement<Completion, CommandFailure> {
    use crate::command::SkipReason;
    match reason {
        "cancelled" => Settlement::Cancelled(api::identity::Cancellation::Operator),
        "already active" | "script already active" => {
            Settlement::Completed(Completion::Skipped(SkipReason::AlreadyActive))
        }
        "logged out" | "disconnected" => {
            Settlement::Completed(Completion::Skipped(SkipReason::NotLoggedIn))
        }
        "removed" => Settlement::Completed(Completion::Skipped(SkipReason::NotLoaded)),
        _ => Settlement::Completed(Completion::Skipped(SkipReason::NotReady)),
    }
}

/// The run of card `key` on `profile`: its identity and generation.
fn live_fence<Io>(core: &OperatorSession<Io>, profile: &str, key: &str) -> Option<(String, u64)> {
    let play = core.play()?;
    let identity = play.script_source_identity(profile)?;
    if identity != key {
        return None;
    }
    Some((identity, play.script_runtime_generation(profile)?))
}

/// Wall members plus any other slot the session holds.
fn slot_names<Io>(core: &OperatorSession<Io>) -> Vec<String> {
    let mut names: Vec<String> = core.members().to_vec();
    for name in core.slots().keys() {
        if !names.iter().any(|n| n == name) {
            names.push(name.clone());
        }
    }
    names
}

fn script_active<Io>(core: &OperatorSession<Io>, name: &str) -> bool {
    matches!(
        core.play()
            .map_or(script::RunState::Idle, |p| p.script_state(name)),
        script::RunState::Starting
            | script::RunState::Running
            | script::RunState::Paused
            | script::RunState::Stopping
    )
}

fn wall_member<Io>(core: &OperatorSession<Io>, name: &str) -> bool {
    core.members().iter().any(|member| member == name)
}

fn arm_flags<Io>(core: &OperatorSession<Io>, name: &str) -> (bool, bool) {
    match core.play().and_then(|play| play.arm(name)) {
        Some(arm) => (true, arm.login_latched() || arm.wants_logout()),
        None => (false, false),
    }
}

/// The Browse selection a saved assignment names.
pub(crate) fn sel_from_assignment(asg: &ScriptAssignment) -> Option<script::ScriptSel> {
    if asg.source_kind == "compiled" {
        return script::compiled_id(&asg.identity).map(script::ScriptSel::Compiled);
    }
    let source = script::parse_source_kind(&asg.source_kind)?;
    Some(script::ScriptSel::Loaded(source, asg.identity.clone()))
}

/// Whether [`sel_from_assignment`] would name a card for `asg`, decided
/// by borrowing only (the selection `sel_from_assignment` builds clones
/// the stored identity, which a per-frame enable check must not do).
pub(crate) fn assignment_has_sel(asg: &ScriptAssignment) -> bool {
    if asg.source_kind == "compiled" {
        return script::compiled_id(&asg.identity).is_some();
    }
    script::parse_source_kind(&asg.source_kind).is_some()
}

/// Make `dst` a copy of `src`, reusing `dst`'s buffer and elements.
pub(crate) fn refill<T: Clone>(dst: &mut Vec<T>, src: &[T]) {
    dst.truncate(src.len());
    let kept = dst.len();
    dst.clone_from_slice(&src[..kept]);
    dst.extend_from_slice(&src[kept..]);
}

/// The library lookup for a card: a File card by path, others by name.
fn lookup_name(source: script::ScriptSource, name: &str, path: &Path) -> String {
    match source {
        script::ScriptSource::File => path.to_string_lossy().into_owned(),
        _ => name.to_string(),
    }
}

#[cfg(test)]
mod tests;
