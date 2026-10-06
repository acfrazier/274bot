//! Input routing: one model for keys and the mouse. Keys go, in order, to
//! Ctrl-Q (always a quit request); the open popup or overlay (params,
//! loadouts, settings, then help/palette/confirm/menu/message/manual walk,
//! then the Script tab's Load, catalog and Browse popups; it takes every
//! key); the focused pane's text field (fleet filter, map search, log
//! search: typing wins); the global chords (F1-F7, Ctrl-P, Tab/Shift-Tab
//! and, outside text, `?`, `:`, `q`); then the focused pane's own keys. No
//! letter is global: each pane's shortcuts live in `commands` and act only
//! while that pane has keyboard focus. The mouse hit-tests the regions of
//! the last draw: an overlay or Script popup first (it swallows clicks and
//! the wheel outside itself), then the header tabs, then the pane under the
//! pointer, which also takes keyboard focus. A right click opens a context
//! menu and never acts as a left click.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

use crate::app::{AppAction, TuiApp};
use crate::commands::{
    script_key_command, Command, Group, CHAT_KEYS, FLEET_KEYS, MAP_KEYS, OVERVIEW_KEYS,
};
use crate::fleet::MARK_COLUMNS;
use crate::help::HelpState;
use crate::layout::{contains, Pane, Screen, SizeClass, Tab};
use crate::overlay::{ConfirmKind, ContextItem, ContextMenu, Modal};
use crate::palette::PaletteState;
use crate::settings::{SettingsKey, SettingsPane};

fn is_text(key: &KeyEvent) -> bool {
    !key.modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER)
}

fn is_ctrl(key: &KeyEvent, c: char) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key.code, KeyCode::Char(k) if k.eq_ignore_ascii_case(&c))
}

fn table_command(table: &[(char, Command)], key: &KeyEvent) -> Option<Command> {
    match key.code {
        KeyCode::Char(c) if is_text(key) => table
            .iter()
            .find(|(bound, _)| *bound == c)
            .map(|(_, command)| *command),
        _ => None,
    }
}

/// The command group a detail tab's context menu and palette favour.
fn screen_group(screen: Screen) -> Group {
    match screen {
        Screen::Overview => Group::Bot,
        Screen::Map => Group::Map,
        Screen::Script => Group::Script,
        Screen::Chat => Group::Chat,
        Screen::Logs => Group::Logs,
    }
}

/// The chords that work from every pane (after text fields had their turn).
fn global_command(key: &KeyEvent) -> Option<Command> {
    if is_ctrl(key, 'p') {
        return Some(Command::Palette);
    }
    let text = is_text(key);
    Some(match key.code {
        KeyCode::F(1) => Command::Help,
        KeyCode::F(2) => Command::ShowFleet,
        KeyCode::F(3) => Command::Show(Screen::Overview),
        KeyCode::F(4) => Command::Show(Screen::Map),
        KeyCode::F(5) => Command::Show(Screen::Script),
        KeyCode::F(6) => Command::Show(Screen::Chat),
        KeyCode::F(7) => Command::Show(Screen::Logs),
        KeyCode::Tab => Command::FocusNext,
        KeyCode::BackTab => Command::FocusPrev,
        KeyCode::Char('?') if text => Command::Help,
        KeyCode::Char(':') if text => Command::Palette,
        KeyCode::Char('q') if text => Command::Quit,
        _ => return None,
    })
}

impl TuiApp {
    /// One key press.
    pub fn on_key(&mut self, key: KeyEvent) -> AppAction {
        if self.quit {
            return AppAction::None;
        }
        if is_ctrl(&key, 'q') {
            return self.run_command(Command::Quit);
        }
        if self.params_state.open {
            return AppAction::ParamsKey(key);
        }
        if self.loadouts_state.open {
            return AppAction::LoadoutsKey(key);
        }
        if self.settings_state.open {
            return self.settings_key(key);
        }
        if self.modal.is_some() {
            return self.modal_key(key);
        }
        if let Some(action) = self.script_popup_key(key) {
            return action;
        }
        if let Some(action) = self.text_entry_key(key) {
            return action;
        }
        if let Some(command) = global_command(&key) {
            return self.run_command(command);
        }
        match self.key_focus {
            Pane::Fleet => self.fleet_key(key),
            Pane::Detail => self.detail_key(key),
            Pane::Drawer => self.drawer_key(key),
        }
    }

    /// The pane the keyboard points at, as help sections name it.
    pub fn key_scope_context(&self) -> &'static str {
        if self.modal == Some(Modal::Manual) {
            return "Manual walk";
        }
        match self.key_focus {
            Pane::Fleet => "Fleet",
            Pane::Drawer => "Logs",
            Pane::Detail => self.screen.label(),
        }
    }

    /// The command group of the focused pane (listed first in the palette).
    pub fn key_scope_group(&self) -> Group {
        match self.key_focus {
            Pane::Fleet => Group::Fleet,
            Pane::Drawer => Group::Logs,
            Pane::Detail => screen_group(self.screen),
        }
    }

    /// The keyboard scope the footer always shows: popup, overlay, text
    /// field or pane.
    pub fn key_scope(&self) -> &'static str {
        if self.params_state.open {
            return "Parameters";
        }
        if self.loadouts_state.open {
            return "Loadouts";
        }
        if self.settings_state.open {
            return "Settings";
        }
        if let Some(modal) = &self.modal {
            return modal.scope();
        }
        if self.script_load_open {
            return "Script: load file";
        }
        if self.rs2b0t_catalog_open {
            return "Script: catalog folder";
        }
        if self.script_browse_open {
            return "Script: Browse";
        }
        match self.key_focus {
            Pane::Fleet if self.table.editing => "Fleet filter",
            Pane::Fleet => "Fleet",
            Pane::Drawer if self.log.editing => "Log search",
            Pane::Drawer => "Log drawer",
            Pane::Detail => match self.screen {
                Screen::Map if self.map_search_open => "Map search",
                Screen::Logs if self.log.editing => "Log search",
                screen => screen.label(),
            },
        }
    }

    /// The keys of [`Self::key_scope`], for the footer.
    pub fn key_hints(&self) -> &'static str {
        if self.params_state.open {
            return "Up/Down row · Enter edit · Space toggle · a apply to all · Esc close";
        }
        if self.loadouts_state.open {
            return "Up/Down/Tab row · Enter/Space act · type into fields · Esc close";
        }
        if self.settings_state.open {
            if self.settings_state.folder_editing {
                return "type path · Backspace erase · Enter/Esc finish editing";
            }
            return if self
                .settings_memory
                .is_some_and(frontend_core::MemoryNotice::can_relog)
            {
                "Up/Down row · Enter/Space act · r Relog now · Esc close"
            } else {
                "Up/Down row · Enter/Space act · Esc close"
            };
        }
        if let Some(modal) = &self.modal {
            return modal.hints();
        }
        match self.key_scope() {
            "Fleet filter" => "type: name, wN, state · Up/Down move · Enter keep · Esc clear",
            "Fleet" => "Up/Down move · Enter select · Space select row · / filter · m load all · U logout all",
            "Log search" => "type to search · Enter done · Esc done",
            "Log drawer" | "Logs" => "/ search · v level · s source · b scope · f follow · Up/Down scroll · w save",
            "Map search" => "name or x,z,plane · Up/Down results · Enter jump · Esc close",
            "Script: load file" => "Up/Down · Enter open · Esc close",
            "Script: catalog folder" => "Up/Down · Enter open · Esc not now",
            "Script: Browse" => "Up/Down pick · Enter or Esc closes, the pick stays",
            "Overview" if self.memory.is_some_and(frontend_core::MemoryNotice::can_relog) =>
                "i login · u logout · r Relog now · x remove · o settings · l loadouts · w manual walk",
            "Overview" => "i login · u logout · x remove · o settings · l loadouts · w manual walk",
            "Map" => "arrows pan · +/- zoom · Enter select/walk · / search · g group · t teleport · Esc back",
            "Script" => "b browse · t start · P pause · e stop · f load · v params · R reload · T/E all",
            "Chat" if self.chat_data.is_modal_open() => "Up/Down choose · Space/Enter answer · p paint/chat",
            "Chat" => "p paint/chat · 1-9 paint buttons · Up/Down/Enter paint button",
            _ => "",
        }
    }

    /// Select `name` (the core `select`, mirrored on the next pump). The
    /// fleet cursor and the row selection stay as they are.
    pub fn select_bot(&mut self, name: &str) -> AppAction {
        let Some(index) = self.names.iter().position(|n| n == name) else {
            return AppAction::None;
        };
        self.focused = Some(index);
        AppAction::Focus(name.to_string())
    }

    fn select_step(&mut self, step: isize) -> AppAction {
        if self.names.is_empty() {
            return AppAction::None;
        }
        let len = self.names.len() as isize;
        let next = match self.focused {
            Some(index) => (index as isize + step).rem_euclid(len),
            None => 0,
        } as usize;
        let name = self.names[next].clone();
        self.select_bot(&name)
    }

    /// Show a detail tab and give it keyboard focus. Leaving the Map
    /// releases it (`MapClose`); entering asks for its catalogue
    /// (`MapOpen`). Switching tabs never runs anything else.
    pub fn show_screen(&mut self, screen: Screen) -> AppAction {
        let leaving_map = self.screen == Screen::Map && screen != Screen::Map;
        self.screen = screen;
        self.key_focus = Pane::Detail;
        self.table.editing = false;
        if screen == Screen::Logs {
            self.log.mark_seen();
        }
        if leaving_map && self.map_active {
            return self.map_close();
        }
        if screen == Screen::Map && !self.map_active {
            return self.map_open();
        }
        AppAction::None
    }

    fn drawer_focusable(&self) -> bool {
        self.regions.class != SizeClass::Compact && self.screen != Screen::Logs
    }

    /// Point the keyboard at `pane` (a pane that is not on screen is
    /// skipped: the drawer at 80x24).
    pub fn focus_pane(&mut self, pane: Pane) {
        if pane == Pane::Drawer && !self.drawer_focusable() {
            return;
        }
        if pane != Pane::Fleet {
            self.table.editing = false;
        }
        self.key_focus = pane;
    }

    fn cycle_focus(&mut self, forward: bool) {
        let order: &[Pane] = if self.drawer_focusable() {
            &[Pane::Fleet, Pane::Detail, Pane::Drawer]
        } else {
            &[Pane::Fleet, Pane::Detail]
        };
        let at = order
            .iter()
            .position(|pane| *pane == self.key_focus)
            .unwrap_or(0);
        let next = if forward {
            (at + 1) % order.len()
        } else {
            (at + order.len() - 1) % order.len()
        };
        self.focus_pane(order[next]);
    }

    /// Run `command` if it is available now; otherwise say why on the
    /// message line. Fleet-wide commands first confirm their frozen scope.
    pub fn run_command(&mut self, command: Command) -> AppAction {
        if let Err(reason) = command.availability(self) {
            self.error = Some(format!("{}: {reason}", command.label(self)));
            return AppAction::None;
        }
        if command.is_bulk() {
            let marked = matches!(
                command,
                Command::ScriptStartAll
                    | Command::ScriptStopAll
                    | Command::ScriptAssignMarked
                    | Command::ScriptRestartMarked
                    | Command::LoadLoginAll
                    | Command::LogoutAll
            ) && !self.table.selection.is_empty();
            let members = if marked {
                self.marked_names()
            } else {
                self.names.clone()
            };
            self.confirm(ConfirmKind::Bulk {
                command,
                members,
                marked,
            });
            return AppAction::None;
        }
        match command {
            Command::Help => self.open_modal(Modal::Help(HelpState::default())),
            Command::Palette => self.open_modal(Modal::Palette(PaletteState::default())),
            Command::Quit if self.names.is_empty() => {
                self.quit = true;
                return AppAction::Quit;
            }
            Command::Quit => self.confirm(ConfirmKind::Quit),
            Command::ShowFleet => self.focus_pane(Pane::Fleet),
            Command::Show(screen) => return self.show_screen(screen),
            Command::FocusNext => self.cycle_focus(true),
            Command::FocusPrev => self.cycle_focus(false),
            Command::SelectNextBot => return self.select_step(1),
            Command::SelectPrevBot => return self.select_step(-1),
            Command::ToggleMouse => {
                self.mouse_capture = !self.mouse_capture;
                return AppAction::MouseCapture(self.mouse_capture);
            }
            Command::ShowMessage => self.open_modal(Modal::Message { scroll: 0 }),
            Command::FilterFleet => {
                self.focus_pane(Pane::Fleet);
                self.table.editing = true;
            }
            Command::MarkAllShown => {
                self.table
                    .sync_with_ids(&self.names, &self.profile_ids, &self.fleet);
                self.table
                    .mark_all_shown_with_ids(&self.names, &self.profile_ids);
            }
            Command::ClearMarks => self.table.selection.clear(),
            Command::Login => return AppAction::Login,
            Command::Logout => return AppAction::Logout,
            Command::Remove => {
                if let Some(name) = self.focused_name() {
                    self.confirm(ConfirmKind::Remove(name));
                }
            }
            Command::Settings => {
                self.modal = None;
                self.loadouts_state.open = false;
                self.params_state.open = false;
                if !self.settings_state.open {
                    // A fresh open: the next pump binds the focused profile
                    // and loads its row. Opening again while bound keeps the
                    // draft (the heading names its profile).
                    self.settings_state.open = true;
                    self.settings_profile = None;
                    self.settings_title.clear();
                    self.settings_title.push_str(crate::settings::TITLE);
                    self.settings_save.form_changed();
                }
            }
            Command::Loadouts => {
                self.modal = None;
                self.settings_state.open = false;
                self.params_state.open = false;
                self.loadouts_state.open = true;
                self.loadouts_state.sel = 0;
                self.loadouts_state.name_scratch.clear();
                self.loadouts_state.worn_scratch.clear();
                self.loadouts_state.carry_scratch.clear();
            }
            Command::ManualWalk => self.open_modal(Modal::Manual),
            Command::DismissNotice => return AppAction::AckBackground,
            Command::ScriptBrowse | Command::ScriptLoad | Command::ImportCatalog => {
                // These open inside the Script tab: show it first.
                let show = if self.screen == Screen::Script {
                    AppAction::None
                } else {
                    self.show_screen(Screen::Script)
                };
                let run = self.script_run(command);
                return AppAction::batch(show, run);
            }
            Command::ScriptStart
            | Command::ScriptPause
            | Command::ScriptStop
            | Command::ScriptParams
            | Command::ScriptReload
            | Command::ScriptReloadCancel => return self.script_run(command),
            Command::ToggleGameChat => {
                self.chat_data.show_game_chat = !self.chat_data.show_game_chat;
            }
            Command::MapSearch => self.open_map_search(),
            Command::MapRecenter => self.recenter_map(),
            Command::MapGroup => self.toggle_walk_send_mode(),
            Command::MapWilderness => self.toggle_map_wilderness(),
            Command::MapWalk => return self.map_enter(),
            Command::MapTeleport => {
                if let Some(pending) = self.map_model.pending() {
                    return AppAction::MapTeleport(pending.requested);
                }
            }
            Command::LogSave => {
                let focused = self.focused_name();
                self.log.save(focused.as_deref());
            }
            Command::LogSessionFile => self.log.toggle_session_file(),
            Command::ScriptApplyMarked => return AppAction::ScriptApplyMarkedPrepare,
            Command::LoadLoginAll
            | Command::LogoutAll
            | Command::ScriptStartAll
            | Command::ScriptStopAll
            | Command::ScriptAssignMarked
            | Command::ScriptRestartMarked => {}
        }
        AppAction::None
    }

    /// Typing into the focused pane's text field. `None` lets the key fall
    /// through (function keys, Tab, Ctrl chords).
    fn text_entry_key(&mut self, key: KeyEvent) -> Option<AppAction> {
        match self.key_focus {
            Pane::Fleet if self.table.editing => self.fleet_filter_key(key),
            Pane::Detail if self.screen == Screen::Map && self.map_search_open => {
                let search_key = matches!(
                    key.code,
                    KeyCode::Esc
                        | KeyCode::Backspace
                        | KeyCode::Up
                        | KeyCode::Down
                        | KeyCode::Enter
                ) || (matches!(key.code, KeyCode::Char(_)) && is_text(&key))
                    || is_ctrl(&key, 'p')
                    || is_ctrl(&key, 'n');
                search_key.then(|| self.map_search_on_key(key))
            }
            Pane::Detail if self.screen == Screen::Logs && self.log.editing => {
                self.log_search_key(key)
            }
            Pane::Drawer if self.log.editing => self.log_search_key(key),
            _ => None,
        }
    }

    fn log_search_key(&mut self, key: KeyEvent) -> Option<AppAction> {
        let focused = self.focused_name();
        self.log
            .on_key(key, focused.as_deref())
            .then_some(AppAction::None)
    }

    fn fleet_filter_key(&mut self, key: KeyEvent) -> Option<AppAction> {
        match key.code {
            KeyCode::Esc => {
                self.table.filter.clear();
                self.table.editing = false;
            }
            KeyCode::Enter => self.table.editing = false,
            KeyCode::Backspace => {
                self.table.filter.pop();
            }
            KeyCode::Up => self.table.move_cursor(-1, &self.names),
            KeyCode::Down => self.table.move_cursor(1, &self.names),
            KeyCode::Char(c) if is_text(&key) => {
                self.table.filter.push(c);
                self.table.cursor = 0;
            }
            _ => return None,
        }
        self.table
            .sync_with_ids(&self.names, &self.profile_ids, &self.fleet);
        Some(AppAction::None)
    }

    fn fleet_key(&mut self, key: KeyEvent) -> AppAction {
        self.table
            .sync_with_ids(&self.names, &self.profile_ids, &self.fleet);
        let page = self.regions.fleet_rows.height.max(1) as isize;
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.table.move_cursor(-1, &self.names),
            KeyCode::Down | KeyCode::Char('j') => self.table.move_cursor(1, &self.names),
            KeyCode::PageUp => self.table.move_cursor(-page, &self.names),
            KeyCode::PageDown => self.table.move_cursor(page, &self.names),
            KeyCode::Home => self.table.cursor_to(0, &self.names),
            KeyCode::End => self.table.cursor_to(usize::MAX, &self.names),
            KeyCode::Enter => {
                let Some(member) = self.table.cursor_member() else {
                    return AppAction::None;
                };
                let name = self.names[member].clone();
                let action = self.select_bot(&name);
                if self.regions.class == SizeClass::Compact {
                    // The fleet drawer closes on a selection.
                    self.key_focus = Pane::Detail;
                }
                return action;
            }
            KeyCode::Char(' ') => {
                if let Some(member) = self.table.cursor_member() {
                    self.table
                        .toggle_mark_with_ids(&self.names, &self.profile_ids, member);
                }
            }
            KeyCode::Char('/') => self.table.editing = true,
            KeyCode::Esc => {
                if !self.table.filter.is_empty() {
                    self.table.filter.clear();
                    self.table
                        .sync_with_ids(&self.names, &self.profile_ids, &self.fleet);
                } else if self.regions.class == SizeClass::Compact {
                    self.key_focus = Pane::Detail;
                }
            }
            _ => {
                if let Some(command) = table_command(FLEET_KEYS, &key) {
                    return self.run_command(command);
                }
            }
        }
        AppAction::None
    }

    fn detail_key(&mut self, key: KeyEvent) -> AppAction {
        match self.screen {
            Screen::Overview
                if key.code == KeyCode::Char('r')
                    && self
                        .memory
                        .is_some_and(frontend_core::MemoryNotice::can_relog) =>
            {
                self.focused_name()
                    .map_or(AppAction::None, AppAction::MemoryRelog)
            }
            Screen::Overview => table_command(OVERVIEW_KEYS, &key)
                .map_or(AppAction::None, |command| self.run_command(command)),
            Screen::Map => {
                if let Some(command) = table_command(MAP_KEYS, &key) {
                    return self.run_command(command);
                }
                if key.code == KeyCode::Esc && self.map.selection.is_none() {
                    return self.show_screen(Screen::Overview);
                }
                self.map_pane_key(key)
            }
            Screen::Script => self.script_pane_key(key),
            Screen::Chat => match table_command(CHAT_KEYS, &key) {
                Some(command) => self.run_command(command),
                None => self.chat_on_key(key).unwrap_or(AppAction::None),
            },
            Screen::Logs => {
                let focused = self.focused_name();
                if self.log.on_key(key, focused.as_deref()) {
                    AppAction::None
                } else if key.code == KeyCode::Esc {
                    self.show_screen(Screen::Overview)
                } else {
                    AppAction::None
                }
            }
        }
    }

    /// The Script tab's letters (`SCRIPT_KEYS`). Its Browse, Load and
    /// catalog popups never get here: they own the keys while open.
    fn script_pane_key(&mut self, key: KeyEvent) -> AppAction {
        match key.code {
            KeyCode::Char(c) if is_text(&key) => {
                script_key_command(c).map_or(AppAction::None, |command| self.run_command(command))
            }
            _ => AppAction::None,
        }
    }

    fn drawer_key(&mut self, key: KeyEvent) -> AppAction {
        let focused = self.focused_name();
        if !self.log.on_key(key, focused.as_deref()) && key.code == KeyCode::Esc {
            self.key_focus = Pane::Detail;
        }
        AppAction::None
    }

    fn queue_nav_preference(&mut self, preference: frontend_core::NavPreference) {
        if !self.nav_preferences_dirty.contains(&preference) {
            self.nav_preferences_dirty.push(preference);
        }
    }

    fn settings_key(&mut self, key: KeyEvent) -> AppAction {
        let mut pane = SettingsPane::new(
            &mut self.settings,
            &mut self.nav,
            &mut self.map_bake,
            &mut self.settings_state,
        )
        .pause_script_on_manual_walk_abort(&mut self.pause_script_on_manual_walk_abort)
        .script_scope_notice_ack(&mut self.script_scope_notice_ack)
        .quester_paths(&mut self.quester_paths);
        pane.memory = self.settings_memory;
        let outcome = pane.on_key(key);
        match outcome {
            SettingsKey::Changed => {
                self.settings_dirty = true;
                // The draft moved on: a notice about its last save no
                // longer describes it.
                self.settings_save.edited();
            }
            SettingsKey::WalkGlobal(preference) => {
                let pending = frontend_core::WalkGlobalsView {
                    globals: self.nav,
                    script_scope_notice_ack: self.script_scope_notice_ack,
                };
                if preference == frontend_core::NavPreference::AllowDangerZones
                    && !pending.danger_this_walk(true)
                {
                    self.map_route_through_zones = false;
                }
                self.queue_nav_preference(preference);
                if self.shared_preferences_path().is_none() {
                    self.refresh_walk_permissions();
                }
            }
            SettingsKey::ScriptScopeNoticeAck => {
                self.script_scope_notice_ack = true;
                self.queue_nav_preference(frontend_core::NavPreference::ScriptScopeNoticeAck);
                if self.shared_preferences_path().is_none() {
                    self.refresh_walk_permissions();
                }
            }
            SettingsKey::MemoryRelog => {
                if let Some(name) = self.settings_profile.clone() {
                    return AppAction::MemoryRelog(name);
                }
            }
            SettingsKey::MapBake => self.map_bake_dirty = true,
            SettingsKey::PauseScriptOnManualWalkAbort => {
                self.pause_script_on_manual_walk_abort_dirty = true;
            }
            SettingsKey::QuesterPathsChanged => {
                self.quester_paths_dirty = true;
                self.quester_paths_controller.clear_notice();
            }
            SettingsKey::ReloadPaths => return AppAction::ReloadPaths,
            SettingsKey::Consumed | SettingsKey::Ignored => {}
        }
        AppAction::None
    }

    /// One mouse event from the terminal.
    pub fn on_mouse(&mut self, event: MouseEvent) -> AppAction {
        match event.kind {
            MouseEventKind::Down(MouseButton::Left) => self.on_click(event.column, event.row),
            MouseEventKind::Down(MouseButton::Right) => {
                self.on_right_click(event.column, event.row)
            }
            MouseEventKind::ScrollUp => self.on_scroll(event.column, event.row, -1),
            MouseEventKind::ScrollDown => self.on_scroll(event.column, event.row, 1),
            _ => AppAction::None,
        }
    }

    /// A left click at a terminal cell.
    pub fn on_click(&mut self, col: u16, row: u16) -> AppAction {
        if self.quit || self.params_state.open || self.loadouts_state.open {
            return AppAction::None;
        }
        if self.settings_state.open {
            return self.settings_click(col, row);
        }
        if self.modal.is_some() {
            return self.modal_click(col, row);
        }
        if self.script_popup_open() {
            return self.script_popup_click(col, row);
        }
        if let Some(tab) = self.regions.tab_at(col, row) {
            return match tab {
                Tab::Fleet => self.run_command(Command::ShowFleet),
                Tab::Screen(screen) => self.run_command(Command::Show(screen)),
            };
        }
        let rects = self.regions.rects;
        if contains(rects.fleet, col, row) {
            self.focus_pane(Pane::Fleet);
        } else if contains(rects.detail, col, row) || contains(rects.side, col, row) {
            self.focus_pane(Pane::Detail);
        } else if contains(rects.drawer, col, row) && self.drawer_focusable() {
            self.focus_pane(Pane::Drawer);
        }
        if let Some(command) = self.regions.button_at(col, row) {
            return self.run_command(command);
        }
        if contains(self.regions.message, col, row) && self.error.is_some() {
            return self.run_command(Command::ShowMessage);
        }
        if contains(self.regions.fleet_rows, col, row) {
            return self.fleet_click(col, row);
        }
        if contains(self.regions.map, col, row) {
            self.map_click(col, row);
            return AppAction::None;
        }
        if contains(self.script_area, col, row) {
            return self.script_click(col, row);
        }
        if contains(self.chat_area, col, row) {
            return self.chat_click(col, row);
        }
        AppAction::None
    }

    fn fleet_row_at(&mut self, row: u16) -> Option<(usize, usize)> {
        let rows = self.regions.fleet_rows;
        let position = self.regions.fleet_first + usize::from(row.checked_sub(rows.y)?);
        self.table
            .sync_with_ids(&self.names, &self.profile_ids, &self.fleet);
        let member = *self.table.shown().get(position)?;
        Some((position, member))
    }

    fn fleet_click(&mut self, col: u16, row: u16) -> AppAction {
        let Some((position, member)) = self.fleet_row_at(row) else {
            return AppAction::None;
        };
        self.table.cursor_to(position, &self.names);
        if col < self.regions.fleet_rows.x + MARK_COLUMNS {
            self.table
                .toggle_mark_with_ids(&self.names, &self.profile_ids, member);
            return AppAction::None;
        }
        let name = self.names[member].clone();
        let action = self.select_bot(&name);
        if self.regions.class == SizeClass::Compact {
            self.key_focus = Pane::Detail;
        }
        action
    }

    fn settings_click(&mut self, col: u16, row: u16) -> AppAction {
        if let Some(setting) = self.settings_state.row_at(col, row) {
            self.settings_state.row = setting;
            return self.settings_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        }
        AppAction::None
    }

    /// A right click opens the context menu of the row or pane under the
    /// pointer. It never acts like a left click.
    pub fn on_right_click(&mut self, col: u16, row: u16) -> AppAction {
        if self.params_state.open || self.loadouts_state.open || self.settings_state.open {
            return AppAction::None;
        }
        if self.modal.is_some() {
            if matches!(self.modal, Some(Modal::Context(_)))
                && !contains(self.regions.modal, col, row)
            {
                self.modal = None;
            }
            return AppAction::None;
        }
        if self.script_popup_open() {
            return AppAction::None;
        }
        let menu = if contains(self.regions.fleet_rows, col, row) {
            let Some((_, member)) = self.fleet_row_at(row) else {
                return AppAction::None;
            };
            let name = self.names[member].clone();
            let mut items = vec![
                ContextItem::Select(name.clone()),
                ContextItem::Mark(name.clone()),
            ];
            items.extend(
                Screen::ALL
                    .iter()
                    .map(|screen| ContextItem::Open(name.clone(), *screen)),
            );
            ContextMenu {
                title: name,
                items,
                cursor: 0,
                at: (col, row),
            }
        } else {
            let (title, group) = if contains(self.regions.rects.fleet, col, row) {
                ("Fleet".to_string(), Group::Fleet)
            } else if contains(self.regions.rects.detail, col, row) {
                (self.screen.label().to_string(), screen_group(self.screen))
            } else if contains(self.regions.rects.drawer, col, row) {
                ("Log".to_string(), Group::Logs)
            } else {
                return AppAction::None;
            };
            let items = Command::ALL
                .iter()
                .filter(|command| command.group() == group)
                .map(|command| ContextItem::Run(*command))
                .collect();
            ContextMenu {
                title,
                items,
                cursor: 0,
                at: (col, row),
            }
        };
        self.modal = Some(Modal::Context(menu));
        AppAction::None
    }

    /// The wheel scrolls the list, log or map under the pointer (`delta`
    /// is -1 for up, 1 for down).
    pub fn on_scroll(&mut self, col: u16, row: u16, delta: isize) -> AppAction {
        if self.params_state.open || self.loadouts_state.open || self.settings_state.open {
            return AppAction::None;
        }
        if self.modal.is_some() {
            self.modal_scroll(delta as i32);
            return AppAction::None;
        }
        if self.script_popup_open() {
            self.script_popup_scroll(delta);
            return AppAction::None;
        }
        if contains(self.regions.fleet_rows, col, row) {
            self.table
                .sync_with_ids(&self.names, &self.profile_ids, &self.fleet);
            self.table.move_cursor(delta * 3, &self.names);
        } else if contains(self.regions.log_rows, col, row) {
            self.log.scroll_by(delta * 3);
        } else if contains(self.regions.map, col, row) {
            self.map_pan_rows(-delta as i32);
        }
        AppAction::None
    }
}

#[cfg(test)]
#[path = "input_tests.rs"]
mod tests;
