//! Native loadout editor: labeled equipment slots, supply quantities, and
//! save/error feedback. Search uses generated wearpos facts; it does not
//! clone the foreign panel.

use dear_imgui_rs::{Condition, StyleVar, Ui, WindowFlags};

use crate::session::Session;
use script::{
    copy_equipment_preserving_supplies, unique_loadout_name, worn_slot_label, CarryEntry, Loadout,
    WORN_SLOT_LAYOUT,
};

const SEARCH_LIMIT: usize = 40;
const SEARCH_POPUP: &str = "##loadout-item-search";

pub fn sync_draft(session: &mut Session) {
    session.loadouts_draft = session
        .loadouts
        .loadouts()
        .get(session.loadouts_sel)
        .cloned();
    session.loadouts_search.clear();
    session.loadouts_search_slot = None;
    session.loadouts_search_supply = None;
}

pub fn save_draft(session: &mut Session) -> bool {
    let Some(draft) = session.loadouts_draft.clone() else {
        session.loadouts_status = "No loadout selected.".into();
        return false;
    };
    if draft.name.trim().is_empty() {
        session.loadouts_status = "Name is required.".into();
        return false;
    }
    let backup = session.loadouts.snapshot();
    if session.loadouts_sel < session.loadouts.loadouts().len() {
        session.loadouts.replace_at(session.loadouts_sel, draft);
    } else {
        session.loadouts.upsert(draft);
    }
    match session.loadouts.save() {
        Ok(()) => {
            session.loadouts_status = "Saved.".into();
            true
        }
        Err(err) => {
            session.loadouts.restore(backup);
            session.loadouts_status = format!("Save failed: {err}");
            false
        }
    }
}

fn add_loadout(session: &mut Session) {
    let name = unique_loadout_name(session.loadouts.loadouts(), "loadout");
    let backup = session.loadouts.snapshot();
    session.loadouts.upsert(Loadout::new(name));
    match session.loadouts.save() {
        Ok(()) => {
            session.loadouts_sel = session.loadouts.loadouts().len().saturating_sub(1);
            sync_draft(session);
            session.loadouts_status = "Saved.".into();
        }
        Err(err) => {
            session.loadouts.restore(backup);
            session.loadouts_status = format!("Save failed: {err}");
        }
    }
}

fn duplicate_loadout(session: &mut Session) {
    let Some(src) = session
        .loadouts
        .loadouts()
        .get(session.loadouts_sel)
        .cloned()
    else {
        return;
    };
    let mut dup = src.clone();
    dup.name = unique_loadout_name(session.loadouts.loadouts(), &format!("{} copy", src.name));
    let backup = session.loadouts.snapshot();
    session.loadouts.upsert(dup);
    match session.loadouts.save() {
        Ok(()) => {
            session.loadouts_sel = session.loadouts.loadouts().len().saturating_sub(1);
            sync_draft(session);
            session.loadouts_status = "Saved.".into();
        }
        Err(err) => {
            session.loadouts.restore(backup);
            session.loadouts_status = format!("Save failed: {err}");
        }
    }
}

fn delete_loadout(session: &mut Session) {
    let Some(name) = session
        .loadouts
        .loadouts()
        .get(session.loadouts_sel)
        .map(|row| row.name.clone())
    else {
        return;
    };
    let backup = session.loadouts.snapshot();
    session.loadouts.remove(&name);
    match session.loadouts.save() {
        Ok(()) => {
            if session.loadouts_sel >= session.loadouts.loadouts().len() {
                session.loadouts_sel = session.loadouts.loadouts().len().saturating_sub(1);
            }
            sync_draft(session);
            session.loadouts_status = "Saved.".into();
        }
        Err(err) => {
            session.loadouts.restore(backup);
            session.loadouts_status = format!("Save failed: {err}");
        }
    }
}

fn copy_equipment(session: &mut Session) {
    let Some(items) = session.focused_equipment_items() else {
        session.loadouts_status = "Copy current equipment needs a focused ingame character.".into();
        return;
    };
    if session.loadouts_draft.is_none() {
        session.loadouts_status = "No loadout selected.".into();
        return;
    }
    let data = session.selected_game_data();
    if let Some(draft) = session.loadouts_draft.as_mut() {
        copy_equipment_preserving_supplies(draft, &items, data.as_deref());
    }
    session.loadouts_status = "Copied current equipment into the draft. Save to keep it.".into();
}

fn assign_slot(session: &mut Session, slot: &str, item: Option<String>) {
    let two_handed = slot == "righthand"
        && item.as_deref().is_some_and(|name| {
            session.selected_game_data().is_some_and(|data| {
                data.items().iter().any(|row| {
                    row.name.as_deref() == Some(name)
                        && row.is_two_handed()
                        && !row.is_certificate()
                })
            })
        });
    let Some(draft) = session.loadouts_draft.as_mut() else {
        return;
    };
    if two_handed {
        draft.worn.remove("lefthand");
    }
    draft.set_slot(slot, item);
}

pub fn window(ui: &Ui, session: &mut Session) {
    if !session.loadouts_open {
        return;
    }
    let mut open = true;
    ui.window("Loadouts")
        .opened(&mut open)
        .flags(WindowFlags::NO_COLLAPSE)
        .size([520.0, 560.0], Condition::FirstUseEver)
        .build(|| {
            ui.text("Presets");
            let names: Vec<String> = session.loadouts.names();
            if names.is_empty() {
                ui.text_wrapped("No loadouts yet — add one below.");
            } else {
                ui.child_window("##loadout-list")
                    .size([0.0, 88.0])
                    .build(ui, || {
                        for (i, name) in names.iter().enumerate() {
                            if ui
                                .selectable_config(name)
                                .selected(i == session.loadouts_sel)
                                .build()
                            {
                                session.loadouts_sel = i;
                                sync_draft(session);
                            }
                        }
                    });
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
            if session.loadouts_draft.is_none() && !session.loadouts.loadouts().is_empty() {
                sync_draft(session);
            }
            if let Some(draft) = session.loadouts_draft.as_mut() {
                ui.input_text("Name", &mut draft.name).build();
            }
            ui.spacing();
            ui.text("Equipment");
            draw_equipment(ui, session);
            if let Some(draft) = session.loadouts_draft.as_ref() {
                if !draft.unassigned.is_empty() {
                    ui.spacing();
                    ui.text("Unassigned legacy gear");
                    ui.text_wrapped(draft.unassigned.join(", "));
                    if ui.button("Clear unassigned") {
                        if let Some(draft) = session.loadouts_draft.as_mut() {
                            draft.unassigned.clear();
                        }
                    }
                }
            }
            ui.spacing();
            ui.text("Supplies");
            draw_supplies(ui, session);
            ui.spacing();
            let can_copy = session.focused_equipment_items().is_some();
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
            if !session.loadouts_status.is_empty() {
                ui.text_wrapped(&session.loadouts_status);
            }
            search_popup(ui, session);
        });
    session.loadouts_open = open;
}

fn draw_equipment(ui: &Ui, session: &mut Session) {
    let cell_w = 150.0;
    let mut open_slot: Option<String> = None;
    for row in WORN_SLOT_LAYOUT {
        for (col, slot) in row.iter().enumerate() {
            if col > 0 {
                ui.same_line();
            }
            match slot {
                Some(slot) => {
                    let label = worn_slot_label(slot);
                    let filled = session
                        .loadouts_draft
                        .as_ref()
                        .and_then(|draft| draft.worn.get(*slot).cloned())
                        .unwrap_or_default();
                    let caption = if filled.is_empty() {
                        format!("{label}\n—")
                    } else {
                        format!("{label}\n{filled}")
                    };
                    let _pad = ui.push_style_var(StyleVar::ButtonTextAlign([0.5, 0.5]));
                    if ui.button_with_size(format!("{caption}##slot-{slot}"), [cell_w, 42.0]) {
                        open_slot = Some((*slot).to_string());
                    }
                }
                None => {
                    let _off = ui.begin_disabled_with_cond(true);
                    ui.button_with_size("##spacer", [cell_w, 42.0]);
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
    let count = session
        .loadouts_draft
        .as_ref()
        .map(|d| d.carry.len())
        .unwrap_or(0);
    for i in 0..count {
        let (item, qty) = session
            .loadouts_draft
            .as_ref()
            .map(|d| (d.carry[i].item.clone(), d.carry[i].qty))
            .unwrap_or_default();
        if ui.button_with_size(format!("{item}##supply-item-{i}"), [220.0, 0.0]) {
            open_supply = Some(i);
        }
        ui.same_line();
        let mut qty_text = qty.to_string();
        if ui
            .input_text(format!("##supply-qty-{i}"), &mut qty_text)
            .build()
        {
            if let Some(draft) = session.loadouts_draft.as_mut() {
                if let Ok(parsed) = qty_text.parse::<u32>() {
                    if parsed > 0 {
                        draft.carry[i].qty = parsed;
                    }
                }
            }
        }
        ui.same_line();
        if ui.button(format!("Remove##supply-{i}")) {
            remove = Some(i);
        }
    }
    if let Some(i) = remove {
        if let Some(draft) = session.loadouts_draft.as_mut() {
            if i < draft.carry.len() {
                draft.carry.remove(i);
            }
        }
    }
    if ui.button("Add supply") {
        if let Some(draft) = session.loadouts_draft.as_mut() {
            draft.carry.push(CarryEntry::new("Lobster", 1));
        }
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
        let heading = if let Some(slot) = session.loadouts_search_slot.as_deref() {
            format!("Search {} items", worn_slot_label(slot))
        } else {
            "Search items".into()
        };
        ui.text(heading);
        ui.input_text("##loadout-search", &mut session.loadouts_search)
            .build();
        let data = session.selected_game_data();
        let hits = if let Some(data) = data.as_ref() {
            if let Some(slot) = session.loadouts_search_slot.as_deref() {
                data.search_slot_items(slot, &session.loadouts_search, SEARCH_LIMIT)
            } else {
                data.search_named_items(&session.loadouts_search, SEARCH_LIMIT)
            }
        } else {
            Vec::new()
        };
        if data.is_none() {
            ui.text_wrapped("Item search needs a bound selected-revision profile. Type a name after closing this picker, or bind a profile.");
        } else if hits.is_empty() {
            ui.text_disabled("No matches.");
        }
        let mut picked: Option<(String, i32)> = None;
        ui.child_window("##loadout-search-hits")
            .size([0.0, 180.0])
            .build(ui, || {
                for hit in &hits {
                    let label = if hit.alias.is_empty() {
                        format!("{}  #{}", hit.name, hit.id)
                    } else {
                        format!("{}  {} #{}", hit.name, hit.alias, hit.id)
                    };
                    if ui.selectable_config(&label).build() {
                        picked = Some((hit.name.clone(), hit.id));
                    }
                }
            });
        if let Some((name, _)) = picked {
            if let Some(slot) = session.loadouts_search_slot.clone() {
                assign_slot(session, &slot, Some(name));
            } else if let Some(i) = session.loadouts_search_supply {
                if let Some(draft) = session.loadouts_draft.as_mut() {
                    if let Some(entry) = draft.carry.get_mut(i) {
                        entry.item = name;
                    }
                }
            }
            ui.close_current_popup();
        }
        if session.loadouts_search_slot.is_some() && ui.button("Clear slot") {
            if let Some(slot) = session.loadouts_search_slot.clone() {
                assign_slot(session, &slot, None);
            }
            ui.close_current_popup();
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
    use script::LoadoutsStore;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn session_with_store() -> Session {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "274bot-panel-loadouts-{n}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let mut store = LoadoutsStore::at(dir.join("loadouts.json"));
        store.upsert(Loadout::new("melee").with_carry("Lobster", 8));
        store.save().unwrap();
        let mut session = Session::new();
        session.loadouts = store;
        session.loadouts_sel = 0;
        sync_draft(&mut session);
        session
    }

    #[test]
    fn save_renames_in_place_without_dropping_supplies() {
        let mut session = session_with_store();
        session.loadouts_draft.as_mut().unwrap().name = "melee2".into();
        assert!(save_draft(&mut session));
        assert_eq!(session.loadouts.loadouts().len(), 1);
        assert_eq!(session.loadouts.loadouts()[0].name, "melee2");
        assert_eq!(
            session.loadouts.loadouts()[0].carry[0],
            CarryEntry::new("Lobster", 8)
        );
        assert_eq!(session.loadouts_status, "Saved.");
    }

    #[test]
    fn invalid_empty_name_preserves_store() {
        let mut session = session_with_store();
        session.loadouts_draft.as_mut().unwrap().name = "  ".into();
        assert!(!save_draft(&mut session));
        assert_eq!(session.loadouts.loadouts()[0].name, "melee");
        assert_eq!(session.loadouts_status, "Name is required.");
    }

    #[test]
    fn failed_save_restores_store_and_does_not_report_saved() {
        let mut session = session_with_store();
        let path = session.loadouts.loadouts();
        let _ = path;
        session.loadouts_draft.as_mut().unwrap().name = "changed".into();
        let path = {
            // Recreate the store path as a directory so the write fails.
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!(
                "274bot-panel-loadouts-fail-{n}-{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            let file = dir.join("loadouts.json");
            let mut store = LoadoutsStore::at(file.clone());
            store.upsert(Loadout::new("melee").with_carry("Lobster", 8));
            store.save().unwrap();
            std::fs::remove_file(&file).unwrap();
            std::fs::create_dir_all(&file).unwrap();
            session.loadouts = store;
            sync_draft(&mut session);
            session.loadouts_draft.as_mut().unwrap().name = "changed".into();
            file
        };
        assert!(!save_draft(&mut session));
        assert_eq!(session.loadouts.loadouts()[0].name, "melee");
        assert!(session.loadouts_status.starts_with("Save failed:"));
        assert!(!session.loadouts_status.contains("Saved."));
        let _ = path;
    }

    #[test]
    fn copy_equipment_helper_keeps_supplies() {
        let mut session = session_with_store();
        let data = api::game_data::for_revision(client::io::ClientRevision::R274).unwrap();
        copy_equipment_preserving_supplies(
            session.loadouts_draft.as_mut().unwrap(),
            &[("Rune scimitar".into(), 1333)],
            Some(&data),
        );
        let draft = session.loadouts_draft.as_ref().unwrap();
        assert_eq!(
            draft.worn.get("righthand").map(String::as_str),
            Some("Rune scimitar")
        );
        assert_eq!(draft.carry[0], CarryEntry::new("Lobster", 8));
    }
}
