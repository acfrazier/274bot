//! Fleet window: the wide table over the shared fleet model (every profile,
//! loaded or not, in the model's order and filter), its identity-keyed marks
//! and toggleable status columns, under the shared fleet toolbar
//! ([`crate::fleet_toolbar`]: card, marks, filter and sort, the nine
//! actions, the plan confirmation and the result). A click on a loaded
//! bot's name focuses it; its check box only marks it. A loaded bot's
//! World / login cell opens the shared world picker (`crate::world_picker`).
//!
//! The window draws the same rows, order, filter and marks as the MultiBox
//! rail and the Grid: all three read [`Session::fleet_model`]. Every decision
//! (the card, the targets, the skips, whether to ask first, the prompt and
//! the result text) is the core's; this module stages and submits through
//! the [`Session`] plan flow. When the window is closed, [`window`] returns
//! before touching projections or formatting, and a hidden column reads
//! nothing from the host.

use std::cell::RefCell;

use dear_imgui_rs::{
    Condition, DragDropTargetFlags, SelectableFlags, StyleColor, TableColumnFlags, TableFlags,
    TableHoveredColumn, TableOptions, TableRowFlags, TableSizingPolicy, Ui, WindowFlags,
};
use frontend_core::progress::{self, NONE};
use frontend_core::{
    FleetModel, FleetOrder, FleetRow, MarkOp, OperatorCommand, Placement, ProfileIdentity,
    ReorderOp, Scripts, Targets,
};

use crate::fleet_columns::{self, FleetColumn};
use crate::fleet_toolbar::{self, label_width, tri_state_box, tri_state_op, tri_state_tooltip};
use crate::session::Session;
use crate::theme::{scale_px, scale_size, WARN};

/// The Fleet window's smallest size at scale 1: wide enough for its longest
/// button, so nothing runs past the edge however narrow it is dragged.
pub(crate) const MIN_WIDTH: f32 = 360.0;
const MIN_HEIGHT: f32 = 240.0;

thread_local! {
    /// One reused buffer for every formatted cell of the open window.
    static CELL: RefCell<String> = const { RefCell::new(String::new()) };
}

/// The drag-and-drop payload type of a profile dragged to reorder the saved
/// Load order. The payload is the profile's [`ProfileIdentity::raw`].
const PROFILE_DND: &str = "274bot-profile";

/// Make the last item a drag source for the profile `identity`; the drag
/// tooltip shows `name`. Returns whether this item is being dragged now.
/// Call it only while the fleet shows Load order.
pub(crate) fn profile_drag_source(ui: &Ui, identity: ProfileIdentity, name: &str) -> bool {
    match ui
        .drag_drop_source_config(PROFILE_DND)
        .begin_payload(identity.raw())
    {
        Some(_tip) => {
            ui.text(name);
            true
        }
        None => false,
    }
}

/// Make the last item a drop target for a dragged profile. Returns the
/// dropped profile's [`ProfileIdentity::raw`] on the frame the button is
/// released over it. Call it only while the fleet shows Load order.
pub(crate) fn profile_drop_target(ui: &Ui) -> Option<u64> {
    let target = ui.drag_drop_target()?;
    match target.accept_payload::<u64, _>(PROFILE_DND, DragDropTargetFlags::NONE) {
        Some(Ok(payload)) if payload.delivery => Some(payload.data),
        _ => None,
    }
}

/// The move a drop of profile `moving` (a [`ProfileIdentity::raw`]) onto the
/// model row `target` asks for: after the target when the moving profile is
/// above it in the shown rows, else before it. `None` when nothing would
/// move (the same row, or a profile the filter now hides).
pub(crate) fn drop_op(model: &FleetModel, moving: u64, target: usize) -> Option<ReorderOp> {
    let moving_index = model
        .identities()
        .iter()
        .position(|identity| identity.raw() == moving)?;
    if moving_index == target {
        return None;
    }
    let from = model.shown().iter().position(|&i| i == moving_index)?;
    let to = model.shown().iter().position(|&i| i == target)?;
    let target = model.identity(target)?;
    let placement = if from < to {
        Placement::After(target)
    } else {
        Placement::Before(target)
    };
    Some(ReorderOp::Move {
        moving: model.identity(moving_index)?,
        to: placement,
    })
}

/// What a test sees of a fleet surface (the Fleet window, or a toolbar in
/// the rail or the strip) after one frame.
#[cfg(test)]
#[derive(Debug, Default, Clone)]
pub(crate) struct FleetLayout {
    /// The site of the last toolbar drawn.
    pub site: Option<fleet_toolbar::Site>,
    /// Left and right edge of the toolbar's content region, in screen space.
    pub content: [f32; 2],
    /// How far the Fleet window scrolls sideways; 0 when nothing runs past
    /// its edge.
    pub scroll_max_x: f32,
    /// The fold header's `[min_x, min_y, max_x, max_y]` (rail and strip).
    pub fold_header: [f32; 4],
    /// The folded summary ("5 marked · <card>"), when the toolbar is folded.
    pub folded: Option<String>,
    /// The filter field.
    pub filter: [f32; 4],
    /// The Sort drop-down's `[min_x, min_y, max_x, max_y]`.
    pub sort_combo: [f32; 4],
    /// Each entry of the open Sort drop-down.
    pub sort_choices: Vec<[f32; 4]>,
    /// Left and right edge of each table cell drawn, row by row.
    pub cells: Vec<[f32; 2]>,
    /// The Profile name cell of each shown row, in row order, as
    /// `[min_x, min_y, max_x, max_y]`.
    pub names: Vec<[f32; 4]>,
    /// The Mark check box of each shown row, in row order: name and rect.
    pub mark_boxes: Vec<(String, [f32; 4])>,
    /// The Marks menu's Move marked to top entry, while the menu is open.
    pub move_to_top: [f32; 4],
    /// The card drop-down's `[min_x, min_y, max_x, max_y]`.
    pub card_combo: [f32; 4],
    /// Each entry of the open card drop-down: its text and rect.
    pub card_entries: Vec<(String, [f32; 4])>,
    /// The marks row's tri-state box.
    pub mark_box: [f32; 4],
    /// The Mark column header's tri-state box.
    pub header_mark_box: [f32; 4],
    /// The marks menu button.
    pub marks_button: [f32; 4],
    /// Each item of the open marks menu: its text and rect.
    pub marks_menu: Vec<(String, [f32; 4])>,
    /// The marks row's count text.
    pub marks_text: String,
    /// Each toolbar button, in toolbar order.
    pub actions: Vec<ActionRecord>,
    /// The waiting plan's prompt, Confirm and Cancel, when one is drawn.
    pub plan: Option<PlanRecord>,
    /// The result line's text and rect, when one is drawn.
    pub result: Option<(String, [f32; 4])>,
    /// Every toolbar item drawn, to check that none runs past the edge.
    pub items: Vec<[f32; 4]>,
    /// The World / login cell of each shown row that opens the world picker
    /// when clicked, in row order: the profile's name and the cell.
    pub world_cells: Vec<(String, [f32; 4])>,
    /// The open world picker's rows.
    pub world_picker: Vec<crate::world_picker::EntryRecord>,
}

/// One toolbar button as a test sees it.
#[cfg(test)]
#[derive(Debug, Clone)]
pub(crate) struct ActionRecord {
    pub action: fleet_toolbar::ToolbarAction,
    pub label: String,
    pub rect: [f32; 4],
    pub disabled: bool,
    /// The label's text and padding: the button is never narrower.
    pub text_width: f32,
    pub tooltip: &'static str,
}

/// The waiting plan row as a test sees it.
#[cfg(test)]
#[derive(Debug, Clone)]
pub(crate) struct PlanRecord {
    pub prompt: String,
    pub confirm: [f32; 4],
    pub cancel: [f32; 4],
}

#[cfg(test)]
std::thread_local! {
    static LAYOUT: RefCell<FleetLayout> = RefCell::new(FleetLayout::default());
    /// Whether [`record`] keeps what it is given (an allocation test turns
    /// it off, so the test's own records do not count).
    static RECORDING: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
}

/// Record into this thread's fleet layout (a test reads it back with
/// [`take_layout`]). Does nothing while recording is off.
#[cfg(test)]
pub(crate) fn record(write: impl FnOnce(&mut FleetLayout)) {
    if RECORDING.get() {
        LAYOUT.with_borrow_mut(write);
    }
}

/// Turn [`record`] on or off on this thread.
#[cfg(test)]
pub(crate) fn set_recording(on: bool) {
    RECORDING.set(on);
}

/// What the fleet surfaces recorded since the last call.
#[cfg(test)]
pub(crate) fn take_layout() -> FleetLayout {
    LAYOUT.with_borrow_mut(std::mem::take)
}

#[cfg(test)]
std::thread_local! {
    /// Plan rows drawn since the last [`take_plan_rows`].
    static PLAN_ROWS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Count one drawn plan row.
#[cfg(test)]
pub(crate) fn count_plan_row() {
    PLAN_ROWS.set(PLAN_ROWS.get() + 1);
}

/// How many plan rows were drawn since the last call.
#[cfg(test)]
pub(crate) fn take_plan_rows() -> usize {
    PLAN_ROWS.replace(0)
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

/// Record the rect of the name text that starts at `at`.
#[cfg(test)]
fn record_name(ui: &Ui, at: [f32; 2], name: &str) {
    let size = [label_width(ui, name), ui.current_font_size()];
    LAYOUT.with_borrow_mut(|layout| {
        layout
            .names
            .push([at[0], at[1], at[0] + size[0], at[1] + size[1]])
    });
}

#[cfg(test)]
fn record_window_layout(ui: &Ui) {
    LAYOUT.with_borrow_mut(|layout| layout.scroll_max_x = ui.scroll_max_x());
}

/// Draw the Fleet window for a test and return what it laid out. The shared
/// world picker is drawn after it, as `ui_frame` does.
#[cfg(test)]
pub(crate) fn draw_test_window(ui: &Ui, session: &mut Session) -> FleetLayout {
    take_layout();
    window(ui, session);
    crate::world_picker::popup(ui, session);
    LAYOUT.with_borrow_mut(|layout| {
        layout.world_picker = crate::world_picker::take_entries();
        std::mem::take(layout)
    })
}

/// The checkboxes that show and hide the status columns. The choice is saved
/// in the panel preferences without touching any other key.
fn column_toggles(ui: &Ui, session: &mut Session) {
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
            let value = (on != column.default_on()).then_some(serde_json::Value::Bool(on));
            let _ = session.persist_panel_member(&["fleet_columns", column.id()], value);
        }
    }
    crate::app::panel_member_feedback_line(ui, session, &["fleet_columns"]);
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

/// The World / login cell of a bot on a public world: [`cell`]'s text, but a
/// click opens the world picker. The `###` suffix keeps the cell's identity
/// while its text follows the phase. Returns whether it was clicked.
fn world_cell(ui: &Ui, row: &FleetRow) -> bool {
    let _id = ui.push_id(row.name());
    let clicked = CELL.with_borrow_mut(|buffer| {
        buffer.clear();
        row.write_world_login(buffer);
        buffer.push_str("###world");
        ui.selectable(&*buffer)
    });
    #[cfg(test)]
    record(|layout| {
        layout
            .world_cells
            .push((row.name().to_owned(), fleet_toolbar::item_rect(ui)));
    });
    ui.set_item_tooltip("click to switch world");
    clicked
}

/// The table over the model's shown rows. A mark, a name click (focus) or a
/// world click is submitted after the rows are drawn (the rows are borrowed
/// from the session's model).
fn table(ui: &Ui, session: &mut Session) {
    let columns: Vec<FleetColumn> = FleetColumn::ALL
        .into_iter()
        .filter(|column| fleet_columns::visible(&session.ui.fleet_columns, *column))
        .collect();
    let needs_progress = columns.iter().any(|column| column.needs_progress());
    let needs_last_line = columns.contains(&FleetColumn::LastLog);
    let table_id = format!("##fleet-table-{}", columns.len());
    let mut mark_click: Option<(usize, bool)> = None;
    let mut header_op: Option<MarkOp> = None;
    let mut dropped: Option<(u64, usize)> = None;
    let can_switch = session.can_switch_world();
    let mut world_click: Option<usize> = None;
    let mut name_click: Option<usize> = None;
    // Dragging reorders the saved Load order, so only that view offers it.
    let reorder = session.fleet_model.order() == FleetOrder::LoadOrder;
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
        // The header row is drawn by hand: the Mark column's header is the
        // tri-state box over the shown rows; the others are plain headers.
        ui.table_next_row_with_flags(TableRowFlags::HEADERS, ui.table_get_header_row_height());
        for column in 0..ui.table_get_column_count() {
            if !ui.table_set_column_index(column) {
                continue;
            }
            if column == 0 {
                let state = session.fleet_model.shown_marks();
                if tri_state_box(ui, "##fleet-mark-all-header", state) {
                    header_op = Some(tri_state_op(state));
                }
                #[cfg(test)]
                record(|layout| layout.header_mark_box = fleet_toolbar::item_rect(ui));
                ui.set_item_tooltip(tri_state_tooltip(state));
            } else {
                ui.table_header(ui.table_get_column_name(column));
            }
        }
        let model = &session.fleet_model;
        for (index, row) in model.shown_rows() {
            let marked_id = model.identities()[index];
            let mut marked = model.marks().contains(marked_id);
            ui.table_next_row();
            ui.table_next_column();
            #[cfg(test)]
            record_cell(ui);
            if ui.checkbox(format!("##fleet-mark-{}", marked_id.raw()), &mut marked) {
                mark_click = Some((index, marked));
            }
            #[cfg(test)]
            record(|layout| {
                layout
                    .mark_boxes
                    .push((row.name().to_owned(), fleet_toolbar::item_rect(ui)));
            });
            ui.table_next_column();
            #[cfg(test)]
            record_cell(ui);
            if reorder {
                // The name is the drag handle and the whole row the drop
                // target (a selectable spanning every column; the Mark
                // checkbox, submitted first, keeps its own clicks). A click
                // focuses the bot only inside the Profile column.
                #[cfg(test)]
                let name_at = ui.cursor_screen_pos();
                {
                    let _dim = (!row.loaded()).then(|| {
                        ui.push_style_color(
                            StyleColor::Text,
                            ui.style_color(StyleColor::TextDisabled),
                        )
                    });
                    let clicked = ui
                        .selectable_config(row.name())
                        .flags(SelectableFlags::SPAN_ALL_COLUMNS | SelectableFlags::ALLOW_OVERLAP)
                        .build();
                    let on_name = matches!(
                        ui.table_get_hovered_column(),
                        TableHoveredColumn::Column(column) if column.get() == 1
                    );
                    if clicked && on_name && row.loaded() {
                        name_click = Some(index);
                    }
                    if row.loaded() {
                        ui.set_item_tooltip("click to focus; drag to reorder");
                    }
                    profile_drag_source(ui, marked_id, row.name());
                    if let Some(moving) = profile_drop_target(ui) {
                        dropped = Some((moving, index));
                    }
                }
                #[cfg(test)]
                record_name(ui, name_at, row.name());
            } else if row.loaded() {
                #[cfg(test)]
                let name_at = ui.cursor_screen_pos();
                if ui.selectable(row.name()) {
                    name_click = Some(index);
                }
                ui.set_item_tooltip("click to focus");
                #[cfg(test)]
                record_name(ui, name_at, row.name());
            } else {
                ui.text_disabled(row.name());
                #[cfg(test)]
                record_name(ui, ui.item_rect_min(), row.name());
            }
            if session.scripts.has_restart_badge(row.name()) {
                ui.same_line();
                ui.text_colored(WARN, "[restart]");
                ui.set_item_tooltip("saved settings need a restart to apply");
            }
            let progress = if needs_progress && row.loaded() {
                session.core.script_progress(row.name())
            } else {
                None
            };
            for column in &columns {
                ui.table_next_column();
                #[cfg(test)]
                record_cell(ui);
                match column {
                    FleetColumn::State => {
                        if can_switch && row.loaded() && row.world().is_some() {
                            if world_cell(ui, row) {
                                world_click = Some(index);
                            }
                        } else {
                            cell(ui, |out| row.write_world_login(out));
                        }
                    }
                    FleetColumn::Card => cell(ui, |out| {
                        match Scripts::assignment_name(&session.core, row.name()) {
                            Some(name) => out.push_str(name),
                            None => out.push_str(NONE),
                        };
                    }),
                    FleetColumn::Run => {
                        ui.text(frontend_core::views::run_state_label(row.script()));
                        if ui.is_item_hovered() {
                            if let Some(error) = session.core.script_last_error(row.name()) {
                                ui.set_item_tooltip(&error);
                            }
                        }
                    }
                    FleetColumn::Runtime => cell(ui, |out| {
                        out.push_str(&progress::runtime_label(progress.as_ref()))
                    }),
                    FleetColumn::LastLog => cell(ui, |out| {
                        if needs_last_line
                            && !frontend_core::log::global().slot_last_message(row.name(), out)
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
    if let Some(op) = header_op {
        session.submit(OperatorCommand::MarkFleet(op), Targets::Application);
    }
    if let Some((index, marked)) = mark_click {
        let name = session
            .fleet_model
            .rows()
            .get(index)
            .map(|row| row.name().to_string());
        if let Some(name) = name {
            session.submit(OperatorCommand::SetMark(marked), Targets::Profile(&name));
        }
    }
    if let Some((moving, target)) = dropped {
        if let Some(op) = drop_op(&session.fleet_model, moving, target) {
            session.reorder_profiles(op);
        }
    }
    let row_name = |session: &Session, index: usize| {
        session
            .fleet_model
            .rows()
            .get(index)
            .map(|row| row.name().to_owned())
    };
    if let Some(name) = world_click.and_then(|index| row_name(session, index)) {
        session.open_world_picker(Some(&name));
    }
    if let Some(name) = name_click.and_then(|index| row_name(session, index)) {
        session.select(&name);
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
            // The rows, their order and the shown set are the shared fleet
            // model's, refreshed at the session sync (`pump_status`): this
            // frame computes nothing.
            fleet_toolbar::body(ui, session, fleet_toolbar::Site::Window, true);
            column_toggles(ui, session);
            ui.separator();
            table(ui, session);
            // A Load-order write that is pending or failed (a reorder, or
            // the quiet write after a rename or delete) says so below the
            // table, so the rows never move while it saves.
            crate::app::setting_feedback_line(ui, session, frontend_core::Setting::ProfileOrder);
            #[cfg(test)]
            record_window_layout(ui);
        });
    session.fleet_open = open;
}

#[cfg(test)]
mod tests {
    use frontend_core::test_support::session::install_vault;
    use frontend_core::test_support::{FixtureProfile, VaultFixture};
    use frontend_core::{
        fill_visible_row_indices, fleet_row_matches, FleetAction, FleetCard, FleetOrder, FleetRow,
        OperatorCommand, Phase, Targets,
    };

    /// The open window survives frames while columns come and go (the table
    /// is rebuilt with a different column count) and with a marked row:
    /// Dear ImGui aborts the process on a table it cannot reconcile.
    #[test]
    fn the_open_window_draws_across_column_changes() {
        let _guard = crate::test_support::imgui_context_guard();
        let dir = crate::test_support::TestDir::new("fleet-window-columns");
        let mut session = crate::session::Session::new();
        let mut vault = VaultFixture::create(&dir.join("vault"), "test-passphrase-01");
        for (i, name) in ["alice", "bob"].into_iter().enumerate() {
            vault.put(FixtureProfile::new(name, "pw").uid(1 + i as i32));
        }
        install_vault(&mut session.core, vault);
        session.fleet_open = true;
        let bob = session.core.profile_identity("bob").unwrap();
        session.fleet_model.set_mark_for_test(bob, true);
        let mut ctx = crate::test_support::imgui_context();
        let mut frame = |session: &mut crate::session::Session| {
            session.pump_status();
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

    /// One Fleet-window session end to end: the window shows the shared
    /// model's rows (every vault profile, not loaded), filters and sorts
    /// them, a mark whose profile is deleted is dropped at the session
    /// sync, and Assign runs as the bulk action over the marks.
    #[test]
    fn fleet_window_session_filters_sorts_marks_and_assigns() {
        let _guard = crate::test_support::imgui_context_guard();
        let dir = crate::test_support::TestDir::new("fleet-window-session");
        let mut session = crate::session::Session::new();
        let mut vault = VaultFixture::create(&dir.join("vault"), "test-passphrase-01");
        for (uid, name) in [(3, "carol"), (2, "bob"), (1, "alice"), (4, "dave")] {
            vault.put(FixtureProfile::new(name, "pw").uid(uid));
        }
        install_vault(&mut session.core, vault);
        session.fleet_open = true;
        let mut ctx = crate::test_support::imgui_context();
        let mut frame = |session: &mut crate::session::Session| {
            session.pump_status();
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
        let shown_names = |session: &crate::session::Session| {
            session
                .fleet_model
                .shown_rows()
                .map(|(_, row)| row.name().to_string())
                .collect::<Vec<_>>()
        };
        // Load order: nothing is loaded, so the vault's order.
        frame(&mut session);
        assert_eq!(shown_names(&session), ["alice", "bob", "carol", "dave"]);
        session.submit(
            frontend_core::OperatorCommand::SetFleetSort(FleetOrder::Name),
            frontend_core::Targets::Application,
        );
        // Filter: only bob's name matches "bob".
        session.fleet_model.staged_filter_mut().push_str("bob");
        session.submit(
            frontend_core::OperatorCommand::SetFleetFilter,
            frontend_core::Targets::Application,
        );
        frame(&mut session);
        assert_eq!(shown_names(&session), ["bob"]);
        // Marks: bob's stays; dave's goes with his deleted profile.
        let bob = session.core.profile_identity("bob").unwrap();
        let dave = session.core.profile_identity("dave").unwrap();
        session.fleet_model.set_mark_for_test(bob, true);
        session.fleet_model.set_mark_for_test(dave, true);
        assert!(session.vault_remove("dave"));
        frame(&mut session);
        assert_eq!(
            session.fleet_model.marks().len(),
            1,
            "the gone mark is dropped"
        );
        assert!(session.fleet_model.marks().contains(bob));
        // Bulk action: a pinned card is assigned to the marked bot only, at
        // once (one click: a marked Assign needs no confirmation).
        #[cfg(feature = "js")]
        let choice = {
            let path = dir.join("looper.ts");
            std::fs::write(
                &path,
                "export default class T extends LoopingBot { override loop() {} }\n",
            )
            .unwrap();
            session.scripts.load_card(&path).unwrap().choice()
        };
        #[cfg(not(feature = "js"))]
        let choice = frontend_core::scripts::ScriptChoice::compiled(
            frontend_core::scripts::CompiledCardId::lookup("Gatherer").unwrap(),
        );
        session.fleet_model.stage_card(FleetCard::pinned(choice));
        session.submit(OperatorCommand::SetFleetCard, Targets::Application);
        session.plan_fleet_action(FleetAction::Assign);
        session.core.flush_writes();
        assert!(session.profile_assignment("bob").is_some());
        assert!(session.profile_assignment("carol").is_none());
        assert!(session.profile_assignment("alice").is_none());
        let report = session
            .core
            .fleet_result()
            .map(|result| result.text().to_owned())
            .expect("the assign result");
        assert!(report.contains("Assign"), "{report}");
    }

    /// Marks follow the profile universe at the per-frame session sync,
    /// not only while the Fleet window is drawn: with the window closed,
    /// one `pump_status` after a delete drops that profile's mark and
    /// keeps the others.
    #[test]
    fn marks_are_pruned_at_the_session_sync_with_the_fleet_window_closed() {
        let dir = crate::test_support::TestDir::new("fleet-prune-closed");
        let mut session = crate::session::Session::new();
        let mut vault = VaultFixture::create(&dir.join("vault"), "test-passphrase-01");
        vault.put(FixtureProfile::new("bob", "pw").uid(2));
        vault.put(FixtureProfile::new("gone", "pw").uid(3));
        install_vault(&mut session.core, vault);
        assert!(!session.fleet_open, "the Fleet window stays closed");
        session.pump_status();
        let bob = session.core.profile_identity("bob").unwrap();
        let gone = session.core.profile_identity("gone").unwrap();
        session.fleet_model.set_mark_for_test(bob, true);
        session.fleet_model.set_mark_for_test(gone, true);
        assert!(session.vault_remove("gone"));
        session.pump_status();
        assert_eq!(
            session.fleet_model.marks().len(),
            1,
            "the gone mark is pruned"
        );
        assert!(session.fleet_model.marks().contains(bob));
    }

    /// The model's prune binary-searches a sorted universe, so its
    /// in-place sort is load-bearing: vault order here is alphabetical by
    /// username, which is not UID order, and every mark must survive the
    /// prune. A vault swap with no Fleet frame then prunes at the next
    /// sync.
    #[test]
    fn prune_keeps_marks_when_vault_order_differs_from_uid_order() {
        let dir = crate::test_support::TestDir::new("fleet-prune-order");
        let mut session = crate::session::Session::new();
        let mut vault = VaultFixture::create(&dir.join("vault"), "test-passphrase-01");
        for (username, uid) in [("zed", 1), ("amy", 3), ("max", 2), ("bea", 5), ("kim", 4)] {
            vault.put(FixtureProfile::new(username, "pw").uid(uid));
        }
        install_vault(&mut session.core, vault);
        let order: Vec<_> = session
            .core
            .profiles()
            .map(|profile| profile.uid())
            .collect();
        let mut sorted = order.clone();
        sorted.sort_unstable();
        assert_ne!(
            order, sorted,
            "the fixture's vault order must differ from UID order"
        );
        for name in ["zed", "amy", "max", "bea", "kim"] {
            let id = session.core.profile_identity(name).unwrap();
            session.fleet_model.set_mark_for_test(id, true);
        }
        session.pump_status();
        assert_eq!(
            session.fleet_model.marks().len(),
            5,
            "every vault mark is kept"
        );
        session.pump_status();
        assert_eq!(
            session.fleet_model.marks().len(),
            5,
            "every vault mark is kept"
        );
        // Universe change without a Fleet frame: a smaller vault replaces it.
        let mut smaller = VaultFixture::create(&dir.join("vault2"), "test-passphrase-01");
        smaller.put(FixtureProfile::new("amy", "pw").uid(3));
        install_vault(&mut session.core, smaller);
        session.pump_status();
        assert_eq!(
            session.fleet_model.marks().len(),
            1,
            "only amy's mark survives the swap"
        );
        assert!(session
            .fleet_model
            .marks()
            .contains(frontend_core::ProfileIdentity::uid(3)));
    }

    fn shown_names<'a>(rows: &'a [FleetRow], filter: &str, order: FleetOrder) -> Vec<&'a str> {
        let mut shown = Vec::new();
        fill_visible_row_indices(rows, filter, order, &mut shown);
        shown.iter().map(|&index| rows[index].name()).collect()
    }

    #[test]
    fn filtering_and_sorting_keep_one_row_per_projection() {
        let rows = vec![
            FleetRow::fixture("bob", Phase::Ready, Some(301), None),
            FleetRow::fixture("alice", Phase::Preparing, Some(302), None),
        ];
        assert!(fleet_row_matches(rows[0].name(), Some(&rows[0]), "idle"));
        assert_eq!(shown_names(&rows, "", FleetOrder::Name)[0], "alice");
        assert_eq!(shown_names(&rows, "starting", FleetOrder::Status).len(), 1);
    }

    #[test]
    fn status_sort_orders_by_phase_label_then_name() {
        let rows = vec![
            FleetRow::fixture("zoe", Phase::Ready, Some(301), None),
            FleetRow::fixture("amy", Phase::Ready, Some(301), None),
            FleetRow::fixture("bob", Phase::Offline, None, None),
        ];
        assert_eq!(
            shown_names(&rows, "", FleetOrder::Status),
            ["bob", "amy", "zoe"],
            "offline < ready, then name"
        );
        assert!(
            fleet_row_matches(rows[2].name(), Some(&rows[2]), "OFF"),
            "brief matches case-insensitively"
        );
    }
}

#[cfg(test)]
#[path = "fleet_flow_tests.rs"]
mod flow_tests;
