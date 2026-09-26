use crossterm::event::{KeyCode, MouseEventKind};

use api::snapshot::ChatOptionView;
use script::{ScriptSel, ScriptSource};

use crate::app::{AppAction, TuiApp};
use crate::commands::Command;
use crate::help::help_rows;
use crate::layout::{Pane, Screen};
use crate::overlay::{ConfirmKind, Modal};
use crate::test_support::{ch, ctrl, draw, find, fleet_app, key, mouse, ready, right_click, text};

fn names(app: &TuiApp) -> Vec<&str> {
    app.names.iter().map(String::as_str).collect()
}

/// Moving the fleet cursor never changes the selected bot; Enter does.
#[test]
fn the_fleet_cursor_never_changes_the_selected_bot_until_enter() {
    let mut app = fleet_app(&["alice", "bob", "carol"]);
    draw(&mut app, 120, 40);
    assert_eq!(app.key_focus, Pane::Fleet);
    for _ in 0..2 {
        assert_eq!(app.on_key(key(KeyCode::Down)), AppAction::None);
        assert_eq!(app.focused_name().as_deref(), Some("alice"));
    }
    let rows = draw(&mut app, 120, 40);
    assert!(
        find(&rows, "> *").is_none(),
        "the cursor left the selected row"
    );
    assert!(
        find(&rows, "[ ] > carol").is_some(),
        "cursor on carol: {}",
        text(&rows)
    );
    assert_eq!(
        app.on_key(key(KeyCode::Enter)),
        AppAction::Focus("carol".into())
    );
    assert_eq!(app.focused_name().as_deref(), Some("carol"));
}

/// Space selects rows for group actions ("selected N of M") without
/// selecting the bot.
#[test]
fn space_selects_rows_without_selecting_the_bot() {
    let mut app = fleet_app(&["alice", "bob", "carol"]);
    draw(&mut app, 120, 40);
    app.on_key(key(KeyCode::Down));
    assert_eq!(app.on_key(ch(' ')), AppAction::None);
    app.on_key(key(KeyCode::Down));
    assert_eq!(app.on_key(ch(' ')), AppAction::None);
    assert_eq!(app.focused_name().as_deref(), Some("alice"));
    let all = text(&draw(&mut app, 120, 40));
    assert!(all.contains("selected 2 of 3"), "{all}");
    assert!(all.contains("[x]"), "{all}");
    assert_eq!(app.on_key(ch(' ')), AppAction::None);
    assert!(text(&draw(&mut app, 120, 40)).contains("selected 1 of 3"));
}

/// Tab only moves keyboard focus; it never selects another bot.
#[test]
fn tab_moves_keyboard_focus_between_panes_only() {
    let mut app = fleet_app(&["alice", "bob"]);
    draw(&mut app, 120, 40);
    let order: Vec<Pane> = (0..3)
        .map(|_| {
            assert_eq!(app.on_key(key(KeyCode::Tab)), AppAction::None);
            app.key_focus
        })
        .collect();
    assert_eq!(order, [Pane::Detail, Pane::Drawer, Pane::Fleet]);
    assert_eq!(app.on_key(key(KeyCode::BackTab)), AppAction::None);
    assert_eq!(app.key_focus, Pane::Drawer);
    assert_eq!(app.focused_name().as_deref(), Some("alice"));

    // At 80x24 there is no drawer to focus: Tab swaps fleet and tab.
    draw(&mut app, 80, 24);
    assert_eq!(
        app.key_focus,
        Pane::Detail,
        "a hidden drawer gives focus back"
    );
    app.on_key(key(KeyCode::Tab));
    assert_eq!(app.key_focus, Pane::Fleet);
    draw(&mut app, 80, 24);
    app.on_key(key(KeyCode::Tab));
    assert_eq!(app.key_focus, Pane::Detail);
}

/// No letter is global: pane shortcuts act only where they are shown.
#[test]
fn pane_letters_act_only_in_their_pane() {
    let mut app = fleet_app(&["alice"]);
    app.script_sel = Some(ScriptSel::Loaded(ScriptSource::File, "thiever".into()));
    for c in ['i', 'u', 'x', 't', 'e', 'w', 'o', 'l', 'p', 'b'] {
        assert_eq!(app.on_key(ch(c)), AppAction::None, "{c} in the fleet pane");
        assert!(app.modal.is_none() && !app.settings_state.open && !app.loadouts_state.open);
    }
    assert_eq!(app.on_key(key(KeyCode::F(3))), AppAction::None);
    assert_eq!(app.on_key(ch('i')), AppAction::Login, "i on the Overview");
    assert_eq!(app.on_key(ch('u')), AppAction::Logout);
    assert_eq!(app.on_key(ch('t')), AppAction::None, "t is a Script key");
    assert_eq!(app.on_key(key(KeyCode::F(5))), AppAction::None);
    assert_eq!(app.on_key(ch('i')), AppAction::None, "i is an Overview key");
    assert_eq!(
        app.on_key(ch('t')),
        AppAction::ScriptStart(ScriptSel::Loaded(ScriptSource::File, "thiever".into()))
    );
}

/// Typing into a text field wins over every global key.
#[test]
fn text_fields_take_typing_before_global_keys() {
    let mut app = fleet_app(&["alice", "quinn"]);
    draw(&mut app, 120, 40);
    assert_eq!(app.on_key(ch('/')), AppAction::None);
    for c in "q?:".chars() {
        assert_eq!(app.on_key(ch(c)), AppAction::None);
    }
    assert_eq!(app.table.filter, "q?:");
    assert!(!app.quit && app.modal.is_none(), "no quit, help or palette");
    assert!(text(&draw(&mut app, 120, 40)).contains("KEYS Fleet filter"));
    for _ in 0..2 {
        app.on_key(key(KeyCode::Backspace));
    }
    assert_eq!(app.table.filter, "q");
    let all = text(&draw(&mut app, 120, 40));
    assert!(all.contains("1/2 shown"), "filter keeps quinn only: {all}");
    assert_eq!(app.on_key(key(KeyCode::Esc)), AppAction::None);
    assert!(app.table.filter.is_empty() && !app.table.editing);

    // Log search on the Logs tab.
    assert_eq!(app.on_key(key(KeyCode::F(7))), AppAction::None);
    app.on_key(ch('/'));
    app.on_key(ch('q'));
    assert_eq!(app.log.search, "q");
    assert!(!app.quit);
}

/// Quitting asks first while bots are loaded; with none it just quits.
#[test]
fn quit_asks_first_while_bots_are_loaded() {
    let mut app = fleet_app(&["alice"]);
    assert_eq!(app.on_key(ctrl('q')), AppAction::None);
    assert!(matches!(
        &app.modal,
        Some(Modal::Confirm(c)) if c.kind == ConfirmKind::Quit
    ));
    assert!(!app.quit);
    assert_eq!(app.on_key(ch('n')), AppAction::None);
    assert!(app.modal.is_none());
    assert_eq!(app.on_key(ch('q')), AppAction::None, "q asks too");
    assert_eq!(app.on_key(key(KeyCode::Enter)), AppAction::Quit);
    assert!(app.quit);

    let mut empty = TuiApp::new("tui");
    assert_eq!(empty.on_key(ch('q')), AppAction::Quit);
}

/// An open overlay takes every key and click; a click outside a menu
/// only closes it and never reaches the bot behind.
#[test]
fn overlays_take_every_key_and_click() {
    let mut app = fleet_app(&["alice", "bob"]);
    let rows = draw(&mut app, 120, 40);
    let (bob_x, bob_y) = find(&rows, "bob").unwrap();
    assert_eq!(app.on_key(ch(':')), AppAction::None);
    for c in "qx".chars() {
        assert_eq!(app.on_key(ch(c)), AppAction::None);
    }
    assert!(matches!(&app.modal, Some(Modal::Palette(p)) if p.query == "qx"));
    assert!(!app.quit);
    let all = text(&draw(&mut app, 120, 40));
    assert!(all.contains("KEYS Command palette"), "{all}");
    assert_eq!(app.on_click(bob_x, bob_y), AppAction::None);
    assert!(app.modal.is_none(), "a click outside closes the palette");
    assert_eq!(
        app.focused_name().as_deref(),
        Some("alice"),
        "the click did not select the bot behind it"
    );

    // A confirmation swallows clicks behind it and stays open.
    app.confirm(ConfirmKind::Remove("alice".into()));
    draw(&mut app, 120, 40);
    assert_eq!(app.on_click(bob_x, bob_y), AppAction::None);
    assert!(matches!(app.modal, Some(Modal::Confirm(_))));
    assert_eq!(app.focused_name().as_deref(), Some("alice"));
    assert_eq!(app.on_key(key(KeyCode::Esc)), AppAction::None);
    assert!(app.modal.is_none());
}

/// The palette shows scope and why a command cannot run, and runs it
/// once it can.
#[test]
fn palette_explains_unavailable_commands_and_runs_available_ones() {
    let mut app = fleet_app(&["alice"]);
    app.focused = None;
    app.on_key(ctrl('p'));
    for c in "log in".chars() {
        app.on_key(ch(c));
    }
    let all = text(&draw(&mut app, 120, 40));
    assert!(all.contains("Log in"), "{all}");
    assert!(all.contains("no bot selected"), "reason shown: {all}");
    assert_eq!(app.on_key(key(KeyCode::Enter)), AppAction::None);
    assert!(
        matches!(&app.modal, Some(Modal::Palette(p)) if p.note.is_some()),
        "Enter on an unavailable command keeps the palette with its reason"
    );
    app.on_key(key(KeyCode::Esc));

    app.focused = Some(0);
    app.on_key(ch(':'));
    for c in "log in".chars() {
        app.on_key(ch(c));
    }
    let all = text(&draw(&mut app, 120, 40));
    assert!(all.contains("BOT alice"), "scope names the target: {all}");
    assert_eq!(app.on_key(key(KeyCode::Enter)), AppAction::Login);
    assert!(app.modal.is_none());
}

/// Help starts with the focused pane's keys and filters as you type.
#[test]
fn help_lists_the_focused_pane_first_and_filters() {
    let mut app = fleet_app(&["alice"]);
    app.on_key(key(KeyCode::F(5)));
    assert_eq!(app.on_key(ch('?')), AppAction::None);
    assert_eq!(help_rows(&app)[0].context, "Script");
    for c in "reload".chars() {
        app.on_key(ch(c));
    }
    let all = text(&draw(&mut app, 120, 40));
    assert!(all.contains("Reload"), "{all}");
    assert!(!all.contains("dots, collision"), "filtered out: {all}");
    assert_eq!(app.on_key(key(KeyCode::Esc)), AppAction::None);
    assert!(app.modal.is_none());
}

/// Remove asks first and removes the member it named, even if the
/// selection changes before the operator confirms.
#[test]
fn remove_confirms_a_frozen_target() {
    let mut app = fleet_app(&["alice", "bob"]);
    app.on_key(key(KeyCode::F(3)));
    assert_eq!(app.on_key(ch('x')), AppAction::None);
    let all = text(&draw(&mut app, 120, 40));
    assert!(all.contains("Remove BOT alice from the fleet?"), "{all}");
    assert!(all.contains("not Delete profile"), "{all}");
    app.focused = Some(1); // the next pump selected bob
    assert_eq!(
        app.on_key(key(KeyCode::Enter)),
        AppAction::Remove("alice".into())
    );
}

/// Fleet-wide commands confirm the frozen membership and refuse to run
/// over a fleet that changed since.
#[test]
fn bulk_commands_confirm_a_frozen_scope() {
    let mut app = fleet_app(&["alice", "bob"]);
    assert_eq!(app.on_key(ch('m')), AppAction::None);
    let all = text(&draw(&mut app, 120, 40));
    assert!(all.contains("Members now (2): alice, bob"), "{all}");
    app.names.push("carol".into());
    assert_eq!(
        app.on_key(key(KeyCode::Enter)),
        AppAction::None,
        "a changed fleet is shown again, not run"
    );
    let all = text(&draw(&mut app, 120, 40));
    assert!(all.contains("alice, bob, carol"), "{all}");
    assert!(all.contains("The fleet changed"), "{all}");
    assert_eq!(app.on_key(key(KeyCode::Enter)), AppAction::SpawnAll);

    assert_eq!(app.on_key(ch('U')), AppAction::None);
    assert_eq!(
        app.on_key(key(KeyCode::Esc)),
        AppAction::None,
        "Esc cancels"
    );
    assert!(app.modal.is_none());
    assert_eq!(app.on_key(ch('U')), AppAction::None);
    assert_eq!(app.on_key(ch('y')), AppAction::LogoutAll);
}

/// An NPC dialogue is visible everywhere but only the Chat tab answers it.
#[test]
fn a_dialogue_never_takes_keys_from_other_panes() {
    let mut app = fleet_app(&["alice", "bob"]);
    app.chat_data.modal_texts = vec!["Which way?".into()];
    app.chat_data.options = vec![ChatOptionView {
        component_id: 1,
        text: "Yes".into(),
    }];
    let rows = draw(&mut app, 120, 40);
    let all = text(&rows);
    assert!(
        all.contains("Chat!"),
        "the Chat tab flags the dialogue: {all}"
    );
    assert!(all.contains("DIALOGUE open (Chat F6)"), "{all}");
    app.on_key(key(KeyCode::Down));
    assert_eq!(
        app.on_key(key(KeyCode::Enter)),
        AppAction::Focus("bob".into()),
        "Enter in the fleet selects; it does not answer"
    );
    app.on_key(key(KeyCode::F(6)));
    assert_eq!(
        app.on_key(key(KeyCode::Enter)),
        AppAction::Chat(crate::chat::ChatAction::Answer(1))
    );
}

/// Left click selects (or ticks the box), right click only opens a menu.
#[test]
fn clicks_select_rows_and_right_click_opens_a_menu() {
    let mut app = fleet_app(&["alice", "bob", "carol"]);
    let rows = draw(&mut app, 120, 40);
    let (x, y) = find(&rows, "bob").unwrap();
    assert_eq!(app.on_click(x, y), AppAction::Focus("bob".into()));
    assert_eq!(app.key_focus, Pane::Fleet);
    let (_, carol_y) = find(&rows, "carol").unwrap();
    assert_eq!(
        app.on_click(2, carol_y),
        AppAction::None,
        "the checkbox column"
    );
    assert!(app.table.is_marked("carol"));
    assert_eq!(app.focused_name().as_deref(), Some("bob"));

    let rows = draw(&mut app, 120, 40);
    let (ax, ay) = find(&rows, "[ ]   alice").unwrap();
    assert_eq!(app.on_mouse(right_click(ax, ay)), AppAction::None);
    assert_eq!(
        app.focused_name().as_deref(),
        Some("bob"),
        "not a left click"
    );
    let rows = draw(&mut app, 120, 40);
    assert!(text(&rows).contains("Map of alice"), "{}", text(&rows));
    let (mx, my) = find(&rows, "Map of alice").unwrap();
    assert_eq!(
        app.on_click(mx, my),
        AppAction::Batch(vec![AppAction::Focus("alice".into()), AppAction::MapOpen]),
        "the menu item selects alice and opens her map"
    );
    assert_eq!(app.screen, Screen::Map);
}

/// The wheel scrolls the list, log or map under the pointer.
#[test]
fn the_wheel_scrolls_the_pane_under_the_pointer() {
    let names: Vec<String> = (0..40).map(|i| format!("bot{i:02}")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut app = fleet_app(&refs);
    draw(&mut app, 120, 40);
    let rows = app.regions.fleet_rows;
    app.on_mouse(mouse(MouseEventKind::ScrollDown, rows.x + 2, rows.y));
    assert_eq!(app.table.cursor, 3);
    assert_eq!(
        app.focused_name().as_deref(),
        Some("bot00"),
        "scrolling selects nothing"
    );

    for i in 0..30 {
        frontend_core::log::global().slot_line(
            "bot00",
            frontend_core::log::Source::Script,
            frontend_core::log::Level::Info,
            &format!("wheel line {i}"),
        );
    }
    draw(&mut app, 120, 40);
    let log = app.regions.log_rows;
    app.on_mouse(mouse(MouseEventKind::ScrollUp, log.x + 2, log.y));
    assert!(
        !app.log.view.follow,
        "wheel up over the drawer pauses follow"
    );
    assert_eq!(app.log.scroll, 3);

    app.world = Some(std::sync::Arc::new(nav::world::NavWorld::from_grid(
        &nav::grid::StepGrid::fixture_open_3x3(),
    )));
    app.on_key(key(KeyCode::F(4)));
    draw(&mut app, 120, 40);
    let map = app.regions.map;
    let before = app.map.pan;
    app.on_mouse(mouse(MouseEventKind::ScrollUp, map.x + 2, map.y + 2));
    assert_eq!(app.map.pan.1, before.1 + 1, "wheel up pans north");
}

/// A map click selects the tile; walking it takes a second, explicit
/// action (Enter or the Walk button).
#[test]
fn a_map_click_selects_and_a_second_action_walks() {
    let mut app = fleet_app(&["alice"]);
    app.world = Some(std::sync::Arc::new(nav::world::NavWorld::from_grid(
        &nav::grid::StepGrid::fixture_open_3x3(),
    )));
    app.here = Some(api::snapshot::WorldTile {
        x: 1,
        z: 1,
        level: 0,
    });
    app.on_key(key(KeyCode::F(4)));
    let rows = draw(&mut app, 120, 40);
    let (x, y) = find(&rows, "@").expect("here marker");
    assert_eq!(app.on_click(x, y), AppAction::None);
    let requested = app
        .map_model
        .pending()
        .expect("the click selected a tile")
        .requested;
    assert_eq!((requested.x, requested.z), (1, 1));
    let rows = draw(&mut app, 120, 40);
    let (wx, wy) = find(&rows, "[Walk]").expect("walk button");
    assert_eq!(app.on_click(wx + 1, wy), AppAction::ArmWalk(requested));
}

/// Mouse capture can be turned off (for terminal copy) from the palette.
#[test]
fn mouse_capture_toggles_from_the_palette() {
    let mut app = fleet_app(&["alice"]);
    app.on_key(ch(':'));
    for c in "mouse".chars() {
        app.on_key(ch(c));
    }
    assert_eq!(
        app.on_key(key(KeyCode::Enter)),
        AppAction::MouseCapture(false)
    );
    assert!(text(&draw(&mut app, 120, 40)).contains("mouse off"));
    assert_eq!(
        app.run_command(Command::ToggleMouse),
        AppAction::MouseCapture(true)
    );
}

/// Selecting through the fleet and switching tabs never spawns, logs in
/// or starts anything: only the explicit commands do.
#[test]
fn switching_tabs_and_selecting_is_always_safe() {
    let mut app = fleet_app(&["alice", "bob"]);
    app.script_sel = Some(ScriptSel::Loaded(ScriptSource::File, "thiever".into()));
    let mut seen = Vec::new();
    for k in [
        key(KeyCode::Down),
        key(KeyCode::Enter),
        key(KeyCode::F(3)),
        key(KeyCode::F(5)),
        key(KeyCode::F(6)),
        key(KeyCode::F(7)),
        key(KeyCode::F(2)),
        key(KeyCode::Tab),
        key(KeyCode::BackTab),
    ] {
        seen.push(app.on_key(k));
    }
    assert!(
        seen.iter()
            .all(|a| matches!(a, AppAction::None | AppAction::Focus(_))),
        "{seen:?}"
    );
    assert_eq!(names(&app), ["alice", "bob"]);
}

/// A member that has not published a status yet reads "offline"; the
/// header counts only what the rows say.
#[test]
fn header_counts_follow_the_rows() {
    let mut app = fleet_app(&["alice", "bob", "carol"]);
    app.statuses = vec![ready("alice", 2)];
    let rows = draw(&mut app, 120, 40);
    assert!(
        rows[0].contains("loaded 3 ready 1 queued 0 failed 0"),
        "{}",
        rows[0]
    );
    assert!(text(&rows).contains("offline"));
}
