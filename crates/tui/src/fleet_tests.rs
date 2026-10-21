use frontend_core::{FleetModel, FleetRow, Phase, ProfileIdentity, QueuePlace};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use std::collections::HashSet;

use super::*;
use crate::bin::seam::Seam;

fn ready(name: &str, world: u16) -> FleetRow {
    FleetRow::fixture(name, Phase::Ready, Some(world), None)
}

fn queued(name: &str, position: u32, total: u32) -> FleetRow {
    FleetRow::fixture(
        name,
        Phase::Queued,
        Some(1),
        Some(QueuePlace::new(position, total)),
    )
}

fn model_of(rows: Vec<FleetRow>) -> FleetModel {
    let mut model = FleetModel::new();
    model.set_universe_for_test(rows, &[]);
    model
}

fn shown_names(model: &FleetModel) -> Vec<&str> {
    model.shown_rows().map(|(_, row)| row.name()).collect()
}

fn id(name: &str) -> ProfileIdentity {
    ProfileIdentity::synthetic(name)
}

#[test]
fn filter_terms_match_name_world_and_state() {
    let mut model = model_of(vec![
        ready("alice", 2),
        queued("bob", 2, 5),
        ready("carol", 1),
    ]);
    let mut seam = Seam::new();
    let mut shown = |model: &mut FleetModel, filter: &str| {
        seam.filter(model, filter);
        shown_names(model)
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>()
    };
    assert_eq!(shown(&mut model, "AL"), ["alice"], "name, case-insensitive");
    assert_eq!(shown(&mut model, "w2"), ["alice"], "world term");
    assert_eq!(shown(&mut model, "world:1"), ["bob", "carol"]);
    assert_eq!(shown(&mut model, "queued"), ["bob"], "phase term");
    assert_eq!(shown(&mut model, "idle"), ["alice", "carol"], "status term");
    assert_eq!(
        shown(&mut model, "ready w1"),
        ["carol"],
        "every term must match"
    );
    assert_eq!(shown(&mut model, ""), ["alice", "bob", "carol"]);
}

#[test]
fn filter_treats_an_unprojected_member_as_offline() {
    assert!(frontend_core::fleet_row_matches("dave", None, "offline"));
    assert!(frontend_core::fleet_row_matches("dave", None, "dav off"));
    assert!(!frontend_core::fleet_row_matches("dave", None, "ready"));
    assert!(
        !frontend_core::fleet_row_matches("dave", None, "w301"),
        "no row, no world"
    );
}

#[test]
fn the_cursor_stays_on_its_profile_when_the_filter_or_fleet_changes() {
    let mut model = model_of(vec![ready("alice", 2), ready("bob", 2), ready("carol", 2)]);
    let mut state = FleetState::default();
    let mut seam = Seam::new();
    state.sync(&model);
    state.move_cursor(2, &model);
    assert_eq!(state.cursor_row(&model), Some(2), "cursor on carol");
    seam.filter(&mut model, "o"); // bob, carol
    state.sync(&model);
    let on = |state: &FleetState, model: &FleetModel| {
        state
            .cursor_row(model)
            .map(|index| model.rows()[index].name().to_owned())
    };
    assert_eq!(on(&state, &model).as_deref(), Some("carol"));
    seam.filter(&mut model, "");
    model.set_universe_for_test(vec![ready("bob", 2), ready("carol", 2)], &[]);
    state.sync(&model); // alice leaves: carol moves up one row
    assert_eq!(on(&state, &model).as_deref(), Some("carol"));
    assert_eq!(state.cursor, 1);
}

#[test]
fn the_cursor_follows_the_identity_not_the_name() {
    // A rename keeps the vault identity: the cursor stays on that profile.
    let mut model = FleetModel::new();
    model.set_universe_for_test(
        vec![ready("alice", 2), ready("bob", 2)],
        &[ProfileIdentity::uid(1), ProfileIdentity::uid(2)],
    );
    let mut state = FleetState::default();
    state.sync(&model);
    state.move_cursor(1, &model);
    model.set_universe_for_test(
        vec![ready("bob-renamed", 2), ready("alice", 2)],
        &[ProfileIdentity::uid(2), ProfileIdentity::uid(1)],
    );
    state.sync(&model);
    assert_eq!(state.cursor, 0, "uid 2 moved to the top");
    assert_eq!(
        state.cursor_row(&model).map(|i| model.rows()[i].name()),
        Some("bob-renamed")
    );
}

#[test]
fn an_unloaded_profile_keeps_its_mark_and_its_row() {
    let mut model = model_of(vec![ready("alice", 2), ready("bob", 2)]);
    model.set_mark_for_test(id("bob"), true);
    // Bob is unloaded: his row stays in the universe as a vault profile.
    model.set_universe_for_test(vec![ready("alice", 2), FleetRow::for_profile("bob")], &[]);
    assert!(model.marks().contains(id("bob")), "the mark stays");
    assert_eq!(model.counts().marked(), 1);
}

fn render(table: FleetTable<'_>, w: u16, h: u16) -> (Vec<String>, Buffer, FleetHits) {
    let area = Rect::new(0, 0, w, h);
    let mut buf = Buffer::empty(area);
    let hits = table.render(area, &mut buf, 0);
    let rows = (0..h)
        .map(|y| (0..w).map(|x| buf[(x, y)].symbol()).collect::<String>())
        .collect();
    (rows, buf, hits)
}

#[test]
fn rows_show_selection_cursor_and_selected_bot_as_plain_text() {
    let mut model = model_of(vec![
        ready("alice", 2),
        queued("bob", 2, 5),
        FleetRow::fixture("carol", Phase::Offline, None, None),
    ]);
    model.set_mark_for_test(id("carol"), true);
    let mut state = FleetState::default();
    state.sync(&model);
    let restart_badges = HashSet::from([id("bob")]);
    state.move_cursor(1, &model);
    let (rows, _, hits) = render(
        FleetTable {
            model: &model,
            restart_badges: &restart_badges,
            state: &mut state,
            selected: Some(id("alice")),
            keys: true,
            class: SizeClass::Compact,
        },
        40,
        6,
    );
    assert!(rows[1].starts_with("[ ]  *alice"), "{rows:?}");
    assert!(
        rows[1].contains("w2") && rows[1].contains("idle"),
        "the panel's status label: {rows:?}"
    );
    assert!(rows[2].contains('!'), "restart badge: {rows:?}");
    assert!(rows[2].starts_with("[ ] > bob"), "cursor row: {rows:?}");
    assert!(rows[2].contains("queued 2/5"), "{rows:?}");
    assert!(rows[3].starts_with("[x]   carol"), "marked row: {rows:?}");
    assert!(rows[3].contains("offline"), "{rows:?}");
    assert_eq!(
        rows[5].trim_end(),
        "3/3 shown · marked 1 · BOT alice",
        "{rows:?}"
    );
    assert_eq!(hits.rows.y, 1, "rows start under the column header");
    assert_eq!(hits.first, 0);
}

#[test]
fn an_unloaded_vault_profile_is_listed_after_the_loaded_rows_dim_and_markable() {
    let mut model = FleetModel::new();
    model.set_universe_for_test(
        vec![
            ready("alice", 2),
            FleetRow::for_profile("zed"),
            FleetRow::for_profile("yan"),
        ],
        &[],
    );
    model.set_mark_for_test(id("zed"), true);
    let mut state = FleetState::default();
    let (rows, buf, _) = render(
        FleetTable {
            model: &model,
            restart_badges: &HashSet::new(),
            state: &mut state,
            selected: Some(id("alice")),
            keys: false,
            class: SizeClass::Standard,
        },
        60,
        7,
    );
    assert!(
        rows[1].contains("alice") && rows[1].contains("w2"),
        "{rows:?}"
    );
    assert!(
        rows[2].starts_with("[x]") && rows[2].contains("zed") && rows[2].contains("not loaded"),
        "an unloaded row is markable and says why it has no state: {rows:?}"
    );
    assert!(
        rows[3].contains("yan") && rows[3].contains("not loaded"),
        "{rows:?}"
    );
    assert!(
        !rows[2].contains('*') && !rows[2].contains("loc"),
        "{rows:?}"
    );
    assert!(rows[2].contains(" - "), "the world column is `-`: {rows:?}");
    assert!(
        buf[(8, 2)].modifier.contains(Modifier::DIM),
        "an unloaded row is dim"
    );
    assert!(
        !buf[(8, 1)].modifier.contains(Modifier::DIM),
        "a loaded row is not"
    );
    assert!(
        rows[6].contains("3/3 shown · marked 1"),
        "the count line counts unloaded rows: {rows:?}"
    );
}

#[test]
fn the_count_line_says_how_many_marks_the_filter_hides() {
    let mut model = model_of(vec![ready("alice", 2), ready("bob", 2), ready("carol", 2)]);
    model.set_mark_for_test(id("alice"), true);
    model.set_mark_for_test(id("bob"), true);
    let mut seam = Seam::new();
    seam.filter(&mut model, "bob");
    let mut state = FleetState::default();
    let (rows, _, _) = render(
        FleetTable {
            model: &model,
            restart_badges: &HashSet::new(),
            state: &mut state,
            selected: None,
            keys: false,
            class: SizeClass::Standard,
        },
        60,
        8,
    );
    assert!(
        rows.iter()
            .any(|row| row.trim_end() == "1/3 shown · marked 2 (1 not shown)"),
        "{rows:?}"
    );
    assert!(model.marks().contains(id("alice")), "the hidden mark stays");
}

#[test]
fn the_selected_marker_only_goes_to_a_loaded_row() {
    let mut model = FleetModel::new();
    model.set_universe_for_test(vec![ready("alice", 2), FleetRow::for_profile("bob")], &[]);
    let mut state = FleetState::default();
    let (rows, _, _) = render(
        FleetTable {
            model: &model,
            restart_badges: &HashSet::new(),
            state: &mut state,
            selected: Some(id("bob")),
            keys: false,
            class: SizeClass::Compact,
        },
        40,
        6,
    );
    assert!(!rows[1].contains('*') && !rows[2].contains('*'), "{rows:?}");
}

#[test]
fn an_empty_universe_says_how_to_load_profiles() {
    let model = FleetModel::new();
    let mut state = FleetState::default();
    let (rows, _, _) = render(
        FleetTable {
            model: &model,
            restart_badges: &HashSet::new(),
            state: &mut state,
            selected: None,
            keys: false,
            class: SizeClass::Standard,
        },
        50,
        6,
    );
    assert!(
        rows.iter()
            .any(|row| row.contains("no bots loaded: m loads every vault profile")),
        "{rows:?}"
    );
}

#[test]
fn a_long_fleet_scrolls_to_keep_the_cursor_visible() {
    let model = model_of(
        (0..100)
            .map(|i| ready(&format!("bot{i:03}"), 2))
            .collect::<Vec<_>>(),
    );
    let mut state = FleetState::default();
    state.sync(&model);
    state.cursor_to(60, &model);
    let (rows, _, hits) = render(
        FleetTable {
            model: &model,
            restart_badges: &HashSet::new(),
            state: &mut state,
            selected: None,
            keys: true,
            class: SizeClass::Standard,
        },
        40,
        12,
    );
    assert!(hits.first > 0 && hits.first <= 60 && 60 < hits.first + usize::from(hits.rows.height));
    assert!(rows.iter().any(|r| r.contains("> bot060")), "{rows:?}");
}
