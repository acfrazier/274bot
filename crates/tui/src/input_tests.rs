use crossterm::event::{KeyCode, MouseEventKind};

use api::snapshot::ChatOptionView;
use script::{ScriptSel, ScriptSource};

use crate::app::{AppAction, TuiApp};
use crate::commands::Command;
use crate::help::help_rows;
use crate::layout::{Pane, Screen};
use crate::overlay::{ConfirmKind, Modal};
use crate::test_support::{ch, ctrl, draw, find, fleet_app, key, mouse, right_click, text};

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

#[test]
fn uid_marks_survive_cursor_moves_and_filter_keys() {
    let mut app = fleet_app(&["alice", "bob", "carol"]);
    app.profile_ids = [
        frontend_core::ProfileIdentity::uid(41),
        frontend_core::ProfileIdentity::uid(42),
        frontend_core::ProfileIdentity::uid(43),
    ]
    .into();
    draw(&mut app, 120, 40);

    // Space follows the production fleet_key path, including its identity
    // projection before each key.
    app.on_key(ch(' '));
    app.on_key(key(KeyCode::Down));
    app.on_key(ch(' '));
    let alice = frontend_core::ProfileIdentity::uid(41);
    let bob = frontend_core::ProfileIdentity::uid(42);
    assert!(app.table.selection.contains(alice));
    assert!(app.table.selection.contains(bob));

    // A cursor move and a filter character both re-project the table.
    app.on_key(key(KeyCode::Down));
    app.on_key(ch('/'));
    app.on_key(ch('b'));
    assert!(app.table.selection.contains(alice));
    assert!(app.table.selection.contains(bob));

    // The filtered, marked row still toggles off through fleet_key.
    app.on_key(key(KeyCode::Enter));
    app.on_key(ch(' '));
    assert!(app.table.selection.contains(alice));
    assert!(!app.table.selection.contains(bob));
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

#[test]
fn marked_bulk_confirms_marked_scope_without_unmarked_churn_loop() {
    let mut app = fleet_app(&["alice", "bob"]);
    app.table
        .selection
        .set(app.profile_id_for_name("alice"), true);
    assert_eq!(app.run_command(Command::ScriptStartAll), AppAction::None);
    let all = text(&draw(&mut app, 120, 40));
    assert!(
        all.contains("Start 1 marked bot on their last successful script"),
        "{all}"
    );
    assert!(!all.contains("Start all 1 member"), "{all}");

    // An unmarked member arriving does not change the frozen marked scope.
    app.names.push("carol".into());
    assert_eq!(app.on_key(key(KeyCode::Enter)), AppAction::ScriptStartAll);
}

#[test]
fn marked_start_confirm_names_the_selected_script_it_will_run() {
    let mut app = fleet_app(&["alice", "bob"]);
    app.table
        .selection
        .set(app.profile_id_for_name("alice"), true);
    app.script_sel = Some(ScriptSel::Loaded(ScriptSource::File, "thiever".into()));
    assert_eq!(app.run_command(Command::ScriptStartAll), AppAction::None);
    let all = text(&draw(&mut app, 120, 40));
    let label = app.script_sel.as_ref().unwrap().label();
    assert!(
        all.contains(&format!("Start {label} on 1 marked bot")),
        "{all}"
    );
    assert!(!all.contains("last successful script"), "{all}");
}

/// With rows marked, Log in and Log out confirm the marked scope and run the
/// marked commands; with none marked they keep their whole-fleet meaning.
#[test]
fn marked_login_and_logout_confirm_the_marked_scope_and_run_marked_commands() {
    let mut app = fleet_app(&["alice", "bob", "carol"]);
    app.table
        .selection
        .set(app.profile_id_for_name("bob"), true);

    assert_eq!(app.on_key(ch('m')), AppAction::None);
    let all = text(&draw(&mut app, 120, 40));
    assert!(all.contains("Log in 1 marked bot"), "{all}");
    assert!(!all.contains("Load every vault profile"), "{all}");
    assert_eq!(app.on_key(key(KeyCode::Enter)), AppAction::LoginMarked);

    assert_eq!(app.on_key(ch('U')), AppAction::None);
    let all = text(&draw(&mut app, 120, 40));
    assert!(all.contains("Log out 1 marked bot"), "{all}");
    assert_eq!(app.on_key(ch('y')), AppAction::LogoutMarked);

    app.table.selection.clear();
    assert_eq!(app.on_key(ch('m')), AppAction::None);
    assert_eq!(app.on_key(key(KeyCode::Enter)), AppAction::SpawnAll);
    assert_eq!(app.on_key(ch('U')), AppAction::None);
    assert_eq!(app.on_key(ch('y')), AppAction::LogoutAll);
}

/// Assign and Assign & restart need marked rows and a selected script, say
/// which is missing, then confirm the marked scope and the script they use.
#[test]
fn marked_assign_commands_need_marks_and_a_script_and_name_both() {
    for command in [Command::ScriptAssignMarked, Command::ScriptRestartMarked] {
        let mut app = fleet_app(&["alice", "bob"]);
        assert_eq!(command.availability(&app), Err("mark fleet rows first"));
        app.table
            .selection
            .set(app.profile_id_for_name("alice"), true);
        assert_eq!(
            command.availability(&app),
            Err("browse to pick a script first")
        );
        app.script_sel = Some(ScriptSel::Loaded(ScriptSource::File, "thiever".into()));
        assert_eq!(command.availability(&app), Ok(()));

        assert_eq!(app.run_command(command), AppAction::None);
        let all = text(&draw(&mut app, 120, 40));
        let label = app.script_sel.as_ref().unwrap().label();
        assert!(all.contains(&label), "{all}");
        assert!(all.contains("1 marked bot"), "{all}");
        let expected = match command {
            Command::ScriptAssignMarked => AppAction::ScriptAssignMarked,
            _ => AppAction::ScriptRestartMarked,
        };
        assert_eq!(app.on_key(key(KeyCode::Enter)), expected);
    }
}

/// Apply focused bot's settings to marked needs marks, a focused bot and a
/// selected script (naming which is missing), and the 80x24 palette shows
/// its whole label with its scope beside it instead of cutting either.
#[test]
fn apply_settings_to_marked_names_what_is_missing_and_fits_the_palette() {
    let command = Command::ScriptApplyMarked;
    let mut app = fleet_app(&["alice", "bob"]);
    assert_eq!(command.availability(&app), Err("mark fleet rows first"));
    app.table
        .selection
        .set(app.profile_id_for_name("bob"), true);
    assert_eq!(
        command.availability(&app),
        Err("browse to pick a script first")
    );
    app.script_sel = Some(ScriptSel::Loaded(ScriptSource::File, "thiever".into()));
    app.focused = None;
    assert_eq!(
        command.availability(&app),
        Err("no bot selected (Fleet: Enter or click a row)")
    );
    app.focused = Some(0);
    assert_eq!(command.availability(&app), Ok(()));

    app.on_key(ctrl('p'));
    for c in "apply focused".chars() {
        app.on_key(ch(c));
    }
    let all = text(&draw(&mut app, 80, 24));
    assert!(
        all.contains("Apply focused bot's settings to marked… alice to 1 marked"),
        "{all}"
    );
    assert!(all.contains("1 commands"), "{all}");
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
    assert!(app
        .table
        .selection
        .contains(app.profile_id_for_name("carol")));
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScriptPopup {
    Browse,
    Load,
    Catalog,
}

const SCRIPT_POPUPS: [ScriptPopup; 3] =
    [ScriptPopup::Browse, ScriptPopup::Load, ScriptPopup::Catalog];

fn popup_open(app: &TuiApp, popup: ScriptPopup) -> bool {
    match popup {
        ScriptPopup::Browse => app.script_browse_open,
        ScriptPopup::Load => app.script_load_open,
        ScriptPopup::Catalog => app.rs2b0t_catalog_open,
    }
}

/// A folder with a sub folder and a script file for the Load and catalog
/// folder lists.
fn scratch_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "274bot-tui-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(dir.join("scripts")).unwrap();
    std::fs::write(dir.join("digbot.js"), "export default class T {}\n").unwrap();
    dir
}

fn browse_card(name: &str) -> crate::script_shape::BrowseCard {
    crate::script_shape::BrowseCard {
        name: name.into(),
        description: String::new(),
        category: "Skilling".into(),
        tags: Vec::new(),
        kind: script::ScriptKind::Compat,
        source: ScriptSource::File,
        unloadable: None,
    }
}

/// Open `popup` the way an operator does, from the Script tab.
fn open_popup(app: &mut TuiApp, popup: ScriptPopup, dir: &std::path::Path) {
    assert_eq!(app.on_key(key(KeyCode::F(5))), AppAction::None);
    match popup {
        ScriptPopup::Browse => assert_eq!(app.on_key(ch('b')), AppAction::ScriptBrowse),
        ScriptPopup::Load => {
            app.script_load_last_dir = Some(dir.to_path_buf());
            assert_eq!(app.on_key(ch('f')), AppAction::None);
        }
        ScriptPopup::Catalog => {
            app.rs2b0t_catalog_dir = dir.to_path_buf();
            app.on_key(ch(':'));
            for c in "import".chars() {
                app.on_key(ch(c));
            }
            assert_eq!(
                app.on_key(key(KeyCode::Enter)),
                AppAction::ScriptImportCatalog
            );
            // What tui-play's dispatch does with that action.
            app.rs2b0t_catalog_open = true;
            app.catalog_sel = 0;
        }
    }
    assert!(popup_open(app, popup), "{popup:?} opened");
}

/// Browse, the Load file browser and the catalog folder prompt own every
/// key while open: no global chord, no other pane's letter and no script
/// letter gets past them (the old router let `q` open the quit dialog over
/// Browse and `t` start a script from it). Only their own keys act; Esc
/// closes them. Ctrl-Q still asks to quit and cancelling returns to them.
#[test]
fn script_popups_own_every_key_until_closed() {
    let dir = scratch_dir("popup-keys");
    let leaks = [
        ch('q'),
        ch('?'),
        ch(':'),
        ctrl('p'),
        key(KeyCode::F(1)),
        key(KeyCode::F(2)),
        key(KeyCode::F(3)),
        key(KeyCode::F(4)),
        key(KeyCode::F(5)),
        key(KeyCode::F(6)),
        key(KeyCode::F(7)),
        key(KeyCode::Tab),
        key(KeyCode::BackTab),
        ch('t'),
        ch('e'),
        ch('T'),
        ch('E'),
        ch('b'),
        ch('f'),
        ch('v'),
        ch('R'),
        ch('m'),
        ch('i'),
        ch('x'),
    ];
    for popup in SCRIPT_POPUPS {
        let mut app = fleet_app(&["alice", "bob"]);
        app.script_sel = Some(ScriptSel::Loaded(ScriptSource::File, "Alpha".into()));
        app.script_cards = vec![browse_card("Alpha"), browse_card("Bravo")];
        open_popup(&mut app, popup, &dir);
        for k in leaks {
            assert_eq!(app.on_key(k), AppAction::None, "{popup:?}: {k:?}");
            assert!(popup_open(&app, popup), "{popup:?} stays open after {k:?}");
            assert!(
                app.modal.is_none(),
                "{popup:?}: {k:?} opened {:?}",
                app.modal
            );
            assert!(!app.quit);
            assert_eq!(app.screen, Screen::Script, "{popup:?}: {k:?}");
            assert_eq!(app.key_focus, Pane::Detail, "{popup:?}: {k:?}");
            assert!(!app.map_active && !app.settings_state.open && !app.loadouts_state.open);
        }
        let footer = draw(&mut app, 80, 24)[23].clone();
        let scope = match popup {
            ScriptPopup::Browse => "KEYS Script: Browse ▸",
            ScriptPopup::Load => "KEYS Script: load file ▸",
            ScriptPopup::Catalog => "KEYS Script: catalog folder ▸",
        };
        assert!(footer.starts_with(scope), "{footer}");

        assert_eq!(app.on_key(ctrl('q')), AppAction::None);
        assert!(matches!(
            &app.modal,
            Some(Modal::Confirm(c)) if c.kind == ConfirmKind::Quit
        ));
        assert_eq!(app.on_key(ch('n')), AppAction::None);
        assert!(
            popup_open(&app, popup),
            "{popup:?} is still there after the quit dialog"
        );

        let esc = app.on_key(key(KeyCode::Esc));
        if popup == ScriptPopup::Catalog {
            assert_eq!(
                esc,
                AppAction::ScriptDeferCatalog,
                "Esc on the prompt is Not now"
            );
        } else {
            assert_eq!(esc, AppAction::None);
            assert!(!popup_open(&app, popup), "{popup:?}: Esc closes it");
            assert_eq!(app.on_key(ch('q')), AppAction::None);
            assert!(
                matches!(&app.modal, Some(Modal::Confirm(c)) if c.kind == ConfirmKind::Quit),
                "{popup:?}: once closed, q asks to quit again"
            );
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// A click outside an open Script popup reaches nothing behind it: Browse
/// and Load close (the pick stays), the catalog prompt stays open because
/// dismissing it means Not now. The wheel moves the popup's own list.
#[test]
fn script_popups_swallow_clicks_outside_themselves() {
    let dir = scratch_dir("popup-clicks");
    for popup in SCRIPT_POPUPS {
        let mut app = fleet_app(&["alice", "bob"]);
        app.script_cards = vec![browse_card("Alpha"), browse_card("Bravo")];
        open_popup(&mut app, popup, &dir);
        let rows = draw(&mut app, 120, 40);
        let (bob_x, bob_y) = find(&rows, "bob").expect("fleet row");
        let (map_x, map_y) = find(&rows, " Map F4 ").expect("map tab");

        let sel_before = app.script_sel.clone();
        let load_before = app.script_load_sel;
        let catalog_before = app.catalog_sel;
        let fleet = app.regions.fleet_rows;
        app.on_mouse(mouse(MouseEventKind::ScrollDown, fleet.x + 2, fleet.y));
        assert_eq!(
            app.table.cursor, 0,
            "{popup:?}: the wheel left the fleet alone"
        );
        match popup {
            ScriptPopup::Browse => assert_ne!(app.script_sel, sel_before),
            ScriptPopup::Load => assert!(app.script_load_sel > load_before),
            ScriptPopup::Catalog => assert!(app.catalog_sel > catalog_before),
        }

        assert_eq!(app.on_click(map_x + 1, map_y), AppAction::None, "{popup:?}");
        assert_eq!(
            app.screen,
            Screen::Script,
            "{popup:?}: the tab click did not switch"
        );
        assert!(!app.map_active);
        if popup != ScriptPopup::Catalog {
            assert!(
                !popup_open(&app, popup),
                "{popup:?}: an outside click closes it"
            );
            open_popup(&mut app, popup, &dir);
            draw(&mut app, 120, 40);
        }
        assert_eq!(app.on_click(bob_x, bob_y), AppAction::None, "{popup:?}");
        assert_eq!(
            app.focused_name().as_deref(),
            Some("alice"),
            "{popup:?}: the click never selected the bot behind it"
        );
        assert_eq!(popup_open(&app, popup), popup == ScriptPopup::Catalog);
        assert!(app.modal.is_none());
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn settings_r_key_requests_a_memory_relog_for_the_bound_member() {
    let mut app = TuiApp::new("tui");
    app.settings_state.open = true;
    app.settings_profile = Some("alice".into());
    assert_eq!(
        app.on_key(ch('r')),
        AppAction::MemoryRelog("alice".into()),
        "r in settings relogs the bound member through the FIFO"
    );
}

#[test]
fn confirming_a_memory_relog_dispatches_it() {
    let mut app = TuiApp::new("tui");
    app.confirm(ConfirmKind::MemoryRelog("alice".into()));
    assert_eq!(
        app.on_key(ch('y')),
        AppAction::MemoryRelogNow("alice".into()),
        "the confirmed relog runs"
    );
}
