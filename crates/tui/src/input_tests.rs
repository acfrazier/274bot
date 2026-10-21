use crossterm::event::{KeyCode, MouseEventKind};

use api::snapshot::ChatOptionView;
use frontend_core::scripts::{CardRuntime, CardSource, ScriptChoice};

use crate::app::{AppAction, TuiApp};
use crate::bin::seam::Seam;
use crate::commands::Command;
use crate::fleet::FilterEdit;
use crate::help::help_rows;
use crate::layout::{Pane, Screen};
use crate::overlay::{ConfirmKind, Modal};
use crate::test_support::{ch, ctrl, draw, find, fleet_app, key, mouse, right_click, text};
use frontend_core::{FleetAction, FleetCard};

fn names(app: &TuiApp) -> Vec<&str> {
    app.names.iter().map(String::as_str).collect()
}

#[test]
fn overview_offers_memory_relog_after_leaving_settings_until_login_or_queue() {
    let mut app = fleet_app(&["alice", "bob"]);
    app.show_screen(Screen::Overview);
    app.memory = Some(frontend_core::MemoryNotice {
        login_lowmem: true,
        desired_lowmem: false,
        relog_pending: false,
        connected: true,
        login_applies_memory: true,
    });
    assert_eq!(app.on_key(ch('r')), AppAction::MemoryRelog("alice".into()));
    assert!(app.key_hints().contains("r Relog now"));
    app.memory.as_mut().unwrap().relog_pending = true;
    assert_eq!(
        app.on_key(ch('r')),
        AppAction::None,
        "queued relogs cannot be repeated"
    );
    assert!(!app.key_hints().contains("r Relog now"));
    app.memory.as_mut().unwrap().relog_pending = false;
    app.memory.as_mut().unwrap().login_lowmem = false;
    assert_eq!(
        app.on_key(ch('r')),
        AppAction::None,
        "the applied mode needs no relog"
    );
    assert!(!app.key_hints().contains("r Relog now"));
    app.memory.as_mut().unwrap().login_lowmem = true;
    app.memory.as_mut().unwrap().connected = false;
    assert_eq!(
        app.on_key(ch('r')),
        AppAction::None,
        "logged-out slots apply their queued mode with Log in, not Relog now"
    );
    assert!(!app.key_hints().contains("r Relog now"));
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
    let mut seam = Seam::new();
    let mut app = fleet_app(&["alice", "bob", "carol"]);
    draw(&mut app, 120, 40);
    app.on_key(key(KeyCode::Down));
    seam.press(&mut app, ch(' '), AppAction::SetMark("bob".into(), true));
    app.on_key(key(KeyCode::Down));
    seam.press(&mut app, ch(' '), AppAction::SetMark("carol".into(), true));
    assert_eq!(app.focused_name().as_deref(), Some("alice"));
    let all = text(&draw(&mut app, 120, 40));
    assert!(all.contains("marked 2"), "{all}");
    assert!(all.contains("[x]"), "{all}");
    seam.press(&mut app, ch(' '), AppAction::SetMark("carol".into(), false));
    assert!(text(&draw(&mut app, 120, 40)).contains("marked 1"));
}

#[test]
fn uid_marks_survive_cursor_moves_and_filter_keys() {
    // The vault knows the members by these uids, as the production session
    // does: `SetMark` names a member and the core resolves its identity.
    let mut seam = Seam::with_profiles(&[("alice", 41), ("bob", 42), ("carol", 43)]);
    let mut app = fleet_app(&["alice", "bob", "carol"]);
    crate::test_support::set_profile_ids(
        &mut app,
        &[
            frontend_core::ProfileIdentity::uid(41),
            frontend_core::ProfileIdentity::uid(42),
            frontend_core::ProfileIdentity::uid(43),
        ],
    );
    draw(&mut app, 120, 40);

    // Space follows the production fleet_key path, including its identity
    // projection before each key.
    seam.press(&mut app, ch(' '), AppAction::SetMark("alice".into(), true));
    app.on_key(key(KeyCode::Down));
    seam.press(&mut app, ch(' '), AppAction::SetMark("bob".into(), true));
    let alice = frontend_core::ProfileIdentity::uid(41);
    let bob = frontend_core::ProfileIdentity::uid(42);
    assert!(app.fleet_model.marks().contains(alice));
    assert!(app.fleet_model.marks().contains(bob));

    // A cursor move and a filter character both re-project the table.
    app.on_key(key(KeyCode::Down));
    app.on_key(ch('/'));
    seam.press(
        &mut app,
        ch('b'),
        AppAction::FleetFilter(FilterEdit::Push('b')),
    );
    assert!(app.fleet_model.marks().contains(alice));
    assert!(app.fleet_model.marks().contains(bob));

    // The filtered, marked row still toggles off through fleet_key.
    app.on_key(key(KeyCode::Enter));
    seam.press(&mut app, ch(' '), AppAction::SetMark("bob".into(), false));
    assert!(app.fleet_model.marks().contains(alice));
    assert!(!app.fleet_model.marks().contains(bob));
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
    app.script_sel = Some(ScriptChoice::loaded(CardSource::File, "thiever"));
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
        AppAction::ScriptStart(ScriptChoice::loaded(CardSource::File, "thiever"))
    );
}

/// Typing into a text field wins over every global key.
#[test]
fn text_fields_take_typing_before_global_keys() {
    let mut seam = Seam::new();
    let mut app = fleet_app(&["alice", "quinn"]);
    draw(&mut app, 120, 40);
    assert_eq!(app.on_key(ch('/')), AppAction::None);
    for c in "q?:".chars() {
        seam.press(&mut app, ch(c), AppAction::FleetFilter(FilterEdit::Push(c)));
    }
    assert_eq!(app.fleet_model.filter().text(), "q?:");
    assert!(!app.quit && app.modal.is_none(), "no quit, help or palette");
    assert!(text(&draw(&mut app, 120, 40)).contains("KEYS Fleet filter"));
    for _ in 0..2 {
        seam.send(&mut app, key(KeyCode::Backspace));
    }
    assert_eq!(app.fleet_model.filter().text(), "q");
    let all = text(&draw(&mut app, 120, 40));
    assert!(all.contains("1/2 shown"), "filter keeps quinn only: {all}");
    seam.press(
        &mut app,
        key(KeyCode::Esc),
        AppAction::FleetFilter(FilterEdit::Clear),
    );
    assert!(app.fleet_model.filter().text().is_empty() && !app.table.editing);

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

/// Help lists the card picker and the shown-row mark keys, with the same
/// labels the palette shows, and finds them by search.
#[test]
fn help_lists_the_card_picker_and_the_shown_row_mark_keys() {
    let mut app = fleet_app(&["alice"]);
    let rows = help_rows(&app);
    let what = |keys: &str| {
        rows.iter()
            .find(|row| row.context == "Fleet" && row.keys == keys)
            .map(|row| row.what.as_str())
    };
    assert_eq!(what("c"), Some("Pick fleet card…"));
    assert_eq!(what("a"), Some("Mark shown rows"));
    assert_eq!(what("A"), Some("Unmark shown rows"));
    assert_eq!(what("m"), Some("Load + log in fleet"));
    assert_eq!(what("U"), Some("Log out fleet"));

    app.on_key(key(KeyCode::F(1)));
    for c in "fleet card".chars() {
        app.on_key(ch(c));
    }
    let shown = text(&draw(&mut app, 120, 40));
    assert!(shown.contains("Pick fleet card…"), "{shown}");
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

/// The fleet keys hand the action to core as a plan. The front end keeps
/// no scope, membership snapshot or confirmation rule of its own: it opens
/// no overlay until core says the plan asks first.
#[test]
fn fleet_action_keys_ask_core_for_a_plan() {
    let mut app = fleet_app(&["alice", "bob"]);
    assert_eq!(
        app.on_key(ch('m')),
        AppAction::PlanFleet(FleetAction::Login)
    );
    assert_eq!(
        app.on_key(ch('U')),
        AppAction::PlanFleet(FleetAction::Logout)
    );
    assert!(app.modal.is_none(), "core decides whether to ask");

    // The Script tab's bulk keys are the same plans.
    app.on_key(key(KeyCode::F(5)));
    assert_eq!(
        app.on_key(ch('T')),
        AppAction::PlanFleet(FleetAction::Start)
    );
    assert_eq!(app.on_key(ch('E')), AppAction::PlanFleet(FleetAction::Stop));
    assert!(app.modal.is_none());
}

/// The confirm overlay shows the plan's prompt as core wrote it. Enter or
/// `y` runs the plan; Esc or `n` cancels it, with no overlay left behind.
#[test]
fn the_confirm_overlay_shows_the_plan_prompt_verbatim() {
    let prompt = "Stop 2 running bots: alice, bob. 1 marked bot is idle and is skipped.";
    for (press, expected) in [
        (key(KeyCode::Enter), AppAction::RunFleetPlan),
        (ch('y'), AppAction::RunFleetPlan),
        (key(KeyCode::Esc), AppAction::CancelFleetPlan),
        (ch('n'), AppAction::CancelFleetPlan),
    ] {
        let mut app = fleet_app(&["alice", "bob"]);
        app.confirm(ConfirmKind::FleetPlan {
            action: FleetAction::Stop,
            prompt: prompt.into(),
        });
        let all = text(&draw(&mut app, 120, 40));
        assert!(all.contains(prompt), "{all}");
        assert!(all.contains("Stop"), "the action titles the overlay: {all}");
        assert!(
            all.contains("[Run y]") && all.contains("[Cancel n]"),
            "{all}"
        );
        assert_eq!(app.on_key(press), expected);
        assert!(app.modal.is_none());
    }

    // The buttons are the same two answers.
    let mut app = fleet_app(&["alice"]);
    app.confirm(ConfirmKind::FleetPlan {
        action: FleetAction::Logout,
        prompt: prompt.into(),
    });
    let rows = draw(&mut app, 120, 40);
    let (cancel_x, cancel_y) = find(&rows, "[Cancel n]").unwrap();
    assert_eq!(app.on_click(cancel_x, cancel_y), AppAction::CancelFleetPlan);
    app.confirm(ConfirmKind::FleetPlan {
        action: FleetAction::Logout,
        prompt: prompt.into(),
    });
    let rows = draw(&mut app, 120, 40);
    let (run_x, run_y) = find(&rows, "[Run y]").unwrap();
    assert_eq!(app.on_click(run_x, run_y), AppAction::RunFleetPlan);
}

/// Assign, Assign & restart and Copy settings are unavailable while the
/// fleet card is each bot's own, and say how to fix it; Start, Stop, Log in
/// and Log out need no card. Picking a card makes them available.
#[test]
fn card_actions_are_unavailable_on_own_until_a_card_is_picked() {
    let mut seam = Seam::new();
    let mut app = fleet_app(&["alice", "bob"]);
    for action in FleetAction::ALL {
        let command = Command::Fleet(action);
        if action.needs_card() {
            assert_eq!(command.availability(&app), Err("pick a card first"));
            assert_eq!(app.run_command(command), AppAction::None);
            let shown = app.error.take().expect("the reason shows");
            assert!(shown.ends_with("pick a card first"), "{shown}");
        } else {
            assert_eq!(command.availability(&app), Ok(()), "{action:?}");
        }
    }
    seam.apply(&mut app, AppAction::PickFleetCard(FleetCard::Focused));
    for action in FleetAction::ALL {
        let command = Command::Fleet(action);
        assert_eq!(command.availability(&app), Ok(()), "{action:?}");
        assert_eq!(app.run_command(command), AppAction::PlanFleet(action));
    }
}

/// The palette lists the card picker, the shown-row marks and each status
/// mark with their keys, and the card actions with the reason they cannot
/// run on Own.
#[test]
fn the_palette_lists_the_fleet_card_marks_and_actions_with_keys() {
    let mut app = fleet_app(&["alice", "bob"]);
    let palette = |app: &mut TuiApp, query: &str| {
        app.on_key(ctrl('p'));
        for c in query.chars() {
            app.on_key(ch(c));
        }
        let shown = text(&draw(app, 120, 40));
        app.on_key(key(KeyCode::Esc));
        shown
    };
    let card = palette(&mut app, "pick fleet");
    assert!(
        card.contains("Pick fleet card…") && card.contains("fleet card"),
        "{card}"
    );
    let marks = palette(&mut app, "mark");
    for label in [
        "Mark shown rows",
        "Unmark shown rows",
        "Mark by status: ready",
        "Mark by status: idle",
        "Mark by status: running",
        "Mark by status: logged out",
        "Mark by status: login error",
        "Mark by status: not loaded",
    ] {
        assert!(marks.contains(label), "{label}: {marks}");
    }
    let invert = palette(&mut app, "invert");
    assert!(invert.contains("Invert shown"), "{invert}");
    for command in [
        Command::PickFleetCard,
        Command::MarkAllShown,
        Command::UnmarkShown,
    ] {
        let keys = crate::palette::key_text(command);
        assert_eq!(
            keys,
            match command {
                Command::PickFleetCard => "c",
                Command::MarkAllShown => "a",
                _ => "A",
            }
        );
    }
    let actions = palette(&mut app, "assign");
    assert!(
        actions.contains("Assign & restart") && actions.contains("pick a card first"),
        "{actions}"
    );
    let copy = palette(&mut app, "copy settings");
    assert!(
        copy.contains("Copy settings") && copy.contains("pick a card first"),
        "{copy}"
    );
    // Every fleet action keeps its Script tab key in the palette.
    let start = palette(&mut app, "start fleet");
    assert!(start.contains("Start fleet"), "{start}");
    assert_eq!(
        crate::palette::key_text(Command::Fleet(FleetAction::Start)),
        "T"
    );
    assert_eq!(
        crate::palette::key_text(Command::Fleet(FleetAction::Stop)),
        "E"
    );
    assert_eq!(
        crate::palette::key_text(Command::Fleet(FleetAction::Login)),
        "m"
    );
    assert_eq!(
        crate::palette::key_text(Command::Fleet(FleetAction::Logout)),
        "U"
    );
}

/// `a` marks the shown rows, `A` unmarks them, and the status marks and
/// Invert shown are palette commands: each one is a core `MarkFleet` row.
#[test]
fn mark_keys_and_palette_entries_submit_core_mark_ops() {
    let mut seam = Seam::with_profiles(&[("alice", 41), ("bob", 42)]);
    let mut app = fleet_app(&["alice", "bob"]);
    assert_eq!(
        app.on_key(ch('a')),
        AppAction::MarkFleet(frontend_core::MarkOp::Shown)
    );
    // Nothing is marked yet in the model: `A` says so and sends nothing.
    assert_eq!(app.on_key(ch('A')), AppAction::None);
    assert!(app
        .error
        .take()
        .unwrap()
        .ends_with("no shown row is marked"));
    seam.apply(&mut app, AppAction::MarkFleet(frontend_core::MarkOp::Shown));
    assert_eq!(
        app.on_key(ch('A')),
        AppAction::MarkFleet(frontend_core::MarkOp::UnmarkShown)
    );
    assert_eq!(
        app.run_command(Command::InvertShown),
        AppAction::MarkFleet(frontend_core::MarkOp::InvertShown)
    );
    for predicate in frontend_core::MarkPredicate::ALL {
        let command = Command::MarkWhere(predicate);
        assert!(
            command.label(&app).ends_with(predicate.label()),
            "the palette label names the core label"
        );
        assert_eq!(
            app.run_command(command),
            AppAction::MarkFleet(frontend_core::MarkOp::Where(predicate))
        );
    }
}

/// `c` opens the picker over Own, Focused and the Browse cards, grouped by
/// category. Enter pins the highlighted card; Esc and an outside click
/// leave the fleet card alone. Neither touches the Browse pick.
#[test]
fn c_opens_the_fleet_card_picker_that_enter_picks_and_esc_keeps() {
    use crate::script_shape::BrowseCard;
    let card = |name: &str, category: &str| BrowseCard {
        name: name.into(),
        description: String::new(),
        category: category.into(),
        tags: Vec::new(),
        kind: CardRuntime::Compat,
        source: CardSource::File,
        unloadable: None,
    };
    let mut app = fleet_app(&["alice", "bob"]);
    crate::test_support::set_cards(
        &mut app,
        vec![
            card("Gatherer", "Skilling"),
            card("Fighter", "Combat"),
            card("Miner", "Skilling"),
        ],
    );
    let browse_pick = ScriptChoice::loaded(CardSource::File, "Thiever");
    app.script_sel = Some(browse_pick.duplicate());

    assert_eq!(app.on_key(ch('c')), AppAction::None);
    assert!(matches!(&app.modal, Some(Modal::FleetCard(_))));
    let all = text(&draw(&mut app, 120, 40));
    for want in [
        "Each bot's own card",
        "Focused bot's card",
        "Gatherer",
        "Fighter",
        "Miner",
        "Skilling",
        "Combat",
        "(current)",
    ] {
        assert!(all.contains(want), "{want}: {all}");
    }
    let at = |name: &str| all.find(name).unwrap();
    assert!(
        at("Each bot's own card") < at("Focused bot's card")
            && at("Focused bot's card") < at("Gatherer"),
        "Own, then Focused, then the cards: {all}"
    );
    assert!(
        at("Gatherer") < at("Miner") && at("Miner") < at("Fighter"),
        "cards stay grouped by category as Browse groups them: {all}"
    );

    // Down Down lands on the first card; Enter pins it.
    app.on_key(key(KeyCode::Down));
    assert_eq!(app.on_key(key(KeyCode::Down)), AppAction::None);
    assert_eq!(
        app.on_key(key(KeyCode::Enter)),
        AppAction::PickFleetCard(FleetCard::Pinned(ScriptChoice::loaded(
            CardSource::File,
            "Gatherer"
        )))
    );
    assert!(app.modal.is_none());

    // Esc leaves the card as it was and sends nothing.
    app.on_key(ch('c'));
    app.on_key(key(KeyCode::Down));
    assert_eq!(app.on_key(key(KeyCode::Esc)), AppAction::None);
    assert!(app.modal.is_none());
    // A click outside does too.
    app.on_key(ch('c'));
    draw(&mut app, 120, 40);
    assert_eq!(app.on_click(0, 0), AppAction::None);
    assert!(app.modal.is_none());

    assert_eq!(
        app.script_sel.as_ref(),
        Some(&browse_pick),
        "the picker never changes the Browse pick"
    );
    assert!(!app.browse_changed, "and never reports a Browse pick");
    assert!(!app.script_browse_open);
}

/// The palette's *Pick fleet card…* opens the same picker.
#[test]
fn the_palette_opens_the_fleet_card_picker() {
    let mut app = fleet_app(&["alice"]);
    assert_eq!(app.run_command(Command::PickFleetCard), AppAction::None);
    assert!(matches!(&app.modal, Some(Modal::FleetCard(_))));
    // The picker is its own overlay: it takes keys and Enter on the
    // current card (Own, the cursor's start) picks Own again.
    assert_eq!(
        app.on_key(key(KeyCode::Enter)),
        AppAction::PickFleetCard(FleetCard::Own)
    );
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
    let mut seam = Seam::new();
    let mark = app.on_click(2, carol_y);
    assert_eq!(
        mark,
        AppAction::SetMark("carol".into(), true),
        "the checkbox column"
    );
    seam.apply(&mut app, mark);

    assert!(app
        .fleet_model
        .marks()
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
        !app.log.view.follow(),
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
        .requested();
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
    app.script_sel = Some(ScriptChoice::loaded(CardSource::File, "thiever"));
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
        kind: CardRuntime::Compat,
        source: CardSource::File,
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
            // "import" alone also matches Import rs2b0t profiles.
            for c in "import rs2b0t catalog".chars() {
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
        app.script_sel = Some(ScriptChoice::loaded(CardSource::File, "Alpha"));
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

        let sel_before = app
            .script_sel
            .as_ref()
            .map(frontend_core::scripts::ScriptChoice::duplicate);
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

/// Screen text with every run of whitespace (including the row breaks
/// a wrapped popup line spans) collapsed to one space, so a phrase is
/// found wherever wrapping put it — but only if it was drawn at all.
fn flattened(rows: &[String]) -> String {
    text(rows).split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `s` without any whitespace: wrapping may break a line at a space or in a
/// word, so the popup's text is compared without it.
fn squashed(s: &str) -> String {
    s.split_whitespace().collect()
}

/// A long plan prompt (core caps the names it spells out, then "+N
/// more") is shown whole at 120x40 and 80x24: the popup sizes to the
/// wrapped rows, so none of core's text is cut.
#[test]
fn a_long_plan_prompt_is_shown_whole_at_both_sizes() {
    let prompt =
        "Assign thiever and restart it on 14 marked bots: bot00name, bot01name, bot02name, \
                  bot03name, bot04name, bot05name, bot06name, bot07name +6 more. This stops the \
                  running script of bot00name, bot01name, bot02name, bot03name, bot04name, \
                  bot05name, bot06name, bot07name +2 more. Skipped 2: bot12name: profile is \
                  being saved, bot13name: already active.";
    for (width, height) in [(120, 40), (80, 24)] {
        let mut app = fleet_app(&["alice"]);
        app.confirm(ConfirmKind::FleetPlan {
            action: FleetAction::AssignRestart,
            prompt: prompt.into(),
        });
        let rows = draw(&mut app, width, height);
        let all = squashed(&popup_rows(&app, &rows).join(" "));
        assert!(all.contains(&squashed(prompt)), "{width}x{height}: {all}");
        assert!(
            flattened(&rows).contains("Assign & restart"),
            "titled by the action"
        );
    }
}

/// Inner rows of the drawn modal popup (borders stripped).
fn popup_rows(app: &TuiApp, rows: &[String]) -> Vec<String> {
    let m = app.regions.modal;
    ((m.y + 1)..(m.y + m.height - 1))
        .map(|y| {
            rows[usize::from(y)]
                .chars()
                .skip(usize::from(m.x + 1))
                .take(usize::from(m.width - 2))
                .collect()
        })
        .collect()
}

/// L1: at 40x20 the popup's text area is 38 cells, so an email-style
/// name wider than that is split across rows by ratatui from a fresh
/// row. The popup must be sized for those rows: core's whole prompt shows
/// and the slack row above the buttons stays blank.
#[test]
fn plan_confirm_at_40_by_20_shows_the_whole_prompt_when_a_name_is_wider_than_the_popup() {
    let names: Vec<String> = (0..4)
        .map(|i| format!("bot{i:02}-with-a-very-long-email-name@example.com"))
        .collect();
    assert!(
        names.iter().all(|name| name.len() > 38),
        "every name is wider than the 40-wide popup's 38-cell text area: {names:?}"
    );
    let prompt = format!(
        "Assign thiever and restart it on 4 marked bots: {}. This stops the running script of {}, {}.",
        names.join(", "),
        names[0],
        names[1]
    );
    let mut app = fleet_app(&["alice"]);
    app.confirm(ConfirmKind::FleetPlan {
        action: FleetAction::AssignRestart,
        prompt: prompt.clone(),
    });
    let rows = draw(&mut app, 40, 20);
    let popup = popup_rows(&app, &rows);
    let all = squashed(&popup.join(" "));
    assert!(
        all.contains(&squashed(&prompt)),
        "the whole prompt shows: {all}"
    );
    let slack = &popup[popup.len() - 2];
    assert!(
        slack.trim().is_empty(),
        "the row above the buttons is slack, not text the popup barely fit: {slack:?}\n{}",
        popup.join("\n")
    );
}

/// The palette scope text of the fleet actions follows the core model's
/// counts: the loaded shown rows with no marks, the marked bots once
/// anything is marked.
#[test]
fn palette_scope_text_follows_the_core_marks() {
    let mut app = fleet_app(&["alice", "bob", "carol"]);
    for action in FleetAction::ALL {
        if action != FleetAction::CopySettings {
            assert_eq!(Command::Fleet(action).scope(&app), "all 3 shown");
        }
    }
    for name in ["alice", "bob"] {
        let id = app.profile_id_for_name(name);
        app.fleet_model.set_mark_for_test(id, true);
    }
    for action in FleetAction::ALL {
        if action != FleetAction::CopySettings {
            assert_eq!(Command::Fleet(action).scope(&app), "2 marked bots");
        }
    }
    assert_eq!(
        Command::Fleet(FleetAction::CopySettings).scope(&app),
        "alice to 2 marked bots"
    );
}

/// An app whose fleet is `loaded` (ready) followed by vault profiles that
/// are not loaded, as the fleet model's universe orders them.
fn universe_app(loaded: &[&str], unloaded: &[&str]) -> TuiApp {
    let mut app = fleet_app(loaded);
    let mut rows: Vec<frontend_core::FleetRow> = app
        .fleet_model
        .rows()
        .iter()
        .map(frontend_core::FleetRow::snapshot)
        .collect();
    rows.extend(
        unloaded
            .iter()
            .map(|name| frontend_core::FleetRow::for_profile(*name)),
    );
    app.fleet_model.set_universe_for_test(rows, &[]);
    app
}

/// The fleet table lists an unloaded vault profile after the loaded rows,
/// as `not loaded`, and Space marks it like any other row.
#[test]
fn space_marks_an_unloaded_profile_row_the_table_lists() {
    let mut seam = Seam::new();
    let mut app = universe_app(&["alice"], &["zed"]);
    let rows = draw(&mut app, 120, 40);
    let (_, alice_y) = find(&rows, "alice").expect("the loaded row");
    let (_, zed_y) = find(&rows, "zed").expect("the unloaded profile is listed");
    assert!(zed_y > alice_y, "after the loaded rows: {}", text(&rows));
    assert!(
        rows[usize::from(zed_y)].contains("not loaded"),
        "{}",
        text(&rows)
    );
    assert!(
        !rows[usize::from(zed_y)].contains("[x]"),
        "not marked yet: {}",
        text(&rows)
    );
    app.on_key(key(KeyCode::Down));
    seam.press(&mut app, ch(' '), AppAction::SetMark("zed".into(), true));
    let rows = draw(&mut app, 120, 40);
    let (_, zed_y) = find(&rows, "zed").unwrap();
    assert!(
        rows[usize::from(zed_y)].contains("[x] > zed"),
        "{}",
        text(&rows)
    );
    assert!(text(&rows).contains("marked 1"), "{}", text(&rows));
    assert_eq!(app.focused_name().as_deref(), Some("alice"));
}

/// A click on the `[x]` box of an unloaded row marks it; a click on the
/// rest of the row focuses nothing.
#[test]
fn clicking_an_unloaded_row_marks_through_its_box_and_never_focuses() {
    let mut app = universe_app(&["alice"], &["zed"]);
    let rows = draw(&mut app, 120, 40);
    let (x, y) = find(&rows, "zed").unwrap();
    let box_x = app.regions.fleet_rows.x;
    assert_eq!(
        app.on_click(box_x + 1, y),
        AppAction::SetMark("zed".into(), true)
    );
    assert_eq!(app.on_click(x, y), AppAction::None);
    assert_eq!(app.focused_name().as_deref(), Some("alice"));
    assert_eq!(
        app.error.as_deref(),
        Some("zed is not loaded: mark it and press m to log in")
    );
}

/// Enter on an unloaded row keeps the focus where it was and says what to
/// do instead.
#[test]
fn enter_on_an_unloaded_row_does_not_focus_and_says_why() {
    let mut app = universe_app(&["alice", "bob"], &["zed"]);
    draw(&mut app, 120, 40);
    assert_eq!(app.focused_name().as_deref(), Some("alice"));
    app.on_key(key(KeyCode::End));
    assert_eq!(app.on_key(key(KeyCode::Enter)), AppAction::None);
    assert_eq!(app.focused_name().as_deref(), Some("alice"));
    assert_eq!(
        app.error.as_deref(),
        Some("zed is not loaded: mark it and press m to log in")
    );
    // A loaded row still focuses.
    app.on_key(key(KeyCode::Up));
    assert_eq!(
        app.on_key(key(KeyCode::Enter)),
        AppAction::Focus("bob".into())
    );
}

/// `a` marks every shown row, loaded or not, through the core mark op, and
/// nothing the filter hides.
#[test]
fn the_mark_shown_key_marks_only_the_shown_rows() {
    let mut seam = Seam::new();
    let mut app = universe_app(&["alice", "bob"], &["zed"]);
    app.fleet_model.set_filter_for_test("z");
    seam.press(
        &mut app,
        ch('a'),
        AppAction::MarkFleet(frontend_core::MarkOp::Shown),
    );
    let id = frontend_core::ProfileIdentity::synthetic;
    assert!(app.fleet_model.marks().contains(id("zed")));
    assert!(!app.fleet_model.marks().contains(id("alice")));
    assert!(!app.fleet_model.marks().contains(id("bob")));
}

/// The context menu of an unloaded row offers only the mark.
#[test]
fn the_context_menu_of_an_unloaded_row_offers_only_the_mark() {
    let mut seam = Seam::new();
    let mut app = universe_app(&["alice"], &["zed"]);
    let rows = draw(&mut app, 120, 40);
    let (x, y) = find(&rows, "zed").unwrap();
    app.on_mouse(right_click(x, y));
    let Some(Modal::Context(menu)) = &app.modal else {
        panic!("a context menu opened");
    };
    assert_eq!(menu.items.len(), 1, "{:?}", menu.items);
    let action = app.on_key(key(KeyCode::Enter));
    assert_eq!(action, AppAction::SetMark("zed".into(), true));
    seam.apply(&mut app, action);
    assert!(app
        .fleet_model
        .marks()
        .contains(frontend_core::ProfileIdentity::synthetic("zed")));
}

fn alt(code: KeyCode) -> crossterm::event::KeyEvent {
    crossterm::event::KeyEvent::new(code, crossterm::event::KeyModifiers::ALT)
}

fn reorder_ids() -> [frontend_core::ProfileIdentity; 3] {
    [
        frontend_core::ProfileIdentity::uid(41),
        frontend_core::ProfileIdentity::uid(42),
        frontend_core::ProfileIdentity::uid(43),
    ]
}

/// Three profiles with vault identities, drawn once so the table has its
/// regions.
fn reorder_app() -> TuiApp {
    let mut app = fleet_app(&["alice", "bob", "carol"]);
    crate::test_support::set_profile_ids(&mut app, &reorder_ids());
    draw(&mut app, 120, 40);
    app
}

fn move_op(moving: frontend_core::ProfileIdentity, to: frontend_core::Placement) -> AppAction {
    AppAction::ReorderProfiles(frontend_core::ReorderOp::Move { moving, to })
}

/// Alt+Up / Alt+Down move the cursor row; the action names the identities
/// of the cursor row and its shown neighbour. At the first or last shown
/// row nothing happens.
#[test]
fn alt_up_and_alt_down_return_the_reorder_action_for_the_cursor_row() {
    use frontend_core::Placement::{After, Before};
    let [alice, bob, carol] = reorder_ids();
    let mut app = reorder_app();
    assert_eq!(
        app.on_key(alt(KeyCode::Up)),
        AppAction::None,
        "alice is on the first row"
    );
    assert_eq!(
        app.on_key(alt(KeyCode::Down)),
        move_op(alice, After(bob)),
        "down: after the next shown row"
    );
    app.on_key(key(KeyCode::Down));
    assert_eq!(
        app.on_key(alt(KeyCode::Up)),
        move_op(bob, Before(alice)),
        "up: before the previous shown row"
    );
    assert_eq!(app.on_key(alt(KeyCode::Down)), move_op(bob, After(carol)));
    app.on_key(key(KeyCode::Down));
    assert_eq!(
        app.on_key(alt(KeyCode::Down)),
        AppAction::None,
        "carol is on the last row"
    );
    assert_eq!(app.on_key(alt(KeyCode::Up)), move_op(carol, Before(bob)));
}

/// The keys act only while the fleet pane has the keyboard.
#[test]
fn alt_up_and_alt_down_are_scoped_to_the_fleet_pane() {
    let mut app = reorder_app();
    app.key_focus = Pane::Detail;
    assert_eq!(app.on_key(alt(KeyCode::Down)), AppAction::None);
    assert_eq!(app.on_key(alt(KeyCode::Up)), AppAction::None);
    app.key_focus = Pane::Fleet;
    assert!(matches!(
        app.on_key(alt(KeyCode::Down)),
        AppAction::ReorderProfiles(_)
    ));
}

/// The palette commands return the same op as the keys, are listed with
/// their reasons, and a sorted fleet makes all three unavailable.
#[test]
fn the_move_commands_match_the_keys_and_follow_their_availability() {
    use frontend_core::Placement::{After, Before};
    use frontend_core::{FleetOrder, ReorderOp};
    let [alice, bob, _] = reorder_ids();
    let mut app = reorder_app();
    assert_eq!(
        Command::MoveProfileUp.availability(&app),
        Err("no shown row above the cursor")
    );
    assert_eq!(
        Command::MoveMarkedToTop.availability(&app),
        Err("mark fleet rows first (Space)")
    );
    assert_eq!(Command::MoveProfileDown.availability(&app), Ok(()));
    assert_eq!(
        app.run_command(Command::MoveProfileDown),
        move_op(alice, After(bob))
    );
    app.on_key(key(KeyCode::Down));
    assert_eq!(
        app.run_command(Command::MoveProfileUp),
        move_op(bob, Before(alice))
    );
    app.fleet_model.set_mark_for_test(bob, true);
    assert_eq!(Command::MoveMarkedToTop.availability(&app), Ok(()));
    assert_eq!(
        app.run_command(Command::MoveMarkedToTop),
        AppAction::ReorderProfiles(ReorderOp::MarkedToTop)
    );

    app.error = None;
    app.fleet_model.set_order_for_test(FleetOrder::Name);
    for command in [
        Command::MoveProfileUp,
        Command::MoveProfileDown,
        Command::MoveMarkedToTop,
    ] {
        assert_eq!(
            command.availability(&app),
            Err("choose Load order to reorder"),
            "{command:?}"
        );
        assert_eq!(app.run_command(command), AppAction::None, "{command:?}");
        let message = app.error.take().expect("the reason is shown");
        assert!(
            message.contains("choose Load order to reorder"),
            "{message}"
        );
    }
    for code in [KeyCode::Up, KeyCode::Down] {
        assert_eq!(app.on_key(alt(code)), AppAction::None);
    }
}

/// The palette lists the commands with their keys and the reason a command
/// cannot run.
#[test]
fn the_palette_lists_the_move_commands_with_keys_and_reasons() {
    let mut app = reorder_app();
    app.on_key(ctrl('p'));
    for c in "move".chars() {
        app.on_key(ch(c));
    }
    let screen = text(&draw(&mut app, 140, 40));
    for wanted in [
        "Move profile up",
        "Alt-Up",
        "Move profile down",
        "Alt-Down",
        "Move marked to top",
        "mark fleet rows first",
        "no shown row above the cursor",
    ] {
        assert!(screen.contains(wanted), "{wanted}: {screen}");
    }
}

/// Help lists the keys and the palette-only command.
#[test]
fn help_lists_the_reorder_keys_and_move_marked_to_top() {
    let app = reorder_app();
    let rows = help_rows(&app);
    assert!(
        rows.iter()
            .any(|row| row.keys.contains("Alt-Up") && row.keys.contains("Alt-Down")),
        "{rows:?}"
    );
    assert!(
        rows.iter()
            .any(|row| row.what.contains("Move marked to top")),
        "{rows:?}"
    );
}

/// The fleet KEYS line names the keys while they work: the fleet pane has
/// the keyboard and the order is Load order.
#[test]
fn the_fleet_keys_line_names_the_reorder_keys_only_while_they_work() {
    let mut app = reorder_app();
    let wanted = "Alt-Up/Down reorder";
    assert!(app.key_hints().contains(wanted), "{}", app.key_hints());
    let screen = text(&draw(&mut app, 120, 40));
    assert!(screen.contains(wanted), "{screen}");
    app.key_focus = Pane::Detail;
    assert!(!app.key_hints().contains(wanted));
    assert!(!text(&draw(&mut app, 120, 40)).contains(wanted));
    app.key_focus = Pane::Fleet;
    app.fleet_model
        .set_order_for_test(frontend_core::FleetOrder::Status);
    assert!(!app.key_hints().contains(wanted));
    assert!(!text(&draw(&mut app, 120, 40)).contains(wanted));
}
