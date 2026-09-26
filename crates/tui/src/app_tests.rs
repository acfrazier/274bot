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

use host_play::walk_map::{
    ActionError, ActionKind, AuthenticatedServices, Catalogue, MapContext, WalkExclude,
    WalkSlotReady, WalkSlotStatus,
};
use nav::map::formats::{
    ClientPois, Coverage, CoverageLevel, ServiceIdentity, ServicePois, NAVPOIS_VERSION,
};
use nav::map::identity::{CatalogueIdentity, Digest};
use nav::map::poi::{
    CapabilityEvidence, DisplayAnchor, EntityKind, Footprint, PoiKey, PoiKind, PoiRecord,
    SourceSpace,
};
use nav::map::{Rows, Text};

use super::{wasd_target, AppAction, MapCatalogueStatus, TuiApp, WalkSendMode};

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

fn bank_poi(name: &str, x: i32, z: i32) -> PoiRecord {
    PoiRecord {
        key: PoiKey {
            entity: EntityKind::Loc,
            id: 2213,
            x,
            z,
            source: SourceSpace::Game { plane: 0 },
            shape: 10,
            rotation: 0,
        },
        name: Text::new(name).unwrap(),
        kind: PoiKind::Bank,
        effective_plane: 0,
        footprint: Footprint {
            width: 1,
            length: 1,
        },
        display: DisplayAnchor {
            x: f64::from(x) + 0.5,
            z: f64::from(z) + 0.5,
            plane: 0,
        },
        evidence: Rows::new(vec![CapabilityEvidence::ActiveQuickBooth]).unwrap(),
        walk_target: None,
    }
}

fn digest(n: u8) -> Digest {
    Digest([n; 32])
}

fn coverage() -> Coverage {
    Coverage {
        npc_placements: CoverageLevel::Unavailable,
        bank_services: CoverageLevel::Limited,
        place_labels: CoverageLevel::Unavailable,
        unresolved: Rows::new(vec![]).unwrap(),
    }
}

fn poi_record(
    entity: EntityKind,
    x: i32,
    z: i32,
    source: SourceSpace,
    kind: PoiKind,
    name: &str,
) -> PoiRecord {
    let plane = source.game_plane().unwrap().unwrap();
    PoiRecord {
        key: PoiKey {
            entity,
            id: if entity == EntityKind::Loc { 2213 } else { 1 },
            x,
            z,
            source,
            shape: if entity == EntityKind::Loc { 10 } else { 0 },
            rotation: 0,
        },
        name: Text::new(name).unwrap(),
        kind,
        effective_plane: plane,
        footprint: Footprint {
            width: 1,
            length: 1,
        },
        display: DisplayAnchor {
            x: f64::from(x) + 0.5,
            z: f64::from(z) + 0.5,
            plane,
        },
        evidence: Rows::new(vec![CapabilityEvidence::ActiveQuickBooth]).unwrap(),
        walk_target: None,
    }
}

fn map_poi_catalogue() -> (std::sync::Arc<Catalogue>, Tile, Tile) {
    let world = std::sync::Arc::new(nav::world::NavWorld::from_grid(
        &nav::grid::StepGrid::fixture_open_3x3(),
    ));
    let identity = CatalogueIdentity {
        revision: 289,
        content: digest(1),
        policy: digest(2),
    };
    let nav = digest(8);
    let booth = poi_record(
        EntityKind::Loc,
        1,
        1,
        SourceSpace::ClientVisual {
            plane: 0,
            link_below: false,
        },
        PoiKind::Bank,
        "Bank booth",
    );
    let client = std::sync::Arc::new(ClientPois {
        schema: 1,
        identity,
        coverage: coverage(),
        records: Rows::new(vec![booth]).unwrap(),
    });
    let mut labels = vec![poi_record(
        EntityKind::Label,
        10,
        10,
        SourceSpace::ServerGame { plane: 0 },
        PoiKind::Label { priority: 1 },
        "Lumbridge",
    )];
    labels.sort_by_key(|r| r.key);
    let doc = ServicePois {
        schema: NAVPOIS_VERSION,
        identity: ServiceIdentity {
            revision: 289,
            content: digest(1),
            nav_sha256: nav,
            source_sha256: digest(3),
            generator_sha256: digest(4),
            policy: digest(5),
        },
        coverage: coverage(),
        records: Rows::new(labels).unwrap(),
    };
    let bytes = doc.encode_navpois().unwrap();
    let services = AuthenticatedServices::decode(&bytes, doc.identity, Digest::of(&bytes)).unwrap();
    let catalogue = Catalogue::new(world, identity, nav, Some(client), Some(services)).unwrap();
    let booth = catalogue
        .entries()
        .find(|e| e.name() == "Bank booth")
        .expect("booth");
    let label = catalogue
        .entries()
        .find(|e| e.name() == "Lumbridge")
        .expect("label");
    let stand = booth.walk_target().expect("physical stand");
    let label_anchor = label.anchor();
    assert_ne!(
        stand,
        booth.anchor(),
        "fixture must distinguish catalogue stand from the display snap"
    );
    assert_eq!(label.walk_target(), None, "view-only label has no stand");
    (std::sync::Arc::new(catalogue), stand, label_anchor)
}

fn bind_catalogue_map(app: &mut TuiApp, catalogue: std::sync::Arc<Catalogue>) {
    app.world = Some(std::sync::Arc::clone(catalogue.world()));
    app.names = vec!["alice".into()];
    app.focused = Some(0);
    app.here = Some(WorldTile {
        x: 0,
        z: 0,
        level: 0,
    });
    app.map_model.bind(MapContext {
        focus: None,
        nav: catalogue.nav_identity(),
        overlay: None,
        generation: 1,
    });
    assert_eq!(app.on_key(key(KeyCode::F(4))), AppAction::MapOpen);
    app.bind_host_catalogue(catalogue);
}

fn search_and_jump(app: &mut TuiApp, query: &str) {
    assert_eq!(app.on_key(key(KeyCode::Char('/'))), AppAction::None);
    for ch in query.chars() {
        app.on_key(key(KeyCode::Char(ch)));
    }
    assert_eq!(app.on_key(key(KeyCode::Enter)), AppAction::None);
}

fn open_map_world() -> TuiApp {
    let mut app = TuiApp::new("274bot headless");
    app.names = vec!["alice".into(), "bob".into()];
    app.focused = Some(0);
    app.statuses = vec![
        host_play::SlotStatus {
            username: "alice".into(),
            ingame: true,
            scene_state: 2,
            tile_x: 1,
            tile_z: 1,
            ..host_play::SlotStatus::default()
        },
        host_play::SlotStatus {
            username: "bob".into(),
            ..host_play::SlotStatus::default()
        },
    ];
    app.world = Some(Arc::new(nav::world::NavWorld::from_grid(
        &nav::grid::StepGrid::fixture_open_3x3(),
    )));
    app.here = Some(WorldTile {
        x: 1,
        z: 1,
        level: 0,
    });
    app.map_pois = vec![bank_poi("Varrock East", 2, 2)];
    app
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

/// Boot draws a neutral inactive Map pane and never requests catalogue work.
#[test]
fn draws_title_containing_274bot_without_map_demand() {
    let mut app = TuiApp::new("274bot headless");
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();

    let buf = terminal.backend().buffer();
    let text: String = buf.content().iter().map(|cell| cell.symbol()).collect();
    assert!(
        text.contains("274bot"),
        "buffer does not contain title: {text:?}"
    );
    assert!(
        text.contains("F4 activate"),
        "map is explicit at boot: {text:?}"
    );
    assert_eq!(
        app.map_catalogue_status,
        super::MapCatalogueStatus::Inactive,
        "draw must not demand a catalogue"
    );
}

/// Map activation is explicit; loaded collision/POIs then render in cells.
#[test]
fn draw_map_paints_walkable_dots_after_explicit_activation() {
    let mut app = TuiApp::new("274bot headless");
    app.world = Some(Arc::new(nav::world::NavWorld::from_grid(
        &nav::grid::StepGrid::fixture_open_3x3(),
    )));
    app.here = Some(api::snapshot::WorldTile {
        x: 1,
        z: 1,
        level: 0,
    });
    assert_eq!(app.on_key(key(KeyCode::F(4))), AppAction::MapOpen);
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let buf = terminal.backend().buffer();
    let text: String = buf.content().iter().map(|cell| cell.symbol()).collect();
    assert!(text.contains('.'), "walkable tiles paint as dots: {text:?}");
    assert!(
        text.contains('@'),
        "the here marker paints on the player tile: {text:?}"
    );
    assert!(text.contains("coverage:"), "coverage is visible: {text:?}");
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
        AppAction::None,
        "Enter must not confirm a map selection outside Map focus"
    );
    assert_eq!(app.on_key(key(KeyCode::F(4))), AppAction::MapOpen);
    assert!(app.map_model.pending().is_none());
    assert_eq!(
        app.on_key(key(KeyCode::Enter)),
        AppAction::None,
        "Enter with no pending selection only selects"
    );
    assert!(app.map_model.pending().is_some());
    assert_eq!(
        app.on_key(key(KeyCode::Enter)),
        AppAction::ArmWalk(tile(1, 1)),
        "the next Enter confirms the pending centre selection"
    );
}

#[test]
fn map_focus_reserves_l_for_pan_and_esc_orders_search_before_close() {
    let mut app = TuiApp::new("274bot headless");
    assert_eq!(app.on_key(key(KeyCode::Char('l'))), AppAction::None);
    assert!(app.loadouts_state.open, "l remains loadouts outside Map");
    app.loadouts_state.open = false;
    app.world = Some(Arc::new(nav::world::NavWorld::from_grid(
        &nav::grid::StepGrid::fixture_open_3x3(),
    )));
    assert_eq!(app.on_key(key(KeyCode::F(4))), AppAction::MapOpen);
    let before = app.map.pan;
    assert_eq!(app.on_key(key(KeyCode::Char('l'))), AppAction::None);
    assert_ne!(app.map.pan, before, "l pans only in Map focus");
    assert!(!app.loadouts_state.open);
    app.map_search_open = true;
    assert_eq!(app.on_key(key(KeyCode::Esc)), AppAction::None);
    assert!(app.map_active, "first Esc closes search only");
    assert_eq!(app.on_key(key(KeyCode::Esc)), AppAction::MapClose);
    assert!(!app.map_active);
}

fn run_map_keyboard_walkthrough(width: u16, height: u16) {
    let mut app = open_map_world();
    assert_eq!(
        app.map_catalogue_status,
        MapCatalogueStatus::Inactive,
        "no catalogue demand before F4"
    );
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    assert_eq!(
        app.map_catalogue_status,
        MapCatalogueStatus::Inactive,
        "draw must not demand a catalogue"
    );
    assert_eq!(
        app.on_key(key(KeyCode::Enter)),
        AppAction::None,
        "Enter outside Map focus does not confirm"
    );

    assert_eq!(app.on_key(key(KeyCode::F(4))), AppAction::MapOpen);
    assert_ne!(
        app.map_catalogue_status,
        MapCatalogueStatus::Inactive,
        "F4 is the catalogue-demand transition"
    );
    app.refresh_walk_send(|name| {
        if name == "alice" {
            WalkSlotStatus::Eligible(WalkSlotReady { origin: tile(1, 1) })
        } else {
            WalkSlotStatus::Excluded(WalkExclude::NoPosition)
        }
    });
    app.map_observed = vec![host_play::walk_map::ObservedService {
        npc_index: 1,
        kind: PoiKind::Bank,
        tile: tile(1, 1),
        evidence: CapabilityEvidence::ActiveQuickBooth,
        context: host_play::walk_map::MapContext {
            focus: None,
            nav: nav::map::identity::Digest::of(b"walkthrough"),
            overlay: None,
            generation: 1,
        },
    }];

    assert_eq!(app.on_key(key(KeyCode::Char('/'))), AppAction::None);
    assert!(app.map_search_open);
    for ch in "varrock".chars() {
        app.on_key(key(KeyCode::Char(ch)));
    }
    assert_eq!(app.on_key(key(KeyCode::Enter)), AppAction::None);
    assert_eq!(app.map_poi_sel, Some(0), "search Enter jumps to the POI");
    assert!(app.map_model.pending().is_none());

    assert_eq!(app.on_key(key(KeyCode::Char('/'))), AppAction::None);
    for ch in "2,2,0".chars() {
        app.on_key(key(KeyCode::Char(ch)));
    }
    assert_eq!(app.on_key(key(KeyCode::Enter)), AppAction::None);
    assert_eq!(app.map.selection, Some(tile(2, 2)));
    assert!(app.map_model.pending().is_some());
    assert_eq!(
        app.on_key(key(KeyCode::Char('t'))),
        AppAction::MapTeleport(tile(2, 2)),
        "t teleports the requested tile"
    );

    assert_eq!(app.on_key(key(KeyCode::PageUp)), AppAction::None);
    assert_eq!(app.map.plane, 1);
    assert!(
        app.map_model.pending().is_none(),
        "plane change clears pending selection"
    );
    let dots_before = app.map.layers.dots;
    assert_eq!(app.on_key(key(KeyCode::Char('d'))), AppAction::None);
    assert_ne!(app.map.layers.dots, dots_before);
    let collision_before = app.map.layers.collision;
    assert_eq!(app.on_key(key(KeyCode::Char('c'))), AppAction::None);
    assert_ne!(app.map.layers.collision, collision_before);
    let reach_before = app.map.layers.reach;
    assert_eq!(app.on_key(key(KeyCode::Char('r'))), AppAction::None);
    assert_ne!(app.map.layers.reach, reach_before);
    assert_eq!(app.on_key(key(KeyCode::Char('R'))), AppAction::None);
    assert_eq!(app.map.plane, 0);
    assert!(
        app.map_model.pending().is_none(),
        "recenter clears pending selection"
    );

    assert_eq!(app.on_key(key(KeyCode::Char('/'))), AppAction::None);
    for ch in "3,3,0".chars() {
        app.on_key(key(KeyCode::Char(ch)));
    }
    assert_eq!(app.on_key(key(KeyCode::Enter)), AppAction::None);
    assert_eq!(
        app.map.selection,
        Some(Tile {
            x: 3,
            z: 3,
            level: 0
        }),
        "blocked tiles remain selectable for Teleport"
    );
    assert_eq!(
        app.on_key(key(KeyCode::Char('t'))),
        AppAction::MapTeleport(Tile {
            x: 3,
            z: 3,
            level: 0
        })
    );

    assert_eq!(app.on_key(key(KeyCode::Char('/'))), AppAction::None);
    for ch in "2,2,0".chars() {
        app.on_key(key(KeyCode::Char(ch)));
    }
    assert_eq!(app.on_key(key(KeyCode::Enter)), AppAction::None);
    assert_eq!(
        app.on_key(key(KeyCode::Enter)),
        AppAction::ArmWalk(tile(2, 2))
    );
    assert!(app.map_model.pending().is_some());

    assert_eq!(app.on_key(key(KeyCode::Char('g'))), AppAction::None);
    assert_eq!(app.walk_send.mode, WalkSendMode::Group);
    assert_eq!(app.walk_send.walk_label(), "Walk 1 bots");
    assert_eq!(
        app.on_key(key(KeyCode::Enter)),
        AppAction::MapWalkGroup,
        "group Enter walks the checked fleet"
    );
    assert!(!app.quit);

    terminal.draw(|frame| app.draw(frame)).unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(
        text.contains("plane 0"),
        "{width}x{height} map title: {text:?}"
    );
    assert!(
        text.contains("legend:"),
        "{width}x{height} map legend: {text:?}"
    );
    assert!(
        text.contains("Walk 1 bots"),
        "{width}x{height} group walk label: {text:?}"
    );
    assert!(
        text.contains("no position yet"),
        "{width}x{height} eligibility: {text:?}"
    );
    assert!(
        text.contains("obs:1"),
        "{width}x{height} observed services: {text:?}"
    );

    assert_eq!(app.on_key(key(KeyCode::F(4))), AppAction::MapClose);
    assert_eq!(app.map_catalogue_status, MapCatalogueStatus::Inactive);
    assert!(app.map_pois.is_empty(), "close releases the catalogue");
    assert!(app.map_observed.is_empty());
    assert!(app.map_host_catalogue.is_none());
}

#[test]
fn map_keyboard_walkthrough_120x40() {
    run_map_keyboard_walkthrough(120, 40);
}

#[test]
fn map_keyboard_walkthrough_80x24() {
    run_map_keyboard_walkthrough(80, 24);
}

#[test]
fn map_poi_confirm_keeps_catalogue_stand_and_blocks_view_only_labels() {
    let (catalogue, stand, label_anchor) = map_poi_catalogue();
    let ctx = MapContext {
        focus: None,
        nav: catalogue.nav_identity(),
        overlay: Some(catalogue.key()),
        generation: 1,
    };
    let mut app = TuiApp::new("274bot headless");
    bind_catalogue_map(&mut app, Arc::clone(&catalogue));

    search_and_jump(&mut app, "lumbridge");
    let pending = app.map_model.pending().expect("label selection");
    assert_eq!(pending.requested, label_anchor);
    assert_eq!(pending.target, None, "view-only label is not a walk stand");
    assert_eq!(
        app.on_key(key(KeyCode::Enter)),
        AppAction::ArmWalk(label_anchor)
    );
    assert_eq!(
        app.map_model
            .confirm(ActionKind::Walk, &ctx, Some(tile(0, 0)), Default::default())
            .expect_err("view-only label"),
        ActionError::Blocked
    );
    assert!(app.map_model.pending().is_none());
    assert_eq!(
        app.map.selection,
        Some(label_anchor),
        "host consume leaves the leftover crosshair"
    );
    assert_eq!(
        app.on_key(key(KeyCode::Enter)),
        AppAction::None,
        "Enter after a consumed label must not walk"
    );
    assert!(app.walk_dest.is_none());

    app.map_model.clear_selection();
    app.map.selection = None;
    search_and_jump(&mut app, "booth");
    let pending = app.map_model.pending().expect("booth selection");
    assert_eq!(pending.target, Some(stand));
    assert_ne!(pending.requested, stand);
    let requested = pending.requested;
    assert_eq!(
        app.on_key(key(KeyCode::Enter)),
        AppAction::ArmWalk(requested)
    );
    assert_eq!(
        app.map_model.pending().map(|p| p.target),
        Some(Some(stand)),
        "confirm must keep the catalogue stand, not a radius snap"
    );
    assert_eq!(
        app.map_model
            .confirm(ActionKind::Walk, &ctx, Some(tile(0, 0)), Default::default())
            .expect_err("no focus token in this headless bind"),
        ActionError::NoFocus
    );
    assert!(app.map_model.pending().is_none());
    assert_eq!(app.map.selection, Some(requested));
    assert_eq!(
        app.on_key(key(KeyCode::Enter)),
        AppAction::None,
        "Enter after a consumed POI walk must not snap a second walk"
    );

    app.map_model.clear_selection();
    app.map.selection = None;
    search_and_jump(&mut app, "booth");
    assert_eq!(app.on_key(key(KeyCode::Char('g'))), AppAction::None);
    assert_eq!(app.on_key(key(KeyCode::Enter)), AppAction::MapWalkGroup);
    assert_eq!(
        app.map_model.pending().expect("group").target,
        Some(stand),
        "group confirm must not replace the POI stand with a snap"
    );
}

#[test]
fn map_status_shows_walk_refusal_at_120_and_80() {
    let mut app = open_map_world();
    assert_eq!(app.on_key(key(KeyCode::F(4))), AppAction::MapOpen);
    app.error = Some(ActionError::NoOrigin.to_string());
    app.walk_dest = None;
    for (width, height) in [(120, 40), (80, 24)] {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(
            text.contains("status: Walk/Teleport unavailable: no observed player"),
            "{width}x{height} missing refusal: {text:?}"
        );
    }
}

#[test]
fn map_search_types_j_and_k_and_navigates_with_arrows() {
    let mut app = open_map_world();
    app.map_pois = vec![
        bank_poi("Varrock East", 2, 2),
        bank_poi("Varrock West", 1, 1),
        bank_poi("Bank kebab jewellery", 0, 0),
    ];
    assert_eq!(app.on_key(key(KeyCode::F(4))), AppAction::MapOpen);
    assert_eq!(app.on_key(key(KeyCode::Char('/'))), AppAction::None);
    for ch in "bank kebab jewellery".chars() {
        app.on_key(key(KeyCode::Char(ch)));
    }
    assert_eq!(
        app.map_search, "bank kebab jewellery",
        "j/k in the search editor must type, not move the result cursor"
    );
    assert_eq!(app.map_search_sel, 0);

    app.map_search.clear();
    app.map_search_sel = 0;
    for ch in "varrock".chars() {
        app.on_key(key(KeyCode::Char(ch)));
    }
    assert_eq!(app.map_search_results.len(), 2);
    assert_eq!(app.on_key(key(KeyCode::Down)), AppAction::None);
    assert_eq!(app.map_search_sel, 1);
    assert_eq!(app.on_key(key(KeyCode::Up)), AppAction::None);
    assert_eq!(app.map_search_sel, 0);
    assert_eq!(
        app.on_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL)),
        AppAction::None
    );
    assert_eq!(app.map_search_sel, 1);
    assert_eq!(
        app.on_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL)),
        AppAction::None
    );
    assert_eq!(app.map_search_sel, 0);
    assert_eq!(app.map_search, "varrock");
}

#[test]
fn new_map_selection_clears_a_previous_blocked_status() {
    let mut app = open_map_world();
    assert_eq!(app.on_key(key(KeyCode::F(4))), AppAction::MapOpen);
    app.error = Some(ActionError::Blocked.to_string());
    assert_eq!(app.on_key(key(KeyCode::Enter)), AppAction::None);
    assert!(
        app.error.is_none(),
        "a new view-centre selection must clear the leftover Blocked status"
    );
    app.error = Some(ActionError::Blocked.to_string());
    assert_eq!(app.on_key(key(KeyCode::Char('/'))), AppAction::None);
    for ch in "2,2,0".chars() {
        app.on_key(key(KeyCode::Char(ch)));
    }
    assert_eq!(app.on_key(key(KeyCode::Enter)), AppAction::None);
    assert!(
        app.error.is_none(),
        "a new coordinate selection must clear the leftover Blocked status"
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

#[test]
fn esc_acks_background_notice_when_map_has_no_selection() {
    let mut app = TuiApp::new("274bot headless");
    app.background_notice = Some("other profiles keep running".into());
    assert_eq!(app.on_key(key(KeyCode::Esc)), AppAction::AckBackground);
    app.map.selection = Some(tile(1, 1));
    app.background_notice = Some("other profiles keep running".into());
    assert_ne!(
        app.on_key(key(KeyCode::Esc)),
        AppAction::AckBackground,
        "Esc still clears a map selection first"
    );
}
