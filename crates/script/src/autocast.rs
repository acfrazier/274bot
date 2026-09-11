//! Selected-world autocast control facts. JS keeps the frozen Autocast ABI
//! and the 3000 ms three-press sequence. Rust owns the packed choose/grid/
//! toggle identities so the shim never hardcodes 328/353/1829/349/108.

use api::game_data::{AutocastControls, SelectedGameData};
use serde_json::{json, Value};
use std::cell::Cell;

thread_local! {
    static TOKEN: Cell<u64> = const { Cell::new(0) };
}

pub fn on_pause() {}
pub fn on_resume() {}
pub fn on_hold(_held: bool) {}

pub fn on_reset() {
    TOKEN.with(|token| token.set(token.get().wrapping_add(1)));
}

fn bump_token() -> u64 {
    TOKEN.with(|token| {
        let next = token.get().wrapping_add(1);
        token.set(next);
        next
    })
}

fn current_token() -> u64 {
    TOKEN.with(Cell::get)
}

pub fn controls_json(data: Option<&SelectedGameData>) -> Value {
    match data.and_then(SelectedGameData::autocast_controls) {
        Some(controls) => json_controls(controls, true),
        None => json_controls(
            &AutocastControls {
                staff_tab_root: -1,
                spell_panel_root: -1,
                choose_com: -1,
                toggle_com: -1,
                spell_grid_base: -1,
                magic_varp: -1,
                selected_value: 2,
                armed_value: 3,
            },
            false,
        ),
    }
}

fn json_controls(controls: &AutocastControls, available: bool) -> Value {
    json!({
        "available": available && controls.available(),
        "staff_tab_root": controls.staff_tab_root,
        "spell_panel_root": controls.spell_panel_root,
        "choose_com": controls.choose_com,
        "toggle_com": controls.toggle_com,
        "spell_grid_base": controls.spell_grid_base,
        "magic_varp": controls.magic_varp,
        "selected_value": controls.selected_value,
        "armed_value": controls.armed_value,
    })
}

pub fn observe(data: Option<&SelectedGameData>, combat_tab_root: i32, magic_varp: i32) -> Value {
    let mut out = controls_json(data);
    let staff = out
        .get("staff_tab_root")
        .and_then(Value::as_i64)
        .unwrap_or(-1);
    let panel = out
        .get("spell_panel_root")
        .and_then(Value::as_i64)
        .unwrap_or(-1);
    let selected = out
        .get("selected_value")
        .and_then(Value::as_i64)
        .unwrap_or(2);
    let armed = out.get("armed_value").and_then(Value::as_i64).unwrap_or(3);
    let available = out
        .get("available")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    out["staff_attached"] = json!(available && staff >= 0 && i64::from(combat_tab_root) == staff);
    out["panel_open"] = json!(available && panel >= 0 && i64::from(combat_tab_root) == panel);
    out["selected"] = json!(available && i64::from(magic_varp) == selected);
    out["armed"] = json!(available && i64::from(magic_varp) == armed);
    out["combat_tab_root"] = json!(combat_tab_root);
    out["magic_varp_value"] = json!(magic_varp);
    out
}

pub fn dispatch(data: Option<&SelectedGameData>, input: &Value) -> Value {
    match input
        .get("op")
        .and_then(Value::as_str)
        .unwrap_or("controls")
    {
        "observe" => observe(
            data,
            input
                .get("combat_tab_root")
                .and_then(Value::as_i64)
                .unwrap_or(-1) as i32,
            input
                .get("magic_varp_value")
                .and_then(Value::as_i64)
                .unwrap_or(0) as i32,
        ),
        "begin" => json!({ "token": bump_token() }),
        "current_token" => json!(current_token()),
        _ => controls_json(data),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use client::io::ClientRevision;

    fn data(rev: ClientRevision) -> std::sync::Arc<SelectedGameData> {
        api::game_data::for_revision(rev).expect("selected data")
    }

    #[test]
    fn both_selected_caches_post_the_audited_staff_spell_controls() {
        for rev in [ClientRevision::R274, ClientRevision::R289] {
            let data = data(rev);
            let controls = data
                .autocast_controls()
                .expect("generated autocast controls");
            assert_eq!(controls.staff_tab_root, 328);
            assert_eq!(controls.choose_com, 353);
            assert_eq!(controls.spell_panel_root, 1829);
            assert_eq!(controls.spell_grid_base, 1830);
            assert_eq!(controls.toggle_com, 349);
            assert_eq!(controls.magic_varp, 108);
            assert_eq!(controls.selected_value, 2);
            assert_eq!(controls.armed_value, 3);
            assert!(controls.available());
            assert_eq!(data.spell_button_com("Wind Strike"), 1830);
        }
    }

    #[test]
    fn observe_tracks_staff_panel_and_armed_without_faking_missing_controls() {
        let missing = observe(None, 328, 3);
        assert_eq!(missing["available"], false);
        assert_eq!(missing["choose_com"], -1);
        assert_eq!(missing["staff_attached"], false);
        assert_eq!(missing["armed"], false);

        let data = data(ClientRevision::R274);
        let staff = observe(Some(data.as_ref()), 328, 0);
        assert_eq!(staff["available"], true);
        assert_eq!(staff["staff_attached"], true);
        assert_eq!(staff["panel_open"], false);
        assert_eq!(staff["armed"], false);
        let panel = observe(Some(data.as_ref()), 1829, 2);
        assert_eq!(panel["panel_open"], true);
        assert_eq!(panel["selected"], true);
        assert_eq!(panel["armed"], false);
        let armed = observe(Some(data.as_ref()), 328, 3);
        assert_eq!(armed["armed"], true);
        assert_eq!(armed["staff_attached"], true);
    }
}
