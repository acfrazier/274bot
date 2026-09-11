//! Selected-world autocast control facts and Rust-owned arm sequencing.
//! Snapshot deltas feed a compact native projection. JS keeps the frozen ABI,
//! dispatches returned buttons and logs the final reason.

use crate::isolate_fb::SnapshotReader;
use api::game_data::{AutocastControls, SelectedGameData};
use serde_json::{json, Value};
use std::cell::RefCell;
use std::time::{Duration, Instant};

const COMBAT_TAB: i32 = 0;
const TAB_WAIT_MS: u64 = 2_000;
const STEP_MS: u64 = 3_000;

thread_local! {
    static RUNTIME: RefCell<AutocastRuntime> = const { RefCell::new(AutocastRuntime::new()) };
    static OBSERVATION: RefCell<NativeObservation> = const { RefCell::new(NativeObservation::new()) };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Idle,
    WaitTab,
    WaitPanel,
    WaitSelected,
    WaitArmed,
}

#[derive(Clone, Copy)]
struct ArmObservation {
    ingame: bool,
    active_side_tab: i32,
    combat_tab_root: i32,
    magic_varp_value: i32,
}

struct NativeObservation {
    arm: ArmObservation,
    magic_varp: i32,
}

impl NativeObservation {
    const fn new() -> Self {
        Self {
            arm: ArmObservation {
                ingame: false,
                active_side_tab: -1,
                combat_tab_root: -1,
                magic_varp_value: 0,
            },
            magic_varp: -1,
        }
    }

    fn clear_facts(&mut self) {
        self.arm = Self::new().arm;
    }

    fn configure(&mut self, data: Option<&SelectedGameData>) {
        *self = Self::new();
        self.magic_varp = data
            .and_then(SelectedGameData::autocast_controls)
            .map_or(-1, |controls| controls.magic_varp);
    }

    fn update(&mut self, snap: &SnapshotReader<'_>) {
        if snap.has_ingame() {
            if !snap.ingame() {
                self.clear_facts();
                return;
            }
            self.arm.ingame = true;
        }
        if snap.has_side_tab() {
            self.arm.active_side_tab = snap.side_tab();
        }
        if snap.has_side_tab_ifaces() {
            self.arm.combat_tab_root = snap
                .side_tab_ifaces()
                .iter()
                .find(|row| row.index() == COMBAT_TAB)
                .map_or(-1, |row| row.id());
        }
        if snap.has_varps() {
            self.arm.magic_varp_value = snap
                .varps()
                .iter()
                .find(|row| row.index() == self.magic_varp)
                .map_or(0, |row| row.value());
        }
    }
}

struct AutocastRuntime {
    paused: bool,
    held: bool,
    frozen_at: Option<Instant>,
    token: u64,
    phase: Phase,
    spell_com: i32,
    deadline: Option<Instant>,
}

impl AutocastRuntime {
    const fn new() -> Self {
        Self {
            paused: false,
            held: false,
            frozen_at: None,
            token: 0,
            phase: Phase::Idle,
            spell_com: -1,
            deadline: None,
        }
    }

    fn frozen(&self) -> bool {
        self.paused || self.held
    }

    fn now(&self) -> Instant {
        self.frozen_at.unwrap_or_else(Instant::now)
    }

    fn set_freeze(&mut self, paused: bool, held: bool) {
        let was_frozen = self.frozen();
        self.paused = paused;
        self.held = held;
        let frozen = self.frozen();
        if !was_frozen && frozen {
            self.frozen_at = Some(Instant::now());
        } else if was_frozen && !frozen {
            if let Some(at) = self.frozen_at.take() {
                if let Some(deadline) = self.deadline.as_mut() {
                    *deadline += Instant::now().saturating_duration_since(at);
                }
            }
        }
    }

    fn abort(&mut self) {
        self.token = self.token.wrapping_add(1);
        self.phase = Phase::Idle;
        self.spell_com = -1;
        self.deadline = None;
    }

    fn command(&mut self, phase: Phase, component_id: i32, timeout_ms: u64) -> Value {
        self.phase = phase;
        self.deadline = Some(self.now() + Duration::from_millis(timeout_ms));
        json!({
            "kind": "if-button",
            "token": self.token,
            "component_id": component_id,
        })
    }

    fn done(&mut self, ok: bool, reason: &str) -> Value {
        self.phase = Phase::Idle;
        self.deadline = None;
        json!({
            "kind": "done",
            "token": self.token,
            "ok": ok,
            "reason": reason,
        })
    }

    fn begin(
        &mut self,
        controls: Option<&AutocastControls>,
        spell_com: i32,
        obs: &ArmObservation,
    ) -> Value {
        self.abort();
        let Some(controls) = controls.filter(|controls| controls.available()) else {
            return self.done(false, "missing-controls");
        };
        if spell_com < 0 {
            return self.done(false, "unknown-spell");
        }
        if !obs.ingame {
            return self.done(false, "missing-facts");
        }
        if obs.combat_tab_root != controls.staff_tab_root {
            return self.done(false, "staff-missing");
        }
        self.spell_com = spell_com;
        if obs.active_side_tab != COMBAT_TAB {
            self.phase = Phase::WaitTab;
            self.deadline = Some(self.now() + Duration::from_millis(TAB_WAIT_MS));
            return json!({"kind": "side-tab", "token": self.token, "tab": COMBAT_TAB});
        }
        self.command(Phase::WaitPanel, controls.choose_com, STEP_MS)
    }

    fn next(
        &mut self,
        token: u64,
        controls: Option<&AutocastControls>,
        obs: &ArmObservation,
    ) -> Value {
        if token != self.token || self.phase == Phase::Idle {
            return json!({"kind": "aborted", "token": self.token});
        }
        if self.frozen() {
            return json!({"kind": "wait", "token": self.token});
        }
        if !obs.ingame {
            return self.done(false, "aborted");
        }
        let Some(controls) = controls.filter(|controls| controls.available()) else {
            return self.done(false, "missing-controls");
        };
        match self.phase {
            Phase::Idle => json!({"kind": "aborted", "token": self.token}),
            Phase::WaitTab if obs.active_side_tab == COMBAT_TAB => {
                self.command(Phase::WaitPanel, controls.choose_com, STEP_MS)
            }
            Phase::WaitPanel if obs.combat_tab_root == controls.spell_panel_root => {
                self.command(Phase::WaitSelected, self.spell_com, STEP_MS)
            }
            Phase::WaitSelected if obs.magic_varp_value == controls.selected_value => {
                self.command(Phase::WaitArmed, controls.toggle_com, STEP_MS)
            }
            Phase::WaitArmed if obs.magic_varp_value == controls.armed_value => {
                self.done(true, "armed")
            }
            phase if self.deadline.is_some_and(|deadline| self.now() >= deadline) => {
                let reason = match phase {
                    Phase::WaitTab => "open-tab",
                    Phase::WaitPanel => "chooser",
                    Phase::WaitSelected => "select",
                    Phase::WaitArmed => "toggle",
                    Phase::Idle => "aborted",
                };
                self.done(false, reason)
            }
            _ => json!({"kind": "wait", "token": self.token}),
        }
    }
}

pub fn on_pause() {
    RUNTIME.with(|runtime| {
        let held = runtime.borrow().held;
        runtime.borrow_mut().set_freeze(true, held);
    });
}

pub fn on_resume() {
    RUNTIME.with(|runtime| {
        let held = runtime.borrow().held;
        runtime.borrow_mut().set_freeze(false, held);
    });
}

pub fn on_hold(held: bool) {
    RUNTIME.with(|runtime| {
        let paused = runtime.borrow().paused;
        runtime.borrow_mut().set_freeze(paused, held);
    });
}

pub fn on_reset() {
    RUNTIME.with(|runtime| runtime.borrow_mut().abort());
    OBSERVATION.with(|observation| observation.borrow_mut().clear_facts());
}

pub fn configure(data: Option<&SelectedGameData>) {
    OBSERVATION.with(|observation| observation.borrow_mut().configure(data));
}

pub fn on_snapshot(snap: &SnapshotReader<'_>) {
    OBSERVATION.with(|observation| observation.borrow_mut().update(snap));
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
    OBSERVATION.with(|observation| {
        let obs = observation.borrow().arm;
        match input
            .get("op")
            .and_then(Value::as_str)
            .unwrap_or("controls")
        {
            "observe" => observe(data, obs.combat_tab_root, obs.magic_varp_value),
            "begin" => RUNTIME.with(|runtime| {
                runtime.borrow_mut().begin(
                    data.and_then(SelectedGameData::autocast_controls),
                    input.get("spell_com").and_then(Value::as_i64).unwrap_or(-1) as i32,
                    &obs,
                )
            }),
            "next" => RUNTIME.with(|runtime| {
                runtime.borrow_mut().next(
                    input.get("token").and_then(Value::as_u64).unwrap_or(0),
                    data.and_then(SelectedGameData::autocast_controls),
                    &obs,
                )
            }),
            "current_token" => RUNTIME.with(|runtime| json!(runtime.borrow().token)),
            _ => controls_json(data),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use client::io::ClientRevision;

    fn data(rev: ClientRevision) -> std::sync::Arc<SelectedGameData> {
        api::game_data::for_revision(rev).expect("selected data")
    }

    fn set_observation(active_side_tab: i32, combat_tab_root: i32, magic_varp_value: i32) {
        OBSERVATION.with(|observation| {
            observation.borrow_mut().arm = ArmObservation {
                ingame: true,
                active_side_tab,
                combat_tab_root,
                magic_varp_value,
            };
        });
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

    #[test]
    fn missing_arm_observation_fails_closed() {
        let data = data(ClientRevision::R274);
        configure(Some(data.as_ref()));
        let result = dispatch(
            Some(data.as_ref()),
            &json!({"op": "begin", "spell_com": 1830}),
        );
        assert_eq!(result["kind"], "done");
        assert_eq!(result["ok"], false);
        assert_eq!(result["reason"], "missing-facts");
    }

    #[test]
    fn native_arm_next_owns_choose_spell_toggle_order() {
        let data = data(ClientRevision::R274);
        configure(Some(data.as_ref()));
        set_observation(0, 328, 0);
        let begin = dispatch(
            Some(data.as_ref()),
            &json!({
                "op": "begin",
                "spell_com": 1830,
            }),
        );
        let token = begin["token"].as_u64().expect("native arm token");
        assert_eq!(begin["kind"], "if-button");
        assert_eq!(begin["component_id"], 353);

        set_observation(0, 1829, 0);
        let spell = dispatch(
            Some(data.as_ref()),
            &json!({
                "op": "next",
                "token": token,
            }),
        );
        assert_eq!(spell["kind"], "if-button");
        assert_eq!(spell["component_id"], 1830);

        set_observation(0, 1829, 2);
        let toggle = dispatch(
            Some(data.as_ref()),
            &json!({
                "op": "next",
                "token": token,
            }),
        );
        assert_eq!(toggle["kind"], "if-button");
        assert_eq!(toggle["component_id"], 349);

        set_observation(0, 328, 3);
        let done = dispatch(
            Some(data.as_ref()),
            &json!({
                "op": "next",
                "token": token,
            }),
        );
        assert_eq!(done["kind"], "done");
        assert_eq!(done["ok"], true);
    }
}
