//! Shared fixtures for the shell tests: key events, a fleet of ready bots,
//! drawing to a `TestBackend` and finding text by terminal cell.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::Terminal;

use host_play::{SlotStatus, StartupPhase};

use crate::app::TuiApp;

pub const TITLE: &str = "289bot headless · local-289 · 127.0.0.1:44594 · revision 289";

pub fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

pub fn ch(c: char) -> KeyEvent {
    key(KeyCode::Char(c))
}

pub fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

pub fn mouse(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
    MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    }
}

pub fn right_click(column: u16, row: u16) -> MouseEvent {
    mouse(MouseEventKind::Down(MouseButton::Right), column, row)
}

/// A logged-in, ready member on `world`.
pub fn ready(name: &str, world: u16) -> SlotStatus {
    SlotStatus {
        username: name.into(),
        world: Some(world),
        startup_phase: StartupPhase::Ready,
        connected: true,
        ingame: true,
        scene_state: 2,
        tile_x: 3200,
        tile_z: 3201,
        ..SlotStatus::default()
    }
}

/// An app whose fleet is `names` (all ready on w2), the first selected.
pub fn fleet_app(names: &[&str]) -> TuiApp {
    let mut app = TuiApp::new(TITLE);
    app.names = names.iter().map(|n| n.to_string()).collect();
    app.statuses = names.iter().map(|n| ready(n, 2)).collect();
    app.focused = (!names.is_empty()).then_some(0);
    app.refresh();
    app
}

pub fn rows(buf: &Buffer) -> Vec<String> {
    let area = buf.area;
    (area.y..area.y + area.height)
        .map(|y| {
            (area.x..area.x + area.width)
                .map(|x| buf[(x, y)].symbol())
                .collect()
        })
        .collect()
}

/// Draw `app` at `width`x`height`; one string per row, one char per cell.
pub fn draw(app: &mut TuiApp, width: u16, height: u16) -> Vec<String> {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    rows(terminal.backend().buffer())
}

/// The cell where `needle` starts (columns counted in cells, not bytes).
pub fn find(rows: &[String], needle: &str) -> Option<(u16, u16)> {
    let want: Vec<char> = needle.chars().collect();
    rows.iter().enumerate().find_map(|(y, row)| {
        let cells: Vec<char> = row.chars().collect();
        (0..cells.len())
            .find(|&x| cells[x..].starts_with(&want))
            .map(|x| (x as u16, y as u16))
    })
}

pub fn text(rows: &[String]) -> String {
    rows.join("\n")
}
