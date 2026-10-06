//! Settings popup (spec `2026-09-01-headless-tui-design.md`): an overlay
//! (`o` on the Overview, or the palette) with the selected profile's
//! `random_events`, `lamp_skill`, and `lamp_auto`, plus durable global walk
//! grants (teleports / wilderness / danger zones), manual WalkTo bank fetch,
//! and the remembered terrain-bake choice shared with the panel. The random
//! toggle flips
//! [`ProfileSettings`] in place (the operator vault; `--live` still
//! ephemeral, no persist). Not crowding the main view — a small centered
//! box drawn after the panes.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::Span;
use ratatui::widgets::{Block, Borders, Clear, Widget};

use frontend_core::quester_paths::{QuesterPathsView, LOAD_PATHS_LABEL, RELOAD_PATHS_LABEL};
use frontend_core::{
    FormNotice, MapBakeChoice, MemoryNotice, NavPreference, BANK_FETCH_PERMISSION_SCOPE,
    GLOBAL_PERMISSION_LABELS, GLOBAL_PERMISSION_SCOPE, NOTHING_SAVED, SCRIPT_SCOPE_NOTICE,
};
use vault::ProfileSettings;

/// The popup's title while no profile is bound to it.
pub const TITLE: &str = "settings";

/// Lamp skills the popup cycles, in display order.
pub const LAMP_SKILLS: [&str; 7] = [
    "attack",
    "strength",
    "defence",
    "hitpoints",
    "ranged",
    "prayer",
    "magic",
];

/// Mutable settings-pane state: whether the popup is open and which row
/// the operator is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SettingsState {
    pub open: bool,
    /// 0=random events, 1=lamp skill, 2=lamp auto, 3=teleports,
    /// 4=wilderness, 5=bank fetch, 6=danger zones, 7=manual-walk pause,
    /// 8=map bake, 9=memory, 10=folder gate, 11=folder, 12=reload.
    pub row: usize,
    /// Path folder text input is active.
    pub folder_editing: bool,
    /// Text changed during this folder edit and needs persistence when closed.
    pub folder_changed: bool,
    /// Hit targets from the last drawn viewport, including wrapped rows.
    pub row_areas: [Rect; 13],
}

impl SettingsState {
    pub(crate) fn row_at(&self, col: u16, row: u16) -> Option<usize> {
        self.row_areas.iter().position(|area| {
            col >= area.x && col < area.right() && row >= area.y && row < area.bottom()
        })
    }
}

/// The outcome of one settings key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsKey {
    /// A profile setting value changed — persist [`ProfileSettings`] back.
    Changed,
    /// The remembered map-bake choice changed; persist it to the shared
    /// prefs (`panel-ui.json`).
    MapBake,
    /// A durable global walk permission changed.
    WalkGlobal(NavPreference),
    /// The one-time script-scope notice was dismissed.
    ScriptScopeNoticeAck,
    /// The global manual-walk pause preference changed and needs persistence.
    PauseScriptOnManualWalkAbort,
    /// The Quester folder settings changed and need shared persistence.
    QuesterPathsChanged,
    /// Reload the shared Path registry without starting a Path.
    ReloadPaths,
    /// Relog the bound member now to apply the entire queued memory mode.
    MemoryRelog,
    /// The key was consumed but nothing changed (navigation, Esc).
    Consumed,
    /// Not a settings key.
    Ignored,
}

/// The settings popup widget over a `ProfileSettings`. `title` names the
/// bound profile (for example `settings — alice`). `notice` is the bound
/// profile's save feedback, drawn under the rows: a refusal or a failed
/// write in red with [`NOTHING_SAVED`], or `Saved <name>.` in green once the
/// write is durable. `memory` is the bound slot's applied vs queued mode;
/// changes take effect only at the next login, with an explicit `r` offer.
/// The notices are drawn into the same buffer as the settings rows.
pub struct SettingsPane<'a> {
    pub settings: &'a mut ProfileSettings,
    pub nav: &'a mut host_play::WalkGlobals,
    pub map_bake: &'a mut MapBakeChoice,
    pub state: &'a mut SettingsState,
    pub pause_script_on_manual_walk_abort: Option<&'a mut bool>,
    pub script_scope_notice_ack: Option<&'a mut bool>,
    pub quester_paths: Option<&'a mut QuesterPathsView>,
    pub quester_paths_notice: Option<&'a str>,
    pub quester_paths_notice_error: bool,
    pub quester_paths_reloading: bool,
    pub title: &'a str,
    pub notice: Option<&'a FormNotice>,
    pub memory: Option<MemoryNotice>,
}

impl<'a> SettingsPane<'a> {
    pub fn new(
        settings: &'a mut ProfileSettings,
        nav: &'a mut host_play::WalkGlobals,
        map_bake: &'a mut MapBakeChoice,
        state: &'a mut SettingsState,
    ) -> Self {
        Self {
            settings,
            nav,
            map_bake,
            state,
            title: TITLE,
            notice: None,
            memory: None,
            pause_script_on_manual_walk_abort: None,
            script_scope_notice_ack: None,
            quester_paths: None,
            quester_paths_notice: None,
            quester_paths_notice_error: false,
            quester_paths_reloading: false,
        }
    }
    /// Bind the shared script-scope acknowledgement.
    pub fn script_scope_notice_ack(mut self, value: &'a mut bool) -> Self {
        self.script_scope_notice_ack = Some(value);
        self
    }

    /// Bind the shared global pause preference to the settings row.
    pub fn pause_script_on_manual_walk_abort(mut self, value: &'a mut bool) -> Self {
        self.pause_script_on_manual_walk_abort = Some(value);
        self
    }

    /// Bind shared Quester Path settings.
    pub fn quester_paths(mut self, value: &'a mut QuesterPathsView) -> Self {
        self.quester_paths = Some(value);
        self
    }

    /// One key while the popup is open. Up/Down move the row; Enter/Space
    /// toggles settings, `d` dismisses the one-time script-scope notice,
    /// `r` relogs the bound member only while its login mode differs and no
    /// relog is already queued; Esc closes.
    pub fn on_key(&mut self, key: KeyEvent) -> SettingsKey {
        if self.state.folder_editing {
            return self.folder_key(key);
        }
        match key.code {
            KeyCode::Char('r') if self.memory.is_some_and(MemoryNotice::can_relog) => {
                SettingsKey::MemoryRelog
            }
            KeyCode::Char('r') => SettingsKey::Consumed,
            KeyCode::Char('d')
                if self
                    .script_scope_notice_ack
                    .as_deref()
                    .is_some_and(|ack| !*ack) =>
            {
                SettingsKey::ScriptScopeNoticeAck
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.state.row = self.state.row.saturating_sub(1);
                SettingsKey::Consumed
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.state.row = (self.state.row + 1).min(12);
                SettingsKey::Consumed
            }
            KeyCode::Enter => self.activate(),
            KeyCode::Char(' ') if self.state.row == 11 => SettingsKey::Consumed,
            KeyCode::Char(' ') => self.activate(),
            KeyCode::Esc => {
                self.state.open = false;
                SettingsKey::Consumed
            }
            _ => SettingsKey::Ignored,
        }
    }

    fn folder_key(&mut self, key: KeyEvent) -> SettingsKey {
        match key.code {
            KeyCode::Enter | KeyCode::Esc => {
                self.state.folder_editing = false;
                if std::mem::take(&mut self.state.folder_changed) {
                    SettingsKey::QuesterPathsChanged
                } else {
                    SettingsKey::Consumed
                }
            }
            KeyCode::Backspace => {
                let Some(paths) = self.quester_paths.as_deref_mut() else {
                    return SettingsKey::Consumed;
                };
                let mut folder = paths.folder.to_string_lossy().into_owned();
                if folder.pop().is_some() {
                    paths.folder = folder.into();
                    self.state.folder_changed = true;
                }
                SettingsKey::Consumed
            }
            KeyCode::Char(character)
                if !key.modifiers.intersects(
                    KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER,
                ) =>
            {
                if let Some(paths) = self.quester_paths.as_deref_mut() {
                    let mut folder = paths.folder.to_string_lossy().into_owned();
                    folder.push(character);
                    paths.folder = folder.into();
                    self.state.folder_changed = true;
                }
                SettingsKey::Consumed
            }
            _ => SettingsKey::Consumed,
        }
    }

    /// The focused row's toggle/cycle or command.
    fn activate(&mut self) -> SettingsKey {
        match self.state.row {
            0..=2 | 9 => {
                self.activate_value();
                SettingsKey::Changed
            }
            3 => {
                self.activate_value();
                SettingsKey::WalkGlobal(NavPreference::AllowTeleports)
            }
            4 => {
                self.activate_value();
                SettingsKey::WalkGlobal(NavPreference::AllowWilderness)
            }
            5 => {
                self.activate_value();
                SettingsKey::WalkGlobal(NavPreference::AllowBankFetch)
            }
            6 => {
                self.activate_value();
                SettingsKey::WalkGlobal(NavPreference::AllowDangerZones)
            }
            7 if self.pause_script_on_manual_walk_abort.is_some() => {
                self.activate_value();
                SettingsKey::PauseScriptOnManualWalkAbort
            }
            8 => {
                self.activate_value();
                SettingsKey::MapBake
            }
            10 => {
                let Some(paths) = self.quester_paths.as_deref_mut() else {
                    return SettingsKey::Consumed;
                };
                paths.enabled = !paths.enabled;
                SettingsKey::QuesterPathsChanged
            }
            11 => {
                if self.quester_paths.is_none() {
                    return SettingsKey::Consumed;
                }
                self.state.folder_editing = true;
                self.state.folder_changed = false;
                SettingsKey::Consumed
            }
            12 => SettingsKey::ReloadPaths,
            _ => SettingsKey::Consumed,
        }
    }

    fn activate_value(&mut self) {
        match self.state.row {
            0 => self.settings.random_events = !self.settings.random_events,
            1 => {
                let next = LAMP_SKILLS
                    .iter()
                    .position(|s| *s == self.settings.lamp_skill)
                    .map(|i| (i + 1) % LAMP_SKILLS.len())
                    .unwrap_or(0);
                self.settings.lamp_skill = LAMP_SKILLS[next].into();
            }
            2 => self.settings.lamp_auto = !self.settings.lamp_auto,
            3 => self.nav.allow_teleports = !self.nav.allow_teleports,
            4 => self.nav.allow_wilderness = !self.nav.allow_wilderness,
            5 => self.nav.allow_bank_fetch = !self.nav.allow_bank_fetch,
            6 => self.nav.allow_danger_zones = !self.nav.allow_danger_zones,
            7 => {
                if let Some(pause) = self.pause_script_on_manual_walk_abort.as_mut() {
                    **pause = !**pause;
                }
            }
            8 => *self.map_bake = self.map_bake.toggled(),
            9 => self.settings.lowmem = !self.settings.lowmem,
            _ => {}
        }
    }
}

impl Widget for SettingsPane<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        self.state.row_areas.fill(Rect::default());
        if !self.state.open || area.width < 3 || area.height < 4 {
            return;
        }
        let memory_note = self
            .memory
            .filter(|n| n.differs())
            .map_or(MemoryNotice::NEXT_LOGIN_NOTE, |n| n.notice_text());
        let show_scope_notice = self
            .script_scope_notice_ack
            .as_deref()
            .is_some_and(|ack| !*ack);
        let marker = |row| if row == self.state.row { "> " } else { "  " };
        let path_folder = self
            .quester_paths
            .as_deref()
            .map(|paths| paths.folder.display().to_string())
            .unwrap_or_else(|| "unavailable".into());
        let paths_enabled = self
            .quester_paths
            .as_deref()
            .is_some_and(|paths| paths.enabled);
        let reload = if self.quester_paths_reloading {
            format!("{} (reloading…)", RELOAD_PATHS_LABEL)
        } else {
            RELOAD_PATHS_LABEL.to_string()
        };
        let folder_hint = if self.state.folder_editing {
            " [type path; Enter/Esc done]"
        } else {
            " [Enter edit]"
        };
        let rows = [
            format!(
                "{}random events: {}",
                marker(0),
                self.settings.random_events
            ),
            format!("{}lamp skill: {}", marker(1), self.settings.lamp_skill),
            format!("{}lamp auto: {}", marker(2), self.settings.lamp_auto),
            format!(
                "{}{}: {} · {}",
                marker(3),
                GLOBAL_PERMISSION_LABELS[0].1,
                self.nav.allow_teleports,
                GLOBAL_PERMISSION_SCOPE
            ),
            format!(
                "{}{}: {} · {}",
                marker(4),
                GLOBAL_PERMISSION_LABELS[1].1,
                self.nav.allow_wilderness,
                GLOBAL_PERMISSION_SCOPE
            ),
            format!(
                "{}{}: {} · {}",
                marker(5),
                GLOBAL_PERMISSION_LABELS[2].1,
                self.nav.allow_bank_fetch,
                BANK_FETCH_PERMISSION_SCOPE
            ),
            format!(
                "{}{}: {} · {}",
                marker(6),
                GLOBAL_PERMISSION_LABELS[3].1,
                self.nav.allow_danger_zones,
                GLOBAL_PERMISSION_SCOPE
            ),
            format!(
                "{}Pause script on manual movement: {}",
                marker(7),
                self.pause_script_on_manual_walk_abort
                    .as_deref()
                    .copied()
                    .unwrap_or(true)
            ),
            format!("{}map bake: {}", marker(8), self.map_bake.as_str()),
            format!(
                "{}memory: {}",
                marker(9),
                MemoryNotice::status_text(self.settings.lowmem, self.memory)
            ),
            format!("{}{}: {}", marker(10), LOAD_PATHS_LABEL, paths_enabled),
            format!("{}Paths folder: {}{}", marker(11), path_folder, folder_hint),
            format!("{}{}", marker(12), reload),
        ];
        let width = area.width.min(100);
        let columns = usize::from(width - 2);
        let row_heights = rows.each_ref().map(|row| wrapped_rows(row, columns));
        let memory_height = wrapped_rows(memory_note, columns);
        let scope_height = if show_scope_notice {
            wrapped_rows(SCRIPT_SCOPE_NOTICE, columns) + 1
        } else {
            0
        };
        let notice_height = self.notice.map_or(0, |notice| {
            notice.error().map_or_else(
                || wrapped_rows(notice.text(), columns),
                |reason| wrapped_rows(reason, columns) + wrapped_rows(NOTHING_SAVED, columns),
            )
        });
        let path_notice_height = self
            .quester_paths_notice
            .map_or(0, |notice| wrapped_rows(notice, columns));
        let height = (row_heights.iter().sum::<u16>()
            + memory_height
            + scope_height
            + notice_height
            + path_notice_height
            + 2)
        .min(area.height - 1);
        let popup = Rect {
            x: area.x + (area.width - width) / 2,
            y: area.y + (area.height - height) / 2,
            width,
            height,
        };
        Clear.render(popup, buf);
        let block = Block::default().borders(Borders::ALL).title(self.title);
        let inner = block.inner(popup);
        block.render(popup, buf);

        // Keep feedback visible and the selected setting reachable when the
        // long global labels or scope notice cannot fit the terminal.
        let footer_capacity = inner.height.saturating_sub(1);
        let path_notice_height = path_notice_height.min(footer_capacity);
        let notice_height = notice_height.min(footer_capacity - path_notice_height);
        let memory_height = memory_height.min(footer_capacity - path_notice_height - notice_height);
        let scope_height =
            scope_height.min(footer_capacity - path_notice_height - notice_height - memory_height);
        let rows_height =
            inner.height - path_notice_height - notice_height - memory_height - scope_height;
        let selected = self.state.row.min(rows.len() - 1);
        let mut first = 0;
        let mut through_selected = row_heights[..=selected].iter().sum::<u16>();
        while first < selected && through_selected > rows_height {
            through_selected -= row_heights[first];
            first += 1;
        }
        let rows_bottom = inner.y + rows_height;
        let mut y = inner.y;
        for (index, text) in rows.iter().enumerate().skip(first) {
            if y >= rows_bottom {
                break;
            }
            let bottom = (y + row_heights[index]).min(rows_bottom);
            self.state.row_areas[index] = Rect::new(inner.x, y, inner.width, bottom - y);
            y = draw_wrapped(buf, inner, y, bottom, text, Style::default());
        }

        let yellow = Style::default().fg(Color::Yellow);
        let memory_bottom = rows_bottom + memory_height;
        draw_wrapped(buf, inner, rows_bottom, memory_bottom, memory_note, yellow);
        let scope_bottom = memory_bottom + scope_height;
        if show_scope_notice && scope_height > 0 {
            let dismiss_y = scope_bottom.saturating_sub(1);
            draw_wrapped(
                buf,
                inner,
                memory_bottom,
                dismiss_y,
                SCRIPT_SCOPE_NOTICE,
                yellow,
            );
            draw_wrapped(
                buf,
                inner,
                dismiss_y,
                scope_bottom,
                "[d] dismiss this notice",
                yellow,
            );
        }
        let profile_notice_bottom = inner.bottom().saturating_sub(path_notice_height);
        if let Some(notice) = self.notice {
            match notice.error() {
                Some(reason) => {
                    let red = Style::default().fg(Color::Red);
                    let saved_y =
                        profile_notice_bottom.saturating_sub(wrapped_rows(NOTHING_SAVED, columns));
                    draw_wrapped(buf, inner, scope_bottom, saved_y, reason, red);
                    draw_wrapped(
                        buf,
                        inner,
                        saved_y,
                        profile_notice_bottom,
                        NOTHING_SAVED,
                        red,
                    );
                }
                None => {
                    draw_wrapped(
                        buf,
                        inner,
                        scope_bottom,
                        profile_notice_bottom,
                        notice.text(),
                        Style::default().fg(Color::Green),
                    );
                }
            }
        }
        if let Some(notice) = self.quester_paths_notice {
            let color = if self.quester_paths_notice_error {
                Color::Red
            } else {
                Color::Green
            };
            draw_wrapped(
                buf,
                inner,
                profile_notice_bottom,
                inner.bottom(),
                notice,
                Style::default().fg(color),
            );
        }
    }
}

/// The width in columns of one character, as [`Span`] measures it.
fn char_width(text: &str, at: usize, ch: char) -> usize {
    Span::raw(&text[at..at + ch.len_utf8()]).width()
}

/// How many rows `text` takes wrapped at `width` columns (a character
/// wrap, as [`draw_wrapped`] draws it). Empty text takes none.
fn wrapped_rows(text: &str, width: usize) -> u16 {
    let mut rows = 0;
    let mut used = width;
    for (at, ch) in text.char_indices() {
        let w = char_width(text, at, ch);
        if used + w > width {
            rows += 1;
            used = 0;
        }
        used += w;
    }
    rows
}

/// Draw `text` into `buf` inside `inner` from row `y`, wrapped at the
/// popup's width, stopping at `bottom`. Returns the next free row. Slices
/// the borrowed text, so it allocates nothing.
fn draw_wrapped(
    buf: &mut Buffer,
    inner: Rect,
    mut y: u16,
    bottom: u16,
    text: &str,
    style: Style,
) -> u16 {
    let width = usize::from(inner.width).max(1);
    let mut start = 0;
    let mut used = 0;
    for (at, ch) in text.char_indices() {
        let w = char_width(text, at, ch);
        if used + w > width {
            if y < bottom {
                buf.set_stringn(inner.x, y, &text[start..at], width, style);
            }
            y += 1;
            start = at;
            used = 0;
        }
        used += w;
    }
    if start < text.len() {
        if y < bottom {
            buf.set_stringn(inner.x, y, &text[start..], width, style);
        }
        y += 1;
    }
    y
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    use frontend_core::{MapBakeChoice, MemoryNotice, NavPreference};
    use vault::ProfileSettings;

    use host_play::WalkGlobals;

    use super::{SettingsKey, SettingsPane, SettingsState, LAMP_SKILLS};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn render(pane: SettingsPane<'_>, w: u16, h: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        terminal
            .draw(|frame| frame.render_widget(pane, frame.area()))
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    /// The spec's settings test: the popup flips `random_events` on a
    /// mock `ProfileSettings`.
    #[test]
    fn popup_flips_random_events_on_the_profile() {
        let mut settings = ProfileSettings::default();
        let mut nav = WalkGlobals::default();
        let mut bake = MapBakeChoice::Ask;
        let mut state = SettingsState {
            open: true,
            ..Default::default()
        };
        assert!(settings.random_events, "default random events on");
        let first = {
            let mut pane = SettingsPane::new(&mut settings, &mut nav, &mut bake, &mut state);
            pane.on_key(key(KeyCode::Enter))
        };
        assert_eq!(first, SettingsKey::Changed, "Enter reports the change");
        assert!(
            !settings.random_events,
            "Enter on the random-events row flips it off"
        );
        let second = {
            let mut pane = SettingsPane::new(&mut settings, &mut nav, &mut bake, &mut state);
            pane.on_key(key(KeyCode::Enter))
        };
        assert_eq!(second, SettingsKey::Changed);
        assert!(settings.random_events, "and back on");
    }

    #[test]
    fn popup_cycles_lamp_skill_and_toggles_lamp_auto() {
        let mut settings = ProfileSettings::default();
        let mut nav = WalkGlobals::default();
        let mut bake = MapBakeChoice::Ask;
        let mut state = SettingsState {
            open: true,
            row: 1,
            ..Default::default()
        };
        {
            let mut pane = SettingsPane::new(&mut settings, &mut nav, &mut bake, &mut state);
            pane.on_key(key(KeyCode::Enter));
        }
        assert_eq!(
            settings.lamp_skill, LAMP_SKILLS[2],
            "default strength (index 1) cycles to the next skill"
        );
        let flipped = {
            let mut pane = SettingsPane::new(&mut settings, &mut nav, &mut bake, &mut state);
            pane.state.row = 2;
            pane.on_key(key(KeyCode::Char(' ')))
        };
        assert_eq!(flipped, SettingsKey::Changed);
        assert!(!settings.lamp_auto, "space toggles lamp auto");
    }

    #[test]
    fn popup_toggles_bank_fetch_and_requests_single_key_persistence() {
        let mut settings = ProfileSettings::default();
        let mut nav = WalkGlobals::default();
        let mut bake = MapBakeChoice::Ask;
        let mut state = SettingsState {
            open: true,
            row: 5,
            ..Default::default()
        };
        let key = {
            let mut pane = SettingsPane::new(&mut settings, &mut nav, &mut bake, &mut state);
            pane.on_key(key(KeyCode::Enter))
        };
        assert_eq!(key, SettingsKey::WalkGlobal(NavPreference::AllowBankFetch));
        assert!(nav.allow_bank_fetch, "bank fetch toggles on");
    }

    #[test]
    fn each_permission_row_requests_its_global_preference() {
        let mut settings = ProfileSettings::default();
        let mut nav = WalkGlobals::default();
        let mut bake = MapBakeChoice::Ask;
        for (row, preference) in [
            (3, NavPreference::AllowTeleports),
            (4, NavPreference::AllowWilderness),
            (5, NavPreference::AllowBankFetch),
            (6, NavPreference::AllowDangerZones),
        ] {
            let mut state = SettingsState {
                open: true,
                row,
                ..Default::default()
            };
            let outcome = {
                let mut pane = SettingsPane::new(&mut settings, &mut nav, &mut bake, &mut state);
                pane.on_key(key(KeyCode::Enter))
            };
            assert_eq!(outcome, SettingsKey::WalkGlobal(preference));
        }
        assert!(nav.allow_teleports);
        assert!(nav.allow_wilderness);
        assert!(nav.allow_bank_fetch);
        assert!(nav.allow_danger_zones);
    }

    #[test]
    fn script_scope_notice_is_dismissible_and_stays_hidden_after_ack() {
        let mut settings = ProfileSettings::default();
        let mut nav = WalkGlobals::default();
        let mut bake = MapBakeChoice::Ask;
        let mut state = SettingsState {
            open: true,
            row: 0,
            ..Default::default()
        };
        let mut acknowledged = false;
        let text = render(
            SettingsPane::new(&mut settings, &mut nav, &mut bake, &mut state)
                .script_scope_notice_ack(&mut acknowledged),
            100,
            24,
        );
        assert!(
            text.contains("Teleports and wilderness in Nav config"),
            "the unacknowledged notice is shown: {text:?}"
        );
        let outcome = {
            let mut pane = SettingsPane::new(&mut settings, &mut nav, &mut bake, &mut state)
                .script_scope_notice_ack(&mut acknowledged);
            pane.on_key(key(KeyCode::Char('d')))
        };
        assert_eq!(outcome, SettingsKey::ScriptScopeNoticeAck);
        acknowledged = true;
        let text = render(
            SettingsPane::new(&mut settings, &mut nav, &mut bake, &mut state)
                .script_scope_notice_ack(&mut acknowledged),
            100,
            24,
        );
        assert!(!text.contains("Teleports and wilderness in Nav config"));
    }

    #[test]
    fn up_and_down_move_the_row_and_esc_closes() {
        let mut settings = ProfileSettings::default();
        let mut nav = WalkGlobals::default();
        let mut bake = MapBakeChoice::Ask;
        let mut state = SettingsState {
            open: true,
            row: 0,
            ..Default::default()
        };
        let rows = {
            let mut pane = SettingsPane::new(&mut settings, &mut nav, &mut bake, &mut state);
            let down = pane.on_key(key(KeyCode::Down));
            let at = pane.state.row;
            let up = pane.on_key(key(KeyCode::Up));
            (down, at, up, pane.on_key(key(KeyCode::Esc)))
        };
        assert_eq!(rows.0, SettingsKey::Consumed);
        assert_eq!(rows.1, 1, "Down moves the row");
        assert_eq!(rows.2, SettingsKey::Consumed);
        assert_eq!(rows.3, SettingsKey::Consumed);
        assert!(!state.open, "Esc closes the popup");
    }

    #[test]
    fn popup_draws_the_rows_while_open_and_nothing_when_closed() {
        let mut settings = ProfileSettings::default();
        let mut nav = WalkGlobals::default();
        let mut bake = MapBakeChoice::Ask;
        let mut state = SettingsState {
            open: true,
            row: 0,
            ..Default::default()
        };
        let text = render(
            SettingsPane::new(&mut settings, &mut nav, &mut bake, &mut state),
            60,
            14,
        );
        assert!(text.contains("random events"), "row paints: {text:?}");
        assert!(text.contains("bank fetch"), "nav row paints: {text:?}");
        state.open = false;
        let text = render(
            SettingsPane::new(&mut settings, &mut nav, &mut bake, &mut state),
            60,
            14,
        );
        assert!(
            !text.contains("random events"),
            "closed popup paints nothing: {text:?}"
        );
    }

    #[test]
    fn manual_movement_pause_row_is_reachable_persistent_and_visible_at_common_sizes() {
        for (width, height) in [(120, 40), (80, 24)] {
            let mut settings = ProfileSettings::default();
            let mut nav = WalkGlobals::default();
            let mut bake = MapBakeChoice::Ask;
            let mut pause = true;
            let mut state = SettingsState {
                open: true,
                row: 0,
                ..Default::default()
            };
            {
                let mut pane = SettingsPane::new(&mut settings, &mut nav, &mut bake, &mut state)
                    .pause_script_on_manual_walk_abort(&mut pause);
                for _ in 0..7 {
                    assert_eq!(pane.on_key(key(KeyCode::Down)), SettingsKey::Consumed);
                }
                assert_eq!(pane.state.row, 7);
                assert_eq!(
                    pane.on_key(key(KeyCode::Enter)),
                    SettingsKey::PauseScriptOnManualWalkAbort
                );
            }
            assert!(!pause, "activation flips the saved global toggle off");
            let text = render(
                SettingsPane::new(&mut settings, &mut nav, &mut bake, &mut state)
                    .pause_script_on_manual_walk_abort(&mut pause),
                width,
                height,
            );
            assert!(
                text.contains("Pause script on manual movement: false"),
                "{width}x{height} must render the reachable row: {text:?}"
            );
        }
    }
    #[test]
    fn memory_row_toggles_lowmem_and_reports_the_change() {
        let mut settings = ProfileSettings::default();
        let mut nav = WalkGlobals::default();
        let mut bake = MapBakeChoice::Ask;
        let mut state = SettingsState {
            open: true,
            row: 9,
            ..Default::default()
        };
        let outcome = {
            let mut pane = SettingsPane::new(&mut settings, &mut nav, &mut bake, &mut state);
            pane.on_key(key(KeyCode::Enter))
        };
        assert_eq!(outcome, SettingsKey::Changed, "the binary must persist it");
        assert!(!settings.lowmem, "Enter on the memory row flips to highmem");
    }

    #[test]
    fn r_requests_relog_only_while_the_server_mode_differs_and_none_is_queued() {
        let mut settings = ProfileSettings::default();
        let mut nav = WalkGlobals::default();
        let mut bake = MapBakeChoice::Ask;
        let mut state = SettingsState {
            open: true,
            row: 0,
            ..Default::default()
        };
        let mut pane = SettingsPane::new(&mut settings, &mut nav, &mut bake, &mut state);
        assert_eq!(
            pane.on_key(key(KeyCode::Char('r'))),
            SettingsKey::Consumed,
            "no divergence means no relog"
        );
        pane.memory = Some(MemoryNotice {
            login_lowmem: false,
            desired_lowmem: true,
            relog_pending: false,
            connected: true,
            login_applies_memory: true,
        });
        assert_eq!(
            pane.on_key(key(KeyCode::Char('r'))),
            SettingsKey::MemoryRelog
        );
        pane.memory.as_mut().unwrap().relog_pending = true;
        assert_eq!(
            pane.on_key(key(KeyCode::Char('r'))),
            SettingsKey::Consumed,
            "a queued relog cannot be restarted"
        );
        assert!(settings.lowmem, "r never flips the setting itself");
    }

    #[test]
    fn popup_shows_the_login_mode_and_relog_hint_while_it_differs() {
        let mut settings = ProfileSettings {
            lowmem: false,
            ..ProfileSettings::default()
        };
        let mut nav = WalkGlobals::default();
        let mut bake = MapBakeChoice::Ask;
        let mut state = SettingsState {
            open: true,
            row: 9,
            ..Default::default()
        };
        let mut pane = SettingsPane::new(&mut settings, &mut nav, &mut bake, &mut state);
        pane.memory = Some(MemoryNotice {
            login_lowmem: true,
            desired_lowmem: false,
            relog_pending: false,
            connected: true,
            login_applies_memory: true,
        });
        let text = render(pane, 60, 14);
        assert!(
            text.contains("memory: lowmem (next login highmem)"),
            "{text:?}"
        );
        assert!(text.contains("Relog now"), "{text:?}");
    }

    #[test]
    fn map_bake_row_flips_the_shared_choice_and_asks_the_binary_to_persist_it() {
        let mut settings = ProfileSettings::default();
        let mut nav = WalkGlobals::default();
        let mut bake = MapBakeChoice::Ask;
        let mut state = SettingsState {
            open: true,
            row: 7,
            ..Default::default()
        };
        let (moved, flipped) = {
            let mut pane = SettingsPane::new(&mut settings, &mut nav, &mut bake, &mut state);
            let moved = pane.on_key(key(KeyCode::Down));
            (moved, pane.on_key(key(KeyCode::Enter)))
        };
        assert_eq!(moved, SettingsKey::Consumed);
        assert_eq!(state.row, 8, "the map-bake row follows the pause toggle");
        assert_eq!(flipped, SettingsKey::MapBake);
        assert_eq!(bake, MapBakeChoice::Always);
        assert!(!nav.allow_bank_fetch, "the bank row is untouched");
        let text = render(
            SettingsPane::new(&mut settings, &mut nav, &mut bake, &mut state),
            60,
            14,
        );
        assert!(text.contains("map bake: always"), "row paints: {text:?}");
        let back = {
            let mut pane = SettingsPane::new(&mut settings, &mut nav, &mut bake, &mut state);
            pane.on_key(key(KeyCode::Char(' ')))
        };
        assert_eq!(back, SettingsKey::MapBake);
        assert_eq!(bake, MapBakeChoice::Ask);
    }
}
