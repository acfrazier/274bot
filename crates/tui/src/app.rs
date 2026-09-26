//! `TuiApp`: view model for the headless panel. The binary (`tui-play`)
//! polls slot statuses and the focused snapshot each frame, refreshes the
//! app, routes keys/clicks, and dispatches the returned [`AppAction`] onto
//! `host_play::Play`. The panes are plain widgets over owned view data
//! (copied from the snapshot each pump), so CI renders them with
//! `TestBackend` and no real terminal.

use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Position, Rect};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Paragraph, Widget, Wrap};
use ratatui::Frame;

use api::snapshot::{ChatLineView, ChatOptionView, WorldTile};
use frontend_core::MapBakeChoice;
use host_play::walk_map::{Catalogue, MapModel, ObservedService, Search, WalkSlotStatus};
use host_play::{ResourceView, SlotStatus};
use nav::map::poi::{PoiKind, PoiRecord};
use nav::router::{FindOptions, Route};
use nav::tile::Tile;
use nav::world::NavWorld;
use script::{RunState, ScriptSel};

use crate::chat::{chat_modal_open, Chat, ChatAction, ChatState, ChatView};
use crate::loadouts::{LoadoutsPane, LoadoutsState};
use crate::map::{Map, MapAction, MapView, ObservedMark};
use crate::script_params::{ParamsKey, ParamsPane, ParamsState};
use crate::script_shape::{
    browse_lines, browse_section_height, rs2b0t_root_has_index, BrowseCard, BrowseLine,
    ScriptClick, ScriptPane,
};
use crate::settings::{SettingsKey, SettingsPane, SettingsState};
use crate::status::StatusPane;

/// Cap Browse detail lines so a small terminal keeps map/status room.
const MAX_BROWSE_LINES: u16 = 14;

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
    /// Switch the focused slot to `name` (mirror onto `Play::focus`).
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
    /// Chat modal advance: queue `WireCmd::Continue` / `Answer`.
    Chat(ChatAction),
    /// MultiBox: load every vault profile and log every member in.
    SpawnAll,
    /// Log in the focused member (explicit handshake).
    Login,
    /// Log out the focused member (it stays loaded, latched).
    Logout,
    /// Log out every member.
    LogoutAll,
    /// Remove the focused member from the fleet (clean logout, then stop).
    Remove,
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
    /// Nothing to dispatch.
    None,
}

/// Session nav find opt-ins (panel `NavSettings` parity for Walk-confirm).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NavFindSettings {
    pub allow_teleports: bool,
    pub allow_wilderness: bool,
    pub allow_bank_fetch: bool,
}

impl NavFindSettings {
    /// The [`FindOptions`] Walk-confirm passes to [`host_play::arm_walk_on`].
    pub fn find_options(self) -> FindOptions {
        FindOptions {
            allow_teleports: self.allow_teleports,
            allow_wilderness: self.allow_wilderness,
            allow_bank_fetch: self.allow_bank_fetch,
            ..FindOptions::default()
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

    fn view(&self) -> ChatView<'_> {
        ChatView {
            lines: &self.lines,
            modal_texts: &self.modal_texts,
            options: &self.options,
            has_continue: self.has_continue,
            script_paint: self.script_paint.as_deref(),
            show_game_chat: self.show_game_chat,
        }
    }

    fn paint_buttons_active(&self) -> bool {
        !self.is_modal_open()
            && !self.show_game_chat
            && self
                .script_paint
                .as_ref()
                .is_some_and(|p| !p.buttons.is_empty())
    }
}

/// Headless panel view model.
pub struct TuiApp {
    title: String,
    /// MultiBox slot list (vault profile names, plus running slots) and
    /// the focused slot.
    pub names: Vec<String>,
    pub focused: Option<usize>,
    /// Polled slot statuses; the binary refreshes them each frame.
    pub statuses: Vec<SlotStatus>,
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
    /// The armed walk route whose remaining tiles paint `*`.
    pub route: Option<Route>,
    /// Armed Walk dest after the host accepts. Refused Walks leave this unset.
    pub walk_dest: Option<Tile>,
    /// Chat pane state (focused option row).
    pub chat: ChatState,
    /// Settings popup over the focused profile's settings; the binary
    /// persists [`TuiApp::settings`] back to the vault when
    /// [`TuiApp::settings_dirty`] flips.
    pub settings: vault::ProfileSettings,
    /// Walk-confirm find opt-ins (teleports / wilderness / BankBudget).
    pub nav: NavFindSettings,
    pub settings_state: SettingsState,
    pub settings_dirty: bool,
    /// Remembered WalkTo terrain-bake choice (shared `panel-ui.json` key).
    pub map_bake: MapBakeChoice,
    /// The settings popup changed [`Self::map_bake`]; the binary persists it.
    pub map_bake_dirty: bool,
    pub loadouts_state: LoadoutsState,
    /// The focused slot's script lifecycle (shape display only).
    pub script_state: RunState,
    /// The Browse-selected script card; Start keys on `(source, name)`.
    pub script_sel: Option<ScriptSel>,
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
    /// Working bag while the params popup is open.
    pub params_bag: serde_json::Map<String, serde_json::Value>,
    pub params_state: ParamsState,
    pub quit: bool,
    /// The last walk/settings/map error shown in the strip.
    pub error: Option<String>,
    /// Process resource snapshot (same sampler as the panel resource card).
    pub resources: ResourceView,
    /// One-line background-bots notice until the operator acks it.
    pub background_notice: Option<String>,
    /// F7 log pane over the shared structured log.
    pub log: crate::log_pane::LogPaneState,
    /// Last draw rects for click hit-testing.
    pub chat_area: Rect,
    pub script_area: Rect,
}

impl TuiApp {
    /// New app showing `title`; no slots, no map, no snapshot.
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            names: Vec::new(),
            focused: None,
            statuses: Vec::new(),
            chat_data: ChatData::default(),
            inv_items: Vec::new(),
            stats_rows: Vec::new(),
            locs_near: Vec::new(),
            world: None,
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
            nav: NavFindSettings::default(),
            settings_state: SettingsState::default(),
            settings_dirty: false,
            map_bake: MapBakeChoice::Ask,
            map_bake_dirty: false,
            loadouts_state: LoadoutsState::default(),
            script_state: RunState::Idle,
            script_sel: None,
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
            params_bag: serde_json::Map::new(),
            params_state: ParamsState::default(),
            quit: false,
            error: None,
            resources: ResourceView::default(),
            background_notice: None,
            log: crate::log_pane::LogPaneState::default(),
            chat_area: Rect::default(),
            script_area: Rect::default(),
        }
    }

    /// The focused slot's status row, `None` when nothing is focused or
    /// the slot has not published a row yet.
    pub fn focused_status(&self) -> Option<&SlotStatus> {
        let name = self.focused_name()?;
        self.statuses.iter().find(|s| s.username == name)
    }

    /// The focused slot's username.
    pub fn focused_name(&self) -> Option<String> {
        self.focused.and_then(|i| self.names.get(i)).cloned()
    }

    /// Re-sync the view from freshly polled statuses: the focused slot's
    /// `here` tile (the map re-centres on it when the view is not panned).
    pub fn refresh(&mut self) {
        self.here = self
            .focused_status()
            .and_then(SlotStatus::ready_tile)
            .map(|(x, z, level)| WorldTile { x, z, level });
        if !self.map_active {
            if let Some(here) = self.here {
                self.map.plane = here.level.clamp(0, 3) as u8;
            }
        }
    }

    /// The chat pane's keys, when a modal is open. Space/Enter/click →
    /// `continue_dialog` / `answer_choice`; Up/Down (j/k) move the option
    /// focus.
    fn chat_on_key(&mut self, key: KeyEvent) -> Option<AppAction> {
        let mut chat = Chat::new(self.chat_data.view(), &mut self.chat, |_| {});
        match chat.on_key(key) {
            action @ (ChatAction::Continue
            | ChatAction::Answer(_)
            | ChatAction::PaintButton(_)
            | ChatAction::PaintChrome(_)) => Some(AppAction::Chat(action)),
            ChatAction::None => None,
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

    fn search_hit_label(&self, index: usize) -> Option<String> {
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
                entry.name(),
                anchor.x,
                anchor.z,
                anchor.level
            ));
        }
        let poi = self.map_pois.get(index)?;
        Some(format!(
            "{} {} ({},{},{})",
            kind_glyph(poi.kind),
            poi.name.as_str(),
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

    fn map_search_on_key(&mut self, key: KeyEvent) -> AppAction {
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

    fn select_requested(&mut self, requested: Tile) {
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
    fn map_enter(&mut self) -> AppAction {
        if let Some(pending) = self.map_model.pending() {
            if self.walk_send.mode == WalkSendMode::Group {
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

    fn set_map_plane(&mut self, plane: u8) {
        let plane = plane.min(3);
        self.map.plane = plane;
        self.map.selection = None;
        self.map_poi_sel = None;
        let _ = self.map_model.set_plane(plane);
        self.map_model.clear_selection();
    }

    fn recenter_map(&mut self) {
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

    pub fn refresh_walk_send(&mut self, eligibility: impl Fn(&str) -> WalkSlotStatus) {
        let names: Vec<String> = if !self.names.is_empty() {
            self.names.clone()
        } else {
            self.statuses.iter().map(|s| s.username.clone()).collect()
        };
        let prev: std::collections::HashMap<String, bool> = self
            .walk_send
            .rows
            .iter()
            .map(|row| (row.name.clone(), row.checked))
            .collect();
        let mut rows = Vec::with_capacity(names.len());
        for name in names {
            let status = eligibility(&name);
            let checked =
                status.is_eligible() && prev.get(&name).copied().unwrap_or(status.is_eligible());
            rows.push(WalkSendRow {
                name,
                status,
                checked,
            });
        }
        self.walk_send.rows = rows;
        self.walk_send.sync_walk_label();
    }

    fn toggle_walk_send_mode(&mut self) {
        self.walk_send.mode = match self.walk_send.mode {
            WalkSendMode::Focused => WalkSendMode::Group,
            WalkSendMode::Group => WalkSendMode::Focused,
        };
        if self.walk_send.mode == WalkSendMode::Group && self.walk_send.checked_count() == 0 {
            for row in &mut self.walk_send.rows {
                row.checked = row.status.is_eligible();
            }
        }
        self.walk_send.sync_walk_label();
    }

    fn toggle_walk_send_focused(&mut self) {
        let Some(focused) = self.focused_name() else {
            return;
        };
        if let Some(row) = self
            .walk_send
            .rows
            .iter_mut()
            .find(|row| row.name == focused)
        {
            if row.status.is_eligible() {
                row.checked = !row.checked;
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

    fn map_open(&mut self) -> AppAction {
        self.map_active = true;
        if self.map_catalogue_status == MapCatalogueStatus::Inactive {
            self.map_catalogue_status = MapCatalogueStatus::Unavailable;
            self.map_coverage =
                "coverage: catalogue unavailable: cache demand manager is not bound; terrain imagery unavailable"
                    .into();
        }
        AppAction::MapOpen
    }

    fn map_close(&mut self) -> AppAction {
        self.map_active = false;
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

    /// Cycle the focus to the next running slot (the strip's `[Tab]`).
    /// Returns the newly focused name so the binary can mirror it onto
    /// `Play::focus` — the app's index alone would leave the session on
    /// the boot slot's sample gate.
    fn cycle_focus(&mut self) -> Option<String> {
        // Running members only: a removed slot still logging out is not in
        // the strip and must not be focusable.
        let running: Vec<&str> = self
            .statuses
            .iter()
            .map(|s| s.username.as_str())
            .filter(|name| self.names.iter().any(|n| n == name))
            .collect();
        if running.is_empty() {
            return None;
        }
        let current = self.focused_name();
        let pos = current
            .as_deref()
            .and_then(|c| running.iter().position(|r| *r == c))
            .map(|p| p + 1)
            .unwrap_or(0)
            % running.len();
        let name = running[pos];
        if let Some(i) = self.names.iter().position(|n| n == name) {
            self.focused = Some(i);
            return Some(name.to_string());
        }
        None
    }

    /// The slot the strip click selects. The strip is
    /// `[{names joined by " "}] …`, so name `i` starts one cell after the
    /// leading `[` plus every earlier `len + 1` span. Sets `app.focused`
    /// to the clicked name's index — the same bookkeeping [`cycle_focus`]
    /// does — and returns the name so the binary can mirror it onto
    /// `Play::focus`. Without the app-side update the UI would keep
    /// driving the old slot while the session samples the new one.
    fn strip_select(&mut self, col: u16) -> Option<String> {
        let mut cursor = 1u16; // after the leading `[`
        for (i, name) in self.names.iter().enumerate() {
            let len = name.len() as u16;
            if col >= cursor && col < cursor + len {
                self.focused = Some(i);
                return Some(name.clone());
            }
            cursor += len + 1;
        }
        None
    }

    /// One key event. Inputs opened by Map (search/coordinate) consume
    /// typing before global shortcuts. Outside explicit Map focus, existing
    /// direct WASD remains available for compatibility; map pan keys do not.
    pub fn on_key(&mut self, key: KeyEvent) -> AppAction {
        if self.quit {
            return AppAction::None;
        }
        if key.code == KeyCode::F(7) && !self.log.open {
            self.log.open = true;
            return AppAction::None;
        }
        if self.log.open {
            let focused = self.focused.and_then(|i| self.names.get(i));
            self.log.on_key(key, focused.map(String::as_str));
            return AppAction::None;
        }
        if self.params_state.open {
            return AppAction::None;
        }
        if self.script_load_open {
            return self.load_on_key(key);
        }
        if self.rs2b0t_catalog_open {
            return self.catalog_on_key(key);
        }
        if self.script_browse_open {
            return self.script_browse_on_key(key);
        }

        if key.code == KeyCode::F(4) {
            return if self.map_active {
                self.map_close()
            } else {
                self.map_open()
            };
        }
        if self.map_active {
            if self.map_search_open {
                return self.map_search_on_key(key);
            }
            if self.settings_state.open || self.loadouts_state.open {
                return AppAction::None;
            }
            if self.chat_data.is_modal_open() {
                return self.chat_on_key(key).unwrap_or(AppAction::None);
            }
            match key.code {
                KeyCode::Char('/') => {
                    self.map_search_open = true;
                    self.map_search.clear();
                    self.map_search_results.clear();
                    self.map_search_sel = 0;
                    return AppAction::None;
                }
                KeyCode::Char('R') => {
                    self.recenter_map();
                    return AppAction::None;
                }
                KeyCode::Esc => {
                    if self.map.selection.is_some() {
                        return self.map_on_key(key);
                    }
                    return self.map_close();
                }
                KeyCode::PageUp => {
                    self.set_map_plane(self.map.plane.saturating_add(1));
                    return AppAction::None;
                }
                KeyCode::PageDown => {
                    self.set_map_plane(self.map.plane.saturating_sub(1));
                    return AppAction::None;
                }
                KeyCode::Char('0'..='3') => {
                    if let KeyCode::Char(level) = key.code {
                        self.set_map_plane(level as u8 - b'0');
                    }
                    return AppAction::None;
                }
                KeyCode::Char('d') => {
                    self.map.layers.dots = !self.map.layers.dots;
                    return AppAction::None;
                }
                KeyCode::Char('c') => {
                    self.map.layers.collision = !self.map.layers.collision;
                    return AppAction::None;
                }
                KeyCode::Char('r') => {
                    self.map.layers.reach = !self.map.layers.reach;
                    return AppAction::None;
                }
                KeyCode::Char('g') => {
                    self.toggle_walk_send_mode();
                    return AppAction::None;
                }
                KeyCode::Char(' ') => {
                    self.toggle_walk_send_focused();
                    return AppAction::None;
                }
                KeyCode::Tab => {
                    return self
                        .cycle_focus()
                        .map(AppAction::Focus)
                        .unwrap_or(AppAction::None);
                }
                KeyCode::Char('t') => {
                    let Some(pending) = self.map_model.pending() else {
                        return AppAction::None;
                    };
                    return AppAction::MapTeleport(pending.requested);
                }
                KeyCode::Enter => return self.map_enter(),
                _ => {}
            }
            return self.map_on_key(key);
        }

        match key.code {
            KeyCode::Char('q') => {
                self.quit = true;
                return AppAction::Quit;
            }
            KeyCode::Char('o') => {
                self.settings_state.open = !self.settings_state.open;
                return AppAction::None;
            }
            // `l` is a loadouts key only outside Map focus. Inside Map it is
            // the east-pan binding, eliminating the old global conflict.
            KeyCode::Char('l') | KeyCode::Char('L') => {
                self.loadouts_state.open = !self.loadouts_state.open;
                if self.loadouts_state.open {
                    self.loadouts_state.sel = 0;
                    self.loadouts_state.name_scratch.clear();
                    self.loadouts_state.worn_scratch.clear();
                    self.loadouts_state.carry_scratch.clear();
                }
                return AppAction::None;
            }
            KeyCode::Char('m') => return AppAction::SpawnAll,
            KeyCode::Char('i') => return AppAction::Login,
            KeyCode::Char('u') => return AppAction::Logout,
            KeyCode::Char('U') => return AppAction::LogoutAll,
            KeyCode::Char('x') => return AppAction::Remove,
            KeyCode::Char('p') => {
                if self.chat_data.script_paint.is_some() {
                    self.chat_data.show_game_chat = !self.chat_data.show_game_chat;
                }
                return AppAction::None;
            }
            KeyCode::Tab => {
                return self
                    .cycle_focus()
                    .map(AppAction::Focus)
                    .unwrap_or(AppAction::None)
            }
            _ => {}
        }
        if self.settings_state.open {
            let mut pane = SettingsPane::new(
                &mut self.settings,
                &mut self.nav,
                &mut self.map_bake,
                &mut self.settings_state,
            );
            match pane.on_key(key) {
                SettingsKey::Changed => self.settings_dirty = true,
                SettingsKey::MapBake => self.map_bake_dirty = true,
                SettingsKey::Consumed | SettingsKey::Ignored => {}
            }
            return AppAction::None;
        }
        if self.loadouts_state.open {
            return AppAction::None;
        }
        if self.chat_data.is_modal_open() {
            return self.chat_on_key(key).unwrap_or(AppAction::None);
        }
        if self.chat_data.paint_buttons_active() {
            match key.code {
                KeyCode::Up
                | KeyCode::Down
                | KeyCode::Enter
                | KeyCode::Char(' ')
                | KeyCode::Char('j')
                | KeyCode::Char('k')
                | KeyCode::Char('1')
                | KeyCode::Char('2')
                | KeyCode::Char('3')
                | KeyCode::Char('4')
                | KeyCode::Char('5')
                | KeyCode::Char('6')
                | KeyCode::Char('7')
                | KeyCode::Char('8')
                | KeyCode::Char('9') => {
                    return self.chat_on_key(key).unwrap_or(AppAction::None);
                }
                _ => {}
            }
        }
        if let Some(here) = self.here {
            if let Some((x, z, level)) = wasd_target((here.x, here.z, here.level), key.code) {
                return AppAction::WalkTile(Tile { x, z, level });
            }
        }
        if key.code == KeyCode::Esc
            && self.background_notice.is_some()
            && self.map.selection.is_none()
        {
            return AppAction::AckBackground;
        }
        AppAction::None
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

    /// Route keys to the params popup when it is open; returns whether the
    /// key was consumed.
    pub fn params_on_key(
        &mut self,
        store: &mut script::ScriptSettingsStore,
        loadouts: &script::LoadoutsStore,
        game_data: Option<&api::game_data::SelectedGameData>,
        key: KeyEvent,
    ) -> bool {
        if !self.params_state.open {
            return false;
        }
        let Some((source, name)) = self.params_script_sel() else {
            self.params_state.open = false;
            return true;
        };
        let schema = self.params_schema.clone();
        let mut pane = ParamsPane {
            schema: &schema,
            bag: &mut self.params_bag,
            store,
            loadouts,
            game_data,
            source,
            name: &name,
            state: &mut self.params_state,
        };
        if pane.on_key(key.code) == ParamsKey::Close {
            self.params_state.open = false;
        }
        true
    }

    fn params_script_sel(&self) -> Option<(script::ScriptSource, String)> {
        match self.script_sel.as_ref()? {
            script::ScriptSel::Loaded(source, name) => Some((*source, name.clone())),
            _ => None,
        }
    }

    /// Open the params popup for the Browse-selected card.
    pub fn open_script_params(&mut self, store: &script::ScriptSettingsStore) {
        let Some((source, name)) = self.params_script_sel() else {
            return;
        };
        if self.params_schema.is_empty() {
            return;
        }
        self.params_bag = store.merged_bag(source, &name, &self.params_schema, None);
        self.params_state = ParamsState {
            open: true,
            cursor: 0,
            ..Default::default()
        };
    }

    /// The merged settings bag Start would post for the selected card.
    pub fn merged_script_settings_bag(
        &self,
        store: &script::ScriptSettingsStore,
    ) -> Option<serde_json::Map<String, serde_json::Value>> {
        let (source, name) = self.params_script_sel()?;
        if self.params_schema.is_empty() {
            return None;
        }
        Some(store.merged_bag(source, &name, &self.params_schema, None))
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

    /// The Browse picker's keys: Up/Down (j/k) cycle the card selection,
    /// Enter/Esc close the picker (the selection stays for Start).
    fn script_browse_on_key(&mut self, key: KeyEvent) -> AppAction {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.move_script_sel(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_script_sel(1),
            KeyCode::Enter | KeyCode::Esc => self.script_browse_open = false,
            _ => {}
        }
        AppAction::None
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
                ScriptSel::Loaded(source, name) => self
                    .script_cards
                    .iter()
                    .position(|c| c.source == *source && c.name == *name),
                _ => None,
            })
            .and_then(|card_idx| card_indices.iter().position(|&i| i == card_idx))
            .unwrap_or(0);
        let next = (pos as i32 + step).rem_euclid(card_indices.len() as i32) as usize;
        let card = &self.script_cards[card_indices[next]];
        self.script_sel = Some(ScriptSel::Loaded(card.source, card.name.clone()));
    }

    /// One mouse click (crossterm col/row). The strip selects a slot; the
    /// chat pane answers options / continues; the script pane answers its
    /// buttons and the Browse picker rows.
    pub fn on_click(&mut self, col: u16, row: u16) -> AppAction {
        if self.params_state.open || self.log.open {
            return AppAction::None;
        }
        if self.settings_state.open {
            return AppAction::None;
        }
        // The slot strip is the top row: clicking a name focuses that
        // slot (mirrored onto `Play::focus` by the binary, like Tab).
        if row == 0 {
            return self
                .strip_select(col)
                .map(AppAction::Focus)
                .unwrap_or(AppAction::None);
        }
        if self.chat_area.contains(Position::new(col, row)) {
            let mut chat = Chat::new(self.chat_data.view(), &mut self.chat, |_| {});
            match chat.on_click(self.chat_area, col, row) {
                action @ (ChatAction::Continue
                | ChatAction::Answer(_)
                | ChatAction::PaintButton(_)
                | ChatAction::PaintChrome(_)) => return AppAction::Chat(action),
                ChatAction::None => {}
            }
        }
        if self.script_area.contains(Position::new(col, row)) {
            return self.script_click(col, row);
        }
        AppAction::None
    }

    /// The script pane's clicks: Browse toggles the picker, Start emits
    /// [`AppAction::ScriptStart`] with the selected card (an error when
    /// nothing is selected), Pause/Stop emit their actions, Load opens
    /// the path input, and picker rows store the selection.
    fn script_click(&mut self, col: u16, row: u16) -> AppAction {
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
        );
        match pane.on_click(self.script_area, col, row) {
            ScriptClick::Button("Browse") => {
                let opening = !self.script_browse_open;
                self.script_browse_open = opening;
                if opening {
                    AppAction::ScriptBrowse
                } else {
                    AppAction::None
                }
            }
            ScriptClick::Button("Start") => match self.script_sel.clone() {
                Some(sel) => AppAction::ScriptStart(sel),
                None => {
                    self.error = Some("script: browse to pick one first".into());
                    AppAction::None
                }
            },
            ScriptClick::Button("Pause") | ScriptClick::Button("Resume") => AppAction::ScriptPause,
            ScriptClick::Button("Stop") => AppAction::ScriptStop,
            ScriptClick::Button("Load") => {
                let last = self.script_load_last_dir.clone();
                self.open_script_load_browser(last.as_deref());
                AppAction::None
            }
            ScriptClick::Params => AppAction::ScriptParams,
            ScriptClick::Button(_) => AppAction::None,
            ScriptClick::ImportCatalog => AppAction::ScriptImportCatalog,
            ScriptClick::Pick(idx) => {
                if let Some(card) = self.script_cards.get(idx) {
                    self.script_sel = Some(ScriptSel::Loaded(card.source, card.name.clone()));
                }
                AppAction::None
            }
            ScriptClick::None => AppAction::None,
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

    /// Render the full layout (spec): slot strip, map, chat, status |
    /// inv/stats/locs, script shape, then the settings popup overlay.
    pub fn draw(&mut self, frame: &mut Frame<'_>) {
        let area = frame.area();
        // The script pane grows for the Browse picker rows and the Load
        // path line (capped so a small terminal keeps map/status room).
        let browse_lines = if self.script_browse_open && !self.rs2b0t_catalog_open {
            browse_lines(
                &self.script_cards,
                &self.script_category_order,
                script::rs2b0t_import_deferred(),
            )
        } else {
            Vec::new()
        };
        let browse_h = if self.script_browse_open && !self.rs2b0t_catalog_open {
            browse_section_height(&browse_lines, &self.script_cards).min(MAX_BROWSE_LINES)
        } else {
            0
        };
        let catalog_h = if self.rs2b0t_catalog_open {
            (self.catalog_entries().len() as u16 + 3).min(16)
        } else {
            0
        };
        let load_h = if self.script_load_open {
            (self.load_entries().len() as u16 + 3).min(16)
        } else {
            0
        };
        let script_h = 4
            + browse_h
            + catalog_h
            + load_h
            + u16::from(self.script_load_open && load_h == 0)
            + u16::from(
                !self.params_schema.is_empty()
                    && !self.script_browse_open
                    && !self.rs2b0t_catalog_open
                    && !self.script_load_open,
            );
        let chat_h = self.chat_data.view().preferred_height();
        let chunks = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(8),
            Constraint::Length(chat_h),
            Constraint::Min(6),
            Constraint::Length(script_h),
        ])
        .split(area);

        self.draw_strip(frame, chunks[0]);
        self.draw_map(frame, chunks[1]);
        self.chat_area = chunks[2];
        self.draw_chat(frame, chunks[2]);
        let bottom = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(chunks[3]);
        self.draw_status(frame, bottom[0]);
        self.draw_inv_locs(frame, bottom[1]);
        self.script_area = chunks[4];
        self.draw_script(frame, chunks[4]);

        if self.settings_state.open {
            let pane = SettingsPane::new(
                &mut self.settings,
                &mut self.nav,
                &mut self.map_bake,
                &mut self.settings_state,
            );
            frame.render_widget(pane, area);
        }
        if self.log.open {
            let focused = self
                .focused
                .and_then(|i| self.names.get(i))
                .map(String::as_str);
            self.log.refresh(focused);
            let below_strip = Rect {
                y: area.y + 1,
                height: area.height.saturating_sub(1),
                ..area
            };
            frame.render_widget(
                crate::log_pane::LogPane {
                    state: &self.log,
                    focused,
                },
                below_strip,
            );
        }
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
        store: &mut script::ScriptSettingsStore,
        loadouts: &script::LoadoutsStore,
        game_data: Option<&api::game_data::SelectedGameData>,
    ) {
        if !self.params_state.open {
            return;
        }
        let Some((source, name)) = self.params_script_sel() else {
            return;
        };
        let schema = self.params_schema.clone();
        let pane = ParamsPane {
            schema: &schema,
            bag: &mut self.params_bag,
            store,
            loadouts,
            game_data,
            source,
            name: &name,
            state: &mut self.params_state,
        };
        frame.render_widget(pane, frame.area());
    }

    fn draw_strip(&mut self, frame: &mut Frame<'_>, area: Rect) {
        let focused = self.focused_name().unwrap_or_else(|| "_".into());
        let mut members = String::new();
        for name in &self.names {
            if !members.is_empty() {
                members.push(' ');
            }
            members.push_str(name);
            if let Some(number) = self
                .statuses
                .iter()
                .find(|s| &s.username == name)
                .and_then(|s| s.world)
            {
                members.push_str(&format!("(w{number})"));
            }
        }
        let mut text = format!(
            "[{members}]  focused: {focused}   {}   F4 map · F7 log · q quit · o options · l loadouts · Tab focus · m load+login all · i login · u logout · U logout all · x remove",
            self.title
        );
        if let Some(err) = &self.error {
            text.push_str(&format!("   !! {err}"));
        }
        let p = Paragraph::new(text).wrap(Wrap { trim: false });
        frame.render_widget(p, area);
    }

    fn draw_map(&mut self, frame: &mut Frame<'_>, area: Rect) {
        if !self.map_active {
            let block = Block::default()
                .borders(Borders::ALL)
                .title("Map (F4 activate; no catalogue demand)");
            frame.render_widget(block, area);
            return;
        }

        let title = format!(
            "Map · plane {} · {:?} · {} · arrows/hjkl pan · +/- zoom · / search · g group · t teleport · Esc close",
            self.map.plane,
            self.map_catalogue_status,
            self.walk_send.walk_label()
        );
        let block = Block::default().borders(Borders::ALL).title(title);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let [map_area, info_area] =
            Layout::vertical([Constraint::Min(1), Constraint::Length(inner.height.min(4))])
                .areas(inner);

        let observed = self.observed_marks();
        let catalogue = self.map_host_catalogue.clone();
        let poi_count = catalogue
            .as_ref()
            .map(|c| c.entries().len())
            .unwrap_or(self.map_pois.len());
        if let Some(world) = self.world.clone() {
            let mut map = Map::new(&world, &mut self.map, |_| {})
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
        let mut info = vec![
            Line::from(send),
            Line::from(if let Some(err) = &self.error {
                format!("status: {err}")
            } else {
                format!(
                    "status: {:?}  {}",
                    self.map_catalogue_status, self.map_coverage
                )
            }),
            Line::from(format!(
                "legend: @ * + B T X N#  POIs:{} obs:{} {}",
                poi_count,
                self.map_observed.len(),
                if self.route.is_some() {
                    "route"
                } else {
                    "none"
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

    fn draw_chat(&mut self, frame: &mut Frame<'_>, area: Rect) {
        let chat = Chat::new(self.chat_data.view(), &mut self.chat, |_| {});
        frame.render_widget(chat, area);
    }

    fn draw_status(&mut self, frame: &mut Frame<'_>, area: Rect) {
        let walk = self
            .walk_dest
            .map(|t| format!("{} {} {}", t.x, t.z, t.level))
            .unwrap_or_else(|| "—".into());
        let mem = if self.settings.lowmem {
            "lowmem"
        } else {
            "highmem"
        };
        let pane = StatusPane::new(self.focused_status(), &walk, mem)
            .resources(&self.resources)
            .notice(self.background_notice.as_deref());
        frame.render_widget(pane, area);
    }

    /// inv / stats / locs pane: the focused snapshot's inventory item
    /// names, used skill rows, and nearest named locs (name + Chebyshev).
    fn draw_inv_locs(&mut self, frame: &mut Frame<'_>, area: Rect) {
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

    fn draw_script(&mut self, frame: &mut Frame<'_>, area: Rect) {
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
        );
        frame.render_widget(pane, area);
    }
}

#[cfg(test)]
#[path = "app_tests.rs"]
mod tests;
