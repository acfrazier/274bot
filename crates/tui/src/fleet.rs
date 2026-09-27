//! Fleet table: the loaded bots with a keyboard cursor, explicit row
//! selection (checkboxes, "selected N of M") and a filter. Moving the
//! cursor never changes the selected bot (the core `select`); Enter or a
//! click on a row does. Rows are drawn only for the visible window and the
//! filter scans member metadata without allocating, so a 1,000-member fleet
//! costs O(members) per frame and nothing at all while headless.

use std::collections::BTreeSet;
use std::fmt::Write as _;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};

use frontend_core::views::run_state_label;
use frontend_core::{FleetRow, Phase};
use script::RunState;

use crate::layout::SizeClass;

/// Member `index`'s row in the core's fleet projection. Rows follow load
/// order, so the common case is one comparison, not a scan; `None` for a
/// member the core has not projected yet.
pub fn member_row<'a>(
    names: &[String],
    rows: &'a [FleetRow],
    index: usize,
) -> Option<&'a FleetRow> {
    let name = names.get(index)?;
    match rows.get(index) {
        Some(row) if row.name == *name => Some(row),
        _ => rows.iter().find(|row| row.name == *name),
    }
}

/// The row's status label (the one the panel's caps show); a member not
/// projected yet is offline.
pub fn state_label(row: Option<&FleetRow>) -> &str {
    row.map_or(Phase::Offline.label(), |row| row.brief.as_str())
}

/// ASCII case-insensitive substring test without allocating.
fn contains_ci(hay: &str, needle: &str) -> bool {
    let (hay, needle) = (hay.as_bytes(), needle.as_bytes());
    needle.is_empty()
        || hay
            .windows(needle.len())
            .any(|window| window.eq_ignore_ascii_case(needle))
}

/// Whether one filter term matches: a name substring, `wN` / `world:N`,
/// or a status substring: the phase (`ready`, `queued`, `login error`, …)
/// or the row's label (`running`, `idle`, …).
fn term_matches(name: &str, row: Option<&FleetRow>, term: &str) -> bool {
    let phase = row.map_or(Phase::Offline, |row| row.phase);
    if contains_ci(name, term)
        || contains_ci(phase.label(), term)
        || contains_ci(state_label(row), term)
    {
        return true;
    }
    let bytes = term.as_bytes();
    let number = if bytes.len() > 6 && bytes[..6].eq_ignore_ascii_case(b"world:") {
        &term[6..]
    } else if bytes.len() > 1 && bytes[0].eq_ignore_ascii_case(&b'w') {
        &term[1..]
    } else {
        return false;
    };
    number
        .parse::<u16>()
        .is_ok_and(|world| row.and_then(|row| row.world) == Some(world))
}

/// Every whitespace-separated filter term must match.
pub fn row_matches(name: &str, row: Option<&FleetRow>, filter: &str) -> bool {
    filter
        .split_whitespace()
        .all(|term| term_matches(name, row, term))
}

/// Renderer-local fleet state. Marks are the explicit row selection
/// ("selected N of M"); they never change the selected bot and are dropped
/// when their member leaves the fleet.
#[derive(Debug, Default)]
pub struct FleetState {
    /// Cursor position in the shown (filtered) rows.
    pub cursor: usize,
    /// First shown row of the drawn window.
    pub scroll: usize,
    pub marks: BTreeSet<String>,
    pub filter: String,
    /// The filter line is being edited.
    pub editing: bool,
    /// Member indices passing the filter, in fleet order.
    shown: Vec<usize>,
    /// The member under the cursor, so a filter or membership change keeps
    /// the cursor on the same bot while it is still shown.
    cursor_name: String,
}

impl FleetState {
    /// Recompute the shown rows for `names` (their projected `rows`), keep
    /// the cursor on the same member when possible and drop marks of
    /// departed members. Reuses its buffers: no allocation in steady state.
    pub fn sync(&mut self, names: &[String], rows: &[FleetRow]) {
        self.shown.clear();
        for (index, name) in names.iter().enumerate() {
            if self.filter.is_empty()
                || row_matches(name, member_row(names, rows, index), &self.filter)
            {
                self.shown.push(index);
            }
        }
        if !self.cursor_name.is_empty() {
            if let Some(pos) = self
                .shown
                .iter()
                .position(|&index| names[index] == self.cursor_name)
            {
                self.cursor = pos;
            }
        }
        self.cursor = self.cursor.min(self.shown.len().saturating_sub(1));
        self.remember_cursor(names);
        if !self.marks.is_empty()
            && names
                .iter()
                .filter(|name| self.marks.contains(*name))
                .count()
                != self.marks.len()
        {
            self.marks.retain(|mark| names.contains(mark));
        }
    }

    fn remember_cursor(&mut self, names: &[String]) {
        self.cursor_name.clear();
        if let Some(name) = self.shown.get(self.cursor).and_then(|&i| names.get(i)) {
            self.cursor_name.push_str(name);
        }
    }

    pub fn shown(&self) -> &[usize] {
        &self.shown
    }

    /// The member index under the cursor.
    pub fn cursor_member(&self) -> Option<usize> {
        self.shown.get(self.cursor).copied()
    }

    /// Move the cursor by `delta` shown rows (clamped).
    pub fn move_cursor(&mut self, delta: isize, names: &[String]) {
        if self.shown.is_empty() {
            return;
        }
        let last = self.shown.len() - 1;
        self.cursor = self.cursor.saturating_add_signed(delta).min(last);
        self.remember_cursor(names);
    }

    pub fn cursor_to(&mut self, position: usize, names: &[String]) {
        if self.shown.is_empty() {
            return;
        }
        self.cursor = position.min(self.shown.len() - 1);
        self.remember_cursor(names);
    }

    /// Toggle the row selection of member `index`.
    pub fn toggle_mark(&mut self, names: &[String], index: usize) {
        let Some(name) = names.get(index) else {
            return;
        };
        if !self.marks.remove(name) {
            self.marks.insert(name.clone());
        }
    }

    pub fn is_marked(&self, name: &str) -> bool {
        self.marks.contains(name)
    }

    pub fn mark_all_shown(&mut self, names: &[String]) {
        for &index in &self.shown {
            if let Some(name) = names.get(index) {
                self.marks.insert(name.clone());
            }
        }
    }

    /// Keep the cursor inside a window of `rows` lines.
    fn scroll_into_view(&mut self, rows: usize) {
        if rows == 0 {
            return;
        }
        if self.cursor < self.scroll {
            self.scroll = self.cursor;
        } else if self.cursor >= self.scroll + rows {
            self.scroll = self.cursor + 1 - rows;
        }
        self.scroll = self.scroll.min(self.shown.len().saturating_sub(rows));
    }
}

/// The table widget over the fleet `names` and the core's projected `rows`.
/// `selected` is the selected bot's member index; `keys` is whether the
/// fleet pane has keyboard focus (the cursor row is drawn reversed only
/// then, so the operator sees where keys go).
pub struct FleetTable<'a> {
    pub names: &'a [String],
    pub rows: &'a [FleetRow],
    pub state: &'a mut FleetState,
    pub selected: Option<usize>,
    pub keys: bool,
    pub class: SizeClass,
}

/// What the table drew: its row area and the shown-row position on its
/// first line (mouse hits map back through these).
#[derive(Debug, Clone, Copy, Default)]
pub struct FleetHits {
    pub rows: Rect,
    pub first: usize,
}

/// Row prefix: `[x]` selection box, ` >` cursor, `*` selected bot.
pub const MARK_COLUMNS: u16 = 3;
const PREFIX: u16 = 6;
const WORLD: usize = 4;
const STATE: usize = 11;
const QUEUE: usize = 7;
/// Widest script state (`starting`, `stopping`).
const SCRIPT: usize = 8;

fn put(buf: &mut Buffer, x: &mut u16, y: u16, right: u16, text: &str, style: Style) {
    if *x < right {
        let (next, _) = buf.set_stringn(*x, y, text, usize::from(right - *x), style);
        *x = next;
    }
}

/// Append `text` cut or padded to exactly `width` columns. A cut name ends
/// in `~` so it is never mistaken for the whole identity.
fn push_fitted(out: &mut String, text: &str, width: usize) {
    let count = text.chars().count();
    if count <= width {
        out.push_str(text);
        out.extend(std::iter::repeat_n(' ', width - count));
    } else if width > 0 {
        out.extend(text.chars().take(width - 1));
        out.push('~');
    }
}

impl FleetTable<'_> {
    /// Draw into `area` (the pane's inner rect): the filter line while a
    /// filter is set or edited, the column header, the visible rows, then
    /// the count line. `reserve` rows at the bottom stay free for the
    /// caller's buttons.
    pub fn render(self, area: Rect, buf: &mut Buffer, reserve: u16) -> FleetHits {
        let Self {
            names,
            rows: fleet,
            state,
            selected,
            keys,
            class,
        } = self;
        state.sync(names, fleet);
        if area.height == 0 || area.width < 12 {
            return FleetHits::default();
        }
        let right = area.x + area.width;
        let dim = Style::default().add_modifier(Modifier::DIM);
        let bottom = area.y + area.height;
        let mut y = area.y;
        if state.editing || !state.filter.is_empty() {
            let mut x = area.x;
            put(buf, &mut x, y, right, "/filter: ", dim);
            put(buf, &mut x, y, right, &state.filter, Style::default());
            if state.editing {
                put(
                    buf,
                    &mut x,
                    y,
                    right,
                    "_",
                    Style::default().add_modifier(Modifier::BOLD),
                );
            }
            y += 1;
        }
        let wide = class == SizeClass::Large;
        let fixed = usize::from(PREFIX)
            + 1
            + WORLD
            + 1
            + STATE
            + if wide { 1 + QUEUE + 1 + SCRIPT } else { 0 };
        let name_w = usize::from(area.width).saturating_sub(fixed).max(4);
        let mut cell = String::with_capacity(usize::from(area.width) + 8);
        if y < bottom {
            cell.push_str("sel   ");
            push_fitted(&mut cell, "name", name_w);
            cell.push(' ');
            push_fitted(&mut cell, "wrld", WORLD);
            cell.push(' ');
            push_fitted(&mut cell, "state", STATE);
            if wide {
                cell.push(' ');
                push_fitted(&mut cell, "queue", QUEUE);
                cell.push_str(" script");
            }
            let mut x = area.x;
            put(buf, &mut x, y, right, &cell, dim);
            y += 1;
        }
        let rows_h = bottom.saturating_sub(y).saturating_sub(1 + reserve);
        state.scroll_into_view(usize::from(rows_h));
        let rows = Rect::new(area.x, y, area.width, rows_h);
        if rows_h > 0 && names.is_empty() {
            let mut x = area.x;
            put(
                buf,
                &mut x,
                y,
                right,
                "no bots loaded: m loads every vault profile",
                dim,
            );
        } else if rows_h > 0 && state.shown().is_empty() {
            let mut x = area.x;
            put(
                buf,
                &mut x,
                y,
                right,
                "no member matches (Esc clears the filter)",
                dim,
            );
        }
        for line in 0..rows_h {
            let position = state.scroll + usize::from(line);
            let Some(&member) = state.shown().get(position) else {
                break;
            };
            let name = names[member].as_str();
            let row = member_row(names, fleet, member);
            let is_cursor = position == state.cursor;
            let is_selected = selected == Some(member);
            let mut style = Style::default();
            if is_selected {
                style = style.add_modifier(Modifier::BOLD);
            }
            if is_cursor && keys {
                style = style.add_modifier(Modifier::REVERSED);
            }
            cell.clear();
            cell.push_str(if state.is_marked(name) { "[x]" } else { "[ ]" });
            cell.push_str(if is_cursor { " >" } else { "  " });
            cell.push(if is_selected { '*' } else { ' ' });
            push_fitted(&mut cell, name, name_w);
            cell.push(' ');
            let world_at = cell.len();
            match row.and_then(|row| row.world) {
                Some(world) => {
                    let _ = write!(cell, "w{world}");
                }
                None => cell.push_str("loc"),
            }
            let world_len = cell.len() - world_at;
            cell.extend(std::iter::repeat_n(' ', WORLD.saturating_sub(world_len)));
            cell.push(' ');
            push_fitted(&mut cell, state_label(row), STATE);
            if wide {
                cell.push(' ');
                let queue_at = cell.len();
                match row.and_then(|row| row.queue) {
                    Some(place) => {
                        let _ = write!(cell, "{}/{}", place.position, place.total);
                    }
                    None => cell.push('-'),
                }
                let queue_len = cell.len() - queue_at;
                cell.extend(std::iter::repeat_n(' ', QUEUE.saturating_sub(queue_len)));
                cell.push(' ');
                cell.push_str(run_state_label(
                    row.map_or(RunState::Idle, |row| row.script),
                ));
            }
            if is_cursor && keys {
                let used = cell.chars().count();
                cell.extend(std::iter::repeat_n(
                    ' ',
                    usize::from(area.width).saturating_sub(used),
                ));
            }
            let mut x = area.x;
            put(buf, &mut x, rows.y + line, right, &cell, style);
        }
        let count_y = rows.y + rows_h;
        if count_y < bottom {
            cell.clear();
            let _ = write!(
                cell,
                "{}/{} shown · selected {} of {}",
                state.shown().len(),
                names.len(),
                state.marks.len(),
                names.len()
            );
            // The selected bot also heads the detail pane; name it here only
            // when it fits whole.
            if let Some(name) = selected.and_then(|i| names.get(i)) {
                if cell.chars().count() + 7 + name.chars().count() <= usize::from(area.width) {
                    let _ = write!(cell, " · BOT {name}");
                }
            }
            let mut x = area.x;
            put(buf, &mut x, count_y, right, &cell, dim);
        }
        FleetHits {
            rows,
            first: state.scroll,
        }
    }
}

#[cfg(test)]
#[path = "fleet_tests.rs"]
mod tests;
