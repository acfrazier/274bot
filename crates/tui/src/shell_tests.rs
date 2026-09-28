use crossterm::event::KeyCode;

use crate::app::AppAction;
use crate::layout::{Pane, Screen};
use crate::test_support::{ch, draw, find, fleet_app, key, ready_detail, text};

/// 80x24: two header rows (the title yields to the counts and the whole
/// meter), one main pane (the fleet drawer or the bot's tab), a three-row
/// message/log drawer and the one-row footer.
#[test]
fn compact_80x24_keeps_one_main_pane_between_header_drawer_and_footer() {
    let mut app = fleet_app(&["alice", "bob"]);
    app.error = Some("Start all: started 2, skipped 0".into());
    app.resources.brief = "cpu 12% ram 263.9 MB net 1.2 KB/s".into();
    let rows = draw(&mut app, 80, 24);
    assert!(rows[0].contains("289bot"), "{}", rows[0]);
    assert!(
        rows[0].contains("2/2 ready Q:0 Err:0 │ cpu 12% ram 263.9 MB net 1.2 KB/s"),
        "{}",
        rows[0]
    );
    assert!(rows[1].starts_with("BOT alice @w2 idle"), "{}", rows[1]);
    assert!(
        rows[1].contains("[Fleet]"),
        "the fleet drawer holds the main pane: {}",
        rows[1]
    );
    assert!(
        find(&rows, "*alice").is_some(),
        "fleet rows: {}",
        text(&rows)
    );
    assert!(
        rows[20].starts_with("msg: Start all: started 2, skipped 0"),
        "the message stays visible at 80 columns: {}",
        rows[20]
    );
    assert!(rows[21].starts_with("log: "), "{}", rows[21]);
    assert!(rows[22].starts_with("logs: "), "{}", rows[22]);
    assert!(rows[23].starts_with("KEYS Fleet ▸"), "{}", rows[23]);

    app.on_key(key(KeyCode::Down));
    assert_eq!(
        app.on_key(key(KeyCode::Enter)),
        AppAction::Focus("bob".into())
    );
    // The next pump copies the newly selected bot's detail.
    app.detail = Some(ready_detail("bob"));
    let rows = draw(&mut app, 80, 24);
    assert!(rows[1].starts_with("BOT bob @w2 idle"), "{}", rows[1]);
    assert!(
        rows[1].contains("[Overview]"),
        "the drawer closed on Enter: {}",
        rows[1]
    );
    assert!(rows[23].starts_with("KEYS Overview ▸"), "{}", rows[23]);
    assert!(text(&rows).contains("[Log in i]"), "{}", text(&rows));
    assert!(
        text(&rows).contains("state: ingame scene 2"),
        "{}",
        text(&rows)
    );
}

/// 120x40: fleet table beside the selected bot's tab, the log drawer and
/// the footer; the keyboard focus is a labelled border.
#[test]
fn standard_120x40_shows_fleet_detail_drawer_and_footer() {
    let mut app = fleet_app(&["alice", "bob", "carol"]);
    let rows = draw(&mut app, 120, 40);
    assert!(
        rows[0].contains("loaded 3 ready 3 queued 0 failed 0 │ cpu … ram … net …"),
        "counts, then the meter still measuring: {}",
        rows[0]
    );
    assert!(rows[1].contains(" Fleet F2 "), "{}", rows[1]);
    assert!(rows[1].contains("[Overview F3]"), "{}", rows[1]);
    assert!(
        rows[1].contains("[? help]") && rows[1].contains("[: cmd]"),
        "{}",
        rows[1]
    );
    assert!(
        rows[2].contains("FLEET") && rows[2].contains("[keys]"),
        "{}",
        rows[2]
    );
    assert!(
        rows[2].contains("BOT alice │ w2 │ ready │ script idle"),
        "{}",
        rows[2]
    );
    for name in ["*alice", "bob", "carol"] {
        assert!(find(&rows, name).is_some(), "{name}: {}", text(&rows));
    }
    assert!(
        text(&rows).contains("3/3 shown · selected 0 of 3"),
        "{}",
        text(&rows)
    );
    assert!(rows[32].contains("LOG alice"), "drawer: {}", rows[32]);
    assert!(rows[33].contains("msg: —"), "{}", rows[33]);
    assert!(rows[39].starts_with("KEYS Fleet ▸"), "{}", rows[39]);

    app.on_key(key(KeyCode::Tab));
    let rows = draw(&mut app, 120, 40);
    let fleet_top: String = rows[2].chars().take(36).collect();
    assert!(
        !fleet_top.contains("[keys]"),
        "focus left the fleet: {fleet_top}"
    );
    assert!(rows[39].starts_with("KEYS Overview ▸"), "{}", rows[39]);
}

/// Large terminals add the selected bot's status and chat beside the tab.
#[test]
fn large_layout_adds_status_and_chat_beside_the_tab() {
    let mut app = fleet_app(&["alice", "bob"]);
    app.show_screen(Screen::Script);
    let rows = draw(&mut app, 200, 60);
    let side: Vec<String> = rows.iter().map(|r| r.chars().skip(154).collect()).collect();
    let side = text(&side);
    assert!(side.contains("status"), "status beside Script: {side}");
    assert!(side.contains("state: ingame scene 2"), "{side}");
    assert!(side.contains("chat"), "chat beside Script: {side}");
    let (_, header_y) = find(&rows, "sel   name").expect("fleet column header");
    let header = &rows[usize::from(header_y)];
    assert!(
        header.contains("queue") && header.contains("script"),
        "wide fleet columns: {header}"
    );
}

/// After a resize every click target comes from the new layout: a stale
/// coordinate hits whatever is there now, never the old target.
#[test]
fn a_resize_recomputes_every_click_target() {
    let mut app = fleet_app(&["alice", "bob"]);
    let wide = draw(&mut app, 120, 40);
    let (old_x, old_y) = find(&wide, " Map F4 ").unwrap();
    let narrow = draw(&mut app, 80, 24);
    let (new_x, new_y) = find(&narrow, " Map ").unwrap();
    assert_ne!((old_x, old_y), (new_x, new_y), "the tab moved");
    assert_ne!(
        app.on_click(old_x + 1, old_y),
        AppAction::MapOpen,
        "a stale position does not open the map"
    );
    assert_eq!(app.on_click(new_x + 1, new_y), AppAction::MapOpen);

    // A long fleet keeps the cursor row on screen and clickable.
    let names: Vec<String> = (0..50).map(|i| format!("bot{i:02}")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut app = fleet_app(&refs);
    app.table
        .sync_with_ids(&app.names, &app.profile_ids, &app.fleet);
    app.table.cursor_to(40, &app.names);
    let wide = draw(&mut app, 120, 40);
    let (_, wide_y) = find(&wide, "> bot40").expect("cursor row at 120x40");
    let narrow = draw(&mut app, 80, 24);
    let (x, y) = find(&narrow, "> bot40").expect("cursor row after the resize");
    assert_ne!(wide_y, y, "the window scrolled for the smaller pane");
    assert_eq!(app.on_click(x + 3, y), AppAction::Focus("bot40".into()));
}

/// The footer always names where the keys go: pane, text field, popup or
/// overlay.
#[test]
fn the_footer_always_names_the_keyboard_scope() {
    let mut app = fleet_app(&["alice"]);
    app.world = Some(std::sync::Arc::new(nav::world::NavWorld::from_grid(
        &nav::grid::StepGrid::fixture_open_3x3(),
    )));
    let footer = |app: &mut crate::app::TuiApp| draw(app, 80, 24)[23].clone();
    assert!(footer(&mut app).starts_with("KEYS Fleet ▸"));
    app.on_key(ch('/'));
    assert!(footer(&mut app).starts_with("KEYS Fleet filter ▸"));
    app.on_key(key(KeyCode::Esc));
    app.on_key(key(KeyCode::F(4)));
    assert!(footer(&mut app).starts_with("KEYS Map ▸"));
    app.on_key(ch('/'));
    assert!(footer(&mut app).starts_with("KEYS Map search ▸"));
    app.on_key(key(KeyCode::Esc));
    app.on_key(ch(':'));
    assert!(footer(&mut app).starts_with("KEYS Command palette ▸"));
    app.on_key(key(KeyCode::Esc));
    app.on_key(key(KeyCode::F(1)));
    assert!(footer(&mut app).starts_with("KEYS Help ▸"));
    app.on_key(key(KeyCode::Esc));
    app.on_key(key(KeyCode::F(3)));
    app.on_key(ch('w'));
    assert!(footer(&mut app).starts_with("KEYS Manual walk ▸"));
    assert_eq!(app.key_focus, Pane::Detail);
}
