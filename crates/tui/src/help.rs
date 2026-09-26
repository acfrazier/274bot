//! Help overlay (F1 or `?`): the keys of the pane the keyboard points at
//! first, then the global keys and every other pane. Command keys come from
//! the same tables the router reads (`commands`), so help cannot drift from
//! routing. Typing filters the rows.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, Clear, Widget};

use crate::app::TuiApp;
use crate::commands::{Command, CHAT_KEYS, FLEET_KEYS, LOG_KEYS, MAP_KEYS, OVERVIEW_KEYS};
use crate::script_shape::SCRIPT_KEYS;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HelpState {
    pub query: String,
    pub scroll: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelpRow {
    pub context: &'static str,
    pub keys: String,
    pub what: String,
}

const GLOBAL: &[(&str, &str)] = &[
    ("F1 / ?", "this help (type to search)"),
    (
        "Ctrl-P / :",
        "command palette: every command, its scope and why it is unavailable",
    ),
    ("F2", "Fleet (the fleet drawer at 80x24)"),
    (
        "F3 F4 F5 F6 F7",
        "Overview, Map, Script, Chat, Logs of the selected bot",
    ),
    ("Tab / Shift-Tab", "move keyboard focus between panes"),
    ("Esc", "close or go back one level; never runs a command"),
    ("q / Ctrl-Q", "quit (asks first while bots are loaded)"),
];

const FLEET_NAV: &[(&str, &str)] = &[
    (
        "Up Down j k",
        "move the cursor (the selected bot does not change)",
    ),
    ("PgUp PgDn Home End", "move by a page / to the ends"),
    ("Enter", "select the bot under the cursor"),
    (
        "Space",
        "select / unselect the row for group actions (Map group walk)",
    ),
    (
        "/",
        "filter: name, wN or world:N, state (ready, queued, error…)",
    ),
];

const MAP_NAV: &[(&str, &str)] = &[
    ("arrows h j k l", "pan"),
    ("+ -", "zoom"),
    (
        "Enter / click",
        "select the centre (click: that tile); Enter again walks",
    ),
    ("PgUp PgDn 0-3", "plane"),
    ("d c r", "dots, collision and reach layers"),
    (
        "Space",
        "select / unselect the selected bot for a group walk",
    ),
    ("Esc", "clear the selection, then leave the map"),
];

const SCRIPT_NAV: &[(&str, &str)] = &[
    ("Up Down j k", "move the Browse / Load / catalog list"),
    (
        "Enter",
        "close Browse (the pick stays) or open the highlighted entry",
    ),
    (
        "Esc",
        "close Browse / Load; the catalog prompt means Not now",
    ),
];

const CHAT_NAV: &[(&str, &str)] = &[
    (
        "Up Down j k",
        "choose a dialogue option or a script paint button",
    ),
    (
        "Space / Enter",
        "continue or answer the dialogue; press the paint button",
    ),
    ("1-9", "press script paint button N"),
];

const LOG_NAV: &[(&str, &str)] = &[
    ("/", "search the log"),
    (
        "v s b",
        "minimum level, source, scope (bot / process / all)",
    ),
    ("f", "follow on / off"),
    ("Up Down PgUp PgDn Home End", "scroll"),
];

const MANUAL: &[(&str, &str)] = &[
    (
        "W A S D / arrows",
        "walk the selected bot one tile (Manual walk only)",
    ),
    ("Esc", "disarm manual walking"),
];

const MOUSE: &[(&str, &str)] = &[
    (
        "left click",
        "focus a pane, select a row or bot, press a button or tab",
    ),
    (
        "right click",
        "context menu for the row or pane (never a left click)",
    ),
    ("wheel", "scroll the list, log or map under the pointer"),
    (
        "palette",
        "Mouse capture off lets the terminal select and copy text",
    ),
];

fn push_static(out: &mut Vec<HelpRow>, context: &'static str, rows: &[(&str, &str)]) {
    out.extend(rows.iter().map(|(keys, what)| HelpRow {
        context,
        keys: (*keys).to_string(),
        what: (*what).to_string(),
    }));
}

fn push_keys(
    out: &mut Vec<HelpRow>,
    app: &TuiApp,
    context: &'static str,
    table: &[(char, Command)],
) {
    out.extend(table.iter().map(|(key, command)| HelpRow {
        context,
        keys: key.to_string(),
        what: command.label(app).to_string(),
    }));
}

fn section(app: &TuiApp, context: &'static str) -> Vec<HelpRow> {
    let mut rows = Vec::new();
    match context {
        "Global" => push_static(&mut rows, context, GLOBAL),
        "Fleet" => {
            push_static(&mut rows, context, FLEET_NAV);
            push_keys(&mut rows, app, context, FLEET_KEYS);
        }
        "Overview" => push_keys(&mut rows, app, context, OVERVIEW_KEYS),
        "Map" => {
            push_static(&mut rows, context, MAP_NAV);
            push_keys(&mut rows, app, context, MAP_KEYS);
        }
        "Script" => {
            rows.extend(SCRIPT_KEYS.iter().map(|(name, key)| HelpRow {
                context,
                keys: key.to_string(),
                what: (*name).to_string(),
            }));
            push_static(&mut rows, context, SCRIPT_NAV);
        }
        "Chat" => {
            push_static(&mut rows, context, CHAT_NAV);
            push_keys(&mut rows, app, context, CHAT_KEYS);
        }
        "Logs" => {
            push_static(&mut rows, context, LOG_NAV);
            push_keys(&mut rows, app, context, LOG_KEYS);
        }
        "Manual walk" => push_static(&mut rows, context, MANUAL),
        _ => push_static(&mut rows, context, MOUSE),
    }
    rows
}

const CONTEXTS: [&str; 9] = [
    "Global",
    "Fleet",
    "Overview",
    "Map",
    "Script",
    "Chat",
    "Logs",
    "Manual walk",
    "Mouse",
];

/// Every help row: the current pane's section, then Global, then the rest.
pub fn help_rows(app: &TuiApp) -> Vec<HelpRow> {
    let current = app.key_scope_context();
    let mut rows = section(app, current);
    for context in CONTEXTS {
        if context != current {
            rows.extend(section(app, context));
        }
    }
    rows
}

fn contains_ci(hay: &str, needle: &str) -> bool {
    let (hay, needle) = (hay.as_bytes(), needle.as_bytes());
    needle.is_empty()
        || hay
            .windows(needle.len())
            .any(|window| window.eq_ignore_ascii_case(needle))
}

/// Rows matching every term of `query` (context, keys or description).
pub fn filter_rows(rows: Vec<HelpRow>, query: &str) -> Vec<HelpRow> {
    rows.into_iter()
        .filter(|row| {
            query.split_whitespace().all(|term| {
                contains_ci(row.context, term)
                    || contains_ci(&row.keys, term)
                    || contains_ci(&row.what, term)
            })
        })
        .collect()
}

pub fn help_rect(area: Rect) -> Rect {
    let width = area.width.saturating_sub(4).clamp(20, 110).min(area.width);
    let height = area.height.saturating_sub(2).max(4).min(area.height);
    Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    )
}

pub fn render_help(state: &mut HelpState, rows: &[HelpRow], area: Rect, buf: &mut Buffer) -> Rect {
    let popup = help_rect(area);
    Clear.render(popup, buf);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Help · type to search · Up/Down scroll · Esc close ");
    let inner = block.inner(popup);
    block.render(popup, buf);
    if inner.height < 2 || inner.width < 10 {
        return popup;
    }
    let dim = Style::default().add_modifier(Modifier::DIM);
    let mut query = String::from("search: ");
    query.push_str(&state.query);
    query.push('_');
    buf.set_stringn(inner.x, inner.y, &query, usize::from(inner.width), dim);
    let rows_h = usize::from(inner.height - 1);
    state.scroll = state.scroll.min(rows.len().saturating_sub(rows_h));
    let keys_w = 26usize.min(usize::from(inner.width) / 3);
    let mut last_context = "";
    let text_x = inner.x + 12.min(inner.width);
    let text_w = usize::from(inner.x + inner.width - text_x);
    for (line, row) in rows.iter().skip(state.scroll).take(rows_h).enumerate() {
        let y = inner.y + 1 + line as u16;
        if row.context != last_context {
            buf.set_stringn(
                inner.x,
                y,
                row.context,
                12.min(usize::from(inner.width)),
                Style::default().add_modifier(Modifier::BOLD),
            );
        }
        last_context = row.context;
        let mut text = String::with_capacity(text_w);
        text.push_str(&row.keys);
        let used = text.chars().count();
        text.extend(std::iter::repeat_n(' ', keys_w.saturating_sub(used)));
        text.push(' ');
        text.push_str(&row.what);
        buf.set_stringn(text_x, y, &text, text_w, Style::default());
    }
    if rows.is_empty() {
        buf.set_stringn(
            inner.x,
            inner.y + 1,
            "no key matches",
            usize::from(inner.width),
            dim,
        );
    }
    popup
}
