//! Native loadout editor: labeled equipment slots, supply quantities, and
//! save/error feedback. Search uses generated wearpos facts; it does not
//! clone the foreign panel.
//!
//! The window edits the core's loadout form ([`frontend_core::scripts::LoadoutForm`],
//! lent to the command seam): a widget edits the staged draft in place and
//! then submits `EditLoadoutDraft`, the buttons submit the loadout rows, and
//! what a row says (`Saved.`, why a save failed) is the form's status. What
//! stays here is presentation: the item search, its picker rows and the
//! supply quantity text.

use dear_imgui_rs::{Condition, StyleVar, Ui, WindowFlags};

use crate::session::Session;
use crate::theme::{scale_px, scale_size, ui_scale};
use api::identity::Settlement;
use api::loadout::CarryEntry;
use frontend_core::command::{Completion, LoadoutField};
use frontend_core::scripts::WornSlot;
use frontend_core::{OperatorCommand, Targets};

const SEARCH_LIMIT: usize = 40;
const SEARCH_POPUP: &str = "##loadout-item-search";

/// Submit `command` for the loadout form; whether it settled `Done`. A row
/// that replaced the draft also rebuilds what the window derived from it
/// (the item search, the supply quantity text).
fn submit_row(session: &mut Session, command: OperatorCommand, targets: Targets<'_>) -> bool {
    let generation = session.loadouts.generation();
    let done = session.submit(command, targets).is_some_and(|submitted| {
        session
            .core
            .commands()
            .report(submitted.request())
            .is_some_and(|report| {
                report.members().iter().all(|member| {
                    matches!(
                        member.terminal(),
                        Some(Settlement::Completed(Completion::Done))
                    )
                })
            })
    });
    if session.loadouts.generation() != generation {
        resync_presentation(session);
    }
    done
}

/// The window's own state that follows the draft: the item search closes and
/// the quantity text is read from the supplies again.
fn resync_presentation(session: &mut Session) {
    session.loadouts_search.clear();
    session.loadouts_search_slot = None;
    session.loadouts_search_supply = None;
    resync_qty_bufs(session);
}

fn resync_qty_bufs(session: &mut Session) {
    session.loadouts_qty_bufs = session
        .loadouts
        .draft()
        .map(|draft| draft.carry.iter().map(|row| row.qty.to_string()).collect())
        .unwrap_or_default();
}

/// Select row `index` and make it the editing draft.
pub fn select(session: &mut Session, index: usize) {
    session.loadouts.stage_selection(index);
    submit_row(
        session,
        OperatorCommand::SelectLoadout,
        Targets::Application,
    );
}

/// Save the draft over the selected row.
pub fn save_draft(session: &mut Session) -> bool {
    submit_row(session, OperatorCommand::SaveLoadout, Targets::Application)
}

fn add_loadout(session: &mut Session) {
    submit_row(
        session,
        OperatorCommand::CreateLoadout,
        Targets::Application,
    );
}

fn duplicate_loadout(session: &mut Session) {
    submit_row(
        session,
        OperatorCommand::DuplicateLoadout,
        Targets::Application,
    );
}

fn delete_loadout(session: &mut Session) {
    submit_row(
        session,
        OperatorCommand::DeleteLoadout,
        Targets::Application,
    );
}

/// Copy the selected member's worn gear into the draft.
fn copy_equipment(session: &mut Session) {
    submit_row(
        session,
        OperatorCommand::CopyEquipmentToLoadout,
        Targets::Selected,
    );
}

/// Whether the selected member has equipment to copy.
fn can_copy_equipment(session: &Session) -> bool {
    session
        .core
        .selected()
        .is_some_and(|name| session.core.can_observe_equipment(name))
}

/// What the item slots and Add supply say while there is nothing to edit.
const NEEDS_A_LOADOUT: &str = "add a loadout first";

/// Whether the window has a loadout draft to edit. The controls that start an
/// edit draw disabled without one (the equipment slot buttons and Add supply,
/// the only ways into the item search), so a click never reaches the early
/// returns of [`assign_slot`] and the supply handlers for want of a draft.
fn has_draft(session: &Session) -> bool {
    session.loadouts.draft().is_some()
}

fn assign_slot(session: &mut Session, slot: WornSlot, item: Option<String>) {
    let two_handed = slot == WornSlot::RightHand
        && item.as_deref().is_some_and(|name| {
            session.selected_game_data().is_some_and(|data| {
                data.items().iter().any(|row| {
                    row.name.as_deref() == Some(name)
                        && row.is_two_handed()
                        && !row.is_certificate()
                })
            })
        });
    // The slot buttons that open the item search are disabled without a draft
    // (`has_draft`), so a click always has one: this only obtains it.
    let Some(draft) = session.loadouts.draft_mut() else {
        return;
    };
    if two_handed {
        draft.worn.remove(WornSlot::LeftHand.id());
    }
    draft.set_slot(slot.id(), item);
    submit_row(
        session,
        OperatorCommand::EditLoadoutDraft(LoadoutField::Equipment),
        Targets::Application,
    );
}

/// Name the item of supply row `index`; `false` when there is no such row.
/// The search that calls this opens from a supply button, which is drawn only
/// for a row of the current draft, and a click anywhere outside the search
/// closes it first, so the row is still there when an item is picked.
fn set_supply_item(session: &mut Session, index: usize, item: String) -> bool {
    let Some(entry) = session
        .loadouts
        .draft_mut()
        .and_then(|draft| draft.carry.get_mut(index))
    else {
        return false;
    };
    entry.item = item;
    submit_row(
        session,
        OperatorCommand::EditLoadoutDraft(LoadoutField::Supplies),
        Targets::Application,
    );
    true
}

/// Stable ImGui id for a layout spacer cell (must be unique across the grid).
pub fn equipment_spacer_id(row: usize, col: usize) -> String {
    format!("##spacer-{row}-{col}")
}

/// Apply a typed quantity buffer to one supply row. Empty/invalid text does not
/// clobber the stored quantity; only positive parses commit.
pub fn apply_qty_text(qty_text: &str, current: u32) -> u32 {
    match qty_text.trim().parse::<u32>() {
        Ok(parsed) if parsed > 0 => parsed,
        _ => current,
    }
}

/// Commit a free-typed item name into the active slot or supply row.
pub fn apply_manual_item_name(session: &mut Session, name: &str) -> bool {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return false;
    }
    if let Some(slot) = session.loadouts_search_slot {
        assign_slot(session, slot, Some(trimmed.to_string()));
        true
    } else if let Some(index) = session.loadouts_search_supply {
        set_supply_item(session, index, trimmed.to_string())
    } else {
        false
    }
}

pub fn window(ui: &Ui, session: &mut Session) {
    if !session.loadouts_open {
        return;
    }
    let mut open = true;
    let viewport = ui.main_viewport();
    let position = viewport.pos();
    let scale = ui_scale(ui);
    let height = scale_px(ui, (viewport.size()[1] / scale - 40.0).clamp(240.0, 560.0));
    crate::app::focus_if_requested(ui, session, crate::session::PanelWindow::Loadouts);
    ui.window(crate::session::PanelWindow::Loadouts.title())
        .opened(&mut open)
        .flags(WindowFlags::NO_COLLAPSE)
        .position(
            [
                position[0] + scale_px(ui, 20.0),
                position[1] + scale_px(ui, 20.0),
            ],
            Condition::FirstUseEver,
        )
        .size(
            scale_size(ui, [520.0, height / scale]),
            Condition::FirstUseEver,
        )
        .build(|| {
            ui.text("Presets");
            if session.loadouts.loadouts().is_empty() {
                ui.text_wrapped("No loadouts yet — add one below.");
            } else {
                let mut clicked = None;
                ui.child_window("##loadout-list")
                    .size([0.0, scale_px(ui, 88.0)])
                    .build(ui, || {
                        for (i, row) in session.loadouts.loadouts().iter().enumerate() {
                            if ui
                                .selectable_config(&row.name)
                                .selected(i == session.loadouts.selected())
                                .build()
                            {
                                clicked = Some(i);
                            }
                        }
                    });
                if let Some(i) = clicked {
                    select(session, i);
                }
            }
            if ui.button("New") {
                add_loadout(session);
            }
            ui.same_line();
            if ui.button("Duplicate") {
                duplicate_loadout(session);
            }
            ui.same_line();
            if ui.button("Delete") {
                delete_loadout(session);
            }
            ui.spacing();
            let rows = session.loadouts.loadouts().len();
            if session.loadouts.draft().is_none() && rows > 0 {
                let index = session.loadouts.selected().min(rows - 1);
                select(session, index);
            }
            let name_edited = session
                .loadouts
                .draft_mut()
                .is_some_and(|draft| ui.input_text("Name", &mut draft.name).build());
            if name_edited {
                submit_row(
                    session,
                    OperatorCommand::EditLoadoutDraft(LoadoutField::Name),
                    Targets::Application,
                );
            }
            ui.spacing();
            ui.text("Equipment");
            draw_equipment(ui, session);
            let mut clear_unassigned = false;
            if let Some(draft) = session.loadouts.draft() {
                if !draft.unassigned.is_empty() {
                    ui.spacing();
                    ui.text("Unassigned legacy gear");
                    ui.text_wrapped(draft.unassigned.join(", "));
                    clear_unassigned = ui.button("Clear unassigned");
                }
            }
            if clear_unassigned {
                if let Some(draft) = session.loadouts.draft_mut() {
                    draft.unassigned.clear();
                }
                submit_row(
                    session,
                    OperatorCommand::EditLoadoutDraft(LoadoutField::Unassigned),
                    Targets::Application,
                );
            }
            ui.spacing();
            ui.text("Supplies");
            draw_supplies(ui, session);
            ui.spacing();
            let can_copy = can_copy_equipment(session);
            {
                let _off = ui.begin_disabled_with_cond(!can_copy);
                if ui.button("Copy current equipment") {
                    copy_equipment(session);
                }
            }
            if !can_copy {
                ui.same_line();
                ui.text_disabled("(needs a focused ingame character)");
            }
            ui.same_line();
            if ui.button("Save") {
                save_draft(session);
            }
            if !session.loadouts.status().is_empty() {
                ui.text_wrapped(session.loadouts.status());
            }
            search_popup(ui, session);
        });
    session.loadouts_open = open;
}

fn draw_equipment(ui: &Ui, session: &mut Session) {
    let cell_w = scale_px(ui, 150.0);
    let mut open_slot: Option<WornSlot> = None;
    let editable = has_draft(session);
    for (row_i, row) in WornSlot::layout().iter().enumerate() {
        for (col, cell) in row.iter().enumerate() {
            if col > 0 {
                ui.same_line();
            }
            match cell {
                Some(slot) => {
                    let label = slot.label();
                    let filled = session
                        .loadouts
                        .draft()
                        .and_then(|draft| draft.worn.get(slot.id()))
                        .map(String::as_str)
                        .unwrap_or_default();
                    let caption = if filled.is_empty() {
                        format!("{label}\n—")
                    } else {
                        format!("{label}\n{filled}")
                    };
                    let _pad = ui.push_style_var(StyleVar::ButtonTextAlign([0.5, 0.5]));
                    let _off = (!editable).then(|| ui.begin_disabled());
                    if ui.button_with_size(
                        format!("{caption}##slot-{}", slot.id()),
                        [cell_w, scale_px(ui, 42.0)],
                    ) {
                        open_slot = Some(*slot);
                    }
                    if !editable {
                        ui.set_item_tooltip(NEEDS_A_LOADOUT);
                    }
                }
                None => {
                    let _off = ui.begin_disabled_with_cond(true);
                    ui.button_with_size(
                        equipment_spacer_id(row_i, col),
                        [cell_w, scale_px(ui, 42.0)],
                    );
                }
            }
        }
    }
    if let Some(slot) = open_slot {
        session.loadouts_search_slot = Some(slot);
        session.loadouts_search_supply = None;
        session.loadouts_search.clear();
        ui.open_popup(SEARCH_POPUP);
    }
}

fn draw_supplies(ui: &Ui, session: &mut Session) {
    let mut remove = None;
    let mut open_supply = None;
    let mut edited = false;
    let editable = has_draft(session);
    let count = session.loadouts.draft().map_or(0, |d| d.carry.len());
    if session.loadouts_qty_bufs.len() != count {
        resync_qty_bufs(session);
    }
    for i in 0..count {
        let item = session
            .loadouts
            .draft()
            .map(|d| d.carry[i].item.clone())
            .unwrap_or_default();
        if ui.button_with_size(
            format!("{item}##supply-item-{i}"),
            [scale_px(ui, 220.0), 0.0],
        ) {
            open_supply = Some(i);
        }
        ui.same_line();
        if session.loadouts_qty_bufs.len() <= i {
            resync_qty_bufs(session);
        }
        let qty_buf = session
            .loadouts_qty_bufs
            .get_mut(i)
            .expect("qty bufs resynced");
        ui.set_next_item_width(scale_px(ui, 72.0));
        if ui.input_text(format!("##supply-qty-{i}"), qty_buf).build() {
            // Keep the typed buffer; only positive parses commit.
            if let Some(entry) = session
                .loadouts
                .draft_mut()
                .and_then(|draft| draft.carry.get_mut(i))
            {
                let typed = apply_qty_text(qty_buf, entry.qty);
                if typed != entry.qty {
                    entry.qty = typed;
                    edited = true;
                }
            }
        }
        ui.same_line();
        if ui.button(format!("Remove##supply-{i}")) {
            remove = Some(i);
        }
    }
    if let Some(i) = remove {
        if let Some(draft) = session.loadouts.draft_mut() {
            if i < draft.carry.len() {
                draft.carry.remove(i);
                edited = true;
            }
        }
        if i < session.loadouts_qty_bufs.len() {
            session.loadouts_qty_bufs.remove(i);
        }
    }
    {
        let _off = (!editable).then(|| ui.begin_disabled());
        if ui.button("Add supply") {
            // Disabled without a draft, so a click always has one.
            if let Some(draft) = session.loadouts.draft_mut() {
                draft.carry.push(CarryEntry::new("Lobster", 1));
                session.loadouts_qty_bufs.push("1".into());
                edited = true;
            }
        }
        if !editable {
            ui.set_item_tooltip(NEEDS_A_LOADOUT);
        }
    }
    if edited {
        submit_row(
            session,
            OperatorCommand::EditLoadoutDraft(LoadoutField::Supplies),
            Targets::Application,
        );
    }
    if let Some(i) = open_supply {
        session.loadouts_search_supply = Some(i);
        session.loadouts_search_slot = None;
        session.loadouts_search.clear();
        ui.open_popup(SEARCH_POPUP);
    }
}

fn search_popup(ui: &Ui, session: &mut Session) {
    ui.popup(SEARCH_POPUP, || {
        let heading = if let Some(slot) = session.loadouts_search_slot {
            format!("Search {} items", slot.label())
        } else {
            "Search items".into()
        };
        ui.text(heading);
        ui.set_next_item_width(scale_px(ui, 460.0));
        ui.input_text("##loadout-search", &mut session.loadouts_search)
            .build();
        let data = session.selected_game_data();
        let data_key = data.as_ref().map(std::sync::Arc::downgrade);
        let cache_matches = session
            .loadouts_search_cache_key
            .as_ref()
            .is_some_and(|(cached_data, cached_query, cached_slot)| {
                let same_data = match (cached_data, &data_key) {
                    (Some(cached), Some(current)) => std::sync::Weak::ptr_eq(cached, current),
                    (None, None) => true,
                    _ => false,
                };
                same_data
                    && cached_query == &session.loadouts_search
                    && *cached_slot == session.loadouts_search_slot
            });
        if !cache_matches {
            let hits = data
                .as_ref()
                .map(|data| {
                    if let Some(slot) = session.loadouts_search_slot {
                        data.search_slot_items(slot.id(), &session.loadouts_search, SEARCH_LIMIT)
                    } else {
                        data.search_named_items(&session.loadouts_search, SEARCH_LIMIT)
                    }
                })
                .unwrap_or_default();
            session.loadouts_search_cache_key = Some((
                data_key,
                session.loadouts_search.clone(),
                session.loadouts_search_slot,
            ));
            session.loadouts_search_hits = hits;
            session.loadouts_picker_rows.invalidate();
        }
        session
            .loadouts_picker_rows
            .ensure(ui, &session.loadouts_search_hits);
        let hits_empty = session.loadouts_search_hits.is_empty();
        if data.is_none() {
            ui.text_wrapped(
                "Item catalog is unavailable for this profile's cache identity. Enter an exact item display name above, then use it below. Bind a profile whose cache matches the generated facts to enable search.",
            );
            let can_use = !session.loadouts_search.trim().is_empty();
            {
                let _off = ui.begin_disabled_with_cond(!can_use);
                if ui.button("Use entered name")
                    && apply_manual_item_name(session, &session.loadouts_search.clone())
                {
                    ui.close_current_popup();
                }
            }
        } else if hits_empty {
            ui.text_disabled("No matches.");
        }
        let hits = &session.loadouts_search_hits;
        // Shared name-hit rows with the Debug picker: the fixed Loadouts width
        // is unchanged, while strings and text measurements are cached.
        let picked_row: Option<usize> = ui
            .child_window("##loadout-search-hits")
            .size([scale_px(ui, 460.0), scale_px(ui, 180.0)])
            .build(ui, || {
                crate::name_picker::draw_hit_rows(
                    ui,
                    hits,
                    &mut session.loadouts_picker_rows,
                    None,
                )
            })
            .flatten();
        let mut picked: Option<(String, i32)> = None;
        if let Some(row) = picked_row {
            if let Some(hit) = hits.get(row) {
                picked = Some((hit.name.clone(), hit.id));
            }
        }
        if let Some((name, _)) = picked {
            if let Some(slot) = session.loadouts_search_slot {
                assign_slot(session, slot, Some(name));
            } else if let Some(index) = session.loadouts_search_supply {
                set_supply_item(session, index, name);
            }
            ui.close_current_popup();
        }
        if let Some(slot) = session.loadouts_search_slot {
            if ui.button("Clear slot") {
                assign_slot(session, slot, None);
                ui.close_current_popup();
            }
        }
        if ui.button("Close") {
            ui.close_current_popup();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::Session;
    use crate::test_support::{
        reload_form_until, save_draft_patiently, save_store_patiently, scratch_service,
        thread_state_root, TestDir,
    };
    use api::loadout::Loadout;
    use frontend_core::scripts::{LoadoutForm, Loadouts};
    use std::collections::HashSet;
    use std::path::Path;

    /// The rows of a service opened on `dir`, a scratch state root (another
    /// such list on the same `dir` is another writer).
    fn store_in(dir: &Path) -> Loadouts {
        Loadouts::operator(&scratch_service(dir))
    }

    /// A session whose Loadouts form holds `rows`, saved in a scratch root,
    /// with the first selected.
    fn session_with(rows: Vec<Loadout>) -> (Session, TestDir) {
        let dir = TestDir::new("loadouts");
        let mut store = store_in(&dir);
        for row in rows {
            store.upsert_for_test(row);
        }
        save_store_patiently(&mut store);
        let mut session = Session::new();
        session.loadouts = LoadoutForm::over(store);
        select(&mut session, 0);
        (session, dir)
    }

    fn session_with_store() -> (Session, TestDir) {
        session_with(vec![Loadout::new("melee").with_carry("Lobster", 8)])
    }

    /// Select the row called `name`: what the saved row's selection did
    /// before a load-slow write was waited out.
    fn select_named(session: &mut Session, name: &str) {
        let index = session
            .loadouts
            .names()
            .iter()
            .position(|row| row == name)
            .unwrap();
        select(session, index);
    }

    /// Whether `command` was submitted and settled `Done`.
    fn settled_done(session: &Session, command: OperatorCommand) -> bool {
        session
            .core
            .commands()
            .reports()
            .filter(|report| report.command() == command)
            .any(|report| {
                report.members().iter().all(|member| {
                    matches!(
                        member.terminal(),
                        Some(Settlement::Completed(Completion::Done))
                    )
                })
            })
    }

    #[test]
    fn save_renames_in_place_without_dropping_supplies() {
        let (mut session, _dir) = session_with_store();
        session.loadouts.draft_mut().unwrap().name = "melee2".into();
        assert!(save_draft_patiently(&mut session));
        assert_eq!(session.loadouts.loadouts().len(), 1);
        assert_eq!(session.loadouts.loadouts()[0].name, "melee2");
        assert_eq!(
            session.loadouts.loadouts()[0].carry[0],
            CarryEntry::new("Lobster", 8)
        );
        assert_eq!(session.loadouts.status(), "Saved.");
    }

    #[test]
    fn invalid_empty_name_preserves_store() {
        let (mut session, _dir) = session_with_store();
        session.loadouts.draft_mut().unwrap().name = "  ".into();
        assert!(!save_draft(&mut session));
        assert_eq!(session.loadouts.loadouts()[0].name, "melee");
        assert_eq!(session.loadouts.status(), "Name is required.");
    }

    /// Rows are keyed by name in the store: renaming a draft onto another
    /// row's name is refused, not a silent replacement of that row.
    #[test]
    fn renaming_onto_another_loadouts_name_is_refused_and_keeps_both() {
        let (mut session, dir) = session_with(vec![
            Loadout::new("melee").with_carry("Lobster", 8),
            Loadout::new("range").with_carry("Shark", 3),
        ]);
        select(&mut session, 1);
        session.loadouts.draft_mut().unwrap().name = "melee".into();
        assert!(!save_draft(&mut session));
        assert_eq!(
            session.loadouts.status(),
            "A loadout named \"melee\" already exists."
        );
        assert_eq!(session.loadouts.names(), ["melee", "range"]);
        assert_eq!(
            session.loadouts.get("melee").unwrap().carry[0],
            CarryEntry::new("Lobster", 8)
        );
        assert_eq!(session.loadouts.selected(), 1);
        assert_eq!(store_in(&dir).names(), ["melee", "range"]);
    }

    #[test]
    fn failed_save_restores_store_and_does_not_report_saved() {
        let (mut session, _dir) = session_with_store();
        session.loadouts.draft_mut().unwrap().name = "changed".into();
        // Recreate the store path as a directory so the write fails.
        let fail_dir = TestDir::new("loadouts-fail");
        let service = scratch_service(&fail_dir);
        let file = service.root().loadouts_file();
        let mut store = Loadouts::operator(&service);
        store.upsert_for_test(Loadout::new("melee").with_carry("Lobster", 8));
        save_store_patiently(&mut store);
        std::fs::remove_file(&file).unwrap();
        std::fs::create_dir_all(&file).unwrap();
        session.loadouts = LoadoutForm::over(store);
        select(&mut session, 0);
        session.loadouts.draft_mut().unwrap().name = "changed".into();
        assert!(!save_draft(&mut session));
        assert_eq!(session.loadouts.loadouts()[0].name, "melee");
        assert!(
            !session.loadouts.store().is_dirty(),
            "the failed edit is dropped"
        );
        assert_eq!(session.loadouts.selected(), 0);
        assert_eq!(
            session.loadouts.draft().unwrap().name,
            "changed",
            "the operator's draft stays for another try"
        );
        assert!(session.loadouts.status().starts_with("Save failed:"));
        assert!(!session.loadouts.status().contains("Saved."));
        drop(fail_dir);
    }

    /// A rename replaces the row where it stands (T5.2 r4): the saved row
    /// keeps its index, in the form and in the file, and the form follows
    /// it by name.
    #[test]
    fn renamed_row_keeps_its_place_and_stays_selected_by_name() {
        let (mut session, dir) = session_with(vec![
            Loadout::new("melee").with_carry("Lobster", 8),
            Loadout::new("range"),
            Loadout::new("tank"),
        ]);
        session.loadouts.draft_mut().unwrap().name = "melee2".into();
        assert!(save_draft_patiently(&mut session));
        assert_eq!(session.loadouts.names(), ["melee2", "range", "tank"]);
        assert_eq!(session.loadouts.selected(), 0);
        let draft = session.loadouts.draft().unwrap();
        assert_eq!(draft.name, "melee2");
        assert_eq!(draft.carry[0], CarryEntry::new("Lobster", 8));
        assert_eq!(store_in(&dir).names(), ["melee2", "range", "tank"]);

        // The next save writes the renamed row, not another one.
        session.loadouts.draft_mut().unwrap().carry[0].qty = 9;
        assert!(save_draft_patiently(&mut session));
        assert_eq!(session.loadouts.names(), ["melee2", "range", "tank"]);
        assert_eq!(session.loadouts.get("melee2").unwrap().carry[0].qty, 9);
        assert!(session.loadouts.get("range").unwrap().carry.is_empty());
    }

    #[test]
    fn new_and_duplicate_select_the_saved_row_by_name() {
        let (mut session, _dir) = session_with(vec![
            Loadout::new("melee").with_carry("Lobster", 8),
            Loadout::new("range"),
        ]);
        add_loadout(&mut session);
        if session.loadouts.status().contains("not durable within") {
            // Load-slow durability: the queued add lands on its own, and a
            // retry could double-add, so wait for it and finish the
            // selection the success path does.
            reload_form_until(&mut session.loadouts, "added loadout", |form| {
                form.names().contains(&"loadout".to_string())
            });
            select_named(&mut session, "loadout");
        } else {
            assert_eq!(session.loadouts.status(), "Saved.");
        }

        select(&mut session, 0);
        duplicate_loadout(&mut session);
        if session.loadouts.status().contains("not durable within") {
            // As above: wait out the queued duplicate instead of retrying.
            reload_form_until(&mut session.loadouts, "duplicated loadout", |form| {
                form.names().contains(&"melee copy".to_string())
            });
            select_named(&mut session, "melee copy");
        }
        assert_eq!(
            session.loadouts.names(),
            ["melee", "range", "loadout", "melee copy"]
        );
        assert_eq!(session.loadouts.selected(), 3);
        let draft = session.loadouts.draft().unwrap();
        assert_eq!(draft.name, "melee copy");
        assert_eq!(draft.carry[0], CarryEntry::new("Lobster", 8));
    }

    /// Another writer changed the loadouts after this form loaded them: the
    /// save is refused, nothing of the other writer is overwritten, the form
    /// shows what the store holds now, and the selection stays on its row
    /// (found by name: that row's index moved).
    #[test]
    fn a_conflicting_save_is_refused_and_the_selection_follows_its_row() {
        let (mut session, dir) = session_with(vec![
            Loadout::new("first"),
            Loadout::new("melee").with_carry("Lobster", 8),
        ]);
        select(&mut session, 1);
        let mut other = store_in(&dir);
        other.remove_for_test("first");
        other.upsert_for_test(Loadout::new("tank"));
        save_store_patiently(&mut other);

        session.loadouts.draft_mut().unwrap().name = "melee2".into();
        // A load-slow worker answers with the bounded-wait timeout instead
        // of the conflict: retry those, keep the first decisive answer.
        let mut saved = save_draft(&mut session);
        let start = std::time::Instant::now();
        while !saved
            && session.loadouts.status().contains("not durable within")
            && start.elapsed() < std::time::Duration::from_secs(60)
        {
            saved = save_draft(&mut session);
        }
        assert!(!saved);
        assert!(
            session.loadouts.status().starts_with("Save failed:"),
            "got {:?}",
            session.loadouts.status()
        );
        assert_eq!(session.loadouts.names(), ["melee", "tank"]);
        assert!(!session.loadouts.store().is_dirty());
        assert_eq!(session.loadouts.selected(), 0, "melee is now the first row");
        assert_eq!(
            session.loadouts.draft().unwrap().name,
            "melee2",
            "the operator's draft stays for another try"
        );
        assert_eq!(store_in(&dir).names(), ["melee", "tank"]);
    }

    /// A file the service refused leaves the form empty with its reason as
    /// the status, and no save writes over it.
    #[test]
    fn unreadable_loadouts_file_is_named_in_the_status_and_never_written_over() {
        let _iso = frontend_core::test_support::IsolatedStores::enter("loadouts-refused");
        let file = thread_state_root().loadouts_file();
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        let corrupt = b"{ not valid json";
        std::fs::write(&file, corrupt).unwrap();

        let mut session = Session::new();
        let error = session
            .loadouts
            .load_error()
            .expect("the refused file is named")
            .to_string();
        assert!(
            session.loadouts.status().contains(&error),
            "the status carries the reason: {:?}",
            session.loadouts.status()
        );
        assert!(session.loadouts.loadouts().is_empty());

        add_loadout(&mut session);
        assert!(
            session.loadouts.status().starts_with("Save failed:"),
            "got {:?}",
            session.loadouts.status()
        );
        assert!(
            session.loadouts.status().contains("did not load"),
            "the status names the failed load: {:?}",
            session.loadouts.status()
        );
        assert!(session.loadouts.loadouts().is_empty());
        assert!(
            !session.loadouts.store().is_dirty(),
            "the failed form is discarded, as for other failed saves"
        );
        assert_eq!(std::fs::read(&file).unwrap(), corrupt);
    }

    #[test]
    fn equipment_spacer_ids_are_unique_across_layout() {
        let mut ids = HashSet::new();
        for (row_i, row) in WornSlot::layout().iter().enumerate() {
            for (col, slot) in row.iter().enumerate() {
                if slot.is_none() {
                    assert!(ids.insert(equipment_spacer_id(row_i, col)));
                }
            }
        }
    }

    #[test]
    fn qty_text_keeps_prior_value_on_empty_or_invalid() {
        assert_eq!(apply_qty_text("", 8), 8);
        assert_eq!(apply_qty_text("  ", 8), 8);
        assert_eq!(apply_qty_text("0", 8), 8);
        assert_eq!(apply_qty_text("abc", 8), 8);
        assert_eq!(apply_qty_text("12", 8), 12);
    }

    #[test]
    fn qty_bufs_survive_empty_intermediate_edit() {
        let (mut session, _dir) = session_with_store();
        assert_eq!(session.loadouts_qty_bufs, vec!["8".to_string()]);
        session.loadouts_qty_bufs[0] = String::new();
        let current = session.loadouts.draft().unwrap().carry[0].qty;
        let next = apply_qty_text(&session.loadouts_qty_bufs[0], current);
        assert_eq!(next, 8);
        assert_eq!(session.loadouts_qty_bufs[0], "");
        session.loadouts_qty_bufs[0] = "12".into();
        let next = apply_qty_text(&session.loadouts_qty_bufs[0], current);
        assert_eq!(next, 12);
        session.loadouts.draft_mut().unwrap().carry[0].qty = next;
        assert_eq!(session.loadouts.draft().unwrap().carry[0].qty, 12);
    }

    #[test]
    fn manual_item_name_assigns_slot_and_supply_without_catalog() {
        let (mut session, _dir) = session_with_store();
        session.loadouts_search_slot = Some(WornSlot::Hat);
        assert!(apply_manual_item_name(&mut session, "Rune full helm"));
        assert_eq!(
            session
                .loadouts
                .draft()
                .unwrap()
                .worn
                .get("hat")
                .map(String::as_str),
            Some("Rune full helm")
        );
        session.loadouts_search_slot = None;
        session.loadouts_search_supply = Some(0);
        assert!(apply_manual_item_name(&mut session, "Shark"));
        assert_eq!(session.loadouts.draft().unwrap().carry[0].item, "Shark");
        assert!(!apply_manual_item_name(&mut session, "  "));
    }

    #[test]
    fn duplicate_preserves_source_and_selects_copy() {
        let (mut session, _dir) = session_with_store();
        duplicate_loadout(&mut session);
        if session.loadouts.status().contains("not durable within") {
            // As above: wait out the queued duplicate instead of retrying.
            reload_form_until(&mut session.loadouts, "duplicated loadout", |form| {
                form.names().contains(&"melee copy".to_string())
            });
            select_named(&mut session, "melee copy");
        }
        assert_eq!(session.loadouts.loadouts().len(), 2);
        assert_eq!(session.loadouts.loadouts()[0].name, "melee");
        assert_eq!(session.loadouts.loadouts()[1].name, "melee copy");
        assert_eq!(session.loadouts.selected(), 1);
        assert_eq!(
            session.loadouts.draft().unwrap().carry[0],
            CarryEntry::new("Lobster", 8)
        );
    }

    /// The window's actions are rows of the command seam: each leaves its
    /// settled report in the core's book, and a widget edit commits through
    /// its own row.
    #[test]
    fn the_window_actions_are_rows_of_the_command_seam() {
        let (mut session, _dir) = session_with_store();
        assert!(settled_done(&session, OperatorCommand::SelectLoadout));

        session.loadouts.draft_mut().unwrap().name = "melee2".into();
        assert!(submit_row(
            &mut session,
            OperatorCommand::EditLoadoutDraft(LoadoutField::Name),
            Targets::Application,
        ));
        assert!(settled_done(
            &session,
            OperatorCommand::EditLoadoutDraft(LoadoutField::Name)
        ));
        assert!(save_draft_patiently(&mut session));
        assert!(settled_done(&session, OperatorCommand::SaveLoadout));
        assert_eq!(session.loadouts.status(), "Saved.");
    }

    /// A row that cannot act reports why instead of settling Done: nothing
    /// is selected in a form with no rows, so Duplicate and Delete skip and
    /// a save has no draft to commit.
    #[test]
    fn rows_with_nothing_to_act_on_do_not_settle_done() {
        let (mut session, _dir) = session_with(Vec::new());
        assert!(session.loadouts.draft().is_none());
        assert!(!submit_row(
            &mut session,
            OperatorCommand::DuplicateLoadout,
            Targets::Application,
        ));
        assert!(!submit_row(
            &mut session,
            OperatorCommand::DeleteLoadout,
            Targets::Application,
        ));
        assert!(!save_draft(&mut session));
        assert_eq!(session.loadouts.status(), "No loadout selected.");
        assert!(session.loadouts.loadouts().is_empty());
    }
}
