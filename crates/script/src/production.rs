//! Rust-owned Make-X count-dialog and anvil main-panel sequencing.
//!
//! Chat `make_products` already posts the Make/Smelt quantity buttons.
//! `ChatDialog.makeX` must click the posted Make-X control, wait the real
//! count-dialog latch, send one Answer-Count, wait the dialog closed, then
//! wait the make-menu to drop. The anvil panel is a distinct main-modal
//! TYPE_INV with Make-N ops: `makeFromPanelMax` presses the largest posted
//! Make-N on the matched row. JavaScript marshals the call argument,
//! dispatches the returned verbs and reports completion — it does not wait
//! one tick and guess the count dialog is open.

use crate::isolate_fb::{RowReader, SnapshotReader};
use serde_json::{json, Value};
use std::cell::RefCell;
use std::time::{Duration, Instant};

/// Frozen family-1 count-dialog open wait.
pub const COUNT_OPEN_MS: u64 = 3_000;
/// Frozen count-dialog close wait after Answer-Count.
pub const COUNT_CLOSE_MS: u64 = 3_000;
/// Frozen chat make-menu close wait after the count dialog drops.
pub const MAKE_MENU_MS: u64 = 5_000;
/// Frozen anvil panel modal-change wait.
pub const PANEL_WAIT_MS: u64 = 5_000;

thread_local! {
    static RUNTIME: RefCell<ProductionRuntime> = const { RefCell::new(ProductionRuntime::new()) };
    static NATIVE_OBSERVATION: RefCell<NativeObservation> =
        const { RefCell::new(NativeObservation::new()) };
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MakeButton {
    qty: i32,
    com_id: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MakeProduct {
    name: String,
    buttons: Vec<MakeButton>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PanelRow {
    name: String,
    id: i32,
    slot: i32,
    component: i32,
    ops: Vec<String>,
}

/// Compact projection of posted snapshot fields this module decides from.
struct NativeObservation {
    ingame: bool,
    count_dialog_open: bool,
    main_modal_id: i32,
    make_products: Vec<MakeProduct>,
    /// `None` = the main skill-multi panel was not decoded this rebuild.
    main_make: Option<Vec<PanelRow>>,
}

impl NativeObservation {
    const fn new() -> Self {
        Self {
            ingame: false,
            count_dialog_open: false,
            main_modal_id: -1,
            make_products: Vec::new(),
            main_make: None,
        }
    }

    fn update(&mut self, snap: &SnapshotReader<'_>) {
        if snap.has_ingame() {
            if !snap.ingame() {
                *self = Self::new();
                return;
            }
            self.ingame = true;
        }
        if snap.has_count_dialog_open() {
            self.count_dialog_open = snap.count_dialog_open();
        }
        if snap.has_main_modal_id() {
            self.main_modal_id = snap.main_modal_id();
        }
        if snap.has_make_products() {
            self.make_products = snap
                .make_products()
                .iter()
                .map(|product| MakeProduct {
                    name: product.name().to_string(),
                    buttons: product
                        .buttons()
                        .iter()
                        .map(|button| MakeButton {
                            qty: button.qty(),
                            com_id: button.com_id(),
                        })
                        .collect(),
                })
                .collect();
        }
        if snap.has_main_make_available() {
            self.main_make = if snap.main_make_available() {
                Some(panel_rows(snap.main_make()))
            } else {
                None
            };
        }
    }
}

fn panel_rows(rows: Vec<RowReader<'_>>) -> Vec<PanelRow> {
    rows.iter()
        .map(|row| PanelRow {
            name: row.name().unwrap_or_default().to_string(),
            id: row.id(),
            slot: row.slot(),
            component: row.component_id(),
            ops: row.ops().iter().map(|op| (*op).to_string()).collect(),
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    MakeX,
    MakeFromPanelMax,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Idle,
    /// Make-X button sent; waiting `count_dialog_open`.
    WaitCountOpen,
    /// One Answer-Count sent; waiting the dialog to close.
    WaitCountClose,
    /// Count dialog closed; waiting chat make-products to drop.
    WaitMakeMenu,
    /// Anvil Make-N sent; waiting the main modal identity to change.
    WaitPanel,
}

struct ProductionRuntime {
    paused: bool,
    held: bool,
    frozen_at: Option<Instant>,
    token: u64,
    phase: Phase,
    kind: Kind,
    /// Product / panel match string.
    match_name: String,
    /// Requested Make-X count.
    count: i32,
    /// Posted Make-X component.
    component_id: i32,
    /// Main modal id observed when the anvil op was sent.
    panel_before: i32,
    deadline: Option<Instant>,
}

impl ProductionRuntime {
    const fn new() -> Self {
        Self {
            paused: false,
            held: false,
            frozen_at: None,
            token: 0,
            phase: Phase::Idle,
            kind: Kind::MakeX,
            match_name: String::new(),
            count: 0,
            component_id: -1,
            panel_before: -1,
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

    fn arm(&mut self, window: u64) {
        self.deadline = Some(self.now() + Duration::from_millis(window));
    }

    fn bound_reached(&self) -> bool {
        self.deadline.is_some_and(|deadline| self.now() >= deadline)
    }

    fn abort_runtime(&mut self) {
        self.token = self.token.wrapping_add(1);
        self.phase = Phase::Idle;
        self.match_name.clear();
        self.count = 0;
        self.component_id = -1;
        self.panel_before = -1;
        self.deadline = None;
    }

    fn done(&mut self, result: bool, reason: &str) -> Value {
        let token = self.token;
        self.phase = Phase::Idle;
        self.deadline = None;
        json!({
            "kind": "done",
            "token": token,
            "result": result,
            "reason": reason,
        })
    }

    fn wait(&self) -> Value {
        json!({ "kind": "wait", "token": self.token })
    }

    fn if_button(&self) -> Value {
        json!({
            "kind": "ops",
            "token": self.token,
            "ops": [{ "op": "if-button", "component_id": self.component_id }],
        })
    }

    fn answer_count(&self) -> Value {
        json!({
            "kind": "ops",
            "token": self.token,
            "ops": [{ "op": "answer-count", "value": self.count }],
        })
    }

    fn panel_verb(&self, row: &PanelRow, operation: i32) -> Value {
        json!({
            "kind": "ops",
            "token": self.token,
            "ops": [{
                "op": "make-panel",
                "id": row.id,
                "slot": row.slot,
                "component": row.component,
                "operation": operation,
            }],
        })
    }
}

pub fn on_snapshot(snap: &SnapshotReader<'_>) {
    NATIVE_OBSERVATION.with(|obs| obs.borrow_mut().update(snap));
}

pub fn on_pause() {
    RUNTIME.with(|rt| {
        let held = rt.borrow().held;
        rt.borrow_mut().set_freeze(true, held);
    });
}

pub fn on_resume() {
    RUNTIME.with(|rt| {
        let held = rt.borrow().held;
        rt.borrow_mut().set_freeze(false, held);
    });
}

pub fn on_hold(held: bool) {
    RUNTIME.with(|rt| {
        let paused = rt.borrow().paused;
        rt.borrow_mut().set_freeze(paused, held);
    });
}

pub fn on_reset() {
    RUNTIME.with(|rt| rt.borrow_mut().abort_runtime());
    NATIVE_OBSERVATION.with(|obs| *obs.borrow_mut() = NativeObservation::new());
}

pub fn dispatch(input: &Value) -> Value {
    match input.get("op").and_then(Value::as_str).unwrap_or("") {
        "begin" => begin(input),
        "next" => next(input.get("token").and_then(Value::as_u64).unwrap_or(0)),
        _ => json!({ "kind": "notImpl", "reason": "unknown op" }),
    }
}

struct Probe<'a> {
    ingame: bool,
    count_dialog_open: bool,
    main_modal_id: i32,
    make_products: &'a [MakeProduct],
    main_make: Option<&'a [PanelRow]>,
}

fn refused(reason: &str) -> Value {
    RUNTIME.with(|rt| {
        let mut rt = rt.borrow_mut();
        rt.abort_runtime();
        let token = rt.token;
        json!({
            "kind": "done",
            "token": token,
            "result": false,
            "reason": reason,
        })
    })
}

fn begin(input: &Value) -> Value {
    let kind = match input.get("kind").and_then(Value::as_str).unwrap_or("") {
        "makeX" => Kind::MakeX,
        "makeFromPanelMax" => Kind::MakeFromPanelMax,
        _ => return json!({ "kind": "notImpl", "reason": "unknown production op" }),
    };
    let match_name = input
        .get("match")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    let obs = NATIVE_OBSERVATION.with(|o| {
        let o = o.borrow();
        (
            o.ingame,
            o.count_dialog_open,
            o.main_modal_id,
            o.make_products.clone(),
            o.main_make.clone(),
        )
    });
    let probe = Probe {
        ingame: obs.0,
        count_dialog_open: obs.1,
        main_modal_id: obs.2,
        make_products: &obs.3,
        main_make: obs.4.as_deref(),
    };
    if !probe.ingame {
        return json!({ "kind": "aborted", "reason": "not ingame" });
    }
    match kind {
        Kind::MakeX => begin_make_x(&probe, &match_name, input.get("count")),
        Kind::MakeFromPanelMax => begin_panel_max(&probe, &match_name),
    }
}

fn begin_make_x(probe: &Probe<'_>, match_name: &str, count: Option<&Value>) -> Value {
    if match_name.is_empty() {
        return refused("no-match");
    }
    let want = match_name.to_ascii_lowercase();
    let Some(product) = probe
        .make_products
        .iter()
        .find(|p| p.name.to_ascii_lowercase().contains(&want))
    else {
        return refused("absent");
    };
    let Some(button) = product.buttons.iter().find(|b| b.qty == -1) else {
        return json!({ "kind": "notImpl", "reason": "missing Make-X button" });
    };
    if button.com_id < 0 {
        return json!({ "kind": "notImpl", "reason": "missing Make-X button" });
    }
    let requested = match count {
        Some(value)
            if value
                .as_i64()
                .is_some_and(|n| (0..=i32::MAX as i64).contains(&n)) =>
        {
            value.as_i64().unwrap_or(0) as i32
        }
        _ => return refused("invalid"),
    };
    RUNTIME.with(|rt| {
        let mut rt = rt.borrow_mut();
        rt.abort_runtime();
        rt.kind = Kind::MakeX;
        rt.match_name = match_name.to_string();
        rt.count = requested;
        rt.component_id = button.com_id;
        rt.phase = Phase::WaitCountOpen;
        rt.arm(COUNT_OPEN_MS);
        rt.if_button()
    })
}

fn begin_panel_max(probe: &Probe<'_>, match_name: &str) -> Value {
    let Some(rows) = probe.main_make else {
        return json!({ "kind": "notImpl", "reason": "missing anvil panel" });
    };
    if match_name.is_empty() {
        return refused("no-match");
    }
    let want = match_name.to_ascii_lowercase();
    let Some(row) = rows
        .iter()
        .find(|row| row.name.to_ascii_lowercase().contains(&want))
    else {
        return refused("absent");
    };
    let Some((index, _)) = largest_make_op(&row.ops) else {
        return refused("missing-make-op");
    };
    let operation = (index + 1) as i32;
    RUNTIME.with(|rt| {
        let mut rt = rt.borrow_mut();
        rt.abort_runtime();
        rt.kind = Kind::MakeFromPanelMax;
        rt.match_name = match_name.to_string();
        rt.panel_before = probe.main_modal_id;
        rt.phase = Phase::WaitPanel;
        rt.arm(PANEL_WAIT_MS);
        rt.panel_verb(row, operation)
    })
}

fn next(token: u64) -> Value {
    NATIVE_OBSERVATION.with(|o| {
        let o = o.borrow();
        let probe = Probe {
            ingame: o.ingame,
            count_dialog_open: o.count_dialog_open,
            main_modal_id: o.main_modal_id,
            make_products: &o.make_products,
            main_make: o.main_make.as_deref(),
        };
        RUNTIME.with(|rt| {
            let mut rt = rt.borrow_mut();
            if token != rt.token || rt.phase == Phase::Idle {
                return json!({ "kind": "aborted", "token": rt.token });
            }
            if rt.frozen() {
                return rt.wait();
            }
            if !probe.ingame {
                let token = rt.token;
                rt.phase = Phase::Idle;
                rt.deadline = None;
                return json!({ "kind": "aborted", "token": token });
            }
            match rt.kind {
                Kind::MakeX => make_x_step(&mut rt, &probe),
                Kind::MakeFromPanelMax => panel_step(&mut rt, &probe),
            }
        })
    })
}

fn make_x_step(rt: &mut ProductionRuntime, probe: &Probe<'_>) -> Value {
    match rt.phase {
        Phase::WaitCountOpen => {
            if probe.count_dialog_open {
                rt.phase = Phase::WaitCountClose;
                rt.arm(COUNT_CLOSE_MS);
                return rt.answer_count();
            }
            if rt.bound_reached() {
                return rt.done(false, "count-open-timeout");
            }
            rt.wait()
        }
        Phase::WaitCountClose => {
            if !probe.count_dialog_open {
                rt.phase = Phase::WaitMakeMenu;
                rt.arm(MAKE_MENU_MS);
                return rt.wait();
            }
            if rt.bound_reached() {
                return rt.done(false, "count-close-timeout");
            }
            rt.wait()
        }
        Phase::WaitMakeMenu => {
            if probe.make_products.is_empty() {
                return rt.done(true, "menu-closed");
            }
            if rt.bound_reached() {
                return rt.done(false, "menu-timeout");
            }
            rt.wait()
        }
        Phase::Idle | Phase::WaitPanel => rt.done(false, "idle"),
    }
}

fn panel_step(rt: &mut ProductionRuntime, probe: &Probe<'_>) -> Value {
    match rt.phase {
        Phase::WaitPanel => {
            if probe.main_modal_id != rt.panel_before {
                return rt.done(true, "panel-closed");
            }
            if rt.bound_reached() {
                return rt.done(false, "panel-timeout");
            }
            rt.wait()
        }
        _ => rt.done(false, "idle"),
    }
}

/// The 0-based op index and parsed quantity of the largest posted Make-N
/// label. A Make op without digits counts as 1. Missing Make controls
/// refuse rather than inventing an index.
pub fn largest_make_op(ops: &[String]) -> Option<(usize, i32)> {
    let mut best: Option<(usize, i32)> = None;
    for (index, op) in ops.iter().enumerate() {
        if op.is_empty() {
            continue;
        }
        if !op.to_ascii_lowercase().contains("make") {
            continue;
        }
        let qty = op
            .chars()
            .filter(|c| c.is_ascii_digit())
            .collect::<String>()
            .parse::<i32>()
            .unwrap_or(1);
        let qty = if qty < 1 { 1 } else { qty };
        match best {
            Some((_, best_qty)) if qty <= best_qty => {}
            _ => best = Some((index, qty)),
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    fn product(name: &str, buttons: &[(i32, i32)]) -> MakeProduct {
        MakeProduct {
            name: name.into(),
            buttons: buttons
                .iter()
                .map(|(qty, com_id)| MakeButton {
                    qty: *qty,
                    com_id: *com_id,
                })
                .collect(),
        }
    }

    fn panel(name: &str, id: i32, slot: i32, component: i32, ops: &[&str]) -> PanelRow {
        PanelRow {
            name: name.into(),
            id,
            slot,
            component,
            ops: ops.iter().map(|op| (*op).to_string()).collect(),
        }
    }

    fn probe<'a>(
        products: &'a [MakeProduct],
        main_make: Option<&'a [PanelRow]>,
        count_open: bool,
        main_modal: i32,
    ) -> Probe<'a> {
        Probe {
            ingame: true,
            count_dialog_open: count_open,
            main_modal_id: main_modal,
            make_products: products,
            main_make,
        }
    }

    #[test]
    fn largest_make_op_picks_the_posted_make_n_not_a_guessed_first_slot() {
        let ops = [
            "Examine".into(),
            "Make 1".into(),
            "Make 5".into(),
            "Make 10".into(),
        ];
        assert_eq!(largest_make_op(&ops), Some((3, 10)));
        let only_make = ["Make".into()];
        assert_eq!(largest_make_op(&only_make), Some((0, 1)));
        let no_make = ["Examine".into(), "Drop".into()];
        assert!(largest_make_op(&no_make).is_none());
    }

    #[test]
    fn make_x_does_not_answer_count_until_the_dialog_is_posted_open() {
        let products = [product("Bow string", &[(-1, 8875), (10, 8876)])];
        let mut rt = ProductionRuntime::new();
        rt.kind = Kind::MakeX;
        rt.component_id = 8875;
        rt.count = 28;
        rt.phase = Phase::WaitCountOpen;
        rt.arm(COUNT_OPEN_MS);
        let closed = probe(&products, None, false, -1);
        let step = make_x_step(&mut rt, &closed);
        assert_eq!(step["kind"], "wait");
        assert_eq!(rt.phase, Phase::WaitCountOpen);

        let open = probe(&products, None, true, -1);
        let step = make_x_step(&mut rt, &open);
        assert_eq!(step["kind"], "ops");
        assert_eq!(step["ops"][0]["op"], "answer-count");
        assert_eq!(step["ops"][0]["value"], 28);
        assert_eq!(rt.phase, Phase::WaitCountClose);
    }

    #[test]
    fn make_x_count_open_timeout_sends_no_answer() {
        let products = [product("Bow string", &[(-1, 8875)])];
        let mut rt = ProductionRuntime::new();
        rt.kind = Kind::MakeX;
        rt.count = 28;
        rt.phase = Phase::WaitCountOpen;
        rt.deadline = Some(Instant::now() - Duration::from_millis(1));
        let closed = probe(&products, None, false, -1);
        let step = make_x_step(&mut rt, &closed);
        assert_eq!(step["kind"], "done");
        assert_eq!(step["result"], false);
        assert_eq!(step["reason"], "count-open-timeout");
    }

    #[test]
    fn make_x_waits_count_close_then_the_make_menu() {
        let products = [product("Bow string", &[(-1, 8875)])];
        let mut rt = ProductionRuntime::new();
        rt.kind = Kind::MakeX;
        rt.phase = Phase::WaitCountClose;
        rt.arm(COUNT_CLOSE_MS);
        let open = probe(&products, None, true, -1);
        let step = make_x_step(&mut rt, &open);
        assert_eq!(step["kind"], "wait");

        let closed = probe(&products, None, false, -1);
        let step = make_x_step(&mut rt, &closed);
        assert_eq!(step["kind"], "wait");
        assert_eq!(rt.phase, Phase::WaitMakeMenu);

        let gone: [MakeProduct; 0] = [];
        let empty = probe(&gone, None, false, -1);
        let step = make_x_step(&mut rt, &empty);
        assert_eq!(step["kind"], "done");
        assert_eq!(step["result"], true);
        assert_eq!(step["reason"], "menu-closed");
    }

    #[test]
    fn pause_and_hold_freeze_the_count_deadline() {
        let mut rt = ProductionRuntime::new();
        rt.arm(COUNT_OPEN_MS);
        let before = rt.deadline.expect("armed");
        rt.set_freeze(true, false);
        assert!(rt.frozen());
        std::thread::sleep(Duration::from_millis(5));
        rt.set_freeze(false, false);
        assert!(!rt.frozen());
        assert!(rt.deadline.expect("still armed") > before);
        rt.set_freeze(false, true);
        assert!(rt.frozen(), "guardian hold freezes too");
    }

    #[test]
    fn abort_runtime_bumps_the_token_and_clears_the_operation() {
        let mut rt = ProductionRuntime::new();
        rt.kind = Kind::MakeX;
        rt.match_name = "Flax".into();
        rt.count = 28;
        rt.component_id = 8875;
        rt.arm(COUNT_OPEN_MS);
        let before = rt.token;
        rt.abort_runtime();
        assert_eq!(rt.token, before.wrapping_add(1));
        assert_eq!(rt.phase, Phase::Idle);
        assert!(rt.match_name.is_empty());
        assert_eq!(rt.count, 0);
        assert_eq!(rt.component_id, -1);
        assert!(rt.deadline.is_none());
    }

    #[test]
    fn panel_max_succeeds_when_the_posted_main_modal_changes() {
        let rows = [panel(
            "Bronze dagger",
            1205,
            0,
            1119,
            &["Make 1", "Make 5", "Make 10"],
        )];
        let mut rt = ProductionRuntime::new();
        rt.kind = Kind::MakeFromPanelMax;
        rt.panel_before = 3000;
        rt.phase = Phase::WaitPanel;
        rt.arm(PANEL_WAIT_MS);
        let still = probe(&[], Some(rows.as_slice()), false, 3000);
        let step = panel_step(&mut rt, &still);
        assert_eq!(step["kind"], "wait");
        let closed = probe(&[], Some(rows.as_slice()), false, -1);
        let step = panel_step(&mut rt, &closed);
        assert_eq!(step["kind"], "done");
        assert_eq!(step["result"], true);
    }

    #[test]
    fn panel_timeout_does_not_invent_a_second_press() {
        let rows = [panel("Bronze dagger", 1205, 0, 1119, &["Make 10"])];
        let mut rt = ProductionRuntime::new();
        rt.kind = Kind::MakeFromPanelMax;
        rt.panel_before = 3000;
        rt.phase = Phase::WaitPanel;
        rt.deadline = Some(Instant::now() - Duration::from_millis(1));
        let still = probe(&[], Some(rows.as_slice()), false, 3000);
        let step = panel_step(&mut rt, &still);
        assert_eq!(step["kind"], "done");
        assert_eq!(step["result"], false);
        assert_eq!(step["reason"], "panel-timeout");
    }
}
