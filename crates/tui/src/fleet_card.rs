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
use crate::text::{cells, tail_elide, wrap_cells};

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
        /// The provenance of a card that shares its name; empty otherwise.
        detail: String,
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
                detail: String::new(),
            },
            Entry::Choice {
                label: FOCUSED_CARD_LABEL.to_owned(),
                card: FleetCard::FOCUSED,
                current: now.is_focused(),
                detail: String::new(),
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
                    let detail = if card.shared_name {
                        card.provenance().to_owned()
                    } else {
                        String::new()
                    };
                    #[cfg(feature = "js")]
                    let mut label = label;
                    #[cfg(feature = "js")]
                    if card
                        .status
                        .as_ref()
                        .is_some_and(|status| status.blocks_start())
                    {
                        label.push_str(" (unavailable)");
                    }
                    let pinned = FleetCard::pinned(choice.duplicate());
                    entries.push(Entry::Choice {
                        label,
                        current: *now == pinned,
                        card: pinned,
                        detail,
                    });
                }
                #[cfg(feature = "js")]
                BrowseLine::ImportCatalog => {}
                BrowseLine::Empty | BrowseLine::NoMatch => {}
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
                // A provenance is shown by its tail in what the row has left,
                // so a card with one asks for room (the area's width caps it).
                Entry::Choice { label, detail, .. } => {
                    let tail = if detail.is_empty() {
                        0
                    } else {
                        cells(SEPARATOR) + detail.chars().count().min(PROVENANCE_TAIL_CELLS)
                    };
                    label.chars().count() + 14 + tail
                }
            })
            .max()
            .unwrap_or(0);
        let width = u16::try_from(longest.max(40))
            .unwrap_or(u16::MAX)
            .min(area.width.saturating_sub(4).max(20))
            .min(area.width);
        let footer_rows = if self
            .entries
            .iter()
            .any(|entry| matches!(entry, Entry::Choice { detail, .. } if !detail.is_empty()))
        {
            FOOTER_ROWS
        } else {
            0
        };
        let height = u16::try_from(self.entries.len() + 3 + footer_rows)
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
        // The highlighted card's full provenance, wrapped, under the rows:
        // two cards that share a name and a long path prefix stay apart.
        let footer: Vec<String> = match self.entries.get(self.cursor) {
            Some(Entry::Choice { detail, .. }) if !detail.is_empty() && inner.height >= 9 => {
                let mut rows = wrap_cells(detail, "  ", "    ", usize::from(inner.width));
                rows.truncate(FOOTER_ROWS);
                rows
            }
            _ => Vec::new(),
        };
        let rows_h = usize::from(inner.height - 1).saturating_sub(footer.len());
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
                Entry::Choice {
                    label,
                    current,
                    detail,
                    ..
                } => {
                    let at = index == self.cursor;
                    let mut text = String::with_capacity(label.len() + 14);
                    text.push_str(if at { "> " } else { "  " });
                    text.push_str(label);
                    // A shared name shows the tail of its provenance, where
                    // equal-name files differ, in the cells the row has left.
                    if !detail.is_empty() {
                        let suffix = if *current { CURRENT.len() } else { 0 };
                        let room = usize::from(inner.width)
                            .saturating_sub(cells(&text) + cells(SEPARATOR) + suffix);
                        if room >= MIN_DETAIL_CELLS {
                            text.push_str(SEPARATOR);
                            text.push_str(&tail_elide(detail, room));
                        }
                    }
                    if *current {
                        text.push_str(CURRENT);
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
        for (line, row) in footer.iter().enumerate() {
            let y = inner.y + inner.height - (footer.len() - line) as u16;
            buf.set_stringn(inner.x, y, row, usize::from(inner.width), dim);
        }
        (popup, hits)
    }
}

/// Rows under the list for the highlighted card's whole provenance.
const FOOTER_ROWS: usize = 3;
/// Between a row's label and its provenance tail.
const SEPARATOR: &str = " · ";
/// After the row of the card the fleet uses now.
const CURRENT: &str = "  (current)";
/// Cells of a provenance the popup asks room for on a row.
const PROVENANCE_TAIL_CELLS: usize = 40;
/// Fewest cells worth showing of a provenance tail on a row.
const MIN_DETAIL_CELLS: usize = 12;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::script_shape::{BrowseCard, CardOrigin};
    use frontend_core::scripts::{CardRuntime, CardSource};

    /// Two files that show one name and share a long path prefix.
    fn long_prefix_pair() -> Vec<BrowseCard> {
        ["one", "two"]
            .into_iter()
            .map(|folder| {
                let path = format!(
                    "/Users/alice/projects/team/a-shared-scripts-collection/{folder}/miner.ts"
                );
                let mut card = BrowseCard::for_test(
                    "Miner",
                    "",
                    "Skilling",
                    &[],
                    CardRuntime::NativeTick,
                    CardSource::File,
                );
                card.shared_name = true;
                card.origin = Some(Box::new(CardOrigin {
                    identity_id: path.clone(),
                    path: Some(path.clone().into()),
                    provenance: format!("File: {path}"),
                    details: Vec::new(),
                }));
                card
            })
            .collect()
    }

    #[test]
    fn equal_names_with_a_long_shared_prefix_stay_distinguishable_at_80x24() {
        let mut app = crate::test_support::fleet_app(&["alice", "bob"]);
        crate::test_support::set_cards(&mut app, long_prefix_pair());
        let mut picker = CardPicker::open(&app);
        let area = Rect::new(0, 0, 80, 24);
        let draw = |picker: &mut CardPicker| {
            let mut buf = Buffer::empty(area);
            let (popup, _) = picker.render(area, &mut buf);
            assert!(popup.width <= 80);
            crate::test_support::rows(&buf)
        };
        // The rows show where the paths differ: their tails.
        let rows = draw(&mut picker);
        for folder in ["one", "two"] {
            let tail = format!("{folder}/miner.ts");
            assert!(
                rows.iter()
                    .any(|row| row.contains("Miner") && row.contains(&tail)),
                "the {folder} row keeps its distinguishing tail:\n{}",
                rows.join("\n")
            );
        }
        // The highlighted card's whole path is on screen, wrapped.
        picker.select(4);
        let rows = draw(&mut picker);
        let squeezed: String = rows
            .iter()
            .map(|row| row.replace('│', "").split_whitespace().collect::<String>())
            .collect();
        assert!(
            squeezed.contains(
                "File:/Users/alice/projects/team/a-shared-scripts-collection/two/miner.ts"
            ),
            "{}",
            rows.join("\n")
        );
        assert!(
            picker.card_at(3) != picker.card_at(4),
            "the rows are two different choices"
        );
    }
}
