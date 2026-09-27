//! Command palette (Ctrl-P or `:`): every operator command with its target
//! scope, its key and whether it can run now (with the reason when not).
//! Typing filters by label or group; Enter runs the highlighted command.
//! The palette is how an SSH terminal that swallows function keys still
//! reaches every screen and command.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, Clear, Widget};

use crate::app::TuiApp;
use crate::commands::{shortcut, Command};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PaletteState {
    pub query: String,
    /// Highlighted row in the filtered list.
    pub cursor: usize,
    /// First drawn row.
    pub scroll: usize,
    /// Why the last Enter did not run (the command stays highlighted).
    pub note: Option<&'static str>,
}

fn contains_ci(hay: &str, needle: &str) -> bool {
    let (hay, needle) = (hay.as_bytes(), needle.as_bytes());
    needle.is_empty()
        || hay
            .windows(needle.len())
            .any(|window| window.eq_ignore_ascii_case(needle))
}

/// Commands matching `query` (every term in the label or group name).
/// Labels that start with the query come first, then the focused pane's
/// group, then the rest, each in [`Command::ALL`] order.
pub fn palette_commands(app: &TuiApp, query: &str) -> Vec<Command> {
    let first = app.key_scope_group();
    let query = query.trim();
    let matches = |command: &Command| {
        let label = command.label(app);
        let group = command.group().label();
        query
            .split_whitespace()
            .all(|term| contains_ci(label, term) || contains_ci(group, term))
    };
    let rank = |command: &Command| {
        let label = command.label(app).as_bytes();
        let prefix = !query.is_empty()
            && label.len() >= query.len()
            && label[..query.len()].eq_ignore_ascii_case(query.as_bytes());
        if prefix {
            0
        } else if command.group() == first {
            1
        } else {
            2
        }
    };
    let mut out: Vec<Command> = Command::ALL.iter().copied().filter(matches).collect();
    out.sort_by_key(rank);
    out
}

/// The key text shown beside a command.
pub fn key_text(command: Command) -> String {
    match (command.hint(), shortcut(command)) {
        (Some(hint), _) => hint.to_string(),
        (None, Some(key)) => key.to_string(),
        (None, None) => String::new(),
    }
}

/// The palette box for a frame `area`.
pub fn palette_rect(area: Rect) -> Rect {
    let width = area.width.saturating_sub(4).clamp(20, 96).min(area.width);
    let height = area.height.saturating_sub(2).clamp(6, 26).min(area.height);
    Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 3,
        width,
        height,
    )
}

/// Draw the palette; returns each drawn row's rect and its index into
/// `commands`.
pub fn render_palette(
    app: &TuiApp,
    state: &mut PaletteState,
    commands: &[Command],
    area: Rect,
    buf: &mut Buffer,
) -> (Rect, Vec<(Rect, usize)>) {
    let popup = palette_rect(area);
    Clear.render(popup, buf);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Command palette · type to filter · Enter run · Esc close ");
    let inner = block.inner(popup);
    block.render(popup, buf);
    let mut hits = Vec::new();
    if inner.height < 3 || inner.width < 10 {
        return (popup, hits);
    }
    let right = inner.x + inner.width;
    let dim = Style::default().add_modifier(Modifier::DIM);
    buf.set_stringn(inner.x, inner.y, ":", 1, dim);
    buf.set_stringn(
        inner.x + 1,
        inner.y,
        &state.query,
        usize::from(inner.width - 1),
        Style::default(),
    );
    let caret = inner.x + 1 + state.query.chars().count() as u16;
    if caret < right {
        buf.set_stringn(
            caret,
            inner.y,
            "_",
            1,
            Style::default().add_modifier(Modifier::BOLD),
        );
    }
    let status_y = inner.y + inner.height - 1;
    let rows_y = inner.y + 1;
    let rows_h = usize::from(inner.height.saturating_sub(2));
    state.cursor = state.cursor.min(commands.len().saturating_sub(1));
    // The highlighted command's reason, whole, even when its row is cut.
    let highlighted = commands
        .get(state.cursor)
        .and_then(|command| command.availability(app).err());
    let status = match (state.note, highlighted) {
        (Some(reason), _) => format!("cannot run: {reason}"),
        (None, Some(reason)) => format!("unavailable: {reason}"),
        (None, None) => format!(
            "{} commands · Enter runs the highlighted one",
            commands.len()
        ),
    };
    buf.set_stringn(inner.x, status_y, &status, usize::from(inner.width), dim);
    if state.cursor < state.scroll {
        state.scroll = state.cursor;
    } else if rows_h > 0 && state.cursor >= state.scroll + rows_h {
        state.scroll = state.cursor + 1 - rows_h;
    }
    let label_w = 34usize.min(usize::from(inner.width) * 2 / 5);
    let scope_w = 22usize.min(usize::from(inner.width) / 4);
    for (line, (index, command)) in commands
        .iter()
        .enumerate()
        .skip(state.scroll)
        .take(rows_h)
        .enumerate()
    {
        let y = rows_y + line as u16;
        let available = command.availability(app);
        let mut style = if available.is_ok() {
            Style::default()
        } else {
            dim
        };
        if index == state.cursor {
            style = style.add_modifier(Modifier::REVERSED);
        }
        let mut text = String::with_capacity(usize::from(inner.width));
        text.push_str(if index == state.cursor { "> " } else { "  " });
        let label = command.label(app);
        text.push_str(label);
        pad(&mut text, 2 + label_w);
        let scope = command.scope(app);
        text.extend(scope.chars().take(scope_w));
        pad(&mut text, 2 + label_w + 1 + scope_w);
        let key = key_text(*command);
        text.push(' ');
        text.push_str(&key);
        if let Err(reason) = available {
            text.push_str("  - ");
            text.push_str(reason);
        }
        pad(&mut text, usize::from(inner.width));
        buf.set_stringn(inner.x, y, &text, usize::from(inner.width), style);
        hits.push((Rect::new(inner.x, y, inner.width, 1), index));
    }
    (popup, hits)
}

fn pad(text: &mut String, width: usize) {
    let used = text.chars().count();
    text.extend(std::iter::repeat_n(' ', width.saturating_sub(used)));
}
