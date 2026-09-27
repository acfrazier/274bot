//! Responsive shell geometry: the size class, the pane rectangles for one
//! terminal size, and the hit regions the last draw recorded. Pure layout;
//! no app state and no drawing. Mouse routing reads [`Regions`] only, so a
//! resize takes effect with the next draw and never reuses stale rects.

use ratatui::layout::{Position, Rect};

use crate::commands::Command;

/// Terminal size class. Compact below 100x30 (80x24 is the floor the shell
/// is designed for), Standard from 100x30 (120x40 is the reference), Large
/// from 160x45.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SizeClass {
    #[default]
    Compact,
    Standard,
    Large,
}

impl SizeClass {
    pub fn of(area: Rect) -> Self {
        if area.width >= 160 && area.height >= 45 {
            Self::Large
        } else if area.width >= 100 && area.height >= 30 {
            Self::Standard
        } else {
            Self::Compact
        }
    }
}

/// The selected bot's detail tabs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Screen {
    #[default]
    Overview,
    Map,
    Script,
    Chat,
    Logs,
}

impl Screen {
    pub const ALL: [Screen; 5] = [
        Screen::Overview,
        Screen::Map,
        Screen::Script,
        Screen::Chat,
        Screen::Logs,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Screen::Overview => "Overview",
            Screen::Map => "Map",
            Screen::Script => "Script",
            Screen::Chat => "Chat",
            Screen::Logs => "Logs",
        }
    }

    /// The function key that shows this tab.
    pub fn key(self) -> &'static str {
        match self {
            Screen::Overview => "F3",
            Screen::Map => "F4",
            Screen::Script => "F5",
            Screen::Chat => "F6",
            Screen::Logs => "F7",
        }
    }
}

/// Which pane receives keys. The selected bot is separate (core state);
/// this is only where the keyboard points.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Pane {
    Fleet,
    #[default]
    Detail,
    /// The log drawer under the panes (Standard and Large only).
    Drawer,
}

/// A header tab: the fleet or one detail screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Fleet,
    Screen(Screen),
}

/// The pane rectangles for one frame. Empty rects are hidden panes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ShellRects {
    /// Two header rows: title/counts/resources, then BOT and tabs.
    pub header: Rect,
    pub fleet: Rect,
    pub detail: Rect,
    /// Large only: status and chat beside the detail tab.
    pub side: Rect,
    /// Message line plus log rows (Compact: three plain rows).
    pub drawer: Rect,
    /// One row: keyboard scope and its keys.
    pub footer: Rect,
}

/// Lay the shell out for `area`. At Compact one main pane holds either the
/// fleet (`fleet_main`, the fleet drawer) or the detail tab. The drawer
/// shrinks to its message line while the Logs tab already shows the log.
pub fn shell_rects(area: Rect, class: SizeClass, fleet_main: bool, screen: Screen) -> ShellRects {
    let header_h = area.height.min(2);
    let footer_h = u16::from(area.height > header_h);
    let logs_shown = screen == Screen::Logs;
    let drawer_want = match class {
        SizeClass::Compact if logs_shown => 1,
        SizeClass::Compact => 3,
        SizeClass::Standard if logs_shown => 1,
        SizeClass::Standard => 7,
        SizeClass::Large if logs_shown => 1,
        SizeClass::Large => 10,
    };
    let free = area.height.saturating_sub(header_h + footer_h);
    // The panes keep at least half of what is left; the drawer never
    // pushes them off screen on a short terminal.
    let drawer_h = drawer_want.min(free / 2);
    let body_h = free - drawer_h;
    let header = Rect::new(area.x, area.y, area.width, header_h);
    let body = Rect::new(area.x, area.y + header_h, area.width, body_h);
    let drawer = Rect::new(area.x, body.y + body_h, area.width, drawer_h);
    let footer = Rect::new(area.x, drawer.y + drawer_h, area.width, footer_h);
    let (fleet, detail, side) = match class {
        SizeClass::Compact if fleet_main => (body, Rect::default(), Rect::default()),
        SizeClass::Compact => (Rect::default(), body, Rect::default()),
        SizeClass::Standard => {
            let fleet_w = (body.width * 3 / 10).clamp(30, 40).min(body.width);
            split_columns(body, fleet_w, 0)
        }
        SizeClass::Large => split_columns(body, 64.min(body.width), 46),
    };
    ShellRects {
        header,
        fleet,
        detail,
        side,
        drawer,
        footer,
    }
}

fn split_columns(body: Rect, fleet_w: u16, side_w: u16) -> (Rect, Rect, Rect) {
    let fleet = Rect::new(body.x, body.y, fleet_w, body.height);
    let rest = body.width - fleet_w;
    let side_w = if rest >= side_w + 40 { side_w } else { 0 };
    let detail = Rect::new(body.x + fleet_w, body.y, rest - side_w, body.height);
    let side = if side_w == 0 {
        Rect::default()
    } else {
        Rect::new(detail.x + detail.width, body.y, side_w, body.height)
    };
    (fleet, detail, side)
}

/// Clickable targets recorded by the last draw. Every draw starts from
/// [`Regions::begin`], so a resize or a hidden pane never leaves a stale
/// target behind.
#[derive(Debug, Default)]
pub struct Regions {
    pub area: Rect,
    pub class: SizeClass,
    pub rects: ShellRects,
    pub tabs: Vec<(Rect, Tab)>,
    /// Command buttons drawn this frame (header, fleet, overview, map, drawer).
    pub buttons: Vec<(Rect, Command)>,
    /// Fleet body rows and the member index shown on its first row.
    pub fleet_rows: Rect,
    pub fleet_first: usize,
    /// Map cells (click selects a tile).
    pub map: Rect,
    /// Log rows (drawer or Logs tab); the wheel scrolls them.
    pub log_rows: Rect,
    /// The message line; a click opens the full text.
    pub message: Rect,
    /// The open overlay's box and its clickable rows (`index` into the
    /// overlay's own list).
    pub modal: Rect,
    pub modal_items: Vec<(Rect, usize)>,
}

impl Regions {
    pub fn begin(&mut self, area: Rect, class: SizeClass, rects: ShellRects) {
        self.area = area;
        self.class = class;
        self.rects = rects;
        self.tabs.clear();
        self.buttons.clear();
        self.fleet_rows = Rect::default();
        self.fleet_first = 0;
        self.map = Rect::default();
        self.log_rows = Rect::default();
        self.message = Rect::default();
        self.modal = Rect::default();
        self.modal_items.clear();
    }

    pub fn button_at(&self, col: u16, row: u16) -> Option<Command> {
        let at = Position::new(col, row);
        self.buttons
            .iter()
            .find(|(rect, _)| rect.contains(at))
            .map(|(_, command)| *command)
    }

    pub fn tab_at(&self, col: u16, row: u16) -> Option<Tab> {
        let at = Position::new(col, row);
        self.tabs
            .iter()
            .find(|(rect, _)| rect.contains(at))
            .map(|(_, tab)| *tab)
    }

    pub fn modal_item_at(&self, col: u16, row: u16) -> Option<usize> {
        let at = Position::new(col, row);
        self.modal_items
            .iter()
            .find(|(rect, _)| rect.contains(at))
            .map(|(_, index)| *index)
    }
}

pub fn contains(rect: Rect, col: u16, row: u16) -> bool {
    rect.contains(Position::new(col, row))
}
