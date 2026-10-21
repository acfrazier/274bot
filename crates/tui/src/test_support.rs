//! Shared fixtures for the shell tests: key events, a fleet of ready bots,
//! drawing to a `TestBackend` and finding text by terminal cell.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::Terminal;

use frontend_core::scripts::ScriptRunState;
use frontend_core::{FleetCounts, FleetRow, Phase, ProfileIdentity, SlotDetail};

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

/// A logged-in, ready member's projected row on `world`.
pub fn ready(name: &str, world: u16) -> FleetRow {
    FleetRow::fixture(name, Phase::Ready, Some(world), None)
}

/// The selected member's projected detail: in game on w2 at 3200,3201.
pub fn ready_detail(name: &str) -> SlotDetail {
    let mut detail = SlotDetail::default();
    detail.set_row(ready(name, 2));
    detail.set_state("ingame scene 2");
    detail.set_tile((3200, 3201, 0));
    detail.set_ready_tile(Some((3200, 3201, 0)));
    detail
}

/// An app whose fleet is `names` (all ready on w2), the first selected.
pub fn fleet_app(names: &[&str]) -> TuiApp {
    let mut app = TuiApp::new(TITLE);
    app.names = names.iter().map(|n| n.to_string()).collect();
    app.fleet_model
        .set_universe_for_test(names.iter().map(|n| ready(n, 2)).collect(), &[]);
    app.counts = FleetCounts {
        loaded: names.len(),
        ready: names.len(),
        ..FleetCounts::default()
    };
    app.focused = (!names.is_empty()).then_some(0);
    app.detail = names.first().map(|name| ready_detail(name));
    app.refresh();
    app
}

/// Give the app's members these vault identities (the model's rows keep
/// their order), as the production session's projection does.
pub fn set_profile_ids(app: &mut TuiApp, ids: &[ProfileIdentity]) {
    app.profile_ids = ids.to_vec();
    let rows: Vec<FleetRow> = app
        .fleet_model
        .rows()
        .iter()
        .map(FleetRow::snapshot)
        .collect();
    app.fleet_model.set_universe_for_test(rows, ids);
}

/// Mark the first `count` rows' scripts as running.
pub fn run_scripts(app: &mut TuiApp, count: usize) {
    let mut rows: Vec<FleetRow> = app
        .fleet_model
        .rows()
        .iter()
        .map(FleetRow::snapshot)
        .collect();
    for row in rows.iter_mut().take(count) {
        row.set_script(ScriptRunState::Running);
    }
    let ids = app.fleet_model.identities().to_vec();
    app.fleet_model.set_universe_for_test(rows, &ids);
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

/// Write a private-state fixture, usually a preferences file, with mode
/// 0600 whatever the process umask. A plain write creates the file 0664
/// under umask 0002, and the product reader refuses a group- or
/// world-writable preferences file. The TUI has no `vault` dependency, so
/// the mode is set here, before any read.
pub fn write_private_fixture(
    path: impl AsRef<std::path::Path>,
    bytes: impl AsRef<[u8]>,
) -> std::io::Result<()> {
    let path = path.as_ref();
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}
