use frontend_core::{FleetRow, Phase, ProfileIdentity, QueuePlace};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use std::collections::HashSet;

use super::*;

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

fn ready(name: &str, world: u16) -> FleetRow {
    FleetRow::fixture(name, Phase::Ready, Some(world), None)
}

fn queued(name: &str, position: u32, total: u32) -> FleetRow {
    FleetRow::fixture(
        name,
        Phase::Queued,
        Some(1),
        Some(QueuePlace { position, total }),
    )
}

#[test]
fn filter_terms_match_name_world_and_state() {
    let members = names(&["alice", "bob", "carol"]);
    let rows = vec![ready("alice", 2), queued("bob", 2, 5), ready("carol", 1)];
    let mut state = FleetState::default();
    let shown = |state: &mut FleetState, filter: &str| {
        state.filter = filter.into();
        state.sync_with_ids(&members, &[], &rows);
        state
            .shown()
            .iter()
            .map(|&i| members[i].as_str())
            .collect::<Vec<_>>()
    };
    assert_eq!(shown(&mut state, "AL"), ["alice"], "name, case-insensitive");
    assert_eq!(shown(&mut state, "w2"), ["alice"], "world term");
    assert_eq!(shown(&mut state, "world:1"), ["bob", "carol"]);
    assert_eq!(shown(&mut state, "queued"), ["bob"], "phase term");
    assert_eq!(shown(&mut state, "idle"), ["alice", "carol"], "status term");
    assert_eq!(
        shown(&mut state, "ready w1"),
        ["carol"],
        "every term must match"
    );
    assert_eq!(shown(&mut state, ""), ["alice", "bob", "carol"]);
}

#[test]
fn the_cursor_stays_on_its_member_when_the_filter_or_fleet_changes() {
    let mut members = names(&["alice", "bob", "carol"]);
    let rows = Vec::new();
    let mut state = FleetState::default();
    state.sync_with_ids(&members, &[], &rows);
    state.move_cursor(2, &members);
    assert_eq!(state.cursor_member(), Some(2), "cursor on carol");
    state.filter = "o".into(); // bob, carol
    state.sync_with_ids(&members, &[], &rows);
    assert_eq!(
        state.cursor_member().map(|i| members[i].as_str()),
        Some("carol")
    );
    state.filter.clear();
    members.remove(0); // alice leaves: carol moves up one row
    state.sync_with_ids(&members, &[], &rows);
    assert_eq!(
        state.cursor_member().map(|i| members[i].as_str()),
        Some("carol")
    );
}

#[test]
fn a_departed_member_loses_its_row_selection() {
    let mut members = names(&["alice", "bob"]);
    let mut state = FleetState::default();
    state.sync_with_ids(&members, &[], &[]);
    state.toggle_mark(&members, 0);
    state.toggle_mark(&members, 1);
    assert_eq!(state.selection.len(), 2);
    members.retain(|n| n != "bob");
    state.sync_with_ids(&members, &[], &[]);
    assert!(state
        .selection
        .contains(ProfileIdentity::synthetic("alice")));
    assert!(
        !state.selection.contains(ProfileIdentity::synthetic("bob")),
        "a removed member is not a hidden group target"
    );
}

fn render(table: FleetTable<'_>, w: u16, h: u16) -> (Vec<String>, FleetHits) {
    let area = Rect::new(0, 0, w, h);
    let mut buf = Buffer::empty(area);
    let hits = table.render(area, &mut buf, 0);
    let rows = (0..h)
        .map(|y| (0..w).map(|x| buf[(x, y)].symbol()).collect::<String>())
        .collect();
    (rows, hits)
}

#[test]
fn rows_show_selection_cursor_and_selected_bot_as_plain_text() {
    let members = names(&["alice", "bob", "carol"]);
    let rows = vec![ready("alice", 2), queued("bob", 2, 5)];
    let mut state = FleetState::default();
    state.sync_with_ids(&members, &[], &rows);
    state.toggle_mark(&members, 2);
    let restart_badges = HashSet::from([ProfileIdentity::synthetic("bob")]);
    state.move_cursor(1, &members);
    let (rows, hits) = render(
        FleetTable {
            names: &members,
            ids: &[],
            rows: &rows,
            restart_badges: &restart_badges,
            state: &mut state,
            selected: Some(0),
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
    assert!(rows[3].starts_with("[x]   carol"), "selected row: {rows:?}");
    assert!(rows[3].contains("offline"), "no status row yet: {rows:?}");
    assert_eq!(
        rows[5].trim_end(),
        "3/3 shown · selected 1 of 3 · BOT alice",
        "{rows:?}"
    );
    assert_eq!(hits.rows.y, 1, "rows start under the column header");
    assert_eq!(hits.first, 0);
}

#[test]
fn a_long_fleet_scrolls_to_keep_the_cursor_visible() {
    let members: Vec<String> = (0..100).map(|i| format!("bot{i:03}")).collect();
    let mut state = FleetState::default();
    state.sync_with_ids(&members, &[], &[]);
    state.cursor_to(60, &members);
    let (rows, hits) = render(
        FleetTable {
            names: &members,
            ids: &[],
            rows: &[],
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
