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
use host_play::SlotStatus;
use nav::router::{FindOptions, Route};
use nav::tile::Tile;
use nav::world::NavWorld;
use script::{RunState, ScriptSel};

use crate::chat::{chat_modal_open, Chat, ChatAction, ChatState, ChatView};
use crate::map::{Map, MapAction, MapView};
use crate::script_shape::{
    browse_lines, browse_section_height, rs2b0t_root_has_index, BrowseCard, BrowseLine, ScriptClick,
    ScriptPane,
};
use crate::script_params::{ParamsKey, ParamsPane, ParamsState};
use crate::settings::{SettingsKey, SettingsPane, SettingsState};
use crate::status::StatusPane;

/// Cap Browse detail lines so a small terminal keeps map/status room.
const MAX_BROWSE_LINES: u16 = 14;

fn default_catalog_browse_dir() -> std::path::PathBuf {
    std::env::var("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("/"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppAction {
    /// Quit the app.
    Quit,
    /// Switch the focused slot to `name` (mirror onto `Play::focus`).
    Focus(String),
    /// Map Walk-confirm: route `from` → tile and arm via `arm_walk_on`.
    ArmWalk(Tile),
    /// WASD one-tile walk: queue a `host_play::WireCmd::Walk`.
    WalkTile(Tile),
    /// Chat modal advance: queue `WireCmd::Continue` / `Answer`.
    Chat(ChatAction),
    /// MultiBox: spawn every vault profile that is not running yet.
    SpawnAll,
    /// Start the Browse-selected JS card on the focused slot:
    /// `tui-play` dispatches `Play::script_start_load` with the card's
    /// source and shape.
    ScriptStart(ScriptSel),
    /// Toggle pause/resume on the focused slot's script (`Play::script_pause`
    /// / `Play::script_resume`, like the panel's `script_toggle_pause`).
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
    /// The focused slot's script paint frame (copied from the status row
    /// each pump); the pane shows it instead of the ring while it is
    /// non-empty.
    pub script_paint: Option<script::shim::ScriptPaint>,
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
            script_paint: self.script_paint.as_ref(),
            show_game_chat: self.show_game_chat,
        }
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
    /// Map view state (pan/zoom/selection).
    pub map: MapView,
    /// The focused slot's observed world tile.
    pub here: Option<WorldTile>,
    /// The armed walk route whose remaining tiles paint `*`.
    pub route: Option<Route>,
    /// The operator's picked walk dest (the status row shows it even when
    /// no route could be found, like the panel's `walk_dest`).
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
    /// The Load path input is open (`script_load_path` is the typed path).
    pub script_load_open: bool,
    pub script_load_path: String,
    /// Selected card's settings schema (refreshed each pump from the library).
    pub params_schema: Vec<script::SettingDef>,
    /// Working bag while the params popup is open.
    pub params_bag: serde_json::Map<String, serde_json::Value>,
    pub params_state: ParamsState,
    pub quit: bool,
    /// The last walk/settings error shown in the strip.
    pub error: Option<String>,
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
            here: None,
            route: None,
            walk_dest: None,
            chat: ChatState::default(),
            settings: vault::ProfileSettings::default(),
            nav: NavFindSettings::default(),
            settings_state: SettingsState::default(),
            settings_dirty: false,
            script_state: RunState::Idle,
            script_sel: None,
            script_cards: Vec::new(),
            script_category_order: Vec::new(),
            script_browse_open: false,
            rs2b0t_catalog_open: false,
            rs2b0t_catalog_dir: default_catalog_browse_dir(),
            catalog_sel: 0,
            script_load_open: false,
            script_load_path: String::new(),
            params_schema: Vec::new(),
            params_bag: serde_json::Map::new(),
            params_state: ParamsState::default(),
            quit: false,
            error: None,
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
            .filter(|s| s.ingame)
            .map(|s| WorldTile {
                x: s.tile_x,
                z: s.tile_z,
                level: s.tile_level,
            });
    }

    /// The chat pane's keys, when a modal is open. Space/Enter/click →
    /// `continue_dialog` / `answer_choice`; Up/Down (j/k) move the option
    /// focus.
    fn chat_on_key(&mut self, key: KeyEvent) -> Option<AppAction> {
        let mut chat = Chat::new(self.chat_data.view(), &mut self.chat, |_| {});
        match chat.on_key(key) {
            action @ (ChatAction::Continue | ChatAction::Answer(_)) => {
                Some(AppAction::Chat(action))
            }
            ChatAction::None => None,
        }
    }

    /// The map pane's keys: pan (arrows/hjkl), zoom (+/-), Enter selects /
    /// confirms a walk, Esc clears the selection.
    fn map_on_key(&mut self, key: KeyEvent) -> AppAction {
        let Some(world) = self.world.clone() else {
            return AppAction::None;
        };
        let mut map = Map::new(&world, &mut self.map, |_| {});
        if let Some(here) = self.here {
            map = map.here(here);
        }
        if let Some(route) = &self.route {
            map = map.route(route);
        }
        match map.on_key(key) {
            MapAction::Walk(tile) => AppAction::ArmWalk(tile),
            MapAction::Moved | MapAction::Ignored => AppAction::None,
        }
    }

    /// Cycle the focus to the next running slot (the strip's `[Tab]`).
    /// Returns the newly focused name so the binary can mirror it onto
    /// `Play::focus` — the app's index alone would leave the session on
    /// the boot slot's sample gate.
    fn cycle_focus(&mut self) -> Option<String> {
        let running: Vec<&str> = self.statuses.iter().map(|s| s.username.as_str()).collect();
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

    /// One key event. Priority: script load input / browse picker,
    /// global keys, the settings popup, the chat modal (when open), then
    /// WASD + map keys.
    pub fn on_key(&mut self, key: KeyEvent) -> AppAction {
        if self.quit {
            return AppAction::None;
        }
        // The script pane's text inputs capture keys before the global
        // shortcuts (typing a load path must not quit on `q`).
        if self.script_load_open {
            return self.script_load_on_key(key);
        }
        if self.rs2b0t_catalog_open {
            return self.catalog_on_key(key);
        }
        if self.script_browse_open {
            return self.script_browse_on_key(key);
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
            KeyCode::Char('m') => return AppAction::SpawnAll,
            KeyCode::Char('p') => {
                // Paint-as-chat toggle: `p` shows the game chat while the
                // focused script paints (a second press brings the paint
                // back). No-op when nothing is painted.
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
            let mut pane =
                SettingsPane::new(&mut self.settings, &mut self.nav, &mut self.settings_state);
            if pane.on_key(key) == SettingsKey::Changed {
                self.settings_dirty = true;
            }
            return AppAction::None;
        }
        if self.chat_data.is_modal_open() {
            return self.chat_on_key(key).unwrap_or(AppAction::None);
        }
        if let Some(here) = self.here {
            if let Some((x, z, level)) = wasd_target((here.x, here.z, here.level), key.code) {
                return AppAction::WalkTile(Tile { x, z, level });
            }
        }
        self.map_on_key(key)
    }

    /// Route keys to the params popup when it is open; returns whether the
    /// key was consumed.
    pub fn params_on_key(
        &mut self,
        store: &mut script::ScriptSettingsStore,
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
        self.params_state.open = true;
        self.params_state.cursor = 0;
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
    /// deletes, Enter submits [`AppAction::ScriptLoad`], Esc cancels.
    fn script_load_on_key(&mut self, key: KeyEvent) -> AppAction {
        match key.code {
            KeyCode::Esc => {
                self.script_load_open = false;
                self.script_load_path.clear();
            }
            KeyCode::Enter => {
                let path = std::mem::take(&mut self.script_load_path);
                self.script_load_open = false;
                let trimmed = path.trim().to_string();
                if !trimmed.is_empty() {
                    return AppAction::ScriptLoad(std::path::PathBuf::from(trimmed));
                }
            }
            KeyCode::Backspace => {
                self.script_load_path.pop();
            }
            KeyCode::Char(c) => self.script_load_path.push(c),
            _ => {}
        }
        AppAction::None
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
        let lines = browse_lines(
            &self.script_cards,
            &self.script_category_order,
            deferred,
        );
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
                ScriptSel::Loaded(source, name) => self.script_cards.iter().position(|c| {
                    c.source == *source && c.name == *name
                }),
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
        if self.params_state.open {
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
                action @ (ChatAction::Continue | ChatAction::Answer(_)) => {
                    return AppAction::Chat(action)
                }
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
            &self.script_load_path,
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
                self.script_load_open = true;
                AppAction::None
            }
            ScriptClick::Params => AppAction::ScriptParams,
            ScriptClick::Button(_) => AppAction::None,
            ScriptClick::ImportCatalog => AppAction::ScriptImportCatalog,
            ScriptClick::Pick(idx) => {
                if let Some(card) = self.script_cards.get(idx) {
                    self.script_sel = Some(ScriptSel::Loaded(
                        card.source,
                        card.name.clone(),
                    ));
                }
                AppAction::None
            }
            ScriptClick::None => AppAction::None,
        }
    }

    fn catalog_click(&mut self, col: u16, row: u16) -> AppAction {
        let inner = Block::default().borders(Borders::ALL).inner(self.script_area);
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
        let script_h = 4
            + browse_h
            + catalog_h
            + u16::from(self.script_load_open)
            + u16::from(!self.params_schema.is_empty() && !self.script_browse_open && !self.rs2b0t_catalog_open && !self.script_load_open);
        let chunks = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(8),
            Constraint::Length(6),
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
            let pane =
                SettingsPane::new(&mut self.settings, &mut self.nav, &mut self.settings_state);
            frame.render_widget(pane, area);
        }
    }

    /// Render the params popup overlay (call after [`Self::draw`]).
    pub fn draw_params_overlay(
        &mut self,
        frame: &mut Frame<'_>,
        store: &mut script::ScriptSettingsStore,
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
            source,
            name: &name,
            state: &mut self.params_state,
        };
        frame.render_widget(&pane, frame.area());
    }

    fn draw_strip(&mut self, frame: &mut Frame<'_>, area: Rect) {
        let focused = self.focused_name().unwrap_or_else(|| "_".into());
        let mut text = format!(
            "[{}]  focused: {focused}   {}   q quit · o options · Tab focus",
            self.names.join(" "),
            self.title
        );
        if let Some(err) = &self.error {
            text.push_str(&format!("   !! {err}"));
        }
        let p = Paragraph::new(text).wrap(Wrap { trim: false });
        frame.render_widget(p, area);
    }

    fn draw_map(&mut self, frame: &mut Frame<'_>, area: Rect) {
        match self.world.clone() {
            Some(world) => {
                let mut map = Map::new(&world, &mut self.map, |_| {});
                if let Some(here) = self.here {
                    map = map.here(here);
                }
                if let Some(route) = &self.route {
                    map = map.route(route);
                }
                frame.render_widget(map, area);
            }
            None => {
                let block = Block::default()
                    .borders(Borders::ALL)
                    .title("map (no nav pack)");
                frame.render_widget(block, area);
            }
        }
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
        let pane = StatusPane::new(self.focused_status(), &walk, mem);
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
            &self.script_load_path,
            !self.params_schema.is_empty(),
            None,
        );
        frame.render_widget(pane, area);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    use api::snapshot::{ChatLineView, ChatOptionView, WorldTile};
    use nav::tile::Tile;
    use script::{RunState, ScriptKind, ScriptSel, ScriptSource};
    use vault::ProfileSettings;

    use crate::script_shape::BrowseCard;

    use super::{wasd_target, AppAction, TuiApp};

    fn bone_burier_card() -> BrowseCard {
        BrowseCard {
            name: "BoneBurier".into(),
            description: String::new(),
            category: "Prayer".into(),
            tags: Vec::new(),
            kind: ScriptKind::Compat,
            source: ScriptSource::Catalog,
        }
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn tile(x: i32, z: i32) -> Tile {
        Tile { x, z, level: 0 }
    }

    fn line(text: &str) -> ChatLineView {
        ChatLineView {
            type_: 0,
            username: Some("npc".into()),
            text: text.into(),
            sequence: 0,
        }
    }

    /// The window title line survives the full chrome draw.
    #[test]
    fn draws_title_containing_274bot() {
        let mut app = TuiApp::new("274bot headless");
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();

        let buf = terminal.backend().buffer();
        let text: String = buf.content().iter().map(|cell| cell.symbol()).collect();
        assert!(
            text.contains("274bot"),
            "buffer does not contain 274bot: {text:?}"
        );
        assert!(
            text.contains("no nav pack"),
            "empty world must title the map pane as missing the pack: {text:?}"
        );
    }

    /// A loaded world drops the empty-state title; the map paints the
    /// walkable field instead of a hollow "no nav pack" block.
    #[test]
    fn draw_map_paints_walkable_dots_when_the_pack_is_loaded() {
        let mut app = TuiApp::new("274bot headless");
        app.world = Some(Arc::new(nav::world::NavWorld::from_grid(
            &nav::grid::StepGrid::fixture_open_3x3(),
        )));
        app.here = Some(api::snapshot::WorldTile {
            x: 1,
            z: 1,
            level: 0,
        });
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let buf = terminal.backend().buffer();
        let text: String = buf.content().iter().map(|cell| cell.symbol()).collect();
        assert!(
            !text.contains("no nav pack"),
            "loaded pack must not keep the empty-state title: {text:?}"
        );
        assert!(text.contains('.'), "walkable tiles paint as dots: {text:?}");
        assert!(
            text.contains('@'),
            "the here marker paints on the player tile: {text:?}"
        );
    }

    /// The spec's WASD test: from (10,10) W steps north. +z is north on
    /// the client's axis (the map's north-up camera — see `map.rs`'s pan
    /// tests), so W is +z: (10,10) → (10,11).
    #[test]
    fn wasd_w_from_10_10_walks_north_to_10_11() {
        let here = (10, 10, 0);
        assert_eq!(
            wasd_target(here, KeyCode::Char('w')),
            Some((10, 11, 0)),
            "W is north = +z on the client axis"
        );
        assert_eq!(
            wasd_target(here, KeyCode::Char('s')),
            Some((10, 9, 0)),
            "S is south = -z"
        );
        assert_eq!(
            wasd_target(here, KeyCode::Char('a')),
            Some((9, 10, 0)),
            "A is west = -x"
        );
        assert_eq!(
            wasd_target(here, KeyCode::Char('d')),
            Some((11, 10, 0)),
            "D is east = +x"
        );
        assert_eq!(wasd_target(here, KeyCode::F(1)), None);
    }

    #[test]
    fn wasd_on_the_app_returns_a_walk_tile_action() {
        let mut app = TuiApp::new("274bot headless");
        app.names = vec!["test".into()];
        app.focused = Some(0);
        app.statuses = vec![host_play::SlotStatus {
            username: "test".into(),
            ingame: true,
            scene_state: 2,
            tile_x: 10,
            tile_z: 10,
            ..host_play::SlotStatus::default()
        }];
        app.refresh();
        assert_eq!(
            app.on_key(key(KeyCode::Char('w'))),
            AppAction::WalkTile(tile(10, 11)),
            "W on the app queues a one-tile north walk"
        );
    }

    /// TASK-014 parity: upstairs origin must arm WASD on the player plane.
    #[test]
    fn player_at_plane_one_refresh_arms_wasd_with_level() {
        let mut app = TuiApp::new("274bot headless");
        app.names = vec!["test".into()];
        app.focused = Some(0);
        app.statuses = vec![host_play::SlotStatus {
            username: "test".into(),
            ingame: true,
            scene_state: 2,
            tile_x: 10,
            tile_z: 10,
            tile_level: 1,
            ..host_play::SlotStatus::default()
        }];
        app.refresh();
        assert_eq!(
            app.here,
            Some(WorldTile {
                x: 10,
                z: 10,
                level: 1,
            }),
            "refresh must publish tile_level, not hardcoded ground"
        );
        assert_eq!(
            app.on_key(key(KeyCode::Char('w'))),
            AppAction::WalkTile(Tile {
                x: 10,
                z: 11,
                level: 1,
            }),
            "W must keep the player plane when arming a one-tile walk"
        );
    }

    #[test]
    fn lowercase_s_walks_south_when_settings_closed() {
        let mut app = TuiApp::new("274bot headless");
        app.names = vec!["test".into()];
        app.focused = Some(0);
        app.statuses = vec![host_play::SlotStatus {
            username: "test".into(),
            ingame: true,
            scene_state: 2,
            tile_x: 10,
            tile_z: 10,
            ..host_play::SlotStatus::default()
        }];
        app.refresh();
        assert!(
            !app.settings_state.open,
            "settings must start closed so s is free for WASD"
        );
        assert_eq!(
            app.on_key(key(KeyCode::Char('s'))),
            AppAction::WalkTile(tile(10, 9)),
            "lowercase s walks south when settings are closed"
        );
    }

    #[test]
    fn q_quits_and_o_toggles_settings() {
        let mut app = TuiApp::new("274bot headless");
        assert_eq!(app.on_key(key(KeyCode::Char('o'))), AppAction::None);
        assert!(app.settings_state.open, "o opens the settings popup");
        assert_eq!(app.on_key(key(KeyCode::Char('q'))), AppAction::Quit);
        assert!(app.quit);
    }

    #[test]
    fn m_spawns_the_rest_of_the_multibox_wall() {
        let mut app = TuiApp::new("274bot headless");
        assert_eq!(
            app.on_key(key(KeyCode::Char('m'))),
            AppAction::SpawnAll,
            "m spawns every parked profile"
        );
    }

    /// A chat modal on the focused snapshot routes Space/Enter to the
    /// chat pane instead of the map.
    #[test]
    fn chat_modal_open_routes_enter_to_continue() {
        let mut app = TuiApp::new("274bot headless");
        app.chat_data.modal_texts = vec!["The stranger waits.".into()];
        app.chat_data.has_continue = true;
        assert_eq!(
            app.on_key(key(KeyCode::Enter)),
            AppAction::Chat(super::ChatAction::Continue),
            "Enter while a chat modal is up continues the dialog"
        );
    }

    #[test]
    fn chat_modal_options_answer_on_space() {
        let mut app = TuiApp::new("274bot headless");
        app.chat_data.modal_texts = vec!["Which way?".into()];
        app.chat_data.options = vec![ChatOptionView {
            component_id: 1,
            text: "Yes".into(),
        }];
        app.chat_data.has_continue = true;
        assert_eq!(
            app.on_key(key(KeyCode::Char(' '))),
            AppAction::Chat(super::ChatAction::Answer(1)),
            "Space answers the focused option"
        );
    }

    #[test]
    fn settings_enter_flips_random_events_and_marks_dirty() {
        let mut app = TuiApp::new("274bot headless");
        app.settings = ProfileSettings::default();
        assert!(app.settings.random_events);
        app.settings_state.open = true;
        app.on_key(key(KeyCode::Enter));
        assert!(!app.settings.random_events, "popup flips random_events");
        assert!(app.settings_dirty, "the binary persists the change");
    }

    #[test]
    fn map_enter_confirms_a_walk_selection() {
        let mut app = TuiApp::new("274bot headless");
        app.names = vec!["test".into()];
        app.focused = Some(0);
        app.world = Some(Arc::new(nav::world::NavWorld::from_grid(
            &nav::grid::StepGrid::fixture_open_3x3(),
        )));
        app.here = Some(api::snapshot::WorldTile {
            x: 1,
            z: 1,
            level: 0,
        });
        app.map.selection = Some(tile(2, 2));
        assert_eq!(
            app.on_key(key(KeyCode::Enter)),
            AppAction::ArmWalk(tile(2, 2)),
            "Enter on a selection arms the walk"
        );
    }

    /// The review's focus test: Tab must produce an action that carries
    /// the newly focused name, so the binary can mirror it onto
    /// `Play::focus` (the app's index alone leaves the session on the
    /// boot slot's sample gate).
    #[test]
    fn tab_produces_a_focus_action_for_the_next_running_slot() {
        let mut app = TuiApp::new("274bot headless");
        app.names = vec!["a".into(), "b".into()];
        app.focused = Some(0);
        app.statuses = vec![
            host_play::SlotStatus {
                username: "a".into(),
                ..host_play::SlotStatus::default()
            },
            host_play::SlotStatus {
                username: "b".into(),
                ..host_play::SlotStatus::default()
            },
        ];
        assert_eq!(
            app.on_key(key(KeyCode::Tab)),
            AppAction::Focus("b".into()),
            "Tab names the newly focused slot so Play::focus follows"
        );
        assert_eq!(app.focused, Some(1));
        assert_eq!(
            app.on_key(key(KeyCode::Tab)),
            AppAction::Focus("a".into()),
            "focus wraps around"
        );
        assert_eq!(app.focused, Some(0));
    }

    #[test]
    fn tab_with_no_running_slots_does_nothing() {
        let mut app = TuiApp::new("274bot headless");
        assert_eq!(app.on_key(key(KeyCode::Tab)), AppAction::None);
    }

    #[test]
    fn strip_click_selects_the_clicked_slot_name() {
        let mut app = TuiApp::new("274bot headless");
        app.names = vec!["a".into(), "b".into()];
        // Strip text: `[a b]  focused: …`. Name spans: `a` at col 1,
        // `b` at col 3.
        assert_eq!(app.on_click(1, 0), AppAction::Focus("a".into()));
        assert_eq!(
            app.focused,
            Some(0),
            "the strip click updates the app focus, not only Play"
        );
        assert_eq!(app.on_click(3, 0), AppAction::Focus("b".into()));
        assert_eq!(
            app.focused,
            Some(1),
            "clicking slot B focuses B in the app too, so UI + input agree"
        );
        // Between the names is a miss.
        assert_eq!(app.on_click(2, 0), AppAction::None);
        assert_eq!(app.focused, Some(1), "a miss keeps the current focus");
    }

    #[test]
    fn full_draw_paints_all_panes() {
        let mut app = TuiApp::new("274bot headless");
        app.names = vec!["test".into()];
        app.focused = Some(0);
        app.statuses = vec![host_play::SlotStatus {
            username: "test".into(),
            ingame: true,
            scene_state: 2,
            tile_x: 10,
            tile_z: 10,
            ..host_play::SlotStatus::default()
        }];
        app.here = Some(api::snapshot::WorldTile {
            x: 10,
            z: 10,
            level: 0,
        });
        app.script_state = RunState::Idle;
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let buf = terminal.backend().buffer();
        let text: String = buf.content().iter().map(|cell| cell.symbol()).collect();
        assert!(text.contains("focused: test"), "strip: {text:?}");
        assert!(text.contains("ingame scene 2"), "status: {text:?}");
        assert!(text.contains("[Start]"), "script shape: {text:?}");
        assert!(text.contains("script: idle"), "script state: {text:?}");
    }

    #[test]
    fn chat_pane_click_routes_to_answer() {
        let mut app = TuiApp::new("274bot headless");
        app.chat_data.modal_texts = vec!["Which way?".into()];
        app.chat_data.options = vec![
            ChatOptionView {
                component_id: 1,
                text: "Yes".into(),
            },
            ChatOptionView {
                component_id: 2,
                text: "No thanks".into(),
            },
        ];
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let chat_area = app.chat_area;
        assert!(chat_area.height >= 4, "chat pane has room for options");
        // Option rows start after border + text + blank (see chat.rs):
        // border row 0, text row 1, blank row 2, options from row 3.
        let row = chat_area.y + 3;
        assert_eq!(
            app.on_click(chat_area.x, row),
            AppAction::Chat(super::ChatAction::Answer(1)),
            "clicking the first option row answers option 1"
        );
    }

    #[test]
    fn chat_data_builds_from_snapshot_views() {
        let mut app = TuiApp::new("274bot headless");
        app.chat_data.lines = vec![line("welcome to 274")];
        assert!(!app.chat_data.is_modal_open());
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let buf = terminal.backend().buffer();
        let text: String = buf.content().iter().map(|cell| cell.symbol()).collect();
        assert!(
            text.contains("welcome to 274"),
            "chat ring paints: {text:?}"
        );
    }

    /// TR-TUI-001: when the focused script is Paused the pane shows
    /// `[Resume]` and clicking it dispatches the pause/resume toggle.
    #[test]
    fn paused_script_shows_resume_and_click_dispatches_toggle() {
        let mut app = TuiApp::new("274bot headless");
        app.script_state = RunState::Paused;
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let buf = terminal.backend().buffer();
        let text: String = buf.content().iter().map(|cell| cell.symbol()).collect();
        assert!(
            text.contains("[Resume]"),
            "paused script paints Resume: {text:?}"
        );
        assert!(
            !text.contains("[Pause]"),
            "paused script must not paint Pause: {text:?}"
        );
        let area = app.script_area;
        // `[Browse] ` + `[Start] ` → `[Resume] ` at inner.x + 17 = area.x + 18.
        assert_eq!(
            app.on_click(area.x + 18, area.y + 2),
            AppAction::ScriptPause,
            "Resume click dispatches the pause/resume toggle"
        );
    }

    /// Task 13: with a Browse-selected JS card, clicking Start returns
    /// `AppAction::ScriptStart` carrying the card name (tui-play starts the
    /// load isolate on the focused slot).
    #[test]
    fn click_start_with_a_selected_card_returns_script_start() {
        let mut app = TuiApp::new("274bot headless");
        app.script_sel = Some(ScriptSel::Loaded(
            ScriptSource::Catalog,
            "BoneBurier".into(),
        ));
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let area = app.script_area;
        assert_eq!(
            app.on_click(area.x + 10, area.y + 2),
            AppAction::ScriptStart(ScriptSel::Loaded(
                ScriptSource::Catalog,
                "BoneBurier".into(),
            )),
            "Start with a selected card starts that card"
        );
    }

    #[test]
    fn browse_rows_select_a_card_for_start() {
        let mut app = TuiApp::new("274bot headless");
        app.script_cards = vec![
            bone_burier_card(),
            BrowseCard {
                name: "MineRobber".into(),
                description: String::new(),
                category: "Skilling".into(),
                tags: Vec::new(),
                kind: ScriptKind::Compat,
                source: ScriptSource::File,
            },
        ];
        app.script_category_order = vec!["Prayer".into(), "Skilling".into()];
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let area = app.script_area;
        // The buttons row is the second inner line; `[Browse]` is first.
        assert_eq!(
            app.on_click(area.x + 1, area.y + 2),
            AppAction::ScriptBrowse,
            "Browse opens the picker"
        );
        assert!(app.script_browse_open);
        // Re-draw: the picker grows the pane and the card rows start at
        // the third inner line (area.y + 3).
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let area = app.script_area;
        assert_eq!(app.on_click(area.x + 2, area.y + 4), AppAction::None);
        assert_eq!(
            app.script_sel,
            Some(ScriptSel::Loaded(
                ScriptSource::Catalog,
                "BoneBurier".into(),
            )),
            "clicking the first card row selects it"
        );
        assert_eq!(
            app.on_click(area.x + 10, area.y + 2),
            AppAction::ScriptStart(ScriptSel::Loaded(
                ScriptSource::Catalog,
                "BoneBurier".into(),
            )),
            "Start starts the card picked in Browse"
        );
    }

    /// Task 13: the Load button opens the path input; typed path + Enter
    /// produces `AppAction::ScriptLoad` with that path.
    #[test]
    fn load_path_typed_and_enter_returns_script_load() {
        let mut app = TuiApp::new("274bot headless");
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let area = app.script_area;
        // `[Load]` starts after Browse(9) + Start(8) + Pause(8) + Stop(7):
        // inner.x + 32 = area.x + 33.
        assert_eq!(
            app.on_click(area.x + 33, area.y + 2),
            AppAction::None,
            "Load opens the path input (no action until Enter)"
        );
        assert!(app.script_load_open);
        for c in "/tmp/digbot.js".chars() {
            app.on_key(key(KeyCode::Char(c)));
        }
        assert_eq!(
            app.on_key(key(KeyCode::Enter)),
            AppAction::ScriptLoad(std::path::PathBuf::from("/tmp/digbot.js")),
            "Enter on the typed path loads that file"
        );
        assert!(!app.script_load_open, "load input closes after Enter");
        // Esc cancels the input without an action.
        app.on_click(area.x + 33, area.y + 2);
        app.on_key(key(KeyCode::Char('x')));
        assert_eq!(app.on_key(key(KeyCode::Esc)), AppAction::None);
        assert!(!app.script_load_open);
        assert!(app.script_load_path.is_empty(), "Esc clears the path");
    }

    /// Task 5 fix: clicking `[Params]` opens the popup; Space toggles a
    /// bool into the bag Start would post.
    #[test]
    fn script_params_click_and_space_toggle_persist_bool() {
        let dir = std::env::temp_dir().join(format!(
            "274bot-tui-app-params-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("script-settings.json");
        let mut store = script::ScriptSettingsStore::at(path);
        let schema = vec![script::SettingDef {
            id: "buryBones".into(),
            ty: "boolean".into(),
            default: Some("true".into()),
            label: Some("Bury bones".into()),
            min: None,
            max: None,
            step: None,
            options: Vec::new(),
            option_labels: Vec::new(),
            group: None,
            show_if: None,
            options_from: None,
            csv_toggle: None,
            help: None,
        }];
        let mut app = TuiApp::new("274bot headless");
        app.script_sel = Some(ScriptSel::Loaded(
            ScriptSource::Catalog,
            "ChickenKiller".into(),
        ));
        app.params_schema = schema;
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let area = app.script_area;
        assert_eq!(
            app.on_click(area.x + 1, area.y + 3),
            AppAction::ScriptParams,
            "[Params] opens the popup"
        );
        app.open_script_params(&store);
        assert!(app.params_state.open);
        app.params_on_key(&mut store, key(KeyCode::Char(' ')));
        assert_eq!(
            app.params_bag.get("buryBones"),
            Some(&serde_json::json!(false))
        );
        let start_bag = app.merged_script_settings_bag(&store).expect("merged bag");
        assert_eq!(
            start_bag.get("buryBones"),
            Some(&serde_json::json!(false)),
            "Start would post the toggled bool"
        );
        terminal
            .draw(|frame| {
                app.draw(frame);
                app.draw_params_overlay(frame, &mut store);
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        let text: String = buf.content().iter().map(|cell| cell.symbol()).collect();
        assert!(
            text.contains("parameters"),
            "params overlay paints: {text:?}"
        );
    }

    /// Task 13: while the focused slot's script paints, the chat pane
    /// shows the paint title and rows instead of the game chat; the `p`
    /// key toggles back to the game chat.
    #[test]
    fn chat_pane_shows_script_paint_instead_of_the_game_chat() {
        let mut app = TuiApp::new("274bot headless");
        app.chat_data.lines = vec![line("last game chat line")];
        app.chat_data.script_paint = Some(script::shim::ScriptPaint {
            title: Some("BoneBurier — digging".into()),
            accent: Some("#f3e6a2".into()),
            lines: vec!["Runtime: 1.2m | Buried: 3".into(), "".into()],
        });
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let buf = terminal.backend().buffer();
        let text: String = buf.content().iter().map(|cell| cell.symbol()).collect();
        assert!(
            text.contains("BoneBurier — digging"),
            "the paint title paints: {text:?}"
        );
        assert!(
            text.contains("Runtime: 1.2m | Buried: 3"),
            "paint rows paint: {text:?}"
        );
        assert!(
            !text.contains("last game chat line"),
            "the game chat is replaced by the paint: {text:?}"
        );
        // The toggle key brings the game chat back.
        assert_eq!(app.on_key(key(KeyCode::Char('p'))), AppAction::None);
        assert!(app.chat_data.show_game_chat);
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let buf = terminal.backend().buffer();
        let text: String = buf.content().iter().map(|cell| cell.symbol()).collect();
        assert!(
            text.contains("last game chat line"),
            "p toggles back to the game chat: {text:?}"
        );
        assert!(
            !text.contains("BoneBurier — digging"),
            "the paint is hidden while toggled off: {text:?}"
        );
    }
}
