//! Selected-world special-attack facts. JS keeps the frozen Special ABI
//! and the two-tick arm wait. Rust owns packed bar/cost identities so the
//! shim never copies SPECBAR_BY_ROOT or SPEC_COST.

use api::game_data::{SelectedGameData, SpecialControls};
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
    match data.and_then(SelectedGameData::special_controls) {
        Some(controls) => json_controls(controls, true),
        None => json_controls(
            &SpecialControls {
                energy_varp: -1,
                armed_varp: -1,
                armed_value: 1,
                max_energy: 1000,
                arm_confirm_ticks: 2,
                bars: Vec::new(),
                weapons: Vec::new(),
            },
            false,
        ),
    }
}

fn json_controls(controls: &SpecialControls, available: bool) -> Value {
    json!({
        "available": available && controls.available(),
        "energy_varp": controls.energy_varp,
        "armed_varp": controls.armed_varp,
        "armed_value": controls.armed_value,
        "max_energy": controls.max_energy,
        "arm_confirm_ticks": controls.arm_confirm_ticks,
    })
}

fn cost_json(data: Option<&SelectedGameData>, weapon: &str) -> Value {
    match data.and_then(|selected| selected.special_cost(weapon)) {
        Some(cost) => json!(cost),
        None => Value::Null,
    }
}

fn bar_json(data: Option<&SelectedGameData>, combat_tab_root: i32) -> Value {
    json!(data
        .map(|selected| selected.special_bar(combat_tab_root))
        .unwrap_or(-1))
}

pub fn dispatch(data: Option<&SelectedGameData>, input: &Value) -> Value {
    match input
        .get("op")
        .and_then(Value::as_str)
        .unwrap_or("controls")
    {
        "cost" => cost_json(
            data,
            input.get("weapon").and_then(Value::as_str).unwrap_or(""),
        ),
        "bar" => bar_json(
            data,
            input
                .get("combat_tab_root")
                .and_then(Value::as_i64)
                .unwrap_or(-1) as i32,
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
    fn both_selected_caches_post_the_audited_special_controls() {
        for rev in [ClientRevision::R274, ClientRevision::R289] {
            let data = data(rev);
            let controls = data.special_controls().expect("generated special controls");
            assert_eq!(controls.energy_varp, 300);
            assert_eq!(controls.armed_varp, 301);
            assert_eq!(controls.armed_value, 1);
            assert_eq!(controls.max_energy, 1000);
            assert_eq!(controls.arm_confirm_ticks, 2);
            assert!(controls.available());
            assert_eq!(controls.cost("Dragon dagger"), Some(250));
            assert_eq!(controls.cost("  dragon DAGGER(p) "), Some(250));
            assert_eq!(controls.cost("Magic shortbow"), Some(350));
            assert_eq!(controls.cost("Rune thrownaxe"), Some(100));
            assert_eq!(controls.cost("Dragon halberd"), Some(300));
            assert_eq!(controls.cost("Rune scimitar"), None);
            assert_eq!(controls.cost("Dragon battleaxe"), None);
            assert_eq!(controls.cost("Excalibur"), None);
            assert_eq!(controls.bar_for_root(425), 7462);
            assert_eq!(controls.bar_for_root(2423), 7587);
            assert_eq!(controls.bar_for_root(8460), 8481);
            assert_eq!(controls.bar_for_root(328), -1);
        }
    }

    #[test]
    fn missing_controls_do_not_invent_costs_or_bars() {
        assert_eq!(controls_json(None)["available"], false);
        assert_eq!(cost_json(None, "Dragon dagger"), Value::Null);
        assert_eq!(bar_json(None, 425), json!(-1));
    }
}
