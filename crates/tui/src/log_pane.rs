//! Log view (the Logs tab, F7, and the log drawer): the shared
//! `frontend_core::log` store with the same level/source/scope filters,
//! text search and follow as the panel, plus Save log and the shared
//! session-file preference. Drawn straight into the frame buffer so a frame
//! allocates nothing per row; fits 80×24. The router owns Esc: a key the
//! view does not use is reported unconsumed.

use std::io::Write as _;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use frontend_core::log::{
    default_save_path, global, save_text, Level, LogEntry, LogScope, LogView, SaveTicket, Source,
};
use frontend_core::log_file::{apply_session_log, persist_session_log_setting};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, Clear, Widget};

/// Which rows the view reads; `Bot` follows the selected bot.
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
    /// Newest sequence the operator has seen on the Logs tab; newer rows
    /// count as unread in the compact drawer.
    seen: u64,
}

impl Default for LogPaneState {
    fn default() -> Self {
        Self {
            view: LogView::new(LogScope::Process),
            scope: PaneScope::Bot,
            editing: false,
            search: String::new(),
            scroll: 0,
            status: None,
            save: None,
            seen: 0,
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
    /// Point the view at the selected bot and pull new lines (one atomic
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

    /// The operator is looking at the log: nothing shown is unread.
    pub fn mark_seen(&mut self) {
        if let Some(entry) = self.view.rows().back() {
            self.seen = self.seen.max(entry.seq);
        }
    }

    /// Rows newer than the last look: `(all, warnings and errors)`.
    pub fn unread(&self) -> (usize, usize) {
        let mut all = 0;
        let mut loud = 0;
        for entry in self.view.rows().iter().rev() {
            if entry.seq <= self.seen {
                break;
            }
            all += 1;
            loud += usize::from(entry.level >= Level::Warn);
        }
        (all, loud)
    }

    pub fn newest(&self) -> Option<&LogEntry> {
        self.view.rows().back()
    }

    /// One key while the view has keyboard focus. Returns whether the view
    /// used it; Esc outside the search line is left to the router.
    pub fn on_key(&mut self, key: KeyEvent, focused: Option<&str>) -> bool {
        let text = !key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER);
        if self.editing {
            match key.code {
                KeyCode::Enter | KeyCode::Esc => self.editing = false,
                KeyCode::Backspace => {
                    self.search.pop();
                    self.apply_search();
                }
                KeyCode::Char(c) if text => {
                    self.search.push(c);
                    self.apply_search();
                }
                _ => return false,
            }
            return true;
        }
        match key.code {
            KeyCode::Char('/') if text => self.editing = true,
            KeyCode::Char('v') if text => {
                let next = next_level(self.view.filter().min_level);
                self.view.edit_filter(|f| f.min_level = next);
            }
            KeyCode::Char('s') if text => {
                let next = next_source(self.view.filter().single_source());
                self.view.edit_filter(|f| f.set_source(next));
            }
            KeyCode::Char('b') if text => self.scope = self.scope.next(),
            KeyCode::Char('f') if text => {
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
            KeyCode::Char('w') if text => self.save(focused),
            KeyCode::Char('F') if text => self.toggle_session_file(),
            _ => return false,
        }
        true
    }

    /// Save the rows the view shows (off the UI thread).
    pub fn save(&mut self, focused: Option<&str>) {
        let label = match self.scope {
            PaneScope::Bot => focused.unwrap_or("process"),
            PaneScope::Process => "process",
            PaneScope::All => "all",
        };
        self.save = Some(save_text(default_save_path(label), self.view.to_text()));
        self.status = Some("saving…".into());
    }

    /// Flip the shared session-file preference and apply it.
    pub fn toggle_session_file(&mut self) {
        let on = !global().file_open();
        self.status = Some(match persist_session_log_setting(on) {
            Ok(()) => match apply_session_log(on) {
                Some(path) => format!("session file {}", path.display()),
                None => "session file off".into(),
            },
            Err(e) => format!("session file setting: {e}"),
        });
    }

    /// Wheel or keyboard scroll by `rows` (negative is up, toward older).
    pub fn scroll_by(&mut self, rows: isize) {
        if rows < 0 {
            self.scroll_up(rows.unsigned_abs());
        } else {
            self.scroll_down(rows.unsigned_abs());
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

    /// Scope, level, source and follow as one short label.
    pub fn header(&self, focused: Option<&str>) -> String {
        let scope = match self.scope {
            PaneScope::Bot => focused.unwrap_or("process"),
            PaneScope::Process => "process",
            PaneScope::All => "all bots",
        };
        format!(
            "{scope} ≥{} src:{} {}",
            self.view.filter().min_level.label(),
            self.view
                .filter()
                .single_source()
                .map_or("all", Source::label),
            if self.view.follow { "follow" } else { "paused" }
        )
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

fn put(buf: &mut Buffer, right: u16, x: &mut u16, y: u16, text: &str, style: Style) {
    if *x < right {
        let (nx, _) = buf.set_stringn(*x, y, text, (right - *x) as usize, style);
        *x = nx;
    }
}

/// Draw the view's rows into `area`, newest at the bottom, `scroll` rows up
/// from the newest. Wide areas get the game-tick column; the all-bots
/// scope gets the slot column.
pub fn render_rows(state: &LogPaneState, area: Rect, buf: &mut Buffer, scroll: usize) {
    let dim = Style::default().fg(Color::DarkGray);
    let accent = Style::default().fg(Color::Yellow);
    let right = area.x + area.width;
    let rows = state.view.rows();
    let end = rows.len().saturating_sub(scroll);
    let start = end.saturating_sub(usize::from(area.height));
    let all = matches!(state.view.scope(), LogScope::All);
    let ticks = area.width >= 100;
    let mut tick = [0u8; 16];
    for (i, entry) in rows.range(start..end).enumerate() {
        let y = area.y + i as u16;
        let mut x = area.x;
        put(buf, right, &mut x, y, entry.clock.as_str(), dim);
        x += 1;
        if ticks {
            let mut cursor = std::io::Cursor::new(&mut tick[..]);
            let _ = match entry.tick {
                Some(t) => write!(cursor, "t{t:<7}"),
                None => write!(cursor, "t-      "),
            };
            let len = cursor.position() as usize;
            put(
                buf,
                right,
                &mut x,
                y,
                std::str::from_utf8(&tick[..len]).unwrap_or(""),
                dim,
            );
            x += 1;
        }
        if all {
            put(
                buf,
                right,
                &mut x,
                y,
                entry.slot.as_deref().unwrap_or("*"),
                accent,
            );
            x += 1;
        }
        put(buf, right, &mut x, y, entry.source.label(), dim);
        x += 1;
        put(
            buf,
            right,
            &mut x,
            y,
            &entry.message,
            level_style(entry.level),
        );
    }
    if rows.is_empty() && area.height > 0 {
        let mut x = area.x;
        put(buf, right, &mut x, area.y, "(no lines match)", dim);
    }
}

/// The full log view (the Logs tab). Returns nothing; the caller records
/// [`LogPane::rows_area`] for wheel hits.
pub struct LogPane<'a> {
    pub state: &'a LogPaneState,
    pub focused: Option<&'a str>,
}

impl LogPane<'_> {
    /// Where the rows land inside `area` (below header and search, above
    /// the help line).
    pub fn rows_area(area: Rect) -> Rect {
        let inner = Block::default().borders(Borders::ALL).inner(area);
        Rect::new(
            inner.x,
            inner.y + 2,
            inner.width,
            inner.height.saturating_sub(3),
        )
    }
}

impl Widget for LogPane<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let state = self.state;
        Clear.render(area, buf);
        let block = Block::default().borders(Borders::ALL).title(" Log ");
        let inner = block.inner(area);
        block.render(area, buf);
        if inner.height < 3 || inner.width < 20 {
            return;
        }
        let dim = Style::default().fg(Color::DarkGray);
        let accent = Style::default().fg(Color::Yellow);
        let right = inner.x + inner.width;
        // Header: scope, level, source, follow, dropped.
        let mut x = inner.x;
        let y = inner.y;
        put(buf, right, &mut x, y, &state.header(self.focused), accent);
        if state.view.dropped() > 0 {
            put(buf, right, &mut x, y, " +rolled off", dim);
        }
        if global().file_open() {
            put(buf, right, &mut x, y, " file", dim);
        }
        // Search line.
        let y = inner.y + 1;
        let mut x = inner.x;
        put(
            buf,
            right,
            &mut x,
            y,
            "/",
            if state.editing { accent } else { dim },
        );
        put(buf, right, &mut x, y, &state.search, Style::default());
        if state.editing {
            put(buf, right, &mut x, y, "_", accent);
        }
        if let Some(status) = &state.status {
            let mut sx = x.saturating_add(2).max(inner.x + inner.width / 2);
            put(buf, right, &mut sx, y, status, dim);
        }
        render_rows(state, Self::rows_area(area), buf, state.scroll);
        let help_y = inner.y + inner.height - 1;
        let mut x = inner.x;
        put(
            buf,
            right,
            &mut x,
            help_y,
            "/ search v level s source b scope f follow ↑↓ scroll w save F file",
            dim,
        );
    }
}

#[cfg(test)]
#[path = "log_pane_tests.rs"]
mod tests;
