//! The Script screen's key-hint row and footer tail as each build shows
//! them. This test binary compiles the library without `cfg(test)`, so it
//! sees the hint strings a shipped `tui-play` has: the strings from before
//! Skip quest existed unless `tui/test-support` is on.

use ratatui::backend::TestBackend;
use ratatui::Terminal;
use tui::{AppAction, Screen, TuiApp};

/// The keyboard scope the footer always leads with.
const SCOPE: &str = "KEYS Script ▸ ";

/// What the footer appends after the hints, by size class.
const TAIL_STANDARD: &str = " │ Tab pane · ^P cmd · ? help · mouse off";
const TAIL_COMPACT: &str = " │ ? help · mouse off";

/// The hint row from before the Skip quest control existed.
#[cfg(not(feature = "test-support"))]
#[cfg(feature = "js")]
const BASE_HINTS: &str =
    "b browse · t start · P pause · e stop · f load · v params · R reload · T/E all";
#[cfg(not(feature = "test-support"))]
#[cfg(not(feature = "js"))]
const BASE_HINTS: &str = "b browse · t start · P pause · e stop · v params · T/E all";

/// A fresh app on the Script screen, with keyboard focus in its pane.
fn script_app() -> TuiApp {
    let mut app = TuiApp::new("274bot headless");
    assert_eq!(app.show_screen(Screen::Script), AppAction::None);
    app
}

/// One draw at `width`x`height`: the footer row, plus the text and
/// per-cell captures the TUI harness writes under `TUI_CAPTURE_DIR`.
fn draw_and_capture(app: &mut TuiApp, width: u16, height: u16, stamp: &str) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("test terminal");
    terminal.draw(|frame| app.draw(frame)).expect("draw");
    let buffer = terminal.backend().buffer();
    let area = buffer.area;
    let rows: Vec<String> = (area.y..area.y + area.height)
        .map(|y| {
            (area.x..area.x + area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect()
        })
        .collect();
    if let Some(dir) = std::env::var_os("TUI_CAPTURE_DIR").map(std::path::PathBuf::from) {
        std::fs::create_dir_all(&dir).expect("capture dir");
        let mut text = String::with_capacity(usize::from(width) * usize::from(height + 1));
        for row in &rows {
            text.push_str(row);
            text.push('\n');
        }
        std::fs::write(dir.join(format!("{stamp}_script-footer.txt")), text).expect("text capture");
        let cells: Vec<_> = buffer
            .content
            .iter()
            .map(|cell| {
                serde_json::json!({
                    "symbol": cell.symbol(),
                    "fg": format!("{:?}", cell.fg),
                    "bg": format!("{:?}", cell.bg),
                })
            })
            .collect();
        let receipt = serde_json::json!({
            "surface": "TUI",
            "cols": width,
            "rows": height,
            "proof": { "screen": "Script", "footer": rows.last() },
            "cells": cells,
        });
        std::fs::write(
            dir.join(format!("{stamp}_script-footer.json")),
            serde_json::to_vec_pretty(&receipt).expect("receipt"),
        )
        .expect("cell capture");
    }
    rows.last().cloned().expect("footer row")
}

/// The row a build shows when the hints are `hints`: the scope, the hints
/// cut to the space the tail leaves, the gap, then the tail.
#[cfg(not(feature = "test-support"))]
fn expected_footer(hints: &str, width: u16) -> String {
    let tail = if width >= 100 {
        TAIL_STANDARD
    } else {
        TAIL_COMPACT
    };
    let room = usize::from(width) - SCOPE.len() - tail.len();
    let shown: String = hints.chars().take(room).collect();
    format!("{SCOPE}{shown:<room$}{tail}")
}

#[cfg(not(feature = "test-support"))]
#[test]
fn release_build_keeps_the_script_hints_and_footer_from_before_skip_quest() {
    let mut app = script_app();
    assert_eq!(
        app.key_hints(),
        BASE_HINTS,
        "a release build shows the base hint row"
    );
    assert!(
        !app.key_hints().contains("skip"),
        "a release build names no Skip key: {}",
        app.key_hints()
    );
    for (width, height) in [(120, 40), (80, 24)] {
        let row = draw_and_capture(&mut app, width, height, &format!("release-{width}x{height}"));
        assert_eq!(
            row,
            expected_footer(BASE_HINTS, width),
            "{width}x{height}: the base footer is unchanged"
        );
    }
    assert!(
        tui::help::help_rows(&app)
            .iter()
            .all(|row| !row.what.contains("Skip")),
        "a release build has no Skip quest help row"
    );
}

#[cfg(feature = "test-support")]
#[test]
fn test_support_build_adds_the_skip_hint_and_keeps_the_rest_fitting() {
    let mut app = script_app();
    let hints = app.key_hints();
    assert!(hints.contains("X skip"), "missing skip hint: {hints}");

    // The standard width holds the whole hint row beside the base tail, so
    // the other hints stay readable.
    let standard = draw_and_capture(&mut app, 120, 40, "test-support-120x40");
    assert!(
        standard.contains(hints),
        "120x40: the hint row is cut: {standard}"
    );
    assert!(
        standard.starts_with(SCOPE) && standard.ends_with(TAIL_STANDARD),
        "120x40: the base footer tail stays: {standard}"
    );
    for key in ["b browse", "t start", "P pause", "e stop", "f load", "v params"] {
        assert!(standard.contains(key), "120x40: missing {key:?}: {standard}");
    }

    // The compact width cuts the row, as the base row was cut, and the Skip
    // hint sits before the cut beside the base tail.
    let compact = draw_and_capture(&mut app, 80, 24, "test-support-80x24");
    assert!(
        compact.contains("X skip"),
        "80x24: missing skip hint: {compact}"
    );
    assert!(
        compact.starts_with(SCOPE) && compact.ends_with(TAIL_COMPACT),
        "80x24: the base footer tail stays: {compact}"
    );
}
