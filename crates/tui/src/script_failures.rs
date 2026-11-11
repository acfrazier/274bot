//! The load-failure list: the script cards that failed to load, one row
//! each, with the selected failure's whole text under the list. The panel
//! shows the same failures in a collapsed list under its script browser;
//! both read [`frontend_core::Scripts::load_failures`] in the same order.
//!
//! It is a Script-tab popup that owns the pane's inner rectangle, like the
//! Load file browser: a header row, the list rows, a rule row, then the text
//! rows. The list takes half of what is left after the header and the rule
//! (the text never gets fewer than three rows when the pane has them), so
//! 78 x 16 (80x24) is 1 + 7 + 1 + 7 rows and 82 x 26 (120x40) is
//! 1 + 12 + 1 + 12. The list scrolls with the selection.

use std::fmt::Write as _;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Paragraph, Widget, Wrap};

use frontend_core::scripts::LoadFailureView;

use crate::script_shape::wrapped_line_count;
use crate::text::{cells, ellipsize};

/// The keys the popup answers to, as the header and the footer show them.
pub const KEYS: &str = "↑↓ pick · PgUp/PgDn text · Esc close";

/// Cells the name column takes: a longer name is cut with `…`.
const NAME_CELLS: usize = 24;

/// Fewest text rows the layout keeps when the pane has them.
const MIN_TEXT_ROWS: u16 = 3;

/// One failed card, as the list and the text area show it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadFailureRow {
    /// The card name.
    pub name: String,
    /// `stage:line:col`, with what is known.
    pub place: String,
    /// The first line of the diagnostic.
    pub first_line: String,
    /// The whole copyable text: the named line, family, identity and
    /// fingerprint.
    pub text: String,
}

impl LoadFailureRow {
    /// The row for one recorded failure.
    pub fn from_view(view: LoadFailureView<'_>) -> Self {
        let mut place = view.stage().as_str().to_owned();
        let (line, column) = view.position();
        if let Some(line) = line {
            let _ = write!(place, ":{line}");
            if let Some(column) = column {
                let _ = write!(place, ":{column}");
            }
        }
        Self {
            name: view.name().to_owned(),
            place,
            first_line: view
                .diagnostic()
                .lines()
                .map(str::trim)
                .find(|line| !line.is_empty())
                .unwrap_or_default()
                .to_owned(),
            text: view.copyable_text(),
        }
    }
}

/// The popup's rows inside the pane's inner rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FailureLayout {
    /// `Load failures: N` and the keys.
    pub header: Rect,
    /// The list rows.
    pub list: Rect,
    /// The rule between the list and the text.
    pub rule: Rect,
    /// The selected failure's text.
    pub text: Rect,
}

/// Split `inner` into the header row, the list rows, the rule row and the
/// text rows.
pub fn layout(inner: Rect) -> FailureLayout {
    let rest = inner.height.saturating_sub(2);
    let list_rows = (rest / 2)
        .min(rest.saturating_sub(MIN_TEXT_ROWS))
        .max(u16::from(rest > 0));
    let text_rows = rest - list_rows;
    let row = |y: u16, height: u16| Rect::new(inner.x, inner.y + y, inner.width, height);
    FailureLayout {
        header: row(0, inner.height.min(1)),
        list: row(1, list_rows.min(inner.height.saturating_sub(1))),
        rule: row(1 + list_rows, u16::from(inner.height > 1 + list_rows)),
        text: row(2 + list_rows, text_rows),
    }
}

/// The list's first shown row: the selection stays on screen, at the bottom
/// once it passes the last row.
pub fn list_offset(selected: usize, list_rows: u16) -> usize {
    let rows = usize::from(list_rows.max(1));
    (selected + 1).saturating_sub(rows)
}

/// The selected failure's text lines, as the text area wraps them.
fn text_lines(rows: &[LoadFailureRow], selected: usize) -> Vec<Line<'_>> {
    rows.get(selected)
        .map(|row| row.text.lines().map(Line::from).collect())
        .unwrap_or_default()
}

/// How far the text can scroll: its wrapped rows beyond the text area.
pub fn max_text_scroll(rows: &[LoadFailureRow], selected: usize, text: Rect) -> u16 {
    let wrapped = wrapped_line_count(&text_lines(rows, selected), text.width);
    wrapped
        .saturating_sub(usize::from(text.height))
        .min(usize::from(u16::MAX)) as u16
}

/// One list row: `> name  stage:line:col  first line`, cut to `width` cells.
fn list_row(row: &LoadFailureRow, selected: bool, width: usize) -> String {
    let name = ellipsize(&row.name, NAME_CELLS);
    let pad = NAME_CELLS.saturating_sub(cells(&name));
    let line = format!(
        "{}{name}{:pad$}  {}  {}",
        if selected { "> " } else { "  " },
        "",
        row.place,
        row.first_line
    );
    ellipsize(&line, width).into_owned()
}

/// Draw the popup into `inner`, the pane's inner rectangle.
pub fn render(
    buf: &mut Buffer,
    inner: Rect,
    rows: &[LoadFailureRow],
    selected: usize,
    scroll: u16,
) {
    let parts = layout(inner);
    let width = usize::from(inner.width);
    if parts.header.height > 0 {
        let header = format!("Load failures: {}  ·  {KEYS}", rows.len());
        buf.set_stringn(
            parts.header.x,
            parts.header.y,
            ellipsize(&header, width),
            width,
            Style::default(),
        );
    }
    let offset = list_offset(selected, parts.list.height);
    for (line, index) in (offset..rows.len())
        .take(usize::from(parts.list.height))
        .enumerate()
    {
        let style = if index == selected {
            Style::default().add_modifier(Modifier::REVERSED)
        } else {
            Style::default()
        };
        buf.set_stringn(
            parts.list.x,
            parts.list.y + line as u16,
            list_row(&rows[index], index == selected, width),
            width,
            style,
        );
    }
    if parts.rule.height > 0 {
        buf.set_stringn(
            parts.rule.x,
            parts.rule.y,
            "─".repeat(width),
            width,
            Style::default(),
        );
    }
    if parts.text.height > 0 {
        let scroll = scroll.min(max_text_scroll(rows, selected, parts.text));
        Paragraph::new(text_lines(rows, selected))
            .wrap(Wrap { trim: false })
            .scroll((scroll, 0))
            .render(parts.text, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_layout_splits_the_pane_into_header_list_rule_and_text() {
        // 80x24: the script pane's inner rectangle is 78 x 16.
        let small = layout(Rect::new(1, 3, 78, 16));
        assert_eq!(
            (
                small.header.height,
                small.list.height,
                small.rule.height,
                small.text.height
            ),
            (1, 7, 1, 7)
        );
        assert_eq!(small.list.y, 4);
        assert_eq!(small.rule.y, 11);
        assert_eq!(small.text.y, 12);
        // 120x40: 82 x 26.
        let large = layout(Rect::new(37, 3, 82, 26));
        assert_eq!(
            (
                large.header.height,
                large.list.height,
                large.rule.height,
                large.text.height
            ),
            (1, 12, 1, 12)
        );
        // A short pane keeps three text rows.
        let short = layout(Rect::new(0, 0, 40, 8));
        assert!(short.text.height >= MIN_TEXT_ROWS, "{short:?}");
        assert_eq!(
            short.header.height + short.list.height + short.rule.height + short.text.height,
            8
        );
    }

    #[test]
    fn the_list_scrolls_with_the_selection() {
        assert_eq!(list_offset(0, 7), 0);
        assert_eq!(list_offset(6, 7), 0);
        assert_eq!(list_offset(7, 7), 1);
        assert_eq!(list_offset(11, 7), 5);
    }

    #[test]
    fn a_long_name_is_cut_to_its_column() {
        let row = LoadFailureRow {
            name: "ANameThatIsMuchLongerThanTwentyFourCells".into(),
            place: "ParseTranspile:3:7".into(),
            first_line: "Unexpected token".into(),
            text: String::new(),
        };
        let line = list_row(&row, true, 78);
        assert!(
            line.starts_with("> ANameThatIsMuchLongerTh…  ParseTranspile:3:7  Unexpected"),
            "{line}"
        );
        assert!(cells(&line) <= 78);
    }
}
