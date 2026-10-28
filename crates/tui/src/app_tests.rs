use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use api::snapshot::{ChatLineView, ChatOptionView, WorldTile};
use frontend_core::scripts::{
    CardRuntime, CardSource, CompiledCardId, Loadouts, OwnedSchema, ScriptChoice, ScriptRunState,
};
use frontend_core::test_support::paint::TestPaint;
use frontend_core::test_support::scripts::{NativeStatus, TestSchema};
use frontend_core::SettingsDraft;
use host_play::local_state::{LocalStateService, StateRoot};
use nav_planner::tile::Tile;

use crate::bin::seam::Seam;
use crate::layout::Screen;
use crate::overlay::Modal;
use crate::script_shape::BrowseCard;
use crate::test_support::{ch, draw, find, key, text};

use frontend_core::walk_map::{
    WalkActionErrorKind, WalkActionKind, WalkCatalogue, WalkExclude, WalkMapContext, WalkServices,
    WalkSlotStatus,
};
use nav_planner::map::formats::{
    ClientPois, Coverage, CoverageLevel, ServiceIdentity, ServicePois, NAVPOIS_VERSION,
};
use nav_planner::map::identity::{CatalogueIdentity, Digest};
use nav_planner::map::poi::{
    CapabilityEvidence, DisplayAnchor, EntityKind, Footprint, PoiKey, PoiKind, PoiRecord,
    SourceSpace,
};
use nav_planner::map::{Rows, Text};

use super::{wasd_target, AppAction, MapCatalogueStatus, TuiApp, WalkSendMode};
use frontend_core::{FleetRow, Phase, SlotDetail};

/// A projected member row (what the core publishes).
fn member(name: &str, phase: Phase) -> FleetRow {
    FleetRow::fixture(name, phase, None, None)
}

/// The selected slot's projected detail while it is in game on `tile`.
fn ready_detail(name: &str, tile: (i32, i32, i32)) -> SlotDetail {
    let mut detail = SlotDetail::default();
    detail.set_row(member(name, Phase::Ready));
    detail.set_state("ingame scene 2");
    detail.set_tile(tile);
    detail.set_ready_tile(Some(tile));
    detail
}

fn bone_burier_card() -> BrowseCard {
    BrowseCard {
        name: "BoneBurier".into(),
        description: String::new(),
        category: "Prayer".into(),
        tags: Vec::new(),
        kind: CardRuntime::Compat,
        source: CardSource::Catalog,
        #[cfg(feature = "js")]
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

fn map_poi_catalogue() -> (std::sync::Arc<WalkCatalogue>, Tile, Tile) {
    let world = std::sync::Arc::new(nav_planner::world::NavWorld::from_grid(
        &nav_planner::grid::StepGrid::fixture_open_3x3(),
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
    let services = WalkServices::decode(&bytes, doc.identity, Digest::of(&bytes)).unwrap();
    let catalogue = WalkCatalogue::new(world, identity, nav, Some(client), Some(services)).unwrap();
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

fn bind_catalogue_map(app: &mut TuiApp, catalogue: std::sync::Arc<WalkCatalogue>) {
    app.world = Some(std::sync::Arc::clone(catalogue.world()));
    app.names = vec!["alice".into()];
    app.focused = Some(0);
    app.here = Some(WorldTile {
        x: 0,
        z: 0,
        level: 0,
    });
    app.map_model.bind(WalkMapContext::browsing(
        catalogue.nav_identity(),
        Some(catalogue.key()),
        1,
    ));
    assert_eq!(app.on_key(key(KeyCode::F(4))), AppAction::MapOpen);
    app.bind_walk_catalogue(catalogue);
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
    app.fleet_model.set_universe_for_test(
        vec![member("alice", Phase::Ready), member("bob", Phase::Offline)],
        &[],
    );
    app.detail = Some(ready_detail("alice", (1, 1, 0)));
    app.world = Some(Arc::new(nav_planner::world::NavWorld::from_grid(
        &nav_planner::grid::StepGrid::fixture_open_3x3(),
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

fn nature_crafter_paint() -> TestPaint {
    TestPaint::new(
        Some("NatureCrafter — Air — runner — restocking at the bank"),
        &[
            "Runtime: 1m | Mode: Runner | To: paintproof",
            "Deliveries: 0 | Ess sent: 0 | Coins: 0",
            "Pack ess: 0 | noted: 0 | unnoted: 0",
        ],
    )
    .with_button("gobank", "Go bank")
}

/// A one-bot app on the Script tab (keyboard on the tab).
fn script_app() -> TuiApp {
    let mut app = crate::test_support::fleet_app(&["alice", "bob"]);
    assert_eq!(app.show_screen(Screen::Script), AppAction::None);
    app
}

fn params_overlay_text(app: &mut TuiApp, loadouts: &Loadouts) -> String {
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 40)).unwrap();
    terminal
        .draw(|frame| app.draw_params_overlay(frame, loadouts, None))
        .unwrap();
    text(&crate::test_support::rows(terminal.backend().buffer()))
}

fn owned_schema(schema: &TestSchema) -> OwnedSchema {
    let mut owned = OwnedSchema::default();
    owned.copy_from(schema.schema_ref());
    owned
}

fn thiever_schema() -> TestSchema {
    TestSchema::setting("target", "string", Some("Man"), None, &[])
}

/// Boot shows the fleet and never requests catalogue work.
#[test]
fn boot_draws_the_fleet_without_map_demand() {
    let mut app = TuiApp::new("274bot headless");
    let rows = draw(&mut app, 80, 24);
    let all = text(&rows);
    assert!(rows[0].contains("274bot"), "title: {all}");
    assert!(
        all.contains("no profiles: press N to add one"),
        "an empty fleet explains how to add a profile: {all}"
    );
    assert_eq!(
        app.map_catalogue_status,
        MapCatalogueStatus::Inactive,
        "draw must not demand a catalogue"
    );
}

/// A parked Quester (an in-quest hold) shows its script phase and park reason on
/// the status card at both common sizes; a slot with no script shows neither.
#[test]
fn status_card_shows_a_parked_quester_phase_and_reason_at_common_sizes() {
    let mut app = crate::test_support::fleet_app(&["alice"]);
    app.show_screen(Screen::Overview);
    let mut detail = crate::test_support::ready_detail("alice");
    detail.row_mut().set_script(ScriptRunState::Running);
    let status = NativeStatus::working(CompiledCardId::QUESTER)
        .with_text_field("action_state", "Action", "blocked")
        .with_text_field("block_reason", "Blocked", "no step for stage");
    app.detail = Some(status.attach_to(detail));
    for (width, height) in [(120, 40), (80, 24)] {
        let all = text(&draw(&mut app, width, height));
        assert!(all.contains("script: blocked"), "{width}x{height}: {all}");
        assert!(all.contains("phase: blocked"), "{width}x{height}: {all}");
        assert!(
            all.contains("reason: no step for stage"),
            "{width}x{height}: {all}"
        );
    }
    app.detail = Some(crate::test_support::ready_detail("alice"));
    let all = text(&draw(&mut app, 120, 40));
    assert!(!all.contains("script:"), "no script, no script row: {all}");
    assert!(!all.contains("reason:"), "no script, no reason row: {all}");
}

/// Map activation is explicit; loaded collision/POIs then render in cells.
#[test]
fn draw_map_paints_walkable_dots_after_explicit_activation() {
    let mut app = TuiApp::new("274bot headless");
    app.world = Some(Arc::new(nav_planner::world::NavWorld::from_grid(
        &nav_planner::grid::StepGrid::fixture_open_3x3(),
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
    let mut seam = Seam::new();
    let mut app = TuiApp::new("274bot headless");
    // A store that refuses writes: the failure surfaces.
    seam.refuse_preference_writes();
    let action = app.toggle_map_wilderness();
    assert_eq!(action, AppAction::MapWilderness(true));
    seam.apply(&mut app, action);

    assert!(app.map.layers.wilderness);
    assert!(
        app.error
            .as_deref()
            .is_some_and(|error| error.starts_with("map wilderness: ")),
        "preference write failure must be surfaced: {:?}",
        app.error
    );

    seam.allow_preference_writes();
    let action = app.toggle_map_wilderness();
    assert_eq!(action, AppAction::MapWilderness(false));
    seam.apply(&mut app, action);
    assert!(!app.map.layers.wilderness);
    assert!(
        app.error.is_none(),
        "a later successful save clears the stale map error: {:?}",
        app.error
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
    app.chat_data.script_paint = Some(
        TestPaint::new(Some("NatureCrafter"), &["status"])
            .with_button("gobank", "Go bank")
            .frame(),
    );
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
    app.chat_data.script_paint = Some(nature_crafter_paint().frame());
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
    app.chat_data.script_paint = Some(nature_crafter_paint().frame());
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
fn quest_paths_settings_are_reachable_editable_and_reload_routes_at_common_sizes() {
    let _iso = frontend_core::test_support::IsolatedStores::enter("tui-quest-paths-settings");
    let _quester_paths_guard = crate::bin::seam::lock_quester_paths_test_state();

    for (width, height) in [(80, 24), (120, 40)] {
        let mut seam = Seam::new();

        let mut app = crate::test_support::fleet_app(&["alice"]);
        app.show_screen(Screen::Overview);
        assert_eq!(app.on_key(ch('o')), AppAction::None);
        assert!(
            app.settings_state.open,
            "Settings opens at {width}x{height}"
        );

        for _ in 0..12 {
            app.on_key(key(KeyCode::Down));
        }
        let rows = draw(&mut app, width, height);
        assert!(
            text(&rows).contains("Quester: Quest Paths (all bots)"),
            "the Path-folder lead line stays visible at {width}x{height}: {rows:?}"
        );
        assert!(
            text(&rows).contains(frontend_core::quester_paths::LOAD_PATHS_LABEL),
            "the off-by-default folder gate stays visible at {width}x{height}: {rows:?}"
        );
        assert!(!app.quester_paths.enabled(), "folder Paths start disabled");
        let action = app.on_key(key(KeyCode::Enter));
        assert_eq!(action, AppAction::DismissPathsNotice);
        seam.apply(&mut app, action);
        seam.pump(&mut app);

        assert!(app.quester_paths.enabled(), "the gate toggles on");

        app.on_key(key(KeyCode::Down));
        let rows = draw(&mut app, width, height);
        assert!(
            text(&rows).contains("Paths folder:"),
            "the folder field stays visible at {width}x{height}: {rows:?}"
        );
        assert_eq!(app.on_key(key(KeyCode::Enter)), AppAction::None);
        let previous = app.quester_paths.folder().to_string_lossy().chars().count();
        for _ in 0..previous {
            app.on_key(key(KeyCode::Backspace));
        }
        let selected_folder = "/tmp/quester-checkout/paths";
        for character in selected_folder.chars() {
            app.on_key(ch(character));
        }
        let action = app.on_key(key(KeyCode::Enter));
        assert_eq!(
            action,
            AppAction::DismissPathsNotice,
            "Enter commits the editable folder field"
        );
        seam.apply(&mut app, action);
        seam.pump(&mut app);

        assert_eq!(
            app.quester_paths.folder(),
            std::path::PathBuf::from(selected_folder)
        );

        app.on_key(key(KeyCode::Down));
        let rows = draw(&mut app, width, height);
        assert!(
            text(&rows).contains(frontend_core::quester_paths::RELOAD_PATHS_LABEL),
            "Reload Paths stays visible at {width}x{height}: {rows:?}"
        );
        let action = app.on_key(key(KeyCode::Enter));
        assert_eq!(
            action,
            AppAction::ReloadPaths,
            "the settings command routes reload without starting a Path"
        );
        seam.apply(&mut app, action);
    }
    frontend_core::test_support::scripts::reset_quester_registry();
}

#[test]
fn routing_scope_note_heads_the_routing_rows_once_at_common_sizes() {
    for (width, height) in [(80, 24), (120, 40)] {
        let mut app = crate::test_support::fleet_app(&["alice"]);
        app.show_screen(Screen::Overview);
        assert_eq!(app.on_key(ch('o')), AppAction::None);
        for _ in 0..7 {
            app.on_key(key(KeyCode::Down));
        }
        let rows = draw(&mut app, width, height);
        let shown = text(&rows);
        assert_eq!(
            shown
                .matches("Settings are global except where otherwise specified.")
                .count(),
            1,
            "the routing scope is one shared note at {width}x{height}: {rows:?}"
        );
        let note = shown.find("Settings are global").expect("routing note");
        let teleports = shown.find("allow teleports:").expect("teleports row");
        assert!(
            note < teleports,
            "the note heads the routing rows at {width}x{height}: {rows:?}"
        );
        assert!(
            shown.contains("Manual WalkTo only."),
            "bank fetch keeps its own scope at {width}x{height}: {rows:?}"
        );
    }
}

#[test]
fn settings_enter_returns_random_events_edit_and_marks_dirty() {
    let mut app = TuiApp::new("274bot headless");
    app.settings = SettingsDraft::default();
    assert!(app.settings.random_events());
    app.settings_state.open = true;
    let action = app.on_key(key(KeyCode::Enter));
    assert_eq!(
        action,
        AppAction::SettingsDraftEdited(crate::settings::SettingsEdit::RandomEvents(false))
    );
    assert!(
        app.settings.random_events(),
        "the dispatcher applies the returned edit"
    );
    assert!(app.settings_dirty, "the binary persists the change");
}

#[test]
fn manual_movement_pause_toggle_is_reachable_and_last_settings_row_stays_clamped() {
    let mut app = TuiApp::new("274bot headless");
    assert!(app.pause_script_on_manual_walk_abort);
    app.settings_state.open = true;
    for _ in 0..9 {
        assert_eq!(app.on_key(key(KeyCode::Down)), AppAction::None);
    }
    assert_eq!(app.settings_state.row, 9);
    assert_eq!(app.on_key(key(KeyCode::Enter)), AppAction::None);
    assert!(!app.pause_script_on_manual_walk_abort);
    assert!(app.pause_script_on_manual_walk_abort_dirty);

    for _ in 0..20 {
        app.on_key(key(KeyCode::Down));
    }
    assert_eq!(
        app.settings_state.row, 15,
        "navigation clamps at the final settings row (cure poison), past Reload Paths at 14"
    );
}

#[test]
fn wrapped_global_settings_rows_use_the_drawn_hit_target() {
    let mut app = TuiApp::new("274bot headless");
    app.settings_state.open = true;
    app.nav = app
        .nav
        .with_danger_level(frontend_core::DangerLevel::Always);
    app.script_scope_notice_ack = true;
    app.survivable_routing_notice_ack = true;
    app.settings_state.row = 8;
    let rows = draw(&mut app, 60, 14);
    let (col, row) = find(&rows, "Danger routing:")
        .expect("the selected global remains visible in a compact terminal");
    app.on_click(col, row + 1);
    assert_eq!(
        app.nav.danger_level(),
        frontend_core::DangerLevel::Never,
        "the wrapped continuation cycles the danger setting"
    );
    assert!(
        app.pause_script_on_manual_walk_abort,
        "a continuation never activates the next setting"
    );
}

#[test]
fn settings_survivable_notice_ack_is_independent_and_reaches_the_shared_projection() {
    let mut app = TuiApp::new("274bot headless");
    app.settings_state.open = true;
    app.script_scope_notice_ack = true;
    assert!(find(
        &draw(&mut app, 100, 24),
        "Danger routing now has three levels"
    )
    .is_some());
    assert_eq!(app.on_key(key(KeyCode::Char('n'))), AppAction::None);
    assert!(app.script_scope_notice_ack && app.survivable_routing_notice_ack);
    assert!(app.walk_permissions.survivable_routing_notice_ack());
    assert!(find(
        &draw(&mut app, 100, 24),
        "Danger routing now has three levels"
    )
    .is_none());
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
    app.world = Some(Arc::new(nav_planner::world::NavWorld::from_grid(
        &nav_planner::grid::StepGrid::fixture_open_3x3(),
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
    app.nav = frontend_core::WalkGlobals::new(true, false, false, false, true);

    assert_eq!(app.on_key(key(KeyCode::F(4))), AppAction::MapOpen);
    assert_eq!(
        app.map_find_options().zones,
        nav_planner::zones::ZoneExempt::NONE
    );
    assert!(app.map_find_options().allow_teleports);

    assert_eq!(app.on_key(ch('z')), AppAction::None);
    assert!(app.map_route_through_zones);
    assert!(app.map_find_options().zones.is_all());
    let crossing = text(&draw(&mut app, 120, 40));
    assert!(
        crossing.contains("crossing (z)"),
        "Map info must show the live zone choice: {crossing}"
    );

    assert_eq!(app.on_key(key(KeyCode::Esc)), AppAction::MapClose);
    assert!(!app.map_route_through_zones);
    assert_eq!(
        app.map_find_options().zones,
        nav_planner::zones::ZoneExempt::NONE
    );

    assert_eq!(app.on_key(key(KeyCode::F(4))), AppAction::MapOpen);
    assert!(!app.map_route_through_zones);
    let avoided = text(&draw(&mut app, 120, 40));
    assert!(
        avoided.contains("avoided"),
        "a fresh Map open starts with zones avoided: {avoided}"
    );

    app.map_route_through_zones = true;
    assert_eq!(app.map_open(), AppAction::MapOpen);
    assert!(
        !app.map_route_through_zones,
        "MapOpen also resets stale state"
    );
    app.nav = app
        .nav
        .with_danger_level(frontend_core::DangerLevel::WhenSurvivable);
    app.map_route_through_zones = true;
    app.settings_state.open = true;
    app.settings_state.row = 8;
    assert_eq!(app.on_key(key(KeyCode::Enter)), AppAction::None);
    app.settings_state.open = false;
    assert!(app.nav.allow_danger_zones());
    assert!(
        !app.map_route_through_zones,
        "enabling global danger clears any hidden one-shot grant"
    );
    assert_eq!(app.on_key(ch('z')), AppAction::None);
    assert!(
        !app.map_route_through_zones,
        "global danger makes the one-shot z control unavailable"
    );
    assert!(app.map_find_options().zones.is_all());
    let global = text(&draw(&mut app, 120, 40));
    let global_lower = global.to_lowercase();
    assert!(
        global_lower.contains("danger") && global_lower.contains("global"),
        "global danger replaces the one-shot label with a warning: {global}"
    );
}

#[test]
fn durable_walk_permissions_refresh_map_and_inherited_script_parameter() {
    let root = std::env::temp_dir().join(format!(
        "274bot-tui-walk-permissions-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let preferences = frontend_core::UiPreferences::of_service_for_test(
        &LocalStateService::open_process(StateRoot::new(root.join(".274bot"))),
    );
    let loadouts = Loadouts::default();
    let mut app = open_map_world();
    Seam::restore_preferences(&mut app, preferences.share());

    app.script_sel = Some(ScriptChoice::loaded(CardSource::Catalog, "WalkPermissions"));
    let schema = TestSchema::setting(
        "allow_danger_zones",
        "boolean",
        Some("false"),
        Some("Danger routing"),
        &[],
    );
    app.params_schema = owned_schema(&schema);
    app.open_script_params(schema.merged_bag(&serde_json::Map::new(), None));
    app.params_state.open = false;
    assert_eq!(app.on_key(key(KeyCode::F(4))), AppAction::MapOpen);

    let initial = text(&draw(&mut app, 120, 40));
    assert!(initial.contains("avoided"), "{initial}");
    assert_eq!(
        app.map_find_options().zones,
        nav_planner::zones::ZoneExempt::NONE
    );
    assert_eq!(app.on_key(ch('z')), AppAction::None);
    assert!(app.map_route_through_zones);
    assert!(app.map_find_options().zones.is_all());
    let crossing = text(&draw(&mut app, 120, 40));
    assert!(crossing.contains("crossing (z)"), "{crossing}");

    frontend_core::nav_prefs::write_nav_preference_for_test(
        &preferences,
        frontend_core::NavPreference::AllowDangerZones,
        true,
    )
    .unwrap();
    let globally_on = text(&draw(&mut app, 120, 40));
    let globally_on_lower = globally_on.to_lowercase();
    assert!(
        globally_on_lower.contains("danger")
            && globally_on_lower.contains("global")
            && !globally_on.contains("crossing (z)"),
        "{globally_on}"
    );
    assert!(
        !app.map_route_through_zones,
        "global-on clears the one-shot"
    );
    assert!(app.map_find_options().zones.is_all());
    app.params_state.open = true;
    let inherited = params_overlay_text(&mut app, &loadouts);
    assert!(
        inherited.contains("On (inherited globally; script cannot veto)"),
        "{inherited}"
    );
    app.params_state.open = false;

    frontend_core::nav_prefs::write_nav_preference_for_test(
        &preferences,
        frontend_core::NavPreference::AllowDangerZones,
        false,
    )
    .unwrap();
    let globally_off = text(&draw(&mut app, 120, 40));
    assert!(
        globally_off.contains("avoided") && !globally_off.contains("crossing (z)"),
        "{globally_off}"
    );
    assert_eq!(
        app.map_find_options().zones,
        nav_planner::zones::ZoneExempt::NONE
    );
    app.params_state.open = true;
    let local = params_overlay_text(&mut app, &loadouts);
    assert!(
        local.contains("Danger routing: Off")
            && !local.contains("On (inherited globally; script cannot veto)"),
        "{local}"
    );
    app.params_state.open = false;
    assert_eq!(app.on_key(ch('z')), AppAction::None);
    assert!(app.map_route_through_zones);
    assert!(app.map_find_options().zones.is_all());
    assert_eq!(app.on_key(ch('z')), AppAction::None);
    assert!(!app.map_route_through_zones);
    assert_eq!(
        app.map_find_options().zones,
        nav_planner::zones::ZoneExempt::NONE
    );

    std::fs::write(preferences.path(), "{corrupt").unwrap();
    // A directly written file is seen after an explicit re-read; a corrupt
    // one is refused from here on.
    assert!(preferences.reload().is_err());
    let corrupt = text(&draw(&mut app, 120, 40));
    assert!(
        corrupt.contains("avoided") && !corrupt.contains("crossing (z)"),
        "{corrupt}"
    );
    assert!(!app.map_route_through_zones);
    assert_eq!(
        app.map_find_options().zones,
        nav_planner::zones::ZoneExempt::NONE
    );
    app.params_state.open = true;
    let fail_closed = params_overlay_text(&mut app, &loadouts);
    assert!(
        !fail_closed.contains("On (inherited globally; script cannot veto)"),
        "{fail_closed}"
    );
    std::fs::remove_dir_all(root).unwrap();
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
    app.world = Some(Arc::new(nav_planner::world::NavWorld::from_grid(
        &nav_planner::grid::StepGrid::fixture_open_3x3(),
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
    let mut seam = Seam::new();
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
            WalkSlotStatus::eligible(tile(1, 1))
        } else {
            WalkSlotStatus::excluded(WalkExclude::NoPosition)
        }
    });
    app.map_observed.clear();

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

    let alice = app.profile_id_for_name("alice");
    seam.press(&mut app, ch('g'), AppAction::MarkAll(vec![alice]));
    assert_eq!(app.walk_send.mode, WalkSendMode::Group);
    assert_eq!(app.sync_walk_label(), "Walk 1 bots");
    assert!(
        app.fleet_model
            .marks()
            .contains(app.profile_id_for_name("alice"))
            && !app
                .fleet_model
                .marks()
                .contains(app.profile_id_for_name("bob")),
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
        all.contains("obs:0"),
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
    assert!(app.map_catalogue.is_none());
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
fn map_poi_selection_keeps_catalogue_stand_and_blocks_view_only_labels() {
    let (catalogue, stand, label_anchor) = map_poi_catalogue();
    let ctx = WalkMapContext::browsing(catalogue.nav_identity(), Some(catalogue.key()), 1);
    let mut app = TuiApp::new("274bot headless");
    bind_catalogue_map(&mut app, Arc::clone(&catalogue));

    search_and_jump(&mut app, "lumbridge");
    let pending = app.map_model.pending().expect("label selection");
    assert_eq!(pending.requested(), label_anchor);
    assert_eq!(
        pending.walk_target(),
        None,
        "view-only label is not a walk stand"
    );
    assert_eq!(
        app.on_key(key(KeyCode::Enter)),
        AppAction::ArmWalk(label_anchor)
    );
    assert_eq!(
        app.map_model
            .availability(WalkActionKind::Walk, ctx, Some(tile(0, 0)))
            .expect_err("view-only label")
            .kind(),
        WalkActionErrorKind::Blocked
    );
    assert!(app.map_model.pending().is_some());
    assert_eq!(
        app.map.selection,
        Some(label_anchor),
        "view-only selection retains the map crosshair"
    );
    assert!(app.walk_dest.is_none());

    app.map_model.clear_selection();
    app.map.selection = None;
    search_and_jump(&mut app, "booth");
    let pending = app.map_model.pending().expect("booth selection");
    assert_eq!(pending.walk_target(), Some(stand));
    assert_ne!(pending.requested(), stand);
    let requested = pending.requested();
    assert_eq!(
        app.on_key(key(KeyCode::Enter)),
        AppAction::ArmWalk(requested)
    );
    assert_eq!(
        app.map_model
            .pending()
            .map(|selection| selection.walk_target()),
        Some(Some(stand)),
        "map selection keeps the catalogue stand, not a radius snap"
    );
    assert_eq!(
        app.map_model
            .availability(WalkActionKind::Walk, ctx, Some(tile(0, 0)))
            .expect_err("browsing context has no focused slot")
            .kind(),
        WalkActionErrorKind::NoFocus
    );
    assert_eq!(app.map.selection, Some(requested));

    app.map_model.clear_selection();
    app.map.selection = None;
    search_and_jump(&mut app, "booth");
    app.names = vec!["poi-walker".into()];
    app.fleet_model
        .set_universe_for_test(vec![member("poi-walker", Phase::Ready)], &[]);
    app.refresh_walk_send(|_| WalkSlotStatus::eligible(tile(0, 0)));
    let walker = app.profile_id_for_name("poi-walker");
    Seam::new().press(&mut app, ch('g'), AppAction::MarkAll(vec![walker]));
    assert_eq!(app.on_key(key(KeyCode::Enter)), AppAction::MapWalkGroup);
    assert_eq!(
        app.map_model.pending().expect("group").walk_target(),
        Some(stand),
        "group confirm must not replace the POI stand with a snap"
    );
}

#[test]
fn map_status_shows_walk_refusal_at_120_and_80() {
    let mut app = open_map_world();
    assert_eq!(app.on_key(key(KeyCode::F(4))), AppAction::MapOpen);
    app.error = Some("Walk/Teleport unavailable: no observed player".into());
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
    app.error = Some("No walkable tile within radius 16 (or no proven POI stand)".into());
    assert_eq!(app.on_key(key(KeyCode::Enter)), AppAction::None);
    assert!(
        app.error.is_none(),
        "a new view-centre selection must clear the leftover Blocked status"
    );
    app.error = Some("No walkable tile within radius 16 (or no proven POI stand)".into());
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
    app.script_state = ScriptRunState::Paused;
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
    app.script_sel = Some(ScriptChoice::loaded(CardSource::Catalog, "BoneBurier"));
    let rows = draw(&mut app, 100, 30);
    let (col, row) = find(&rows, "[Start t]").unwrap();
    assert_eq!(
        app.on_click(col + 1, row),
        AppAction::ScriptStart(ScriptChoice::loaded(CardSource::Catalog, "BoneBurier")),
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
            kind: CardRuntime::Compat,
            source: CardSource::File,
            #[cfg(feature = "js")]
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
        Some(ScriptChoice::loaded(CardSource::Catalog, "BoneBurier")),
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
        AppAction::ScriptStart(ScriptChoice::loaded(CardSource::Catalog, "BoneBurier")),
        "t starts the card picked in Browse"
    );
}

/// The Load button opens the file browser; Enter on a file produces
/// `AppAction::ScriptLoad` with that path.
#[cfg(feature = "js")]
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
    let loadouts = Loadouts::default();
    let mut commits: Vec<(String, serde_json::Value)> = Vec::new();
    let schema = TestSchema::setting(
        "buryBones",
        "boolean",
        Some("true"),
        Some("Bury bones"),
        &[],
    );
    let mut app = script_app();
    app.script_sel = Some(ScriptChoice::loaded(CardSource::Catalog, "ChickenKiller"));
    app.params_schema = owned_schema(&schema);
    let rows = draw(&mut app, 100, 30);
    let (col, row) = find(&rows, "[Params v]").unwrap();
    assert_eq!(
        app.on_click(col + 1, row),
        AppAction::ScriptParams,
        "[Params] opens the popup"
    );
    app.open_script_params(schema.merged_bag(&serde_json::Map::new(), None));
    assert!(app.params_state.open);
    let mut commit = |id: &str, value: serde_json::Value, _current: Option<serde_json::Value>| {
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
    let loadouts = Loadouts::default();
    let mut commits: Vec<(String, serde_json::Value)> = Vec::new();
    let mut commit = |id: &str, value: serde_json::Value, _current: Option<serde_json::Value>| {
        commits.push((id.to_string(), value));
        Ok(())
    };
    let schema = TestSchema::setting("alchs", "number", Some("27"), Some("Alchs per trip"), &[]);
    let mut app = script_app();
    app.script_sel = Some(ScriptChoice::loaded(CardSource::Catalog, "Alcher"));
    app.params_schema = owned_schema(&schema);
    app.open_script_params(schema.merged_bag(&serde_json::Map::new(), None));
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
    app.chat_data.script_paint = Some(
        TestPaint::new(
            Some("BoneBurier — digging"),
            &["Runtime: 1.2m | Buried: 3", ""],
        )
        .with_accent("#f3e6a2")
        .frame(),
    );
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
    assert!(
        matches!(&app.on_key(ch('n')), AppAction::Refused(refusal)
            if refusal.command == Some(frontend_core::OperatorCommand::AcknowledgeNotice)),
        "no notice, nothing to dismiss: the command is refused"
    );
}

/// At 80×24 the Script tab keeps both command rows on screen and
/// clickable; every command also has its key, and the fleet-wide ones ask
/// to confirm their scope first.
#[cfg(feature = "js")]
#[test]
fn script_commands_are_reachable_at_80x24() {
    let mut app = script_app();
    app.chat_data.lines = (0..8).map(|i| line(&format!("chat {i}"))).collect();
    app.script_state = ScriptRunState::Running;
    app.script_sel = Some(ScriptChoice::loaded(CardSource::File, "thiever"));
    app.params_schema = owned_schema(&thiever_schema());
    let rows = draw(&mut app, 80, 24);
    let (main_x, main_y) =
        find(&rows, "[Browse b] [Start t] [Pause P] [Stop e] [Load f]").expect("main script row");
    let (bulk_x, bulk_y) = find(
        &rows,
        "[Params v] [Reload R] [Start fleet T] [Stop fleet E]",
    )
    .expect("bulk script row");
    let at = |x: u16, row: &str, label: &str| x + row.find(label).unwrap() as u16 + 1;
    let main = "[Browse b] [Start t] [Pause P] [Stop e] [Load f]";
    let bulk = "[Params v] [Reload R] [Start fleet T] [Stop fleet E]";
    let sel = app.script_sel.as_ref().unwrap().duplicate();
    let clicks = [
        (
            main_y,
            at(main_x, main, "[Start"),
            AppAction::ScriptStart(sel.duplicate()),
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
        (
            "[Start fleet",
            AppAction::PlanFleet(frontend_core::FleetAction::Start),
        ),
        (
            "[Stop fleet",
            AppAction::PlanFleet(frontend_core::FleetAction::Stop),
        ),
    ] {
        assert_eq!(
            app.on_click(at(bulk_x, bulk, label), bulk_y),
            want,
            "{label} asks core for a plan"
        );
        assert!(app.modal.is_none(), "{label}: only core decides to ask");
    }
    let keys = [
        ('t', AppAction::ScriptStart(sel.duplicate())),
        ('P', AppAction::ScriptPause),
        ('e', AppAction::ScriptStop),
        ('v', AppAction::ScriptParams),
        ('R', AppAction::ScriptReload),
    ];
    for (c, want) in keys {
        assert_eq!(app.on_key(ch(c)), want, "key {c}");
    }
    for (c, want) in [
        ('T', AppAction::PlanFleet(frontend_core::FleetAction::Start)),
        ('E', AppAction::PlanFleet(frontend_core::FleetAction::Stop)),
    ] {
        assert_eq!(app.on_key(ch(c)), want, "{c} asks core for a plan");
        assert!(app.modal.is_none(), "{c}: only core decides to ask");
    }
    // `C` cancels only a shown reload warning; without one it is refused.
    assert!(matches!(
        &app.on_key(ch('C')),
        AppAction::Refused(refusal)
            if refusal.command == Some(frontend_core::OperatorCommand::CancelReload)
    ));
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

#[cfg(not(feature = "js"))]
#[test]
fn script_commands_are_reachable_at_80x24() {
    let mut app = script_app();
    app.script_state = ScriptRunState::Running;
    let id = CompiledCardId::QUESTER;
    app.script_sel = Some(ScriptChoice::compiled(id));
    app.script_cards = vec![BrowseCard {
        name: id.as_str().into(),
        description: String::new(),
        category: "Questing".into(),
        tags: Vec::new(),
        kind: CardRuntime::Compiled,
        source: CardSource::Builtin,
    }];
    app.script_card_choices = vec![ScriptChoice::compiled(id)];
    app.params_schema = owned_schema(&thiever_schema());

    let rows = draw(&mut app, 80, 24);
    let all = text(&rows);
    assert!(
        all.contains("[Browse b] [Start t] [Pause P] [Stop e]"),
        "native script controls paint: {all}"
    );
    assert!(
        all.contains("[Params v] [Start fleet T] [Stop fleet E]"),
        "compiled settings and fleet controls paint: {all}"
    );
    assert!(
        !all.contains("[Load f]"),
        "file loading is not exposed: {all}"
    );
    assert!(!all.contains("[Reload R]"), "reload is not exposed: {all}");

    assert_eq!(
        app.on_key(ch('t')),
        AppAction::ScriptStart(ScriptChoice::compiled(id))
    );
    assert_eq!(app.on_key(ch('P')), AppAction::ScriptPause);
    assert_eq!(app.on_key(ch('e')), AppAction::ScriptStop);
    assert_eq!(app.on_key(ch('f')), AppAction::None);
}

/// With `TUI_CAPTURE_DIR` set, record the control at both reference terminal sizes.
#[test]
fn running_quester_skip_control_is_clickable_at_both_terminal_sizes_and_in_help() {
    let id = CompiledCardId::QUESTER;
    let mut app = script_app();
    app.script_state = ScriptRunState::Running;
    app.script_sel = Some(ScriptChoice::compiled(id));
    let mut detail = crate::test_support::ready_detail("alice");
    detail.row_mut().set_script(ScriptRunState::Running);
    app.detail = Some(NativeStatus::working(id).attach_to(detail));

    let capture_dir = std::env::var_os("TUI_CAPTURE_DIR").map(std::path::PathBuf::from);
    if let Some(dir) = capture_dir.as_deref() {
        std::fs::create_dir_all(dir).unwrap();
    }

    for (width, height) in [(120, 40), (80, 24)] {
        let rows = draw(&mut app, width, height);
        let footer = rows.last().expect("footer row");
        assert!(
            footer.contains("X skip"),
            "{width}x{height}: missing skip hint: {footer}"
        );
        #[cfg(feature = "js")]
        for key in ["f load", "v params", "R reload"] {
            assert!(
                footer.contains(key),
                "{width}x{height}: missing {key:?}: {footer}"
            );
        }
        if let Some(dir) = capture_dir.as_deref() {
            crate::bin::manual_click_live_tests::capture(
                &mut app,
                width,
                height,
                dir,
                &format!("{width}x{height}"),
                "quester-skip-control",
                &serde_json::json!({
                    "card": "Quester",
                    "state": "running",
                    "skip_key": "X",
                    "skip_hint": footer.contains("X skip"),
                    "load_hint": footer.contains("f load"),
                    "params_hint": footer.contains("v params"),
                    "reload_hint": footer.contains("R reload"),
                }),
            )
            .unwrap();
        }
        let (x, y) = find(&rows, "[Skip quest X]").expect("running Quester button");
        assert!(
            matches!(
                app.on_click(x + 1, y),
                AppAction::ScriptSkipQuest(Some(_))
            ),
            "{width}x{height}: click"
        );
        assert!(
            matches!(app.on_key(ch('X')), AppAction::ScriptSkipQuest(Some(_))),
            "{width}x{height}: key"
        );
        assert_eq!(app.on_key(ch('k')), AppAction::None, "{width}x{height}: k");
    }
    assert!(crate::help::help_rows(&app)
        .iter()
        .any(|row| row.context == "Script" && row.keys == "X" && row.what == "Skip quest"));
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

/// The host-capacity warning is a wrapped banner above the header: whole
/// at 80x24 and 120x40, on the fleet and on every detail tab.
#[test]
fn capacity_warning_is_whole_above_the_header_at_80x24() {
    let line = frontend_core::CapacityNotice::from_warning(frontend_core::CapacityWarning::new(
        18,
        92,
        10,
        Some(std::time::Duration::from_millis(1470)),
        Some(std::time::Duration::from_millis(750)),
        frontend_core::RenderPolicyLabel::Headless,
    ))
    .notice_text()
    .to_owned();
    assert!(line.len() > 80, "the line must need wrapping: {line}");
    for (width, height, banner) in [(80, 24, 2), (120, 40, 1)] {
        for screen in [None, Some(KeyCode::F(4)), Some(KeyCode::F(7))] {
            let mut app = TuiApp::new("274bot headless");
            app.capacity = Some(line.clone());
            if let Some(code) = screen {
                app.on_key(key(code));
            }
            let rows = draw(&mut app, width, height);
            let shown = rows[..banner]
                .iter()
                .map(|row| row.trim_end())
                .collect::<Vec<_>>()
                .join(" ");
            assert_eq!(shown, line, "{width}x{height} {screen:?}");
            assert!(
                rows[banner].contains("274bot"),
                "the header follows the banner: {}",
                text(&rows)
            );
        }
    }
    let mut app = TuiApp::new("274bot headless");
    let rows = draw(&mut app, 80, 24);
    assert!(rows[0].contains("274bot"), "no warning, no banner");
}

/// Every member of `app` can walk.
fn all_eligible(_: &str) -> WalkSlotStatus {
    WalkSlotStatus::eligible(tile(1, 1))
}

/// The Group checklist is the fleet model's marks: it has no flags of its
/// own, so its count follows `SetMark` and `MarkAll` through the command
/// seam and nothing else.
#[test]
fn the_group_checklist_count_follows_the_models_marks() {
    let mut seam = Seam::new();
    let mut app = crate::test_support::fleet_app(&["alice", "bob"]);
    app.refresh_walk_send(all_eligible);
    let count = |app: &TuiApp| app.walk_send.checked_count(app.fleet_model.marks());
    assert_eq!(count(&app), 0);
    seam.apply(&mut app, AppAction::SetMark("alice".into(), true));
    assert_eq!(count(&app), 1, "a mark checks the row");
    assert!(app.walk_send.rows()[0].checked(app.fleet_model.marks()));
    assert!(!app.walk_send.rows()[1].checked(app.fleet_model.marks()));
    seam.apply(&mut app, AppAction::SetMark("bob".into(), true));
    assert_eq!(count(&app), 2);
    seam.apply(&mut app, AppAction::SetMark("alice".into(), false));
    assert_eq!(count(&app), 1, "an unmark unchecks it");
    // An ineligible marked row is not checked.
    app.refresh_walk_send(|name| {
        if name == "bob" {
            WalkSlotStatus::excluded(WalkExclude::NoPosition)
        } else {
            all_eligible(name)
        }
    });
    assert_eq!(count(&app), 0);
    // Group mode with nothing checked marks every eligible row through
    // `MarkAll`, and keeps no flag of its own.
    let alice = app.profile_id_for_name("alice");
    assert_eq!(app.toggle_walk_send_mode(), AppAction::MarkAll(vec![alice]));
    assert_eq!(count(&app), 0, "nothing is checked before MarkAll lands");
    seam.apply(&mut app, AppAction::MarkAll(vec![alice]));
    assert_eq!(count(&app), 1);
    assert_eq!(app.sync_walk_label(), "Walk 1 bots");
}

/// With nothing marked, the Map's group send (`g`) marks the bots a fleet
/// action covers without marks: the loaded members the fleet filter
/// shows. A member the filter hides is not added, though it can walk.
#[test]
fn the_group_send_without_marks_keeps_the_shown_scope() {
    let mut seam = Seam::new();
    let mut app = open_map_world();
    assert_eq!(app.on_key(key(KeyCode::F(4))), AppAction::MapOpen);
    app.refresh_walk_send(all_eligible);
    seam.filter(&mut app.fleet_model, "alice");
    assert!(app.fleet_model.marks().is_empty());
    assert_eq!(app.fleet_model.counts().shown_loaded(), 1, "bob is hidden");
    let alice = app.profile_id_for_name("alice");
    seam.press(&mut app, ch('g'), AppAction::MarkAll(vec![alice]));
    assert_eq!(app.walk_send.mode, WalkSendMode::Group);
    let checked: Vec<&str> = app
        .walk_send
        .rows()
        .iter()
        .filter(|row| row.checked(app.fleet_model.marks()))
        .map(|row| row.name())
        .collect();
    assert_eq!(checked, ["alice"], "the hidden bob is not added");
    assert_eq!(app.fleet_model.marks().len(), 1);
    assert_eq!(app.sync_walk_label(), "Walk 1 bots");
}

/// A warm, unchanged Group Map draw borrows the cached Walk label: it is
/// not rebuilt (no count, no new string) until a mark, the rows or the
/// mode moves, and then it is rebuilt once in its own buffer.
#[test]
fn a_steady_group_map_draw_reuses_the_walk_label() {
    let mut seam = Seam::new();
    let mut app = open_map_world();
    assert_eq!(app.on_key(key(KeyCode::F(4))), AppAction::MapOpen);
    app.refresh_walk_send(all_eligible);
    let ids = vec![
        app.profile_id_for_name("alice"),
        app.profile_id_for_name("bob"),
    ];
    seam.press(&mut app, ch('g'), AppAction::MarkAll(ids));
    assert_eq!(app.walk_send.mode, WalkSendMode::Group);
    let first = text(&draw(&mut app, 160, 40));
    assert!(first.contains("Walk 2 bots"), "{first}");
    let builds = app.walk_send.label_builds;
    let buffer = app.walk_send.walk_label().as_ptr();
    for _ in 0..5 {
        draw(&mut app, 160, 40);
    }
    assert_eq!(
        app.walk_send.label_builds, builds,
        "steady draws rebuild nothing"
    );
    assert_eq!(
        app.walk_send.walk_label().as_ptr(),
        buffer,
        "the same string"
    );

    seam.apply(&mut app, AppAction::SetMark("bob".into(), false));
    let after = text(&draw(&mut app, 160, 40));
    assert!(
        after.contains("Walk 1 bots"),
        "an unmark moves the label: {after}"
    );
    assert_eq!(app.walk_send.label_builds, builds + 1, "rebuilt once");
    assert_eq!(
        app.walk_send.walk_label().as_ptr(),
        buffer,
        "in its own buffer"
    );
    draw(&mut app, 160, 40);
    assert_eq!(app.walk_send.label_builds, builds + 1);
}

/// Unloading a marked profile keeps its mark: the table still shows it
/// marked and counts it, and once it is loaded again the Group checklist
/// has it checked.
#[test]
fn unloading_a_marked_profile_keeps_its_mark_for_the_next_group_walk() {
    use frontend_core::ProfileIdentity;
    let mut seam = Seam::with_profiles(&[("alice", 1), ("bob", 2)]);
    let mut app = crate::test_support::fleet_app(&["alice", "bob"]);
    let ids = [ProfileIdentity::uid(1), ProfileIdentity::uid(2)];
    crate::test_support::set_profile_ids(&mut app, &ids);
    seam.apply(&mut app, AppAction::SetMark("bob".into(), true));
    assert!(app.fleet_model.marks().contains(ids[1]));

    // Bob logs out: he leaves the members but stays a vault profile.
    app.names = vec!["alice".into()];
    app.profile_ids = vec![ids[0]];
    app.fleet_model.set_universe_for_test(
        vec![member("alice", Phase::Ready), FleetRow::for_profile("bob")],
        &ids,
    );
    app.focused = Some(0);
    assert!(
        app.fleet_model.marks().contains(ids[1]),
        "the mark outlives the member"
    );
    let screen = text(&draw(&mut app, 120, 40));
    assert!(screen.contains("[x]"), "the table still shows it: {screen}");
    assert!(screen.contains("2/2 shown · marked 1"), "{screen}");

    // He logs in again: the checklist has him checked.
    app.names = vec!["alice".into(), "bob".into()];
    app.profile_ids = ids.to_vec();
    app.fleet_model.set_universe_for_test(
        vec![member("alice", Phase::Ready), member("bob", Phase::Ready)],
        &ids,
    );
    app.refresh_walk_send(all_eligible);
    let marks = app.fleet_model.marks();
    let rows = app.walk_send.rows();
    assert!(!rows[0].checked(marks));
    assert!(rows[1].checked(marks), "bob is checked again");
    assert_eq!(app.walk_send.checked_count(marks), 1);
    // Entering Group mode keeps the existing mark: no `MarkAll`.
    assert_eq!(app.toggle_walk_send_mode(), AppAction::None);
    assert_eq!(app.sync_walk_label(), "Walk 1 bots");
}

/// The Space key on the Map toggles the focused bot's mark through
/// `SetMark`, from the model's marks.
#[test]
fn space_on_the_map_toggles_the_focused_bots_model_mark() {
    let mut seam = Seam::new();
    let mut app = open_map_world();
    app.refresh_walk_send(all_eligible);
    assert_eq!(
        app.toggle_walk_send_focused(),
        AppAction::SetMark("alice".into(), true)
    );
    seam.apply(&mut app, AppAction::SetMark("alice".into(), true));
    assert_eq!(
        app.toggle_walk_send_focused(),
        AppAction::SetMark("alice".into(), false)
    );
}
