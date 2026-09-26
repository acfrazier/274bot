//! Log pane (F7): the shared `frontend_core::log` store with the same
//! level/source/scope filters, text search and follow as the panel, plus
//! Save log and the shared session-file preference. Drawn straight into
//! the frame buffer so a frame allocates nothing per row; fits 80×24.

use std::io::Write as _;

use crossterm::event::{KeyCode, KeyEvent};
use frontend_core::log::{
    default_save_path, global, save_text, Level, LogScope, LogView, SaveTicket, Source,
};
use frontend_core::log_file::{apply_session_log, persist_session_log_setting};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, Clear, Widget};

/// Which rows the pane reads; `Bot` follows the focused bot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneScope {
    Bot,
    Process,
    All,
}

impl PaneScope {
    fn next(self) -> Self {
        match self {
            PaneScope::Bot => PaneScope::Process,
            PaneScope::Process => PaneScope::All,
            PaneScope::All => PaneScope::Bot,
        }
    }
}

pub struct LogPaneState {
    pub open: bool,
    pub view: LogView,
    pub scope: PaneScope,
    /// The search line is being edited (keys type into it).
    pub editing: bool,
    pub search: String,
    /// Rows scrolled up from the newest while not following.
    pub scroll: usize,
    /// Last Save / session-file outcome.
    pub status: Option<String>,
    save: Option<SaveTicket>,
}

impl Default for LogPaneState {
    fn default() -> Self {
        Self {
            open: false,
            view: LogView::new(LogScope::Process),
            scope: PaneScope::Bot,
            editing: false,
            search: String::new(),
            scroll: 0,
            status: None,
            save: None,
        }
    }
}

fn next_level(level: Level) -> Level {
    match level {
        Level::Debug => Level::Info,
        Level::Info => Level::Warn,
        Level::Warn => Level::Error,
        Level::Error => Level::Debug,
    }
}

fn next_source(source: Option<Source>) -> Option<Source> {
    match source {
        None => Some(Source::ALL[0]),
        Some(s) => Source::ALL
            .iter()
            .position(|x| *x == s)
            .and_then(|i| Source::ALL.get(i + 1).copied()),
    }
}

impl LogPaneState {
    /// Point the view at the focused bot and pull new lines (one atomic
    /// load when nothing changed).
    pub fn refresh(&mut self, focused: Option<&str>) {
        match self.scope {
            PaneScope::Bot => self.view.follow_slot(focused),
            PaneScope::Process => self.view.set_scope(LogScope::Process),
            PaneScope::All => self.view.set_scope(LogScope::All),
        }
        let newest = self.view.rows().back().map(|e| e.seq);
        if global().refresh(&mut self.view) && !self.view.follow {
            // Keep a paused view on the rows it shows.
            let added = self
                .view
                .rows()
                .iter()
                .rev()
                .take_while(|e| newest.is_some_and(|seq| e.seq > seq))
                .count();
            self.scroll = (self.scroll + added).min(self.view.len().saturating_sub(1));
        }
        if let Some(ticket) = &self.save {
            if let Some(result) = ticket.poll() {
                self.save = None;
                self.status = Some(match result {
                    Ok(path) => format!("saved {}", path.display()),
                    Err(e) => e,
                });
            }
        }
        if self.view.follow {
            self.scroll = 0;
        }
    }

    /// Keys while the pane is open; Esc/F7 closes it.
    pub fn on_key(&mut self, key: KeyEvent, focused: Option<&str>) {
        if self.editing {
            match key.code {
                KeyCode::Enter | KeyCode::Esc => self.editing = false,
                KeyCode::Backspace => {
                    self.search.pop();
                    self.apply_search();
                }
                KeyCode::Char(c) => {
                    self.search.push(c);
                    self.apply_search();
                }
                _ => {}
            }
            return;
        }
        match key.code {
            KeyCode::Esc | KeyCode::F(7) => self.open = false,
            KeyCode::Char('/') => self.editing = true,
            KeyCode::Char('v') => {
                let next = next_level(self.view.filter().min_level);
                self.view.edit_filter(|f| f.min_level = next);
            }
            KeyCode::Char('s') => {
                let next = next_source(self.view.filter().single_source());
                self.view.edit_filter(|f| f.set_source(next));
            }
            KeyCode::Char('b') => self.scope = self.scope.next(),
            KeyCode::Char('f') => {
                self.view.follow = !self.view.follow;
                self.scroll = 0;
            }
            KeyCode::Up | KeyCode::Char('k') => self.scroll_up(1),
            KeyCode::PageUp => self.scroll_up(10),
            KeyCode::Down | KeyCode::Char('j') => self.scroll_down(1),
            KeyCode::PageDown => self.scroll_down(10),
            KeyCode::Home => self.scroll_up(usize::MAX / 2),
            KeyCode::End => {
                self.scroll = 0;
                self.view.follow = true;
            }
            KeyCode::Char('w') => {
                let label = match self.scope {
                    PaneScope::Bot => focused.unwrap_or("process"),
                    PaneScope::Process => "process",
                    PaneScope::All => "all",
                };
                self.save = Some(save_text(default_save_path(label), self.view.to_text()));
                self.status = Some("saving…".into());
            }
            KeyCode::Char('F') => {
                let on = global().file_path().is_none();
                self.status = Some(match persist_session_log_setting(on) {
                    Ok(()) => match apply_session_log(on) {
                        Some(path) => format!("session file {}", path.display()),
                        None => "session file off".into(),
                    },
                    Err(e) => format!("session file setting: {e}"),
                });
            }
            _ => {}
        }
    }

    fn apply_search(&mut self) {
        let text = self.search.clone();
        self.view.edit_filter(|f| f.text = text);
        self.scroll = 0;
    }

    fn scroll_up(&mut self, rows: usize) {
        self.view.follow = false;
        self.scroll = self
            .scroll
            .saturating_add(rows)
            .min(self.view.len().saturating_sub(1));
    }

    fn scroll_down(&mut self, rows: usize) {
        self.scroll = self.scroll.saturating_sub(rows);
        if self.scroll == 0 {
            self.view.follow = true;
        }
    }
}

fn level_style(level: Level) -> Style {
    match level {
        Level::Debug => Style::default().fg(Color::DarkGray),
        Level::Info => Style::default(),
        Level::Warn => Style::default().fg(Color::Yellow),
        Level::Error => Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
    }
}

/// Draws the pane over `area` (call after the main panes).
pub struct LogPane<'a> {
    pub state: &'a LogPaneState,
    pub focused: Option<&'a str>,
}

impl Widget for LogPane<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let state = self.state;
        Clear.render(area, buf);
        let scope = match state.scope {
            PaneScope::Bot => self.focused.unwrap_or("process"),
            PaneScope::Process => "process",
            PaneScope::All => "all bots",
        };
        let block = Block::default().borders(Borders::ALL).title(" Log ");
        let inner = block.inner(area);
        block.render(area, buf);
        if inner.height < 3 || inner.width < 20 {
            return;
        }
        let dim = Style::default().fg(Color::DarkGray);
        let accent = Style::default().fg(Color::Yellow);
        // Header: scope, level, source, follow, dropped.
        let mut x = inner.x;
        let right = inner.x + inner.width;
        let mut put = |x: &mut u16, y: u16, text: &str, style: Style| {
            if *x < right {
                let (nx, _) = buf.set_stringn(*x, y, text, (right - *x) as usize, style);
                *x = nx;
            }
        };
        let y = inner.y;
        put(&mut x, y, scope, accent);
        put(&mut x, y, " ≥", dim);
        put(
            &mut x,
            y,
            state.view.filter().min_level.label(),
            Style::default(),
        );
        put(&mut x, y, " src:", dim);
        put(
            &mut x,
            y,
            state
                .view
                .filter()
                .single_source()
                .map_or("all", Source::label),
            Style::default(),
        );
        put(
            &mut x,
            y,
            if state.view.follow {
                " follow"
            } else {
                " paused"
            },
            dim,
        );
        if state.view.dropped() > 0 {
            put(&mut x, y, " +rolled off", dim);
        }
        if global().file_path().is_some() {
            put(&mut x, y, " file", dim);
        }
        // Search line.
        let y = inner.y + 1;
        let mut x = inner.x;
        put(&mut x, y, "/", if state.editing { accent } else { dim });
        put(&mut x, y, &state.search, Style::default());
        if state.editing {
            put(&mut x, y, "_", accent);
        }
        if let Some(status) = &state.status {
            let mut sx = x.saturating_add(2).max(inner.x + inner.width / 2);
            put(&mut sx, y, status, dim);
        }
        // Rows, newest at the bottom, minus the help line.
        let rows_top = inner.y + 2;
        let rows_h = inner.height.saturating_sub(3) as usize;
        let rows = state.view.rows();
        let end = rows.len().saturating_sub(state.scroll);
        let start = end.saturating_sub(rows_h);
        let all = matches!(state.view.scope(), LogScope::All);
        // Wide terminals also get the game-tick column.
        let ticks = inner.width >= 100;
        let mut tick = [0u8; 16];
        for (i, entry) in rows.range(start..end).enumerate() {
            let y = rows_top + i as u16;
            let mut x = inner.x;
            put(&mut x, y, entry.clock.as_str(), dim);
            x += 1;
            if ticks {
                let mut cursor = std::io::Cursor::new(&mut tick[..]);
                let _ = match entry.tick {
                    Some(t) => write!(cursor, "t{t:<7}"),
                    None => write!(cursor, "t-      "),
                };
                let len = cursor.position() as usize;
                put(
                    &mut x,
                    y,
                    std::str::from_utf8(&tick[..len]).unwrap_or(""),
                    dim,
                );
                x += 1;
            }
            if all {
                put(&mut x, y, entry.slot.as_deref().unwrap_or("*"), accent);
                x += 1;
            }
            put(&mut x, y, entry.source.label(), dim);
            x += 1;
            put(&mut x, y, &entry.message, level_style(entry.level));
        }
        if rows.is_empty() {
            put(&mut inner.x.clone(), rows_top, "(no lines match)", dim);
        }
        let help_y = inner.y + inner.height - 1;
        let mut x = inner.x;
        put(
            &mut x,
            help_y,
            "Esc close / search v level s source b scope f follow ↑↓ scroll w save F file",
            dim,
        );
    }
}

#[cfg(test)]
#[path = "log_pane_tests.rs"]
mod tests;
