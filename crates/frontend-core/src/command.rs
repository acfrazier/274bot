//! The operator command vocabulary and its pending/terminal reports (F0
//! item 5: contract D7, §26 §1 item 5 and §3.2–§3.3).
//!
//! Frozen here, defined over F0-1/2's identities and terminals and not yet
//! wired into [`crate::OperatorSession`] (T3.3 activates it):
//!
//! - [`OperatorCommand`]: one row per operator command or application
//!   decision the panel, the TUI or a headless driver can issue today,
//!   carrying only scalar parameters. A command's subject is a stable
//!   [`ProfileIdentity`] or the application's one shared state
//!   ([`Subject`]), never a row index or a display name; owned inputs (a
//!   profile draft, a settings bag, a filter text, a map destination) are
//!   core-owned staged models the command commits or changes.
//! - Each row's frozen columns: its subject kind ([`OperatorCommand::subject`]),
//!   the [`Intent`] it posts to the slot's `IntentMailbox`
//!   ([`OperatorCommand::intent`]), the process capabilities it needs at
//!   admission and at each later stage ([`OperatorCommand::capabilities`],
//!   [`OperatorCommand::needs`]), how a pending member settles
//!   ([`OperatorCommand::settles`]), whether its completion waits on real
//!   retirement ([`OperatorCommand::retirement_dependent`]), which older
//!   pending commands it supersedes ([`OperatorCommand::supersedes`]) and the
//!   current report it replaces ([`OperatorCommand::action_kind`]).
//! - Command identity: a host-namespace [`RequestId`] minted by the
//!   composition's one `RequestIdMinter`, so a command, its member terminals
//!   and an [`UnavailableTerminal`] correlate by the same id.
//! - [`CommandBook`]: a command opens with its whole target set; every
//!   accepted member is Pending, then settles exactly once into a
//!   `Settlement<Completion, CommandFailure>`. A member refused by a latched
//!   process condition is a Failed terminal at admission, never Pending. A
//!   mailbox row's member is bound once to its admitted slot; it settles on
//!   that slot owner's `SessionEventOut::IntentApplied` for its receipt's
//!   generation or a later one, and an older acknowledgement, or another
//!   slot's, never settles it. What a pending member still needs from the
//!   process follows its [`Stage`]: a login's resolver exemption is bound
//!   to the attempt it was resolved for ([`LoginAttempt`]) and lapses with
//!   it; a native settings save stops needing the background service once
//!   prepared. Settled history is bounded; pending work is never evicted.
//!
//! UI strings are presentation: reports carry typed reasons; free text from
//! a lower layer survives only as [`MemberReport::detail`].

use std::collections::VecDeque;

use api::identity::{Cancellation, Event, RequestId, RequestNamespace, Scope, Settlement, SlotKey};
use api::lifecycle::{
    Intent, IntentCommand, IntentGeneration, IntentSnapshot, MailboxError, MemoryMode, ParkReason,
    PostReceipt, SessionEventOut, WorldChoice,
};
use api::selected::RunKey;
use api::process_condition::{
    ConditionId, ProcessCapability, ProcessConditions, ProcessServiceCondition, UnavailableTerminal,
};

use crate::map_bake::MapBakeChoice;
use crate::nav_prefs::NavPreference;
use crate::operations::ActionKind;
use crate::rs2b0t_import::ConflictPolicy;
use crate::selection::ProfileIdentity;
use crate::walk_permissions::DangerLevel;

/// Settled reports kept for inspection, newest settled last; a report with
/// a pending member is never evicted (same bound and rule as today's
/// `OperationBook`, `operations.rs:9-14`).
pub const SETTLED_REPORT_CAP: usize = 64;

/// Evicted member lists kept for reuse.
const SPARE_MEMBER_LISTS: usize = 4;

/// One operator command. Each row documents the current code it
/// characterizes. Rows with a parameter take only scalar values; the
/// subjects are supplied per member to [`CommandBook::open`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperatorCommand {
    // --- Fleet and session lifecycle ---
    /// Add a profile to the fleet and spawn its slot
    /// (`OperatorSession::{load, load_all, open_slot, ensure_slot}`,
    /// `session.rs:678-880`). The spawned mailbox starts from the saved
    /// auto-login preference; no intent row is posted.
    Load,
    /// Remove a member: clean logout when connected, then stop
    /// (`OperatorSession::remove`, `session.rs:1194-1271`). Its post closes
    /// the slot's mailbox lifetime.
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
    /// Offline it is a plain login; a stronger row (Log in, Log out, Remove)
    /// supersedes its pending login half.
    RelogNow,
    /// Switch a logged-in bot to a public world: save its world, then a
    /// turnover that relogs it through the login queue
    /// (`session/world_switch.rs`; the fleet plan's world picker). Settled
    /// by the relog, not by the post.
    SwitchWorld(u16),
    /// Persist the auto-login preference (`OperatorSession::set_auto_login`,
    /// `session.rs:2053-2064`).
    SetAutoLogin(bool),
    /// Persist the world preference for the next handshake (the profile
    /// form's world field through `save_profile`, mirrored by
    /// `ArmMirror::remember`).
    SetWorld(WorldChoice),
    /// Persist the memory mode for the next fresh handshake
    /// (`OperatorSession::set_memory_mode`, `session.rs:1075-1095`).
    SetMemoryMode(MemoryMode),
    /// Persist the random-event guardian fields staged in the profile form
    /// (`OperatorSession::set_random_settings`, `session.rs:2068-2093`) or
    /// one hostile random's policy (`OperatorSession::set_random_event_policy`,
    /// `session.rs:2171-2191`).
    SetGuardian,
    /// Persist the Cure poison setting staged for the profile
    /// (`OperatorSession::set_survival_settings`): written through
    /// `save_profile` and mirrored to a running slot once durable.
    SetSurvival,
    /// Quit the application (panel window close, TUI `q`/`Ctrl-Q`). One
    /// member per loaded slot, each completed by its slot's retirement.
    Quit,

    // --- Vault ---
    /// Unlock the vault with the staged password (panel `Unlock vault`,
    /// `--vault-pass-stdin`; then `OperatorSession::start`, `session.rs:421`).
    UnlockVault,
    /// Create a new vault (panel `Create vault`).
    CreateVault,
    /// Reset the vault after confirmation (panel `Reset vault`).
    ResetVault,

    // --- Fleet view ---
    /// Apply the fleet filter text staged in the core-owned fleet model
    /// (panel `Filter` field, `fleet.rs:609-611`; TUI `/` filter editing and
    /// `Esc` clear, `input.rs:469-483,522-526`). The text is owned state both
    /// surfaces share, not presentation.
    SetFleetFilter,
    /// Order the fleet rows (panel `Sort:` drop-down, `fleet.rs:400-416`).
    SetFleetSort(FleetOrder),

    // --- Script runs ---
    /// Start the assigned script (`OperatorSession::start_script`,
    /// `session.rs:1696-1725`; `Scripts::{start_profile, start_selected,
    /// start_all}`; `selection::start_marked`).
    StartScript,
    /// Pause the running script (`OperatorSession::toggle_pause`,
    /// `session.rs:1728-1750`).
    PauseScript,
    /// Request a skip for the active Quester run at its next eligible tick.
    /// `target` fences the command against a stale native status view.
    SkipQuest { target: Option<RunKey> },
    /// Resume a paused script through the same operator row.
    ResumeScript,
    /// Stop the script (`OperatorSession::{stop_script, stop_scripts}`,
    /// `session.rs:1754-1796`; `Scripts::stop_all`; `selection::stop_marked`).
    StopScript,
    /// Drop a queued Start-all/marked Start before it is admitted
    /// (`Scripts::{cancel_queued, cancel_queued_as}`, `scripts/mod.rs:973-998`).
    CancelQueuedStart,
    /// Make a card the shared Browse selection displayed by both surfaces.
    /// This row is application-scoped and needs no profile. A focused-profile
    /// handler may then submit [`Self::SelectScriptCard`]; a loaded card may
    /// also submit [`Self::PrepareScriptCard`]. Each row has its own settlement.
    PickBrowseCard,
    /// Apply the staged Browse card to the member's pending profile selection.
    /// This neither persists the assignment ([`Self::AssignScript`]) nor starts
    /// a run ([`Self::StartScript`]).
    SelectScriptCard,
    /// Queue worker preparation (transpilation) of one staged loaded card. It
    /// has its own admission and terminal; unlike [`Self::TranspileAll`], it
    /// prepares only the selected card, not every unwarmed catalog card.
    PrepareScriptCard,
    /// Assign the staged script selection (`Scripts::persist_assignment`,
    /// `scripts/mod.rs:638`; `marked::assign_marked`).
    AssignScript,
    /// Assign then restart running members (`marked::assign_and_restart_marked`,
    /// `marked.rs:197`).
    AssignAndRestart,
    /// Save the member's staged parameters of one card. The kind of card
    /// decides the prerequisites ([`CardKind`]): a loaded JS card's value
    /// is coerced and staged at once and the durable writer takes the row
    /// (`Scripts::set_profile_setting`, `scripts/mod.rs:882-927`;
    /// `profile_save_live`), no background service involved; a native
    /// card's bag is first prepared by a background job
    /// (`set_compiled_setting`/`set_compiled_overrides`, `native.rs:124-221`;
    /// `queue_native_settings`, `native_settings.rs:246`) and the durable
    /// write follows ([`Stage::Prepared`], [`CommandBook::prepared`]).
    SaveScriptSettings(CardKind),
    /// Save a parameter of a loaded JS card to the shared legacy overrides
    /// when no profile is focused (panel `persist_profile_setting`,
    /// `app.rs:3339-3344`: `scripts.legacy.set_value` then `legacy.save`).
    /// A durable write of the application's shared settings file, settled
    /// by its writer; it needs no background service.
    SaveLegacyScriptSettings,
    /// A staged edit of the parameter editor's text buffer for one field
    /// (`Scripts::{parameter_text_mut, parameter_text_mut_for}` and the
    /// validation of that buffer, `parameter_edit.rs:715-818`): the
    /// persistent buffer both front ends bind, keyed by profile, card and
    /// field. It changes the staged text only; [`Self::SaveScriptSettings`]
    /// commits it (`commit_parameter_value`).
    EditParameterText,
    /// Cancel a field's staged text (`Scripts::{discard_parameter_text,
    /// discard_parameter_text_for}`, `parameter_edit.rs:820-840,960-968`);
    /// a write the buffer already queued still settles first, then the
    /// discard is honoured.
    DiscardParameterText,
    /// Apply the focused member's parameters to the marked same-card members
    /// ("Apply to all": `Scripts::{prepare_settings_sync,
    /// prepare_compiled_settings_sync, apply_settings_sync}`,
    /// `scripts/sync.rs:783-1070`; `marked::prepare_apply_settings_marked`).
    /// Same card for every member, so one [`CardKind`] decides the
    /// prerequisites as for [`Self::SaveScriptSettings`]: a JS copy is
    /// staged and written, a native copy is prepared per member first.
    ApplySettingsToMarked(CardKind),
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
    /// Queue preparation (transpile) of every loaded card that still needs
    /// it (panel `Transpile all`, `app.rs:2639-2640`;
    /// `queue_transpile_all`, `session_catalog.rs:145-160`).
    TranspileAll,
    /// Defer the first-run catalogue import durably ("Not now":
    /// `script::set_rs2b0t_import_deferred_at`; panel
    /// `defer_rs2b0t_catalog`, `session_catalog.rs:94-96`,
    /// `app.rs:2974-2975`; TUI `Esc`/`Not now`, `app.rs:1399,1415`,
    /// `bin.rs:1757-1760,3406`). A durable write, unlike closing the prompt.
    DeferCatalogImport,
    /// Answer a running script's paint/modal prompt (panel paint strip, TUI
    /// `1-9`/`Enter`; `Play::script_paint_*`).
    ScriptPaintInput,

    // --- Profiles and the profile form ---
    /// Save the profile form (`OperatorSession::save_profile`,
    /// `session.rs:1803`; `ProfileFormSave::submitted`, `profile_form.rs:118`).
    SaveProfile,
    /// Create or rename a profile from the form
    /// (`OperatorSession::{create_profile, rename_profile}`, `session.rs:1844-1921`).
    /// A rename is today's composition of this row; it has no row of its own.
    CreateProfile,
    /// Delete a profile (`OperatorSession::vault_remove`, `session.rs:1950`).
    /// While the form edits that profile with unsaved changes, the Discard /
    /// Keep editing decision is staged first (panel `EditLeave::DeleteTarget`).
    DeleteProfile,
    /// Point the profile form at the member's profile (panel
    /// `begin_edit_profile`, `session/chooser.rs:185-203`). With unsaved
    /// edits on another target, or the form's save still being written, the
    /// form keeps its target and draft and the Discard / Keep editing
    /// decision is staged instead (`Completed(Skipped(DecisionStaged))`).
    /// Focus never retargets the form.
    OpenProfileForm,
    /// Point the profile form at a blank new-profile row (the same path,
    /// target `""`; same staging rule).
    OpenNewProfileForm,
    /// A staged edit of the profile form's draft (panel `##cred-user`,
    /// `##cred-pass` and the settings sections, `app.rs:5422-5433,5586-5589`;
    /// `note_chooser_edited`). It changes the draft only; [`Self::SaveProfile`]
    /// writes it.
    EditProfileDraft(ProfileDraftField),
    /// Discard the draft and carry out the staged decision: open its
    /// target, close the form or Profiles, delete the target or turn
    /// MultiBox off (panel `Discard`, `confirm_pending_edit_switch`,
    /// `session/chooser.rs:251-272`).
    DiscardProfileEdits,
    /// Keep editing: withdraw the staged Discard / Keep editing decision,
    /// the form and its draft untouched (`cancel_pending_edit_switch`,
    /// `session/chooser.rs:274-276`).
    KeepEditing,
    /// Close the profile form, or Profiles with it (`request_cancel_edit`,
    /// `request_close_profiles`, `session/chooser.rs:281-305`). While the
    /// form's save is still being written nothing closes: the Discard / Keep
    /// editing decision is staged (`Completed(Skipped(DecisionStaged))`).
    CloseProfileForm(FormClose),
    /// Record that the member's profile skipped the tutorial: TutSkip's
    /// durable half (`Session::mark_tutorial_skipped`,
    /// `panel/src/session.rs:3513-3535`, a `save_profile` of
    /// `tutorial_skipped` with no arm mirror). The TutSkip button
    /// (`app.rs:2100-2104`) is two commands with separate reports:
    /// [`Self::DebugCommand`] (`setvar tutorial 1000`) and this row; either
    /// can fail without undoing the other.
    MarkTutorialSkipped,
    // --- rs2b0t profile import ---
    /// Edit the shared export path held by the importer
    /// (`ProfileImport::path_mut`; the panel path field and TUI overlay).
    EditProfileImportPath,
    /// Read and map the rs2b0t export into the preview without writing
    /// (`ProfileImport::preview`; the panel Preview button and TUI action).
    PreviewProfileImport,
    /// Choose how an existing profile in the preview is handled
    /// (`ProfileImport::set_policy`).
    SetProfileImportPolicy(ConflictPolicy),
    /// Include accounts the 289 client cannot log into in the import
    /// (`ProfileImport::set_include_unusable`).
    SetProfileImportIncludeUnusable(bool),
    /// Return to the path field, dropping the parsed preview file and its
    /// passwords (`ProfileImport::back`).
    ReturnToProfileImportPath,
    /// Show or withdraw the export-file deletion confirmation
    /// (`ProfileImport::{ask_delete, cancel_delete}`).
    SetProfileImportDeletePrompt(bool),
    /// Delete the plaintext export after explicit confirmation
    /// (`ProfileImport::confirm_delete`, `delete_export_file`).
    DeleteProfileImportExport,

    // --- Loadouts ---
    /// Make the selected loadout the editing draft (panel Loadouts list and
    /// opener, `loadouts::select`; TUI loadouts popup row 0 and its open).
    /// The front end stages the index (`LoadoutForm::stage_selection`).
    /// Unsaved draft edits of the previous selection are dropped, as today;
    /// `Skipped(NothingSelected)` when the index names no loadout.
    SelectLoadout,
    /// A staged edit of the loadout draft (panel name, worn slot, supplies,
    /// Clear unassigned: `loadouts.rs:267,277-281,311-434` and the item
    /// search popup). The widget edits the draft in place through
    /// `LoadoutForm::draft_mut`; the row commits the edit: a status about
    /// the draft's last save or copy no longer describes it. It changes the
    /// draft only; [`Self::SaveLoadout`] writes it. `Rejected(NotStaged)`
    /// without a draft. The TUI edits its scratch text itself and stages
    /// the parsed draft for [`Self::SaveLoadout`].
    EditLoadoutDraft(LoadoutField),
    /// Copy the member's current equipment into the loadout draft, keeping
    /// its supplies (panel `copy_equipment`; `OperatorSession::observed_equipment`):
    /// a draft edit, nothing is written. Skipped `NotLoggedIn` without an
    /// in-game focused character, `NothingSelected` without a draft.
    CopyEquipmentToLoadout,
    /// Create a loadout and write the loadout file at once, the in-memory
    /// set rolled back if the write fails (panel `add_loadout`,
    /// `loadouts.rs:65-80`; TUI row 5, `tui/src/loadouts.rs:123-140`).
    /// Settles `Done`, or `Failed(WriteFailed)` with the store's reason.
    CreateLoadout,
    /// Duplicate the selected loadout and write at once, rolled back if the
    /// write fails (panel `duplicate_loadout`, `loadouts.rs:82-106`; TUI
    /// row 6, `tui/src/loadouts.rs:141-166`). `Skipped(NothingSelected)`
    /// without a selection; `Failed(WriteFailed)` when the write is rolled
    /// back.
    DuplicateLoadout,
    /// Save the edited loadout draft (panel `save_draft`, `loadouts.rs:37-63`;
    /// TUI Enter on a scratch row or row 8, `commit_scratch`,
    /// `tui/src/loadouts.rs:56-85`). `Rejected(NotStaged)` without a draft,
    /// `Rejected(NotApplicable)` for a blank name or a name another loadout
    /// has, `Failed(WriteFailed)` when the write is rolled back (the draft is
    /// kept).
    SaveLoadout,
    /// Delete the selected loadout, written at once (panel `delete_loadout`,
    /// `loadouts.rs:108-132`; TUI row 7, `tui/src/loadouts.rs:167-190`).
    /// `Skipped(NothingSelected)` without a selection;
    /// `Failed(WriteFailed)` when the write is rolled back.
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
    /// Reorder the saved Load order with the staged
    /// [`crate::profile_order::ReorderOp`] (a rail or Fleet-row drag, the
    /// TUI move keys, Move marked to top;
    /// [`crate::OperatorSession::stage_reorder`]). Refused unless the fleet
    /// view shows Load order. Settles `Done` when the saved order is
    /// durable (at once when nothing moved).
    ReorderProfiles,
    /// Commit the fleet card the lent fleet model staged
    /// ([`crate::FleetModel::stage_card`]): which card the fleet buttons
    /// use. Changes no assignment, Browse pick, form or run.
    SetFleetCard,
    /// Mark, unmark or invert the shown fleet rows, or mark the shown rows
    /// in a status ([`crate::MarkOp`]).
    MarkFleet(crate::fleet::MarkOp),
    /// Freeze a fleet action into a plan ([`crate::fleet_plan`]): the
    /// marked profiles (hidden ones too), or the loaded shown rows with no
    /// marks, the resolved card, the interrupted and skipped rows and the
    /// prompt. The plan then runs with [`crate::Targets::Plan`].
    PlanFleetAction(crate::fleet_plan::FleetAction),
    /// Drop the pending fleet plan without running it (a settings copy's
    /// start hold is released; nothing is written).
    CancelFleetPlan,

    // --- Movement and game input ---
    /// Walk the focused member to the staged map destination (panel
    /// `WalkTo`/`Go`, TUI map `Enter`; `Play::map_walk`).
    WalkTo,
    /// Walk the marked members (`group_walk::walk_marked`, `group_walk.rs:58`).
    GroupWalk,
    /// Local debug teleport (panel `Teleport`, TUI `t`; `Play::map_teleport`).
    Teleport,
    /// Send a confirmed debug command (panel `Send##debug-send`, the
    /// Lumbridge and maxme buttons and TutSkip's network half;
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
    /// `Session::{request_focused_raster, set_focused_raster}`,
    /// `panel/src/session.rs:4123-4138`): the renderer switches at once and
    /// the profile's raster preference is written durably
    /// (`persist_game_render_prefs` → `save_profile`, "render prefs"), so
    /// the member settles on the writer.
    SetRaster(RasterMode),
    /// One draw-policy switch (panel `game renderer`, `capture input`,
    /// `sidecar 50 fps`, `focused 50 fps`, `full rate (this run)`). The
    /// switch applies at once; a saved switch settles on the preference
    /// writer's durable receipt.
    SetDraw(DrawSwitch, bool),

    // --- Preferences and notices ---
    /// Set one shared navigation preference (`nav_prefs.rs:5-25`).
    SetNavPreference(NavPreference, bool),
    /// Set the global danger-routing level
    /// (`WalkGlobalsView::persist_changed_at`, `walk_permissions.rs:183`).
    SetDangerLevel(DangerLevel),
    /// Turn the session log file on or off (`log_file::apply_session_log`).
    SetSessionLog(bool),
    /// Persist the map-bake choice (`map_bake::persist_map_bake_choice`;
    /// `MapBakeGate::set_choice`).
    SetMapBakeChoice(MapBakeChoice),
    /// Open the map demand for the bound profile through the bake gate
    /// (`MapBakeGate::{open_profile, open}`, `map_bake.rs:156-191`): the
    /// TUI's catalogue-only demand (`MapOpen` → `open_map_catalogue`,
    /// `bin.rs:1335-1352,3370-3373`) and the panel's walk-map demand
    /// (`sync_walk_map`, `panel/src/session.rs:4502-4513`), which the gate
    /// keeps catalogue-only and prompts for without bake consent. The
    /// demand admits catalogue decode or terrain bake work and settles
    /// when it is ready; a held demand is custody, not presentation.
    /// Distinct from [`Self::WalkTo`] and from [`Self::BakeMapNow`].
    OpenMapDemand,
    /// Release the held map demand and close the shared map model (panel
    /// `release_walk_map`, `panel/src/session.rs:4578-4587`, which also
    /// reaps the demand; TUI `MapClose` → `release_map_catalogue` and
    /// `map_model.close()`, `bin.rs:1470-1473,3374-3378`).
    ReleaseMapDemand,
    /// Bake now: consent to the local terrain bake for this process and
    /// reopen the images demand that starts it (`MapBakeGate::accept` then
    /// `open_profile(Images)`: panel `accept_map_bake`,
    /// `panel/src/session.rs:4547-4562`; "Always bake" composes
    /// [`Self::SetMapBakeChoice`] first). Settled by the bake demand.
    BakeMapNow,
    /// Decline the map-bake prompt (`MapBakeGate::decline`, `map_bake.rs:149`):
    /// the open demand stays catalogue-only.
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
    /// danger-routing notices). A saved acknowledgement settles on the
    /// preference writer's durable receipt.
    AcknowledgeNotice,
    /// Save the log view to a file (panel log `Save`, TUI log `w`).
    SaveLog,
}

/// A raster mode for one member's game image. Saved lowercase, as the
/// vault saves it (the panel keeps its global choice in its preferences).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum RasterMode {
    /// No game image.
    Off,
    /// GPU raster.
    #[default]
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

/// The fleet row order, owned by the core fleet model
/// ([`crate::FleetModel`]). Load order is the default on every surface;
/// Name and Status are view sorts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FleetOrder {
    /// The order the universe holds the rows in (load order).
    LoadOrder,
    /// By profile name.
    Name,
    /// By status (phase label), then profile name.
    Status,
}

impl FleetOrder {
    /// The orders the Sort drop-down offers, the default first.
    pub const CHOICES: [FleetOrder; 3] = [Self::LoadOrder, Self::Name, Self::Status];

    /// The drop-down entry.
    pub fn label(self) -> &'static str {
        match self {
            Self::LoadOrder => "Load order",
            Self::Name => "Name",
            Self::Status => "Status",
        }
    }
}

/// Which part of the profile form's draft an edit changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProfileDraftField {
    /// The username.
    Username,
    /// The password.
    Password,
    /// A profile setting (capture, random events, clue partner, world,
    /// memory mode).
    Settings,
}

/// Which part of the loadout draft an edit changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LoadoutField {
    /// The loadout's name.
    Name,
    /// A worn equipment slot.
    Equipment,
    /// A supply entry: added, removed, item or quantity changed.
    Supplies,
    /// The unassigned legacy gear list: cleared (panel `Clear unassigned`)
    /// or edited as free text committed as a list (TUI scratch row 3,
    /// `split_csv`).
    Unassigned,
}

/// The kind of card a parameter save is for; it decides what the save
/// needs from the process (`SaveScriptSettings`, `ApplySettingsToMarked`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CardKind {
    /// A loaded JS card: the bag is coerced and staged synchronously and
    /// the durable writer takes the profile row; no background job.
    Script,
    /// A native (compiled) card: the bag is prepared off-pump by a
    /// background job before the row is staged and written.
    Native,
}

/// What a profile-form close leaves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FormClose {
    /// Close the form; Profiles stays open (the form's Cancel).
    Form,
    /// Close Profiles and the form with it (its Close button and its ✕).
    Window,
}

/// What a command's members are.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SubjectKind {
    /// One member per targeted profile.
    Profile,
    /// Exactly one member: the application's shared state (the vault, the
    /// fleet view, the profile form, the loadout set, a shared preference).
    Application,
}

/// The subject of one member of a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Subject {
    /// One profile, by its stable identity.
    Profile(ProfileIdentity),
    /// The application's shared state, in [`Scope::Process`].
    Application,
}

impl Subject {
    /// Which kind of subject this is.
    pub const fn kind(self) -> SubjectKind {
        match self {
            Self::Profile(_) => SubjectKind::Profile,
            Self::Application => SubjectKind::Application,
        }
    }
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

/// The identity of one login attempt a slot owner runs: the mailbox
/// command it read the attempt as (its generation and cancellation fence)
/// and the world fence it was read under. The attempt is current while the
/// slot's mailbox still wants a login and neither fence moved
/// (`IntentMailbox::attempt_is_current`); a world change, an identity
/// change, a withdrawal, a closed lifetime or a completed login replaces it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LoginAttempt {
    command: IntentCommand,
    world_generation: u64,
}

impl LoginAttempt {
    /// The attempt the slot owner read as `command` under `world_generation`
    /// (`IntentMailbox::login_command` and `IntentSnapshot::world_generation`).
    pub const fn new(command: IntentCommand, world_generation: u64) -> Self {
        Self {
            command,
            world_generation,
        }
    }

    /// The command the attempt was read as.
    pub const fn command(self) -> IntentCommand {
        self.command
    }

    /// The world fence the attempt was read under.
    pub const fn world_generation(self) -> u64 {
        self.world_generation
    }

    /// Whether the attempt is still the one the slot's mailbox asks for,
    /// judged by `snapshot`: the same test as `IntentMailbox::attempt_is_current`.
    pub fn is_current(self, snapshot: &IntentSnapshot) -> bool {
        snapshot.login_wanted()
            && snapshot.cancellation() == self.command.cancellation()
            && snapshot.world_generation() == self.world_generation
    }
}

/// How far a pending member's work has progressed, as far as the process
/// capabilities it still needs are concerned (§26 §3.2). Only an owner
/// moves a member past [`Stage::Admitted`]: the slot owner's resolution
/// fact ([`CommandBook::address_resolved`]) or the preparation owner's
/// completion ([`CommandBook::prepared`]); a stale resolution is revoked
/// by the slot's mailbox snapshot ([`CommandBook::observe_intent`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Stage {
    /// Nothing the row needs is secured: it needs every capability of
    /// [`OperatorCommand::capabilities`].
    Admitted,
    /// A login-wanting member whose handshake address the slot owner
    /// resolved for this attempt, which is still current: it needs the
    /// transport (new connection, login) and the resolved address, no
    /// longer name resolution (§26 §3.2 resolver row: an already admitted,
    /// already-resolved handshake continues). Once the attempt is replaced
    /// the member is [`Stage::Admitted`] again.
    AddressResolved(LoginAttempt),
    /// A native settings member whose preparation job completed: the
    /// durable writer holds it, and it needs no process service any more.
    Prepared,
}

const NO_CAPABILITY: &[ProcessCapability] = &[];
const SESSION_LOGIN: &[ProcessCapability] = &[
    ProcessCapability::NewConnection,
    ProcessCapability::Login,
    ProcessCapability::NameResolution,
];
const RESOLVED_LOGIN: &[ProcessCapability] = &[
    ProcessCapability::NewConnection,
    ProcessCapability::Login,
    ProcessCapability::ResolvedAddress,
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
    pub const ALL: [OperatorCommand; 98] = [
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
        Self::SetSurvival,
        Self::Quit,
        Self::UnlockVault,
        Self::CreateVault,
        Self::ResetVault,
        Self::SetFleetFilter,
        Self::SetFleetSort(FleetOrder::Status),
        Self::StartScript,
        Self::PauseScript,
        Self::ResumeScript,
        Self::SkipQuest { target: None },
        Self::StopScript,
        Self::CancelQueuedStart,
        Self::PickBrowseCard,
        Self::SelectScriptCard,
        Self::PrepareScriptCard,
        Self::AssignScript,
        Self::AssignAndRestart,
        Self::SaveScriptSettings(CardKind::Native),
        Self::SaveLegacyScriptSettings,
        Self::EditParameterText,
        Self::DiscardParameterText,
        Self::ApplySettingsToMarked(CardKind::Native),
        Self::CancelSettingsSync,
        Self::RestartForSettings,
        Self::DeferSettingsRestart,
        Self::LoadScriptFile,
        Self::ReloadScripts,
        Self::ConfirmReload,
        Self::CancelReload,
        Self::ImportCatalog,
        Self::TranspileAll,
        Self::DeferCatalogImport,
        Self::ScriptPaintInput,
        Self::SaveProfile,
        Self::CreateProfile,
        Self::DeleteProfile,
        Self::OpenProfileForm,
        Self::OpenNewProfileForm,
        Self::EditProfileDraft(ProfileDraftField::Username),
        Self::DiscardProfileEdits,
        Self::KeepEditing,
        Self::CloseProfileForm(FormClose::Form),
        Self::MarkTutorialSkipped,
        Self::EditProfileImportPath,
        Self::PreviewProfileImport,
        Self::SetProfileImportPolicy(ConflictPolicy::Skip),
        Self::SetProfileImportIncludeUnusable(true),
        Self::ReturnToProfileImportPath,
        Self::SetProfileImportDeletePrompt(true),
        Self::DeleteProfileImportExport,
        Self::SelectLoadout,
        Self::EditLoadoutDraft(LoadoutField::Supplies),
        Self::CopyEquipmentToLoadout,
        Self::CreateLoadout,
        Self::DuplicateLoadout,
        Self::SaveLoadout,
        Self::DeleteLoadout,
        Self::Select,
        Self::SetMark(true),
        Self::MarkAll,
        Self::ClearMarks,
        Self::ReorderProfiles,
        Self::SetFleetCard,
        Self::MarkFleet(crate::fleet::MarkOp::Shown),
        Self::PlanFleetAction(crate::fleet_plan::FleetAction::Start),
        Self::CancelFleetPlan,
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
        Self::OpenMapDemand,
        Self::ReleaseMapDemand,
        Self::BakeMapNow,
        Self::DeclineMapBake,
        Self::SetQuesterPaths,
        Self::ReloadQuesterPaths,
        Self::SetRenderAll(true),
        Self::AcknowledgeNotice,
        Self::SaveLog,
    ];

    /// What the command's members are. A profile row has one member per
    /// targeted profile; an application row has exactly one member,
    /// [`Subject::Application`] in [`Scope::Process`].
    pub const fn subject(self) -> SubjectKind {
        match self {
            Self::UnlockVault
            | Self::CreateVault
            | Self::ResetVault
            | Self::SetFleetFilter
            | Self::SetFleetSort(_)
            | Self::PickBrowseCard
            | Self::PrepareScriptCard
            | Self::SaveLegacyScriptSettings
            | Self::EditParameterText
            | Self::DiscardParameterText
            | Self::CancelSettingsSync
            | Self::RestartForSettings
            | Self::DeferSettingsRestart
            | Self::LoadScriptFile
            | Self::ReloadScripts
            | Self::ConfirmReload
            | Self::CancelReload
            | Self::ImportCatalog
            | Self::TranspileAll
            | Self::DeferCatalogImport
            | Self::EditProfileImportPath
            | Self::PreviewProfileImport
            | Self::SetProfileImportPolicy(_)
            | Self::SetProfileImportIncludeUnusable(_)
            | Self::ReturnToProfileImportPath
            | Self::SetProfileImportDeletePrompt(_)
            | Self::DeleteProfileImportExport
            | Self::CreateProfile
            | Self::OpenNewProfileForm
            | Self::EditProfileDraft(_)
            | Self::DiscardProfileEdits
            | Self::KeepEditing
            | Self::CloseProfileForm(_)
            | Self::SelectLoadout
            | Self::EditLoadoutDraft(_)
            | Self::CreateLoadout
            | Self::DuplicateLoadout
            | Self::SaveLoadout
            | Self::DeleteLoadout
            | Self::MarkAll
            | Self::ClearMarks
            | Self::ReorderProfiles
            | Self::SetFleetCard
            | Self::MarkFleet(_)
            | Self::PlanFleetAction(_)
            | Self::CancelFleetPlan
            | Self::SetMultiBox(_)
            | Self::SetDraw(..)
            | Self::SetNavPreference(..)
            | Self::SetDangerLevel(_)
            | Self::SetSessionLog(_)
            | Self::SetMapBakeChoice(_)
            | Self::OpenMapDemand
            | Self::ReleaseMapDemand
            | Self::BakeMapNow
            | Self::DeclineMapBake
            | Self::SetQuesterPaths
            | Self::ReloadQuesterPaths
            | Self::SetRenderAll(_)
            | Self::AcknowledgeNotice
            | Self::SaveLog => SubjectKind::Application,
            _ => SubjectKind::Profile,
        }
    }

    /// Whether this command accepts a member of `kind`.
    ///
    /// `WalkTo` also accepts the process member for an unfocused refusal
    /// receipt; its ordinary target remains a profile.
    pub(crate) fn accepts_subject(self, kind: SubjectKind) -> bool {
        self.subject() == kind || matches!((self, kind), (Self::WalkTo, SubjectKind::Application))
    }

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

    /// The process capabilities a new command needs at admission (§26
    /// §3.1). A latched condition affecting one of them refuses the command
    /// unless it is [`Self::retirement_dependent`]. A login is admitted
    /// unresolved, so it needs name resolution. A settings save needs the
    /// background service only for a native card's preparation
    /// ([`CardKind`]); a JS save and the legacy save are staged
    /// synchronously and written by the durable writer.
    pub const fn capabilities(self) -> &'static [ProcessCapability] {
        match self {
            Self::Login | Self::RelogNow | Self::SwitchWorld(_) => SESSION_LOGIN,
            Self::Logout | Self::Remove | Self::Teleport | Self::DebugCommand => NETWORK_EFFECT,
            Self::ScriptPaintInput | Self::GameInput => NETWORK_EFFECT,
            Self::SaveScriptSettings(CardKind::Native)
            | Self::ApplySettingsToMarked(CardKind::Native) => BACKGROUND_WORK,
            Self::SaveScriptSettings(CardKind::Script)
            | Self::ApplySettingsToMarked(CardKind::Script) => NO_CAPABILITY,
            Self::StartScript
            | Self::AssignAndRestart
            | Self::RestartForSettings
            | Self::PrepareScriptCard
            | Self::LoadScriptFile
            | Self::ReloadScripts
            | Self::ConfirmReload
            | Self::ImportCatalog
            | Self::TranspileAll
            | Self::OpenMapDemand
            | Self::BakeMapNow
            | Self::ReloadQuesterPaths => BACKGROUND_WORK,
            Self::WalkTo | Self::GroupWalk => ROUTED_MOVEMENT,
            _ => NO_CAPABILITY,
        }
    }

    /// The capabilities an accepted member still needs at `stage`: what an
    /// outage can make impossible and what its status shows while it waits.
    /// A login-wanting member past resolution no longer needs the resolver;
    /// a prepared native settings member needs nothing more; every other
    /// row's needs do not change after admission.
    pub const fn needs(self, stage: Stage) -> &'static [ProcessCapability] {
        match stage {
            Stage::AddressResolved(_) if self.wants_login() => RESOLVED_LOGIN,
            Stage::Prepared if self.prepares() => NO_CAPABILITY,
            Stage::Admitted | Stage::AddressResolved(_) | Stage::Prepared => self.capabilities(),
        }
    }

    /// Whether the row's work has a preparation phase before its durable
    /// write: a native card's settings are prepared by a background job
    /// and the durable writer takes the row afterwards
    /// ([`CommandBook::prepared`]).
    pub const fn prepares(self) -> bool {
        matches!(
            self,
            Self::SaveScriptSettings(CardKind::Native)
                | Self::ApplySettingsToMarked(CardKind::Native)
        )
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
            Self::Login | Self::Logout | Self::RelogNow => Settles::IntentApplied,
            Self::Select
            | Self::SetMark(_)
            | Self::MarkAll
            | Self::ClearMarks
            | Self::SetFleetCard
            | Self::MarkFleet(_)
            | Self::PlanFleetAction(_)
            | Self::CancelFleetPlan
            | Self::SetFleetFilter
            | Self::SetFleetSort(_)
            | Self::PickBrowseCard
            | Self::SelectScriptCard
            | Self::EditParameterText
            | Self::DiscardParameterText
            | Self::OpenProfileForm
            | Self::OpenNewProfileForm
            | Self::EditProfileDraft(_)
            | Self::DiscardProfileEdits
            | Self::KeepEditing
            | Self::CloseProfileForm(_)
            | Self::SelectLoadout
            | Self::EditLoadoutDraft(_)
            | Self::CopyEquipmentToLoadout
            | Self::EditProfileImportPath
            | Self::PreviewProfileImport
            | Self::SetProfileImportPolicy(_)
            | Self::SetProfileImportIncludeUnusable(_)
            | Self::ReturnToProfileImportPath
            | Self::SetProfileImportDeletePrompt(_)
            | Self::DeleteProfileImportExport
            | Self::SkipQuest { .. }
            | Self::CancelQueuedStart
            | Self::CancelSettingsSync
            | Self::DeferSettingsRestart
            | Self::CancelReload
            | Self::ReleaseMapDemand
            | Self::DeclineMapBake
            | Self::SetRenderAll(_)
            | Self::SetMultiBox(_)
            | Self::GameInput => Settles::AtDispatch,
            _ => Settles::Owner,
        }
    }

    /// Whether posting `self` for a member supersedes an older pending
    /// `older` of the same member: the older intent can no longer be what
    /// the slot acknowledges, so it settles `Cancelled(Superseded)`. Today's
    /// rule (`session.rs:958-960,1012-1019,1147-1148,1214`): Log in, Log out
    /// and Relog now cancel the member's pending Log in and Log out (Relog
    /// now reports as a Log out op); Remove cancels the pending Log in.
    /// Switch world joins the login-wanting turnovers. This matches the
    /// mailbox (F0-1/2 r2): Log in, Log out and Remove supersede a pending
    /// turnover's login half.
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

    /// What a member this row supersedes ([`Self::supersedes`]) reads as
    /// its reason in a result: what the book knows when this row is
    /// posted, in the operator's words. A posted Remove or Quit is still
    /// pending (each settles when the slot retires), so it reads as
    /// requested, not as done. `None` for a row that supersedes nothing.
    pub const fn took_over_words(self) -> Option<&'static str> {
        match self {
            Self::Login => Some("a later Log in took over"),
            Self::Logout => Some("a later Log out took over"),
            Self::RelogNow => Some("a later Relog now took over"),
            Self::SwitchWorld(_) => Some("a later Switch world took over"),
            Self::Remove => Some("Remove was requested"),
            Self::Quit => Some("Quit was requested"),
            _ => None,
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
            | Self::SetSurvival
            | Self::AssignScript
            | Self::SaveScriptSettings(_)
            | Self::SetRaster(_)
            | Self::MarkTutorialSkipped => Some(ActionKind::SaveProfile),
            Self::DeleteProfile => Some(ActionKind::DeleteProfile),
            Self::ApplySettingsToMarked(_) => Some(ActionKind::SyncSettings),
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
    /// Not logged in (`marked.rs:330`; `bulk.rs` walk rows; Copy current
    /// equipment without an in-game character, `loadouts.rs:135-137`).
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
    /// No loadout is selected to copy into or duplicate
    /// (`loadouts.rs:88-90,139-141`).
    NothingSelected,
    /// The profile form has unsaved edits or a save still being written:
    /// the Discard / Keep editing decision was staged instead
    /// (`session/chooser.rs:193-200,282-285,295-299`).
    DecisionStaged,
    /// The bot is already on the requested world (Switch world).
    AlreadyOnWorld,
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
    /// The front end that issued the command does not hold the model the
    /// row changes (for example the TUI and the panel's profile form):
    /// nothing was changed.
    NoModel,
    /// The row commits a staged input and nothing was staged (no
    /// passphrase, no destination, no selected card, no loadout draft).
    NotStaged,
    /// The vault could not be opened or created with the staged
    /// passphrase, or the vault file could not be reset.
    VaultRefused,
    /// The row's target state already holds or does not apply to this
    /// member (a mark on an unknown row, a walk with no map bound).
    NotApplicable,
    /// A saved-setting write was refused before the preference queue
    /// accepted it (the queue is full, hydration has not finished, or the
    /// value is invalid). Nothing was saved and nothing is pending: a write
    /// the queue accepted and that failed later is
    /// [`FailureKind::WriteFailed`].
    WriteRefused,
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
    /// A durable write failed (`session.rs:2296`; `native_settings.rs:298,409,519`;
    /// the loadout writes, `panel/src/loadouts.rs:57-61,75-78,101-104`).
    WriteFailed,
    /// Saved but the live run rejected the settings (`scripts/sync.rs:760`).
    LiveRejected,
    /// A preparation, validation or route job failed
    /// (`scripts/reload.rs:162,249`; `group_walk.rs:96,132-137`).
    JobFailed,
    /// The slot's mailbox refused the row because a generation is
    /// exhausted (`MailboxError::Exhausted`): nothing was posted and the
    /// mailbox is unchanged.
    IntentExhausted,
}

/// What a pending member waits for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Pending {
    /// Accepted; its owner has not settled it and, for a mailbox row,
    /// nothing is posted yet.
    Accepted,
    /// Posted to its admitted slot's mailbox; waits for `IntentApplied` of
    /// `generation` or later from that slot (or its owner's report).
    Posted {
        /// The slot whose mailbox received the post: always the member's
        /// admitted slot.
        slot: SlotKey,
        /// The receipt's generation.
        generation: IntentGeneration,
    },
}

/// A book operation that does not match the protocol. A refused operation
/// changes nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CommandError {
    /// Command identities are host-namespace ids.
    #[error("command identity {0:?} is not a host request id")]
    IsolateRequest(RequestId),
    /// The identity was already used.
    #[error("command identity {0:?} is already in use")]
    DuplicateRequest(RequestId),
    /// A command names at least one subject.
    #[error("command {0:?} has no members")]
    NoMembers(RequestId),
    /// No such command (or it was evicted after settling).
    #[error("unknown command {0:?}")]
    UnknownRequest(RequestId),
    /// The subject was named twice in one command.
    #[error("{subject:?} is already a member of command {request:?}")]
    DuplicateMember {
        /// The command.
        request: RequestId,
        /// The member.
        subject: Subject,
    },
    /// The subject's kind is not the row's ([`OperatorCommand::subject`]),
    /// or an application member is not in [`Scope::Process`].
    #[error("{subject:?} in {scope:?} cannot be a member of command {request:?}")]
    WrongSubject {
        /// The command.
        request: RequestId,
        /// The member.
        subject: Subject,
        /// The scope it was offered in.
        scope: Scope,
    },
    /// No such member of this command.
    #[error("{subject:?} is not a member of command {request:?}")]
    UnknownMember {
        /// The command.
        request: RequestId,
        /// The member.
        subject: Subject,
    },
    /// The member already has its one terminal.
    #[error("{subject:?} of command {request:?} is already settled")]
    AlreadySettled {
        /// The command.
        request: RequestId,
        /// The member.
        subject: Subject,
    },
    /// Only a mailbox row's member can record a post.
    #[error("command {0:?} posts no intent")]
    NoIntentRow(RequestId),
    /// The member's one post is already recorded; a second receipt is
    /// never accepted.
    #[error("{subject:?} of command {request:?} is already posted")]
    AlreadyPosted {
        /// The command.
        request: RequestId,
        /// The member.
        subject: Subject,
    },
    /// Only a row with a preparation phase ([`OperatorCommand::prepares`])
    /// can report a completed preparation.
    #[error("command {0:?} has no preparation phase")]
    NoPreparation(RequestId),
    /// The member's preparation is already recorded; a second report is
    /// never accepted.
    #[error("{subject:?} of command {request:?} is already prepared")]
    AlreadyPrepared {
        /// The command.
        request: RequestId,
        /// The member.
        subject: Subject,
    },
    /// The post named a slot other than the member's admitted slot.
    #[error("{subject:?} of command {request:?} was not admitted on {slot:?}")]
    WrongSlot {
        /// The command.
        request: RequestId,
        /// The member.
        subject: Subject,
        /// The slot the post named.
        slot: SlotKey,
    },
    /// Condition terminals come only from admission, the slot owner's
    /// `Unavailable` event and [`CommandBook::fail_unavailable`]; an owner
    /// report cannot carry one.
    #[error(
        "{subject:?} of command {request:?}: an owner report cannot carry a condition terminal"
    )]
    ConditionTerminal {
        /// The command.
        request: RequestId,
        /// The member.
        subject: Subject,
    },
}

#[derive(Debug)]
enum MemberState {
    Pending(Pending),
    Settled(Settlement<Completion, CommandFailure>),
}

/// One targeted member of a command.
#[derive(Debug)]
pub struct MemberReport {
    subject: Subject,
    scope: Scope,
    stage: Stage,
    state: MemberState,
    detail: Option<Box<str>>,
}

impl MemberReport {
    /// The member's subject.
    pub fn subject(&self) -> Subject {
        self.subject
    }

    /// The scope the member was admitted in and its terminal is reported in.
    pub fn scope(&self) -> Scope {
        self.scope
    }

    /// How far its work has progressed ([`OperatorCommand::needs`]).
    pub fn stage(&self) -> Stage {
        self.stage
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

    fn on_slot(&self, slot: SlotKey) -> bool {
        self.scope.slot() == Some(slot)
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

    /// Every member, in the order the command named them.
    pub fn members(&self) -> &[MemberReport] {
        &self.members
    }

    /// One member.
    pub fn member(&self, subject: Subject) -> Option<&MemberReport> {
        self.members.iter().find(|m| m.subject == subject)
    }

    /// Whether every member has its terminal. A command's members are all
    /// named when it opens, so a settled report never gains another.
    pub fn is_settled(&self) -> bool {
        self.members.iter().all(|m| m.pending().is_none())
    }

    fn member_mut(&mut self, subject: Subject) -> Result<&mut MemberReport, CommandError> {
        let request = self.request;
        self.members
            .iter_mut()
            .find(|m| m.subject == subject)
            .ok_or(CommandError::UnknownMember { request, subject })
    }
}

/// The bounded command history and its settlement protocol. Reports with a
/// pending member are kept in the order they opened; once a report's last
/// member settles it joins the settled history, which keeps the
/// [`SETTLED_REPORT_CAP`] most recently settled reports.
#[derive(Debug, Default)]
pub struct CommandBook {
    pending: Vec<CommandReport>,
    settled: VecDeque<CommandReport>,
    /// Member buffers of evicted reports, reused by [`Self::open`] so a
    /// steady stream of commands allocates no member lists.
    spare: Vec<Vec<MemberReport>>,
}

impl CommandBook {
    /// An empty book.
    pub fn new() -> Self {
        Self::default()
    }

    /// Open a command under `request`, minted by the composition's host
    /// `RequestIdMinter`, naming every member at once as `(subject, scope)`.
    /// A member whose command needs a capability a latched condition
    /// affects is refused here (unless the row is retirement-dependent): it
    /// settles at once as `Failed(CommandFailure::Refused)`, keeping the
    /// command identity and scope, and is never Pending. Every other member
    /// is Pending. A refused open changes nothing.
    pub fn open(
        &mut self,
        request: RequestId,
        command: OperatorCommand,
        targets: &[(Subject, Scope)],
        conditions: &ProcessConditions,
    ) -> Result<(), CommandError> {
        if request.namespace() != RequestNamespace::Host {
            return Err(CommandError::IsolateRequest(request));
        }
        if self.report(request).is_some() {
            return Err(CommandError::DuplicateRequest(request));
        }
        if targets.is_empty() {
            return Err(CommandError::NoMembers(request));
        }
        for (index, &(subject, scope)) in targets.iter().enumerate() {
            let application_in_process =
                subject.kind() != SubjectKind::Application || scope == Scope::Process;
            if !command.accepts_subject(subject.kind()) || !application_in_process {
                return Err(CommandError::WrongSubject {
                    request,
                    subject,
                    scope,
                });
            }
            if targets[..index].iter().any(|&(seen, _)| seen == subject) {
                return Err(CommandError::DuplicateMember { request, subject });
            }
        }
        let blocked = if command.retirement_dependent() {
            None
        } else {
            first_block(command.capabilities(), conditions)
        };
        let state = match blocked {
            Some((condition, capability)) => {
                MemberState::Settled(Settlement::Failed(CommandFailure::Refused {
                    condition,
                    capability,
                }))
            }
            None => MemberState::Pending(Pending::Accepted),
        };
        let mut members = self.spare.pop().unwrap_or_default();
        members.extend(targets.iter().map(|&(subject, scope)| MemberReport {
            subject,
            scope,
            stage: Stage::Admitted,
            state: match &state {
                MemberState::Pending(pending) => MemberState::Pending(*pending),
                MemberState::Settled(terminal) => MemberState::Settled(terminal.clone()),
            },
            detail: None,
        }));
        self.pending.push(CommandReport {
            request,
            command,
            members,
        });
        self.file_settled();
        Ok(())
    }

    /// Whether any report still has a pending member.
    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    /// The report for `request`.
    pub fn report(&self, request: RequestId) -> Option<&CommandReport> {
        self.reports().find(|r| r.request == request)
    }

    /// Every retained report: the settled history, oldest settled first,
    /// then the reports with a pending member, in the order they opened.
    pub fn reports(&self) -> impl Iterator<Item = &CommandReport> {
        self.settled.iter().chain(&self.pending)
    }

    /// Whether [`Self::posted`] would record a post of `request`'s intent
    /// row to `slot` for `subject`. An owner that posts the row itself asks
    /// first, so a refused member never reaches the slot's mailbox.
    pub fn may_post(
        &self,
        request: RequestId,
        subject: Subject,
        slot: SlotKey,
    ) -> Result<(), CommandError> {
        self.postable(request, subject, slot).map(|_| ())
    }

    /// Record the receipt of posting the command's intent row to `slot` for
    /// `subject`, and supersede that member's older pending commands this
    /// one replaces ([`OperatorCommand::supersedes`]). Only a Pending
    /// Accepted member of a mailbox row can be posted, once, and only to
    /// the slot it was admitted on: a second post, a wrong slot or a
    /// settled member is refused and changes nothing.
    pub fn posted(
        &mut self,
        request: RequestId,
        subject: Subject,
        slot: SlotKey,
        receipt: PostReceipt,
    ) -> Result<(), CommandError> {
        let command = self.postable(request, subject, slot)?;
        if let Some(member) = self.pending_member_mut(request, subject) {
            member.state = MemberState::Pending(Pending::Posted {
                slot,
                generation: receipt.generation(),
            });
        }
        for older in self.pending.iter_mut().filter(|r| r.request != request) {
            if !command.supersedes(older.command) {
                continue;
            }
            for member in older.members.iter_mut().filter(|m| m.subject == subject) {
                if matches!(member.state, MemberState::Pending(_)) {
                    member.state =
                        MemberState::Settled(Settlement::Cancelled(Cancellation::Superseded));
                    // Results name what took the member over.
                    member.detail = command.took_over_words().map(Box::from);
                }
            }
        }
        self.file_settled();
        Ok(())
    }

    /// The slot's mailbox refused the member's row (F0-1/2 r2: a refused
    /// row changes nothing in the mailbox). A closed slot lifetime
    /// (`MailboxError::Closed`: Remove or shutdown came first) invalidates
    /// the member's scope, `Cancelled(ScopeInvalidated)`, except that a
    /// retirement-dependent member stays Pending until the slot really
    /// retires ([`Self::slot_retired`]); an exhausted generation fails it
    /// `Failed(IntentExhausted)`. Nothing supersedes anything: the row was
    /// never posted. The same protocol checks as [`Self::posted`] apply.
    pub fn post_refused(
        &mut self,
        request: RequestId,
        subject: Subject,
        slot: SlotKey,
        error: MailboxError,
    ) -> Result<(), CommandError> {
        let command = self.postable(request, subject, slot)?;
        let terminal = match error {
            MailboxError::Closed if command.retirement_dependent() => return Ok(()),
            MailboxError::Closed => Settlement::Cancelled(Cancellation::ScopeInvalidated),
            MailboxError::Exhausted(_) => {
                Settlement::Failed(CommandFailure::Failed(FailureKind::IntentExhausted))
            }
        };
        if let Some(member) = self.pending_member_mut(request, subject) {
            member.state = MemberState::Settled(terminal);
        }
        self.file_settled();
        Ok(())
    }

    /// The slot owner resolved the handshake address of the login attempt
    /// it read as `attempt` on `slot`, reported with the mailbox snapshot
    /// it holds now: every pending login-wanting member posted to `slot` at
    /// the attempt's generation or earlier no longer needs name resolution
    /// ([`Stage::AddressResolved`]) for as long as that attempt is current.
    /// A member posted later still does. A fact for an attempt the mailbox
    /// already replaced (`!attempt.is_current(snapshot)`: a world change,
    /// an identity change, a withdrawal or a completed login moved a
    /// fence) is late and changes nothing. Returns how many members moved.
    pub fn address_resolved(
        &mut self,
        slot: SlotKey,
        attempt: LoginAttempt,
        snapshot: &IntentSnapshot,
    ) -> usize {
        if !attempt.is_current(snapshot) {
            return 0;
        }
        let through = attempt.command().generation();
        let mut moved = 0;
        for report in self.pending.iter_mut().filter(|r| r.command.wants_login()) {
            for member in &mut report.members {
                if let MemberState::Pending(Pending::Posted {
                    slot: posted,
                    generation,
                }) = member.state
                {
                    if posted == slot && generation <= through && member.stage == Stage::Admitted {
                        member.stage = Stage::AddressResolved(attempt);
                        moved += 1;
                    }
                }
            }
        }
        moved
    }

    /// The preparation owner reports that the member's preparation job
    /// completed and the durable writer now holds the row
    /// ([`Stage::Prepared`]): the member no longer needs the background
    /// service. Only a pending, still unprepared member of a row with a
    /// preparation phase ([`OperatorCommand::prepares`]) accepts it; a
    /// second report, a settled member or another row is refused and
    /// changes nothing. The writer's own terminal still settles the member
    /// ([`Self::settle`]).
    pub fn prepared(&mut self, request: RequestId, subject: Subject) -> Result<(), CommandError> {
        let report = self
            .report(request)
            .ok_or(CommandError::UnknownRequest(request))?;
        if !report.command.prepares() {
            return Err(CommandError::NoPreparation(request));
        }
        let member = self.locate_pending(request, subject)?;
        if member.stage == Stage::Prepared {
            return Err(CommandError::AlreadyPrepared { request, subject });
        }
        member.stage = Stage::Prepared;
        Ok(())
    }

    /// Apply one slot-scoped session event; returns how many members
    /// settled.
    ///
    /// - `IntentApplied { generation }` from slot S completes every pending
    ///   [`Settles::IntentApplied`] member posted to S at `generation` or
    ///   earlier; a member posted at a later generation, or to another slot,
    ///   is untouched (a stale acknowledgement never settles a newer post).
    /// - `Unavailable { capability, condition }` from slot S is the slot
    ///   owner's decision that its next step needs a lost capability: every
    ///   pending, non-retirement member admitted on S whose current stage
    ///   still needs `capability` gets its one `Failed(Unavailable)` terminal
    ///   referencing `condition`, its own command and its scope. An owner
    ///   whose next step is name resolution is not continuing a resolved
    ///   handshake, so `NameResolution` first revokes the resolved stage of
    ///   S's login-wanting members ([`Stage::AddressResolved`]) and they
    ///   fail like unresolved ones. A retirement-dependent member stays
    ///   Pending (§26 §3.2 row 3), and no retirement or closure is inferred.
    /// - Every other event settles nothing. In particular
    ///   `SessionEnded { cause: ServiceUnavailable }` ends the session, not a
    ///   command: a pending Log out still needs its acknowledgement or the
    ///   slot's retirement, and a pending login's impossibility comes from
    ///   the slot's `Unavailable` or the ledger ([`Self::fail_unavailable`]).
    pub fn observe(&mut self, event: &Event<SessionEventOut>) -> usize {
        let Some(slot) = event.scope().slot() else {
            return 0;
        };
        let settled = match *event.payload() {
            SessionEventOut::IntentApplied { generation } => self.applied(slot, generation),
            SessionEventOut::Unavailable {
                capability,
                condition,
            } => self.unavailable_on(slot, capability, condition),
            _ => 0,
        };
        self.file_settled();
        settled
    }

    /// Why a login the slot no longer wants was withdrawn, in the
    /// operator's words, from the snapshot's park reason alone: a lifecycle
    /// fact (the repeat guard, a saved logged-out hold) or a logout row the
    /// slot's mailbox applied, which this book did not post (a row it posts
    /// settles the members it supersedes when posted). A logout row comes
    /// from more than the Log out command (Remove logs the bot out first)
    /// and may still be under way, so it reads as requested and names no
    /// command. `None` when the snapshot names no cause (the result then
    /// says only "cancelled") and for a world error, which fails the member
    /// instead.
    pub(crate) const fn withdrawn_words(reason: Option<ParkReason>) -> Option<&'static str> {
        match reason {
            Some(ParkReason::RepeatGuard) => {
                Some("the automatic login guard stopped repeated logins")
            }
            Some(ParkReason::OperatorLogout) => Some("a logout was requested"),
            Some(ParkReason::PersistedLogout) => Some("the bot is set to stay logged out"),
            Some(ParkReason::NoIntent | ParkReason::WorldError) | None => None,
        }
    }

    /// Apply the slot's mailbox snapshot, read after its events were
    /// observed. First the resolution fence: a pending login-wanting member
    /// posted to `slot` whose resolved attempt the mailbox no longer asks
    /// for ([`LoginAttempt::is_current`]: the world or cancellation fence
    /// moved while a login is still wanted) is [`Stage::Admitted`] again
    /// and needs name resolution for its replacement attempt; the login
    /// itself is neither completed nor cancelled by that. Then the
    /// withdrawal rule: a pending login-wanting member (Log in, Relog now,
    /// Switch world) posted to `slot` whose want a later generation
    /// withdrew settles: a world error as `Failed(WorldRejected)`, a latch
    /// or withdrawal as `Cancelled(Superseded)` (today `arm.login_latched()`
    /// cancels the pending Log in, `session.rs:1641-1644`), with the
    /// snapshot's park reason as its detail ([`Self::withdrawn_words`]).
    /// Members posted to another slot are untouched. Returns how many
    /// settled.
    pub fn observe_intent(&mut self, slot: SlotKey, snapshot: &IntentSnapshot) -> usize {
        for report in self.pending.iter_mut().filter(|r| r.command.wants_login()) {
            for member in report.members.iter_mut().filter(|m| m.on_slot(slot)) {
                if let Stage::AddressResolved(attempt) = member.stage {
                    if !attempt.is_current(snapshot) {
                        member.stage = Stage::Admitted;
                    }
                }
            }
        }
        if snapshot.login_wanted() {
            return 0;
        }
        let terminal = || match snapshot.park_reason() {
            Some(ParkReason::WorldError) => {
                Settlement::Failed(CommandFailure::Failed(FailureKind::WorldRejected))
            }
            _ => Settlement::Cancelled(Cancellation::Superseded),
        };
        // A row this book posted settled the members it superseded when it
        // was posted; what is left was withdrawn by the slot's own latch.
        let detail = Self::withdrawn_words(snapshot.park_reason());
        let mut settled = 0;
        for report in self.pending.iter_mut().filter(|r| r.command.wants_login()) {
            for member in &mut report.members {
                if let MemberState::Pending(Pending::Posted {
                    slot: posted,
                    generation,
                }) = member.state
                {
                    if posted == slot && generation < snapshot.generation() {
                        member.state = MemberState::Settled(terminal());
                        member.detail = detail.map(Box::from);
                        settled += 1;
                    }
                }
            }
        }
        self.file_settled();
        settled
    }

    /// Settle one member with its owner's terminal (a dispatch result, a
    /// durable receipt, a run-state report, a job terminal). Exactly once:
    /// a second terminal is refused and changes nothing. A condition
    /// terminal is refused here ([`CommandError::ConditionTerminal`]).
    pub fn settle(
        &mut self,
        request: RequestId,
        subject: Subject,
        terminal: Settlement<Completion, CommandFailure>,
        detail: Option<&str>,
    ) -> Result<(), CommandError> {
        if let Settlement::Failed(failure) = &terminal {
            if failure.condition().is_some() {
                return Err(CommandError::ConditionTerminal { request, subject });
            }
        }
        let member = self.locate_pending(request, subject)?;
        member.state = MemberState::Settled(terminal);
        member.detail = detail.map(Box::from);
        self.file_settled();
        Ok(())
    }

    /// The slot's lifetime ended (removed or its worker reaped). Its pending
    /// members settle: a withdrawal whose goal now holds (Log out, Remove,
    /// Stop, Quit) completes; Pause/Resume have no script left
    /// (`session.rs:1663-1680`); anything else is `Cancelled(ScopeInvalidated)`
    /// (today a pending Log in of a gone slot cancels, `session.rs:1628-1629`).
    /// A settled member keeps its terminal; a repeated retirement settles
    /// nothing. Returns how many settled.
    pub fn slot_retired(&mut self, slot: SlotKey) -> usize {
        let mut settled = 0;
        for report in &mut self.pending {
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
                if matches!(member.state, MemberState::Pending(_)) && member.on_slot(slot) {
                    member.state = MemberState::Settled(terminal.clone());
                    settled += 1;
                }
            }
        }
        self.file_settled();
        settled
    }

    /// Settle every accepted pending member the latched conditions make
    /// impossible with its one `Failed(Unavailable)` terminal, referencing
    /// the condition and keeping the command identity and scope (§26 §3.2
    /// row 2). Impossibility is per member: a condition fails a member only
    /// when its current stage still needs an affected capability
    /// ([`OperatorCommand::needs`]), so an unresolved login fails under a
    /// resolver outage and an already-resolved one continues while the
    /// transport is healthy. Retirement-dependent members stay Pending (row
    /// 3). This settles result custody only; it acknowledges no retirement.
    pub fn fail_unavailable(&mut self, conditions: &ProcessConditions) -> usize {
        let mut settled = 0;
        for report in self
            .pending
            .iter_mut()
            .filter(|r| !r.command.retirement_dependent())
        {
            let request = report.request;
            for member in &mut report.members {
                if !matches!(member.state, MemberState::Pending(_)) {
                    continue;
                }
                if let Some((condition, _)) =
                    first_block(report.command.needs(member.stage), conditions)
                {
                    member.state =
                        MemberState::Settled(Settlement::Failed(CommandFailure::Unavailable(
                            UnavailableTerminal::new(condition, request, member.scope),
                        )));
                    settled += 1;
                }
            }
        }
        self.file_settled();
        settled
    }

    /// For a pending member, the latched condition that blocks a capability
    /// its current stage still needs: what its status shows while a
    /// retirement-dependent command waits (§26 §3.2 row 3).
    pub fn blocked_by(
        &self,
        request: RequestId,
        subject: Subject,
        conditions: &ProcessConditions,
    ) -> Option<ConditionId> {
        let report = self.report(request)?;
        let member = report.member(subject)?;
        member.pending()?;
        first_block(report.command.needs(member.stage), conditions).map(|(condition, _)| condition)
    }

    /// Complete the IntentApplied members posted to `slot` at `generation`
    /// or earlier.
    fn applied(&mut self, slot: SlotKey, generation: IntentGeneration) -> usize {
        let mut settled = 0;
        for report in self
            .pending
            .iter_mut()
            .filter(|r| r.command.settles() == Settles::IntentApplied)
        {
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

    /// Fail the non-retirement members admitted on `slot` whose stage still
    /// needs `capability`; the owner's need for name resolution revokes
    /// the slot's resolved stages first.
    fn unavailable_on(
        &mut self,
        slot: SlotKey,
        capability: ProcessCapability,
        condition: ConditionId,
    ) -> usize {
        let mut settled = 0;
        for report in self
            .pending
            .iter_mut()
            .filter(|r| !r.command.retirement_dependent())
        {
            let request = report.request;
            for member in report
                .members
                .iter_mut()
                .filter(|m| m.on_slot(slot) && matches!(m.state, MemberState::Pending(_)))
            {
                if capability == ProcessCapability::NameResolution
                    && matches!(member.stage, Stage::AddressResolved(_))
                {
                    member.stage = Stage::Admitted;
                }
                if report.command.needs(member.stage).contains(&capability) {
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

    /// Check that `subject` of `request` may record its one post on `slot`,
    /// returning the command.
    fn postable(
        &self,
        request: RequestId,
        subject: Subject,
        slot: SlotKey,
    ) -> Result<OperatorCommand, CommandError> {
        let report = self
            .report(request)
            .ok_or(CommandError::UnknownRequest(request))?;
        let command = report.command;
        if command.intent().is_none() {
            return Err(CommandError::NoIntentRow(request));
        }
        let member = report
            .member(subject)
            .ok_or(CommandError::UnknownMember { request, subject })?;
        match member.state {
            MemberState::Settled(_) => Err(CommandError::AlreadySettled { request, subject }),
            MemberState::Pending(Pending::Posted { .. }) => {
                Err(CommandError::AlreadyPosted { request, subject })
            }
            MemberState::Pending(Pending::Accepted) if !member.on_slot(slot) => {
                Err(CommandError::WrongSlot {
                    request,
                    subject,
                    slot,
                })
            }
            MemberState::Pending(Pending::Accepted) => Ok(command),
        }
    }

    /// The pending member `subject` of `request`, or why there is none.
    fn locate_pending(
        &mut self,
        request: RequestId,
        subject: Subject,
    ) -> Result<&mut MemberReport, CommandError> {
        let report = match self.pending.iter_mut().find(|r| r.request == request) {
            Some(report) => report,
            None => self
                .settled
                .iter_mut()
                .find(|r| r.request == request)
                .ok_or(CommandError::UnknownRequest(request))?,
        };
        let member = report.member_mut(subject)?;
        match member.state {
            MemberState::Pending(_) => Ok(member),
            MemberState::Settled(_) => Err(CommandError::AlreadySettled { request, subject }),
        }
    }

    fn pending_member_mut(
        &mut self,
        request: RequestId,
        subject: Subject,
    ) -> Option<&mut MemberReport> {
        self.pending
            .iter_mut()
            .find(|r| r.request == request)?
            .members
            .iter_mut()
            .find(|m| m.subject == subject)
    }

    /// Move every report whose last member just settled into the settled
    /// history (in the order they opened) and evict the oldest settled
    /// reports beyond [`SETTLED_REPORT_CAP`]. Pending reports are never
    /// evicted.
    fn file_settled(&mut self) {
        let mut index = 0;
        while index < self.pending.len() {
            if self.pending[index].is_settled() {
                let report = self.pending.remove(index);
                self.settled.push_back(report);
            } else {
                index += 1;
            }
        }
        while self.settled.len() > SETTLED_REPORT_CAP {
            if let Some(evicted) = self.settled.pop_front() {
                let mut members = evicted.members;
                members.clear();
                if self.spare.len() < SPARE_MEMBER_LISTS {
                    self.spare.push(members);
                }
            }
        }
    }
}

/// The first latched condition affecting one of `needs`.
fn first_block(
    needs: &[ProcessCapability],
    conditions: &ProcessConditions,
) -> Option<(ConditionId, ProcessCapability)> {
    needs.iter().find_map(|&capability| {
        conditions
            .blocks(capability)
            .map(|condition: &ProcessServiceCondition| (condition.id(), capability))
    })
}

#[cfg(test)]
#[path = "command_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "command_census_tests.rs"]
mod census;
