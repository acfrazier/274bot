use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::Terminal;

use api::snapshot::{ChatLineView, ChatOptionView, WorldTile};
use nav::tile::Tile;
use script::{RunState, ScriptKind, ScriptSel, ScriptSource};
use vault::ProfileSettings;

use crate::script_shape::BrowseCard;

use super::{wasd_target, AppAction, TuiApp};

fn bone_burier_card() -> BrowseCard {
    BrowseCard {
        name: "BoneBurier".into(),
        description: String::new(),
        category: "Prayer".into(),
        tags: Vec::new(),
        kind: ScriptKind::Compat,
        source: ScriptSource::Catalog,
        unloadable: None,
    }
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn tile(x: i32, z: i32) -> Tile {
    Tile { x, z, level: 0 }
}

fn line(text: &str) -> ChatLineView {
    ChatLineView {
        type_: 0,
        username: Some("npc".into()),
        text: text.into(),
        sequence: 0,
    }
}

fn nature_crafter_paint() -> script::shim::ScriptPaint {
    script::shim::ScriptPaint {
        title: Some("NatureCrafter — Air — runner — restocking at the bank".into()),
        accent: None,
        lines: vec![
            "Runtime: 1m | Mode: Runner | To: paintproof".into(),
            "Deliveries: 0 | Ess sent: 0 | Coins: 0".into(),
            "Pack ess: 0 | noted: 0 | unnoted: 0".into(),
        ],
        buttons: vec![script::shim::ScriptPaintButton {
            id: "gobank".into(),
            label: "Go bank".into(),
        }],
        generation: 0,
        canvas: Vec::new(),
        ..Default::default()
    }
}

fn buffer_position(buf: &Buffer, width: u16, needle: &str) -> Option<(u16, u16)> {
    buf.content()
        .chunks(usize::from(width))
        .enumerate()
        .find_map(|(row, cells)| {
            let text: String = cells.iter().map(|cell| cell.symbol()).collect();
            text.find(needle).map(|col| (col as u16, row as u16))
        })
}

/// The window title line survives the full chrome draw.
#[test]
fn draws_title_containing_274bot() {
    let mut app = TuiApp::new("274bot headless");
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();

    let buf = terminal.backend().buffer();
    let text: String = buf.content().iter().map(|cell| cell.symbol()).collect();
    assert!(
        text.contains("274bot"),
        "buffer does not contain 274bot: {text:?}"
    );
    assert!(
        text.contains("no nav pack"),
        "empty world must title the map pane as missing the pack: {text:?}"
    );
}

/// A loaded world drops the empty-state title; the map paints the
/// walkable field instead of a hollow "no nav pack" block.
#[test]
fn draw_map_paints_walkable_dots_when_the_pack_is_loaded() {
    let mut app = TuiApp::new("274bot headless");
    app.world = Some(Arc::new(nav::world::NavWorld::from_grid(
        &nav::grid::StepGrid::fixture_open_3x3(),
    )));
    app.here = Some(api::snapshot::WorldTile {
        x: 1,
        z: 1,
        level: 0,
    });
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let buf = terminal.backend().buffer();
    let text: String = buf.content().iter().map(|cell| cell.symbol()).collect();
    assert!(
        !text.contains("no nav pack"),
        "loaded pack must not keep the empty-state title: {text:?}"
    );
    assert!(text.contains('.'), "walkable tiles paint as dots: {text:?}");
    assert!(
        text.contains('@'),
        "the here marker paints on the player tile: {text:?}"
    );
}

/// The spec's WASD test: from (10,10) W steps north. +z is north on
/// the client's axis (the map's north-up camera — see `map.rs`'s pan
/// tests), so W is +z: (10,10) → (10,11).
#[test]
fn wasd_w_from_10_10_walks_north_to_10_11() {
    let here = (10, 10, 0);
    assert_eq!(
        wasd_target(here, KeyCode::Char('w')),
        Some((10, 11, 0)),
        "W is north = +z on the client axis"
    );
    assert_eq!(
        wasd_target(here, KeyCode::Char('s')),
        Some((10, 9, 0)),
        "S is south = -z"
    );
    assert_eq!(
        wasd_target(here, KeyCode::Char('a')),
        Some((9, 10, 0)),
        "A is west = -x"
    );
    assert_eq!(
        wasd_target(here, KeyCode::Char('d')),
        Some((11, 10, 0)),
        "D is east = +x"
    );
    assert_eq!(wasd_target(here, KeyCode::F(1)), None);
}

#[test]
fn wasd_on_the_app_returns_a_walk_tile_action() {
    let mut app = TuiApp::new("274bot headless");
    app.names = vec!["test".into()];
    app.focused = Some(0);
    app.statuses = vec![host_play::SlotStatus {
        username: "test".into(),
        ingame: true,
        scene_state: 2,
        tile_x: 10,
        tile_z: 10,
        ..host_play::SlotStatus::default()
    }];
    app.refresh();
    assert_eq!(
        app.on_key(key(KeyCode::Char('w'))),
        AppAction::WalkTile(tile(10, 11)),
        "W on the app queues a one-tile north walk"
    );
}

/// TASK-014 parity: upstairs origin must arm WASD on the player plane.
#[test]
fn player_at_plane_one_refresh_arms_wasd_with_level() {
    let mut app = TuiApp::new("274bot headless");
    app.names = vec!["test".into()];
    app.focused = Some(0);
    app.statuses = vec![host_play::SlotStatus {
        username: "test".into(),
        ingame: true,
        scene_state: 2,
        tile_x: 10,
        tile_z: 10,
        tile_level: 1,
        ..host_play::SlotStatus::default()
    }];
    app.refresh();
    assert_eq!(
        app.here,
        Some(WorldTile {
            x: 10,
            z: 10,
            level: 1,
        }),
        "refresh must publish tile_level, not hardcoded ground"
    );
    assert_eq!(
        app.on_key(key(KeyCode::Char('w'))),
        AppAction::WalkTile(Tile {
            x: 10,
            z: 11,
            level: 1,
        }),
        "W must keep the player plane when arming a one-tile walk"
    );
}

#[test]
fn lowercase_s_walks_south_when_settings_closed() {
    let mut app = TuiApp::new("274bot headless");
    app.names = vec!["test".into()];
    app.focused = Some(0);
    app.statuses = vec![host_play::SlotStatus {
        username: "test".into(),
        ingame: true,
        scene_state: 2,
        tile_x: 10,
        tile_z: 10,
        ..host_play::SlotStatus::default()
    }];
    app.refresh();
    assert!(
        !app.settings_state.open,
        "settings must start closed so s is free for WASD"
    );
    assert_eq!(
        app.on_key(key(KeyCode::Char('s'))),
        AppAction::WalkTile(tile(10, 9)),
        "lowercase s walks south when settings are closed"
    );
}

#[test]
fn q_quits_and_o_toggles_settings() {
    let mut app = TuiApp::new("274bot headless");
    assert_eq!(app.on_key(key(KeyCode::Char('o'))), AppAction::None);
    assert!(app.settings_state.open, "o opens the settings popup");
    assert_eq!(app.on_key(key(KeyCode::Char('q'))), AppAction::Quit);
    assert!(app.quit);
}

#[test]
fn m_spawns_the_rest_of_the_multibox_wall() {
    let mut app = TuiApp::new("274bot headless");
    assert_eq!(
        app.on_key(key(KeyCode::Char('m'))),
        AppAction::SpawnAll,
        "m spawns every parked profile"
    );
}

/// A chat modal on the focused snapshot routes Space/Enter to the
/// chat pane instead of the map.
#[test]
fn chat_modal_open_routes_enter_to_continue() {
    let mut app = TuiApp::new("274bot headless");
    app.chat_data.modal_texts = vec!["The stranger waits.".into()];
    app.chat_data.has_continue = true;
    assert_eq!(
        app.on_key(key(KeyCode::Enter)),
        AppAction::Chat(super::ChatAction::Continue),
        "Enter while a chat modal is up continues the dialog"
    );
}

#[test]
fn chat_modal_options_answer_on_space() {
    let mut app = TuiApp::new("274bot headless");
    app.chat_data.modal_texts = vec!["Which way?".into()];
    app.chat_data.options = vec![ChatOptionView {
        component_id: 1,
        text: "Yes".into(),
    }];
    app.chat_data.has_continue = true;
    assert_eq!(
        app.on_key(key(KeyCode::Char(' '))),
        AppAction::Chat(super::ChatAction::Answer(1)),
        "Space answers the focused option"
    );
}

#[test]
fn paint_showing_digit_routes_to_paint_button_not_wire() {
    let mut app = TuiApp::new("274bot headless");
    app.chat_data.script_paint = Some(std::sync::Arc::new(script::shim::ScriptPaint {
        title: Some("NatureCrafter".into()),
        accent: None,
        lines: vec!["status".into()],
        buttons: vec![script::shim::ScriptPaintButton {
            id: "gobank".into(),
            label: "Go bank".into(),
        }],
        generation: 0,
        canvas: Vec::new(),
        ..Default::default()
    }));
    assert_eq!(
        app.on_key(key(KeyCode::Char('1'))),
        AppAction::Chat(super::ChatAction::PaintButton(0)),
        "digit 1 dispatches the advertised paint button"
    );
    app.chat_data.has_continue = true;
    app.chat_data.modal_texts = vec!["Wait.".into()];
    assert_eq!(
        app.on_key(key(KeyCode::Enter)),
        AppAction::Chat(super::ChatAction::Continue),
        "modal still wins over paint buttons"
    );
}

#[test]
fn nature_crafter_button_is_rendered_and_only_its_row_is_clickable_at_140x40() {
    const WIDTH: u16 = 140;
    let mut app = TuiApp::new("274bot headless");
    app.chat_data.script_paint = Some(std::sync::Arc::new(nature_crafter_paint()));
    let mut terminal = Terminal::new(TestBackend::new(WIDTH, 40)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();

    let buf = terminal.backend().buffer();
    let (button_col, button_row) = buffer_position(buf, WIDTH, "[1] Go bank")
        .expect("the advertised NatureCrafter button must be visible");
    let (title_col, title_row) = buffer_position(buf, WIDTH, "NatureCrafter — Air")
        .expect("the paint title must remain visible");
    let (body_col, body_row) = buffer_position(buf, WIDTH, "Runtime: 1m")
        .expect("the first paint status row must remain visible");

    assert_eq!(
        app.on_click(button_col, button_row),
        AppAction::Chat(super::ChatAction::PaintButton(0)),
        "clicking the actual rendered label row dispatches its button"
    );
    assert_eq!(app.on_click(title_col, title_row), AppAction::None);
    assert_eq!(app.on_click(body_col, body_row), AppAction::None);
    assert_eq!(
        app.on_click(app.chat_area.x + 1, button_row - 1),
        AppAction::None,
        "the rendered spacer above the button is not a hit target"
    );
    assert_eq!(
        app.on_click(app.chat_area.x, button_row),
        AppAction::None,
        "the pane border is not a button hit target"
    );
}

#[test]
fn nature_crafter_button_remains_visible_and_clickable_in_a_compact_terminal() {
    const WIDTH: u16 = 48;
    let mut app = TuiApp::new("274bot headless");
    app.chat_data.script_paint = Some(std::sync::Arc::new(nature_crafter_paint()));
    let mut terminal = Terminal::new(TestBackend::new(WIDTH, 18)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();

    let (button_col, button_row) =
        buffer_position(terminal.backend().buffer(), WIDTH, "[1] Go bank")
            .expect("the focused paint button must survive compact layout clipping");
    assert_eq!(
        app.on_click(button_col, button_row),
        AppAction::Chat(super::ChatAction::PaintButton(0))
    );
    assert_eq!(
        app.on_click(app.chat_area.x, app.chat_area.y),
        AppAction::None,
        "the compact pane's title border is not a paint hit target"
    );
}

#[test]
fn settings_enter_flips_random_events_and_marks_dirty() {
    let mut app = TuiApp::new("274bot headless");
    app.settings = ProfileSettings::default();
    assert!(app.settings.random_events);
    app.settings_state.open = true;
    app.on_key(key(KeyCode::Enter));
    assert!(!app.settings.random_events, "popup flips random_events");
    assert!(app.settings_dirty, "the binary persists the change");
}

#[test]
fn map_enter_confirms_a_walk_selection() {
    let mut app = TuiApp::new("274bot headless");
    app.names = vec!["test".into()];
    app.focused = Some(0);
    app.world = Some(Arc::new(nav::world::NavWorld::from_grid(
        &nav::grid::StepGrid::fixture_open_3x3(),
    )));
    app.here = Some(api::snapshot::WorldTile {
        x: 1,
        z: 1,
        level: 0,
    });
    app.map.selection = Some(tile(2, 2));
    assert_eq!(
        app.on_key(key(KeyCode::Enter)),
        AppAction::ArmWalk(tile(2, 2)),
        "Enter on a selection arms the walk"
    );
}

/// The review's focus test: Tab must produce an action that carries
/// the newly focused name, so the binary can mirror it onto
/// `Play::focus` (the app's index alone leaves the session on the
/// boot slot's sample gate).
#[test]
fn tab_produces_a_focus_action_for_the_next_running_slot() {
    let mut app = TuiApp::new("274bot headless");
    app.names = vec!["a".into(), "b".into()];
    app.focused = Some(0);
    app.statuses = vec![
        host_play::SlotStatus {
            username: "a".into(),
            ..host_play::SlotStatus::default()
        },
        host_play::SlotStatus {
            username: "b".into(),
            ..host_play::SlotStatus::default()
        },
    ];
    assert_eq!(
        app.on_key(key(KeyCode::Tab)),
        AppAction::Focus("b".into()),
        "Tab names the newly focused slot so Play::focus follows"
    );
    assert_eq!(app.focused, Some(1));
    assert_eq!(
        app.on_key(key(KeyCode::Tab)),
        AppAction::Focus("a".into()),
        "focus wraps around"
    );
    assert_eq!(app.focused, Some(0));
}

#[test]
fn tab_with_no_running_slots_does_nothing() {
    let mut app = TuiApp::new("274bot headless");
    assert_eq!(app.on_key(key(KeyCode::Tab)), AppAction::None);
}

#[test]
fn strip_click_selects_the_clicked_slot_name() {
    let mut app = TuiApp::new("274bot headless");
    app.names = vec!["a".into(), "b".into()];
    // Strip text: `[a b]  focused: …`. Name spans: `a` at col 1,
    // `b` at col 3.
    assert_eq!(app.on_click(1, 0), AppAction::Focus("a".into()));
    assert_eq!(
        app.focused,
        Some(0),
        "the strip click updates the app focus, not only Play"
    );
    assert_eq!(app.on_click(3, 0), AppAction::Focus("b".into()));
    assert_eq!(
        app.focused,
        Some(1),
        "clicking slot B focuses B in the app too, so UI + input agree"
    );
    // Between the names is a miss.
    assert_eq!(app.on_click(2, 0), AppAction::None);
    assert_eq!(app.focused, Some(1), "a miss keeps the current focus");
}

#[test]
fn full_draw_paints_all_panes() {
    let mut app = TuiApp::new("274bot headless");
    app.names = vec!["test".into()];
    app.focused = Some(0);
    app.statuses = vec![host_play::SlotStatus {
        username: "test".into(),
        ingame: true,
        scene_state: 2,
        tile_x: 10,
        tile_z: 10,
        ..host_play::SlotStatus::default()
    }];
    app.here = Some(api::snapshot::WorldTile {
        x: 10,
        z: 10,
        level: 0,
    });
    app.script_state = RunState::Idle;
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let buf = terminal.backend().buffer();
    let text: String = buf.content().iter().map(|cell| cell.symbol()).collect();
    assert!(text.contains("focused: test"), "strip: {text:?}");
    assert!(text.contains("ingame scene 2"), "status: {text:?}");
    assert!(text.contains("[Start]"), "script shape: {text:?}");
    assert!(text.contains("script: idle"), "script state: {text:?}");
}

#[test]
fn chat_pane_click_routes_to_answer() {
    let mut app = TuiApp::new("274bot headless");
    app.chat_data.modal_texts = vec!["Which way?".into()];
    app.chat_data.options = vec![
        ChatOptionView {
            component_id: 1,
            text: "Yes".into(),
        },
        ChatOptionView {
            component_id: 2,
            text: "No thanks".into(),
        },
    ];
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let chat_area = app.chat_area;
    assert!(chat_area.height >= 4, "chat pane has room for options");
    // Option rows start after border + text + blank (see chat.rs):
    // border row 0, text row 1, blank row 2, options from row 3.
    let row = chat_area.y + 3;
    assert_eq!(
        app.on_click(chat_area.x, row),
        AppAction::Chat(super::ChatAction::Answer(1)),
        "clicking the first option row answers option 1"
    );
}

#[test]
fn chat_data_builds_from_snapshot_views() {
    let mut app = TuiApp::new("274bot headless");
    app.chat_data.lines = vec![line("welcome to 274")];
    assert!(!app.chat_data.is_modal_open());
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let buf = terminal.backend().buffer();
    let text: String = buf.content().iter().map(|cell| cell.symbol()).collect();
    assert!(
        text.contains("welcome to 274"),
        "chat ring paints: {text:?}"
    );
}

/// TR-TUI-001: when the focused script is Paused the pane shows
/// `[Resume]` and clicking it dispatches the pause/resume toggle.
#[test]
fn paused_script_shows_resume_and_click_dispatches_toggle() {
    let mut app = TuiApp::new("274bot headless");
    app.script_state = RunState::Paused;
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let buf = terminal.backend().buffer();
    let text: String = buf.content().iter().map(|cell| cell.symbol()).collect();
    assert!(
        text.contains("[Resume]"),
        "paused script paints Resume: {text:?}"
    );
    assert!(
        !text.contains("[Pause]"),
        "paused script must not paint Pause: {text:?}"
    );
    let area = app.script_area;
    // `[Browse] ` + `[Start] ` → `[Resume] ` at inner.x + 17 = area.x + 18.
    assert_eq!(
        app.on_click(area.x + 18, area.y + 2),
        AppAction::ScriptPause,
        "Resume click dispatches the pause/resume toggle"
    );
}

/// Task 13: with a Browse-selected JS card, clicking Start returns
/// `AppAction::ScriptStart` carrying the card name (tui-play starts the
/// load isolate on the focused slot).
#[test]
fn click_start_with_a_selected_card_returns_script_start() {
    let mut app = TuiApp::new("274bot headless");
    app.script_sel = Some(ScriptSel::Loaded(
        ScriptSource::Catalog,
        "BoneBurier".into(),
    ));
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let area = app.script_area;
    assert_eq!(
        app.on_click(area.x + 10, area.y + 2),
        AppAction::ScriptStart(ScriptSel::Loaded(
            ScriptSource::Catalog,
            "BoneBurier".into(),
        )),
        "Start with a selected card starts that card"
    );
}

#[test]
fn browse_rows_select_a_card_for_start() {
    let mut app = TuiApp::new("274bot headless");
    app.script_cards = vec![
        bone_burier_card(),
        BrowseCard {
            name: "MineRobber".into(),
            description: String::new(),
            category: "Skilling".into(),
            tags: Vec::new(),
            kind: ScriptKind::Compat,
            source: ScriptSource::File,
            unloadable: None,
        },
    ];
    app.script_category_order = vec!["Prayer".into(), "Skilling".into()];
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let area = app.script_area;
    // The buttons row is the second inner line; `[Browse]` is first.
    assert_eq!(
        app.on_click(area.x + 1, area.y + 2),
        AppAction::ScriptBrowse,
        "Browse opens the picker"
    );
    assert!(app.script_browse_open);
    // Re-draw: the picker grows the pane and the card rows start at
    // the third inner line (area.y + 3).
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let area = app.script_area;
    assert_eq!(app.on_click(area.x + 2, area.y + 4), AppAction::None);
    assert_eq!(
        app.script_sel,
        Some(ScriptSel::Loaded(
            ScriptSource::Catalog,
            "BoneBurier".into(),
        )),
        "clicking the first card row selects it"
    );
    assert_eq!(
        app.on_click(area.x + 10, area.y + 2),
        AppAction::ScriptStart(ScriptSel::Loaded(
            ScriptSource::Catalog,
            "BoneBurier".into(),
        )),
        "Start starts the card picked in Browse"
    );
}

/// Task 7: the Load button opens the file browser; Enter on a file
/// produces `AppAction::ScriptLoad` with that path.
#[test]
fn load_browser_enter_returns_script_load() {
    let dir = std::env::temp_dir().join(format!("274bot-tui-load-browser-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let bot = dir.join("digbot.js");
    std::fs::write(&bot, "export function tick(api) { globalThis.__rs_n = 1 }").unwrap();

    let mut app = TuiApp::new("274bot headless");
    app.script_load_dir = dir.clone();
    app.script_load_open = true;
    app.script_load_sel = 1; // [Up]=0, file=1

    assert_eq!(
        app.on_key(key(KeyCode::Enter)),
        AppAction::ScriptLoad(bot),
        "Enter on a file row loads that path"
    );
    assert!(!app.script_load_open, "load browser closes after Enter");

    app.open_script_load_browser(Some(&dir));
    assert_eq!(app.on_key(key(KeyCode::Esc)), AppAction::None);
    assert!(!app.script_load_open, "Esc closes the load browser");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Task 5 fix: clicking `[Params]` opens the popup; Space toggles a
/// bool into the bag Start would post.
#[test]
fn script_params_click_and_space_toggle_persist_bool() {
    let dir = std::env::temp_dir().join(format!("274bot-tui-app-params-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("script-settings.json");
    let mut store = script::ScriptSettingsStore::at(path);
    let loadouts = script::LoadoutsStore::at(dir.join("loadouts.json"));
    let schema = vec![script::SettingDef {
        id: "buryBones".into(),
        ty: "boolean".into(),
        default: Some("true".into()),
        label: Some("Bury bones".into()),
        min: None,
        max: None,
        step: None,
        options: Vec::new(),
        option_labels: Vec::new(),
        group: None,
        show_if: None,
        options_from: None,
        csv_toggle: None,
        help: None,
        item_option_spec: None,
    }];
    let mut app = TuiApp::new("274bot headless");
    app.script_sel = Some(ScriptSel::Loaded(
        ScriptSource::Catalog,
        "ChickenKiller".into(),
    ));
    app.params_schema = schema;
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let area = app.script_area;
    assert_eq!(
        app.on_click(area.x + 1, area.y + 3),
        AppAction::ScriptParams,
        "[Params] opens the popup"
    );
    app.open_script_params(&store);
    assert!(app.params_state.open);
    app.params_on_key(&mut store, &loadouts, None, key(KeyCode::Char(' ')));
    assert_eq!(
        app.params_bag.get("buryBones"),
        Some(&serde_json::json!(false))
    );
    let start_bag = app.merged_script_settings_bag(&store).expect("merged bag");
    assert_eq!(
        start_bag.get("buryBones"),
        Some(&serde_json::json!(false)),
        "Start would post the toggled bool"
    );
    terminal
        .draw(|frame| {
            app.draw(frame);
            app.draw_params_overlay(frame, &mut store, &loadouts, None);
        })
        .unwrap();
    let buf = terminal.backend().buffer();
    let text: String = buf.content().iter().map(|cell| cell.symbol()).collect();
    assert!(
        text.contains("parameters"),
        "params overlay paints: {text:?}"
    );
}

#[test]
fn script_params_numeric_edit_persists_and_global_keys_stay_consumed() {
    let dir = std::env::temp_dir().join(format!(
        "274bot-tui-app-alcher-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let mut store = script::ScriptSettingsStore::at(dir.join("script-settings.json"));
    let loadouts = script::LoadoutsStore::at(dir.join("loadouts.json"));
    let schema = vec![script::SettingDef {
        id: "alchs".into(),
        ty: "number".into(),
        default: Some("27".into()),
        label: Some("Alchs per trip".into()),
        min: None,
        max: None,
        step: None,
        options: Vec::new(),
        option_labels: Vec::new(),
        group: None,
        show_if: None,
        options_from: None,
        csv_toggle: None,
        help: None,
        item_option_spec: None,
    }];
    let mut app = TuiApp::new("274bot headless");
    app.script_sel = Some(ScriptSel::Loaded(ScriptSource::Catalog, "Alcher".into()));
    app.params_schema = schema;
    app.open_script_params(&store);
    assert!(app.params_state.open);
    assert_eq!(app.on_key(key(KeyCode::Char('q'))), AppAction::None);
    assert!(!app.quit, "params overlay must consume q");
    app.params_on_key(&mut store, &loadouts, None, key(KeyCode::Enter));
    assert!(app.params_state.editing);
    while !app.params_state.scratch.is_empty() {
        app.params_on_key(&mut store, &loadouts, None, key(KeyCode::Backspace));
    }
    app.params_on_key(&mut store, &loadouts, None, key(KeyCode::Char('5')));
    app.params_on_key(&mut store, &loadouts, None, key(KeyCode::Esc));
    assert!(!app.params_state.editing);
    assert_eq!(
        app.params_bag.get("alchs").and_then(|v| v.as_f64()),
        Some(27.0)
    );
    app.params_on_key(&mut store, &loadouts, None, key(KeyCode::Enter));
    while !app.params_state.scratch.is_empty() {
        app.params_on_key(&mut store, &loadouts, None, key(KeyCode::Backspace));
    }
    app.params_on_key(&mut store, &loadouts, None, key(KeyCode::Char('5')));
    app.params_on_key(&mut store, &loadouts, None, key(KeyCode::Enter));
    assert_eq!(
        app.params_bag.get("alchs").and_then(|v| v.as_f64()),
        Some(5.0)
    );
    let start_bag = app.merged_script_settings_bag(&store).expect("merged bag");
    assert_eq!(start_bag.get("alchs").and_then(|v| v.as_f64()), Some(5.0));
    app.params_on_key(&mut store, &loadouts, None, key(KeyCode::Esc));
    assert!(!app.params_state.open);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Task 13: while the focused slot's script paints, the chat pane
/// shows the paint title and rows instead of the game chat; the `p`
/// key toggles back to the game chat.
#[test]
fn chat_pane_shows_script_paint_instead_of_the_game_chat() {
    let mut app = TuiApp::new("274bot headless");
    app.chat_data.lines = vec![line("last game chat line")];
    app.chat_data.script_paint = Some(std::sync::Arc::new(script::shim::ScriptPaint {
        title: Some("BoneBurier — digging".into()),
        accent: Some("#f3e6a2".into()),
        lines: vec!["Runtime: 1.2m | Buried: 3".into(), "".into()],
        buttons: Vec::new(),
        generation: 0,
        canvas: Vec::new(),
        ..Default::default()
    }));
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let buf = terminal.backend().buffer();
    let text: String = buf.content().iter().map(|cell| cell.symbol()).collect();
    assert!(
        text.contains("BoneBurier — digging"),
        "the paint title paints: {text:?}"
    );
    assert!(
        text.contains("Runtime: 1.2m | Buried: 3"),
        "paint rows paint: {text:?}"
    );
    assert!(
        !text.contains("last game chat line"),
        "the game chat is replaced by the paint: {text:?}"
    );
    // The toggle key brings the game chat back.
    assert_eq!(app.on_key(key(KeyCode::Char('p'))), AppAction::None);
    assert!(app.chat_data.show_game_chat);
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let buf = terminal.backend().buffer();
    let text: String = buf.content().iter().map(|cell| cell.symbol()).collect();
    assert!(
        text.contains("last game chat line"),
        "p toggles back to the game chat: {text:?}"
    );
    assert!(
        !text.contains("BoneBurier — digging"),
        "the paint is hidden while toggled off: {text:?}"
    );
}
