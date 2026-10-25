//! The fleet-card picker: the Browse list in fleet-card mode. Its entries
//! are "Each bot's own card", "Focused bot's card" and then every Browse
//! card, grouped by category as the Script tab's Browse is. Enter pins the
//! highlighted entry as the card the fleet actions use; Esc leaves the
//! card as it was. Picking changes nothing else: not the Script tab's
//! Browse pick, not the profile form, not a run.
//!
//! The entries are built once when the picker opens, so drawing it
//! allocates nothing per frame and a card list that reloads underneath
//! never moves the cursor onto a stale row.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, Clear, Widget};

use frontend_core::{FleetCard, FOCUSED_CARD_LABEL, OWN_CARD_LABEL};

use crate::app::TuiApp;
use crate::script_shape::{browse_lines, kind_badge, source_badge, BrowseLine};

/// One row of the picker.
#[derive(Debug, PartialEq, Eq)]
enum Entry {
    /// A category heading (not selectable).
    Heading(String),
    /// A card the fleet can use.
    Choice {
        label: String,
        card: FleetCard,
        /// It is the card the fleet uses now.
        current: bool,
    },
}

/// The picker overlay: its rows, the highlighted row and the first drawn
/// row.
#[derive(Debug, PartialEq, Eq)]
pub struct CardPicker {
    entries: Vec<Entry>,
    cursor: usize,
    scroll: usize,
}

impl CardPicker {
    /// The picker over `app`'s Browse cards, with the cursor on the card
    /// the fleet uses now.
    pub fn open(app: &TuiApp) -> Self {
        let now = app.fleet_model.card();
        let mut entries = vec![
            Entry::Choice {
                label: OWN_CARD_LABEL.to_owned(),
                card: FleetCard::OWN,
                current: now.is_own(),
            },
            Entry::Choice {
                label: FOCUSED_CARD_LABEL.to_owned(),
                card: FleetCard::FOCUSED,
                current: now.is_focused(),
            },
        ];
        #[cfg(feature = "js")]
        let lines = browse_lines(&app.script_cards, &app.script_category_order, false);
        #[cfg(not(feature = "js"))]
        let lines = browse_lines(&app.script_cards, &app.script_category_order);
        for line in lines {
            match line {
                BrowseLine::Category(name) => entries.push(Entry::Heading(name)),
                BrowseLine::Card(index) => {
                    let (Some(card), Some(choice)) = (
                        app.script_cards.get(index),
                        app.script_card_choices.get(index),
                    ) else {
                        continue;
                    };
                    let label = format!(
                        "{}  [{}] [{}]",
                        card.name,
                        kind_badge(card.kind),
                        source_badge(card.source)
                    );
                    #[cfg(feature = "js")]
                    let mut label = label;
                    #[cfg(feature = "js")]
                    if card.unloadable.is_some() {
                        label.push_str(" (unloadable)");
                    }
                    let pinned = FleetCard::pinned(choice.duplicate());
                    entries.push(Entry::Choice {
                        label,
                        current: *now == pinned,
                        card: pinned,
                    });
                }
                #[cfg(feature = "js")]
                BrowseLine::ImportCatalog => {}
                BrowseLine::Empty => {}
            }
        }
        let cursor = entries
            .iter()
            .position(|entry| matches!(entry, Entry::Choice { current: true, .. }))
            .unwrap_or(0);
        Self {
            entries,
            cursor,
            scroll: 0,
        }
    }

    /// Move the highlight `delta` selectable rows (headings are skipped;
    /// the ends stop it).
    pub fn step(&mut self, delta: isize) {
        let mut at = self.cursor;
        let mut left = delta.unsigned_abs();
        while left > 0 {
            let next = if delta < 0 {
                self.entries[..at]
                    .iter()
                    .rposition(|entry| matches!(entry, Entry::Choice { .. }))
            } else {
                self.entries[at + 1..]
                    .iter()
                    .position(|entry| matches!(entry, Entry::Choice { .. }))
                    .map(|offset| at + 1 + offset)
            };
            match next {
                Some(next) => at = next,
                None => break,
            }
            left -= 1;
        }
        self.cursor = at;
    }

    /// Highlight the first or the last card.
    pub fn jump(&mut self, last: bool) {
        // Row 0 ("Each bot's own card") is always selectable.
        self.cursor = if last {
            self.entries
                .iter()
                .rposition(|entry| matches!(entry, Entry::Choice { .. }))
                .unwrap_or(0)
        } else {
            0
        };
    }

    /// Highlight row `index` if it is a card.
    pub fn select(&mut self, index: usize) {
        if matches!(self.entries.get(index), Some(Entry::Choice { .. })) {
            self.cursor = index;
        }
    }

    /// The card of row `index`, if it is a card.
    pub fn card_at(&self, index: usize) -> Option<FleetCard> {
        match self.entries.get(index) {
            Some(Entry::Choice { card, .. }) => Some(card.duplicate()),
            _ => None,
        }
    }

    /// The highlighted row's card.
    pub fn highlighted(&self) -> Option<FleetCard> {
        self.card_at(self.cursor)
    }

    /// Draw the picker over `area`; returns its rectangle and the drawn
    /// card rows (their rectangle and row index) for mouse hits.
    pub fn render(&mut self, area: Rect, buf: &mut Buffer) -> (Rect, Vec<(Rect, usize)>) {
        let longest = self
            .entries
            .iter()
            .map(|entry| match entry {
                Entry::Heading(name) => name.chars().count() + 4,
                Entry::Choice { label, .. } => label.chars().count() + 14,
            })
            .max()
            .unwrap_or(0);
        let width = u16::try_from(longest.max(40))
            .unwrap_or(u16::MAX)
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
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Pick fleet card · Up/Down · Enter pick · Esc keep ");
        let inner = block.inner(popup);
        block.render(popup, buf);
        let mut hits = Vec::new();
        if inner.height < 2 || inner.width < 8 {
            return (popup, hits);
        }
        let dim = Style::default().add_modifier(Modifier::DIM);
        buf.set_stringn(
            inner.x,
            inner.y,
            "The fleet actions use this card. Nothing else changes.",
            usize::from(inner.width),
            dim,
        );
        let rows_h = usize::from(inner.height - 1);
        if self.cursor < self.scroll {
            self.scroll = self.cursor;
        } else if self.cursor >= self.scroll + rows_h {
            self.scroll = self.cursor + 1 - rows_h;
        }
        // Keep the heading of the cursor's category in view.
        if self.scroll > 0 && self.cursor == self.scroll {
            if let Some(Entry::Heading(_)) = self.entries.get(self.scroll - 1) {
                self.scroll -= 1;
            }
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
            match entry {
                Entry::Heading(name) => {
                    let mut text = String::with_capacity(name.len() + 4);
                    text.push_str("  ");
                    text.push_str(name);
                    buf.set_stringn(inner.x, y, &text, usize::from(inner.width), dim);
                }
                Entry::Choice { label, current, .. } => {
                    let at = index == self.cursor;
                    let mut text = String::with_capacity(label.len() + 14);
                    text.push_str(if at { "> " } else { "  " });
                    text.push_str(label);
                    if *current {
                        text.push_str("  (current)");
                    }
                    let mut style = Style::default();
                    if at {
                        style = style.add_modifier(Modifier::REVERSED);
                    }
                    buf.set_stringn(inner.x, y, &text, usize::from(inner.width), style);
                    hits.push((Rect::new(inner.x, y, inner.width, 1), index));
                }
            }
        }
        (popup, hits)
    }
}
