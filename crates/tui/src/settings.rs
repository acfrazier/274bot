//! Settings popup (spec `2026-09-01-headless-tui-design.md`): an overlay
//! (`o` on the Overview, or the palette) with the selected profile's
//! `random_events`, `lamp_skill`, and `lamp_auto`, plus session nav find
//! opt-ins (teleports / wilderness / BankBudget) and the remembered WalkTo
//! terrain-bake choice shared with the panel. Bank fetch plans from the
//! open bank only — a closed bank has no inventory, so a fetch walk that
//! needs a banked item reports no path until the bank is open. The random
//! toggle flips
//! [`ProfileSettings`] in place (the operator vault; `--live` still
//! ephemeral, no persist). Not crowding the main view — a small centered
//! box drawn after the panes.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget, Wrap};

use crate::app::NavFindSettings;
use frontend_core::{FormNotice, MapBakeChoice, MemoryNotice, NOTHING_SAVED};
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
    /// 0 = random events, 1 = lamp skill, 2 = lamp auto, 3 = teleports,
    /// 4 = wilderness, 5 = bank fetch, 6 = map bake, 7 = memory.
    pub row: usize,
}

/// The outcome of one settings key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsKey {
    /// A profile setting value changed — persist [`ProfileSettings`] back.
    Changed,
    /// The remembered map-bake choice changed — persist it to the shared
    /// prefs (`panel-ui.json`).
    MapBake,
    /// Relog the bound member now so a memory-mode switch reaches the
    /// server (the binary confirms first when a script would stop).
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
/// write is durable. `memory` is the bound slot's login-time vs current
/// memory mode, drawn under the memory row while they differ (with the `r`
/// relog hint). The notice is drawn straight into the buffer, so it costs a
/// frame no allocation.
pub struct SettingsPane<'a> {
    pub settings: &'a mut ProfileSettings,
    pub nav: &'a mut NavFindSettings,
    pub map_bake: &'a mut MapBakeChoice,
    pub state: &'a mut SettingsState,
    pub title: &'a str,
    pub notice: Option<&'a FormNotice>,
    pub memory: Option<MemoryNotice>,
}

impl<'a> SettingsPane<'a> {
    pub fn new(
        settings: &'a mut ProfileSettings,
        nav: &'a mut NavFindSettings,
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
        }
    }

    /// One key while the popup is open. Up/Down move the row; Enter/Space
    /// toggle the row's setting (the random toggle flips `random_events`,
    /// lamp auto flips `lamp_auto`, lamp skill cycles [`LAMP_SKILLS`], nav
    /// rows flip session find opt-ins, the map-bake row flips ask / always,
    /// the memory row flips highmem / lowmem); `r` relogs the bound member
    /// now so a memory switch reaches the server; Esc closes.
    pub fn on_key(&mut self, key: KeyEvent) -> SettingsKey {
        match key.code {
            KeyCode::Char('r') => SettingsKey::MemoryRelog,
            KeyCode::Up | KeyCode::Char('k') => {
                self.state.row = self.state.row.saturating_sub(1);
                SettingsKey::Consumed
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.state.row = (self.state.row + 1).min(7);
                SettingsKey::Consumed
            }
            KeyCode::Enter | KeyCode::Char(' ') => {
                self.activate();
                match self.state.row {
                    0..=2 | 7 => SettingsKey::Changed,
                    6 => SettingsKey::MapBake,
                    _ => SettingsKey::Consumed,
                }
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
            2 => self.settings.lamp_auto = !self.settings.lamp_auto,
            3 => self.nav.allow_teleports = !self.nav.allow_teleports,
            4 => self.nav.allow_wilderness = !self.nav.allow_wilderness,
            5 => self.nav.allow_bank_fetch = !self.nav.allow_bank_fetch,
            7 => self.settings.lowmem = !self.settings.lowmem,
            _ => *self.map_bake = self.map_bake.toggled(),
        }
    }

    /// The popup rect: centered, sized to the eight rows.
    pub fn popup_rect(area: Rect) -> Rect {
        let w = area.width.min(36);
        let h = 10.min(area.height);
        Rect {
            x: area.x + area.width.saturating_sub(w) / 2,
            y: area.y + area.height.saturating_sub(h) / 2,
            width: w,
            height: h,
        }
    }

    /// The drawn popup: [`Self::popup_rect`] grown down (and widened when
    /// its text needs it) for the memory note and the save notice under
    /// the rows. The rows keep their place, so a click still lands on the
    /// row it points at.
    fn drawn_rect(area: Rect, notice: Option<&FormNotice>, memory_note: Option<&str>) -> Rect {
        let rows = Self::popup_rect(area);
        let (text, extra) = match notice {
            Some(notice) => match notice.error() {
                Some(reason) => (reason, NOTHING_SAVED),
                None => (notice.text(), ""),
            },
            None => ("", ""),
        };
        let mut wide = Span::raw(text).width().max(Span::raw(extra).width());
        if let Some(note) = memory_note {
            wide = wide.max(Span::raw(note).width());
        }
        if wide == 0 {
            return rows;
        }
        let width = u16::try_from(wide + 2)
            .unwrap_or(u16::MAX)
            .clamp(rows.width, area.width);
        let inner = usize::from(width.saturating_sub(2)).max(1);
        let mut lines = wrapped_rows(text, inner) + wrapped_rows(extra, inner);
        if let Some(note) = memory_note {
            lines += wrapped_rows(note, inner);
        }
        Rect {
            x: area.x + (area.width - width) / 2,
            y: rows.y,
            width,
            height: (rows.height + lines).min(area.y + area.height - rows.y),
        }
    }
}

impl Widget for SettingsPane<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if !self.state.open {
            return;
        }
        let memory_note;
        let memory_value = match self.memory.filter(|n| n.differs()) {
            Some(n) => {
                let login = if n.login_lowmem { "lowmem" } else { "highmem" };
                memory_note = Some(if n.relog_pending {
                    format!("server: {login} — relog queued, login follows once parked")
                } else {
                    format!("server: {login} — tabs/sound at next login (r = relog now)")
                });
                if self.settings.lowmem {
                    format!("lowmem (login {login})")
                } else {
                    format!("highmem (login {login})")
                }
            }
            None => {
                memory_note = None;
                if self.settings.lowmem {
                    "lowmem".to_string()
                } else {
                    "highmem".to_string()
                }
            }
        };
        let popup = Self::drawn_rect(area, self.notice, memory_note.as_deref());
        Clear.render(popup, buf);
        let block = Block::default().borders(Borders::ALL).title(self.title);
        let inner = block.inner(popup);
        block.render(popup, buf);
        let rows = [
            ("random events", format!("{}", self.settings.random_events)),
            ("lamp skill", self.settings.lamp_skill.clone()),
            ("lamp auto", format!("{}", self.settings.lamp_auto)),
            ("allow teleports", format!("{}", self.nav.allow_teleports)),
            ("allow wilderness", format!("{}", self.nav.allow_wilderness)),
            ("bank fetch", format!("{}", self.nav.allow_bank_fetch)),
            ("map bake", self.map_bake.as_str().to_string()),
            ("memory", memory_value),
        ];
        let mut lines: Vec<Line> = rows
            .iter()
            .enumerate()
            .map(|(i, (name, value))| {
                let marker = if i == self.state.row { "> " } else { "  " };
                Line::from(format!("{marker}{name}: {value}"))
            })
            .collect();
        if let Some(note) = memory_note.as_deref() {
            lines.push(Line::styled(note, Style::default().fg(Color::Yellow)));
        }
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .render(inner, buf);
        if let Some(notice) = self.notice {
            let bottom = inner.y + inner.height;
            let mut y = inner.y
                + ROWS
                + memory_note
                    .as_deref()
                    .map_or(0, |note| wrapped_rows(note, usize::from(inner.width).max(1)));
            match notice.error() {
                Some(reason) => {
                    let red = Style::default().fg(Color::Red);
                    y = draw_wrapped(buf, inner, y, bottom, reason, red);
                    draw_wrapped(buf, inner, y, bottom, NOTHING_SAVED, red);
                }
                None => {
                    let green = Style::default().fg(Color::Green);
                    draw_wrapped(buf, inner, y, bottom, notice.text(), green);
                }
            }
        }
    }
}

/// The popup's rows: random events, lamp skill, lamp auto, three nav
/// opt-ins and the map-bake choice. The notice starts under them.
const ROWS: u16 = 7;

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

    use frontend_core::MapBakeChoice;
    use frontend_core::MemoryNotice;
    use vault::ProfileSettings;

    use crate::app::NavFindSettings;

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
        let mut nav = NavFindSettings::default();
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
        let mut nav = NavFindSettings::default();
        let mut bake = MapBakeChoice::Ask;
        let mut state = SettingsState { open: true, row: 1 };
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
    fn popup_toggles_bank_fetch_without_marking_profile_dirty() {
        let mut settings = ProfileSettings::default();
        let mut nav = NavFindSettings::default();
        let mut bake = MapBakeChoice::Ask;
        let mut state = SettingsState { open: true, row: 5 };
        let key = {
            let mut pane = SettingsPane::new(&mut settings, &mut nav, &mut bake, &mut state);
            pane.on_key(key(KeyCode::Enter))
        };
        assert_eq!(key, SettingsKey::Consumed, "nav rows are session-only");
        assert!(nav.allow_bank_fetch, "bank fetch toggles on");
    }

    #[test]
    fn up_and_down_move_the_row_and_esc_closes() {
        let mut settings = ProfileSettings::default();
        let mut nav = NavFindSettings::default();
        let mut bake = MapBakeChoice::Ask;
        let mut state = SettingsState { open: true, row: 0 };
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
        let mut nav = NavFindSettings::default();
        let mut bake = MapBakeChoice::Ask;
        let mut state = SettingsState { open: true, row: 0 };
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
    fn memory_row_toggles_lowmem_and_reports_the_change() {
        let mut settings = ProfileSettings::default();
        let mut nav = NavFindSettings::default();
        let mut bake = MapBakeChoice::Ask;
        let mut state = SettingsState { open: true, row: 7 };
        assert!(settings.lowmem, "profiles start lowmem");
        let outcome = {
            let mut pane = SettingsPane::new(&mut settings, &mut nav, &mut bake, &mut state);
            pane.on_key(key(KeyCode::Enter))
        };
        assert_eq!(outcome, SettingsKey::Changed, "the binary must persist it");
        assert!(!settings.lowmem, "Enter on the memory row flips to highmem");
    }

    #[test]
    fn r_key_requests_a_memory_relog_from_any_row() {
        let mut settings = ProfileSettings::default();
        let mut nav = NavFindSettings::default();
        let mut bake = MapBakeChoice::Ask;
        let mut state = SettingsState { open: true, row: 0 };
        let outcome = {
            let mut pane = SettingsPane::new(&mut settings, &mut nav, &mut bake, &mut state);
            pane.on_key(key(KeyCode::Char('r')))
        };
        assert_eq!(outcome, SettingsKey::MemoryRelog);
        assert!(settings.lowmem, "r never flips the setting itself");
    }

    #[test]
    fn popup_shows_the_login_mode_and_relog_hint_while_it_differs() {
        let mut settings = ProfileSettings {
            lowmem: false,
            ..ProfileSettings::default()
        };
        let mut nav = NavFindSettings::default();
        let mut bake = MapBakeChoice::Ask;
        let mut state = SettingsState { open: true, row: 7 };
        let mut pane = SettingsPane::new(&mut settings, &mut nav, &mut bake, &mut state);
        pane.memory = Some(MemoryNotice {
            login_lowmem: true,
            desired_lowmem: false,
            relog_pending: false,
        });
        let text = render(pane, 60, 14);
        assert!(text.contains("memory: highmem (login lowmem)"), "{text:?}");
        assert!(text.contains("r = relog now"), "{text:?}");
    }

    #[test]
    fn popup_shows_a_queued_relog() {
        let mut settings = ProfileSettings {
            lowmem: false,
            ..ProfileSettings::default()
        };
        let mut nav = NavFindSettings::default();
        let mut bake = MapBakeChoice::Ask;
        let mut state = SettingsState { open: true, row: 7 };
        let mut pane = SettingsPane::new(&mut settings, &mut nav, &mut bake, &mut state);
        pane.memory = Some(MemoryNotice {
            login_lowmem: true,
            desired_lowmem: false,
            relog_pending: true,
        });
        let text = render(pane, 60, 14);
        assert!(text.contains("relog queued"), "{text:?}");
    }

    #[test]
    fn map_bake_row_flips_the_shared_choice_and_asks_the_binary_to_persist_it() {
        let mut settings = ProfileSettings::default();
        let mut nav = NavFindSettings::default();
        let mut bake = MapBakeChoice::Ask;
        let mut state = SettingsState { open: true, row: 5 };
        let (moved, flipped) = {
            let mut pane = SettingsPane::new(&mut settings, &mut nav, &mut bake, &mut state);
            let moved = pane.on_key(key(KeyCode::Down));
            (moved, pane.on_key(key(KeyCode::Enter)))
        };
        assert_eq!(moved, SettingsKey::Consumed);
        assert_eq!(state.row, 6, "the map-bake row is last");
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
