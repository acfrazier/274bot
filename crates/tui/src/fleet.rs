//! Fleet table: the loaded bots with a keyboard cursor, explicit row
//! selection (checkboxes, "selected N of M") and a filter. Moving the
//! cursor never changes the selected bot (the core `select`); Enter or a
//! click on a row does. Rows are drawn only for the visible window and the
//! filter scans member metadata without allocating, so a 1,000-member fleet
//! costs O(members) per frame and nothing at all while headless.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use std::collections::HashSet;
use std::fmt::Write as _;

use frontend_core::scripts::ScriptRunState;
use frontend_core::views::run_state_label;
use frontend_core::{
    fill_visible_indices, member_row, profile_id_at, state_label, FleetFilter, FleetOrder,
    FleetRow, MarkedSelection, ProfileIdentity,
};

use crate::layout::SizeClass;

/// One edit of the fleet filter field. The field reports it; the dispatcher
/// stages it on the core-owned filter text and commits it
/// (`SetFleetFilter`), so the shown rows follow the text as it is typed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterEdit {
    /// Type a character.
    Push(char),
    /// Delete the last character.
    Pop,
    /// Clear the text.
    Clear,
}

/// Renderer-local fleet state. Marks are the shared identity-based selection
/// ("selected N of M"); they never change the selected bot and are dropped
/// when their member leaves the fleet. The marks and the filter are
/// operator models the command seam changes (`SetMark`, `MarkAll`,
/// `ClearMarks`, `SetFleetFilter`); the cursor, the scroll window and the
/// edit flag are presentation only.
#[derive(Debug)]
pub struct FleetState {
    /// Cursor position in the shown (filtered) rows.
    pub cursor: usize,
    /// First shown row of the drawn window.
    pub scroll: usize,
    /// Shared checkbox state; identities are stable across renames.
    pub selection: MarkedSelection,
    /// The filter text and row order: the field edits the staged text and
    /// `SetFleetFilter` commits it to the filter the rows are shown with.
    pub filter: FleetFilter,
    /// The filter line is being edited.
    pub editing: bool,
    /// Member indices passing the filter, in fleet order.
    shown: Vec<usize>,
    /// The member under the cursor, so a filter or membership change keeps
    /// the cursor on the same bot while it is still shown.
    cursor_name: String,
}

impl Default for FleetState {
    fn default() -> Self {
        Self {
            cursor: 0,
            scroll: 0,
            selection: MarkedSelection::default(),
            filter: FleetFilter::new(FleetOrder::LoadOrder),
            editing: false,
            shown: Vec::new(),
            cursor_name: String::new(),
        }
    }
}

impl FleetState {
    /// Recompute the shown rows for `names` (their projected `rows`), keep
    /// the cursor on the same member when possible and drop marks of
    /// departed identities. `ids` parallels `names`; missing ids use a
    /// deterministic fixture identity.
    pub fn sync_with_ids(&mut self, names: &[String], ids: &[ProfileIdentity], rows: &[FleetRow]) {
        fill_visible_indices(
            names,
            rows,
            self.filter.text(),
            self.filter.order(),
            &mut self.shown,
        );
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
        let identities = (0..names.len()).filter_map(|index| profile_id_at(ids, names, index));
        self.selection.retain(identities);
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

    /// The `SetMark` request that flips member `index`'s row mark, using
    /// the stable id when the caller has a vault projection and a fixture
    /// id otherwise: the member's name and the value its mark takes.
    pub fn toggled_mark(
        &self,
        names: &[String],
        ids: &[ProfileIdentity],
        index: usize,
    ) -> Option<(String, bool)> {
        let name = names.get(index)?;
        let identity = profile_id_at(ids, names, index)?;
        Some((name.clone(), !self.selection.contains(identity)))
    }

    pub fn is_marked_id(&self, identity: ProfileIdentity) -> bool {
        self.selection.contains(identity)
    }

    /// The identities of the shown rows: the rows `MarkAll` marks for
    /// "select all shown".
    pub fn shown_identities(
        &self,
        names: &[String],
        ids: &[ProfileIdentity],
    ) -> Vec<ProfileIdentity> {
        self.shown
            .iter()
            .filter_map(|&index| profile_id_at(ids, names, index))
            .collect()
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
/// `ids` parallels `names` and keeps row marks stable through sorting/filter.
/// `selected` is the selected bot's member index; `keys` is whether the
/// fleet pane has keyboard focus (the cursor row is drawn reversed only
/// then, so the operator sees where keys go).
pub struct FleetTable<'a> {
    pub names: &'a [String],
    pub ids: &'a [ProfileIdentity],
    pub rows: &'a [FleetRow],
    pub restart_badges: &'a HashSet<ProfileIdentity>,
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
            ids,
            rows: fleet,
            state,
            selected,
            restart_badges,
            keys,
            class,
        } = self;
        state.sync_with_ids(names, ids, fleet);
        if area.height == 0 || area.width < 12 {
            return FleetHits::default();
        }
        let right = area.x + area.width;
        let dim = Style::default().add_modifier(Modifier::DIM);
        let bottom = area.y + area.height;
        let mut y = area.y;
        if state.editing || !state.filter.text().is_empty() {
            let mut x = area.x;
            put(buf, &mut x, y, right, "/filter: ", dim);
            // The field shows what is typed while it is edited, the
            // applied filter otherwise.
            let line = if state.editing {
                state.filter.staged()
            } else {
                state.filter.text()
            };
            put(buf, &mut x, y, right, line, Style::default());
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
            let identity = profile_id_at(ids, names, member).expect("fleet row identity");
            cell.push_str(if state.is_marked_id(identity) {
                "[x]"
            } else {
                "[ ]"
            });
            cell.push_str(if is_cursor { " >" } else { "  " });
            cell.push(if is_selected { '*' } else { ' ' });
            let restart_required = restart_badges.contains(&identity);
            push_fitted(
                &mut cell,
                name,
                name_w.saturating_sub(usize::from(restart_required)),
            );
            if restart_required {
                cell.push('!');
            }
            cell.push(' ');
            let world_at = cell.len();
            match row.and_then(|row| row.world()) {
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
                match row.and_then(|row| row.queue()) {
                    Some(place) => {
                        let _ = write!(cell, "{}/{}", place.position, place.total);
                    }
                    None => cell.push('-'),
                }
                let queue_len = cell.len() - queue_at;
                cell.extend(std::iter::repeat_n(' ', QUEUE.saturating_sub(queue_len)));
                cell.push(' ');
                cell.push_str(run_state_label(
                    row.map_or(ScriptRunState::Idle, |row| row.script()),
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
                state.selection.len(),
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
