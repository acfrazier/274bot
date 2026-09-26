use super::*;
use crossterm::event::KeyModifiers;
use frontend_core::log::Record;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn push(slot: &str, source: Source, level: Level, message: &str) {
    global().push(&Record {
        slot: Some(slot),
        tick: Some(77),
        source,
        level,
        message,
    });
}

fn screen(state: &LogPaneState, focused: &str, w: u16, h: u16) -> Vec<String> {
    let area = Rect::new(0, 0, w, h);
    let mut buf = Buffer::empty(area);
    LogPane {
        state,
        focused: Some(focused),
    }
    .render(area, &mut buf);
    (0..h)
        .map(|y| (0..w).map(|x| buf[(x, y)].symbol()).collect::<String>())
        .collect()
}

#[test]
fn the_pane_filters_like_the_panel_and_fits_80x24() {
    let slot = "tuilog-alice";
    push(slot, Source::Script, Level::Info, "Chopping oak");
    push(
        slot,
        Source::Bank,
        Level::Warn,
        "withdraw lobster: not in bank",
    );
    push(slot, Source::Login, Level::Error, "login code 3: invalid");
    let mut state = LogPaneState {
        open: true,
        ..LogPaneState::default()
    };
    state.refresh(Some(slot));
    let lines = screen(&state, slot, 80, 23);
    let text = lines.join("\n");
    assert!(text.contains("tuilog-alice ≥info src:all follow"), "{text}");
    assert!(
        lines.iter().any(|l| l.contains("script Chopping oak")),
        "{text}"
    );
    assert!(
        lines.iter().any(|l| l.contains("bank withdraw lobster")),
        "{text}"
    );
    assert!(
        lines[lines.len() - 2].contains("Esc close"),
        "help sits above the border"
    );
    let clock = lines
        .iter()
        .find(|l| l.contains("Chopping oak"))
        .map(|l| l.trim_start_matches('│').to_string())
        .unwrap();
    assert_eq!(&clock[2..3], ":");
    assert_eq!(&clock[8..9], ".");

    state.on_key(key(KeyCode::Char('v')), Some(slot)); // info → warn
    state.refresh(Some(slot));
    let text = screen(&state, slot, 80, 23).join("\n");
    assert!(!text.contains("Chopping oak"));
    assert!(text.contains("withdraw lobster"));
    assert!(text.contains("login code 3"));

    state.on_key(key(KeyCode::Char('s')), Some(slot)); // all → script
    state.on_key(key(KeyCode::Char('s')), Some(slot)); // script → login
    state.refresh(Some(slot));
    let text = screen(&state, slot, 80, 23).join("\n");
    assert!(text.contains("src:login"));
    assert!(!text.contains("withdraw lobster"));
    assert!(text.contains("login code 3"));

    state.on_key(key(KeyCode::Char('s')), Some(slot)); // login → nav
    for _ in 0..4 {
        state.on_key(key(KeyCode::Char('s')), Some(slot)); // … → all
    }
    state.on_key(key(KeyCode::Char('v')), Some(slot)); // warn → error
    state.on_key(key(KeyCode::Char('v')), Some(slot)); // error → debug
    state.on_key(key(KeyCode::Char('/')), Some(slot));
    for c in "LOBSTER".chars() {
        state.on_key(key(KeyCode::Char(c)), Some(slot));
    }
    state.on_key(key(KeyCode::Enter), Some(slot));
    state.refresh(Some(slot));
    assert_eq!(state.view.len(), 1, "case-insensitive search");
    assert!(!state.editing);

    state.on_key(key(KeyCode::Esc), Some(slot));
    assert!(!state.open);
}

#[test]
fn scrolling_up_pauses_follow_and_end_resumes_it() {
    let slot = "tuilog-bob";
    for i in 0..60 {
        push(slot, Source::Nav, Level::Info, &format!("hop {i:02}"));
    }
    let mut state = LogPaneState::default();
    state.refresh(Some(slot));
    let newest = screen(&state, slot, 80, 24).join("\n");
    assert!(newest.contains("hop 59"));
    state.on_key(key(KeyCode::PageUp), Some(slot));
    assert!(!state.view.follow);
    push(slot, Source::Nav, Level::Info, "hop 60");
    state.refresh(Some(slot));
    let paused = screen(&state, slot, 80, 24).join("\n");
    assert!(!paused.contains("hop 60"), "a paused view keeps its place");
    assert!(paused.contains("paused"));
    state.on_key(key(KeyCode::End), Some(slot));
    state.refresh(Some(slot));
    assert!(screen(&state, slot, 80, 24).join("\n").contains("hop 60"));
}
