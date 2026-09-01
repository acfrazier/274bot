//! `TuiApp`: view model for the headless panel. The binary (`tui-play`)
//! polls slot statuses and the focused snapshot each frame, refreshes the
//! app, routes keys/clicks, and dispatches the returned [`AppAction`] onto
//! `host_play::Play`. The panes are plain widgets over owned view data
//! (copied from the snapshot each pump), so CI renders them with
//! `TestBackend` and no real terminal.

use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Layout, Position, Rect};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Paragraph, Widget, Wrap};
use ratatui::Frame;

use api::snapshot::{ChatLineView, ChatOptionView, WorldTile};
use host_play::SlotStatus;
use nav::router::Route;
use nav::tile::Tile;
use nav::world::NavWorld;
use script::RunState;

use crate::chat::{chat_modal_open, Chat, ChatAction, ChatState, ChatView};
use crate::map::{Map, MapAction, MapView};
use crate::script_shape::ScriptPane;
use crate::settings::{SettingsKey, SettingsPane, SettingsState};
use crate::status::StatusPane;

/// One operator action the binary dispatches onto `host_play::Play`.
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
    /// Nothing to dispatch.
    None,
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
    pub settings_state: SettingsState,
    pub settings_dirty: bool,
    /// The focused slot's script lifecycle (shape display only).
    pub script_state: RunState,
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
            settings_state: SettingsState::default(),
            settings_dirty: false,
            script_state: RunState::Idle,
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
                level: 0,
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

    /// The slot name whose span the strip click lands on. The strip is
    /// `[{names joined by " "}] …`, so name `i` starts one cell after the
    /// leading `[` plus every earlier `len + 1` span.
    fn strip_select(&self, col: u16) -> Option<String> {
        let mut cursor = 1u16; // after the leading `[`
        for name in &self.names {
            let len = name.len() as u16;
            if col >= cursor && col < cursor + len {
                return Some(name.clone());
            }
            cursor += len + 1;
        }
        None
    }

    /// One key event. Priority: global keys, the settings popup, the chat
    /// modal (when open), then WASD + map keys.
    pub fn on_key(&mut self, key: KeyEvent) -> AppAction {
        if self.quit {
            return AppAction::None;
        }
        match key.code {
            KeyCode::Char('q') => {
                self.quit = true;
                return AppAction::Quit;
            }
            KeyCode::Char('s') => {
                self.settings_state.open = !self.settings_state.open;
                return AppAction::None;
            }
            KeyCode::Char('m') => return AppAction::SpawnAll,
            KeyCode::Tab => {
                return self
                    .cycle_focus()
                    .map(AppAction::Focus)
                    .unwrap_or(AppAction::None)
            }
            _ => {}
        }
        if self.settings_state.open {
            let mut pane = SettingsPane::new(&mut self.settings, &mut self.settings_state);
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

    /// One mouse click (crossterm col/row). The strip selects a slot; the
    /// chat pane answers options / continues; the script pane answers
    /// button labels the binary ignores (not wired this tag).
    pub fn on_click(&mut self, col: u16, row: u16) -> AppAction {
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
            // Shape only: the label is ignored until wired scripts land.
            return AppAction::None;
        }
        AppAction::None
    }

    /// Render the full layout (spec): slot strip, map, chat, status |
    /// inv/stats/locs, script shape, then the settings popup overlay.
    pub fn draw(&mut self, frame: &mut Frame<'_>) {
        let area = frame.area();
        let chunks = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(8),
            Constraint::Length(6),
            Constraint::Min(6),
            Constraint::Length(4),
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
            let pane = SettingsPane::new(&mut self.settings, &mut self.settings_state);
            frame.render_widget(pane, area);
        }
    }

    fn draw_strip(&mut self, frame: &mut Frame<'_>, area: Rect) {
        let focused = self.focused_name().unwrap_or_else(|| "_".into());
        let mut text = format!(
            "[{}]  focused: {focused}   {}   q quit · s settings · Tab focus",
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
        let pane = ScriptPane::new(self.script_state, None);
        frame.render_widget(pane, area);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    use api::snapshot::{ChatLineView, ChatOptionView};
    use nav::tile::Tile;
    use script::RunState;
    use vault::ProfileSettings;

    use super::{wasd_target, AppAction, TuiApp};

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

    #[test]
    fn q_quits_and_s_toggles_settings() {
        let mut app = TuiApp::new("274bot headless");
        assert_eq!(app.on_key(key(KeyCode::Char('s'))), AppAction::None);
        assert!(app.settings_state.open, "s opens the settings popup");
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
        assert_eq!(app.on_click(3, 0), AppAction::Focus("b".into()));
        // Between the names is a miss.
        assert_eq!(app.on_click(2, 0), AppAction::None);
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
}
