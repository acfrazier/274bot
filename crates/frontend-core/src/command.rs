//! The operator command vocabulary and its pending/terminal reports (F0
//! item 5: contract D7, §26 §1 item 5 and §3.2–§3.3).
//!
//! Frozen here, defined over F0-1/2's identities and terminals and not yet
//! wired into [`crate::OperatorSession`] (T3.3 activates it):
//!
//! - [`OperatorCommand`]: one row per operator command the panel, the TUI or
//!   a headless driver can issue today, carrying only scalar parameters. A
//!   command's subject is a stable [`ProfileIdentity`], never a row index or
//!   a display name; owned inputs (a profile draft, a settings bag, a map
//!   destination) are core-owned staged models the command commits.
//! - Each row's frozen columns: the [`Intent`] it posts to the slot's
//!   `IntentMailbox` ([`OperatorCommand::intent`]), the process capabilities
//!   it needs ([`OperatorCommand::capabilities`]), how a pending member
//!   settles ([`OperatorCommand::settles`]), whether its completion waits on
//!   real retirement ([`OperatorCommand::retirement_dependent`]), which older
//!   pending commands it supersedes ([`OperatorCommand::supersedes`]) and the
//!   current report it replaces ([`OperatorCommand::action_kind`]).
//! - Command identity: a host-namespace [`RequestId`] minted by the
//!   composition's one `RequestIdMinter`, so a command, its member terminals
//!   and an [`UnavailableTerminal`] correlate by the same id.
//! - [`CommandBook`]: every member of an accepted command is Pending, then
//!   settles exactly once into a `Settlement<Completion, CommandFailure>`.
//!   A command refused by a latched process condition is a Failed terminal at
//!   admission, never Pending. Mailbox rows settle on the slot owner's
//!   `SessionEventOut::IntentApplied` for their receipt's generation or a
//!   later one; an older acknowledgement never settles a newer post.
//!
//! UI strings are presentation: reports carry typed reasons; free text from
//! a lower layer survives only as [`MemberReport::detail`].

use std::collections::VecDeque;

use api::identity::{Cancellation, Event, RequestId, RequestNamespace, Scope, Settlement, SlotKey};
use api::lifecycle::{
    Intent, IntentGeneration, IntentSnapshot, MemoryMode, ParkReason, PostReceipt, SessionEventOut,
    WorldChoice,
};
use api::process_condition::{
    ConditionId, ProcessCapability, ProcessConditions, ProcessServiceCondition, UnavailableTerminal,
};

use crate::map_bake::MapBakeChoice;
use crate::nav_prefs::NavPreference;
use crate::operations::ActionKind;
use crate::selection::ProfileIdentity;
use crate::walk_permissions::DangerLevel;

/// Settled reports kept for inspection; a report with a pending member is
/// never evicted (same bound and rule as today's `OperationBook`,
/// `operations.rs:9-14`).
pub const SETTLED_REPORT_CAP: usize = 64;

/// One operator command. Each row documents the current code it
/// characterizes. Rows with a parameter take only scalar values; the
/// subject profile is supplied per member to [`CommandBook::admit`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperatorCommand {
    // --- Fleet and session lifecycle ---
    /// Add a profile to the fleet and spawn its slot
    /// (`OperatorSession::{load, load_all, open_slot, ensure_slot}`,
    /// `session.rs:678-880`). The spawned mailbox starts from the saved
    /// auto-login preference; no intent row is posted.
    Load,
    /// Remove a member: clean logout when connected, then stop
    /// (`OperatorSession::remove`, `session.rs:1194-1271`).
    Remove,
    /// Explicit Log in (`OperatorSession::{login, login_all, login_members}`,
    /// `session.rs:881-973`; `marked::login_marked`).
    Login,
    /// Clean Log out, latched until an explicit Log in
    /// (`OperatorSession::{logout, logout_all, logout_members}`,
    /// `session.rs:978-1027`; `marked::logout_marked`).
    Logout,
    /// Relog now: clean turnover then a queued login, no logout latch
    /// (`OperatorSession::request_memory_relog`, `session.rs:1102-1173`).
    RelogNow,
    /// Switch to a world: the contract row composed today of Logout, a world
    /// save and Log in (B03 `03-lifecycle.md:50`); no single current command.
    SwitchWorld(u16),
    /// Persist the auto-login preference (`OperatorSession::set_auto_login`,
    /// `session.rs:2053-2064`).
    SetAutoLogin(bool),
    /// Persist the world preference for the next handshake (the profile
    /// form's world field through `save_profile`, mirrored by
    /// `ArmMirror::Remember`, `session.rs:152-155`).
    SetWorld(WorldChoice),
    /// Persist the memory mode for the next fresh handshake
    /// (`OperatorSession::set_memory_mode`, `session.rs:1075-1095`).
    SetMemoryMode(MemoryMode),
    /// Persist the random-event guardian fields staged in the profile form
    /// (`OperatorSession::set_random_settings`, `session.rs:2068-2093`).
    SetGuardian,
    /// Quit the application (panel window close, TUI `q`/`Ctrl-Q`).
    Quit,

    // --- Vault ---
    /// Unlock the vault with the staged password (panel `Unlock vault`,
    /// `--vault-pass-stdin`; then `OperatorSession::start`, `session.rs:421`).
    UnlockVault,
    /// Create a new vault (panel `Create vault`).
    CreateVault,
    /// Reset the vault after confirmation (panel `Reset vault`).
    ResetVault,

    // --- Script runs ---
    /// Start the assigned script (`OperatorSession::start_script`,
    /// `session.rs:1696-1725`; `Scripts::{start_profile, start_selected,
    /// start_all}`; `selection::start_marked`).
    StartScript,
    /// Pause the running script (`OperatorSession::toggle_pause`,
    /// `session.rs:1728-1750`; native `NativeCommand::Pause`).
    PauseScript,
    /// Resume a paused script (same paths; native `NativeCommand::Resume`).
    ResumeScript,
    /// Stop the script (`OperatorSession::{stop_script, stop_scripts}`,
    /// `session.rs:1754-1796`; `Scripts::stop_all`; `selection::stop_marked`;
    /// native `NativeCommand::Stop`).
    StopScript,
    /// Drop a queued Start-all/marked Start before it is admitted
    /// (`Scripts::{cancel_queued, cancel_queued_as}`, `scripts/mod.rs:973-998`).
    CancelQueuedStart,
    /// Assign the staged script selection (`Scripts::persist_assignment`,
    /// `scripts/mod.rs:638`; `marked::assign_marked`).
    AssignScript,
    /// Assign then restart running members (`marked::assign_and_restart_marked`,
    /// `marked.rs:197`).
    AssignAndRestart,
    /// Save the staged script parameters (`Scripts::{set_profile_setting,
    /// profile_save_live, set_compiled_setting, set_compiled_overrides}`).
    SaveScriptSettings,
    /// Apply the focused member's parameters to the marked same-card members
    /// ("Apply to all": `Scripts::{prepare_settings_sync,
    /// prepare_compiled_settings_sync, apply_settings_sync}`,
    /// `scripts/sync.rs:783-1070`; `marked::prepare_apply_settings_marked`).
    ApplySettingsToMarked,
    /// Withdraw a prepared Apply-to-all (`Scripts::cancel_settings_sync`,
    /// `scripts/sync.rs:919`).
    CancelSettingsSync,
    /// Restart runs whose saved settings need a restart
    /// (`Scripts::restart_pending_settings`, `scripts/mod.rs:364`).
    RestartForSettings,
    /// Keep the restart prompt for later (`Scripts::dismiss_restart_prompt`,
    /// `scripts/mod.rs:346`).
    DeferSettingsRestart,
    /// Load a JS script file chosen in the browser (panel `Load`, TUI `f`).
    LoadScriptFile,
    /// Begin a script reload (`Scripts::begin_reload`, `scripts/reload.rs:398`).
    ReloadScripts,
    /// Confirm a reload that warned (`scripts/reload.rs:190`).
    ConfirmReload,
    /// Cancel a pending reload (`Scripts::cancel_reload`, `scripts/reload.rs:460`).
    CancelReload,
    /// Import or refresh the script catalogue (`Scripts::{register_catalog,
    /// fill_catalog_once, begin_refresh_catalog}`).
    ImportCatalog,
    /// Answer a running script's paint/modal prompt (panel paint strip, TUI
    /// `1-9`/`Enter`; `Play::script_paint_*`).
    ScriptPaintInput,

    // --- Profiles and loadouts ---
    /// Save the profile form (`OperatorSession::save_profile`,
    /// `session.rs:1803`; `ProfileFormSave::submitted`, `profile_form.rs:118`).
    SaveProfile,
    /// Create or rename a profile from the form
    /// (`OperatorSession::{create_profile, rename_profile}`, `session.rs:1844-1921`).
    CreateProfile,
    /// Delete a profile (`OperatorSession::vault_remove`, `session.rs:1950`).
    DeleteProfile,
    /// Discard the dirty form when switching its target (panel `Discard`).
    DiscardProfileEdits,
    /// Save the staged loadout (panel/TUI Loadouts `Save`).
    SaveLoadout,
    /// Delete a loadout (panel/TUI Loadouts `Delete`).
    DeleteLoadout,

    // --- Selection and marks ---
    /// Select the focused member (`OperatorSession::select`, `session.rs:641`).
    Select,
    /// Mark or unmark a member (`MarkedSelection::{set, toggle}`, `selection.rs:77-92`).
    SetMark(bool),
    /// Mark every shown member (`MarkedSelection::mark_all`).
    MarkAll,
    /// Clear the marks (`MarkedSelection::clear`).
    ClearMarks,

    // --- Movement ---
    /// Walk the focused member to the staged map destination (panel
    /// `WalkTo`/`Go`, TUI map `Enter`; `Play::map_walk`).
    WalkTo,
    /// Walk the marked members (`group_walk::walk_marked`, `group_walk.rs:58`).
    GroupWalk,
    /// Local debug teleport (panel `Teleport`, TUI `t`; `Play::map_teleport`).
    Teleport,
    /// Send a confirmed debug command (panel `Send##debug-send`;
    /// `Play::cheat`).
    DebugCommand,
    /// Game input for the focused member: captured keys (panel `capture
    /// input`, `input_capture.rs`), a manual WASD step (TUI `w` modal;
    /// `Play::queue_wire(WireCmd::Walk)`) or a dialogue answer (TUI chat;
    /// `WireCmd::Continue/Answer`).
    GameInput,

    // --- Draw policy and fleet layout (D7 `DrawPolicy`) ---
    /// MultiBox on or off (panel `MultiBox`): whether the fleet adopts
    /// every running slot (`OperatorSession::seed_running`, `session.rs:1177`)
    /// and Load all focuses like MultiBox-on (`pick_focus`, `session.rs:44`).
    SetMultiBox(bool),
    /// The focused member's raster mode (panel `Off`/`Gpu`/`Cpu`,
    /// `Session::request_focused_raster`).
    SetRaster(RasterMode),
    /// One draw-policy switch (panel `game renderer`, `capture input`,
    /// `sidecar 50 fps`, `focused 50 fps`, `full rate (this run)`).
    SetDraw(DrawSwitch, bool),

    // --- Preferences and notices ---
    /// Set one shared navigation preference (`nav_prefs.rs:5-25`).
    SetNavPreference(NavPreference, bool),
    /// Set the global danger-routing level
    /// (`WalkGlobalsView::persist_changed_at`, `walk_permissions.rs:183`).
    SetDangerLevel(DangerLevel),
    /// Turn the session log file on or off (`log_file::apply_session_log`).
    SetSessionLog(bool),
    /// Persist the map-bake choice (`map_bake::persist_map_bake_choice`).
    SetMapBakeChoice(MapBakeChoice),
    /// Accept the map-bake prompt (`MapBakeGate::accept`, `map_bake.rs:142`).
    BakeMapNow,
    /// Decline the map-bake prompt (`MapBakeGate::decline`, `map_bake.rs:149`).
    DeclineMapBake,
    /// Persist the staged Quester paths settings
    /// (`QuesterPathsController::apply_changed_at`, `quester_paths.rs:185`).
    SetQuesterPaths,
    /// Reload Quester paths (`QuesterPathsController::request_reload`,
    /// `quester_paths.rs:218`).
    ReloadQuesterPaths,
    /// Render every member or only the selected one (`render all` /
    /// `only render selected`; the capacity label,
    /// `OperatorSession::set_capacity_render`, `session.rs:564`).
    SetRenderAll(bool),
    /// Acknowledge a one-time notice (background bots, walk-permission and
    /// danger-routing notices).
    AcknowledgeNotice,
    /// Save the log view to a file (panel log `Save`, TUI log `w`).
    SaveLog,
}

/// A raster mode for one member's game image.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RasterMode {
    /// No game image.
    Off,
    /// GPU raster.
    Gpu,
    /// CPU raster.
    Cpu,
}

/// A boolean draw-policy switch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DrawSwitch {
    /// The focused member's game renderer.
    GameRenderer,
    /// Route keyboard input to the focused member's game.
    CaptureInput,
    /// Wall (sidecar) members draw at full rate.
    SidecarFullRate,
    /// The focused member draws at full rate.
    FocusedFullRate,
    /// Full rate for this run only.
    LiveFullRate,
}

/// How a pending member of a command settles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Settles {
    /// Decided when the command is dispatched: the member never stays
    /// Pending past dispatch.
    AtDispatch,
    /// By the slot owner's `SessionEventOut::IntentApplied` for the
    /// member's receipt generation or a later one.
    IntentApplied,
    /// By its owner's report: the durable writer, the script-run owner, the
    /// preparation/route job, the slot retirement or the vault.
    Owner,
}

const NO_CAPABILITY: &[ProcessCapability] = &[];
const SESSION_LOGIN: &[ProcessCapability] = &[
    ProcessCapability::NewConnection,
    ProcessCapability::Login,
    ProcessCapability::NameResolution,
];
const NETWORK_EFFECT: &[ProcessCapability] = &[ProcessCapability::NetworkEffect];
const BACKGROUND_WORK: &[ProcessCapability] = &[ProcessCapability::BackgroundWork];
const ROUTED_MOVEMENT: &[ProcessCapability] = &[
    ProcessCapability::BackgroundWork,
    ProcessCapability::NetworkEffect,
];

impl OperatorCommand {
    /// Every row once (parameterized rows with a representative value), for
    /// exhaustive checks.
    pub const ALL: [OperatorCommand; 61] = [
        Self::Load,
        Self::Remove,
        Self::Login,
        Self::Logout,
        Self::RelogNow,
        Self::SwitchWorld(1),
        Self::SetAutoLogin(true),
        Self::SetWorld(WorldChoice::Automatic),
        Self::SetMemoryMode(MemoryMode::Low),
        Self::SetGuardian,
        Self::Quit,
        Self::UnlockVault,
        Self::CreateVault,
        Self::ResetVault,
        Self::StartScript,
        Self::PauseScript,
        Self::ResumeScript,
        Self::StopScript,
        Self::CancelQueuedStart,
        Self::AssignScript,
        Self::AssignAndRestart,
        Self::SaveScriptSettings,
        Self::ApplySettingsToMarked,
        Self::CancelSettingsSync,
        Self::RestartForSettings,
        Self::DeferSettingsRestart,
        Self::LoadScriptFile,
        Self::ReloadScripts,
        Self::ConfirmReload,
        Self::CancelReload,
        Self::ImportCatalog,
        Self::ScriptPaintInput,
        Self::SaveProfile,
        Self::CreateProfile,
        Self::DeleteProfile,
        Self::DiscardProfileEdits,
        Self::SaveLoadout,
        Self::DeleteLoadout,
        Self::Select,
        Self::SetMark(true),
        Self::MarkAll,
        Self::ClearMarks,
        Self::WalkTo,
        Self::GroupWalk,
        Self::Teleport,
        Self::DebugCommand,
        Self::GameInput,
        Self::SetMultiBox(true),
        Self::SetRaster(RasterMode::Gpu),
        Self::SetDraw(DrawSwitch::GameRenderer, true),
        Self::SetNavPreference(NavPreference::AllowTeleports, true),
        Self::SetDangerLevel(DangerLevel::Never),
        Self::SetSessionLog(true),
        Self::SetMapBakeChoice(MapBakeChoice::Ask),
        Self::BakeMapNow,
        Self::DeclineMapBake,
        Self::SetQuesterPaths,
        Self::ReloadQuesterPaths,
        Self::SetRenderAll(true),
        Self::AcknowledgeNotice,
        Self::SaveLog,
    ];

    /// The `IntentMailbox` row this command posts to each member's slot, if
    /// it changes operator intent.
    pub const fn intent(self) -> Option<Intent> {
        match self {
            Self::Remove => Some(Intent::RemoveSlot),
            Self::Login => Some(Intent::Login),
            Self::Logout => Some(Intent::Logout),
            Self::RelogNow => Some(Intent::RelogNow),
            Self::SwitchWorld(world) => Some(Intent::SwitchWorld(world)),
            Self::SetAutoLogin(on) => Some(Intent::AutoLogin(on)),
            Self::SetWorld(world) => Some(Intent::WorldPreference(world)),
            Self::SetMemoryMode(mode) => Some(Intent::MemoryMode(mode)),
            Self::Quit => Some(Intent::ProcessShutdown),
            // Assign & restart and the settings restart are compositions
            // (Stop, then Start) driven by their owner; they post no single row.
            Self::StartScript => Some(Intent::StartScript),
            Self::PauseScript => Some(Intent::PauseScript),
            Self::ResumeScript => Some(Intent::ResumeScript),
            Self::StopScript => Some(Intent::StopScript),
            _ => None,
        }
    }

    /// The process capabilities the command's effect needs (§26 §3.1). A
    /// latched condition affecting one of them refuses a new command unless
    /// it is [`Self::retirement_dependent`].
    pub const fn capabilities(self) -> &'static [ProcessCapability] {
        match self {
            Self::Login | Self::RelogNow | Self::SwitchWorld(_) => SESSION_LOGIN,
            Self::Logout | Self::Remove | Self::Teleport | Self::DebugCommand => NETWORK_EFFECT,
            Self::ScriptPaintInput | Self::GameInput => NETWORK_EFFECT,
            Self::StartScript
            | Self::AssignAndRestart
            | Self::RestartForSettings
            | Self::SaveScriptSettings
            | Self::ApplySettingsToMarked
            | Self::LoadScriptFile
            | Self::ReloadScripts
            | Self::ConfirmReload
            | Self::ImportCatalog
            | Self::BakeMapNow
            | Self::ReloadQuesterPaths => BACKGROUND_WORK,
            Self::WalkTo | Self::GroupWalk => ROUTED_MOVEMENT,
            _ => NO_CAPABILITY,
        }
    }

    /// Whether completion waits on a real retirement predicate (no session,
    /// no run, no slot). Such a command is never refused by a process
    /// condition and never turned into a failure by one: Logout never
    /// becomes a fresh login and Stop/Remove never a retry (§26 §3.2 row 3).
    /// While a condition blocks it, it stays Pending and its status exposes
    /// the condition ([`CommandBook::blocked_by`]).
    pub const fn retirement_dependent(self) -> bool {
        matches!(
            self,
            Self::Logout | Self::Remove | Self::StopScript | Self::Quit
        )
    }

    /// How a pending member settles.
    pub const fn settles(self) -> Settles {
        match self {
            Self::Login | Self::Logout | Self::RelogNow | Self::SwitchWorld(_) => {
                Settles::IntentApplied
            }
            Self::Select
            | Self::SetMark(_)
            | Self::MarkAll
            | Self::ClearMarks
            | Self::DiscardProfileEdits
            | Self::CancelQueuedStart
            | Self::CancelSettingsSync
            | Self::DeferSettingsRestart
            | Self::CancelReload
            | Self::DeclineMapBake
            | Self::SetRenderAll(_)
            | Self::SetMultiBox(_)
            | Self::SetRaster(_)
            | Self::SetDraw(..)
            | Self::GameInput
            | Self::AcknowledgeNotice => Settles::AtDispatch,
            _ => Settles::Owner,
        }
    }

    /// Whether posting `self` for a member supersedes an older pending
    /// `older` of the same member: the older intent can no longer be what
    /// the slot acknowledges, so it settles `Cancelled(Superseded)`. Today's
    /// rule (`session.rs:958-960,1012-1019,1147-1148,1214`): Log in, Log out
    /// and Relog now cancel the member's pending Log in and Log out (Relog
    /// now reports as a Log out op); Remove cancels the pending Log in.
    /// Switch world joins the login-wanting turnovers.
    pub const fn supersedes(self, older: OperatorCommand) -> bool {
        let older_session = matches!(
            older,
            Self::Login | Self::Logout | Self::RelogNow | Self::SwitchWorld(_)
        );
        match self {
            Self::Login | Self::Logout | Self::RelogNow | Self::SwitchWorld(_) => older_session,
            Self::Remove | Self::Quit => {
                matches!(older, Self::Login | Self::RelogNow | Self::SwitchWorld(_))
            }
            _ => false,
        }
    }

    /// The current operation report kind this row replaces
    /// (`operations.rs:20-34`), if it has one today.
    pub const fn action_kind(self) -> Option<ActionKind> {
        match self {
            Self::Select => Some(ActionKind::Select),
            Self::Load => Some(ActionKind::Load),
            Self::Remove => Some(ActionKind::Remove),
            Self::Login => Some(ActionKind::Login),
            Self::Logout | Self::RelogNow => Some(ActionKind::Logout),
            Self::StartScript => Some(ActionKind::ScriptStart),
            Self::PauseScript => Some(ActionKind::ScriptPause),
            Self::ResumeScript => Some(ActionKind::ScriptResume),
            Self::StopScript => Some(ActionKind::ScriptStop),
            Self::SaveProfile
            | Self::CreateProfile
            | Self::SetAutoLogin(_)
            | Self::SetWorld(_)
            | Self::SetMemoryMode(_)
            | Self::SetGuardian
            | Self::AssignScript
            | Self::SaveScriptSettings => Some(ActionKind::SaveProfile),
            Self::DeleteProfile => Some(ActionKind::DeleteProfile),
            Self::ApplySettingsToMarked => Some(ActionKind::SyncSettings),
            _ => None,
        }
    }

    /// Whether the command wants the member logged in when it completes.
    const fn wants_login(self) -> bool {
        matches!(self, Self::Login | Self::RelogNow | Self::SwitchWorld(_))
    }
}

/// A successful terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Completion {
    /// The requested effect holds.
    Done,
    /// Nothing was done because the member did not need it or could not
    /// take part; not a failure.
    Skipped(SkipReason),
}

/// Why a member was skipped (today's `Outcome::Skipped`/`BulkOutcome`
/// strings, typed).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SkipReason {
    /// No script on the member (`session.rs:1676,1791`; `selection.rs:254`).
    NoScript,
    /// No running script to pause (`session.rs:1667`).
    NoRunningScript,
    /// A script is already starting or running (`selection.rs:197`).
    AlreadyActive,
    /// The script is already stopping (`selection.rs:249`).
    AlreadyStopping,
    /// Already logged in (`marked.rs:282`).
    AlreadyLoggedIn,
    /// A login is already in progress (`marked.rs:292`).
    AlreadyLoggingIn,
    /// Not logged in (`marked.rs:330`; `bulk.rs` walk rows).
    NotLoggedIn,
    /// Not loaded in the fleet (`marked.rs:233`).
    NotLoaded,
    /// Not ready (`selection.rs:373`).
    NotReady,
    /// The profile has no durable row or is unknown (`marked.rs:17`,
    /// `selection.rs:237`, `group_walk.rs:73`).
    ProfileUnavailable,
    /// The profile's first save is still being written (`session.rs:844`).
    ProfileSaving,
    /// A running script blocks a plain assignment (`marked.rs:154`).
    ScriptActive,
    /// The member was excluded from a group walk (`group_walk.rs:130`).
    WalkExcluded,
}

/// Why a member failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandFailure {
    /// A latched process condition refused the new command (§26 §3.2 row
    /// 1): it was never Pending.
    Refused {
        /// The latched condition.
        condition: ConditionId,
        /// The capability the command needed.
        capability: ProcessCapability,
    },
    /// The outage made an accepted pending command impossible (§26 §3.2
    /// row 2): the one logical Failed/Unavailable terminal.
    Unavailable(UnavailableTerminal),
    /// Refused by the owner at dispatch.
    Rejected(Rejection),
    /// The owner reported a failure.
    Failed(FailureKind),
}

impl CommandFailure {
    /// The process condition this failure references, if any.
    pub const fn condition(&self) -> Option<ConditionId> {
        match self {
            Self::Refused { condition, .. } => Some(*condition),
            Self::Unavailable(terminal) => Some(terminal.condition()),
            Self::Rejected(_) | Self::Failed(_) => None,
        }
    }

    /// The exact restart-required operator text (§26 §3.1) for a failure
    /// caused by a process condition.
    pub const fn operator_text(&self) -> Option<&'static str> {
        match self.condition() {
            Some(condition) => Some(condition.code().operator_text()),
            None => None,
        }
    }
}

/// A dispatch-time refusal by the owner (today's `Outcome::Failed` strings
/// produced before anything was accepted).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Rejection {
    /// No host session: the vault is locked or `Play` is gone
    /// (`session.rs:963,1744,1763`).
    NoHost,
    /// The profile does not exist (`session.rs:968`).
    UnknownProfile,
    /// The profile's first save is still being written (`session.rs:1203`).
    ProfileSaving,
    /// The script start was refused (`script::StartLoadError`,
    /// `session.rs:1701-1705`).
    StartRefused,
}

/// An owner-reported failure (today's `Outcome::Failed` strings produced
/// after acceptance).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FailureKind {
    /// A terminal login/startup error (`session.rs:1635-1636`).
    LoginFailed,
    /// The slot worker ended (`session.rs:1637-1640`).
    WorkerEnded,
    /// The world preference was refused; the login is held until the
    /// operator changes something (`ParkReason::WorldError`).
    WorldRejected,
    /// The script failed to start (`session.rs:1607-1608`).
    StartFailed,
    /// A durable write failed (`session.rs:2296`; `native_settings.rs:298,409,519`).
    WriteFailed,
    /// Saved but the live run rejected the settings (`scripts/sync.rs:760`).
    LiveRejected,
    /// A preparation, validation or route job failed
    /// (`scripts/reload.rs:162,249`; `group_walk.rs:96,132-137`).
    JobFailed,
}

/// What a pending member waits for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Pending {
    /// Accepted; its owner has not settled it.
    Accepted,
    /// Posted to the slot's mailbox; waits for `IntentApplied` of
    /// `generation` or later from that slot (or its owner's report).
    Posted {
        /// The slot whose mailbox received the post.
        slot: SlotKey,
        /// The receipt's generation.
        generation: IntentGeneration,
    },
}

/// The outcome of [`CommandBook::admit`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admission {
    /// The member is Pending (or, for an [`Settles::AtDispatch`] row,
    /// waits for its dispatch result).
    Accepted,
    /// A latched condition refused it; the member's terminal is already
    /// `Failed(CommandFailure::Refused)`.
    Refused {
        /// The latched condition.
        condition: ConditionId,
        /// The capability the command needed.
        capability: ProcessCapability,
    },
}

/// A book operation that does not match the protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CommandError {
    /// Command identities are host-namespace ids.
    #[error("command identity {0:?} is not a host request id")]
    IsolateRequest(RequestId),
    /// The identity was already used.
    #[error("command identity {0:?} is already in use")]
    DuplicateRequest(RequestId),
    /// No such command (or it was evicted after settling).
    #[error("unknown command {0:?}")]
    UnknownRequest(RequestId),
    /// The member was already admitted to this command.
    #[error("{profile:?} is already a member of command {request:?}")]
    DuplicateMember {
        /// The command.
        request: RequestId,
        /// The member.
        profile: ProfileIdentity,
    },
    /// No such member of this command.
    #[error("{profile:?} is not a member of command {request:?}")]
    UnknownMember {
        /// The command.
        request: RequestId,
        /// The member.
        profile: ProfileIdentity,
    },
    /// The member already has its one terminal.
    #[error("{profile:?} of command {request:?} is already settled")]
    AlreadySettled {
        /// The command.
        request: RequestId,
        /// The member.
        profile: ProfileIdentity,
    },
    /// Only a mailbox row's member can record a post.
    #[error("command {0:?} posts no intent")]
    NoIntentRow(RequestId),
}

#[derive(Debug)]
enum MemberState {
    Pending(Pending),
    Settled(Settlement<Completion, CommandFailure>),
}

/// One targeted member of a command.
#[derive(Debug)]
pub struct MemberReport {
    profile: ProfileIdentity,
    scope: Scope,
    state: MemberState,
    detail: Option<Box<str>>,
}

impl MemberReport {
    /// The member's stable identity.
    pub fn profile(&self) -> ProfileIdentity {
        self.profile
    }

    /// The scope the member's terminal is reported in.
    pub fn scope(&self) -> Scope {
        self.scope
    }

    /// What the member waits for, while Pending.
    pub fn pending(&self) -> Option<Pending> {
        match self.state {
            MemberState::Pending(pending) => Some(pending),
            MemberState::Settled(_) => None,
        }
    }

    /// The member's one terminal, once settled.
    pub fn terminal(&self) -> Option<&Settlement<Completion, CommandFailure>> {
        match &self.state {
            MemberState::Pending(_) => None,
            MemberState::Settled(terminal) => Some(terminal),
        }
    }

    /// Presentation-only text a lower layer attached to the terminal (a
    /// server login message, a write error). Never drives behaviour.
    pub fn detail(&self) -> Option<&str> {
        self.detail.as_deref()
    }
}

/// One command's report: Pending members, then one terminal each.
#[derive(Debug)]
pub struct CommandReport {
    request: RequestId,
    command: OperatorCommand,
    members: Vec<MemberReport>,
}

impl CommandReport {
    /// The command identity.
    pub fn request(&self) -> RequestId {
        self.request
    }

    /// The command: the operation a report names.
    pub fn command(&self) -> OperatorCommand {
        self.command
    }

    /// Every member, in admission order.
    pub fn members(&self) -> &[MemberReport] {
        &self.members
    }

    /// One member.
    pub fn member(&self, profile: ProfileIdentity) -> Option<&MemberReport> {
        self.members.iter().find(|m| m.profile == profile)
    }

    /// Whether every member has its terminal.
    pub fn is_settled(&self) -> bool {
        self.members.iter().all(|m| m.pending().is_none())
    }
}

/// The bounded command history and its settlement protocol.
#[derive(Debug, Default)]
pub struct CommandBook {
    reports: VecDeque<CommandReport>,
}

impl CommandBook {
    /// An empty book.
    pub fn new() -> Self {
        Self::default()
    }

    /// Open a command under `request`, minted by the composition's host
    /// `RequestIdMinter`. Evicts the oldest settled report beyond
    /// [`SETTLED_REPORT_CAP`]; pending reports are never evicted.
    pub fn open(
        &mut self,
        request: RequestId,
        command: OperatorCommand,
    ) -> Result<(), CommandError> {
        if request.namespace() != RequestNamespace::Host {
            return Err(CommandError::IsolateRequest(request));
        }
        if self.report(request).is_some() {
            return Err(CommandError::DuplicateRequest(request));
        }
        if self.reports.iter().filter(|r| r.is_settled()).count() >= SETTLED_REPORT_CAP {
            if let Some(index) = self.reports.iter().position(CommandReport::is_settled) {
                self.reports.remove(index);
            }
        }
        self.reports.push_back(CommandReport {
            request,
            command,
            members: Vec::new(),
        });
        Ok(())
    }

    /// The report for `request`.
    pub fn report(&self, request: RequestId) -> Option<&CommandReport> {
        self.reports.iter().find(|r| r.request == request)
    }

    /// Every retained report, oldest first.
    pub fn reports(&self) -> impl Iterator<Item = &CommandReport> {
        self.reports.iter()
    }

    /// Admit `profile` as a member in `scope`. A command needing a
    /// capability a latched condition affects is refused here (unless it is
    /// retirement-dependent): its member settles at once as
    /// `Failed(CommandFailure::Refused)`, keeping the command identity and
    /// scope, and is never Pending.
    pub fn admit(
        &mut self,
        request: RequestId,
        profile: ProfileIdentity,
        scope: Scope,
        conditions: &ProcessConditions,
    ) -> Result<Admission, CommandError> {
        let report = self.report_mut(request)?;
        if report.member(profile).is_some() {
            return Err(CommandError::DuplicateMember { request, profile });
        }
        let command = report.command;
        let blocked = if command.retirement_dependent() {
            None
        } else {
            first_block(command, conditions)
        };
        let (state, admission) = match blocked {
            Some((condition, capability)) => (
                MemberState::Settled(Settlement::Failed(CommandFailure::Refused {
                    condition,
                    capability,
                })),
                Admission::Refused {
                    condition,
                    capability,
                },
            ),
            None => (MemberState::Pending(Pending::Accepted), Admission::Accepted),
        };
        report.members.push(MemberReport {
            profile,
            scope,
            state,
            detail: None,
        });
        Ok(admission)
    }

    /// Record the receipt of posting the command's intent row to `slot` for
    /// `profile`, and supersede that member's older pending commands this
    /// one replaces ([`OperatorCommand::supersedes`]).
    pub fn posted(
        &mut self,
        request: RequestId,
        profile: ProfileIdentity,
        slot: SlotKey,
        receipt: PostReceipt,
    ) -> Result<(), CommandError> {
        let report = self.report_mut(request)?;
        let command = report.command;
        if command.intent().is_none() {
            return Err(CommandError::NoIntentRow(request));
        }
        let member = pending_member(report, request, profile)?;
        member.state = MemberState::Pending(Pending::Posted {
            slot,
            generation: receipt.generation(),
        });
        for older in self.reports.iter_mut().filter(|r| r.request != request) {
            if !command.supersedes(older.command) {
                continue;
            }
            for member in older.members.iter_mut().filter(|m| m.profile == profile) {
                if matches!(member.state, MemberState::Pending(_)) {
                    member.state =
                        MemberState::Settled(Settlement::Cancelled(Cancellation::Superseded));
                }
            }
        }
        Ok(())
    }

    /// Apply one slot-scoped session event. `IntentApplied { generation }`
    /// from slot S completes every pending [`Settles::IntentApplied`]
    /// member posted to S at `generation` or earlier; a member posted at a
    /// later generation is untouched (a stale acknowledgement never settles
    /// a newer post). Returns how many members settled.
    pub fn observe(&mut self, event: &Event<SessionEventOut>) -> usize {
        let SessionEventOut::IntentApplied { generation } = *event.payload() else {
            return 0;
        };
        let Some(slot) = event.scope().slot() else {
            return 0;
        };
        let mut settled = 0;
        for report in &mut self.reports {
            if report.command.settles() != Settles::IntentApplied {
                continue;
            }
            for member in &mut report.members {
                if let MemberState::Pending(Pending::Posted {
                    slot: posted,
                    generation: at,
                }) = member.state
                {
                    if posted == slot && at <= generation {
                        member.state =
                            MemberState::Settled(Settlement::Completed(Completion::Done));
                        settled += 1;
                    }
                }
            }
        }
        settled
    }

    /// Apply the slot's mailbox snapshot, read after its events were
    /// observed. A pending login-wanting member (Log in, Relog now, Switch
    /// world) whose want a later generation withdrew settles: a world error
    /// as `Failed(WorldRejected)`, a latch or withdrawal as
    /// `Cancelled(Superseded)` (today `arm.login_latched()` cancels the
    /// pending Log in, `session.rs:1641-1644`). Returns how many settled.
    pub fn observe_intent(&mut self, slot: SlotKey, snapshot: &IntentSnapshot) -> usize {
        if snapshot.login_wanted() {
            return 0;
        }
        let terminal = || match snapshot.park_reason() {
            Some(ParkReason::WorldError) => {
                Settlement::Failed(CommandFailure::Failed(FailureKind::WorldRejected))
            }
            _ => Settlement::Cancelled(Cancellation::Superseded),
        };
        let mut settled = 0;
        for report in self.reports.iter_mut().filter(|r| r.command.wants_login()) {
            for member in &mut report.members {
                if let MemberState::Pending(Pending::Posted {
                    slot: posted,
                    generation,
                }) = member.state
                {
                    if posted == slot && generation < snapshot.generation() {
                        member.state = MemberState::Settled(terminal());
                        settled += 1;
                    }
                }
            }
        }
        settled
    }

    /// Settle one member with its owner's terminal (a dispatch result, a
    /// durable receipt, a run-state report, a job terminal). Exactly once:
    /// a second terminal is refused and changes nothing.
    pub fn settle(
        &mut self,
        request: RequestId,
        profile: ProfileIdentity,
        terminal: Settlement<Completion, CommandFailure>,
        detail: Option<&str>,
    ) -> Result<(), CommandError> {
        let report = self.report_mut(request)?;
        let member = pending_member(report, request, profile)?;
        member.state = MemberState::Settled(terminal);
        member.detail = detail.map(Box::from);
        Ok(())
    }

    /// The slot's lifetime ended (removed or its worker reaped). Its pending
    /// members settle: a withdrawal whose goal now holds (Log out, Remove,
    /// Stop, Quit) completes; Pause/Resume have no script left
    /// (`session.rs:1663-1680`); anything else is `Cancelled(ScopeInvalidated)`
    /// (today a pending Log in of a gone slot cancels, `session.rs:1628-1629`).
    pub fn slot_retired(&mut self, slot: SlotKey) -> usize {
        let mut settled = 0;
        for report in &mut self.reports {
            let terminal = match report.command {
                command if command.retirement_dependent() => {
                    Settlement::Completed(Completion::Done)
                }
                OperatorCommand::PauseScript => {
                    Settlement::Completed(Completion::Skipped(SkipReason::NoRunningScript))
                }
                OperatorCommand::ResumeScript => {
                    Settlement::Completed(Completion::Skipped(SkipReason::NoScript))
                }
                _ => Settlement::Cancelled(Cancellation::ScopeInvalidated),
            };
            for member in &mut report.members {
                let MemberState::Pending(pending) = member.state else {
                    continue;
                };
                let on_slot = match pending {
                    Pending::Posted { slot: posted, .. } => posted == slot,
                    Pending::Accepted => member.scope.slot() == Some(slot),
                };
                if on_slot {
                    member.state = MemberState::Settled(terminal.clone());
                    settled += 1;
                }
            }
        }
        settled
    }

    /// Settle every accepted pending member the latched conditions make
    /// impossible with its one `Failed(Unavailable)` terminal, referencing
    /// the condition and keeping the command identity and scope (§26 §3.2
    /// row 2). Retirement-dependent members stay Pending (row 3). This
    /// settles result custody only; it acknowledges no retirement.
    pub fn fail_unavailable(&mut self, conditions: &ProcessConditions) -> usize {
        let mut settled = 0;
        for report in &mut self.reports {
            if report.command.retirement_dependent() {
                continue;
            }
            let Some((condition, _)) = first_block(report.command, conditions) else {
                continue;
            };
            let request = report.request;
            for member in &mut report.members {
                if matches!(member.state, MemberState::Pending(_)) {
                    member.state =
                        MemberState::Settled(Settlement::Failed(CommandFailure::Unavailable(
                            UnavailableTerminal::new(condition, request, member.scope),
                        )));
                    settled += 1;
                }
            }
        }
        settled
    }

    /// For a pending member, the latched condition that blocks its
    /// progress: what its status shows while a retirement-dependent command
    /// waits (§26 §3.2 row 3).
    pub fn blocked_by(
        &self,
        request: RequestId,
        profile: ProfileIdentity,
        conditions: &ProcessConditions,
    ) -> Option<ConditionId> {
        let report = self.report(request)?;
        report.member(profile)?.pending()?;
        first_block(report.command, conditions).map(|(condition, _)| condition)
    }

    fn report_mut(&mut self, request: RequestId) -> Result<&mut CommandReport, CommandError> {
        self.reports
            .iter_mut()
            .find(|r| r.request == request)
            .ok_or(CommandError::UnknownRequest(request))
    }
}

fn pending_member(
    report: &mut CommandReport,
    request: RequestId,
    profile: ProfileIdentity,
) -> Result<&mut MemberReport, CommandError> {
    let member = report
        .members
        .iter_mut()
        .find(|m| m.profile == profile)
        .ok_or(CommandError::UnknownMember { request, profile })?;
    match member.state {
        MemberState::Pending(_) => Ok(member),
        MemberState::Settled(_) => Err(CommandError::AlreadySettled { request, profile }),
    }
}

/// The first latched condition affecting one of `command`'s capabilities.
fn first_block(
    command: OperatorCommand,
    conditions: &ProcessConditions,
) -> Option<(ConditionId, ProcessCapability)> {
    command.capabilities().iter().find_map(|&capability| {
        conditions
            .blocks(capability)
            .map(|condition: &ProcessServiceCondition| (condition.id(), capability))
    })
}

#[cfg(test)]
#[path = "command_tests.rs"]
mod tests;
