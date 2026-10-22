//! The world picker: the bound server's public worlds, one row each, with
//! the bot's own world ticked. `Enter` (or a click) picks a world and the
//! dispatcher asks core for a Switch world plan, which the confirm overlay
//! then shows. `Esc` (or a click outside) leaves everything as it was.
//! Picking the world the bot is already on closes the picker and does
//! nothing: no plan, no write, no relog.
//!
//! The rows are built once when the picker opens, from the core's cached
//! world list (nothing is fetched or read from disk), so drawing it
//! allocates nothing per frame.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, Clear, Widget};

use crate::app::AppAction;

/// One world of the picker: its number and its row text (`[x] World 2`).
#[derive(Debug, PartialEq, Eq)]
struct Entry {
    world: u16,
    label: String,
}

/// The picker overlay: its rows, the highlighted row and the first drawn
/// row.
#[derive(Debug, PartialEq, Eq)]
pub struct WorldPicker {
    /// The profile it moves (the fleet row under the cursor); `None` for
    /// the marked rows, or the loaded shown rows when none is marked.
    profile: Option<String>,
    /// The world the profile is on now, if it has one: the ticked row.
    current: Option<u16>,
    entries: Vec<Entry>,
    cursor: usize,
    scroll: usize,
    /// The box title, built when the picker opens.
    title: String,
    /// The line under the title.
    note: &'static str,
    /// The widest line the box needs: title, note and rows.
    wanted: usize,
}

impl WorldPicker {
    /// The picker over `worlds` for `profile` (`None`: the marked rows),
    /// which is on `current`. The cursor starts on that world.
    pub fn open(
        profile: Option<String>,
        current: Option<u16>,
        worlds: impl Iterator<Item = u16>,
    ) -> Self {
        use std::fmt::Write as _;
        let entries: Vec<Entry> = worlds
            .map(|world| {
                let mut label = String::new();
                let tick = if current == Some(world) { 'x' } else { ' ' };
                let _ = write!(label, "[{tick}] World {world}");
                Entry { world, label }
            })
            .collect();
        let cursor = entries
            .iter()
            .position(|entry| current == Some(entry.world))
            .unwrap_or(0);
        let mut title = String::new();
        let note = match profile.as_deref() {
            Some(name) => {
                let _ = write!(title, " Switch world of {name} · Enter pick · Esc cancel ");
                "The ticked world is its own. It moves after you confirm."
            }
            None => {
                title.push_str(" Switch world of marked · Enter pick · Esc cancel ");
                "The marked bots, else the loaded shown bots, move after you confirm."
            }
        };
        let rows = entries
            .iter()
            .map(|entry| entry.label.chars().count() + 4)
            .max()
            .unwrap_or(0);
        let wanted = rows
            .max(title.chars().count())
            .max(note.chars().count() + 2);
        Self {
            profile,
            current,
            entries,
            cursor,
            scroll: 0,
            title,
            note,
            wanted,
        }
    }

    /// Move the highlight `delta` rows (the ends stop it).
    pub fn step(&mut self, delta: isize) {
        let last = self.entries.len().saturating_sub(1);
        self.cursor = self.cursor.saturating_add_signed(delta).min(last);
    }

    /// Highlight the first or the last world.
    pub fn jump(&mut self, last: bool) {
        self.cursor = if last {
            self.entries.len().saturating_sub(1)
        } else {
            0
        };
    }

    /// The highlighted row.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// What picking row `index` asks for: a Switch world plan for another
    /// world, nothing for the world the bot is on (or a row that is not
    /// there).
    pub fn pick(&self, index: usize) -> AppAction {
        match self.entries.get(index) {
            Some(entry) if self.current != Some(entry.world) => {
                AppAction::PlanSwitchWorld(self.profile.clone(), entry.world)
            }
            _ => AppAction::None,
        }
    }

    /// Draw the picker over `area`; the clickable rows (their rectangle and
    /// row index) are pushed onto `hits`. Returns the picker's rectangle.
    pub fn render(&mut self, area: Rect, buf: &mut Buffer, hits: &mut Vec<(Rect, usize)>) -> Rect {
        let width = u16::try_from(self.wanted + 2)
            .unwrap_or(u16::MAX)
            .max(40)
            .min(area.width.saturating_sub(4).max(20))
            .min(area.width);
        let height = u16::try_from(self.entries.len() + 3)
            .unwrap_or(u16::MAX)
            .min(area.height.saturating_sub(2).max(5))
            .min(area.height);
        let popup = Rect::new(
            area.x + (area.width - width) / 2,
            area.y + (area.height - height) / 2,
            width,
            height,
        );
        Clear.render(popup, buf);
        let block = Block::default().borders(Borders::ALL);
        let inner = block.inner(popup);
        block.render(popup, buf);
        if popup.height == 0 || popup.width < 3 {
            return popup;
        }
        // The title is written over the top border, not handed to the
        // block: a block title is a fresh `Line` every frame.
        buf.set_stringn(
            popup.x + 1,
            popup.y,
            &self.title,
            usize::from(popup.width.saturating_sub(2)),
            Style::default(),
        );
        if inner.height < 2 || inner.width < 8 {
            return popup;
        }
        let dim = Style::default().add_modifier(Modifier::DIM);
        buf.set_stringn(inner.x, inner.y, self.note, usize::from(inner.width), dim);
        let rows_h = usize::from(inner.height - 1);
        if self.cursor < self.scroll {
            self.scroll = self.cursor;
        } else if self.cursor >= self.scroll + rows_h {
            self.scroll = self.cursor + 1 - rows_h;
        }
        for (line, (index, entry)) in self
            .entries
            .iter()
            .enumerate()
            .skip(self.scroll)
            .take(rows_h)
            .enumerate()
        {
            let y = inner.y + 1 + line as u16;
            let at = index == self.cursor;
            let style = if at {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                Style::default()
            };
            let row = Rect::new(inner.x, y, inner.width, 1);
            buf.set_style(row, style);
            let (x, _) = buf.set_stringn(
                inner.x,
                y,
                if at { "> " } else { "  " },
                usize::from(inner.width),
                style,
            );
            buf.set_stringn(
                x,
                y,
                &entry.label,
                usize::from(inner.x + inner.width - x),
                style,
            );
            hits.push((row, index));
        }
        popup
    }
}
