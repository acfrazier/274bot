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
use frontend_core::{
    fill_visible_row_indices, marked_availability, marked_startable, FleetOrder, FleetRow,
    MarkedAction, MarkedBlock, Scripts,
};

use crate::fleet_columns::{self, FleetColumn};
use crate::session::Session;
use crate::theme::{scale_px, scale_size, WARN};

const RESTART_POPUP: &str = "##fleet-restart-confirm";

/// The Fleet window's smallest size at scale 1: wide enough for its longest
/// button, so nothing runs past the edge however narrow it is dragged.
pub(crate) const MIN_WIDTH: f32 = 360.0;
const MIN_HEIGHT: f32 = 240.0;

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

fn profile_rows(session: &Session) -> Vec<FleetRow> {
    let projected = session.core.fleet_view().rows();
    if !session.core.vault_open() {
        return projected.iter().map(FleetRow::snapshot).collect();
    }
    session
        .core
        .profiles()
        .map(|profile| {
            let name = profile.username();
            projected
                .iter()
                .find(|row| row.name() == name)
                .map(FleetRow::snapshot)
                .unwrap_or_else(|| FleetRow::for_profile(name))
        })
        .collect()
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
    fn new(count: usize, has_card: bool) -> Self {
        Self {
            start: if has_card {
                format!("Start selected card on {count} marked bots")
            } else {
                format!("Start saved script on {count} marked bots")
            },
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
    let blocked = |action: MarkedAction| marked_availability(action, count, has_card, true);
    let card_tip = |action: MarkedAction, ready: &str| match blocked(action) {
        Err(MarkedBlock::NothingMarked) => "mark at least one profile".to_string(),
        Err(MarkedBlock::NoCard) => "select a card in Scripts first".to_string(),
        _ => ready.to_string(),
    };
    let startable = marked_availability(MarkedAction::Start, count, has_card, true).is_ok()
        && marked_startable(
            &session.fleet_selection,
            &session.core,
            session.script_sel.as_ref(),
        );
    let ActionLabels {
        start: start_label,
        stop: stop_label,
        log_in: log_in_label,
        log_out: log_out_label,
        walk: walk_label,
    } = ActionLabels::new(count, has_card);
    {
        let _disabled = (!startable).then(|| ui.begin_disabled());
        if ui.button(start_label) {
            session.fleet_start_selected();
        }
        ui.set_item_tooltip(if count == 0 {
            "mark at least one profile"
        } else if !startable {
            "select a card in Scripts first"
        } else if has_card {
            "start the selected card on every marked profile"
        } else {
            "start each marked profile's saved script"
        });
    }
    continue_line_if_fits(ui, button_width(ui, &stop_label));
    {
        let _disabled = blocked(MarkedAction::Stop)
            .is_err()
            .then(|| ui.begin_disabled());
        if ui.button(stop_label) {
            session.fleet_stop_selected();
        }
        ui.set_item_tooltip("stop every marked profile; ineligible rows are reported");
    }
    {
        let _disabled = blocked(MarkedAction::Assign)
            .is_err()
            .then(|| ui.begin_disabled());
        if ui.button(ASSIGN_LABEL) {
            session.fleet_assign_selected();
        }
        ui.set_item_tooltip(card_tip(
            MarkedAction::Assign,
            "save the selected card on every marked profile without starting it, \
             so each bot's settings can be adjusted first; running bots are skipped",
        ));
        continue_line_if_fits(ui, button_width(ui, ASSIGN_RESTART_LABEL));
        if ui.button(ASSIGN_RESTART_LABEL) {
            session.fleet_restart_confirm = true;
            ui.open_popup(RESTART_POPUP);
        }
        ui.set_item_tooltip(card_tip(
            MarkedAction::AssignAndRestart,
            "save the selected card and start it on every marked profile; \
             running scripts are stopped first (a confirmation names them)",
        ));
    }
    restart_confirm(ui, session);
    {
        let _disabled = blocked(MarkedAction::Login)
            .is_err()
            .then(|| ui.begin_disabled());
        if ui.button(log_in_label) {
            session.fleet_login_selected();
        }
        ui.set_item_tooltip("log in every marked profile, loading the ones not loaded yet");
    }
    continue_line_if_fits(ui, button_width(ui, &log_out_label));
    {
        let _disabled = blocked(MarkedAction::Logout)
            .is_err()
            .then(|| ui.begin_disabled());
        if ui.button(log_out_label) {
            session.fleet_logout_selected();
        }
        ui.set_item_tooltip("log out every marked profile that is logged in");
    }
    continue_line_if_fits(ui, button_width(ui, &walk_label));
    {
        let _disabled = blocked(MarkedAction::Walk)
            .is_err()
            .then(|| ui.begin_disabled());
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
    let ready = marked_availability(
        MarkedAction::ApplySettings,
        count,
        session.script_sel.is_some(),
        session.focused_name().is_some(),
    )
    .is_ok();
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
        for line in scope.prompt_lines() {
            ui.text_wrapped(line);
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
        crate::ui_state::save(&session.preferences, &session.ui);
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
        for sort in FleetOrder::PANEL_CHOICES {
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

fn table(ui: &Ui, session: &mut Session, rows: &[FleetRow], shown: &[usize]) {
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
        for &index in shown {
            let row = &rows[index];
            let marked_id = session.core.profile_identity_or_synthetic(row.name());
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
            ui.text(row.name());
            if session.scripts.has_restart_badge(row.name()) {
                ui.same_line();
                ui.text_colored(WARN, "[restart]");
                ui.set_item_tooltip("saved settings need a restart to apply");
            }
            let progress = if needs_progress {
                session.core.script_progress(row.name())
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
            // The surface's universe is every vault profile; marks are
            // pruned to it at the session sync (see
            // `Session::prune_fleet_marks`, run here as well so the
            // counts below never lag it), then the core model decides
            // the shown rows over it.
            let rows = profile_rows(session);
            session.prune_fleet_marks();
            let mut shown = std::mem::take(&mut session.fleet_shown);
            fill_visible_row_indices(&rows, &session.fleet_filter, session.fleet_sort, &mut shown);
            ui.separator();
            action_bar(ui, session);
            column_toggles(ui, session);
            ui.separator();
            table(ui, session, &rows, &shown);
            session.fleet_shown = shown;
            #[cfg(test)]
            record_window_layout(ui);
        });
    session.fleet_open = open;
}

#[cfg(test)]
mod tests {
    use super::ActionLabels;
    use frontend_core::test_support::session::install_vault;
    use frontend_core::test_support::{FixtureProfile, VaultFixture};
    use frontend_core::{fill_visible_row_indices, fleet_row_matches, FleetOrder, FleetRow, Phase};

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

    /// One Fleet-window session end to end: the window sorts and filters
    /// the vault profiles, a mark whose profile is gone is pruned at the
    /// window's sync, and Assign runs as the bulk action over the marks.
    #[test]
    fn fleet_window_session_filters_sorts_marks_and_assigns() {
        let _guard = crate::test_support::imgui_context_guard();
        let dir = crate::test_support::TestDir::new("fleet-window-session");
        let mut session = crate::session::Session::new();
        let mut vault = VaultFixture::create(&dir.join("vault"), "test-passphrase-01");
        for (uid, name) in [(3, "carol"), (2, "bob"), (1, "alice")] {
            vault.put(FixtureProfile::new(name, "pw").uid(uid));
        }
        install_vault(&mut session.core, vault);
        session.fleet_open = true;
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
        let shown_names = |session: &crate::session::Session| {
            let rows = super::profile_rows(session);
            session
                .fleet_shown
                .iter()
                .map(|&index| rows[index].name().to_string())
                .collect::<Vec<_>>()
        };
        // Sort: Name order over the vault's insertion order.
        frame(&mut session);
        assert_eq!(shown_names(&session), ["alice", "bob", "carol"]);
        // Filter: only bob's name matches "bob" (a bare "o" would also
        // match the offline status of every row).
        session.fleet_filter = "bob".into();
        frame(&mut session);
        assert_eq!(shown_names(&session), ["bob"]);
        // Marks: bob's stays; a mark outside the universe is pruned.
        let bob = session.core.profile_identity("bob").unwrap();
        session.fleet_selection.set(bob, true);
        session
            .fleet_selection
            .set(frontend_core::ProfileIdentity::synthetic("gone"), true);
        frame(&mut session);
        assert_eq!(session.fleet_selection.len(), 1, "the gone mark is pruned");
        assert!(session.fleet_selection.contains(bob));
        // Bulk action: assign the selected card to the marked bot only.
        let path = dir.join("looper.ts");
        std::fs::write(
            &path,
            "export default class T extends LoopingBot { override loop() {} }\n",
        )
        .unwrap();
        let card = session.scripts.load_card(&path).unwrap();
        session.script_sel = Some(card.choice());
        session.fleet_assign_selected();
        session.core.flush_writes();
        assert!(session.profile_assignment("bob").is_some());
        assert!(session.profile_assignment("carol").is_none());
        assert!(session.profile_assignment("alice").is_none());
        let report = session.fleet_report.clone().expect("the assign report");
        assert!(report.contains("Assign"), "{report}");
    }

    /// Marks are pruned at the per-frame session sync, not only while
    /// the Fleet window is drawn: with the window closed, one
    /// `pump_status` drops the mark whose profile is gone and keeps
    /// the vault profile's mark.
    #[test]
    fn marks_are_pruned_at_the_session_sync_with_the_fleet_window_closed() {
        let dir = crate::test_support::TestDir::new("fleet-prune-closed");
        let mut session = crate::session::Session::new();
        let mut vault = VaultFixture::create(&dir.join("vault"), "test-passphrase-01");
        vault.put(FixtureProfile::new("bob", "pw").uid(2));
        install_vault(&mut session.core, vault);
        assert!(!session.fleet_open, "the Fleet window stays closed");
        let bob = session.core.profile_identity("bob").unwrap();
        session.fleet_selection.set(bob, true);
        session
            .fleet_selection
            .set(frontend_core::ProfileIdentity::synthetic("gone"), true);
        session.pump_status();
        assert_eq!(session.fleet_selection.len(), 1, "the gone mark is pruned");
        assert!(session.fleet_selection.contains(bob));
    }

    /// The session-sync prune binary-searches a sorted universe, so
    /// the in-place sort in `prune_fleet_marks` is load-bearing:
    /// vault order here is alphabetical by username, which is not
    /// UID order, and every mark must survive the prune. A vault
    /// swap with no Fleet frame then prunes at the next sync.
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
            session.fleet_selection.set(id, true);
        }
        session.pump_status();
        assert_eq!(session.fleet_selection.len(), 5, "every vault mark is kept");
        session.pump_status();
        assert_eq!(session.fleet_selection.len(), 5, "every vault mark is kept");
        // Universe change without a Fleet frame: a smaller vault replaces it.
        let mut smaller = VaultFixture::create(&dir.join("vault2"), "test-passphrase-01");
        smaller.put(FixtureProfile::new("amy", "pw").uid(3));
        install_vault(&mut session.core, smaller);
        session.pump_status();
        assert_eq!(
            session.fleet_selection.len(),
            1,
            "only amy's mark survives the swap"
        );
        assert!(session
            .fleet_selection
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

    /// No button of the action bar ends in an ellipsis, whatever the count.
    #[test]
    fn action_button_labels_have_no_ellipsis() {
        for count in [0, 1, 12] {
            for has_card in [true, false] {
                for label in ActionLabels::new(count, has_card).all() {
                    assert!(
                        !label.contains('\u{2026}') && !label.contains("..."),
                        "{label:?}"
                    );
                }
            }
        }
    }
}
