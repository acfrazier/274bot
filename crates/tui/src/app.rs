//! `TuiApp`: view model for the headless panel. The binary (`tui-play`)
//! copies the shared projection (fleet rows, selected detail, resource
//! meter) and the selected bot's snapshot each frame, refreshes the app,
//! hands it keys and mouse events, and dispatches the returned
//! [`AppAction`] onto the shared operator session. This module owns the app
//! state and the per-pane behaviour (map, script, chat); `input` routes
//! events to it, `shell` draws the responsive layout and `overlay` owns
//! help, the palette and confirmations. The panes are plain widgets over
//! owned view data, so CI renders them with `TestBackend` and no real
//! terminal.

use std::collections::HashSet;
use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Paragraph, Widget, Wrap};
use ratatui::Frame;

use api::snapshot::{ChatLineView, ChatOptionView, WorldTile};
use frontend_core::quester_paths::QuesterPathsView;
use frontend_core::scripts::{
    LoadoutForm, Loadouts, OwnedSchema, PaintFrame, ScriptChoice, ScriptRunState,
};
use frontend_core::walk_map::{
    display_walk_name_into, WalkCatalogue, WalkMap, WalkObservedService, WalkSearch, WalkSlotStatus,
};
use frontend_core::{FleetCounts, OperatorCommand, ResourceView, SlotDetail, WalkGlobalsView};
use frontend_core::{MapBakeChoice, NavPreference};
use nav_planner::map::poi::{PoiKind, PoiRecord};
use nav_planner::router::{FindOptions, Route};
use nav_planner::tile::Tile;
use nav_planner::world::NavWorld;

use crate::chat::{chat_modal_open, Chat, ChatAction, ChatState, ChatView};
use crate::commands::{script_command, Command, Unavailable};
use crate::fleet::FleetState;
use crate::layout::{Pane, Regions, Screen};
use crate::loadouts::{LoadoutsPane, LoadoutsState};
use crate::map::{Map, MapAction, MapView, ObservedMark, ZOOMS};
use crate::overlay::Modal;
use crate::script_params::{ParamsCommit, ParamsKey, ParamsPane, ParamsState};
#[cfg(feature = "js")]
use crate::script_shape::rs2b0t_root_has_index;
use crate::script_shape::{card_selection, BrowseCard, ScriptClick, ScriptPane};
use crate::settings::{SettingsEdit, SettingsState};
use crate::status::StatusPane;

#[cfg(feature = "js")]
fn default_catalog_browse_dir() -> std::path::PathBuf {
    let home = frontend_core::scripts::paths::bot_home();
    if home.as_os_str() == "." {
        std::path::PathBuf::from("/")
    } else {
        home
    }
}

#[cfg(feature = "js")]
fn default_load_browse_dir(last: Option<&std::path::Path>) -> std::path::PathBuf {
    last.filter(|p| p.is_dir())
        .map(std::path::Path::to_path_buf)
        .unwrap_or_else(default_catalog_browse_dir)
}

#[cfg(feature = "js")]
#[derive(Debug, Clone, PartialEq, Eq)]
enum LoadEntry {
    Up,
    Subdir(String),
    File(String),
    Cancel,
}

/// Catalogue demand state. Drawing an inactive map never changes this state;
/// only the explicit Map pane transition may request the catalogue-only stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MapCatalogueStatus {
    #[default]
    Inactive,
    ReadingCache,
    DerivingPois,
    Ready,
    Unavailable,
}

/// WalkTo Send: focused bot or a Group checklist of fleet members.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WalkSendMode {
    Focused,
    Group,
}

/// One Walk-send checklist row: a loaded member and its host eligibility.
/// Whether it is checked is not stored here: it is eligible and marked in
/// the fleet model ([`WalkSendRow::checked`]).
#[derive(Debug, Clone)]
pub struct WalkSendRow {
    pub name: String,
    identity: frontend_core::ProfileIdentity,
    pub status: WalkSlotStatus,
}

impl WalkSendRow {
    /// Profile name for this send row.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Marked and able to walk.
    pub fn checked(&self, marks: &frontend_core::MarkedSelection) -> bool {
        self.status.is_eligible() && marks.contains(self.identity)
    }
}

#[derive(Debug, Clone)]
pub struct WalkSendState {
    pub mode: WalkSendMode,
    rows: Vec<WalkSendRow>,
    /// Moves whenever `rows` is replaced, so the label knows the
    /// eligibility changed.
    rows_version: u64,
    /// The Walk button label, rebuilt in place only when the mode, the
    /// rows or the marks moved ([`Self::sync_label`]); a draw borrows it.
    label: String,
    /// The mode, rows version and marks generation `label` was built from.
    label_key: Option<(WalkSendMode, u64, u64)>,
    /// Label rebuilds, so a test can prove a steady draw does none.
    #[cfg(test)]
    label_builds: u64,
}

impl Default for WalkSendState {
    fn default() -> Self {
        Self {
            mode: WalkSendMode::Focused,
            rows: Vec::new(),
            rows_version: 0,
            label: String::new(),
            label_key: None,
            #[cfg(test)]
            label_builds: 0,
        }
    }
}

impl WalkSendState {
    pub fn rows(&self) -> &[WalkSendRow] {
        &self.rows
    }

    /// Replace the Group rows (their eligibility moved).
    fn set_rows(&mut self, rows: Vec<WalkSendRow>) {
        self.rows = rows;
        self.rows_version = self.rows_version.wrapping_add(1);
    }

    /// Bring the Walk button label up to date: `Walk`, or `Walk N bots` in
    /// Group mode with the fleet model's `marks`, whose changes move
    /// `marks_generation`. Nothing is counted or written when the mode,
    /// the rows and the marks are unchanged, and a rebuild reuses the
    /// label's buffer.
    pub fn sync_label(&mut self, marks: &frontend_core::MarkedSelection, marks_generation: u64) {
        let key = (self.mode, self.rows_version, marks_generation);
        if self.label_key == Some(key) {
            return;
        }
        self.label_key = Some(key);
        self.label.clear();
        match self.mode {
            WalkSendMode::Focused => self.label.push_str("Walk"),
            WalkSendMode::Group => {
                use std::fmt::Write as _;
                let _ = write!(self.label, "Walk {} bots", self.checked_count(marks));
            }
        }
        #[cfg(test)]
        {
            self.label_builds += 1;
        }
    }

    /// The Walk button label as of the last [`Self::sync_label`].
    pub fn walk_label(&self) -> &str {
        &self.label
    }

    /// Rows that are marked and eligible.
    pub fn checked_count(&self, marks: &frontend_core::MarkedSelection) -> usize {
        self.rows.iter().filter(|row| row.checked(marks)).count()
    }
}

/// What a refusal on the message line waits for: the action that supplies
/// it clears the line at once (`TuiSession::clear_resolved_error`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorNeed {
    /// The precondition the reason names holds again
    /// ([`Unavailable::missing`] is false).
    Precondition(Unavailable),
    /// The profile called this is loaded (logged in).
    Loaded(String),
}

impl ErrorNeed {
    /// Whether the need holds in `app`.
    pub fn met(&self, app: &TuiApp) -> bool {
        match self {
            ErrorNeed::Precondition(reason) => !reason.missing(app),
            ErrorNeed::Loaded(name) => app
                .fleet_model
                .rows()
                .iter()
                .any(|row| row.name() == name && row.loaded()),
        }
    }
}

/// A command the app refuses before any submission: the session shows
/// `reason`, which clears when `need` holds, and logs the refusal at the
/// command boundary (the command, `bot` and `reason`). `command` is `None`
/// for a command that only changes the screen ([`Command::refusal`]): no
/// operator command is refused, so nothing is logged.
#[derive(Debug, PartialEq, Eq)]
pub struct Refusal {
    pub command: Option<OperatorCommand>,
    /// The bot the refused command named, if it named one.
    pub bot: Option<String>,
    pub reason: std::borrow::Cow<'static, str>,
    pub need: Option<ErrorNeed>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum AppAction {
    /// Quit the app.
    Quit,
    /// The app refuses a command before any submission (nothing to act on).
    /// The session shows the reason and logs a refused operator command at
    /// the command boundary.
    Refused(Refusal),
    /// Select `name` (mirrored onto the core `select`). Never a spawn or
    /// login; only Enter or a click on a fleet row, a context menu or the
    /// palette produce it.
    Focus(String),
    /// Activate the focused Map pane and request catalogue-only demand.
    MapOpen,
    /// Leave the Map pane and release map-owned resident state.
    MapClose,
    /// Map Walk-confirm: route `from` → tile and arm via `arm_walk_on`.
    ArmWalk(Tile),
    /// Group Walk-confirm: consume one destination plan for checked bots.
    MapWalkGroup,
    /// WASD one-tile walk: a direct one-step move on the focused slot.
    WalkTile(Tile),
    /// Local debug teleport is intentionally a separate named action. The
    /// binary/host must authorize it; the TUI never treats a pin as proof.
    MapTeleport(Tile),
    /// Reload the shared Quester Path registry without starting a Path.
    ReloadPaths,
    /// Dismiss the Quester Paths status line (`AcknowledgeNotice`).
    DismissPathsNotice,
    /// Mark or unmark `name`'s fleet row (`SetMark`): the row selection and
    /// the Walk-send group checklist.
    SetMark(String, bool),
    /// Mark these members' fleet rows (`MarkAll`): the eligible Walk-send
    /// rows.
    MarkAll(Vec<frontend_core::ProfileIdentity>),
    /// Clear every row mark (`ClearMarks`).
    ClearMarks,
    /// Mark, unmark or invert the shown rows, or mark the shown rows in a
    /// status (`MarkFleet`).
    MarkFleet(frontend_core::MarkOp),
    /// Make this the card the fleet actions use (`SetFleetCard`). Changes
    /// no Browse pick, form or run.
    PickFleetCard(frontend_core::FleetCard),
    /// Show this fleet order (`SetFleetSort`). The shown rows follow it and
    /// the cursor stays on its profile.
    SetFleetSort(frontend_core::FleetOrder),
    /// Open the world picker: `Some(name)` for that profile (`W` over the
    /// fleet row under the cursor), `None` for the marked rows (the
    /// palette's Switch world of marked…). The dispatcher reads the worlds
    /// from the core; a server with one world gets a message instead.
    PickWorld(Option<String>),
    /// Plan moving `profile` (`None`: the marked rows) to this public
    /// world: the dispatcher stages the profile, then plans
    /// `SwitchWorld` through the fleet plan flow.
    PlanSwitchWorld(Option<String>, u16),
    /// Move profiles in the saved Load order (`ReorderProfiles`): the
    /// dispatcher stages the op and submits it with the fleet model lent.
    ReorderProfiles(frontend_core::ReorderOp),
    /// Edit the fleet filter text and commit it (`SetFleetFilter`).
    FleetFilter(crate::fleet::FilterEdit),
    /// Save the rows the log view shows (`SaveLog`).
    SaveLog,
    /// Turn the session log file on or off (`SetSessionLog`).
    SetSessionLog(bool),
    /// Save the map's special-areas overlay choice (`SetNavPreference`).
    MapWilderness(bool),
    /// Chat modal advance: Continue / Answer on the focused slot.
    Chat(ChatAction),
    /// Log in the selected member (explicit handshake).
    Login,
    /// Log out the selected member (it stays loaded, latched).
    Logout,
    /// Relog the settings-bound member for a memory-mode switch (the
    /// settings popup's `r` key): the binary confirms first when a
    /// running script would be interrupted.
    MemoryRelog(String),
    /// Confirmed memory Relog-now: log out and back in through the FIFO.
    MemoryRelogNow(String),
    /// Remove this member from the fleet (clean logout, then stop). The
    /// name was frozen when the operator confirmed.
    Remove(String),
    /// Start the Browse-selected script on the focused slot.
    ScriptStart(ScriptChoice),
    /// Toggle pause/resume on the focused slot's script (`Play::script_pause`
    /// / [`Play::script_resume`], like the panel's `script_toggle_pause`).
    ScriptPause,
    /// Skip the active row in the focused Quester queue.
    ScriptSkipQuest,
    /// Stop the focused slot's script (`Play::script_stop`).
    ScriptStop,
    /// Open the script Browse picker (registry cards).
    ScriptBrowse,
    /// Open the script params popup for the selected card.
    ScriptParams,
    /// Freeze a fleet action into a core plan (`PlanFleetAction`): it runs
    /// at once when it needs no confirmation, otherwise the confirm
    /// overlay shows the plan's prompt.
    PlanFleet(frontend_core::FleetAction),
    /// Confirm: run the waiting fleet plan for this action (`Targets::Plan`).
    RunFleetPlan(frontend_core::FleetAction),
    /// Cancel: drop the waiting fleet plan (`CancelFleetPlan`).
    CancelFleetPlan,
    /// Reload the focused heading's card, or confirm a shown warning.
    #[cfg(feature = "js")]
    ScriptReload,
    /// Discard a prepared reload without touching any run.
    #[cfg(feature = "js")]
    ScriptReloadCancel,
    /// Prepare Apply to all for the params popup's card.
    ScriptSyncPrepare,
    /// Apply the prepared Apply to all.
    ScriptSyncApply,
    /// Drop the prepared Apply to all.
    ScriptSyncCancel,
    /// Restart only profiles whose saved settings need a new script run.
    ScriptRestartSettings,
    /// Leave the current runs active and defer these settings.
    ScriptRestartLater,
    /// Open the first-run rs2b0t catalog folder browser.
    #[cfg(feature = "js")]
    ScriptImportCatalog,
    /// Defer the rs2b0t catalog import (Not now).
    #[cfg(feature = "js")]
    ScriptDeferCatalog,
    /// Import catalog from the chosen clone root.
    #[cfg(feature = "js")]
    ScriptUseCatalog,
    /// Load the JS bot at `path` into the library and select it.
    #[cfg(feature = "js")]
    ScriptLoad(std::path::PathBuf),
    /// Persist the background-bots notice ("Got it, don't show again").
    AckBackground,
    /// Commit a changed TUI settings draft to the shared form-save model.
    SettingsDraftEdited(SettingsEdit),
    /// One step of the rs2b0t profile import; the binary owns the flow.
    #[cfg(feature = "js")]
    ProfileImport(crate::profile_import::ImportAction),
    /// Open the shared New profile form (`OpenNewProfileForm`); the
    /// overlay is already open, the binary opens the editor behind it.
    OpenNewProfile,
    /// A key for the New profile overlay: the binary owns the editor.
    NewProfileKey(KeyEvent),
    /// A key for the parameters popup: the binary owns the commit path.
    ParamsKey(KeyEvent),
    /// A key for the loadouts popup: the binary owns the form.
    LoadoutsKey(KeyEvent),
    /// Turn terminal mouse capture on or off (off lets the terminal
    /// select and copy text).
    MouseCapture(bool),
    /// Two actions from one input, in order (for example leaving the Map
    /// and opening Browse).
    Batch(Vec<AppAction>),
    /// Nothing to dispatch.
    None,
}

impl AppAction {
    /// `first` then `second`, dropping `None`s.
    pub fn batch(first: AppAction, second: AppAction) -> AppAction {
        match (first, second) {
            (AppAction::None, second) => second,
            (first, AppAction::None) => first,
            (first, second) => AppAction::Batch(vec![first, second]),
        }
    }
}

/// Parse the explicit `x,z,plane` form used by Map search/coordinate entry.
/// It is deliberately the same selection path as a centre/POI target.
fn parse_coordinate(value: &str) -> Option<Tile> {
    let mut fields = value.split(',').map(str::trim);
    let x = fields.next()?.parse().ok()?;
    let z = fields.next()?.parse().ok()?;
    let level = fields.next()?.parse().ok()?;
    if fields.next().is_some() {
        return None;
    }
    Some(Tile { x, z, level })
}

#[cfg(feature = "js")]
enum CatalogEntry {
    Up,
    Subdir(String),
    UseFolder,
    NotNow,
}

/// Adjacent world tile for a WASD step from `here`. +z is north on the
/// client's axis (the map's north-up camera — see the map pan tests), so
/// W is +z and S is -z; A is -x (west) and D is +x (east). Level is
/// carried through unchanged.
pub fn wasd_target(here: (i32, i32, i32), code: KeyCode) -> Option<(i32, i32, i32)> {
    match code {
        KeyCode::Char('w') | KeyCode::Char('W') => Some((here.0, here.1 + 1, here.2)),
        KeyCode::Char('s') | KeyCode::Char('S') => Some((here.0, here.1 - 1, here.2)),
        KeyCode::Char('a') | KeyCode::Char('A') => Some((here.0 - 1, here.1, here.2)),
        KeyCode::Char('d') | KeyCode::Char('D') => Some((here.0 + 1, here.1, here.2)),
        _ => None,
    }
}

/// The chat pane's owned data, copied from the focused snapshot each pump
/// (the ring is capped at 100 lines, so the copy is small).
#[derive(Debug, Default)]
pub struct ChatData {
    /// The public chat ring, newest first.
    pub lines: Vec<ChatLineView>,
    /// The chat modal's text pages.
    pub modal_texts: Vec<String>,
    /// The chat modal's BUTTON_OK choices.
    pub options: Vec<ChatOptionView>,
    /// A BUTTON_CONTINUE component is up.
    pub has_continue: bool,
    /// The focused slot's script paint frame (shared with the status row,
    /// which shares it with the isolate recorder); the pane shows it
    /// instead of the ring while it is non-empty.
    pub script_paint: Option<PaintFrame>,
    /// Operator toggle (`p`): show the game chat even while the script
    /// paints. Preserved across pumps (it is operator state, not a
    /// snapshot view).
    pub show_game_chat: bool,
}

impl ChatData {
    /// A chat modal is open when the snapshot shows dialog text, choices,
    /// or a Continue button.
    pub fn is_modal_open(&self) -> bool {
        chat_modal_open(&self.view())
    }

    pub(crate) fn view(&self) -> ChatView<'_> {
        ChatView {
            lines: &self.lines,
            modal_texts: &self.modal_texts,
            options: &self.options,
            has_continue: self.has_continue,
            script_paint: self.script_paint.as_ref().map(PaintFrame::view),
            show_game_chat: self.show_game_chat,
        }
    }
}

/// What the settings popup and the notices show beside a saved setting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingFeedback {
    /// An accepted write has not reached its terminal result.
    Pending,
    /// The retained failure the core holds for the setting.
    Failed(String),
}

/// The saved settings whose write the core holds pending. `all` stands for
/// a run that saves nothing (a live harness run): every draft stays.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct PendingSettings {
    pub(crate) all: bool,
    pub(crate) teleports: bool,
    pub(crate) wilderness: bool,
    pub(crate) bank_fetch: bool,
    pub(crate) danger: bool,
    pub(crate) scope_ack: bool,
    pub(crate) survivable_ack: bool,
    pub(crate) pause: bool,
    pub(crate) special_areas: bool,
    pub(crate) map_bake: bool,
    pub(crate) quester: bool,
    /// The saved Load order (it has no popup row: the message line says so).
    pub(crate) load_order: bool,
}

/// Headless panel view model.
pub struct TuiApp {
    title: String,
    /// Fleet members (load order) and the selected bot's index: the core
    /// `select`, not where the keyboard points (that is `key_focus`).
    pub names: Vec<String>,
    /// Stable vault identities parallel to `names`; fixture-only rows fall
    /// back to a deterministic synthetic identity in the fleet table.
    pub profile_ids: Vec<frontend_core::ProfileIdentity>,
    pub focused: Option<usize>,
    /// The fleet model: every profile (loaded members, then vault profiles
    /// that are not loaded), the marks, the filter and the shown rows. The
    /// binary refreshes it from the core once per loop and lends it to the
    /// command seam; the table and the inputs only read it.
    pub fleet_model: frontend_core::FleetModel,
    /// Profile identities whose saved script settings still need a restart.
    pub restart_badges: HashSet<frontend_core::ProfileIdentity>,
    /// Core badge generation used to refresh this set only when state moves.
    pub restart_badge_generation: u64,
    pub counts: FleetCounts,
    /// The selected slot's projected detail (the panel's status section
    /// shows the same).
    pub detail: Option<SlotDetail>,
    /// The focused slot's chat ring / dialogue, from the snapshot.
    pub chat_data: ChatData,
    /// The focused slot's inventory (name, count), from the snapshot.
    pub inv_items: Vec<(String, i32)>,
    /// The focused slot's used skill rows (name, level).
    pub stats_rows: Vec<(String, i32)>,
    /// The nearest named locs (Chebyshev from here, name).
    pub locs_near: Vec<(i32, String)>,
    /// The shared nav world the map routes and paints over.
    pub world: Option<Arc<NavWorld>>,
    /// Paint-only sidecar, requested only with the map reach layer enabled.
    pub map_reach: Option<Arc<[u64]>>,
    /// Drawing state for the map widget (pan, plane and optional layers).
    pub map: MapView,
    /// Shared host selection/action model. The TUI owns only this one
    /// application view, never a model per bot.
    pub map_model: WalkMap,
    pub map_active: bool,
    /// Search editor and bounded result indices into `map_pois`.
    pub map_search_open: bool,
    pub map_search: String,
    pub map_search_results: Vec<usize>,
    pub map_search_sel: usize,
    /// The shared compact catalogue projection. It is application-owned,
    /// never copied into each bot slot.
    pub map_pois: Vec<PoiRecord>,
    pub map_catalogue: Option<Arc<WalkCatalogue>>,
    map_search_index: WalkSearch,
    pub map_poi_sel: Option<usize>,
    pub map_catalogue_status: MapCatalogueStatus,
    pub map_coverage: String,
    /// Focused-bot observed NPC services while the map is open.
    pub map_observed: Vec<WalkObservedService>,
    /// Fleet Walk send: Focused vs Group checklist.
    pub walk_send: WalkSendState,
    /// The focused slot's observed world tile.
    pub here: Option<WorldTile>,
    /// The armed walk route whose remaining tiles paint `*`. Shared with the
    /// arm, so each observe is a reference-count bump, not a route copy.
    pub route: Option<Arc<Route>>,
    /// Armed Walk dest after the host accepts. Refused Walks leave this unset.
    pub walk_dest: Option<Tile>,
    /// Chat pane state (focused option row).
    pub chat: ChatState,
    /// Settings popup over the settings of the profile it is bound to
    /// ([`TuiApp::settings_profile`]); the binary persists
    /// [`TuiApp::settings`] back to the vault when
    /// [`TuiApp::settings_dirty`] flips.
    pub settings: frontend_core::SettingsDraft,
    /// Editable settings buffer; durable reads are rendered from `walk_permissions`.
    pub nav: frontend_core::WalkGlobals,
    /// Current durable projection shared with admission and inherited script rows.
    pub walk_permissions: WalkGlobalsView,
    /// Per-Map-open danger permission; never persisted.
    pub map_route_through_zones: bool,
    /// The shared one-time script-scope notice was dismissed.
    pub script_scope_notice_ack: bool,
    /// The shared one-time danger-routing migration notice was dismissed.
    pub survivable_routing_notice_ack: bool,
    /// Shared settings writes queued one nested nav preference at a time.
    pub nav_preferences_dirty: Vec<NavPreference>,
    pub settings_state: SettingsState,
    pub settings_dirty: bool,
    /// (or before the first pump binds it).
    pub settings_profile: Option<String>,
    /// The popup's title, naming its bound profile: built when it binds, so
    /// drawing only borrows it.
    pub settings_title: String,
    /// The popup's save feedback: an inline refusal, or `Saved <name>.`
    /// once its last persist is durable.
    pub settings_save: frontend_core::ProfileFormSave,
    /// Login-time vs effective mode for the focused slot; the status pane
    /// reads it. Copied from the core each pump.
    pub memory: Option<frontend_core::MemoryNotice>,
    /// Login-time vs effective mode for [`Self::settings_profile`]. The
    /// popup stays bound across focus changes, so this is deliberately
    /// separate from [`Self::memory`].
    pub settings_memory: Option<frontend_core::MemoryNotice>,
    /// Remembered WalkTo terrain-bake choice (shared `panel-ui.json` key).
    pub map_bake: MapBakeChoice,
    /// Shared host/folder source settings for Quester Paths.
    pub quester_paths: QuesterPathsView,
    /// Shared settings persistence, reload worker, and user-visible notice.
    pub(crate) quester_paths_controller: frontend_core::QuesterPathsController,
    /// The folder gate or path changed; the session persists it on pump.
    pub(crate) quester_paths_dirty: bool,
    /// The settings popup changed [`Self::map_bake`]; the binary persists it.
    pub map_bake_dirty: bool,
    /// Global nav preference: pause a script after its owned walk is
    /// cancelled by manual movement.
    pub pause_script_on_manual_walk_abort: bool,
    /// The global manual-movement pause preference changed; the session persists it.
    pub pause_script_on_manual_walk_abort_dirty: bool,
    shared_preferences: Option<frontend_core::UiPreferences>,
    /// Which saved settings have a write the core still holds pending
    /// (copied from the core each pump): their drafts stay the operator's.
    pending: PendingSettings,
    /// What the popup and the notices show beside a saved setting: pending
    /// or the retained failure (copied from the core each pump).
    pub(crate) setting_feedback: Vec<(frontend_core::Setting, SettingFeedback)>,
    preferences_adopted: bool,
    pub loadouts_state: LoadoutsState,
    /// The focused slot's script lifecycle (shape display only).
    pub script_state: ScriptRunState,
    /// Focused member has a Start still waiting (a paced place, a card
    /// preparation or an Assign & restart); Stop still applies.
    pub script_queued: bool,
    /// The focused profile's script heading; Start keys on `(source, name)`.
    pub script_sel: Option<ScriptChoice>,
    /// The operator picked a card in Browse since the last pump: the
    /// binary records it as the focused profile's pending selection.
    pub browse_changed: bool,
    /// A reload warning awaits confirmation: Reload reads "Confirm".
    #[cfg(feature = "js")]
    pub reload_confirm: bool,
    /// Registry cards the Browse picker lists (copied from the session each pump).
    pub script_cards: Vec<BrowseCard>,
    /// The core picker choice of each of `script_cards` (same order): what
    /// the fleet-card picker pins. A card's choice names its identity, not
    /// only its display name.
    pub script_card_choices: Vec<ScriptChoice>,
    /// Persisted category order keys for Browse grouping.
    pub script_category_order: Vec<String>,
    /// The Browse picker is open.
    pub script_browse_open: bool,
    /// First-run rs2b0t clone-root folder browser.
    #[cfg(feature = "js")]
    pub rs2b0t_catalog_open: bool,
    #[cfg(feature = "js")]
    pub rs2b0t_catalog_dir: std::path::PathBuf,
    /// Highlight row in the catalog folder browser.
    #[cfg(feature = "js")]
    pub catalog_sel: usize,
    /// The Load file browser is open.
    #[cfg(feature = "js")]
    pub script_load_open: bool,
    #[cfg(feature = "js")]
    pub script_load_dir: std::path::PathBuf,
    #[cfg(feature = "js")]
    pub script_load_sel: usize,
    #[cfg(feature = "js")]
    pub script_load_last_dir: Option<std::path::PathBuf>,
    /// Selected card's settings schema (refreshed each pump from the library).
    pub params_schema: OwnedSchema,
    /// Missing metadata or an unsupported saved native schema is not empty.
    pub params_unavailable: Option<&'static str>,
    /// Working bag while the params popup is open.
    pub params_bag: serde_json::Map<String, serde_json::Value>,
    pub params_state: ParamsState,
    pub quit: bool,
    /// The last report or error, shown on the message line.
    pub error: Option<String>,
    /// The process resource meter (the core's one sampler, as the panel's
    /// resource card shows it).
    pub resources: ResourceView,
    /// One-line background-bots notice until the operator acks it.
    pub background_notice: Option<String>,
    /// Shared host-capacity warning from frontend-core, drawn as a wrapped
    /// banner above the header on every screen.
    pub capacity: Option<String>,
    /// The shared structured log (Logs tab and the log drawer).
    pub log: crate::log_pane::LogPaneState,
    /// The detail tab shown for the selected bot.
    pub screen: Screen,
    /// Where keys go: the fleet, the detail tab or the log drawer.
    pub key_focus: Pane,
    /// Fleet table cursor, scroll and filter-edit flag (renderer-local).
    pub table: FleetState,
    /// The open app overlay (help, palette, confirmation, menu, message,
    /// manual walk); it takes every key and click while open.
    pub modal: Option<Modal>,
    /// Terminal mouse capture is on (the binary applies changes).
    pub mouse_capture: bool,
    /// Hit regions of the last draw.
    pub regions: Regions,
    /// Last draw rects for chat and script clicks (empty when not drawn).
    pub chat_area: Rect,
    pub script_area: Rect,
}

impl TuiApp {
    /// New app showing `title`; no slots, no map, no snapshot.
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            names: Vec::new(),
            profile_ids: Vec::new(),
            focused: None,
            fleet_model: frontend_core::FleetModel::new(),
            counts: FleetCounts::default(),
            restart_badges: HashSet::new(),
            restart_badge_generation: 0,
            detail: None,
            chat_data: ChatData::default(),
            inv_items: Vec::new(),
            stats_rows: Vec::new(),
            locs_near: Vec::new(),
            world: None,
            map_reach: None,
            map: MapView::new(),
            map_model: WalkMap::new(),
            map_active: false,
            here: None,
            map_search_open: false,
            map_search: String::new(),
            map_search_results: Vec::new(),
            map_search_sel: 0,
            map_pois: Vec::new(),
            map_catalogue: None,
            map_search_index: WalkSearch::default(),
            map_poi_sel: None,
            map_catalogue_status: MapCatalogueStatus::Inactive,
            map_coverage: "coverage: unavailable until Map is opened".into(),
            map_observed: Vec::new(),
            walk_send: WalkSendState::default(),
            route: None,
            walk_dest: None,
            chat: ChatState::default(),
            settings: frontend_core::SettingsDraft::default(),
            nav: frontend_core::WalkGlobals::default(),
            walk_permissions: WalkGlobalsView::default(),
            map_route_through_zones: false,
            script_scope_notice_ack: false,
            survivable_routing_notice_ack: false,
            nav_preferences_dirty: Vec::new(),
            settings_state: SettingsState::default(),
            settings_dirty: false,
            settings_profile: None,
            settings_title: crate::settings::TITLE.to_string(),
            settings_save: frontend_core::ProfileFormSave::default(),
            memory: None,
            settings_memory: None,
            map_bake: MapBakeChoice::Ask,
            quester_paths: QuesterPathsView::default(),
            quester_paths_controller: frontend_core::QuesterPathsController::default(),
            quester_paths_dirty: false,
            pause_script_on_manual_walk_abort: true,
            pause_script_on_manual_walk_abort_dirty: false,
            shared_preferences: None,
            pending: PendingSettings::default(),
            setting_feedback: Vec::new(),
            preferences_adopted: false,
            map_bake_dirty: false,
            loadouts_state: LoadoutsState::default(),
            script_state: ScriptRunState::Idle,
            script_queued: false,
            script_sel: None,
            browse_changed: false,
            #[cfg(feature = "js")]
            reload_confirm: false,
            script_cards: Vec::new(),
            script_card_choices: Vec::new(),
            script_category_order: Vec::new(),
            script_browse_open: false,
            #[cfg(feature = "js")]
            rs2b0t_catalog_open: false,
            #[cfg(feature = "js")]
            rs2b0t_catalog_dir: default_catalog_browse_dir(),
            #[cfg(feature = "js")]
            catalog_sel: 0,
            #[cfg(feature = "js")]
            script_load_open: false,
            #[cfg(feature = "js")]
            script_load_dir: default_load_browse_dir(None),
            #[cfg(feature = "js")]
            script_load_sel: 0,
            #[cfg(feature = "js")]
            script_load_last_dir: None,
            params_schema: OwnedSchema::default(),
            params_unavailable: None,
            params_bag: serde_json::Map::new(),
            params_state: ParamsState::default(),
            quit: false,
            error: None,
            resources: ResourceView::default(),
            background_notice: None,
            capacity: None,
            log: crate::log_pane::LogPaneState::default(),
            screen: Screen::Overview,
            key_focus: Pane::Fleet,
            table: FleetState::default(),
            modal: None,
            mouse_capture: true,
            regions: Regions::default(),
            chat_area: Rect::default(),
            script_area: Rect::default(),
        }
    }

    /// Bind the shared process-service preferences. Every draft follows the
    /// durable value again on each refresh (for every relevant render and
    /// admission), except while a change of its own is unsubmitted or its
    /// write is pending ([`Self::set_pending_settings`]): a failed write
    /// puts the durable value back at its setting.
    pub fn restore_preferences(&mut self, preferences: frontend_core::UiPreferences) {
        self.shared_preferences = Some(preferences.share());
        if !preferences.ready() {
            let view = WalkGlobalsView::read(&preferences);
            self.walk_permissions = view;
            self.nav = view.globals();
            return;
        }
        self.preferences_adopted = true;
        self.quester_paths_controller.restore(&preferences);
        self.follow_saved_drafts(&preferences);
        self.refresh_walk_permissions();
    }

    /// Which saved settings have a write the core still holds pending (or
    /// every one of them, for a run that saves nothing): their drafts are
    /// the operator's change, not yet the durable value.
    pub(crate) fn set_pending_settings(&mut self, pending: PendingSettings) {
        self.pending = pending;
    }

    /// Whether the saved Load order is being written: the message line
    /// says so while it is the only thing to say.
    pub(crate) fn load_order_saving(&self) -> bool {
        self.pending.load_order
    }

    /// Whether the draft of `preference` is held: unsubmitted, or its write
    /// is pending.
    pub(crate) fn holds(&self, preference: NavPreference) -> bool {
        let pending = match preference {
            NavPreference::AllowTeleports => self.pending.teleports,
            NavPreference::AllowWilderness => self.pending.wilderness,
            NavPreference::AllowBankFetch => self.pending.bank_fetch,
            NavPreference::AllowDangerZones | NavPreference::SurvivableRouting => {
                self.pending.danger
            }
            NavPreference::ScriptScopeNoticeAck => self.pending.scope_ack,
            NavPreference::SurvivableRoutingNoticeAck => self.pending.survivable_ack,
            NavPreference::PauseScriptOnManualWalkAbort => {
                self.pending.pause || self.pause_script_on_manual_walk_abort_dirty
            }
            NavPreference::ShowSpecialAreas => self.pending.special_areas,
        };
        let dirty = match preference {
            NavPreference::AllowDangerZones | NavPreference::SurvivableRouting => {
                self.nav_preferences_dirty.iter().any(|p| {
                    matches!(
                        p,
                        NavPreference::AllowDangerZones | NavPreference::SurvivableRouting
                    )
                })
            }
            other => self.nav_preferences_dirty.contains(&other),
        };
        self.pending.all || pending || dirty
    }

    /// Drafts of the saved settings that are not walk permissions follow
    /// their durable values: the manual-walk pause, the map-bake choice, the
    /// special-areas overlay and the Quester source (the controller holds
    /// the source the core adopted from the snapshot).
    fn follow_saved_drafts(&mut self, preferences: &frontend_core::UiPreferences) {
        if !(self.pending.all || self.pending.quester || self.quester_paths_dirty) {
            let settings = self.quester_paths_controller.settings();
            let enabled = settings.enabled();
            let follow_folder =
                !self.settings_state.folder_changed && !self.settings_state.folder_editing;
            let folder_differs = self.quester_paths.folder() != settings.folder();
            let folder = (follow_folder && folder_differs).then(|| settings.folder().to_path_buf());
            self.quester_paths.set_enabled(enabled);
            if let Some(folder) = folder {
                self.quester_paths.set_folder(folder);
            }
        }
        if !self.holds(NavPreference::PauseScriptOnManualWalkAbort) {
            self.pause_script_on_manual_walk_abort = frontend_core::nav_preference(
                preferences,
                NavPreference::PauseScriptOnManualWalkAbort,
            )
            .unwrap_or(true);
        }
        if !(self.pending.all || self.pending.map_bake || self.map_bake_dirty) {
            self.map_bake = frontend_core::load_map_bake_choice(preferences);
        }
        if !self.holds(NavPreference::ShowSpecialAreas) {
            self.map.restore_persisted_wilderness(preferences);
        }
    }

    /// Refresh all TUI views of the shared walk permissions from their
    /// durable source. An unbound app uses its local values for isolated
    /// widgets; production binds and rereads the shared preferences (cheap
    /// per pump: the service re-reads the file at most once per second).
    pub(crate) fn refresh_walk_permissions(&mut self) {
        if !self.preferences_adopted {
            if let Some(preferences) = self
                .shared_preferences
                .as_ref()
                .filter(|prefs| prefs.ready())
            {
                self.restore_preferences(preferences.share());
            }
        }
        let view = self.shared_preferences.as_ref().map_or(
            WalkGlobalsView::new(
                self.nav,
                self.script_scope_notice_ack,
                self.survivable_routing_notice_ack,
            ),
            WalkGlobalsView::read,
        );
        if !self.preferences_adopted && self.shared_preferences.is_some() {
            self.walk_permissions = view;
        } else {
            self.adopt_walk_permissions(view);
            if let Some(preferences) = self
                .shared_preferences
                .as_ref()
                .filter(|prefs| prefs.ready())
                .map(frontend_core::UiPreferences::share)
            {
                self.follow_saved_drafts(&preferences);
            }
        }
    }

    /// Adopt `view`, the durable walk permissions the core just read, into
    /// every TUI view of them (the edit buffer, the acknowledgements, the
    /// one-shot zone grant and an open parameters popup). A draft the
    /// operator changed whose write is not settled stays; every other draft
    /// is the durable value, so a failed write shows the saved value again.
    pub(crate) fn adopt_walk_permissions(&mut self, view: WalkGlobalsView) {
        let next = view.globals();
        let keep = |held: bool, draft: bool, next: bool| if held { draft } else { next };
        self.nav = frontend_core::WalkGlobals::new(
            keep(
                self.holds(NavPreference::AllowTeleports),
                self.nav.allow_teleports(),
                next.allow_teleports(),
            ),
            keep(
                self.holds(NavPreference::AllowWilderness),
                self.nav.allow_wilderness(),
                next.allow_wilderness(),
            ),
            keep(
                self.holds(NavPreference::AllowBankFetch),
                self.nav.allow_bank_fetch(),
                next.allow_bank_fetch(),
            ),
            keep(
                self.holds(NavPreference::AllowDangerZones),
                self.nav.allow_danger_zones(),
                next.allow_danger_zones(),
            ),
            keep(
                self.holds(NavPreference::SurvivableRouting),
                self.nav.survivable_routing(),
                next.survivable_routing(),
            ),
        );
        self.script_scope_notice_ack = keep(
            self.holds(NavPreference::ScriptScopeNoticeAck),
            self.script_scope_notice_ack,
            view.script_scope_notice_ack(),
        );
        self.survivable_routing_notice_ack = keep(
            self.holds(NavPreference::SurvivableRoutingNoticeAck),
            self.survivable_routing_notice_ack,
            view.survivable_routing_notice_ack(),
        );
        self.walk_permissions = view;
        if !view.danger_this_walk(true) {
            self.map_route_through_zones = false;
        }
        if self.params_state.open {
            self.params_state.walk_permissions = view;
        }
    }

    pub(crate) fn shared_preferences(&self) -> Option<&frontend_core::UiPreferences> {
        self.shared_preferences.as_ref()
    }

    /// What is shown beside `setting`: `None` once it is saved.
    pub fn setting_feedback(&self, setting: frontend_core::Setting) -> Option<&SettingFeedback> {
        self.setting_feedback
            .iter()
            .find(|(shown, _)| *shown == setting)
            .map(|(_, feedback)| feedback)
    }

    /// Show `status` beside `setting` (the core's state of it). Allocates
    /// only when the failure text changed.
    pub(crate) fn show_setting_status(
        &mut self,
        setting: frontend_core::Setting,
        status: frontend_core::SettingStatus<'_>,
    ) {
        let position = self
            .setting_feedback
            .iter()
            .position(|(shown, _)| *shown == setting);
        let next = match status {
            frontend_core::SettingStatus::Saved => {
                if let Some(position) = position {
                    self.setting_feedback.swap_remove(position);
                }
                return;
            }
            frontend_core::SettingStatus::Pending => SettingFeedback::Pending,
            frontend_core::SettingStatus::Failed(text) => {
                if let Some(position) = position {
                    if matches!(&self.setting_feedback[position].1, SettingFeedback::Failed(shown) if shown == text)
                    {
                        return;
                    }
                }
                SettingFeedback::Failed(text.to_owned())
            }
        };
        match position {
            Some(position) => self.setting_feedback[position].1 = next,
            None => self.setting_feedback.push((setting, next)),
        }
    }

    /// Flip the map's special-areas overlay. Saving the choice (shared with
    /// the panel) is the `SetNavPreference` the returned action submits.
    pub(crate) fn toggle_map_wilderness(&mut self) -> AppAction {
        self.map.layers.wilderness = !self.map.layers.wilderness;
        AppAction::MapWilderness(self.map.layers.wilderness)
    }

    /// The header title (profile, server and revision).
    pub fn title(&self) -> &str {
        &self.title
    }

    /// The selected slot's projected detail, `None` when nothing is
    /// selected (or the projection has not caught up with the selection).
    pub fn focused_detail(&self) -> Option<&SlotDetail> {
        let name = self.focused_name()?;
        self.detail
            .as_ref()
            .filter(|detail| detail.row().name() == name)
    }

    /// The focused slot's username.
    pub fn focused_name(&self) -> Option<String> {
        self.focused.and_then(|i| self.names.get(i)).cloned()
    }
    /// Whether a bot has the focus ([`Self::focused_name`] without the copy).
    pub(crate) fn has_focus(&self) -> bool {
        self.focused.is_some_and(|i| i < self.names.len())
    }
    /// Whether Log out acts on the focused bot: the shared projection's
    /// answer from its fleet row, the same one Fleet Log out uses.
    pub(crate) fn focused_logout_eligible(&self) -> bool {
        self.focused
            .and_then(|i| self.names.get(i))
            .and_then(|name| {
                let index = self.fleet_model.index_of_name(name)?;
                self.fleet_model.rows().get(index)
            })
            .is_some_and(|row| row.logout_eligibility().eligible())
    }
    pub(crate) fn profile_id_for_name(&self, name: &str) -> frontend_core::ProfileIdentity {
        frontend_core::profile_id_for_name(&self.names, &self.profile_ids, name)
    }
    /// Whether the profile called `name` (loaded or not) is marked.
    pub(crate) fn is_marked_name(&self, name: &str) -> bool {
        self.fleet_model
            .index_of_name(name)
            .is_some_and(|index| self.fleet_model.is_marked(index))
    }

    /// The identity of the focused member, for the table's `*` marker.
    pub(crate) fn focused_identity(&self) -> Option<frontend_core::ProfileIdentity> {
        self.focused
            .and_then(|index| frontend_core::profile_id_at(&self.profile_ids, &self.names, index))
    }

    /// Re-sync the view from the fresh projection: the selected slot's
    /// `here` tile (the map re-centres on it when the view is not panned).
    pub fn refresh(&mut self) {
        self.here = self
            .focused_detail()
            .and_then(|detail| detail.ready_tile())
            .map(|(x, z, level)| WorldTile { x, z, level });
        if !self.map_active {
            if let Some(here) = self.here {
                self.map.plane = here.level.clamp(0, 3) as u8;
            }
        }
    }

    /// The Chat tab's keys (only while it has keyboard focus): Space/Enter
    /// → `continue_dialog` / `answer_choice`, Up/Down (j/k) move the option
    /// focus; with script paint, digits and Enter press paint buttons.
    pub(crate) fn chat_on_key(&mut self, key: KeyEvent) -> Option<AppAction> {
        let mut chat = Chat::new(self.chat_data.view(), &mut self.chat, |_| {});
        match chat.on_key(key) {
            action @ (ChatAction::Continue
            | ChatAction::Answer(_)
            | ChatAction::PaintButton(_)
            | ChatAction::PaintChrome(_)) => Some(AppAction::Chat(action)),
            ChatAction::None => None,
        }
    }

    /// A click inside the drawn chat pane: answer, continue or a paint
    /// button / chrome row.
    pub(crate) fn chat_click(&mut self, col: u16, row: u16) -> AppAction {
        let area = self.chat_area;
        let mut chat = Chat::new(self.chat_data.view(), &mut self.chat, |_| {});
        match chat.on_click(area, col, row) {
            action @ (ChatAction::Continue
            | ChatAction::Answer(_)
            | ChatAction::PaintButton(_)
            | ChatAction::PaintChrome(_)) => AppAction::Chat(action),
            ChatAction::None => AppAction::None,
        }
    }

    /// The map pane's keys: pan (arrows/hjkl), zoom (+/-), Enter selects /
    /// confirms a walk, Esc clears the selection.
    fn map_on_key(&mut self, key: KeyEvent) -> AppAction {
        let Some(world) = self.world.clone() else {
            return AppAction::None;
        };
        let mut map = Map::new(&world, &mut self.map, |_| {})
            .pois(&self.map_pois)
            .selected_poi(self.map_poi_sel);
        if let Some(here) = self.here {
            map = map.here(here);
        }
        if let Some(route) = &self.route {
            map = map.route(route);
        }
        let outcome = map.on_key(key);
        if key.code == KeyCode::Esc {
            self.map_model.clear_selection();
        }
        match outcome {
            MapAction::Walk(tile) => AppAction::ArmWalk(tile),
            MapAction::Moved | MapAction::Ignored => AppAction::None,
        }
    }

    fn update_map_search(&mut self) {
        if let Some(catalogue) = self.map_catalogue.clone() {
            if self.map_search.is_empty() {
                self.map_search_index.clear();
                self.map_search_results.clear();
            } else if self
                .map_search_index
                .update(&catalogue, &self.map_search)
                .is_ok()
            {
                self.map_search_results = self.map_search_index.results().to_vec();
            }
        } else {
            let needle = self.map_search.to_ascii_lowercase();
            self.map_search_results.clear();
            if !needle.is_empty() {
                self.map_search_results.extend(
                    self.map_pois
                        .iter()
                        .enumerate()
                        .filter(|(_, poi)| poi.name.as_str().to_ascii_lowercase().contains(&needle))
                        .map(|(index, _)| index),
                );
            }
        }
        self.map_search_sel = self
            .map_search_sel
            .min(self.map_search_results.len().saturating_sub(1));
    }

    pub(crate) fn search_hit_label(&self, index: usize) -> Option<String> {
        fn kind_glyph(kind: PoiKind) -> &'static str {
            match kind {
                PoiKind::Bank => "B",
                PoiKind::Transport => "T",
                PoiKind::Teleport => "X",
                _ => "·",
            }
        }
        if let Some(catalogue) = &self.map_catalogue {
            let entry = catalogue.entry(index)?;
            let anchor = entry.anchor();
            return Some(format!(
                "{} {} ({},{},{})",
                kind_glyph(entry.kind()),
                entry.display_name(),
                anchor.x,
                anchor.z,
                anchor.level
            ));
        }
        let poi = self.map_pois.get(index)?;
        let mut name = String::with_capacity(poi.name.as_str().len());
        display_walk_name_into(poi.name.as_str(), &mut name);
        Some(format!(
            "{} {} ({},{},{})",
            kind_glyph(poi.kind),
            name,
            poi.display.x as i32,
            poi.display.z as i32,
            poi.effective_plane
        ))
    }

    fn jump_to_poi(&mut self, index: usize) {
        let (bx, bz) = self
            .here
            .map(|h| (h.x, h.z))
            .unwrap_or(crate::map::DEFAULT_CENTRE);
        if let Some(catalogue) = self.map_catalogue.clone() {
            if self.map_model.select_poi(&catalogue, index).is_ok() {
                if let Some(entry) = catalogue.entry(index) {
                    let anchor = entry.anchor();
                    self.map.pan = (anchor.x - bx, anchor.z - bz);
                    if (0..4).contains(&anchor.level) {
                        self.map.plane = anchor.level as u8;
                    }
                    self.map.selection = Some(anchor);
                    self.map_poi_sel = Some(index);
                    self.error = None;
                }
            }
            return;
        }
        let Some(poi) = self.map_pois.get(index) else {
            return;
        };
        self.map.pan = (
            poi.display.x.floor() as i32 - bx,
            poi.display.z.floor() as i32 - bz,
        );
        self.map.plane = poi.effective_plane;
        self.map.selection = None;
        self.map_model.clear_selection();
        self.map_poi_sel = Some(index);
    }

    pub(crate) fn map_search_on_key(&mut self, key: KeyEvent) -> AppAction {
        match key.code {
            KeyCode::Esc => {
                self.map_search_open = false;
                self.map_search.clear();
                self.map_search_results.clear();
                self.map_search_sel = 0;
            }
            KeyCode::Backspace => {
                self.map_search.pop();
                self.update_map_search();
            }
            KeyCode::Up => {
                self.map_search_sel = self.map_search_sel.saturating_sub(1);
            }
            KeyCode::Down => {
                if self.map_search_sel + 1 < self.map_search_results.len() {
                    self.map_search_sel += 1;
                }
            }
            KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.map_search_sel = self.map_search_sel.saturating_sub(1);
            }
            KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if self.map_search_sel + 1 < self.map_search_results.len() {
                    self.map_search_sel += 1;
                }
            }
            KeyCode::Enter => {
                if let Some(&index) = self.map_search_results.get(self.map_search_sel) {
                    self.jump_to_poi(index);
                } else if let Some(tile) = parse_coordinate(&self.map_search) {
                    self.select_requested(tile);
                }
                self.map_search_open = false;
            }
            KeyCode::Char(c) if !c.is_control() && self.map_search.len() < 256 => {
                self.map_search.push(c);
                self.update_map_search();
            }
            _ => {}
        }
        AppAction::None
    }

    pub(crate) fn select_requested(&mut self, requested: Tile) {
        self.map.selection = Some(requested);
        if (0..4).contains(&requested.level) {
            self.map.plane = requested.level as u8;
        }
        if let Some(world) = self.world.clone() {
            self.map_model.select_tile(&world, requested);
        }
        self.error = None;
    }

    /// Enter confirms only an existing pending selection. With none, it selects
    /// the view-centre tile through the shared model and does not dispatch.
    pub(crate) fn map_enter(&mut self) -> AppAction {
        if let Some(pending) = self.map_model.pending() {
            if self.walk_send.mode == WalkSendMode::Group {
                let reason = Unavailable::NoCheckedBots;
                if reason.missing(self) {
                    return AppAction::Refused(Refusal {
                        command: Some(OperatorCommand::GroupWalk),
                        bot: None,
                        reason: reason.text().into(),
                        need: reason.need(),
                    });
                }
                return AppAction::MapWalkGroup;
            }
            return AppAction::ArmWalk(pending.requested());
        }
        self.select_view_centre();
        AppAction::None
    }

    fn select_view_centre(&mut self) {
        let Some(world) = self.world.clone() else {
            return;
        };
        let (bx, bz) = self
            .here
            .map(|h| (h.x, h.z))
            .unwrap_or(crate::map::DEFAULT_CENTRE);
        let requested = Tile {
            x: bx + self.map.pan.0,
            z: bz + self.map.pan.1,
            level: i32::from(self.map.plane),
        };
        if !(0..4).contains(&requested.level) {
            return;
        }
        self.map.selection = Some(requested);
        self.map_model.select_tile(&world, requested);
        self.error = None;
    }

    /// Host confirm consumes the pending selection; drop the leftover crosshair
    /// so a later Enter cannot radius-snap the old anchor in the same press.
    pub(crate) fn clear_consumed_map_selection(&mut self) {
        self.map.selection = None;
        self.map_poi_sel = None;
    }

    pub(crate) fn set_map_plane(&mut self, plane: u8) {
        let plane = plane.min(3);
        self.map.plane = plane;
        self.map.selection = None;
        self.map_poi_sel = None;
        let _ = self.map_model.set_plane(plane);
        self.map_model.clear_selection();
    }

    pub(crate) fn recenter_map(&mut self) {
        self.map.pan = (0, 0);
        self.map.selection = None;
        self.map_poi_sel = None;
        let observed = self
            .here
            .filter(|h| (0..4).contains(&h.level))
            .map(|h| Tile {
                x: h.x,
                z: h.z,
                level: h.level,
            });
        self.map_model.recenter(observed);
        self.map.plane = self.map_model.plane();
    }

    /// Group-walk rows: every member with its host eligibility. The fleet
    /// model's marks are the group checklist: a row is checked when its
    /// member is marked and eligible ([`WalkSendRow::checked`]).
    pub fn refresh_walk_send(&mut self, eligibility: impl Fn(&str) -> WalkSlotStatus) {
        let names: Vec<String> = if !self.names.is_empty() {
            self.names.clone()
        } else {
            self.fleet_model
                .rows()
                .iter()
                .filter(|row| row.loaded())
                .map(|row| row.name().to_owned())
                .collect()
        };
        let mut rows = Vec::with_capacity(names.len());
        for name in names {
            let status = eligibility(&name);
            let identity = self.profile_id_for_name(&name);
            rows.push(WalkSendRow {
                name,
                identity,
                status,
            });
        }
        self.walk_send.set_rows(rows);
    }

    /// Bring the Walk button label up to date with the fleet model's
    /// marks (nothing to do while they, the rows and the mode are
    /// unchanged), then borrow it.
    pub fn sync_walk_label(&mut self) -> &str {
        self.walk_send
            .sync_label(self.fleet_model.marks(), self.fleet_model.generation());
        self.walk_send.walk_label()
    }

    /// Focused ↔ Group. Entering Group with no eligible row marked marks
    /// the bots a fleet action covers while nothing is marked (the loaded,
    /// shown members) that can walk, shown as `[x]` in the fleet: the
    /// returned `MarkAll` marks them. A member the fleet filter hides is not
    /// added.
    pub(crate) fn toggle_walk_send_mode(&mut self) -> AppAction {
        self.walk_send.mode = match self.walk_send.mode {
            WalkSendMode::Focused => WalkSendMode::Group,
            WalkSendMode::Group => WalkSendMode::Focused,
        };
        if self.walk_send.mode == WalkSendMode::Group
            && self.walk_send.checked_count(self.fleet_model.marks()) == 0
        {
            let eligible_ids = self
                .walk_send
                .rows
                .iter()
                .filter(|row| {
                    row.status.is_eligible() && self.fleet_model.in_unmarked_scope(row.identity)
                })
                .map(|row| row.identity)
                .collect::<Vec<_>>();
            if !eligible_ids.is_empty() {
                return AppAction::MarkAll(eligible_ids);
            }
        }
        AppAction::None
    }

    /// Space on the map: mark / unmark the selected bot for the group: the
    /// returned `SetMark` marks or unmarks it.
    pub(crate) fn toggle_walk_send_focused(&mut self) -> AppAction {
        let Some(focused) = self.focused_name() else {
            return AppAction::None;
        };
        match self.walk_send.rows.iter().find(|row| row.name() == focused) {
            Some(row) if row.status.is_eligible() => {
                let on = !self.fleet_model.marks().contains(row.identity);
                AppAction::SetMark(focused, on)
            }
            _ => AppAction::None,
        }
    }

    fn observed_marks(&self) -> Vec<ObservedMark> {
        self.map_observed
            .iter()
            .map(|obs| ObservedMark {
                x: obs.tile().x,
                z: obs.tile().z,
                level: obs.tile().level,
            })
            .collect()
    }

    pub(crate) fn map_open(&mut self) -> AppAction {
        self.map_active = true;
        self.map_route_through_zones = false;
        if self.map_catalogue_status == MapCatalogueStatus::Inactive {
            self.map_catalogue_status = MapCatalogueStatus::Unavailable;
            self.map_coverage =
                "coverage: catalogue unavailable: cache demand manager is not bound; terrain imagery unavailable"
                    .into();
        }
        AppAction::MapOpen
    }

    pub(crate) fn map_close(&mut self) -> AppAction {
        self.map_active = false;
        self.map_route_through_zones = false;
        self.map_search_open = false;
        self.map_search.clear();
        self.map_search_results.clear();
        self.map.selection = None;
        self.map_model.close();
        self.map_poi_sel = None;
        self.map_pois.clear();
        self.map_catalogue = None;
        self.map_search_index.clear();
        self.map_observed.clear();
        self.walk_send = WalkSendState::default();
        self.map_catalogue_status = MapCatalogueStatus::Inactive;
        self.map_coverage = "coverage: unavailable until Map is opened".into();
        AppAction::MapClose
    }
    /// Resolve fresh durable globals and the one-shot permission through
    /// the shared projection used to render the same controls.
    pub fn map_find_options(&mut self) -> FindOptions {
        self.refresh_walk_permissions();
        self.walk_permissions
            .manual_options(self.map_route_through_zones)
    }

    /// Shared model adapters may publish a ready catalogue after activation.
    /// This does not decode PNGs and is intentionally separate from `draw`.
    pub fn set_map_catalogue(&mut self, pois: Vec<PoiRecord>, coverage: impl Into<String>) {
        self.map_pois = pois;
        self.map_coverage = coverage.into();
        self.map_catalogue_status = MapCatalogueStatus::Ready;
        self.update_map_search();
    }

    /// Retain the frontend-owned catalogue; draw and search borrow it.
    pub fn bind_walk_catalogue(&mut self, catalogue: Arc<WalkCatalogue>) {
        let messages: Vec<_> = catalogue.coverage_messages().collect();
        let coverage = if messages.is_empty() {
            format!(
                "coverage: client {:?}  navpois {:?}",
                catalogue.client_status(),
                catalogue.service_status()
            )
        } else {
            format!("coverage: {}", messages.join("; "))
        };
        if let Some(context) = self.map_model.context() {
            self.map_model
                .bind(context.with_overlay(Some(catalogue.key())));
        }
        self.map_catalogue = Some(catalogue);
        self.map_pois.clear();
        self.map_coverage = coverage;
        self.map_catalogue_status = MapCatalogueStatus::Ready;
        self.update_map_search();
    }

    pub fn set_map_unavailable(&mut self, reason: impl Into<String>) {
        self.map_catalogue_status = MapCatalogueStatus::Unavailable;
        self.map_coverage = reason.into();
        self.map_pois.clear();
        self.map_catalogue = None;
        self.map_search_index.clear();
        self.map_search_results.clear();
    }

    /// The Map tab's own keys (after the router tried the `MAP_KEYS`
    /// commands and Esc-to-leave): plane, diagnostic layers, the group
    /// toggle for the selected bot, one-shot zone crossing, Enter select/confirm, pan and zoom.
    pub(crate) fn map_pane_key(&mut self, key: KeyEvent) -> AppAction {
        match key.code {
            KeyCode::PageUp => self.set_map_plane(self.map.plane.saturating_add(1)),
            KeyCode::PageDown => self.set_map_plane(self.map.plane.saturating_sub(1)),
            KeyCode::Char(level @ '0'..='3') => self.set_map_plane(level as u8 - b'0'),
            KeyCode::Char('d') => self.map.layers.dots = !self.map.layers.dots,
            KeyCode::Char('c') => self.map.layers.collision = !self.map.layers.collision,
            KeyCode::Char('r') => self.map.layers.reach = !self.map.layers.reach,
            KeyCode::Char(' ') => return self.toggle_walk_send_focused(),
            KeyCode::Char('z') => {
                self.refresh_walk_permissions();
                if self.walk_permissions.danger_this_walk(true) {
                    self.map_route_through_zones = !self.map_route_through_zones;
                }
            }
            KeyCode::Enter => return self.map_enter(),
            _ => return self.map_on_key(key),
        }
        AppAction::None
    }

    /// Open the map search line (name or `x,z,plane`).
    pub(crate) fn open_map_search(&mut self) {
        self.map_search_open = true;
        self.map_search.clear();
        self.map_search_results.clear();
        self.map_search_sel = 0;
    }

    /// A click on the map cells selects that tile through the shared map
    /// model; walking it stays a second, explicit action.
    pub(crate) fn map_click(&mut self, col: u16, row: u16) {
        let area = self.regions.map;
        let step = ZOOMS[self.map.zoom.min(ZOOMS.len() - 1)] as i32;
        let (bx, bz) = self
            .here
            .map(|h| (h.x, h.z))
            .unwrap_or(crate::map::DEFAULT_CENTRE);
        let centre = (bx + self.map.pan.0, bz + self.map.pan.1);
        let Some((x, z)) = crate::map::cell_tile(centre, step, area, col, row) else {
            return;
        };
        self.select_requested(Tile {
            x,
            z,
            level: i32::from(self.map.plane),
        });
    }

    /// Pan the map by whole zoom steps north (`rows > 0`) or south.
    pub(crate) fn map_pan_rows(&mut self, rows: i32) {
        let step = ZOOMS[self.map.zoom.min(ZOOMS.len() - 1)] as i32;
        self.map.pan.1 += rows * step;
    }

    /// Route keys to the params popup when it is open. Edits go through
    /// `commit`; Apply-to-all keys come back as actions.
    pub fn params_on_key(
        &mut self,
        commit: &mut ParamsCommit<'_>,
        loadouts: &Loadouts,
        game_data: Option<&api::game_data::SelectedGameData>,
        key: KeyEvent,
    ) -> AppAction {
        if !self.params_state.open {
            return AppAction::None;
        }
        if self.params_card().is_none() {
            self.params_state.open = false;
            return AppAction::None;
        }
        self.refresh_walk_permissions();
        let mut pane = ParamsPane {
            schema: self.params_schema.as_ref(),
            bag: &mut self.params_bag,
            commit,
            loadouts,
            game_data,
            state: &mut self.params_state,
        };
        match pane.on_key(key.code) {
            ParamsKey::Close => {
                self.params_state.open = false;
                AppAction::None
            }
            ParamsKey::SyncPrepare => AppAction::ScriptSyncPrepare,
            ParamsKey::SyncApply => AppAction::ScriptSyncApply,
            ParamsKey::SyncCancel => AppAction::ScriptSyncCancel,
            _ => AppAction::None,
        }
    }

    /// The selected card whose shared schema the params popup edits.
    pub fn params_card(&self) -> Option<ScriptChoice> {
        self.script_sel.as_ref().map(ScriptChoice::duplicate)
    }

    /// Open the params popup over `bag`, the focused profile's merged bag
    /// for the selected card.
    pub fn open_script_params(&mut self, bag: serde_json::Map<String, serde_json::Value>) {
        if self.params_card().is_none()
            || self.params_unavailable.is_some()
            || self.params_schema.as_ref().is_empty()
        {
            return;
        }
        self.params_bag = bag;
        self.params_state = ParamsState {
            open: true,
            cursor: 0,
            walk_permissions: self.walk_permissions,
            ..Default::default()
        };
    }
    #[cfg(feature = "js")]
    fn load_entries(&self) -> Vec<LoadEntry> {
        let mut out = vec![LoadEntry::Up];
        let mut subdirs = Vec::new();
        let mut files = Vec::new();
        if let Ok(read) = std::fs::read_dir(&self.script_load_dir) {
            for entry in read.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') {
                    continue;
                }
                if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    subdirs.push(name);
                } else if name.ends_with(".ts") || name.ends_with(".js") {
                    files.push(name);
                }
            }
        }
        subdirs.sort();
        files.sort();
        for name in subdirs {
            out.push(LoadEntry::Subdir(name));
        }
        for name in files {
            out.push(LoadEntry::File(name));
        }
        out.push(LoadEntry::Cancel);
        out
    }

    #[cfg(feature = "js")]
    fn load_on_key(&mut self, key: KeyEvent) -> AppAction {
        let entries = self.load_entries();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                if self.script_load_sel > 0 {
                    self.script_load_sel -= 1;
                }
                AppAction::None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.script_load_sel + 1 < entries.len() {
                    self.script_load_sel += 1;
                }
                AppAction::None
            }
            KeyCode::Esc => {
                self.script_load_open = false;
                AppAction::None
            }
            KeyCode::Enter => match entries.get(self.script_load_sel) {
                Some(LoadEntry::Cancel) => {
                    self.script_load_open = false;
                    AppAction::None
                }
                Some(LoadEntry::Up) => {
                    if let Some(parent) = self.script_load_dir.parent() {
                        self.script_load_dir = parent.to_path_buf();
                        self.script_load_sel = 0;
                    }
                    AppAction::None
                }
                Some(LoadEntry::Subdir(name)) => {
                    self.script_load_dir.push(name.clone());
                    self.script_load_sel = 0;
                    AppAction::None
                }
                Some(LoadEntry::File(name)) => {
                    let path = self.script_load_dir.join(name);
                    self.script_load_open = false;
                    AppAction::ScriptLoad(path)
                }
                _ => AppAction::None,
            },
            _ => AppAction::None,
        }
    }

    /// Open the out-of-tree Load file browser.
    #[cfg(feature = "js")]
    pub fn open_script_load_browser(&mut self, last_dir: Option<&std::path::Path>) {
        self.script_load_dir = default_load_browse_dir(last_dir);
        self.script_load_sel = 0;
        self.script_load_open = true;
    }

    /// First-run catalog folder browser keys.
    #[cfg(feature = "js")]
    fn catalog_on_key(&mut self, key: KeyEvent) -> AppAction {
        let entries = self.catalog_entries();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                if self.catalog_sel > 0 {
                    self.catalog_sel -= 1;
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.catalog_sel + 1 < entries.len() {
                    self.catalog_sel += 1;
                }
            }
            KeyCode::Esc => return AppAction::ScriptDeferCatalog,
            KeyCode::Enter => {
                return match entries.get(self.catalog_sel) {
                    Some(CatalogEntry::Up) => {
                        if let Some(parent) = self.rs2b0t_catalog_dir.parent() {
                            self.rs2b0t_catalog_dir = parent.to_path_buf();
                            self.catalog_sel = 0;
                        }
                        AppAction::None
                    }
                    Some(CatalogEntry::Subdir(name)) => {
                        self.rs2b0t_catalog_dir.push(name.clone());
                        self.catalog_sel = 0;
                        AppAction::None
                    }
                    Some(CatalogEntry::UseFolder) => AppAction::ScriptUseCatalog,
                    Some(CatalogEntry::NotNow) => AppAction::ScriptDeferCatalog,
                    None => AppAction::None,
                };
            }
            _ => {}
        }
        AppAction::None
    }

    #[cfg(feature = "js")]
    fn catalog_entries(&self) -> Vec<CatalogEntry> {
        let mut out = vec![CatalogEntry::Up];
        if let Ok(read) = std::fs::read_dir(&self.rs2b0t_catalog_dir) {
            let mut subdirs: Vec<String> = read
                .flatten()
                .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
                .filter_map(|e| {
                    e.file_name()
                        .into_string()
                        .ok()
                        .filter(|s| !s.starts_with('.'))
                })
                .collect();
            subdirs.sort();
            for name in subdirs {
                out.push(CatalogEntry::Subdir(name));
            }
        }
        if rs2b0t_root_has_index(&self.rs2b0t_catalog_dir) {
            out.push(CatalogEntry::UseFolder);
        }
        out.push(CatalogEntry::NotNow);
        out
    }

    /// Move the Browse selection `step` cards through the list (wrapping).
    fn move_script_sel(&mut self, step: i32) {
        let lines = {
            #[cfg(feature = "js")]
            {
                crate::script_shape::browse_lines(
                    &self.script_cards,
                    &self.script_category_order,
                    frontend_core::scripts::paths::rs2b0t_import_deferred(),
                )
            }
            #[cfg(not(feature = "js"))]
            {
                crate::script_shape::browse_lines(&self.script_cards, &self.script_category_order)
            }
        };
        let card_indices: Vec<usize> = lines
            .into_iter()
            .filter_map(|line| match line {
                crate::script_shape::BrowseLine::Card(index) => Some(index),
                _ => None,
            })
            .collect();
        if card_indices.is_empty() {
            return;
        }
        let pos = self
            .script_sel
            .as_ref()
            .and_then(|sel| match sel.loaded_card() {
                None => self
                    .script_cards
                    .iter()
                    .position(|card| card_selection(card).as_ref() == Some(sel)),
                Some((source, name)) => self
                    .script_cards
                    .iter()
                    .position(|card| card.source == source && card.name == name),
            })
            .and_then(|card_idx| card_indices.iter().position(|&i| i == card_idx))
            .unwrap_or(0);
        let next = (pos as i32 + step).rem_euclid(card_indices.len() as i32) as usize;
        let card = &self.script_cards[card_indices[next]];
        self.script_sel = card_selection(card);
        self.browse_changed = true;
    }

    /// Whether the script popup is open.
    pub(crate) fn script_popup_open(&self) -> bool {
        #[cfg(feature = "js")]
        {
            self.script_load_open || self.rs2b0t_catalog_open || self.script_browse_open
        }
        #[cfg(not(feature = "js"))]
        {
            self.script_browse_open
        }
    }

    /// Keys while a Script popup is open; every other key is ignored.
    pub(crate) fn script_popup_key(&mut self, key: KeyEvent) -> Option<AppAction> {
        #[cfg(feature = "js")]
        {
            if self.script_load_open {
                return Some(self.load_on_key(key));
            }
            if self.rs2b0t_catalog_open {
                return Some(self.catalog_on_key(key));
            }
        }
        if !self.script_browse_open {
            return None;
        }
        let text = !key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER);
        match key.code {
            KeyCode::Up => self.move_script_sel(-1),
            KeyCode::Char('k') if text => self.move_script_sel(-1),
            KeyCode::Down => self.move_script_sel(1),
            KeyCode::Char('j') if text => self.move_script_sel(1),
            KeyCode::Enter | KeyCode::Esc => self.script_browse_open = false,
            _ => {}
        }
        Some(AppAction::None)
    }

    /// Handle clicks while a Script popup is open.
    pub(crate) fn script_popup_click(&mut self, col: u16, row: u16) -> AppAction {
        if crate::layout::contains(self.script_area, col, row) {
            return self.script_click(col, row);
        }
        #[cfg(feature = "js")]
        if !self.rs2b0t_catalog_open {
            self.script_load_open = false;
            self.script_browse_open = false;
        }
        #[cfg(not(feature = "js"))]
        {
            self.script_browse_open = false;
        }
        AppAction::None
    }

    /// Move the open script picker on wheel input.
    pub(crate) fn script_popup_scroll(&mut self, delta: isize) {
        #[cfg(feature = "js")]
        if self.script_load_open {
            let last = self.load_entries().len().saturating_sub(1);
            self.script_load_sel = self.script_load_sel.saturating_add_signed(delta).min(last);
        } else if self.rs2b0t_catalog_open {
            let last = self.catalog_entries().len().saturating_sub(1);
            self.catalog_sel = self.catalog_sel.saturating_add_signed(delta).min(last);
        } else if self.script_browse_open {
            self.move_script_sel(delta as i32);
        }
        #[cfg(not(feature = "js"))]
        if self.script_browse_open {
            self.move_script_sel(delta as i32);
        }
    }

    /// The script pane's clicks: a button runs its command, picker rows store
    /// the selection.
    pub(crate) fn script_click(&mut self, col: u16, row: u16) -> AppAction {
        #[cfg(feature = "js")]
        {
            if self.script_load_open {
                return self.load_click(col, row);
            }
            if self.rs2b0t_catalog_open {
                return self.catalog_click(col, row);
            }
        }
        let pane = ScriptPane::new(
            self.script_state,
            self.script_sel.as_ref(),
            &self.script_cards,
            &self.script_category_order,
            self.script_browse_open,
            !self.params_schema.as_ref().is_empty(),
        )
        .with_native_status(
            self.focused_detail()
                .and_then(|detail| detail.native_status()),
        );
        #[cfg(feature = "js")]
        let pane = pane.with_js_controls(
            frontend_core::scripts::paths::rs2b0t_import_deferred(),
            self.script_load_open,
            self.reload_confirm,
        );
        match pane.on_click(self.script_area, col, row) {
            ScriptClick::Button(label) => {
                script_command(label).map_or(AppAction::None, |command| self.run_command(command))
            }
            ScriptClick::Params => self.run_command(Command::ScriptParams),
            #[cfg(feature = "js")]
            ScriptClick::ImportCatalog => self.run_command(Command::ImportCatalog),
            ScriptClick::Pick(idx) => {
                if let Some(card) = self.script_cards.get(idx) {
                    self.script_sel = card_selection(card);
                    self.browse_changed = true;
                }
                AppAction::None
            }
            ScriptClick::None => AppAction::None,
        }
    }

    /// One script command after the router checked it is available.
    pub(crate) fn script_run(&mut self, command: Command) -> AppAction {
        match command {
            Command::ScriptBrowse => {
                let opening = !self.script_browse_open;
                self.script_browse_open = opening;
                if opening {
                    AppAction::ScriptBrowse
                } else {
                    AppAction::None
                }
            }
            Command::ScriptStart => match self.script_sel.as_ref().map(ScriptChoice::duplicate) {
                Some(sel) => AppAction::ScriptStart(sel),
                None => AppAction::Refused(Refusal {
                    command: Some(OperatorCommand::StartScript),
                    bot: self.focused_name(),
                    reason: "script: browse to pick one first".into(),
                    need: Unavailable::NoScriptPicked.need(),
                }),
            },
            Command::ScriptPause => AppAction::ScriptPause,
            Command::ScriptSkipQuest => AppAction::ScriptSkipQuest,
            Command::ScriptStop => AppAction::ScriptStop,
            #[cfg(feature = "js")]
            Command::ScriptLoad => {
                let last = self.script_load_last_dir.clone();
                self.open_script_load_browser(last.as_deref());
                AppAction::None
            }
            Command::ScriptParams => AppAction::ScriptParams,
            #[cfg(feature = "js")]
            Command::ScriptReload => AppAction::ScriptReload,
            #[cfg(feature = "js")]
            Command::ScriptReloadCancel => AppAction::ScriptReloadCancel,
            #[cfg(feature = "js")]
            Command::ImportCatalog => AppAction::ScriptImportCatalog,
            _ => AppAction::None,
        }
    }

    #[cfg(feature = "js")]
    fn load_click(&mut self, col: u16, row: u16) -> AppAction {
        let inner = Block::default()
            .borders(Borders::ALL)
            .inner(self.script_area);
        if row < inner.y + 2 {
            return AppAction::None;
        }
        let line = row - (inner.y + 2);
        let entries = self.load_entries();
        if usize::from(line) == self.script_load_sel && entries.get(self.script_load_sel).is_some()
        {
            self.script_load_sel = usize::from(line);
            return self.load_on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        }
        if usize::from(line) < entries.len() {
            self.script_load_sel = usize::from(line);
        }
        let _ = col;
        AppAction::None
    }

    #[cfg(feature = "js")]
    fn catalog_click(&mut self, col: u16, row: u16) -> AppAction {
        let inner = Block::default()
            .borders(Borders::ALL)
            .inner(self.script_area);
        if row < inner.y + 2 {
            return AppAction::None;
        }
        let line = row - (inner.y + 2);
        let entries = self.catalog_entries();
        if usize::from(line) == self.catalog_sel && entries.get(self.catalog_sel).is_some() {
            // Enter-equivalent on the highlighted row.
            self.catalog_sel = usize::from(line);
            return self.catalog_on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        }
        if usize::from(line) < entries.len() {
            self.catalog_sel = usize::from(line);
        }
        let _ = col;
        AppAction::None
    }

    /// Render the loadouts popup overlay (call after [`Self::draw`]).
    pub fn draw_loadouts_overlay(&mut self, frame: &mut Frame<'_>, form: &LoadoutForm) {
        if !self.loadouts_state.open {
            return;
        }
        if self.loadouts_state.name_scratch.is_empty() && !form.loadouts().is_empty() {
            self.loadouts_state.fill(form);
        }
        frame.render_widget(
            LoadoutsPane {
                form,
                state: &self.loadouts_state,
            },
            frame.area(),
        );
    }

    /// Render the params popup overlay (call after [`Self::draw`]).
    pub fn draw_params_overlay(
        &mut self,
        frame: &mut Frame<'_>,
        loadouts: &Loadouts,
        game_data: Option<&api::game_data::SelectedGameData>,
    ) {
        self.refresh_walk_permissions();
        if !self.params_state.open || self.params_card().is_none() {
            return;
        }
        self.params_state.walk_permissions = self.walk_permissions;
        // Rendering never commits.
        let mut read_only = |_: &str, _: serde_json::Value, _: Option<serde_json::Value>| Ok(());
        let pane = ParamsPane {
            schema: self.params_schema.as_ref(),
            bag: &mut self.params_bag,
            commit: &mut read_only,
            loadouts,
            game_data,
            state: &mut self.params_state,
        };
        frame.render_widget(pane, frame.area());
    }

    pub(crate) fn draw_map(&mut self, frame: &mut Frame<'_>, area: Rect) {
        if !self.map_active {
            let block = Block::default()
                .borders(Borders::ALL)
                .title("Map (F4 opens it; no catalogue demand until then)");
            frame.render_widget(block, area);
            return;
        }
        self.refresh_walk_permissions();
        self.sync_walk_label();
        let title = format!(
            "Map · plane {} · {:?} · {} · arrows/hjkl pan · +/- zoom · / search",
            self.map.plane,
            self.map_catalogue_status,
            self.walk_send.walk_label()
        );
        let block = Block::default().borders(Borders::ALL).title(title);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let [map_area, button_area, info_area] = Layout::vertical([
            Constraint::Min(1),
            Constraint::Length(inner.height.min(1)),
            // Shared permission copy wraps; keep the legend and observations visible.
            Constraint::Length(inner.height.saturating_sub(2).min(9)),
        ])
        .areas(inner);
        self.regions.map = map_area;
        self.draw_map_buttons(frame.buffer_mut(), button_area);

        let observed = self.observed_marks();
        let catalogue = self.map_catalogue.clone();
        let poi_count = catalogue
            .as_ref()
            .map(|c| c.entries().len())
            .unwrap_or(self.map_pois.len());
        if let Some(world) = self.world.clone() {
            let mut map = Map::new(&world, &mut self.map, |_| {})
                .reach(self.map_reach.as_deref())
                .selected_poi(self.map_poi_sel)
                .observed(&observed);
            map = if let Some(catalogue) = catalogue.as_ref() {
                map.catalogue(catalogue)
            } else {
                map.pois(&self.map_pois)
            };
            if let Some(here) = self.here {
                map = map.here(here);
            }
            if let Some(route) = &self.route {
                map = map.route(route);
            }
            frame.render_widget(map, map_area);
        } else {
            Paragraph::new("collision unavailable (nav pack not loaded)")
                .render(map_area, frame.buffer_mut());
        }

        let send = match self.walk_send.mode {
            WalkSendMode::Focused => "Send: Focused".to_string(),
            WalkSendMode::Group => {
                let rows = self
                    .walk_send
                    .rows
                    .iter()
                    .map(|row| {
                        let mark = if row.checked(self.fleet_model.marks()) {
                            "*"
                        } else {
                            " "
                        };
                        if row.status.is_eligible() {
                            format!("{mark}{}", row.name())
                        } else if let Some(reason) = row.status.exclusion() {
                            format!("{mark}{} ({})", row.name(), reason.as_str())
                        } else {
                            format!("{mark}{}", row.name())
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("  ");
                format!("Send: Group  {}  {rows}", self.walk_send.walk_label())
            }
        };
        let crossing_enabled = self.walk_permissions.danger_this_walk(true);
        let zones = if crossing_enabled {
            Line::from(format!(
                "{send} · {}: {}",
                frontend_core::DANGER_THIS_WALK_LABEL,
                if self.map_route_through_zones {
                    "crossing (z)"
                } else {
                    "avoided"
                }
            ))
        } else {
            Line::styled(
                format!("{send} · {}", frontend_core::GLOBAL_DANGER_WARNING),
                Style::default().fg(Color::Red),
            )
        };
        let zone_hint = if crossing_enabled {
            Line::from(format!(
                "z: {} allows routes past monsters that may kill your bot.",
                frontend_core::DANGER_THIS_WALK_LABEL
            ))
        } else {
            Line::styled(
                frontend_core::GLOBAL_DANGER_WARNING,
                Style::default().fg(Color::Red),
            )
        };
        let mut info = vec![
            zones,
            zone_hint,
            Line::from(if let Some(err) = &self.error {
                format!("status: {err}")
            } else {
                format!(
                    "status: {:?}  {}",
                    self.map_catalogue_status, self.map_coverage
                )
            }),
            Line::from(format!(
                "legend: @ * + shade B T X N #  POIs:{} obs:{} {} · w wilderness:{}",
                poi_count,
                self.map_observed.len(),
                if self.route.is_some() {
                    "route"
                } else {
                    "none"
                },
                if self.map.layers.wilderness {
                    "on"
                } else {
                    "off"
                }
            )),
        ];
        if self.map_search_open {
            info.push(Line::from(format!("/{}", self.map_search)));
        } else if !self.map_search_results.is_empty() {
            let rows = self
                .map_search_results
                .iter()
                .take(2)
                .filter_map(|&index| self.search_hit_label(index))
                .collect::<Vec<_>>()
                .join("  ");
            if !rows.is_empty() {
                info.push(Line::from(format!("POI: {rows}")));
            }
        } else if !self.map_observed.is_empty() {
            let rows = self
                .map_observed
                .iter()
                .take(2)
                .map(|obs| {
                    format!(
                        "N {:?} ({},{},{})",
                        obs.kind(),
                        obs.tile().x,
                        obs.tile().z,
                        obs.tile().level
                    )
                })
                .collect::<Vec<_>>()
                .join("  ");
            info.push(Line::from(format!("observed: {rows}")));
        }
        Paragraph::new(info)
            .wrap(Wrap { trim: true })
            .render(info_area, frame.buffer_mut());
    }

    pub(crate) fn draw_chat(&mut self, frame: &mut Frame<'_>, area: Rect) {
        let chat = Chat::new(self.chat_data.view(), &mut self.chat, |_| {});
        frame.render_widget(chat, area);
    }

    pub(crate) fn draw_status(&mut self, frame: &mut Frame<'_>, area: Rect) {
        let walk = self
            .walk_dest
            .map(|t| format!("{} {} {}", t.x, t.z, t.level))
            .unwrap_or_else(|| "—".into());
        let mem = frontend_core::MemoryNotice::status_text(self.settings.lowmem(), self.memory);
        let note = self.memory.filter(|n| n.differs()).map(|n| n.notice_text());
        let pane = StatusPane::new(self.focused_detail(), &walk, &mem)
            .mem_notice(note)
            .resources(&self.resources)
            .notice(self.background_notice.as_deref());
        frame.render_widget(pane, area);
    }

    /// inv / stats / locs pane: the focused snapshot's inventory item
    /// names, used skill rows, and nearest named locs (name + Chebyshev).
    pub(crate) fn draw_inv_locs(&mut self, frame: &mut Frame<'_>, area: Rect) {
        let block = Block::default()
            .borders(Borders::ALL)
            .title("inv/stats/locs");
        let inner = block.inner(area);
        block.render(area, frame.buffer_mut());
        let mut lines: Vec<Line> = Vec::new();
        let inv: Vec<String> = self
            .inv_items
            .iter()
            .take(4)
            .map(|(name, count)| {
                if *count > 1 {
                    format!("{name} x{count}")
                } else {
                    name.clone()
                }
            })
            .collect();
        let more = self.inv_items.len().saturating_sub(4);
        let mut inv_line = if inv.is_empty() {
            "inv: —".to_string()
        } else {
            format!("inv: {}", inv.join(", "))
        };
        if more > 0 {
            inv_line.push_str(&format!("  +{more} more"));
        }
        lines.push(Line::from(inv_line));

        let stats: Vec<String> = self
            .stats_rows
            .iter()
            .take(6)
            .map(|(name, level)| format!("{name} {level}"))
            .collect();
        if !stats.is_empty() {
            lines.push(Line::from(format!("stats: {}", stats.join("  "))));
        }

        if self.locs_near.is_empty() {
            lines.push(Line::from("locs: —"));
        } else {
            let list: Vec<String> = self
                .locs_near
                .iter()
                .take(3)
                .map(|(d, n)| format!("{n} ({d})"))
                .collect();
            lines.push(Line::from(format!("locs: {}", list.join("  "))));
        }
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .render(inner, frame.buffer_mut());
    }

    pub(crate) fn draw_script(&mut self, frame: &mut Frame<'_>, area: Rect) {
        #[cfg(feature = "js")]
        if self.script_load_open {
            let block = Block::default().borders(Borders::ALL).title("load script");
            let inner = block.inner(area);
            block.render(area, frame.buffer_mut());
            let mut lines = vec![
                Line::from("Choose a .ts or .js bot file (out-of-tree):"),
                Line::from(self.script_load_dir.to_string_lossy().to_string()),
            ];
            for (i, entry) in self.load_entries().iter().enumerate() {
                let mark = if i == self.script_load_sel {
                    "> "
                } else {
                    "  "
                };
                let label = match entry {
                    LoadEntry::Up => "[Up]".into(),
                    LoadEntry::Subdir(name) => format!("{name}/"),
                    LoadEntry::File(name) => name.clone(),
                    LoadEntry::Cancel => "[Cancel]".into(),
                };
                lines.push(Line::from(format!("{mark}{label}")));
            }
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .render(inner, frame.buffer_mut());
            return;
        }
        #[cfg(feature = "js")]
        if self.rs2b0t_catalog_open {
            let block = Block::default()
                .borders(Borders::ALL)
                .title("import rs2b0t catalog");
            let inner = block.inner(area);
            block.render(area, frame.buffer_mut());
            let mut lines = vec![
                Line::from("Choose clone root (src/bot/scripts/index.ts):"),
                Line::from(self.rs2b0t_catalog_dir.to_string_lossy().to_string()),
            ];
            if rs2b0t_root_has_index(&self.rs2b0t_catalog_dir) {
                lines.push(Line::from("catalog index found"));
            } else {
                lines.push(Line::from("no src/bot/scripts/index.ts here"));
            }
            for (i, entry) in self.catalog_entries().iter().enumerate() {
                let mark = if i == self.catalog_sel { "> " } else { "  " };
                let label = match entry {
                    CatalogEntry::Up => "[Up]".into(),
                    CatalogEntry::Subdir(name) => format!("{name}/"),
                    CatalogEntry::UseFolder => "[Use this folder]".into(),
                    CatalogEntry::NotNow => "[Not now]".into(),
                };
                lines.push(Line::from(format!("{mark}{label}")));
            }
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .render(inner, frame.buffer_mut());
            return;
        }
        let pane = ScriptPane::new(
            self.script_state,
            self.script_sel.as_ref(),
            &self.script_cards,
            &self.script_category_order,
            self.script_browse_open,
            !self.params_schema.as_ref().is_empty(),
        );
        #[cfg(feature = "js")]
        let pane = pane.with_js_controls(
            frontend_core::scripts::paths::rs2b0t_import_deferred(),
            self.script_load_open,
            self.reload_confirm,
        );
        let pane = pane
            .with_native_status(
                self.focused_detail()
                    .and_then(|detail| detail.native_status()),
            )
            .with_walk_risk(self.focused_detail().and_then(|detail| detail.walk_risk()));
        frame.render_widget(pane, area);
    }
}

#[cfg(test)]
#[path = "app_tests.rs"]
mod tests;
