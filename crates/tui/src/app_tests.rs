use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use api::snapshot::{ChatLineView, ChatOptionView, WorldTile};
use nav::tile::Tile;
use script::{RunState, ScriptKind, ScriptSel, ScriptSource};
use vault::ProfileSettings;

use crate::layout::Screen;
use crate::overlay::Modal;
use crate::script_shape::BrowseCard;
use crate::test_support::{ch, draw, find, key, text};

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
use frontend_core::{FleetRow, Phase, SlotDetail};

/// A projected member row (what the core publishes).
fn member(name: &str, phase: Phase) -> FleetRow {
    FleetRow::fixture(name, phase, None, None)
}

/// The selected slot's projected detail while it is in game on `tile`.
fn ready_detail(name: &str, tile: (i32, i32, i32)) -> SlotDetail {
    SlotDetail {
        row: member(name, Phase::Ready),
        state: "ingame scene 2".into(),
        tile,
        ready_tile: Some(tile),
        ..SlotDetail::default()
    }
}

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
        "Lumbridge/Swamp",
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
        .find(|e| e.name() == "Lumbridge/Swamp")
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
    assert_eq!(app.on_key(ch('/')), AppAction::None);
    for c in query.chars() {
        app.on_key(ch(c));
    }
    assert_eq!(app.on_key(key(KeyCode::Enter)), AppAction::None);
}

fn open_map_world() -> TuiApp {
    let mut app = TuiApp::new("274bot headless");
    app.names = vec!["alice".into(), "bob".into()];
    app.focused = Some(0);
    app.fleet = vec![member("alice", Phase::Ready), member("bob", Phase::Offline)];
    app.detail = Some(ready_detail("alice", (1, 1, 0)));
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

/// A one-bot app on the Script tab (keyboard on the tab).
fn script_app() -> TuiApp {
    let mut app = crate::test_support::fleet_app(&["alice", "bob"]);
    assert_eq!(app.show_screen(Screen::Script), AppAction::None);
    app
}

fn thiever_schema() -> Vec<script::SettingDef> {
    vec![script::SettingDef {
        id: "target".into(),
        ty: "string".into(),
        default: Some("Man".into()),
        label: None,
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
    }]
}

/// Boot shows the fleet and never requests catalogue work.
#[test]
fn boot_draws_the_fleet_without_map_demand() {
    let mut app = TuiApp::new("274bot headless");
    let rows = draw(&mut app, 80, 24);
    let all = text(&rows);
    assert!(rows[0].contains("274bot"), "title: {all}");
    assert!(
        all.contains("no bots loaded: m loads every vault profile"),
        "an empty fleet says how to load: {all}"
    );
    assert_eq!(
        app.map_catalogue_status,
        MapCatalogueStatus::Inactive,
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
    let all = text(&draw(&mut app, 80, 24));
    assert!(all.contains('.'), "walkable tiles paint as dots: {all:?}");
    assert!(
        all.contains('@'),
        "the here marker paints on the player tile: {all:?}"
    );
    assert!(all.contains("coverage:"), "coverage is visible: {all:?}");
}
#[test]
fn wilderness_persistence_errors_are_visible() {
    let root =
        std::env::temp_dir().join(format!("274bot-tui-map-pref-error-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let parent = root.join("not-a-directory");
    std::fs::write(&parent, b"file").unwrap();

    let mut app = TuiApp::new("274bot headless");
    app.restore_preferences(parent.join("panel-ui.json"));
    app.toggle_map_wilderness();

    assert!(app.map.layers.wilderness);
    assert!(
        app.error
            .as_deref()
            .is_some_and(|error| error.starts_with("map wilderness: ")),
        "preference write failure must be surfaced: {:?}",
        app.error
    );

    std::fs::remove_file(&parent).unwrap();
    std::fs::create_dir(&parent).unwrap();
    app.toggle_map_wilderness();
    assert!(!app.map.layers.wilderness);
    assert!(
        app.error.is_none(),
        "a later successful save clears the stale map error: {:?}",
        app.error
    );

    std::fs::remove_dir_all(root).unwrap();
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

/// WASD walks only while Manual walk is armed, keeps the player's plane,
/// and Esc disarms it. Outside Manual walk the letters are not walks.
#[test]
fn wasd_walks_only_while_manual_walk_is_armed() {
    let mut app = TuiApp::new("274bot headless");
    app.names = vec!["test".into()];
    app.focused = Some(0);
    app.detail = Some(ready_detail("test", (10, 10, 1)));
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
    for c in ['w', 'a', 's', 'd'] {
        assert!(
            !matches!(app.on_key(ch(c)), AppAction::WalkTile(_)),
            "{c} in the fleet pane is not a walk"
        );
    }
    app.show_screen(Screen::Overview);
    assert_eq!(app.on_key(ch('w')), AppAction::None, "w arms Manual walk");
    assert_eq!(app.modal, Some(Modal::Manual));
    let north = Tile {
        x: 10,
        z: 11,
        level: 1,
    };
    assert_eq!(app.on_key(ch('w')), AppAction::WalkTile(north));
    assert_eq!(app.on_key(key(KeyCode::Up)), AppAction::WalkTile(north));
    assert_eq!(
        app.on_key(ch('d')),
        AppAction::WalkTile(Tile {
            x: 11,
            z: 10,
            level: 1,
        })
    );
    assert_eq!(app.on_key(key(KeyCode::Esc)), AppAction::None);
    assert_eq!(app.modal, None, "Esc disarms");
    assert_eq!(app.on_key(ch('s')), AppAction::None, "s is not a walk now");
}

/// A chat modal is answered from the Chat tab only.
#[test]
fn chat_tab_routes_enter_to_continue() {
    let mut app = TuiApp::new("274bot headless");
    app.chat_data.modal_texts = vec!["The stranger waits.".into()];
    app.chat_data.has_continue = true;
    app.show_screen(Screen::Chat);
    assert_eq!(
        app.on_key(key(KeyCode::Enter)),
        AppAction::Chat(super::ChatAction::Continue),
        "Enter on the Chat tab continues the dialog"
    );
}

#[test]
fn chat_tab_answers_options_on_space() {
    let mut app = TuiApp::new("274bot headless");
    app.chat_data.modal_texts = vec!["Which way?".into()];
    app.chat_data.options = vec![ChatOptionView {
        component_id: 1,
        text: "Yes".into(),
    }];
    app.chat_data.has_continue = true;
    app.show_screen(Screen::Chat);
    assert_eq!(
        app.on_key(ch(' ')),
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
    app.show_screen(Screen::Chat);
    assert_eq!(
        app.on_key(ch('1')),
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
    let mut app = TuiApp::new("274bot headless");
    app.chat_data.script_paint = Some(std::sync::Arc::new(nature_crafter_paint()));
    app.show_screen(Screen::Chat);
    let rows = draw(&mut app, 140, 40);

    let (button_col, button_row) =
        find(&rows, "[1] Go bank").expect("the advertised NatureCrafter button must be visible");
    let (title_col, title_row) =
        find(&rows, "NatureCrafter — Air").expect("the paint title must remain visible");
    let (body_col, body_row) =
        find(&rows, "Runtime: 1m").expect("the first paint status row must remain visible");

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
    let mut app = TuiApp::new("274bot headless");
    app.chat_data.script_paint = Some(std::sync::Arc::new(nature_crafter_paint()));
    app.show_screen(Screen::Chat);
    let rows = draw(&mut app, 48, 18);
    let (button_col, button_row) = find(&rows, "[1] Go bank")
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
fn manual_movement_pause_toggle_is_reachable_and_last_settings_row_stays_clamped() {
    let mut app = TuiApp::new("274bot headless");
    assert!(app.pause_script_on_manual_walk_abort);
    app.settings_state.open = true;
    for _ in 0..6 {
        assert_eq!(app.on_key(key(KeyCode::Down)), AppAction::None);
    }
    assert_eq!(app.settings_state.row, 6);
    assert_eq!(app.on_key(key(KeyCode::Enter)), AppAction::None);
    assert!(!app.pause_script_on_manual_walk_abort);
    assert!(app.pause_script_on_manual_walk_abort_dirty);

    for _ in 0..20 {
        app.on_key(key(KeyCode::Down));
    }
    assert_eq!(
        app.settings_state.row, 8,
        "memory remains reachable at the last row"
    );
}

/// The settings popup owns the keyboard: global letters do not leak out
/// (the old `q`/`x` in settings quit or removed the focused bot).
#[test]
fn settings_popup_keeps_letters_from_quitting_or_removing() {
    let mut app = crate::test_support::fleet_app(&["alice"]);
    app.show_screen(Screen::Overview);
    assert_eq!(app.on_key(ch('o')), AppAction::None);
    assert!(app.settings_state.open, "o opens the settings popup");
    for c in ['q', 'x', 'i', 'm'] {
        assert_eq!(app.on_key(ch(c)), AppAction::None, "{c} stays in settings");
    }
    assert!(!app.quit);
    assert!(
        app.modal.is_none(),
        "no confirmation opened behind settings"
    );
    assert_eq!(app.on_key(key(KeyCode::Esc)), AppAction::None);
    assert!(!app.settings_state.open);
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
    assert!(
        !matches!(app.on_key(key(KeyCode::Enter)), AppAction::ArmWalk(_)),
        "Enter must not confirm a map selection outside the Map tab"
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
fn map_zone_toggle_selects_request_policy_and_resets_per_open() {
    let mut app = open_map_world();
    app.nav.allow_teleports = true;

    assert_eq!(app.on_key(key(KeyCode::F(4))), AppAction::MapOpen);
    assert_eq!(app.map_find_options().zones, nav::zones::ZoneExempt::NONE);
    assert!(app.map_find_options().allow_teleports);

    assert_eq!(app.on_key(ch('z')), AppAction::None);
    assert!(app.map_route_through_zones);
    assert!(app.map_find_options().zones.is_all());
    let crossing = text(&draw(&mut app, 120, 40));
    assert!(
        crossing.contains("zones: crossing (z)"),
        "Map info must show the live zone choice: {crossing}"
    );

    assert_eq!(app.on_key(key(KeyCode::Esc)), AppAction::MapClose);
    assert!(!app.map_route_through_zones);
    assert_eq!(app.map_find_options().zones, nav::zones::ZoneExempt::NONE);

    assert_eq!(app.on_key(key(KeyCode::F(4))), AppAction::MapOpen);
    assert!(!app.map_route_through_zones);
    let avoided = text(&draw(&mut app, 120, 40));
    assert!(
        avoided.contains("zones: avoided"),
        "a fresh Map open starts with zones avoided: {avoided}"
    );

    app.map_route_through_zones = true;
    assert_eq!(app.map_open(), AppAction::MapOpen);
    assert!(
        !app.map_route_through_zones,
        "MapOpen also resets stale state"
    );
}

#[test]
fn map_focus_reserves_l_for_pan_and_esc_orders_search_before_close() {
    let mut app = crate::test_support::fleet_app(&["alice"]);
    app.show_screen(Screen::Overview);
    assert_eq!(app.on_key(ch('l')), AppAction::None);
    assert!(
        app.loadouts_state.open,
        "l opens loadouts from the Overview"
    );
    app.loadouts_state.open = false;
    app.world = Some(Arc::new(nav::world::NavWorld::from_grid(
        &nav::grid::StepGrid::fixture_open_3x3(),
    )));
    assert_eq!(app.on_key(key(KeyCode::F(4))), AppAction::MapOpen);
    let before = app.map.pan;
    assert_eq!(app.on_key(ch('l')), AppAction::None);
    assert_ne!(app.map.pan, before, "l pans in the Map tab");
    assert!(!app.loadouts_state.open);
    app.map_search_open = true;
    assert_eq!(app.on_key(key(KeyCode::Esc)), AppAction::None);
    assert!(app.map_active, "first Esc closes search only");
    assert_eq!(app.on_key(key(KeyCode::Esc)), AppAction::MapClose);
    assert!(!app.map_active);
    assert_eq!(app.screen, Screen::Overview, "leaving the map goes back");
}

fn run_map_keyboard_walkthrough(width: u16, height: u16) {
    let mut app = open_map_world();
    assert_eq!(
        app.map_catalogue_status,
        MapCatalogueStatus::Inactive,
        "no catalogue demand before F4"
    );
    draw(&mut app, width, height);
    assert_eq!(
        app.map_catalogue_status,
        MapCatalogueStatus::Inactive,
        "draw must not demand a catalogue"
    );
    assert!(
        !matches!(
            app.on_key(key(KeyCode::Enter)),
            AppAction::ArmWalk(_) | AppAction::MapWalkGroup
        ),
        "Enter outside the Map tab does not confirm"
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

    assert_eq!(app.on_key(ch('/')), AppAction::None);
    assert!(app.map_search_open);
    for c in "varrock".chars() {
        app.on_key(ch(c));
    }
    assert_eq!(app.on_key(key(KeyCode::Enter)), AppAction::None);
    assert_eq!(app.map_poi_sel, Some(0), "search Enter jumps to the POI");
    assert!(app.map_model.pending().is_none());

    assert_eq!(app.on_key(ch('/')), AppAction::None);
    for c in "2,2,0".chars() {
        app.on_key(ch(c));
    }
    assert_eq!(app.on_key(key(KeyCode::Enter)), AppAction::None);
    assert_eq!(app.map.selection, Some(tile(2, 2)));
    assert!(app.map_model.pending().is_some());
    assert_eq!(
        app.on_key(ch('t')),
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
    assert_eq!(app.on_key(ch('d')), AppAction::None);
    assert_ne!(app.map.layers.dots, dots_before);
    let collision_before = app.map.layers.collision;
    assert_eq!(app.on_key(ch('c')), AppAction::None);
    assert_ne!(app.map.layers.collision, collision_before);
    let reach_before = app.map.layers.reach;
    assert_eq!(app.on_key(ch('r')), AppAction::None);
    assert_ne!(app.map.layers.reach, reach_before);
    assert_eq!(app.on_key(ch('R')), AppAction::None);
    assert_eq!(app.map.plane, 0);
    assert!(
        app.map_model.pending().is_none(),
        "recenter clears pending selection"
    );

    assert_eq!(app.on_key(ch('/')), AppAction::None);
    for c in "3,3,0".chars() {
        app.on_key(ch(c));
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
        app.on_key(ch('t')),
        AppAction::MapTeleport(Tile {
            x: 3,
            z: 3,
            level: 0
        })
    );

    assert_eq!(app.on_key(ch('/')), AppAction::None);
    for c in "2,2,0".chars() {
        app.on_key(ch(c));
    }
    assert_eq!(app.on_key(key(KeyCode::Enter)), AppAction::None);
    assert_eq!(
        app.on_key(key(KeyCode::Enter)),
        AppAction::ArmWalk(tile(2, 2))
    );
    assert!(app.map_model.pending().is_some());

    assert_eq!(app.on_key(ch('g')), AppAction::None);
    assert_eq!(app.walk_send.mode, WalkSendMode::Group);
    assert_eq!(app.walk_send.walk_label(), "Walk 1 bots");
    assert!(
        app.table
            .selection
            .contains(app.profile_id_for_name("alice"))
            && !app.table.selection.contains(app.profile_id_for_name("bob")),
        "the group is the fleet's row selection: only eligible alice"
    );
    assert_eq!(
        app.on_key(key(KeyCode::Enter)),
        AppAction::MapWalkGroup,
        "group Enter walks the checked fleet"
    );
    assert!(!app.quit);

    let all = text(&draw(&mut app, width, height));
    assert!(all.contains("plane 0"), "{width}x{height} map title: {all}");
    assert!(
        all.contains("legend:"),
        "{width}x{height} map legend: {all}"
    );
    assert!(
        all.contains("Walk 1 bots"),
        "{width}x{height} group walk label: {all}"
    );
    assert!(
        all.contains("no position yet"),
        "{width}x{height} eligibility: {all}"
    );
    assert!(
        all.contains("obs:1"),
        "{width}x{height} observed services: {all}"
    );

    assert_eq!(
        app.on_key(key(KeyCode::Esc)),
        AppAction::None,
        "Esc clears the selection first"
    );
    assert_eq!(app.on_key(key(KeyCode::Esc)), AppAction::MapClose);
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
    app.names = vec!["poi-walker".into()];
    app.refresh_walk_send(|_| WalkSlotStatus::Eligible(WalkSlotReady { origin: tile(0, 0) }));
    assert_eq!(app.on_key(ch('g')), AppAction::None);
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
        let all = text(&draw(&mut app, width, height));
        assert!(
            all.contains("status: Walk/Teleport unavailable: no observed player"),
            "{width}x{height} missing refusal: {all}"
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
    assert_eq!(app.on_key(ch('/')), AppAction::None);
    for c in "bank kebab jewellery".chars() {
        app.on_key(ch(c));
    }
    assert_eq!(
        app.map_search, "bank kebab jewellery",
        "j/k in the search editor must type, not move the result cursor"
    );
    assert_eq!(app.map_search_sel, 0);

    app.map_search.clear();
    app.map_search_sel = 0;
    for c in "varrock".chars() {
        app.on_key(ch(c));
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
        AppAction::None,
        "Ctrl-P in the map search moves the result cursor, not the palette"
    );
    assert_eq!(app.map_search_sel, 0);
    assert!(app.modal.is_none());
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
    assert_eq!(app.on_key(ch('/')), AppAction::None);
    for c in "2,2,0".chars() {
        app.on_key(ch(c));
    }
    assert_eq!(app.on_key(key(KeyCode::Enter)), AppAction::None);
    assert!(
        app.error.is_none(),
        "a new coordinate selection must clear the leftover Blocked status"
    );
}

#[test]
fn chat_tab_click_routes_to_answer() {
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
    app.show_screen(Screen::Chat);
    draw(&mut app, 100, 30);
    let chat_area = app.chat_area;
    assert!(chat_area.height >= 4, "chat pane has room for options");
    // Option rows start after border + text + blank (see chat.rs).
    assert_eq!(
        app.on_click(chat_area.x, chat_area.y + 3),
        AppAction::Chat(super::ChatAction::Answer(1)),
        "clicking the first option row answers option 1"
    );
}

#[test]
fn chat_tab_paints_the_snapshot_ring() {
    let mut app = TuiApp::new("274bot headless");
    app.chat_data.lines = vec![line("welcome to 274")];
    assert!(!app.chat_data.is_modal_open());
    app.show_screen(Screen::Chat);
    let all = text(&draw(&mut app, 100, 30));
    assert!(all.contains("welcome to 274"), "chat ring paints: {all:?}");
}

/// TR-TUI-001: when the selected bot's script is Paused the pane shows
/// `[Resume P]` and clicking it dispatches the pause/resume toggle.
#[test]
fn paused_script_shows_resume_and_click_dispatches_toggle() {
    let mut app = script_app();
    app.script_state = RunState::Paused;
    let rows = draw(&mut app, 100, 30);
    let all = text(&rows);
    assert!(
        all.contains("[Resume P]"),
        "paused script paints Resume: {all}"
    );
    assert!(!all.contains("[Pause P]"), "no Pause while paused: {all}");
    let (col, row) = find(&rows, "[Resume P]").unwrap();
    assert_eq!(
        app.on_click(col + 2, row),
        AppAction::ScriptPause,
        "Resume click dispatches the pause/resume toggle"
    );
}

/// With a Browse-selected card, clicking Start carries that card.
#[test]
fn click_start_with_a_selected_card_returns_script_start() {
    let mut app = script_app();
    app.script_sel = Some(ScriptSel::Loaded(
        ScriptSource::Catalog,
        "BoneBurier".into(),
    ));
    let rows = draw(&mut app, 100, 30);
    let (col, row) = find(&rows, "[Start t]").unwrap();
    assert_eq!(
        app.on_click(col + 1, row),
        AppAction::ScriptStart(ScriptSel::Loaded(
            ScriptSource::Catalog,
            "BoneBurier".into(),
        )),
        "Start with a selected card starts that card"
    );
}

#[test]
fn browse_rows_select_a_card_for_start() {
    let mut app = script_app();
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
    let rows = draw(&mut app, 100, 30);
    let (col, row) = find(&rows, "[Browse b]").unwrap();
    assert_eq!(
        app.on_click(col + 1, row),
        AppAction::ScriptBrowse,
        "Browse opens the picker"
    );
    assert!(app.script_browse_open);
    let rows = draw(&mut app, 100, 30);
    let (col, row) = find(&rows, "BoneBurier").unwrap();
    assert_eq!(app.on_click(col, row), AppAction::None);
    assert_eq!(
        app.script_sel,
        Some(ScriptSel::Loaded(
            ScriptSource::Catalog,
            "BoneBurier".into(),
        )),
        "clicking the first card row selects it"
    );
    assert_eq!(
        app.on_key(ch('t')),
        AppAction::None,
        "Browse keeps the keys while it is open: t does not start yet"
    );
    assert_eq!(app.on_key(key(KeyCode::Enter)), AppAction::None);
    assert!(
        !app.script_browse_open,
        "Enter closes Browse; the pick stays"
    );
    assert_eq!(
        app.on_key(ch('t')),
        AppAction::ScriptStart(ScriptSel::Loaded(
            ScriptSource::Catalog,
            "BoneBurier".into(),
        )),
        "t starts the card picked in Browse"
    );
}

/// The Load button opens the file browser; Enter on a file produces
/// `AppAction::ScriptLoad` with that path.
#[test]
fn load_browser_enter_returns_script_load() {
    let dir = std::env::temp_dir().join(format!("274bot-tui-load-browser-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let bot = dir.join("digbot.js");
    std::fs::write(&bot, "export function tick(api) { globalThis.__rs_n = 1 }").unwrap();

    let mut app = script_app();
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

/// Clicking `[Params]` opens the popup; Space toggles a bool into the bag
/// Start would post.
#[test]
fn script_params_click_and_space_toggle_persist_bool() {
    let dir = std::env::temp_dir().join(format!("274bot-tui-app-params-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let loadouts = script::LoadoutsStore::at(dir.join("loadouts.json"));
    let mut commits: Vec<(String, serde_json::Value)> = Vec::new();
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
    let mut app = script_app();
    app.script_sel = Some(ScriptSel::Loaded(
        ScriptSource::Catalog,
        "ChickenKiller".into(),
    ));
    app.params_schema = schema;
    let rows = draw(&mut app, 100, 30);
    let (col, row) = find(&rows, "[Params v]").unwrap();
    assert_eq!(
        app.on_click(col + 1, row),
        AppAction::ScriptParams,
        "[Params] opens the popup"
    );
    app.open_script_params(script::merge_bag(
        &app.params_schema,
        &serde_json::Map::new(),
        None,
    ));
    assert!(app.params_state.open);
    let mut commit = |id: &str, value: serde_json::Value| {
        commits.push((id.to_string(), value));
        Ok(())
    };
    app.params_on_key(&mut commit, &loadouts, None, ch(' '));
    assert_eq!(
        app.params_bag.get("buryBones"),
        Some(&serde_json::json!(false))
    );
    assert_eq!(
        commits,
        [("buryBones".to_string(), serde_json::json!(false))],
        "the toggle is committed to the profile"
    );
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 30)).unwrap();
    terminal
        .draw(|frame| {
            app.draw(frame);
            app.draw_params_overlay(frame, &loadouts, None);
        })
        .unwrap();
    let all = text(&crate::test_support::rows(terminal.backend().buffer()));
    assert!(all.contains("parameters"), "params overlay paints: {all}");
    assert!(
        all.contains("KEYS Parameters"),
        "the footer names the popup that has the keys: {all}"
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
    let loadouts = script::LoadoutsStore::at(dir.join("loadouts.json"));
    let mut commits: Vec<(String, serde_json::Value)> = Vec::new();
    let mut commit = |id: &str, value: serde_json::Value| {
        commits.push((id.to_string(), value));
        Ok(())
    };
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
    let mut app = script_app();
    app.script_sel = Some(ScriptSel::Loaded(ScriptSource::Catalog, "Alcher".into()));
    app.params_schema = schema;
    app.open_script_params(script::merge_bag(
        &app.params_schema,
        &serde_json::Map::new(),
        None,
    ));
    assert!(app.params_state.open);
    assert_eq!(
        app.on_key(ch('q')),
        AppAction::ParamsKey(ch('q')),
        "the params popup gets q, not the quit request"
    );
    assert!(!app.quit, "params overlay must consume q");
    assert!(app.modal.is_none());
    app.params_on_key(&mut commit, &loadouts, None, key(KeyCode::Enter));
    assert!(app.params_state.editing);
    while !app.params_state.scratch.is_empty() {
        app.params_on_key(&mut commit, &loadouts, None, key(KeyCode::Backspace));
    }
    app.params_on_key(&mut commit, &loadouts, None, ch('5'));
    app.params_on_key(&mut commit, &loadouts, None, key(KeyCode::Esc));
    assert!(!app.params_state.editing);
    assert_eq!(
        app.params_bag.get("alchs").and_then(|v| v.as_f64()),
        Some(27.0)
    );
    app.params_on_key(&mut commit, &loadouts, None, key(KeyCode::Enter));
    while !app.params_state.scratch.is_empty() {
        app.params_on_key(&mut commit, &loadouts, None, key(KeyCode::Backspace));
    }
    app.params_on_key(&mut commit, &loadouts, None, ch('5'));
    app.params_on_key(&mut commit, &loadouts, None, key(KeyCode::Enter));
    assert_eq!(
        app.params_bag.get("alchs").and_then(|v| v.as_f64()),
        Some(5.0)
    );
    app.params_on_key(&mut commit, &loadouts, None, key(KeyCode::Esc));
    assert!(!app.params_state.open);
    assert_eq!(
        commits,
        [("alchs".to_string(), serde_json::json!(5))],
        "the saved integral edit commits as an integer; the cancelled edit is not"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// While the selected bot's script paints, the Chat tab shows the paint
/// instead of the game chat; `p` there toggles back to the game chat.
#[test]
fn chat_tab_shows_script_paint_instead_of_the_game_chat() {
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
    app.show_screen(Screen::Chat);
    let all = text(&draw(&mut app, 100, 30));
    assert!(
        all.contains("BoneBurier — digging"),
        "the paint title paints: {all:?}"
    );
    assert!(
        all.contains("Runtime: 1.2m | Buried: 3"),
        "paint rows paint: {all:?}"
    );
    assert!(
        !all.contains("last game chat line"),
        "the game chat is replaced by the paint: {all:?}"
    );
    assert_eq!(app.on_key(ch('p')), AppAction::None);
    assert!(app.chat_data.show_game_chat);
    let all = text(&draw(&mut app, 100, 30));
    assert!(
        all.contains("last game chat line"),
        "p toggles back to the game chat: {all:?}"
    );
    assert!(
        !all.contains("BoneBurier — digging"),
        "the paint is hidden while toggled off: {all:?}"
    );
}

/// The background-bots notice is dismissed only by its explicit Got it
/// (key `n` on the Overview, its button, or the palette); Esc never
/// persists "don't show again" by accident.
#[test]
fn background_notice_needs_an_explicit_got_it() {
    let mut app = crate::test_support::fleet_app(&["alice"]);
    app.background_notice = Some("other profiles keep running".into());
    app.show_screen(Screen::Overview);
    assert_eq!(app.on_key(key(KeyCode::Esc)), AppAction::None);
    let rows = draw(&mut app, 120, 40);
    let (col, row) = find(&rows, "[Got it n]").expect("the notice has a visible Got it");
    assert_eq!(app.on_click(col + 1, row), AppAction::AckBackground);
    assert_eq!(app.on_key(ch('n')), AppAction::AckBackground);
    app.background_notice = None;
    assert_eq!(
        app.on_key(ch('n')),
        AppAction::None,
        "no notice, nothing to dismiss"
    );
}

/// At 80×24 the Script tab keeps both command rows on screen and
/// clickable; every command also has its key, and the fleet-wide ones ask
/// to confirm their scope first.
#[test]
fn script_commands_are_reachable_at_80x24() {
    let mut app = script_app();
    app.chat_data.lines = (0..8).map(|i| line(&format!("chat {i}"))).collect();
    app.script_state = RunState::Running;
    app.script_sel = Some(ScriptSel::Loaded(ScriptSource::File, "thiever".into()));
    app.params_schema = thiever_schema();
    let rows = draw(&mut app, 80, 24);
    let (main_x, main_y) =
        find(&rows, "[Browse b] [Start t] [Pause P] [Stop e] [Load f]").expect("main script row");
    let (bulk_x, bulk_y) =
        find(&rows, "[Params v] [Reload R] [Start all T] [Stop all E]").expect("bulk script row");
    let at = |x: u16, row: &str, label: &str| x + row.find(label).unwrap() as u16 + 1;
    let main = "[Browse b] [Start t] [Pause P] [Stop e] [Load f]";
    let bulk = "[Params v] [Reload R] [Start all T] [Stop all E]";
    let sel = app.script_sel.clone().unwrap();
    let clicks = [
        (
            main_y,
            at(main_x, main, "[Start"),
            AppAction::ScriptStart(sel.clone()),
        ),
        (main_y, at(main_x, main, "[Pause"), AppAction::ScriptPause),
        (main_y, at(main_x, main, "[Stop"), AppAction::ScriptStop),
        (bulk_y, at(bulk_x, bulk, "[Params"), AppAction::ScriptParams),
        (bulk_y, at(bulk_x, bulk, "[Reload"), AppAction::ScriptReload),
    ];
    for (y, x, want) in clicks {
        assert_eq!(app.on_click(x, y), want, "click at {x},{y}");
    }
    for (label, want) in [
        ("[Start all", AppAction::ScriptStartAll),
        ("[Stop all", AppAction::ScriptStopAll),
    ] {
        assert_eq!(
            app.on_click(at(bulk_x, bulk, label), bulk_y),
            AppAction::None,
            "{label} asks first"
        );
        assert!(matches!(app.modal, Some(Modal::Confirm(_))), "{label}");
        assert_eq!(app.on_key(key(KeyCode::Enter)), want, "{label} confirmed");
    }
    let keys = [
        ('t', AppAction::ScriptStart(sel)),
        ('P', AppAction::ScriptPause),
        ('e', AppAction::ScriptStop),
        ('v', AppAction::ScriptParams),
        ('R', AppAction::ScriptReload),
    ];
    for (c, want) in keys {
        assert_eq!(app.on_key(ch(c)), want, "key {c}");
    }
    for (c, want) in [
        ('T', AppAction::ScriptStartAll),
        ('E', AppAction::ScriptStopAll),
    ] {
        assert_eq!(app.on_key(ch(c)), AppAction::None, "{c} asks first");
        assert_eq!(app.on_key(ch('y')), want, "{c} confirmed");
    }
    // `C` cancels only a shown reload warning.
    assert_eq!(app.on_key(ch('C')), AppAction::None);
    app.reload_confirm = true;
    assert_eq!(app.on_key(ch('C')), AppAction::ScriptReloadCancel);
    // Browse and Load open in the tab and keep the keys until closed.
    assert_eq!(app.on_key(ch('b')), AppAction::ScriptBrowse);
    assert!(app.script_browse_open);
    assert_eq!(app.on_key(key(KeyCode::Esc)), AppAction::None);
    assert!(!app.script_browse_open);
    assert_eq!(app.on_key(ch('f')), AppAction::None);
    assert!(app.script_load_open);
}

#[test]
fn map_search_list_uses_display_name_not_slash_breaks() {
    let (catalogue, _, label_anchor) = map_poi_catalogue();
    let mut app = TuiApp::new("274bot headless");
    bind_catalogue_map(&mut app, std::sync::Arc::clone(&catalogue));
    let index = catalogue
        .entries()
        .find(|e| e.name() == "Lumbridge/Swamp")
        .unwrap()
        .index();
    let row = app.search_hit_label(index).expect("label row");
    assert!(
        row.contains("Lumbridge Swamp"),
        "list must show a space, not the stored line-break: {row}"
    );
    assert!(
        !row.contains("Lumbridge/Swamp"),
        "slash line-break must not leak into the list: {row}"
    );
    assert!(row.contains(&format!(
        "({},{},{})",
        label_anchor.x, label_anchor.z, label_anchor.level
    )));

    search_and_jump(&mut app, "lumbridge");
    let drawn = text(&draw(&mut app, 120, 40));
    assert!(
        drawn.contains("Lumbridge Swamp"),
        "drawn search rows must use the display name: {drawn}"
    );
    assert!(
        !drawn.contains("Lumbridge/Swamp"),
        "drawn search rows must not leak the slash: {drawn}"
    );
}
