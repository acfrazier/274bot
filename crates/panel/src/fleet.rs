//! Minimal Fleet window: shared identity-keyed marks plus Start/Stop actions.
//!
//! The window is deliberately independent of the Grid/wall surfaces.  When it
//! is closed, [`window`] returns before touching projections or formatting.

use dear_imgui_rs::{Condition, TableColumnFlags, TableFlags, Ui, WindowFlags};
use frontend_core::{FleetRow, ProfileIdentity};

use crate::session::Session;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FleetSort {
    Name,
    Status,
}

fn identity(session: &Session, name: &str) -> ProfileIdentity {
    session
        .core
        .profile_identity(name)
        .unwrap_or_else(|| ProfileIdentity::synthetic(name))
}

fn contains_ci(value: &str, query: &str) -> bool {
    query.is_empty()
        || value
            .to_ascii_lowercase()
            .contains(&query.to_ascii_lowercase())
}

fn row_matches(row: &FleetRow, filter: &str) -> bool {
    contains_ci(&row.name, filter) || contains_ci(&row.brief, filter)
}

fn sorted_rows<'a>(rows: &'a [FleetRow], filter: &str, sort: FleetSort) -> Vec<&'a FleetRow> {
    let mut visible = rows
        .iter()
        .filter(|row| row_matches(row, filter))
        .collect::<Vec<_>>();
    visible.sort_by(|left, right| match sort {
        FleetSort::Name => left.name.cmp(&right.name),
        FleetSort::Status => left
            .phase
            .label()
            .cmp(right.phase.label())
            .then_with(|| left.name.cmp(&right.name)),
    });
    visible
}

fn profile_rows(session: &Session) -> Vec<FleetRow> {
    let projected = session.core.fleet_view().rows();
    session
        .core
        .vault()
        .map(|vault| {
            vault
                .profiles()
                .map(|profile| {
                    projected
                        .iter()
                        .find(|row| row.name == profile.username)
                        .cloned()
                        .unwrap_or_else(|| FleetRow {
                            name: profile.username.clone(),
                            ..FleetRow::default()
                        })
                })
                .collect()
        })
        .unwrap_or_else(|| projected.to_vec())
}

fn action_bar(ui: &Ui, session: &mut Session) {
    let count = session.fleet_selection.len();
    let has_card = session.script_sel.is_some();
    let start_enabled = count > 0 && has_card;
    {
        let _disabled = (!start_enabled).then(|| ui.begin_disabled());
        if ui.button(format!("Start selected card on {count} marked bots")) {
            session.fleet_start_selected();
        }
        ui.set_item_tooltip(if !has_card {
            "select a card in Scripts first"
        } else if count == 0 {
            "mark at least one profile"
        } else {
            "start the selected card on every marked profile"
        });
    }
    ui.same_line();
    {
        let _disabled = (count == 0).then(|| ui.begin_disabled());
        if ui.button(format!("Stop {count} marked bots")) {
            session.fleet_stop_selected();
        }
        ui.set_item_tooltip("stop every marked profile; ineligible rows are reported");
    }
    if let Some(report) = session.fleet_report.as_deref() {
        ui.text_wrapped(report);
    }
}

fn table(ui: &Ui, session: &mut Session, rows: &[FleetRow]) {
    let visible = sorted_rows(rows, &session.fleet_filter, session.fleet_sort);
    if let Some(_table) = ui.begin_table_with_sizing(
        "##fleet-table",
        4,
        TableFlags::ROW_BG | TableFlags::BORDERS_INNER_H | TableFlags::SCROLL_Y,
        [0.0, (ui.content_region_avail()[1] - 70.0).max(80.0)],
        0.0,
    ) {
        ui.table_setup_column("Mark", TableColumnFlags::NONE, None, None);
        ui.table_setup_column("Profile", TableColumnFlags::NONE, None, None);
        ui.table_setup_column("Status", TableColumnFlags::NONE, None, None);
        ui.table_setup_column("Script", TableColumnFlags::NONE, None, None);
        ui.table_headers_row();
        for row in visible {
            let marked_id = identity(session, &row.name);
            let mut marked = session.fleet_selection.contains(marked_id);
            ui.table_next_row();
            ui.table_next_column();
            if ui.checkbox(format!("##fleet-mark-{}", marked_id.raw()), &mut marked) {
                session.fleet_selection.set(marked_id, marked);
            }
            ui.table_next_column();
            ui.text(&row.name);
            ui.table_next_column();
            ui.text(&row.brief);
            ui.table_next_column();
            ui.text(frontend_core::views::run_state_label(row.script));
        }
    }
}

/// Draw the Fleet window only while it is open.  No projection, sorting or
/// string formatting occurs on frames where the operator has closed it.
pub fn window(ui: &Ui, session: &mut Session) {
    if !session.fleet_open {
        return;
    }
    let mut open = true;
    ui.window("Fleet###fleet-window")
        .opened(&mut open)
        .flags(WindowFlags::NO_COLLAPSE)
        .size([560.0, 420.0], Condition::FirstUseEver)
        .build(|| {
            ui.input_text("Filter", &mut session.fleet_filter)
                .hint("name or status")
                .build();
            ui.same_line();
            if ui.small_button(match session.fleet_sort {
                FleetSort::Name => "Sort: name",
                FleetSort::Status => "Sort: status",
            }) {
                session.fleet_sort = match session.fleet_sort {
                    FleetSort::Name => FleetSort::Status,
                    FleetSort::Status => FleetSort::Name,
                };
            }
            ui.separator();
            action_bar(ui, session);
            ui.separator();
            let rows = profile_rows(session);
            table(ui, session, &rows);
        });
    session.fleet_open = open;
}

#[cfg(test)]
mod tests {
    use super::{row_matches, sorted_rows, FleetSort};
    use frontend_core::{FleetRow, Phase};

    #[test]
    fn filtering_and_sorting_keep_one_row_per_projection() {
        let rows = vec![
            FleetRow::fixture("bob", Phase::Ready, Some(301), None),
            FleetRow::fixture("alice", Phase::Preparing, Some(302), None),
        ];
        assert!(row_matches(&rows[0], "idle"));
        assert_eq!(sorted_rows(&rows, "", FleetSort::Name)[0].name, "alice");
        assert_eq!(sorted_rows(&rows, "starting", FleetSort::Status).len(), 1);
    }
}
