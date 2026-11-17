//! Shared fixtures for the shell tests: key events, a fleet of ready bots,
//! drawing to a `TestBackend` and finding text by terminal cell.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::Terminal;

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

/// A process local-state service on the state root `root` whose legacy
/// `panel-ui.json` is over the size bound: the first open refuses the
/// preferences (no import, no defaults), and every save fails at once. The
/// legacy file's path is returned for [`repair_refused_state`].
pub fn open_refused_state(
    root: &std::path::Path,
) -> (
    std::sync::Arc<host_play::local_state::LocalStateService>,
    std::path::PathBuf,
) {
    use host_play::local_state::{LocalStateService, StateRoot};
    let legacy = StateRoot::new(root.to_path_buf()).preferences_file();
    write_private_fixture(
        &legacy,
        vec![b' '; host_play::local_state::MAX_PREFERENCES_BYTES as usize + 1],
    )
    .expect("write the over-size legacy preferences");
    let service = LocalStateService::open_process(StateRoot::new(root.to_path_buf()));
    let preferences = frontend_core::UiPreferences::of_service_for_test(&service);
    assert!(
        preferences.reload().is_err(),
        "an over-size legacy file refuses the preferences"
    );
    (service, legacy)
}

/// Replace the refused legacy file at `legacy` with a private valid empty
/// document, as the operator's recovery does: the next read imports it and
/// the preferences work.
pub fn repair_refused_state(preferences: &frontend_core::UiPreferences, legacy: &std::path::Path) {
    write_private_fixture(legacy, b"{}").expect("write the repaired legacy preferences");
    preferences
        .reload()
        .expect("the repaired legacy file imports");
}

/// The saved preferences as one JSON object, read through the store (a
/// fresh read, so another writer's save is seen too).
pub fn saved_preferences(preferences: &frontend_core::UiPreferences) -> serde_json::Value {
    let document = preferences.reload().expect("the preferences read");
    serde_json::Value::Object(document.as_ref().clone())
}

/// Another process's save: a second service over the same state root as
/// `preferences` replaces the member `key` with `value`, and it is durable
/// when this returns.
pub fn peer_saves(preferences: &frontend_core::UiPreferences, key: &str, value: serde_json::Value) {
    let database = preferences.path();
    let root = database
        .parent()
        .and_then(std::path::Path::parent)
        .expect("the state root");
    let peer = host_play::local_state::LocalStateService::open_process(
        host_play::local_state::StateRoot::new(root.to_path_buf()),
    );
    frontend_core::UiPreferences::of_service_for_test(&peer)
        .set(key, value)
        .expect("the peer's save is durable");
}

/// Give the app these Browse cards and the core picker choice of each (the
/// pump copies both from the session): what the fleet-card picker lists.
pub fn set_cards(app: &mut TuiApp, cards: Vec<crate::script_shape::BrowseCard>) {
    app.script_card_choices = cards
        .iter()
        .map(|card| crate::script_shape::card_selection(card).expect("a card with a choice"))
        .collect();
    app.script_cards = cards;
}
