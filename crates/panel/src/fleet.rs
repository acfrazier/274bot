//! Fleet window: the wide table over the shared fleet model (every profile,
//! loaded or not, in the model's order and filter), its identity-keyed marks,
//! the fleet card drop-down, the marks row (tri-state box, marks menu and
//! count), the action bar (Start / Stop, Assign, Assign & restart, log in /
//! out, Copy settings, group walk) with its plan confirmation and result
//! line, and toggleable status columns.
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
    TableOptions, TableRowFlags, TableSizingPolicy, Ui, WindowFlags,
};
use frontend_core::progress::{self, NONE};
use frontend_core::{
    contains_ci, FleetAction, FleetCard, FleetModel, FleetOrder, MarkOp, MarkPredicate,
    OperatorCommand, Placement, ProfileIdentity, ReorderOp, Scripts, ShownMarks, Targets,
    FOCUSED_CARD_LABEL, OWN_CARD_LABEL,
};

use crate::fleet_columns::{self, FleetColumn};
use crate::script_picker::{display_category, resolve_category_order};
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
    /// The Profile name cell of each shown row, in row order, as
    /// `[min_x, min_y, max_x, max_y]`.
    pub names: Vec<[f32; 4]>,
    /// The Move marked to top button's `[min_x, min_y, max_x, max_y]`.
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
    /// Each fleet action button, in action-bar order.
    pub actions: Vec<ActionRecord>,
    /// The waiting plan's prompt, Confirm and Cancel, when one is drawn.
    pub plan: Option<PlanRecord>,
    /// The result line's text and rect, when one is drawn.
    pub result: Option<(String, [f32; 4])>,
}

/// One fleet action button as a test sees it.
#[cfg(test)]
#[derive(Debug, Clone)]
pub(crate) struct ActionRecord {
    pub action: FleetAction,
    pub label: String,
    pub rect: [f32; 4],
    pub disabled: bool,
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
}

#[cfg(test)]
std::thread_local! {
    /// Plan rows drawn since the last [`take_plan_rows`].
    static PLAN_ROWS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// How many plan rows were drawn since the last call.
#[cfg(test)]
pub(crate) fn take_plan_rows() -> usize {
    PLAN_ROWS.replace(0)
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
fn record_move_to_top(ui: &Ui) {
    let (min, max) = (ui.item_rect_min(), ui.item_rect_max());
    LAYOUT.with_borrow_mut(|layout| layout.move_to_top = [min[0], min[1], max[0], max[1]]);
}

/// The last item's `[min_x, min_y, max_x, max_y]`.
#[cfg(test)]
fn item_rect(ui: &Ui) -> [f32; 4] {
    let (min, max) = (ui.item_rect_min(), ui.item_rect_max());
    [min[0], min[1], max[0], max[1]]
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

const MOVE_TO_TOP_LABEL: &str = "Move marked to top";
const MARKS_MENU_POPUP: &str = "##fleet-marks-menu";
/// The tooltip of a button that needs a card while the fleet card is each
/// bot's own.
const PICK_A_CARD: &str = "pick a card above";
/// The tooltip of a fleet button with no bot to act on.
const NOTHING_TO_ACT_ON: &str = "mark bots, or show loaded ones, first";

/// The window's count-bearing labels (the marks count and the action
/// buttons), rebuilt only when the fleet model's generation moves, never
/// per frame. No label ends in an ellipsis: a button that opens a
/// confirmation still names its action plainly. Each carries an id part
/// after `###`, so a changed count never changes the button's identity.
#[derive(Debug, Default)]
pub(crate) struct FleetLabels {
    generation: Option<u64>,
    /// How many bots are marked, shown or not.
    marked: usize,
    /// How many bots an action covers: the marks, else the loaded shown rows.
    scope: usize,
    /// "5 marked, 2 not shown".
    marks: String,
    /// "Stop 5" with marks, "Stop all 12" without, in [`FleetAction::ALL`] order.
    actions: [String; FleetAction::ALL.len()],
    walk: String,
}

impl FleetLabels {
    /// Rebuild every label when `model` changed since the last call.
    fn sync(&mut self, model: &FleetModel) {
        use std::fmt::Write as _;
        if self.generation == Some(model.generation()) {
            return;
        }
        self.generation = Some(model.generation());
        let counts = model.counts();
        // The mark set itself, not the per-row count: it is right before the
        // first session sync too, and the plan acts on it.
        self.marked = model.marks().len();
        self.scope = if self.marked > 0 {
            self.marked
        } else {
            counts.shown_loaded()
        };
        self.marks.clear();
        let _ = write!(
            self.marks,
            "{} marked, {} not shown",
            self.marked,
            counts.hidden_marked()
        );
        for (index, (label, action)) in self.actions.iter_mut().zip(FleetAction::ALL).enumerate() {
            label.clear();
            let _ = if self.marked > 0 {
                write!(
                    label,
                    "{} {}###fleet-action-{index}",
                    action.verb(),
                    self.marked
                )
            } else {
                write!(
                    label,
                    "{} all {}###fleet-action-{index}",
                    action.verb(),
                    self.scope
                )
            };
        }
        self.walk.clear();
        let _ = write!(self.walk, "Walk {} marked bots", self.marked);
    }

    /// Every label of the bar, including the fixed one.
    #[cfg(test)]
    fn all(&self) -> impl Iterator<Item = &str> {
        self.actions.iter().map(String::as_str).chain([
            self.walk.as_str(),
            MOVE_TO_TOP_LABEL,
            self.marks.as_str(),
        ])
    }

    /// The buffers of every cached label, to see that a frame kept them.
    #[cfg(test)]
    pub(crate) fn buffers(&self) -> Vec<*const u8> {
        self.actions
            .iter()
            .map(|label| label.as_ptr())
            .chain([self.marks.as_ptr(), self.walk.as_ptr()])
            .collect()
    }
}

/// The tooltip of `action`'s button: what it does and over whom.
fn action_tooltip(action: FleetAction, marked: bool) -> &'static str {
    match (action, marked) {
        (FleetAction::Start, true) => {
            "start the fleet card on every marked bot (each bot's own card when that is picked)"
        }
        (FleetAction::Start, false) => {
            "start the fleet card on every loaded bot shown, after asking first"
        }
        (FleetAction::Stop, true) => {
            "stop every marked bot's script; a confirmation names the running ones"
        }
        (FleetAction::Stop, false) => "stop every loaded bot shown, after asking first",
        (FleetAction::Assign, true) => {
            "save the fleet card on every marked bot without starting it, so each bot's \
             settings can be adjusted first"
        }
        (FleetAction::Assign, false) => {
            "save the fleet card on every loaded bot shown, after asking first"
        }
        (FleetAction::AssignRestart, true) => {
            "save the fleet card and start it on every marked bot; running scripts are \
             stopped first (a confirmation names them)"
        }
        (FleetAction::AssignRestart, false) => {
            "save the fleet card and start it on every loaded bot shown, after asking first"
        }
        (FleetAction::Login, true) => "log in every marked bot, loading the ones not loaded yet",
        (FleetAction::Login, false) => "log in every loaded bot shown, after asking first",
        (FleetAction::Logout, true) => "log out every marked bot",
        (FleetAction::Logout, false) => "log out every loaded bot shown, after asking first",
        (FleetAction::CopySettings, true) => {
            "copy the focused bot's settings for the fleet card to the marked bots on that \
             card; a confirmation names them first"
        }
        (FleetAction::CopySettings, false) => {
            "copy the focused bot's settings for the fleet card to the loaded bots shown \
             that are on that card, after asking first"
        }
    }
}

/// Stage `card` as the fleet card and commit it. Nothing else changes: no
/// Browse pick, no assignment, no profile form, no Start.
fn pick_card(session: &mut Session, card: FleetCard) {
    session.fleet_model.stage_card(card);
    session.submit(OperatorCommand::SetFleetCard, Targets::Application);
}

/// The "Card:" drop-down, full width like Sort: the card the Start, Assign,
/// Assign & restart and Copy settings buttons use. Entries are each bot's
/// own card, the focused bot's card, then every Browse card under its
/// category in the Browse window's category order. While it is open a
/// search field narrows the entries.
fn card_combo(ui: &Ui, session: &mut Session) {
    ui.text("Card:");
    session
        .fleet_model
        .sync_card_label(&session.scripts, &session.core);
    ui.set_next_item_width(-1.0);
    let combo = ui.begin_combo("##fleet-card", session.fleet_model.card_label());
    #[cfg(test)]
    LAYOUT.with_borrow_mut(|layout| layout.card_combo = item_rect(ui));
    let Some(_open) = combo else {
        session.fleet_card_filter = None;
        return;
    };
    let mut filter = session.fleet_card_filter.take().unwrap_or_else(|| {
        ui.set_keyboard_focus_here();
        String::new()
    });
    ui.set_next_item_width(-1.0);
    ui.input_text("##fleet-card-filter", &mut filter)
        .hint("Filter cards")
        .build();
    ui.separator();
    let picked = card_entries(ui, session, &filter);
    session.fleet_card_filter = Some(filter);
    if let Some(card) = picked {
        pick_card(session, card);
    }
}

/// The open card drop-down's entries that match `filter`; the one picked.
fn card_entries(ui: &Ui, session: &Session, filter: &str) -> Option<FleetCard> {
    let current = session.fleet_model.card();
    let matches = |text: &str| filter.is_empty() || contains_ci(text, filter);
    let mut picked = None;
    let entry = |label: &str, selected: bool, picked: &mut Option<FleetCard>, card| {
        let clicked = ui.selectable_config(label).selected(selected).build();
        #[cfg(test)]
        LAYOUT.with_borrow_mut(|layout| {
            layout.card_entries.push((label.to_owned(), item_rect(ui)));
        });
        if clicked {
            *picked = Some(card);
        }
        if selected {
            ui.set_item_default_focus();
        }
    };
    if matches(OWN_CARD_LABEL) {
        entry(
            OWN_CARD_LABEL,
            current.is_own(),
            &mut picked,
            FleetCard::OWN,
        );
    }
    if matches(FOCUSED_CARD_LABEL) {
        entry(
            FOCUSED_CARD_LABEL,
            current.is_focused(),
            &mut picked,
            FleetCard::FOCUSED,
        );
    }
    ui.separator();
    let mut present: Vec<String> = Vec::new();
    for card in session.scripts.browse_cards() {
        let category = display_category(card.category());
        if !present.iter().any(|known| known == category) {
            present.push(category.to_owned());
        }
    }
    let order = resolve_category_order(&session.ui.script_category_order, &present);
    let mut any = false;
    for category in &order {
        let mut heading = false;
        for (index, card) in session.scripts.browse_cards().enumerate() {
            if display_category(card.category()) != category.as_str() || !matches(card.name()) {
                continue;
            }
            if !heading {
                heading = true;
                ui.text_disabled(category);
            }
            any = true;
            let _id = ui.push_id(index);
            let choice = card.selection();
            let selected = current.pinned_card() == Some(&choice);
            entry(
                card.name(),
                selected,
                &mut picked,
                FleetCard::pinned(choice),
            );
        }
    }
    if !any {
        ui.text_disabled("no matching cards");
    }
    picked
}

/// A check box over the shown rows that also reads "some": a dash in the
/// box while only some shown rows are marked. Returns whether it was
/// clicked; the caller decides what each state's click does.
fn tri_state_box(ui: &Ui, id: &str, state: ShownMarks) -> bool {
    let mut checked = state == ShownMarks::All;
    let clicked = ui.checkbox(id, &mut checked);
    if state == ShownMarks::Some {
        let (min, max) = (ui.item_rect_min(), ui.item_rect_max());
        let side = max[1] - min[1];
        let (mid_y, thickness) = ((min[1] + max[1]) / 2.0, (side * 0.14).max(2.0));
        ui.get_window_draw_list()
            .add_rect(
                [min[0] + side * 0.28, mid_y - thickness / 2.0],
                [min[0] + side * 0.72, mid_y + thickness / 2.0],
                ui.style_color(StyleColor::CheckMark),
            )
            .filled(true)
            .build();
    }
    clicked
}

/// What a click on the tri-state box asks for: no shown row marked or some
/// marked mark every shown row; every shown row marked unmarks them.
fn tri_state_op(state: ShownMarks) -> MarkOp {
    match state {
        ShownMarks::All => MarkOp::UnmarkShown,
        ShownMarks::None | ShownMarks::Some => MarkOp::Shown,
    }
}

/// The tooltip of the tri-state box in `state`.
fn tri_state_tooltip(state: ShownMarks) -> &'static str {
    match state {
        ShownMarks::None => "mark every shown bot",
        ShownMarks::Some => "mark the rest of the shown bots",
        ShownMarks::All => "unmark every shown bot",
    }
}

/// The marks row: the tri-state box over the shown rows, the marks menu and
/// the count ("5 marked, 2 not shown"; filtered-out marks stay).
fn marks_row(ui: &Ui, session: &mut Session) {
    let state = session.fleet_model.shown_marks();
    let no_rows = session.fleet_model.counts().shown() == 0;
    let mut op = None;
    {
        let _disabled = no_rows.then(|| ui.begin_disabled());
        if tri_state_box(ui, "##fleet-mark-all", state) {
            op = Some(tri_state_op(state));
        }
        #[cfg(test)]
        LAYOUT.with_borrow_mut(|layout| layout.mark_box = item_rect(ui));
        ui.set_item_tooltip(tri_state_tooltip(state));
    }
    ui.same_line();
    if ui.button("Marks##fleet-marks-menu-button") {
        ui.open_popup(MARKS_MENU_POPUP);
    }
    #[cfg(test)]
    LAYOUT.with_borrow_mut(|layout| layout.marks_button = item_rect(ui));
    ui.set_item_tooltip("mark or unmark the shown bots, or mark them by status");
    if let Some(_menu) = ui.begin_popup(MARKS_MENU_POPUP) {
        let item = |label: &str| {
            let clicked = ui.menu_item(label);
            #[cfg(test)]
            LAYOUT.with_borrow_mut(|layout| {
                layout.marks_menu.push((label.to_owned(), item_rect(ui)));
            });
            clicked
        };
        if item("Mark shown") {
            op = Some(MarkOp::Shown);
        }
        if item("Unmark shown") {
            op = Some(MarkOp::UnmarkShown);
        }
        if item("Invert shown") {
            op = Some(MarkOp::InvertShown);
        }
        let by_status = ui.begin_menu("Mark by status");
        #[cfg(test)]
        LAYOUT.with_borrow_mut(|layout| {
            layout
                .marks_menu
                .push(("Mark by status".to_owned(), item_rect(ui)));
        });
        if let Some(_status) = by_status {
            for predicate in MarkPredicate::ALL {
                if item(predicate.label()) {
                    op = Some(MarkOp::Where(predicate));
                }
            }
        }
    }
    ui.same_line();
    ui.text(&session.fleet_labels.marks);
    #[cfg(test)]
    LAYOUT.with_borrow_mut(|layout| layout.marks_text = session.fleet_labels.marks.clone());
    if let Some(op) = op {
        session.submit(OperatorCommand::MarkFleet(op), Targets::Application);
    }
}

/// The action bar: one button per fleet action, naming the verb and how
/// many bots it covers, then Walk and Move marked to top. A click plans the
/// action (see [`Session::plan_fleet_action`]).
fn action_bar(ui: &Ui, session: &mut Session) {
    let labels = &session.fleet_labels;
    let own = session.fleet_model.card().is_own();
    let marked = labels.marked > 0;
    let nothing = labels.scope == 0;
    let mut planned = None;
    for (index, action) in FleetAction::ALL.into_iter().enumerate() {
        let label = labels.actions[index].as_str();
        if index > 0 {
            continue_line_if_fits(ui, button_width(ui, label));
        }
        let needs_card = action.needs_card() && own;
        let tooltip = if needs_card {
            PICK_A_CARD
        } else if nothing {
            NOTHING_TO_ACT_ON
        } else {
            action_tooltip(action, marked)
        };
        let _disabled = (needs_card || nothing).then(|| ui.begin_disabled());
        if ui.button(label) {
            planned = Some(action);
        }
        ui.set_item_tooltip(tooltip);
        #[cfg(test)]
        LAYOUT.with_borrow_mut(|layout| {
            layout.actions.push(ActionRecord {
                action,
                label: label.to_owned(),
                rect: item_rect(ui),
                disabled: needs_card || nothing,
                tooltip,
            });
        });
    }
    let mut walk = false;
    continue_line_if_fits(ui, button_width(ui, &labels.walk));
    {
        let _disabled = (!marked).then(|| ui.begin_disabled());
        if ui.button(&labels.walk) {
            walk = true;
        }
        ui.set_item_tooltip(
            "open the WalkTo picker for the marked bots: pick a tile and every bot \
             walks there from where it stands",
        );
    }
    continue_line_if_fits(ui, button_width(ui, MOVE_TO_TOP_LABEL));
    let mut move_to_top = false;
    {
        let load_order = session.fleet_model.order() == FleetOrder::LoadOrder;
        let ready = load_order && marked;
        let _disabled = (!ready).then(|| ui.begin_disabled());
        if ui.button(MOVE_TO_TOP_LABEL) {
            move_to_top = true;
        }
        #[cfg(test)]
        record_move_to_top(ui);
        ui.set_item_tooltip(if !load_order {
            "choose Load order to reorder"
        } else if !marked {
            "mark profiles first"
        } else {
            "move the marked profiles to the top of the saved Load order, \
             keeping their relative order"
        });
    }
    if let Some(action) = planned {
        session.plan_fleet_action(action);
    }
    if walk {
        session.open_walkto_for_marked();
    }
    if move_to_top {
        session.reorder_profiles(ReorderOp::MarkedToTop);
    }
}

/// The waiting plan's confirmation row: the core's prompt, wrapped, with
/// Confirm and Cancel. Draws nothing while no plan waits. Draw it from one
/// window only: the Fleet window, or the Scripts window while the Fleet
/// window is closed.
pub(crate) fn plan_row(ui: &Ui, session: &mut Session) {
    let Some(plan) = session.core.fleet_plan() else {
        return;
    };
    #[cfg(test)]
    PLAN_ROWS.set(PLAN_ROWS.get() + 1);
    ui.text_wrapped(plan.prompt());
    #[cfg(test)]
    let prompt = plan.prompt().to_owned();
    let confirm = ui.button("Confirm##fleet-plan");
    #[cfg(test)]
    let confirm_rect = item_rect(ui);
    ui.same_line();
    let cancel = ui.button("Cancel##fleet-plan");
    #[cfg(test)]
    LAYOUT.with_borrow_mut(|layout| {
        layout.plan = Some(PlanRecord {
            prompt,
            confirm: confirm_rect,
            cancel: item_rect(ui),
        });
    });
    if confirm {
        session.confirm_fleet_plan();
    } else if cancel {
        session.cancel_fleet_plan();
    }
}

/// The one result line of the latest fleet action (the core's text:
/// failures, skips, done, pending), wrapped, with the full text in a
/// tooltip. Draws nothing before the first action.
pub(crate) fn result_line(ui: &Ui, session: &Session) {
    let Some(result) = session.core.fleet_result() else {
        return;
    };
    ui.text_wrapped(result.text());
    #[cfg(test)]
    LAYOUT.with_borrow_mut(|layout| {
        layout.result = Some((result.text().to_owned(), item_rect(ui)));
    });
    if ui.is_item_hovered() {
        ui.tooltip(|| {
            let _wrap = ui.push_text_wrap_pos(scale_px(ui, 420.0));
            ui.text_wrapped(result.text());
        });
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
    let order = session.fleet_model.order();
    let combo = ui.begin_combo("##fleet-sort", order.label());
    #[cfg(test)]
    record_sort_combo(ui);
    if let Some(_open) = combo {
        for sort in FleetOrder::CHOICES {
            let selected = sort == order;
            let picked = ui
                .selectable_config(sort.label())
                .selected(selected)
                .build();
            #[cfg(test)]
            record_sort_choice(ui);
            if picked {
                session.submit(OperatorCommand::SetFleetSort(sort), Targets::Application);
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

/// The table over the model's shown rows. A mark click is submitted after
/// the rows are drawn (the rows are borrowed from the session's model).
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
                LAYOUT.with_borrow_mut(|layout| layout.header_mark_box = item_rect(ui));
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
            ui.table_next_column();
            #[cfg(test)]
            record_cell(ui);
            if reorder {
                // The name is the drag handle and the whole row the drop
                // target (a selectable spanning every column; the Mark
                // checkbox, submitted first, keeps its own clicks).
                #[cfg(test)]
                let name_at = ui.cursor_screen_pos();
                {
                    let _dim = (!row.loaded()).then(|| {
                        ui.push_style_color(
                            StyleColor::Text,
                            ui.style_color(StyleColor::TextDisabled),
                        )
                    });
                    ui.selectable_config(row.name())
                        .flags(SelectableFlags::SPAN_ALL_COLUMNS | SelectableFlags::ALLOW_OVERLAP)
                        .build();
                    profile_drag_source(ui, marked_id, row.name());
                    if let Some(moving) = profile_drop_target(ui) {
                        dropped = Some((moving, index));
                    }
                }
                #[cfg(test)]
                record_name(ui, name_at, row.name());
            } else if row.loaded() {
                ui.text(row.name());
                #[cfg(test)]
                record_name(ui, ui.item_rect_min(), row.name());
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
            if ui
                .input_text("Filter", session.fleet_model.staged_filter_mut())
                .hint("name or status")
                .build()
            {
                session.submit(OperatorCommand::SetFleetFilter, Targets::Application);
            }
            sort_combo(ui, session);
            // The rows, their order and the shown set are the shared fleet
            // model's, refreshed at the session sync (`pump_status`): this
            // frame computes nothing.
            ui.separator();
            session.fleet_labels.sync(&session.fleet_model);
            card_combo(ui, session);
            marks_row(ui, session);
            action_bar(ui, session);
            plan_row(ui, session);
            result_line(ui, session);
            column_toggles(ui, session);
            ui.separator();
            table(ui, session);
            #[cfg(test)]
            record_window_layout(ui);
        });
    session.fleet_open = open;
}

#[cfg(test)]
mod tests {
    use super::FleetLabels;
    use frontend_core::test_support::session::install_vault;
    use frontend_core::test_support::{FixtureProfile, VaultFixture};
    use frontend_core::{
        fill_visible_row_indices, fleet_row_matches, FleetAction, FleetCard, FleetModel,
        FleetOrder, FleetRow, OperatorCommand, Phase, Targets,
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
        let path = dir.join("looper.ts");
        std::fs::write(
            &path,
            "export default class T extends LoopingBot { override loop() {} }\n",
        )
        .unwrap();
        let card = session.scripts.load_card(&path).unwrap();
        session
            .fleet_model
            .stage_card(FleetCard::pinned(card.choice()));
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

    /// No label of the action bar ends in an ellipsis, whatever the marks.
    #[test]
    fn action_button_labels_have_no_ellipsis() {
        let mut model = FleetModel::new();
        model.set_universe_for_test(
            vec![
                FleetRow::fixture("bob", Phase::Ready, Some(301), None),
                FleetRow::fixture("alice", Phase::Ready, Some(302), None),
            ],
            &[],
        );
        let mut labels = FleetLabels::default();
        for marked in [0, 1, 2] {
            model.clear_marks_for_test();
            for row in 0..marked {
                model.set_mark_for_test(model.identity(row).unwrap(), true);
            }
            labels.sync(&model);
            for label in labels.all() {
                assert!(
                    !label.contains('\u{2026}') && !label.contains("..."),
                    "{label:?}"
                );
            }
        }
    }
}

#[cfg(test)]
#[path = "fleet_flow_tests.rs"]
mod flow_tests;
