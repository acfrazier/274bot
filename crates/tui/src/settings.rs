//! Settings popup (spec `2026-09-01-headless-tui-design.md`): an overlay
//! keyed `s` with the focused profile's `random_events`, `lamp_skill`, and
//! `lamp_auto`. The random toggle flips [`ProfileSettings`] in place (the
//! operator vault; `--live` still ephemeral, no persist). Not crowding the
//! main view — a small centered box drawn after the panes.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget, Wrap};

use vault::ProfileSettings;

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
    /// 0 = random events, 1 = lamp skill, 2 = lamp auto.
    pub row: usize,
}

/// The outcome of one settings key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsKey {
    /// A setting value changed — persist [`ProfileSettings`] back.
    Changed,
    /// The key was consumed but nothing changed (navigation, Esc).
    Consumed,
    /// Not a settings key.
    Ignored,
}

/// The settings popup widget over a `ProfileSettings`.
pub struct SettingsPane<'a> {
    pub settings: &'a mut ProfileSettings,
    pub state: &'a mut SettingsState,
}

impl<'a> SettingsPane<'a> {
    pub fn new(settings: &'a mut ProfileSettings, state: &'a mut SettingsState) -> Self {
        Self { settings, state }
    }

    /// One key while the popup is open. Up/Down move the row; Enter/Space
    /// toggle the row's setting (the random toggle flips `random_events`,
    /// lamp auto flips `lamp_auto`, lamp skill cycles [`LAMP_SKILLS`]);
    /// Esc closes.
    pub fn on_key(&mut self, key: KeyEvent) -> SettingsKey {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.state.row = self.state.row.saturating_sub(1);
                SettingsKey::Consumed
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.state.row = (self.state.row + 1).min(2);
                SettingsKey::Consumed
            }
            KeyCode::Enter | KeyCode::Char(' ') => {
                self.activate();
                SettingsKey::Changed
            }
            KeyCode::Esc => {
                self.state.open = false;
                SettingsKey::Consumed
            }
            _ => SettingsKey::Ignored,
        }
    }

    /// The focused row's toggle/cycle.
    fn activate(&mut self) {
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
            _ => self.settings.lamp_auto = !self.settings.lamp_auto,
        }
    }

    /// The popup rect: centered, sized to the three rows.
    pub fn popup_rect(area: Rect) -> Rect {
        let w = area.width.min(34);
        let h = 5.min(area.height);
        Rect {
            x: area.x + area.width.saturating_sub(w) / 2,
            y: area.y + area.height.saturating_sub(h) / 2,
            width: w,
            height: h,
        }
    }
}

impl Widget for SettingsPane<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if !self.state.open {
            return;
        }
        let popup = Self::popup_rect(area);
        Clear.render(popup, buf);
        let block = Block::default().borders(Borders::ALL).title("settings");
        let inner = block.inner(popup);
        block.render(popup, buf);
        let rows = [
            ("random events", format!("{}", self.settings.random_events)),
            ("lamp skill", self.settings.lamp_skill.clone()),
            ("lamp auto", format!("{}", self.settings.lamp_auto)),
        ];
        let lines: Vec<Line> = rows
            .iter()
            .enumerate()
            .map(|(i, (name, value))| {
                let marker = if i == self.state.row { "> " } else { "  " };
                Line::from(format!("{marker}{name}: {value}"))
            })
            .collect();
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .render(inner, buf);
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    use vault::ProfileSettings;

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
        let mut state = SettingsState::default();
        state.open = true;
        assert!(settings.random_events, "default random events on");
        let first = {
            let mut pane = SettingsPane::new(&mut settings, &mut state);
            pane.on_key(key(KeyCode::Enter))
        };
        assert_eq!(first, SettingsKey::Changed, "Enter reports the change");
        assert!(
            !settings.random_events,
            "Enter on the random-events row flips it off"
        );
        let second = {
            let mut pane = SettingsPane::new(&mut settings, &mut state);
            pane.on_key(key(KeyCode::Enter))
        };
        assert_eq!(second, SettingsKey::Changed);
        assert!(settings.random_events, "and back on");
    }

    #[test]
    fn popup_cycles_lamp_skill_and_toggles_lamp_auto() {
        let mut settings = ProfileSettings::default();
        let mut state = SettingsState { open: true, row: 1 };
        {
            let mut pane = SettingsPane::new(&mut settings, &mut state);
            pane.on_key(key(KeyCode::Enter));
        }
        assert_eq!(
            settings.lamp_skill, LAMP_SKILLS[2],
            "default strength (index 1) cycles to the next skill"
        );
        let flipped = {
            let mut pane = SettingsPane::new(&mut settings, &mut state);
            pane.state.row = 2;
            pane.on_key(key(KeyCode::Char(' ')))
        };
        assert_eq!(flipped, SettingsKey::Changed);
        assert!(!settings.lamp_auto, "space toggles lamp auto");
    }

    #[test]
    fn up_and_down_move_the_row_and_esc_closes() {
        let mut settings = ProfileSettings::default();
        let mut state = SettingsState { open: true, row: 0 };
        let rows = {
            let mut pane = SettingsPane::new(&mut settings, &mut state);
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
        let mut state = SettingsState { open: true, row: 0 };
        let text = render(SettingsPane::new(&mut settings, &mut state), 60, 12);
        assert!(text.contains("random events"), "row paints: {text:?}");
        assert!(text.contains("lamp skill"), "row paints: {text:?}");
        state.open = false;
        let text = render(SettingsPane::new(&mut settings, &mut state), 60, 12);
        assert!(
            !text.contains("random events"),
            "closed popup paints nothing: {text:?}"
        );
    }
}
