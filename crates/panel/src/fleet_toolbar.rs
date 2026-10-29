//! The fleet toolbar: one draw function for the three places that show it.
//! The MultiBox rail draws it above its tiles, the right-hand strip draws it
//! in its "fleet" section while the rail is hidden (Grid, or MultiBox off),
//! and the Fleet window draws it above its table.
//!
//! The toolbar is, in order: the fleet card drop-down, the marks row
//! (tri-state box over the shown rows, Marks menu, count), the filter and
//! the sort, the nine action buttons (each names its scope and count), the
//! waiting plan's prompt with Confirm and Cancel, and the last result. The
//! rail and the strip draw a fold header above it. Folded, the toolbar
//! shows one summary line (marks and card), the filter, the waiting plan
//! and the result.
//!
//! Every surface reads the same state: the session's fleet model and the
//! core's plan and result. Every click goes through the session's command
//! flow; nothing here changes marks, settings, preferences other than the
//! fold, or lifecycle directly. The count-bearing labels are rebuilt only
//! when the fleet model's generation moves, and an open card drop-down
//! keeps its search text and card list ([`CardSearch`]), so a warm frame
//! allocates nothing in any state the toolbar shows.

#[cfg(feature = "js")]
use std::path::PathBuf;

use dear_imgui_rs::{ImString, ItemHoveredFlags, StyleColor, TreeNodeFlags, Ui};
#[cfg(feature = "js")]
use frontend_core::scripts::CardSource;
use frontend_core::scripts::{BrowseCard, ScriptChoice};
use frontend_core::{
    contains_ci, FleetAction, FleetCard, FleetModel, FleetOrder, MarkOp, MarkPredicate,
    OperatorCommand, ReorderOp, ShownMarks, Targets, FOCUSED_CARD_LABEL, OWN_CARD_LABEL,
};

#[cfg(test)]
use crate::fleet::{record, ActionRecord, PlanRecord};
use crate::script_picker::{display_category, resolve_category_order};
use crate::session::Session;
use crate::theme::scale_px;

/// Where a toolbar is drawn. The three sites draw the same function with
/// the same state; the site only names which card drop-down owns the
/// open search (and labels what a test records).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Site {
    /// The top of the MultiBox rail, above the scrolling tiles.
    Rail,
    /// The right-hand strip's "fleet" section, while the rail is hidden.
    Strip,
    /// The Fleet window, above its table.
    Window,
}

/// One toolbar button: a fleet plan, or a button that opens a picker
/// first (WalkTo for Walk, the world picker for Switch world).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ToolbarAction {
    /// Plan this fleet action over the scope.
    Plan(FleetAction),
    /// Open WalkTo in Group mode over the scope.
    Walk,
    /// Open the world picker over the scope.
    SwitchWorld,
}

impl ToolbarAction {
    /// The button verb.
    fn verb(self) -> &'static str {
        match self {
            Self::Plan(action) => action.verb(),
            Self::Walk => "Walk",
            Self::SwitchWorld => FleetAction::SwitchWorld(0).verb(),
        }
    }
}

/// The nine toolbar buttons, in draw order.
pub(crate) const ACTIONS: [ToolbarAction; 9] = [
    ToolbarAction::Plan(FleetAction::Start),
    ToolbarAction::Plan(FleetAction::Stop),
    ToolbarAction::Plan(FleetAction::Assign),
    ToolbarAction::Plan(FleetAction::AssignRestart),
    ToolbarAction::Plan(FleetAction::Login),
    ToolbarAction::Plan(FleetAction::Logout),
    ToolbarAction::Walk,
    ToolbarAction::SwitchWorld,
    ToolbarAction::Plan(FleetAction::CopySettings),
];

pub(crate) const MOVE_TO_TOP_LABEL: &str = "Move marked to top";
const MARKS_MENU_POPUP: &str = "##fleet-marks-menu";
/// The tooltip of a button that needs a card while the fleet card is each
/// bot's own.
const PICK_A_CARD: &str = "pick a card above";
/// The tooltip of a fleet button with no bot to act on.
const NOTHING_TO_ACT_ON: &str = "mark bots, or show loaded ones, first";
/// Why the world picker is not offered: a local server has one world.
pub(crate) const NO_WORLDS: &str = "this server has one world";
/// The filter field's hint.
const FILTER_HINT: &str = "filter: name, status, w3";
/// The narrowest filter field that shares its row with the sort.
const MIN_FILTER_W: f32 = 96.0;

/// The toolbar's count-bearing labels (the marks count, the folded summary
/// and the action buttons), rebuilt only when the fleet model's generation
/// moves, never per frame. No label ends in an ellipsis: a button that
/// opens a confirmation or a picker still names its action plainly. Each
/// button label carries an id part after `###`, so a changed count never
/// changes the button's identity.
#[derive(Debug, Default)]
pub(crate) struct FleetLabels {
    generation: Option<u64>,
    /// How many bots are marked, shown or not.
    pub(crate) marked: usize,
    /// How many bots an action covers: the marks, else the loaded shown rows.
    pub(crate) scope: usize,
    /// "5 marked, 2 not shown".
    marks: String,
    /// "5 marked ·": the folded header's summary, before the card label.
    folded: String,
    /// "Stop 5" with marks, "Stop all 12" without, in [`ACTIONS`] order.
    actions: [String; ACTIONS.len()],
}

impl FleetLabels {
    /// Rebuild every label when `model` changed since the last call.
    pub(crate) fn sync(&mut self, model: &FleetModel) {
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
        self.folded.clear();
        let _ = write!(self.folded, "{} marked \u{b7}", self.marked);
        for (index, (label, action)) in self.actions.iter_mut().zip(ACTIONS).enumerate() {
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
    }

    /// Every label of the toolbar, including the fixed one.
    #[cfg(test)]
    pub(crate) fn all(&self) -> impl Iterator<Item = &str> {
        self.actions.iter().map(String::as_str).chain([
            MOVE_TO_TOP_LABEL,
            self.marks.as_str(),
            self.folded.as_str(),
        ])
    }

    /// The heap bytes the labels hold (their buffers' capacities).
    #[cfg(test)]
    pub(crate) fn heap_bytes(&self) -> usize {
        self.actions
            .iter()
            .chain([&self.marks, &self.folded])
            .map(String::capacity)
            .sum()
    }

    /// The buffers of every cached label, to see that a frame kept them.
    #[cfg(test)]
    pub(crate) fn buffers(&self) -> Vec<*const u8> {
        self.actions
            .iter()
            .map(|label| label.as_ptr())
            .chain([self.marks.as_ptr(), self.folded.as_ptr()])
            .collect()
    }
}

/// The tooltip of `action`'s button: what it does and over whom.
fn action_tooltip(action: ToolbarAction, marked: bool) -> &'static str {
    use ToolbarAction::{Plan, SwitchWorld, Walk};
    match (action, marked) {
        (Plan(FleetAction::Start), true) => {
            "start the fleet card on every marked bot (each bot's own card when that is picked)"
        }
        (Plan(FleetAction::Start), false) => {
            "start the fleet card on every loaded bot shown, after asking first"
        }
        (Plan(FleetAction::Stop), true) => {
            "stop every marked bot's script; a confirmation names the running ones"
        }
        (Plan(FleetAction::Stop), false) => "stop every loaded bot shown, after asking first",
        (Plan(FleetAction::Assign), true) => {
            "save the fleet card on every marked bot without starting it, so each bot's \
             settings can be adjusted first"
        }
        (Plan(FleetAction::Assign), false) => {
            "save the fleet card on every loaded bot shown, after asking first"
        }
        (Plan(FleetAction::AssignRestart), true) => {
            "save the fleet card and start it on every marked bot; running scripts are \
             stopped first (a confirmation names them)"
        }
        (Plan(FleetAction::AssignRestart), false) => {
            "save the fleet card and start it on every loaded bot shown, after asking first"
        }
        (Plan(FleetAction::Login), true) => {
            "log in every marked bot, loading the ones not loaded yet"
        }
        (Plan(FleetAction::Login), false) => "log in every loaded bot shown, after asking first",
        (Plan(FleetAction::Logout), true) => "log out every marked bot",
        (Plan(FleetAction::Logout), false) => "log out every loaded bot shown, after asking first",
        (Plan(FleetAction::CopySettings), true) => {
            "copy the focused bot's settings for the fleet card to the marked bots on that \
             card; a confirmation names them first"
        }
        (Plan(FleetAction::CopySettings), false) => {
            "copy the focused bot's settings for the fleet card to the loaded bots shown \
             that are on that card, after asking first"
        }
        (Plan(FleetAction::SwitchWorld(_)) | SwitchWorld, true) => {
            "move every marked bot to the world you pick; a confirmation names them first"
        }
        (Plan(FleetAction::SwitchWorld(_)) | SwitchWorld, false) => {
            "move every loaded bot shown to the world you pick; a confirmation names them first"
        }
        (Walk, true) => {
            "open WalkTo for the marked bots: pick a tile and every marked bot walks there \
             from where it stands"
        }
        (Walk, false) => {
            "open WalkTo for every loaded bot shown that can walk (WalkTo marks them): pick \
             a tile and each walks there from where it stands"
        }
    }
}

/// The fold header ("Fleet" in the rail, "fleet" in the strip). A click
/// folds the toolbar to this header and its summary, or opens it again. The
/// choice is one panel preference (`fleet_toolbar_open`), apart from the
/// tile folds. Returns whether the toolbar is open.
pub(crate) fn header(ui: &Ui, session: &mut Session, label: &str) -> bool {
    let want = session.ui.fleet_toolbar_open;
    ui.set_next_item_open(want);
    let open = ui.collapsing_header(label, TreeNodeFlags::NONE);
    #[cfg(test)]
    record(|layout| layout.fold_header = item_rect(ui));
    if open != want {
        session.ui.fleet_toolbar_open = open;
        let _ = session
            .persist_panel_member(&["fleet_toolbar_open"], Some(serde_json::Value::Bool(open)));
    }
    crate::app::panel_member_feedback_line(ui, session, &["fleet_toolbar_open"]);
    open
}

/// The toolbar under its header (`open`), or the folded summary. `site`
/// names the drop-down that owns an open card search, and labels what a
/// test records.
pub(crate) fn body(ui: &Ui, session: &mut Session, site: Site, open: bool) {
    #[cfg(test)]
    {
        let left = ui.cursor_screen_pos()[0];
        let right = left + ui.content_region_avail()[0];
        record(|layout| {
            layout.site = Some(site);
            layout.content = [left, right];
        });
    }
    session.fleet_labels.sync(&session.fleet_model);
    session
        .fleet_model
        .sync_card_label(&session.scripts, &session.core);
    if open {
        card_combo(ui, session, site);
        marks_row(ui, session);
        filter_row(ui, session);
        action_bar(ui, session);
    } else {
        folded_summary(ui, session);
    }
    plan_row(ui, session);
    result_line(ui, session);
}

/// The folded toolbar's summary: "5 marked · <card>", then the filter
/// while one is set (the filter field is folded away).
fn folded_summary(ui: &Ui, session: &Session) {
    ui.text(&session.fleet_labels.folded);
    #[cfg(test)]
    let first = item_rect(ui);
    ui.same_line();
    ui.text_wrapped(session.fleet_model.card_label());
    #[cfg(test)]
    record(|layout| {
        layout.items.push(first);
        layout.items.push(item_rect(ui));
        layout.folded = Some(format!(
            "{} {}",
            session.fleet_labels.folded,
            session.fleet_model.card_label()
        ));
    });
    let filter = session.fleet_model.filter().text();
    if !filter.is_empty() {
        ui.text_disabled("filter:");
        ui.same_line();
        ui.text_wrapped(filter);
    }
}

/// Stage `card` as the fleet card and commit it. Nothing else changes: no
/// Browse pick, no assignment, no profile form, no Start.
fn pick_card(session: &mut Session, card: FleetCard) {
    session.fleet_model.stage_card(card);
    session.submit(OperatorCommand::SetFleetCard, Targets::Application);
}

/// "Card:" and its drop-down to the edge: the card the Start, Assign,
/// Assign & restart and Copy settings buttons use. Entries are each bot's
/// own card, the focused bot's card, then every Browse card under its
/// category in the Browse window's category order. While it is open a
/// search field narrows the entries. The search belongs to the site whose
/// drop-down is open: another toolbar on screen, whose drop-down is
/// closed, leaves it alone (else it would reset the search and refocus
/// the field every frame, and a click on an entry would be lost).
fn card_combo(ui: &Ui, session: &mut Session, site: Site) {
    ui.align_text_to_frame_padding();
    ui.text("Card:");
    ui.same_line();
    ui.set_next_item_width(-1.0);
    let combo = ui.begin_combo("##fleet-card", session.fleet_model.card_label());
    #[cfg(test)]
    record(|layout| {
        layout.card_combo = item_rect(ui);
        layout.items.push(item_rect(ui));
    });
    let Some(_open) = combo else {
        if session
            .fleet_card_filter
            .as_ref()
            .is_some_and(|(owner, _)| *owner == site)
        {
            session.fleet_card_filter = None;
        }
        return;
    };
    let mut search = match session.fleet_card_filter.take() {
        Some((owner, search)) if owner == site => search,
        _ => {
            ui.set_keyboard_focus_here();
            CardSearch::new()
        }
    };
    ui.set_next_item_width(-1.0);
    ui.input_text_imstr("##fleet-card-filter", &mut search.text)
        .hint("Filter cards")
        .build();
    ui.separator();
    search
        .menu
        .sync(&session.scripts, &session.ui.script_category_order);
    let picked = card_entries(ui, session, &search);
    session.fleet_card_filter = Some((site, search));
    if let Some(card) = picked {
        pick_card(session, card);
    }
}

/// The open card drop-down's search: the text its field edits in place and
/// the Browse cards it narrows, kept while the drop-down stays open and
/// dropped when it closes. A frame with the drop-down open edits, matches
/// and draws; it allocates nothing until the text, the Browse cards or the
/// category order change.
#[derive(Debug)]
pub(crate) struct CardSearch {
    /// The search text (an `ImString`: ImGui edits it in place).
    text: ImString,
    /// The Browse cards in the drop-down's order.
    menu: CardMenu,
}

impl CardSearch {
    fn new() -> Self {
        Self {
            text: ImString::with_capacity(32),
            menu: CardMenu::default(),
        }
    }
}

/// The Browse cards as the card drop-down lists them: the categories in the
/// Browse window's order and, for each card in Browse order, its category's
/// place in that order and its fleet card choice. It is built when the
/// drop-down opens and again only when a Browse card or the saved category
/// order changed, so an open drop-down's frame neither sorts the categories
/// nor builds a choice (a file card's choice reads its canonical path).
#[derive(Debug, Default)]
struct CardMenu {
    /// The saved category order `order` was resolved from.
    saved: Vec<String>,
    /// The categories of the Browse cards, in draw order.
    order: Vec<String>,
    /// One entry per Browse card, in Browse order.
    cards: Vec<CardEntry>,
}

/// One Browse card of a [`CardMenu`].
#[derive(Debug)]
struct CardEntry {
    /// The card's category, as its place in [`CardMenu::order`].
    slot: usize,
    /// The card as a fleet card choice.
    choice: ScriptChoice,
    #[cfg(feature = "js")]
    /// A loaded card's source, name and file: what its choice was built
    /// from (a compiled card's choice is its id).
    loaded: Option<(CardSource, String, PathBuf)>,
}

impl CardEntry {
    /// Whether `card` is still the card this entry was built from, under
    /// category `category` of the menu.
    fn describes(&self, card: BrowseCard<'_>, category: Option<&String>) -> bool {
        #[cfg(feature = "js")]
        let same_card = match (card.loaded(), &self.loaded) {
            (None, None) => {
                card.compiled_id().is_some() && card.compiled_id() == self.choice.compiled_id()
            }
            (Some(loaded), Some((source, name, path))) => {
                loaded.source() == *source && loaded.name() == name && loaded.path() == path
            }
            _ => false,
        };
        #[cfg(not(feature = "js"))]
        let same_card = card
            .compiled_id()
            .is_some_and(|id| Some(id) == self.choice.compiled_id());
        same_card && category.is_some_and(|known| known == display_category(card.category()))
    }
}

impl CardMenu {
    /// Build the menu again when a Browse card or `saved` (the Browse
    /// window's category order) changed since it was built.
    fn sync(&mut self, scripts: &frontend_core::Scripts, saved: &[String]) {
        let current = self.saved == saved
            && scripts.browse_cards().count() == self.cards.len()
            && scripts
                .browse_cards()
                .zip(&self.cards)
                .all(|(card, entry)| entry.describes(card, self.order.get(entry.slot)));
        if current {
            return;
        }
        let mut present: Vec<String> = Vec::new();
        for card in scripts.browse_cards() {
            let category = display_category(card.category());
            if !present.iter().any(|known| known == category) {
                present.push(category.to_owned());
            }
        }
        self.order = resolve_category_order(saved, &present);
        self.saved.clear();
        self.saved.extend_from_slice(saved);
        self.cards.clear();
        for card in scripts.browse_cards() {
            let category = display_category(card.category());
            // The resolved order holds every present category.
            let slot = match self.order.iter().position(|known| known == category) {
                Some(slot) => slot,
                None => {
                    self.order.push(category.to_owned());
                    self.order.len() - 1
                }
            };
            self.cards.push(CardEntry {
                slot,
                choice: card.selection(),
                #[cfg(feature = "js")]
                loaded: card.loaded().map(|loaded| {
                    (
                        loaded.source(),
                        loaded.name().to_owned(),
                        loaded.path().to_path_buf(),
                    )
                }),
            });
        }
    }
}

/// The open card drop-down's entries that match `search`; the one picked.
fn card_entries(ui: &Ui, session: &Session, search: &CardSearch) -> Option<FleetCard> {
    let filter = search.text.to_str();
    let current = session.fleet_model.card();
    let matches = |text: &str| filter.is_empty() || contains_ci(text, filter);
    let entry = |label: &str, selected: bool| {
        let clicked = ui.selectable_config(label).selected(selected).build();
        #[cfg(test)]
        record(|layout| layout.card_entries.push((label.to_owned(), item_rect(ui))));
        if selected {
            ui.set_item_default_focus();
        }
        clicked
    };
    let mut picked = None;
    if matches(OWN_CARD_LABEL) && entry(OWN_CARD_LABEL, current.is_own()) {
        picked = Some(FleetCard::OWN);
    }
    if matches(FOCUSED_CARD_LABEL) && entry(FOCUSED_CARD_LABEL, current.is_focused()) {
        picked = Some(FleetCard::FOCUSED);
    }
    ui.separator();
    let menu = &search.menu;
    let mut any = false;
    for (slot, category) in menu.order.iter().enumerate() {
        let mut heading = false;
        for (index, (card, card_entry)) in
            session.scripts.browse_cards().zip(&menu.cards).enumerate()
        {
            if card_entry.slot != slot || !matches(card.name()) {
                continue;
            }
            if !heading {
                heading = true;
                ui.text_disabled(category);
            }
            any = true;
            let _id = ui.push_id(index);
            let selected = current.pinned_card() == Some(&card_entry.choice);
            if entry(card.name(), selected) {
                picked = Some(FleetCard::pinned(card_entry.choice.duplicate()));
            }
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
pub(crate) fn tri_state_box(ui: &Ui, id: &str, state: ShownMarks) -> bool {
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
pub(crate) fn tri_state_op(state: ShownMarks) -> MarkOp {
    match state {
        ShownMarks::All => MarkOp::UnmarkShown,
        ShownMarks::None | ShownMarks::Some => MarkOp::Shown,
    }
}

/// The tooltip of the tri-state box in `state`.
pub(crate) fn tri_state_tooltip(state: ShownMarks) -> &'static str {
    match state {
        ShownMarks::None => "mark every shown bot",
        ShownMarks::Some => "mark the rest of the shown bots",
        ShownMarks::All => "unmark every shown bot",
    }
}

/// The marks row: the tri-state box over the shown rows, the Marks menu and
/// the count ("5 marked, 2 not shown"; filtered-out marks stay), wrapped
/// to the width left.
fn marks_row(ui: &Ui, session: &mut Session) {
    let state = session.fleet_model.shown_marks();
    let no_rows = session.fleet_model.counts().shown() == 0;
    let mut op = None;
    let mut move_to_top = false;
    {
        let _disabled = no_rows.then(|| ui.begin_disabled());
        if tri_state_box(ui, "##fleet-mark-all", state) {
            op = Some(tri_state_op(state));
        }
        #[cfg(test)]
        record(|layout| {
            layout.mark_box = item_rect(ui);
            layout.items.push(item_rect(ui));
        });
        ui.set_item_tooltip(tri_state_tooltip(state));
    }
    ui.same_line();
    if ui.button("Marks##fleet-marks-menu-button") {
        ui.open_popup(MARKS_MENU_POPUP);
    }
    #[cfg(test)]
    record(|layout| {
        layout.marks_button = item_rect(ui);
        layout.items.push(item_rect(ui));
    });
    ui.set_item_tooltip("mark or unmark the shown bots, mark them by status, or reorder them");
    if let Some(_menu) = ui.begin_popup(MARKS_MENU_POPUP) {
        let item = |label: &str| {
            let clicked = ui.menu_item(label);
            #[cfg(test)]
            record(|layout| layout.marks_menu.push((label.to_owned(), item_rect(ui))));
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
        record(|layout| {
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
        ui.separator();
        let load_order = session.fleet_model.order() == FleetOrder::LoadOrder;
        let marked = session.fleet_labels.marked > 0;
        let clicked = ui.menu_item_enabled_selected_no_shortcut(
            MOVE_TO_TOP_LABEL,
            false,
            load_order && marked,
        );
        #[cfg(test)]
        record(|layout| {
            layout
                .marks_menu
                .push((MOVE_TO_TOP_LABEL.to_owned(), item_rect(ui)));
            layout.move_to_top = item_rect(ui);
        });
        if ui.is_item_hovered_with_flags(ItemHoveredFlags::ALLOW_WHEN_DISABLED) {
            ui.tooltip_text(if !load_order {
                "choose Load order to reorder"
            } else if !marked {
                "mark profiles first"
            } else {
                "move the marked profiles to the top of the saved Load order, \
                 keeping their relative order"
            });
        }
        move_to_top |= clicked;
    }
    ui.same_line();
    ui.text_wrapped(&session.fleet_labels.marks);
    #[cfg(test)]
    record(|layout| {
        layout.marks_text = session.fleet_labels.marks.clone();
        layout.items.push(item_rect(ui));
    });
    if let Some(op) = op {
        session.submit(OperatorCommand::MarkFleet(op), Targets::Application);
    }
    if move_to_top {
        session.reorder_profiles(ReorderOp::MarkedToTop);
    }
}

/// The filter field and the sort drop-down: on one row while the field
/// keeps [`MIN_FILTER_W`], else one above the other, each to the edge.
fn filter_row(ui: &Ui, session: &mut Session) {
    let avail = ui.content_region_avail()[0];
    let spacing = ui.clone_style().item_spacing()[0];
    let sort_w = FleetOrder::CHOICES
        .into_iter()
        .map(|order| label_width(ui, order.label()))
        .fold(0.0, f32::max)
        + ui.frame_height()
        + 2.0 * ui.clone_style().frame_padding()[0];
    let one_row = avail - sort_w - spacing >= scale_px(ui, MIN_FILTER_W);
    ui.set_next_item_width(if one_row {
        avail - sort_w - spacing
    } else {
        -1.0
    });
    // The field edits a buffer the session keeps (an `ImString`: ImGui
    // edits it in place, so a frame allocates nothing). It follows the
    // model's filter while no item is being edited.
    let edit = &mut session.fleet_filter_edit;
    let committed = session.fleet_model.filter().text();
    if edit.to_str() != committed && !ui.is_any_item_active() {
        edit.clear();
        edit.push_str(committed);
    }
    let edited = ui
        .input_text_imstr("##fleet-filter", edit)
        .hint(FILTER_HINT)
        .build();
    #[cfg(test)]
    record(|layout| {
        layout.filter = item_rect(ui);
        layout.items.push(item_rect(ui));
    });
    ui.set_item_tooltip(
        "show the rows whose name, status or world (w3) holds every word; marks stay",
    );
    if edited {
        let staged = session.fleet_model.staged_filter_mut();
        staged.clear();
        staged.push_str(session.fleet_filter_edit.to_str());
        session.submit(OperatorCommand::SetFleetFilter, Targets::Application);
    }
    if one_row {
        ui.same_line();
    }
    ui.set_next_item_width(-1.0);
    let order = session.fleet_model.order();
    let combo = ui.begin_combo("##fleet-sort", order.label());
    #[cfg(test)]
    record(|layout| {
        layout.sort_combo = item_rect(ui);
        layout.items.push(item_rect(ui));
    });
    if combo.is_none() {
        ui.set_item_tooltip("sort: Load order (the saved order), Name or Status");
    }
    if let Some(_open) = combo {
        for sort in FleetOrder::CHOICES {
            let selected = sort == order;
            let picked = ui
                .selectable_config(sort.label())
                .selected(selected)
                .build();
            #[cfg(test)]
            record(|layout| layout.sort_choices.push(item_rect(ui)));
            if picked {
                session.submit(OperatorCommand::SetFleetSort(sort), Targets::Application);
            }
            if selected {
                ui.set_item_default_focus();
            }
        }
    }
}

/// Where each button of a toolbar row set goes, for buttons `widths` wide
/// (text and padding) in `avail` with `spacing` between buttons: its width
/// and whether it continues the row of the button before it. Every button
/// of the set gets the same width while `columns` of the widest fit side by
/// side (`columns` >= 2). Otherwise buttons pair up two per row while both
/// fit at half width, else one per row; each fills its share of the row.
/// No button is narrower than its text unless `avail` itself is.
pub(crate) fn button_rows<const N: usize>(
    avail: f32,
    spacing: f32,
    widths: [f32; N],
) -> [(f32, bool); N] {
    let widest = widths.iter().copied().fold(0.0, f32::max);
    let columns = (((avail + spacing) / (widest + spacing)).floor() as usize).clamp(1, N.max(1));
    let mut rows = [(avail, false); N];
    if columns >= 2 {
        let width = ((avail - spacing * (columns - 1) as f32) / columns as f32).floor();
        for (index, slot) in rows.iter_mut().enumerate() {
            *slot = (width, index % columns != 0);
        }
        return rows;
    }
    let half = ((avail - spacing) / 2.0).floor();
    let mut index = 0;
    while index < N {
        if index + 1 < N && widths[index] <= half && widths[index + 1] <= half {
            rows[index] = (half, false);
            rows[index + 1] = (half, true);
            index += 2;
        } else {
            rows[index] = (avail, false);
            index += 1;
        }
    }
    rows
}

/// The nine buttons: each names its verb and how many bots it covers. A
/// fleet action plans (see [`Session::plan_fleet_action`]); Walk opens
/// WalkTo in Group mode; Switch world opens the world picker.
fn action_bar(ui: &Ui, session: &mut Session) {
    let labels = &session.fleet_labels;
    let own = session.fleet_model.card().is_own();
    let marked = labels.marked > 0;
    let nothing = labels.scope == 0;
    let worlds = session.can_switch_world();
    let widths: [f32; ACTIONS.len()] =
        std::array::from_fn(|index| button_width(ui, &labels.actions[index]));
    let rows = button_rows(
        ui.content_region_avail()[0],
        ui.clone_style().item_spacing()[0],
        widths,
    );
    let mut clicked = None;
    for (index, action) in ACTIONS.into_iter().enumerate() {
        let label = labels.actions[index].as_str();
        let (width, same_line) = rows[index];
        if same_line {
            ui.same_line();
        }
        let needs_card = matches!(action, ToolbarAction::Plan(plan) if plan.needs_card()) && own;
        let one_world = action == ToolbarAction::SwitchWorld && !worlds;
        let tooltip = if needs_card {
            PICK_A_CARD
        } else if one_world {
            NO_WORLDS
        } else if nothing {
            NOTHING_TO_ACT_ON
        } else {
            action_tooltip(action, marked)
        };
        let disabled = needs_card || one_world || nothing;
        {
            let _disabled = disabled.then(|| ui.begin_disabled());
            if ui.button_with_size(label, [width, 0.0]) {
                clicked = Some(action);
            }
            ui.set_item_tooltip(tooltip);
        }
        #[cfg(test)]
        record(|layout| {
            layout.actions.push(ActionRecord {
                action,
                label: label.to_owned(),
                rect: item_rect(ui),
                disabled,
                text_width: widths[index],
                tooltip,
            });
            layout.items.push(item_rect(ui));
        });
    }
    match clicked {
        Some(ToolbarAction::Plan(action)) => session.plan_fleet_action(action),
        Some(ToolbarAction::Walk) => session.open_walkto_for_fleet(),
        Some(ToolbarAction::SwitchWorld) => {
            session.open_world_picker(None);
        }
        None => {}
    }
}

/// The waiting plan's confirmation row: the core's prompt, wrapped, with
/// Confirm and Cancel. Draws nothing while no plan waits. Every toolbar
/// draws it; the core takes the plan once, so a second toolbar's Confirm
/// finds nothing to run.
pub(crate) fn plan_row(ui: &Ui, session: &mut Session) {
    let Some(plan) = session.core.fleet_plan() else {
        return;
    };
    #[cfg(test)]
    crate::fleet::count_plan_row();
    ui.text_wrapped(plan.prompt());
    #[cfg(test)]
    let prompt_rect = item_rect(ui);
    let confirm = ui.button("Confirm##fleet-plan");
    #[cfg(test)]
    let confirm_rect = item_rect(ui);
    ui.same_line();
    let cancel = ui.button("Cancel##fleet-plan");
    #[cfg(test)]
    record(|layout| {
        layout.plan = Some(PlanRecord {
            prompt: plan.prompt().to_owned(),
            confirm: confirm_rect,
            cancel: item_rect(ui),
        });
        layout
            .items
            .extend([prompt_rect, confirm_rect, item_rect(ui)]);
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
    record(|layout| {
        layout.result = Some((result.text().to_owned(), item_rect(ui)));
        layout.items.push(item_rect(ui));
    });
    if ui.is_item_hovered() {
        ui.tooltip(|| {
            // `text_wrapped` would push its own wrap position (the window
            // edge) over this one, and a tooltip sizes itself to its text:
            // the edge is then zero wide and the text stacks one letter high.
            let _wrap = ui.push_text_wrap_pos(scale_px(ui, 420.0));
            ui.text(result.text());
        });
    }
}

/// Width of a label's visible text (what follows `##` is only an id).
pub(crate) fn label_width(ui: &Ui, label: &str) -> f32 {
    let visible = label.split("##").next().unwrap_or(label);
    ui.current_font()
        .calc_text_size(ui.current_font_size(), f32::MAX, 0.0, visible)[0]
}

/// A button's width: its text and the frame padding either side.
pub(crate) fn button_width(ui: &Ui, label: &str) -> f32 {
    label_width(ui, label) + 2.0 * ui.clone_style().frame_padding()[0]
}

/// The last item's `[min_x, min_y, max_x, max_y]`.
#[cfg(test)]
pub(crate) fn item_rect(ui: &Ui) -> [f32; 4] {
    let (min, max) = (ui.item_rect_min(), ui.item_rect_max());
    [min[0], min[1], max[0], max[1]]
}

#[cfg(test)]
mod tests {
    use super::{button_rows, FleetLabels};
    use frontend_core::{FleetModel, FleetRow, Phase};

    /// No label of the toolbar ends in an ellipsis, whatever the marks.
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

    /// Equal columns while the widest fits; pairs, then one per row, when
    /// it does not; never a button narrower than its text.
    #[test]
    fn buttons_share_rows_only_while_their_text_fits() {
        // Four columns of the widest fit in 400 with 8 between.
        let rows = button_rows(400.0, 8.0, [60.0, 90.0, 40.0, 70.0, 50.0]);
        assert_eq!(rows.map(|(_, same)| same), [false, true, true, true, false]);
        assert!(rows.iter().all(|(width, _)| *width == 94.0));
        // 200 wide: the widest (150) has a row of its own; the short ones pair.
        let rows = button_rows(200.0, 8.0, [60.0, 70.0, 150.0, 80.0, 90.0, 40.0]);
        assert_eq!(
            rows.map(|(_, same)| same),
            [false, true, false, false, true, false]
        );
        assert_eq!(rows[0].0, 96.0);
        assert_eq!(rows[2].0, 200.0);
        assert_eq!(rows[5].0, 200.0);
        for (width, text) in rows.iter().zip([60.0, 70.0, 150.0, 80.0, 90.0, 40.0]) {
            assert!(width.0 >= text, "{width:?} < {text}");
        }
    }
}

#[cfg(test)]
#[path = "fleet_toolbar_tests.rs"]
mod site_tests;
