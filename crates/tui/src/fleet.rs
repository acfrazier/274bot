//! Fleet table: every profile the operator can load (the loaded bots and
//! the vault profiles that are not loaded), a keyboard cursor, explicit row
//! marks (checkboxes, "marked N") and a filter. The rows, marks, filter and
//! shown order live in the core `FleetModel`; this module keeps only the
//! presentation: the cursor, the scroll window and the edit flag. Moving the
//! cursor never changes the selected bot (the core `select`); Enter or a
//! click on a loaded row does. Rows are drawn only for the visible window,
//! so a 1,000-profile fleet costs O(window) per frame and nothing at all
//! while headless.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use std::collections::HashSet;
use std::fmt::Write as _;

use frontend_core::views::run_state_label;
use frontend_core::{
    state_label, FleetModel, FleetOrder, Placement, ProfileIdentity, ReorderOp, OWN_CARD_LABEL,
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

/// Between header parts that share a row.
const HEADER_SEP: &str = " · ";

/// The fleet header and what it was built from.
#[derive(Debug, Default)]
struct Header {
    /// The wrapped rows.
    lines: Vec<String>,
    card: String,
    order: Option<FleetOrder>,
    marked: usize,
    width: usize,
}

/// Lay the header `parts` out in rows of at most `width` cells: parts
/// share a row while they fit and move whole to the next row when they do
/// not; a part wider than a row wraps at its spaces (a word wider than a
/// row is cut).
fn wrap_header(parts: [String; 3], width: usize, lines: &mut Vec<String>) {
    lines.clear();
    let width = width.max(1);
    let sep = HEADER_SEP.chars().count();
    let mut line = String::new();
    let mut used = 0usize;
    for part in &parts {
        let part_width = part.chars().count();
        if used > 0 && used + sep + part_width <= width {
            line.push_str(HEADER_SEP);
            line.push_str(part);
            used += sep + part_width;
            continue;
        }
        if used > 0 {
            lines.push(std::mem::take(&mut line));
            used = 0;
        }
        if part_width <= width {
            line.push_str(part);
            used = part_width;
            continue;
        }
        for word in part.split(' ') {
            let mut word_width = word.chars().count();
            if used > 0 && used + 1 + word_width <= width {
                line.push(' ');
                line.push_str(word);
                used += 1 + word_width;
                continue;
            }
            if used > 0 {
                lines.push(std::mem::take(&mut line));
            }
            let mut rest = word;
            while word_width > width {
                let cut = rest
                    .char_indices()
                    .nth(width)
                    .map_or(rest.len(), |(at, _)| at);
                lines.push(rest[..cut].to_owned());
                rest = &rest[cut..];
                word_width -= width;
            }
            line.push_str(rest);
            used = word_width;
        }
    }
    if used > 0 {
        lines.push(line);
    }
}

/// Renderer-local fleet state: presentation only. The rows, marks, filter
/// and shown order are the core `FleetModel`'s; the cursor is a position in
/// its shown rows that follows its profile (by identity) when the rows or
/// the filter change.
#[derive(Debug, Default)]
pub struct FleetState {
    /// Cursor position in the shown (filtered) rows.
    pub cursor: usize,
    /// First shown row of the drawn window.
    pub scroll: usize,
    /// The filter line is being edited.
    pub editing: bool,
    /// The profile under the cursor, so a filter or membership change keeps
    /// the cursor on the same profile while it is still shown.
    cursor_identity: Option<ProfileIdentity>,
    /// The model generation the cursor was last fitted to.
    synced: Option<u64>,
    /// The cached header line and the inputs it was built from.
    header: Header,
}

impl FleetState {
    /// The header lines: `card: <card> · sort: <order> · N marked`,
    /// wrapped to `width` cells. They are rebuilt only when the card
    /// label, the order, the marked count or the width changed since the
    /// last draw, so a steady draw allocates nothing.
    fn header(&mut self, model: &FleetModel, width: usize) -> &[String] {
        let order = model.order();
        let marked = model.counts().marked();
        // Before the first per-loop `sync_card_label` the card is the
        // default one.
        let card = match model.card_label() {
            "" => OWN_CARD_LABEL,
            label => label,
        };
        let header = &mut self.header;
        if header.order != Some(order)
            || header.marked != marked
            || header.width != width
            || header.card != card
        {
            header.order = Some(order);
            header.marked = marked;
            header.width = width;
            header.card.clear();
            header.card.push_str(card);
            wrap_header(
                [
                    format!("card: {card}"),
                    format!("sort: {}", order.label()),
                    format!("{marked} marked"),
                ],
                width,
                &mut header.lines,
            );
        }
        &header.lines
    }

    /// Fit the cursor to `model`'s shown rows: keep it on the same profile
    /// when that is still shown, else clamp it. Does nothing while the model
    /// has not moved since the last call.
    pub fn sync(&mut self, model: &FleetModel) {
        if self.synced == Some(model.generation()) {
            return;
        }
        self.synced = Some(model.generation());
        if let Some(identity) = self.cursor_identity {
            if let Some(pos) = model
                .shown()
                .iter()
                .position(|&index| model.identity(index) == Some(identity))
            {
                self.cursor = pos;
            }
        }
        self.cursor = self.cursor.min(model.shown().len().saturating_sub(1));
        self.remember_cursor(model);
    }

    fn remember_cursor(&mut self, model: &FleetModel) {
        self.cursor_identity = model
            .shown()
            .get(self.cursor)
            .and_then(|&index| model.identity(index));
    }

    /// The universe row index under the cursor.
    pub fn cursor_row(&self, model: &FleetModel) -> Option<usize> {
        model.shown().get(self.cursor).copied()
    }

    /// Move the cursor by `delta` shown rows (clamped).
    pub fn move_cursor(&mut self, delta: isize, model: &FleetModel) {
        if model.shown().is_empty() {
            return;
        }
        let last = model.shown().len() - 1;
        self.cursor = self.cursor.saturating_add_signed(delta).min(last);
        self.remember_cursor(model);
    }

    pub fn cursor_to(&mut self, position: usize, model: &FleetModel) {
        if model.shown().is_empty() {
            return;
        }
        self.cursor = position.min(model.shown().len() - 1);
        self.remember_cursor(model);
    }

    /// The reorder that moves the cursor's profile one shown row up
    /// (`delta < 0`, lands before that row) or down (`delta > 0`, lands
    /// after it). `None` at the first or last shown row, or with no cursor
    /// row. The target is a shown row; hidden rows keep their place in the
    /// saved order (the core resolves that).
    pub fn reorder_step(&self, model: &FleetModel, delta: isize) -> Option<ReorderOp> {
        let shown = model.shown();
        let moving = model.identity(*shown.get(self.cursor)?)?;
        let neighbour = self.cursor.checked_add_signed(delta)?;
        let target = model.identity(*shown.get(neighbour)?)?;
        Some(ReorderOp::Move {
            moving,
            to: if delta < 0 {
                Placement::Before(target)
            } else {
                Placement::After(target)
            },
        })
    }

    /// Keep the cursor inside a window of `rows` lines over `shown` rows.
    fn scroll_into_view(&mut self, rows: usize, shown: usize) {
        if rows == 0 {
            return;
        }
        if self.cursor < self.scroll {
            self.scroll = self.cursor;
        } else if self.cursor >= self.scroll + rows {
            self.scroll = self.cursor + 1 - rows;
        }
        self.scroll = self.scroll.min(shown.saturating_sub(rows));
    }
}

/// The `SetMark` request that flips universe row `index`'s mark: the
/// profile's name and the value its mark takes. Any row (loaded or not) can
/// be marked.
pub fn toggled_mark(model: &FleetModel, index: usize) -> Option<(String, bool)> {
    let row = model.rows().get(index)?;
    Some((row.name().to_owned(), !model.is_marked(index)))
}

/// The table widget over the core's fleet model: its shown rows, loaded
/// and not. `selected` is the focused bot's identity (it gets the `*`
/// marker; an unloaded row is never focused); `keys` is whether the fleet
/// pane has keyboard focus (the cursor row is drawn reversed only then, so
/// the operator sees where keys go).
pub struct FleetTable<'a> {
    pub model: &'a FleetModel,
    pub restart_badges: &'a HashSet<ProfileIdentity>,
    pub state: &'a mut FleetState,
    pub selected: Option<ProfileIdentity>,
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
    /// Draw into `area` (the pane's inner rect): the card / sort / marks
    /// header, the filter line while a filter is set or edited, the column
    /// header, the visible rows, then the count line. `reserve` rows at the
    /// bottom stay free for the caller's buttons.
    pub fn render(self, area: Rect, buf: &mut Buffer, reserve: u16) -> FleetHits {
        let Self {
            model,
            state,
            selected,
            restart_badges,
            keys,
            class,
        } = self;
        state.sync(model);
        if area.height == 0 || area.width < 12 {
            return FleetHits::default();
        }
        let right = area.x + area.width;
        let dim = Style::default().add_modifier(Modifier::DIM);
        let bottom = area.y + area.height;
        let mut y = area.y;
        for line in state.header(model, usize::from(area.width)) {
            if y >= bottom {
                break;
            }
            let mut x = area.x;
            put(buf, &mut x, y, right, line, dim);
            y += 1;
        }
        let filter = model.filter();
        if state.editing || !filter.text().is_empty() {
            let mut x = area.x;
            put(buf, &mut x, y, right, "/filter: ", dim);
            // The field shows what is typed while it is edited, the
            // applied filter otherwise.
            let line = if state.editing {
                filter.staged()
            } else {
                filter.text()
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
        state.scroll_into_view(usize::from(rows_h), model.shown().len());
        let rows = Rect::new(area.x, y, area.width, rows_h);
        if rows_h > 0 && model.rows().is_empty() {
            let mut x = area.x;
            put(
                buf,
                &mut x,
                y,
                right,
                "no profiles: nothing to mark or log in",
                dim,
            );
        } else if rows_h > 0 && model.shown().is_empty() {
            let mut x = area.x;
            put(
                buf,
                &mut x,
                y,
                right,
                "no profile matches (Esc clears the filter)",
                dim,
            );
        }
        for line in 0..rows_h {
            let position = state.scroll + usize::from(line);
            let Some(&index) = model.shown().get(position) else {
                break;
            };
            let row = &model.rows()[index];
            let identity = model.identities()[index];
            let loaded = row.loaded();
            let is_cursor = position == state.cursor;
            let is_selected = loaded && selected == Some(identity);
            let mut style = Style::default();
            if !loaded {
                style = style.add_modifier(Modifier::DIM);
            }
            if is_selected {
                style = style.add_modifier(Modifier::BOLD);
            }
            if is_cursor && keys {
                style = style.add_modifier(Modifier::REVERSED);
            }
            cell.clear();
            cell.push_str(if model.is_marked(index) { "[x]" } else { "[ ]" });
            cell.push_str(if is_cursor { " >" } else { "  " });
            cell.push(if is_selected { '*' } else { ' ' });
            let restart_required = restart_badges.contains(&identity);
            push_fitted(
                &mut cell,
                row.name(),
                name_w.saturating_sub(usize::from(restart_required)),
            );
            if restart_required {
                cell.push('!');
            }
            cell.push(' ');
            let world_at = cell.len();
            match row.world() {
                _ if !loaded => cell.push('-'),
                Some(world) => {
                    let _ = write!(cell, "w{world}");
                }
                None => cell.push_str("loc"),
            }
            let world_len = cell.len() - world_at;
            cell.extend(std::iter::repeat_n(' ', WORLD.saturating_sub(world_len)));
            cell.push(' ');
            push_fitted(&mut cell, state_label(Some(row)), STATE);
            if wide {
                cell.push(' ');
                let queue_at = cell.len();
                match row.queue() {
                    Some(place) if loaded => {
                        let _ = write!(cell, "{}/{}", place.position, place.total);
                    }
                    _ => cell.push('-'),
                }
                let queue_len = cell.len() - queue_at;
                cell.extend(std::iter::repeat_n(' ', QUEUE.saturating_sub(queue_len)));
                cell.push(' ');
                cell.push_str(if loaded {
                    run_state_label(row.script())
                } else {
                    "-"
                });
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
            let counts = model.counts();
            let _ = write!(
                cell,
                "{}/{} shown · marked {}",
                counts.shown(),
                counts.rows(),
                counts.marked()
            );
            if counts.hidden_marked() > 0 {
                let _ = write!(cell, " ({} not shown)", counts.hidden_marked());
            }
            // The focused bot also heads the detail pane; name it here only
            // when it fits whole.
            if let Some(row) = selected
                .and_then(|identity| model.index_of(identity))
                .map(|index| &model.rows()[index])
            {
                let name = row.name();
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
