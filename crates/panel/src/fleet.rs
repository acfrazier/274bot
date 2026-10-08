//! Fleet window: shared identity-keyed marks, the action bar over the marked
//! rows (Start / Stop, Assign, Assign & restart, log in / out, group walk) and
//! toggleable status columns.
//!
//! The window is deliberately independent of the Grid/wall surfaces.  When it
//! is closed, [`window`] returns before touching projections or formatting,
//! and a hidden column reads nothing from the host.

use std::cell::RefCell;

use dear_imgui_rs::{Condition, TableColumnFlags, TableFlags, Ui, WindowFlags};
use frontend_core::progress::{self, NONE};
use frontend_core::{FleetRow, ProfileIdentity, Scripts};

use crate::fleet_columns::{self, FleetColumn};
use crate::session::Session;
use crate::theme::{scale_px, scale_size, WARN};

const RESTART_POPUP: &str = "##fleet-restart-confirm";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FleetSort {
    Name,
    Status,
}

thread_local! {
    /// One reused buffer for every formatted cell of the open window.
    static CELL: RefCell<String> = const { RefCell::new(String::new()) };
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
    let card_tip = |action: &str| {
        if !has_card {
            "select a card in Scripts first".to_string()
        } else if count == 0 {
            "mark at least one profile".to_string()
        } else {
            action.to_string()
        }
    };
    {
        let _disabled = (count == 0 || !has_card).then(|| ui.begin_disabled());
        if ui.button(format!("Start selected card on {count} marked bots")) {
            session.fleet_start_selected();
        }
        ui.set_item_tooltip(card_tip("start the selected card on every marked profile"));
    }
    ui.same_line();
    {
        let _disabled = (count == 0).then(|| ui.begin_disabled());
        if ui.button(format!("Stop {count} marked bots")) {
            session.fleet_stop_selected();
        }
        ui.set_item_tooltip("stop every marked profile; ineligible rows are reported");
    }
    {
        let _disabled = (count == 0 || !has_card).then(|| ui.begin_disabled());
        if ui.button("Assign selected card") {
            session.fleet_assign_selected();
        }
        ui.set_item_tooltip(card_tip(
            "save the selected card on every marked profile without starting it, \
             so each bot's settings can be adjusted first; running bots are skipped",
        ));
        ui.same_line();
        if ui.button("Assign & restart...") {
            session.fleet_restart_confirm = true;
            ui.open_popup(RESTART_POPUP);
        }
        ui.set_item_tooltip(card_tip(
            "save the selected card and start it on every marked profile; \
             running scripts are stopped first (a confirmation names them)",
        ));
    }
    restart_confirm(ui, session);
    {
        let _disabled = (count == 0).then(|| ui.begin_disabled());
        if ui.button(format!("Log in {count}")) {
            session.fleet_login_selected();
        }
        ui.set_item_tooltip("log in every marked profile, loading the ones not loaded yet");
        ui.same_line();
        if ui.button(format!("Log out {count}")) {
            session.fleet_logout_selected();
        }
        ui.set_item_tooltip("log out every marked profile that is logged in");
        ui.same_line();
        if ui.button(format!("Walk {count} to...")) {
            session.open_walkto_for_marked();
        }
        ui.set_item_tooltip(
            "open the WalkTo picker for the marked bots: pick a tile and every bot \
             walks there from where it stands",
        );
    }
    apply_settings_row(ui, session, count);
    if let Some(report) = session.fleet_report.as_deref() {
        ui.text_wrapped(report);
    }
}
/// Copy the focused bot's parameters for the selected card to the marked
/// bots on that card: prepare, then the frozen scope names them and waits
/// for Apply (the script window's Apply to all, narrowed to the marks).
fn apply_settings_row(ui: &Ui, session: &mut Session, count: usize) {
    if let Some(scope) = session.scripts.prepared_settings_sync() {
        ui.text_wrapped(scope.prompt());
        if ui.button("Apply##fleet-apply") {
            session.apply_settings_sync();
        }
        ui.same_line();
        if ui.button("Cancel##fleet-apply") {
            session.scripts.cancel_settings_sync();
        }
        return;
    }
    let ready = count > 0 && session.script_sel.is_some() && session.focused_name().is_some();
    let _disabled = (!ready).then(|| ui.begin_disabled());
    if ui.button("Apply focused bot's settings to marked...") {
        session.fleet_prepare_apply_settings();
    }
    ui.set_item_tooltip(if ready {
        "copy the focused bot's parameters for the selected card to the marked bots \
         on that card; a confirmation names them first"
    } else {
        "mark bots, select a card in Scripts and focus the bot whose settings to copy"
    });
    if let Some(report) = session.scripts.last_settings_sync() {
        ui.text_wrapped(report.summary());
    }
}

/// The Assign & restart confirmation: it names every bot whose running
/// script the restart stops.
fn restart_confirm(ui: &Ui, session: &mut Session) {
    if !session.fleet_restart_confirm {
        return;
    }
    let mut decision = None;
    ui.popup(RESTART_POPUP, || {
        let scope = session.fleet_restart_scope();
        if scope.interrupted.is_empty() && scope.starting.is_empty() {
            ui.text_wrapped("No loaded marked bot to start the card on.");
        } else {
            if !scope.interrupted.is_empty() {
                ui.text_wrapped(format!(
                    "Stop and restart {} running {}: {}",
                    scope.interrupted.len(),
                    if scope.interrupted.len() == 1 {
                        "bot"
                    } else {
                        "bots"
                    },
                    scope.interrupted.join(", ")
                ));
            }
            if !scope.starting.is_empty() {
                ui.text_wrapped(format!(
                    "Start the card on {} idle {}: {}",
                    scope.starting.len(),
                    if scope.starting.len() == 1 {
                        "bot"
                    } else {
                        "bots"
                    },
                    scope.starting.join(", ")
                ));
            }
        }
        if ui.button("Assign & restart") {
            decision = Some(true);
            ui.close_current_popup();
        }
        ui.same_line();
        if ui.button("Cancel##fleet-restart") {
            decision = Some(false);
            ui.close_current_popup();
        }
    });
    match decision {
        Some(true) => session.fleet_assign_restart_selected(),
        Some(false) => session.fleet_restart_confirm = false,
        // Dismissed by a click outside the popup.
        None if !ui.is_popup_open(RESTART_POPUP) => session.fleet_restart_confirm = false,
        None => {}
    }
}

/// The checkboxes that show and hide the status columns. The choice is saved
/// in the panel preferences without touching any other key.
fn column_toggles(ui: &Ui, session: &mut Session) {
    let mut changed = false;
    for (i, column) in FleetColumn::ALL.into_iter().enumerate() {
        let mut on = fleet_columns::visible(&session.ui.fleet_columns, column);
        if ui.checkbox(column.title(), &mut on) {
            fleet_columns::set_visible(&mut session.ui.fleet_columns, column, on);
            changed = true;
        }
        if i + 1 < FleetColumn::ALL.len() {
            ui.same_line();
        }
    }
    if changed && session.persist_ui {
        crate::ui_state::save(&session.ui);
    }
}

fn cell(ui: &Ui, write: impl FnOnce(&mut String)) {
    CELL.with_borrow_mut(|buffer| {
        buffer.clear();
        write(buffer);
        ui.text(&*buffer);
    });
}

fn table(ui: &Ui, session: &mut Session, rows: &[FleetRow]) {
    let visible = sorted_rows(rows, &session.fleet_filter, session.fleet_sort);
    let columns: Vec<FleetColumn> = FleetColumn::ALL
        .into_iter()
        .filter(|column| fleet_columns::visible(&session.ui.fleet_columns, *column))
        .collect();
    let needs_progress = columns.iter().any(|column| column.needs_progress());
    let needs_last_line = columns.contains(&FleetColumn::LastLog);
    let table_id = format!("##fleet-table-{}", columns.len());
    if let Some(_table) = ui.begin_table_with_sizing(
        &table_id,
        2 + columns.len(),
        TableFlags::ROW_BG
            | TableFlags::BORDERS_INNER_H
            | TableFlags::SCROLL_Y
            | TableFlags::SCROLL_X
            | TableFlags::RESIZABLE,
        [
            0.0,
            (ui.content_region_avail()[1] - scale_px(ui, 70.0)).max(scale_px(ui, 80.0)),
        ],
        0.0,
    ) {
        ui.table_setup_column("Mark", TableColumnFlags::NONE, None, None);
        ui.table_setup_column("Profile", TableColumnFlags::NONE, None, None);
        for column in &columns {
            match column.init_width() {
                Some(width) => ui.table_setup_column_fixed_width(
                    column.title(),
                    TableColumnFlags::NONE,
                    scale_px(ui, width),
                    None,
                ),
                None => ui.table_setup_column_stretch_weight(
                    column.title(),
                    TableColumnFlags::NONE,
                    1.0,
                    None,
                ),
            }
        }
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
            if session.scripts.has_restart_badge(&row.name) {
                ui.same_line();
                ui.text_colored(WARN, "[restart]");
                ui.set_item_tooltip("saved settings need a restart to apply");
            }
            let progress = if needs_progress {
                session
                    .core
                    .play()
                    .and_then(|play| play.script_progress(&row.name))
            } else {
                None
            };
            for column in &columns {
                ui.table_next_column();
                match column {
                    FleetColumn::State => cell(ui, |out| row.write_world_login(out)),
                    FleetColumn::Card => cell(ui, |out| {
                        match Scripts::assignment(&session.core, &row.name) {
                            Some(assignment) => out.push_str(&assignment.display_name),
                            None => out.push_str(NONE),
                        };
                    }),
                    FleetColumn::Run => {
                        ui.text(frontend_core::views::run_state_label(row.script));
                        if ui.is_item_hovered() {
                            if let Some(error) = session
                                .core
                                .play()
                                .and_then(|play| play.script_last_error(&row.name))
                            {
                                ui.set_item_tooltip(&error);
                            }
                        }
                    }
                    FleetColumn::Runtime => cell(ui, |out| {
                        out.push_str(&progress::runtime_label(progress.as_ref()))
                    }),
                    FleetColumn::LastLog => cell(ui, |out| {
                        if needs_last_line
                            && !frontend_core::log::global().slot_last_message(&row.name, out)
                        {
                            out.push_str(NONE);
                        }
                    }),
                    FleetColumn::Idle => cell(ui, |out| {
                        out.push_str(&progress::idle_label(progress.as_ref()))
                    }),
                    FleetColumn::Levels => {
                        cell(ui, |out| {
                            out.push_str(&progress::levels_label(progress.as_ref()))
                        });
                        if ui.is_item_hovered() {
                            if let Some(detail) = progress
                                .as_ref()
                                .map(progress::levels_detail)
                                .filter(|detail| !detail.is_empty())
                            {
                                ui.set_item_tooltip(&detail);
                            }
                        }
                    }
                }
            }
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
        .size(scale_size(ui, [760.0, 460.0]), Condition::FirstUseEver)
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
            column_toggles(ui, session);
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

    /// The open window survives frames while columns come and go (the table
    /// is rebuilt with a different column count) and with a marked row:
    /// Dear ImGui aborts the process on a table it cannot reconcile.
    #[test]
    fn the_open_window_draws_across_column_changes() {
        let _guard = crate::test_support::imgui_context_guard();
        let dir = crate::test_support::TestDir::new("fleet-window-columns");
        let mut session = crate::session::Session::new();
        let mut vault = vault::Vault::create(&dir.join("vault"), "test-passphrase-01").unwrap();
        for (i, name) in ["alice", "bob"].into_iter().enumerate() {
            vault
                .upsert(vault::Profile {
                    username: name.into(),
                    password: "pw".into(),
                    uid: 1 + i as i32,
                    settings: vault::ProfileSettings::default(),
                })
                .unwrap();
        }
        session.core.set_vault(Some(vault));
        session.fleet_open = true;
        let bob = session.core.profile_identity("bob").unwrap();
        session.fleet_selection.set(bob, true);
        let mut ctx = dear_imgui_rs::Context::create();
        let mut frame = |session: &mut crate::session::Session| {
            ctx.prepare_frame(
                dear_imgui_rs::FramePrepareOptions::new([900.0, 700.0], 1.0 / 60.0)
                    .renderer_has_textures(),
            );
            {
                let ui = ctx.frame();
                super::window(ui, session);
            }
            ctx.render();
        };
        for _ in 0..3 {
            frame(&mut session);
        }
        for column in crate::fleet_columns::FleetColumn::ALL {
            crate::fleet_columns::set_visible(&mut session.ui.fleet_columns, column, true);
        }
        for _ in 0..3 {
            frame(&mut session);
        }
        for column in crate::fleet_columns::FleetColumn::ALL {
            crate::fleet_columns::set_visible(&mut session.ui.fleet_columns, column, false);
        }
        for _ in 0..3 {
            frame(&mut session);
        }
        assert!(session.fleet_open, "drawing never closes the window");
    }

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
