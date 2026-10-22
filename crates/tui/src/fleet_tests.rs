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
            header_parts: &mut Vec::new(),
        },
        40,
        9,
    );
    // Two header rows (the pane is 40 wide), then the column header.
    assert!(rows[3].starts_with("[ ]  *alice"), "{rows:?}");
    assert!(
        rows[3].contains("w2") && rows[3].contains("idle"),
        "the panel's status label: {rows:?}"
    );
    assert!(rows[4].contains('!'), "restart badge: {rows:?}");
    assert!(rows[4].starts_with("[ ] > bob"), "cursor row: {rows:?}");
    assert!(rows[4].contains("queued 2/5"), "{rows:?}");
    assert!(rows[5].starts_with("[x]   carol"), "marked row: {rows:?}");
    assert!(rows[5].contains("offline"), "{rows:?}");
    assert_eq!(
        rows[8].trim_end(),
        "3/3 shown · marked 1 · BOT alice",
        "{rows:?}"
    );
    assert_eq!(hits.rows.y, 3, "rows start under the column header");
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
            header_parts: &mut Vec::new(),
        },
        60,
        7,
    );
    assert!(
        rows[2].contains("alice") && rows[2].contains("w2"),
        "{rows:?}"
    );
    assert!(
        rows[3].starts_with("[x]") && rows[3].contains("zed") && rows[3].contains("not loaded"),
        "an unloaded row is markable and says why it has no state: {rows:?}"
    );
    assert!(
        rows[4].contains("yan") && rows[4].contains("not loaded"),
        "{rows:?}"
    );
    assert!(
        !rows[3].contains('*') && !rows[3].contains("loc"),
        "{rows:?}"
    );
    assert!(rows[3].contains(" - "), "the world column is `-`: {rows:?}");
    assert!(
        buf[(8, 3)].modifier.contains(Modifier::DIM),
        "an unloaded row is dim"
    );
    assert!(
        !buf[(8, 2)].modifier.contains(Modifier::DIM),
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
            header_parts: &mut Vec::new(),
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
            header_parts: &mut Vec::new(),
        },
        40,
        6,
    );
    assert!(!rows[3].contains('*') && !rows[4].contains('*'), "{rows:?}");
}

#[test]
fn an_empty_universe_says_there_is_nothing_to_act_on() {
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
            header_parts: &mut Vec::new(),
        },
        50,
        6,
    );
    assert!(
        rows.iter()
            .any(|row| row.contains("no profiles: nothing to mark or log in")),
        "{rows:?}"
    );
}

fn header_rows(model: &FleetModel, state: &mut FleetState, width: u16) -> Vec<String> {
    let (rows, _, _) = render(
        FleetTable {
            model,
            restart_badges: &HashSet::new(),
            state,
            selected: None,
            keys: false,
            class: SizeClass::Standard,
            header_parts: &mut Vec::new(),
        },
        width,
        8,
    );
    rows
}

/// The header names the card, the sort and the marked count on one row
/// when they fit, and wraps each part whole onto the next row when the
/// pane is narrow, so none is cut off.
#[test]
fn the_header_names_the_card_the_sort_and_the_marks() {
    let mut model = model_of(vec![ready("alice", 2), ready("bob", 2), ready("carol", 2)]);
    model.set_mark_for_test(id("alice"), true);
    model.set_mark_for_test(id("bob"), true);
    let mut state = FleetState::default();
    let rows = header_rows(&model, &mut state, 80);
    assert_eq!(
        rows[0].trim_end(),
        "card: Each bot's own card · sort: Load order · 2 marked"
    );
    model.set_order_for_test(frontend_core::FleetOrder::Name);
    let rows = header_rows(&model, &mut state, 80);
    assert_eq!(
        rows[0].trim_end(),
        "card: Each bot's own card · sort: Name · 2 marked"
    );

    // A 34-cell pane (the 120-column layout's fleet pane) wraps by part.
    let rows = header_rows(&model, &mut state, 34);
    assert_eq!(rows[0].trim_end(), "card: Each bot's own card");
    assert_eq!(rows[1].trim_end(), "sort: Name · 2 marked");
    assert!(rows[2].starts_with("sel "), "the column header follows");
}

/// The header text is built when the card label, the order or the marked
/// count changes, not every draw.
#[test]
fn the_header_is_rebuilt_only_when_an_input_changed() {
    let mut model = model_of(vec![ready("alice", 2), ready("bob", 2)]);
    let mut state = FleetState::default();
    header_rows(&model, &mut state, 80);
    // Overwrite the cached row: a draw that rebuilt it would restore it.
    state.header.lines[0] = "CACHED".into();
    let rows = header_rows(&model, &mut state, 80);
    assert!(rows[0].starts_with("CACHED"), "{rows:?}");
    // The recorded cells are cached with the text.
    state.header.segments[0].width = 3;
    let cells = header_cells(&model, &mut state, 80);
    assert_eq!(cells[0].0.width, 3, "{cells:?}");
    // A different width rebuilds it too.
    let rows = header_rows(&model, &mut state, 60);
    assert!(rows[0].starts_with("card: "), "{rows:?}");
    state.header.lines[0] = "CACHED".into();
    let rows = header_rows(&model, &mut state, 60);
    assert!(rows[0].contains("CACHED"), "{rows:?}");
    model.set_mark_for_test(id("alice"), true);
    let rows = header_rows(&model, &mut state, 80);
    assert!(
        rows[0].contains("1 marked") && !rows[0].contains("CACHED"),
        "a mark rebuilds it: {rows:?}"
    );
    model.set_order_for_test(frontend_core::FleetOrder::Status);
    let rows = header_rows(&model, &mut state, 80);
    assert!(rows[0].contains("sort: Status"), "{rows:?}");
}

/// A card label wider than the pane wraps at its spaces: every part reads
/// in full. A single word wider than the pane is cut into rows.
#[test]
fn a_long_card_label_wraps_by_word_and_a_long_word_is_cut() {
    let mut lines = Vec::new();
    wrap_header(
        [
            "card: Focused bot's card: Alcher (pending)".into(),
            "sort: Load order".into(),
            "0 marked".into(),
        ],
        32,
        &mut lines,
        &mut Vec::new(),
    );
    assert_eq!(
        lines,
        [
            "card: Focused bot's card: Alcher",
            "(pending) · sort: Load order",
            "0 marked"
        ]
    );
    wrap_header(
        [
            "card: /a/very/long/path/with/no/spaces.ts".into(),
            "sort: Name".into(),
            "1 marked".into(),
        ],
        16,
        &mut lines,
        &mut Vec::new(),
    );
    assert_eq!(
        lines,
        [
            "card:",
            "/a/very/long/pat",
            "h/with/no/spaces",
            ".ts · sort: Name",
            "1 marked"
        ]
    );
    assert!(
        lines.iter().all(|line| line.chars().count() <= 16),
        "{lines:?}"
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
            header_parts: &mut Vec::new(),
        },
        40,
        12,
    );
    assert!(hits.first > 0 && hits.first <= 60 && 60 < hits.first + usize::from(hits.rows.height));
    assert!(rows.iter().any(|r| r.contains("> bot060")), "{rows:?}");
}

/// The cells the header parts were recorded in, for a draw at `width`.
fn header_cells(model: &FleetModel, state: &mut FleetState, width: u16) -> Vec<(Rect, HeaderPart)> {
    let mut parts = Vec::new();
    render(
        FleetTable {
            model,
            restart_badges: &HashSet::new(),
            state,
            selected: None,
            keys: false,
            class: SizeClass::Standard,
            header_parts: &mut parts,
        },
        width,
        8,
    );
    parts
}

/// The card and sort parts are recorded where they are drawn; the marked
/// count is not clickable. A narrow pane records each part on its own row.
#[test]
fn the_header_records_the_cells_of_the_card_and_sort_parts() {
    let mut model = model_of(vec![ready("alice", 2)]);
    let mut state = FleetState::default();
    // "card: Each bot's own card" is 25 cells; the separator is 3.
    assert_eq!(
        header_cells(&model, &mut state, 80),
        [
            (Rect::new(0, 0, 25, 1), HeaderPart::Card),
            (Rect::new(28, 0, 16, 1), HeaderPart::Sort),
        ]
    );
    model.set_order_for_test(frontend_core::FleetOrder::Name);
    assert_eq!(
        header_cells(&model, &mut state, 34),
        [
            (Rect::new(0, 0, 25, 1), HeaderPart::Card),
            (Rect::new(0, 1, 10, 1), HeaderPart::Sort),
        ],
        "sort: Name moved to the second row"
    );
}

/// A part that wraps has one cell run per row, and a cut word records the
/// cells it fills.
#[test]
fn a_wrapped_part_records_one_cell_run_per_row() {
    let mut lines = Vec::new();
    let mut segments = Vec::new();
    wrap_header(
        [
            "card: Focused bot's card: Alcher (pending)".into(),
            "sort: Load order".into(),
            "0 marked".into(),
        ],
        32,
        &mut lines,
        &mut segments,
    );
    let cells: Vec<_> = segments
        .iter()
        .map(|s| (s.part, s.line, s.x, s.width))
        .collect();
    assert_eq!(
        cells,
        [
            (HeaderPart::Card, 0, 0, 32),
            (HeaderPart::Card, 1, 0, 9),
            (HeaderPart::Sort, 1, 12, 16),
        ]
    );
    wrap_header(
        [
            "card: /a/very/long/path/with/no/spaces.ts".into(),
            "sort: Name".into(),
            "1 marked".into(),
        ],
        16,
        &mut lines,
        &mut segments,
    );
    let cells: Vec<_> = segments
        .iter()
        .map(|s| (s.part, s.line, s.x, s.width))
        .collect();
    assert_eq!(
        cells,
        [
            (HeaderPart::Card, 0, 0, 5),
            (HeaderPart::Card, 1, 0, 16),
            (HeaderPart::Card, 2, 0, 16),
            (HeaderPart::Card, 3, 0, 3),
            (HeaderPart::Sort, 3, 6, 10),
        ]
    );
}

/// Header rows the pane has no room for have no clickable cells.
#[test]
fn a_header_row_that_is_not_drawn_records_no_cells() {
    let model = model_of(vec![ready("alice", 2)]);
    let mut state = FleetState::default();
    let mut parts = Vec::new();
    let area = Rect::new(0, 0, 34, 1);
    let mut buf = Buffer::empty(area);
    FleetTable {
        model: &model,
        restart_badges: &HashSet::new(),
        state: &mut state,
        selected: None,
        keys: false,
        class: SizeClass::Standard,
        header_parts: &mut parts,
    }
    .render(area, &mut buf, 0);
    assert_eq!(
        parts,
        [(Rect::new(0, 0, 25, 1), HeaderPart::Card)],
        "the sort part would sit on the second row, which is not drawn"
    );
}

/// The order after each choice wraps at the end of the choices.
#[test]
fn the_next_order_follows_the_choices_and_wraps() {
    use frontend_core::FleetOrder::{LoadOrder, Name, Status};
    assert_eq!(next_order(LoadOrder), Name);
    assert_eq!(next_order(Name), Status);
    assert_eq!(next_order(Status), LoadOrder);
}
