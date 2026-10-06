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

use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Paragraph, Widget, Wrap};
use ratatui::Frame;

use api::snapshot::{ChatLineView, ChatOptionView, WorldTile};
use frontend_core::quester_paths::QuesterPathsView;
use frontend_core::{FleetCounts, FleetRow, ResourceView, SlotDetail, WalkGlobalsView};
use frontend_core::{MapBakeChoice, NavPreference};
use host_play::walk_map::{
    Catalogue, DisplayName, MapModel, ObservedService, Search, WalkSlotStatus,
};
use nav::map::poi::{PoiKind, PoiRecord};
use nav::router::{FindOptions, Route};
use nav::tile::Tile;
use nav::world::NavWorld;
use script::{RunState, ScriptSel};

use crate::chat::{chat_modal_open, Chat, ChatAction, ChatState, ChatView};
use crate::commands::{script_command, Command};
use crate::fleet::FleetState;
use crate::layout::{Pane, Regions, Screen};
use crate::loadouts::{LoadoutsPane, LoadoutsState};
use crate::map::{Map, MapAction, MapView, ObservedMark, ZOOMS};
use crate::overlay::Modal;
use crate::script_params::{ParamsCommit, ParamsKey, ParamsPane, ParamsState};
use crate::script_shape::{
    browse_lines, card_selection, rs2b0t_root_has_index, BrowseCard, BrowseLine, ScriptClick,
    ScriptPane,
};
use crate::settings::SettingsState;
use crate::status::StatusPane;

fn default_catalog_browse_dir() -> std::path::PathBuf {
    let home = script::bot_home();
    if home.as_os_str() == "." {
        std::path::PathBuf::from("/")
    } else {
        home
    }
}

fn default_load_browse_dir(last: Option<&std::path::Path>) -> std::path::PathBuf {
    last.filter(|p| p.is_dir())
        .map(std::path::Path::to_path_buf)
        .unwrap_or_else(default_catalog_browse_dir)
}

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

#[derive(Debug, Clone)]
pub struct WalkSendRow {
    pub name: String,
    pub status: WalkSlotStatus,
    pub checked: bool,
}

#[derive(Debug, Clone)]
pub struct WalkSendState {
    pub mode: WalkSendMode,
    rows: Vec<WalkSendRow>,
    walk_label: String,
}

impl Default for WalkSendState {
    fn default() -> Self {
        Self {
            mode: WalkSendMode::Focused,
            rows: Vec::new(),
            walk_label: "Walk".into(),
        }
    }
}

impl WalkSendState {
    pub fn rows(&self) -> &[WalkSendRow] {
        &self.rows
    }

    pub fn walk_label(&self) -> &str {
        &self.walk_label
    }

    pub fn checked_count(&self) -> usize {
        self.rows.iter().filter(|row| row.checked).count()
    }

    fn sync_walk_label(&mut self) {
        self.walk_label = match self.mode {
            WalkSendMode::Focused => "Walk".into(),
            WalkSendMode::Group => format!("Walk {} bots", self.checked_count()),
        };
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppAction {
    /// Quit the app.
    Quit,
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
    /// WASD one-tile walk: queue a `host_play::WireCmd::Walk`.
    WalkTile(Tile),
    /// Local debug teleport is intentionally a separate named action. The
    /// binary/host must authorize it; the TUI never treats a pin as proof.
    MapTeleport(Tile),
    /// Reload the shared Quester Path registry without starting a Path.
    ReloadPaths,
    /// Chat modal advance: queue `WireCmd::Continue` / `Answer`.
    Chat(ChatAction),
    /// MultiBox: load every vault profile and log every member in (after
    /// the scope confirmation).
    SpawnAll,
    /// Log in the selected member (explicit handshake).
    Login,
    /// Log out the selected member (it stays loaded, latched).
    Logout,
    /// Log out every member (after the scope confirmation).
    LogoutAll,
    /// Relog the settings-bound member for a memory-mode switch (the
    /// settings popup's `r` key): the binary confirms first when a
    /// running script would be interrupted.
    MemoryRelog(String),
    /// Confirmed memory Relog-now: log out and back in through the FIFO.
    MemoryRelogNow(String),
    /// Remove this member from the fleet (clean logout, then stop). The
    /// name was frozen when the operator confirmed.
    Remove(String),
    /// Start the Browse-selected JS card on the focused slot:
    /// `tui-play` dispatches `Play::script_start_load` with the card's
    /// source and shape.
    ScriptStart(ScriptSel),
    /// Toggle pause/resume on the focused slot's script (`Play::script_pause`
    /// / [`Play::script_resume`], like the panel's `script_toggle_pause`).
    ScriptPause,
    /// Stop the focused slot's script (`Play::script_stop`).
    ScriptStop,
    /// Open the script Browse picker (registry cards).
    ScriptBrowse,
    /// Open the script params popup for the selected card.
    ScriptParams,
    /// Start every wall member on its last successful assignment.
    ScriptStartAll,
    /// Stop every member's script (queued replacement Starts included).
    ScriptStopAll,
    /// Save the selected script as the assignment of every marked bot.
    ScriptAssignMarked,
    /// Assign the selected script to every marked bot and start it there.
    ScriptRestartMarked,
    /// Freeze copying the focused bot's settings for the selected script to
    /// the marked bots, then ask to confirm the frozen scope.
    ScriptApplyMarkedPrepare,
    /// Log in every marked bot (loading the ones not loaded yet).
    LoginMarked,
    /// Log out every marked bot.
    LogoutMarked,
    /// Reload the focused heading's card, or confirm a shown warning.
    ScriptReload,
    /// Discard a prepared reload without touching any run.
    ScriptReloadCancel,
    /// Prepare Apply to all for the params popup's card.
    ScriptSyncPrepare,
    /// Apply the prepared Apply to all.
    ScriptSyncApply,
    /// Drop the prepared Apply to all.
    ScriptSyncCancel,
    /// Open the first-run rs2b0t catalog folder browser.
    ScriptImportCatalog,
    /// Defer the rs2b0t catalog import (Not now).
    ScriptDeferCatalog,
    /// Import catalog from the chosen clone root.
    ScriptUseCatalog,
    /// Load the JS bot at `path` into the library and select it.
    ScriptLoad(std::path::PathBuf),
    /// Persist the background-bots notice ("Got it, don't show again").
    AckBackground,
    /// A key for the parameters popup: the binary owns the commit path.
    ParamsKey(KeyEvent),
    /// A key for the loadouts popup: the binary owns the store.
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
#[derive(Debug, Clone, Default)]
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
    pub script_paint: Option<std::sync::Arc<script::shim::ScriptPaint>>,
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
            script_paint: self.script_paint.as_deref(),
            show_game_chat: self.show_game_chat,
        }
    }
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
    /// The fleet rows (one per member) and their totals from the shared
    /// `frontend-core` projection; the binary copies them when the core's
    /// rows move.
    pub fleet: Vec<FleetRow>,
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
    pub map_model: MapModel,
    pub map_active: bool,
    /// Search editor and bounded result indices into `map_pois`.
    pub map_search_open: bool,
    pub map_search: String,
    pub map_search_results: Vec<usize>,
    pub map_search_sel: usize,
    /// The shared compact catalogue projection. It is application-owned,
    /// never copied into each bot slot.
    pub map_pois: Vec<PoiRecord>,
    pub map_host_catalogue: Option<Arc<Catalogue>>,
    map_search_index: Search,
    pub map_poi_sel: Option<usize>,
    pub map_catalogue_status: MapCatalogueStatus,
    pub map_coverage: String,
    /// Focused-bot observed NPC services while the map is open.
    pub map_observed: Vec<ObservedService>,
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
    pub settings: vault::ProfileSettings,
    /// Editable settings buffer; durable reads are rendered from `walk_permissions`.
    pub nav: host_play::WalkGlobals,
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
    shared_preferences_path: Option<std::path::PathBuf>,
    pub loadouts_state: LoadoutsState,
    /// The focused slot's script lifecycle (shape display only).
    pub script_state: RunState,
    /// Focused member holds a Start-all permit place (Stop still applies).
    pub script_queued: bool,
    /// The focused profile's script heading; Start keys on `(source, name)`.
    pub script_sel: Option<ScriptSel>,
    /// The operator picked a card in Browse since the last pump: the
    /// binary records it as the focused profile's pending selection.
    pub browse_changed: bool,
    /// A reload warning awaits confirmation: Reload reads "Confirm".
    pub reload_confirm: bool,
    /// Registry cards the Browse picker lists (copied from the session each pump).
    pub script_cards: Vec<BrowseCard>,
    /// Persisted category order keys for Browse grouping.
    pub script_category_order: Vec<String>,
    /// The Browse picker is open.
    pub script_browse_open: bool,
    /// First-run rs2b0t clone-root folder browser.
    pub rs2b0t_catalog_open: bool,
    pub rs2b0t_catalog_dir: std::path::PathBuf,
    /// Highlight row in the catalog folder browser.
    pub catalog_sel: usize,
    /// The Load file browser is open.
    pub script_load_open: bool,
    pub script_load_dir: std::path::PathBuf,
    pub script_load_sel: usize,
    pub script_load_last_dir: Option<std::path::PathBuf>,
    /// Selected card's settings schema (refreshed each pump from the library).
    pub params_schema: Vec<script::SettingDef>,
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
    /// The shared structured log (Logs tab and the log drawer).
    pub log: crate::log_pane::LogPaneState,
    /// The detail tab shown for the selected bot.
    pub screen: Screen,
    /// Where keys go: the fleet, the detail tab or the log drawer.
    pub key_focus: Pane,
    /// Fleet table cursor, row selection and filter (renderer-local).
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
            fleet: Vec::new(),
            counts: FleetCounts::default(),
            detail: None,
            chat_data: ChatData::default(),
            inv_items: Vec::new(),
            stats_rows: Vec::new(),
            locs_near: Vec::new(),
            world: None,
            map_reach: None,
            map: MapView::new(),
            map_model: MapModel::default(),
            map_active: false,
            here: None,
            map_search_open: false,
            map_search: String::new(),
            map_search_results: Vec::new(),
            map_search_sel: 0,
            map_pois: Vec::new(),
            map_host_catalogue: None,
            map_search_index: Search::default(),
            map_poi_sel: None,
            map_catalogue_status: MapCatalogueStatus::Inactive,
            map_coverage: "coverage: unavailable until Map is opened".into(),
            map_observed: Vec::new(),
            walk_send: WalkSendState::default(),
            route: None,
            walk_dest: None,
            chat: ChatState::default(),
            settings: vault::ProfileSettings::default(),
            nav: host_play::WalkGlobals::default(),
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
            shared_preferences_path: None,
            map_bake_dirty: false,
            loadouts_state: LoadoutsState::default(),
            script_state: RunState::Idle,
            script_queued: false,
            script_sel: None,
            browse_changed: false,
            reload_confirm: false,
            script_cards: Vec::new(),
            script_category_order: Vec::new(),
            script_browse_open: false,
            rs2b0t_catalog_open: false,
            rs2b0t_catalog_dir: default_catalog_browse_dir(),
            catalog_sel: 0,
            script_load_open: false,
            script_load_dir: default_load_browse_dir(None),
            script_load_sel: 0,
            script_load_last_dir: None,
            params_schema: Vec::new(),
            params_unavailable: None,
            params_bag: serde_json::Map::new(),
            params_state: ParamsState::default(),
            quit: false,
            error: None,
            resources: ResourceView::default(),
            background_notice: None,
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

    /// Bind the shared `panel-ui.json` preference store. Walk permissions
    /// are refreshed again for every relevant render and admission.
    pub fn restore_preferences(&mut self, path: impl Into<std::path::PathBuf>) {
        let path = path.into();
        self.quester_paths_controller.restore_at(&path);
        self.quester_paths = self.quester_paths_controller.settings().clone();
        self.pause_script_on_manual_walk_abort = frontend_core::nav_preference_at(
            &path,
            frontend_core::NavPreference::PauseScriptOnManualWalkAbort,
            None,
        )
        .unwrap_or(true);
        self.shared_preferences_path = Some(path.clone());
        self.refresh_walk_permissions();
        self.map.restore_persisted_wilderness(path);
    }

    /// Refresh all TUI views of the shared walk permissions from their
    /// durable source. An unbound app uses its local values for isolated
    /// widgets; production binds and rereads the shared preference file.
    pub(crate) fn refresh_walk_permissions(&mut self) {
        let view = self.shared_preferences_path.as_deref().map_or(
            WalkGlobalsView {
                globals: self.nav,
                script_scope_notice_ack: self.script_scope_notice_ack,
                survivable_routing_notice_ack: self.survivable_routing_notice_ack,
            },
            WalkGlobalsView::read_at,
        );
        self.walk_permissions = view;
        self.nav = view.globals;
        self.script_scope_notice_ack = view.script_scope_notice_ack;
        self.survivable_routing_notice_ack = view.survivable_routing_notice_ack;
        if !view.danger_this_walk(true) {
            self.map_route_through_zones = false;
        }
        if self.params_state.open {
            self.params_state.walk_permissions = view;
        }
    }

    pub(crate) fn shared_preferences_path(&self) -> Option<&std::path::Path> {
        self.shared_preferences_path.as_deref()
    }

    pub(crate) fn toggle_map_wilderness(&mut self) {
        match self.map.toggle_wilderness() {
            Ok(()) => {
                if self
                    .error
                    .as_deref()
                    .is_some_and(|error| error.starts_with("map wilderness:"))
                {
                    self.error = None;
                }
            }
            Err(error) => {
                self.error = Some(format!("map wilderness: {error}"));
            }
        }
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
            .filter(|detail| detail.row.name == name)
    }

    /// The focused slot's username.
    pub fn focused_name(&self) -> Option<String> {
        self.focused.and_then(|i| self.names.get(i)).cloned()
    }
    pub(crate) fn profile_id_for_name(&self, name: &str) -> frontend_core::ProfileIdentity {
        self.names
            .iter()
            .position(|candidate| candidate == name)
            .and_then(|index| self.profile_ids.get(index).copied())
            .unwrap_or_else(|| frontend_core::ProfileIdentity::synthetic(name))
    }
    pub(crate) fn is_marked_name(&self, name: &str) -> bool {
        self.table
            .selection
            .contains(self.profile_id_for_name(name))
    }
    pub(crate) fn marked_names(&self) -> Vec<String> {
        self.names
            .iter()
            .enumerate()
            .filter(|(index, name)| {
                self.table.selection.contains(
                    self.profile_ids
                        .get(*index)
                        .copied()
                        .unwrap_or_else(|| frontend_core::ProfileIdentity::synthetic(name)),
                )
            })
            .map(|(_, name)| name.clone())
            .collect()
    }

    /// Re-sync the view from the fresh projection: the selected slot's
    /// `here` tile (the map re-centres on it when the view is not panned).
    pub fn refresh(&mut self) {
        self.here = self
            .focused_detail()
            .and_then(|detail| detail.ready_tile)
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
        if let Some(catalogue) = self.map_host_catalogue.clone() {
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
        if let Some(catalogue) = &self.map_host_catalogue {
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
        Some(format!(
            "{} {} ({},{},{})",
            kind_glyph(poi.kind),
            DisplayName::new(poi.name.as_str()),
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
        if let Some(catalogue) = self.map_host_catalogue.clone() {
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
                if self.walk_send.checked_count() == 0 {
                    self.error = Some("no bots selected".into());
                    return AppAction::None;
                }
                return AppAction::MapWalkGroup;
            }
            return AppAction::ArmWalk(pending.requested);
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
        self.map.plane = self.map_model.plane;
    }

    /// Group-walk rows: every member with its host eligibility. The fleet's
    /// row selection is the group checklist: a row is checked when its
    /// member is selected in the fleet and eligible.
    pub fn refresh_walk_send(&mut self, eligibility: impl Fn(&str) -> WalkSlotStatus) {
        let names: Vec<String> = if !self.names.is_empty() {
            self.names.clone()
        } else {
            self.fleet.iter().map(|row| row.name.clone()).collect()
        };
        let mut rows = Vec::with_capacity(names.len());
        for name in names {
            let status = eligibility(&name);
            let checked = status.is_eligible()
                && self
                    .table
                    .selection
                    .contains(self.profile_id_for_name(&name));
            rows.push(WalkSendRow {
                name,
                status,
                checked,
            });
        }
        self.walk_send.rows = rows;
        self.walk_send.sync_walk_label();
    }

    /// Focused ↔ Group. Entering Group with no eligible row selected
    /// selects every eligible member (shown as `[x]` in the fleet).
    pub(crate) fn toggle_walk_send_mode(&mut self) {
        self.walk_send.mode = match self.walk_send.mode {
            WalkSendMode::Focused => WalkSendMode::Group,
            WalkSendMode::Group => WalkSendMode::Focused,
        };
        if self.walk_send.mode == WalkSendMode::Group && self.walk_send.checked_count() == 0 {
            let eligible_ids = self
                .walk_send
                .rows
                .iter()
                .filter(|row| row.status.is_eligible())
                .map(|row| self.profile_id_for_name(&row.name))
                .collect::<Vec<_>>();
            for row in &mut self.walk_send.rows {
                if row.status.is_eligible() {
                    row.checked = true;
                }
            }
            self.table.selection.mark_all(eligible_ids);
        }
        self.walk_send.sync_walk_label();
    }

    /// Space on the map: select / unselect the selected bot for the group.
    pub(crate) fn toggle_walk_send_focused(&mut self) {
        let Some(focused) = self.focused_name() else {
            return;
        };
        let identity = self.profile_id_for_name(&focused);
        if let Some(row) = self
            .walk_send
            .rows
            .iter_mut()
            .find(|row| row.name == focused)
        {
            if row.status.is_eligible() {
                row.checked = !row.checked;
                self.table.selection.set(identity, row.checked);
            }
        }
        self.walk_send.sync_walk_label();
    }

    fn observed_marks(&self) -> Vec<ObservedMark> {
        self.map_observed
            .iter()
            .map(|obs| ObservedMark {
                x: obs.tile.x,
                z: obs.tile.z,
                level: obs.tile.level,
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
        self.map_host_catalogue = None;
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

    /// Retain E's catalogue owner. Search/draw borrow it; no POI payload copy.
    pub fn bind_host_catalogue(&mut self, catalogue: Arc<Catalogue>) {
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
        if let Some(ctx) = self.map_model.context() {
            let mut next = ctx;
            next.overlay = Some(catalogue.key());
            self.map_model.bind(next);
        }
        self.map_host_catalogue = Some(catalogue);
        self.map_pois.clear();
        self.map_coverage = coverage;
        self.map_catalogue_status = MapCatalogueStatus::Ready;
        self.update_map_search();
    }

    pub fn set_map_unavailable(&mut self, reason: impl Into<String>) {
        self.map_catalogue_status = MapCatalogueStatus::Unavailable;
        self.map_coverage = reason.into();
        self.map_pois.clear();
        self.map_host_catalogue = None;
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
            KeyCode::Char(' ') => self.toggle_walk_send_focused(),
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

    /// Route keys to the loadouts popup when it is open.
    pub fn loadouts_on_key(&mut self, store: &mut script::LoadoutsStore, key: KeyEvent) -> bool {
        if !self.loadouts_state.open {
            return false;
        }
        let mut pane = LoadoutsPane {
            store,
            state: &mut self.loadouts_state,
        };
        if pane.state.name_scratch.is_empty() && !pane.store.loadouts().is_empty() {
            pane.sync_scratch_from_selection();
        }
        let _ = pane.on_key(key);
        true
    }

    /// Route keys to the params popup when it is open. Edits go through
    /// `commit`; Apply-to-all keys come back as actions.
    pub fn params_on_key(
        &mut self,
        commit: &mut ParamsCommit<'_>,
        loadouts: &script::LoadoutsStore,
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
            schema: &self.params_schema,
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
    pub fn params_card(&self) -> Option<script::ScriptSel> {
        self.script_sel.clone()
    }

    /// Open the params popup over `bag`, the focused profile's merged bag
    /// for the selected card.
    pub fn open_script_params(&mut self, bag: serde_json::Map<String, serde_json::Value>) {
        if self.params_card().is_none()
            || self.params_unavailable.is_some()
            || self.params_schema.is_empty()
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
    pub fn open_script_load_browser(&mut self, last_dir: Option<&std::path::Path>) {
        self.script_load_dir = default_load_browse_dir(last_dir);
        self.script_load_sel = 0;
        self.script_load_open = true;
    }

    /// First-run catalog folder browser keys.
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

    /// Move the Browse selection `step` cards through the grouped list
    /// (wrapping).
    fn move_script_sel(&mut self, step: i32) {
        let deferred = script::rs2b0t_import_deferred();
        let lines = browse_lines(&self.script_cards, &self.script_category_order, deferred);
        let card_indices: Vec<usize> = lines
            .iter()
            .filter_map(|l| match l {
                BrowseLine::Card(i) => Some(*i),
                _ => None,
            })
            .collect();
        if card_indices.is_empty() {
            return;
        }
        let pos = self
            .script_sel
            .as_ref()
            .and_then(|sel| match sel {
                ScriptSel::Compiled(id) => self
                    .script_cards
                    .iter()
                    .position(|c| card_selection(c) == Some(ScriptSel::Compiled(*id))),
                ScriptSel::Loaded(source, name) => self
                    .script_cards
                    .iter()
                    .position(|c| c.source == *source && c.name == *name),
            })
            .and_then(|card_idx| card_indices.iter().position(|&i| i == card_idx))
            .unwrap_or(0);
        let next = (pos as i32 + step).rem_euclid(card_indices.len() as i32) as usize;
        let card = &self.script_cards[card_indices[next]];
        self.script_sel = card_selection(card);
        self.browse_changed = true;
    }

    /// Browse, the Load file browser or the catalog folder prompt is open.
    /// Each is drawn in the Script tab and, like an overlay, owns every key
    /// and click until it closes.
    pub(crate) fn script_popup_open(&self) -> bool {
        self.script_load_open || self.rs2b0t_catalog_open || self.script_browse_open
    }

    /// Keys while a Script popup is open: its own navigation, Enter and
    /// Esc act; every other key is ignored, so no global chord, other
    /// pane's letter or script letter gets past it. `None` when no popup is
    /// open. Load sits over the catalog prompt, which sits over Browse.
    pub(crate) fn script_popup_key(&mut self, key: KeyEvent) -> Option<AppAction> {
        if self.script_load_open {
            return Some(self.load_on_key(key));
        }
        if self.rs2b0t_catalog_open {
            return Some(self.catalog_on_key(key));
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
            // The pick stays for Start.
            KeyCode::Enter | KeyCode::Esc => self.script_browse_open = false,
            _ => {}
        }
        Some(AppAction::None)
    }

    /// A click while a Script popup is open. Inside the Script pane it
    /// picks a row or presses one of the pane's buttons; outside it reaches
    /// nothing: Browse and Load close (the pick stays), the catalog prompt
    /// stays open because dismissing it means Not now.
    pub(crate) fn script_popup_click(&mut self, col: u16, row: u16) -> AppAction {
        if crate::layout::contains(self.script_area, col, row) {
            return self.script_click(col, row);
        }
        if !self.rs2b0t_catalog_open {
            self.script_load_open = false;
            self.script_browse_open = false;
        }
        AppAction::None
    }

    /// The wheel over anything while a Script popup is open moves that
    /// popup's list (`delta` -1 up, 1 down).
    pub(crate) fn script_popup_scroll(&mut self, delta: isize) {
        if self.script_load_open {
            let last = self.load_entries().len().saturating_sub(1);
            self.script_load_sel = self.script_load_sel.saturating_add_signed(delta).min(last);
        } else if self.rs2b0t_catalog_open {
            let last = self.catalog_entries().len().saturating_sub(1);
            self.catalog_sel = self.catalog_sel.saturating_add_signed(delta).min(last);
        } else if self.script_browse_open {
            self.move_script_sel(delta as i32);
        }
    }

    /// The script pane's clicks: a button runs its command (the same path
    /// as its key and the palette), picker rows store the selection.
    pub(crate) fn script_click(&mut self, col: u16, row: u16) -> AppAction {
        if self.script_load_open {
            return self.load_click(col, row);
        }
        if self.rs2b0t_catalog_open {
            return self.catalog_click(col, row);
        }
        let deferred = script::rs2b0t_import_deferred();
        let pane = ScriptPane::new(
            self.script_state,
            self.script_sel.as_ref(),
            &self.script_cards,
            &self.script_category_order,
            deferred,
            self.script_browse_open,
            self.script_load_open,
            "",
            !self.params_schema.is_empty(),
            None,
        )
        .with_reload_confirm(self.reload_confirm);
        match pane.on_click(self.script_area, col, row) {
            ScriptClick::Button(label) => {
                script_command(label).map_or(AppAction::None, |command| self.run_command(command))
            }
            ScriptClick::Params => self.run_command(Command::ScriptParams),
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

    /// One script command after the router checked it is available:
    /// Browse toggles the picker, Start carries the heading card, Load
    /// opens the file browser, the rest map to their actions.
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
            Command::ScriptStart => match self.script_sel.clone() {
                Some(sel) => AppAction::ScriptStart(sel),
                None => {
                    self.error = Some("script: browse to pick one first".into());
                    AppAction::None
                }
            },
            Command::ScriptPause => AppAction::ScriptPause,
            Command::ScriptStop => AppAction::ScriptStop,
            Command::ScriptLoad => {
                let last = self.script_load_last_dir.clone();
                self.open_script_load_browser(last.as_deref());
                AppAction::None
            }
            Command::ScriptParams => AppAction::ScriptParams,
            Command::ScriptReload => AppAction::ScriptReload,
            Command::ScriptReloadCancel => AppAction::ScriptReloadCancel,
            Command::ImportCatalog => AppAction::ScriptImportCatalog,
            _ => AppAction::None,
        }
    }

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
    pub fn draw_loadouts_overlay(
        &mut self,
        frame: &mut Frame<'_>,
        store: &mut script::LoadoutsStore,
    ) {
        if !self.loadouts_state.open {
            return;
        }
        let mut pane = LoadoutsPane {
            store,
            state: &mut self.loadouts_state,
        };
        if pane.state.name_scratch.is_empty() && !pane.store.loadouts().is_empty() {
            pane.sync_scratch_from_selection();
        }
        frame.render_widget(pane, frame.area());
    }

    /// Render the params popup overlay (call after [`Self::draw`]).
    pub fn draw_params_overlay(
        &mut self,
        frame: &mut Frame<'_>,
        loadouts: &script::LoadoutsStore,
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
            schema: &self.params_schema,
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
        let catalogue = self.map_host_catalogue.clone();
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
                        let mark = if row.checked { "*" } else { " " };
                        match row.status {
                            WalkSlotStatus::Eligible(_) => {
                                format!("{mark}{}", row.name)
                            }
                            WalkSlotStatus::Excluded(reason) => {
                                format!("{mark}{} ({reason})", row.name)
                            }
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
                        obs.kind, obs.tile.x, obs.tile.z, obs.tile.level
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
        let mem = frontend_core::MemoryNotice::status_text(self.settings.lowmem, self.memory);
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
            script::rs2b0t_import_deferred(),
            self.script_browse_open,
            self.script_load_open,
            "",
            !self.params_schema.is_empty(),
            None,
        )
        .with_reload_confirm(self.reload_confirm)
        .with_native_status(
            self.focused_detail()
                .and_then(|detail| detail.native_status.as_deref()),
        )
        .with_walk_risk(
            self.focused_detail()
                .and_then(|detail| detail.walk_risk.as_deref()),
        );
        frame.render_widget(pane, area);
    }
}

#[cfg(test)]
#[path = "app_tests.rs"]
mod tests;
