//! Fleet window: shared identity-keyed marks, the action bar over the marked
//! rows (Start / Stop, Assign, Assign & restart, log in / out, group walk) and
//! toggleable status columns.
//!
//! The window is deliberately independent of the Grid/wall surfaces.  When it
//! is closed, [`window`] returns before touching projections or formatting,
//! and a hidden column reads nothing from the host.

use std::cell::RefCell;

use dear_imgui_rs::{
    Condition, TableColumnFlags, TableFlags, TableOptions, TableSizingPolicy, Ui, WindowFlags,
};
use frontend_core::progress::{self, NONE};
use frontend_core::{FleetRow, ProfileIdentity, Scripts};

use crate::fleet_columns::{self, FleetColumn};
use crate::session::Session;
use crate::theme::{scale_px, scale_size, WARN};

const RESTART_POPUP: &str = "##fleet-restart-confirm";

/// The Fleet window's smallest size at scale 1: wide enough for its longest
/// button, so nothing runs past the edge however narrow it is dragged.
pub(crate) const MIN_WIDTH: f32 = 360.0;
const MIN_HEIGHT: f32 = 240.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FleetSort {
    Name,
    Status,
}

impl FleetSort {
    pub const ALL: [FleetSort; 2] = [Self::Name, Self::Status];

    /// The drop-down entry.
    pub fn label(self) -> &'static str {
        match self {
            Self::Name => "Name",
            Self::Status => "Status",
        }
    }
}

thread_local! {
    /// One reused buffer for every formatted cell of the open window.
    static CELL: RefCell<String> = const { RefCell::new(String::new()) };
}

/// What a test sees of the Fleet window after one frame.
#[cfg(test)]
#[derive(Debug, Default, Clone)]
pub(crate) struct FleetLayout {
    /// Left and right edge of the window's content region, in screen space.
    pub content: [f32; 2],
    /// How far the window scrolls sideways; 0 when nothing runs past its edge.
    pub scroll_max_x: f32,
    /// The Sort drop-down's `[min_x, min_y, max_x, max_y]`.
    pub sort_combo: [f32; 4],
    /// Each entry of the open Sort drop-down.
    pub sort_choices: Vec<[f32; 4]>,
    /// Left and right edge of each table cell drawn, row by row.
    pub cells: Vec<[f32; 2]>,
}

#[cfg(test)]
std::thread_local! {
    static LAYOUT: RefCell<FleetLayout> = RefCell::new(FleetLayout::default());
}

#[cfg(test)]
fn record_content_edges(ui: &Ui) {
    let left = ui.cursor_screen_pos()[0];
    LAYOUT.with_borrow_mut(|layout| layout.content = [left, left + ui.content_region_avail()[0]]);
}

#[cfg(test)]
fn record_sort_combo(ui: &Ui) {
    let (min, max) = (ui.item_rect_min(), ui.item_rect_max());
    LAYOUT.with_borrow_mut(|layout| layout.sort_combo = [min[0], min[1], max[0], max[1]]);
}

#[cfg(test)]
fn record_sort_choice(ui: &Ui) {
    let (min, max) = (ui.item_rect_min(), ui.item_rect_max());
    LAYOUT.with_borrow_mut(|layout| layout.sort_choices.push([min[0], min[1], max[0], max[1]]));
}

#[cfg(test)]
fn record_cell(ui: &Ui) {
    let left = ui.cursor_screen_pos()[0];
    LAYOUT.with_borrow_mut(|layout| {
        layout
            .cells
            .push([left, left + ui.content_region_avail()[0]])
    });
}

#[cfg(test)]
fn record_window_layout(ui: &Ui) {
    LAYOUT.with_borrow_mut(|layout| layout.scroll_max_x = ui.scroll_max_x());
}

/// Draw the Fleet window for a test and return what it laid out.
#[cfg(test)]
pub(crate) fn draw_test_window(ui: &Ui, session: &mut Session) -> FleetLayout {
    LAYOUT.with_borrow_mut(|layout| *layout = FleetLayout::default());
    window(ui, session);
    LAYOUT.with_borrow_mut(std::mem::take)
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

const ASSIGN_LABEL: &str = "Assign selected card";
const ASSIGN_RESTART_LABEL: &str = "Assign & restart";
const APPLY_SETTINGS_LABEL: &str = "Apply focused bot's settings to marked";

/// The action bar's count-bearing button labels. None ends in an ellipsis: a
/// button that opens a window or a confirmation still names the action plainly.
struct ActionLabels {
    start: String,
    stop: String,
    log_in: String,
    log_out: String,
    walk: String,
}

impl ActionLabels {
    fn new(count: usize) -> Self {
        Self {
            start: format!("Start selected card on {count} marked bots"),
            stop: format!("Stop {count} marked bots"),
            log_in: format!("Log in {count}"),
            log_out: format!("Log out {count}"),
            walk: format!("Walk {count} marked bots"),
        }
    }

    /// Every label of the bar, including the fixed ones.
    #[cfg(test)]
    fn all(&self) -> [&str; 8] {
        [
            &self.start,
            &self.stop,
            ASSIGN_LABEL,
            ASSIGN_RESTART_LABEL,
            &self.log_in,
            &self.log_out,
            &self.walk,
            APPLY_SETTINGS_LABEL,
        ]
    }
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
    let ActionLabels {
        start: start_label,
        stop: stop_label,
        log_in: log_in_label,
        log_out: log_out_label,
        walk: walk_label,
    } = ActionLabels::new(count);
    {
        let _disabled = (count == 0 || !has_card).then(|| ui.begin_disabled());
        if ui.button(start_label) {
            session.fleet_start_selected();
        }
        ui.set_item_tooltip(card_tip("start the selected card on every marked profile"));
    }
    continue_line_if_fits(ui, button_width(ui, &stop_label));
    {
        let _disabled = (count == 0).then(|| ui.begin_disabled());
        if ui.button(stop_label) {
            session.fleet_stop_selected();
        }
        ui.set_item_tooltip("stop every marked profile; ineligible rows are reported");
    }
    {
        let _disabled = (count == 0 || !has_card).then(|| ui.begin_disabled());
        if ui.button(ASSIGN_LABEL) {
            session.fleet_assign_selected();
        }
        ui.set_item_tooltip(card_tip(
            "save the selected card on every marked profile without starting it, \
             so each bot's settings can be adjusted first; running bots are skipped",
        ));
        continue_line_if_fits(ui, button_width(ui, ASSIGN_RESTART_LABEL));
        if ui.button(ASSIGN_RESTART_LABEL) {
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
        if ui.button(log_in_label) {
            session.fleet_login_selected();
        }
        ui.set_item_tooltip("log in every marked profile, loading the ones not loaded yet");
        continue_line_if_fits(ui, button_width(ui, &log_out_label));
        if ui.button(log_out_label) {
            session.fleet_logout_selected();
        }
        ui.set_item_tooltip("log out every marked profile that is logged in");
        continue_line_if_fits(ui, button_width(ui, &walk_label));
        if ui.button(walk_label) {
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
    if ui.button(APPLY_SETTINGS_LABEL) {
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
        if i > 0 {
            let width = ui.frame_height()
                + ui.clone_style().item_inner_spacing()[0]
                + label_width(ui, column.title());
            continue_line_if_fits(ui, width);
        }
        if ui.checkbox(column.title(), &mut on) {
            fleet_columns::set_visible(&mut session.ui.fleet_columns, column, on);
            changed = true;
        }
    }
    if changed && session.persist_ui {
        crate::ui_state::save(&session.ui);
    }
}

/// The "Sort:" drop-down, full width like the Danger routing one.
fn sort_combo(ui: &Ui, session: &mut Session) {
    ui.text("Sort:");
    ui.set_next_item_width(-1.0);
    let combo = ui.begin_combo("##fleet-sort", session.fleet_sort.label());
    #[cfg(test)]
    record_sort_combo(ui);
    if let Some(_open) = combo {
        for sort in FleetSort::ALL {
            let selected = sort == session.fleet_sort;
            let picked = ui
                .selectable_config(sort.label())
                .selected(selected)
                .build();
            #[cfg(test)]
            record_sort_choice(ui);
            if picked {
                session.fleet_sort = sort;
            }
            if selected {
                ui.set_item_default_focus();
            }
        }
    }
}

/// Width of a label's visible text (what follows `##` is only an id).
fn label_width(ui: &Ui, label: &str) -> f32 {
    let visible = label.split("##").next().unwrap_or(label);
    ui.current_font()
        .calc_text_size(ui.current_font_size(), f32::MAX, 0.0, visible)[0]
}

/// A button's width: its text and the frame padding either side.
fn button_width(ui: &Ui, label: &str) -> f32 {
    label_width(ui, label) + 2.0 * ui.clone_style().frame_padding()[0]
}

/// Continue the current line with an item `width` wide when it still fits
/// inside the window; otherwise leave the cursor so the item starts the next
/// line. Call straight after the previous item.
fn continue_line_if_fits(ui: &Ui, width: f32) {
    let spacing = ui.clone_style().item_spacing()[0];
    let right = ui.cursor_screen_pos()[0] + ui.content_region_avail()[0];
    if ui.item_rect_max()[0] + spacing + width <= right + 0.5 {
        ui.same_line();
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
    // Every column stretches with the window, so the table is never wider
    // than the window and has no horizontal scroll; only the Mark column keeps
    // a fixed width. The weights follow the width each column wants.
    if let Some(_table) = ui.begin_table_with_sizing(
        &table_id,
        2 + columns.len(),
        TableOptions::new()
            .flags(
                TableFlags::ROW_BG
                    | TableFlags::BORDERS_INNER_H
                    | TableFlags::SCROLL_Y
                    | TableFlags::RESIZABLE,
            )
            .sizing_policy(TableSizingPolicy::StretchProp),
        [
            0.0,
            (ui.content_region_avail()[1] - scale_px(ui, 70.0)).max(scale_px(ui, 80.0)),
        ],
        0.0,
    ) {
        ui.table_setup_column_fixed_width(
            "Mark",
            TableColumnFlags::NONE,
            ui.frame_height() + 2.0 * ui.clone_style().cell_padding()[0],
            None,
        );
        ui.table_setup_column_stretch_weight(
            "Profile",
            TableColumnFlags::NONE,
            fleet_columns::PROFILE_WEIGHT,
            None,
        );
        for column in &columns {
            ui.table_setup_column_stretch_weight(
                column.title(),
                TableColumnFlags::NONE,
                column.weight(),
                None,
            );
        }
        ui.table_headers_row();
        for row in visible {
            let marked_id = identity(session, &row.name);
            let mut marked = session.fleet_selection.contains(marked_id);
            ui.table_next_row();
            ui.table_next_column();
            #[cfg(test)]
            record_cell(ui);
            if ui.checkbox(format!("##fleet-mark-{}", marked_id.raw()), &mut marked) {
                session.fleet_selection.set(marked_id, marked);
            }
            ui.table_next_column();
            #[cfg(test)]
            record_cell(ui);
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
                #[cfg(test)]
                record_cell(ui);
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
    crate::app::focus_if_requested(ui, session, crate::session::PanelWindow::Fleet);
    ui.window(crate::session::PanelWindow::Fleet.title())
        .opened(&mut open)
        .flags(WindowFlags::NO_COLLAPSE)
        .size(scale_size(ui, [760.0, 460.0]), Condition::FirstUseEver)
        .size_constraints(
            scale_size(ui, [MIN_WIDTH, MIN_HEIGHT]),
            [f32::MAX, f32::MAX],
        )
        .build(|| {
            #[cfg(test)]
            record_content_edges(ui);
            ui.input_text("Filter", &mut session.fleet_filter)
                .hint("name or status")
                .build();
            sort_combo(ui, session);
            ui.separator();
            action_bar(ui, session);
            column_toggles(ui, session);
            ui.separator();
            let rows = profile_rows(session);
            table(ui, session, &rows);
            #[cfg(test)]
            record_window_layout(ui);
        });
    session.fleet_open = open;
}

#[cfg(test)]
mod tests {
    use super::{row_matches, sorted_rows, ActionLabels, FleetSort};
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
        let mut ctx = crate::test_support::imgui_context();
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

    /// No button of the action bar ends in an ellipsis, whatever the count.
    #[test]
    fn action_button_labels_have_no_ellipsis() {
        for count in [0, 1, 12] {
            for label in ActionLabels::new(count).all() {
                assert!(
                    !label.contains('\u{2026}') && !label.contains("..."),
                    "{label:?}"
                );
            }
        }
    }
}
